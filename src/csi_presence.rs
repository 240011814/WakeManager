//! WakeManager - Wi-Fi CSI 近场趋势探测（靠近 / 离开两个事件）
//!
//! 产品语义（2026-10 定稿，明确只报两个触发）：
//!   - **approach（靠近）**：① 上升斜率 ≥ +3%（**快路径**：带动作走近，2s 内）；
//!     ② 平滑水位 ≥ 空场地板 ×1.02 持续 3s（**慢路径**：安静坐下/走近——实测
//!     水位 1~2 分钟慢慢爬升，纯斜率判据拖 2 分钟才触发）
//!   - **leave（离开）**：平滑水位**回到空场地板** ×1.015 以内持续 3s，**且近 8s
//!     内有活动**（人离开必有行走动作；坐着时的水位下凹是安静的——活动限定
//!     防下凹误报）
//! 不做「有人/无人」状态判定、不做基线校准（地板 = 滑动窗口最小值，自学习）。
//!
//! **leave 为什么不用下降斜率**（真机跳变教训）：坐着的人静止水位只比空场高
//! ~4%，动作峰却 +10~30%——下降斜率把"坐着时的动作回落"全当成了离开，
//! approach/leave 3~5s 交替跳变。改为认「回到空场地板」后：坐着时水位始终在
//! 地板 +4% 以上 → 完全静默；人真走了水位落到地板 → 报 leave。
//!
//! 触发语义：每个方向**每次趋势只发一条**（交替触发防刷屏）+ 最小事件间隔
//! `CSI_TREND_REFRACTORY_MS` 兜底。响应 ≤5s（2s 平滑窗 + 1s/1.5s 防抖）。
//!
//! 数据流：
//!   Wi-Fi 任务（CSI 回调，只做幅度统计，禁日志/分配）→ 环形缓冲 →
//!   检测线程（500ms 评估窗口幅度 + 趋势斜率）→ **趋势事件**发布 JSON 通知到
//!   MQTT `csi/notify` 主题：`{"id":..,"cmd":"csi","event":"approach|leave",
//!   "slope":..,"amp":..,"rs":..,"frames":..}`
//!
//! **帧率依赖主动探测流量**（关键教训）：CSI 只对 **OFDM/HT 速率**的帧生成，AP 的
//! beacon 走 802.11b/CCK 基本速率**不产生 CSI**——STA 静默时帧率只有 ~0.2 帧/秒。
//! 因此开机即：① 关省电（`esp_wifi_set_ps(WIFI_PS_NONE)`）；② **UDP 单播探测流量**
//! 打默认网关（每帧必被 AP MAC-ACK，`dump_ack_en` 下 ACK 帧出 CSI）；③ ICMP ping
//! 网关（100ms = 10 帧/s 保底）——必须留兜底：UDP 的 ACK 在速率自适应落到 CCK 时
//! 会断流（实测 30s 空窗）。窗口按时长（`CSI_WINDOW_MS`=2s）统计，帧率无关。
//!
//! 幅度度量 = **全部子载波段 RMS**（LLTF+HTLTF+STBC 混合；实测对活动最敏感）。
//! 注意：人体贴近在本机实测是**幅度上升**（+5~10%）；若换环境出现贴近反而下降，
//! 上升/下降的事件语义会互换（改 CSI_TREND_ENTER 的符号解释即可）。
//!
//! FFI：`esp_wifi_set_csi_config / esp_wifi_set_csi_rx_cb / esp_wifi_set_csi`
//! （需 sdkconfig 开 `CONFIG_ESP_WIFI_CSI_ENABLED`，本项目已在 sdkconfig.defaults
//! 开启；函数声明不随该开关门控，绑定始终可见）。

use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use esp_idf_svc::sys::*;

use crate::app_config as ac;
use crate::control_mqtt;
use crate::device_info;
use crate::netinfo;

/// 一帧 CSI 的轻量统计（回调 → 检测线程）
struct FrameStat {
    /// 幅度度量：全部子载波段 RMS（对活动最敏感）
    amp: f32,
    rssi: i8,
    /// 收帧时刻（时长窗口统计用，回调里盖戳）
    at: Instant,
}

