//! Patch Bridge - translates between Patch data and Engine commands.
//!
//! This module handles the conversion between:
//! - Patch file format (serialized state)
//! - PatchEditor UI state
//! - Engine commands for audio processing

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::Pos2;

use crate::effects::{Chorus, Delay, Distortion, Reverb};
use crate::engine::commands::{PolyModule, PortId};
use crate::engine::graph::Connection;
use crate::engine::instrument::InstrumentId;
use crate::engine::typed_params::{ModuleType, Param};
use crate::engine::{EngineCommand, EngineHandle, ModuleId};
use crate::gui::keyboard::PianoKeyboard;
use crate::gui::patch_editor::{EffectType, PatchEditor};
use crate::modules::{
    Amplifier, Describable, Envelope, Filter, Lfo, MathOscillator, Mixer, ModuleCategory,
    ModuleDescriptor, NoiseGenerator, Oscillator, StereoOutput, SubOscillator,
};
use crate::patch::{
    ConnectionState, ModuleState, ModuleType as PatchModuleType, ParamValue, Patch,
};
use crate::visualizers::{LevelMeter, Oscilloscope};

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
    instance_counters: &mut HashMap<ModuleType, u16>,
    handle: &mut EngineHandle,
    keyboard: &mut PianoKeyboard,
    glide_time: &mut f32,
    instrument_id: InstrumentId,
) {
    // Clear only the target instrument's modules (not destructive for multi-timbral)
    // First remove all modules from the engine for this instrument
    for module_id in patch_editor.module_ids() {
        // Check if this is an effect or visualizer (global) vs instrument module
        let category = patch_editor
            .module_descriptor(module_id)
            .map(|d| d.category);
        match category {
            Some(crate::modules::ModuleCategory::Effect) => {
                // Effects are now per-instrument - remove from this instrument's effect chain
                handle.send_blocking(EngineCommand::RemoveEffect {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                });
            }
            Some(crate::modules::ModuleCategory::Visualizer) => {
                // Visualizers are now per-instrument - remove from this instrument's effect chain
                handle.send_blocking(EngineCommand::RemoveVisualizer {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                });
                handle.remove_visualization_buffer(module_id);
            }
            _ => {
                handle.send_blocking(EngineCommand::RemoveModule {
                    instrument_id: Some(instrument_id),
                    id: module_id,
                });
            }
        }
    }

    // Clear GUI state
    patch_editor.clear();
    instance_counters.clear();

    // Add modules from patch
    for module_state in &patch.modules {
        load_module(
            module_state,
            patch_editor,
            instance_counters,
            handle,
            instrument_id,
        );
    }

    // Add connections to both patch_editor and engine
    for conn in &patch.connections {
        if let (Ok(from_id), Ok(to_id)) = (
            conn.from.0.parse::<ModuleId>(),
            conn.to.0.parse::<ModuleId>(),
        ) {
            let connection = Connection::new(from_id, &conn.from.1, to_id, &conn.to.1);
            patch_editor.add_connection(connection);

            // Send connection to engine - blocking to ensure all connections are established
            // Use the target instrument_id for instrument-level connections
            handle.send_blocking(EngineCommand::Connect {
                instrument_id: Some(instrument_id),
                from: PortId {
                    module: from_id,
                    port: conn.from.1.clone(),
                },
                to: PortId {
                    module: to_id,
                    port: conn.to.1.clone(),
                },
            });
        }
    }

    // Apply settings
    keyboard.set_octave_offset(patch.settings.octave_offset);
    *glide_time = patch.settings.glide_time;
    handle.send_blocking(EngineCommand::SetMasterVolume(crate::types::Gain::new(
        patch.settings.master_volume,
    )));
    handle.send_blocking(EngineCommand::SetGlideTime(crate::types::Seconds::new(
        patch.settings.glide_time,
    )));

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
    instance_counters: &mut HashMap<ModuleType, u16>,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
) {
    // Parse module ID from patch file (e.g., "osc-1" -> ModuleId)
    let module_id: ModuleId = match module_state.id.parse() {
        Ok(id) => id,
        Err(_) => return, // Skip invalid IDs
    };

    // Update instance counter to track highest instance number
    let counter = instance_counters.entry(module_id.module_type).or_insert(0);
    if module_id.instance > *counter {
        *counter = module_id.instance;
    }

    let position = Pos2::new(module_state.position.0, module_state.position.1);

    // Create module descriptor and instance based on type
    match module_state.module_type {
        PatchModuleType::Oscillator => {
            let m = Oscillator::new();
            let descriptor = m.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
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
        PatchModuleType::MathOscillator => {
            let m = MathOscillator::new();
            let descriptor = m.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
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
        PatchModuleType::SubOscillator => {
            let m = SubOscillator::new();
            let descriptor = m.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
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
        PatchModuleType::Noise => {
            let m = NoiseGenerator::new();
            let descriptor = m.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
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
        PatchModuleType::Filter => {
            let m = Filter::new();
            let descriptor = m.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
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
        PatchModuleType::Envelope => {
            let m = Envelope::new();
            let descriptor = m.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
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
        PatchModuleType::Lfo => {
            let m = Lfo::new();
            let descriptor = m.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
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
        PatchModuleType::Amplifier => {
            let m = Amplifier::new();
            let descriptor = m.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
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
        PatchModuleType::Mixer => {
            let m = Mixer::new();
            let descriptor = m.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
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
        PatchModuleType::StereoOutput => {
            let m = StereoOutput::new();
            let descriptor = m.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
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
        PatchModuleType::Delay => {
            let e = Delay::new();
            let descriptor = e.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
            // Effects are per-instrument - add to this instrument's effect chain
            handle.send(EngineCommand::AddEffectInstance {
                instrument_id: Some(instrument_id),
                id: module_id,
                effect: Box::new(e),
            });
            apply_module_parameters(
                module_id,
                &descriptor,
                &module_state.parameters,
                Some(EffectType::Delay),
                patch_editor,
                handle,
                instrument_id,
            );
        }
        PatchModuleType::Reverb => {
            let e = Reverb::new();
            let descriptor = e.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddEffectInstance {
                instrument_id: Some(instrument_id),
                id: module_id,
                effect: Box::new(e),
            });
            apply_module_parameters(
                module_id,
                &descriptor,
                &module_state.parameters,
                Some(EffectType::Reverb),
                patch_editor,
                handle,
                instrument_id,
            );
        }
        PatchModuleType::Distortion => {
            let e = Distortion::new();
            let descriptor = e.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddEffectInstance {
                instrument_id: Some(instrument_id),
                id: module_id,
                effect: Box::new(e),
            });
            apply_module_parameters(
                module_id,
                &descriptor,
                &module_state.parameters,
                Some(EffectType::Distortion),
                patch_editor,
                handle,
                instrument_id,
            );
        }
        PatchModuleType::Chorus => {
            let e = Chorus::new();
            let descriptor = e.descriptor();
            patch_editor.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddEffectInstance {
                instrument_id: Some(instrument_id),
                id: module_id,
                effect: Box::new(e),
            });
            apply_module_parameters(
                module_id,
                &descriptor,
                &module_state.parameters,
                Some(EffectType::Chorus),
                patch_editor,
                handle,
                instrument_id,
            );
        }
        PatchModuleType::Oscilloscope => {
            let descriptor = Oscilloscope::new().descriptor();
            patch_editor.add_module_at(module_id, descriptor, position);

            // Create shared visualization buffer
            let buffer = Arc::new(crate::visualizers::VisualizationBuffer::new(4096));
            handle.add_visualization_buffer(module_id, buffer.clone());

            // Visualizers are per-instrument - add to this instrument's effect chain
            handle.send(EngineCommand::AddVisualizer {
                instrument_id: Some(instrument_id),
                id: module_id,
                visualizer_type: crate::engine::commands::VisualizerType::Oscilloscope,
                buffer,
            });
        }
        PatchModuleType::LevelMeter => {
            let descriptor = LevelMeter::new().descriptor();
            patch_editor.add_module_at(module_id, descriptor, position);

            // Create shared visualization buffer
            let buffer = Arc::new(crate::visualizers::VisualizationBuffer::new(4096));
            handle.add_visualization_buffer(module_id, buffer.clone());

            // Visualizers are per-instrument - add to this instrument's effect chain
            handle.send(EngineCommand::AddVisualizer {
                instrument_id: Some(instrument_id),
                id: module_id,
                visualizer_type: crate::engine::commands::VisualizerType::LevelMeter,
                buffer,
            });
        }
    }
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

            // Update patch_editor
            patch_editor.set_parameter_by_name(module_id, param_name, f32_value);

            // Create the Param with value and send to engine
            let param = param_desc.id.with_f32(f32_value);

            if let Some(et) = effect_type {
                // Effects are per-instrument - send to this instrument's effect chain
                handle.send(EngineCommand::SetEffectParameter {
                    instrument_id: Some(instrument_id),
                    effect_type: et,
                    param,
                });
            } else if let Some(voice_module) = get_voice_module_for_param(module_id, &param) {
                handle.send(EngineCommand::SetVoiceParameter {
                    instrument_id,
                    target: voice_module,
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
) -> Option<Patch> {
    let mut patch = Patch::new(patch_name);
    patch.author = Some("User".to_string());

    // Add modules
    for module_id in patch_editor.module_ids() {
        if let Some((descriptor, position, params)) = patch_editor.get_module_data(module_id) {
            let module_type = match descriptor.category {
                ModuleCategory::Oscillator => PatchModuleType::Oscillator,
                ModuleCategory::Filter => PatchModuleType::Filter,
                ModuleCategory::Envelope => PatchModuleType::Envelope,
                ModuleCategory::LFO => PatchModuleType::Lfo,
                ModuleCategory::Amplifier => PatchModuleType::Amplifier,
                ModuleCategory::Mixer => PatchModuleType::Mixer,
                ModuleCategory::Effect => PatchModuleType::Delay, // Default to delay for effects
                _ => continue,
            };

            // params is now HashMap<String, f32>
            let mut param_map = HashMap::new();
            for (name, value) in params {
                param_map.insert(name, ParamValue::Float(value));
            }

            patch.modules.push(ModuleState {
                id: module_id.to_string(),
                module_type,
                position: (position.x, position.y),
                parameters: param_map,
            });
        }
    }

    // Add connections
    for conn in patch_editor.connections() {
        patch.connections.push(ConnectionState {
            from: (conn.from_module.to_string(), conn.from_port.clone()),
            to: (conn.to_module.to_string(), conn.to_port.clone()),
        });
    }

    patch.settings.octave_offset = keyboard.octave_offset();
    patch.settings.master_volume = handle.master_volume();
    patch.settings.glide_time = glide_time;

    Some(patch)
}

/// Get the PolyModule target for a given Param.
pub fn get_voice_module_for_param(module_id: ModuleId, param: &Param) -> Option<PolyModule> {
    // First try to get from module ID mapping
    if let Some(vm) = PolyModule::from_module_id(module_id) {
        return Some(vm);
    }

    // Fall back to parameter type inference
    match param {
        Param::Oscillator(_) => Some(PolyModule::Oscillator1),
        Param::MathOscillator(_) => Some(PolyModule::Oscillator1),
        Param::Filter(_) => Some(PolyModule::Filter),
        Param::Envelope(_) => Some(PolyModule::AmpEnvelope),
        Param::Lfo(_) => Some(PolyModule::Lfo),
        Param::Amplifier(_) => Some(PolyModule::Amplifier),
        Param::Mixer(_) => Some(PolyModule::Mixer),
        // Effects are handled separately
        Param::Delay(_)
        | Param::Reverb(_)
        | Param::Distortion(_)
        | Param::Chorus(_)
        | Param::Phaser(_)
        | Param::Flanger(_)
        | Param::Compressor(_)
        | Param::Eq(_) => None,
        // Visualizers and other types
        _ => None,
    }
}

/// Get the EffectType for a module from its descriptor.
pub fn get_effect_type_from_module(
    patch_editor: &PatchEditor,
    module_id: ModuleId,
) -> Option<EffectType> {
    let desc = patch_editor.module_descriptor(module_id)?;

    // Match based on type_id
    match desc.type_id.0.as_str() {
        "chorus" => Some(EffectType::Chorus),
        "delay" => Some(EffectType::Delay),
        "reverb" => Some(EffectType::Reverb),
        "distortion" => Some(EffectType::Distortion),
        "phaser" => Some(EffectType::Phaser),
        "flanger" => Some(EffectType::Flanger),
        "compressor" => Some(EffectType::Compressor),
        "eq" => Some(EffectType::Eq),
        _ => None,
    }
}
