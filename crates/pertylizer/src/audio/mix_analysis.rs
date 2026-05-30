//! Mix-bus audio analysis primitives.
//!
//! Takes a stereo-interleaved f32 buffer (the master-bus output of an offline
//! arrangement render) and returns metrics relevant for judging a mix as a
//! whole rather than a single sound:
//!
//! - Peak / RMS / crest factor.
//! - **True peak** (inter-sample peak) via 4× polyphase oversampling per
//!   ITU-R BS.1770-4 Annex 2.
//! - **Integrated, short-term, and momentary LUFS** following ITU-R BS.1770-4
//!   (K-weighting + 400 ms gating blocks at 75% overlap, absolute -70 LUFS
//!   gate, relative -10 LU gate). LUFS-M (momentary) is the max single-block
//!   loudness; LUFS-S (short-term) is the max over a 3 s sliding window
//!   (= 30 consecutive 100 ms hops).
//! - 4-band frequency balance (delegates to `analysis::energy_bands` on a
//!   mono mix-down).
//! - Stereo correlation (delegates to `analysis::stereo_correlation`).
//! - Mid/side RMS split.
//! - Mono-compatibility score (how much energy survives an L+R sum).
//!
//! All analysis is offline and deterministic. The K-weighting filter
//! coefficients are hard-wired for 44.1 kHz, which matches the default
//! (full-quality) offline render path. Sample rates near 44.1 kHz produce only
//! slightly biased LUFS readings (typically within ~0.5 dB), but the 22.05 kHz
//! `draft` render rate shifts the K-weighting knees enough to bias LUFS
//! noticeably — draft LUFS is documented as unreliable. The rest of the metrics
//! are sample-rate independent (band edges and oversampling adapt to the rate).

use std::sync::OnceLock;

use crate::audio::analysis::{
    EnergyBands, energy_bands, peak_amplitude, rms_overall, stereo_correlation,
};

/// Sentinel substitute for `-inf` so JSON consumers don't have to handle
/// non-finite values. Reported by `lin_to_db` and the LUFS path for silence.
pub const SILENT_FLOOR_DBFS: f32 = -200.0;

/// Sample amplitude that counts as clipping for the `clipped_samples` metric.
/// Slightly below 1.0 catches loud-but-not-quite-fullscale floats that any
/// downstream int16 conversion would also wrap.
const CLIP_THRESHOLD: f32 = 0.999;

/// Window length used by `energy_bands_windowed` so band energy reflects the
/// whole buffer rather than the head-truncated 65 K FFT cap. ~1 s at any
/// reasonable sample rate.
const BAND_WINDOW_SECONDS: f32 = 1.0;

/// Output of [`analyze_mix_buffer`]. All dB fields are dBFS unless noted.
#[derive(Debug, Clone, Copy)]
pub struct MixAnalysis {
    /// Sample peak across both channels, linear (0.0..=1.0+).
    pub peak: f32,
    /// Sample peak in dBFS. `-inf` is reported as -200.0.
    pub peak_dbfs: f32,
    /// Sample peak of the left channel only, linear.
    pub peak_left: f32,
    /// Sample peak of the right channel only, linear.
    pub peak_right: f32,
    /// Inter-sample (true) peak across both channels, linear. Computed by 4×
    /// polyphase oversampling per ITU-R BS.1770-4 Annex 2. Always ≥ `peak` —
    /// a band-limited signal can exceed the worst sample-grid value between
    /// samples, and that overshoot is what surfaces as clipping after DA
    /// conversion or lossy encoding.
    pub true_peak: f32,
    /// True peak in dBTP (dB true peak). `-inf` reported as -200.0.
    pub true_peak_dbtp: f32,
    /// Overall RMS, linear (0.0..=1.0+).
    pub rms: f32,
    /// Overall RMS in dBFS. `-inf` reported as -200.0.
    pub rms_dbfs: f32,
    /// Crest factor = peak_dBFS - rms_dBFS. Higher = more dynamic.
    pub crest_factor_db: f32,
    /// Integrated loudness (ITU-R BS.1770-4 LUFS). `-inf` reported as -200.0.
    pub lufs_integrated: f32,
    /// Maximum momentary loudness over the buffer (single 400 ms K-weighted
    /// block, no overlap inside the window). `-inf` reported as -200.0.
    pub lufs_momentary_max: f32,
    /// Maximum short-term loudness over the buffer (3 s K-weighted window,
    /// stepped every 100 ms). `-inf` reported as -200.0.
    pub lufs_short_term_max: f32,
    /// 4-band frequency energy on the mono mix-down (sub/low/mid/high).
    pub energy_bands: EnergyBands,
    /// Pearson correlation between L and R channels, [-1.0, 1.0].
    pub stereo_correlation: f32,
    /// RMS of the mid (L+R)/2 component, linear.
    pub mid_rms: f32,
    /// RMS of the side (L-R)/2 component, linear.
    pub side_rms: f32,
    /// Stereo width: side_rms / mid_rms, clamped to a finite value. 0.0 = mono.
    pub stereo_width: f32,
    /// Mono-compatibility score, 0.0..=1.0. Defined as 2 * mid_rms / (L_rms + R_rms);
    /// 1.0 = perfectly mono-summable (L and R fully in phase),
    /// 0.0 = full anti-phase cancellation, total loss when summed.
    pub mono_compat: f32,
    /// Count of samples whose absolute value reached or exceeded 0.999 — non-zero
    /// indicates clipping into the f32 fullscale ceiling.
    pub clipped_samples: u32,
}

