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
        assert!(cur > prev, "tube_saturate should be monotonically increasing");
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
