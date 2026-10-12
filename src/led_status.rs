//! WakeManager - status indicator LED（对应 C 版 components/led_status）
//!
//! 一个后台线程独占 LED，根据 `set()` 的状态闪烁；
//! MQTT 活动通过 `activity()` 叠加一段快速闪烁。
//! GPIO 走 esp-idf-hal 的 `PinDriver`（无裸 FFI）；线程用 Condvar 事件驱动，
//! 仅在闪烁节拍或 set()/activity() 唤醒时运行，常亮期间不再高频轮询。

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use esp_idf_svc::hal::gpio::{Level, Output, PinDriver};
use esp_idf_svc::hal::peripherals::Peripherals;

/* --- Indicator configuration ------------------------------------------- */
const LED_GPIO: u8 = 48;
const LED_ACTIVE_LOW: bool = true; // active-low：点亮 = 输出低电平
const LED_BLINK_MS: u64 = 500; // half period of the "connecting" blink

const ACTIVITY_BLINK_MS: u64 = 100;
const ACTIVITY_DURATION_MS: u64 = 2000;
/// 已连接常亮态的重估周期：电平不变，仅兜底重读状态
const STEADY_RECHECK_MS: u64 = 1000;
/* ----------------------------------------------------------------------- */

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    Connecting = 0,
    Connected = 1,
}

static STATUS: AtomicU8 = AtomicU8::new(0);
/// MQTT 活动闪烁截止时刻（None = 无活动）
static ACTIVITY_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
/// 唤醒信号：set()/activity() 置位并通知 LED 线程立即重估相位
static WAKE: Mutex<bool> = Mutex::new(false);
static WAKE_CV: Condvar = Condvar::new();

fn wake() {
    let mut w = WAKE.lock().unwrap();
    *w = true;
    WAKE_CV.notify_all();
}

fn set_level(pin: &mut PinDriver<'static, Output>, on: bool) {
    let level = if on { !LED_ACTIVE_LOW } else { LED_ACTIVE_LOW };
    let _ = pin.set_level(Level::from(level));
}

fn led_task(mut pin: PinDriver<'static, Output>) {
    let mut on = false;
    let mut guard = WAKE.lock().unwrap();
    loop {
        // 过期的活动窗口先清掉，再判断当前相位
        let activity = {
            let mut act = ACTIVITY_UNTIL.lock().unwrap();
            if matches!(*act, Some(until) if Instant::now() >= until) {
                *act = None;
            }
            act.is_some()
        };

        let wait_ms = if activity {
            // MQTT 流量活动：快速闪烁，窗口结束后自动回到常规状态
            on = !on;
            set_level(&mut pin, on);
            ACTIVITY_BLINK_MS
        } else if STATUS.load(Ordering::SeqCst) == 1 {
            // 已连接：常亮（set_level 幂等，重复调用无副作用）
            set_level(&mut pin, true);
            STEADY_RECHECK_MS
        } else {
            // 配网 / 重连中：慢闪
            on = !on;
            set_level(&mut pin, on);
            LED_BLINK_MS
        };

        // 等待下一拍；唤醒会提前打断并立即重估相位。若唤醒恰发生在处理
        // 期间（notify 已错过），WAKE 标志保证补一次立即重估。
        let due = if *guard {
            Duration::ZERO
        } else {
            Duration::from_millis(wait_ms)
        };
        let (g, _) = WAKE_CV.wait_timeout(guard, due).unwrap();
        guard = g;
        *guard = false;
    }
}

/// 对应 C 版 `led_status_init()`
pub fn init() {
    // 本项目中 esp-idf-hal 外设句柄：LED 用 take()（gpio48），
    // debug_uart 用 steal() 取 uart1 + gpio43/44（U0TXD/U0RXD，经矩阵映射）；Wi-Fi 走裸 FFI 不占用
    let peripherals = Peripherals::take().expect("Peripherals::take (led is the only user)");
    let mut pin = PinDriver::output(peripherals.pins.gpio48)
        .expect("led gpio output config failed");
    set_level(&mut pin, false);

    std::thread::Builder::new()
        .name("led_status".into())
        .stack_size(4096)
        .spawn(move || led_task(pin))
        .expect("spawn led_status task");

    ::log::info!("Indicator LED initialized on GPIO{}", LED_GPIO);
}

/// 对应 C 版 `led_status_set()`
pub fn set(status: Status) {
    STATUS.store(status as u8, Ordering::SeqCst);
    wake();
}

/// 对应 C 版 `led_status_activity()`：2 秒快速闪烁窗口
pub fn activity() {
    *ACTIVITY_UNTIL.lock().unwrap() =
        Some(Instant::now() + Duration::from_millis(ACTIVITY_DURATION_MS));
    wake();
}
