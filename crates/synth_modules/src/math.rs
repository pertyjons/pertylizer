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

// ── Edge detection ─────────────────────────────────────────────────────────

/// Detect a rising edge: `current` crossed above 0.5 while `previous` was at
/// or below 0.5.  Used for gate/trigger detection throughout the module graph.
#[inline]
pub fn rising_edge(current: f32, previous: f32) -> bool {
    current > 0.5 && previous <= 0.5
}

// ── Timing ─────────────────────────────────────────────────────────────────

/// Samples per 16th-note step at the given BPM and sample rate.
///
/// `bpm` is clamped to a minimum of 20 to avoid division by zero.
#[inline]
pub fn samples_per_16th(sample_rate_hz: f32, bpm: f32) -> f32 {
    sample_rate_hz * 60.0 / bpm.max(20.0) / 4.0
}

// ── PRNG ───────────────────────────────────────────────────────────────────

/// Advance a xorshift32 state and return a value in \[0, 1).
///
/// RT-safe: no branches, no allocation.
#[inline]
pub fn xorshift32(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state as f32) / (u32::MAX as f32)
}

// ── Pitch / frequency helpers ──────────────────────────────────────────────

/// Convert a cent offset to a frequency ratio: `2^(cents / 1200)`.
#[inline]
pub fn cents_to_ratio(cents: f32) -> f32 {
    (cents / 1200.0).exp2()
}

/// Convert a semitone offset to a frequency ratio: `2^(semitones / 12)`.
#[inline]
pub fn semitones_to_ratio(semitones: f32) -> f32 {
    (semitones / 12.0).exp2()
}

/// Convert a frequency (Hz) to 1 V/octave CV relative to C4 (261.63 Hz).
///
/// Returns 0 for frequencies ≤ 0.
#[inline]
pub fn freq_to_cv(freq_hz: f32) -> f32 {
    if freq_hz <= 0.0 {
        return 0.0;
    }
    (freq_hz / 261.63).log2()
}

// ── Interpolation ──────────────────────────────────────────────────────────

/// Linear interpolation between `a` and `b` by factor `t` (0 = a, 1 = b).
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Dry/wet linear crossfade: `dry * (1 - mix) + wet * mix`.
#[inline]
pub fn linear_mix(dry: f32, wet: f32, mix: f32) -> f32 {
    dry * (1.0 - mix) + wet * mix
}

/// One-pole parameter smoother.
///
/// `smoothing` in \[0, 1\]: 0 = instant, 1 = frozen.
#[inline]
pub fn smooth_value(current: f32, target: f32, smoothing: f32) -> f32 {
    current + (1.0 - smoothing) * (target - current)
}

/// Parabolic interpolation for sub-sample peak refinement.
///
/// Given three equally-spaced samples (`prev`, `curr`, `next`), returns the
/// fractional offset of the true peak relative to `curr`.
#[inline]
pub fn parabolic_interpolation(prev: f32, curr: f32, next: f32) -> f32 {
    let denom = prev - 2.0 * curr + next;
    if denom.abs() > 1e-10 {
        0.5 * (prev - next) / denom
    } else {
        0.0
    }
}

// ── Curve shaping ──────────────────────────────────────────────────────────

/// Apply curve shaping to an exponential envelope coefficient.
///
/// `curve` in \[−1, +1\]: negative = slower start (raise coeff),
/// positive = faster start (lower coeff), 0 = linear.
#[inline]
pub fn apply_curve_shaping(base_coef: f32, curve: f32) -> f32 {
    if curve.abs() < 0.01 {
        base_coef
    } else if curve < 0.0 {
        base_coef.powf(1.0 + (-curve) * 3.0)
    } else {
        base_coef.powf(1.0 / (1.0 + curve * 3.0))
    }
}

/// Interpolate between `from` and `to` with a shaped time parameter.
///
/// `t` in \[0, 1\], `curve` in \[−1, +1\]:
/// - 0 = linear
/// - positive = exponential (slow start, fast end)
/// - negative = logarithmic (fast start, slow end)
#[inline]
pub fn interpolate_with_curve(from: f32, to: f32, t: f32, curve: f32) -> f32 {
    let t_clamped = t.clamp(0.0, 1.0);
    let shaped_t = if curve.abs() < 0.01 {
        t_clamped
    } else if curve > 0.0 {
        t_clamped.powf(1.0 + curve * 4.0)
    } else {
        1.0 - (1.0 - t_clamped).powf(1.0 + (-curve) * 4.0)
    };
    from + (to - from) * shaped_t
}

