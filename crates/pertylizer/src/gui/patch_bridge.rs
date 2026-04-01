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
use crate::gui::patch_editor::{EffectType, PatchEditor};
use crate::patch::{
    Author, ConnectionState, ExposedPortState, GroupId, GroupTemplate, ModuleState, ParamValue,
    Patch, Position,
};
use crate::session::SynthSession;
use synth_core::ModuleType;
use synth_core::Param;
use synth_core::{Describable, ModuleDescriptor};
use synth_engine::commands::PortId;
use synth_engine::graph::Connection;
use synth_engine::instrument::InstrumentId;
use synth_engine::visualizers::{LevelMeter, Oscilloscope, SpectrumAnalyzer};
use synth_engine::{EngineCommand, EngineHandle, ModuleId};

/// Load a patch into a specific instrument's rack view and send commands to the engine.
///
/// This function handles all the logic for:
/// - Clearing existing state for the target instrument only
/// - Creating module instances
/// - Applying parameters
/// - Establishing connections
///
/// When `apply_global` is `true` (full project load), master volume, glide time,
/// and AWE settings are applied. When `false` (per-instrument patch load), those
/// global settings are skipped to avoid overwriting state from other instruments.
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

    // Apply per-instrument settings (always)
    keyboard.set_octave_offset(patch.settings.octave_offset);

    // Apply global settings only during full project load, not per-instrument patch load
    if apply_global {
        *glide_time = patch.settings.glide_time;
        handle.send_blocking(EngineCommand::SetMasterVolume(patch.settings.master_volume));
        handle.send_blocking(EngineCommand::SetGlideTime(patch.settings.glide_time));

        // Load full AWE state if present
        if let Some(awe) = &patch.settings.awe {
            handle.send_blocking(EngineCommand::SetAweEnabled {
                enabled: awe.enabled,
            });
            handle.send_blocking(EngineCommand::SetAweParameter {
                param: synth_awe::AweParam::RoomShape(awe.room),
            });
            handle.send_blocking(EngineCommand::SetAweParameter {
                param: synth_awe::AweParam::Material(awe.material),
            });
            handle.send_blocking(EngineCommand::SetAweState {
                snapshot: awe.to_snapshot(),
            });
            handle.send_blocking(EngineCommand::SetAweParameter {
                param: synth_awe::AweParam::SpatialEnabled(awe.spatial_enabled),
            });
            handle.send_blocking(EngineCommand::SetAweParameter {
                param: synth_awe::AweParam::NoteMapping(awe.note_mapping),
            });
        }
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

    // Special cases: modules that need VisualizationBuffer injection (GUI-only)
    match module_state.module_type {
        ModuleType::SignalMonitor => {
            load_signal_monitor(
                module_id,
                module_state,
                patch_editor,
                session,
                handle,
                instrument_id,
                position,
            );
            return;
        }
        ModuleType::Oscilloscope => {
            load_visualizer(
                module_id,
                patch_editor,
                session,
                handle,
                instrument_id,
                position,
                synth_engine::commands::VisualizerType::Oscilloscope,
                Oscilloscope::new().descriptor(),
            );
            return;
        }
        ModuleType::LevelMeter => {
            load_visualizer(
                module_id,
                patch_editor,
                session,
                handle,
                instrument_id,
                position,
                synth_engine::commands::VisualizerType::LevelMeter,
                LevelMeter::new().descriptor(),
            );
            return;
        }
        ModuleType::SpectrumAnalyzer => {
            load_visualizer(
                module_id,
                patch_editor,
                session,
                handle,
                instrument_id,
                position,
                synth_engine::commands::VisualizerType::SpectrumAnalyzer,
                SpectrumAnalyzer::new().descriptor(),
            );
            return;
        }
        _ => {}
    }

    // All other modules: delegate to session
    let descriptor =
        match session.add_module_with_id(instrument_id, module_id, module_id.module_type) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Warning: failed to load module {}: {e}", module_state.id);
                return;
            }
        };

    patch_editor.add_module_at(module_id, descriptor.clone(), position);

    // Determine effect type for parameter routing
    let effect_type = EffectType::from_module_type(module_id.module_type);

    apply_module_parameters(
        module_id,
        &descriptor,
        &module_state.parameters,
        effect_type,
        patch_editor,
        handle,
        instrument_id,
    );
}

