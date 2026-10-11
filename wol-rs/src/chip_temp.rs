//! WakeManager - on-chip temperature sensor（对应 C 版 components/chip_temp）
//!
//! 错误码直接用 `esp_err_t`（与 C 版返回值一致）。
//! 句柄以 usize 存进 static（裸指针不满足 Sync；init 后到重启前一直有效）。

use std::sync::Mutex;

use esp_idf_svc::sys::*;

/// Measurement range：驱动拒绝无法覆盖的范围，保持在典型工作温度附近
const RANGE_MIN: i32 = -10;
const RANGE_MAX: i32 = 80;

static SENSOR: Mutex<Option<usize>> = Mutex::new(None);

fn handle() -> Option<temperature_sensor_handle_t> {
    SENSOR
        .lock()
        .unwrap()
        .map(|addr| addr as temperature_sensor_handle_t)
}

/// 对应 C 版 `chip_temp_init()`
pub fn init() -> Result<(), esp_err_t> {
    let mut guard = SENSOR.lock().unwrap();
    if guard.is_some() {
        return Ok(());
    }

    // C 宏 TEMPERATURE_SENSOR_CONFIG_DEFAULT(-10, 80) 在 bindgen 中不可见 → 手工构造
    let cfg = temperature_sensor_config_t {
        range_min: RANGE_MIN,
        range_max: RANGE_MAX,
        clk_src: soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_DEFAULT,
        flags: temperature_sensor_config_t__bindgen_ty_1 { allow_pd: 0 },
    };

    let mut h: temperature_sensor_handle_t = std::ptr::null_mut();
    unsafe {
        let err = temperature_sensor_install(&cfg, &mut h);
        if err != ESP_OK {
            ::log::error!("temperature_sensor_install failed: {err:#x}");
            return Err(err);
        }
        let err = temperature_sensor_enable(h);
        if err != ESP_OK {
            ::log::error!("temperature_sensor_enable failed: {err:#x}");
            temperature_sensor_uninstall(h);
            return Err(err);
        }
    }

    ::log::info!(
        "Internal temperature sensor enabled (range {RANGE_MIN}..{RANGE_MAX} C)"
    );
    *guard = Some(h as usize);
    Ok(())
}

/// 对应 C 版 `chip_temp_read()`
pub fn read() -> Result<f32, esp_err_t> {
    let h = handle().ok_or(ESP_ERR_INVALID_STATE)?;
    let mut out: f32 = 0.0;
    unsafe {
        let err = temperature_sensor_get_celsius(h, &mut out);
        if err != ESP_OK {
            return Err(err);
        }
    }
    Ok(out)
}
