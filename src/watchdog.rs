//! WakeManager - Task Watchdog (TWDT) control（对应 C 版 components/watchdog）

use std::sync::atomic::{AtomicBool, Ordering};

use esp_idf_svc::sys::*;

use crate::app_config as ac;

static READY: AtomicBool = AtomicBool::new(false);

/// 对应 C 版 `watchdog_init()`
pub fn init() -> Result<(), esp_err_t> {
    let cfg = esp_task_wdt_config_t {
        timeout_ms: ac::WATCHDOG_TIMEOUT_MS,
        // 两核的 idle 任务都订阅（(1 << portNUM_PROCESSORS) - 1，S3 = 0b11）
        idle_core_mask: 0b11,
        trigger_panic: ac::WATCHDOG_TRIGGER_PANIC,
    };

    // IDF 可能已自动初始化 TWDT（CONFIG_ESP_TASK_WDT_INIT），此时改用 reconfigure
    let mut err = unsafe { esp_task_wdt_reconfigure(&cfg) };
    if err == ESP_ERR_INVALID_STATE {
        err = unsafe { esp_task_wdt_init(&cfg) };
    }
    if err != ESP_OK {
        ::log::error!("Failed to enable task watchdog: {err:#x}");
        return Err(err);
    }

    READY.store(true, Ordering::SeqCst);
    ::log::info!(
        "Task watchdog enabled: timeout={} ms, panic={}",
        ac::WATCHDOG_TIMEOUT_MS,
        ac::WATCHDOG_TRIGGER_PANIC
    );
    Ok(())
}

/// 对应 C 版 `watchdog_add_current_task()`（订阅当前任务）
pub fn add_current_task() -> Result<(), esp_err_t> {
    if !READY.load(Ordering::SeqCst) {
        return Err(ESP_ERR_INVALID_STATE);
    }
    let err = unsafe { esp_task_wdt_add(std::ptr::null_mut()) };
    if err != ESP_OK && err != ESP_ERR_INVALID_ARG {
        ::log::warn!("Failed to subscribe task to watchdog: {err:#x}");
    }
    Ok(())
}

/// 对应 C 版 `watchdog_feed()`
pub fn feed() {
    if READY.load(Ordering::SeqCst) {
        unsafe {
            esp_task_wdt_reset();
        }
    }
}
