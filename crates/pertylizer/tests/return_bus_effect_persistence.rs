//! Round-trip test for return-bus effect-chain persistence (Phase 7a follow-up).
//!
//! A return bus's fader/name live in the `Song` (and round-trip with it); its
//! effect chain is engine-side runtime state published into a shared snapshot
//! and persisted in `GlobalProjectState.return_bus_effects`. This drives a real
//! engine: create a return bus + add an effect, build a `ProjectFile` from the
//! engine, apply it to a fresh engine, rebuild, and assert the effect chain
//! survived the save → load → save cycle.

#![cfg(feature = "mcp")]

use std::sync::Arc;

use synth_core::audio::DeviceSampleRate as HwSampleRate;
use synth_core::{AudioCallbackContext, AudioProcessor, ModuleType};
use synth_engine::{EngineCommand, ModuleId, SynthEngine};
use synth_sampler::SampleLibrary;
use synth_sequencer::Song;

use pertylizer::project_apply::{self, ProjectBuildOptions};
use pertylizer::session::SynthSession;

const TEST_SR: u32 = 44_100;

struct Rig {
    engine: SynthEngine,
    session: Arc<SynthSession>,
    song: Arc<synth_sequencer::SharedSong>,
    sample_library: Arc<std::sync::RwLock<SampleLibrary>>,
    block: Vec<f32>,
    ctx: AudioCallbackContext,
    handle: synth_engine::EngineHandle,
}

impl Rig {
    fn new() -> Self {
        let (mut engine, handle) = SynthEngine::new();
        let session = Arc::new(SynthSession::new(
            handle.command_sender(),
            Arc::clone(&handle.state),
        ));
        let song: Arc<synth_sequencer::SharedSong> =
            Arc::new(synth_sequencer::SharedSong::new(Song::new("test")));
        handle.command_sender().send(EngineCommand::SetSong {
            song: Arc::clone(&song),
        });
        let sample_library = Arc::new(std::sync::RwLock::new(SampleLibrary::default()));

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(TEST_SR),
            buffer_size: synth_core::BufferSize::new(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);

        let ctx = AudioCallbackContext {
            sample_rate: HwSampleRate::new(TEST_SR),
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
            block: vec![0.0f32; 256 * 2],
            ctx,
            handle,
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

    fn build_project(&self) -> pertylizer::project::ProjectFile {
        project_apply::build_project_from_engine(
            &self.session,
            &self.song,
            &self.sample_library,
            ProjectBuildOptions::default(),
        )
    }
}

#[test]
fn return_bus_effect_chain_survives_save_load_save() {
    // --- Author a return bus with one effect on a real engine ---------------
    let mut rig = Rig::new();
    let rid = rig.song.write().create_return_bus("Reverb");
    rig.handle
        .command_sender()
        .send(EngineCommand::CreateReturnBus { id: rid });
    rig.handle
        .command_sender()
        .send(EngineCommand::AddReturnEffect {
            return_id: rid,
            id: ModuleId::new(ModuleType::Distortion, 1),
            effect: Box::new(synth_modules::Distortion::new()),
        });
    rig.pump(4);

    // --- Save: the engine snapshot is captured into the project -------------
    let project = rig.build_project();
    assert_eq!(
        project.song.return_busses().len(),
        1,
        "return bus definition should be in the song"
    );
    assert_eq!(
        project.global.return_bus_effects.len(),
        1,
        "return bus effect chain should be persisted"
    );
    let saved = &project.global.return_bus_effects[0];
    assert_eq!(saved.id, rid.0);
    assert_eq!(saved.effects.len(), 1);
    assert_eq!(saved.effects[0].module_type, ModuleType::Distortion);
    assert!(
        !saved.effects[0].parameters.is_empty(),
        "effect parameters should be captured"
    );

    // --- Load into a fresh engine, then re-save and compare -----------------
    let mut rig2 = Rig::new();
    project_apply::apply_project(&project, &rig2.session, &rig2.song, &rig2.sample_library)
        .expect("apply_project");
    rig2.pump(8);

    let project2 = rig2.build_project();
    let before = serde_json::to_value(&project.global.return_bus_effects).unwrap();
    let after = serde_json::to_value(&project2.global.return_bus_effects).unwrap();
    assert_eq!(
        before, after,
        "return-bus effect chain must round-trip through save → load → save"
    );
}

#[test]
fn applying_a_project_resets_existing_return_channels_instead_of_stacking() {
    // An engine that already holds a return bus with one effect...
    let mut rig = Rig::new();
    let rid = rig.song.write().create_return_bus("Reverb");
    rig.handle
        .command_sender()
        .send(EngineCommand::CreateReturnBus { id: rid });
    rig.handle
        .command_sender()
        .send(EngineCommand::AddReturnEffect {
            return_id: rid,
            id: ModuleId::new(ModuleType::Distortion, 1),
            effect: Box::new(synth_modules::Distortion::new()),
        });
    rig.pump(4);

    let project = rig.build_project();
    assert_eq!(project.global.return_bus_effects[0].effects.len(), 1);

    // ...re-applying a project must reset return channels first, not append to
    // the surviving channel (CreateReturnBus is a no-op for an existing id).
    project_apply::apply_project(&project, &rig.session, &rig.song, &rig.sample_library)
        .expect("apply_project");
    rig.pump(8);

    let project2 = rig.build_project();
    assert_eq!(
        project2.global.return_bus_effects.len(),
        1,
        "exactly one return bus after re-apply"
    );
    assert_eq!(
        project2.global.return_bus_effects[0].effects.len(),
        1,
        "re-loading must reset the return channel, not stack effects onto the old chain"
    );
}
