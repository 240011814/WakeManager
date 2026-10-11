//! WakeManager - Wi-Fi provisioning module
//! 对应 C 版 `components/wifi_prov/wifi_prov.c`
//!
//! 所有 Wi-Fi / 配网配置都在这里；app_main 只调用 `start()` 和 `wait_connected()`。
//! 依赖 `network_provisioning` 托管组件的 FFI 绑定（P3 原型已验证）。
//! `sim` feature（Wokwi 模拟构建）下跳过配网管理器，直接 STA 连接模拟 AP。

use std::ffi::c_void;
use std::ptr;
use std::sync::{Condvar, Mutex};

use esp_idf_sys::*;

#[cfg(not(feature = "sim"))]
use crate::app_config as ac;
use crate::led_status;

/// IP_EVENT_STA_GOT_IP 置位；`wait_connected()` 在条件变量上等待（无轮询）
static CONNECTED: Mutex<bool> = Mutex::new(false);
static CONNECTED_CV: Condvar = Condvar::new();

fn signal_connected() {
    *CONNECTED.lock().unwrap() = true;
    CONNECTED_CV.notify_all();
}

/// 复刻 C 宏 `WIFI_INIT_CONFIG_DEFAULT()`（bindgen 不可见：宏内含大量
/// CONFIG_ 派生宏）。
///
/// 数值全部取自 esp-idf-sys 由当前 sdkconfig 生成的 `WIFI_*` / `CONFIG_*`
/// 常量（与 esp-idf-svc 内部实现一致），因此改 menuconfig 后会自动跟随，
/// 不再需要手工同步硬编码值。
fn wifi_init_config_default() -> wifi_init_config_t {
    // SAFETY: g_wifi_osi_funcs 由 esp_wifi 组件提供；wpa crypto 表为只读默认值
    unsafe {
        wifi_init_config_t {
            osi_funcs: &raw mut g_wifi_osi_funcs,
            wpa_crypto_funcs: ptr::read(&raw const g_wifi_default_wpa_crypto_funcs),
            static_rx_buf_num: CONFIG_ESP_WIFI_STATIC_RX_BUFFER_NUM as _,
            dynamic_rx_buf_num: CONFIG_ESP_WIFI_DYNAMIC_RX_BUFFER_NUM as _,
            tx_buf_type: CONFIG_ESP_WIFI_TX_BUFFER_TYPE as _,
            static_tx_buf_num: WIFI_STATIC_TX_BUFFER_NUM as _,
            dynamic_tx_buf_num: WIFI_DYNAMIC_TX_BUFFER_NUM as _,
            rx_mgmt_buf_type: CONFIG_ESP_WIFI_DYNAMIC_RX_MGMT_BUF as _,
            rx_mgmt_buf_num: WIFI_RX_MGMT_BUF_NUM_DEF as _,
            cache_tx_buf_num: WIFI_CACHE_TX_BUFFER_NUM as _,
            csi_enable: WIFI_CSI_ENABLED as _,
            ampdu_rx_enable: WIFI_AMPDU_RX_ENABLED as _,
            ampdu_tx_enable: WIFI_AMPDU_TX_ENABLED as _,
            amsdu_tx_enable: WIFI_AMSDU_TX_ENABLED as _,
            nvs_enable: 1,
            nano_enable: WIFI_NANO_FORMAT_ENABLED as _,
            rx_ba_win: WIFI_DEFAULT_RX_BA_WIN as _,
            wifi_task_core_id: WIFI_TASK_CORE_ID as _,
            beacon_max_len: WIFI_SOFTAP_BEACON_MAX_LEN as _,
            mgmt_sbuf_num: WIFI_MGMT_SBUF_NUM as _,
            feature_caps: WIFI_FEATURE_CAPS as _,
            sta_disconnected_pm: WIFI_STA_DISCONNECTED_PM_ENABLED != 0,
            espnow_max_encrypt_num: CONFIG_ESP_WIFI_ESPNOW_MAX_ENCRYPT_NUM as _,
            tx_hetb_queue_num: WIFI_TX_HETB_QUEUE_NUM as _,
            dump_hesigb_enable: WIFI_DUMP_HESIGB_ENABLED != 0,
            privacy_enhancements: WIFI_PRIVACY_ENHANCEMENTS_ENABLED != 0,
            rmac_auto_reset_int: WIFI_RMAC_AUTO_RESET_INTERVAL as _,
            magic: WIFI_INIT_CONFIG_MAGIC as _, // 必须正确
        }
    }
}

