//! WakeManager - HTTP control server
//! 对应 C 版 `components/wol_http/control_http.c`
//!
//! 路由：
//!   GET  /              -> index.html（内嵌，来自 C 版同文件）
//!   GET  /api/status    -> JSON 状态
//!   GET  /api/config    -> JSON 配置
//!   POST /api/config    -> 保存配置（x-www-form-urlencoded）+ 重启 MQTT
//!   GET  /api/wake      -> 发送 magic packet（可带 ?mac= ?ip=）
//!   GET  /api/mac       -> MAC/IP 地址簿

use std::sync::Mutex;

use esp_idf_svc::http::server::{
    Configuration, EspHttpConnection, EspHttpServer, Method, Request,
};
use esp_idf_svc::io::EspIOError;

use crate::app_config as ac;
use crate::chip_temp;
use crate::config_store;
use crate::control_mqtt;
use crate::device_info;
use crate::mac_store;
use crate::netinfo;
use crate::time_sync;
use crate::wol;

/// 内嵌 Web UI（与 C 版 components/wol_http/index.html 同一文件）
static INDEX_HTML: &str = include_str!("../../components/wol_http/index.html");

/// HTTP 服务保持存活（httpd 任务在注册的闭包里运行）。
/// EspHttpServer 内含裸指针（非 Send/Sync），用新类型包装：
/// 所有访问都在 Mutex 下串行化，httpd 内部自身线程安全。
struct ServerHolder(EspHttpServer<'static>);
unsafe impl Send for ServerHolder {}
unsafe impl Sync for ServerHolder {}

static SERVER: Mutex<Option<ServerHolder>> = Mutex::new(None);

/* ---------------- small helpers ---------------- */

/// %XX / '+' 解码（对应 C 版 url_decode）
fn url_decode(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len()
                && bytes[i + 1].is_ascii_hexdigit()
                && bytes[i + 2].is_ascii_hexdigit() =>
            {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'%'));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 解析 urlencoded 查询串 / 表单体为 (key, decoded value) 列表
fn parse_params(text: &str) -> Vec<(String, String)> {
    text.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (url_decode(k), url_decode(v)),
            None => (url_decode(pair), String::new()),
        })
        .collect()
}

fn get_param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// 等价 strlcpy 截断
fn truncate(s: &str, cap: usize) -> String {
    s.chars().take(cap - 1).collect()
}

fn respond(
    request: Request<&mut EspHttpConnection<'_>>,
    status: u16,
    message: Option<&str>,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Result<(), EspIOError> {
    let mut resp = request.into_response(status, message, headers)?;
    let mut off = 0;
    while off < body.len() {
        let n = resp.write(&body[off..])?;
        if n == 0 {
            break;
        }
        off += n;
    }
    Ok(())
}

fn send_json(request: Request<&mut EspHttpConnection<'_>>, json: &str) -> Result<(), EspIOError> {
    respond(
        request,
        200,
        None,
        &[
            ("Content-Type", "application/json"),
            ("Cache-Control", "no-store"),
        ],
        json.as_bytes(),
    )
}

fn send_err(
    request: Request<&mut EspHttpConnection<'_>>,
    status: u16,
    message: &str,
    body: &str,
) -> Result<(), EspIOError> {
    respond(
        request,
        status,
        Some(message),
        &[("Content-Type", "text/plain")],
        body.as_bytes(),
    )
}

/// 对应 C 版 status_get_handler 的 JSON 构造
fn status_json() -> String {
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
    m.insert("mqtt_connected".into(), control_mqtt::is_connected().into());
    m.insert("broadcast".into(), wol::get_default_broadcast().into());
    m.insert("mac_count".into(), (mac_store::count() as u64).into());
    if let Ok(c) = chip_temp::read() {
        m.insert("temp_c".into(), (c as f64).into());
    }
    if time_sync::is_synced() {
        m.insert("time".into(), time_sync::format_now().into());
    }
    serde_json::Value::Object(m).to_string()
}

/* ---------------- handlers ---------------- */

fn index_get(request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    respond(
        request,
        200,
        None,
        &[("Content-Type", "text/html")],
        INDEX_HTML.as_bytes(),
    )
}

fn status_get(request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    send_json(request, &status_json())
}

fn config_get(request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    let cfg = config_store::get();
    let m = serde_json::json!({
        "wol_udp_port": cfg.wol_udp_port,
        "mqtt_enable": cfg.mqtt_enable,
        "mqtt_uri": cfg.mqtt_uri,
        "mqtt_username": cfg.mqtt_username,
        "mqtt_password": cfg.mqtt_password,
    });
    send_json(request, &m.to_string())
}

fn config_post(mut request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    // 读取 body（对应 C 版 read_body：content_len 必须 (0, 4096]）
    let len: usize = request
        .header("Content-Length")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    if len == 0 || len > 4096 {
        return send_err(request, 400, "Bad Request", "Missing body");
    }

    let mut body = Vec::with_capacity(len);
    let mut buf = [0u8; 512];
    while body.len() < len {
        match request.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&buf[..n]),
            Err(_) => return send_err(request, 400, "Bad Request", "Missing body"),
        }
    }
    let body = String::from_utf8_lossy(&body).into_owned();
    let params = parse_params(&body);

    let mut cfg = config_store::get();

    if let Some(v) = get_param(&params, "wol_udp_port") {
        let p: i64 = v.parse().unwrap_or(0);
        if p <= 0 || p > 65535 {
            return send_err(request, 400, "Bad Request", "Invalid UDP port");
        }
        cfg.wol_udp_port = p as u16;
    }
    if let Some(v) = get_param(&params, "mqtt_enable") {
        cfg.mqtt_enable = matches!(
            v.chars().next(),
            Some('1' | 't' | 'T' | 'y' | 'Y')
        );
    }
    if let Some(v) = get_param(&params, "mqtt_uri") {
        cfg.mqtt_uri = truncate(v, ac::WM_CFG_HOST_LEN);
    }
    if let Some(v) = get_param(&params, "mqtt_username") {
        cfg.mqtt_username = truncate(v, ac::WM_CFG_USER_LEN);
    }
    if let Some(v) = get_param(&params, "mqtt_password") {
        cfg.mqtt_password = truncate(v, ac::WM_CFG_PASS_LEN);
    }

    if let Err(e) = config_store::save(&cfg) {
        log::error!("Failed to save config: {e:?}");
        return send_err(request, 500, "Internal Server Error", "Failed to save config");
    }

    // 立即应用 MQTT 变更
    control_mqtt::restart();

    send_json(request, "{\"ok\":true}")
}

