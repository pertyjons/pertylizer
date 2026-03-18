//! Tests for `synth_modules::math` shared DSP math functions.

use std::f32::consts::TAU;

use synth_core::{Hertz, SampleRate};
use synth_modules::math::*;

const EPSILON: f32 = 1e-6;

// ── soft_clip ───────────────────────────────────────────────────────────

#[test]
fn soft_clip_zero_is_zero() {
    assert!((soft_clip(0.0)).abs() < EPSILON);
}

#[test]
fn soft_clip_preserves_sign() {
    assert!(soft_clip(1.0) > 0.0);
    assert!(soft_clip(-1.0) < 0.0);
}

#[test]
fn soft_clip_bounded() {
    // tanh saturates to +-1; for moderate inputs it stays strictly within
    assert!(soft_clip(2.0) < 1.0);
    assert!(soft_clip(-2.0) > -1.0);
    // Even extreme inputs never exceed +-1
    assert!(soft_clip(100.0) <= 1.0);
    assert!(soft_clip(-100.0) >= -1.0);
}

#[test]
fn soft_clip_is_odd_function() {
    for &x in &[0.1, 0.5, 1.0, 3.0, 10.0] {
        assert!((soft_clip(x) + soft_clip(-x)).abs() < EPSILON);
    }
}

// ── hard_clip ───────────────────────────────────────────────────────────

#[test]
fn hard_clip_passes_within_range() {
    assert!((hard_clip(0.5) - 0.5).abs() < EPSILON);
    assert!((hard_clip(-0.3) - (-0.3)).abs() < EPSILON);
}

#[test]
fn hard_clip_clamps_positive() {
    assert!((hard_clip(1.5) - 1.0).abs() < EPSILON);
    assert!((hard_clip(100.0) - 1.0).abs() < EPSILON);
}

#[test]
fn hard_clip_clamps_negative() {
    assert!((hard_clip(-1.5) - (-1.0)).abs() < EPSILON);
    assert!((hard_clip(-100.0) - (-1.0)).abs() < EPSILON);
}

// ── foldback ────────────────────────────────────────────────────────────

#[test]
fn foldback_passes_within_threshold() {
    assert!((foldback(0.5, 1.0) - 0.5).abs() < EPSILON);
    assert!((foldback(-0.7, 1.0) - (-0.7)).abs() < EPSILON);
}

#[test]
fn foldback_reflects_once() {
    // 1.3 folds to 1.0 - 0.3 = 0.7
    assert!((foldback(1.3, 1.0) - 0.7).abs() < EPSILON);
    // -1.3 folds to -1.0 + 0.3 = -0.7
    assert!((foldback(-1.3, 1.0) - (-0.7)).abs() < EPSILON);
}

#[test]
fn foldback_reflects_twice() {
    // 2.3 -> first fold: 1.0 - 1.3 = -0.3 -> within range
    assert!((foldback(2.3, 1.0) - (-0.3)).abs() < EPSILON);
}

#[test]
fn foldback_stays_bounded() {
    let result = foldback(50.0, 1.0);
    assert!(result >= -1.0 && result <= 1.0);
}

#[test]
fn foldback_custom_threshold() {
    // With threshold 0.5: input 0.8 -> 0.5 - 0.3 = 0.2
    assert!((foldback(0.8, 0.5) - 0.2).abs() < EPSILON);
}

#[test]
fn foldback_at_threshold_is_threshold() {
    assert!((foldback(1.0, 1.0) - 1.0).abs() < EPSILON);
    assert!((foldback(-1.0, 1.0) - (-1.0)).abs() < EPSILON);
}

// ── tube_saturate ───────────────────────────────────────────────────────

#[test]
fn tube_saturate_zero_is_zero() {
    assert!((tube_saturate(0.0)).abs() < EPSILON);
}

#[test]
fn tube_saturate_positive_bounded() {
    assert!(tube_saturate(10.0) < 1.0);
    assert!(tube_saturate(10.0) > 0.9);
}

#[test]
fn tube_saturate_negative_bounded() {
    assert!(tube_saturate(-10.0) > -1.0);
    assert!(tube_saturate(-10.0) < -0.9);
}

#[test]
fn tube_saturate_is_odd_function() {
    for &x in &[0.1, 0.5, 1.0, 3.0] {
        assert!((tube_saturate(x) + tube_saturate(-x)).abs() < EPSILON);
    }
}

#[test]
fn tube_saturate_monotonic() {
    let mut prev = tube_saturate(0.0);
    for i in 1..20 {
        let x = i as f32 * 0.5;
        let cur = tube_saturate(x);
        assert!(
            cur > prev,
            "tube_saturate should be monotonically increasing"
        );
        prev = cur;
    }
}

// ── drive_gain ──────────────────────────────────────────────────────────

#[test]
fn drive_gain_zero_is_unity() {
    assert!((drive_gain(0.0, 50.0) - 1.0).abs() < EPSILON);
}

#[test]
fn drive_gain_full_distortion() {
    assert!((drive_gain(1.0, 50.0) - 51.0).abs() < EPSILON);
}

#[test]
fn drive_gain_full_waveshaper() {
    assert!((drive_gain(1.0, 19.0) - 20.0).abs() < EPSILON);
}