/// Bipolar curve shaping for values in \[−1, +1\].
///
/// `curve` > 0: exponential (more center, less extremes).
/// `curve` < 0: logarithmic (less center, more extremes).
/// `curve` ≈ 0: linear pass-through.
#[inline]
pub fn bipolar_curve(value: f32, curve: f32) -> f32 {
    let safe_curve = curve.clamp(-0.99, 0.99);
    if safe_curve.abs() > 0.01 {
        if safe_curve > 0.0 {
            value.signum() * value.abs().powf(1.0 + safe_curve)
        } else {
            value.signum() * value.abs().powf(1.0 / (1.0 - safe_curve))
        }
    } else {
        value
    }
}

// ── Waveforms ──────────────────────────────────────────────────────────────

/// Band-limited triangle wave from a normalized phase in \[0, 1\].
///
/// Output range is \[−1, +1\].
#[inline]
pub fn triangle_wave(phase: f32) -> f32 {
    if phase < 0.5 {
        phase * 4.0 - 1.0
    } else {
        3.0 - phase * 4.0
    }
}

// ── Fast approximations ───────────────────────────────────────────────────

/// Fast sine approximation from a phase value in turns (fractions of a full
/// rotation).  Computes `sin(2π × turns)` using a corrected parabolic
/// approximation.
///
/// Max error ≈ 0.001 (−60 dB) — inaudible for oscillator use.
/// Approximately 4× faster than `(turns * TAU).sin()`.
///
/// `turns` can be any float; it is automatically wrapped to \[0, 1).
#[inline]
pub fn fast_sin_turns(turns: f32) -> f32 {
    // Wrap to [0, 1) — floor is a single instruction on x86 (vroundss)
    let phase = turns - turns.floor();
    // Map to [-0.5, 0.5] centered at zero crossing
    let x = if phase < 0.5 { phase } else { phase - 1.0 };
    // Parabolic approximation: exact at 0, ±0.25, ±0.5
    let y = 16.0 * x * (0.5 - x.abs());
    // Correction term: reduces max error from ~0.056 to ~0.001
    y * (0.225 * y.abs() + 0.775)
}

// ── Additional window functions ────────────────────────────────────────────

/// Gaussian window function.
///
/// `phase` in \[0, 1\], `sigma` controls width (typical: 0.4).
#[inline]
pub fn gaussian_window(phase: f32, sigma: f32) -> f32 {
    let x = (phase - 0.5) / sigma;
    (-0.5 * x * x).exp()
}

/// Trapezoid window function.
///
/// `phase` in \[0, 1\], `ramp` is the fraction of the window used for
/// fade-in/fade-out (typical: 0.1).
#[inline]
pub fn trapezoid_window(phase: f32, ramp: f32) -> f32 {
    let ramp = ramp.max(f32::EPSILON);
    if phase < ramp {
        phase / ramp
    } else if phase > 1.0 - ramp {
        (1.0 - phase) / ramp
    } else {
        1.0
    }
}

// ── Mid/Side stereo ────────────────────────────────────────────────────────

/// Encode a stereo pair into mid/side representation.
///
/// Returns `(mid, side)` where `mid = (L + R) / 2`, `side = (L − R) / 2`.
#[inline]
pub fn mid_side_encode(left: f32, right: f32) -> (f32, f32) {
    ((left + right) * 0.5, (left - right) * 0.5)
}

/// Decode mid/side back to stereo with a width parameter.
///
/// `width` = 1.0 for normal stereo, 0.0 for mono, 2.0 for extra-wide.
/// Returns `(left, right)`.
#[inline]
pub fn mid_side_decode(mid: f32, side: f32, width: f32) -> (f32, f32) {
    (mid + side * width, mid - side * width)
}

// ── Quantization / bit reduction ───────────────────────────────────────────

/// Bit-crush a signal by quantizing to `2^bit_depth` levels.
///
/// `bit_depth` is clamped to ≥ 1.  Output is clamped to \[−1, +1\].
#[inline]
pub fn bitcrush(sample: f32, bit_depth: f32) -> f32 {
    let levels = 2.0f32.powf(bit_depth.max(1.0));
    (sample * levels).round() / levels
}

