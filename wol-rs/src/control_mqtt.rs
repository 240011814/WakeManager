//! WakeManager - MQTT control client
//! 对应 C 版 `components/wol_mqtt/control_mqtt.c`
//!
//! 订阅：
//!   - broadcast（共享）：发现/status 查询
//!   - cmd/<id>：唤醒命令
//!   - mac/<id>、mac/get/<id>：地址簿管理/查询
//! 发布：
//!   - status：命令结果、发现应答、地址簿列表
//!
//! 生命周期用 esp-idf-svc 的 `EspMqttClient`（回调驱动）；
//! 回调内的 publish/subscribe 用裸 FFI（与 C 版在事件回调里
//! 调 `esp_mqtt_client_publish(client,...)` 的模式一致，client 指针
//! 存于 RAW 静态变量）。

use std::ffi::{c_char, CString};
use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock};

use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::mqtt::client::{
    Details, EspMqttClient, EspMqttEvent, EventPayload, MqttClientConfiguration,
};
use esp_idf_svc::sys::{
    esp_mqtt_client, esp_mqtt_client_publish, esp_mqtt_client_subscribe_single,
};

use crate::app_config as ac;
use crate::chip_temp;
use crate::config_store;
use crate::device_info;
use crate::led_status;
use crate::mac_store;
use crate::netinfo;
use crate::time_sync;
use crate::wol;

/// 回调内 FFI publish/subscribe 用的裸 client 句柄（new_cb 返回后写入；
/// 首个 CONNECTED 事件到达时必然已写入——TCP 握手远慢于 store）
static RAW: AtomicPtr<esp_mqtt_client> = AtomicPtr::new(std::ptr::null_mut());
static CONNECTED: AtomicBool = AtomicBool::new(false);
static CLIENT: Mutex<Option<EspMqttClient<'static>>> = Mutex::new(None);

/// 已解析的四个订阅主题（fixed base + "/" + device id）
struct Topics {
    bcast: CString,
    wake: CString,
    mac: CString,
    mac_get: CString,
    bcast_str: String,
    wake_str: String,
    mac_str: String,
    mac_get_str: String,
}

static TOPICS: OnceLock<Topics> = OnceLock::new();

/// 跨 MQTT_EVENT_DATA 分片的消息累积器（对应 C 版 s_cmd[256]）
static ACC: Mutex<(Vec<u8>, usize)> = Mutex::new((Vec::new(), 0));

fn check() -> Option<*mut esp_mqtt_client> {
    let raw = RAW.load(Ordering::SeqCst);
    (!raw.is_null()).then_some(raw)
}

/// 对应 C 版 publish_status()：发布原始字符串到 status 主题（QoS 1）
fn publish_status(msg: &str) {
    let Some(raw) = check() else { return };
    // MQTT 发送活动指示
    led_status::activity();
    let id = unsafe {
        esp_mqtt_client_publish(
            raw,
            c"wakemanager/status".as_ptr(),
            msg.as_ptr() as *const c_char,
            msg.len() as i32,
            1,
            0,
        )
    };
    ::log::info!(
        "Publishing to \"{}\": \"{}\" (msg_id={})",
        ac::WOL_MQTT_STATUS_TOPIC,
        msg,
        id
    );
}

/// 本设备状态快照（对应 C 版 publish_device_status，cJSON → serde_json）
fn publish_device_status() {
    let mut m = serde_json::Map::new();
    m.insert("id".into(), device_info::get_id().into());
    m.insert("ip".into(), netinfo::sta_ip().into());
    match netinfo::sta_ssid_rssi() {
        Some((ssid, rssi)) => {
            m.insert("ssid".into(), ssid.into());
            m.insert("rssi".into(), (rssi as i64).into());
        }
        None => {
            m.insert("ssid".into(), "".into());
            m.insert("rssi".into(), 0.into());
        }
    }
    m.insert("heap".into(), (netinfo::free_heap() as u64).into());
    m.insert("broadcast".into(), wol::get_default_broadcast().into());
    m.insert("mqtt_connected".into(), CONNECTED.load(Ordering::SeqCst).into());
    m.insert("mac_count".into(), (mac_store::count() as u64).into());
    if let Ok(c) = chip_temp::read() {
        m.insert("temp_c".into(), (c as f64).into());
    }
    if time_sync::is_synced() {
        m.insert("time".into(), time_sync::format_now().into());
    }
    publish_status(&serde_json::Value::Object(m).to_string());
}

