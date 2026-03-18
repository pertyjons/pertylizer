//! Shared mathematical functions for DSP modules.
//!
//! Common audio math operations extracted from individual modules
//! to reduce duplication and ensure consistency.

use std::f32::consts::{FRAC_PI_2, TAU};

use synth_core::{Hertz, SampleRate};

// ── Waveshaping / Saturation ────────────────────────────────────────────────

/// Soft-clip a signal using `tanh` saturation.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    x.tanh()
}

/// Hard-clip a signal to the \[−1, +1\] range.
#[inline]
pub fn hard_clip(x: f32) -> f32 {
    x.clamp(-1.0, 1.0)
}

/// Foldback distortion: reflect the signal at ±`threshold`.
///
/// Iterates up to 16 times (real-time safe) to fold extreme values back
/// into the \[−threshold, +threshold\] range.
#[inline]
pub fn foldback(x: f32, threshold: f32) -> f32 {
    let mut v = x;
    for _ in 0..16 {
        if v > threshold {
            v = threshold - (v - threshold);
        } else if v < -threshold {
            v = -threshold - (v + threshold);
        } else {
            break;
        }
    }
    v.clamp(-threshold, threshold)
}

/// Tube-style asymmetric soft clipping.
///
/// Positive half uses `1 − e^(−x)`, negative half uses `−(1 − e^x)`.
#[inline]
pub fn tube_saturate(x: f32) -> f32 {
    if x >= 0.0 {
        1.0 - (-x).exp()
    } else {
        -1.0 + x.exp()
    }
}

// ── Drive / Gain mapping ────────────────────────────────────────────────────

/// Map a normalized drive value (0–1) to a gain multiplier using a quadratic
/// curve: `1 + d² × scale`.
///
/// Typical `scale` values: 19.0 (waveshaper), 50.0 (distortion).
#[inline]
pub fn drive_gain(drive: f32, scale: f32) -> f32 {
    1.0 + drive * drive * scale
}

// ── Stereo panning ──────────────────────────────────────────────────────────

/// Equal-power (constant-power) stereo panning.
///
/// `pan` is bipolar: −1 = full left, 0 = center, +1 = full right.
/// Returns `(left_gain, right_gain)`.
#[inline]
pub fn equal_power_pan(pan: f32) -> (f32, f32) {
    let angle = (pan + 1.0) * 0.5 * FRAC_PI_2;
    (angle.cos(), angle.sin())
}

// ── Biquad helpers ──────────────────────────────────────────────────────────

/// Pre-computed angular frequency components for biquad filter design.
pub struct BiquadOmega {
    pub cos_w0: f32,
    pub sin_w0: f32,
}

/// Compute the angular frequency components (`cos ω₀`, `sin ω₀`) for a biquad
/// filter at the given frequency and sample rate.
///
/// Clamps the frequency to 0.49 × Nyquist for numerical safety.
#[inline]
pub fn biquad_omega(freq: Hertz, sample_rate: SampleRate) -> BiquadOmega {
    let sr = sample_rate.as_f32();
    let clamped = freq.as_f32().min(sr * 0.49);
    let w0 = TAU * clamped / sr;
    BiquadOmega {
        cos_w0: w0.cos(),
        sin_w0: w0.sin(),
    }
}

/// Compute the `alpha` parameter for a peaking/bandpass biquad from `sin ω₀`
/// and Q factor.
#[inline]
pub fn biquad_alpha_from_q(sin_w0: f32, q: f32) -> f32 {
    sin_w0 / (2.0 * q.max(0.01))
}

/// Compute the `alpha` parameter for a shelving biquad from `sin ω₀`, gain `a`,
/// and shelf slope `s`.
#[inline]
pub fn biquad_alpha_shelf(sin_w0: f32, a: f32, slope: f32) -> f32 {
    sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0).sqrt()
}

// ── dB / amplitude ──────────────────────────────────────────────────────────

/// Convert a decibel gain value to a linear amplitude factor suitable for
/// biquad EQ shelving filters.
///
/// Uses the Audio EQ Cookbook formula: `10^(dB / 40)`.
#[inline]
pub fn db_to_eq_amplitude(db: f32) -> f32 {
    10.0_f32.powf(db / 40.0)
}

// ── Windowing ───────────────────────────────────────────────────────────────

/// Hann window function.
///
/// `phase` is in the range \[0, 1\].  Returns 0 at the edges and 1 at the
/// centre.
#[inline]
pub fn hann_window(phase: f32) -> f32 {
    0.5 * (1.0 - (phase * TAU).cos())
}

// ── Normalization ───────────────────────────────────────────────────────────

/// Normalization gain for additive synthesis: `1 / √n`.
///
/// Keeps total power roughly constant when summing `n` equal-amplitude voices.
#[inline]
pub fn normalization_gain(voice_count: usize) -> f32 {
    1.0 / (voice_count as f32).sqrt()
}