//! WakeManager - Wi-Fi CSI 近场人体感知（探测 ~1 米内是否有人）
//!
//! 原理（**近场扰动近似**，非精确测距）：STA 模式下 Wi-Fi 每收到一帧（主要是 AP 的
//! beacon 与下行帧）就给出 CSI（信道状态信息）子载波 I/Q 数据。人体贴近设备（约 1 米内）
//! 活动会显著改变信道：帧间波动（std）大幅升高（实测静场 std/amp ≈ 2.3%，1 米内活动
//! ≈ 13.5%），幅度均值也可能衰减/起伏。判定 = 「窗口帧间波动 ≥ 阈值」**或**
//! 「幅度相对无人基线衰减 ≥ 阈值」→ 有人。CSI 无法真正测距，「1 米」是近场扰动强度的
//! 近似（远处走动也可能触发），阈值按现场微调（见 app_config）。
//!
//! 数据流：
//!   Wi-Fi 任务（CSI 回调，只做轻量统计，禁日志/分配）→ 环形缓冲 →
//!   检测线程（500ms 评估窗口均值/波动）→ 状态机（迟滞 + 持续防抖）→
//!   **状态变化时**发布 JSON 通知到 MQTT `csi/notify` 主题（有人↔无人各发一条）
//!
//! **帧率依赖主动探测流量**（关键教训）：CSI 只对 **OFDM/HT 速率**的帧生成，AP 的
//! beacon 走 802.11b/CCK 基本速率**不产生 CSI**——STA 静默时帧率只有 ~0.2 帧/秒
//! （实测 40 帧收 3 分钟，关 Wi-Fi 省电也救不回来），窗口横跨几分钟、人体扰动被
//! 完全抹平。因此开机即：① 关省电（`esp_wifi_set_ps(WIFI_PS_NONE)`）；② **UDP
//! 单播探测流量**打默认网关（每帧必被 AP MAC-ACK，`dump_ack_en` 下 ACK 帧出 CSI，
//! `CSI_PROBE_INTERVAL_MS`=50ms → 20 包/s，稳定不依赖对端回应）；③ ICMP ping 网关
//! 补充数据帧 CSI（echo reply 的 LTF 齐全）。窗口按时长（`CSI_WINDOW_MS`=2s）不按
//! 帧数统计，帧率变化不影响检测时序。
//!
//! 基线自适应：只在「无人」状态自适应（τ≈25s），吸收环境慢漂移；「有人」状态
//! 冻结基线（否则久坐的人会被吸进基线导致漏检）。开机校准假设现场无人——若
//! 校准时有人，基线会被污染，人走后 ~30s 自动修正，首个「离开」事件可能漏报；
//! 环境永久变化发生在有人期间可能造成状态粘滞，串口 `csi recal` 强制重新校准。
//!
//! FFI：`esp_wifi_set_csi_config / esp_wifi_set_csi_rx_cb / esp_wifi_set_csi`
//! （需 sdkconfig 开 `CONFIG_ESP_WIFI_CSI_ENABLED`，本项目已在 sdkconfig.defaults
//! 开启；函数声明不随该开关门控，绑定始终可见）。

use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use esp_idf_svc::sys::*;

use crate::app_config as ac;
use crate::control_mqtt;
use crate::device_info;
use crate::netinfo;

/// 一帧 CSI 的轻量统计（回调 → 检测线程）
struct FrameStat {
    /// 幅度度量：各子载波平均功率开方（同一度量下做相对比较，无需绝对标定）
    amp: f32,
    rssi: i8,
    /// 收帧时刻（时长窗口统计用，回调里盖戳）
    at: Instant,
}

/// 回调写入的环形缓冲（Wi-Fi 任务上下文，锁内只做 push/淘汰，不阻塞）
static RING: Mutex<VecDeque<FrameStat>> = Mutex::new(VecDeque::new());

/// 检测状态
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// 校准中（收集无人基线）
    Calibrating,
    /// 1 米内无人
    Absent,
    /// 1 米内有人
    Present,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Calibrating => "calibrating",
            State::Absent => "absent",
            State::Present => "present",
        }
    }
}

