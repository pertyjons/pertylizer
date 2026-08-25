//! Shared fixtures for integration tests.
//!
//! Each integration test is its own binary, so common setup needs to live in
//! a `mod common;` submodule rather than a regular crate. Cargo treats files
//! under `tests/common/` (with `mod.rs`) as a module rather than as their
//! own test target.

#![allow(dead_code)] // not every consumer uses every helper

use std::path::PathBuf;
use std::sync::Arc;

use synth_core::audio::DeviceSampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor, ModuleType};
use synth_engine::SynthEngine;
use synth_sequencer::{
    AutomationPoint, AutomationTarget, Duration as SeqDuration, InstrumentId, PatternId,
    PatternTick, Pitch, Song, Tick, TrackId, Velocity,
};

use pertylizer::patch::{ModuleBuilder, Patch};
use pertylizer::session::SynthSession;

pub const TEST_SR: u32 = 44_100;

/// Append env-1 (snappy ADSR), amp-1, and out-1 to `patch`, plus the three
/// connections that wire the amp envelope and stereo output. The caller is
/// expected to have added a source module and connected its output to
/// `amp-1.in` beforehand.
pub fn add_env_amp_out_tail(patch: &mut Patch) {
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
    patch.add_connection("env-1", "out", "amp-1", "cv");
    patch.add_connection("amp-1", "left", "out-1", "in_l");
    patch.add_connection("amp-1", "right", "out-1", "in_r");
}

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
    add_env_amp_out_tail(&mut patch);
    patch.add_connection("osc-1", "out", "amp-1", "in");
    patch
}

pub struct Rig {
    pub _engine: SynthEngine,
    pub _handle: synth_engine::EngineHandle,
    pub session: SynthSession,
    pub sample_library: pertylizer::audio::preview::SharedSampleLibrary,
}

pub fn setup_with_patch(patch: &Patch) -> Rig {
    let (mut engine, handle) = SynthEngine::new();
    let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
    session
        .add_instrument_with_id(InstrumentId::FIRST, "Test")
        .expect("add instrument");

    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate::new(TEST_SR),
        buffer_size: synth_core::BufferSize::new(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let context = AudioCallbackContext {
        sample_rate: HwSampleRate::new(TEST_SR),
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
        sample_library: Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        )),
    }
}

/// Build a single-pattern, single-track song on `InstrumentId::new(0)` placed
/// at tick 0. `notes` lists `(start, pitch_midi, duration)` triples; the
/// pattern length is `SeqDuration::WHOLE` (3840 ticks = 2 s at 120 BPM).
pub fn build_single_pattern_song(
    name: &str,
    notes: &[(PatternTick, u8, SeqDuration)],
) -> Arc<synth_sequencer::SharedSong> {
    let mut song = Song::new(name);
    let pattern_id = song.create_pattern(SeqDuration::WHOLE);
    {
        let pattern = song
            .pattern_mut(pattern_id)
            .expect("pattern just created should exist");
        for (start, midi, duration) in notes {
            let nid = pattern.add_note(
                *start,
                Pitch::new(*midi).expect("valid MIDI note"),
                Velocity::MF,
            );
            if let Some(note) = pattern.note_mut(nid) {
                note.duration = Some(*duration);
            }
        }
    }

    let track_id = song.create_track("T1");
    if let Some(track) = song.track_mut(track_id) {
        track.instrument = InstrumentId::new(0);
    }
    assert!(
        song.place_pattern(pattern_id, track_id, Tick(0)),
        "place_pattern should succeed"
    );

    Arc::new(synth_sequencer::SharedSong::new(song))
}

/// One-bar 4-note C-major arpeggio on `InstrumentId::new(0)`, placed at tick 0.
/// At 120 BPM the pattern spans exactly 2 s (3840 ticks).
pub fn build_arpeggio_song() -> Arc<synth_sequencer::SharedSong> {
    let notes: Vec<_> = [60u8, 64, 67, 72]
        .iter()
        .enumerate()
        .map(|(i, midi)| (PatternTick(i as u32 * 960), *midi, SeqDuration(900)))
        .collect();
    build_single_pattern_song("Arpeggio", &notes)
}

/// Count sign changes in a stereo-interleaved buffer's left channel — a cheap
/// fundamental-frequency proxy for a simple periodic patch.
pub fn left_zero_crossings(stereo: &[f32]) -> usize {
    let mut frames = stereo.as_chunks::<2>().0.iter().map(|f| f[0]);
    let Some(mut prev) = frames.next() else {
        return 0;
    };
    let mut crossings = 0;
    for s in frames {
        if (s > 0.0 && prev <= 0.0) || (s < 0.0 && prev >= 0.0) {
            crossings += 1;
        }
        prev = s;
    }
    crossings
}

