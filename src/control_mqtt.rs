//! WakeManager - MQTT control client
//! 对应 C 版 `components/wol_mqtt/control_mqtt.c`
//!
//! 订阅：
//!   - broadcast（共享）：发现/status 查询
//!   - cmd/<id>：唤醒命令
//!   - mac/<id>、mac/get/<id>：地址簿管理/查询
//!   - ota/<id>：OTA 升级命令（check / upgrade / status）
//! 发布：
//!   - status：命令结果、发现应答、地址簿列表
//!   - csi/notify：CSI 近场感知状态变化通知（由 csi_presence 触发）
//!
//! 生命周期用 esp-idf-svc 的 `EspMqttClient`：`EspMqttClient::new` 返回
//! `(client, connection)`，由一个专属线程持有 `connection` 并用阻塞的
//! `connection.next()` 驱动事件循环。
//!
//! **线程模型（重要）**：espressif/mqtt 1.1.0 起任务循环在 API 锁内分发事件
//! （`MQTT_API_LOCK → run_event_loop → dispatch → 回调 share() 阻塞`），
//! 因此**事件处理路径上禁止调用任何 client API**（subscribe/publish 会与
//! mqtt 任务互相等待而死锁）。所有 subscribe/publish 都入队给专属的
//! `mqtt_api` 工作线程执行；事件线程只解析命令并入队。

use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;

use esp_idf_svc::mqtt::client::{
    Details, EspMqttClient, EspMqttConnection, EspMqttEvent, EventPayload,
    MqttClientConfiguration, QoS,
};

use crate::app_config as ac;
use crate::config_store;
use crate::device_info;
use crate::led_status;
use crate::mac_store;
use crate::status;
use crate::wol;

static CONNECTED: AtomicBool = AtomicBool::new(false);

/// API 工作线程的入队口（None = 客户端未启动/已停止）
static API_TX: Mutex<Option<mpsc::Sender<ApiOp>>> = Mutex::new(None);
/// API 工作线程，`stop()` 时先关队列再 join（线程退出时 drop client，
/// 从而关闭事件通道让事件线程退出）
static WORKER_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
/// 事件循环线程，`stop()` 时 join。
static EVENT_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

/// 已解析的订阅主题（fixed base + "/" + device id）
struct Topics {
    bcast: String,
    wake: String,
    mac: String,
    mac_get: String,
    ota: String,
}

static TOPICS: OnceLock<Topics> = OnceLock::new();

/// 跨 MQTT_EVENT_DATA 分片的消息累积器（对应 C 版 s_cmd[256]）
static ACC: Mutex<(Vec<u8>, usize)> = Mutex::new((Vec::new(), 0));

/// 命令缓冲上限（对应 C 版 s_cmd[256]：255 字节 + NUL）
const MAX_CMD_LEN: usize = 255;

/// API 工作线程的操作队列（在事件处理路径外执行 client API）
enum ApiOp {
    SubscribeAll,
    Publish(String),
    /// CSI 近场感知通知（csi/notify 主题）
    Notify(String),
}

/// 对应 C 版 publish_status()：发布原始字符串到 status 主题（QoS 1）。
/// 经 API 工作线程执行（事件处理路径不可直接调用 client API）。
fn publish_status(msg: &str) {
    // MQTT 发送活动指示
    led_status::activity();

    let tx = API_TX.lock().unwrap();
    let Some(tx) = tx.as_ref() else {
        ::log::warn!("MQTT publish skipped (client not started): {msg}");
        return;
    };
    if tx.send(ApiOp::Publish(msg.to_string())).is_err() {
        ::log::warn!("MQTT publish skipped (worker gone): {msg}");
    }
}

/// CSI 近场感知通知（`csi/notify` 主题，QoS 1）：状态变化（1 米内有人↔无人）
/// 时由 `csi_presence` 发布。经 API 工作线程执行（事件处理路径不可直接调用
/// client API）。
pub fn publish_notify(msg: &str) {
    // MQTT 发送活动指示
    led_status::activity();

    let tx = API_TX.lock().unwrap();
    let Some(tx) = tx.as_ref() else {
        ::log::warn!("MQTT notify skipped (client not started): {msg}");
        return;
    };
    if tx.send(ApiOp::Notify(msg.to_string())).is_err() {
        ::log::warn!("MQTT notify skipped (worker gone): {msg}");
    }
}

/// 本设备状态快照（对应 C 版 publish_device_status；JSON 由 `status` 模块构造）
fn publish_device_status() {
    publish_status(&status::device_status_json());
}

