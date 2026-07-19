//! Integration tests for the `render_to_wav` MCP tool
//! (`pertylizer::mcp_bridge::render_to_wav_impl`).
//!
//! Builds a real engine + session, installs a sustaining patch, then renders an
//! arrangement window to a 32-bit float WAV on disk and decodes it back with
//! `hound` to assert it is a valid, non-silent file with the expected sample
//! rate / channel count / length. Also covers the instrument-solo path's
//! warning when the requested instrument drives no track.

mod common;

use synth_core::InstrumentId;
use synth_mcp::AnalysisScope;

use pertylizer::mcp_bridge::render_to_wav_impl;
use pertylizer::mcp_shared::McpSharedState;

use common::{TEST_SR, build_arpeggio_song, setup_with_patch, sustain_patch};

#[test]
fn render_to_wav_writes_readable_wav() {
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("render.wav");

    let result = render_to_wav_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        path.to_string_lossy().into_owned(),
        2.0,
        Some(synth_sequencer::Tick(0)),
        None,
        AnalysisScope::default(),
    )
    .expect("render_to_wav should succeed");

    // Result fields describe the written file.
    assert_eq!(result.sample_rate, TEST_SR);
    assert_eq!(result.channels, 2);
    assert_eq!(result.soloed_instrument_id, None);
    assert!(
        result.warnings.is_empty(),
        "got warnings: {:?}",
        result.warnings
    );
    assert!(
        result.duration_seconds > 1.8 && result.duration_seconds < 2.2,
        "expected ~2s render, got {} s",
        result.duration_seconds
    );
    assert!(
        result.peak > 0.05 && result.peak <= 1.5,
        "rendered audio should be audible and bounded, peak = {}",
        result.peak
    );

    // The file exists and decodes as a valid WAV with matching metadata.
    assert!(path.exists(), "WAV file should exist on disk");
    let reader = hound::WavReader::open(&path).expect("WAV should be readable");
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, TEST_SR);
    assert_eq!(spec.channels, 2);
    assert_eq!(spec.bits_per_sample, 32);
    assert_eq!(spec.sample_format, hound::SampleFormat::Float);

    // Frame count (samples per channel) matches the reported duration and the
    // result's `frames` field.
    let total_samples = reader.len() as u64;
    let frames = total_samples / u64::from(spec.channels);
    assert_eq!(frames, result.frames, "result.frames must match the file");
    let expected_frames = (result.duration_seconds * spec.sample_rate as f32) as u64;
    assert!(
        frames.abs_diff(expected_frames) <= 1,
        "WAV length {frames} frames doesn't match expected {expected_frames}"
    );

    // And it is genuinely non-silent.
    let peak = hound::WavReader::open(&path)
        .expect("reopen")
        .samples::<f32>()
        .map(|s| s.expect("sample").abs())
        .fold(0.0_f32, f32::max);
    assert!(
        peak > 0.05,
        "decoded WAV should be non-silent, peak = {peak}"
    );
}

#[test]
fn render_to_wav_unknown_instrument_warns_and_is_silent() {
    // Soloing an instrument that drives no track must not error — it warns and
    // writes a silent (but valid) WAV.
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("solo.wav");

    let result = render_to_wav_impl(
        &rig.session,
        &rig.sample_library,
        &shared,
        path.to_string_lossy().into_owned(),
        1.0,
        Some(synth_sequencer::Tick(0)),
        Some(InstrumentId::new(99)), // no track uses instrument 99
        AnalysisScope::default(),
    )
    .expect("render_to_wav should succeed even with no matching instrument");

    assert_eq!(result.soloed_instrument_id, Some(InstrumentId::new(99)));
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("drives no track")),
        "expected a 'drives no track' warning, got {:?}",
        result.warnings
    );
    assert!(
        result.peak < 0.01,
        "soloing an unused instrument should render near-silence, peak = {}",
        result.peak
    );
    assert!(path.exists(), "a valid WAV should still be written");
}