/// RMS of a stereo-interleaved buffer's left channel.
pub fn left_rms(stereo: &[f32]) -> f32 {
    let lefts: Vec<f32> = stereo.as_chunks::<2>().0.iter().map(|f| f[0]).collect();
    if lefts.is_empty() {
        return 0.0;
    }
    let sq: f32 = lefts.iter().map(|s| s * s).sum();
    (sq / lefts.len() as f32).sqrt()
}

/// RMS of a stereo-interleaved buffer's right channel.
pub fn right_rms(stereo: &[f32]) -> f32 {
    let rights: Vec<f32> = stereo.as_chunks::<2>().0.iter().map(|f| f[1]).collect();
    if rights.is_empty() {
        return 0.0;
    }
    let sq: f32 = rights.iter().map(|s| s * s).sum();
    (sq / rights.len() as f32).sqrt()
}

/// Build a song with one note sustained across the whole pattern on track 0,
/// returning the song plus its `PatternId` and `TrackId` so tests can attach
/// automation lanes. Steady amplitude (absent automation) makes RMS-over-time
/// comparisons meaningful.
pub fn build_sustained_note_song(
    name: &str,
) -> (Arc<synth_sequencer::SharedSong>, PatternId, TrackId) {
    let mut song = Song::new(name);
    let pattern_id = song.create_pattern(SeqDuration::WHOLE);
    {
        let pattern = song
            .pattern_mut(pattern_id)
            .expect("pattern just created should exist");
        let nid = pattern.add_note(
            PatternTick(0),
            Pitch::new(60).expect("valid MIDI note"),
            Velocity::MF,
        );
        if let Some(note) = pattern.note_mut(nid) {
            note.duration = Some(SeqDuration::WHOLE);
        }
    }
    let track_id = song.create_track("T1");
    if let Some(track) = song.track_mut(track_id) {
        track.instrument = InstrumentId::new(0);
    }
    assert!(
        song.place_pattern(pattern_id, track_id, Tick(0)),
        "place_pattern should succeed"
    );
    (
        Arc::new(synth_sequencer::SharedSong::new(song)),
        pattern_id,
        track_id,
    )
}

/// Attach a two-point linear ramp (`from` → `to` across the 3840-tick pattern)
/// for `target` to the pattern's automation lanes.
pub fn add_ramp_automation(
    song: &Arc<synth_sequencer::SharedSong>,
    pattern_id: PatternId,
    target: AutomationTarget,
    from: f32,
    to: f32,
) {
    let mut song_w = song.write();
    let lane = song_w
        .pattern_mut(pattern_id)
        .expect("pattern should exist")
        .get_or_create_automation(target);
    lane.add_point(AutomationPoint::new(
        PatternTick(0),
        synth_core::NormalizedValue::new(from),
    ));
    lane.add_point(AutomationPoint::new(
        PatternTick(3840),
        synth_core::NormalizedValue::new(to),
    ));
}

/// Set the first track's volume and pan on a shared song (test-side write
/// lock). Track faders are read live from the `Song` during render, so no
/// engine drain is needed after calling this.
pub fn set_first_track_fader(song: &Arc<synth_sequencer::SharedSong>, volume: f32, pan: f32) {
    let track_id = song.read().tracks().next().expect("song has a track").id;
    let mut song_w = song.write();
    if let Some(track) = song_w.track_mut(track_id) {
        track.volume = synth_core::NormalizedValue::new(volume);
        track.pan = synth_core::BipolarValue::new(pan);
    }
}

/// Drive `engine` for `blocks` callbacks of 256 stereo frames each. Used to
/// flush queued `EngineCommand`s and refresh the shared `instrument_snapshots`
/// (volume/pan/etc.) that an offline render reads at construction time.
pub fn process_block(engine: &mut SynthEngine, blocks: usize) {
    let mut block = vec![0.0f32; 256 * 2];
    let context = AudioCallbackContext {
        sample_rate: HwSampleRate::new(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    for _ in 0..blocks {
        block.fill(0.0);
        engine.process(&mut block, &context);
    }
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

/// Panic with a sample-index/frame/timestamp report if `a` and `b` are not
/// bit-exact equal. `label` distinguishes the comparison in test output.
pub fn assert_bit_exact(label: &str, a: &[f32], b: &[f32], sample_rate: u32) {
    if let Some(idx) = first_divergence(a, b) {
        let frame = idx / 2;
        let seconds = frame as f32 / sample_rate as f32;
        let (x, y) = (a[idx], b[idx]);
        panic!(
            "{label} diverge at sample {idx} (frame {frame}, t={seconds:.4}s): \
             {x} vs {y} (bits: {:#x} vs {:#x})",
            x.to_bits(),
            y.to_bits()
        );
    }
}

/// Workspace-relative path to the bundled example project directory.
pub fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .join("assets/examples/projects")
}

/// Every bundled example project, sorted by path. Accepts both plain
/// `.ptz` projects and `.zip` bundles (named `<name>.ptz.zip`).
pub fn list_example_projects() -> Vec<PathBuf> {
    let dir = examples_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e == "ptz" || e == "zip")
        })
        .collect();
    files.sort();
    files
}