#[test]
fn drive_gain_quadratic_midpoint() {
    assert!((drive_gain(0.5, 50.0) - 13.5).abs() < EPSILON);
}

// ── equal_power_pan ─────────────────────────────────────────────────────

#[test]
fn equal_power_pan_center() {
    let (l, r) = equal_power_pan(0.0);
    assert!((l - r).abs() < EPSILON);
    assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-4);
}

#[test]
fn equal_power_pan_hard_left() {
    let (l, r) = equal_power_pan(-1.0);
    assert!((l - 1.0).abs() < EPSILON);
    assert!(r.abs() < EPSILON);
}

#[test]
fn equal_power_pan_hard_right() {
    let (l, r) = equal_power_pan(1.0);
    assert!(l.abs() < EPSILON);
    assert!((r - 1.0).abs() < EPSILON);
}

#[test]
fn equal_power_pan_preserves_energy() {
    for i in 0..=20 {
        let pan = (i as f32 / 10.0) - 1.0;
        let (l, r) = equal_power_pan(pan);
        let energy = l * l + r * r;
        assert!(
            (energy - 1.0).abs() < 1e-4,
            "Energy not preserved at pan={pan}: l²+r²={energy}"
        );
    }
}

// ── biquad_omega ────────────────────────────────────────────────────────

#[test]
fn biquad_omega_1khz_at_48k() {
    let omega = biquad_omega(Hertz::new(1000.0), SampleRate::DVD_QUALITY);
    let expected_w0 = TAU * 1000.0 / 48000.0;
    assert!((omega.cos_w0 - expected_w0.cos()).abs() < EPSILON);
    assert!((omega.sin_w0 - expected_w0.sin()).abs() < EPSILON);
}

#[test]
fn biquad_omega_clamps_near_nyquist() {
    let omega = biquad_omega(Hertz::new(30000.0), SampleRate::DVD_QUALITY);
    let max_freq = 48000.0 * 0.49;
    let expected_w0 = TAU * max_freq / 48000.0;
    assert!((omega.cos_w0 - expected_w0.cos()).abs() < EPSILON);
    assert!((omega.sin_w0 - expected_w0.sin()).abs() < EPSILON);
}

#[test]
fn biquad_omega_dc_is_near_zero() {
    let omega = biquad_omega(Hertz::new(0.0), SampleRate::DVD_QUALITY);
    assert!((omega.cos_w0 - 1.0).abs() < EPSILON);
    assert!(omega.sin_w0.abs() < EPSILON);
}

// ── biquad_alpha_from_q ─────────────────────────────────────────────────

#[test]
fn biquad_alpha_from_q_basic() {
    assert!((biquad_alpha_from_q(0.5, 1.0) - 0.25).abs() < EPSILON);
}

#[test]
fn biquad_alpha_from_q_high_q() {
    assert!((biquad_alpha_from_q(0.5, 10.0) - 0.025).abs() < EPSILON);
}

#[test]
fn biquad_alpha_from_q_clamps_minimum() {
    let result = biquad_alpha_from_q(0.5, 0.0);
    let expected = 0.5 / (2.0 * 0.01);
    assert!((result - expected).abs() < EPSILON);
}

// ── biquad_alpha_shelf ──────────────────────────────────────────────────

#[test]
fn biquad_alpha_shelf_unity_gain() {
    let sin_w0 = 0.5;
    let a = 1.0;
    let slope = 0.9;
    let result = biquad_alpha_shelf(sin_w0, a, slope);
    let inner = (a + 1.0 / a) * (1.0 / slope - 1.0) + 2.0;
    let expected = sin_w0 / 2.0 * inner.sqrt();
    assert!((result - expected).abs() < EPSILON);
}

#[test]
fn biquad_alpha_shelf_positive() {
    assert!(biquad_alpha_shelf(0.3, 2.0, 0.9) > 0.0);
}

// ── db_to_eq_amplitude ──────────────────────────────────────────────────

#[test]
fn db_to_eq_amplitude_zero_db() {
    assert!((db_to_eq_amplitude(0.0) - 1.0).abs() < EPSILON);
}

#[test]
fn db_to_eq_amplitude_positive() {
    let result = db_to_eq_amplitude(6.0);
    assert!((result - 10.0_f32.powf(0.15)).abs() < EPSILON);
}

#[test]
fn db_to_eq_amplitude_negative() {
    let result = db_to_eq_amplitude(-6.0);
    assert!(result < 1.0);
    assert!(result > 0.0);
}

#[test]
fn db_to_eq_amplitude_symmetric() {
    let pos = db_to_eq_amplitude(12.0);
    let neg = db_to_eq_amplitude(-12.0);
    assert!((pos * neg - 1.0).abs() < 1e-5);
}

// ── hann_window ─────────────────────────────────────────────────────────

#[test]
fn hann_window_zero_at_edges() {
    assert!(hann_window(0.0).abs() < EPSILON);
    assert!(hann_window(1.0).abs() < EPSILON);
}

#[test]
fn hann_window_peak_at_center() {
    assert!((hann_window(0.5) - 1.0).abs() < EPSILON);
}

#[test]
fn hann_window_symmetric() {
    for i in 1..10 {
        let phase = i as f32 / 20.0;
        let a = hann_window(phase);
        let b = hann_window(1.0 - phase);
        assert!(
            (a - b).abs() < EPSILON,
            "Hann window not symmetric at phase={phase}"
        );
    }
}

