//! Pure helpers for patch sweeps.
//!
//! The bridge runs the offline-render loop (one render per swept note or
//! velocity) and feeds the resulting [`AnalyzeNoteResult`]s through here.
//! Splitting the per-step extraction and the cross-step issue detection from
//! the render loop keeps the engine plumbing in `mcp_bridge.rs` and lets these
//! helpers be unit-tested with synthesized inputs — no audio engine needed.

use synth_core::MidiNote;
use synth_mcp::types::{
    AnalyzeNoteResult, InstrumentRangeIssues, InstrumentRangeStep, VelocityResponseIssues,
    VelocityResponseStep,
};

use crate::audio::mix_analysis::lin_to_db;

/// Amplitude floor below which a step is considered silent (≈ -60 dBFS).
pub const SILENCE_FLOOR: f32 = 0.001;

/// Confidence below which a high-centroid step is flagged as likely aliased.
const ALIASING_CONFIDENCE_CEILING: f32 = 0.3;

/// Confidence below which the fundamental / centroid columns for a step are
/// treated as unusable noise (not the same as `pitch_lost`, which answers
/// "did the patch break" — this answers "is the reading meaningful").
const PITCH_UNRELIABLE_CONFIDENCE_CEILING: f32 = 0.10;

/// Confidence below which `pitch_lost` fires regardless of cent error.
const PITCH_LOST_CONFIDENCE_CEILING: f32 = 0.20;

/// Cent error magnitude above which `pitch_lost` fires regardless of confidence.
const PITCH_LOST_CENTS_CEILING: f32 = 1200.0;

/// Velocity-range smaller than this in dB flags the patch as unresponsive.
const VELOCITY_RESPONSE_MIN_DB: f32 = 3.0;

/// Upper bound (exclusive) of the "musically compressed" band. Patches that
/// respond to velocity but with a tight dynamic envelope (3..10 dB) get the
/// compressed-response flag instead of `velocity_unresponsive`.
const VELOCITY_COMPRESSED_MAX_DB: f32 = 10.0;

/// Median of `centroid_envelope` (sustain-time brightness). `0.0` when empty
/// or all-zero. Median (not mean) so a noisy attack/release window doesn't
/// drag the value.
fn centroid_summary(env: &[f32]) -> f32 {
    let mut values: Vec<f32> = env.iter().copied().filter(|v| *v > 0.0).collect();
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}

/// Reduce an `AnalyzeNoteResult` to a compact per-step entry for an
/// instrument-range sweep. `nyquist_hz` is needed for the aliasing heuristic
/// (centroid > nyquist/2 + low confidence).
pub fn range_step_from_analysis(
    note: u8,
    result: &AnalyzeNoteResult,
    nyquist_hz: f32,
) -> InstrumentRangeStep {
    let centroid_hz = centroid_summary(&result.centroid_envelope);
    let pitch_confidence = result.pitch_confidence.unwrap_or(0.0);
    let silent = result.peak_amplitude < SILENCE_FLOOR;
    let likely_aliased =
        !silent && pitch_confidence < ALIASING_CONFIDENCE_CEILING && centroid_hz > nyquist_hz * 0.5;
    let pitch_unreliable = !silent && pitch_confidence < PITCH_UNRELIABLE_CONFIDENCE_CEILING;
    let pitch_lost = !silent && pitch_lost_for(result, pitch_confidence);

    InstrumentRangeStep {
        note: MidiNote::new(note),
        note_played: result.note_played,
        expected_hz: result.expected_fundamental_hz,
        fundamental_hz: result.fundamental_hz,
        pitch_error_cents: result.pitch_error_cents,
        pitch_confidence,
        peak_amplitude: result.peak_amplitude,
        rms_overall: result.rms_overall,
        centroid_hz,
        clipped_samples: result.clipped_samples,
        silent,
        likely_aliased,
        pitch_lost,
        pitch_unreliable,
    }
}

/// `pitch_lost` fires when the pitch tracker is so far off that the step's
/// data is unusable — three independent paths can trip it:
///
/// * Confidence below `PITCH_LOST_CONFIDENCE_CEILING` (0.20). The detected
///   fundamental is essentially noise even when its ratio to the expected
///   pitch *looks* fine.
/// * Cent error magnitude above `PITCH_LOST_CENTS_CEILING` (1200). A full
///   octave off in cents, even when the ratio sits inside the octave window
///   below (e.g. confidence-weighted blend reports an in-range Hz but the
///   per-channel cent measurement disagrees).
/// * Ratio outside `[0.5, 2.0]` — the original octave-error guard.
fn pitch_lost_for(result: &AnalyzeNoteResult, pitch_confidence: f32) -> bool {
    if pitch_confidence < PITCH_LOST_CONFIDENCE_CEILING {
        return true;
    }
    if result.pitch_error_cents.abs() > PITCH_LOST_CENTS_CEILING {
        return true;
    }
    if result.expected_fundamental_hz <= 0.0 || result.fundamental_hz <= 0.0 {
        return false;
    }
    let ratio = result.fundamental_hz / result.expected_fundamental_hz;
    !(0.5..=2.0).contains(&ratio)
}

