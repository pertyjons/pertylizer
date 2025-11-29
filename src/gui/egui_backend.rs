//! Egui-based graphical GUI backend.
//!
//! This provides a full graphical interface using egui/eframe,
//! with a modular synthesizer-style layout featuring:
//! - Visual node-based module arrangement
//! - Cable connections between modules
//! - Dynamic module addition/removal
//! - Real-time audio metering
//! - Save/load patches

use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{self, Color32, Pos2, RichText, Stroke, Vec2};

use crate::audio::AudioHostTrait;
use crate::engine::{EngineCommand, EngineHandle, SynthEngine, ModuleId};
use crate::engine::commands::{VoiceModule, PortId};
use crate::engine::{
    ModuleType as TypedModuleType,
    TypedParam, TypedValue, OscillatorParam, FilterParam, EnvelopeParam, LfoParam,
    AmplifierParam, DelayParam, DistortionParam,
    TypedWaveform, TypedLfoWaveform, FilterMode, DelayMode, DistortionMode,
};
use crate::engine::graph::Connection;
use crate::gui::{GuiBackend, GuiResult, SynthGuiConfig};
use crate::gui::widgets::colors;
use crate::gui::keyboard::PianoKeyboard;
use crate::gui::rack_view::{RackView, ModulePalette, PaletteSelection, EffectType, VisualizerType};
use crate::modules::{
    Describable, ModuleCategory, ModuleDescriptor,
    Oscillator, MathOscillator, Filter, Envelope, Lfo, Amplifier, Mixer, StereoOutput,
};
use crate::effects::{Chorus, Compressor, Delay, Distortion, Eq, Flanger, Phaser, Reverb};
use crate::visualizers::{Oscilloscope, LevelMeter};
use crate::patch::{Patch, ModuleType as PatchModuleType, ModuleState, ConnectionState, example_patches};

/// Egui-based GUI backend.
pub struct EguiBackend;

impl EguiBackend {
    /// Create a new egui backend.
    pub fn new() -> Self {
        Self
    }
}

impl Default for EguiBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl GuiBackend for EguiBackend {
    fn name(&self) -> &'static str {
        "egui"
    }

    fn run(
        self: Box<Self>,
        engine: SynthEngine,
        handle: EngineHandle,
        mut host: Box<dyn AudioHostTrait>,
        config: SynthGuiConfig,
    ) -> GuiResult<()> {
        // Start audio before GUI
        let stream_info = host.start_output(None, &config.stream_config, Box::new(engine))?;
        
        let app = SynthApp::new(handle, host, config.clone(), stream_info.output_latency);
        
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([config.width as f32, config.height as f32])
                .with_title(&config.title)
                .with_min_inner_size([800.0, 600.0]),
            ..Default::default()
        };

        eframe::run_native(
            &config.title,
            options,
            Box::new(|cc| {
                setup_custom_style(&cc.egui_ctx);
                Ok(Box::new(app))
            }),
        ).map_err(|e| format!("Egui error: {e}"))?;

        Ok(())
    }
}

/// Setup custom egui style for synth look.
fn setup_custom_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    
    // Dark theme with synth colors
    style.visuals.dark_mode = true;
    style.visuals.override_text_color = Some(colors::TEXT_PRIMARY);
    style.visuals.panel_fill = colors::BG_PANEL;
    style.visuals.window_fill = colors::BG_MODULE;
    style.visuals.faint_bg_color = colors::BG_WIDGET;
    
    // Widget styling
    style.visuals.widgets.inactive.bg_fill = colors::BG_WIDGET;
    style.visuals.widgets.inactive.weak_bg_fill = colors::BG_WIDGET;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, colors::TEXT_SECONDARY);
    
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(60, 65, 80);
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, colors::TEXT_PRIMARY);
    
    style.visuals.widgets.active.bg_fill = colors::ACCENT_ORANGE;
    style.visuals.widgets.active.fg_stroke = Stroke::new(2.0, colors::BG_DARK);
    
    style.visuals.selection.bg_fill = colors::ACCENT_ORANGE.gamma_multiply(0.4);
    style.visuals.selection.stroke = Stroke::new(1.0, colors::ACCENT_ORANGE);
    
    // Rounded corners
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(4);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(4);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(4);
    
    // Spacing
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    
    ctx.set_style(style);
}

/// Main application state.
struct SynthApp {
    handle: EngineHandle,
    host: Option<Box<dyn AudioHostTrait>>,
    config: SynthGuiConfig,
    latency: std::time::Duration,

    // Rack view state
    rack_view: RackView,

    // Module ID generation - track instance counts per module type
    instance_counters: HashMap<TypedModuleType, u16>,

    // Keyboard state
    keyboard: PianoKeyboard,
    pressed_keys: HashMap<u8, bool>, // For computer keyboard tracking

    // Dialogs
    show_settings: bool,
    show_about: bool,
    show_add_module: bool,
    show_load_patch: bool,
    show_save_patch: bool,

    // Patch state
    current_patch_name: String,
    current_patch_path: Option<PathBuf>,
    patch_save_name: String,
    status_message: Option<(String, std::time::Instant)>,

    // Global synth settings
    glide_time: f32,
}

