//! Headless MCP-bridge tests for the project I/O refactor
//! (Phase 0.3 of the `project-io-off-gui-thread` plan).
//!
//! These exercise `AppSynthBridge` directly — without an egui app — to:
//!   1. Prove the bridge can be constructed and that **non**-project-IO
//!      operations work today (set_song_tempo etc. don't go through the
//!      GUI handoff and so should succeed in any context).
//!   2. Document the bug we're fixing: `load_project` / `save_project` /
//!      `new_project` route through `submit_project_action`, which blocks
//!      on a condvar until the GUI drains `pending_project_action`. With
//!      no GUI present, the call hangs until the 30 s timeout. Those
//!      tests are `#[ignore]`d on purpose — un-ignore them after Phase 3
//!      lands the apply-on-MCP-thread refactor.

#![cfg(feature = "mcp")]

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock as PlRwLock;
use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor};
use synth_engine::SynthEngine;
use synth_mcp::bridge::SynthBridge;
use synth_sequencer::Song;

use pertylizer::mcp_bridge::AppSynthBridge;
use pertylizer::mcp_shared::McpSharedState;
use pertylizer::session::SynthSession;

const TEST_SR: u32 = 44_100;

struct HeadlessRig {
    bridge: AppSynthBridge,
    /// Engine + block + ctx so tests can pump it to flush command queues.
    engine: SynthEngine,
    block: Vec<f32>,
    ctx: AudioCallbackContext,
    _handle: synth_engine::EngineHandle,
    /// Shared Song so the bridge and engine see the same data.
    song: Arc<PlRwLock<Song>>,
    _shared: Arc<McpSharedState>,
    _session: Arc<SynthSession>,
    _sample_library: Arc<std::sync::RwLock<synth_sampler::SampleLibrary>>,
}

impl HeadlessRig {
    /// Drive `n` audio blocks through the engine so command queues drain
    /// into shared state.
    fn pump(&mut self, n: usize) {
        for _ in 0..n {
            self.block.fill(0.0);
            self.engine.process(&mut self.block, &self.ctx);
        }
    }
}

fn build_headless_rig() -> HeadlessRig {
    let (mut engine, handle) = SynthEngine::new();

    let song = Arc::new(PlRwLock::new(Song::new("Headless")));
    let _ = handle
        .command_sender()
        .send(synth_engine::EngineCommand::SetSong {
            song: Arc::clone(&song),
        });

    let session = Arc::new(SynthSession::new(
        handle.command_sender(),
        Arc::clone(&handle.state),
    ));
    let sample_library = Arc::new(std::sync::RwLock::new(
        synth_sampler::SampleLibrary::default(),
    ));
    let shared = Arc::new(McpSharedState::with_song(Arc::clone(&song)));

    let bridge = AppSynthBridge::new(
        Arc::clone(&session),
        Arc::clone(&shared),
        Arc::clone(&sample_library),
    );

    // Drive a process tick so SetSong is consumed and any startup commands
    // settle. Mirrors how main.rs warms the engine before the first MCP call.
    let stream_info = synth_core::StreamInfo {
        sample_rate: HwSampleRate(TEST_SR),
        buffer_size: synth_core::BufferSize(256),
        channels: synth_core::ChannelCount::Stereo,
        output_latency: std::time::Duration::ZERO,
        input_latency: None,
    };
    engine.on_stream_start(&stream_info);

    let mut block = vec![0.0f32; 256 * 2];
    let ctx = AudioCallbackContext {
        sample_rate: HwSampleRate(TEST_SR),
        frames: 256,
        channels: 2,
        stream_time: 0.0,
        sample_position: 0,
        output_latency: synth_core::Seconds::ZERO,
    };
    for _ in 0..4 {
        block.fill(0.0);
        engine.process(&mut block, &ctx);
    }

    HeadlessRig {
        bridge,
        engine,
        block,
        ctx,
        _handle: handle,
        song,
        _shared: shared,
        _session: session,
        _sample_library: sample_library,
    }
}

fn example_project_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("workspace root")
        .join("assets/examples/projects/Neuro F#m 174.json")
}

// ---------------------------------------------------------------------------
// Baseline — passing today. Confirms the headless rig is functional and
// non-project-IO bridge calls work without a GUI in the loop.
// ---------------------------------------------------------------------------

#[test]
fn bridge_constructs_with_shared_state() {
    let _rig = build_headless_rig();
    // The mere fact that this builds proves the bridge does not require an
    // eframe App or any of `SynthApp`'s UI state to exist.
}

#[test]
fn set_song_tempo_via_bridge_mutates_shared_song() {
    let rig = build_headless_rig();

    let initial = rig.song.read().default_tempo.as_f32();
    assert!(
        (initial - 120.0).abs() < f32::EPSILON,
        "Song::new defaults to 120 BPM, got {initial}"
    );

    rig.bridge
        .set_song_tempo(144.0)
        .expect("set_song_tempo should succeed via shared state");

    let after = rig.song.read().default_tempo.as_f32();
    assert!(
        (after - 144.0).abs() < 1.0e-3,
        "tempo write did not reach shared Song: {after}"
    );
}

