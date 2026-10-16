//! WakeManager - Wi-Fi CSI 近场感知（靠近 / 离开两个事件）
//!
//! 产品语义（不变，范围=设备周围 ~1m）：**只报两个事件**——`approach`（靠近）
//! / `leave`（离开）；不做「有人/无人」状态判定；无人工校准（近场基线是
//! 60s 慢自适应窗中值，见下）。
//!
//! 检测内核（2026-10，参照 ESPectre 轻量检测器，Rust 独立重实现，
//! 数值公式见 `csi_features.rs`）+ **近场电平门（范围控制）**：
//!   - 每帧空间湍流 = 12 个固定子载波幅度的 std/mean（免疫 AGC/慢漂移），
//!     以及五 bin 邻域聚合湍流；Hampel（窗 7、5×MAD）去野值；
//!   - 1s 窗口两特征：湍流 lag-1 自相关 + 聚合湍流 IQR/均值；固定权重
//!     logistic 融合 → 运动概率 p（`CSI_MOTION_THRESHOLD`）——p 只作
//!     **运动佐证**：行走/坐下的 p 连续高企数秒，纯电平漂移没有运动证据；
//!   - **近场电平门**：窗口 RSSI 中值相对慢自适应基线（60s 环中值、只统计
//!     非近场时刻）的降幅 = 人体在 ~1m 内的近场吸收/遮挡（**绝对**电平变化；
//!     湍流特征是比值，天然对距离不敏感）。降幅 ≥`CSI_NEAR_ENTER_DB` 保持
//!     `CSI_NEAR_ENTER_HOLD_MS` + 运动佐证 → **approach**（进入近场区）；
//!     回到 `CSI_NEAR_EXIT_DB` 内保持 `CSI_NEAR_EXIT_HOLD_MS` + 佐证 →
//!     **leave**（离开近场区）。一拍 = `CSI_EVAL_INTERVAL_MS`（250ms）。
//!
//! **为什么这样定**（真机教训的背景）：① 空场湍流序列近似白噪声（自相关
//! ≈0、IQR 平缓）；人活动让多径扰动在时间上相关，自相关与 IQR 双双抬升，
//! p 作运动代理有效（旧「平滑幅度 vs 地板」方案在慢漂移/坐着静止场景翻车，
//! 故换增益无关特征）。② 但纯 p 边沿触发在真机反复横跳：人走近后一停步
//! p 立刻塌到 <0.1 → 数拍后误发 leave，再一动又 approach（实测 1~3s 一对
//! 事件）；且 p 对全屋任何运动敏感、没有距离概念。③ 因此事件改由「近场区
//! 状态机」定：进区/出区都要求电平跨门 + 保持 + 运动佐证，区状态翻转才发
//! 事件——区内坐着不动不误发 leave（电平没恢复），远处走动不触发（电平
//! 不动，范围门挡住）。
//!
//! 数据流（参照 RuView 的「回调只做轻量统计、重活在任务」的管线形态）：
//!   Wi-Fi 任务（CSI 回调：质量门（rx_state）→ 时隙准入 → 每帧特征，禁日志/
//!   分配）→ 环形缓冲 → 检测线程（Hampel → 1s 窗口 → 融合概率 + RSSI 中值
//!   → 近场区状态机）→ 事件 JSON 发 MQTT `csi/notify`：
//!   `{"id":..,"cmd":"csi","event":"approach|leave","p":..,"ac":..,"iqr":..,
//!   "turb":..,"frames":..}`
//!
//! **帧率与速率（2026-10 真机实测后照抄 ESPectre，修正自旧「ACK 主帧源」）**：
//!   - CSI 只对 **OFDM/HT** 帧生成；实测本硬件（S3 + 该 AP）**上行帧的
//!     ACK-CSI 一帧不出**（UDP 打网关 100 包/s 完全无帧）——旧方案退役。
//!   - 主帧源 = **ICMP ping 100pps**（`CSI_PING_INTERVAL_MS`=10ms）：echo
//!     reply 是 AP 发的下行 HT 帧，稳定出 CSI（ESPectre 默认 ping 流量模式；
//!     其 S3 目标连上行速率钉定都不需要，我们仍留 6M 只作链路稳定）。
//!   - CSI 采集配置同 ESPectre HT20 档：**只采 HT-LTF**（关 LLTF/STBC-
//!     HT-LTF2/合并/ACK dump、关信道滤波），载荷固定 128B 全宽——与特征
//!     流水线标定语料一致（此前全开配置出 384B 三份 LTF，属未标定形态）。
//!   - sdkconfig 同 ESPectre：`AMPDU_TX/RX=n`（TX A-MPDU 会让速率钉定
//!     失效）、`FREERTOS_HZ=1000`（10ms 节拍精度，esp_ping 用 vTaskDelayUntil）。
//!   - 时隙准入：同一 10ms 时隙只准入第一帧——把不定帧率规整成拟合节拍
//!     100pps（检测器常数按 100pps/1000ms 窗口标定）。
//!   - 帧率异常排查：5s 日志里 `rx=`（回调入口帧数）与
//!     `drop(rs=rx_state slot=时隙 parse=解析)` 分级计数。
//!
//! FFI：`esp_wifi_set_csi_config / esp_wifi_set_csi_rx_cb / esp_wifi_set_csi`
//! （需 sdkconfig 开 `CONFIG_ESP_WIFI_CSI_ENABLED`，本项目已开启；函数声明
//! 不随该开关门控，绑定始终可见）。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use esp_idf_svc::sys::*;