#[test]
fn hann_window_always_non_negative() {
    for i in 0..=100 {
        let phase = i as f32 / 100.0;
        assert!(
            hann_window(phase) >= -EPSILON,
            "Hann window negative at phase={phase}"
        );
    }
}

// ── normalization_gain ──────────────────────────────────────────────────

#[test]
fn normalization_gain_one_voice() {
    assert!((normalization_gain(1) - 1.0).abs() < EPSILON);
}

#[test]
fn normalization_gain_four_voices() {
    assert!((normalization_gain(4) - 0.5).abs() < EPSILON);
}

#[test]
fn normalization_gain_decreases_with_count() {
    let mut prev = normalization_gain(1);
    for n in 2..=16 {
        let cur = normalization_gain(n);
        assert!(cur < prev);
        prev = cur;
    }
}

#[test]
fn normalization_gain_preserves_total_power() {
    for n in 1..=32 {
        let g = normalization_gain(n);
        let total_power = n as f32 * g * g;
        assert!(
            (total_power - 1.0).abs() < 1e-5,
            "Total power not 1.0 for {n} voices: {total_power}"
        );
    }
}

// ── rising_edge ────────────────────────────────────────────────────────────

#[test]
fn rising_edge_basic_crossing() {
    assert!(rising_edge(0.6, 0.4));
}

#[test]
fn rising_edge_at_threshold() {
    // current > 0.5 and previous <= 0.5
    assert!(rising_edge(0.51, 0.5));
    // current exactly at 0.5 should NOT trigger
    assert!(!rising_edge(0.5, 0.4));
}

#[test]
fn rising_edge_below_threshold() {
    assert!(!rising_edge(0.3, 0.2));
}

#[test]
fn rising_edge_already_above() {
    assert!(!rising_edge(0.8, 0.7));
}

// ── samples_per_16th ───────────────────────────────────────────────────────

#[test]
fn samples_per_16th_120bpm_48k() {
    // 48000 * 60 / 120 / 4 = 6000
    let result = samples_per_16th(48000.0, 120.0);
    assert!((result - 6000.0).abs() < EPSILON);
}

#[test]
fn samples_per_16th_clamps_low_bpm() {
    // BPM below 20 is clamped to 20
    let result = samples_per_16th(48000.0, 5.0);
    let expected = 48000.0 * 60.0 / 20.0 / 4.0;
    assert!((result - expected).abs() < EPSILON);
}

#[test]
fn samples_per_16th_zero_bpm() {
    // 0 BPM clamped to 20
    let result = samples_per_16th(48000.0, 0.0);
    assert!(result.is_finite());
    assert!(result > 0.0);
}

// ── xorshift32 ─────────────────────────────────────────────────────────────

#[test]
fn xorshift32_deterministic() {
    let mut s1 = 42u32;
    let mut s2 = 42u32;
    let a = xorshift32(&mut s1);
    let b = xorshift32(&mut s2);
    assert!((a - b).abs() < EPSILON);
}

#[test]
fn xorshift32_in_unit_range() {
    let mut state = 12345u32;
    for _ in 0..1000 {
        let v = xorshift32(&mut state);
        assert!(v >= 0.0, "xorshift32 returned {v} < 0");
        assert!(v < 1.0, "xorshift32 returned {v} >= 1");
    }
}

#[test]
fn xorshift32_advances_state() {
    let mut state = 1u32;
    let old = state;
    let _ = xorshift32(&mut state);
    assert_ne!(state, old);
}

// ── cents_to_ratio ─────────────────────────────────────────────────────────

#[test]
fn cents_to_ratio_zero() {
    assert!((cents_to_ratio(0.0) - 1.0).abs() < EPSILON);
}

#[test]
fn cents_to_ratio_octave_up() {
    assert!((cents_to_ratio(1200.0) - 2.0).abs() < EPSILON);
}

#[test]
fn cents_to_ratio_octave_down() {
    assert!((cents_to_ratio(-1200.0) - 0.5).abs() < EPSILON);
}

// ── semitones_to_ratio ─────────────────────────────────────────────────────

#[test]
fn semitones_to_ratio_zero() {
    assert!((semitones_to_ratio(0.0) - 1.0).abs() < EPSILON);
}

#[test]
fn semitones_to_ratio_octave_up() {
    assert!((semitones_to_ratio(12.0) - 2.0).abs() < EPSILON);
}

#[test]
fn semitones_to_ratio_octave_down() {
    assert!((semitones_to_ratio(-12.0) - 0.5).abs() < EPSILON);
}

// ── freq_to_cv ─────────────────────────────────────────────────────────────

#[test]
fn freq_to_cv_c4() {
    assert!(freq_to_cv(261.63).abs() < 1e-4);
}

#[test]
fn freq_to_cv_c5() {
    assert!((freq_to_cv(523.26) - 1.0).abs() < 1e-3);
}

#[test]
fn freq_to_cv_zero_hz() {
    assert!((freq_to_cv(0.0)).abs() < EPSILON);
}

#[test]
fn freq_to_cv_negative_hz() {
    assert!((freq_to_cv(-100.0)).abs() < EPSILON);
}

// ── lerp ───────────────────────────────────────────────────────────────────