/// Load a SignalMonitor module (needs VisualizationBuffer injection).
fn load_signal_monitor(
    module_id: ModuleId,
    module_state: &ModuleState,
    patch_editor: &mut PatchEditor,
    session: &SynthSession,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
    position: Pos2,
) {
    let mut m = synth_modules::SignalMonitor::new();
    let descriptor = Describable::descriptor(&m);

    // Update session counter so future IDs don't collide
    {
        let mut counters = session.counters_lock();
        let counter = counters
            .entry((instrument_id, module_id.module_type))
            .or_insert(0);
        if module_id.instance > *counter {
            *counter = module_id.instance;
        }
    }

    session.register_descriptor(instrument_id, module_id, descriptor.clone());
    patch_editor.add_module_at(module_id, descriptor.clone(), position);

    let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
    handle.add_visualization_buffer(module_id, buffer.clone());
    m.set_vis_sink(buffer);

    handle.send(EngineCommand::AddModuleInstance {
        instrument_id: Some(instrument_id),
        id: module_id,
        module: Box::new(m),
    });

    apply_module_parameters(
        module_id,
        &descriptor,
        &module_state.parameters,
        None,
        patch_editor,
        handle,
        instrument_id,
    );
}

/// Load a visualizer module (Oscilloscope, LevelMeter, SpectrumAnalyzer).
#[allow(clippy::too_many_arguments)]
fn load_visualizer(
    module_id: ModuleId,
    patch_editor: &mut PatchEditor,
    session: &SynthSession,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
    position: Pos2,
    visualizer_type: synth_engine::commands::VisualizerType,
    descriptor: ModuleDescriptor,
) {
    // Update session counter
    {
        let mut counters = session.counters_lock();
        let counter = counters
            .entry((instrument_id, module_id.module_type))
            .or_insert(0);
        if module_id.instance > *counter {
            *counter = module_id.instance;
        }
    }

    session.register_descriptor(instrument_id, module_id, descriptor.clone());
    patch_editor.add_module_at(module_id, descriptor, position);

    let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
    handle.add_visualization_buffer(module_id, buffer.clone());

    handle.send(EngineCommand::AddVisualizer {
        instrument_id: Some(instrument_id),
        id: module_id,
        visualizer_type,
        buffer,
    });
}

