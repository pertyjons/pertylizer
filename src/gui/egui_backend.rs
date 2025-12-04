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
use crate::engine::{EngineCommand, EngineEvent, EngineHandle, SynthEngine, ModuleId};
use crate::engine::commands::PortId;
use crate::engine::ModuleType as TypedModuleType;
use crate::io::MidiHandler;
use crate::gui::{GuiBackend, GuiResult, SynthGuiConfig};
use crate::gui::dialogs::{DialogState, LoadPatchResult, SavePatchResult, show_settings_dialog, show_about_dialog, show_load_patch_dialog, show_save_patch_dialog, show_status_toast};
use crate::gui::widgets::colors;
use crate::gui::keyboard::PianoKeyboard;
use crate::gui::patch_editor::{PatchEditor, ModulePalette, PaletteSelection, EffectType, VisualizerType};
use crate::gui::patch_bridge;
use crate::gui::instrument_rack::{InstrumentUiState, show_instrument_rack};
use crate::engine::instrument::{InstrumentId, MidiChannel};
use crate::types::NormalizedValue;
use crate::modules::{
    Describable, ModuleCategory,
    Oscillator, MathOscillator, SubOscillator, NoiseGenerator,
    Filter, Envelope, Lfo, Amplifier, Mixer, StereoOutput,
};
use crate::effects::{Chorus, Compressor, Delay, Distortion, Eq, Flanger, Phaser, Reverb};
use crate::visualizers::{Oscilloscope, LevelMeter};
use crate::patch::{Patch, example_patches};

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
                setup_custom_fonts(&cc.egui_ctx);
                setup_custom_style(&cc.egui_ctx);
                Ok(Box::new(app))
            }),
        ).map_err(|e| format!("Egui error: {e}"))?;

        Ok(())
    }
}

/// Setup custom fonts for retro-digital aesthetic.
fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Install Share Tech Mono - a retro-digital monospace font
    fonts.font_data.insert(
        "ShareTechMono".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/ShareTechMono-Regular.ttf"
        ))),
    );

    // Set as highest priority for Proportional (UI text)
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "ShareTechMono".to_owned());

    // Set as highest priority for Monospace (values/code)
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "ShareTechMono".to_owned());

    ctx.set_fonts(fonts);
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

    // MIDI input handler
    midi_handler: MidiHandler,

    // Module ID generation - track instance counts per module type
    instance_counters: HashMap<TypedModuleType, u16>,

    // Keyboard state
    keyboard: PianoKeyboard,
    pressed_keys: HashMap<u8, bool>, // For computer keyboard tracking

    // Dialog state (centralized in dialogs module)
    dialog_state: DialogState,
    show_add_module: bool,

    // Patch state
    current_patch_name: String,
    current_patch_path: Option<PathBuf>,

    // Global synth settings
    glide_time: f32,

    // Instrument rack state
    instruments: Vec<InstrumentUiState>,
    active_instrument_id: InstrumentId,
    next_instrument_id: u64,
}