/// 统一事件处理：配网生命周期 + Wi-Fi 重连 + IP 获取 + BLE/安全会话日志。
/// 对应 C 版 wifi_prov.c 的 event_handler()。
///
/// # Safety
/// 由 esp_event 循环以裸指针回调；只读取 event_data 中需要的字段。
#[allow(non_upper_case_globals)] // 匹配 bindgen 生成的常量名
unsafe extern "C" fn event_handler(
    _arg: *mut c_void,
    event_base: esp_event_base_t,
    event_id: i32,
    event_data: *mut c_void,
) {
    if event_base == NETWORK_PROV_EVENT {
        match event_id as u32 {
            network_prov_cb_event_t_NETWORK_PROV_START => {
                ::log::info!("Provisioning started");
                led_status::set(led_status::Status::Connecting);
            }
            network_prov_cb_event_t_NETWORK_PROV_WIFI_CRED_RECV => {
                let cfg = &*(event_data as *const wifi_sta_config_t);
                let len = cfg.ssid.iter().position(|&b| b == 0).unwrap_or(32);
                ::log::info!(
                    "Received Wi-Fi credentials\n\tSSID: {}",
                    String::from_utf8_lossy(&cfg.ssid[..len])
                );
            }
            network_prov_cb_event_t_NETWORK_PROV_WIFI_CRED_FAIL => {
                let reason = *(event_data as *const network_prov_wifi_sta_fail_reason_t);
                ::log::error!(
                    "Provisioning failed: {}",
                    if reason == network_prov_wifi_sta_fail_reason_t_NETWORK_PROV_WIFI_STA_AUTH_ERROR
                    {
                        "Wi-Fi station authentication failed"
                    } else {
                        "Wi-Fi access-point not found"
                    }
                );
                led_status::set(led_status::Status::Connecting);
            }
            network_prov_cb_event_t_NETWORK_PROV_WIFI_CRED_SUCCESS => {
                ::log::info!("Provisioning successful");
            }
            network_prov_cb_event_t_NETWORK_PROV_END => {
                // 配网服务停止 → 释放管理器
                ::log::info!("Provisioning ended");
                let err = network_prov_mgr_deinit();
                if err != ESP_OK {
                    ::log::error!("Failed to de-initialize provisioning manager: {err:#x}");
                }
            }
            _ => {}
        }
    } else if event_base == WIFI_EVENT {
        match event_id {
            x if x == wifi_event_t_WIFI_EVENT_STA_START as i32 => {
                esp_wifi_connect();
            }
            x if x == wifi_event_t_WIFI_EVENT_STA_DISCONNECTED as i32 => {
                ::log::info!("Disconnected, reconnecting...");
                led_status::set(led_status::Status::Connecting);
                esp_wifi_connect();
            }
            _ => {}
        }
    } else if event_base == IP_EVENT && event_id == ip_event_t_IP_EVENT_STA_GOT_IP as i32 {
        let event = &*(event_data as *const ip_event_got_ip_t);
        let ip = &event.ip_info.ip;
        ::log::info!(
            "Connected with IP address: {}.{}.{}.{}",
            ip.addr as u32 & 0xff,
            (ip.addr as u32 >> 8) & 0xff,
            (ip.addr as u32 >> 16) & 0xff,
            (ip.addr as u32 >> 24) & 0xff
        );
        led_status::set(led_status::Status::Connected);
        signal_connected();
    } else if event_base == PROTOCOMM_TRANSPORT_BLE_EVENT {
        match event_id as u32 {
            protocomm_transport_ble_event_t_PROTOCOMM_TRANSPORT_BLE_CONNECTED => {
                ::log::info!("BLE transport: Connected");
            }
            protocomm_transport_ble_event_t_PROTOCOMM_TRANSPORT_BLE_DISCONNECTED => {
                ::log::info!("BLE transport: Disconnected");
            }
            _ => {}
        }
    } else if event_base == PROTOCOMM_SECURITY_SESSION_EVENT {
        match event_id as u32 {
            protocomm_security_session_event_t_PROTOCOMM_SECURITY_SESSION_SETUP_OK => {
                ::log::info!("Secured session established");
            }
            protocomm_security_session_event_t_PROTOCOMM_SECURITY_SESSION_INVALID_SECURITY_PARAMS
            | protocomm_security_session_event_t_PROTOCOMM_SECURITY_SESSION_CREDENTIALS_MISMATCH => {
                ::log::error!("Failed to establish a secure session");
            }
            _ => {}
        }
    }
}

