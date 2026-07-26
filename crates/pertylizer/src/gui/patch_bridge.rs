//! Patch Bridge - translates between Patch data and Engine commands.
//!
//! This module handles the conversion between:
//! - Patch file format (serialized state)
//! - PatchEditor UI state
//! - Engine commands for audio processing

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use eframe::egui::Pos2;

use crate::gui::keyboard::PianoKeyboard;
use crate::gui::patch_editor::PatchEditor;
use crate::patch::{
    Author, ConnectionState, ExposedPortState, GroupId, GroupTemplate, ModuleState, ParamValue,
    Patch, Position,
};
use crate::session::SynthSession;
use synth_core::ModuleParam;
use synth_core::ModuleType;
use synth_core::Param;
use synth_core::{Describable, ModuleDescriptor};
use synth_engine::commands::PortId;
use synth_engine::graph::Connection;
use synth_engine::instrument::InstrumentId;
use synth_engine::{EngineCommand, EngineHandle, ModuleId};

const VISUALIZATION_BUFFER_CAPACITY: usize = 4096;

/// Allocate the next module ID for a GUI-owned installation.
///
/// Most modules normally let [`SynthSession::add_module`] do this internally.
/// GUI-owned visualizers and signal monitors need their ID before construction,
/// so every editor creation path uses this helper and then installs by ID.
fn next_editor_module_id(
    session: &SynthSession,
    instrument_id: InstrumentId,
    module_type: ModuleType,
) -> ModuleId {
    let mut counters = session.counters_lock();
    let counter = counters.entry((instrument_id, module_type)).or_insert(0);
    *counter += 1;
    ModuleId::new(module_type, *counter)
}

/// Keep the session's per-type counter ahead of a module loaded with a fixed ID.
fn register_editor_module_id(
    session: &SynthSession,
    instrument_id: InstrumentId,
    module_id: ModuleId,
) {
    let mut counters = session.counters_lock();
    let counter = counters
        .entry((instrument_id, module_id.module_type))
        .or_insert(0);
    *counter = (*counter).max(module_id.instance);
}

fn visualizer_type(module_type: ModuleType) -> Option<synth_engine::commands::VisualizerType> {
    match module_type {
        ModuleType::Oscilloscope => Some(synth_engine::commands::VisualizerType::Oscilloscope),
        ModuleType::LevelMeter => Some(synth_engine::commands::VisualizerType::LevelMeter),
        ModuleType::SpectrumAnalyzer => {
            Some(synth_engine::commands::VisualizerType::SpectrumAnalyzer)
        }
        _ => None,
    }
}

/// Install one module in both the engine/session and a patch editor.
///
/// This is the single GUI-side installation path for palette creation, patch
/// loading, group templates, and clipboard paste. Signal monitors and
/// visualizers need a GUI-owned [`synth_engine::visualizers::VisualizationBuffer`];
/// ordinary voice modules and effects delegate to [`SynthSession`].
pub(crate) fn install_editor_module_with_id(
    session: &SynthSession,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
    patch_editor: &mut PatchEditor,
    module_id: ModuleId,
    position: Pos2,
) -> Result<ModuleDescriptor, String> {
    register_editor_module_id(session, instrument_id, module_id);
    let module_type = module_id.module_type;

    let descriptor = if module_type == ModuleType::SignalMonitor {
        let mut module = synth_modules::SignalMonitor::new();
        let descriptor = module.descriptor();
        let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(
            VISUALIZATION_BUFFER_CAPACITY,
        ));
        handle.add_visualization_buffer(module_id, buffer.clone());
        module.set_vis_sink(buffer);
        session.register_descriptor(instrument_id, module_id, descriptor.clone());
        handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(instrument_id),
            id: module_id,
            module: Box::new(module),
        });
        descriptor
    } else if let Some(visualizer_type) = visualizer_type(module_type) {
        let descriptor = crate::module_factory::get_descriptor(module_type)
            .ok_or_else(|| format!("Missing descriptor for {module_type:?}"))?;
        let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(
            VISUALIZATION_BUFFER_CAPACITY,
        ));
        handle.add_visualization_buffer(module_id, buffer.clone());
        session.register_descriptor(instrument_id, module_id, descriptor.clone());
        handle.send(EngineCommand::AddVisualizer {
            instrument_id: Some(instrument_id),
            id: module_id,
            visualizer_type,
            buffer,
        });
        descriptor
    } else {
        session
            .add_module_with_id(instrument_id, module_id, module_type)
            .map_err(|error| format!("Failed to create module {module_type:?}: {error}"))?
    };

    patch_editor.add_module_at(module_id, descriptor.clone(), position);
    Ok(descriptor)
}

