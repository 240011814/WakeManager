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


/* ---------- Wi-Fi CSI 近场趋势探测（csi_presence） ----------
 * 产品语义（2026-10 定稿）：**只报两个事件**——幅度上升趋势 = 靠近
 * （approach），下降趋势 = 离开（leave）；不做「有人/无人」状态判定、
 * 不做基线校准。趋势 = 当前平滑幅度 vs N 秒前的相对变化，天然免疫慢漂移。 */

/// 检测线程评估周期（ms）
pub const CSI_EVAL_INTERVAL_MS: u64 = 500;
/// 判定窗口时长（ms）：最近 2s 帧的幅度均值/波动（**时长窗口**，帧率无关）
pub const CSI_WINDOW_MS: u64 = 2_000;
/// 窗口最少样本数（不足不判定；探测流量异常低时防误判）
pub const CSI_WINDOW_MIN_FRAMES: usize = 8;
/// 帧历史上限（FrameStat 约 16B/帧）
pub const CSI_HIST_CAP: usize = 256;
/// 趋势斜率比较间隔（ms）：slope = (当前平滑幅度 − N ms 前的值) / N ms 前的值
pub const CSI_TREND_WINDOW_MS: u64 = 3_000;
/// 判定「靠近」的斜率阈值：≥ +3%（带动作走近/动作峰的**快路径**，2s 内触发）
pub const CSI_TREND_ENTER: f32 = 0.03;
/// 斜率快路径需连续达标时长（ms，防抖）
pub const CSI_TREND_SUSTAIN_MS: u64 = 1_000;
/// 判定「靠近」的**水位路径**：平滑水位 ≥ 空场地板 ×(1+MARGIN) 持续达标。
/// 2%——安静走近/坐下时水位是 1~2 分钟慢慢爬升的（实测每 3s 斜率仅 +0.003，
/// 纯斜率判据拖 2 分钟才触发，真机教训），必须有水位路径
pub const CSI_APPROACH_MARGIN: f32 = 0.02;
/// 判定「靠近」水位路径需连续达标时长（ms）
pub const CSI_APPROACH_SUSTAIN_MS: u64 = 3_000;
/// 判定「离开」的水位阈值（迟滞带 [1.5%, 2%]）：平滑水位回落到空场地板 ×(1+MARGIN)
/// 以内。坐着的人静止水位只比空场高 ~4%、动作峰 +10~30%——纯下降斜率会被坐着时的
/// 动作回落误触发（3~5s 交替跳变教训），必须认「回到空场水位」
pub const CSI_LEAVE_MARGIN: f32 = 0.015;
/// 判定「离开」需连续达标时长（ms，防抖）
pub const CSI_LEAVE_SUSTAIN_MS: u64 = 3_000;
/// 判定「离开」还要求近 N ms 内有**活动**（rs ≥ CSI_MOTION_RS 或 |slope| ≥ 2%）：
/// 人离开必有行走动作；坐着时的水位下凹（实测可凹到空场水位）是安静的——
/// 活动限定才不会把下凹误报成离开（真机跳变教训）。开机首条 leave 免此要求
pub const CSI_LEAVE_MOTION_MS: u64 = 8_000;
/// 活动判定的 rs 阈值（std/amp）
pub const CSI_MOTION_RS: f32 = 0.05;
/// 空场地板 = 最近 N ms 平滑水位的**最小值**（滑动窗口；环境慢漂靠窗口滑出吸收。
/// 开机时人在设备旁会污染地板，人真正离开一次后自动校正）
pub const CSI_FLOOR_WINDOW_MS: u64 = 600_000;
/// 水位平滑系数（每评估周期 0.25 → τ≈1.7s，压掉动作毛刺保留水位）
pub const CSI_LEVEL_ALPHA: f32 = 0.25;
/// 事件最小间隔（ms，双方向共用；兜底残留跳变场景的连发）
pub const CSI_TREND_REFRACTORY_MS: u64 = 15_000;
/// 开机热身（ms）：忽略起始波动，同时覆盖斜率历史积累（比较需要 3s 历史）
pub const CSI_TREND_WARMUP_MS: u64 = 5_000;
/// CSI 数据断流告警阈值（ms 无新帧告警一次：Wi-Fi 空闲或 CSI 被关闭）
pub const CSI_STALL_WARN_MS: u64 = 30_000;
/// CSI 探测流量（ping 网关）间隔（ms）：echo reply 走 HT 速率**必出** CSI，是
/// 稳定帧源的兜底（UDP 探测的 ACK 在速率自适应落到 CCK 时会断流，实测 30s 空窗）；
/// 100ms = 10 帧/s 保底
pub const CSI_PING_INTERVAL_MS: u32 = 100;
/// CSI 探测 UDP 流量间隔（ms）：向默认网关单播小包，**每帧必被 AP MAC 层 ACK**
///（dump_ack_en 下 ACK 帧出 CSI），速率稳定、不依赖对端回应/ICMP 策略；
/// 50ms = 20 包/s（参考方案的 50~100 包/s 是双设备对发场景，单设备经 AP 回程
/// 20 包/s 已给足 2s 窗口 ~40 样本）
pub const CSI_PROBE_INTERVAL_MS: u64 = 50;
/// 定期打印间隔（ms）：每 5s 打一行当前基线与测量值，现场观察检测量/调阈值用；
/// 0 = 关闭定期打印（状态变化日志不受影响）
pub const CSI_LOG_INTERVAL_MS: u64 = 5_000;

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