#[test]
fn lerp_at_start() {
    assert!((lerp(2.0, 8.0, 0.0) - 2.0).abs() < EPSILON);
}

#[test]
fn lerp_at_end() {
    assert!((lerp(2.0, 8.0, 1.0) - 8.0).abs() < EPSILON);
}

#[test]
fn lerp_midpoint() {
    assert!((lerp(2.0, 8.0, 0.5) - 5.0).abs() < EPSILON);
}

// ── linear_mix ─────────────────────────────────────────────────────────────

#[test]
fn linear_mix_dry_only() {
    assert!((linear_mix(0.7, 0.3, 0.0) - 0.7).abs() < EPSILON);
}

#[test]
fn linear_mix_wet_only() {
    assert!((linear_mix(0.7, 0.3, 1.0) - 0.3).abs() < EPSILON);
}

#[test]
fn linear_mix_half() {
    assert!((linear_mix(0.0, 1.0, 0.5) - 0.5).abs() < EPSILON);
}

// ── smooth_value ───────────────────────────────────────────────────────────

#[test]
fn smooth_value_instant() {
    // smoothing=0 → jump to target immediately
    assert!((smooth_value(0.0, 1.0, 0.0) - 1.0).abs() < EPSILON);
}

#[test]
fn smooth_value_frozen() {
    // smoothing=1 → stay at current
    assert!((smooth_value(0.3, 1.0, 1.0) - 0.3).abs() < EPSILON);
}

#[test]
fn smooth_value_midpoint() {
    // smoothing=0.5 → move halfway to target
    assert!((smooth_value(0.0, 1.0, 0.5) - 0.5).abs() < EPSILON);
}

// ── parabolic_interpolation ────────────────────────────────────────────────

#[test]
fn parabolic_interpolation_symmetric_peak() {
    // Symmetric around curr → offset = 0
    assert!(parabolic_interpolation(0.5, 1.0, 0.5).abs() < EPSILON);
}

#[test]
fn parabolic_interpolation_asymmetric() {
    // prev=0, curr=1, next=0.5 → offset should be negative (peak before curr)
    let offset = parabolic_interpolation(0.0, 1.0, 0.5);
    assert!(offset != 0.0);
    // Manual: 0.5*(0.0 - 0.5) / (0.0 - 2.0 + 0.5) = 0.5*(-0.5)/(-1.5) = 1/6
    assert!((offset - (1.0 / 6.0)).abs() < EPSILON);
}

#[test]
fn parabolic_interpolation_flat() {
    // All equal → denom ≈ 0 → returns 0
    assert!(parabolic_interpolation(1.0, 1.0, 1.0).abs() < EPSILON);
}

// ── apply_curve_shaping ────────────────────────────────────────────────────

#[test]
fn apply_curve_shaping_zero_passthrough() {
    assert!((apply_curve_shaping(0.5, 0.0) - 0.5).abs() < EPSILON);
}

#[test]
fn apply_curve_shaping_positive() {
    let base = 0.5;
    let result = apply_curve_shaping(base, 0.5);
    // Positive curve → powf(1/(1+1.5)) = powf(0.4) → result > base
    assert!(result > base);
}

#[test]
fn apply_curve_shaping_negative() {
    let base = 0.5;
    let result = apply_curve_shaping(base, -0.5);
    // Negative curve → powf(1+1.5) = powf(2.5) → result < base
    assert!(result < base);
}

// ── interpolate_with_curve ─────────────────────────────────────────────────

#[test]
fn interpolate_with_curve_linear() {
    assert!((interpolate_with_curve(0.0, 1.0, 0.5, 0.0) - 0.5).abs() < EPSILON);
}

#[test]
fn interpolate_with_curve_endpoints() {
    assert!((interpolate_with_curve(2.0, 8.0, 0.0, 0.5) - 2.0).abs() < EPSILON);
    assert!((interpolate_with_curve(2.0, 8.0, 1.0, 0.5) - 8.0).abs() < EPSILON);
}

#[test]
fn interpolate_with_curve_exponential() {
    // Positive curve → slow start → midpoint result < linear midpoint
    let result = interpolate_with_curve(0.0, 1.0, 0.5, 0.8);
    assert!(result < 0.5);
}

#[test]
fn interpolate_with_curve_logarithmic() {
    // Negative curve → fast start → midpoint result > linear midpoint
    let result = interpolate_with_curve(0.0, 1.0, 0.5, -0.8);
    assert!(result > 0.5);
}

// ── bipolar_curve ──────────────────────────────────────────────────────────

#[test]
fn bipolar_curve_zero_passthrough() {
    assert!((bipolar_curve(0.7, 0.0) - 0.7).abs() < EPSILON);
    assert!((bipolar_curve(-0.3, 0.0) - (-0.3)).abs() < EPSILON);
}

#[test]
fn bipolar_curve_positive() {
    // Positive curve → exponential → values pulled toward zero
    let result = bipolar_curve(0.5, 0.5);
    assert!(result < 0.5);
    assert!(result > 0.0);
}

#[test]
fn bipolar_curve_negative() {
    // Negative curve → logarithmic → values pushed toward extremes
    let result = bipolar_curve(0.5, -0.5);
    assert!(result > 0.5);
    assert!(result < 1.0);
}