use crate::app_config as ac;
use crate::control_mqtt;
use crate::csi_features as feat;
use crate::device_info;
use crate::netinfo;

/// 一帧 CSI 的轻量特征（回调 → 检测线程；Hampel 前的原始值）
struct FrameStat {
    /// 频带空间湍流（12 子载波 std/mean）
    turb: f32,
    /// 五 bin 聚合湍流
    aggr: f32,
    rssi: i8,
    /// 原始载荷长度（诊断：128=全宽 HT、106=紧凑 LLTF、114=短 HT…）
    len: u16,
    /// 收帧时刻（时长窗口统计用，回调里盖戳）
    at: Instant,
}

/// 回调写入的环形缓冲（Wi-Fi 任务上下文，锁内只做 push/淘汰，不阻塞）
static RING: Mutex<VecDeque<FrameStat>> = Mutex::new(VecDeque::new());

/// 帧准入时隙号（ESPectre fixed temporal admission 的简化版）：同一时隙内
/// 只准入第一帧。仅 CSI 回调（Wi-Fi 任务）访问。Xtensa 目标无 64 位原子，
/// i32 槽号每 ~248 天绕一次——只做相邻帧相等性比较，绕圈无影响。
static LAST_SLOT: AtomicI32 = AtomicI32::new(-1);

/// 回调侧分级诊断计数（累积值，5s 日志打印；原子操作，回调内零锁零分配）：
/// 定位「帧到底来没来、丢在哪一级过滤」——rx 回调入口 → rs 质量门 →
/// slot 时隙 ➜ parse 载荷归一化 → admit 入窗（≈ threads 侧 frames 计数）
static DIAG_RX: AtomicU32 = AtomicU32::new(0);
static DIAG_RS: AtomicU32 = AtomicU32::new(0);
static DIAG_SLOT: AtomicU32 = AtomicU32::new(0);
static DIAG_PARSE: AtomicU32 = AtomicU32::new(0);
static DIAG_ADMIT: AtomicU32 = AtomicU32::new(0);

