//! WakeManager - application entry point（Rust 版）
//! 对应 C 版 `main/app_main.c`
//!
//! 启动顺序与 C 版一致：
//!   chip info → LED → NVS → config → watchdog → 地址簿 → 温度 →
//!   Wi-Fi/配网 → 等 IP → SNTP → 订阅看门狗 → HTTP/MQTT → 主循环喂狗

mod app_config;
mod chip_temp;
mod config_store;
mod control_http;
mod control_mqtt;
mod debug_uart;
mod device_info;
mod led_status;
mod mac_store;
mod netinfo;
mod serial_cmd;
mod status;
mod time_sync;
mod touch;
mod watchdog;
mod wifi_prov;
mod wol;

use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::sys::*;

use app_config as ac;

/// 对应 esp_chip_info.h 中的 `CHIP_FEATURE_*` 宏（含移位表达式，bindgen 不翻译）
const CHIP_FEATURE_WIFI_BGN: u32 = 1 << 0;
const CHIP_FEATURE_BT: u32 = 1 << 1;
const CHIP_FEATURE_BLE: u32 = 1 << 2;
const CHIP_FEATURE_EMB_FLASH: u32 = 1 << 4;

/// 对应 C 版 print_chip_info()
fn print_chip_info() {
    unsafe {
        let mut chip_info: esp_chip_info_t = std::mem::zeroed();
        esp_chip_info(&mut chip_info);

        let feat = |bit: u32, s: &'static str| {
            if chip_info.features & bit != 0 {
                s
            } else {
                ""
            }
        };
        ::log::info!(
            "This is esp32s3 chip with {} CPU core(s), {}{}{}, silicon revision v{}.{}, ",
            chip_info.cores,
            feat(CHIP_FEATURE_WIFI_BGN, "WiFi/"),
            feat(CHIP_FEATURE_BT, "BT"),
            feat(CHIP_FEATURE_BLE, "BLE"),
            chip_info.revision / 100,
            chip_info.revision % 100
        );

        let mut flash_size: u32 = 0;
        if esp_flash_get_size(std::ptr::null_mut(), &mut flash_size) == ESP_OK {
            let flash_kind = if chip_info.features & CHIP_FEATURE_EMB_FLASH != 0 {
                "embedded"
            } else {
                "external"
            };
            ::log::info!(
                "{}MB {} flash",
                flash_size / (1024 * 1024),
                flash_kind
            );
        } else {
            ::log::info!("Get flash size failed");
        }

        ::log::info!(
            "Minimum free heap size: {} bytes",
            esp_get_free_heap_size()
        );
    }
}

fn main() {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly.
    // https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    EspLogger::initialize_default();
    ::log::info!("WakeManager (Rust)");

    // ---- 芯片 / flash 基本信息 ----
    print_chip_info();

    // ---- 状态指示：先慢闪，Wi-Fi 连上后常亮 ----
    led_status::init();
    led_status::set(led_status::Status::Connecting);

    // ---- 触摸：GPIO1 轻触闪灯；GPIO7 长按 20s 清 Wi-Fi 进配网 ----
    touch::start();

    // ---- NVS（Wi-Fi、配网与 config/mac store 都依赖它）
    // take() 内部完成 nvs_flash_init + 必要时 erase 重试（等价 C 版逻辑）
    let nvs = match EspDefaultNvsPartition::take() {
        Ok(n) => n,
        Err(e) => {
            ::log::error!("NVS init failed: {e:?}");
            std::process::abort();
        }
    };

    // ---- 运行时配置（NVS 存储值，否则 app_config.h 默认值）----
    if let Err(e) = config_store::init(nvs.clone()) {
        ::log::error!("config_store_init failed: {e:?}");
        std::process::abort();
    }

    // ---- 尽早启用任务看门狗，从启动开始捕获挂死 ----
    if watchdog::init().is_err() {
        ::log::warn!("Task watchdog could not be enabled (continuing)");
    }

    // ---- MAC/IP 地址簿（NVS）----
    if let Err(e) = mac_store::init(nvs.clone()) {
        ::log::error!("mac_store_init failed: {e:?}");
        std::process::abort();
    }

    // ---- 片内温度传感器（状态快照时按需读取）----
    if chip_temp::init().is_err() {
        ::log::warn!("Internal temperature sensor unavailable (continuing)");
    }

    debug_uart::start();

    // ---- Wi-Fi 与配网（已配网则直接 STA 连接）----
    wifi_prov::start();

    // ---- 阻塞直到拿到 IP ----
    wifi_prov::wait_connected();

    ::log::info!(
        "Free heap after Wi-Fi connected: {} bytes",
        netinfo::free_heap()
    );

    // ---- 网络就绪：SNTP 校时（非阻塞；句柄全程保活，Drop 会停掉 SNTP）----
    let _sntp = time_sync::start();

    // ---- 从现在起主任务必须持续喂狗 ----
    if watchdog::add_current_task().is_err() {
        ::log::warn!("Could not subscribe main task to watchdog");
    }

    // ---- 触发 Wake-on-LAN 的控制接口 ----
    if ac::WOL_HTTP_ENABLE {
        if let Err(e) = control_http::start() {
            ::log::error!("HTTP control failed to start: {e:?}");
            std::process::abort();
        }
    }
    if ac::WOL_MQTT_ENABLE {
        if let Err(e) = control_mqtt::start() {
            ::log::warn!("MQTT control failed to start: {e:?} (continuing)");
        }
    }

    ::log::info!("WakeManager ready");

    // ---- 主任务空转，定期喂狗；工作在 HTTP/MQTT 回调与 LED 线程里 ----
    loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
        watchdog::feed();
    }
}