/// 检测线程每拍刷新的状态快照（串口 `csi` 命令读取）
#[derive(Clone, Copy)]
pub struct Snapshot {
    pub state: &'static str,
    /// 无人基线幅度
    pub baseline: f32,
    /// 当前窗口幅度均值
    pub amp: f32,
    /// 当前窗口帧间波动（总体标准差）
    pub std: f32,
    /// 相对衰减 = (baseline - amp) / baseline，正 = 比基线低 = 有人贴近
    pub drop: f32,
    pub rssi: i8,
    /// 累计收到的 CSI 帧数（诊断用）
    pub frames: u64,
}

static SNAPSHOT: Mutex<Snapshot> = Mutex::new(Snapshot {
    state: "calibrating",
    baseline: 0.0,
    amp: 0.0,
    std: 0.0,
    drop: 0.0,
    rssi: 0,
    frames: 0,
});

/// 串口 `csi recal`：强制重新校准基线（环境永久变化导致状态粘滞时的恢复手段）
static RECAL: AtomicBool = AtomicBool::new(false);

/// 发布到 `csi/notify` 的通知消息
#[derive(serde::Serialize)]
struct NotifyMsg<'a> {
    id: &'a str,
    cmd: &'a str,
    /// "init" = 开机校准完成的初始状态；"change" = 状态变化
    event: &'a str,
    /// "absent" = 1 米内无人；"present" = 1 米内有人
    state: &'a str,
    /// 相对衰减（正 = 幅度低于基线 = 有人贴近）
    drop: f32,
    /// 窗口内帧间波动（总体标准差）
    std: f32,
    baseline: f32,
    amp: f32,
    frames: u64,
}

/// 状态变化（含开机初始状态）→ JSON 通知经 MQTT 发布路径入队
fn publish_event(event: &str, state: State, snap: &Snapshot) {
    let id = device_info::get_id();
    let msg = NotifyMsg {
        id: &id,
        cmd: "csi",
        event,
        state: state.as_str(),
        drop: snap.drop,
        std: snap.std,
        baseline: snap.baseline,
        amp: snap.amp,
        frames: snap.frames,
    };
    match serde_json::to_string(&msg) {
        Ok(json) => control_mqtt::publish_notify(&json),
        Err(e) => ::log::error!("CSI notify serialize failed: {e:?}"),
    }
}

/// CSI 接收回调（Wi-Fi 任务上下文）：只做逐子载波功率累加与入队，
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

/// 全部历史帧幅度的均值（基线校准用）
fn mean_all(hist: &VecDeque<FrameStat>) -> f32 {
    if hist.is_empty() {
        return 0.0;
    }
    let mut sum = 0f32;
    for f in hist {
        sum += f.amp;
    }
    sum / hist.len() as f32
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
    let mut seen = 0usize;
    for f in hist.iter().rev() {
        if f.at < cutoff {
            break;
        }
        let d = f.amp - mean;
        var += d * d;
        seen += 1;
    }
    (mean, (var / seen as f32).sqrt(), n)
}

fn update_snapshot(
    state: State,
    baseline: f32,
    amp: f32,
    std: f32,
    drop: f32,
    rssi: i8,
    frames: u64,
) -> Snapshot {
    let snap = Snapshot {
        state: state.as_str(),
        baseline,
        amp,
        std,
        drop,
        rssi,
        frames,
    };
    if let Ok(mut g) = SNAPSHOT.lock() {
        *g = snap;
    }
    snap
}

