//! Integration test for the offline arrangement renderer
//! (`crate::audio::arrangement_render::render_arrangement_to_buffer`).
//!
//! Builds a real `SynthEngine` + `SynthSession`, installs a sustaining patch
//! on a single instrument, constructs an isolated `Song` with a four-note
//! pattern placed on a track, then renders a tick range covering the pattern
//! and asserts that:
//!
//! - The buffer length matches the wall-clock duration of the tick range.
//! - The rendered audio is not silent (RMS comfortably above the noise floor).
//! - `analyze_mix_buffer` reports a finite LUFS reading and a sane peak.
//! - An empty arrangement (no placements) produces a quiet buffer rather
//!   than panicking.
//!
//! This is the smoke test for the full `engine snapshot → offline render →
//! analyze` pipeline that the `analyze_mix_bus` and `analyze_section` MCP
//! tools depend on.

use std::sync::Arc;

use parking_lot::RwLock;

use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor, ModuleType};
use synth_engine::SynthEngine;
use synth_engine::instrument::InstrumentId;
use synth_sequencer::{
    Duration as SeqDuration, PatternTick, Pitch, SeqInstrumentId, Song, Tick, Velocity,
};

use pertylizer::audio::arrangement_render::render_arrangement_to_buffer;
use pertylizer::audio::mix_analysis::analyze_mix_buffer;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::patch::{ModuleBuilder, Patch};
use pertylizer::session::SynthSession;

const TEST_SR: u32 = 44_100;

/// Minimal sustaining patch — same shape as `preview_integration.rs`'s
/// `sustain_patch_no_envelope` so we can rely on it producing audible output
/// when a note is held. Sawtooth oscillator → amp (envelope-gated) → output.
fn sustain_patch() -> Patch {
    let mut patch = Patch::new("ArrangementRenderTest");
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sawtooth")
            .param_f("level", 0.5)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Envelope)
            .param_f("attack", 0.005)
            .param_f("decay", 0.0)
            .param_f("sustain", 1.0)
            .param_f("release", 0.05)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Amplifier)
            .param_f("level", 1.0)
            .build(),
    );
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::StereoOutput)
            .param_f("master", 1.0)
            .build(),
    );
    patch.add_connection("osc-1", "out", "amp-1", "in");
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
    patch
}

/// Engine + session set up with one instrument, drained so the shared graph
/// snapshot reflects the patch.
struct Rig {
    _engine: SynthEngine,
    _handle: synth_engine::EngineHandle,
    session: SynthSession,
}

