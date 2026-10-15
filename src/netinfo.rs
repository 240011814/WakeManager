//! WakeManager - 站点网络信息查询（netif / Wi-Fi 链路状态）
//! 供 status JSON、广播地址推导使用（对应 C 版各处的 esp_netif/esp_wifi 调用）

use esp_idf_svc::sys::*;

/// STA 接口当前 IPv4 与子网掩码（网络字节序 → [a,b,c,d]），无可用地址返回 None
pub fn sta_ip_netmask() -> Option<([u8; 4], [u8; 4])> {
    unsafe {
        let netif = esp_netif_get_handle_from_ifkey(c"WIFI_STA_DEF".as_ptr());
        if netif.is_null() {
            return None;
        }
        let mut ip_info: esp_netif_ip_info_t = std::mem::zeroed();
        if esp_netif_get_ip_info(netif, &mut ip_info) != ESP_OK {
            return None;
        }
        if ip_info.ip.addr == 0 || ip_info.netmask.addr == 0 {
            return None;
        }
        // s_addr 为网络字节序；Xtensa 小端 → to_le_bytes 即内存序 a.b.c.d
        Some((ip_info.ip.addr.to_le_bytes(), ip_info.netmask.addr.to_le_bytes()))
    }
}

/// STA 接口当前 IPv4 文本（无地址时 "0.0.0.0"，与 C 版一致）
pub fn sta_ip() -> String {
    match sta_ip_netmask() {
        Some((ip, _)) => format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),
        None => "0.0.0.0".to_string(),
    }
}

/// STA 默认网关 IPv4（网络字节序 → [a,b,c,d]），无网关返回 None
pub fn sta_gateway() -> Option<[u8; 4]> {
    unsafe {
        let netif = esp_netif_get_handle_from_ifkey(c"WIFI_STA_DEF".as_ptr());
        if netif.is_null() {
            return None;
        }
        let mut ip_info: esp_netif_ip_info_t = std::mem::zeroed();
        if esp_netif_get_ip_info(netif, &mut ip_info) != ESP_OK {
            return None;
        }
        if ip_info.gw.addr == 0 {
            return None;
        }
        Some(ip_info.gw.addr.to_le_bytes())
    }
}

/// 当前连接的 SSID 与 RSSI（未连接返回 None，与 `esp_wifi_sta_get_ap_info` 失败一致）
pub fn sta_ssid_rssi() -> Option<(String, i8)> {
    unsafe {
        let mut ap: wifi_ap_record_t = std::mem::zeroed();
        if esp_wifi_sta_get_ap_info(&mut ap) != ESP_OK {
            return None;
        }
        let len = ap.ssid.iter().position(|&b| b == 0).unwrap_or(32);
        let ssid = String::from_utf8_lossy(&ap.ssid[..len]).into_owned();
        Some((ssid, ap.rssi))
    }
}

/// 空闲堆（对应 esp_get_free_heap_size）
pub fn free_heap() -> u32 {
    unsafe { esp_get_free_heap_size() }
}
