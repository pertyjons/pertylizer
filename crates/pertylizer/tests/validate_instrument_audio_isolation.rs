//! Regression test for MCP feedback (2026-07-20): offline instrument-audio
//! validation must not leak mixer solo/mute state into the live project, even
//! under concurrent calls.
//!
//! `validate_instrument_audio` delegates to `analyze_note`, which renders on a
//! fresh, fully isolated engine (never the live one) and only *reads* the shared
//! song. This test pins that guarantee: it establishes a known track mixer state
//! (one soloed, one muted track), runs several validations concurrently —
//! including one that errors on an unknown instrument — and asserts every
//! track's solo/mute flag is byte-for-byte unchanged afterward.

#![cfg(feature = "mcp")]

mod common;

use std::sync::Arc;

use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioProcessor, MidiNote, ModuleType};
use synth_engine::SynthEngine;
use synth_sequencer::{InstrumentId, Song, TrackId};

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::patch::{ModuleBuilder, Patch};
use pertylizer::session::SynthSession;
use synth_mcp::SynthBridge;

use common::TEST_SR;

/// Minimal audible patch: osc → amp → out, with an envelope gating the amp so a
/// rendered note actually produces signal.
fn simple_patch(name: &str) -> Patch {
    let mut patch = Patch::new(name);
    patch.add_module(
        ModuleBuilder::new(1, ModuleType::Oscillator)
            .waveform("sine")
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

/// Snapshot every track's `(id, solo, muted)` for byte-for-byte comparison.
fn mixer_flags(bridge: &AppSynthBridge) -> Vec<(TrackId, bool, bool)> {
    let mut flags: Vec<(TrackId, bool, bool)> = bridge
        .list_tracks()
        .expect("list tracks")
        .iter()
        .map(|t| (t.id, t.solo, t.mute))
        .collect();
    flags.sort_by_key(|(id, _, _)| id.0);
    flags
}

#[test]
fn concurrent_validation_preserves_track_mixer_flags() {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    for (id, name) in [(0u64, "Timpani"), (1, "Taiko"), (2, "Brass")] {
        session
            .add_instrument_with_id(InstrumentId::new(id), name)
            .expect("add instrument");
    }

    // Mirror the applied patches into the live engine's shared_graph (the audio
    // thread would normally do this) by processing a few blocks, so the offline
    // validator sees real module graphs to render.
    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate::new(TEST_SR),
        buffer_size: synth_core::BufferSize::new(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);
    let context = synth_core::AudioCallbackContext {
        sample_rate: HwSampleRate::new(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    let mut block = vec![0.0f32; 256 * 2];
    engine.process(&mut block, &context);
    let _ = session.apply_patch(InstrumentId::new(0), &simple_patch("Timpani"));
    let _ = session.apply_patch(InstrumentId::new(1), &simple_patch("Taiko"));
    let _ = session.apply_patch(InstrumentId::new(2), &simple_patch("Brass"));
    for _ in 0..16 {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }

    // Song: three tracks bound to the three instruments.
    let mut song = Song::new("Nemesis-ish");
    let t0 = song.create_track("Timpani");
    let t1 = song.create_track("Taiko");
    let t2 = song.create_track("Brass");
    for (tid, iid) in [(t0, 0u64), (t1, 1), (t2, 2)] {
        song.track_mut(tid).expect("track").instrument = InstrumentId::new(iid);
    }

    let shared_song = Arc::new(synth_sequencer::SharedSong::new(song));
    let _ = handle
        .command_sender()
        .send(synth_engine::EngineCommand::SetSong {
            song: Arc::clone(&shared_song),
        });
    let shared = Arc::new(McpSharedState::with_song(Arc::clone(&shared_song)));
    let sample_library = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    let bridge = AppSynthBridge::new(Arc::new(session), shared, sample_library);

    // Establish a known, non-default mixer state through the public API.
    bridge.set_track_solo(t0, true).expect("solo timpani");
    bridge.set_track_mute(t1, true).expect("mute taiko");

    let before = mixer_flags(&bridge);
    assert!(
        before.contains(&(t0, true, false)),
        "precondition: timpani soloed: {before:?}"
    );
    assert!(
        before.contains(&(t1, false, true)),
        "precondition: taiko muted: {before:?}"
    );

    // Validate several instruments at once, including an unknown id so the
    // error path is covered too ("even on errors and concurrent calls").
    let outcomes: std::sync::Mutex<Vec<(u64, bool)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for id in [0u64, 1, 2, 0, 1, 2, 999] {
            let bridge = &bridge;
            let outcomes = &outcomes;
            s.spawn(move || {
                let ok = bridge
                    .analyze_note(
                        InstrumentId::new(id),
                        MidiNote::C4,
                        100,
                        200,
                        100,
                        None,
                        None,
                    )
                    .is_ok();
                outcomes.lock().expect("outcomes lock").push((id, ok));
            });
        }
    });

    // Sanity-check that the real render path (not just the error path) ran: the
    // three built instruments must validate successfully, the unknown id must
    // fail. Otherwise the isolation assertion below would be vacuous.
    let outcomes = outcomes.into_inner().expect("outcomes");
    for id in [0u64, 1, 2] {
        assert!(
            outcomes.iter().any(|&(oid, ok)| oid == id && ok),
            "instrument {id} should validate (render path exercised): {outcomes:?}"
        );
    }
    assert!(
        outcomes.iter().any(|&(oid, ok)| oid == 999 && !ok),
        "unknown instrument must fail: {outcomes:?}"
    );

    let after = mixer_flags(&bridge);
    assert_eq!(
        before, after,
        "concurrent validate_instrument_audio must not change any track's mixer flags"
    );
    // t2 was never touched and must remain fully default.
    assert!(
        after.contains(&(t2, false, false)),
        "brass untouched: {after:?}"
    );
}