// ── triangle_wave ──────────────────────────────────────────────────────────

#[test]
fn triangle_wave_at_zero() {
    assert!((triangle_wave(0.0) - (-1.0)).abs() < EPSILON);
}

#[test]
fn triangle_wave_at_quarter() {
    assert!((triangle_wave(0.25) - 0.0).abs() < EPSILON);
}

#[test]
fn triangle_wave_at_half() {
    assert!((triangle_wave(0.5) - 1.0).abs() < EPSILON);
}

#[test]
fn triangle_wave_at_three_quarter() {
    assert!((triangle_wave(0.75) - 0.0).abs() < EPSILON);
}

#[test]
fn triangle_wave_near_one() {
    // Just before 1.0 should approach -1.0
    assert!((triangle_wave(0.999) - (-0.996)).abs() < 0.01);
}

// ── gaussian_window ────────────────────────────────────────────────────────

#[test]
fn gaussian_window_peak_at_center() {
    assert!((gaussian_window(0.5, 0.4) - 1.0).abs() < EPSILON);
}

#[test]
fn gaussian_window_symmetric() {
    for i in 1..10 {
        let phase = i as f32 / 20.0;
        let a = gaussian_window(phase, 0.4);
        let b = gaussian_window(1.0 - phase, 0.4);
        assert!(
            (a - b).abs() < EPSILON,
            "Gaussian window not symmetric at phase={phase}"
        );
    }
}

#[test]
fn gaussian_window_always_positive() {
    for i in 0..=100 {
        let phase = i as f32 / 100.0;
        assert!(gaussian_window(phase, 0.4) > 0.0);
    }
}

// ── trapezoid_window ───────────────────────────────────────────────────────

#[test]
fn trapezoid_window_flat_middle() {
    assert!((trapezoid_window(0.5, 0.1) - 1.0).abs() < EPSILON);
    assert!((trapezoid_window(0.3, 0.1) - 1.0).abs() < EPSILON);
}

#[test]
fn trapezoid_window_ramp_edges() {
    // At phase=0 → 0/ramp = 0
    assert!(trapezoid_window(0.0, 0.1).abs() < EPSILON);
    // At half of ramp → 0.5
    assert!((trapezoid_window(0.05, 0.1) - 0.5).abs() < EPSILON);
    // Fade-out end
    assert!((trapezoid_window(0.95, 0.1) - 0.5).abs() < EPSILON);
}

// ── mid_side_encode ────────────────────────────────────────────────────────

#[test]
fn mid_side_encode_mono_signal() {
    let (mid, side) = mid_side_encode(0.8, 0.8);
    assert!((mid - 0.8).abs() < EPSILON);
    assert!(side.abs() < EPSILON);
}

#[test]
fn mid_side_encode_stereo_signal() {
    let (mid, side) = mid_side_encode(1.0, 0.0);
    assert!((mid - 0.5).abs() < EPSILON);
    assert!((side - 0.5).abs() < EPSILON);
}

// ── mid_side_decode ────────────────────────────────────────────────────────

#[test]
fn mid_side_decode_roundtrip() {
    let left = 0.7;
    let right = 0.3;
    let (mid, side) = mid_side_encode(left, right);
    let (l, r) = mid_side_decode(mid, side, 1.0);
    assert!((l - left).abs() < EPSILON);
    assert!((r - right).abs() < EPSILON);
}

#[test]
fn mid_side_decode_mono() {
    let (mid, side) = mid_side_encode(0.7, 0.3);
    let (l, r) = mid_side_decode(mid, side, 0.0);
    // width=0 → both channels equal mid
    assert!((l - r).abs() < EPSILON);
    assert!((l - mid).abs() < EPSILON);
}

// ── bitcrush ───────────────────────────────────────────────────────────────

#[test]
fn bitcrush_high_depth_passthrough() {
    let sample = 0.3;
    let result = bitcrush(sample, 16.0);
    assert!((result - sample).abs() < 1e-3);
}

#[test]
fn bitcrush_low_depth_quantizes() {
    let a = bitcrush(0.3, 1.0);
    let b = bitcrush(0.4, 1.0);
    // With 2 levels, nearby values should quantize to the same step
    assert!((a - b).abs() < EPSILON);
}

// ── quantize_signal ────────────────────────────────────────────────────────

#[test]
fn quantize_signal_many_levels_passthrough() {
    let sample = 0.42;
    let result = quantize_signal(sample, 10000.0);
    assert!((result - sample).abs() < 1e-3);
}

#[test]
fn quantize_signal_few_levels_quantizes() {
    let a = quantize_signal(0.3, 2.0);
    let b = quantize_signal(0.4, 2.0);
    // With only 2 levels, nearby values collapse
    assert!((a - b).abs() < EPSILON);
}

#[test]
fn quantize_signal_output_clamped() {
    let result = quantize_signal(2.0, 3.0);
    assert!(result <= 1.0);
    assert!(result >= -1.0);
}

// ── chebyshev_t2 ───────────────────────────────────────────────────────────

#[test]
fn chebyshev_t2_at_zero() {
    assert!((chebyshev_t2(0.0) - (-1.0)).abs() < EPSILON);
}

#[test]
fn chebyshev_t2_at_one() {
    assert!((chebyshev_t2(1.0) - 1.0).abs() < EPSILON);
}

