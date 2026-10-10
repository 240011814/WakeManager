//! WakeManager - status indicator LED（对应 C 版 components/led_status）
//!
//! 一个后台线程独占 LED，根据 `set()` 的状态闪烁；
//! MQTT 活动通过 `activity()` 叠加一段快速闪烁。

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use esp_idf_svc::sys::*;

/* --- Indicator configuration ------------------------------------------- */
const LED_GPIO: i32 = gpio_num_t_GPIO_NUM_48;
const LED_ACTIVE_LEVEL: u32 = 0; // active-low
const LED_OFF_LEVEL: u32 = 1;
const LED_BLINK_MS: u64 = 500; // half period of the "connecting" blink

const ACTIVITY_BLINK_MS: u64 = 100;
const ACTIVITY_DURATION_MS: u64 = 2000;
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

fn set_level(on: bool) {
    let level = if on { LED_ACTIVE_LEVEL } else { LED_OFF_LEVEL };
    unsafe {
        gpio_set_level(LED_GPIO, level);
    }
}

fn led_task() {
    let mut on = false;
    loop {
        let mut guard = ACTIVITY_UNTIL.lock().unwrap();
        let activity = matches!(*guard, Some(until) if Instant::now() < until);
        if !activity {
            *guard = None;
        }
        drop(guard);

        if activity {
            // MQTT 流量活动：快速闪烁，窗口结束后自动回到常规状态
            on = !on;
            set_level(on);
            std::thread::sleep(Duration::from_millis(ACTIVITY_BLINK_MS));
        } else if STATUS.load(Ordering::SeqCst) == 1 {
            // 已连接：常亮
            set_level(true);
            std::thread::sleep(Duration::from_millis(100));
        } else {
            // 配网 / 重连中：慢闪
            on = !on;
            set_level(on);
            std::thread::sleep(Duration::from_millis(LED_BLINK_MS));
        }
    }
}

/// 对应 C 版 `led_status_init()`
pub fn init() {
    let cfg = gpio_config_t {
        pin_bit_mask: 1u64 << LED_GPIO,
        mode: gpio_mode_t_GPIO_MODE_OUTPUT,
        pull_up_en: gpio_pullup_t_GPIO_PULLUP_DISABLE,
        pull_down_en: gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
        intr_type: gpio_int_type_t_GPIO_INTR_DISABLE,
    };
    unsafe {
        let err = gpio_config(&cfg);
        assert_eq!(err, ESP_OK, "gpio_config failed: {err:#x}");
        gpio_set_level(LED_GPIO, LED_OFF_LEVEL);
    }

    std::thread::Builder::new()
        .name("led_status".into())
        .stack_size(4096)
        .spawn(|| led_task())
        .expect("spawn led_status task");

    ::log::info!("Indicator LED initialized on GPIO{}", LED_GPIO);
}

/// 对应 C 版 `led_status_set()`
pub fn set(status: Status) {
    STATUS.store(status as u8, Ordering::SeqCst);
}

/// 对应 C 版 `led_status_activity()`：2 秒快速闪烁窗口
pub fn activity() {
    *ACTIVITY_UNTIL.lock().unwrap() = Some(Instant::now() + Duration::from_millis(ACTIVITY_DURATION_MS));
}