/// Analyze a stereo-interleaved buffer (L0, R0, L1, R1, ...).
#[must_use]
pub fn analyze_mix_buffer(stereo: &[f32], sample_rate: u32) -> MixAnalysis {
    if stereo.is_empty() || sample_rate == 0 {
        return zero_analysis();
    }

    let n_frames = stereo.len() / 2;
    let peak = peak_amplitude(stereo);
    let rms = rms_overall(stereo);
    let peak_dbfs = lin_to_db(peak);
    let rms_dbfs = lin_to_db(rms);
    let crest_factor_db = (peak_dbfs - rms_dbfs).max(0.0);

    // Mid (L+R)/2 doubles as the mono mix-down for band analysis.
    let mut mid = Vec::with_capacity(n_frames);
    let mut side = Vec::with_capacity(n_frames);
    let mut left_sum_sq = 0.0_f64;
    let mut right_sum_sq = 0.0_f64;
    let mut clipped = 0u32;
    let mut peak_left = 0.0_f32;
    let mut peak_right = 0.0_f32;
    for frame in stereo.chunks_exact(2) {
        let l = frame[0];
        let r = frame[1];
        let la = l.abs();
        let ra = r.abs();
        if la > peak_left {
            peak_left = la;
        }
        if ra > peak_right {
            peak_right = ra;
        }
        if la >= CLIP_THRESHOLD {
            clipped += 1;
        }
        if ra >= CLIP_THRESHOLD {
            clipped += 1;
        }
        mid.push((l + r) * 0.5);
        side.push((l - r) * 0.5);
        left_sum_sq += f64::from(l) * f64::from(l);
        right_sum_sq += f64::from(r) * f64::from(r);
    }
    let bands = energy_bands_windowed(&mid, sample_rate);
    let correlation = stereo_correlation(stereo);
    let mid_rms = rms_overall(&mid);
    let side_rms = rms_overall(&side);

    let stereo_width = if mid_rms > 1e-9 {
        side_rms / mid_rms
    } else {
        0.0
    };

    let left_rms = (left_sum_sq / n_frames as f64).sqrt() as f32;
    let right_rms = (right_sum_sq / n_frames as f64).sqrt() as f32;
    let lr_sum = left_rms + right_rms;
    let mono_compat = if lr_sum > 1e-9 {
        ((2.0 * mid_rms) / lr_sum).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let loudness = compute_loudness(stereo, sample_rate);
    let true_peak = compute_true_peak_stereo(stereo);
    let true_peak_dbtp = lin_to_db(true_peak);

    MixAnalysis {
        peak,
        peak_dbfs,
        peak_left,
        peak_right,
        true_peak,
        true_peak_dbtp,
        rms,
        rms_dbfs,
        crest_factor_db,
        lufs_integrated: loudness.integrated,
        lufs_momentary_max: loudness.momentary_max,
        lufs_short_term_max: loudness.short_term_max,
        energy_bands: bands,
        stereo_correlation: correlation,
        mid_rms,
        side_rms,
        stereo_width,
        mono_compat,
        clipped_samples: clipped,
    }
}

fn zero_analysis() -> MixAnalysis {
    MixAnalysis {
        peak: 0.0,
        peak_dbfs: SILENT_FLOOR_DBFS,
        peak_left: 0.0,
        peak_right: 0.0,
        true_peak: 0.0,
        true_peak_dbtp: SILENT_FLOOR_DBFS,
        rms: 0.0,
        rms_dbfs: SILENT_FLOOR_DBFS,
        crest_factor_db: 0.0,
        lufs_integrated: SILENT_FLOOR_DBFS,
        lufs_momentary_max: SILENT_FLOOR_DBFS,
        lufs_short_term_max: SILENT_FLOOR_DBFS,
        energy_bands: EnergyBands {
            sub: 0.0,
            low: 0.0,
            mid: 0.0,
            high: 0.0,
        },
        stereo_correlation: 0.0,
        mid_rms: 0.0,
        side_rms: 0.0,
        stereo_width: 0.0,
        mono_compat: 0.0,
        clipped_samples: 0,
    }
}

/// Convert a linear amplitude to dBFS. Sub-zero / non-positive inputs collapse
/// to [`SILENT_FLOOR_DBFS`] so JSON consumers never see `-inf`.
#[inline]
pub(crate) fn lin_to_db(linear: f32) -> f32 {
    if linear > 0.0 {
        20.0 * linear.log10()
    } else {
        SILENT_FLOOR_DBFS
    }
}

/// `energy_bands` averaged over non-overlapping ~1 s windows. The underlying
/// FFT caps at 65 K bins (≈1.5 s at 44.1 kHz), so calling it directly on a
/// multi-minute buffer would silently truncate to the head. Windowing covers
/// the whole buffer and yields a stable band balance.
fn energy_bands_windowed(mono: &[f32], sample_rate: u32) -> EnergyBands {
    if mono.is_empty() || sample_rate == 0 {
        return EnergyBands {
            sub: 0.0,
            low: 0.0,
            mid: 0.0,
            high: 0.0,
        };
    }
    let window = (BAND_WINDOW_SECONDS * sample_rate as f32) as usize;
    if mono.len() <= window {
        return energy_bands(mono, sample_rate);
    }
    let mut sub = 0.0f32;
    let mut low = 0.0f32;
    let mut mid = 0.0f32;
    let mut high = 0.0f32;
    let mut count = 0u32;
    for chunk in mono.chunks(window) {
        if chunk.len() < window / 4 {
            // Tail chunk too short to give a stable FFT — skip.
            continue;
        }
        let b = energy_bands(chunk, sample_rate);
        sub += b.sub;
        low += b.low;
        mid += b.mid;
        high += b.high;
        count += 1;
    }
    let n = count.max(1) as f32;
    EnergyBands {
        sub: sub / n,
        low: low / n,
        mid: mid / n,
        high: high / n,
    }
}

// ---------------------------------------------------------------------------
// LUFS / ITU-R BS.1770-4
// ---------------------------------------------------------------------------

/// Stage-1 K-weighting filter (high-shelf, +4 dB) at 44.1 kHz.
/// Pre-warped bilinear-transform coefficients from the ITU-R BS.1770-4
/// analog prototype.
#[allow(clippy::excessive_precision)]
const K_PRE_B: [f32; 3] = [1.530841230049835, -2.650979900183889, 1.169079079941625];
#[allow(clippy::excessive_precision)]
const K_PRE_A: [f32; 3] = [1.0, -1.663655113256020, 0.712595428073225];

/// Stage-2 RLB filter (high-pass, ~38 Hz) at 44.1 kHz.
#[allow(clippy::excessive_precision)]
const K_RLB_B: [f32; 3] = [1.0, -2.0, 1.0];
#[allow(clippy::excessive_precision)]
const K_RLB_A: [f32; 3] = [1.0, -1.989169673629770, 0.989199035787039];

/// Simple biquad direct-form-I filter, single channel.
fn biquad_filter_inplace(samples: &mut [f32], b: &[f32; 3], a: &[f32; 3]) {
    let mut x1 = 0.0f64;
    let mut x2 = 0.0f64;
    let mut y1 = 0.0f64;
    let mut y2 = 0.0f64;
    let b0 = f64::from(b[0]);
    let b1 = f64::from(b[1]);
    let b2 = f64::from(b[2]);
    let a1 = f64::from(a[1]);
    let a2 = f64::from(a[2]);
    for s in samples.iter_mut() {
        let x0 = f64::from(*s);
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        x2 = x1;
        x1 = x0;
        y2 = y1;
        y1 = y0;
        *s = y0 as f32;
    }
}

/// Apply K-weighting (pre-filter + RLB) to a mono channel in place.
fn k_weight_inplace(samples: &mut [f32]) {
    biquad_filter_inplace(samples, &K_PRE_B, &K_PRE_A);
    biquad_filter_inplace(samples, &K_RLB_B, &K_RLB_A);
}

/// Combined LUFS-I / LUFS-M / LUFS-S results from a single K-weighting pass.
struct LoudnessSummary {
    /// Integrated LUFS (ITU-R BS.1770-4, abs/rel gated). `-200.0` when silent.
    integrated: f32,
    /// Maximum momentary loudness — max over single 400 ms K-weighted blocks.
    /// `-200.0` when no block was audible.
    momentary_max: f32,
    /// Maximum short-term loudness — max over 3 s K-weighted sliding windows
    /// (30 consecutive 100 ms hops). `-200.0` when no window was audible or
    /// the buffer is shorter than 3 s.
    short_term_max: f32,
}

impl LoudnessSummary {
    const SILENT: Self = Self {
        integrated: SILENT_FLOOR_DBFS,
        momentary_max: SILENT_FLOOR_DBFS,
        short_term_max: SILENT_FLOOR_DBFS,
    };
}

/// Per ITU-R BS.1770-4 / EBU R 128: integrated + momentary-max + short-term-max
/// loudness in one pass.
///
/// Reuses the K-weighting + 400 ms / 100 ms-hop block decomposition for all
/// three readouts. Momentary loudness == single 400 ms block. Short-term
/// loudness == 3 s sliding window stepped every 100 ms (= 30 consecutive
/// blocks at 75 % overlap = 0.4 s + 29 × 0.1 s = 3.3 s total cover; ITU-R
/// allows ±10 % on the 3 s requirement, so 3.3 s is in spec).
///
/// Stereo (channel weights 1.0, 1.0). Buffers shorter than 400 ms produce
/// `-200.0` for integrated / momentary; buffers shorter than 3.3 s produce
/// `-200.0` for short_term.
fn compute_loudness(stereo: &[f32], sample_rate: u32) -> LoudnessSummary {
    let n_frames = stereo.len() / 2;
    if n_frames == 0 || sample_rate == 0 {
        return LoudnessSummary::SILENT;
    }
    let block_samples = (sample_rate as usize * 4) / 10; // 400 ms
    let hop_samples = block_samples / 4; // 75% overlap → 100 ms hop
    if n_frames < block_samples {
        return LoudnessSummary::SILENT;
    }

    let block_energy = k_weighted_block_energies(stereo, n_frames, block_samples, hop_samples);
    if block_energy.is_empty() {
        return LoudnessSummary::SILENT;
    }

    LoudnessSummary {
        integrated: integrated_lufs(&block_energy),
        momentary_max: momentary_max_lufs(&block_energy),
        short_term_max: short_term_max_lufs(&block_energy),
    }
}

/// K-weight the stereo input, slice into overlapping 400 ms blocks, and emit
/// each block's combined L+R mean-square energy. Loudness for any block is
/// then `energy_to_lufs(energy)`. Storing energy (not loudness) lets the
/// short-term sliding window sum directly without a dB→linear round-trip.
fn k_weighted_block_energies(
    stereo: &[f32],
    n_frames: usize,
    block_samples: usize,
    hop_samples: usize,
) -> Vec<f64> {
    let mut left = Vec::with_capacity(n_frames);
    let mut right = Vec::with_capacity(n_frames);
    for frame in stereo.chunks_exact(2) {
        left.push(frame[0]);
        right.push(frame[1]);
    }
    k_weight_inplace(&mut left);
    k_weight_inplace(&mut right);

    let est_blocks = n_frames.saturating_sub(block_samples) / hop_samples + 1;
    let mut block_energy: Vec<f64> = Vec::with_capacity(est_blocks);
    let mut start = 0usize;
    while start + block_samples <= n_frames {
        let mut sum_l = 0.0_f64;
        let mut sum_r = 0.0_f64;
        for i in start..(start + block_samples) {
            sum_l += f64::from(left[i]) * f64::from(left[i]);
            sum_r += f64::from(right[i]) * f64::from(right[i]);
        }
        block_energy.push((sum_l + sum_r) / block_samples as f64);
        start += hop_samples;
    }
    block_energy
}

/// LUFS = -0.691 + 10·log₁₀(energy). Returns `SILENT_FLOOR_DBFS` for ≤ 0 input.
#[inline]
fn energy_to_lufs(energy: f64) -> f32 {
    if energy > 0.0 {
        (-0.691 + 10.0 * energy.log10()) as f32
    } else {
        SILENT_FLOOR_DBFS
    }
}

fn momentary_max_lufs(block_energy: &[f64]) -> f32 {
    let max_energy = block_energy.iter().copied().fold(0.0_f64, f64::max);
    energy_to_lufs(max_energy)
}

/// 3 s sliding window stepped every 100 ms (= 30 consecutive 400 ms blocks at
/// 75 % overlap). Buffers shorter than 30 blocks report -200 LUFS.
fn short_term_max_lufs(block_energy: &[f64]) -> f32 {
    const SHORT_TERM_BLOCKS: usize = 30;
    if block_energy.len() < SHORT_TERM_BLOCKS {
        return SILENT_FLOOR_DBFS;
    }
    let inv = 1.0 / SHORT_TERM_BLOCKS as f64;
    let mut running: f64 = block_energy[..SHORT_TERM_BLOCKS].iter().sum();
    let mut best = running * inv;
    for i in SHORT_TERM_BLOCKS..block_energy.len() {
        running += block_energy[i] - block_energy[i - SHORT_TERM_BLOCKS];
        let mean = running * inv;
        if mean > best {
            best = mean;
        }
    }
    energy_to_lufs(best)
}

/// ITU-R BS.1770-4 integrated loudness: absolute -70 LUFS gate, then a
/// relative (mean − 10 LU) gate, applied to per-block energies.
fn integrated_lufs(block_energy: &[f64]) -> f32 {
    let abs_gate_energy = 10f64.powf((-70.0 + 0.691) / 10.0);
    let gated_abs: Vec<f64> = block_energy
        .iter()
        .copied()
        .filter(|e| *e > abs_gate_energy)
        .collect();
    if gated_abs.is_empty() {
        return SILENT_FLOOR_DBFS;
    }
    let mean_energy_abs = gated_abs.iter().sum::<f64>() / gated_abs.len() as f64;
    let rel_gate_energy = mean_energy_abs * 10f64.powf(-1.0); // -10 LU == ×0.1 energy

    let mut rel_sum = 0.0_f64;
    let mut rel_count = 0usize;
    for &e in &gated_abs {
        if e > rel_gate_energy {
            rel_sum += e;
            rel_count += 1;
        }
    }
    if rel_count == 0 {
        SILENT_FLOOR_DBFS
    } else {
        energy_to_lufs(rel_sum / rel_count as f64)
    }
}

// ---------------------------------------------------------------------------
// True peak (inter-sample peak) via 4× polyphase oversampling
// ---------------------------------------------------------------------------

/// Oversample factor used for true-peak detection per ITU-R BS.1770-4 Annex 2
/// (minimum 4× — this is the spec minimum that catches all but the most
/// pathological inter-sample peaks).
const TRUE_PEAK_OVERSAMPLE: usize = 4;

/// Taps per polyphase phase. 12 × 4 = 48-tap prototype filter — ITU-R BS.1770-4
/// Annex 2 specifies a 47-tap minimum.
const TRUE_PEAK_TAPS_PER_PHASE: usize = 12;

/// Total prototype-filter length (= phases × taps per phase).
const TRUE_PEAK_PROTOTYPE_LEN: usize = TRUE_PEAK_OVERSAMPLE * TRUE_PEAK_TAPS_PER_PHASE;

/// Polyphase coefficients indexed `[phase][tap]`. Lazily initialised because
/// const-fn floating-point math is still gated on stable Rust.
fn true_peak_kernel() -> &'static [[f32; TRUE_PEAK_TAPS_PER_PHASE]; TRUE_PEAK_OVERSAMPLE] {
    static KERNEL: OnceLock<[[f32; TRUE_PEAK_TAPS_PER_PHASE]; TRUE_PEAK_OVERSAMPLE]> =
        OnceLock::new();
    KERNEL.get_or_init(build_true_peak_kernel)
}

