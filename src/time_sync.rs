//! WakeManager - SNTP clock calibration（对应 C 版 components/time_sync）
//!
//! SNTP 生命周期走 esp-idf-svc 的 `EspSntp` 高层封装（operatingmode、
//! servername、同步通知回调、init/deinit 全部由它托管，server 字符串的
//! 生命周期也由句柄持有）；时区仍走 libc setenv/tzset（0.53 的 SntpConf
//! 无 timezone 字段）。`format_now` 保留 localtime_r FFI。

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use esp_idf_svc::sntp::{EspSntp, SntpConf};
use esp_idf_svc::sys::*;

use crate::app_config as ac;

static SYNCED: AtomicBool = AtomicBool::new(false);
/// 开机时刻（Unix 秒；None = 尚未记录）。首次 SNTP 校时后由回调填充。
/// xtensa 目标无 64 位原子操作 → 用 Mutex。
static BOOT_TIME: Mutex<Option<i64>> = Mutex::new(None);

/// localtime_r + 手工格式化（与 C 版 strftime("%Y-%m-%d %H:%M:%S") 一致）
fn format_time(t: time_t) -> String {
    unsafe {
        let mut tm: tm = std::mem::zeroed();
        if localtime_r(&t, &mut tm).is_null() {
            return String::new();
        }
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

/// 当前（本地）时间
pub fn format_now() -> String {
    format_time(unsafe { time(std::ptr::null_mut()) })
}

/// 开机时间（本地时间）。首次 SNTP 校时前返回 None。
/// 校时时刻 − 开机以来的运行时长（esp_timer_get_time），即真实上电/重启时刻。
pub fn boot_time() -> Option<String> {
    BOOT_TIME.lock().unwrap().map(|t| format_time(t as time_t))
}

/// 对应 C 版 `time_sync_start()`。
/// 返回的句柄必须全程保活：`EspSntp` 的 Drop 会停止 SNTP 服务。
pub fn start() -> EspSntp<'static> {
    unsafe {
        // 先应用时区，SNTP 完成后 localtime_r 即返回本地时间
        let tz = CString::new(ac::TIME_TIMEZONE).expect("no NUL");
        setenv(c"TZ".as_ptr(), tz.as_ptr(), 1);
        tzset();
    }

    // CONFIG_LWIP_SNTP_MAX_SERVERS=1 → servers 只有一个槽位，覆盖即可
    let mut conf = SntpConf::default();
    conf.servers[0] = ac::NTP_SERVER;
    let sntp = EspSntp::new_with_callback(&conf, |_delay| {
        // 首次校时：用"同步时刻 − 运行时长"反推真实开机时刻（后续重校时不更新）
        if BOOT_TIME.lock().unwrap().is_none() {
            let synced_at = unsafe { time(std::ptr::null_mut()) };
            let uptime_s = unsafe { esp_timer_get_time() } / 1_000_000;
            let boot = synced_at - uptime_s;
            *BOOT_TIME.lock().unwrap() = Some(boot);
            ::log::info!("Boot time: {} (uptime {}s)", format_time(boot), uptime_s);
        }
        SYNCED.store(true, Ordering::SeqCst);
        ::log::info!("Clock synchronised: {}", format_now());
    })
    .expect("SNTP init failed");

    ::log::info!(
        "SNTP started (server={}, tz={})",
        ac::NTP_SERVER,
        ac::TIME_TIMEZONE
    );
    sntp
}

/// 对应 C 版 `time_sync_is_synced()`
pub fn is_synced() -> bool {
    SYNCED.load(Ordering::SeqCst)
}
