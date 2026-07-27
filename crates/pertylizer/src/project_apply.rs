//! Canonical engine-side project lifecycle: apply a `ProjectFile` to
//! the engine/session/song without touching any GUI state. Visualizer
//! modules are skipped here — they need a GUI-owned
//! `VisualizationBuffer` to write into.

use std::collections::BTreeMap;
use std::sync::Arc;

use synth_core::ModuleParam;
use synth_core::{ModuleType, Param, ParameterDescriptor, VoiceCount};
use synth_engine::commands::InstrumentParam;
use synth_engine::shared_state::{
    ConnectionSnapshot, InstrumentSnapshot, ModuleStateSnapshot, ReturnBusSnapshot,
};
use synth_engine::{CommandSender, EngineCommand, InstrumentId, MidiChannelSelection, ModuleId};
use synth_sampler::SampleLibrary;
use synth_sequencer::Song;

use crate::patch::{ConnectionState, InstrumentState, ModuleState, ParamValue, Patch, Position};
use crate::project::{GlobalProjectState, ProjectFile, ReturnBusEffectsState};
use crate::session::{SessionError, SynthSession};

type SharedSong = Arc<synth_sequencer::SharedSong>;
type SharedSampleLibrary = Arc<std::sync::RwLock<SampleLibrary>>;

/// Empty the shared sample library. Call this before applying a plain-JSON
/// project — JSON projects carry no embedded sample data, and a non-empty
/// library silently switches the next save to bundle format and re-embeds
/// the stale samples. Bundle loads clear inside `bundle::load_bundle`.
pub(crate) fn clear_sample_library(sample_library: &SharedSampleLibrary) {
    let mut lib = sample_library.write().unwrap_or_else(|e| e.into_inner());
    lib.clear();
}

/// Recompile every pooled graph's `NoteScriptTransform` programs from their
/// serialized `source`. The compiled `BoundScript` is `#[serde(skip)]`, so a
/// freshly-deserialized (or MCP/serde-built) script node starts with no program
/// and would silently pass notes through — this is the non-RT resync step that
/// installs the program (or clears it on a compile error / empty source). Call
/// after any path that installs a `Song` from serialized data.
pub(crate) fn recompile_note_graph_scripts(song: &mut Song) {
    for graph in song.note_graphs_mut() {
        recompile_graph_scripts(graph);
    }
}

/// Recompile one graph's `NoteScriptTransform` programs from their `source`.
/// The per-graph unit of [`recompile_note_graph_scripts`], reused by the Note
/// Grid GUI and MCP mutation paths after they add or replace a node.
pub(crate) fn recompile_graph_scripts(graph: &mut synth_sequencer::NoteGraph) {
    for config in graph.nodes_mut() {
        if let synth_sequencer::NoteModuleConfig::NoteScriptTransform(transform) = config {
            let compiled = if transform.source().trim().is_empty() {
                None
            } else {
                crate::session::compile_note_event_script(transform.source()).ok()
            };
            transform.set_compiled(compiled);
        }
    }
}

/// Replace all engine/session/song state with the contents of `project`.
///
/// Visualizer modules in the patch (Oscilloscope, LevelMeter, SpectrumAnalyzer,
/// SignalMonitor) surface as `VisualizerRequiresGui` entries in the
/// per-instrument error log and are silently skipped — the session refuses
/// them because they need a GUI-side `VisualizationBuffer` to write into.
pub fn apply_project(
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
        // Upgrade legacy pre-Note-Grid projects: fold each pattern's linear
        // NoteProcessor rack into a pooled graph (before the rebuild below, which
        // then builds the new graphs' derived order alongside the existing pool).
        let migrated = s.migrate_processor_racks_to_graphs();
        if migrated > 0 {
            tracing::info!(
                migrated,
                "Migrated {migrated} note-processor rack(s) to note graphs on load"
            );
        }
        // `NoteGraph::processing_order` is `#[serde(skip)]`, so a freshly
        // deserialized pool has empty derived orders. This in-place install
        // path never sends `EngineCommand::SetSong` (which is where the engine
        // otherwise rebuilds), so rebuild here or every graph-bound pattern
        // would expand through zero nodes and play dry until an unrelated edit.
        s.rebuild_note_graphs();
        // Every `NoteScriptTransform` node's compiled program is `#[serde(skip)]`
        // too, so a just-loaded script node is pass-through until recompiled from
        // its `source`. Same reasoning as the rebuild above — no `SetSong` here.
        recompile_note_graph_scripts(&mut s);
        // Sanitize the Mod Grid pool the same way: drop any cable a just-loaded
        // graph can't validate (unknown node / cycle) so a corrupt save loads
        // playable, and bump `mod_grid_generation` so the GUI's per-frame sync
        // rebuilds and ships the running instances for the loaded pool.
        s.rebuild_mod_graphs();
    }
    // Reinstall the shared song even though its allocation is unchanged. A
    // deserialized Song starts with a non-persisted structure generation that
    // can equal the sequencer's cached generation from the previous project.
    // SetSong explicitly refreshes cached tempo/length and resets transport, so
    // playback cannot stop at the previous project's arrangement end.
    if !sender.send(EngineCommand::SetSong {
        song: Arc::clone(song),
    }) {
        return Err("failed to refresh sequencer state for loaded song".to_string());
    }

    // Restore the saved transport loop into the engine's sequencer (runtime
    // state that isn't part of the shared Song). Sending an explicit disabled
    // region when none was saved also clears any loop left over from a
    // previously-loaded project — mirroring the ClearReturnBusses/ClearMasterEffects
    // resets below.
    let loop_cmd = project.song.transport_loop().map_or(
        EngineCommand::SetLoop {
            start: synth_sequencer::Tick::ZERO,
            end: synth_sequencer::Tick::ZERO,
            enabled: false,
        },
        |r| EngineCommand::SetLoop {
            start: r.start,
            end: r.end,
            enabled: r.enabled,
        },
    );
    sender.send(loop_cmd);

    sender.send(EngineCommand::SetMasterVolume(project.global.master_volume));
    sender.send(EngineCommand::SetGlideTime(project.global.glide_time));

    // Reconstruct the send/return-bus channels + return-effect chains + master
    // effect chain. Non-blocking `send`: the live audio thread drains the queue
    // continuously. The offline WAV export shares this exact reconstruction via
    // `apply_global_mix_chain` (with `send_blocking`), so both hear the same mix.
    apply_global_mix_chain(project, |c| sender.send(c));

    // Bridge the async gap between queued `AddInstrument` commands and the
    // audio thread updating its snapshot — without this, a client calling
    // `load_project` immediately followed by `list_instruments` would see a
    // half-populated list.
    wait_for_instrument_count(session, project.instruments.len(), 2_000);

    // Focus the saved active instrument (or the first one) so MIDI /
    // keyboard input is routed correctly. Without this, focus stays on
    // whatever instrument was focused before the load.
    let focused = {
        let target = InstrumentId::new(project.active_instrument_id);
        if project.instruments.iter().any(|i| i.id == target) {
            Some(target)
        } else {
            project.instruments.first().map(|i| i.id)
        }
    };
    sender.send(EngineCommand::SetFocusedInstrument(focused));

    let pattern_count = project.song.patterns().count();
    let track_count = project.song.tracks().count();
    Ok(format!(
        "Loaded project: {} instrument(s), {} pattern(s), {} track(s)",
        project.instruments.len(),
        pattern_count,
        track_count
    ))
}

