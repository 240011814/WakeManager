//! WakeManager - 原生电容触摸检测（GPIO1 / GPIO7）
//!
//! ESP32-S3 触摸通道映射：Tn = GPIOn（T0 为内部去噪通道，无外部引脚），
//! 直接接裸铜箔 / 弹簧 / 触摸焊盘，无需外部触摸芯片。
//!
//! 通道与功能：
//!   - **GPIO1（T1）**：轻触反馈 —— 打印日志 + 状态 LED 闪一下
//!   - **GPIO7（T7）**：长按 20 s —— 清除 Wi-Fi 配置（`esp_wifi_restore`）并重启，
//!     开机后配网管理器检测到 STA SSID 为空，自动进入 BLE 配网模式
//!     （与串口命令 `wifi-clear` 的动作一致）
//!
//! 工作方式（legacy `driver/touch_pad.h`，polling 版）：
//!   1. `touch_pad_init` → 定时器 FSM 模式 → 逐通道 `touch_pad_config` → `touch_pad_fsm_start`
//!   2. 基线校准用**滑动窗口中位数**：窗口内极差 ≤ 25%（稳定未触摸）才接受；
//!      开机时手指若还按着（例如刚用长按触发过重启），窗口会一直不稳定 → 持续等待，
//!      手指离开后自动完成；极端干扰下 10 s 后强行接受并告警
//!   3. 后台线程 50 ms 轮询原始计数。注意：**ESP32-S3（触摸硬件 v2）触摸时计数上升**
//!      （硬件触发条件 = raw − benchmark ≥ 阈值，见 touch_sensor_ll.h；与老 ESP32 v1 的
//!      “触摸下降”相反），超过基线 × 1.5 判定为触摸（回落到 × 1.2 以下才释放，带迟滞防抖）
//!   4. 运行期基线自愈：未按下时读数持续 1 s 低于基线 × 0.9（开机校准被触摸污染等
//!      异常场景）→ 自动把基线重校准到当前读数；未按下且读数明显偏高时基线慢速向上漂移
//!      （吸收环境/温度漂移，速度慢到不会被一次触摸带偏）
//!
//! GPIO 走触摸外设的模拟通道，不占用 `Peripherals::take()`（与 debug_uart/LED 无关）。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use esp_idf_svc::sys::*;

use crate::led_status;

/// 轮询周期
const POLL_MS: u64 = 50;
/// 校准前丢弃的样本数，等待测量稳定
const WARMUP_SAMPLES: u32 = 5;
/// 校准滑动窗口：warmup 后取 2 s 窗口样本的中位数作为基线
const CALIB_WINDOW: usize = 40;
/// 校准窗口上限：窗口极差一直超限（有触摸/干扰）时，最多再等 ~8 s 后强行接受
const CALIB_MAX: usize = 200;
/// 校准接受条件：窗口内 (max-min) ≤ max × 25% 视为“稳定的未触摸状态”
const CALIB_SPREAD: f64 = 0.25;
/// 运行期自愈：未按下时连续 1 s 读数低于基线 × 0.9 → 基线重校准到当前读数
const RECAL_SUSTAIN_SAMPLES: u32 = 20;
/// 未按下时基线向上慢漂的吸收分母：仅当 raw - baseline ≥ 1200 时才吸收
/// （约 1/1200 per poll，τ≈60 s；一次几秒的触摸几乎带不动它）
const DRIFT_DIVISOR: u32 = 1200;
/// 触发阈值：原始计数 > 基线 ×（1 + 0.5），S3 触摸时计数上升（实测触摸为基线的 6~15 倍）
const TOUCH_RISE_RATIO: f64 = 0.5;
/// 释放阈值：原始计数回落到 基线 ×（1 + 0.2） 以下（迟滞，防抖）
const RELEASE_RISE_RATIO: f64 = 0.2;