/// 回调写入的环形缓冲（Wi-Fi 任务上下文，锁内只做 push/淘汰，不阻塞）
static RING: Mutex<VecDeque<FrameStat>> = Mutex::new(VecDeque::new());

/// 趋势方向
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trend {
    /// 上升 = 靠近
    Approach,
    /// 下降 = 离开
    Leave,
}

impl Trend {
    fn as_str(self) -> &'static str {
        match self {
            Trend::Approach => "approach",
            Trend::Leave => "leave",
        }
    }
}

/// 检测线程每拍刷新的状态快照（串口 `csi` 命令读取）
#[derive(Clone, Copy)]
pub struct Snapshot {
    /// 最近一次趋势事件："none" / "approach" / "leave"
    pub last: &'static str,
    /// 当前窗口幅度均值
    pub amp: f32,
    /// 平滑水位（动作毛刺被压掉；离开判定用）
    pub level: f32,
    /// 空场地板：近 10 分钟平滑水位最小值
    pub floor: f32,
    /// 趋势斜率：(当前 − 3s 前) / 3s 前
    pub slope: f32,
    /// 当前窗口帧间波动（总体标准差）
    pub std: f32,
    pub rssi: i8,
    /// 自本次启动收到的 CSI 帧数（诊断用，饱和计数不回绕）
    pub frames: u64,
}

static SNAPSHOT: Mutex<Snapshot> = Mutex::new(Snapshot {
    last: "none",
    amp: 0.0,
    level: 0.0,
    floor: 0.0,
    slope: 0.0,
    std: 0.0,
    rssi: 0,
    frames: 0,
});

/// 发布到 `csi/notify` 的趋势事件消息
#[derive(serde::Serialize)]
struct NotifyMsg<'a> {
    id: &'a str,
    cmd: &'a str,
    /// "approach" = 靠近（上升趋势）；"leave" = 离开（下降趋势）
    event: &'a str,
    /// 触发时刻的相对斜率（3s 变化率）
    slope: f32,
    amp: f32,
    /// 帧间波动相对值（std/amp，诊断用）
    rs: f32,
    frames: u64,
}

/// 趋势事件 → JSON 通知经 MQTT 发布路径入队
fn publish_event(event: Trend, slope: f32, amp: f32, rs: f32, frames: u64) {
    let id = device_info::get_id();
    let msg = NotifyMsg {
        id: &id,
        cmd: "csi",
        event: event.as_str(),
        slope,
        amp,
        rs,
        frames,
    };
    match serde_json::to_string(&msg) {
        Ok(json) => control_mqtt::publish_notify(&json),
        Err(e) => ::log::error!("CSI notify serialize failed: {e:?}"),
    }
}

/// CSI 接收回调（Wi-Fi 任务上下文）：只算幅度 RMS 与入队，
/// **不得**日志/分配/阻塞；缓冲由检测线程消费。
unsafe extern "C" fn csi_rx_cb(_ctx: *mut core::ffi::c_void, data: *mut wifi_csi_info_t) {
    if data.is_null() {
        return;
    }
    let info = &*data;
    if info.buf.is_null() || info.len == 0 {
        return;
    }
    // 前 4 字节（两对 I/Q）受硬件限制可能无效
    let skip = if info.first_word_invalid { 4usize } else { 0 };
    let total = info.len as usize;
    if total <= skip {
        return;
    }
    let raw = core::slice::from_raw_parts(info.buf.add(skip) as *const u8, total - skip);

    // I/Q 按 int8 交错：累计平均功率，帧尾一次开方得幅度度量
    let mut power = 0f32;
    let mut n = 0usize;
    for pair in raw.chunks_exact(2) {
        let i = pair[0] as i8 as f32;
        let q = pair[1] as i8 as f32;
        power += i * i + q * q;
        n += 1;
    }
    if n == 0 {
        return;
    }
    let stat = FrameStat {
        amp: (power / n as f32).sqrt(),
        rssi: info.rx_ctrl.rssi() as i8,
        at: Instant::now(),
    };

    if let Ok(mut ring) = RING.lock() {
        if ring.len() >= ac::CSI_HIST_CAP {
            ring.pop_front();
        }
        ring.push_back(stat);
    }
}