/// Aggregate per-step entries into a cross-step issue summary.
pub fn range_issues_from_steps(steps: &[InstrumentRangeStep]) -> InstrumentRangeIssues {
    let mut issues = InstrumentRangeIssues::default();
    let mut min_peak: Option<f32> = None;
    let mut max_peak: Option<f32> = None;
    for step in steps {
        if step.silent {
            issues.silent_notes.push(step.note.as_u8());
        } else {
            min_peak = Some(min_peak.map_or(step.peak_amplitude, |m| m.min(step.peak_amplitude)));
            max_peak = Some(max_peak.map_or(step.peak_amplitude, |m| m.max(step.peak_amplitude)));
        }
        if step.likely_aliased {
            issues.aliased_notes.push(step.note.as_u8());
        }
        if step.pitch_lost {
            issues.pitch_lost_notes.push(step.note.as_u8());
        }
        if step.pitch_unreliable {
            issues.pitch_unreliable_notes.push(step.note.as_u8());
        }
        if step.clipped_samples > 0 {
            issues.clipping_notes.push(step.note.as_u8());
        }
    }
    if let (Some(min), Some(max)) = (min_peak, max_peak)
        && min > 0.0
    {
        issues.level_spread_db = lin_to_db(max) - lin_to_db(min);
    }
    issues
}

/// Reduce an `AnalyzeNoteResult` to a compact per-velocity entry.
pub fn velocity_step_from_analysis(
    velocity: u8,
    result: &AnalyzeNoteResult,
) -> VelocityResponseStep {
    VelocityResponseStep {
        velocity,
        peak_amplitude: result.peak_amplitude,
        rms_overall: result.rms_overall,
        centroid_hz: centroid_summary(&result.centroid_envelope),
        clipped_samples: result.clipped_samples,
    }
}