#[test]
fn list_instruments_starts_empty_and_create_instrument_appears() {
    let mut rig = build_headless_rig();

    let initial = rig
        .bridge
        .list_instruments()
        .expect("list_instruments should succeed");
    assert!(
        initial.is_empty(),
        "fresh bridge should have no instruments, got {initial:?}"
    );

    let info = rig
        .bridge
        .create_instrument("Lead")
        .expect("create_instrument should succeed without GUI");
    assert_eq!(info.name, "Lead");

    // Drain the AddInstrument command into shared state before reading back.
    rig.pump(8);

    let after = rig
        .bridge
        .list_instruments()
        .expect("list_instruments after create");
    assert_eq!(
        after.len(),
        1,
        "expected exactly one instrument after create, got {after:?}"
    );
}

// ---------------------------------------------------------------------------
// Phase 3 — the bug fix. After the MCP bridge stopped routing project I/O
// through the GUI condvar, these calls must succeed in any context.
// ---------------------------------------------------------------------------

#[test]
fn new_project_works_without_gui() {
    use std::sync::atomic::Ordering;

    let mut rig = build_headless_rig();

    // First poke the song so we have a baseline change to revert.
    rig.bridge
        .set_song_tempo(99.0)
        .expect("set_song_tempo before reset");

    let before = rig._shared.project_revision.load(Ordering::Acquire);
    rig.bridge
        .new_project()
        .expect("new_project should succeed without a GUI");
    rig.pump(8);
    let after = rig._shared.project_revision.load(Ordering::Acquire);

    assert_eq!(
        after,
        before + 1,
        "new_project should bump project_revision exactly once"
    );

    let refresh = rig
        ._shared
        .pending_project_refresh
        .lock()
        .expect("refresh lock")
        .take();
    assert!(
        matches!(refresh, Some(pertylizer::mcp_shared::ProjectRefresh::Reset)),
        "new_project should stash ProjectRefresh::Reset for GUI"
    );

    assert!(
        rig._shared
            .last_loaded_project_path
            .lock()
            .expect("path lock")
            .is_none(),
        "new_project should clear last_loaded_project_path"
    );

    // Tempo reset back to default 120.
    assert!(
        (rig.song.read().default_tempo.as_f32() - 120.0).abs() < 1e-3,
        "song should be reset to default tempo"
    );
}

#[test]
fn load_project_works_without_gui() {
    use std::sync::atomic::Ordering;

    let mut rig = build_headless_rig();
    let path = example_project_path();

    let before = rig._shared.project_revision.load(Ordering::Acquire);
    rig.bridge
        .load_project(path.to_str().expect("UTF-8 path"))
        .expect("load_project should succeed without a GUI");
    rig.pump(32);
    let after = rig._shared.project_revision.load(Ordering::Acquire);

    assert_eq!(
        after,
        before + 1,
        "load_project should bump project_revision exactly once"
    );

    let refresh = rig
        ._shared
        .pending_project_refresh
        .lock()
        .expect("refresh lock")
        .take();
    let loaded_project = match refresh {
        Some(pertylizer::mcp_shared::ProjectRefresh::Loaded(p)) => p,
        other => panic!(
            "expected ProjectRefresh::Loaded, got {:?}",
            other.as_ref().map(|_| "other")
        ),
    };

    // Loaded project metadata round-trips into shared state.
    let stored_path = rig
        ._shared
        .last_loaded_project_path
        .lock()
        .expect("path lock")
        .clone();
    assert_eq!(
        stored_path.as_deref(),
        Some(path.as_path()),
        "last_loaded_project_path should match the load argument"
    );

    // Engine state matches the loaded file.
    let instruments = rig.bridge.list_instruments().expect("list_instruments");
    assert_eq!(
        instruments.len(),
        loaded_project.instruments.len(),
        "instrument count should match the loaded project"
    );
    let expected_tempo = loaded_project.song.default_tempo.as_f32();
    let actual_tempo = rig.song.read().default_tempo.as_f32();
    assert!(
        (actual_tempo - expected_tempo).abs() < 1e-3,
        "song tempo should match loaded project: expected {expected_tempo}, got {actual_tempo}"
    );

    // Status entry written.
    let status = rig
        ._shared
        .last_project_io_status
        .lock()
        .expect("status lock")
        .clone();
    assert!(
        matches!(status, Some(Ok(_))),
        "last_project_io_status should be Ok after successful load, got {status:?}"
    );
}

#[test]
fn save_project_works_without_gui() {
    let mut rig = build_headless_rig();

    // Load something into the engine first so save has non-trivial content.
    let src = example_project_path();
    rig.bridge
        .load_project(src.to_str().expect("UTF-8 path"))
        .expect("load_project");
    rig.pump(32);

    // Use a .json suffix so `normalize_project_path` is a no-op — the
    // loaded example has no samples, so save chooses JSON and we want
    // the temp file's name to match.
    let tmp = tempfile::Builder::new()
        .suffix(".json")
        .tempfile()
        .expect("temp file");
    let target = tmp.path().to_str().expect("UTF-8 path");
    rig.bridge
        .save_project(target)
        .expect("save_project should succeed without a GUI");

    // File exists and re-parses as a ProjectFile.
    let reloaded = pertylizer::project::ProjectFile::load(tmp.path()).expect("reloaded saved file");
    assert!(
        !reloaded.instruments.is_empty(),
        "saved project should contain instruments from the loaded source"
    );

    let status = rig
        ._shared
        .last_project_io_status
        .lock()
        .expect("status lock")
        .clone();
    assert!(
        matches!(status, Some(Ok(_))),
        "last_project_io_status should be Ok after successful save, got {status:?}"
    );
}