/// Build a 48-tap Hamming-windowed sinc lowpass at fc = 1/(2·oversample)
/// cycles per upsampled sample, then de-interleave into 4 polyphase phases.
///
/// Coefficients per phase are normalised so summing one phase against a DC
/// input yields 1.0 — the upsampler then preserves DC level and the existing
/// `peak`/`true_peak` comparison stays meaningful.
fn build_true_peak_kernel() -> [[f32; TRUE_PEAK_TAPS_PER_PHASE]; TRUE_PEAK_OVERSAMPLE] {
    use std::f64::consts::PI;
    let n = TRUE_PEAK_PROTOTYPE_LEN;
    let mut proto = [0.0_f64; TRUE_PEAK_PROTOTYPE_LEN];
    let center = (n as f64 - 1.0) * 0.5;
    let fc = 0.5 / TRUE_PEAK_OVERSAMPLE as f64;
    for (i, slot) in proto.iter_mut().enumerate() {
        let m = i as f64 - center;
        let sinc = if m.abs() < 1e-12 {
            2.0 * fc
        } else {
            (2.0 * PI * fc * m).sin() / (PI * m)
        };
        // Hamming window.
        let w = 0.54 - 0.46 * (2.0 * PI * i as f64 / (n as f64 - 1.0)).cos();
        *slot = sinc * w;
    }
    // De-interleave into polyphase phases and normalise each phase so DC is
    // preserved (sum of each phase's taps == 1.0).
    let mut phases = [[0.0_f32; TRUE_PEAK_TAPS_PER_PHASE]; TRUE_PEAK_OVERSAMPLE];
    for phase in 0..TRUE_PEAK_OVERSAMPLE {
        let mut sum = 0.0_f64;
        for tap in 0..TRUE_PEAK_TAPS_PER_PHASE {
            let proto_idx = tap * TRUE_PEAK_OVERSAMPLE + phase;
            sum += proto[proto_idx];
        }
        let inv = if sum.abs() > 1e-12 { 1.0 / sum } else { 1.0 };
        for tap in 0..TRUE_PEAK_TAPS_PER_PHASE {
            let proto_idx = tap * TRUE_PEAK_OVERSAMPLE + phase;
            phases[phase][tap] = (proto[proto_idx] * inv) as f32;
        }
    }
    phases
}

