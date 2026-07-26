//! Tests for `pre_master_peak_tests`.

use super::pre_master_peak_for;
use synth_core::{BipolarValue, Gain};

/// Default rig: pan = center, volume = MAX. Per-channel peaks both equal
/// `internal × 0.7071` (constant-power pan-law at center). Dividing each
/// channel peak by its own gain restores the internal value, and we take
/// the larger of the two — should land at the internal patch peak.
#[test]
fn reverses_constant_power_pan_law_at_center() {
    let internal = 0.8_f32;
    let attenuated = internal * std::f32::consts::FRAC_1_SQRT_2;
    let gains = (Gain::new(1.0), BipolarValue::CENTER);
    let (peak, dbfs) = pre_master_peak_for(Some(&gains), attenuated, attenuated);
    assert!((peak - internal).abs() < 1e-4, "got {peak}");
    assert!((dbfs - 20.0 * internal.log10()).abs() < 1e-3);
}

/// Volume drop should also be reversed: at half volume the rendered peak
/// halves on top of the pan-law attenuation; the restored peak must still
/// match the internal value.
#[test]
fn reverses_volume_drop() {
    let internal = 0.8_f32;
    let rendered = internal * 0.5 * std::f32::consts::FRAC_1_SQRT_2;
    let gains = (Gain::new(0.5), BipolarValue::CENTER);
    let (peak, _) = pre_master_peak_for(Some(&gains), rendered, rendered);
    assert!((peak - internal).abs() < 1e-4, "got {peak}");
}

/// Hard-panned signals leave one channel silent. The silent channel's
/// `peak / 0` division must be skipped (1e-6 floor) and the live channel
/// should drive the result.
#[test]
fn handles_hard_pan_without_division_by_zero() {
    let internal = 0.6_f32;
    let gains = (Gain::new(1.0), BipolarValue::new(1.0)); // full right
    // Only the right channel carries signal; left is silent.
    let (peak, _) = pre_master_peak_for(Some(&gains), 0.0, internal);
    assert!((peak - internal).abs() < 1e-4, "got {peak}");
}

/// Missing snapshot (instrument id we don't know) falls back to the raw
/// per-channel peak so the caller still gets a meaningful lower bound.
#[test]
fn falls_back_to_raw_peak_when_gains_unknown() {
    let (peak, _) = pre_master_peak_for(None, 0.3, 0.6);
    assert!((peak - 0.6).abs() < 1e-4, "got {peak}");
}

/// Silence in → silence out, dBFS clamps to the silent floor instead of
/// reporting `-inf`.
#[test]
fn silence_reports_silent_floor() {
    let gains = (Gain::new(1.0), BipolarValue::CENTER);
    let (peak, dbfs) = pre_master_peak_for(Some(&gains), 0.0, 0.0);
    assert_eq!(peak, 0.0);
    assert_eq!(dbfs, crate::audio::mix_analysis::SILENT_FLOOR_DBFS);
}
