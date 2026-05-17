//! Headless project lifecycle — apply a `ProjectFile` to the engine/session/song
//! without depending on any GUI state.
//!
//! The GUI's `egui_backend::load_project_data` does the same job but also
//! rebuilds `InstrumentUiState`, `PatchEditor` canvases, visualization buffers,
//! and `PianoKeyboard` state. In headless mode none of that exists, so this
//! module provides a smaller path that touches only the shared engine state
//! (`SynthSession`, `Song`, `SampleLibrary`, and the engine command channel).
//!
//! Used by the headless MCP worker in `main.rs::run_headless_mcp` to service
//! `pending_project_action` requests originating from MCP.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use parking_lot::RwLock as PRwLock;

use synth_core::{ModuleType, VoiceCount};
use synth_engine::commands::InstrumentParam;
use synth_engine::{CommandSender, EngineCommand, InstrumentId, MidiChannel, ModuleId};
use synth_sampler::SampleLibrary;
use synth_sequencer::Song;

use crate::mcp_shared::{McpSharedState, ProjectAction};
use crate::patch::{InstrumentState, Patch};
use crate::project::ProjectFile;
use crate::session::{SessionError, SynthSession};

type SharedSong = Arc<PRwLock<Song>>;
type SharedSampleLibrary = Arc<std::sync::RwLock<SampleLibrary>>;

/// Handle returned by [`spawn_worker`] — the caller signals shutdown via the
/// flag and then joins the thread to wait for it to exit.
pub struct HeadlessWorker {
    pub running: Arc<AtomicBool>,
    pub handle: JoinHandle<()>,
}

impl HeadlessWorker {
    /// Signal the worker to stop and wait for it to finish in-flight work.
    pub fn shutdown(self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = self.handle.join();
    }
}

/// Spawn the headless MCP project-action worker. Mirrors the GUI's per-frame
/// poll of `pending_project_action`; without it, `submit_project_action`
/// would always hit the 5-second condvar timeout in `--headless` mode.
pub fn spawn_worker(
    shared: Arc<McpSharedState>,
    session: Arc<SynthSession>,
    sample_library: SharedSampleLibrary,
) -> HeadlessWorker {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);
    let handle = std::thread::spawn(move || {
        while running_clone.load(Ordering::Relaxed) {
            let action = shared
                .pending_project_action
                .lock()
                .ok()
                .and_then(|mut a| a.take());
            let Some(action) = action else {
                std::thread::sleep(std::time::Duration::from_millis(25));
                continue;
            };
            let result = dispatch_action(action, &session, &shared.song, &sample_library);
            let (lock, cvar) = &shared.project_action_result;
            if let Ok(mut guard) = lock.lock() {
                *guard = Some(result);
                cvar.notify_one();
            }
        }
    });
    HeadlessWorker { running, handle }
}

fn dispatch_action(
    action: ProjectAction,
    session: &SynthSession,
    song: &SharedSong,
    sample_library: &SharedSampleLibrary,
) -> Result<String, String> {
    match action {
        ProjectAction::New => reset_to_new_project(session, song, sample_library),
        ProjectAction::Save(path) => save_project_to(&path, session, song, sample_library),
        ProjectAction::Load(path) => load_file_into_engine(&path, session, song, sample_library),
    }
}

