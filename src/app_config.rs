//! WakeManager - build-time / user tunable configuration
//! 对应 C 版 `components/app_config/include/app_config.h`

/* ---------- Wake-on-LAN ---------- */

/// UDP port for the magic packet (9 = classic wakeonlan)
pub const WOL_UDP_PORT: u16 = 9;

/* ---------- HTTP control server ---------- */

pub const WOL_HTTP_PORT: u16 = 80;
pub const WOL_HTTP_ENABLE: bool = true;

/* ---------- MQTT control ---------- */

pub const WOL_MQTT_ENABLE: bool = true;
pub const WOL_MQTT_BROKER_URI: &str = "mqtt://demonkingqianjing.cloud:21883";
pub const WOL_MQTT_CLIENT_ID: &str = "wakemanager";
pub const WOL_MQTT_USERNAME: &str = "";
pub const WOL_MQTT_PASSWORD: &str = "";

pub const WOL_MQTT_CMD_TOPIC: &str = "wakemanager/cmd";
pub const WOL_MQTT_BROADCAST_TOPIC: &str = "wakemanager/broadcast";
pub const WOL_MQTT_STATUS_TOPIC: &str = "wakemanager/status";
pub const WOL_MQTT_MAC_TOPIC: &str = "wakemanager/mac";
pub const WOL_MQTT_MAC_GET_TOPIC: &str = "wakemanager/mac/get";
/// OTA 升级命令主题（每设备追加 /<id>）：check / upgrade / status
pub const WOL_MQTT_OTA_TOPIC: &str = "wakemanager/ota";

/* ---------- Time sync (SNTP) ---------- */

pub const NTP_SERVER: &str = "pool.ntp.org";
/// POSIX timezone string (UTC+8)
pub const TIME_TIMEZONE: &str = "CST-8";

/* ---------- Watchdog ---------- */

pub const WATCHDOG_TIMEOUT_MS: u32 = 30_000;
pub const WATCHDOG_TRIGGER_PANIC: bool = true;

/* ---------- Wi-Fi provisioning (BLE transport) ----------
 * （sim 模拟构建跳过配网，这些常量随之不用） */
#[cfg(not(feature = "sim"))]
pub const PROV_POP: &str = "12345678";
#[cfg(not(feature = "sim"))]
pub const PROV_DEVICE_NAME_PREFIX: &str = "PROV_";
#[cfg(not(feature = "sim"))]
pub const PROV_QR_VERSION: &str = "v1";
#[cfg(not(feature = "sim"))]
pub const PROV_QR_TRANSPORT: &str = "ble";
#[cfg(not(feature = "sim"))]
pub const PROV_QRCODE_BASE_URL: &str =
    "https://espressif.github.io/esp-jumpstart/qrcode.html";

/* ---------- OTA (GitHub Releases) ---------- */

/// 构建时注入的版本号（CI 传 WOL_VERSION，本地为 "dev"）
pub const VERSION: &str = env!("WOL_VERSION");

/// OTA 更新源仓库的 Release 中裸固件资产名（CI 上传）
pub const OTA_ASSET_NAME: &str = "wol-rs.bin";
/// 版本检查 API（GitHub releases/latest；仓库写死于此常量）
pub const OTA_CHECK_URL: &str = "https://api.github.com/repos/240011814/WakeManager/releases/latest";


/* ---------- Address book ---------- */

/// Maximum number of stored MAC/IP entries.
pub const WM_MAC_STORE_MAX: usize = 10;

/* ---------- Config store field sizes（与 C 的 char 数组一致）---------- */

pub const WM_CFG_MAC_LEN: usize = 18;
pub const WM_CFG_HOST_LEN: usize = 128;
pub const WM_CFG_USER_LEN: usize = 64;
pub const WM_CFG_PASS_LEN: usize = 64;
/// String length for a dotted IPv4 address (including NUL in C).
pub const WM_MAC_IP_LEN: usize = 16;