/// 检测线程每拍刷新的状态快照（串口 `csi` 命令读取）
#[derive(Clone, Copy)]
pub struct Snapshot {
    /// 最近一次事件："none" / "approach" / "leave"
    pub last: &'static str,
    /// 近场区状态："in"（人 ~1m 内）/ "out"（不在）
    pub state: &'static str,
    /// 融合运动概率 [0,1]（仅作佐证）
    pub p: f32,
    /// 湍流 lag-1 自相关
    pub ac: f32,
    /// 聚合湍流 IQR/均值
    pub iqr: f32,
    /// 窗口湍流均值（诊断）
    pub turb: f32,
    /// 窗口 RSSI 中值（dBm，四舍五入）
    pub rssi: i8,
    /// 基线就绪标志（false = 还在采样空场基线，不判定事件）
    pub base_ready: bool,
    /// 「空场」基线（RSSI 中值，dBm）
    pub base: f32,
    /// 近场降幅 = base − rssi_med（正 = 信号被压低 = 人近）
    pub drop: f32,
    /// 自本次启动收到的 CSI 帧数（饱和计数不回绕）
    pub frames: u64,
}

static SNAPSHOT: Mutex<Snapshot> = Mutex::new(Snapshot {
    last: "none",
    state: "out",
    p: 0.0,
    ac: 0.0,
    iqr: 0.0,
    turb: 0.0,
    rssi: 0,
    base_ready: false,
    base: 0.0,
    drop: 0.0,
    frames: 0,
});

/// 发布到 `csi/notify` 的事件消息
#[derive(serde::Serialize)]
struct NotifyMsg<'a> {
    id: &'a str,
    cmd: &'a str,
    /// "approach" = 靠近；"leave" = 离开
    event: &'a str,
    /// 触发时刻运动概率
    p: f32,
    /// 触发时刻湍流自相关
    ac: f32,
    /// 触发时刻聚合湍流 IQR/均值
    iqr: f32,
    /// 触发时刻窗口湍流均值
    turb: f32,
    frames: u64,
}

/// 事件 → JSON 通知经 MQTT 发布路径入队
fn publish_event(event: &str, p: f32, ac: f32, iqr: f32, turb: f32, frames: u64) {
    let id = device_info::get_id();
    let msg = NotifyMsg {
        id: &id,
        cmd: "csi",
        event,
        p,
        ac,
        iqr,
        turb,
        frames,
    };
    match serde_json::to_string(&msg) {
        Ok(json) => control_mqtt::publish_notify(&json),
        Err(e) => ::log::error!("CSI notify serialize failed: {e:?}"),
    }
}

/// CSI 接收回调（Wi-Fi 任务上下文）：质量门 → 时隙准入 → 每帧特征 → 入队，
/// **不得**日志/分配/阻塞；缓冲由检测线程消费。
unsafe extern "C" fn csi_rx_cb(_ctx: *mut core::ffi::c_void, data: *mut wifi_csi_info_t) {
    if data.is_null() {
        return;
    }
    let info = &*data;
    DIAG_RX.fetch_add(1, Ordering::Relaxed);
    // 质量门（ESPectre 同款）：硬件标记 rx_state != 0 的帧直接丢弃
    if info.rx_ctrl.rx_state() != 0 {
        DIAG_RS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    if info.buf.is_null() || info.len < 2 {
        DIAG_PARSE.fetch_add(1, Ordering::Relaxed);
        return;
    }
    // 时隙准入：同一 10ms 时隙只取第一帧（突发规整，节拍对齐 100pps 标定）
    let slot = (unsafe { esp_timer_get_time() } / ac::CSI_ADMIT_SLOT_US) as i32;
    if LAST_SLOT.swap(slot, Ordering::Relaxed) == slot {
        DIAG_SLOT.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let raw = core::slice::from_raw_parts(info.buf as *const i8, info.len as usize);
    let Some((turb, aggr)) = feat::frame_turbulence(raw, info.first_word_invalid) else {
        DIAG_PARSE.fetch_add(1, Ordering::Relaxed);
        return; // 载荷无法归一化（退化/稀疏帧）
    };
    DIAG_ADMIT.fetch_add(1, Ordering::Relaxed);
    let stat = FrameStat {
        turb,
        aggr,
        rssi: info.rx_ctrl.rssi() as i8,
        len: info.len,
        at: Instant::now(),
    };

    if let Ok(mut ring) = RING.lock() {
        if ring.len() >= ac::CSI_HIST_CAP {
            ring.pop_front();
        }
        ring.push_back(stat);
    }
}

fn update_snapshot(snap: Snapshot) -> Snapshot {
    if let Ok(mut g) = SNAPSHOT.lock() {
        *g = snap;
    }
    snap
}

/// 已排序切片的中值
fn median_sorted(v: &[f32]) -> f32 {
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) * 0.5
    }
}

