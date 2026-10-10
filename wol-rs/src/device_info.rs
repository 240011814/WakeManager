//! WakeManager - device identity helpers（对应 C 版 components/device_info）

use esp_idf_svc::sys::*;

/// STA MAC（6 字节）。`esp_wifi_get_mac` 需在 esp_wifi_init 之后调用。
pub fn get_mac() -> Result<[u8; 6], EspError> {
    let mut mac = [0u8; 6];
    let err = unsafe { esp_wifi_get_mac(wifi_interface_t_WIFI_IF_STA, mac.as_mut_ptr()) };
    EspError::convert(err)?;
    Ok(mac)
}

/// 设备 id：STA MAC 的 12 位大写 hex（对应 C 版 `device_info_get_id()`）。
/// MAC 读取失败时回退为全零 id（与 C 版忽略错误的行为一致）。
pub fn get_id() -> String {
    match get_mac() {
        Ok(m) => format!(
            "{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            m[0], m[1], m[2], m[3], m[4], m[5]
        ),
        Err(e) => {
            ::log::warn!("esp_wifi_get_mac failed while building device id: {e:?}");
            "000000000000".to_string()
        }
    }
}