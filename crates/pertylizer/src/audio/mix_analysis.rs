//! Mix-bus audio analysis primitives.
//!
//! Takes a stereo-interleaved f32 buffer (the master-bus output of an offline
//! arrangement render) and returns metrics relevant for judging a mix as a
//! whole rather than a single sound:
//!
//! - Peak / RMS / crest factor.
//! - **Integrated LUFS** following ITU-R BS.1770-4 (K-weighting + 400 ms
//!   gating blocks at 75% overlap, absolute -70 LUFS gate, relative -10 LU
//!   gate).
//! - 4-band frequency balance (delegates to `analysis::energy_bands` on a
//!   mono mix-down).
//! - Stereo correlation (delegates to `analysis::stereo_correlation`).
//! - Mid/side RMS split.
//! - Mono-compatibility score (how much energy survives an L+R sum).
//!
//! All analysis is offline and deterministic. The K-weighting filter
//! coefficients are hard-wired for 44.1 kHz, which matches the offline render
//! path. Other sample rates produce slightly biased LUFS readings (typically
//! within ~0.5 dB) but the rest of the metrics are sample-rate independent.

use crate::audio::analysis::{
    EnergyBands, energy_bands, peak_amplitude, rms_overall, stereo_correlation,
};

/// Output of [`analyze_mix_buffer`]. All dB fields are dBFS unless noted.
#[derive(Debug, Clone, Copy)]
pub struct MixAnalysis {
    /// Sample peak across both channels, linear (0.0..=1.0+).
    pub peak: f32,
    /// Sample peak in dBFS. `-inf` is reported as -200.0.
    pub peak_dbfs: f32,
    /// Overall RMS, linear (0.0..=1.0+).
    pub rms: f32,
    /// Overall RMS in dBFS. `-inf` reported as -200.0.
    pub rms_dbfs: f32,
    /// Crest factor = peak_dBFS - rms_dBFS. Higher = more dynamic.
    pub crest_factor_db: f32,
    /// Integrated loudness (ITU-R BS.1770-4 LUFS). `-inf` reported as -200.0.
    pub lufs_integrated: f32,
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

    // Mono mix-down for band analysis (preserve the L+R energy without
    // dividing by 2 so banded energy stays comparable to RMS).
    let mut mono = Vec::with_capacity(n_frames);
    let mut mid = Vec::with_capacity(n_frames);
    let mut side = Vec::with_capacity(n_frames);
    let mut left_sum_sq = 0.0_f64;
    let mut right_sum_sq = 0.0_f64;
    let mut clipped = 0u32;
    for frame in stereo.chunks_exact(2) {
        let l = frame[0];
        let r = frame[1];
        if l.abs() >= 0.999 {
            clipped += 1;
        }
        if r.abs() >= 0.999 {
            clipped += 1;
        }
        mono.push((l + r) * 0.5);
        mid.push((l + r) * 0.5);
        side.push((l - r) * 0.5);
        left_sum_sq += f64::from(l) * f64::from(l);
        right_sum_sq += f64::from(r) * f64::from(r);
    }
    let bands = energy_bands(&mono, sample_rate);
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

    let lufs = lufs_integrated(stereo, sample_rate);

    MixAnalysis {
        peak,
        peak_dbfs,
        rms,
        rms_dbfs,
        crest_factor_db,
        lufs_integrated: lufs,
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
        peak_dbfs: -200.0,
        rms: 0.0,
        rms_dbfs: -200.0,
        crest_factor_db: 0.0,
        lufs_integrated: -200.0,
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

#[inline]
fn lin_to_db(linear: f32) -> f32 {
    if linear > 0.0 {
        20.0 * linear.log10()
    } else {
        -200.0
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

/// Integrated loudness (LUFS) per ITU-R BS.1770-4.
///
/// Stereo (channel weights 1.0, 1.0). For short inputs (less than one 400 ms
/// block) returns `-200.0` rather than `-inf`/`NaN`.
fn lufs_integrated(stereo: &[f32], sample_rate: u32) -> f32 {
    let n_frames = stereo.len() / 2;
    if n_frames == 0 || sample_rate == 0 {
        return -200.0;
    }
    let block_samples = (sample_rate as usize * 4) / 10; // 400 ms
    let hop_samples = block_samples / 4; // 75% overlap → 100 ms hop
    if n_frames < block_samples {
        return -200.0;
    }

    // Deinterleave to per-channel buffers so we can filter independently.
    let mut left = Vec::with_capacity(n_frames);
    let mut right = Vec::with_capacity(n_frames);
    for frame in stereo.chunks_exact(2) {
        left.push(frame[0]);
        right.push(frame[1]);
    }
    k_weight_inplace(&mut left);
    k_weight_inplace(&mut right);

    // Mean-square per 400 ms block, 75 % overlap. Block loudness:
    //     L_K = -0.691 + 10 log10( Σ_ch weight_ch · MS_ch )
    // For stereo, weight_L = weight_R = 1.0.
    let mut block_loudness: Vec<f32> = Vec::new();
    let mut start = 0usize;
    while start + block_samples <= n_frames {
        let end = start + block_samples;
        let mut sum_l = 0.0_f64;
        let mut sum_r = 0.0_f64;
        for i in start..end {
            sum_l += f64::from(left[i]) * f64::from(left[i]);
            sum_r += f64::from(right[i]) * f64::from(right[i]);
        }
        let ms_l = sum_l / block_samples as f64;
        let ms_r = sum_r / block_samples as f64;
        let weighted = ms_l + ms_r;
        if weighted > 0.0 {
            let lk = -0.691 + 10.0 * weighted.log10();
            block_loudness.push(lk as f32);
        } else {
            block_loudness.push(-200.0);
        }
        start += hop_samples;
    }

    if block_loudness.is_empty() {
        return -200.0;
    }

    // Absolute gate: drop blocks below -70 LUFS.
    let gated_abs: Vec<f32> = block_loudness
        .iter()
        .copied()
        .filter(|l| *l > -70.0)
        .collect();
    if gated_abs.is_empty() {
        return -200.0;
    }

    // Compute mean energy of absolute-gated blocks then derive the relative
    // gate threshold = mean_loudness - 10 LU.
    let mean_energy: f64 = gated_abs
        .iter()
        .map(|l| 10f64.powf((f64::from(*l) + 0.691) / 10.0))
        .sum::<f64>()
        / gated_abs.len() as f64;
    let mean_loudness_lufs = -0.691 + 10.0 * mean_energy.log10();
    let relative_threshold = mean_loudness_lufs - 10.0;

    let gated_rel: Vec<f32> = gated_abs
        .into_iter()
        .filter(|l| f64::from(*l) > relative_threshold)
        .collect();
    if gated_rel.is_empty() {
        return -200.0;
    }

    let final_energy: f64 = gated_rel
        .iter()
        .map(|l| 10f64.powf((f64::from(*l) + 0.691) / 10.0))
        .sum::<f64>()
        / gated_rel.len() as f64;
    let lufs_i = -0.691 + 10.0 * final_energy.log10();
    lufs_i as f32
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
}
