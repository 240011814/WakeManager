//! WakeManager - 串口命令分发
//!
//! `debug_uart` 把 UART1（板载 CH340K/USB 口，GPIO43/44）收到的字节拼成行后
//! 交给 `handle()`；响应回写 UART1 TX（同一物理口），同时关键动作记控制台 log。
//!
//! 命令集（先支持两个需求）：
//!   help        命令菜单
//!   status      设备状态查看
//!   csi         CSI 趋势探测状态查看（approach=靠近 / leave=离开）
//!   wifi-clear  清除 Wi-Fi 配置（SSID/密码）→ 重启 → 开机自动进 BLE 配网
//!
//! 扩展方式：在 `handle()` 的 match 里加分支 + 菜单里加一行即可。

use embedded_io::Write as _;
use esp_idf_svc::hal::uart::UartDriver;
use esp_idf_svc::sys::{esp_restart, esp_wifi_restore, ESP_OK};

use crate::status;

const MENU: &str = "\
==== WakeManager 串口命令 ====
help         显示本菜单
status       查看设备状态
csi          查看 CSI 趋势探测状态（approach=靠近 / leave=离开）
wifi-clear   清除 Wi-Fi 配置并重新进入配网模式，完成后设备将重启";

/// 回写一行到串口（自动补 CRLF 并等 TX 排空，下一条响应不会插队）
pub(crate) fn write_line(driver: &mut UartDriver<'static>, text: &str) {
    let _ = driver.write_all(text.as_bytes());
    let _ = driver.write_all(b"\r\n");
    let _ = driver.flush();
}

/// 分发一条完整命令（`line` 已 trim、非空）
pub fn handle(driver: &mut UartDriver<'static>, line: &str) {
    ::log::info!("serial cmd: {line}");
    match line {
        "help" | "cmd" | "menu" | "?" => write_line(driver, MENU),
        "status" | "st" => write_line(driver, &status_text()),
        "csi" => write_line(driver, &crate::csi_presence::snapshot_text()),
        "wifi-clear" | "reset-wifi" => wifi_clear(driver),
        other => write_line(driver, &format!("未知命令: {other}\r\n\r\n{MENU}")),
    }
}

/// status 命令输出：与 `/api/status` 同一份数据的可读文本版
fn status_text() -> String {
    let s = status::device_status();
    let rssi = if s.rssi == 0 {
        // 未连接时 device_status() 给默认值 0，标注出来避免误读
        "n/a".to_string()
    } else {
        format!("{} dBm", s.rssi)
    };
    let temp = s
        .temp_c
        .map(|t| format!("{t:.1} C"))
        .unwrap_or_else(|| "n/a".to_string());
    let time = s.time.unwrap_or_else(|| "未同步".to_string());
    let boot = s.boot_time.unwrap_or_else(|| "n/a".to_string());
    format!(
        "id={}\r\nversion={}\r\nip={}\r\nssid={}\r\nrssi={}\r\nheap={} B\r\nmqtt={}\r\n\
         broadcast={}\r\nmacs={}\r\npresence={}\r\ntemp={}\r\ntime={}\r\nboot={}",
        s.id,
        s.version,
        s.ip,
        if s.ssid.is_empty() { "n/a" } else { &s.ssid },
        rssi,
        s.heap,
        if s.mqtt_connected { "up" } else { "down" },
        s.broadcast,
        s.mac_count,
        s.presence,
        temp,
        time,
        boot,
    )
}

/// wifi-clear：清 Wi-Fi 配置后重启。开机时配网管理器检测到 STA SSID 为空
/// （manager.c 的 is_wifi_provisioned 判定），自动重新进入 BLE 配网模式。
fn wifi_clear(driver: &mut UartDriver<'static>) {
    write_line(
        driver,
        "已清除 Wi-Fi 配置（SSID/密码），设备即将重启并进入 BLE 配网模式...",
    );
    ::log::warn!("serial cmd wifi-clear: restoring Wi-Fi config, rebooting");
    let err = unsafe { esp_wifi_restore() };
    if err != ESP_OK {
        ::log::error!("esp_wifi_restore failed: {err:#x}");
        write_line(driver, &format!("清除失败: {err:#x}（设备未重启）"));
        return;
    }
    // 给 TX 排空与控制台 log 落盘留一点时间，再重启
    std::thread::sleep(std::time::Duration::from_millis(300));
    unsafe { esp_restart() };
}