/// 基线环中值（内部排序副本；环 ≤240 个 f32，每拍一次开销可忽略）
fn base_median(ring: &VecDeque<f32>) -> f32 {
    let mut v: Vec<f32> = ring.iter().copied().collect();
    v.sort_by(f32::total_cmp);
    median_sorted(&v)
}

/// 运动佐证位环是 u32 移位寄存器：记忆拍数不得超 32
const _: () = assert!(ac::CSI_NEAR_MOTION_MEMORY_TICKS <= 32);

/// 检测线程：Hampel → 1s 窗口特征 → 融合概率 → 命中滤波 → approach/leave 发通知
fn detector_task() {
    ::log::info!("CSI detector started (turbulence fusion + near-field RSSI zone gate)");
    let tick = Duration::from_millis(ac::CSI_EVAL_INTERVAL_MS);
    let window = Duration::from_millis(ac::CSI_WINDOW_MS);
    let warmup = Duration::from_millis(ac::CSI_WARMUP_MS);

    let mut hist: VecDeque<FrameStat> = VecDeque::new(); // 帧历史（时长窗口取最近段）
    let mut hampel_t = feat::Hampel::new(ac::CSI_HAMPEL_WINDOW, ac::CSI_HAMPEL_THRESHOLD);
    let mut hampel_a = feat::Hampel::new(ac::CSI_HAMPEL_WINDOW, ac::CSI_HAMPEL_THRESHOLD);
    let mut len_last: u16 = 0;
    let mut frames_total: u64 = 0;
    // ---- 近场电平门 / 近场区状态机（范围控制，2026-10 真机误触复盘后加入）----
    // 进区：RSSI 降幅 ≥ENTER_DB 保持 ENTER_HOLD + 运动佐证 → approach
    // 出区：降幅回到 ≤EXIT_DB 保持 EXIT_HOLD + 运动佐证 → leave
    // 区状态翻转才发事件；未就绪的拍不读数据、不推进计数（短暂断流不许
    // 把区状态推进/关掉）。
    let mut in_zone = false;
    let mut near_hold: u32 = 0;
    let mut far_hold: u32 = 0;
    let mut snap_hold: u32 = 0;
    // 运动佐证位环：每拍右移入位（p 超阈=1），最近 32 拍 = 8s
    let mut motion_bits: u32 = 0;
    // 慢自适应「空场」基线环（每拍一个 1s 窗口 RSSI 中值样本）
    let mut base_ring: VecDeque<f32> = VecDeque::new();
    let mut baseline: f32 = 0.0;
    let mut base_ready = false;
    let mut last_event: &'static str = "none";
    let mut last_frame_at = Instant::now();
    let mut stalled_warned = false;
    let mut last_print = Instant::now();
    let started_at = Instant::now();
    let mut p = 0f32;
    let mut ac = 0f32;
    let mut iqr = 0f32;
    let mut turb; // 每拍先赋值再读取（见循环内 n==0 分支）
    let mut n;

    loop {
        std::thread::sleep(tick);

        // ---- 取新帧：Hampel 去野值后入历史 ----
        let mut fresh: usize = 0;
        {
            let mut ring = RING.lock().unwrap();
            while let Some(f) = ring.pop_front() {
                let stat = FrameStat {
                    turb: hampel_t.filter(f.turb),
                    aggr: hampel_a.filter(f.aggr),
                    rssi: f.rssi,
                    len: f.len,
                    at: f.at,
                };
                if hist.len() >= ac::CSI_HIST_CAP {
                    hist.pop_front();
                }
                len_last = stat.len;
                hist.push_back(stat);
                // 饱和计数：u64 在 100 帧/s 下约 58 亿年到顶，saturating 只是
                // 兜底——绝不允许回绕成 0 干扰诊断显示
                frames_total = frames_total.saturating_add(1);
                fresh += 1;
            }
        }
        if fresh > 0 {
            last_frame_at = Instant::now();
            stalled_warned = false;
        } else if last_frame_at.elapsed() > Duration::from_millis(ac::CSI_STALL_WARN_MS) && !stalled_warned
        {
            ::log::warn!(
                "CSI data stalled (no frames for {}s): Wi-Fi quiet or CSI disabled",
                ac::CSI_STALL_WARN_MS / 1000
            );
            stalled_warned = true;
        }

        // ---- 窗口（近 `CSI_WINDOW_MS` 内准入样本，时间升序）----
        let now = Instant::now();
        let cutoff = now - window;
        let hist_cut = now - window * 2; // 历史保留 2 个窗口，足够取完整窗口段
        while matches!(hist.front(), Some(f) if f.at < hist_cut) {
            hist.pop_front();
        }
        let mut t_series: Vec<f32> = Vec::with_capacity(128);
        let mut a_series: Vec<f32> = Vec::with_capacity(128);
        let mut r_series: Vec<f32> = Vec::with_capacity(128);
        for f in hist.iter().rev() {
            if f.at < cutoff {
                break;
            }
            t_series.push(f.turb);
            a_series.push(f.aggr);
            r_series.push(f.rssi as f32);
        }
        t_series.reverse();
        a_series.reverse();
        n = t_series.len();
        let mut rssi_med = 0f32;
        if n > 0 {
            turb = t_series.iter().sum::<f32>() / n as f32;
            // 窗口 RSSI 中值（近场电平门用；中值抗单帧异常）
            r_series.sort_by(f32::total_cmp);
            rssi_med = median_sorted(&r_series);
        } else {
            turb = 0.0;
            p = 0.0;
            ac = 0.0;
            iqr = 0.0;
        }

        // ---- 就绪门：热身期与样本不足的拍不评估（保持区状态与计数）----
        let ready = n >= ac::CSI_WINDOW_MIN_FRAMES;
        if started_at.elapsed() >= warmup && ready {
            ac = feat::autocorr_lag1(&t_series);
            iqr = feat::iqr_over_mean(&a_series);
            p = feat::motion_probability(ac, iqr);

            // 运动佐证位环（8s 记忆；行走/坐下的 p 连续超阈，漂移不超阈）
            motion_bits = (motion_bits << 1) | u32::from(p > ac::CSI_MOTION_THRESHOLD);

            if !base_ready {
                // 基线采样期（首 ~8s）：无条件收样本，收满才武装事件判定
                base_ring.push_back(rssi_med);
                if base_ring.len() > ac::CSI_BASE_RING_LEN {
                    base_ring.pop_front();
                }
                if base_ring.len() >= ac::CSI_BASE_MIN_TICKS {
                    baseline = base_median(&base_ring);
                    base_ready = true;
                    ::log::info!(
                        "CSI RSSI baseline ready: {:.1} dBm (near-field gate armed)",
                        baseline
                    );
                }
            } else {
                let drop = baseline - rssi_med; // 正 = 信号被压低（人体近场吸收）
                let near = drop >= ac::CSI_NEAR_ENTER_DB;
                // 退出带上界 -SNAP_UP：电平蹿到基线 +4dB 以上（异常过冲，
                // 如贴着设备挥舞反射）不判「已离开」——防误发 leave
                let far = drop <= ac::CSI_NEAR_EXIT_DB && drop >= -ac::CSI_BASE_SNAP_UP_DB;
                near_hold = if near { near_hold + 1 } else { 0 };
                far_hold = if far { far_hold + 1 } else { 0 };

                // 基线慢自适应：只在非近场更新（近场样本不污染「空场」参考）；
                // 向上快重锚：电平高于基线 SNAP_UP 持续 SNAP_UP_MS（人离开后
                // 基线仍停在占用期电平、或 AP 功率上调）→ 清环重锚。只向上：
                // 向下重锚会把已经在近场的人吞进基线导致漏报
                if !near && !in_zone {
                    base_ring.push_back(rssi_med);
                    if base_ring.len() > ac::CSI_BASE_RING_LEN {
                        base_ring.pop_front();
                    }
                    baseline = base_median(&base_ring);
                    snap_hold = if drop <= -ac::CSI_BASE_SNAP_UP_DB {
                        snap_hold + 1
                    } else {
                        0
                    };
                    if snap_hold as u64 * ac::CSI_EVAL_INTERVAL_MS >= ac::CSI_BASE_SNAP_UP_MS {
                        base_ring.clear();
                        base_ring.push_back(rssi_med);
                        baseline = rssi_med;
                        snap_hold = 0;
                    }
                } else {
                    snap_hold = 0;
                }

                // 区状态翻转才发事件（区内活动/静坐不重复触发）
                let motion_ticks = motion_bits.count_ones();
                let enter = near_hold as u64 * ac::CSI_EVAL_INTERVAL_MS
                    >= ac::CSI_NEAR_ENTER_HOLD_MS
                    && motion_ticks >= ac::CSI_NEAR_ENTER_MOTION_TICKS;
                let exit = far_hold as u64 * ac::CSI_EVAL_INTERVAL_MS >= ac::CSI_NEAR_EXIT_HOLD_MS
                    && motion_ticks >= ac::CSI_NEAR_EXIT_MOTION_TICKS;
                if !in_zone && enter {
                    in_zone = true;
                    near_hold = 0;
                    far_hold = 0;
                    last_event = "approach";
                    ::log::info!(
                        "CSI event: approach (drop={:.1}dB, base={:.1}, rssi_med={:.1}, p={:.2}, ac={:.2}, iqr={:.2}, turb={:.2}, frames={})",
                        drop,
                        baseline,
                        rssi_med,
                        p,
                        ac,
                        iqr,
                        turb,
                        frames_total
                    );
                    publish_event("approach", p, ac, iqr, turb, frames_total);
                } else if in_zone && exit {
                    in_zone = false;
                    near_hold = 0;
                    far_hold = 0;
                    last_event = "leave";
                    ::log::info!(
                        "CSI event: leave (drop={:.1}dB, base={:.1}, rssi_med={:.1}, p={:.2}, ac={:.2}, iqr={:.2}, turb={:.2}, frames={})",
                        drop,
                        baseline,
                        rssi_med,
                        p,
                        ac,
                        iqr,
                        turb,
                        frames_total
                    );
                    publish_event("leave", p, ac, iqr, turb, frames_total);
                }
            }
        }

        // ---- 每 CSI_LOG_INTERVAL_MS 打印当前测量值（观察/调阈值）----
        if ac::CSI_LOG_INTERVAL_MS > 0
            && last_print.elapsed() >= Duration::from_millis(ac::CSI_LOG_INTERVAL_MS)
        {
            last_print = Instant::now();
            ::log::info!(
                "csi: state={} drop={:+.1}dB base={:.1} rssi_med={:.1} p={:.2} ac={:.2} iqr={:.2} turb={:.2} n={} len={} frames={} rx={} drop(rs={} slot={} parse={})",
                if in_zone { "in" } else { "out" },
                if base_ready { baseline - rssi_med } else { 0.0 },
                if base_ready { baseline } else { 0.0 },
                rssi_med,
                p,
                ac,
                iqr,
                turb,
                n,
                len_last,
                frames_total,
                DIAG_RX.load(Ordering::Relaxed),
                DIAG_RS.load(Ordering::Relaxed),
                DIAG_SLOT.load(Ordering::Relaxed),
                DIAG_PARSE.load(Ordering::Relaxed)
            );
        }

        update_snapshot(Snapshot {
            last: last_event,
            state: if in_zone { "in" } else { "out" },
            p,
            ac,
            iqr,
            turb,
            rssi: rssi_med.round() as i8,
            base_ready,
            base: baseline,
            drop: if base_ready { baseline - rssi_med } else { 0.0 },
            frames: frames_total,
        });
    }
}