/// 地址簿列表（对应 C 版 publish_mac_list）
fn publish_mac_list() {
    let table = mac_store::get();
    let mut m = serde_json::Map::new();
    m.insert("id".into(), device_info::get_id().into());
    m.insert("cmd".into(), "mac_list".into());
    m.insert("count".into(), (table.entries.len() as u64).into());
    let arr: Vec<serde_json::Value> = table
        .entries
        .iter()
        .map(|e| {
            serde_json::json!({ "mac": e.mac, "ip": e.ip })
        })
        .collect();
    m.insert("macs".into(), arr.into());
    publish_status(&serde_json::Value::Object(m).to_string());
}

/// JSON 命令对象（对应 C 版 parse_command：非字符串字段按缺省处理）
struct Command {
    name: String,
    mac: String,
    ip: String,
}

fn parse_command(text: &str) -> Option<Command> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let s = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    };
    Some(Command {
        name: s("cmd"),
        mac: s("mac"),
        ip: s("ip"),
    })
}

/// 对应 C 版 trim()：裁掉首尾空白
fn trim(s: &str) -> &str {
    s.trim()
}

/// 唤醒命令（cmd/<id>）：
/// `{"cmd":"wake","mac":"AA:BB:..","ip":"192.168.1.255"}`
fn handle_wake_command(cmd: &str) {
    let text = trim(cmd);
    if text.is_empty() {
        return;
    }

    let Some(c) = parse_command(text) else {
        ::log::warn!("Invalid JSON wake command: \"{cmd}\"");
        publish_status("error: invalid JSON command");
        return;
    };

    if !c.name.is_empty() && !c.name.eq_ignore_ascii_case("wake") {
        ::log::warn!("Command \"{}\" is not accepted on the wake topic", c.name);
        publish_status("error: unsupported command on wake topic");
        return;
    }

    if c.mac.is_empty() {
        ::log::warn!("Wake command has no MAC address: \"{cmd}\"");
        publish_status("error: no MAC address in command");
        return;
    }

    // 命令没带 IP → 查地址簿；再没有则用自动推导的广播地址
    let mut stored_ip = String::new();
    let target_ip: &str = if !c.ip.is_empty() {
        c.ip.as_str()
    } else if mac_store::get_ip(&c.mac)
        .map(|ip| {
            if !ip.is_empty() {
                stored_ip = ip;
                true
            } else {
                false
            }
        })
        .unwrap_or(false)
    {
        ::log::info!("Using stored IP {} for {}", stored_ip, c.mac);
        stored_ip.as_str()
    } else {
        ""
    };

    ::log::info!(
        "Wake request: mac={} ip={}",
        c.mac,
        if target_ip.is_empty() {
            "(default)"
        } else {
            target_ip
        }
    );

    match wol::send_str_to(&c.mac, Some(target_ip)) {
        Ok(()) => publish_status("ok: magic packet sent"),
        Err(wol::WolError::InvalidArg) => publish_status("error: invalid MAC or IP in command"),
        Err(_) => publish_status("error: failed to send magic packet"),
    }
}

/// 地址簿更新（mac/<id>）：
/// set/add/update、del/delete/remove、clear
fn handle_mac_set_command(cmd: &str) {
    let text = trim(cmd);
    if text.is_empty() {
        publish_status("error: empty command on mac topic");
        return;
    }
    let Some(c) = parse_command(text) else {
        ::log::warn!("Invalid JSON mac command: \"{cmd}\"");
        publish_status("error: invalid JSON command");
        return;
    };

    let n = c.name.to_lowercase();
    if n == "set" || n == "add" || n == "update" {
        if c.mac.is_empty() {
            publish_status("error: no MAC address in command");
            return;
        }
        match mac_store::set(&c.mac, &c.ip) {
            Ok(()) => publish_status("ok: mac stored"),
            Err(mac_store::MacError::Full) => publish_status("error: address book full"),
            Err(_) => publish_status("error: invalid MAC or IP"),
        }
    } else if n == "del" || n == "delete" || n == "remove" {
        if c.mac.is_empty() {
            publish_status("error: no MAC address in command");
            return;
        }
        let ok = mac_store::remove(&c.mac).is_ok();
        publish_status(if ok { "ok: mac removed" } else { "error: mac not found" });
    } else if n == "clear" {
        let ok = mac_store::clear().is_ok();
        publish_status(if ok {
            "ok: address book cleared"
        } else {
            "error: failed to clear address book"
        });
    } else {
        ::log::warn!("Unsupported command \"{}\" on mac topic", c.name);
        publish_status("error: unsupported command on mac topic");
    }
}

