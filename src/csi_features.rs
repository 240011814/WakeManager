//! WakeManager - Wi-Fi CSI 特征流水线（纯计算，无 ESP-IDF 依赖）
//!
//! 参照 ESPectre（github.com/francescopace/espectre，轻量检测器）的公开算法
//! 在 Rust 中独立重实现，数值公式与其 C++/Python 实现对齐（同参同式）：
//!
//! 1. **HT20 归一化**：不同载荷长度（128B 全宽 / 256B 双份取前一份 / 114B
//!    短 HT 57 子载波 / 228B 双份 / 106B 紧凑 LLTF 53 子载波）统一到居中
//!    布局的 64 bin 幅度谱（bin = 子载波 + 32，DC 在 bin 32）。经典 MAC 芯片
//!    （ESP32/S3）原生顺序是 "0~31, -32~-1"（DC 在 bin 0），用保护带 null
//!    证据判别后旋转半段对调到居中布局。LLTF 载荷缺 ±27/±28 四个边音
//!    （bin 4/5/59/60），从最近的 ±26（bin 6/58）补齐。
//! 2. **空间湍流**（spatial turbulence）= 12 个固定子载波幅度的变异系数
//!    std/mean。比值形式**增益无关**：AGC/发射功率/距离引起的整体缩放不影响
//!    特征，天然免疫慢漂移，无需基线校准。
//! 3. **聚合湍流**：每个频带子载波先替换为相邻 5 bin 的幅度均值（DC 跳过、
//!    窗口钳在保护带 4..60 内）再算 CV——压掉单音噪声，保留 3MHz 相干的人体
//!    扰动。
//! 4. **Hampel 去野值**（窗 7、5×MAD×1.4826）：重传/硬件毛刺替换为中位数。
//! 5. **窗口特征**：湍流序列 lag-1 自相关（人活动时序列平滑相关 ↑，空场白
//!    噪声 ≈0）+ 聚合湍流 IQR/均值（活动时的突发散布）。
//! 6. **固定权重 logistic 融合**：两个特征 z-score 后线性加权 + sigmoid
//!    → 运动概率（常数取自其训练/验证流程的拟合值）。
//!
//! 12 子载波频带（子载波 ±4, ±9, ±14, ±19, ±24, ±28）：人体扰动在 ~10 个
//! 子载波（3.1MHz）内相干，而空场噪声逐音独立——跨全带宽取样买的是
//! 「独立观察次数」。

/// HT20 子载波数（64 bin，bin = 子载波 + 32，DC 在 bin 32）
pub const HT20_BINS: usize = 64;

/// 检测频带：12 个固定子载波（居中布局 bin 索引）
pub const BAND: [usize; 12] = [4, 8, 13, 18, 23, 28, 36, 41, 46, 51, 56, 60];

/// 保护带边界（bin 4..=60 为可用频带）
pub const GUARD_LOW: usize = 4;
pub const GUARD_HIGH: usize = 60;
/// DC null
pub const DC_BIN: usize = 32;
/// 聚合窗宽（相邻 bin 数，奇数）
pub const AGG_WIDTH: usize = 5;

/// 居中布局下只在「经典布局」为 null 的 bin：其中任一有能量 → 不是经典布局
const CLASSIC_ONLY_NULL: [usize; 6] = [29, 30, 31, 33, 34, 35];
/// 居中布局下只在「居中布局」为 null 的 bin：其中任一有能量 → 不是居中布局
const CENTERED_ONLY_NULL: [usize; 6] = [1, 2, 3, 61, 62, 63];

/// MAD 尺度系数（正态分布下 MAD → σ 的换算）
pub const MAD_SCALE: f32 = 1.4826;

