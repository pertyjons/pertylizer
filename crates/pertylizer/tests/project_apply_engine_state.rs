//! Engine-state regression tests for `project_apply::apply_project`
//! (Phase 1.1 of the `project-io-off-gui-thread` plan).
//!
//! Today the GUI's `load_project_data` is the canonical apply path; the
//! plan moves both GUI and MCP onto `project_apply::apply_project`. To
//! prove the new path doesn't drop engine writes that the GUI path used to
//! make, this test loads a bundled example project, runs it through
//! `apply_project`, drives the audio engine far enough for the shared
//! snapshot to settle, and asserts on the resulting `InstrumentSnapshot`s
//! plus a few global engine flags.
//!
//! Coverage targets the four observable engine differences identified in
//! the plan's behavior-diff table:
//!
//!   1. Instrument-level metadata (volume, pan, mute, solo, category,
//!      key_range, transpose, sidechain_source_id, …) round-trips from
//!      file → engine snapshot. Catches missing session.set_* calls.
//!   2. `InstrumentSnapshot.enabled == true` after apply. Catches missing
//!      `EngineCommand::SetInstrumentEnabled(true)`.
//!   3. `EngineState::get_focused_instrument()` equals the project's
//!      `active_instrument_id` (or first instrument). Catches missing
//!      `EngineCommand::SetFocusedInstrument`.
//!   4. Module counts per instrument match the patch (post-visualizer
//!      filter — visualizers are documented as "skipped" in apply_project,
//!      so the count is `patch.modules.len() - visualizer_count`).

#![cfg(feature = "mcp")]

use std::sync::Arc;

use parking_lot::RwLock as PlRwLock;
use synth_core::audio::SampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor};
use synth_engine::SynthEngine;
use synth_sampler::SampleLibrary;
use synth_sequencer::Song;

use pertylizer::bundle::{is_zip_file, load_bundle};
use pertylizer::patch::InstrumentState;
use pertylizer::project::ProjectFile;
use pertylizer::project_apply;
use pertylizer::session::SynthSession;

const TEST_SR: u32 = 44_100;

struct Rig {
    engine: SynthEngine,
    session: Arc<SynthSession>,
    song: Arc<PlRwLock<Song>>,
    sample_library: Arc<std::sync::RwLock<SampleLibrary>>,
    block: Vec<f32>,
    ctx: AudioCallbackContext,
    _handle: synth_engine::EngineHandle,
}

impl Rig {
    fn new() -> Self {
        let (mut engine, handle) = SynthEngine::new();
        let session = Arc::new(SynthSession::new(
            handle.command_sender(),
            Arc::clone(&handle.state),
        ));
        let song: Arc<PlRwLock<Song>> = Arc::new(PlRwLock::new(Song::new("test")));
        handle
            .command_sender()
            .send(synth_engine::EngineCommand::SetSong {
                song: Arc::clone(&song),
            });
        let sample_library = Arc::new(std::sync::RwLock::new(SampleLibrary::default()));

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate(TEST_SR),
            buffer_size: synth_core::BufferSize(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);

        let block = vec![0.0f32; 256 * 2];
        let ctx = AudioCallbackContext {
            sample_rate: HwSampleRate(TEST_SR),
            frames: 256,
            channels: 2,
            stream_time: 0.0,
            sample_position: 0,
            output_latency: synth_core::Seconds::ZERO,
        };

        let mut rig = Self {
            engine,
            session,
            song,
            sample_library,
            block,
            ctx,
            _handle: handle,
        };
        rig.pump(4);
        rig
    }

    fn pump(&mut self, blocks: usize) {
        for _ in 0..blocks {
            self.block.fill(0.0);
            self.engine.process(&mut self.block, &self.ctx);
        }
    }
}

mod common;
use common::examples_dir;

fn example_project(filename: &str) -> (ProjectFile, SampleLibrary) {
    load_project_at(&examples_dir().join(filename))
}

fn load_project_at(path: &std::path::Path) -> (ProjectFile, SampleLibrary) {
    let mut lib = SampleLibrary::default();
    let project = if is_zip_file(path) {
        load_bundle(path, &mut lib).expect("load bundle")
    } else {
        ProjectFile::load(path).expect("load json project")
    };
    (project, lib)
}