/// Reconstruct a project's global mix chain in the engine: the send/return-bus
/// channels, each return bus's effect chain, and the master effect chain. Shared
/// by the live project loader ([`apply_project`]) and the offline WAV export so
/// both render the exact same send/return + master processing — a loader that
/// only installs instrument voices silently drops every send, return effect, and
/// master effect.
///
/// `send` is the caller's enqueue closure: the live loader passes a non-blocking
/// `CommandSender::send` (the audio thread drains the queue continuously); the
/// offline export passes `send_blocking`, because its fresh engine is not
/// draining while it loads. Commands are emitted in dependency order
/// (Clear → CreateReturnBus → AddReturnEffect → params; Clear → AddEffectInstance
/// → params), so an in-order queue reconstructs the chains correctly.
///
/// `on_stream_start` must have run on the target engine first: the effect-add
/// handlers stamp each effect with the engine's current sample rate.
pub(crate) fn apply_global_mix_chain(
    project: &ProjectFile,
    mut send: impl FnMut(EngineCommand) -> bool,
) {
    // Reset any return-bus channels left over from a previously-loaded project
    // before recreating this project's — otherwise CreateReturnBus is a no-op
    // for a surviving id and AddReturnEffect would stack onto the old chain.
    send(EngineCommand::ClearReturnBusses);
    // Create the engine-side runtime channel for each return bus defined in the
    // song (faders are read live from the song), then rebuild its effect chain
    // in order. Commands drain in order, so AddReturnEffect after CreateReturnBus
    // is safe.
    for bus in project.song.return_busses() {
        send(EngineCommand::CreateReturnBus { id: bus.id });
    }
    for rb in &project.global.return_bus_effects {
        let return_id = synth_sequencer::ReturnBusId(rb.id);
        for fx in &rb.effects {
            let Ok(module_id) = fx.id.parse::<ModuleId>() else {
                continue;
            };
            let Some((effect, descriptor)) = crate::module_factory::create_effect(fx.module_type)
            else {
                continue;
            };
            send(EngineCommand::AddReturnEffect {
                return_id,
                id: module_id,
                effect,
            });
            for (type_id, value) in &fx.parameters {
                if let Some(param_desc) =
                    descriptor.parameters.iter().find(|p| &p.type_id == type_id)
                {
                    send(EngineCommand::SetReturnEffectParameter {
                        return_id,
                        module_id,
                        param: value.to_param(param_desc),
                    });
                }
            }
        }
    }

    // Rebuild the master effect chain (instrument_id: None). Clear first so a
    // load over a previous project starts from an empty master chain rather than
    // stacking onto leftover effects.
    send(EngineCommand::ClearMasterEffects);
    for fx in &project.global.master_effects {
        let Ok(module_id) = fx.id.parse::<ModuleId>() else {
            continue;
        };
        let Some((effect, descriptor)) = crate::module_factory::create_effect(fx.module_type)
        else {
            continue;
        };
        send(EngineCommand::AddEffectInstance {
            instrument_id: None,
            id: module_id,
            effect,
        });
        for (type_id, value) in &fx.parameters {
            if let Some(param_desc) = descriptor.parameters.iter().find(|p| &p.type_id == type_id) {
                send(EngineCommand::SetEffectParameter {
                    instrument_id: None,
                    module_id,
                    param: value.to_param(param_desc),
                });
            }
        }
    }
}

/// Reset to an empty project — clears all instruments and replaces the song
/// with a freshly-named "Untitled".
pub fn reset_to_new_project(
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
    clear_sample_library(sample_library);
    apply_project(&empty, session, song, sample_library)
}

/// Load any project-ish file (project JSON, ZIP bundle, single patch).
/// Mirrors the GUI's `LoadedFile` dispatch.
pub fn load_file_into_engine(
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
        LoadedFile::Project(proj) => {
            clear_sample_library(sample_library);
            apply_project(&proj, session, song, sample_library)
        }
        LoadedFile::Patch(patch) => apply_patch_as_single_instrument(&patch, session),
        LoadedFile::Bundle(bundle_path) => {
            load_bundle_into_engine(&bundle_path, session, song, sample_library)
        }
    }
}

/// Create the parent directory of a save path when it has a non-empty one.
/// Shared by the project and single-patch save paths.
fn ensure_parent_dir(path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("create parent dir: {e}"))?;
    }
    Ok(())
}

/// Save the current engine + session + song state to `path` as a JSON
/// `ProjectFile`. Headless counterpart of `egui_backend::create_project_from_app`,
/// reading instrument metadata from `InstrumentSnapshot` (engine-mirrored,
/// extended in §9.7 to carry every field the file format records) and module
/// graphs from `SharedGraphState`.
///
/// Limitations versus the GUI save path:
/// - Module positions default to `(0, 0)` (headless has no patch-editor canvas).
/// - Author metadata is omitted (no settings/UI source).
/// - `glide_time` and `octave_offset` use defaults.
pub fn save_project_to(
    path: &std::path::Path,
    session: &SynthSession,
    song: &SharedSong,
    sample_library: &SharedSampleLibrary,
    opts: ProjectBuildOptions,
) -> Result<String, String> {
    let project = build_project_from_engine(session, song, sample_library, opts);

    ensure_parent_dir(path)?;

    project.save(path).map_err(|e| e.to_string())?;

    let pattern_count = project.song.patterns().count();
    let track_count = project.song.tracks().count();
    Ok(format!(
        "Saved project: {} ({} instrument(s), {} pattern(s), {} track(s))",
        path.display(),
        project.instruments.len(),
        pattern_count,
        track_count
    ))
}

/// Save a single instrument's voice graph as a standalone patch file — the same
/// single-instrument format the GUI's "Save Patch" writes and that
/// `project::load_file` reads back as `LoadedFile::Patch`. Unlike
/// [`save_project_to`], this writes exactly one instrument's modules,
/// connections, and patch metadata (no song, no other instruments).
pub fn save_patch_to(
    path: &std::path::Path,
    session: &SynthSession,
    instrument_id: InstrumentId,
) -> Result<String, String> {
    // The save reads the async-mirrored instrument snapshot + shared_graph. If
    // the instrument is known to the synchronous registry but not yet mirrored
    // (e.g. it was just created in the same batch), give the audio thread a
    // bounded window to catch up — the same gap-bridging apply_project does after
    // AddInstrument — rather than failing a valid save. The command-drain wait
    // additionally covers module *mutations* (add_module/connect on an existing
    // instrument) queued in the same batch, which the instrument-existence wait
    // alone does not, so the saved graph isn't stale/truncated.
    if session.instrument_exists(instrument_id) {
        let _ = session.wait_for_pending_commands(SNAPSHOT_SYNC_TIMEOUT_MS);
        wait_for_instrument(session, instrument_id, SNAPSHOT_SYNC_TIMEOUT_MS);
    }

    let snapshots = session.list_instruments();
    let snap = snapshots
        .iter()
        .find(|s| s.id == instrument_id)
        .ok_or_else(|| {
            // Distinguish a genuinely unknown id from one that never mirrored in
            // time, so the failure is honest rather than a misleading "not found".
            if session.instrument_exists(instrument_id) {
                format!(
                    "instrument {} exists but its state could not be mirrored to the save \
                     snapshot in time",
                    instrument_id.as_u64()
                )
            } else {
                format!("instrument {} not found", instrument_id.as_u64())
            }
        })?;

    let engine_state = session.state();
    let shared_graph = &engine_state.shared_graph;
    // These filter-then-clone (only the target instrument's snapshots), cheaper
    // than get_all_modules()/get_connections() which clone the whole graph first.
    let modules = shared_graph.get_modules_for_instrument(instrument_id);
    let connections = shared_graph.get_connections_for_instrument(instrument_id);

    let patch = build_patch_from_engine(session, snap, &modules, &connections);

    ensure_parent_dir(path)?;
    patch.save(path).map_err(|e| e.to_string())?;

    Ok(format!(
        "Saved patch: {} ({}, {} module(s), {} connection(s))",
        path.display(),
        snap.name,
        patch.modules.len(),
        patch.connections.len()
    ))
}