#[test]
fn chebyshev_t2_at_neg_one() {
    assert!((chebyshev_t2(-1.0) - 1.0).abs() < EPSILON);
}

// ── chebyshev_t3 ───────────────────────────────────────────────────────────

#[test]
fn chebyshev_t3_at_zero() {
    assert!(chebyshev_t3(0.0).abs() < EPSILON);
}

#[test]
fn chebyshev_t3_at_one() {
    assert!((chebyshev_t3(1.0) - 1.0).abs() < EPSILON);
}

#[test]
fn chebyshev_t3_at_neg_one() {
    assert!((chebyshev_t3(-1.0) - (-1.0)).abs() < EPSILON);
}

// ── asymmetric_soft_clip ───────────────────────────────────────────────────

#[test]
fn asymmetric_soft_clip_symmetric() {
    // symmetry=0 → equal drive for positive and negative
    let pos = asymmetric_soft_clip(0.5, 0.0);
    let neg = asymmetric_soft_clip(-0.5, 0.0);
    assert!((pos + neg).abs() < EPSILON);
}

#[test]
fn asymmetric_soft_clip_positive_symmetry() {
    // Positive symmetry → harder positive clipping → positive side more saturated
    let sym_pos = asymmetric_soft_clip(0.5, 0.5);
    let sym_zero = asymmetric_soft_clip(0.5, 0.0);
    // Higher drive means closer to saturation → higher value for same input
    assert!(sym_pos > sym_zero);
}

#[test]
fn asymmetric_soft_clip_preserves_sign() {
    assert!(asymmetric_soft_clip(0.5, 0.3) > 0.0);
    assert!(asymmetric_soft_clip(-0.5, 0.3) < 0.0);
}

// ── soft_knee_limit ────────────────────────────────────────────────────────

#[test]
fn soft_knee_limit_below_threshold_passthrough() {
    assert!((soft_knee_limit(0.3, 0.5) - 0.3).abs() < EPSILON);
    assert!((soft_knee_limit(-0.4, 0.5) - (-0.4)).abs() < EPSILON);
}

#[test]
fn soft_knee_limit_above_threshold_limited() {
    let result = soft_knee_limit(2.0, 0.5);
    // Should be less than the input but above the threshold
    assert!(result < 2.0);
    assert!(result > 0.5);
}

#[test]
fn soft_knee_limit_bounded() {
    // Even extreme input should remain bounded
    let result = soft_knee_limit(100.0, 0.5);
    assert!(result < 2.0);
    assert!(result > 0.0);
}

// ── velocity_sensitivity ───────────────────────────────────────────────────

#[test]
fn velocity_sensitivity_zero_sens() {
    assert!((velocity_sensitivity(0.5, 0.0) - 1.0).abs() < EPSILON);
}

#[test]
fn velocity_sensitivity_full_sens() {
    assert!((velocity_sensitivity(0.7, 1.0) - 0.7).abs() < EPSILON);
}

#[test]
fn velocity_sensitivity_half_sens() {
    // sens=0.5, vel=0.6 → 1 - 0.5*(1-0.6) = 1 - 0.2 = 0.8
    assert!((velocity_sensitivity(0.6, 0.5) - 0.8).abs() < EPSILON);
}

// ── envelope_coeff ─────────────────────────────────────────────────────────

#[test]
fn envelope_coeff_range() {
    let c = envelope_coeff(0.01, 48000.0);
    assert!(c > 0.0);
    assert!(c < 1.0);
}

#[test]
fn envelope_coeff_short_time_small() {
    let short = envelope_coeff(0.001, 48000.0);
    let long = envelope_coeff(0.1, 48000.0);
    assert!(short < long, "Shorter time should give smaller coefficient");
}

#[test]
fn envelope_coeff_long_time_near_one() {
    let c = envelope_coeff(10.0, 48000.0);
    assert!(c > 0.99);
}

// ── bipolar_to_unipolar ─────────────────────────────────────────────────────

#[test]
fn bipolar_to_unipolar_neg_one() {
    assert!((bipolar_to_unipolar(-1.0) - 0.0).abs() < EPSILON);
}

#[test]
fn bipolar_to_unipolar_zero() {
    assert!((bipolar_to_unipolar(0.0) - 0.5).abs() < EPSILON);
}

#[test]
fn bipolar_to_unipolar_pos_one() {
    assert!((bipolar_to_unipolar(1.0) - 1.0).abs() < EPSILON);
}

// ── unipolar_to_bipolar ─────────────────────────────────────────────────────

#[test]
fn unipolar_to_bipolar_zero() {
    assert!((unipolar_to_bipolar(0.0) - (-1.0)).abs() < EPSILON);
}

#[test]
fn unipolar_to_bipolar_half() {
    assert!((unipolar_to_bipolar(0.5) - 0.0).abs() < EPSILON);
}

#[test]
fn unipolar_to_bipolar_one() {
    assert!((unipolar_to_bipolar(1.0) - 1.0).abs() < EPSILON);
}

// ── bipolar / unipolar roundtrip ────────────────────────────────────────────

#[test]
fn bipolar_unipolar_roundtrip() {
    for &v in &[-1.0, -0.5, 0.0, 0.25, 0.75, 1.0] {
        let result = unipolar_to_bipolar(bipolar_to_unipolar(v));
        assert!(
            (result - v).abs() < EPSILON,
            "Roundtrip failed for {v}: got {result}"
        );
    }
}