/// 最近 `window_ms` 毫秒内帧幅度的 (均值, 总体标准差, 样本数)。
/// **时长窗口**（非帧数窗口）：探测流量帧率变化时窗口覆盖的真实时间不变。
fn window_stats(hist: &VecDeque<FrameStat>, window_ms: u64, now: Instant) -> (f32, f32, usize) {
    let cutoff = now - Duration::from_millis(window_ms);
    let mut sum = 0f32;
    let mut n = 0usize;
    for f in hist.iter().rev() {
        if f.at < cutoff {
            break;
        }
        sum += f.amp;
        n += 1;
    }
    if n == 0 {
        return (0.0, 0.0, 0);
    }
    let mean = sum / n as f32;
    let mut var = 0f32;
    for f in hist.iter().rev() {
        if f.at < cutoff {
            break;
        }
        let d = f.amp - mean;
        var += d * d;
    }
    (mean, (var / n as f32).sqrt(), n)
}

fn update_snapshot(snap: Snapshot) -> Snapshot {
    if let Ok(mut g) = SNAPSHOT.lock() {
        *g = snap;
    }
    snap
}

/// 检测线程：窗口幅度 → 趋势斜率 → approach/leave 事件发 MQTT 通知
fn detector_task() {
    ::log::info!("CSI trend detector started");
    let tick = Duration::from_millis(ac::CSI_EVAL_INTERVAL_MS);

    let mut hist: VecDeque<FrameStat> = VecDeque::new(); // 帧历史（时长窗口取最近段）
    let mut amp_hist: VecDeque<(Instant, f32)> = VecDeque::new(); // 平滑幅度历史（取 3s 前样本算斜率）
    let mut floor_hist: VecDeque<(Instant, f32)> = VecDeque::new(); // 水位采样（空场地板 = 窗口最小值）
    let mut level: f32 = 0.0; // 平滑水位（离开判定用）
    let mut last_floor_at = Instant::now();
    let mut rssi_last: i8 = 0;
    let mut frames_total: u64 = 0;
    let mut last_event: Option<Trend> = None;
    let mut last_event_at = Instant::now()
        .checked_sub(Duration::from_millis(ac::CSI_TREND_REFRACTORY_MS))
        .unwrap_or_else(Instant::now);
    let mut cand_up: Option<Instant> = None; // 上升斜率（快路径）的连续达标起点
    let mut cand_lvl: Option<Instant> = None; // 水位超地板（慢路径）的连续达标起点
    let mut cand_dn: Option<Instant> = None; // 回落空场地板的连续达标起点
    let mut last_motion_at = Instant::now(); // 最后一次活动时刻（离开判定的活动限定）
    let mut last_frame_at = Instant::now();
    let mut stalled_warned = false;
    let mut last_print = Instant::now();
    let started_at = Instant::now();

    loop {
        std::thread::sleep(tick);

        // ---- 取新帧 ----
        let mut fresh: usize = 0;
        {
            let mut ring = RING.lock().unwrap();
            while let Some(f) = ring.pop_front() {
                if hist.len() >= ac::CSI_HIST_CAP {
                    hist.pop_front();
                }
                rssi_last = f.rssi;
                hist.push_back(f);
                // 饱和计数：u64 在 20 帧/s 下约 290 亿年才到顶，saturating 只是
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

        // ---- 窗口统计（样本不足 = 帧率异常低，保持现状）----
        let now = Instant::now();
        let (amp, std, n) = window_stats(&hist, ac::CSI_WINDOW_MS, now);
        if n < ac::CSI_WINDOW_MIN_FRAMES {
            continue;
        }
        let rs = if amp > 0.0 { std / amp } else { 0.0 };

        // ---- 平滑水位（压掉动作毛刺）与空场地板（滑动窗口最小值）----
        level = if level <= 0.0 {
            amp
        } else {
            level + ac::CSI_LEVEL_ALPHA * (amp - level)
        };
        if last_floor_at.elapsed() >= Duration::from_millis(5_000) {
            last_floor_at = now;
            floor_hist.push_back((now, level));
            let cut = now - Duration::from_millis(ac::CSI_FLOOR_WINDOW_MS);
            while matches!(floor_hist.front(), Some((at, _)) if *at < cut) {
                floor_hist.pop_front();
            }
        }
        let floor = floor_hist
            .iter()
            .map(|(_, v)| *v)
            .fold(level, f32::min);

        // ---- 趋势斜率：当前平滑幅度 vs CSI_TREND_WINDOW_MS 前 ----
        amp_hist.push_back((now, amp));
        while amp_hist.len() > 24 {
            amp_hist.pop_front(); // 保留 ~12s，远超比较间隔
        }
        let target_at = now - Duration::from_millis(ac::CSI_TREND_WINDOW_MS);
        let old = amp_hist
            .iter()
            .rev()
            .find(|(at, _)| *at <= target_at)
            .map(|(_, v)| *v);
        let slope = match old {
            Some(s_old) if s_old > 0.0 => (amp - s_old) / s_old,
            _ => 0.0, // 历史不足（开机前几秒），热身期也会跳过事件
        };

        // ---- 每 CSI_LOG_INTERVAL_MS 打印当前测量值（观察/调阈值）----
        if ac::CSI_LOG_INTERVAL_MS > 0
            && last_print.elapsed() >= Duration::from_millis(ac::CSI_LOG_INTERVAL_MS)
        {
            last_print = Instant::now();
            let s = *SNAPSHOT.lock().unwrap();
            ::log::info!(
                "csi: last={} amp={:.1} level={:.1} floor={:.1} slope={:+.3} rs={:.3} rssi={} dBm frames={}",
                s.last,
                s.amp,
                s.level,
                s.floor,
                s.slope,
                rs,
                s.rssi,
                s.frames
            );
        }

        // ---- 事件判定（每个方向每次趋势只发一条 + 最小事件间隔兜底）----
        // approach：① 上升斜率快路径（带动作走近，2s）② 水位超地板慢路径
        //（安静坐下——实测水位 1~2 分钟慢慢爬升，纯斜率判据拖 2 分钟，真机教训）
        // leave：水位回到空场地板 **且近 8s 内有活动**（人离开必有行走动作；
        // 坐着时的水位下凹是安静的——活动限定防下凹误报，真机跳变教训）
        if rs >= ac::CSI_MOTION_RS || slope.abs() >= 0.02 {
            last_motion_at = now;
        }
        let refractory_ok = last_event_at.elapsed()
            >= Duration::from_millis(ac::CSI_TREND_REFRACTORY_MS);
        if started_at.elapsed() >= Duration::from_millis(ac::CSI_TREND_WARMUP_MS) {
            let fire_approach = |cand: &mut Option<Instant>| -> bool {
                let since = cand.unwrap_or(now);
                *cand = Some(since);
                if since.elapsed() >= Duration::from_millis(ac::CSI_TREND_SUSTAIN_MS)
                    && last_event != Some(Trend::Approach)
                    && refractory_ok
                {
                    *cand = None;
                    true
                } else {
                    false
                }
            };

            // 上升斜率快路径 → approach
            if slope >= ac::CSI_TREND_ENTER {
                if fire_approach(&mut cand_up) {
                    last_event = Some(Trend::Approach);
                    last_event_at = now;
                    cand_up = None;
                    cand_lvl = None;
                    cand_dn = None;
                    ::log::info!(
                        "CSI trend: {} via slope (slope={:+.3}, level={:.1}, floor={:.1}, rs={:.3})",
                        Trend::Approach.as_str(),
                        slope,
                        level,
                        floor,
                        rs
                    );
                    publish_event(Trend::Approach, slope, level, rs, frames_total);
                }
            } else {
                cand_up = None;
            }

            // 水位超地板慢路径 → approach（安静坐下/走近）
            if level >= floor * (1.0 + ac::CSI_APPROACH_MARGIN) {
                let since = cand_lvl.unwrap_or(now);
                cand_lvl = Some(since);
                if since.elapsed() >= Duration::from_millis(ac::CSI_APPROACH_SUSTAIN_MS)
                    && last_event != Some(Trend::Approach)
                    && refractory_ok
                {
                    last_event = Some(Trend::Approach);
                    last_event_at = now;
                    cand_up = None;
                    cand_lvl = None;
                    cand_dn = None;
                    ::log::info!(
                        "CSI trend: {} via level (level={:.1} >= floor {:.1}x{:.2}, slope={:+.3}, rs={:.3})",
                        Trend::Approach.as_str(),
                        level,
                        floor,
                        1.0 + ac::CSI_APPROACH_MARGIN,
                        slope,
                        rs
                    );
                    publish_event(Trend::Approach, slope, level, rs, frames_total);
                }
            } else {
                cand_lvl = None;
            }

            // 回落到空场地板 + 近期有活动 → leave
            let motion_ok = last_event.is_none()
                || last_motion_at.elapsed() < Duration::from_millis(ac::CSI_LEAVE_MOTION_MS);
            if level <= floor * (1.0 + ac::CSI_LEAVE_MARGIN) && motion_ok {
                let since = cand_dn.unwrap_or(now);
                cand_dn = Some(since);
                if since.elapsed() >= Duration::from_millis(ac::CSI_LEAVE_SUSTAIN_MS)
                    && last_event != Some(Trend::Leave)
                    && refractory_ok
                {
                    last_event = Some(Trend::Leave);
                    last_event_at = now;
                    cand_up = None;
                    cand_lvl = None;
                    cand_dn = None;
                    ::log::info!(
                        "CSI trend: {} (level={:.1} <= floor {:.1}x{:.2}, slope={:+.3}, rs={:.3})",
                        Trend::Leave.as_str(),
                        level,
                        floor,
                        1.0 + ac::CSI_LEAVE_MARGIN,
                        slope,
                        rs
                    );
                    publish_event(Trend::Leave, slope, level, rs, frames_total);
                }
            } else {
                cand_dn = None;
            }
        }

        update_snapshot(Snapshot {
            last: last_event.map(|t| t.as_str()).unwrap_or("none"),
            amp,
            level,
            floor,
            slope,
            std,
            rssi: rssi_last,
            frames: frames_total,
        });
    }
}

/// 当前感知状态（DeviceStatus.presence）：最近事件 approach → "present"，
/// 其余（含开机未触发过）→ "absent"。趋势事件才是权威输出，此字段仅兼容既有视图
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
        "csi last={} amp={:.1} level={:.1} floor={:.1} slope={:+.3} std={:.2} rssi={} dBm frames={}\r\n\
         判定: slope>={:.2} 或 level>=floor*{:.2} 持续 {}ms → approach（靠近）；\
         level<=floor*{:.2} 持续 {}ms 且近 {}ms 有活动 → leave（离开）；每方向每次趋势只发一条",
        s.last,
        s.amp,
        s.level,
        s.floor,
        s.slope,
        s.std,
        s.rssi,
        s.frames,
        ac::CSI_TREND_ENTER,
        1.0 + ac::CSI_APPROACH_MARGIN,
        ac::CSI_APPROACH_SUSTAIN_MS,
        1.0 + ac::CSI_LEAVE_MARGIN,
        ac::CSI_LEAVE_SUSTAIN_MS,
        ac::CSI_LEAVE_MOTION_MS,
    )
}

