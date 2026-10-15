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
/// CSI 近场感知通知主题（固定主题，不追加设备 id，消息内含 id 字段）
pub const WOL_MQTT_CSI_NOTIFY_TOPIC: &str = "csi/notify";

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


/* ---------- Wi-Fi CSI 近场人体感知（csi_presence） ----------
 * 「1 米内有人」= 近场遮挡近似：人体贴近使接收幅度相对无人基线显著衰减。
 * 阈值需按现场（AP 距离/摆放）微调，串口 `csi` 命令可实时观察各量。 */

/// 检测线程评估周期（ms）
pub const CSI_EVAL_INTERVAL_MS: u64 = 500;
/// 开机基线校准所需帧数（CSI 帧来自 AP beacon/下行帧，约 2~10 帧/s）
pub const CSI_CALIB_FRAMES: usize = 40;
/// 判定窗口帧数（最近 N 帧的幅度均值/波动与基线比较，约 2~10s）
pub const CSI_WINDOW_FRAMES: usize = 20;
/// 环形缓冲/历史保留帧数上限
pub const CSI_HIST_CAP: usize = 512;
/// 判定「有人」的幅度衰减阈值：窗口均值比基线低 10% 以上
pub const CSI_ENTER_DROP: f32 = 0.10;
/// 判定「无人」的衰减阈值（迟滞）：回落到 5% 以内
pub const CSI_LEAVE_DROP: f32 = 0.05;
/// 进入「有人」需连续达标时长（ms，防抖）
pub const CSI_ENTER_SUSTAIN_MS: u64 = 2_000;
/// 退回「无人」需连续达标时长（ms，防抖）
pub const CSI_LEAVE_SUSTAIN_MS: u64 = 6_000;
/// 「无人」状态基线自适应系数（每评估周期 0.02 → τ≈25s，吸收环境慢漂移；
/// 「有人」状态基线冻结，否则久坐的人会被吸进基线导致漏检）
pub const CSI_BASELINE_ALPHA: f32 = 0.02;
/// CSI 数据断流告警阈值（ms 无新帧告警一次：Wi-Fi 空闲或 CSI 被关闭）
pub const CSI_STALL_WARN_MS: u64 = 30_000;

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