/// Allocate and install one module in both the engine/session and patch editor.
pub(crate) fn create_editor_module(
    session: &SynthSession,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
    patch_editor: &mut PatchEditor,
    module_type: ModuleType,
    position: Pos2,
) -> Result<(ModuleId, ModuleDescriptor), String> {
    let module_id = next_editor_module_id(session, instrument_id, module_type);
    let descriptor = install_editor_module_with_id(
        session,
        handle,
        instrument_id,
        patch_editor,
        module_id,
        position,
    )?;
    Ok((module_id, descriptor))
}

/// Load a patch into a specific instrument's rack view and send commands to the engine.
///
/// This function handles all the logic for:
/// - Clearing existing state for the target instrument only
/// - Creating module instances
/// - Applying parameters
/// - Establishing connections
///
/// When `apply_global` is `true` (full project load), master volume and glide
/// time are applied. When `false` (per-instrument patch load), those global
/// settings are skipped to avoid overwriting state from other instruments.
///
/// Note: Effects are per-instrument (on EffectChain) - each instrument has its own effects.
#[allow(clippy::too_many_arguments)]
pub fn load_patch(
    patch: &Patch,
    patch_editor: &mut PatchEditor,
    session: &SynthSession,
    handle: &mut EngineHandle,
    keyboard: &mut PianoKeyboard,
    glide_time: &mut synth_core::Seconds,
    instrument_id: InstrumentId,
    apply_global: bool,
) {
    // Remove visualizers directly (session doesn't track them)
    for module_id in patch_editor.module_ids() {
        let category = patch_editor
            .module_descriptor(module_id)
            .map(|d| d.category);
        if matches!(category, Some(synth_core::ModuleCategory::Visualizer)) {
            handle.send_blocking(EngineCommand::RemoveVisualizer {
                instrument_id: Some(instrument_id),
                id: module_id,
            });
            handle.remove_visualization_buffer(module_id);
        }
    }

    // Clear all non-visualizer modules via session — only clear GUI if engine succeeds
    if session.clear_graph(instrument_id).is_err() {
        eprintln!("Patch load: failed to clear graph for instrument {instrument_id:?}");
        return;
    }
    patch_editor.clear();

    // Add modules from patch
    for module_state in &patch.modules {
        load_module(module_state, patch_editor, session, handle, instrument_id);
    }

    // Load group metadata (UI-only)
    patch_editor.load_groups_from_patch(&patch.groups);

    // Add connections to both patch_editor and engine
    for conn in &patch.connections {
        let from_result = conn.from.0.parse::<ModuleId>();
        let to_result = conn.to.0.parse::<ModuleId>();
        if from_result.is_err() || to_result.is_err() {
            eprintln!(
                "Patch load: skipping connection with invalid module ID(s): '{}' -> '{}'",
                conn.from.0, conn.to.0
            );
            continue;
        }
        if let (Ok(from_id), Ok(to_id)) = (from_result, to_result) {
            let connection = Connection::new(from_id, &*conn.from.1, to_id, &*conn.to.1);
            patch_editor.add_connection(connection);

            // Send connection to engine - blocking to ensure all connections are established
            // Use the target instrument_id for instrument-level connections
            handle.send_blocking(EngineCommand::Connect {
                instrument_id: Some(instrument_id),
                from: PortId::new(from_id, &*conn.from.1),
                to: PortId::new(to_id, &*conn.to.1),
            });
        }
    }

    let saved_order: Vec<ModuleId> = patch
        .settings
        .effect_chain_order
        .iter()
        .filter_map(|s| s.parse::<ModuleId>().ok())
        .collect();
    if !saved_order.is_empty() {
        handle.send_blocking(EngineCommand::SetEffectChainOrder {
            instrument_id: Some(instrument_id),
            order: saved_order.clone(),
        });
    }
    patch_editor.mark_effect_chain_aligned(saved_order);

    // Apply per-instrument settings (always)
    keyboard.set_octave_offset(patch.settings.octave_offset);

    // Apply global settings only during full project load, not per-instrument patch load
    if apply_global {
        *glide_time = patch.settings.glide_time;
        handle.send_blocking(EngineCommand::SetMasterVolume(patch.settings.master_volume));
        handle.send_blocking(EngineCommand::SetGlideTime(patch.settings.glide_time));
    }

    // Ensure the target instrument is enabled after loading
    handle.send_blocking(EngineCommand::SetInstrumentEnabled {
        instrument_id,
        enabled: true,
    });
}