// ---------------------------------------------------------------------------
// 固定权重 logistic 融合（ESPectre 轻量检测器拟合常数；别手改，改阈值即可）
// ---------------------------------------------------------------------------
const FUSION_AC_CENTER: f32 = 0.391_934_5;
const FUSION_AC_SCALE: f32 = 0.379_864_84;
const FUSION_AC_WEIGHT: f32 = 5.083_034_5;
const FUSION_IQR_CENTER: f32 = 0.246_121_4;
const FUSION_IQR_SCALE: f32 = 0.200_565_99;
const FUSION_IQR_WEIGHT: f32 = 4.997_501_7;
const FUSION_INTERCEPT: f32 = 1.077_677;

/// HT20 bin 布局
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// bin = 子载波 + 32（Wi-Fi 6 芯片 / 归一化后）
    Centered,
    /// Espressif 原生 "0~31, -32~-1"，DC 在 bin 0（经典 MAC 芯片）
    Classic,
}

/// 原始 I/Q 载荷 → 居中布局 64 bin 幅度谱；载荷无法可靠归一化时 None（丢帧）。
///
/// 支持长度：106（紧凑 LLTF 53SC）、114（短 HT 57SC）、228（双份短 HT 取前份）、
/// 128 及以上（全宽 64SC；256 双份取前份）。其余长度返回 None。
pub fn amplitudes_from_raw(raw: &[i8], first_word_invalid: bool) -> Option<[f32; HT20_BINS]> {
    let mut energy = [0f32; HT20_BINS];
    let total: f32;

    match raw.len() {
        // 紧凑 LLTF：53 子载波 -26..+26（含 DC），左垫 6 bin → DC 落 bin 32。
        // ±27/±28（bin 4/5/59/60）此载荷不含 → 由最近的 ±26（bin 6/58）补齐。
        106 => {
            fill_energies(&raw[..106], &mut energy, 6);
            energy[4] = energy[6];
            energy[5] = energy[6];
            energy[59] = energy[58];
            energy[60] = energy[58];
            total = energy_total(&energy);
        }
        // 短 HT：57 子载波 -28..+28，左垫 4 bin → 覆盖 bin 4..60，无需补音。
        114 | 228 => {
            fill_energies(&raw[..114], &mut energy, 4);
            total = energy_total(&energy);
        }
        // 全宽 64 子载波：判布局（经典布局旋转半段到居中）。
        n if n >= 128 => {
            fill_energies(&raw[..128], &mut energy, 0);
            match detect_layout(&energy, first_word_invalid) {
                Some(Layout::Centered) => {}
                Some(Layout::Classic) => rotate_halves(&mut energy),
                None => return None, // 稀疏/退化/非 HT 载荷：不可用
            }
            // LLTF-only 全宽载荷：±27/±28 为 0 → 与短载荷一致地补齐
            if energy[4] == 0.0 && energy[5] == 0.0 && energy[59] == 0.0 && energy[60] == 0.0
            {
                if energy[6] > 0.0 {
                    energy[4] = energy[6];
                    energy[5] = energy[6];
                }
                if energy[58] > 0.0 {
                    energy[59] = energy[58];
                    energy[60] = energy[58];
                }
            }
            total = energy_total(&energy);
        }
        _ => return None,
    }

    if total <= 0.0 {
        return None; // 死帧（全零）
    }
    let mut amps = [0f32; HT20_BINS];
    for (a, e) in amps.iter_mut().zip(energy.iter()) {
        *a = e.sqrt();
    }
    Some(amps)
}

/// 一帧 CSI →（湍流, 聚合湍流）。载荷无法归一化时 None（丢帧）。
pub fn frame_turbulence(raw: &[i8], first_word_invalid: bool) -> Option<(f32, f32)> {
    let amps = amplitudes_from_raw(raw, first_word_invalid)?;
    Some((band_turbulence(&amps), aggr_turbulence(&amps)))
}

/// I/Q 交织字节（每音 2 字节，byte0=虚部、byte1=实部）→ 平方幅度，左垫 `pad` 个 bin。
fn fill_energies(raw: &[i8], energy: &mut [f32; HT20_BINS], pad: usize) {
    let n = (raw.len() / 2).min(HT20_BINS.saturating_sub(pad));
    for k in 0..n {
        let i = raw[2 * k] as f32;
        let q = raw[2 * k + 1] as f32;
        energy[pad + k] = i * i + q * q;
    }
}