/// 地址簿查询（mac/get/<id>）：任意载荷（含空）都回列表
fn handle_mac_get_command(_cmd: &str) {
    ::log::info!("Address book requested");
    publish_mac_list();
}

/// 发现命令（共享 broadcast）：`{"cmd":"status"}`，空载荷也触发
fn handle_broadcast_command(cmd: &str) {
    let text = trim(cmd);
    if text.is_empty() {
        publish_device_status();
        return;
    }
    let Some(c) = parse_command(text) else {
        ::log::warn!("Invalid JSON broadcast command: \"{cmd}\"");
        publish_status("error: invalid JSON command");
        return;
    };

    if c.name.is_empty() || c.name.eq_ignore_ascii_case("status") {
        ::log::info!("Status requested on broadcast topic, announcing device status");
        publish_device_status();
    } else {
        ::log::warn!("Ignoring command \"{}\" on broadcast topic", c.name);
    }
}

/// 事件回调：连接/断开/数据分片累积/命令分发（对应 mqtt_event_handler）
fn handle_event(event: EspMqttEvent<'_>) {
    match event.payload() {
        EventPayload::Connected(_) => {
            CONNECTED.store(true, Ordering::SeqCst);

            // new_cb 内部已 start；首个 CONNECTED 到达时 RAW 通常已写入，
            // 兜底等待极短时间再订阅
            for _ in 0..100 {
                if check().is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let Some(raw) = check() else {
                ::log::error!("MQTT client handle unavailable, cannot subscribe");
                return;
            };
            let Some(topics) = TOPICS.get() else { return };
            for (label, t) in [
                ("broadcast", &topics.bcast),
                ("wake", &topics.wake),
                ("mac", &topics.mac),
                ("mac get", &topics.mac_get),
            ] {
                if t.to_bytes().is_empty() {
                    continue;
                }
                let id = unsafe { esp_mqtt_client_subscribe_single(raw, t.as_ptr(), 1) };
                ::log::info!(
                    "Subscribed to {} topic {} (msg_id={})",
                    label,
                    t.to_string_lossy(),
                    id
                );
            }
            publish_status("online");
        }

        EventPayload::Subscribed(id) => ::log::info!("Subscribed, msg_id={id}"),
        EventPayload::Unsubscribed(id) => ::log::info!("Unsubscribed, msg_id={id}"),
        EventPayload::Published(id) => ::log::debug!("Published, msg_id={id}"),
        EventPayload::Disconnected => {
            CONNECTED.store(false, Ordering::SeqCst);
            ::log::warn!("Disconnected from broker");
        }

        EventPayload::Received {
            topic, data, details, ..
        } => {
            // MQTT 接收活动指示
            led_status::activity();

            let topic = topic.unwrap_or("");
            ::log::info!(
                "Message received: topic=\"{}\" payload=\"{}\" ({} bytes)",
                topic,
                String::from_utf8_lossy(data),
                data.len()
            );

            let Some(topics) = TOPICS.get() else { return };
            let is_wake = topic == topics.wake_str;
            let is_bcast = topic == topics.bcast_str;
            let is_mac = topic == topics.mac_str;
            let is_mac_get = topic == topics.mac_get_str;
            if !is_wake && !is_bcast && !is_mac && !is_mac_get {
                return;
            }

            // 分片累积（对应 C 版 s_cmd[256]：offset 0 重置、255 字节上限、
            // 收满 total 后分发）
            let text = {
                let mut g = ACC.lock().unwrap();
                match details {
                    Details::InitialChunk(d) => {
                        g.0.clear();
                        g.1 = d.total_data_size;
                    }
                    Details::SubsequentChunk(d) => g.1 = d.total_data_size,
                    Details::Complete => g.1 = g.0.len() + data.len(),
                }
                let space = 255usize.saturating_sub(g.0.len());
                let copy = data.len().min(space);
                g.0.extend_from_slice(&data[..copy]);

                if g.0.len() >= g.1 {
                    let text = String::from_utf8_lossy(&g.0).into_owned();
                    g.0.clear();
                    g.1 = 0;
                    Some(text)
                } else {
                    None
                }
            };

            if let Some(text) = text {
                ::log::info!("Command received: \"{}\"", text);
                if is_wake {
                    handle_wake_command(&text);
                } else if is_mac {
                    handle_mac_set_command(&text);
                } else if is_mac_get {
                    handle_mac_get_command(&text);
                } else {
                    handle_broadcast_command(&text);
                }
            }
        }

        EventPayload::Error(e) => {
            CONNECTED.store(false, Ordering::SeqCst);
            ::log::error!("MQTT error: {e:?}");
        }

        _ => {}
    }
}

/// 对应 C 版 dns_debug_task()：一次性 DNS 诊断日志
fn dns_debug(uri: String) {
    let after = uri.split("://").nth(1).unwrap_or(uri.as_str());
    let authority = after.split('/').next().unwrap_or(after);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            (h, p.parse::<u16>().unwrap_or(1883))
        }
        _ => (authority, 1883u16),
    };

    match (host, port).to_socket_addrs() {
        Ok(addrs) => {
            for a in addrs {
                ::log::info!("DNS: {}:{} -> {}", host, port, a.ip());
            }
        }
        Err(e) => ::log::error!("DNS: cannot resolve {}:{}", host, e),
    }
}