/// Quantize a signal to a fixed number of levels.
///
/// Output is clamped to \[−1, +1\].
#[inline]
pub fn quantize_signal(sample: f32, levels: f32) -> f32 {
    let l = levels.max(2.0);
    ((sample * l).round() / l).clamp(-1.0, 1.0)
}

// ── Chebyshev polynomials ──────────────────────────────────────────────────

/// 2nd-order Chebyshev polynomial: `T₂(x) = 2x² − 1`.
///
/// Generates 2nd-harmonic content.
#[inline]
pub fn chebyshev_t2(x: f32) -> f32 {
    2.0 * x * x - 1.0
}

/// 3rd-order Chebyshev polynomial: `T₃(x) = 4x³ − 3x`.
///
/// Generates 3rd-harmonic content.
#[inline]
pub fn chebyshev_t3(x: f32) -> f32 {
    4.0 * x * x * x - 3.0 * x
}

// ── Additional saturation ──────────────────────────────────────────────────

/// Asymmetric tanh soft clipping.
///
/// `symmetry` in \[−1, +1\]: 0 = equal, +1 = harder positive, −1 = harder negative.
#[inline]
pub fn asymmetric_soft_clip(x: f32, symmetry: f32) -> f32 {
    let pos_drive = 1.0 + (1.0 + symmetry) * 0.5;
    let neg_drive = 1.0 + (1.0 - symmetry) * 0.5;
    if x >= 0.0 {
        (x * pos_drive).tanh()
    } else {
        (x * neg_drive).tanh()
    }
}

/// Soft-knee limiter using tanh saturation above `threshold`.
///
/// Below threshold, signal passes through unchanged. Above, excess is
/// compressed via `tanh` into the remaining headroom.
#[inline]
pub fn soft_knee_limit(sample: f32, threshold: f32) -> f32 {
    if sample.abs() > threshold {
        let sign = sample.signum();
        let excess = sample.abs() - threshold;
        sign * (threshold + (1.0 - threshold) * (excess / (1.0 + excess)).tanh())
    } else {
        sample
    }
}

// ── Velocity / dynamics ────────────────────────────────────────────────────

/// Apply velocity sensitivity scaling.
///
/// When `sensitivity` = 0, output = 1.0 (no velocity response).
/// When `sensitivity` = 1, output = `velocity`.
#[inline]
pub fn velocity_sensitivity(velocity: f32, sensitivity: f32) -> f32 {
    1.0 - sensitivity * (1.0 - velocity)
}

// ── Envelope coefficients ──────────────────────────────────────────────────

/// Compute an exponential envelope coefficient from time in seconds.
///
/// Used for attack/release in compressors, limiters, and envelope followers.
/// Returns `exp(−1 / (time_secs × sample_rate_hz))`.
#[inline]
pub fn envelope_coeff(time_secs: f32, sample_rate_hz: f32) -> f32 {
    (-1.0 / (time_secs * sample_rate_hz).max(1.0)).exp()
}

// ── Bipolar / unipolar conversion ──────────────────────────────────────────

/// Convert a bipolar value \[−1, +1\] to unipolar \[0, 1\].
#[inline]
pub fn bipolar_to_unipolar(value: f32) -> f32 {
    (value + 1.0) * 0.5
}

/// Convert a unipolar value \[0, 1\] to bipolar \[−1, +1\].
#[inline]
pub fn unipolar_to_bipolar(value: f32) -> f32 {
    value * 2.0 - 1.0
}

// ── Delay / reverb helpers ─────────────────────────────────────────────────

/// Estimate the decay tail time (in seconds) for a delay line with feedback.
///
/// Uses the −60 dB rule: `delay_time × ln(1 / (1 − feedback)) / 3`.
/// Feedback is clamped to \[0, 0.99\] to avoid infinite tails.
#[inline]
pub fn feedback_decay_time(delay_time: f32, feedback: f32) -> f32 {
    let fb = feedback.min(0.99);
    if fb < 0.001 {
        return delay_time;
    }
    delay_time * (1.0 / (1.0 - fb)).ln() / 3.0
}