/// 当前感知状态（DeviceStatus.presence）：最近事件 approach → "present"，
/// 其余（含开机未触发过）→ "absent"。事件才是权威输出，此字段仅兼容既有视图
pub fn presence() -> &'static str {
    match SNAPSHOT.lock().map(|s| s.last) {
        Ok("approach") => "present",
        _ => "absent",
    }
}

/// 串口 `csi` 命令输出
pub fn snapshot_text() -> String {
    let s = *SNAPSHOT.lock().unwrap();
    format!(
        "csi zone={} last={} drop={:+.1}dB base={} rssi_med={} dBm p={:.2} ac={:.2} iqr={:.2} turb={:.2} frames={}\r\n\
         判定（近场 ~1m 范围门）: 电平降幅≥{:.1}dB 持续 {}ms + 运动佐证≥{}拍 → approach（靠近）; \
         降幅回到≤{:.1}dB 持续 {}ms + 佐证≥{}拍 → leave（离开）; 一拍={}ms 窗口={}ms; \
         运动内核 = ESPectre 湍流特征（自相关+IQR→p，阈值{:.2}），p 仅作佐证; \
         基线 = 非近场 60s 窗中值慢自适应（无人工校准）",
        s.state,
        s.last,
        s.drop,
        if s.base_ready {
            format!("{:.1}", s.base)
        } else {
            "--(采样中)".to_string()
        },
        s.rssi,
        s.p,
        s.ac,
        s.iqr,
        s.turb,
        s.frames,
        ac::CSI_NEAR_ENTER_DB,
        ac::CSI_NEAR_ENTER_HOLD_MS,
        ac::CSI_NEAR_ENTER_MOTION_TICKS,
        ac::CSI_NEAR_EXIT_DB,
        ac::CSI_NEAR_EXIT_HOLD_MS,
        ac::CSI_NEAR_EXIT_MOTION_TICKS,
        ac::CSI_EVAL_INTERVAL_MS,
        ac::CSI_WINDOW_MS,
        ac::CSI_MOTION_THRESHOLD,
    )
}