impl SynthApp {
    fn new(
        mut handle: EngineHandle,
        host: Box<dyn AudioHostTrait>,
        config: SynthGuiConfig,
        latency: std::time::Duration,
    ) -> Self {
        // IMPORTANT: We use a startup patch instead of manually building GUI state.
        // This ensures that GUI and Engine are 100% synchronized from the first millisecond.
        //
        // Previously, the GUI built its own visual representation while the engine had
        // a hardcoded "Spacey Bass" template internally. This caused "ghost sounds" when
        // modules were removed from the GUI - the engine's hardcoded voices kept playing.
        //
        // By loading a patch at startup, we:
        // 1. Clear the engine completely (ClearAllModules)
        // 2. Send commands to create each module
        // 3. Send commands to connect them
        // 4. Update the GUI to match
        //
        // This guarantees GUI and Engine have exactly the same state.

        let mut instance_counters = HashMap::new();
        let mut keyboard = PianoKeyboard::new();
        let mut glide_time = 0.0;

        // Initialize instruments with a default instrument that matches the engine's default
        // The engine starts with one instrument on CH1 (InstrumentId::FIRST)
        // Each instrument owns its own PatchEditor for independent visual graphs.
        let mut default_instrument = InstrumentUiState::default();
        let active_instrument_id = InstrumentId::FIRST;
        let next_instrument_id = 1; // Start at 1 since FIRST (0) is already used

        // Create and load the startup patch into the default instrument
        // Uses send_blocking to ensure commands aren't dropped during startup
        let startup_patch = crate::patches::patch_spacey_bass();
        let patch_name = startup_patch.name.clone();

        patch_bridge::load_patch(
            &startup_patch,
            &mut default_instrument.patch_editor,
            &mut instance_counters,
            &mut handle,
            &mut keyboard,
            &mut glide_time,
            active_instrument_id,
        );

        let instruments = vec![default_instrument];

        // Initialize MIDI input (connects to first available port)
        // The MidiHandler gets a clone of the command sender, so both GUI and MIDI
        // can send commands to the engine.
        let midi_handler = MidiHandler::new(handle.command_sender());

        Self {
            handle,
            host: Some(host),
            config,
            latency,
            midi_handler,
            instance_counters,
            keyboard,
            pressed_keys: HashMap::new(),
            dialog_state: DialogState::new(),
            show_add_module: false,
            current_patch_name: patch_name,
            current_patch_path: None,
            glide_time,
            instruments,
            active_instrument_id,
            next_instrument_id,
        }
    }

    /// Get the next ModuleId for a given module type.
    fn next_module_id(&mut self, module_type: TypedModuleType) -> ModuleId {
        let counter = self.instance_counters.entry(module_type).or_insert(0);
        *counter += 1;
        ModuleId::new(module_type, *counter)
    }

    /// Get the active instrument's patch editor.
    fn active_patch_editor(&mut self) -> &mut PatchEditor {
        self.instruments.iter_mut()
            .find(|i| i.id == self.active_instrument_id)
            .map(|i| &mut i.patch_editor)
            .expect("Active instrument not found")
    }

    /// Get the active instrument's patch editor (immutable).
    fn active_patch_editor_ref(&self) -> &PatchEditor {
        self.instruments.iter()
            .find(|i| i.id == self.active_instrument_id)
            .map(|i| &i.patch_editor)
            .expect("Active instrument not found")
    }
}

