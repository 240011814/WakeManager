//! WakeManager - Wake-on-LAN magic packet sender
//! 对应 C 版 `components/wol/wol.c`
//!
//! Magic packet = 6 × 0xFF + 目标 MAC × 16（102 字节），UDP 广播发送。

use std::net::UdpSocket;

use crate::app_config as ac;
use crate::config_store;
use crate::netinfo;

/// Value used when the subnet broadcast cannot be derived.
const LIMITED_BROADCAST: &str = "255.255.255.255";

#[derive(Debug, PartialEq, Eq)]
pub enum WolError {
    InvalidArg,
    Io,
}

/// 对应 C 版 `wol_parse_mac()`：支持 "AA:BB:.."、"AA-BB-.."、12 位裸 hex
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    if s.is_empty() {
        return None;
    }

    let sep = if s.contains(':') {
        Some(':')
    } else if s.contains('-') {
        Some('-')
    } else {
        None
    };

    let parts: Vec<&str> = match sep {
        Some(d) => s.split(d).collect(),
        None => {
            if s.len() != 12 {
                return None;
            }
            (0..6)
                .map(|i| &s[i * 2..i * 2 + 2])
                .collect::<Vec<&str>>()
        }
    };

    if parts.len() != 6 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 2 || !p.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        mac[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(mac)
}

/// 对应 C 版 `wol_mac_to_str()`（大写、冒号分隔）
pub fn mac_to_str(mac: &[u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

/// 严格解析点分 IPv4（等价 inet_pton(AF_INET)）
pub fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let v: u16 = p.parse().ok()?;
        if v > 255 {
            return None;
        }
        out[i] = v as u8;
    }
    Some(out)
}

/// 对应 C 版 `wol_auto_broadcast()`：
/// 无可用地址时退化为 255.255.255.255，否则 (ip & mask) | ~mask（子网定向广播）
pub fn auto_broadcast() -> String {
    match netinfo::sta_ip_netmask() {
        Some((ip, mask)) => {
            let b = std::array::from_fn::<u8, 4, _>(|i| (ip[i] & mask[i]) | !mask[i]);
            format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
        }
        None => LIMITED_BROADCAST.to_string(),
    }
}

/// 对应 C 版 `wol_get_default_broadcast()`
pub fn get_default_broadcast() -> String {
    auto_broadcast()
}

/// 对应 C 版 `wol_send_to()`：
/// `dest_ip` 为空/None → 子网定向广播；无效 IP → InvalidArg
pub fn send_to(mac: &[u8; 6], dest_ip: Option<&str>) -> Result<(), WolError> {
    let mut packet = [0xFFu8; 6 + 6 * 16];
    for i in 0..16 {
        packet[6 + i * 6..6 + i * 6 + 6].copy_from_slice(mac);
    }

    let cfg = config_store::get();
    let port = if cfg.wol_udp_port != 0 {
        cfg.wol_udp_port
    } else {
        ac::WOL_UDP_PORT
    };

    let dest = match dest_ip {
        Some(ip) if !ip.is_empty() => parse_ipv4(ip).ok_or(WolError::InvalidArg)?,
        _ => parse_ipv4(&auto_broadcast()).unwrap_or([255, 255, 255, 255]),
    };
    let dest_str = format!("{}.{}.{}.{}", dest[0], dest[1], dest[2], dest[3]);

    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|_| WolError::Io)?;
    sock.set_broadcast(true).map_err(|_| WolError::Io)?;
    let sent = sock
        .send_to(&packet, format!("{dest_str}:{port}"))
        .map_err(|_| WolError::Io)?;
    if sent != packet.len() {
        return Err(WolError::Io);
    }

    log::info!(
        "Magic packet sent to {} ({}:{})",
        mac_to_str(mac),
        dest_str,
        port
    );
    Ok(())
}

/// 对应 C 版 `wol_send_str_to()`
pub fn send_str_to(mac_str: &str, dest_ip: Option<&str>) -> Result<(), WolError> {
    let mac = parse_mac(mac_str).ok_or_else(|| {
        log::error!("Invalid MAC address: {mac_str}");
        WolError::InvalidArg
    })?;
    send_to(&mac, dest_ip)
}