/// Replace all engine/session/song state with the contents of `project`.
///
/// Visualizer modules in the patch (Oscilloscope, LevelMeter, SpectrumAnalyzer,
/// SignalMonitor) surface as `VisualizerRequiresGui` entries in the
/// per-instrument error log and are silently skipped — the session refuses
/// them because they need a GUI-side `VisualizationBuffer` to write into.
pub(crate) fn apply_project(
    project: &ProjectFile,
    session: &SynthSession,
    song: &SharedSong,
    sample_library: &SharedSampleLibrary,
) -> Result<String, String> {
    let sender = session.command_sender();
    sender.send(EngineCommand::Stop);

    tear_down_all_instruments(session);

    for inst_state in &project.instruments {
        install_instrument(session, &sender, inst_state)?;
    }

    push_loaded_sample_data(&sender, project, sample_library);

    sender.send(EngineCommand::SetTempo(project.song.default_tempo));
    {
        let mut s = song.write();
        *s = project.song.clone();
    }

    sender.send(EngineCommand::SetMasterVolume(project.global.master_volume));
    sender.send(EngineCommand::SetGlideTime(project.global.glide_time));

    // Bridge the async gap between queued `AddInstrument` commands and the
    // audio thread updating its snapshot — without this, a client calling
    // `load_project` immediately followed by `list_instruments` would see a
    // half-populated list.
    wait_for_instrument_count(session, project.instruments.len(), 2_000);

    if let Some(awe) = &project.global.awe {
        apply_awe_state(&sender, awe);
    }

    let pattern_count = project.song.patterns().count();
    let track_count = project.song.tracks().count();
    Ok(format!(
        "Loaded project: {} instrument(s), {} pattern(s), {} track(s)",
        project.instruments.len(),
        pattern_count,
        track_count
    ))
}

/// Reset to an empty project — clears all instruments and replaces the song
/// with a freshly-named "Untitled".
pub(crate) fn reset_to_new_project(
    session: &SynthSession,
    song: &SharedSong,
    sample_library: &SharedSampleLibrary,
) -> Result<String, String> {
    let empty = ProjectFile::new(
        Vec::new(),
        0,
        None,
        Song::new("Untitled"),
        crate::project::GlobalProjectState::default(),
    );
    let result = apply_project(&empty, session, song, sample_library)?;
    if let Ok(mut lib) = sample_library.write() {
        lib.clear();
    }
    Ok(result)
}

/// Load any project-ish file (project JSON, ZIP bundle, single patch, AWE
/// preset). Mirrors the GUI's `LoadedFile` dispatch.
pub(crate) fn load_file_into_engine(
    path: &std::path::Path,
    session: &SynthSession,
    song: &SharedSong,
    sample_library: &SharedSampleLibrary,
) -> Result<String, String> {
    use crate::project::{LoadedFile, load_file};

    if !path.exists() {
        return Err(format!("File not found: {}", path.display()));
    }

    match load_file(path).map_err(|e| e.to_string())? {
        LoadedFile::Project(proj) => apply_project(&proj, session, song, sample_library),
        LoadedFile::Patch(patch) => apply_patch_as_single_instrument(&patch, session),
        LoadedFile::AwePreset(_) => {
            Err("AWE preset files cannot be loaded as a project in headless mode".to_string())
        }
        LoadedFile::Bundle(bundle_path) => {
            load_bundle_into_engine(&bundle_path, session, song, sample_library)
        }
    }
}

/// Headless `save_project` stub. Not yet supported because `InstrumentSnapshot`
/// (engine-side metadata exposed by `SynthSession::list_instruments`) is
/// missing seven fields the project file records (`key_range`, `transpose`,
/// `oversampling`, `allocation_mode`, `stealing_strategy`, `max_voices`, plus
/// the two velocity sensitivities). The GUI path reads them from
/// `InstrumentUiState`. Extending the snapshot is the right fix — see §9.7 of
/// `docs/mcp-music-tools-plan.md`.
pub(crate) fn save_project_to(
    _path: &std::path::Path,
    _session: &SynthSession,
    _song: &SharedSong,
    _sample_library: &SharedSampleLibrary,
) -> Result<String, String> {
    Err("save_project is not yet supported in --headless mode (InstrumentSnapshot is missing several persisted fields). Run the GUI build to save.".to_string())
}

fn load_bundle_into_engine(
    path: &std::path::Path,
    session: &SynthSession,
    song: &SharedSong,
    sample_library: &SharedSampleLibrary,
) -> Result<String, String> {
    let project = {
        let mut lib = sample_library
            .write()
            .map_err(|_| "sample library lock poisoned".to_string())?;
        crate::bundle::load_bundle(path, &mut lib).map_err(|e| e.to_string())?
    };
    apply_project(&project, session, song, sample_library).map(|msg| format!("{msg} (bundle)"))
}