// ── feedback_decay_time ─────────────────────────────────────────────────────

#[test]
fn feedback_decay_time_at_zero_feedback() {
    let dt = 0.5;
    assert!((feedback_decay_time(dt, 0.0) - dt).abs() < EPSILON);
}

#[test]
fn feedback_decay_time_zero_feedback_returns_delay_time() {
    let dt = 0.25;
    assert!((feedback_decay_time(dt, 0.0) - dt).abs() < EPSILON);
}

#[test]
fn feedback_decay_time_half_feedback_positive() {
    let dt = 0.5;
    let result = feedback_decay_time(dt, 0.5);
    assert!(
        result > 0.0,
        "feedback=0.5 should give positive decay time, got {result}"
    );
}

#[test]
fn feedback_decay_time_high_feedback_much_larger() {
    let dt = 0.5;
    let mid = feedback_decay_time(dt, 0.5);
    let high = feedback_decay_time(dt, 0.99);
    assert!(
        high > mid,
        "feedback=0.99 ({high}) should give larger decay than feedback=0.5 ({mid})"
    );
}

// ── exponential_frequency_map ───────────────────────────────────────────────

#[test]
fn exponential_frequency_map_at_zero() {
    assert!((exponential_frequency_map(0.0, 20.0, 20000.0) - 20.0).abs() < EPSILON);
}

#[test]
fn exponential_frequency_map_at_one() {
    assert!((exponential_frequency_map(1.0, 20.0, 20000.0) - 20000.0).abs() < 0.01);
}

#[test]
fn exponential_frequency_map_at_half_geometric_mean() {
    let result = exponential_frequency_map(0.5, 20.0, 20000.0);
    let geometric_mean = (20.0_f32 * 20000.0).sqrt();
    assert!(
        (result - geometric_mean).abs() < 0.1,
        "At 0.5 expected geometric mean {geometric_mean}, got {result}"
    );
}

// ── gate_pulse ──────────────────────────────────────────────────────────────

#[test]
fn gate_pulse_on_at_zero() {
    assert!(gate_pulse(0.0, 0.5));
}

#[test]
fn gate_pulse_off_past_duty() {
    assert!(!gate_pulse(0.6, 0.5));
}

#[test]
fn gate_pulse_on_within_duty() {
    assert!(gate_pulse(0.3, 0.5));
}

// ── bilinear_equal_power_gains ──────────────────────────────────────────────

#[test]
fn bilinear_equal_power_gains_center_all_equal() {
    let (a, b, c, d) = bilinear_equal_power_gains(0.5, 0.5);
    assert!((a - b).abs() < EPSILON);
    assert!((b - c).abs() < EPSILON);
    assert!((c - d).abs() < EPSILON);
}

#[test]
fn bilinear_equal_power_gains_corner_dominates() {
    let (tl, tr, bl, br) = bilinear_equal_power_gains(0.0, 1.0);
    // x=0, y=1 → top_left = sqrt((1-0)*1) = 1.0
    assert!((tl - 1.0).abs() < EPSILON);
    assert!(tr.abs() < EPSILON);
    assert!(bl.abs() < EPSILON);
    assert!(br.abs() < EPSILON);
}

#[test]
fn bilinear_equal_power_gains_total_energy_constant() {
    for xi in 0..=10 {
        for yi in 0..=10 {
            let x = xi as f32 / 10.0;
            let y = yi as f32 / 10.0;
            let (a, b, c, d) = bilinear_equal_power_gains(x, y);
            let energy = a * a + b * b + c * c + d * d;
            assert!(
                (energy - 1.0).abs() < 1e-4,
                "Energy not ~1.0 at ({x}, {y}): {energy}"
            );
        }
    }
}

// ── quantize_to_scale ───────────────────────────────────────────────────────

#[test]
fn quantize_to_scale_chromatic_passthrough() {
    let chromatic: Vec<u8> = (0..12).collect();
    for semi in 0..12 {
        let result = quantize_to_scale(semi as f32, &chromatic);
        assert!(
            (result - semi as f32 / 12.0).abs() < EPSILON,
            "Chromatic passthrough failed for {semi}"
        );
    }
}

#[test]
fn quantize_to_scale_major_snaps() {
    let major = [0u8, 2, 4, 5, 7, 9, 11];
    // 1 semitone should snap to 0 or 2 (nearest degree)
    let result = quantize_to_scale(1.0, &major);
    let quantized_semi = result * 12.0;
    assert!(
        (quantized_semi - 0.0).abs() < EPSILON || (quantized_semi - 2.0).abs() < EPSILON,
        "1 semitone should snap to 0 or 2 in major scale, got {quantized_semi}"
    );
}

#[test]
fn quantize_to_scale_zero_semitones() {
    let major = [0u8, 2, 4, 5, 7, 9, 11];
    let result = quantize_to_scale(0.0, &major);
    assert!(
        (result - 0.0).abs() < EPSILON,
        "0 semitones should quantize to 0.0"
    );
}

// ── spectral_rolloff ────────────────────────────────────────────────────────