/// Count voice-graph modules in a patch — what `InstrumentSnapshot.module_count`
/// reports (`inst.voice_graph().len()` on the engine side). Visualizers,
/// SignalMonitor, and effect-chain modules are excluded.
fn voice_module_count(inst: &InstrumentState) -> usize {
    inst.patch
        .modules
        .iter()
        .filter(|m| {
            !matches!(
                m.module_type,
                synth_core::ModuleType::Oscilloscope
                    | synth_core::ModuleType::LevelMeter
                    | synth_core::ModuleType::SpectrumAnalyzer
                    | synth_core::ModuleType::SignalMonitor
            ) && !m.module_type.is_effect()
        })
        .count()
}

/// Count effect-chain modules in a patch — what `InstrumentSnapshot.effect_count`
/// reports.
fn effect_module_count(inst: &InstrumentState) -> usize {
    inst.patch
        .modules
        .iter()
        .filter(|m| m.module_type.is_effect())
        .count()
}

fn assert_snapshot_matches(project: &ProjectFile, rig: &Rig) {
    let snapshots = rig.session.list_instruments();
    assert_eq!(
        snapshots.len(),
        project.instruments.len(),
        "instrument count mismatch: file has {}, engine has {}",
        project.instruments.len(),
        snapshots.len()
    );

    for inst in &project.instruments {
        let snap = snapshots
            .iter()
            .find(|s| s.id == inst.id)
            .unwrap_or_else(|| panic!("instrument {} missing from engine snapshot", inst.id.0));

        // (2) Every loaded instrument must be enabled.
        assert!(
            snap.enabled,
            "instrument {} ({}) is disabled after apply — \
             apply_project is missing SetInstrumentEnabled(true)",
            inst.id.0, inst.name
        );

        // (1) Instrument-level metadata round-trip.
        assert_eq!(snap.name, inst.name, "name mismatch for {}", inst.id.0);
        assert!(
            (snap.volume.as_f32() - inst.volume.as_f32()).abs() < 1e-5,
            "volume mismatch for {}",
            inst.id.0
        );
        assert!(
            (snap.pan.as_f32() - inst.pan.as_f32()).abs() < 1e-5,
            "pan mismatch for {}",
            inst.id.0
        );
        assert_eq!(snap.muted, inst.muted, "mute mismatch for {}", inst.id.0);
        assert_eq!(snap.solo, inst.solo, "solo mismatch for {}", inst.id.0);
        assert_eq!(
            snap.category.as_u8(),
            inst.category,
            "category mismatch for {}",
            inst.id.0
        );
        assert_eq!(
            (snap.key_range.low.as_u8(), snap.key_range.high.as_u8()),
            inst.key_range,
            "key_range mismatch for {}",
            inst.id.0
        );
        assert_eq!(
            snap.midi_channel.as_u8(),
            inst.channel,
            "midi_channel mismatch for {}",
            inst.id.0
        );
        assert_eq!(
            snap.allocation_mode, inst.allocation_mode,
            "allocation_mode mismatch for {}",
            inst.id.0
        );
        assert_eq!(
            snap.max_voices, inst.max_voices,
            "max_voices mismatch for {}",
            inst.id.0
        );
        assert_eq!(
            snap.sidechain_source_id.map(|i| i.as_u64()),
            inst.sidechain_source_id,
            "sidechain_source_id mismatch for {}",
            inst.id.0
        );

        // (4) Voice-graph modules — visualizers + SignalMonitor stripped
        // (apply_project skips them) AND effect-chain modules are tracked
        // separately under `effect_count`.
        let expected_voice = voice_module_count(inst);
        assert_eq!(
            snap.module_count, expected_voice,
            "module_count mismatch for {} ({}): expected {} voice modules, got {}",
            inst.id.0, inst.name, expected_voice, snap.module_count
        );
        let expected_effects = effect_module_count(inst);
        assert_eq!(
            snap.effect_count, expected_effects,
            "effect_count mismatch for {} ({}): expected {} effect modules, got {}",
            inst.id.0, inst.name, expected_effects, snap.effect_count
        );
    }
}