/// 拿不到默认网关时的兜底 ping 目标（阿里公共 DNS，国内常可达）
const PING_FALLBACK: [u8; 4] = [223, 5, 5, 5];

/// 启动持续 ICMP ping（无限会话，内部 ping 任务跑，本模块不持有句柄）：
/// **主帧源**——echo reply 是 AP 发的下行 HT 帧，稳定出 CSI。10ms 间隔 =
/// 100 包/s；`esp_ping` 内部用 `vTaskDelayUntil` 节拍 + 自带接收匹配，
/// 配合 sdkconfig `CONFIG_FREERTOS_HZ=1000` 保证 10ms 粒度（ESPectre 默认
/// ping 流量模式同款思路）。TOS 打 DSCP 46（网关可能忽略，非致命）。
fn start_ping(target_ip: [u8; 4]) {
    // ip_addr_t = { u_addr: ip4|ip6, type_ }，IPv4 手工构造（IP_ADDR4_INIT 宏不可见）
    let mut target: ip_addr_t = unsafe { std::mem::zeroed() };
    target.type_ = lwip_ip_addr_type_IPADDR_TYPE_V4 as u8;
    // union 字段写入是安全操作（读才是 unsafe）
    target.u_addr.ip4.addr = u32::from_le_bytes(target_ip);

    let cfg = esp_ping_config_t {
        count: 0, // 无限 ping
        interval_ms: ac::CSI_PING_INTERVAL_MS,
        timeout_ms: ac::CSI_PING_TIMEOUT_MS,
        data_size: 32,
        tos: ac::CSI_PING_TOS,
        ttl: 64,
        target_addr: target,
        task_stack_size: 4096,
        task_prio: 2,
        interface: 0, // NETIF_NO_INDEX
    };
    let cbs = esp_ping_callbacks_t {
        cb_args: core::ptr::null_mut(),
        on_ping_success: None,
        on_ping_timeout: None,
        on_ping_end: None,
    };

    unsafe {
        let mut hdl: esp_ping_handle_t = core::ptr::null_mut();
        if esp_ping_new_session(&cfg, &cbs, &mut hdl) != ESP_OK {
            ::log::warn!("CSI ping session create failed (CSI frame rate may stay low)");
            return;
        }
        if esp_ping_start(hdl) != ESP_OK {
            ::log::warn!("CSI ping session start failed (CSI frame rate may stay low)");
        }
    }
    ::log::info!(
        "CSI ping probe started: target={}.{}.{}.{} interval={}ms (generates CSI frames)",
        target_ip[0],
        target_ip[1],
        target_ip[2],
        target_ip[3],
        ac::CSI_PING_INTERVAL_MS
    );
}