/// 检测线程：定时消费环形缓冲 → 窗口统计 → 状态机 → 状态变化发 MQTT 通知
fn detector_task() {
    ::log::info!("CSI presence detector started");
    let tick = Duration::from_millis(ac::CSI_EVAL_INTERVAL_MS);

    let mut hist: VecDeque<FrameStat> = VecDeque::new(); // 帧历史（时长窗口取最近段）
    let mut rssi_last: i8 = 0;
    let mut baseline: f32 = 0.0;
    let mut state = State::Calibrating;
    let mut frames_total: u64 = 0;
    let mut cand: Option<(State, Instant)> = None; // 翻转候选 + 连续达标起点
    let mut last_frame_at = Instant::now();
    let mut stalled_warned = false;
    let mut calib_logged = false;
    let mut last_print = Instant::now();

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
                frames_total += 1;
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

        // ---- 每 CSI_LOG_INTERVAL_MS 打印当前基线与测量值（观察检测量/调阈值）----
        if ac::CSI_LOG_INTERVAL_MS > 0
            && last_print.elapsed() >= Duration::from_millis(ac::CSI_LOG_INTERVAL_MS)
        {
            last_print = Instant::now();
            let s = SNAPSHOT.lock().unwrap();
            if s.state == State::Calibrating.as_str() {
                ::log::info!(
                    "csi: 校准中 {}/{} frames amp={:.1} std={:.2} rssi={} dBm",
                    s.frames,
                    ac::CSI_CALIB_FRAMES,
                    s.amp,
                    s.std,
                    s.rssi
                );
            } else {
                ::log::info!(
                    "csi: state={} baseline={:.1} amp={:.1} std={:.2} rs={:.3} drop={:.3} rssi={} dBm frames={}",
                    s.state,
                    s.baseline,
                    s.amp,
                    s.std,
                    if s.amp > 0.0 { s.std / s.amp } else { 0.0 },
                    s.drop,
                    s.rssi,
                    s.frames
                );
            }
        }

        // ---- 重新校准（串口 csi recal）----
        if RECAL.swap(false, Ordering::SeqCst) {
            ::log::info!("CSI recalibration requested, collecting new baseline");
            state = State::Calibrating;
            cand = None;
            hist.clear();
            frames_total = 0; // 帧计数改为「自本次校准起」，校准进度显示才准确
            calib_logged = false;
        }

        // ---- 校准：收够帧数取均值为无人基线，按「现场无人」初始化 ----
        if state == State::Calibrating {
            if !calib_logged {
                ::log::info!(
                    "CSI calibrating baseline (need {} frames, assuming nobody near)",
                    ac::CSI_CALIB_FRAMES
                );
                calib_logged = true;
            }
            if hist.len() >= ac::CSI_CALIB_FRAMES {
                let amp = mean_all(&hist);
                baseline = amp;
                state = State::Absent;
                cand = None;
                calib_logged = false;
                ::log::info!(
                    "CSI baseline calibrated: amp={baseline:.1} ({} frames, rssi={} dBm)",
                    hist.len(),
                    rssi_last
                );
                let (m, sd, _) = window_stats(&hist, ac::CSI_WINDOW_MS, Instant::now());
                let drop = if baseline > 0.0 {
                    (baseline - m) / baseline
                } else {
                    0.0
                };
                let snap = update_snapshot(State::Absent, baseline, m, sd, drop, rssi_last, frames_total);
                publish_event("init", State::Absent, &snap);
            } else {
                // 校准中也刷新测量值快照，定期打印可观察当前读数
                let (m, sd, _) = window_stats(&hist, ac::CSI_WINDOW_MS, Instant::now());
                update_snapshot(
                    State::Calibrating,
                    0.0,
                    m,
                    sd,
                    0.0,
                    rssi_last,
                    frames_total,
                );
            }
            continue;
        }

        // ---- 窗口统计（时长窗口；样本不足 = 帧率异常低，保持现状）----
        let (amp, std, n) = window_stats(&hist, ac::CSI_WINDOW_MS, Instant::now());
        if n < ac::CSI_WINDOW_MIN_FRAMES {
            continue;
        }
        let drop = if baseline > 0.0 {
            (baseline - amp) / baseline
        } else {
            0.0
        };
        // 帧间波动相对值（主判据）：静场 ≈0.023，人体贴近活动实测 ≈0.135
        let rel_std = if amp > 0.0 { std / amp } else { 0.0 };

        // ---- 状态机：迟滞阈值 + 连续达标防抖 ----
        // 有人：波动骤增（人体活动）或幅度显著衰减（遮挡型贴近）任一达标
        // 无人：两者都回落到迟滞阈值以内
        let target = if rel_std >= ac::CSI_ENTER_STD || drop >= ac::CSI_ENTER_DROP {
            Some(State::Present)
        } else if rel_std <= ac::CSI_LEAVE_STD && drop <= ac::CSI_LEAVE_DROP {
            Some(State::Absent)
        } else {
            None // 迟滞带内：不翻转也不累计
        };

        match target {
            Some(t) if t == state => cand = None,
            Some(t) => {
                let since = match cand {
                    Some((s, at)) if s == t => at,
                    _ => Instant::now(),
                };
                cand = Some((t, since));
                let sustain = if t == State::Present {
                    ac::CSI_ENTER_SUSTAIN_MS
                } else {
                    ac::CSI_LEAVE_SUSTAIN_MS
                };
                if since.elapsed() >= Duration::from_millis(sustain) {
                    state = t;
                    cand = None;
                    let snap =
                        update_snapshot(state, baseline, amp, std, drop, rssi_last, frames_total);
                    ::log::info!(
                        "CSI presence: 1m {} (drop={:.3}, baseline={:.1}, amp={:.1}, std={:.2})",
                        if state == State::Present { "有人" } else { "无人" },
                        drop,
                        baseline,
                        amp,
                        std
                    );
                    publish_event("change", state, &snap);
                }
            }
            None => cand = None,
        }

        // ---- 基线自适应：仅「无人」状态（有人时冻结，防久坐被吸进基线）----
        if state == State::Absent {
            baseline += ac::CSI_BASELINE_ALPHA * (amp - baseline);
        }

        update_snapshot(state, baseline, amp, std, drop, rssi_last, frames_total);
    }
}

