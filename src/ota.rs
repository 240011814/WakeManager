//! WakeManager - OTA 固件升级（从 GitHub Releases 拉取更新）
//!
//! 流程：
//!   check()          -> GET releases/latest API，取 tag 与 wol-rs.bin 资产 URL，
//!                       与本机版本（app_config::VERSION）比对；
//!   start_upgrade()  -> 专属线程流式下载（跟随 GitHub 302 重定向）->
//!                       esp_ota 写入非活动槽 -> 校验镜像 -> 切换启动分区 -> 重启。
//!
//! 状态机存全局（Mutex），HTTP `/api/ota/*` 触发、`/api/status` 轮询读取；
//! 检查 / 升级都在独立线程执行（TLS 握手栈开销大，不能跑在 httpd 线程上），
//! 进行中拒绝重复触发。

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use esp_idf_svc::http::client::{
    Configuration as HttpConfiguration, EspHttpConnection, FollowRedirectsPolicy, Method,
};
use esp_idf_svc::ota::EspOta;
use esp_idf_svc::sys::esp_crt_bundle_attach;

use serde::{Deserialize, Serialize};

use crate::app_config as ac;

/// 检查/下载线程栈大小（mbedtls TLS 握手需要较大栈）
const OTA_THREAD_STACK: usize = 32 * 1024;
/// HTTP 读缓冲（下载分片）
const DL_BUF_SIZE: usize = 4096;
/// 版本检查响应体上限（releases/latest JSON 通常 < 8KB，防跑飞）
const CHECK_BODY_MAX: usize = 32 * 1024;

/* ---------------- 状态机 ---------------- */

#[derive(Clone, Serialize)]
pub struct OtaStatus {
    /// idle / checking / update_available / downloading / rebooting / failed
    pub state: &'static str,
    pub current: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    /// 下载进度 0-100（downloading 时有值）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl OtaStatus {
    fn new() -> Self {
        Self {
            state: "idle",
            current: ac::VERSION.to_string(),
            latest: None,
            progress: None,
            error: None,
        }
    }
}

fn status_cell() -> &'static Mutex<OtaStatus> {
    static CELL: OnceLock<Mutex<OtaStatus>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(OtaStatus::new()))
}

/// check() 找到的最新固件下载地址（供 start_upgrade 使用）
fn latest_url_cell() -> &'static Mutex<Option<String>> {
    static CELL: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

pub fn status() -> OtaStatus {
    status_cell()
        .lock()
        .map(|s| s.clone())
        // Mutex 中毒时不 panic，返回初始状态
        .unwrap_or_else(|_| OtaStatus::new())
}

fn set_state(state: &'static str, latest: Option<String>, progress: Option<u32>, error: Option<String>) {
    if let Ok(mut s) = status_cell().lock() {
        s.state = state;
        s.latest = latest;
        s.progress = progress;
        s.error = error;
    }
}

/* ---------------- GitHub API 响应 ---------------- */

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    /// Release 目标 commit（创建草稿时由 CI 传入 github.sha）——版本检查
    /// 只靠 API 响应字段，不下载任何资产（资产域名国内直连不稳）
    target_commitish: String,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// commit 短 SHA（7 位，与 CI 注入的 WOL_VERSION 格式一致）
fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// 版本比对：短 SHA 不同即视为有更新（commit 顺序无法本地比较，
/// "有差异就升级" 是最简单可靠的语义）；本地 dev 构建视为总是落后
fn has_update(current: &str, latest: &str) -> bool {
    let cur = current.trim();
    if cur == "dev" || cur.is_empty() {
        return true;
    }
    cur != latest.trim()
}

/* ---------------- HTTP 基础 ---------------- */