impl SynthApp {
    fn new(
        handle: EngineHandle,
        host: Box<dyn AudioHostTrait>,
        config: SynthGuiConfig,
        latency: std::time::Duration,
    ) -> Self {
        let mut rack_view = RackView::new();
        
        // Add default modules to rack view with spacey bass preset values
        // Use VoiceModule enum for type-safe module IDs
        
        // Oscillator 1 - Sawtooth, main sound
        let osc1 = Oscillator::new();
        let osc1_id = VoiceModule::Oscillator1.module_id();
        rack_view.add_module(osc1_id, osc1.descriptor());
        rack_view.set_parameter(osc1_id, TypedParam::Oscillator(OscillatorParam::Waveform), 2.0); // Sawtooth
        rack_view.set_parameter(osc1_id, TypedParam::Oscillator(OscillatorParam::Level), 0.6);

        // Oscillator 2 - Sawtooth, detuned for thickness
        let osc2 = Oscillator::new();
        let osc2_id = VoiceModule::Oscillator2.module_id();
        rack_view.add_module(osc2_id, osc2.descriptor());
        rack_view.set_parameter(osc2_id, TypedParam::Oscillator(OscillatorParam::Waveform), 2.0); // Sawtooth
        rack_view.set_parameter(osc2_id, TypedParam::Oscillator(OscillatorParam::Level), 0.5);
        rack_view.set_parameter(osc2_id, TypedParam::Oscillator(OscillatorParam::Detune), 7.0); // +7 cents

        // Filter - Low pass with moderate resonance
        let filter = Filter::new();
        let filter_id = VoiceModule::Filter.module_id();
        rack_view.add_module(filter_id, filter.descriptor());
        rack_view.set_parameter(filter_id, TypedParam::Filter(FilterParam::Cutoff), 400.0);
        rack_view.set_parameter(filter_id, TypedParam::Filter(FilterParam::Resonance), 0.4);

        // Amp Envelope - Punchy bass envelope
        let amp_env = Envelope::new();
        let amp_env_id = VoiceModule::AmpEnvelope.module_id();
        rack_view.add_module(amp_env_id, amp_env.descriptor());
        rack_view.set_parameter(amp_env_id, TypedParam::Envelope(EnvelopeParam::Attack), 0.005);
        rack_view.set_parameter(amp_env_id, TypedParam::Envelope(EnvelopeParam::Decay), 0.2);
        rack_view.set_parameter(amp_env_id, TypedParam::Envelope(EnvelopeParam::Sustain), 0.6);
        rack_view.set_parameter(amp_env_id, TypedParam::Envelope(EnvelopeParam::Release), 0.3);

        // Filter Envelope - Opens filter on attack
        let filter_env = Envelope::new();
        let filter_env_id = VoiceModule::FilterEnvelope.module_id();
        rack_view.add_module(filter_env_id, filter_env.descriptor());
        rack_view.set_parameter(filter_env_id, TypedParam::Envelope(EnvelopeParam::Attack), 0.001);
        rack_view.set_parameter(filter_env_id, TypedParam::Envelope(EnvelopeParam::Decay), 0.3);
        rack_view.set_parameter(filter_env_id, TypedParam::Envelope(EnvelopeParam::Sustain), 0.2);
        rack_view.set_parameter(filter_env_id, TypedParam::Envelope(EnvelopeParam::Release), 0.4);

        // LFO - Slow sine for movement
        let lfo = Lfo::new();
        let lfo_id = VoiceModule::Lfo.module_id();
        rack_view.add_module(lfo_id, lfo.descriptor());
        rack_view.set_parameter(lfo_id, TypedParam::Lfo(LfoParam::Rate), 0.3);
        rack_view.set_parameter(lfo_id, TypedParam::Lfo(LfoParam::Depth), 0.25);

        // Amplifier
        let amp = Amplifier::new();
        let amp_id = VoiceModule::Amplifier.module_id();
        rack_view.add_module(amp_id, amp.descriptor());
        rack_view.set_parameter(amp_id, TypedParam::Amplifier(AmplifierParam::Level), 0.7);
        
        // Add default connections using enum IDs
        // Osc1 -> Filter, Osc2 -> Filter (both mixed into filter)
        rack_view.add_connection(Connection::new(osc1_id, "out", filter_id, "in"));
        rack_view.add_connection(Connection::new(osc2_id, "out", filter_id, "in"));
        // Filter -> Amp
        rack_view.add_connection(Connection::new(filter_id, "out", amp_id, "in"));
        // Amp Env -> Amp CV
        rack_view.add_connection(Connection::new(amp_env_id, "out", amp_id, "cv"));
        // Filter Env -> Filter Cutoff CV
        rack_view.add_connection(Connection::new(filter_env_id, "out", filter_id, "cutoff_cv"));
        // LFO -> Filter Cutoff CV (mixed with envelope)
        rack_view.add_connection(Connection::new(lfo_id, "out", filter_id, "cutoff_cv"));
        // LFO -> Osc1 FM (vibrato)
        rack_view.add_connection(Connection::new(lfo_id, "out", osc1_id, "fm"));
        
        Self {
            handle,
            host: Some(host),
            config,
            latency,
            rack_view,
            instance_counters: HashMap::new(),
            keyboard: PianoKeyboard::new(),
            pressed_keys: HashMap::new(),
            show_settings: false,
            show_about: false,
            show_add_module: false,
            show_load_patch: false,
            show_save_patch: false,
            current_patch_name: "Spacey Bass".to_string(),
            current_patch_path: None,
            patch_save_name: String::new(),
            status_message: None,
            glide_time: 0.0,
        }
    }