/// 启动近场感知：关闭 Wi-Fi 省电 → 注册 CSI 配置/回调并使能 → 启动探测流量
/// → 拉起检测线程。需在 Wi-Fi 启动后调用；CSI 数据只在收到帧时产生。
/// 上行速率钉 6Mbps 在 `wifi_prov`（`esp_wifi_start` 之前的 API 约束）。
pub fn start() -> Result<(), esp_err_t> {
    // 省电模式（modem sleep）下射频大部分时间休眠，CSI 帧率只有 ~0.2 帧/秒；
    // 本设备常供电，直接关闭省电换帧率。
    // 注意：stop() 不存在，省电保持关闭（恢复可调 esp_wifi_set_ps(WIFI_PS_MIN_MODEM)）。
    let err = unsafe { esp_wifi_set_ps(wifi_ps_type_t_WIFI_PS_NONE) };
    if err != ESP_OK {
        ::log::warn!("esp_wifi_set_ps(WIFI_PS_NONE) failed: {err:#x} (CSI frame rate may be low)");
    }

    // CSI 采集配置照抄 ESPectre（HT20 档）：**只采 HT-LTF 符号**——关 LLTF
    // （非 HT 帧不出 CSI，属预期）、关 STBC-HT-LTF2、关 LTF 合并、关信道滤波；
    // 关 dump_ack_en（实测上行 ACK 从不出 CSI，主帧源为 echo reply 下行帧）。
    // 载荷固定 128B 全宽，与特征流水线标定语料一致
    let cfg = wifi_csi_config_t {
        lltf_en: false,
        htltf_en: true,
        stbc_htltf2_en: false,
        ltf_merge_en: false,
        channel_filter_en: false,
        manu_scale: false, // 自动缩放
        shift: 0,
        dump_ack_en: false,
    };

    let err = unsafe {
        let e = esp_wifi_set_csi_config(&cfg);
        if e != ESP_OK {
            e
        } else {
            let e = esp_wifi_set_csi_rx_cb(Some(csi_rx_cb), core::ptr::null_mut());
            if e != ESP_OK {
                e
            } else {
                esp_wifi_set_csi(true)
            }
        }
    };
    if err != ESP_OK {
        ::log::error!("Wi-Fi CSI init failed: {err:#x}");
        return Err(err);
    }
    ::log::info!("Wi-Fi CSI enabled (approach/leave event probe)");

    // 主动探测流量（帧率来源，见模块注释）：ICMP ping 100pps → echo reply
    // （AP 下行 HT 帧）喂 CSI；目标 = 默认网关（拿不到时退公共 DNS）
    let target_ip = netinfo::sta_gateway().unwrap_or(PING_FALLBACK);
    start_ping(target_ip);

    std::thread::Builder::new()
        .name("csi_det".into())
        // 栈含日志格式化（localtime_r）+ serde_json + FFI 余量；4096 实测栈溢出
        //（task pthread 栈金丝雀报警，2026-10 真机），8192 与 mqtt 事件线程同级
        .stack_size(8192)
        .spawn(detector_task)
        .expect("spawn csi detector task");
    Ok(())
}