/// 自定义配网端点 "custom-data" 的处理函数（对应 C 版 custom_prov_data_handler）。
/// 响应缓冲区必须堆分配，由 protocomm 层负责释放。
///
/// # Safety
/// inbuf/outbuf 由调用方（protocomm）保证有效；这里只做一次性读写。
#[cfg(not(feature = "sim"))]
unsafe extern "C" fn custom_data_handler(
    _session_id: u32,
    inbuf: *const u8,
    inlen: isize,
    outbuf: *mut *mut u8,
    outlen: *mut isize,
    _priv_data: *mut c_void,
) -> esp_err_t {
    if !inbuf.is_null() {
        let data = std::slice::from_raw_parts(inbuf, inlen as usize);
        ::log::info!("Received custom data: {:?}", String::from_utf8_lossy(data));
    }
    // SUCCESS + NUL；into_raw 把所有权交给 C 侧（protocomm 用 free() 释放，
    // ESP-IDF std 的 Rust 分配器底层即 malloc/free，兼容）
    let resp = std::ffi::CString::new("SUCCESS").expect("no NUL in literal");
    let resp_len = resp.as_bytes_with_nul().len();
    *outbuf = resp.into_raw() as *mut u8;
    *outlen = resp_len as isize;
    ESP_OK
}

fn check(err: esp_err_t, ctx: &str) {
    if err != ESP_OK {
        ::log::error!("{ctx} failed: {err:#x}");
        std::process::abort();
    }
}

/// BLE 名称 / 服务名：`PROV_` + STA MAC 后 3 字节（与 C 版一致）。
/// `esp_wifi_get_mac` 失败时返回 `None`（调用方回退到全零后缀并记录）。
#[cfg(not(feature = "sim"))]
fn device_service_name() -> Option<std::ffi::CString> {
    match crate::device_info::get_mac() {
        Ok(mac) => Some(
            std::ffi::CString::new(format!(
                "{}{:02X}{:02X}{:02X}",
                ac::PROV_DEVICE_NAME_PREFIX,
                mac[3],
                mac[4],
                mac[5]
            ))
            .expect("service name has no NUL"),
        ),
        Err(e) => {
            ::log::error!("Cannot read STA MAC for provisioning service name: {e:?}");
            None
        }
    }
}

/// 对应 C 版 prov_print_qr()：打印配网 payload（日志用）
#[cfg(not(feature = "sim"))]
fn prov_print_qr(name: &str, pop: &str) {
    let payload = format!(
        "{{\"ver\":\"{}\",\"name\":\"{}\",\"pop\":\"{}\",\"transport\":\"{}\"}}",
        ac::PROV_QR_VERSION,
        name,
        pop,
        ac::PROV_QR_TRANSPORT
    );
    ::log::info!("Scan this payload with the provisioning app (or generate a QR from it):");
    ::log::info!("{}?data={}", ac::PROV_QRCODE_BASE_URL, payload);
}