fn energy_total(energy: &[f32; HT20_BINS]) -> f32 {
    energy.iter().sum()
}

/// 用保护带 null 证据判别布局（要求双向正证据：一组防护 bin 全零且另一组全非零）
fn detect_layout(energy: &[f32; HT20_BINS], first_word_invalid: bool) -> Option<Layout> {
    let classic_energy = CLASSIC_ONLY_NULL.iter().filter(|&&b| energy[b] > 0.0).count();
    let centered_energy = CENTERED_ONLY_NULL
        .iter()
        .filter(|&&b| !(first_word_invalid && b < 2) && energy[b] > 0.0)
        .count();
    // first_word_invalid 时 bin 0/1 的字节被硬件标记无效 → 居中侧少要求一个
    let centered_need = if first_word_invalid { 5 } else { 6 };

    if classic_energy == 0 && centered_energy == centered_need {
        Some(Layout::Classic)
    } else if centered_energy == 0 && classic_energy == 6 {
        Some(Layout::Centered)
    } else {
        None
    }
}

/// 旋转半段（对合操作：一次交换即完成 经典 ↔ 居中 映射）
fn rotate_halves(v: &mut [f32; HT20_BINS]) {
    let (left, right) = v.split_at_mut(HT20_BINS / 2);
    for (a, b) in left.iter_mut().zip(right.iter_mut()) {
        core::mem::swap(a, b);
    }
}

/// 空间湍流：一组幅度值的变异系数 std/mean（增益无关；mean ≤ 0 → 0）
pub fn spatial_turbulence(amps: &[f32]) -> f32 {
    let (mean, var) = mean_var2(amps);
    if mean > 0.0 {
        var.max(0.0).sqrt() / mean
    } else {
        0.0
    }
}

/// 频带湍流：12 个固定子载波幅度的 CV
pub fn band_turbulence(amps: &[f32; HT20_BINS]) -> f32 {
    let mut sel = [0f32; 12];
    for (i, &b) in BAND.iter().enumerate() {
        sel[i] = amps[b];
    }
    spatial_turbulence(&sel)
}

/// 聚合频带：每个频带子载波 → 相邻 5 bin 幅度均值（DC 跳过、钳在 4..60）
fn aggregated_band(amps: &[f32; HT20_BINS]) -> [f32; 12] {
    const HALF: isize = (AGG_WIDTH as isize - 1) / 2;
    let mut out = [0f32; 12];
    for (i, &b) in BAND.iter().enumerate() {
        let mut low = b as isize - HALF;
        let mut high = low + AGG_WIDTH as isize - 1;
        if low < GUARD_LOW as isize {
            low = GUARD_LOW as isize;
            high = low + AGG_WIDTH as isize - 1;
        }
        if high > GUARD_HIGH as isize {
            high = GUARD_HIGH as isize;
            low = high - AGG_WIDTH as isize + 1;
        }
        let mut sum = 0f32;
        let mut n = 0u32;
        for bin in low..=high {
            if bin == DC_BIN as isize {
                continue;
            }
            sum += amps[bin as usize];
            n += 1;
        }
        out[i] = if n > 0 { sum / n as f32 } else { 0.0 };
    }
    out
}

/// 聚合湍流：五 bin 邻域均值幅度序列的 CV
pub fn aggr_turbulence(amps: &[f32; HT20_BINS]) -> f32 {
    spatial_turbulence(&aggregated_band(amps))
}

/// 两遍法均值/总体方差（数值稳定；与 ESPectre 对齐）
pub fn mean_var2(values: &[f32]) -> (f32, f32) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let n = values.len() as f32;
    let mut mean = 0f32;
    for &x in values {
        mean += x;
    }
    mean /= n;
    let mut var = 0f32;
    for &x in values {
        let d = x - mean;
        var += d * d;
    }
    var /= n;
    (mean, var)
}