/// Caller-supplied fields that are not derivable from engine state alone.
/// Headless / MCP callers fill in only what shared state knows (author);
/// the GUI also passes `glide_time` and `octave_offset` from its widget
/// state.
#[derive(Debug, Clone, Default)]
pub struct ProjectBuildOptions {
    /// Project author / composer. `None` writes a missing `author` field.
    pub author: Option<crate::patch::Author>,
    /// Global glide / portamento time. Defaults to `Seconds::ZERO` when
    /// unset (matches `GlobalProjectState::default()`).
    pub glide_time: Option<synth_core::Seconds>,
    /// Keyboard octave offset. Defaults to `0` when unset.
    pub octave_offset: Option<i32>,
}

/// Build a `ProjectFile` from current engine + session + song state,
/// in memory (no disk I/O). Engine-derived fields (instruments, module
/// graph, song, master volume) come from `session` / `song`; anything
/// not in engine state (author, octave, glide) comes from `opts`.
/// Module positions default to `Position::default()` — GUI callers
/// overlay their `PatchEditor` positions afterwards.
///
/// **Blocks briefly:** this first runs the shared save sync barrier
/// (`wait_for_pending_commands`, bounded to `SNAPSHOT_SYNC_TIMEOUT_MS`) so
/// the built project reflects graph mutations queued but not yet mirrored.
/// It returns *immediately* when the command queue is already caught up —
/// the normal case while the audio thread is draining — so it is safe to
/// call on the UI thread (GUI save / offline-export). The full timeout is
/// only ever spent if the audio thread has stopped draining entirely.
#[must_use]
pub fn build_project_from_engine(
    session: &SynthSession,
    song: &SharedSong,
    _sample_library: &SharedSampleLibrary,
    opts: ProjectBuildOptions,
) -> ProjectFile {
    // Drain graph mutations queued earlier (an MCP batch's add_module/connect, or
    // a GUI edit) before reading the async-mirrored shared_graph, so the built
    // project reflects every queued command instead of a stale/truncated graph.
    // This is the single sync barrier shared by *all* project save paths — the GUI
    // File-menu save (`create_project_from_app`), the MCP save (`save_project_to`
    // + `save_project_as_bundle`), and the rollback snapshot — so parity no longer
    // depends on each caller remembering to wait. Bounded; returns immediately once
    // the queue is caught up (the live case) or the audio thread is idle. Patch
    // save has its own barrier in `save_patch_to`.
    let _ = session.wait_for_pending_commands(SNAPSHOT_SYNC_TIMEOUT_MS);

    let snapshots = session.list_instruments();
    let engine_state = session.state();
    let shared_graph = &engine_state.shared_graph;

    // Acquire the two shared-graph read locks exactly once, then bucket by
    // instrument id locally. `get_modules_for_instrument` / `_connections_*`
    // each take their own lock and re-scan the entire collection — calling
    // them in a per-instrument loop would be O(N*M) lock churn.
    let mut modules_by_inst: std::collections::HashMap<InstrumentId, Vec<ModuleStateSnapshot>> =
        std::collections::HashMap::with_capacity(snapshots.len());
    for module in shared_graph.get_all_modules() {
        modules_by_inst
            .entry(module.instrument_id)
            .or_default()
            .push(module);
    }
    let mut conns_by_inst: std::collections::HashMap<InstrumentId, Vec<ConnectionSnapshot>> =
        std::collections::HashMap::with_capacity(snapshots.len());
    for conn in shared_graph.get_connections() {
        conns_by_inst
            .entry(conn.instrument_id)
            .or_default()
            .push(conn);
    }

    let mut instrument_states: Vec<InstrumentState> = Vec::with_capacity(snapshots.len());
    for snap in &snapshots {
        let modules = modules_by_inst.remove(&snap.id).unwrap_or_default();
        let connections = conns_by_inst.remove(&snap.id).unwrap_or_default();
        let patch = build_patch_from_engine(session, snap, &modules, &connections);
        instrument_states.push(snapshot_to_instrument_state(snap, patch));
    }

    let mut song_clone = { song.read().clone() };
    // Persist the transport loop region. The runtime loop is owned by the
    // sequencer engine (audio thread) and only mirrored into `TransportState`,
    // not into the shared Song — so capture it here or it is lost on save. A
    // non-positive span means "no loop set" and stores as `None`.
    let (loop_enabled, loop_start, loop_end) = engine_state.transport.loop_state();
    song_clone.set_transport_loop(
        (loop_end > loop_start).then_some(synth_sequencer::LoopRegion {
            start: loop_start,
            end: loop_end,
            enabled: loop_enabled,
        }),
    );
    let master_volume = synth_core::Gain::new(engine_state.master_volume.load());
    let return_bus_effects = build_return_bus_effects(&engine_state.return_bus_effects.read());
    let master_effects = build_effect_states(&engine_state.master_effects.read());
    let global = GlobalProjectState {
        master_volume,
        octave_offset: opts.octave_offset.unwrap_or(0),
        glide_time: opts.glide_time.unwrap_or(synth_core::Seconds::ZERO),
        return_bus_effects,
        master_effects,
    };

    let active_instrument_id = snapshots.first().map_or(0, |s| s.id.as_u64());
    ProjectFile::new(
        instrument_states,
        active_instrument_id,
        opts.author,
        song_clone,
        global,
    )
}

/// Translate an `InstrumentSnapshot` + freshly-built `Patch` into the
/// serializable `InstrumentState`. Mirrors `create_project_from_app`'s
/// per-instrument struct construction but reads exclusively from the engine
/// snapshot — no `InstrumentUiState` involvement.
fn snapshot_to_instrument_state(snap: &InstrumentSnapshot, mut patch: Patch) -> InstrumentState {
    patch.description = snap.patch_description.clone();
    patch.color = snap.patch_color.clone();
    InstrumentState {
        id: snap.id,
        name: snap.name.clone(),
        channel: snap.midi_channel.map_or(0, synth_core::MidiChannel::as_u8),
        volume: snap.volume,
        pan: snap.pan,
        muted: snap.muted,
        solo: snap.solo,
        key_range: (snap.key_range.low.as_u8(), snap.key_range.high.as_u8()),
        transpose: snap.transpose,
        oversampling: snap.oversampling.factor() as u8,
        category: snap.category.as_u8(),
        description: snap.description.clone(),
        color: snap.color.clone(),
        allocation_mode: snap.allocation_mode,
        stealing_strategy: snap.stealing_strategy,
        unison_detune: snap.unison_detune,
        unison_spread: snap.unison_spread,
        max_voices: snap.max_voices,
        velocity_amp_sensitivity: snap.velocity_amp_sensitivity,
        velocity_filter_sensitivity: snap.velocity_filter_sensitivity,
        sidechain_source_id: snap.sidechain_source_id.map(|id| id.as_u64()),
        patch,
    }
}