/// 对应 C 版 `wifi_prov_start()`：
/// netif/事件循环 → Wi-Fi init → 配网管理器 → 未配网走 BLE 配网 / 已配网走 STA 直连
pub fn start() {
    unsafe {
        // 注意：NVS 与默认事件循环由调用方（app_main）初始化

        // ---- TCP/IP 栈 + 默认事件循环 ----
        check(esp_netif_init(), "esp_netif_init");
        check(esp_event_loop_create_default(), "esp_event_loop_create_default");

        // ---- 订阅配网 / BLE 传输 / 安全会话事件（模拟构建跳过配网）----
        #[cfg(not(feature = "sim"))]
        {
            check(
                esp_event_handler_register(
                    NETWORK_PROV_EVENT,
                    ESP_EVENT_ANY_ID,
                    Some(event_handler),
                    ptr::null_mut(),
                ),
                "register NETWORK_PROV_EVENT",
            );
            check(
                esp_event_handler_register(
                    PROTOCOMM_TRANSPORT_BLE_EVENT,
                    ESP_EVENT_ANY_ID,
                    Some(event_handler),
                    ptr::null_mut(),
                ),
                "register PROTOCOMM_TRANSPORT_BLE_EVENT",
            );
            check(
                esp_event_handler_register(
                    PROTOCOMM_SECURITY_SESSION_EVENT,
                    ESP_EVENT_ANY_ID,
                    Some(event_handler),
                    ptr::null_mut(),
                ),
                "register PROTOCOMM_SECURITY_SESSION_EVENT",
            );
        }

        // ---- 订阅 IP 事件（配网与直连两条路径共用）----
        check(
            esp_event_handler_register(
                IP_EVENT,
                ip_event_t_IP_EVENT_STA_GOT_IP as i32,
                Some(event_handler),
                ptr::null_mut(),
            ),
            "register IP_EVENT",
        );

        // ---- Wi-Fi（BLE 配网传输层运行在 STA 模式）----
        esp_netif_create_default_wifi_sta();
        let wifi_cfg = wifi_init_config_default();
        check(esp_wifi_init(&wifi_cfg), "esp_wifi_init");

        // ---- 模拟构建：跳过 BLE 配网（模拟器不支持蓝牙），直连模拟 AP ----
        #[cfg(feature = "sim")]
        {
            ::log::warn!(
                "Simulation build (feature sim): BLE provisioning disabled, \
                 connecting to Wokwi-GUEST directly"
            );
            check(
                esp_event_handler_register(
                    WIFI_EVENT,
                    ESP_EVENT_ANY_ID,
                    Some(event_handler),
                    ptr::null_mut(),
                ),
                "register WIFI_EVENT",
            );
            check(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_STA), "set_mode");
            let mut wifi_cfg: wifi_config_t = std::mem::zeroed();
            wifi_cfg.sta.ssid[..b"Wokwi-GUEST".len()].copy_from_slice(b"Wokwi-GUEST");
            // password 保持全零 = 开放网络
            check(
                esp_wifi_set_config(wifi_interface_t_WIFI_IF_STA, &mut wifi_cfg),
                "set_config",
            );
            check(esp_wifi_start(), "esp_wifi_start");
        }

        // ---- 配网管理器配置：BLE GATT 传输 ----
        #[cfg(not(feature = "sim"))]
        {
            // C 宏 NETWORK_PROV_EVENT_HANDLER_* / *_FREE_BTDM 是复合字面量，
            // bindgen 不可见 → 手工构造同构结构体
            let mut prov_cfg: network_prov_mgr_config_t = std::mem::zeroed();
            prov_cfg.scheme = ptr::read(&raw const network_prov_scheme_ble);
            // 配网结束后释放 BT/BLE 内存（WakeManager 之后不需要蓝牙）
            prov_cfg.scheme_event_handler = network_prov_event_handler_t {
                event_cb: Some(network_prov_scheme_ble_event_cb_free_btdm),
                user_data: ptr::null_mut(),
            };
            prov_cfg.app_event_handler = network_prov_event_handler_t {
                event_cb: None,
                user_data: ptr::null_mut(),
            };
            check(network_prov_mgr_init(prov_cfg), "network_prov_mgr_init");

            let mut provisioned: bool = false;
            check(
                network_prov_mgr_is_wifi_provisioned(&mut provisioned),
                "network_prov_mgr_is_wifi_provisioned",
            );

            if !provisioned {
                ::log::info!("Device is not provisioned, starting BLE provisioning");

                let service_name = device_service_name().unwrap_or_else(|| {
                    std::ffi::CString::new(format!("{}000000", ac::PROV_DEVICE_NAME_PREFIX))
                        .expect("service name has no NUL")
                });

                // 可选自定义端点：先 create（start 之前），start 之后再 register
                let ep_name = c"custom-data";
                check(
                    network_prov_mgr_endpoint_create(ep_name.as_ptr()),
                    "endpoint_create",
                );

                // Security 1：X25519 密钥交换 + PoP + AES-CTR；pop 需在配网期间保持有效
                let pop = std::ffi::CString::new(ac::PROV_POP).expect("pop has no NUL");
                check(
                    network_prov_mgr_start_provisioning(
                        network_prov_security_NETWORK_PROV_SECURITY_1,
                        pop.as_ptr() as *const c_void,
                        service_name.as_ptr(),
                        ptr::null(),
                    ),
                    "start_provisioning",
                );
                check(
                    network_prov_mgr_endpoint_register(
                        ep_name.as_ptr(),
                        Some(custom_data_handler),
                        ptr::null_mut(),
                    ),
                    "endpoint_register",
                );

                prov_print_qr(
                    &service_name.to_string_lossy(),
                    ac::PROV_POP,
                );
            } else {
                ::log::info!("Device is already provisioned, starting Wi-Fi STA");

                // 不再需要配网管理器
                check(network_prov_mgr_deinit(), "network_prov_mgr_deinit");

                // 启用重连处理并连接到已存 AP
                check(
                    esp_event_handler_register(
                        WIFI_EVENT,
                        ESP_EVENT_ANY_ID,
                        Some(event_handler),
                        ptr::null_mut(),
                    ),
                    "register WIFI_EVENT",
                );
                check(esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_STA), "set_mode");
                check(esp_wifi_start(), "esp_wifi_start");
            }
        }
    }
}

/// 对应 C 版 `wifi_prov_wait_connected()`：阻塞直到拿到 IP
pub fn wait_connected() {
    ::log::info!("Waiting for IP address...");
    let mut connected = CONNECTED.lock().unwrap();
    while !*connected {
        connected = CONNECTED_CV.wait(connected).unwrap();
    }
}
