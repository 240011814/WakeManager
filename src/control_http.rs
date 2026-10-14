//! WakeManager - HTTP control server
//! 对应 C 版 `components/wol_http/control_http.c`
//!
//! 路由：
//!   GET  /              -> index.html（内嵌）
//!   GET  /api/status    -> JSON 状态
//!   GET  /api/config    -> JSON 配置
//!   POST /api/config    -> 保存配置（x-www-form-urlencoded）+ 重启 MQTT
//!   GET  /api/wake      -> 发送 magic packet（?mac= 必传，?ip= 可选）
//!   GET  /api/mac       -> MAC/IP 地址簿
//!   GET  /api/ota/check -> 触发 OTA 版本检查（异步，结果经 /api/status 轮询）
//!   POST /api/ota       -> 触发 OTA 升级下载（异步，完成后自动重启）
//!
//! 服务器由专属线程创建并持有（线程内 `EspHttpServer` 不跨线程移动，
//! 因此无需 `unsafe impl Send/Sync`）。

use esp_idf_svc::http::server::{
    Configuration, EspHttpConnection, EspHttpServer, Method, Request,
};
use esp_idf_svc::io::EspIOError;
use esp_idf_svc::sys::{EspError, ESP_FAIL};

use serde::Serialize;

use crate::app_config as ac;
use crate::config_store;
use crate::control_mqtt;
use crate::status;
use crate::wol;

/// 内嵌 Web UI（从 C 版 components/wol_http/index.html 迁移而来，随 crate 一起版本化）
static INDEX_HTML: &str = include_str!("index.html");

/// 保持服务器存活的线程栈大小。
const HTTPD_OWNER_STACK: usize = 8192;

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

/// 解析 urlencoded 键值串为 (key, decoded value) 列表
fn parse_params(text: &str) -> Vec<(String, String)> {
    text.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((k, v)) => (url_decode(k), url_decode(v)),
            None => (url_decode(pair), String::new()),
        })
        .collect()
}

/// 从请求 URI 中取出查询串并解析为参数列表。
///
/// esp-idf-svc 0.53 的 `Request` 只有 `uri()`（含查询串）、没有独立的
/// `query()`，因此这里统一封装查询串的切分与解码。
fn query_params(uri: &str) -> Vec<(String, String)> {
    match uri.split_once('?') {
        Some((_, q)) => parse_params(q),
        None => Vec::new(),
    }
}

fn get_param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// 等价 strlcpy 截断：最多 `cap - 1` 个**字节**，并对齐 UTF-8 边界。
fn truncate(s: &str, cap: usize) -> String {
    if cap == 0 {
        return String::new();
    }
    let max = cap - 1;
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
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

#[derive(Serialize)]
struct WakeResponse {
    ok: bool,
    mac: String,
    ip: String,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
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
    send_json(request, &status::device_status_json())
}

fn config_get(request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    let cfg = config_store::get();
    send_json(request, &status::config_json(&cfg))
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
    // 校验读满，避免用不完整的表单覆盖配置
    if body.len() != len {
        return send_err(request, 400, "Bad Request", "Missing body");
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
        cfg.mqtt_enable = matches!(v.chars().next(), Some('1' | 't' | 'T' | 'y' | 'Y'));
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

    let resp = serde_json::to_string(&OkResponse { ok: true }).unwrap_or_else(|_| "{}".to_string());
    send_json(request, &resp)
}

fn wake_get(request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    // uri() 含查询串（与 C 版 req->uri 一致）
    let params = query_params(request.uri());
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

    let resp = serde_json::to_string(&WakeResponse {
        ok: true,
        mac,
        ip,
    })
    .unwrap_or_else(|_| "{}".to_string());
    send_json(request, &resp)
}

fn mac_get(request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    send_json(request, &status::mac_list_http_json())
}

fn ota_check_get(request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    match crate::ota::start_check() {
        Ok(()) => send_json(request, r#"{"ok":true}"#),
        Err(e) => send_err(request, 409, "Conflict", &e),
    }
}

fn ota_post(request: Request<&mut EspHttpConnection<'_>>) -> Result<(), EspIOError> {
    match crate::ota::start_upgrade() {
        Ok(()) => send_json(request, r#"{"ok":true}"#),
        Err(e) => send_err(request, 409, "Conflict", &e),
    }
}

/* ---------------- server ---------------- */

fn build_server() -> Result<EspHttpServer<'static>, EspError> {
    let config = Configuration {
        http_port: ac::WOL_HTTP_PORT,
        lru_purge_enable: true,
        max_uri_handlers: 8,
        ..Default::default()
    };
    let mut server = EspHttpServer::new(&config).map_err(|EspIOError(e)| e)?;

    server.fn_handler("/", Method::Get, index_get)?;
    server.fn_handler("/api/status", Method::Get, status_get)?;
    server.fn_handler("/api/config", Method::Get, config_get)?;
    server.fn_handler("/api/config", Method::Post, config_post)?;
    server.fn_handler("/api/wake", Method::Get, wake_get)?;
    server.fn_handler("/api/mac", Method::Get, mac_get)?;
    server.fn_handler("/api/ota/check", Method::Get, ota_check_get)?;
    server.fn_handler("/api/ota", Method::Post, ota_post)?;

    Ok(server)
}

/// 对应 C 版 `control_http_start()`：在专属线程里启动并持有服务器。
pub fn start() -> Result<(), EspError> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), EspError>>();

    std::thread::Builder::new()
        .name("httpd_owner".into())
        .stack_size(HTTPD_OWNER_STACK)
        .spawn(move || match build_server() {
            Ok(server) => {
                log::info!("HTTP control server started on port {}", ac::WOL_HTTP_PORT);
                let _ = tx.send(Ok(()));
                // 服务器由本线程持有；线程不退出 → 服务器一直存活。
                let _server = server;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            }
            Err(e) => {
                log::error!("HTTP control server failed to start: {e:?}");
                let _ = tx.send(Err(e));
            }
        })
        .map_err(|_| EspError::from_infallible::<ESP_FAIL>())?;

    rx.recv()
        .unwrap_or(Err(EspError::from_infallible::<ESP_FAIL>()))
}