/// 发起 GET 并完成响应头读取；TLS 用 IDF 证书 bundle（覆盖 GitHub 证书链）。
///
/// 手动跟随 3xx 重定向（最多 5 跳）：release 资产的 browser_download_url 会
/// 302 到 objects.githubusercontent.com，而 esp_http_client 的自动重定向在
/// 跨主机时有 "delayed connect error: Software caused connection abort"
/// 的已知问题（旧 socket 未干净关闭就开新连接）→ 这里关掉自动重定向，
/// 每一跳用全新连接手动跟 Location。
fn http_get(uri: &str) -> Result<EspHttpConnection, String> {
    let mut current = uri.to_string();
    for _ in 0..5 {
        let conn = http_get_once(&current)?;
        let status = conn.status();
        if (300..400).contains(&status) {
            let location = conn.header("Location").map(|s| s.to_string());
            let Some(loc) = location.filter(|l| !l.is_empty()) else {
                return Err(format!("redirect {status} without Location"));
            };
            // drop conn（关闭连接）后再开下一跳
            drop(conn);
            // Location 可能是相对路径；GitHub 两个跳转都是绝对 URL，
            // 相对时按当前 URI 的 scheme+host 拼接
            current = if loc.starts_with("http://") || loc.starts_with("https://") {
                loc
            } else {
                let origin = current
                    .split_once("://")
                    .map(|(sch, rest)| {
                        let host = rest.split('/').next().unwrap_or("");
                        format!("{sch}://{host}")
                    })
                    .unwrap_or_default();
                format!("{origin}{loc}")
            };
            // 日志只打 scheme+host+path：签名 URL 的查询串（SAS+JWT）长达 1KB+，纯噪音
            ::log::info!(
                "OTA: following redirect to {}",
                current.split('?').next().unwrap_or(&current)
            );
            continue;
        }
        return Ok(conn);
    }
    Err("too many redirects".into())
}

fn http_get_once(uri: &str) -> Result<EspHttpConnection, String> {
    let mut conn = EspHttpConnection::new(&HttpConfiguration {
        // RX 缓冲（响应头 + 下载分片）
        buffer_size: Some(2048),
        // TX 缓冲：请求行 "GET <path>?<query> HTTP/1.1\r\n" 必须整体放进该缓冲
        // （esp_http_client http_client_prepare_first_line），放不下报
        // "HTTP_CLIENT: Out of buffer"。GitHub 302 到 release-assets 的签名资产
        // URL 查询串（SAS + JWT）约 1KB，请求行 >1.2KB，默认 512 必然爆。
        buffer_size_tx: Some(4096),
        timeout: Some(Duration::from_secs(30)),
        // 重定向由 http_get 手动处理（自动重定向跨主机有已知 bug）
        follow_redirects_policy: FollowRedirectsPolicy::FollowNone,
        crt_bundle_attach: Some(esp_crt_bundle_attach),
        ..Default::default()
    })
    .map_err(|e| format!("http client init: {e:?}"))?;

    conn.initiate_request(
        Method::Get,
        uri,
        &[
            // GitHub API 强制要求 User-Agent，缺了返回 403
            ("User-Agent", "WakeManager-OTA"),
            ("Accept", "application/vnd.github+json"),
        ],
    )
    .map_err(|e| format!("request: {e:?}"))?;

    conn.initiate_response()
        .map_err(|e| format!("response: {e:?}"))?;
    Ok(conn)
}

/* ---------------- 对外接口 ---------------- */

/// 触发版本检查（异步；结果经 /api/status 轮询）。
pub fn start_check() -> Result<(), String> {
    match status().state {
        "checking" | "downloading" => return Err("OTA 正在进行中".into()),
        _ => {}
    }
    set_state("checking", None, None, None);

    std::thread::Builder::new()
        .name("ota_check".into())
        .stack_size(OTA_THREAD_STACK)
        .spawn(check_worker)
        .map_err(|e| format!("spawn: {e:?}"))?;
    Ok(())
}

fn check_worker() {
    match do_check() {
        Ok(latest) => {
            let cur = ac::VERSION.to_string();
            if has_update(&cur, &latest) {
                ::log::info!("OTA: update available: {cur} -> {latest}");
                set_state("update_available", Some(latest), None, None);
            } else {
                ::log::info!("OTA: already up to date ({cur})");
                set_state("idle", Some(latest), None, None);
            }
        }
        Err(e) => {
            ::log::error!("OTA: check failed: {e}");
            set_state("failed", None, None, Some(e));
        }
    }
}