/// Load a single module from patch state.
fn load_module(
    module_state: &ModuleState,
    patch_editor: &mut PatchEditor,
    session: &SynthSession,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
) {
    // Parse module ID from patch file (e.g., "osc-1" -> ModuleId)
    let module_id: ModuleId = match module_state.id.parse() {
        Ok(id) => id,
        Err(_) => {
            eprintln!(
                "patch_bridge: skipping module with invalid ID {:?} in instrument {instrument_id:?}",
                module_state.id
            );
            return;
        }
    };

    let position = Pos2::new(module_state.position.x, module_state.position.y);

    let descriptor = match install_editor_module_with_id(
        session,
        handle,
        instrument_id,
        patch_editor,
        module_id,
        position,
    ) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            eprintln!(
                "Warning: failed to load module {}: {error}",
                module_state.id
            );
            return;
        }
    };

    apply_module_parameters(
        module_id,
        &descriptor,
        &module_state.parameters,
        patch_editor,
        handle,
        instrument_id,
    );

    // Install per-slot control scripts (Step 2). The engine-side apply
    // (`session.apply_patch`) does this on the headless/project path, but the GUI
    // load path builds the graph module-by-module and previously skipped scripts,
    // so a saved patch's Mod Matrix / Script / AudioScript slot scripts were
    // silently dropped on load (their source/marker glyphs vanished). Applied
    // after params so the routing already exists; the GUI panel's `slot_scripts`
    // mirror then repopulates from the engine snapshot via `sync_module_scripts`.
    // Keys are 1-based; a bad key or compile error is logged and skipped so the
    // rest of the patch still loads.
    for (slot_key, source) in &module_state.scripts {
        let slot = match crate::session::parse_mod_slot_key(slot_key) {
            Ok(slot) => slot,
            Err(msg) => {
                eprintln!("patch_bridge: {module_id} script: {msg}");
                continue;
            }
        };
        if let Err(e) = session.set_mod_script(instrument_id, module_id, slot, source) {
            eprintln!("patch_bridge: {module_id} slot {slot_key} script: {e}");
        }
    }
}

/// Resolve a parameter name (descriptor `type_id`, falling back to
/// display `name`, both case-insensitive) and mirror its value into
/// the editor's cache. Returns the matched descriptor so callers that
/// also need to drive the engine can compute `value.to_param(...)`.
fn apply_param_to_editor<'a>(
    module_id: ModuleId,
    descriptor: &'a ModuleDescriptor,
    param_name: &str,
    value: &ParamValue,
    patch_editor: &mut PatchEditor,
) -> Option<&'a synth_core::ParameterDescriptor> {
    let lower = param_name.to_lowercase();
    let param_desc = descriptor
        .parameters
        .iter()
        .find(|p| p.type_id.to_lowercase() == lower)
        .or_else(|| {
            descriptor
                .parameters
                .iter()
                .find(|p| p.name.to_lowercase() == lower)
        })?;
    // GUI cache uses f32 (lossy for SampleId is fine — display only).
    // Engine command uses to_param so SampleId round-trips losslessly.
    let f32_value = value.to_f32(param_desc);
    patch_editor.set_parameter_by_name(module_id, &param_desc.name, f32_value);
    Some(param_desc)
}

/// Apply parameters to a module during patch loading.
///
/// Routes effect-chain modules through `SetEffectParameter` (so duplicate
/// effects of the same type can be targeted by `ModuleId`) and voice-graph
/// modules through `SetModuleParameter`.
pub fn apply_module_parameters(
    module_id: ModuleId,
    descriptor: &ModuleDescriptor,
    parameters: &BTreeMap<String, ParamValue>,
    patch_editor: &mut PatchEditor,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
) {
    let is_effect = module_id.module_type.is_effect();
    for (param_name, value) in parameters {
        let Some(param_desc) =
            apply_param_to_editor(module_id, descriptor, param_name, value, patch_editor)
        else {
            continue;
        };
        let param = value.to_param(param_desc);
        if is_effect {
            handle.send(EngineCommand::SetEffectParameter {
                instrument_id: Some(instrument_id),
                module_id,
                param,
            });
        } else {
            // SetModuleParameter (not SetVoiceParameter) so arbitrary
            // module instances like env-3 / amp-2 — outside the fixed
            // PolyModule enum — still route correctly.
            handle.send(EngineCommand::SetModuleParameter {
                instrument_id: Some(instrument_id),
                module_id,
                param,
            });
        }
    }
}