/// Stereo true-peak: take the larger of the two per-channel true peaks.
fn compute_true_peak_stereo(stereo: &[f32]) -> f32 {
    if stereo.is_empty() {
        return 0.0;
    }
    let n_frames = stereo.len() / 2;
    if n_frames < TRUE_PEAK_TAPS_PER_PHASE {
        // Buffer too short to give the FIR meaningful state; fall back to
        // the sample-grid peak. Better than returning 0 or a garbage value.
        return peak_amplitude(stereo);
    }
    let mut left = Vec::with_capacity(n_frames);
    let mut right = Vec::with_capacity(n_frames);
    for frame in stereo.chunks_exact(2) {
        left.push(frame[0]);
        right.push(frame[1]);
    }
    let lp = channel_true_peak(&left);
    let rp = channel_true_peak(&right);
    lp.max(rp)
}

/// Single-channel true peak: 4× upsample via polyphase FIR and take the max
/// absolute value across both the original samples and the interpolated ones.
fn channel_true_peak(samples: &[f32]) -> f32 {
    let kernel = true_peak_kernel();
    let taps = TRUE_PEAK_TAPS_PER_PHASE;
    let mut peak = peak_amplitude(samples);
    if samples.len() < taps {
        return peak;
    }
    for k in (taps - 1)..samples.len() {
        for coeffs in kernel.iter() {
            let mut acc = 0.0_f32;
            for tap in 0..taps {
                acc += coeffs[tap] * samples[k - tap];
            }
            let a = acc.abs();
            if a > peak {
                peak = a;
            }
        }
    }
    peak
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine_stereo(freq: f32, sample_rate: u32, seconds: f32, amplitude: f32) -> Vec<f32> {
        let n = (seconds * sample_rate as f32) as usize;
        let mut buf = Vec::with_capacity(n * 2);
        for i in 0..n {
            let phase = TAU * freq * (i as f32) / (sample_rate as f32);
            let v = amplitude * phase.sin();
            buf.push(v);
            buf.push(v);
        }
        buf
    }

    #[test]
    fn empty_buffer_returns_zero_analysis() {
        let result = analyze_mix_buffer(&[], 44_100);
        assert_eq!(result.peak, 0.0);
        assert_eq!(result.rms, 0.0);
        assert_eq!(result.lufs_integrated, -200.0);
    }

    #[test]
    fn unity_sine_peak_and_rms() {
        let buf = sine_stereo(1000.0, 44_100, 2.0, 1.0);
        let result = analyze_mix_buffer(&buf, 44_100);
        assert!((result.peak - 1.0).abs() < 0.001);
        // Sine RMS = amplitude / sqrt(2) ≈ 0.707
        assert!((result.rms - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.005);
        // RMS in dB ≈ -3 dBFS
        assert!((result.rms_dbfs + 3.0).abs() < 0.1);
    }

    #[test]
    fn mono_signal_has_max_mono_compat_and_high_correlation() {
        let buf = sine_stereo(1000.0, 44_100, 1.0, 0.5);
        let result = analyze_mix_buffer(&buf, 44_100);
        assert!(
            result.mono_compat > 0.99,
            "mono_compat = {}",
            result.mono_compat
        );
        assert!(
            result.stereo_correlation > 0.99,
            "correlation = {}",
            result.stereo_correlation
        );
        assert!(result.side_rms < 0.001, "side_rms = {}", result.side_rms);
    }

    #[test]
    fn anti_phase_signal_has_near_zero_mono_compat() {
        let n = 44_100 * 2;
        let mut buf = Vec::with_capacity(n * 2);
        for i in 0..n {
            let phase = TAU * 1000.0 * (i as f32) / 44_100.0;
            let v = 0.5 * phase.sin();
            buf.push(v);
            buf.push(-v);
        }
        let result = analyze_mix_buffer(&buf, 44_100);
        assert!(
            result.mono_compat < 0.01,
            "mono_compat = {}",
            result.mono_compat
        );
        assert!(
            result.stereo_correlation < -0.99,
            "correlation = {}",
            result.stereo_correlation
        );
    }

    #[test]
    fn lufs_integrated_returns_finite_value_for_audible_signal() {
        let buf = sine_stereo(1000.0, 44_100, 2.0, 0.5);
        let result = analyze_mix_buffer(&buf, 44_100);
        // 1 kHz sine at -6 dBFS through K-weighting (+4 dB shelf above 1 kHz) ≈
        // -18..-15 LUFS. Loose bounds — the exact value depends on which side
        // of the 1.5 kHz centre frequency the shelf shapes it. Just make sure
        // it's in a plausible range and not -200.
        assert!(
            result.lufs_integrated > -30.0 && result.lufs_integrated < 0.0,
            "lufs_integrated = {}",
            result.lufs_integrated
        );
    }

    #[test]
    fn lufs_returns_silent_floor_for_short_buffer() {
        // Less than one 400 ms block.
        let buf = sine_stereo(1000.0, 44_100, 0.1, 0.5);
        let result = analyze_mix_buffer(&buf, 44_100);
        assert_eq!(result.lufs_integrated, -200.0);
    }

    #[test]
    fn crest_factor_zero_for_dc() {
        // DC signal — peak == rms, crest == 0
        let buf = vec![0.5f32; 44_100 * 2];
        let result = analyze_mix_buffer(&buf, 44_100);
        assert!(result.crest_factor_db < 0.1);
    }

    #[test]
    fn clipping_is_counted() {
        let mut buf = vec![0.0f32; 4 * 2];
        // Two clipping samples on the left channel.
        buf[0] = 1.0;
        buf[2] = -1.0;
        let result = analyze_mix_buffer(&buf, 44_100);
        assert_eq!(result.clipped_samples, 2);
    }

    #[test]
    fn true_peak_is_at_least_sample_peak() {
        let buf = sine_stereo(1000.0, 44_100, 1.0, 0.8);
        let result = analyze_mix_buffer(&buf, 44_100);
        assert!(
            result.true_peak >= result.peak - 1e-4,
            "true_peak ({}) must be ≥ sample peak ({})",
            result.true_peak,
            result.peak
        );
        // For a moderate-amplitude in-band sine, true_peak should still be
        // close to the sample peak — within ~1 dB.
        assert!(
            result.true_peak <= result.peak * 1.2,
            "true_peak ({}) wildly above sample peak ({}) — kernel may be misnormalised",
            result.true_peak,
            result.peak
        );
        assert!(result.true_peak_dbtp > -10.0);
    }

    #[test]
    fn true_peak_exceeds_sample_peak_for_intersample_overshoot() {
        // Intersample-peak test signal: a 11025 Hz sine (fs/4 at fs = 44.1 kHz)
        // sampled at quadrature phases (π/4, 3π/4, …). Every sample lands at
        // ±sin(π/4) ≈ ±0.7071, but the reconstructed waveform's true peak is
        // the underlying amplitude A. With A = 1.0 we expect sample peak ≈
        // 0.7071 and true_peak ≥ ~0.99 once the FIR has settled.
        let n = 44_100;
        let mut buf = Vec::with_capacity(n * 2);
        let phase_offset = std::f32::consts::PI * 0.25;
        for i in 0..n {
            let phase = TAU * 11025.0 * (i as f32) / 44_100.0 + phase_offset;
            let v = phase.sin();
            buf.push(v);
            buf.push(v);
        }
        let result = analyze_mix_buffer(&buf, 44_100);
        let expected_sample = std::f32::consts::FRAC_1_SQRT_2; // ≈ 0.7071
        assert!(
            (result.peak - expected_sample).abs() < 0.01,
            "sample peak should be ≈ {}, got {}",
            expected_sample,
            result.peak
        );
        assert!(
            result.true_peak > result.peak + 0.1,
            "true_peak ({}) should clearly overshoot sample peak ({}) for this quadrature-phase signal",
            result.true_peak,
            result.peak
        );
    }

    #[test]
    fn lufs_short_term_max_meets_short_term_threshold() {
        // 4 s of audible content is comfortably above the 3.3 s short-term
        // window minimum.
        let buf = sine_stereo(1000.0, 44_100, 4.0, 0.5);
        let result = analyze_mix_buffer(&buf, 44_100);
        assert!(
            result.lufs_short_term_max > -30.0 && result.lufs_short_term_max < 0.0,
            "lufs_short_term_max = {}",
            result.lufs_short_term_max
        );
        // Short-term and momentary should be close for stationary content.
        // (LUFS-S averages over 3 s, LUFS-M over 400 ms — both readouts of
        // the same stationary sine should land in the same neighbourhood.)
        assert!(
            (result.lufs_short_term_max - result.lufs_momentary_max).abs() < 1.0,
            "stationary signal should give close LUFS-S ({}) and LUFS-M ({})",
            result.lufs_short_term_max,
            result.lufs_momentary_max
        );
    }

    #[test]
    fn lufs_short_term_returns_silent_floor_below_three_seconds() {
        // 2 s — long enough for LUFS-I/M, too short for LUFS-S.
        let buf = sine_stereo(1000.0, 44_100, 2.0, 0.5);
        let result = analyze_mix_buffer(&buf, 44_100);
        assert!(result.lufs_momentary_max > -30.0);
        assert_eq!(result.lufs_short_term_max, -200.0);
    }
}