fn do_check() -> Result<String, String> {
    let mut conn = http_get(ac::OTA_CHECK_URL)?;
    // GitHub 在仓库没有任何已发布 Release 时对 latest 接口返回 404
    if conn.status() == 404 {
        return Err("服务器上还没有已发布的版本".into());
    }
    if conn.status() != 200 {
        return Err(format!("check HTTP {}", conn.status()));
    }

    // 读全响应体（上限 CHECK_BODY_MAX）
    let mut body = Vec::with_capacity(8192);
    let mut buf = [0u8; 1024];
    loop {
        let n = conn.read(&mut buf).map_err(|e| format!("read: {e:?}"))?;
        if n == 0 {
            break;
        }
        if body.len() + n > CHECK_BODY_MAX {
            return Err("check response too large".into());
        }
        body.extend_from_slice(&buf[..n]);
    }

    let release: GhRelease =
        serde_json::from_slice(&body).map_err(|e| format!("parse: {e:?}"))?;

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == ac::OTA_ASSET_NAME)
        .ok_or_else(|| format!("release {} has no asset {}", release.tag_name, ac::OTA_ASSET_NAME))?;

    // 下载地址留给 start_upgrade（直连 browser_download_url，升级时重试 2 次）
    if let Ok(mut cell) = latest_url_cell().lock() {
        *cell = Some(asset.browser_download_url.clone());
    }

    // 版本 = Release 目标 commit 的短 SHA（CI 建草稿时写入 github.sha）；
    // 只用 API 响应字段，不再下载 manifest.json（资产域名国内直连不稳）
    Ok(short_sha(&release.target_commitish))
}

/// 触发固件升级（异步；下载完成后自动重启）。
pub fn start_upgrade() -> Result<(), String> {
    let (state, latest) = {
        let s = status();
        (s.state, s.latest)
    };
    match state {
        "checking" | "downloading" => return Err("OTA 正在进行中".into()),
        "update_available" => {}
        _ => return Err("请先检查更新".into()),
    }
    let url = latest_url_cell()
        .lock()
        .ok()
        .and_then(|c| c.clone())
        .ok_or("缺少下载地址，请重新检查更新")?;

    set_state("downloading", latest.clone(), Some(0), None);
    std::thread::Builder::new()
        .name("ota_upgrade".into())
        .stack_size(OTA_THREAD_STACK)
        .spawn(move || upgrade_worker(url, latest))
        .map_err(|e| format!("spawn: {e:?}"))?;
    Ok(())
}

fn upgrade_worker(url: String, latest: Option<String>) {
    let mut last_err = String::from("download failed");
    // 直连下载，尝试 2 次
    for attempt in 1..=2 {
        ::log::info!("OTA: downloading attempt {attempt}: {url}");
        match download_and_flash(&url) {
            Ok(()) => {
                ::log::info!("OTA: image written & verified, rebooting into new firmware");
                set_state("rebooting", latest, Some(100), None);
                // 留一点时间让 HTTP 轮询拿到最终状态
                std::thread::sleep(Duration::from_secs(2));
                unsafe { esp_idf_svc::sys::esp_restart() };
            }
            Err(e) => {
                ::log::warn!("OTA: download failed: {e}");
                last_err = e;
                // 失败后重置进度，等 2s 再试
                if let Ok(mut s) = status_cell().lock() {
                    s.progress = Some(0);
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
    ::log::error!("OTA: all download attempts failed: {last_err}");
    set_state("failed", latest, None, Some(last_err));
}

fn download_and_flash(url: &str) -> Result<(), String> {
    let mut conn = http_get(url)?;
    if conn.status() != 200 {
        return Err(format!("download HTTP {}", conn.status()));
    }
    let total: Option<usize> = conn
        .header("Content-Length")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&t| t > 0);

    let mut ota = EspOta::new().map_err(|e| format!("ota init: {e:?}"))?;
    // 已知大小时只擦除镜像占用的扇区（未知则整槽擦除，3.9MB 槽要等较久）
    let mut update = match total {
        Some(t) => ota
            .initiate_update_with_known_size(t)
            .map_err(|e| format!("ota begin: {e:?}"))?,
        None => ota.initiate_update().map_err(|e| format!("ota begin: {e:?}"))?,
    };

    let mut received: usize = 0;
    let mut buf = [0u8; DL_BUF_SIZE];
    loop {
        let n = conn.read(&mut buf).map_err(|e| format!("download: {e:?}"))?;
        if n == 0 {
            break;
        }
        update
            .write(&buf[..n])
            .map_err(|e| format!("ota write: {e:?}"))?;
        received += n;
        if let Some(t) = total {
            let pct = (received as u64 * 100 / t as u64).min(100) as u32;
            if let Ok(mut s) = status_cell().lock() {
                s.progress = Some(pct);
            }
        }
    }

    if received == 0 {
        return Err("empty download".into());
    }
    if let Some(t) = total {
        if received != t {
            return Err(format!("incomplete image: {received}/{t} bytes"));
        }
    }

    // 校验镜像（esp_ota_end）并设置启动分区；下次重启进入新固件
    update
        .complete()
        .map_err(|e| format!("ota finalize: {e:?}"))?;
    Ok(())
}
