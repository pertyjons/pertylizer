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
use crate::patch::{ConnectionState, ModuleState, ParamValue, Patch, PatchModuleType};
use crate::session::SynthSession;
use synth_core::{Describable, ModuleCategory, ModuleDescriptor};
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
        PatchModuleType::SignalMonitor => {
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
        PatchModuleType::InlineSignalMonitor => {
            load_inline_signal_monitor(
                module_id,
                patch_editor,
                session,
                handle,
                instrument_id,
                position,
            );
            return;
        }
        PatchModuleType::Oscilloscope => {
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
        PatchModuleType::LevelMeter => {
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
        PatchModuleType::SpectrumAnalyzer => {
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

/// Load an InlineSignalMonitor module (compact pass-through with vis buffer).
fn load_inline_signal_monitor(
    module_id: ModuleId,
    patch_editor: &mut PatchEditor,
    session: &SynthSession,
    handle: &mut EngineHandle,
    instrument_id: InstrumentId,
    position: Pos2,
) {
    let mut m = synth_modules::SignalMonitor::new();
    let inline_descriptor = synth_core::ModuleDescriptor::new("inline_signal_monitor", "Mon")
        .description("Inline signal monitor (compact pass-through)")
        .category(synth_core::ModuleCategory::Utility)
        .port(synth_core::PortDescriptor::audio_input("in", "In"))
        .port(synth_core::PortDescriptor::audio_output("out", "Out"));

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

    patch_editor.add_module_at(module_id, inline_descriptor, position);

    let buffer = Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
    handle.add_visualization_buffer(module_id, buffer.clone());
    m.set_vis_sink(buffer);

    handle.send(EngineCommand::AddModuleInstance {
        instrument_id: Some(instrument_id),
        id: module_id,
        module: Box::new(m),
    });
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
    let mut patch = Patch::new(patch_name);
    patch.author = Some("User".to_string());

    // Add modules
    for module_id in patch_editor.module_ids() {
        if let Some((descriptor, position, params)) = patch_editor.get_module_data(module_id) {
            let module_type = match descriptor.category {
                ModuleCategory::Oscillator => {
                    // Distinguish oscillator subtypes by type_id
                    match descriptor.type_id.0.as_str() {
                        "math_oscillator" => PatchModuleType::MathOscillator,
                        "sub_oscillator" => PatchModuleType::SubOscillator,
                        "noise" => PatchModuleType::Noise,
                        "ring_mod" => PatchModuleType::RingMod,
                        "wavetable_osc" => PatchModuleType::WavetableOsc,
                        "additive_osc" => PatchModuleType::AdditiveOsc,
                        _ => PatchModuleType::Oscillator,
                    }
                }
                ModuleCategory::Filter => PatchModuleType::Filter,
                ModuleCategory::Envelope => match descriptor.type_id.0.as_str() {
                    "mseg" => PatchModuleType::Mseg,
                    _ => PatchModuleType::Envelope,
                },
                ModuleCategory::LFO => match descriptor.type_id.0.as_str() {
                    "euclidean" => PatchModuleType::Euclidean,
                    "turing_machine" => PatchModuleType::TuringMachine,
                    "random_gates" => PatchModuleType::RandomGates,
                    "kinetic_modulator" => PatchModuleType::KineticModulator,
                    _ => PatchModuleType::Lfo,
                },
                ModuleCategory::Amplifier => PatchModuleType::Amplifier,
                ModuleCategory::Mixer => PatchModuleType::Mixer,
                ModuleCategory::Output => PatchModuleType::StereoOutput,
                ModuleCategory::Effect => match descriptor.type_id.0.as_str() {
                    "delay" => PatchModuleType::Delay,
                    "reverb" => PatchModuleType::Reverb,
                    "distortion" => PatchModuleType::Distortion,
                    "chorus" => PatchModuleType::Chorus,
                    "waveshaper" => PatchModuleType::Waveshaper,
                    "mid_side" => PatchModuleType::MidSide,
                    "bbd_delay" => PatchModuleType::BbdDelay,
                    "limiter" => PatchModuleType::Limiter,
                    _ => PatchModuleType::Delay, // Fallback for other effects
                },
                ModuleCategory::Utility => match descriptor.type_id.0.as_str() {
                    "mod_matrix" => PatchModuleType::ModMatrix,
                    "envelope_follower" => PatchModuleType::EnvelopeFollower,
                    "signal_monitor" => PatchModuleType::SignalMonitor,
                    "inline_signal_monitor" => PatchModuleType::InlineSignalMonitor,
                    _ => continue,
                },
                ModuleCategory::PhysicalModeling => match descriptor.type_id.0.as_str() {
                    "keyboard_panner" => PatchModuleType::KeyboardPanner,
                    "body_resonance" => PatchModuleType::BodyResonance,
                    "mechanical_noise" => PatchModuleType::MechanicalNoise,
                    _ => continue,
                },
                ModuleCategory::Visualizer => match descriptor.type_id.0.as_str() {
                    "oscilloscope" => PatchModuleType::Oscilloscope,
                    "level_meter" => PatchModuleType::LevelMeter,
                    "spectrum_analyzer" => PatchModuleType::SpectrumAnalyzer,
                    _ => continue,
                },
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
            from: (conn.from_module.to_string(), conn.from_port.into()),
            to: (conn.to_module.to_string(), conn.to_port.into()),
        });
    }

    patch.settings.octave_offset = keyboard.octave_offset();
    patch.settings.master_volume = handle.master_volume();
    patch.settings.glide_time = glide_time;

    // Save full AWE state
    if awe_enabled {
        patch.settings.awe = Some(awe_ui.to_awe_state(true));
    }

    Some(patch)
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
        "waveshaper" => Some(EffectType::Waveshaper),
        "mid_side" => Some(EffectType::MidSide),
        "bbd_delay" => Some(EffectType::BbdDelay),
        "limiter" => Some(EffectType::Limiter),
        "convolver" => Some(EffectType::Convolver),
        "phase_vocoder" => Some(EffectType::PhaseVocoder),
        "frequency_shifter" => Some(EffectType::FrequencyShifter),
        _ => None,
    }
}
