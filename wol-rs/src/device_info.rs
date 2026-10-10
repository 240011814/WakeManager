//! WakeManager - device identity helpers（对应 C 版 components/device_info）

use esp_idf_svc::sys::*;

/// STA MAC（6 字节）。`esp_wifi_get_mac` 需在 esp_wifi_init 之后调用。
pub fn get_mac() -> [u8; 6] {
    let mut mac = [0u8; 6];
    unsafe {
        esp_wifi_get_mac(wifi_interface_t_WIFI_IF_STA, mac.as_mut_ptr());
    }
    mac
}

/// 设备 id：STA MAC 的 12 位大写 hex（对应 C 版 `device_info_get_id()`）
pub fn get_id() -> String {
    let m = get_mac();
    format!(
        "{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        m[0], m[1], m[2], m[3], m[4], m[5]
    )
}