/// Build a `Patch` for one instrument from the engine's shared graph state.
///
/// Mirrors `patch_bridge::create_patch_from_editor` but skips the
/// `PatchEditor` (canvas positions, group metadata): headless has none. For
/// each module we look up its descriptor on `SynthSession` to map engine
/// `Param` values to descriptor `type_id` keys — same scheme the GUI uses, so
/// the saved JSON round-trips cleanly through `apply_patch` on load.
fn build_patch_from_engine(
    session: &SynthSession,
    snap: &InstrumentSnapshot,
    modules: &[ModuleStateSnapshot],
    connections: &[ConnectionSnapshot],
) -> Patch {
    let mut patch = Patch::new(&snap.name);
    patch.description = snap.patch_description.clone();
    patch.color = snap.patch_color.clone();

    // Emit modules in stable order: `ModuleId: Ord` sorts by (module_type,
    // instance). Same ordering as the GUI's `create_patch_from_editor`, so
    // GUI-saved and headless-saved JSON for the same engine state are
    // structurally identical.
    let mut sorted_modules: Vec<&ModuleStateSnapshot> = modules.iter().collect();
    sorted_modules.sort_by_key(|m| m.id);

    for module in sorted_modules {
        let param_map = session
            .module_descriptor(snap.id, module.id)
            .map(|d| map_params_to_values(&d.parameters, &module.parameters))
            .unwrap_or_default();
        patch.modules.push(ModuleState {
            id: module.id.to_string(),
            module_type: module.module_type,
            position: Position::default(),
            // Per-instance description round-trips via the snapshot.
            description: module.description.clone(),
            parameters: param_map,
            // Per-slot control scripts (Step 2) round-trip via the snapshot.
            scripts: module.scripts.clone(),
        });
    }

    // Sort connections by (from_module, from_port, to_module, to_port) so
    // re-saving an unchanged engine state produces byte-identical JSON.
    let mut sorted_conns: Vec<&ConnectionSnapshot> = connections.iter().collect();
    sorted_conns.sort_by_cached_key(|c| {
        let from_port: String = c.from_port.into();
        let to_port: String = c.to_port.into();
        (c.from_module, from_port, c.to_module, to_port)
    });
    patch.connections = sorted_conns
        .iter()
        .map(|c| ConnectionState {
            from: (c.from_module.to_string(), c.from_port.into()),
            to: (c.to_module.to_string(), c.to_port.into()),
        })
        .collect();

    patch.settings.effect_chain_order = snap
        .effect_chain_order
        .iter()
        .map(ModuleId::to_string)
        .collect();

    patch
}

/// Map a module's live engine `Param` values onto its descriptor's parameters,
/// keyed by `type_id`, for serialization. Choice params serialize by id (via
/// `ParamValue::from_param`). Shared by instrument-patch and return-effect save.
fn map_params_to_values(
    descriptor_params: &[ParameterDescriptor],
    engine_params: &[Param],
) -> BTreeMap<String, ParamValue> {
    let mut map: BTreeMap<String, ParamValue> = BTreeMap::new();
    for desc_param in descriptor_params {
        if let Some(engine_param) = engine_params.iter().find(|p| p.same_kind(&desc_param.id)) {
            map.insert(
                desc_param.type_id.clone(),
                ParamValue::from_param(engine_param, desc_param),
            );
        }
    }
    map
}

/// Convert the engine's published return-bus effect snapshots into the
/// serializable project form, using a type-derived descriptor (from
/// `create_effect`) to map param values exactly like instrument effects.
fn build_return_bus_effects(snaps: &[ReturnBusSnapshot]) -> Vec<ReturnBusEffectsState> {
    snaps
        .iter()
        .map(|bus| ReturnBusEffectsState {
            id: bus.id.0,
            effects: build_effect_states(&bus.effects),
        })
        .collect()
}