fn apply_patch_as_single_instrument(
    patch: &Patch,
    session: &SynthSession,
) -> Result<String, String> {
    tear_down_all_instruments(session);

    let cfg = synth_engine::voice_allocator::AllocatorConfig {
        max_voices: VoiceCount::OCTO,
        ..Default::default()
    };
    let inst_id = InstrumentId::FIRST;
    if let Err(e) = session.add_instrument_with_id_and_config(inst_id, &patch.name, Some(cfg)) {
        return Err(format!("create instrument: {e}"));
    }
    let apply_result = session.apply_patch(inst_id, patch);
    if !apply_result.errors.is_empty() {
        eprintln!("apply_patch errors: {:?}", apply_result.errors);
    }
    Ok(format!("Loaded patch '{}' as instrument 1", patch.name))
}

fn install_instrument(
    session: &SynthSession,
    sender: &CommandSender,
    inst_state: &InstrumentState,
) -> Result<(), String> {
    let inst_id = inst_state.id;
    let cfg = synth_engine::voice_allocator::AllocatorConfig {
        max_voices: inst_state.max_voices,
        mode: inst_state.allocation_mode,
        stealing: inst_state.stealing_strategy,
        ..Default::default()
    };
    if let Err(e) = session.add_instrument_with_id_and_config(inst_id, &inst_state.name, Some(cfg))
    {
        return Err(format!(
            "failed to create instrument {} ({}): {e}",
            inst_state.name,
            inst_id.as_u64()
        ));
    }

    let apply_result = session.apply_patch(inst_id, &inst_state.patch);
    for err in &apply_result.errors {
        eprintln!(
            "apply_patch({}, {}): {err}",
            inst_state.name,
            inst_id.as_u64()
        );
    }

    let order: Vec<ModuleId> = inst_state
        .patch
        .settings
        .effect_chain_order
        .iter()
        .filter_map(|s| s.parse::<ModuleId>().ok())
        .collect();
    if !order.is_empty() {
        sender.send_blocking(EngineCommand::SetEffectChainOrder {
            instrument_id: Some(inst_id),
            order,
        });
    }

    apply_instrument_metadata(session, inst_state, inst_id);
    push_instrument_params(sender, inst_state, inst_id);
    Ok(())
}

/// Route per-instrument settings through `session` (rather than direct engine
/// commands) so the changes are mirrored into the engine snapshot that MCP
/// read tools consume.
fn apply_instrument_metadata(
    session: &SynthSession,
    inst_state: &InstrumentState,
    inst_id: InstrumentId,
) {
    let channel = MidiChannel::from_one_indexed(inst_state.channel).unwrap_or(MidiChannel::CH1);
    log_err(
        "rename_instrument",
        session.rename_instrument(inst_id, &inst_state.name),
    );
    if !inst_state.description.is_empty() {
        log_err(
            "set_instrument_description",
            session.set_instrument_description(inst_id, &inst_state.description),
        );
    }
    if let Some(patch_desc) = inst_state.patch.description.as_deref()
        && !patch_desc.is_empty()
    {
        log_err(
            "set_patch_description",
            session.set_patch_description(inst_id, Some(patch_desc)),
        );
    }
    if let Some(src) = inst_state.sidechain_source_id {
        log_err(
            "set_sidechain_source",
            session.set_sidechain_source(inst_id, Some(InstrumentId::new(src))),
        );
    }
    log_err(
        "set_instrument_category",
        session.set_instrument_category(
            inst_id,
            synth_engine::InstrumentCategory::from_u8(inst_state.category),
        ),
    );
    log_err(
        "set_instrument_volume",
        session.set_instrument_volume(inst_id, inst_state.volume),
    );
    log_err(
        "set_instrument_pan",
        session.set_instrument_pan(inst_id, inst_state.pan),
    );
    log_err(
        "set_instrument_mute",
        session.set_instrument_mute(inst_id, inst_state.muted),
    );
    log_err(
        "set_instrument_solo",
        session.set_instrument_solo(inst_id, inst_state.solo),
    );
    log_err(
        "set_instrument_midi_channel",
        session.set_instrument_midi_channel(inst_id, channel),
    );
}