    /// Get the next ModuleId for a given module type.
    fn next_module_id(&mut self, module_type: TypedModuleType) -> ModuleId {
        let counter = self.instance_counters.entry(module_type).or_insert(0);
        *counter += 1;
        ModuleId::new(module_type, *counter)
    }
}

impl eframe::App for SynthApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle keyboard input
        self.handle_keyboard_input(ctx);
        
        // Request continuous repaint for meters
        ctx.request_repaint();
        
        // Top menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("📂 Load Patch...").clicked() {
                        self.show_load_patch = true;
                        ui.close();
                    }
                    if ui.button("💾 Save Patch...").clicked() {
                        self.patch_save_name = self.current_patch_name.clone();
                        self.show_save_patch = true;
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("📋 Example Patches", |ui| {
                        for patch in example_patches() {
                            if ui.button(&patch.name).clicked() {
                                self.load_patch_data(&patch);
                                self.current_patch_name = patch.name.clone();
                                self.status_message = Some((
                                    format!("Loaded: {}", patch.name),
                                    std::time::Instant::now()
                                ));
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("⚙ Settings...").clicked() {
                        self.show_settings = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("🚪 Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                
                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        self.show_about = true;
                        ui.close();
                    }
                });
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Status indicators
                    let cpu = self.handle.cpu_usage();
                    let cpu_color = if cpu > 0.8 {
                        colors::METER_RED
                    } else if cpu > 0.5 {
                        colors::METER_YELLOW
                    } else {
                        colors::METER_GREEN
                    };
                    ui.label(RichText::new(format!("CPU: {:.0}%", cpu * 100.0)).color(cpu_color));
                    ui.separator();
                    ui.label(RichText::new(format!("Voices: {}", self.handle.voice_count())).color(colors::TEXT_SECONDARY));
                    ui.separator();
                    ui.label(RichText::new(format!("Latency: {:.1}ms", self.latency.as_secs_f64() * 1000.0)).color(colors::TEXT_DIM));
                    ui.separator();
                    // Current patch name
                    ui.label(RichText::new(format!("Patch: {}", self.current_patch_name)).color(colors::ACCENT_CYAN));
                });
            });
        });
        
        // Toolbar for adding modules
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(selection) = ModulePalette::show(ui) {
                    match selection {
                        PaletteSelection::Category(category) => {
                            self.add_module_of_category(category);
                        }
                        PaletteSelection::MathOscillator => {
                            self.add_math_oscillator_module();
                        }
                        PaletteSelection::Effect(effect_type) => {
                            self.add_effect_module(effect_type);
                        }
                        PaletteSelection::Visualizer(viz_type) => {
                            self.add_visualizer_module(viz_type);
                        }
                        PaletteSelection::StereoOutput => {
                            self.add_stereo_output_module();
                        }
                    }
                }
                
                ui.separator();

                // Glide/Portamento control
                ui.label(RichText::new("Glide:").color(colors::TEXT_DIM));
                let glide_response = ui.add(
                    egui::Slider::new(&mut self.glide_time, 0.0..=2.0)
                        .suffix(" s")
                        .fixed_decimals(2)
                        .custom_formatter(|v, _| {
                            if v < 0.001 { "Off".to_string() } else { format!("{:.2}s", v) }
                        })
                );
                if glide_response.changed() {
                    self.handle.send(EngineCommand::SetGlideTime(self.glide_time));
                }

                ui.separator();

                // Connection info
                let conn_count = self.rack_view.connections().len();
                let module_count = self.rack_view.module_ids().len();
                ui.label(RichText::new(format!("Modules: {} | Connections: {}", module_count, conn_count))
                    .color(colors::TEXT_DIM));
            });
        });
        
        // Bottom panel with keyboard
        egui::TopBottomPanel::bottom("keyboard_panel")
            .min_height(120.0)
            .show(ctx, |ui| {
                self.draw_keyboard(ui);
            });
        
        // Side panel with meters
        egui::SidePanel::right("meters_panel")
            .min_width(80.0)
            .show(ctx, |ui| {
                self.draw_meters(ui);
            });
        
        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            let result = self.rack_view.show(ui, &self.handle);
            
            // Handle parameter changes - send typed params directly
            for (module_id, typed_param, typed_value) in result.param_changes {
                // Check module category
                let category = self.rack_view.module_descriptor(module_id)
                    .map(|d| d.category);

                match category {
                    Some(ModuleCategory::Effect) => {
                        // Effect module - use SetEffectParameter
                        if let Some(effect_type) = self.get_effect_type_from_module(module_id) {
                            self.handle.send(EngineCommand::SetEffectParameter {
                                effect_type,
                                param: typed_param,
                                value: typed_value,
                            });
                        }
                    }
                    Some(ModuleCategory::Oscillator) |
                    Some(ModuleCategory::Filter) |
                    Some(ModuleCategory::Envelope) |
                    Some(ModuleCategory::LFO) |
                    Some(ModuleCategory::Amplifier) |
                    Some(ModuleCategory::Mixer) => {
                        // Voice/modular module - send to both targets
                        // 1. Send to global module graph
                        self.handle.send(EngineCommand::SetModuleParameter {
                            module_id,
                            param: typed_param,
                            value: typed_value.clone(),
                        });

                        // 2. Also send to voice modules for backwards compatibility
                        if let Some(voice_module) = self.get_voice_module_for_typed_param(module_id, typed_param) {
                            self.handle.send(EngineCommand::SetVoiceParameter {
                                target: voice_module,
                                param: typed_param,
                                value: typed_value,
                            });
                        }
                    }
                    _ => {}
                }
            }
            
            // Handle module removal
            for module_id in result.modules_to_remove {
                // Get module descriptor to determine type
                let category = self.rack_view.module_descriptor(module_id)
                    .map(|d| d.category);
                
                self.rack_view.remove_module(module_id);
                
                // Send appropriate remove command to engine based on category
                match category {
                    Some(ModuleCategory::Visualizer) => {
                        self.handle.send(EngineCommand::RemoveVisualizer { id: module_id });
                        self.handle.remove_visualization_buffer(module_id);
                    }
                    Some(ModuleCategory::Effect) => {
                        self.handle.send(EngineCommand::RemoveEffect { id: module_id });
                    }
                    Some(ModuleCategory::Oscillator) |
                    Some(ModuleCategory::Filter) |
                    Some(ModuleCategory::Envelope) |
                    Some(ModuleCategory::LFO) |
                    Some(ModuleCategory::Amplifier) |
                    Some(ModuleCategory::Mixer) => {
                        // Remove from modular graph
                        self.handle.send(EngineCommand::RemoveModule { id: module_id });
                    }
                    _ => {}
                }
            }
            
            // Handle new connections - now synced with engine
            for connection in result.connections_to_add {
                self.rack_view.add_connection(connection.clone());
                
                // Send Connect command to engine
                self.handle.send(EngineCommand::Connect {
                    from: PortId::new(connection.from_module, connection.from_port.clone()),
                    to: PortId::new(connection.to_module, connection.to_port.clone()),
                });
            }
        });
        
        // Dialogs
        self.show_dialogs(ctx);
    }
    
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Stop audio
        if let Some(ref mut host) = self.host {
            let _ = host.stop();
        }
    }
}

