//! U0TXD/U0RXD（GPIO43/44）调试串口：收包 → 文本打印到控制台 log
//!
//! 引脚：GPIO43 = U0TXD（TX）、GPIO44 = U0RXD（RX），115200 8N1 无流控。
//! 这两个脚就是板载 CH340K/USB 调试口的物理走线，所以从板载 USB 就能收。
//! 本模块用 UART1 外设经 GPIO 矩阵映射到 43/44，收到的字节按 UTF-8（lossy）
//! 转文本打到控制台 log（经 USB-Serial-JTAG 镜像到 /dev/cu.usbmodem*）。
//!
//! 注意：GPIO43/44 也是 UART0 控制台的默认脚；映射到 UART1 后，控制台在
//! GPIO43 上的输出会被顶掉（log 仍会走 USB-Serial-JTAG 次级控制台）。

use std::time::Duration;

use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::uart::{
    config::{DataBits, StopBits},
    UartConfig, UartDriver,
};
use esp_idf_svc::hal::units::Hertz;
use esp_idf_svc::sys::ESP_ERR_TIMEOUT;

// 直接使用 U0TXD/U0RXD（GPIO43/44）——板载 CH340K/USB 调试口的走线脚。
// 注意：这两个脚同时是 UART0 控制台的默认脚，UART1 映射上来后会顶掉
// GPIO43 上的控制台输出（log 仍走 USB-Serial-JTAG 次级控制台）。
const TX_GPIO: u8 = 43;
const RX_GPIO: u8 = 44;
const BAUD: u32 = 115_200;

/// 收包轮询间隔：read 带超时阻塞到这里长，既是收包调度粒度也是线程节拍
const RX_POLL_MS: u64 = 50;
/// 一次 read 的缓冲大小（满速 50ms 约可收 575 字节，超量由驱动 RX 环形缓冲暂存）
const RX_BUF: usize = 512;

/// 启动调试串口线程（独立线程，不阻塞主流程）
pub fn start() {
    std::thread::Builder::new()
        .name("dbg_uart".into())
        // 栈必须 ≥8192（实测 pthread 默认 4096 会栈金丝雀整机复位）：
        // UartDriver::new（uart_driver_install）+ format!/字符串分配都在本线程栈上。
        .stack_size(8192)
        .spawn(reporter)
        .expect("spawn debug uart reporter");
    ::log::info!(
        "Debug UART started: UART1 TX=GPIO{TX_GPIO} RX=GPIO{RX_GPIO} {BAUD} 8N1, rx → console log"
    );
}

fn reporter() {
    // Peripherals::take() 已被 led_status::init() 消耗；这里 steal 出 uart1 与
    // TX/RX 引脚（gpio43/44）。GPIO43/44 虽被 UART0 控制台占用，但把 UART1
    // 经 GPIO 矩阵映射上去即接管其物理走线（板载 CH340K/USB 调试口）。
    let peri = unsafe { Peripherals::steal() };
    let config = UartConfig::default()
        .baudrate(Hertz(BAUD))
        .data_bits(DataBits::DataBits8)
        .parity_none()
        .stop_bits(StopBits::STOP1);
    // AnyIOPin::steal：直接按引脚号取（0~48），TX_GPIO/RX_GPIO 换脚只改常量
    let tx = unsafe { AnyIOPin::steal(TX_GPIO) };
    let rx = unsafe { AnyIOPin::steal(RX_GPIO) };
    match UartDriver::new(
        peri.uart1,
        tx,
        rx,
        None::<AnyIOPin>, // 无 CTS
        None::<AnyIOPin>, // 无 RTS
        &config,
    ) {
        Ok(driver) => receive_loop(driver),
        Err(e) => ::log::error!(
            "UART1 init failed (GPIO{TX_GPIO}/GPIO{RX_GPIO}): {e:?} — rx log disabled"
        ),
    }
}

fn receive_loop(driver: UartDriver<'static>) {
    let mut buf = [0u8; RX_BUF];
    loop {
        match driver.read(&mut buf, TickType::new_millis(RX_POLL_MS).ticks()) {
            // hal 把“超时无数据”映射为 Err(Timeout)（uart.rs 文档），属正常空闲态
            Err(e) if e.code() == ESP_ERR_TIMEOUT as i32 => {}
            Ok(0) => {}
            Ok(n) => {
                // lossy：非法 UTF-8 序列替换为 U+FFFD，不会 panic
                let text = String::from_utf8_lossy(&buf[..n]);
                ::log::info!("uart1 rx ({n}B): {text}");
            }
            Err(e) => {
                // 出错立即返回 → 必须 sleep，否则线程忙等烧满一个核
                ::log::error!("UART1 read failed: {e:?}");
                std::thread::sleep(Duration::from_millis(RX_POLL_MS));
            }
        }
    }
}
