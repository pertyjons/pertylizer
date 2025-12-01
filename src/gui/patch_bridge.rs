//! Patch Bridge - translates between Patch data and Engine commands.
//!
//! This module handles the conversion between:
//! - Patch file format (serialized state)
//! - RackView UI state
//! - Engine commands for audio processing

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::Pos2;

use crate::engine::{EngineCommand, EngineHandle, ModuleId};
use crate::engine::commands::{VoiceModule, PortId};
use crate::engine::part::PartId;
use crate::engine::typed_params::{ModuleType, Param};
use crate::engine::graph::Connection;
use crate::gui::rack_view::{RackView, EffectType};
use crate::gui::keyboard::PianoKeyboard;
use crate::modules::{
    Describable, ModuleCategory, ModuleDescriptor,
    Oscillator, MathOscillator, SubOscillator, NoiseGenerator,
    Filter, Envelope, Lfo, Amplifier, Mixer, StereoOutput,
};
use crate::effects::{Chorus, Delay, Distortion, Reverb};
use crate::visualizers::{Oscilloscope, LevelMeter};
use crate::patch::{Patch, ModuleType as PatchModuleType, ModuleState, ConnectionState, ParamValue};

/// Load a patch into the rack view and send commands to the engine.
///
/// This function handles all the logic for:
/// - Clearing existing state
/// - Creating module instances
/// - Applying parameters
/// - Establishing connections
pub fn load_patch(
    patch: &Patch,
    rack_view: &mut RackView,
    instance_counters: &mut HashMap<ModuleType, u16>,
    handle: &mut EngineHandle,
    keyboard: &mut PianoKeyboard,
    glide_time: &mut f32,
) {
    // Clear existing modules and connections
    rack_view.clear();
    instance_counters.clear();

    // Clear engine state - blocking to ensure it completes before adding new modules
    handle.send_blocking(EngineCommand::ClearAllModules);

    // Add modules from patch
    for module_state in &patch.modules {
        load_module(module_state, rack_view, instance_counters, handle);
    }

    // Add connections to both rack_view and engine
    for conn in &patch.connections {
        if let (Ok(from_id), Ok(to_id)) = (conn.from.0.parse::<ModuleId>(), conn.to.0.parse::<ModuleId>()) {
            let connection = Connection::new(
                from_id,
                &conn.from.1,
                to_id,
                &conn.to.1,
            );
            rack_view.add_connection(connection);

            // Send connection to engine - blocking to ensure all connections are established
            handle.send_blocking(EngineCommand::Connect {
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
    handle.send_blocking(EngineCommand::SetMasterVolume(
        crate::types::Gain::new(patch.settings.master_volume)
    ));
    handle.send_blocking(EngineCommand::SetGlideTime(
        crate::types::Seconds::new(patch.settings.glide_time)
    ));

    // Re-enable first part after loading (ClearAllModules disables all parts)
    handle.send_blocking(EngineCommand::SetPartEnabled {
        part_id: PartId::FIRST,
        enabled: true,
    });
}

/// Load a single module from patch state.
fn load_module(
    module_state: &ModuleState,
    rack_view: &mut RackView,
    instance_counters: &mut HashMap<ModuleType, u16>,
    handle: &mut EngineHandle,
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
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddModuleInstance {
                id: module_id,
                module: Box::new(m),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters, None, rack_view, handle);
        }
        PatchModuleType::MathOscillator => {
            let m = MathOscillator::new();
            let descriptor = m.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddModuleInstance {
                id: module_id,
                module: Box::new(m),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters, None, rack_view, handle);
        }
        PatchModuleType::SubOscillator => {
            let m = SubOscillator::new();
            let descriptor = m.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddModuleInstance {
                id: module_id,
                module: Box::new(m),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters, None, rack_view, handle);
        }
        PatchModuleType::Noise => {
            let m = NoiseGenerator::new();
            let descriptor = m.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddModuleInstance {
                id: module_id,
                module: Box::new(m),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters, None, rack_view, handle);
        }
        PatchModuleType::Filter => {
            let m = Filter::new();
            let descriptor = m.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddModuleInstance {
                id: module_id,
                module: Box::new(m),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters, None, rack_view, handle);
        }
        PatchModuleType::Envelope => {
            let m = Envelope::new();
            let descriptor = m.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddModuleInstance {
                id: module_id,
                module: Box::new(m),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters, None, rack_view, handle);
        }
        PatchModuleType::Lfo => {
            let m = Lfo::new();
            let descriptor = m.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddModuleInstance {
                id: module_id,
                module: Box::new(m),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters, None, rack_view, handle);
        }
        PatchModuleType::Amplifier => {
            let m = Amplifier::new();
            let descriptor = m.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddModuleInstance {
                id: module_id,
                module: Box::new(m),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters, None, rack_view, handle);
        }
        PatchModuleType::Mixer => {
            let m = Mixer::new();
            let descriptor = m.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddModuleInstance {
                id: module_id,
                module: Box::new(m),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters, None, rack_view, handle);
        }
        PatchModuleType::StereoOutput => {
            let m = StereoOutput::new();
            let descriptor = m.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddModuleInstance {
                id: module_id,
                module: Box::new(m),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters, None, rack_view, handle);
        }
        PatchModuleType::Delay => {
            let e = Delay::new();
            let descriptor = e.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddEffectInstance {
                id: module_id,
                effect: Box::new(e),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters,
                Some(EffectType::Delay), rack_view, handle);
        }
        PatchModuleType::Reverb => {
            let e = Reverb::new();
            let descriptor = e.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddEffectInstance {
                id: module_id,
                effect: Box::new(e),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters,
                Some(EffectType::Reverb), rack_view, handle);
        }
        PatchModuleType::Distortion => {
            let e = Distortion::new();
            let descriptor = e.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddEffectInstance {
                id: module_id,
                effect: Box::new(e),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters,
                Some(EffectType::Distortion), rack_view, handle);
        }
        PatchModuleType::Chorus => {
            let e = Chorus::new();
            let descriptor = e.descriptor();
            rack_view.add_module_at(module_id, descriptor.clone(), position);
            handle.send(EngineCommand::AddEffectInstance {
                id: module_id,
                effect: Box::new(e),
            });
            apply_module_parameters(module_id, &descriptor, &module_state.parameters,
                Some(EffectType::Chorus), rack_view, handle);
        }
        PatchModuleType::Oscilloscope => {
            let descriptor = Oscilloscope::new().descriptor();
            rack_view.add_module_at(module_id, descriptor, position);

            // Create shared visualization buffer
            let buffer = Arc::new(crate::visualizers::VisualizationBuffer::new(4096));
            handle.add_visualization_buffer(module_id, buffer.clone());

            handle.send(EngineCommand::AddVisualizer {
                id: module_id,
                visualizer_type: crate::engine::commands::VisualizerType::Oscilloscope,
                buffer,
            });
        }
        PatchModuleType::LevelMeter => {
            let descriptor = LevelMeter::new().descriptor();
            rack_view.add_module_at(module_id, descriptor, position);

            // Create shared visualization buffer
            let buffer = Arc::new(crate::visualizers::VisualizationBuffer::new(4096));
            handle.add_visualization_buffer(module_id, buffer.clone());

            handle.send(EngineCommand::AddVisualizer {
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
    rack_view: &mut RackView,
    handle: &mut EngineHandle,
) {
    for (param_name, value) in parameters {
        // Find the parameter descriptor by name
        let param_desc = descriptor.parameters.iter()
            .find(|p| p.name.to_lowercase() == param_name.to_lowercase());

        if let Some(param_desc) = param_desc {
            // Convert the ParamValue to f32 for rack_view
            let f32_value = match value {
                ParamValue::Float(f) => *f,
                ParamValue::Int(i) => *i as f32,
                ParamValue::Bool(b) => if *b { 1.0 } else { 0.0 },
                ParamValue::Choice(s) => {
                    // Look up choice index
                    if let Some(ref choices) = param_desc.choices {
                        choices.iter().position(|c| c.id == *s)
                            .map(|i| i as f32)
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    }
                }
            };

            // Update rack_view
            rack_view.set_parameter_by_name(module_id, param_name, f32_value);

            // Create the Param with value and send to engine
            let param = param_desc.id.with_f32(f32_value);

            if let Some(et) = effect_type {
                handle.send(EngineCommand::SetEffectParameter {
                    effect_type: et,
                    param,
                });
            } else if let Some(voice_module) = get_voice_module_for_param(module_id, &param) {
                handle.send(EngineCommand::SetVoiceParameter {
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
    rack_view: &RackView,
    keyboard: &PianoKeyboard,
    handle: &EngineHandle,
    glide_time: f32,
) -> Option<Patch> {
    let mut patch = Patch::new(patch_name);
    patch.author = Some("User".to_string());

    // Add modules
    for module_id in rack_view.module_ids() {
        if let Some((descriptor, position, params)) = rack_view.get_module_data(module_id) {
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
    for conn in rack_view.connections() {
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

/// Get the VoiceModule target for a given Param.
pub fn get_voice_module_for_param(module_id: ModuleId, param: &Param) -> Option<VoiceModule> {
    // First try to get from module ID mapping
    if let Some(vm) = VoiceModule::from_module_id(module_id) {
        return Some(vm);
    }

    // Fall back to parameter type inference
    match param {
        Param::Oscillator(_) => Some(VoiceModule::Oscillator1),
        Param::MathOscillator(_) => Some(VoiceModule::Oscillator1),
        Param::Filter(_) => Some(VoiceModule::Filter),
        Param::Envelope(_) => Some(VoiceModule::AmpEnvelope),
        Param::Lfo(_) => Some(VoiceModule::Lfo),
        Param::Amplifier(_) => Some(VoiceModule::Amplifier),
        Param::Mixer(_) => Some(VoiceModule::Mixer),
        // Effects are handled separately
        Param::Delay(_) | Param::Reverb(_) |
        Param::Distortion(_) | Param::Chorus(_) |
        Param::Phaser(_) | Param::Flanger(_) |
        Param::Compressor(_) | Param::Eq(_) => None,
        // Visualizers and other types
        _ => None,
    }
}

/// Get the EffectType for a module from its descriptor.
pub fn get_effect_type_from_module(rack_view: &RackView, module_id: ModuleId) -> Option<EffectType> {
    let desc = rack_view.module_descriptor(module_id)?;

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