/// 地址簿列表（对应 C 版 publish_mac_list）
fn publish_mac_list() {
    publish_status(&status::mac_list_mqtt_json());
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

    // 命令没带 IP → 查地址簿（只取非空记录）；再没有则 None，
    // 由 wol::send_to 退回自动推导的子网定向广播地址
    let target_ip = if !c.ip.is_empty() {
        Some(c.ip.clone())
    } else {
        let stored = mac_store::get_ip(&c.mac).filter(|ip| !ip.is_empty());
        if let Some(ip) = &stored {
            ::log::info!("Using stored IP {} for {}", ip, c.mac);
        }
        stored
    };

    ::log::info!(
        "Wake request: mac={} ip={}",
        c.mac,
        target_ip.as_deref().unwrap_or("(default)")
    );

    match wol::send_str_to(&c.mac, target_ip.as_deref()) {
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

/// OTA 命令（ota/<id>）：`{"cmd":"check"}` / `{"cmd":"upgrade"}` / `{"cmd":"status"}`
/// check/upgrade 只触发（检查/下载在 OTA 专属线程跑），结果与进度经
/// status 主题的 `{"cmd":"status"}` 应答或设备状态快照里的 ota 字段回传
fn handle_ota_command(cmd: &str) {
    let text = trim(cmd);
    if text.is_empty() {
        publish_status("error: empty command on ota topic");
        return;
    }
    let Some(c) = parse_command(text) else {
        ::log::warn!("Invalid JSON ota command: \"{cmd}\"");
        publish_status("error: invalid JSON command");
        return;
    };

    let n = c.name.to_lowercase();
    if n == "check" {
        match crate::ota::start_check() {
            Ok(()) => publish_status("ok: version check started"),
            Err(e) => publish_status(&format!("error: {e}")),
        }
    } else if n == "upgrade" {
        match crate::ota::start_upgrade() {
            Ok(()) => publish_status("ok: upgrade started, device will reboot when done"),
            Err(e) => publish_status(&format!("error: {e}")),
        }
    } else if n == "status" {
        ::log::info!("OTA status requested on ota topic");
        publish_device_status();
    } else {
        ::log::warn!("Unsupported command \"{}\" on ota topic", c.name);
        publish_status("error: unsupported command on ota topic");
    }
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

/// 事件回调（在事件线程中调用，对应 C 版 mqtt_event_handler）。
/// 注意：此路径上禁止直接调用 client API（死锁），一律经 API_TX 入队。
fn handle_event(event: &EspMqttEvent<'_>) {
    match event.payload() {
        EventPayload::Connected(_) => {
            CONNECTED.store(true, Ordering::SeqCst);

            // 订阅四个主题 + 上线通告：全部交 API 工作线程执行
            if let Some(tx) = API_TX.lock().unwrap().as_ref() {
                let _ = tx.send(ApiOp::SubscribeAll);
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
            let is_wake = topic == topics.wake.as_str();
            let is_bcast = topic == topics.bcast.as_str();
            let is_mac = topic == topics.mac.as_str();
            let is_mac_get = topic == topics.mac_get.as_str();
            let is_ota = topic == topics.ota.as_str();
            if !is_wake && !is_bcast && !is_mac && !is_mac_get && !is_ota {
                return;
            }

            // 分片累积（对应 C 版 s_cmd[256]：offset 0 重置、255 字节上限、
            // 收满 total 后分发；超限消息整体丢弃并清空残留）
            let text = {
                let mut g = ACC.lock().unwrap();
                match details {
                    Details::InitialChunk(d) => {
                        g.0.clear();
                        g.1 = d.total_data_size;
                    }
                    Details::SubsequentChunk(d) => g.1 = d.total_data_size,
                    Details::Complete => {
                        // 完整消息单事件到达：先清残留，防止与上一条未分发
                        // 的数据拼接
                        g.0.clear();
                        g.1 = data.len();
                    }
                }

                if g.1 > MAX_CMD_LEN {
                    // 超限：丢弃整条消息。同一消息的后续分片没有 InitialChunk，
                    // 但 SubsequentChunk 会再次带来超限的 total，同样在此丢弃。
                    ::log::warn!(
                        "MQTT message of {} bytes exceeds {}-byte command limit, dropped",
                        g.1,
                        MAX_CMD_LEN
                    );
                    g.0.clear();
                    g.1 = 0;
                    None
                } else {
                    let space = MAX_CMD_LEN.saturating_sub(g.0.len());
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
                } else if is_ota {
                    handle_ota_command(&text);
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

/// 事件循环线程主体：阻塞等待事件直到通道关闭（client 被 drop）。
fn mqtt_event_loop(mut connection: EspMqttConnection) {
    while let Ok(event) = connection.next() {
        handle_event(event);
    }
    CONNECTED.store(false, Ordering::SeqCst);
    ::log::info!("MQTT event loop terminated");
}

/// API 工作线程：独占 client，串行执行所有 subscribe/publish。
/// 队列关闭（发送端 drop）后退出，client 随之 drop 并关闭事件通道。
fn api_worker(mut client: EspMqttClient<'static>, rx: mpsc::Receiver<ApiOp>) {
    while let Ok(op) = rx.recv() {
        match op {
            ApiOp::SubscribeAll => {
                let Some(topics) = TOPICS.get() else { continue };
                for (label, t) in [
                    ("broadcast", &topics.bcast),
                    ("wake", &topics.wake),
                    ("mac", &topics.mac),
                    ("mac get", &topics.mac_get),
                    ("ota", &topics.ota),
                ] {
                    match client.subscribe(t, QoS::AtLeastOnce) {
                        Ok(id) => ::log::info!("Subscribed to {label} topic {t} (msg_id={id})"),
                        Err(e) => {
                            ::log::error!("Failed to subscribe to {label} topic {t}: {e:?}")
                        }
                    }
                }
            }
            ApiOp::Publish(msg) => publish_msg(&mut client, ac::WOL_MQTT_STATUS_TOPIC, &msg),
            ApiOp::Notify(msg) => publish_msg(&mut client, ac::WOL_MQTT_CSI_NOTIFY_TOPIC, &msg),
        }
    }
    ::log::info!("MQTT API worker terminated");
}

/// 在 API 工作线程内执行一次 publish（QoS 1、不保留）
fn publish_msg(client: &mut EspMqttClient<'static>, topic: &str, msg: &str) {
    match client.publish(topic, QoS::AtLeastOnce, false, msg.as_bytes()) {
        Ok(id) => ::log::info!("Publishing to \"{topic}\": \"{msg}\" (msg_id={id})"),
        Err(e) => ::log::error!("Failed to publish to \"{topic}\": {e:?}"),
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
#[allow(dead_code)] // 字段随 Debug 输出使用
pub enum MqttStartError {
    NoUri,
    Init(esp_idf_svc::sys::EspError),
    Thread(std::io::Error),
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
    let _ = TOPICS.set(Topics {
        bcast: ac::WOL_MQTT_BROADCAST_TOPIC.to_string(),
        wake: format!("{}/{}", ac::WOL_MQTT_CMD_TOPIC, dev_id),
        mac: format!("{}/{}", ac::WOL_MQTT_MAC_TOPIC, dev_id),
        mac_get: format!("{}/{}", ac::WOL_MQTT_MAC_GET_TOPIC, dev_id),
        ota: format!("{}/{}", ac::WOL_MQTT_OTA_TOPIC, dev_id),
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

    // client id 追加设备 id：broker 对相同 client id 会挤掉旧连接，固定 ID
    // 会让多台设备（或同名测试客户端）互踢，发现时好时坏
    let client_id = format!("{}-{}", ac::WOL_MQTT_CLIENT_ID, dev_id);
    let conf = MqttClientConfiguration {
        client_id: Some(client_id.as_str()),
        username: (!cfg.mqtt_username.is_empty()).then_some(cfg.mqtt_username.as_str()),
        password: (!cfg.mqtt_password.is_empty()).then_some(cfg.mqtt_password.as_str()),
        ..Default::default()
    };

    let (client, connection) =
        EspMqttClient::new(&cfg.mqtt_uri, &conf).map_err(MqttStartError::Init)?;

    // API 工作线程独占 client（见模块注释：事件路径禁止直接调用 client API）
    let (api_tx, api_rx) = mpsc::channel::<ApiOp>();
    *API_TX.lock().unwrap() = Some(api_tx);
    let worker = std::thread::Builder::new()
        .name("mqtt_api".into())
        .stack_size(6144)
        .spawn(move || api_worker(client, api_rx))
        .map_err(MqttStartError::Thread)?;
    *WORKER_THREAD.lock().unwrap() = Some(worker);

    // 事件处理含 serde_json 解析/序列化 + 大 FFI 栈结构（wifi_ap_record_t 约
    // 800B）+ localtime_r 等，4KB 会顶穿栈金丝雀导致整机遇测复位；旧实现跑在
    // esp-mqtt 任务（默认 6144）上，这里给到 8192 留足余量
    let handle = std::thread::Builder::new()
        .name("mqtt_events".into())
        .stack_size(8192)
        .spawn(move || mqtt_event_loop(connection))
        .map_err(MqttStartError::Thread)?;
    *EVENT_THREAD.lock().unwrap() = Some(handle);

    ::log::info!("MQTT client starting, broker: {}, id: {}", cfg.mqtt_uri, dev_id);
    if let Some(t) = TOPICS.get() {
        ::log::info!(
            "Topics: broadcast={} wake={} mac={} mac_get={} ota={} status={}",
            t.bcast,
            t.wake,
            t.mac,
            t.mac_get,
            t.ota,
            ac::WOL_MQTT_STATUS_TOPIC
        );
    }
    Ok(())
}

/// 对应 C 版 `control_mqtt_stop()`
pub fn stop() {
    CONNECTED.store(false, Ordering::SeqCst);

    // 关闭 API 队列 → 工作线程退出并 drop client → 事件通道关闭 →
    // 事件线程退出；最后 join 两个线程
    API_TX.lock().unwrap().take();
    if let Some(handle) = WORKER_THREAD.lock().unwrap().take() {
        let _ = handle.join();
    }
    if let Some(handle) = EVENT_THREAD.lock().unwrap().take() {
        let _ = handle.join();
    }
    ::log::info!("MQTT client stopped");
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