/// 湍流序列 lag-1 自相关（归一化自协方差）。空场白噪声 ≈ 0；人活动时
/// 小幅慢波动使相邻样本相关 → 升高。
pub fn autocorr_lag1(values: &[f32]) -> f32 {
    let count = values.len();
    if count < 3 {
        return 0.0;
    }
    let (mean, var) = mean_var2(values);
    if var < 1e-10 {
        return 0.0;
    }
    let mut cov = 0f32;
    for i in 0..count - 1 {
        cov += (values[i] - mean) * (values[i + 1] - mean);
    }
    cov /= (count - 1) as f32;
    cov / var
}

/// IQR/mean 的稳健散布（IQR = Q75 − Q25，线性插值分位数）
pub fn iqr_over_mean(values: &[f32]) -> f32 {
    let count = values.len();
    if count < 2 {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let q75 = percentile_sorted(&sorted, 0.75);
    let q25 = percentile_sorted(&sorted, 0.25);
    let mean = values.iter().sum::<f32>() / count as f32;
    (q75 - q25) / mean.abs().max(1e-6)
}

fn percentile_sorted(sorted: &[f32], q: f32) -> f32 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    let pos = (n - 1) as f32 * q;
    let lower = pos as usize;
    if lower >= n - 1 {
        return sorted[n - 1];
    }
    let frac = pos - lower as f32;
    sorted[lower] * (1.0 - frac) + sorted[lower + 1] * frac
}

fn median_sorted(sorted: &[f32]) -> f32 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 0 {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    }
}

/// Hampel 去野值（窗 3..11 样本、threshold×MAD×1.4826 判野值 → 替换为中位数）
pub struct Hampel {
    window: usize,
    threshold: f32,
    buf: [f32; 11],
    idx: usize,
    count: usize,
}

impl Hampel {
    pub fn new(window: usize, threshold: f32) -> Self {
        Self {
            window: window.clamp(3, 11),
            threshold,
            buf: [0f32; 11],
            idx: 0,
            count: 0,
        }
    }

    /// 推入新样本，返回去野值后的值（窗口不足 3 个样本时原样返回）
    pub fn filter(&mut self, x: f32) -> f32 {
        if !x.is_finite() {
            return x;
        }
        self.buf[self.idx] = x;
        self.idx = (self.idx + 1) % self.window;
        if self.count < self.window {
            self.count += 1;
        }
        if self.count < 3 {
            return x;
        }

        let mut sorted = [0f32; 11];
        sorted[..self.count].copy_from_slice(&self.buf[..self.count]);
        sorted[..self.count]
            .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        let median = median_sorted(&sorted[..self.count]);

        let mut dev = [0f32; 11];
        for i in 0..self.count {
            dev[i] = (self.buf[i] - median).abs();
        }
        dev[..self.count]
            .sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        let mad = median_sorted(&dev[..self.count]);

        if mad > 1e-6 && (x - median).abs() > self.threshold * MAD_SCALE * mad {
            median
        } else {
            x
        }
    }
}

/// 固定权重 logistic 融合 → 运动概率 [0,1]
pub fn motion_probability(turb_ac: f32, turb_iqr_over_mean: f32) -> f32 {
    let logit = FUSION_INTERCEPT
        + FUSION_AC_WEIGHT * (turb_ac - FUSION_AC_CENTER) / FUSION_AC_SCALE
        + FUSION_IQR_WEIGHT * (turb_iqr_over_mean - FUSION_IQR_CENTER) / FUSION_IQR_SCALE;
    sigmoid(logit)
}

fn sigmoid(x: f32) -> f32 {
    if x < -20.0 {
        0.0
    } else if x > 20.0 {
        1.0
    } else {
        1.0 / (1.0 + (-x).exp())
    }
}