/// 拿不到默认网关时的兜底 ping 目标（阿里公共 DNS，国内常可达）
const PING_FALLBACK: [u8; 4] = [223, 5, 5, 5];

/// UDP 探测流量线程：向默认网关单播小包。**每帧都被 AP MAC 层 ACK**，
/// `dump_ack_en` 下 ACK 帧出 CSI——主帧源：速率稳定，不依赖对端回应
/// 或 ICMP 限速策略。端口 9（discard）：网关静默丢弃或回 ICMP 不可达均可，
/// 两种情况 MAC ACK 都存在。
fn probe_task(target_ip: [u8; 4], interval_ms: u64) {
    let addr = SocketAddr::from((target_ip, 9));
    let sock = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(e) => {
            ::log::warn!("CSI probe socket bind failed: {e:?} (ACK-CSI source off)");
            return;
        }
    };
    let payload = [0u8; 32];
    loop {
        let _ = sock.send_to(&payload, addr);
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}

/// 启动持续 ICMP ping（无限会话，内部 ping 任务跑，本模块不持有句柄）：
/// echo reply 走 HT 速率**必出** CSI，是稳定帧源的兜底（UDP 的 ACK 在速率
/// 自适应落到 CCK 时会断流）。100ms = 10 帧/s 保底。
fn start_ping(target_ip: [u8; 4]) {
    // ip_addr_t = { u_addr: ip4|ip6, type_ }，IPv4 手工构造（IP_ADDR4_INIT 宏不可见）
    let mut target: ip_addr_t = unsafe { std::mem::zeroed() };
    target.type_ = lwip_ip_addr_type_IPADDR_TYPE_V4 as u8;
    // union 字段写入是安全操作（读才是 unsafe）
    target.u_addr.ip4.addr = u32::from_le_bytes(target_ip);

    let cfg = esp_ping_config_t {
        count: 0, // 无限 ping
        interval_ms: ac::CSI_PING_INTERVAL_MS,
        timeout_ms: 1_000,
        data_size: 32,
        tos: 0,
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

/// 启动趋势探测：关闭 Wi-Fi 省电 → 注册 CSI 配置/回调并使能 → 启动探测流量
/// → 拉起检测线程。需在 Wi-Fi 启动后调用；CSI 数据只在收到帧时产生。
pub fn start() -> Result<(), esp_err_t> {
    // 省电模式（modem sleep）下射频大部分时间休眠，CSI 帧率只有 ~0.2 帧/秒；
    // 本设备常供电，直接关闭省电换帧率。
    // 注意：stop() 不存在，省电保持关闭（恢复可调 esp_wifi_set_ps(WIFI_PS_MIN_MODEM)）。
    let err = unsafe { esp_wifi_set_ps(wifi_ps_type_t_WIFI_PS_NONE) };
    if err != ESP_OK {
        ::log::warn!("esp_wifi_set_ps(WIFI_PS_NONE) failed: {err:#x} (CSI frame rate may be low)");
    }

    // wifi_csi_config_t 无 Default 可用值（Default 全 false，会关掉所有 LTF），
    // 按 esp_wifi.h 文档默认值手工构造
    let cfg = wifi_csi_config_t {
        lltf_en: true,
        htltf_en: true,
        stbc_htltf2_en: true,
        ltf_merge_en: true,
        channel_filter_en: true,
        manu_scale: false, // 自动缩放
        shift: 0,
        dump_ack_en: true, // AP 对上行帧的 ACK 也出 CSI（主帧源，见 probe_task）
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
    ::log::info!("Wi-Fi CSI enabled (approach/leave trend probe)");

    // 主动探测流量（帧率来源，见模块注释）：
    // ① UDP 单播 → AP MAC ACK（主帧源，20 包/s）；② ICMP echo reply（10 帧/s 保底）
    let target_ip = netinfo::sta_gateway().unwrap_or(PING_FALLBACK);
    start_ping(target_ip);
    std::thread::Builder::new()
        .name("csi_probe".into())
        .stack_size(4096)
        .spawn(move || probe_task(target_ip, ac::CSI_PROBE_INTERVAL_MS))
        .expect("spawn csi probe task");

    std::thread::Builder::new()
        .name("csi_det".into())
        // 栈含日志格式化（localtime_r）+ serde_json + FFI 余量；4096 实测栈溢出
        //（task pthread 栈金丝雀报警，2026-10 真机），8192 与 mqtt 事件线程同级
        .stack_size(8192)
        .spawn(detector_task)
        .expect("spawn csi detector task");
    Ok(())
}