impl SynthApp {
    /// Add a new module of the given category.
    /// 
    /// Creates the module in the GUI thread and sends it via AddModuleInstance
    /// for real-time safe addition to the audio engine.
    fn add_module_of_category(&mut self, category: ModuleCategory) {
        // Create module in GUI thread (real-time safe allocation)
        let (module, descriptor, module_type): (Box<dyn crate::modules::VoiceModule>, _, TypedModuleType) = match category {
            ModuleCategory::Oscillator => {
                let m = Oscillator::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Oscillator)
            }
            ModuleCategory::Filter => {
                let m = Filter::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Filter)
            }
            ModuleCategory::Envelope => {
                let m = Envelope::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Envelope)
            }
            ModuleCategory::LFO => {
                let m = Lfo::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Lfo)
            }
            ModuleCategory::Amplifier => {
                let m = Amplifier::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Amplifier)
            }
            ModuleCategory::Mixer => {
                let m = Mixer::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Mixer)
            }
            _ => return, // Effects handled separately
        };

        let next_id = self.next_module_id(module_type);
        self.rack_view.add_module(next_id, descriptor);

        // Send pre-created module to engine (real-time safe - just moves a pointer)
        self.handle.send(EngineCommand::AddModuleInstance {
            id: next_id,
            module,
        });
    }

    fn add_math_oscillator_module(&mut self) {
        let m = MathOscillator::new();
        let descriptor = m.descriptor();
        let module: Box<dyn crate::modules::VoiceModule> = Box::new(m);

        let next_id = self.next_module_id(TypedModuleType::MathOscillator);
        self.rack_view.add_module(next_id, descriptor);

        self.handle.send(EngineCommand::AddModuleInstance {
            id: next_id,
            module,
        });
    }

    fn add_effect_module(&mut self, effect_type: EffectType) {
        // Create effect in GUI thread (real-time safe allocation)
        let (effect, descriptor, module_type): (Box<dyn crate::modules::EffectModule>, _, TypedModuleType) = match effect_type {
            EffectType::Delay => {
                let e = Delay::new();
                let d = e.descriptor();
                (Box::new(e), d, TypedModuleType::Delay)
            }
            EffectType::Reverb => {
                let e = Reverb::new();
                let d = e.descriptor();
                (Box::new(e), d, TypedModuleType::Reverb)
            }
            EffectType::Distortion => {
                let e = Distortion::new();
                let d = e.descriptor();
                (Box::new(e), d, TypedModuleType::Distortion)
            }
            EffectType::Chorus => {
                let e = Chorus::new();
                let d = e.descriptor();
                (Box::new(e), d, TypedModuleType::Chorus)
            }
            EffectType::Phaser => {
                let e = Phaser::new();
                let d = e.descriptor();
                (Box::new(e), d, TypedModuleType::Phaser)
            }
            EffectType::Flanger => {
                let e = Flanger::new();
                let d = e.descriptor();
                (Box::new(e), d, TypedModuleType::Flanger)
            }
            EffectType::Compressor => {
                let e = Compressor::new();
                let d = e.descriptor();
                (Box::new(e), d, TypedModuleType::Compressor)
            }
            EffectType::Eq => {
                let e = Eq::new();
                let d = e.descriptor();
                (Box::new(e), d, TypedModuleType::Eq)
            }
        };

        let next_id = self.next_module_id(module_type);
        self.rack_view.add_module(next_id, descriptor);
        
        // Send pre-created effect to engine (real-time safe - just moves a pointer)
        self.handle.send(EngineCommand::AddEffectInstance {
            id: next_id,
            effect,
        });
    }
    
    fn add_visualizer_module(&mut self, viz_type: VisualizerType) {
        let (descriptor, module_type) = match viz_type {
            VisualizerType::Oscilloscope => (Oscilloscope::new().descriptor(), TypedModuleType::Oscilloscope),
            VisualizerType::LevelMeter => (LevelMeter::new().descriptor(), TypedModuleType::LevelMeter),
        };

        let next_id = self.next_module_id(module_type);
        self.rack_view.add_module(next_id, descriptor);

        // Create shared visualization buffer wrapped in Arc
        let buffer = std::sync::Arc::new(crate::visualizers::VisualizationBuffer::new(4096));

        // Store Arc clone in our handle for GUI access (same buffer!)
        self.handle.add_visualization_buffer(next_id, buffer.clone());

        // Convert GUI VisualizerType to engine VisualizerType
        let engine_viz_type = match viz_type {
            VisualizerType::Oscilloscope => crate::engine::commands::VisualizerType::Oscilloscope,
            VisualizerType::LevelMeter => crate::engine::commands::VisualizerType::LevelMeter,
        };

        // Send command to engine with the shared Arc buffer
        self.handle.send(EngineCommand::AddVisualizer {
            id: next_id,
            visualizer_type: engine_viz_type,
            buffer,
        });
    }

    fn add_stereo_output_module(&mut self) {
        let output = StereoOutput::new();
        let descriptor = output.descriptor();
        let next_id = self.next_module_id(TypedModuleType::StereoOutput);
        self.rack_view.add_module(next_id, descriptor);

        // Send pre-created module to engine (real-time safe - just moves a pointer)
        self.handle.send(EngineCommand::AddModuleInstance {
            id: next_id,
            module: Box::new(output),
        });
    }

    fn draw_meters(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("OUTPUT").color(colors::TEXT_DIM).small());
            ui.add_space(8.0);
            
            let (peak_l, peak_r) = self.handle.peak_meters();
            let (rms_l, rms_r) = self.handle.rms_meters();
            
            ui.horizontal(|ui| {
                // Left channel
                ui.vertical(|ui| {
                    ui.label(RichText::new("L").color(colors::TEXT_DIM).small());
                    draw_meter(ui, peak_l, rms_l, 16.0, 150.0);
                    ui.label(RichText::new(format!("{:.1}", 20.0 * peak_l.max(0.0001).log10()))
                        .color(colors::TEXT_DIM).small());
                });
                
                // Right channel
                ui.vertical(|ui| {
                    ui.label(RichText::new("R").color(colors::TEXT_DIM).small());
                    draw_meter(ui, peak_r, rms_r, 16.0, 150.0);
                    ui.label(RichText::new(format!("{:.1}", 20.0 * peak_r.max(0.0001).log10()))
                        .color(colors::TEXT_DIM).small());
                });
            });
        });
    }
    
    fn draw_keyboard(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Panic button (moved here since keyboard handles its own header)
            if ui.add(egui::Button::new(RichText::new("PANIC").color(colors::ACCENT_RED))).clicked() {
                self.handle.send(EngineCommand::Reset);
                self.pressed_keys.clear();
                self.keyboard.clear_pressed();
            }
        });

        // Sync pressed keys to keyboard for visual feedback
        for (&note, &pressed) in &self.pressed_keys {
            self.keyboard.set_note_pressed(note, pressed);
        }

        // Show the 88-key piano keyboard
        let event = self.keyboard.show(ui);

        // Handle note events from mouse interaction
        if let Some(note) = event.note_on {
            self.handle.note_on(note, 0.8);
        }
        for note in event.note_off {
            self.handle.note_off(note);
            self.keyboard.set_note_pressed(note, false);
        }
    }
    
    fn handle_keyboard_input(&mut self, ctx: &egui::Context) {
        let key_map: &[(egui::Key, u8)] = &[
            // Lower row: Z-M = C3-B3
            (egui::Key::Z, 48), (egui::Key::S, 49), (egui::Key::X, 50),
            (egui::Key::D, 51), (egui::Key::C, 52), (egui::Key::V, 53),
            (egui::Key::G, 54), (egui::Key::B, 55), (egui::Key::H, 56),
            (egui::Key::N, 57), (egui::Key::J, 58), (egui::Key::M, 59),
            // Upper row: Q-P = C4-E5
            (egui::Key::Q, 60), (egui::Key::Num2, 61), (egui::Key::W, 62),
            (egui::Key::Num3, 63), (egui::Key::E, 64), (egui::Key::R, 65),
            (egui::Key::Num5, 66), (egui::Key::T, 67), (egui::Key::Num6, 68),
            (egui::Key::Y, 69), (egui::Key::Num7, 70), (egui::Key::U, 71),
            (egui::Key::I, 72),
        ];

        let octave_offset = self.keyboard.octave_offset();

        ctx.input(|input| {
            for (key, base_note) in key_map {
                let note_i32 = *base_note as i32 + octave_offset * 12;
                if note_i32 < 0 || note_i32 > 127 {
                    continue; // Skip invalid notes
                }
                let note = note_i32 as u8;

                if input.key_pressed(*key) && !self.pressed_keys.get(&note).copied().unwrap_or(false) {
                    self.handle.note_on(note, 0.8);
                    self.pressed_keys.insert(note, true);
                    self.keyboard.set_note_pressed(note, true);
                }

                if input.key_released(*key) {
                    self.handle.note_off(note);
                    self.pressed_keys.insert(note, false);
                    self.keyboard.set_note_pressed(note, false);
                }
            }

            // Octave shift via keyboard
            if input.key_pressed(egui::Key::Minus) && octave_offset > -2 {
                self.keyboard.set_octave_offset(octave_offset - 1);
            }
            if input.key_pressed(egui::Key::Plus) && octave_offset < 4 {
                self.keyboard.set_octave_offset(octave_offset + 1);
            }
        });
    }
    
    fn show_dialogs(&mut self, ctx: &egui::Context) {
        // Settings dialog
        if self.show_settings {
            egui::Window::new("Settings")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("Audio settings coming soon...");
                    ui.add_space(16.0);
                    if ui.button("Close").clicked() {
                        self.show_settings = false;
                    }
                });
        }
        
        // About dialog
        if self.show_about {
            egui::Window::new("About")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.heading("Modular Synthesizer");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.add_space(8.0);
                    ui.label("A modular synthesizer written in Rust.");
                    ui.add_space(8.0);
                    ui.label("Keyboard controls:");
                    ui.label("  Z-M: Lower octave");
                    ui.label("  Q-I: Upper octave");
                    ui.label("  -/+: Change octave");
                    ui.add_space(16.0);
                    if ui.button("Close").clicked() {
                        self.show_about = false;
                    }
                });
        }
        
        // Load patch dialog
        if self.show_load_patch {
            egui::Window::new("Load Patch")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Select a patch to load:");
                    ui.add_space(8.0);
                    
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        ui.heading("Built-in Patches");
                        for patch in example_patches() {
                            ui.horizontal(|ui| {
                                if ui.button(&patch.name).clicked() {
                                    self.load_patch_data(&patch);
                                    self.current_patch_name = patch.name.clone();
                                    self.show_load_patch = false;
                                    self.status_message = Some((
                                        format!("Loaded: {}", patch.name),
                                        std::time::Instant::now()
                                    ));
                                }
                                if let Some(ref desc) = patch.description {
                                    ui.label(RichText::new(desc).small().color(colors::TEXT_DIM));
                                }
                            });
                        }
                    });
                    
                    ui.add_space(16.0);
                    if ui.button("Cancel").clicked() {
                        self.show_load_patch = false;
                    }
                });
        }
        
        // Save patch dialog
        if self.show_save_patch {
            egui::Window::new("Save Patch")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Enter a name for your patch:");
                    ui.add_space(8.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.patch_save_name);
                    });
                    
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() && !self.patch_save_name.is_empty() {
                            if let Some(patch) = self.create_patch_from_rack() {
                                let filename = format!("{}.json", 
                                    self.patch_save_name.to_lowercase().replace(' ', "_"));
                                if let Err(e) = patch.save(&filename) {
                                    self.status_message = Some((
                                        format!("Error saving: {}", e),
                                        std::time::Instant::now()
                                    ));
                                } else {
                                    self.current_patch_name = self.patch_save_name.clone();
                                    self.current_patch_path = Some(PathBuf::from(&filename));
                                    self.status_message = Some((
                                        format!("Saved: {}", filename),
                                        std::time::Instant::now()
                                    ));
                                }
                            }
                            self.show_save_patch = false;
                        }
                        if ui.button("Cancel").clicked() {
                            self.show_save_patch = false;
                        }
                    });
                });
        }
        
        // Status message toast
        if let Some((ref msg, instant)) = self.status_message {
            if instant.elapsed().as_secs() < 3 {
                egui::Window::new("Status")
                    .collapsible(false)
                    .resizable(false)
                    .title_bar(false)
                    .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -50.0])
                    .show(ctx, |ui| {
                        ui.label(RichText::new(msg).color(colors::ACCENT_GREEN));
                    });
            } else {
                self.status_message = None;
            }
        }
    }
    
    /// Load a patch into the rack view.
    fn load_patch_data(&mut self, patch: &Patch) {
        // Clear existing modules and connections
        self.rack_view.clear();
        self.instance_counters.clear();
        self.handle.visualization_buffers.clear();

        // Clear engine state
        self.handle.send(EngineCommand::ClearAllModules);

        // Add modules from patch
        for module_state in &patch.modules {
            // Parse module ID from patch file (e.g., "osc-1" -> ModuleId)
            let module_id: ModuleId = match module_state.id.parse() {
                Ok(id) => id,
                Err(_) => continue, // Skip invalid IDs
            };

            // Update instance counter to track highest instance number
            let counter = self.instance_counters.entry(module_id.module_type).or_insert(0);
            if module_id.instance > *counter {
                *counter = module_id.instance;
            }

            // Create module descriptor and instance based on type
            match module_state.module_type {
                PatchModuleType::Oscillator => {
                    let m = Oscillator::new();
                    let descriptor = m.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddModuleInstance {
                        id: module_id,
                        module: Box::new(m),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters, None);
                }
                PatchModuleType::MathOscillator => {
                    let m = MathOscillator::new();
                    let descriptor = m.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddModuleInstance {
                        id: module_id,
                        module: Box::new(m),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters, None);
                }
                PatchModuleType::Filter => {
                    let m = Filter::new();
                    let descriptor = m.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddModuleInstance {
                        id: module_id,
                        module: Box::new(m),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters, None);
                }
                PatchModuleType::Envelope => {
                    let m = Envelope::new();
                    let descriptor = m.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddModuleInstance {
                        id: module_id,
                        module: Box::new(m),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters, None);
                }
                PatchModuleType::Lfo => {
                    let m = Lfo::new();
                    let descriptor = m.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddModuleInstance {
                        id: module_id,
                        module: Box::new(m),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters, None);
                }
                PatchModuleType::Amplifier => {
                    let m = Amplifier::new();
                    let descriptor = m.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddModuleInstance {
                        id: module_id,
                        module: Box::new(m),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters, None);
                }
                PatchModuleType::Mixer => {
                    let m = Mixer::new();
                    let descriptor = m.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddModuleInstance {
                        id: module_id,
                        module: Box::new(m),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters, None);
                }
                PatchModuleType::StereoOutput => {
                    let m = StereoOutput::new();
                    let descriptor = m.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddModuleInstance {
                        id: module_id,
                        module: Box::new(m),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters, None);
                }
                PatchModuleType::Delay => {
                    let e = Delay::new();
                    let descriptor = e.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddEffectInstance {
                        id: module_id,
                        effect: Box::new(e),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters,
                        Some(EffectType::Delay));
                }
                PatchModuleType::Reverb => {
                    let e = Reverb::new();
                    let descriptor = e.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddEffectInstance {
                        id: module_id,
                        effect: Box::new(e),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters,
                        Some(EffectType::Reverb));
                }
                PatchModuleType::Distortion => {
                    let e = Distortion::new();
                    let descriptor = e.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddEffectInstance {
                        id: module_id,
                        effect: Box::new(e),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters,
                        Some(EffectType::Distortion));
                }
                PatchModuleType::Chorus => {
                    let e = Chorus::new();
                    let descriptor = e.descriptor();
                    self.rack_view.add_module_at(module_id, descriptor.clone(),
                        Pos2::new(module_state.position.0, module_state.position.1));
                    self.handle.send(EngineCommand::AddEffectInstance {
                        id: module_id,
                        effect: Box::new(e),
                    });
                    self.apply_module_parameters(module_id, &descriptor, &module_state.parameters,
                        Some(EffectType::Chorus));
                }
                PatchModuleType::Oscilloscope => {
                    let descriptor = Oscilloscope::new().descriptor();
                    self.rack_view.add_module_at(module_id, descriptor,
                        Pos2::new(module_state.position.0, module_state.position.1));

                    // Create shared visualization buffer
                    let buffer = std::sync::Arc::new(crate::visualizers::VisualizationBuffer::new(4096));
                    self.handle.add_visualization_buffer(module_id, buffer.clone());

                    self.handle.send(EngineCommand::AddVisualizer {
                        id: module_id,
                        visualizer_type: crate::engine::commands::VisualizerType::Oscilloscope,
                        buffer,
                    });
                }
                PatchModuleType::LevelMeter => {
                    let descriptor = LevelMeter::new().descriptor();
                    self.rack_view.add_module_at(module_id, descriptor,
                        Pos2::new(module_state.position.0, module_state.position.1));

                    // Create shared visualization buffer
                    let buffer = std::sync::Arc::new(crate::visualizers::VisualizationBuffer::new(4096));
                    self.handle.add_visualization_buffer(module_id, buffer.clone());

                    self.handle.send(EngineCommand::AddVisualizer {
                        id: module_id,
                        visualizer_type: crate::engine::commands::VisualizerType::LevelMeter,
                        buffer,
                    });
                }
            }
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
                self.rack_view.add_connection(connection);

                // Send connection to engine
                self.handle.send(EngineCommand::Connect {
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
        self.keyboard.set_octave_offset(patch.settings.octave_offset);
        self.glide_time = patch.settings.glide_time;
        self.handle.send(EngineCommand::SetMasterVolume(patch.settings.master_volume));
        self.handle.send(EngineCommand::SetGlideTime(patch.settings.glide_time));
    }

    /// Helper to apply parameters to a module during patch loading.
    fn apply_module_parameters(
        &mut self,
        module_id: ModuleId,
        descriptor: &ModuleDescriptor,
        parameters: &HashMap<String, crate::patch::ParamValue>,
        effect_type: Option<EffectType>,
    ) {
        for (param_name, value) in parameters {
            if let Some(typed_param) = self.find_param_by_name(descriptor, param_name) {
                // Update rack_view
                match value {
                    crate::patch::ParamValue::Float(f) => {
                        self.rack_view.set_parameter(module_id, typed_param, *f);
                    }
                    crate::patch::ParamValue::Int(i) => {
                        self.rack_view.set_parameter(module_id, typed_param, *i as f32);
                    }
                    crate::patch::ParamValue::Bool(b) => {
                        self.rack_view.set_parameter(module_id, typed_param, if *b { 1.0 } else { 0.0 });
                    }
                    crate::patch::ParamValue::Choice(s) => {
                        if let Some(param_spec) = descriptor.parameters.iter().find(|p| p.id == typed_param) {
                            if let Some(ref choices) = param_spec.choices {
                                if let Some(idx) = choices.iter().position(|c| c.id == *s) {
                                    self.rack_view.set_parameter(module_id, typed_param, idx as f32);
                                }
                            }
                        }
                    }
                }

                // Send to engine
                let typed_value = self.convert_param_value_to_typed(typed_param, value);
                if let Some(et) = effect_type {
                    self.handle.send(EngineCommand::SetEffectParameter {
                        effect_type: et,
                        param: typed_param,
                        value: typed_value,
                    });
                } else if let Some(voice_module) = self.get_voice_module_for_typed_param(module_id, typed_param) {
                    self.handle.send(EngineCommand::SetVoiceParameter {
                        target: voice_module,
                        param: typed_param,
                        value: typed_value,
                    });
                }
            }
        }
    }
    
    /// Create a patch from current rack state.
    fn create_patch_from_rack(&self) -> Option<Patch> {
        let mut patch = Patch::new(&self.patch_save_name);
        patch.author = Some("User".to_string());

        // Add modules
        for module_id in self.rack_view.module_ids() {
            if let Some((descriptor, position, params)) = self.rack_view.get_module_data(module_id) {
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
                    if let Some(name) = self.typed_param_to_name(typed_param) {
                        param_map.insert(name, crate::patch::ParamValue::Float(value));
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
        for conn in self.rack_view.connections() {
            patch.connections.push(ConnectionState {
                from: (conn.from_module.to_string(), conn.from_port.clone()),
                to: (conn.to_module.to_string(), conn.to_port.clone()),
            });
        }
        
        patch.settings.octave_offset = self.keyboard.octave_offset();
        patch.settings.master_volume = self.handle.master_volume();
        patch.settings.glide_time = self.glide_time;

        Some(patch)
    }
    
    /// Find TypedParam by name from a module descriptor.
    fn find_param_by_name(&self, descriptor: &ModuleDescriptor, name: &str) -> Option<TypedParam> {
        // Normalize name for comparison
        let normalized = name.to_lowercase();
        descriptor.parameters.iter()
            .find(|p| p.name.to_lowercase() == normalized || p.id.name().to_lowercase() == normalized)
            .map(|p| p.id)
    }

    /// Convert TypedParam to name for serialization.
    fn typed_param_to_name(&self, param: TypedParam) -> Option<String> {
        Some(param.name().to_lowercase())
    }

    /// Convert ParamValue to TypedValue based on the TypedParam type.
    fn convert_param_value_to_typed(&self, param: TypedParam, value: &crate::patch::ParamValue) -> TypedValue {
        match value {
            crate::patch::ParamValue::Float(f) => TypedValue::Float(*f),
            crate::patch::ParamValue::Int(i) => TypedValue::Int(*i),
            crate::patch::ParamValue::Bool(b) => TypedValue::Bool(*b),
            crate::patch::ParamValue::Choice(s) => {
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
    fn get_voice_module_for_typed_param(&self, module_id: ModuleId, param: TypedParam) -> Option<VoiceModule> {
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
    fn get_effect_type_from_module(&self, module_id: ModuleId) -> Option<EffectType> {
        let desc = self.rack_view.module_descriptor(module_id)?;
        
        // Match based on type_id
        match desc.type_id.0.as_str() {
            "chorus" => Some(EffectType::Chorus),
            "delay" => Some(EffectType::Delay),
            "reverb" => Some(EffectType::Reverb),
            "distortion" => Some(EffectType::Distortion),
            _ => None,
        }
    }
}

/// Draw a vertical audio meter.
fn draw_meter(ui: &mut egui::Ui, peak: f32, rms: f32, width: f32, height: f32) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter();
    
    // Background
    painter.rect_filled(rect, 2.0, colors::BG_DARK);
    
    // RMS level
    let rms_height = rect.height() * rms.clamp(0.0, 1.0);
    let rms_rect = egui::Rect::from_min_size(
        Pos2::new(rect.left(), rect.bottom() - rms_height),
        Vec2::new(rect.width(), rms_height),
    );
    
    let rms_color = if rms > 0.9 {
        colors::METER_RED
    } else if rms > 0.7 {
        colors::METER_YELLOW
    } else {
        colors::METER_GREEN
    };
    
    painter.rect_filled(rms_rect, 0.0, rms_color);
    
    // Peak indicator
    let peak_y = rect.bottom() - rect.height() * peak.clamp(0.0, 1.0);
    painter.line_segment(
        [Pos2::new(rect.left(), peak_y), Pos2::new(rect.right(), peak_y)],
        Stroke::new(2.0, colors::TEXT_PRIMARY),
    );
}
