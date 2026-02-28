//! Patch Bridge - translates between Patch data and Engine commands.
//!
//! This module handles the conversion between:
//! - Patch file format (serialized state)
//! - PatchEditor UI state
//! - Engine commands for audio processing

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::Pos2;

use crate::gui::keyboard::PianoKeyboard;
use crate::gui::patch_editor::{EffectType, PatchEditor};
use crate::patch::{
    ConnectionState, ExposedPortState, GroupId, GroupTemplate, ModuleState, ParamValue, Patch,
};
use crate::session::SynthSession;
use synth_core::ModuleType;
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
/// Note: Effects are per-instrument (on EffectChain) - each instrument has its own effects.
pub fn load_patch(
    patch: &Patch,
    patch_editor: &mut PatchEditor,
    session: &SynthSession,
    handle: &mut EngineHandle,
    keyboard: &mut PianoKeyboard,
    glide_time: &mut f32,
    instrument_id: InstrumentId,
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

    // Clear all non-visualizer modules via session
    let _ = session.clear_graph(instrument_id);

    // Clear GUI state
    patch_editor.clear();

    // Add modules from patch
    for module_state in &patch.modules {
        load_module(module_state, patch_editor, session, handle, instrument_id);
    }

    // Load group metadata (UI-only)
    patch_editor.load_groups_from_patch(&patch.groups);

    // Add connections to both patch_editor and engine
    for conn in &patch.connections {
        if let (Ok(from_id), Ok(to_id)) = (
            conn.from.0.parse::<ModuleId>(),
            conn.to.0.parse::<ModuleId>(),
        ) {
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

    // Apply settings
    keyboard.set_octave_offset(patch.settings.octave_offset);
    *glide_time = patch.settings.glide_time;
    handle.send_blocking(EngineCommand::SetMasterVolume(synth_core::Gain::new(
        patch.settings.master_volume,
    )));
    handle.send_blocking(EngineCommand::SetGlideTime(synth_core::Seconds::new(
        patch.settings.glide_time,
    )));

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
        Err(_) => return, // Skip invalid IDs
    };

    let position = Pos2::new(module_state.position.0, module_state.position.1);

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
    parameters: &HashMap<String, ParamValue>,
    effect_type: Option<EffectType>,
    patch_editor: &mut PatchEditor,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
) {
    for (param_name, value) in parameters {
        // Find the parameter descriptor by name
        let param_desc = descriptor
            .parameters
            .iter()
            .find(|p| p.name.to_lowercase() == param_name.to_lowercase());

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
pub fn create_patch_from_rack(
    patch_name: &str,
    patch_editor: &PatchEditor,
    keyboard: &PianoKeyboard,
    handle: &EngineHandle,
    glide_time: f32,
    awe_enabled: bool,
    awe_ui: &crate::gui::awe_view::AweUiState,
) -> Option<Patch> {
    let mut patch = create_patch_from_editor(patch_name, patch_editor);
    patch.author = Some("User".to_string());
    patch.settings.octave_offset = keyboard.octave_offset();
    patch.settings.master_volume = handle.master_volume();
    patch.settings.glide_time = glide_time;
    if awe_enabled {
        patch.settings.awe = Some(awe_ui.to_awe_state(true));
    }
    Some(patch)
}

/// Create a patch from a `PatchEditor` without global settings.
///
/// Used by project save to extract each instrument's module graph independently.
/// Global settings (master volume, keyboard, glide, AWE) are stored at the
/// project level, not per-instrument.
pub fn create_patch_from_editor(name: &str, patch_editor: &PatchEditor) -> Patch {
    let mut patch = Patch::new(name);

    for module_id in patch_editor.module_ids() {
        if let Some((_descriptor, position, params)) = patch_editor.get_module_data(module_id) {
            let mut param_map = HashMap::new();
            for (pname, value) in params {
                param_map.insert(pname, ParamValue::Float(value));
            }
            patch.modules.push(ModuleState {
                id: module_id.to_string(),
                module_type: module_id.module_type,
                position: (position.x, position.y),
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
            module_state.position.0 + drop_pos.x,
            module_state.position.1 + drop_pos.y,
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
        let _ = session.connect(
            instrument_id,
            connection.from_module,
            connection.from_port,
            connection.to_module,
            connection.to_port,
        );
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