/// GPIO7 长按目标时长
const LONG_PRESS_MS: u64 = 20_000;
/// 长按过程中断续低于释放阈值的容忍时间（手指轻微晃动不取消长按）
const LONG_PRESS_RELEASE_GRACE_MS: u64 = 500;
/// 长按进度日志间隔
const LONG_PRESS_LOG_MS: u64 = 5_000;

/// 一个触摸通道的状态
struct Chan {
    pad: touch_pad_t,
    gpio: u32,
    /// `Some(ms)` = 长按通道；`None` = 轻触通道
    long_press_ms: Option<u64>,
    /// 是否仍在基线校准阶段
    calibrating: bool,
    /// 校准阶段剩余丢弃样本数
    warmup: u32,
    /// 校准滑动窗口
    samples: VecDeque<u32>,
    /// 校准等待超过 5 s 的提示已发过
    calib_warned: bool,
    /// 运行期：连续低于基线×0.9 的样本计数（基线自愈）
    low_count: u32,
    baseline: u32,
    touch_th: u32,
    release_th: u32,
    /// 当前是否处于按下状态
    pressed: bool,
    /// 长按通道：本次按下起始时刻
    press_since: Option<Instant>,
    /// 长按通道：开始低于释放阈值的时刻（grace 计时）
    release_since: Option<Instant>,
    /// 长按通道：下一次进度日志的已按住毫秒数
    next_log_ms: u64,
    /// 触摸次数
    count: u32,
}

impl Chan {
    fn new(pad: touch_pad_t, gpio: u32, long_press_ms: Option<u64>) -> Self {
        Self {
            pad,
            gpio,
            long_press_ms,
            calibrating: true,
            warmup: WARMUP_SAMPLES,
            samples: VecDeque::with_capacity(CALIB_MAX),
            calib_warned: false,
            low_count: 0,
            baseline: 0,
            touch_th: 0,
            release_th: 0,
            pressed: false,
            press_since: None,
            release_since: None,
            next_log_ms: LONG_PRESS_LOG_MS,
            count: 0,
        }
    }

    /// 基线变化后重算触发/释放阈值
    fn finish_calibration(&mut self) {
        self.touch_th = (self.baseline as f64 * (1.0 + TOUCH_RISE_RATIO)) as u32;
        self.release_th = (self.baseline as f64 * (1.0 + RELEASE_RISE_RATIO)) as u32;
    }
}

