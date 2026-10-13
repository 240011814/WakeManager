//! WakeManager - JSON 状态 / 配置 / 地址簿视图（`serde` 派生结构体）
//!
//! 把原先手写 `serde_json::Map` 的构造集中到这里：字段顺序与 C 版一致，
//! 可空字段（`temp_c` / `time`）在缺失时整体省略（对应 C 的按需追加）。

use serde::Serialize;

use crate::device_info;
use crate::mac_store;
use crate::netinfo;
use crate::time_sync;
use crate::wol;

/// 把派生结构体序列化为 JSON 字符串（对这些结构体序列化不会失败）。
fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}

/// 设备状态快照；HTTP `/api/status` 与 MQTT 状态发布共用同一形状。
#[derive(Serialize)]
pub struct DeviceStatus {
    pub id: String,
    pub ip: String,
    pub ssid: String,
    pub rssi: i8,
    pub heap: u32,
    pub mqtt_connected: bool,
    pub broadcast: String,
    pub mac_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_c: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
}

/// 设备状态快照；HTTP `/api/status`、MQTT 状态发布与串口 `status` 命令共用。
pub fn device_status() -> DeviceStatus {
    let (ssid, rssi) = netinfo::sta_ssid_rssi().unwrap_or_default();
    DeviceStatus {
        id: device_info::get_id(),
        ip: netinfo::sta_ip(),
        ssid,
        rssi,
        heap: netinfo::free_heap(),
        mqtt_connected: crate::control_mqtt::is_connected(),
        broadcast: wol::get_default_broadcast(),
        mac_count: mac_store::count(),
        temp_c: crate::chip_temp::read().ok(),
        time: time_sync::is_synced().then(time_sync::format_now),
    }
}

pub fn device_status_json() -> String {
    to_json(&device_status())
}

#[derive(Serialize)]
pub struct MacEntry {
    pub mac: String,
    pub ip: String,
}

/// 地址簿列表。`id`/`cmd` 仅 MQTT 版本使用（为空时省略）。
#[derive(Serialize)]
pub struct MacList {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmd: Option<&'static str>,
    pub count: usize,
    pub macs: Vec<MacEntry>,
}

fn mac_list(id: Option<String>, cmd: Option<&'static str>) -> MacList {
    let table = mac_store::get();
    MacList {
        id,
        cmd,
        count: table.entries.len(),
        macs: table
            .entries
            .into_iter()
            .map(|e| MacEntry { mac: e.mac, ip: e.ip })
            .collect(),
    }
}

/// `GET /api/mac`：`{"count":n,"macs":[...]}`
pub fn mac_list_http_json() -> String {
    to_json(&mac_list(None, None))
}

/// MQTT `mac_list` 发布：`{"id":..,"cmd":"mac_list","count":n,"macs":[...]}`
pub fn mac_list_mqtt_json() -> String {
    to_json(&mac_list(Some(device_info::get_id()), Some("mac_list")))
}

/// `GET /api/config`：运行配置视图。
#[derive(Serialize)]
pub struct ConfigView<'a> {
    pub wol_udp_port: u16,
    pub mqtt_enable: bool,
    pub mqtt_uri: &'a str,
    pub mqtt_username: &'a str,
    pub mqtt_password: &'a str,
}

pub fn config_json(cfg: &crate::config_store::WmConfig) -> String {
    to_json(&ConfigView {
        wol_udp_port: cfg.wol_udp_port,
        mqtt_enable: cfg.mqtt_enable,
        mqtt_uri: &cfg.mqtt_uri,
        mqtt_username: &cfg.mqtt_username,
        mqtt_password: &cfg.mqtt_password,
    })
}