/// Aggregate per-velocity entries into monotonicity / responsiveness flags.
pub fn velocity_issues_from_steps(steps: &[VelocityResponseStep]) -> VelocityResponseIssues {
    let mut issues = VelocityResponseIssues::default();
    let non_silent: Vec<&VelocityResponseStep> = steps
        .iter()
        .filter(|s| s.peak_amplitude >= SILENCE_FLOOR)
        .collect();
    if non_silent.len() >= 2 {
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for s in &non_silent {
            lo = lo.min(s.peak_amplitude);
            hi = hi.max(s.peak_amplitude);
        }
        issues.amplitude_range_db = lin_to_db(hi) - lin_to_db(lo);
    }
    for pair in steps.windows(2) {
        if pair[1].peak_amplitude + 1e-6 < pair[0].peak_amplitude {
            issues.non_monotonic_amplitude_steps += 1;
        }
        if pair[0].centroid_hz > 0.0
            && pair[1].centroid_hz > 0.0
            && pair[1].centroid_hz + 1.0 < pair[0].centroid_hz
        {
            issues.non_monotonic_centroid_steps += 1;
        }
    }
    issues.velocity_unresponsive = issues.amplitude_range_db < VELOCITY_RESPONSE_MIN_DB;
    issues.velocity_compressed_response = issues.amplitude_range_db >= VELOCITY_RESPONSE_MIN_DB
        && issues.amplitude_range_db < VELOCITY_COMPRESSED_MAX_DB;
    // Centroid inversion plus a last-step centroid below the first-step
    // centroid means velocity makes the patch darker overall — almost always
    // a routing mistake (filter cutoff bound to inverse velocity, etc.).
    let half_steps = (steps.len() as u32) / 2;
    let endpoints_inverted = steps
        .first()
        .zip(steps.last())
        .is_some_and(|(first, last)| {
            first.centroid_hz > 0.0
                && last.centroid_hz > 0.0
                && last.centroid_hz < first.centroid_hz
        });
    issues.velocity_brightness_inverted =
        half_steps > 0 && issues.non_monotonic_centroid_steps >= half_steps && endpoints_inverted;
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_mcp::types::{
        AnalysisSignalMode, AnalyzeEnergyBands, AnalyzeEnvelopeEstimate, AnalyzeFlags,
        AnalyzeHarmonicContent, AnalyzeNoteResult,
    };

    fn dummy_result(
        peak: f32,
        centroid: f32,
        fundamental: f32,
        expected: f32,
    ) -> AnalyzeNoteResult {
        AnalyzeNoteResult {
            note_requested: 60,
            note_played: 60,
            velocity: 100,
            sample_rate: 44_100,
            duration_seconds: 1.0,
            fundamental_hz: fundamental,
            analysis_signal_mode: AnalysisSignalMode::MaxAbsStereo,
            fundamental_left: None,
            fundamental_right: None,
            fundamental_left_confidence: None,
            fundamental_right_confidence: None,
            expected_fundamental_hz: expected,
            pitch_error_cents: 0.0,
            peak_amplitude: peak,
            rms_overall: peak * 0.5,
            dc_offset: 0.0,
            clipped_samples: 0,
            envelope_window_ms: 25.0,
            rms_envelope: vec![peak * 0.5; 4],
            centroid_envelope: vec![centroid; 4],
            spectrum_attack: vec![],
            spectrum_sustain: vec![],
            spectrum_release: vec![],
            pitch_envelope: vec![],
            pitch_envelope_window_ms: 200.0,
            stereo_correlation: 1.0,
            energy_bands: AnalyzeEnergyBands {
                sub: 0.0,
                low: 0.0,
                mid: 0.0,
                high: 0.0,
            },
            harmonic_content: AnalyzeHarmonicContent {
                thd_db: -120.0,
                odd_even_ratio_db: 0.0,
                n_harmonics: 0,
            },
            envelope_estimate: AnalyzeEnvelopeEstimate {
                attack_ms: 0.0,
                decay_ms: 0.0,
                sustain_level: 0.0,
                release_ms: 0.0,
            },
            centroid_trend_hz_per_sec: 0.0,
            flags: AnalyzeFlags::default(),
            peak_left: None,
            peak_right: None,
            rms_left: None,
            rms_right: None,
            dc_left: None,
            dc_right: None,
            clipped_left: None,
            clipped_right: None,
            mid_rms: None,
            side_rms: None,
            stereo_width: None,
            pitch_confidence: Some(0.9),
            trimmed_tail_windows: None,
            attack_window_start_ms: None,
            sustain_window_start_ms: None,
            release_window_start_ms: None,
            warnings: vec![],
            module_descriptions: vec![],
        }
    }

    #[test]
    fn centroid_summary_picks_median() {
        assert_eq!(centroid_summary(&[]), 0.0);
        assert_eq!(centroid_summary(&[0.0, 0.0, 0.0]), 0.0);
        assert_eq!(centroid_summary(&[100.0, 200.0, 300.0]), 200.0);
        // Zeros are filtered out before computing the median.
        assert_eq!(centroid_summary(&[0.0, 100.0, 200.0, 300.0]), 200.0);
    }

    #[test]
    fn silent_step_skips_aliasing_and_pitch_lost_flags() {
        let result = dummy_result(0.0001, 0.0, 0.0, 440.0);
        let step = range_step_from_analysis(60, &result, 22_050.0);
        assert!(step.silent);
        assert!(!step.likely_aliased);
        assert!(!step.pitch_lost);
    }

    #[test]
    fn high_centroid_low_confidence_flags_aliasing() {
        let mut result = dummy_result(0.5, 18_000.0, 18_000.0, 19_000.0);
        result.pitch_confidence = Some(0.1);
        let step = range_step_from_analysis(108, &result, 22_050.0);
        assert!(!step.silent);
        assert!(step.likely_aliased);
    }

    #[test]
    fn confident_high_centroid_does_not_alias() {
        let mut result = dummy_result(0.5, 18_000.0, 18_000.0, 19_000.0);
        result.pitch_confidence = Some(0.9);
        let step = range_step_from_analysis(108, &result, 22_050.0);
        assert!(!step.likely_aliased);
    }

    #[test]
    fn pitch_lost_when_fundamental_off_by_more_than_octave() {
        // 200 Hz is below 440 / 2 = 220, i.e. more than an octave below the
        // expected pitch — the boundary itself is allowed (see `pitch_lost_for`).
        let result = dummy_result(0.5, 1000.0, 200.0, 440.0);
        let step = range_step_from_analysis(69, &result, 22_050.0);
        assert!(step.pitch_lost);
    }

    #[test]
    fn pitch_at_exact_octave_is_not_lost() {
        // An exact octave (220 vs 440) is the boundary and counts as in-range.
        let result = dummy_result(0.5, 1000.0, 220.0, 440.0);
        let step = range_step_from_analysis(69, &result, 22_050.0);
        assert!(!step.pitch_lost);
    }

    #[test]
    fn pitch_within_octave_is_not_lost() {
        let result = dummy_result(0.5, 1000.0, 450.0, 440.0);
        let step = range_step_from_analysis(69, &result, 22_050.0);
        assert!(!step.pitch_lost);
    }

    #[test]
    fn pitch_unreliable_when_confidence_below_threshold() {
        // Reese-style failure case: ratio looks OK but confidence is at noise
        // level and the cent error is way outside musical range.
        let mut result = dummy_result(0.5, 1000.0, 440.0, 440.0);
        result.pitch_confidence = Some(0.007);
        let step = range_step_from_analysis(96, &result, 22_050.0);
        assert!(step.pitch_unreliable);
        // Confidence 0.007 < 0.20 → pitch_lost also fires.
        assert!(step.pitch_lost);
    }

    #[test]
    fn pitch_lost_when_cent_error_exceeds_octave() {
        // Cent error magnitude well above 1200 → pitch_lost regardless of the
        // fundamental_hz / expected_hz ratio.
        let mut result = dummy_result(0.5, 1000.0, 440.0, 440.0);
        result.pitch_error_cents = 1500.0;
        result.pitch_confidence = Some(0.5);
        let step = range_step_from_analysis(69, &result, 22_050.0);
        assert!(step.pitch_lost);
        // Confidence above 0.10 → not flagged as unreliable.
        assert!(!step.pitch_unreliable);
    }

    #[test]
    fn pitch_lost_when_confidence_at_floor() {
        let mut result = dummy_result(0.5, 1000.0, 440.0, 440.0);
        result.pitch_confidence = Some(0.05);
        let step = range_step_from_analysis(69, &result, 22_050.0);
        assert!(step.pitch_lost);
        assert!(step.pitch_unreliable);
    }

    #[test]
    fn range_issues_collect_pitch_unreliable_notes() {
        let mut a = dummy_result(0.5, 1000.0, 440.0, 440.0);
        a.pitch_confidence = Some(0.05);
        let mut b = dummy_result(0.5, 1000.0, 440.0, 440.0);
        b.pitch_confidence = Some(0.5);
        let steps = [
            range_step_from_analysis(60, &a, 22_050.0),
            range_step_from_analysis(72, &b, 22_050.0),
        ];
        let issues = range_issues_from_steps(&steps);
        assert_eq!(issues.pitch_unreliable_notes, vec![60]);
    }

    #[test]
    fn range_issues_collect_per_note_problems() {
        let mut a = dummy_result(0.5, 1000.0, 440.0, 440.0);
        a.clipped_samples = 5;
        let mut b = dummy_result(0.001, 0.0, 0.0, 880.0);
        b.peak_amplitude = 0.0001;
        let mut c = dummy_result(0.5, 18_000.0, 18_000.0, 19_000.0);
        c.pitch_confidence = Some(0.1);
        let steps = [
            range_step_from_analysis(60, &a, 22_050.0),
            range_step_from_analysis(72, &b, 22_050.0),
            range_step_from_analysis(108, &c, 22_050.0),
        ];
        let issues = range_issues_from_steps(&steps);
        assert_eq!(issues.silent_notes, vec![72]);
        assert_eq!(issues.aliased_notes, vec![108]);
        assert_eq!(issues.clipping_notes, vec![60]);
    }

    #[test]
    fn range_level_spread_db_uses_non_silent_steps() {
        let quiet = dummy_result(0.1, 1000.0, 440.0, 440.0);
        let loud = dummy_result(1.0, 1000.0, 880.0, 880.0);
        let silent = dummy_result(0.0001, 0.0, 0.0, 0.0);
        let steps = [
            range_step_from_analysis(60, &quiet, 22_050.0),
            range_step_from_analysis(72, &loud, 22_050.0),
            range_step_from_analysis(84, &silent, 22_050.0),
        ];
        let issues = range_issues_from_steps(&steps);
        // 20 * log10(1.0 / 0.1) = 20 dB
        assert!((issues.level_spread_db - 20.0).abs() < 0.01);
    }

    #[test]
    fn velocity_issues_flag_unresponsive_patch() {
        let a = dummy_result(0.5, 1000.0, 440.0, 440.0);
        let b = dummy_result(0.55, 1000.0, 440.0, 440.0);
        let steps = [
            velocity_step_from_analysis(40, &a),
            velocity_step_from_analysis(127, &b),
        ];
        let issues = velocity_issues_from_steps(&steps);
        // 20 * log10(0.55 / 0.5) ≈ 0.83 dB — well under the 3 dB threshold.
        assert!(issues.velocity_unresponsive);
        assert_eq!(issues.non_monotonic_amplitude_steps, 0);
    }

    #[test]
    fn velocity_issues_count_amplitude_inversions() {
        let a = dummy_result(0.2, 1000.0, 440.0, 440.0);
        let b = dummy_result(0.5, 1000.0, 440.0, 440.0);
        // c < b: inversion.
        let c = dummy_result(0.3, 1000.0, 440.0, 440.0);
        let steps = [
            velocity_step_from_analysis(40, &a),
            velocity_step_from_analysis(80, &b),
            velocity_step_from_analysis(120, &c),
        ];
        let issues = velocity_issues_from_steps(&steps);
        assert_eq!(issues.non_monotonic_amplitude_steps, 1);
        assert!(!issues.velocity_unresponsive);
    }

    #[test]
    fn velocity_issues_count_centroid_inversions() {
        let a = dummy_result(0.2, 1000.0, 440.0, 440.0);
        let b = dummy_result(0.5, 2000.0, 440.0, 440.0);
        // Centroid falls back to 1500 from 2000 as velocity rises.
        let c = dummy_result(0.8, 1500.0, 440.0, 440.0);
        let steps = [
            velocity_step_from_analysis(40, &a),
            velocity_step_from_analysis(80, &b),
            velocity_step_from_analysis(120, &c),
        ];
        let issues = velocity_issues_from_steps(&steps);
        assert_eq!(issues.non_monotonic_centroid_steps, 1);
    }

    #[test]
    fn velocity_issues_flag_compressed_response_between_3_and_10_db() {
        // 0.3 vs 0.5 → 20*log10(0.5/0.3) ≈ 4.4 dB — in the compressed band.
        let a = dummy_result(0.3, 1000.0, 440.0, 440.0);
        let b = dummy_result(0.5, 1000.0, 440.0, 440.0);
        let steps = [
            velocity_step_from_analysis(40, &a),
            velocity_step_from_analysis(127, &b),
        ];
        let issues = velocity_issues_from_steps(&steps);
        assert!(!issues.velocity_unresponsive);
        assert!(issues.velocity_compressed_response);
    }

    #[test]
    fn velocity_issues_no_compressed_flag_for_responsive_patch() {
        // 0.05 vs 1.0 → 26 dB range — well past the 10 dB ceiling.
        let a = dummy_result(0.05, 1000.0, 440.0, 440.0);
        let b = dummy_result(1.0, 1000.0, 440.0, 440.0);
        let steps = [
            velocity_step_from_analysis(40, &a),
            velocity_step_from_analysis(127, &b),
        ];
        let issues = velocity_issues_from_steps(&steps);
        assert!(!issues.velocity_unresponsive);
        assert!(!issues.velocity_compressed_response);
    }

    #[test]
    fn velocity_issues_flag_brightness_inversion() {
        // Centroid drops monotonically with velocity, last < first.
        let a = dummy_result(0.2, 4000.0, 440.0, 440.0);
        let b = dummy_result(0.5, 3000.0, 440.0, 440.0);
        let c = dummy_result(0.8, 2000.0, 440.0, 440.0);
        let d = dummy_result(1.0, 1000.0, 440.0, 440.0);
        let steps = [
            velocity_step_from_analysis(16, &a),
            velocity_step_from_analysis(48, &b),
            velocity_step_from_analysis(80, &c),
            velocity_step_from_analysis(112, &d),
        ];
        let issues = velocity_issues_from_steps(&steps);
        assert_eq!(issues.non_monotonic_centroid_steps, 3);
        assert!(issues.velocity_brightness_inverted);
    }

    #[test]
    fn velocity_issues_brightness_not_flagged_when_endpoints_match() {
        // One mid-step dip but the last centroid is still ≥ the first.
        let a = dummy_result(0.2, 2000.0, 440.0, 440.0);
        let b = dummy_result(0.5, 1500.0, 440.0, 440.0);
        let c = dummy_result(0.8, 2500.0, 440.0, 440.0);
        let steps = [
            velocity_step_from_analysis(40, &a),
            velocity_step_from_analysis(80, &b),
            velocity_step_from_analysis(120, &c),
        ];
        let issues = velocity_issues_from_steps(&steps);
        assert!(!issues.velocity_brightness_inverted);
    }
}