/// Populate `patch_editor` from a patch when the engine already has it
/// applied. Reads descriptors back from `session`, registers visualizer
/// modules (which the engine-side apply skips because they need a
/// GUI-owned `VisualizationBuffer`), and adds connections + effect-chain
/// order to the editor's view.
pub fn populate_editor_from_patch(
    patch: &Patch,
    patch_editor: &mut PatchEditor,
    session: &SynthSession,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
) {
    patch_editor.clear();

    for module_state in &patch.modules {
        populate_editor_module(module_state, patch_editor, session, handle, instrument_id);
    }

    patch_editor.load_groups_from_patch(&patch.groups);

    for conn in &patch.connections {
        let from_result = conn.from.0.parse::<ModuleId>();
        let to_result = conn.to.0.parse::<ModuleId>();
        if from_result.is_err() || to_result.is_err() {
            eprintln!(
                "populate_editor_from_patch: skipping connection with invalid module ID(s): '{}' -> '{}'",
                conn.from.0, conn.to.0
            );
            continue;
        }
        if let (Ok(from_id), Ok(to_id)) = (from_result, to_result) {
            let connection = Connection::new(from_id, &*conn.from.1, to_id, &*conn.to.1);
            patch_editor.add_connection(connection);
        }
    }

    let saved_order: Vec<ModuleId> = patch
        .settings
        .effect_chain_order
        .iter()
        .filter_map(|s| s.parse::<ModuleId>().ok())
        .collect();
    let baseline_order = if saved_order.is_empty() {
        patch
            .modules
            .iter()
            .filter_map(|m| m.id.parse::<ModuleId>().ok())
            .filter(|id| id.module_type.is_effect())
            .collect()
    } else {
        saved_order
    };
    patch_editor.mark_effect_chain_aligned(baseline_order);
}

/// Add a single module to `patch_editor` after `apply_project` has run.
///
/// Visualizers and SignalMonitor are still created here (with their
/// `VisualizationBuffer`s) because `apply_project` skips them. All other
/// modules read their descriptor from `session` and only update the
/// editor's local cache — no engine commands are sent.
fn populate_editor_module(
    module_state: &ModuleState,
    patch_editor: &mut PatchEditor,
    session: &SynthSession,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
) {
    let module_id: ModuleId = match module_state.id.parse() {
        Ok(id) => id,
        Err(_) => {
            eprintln!(
                "populate_editor_from_patch: skipping module with invalid ID {:?} in instrument {instrument_id:?}",
                module_state.id
            );
            return;
        }
    };

    let position = Pos2::new(module_state.position.x, module_state.position.y);

    if module_id.module_type == ModuleType::SignalMonitor || module_id.module_type.is_visualizer() {
        let Ok(descriptor) = install_editor_module_with_id(
            session,
            handle,
            instrument_id,
            patch_editor,
            module_id,
            position,
        ) else {
            return;
        };
        apply_module_parameters(
            module_id,
            &descriptor,
            &module_state.parameters,
            patch_editor,
            handle,
            instrument_id,
        );
        return;
    }

    // Non-visualizer modules: `apply_project` already registered the
    // descriptor and sent the engine commands; only the editor cache
    // needs filling.
    let descriptor = match session.module_descriptor(instrument_id, module_id) {
        Some(d) => d,
        None => {
            eprintln!(
                "populate_editor_from_patch: missing descriptor for {module_id} in instrument {instrument_id:?} \
                 (apply_project failed earlier?); skipping module"
            );
            return;
        }
    };

    patch_editor.add_module_at(module_id, descriptor.clone(), position);
    apply_module_parameters_ui(
        module_id,
        &descriptor,
        &module_state.parameters,
        patch_editor,
    );
}

/// UI-only counterpart of `apply_module_parameters` — updates the
/// editor's parameter cache only. Used after `apply_project` has
/// already written the engine values via `session.apply_patch`.
fn apply_module_parameters_ui(
    module_id: ModuleId,
    descriptor: &ModuleDescriptor,
    parameters: &BTreeMap<String, ParamValue>,
    patch_editor: &mut PatchEditor,
) {
    for (param_name, value) in parameters {
        apply_param_to_editor(module_id, descriptor, param_name, value, patch_editor);
    }
}

