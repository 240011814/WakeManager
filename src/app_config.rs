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


/* ---------- Wi-Fi CSI 近场感知（csi_presence） ----------
 * 产品语义（2026-10 定稿，范围=设备周围 ~1m）：**只报两个事件**——approach
 * （靠近）/ leave（离开）；不做「有人/无人」状态判定、不做人工基线校准。
 * 判定 = ESPectre 轻量运动内核（见 csi_features.rs）+ **近场电平门**（本
 * 产品特有：ESPectre 是全屋运动感应、没有距离概念；近场门用 RSSI 相对慢
 * 自适应基线的降幅引入「~1m 范围」——人体近场吸收/遮挡造成绝对电平下降，
 * 真机实测空场 -57~-58 dBm、1m 内 -62~-66 dBm）：
 *   - 运动内核：湍流特征 → 1s 窗口（lag-1 自相关 + IQR/均值）→ logistic
 *     融合 p；p 只作**运动佐证**（行走/坐下必然连续超阈数秒），不单独定事件；
 *   - 近场区状态机：降幅 ≥ENTER 保持 ENTER_HOLD + 佐证 → approach（进区）；
 *     降幅回到 EXIT 内保持 EXIT_HOLD + 佐证 → leave（出区）；区状态翻转才
 *     发事件 → 区内坐着/打字不重复触发、不因停止运动误发 leave；远处走动
 *     电平不动 → 不触发。一拍 = CSI_EVAL_INTERVAL_MS。 */