fn setup_with_patch(patch: &Patch) -> Rig {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    session
        .add_instrument_with_id(InstrumentId::FIRST, "Test")
        .expect("add instrument");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(TEST_SR),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let context = AudioCallbackContext {
        sample_rate: HwSampleRate(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    engine.process(&mut block, &context);

    let _ = session.apply_patch(InstrumentId::FIRST, patch);

    // Drain so add_module / connect / set_param commands land in shared state.
    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    Rig {
        _engine: engine,
        _handle: handle,
        session,
    }
}

/// Build a song with one 4-beat pattern playing a C major arpeggio on the
/// default instrument (SeqInstrumentId(0) ↔ InstrumentId::FIRST), placed on a
/// single track at tick 0. At the default 120 BPM this spans exactly 2 s.
fn build_arpeggio_song() -> Arc<RwLock<Song>> {
    let mut song = Song::new("Arpeggio");
    // Whole-note pattern = 4 × 960 = 3840 ticks at 4/4.
    let pattern_id = song.create_pattern(SeqDuration::WHOLE);
    {
        let pattern = song
            .pattern_mut(pattern_id)
            .expect("pattern just created should exist");
        let pitches = [60u8, 64, 67, 72];
        for (i, midi) in pitches.iter().enumerate() {
            let start = PatternTick(i as u32 * 960);
            let nid = pattern.add_note(
                start,
                Pitch::new(*midi).expect("valid MIDI note"),
                Velocity::MF,
                SeqInstrumentId(0),
            );
            // Each note holds for almost the whole beat so the renderer has
            // sustained content to measure on, then releases before the next.
            if let Some(note) = pattern.note_mut(nid) {
                note.duration = Some(SeqDuration(900));
            }
        }
    }

    let track_id = song.create_track("T1");
    if let Some(track) = song.track_mut(track_id) {
        track.instrument = Some(SeqInstrumentId(0));
    }
    assert!(
        song.place_pattern(pattern_id, track_id, Tick(0)),
        "place_pattern should succeed"
    );

    Arc::new(RwLock::new(song))
}

/// RMS of a stereo-interleaved buffer's left channel.
fn left_rms(stereo: &[f32]) -> f32 {
    let lefts: Vec<f32> = stereo.chunks_exact(2).map(|f| f[0]).collect();
    if lefts.is_empty() {
        return 0.0;
    }
    let sq: f32 = lefts.iter().map(|s| s * s).sum();
    (sq / lefts.len() as f32).sqrt()
}

#[test]
fn renders_audible_arrangement_from_engine_snapshot() {
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    // 2 s at 120 BPM = 4 quarter notes = 3840 ticks.
    let start_tick = 0u64;
    let end_tick = 3840u64;

    let rendered = render_arrangement_to_buffer(&rig.session, &shared, start_tick, end_tick)
        .expect("arrangement render should succeed");

    // Sanity: buffer covers ~2 s at 44.1 kHz stereo.
    let frame_count = rendered.samples.len() / 2;
    let expected_frames = (rendered.duration_seconds * rendered.sample_rate as f32) as usize;
    assert!(
        frame_count.abs_diff(expected_frames) <= 1,
        "buffer length {frame_count} frames doesn't match expected {expected_frames}"
    );
    assert_eq!(rendered.start_tick, start_tick);
    assert_eq!(rendered.end_tick, end_tick);
    assert_eq!(rendered.sample_rate, TEST_SR);
    assert_eq!(rendered.channels, 2);
    assert!(
        rendered.duration_seconds > 1.8 && rendered.duration_seconds < 2.2,
        "expected ~2s render, got {} s",
        rendered.duration_seconds
    );

    // The patch is a sawtooth at full level — the rendered window must be
    // far above the silence floor. Empirically the arpeggio sits around
    // RMS ≈ 0.1; assert a much lower threshold so the test stays robust.
    let rms = left_rms(&rendered.samples);
    assert!(
        rms > 0.01,
        "rendered arrangement should be audible (left RMS = {rms})"
    );

    // No warnings on a clean render of a single-instrument song.
    assert!(
        rendered.warnings.is_empty(),
        "clean render should produce no warnings, got {:?}",
        rendered.warnings
    );

    // Run the mix-bus analyzer on the result; it should produce a finite
    // LUFS reading and a plausible peak/RMS.
    let analysis = analyze_mix_buffer(&rendered.samples, rendered.sample_rate);
    assert!(
        analysis.peak > 0.05 && analysis.peak <= 1.5,
        "peak amplitude should be audible and bounded, got {}",
        analysis.peak
    );
    assert!(
        analysis.rms > 0.005,
        "analyzer RMS should match audible content, got {}",
        analysis.rms
    );
    assert!(
        analysis.lufs_integrated > -50.0 && analysis.lufs_integrated < 0.0,
        "LUFS-I should fall in a plausible range, got {}",
        analysis.lufs_integrated
    );
    assert!(
        analysis.stereo_correlation > 0.95,
        "mono-routed patch should produce highly correlated stereo, got {}",
        analysis.stereo_correlation
    );
    assert!(
        analysis.mono_compat > 0.95,
        "mono-routed patch should be near-perfectly mono-compatible, got {}",
        analysis.mono_compat
    );
}

#[test]
fn empty_arrangement_renders_silently_without_error() {
    // Same engine + patch, but the song has no tracks or placements.
    let rig = setup_with_patch(&sustain_patch());
    let song = Arc::new(RwLock::new(Song::new("Empty")));
    let shared = McpSharedState::with_song(song);

    let rendered = render_arrangement_to_buffer(&rig.session, &shared, 0, 3840)
        .expect("empty-arrangement render should succeed");

    // The buffer should be silent (or very nearly so) — no notes were
    // played. We allow a tiny non-zero RMS for engine startup transients
    // and pre-allocated buffer noise, but it should be far below an
    // audible threshold.
    let rms = left_rms(&rendered.samples);
    assert!(
        rms < 0.01,
        "empty arrangement should render near-silence, got RMS = {rms}"
    );
}

#[test]
fn render_rejects_inverted_range() {
    let rig = setup_with_patch(&sustain_patch());
    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    // end_tick <= start_tick should return an explicit error instead of
    // panicking or producing a zero-length buffer.
    let result = render_arrangement_to_buffer(&rig.session, &shared, 3840, 3840);
    assert!(result.is_err(), "zero-width range should fail");

    let result = render_arrangement_to_buffer(&rig.session, &shared, 3840, 0);
    assert!(result.is_err(), "inverted range should fail");
}

#[test]
fn render_fails_when_no_instruments_loaded() {
    // Build an engine + session but never add any instruments.
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(TEST_SR),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let song = build_arpeggio_song();
    let shared = McpSharedState::with_song(song);

    let result = render_arrangement_to_buffer(&session, &shared, 0, 3840);
    assert!(
        result.is_err(),
        "render should fail when no instruments are loaded"
    );
}