/// Create a patch from current rack state.
#[allow(clippy::too_many_arguments)]
pub fn create_patch_from_rack(
    patch_name: &str,
    author: &Author,
    patch_editor: &PatchEditor,
    keyboard: &PianoKeyboard,
    handle: &EngineHandle,
    glide_time: synth_core::Seconds,
    engine_state: Option<(&synth_engine::state::EngineState, InstrumentId)>,
) -> Option<Patch> {
    let mut patch = create_patch_from_editor(patch_name, patch_editor, engine_state);
    if !author.is_empty() {
        patch.author = Some(author.clone());
    }
    patch.settings.octave_offset = keyboard.octave_offset();
    patch.settings.master_volume = synth_core::Gain::new(handle.master_volume());
    patch.settings.glide_time = glide_time;
    // Save canvas size so layout is restored correctly on load
    let content_size = patch_editor.content_size();
    patch.settings.canvas_size = Some(crate::patch::CanvasSize::new(
        content_size.x,
        content_size.y,
    ));
    Some(patch)
}

/// Create a patch from a `PatchEditor` without global settings.
///
/// Used by project save to extract each instrument's module graph independently.
/// Global settings (master volume, keyboard, glide) are stored at the
/// project level, not per-instrument.
///
/// If `engine_state` is provided, actual parameter values are read from the
/// running engine instead of the GUI's cached values (which may be stale for
/// modules created via MCP).
pub fn create_patch_from_editor(
    name: &str,
    patch_editor: &PatchEditor,
    engine_state: Option<(&synth_engine::state::EngineState, InstrumentId)>,
) -> Patch {
    let mut patch = Patch::new(name);

    // Fields that live only in the engine snapshot, not the GUI module cache:
    // parameter values (authoritative over stale GUI cache, e.g. after MCP edits),
    // per-instance descriptions, and per-slot control scripts. Read them in ONE
    // `get_modules_for_instrument` pass (one lock, one snapshot clone) so all three
    // come from a single coherent view — and so "Save Patch…" keeps descriptions/
    // scripts set via MCP rather than dropping them.
    let mut engine_params: HashMap<String, Vec<Param>> = HashMap::new();
    let mut engine_descriptions: HashMap<String, String> = HashMap::new();
    let mut engine_scripts: HashMap<String, BTreeMap<String, String>> = HashMap::new();
    if let Some((state, inst_id)) = engine_state {
        for m in state.shared_graph.get_modules_for_instrument(inst_id) {
            let key = m.id.to_string();
            if !m.scripts.is_empty() {
                engine_scripts.insert(key.clone(), m.scripts);
            }
            if !m.description.is_empty() {
                engine_descriptions.insert(key.clone(), m.description);
            }
            engine_params.insert(key, m.parameters);
        }
    }

    // Pull the per-instrument fields that live only in the engine snapshot
    // (not the GUI module cache) in a single locked lookup: the effect-chain
    // order and the patch-level color. Pulling the color here means a standalone
    // "Save Patch…" keeps a color set via MCP; doing it in one read avoids a
    // second lock acquisition that could observe a different snapshot.
    let (effect_chain_order, patch_color): (Vec<String>, Option<String>) = engine_state
        .and_then(|(state, inst_id)| {
            state
                .instrument_snapshots
                .read()
                .iter()
                .find(|s| s.id == inst_id)
                .map(|s| {
                    (
                        s.effect_chain_order
                            .iter()
                            .map(ModuleId::to_string)
                            .collect(),
                        s.patch_color.clone(),
                    )
                })
        })
        .unwrap_or_default();

    // Emit in a deterministic order so re-saving an unchanged patch produces
    // byte-identical JSON. `ModuleId: Ord` sorts by (module_type, instance).
    let mut sorted_ids = patch_editor.module_ids();
    sorted_ids.sort();

    for module_id in sorted_ids {
        if let Some((descriptor, position, gui_params)) = patch_editor.get_module_data(module_id) {
            let mut param_map = BTreeMap::new();

            if let Some(engine_param_list) = engine_params.get(&module_id.to_string()) {
                // Use engine values: match each descriptor to its engine
                // param by kind, ensuring we always use the stable type_id.
                for desc in &descriptor.parameters {
                    if let Some(ep) = engine_param_list.iter().find(|p| p.same_kind(&desc.id)) {
                        param_map.insert(desc.type_id.clone(), ParamValue::from_param(ep, desc));
                    }
                }
            } else {
                // Fall back to GUI-cached values (keyed by display name).
                // Map display name → type_id using the descriptor.
                let name_to_type_id: HashMap<&str, &str> = descriptor
                    .parameters
                    .iter()
                    .map(|p| (p.name.as_str(), p.type_id.as_str()))
                    .collect();
                for (display_name, value) in &gui_params {
                    let key = name_to_type_id
                        .get(display_name.as_str())
                        .map_or_else(|| display_name.clone(), |tid| (*tid).to_string());
                    param_map.insert(key, ParamValue::Float(*value));
                }
            }
            patch.modules.push(ModuleState {
                id: module_id.to_string(),
                module_type: module_id.module_type,
                position: Position::new(position.x, position.y),
                description: engine_descriptions
                    .get(&module_id.to_string())
                    .cloned()
                    .unwrap_or_default(),
                parameters: param_map,
                scripts: engine_scripts
                    .get(&module_id.to_string())
                    .cloned()
                    .unwrap_or_default(),
            });
        }
    }

    // Port names are interned `PortName(u32)` whose numeric ordering depends
    // on intern-time and is not stable across runs, so the secondary sort key
    // uses the rendered string form.
    let mut sorted_conns: Vec<Connection> = patch_editor.connections().to_vec();
    sorted_conns.sort_by_cached_key(|c| {
        let from_port: String = c.from_port.into();
        let to_port: String = c.to_port.into();
        (c.from_module, from_port, c.to_module, to_port)
    });
    patch.connections = sorted_conns.iter().map(ConnectionState::from).collect();

    patch.groups = patch_editor.group_states();

    // Save canvas size so layout is restored correctly on load
    let content_size = patch_editor.content_size();
    patch.settings.canvas_size = Some(crate::patch::CanvasSize::new(
        content_size.x,
        content_size.y,
    ));

    patch.settings.effect_chain_order = effect_chain_order;
    patch.color = patch_color;

    patch
}