fn assert_focused_instrument_matches(project: &ProjectFile, rig: &Rig) {
    // (3) Focused instrument must equal active_instrument_id if it exists,
    // else first instrument, else None.
    let target_id = synth_engine::InstrumentId::new(project.active_instrument_id);
    let expected = if project.instruments.iter().any(|i| i.id == target_id) {
        Some(target_id)
    } else {
        project.instruments.first().map(|i| i.id)
    };

    let actual = rig.session.state().get_focused_instrument();
    assert_eq!(
        actual, expected,
        "focused_instrument mismatch after apply — apply_project missing SetFocusedInstrument? \
         file active_instrument_id={}, expected={:?}, actual={:?}",
        project.active_instrument_id, expected, actual
    );
}

#[test]
fn apply_project_sidechain_demo_engine_state() {
    let mut rig = Rig::new();
    let (project, _lib) = example_project("Sidechain Demo.json");
    project_apply::apply_project(&project, &rig.session, &rig.song, &rig.sample_library)
        .expect("apply_project");

    // Let the shared snapshot settle (apply_project's wait_for_instrument_count
    // already waits up to 2 s, but per-instrument params route via the audio
    // queue and need a few extra blocks before update_shared_instruments fires).
    rig.pump(32);

    assert_snapshot_matches(&project, &rig);
    assert_focused_instrument_matches(&project, &rig);
}

#[test]
fn apply_project_neuro_engine_state() {
    let mut rig = Rig::new();
    let (project, _lib) = example_project("Neuro F#m 174-extended.json");
    project_apply::apply_project(&project, &rig.session, &rig.song, &rig.sample_library)
        .expect("apply_project");
    rig.pump(32);

    assert_snapshot_matches(&project, &rig);
    assert_focused_instrument_matches(&project, &rig);
}

#[test]
fn apply_new_project_clears_engine() {
    let mut rig = Rig::new();

    // First load a project so the engine is non-empty.
    let (project, _lib) = example_project("Sidechain Demo.json");
    project_apply::apply_project(&project, &rig.session, &rig.song, &rig.sample_library)
        .expect("apply_project");
    rig.pump(32);
    assert!(!rig.session.list_instruments().is_empty());

    // Now reset.
    project_apply::reset_to_new_project(&rig.session, &rig.song, &rig.sample_library)
        .expect("reset_to_new_project");
    rig.pump(32);

    assert!(
        rig.session.list_instruments().is_empty(),
        "instruments not cleared after reset_to_new_project: {:?}",
        rig.session.list_instruments()
    );
    assert_eq!(
        rig.song.read().name,
        "Untitled",
        "song should be reset to Untitled"
    );
}

/// Every bundled example project must round-trip through `apply_project`
/// into a coherent engine snapshot. Catches regressions where a save-format
/// change (Vec migrations, field renames, …) loads in serde but breaks the
/// file → engine pipeline for a specific example.
/// Representative subset of example projects exercised end-to-end through
/// `apply_project` into a live engine. The full 14-project sweep was too slow
/// for every CI run (~27s of instrument building); these three cover the
/// distinct paths — a bundle with embedded samples, a rich multi-instrument
/// patch with effect chains, and a simpler patch. Loading/validation of *all*
/// examples stays covered cheaply by `project_load_snapshot` and
/// `schemas_validate_examples`.
#[test]
fn apply_representative_example_projects_to_engine() {
    const REPRESENTATIVE: &[&str] = &[
        "echoing.zip",                      // bundle + embedded samples
        "Oxygene Dreams (80s Techno).json", // rich multi-instrument + effects
        "Classic Amiga module.json",        // simpler patch
    ];

    let mut failures: Vec<String> = Vec::new();
    for name in REPRESENTATIVE {
        let path = examples_dir().join(name);
        assert!(
            path.exists(),
            "representative example missing (was it renamed/removed?): {}",
            path.display()
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut rig = Rig::new();
            let (project, _lib) = load_project_at(&path);
            project_apply::apply_project(&project, &rig.session, &rig.song, &rig.sample_library)
                .expect("apply_project");
            rig.pump(32);

            assert_snapshot_matches(&project, &rig);
            assert_focused_instrument_matches(&project, &rig);
        }));
        if let Err(panic) = result {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "<non-string panic>".to_owned());
            failures.push(format!("{name}: {msg}"));
        }
    }

    assert!(
        failures.is_empty(),
        "apply_project regressions on {} example(s):\n  - {}",
        failures.len(),
        failures.join("\n  - "),
    );
}
