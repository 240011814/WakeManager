//! GPIO45/46 调试/通信串口（UART1）：周期状态行 + 收发回环（echo）
//!
//! 引脚：GPIO45 = TX、GPIO46 = RX，115200 8N1 无流控（走 GPIO 矩阵，
//! 与 UART0 控制台的 GPIO43/44 互不冲突）。外部 USB-TTL 接
//! RX←GPIO45 + GND 即可收；把 TTL 的 TX 接 GPIO46 后，本模块会把收到的
//! 字节原样回发（echo），用于验证链路。
//!
//! 注意：所有 log 仍走控制台（UART0 GPIO43/44 + USB-Serial-JTAG），
//! 状态行同时写到这两个地方，方便对照。

use std::time::{Duration, Instant};

use embedded_io::Write as _;
use esp_idf_svc::hal::delay::TickType;
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::uart::{
    config::{DataBits, StopBits},
    UartConfig, UartDriver,
};
use esp_idf_svc::hal::units::Hertz;
use esp_idf_svc::sys::{esp_get_minimum_free_heap_size, esp_timer_get_time};

use crate::{chip_temp, control_mqtt, netinfo};

// ESP32-S3 的 GPIO 矩阵最大 GPIO48（SOC_GPIO_PIN_COUNT=49），无 49/50。
// GPIO45/46 是启动 strapping 脚（45=VDD_SPI 电压选择、46=ROM 打印控制），
// 仅在上电采样时刻有要求，之后可正常作 UART 用；若你的模块带八线 PSRAM
// 或板子把 46 接了特殊电路，换脚只改这两个常量。
// TX/RX 接反时对调即可（设备 TX 应接 USB-TTL 的 RX）。
const TX_GPIO: u8 = 45;
const RX_GPIO: u8 = 46;
const BAUD: u32 = 115_200;

/// UART1 收包轮询 / echo 间隔（既决定 echo 延迟，也是状态行的调度粒度）
const ECHO_POLL_MS: u64 = 50;
/// 周期状态行的间隔（秒）
const STATUS_PERIOD_S: u64 = 10;

/// 启动调试串口线程（独立线程，不阻塞主流程）
pub fn start() {
    std::thread::Builder::new()
        .name("dbg_uart".into())
        // 栈必须 ≥8192：reporter 每轮做 UartDriver::new（uart_driver_install）
        // + sta_ssid_rssi（栈上 wifi_ap_record_t）+ chip_temp/format!。
        // pthread 默认栈仅 4096（见 sdkconfig.defaults），会栈金丝雀整机复位。
        .stack_size(8192)
        .spawn(reporter)
        .expect("spawn debug uart reporter");
    ::log::info!(
        "Debug UART started: UART1 TX=GPIO{TX_GPIO} RX=GPIO{RX_GPIO} {BAUD} 8N1, \
         echo + one status line per {STATUS_PERIOD_S}s"
    );
}

fn reporter() {
    // Peripherals::take() 已被 led_status::init() 消耗；uart1 与 TX/RX 引脚
    // （gpio45/46）无人使用，这里 steal 出来本线程独占（驱动在本线程内创建
    // 并持有到进程结束）
    let peri = unsafe { Peripherals::steal() };
    let config = UartConfig::default()
        .baudrate(Hertz(BAUD))
        .data_bits(DataBits::DataBits8)
        .parity_none()
        .stop_bits(StopBits::STOP1);
    // AnyIOPin::steal：直接按引脚号取（0~48），TX_GPIO/RX_GPIO 换脚只改常量
    let tx = unsafe { AnyIOPin::steal(TX_GPIO) };
    let rx = unsafe { AnyIOPin::steal(RX_GPIO) };
    let mut driver = match UartDriver::new(
        peri.uart1,
        tx,
        rx,
        None::<AnyIOPin>, // 无 CTS
        None::<AnyIOPin>, // 无 RTS
        &config,
    ) {
        Ok(d) => Some(d),
        Err(e) => {
            ::log::error!(
                "UART1 init failed (GPIO{TX_GPIO}/GPIO{RX_GPIO}): {e:?} — status lines will only go to console"
            );
            None
        }
    };

    let mut seq: u64 = 0;
    let mut buf = [0u8; 128];
    // 立即发第一条状态行，之后每 STATUS_PERIOD_S 秒一条
    let mut next_status = Instant::now();

    loop {
        // ---- 收发回环：把 UART1 收到的字节原样回发（用 USB-TTL 验证链路）----
        // read 带超时，既当收包轮询又保证状态行准点；ECHO_POLL_MS 决定 echo 延迟
        if let Some(d) = driver.as_mut() {
            if let Ok(n) = d.read(&mut buf, TickType::new_millis(ECHO_POLL_MS).ticks()) {
                if n > 0 {
                    let _ = d.write(&buf[..n]);
                }
            }
        } else {
            // UART1 未起来：不轮询，按同一节拍睡，避免忙等
            std::thread::sleep(Duration::from_millis(ECHO_POLL_MS));
        }

        // ---- 周期状态行：同时写 GPIO45（UART1 TX）与控制台日志 ----
        if Instant::now() >= next_status {
            next_status += Duration::from_secs(STATUS_PERIOD_S);
            seq += 1;

            // esp_timer_get_time 返回微秒（自启动起，含睡眠）
            let up_s = (unsafe { esp_timer_get_time() } as u64) / 1_000_000;
            let heap = netinfo::free_heap();
            let min_heap = unsafe { esp_get_minimum_free_heap_size() };
            let temp = match chip_temp::read() {
                Ok(t) => format!("{t:.1}C"),
                Err(_) => "n/a".to_string(),
            };
            let ip = netinfo::sta_ip();
            let rssi = match netinfo::sta_ssid_rssi() {
                Some((_, r)) => format!("{r}dBm"),
                None => "n/a".to_string(),
            };
            let mqtt = if control_mqtt::is_connected() {
                "up"
            } else {
                "down"
            };

            let line = format!(
                "[UartTest #{seq}] up={up_s}s heap={heap} min={min_heap} temp={temp} ip={ip} rssi={rssi} mqtt={mqtt}\r\n"
            );

            if let Some(d) = driver.as_mut() {
                let _ = d.write_all(line.as_bytes());
                let _ = d.flush();
            }
            ::log::info!("{}", line.trim_end());
        }
    }
}