/// Insert a group template into an existing instrument, remapping all IDs.
///
/// `drop_pos` is the top-left origin (world coords) where the template's
/// internal layout should be placed.
pub fn insert_group_template(
    template: &GroupTemplate,
    drop_pos: Pos2,
    patch_editor: &mut PatchEditor,
    session: &SynthSession,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
) -> Result<GroupId, String> {
    use crate::gui::patch_editor::ExposedPort;
    use synth_core::PortName;

    if template.modules.is_empty() {
        return Err("Template has no modules".to_string());
    }

    let mut id_map: HashMap<String, ModuleId> = HashMap::new();
    let mut new_members: Vec<ModuleId> = Vec::new();

    for module_state in &template.modules {
        let module_type = module_state.module_type;
        let position = Pos2::new(
            module_state.position.x + drop_pos.x,
            module_state.position.y + drop_pos.y,
        );

        let (module_id, descriptor) = create_editor_module(
            session,
            handle,
            instrument_id,
            patch_editor,
            module_type,
            position,
        )?;

        apply_module_parameters(
            module_id,
            &descriptor,
            &module_state.parameters,
            patch_editor,
            handle,
            instrument_id,
        );

        id_map.insert(module_state.id.clone(), module_id);
        new_members.push(module_id);
    }

    // Add connections with remapped IDs
    for conn in &template.connections {
        let Some(from_id) = id_map.get(&conn.from.0) else {
            continue;
        };
        let Some(to_id) = id_map.get(&conn.to.0) else {
            continue;
        };
        let connection = Connection::new(*from_id, &*conn.from.1, *to_id, &*conn.to.1);
        patch_editor.add_connection(connection);
        if let Err(e) = session.connect(
            instrument_id,
            connection.from_module,
            connection.from_port,
            connection.to_module,
            connection.to_port,
        ) {
            eprintln!(
                "patch_bridge: failed to connect {} -> {}: {e}",
                connection.from_module, connection.to_module
            );
        }
    }

    // Map exposed ports to new IDs
    let map_port = |p: &ExposedPortState| -> Option<ExposedPort> {
        let new_id = *id_map.get(&p.module_id)?;
        let port_name: PortName = p.port.clone().into();
        Some(ExposedPort {
            label: p.label.clone(),
            module_id: new_id,
            port_name,
        })
    };

    let exposed_inputs: Vec<ExposedPort> = template
        .exposed_inputs
        .iter()
        .filter_map(map_port)
        .collect();
    let exposed_outputs: Vec<ExposedPort> = template
        .exposed_outputs
        .iter()
        .filter_map(map_port)
        .collect();

    let group_id = patch_editor.insert_group(
        template.name.clone(),
        template.color.clone(),
        new_members,
        exposed_inputs,
        exposed_outputs,
        false,
        drop_pos,
    );

    Ok(group_id)
}

