//! WakeManager - GPIO1 原生电容触摸检测
//!
//! GPIO1 是 ESP32-S3 的电容触摸通道 **T1**（T0 为内部去噪通道，无外部引脚），
//! 直接接裸铜箔 / 弹簧 / 触摸焊盘，无需外部触摸芯片。
//!
//! 工作方式（legacy `driver/touch_pad.h`，polling 版）：
//!   1. `touch_pad_init` → 定时器 FSM 模式 → `touch_pad_config(T1)` → `touch_pad_fsm_start`
//!   2. 启动时采样约 1 s 校准“未触摸”基线（取最大值）
//!   3. 后台线程 50 ms 轮询原始计数。注意：**ESP32-S3（触摸硬件 v2）触摸时计数上升**
//!      （硬件触发条件 = raw − benchmark ≥ 阈值，见 touch_sensor_ll.h；与老 ESP32 v1 的
//!      “触摸下降”相反），超过基线 × 1.5 判定为触摸（回落到 × 1.2 以下才释放，带迟滞防抖）
//!   4. 触摸时：打印日志 + `led_status::touch_flash()` 让状态 LED 闪一下
//!
//! GPIO 走触摸外设的模拟通道，不占用 `Peripherals::take()`（与 debug_uart/LED 无关）。

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use esp_idf_svc::sys::*;

use crate::led_status;

/// 触摸通道：GPIO1 = T1（`touch_pad_t_TOUCH_PAD_NUM0` 是内部去噪通道）
const TOUCH_PAD: touch_pad_t = touch_pad_t_TOUCH_PAD_NUM1;
const TOUCH_GPIO: u32 = 1;

/// 轮询周期
const POLL_MS: u64 = 50;
/// 校准前丢弃的样本数，等待测量稳定
const WARMUP_SAMPLES: u32 = 5;
/// 校准样本数（含 warmup，约 0.5~1 s）
const CALIB_SAMPLES: u32 = 25;
/// 触发阈值：原始计数 > 基线 ×（1 + 0.5），S3 触摸时计数上升（实测触摸为基线的 6~15 倍）
const TOUCH_RISE_RATIO: f64 = 0.5;
/// 释放阈值：原始计数回落到 基线 ×（1 + 0.2） 以下（迟滞，防抖）
const RELEASE_RISE_RATIO: f64 = 0.2;

/// 触摸计数（日志里显示是第几次触摸）
static TOUCH_COUNT: AtomicU32 = AtomicU32::new(0);

/// 初始化触摸外设并启动检测线程（只应在 main 中调用一次）
pub fn start() {
    // 初始化外设（内部会复位并停掉测量 FSM）
    if let Err(e) = check(unsafe { touch_pad_init() }, "touch_pad_init") {
        ::log::error!("Touch sensor init failed: {e}");
        return;
    }
    // 定时器模式：硬件自动周期测量，线程只负责读取
    let _ = check(
        unsafe { touch_pad_set_fsm_mode(touch_fsm_mode_t_TOUCH_FSM_MODE_TIMER) },
        "touch_pad_set_fsm_mode",
    );
    // 使能通道 T1（内部顺带把 GPIO1 配置为模拟输入）
    if let Err(e) = check(unsafe { touch_pad_config(TOUCH_PAD) }, "touch_pad_config") {
        ::log::error!("Touch channel config failed: {e}");
        return;
    }
    // 启动测量 FSM（touch_pad_init 会停掉它，S3 上不会自动重启）
    if let Err(e) = check(unsafe { touch_pad_fsm_start() }, "touch_pad_fsm_start") {
        ::log::error!("Touch FSM start failed: {e}");
        return;
    }

    std::thread::Builder::new()
        .name("touch".into())
        .stack_size(8192)
        .spawn(touch_task)
        .expect("spawn touch task");

    ::log::info!("Touch sensor initialized on GPIO{TOUCH_GPIO} (channel T1)");
}

/// esp_err_t 检查：Ok 返回 ()，Err 返回带函数名的字符串
fn check(err: esp_err_t, what: &str) -> Result<(), String> {
    if err == ESP_OK {
        Ok(())
    } else {
        Err(format!("{what} failed: {err:#x}"))
    }
}

/// 读一次原始计数（单位：充放电时钟周期数）
fn read_raw() -> Option<u32> {
    let mut raw: u32 = 0;
    if unsafe { touch_pad_read_raw_data(TOUCH_PAD, &mut raw) } == ESP_OK {
        Some(raw)
    } else {
        None
    }
}

fn touch_task() {
    // ---- 校准：采集未触摸状态下的基线 ----
    let mut baseline: u32 = 0;
    for i in 0..CALIB_SAMPLES {
        std::thread::sleep(Duration::from_millis(POLL_MS));
        let Some(raw) = read_raw() else {
            ::log::warn!("touch read failed during calibration");
            continue;
        };
        if i >= WARMUP_SAMPLES {
            baseline = baseline.max(raw);
        }
    }

    if baseline == 0 {
        // 基线为 0 说明测量未运行，无法判定触摸；持续告警便于排查
        ::log::warn!(
            "Touch baseline is 0 on GPIO{TOUCH_GPIO} - no valid measurement (touch disabled)"
        );
    } else {
        ::log::info!(
            "Touch baseline on GPIO{TOUCH_GPIO}: {baseline} (touch > {}, release < {})",
            (baseline as f64 * (1.0 + TOUCH_RISE_RATIO)) as u32,
            (baseline as f64 * (1.0 + RELEASE_RISE_RATIO)) as u32
        );
    }

    // ---- 轮询检测（带迟滞；S3 触摸时计数上升）----
    let touch_th = (baseline as f64 * (1.0 + TOUCH_RISE_RATIO)) as u32;
    let release_th = (baseline as f64 * (1.0 + RELEASE_RISE_RATIO)) as u32;
    let mut touched = false;

    loop {
        std::thread::sleep(Duration::from_millis(POLL_MS));
        let Some(raw) = read_raw() else { continue };
       // ::log::info!("Touch value on baseline{baseline} (raw: {raw})");
        if !touched && baseline != 0 && raw > touch_th {
            touched = true;
            let count = TOUCH_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
            ::log::info!(
                "Touch detected on GPIO{TOUCH_GPIO} (count: {count}, raw: {raw}, baseline: {baseline})"
            );
            led_status::touch_flash();
        } else if touched && raw < release_th {
            touched = false;
            ::log::info!("Touch released on GPIO{TOUCH_GPIO} (raw: {raw})");
        }
    }
}
