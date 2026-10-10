//! WakeManager - SNTP clock calibration（对应 C 版 components/time_sync）
//!
//! 直接用 esp_sntp FFI，与 C 版调用序列一致：
//! setenv(TZ) → tzset → operatingmode → servername → 通知回调 → init。

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};

use esp_idf_svc::sys::*;

use crate::app_config as ac;

static SYNCED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn sync_notification_cb(_tv: *mut timeval) {
    SYNCED.store(true, Ordering::SeqCst);
    ::log::info!("Clock synchronised: {}", format_now());
}

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

/// 对应 C 版 `time_sync_start()`
pub fn start() {
    unsafe {
        // 先应用时区，SNTP 完成后 localtime_r 即返回本地时间
        let tz = CString::new(ac::TIME_TIMEZONE).expect("no NUL");
        setenv(c"TZ".as_ptr(), tz.as_ptr(), 1);
        tzset();

        esp_sntp_setoperatingmode(esp_sntp_operatingmode_t_ESP_SNTP_OPMODE_POLL);
        // lwIP 的 setservername 只存指针不拷贝 → 泄漏一份静态字符串
        let server: &'static CString = Box::leak(Box::new(
            CString::new(ac::NTP_SERVER).expect("no NUL"),
        ));
        esp_sntp_setservername(0, server.as_ptr());
        sntp_set_time_sync_notification_cb(Some(sync_notification_cb));
        esp_sntp_init();
    }

    ::log::info!(
        "SNTP started (server={}, tz={})",
        ac::NTP_SERVER,
        ac::TIME_TIMEZONE
    );
}

/// 对应 C 版 `time_sync_is_synced()`
pub fn is_synced() -> bool {
    SYNCED.load(Ordering::SeqCst)
}