#[test]
fn spectral_rolloff_zero_tilt_flat() {
    assert!((spectral_rolloff(5.0, 0.0) - 1.0).abs() < EPSILON);
}

#[test]
fn spectral_rolloff_harmonic_1_always_near_one() {
    for &tilt in &[0.0, 0.5, 1.0] {
        let result = spectral_rolloff(1.0, tilt);
        assert!(
            (result - 1.0).abs() < EPSILON,
            "Harmonic 1 at tilt={tilt} should be ~1.0, got {result}"
        );
    }
}

#[test]
fn spectral_rolloff_higher_harmonics_attenuated() {
    let result = spectral_rolloff(8.0, 0.5);
    assert!(
        result < 1.0,
        "Higher harmonic with tilt>0 should be attenuated"
    );
    assert!(result > 0.0, "Gain should still be positive");
}

// ── odd_even_balance ────────────────────────────────────────────────────────

#[test]
fn odd_even_balance_zero_favors_odd() {
    let odd_gain = odd_even_balance(0, 0.0); // index 0 → harmonic 1 (odd)
    let even_gain = odd_even_balance(1, 0.0); // index 1 → harmonic 2 (even)
    assert!(
        odd_gain > even_gain,
        "balance=0 should favor odd: odd={odd_gain}, even={even_gain}"
    );
}

#[test]
fn odd_even_balance_one_favors_even() {
    let odd_gain = odd_even_balance(0, 1.0);
    let even_gain = odd_even_balance(1, 1.0);
    assert!(
        even_gain > odd_gain,
        "balance=1 should favor even: odd={odd_gain}, even={even_gain}"
    );
}

// ── brightness_boost ────────────────────────────────────────────────────────

#[test]
fn brightness_boost_harmonic_1_always_one() {
    for &b in &[0.0, 0.5, 1.0] {
        assert!(
            (brightness_boost(1.0, b) - 1.0).abs() < EPSILON,
            "Harmonic 1 should always return 1.0"
        );
    }
}

#[test]
fn brightness_boost_neutral_at_half() {
    assert!((brightness_boost(8.0, 0.5) - 1.0).abs() < EPSILON);
}

#[test]
fn brightness_boost_above_half_boosts() {
    let result = brightness_boost(8.0, 1.0);
    assert!(
        result > 1.0,
        "brightness=1.0 should boost high harmonics, got {result}"
    );
}

// ── exponential_decay ───────────────────────────────────────────────────────

#[test]
fn exponential_decay_at_zero_is_one() {
    assert!((exponential_decay(0.0, 5.0) - 1.0).abs() < EPSILON);
}

#[test]
fn exponential_decay_large_phase_near_zero() {
    let result = exponential_decay(10.0, 5.0);
    assert!(
        result < 1e-10,
        "Large phase should decay near zero, got {result}"
    );
}

#[test]
fn exponential_decay_monotonic_decrease() {
    let rate = 3.0;
    let mut prev = exponential_decay(0.0, rate);
    for i in 1..=20 {
        let phase = i as f32 * 0.05;
        let cur = exponential_decay(phase, rate);
        assert!(
            cur < prev,
            "Decay should be monotonically decreasing at phase={phase}"
        );
        prev = cur;
    }
}

// ── fast_sin_turns ────────────────────────────────────────────────────────

#[test]
fn fast_sin_turns_zero_crossings() {
    // sin(0) = 0, sin(π) = 0
    assert!(fast_sin_turns(0.0).abs() < 0.002);
    assert!(fast_sin_turns(0.5).abs() < 0.002);
    assert!(fast_sin_turns(1.0).abs() < 0.002);
}

#[test]
fn fast_sin_turns_peaks() {
    // sin(π/2) = 1, sin(3π/2) = -1
    assert!((fast_sin_turns(0.25) - 1.0).abs() < 0.002);
    assert!((fast_sin_turns(0.75) - (-1.0)).abs() < 0.002);
}

#[test]
fn fast_sin_turns_matches_std_sin() {
    // Verify max error < 0.002 across full cycle
    let mut max_error = 0.0_f32;
    for i in 0..1000 {
        let phase = i as f32 / 1000.0;
        let fast = fast_sin_turns(phase);
        let exact = (phase * std::f32::consts::TAU).sin();
        let error = (fast - exact).abs();
        if error > max_error {
            max_error = error;
        }
    }
    assert!(
        max_error < 0.002,
        "Max error should be < 0.002, got {max_error}"
    );
}

#[test]
fn fast_sin_turns_wraps_correctly() {
    // Values outside [0,1) should wrap
    assert!((fast_sin_turns(1.25) - fast_sin_turns(0.25)).abs() < 0.002);
    assert!((fast_sin_turns(-0.25) - fast_sin_turns(0.75)).abs() < 0.002);
    assert!((fast_sin_turns(5.25) - fast_sin_turns(0.25)).abs() < 0.002);
}

#[test]
fn fast_sin_turns_is_odd_symmetric() {
    for i in 1..10 {
        let phase = i as f32 / 20.0;
        let pos = fast_sin_turns(phase);
        let neg = fast_sin_turns(1.0 - phase); // sin(2π(1-x)) = -sin(2πx)
        assert!(
            (pos + neg).abs() < 0.002,
            "Odd symmetry broken at phase={phase}: {pos} vs -{neg}"
        );
    }
}
