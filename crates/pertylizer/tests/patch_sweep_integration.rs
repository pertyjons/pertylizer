//! Integration tests for `analyze_instrument_range` and
//! `analyze_velocity_response`. Builds a real `SynthEngine` + `SynthSession`,
//! installs a sustaining patch, then calls the bridge impls directly.

use synth_core::MidiNote;
use synth_engine::instrument::InstrumentId;

use pertylizer::mcp_bridge::{analyze_instrument_range_impl, analyze_velocity_response_impl};

mod common;
use common::{setup_with_patch, sustain_patch};

#[test]
fn analyze_instrument_range_returns_one_step_per_swept_note() {
    let rig = setup_with_patch(&sustain_patch());
    let result = analyze_instrument_range_impl(
        &rig.session,
        &rig.sample_library,
        InstrumentId::FIRST,
        48, // C3
        72, // C5
        Some(12),
        Some(100),
        Some(200),
        Some(100),
    )
    .expect("instrument range sweep");
    assert_eq!(result.steps.len(), 3, "C3 / C4 / C5");
    let notes: Vec<u8> = result.steps.iter().map(|s| s.note.as_u8()).collect();
    assert_eq!(notes, vec![48, 60, 72]);
    assert_eq!(result.low_note, 48);
    assert_eq!(result.high_note, 72);
    assert_eq!(result.step_semitones, 12);
    for step in &result.steps {
        assert!(
            !step.silent,
            "step at MIDI {} unexpectedly silent (peak {})",
            step.note, step.peak_amplitude
        );
    }
    assert!(
        result.issues.silent_notes.is_empty(),
        "issues.silent_notes should be empty for a healthy patch: {:?}",
        result.issues.silent_notes
    );
}

#[test]
fn analyze_instrument_range_rejects_inverted_range() {
    let rig = setup_with_patch(&sustain_patch());
    let err = analyze_instrument_range_impl(
        &rig.session,
        &rig.sample_library,
        InstrumentId::FIRST,
        72,
        48,
        Some(12),
        Some(100),
        Some(200),
        Some(100),
    )
    .expect_err("inverted range should error");
    let msg = err.to_string();
    assert!(
        msg.contains("low_note") && msg.contains("high_note"),
        "error should mention low_note/high_note: {msg}"
    );
}

#[test]
fn analyze_velocity_response_curve_rises_with_velocity() {
    let rig = setup_with_patch(&sustain_patch());
    let result = analyze_velocity_response_impl(
        &rig.session,
        &rig.sample_library,
        InstrumentId::FIRST,
        MidiNote::new(60),
        Some(32),
        Some(127),
        Some(32),
        Some(200),
        Some(100),
    )
    .expect("velocity sweep");
    // sweep_range clamps the final step to the upper bound, so 32, 64, 96, 127.
    let velocities: Vec<u8> = result.steps.iter().map(|s| s.velocity).collect();
    assert_eq!(velocities, vec![32, 64, 96, 127]);
    let peaks: Vec<f32> = result.steps.iter().map(|s| s.peak_amplitude).collect();
    assert!(
        peaks.windows(2).all(|w| w[1] + 1e-4 >= w[0]),
        "expected non-decreasing peaks across velocity sweep, got {peaks:?}"
    );
    assert_eq!(
        result.issues.non_monotonic_amplitude_steps, 0,
        "no inversions expected on velocity-sensitive patch"
    );
    assert!(
        !result.issues.velocity_unresponsive,
        "expected patch to be responsive (range_db = {})",
        result.issues.amplitude_range_db
    );
}

#[test]
fn analyze_velocity_response_inclusive_upper_bound() {
    let rig = setup_with_patch(&sustain_patch());
    let result = analyze_velocity_response_impl(
        &rig.session,
        &rig.sample_library,
        InstrumentId::FIRST,
        MidiNote::new(60),
        Some(100),
        Some(127),
        Some(27),
        Some(200),
        Some(100),
    )
    .expect("velocity sweep");
    let velocities: Vec<u8> = result.steps.iter().map(|s| s.velocity).collect();
    assert_eq!(velocities, vec![100, 127]);
}

#[test]
fn analyze_velocity_response_rejects_inverted_range() {
    let rig = setup_with_patch(&sustain_patch());
    let err = analyze_velocity_response_impl(
        &rig.session,
        &rig.sample_library,
        InstrumentId::FIRST,
        MidiNote::new(60),
        Some(127),
        Some(32),
        Some(16),
        Some(200),
        Some(100),
    )
    .expect_err("inverted velocity range should error");
    let msg = err.to_string();
    assert!(
        msg.contains("velocity_low") && msg.contains("velocity_high"),
        "error should mention velocity_low/velocity_high: {msg}"
    );
}