/// Exponential frequency mapping for perceptual linearity.
///
/// Maps a normalized \[0, 1\] value to a frequency range using exponential
/// interpolation: `low_hz × (high_hz / low_hz)^normalized`.
#[inline]
pub fn exponential_frequency_map(normalized: f32, low_hz: f32, high_hz: f32) -> f32 {
    low_hz * (high_hz / low_hz).powf(normalized)
}

// ── Gate / trigger helpers ─────────────────────────────────────────────────

/// Gate pulse from a phase value and duty cycle.
///
/// Returns `true` when `phase < duty_cycle`.  Typical duty cycle is 0.5.
#[inline]
pub fn gate_pulse(phase: f32, duty_cycle: f32) -> bool {
    phase < duty_cycle
}

// ── Vector mixing ──────────────────────────────────────────────────────────

/// Bilinear equal-power crossfade gains for a 4-corner XY vector mixer.
///
/// `x_norm` and `y_norm` are in \[0, 1\].
/// Returns `(top_left, top_right, bottom_left, bottom_right)` gains.
#[inline]
pub fn bilinear_equal_power_gains(x_norm: f32, y_norm: f32) -> (f32, f32, f32, f32) {
    let w_a = ((1.0 - x_norm) * y_norm).sqrt();
    let w_b = (x_norm * y_norm).sqrt();
    let w_c = ((1.0 - x_norm) * (1.0 - y_norm)).sqrt();
    let w_d = (x_norm * (1.0 - y_norm)).sqrt();
    (w_a, w_b, w_c, w_d)
}

// ── Scale quantization ─────────────────────────────────────────────────────

/// Quantize a value in semitones to the nearest scale degree.
///
/// `semitones` is the raw pitch value.  `intervals` is a slice of scale
/// intervals within one octave (e.g., `[0, 2, 4, 5, 7, 9, 11]` for major).
/// Returns the quantized value in octaves (V/oct).
#[inline]
pub fn quantize_to_scale(semitones: f32, intervals: &[u8]) -> f32 {
    if intervals.is_empty() {
        return 0.0;
    }
    let octave = (semitones / 12.0).floor();
    let note_in_octave = semitones - octave * 12.0;

    let mut closest = intervals[0];
    let mut min_dist = f32::MAX;
    for &interval in intervals {
        let dist = (note_in_octave - interval as f32).abs();
        if dist < min_dist {
            min_dist = dist;
            closest = interval;
        }
    }
    (octave * 12.0 + closest as f32) / 12.0
}

// ── Additive synthesis helpers ─────────────────────────────────────────────

/// Spectral rolloff using exponential tilt attenuation.
///
/// `harmonic_num` is the 1-based harmonic number.  `tilt` controls rolloff
/// intensity (0 = flat, 1 = steep).  Returns a gain factor in \[0, 1\].
#[inline]
pub fn spectral_rolloff(harmonic_num: f32, tilt: f32) -> f32 {
    let octave = harmonic_num.max(1.0).log2();
    (-tilt * octave * 3.0).exp()
}

/// Odd/even harmonic balance factor.
///
/// `harmonic_index` is 0-based.  `balance` in \[0, 1\]:
/// 0 = odd harmonics dominant, 1 = even harmonics dominant.
#[inline]
pub fn odd_even_balance(harmonic_index: usize, balance: f32) -> f32 {
    let is_odd = (harmonic_index + 1) % 2 == 1;
    if is_odd {
        1.0 - balance * 0.8
    } else {
        balance * 1.2 + 0.2
    }
}

/// Brightness boost for upper harmonics.
///
/// `harmonic_num` is the 1-based harmonic number.  `brightness` in \[0, 1\]:
/// 0.5 = neutral, above = boost high harmonics, below = attenuate.
#[inline]
pub fn brightness_boost(harmonic_num: f32, brightness: f32) -> f32 {
    if harmonic_num > 4.0 {
        let boost = (brightness - 0.5) * 2.0;
        (1.0 + boost * (harmonic_num / 8.0).min(1.0)).max(0.0)
    } else {
        1.0
    }
}

// ── Decay envelopes ────────────────────────────────────────────────────────

/// Simple exponential decay: `e^(−phase × rate)`.
///
/// `phase_norm` in \[0, 1\], `decay_rate` > 0 (higher = faster decay).
#[inline]
pub fn exponential_decay(phase_norm: f32, decay_rate: f32) -> f32 {
    (-phase_norm * decay_rate).exp()
}