/// Apply parameters to a module during patch loading.
pub fn apply_module_parameters(
    module_id: ModuleId,
    descriptor: &ModuleDescriptor,
    parameters: &BTreeMap<String, ParamValue>,
    effect_type: Option<EffectType>,
    patch_editor: &mut PatchEditor,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
) {
    for (param_name, value) in parameters {
        // Find the parameter descriptor by type_id (stable JSON key),
        // falling back to name for legacy files.
        let param_desc = descriptor
            .parameters
            .iter()
            .find(|p| p.type_id.to_lowercase() == param_name.to_lowercase())
            .or_else(|| {
                descriptor
                    .parameters
                    .iter()
                    .find(|p| p.name.to_lowercase() == param_name.to_lowercase())
            });

        if let Some(param_desc) = param_desc {
            // Convert the ParamValue to f32 for patch_editor
            let f32_value = match value {
                ParamValue::Float(f) => *f,
                ParamValue::Int(i) => *i as f32,
                ParamValue::Bool(b) => {
                    if *b {
                        1.0
                    } else {
                        0.0
                    }
                }
                ParamValue::Choice(s) => {
                    // Look up choice index
                    if let Some(ref choices) = param_desc.choices {
                        choices
                            .iter()
                            .position(|c| c.id == *s)
                            .map(|i| i as f32)
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    }
                }
            };

            // Update patch_editor — use descriptor name (not patch file name) to match
            // the key case used when param_values was initialized from the descriptor.
            patch_editor.set_parameter_by_name(module_id, &param_desc.name, f32_value);

            // Create the Param with value and send to engine
            let param = param_desc.id.with_f32(f32_value);

            if let Some(et) = effect_type {
                // Effects are per-instrument - send to this instrument's effect chain
                handle.send(EngineCommand::SetEffectParameter {
                    instrument_id: Some(instrument_id),
                    effect_type: et,
                    param,
                });
            } else {
                // Voice modules - use SetModuleParameter with direct module_id
                // This correctly handles arbitrary module instances (env-3, amp-2, etc)
                // that don't fit the fixed PolyModule enum
                handle.send(EngineCommand::SetModuleParameter {
                    instrument_id: Some(instrument_id),
                    module_id,
                    param,
                });
            }
        }
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
    awe_enabled: bool,
    awe_ui: &crate::gui::awe_view::AweUiState,
    engine_state: Option<(&synth_engine::state::EngineState, InstrumentId)>,
) -> Option<Patch> {
    let mut patch = create_patch_from_editor(patch_name, patch_editor, engine_state);
    if !author.is_empty() {
        patch.author = Some(author.clone());
    }
    patch.settings.octave_offset = keyboard.octave_offset();
    patch.settings.master_volume = synth_core::Gain::new(handle.master_volume());
    patch.settings.glide_time = glide_time;
    if awe_enabled {
        patch.settings.awe = Some(awe_ui.to_awe_state(true));
    }
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
/// Global settings (master volume, keyboard, glide, AWE) are stored at the
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

    // Build a lookup of engine parameter values when available.
    // Store raw Param values so we can match them to descriptors by kind.
    let engine_params: HashMap<String, Vec<Param>> = engine_state
        .map(|(state, inst_id)| {
            state
                .shared_graph
                .get_modules_for_instrument(inst_id)
                .into_iter()
                .map(|m| (m.id.to_string(), m.parameters.clone()))
                .collect()
        })
        .unwrap_or_default();

    for module_id in patch_editor.module_ids() {
        if let Some((descriptor, position, gui_params)) = patch_editor.get_module_data(module_id) {
            let mut param_map = BTreeMap::new();

            if let Some(engine_param_list) = engine_params.get(&module_id.to_string()) {
                // Use engine values: match each descriptor to its engine
                // param by kind, ensuring we always use the stable type_id.
                for desc in &descriptor.parameters {
                    if let Some(ep) = engine_param_list.iter().find(|p| p.same_kind(&desc.id)) {
                        param_map.insert(desc.type_id.clone(), ParamValue::Float(ep.as_f32()));
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
                parameters: param_map,
            });
        }
    }

    for conn in patch_editor.connections() {
        patch.connections.push(ConnectionState {
            from: (conn.from_module.to_string(), conn.from_port.into()),
            to: (conn.to_module.to_string(), conn.to_port.into()),
        });
    }

    patch.groups = patch_editor.group_states();

    // Save canvas size so layout is restored correctly on load
    let content_size = patch_editor.content_size();
    patch.settings.canvas_size = Some(crate::patch::CanvasSize::new(
        content_size.x,
        content_size.y,
    ));

    patch
}

/// Get the EffectType for a module from its ModuleId.
pub fn get_effect_type_from_module(
    _patch_editor: &PatchEditor,
    module_id: ModuleId,
) -> Option<EffectType> {
    EffectType::from_module_type(module_id.module_type)
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

    // Helper to allocate a new module ID for this instrument/type.
    let next_id = |module_type: ModuleType| {
        let mut counters = session.counters_lock();
        let counter = counters.entry((instrument_id, module_type)).or_insert(0);
        *counter += 1;
        ModuleId::new(module_type, *counter)
    };

    for module_state in &template.modules {
        let module_type = module_state.module_type;
        let module_id = next_id(module_type);
        let position = Pos2::new(
            module_state.position.x + drop_pos.x,
            module_state.position.y + drop_pos.y,
        );

        let descriptor = match module_type {
            ModuleType::SignalMonitor => {
                let mut m = synth_modules::SignalMonitor::new();
                let d = m.descriptor();
                session.register_descriptor(instrument_id, module_id, d.clone());
                patch_editor.add_module_at(module_id, d.clone(), position);

                let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(module_id, buffer.clone());
                m.set_vis_sink(buffer);
                handle.send(EngineCommand::AddModuleInstance {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                    module: Box::new(m),
                });
                d
            }
            ModuleType::Oscilloscope => {
                let descriptor = Oscilloscope::new().descriptor();
                patch_editor.add_module_at(module_id, descriptor.clone(), position);
                let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(module_id, buffer.clone());
                handle.send(EngineCommand::AddVisualizer {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                    visualizer_type: synth_engine::commands::VisualizerType::Oscilloscope,
                    buffer,
                });
                descriptor
            }
            ModuleType::LevelMeter => {
                let descriptor = LevelMeter::new().descriptor();
                patch_editor.add_module_at(module_id, descriptor.clone(), position);
                let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(module_id, buffer.clone());
                handle.send(EngineCommand::AddVisualizer {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                    visualizer_type: synth_engine::commands::VisualizerType::LevelMeter,
                    buffer,
                });
                descriptor
            }
            ModuleType::SpectrumAnalyzer => {
                let descriptor = SpectrumAnalyzer::new().descriptor();
                patch_editor.add_module_at(module_id, descriptor.clone(), position);
                let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(module_id, buffer.clone());
                handle.send(EngineCommand::AddVisualizer {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                    visualizer_type: synth_engine::commands::VisualizerType::SpectrumAnalyzer,
                    buffer,
                });
                descriptor
            }
            _ => {
                let descriptor = session
                    .add_module_with_id(instrument_id, module_id, module_type)
                    .map_err(|e| format!("Failed to create module {module_type:?}: {e}"))?;
                patch_editor.add_module_at(module_id, descriptor.clone(), position);
                descriptor
            }
        };

        let effect_type = EffectType::from_module_type(module_type);
        apply_module_parameters(
            module_id,
            &descriptor,
            &module_state.parameters,
            effect_type,
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

        // Allocate a fresh ID via session counters
        let module_id = {
            let mut counters = session.counters_lock();
            let counter = counters.entry((instrument_id, module_type)).or_insert(0);
            *counter += 1;
            ModuleId::new(module_type, *counter)
        };

        let position = Pos2::new(
            module_state.position.x + offset_x,
            module_state.position.y + offset_y,
        );

        let descriptor = match module_type {
            ModuleType::SignalMonitor => {
                let mut m = synth_modules::SignalMonitor::new();
                let d = synth_core::Describable::descriptor(&m);
                session.register_descriptor(instrument_id, module_id, d.clone());
                patch_editor.add_module_at(module_id, d.clone(), position);

                let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(module_id, buffer.clone());
                m.set_vis_sink(buffer);
                handle.send(EngineCommand::AddModuleInstance {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                    module: Box::new(m),
                });
                d
            }
            ModuleType::Oscilloscope => {
                let descriptor = Oscilloscope::new().descriptor();
                patch_editor.add_module_at(module_id, descriptor.clone(), position);
                let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(module_id, buffer.clone());
                handle.send(EngineCommand::AddVisualizer {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                    visualizer_type: synth_engine::commands::VisualizerType::Oscilloscope,
                    buffer,
                });
                descriptor
            }
            ModuleType::LevelMeter => {
                let descriptor = LevelMeter::new().descriptor();
                patch_editor.add_module_at(module_id, descriptor.clone(), position);
                let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(module_id, buffer.clone());
                handle.send(EngineCommand::AddVisualizer {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                    visualizer_type: synth_engine::commands::VisualizerType::LevelMeter,
                    buffer,
                });
                descriptor
            }
            ModuleType::SpectrumAnalyzer => {
                let descriptor = SpectrumAnalyzer::new().descriptor();
                patch_editor.add_module_at(module_id, descriptor.clone(), position);
                let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(module_id, buffer.clone());
                handle.send(EngineCommand::AddVisualizer {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                    visualizer_type: synth_engine::commands::VisualizerType::SpectrumAnalyzer,
                    buffer,
                });
                descriptor
            }
            _ => {
                let Ok(d) = session.add_module_with_id(instrument_id, module_id, module_type)
                else {
                    continue;
                };
                patch_editor.add_module_at(module_id, d.clone(), position);
                d
            }
        };

        let effect_type = EffectType::from_module_type(module_type);
        apply_module_parameters(
            module_id,
            &descriptor,
            &module_state.parameters,
            effect_type,
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