fn wake_get(request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    // uri() 含查询串（与 C 版 req->uri 一致）
    let params: Vec<(String, String)> = match request.uri().split_once('?') {
        Some((_, q)) => parse_params(q),
        None => Vec::new(),
    };
    // 对应 C 版缓冲区上限：mac 17 字符 / ip 15 字符
    let mac_str = get_param(&params, "mac").map(|v| truncate(v, ac::WM_CFG_MAC_LEN));
    let ip_str = get_param(&params, "ip").map(|v| truncate(v, ac::WM_MAC_IP_LEN));

    let Some(mac) = mac_str.filter(|m| !m.is_empty()) else {
        return send_err(request, 400, "Bad Request", "Missing mac parameter");
    };

    let ip = ip_str.unwrap_or_default();
    if wol::send_str_to(&mac, Some(&ip)).is_err() {
        return send_err(request, 400, "Bad Request", "Failed to send magic packet");
    }

    let resp = serde_json::json!({ "ok": true, "mac": mac, "ip": ip });
    send_json(request, &resp.to_string())
}

fn mac_get(request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    let table = mac_store::get();
    let m = serde_json::json!({
        "count": table.entries.len(),
        "macs": table
            .entries
            .iter()
            .map(|e| serde_json::json!({ "mac": e.mac, "ip": e.ip }))
            .collect::<Vec<_>>(),
    });
    send_json(request, &m.to_string())
}

/// 对应 C 版 `control_http_start()`
pub fn start() -> Result<(), esp_idf_svc::sys::EspError> {
    let config = Configuration {
        http_port: ac::WOL_HTTP_PORT,
        lru_purge_enable: true,
        max_uri_handlers: 6,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&config)
        .map_err(|esp_idf_svc::io::EspIOError(e)| e)?;

    server.fn_handler("/", Method::Get, index_get)?;
    server.fn_handler("/api/status", Method::Get, status_get)?;
    server.fn_handler("/api/config", Method::Get, config_get)?;
    server.fn_handler("/api/config", Method::Post, config_post)?;
    server.fn_handler("/api/wake", Method::Get, wake_get)?;
    server.fn_handler("/api/mac", Method::Get, mac_get)?;

    log::info!(
        "HTTP control server started on port {}",
        ac::WOL_HTTP_PORT
    );

    *SERVER.lock().unwrap() = Some(ServerHolder(server));
    Ok(())
}

/// 对应 C 版 `control_http_stop()`
pub fn stop() {
    if SERVER.lock().unwrap().take().is_some() {
        log::info!("HTTP control server stopped");
    }
}

/// 状态指示：HTTP 已启动（供主流程日志使用）
pub fn is_running() -> bool {
    SERVER.lock().unwrap().is_some()
}