/// Convert a flat engine effect-chain snapshot (return bus or master) into the
/// serializable `ModuleState` list, mapping params via a type-derived descriptor.
fn build_effect_states(effects: &[synth_engine::ReturnEffectSnapshot]) -> Vec<ModuleState> {
    effects
        .iter()
        .filter_map(|fx| {
            let (_, descriptor) = crate::module_factory::create_effect(fx.module_type)?;
            Some(ModuleState {
                id: fx.module_id.to_string(),
                module_type: fx.module_type,
                position: Position::default(),
                description: String::new(),
                parameters: map_params_to_values(&descriptor.parameters, &fx.parameters),
                scripts: std::collections::BTreeMap::new(),
            })
        })
        .collect()
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

pub(crate) fn install_instrument(
    session: &SynthSession,
    sender: &CommandSender,
    inst_state: &InstrumentState,
) -> Result<(), String> {
    let inst_id = inst_state.id;
    let cfg = synth_engine::voice_allocator::AllocatorConfig {
        max_voices: inst_state.max_voices,
        mode: inst_state.allocation_mode,
        stealing: inst_state.stealing_strategy,
        unison_detune: inst_state.unison_detune,
        unison_spread: inst_state.unison_spread,
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
        let _ = sender.send_blocking(EngineCommand::SetEffectChainOrder {
            instrument_id: Some(inst_id),
            order,
        });
    }

    apply_instrument_metadata(session, inst_state, inst_id);
    push_instrument_params(sender, inst_state, inst_id);

    // Default-enable on install — otherwise the instrument produces no
    // audio until something else (solo/mute logic, manual toggle) flips
    // the flag.
    sender.send(EngineCommand::SetInstrumentEnabled {
        instrument_id: inst_id,
        enabled: true,
    });

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
    let channel = if inst_state.channel == 0 {
        MidiChannelSelection::OMNI
    } else {
        MidiChannelSelection::from_one_indexed(inst_state.channel)
            .unwrap_or(MidiChannelSelection::CH1)
    };
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
    if let Some(color) = inst_state.color.as_deref()
        && !color.is_empty()
    {
        log_err(
            "set_instrument_color",
            session.set_instrument_color(inst_id, Some(color)),
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
    if let Some(patch_color) = inst_state.patch.color.as_deref()
        && !patch_color.is_empty()
    {
        log_err(
            "set_patch_color",
            session.set_patch_color(inst_id, Some(patch_color)),
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
        InstrumentParam::UnisonDetune(inst_state.unison_detune),
        InstrumentParam::UnisonSpread(inst_state.unison_spread),
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
    // The per-instrument state above is cleaned by `remove_instrument`, but the
    // monotonic instrument-ID counter is not — reset it so the next project (or
    // a fresh New Project) starts numbering from 1 again rather than continuing
    // from the torn-down project's high-water mark.
    session.reset_instrument_counter();
}

/// Send `LoadSampleData` for every Sampler module in the project that
/// references a non-zero sample id — `set_parameter` on a sampler only stores
/// the id; the engine also needs the audio buffer. Mirrors
/// `egui_backend::send_loaded_sample_data`. Also called by the offline WAV
/// export so its samplers render audio instead of silence.
pub(crate) fn push_loaded_sample_data(
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

/// Collect `SampleId`s referenced by live `Sampler` modules. Live-engine
/// counterpart of `push_loaded_sample_data` (which reads a `ProjectFile`).
pub(crate) fn collect_used_sample_ids(
    session: &SynthSession,
) -> std::collections::HashSet<synth_sampler::SampleId> {
    let state = session.state();
    session
        .list_instruments()
        .into_iter()
        .flat_map(|snap| state.shared_graph.get_modules_for_instrument(snap.id))
        .filter(|m| m.module_type == ModuleType::Sampler)
        .filter_map(|m| crate::audio::preview::sampler_sample_id(&m))
        .collect()
}

/// Drop samples no `Sampler` module currently references. Returns the
/// removed samples' names, ordered by `SampleId` for deterministic
/// output. A non-empty library forces the next save into bundle
/// format, so pruning here keeps round-trips on plain JSON when the
/// user never needed samples in the first place.
pub(crate) fn prune_unused_samples(
    session: &SynthSession,
    sample_library: &SharedSampleLibrary,
) -> Vec<String> {
    // Empty library is the common case for sample-free projects; bail
    // before the engine walk so the menu action stays cheap.
    if sample_library
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty()
    {
        return Vec::new();
    }

    let used = collect_used_sample_ids(session);
    let mut lib = sample_library.write().unwrap_or_else(|e| e.into_inner());
    let mut to_remove: Vec<(synth_sampler::SampleId, String)> = lib
        .list()
        .iter()
        .filter(|m| !used.contains(&m.id))
        .map(|m| (m.id, m.name.clone()))
        .collect();
    to_remove.sort_by_key(|(id, _)| id.as_u64());
    for (id, _) in &to_remove {
        lib.remove(*id);
    }
    to_remove.into_iter().map(|(_, name)| name).collect()
}

/// Bounded window a snapshot-reading save waits for the audio thread to mirror a
/// just-created instrument (matches `apply_project`'s post-load bridge).
const SNAPSHOT_SYNC_TIMEOUT_MS: u64 = 2_000;

/// Poll `session` (5 ms tick) until `done` holds or `timeout_ms` elapses.
/// Bridges the gap between synchronous engine commands and the audio thread
/// mirroring them into the instrument snapshot the load/save paths read.
fn wait_until(session: &SynthSession, timeout_ms: u64, done: impl Fn(&SynthSession) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    while std::time::Instant::now() < deadline {
        if done(session) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn wait_for_instrument_count(session: &SynthSession, target: usize, timeout_ms: u64) {
    wait_until(session, timeout_ms, |s| {
        s.list_instruments().len() >= target
    });
}

fn wait_for_instrument(session: &SynthSession, instrument_id: InstrumentId, timeout_ms: u64) {
    wait_until(session, timeout_ms, |s| {
        s.list_instruments().iter().any(|i| i.id == instrument_id)
    });
}

fn log_err<T>(op: &str, result: Result<T, SessionError>) {
    if let Err(e) = result {
        eprintln!("{op}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use synth_core::audio::DeviceSampleRate as HwSampleRate;
    use synth_core::{
        AudioCallbackContext, AudioProcessor, BipolarValue, MidiNote, NormalizedValue,
    };
    use synth_engine::SynthEngine;
    use synth_engine::instrument::KeyRange;
    use synth_engine::voice_allocator::{AllocationMode, StealingStrategy};

    use crate::patch::ModuleBuilder;

    const TEST_SR: u32 = 44_100;

    /// Build a minimal patch (Osc → Amp → StereoOutput) so the saved JSON
    /// has at least one module + connections to exercise the engine→Patch
    /// translation in `build_patch_from_engine`.
    fn minimal_patch(name: &str) -> Patch {
        let mut patch = Patch::new(name);
        patch.add_module(
            ModuleBuilder::new(1, ModuleType::Oscillator)
                .waveform("sawtooth")
                .param_f("level", 0.7)
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
        patch.add_connection("amp-1", "out_l", "out-1", "in_l");
        patch.add_connection("amp-1", "out_r", "out-1", "in_r");
        patch
    }

    fn drive(engine: &mut SynthEngine, blocks: usize) {
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

    /// A `NoteScriptTransform`'s compiled program is `#[serde(skip)]`, so a
    /// project loaded from disk starts with the node pass-through. The load-time
    /// `recompile_note_graph_scripts` step must rebuild the program from the
    /// saved `source` — otherwise a saved script silently stops transforming
    /// notes after a reload. This exercises the serde round-trip + recompile and
    /// asserts the reloaded graph both reports compiled and actually transforms.
    #[test]
    fn reloaded_note_script_recompiles_and_transforms() {
        use synth_sequencer::{
            Duration as SeqDuration, NoteModuleConfig, NoteModuleId, NoteScriptTransform,
            PatternTick, Pitch, Velocity,
        };

        // Build a song whose pooled graph has a single up-an-octave script node.
        let mut song = Song::new("Scripted Notes");
        let gid = song.create_note_graph("octave up");
        song.note_graph_mut(gid)
            .expect("graph")
            .try_insert_node(
                NoteModuleId::new(0),
                NoteModuleConfig::NoteScriptTransform(NoteScriptTransform::new(
                    "out.pitch = note_pitch + 12",
                )),
            )
            .expect("insert script node");

        // Round-trip through JSON — the compiled program does not serialize, so
        // the reloaded node is uncompiled (pass-through) until recompiled.
        let json = serde_json::to_string(&song).expect("serialize song");
        let mut reloaded: Song = serde_json::from_str(&json).expect("deserialize song");
        reloaded.rebuild_note_graphs();
        assert!(
            !script_node_compiled(&reloaded, gid),
            "a freshly-deserialized script node must start uncompiled"
        );

        // The load-time recompile step installs the program.
        recompile_note_graph_scripts(&mut reloaded);
        assert!(
            script_node_compiled(&reloaded, gid),
            "recompile must install the program from the saved source"
        );

        // And it must actually transform: C4 (60) → C5 (72) after freezing the
        // bound pattern through the graph.
        let pid = reloaded.create_pattern(SeqDuration(960));
        {
            let p = reloaded.pattern_mut(pid).expect("pattern");
            let _ = p.add_note(PatternTick(0), Pitch::new(60).expect("pitch"), Velocity::MF);
            p.set_note_graph(Some(gid));
        }
        assert!(
            reloaded
                .freeze_pattern_note_graph(pid, synth_core::Bpm::DEFAULT)
                .is_some()
        );
        let pitches: Vec<u8> = reloaded
            .pattern(pid)
            .expect("pattern")
            .notes()
            .iter()
            .map(|n| n.pitch.as_midi())
            .collect();
        assert_eq!(
            pitches,
            vec![72],
            "the recompiled script must raise the note an octave"
        );
    }

    /// Whether the given graph's single script node reports a compiled program.
    fn script_node_compiled(song: &Song, gid: synth_sequencer::NoteGraphId) -> bool {
        song.note_graph(gid)
            .expect("graph present")
            .nodes()
            .values()
            .any(|c| match c {
                synth_sequencer::NoteModuleConfig::NoteScriptTransform(t) => t.is_compiled(),
                _ => false,
            })
    }

    /// New Project must reset the instrument-ID counter: after tearing down a
    /// project whose instruments climbed the counter, the next `add_instrument`
    /// should start from a low ID again rather than continuing the high-water
    /// mark (the bug where a fresh project's first instrument got ID 41).
    #[test]
    fn new_project_resets_instrument_id_counter() {
        let (mut engine, handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
        let song: SharedSong = Arc::new(synth_sequencer::SharedSong::new(Song::new("Counter")));
        let sample_library: SharedSampleLibrary = Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        ));

        // Climb the counter: three instruments → next free ID is 4.
        for name in ["a", "b", "c"] {
            let _ = session.add_instrument(name).expect("add_instrument");
        }
        drive(&mut engine, 2);

        reset_to_new_project(&session, &song, &sample_library).expect("new project");
        drive(&mut engine, 2);

        let fresh = session.add_instrument("fresh").expect("add after reset");
        assert_eq!(
            fresh,
            InstrumentId::new(1),
            "New Project should restart instrument IDs from 1"
        );
    }

    /// A YAMS control script installed on a Mod Matrix slot survives the engine
    /// → snapshot → save path: `set_mod_script` refreshes the shared snapshot
    /// (`ModuleStateSnapshot.scripts`), and `build_patch_from_engine` persists it
    /// into the project's `ModuleState.scripts` by 1-based slot key (S2.2b-2iv).
    #[test]
    fn save_project_round_trips_control_scripts() {
        let (mut engine, handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
        let song: SharedSong = Arc::new(synth_sequencer::SharedSong::new(Song::new("Scripts")));
        let sample_library: SharedSampleLibrary = Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        ));

        let id = InstrumentId::new(0);
        session
            .add_instrument_with_id(id, "Scripted")
            .expect("add instrument");
        // Host the script on a Mod Matrix module.
        let mut patch = minimal_patch("Scripted Patch");
        patch.add_module(ModuleBuilder::new(1, ModuleType::ModMatrix).build());
        let _ = session.apply_patch(id, &patch);

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(TEST_SR),
            buffer_size: synth_core::BufferSize::new(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);
        drive(&mut engine, 4);

        // Install on slot 1 (0-based 0), then drain so the snapshot refreshes.
        session
            .set_mod_script(
                id,
                "mmx-1".parse().expect("mmx id"),
                0,
                "out = velocity * 0.5",
            )
            .expect("set_mod_script");
        drive(&mut engine, 4);

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scripts.json");
        save_project_to(
            &path,
            &session,
            &song,
            &sample_library,
            ProjectBuildOptions::default(),
        )
        .expect("save_project_to");

        let json = std::fs::read_to_string(&path).expect("read saved project");
        let compact = serde_json::from_str::<serde_json::Value>(&json)
            .expect("parse saved project")
            .to_string();
        assert!(
            compact.contains(r#""scripts":{"1":"out = velocity * 0.5"}"#),
            "saved project must round-trip the control script; got: {compact}"
        );
    }

    /// A saved patch's control scripts must survive being *loaded back* — the
    /// save side is covered above, but nothing exercised the load side, where
    /// `apply_patch` re-installs each slot script. Regression guard for the
    /// script-load bug (glyphs/markers vanished after reload).
    #[test]
    fn load_patch_round_trips_control_scripts() {
        let (mut engine, handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
        let song: SharedSong = Arc::new(synth_sequencer::SharedSong::new(Song::new("Scripts")));
        let sample_library: SharedSampleLibrary = Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        ));

        let id = InstrumentId::new(0);
        session
            .add_instrument_with_id(id, "Scripted")
            .expect("add instrument");
        let mut patch = minimal_patch("Scripted Patch");
        patch.add_module(ModuleBuilder::new(1, ModuleType::ModMatrix).build());
        let _ = session.apply_patch(id, &patch);

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(TEST_SR),
            buffer_size: synth_core::BufferSize::new(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);
        drive(&mut engine, 4);

        session
            .set_mod_script(
                id,
                "mmx-1".parse().expect("mmx id"),
                0,
                "out = velocity * 0.5",
            )
            .expect("set_mod_script");
        drive(&mut engine, 4);

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scripts.json");
        save_patch_to(&path, &session, id).expect("save_patch_to");
        assert!(
            std::fs::read_to_string(&path)
                .expect("read saved patch")
                .contains("out = velocity * 0.5"),
            "sanity: the saved patch file must contain the script"
        );

        // Load the patch back through the real load path, then let the queued
        // clear/add/param/script commands drain.
        load_file_into_engine(&path, &session, &song, &sample_library).expect("load patch");
        drive(&mut engine, 16);

        // Re-serialise the reloaded instrument; the script must have survived.
        let path2 = dir.path().join("scripts_reloaded.json");
        save_patch_to(&path2, &session, InstrumentId::FIRST).expect("save reloaded");
        let reloaded = std::fs::read_to_string(&path2).expect("read reloaded patch");
        assert!(
            reloaded.contains("out = velocity * 0.5"),
            "loading a patch must restore its control scripts; got: {reloaded}"
        );
    }

    /// The GUI load path (`patch_bridge::load_patch`) builds the graph
    /// module-by-module rather than via `session.apply_patch`, and previously
    /// skipped per-slot control scripts entirely — so a saved patch's Mod Matrix /
    /// Script / AudioScript slot scripts silently vanished on load in the app
    /// (their marker glyphs disappeared). Guards that the GUI path now installs
    /// them. Note the *headless* path (`load_patch_round_trips_control_scripts`)
    /// already worked — this covers the separate GUI code path.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn gui_load_patch_installs_control_scripts() {
        use crate::gui::keyboard::PianoKeyboard;
        use crate::gui::patch_editor::PatchEditor;

        let (mut engine, mut handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
        let id = InstrumentId::new(0);
        session
            .add_instrument_with_id(id, "Scripted")
            .expect("add instrument");

        let mut patch = minimal_patch("Scripted Patch");
        let mut mm = ModuleBuilder::new(1, ModuleType::ModMatrix).build();
        mm.scripts
            .insert("1".to_string(), "out = velocity * 0.5".to_string());
        patch.add_module(mm);
        // Round-trip through JSON so the test also covers on-disk deserialization
        // of `scripts` (the real load reads a file), not just an in-memory patch.
        let patch: Patch =
            serde_json::from_str(&serde_json::to_string(&patch).expect("serialize patch"))
                .expect("deserialize patch");
        let mmx = patch
            .modules
            .iter()
            .find(|m| m.id == "mmx-1")
            .expect("mmx module present after round-trip");
        assert_eq!(
            mmx.scripts.get("1").map(String::as_str),
            Some("out = velocity * 0.5"),
            "scripts must survive JSON round-trip"
        );

        let mut editor = PatchEditor::new();
        let mut keyboard = PianoKeyboard::new();
        let mut glide = synth_core::Seconds::ZERO;
        crate::gui::patch_bridge::load_patch(
            &patch,
            &mut editor,
            &session,
            &mut handle,
            &mut keyboard,
            &mut glide,
            id,
            false,
        );

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(TEST_SR),
            buffer_size: synth_core::BufferSize::new(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);
        drive(&mut engine, 16);

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gui_scripts.json");
        save_patch_to(&path, &session, id).expect("save_patch_to");
        let json = std::fs::read_to_string(&path).expect("read saved patch");
        assert!(
            json.contains("out = velocity * 0.5"),
            "GUI load_patch must install slot control scripts; got: {json}"
        );
    }

    /// Faithful repro of the real failure: a Script module and a Mod Matrix slot
    /// whose scripts *reference other modules* (`src o = osc-1.out`), loaded via
    /// the GUI path. The earlier test used a reference-free script and passed even
    /// while the app still dropped these.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn gui_load_patch_installs_module_referencing_scripts() {
        use crate::gui::keyboard::PianoKeyboard;
        use crate::gui::patch_editor::PatchEditor;

        let (mut engine, mut handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
        let id = InstrumentId::new(0);
        session
            .add_instrument_with_id(id, "Scripted")
            .expect("add instrument");

        let mut patch = minimal_patch("Scripted Patch");
        let mut scr = ModuleBuilder::new(1, ModuleType::Script).build();
        scr.scripts
            .insert("1".to_string(), "src o = osc-1.out\nout = o".to_string());
        patch.add_module(scr);
        let mut mm = ModuleBuilder::new(1, ModuleType::ModMatrix).build();
        mm.scripts
            .insert("1".to_string(), "src a = amp-1.level\nout = a".to_string());
        patch.add_module(mm);
        let patch: Patch = serde_json::from_str(&serde_json::to_string(&patch).expect("serialize"))
            .expect("deserialize");

        let mut editor = PatchEditor::new();
        let mut keyboard = PianoKeyboard::new();
        let mut glide = synth_core::Seconds::ZERO;
        crate::gui::patch_bridge::load_patch(
            &patch,
            &mut editor,
            &session,
            &mut handle,
            &mut keyboard,
            &mut glide,
            id,
            false,
        );

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(TEST_SR),
            buffer_size: synth_core::BufferSize::new(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);
        drive(&mut engine, 16);

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("gui_ref_scripts.json");
        save_patch_to(&path, &session, id).expect("save_patch_to");
        let json = std::fs::read_to_string(&path).expect("read saved patch");
        assert!(
            json.contains("src o = osc-1.out"),
            "Script-module slot script must install on GUI load; got: {json}"
        );
        assert!(
            json.contains("src a = amp-1.level"),
            "Mod Matrix slot script must install on GUI load; got: {json}"
        );
    }

    /// The GUI *save* path (`create_patch_from_editor`) must capture per-slot
    /// control scripts from the engine snapshot — it previously hardcoded an empty
    /// map, so "Save Patch…" silently stripped every slot script (the saved file
    /// then loaded scriptless, markers gone). Complements the load-side guards.
    #[cfg(feature = "gui-egui")]
    #[test]
    fn gui_save_patch_captures_control_scripts() {
        use crate::gui::patch_editor::PatchEditor;

        let (mut engine, handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
        let id = InstrumentId::new(0);
        session
            .add_instrument_with_id(id, "Scripted")
            .expect("add instrument");

        let mmx = ModuleId::new(ModuleType::ModMatrix, 1);
        let descriptor = session
            .add_module_with_id(id, mmx, ModuleType::ModMatrix)
            .expect("add module");
        session
            .set_mod_script(id, mmx, 0, "out = velocity * 0.5")
            .expect("set_mod_script");

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(TEST_SR),
            buffer_size: synth_core::BufferSize::new(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);
        drive(&mut engine, 16);

        let mut editor = PatchEditor::new();
        editor.add_module_at(mmx, descriptor, eframe::egui::Pos2::ZERO);

        let patch = crate::gui::patch_bridge::create_patch_from_editor(
            "Scripted",
            &editor,
            Some((&**session.state(), id)),
        );
        let m = patch
            .modules
            .iter()
            .find(|m| m.id == "mmx-1")
            .expect("mmx module present in saved patch");
        assert_eq!(
            m.scripts.get("1").map(String::as_str),
            Some("out = velocity * 0.5"),
            "GUI Save Patch must capture slot scripts from the engine snapshot"
        );
    }

    /// The command-drain barrier (`SynthSession::wait_for_pending_commands`)
    /// tracks the exact staleness the save path must avoid: a graph mutation
    /// queued on an already-published instrument is visible in `shared_graph`
    /// immediately, while the barrier still reports "not caught up" (`false`)
    /// until the DSP mutation reaches the audio thread.
    #[test]
    fn pending_command_wait_tracks_graph_mutation_drain() {
        let (mut engine, handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

        let id = InstrumentId::new(0);
        session
            .add_instrument_with_id(id, "X")
            .expect("add instrument");
        let _ = session.apply_patch(id, &minimal_patch("X"));

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(TEST_SR),
            buffer_size: synth_core::BufferSize::new(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);
        drive(&mut engine, 4);
        // Everything queued so far is drained.
        assert!(
            session.wait_for_pending_commands(0),
            "wait must report caught-up once the queue is drained"
        );

        let before = session
            .state()
            .shared_graph
            .get_modules_for_instrument(id)
            .len();

        // Queue a mutation on the existing instrument, then do not drive. The
        // control snapshot is synchronous while the DSP barrier remains open.
        session
            .add_module(id, ModuleType::Lfo)
            .expect("queue add_module");
        assert!(
            !session.wait_for_pending_commands(30),
            "an undrained mutation must report not-caught-up"
        );
        assert_eq!(
            session
                .state()
                .shared_graph
                .get_modules_for_instrument(id)
                .len(),
            before + 1,
            "shared_graph publishes on the control thread before audio drain"
        );

        // Drive: the DSP mutation drains and the barrier catches up. The
        // already-published snapshot remains stable.
        drive(&mut engine, 4);
        assert!(
            session.wait_for_pending_commands(0),
            "wait must catch up after the queue drains"
        );
        assert_eq!(
            session
                .state()
                .shared_graph
                .get_modules_for_instrument(id)
                .len(),
            before + 1,
            "audio drain must not duplicate the control snapshot"
        );
    }

    /// `save_patch_to` writes a single instrument's graph as a standalone patch
    /// file that `load_file` reads back as `LoadedFile::Patch` (not a project),
    /// carrying exactly that instrument's modules + connections.
    #[test]
    fn save_patch_writes_a_loadable_single_instrument_patch() {
        use crate::project::{LoadedFile, load_file};

        let (mut engine, handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));

        let lead = InstrumentId::new(0);
        session
            .add_instrument_with_id(lead, "Lead")
            .expect("add lead");
        let _ = session.apply_patch(lead, &minimal_patch("Lead Patch"));

        // A second instrument with a distinctive module (an LFO the lead lacks),
        // so the save must actually scope to `lead` — an unscoped or inverted
        // filter would pull this module into the saved patch.
        let other = InstrumentId::new(1);
        session
            .add_instrument_with_id(other, "Other")
            .expect("add other");
        let mut other_patch = minimal_patch("Other Patch");
        other_patch.add_module(ModuleBuilder::new(1, ModuleType::Lfo).build());
        let _ = session.apply_patch(other, &other_patch);

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(TEST_SR),
            buffer_size: synth_core::BufferSize::new(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);
        drive(&mut engine, 4);

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("lead.json");
        let msg = save_patch_to(&path, &session, lead).expect("save_patch_to");
        assert!(msg.contains("Lead"), "message names the instrument: {msg}");

        match load_file(&path).expect("load saved patch") {
            LoadedFile::Patch(loaded) => {
                // Patch name mirrors the instrument name (build_patch_from_engine).
                assert_eq!(loaded.name, "Lead");
                assert_eq!(loaded.modules.len(), 3, "only the lead's osc + amp + out");
                assert_eq!(loaded.connections.len(), 3, "all three cables persisted");
                assert!(
                    loaded
                        .modules
                        .iter()
                        .all(|m| m.module_type != ModuleType::Lfo),
                    "the other instrument's LFO must not leak into the lead's patch"
                );
            }
            _ => panic!("expected a single-instrument patch, got a different file kind"),
        }
    }

    /// Spin up a fresh engine, install two instruments with patches and
    /// non-default per-instrument settings, drain a few process() blocks so
    /// `update_shared_instruments` mirrors everything into the snapshot, then
    /// save + reload and assert all seven new persisted fields round-trip.
    #[test]
    fn save_project_round_trip_preserves_extended_instrument_fields() {
        let (mut engine, handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
        let song: SharedSong = Arc::new(synth_sequencer::SharedSong::new(Song::new("RoundTrip")));
        let sample_library: SharedSampleLibrary = Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        ));

        // --- Instrument 0: lead (Mono / SameNote, custom key range, oversampling 2x) ---
        let lead_id = InstrumentId::new(0);
        let lead_cfg = synth_engine::voice_allocator::AllocatorConfig {
            max_voices: VoiceCount::new(6),
            mode: AllocationMode::Mono,
            stealing: StealingStrategy::SameNote,
            ..Default::default()
        };
        session
            .add_instrument_with_id_and_config(lead_id, "Lead", Some(lead_cfg))
            .expect("add lead");
        let _ = session.apply_patch(lead_id, &minimal_patch("Lead Patch"));
        session
            .set_instrument_volume(lead_id, synth_core::Gain::new(0.42))
            .expect("set lead volume");
        session
            .set_instrument_pan(lead_id, BipolarValue::new(-0.5))
            .expect("set lead pan");

        let lead_param_cmds = [
            InstrumentParam::KeyRange(KeyRange::new(MidiNote::new(36), MidiNote::new(72))),
            InstrumentParam::Transpose(synth_core::Semitones::new(7.0)),
            InstrumentParam::OversamplingFactor(synth_dsp::OversamplingFactor::X2),
            InstrumentParam::VelocityAmpSensitivity(NormalizedValue::new(0.75)),
            InstrumentParam::VelocityFilterSensitivity(NormalizedValue::new(0.5)),
        ];
        for param in lead_param_cmds {
            session
                .command_sender()
                .send(EngineCommand::SetInstrumentParameter {
                    instrument_id: lead_id,
                    param,
                });
        }

        // --- Instrument 1: pad (Unison / Oldest, 4 voices) ---
        let pad_id = InstrumentId::new(1);
        let pad_cfg = synth_engine::voice_allocator::AllocatorConfig {
            max_voices: VoiceCount::new(4),
            mode: AllocationMode::Unison,
            stealing: StealingStrategy::Oldest,
            ..Default::default()
        };
        session
            .add_instrument_with_id_and_config(pad_id, "Pad", Some(pad_cfg))
            .expect("add pad");
        let _ = session.apply_patch(pad_id, &minimal_patch("Pad Patch"));
        session
            .command_sender()
            .send(EngineCommand::SetInstrumentParameter {
                instrument_id: pad_id,
                param: InstrumentParam::KeyRange(KeyRange::new(
                    MidiNote::new(48),
                    MidiNote::new(96),
                )),
            });

        // Drain so add_instrument / apply_patch / SetInstrumentParameter all
        // land and `update_shared_instruments` mirrors the post-state into
        // `InstrumentSnapshot`.
        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(TEST_SR),
            buffer_size: synth_core::BufferSize::new(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);
        drive(&mut engine, 32);

        // --- Save ---
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("round_trip.json");
        let save_msg = save_project_to(
            &path,
            &session,
            &song,
            &sample_library,
            ProjectBuildOptions::default(),
        )
        .expect("save_project_to should succeed in headless mode");
        assert!(save_msg.contains("Saved project"));
        assert!(path.exists(), "saved file must exist at {}", path.display());

        // --- Reload as raw ProjectFile and validate the persisted JSON ---
        let reloaded = ProjectFile::load(&path).expect("reload project");
        assert_eq!(
            reloaded.instruments.len(),
            2,
            "two instruments round-tripped"
        );

        let lead = reloaded
            .instruments
            .iter()
            .find(|i| i.name == "Lead")
            .expect("Lead survives");
        assert_eq!(lead.key_range, (36, 72), "lead key_range");
        assert!(
            (lead.transpose.as_f32() - 7.0).abs() < f32::EPSILON,
            "lead transpose 7 semitones"
        );
        assert_eq!(lead.oversampling, 2, "lead 2x oversampling");
        assert_eq!(lead.allocation_mode, AllocationMode::Mono);
        assert_eq!(lead.stealing_strategy, StealingStrategy::SameNote);
        assert_eq!(lead.max_voices, VoiceCount::new(6));
        assert!(
            (lead.velocity_amp_sensitivity.as_f32() - 0.75).abs() < 1e-6,
            "lead velocity_amp_sensitivity"
        );
        assert!(
            (lead.velocity_filter_sensitivity.as_f32() - 0.5).abs() < 1e-6,
            "lead velocity_filter_sensitivity"
        );

        let pad = reloaded
            .instruments
            .iter()
            .find(|i| i.name == "Pad")
            .expect("Pad survives");
        assert_eq!(pad.key_range, (48, 96), "pad key_range");
        assert_eq!(pad.allocation_mode, AllocationMode::Unison);
        assert_eq!(pad.max_voices, VoiceCount::new(4));

        // --- Round-trip 2: reapply the saved project to a fresh engine and
        // confirm the engine snapshot of the lead instrument matches what we
        // wrote in the first place (the JSON is round-trippable end-to-end).
        let (mut engine2, handle2) = SynthEngine::new();
        let session2 = SynthSession::new(handle2.command_sender(), Arc::clone(&handle2.state));
        let song2: SharedSong = Arc::new(synth_sequencer::SharedSong::new(Song::new("RoundTrip2")));
        let sample_library2: SharedSampleLibrary = Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        ));
        engine2.on_stream_start(&stream_info);
        apply_project(&reloaded, &session2, &song2, &sample_library2)
            .expect("apply reloaded project");
        drive(&mut engine2, 32);

        let reapplied = session2.list_instruments();
        assert_eq!(reapplied.len(), 2, "reapplied two instruments");
        let lead2 = reapplied
            .iter()
            .find(|s| s.name == "Lead")
            .expect("lead survives reapply");
        assert_eq!(lead2.key_range.low.as_u8(), 36);
        assert_eq!(lead2.key_range.high.as_u8(), 72);
        assert_eq!(lead2.allocation_mode, AllocationMode::Mono);
        assert_eq!(lead2.max_voices, VoiceCount::new(6));
        assert!(
            (lead2.velocity_amp_sensitivity.as_f32() - 0.75).abs() < 1e-6,
            "velocity_amp_sensitivity round-trips through engine"
        );
    }

    #[test]
    fn prune_unused_samples_keeps_referenced_drops_orphans() {
        use synth_sampler::Sample;
        use synth_sampler::{SampleMeta, SampleSource};

        fn make_sample(name: &str) -> Sample {
            Sample::new(
                SampleMeta {
                    id: synth_sampler::SampleId::new(0),
                    name: name.to_string(),
                    description: String::new(),
                    sample_rate: HwSampleRate::new(44_100),
                    channels: synth_core::ChannelCount::Mono,
                    frame_count: synth_core::SampleCount::new(100),
                    root_note: None,
                    loop_region: None,
                    crop: None,
                    source: SampleSource::Generated,
                },
                vec![0.0_f32; 100].into(),
            )
        }

        let (mut engine, handle) = SynthEngine::new();
        let session = SynthSession::new(handle.command_sender(), Arc::clone(&handle.state));
        let sample_library: SharedSampleLibrary = Arc::new(std::sync::RwLock::new(
            synth_sampler::SampleLibrary::default(),
        ));

        // Two samples live in the library; only "kept" is referenced.
        let (kept_id, orphan_id) = {
            let mut lib = sample_library.write().unwrap();
            (lib.add(make_sample("kept")), lib.add(make_sample("orphan")))
        };

        let inst_id = InstrumentId::new(0);
        session
            .add_instrument_with_id_and_config(inst_id, "Inst", None)
            .expect("add instrument");
        let (mod_id, _) = session
            .add_module(inst_id, ModuleType::Sampler)
            .expect("add Sampler module");
        session
            .command_sender()
            .send(EngineCommand::SetModuleParameter {
                instrument_id: Some(inst_id),
                module_id: mod_id,
                param: synth_core::Param::sample_select(kept_id.as_u64()),
            });

        let stream_info = synth_core::StreamInfo {
            sample_rate: HwSampleRate::new(TEST_SR),
            buffer_size: synth_core::BufferSize::new(256),
            channels: synth_core::ChannelCount::Stereo,
            output_latency: std::time::Duration::ZERO,
            input_latency: None,
        };
        engine.on_stream_start(&stream_info);
        drive(&mut engine, 32);

        let removed = prune_unused_samples(&session, &sample_library);
        assert_eq!(removed, vec!["orphan".to_string()]);

        let lib = sample_library.read().unwrap();
        assert_eq!(lib.len(), 1, "only the referenced sample remains");
        assert!(lib.get(kept_id).is_some(), "kept sample still in library");
        assert!(lib.get(orphan_id).is_none(), "orphan sample removed");
    }
}