impl eframe::App for SynthApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Clean up any modules returned from audio thread (dropped on main thread)
        self.handle.cleanup_dropped_modules();

        // Poll for engine events (note feedback, etc.)
        // This ensures the GUI keyboard reflects what the engine is actually playing,
        // regardless of whether notes came from MIDI, sequencer, or GUI.
        while let Some(event) = self.handle.poll_event() {
            match event {
                EngineEvent::NoteTriggered { note, velocity, .. } => {
                    self.keyboard.set_note_on(note, velocity);
                }
                EngineEvent::NoteReleased { note, .. } => {
                    self.keyboard.set_note_off(note);
                    self.pressed_keys.remove(&note);
                }
                EngineEvent::AllNotesReleased => {
                    self.keyboard.clear_pressed();
                    self.pressed_keys.clear();
                }
                // Other events (meters, etc.) are handled elsewhere
                _ => {}
            }
        }

        // Handle keyboard input
        self.handle_keyboard_input(ctx);

        // Request continuous repaint for meters
        ctx.request_repaint();
        
        // Top menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("📄 New Patch").clicked() {
                        self.reset_to_new_patch();
                        self.dialog_state.set_status("New patch created".to_string());
                        ui.close();
                    }
                    if ui.button("📂 Load Patch...").clicked() {
                        self.dialog_state.show_load_patch = true;
                        ui.close();
                    }
                    if ui.button("💾 Save Patch...").clicked() {
                        self.dialog_state.patch_save_name = self.current_patch_name.clone();
                        self.dialog_state.show_save_patch = true;
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("📋 Example Patches", |ui| {
                        for patch in example_patches() {
                            if ui.button(&patch.name).clicked() {
                                self.load_patch_data(&patch);
                                self.current_patch_name = patch.name.clone();
                                self.dialog_state.set_status(format!("Loaded: {}", patch.name));
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("⚙ Settings...").clicked() {
                        self.dialog_state.show_settings = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("🚪 Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        self.dialog_state.show_about = true;
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
                    // MIDI port selector (styled as button with dropdown indicator)
                    let midi_label = if self.midi_handler.is_connected() {
                        let port_name = self.midi_handler.port_name().unwrap_or("Unknown");
                        // Shorten long port names for display
                        let short_name = if port_name.len() > 20 {
                            format!("{}...", &port_name[..17])
                        } else {
                            port_name.to_string()
                        };
                        RichText::new(format!("🎹 {} ▼", short_name)).color(colors::METER_GREEN)
                    } else {
                        RichText::new("🎹 MIDI ▼").color(colors::TEXT_DIM)
                    };

                    ui.menu_button(midi_label, |ui| {
                        ui.set_min_width(250.0);
                        let ports = MidiHandler::list_ports();
                        if ports.is_empty() {
                            ui.label(RichText::new("No MIDI ports available").color(colors::TEXT_DIM));
                        } else {
                            for port in &ports {
                                let is_current = self.midi_handler.port_name() == Some(port.as_str());
                                let label = if is_current {
                                    RichText::new(format!("● {}", port)).color(colors::METER_GREEN)
                                } else {
                                    RichText::new(format!("  {}", port))
                                };
                                if ui.button(label).clicked() {
                                    if let Err(e) = self.midi_handler.connect_to(port) {
                                        eprintln!("MIDI connection error: {}", e);
                                    }
                                    ui.close();
                                }
                            }
                        }
                        ui.separator();
                        if ui.button("🔄 Refresh").clicked() {
                            ui.close();
                        }
                    });
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
                        PaletteSelection::SubOscillator => {
                            self.add_sub_oscillator_module();
                        }
                        PaletteSelection::Noise => {
                            self.add_noise_module();
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
                    self.handle.send(EngineCommand::SetGlideTime(
                        crate::types::Seconds::new(self.glide_time)
                    ));
                }

                ui.separator();

                // Connection info (from active instrument's patch editor)
                let conn_count = self.active_patch_editor_ref().connections().len();
                let module_count = self.active_patch_editor_ref().module_ids().len();
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

        // Left side panel with instrument rack
        egui::SidePanel::left("instrument_rack_panel")
            .min_width(280.0)
            .max_width(350.0)
            .show(ctx, |ui| {
                show_instrument_rack(
                    ui,
                    &mut self.instruments,
                    &mut self.active_instrument_id,
                    &mut self.handle,
                    &mut self.next_instrument_id,
                );
            });

        // Right side panel with meters
        egui::SidePanel::right("meters_panel")
            .min_width(80.0)
            .show(ctx, |ui| {
                self.draw_meters(ui);
            });

        // Main content - show the active instrument's patch editor
        // Get active instrument id for engine commands
        let active_id = self.active_instrument_id;

        egui::CentralPanel::default().show(ctx, |ui| {
            // Get the active instrument's patch editor
            let patch_editor = self.instruments.iter_mut()
                .find(|i| i.id == active_id)
                .map(|i| &mut i.patch_editor)
                .expect("Active instrument not found");

            let result = patch_editor.show(ui, &self.handle);

            // Handle parameter changes - send Param directly (carries its own value)
            for (module_id, param) in result.param_changes {
                // Check module category
                let category = patch_editor.module_descriptor(module_id)
                    .map(|d| d.category);

                match category {
                    Some(ModuleCategory::Effect) => {
                        // Effect module - use SetEffectParameter (targets active instrument)
                        if let Some(effect_type) = patch_bridge::get_effect_type_from_module(patch_editor, module_id) {
                            self.handle.send(EngineCommand::SetEffectParameter {
                                instrument_id: Some(active_id),
                                effect_type,
                                param,
                            });
                        }
                    }
                    Some(ModuleCategory::Oscillator) |
                    Some(ModuleCategory::Filter) |
                    Some(ModuleCategory::Envelope) |
                    Some(ModuleCategory::LFO) |
                    Some(ModuleCategory::Amplifier) |
                    Some(ModuleCategory::Mixer) |
                    Some(ModuleCategory::Output) => {
                        // Voice/modular module - send to active instrument's voice graph
                        self.handle.send(EngineCommand::SetModuleParameter {
                            instrument_id: Some(active_id),
                            module_id,
                            param: param.clone(),
                        });

                        // Also send to voice modules for real-time voice param updates
                        if let Some(voice_module) = patch_bridge::get_voice_module_for_param(module_id, &param) {
                            self.handle.send(EngineCommand::SetVoiceParameter {
                                instrument_id: active_id,
                                target: voice_module,
                                param,
                            });
                        }
                    }
                    _ => {}
                }
            }

            // Handle module removal
            for module_id in result.modules_to_remove {
                // Get module descriptor to determine type
                let category = patch_editor.module_descriptor(module_id)
                    .map(|d| d.category);

                patch_editor.remove_module(module_id);

                // Send appropriate remove command to engine based on category
                match category {
                    Some(ModuleCategory::Visualizer) => {
                        self.handle.send(EngineCommand::RemoveVisualizer {
                            instrument_id: Some(active_id),
                            id: module_id,
                        });
                        self.handle.remove_visualization_buffer(module_id);
                    }
                    Some(ModuleCategory::Effect) => {
                        self.handle.send(EngineCommand::RemoveEffect {
                            instrument_id: Some(active_id),
                            id: module_id,
                        });
                    }
                    Some(ModuleCategory::Oscillator) |
                    Some(ModuleCategory::Filter) |
                    Some(ModuleCategory::Envelope) |
                    Some(ModuleCategory::LFO) |
                    Some(ModuleCategory::Amplifier) |
                    Some(ModuleCategory::Mixer) |
                    Some(ModuleCategory::Output) => {
                        // Remove from active instrument's voice graph
                        self.handle.send(EngineCommand::RemoveModule {
                            instrument_id: Some(active_id),
                            id: module_id,
                        });
                    }
                    _ => {}
                }
            }

            // Handle new connections - now synced with engine
            for connection in result.connections_to_add {
                patch_editor.add_connection(connection.clone());

                // Send Connect command to engine (active instrument's voice graph)
                self.handle.send(EngineCommand::Connect {
                    instrument_id: Some(active_id),
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
        let (module, descriptor, module_type): (Box<dyn crate::modules::PolyModule>, _, TypedModuleType) = match category {
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
        self.active_patch_editor().add_module(next_id, descriptor);

        // Send pre-created module to engine (active instrument's voice graph)
        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    fn add_math_oscillator_module(&mut self) {
        let m = MathOscillator::new();
        let descriptor = m.descriptor();
        let module: Box<dyn crate::modules::PolyModule> = Box::new(m);

        let next_id = self.next_module_id(TypedModuleType::MathOscillator);
        self.active_patch_editor().add_module(next_id, descriptor);

        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    fn add_sub_oscillator_module(&mut self) {
        let m = SubOscillator::new();
        let descriptor = m.descriptor();
        let module: Box<dyn crate::modules::PolyModule> = Box::new(m);

        let next_id = self.next_module_id(TypedModuleType::SubOscillator);
        self.active_patch_editor().add_module(next_id, descriptor);

        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    fn add_noise_module(&mut self) {
        let m = NoiseGenerator::new();
        let descriptor = m.descriptor();
        let module: Box<dyn crate::modules::PolyModule> = Box::new(m);

        let next_id = self.next_module_id(TypedModuleType::Noise);
        self.active_patch_editor().add_module(next_id, descriptor);

        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    fn add_effect_module(&mut self, effect_type: EffectType) {
        // Create effect in GUI thread (real-time safe allocation)
        let (effect, descriptor, module_type): (Box<dyn crate::modules::AudioEffect>, _, TypedModuleType) = match effect_type {
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
        // Effects are added to the active instrument's patch editor for visual display
        self.active_patch_editor().add_module(next_id, descriptor);

        // Send pre-created effect to active instrument's effect chain
        self.handle.send(EngineCommand::AddEffectInstance {
            instrument_id: Some(self.active_instrument_id),
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
        // Visualizers are added to the active instrument's patch editor for visual display
        self.active_patch_editor().add_module(next_id, descriptor);

        // Create shared visualization buffer wrapped in Arc
        let buffer = std::sync::Arc::new(crate::visualizers::VisualizationBuffer::new(4096));

        // Store Arc clone in our handle for GUI access (same buffer!)
        self.handle.add_visualization_buffer(next_id, buffer.clone());

        // Convert GUI VisualizerType to engine VisualizerType
        let engine_viz_type = match viz_type {
            VisualizerType::Oscilloscope => crate::engine::commands::VisualizerType::Oscilloscope,
            VisualizerType::LevelMeter => crate::engine::commands::VisualizerType::LevelMeter,
        };

        // Send command to active instrument's effect chain
        self.handle.send(EngineCommand::AddVisualizer {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            visualizer_type: engine_viz_type,
            buffer,
        });
    }

    fn add_stereo_output_module(&mut self) {
        let output = StereoOutput::new();
        let descriptor = output.descriptor();
        let next_id = self.next_module_id(TypedModuleType::StereoOutput);
        self.active_patch_editor().add_module(next_id, descriptor);

        // Send pre-created module to engine (active instrument's voice graph)
        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
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
        // Get the MIDI channel for the active instrument
        let active_channel = self.instruments.iter()
            .find(|p| p.id == self.active_instrument_id)
            .map(|p| p.channel)
            .unwrap_or(MidiChannel::CH1);

        ui.horizontal(|ui| {
            // Panic button (moved here since keyboard handles its own header)
            if ui.add(egui::Button::new(RichText::new("PANIC").color(colors::ACCENT_RED))).clicked() {
                self.handle.send(EngineCommand::AllNotesOff);
                self.pressed_keys.clear();
                // Keyboard will be cleared by AllNotesReleased event from engine
            }

            // Show active instrument indicator
            let active_name = self.instruments.iter()
                .find(|p| p.id == self.active_instrument_id)
                .map(|p| p.name.as_str())
                .unwrap_or("Instrument 1");
            ui.separator();
            ui.label(RichText::new(format!("Playing: {}", active_name)).color(colors::ACCENT_ORANGE));
        });

        // Note: Keyboard visual state is now driven by engine events (NoteTriggered/NoteReleased)
        // in the main update loop. This ensures the GUI reflects what the engine is actually
        // playing, regardless of input source (MIDI, sequencer, or GUI).

        // Show the 88-key piano keyboard
        let event = self.keyboard.show(ui);

        // Handle note events from mouse interaction - send to active instrument's channel
        if let Some(note) = event.note_on {
            self.handle.note_on_channel(note, NormalizedValue::new(0.8), active_channel);
        }
        for note in event.note_off {
            self.handle.note_off_channel(note, active_channel);
            // Note release will be reflected via NoteReleased event from engine
        }
    }
    
    fn handle_keyboard_input(&mut self, ctx: &egui::Context) {
        // Get the MIDI channel for the active instrument
        let active_channel = self.instruments.iter()
            .find(|p| p.id == self.active_instrument_id)
            .map(|p| p.channel)
            .unwrap_or(MidiChannel::CH1);

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
                    self.handle.note_on_channel(note, NormalizedValue::new(0.8), active_channel);
                    self.pressed_keys.insert(note, true);
                    // Visual feedback will come from NoteTriggered engine event
                }

                if input.key_released(*key) {
                    self.handle.note_off_channel(note, active_channel);
                    self.pressed_keys.insert(note, false);
                    // Visual feedback will come from NoteReleased event from engine
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
        // Update dialog state (clears expired status messages)
        self.dialog_state.update();

        // Settings dialog
        show_settings_dialog(ctx, &mut self.dialog_state.show_settings);

        // About dialog
        show_about_dialog(ctx, &mut self.dialog_state.show_about);

        // Load patch dialog
        match show_load_patch_dialog(ctx, &mut self.dialog_state.show_load_patch) {
            LoadPatchResult::LoadBuiltin(patch) => {
                self.load_patch_data(&patch);
                self.current_patch_name = patch.name.clone();
                self.dialog_state.set_status(format!("Loaded: {}", patch.name));
            }
            LoadPatchResult::Cancelled | LoadPatchResult::None => {}
        }

        // Save patch dialog
        match show_save_patch_dialog(ctx, &mut self.dialog_state.show_save_patch, &mut self.dialog_state.patch_save_name) {
            SavePatchResult::Save(name) => {
                if let Some(patch) = self.create_patch_from_rack() {
                    let filename = format!("{}.json", name.to_lowercase().replace(' ', "_"));
                    if let Err(e) = patch.save(&filename) {
                        self.dialog_state.set_status(format!("Error saving: {}", e));
                    } else {
                        self.current_patch_name = name;
                        self.current_patch_path = Some(PathBuf::from(&filename));
                        self.dialog_state.set_status(format!("Saved: {}", filename));
                    }
                }
            }
            SavePatchResult::Cancelled | SavePatchResult::None => {}
        }

        // Status message toast
        show_status_toast(ctx, &mut self.dialog_state);
    }
    
    /// Load a patch into the active instrument's rack view.
    fn load_patch_data(&mut self, patch: &Patch) {
        // Clear visualization buffers (not handled by patch_bridge)
        self.handle.visualization_buffers.clear();

        // Delegate to patch_bridge for the main loading logic
        // Load into the active instrument's patch editor
        let active_id = self.active_instrument_id;
        let patch_editor = self.instruments.iter_mut()
            .find(|i| i.id == active_id)
            .map(|i| &mut i.patch_editor)
            .expect("Active instrument not found");

        patch_bridge::load_patch(
            patch,
            patch_editor,
            &mut self.instance_counters,
            &mut self.handle,
            &mut self.keyboard,
            &mut self.glide_time,
            active_id,
        );
    }

    /// Reset the active instrument to a new empty patch.
    /// Clears all modules and adds a default StereoOutput for immediate sound.
    fn reset_to_new_patch(&mut self) {
        // 1. Clear active instrument's GUI state
        let active_id = self.active_instrument_id;

        // Clear all modules from the active instrument in the engine
        {
            let patch_editor = self.instruments.iter()
                .find(|i| i.id == active_id)
                .map(|i| &i.patch_editor)
                .expect("Active instrument not found");

            for module_id in patch_editor.module_ids() {
                let category = patch_editor.module_descriptor(module_id).map(|d| d.category);
                match category {
                    Some(ModuleCategory::Effect) => {
                        self.handle.send_blocking(EngineCommand::RemoveEffect {
                            instrument_id: Some(active_id),
                            id: module_id,
                        });
                    }
                    Some(ModuleCategory::Visualizer) => {
                        self.handle.send_blocking(EngineCommand::RemoveVisualizer {
                            instrument_id: Some(active_id),
                            id: module_id,
                        });
                        self.handle.remove_visualization_buffer(module_id);
                    }
                    _ => {
                        self.handle.send_blocking(EngineCommand::RemoveModule {
                            instrument_id: Some(active_id),
                            id: module_id,
                        });
                    }
                }
            }
        }

        // Clear the patch editor GUI state
        self.active_patch_editor().clear();
        self.instance_counters.clear();
        self.handle.visualization_buffers.clear();

        // 2. Reset keyboard state
        self.keyboard = PianoKeyboard::new();
        self.glide_time = 0.0;

        // 3. Add default StereoOutput so user gets sound immediately
        self.add_stereo_output_module();

        // 4. Update patch name
        self.current_patch_name = "New Patch".to_string();
    }

    /// Create a patch from current rack state.
    fn create_patch_from_rack(&self) -> Option<Patch> {
        patch_bridge::create_patch_from_rack(
            &self.dialog_state.patch_save_name,
            self.active_patch_editor_ref(),
            &self.keyboard,
            &self.handle,
            self.glide_time,
        )
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
