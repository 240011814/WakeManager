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
//! **开机即关闭 Wi-Fi 省电**（`esp_wifi_set_ps(WIFI_PS_NONE)`）：省电模式下射频大部分
//! 时间休眠，CSI 帧率只有 ~0.2 帧/秒（实测 40 帧收了 3 分钟），检测时序完全失真；
//! 常开后约 10 帧/秒（beacon 速率），校准 4s、窗口 2s 才符合设计。
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use esp_idf_svc::sys::*;

use crate::app_config as ac;
use crate::control_mqtt;
use crate::device_info;

/// 一帧 CSI 的轻量统计（回调 → 检测线程）
struct FrameStat {
    /// 幅度度量：各子载波平均功率开方（同一度量下做相对比较，无需绝对标定）
    amp: f32,
    rssi: i8,
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
    };

    if let Ok(mut ring) = RING.lock() {
        if ring.len() >= ac::CSI_HIST_CAP {
            ring.pop_front();
        }
        ring.push_back(stat);
    }
}

/// 最近 `n` 帧幅度的 (均值, 总体标准差)
fn window_stats(hist: &VecDeque<f32>, n: usize) -> (f32, f32) {
    let take = n.min(hist.len());
    if take == 0 {
        return (0.0, 0.0);
    }
    let mut sum = 0f32;
    for v in hist.iter().skip(hist.len() - take) {
        sum += *v;
    }
    let mean = sum / take as f32;
    let mut var = 0f32;
    for v in hist.iter().skip(hist.len() - take) {
        let d = v - mean;
        var += d * d;
    }
    (mean, (var / take as f32).sqrt())
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

    let mut hist: VecDeque<f32> = VecDeque::new(); // 帧幅度历史（滑动窗口取尾部）
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
                hist.push_back(f.amp);
                rssi_last = f.rssi;
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
                let (amp, _) = window_stats(&hist, hist.len());
                baseline = amp;
                state = State::Absent;
                cand = None;
                calib_logged = false;
                ::log::info!(
                    "CSI baseline calibrated: amp={baseline:.1} ({} frames, rssi={} dBm)",
                    hist.len(),
                    rssi_last
                );
                let (m, sd) = window_stats(&hist, ac::CSI_WINDOW_FRAMES);
                let drop = if baseline > 0.0 {
                    (baseline - m) / baseline
                } else {
                    0.0
                };
                let snap = update_snapshot(State::Absent, baseline, m, sd, drop, rssi_last, frames_total);
                publish_event("init", State::Absent, &snap);
            } else {
                // 校准中也刷新测量值快照，定期打印可观察当前读数
                let (m, sd) = window_stats(&hist, ac::CSI_WINDOW_FRAMES);
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

        // ---- 窗口统计（帧不足一窗 = Wi-Fi 过于安静，保持现状）----
        if hist.len() < ac::CSI_WINDOW_FRAMES {
            continue;
        }
        let (amp, std) = window_stats(&hist, ac::CSI_WINDOW_FRAMES);
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

/// 启动近场感知：关闭 Wi-Fi 省电 → 注册 CSI 配置/回调并使能 → 拉起检测线程。
/// 需在 Wi-Fi 启动后调用；CSI 数据只在收到帧（AP beacon 等）时产生。
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
        dump_ack_en: false, // 不需要 ACK 帧的 CSI
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

    std::thread::Builder::new()
        .name("csi_det".into())
        // 栈含日志格式化（localtime_r）+ serde_json + FFI 余量；4096 实测栈溢出
        //（task pthread 栈金丝雀报警，2026-10 真机），8192 与 mqtt 事件线程同级
        .stack_size(8192)
        .spawn(detector_task)
        .expect("spawn csi detector task");
    Ok(())
}