/// 初始化触摸外设并启动检测线程（只应在 main 中调用一次）
pub fn start() {
    let mut chans = vec![
        // GPIO1：轻触反馈（LED 闪一下 + 日志）
        Chan::new(touch_pad_t_TOUCH_PAD_NUM1, 1, None),
        // GPIO7：长按 20 s 清 Wi-Fi → 重启进 BLE 配网
        Chan::new(touch_pad_t_TOUCH_PAD_NUM7, 7, Some(LONG_PRESS_MS)),
    ];

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
    // 逐通道使能（内部顺带把对应 GPIO 配置为模拟输入）
    for ch in &mut chans {
        if let Err(e) = check(unsafe { touch_pad_config(ch.pad) }, "touch_pad_config") {
            ::log::error!("Touch channel config failed on GPIO{}: {e}", ch.gpio);
            return;
        }
    }
    // 启动测量 FSM（touch_pad_init 会停掉它，S3 上不会自动重启）
    if let Err(e) = check(unsafe { touch_pad_fsm_start() }, "touch_pad_fsm_start") {
        ::log::error!("Touch FSM start failed: {e}");
        return;
    }

    std::thread::Builder::new()
        .name("touch".into())
        .stack_size(8192)
        .spawn(move || touch_task(chans))
        .expect("spawn touch task");

    ::log::info!("Touch sensor initialized on GPIO1 (T1) and GPIO7 (T7)");
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
fn read_raw(pad: touch_pad_t) -> Option<u32> {
    let mut raw: u32 = 0;
    if unsafe { touch_pad_read_raw_data(pad, &mut raw) } == ESP_OK {
        Some(raw)
    } else {
        None
    }
}

/// 窗口中位数（拷贝排序取中间，窗口 ≤ 200 个样本）
fn median(win: &VecDeque<u32>) -> u32 {
    let mut v: Vec<u32> = win.iter().copied().collect();
    v.sort_unstable();
    v[v.len() / 2]
}

fn touch_task(mut chans: Vec<Chan>) {
    // ---- 校准：滑动窗口取稳定“未触摸”中位数 ----
    let calib_start = Instant::now();
    while chans.iter().any(|c| c.calibrating) {
        std::thread::sleep(Duration::from_millis(POLL_MS));
        let waited = calib_start.elapsed();

        for ch in chans.iter_mut().filter(|c| c.calibrating) {
            let Some(raw) = read_raw(ch.pad) else { continue };
            if ch.warmup > 0 {
                ch.warmup -= 1;
                continue;
            }
            ch.samples.push_back(raw);
            if ch.samples.len() > CALIB_MAX {
                ch.samples.pop_front();
            }
            if ch.samples.len() < CALIB_WINDOW {
                continue;
            }

            let (min, max) = (
                *ch.samples.iter().min().unwrap(),
                *ch.samples.iter().max().unwrap(),
            );
            let spread_ok = max > 0 && (max - min) as f64 <= max as f64 * CALIB_SPREAD;
            if spread_ok || ch.samples.len() >= CALIB_MAX {
                ch.baseline = median(&ch.samples);
                ch.finish_calibration();
                ch.calibrating = false;
                if !spread_ok {
                    ::log::warn!(
                        "Touch baseline on GPIO{} accepted after timeout (unstable window min {min} max {max}) - interference or pad was touched?",
                        ch.gpio
                    );
                }
                ::log::info!(
                    "Touch baseline on GPIO{}: {} (touch > {}, release < {})",
                    ch.gpio,
                    ch.baseline,
                    ch.touch_th,
                    ch.release_th
                );
            } else if !ch.calib_warned && waited >= Duration::from_secs(5) {
                // 手指一直按着（如刚长按触发过重启）/干扰持续 → 继续等待并提示
                ch.calib_warned = true;
                ::log::warn!(
                    "Waiting for touch baseline calibration on GPIO{} (window min {min} max {max}) - keep the pad untouched",
                    ch.gpio
                );
            }
        }
    }

    // ---- 轮询检测（带迟滞；S3 触摸时计数上升）----
    loop {
        std::thread::sleep(Duration::from_millis(POLL_MS));
        let now = Instant::now();
        for ch in chans.iter_mut() {
            let Some(raw) = read_raw(ch.pad) else { continue };
    
            if !ch.pressed {
                // 未按下：超过触发阈值则按下
                if raw > ch.touch_th {
                    ch.pressed = true;
                    ch.count += 1;
                    // 自愈计时清零（避免按下期间累积）
                    ch.low_count = 0;
                    ::log::info!(
                        "Touch detected on GPIO{} (count: {}, raw: {raw}, baseline: {})",
                        ch.gpio,
                        ch.count,
                        ch.baseline
                    );
                    match ch.long_press_ms {
                        // 轻触通道：LED 闪一下
                        None => led_status::touch_flash(),
                        // 长按通道：开始计时
                        Some(total) => {
                            ch.press_since = Some(now);
                            ch.release_since = None;
                            ch.next_log_ms = LONG_PRESS_LOG_MS;
                            ::log::info!(
                                "GPIO{} long press started, hold {} s to clear Wi-Fi and re-enter provisioning",
                                ch.gpio,
                                total / 1000
                            );
                        }
                    }
                } else {
                    // 基线自愈：持续低于基线×0.9（开机校准被触摸污染等异常）→ 重校准
                    // （触摸只会抬高读数，"持续偏低"只可能是参考基线错了）
                    if raw + raw / 10 < ch.baseline {
                        ch.low_count += 1;
                        if ch.low_count >= RECAL_SUSTAIN_SAMPLES {
                            ::log::info!(
                                "Touch baseline on GPIO{} recalibrated: {} -> {raw}",
                                ch.gpio,
                                ch.baseline
                            );
                            ch.baseline = raw;
                            ch.finish_calibration();
                            ch.low_count = 0;
                        }
                    } else {
                        ch.low_count = 0;
                        // 向上慢漂：吸收环境/温度导致的缓慢上移（分母大，一次触摸带不动）
                        if raw > ch.baseline + DRIFT_DIVISOR {
                            ch.baseline += (raw - ch.baseline) / DRIFT_DIVISOR;
                            ch.finish_calibration();
                        }
                    }
                }
                continue;
            }

            // 已按下
            match ch.long_press_ms {
                None => {
                    // 轻触通道：回落到释放阈值以下立即释放
                    if raw < ch.release_th {
                        ch.pressed = false;
                        ::log::info!("Touch released on GPIO{} (raw: {raw})", ch.gpio);
                    }
                }
                Some(total) => {
                    if raw < ch.release_th {
                        // 短暂低于释放阈值：grace 计时，恢复则继续长按
                        if ch.release_since.is_none() {
                            ch.release_since = Some(now);
                        }
                        if now.duration_since(ch.release_since.unwrap())
                            >= Duration::from_millis(LONG_PRESS_RELEASE_GRACE_MS)
                        {
                            let held = ch.press_since.map(|s| now.duration_since(s));
                            ::log::info!(
                                "GPIO{} long press canceled (held {:?}, raw: {raw})",
                                ch.gpio,
                                held
                            );
                            ch.pressed = false;
                            ch.press_since = None;
                            ch.release_since = None;
                        }
                        continue;
                    }
                    ch.release_since = None;

                    let held = match ch.press_since {
                        Some(since) => now.duration_since(since),
                        None => continue,
                    };
                    // 进度日志
                    if held.as_millis() as u64 >= ch.next_log_ms {
                        ::log::info!(
                            "GPIO{} long press: {} / {} s",
                            ch.gpio,
                            ch.next_log_ms / 1000,
                            total / 1000
                        );
                        ch.next_log_ms += LONG_PRESS_LOG_MS;
                    }
                    // 达到目标时长：清 Wi-Fi 并重启进配网
                    if held >= Duration::from_millis(total) {
                        if enter_provisioning(ch.gpio) {
                            // 理论上 esp_restart() 不会返回；万一返回则不再重复触发，
                            // 等手指抬起后才能再次长按
                            ch.press_since = None;
                        }
                    }
                }
            }
        }
    }
}

/// 长按达成：清除 Wi-Fi 配置（SSID/密码）→ 重启。
/// 开机时配网管理器检测到 STA SSID 为空（manager.c 的 is_wifi_provisioned 判定），
/// 自动重新进入 BLE 配网模式。与串口命令 `wifi-clear` 的动作一致。
/// 返回 true = 已请求重启（不应返回），false = 清除失败（设备保持运行，可松手后重试）
#[allow(unreachable_code)] // esp_restart() 不会返回，末尾 true 仅为满足签名
fn enter_provisioning(gpio: u32) -> bool {
    ::log::warn!(
        "GPIO{gpio} long press {LONG_PRESS_MS}ms reached: clearing Wi-Fi config, rebooting into BLE provisioning"
    );
    let err = unsafe { esp_wifi_restore() };
    if err != ESP_OK {
        ::log::error!("esp_wifi_restore failed: {err:#x} (device keeps running)");
        return false;
    }
    // 给控制台 log 落盘留一点时间，再重启
    std::thread::sleep(Duration::from_millis(300));
    unsafe { esp_restart() };
    true
}