fn push_instrument_params(
    sender: &CommandSender,
    inst_state: &InstrumentState,
    inst_id: InstrumentId,
) {
    let oversampling = match inst_state.oversampling {
        2 => synth_dsp::OversamplingFactor::X2,
        4 => synth_dsp::OversamplingFactor::X4,
        _ => synth_dsp::OversamplingFactor::X1,
    };
    let key_range = synth_engine::instrument::KeyRange::new(
        synth_core::MidiNote::new(inst_state.key_range.0),
        synth_core::MidiNote::new(inst_state.key_range.1),
    );
    for param in [
        InstrumentParam::OversamplingFactor(oversampling),
        InstrumentParam::KeyRange(key_range),
        InstrumentParam::Transpose(inst_state.transpose),
        InstrumentParam::AllocationMode(inst_state.allocation_mode),
        InstrumentParam::StealingStrategy(inst_state.stealing_strategy),
        InstrumentParam::VelocityAmpSensitivity(inst_state.velocity_amp_sensitivity),
        InstrumentParam::VelocityFilterSensitivity(inst_state.velocity_filter_sensitivity),
    ] {
        sender.send(EngineCommand::SetInstrumentParameter {
            instrument_id: inst_id,
            param,
        });
    }
}

fn tear_down_all_instruments(session: &SynthSession) {
    let existing: Vec<InstrumentId> = session.list_instruments().iter().map(|s| s.id).collect();
    for inst_id in existing {
        let _ = session.clear_graph(inst_id);
        let _ = session.remove_instrument(inst_id);
    }
}

/// Push every field of `awe` onto the engine. Uses non-blocking sends because
/// these all land in the same audio callback batch and no later step gates on
/// their arrival — matches `egui_backend::load_project_data`.
fn apply_awe_state(sender: &CommandSender, awe: &synth_awe::AweState) {
    sender.send(EngineCommand::SetAweEnabled {
        enabled: awe.enabled,
    });
    sender.send(EngineCommand::SetAweParameter {
        param: synth_awe::AweParam::RoomShape(awe.room),
    });
    sender.send(EngineCommand::SetAweParameter {
        param: synth_awe::AweParam::Material(awe.material),
    });
    sender.send(EngineCommand::SetAweState {
        snapshot: awe.to_snapshot(),
    });
    sender.send(EngineCommand::SetAweParameter {
        param: synth_awe::AweParam::SpatialEnabled(awe.spatial_enabled),
    });
    sender.send(EngineCommand::SetAweParameter {
        param: synth_awe::AweParam::NoteMapping(awe.note_mapping),
    });
}

/// Send `LoadSampleData` for every Sampler module in the project that
/// references a non-zero sample id — `set_parameter` on a sampler only stores
/// the id; the engine also needs the audio buffer. Mirrors
/// `egui_backend::send_loaded_sample_data`.
fn push_loaded_sample_data(
    sender: &CommandSender,
    project: &ProjectFile,
    sample_library: &SharedSampleLibrary,
) {
    let Ok(lib) = sample_library.read() else {
        return;
    };
    for inst_state in &project.instruments {
        for module in &inst_state.patch.modules {
            if module.module_type != ModuleType::Sampler {
                continue;
            }
            let sample_id = match module.parameters.get("sample_select") {
                Some(crate::patch::ParamValue::SampleId { sample_id }) => *sample_id,
                _ => continue,
            };
            if sample_id == 0 {
                continue;
            }
            let Ok(mod_id) = module.id.parse::<ModuleId>() else {
                continue;
            };
            let Some(sample) = lib.get(synth_sampler::SampleId::new(sample_id)) else {
                continue;
            };
            sender.send(crate::audio::preview::load_sample_data_command(
                inst_state.id,
                mod_id,
                sample,
            ));
        }
    }
}

fn wait_for_instrument_count(session: &SynthSession, target: usize, timeout_ms: u64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if session.list_instruments().len() >= target {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn log_err<T>(op: &str, result: Result<T, SessionError>) {
    if let Err(e) = result {
        eprintln!("{op}: {e}");
    }
}