#[derive(Debug)]
pub enum MqttStartError {
    NoUri,
    Init(esp_idf_svc::sys::EspError),
}

/// 对应 C 版 `control_mqtt_start()`
pub fn start() -> Result<(), MqttStartError> {
    let cfg = config_store::get();

    if !cfg.mqtt_enable {
        ::log::info!("MQTT disabled by configuration");
        return Ok(()); // 与 C 版一致：禁用不是错误
    }
    if cfg.mqtt_uri.is_empty() {
        ::log::warn!("MQTT enabled but no broker URI set");
        return Err(MqttStartError::NoUri);
    }

    // 解析固定主题；每设备主题追加 "/<device id>"
    let dev_id = device_info::get_id();
    let mk = |base: &str| -> (CString, String) {
        let s = format!("{base}/{dev_id}");
        (CString::new(s.clone()).expect("no NUL"), s)
    };
    let (wake, wake_str) = mk(ac::WOL_MQTT_CMD_TOPIC);
    let (mac, mac_str) = mk(ac::WOL_MQTT_MAC_TOPIC);
    let (mac_get, mac_get_str) = mk(ac::WOL_MQTT_MAC_GET_TOPIC);
    let bcast_str = ac::WOL_MQTT_BROADCAST_TOPIC.to_string();
    let _ = TOPICS.set(Topics {
        bcast: CString::new(bcast_str.clone()).expect("no NUL"),
        wake,
        mac,
        mac_get,
        bcast_str,
        wake_str,
        mac_str,
        mac_get_str,
    });

    // 诊断：日志打印 broker 域名解析结果
    std::thread::Builder::new()
        .name("mqtt_dns".into())
        .stack_size(3072)
        .spawn({
            let uri = cfg.mqtt_uri.clone();
            move || dns_debug(uri)
        })
        .ok();

    let conf = MqttClientConfiguration {
        client_id: Some(ac::WOL_MQTT_CLIENT_ID),
        username: (!cfg.mqtt_username.is_empty()).then_some(cfg.mqtt_username.as_str()),
        password: (!cfg.mqtt_password.is_empty()).then_some(cfg.mqtt_password.as_str()),
        ..Default::default()
    };

    let client = EspMqttClient::new_cb(&cfg.mqtt_uri, &conf, handle_event)
        .map_err(MqttStartError::Init)?;

    // 回调内的 FFI 发布用裸句柄（在 start 期间写入，早于任何 CONNECTED）
    RAW.store(client.handle(), Ordering::SeqCst);

    let topics = TOPICS.get().expect("topics just set");
    ::log::info!(
        "MQTT client starting, broker: {}, id: {}",
        cfg.mqtt_uri,
        dev_id
    );
    ::log::info!(
        "Topics: broadcast={} wake={} mac={} mac_get={} status={}",
        topics.bcast_str,
        topics.wake_str,
        topics.mac_str,
        topics.mac_get_str,
        ac::WOL_MQTT_STATUS_TOPIC
    );

    *CLIENT.lock().unwrap() = Some(client);
    Ok(())
}

/// 对应 C 版 `control_mqtt_stop()`
pub fn stop() {
    CONNECTED.store(false, Ordering::SeqCst);
    // take → drop → EspMqttClient::drop 内部调用 esp_mqtt_client_destroy
    if CLIENT.lock().unwrap().take().is_some() {
        ::log::info!("MQTT client stopped");
    }
    RAW.store(std::ptr::null_mut(), Ordering::SeqCst);
}

/// 对应 C 版 `control_mqtt_restart()`
pub fn restart() {
    stop();
    let _ = start();
}

/// 对应 C 版 `control_mqtt_is_connected()`
pub fn is_connected() -> bool {
    CONNECTED.load(Ordering::SeqCst)
}
