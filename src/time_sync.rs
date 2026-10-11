//! WakeManager - SNTP clock calibration（对应 C 版 components/time_sync）
//!
//! SNTP 生命周期走 esp-idf-svc 的 `EspSntp` 高层封装（operatingmode、
//! servername、同步通知回调、init/deinit 全部由它托管，server 字符串的
//! 生命周期也由句柄持有）；时区仍走 libc setenv/tzset（0.53 的 SntpConf
//! 无 timezone 字段）。`format_now` 保留 localtime_r FFI。

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};

use esp_idf_svc::sntp::{EspSntp, SntpConf};
use esp_idf_svc::sys::*;

use crate::app_config as ac;

static SYNCED: AtomicBool = AtomicBool::new(false);

/// localtime_r + 手工格式化（与 C 版 strftime("%Y-%m-%d %H:%M:%S") 一致）
pub fn format_now() -> String {
    unsafe {
        let now: time_t = time(std::ptr::null_mut());
        let mut tm: tm = std::mem::zeroed();
        if localtime_r(&now, &mut tm).is_null() {
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
