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
use crate::engine::{
    ModuleType as TypedModuleType,
    TypedParam, TypedValue, OscillatorParam, FilterParam, LfoParam,
    TypedWaveform, TypedLfoWaveform, FilterMode, DelayParam, DelayMode,
    DistortionParam, DistortionMode,
};
use crate::engine::graph::Connection;
use crate::gui::rack_view::{RackView, EffectType};
use crate::gui::keyboard::PianoKeyboard;
use crate::modules::{
    Describable, ModuleCategory, ModuleDescriptor,
    Oscillator, MathOscillator, Filter, Envelope, Lfo, Amplifier, Mixer, StereoOutput,
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
    instance_counters: &mut HashMap<TypedModuleType, u16>,
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
    handle.send_blocking(EngineCommand::SetMasterVolume(patch.settings.master_volume));
    handle.send_blocking(EngineCommand::SetGlideTime(patch.settings.glide_time));
}

/// Load a single module from patch state.
fn load_module(
    module_state: &ModuleState,
    rack_view: &mut RackView,
    instance_counters: &mut HashMap<TypedModuleType, u16>,
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
        if let Some(typed_param) = find_param_by_name(descriptor, param_name) {
            // Update rack_view
            match value {
                ParamValue::Float(f) => {
                    rack_view.set_parameter(module_id, typed_param, *f);
                }
                ParamValue::Int(i) => {
                    rack_view.set_parameter(module_id, typed_param, *i as f32);
                }
                ParamValue::Bool(b) => {
                    rack_view.set_parameter(module_id, typed_param, if *b { 1.0 } else { 0.0 });
                }
                ParamValue::Choice(s) => {
                    if let Some(param_spec) = descriptor.parameters.iter().find(|p| p.id == typed_param) {
                        if let Some(ref choices) = param_spec.choices {
                            if let Some(idx) = choices.iter().position(|c| c.id == *s) {
                                rack_view.set_parameter(module_id, typed_param, idx as f32);
                            }
                        }
                    }
                }
            }

            // Send to engine
            let typed_value = convert_param_value_to_typed(typed_param, value);
            if let Some(et) = effect_type {
                handle.send(EngineCommand::SetEffectParameter {
                    effect_type: et,
                    param: typed_param,
                    value: typed_value,
                });
            } else if let Some(voice_module) = get_voice_module_for_typed_param(module_id, typed_param) {
                handle.send(EngineCommand::SetVoiceParameter {
                    target: voice_module,
                    param: typed_param,
                    value: typed_value,
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

            let mut param_map = HashMap::new();
            for (typed_param, value) in params {
                if let Some(name) = typed_param_to_name(typed_param) {
                    param_map.insert(name, ParamValue::Float(value));
                }
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

/// Find TypedParam by name from a module descriptor.
pub fn find_param_by_name(descriptor: &ModuleDescriptor, name: &str) -> Option<TypedParam> {
    // Normalize name for comparison
    let normalized = name.to_lowercase();
    descriptor.parameters.iter()
        .find(|p| p.name.to_lowercase() == normalized || p.id.name().to_lowercase() == normalized)
        .map(|p| p.id)
}

/// Convert TypedParam to name for serialization.
pub fn typed_param_to_name(param: TypedParam) -> Option<String> {
    Some(param.name().to_lowercase())
}

/// Convert ParamValue to TypedValue based on the TypedParam type.
pub fn convert_param_value_to_typed(param: TypedParam, value: &ParamValue) -> TypedValue {
    match value {
        ParamValue::Float(f) => TypedValue::Float(*f),
        ParamValue::Int(i) => TypedValue::Int(*i),
        ParamValue::Bool(b) => TypedValue::Bool(*b),
        ParamValue::Choice(s) => {
            // Convert choice strings to appropriate TypedValue based on param type
            match param {
                TypedParam::Oscillator(OscillatorParam::Waveform) => {
                    let wf = match s.as_str() {
                        "sine" => TypedWaveform::Sine,
                        "triangle" => TypedWaveform::Triangle,
                        "sawtooth" => TypedWaveform::Sawtooth,
                        "square" => TypedWaveform::Square,
                        "pulse" => TypedWaveform::Pulse,
                        "noise" => TypedWaveform::Noise,
                        "pink_noise" => TypedWaveform::PinkNoise,
                        _ => TypedWaveform::Sawtooth,
                    };
                    TypedValue::Waveform(wf)
                }
                TypedParam::Lfo(LfoParam::Waveform) => {
                    let wf = match s.as_str() {
                        "sine" => TypedLfoWaveform::Sine,
                        "triangle" => TypedLfoWaveform::Triangle,
                        "sawtooth" => TypedLfoWaveform::Sawtooth,
                        "square" => TypedLfoWaveform::Square,
                        "sample_and_hold" | "s&h" => TypedLfoWaveform::SampleAndHold,
                        _ => TypedLfoWaveform::Sine,
                    };
                    TypedValue::LfoWaveform(wf)
                }
                TypedParam::Filter(FilterParam::Mode) => {
                    let mode = match s.as_str() {
                        "lowpass" | "lp" => FilterMode::Lowpass,
                        "highpass" | "hp" => FilterMode::Highpass,
                        "bandpass" | "bp" => FilterMode::Bandpass,
                        "notch" => FilterMode::Notch,
                        "peak" => FilterMode::Peak,
                        "low_shelf" => FilterMode::LowShelf,
                        "high_shelf" => FilterMode::HighShelf,
                        _ => FilterMode::Lowpass,
                    };
                    TypedValue::FilterMode(mode)
                }
                TypedParam::Delay(DelayParam::Mode) => {
                    let mode = match s.as_str() {
                        "mono" => DelayMode::Mono,
                        "stereo" => DelayMode::Stereo,
                        "ping_pong" => DelayMode::PingPong,
                        _ => DelayMode::Mono,
                    };
                    TypedValue::DelayMode(mode)
                }
                TypedParam::Distortion(DistortionParam::Mode) => {
                    let mode = match s.as_str() {
                        "soft_clip" => DistortionMode::SoftClip,
                        "hard_clip" => DistortionMode::HardClip,
                        "tube" => DistortionMode::Tube,
                        "foldback" => DistortionMode::Foldback,
                        "bitcrush" => DistortionMode::Bitcrush,
                        _ => DistortionMode::SoftClip,
                    };
                    TypedValue::DistortionMode(mode)
                }
                _ => TypedValue::Float(0.0),
            }
        }
    }
}

/// Get the VoiceModule target for a given TypedParam.
pub fn get_voice_module_for_typed_param(module_id: ModuleId, param: TypedParam) -> Option<VoiceModule> {
    // First try to get from module ID mapping
    if let Some(vm) = VoiceModule::from_module_id(module_id) {
        return Some(vm);
    }

    // Fall back to parameter type inference
    match param {
        TypedParam::Oscillator(_) => Some(VoiceModule::Oscillator1),
        TypedParam::Filter(_) => Some(VoiceModule::Filter),
        TypedParam::Envelope(_) => Some(VoiceModule::AmpEnvelope),
        TypedParam::Lfo(_) => Some(VoiceModule::Lfo),
        TypedParam::Amplifier(_) => Some(VoiceModule::Amplifier),
        TypedParam::Mixer(_) => Some(VoiceModule::Mixer),
        // Effects are handled separately
        TypedParam::Delay(_) | TypedParam::Reverb(_) |
        TypedParam::Distortion(_) | TypedParam::Chorus(_) => None,
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
