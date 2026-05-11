//! Shared fixtures for `arrangement_render_*` integration tests.
//!
//! Each integration test is its own binary, so common setup needs to live in
//! a `mod common;` submodule rather than a regular crate. Cargo treats files
//! under `tests/common/` (with `mod.rs`) as a module rather than as their
//! own test target.

#![allow(dead_code)] // not every consumer uses every helper

use std::sync::Arc;

use parking_lot::RwLock;

use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor, ModuleType};
use synth_engine::SynthEngine;
use synth_engine::instrument::InstrumentId;
use synth_sequencer::{
    Duration as SeqDuration, PatternTick, Pitch, SeqInstrumentId, Song, Tick, Velocity,
};

use pertylizer::patch::{ModuleBuilder, Patch};
use pertylizer::session::SynthSession;

pub const TEST_SR: u32 = 44_100;

/// Minimal sustaining patch: sawtooth → amp (envelope-gated) → output.
/// Produces audible content when a note is held — used as the default
/// patch for arrangement-render smoke and determinism tests.
pub fn sustain_patch() -> Patch {
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

pub struct Rig {
    pub _engine: SynthEngine,
    pub _handle: synth_engine::EngineHandle,
    pub session: SynthSession,
}

pub fn setup_with_patch(patch: &Patch) -> Rig {
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

/// One-bar 4-note C-major arpeggio on `SeqInstrumentId(0)`, placed at tick 0.
/// At 120 BPM the pattern spans exactly 2 s (3840 ticks).
pub fn build_arpeggio_song() -> Arc<RwLock<Song>> {
    let mut song = Song::new("Arpeggio");
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
pub fn left_rms(stereo: &[f32]) -> f32 {
    let lefts: Vec<f32> = stereo.chunks_exact(2).map(|f| f[0]).collect();
    if lefts.is_empty() {
        return 0.0;
    }
    let sq: f32 = lefts.iter().map(|s| s * s).sum();
    (sq / lefts.len() as f32).sqrt()
}

/// First sample index at which two equal-length stereo buffers diverge
/// bit-exactly. Panics if the lengths differ — equal-length is part of the
/// determinism contract.
pub fn first_divergence(a: &[f32], b: &[f32]) -> Option<usize> {
    assert_eq!(
        a.len(),
        b.len(),
        "first_divergence requires equal-length buffers (left={}, right={})",
        a.len(),
        b.len()
    );
    a.iter()
        .zip(b.iter())
        .position(|(x, y)| x.to_bits() != y.to_bits())
}
