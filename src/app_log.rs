//! WakeManager - 日志前端（替代 esp-idf-svc 的 EspLogger）
//!
//! 背景：本项目打开 `CONFIG_LOG_TIMESTAMP_SOURCE_SYSTEM_FULL`（时间戳带日期，
//! 见 sdkconfig.defaults）。C 侧 esp_log v2 会输出 `I (26-10-13 03:43:51.911) tag: msg`，
//! 但 esp-idf-svc 0.53 的 `EspLogger` 是在 Rust 侧自己拼前缀，只认 cfg
//! `esp_idf_log_timestamp_source_{rtos,system}`，不认 `..._system_full` →
//! 括号里空着（`I () tag: msg`）。因此这里自带一个与 esp_log v2 + SYSTEM_FULL
//! 格式完全一致的 logger：
//!
//!   - 时间戳：`gettimeofday` + `localtime_r`（与 IDF 的 `esp_log_timestamp_str` 同源）
//!   - 过滤：沿用 ESP-IDF 的 `esp_log_level_get(tag)`，运行时 `esp_log_level_set` 依然生效
//!   - 颜色：跟随 `CONFIG_LOG_COLORS`（cfg `esp_idf_log_colors`），与 C 日志一致

use std::fmt::Write as _;
use std::io::Write as _;

use ::log::{Level, LevelFilter, Log, Metadata, Record};
use esp_idf_svc::sys::*;

struct AppLogger;

static LOGGER: AppLogger = AppLogger;

/// 在 main 最早期调用（原 `EspLogger::initialize_default()` 的位置）
pub fn init() {
    ::log::set_logger(&LOGGER).expect("log logger already initialized");
    // 实际过滤在 enabled() 里按 tag 查询 esp_log，这里先放开
    ::log::set_max_level(LevelFilter::Trace);
}

/// 与 esp_log 的 `s_lvl_name` 一致
fn marker(level: Level) -> &'static str {
    match level {
        Level::Error => "E",
        Level::Warn => "W",
        Level::Info => "I",
        Level::Debug => "D",
        Level::Trace => "V",
    }
}

/// 与 esp_log 的 `s_lvl_color` 一致（跟随 sdkconfig CONFIG_LOG_COLORS）
fn color_code(level: Level) -> Option<u8> {
    #[cfg(esp_idf_log_colors)]
    {
        match level {
            Level::Error => Some(31), // red
            Level::Warn => Some(33),  // brown/yellow
            Level::Info => Some(32),  // green
            _ => None,
        }
    }
    #[cfg(not(esp_idf_log_colors))]
    {
        let _ = level;
        None
    }
}

/// 写入 `YY-MM-DD HH:MM:SS.mmm`（与 CONFIG_LOG_TIMESTAMP_SOURCE_SYSTEM_FULL 相同）
fn timestamp(buf: &mut String) {
    let mut tv = timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe { gettimeofday(&mut tv, core::ptr::null_mut()) };

    let mut t: tm = unsafe { core::mem::zeroed() };
    let sec = tv.tv_sec;
    unsafe { localtime_r(&sec as *const time_t, &mut t) };

    let _ = write!(
        buf,
        "{:02}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        (t.tm_year + 1900) % 100,
        t.tm_mon + 1,
        t.tm_mday,
        t.tm_hour,
        t.tm_min,
        t.tm_sec,
        tv.tv_usec / 1000
    );
}

impl Log for AppLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // 与 IDF 运行时级别（esp_log_level_set / 默认级别）保持一致
        let Ok(tag) = std::ffi::CString::new(metadata.target()) else {
            return true;
        };
        let max = unsafe { esp_log_level_get(tag.as_ptr()) };
        (metadata.level() as u32) <= max
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let mut line = String::with_capacity(128);
        let color = color_code(record.level());
        if let Some(c) = color {
            let _ = write!(line, "\x1b[0;{c}m");
        }
        let _ = write!(line, "{} (", marker(record.level()));
        timestamp(&mut line);
        let _ = write!(line, ") {}: {}\n", record.target(), record.args());
        if color.is_some() {
            line.push_str("\x1b[0m");
        }

        // 整行一次写出，避免与其他线程的 C 日志在行中间交错
        let _ = std::io::stdout().write_all(line.as_bytes());
    }

    fn flush(&self) {}
}