/// 检测线程评估周期（ms）：一拍 = 250ms（ESPectre 生产配置）
pub const CSI_EVAL_INTERVAL_MS: u64 = 250;
/// 判定窗口时长（ms）：最近 1s 的准入样本（≈100 样本，对齐拟合标定）
pub const CSI_WINDOW_MS: u64 = 1_000;
/// 窗口最少样本数（不足不判定；探测流量偏低时防误判）。目标 100pps 下
/// 窗口 ≈ 100 样本；下限 20 是「帧率打折也别停判定」的兜底（现场实测
/// 曾因下限 40 > 实际 20 帧/s 而整场不评估）
pub const CSI_WINDOW_MIN_FRAMES: usize = 20;
/// 帧历史上限（FrameStat 约 24B/帧）
pub const CSI_HIST_CAP: usize = 256;
/// 运动概率阈值（ESPectre 轻量检测器拟合默认值 0.6622；现仅作运动佐证）
pub const CSI_MOTION_THRESHOLD: f32 = 0.662_185_45;
/// 近场门进入阈值（dB）：窗口 RSSI 中值比慢基线低这么多 → 判「近场」。
/// 真机实测：空场 -57~-58 dBm，人走近 1m 内 -62~-66（降 4~8dB）；3.0 =
/// 约 1~1.5m 的保守门槛（物理检测范围直接由此调）
pub const CSI_NEAR_ENTER_DB: f32 = 3.0;
/// 近场门退出阈值（dB）：降幅回到这个值以内 → 判「已离开近场」；
/// 与进入阈值间留 1.5dB 滞后带防边界抖动
pub const CSI_NEAR_EXIT_DB: f32 = 1.5;
/// 进入保持时长（ms）：降幅连续保持这么久才确认 approach
pub const CSI_NEAR_ENTER_HOLD_MS: u64 = 1_500;
/// 退出保持时长（ms）：电平恢复到基线附近持续这么久才确认 leave
pub const CSI_NEAR_EXIT_HOLD_MS: u64 = 3_000;
/// 运动佐证拍数（最近 CSI_NEAR_MOTION_MEMORY_TICKS 拍内 p 超阈的拍数）：
/// approach 需 ≥2、leave 需 ≥3 拍——纯电平漂移（AP 功率/AGC 阶跃）不带
/// 运动证据不误报；真行走的 p 连续高企数秒（实测 0.7~1.0），远超门槛
pub const CSI_NEAR_ENTER_MOTION_TICKS: u32 = 2;
pub const CSI_NEAR_EXIT_MOTION_TICKS: u32 = 3;
/// 运动佐证记忆长度（拍）：32 拍 = 8s
pub const CSI_NEAR_MOTION_MEMORY_TICKS: u32 = 32;
/// 基线环长度（拍，1 拍 = CSI_EVAL_INTERVAL_MS）：每拍一个 1s 窗口 RSSI
/// 中值样本；240 拍 = 60s 慢自适应「空场」参考电平，只统计非近场时刻
/// （近场吸收样本不污染基线）
pub const CSI_BASE_RING_LEN: usize = 240;
/// 基线就绪所需最少样本（拍）：32 拍 = 8s；未就绪不判定事件
pub const CSI_BASE_MIN_TICKS: usize = 32;
/// 基线向上快重锚（dB / ms）：RSSI 比基线高 4dB 持续 6s（人离开后基线仍
/// 停在占用期电平、或 AP 功率上调）→ 清环以当前电平重锚。只向上——向下
/// 重锚会把已经在近场的人吞进基线导致漏报
pub const CSI_BASE_SNAP_UP_DB: f32 = 4.0;
pub const CSI_BASE_SNAP_UP_MS: u64 = 6_000;
/// Hampel 去野值窗宽（样本数，有效范围 3..11）
pub const CSI_HAMPEL_WINDOW: usize = 7;
/// Hampel 判野值阈值（×MAD×1.4826）
pub const CSI_HAMPEL_THRESHOLD: f32 = 5.0;
/// 开机热身（ms）：窗口填充 + 启动瞬时流量（SNTP/MQTT 连接）抑制，
/// 热身期内不评估、不触发事件
pub const CSI_WARMUP_MS: u64 = 3_000;
/// 帧准入时隙（µs）：同一时隙只准入第一帧——把不定帧率规整成拟合节拍
/// 100pps（检测器常数按 100pps/1s 窗口标定）
pub const CSI_ADMIT_SLOT_US: i64 = 10_000;
/// CSI 数据断流告警阈值（ms 无新帧告警一次：Wi-Fi 空闲或 CSI 被关闭）
pub const CSI_STALL_WARN_MS: u64 = 30_000;
/// CSI 探测 ping 间隔（ms）：**主帧源**——ICMP echo reply 是 AP 发的下行
/// HT 帧，稳定出 CSI；10ms = 100 包/s，对齐检测器 100pps/1s 窗口标定。
/// 目标 = 默认网关（拿不到时退公共 DNS）。ESPectre 默认 ping 流量模式同款；
/// 旧「UDP 打网关靠 AP ACK 出 CSI」方案在本硬件实测一帧不出（上行 ACK-CSI
/// 不可靠），已弃用
pub const CSI_PING_INTERVAL_MS: u32 = 10;
/// CSI 探测 ping 单包等待回应超时（ms）：`esp_ping` 是「发一包→同步等回应」
/// 结构，丢一个回应会按 timeout 阻塞整条节拍（1000ms 默认值 = 丢一包空窗
/// 1 秒）；压到 100ms：正常 LAN RTT 约几 ms 不受影响，丢包最多停 10 拍
pub const CSI_PING_TIMEOUT_MS: u32 = 100;
/// 探测流量 IP TOS（DSCP 46/EF，要求 WMM 高优先队列；ESPectre 同款，
/// 网关可能忽略，尽力而为不致命）
pub const CSI_PING_TOS: i32 = 46 << 2;
/// 钉住 STA 上行速率 6 Mbps 传统 OFDM（在 wifi_prov 的 esp_wifi_start 之前
/// 应用）：保持上行链路 OFDM/低速率稳定（探测包时延抖动小）；主帧源是下行
/// echo reply，此钉定只作链路稳定保障（ESPectre 的 6Mbps 档同款选项）
pub const CSI_TX_RATE_6M: bool = true;
/// 定期打印间隔（ms）：每 5s 打一行当前测量值，现场观察检测量/调阈值用；
/// 0 = 关闭定期打印（事件日志不受影响）
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