/// Paste modules from clipboard data into an instrument.
///
/// Creates new module instances with fresh IDs, applies stored parameters,
/// remaps internal connections, and selects the newly pasted modules.
///
/// `paste_pos` is the target center position; modules are offset relative
/// to the clipboard's reference position.
///
/// Returns the set of newly created module IDs.
#[allow(clippy::too_many_arguments)]
pub fn paste_clipboard_modules(
    clipboard_modules: &[crate::patch::ModuleState],
    clipboard_connections: &[crate::patch::ConnectionState],
    reference_pos: (f32, f32),
    paste_pos: (f32, f32),
    patch_editor: &mut PatchEditor,
    session: &crate::session::SynthSession,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
) -> std::collections::HashSet<ModuleId> {
    use std::collections::{HashMap, HashSet};

    let mut id_map: HashMap<String, ModuleId> = HashMap::new();
    let mut new_ids: HashSet<ModuleId> = HashSet::new();

    let offset_x = paste_pos.0 - reference_pos.0;
    let offset_y = paste_pos.1 - reference_pos.1;

    for module_state in clipboard_modules {
        let module_type = module_state.module_type;

        let position = Pos2::new(
            module_state.position.x + offset_x,
            module_state.position.y + offset_y,
        );

        let Ok((module_id, descriptor)) = create_editor_module(
            session,
            handle,
            instrument_id,
            patch_editor,
            module_type,
            position,
        ) else {
            continue;
        };

        apply_module_parameters(
            module_id,
            &descriptor,
            &module_state.parameters,
            patch_editor,
            handle,
            instrument_id,
        );

        id_map.insert(module_state.id.clone(), module_id);
        new_ids.insert(module_id);
    }

    // Remap and create connections
    for conn in clipboard_connections {
        let Some(from_id) = id_map.get(&conn.from.0) else {
            continue;
        };
        let Some(to_id) = id_map.get(&conn.to.0) else {
            continue;
        };
        let connection = Connection::new(*from_id, &*conn.from.1, *to_id, &*conn.to.1);
        patch_editor.add_connection(connection);
        if let Err(e) = session.connect(
            instrument_id,
            connection.from_module,
            connection.from_port,
            connection.to_module,
            connection.to_port,
        ) {
            eprintln!(
                "patch_bridge: failed to connect {} -> {}: {e}",
                connection.from_module, connection.to_module
            );
        }
    }

    new_ids
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use synth_core::ModuleCategory;

    fn descriptor(category: ModuleCategory) -> ModuleDescriptor {
        ModuleDescriptor::new("test", "test").category(category)
    }

    fn add(editor: &mut PatchEditor, ty: ModuleType, n: u16, pos: (f32, f32)) -> ModuleId {
        add_cat(editor, ty, n, pos, ModuleCategory::Oscillator)
    }

    fn add_cat(
        editor: &mut PatchEditor,
        ty: ModuleType,
        n: u16,
        pos: (f32, f32),
        category: ModuleCategory,
    ) -> ModuleId {
        let id = ModuleId::new(ty, n);
        editor.add_module_at(id, descriptor(category), Pos2::new(pos.0, pos.1));
        id
    }

    #[test]
    fn create_patch_module_order_is_stable_and_sorted() {
        let mut editor = PatchEditor::new();
        // osc-10 vs osc-2 catches accidental string ordering.
        add(&mut editor, ModuleType::Oscillator, 2, (0.0, 0.0));
        add(&mut editor, ModuleType::Filter, 1, (0.0, 0.0));
        add(&mut editor, ModuleType::Oscillator, 10, (0.0, 0.0));
        add(&mut editor, ModuleType::Oscillator, 1, (0.0, 0.0));
        add(&mut editor, ModuleType::Amplifier, 3, (0.0, 0.0));

        let patch = create_patch_from_editor("Test", &editor, None);

        let actual: Vec<&str> = patch.modules.iter().map(|m| m.id.as_str()).collect();
        let expected: Vec<&str> = vec!["osc-1", "osc-2", "osc-10", "flt-1", "amp-3"];
        assert_eq!(
            actual, expected,
            "modules must be sorted by (module_type, instance) numerically"
        );
    }

    #[test]
    fn create_patch_repeated_save_is_byte_identical() {
        let mut editor = PatchEditor::new();
        let osc1 = add(&mut editor, ModuleType::Oscillator, 1, (10.0, 20.0));
        let osc2 = add(&mut editor, ModuleType::Oscillator, 2, (30.0, 40.0));
        let flt1 = add(&mut editor, ModuleType::Filter, 1, (50.0, 60.0));
        let amp1 = add(&mut editor, ModuleType::Amplifier, 1, (70.0, 80.0));

        // Add connections in an order that doesn't match the desired sort.
        editor.add_connection(Connection::new(osc2, "out", flt1, "in"));
        editor.add_connection(Connection::new(flt1, "out", amp1, "in"));
        editor.add_connection(Connection::new(osc1, "out", flt1, "in"));

        let first = create_patch_from_editor("Test", &editor, None);
        let second = create_patch_from_editor("Test", &editor, None);

        let first_json = serde_json::to_string_pretty(&first).unwrap();
        let second_json = serde_json::to_string_pretty(&second).unwrap();
        assert_eq!(
            first_json, second_json,
            "re-saving an unchanged patch must produce byte-identical JSON"
        );
    }

    #[test]
    fn create_patch_connections_sorted_by_endpoint() {
        let mut editor = PatchEditor::new();
        let osc1 = add(&mut editor, ModuleType::Oscillator, 1, (0.0, 0.0));
        let osc2 = add(&mut editor, ModuleType::Oscillator, 2, (0.0, 0.0));
        let flt1 = add(&mut editor, ModuleType::Filter, 1, (0.0, 0.0));

        // Insertion order is intentionally scrambled.
        editor.add_connection(Connection::new(osc2, "out", flt1, "in"));
        editor.add_connection(Connection::new(osc1, "out", flt1, "in"));

        let patch = create_patch_from_editor("Test", &editor, None);

        let actual: Vec<(String, String)> = patch
            .connections
            .iter()
            .map(|c| (c.from.0.clone(), c.to.0.clone()))
            .collect();
        let expected = vec![
            ("osc-1".to_string(), "flt-1".to_string()),
            ("osc-2".to_string(), "flt-1".to_string()),
        ];
        assert_eq!(
            actual, expected,
            "connections must be sorted by (from sort_key, ..., to sort_key, ...)"
        );
    }

    #[test]
    fn baselined_chain_order_skips_first_render_realignment() {
        let mut editor = PatchEditor::new();
        let delay = add_cat(
            &mut editor,
            ModuleType::Delay,
            1,
            (123.0, 456.0),
            ModuleCategory::Effect,
        );
        let reverb = add_cat(
            &mut editor,
            ModuleType::Reverb,
            1,
            (789.0, 1011.0),
            ModuleCategory::Effect,
        );

        let loaded_order = vec![delay, reverb];
        editor.mark_effect_chain_aligned(loaded_order.clone());

        editor.realign_effect_chain_if_changed(&loaded_order);

        let (_, delay_pos, _) = editor.get_module_data(delay).unwrap();
        let (_, reverb_pos, _) = editor.get_module_data(reverb).unwrap();
        assert_eq!(delay_pos, Pos2::new(123.0, 456.0));
        assert_eq!(reverb_pos, Pos2::new(789.0, 1011.0));
    }

    #[test]
    fn chain_order_change_still_realigns() {
        let mut editor = PatchEditor::new();
        let delay = add_cat(
            &mut editor,
            ModuleType::Delay,
            1,
            (100.0, 200.0),
            ModuleCategory::Effect,
        );
        let reverb = add_cat(
            &mut editor,
            ModuleType::Reverb,
            1,
            (100.0, 400.0),
            ModuleCategory::Effect,
        );

        editor.mark_effect_chain_aligned(vec![delay, reverb]);
        editor.realign_effect_chain_if_changed(&[reverb, delay]);

        let (_, delay_pos, _) = editor.get_module_data(delay).unwrap();
        let (_, reverb_pos, _) = editor.get_module_data(reverb).unwrap();
        assert!(
            reverb_pos.y < delay_pos.y,
            "after reorder, reverb should sit above delay (reverb={reverb_pos:?}, delay={delay_pos:?})"
        );
    }
}