/// 当前感知状态（"calibrating" / "absent" / "present"，供 DeviceStatus 等读取）
pub fn presence() -> &'static str {
    SNAPSHOT.lock().map(|s| s.state).unwrap_or("calibrating")
}

/// 串口 `csi` 命令输出
pub fn snapshot_text() -> String {
    let s = SNAPSHOT.lock().unwrap();
    let rs = if s.amp > 0.0 { s.std / s.amp } else { 0.0 };
    format!(
        "csi state={} baseline={:.1} amp={:.1} std={:.2} rs={:.3} drop={:.3} rssi={} dBm frames={}\r\n\
         判定: rs(std/amp)>={:.2} 或 drop>={:.2} 连续 {}ms → 1m有人；rs<={:.2} 且 drop<={:.2} 连续 {}ms → 1m无人（csi recal 重新校准）",
        s.state,
        s.baseline,
        s.amp,
        s.std,
        rs,
        s.drop,
        s.rssi,
        s.frames,
        ac::CSI_ENTER_STD,
        ac::CSI_ENTER_DROP,
        ac::CSI_ENTER_SUSTAIN_MS,
        ac::CSI_LEAVE_STD,
        ac::CSI_LEAVE_DROP,
        ac::CSI_LEAVE_SUSTAIN_MS,
    )
}

/// 串口 `csi recal`：请求重新校准基线（检测线程下拍执行）
pub fn recalibrate() {
    RECAL.store(true, Ordering::SeqCst);
}

/// 拿不到默认网关时的兜底 ping 目标（阿里公共 DNS，国内常可达）
const PING_FALLBACK: [u8; 4] = [223, 5, 5, 5];

/// UDP 探测流量线程：向默认网关单播小包。**每帧都被 AP MAC 层 ACK**，
/// `dump_ack_en` 下 ACK 帧出 CSI——这是主帧源：速率稳定，不依赖对端回应
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
/// 补充**数据帧** CSI（echo reply 走 OFDM/HT 速率，LTF 齐全，与 ACK 帧的
/// LLTF 互补）。CSI 只对 OFDM/HT 速率帧生成，AP beacon 走 CCK 基本速率不出
/// CSI（静默时 ~0.2 帧/秒），故必须主动制造下行流量。
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

/// 启动近场感知：关闭 Wi-Fi 省电 → 注册 CSI 配置/回调并使能 → 启动 ping 探测流量
/// → 拉起检测线程。需在 Wi-Fi 启动后调用；CSI 数据只在收到帧时产生。
pub fn start() -> Result<(), esp_err_t> {
    // 省电模式（modem sleep）下射频大部分时间休眠，CSI 帧率只有 ~0.2 帧/秒，
    // 校准要 3 分钟、检测时序失真；本设备常供电，直接关闭省电换帧率（~10 帧/s）。
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
        dump_ack_en: true, // AP 对上行帧的 ACK 也出 CSI（补帧率，尽力而为）
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
    ::log::info!("Wi-Fi CSI enabled (near-field presence probe)");

    // 主动探测流量（帧率来源，见模块注释）：
    // ① UDP 单播 → AP MAC ACK（主帧源，20 包/s）；② ICMP echo reply（数据帧补充）
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
