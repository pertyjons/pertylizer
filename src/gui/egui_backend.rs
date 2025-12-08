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
use crate::effects::{Chorus, Compressor, Delay, Distortion, Eq, Flanger, Phaser, Reverb};
use crate::engine::ModuleType as TypedModuleType;
use crate::engine::commands::PortId;
use crate::engine::instrument::{InstrumentId, MidiChannel};
use crate::engine::params::{
    ChorusParam, CompressorParam, DelayParam, DistortionParam, EqParam, FlangerParam, Param,
    PhaserParam, ReverbParam,
};
use crate::engine::{EngineCommand, EngineEvent, EngineHandle, ModuleId, SynthEngine};
use crate::gui::dialogs::{
    DialogState, LoadPatchResult, SavePatchResult, show_about_dialog, show_load_patch_dialog,
    show_save_patch_dialog, show_settings_dialog, show_status_toast,
};
use crate::gui::instrument_rack::{InstrumentUiState, show_instrument_rack};
use crate::gui::keyboard::PianoKeyboard;
use crate::gui::patch_bridge;
use crate::gui::patch_editor::{
    EffectType, ModulePalette, PaletteSelection, PatchEditor, VisualizerType,
};
use crate::gui::widgets::colors;
use crate::gui::{GuiBackend, GuiResult, SynthGuiConfig};
use crate::io::MidiHandler;
use crate::modules::{
    Amplifier, Describable, Envelope, Filter, Lfo, MathOscillator, Mixer, ModuleCategory,
    NoiseGenerator, Oscillator, StereoOutput, SubOscillator,
};
use crate::patch::{Patch, example_patches};
use crate::types::{MidiNote, NormalizedValue};
use crate::visualizers::{LevelMeter, Oscilloscope};

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
        )
        .map_err(|e| format!("Egui error: {e}"))?;

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

/// Stored parameter values for each effect type.
#[derive(Clone)]
enum MasterEffectParams {
    Compressor {
        threshold: f32, // dB: -60 to 0
        ratio: f32,     // 1:1 to 20:1
        attack: f32,    // ms: 0.1 to 100
        release: f32,   // ms: 10 to 1000
        makeup: f32,    // dB: 0 to 24
        mix: f32,       // 0-1
    },
    Eq {
        low_gain: f32,  // dB: -12 to 12
        mid_gain: f32,  // dB: -12 to 12
        high_gain: f32, // dB: -12 to 12
        mix: f32,       // 0-1
    },
    Reverb {
        room_size: f32, // 0-1
        damping: f32,   // 0-1
        width: f32,     // 0-1
        mix: f32,       // 0-1
    },
    Delay {
        time: f32,     // seconds: 0.01 to 2.0
        feedback: f32, // 0-1
        mix: f32,      // 0-1
    },
    Chorus {
        rate: f32,  // Hz: 0.1 to 5.0
        depth: f32, // 0-1
        mix: f32,   // 0-1
    },
    Phaser {
        rate: f32,     // Hz: 0.1 to 5.0
        depth: f32,    // 0-1
        feedback: f32, // -1 to 1
        mix: f32,      // 0-1
    },
    Flanger {
        rate: f32,     // Hz: 0.1 to 5.0
        depth: f32,    // 0-1
        feedback: f32, // -1 to 1
        mix: f32,      // 0-1
    },
    Distortion {
        drive: f32, // 0-1
        tone: f32,  // 0-1
        mix: f32,   // 0-1
    },
}

impl MasterEffectParams {
    fn new(effect_type: EffectType) -> Self {
        match effect_type {
            EffectType::Compressor => Self::Compressor {
                threshold: -20.0,
                ratio: 4.0,
                attack: 10.0,
                release: 100.0,
                makeup: 0.0,
                mix: 1.0,
            },
            EffectType::Eq => Self::Eq {
                low_gain: 0.0,
                mid_gain: 0.0,
                high_gain: 0.0,
                mix: 1.0,
            },
            EffectType::Reverb => Self::Reverb {
                room_size: 0.5,
                damping: 0.5,
                width: 1.0,
                mix: 0.3,
            },
            EffectType::Delay => Self::Delay {
                time: 0.25,
                feedback: 0.4,
                mix: 0.3,
            },
            EffectType::Chorus => Self::Chorus {
                rate: 1.0,
                depth: 0.5,
                mix: 0.5,
            },
            EffectType::Phaser => Self::Phaser {
                rate: 0.5,
                depth: 0.5,
                feedback: 0.3,
                mix: 0.5,
            },
            EffectType::Flanger => Self::Flanger {
                rate: 0.3,
                depth: 0.5,
                feedback: 0.3,
                mix: 0.5,
            },
            EffectType::Distortion => Self::Distortion {
                drive: 0.5,
                tone: 0.5,
                mix: 0.5,
            },
        }
    }
}

/// UI state for a master effect in the effects chain.
#[derive(Clone)]
struct MasterEffectUiState {
    /// Module ID for this effect
    id: ModuleId,
    /// Effect type
    effect_type: EffectType,
    /// Whether the panel is expanded (showing parameters)
    expanded: bool,
    /// Whether the effect is bypassed
    bypassed: bool,
    /// Current parameter values
    params: MasterEffectParams,
}

impl MasterEffectUiState {
    fn new(id: ModuleId, effect_type: EffectType) -> Self {
        Self {
            id,
            effect_type,
            expanded: true, // Start expanded so user can see parameters
            bypassed: false,
            params: MasterEffectParams::new(effect_type),
        }
    }

    /// Get a display name for this effect
    fn display_name(&self) -> &'static str {
        match self.effect_type {
            EffectType::Compressor => "Compressor",
            EffectType::Eq => "EQ",
            EffectType::Reverb => "Reverb",
            EffectType::Delay => "Delay",
            EffectType::Chorus => "Chorus",
            EffectType::Phaser => "Phaser",
            EffectType::Flanger => "Flanger",
            EffectType::Distortion => "Distortion",
        }
    }
}

/// Main application state.
struct SynthApp {
    handle: EngineHandle,
    host: Option<Box<dyn AudioHostTrait>>,
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

    // Patch state
    current_patch_name: String,
    current_patch_path: Option<PathBuf>,

    // Global synth settings
    glide_time: f32,

    // Instrument rack state
    instruments: Vec<InstrumentUiState>,
    active_instrument_id: InstrumentId,
    next_instrument_id: u64,

    // Master effects chain UI state
    master_effects: Vec<MasterEffectUiState>,
}

impl SynthApp {
    fn new(
        mut handle: EngineHandle,
        host: Box<dyn AudioHostTrait>,
        _config: SynthGuiConfig, // Used only for initial setup, not stored
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
            latency,
            midi_handler,
            instance_counters,
            keyboard,
            pressed_keys: HashMap::new(),
            dialog_state: DialogState::new(),
            current_patch_name: patch_name,
            current_patch_path: None,
            glide_time,
            instruments,
            active_instrument_id,
            next_instrument_id,
            master_effects: Vec::new(),
        }
    }

    /// Get the next ModuleId for a given module type.
    fn next_module_id(&mut self, module_type: TypedModuleType) -> ModuleId {
        let counter = self.instance_counters.entry(module_type).or_insert(0);
        *counter += 1;
        ModuleId::new(module_type, *counter)
    }

    /// Get the active instrument's patch editor.
    ///
    /// # Panics
    /// Panics if active_instrument_id doesn't match any instrument (programming error).
    #[allow(clippy::expect_used)]
    fn active_patch_editor(&mut self) -> &mut PatchEditor {
        self.instruments
            .iter_mut()
            .find(|i| i.id == self.active_instrument_id)
            .map(|i| &mut i.patch_editor)
            .expect("Active instrument not found")
    }

    /// Get the active instrument's patch editor (immutable).
    ///
    /// # Panics
    /// Panics if active_instrument_id doesn't match any instrument (programming error).
    #[allow(clippy::expect_used)]
    fn active_patch_editor_ref(&self) -> &PatchEditor {
        self.instruments
            .iter()
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
                    self.pressed_keys.remove(&note.as_u8());
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
                        self.dialog_state
                            .set_status("New patch created".to_string());
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
                                self.dialog_state
                                    .set_status(format!("Loaded: {}", patch.name));
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
                    ui.label(
                        RichText::new(format!("Voices: {}", self.handle.voice_count()))
                            .color(colors::TEXT_SECONDARY),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "Latency: {:.1}ms",
                            self.latency.as_secs_f64() * 1000.0
                        ))
                        .color(colors::TEXT_DIM),
                    );
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
                            ui.label(
                                RichText::new("No MIDI ports available").color(colors::TEXT_DIM),
                            );
                        } else {
                            for port in &ports {
                                let is_current =
                                    self.midi_handler.port_name() == Some(port.as_str());
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
                    ui.label(
                        RichText::new(format!("Patch: {}", self.current_patch_name))
                            .color(colors::ACCENT_CYAN),
                    );
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
                            if v < 0.001 {
                                "Off".to_string()
                            } else {
                                format!("{:.2}s", v)
                            }
                        }),
                );
                if glide_response.changed() {
                    self.handle
                        .send(EngineCommand::SetGlideTime(crate::types::Seconds::new(
                            self.glide_time,
                        )));
                }

                ui.separator();

                // Connection info (from active instrument's patch editor)
                let conn_count = self.active_patch_editor_ref().connections().len();
                let module_count = self.active_patch_editor_ref().module_ids().len();
                ui.label(
                    RichText::new(format!(
                        "Modules: {} | Connections: {}",
                        module_count, conn_count
                    ))
                    .color(colors::TEXT_DIM),
                );
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
            .min_width(320.0)
            .max_width(400.0)
            .show(ctx, |ui| {
                show_instrument_rack(
                    ui,
                    &mut self.instruments,
                    &mut self.active_instrument_id,
                    &mut self.handle,
                    &mut self.next_instrument_id,
                );
            });

        // Right side panel with meters and master effects (resizable)
        egui::SidePanel::right("meters_panel")
            .min_width(140.0)
            .default_width(180.0)
            .max_width(300.0)
            .resizable(true)
            .show(ctx, |ui| {
                self.draw_meters(ui);
            });

        // Main content - show the active instrument's patch editor
        // Get active instrument id for engine commands
        let active_id = self.active_instrument_id;

        egui::CentralPanel::default().show(ctx, |ui| {
            // Get the active instrument's patch editor
            #[allow(clippy::expect_used)]
            let patch_editor = self
                .instruments
                .iter_mut()
                .find(|i| i.id == active_id)
                .map(|i| &mut i.patch_editor)
                .expect("Active instrument not found");

            let result = patch_editor.show(ui, &self.handle, active_id.as_u64());

            // Handle parameter changes - send Param directly (carries its own value)
            for (module_id, param) in result.param_changes {
                // Check module category
                let category = patch_editor
                    .module_descriptor(module_id)
                    .map(|d| d.category);

                match category {
                    Some(ModuleCategory::Effect) => {
                        // Effect module - use SetEffectParameter (targets active instrument)
                        if let Some(effect_type) =
                            patch_bridge::get_effect_type_from_module(patch_editor, module_id)
                        {
                            self.handle.send(EngineCommand::SetEffectParameter {
                                instrument_id: Some(active_id),
                                effect_type,
                                param,
                            });
                        }
                    }
                    Some(
                        ModuleCategory::Oscillator
                        | ModuleCategory::Filter
                        | ModuleCategory::Envelope
                        | ModuleCategory::LFO
                        | ModuleCategory::Amplifier
                        | ModuleCategory::Mixer
                        | ModuleCategory::Output,
                    ) => {
                        // Voice/modular module - send to active instrument's voice graph
                        self.handle.send(EngineCommand::SetModuleParameter {
                            instrument_id: Some(active_id),
                            module_id,
                            param,
                        });

                        // Also send to voice modules for real-time voice param updates
                        if let Some(voice_module) =
                            patch_bridge::get_voice_module_for_param(module_id, &param)
                        {
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
                let category = patch_editor
                    .module_descriptor(module_id)
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
                    Some(
                        ModuleCategory::Oscillator
                        | ModuleCategory::Filter
                        | ModuleCategory::Envelope
                        | ModuleCategory::LFO
                        | ModuleCategory::Amplifier
                        | ModuleCategory::Mixer
                        | ModuleCategory::Output,
                    ) => {
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

            // Handle bypass toggles - send SetBypass commands to engine
            for (module_id, new_bypass_state) in result.bypass_toggles {
                self.handle.send(EngineCommand::SetBypass {
                    module: module_id,
                    bypass: new_bypass_state,
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
        let (module, descriptor, module_type): (
            Box<dyn crate::modules::PolyModule>,
            _,
            TypedModuleType,
        ) = match category {
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
        let (effect, descriptor, module_type): (
            Box<dyn crate::modules::AudioEffect>,
            _,
            TypedModuleType,
        ) = match effect_type {
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
            VisualizerType::Oscilloscope => (
                Oscilloscope::new().descriptor(),
                TypedModuleType::Oscilloscope,
            ),
            VisualizerType::LevelMeter => {
                (LevelMeter::new().descriptor(), TypedModuleType::LevelMeter)
            }
        };

        let next_id = self.next_module_id(module_type);
        // Visualizers are added to the active instrument's patch editor for visual display
        self.active_patch_editor().add_module(next_id, descriptor);

        // Create shared visualization buffer wrapped in Arc
        let buffer = std::sync::Arc::new(crate::visualizers::VisualizationBuffer::new(4096));

        // Store Arc clone in our handle for GUI access (same buffer!)
        self.handle
            .add_visualization_buffer(next_id, buffer.clone());

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

    /// Add an effect to the master bus.
    fn add_master_effect(&mut self, effect_type: EffectType) {
        // Create effect in GUI thread (real-time safe allocation)
        let (effect, module_type): (Box<dyn crate::modules::AudioEffect>, TypedModuleType) =
            match effect_type {
                EffectType::Delay => (Box::new(Delay::new()), TypedModuleType::Delay),
                EffectType::Reverb => (Box::new(Reverb::new()), TypedModuleType::Reverb),
                EffectType::Distortion => {
                    (Box::new(Distortion::new()), TypedModuleType::Distortion)
                }
                EffectType::Chorus => (Box::new(Chorus::new()), TypedModuleType::Chorus),
                EffectType::Phaser => (Box::new(Phaser::new()), TypedModuleType::Phaser),
                EffectType::Flanger => (Box::new(Flanger::new()), TypedModuleType::Flanger),
                EffectType::Compressor => {
                    (Box::new(Compressor::new()), TypedModuleType::Compressor)
                }
                EffectType::Eq => (Box::new(Eq::new()), TypedModuleType::Eq),
            };

        let next_id = self.next_module_id(module_type);

        // Send to engine with instrument_id: None to target master bus
        self.handle.send(EngineCommand::AddEffectInstance {
            instrument_id: None, // Master bus!
            id: next_id,
            effect,
        });

        // Track in UI state
        self.master_effects
            .push(MasterEffectUiState::new(next_id, effect_type));
    }

    fn draw_meters(&mut self, ui: &mut egui::Ui) {
        // Output meters section - horizontal layout
        ui.vertical(|ui| {
            ui.label(RichText::new("OUTPUT").color(colors::TEXT_DIM).small());
            ui.add_space(4.0);

            let (peak_l, peak_r) = self.handle.peak_meters();
            let (rms_l, rms_r) = self.handle.rms_meters();
            let meter_width = ui.available_width() - 30.0;

            // Left channel - horizontal
            ui.horizontal(|ui| {
                ui.label(RichText::new("L").color(colors::TEXT_DIM).size(10.0));
                draw_meter_horizontal(ui, peak_l, rms_l, meter_width, 12.0);
            });

            // Right channel - horizontal
            ui.horizontal(|ui| {
                ui.label(RichText::new("R").color(colors::TEXT_DIM).size(10.0));
                draw_meter_horizontal(ui, peak_r, rms_r, meter_width, 12.0);
            });

            // dB readout
            ui.horizontal(|ui| {
                let db_l = 20.0 * peak_l.max(0.0001).log10();
                let db_r = 20.0 * peak_r.max(0.0001).log10();
                ui.label(
                    RichText::new(format!("{:+.1} / {:+.1} dB", db_l, db_r))
                        .color(colors::TEXT_DIM)
                        .size(9.0),
                );
            });
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Master FX section
        self.draw_master_fx_section(ui);
    }

    /// Draw the Master FX section in the sidebar
    #[allow(clippy::too_many_lines)]
    fn draw_master_fx_section(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.label(RichText::new("MASTER FX").color(colors::TEXT_DIM).small());
            ui.add_space(4.0);

            // Track actions to apply after iteration (to avoid borrow issues)
            let mut effect_to_remove: Option<usize> = None;
            let mut effect_to_toggle_bypass: Option<usize> = None;
            let mut effect_to_toggle_expand: Option<usize> = None;
            let mut param_changes: Vec<(EffectType, Param)> = Vec::new();

            // Clone effect data for iteration (to allow mutation)
            let effects_snapshot: Vec<_> = self
                .master_effects
                .iter()
                .map(|e| (e.effect_type, e.expanded, e.bypassed, e.params.clone()))
                .collect();

            // Scrollable list of effects
            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 40.0)
                .show(ui, |ui| {
                    for (idx, (effect_type, is_expanded, is_bypassed, params)) in
                        effects_snapshot.iter().enumerate()
                    {
                        // Effect header frame
                        let frame_color = if *is_bypassed {
                            colors::BG_WIDGET.gamma_multiply(0.5)
                        } else {
                            colors::BG_WIDGET
                        };

                        egui::Frame::new()
                            .fill(frame_color)
                            .inner_margin(4.0)
                            .corner_radius(4.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // Expand/collapse toggle
                                    let arrow = if *is_expanded { "▼" } else { "▶" };
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(arrow)
                                                    .color(colors::TEXT_DIM)
                                                    .size(10.0),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                    {
                                        effect_to_toggle_expand = Some(idx);
                                    }

                                    // Effect name
                                    let name = self.master_effects[idx].display_name();
                                    let name_color = if *is_bypassed {
                                        colors::TEXT_DIM
                                    } else {
                                        colors::TEXT_PRIMARY
                                    };
                                    ui.label(RichText::new(name).color(name_color).size(11.0));

                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            // Remove button
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("×")
                                                            .color(colors::TEXT_DIM)
                                                            .size(12.0),
                                                    )
                                                    .min_size(egui::vec2(18.0, 18.0)),
                                                )
                                                .on_hover_text("Remove effect")
                                                .clicked()
                                            {
                                                effect_to_remove = Some(idx);
                                            }

                                            // Bypass button
                                            let bypass_color = if *is_bypassed {
                                                colors::ACCENT_YELLOW
                                            } else {
                                                colors::TEXT_DIM
                                            };
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new("B")
                                                            .color(bypass_color)
                                                            .size(10.0),
                                                    )
                                                    .min_size(egui::vec2(18.0, 18.0)),
                                                )
                                                .on_hover_text("Bypass effect")
                                                .clicked()
                                            {
                                                effect_to_toggle_bypass = Some(idx);
                                            }
                                        },
                                    );
                                });

                                // Expanded content with parameters
                                if *is_expanded {
                                    ui.add_space(4.0);
                                    ui.separator();
                                    ui.add_space(2.0);

                                    // Draw parameters based on effect type
                                    draw_effect_params(
                                        ui,
                                        idx,
                                        *effect_type,
                                        params,
                                        &mut self.master_effects,
                                        &mut param_changes,
                                    );
                                }
                            });

                        ui.add_space(2.0);
                    }
                });

            // Apply actions after iteration
            if let Some(idx) = effect_to_toggle_expand {
                self.master_effects[idx].expanded = !self.master_effects[idx].expanded;
            }

            if let Some(idx) = effect_to_toggle_bypass {
                let effect = &mut self.master_effects[idx];
                effect.bypassed = !effect.bypassed;
                self.handle.send(EngineCommand::SetEffectEnabled {
                    instrument_id: None, // Master bus
                    effect_type: effect.effect_type,
                    enabled: !effect.bypassed,
                });
            }

            if let Some(idx) = effect_to_remove {
                let removed = self.master_effects.remove(idx);
                self.handle.send(EngineCommand::RemoveEffect {
                    instrument_id: None, // Master bus
                    id: removed.id,
                });
            }

            // Send parameter changes to engine
            for (effect_type, param) in param_changes {
                self.handle.send(EngineCommand::SetEffectParameter {
                    instrument_id: None, // Master bus
                    effect_type,
                    param,
                });
            }

            ui.add_space(8.0);

            // Add effect dropdown
            egui::ComboBox::from_id_salt("add_master_fx")
                .selected_text(RichText::new("+ Add Effect").size(10.0))
                .width(ui.available_width() - 8.0)
                .show_ui(ui, |ui| {
                    let effect_types = [
                        (EffectType::Compressor, "Compressor"),
                        (EffectType::Eq, "EQ"),
                        (EffectType::Reverb, "Reverb"),
                        (EffectType::Delay, "Delay"),
                        (EffectType::Chorus, "Chorus"),
                        (EffectType::Phaser, "Phaser"),
                        (EffectType::Flanger, "Flanger"),
                        (EffectType::Distortion, "Distortion"),
                    ];

                    for (effect_type, name) in effect_types {
                        if ui.selectable_label(false, name).clicked() {
                            self.add_master_effect(effect_type);
                        }
                    }
                });
        });
    }

    fn draw_keyboard(&mut self, ui: &mut egui::Ui) {
        // Get the MIDI channel for the active instrument
        let active_channel = self
            .instruments
            .iter()
            .find(|p| p.id == self.active_instrument_id)
            .map(|p| p.channel)
            .unwrap_or(MidiChannel::CH1);

        ui.horizontal(|ui| {
            // Panic button (moved here since keyboard handles its own header)
            if ui
                .add(egui::Button::new(
                    RichText::new("PANIC").color(colors::ACCENT_RED),
                ))
                .clicked()
            {
                self.handle.send(EngineCommand::AllNotesOff);
                self.pressed_keys.clear();
                // Keyboard will be cleared by AllNotesReleased event from engine
            }

            // Show active instrument indicator
            let active_name = self
                .instruments
                .iter()
                .find(|p| p.id == self.active_instrument_id)
                .map(|p| p.name.as_str())
                .unwrap_or("Instrument 1");
            ui.separator();
            ui.label(
                RichText::new(format!("Playing: {}", active_name)).color(colors::ACCENT_ORANGE),
            );
        });

        // Note: Keyboard visual state is now driven by engine events (NoteTriggered/NoteReleased)
        // in the main update loop. This ensures the GUI reflects what the engine is actually
        // playing, regardless of input source (MIDI, sequencer, or GUI).

        // Show the 88-key piano keyboard
        let event = self.keyboard.show(ui);

        // Handle note events from mouse interaction - send to active instrument's channel
        if let Some(note) = event.note_on {
            self.handle
                .note_on_channel(note, NormalizedValue::new(0.8), active_channel);
        }
        for note in event.note_off {
            self.handle.note_off_channel(note, active_channel);
            // Note release will be reflected via NoteReleased event from engine
        }
    }

    fn handle_keyboard_input(&mut self, ctx: &egui::Context) {
        // Get the MIDI channel for the active instrument
        let active_channel = self
            .instruments
            .iter()
            .find(|p| p.id == self.active_instrument_id)
            .map(|p| p.channel)
            .unwrap_or(MidiChannel::CH1);

        let key_map: &[(egui::Key, u8)] = &[
            // Lower row: Z-M = C3-B3
            (egui::Key::Z, 48),
            (egui::Key::S, 49),
            (egui::Key::X, 50),
            (egui::Key::D, 51),
            (egui::Key::C, 52),
            (egui::Key::V, 53),
            (egui::Key::G, 54),
            (egui::Key::B, 55),
            (egui::Key::H, 56),
            (egui::Key::N, 57),
            (egui::Key::J, 58),
            (egui::Key::M, 59),
            // Upper row: Q-P = C4-E5
            (egui::Key::Q, 60),
            (egui::Key::Num2, 61),
            (egui::Key::W, 62),
            (egui::Key::Num3, 63),
            (egui::Key::E, 64),
            (egui::Key::R, 65),
            (egui::Key::Num5, 66),
            (egui::Key::T, 67),
            (egui::Key::Num6, 68),
            (egui::Key::Y, 69),
            (egui::Key::Num7, 70),
            (egui::Key::U, 71),
            (egui::Key::I, 72),
        ];

        let octave_offset = self.keyboard.octave_offset();

        ctx.input(|input| {
            for (key, base_note) in key_map {
                let note_i32 = *base_note as i32 + octave_offset * 12;
                if !(0..=127).contains(&note_i32) {
                    continue; // Skip invalid notes
                }
                let note = note_i32 as u8;

                if input.key_pressed(*key)
                    && !self.pressed_keys.get(&note).copied().unwrap_or(false)
                {
                    self.handle.note_on_channel(
                        MidiNote::new(note),
                        NormalizedValue::new(0.8),
                        active_channel,
                    );
                    self.pressed_keys.insert(note, true);
                    // Visual feedback will come from NoteTriggered engine event
                }

                if input.key_released(*key) {
                    self.handle
                        .note_off_channel(MidiNote::new(note), active_channel);
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
                self.dialog_state
                    .set_status(format!("Loaded: {}", patch.name));
            }
            LoadPatchResult::Cancelled | LoadPatchResult::None => {}
        }

        // Save patch dialog
        match show_save_patch_dialog(
            ctx,
            &mut self.dialog_state.show_save_patch,
            &mut self.dialog_state.patch_save_name,
        ) {
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
        #[allow(clippy::expect_used)]
        let patch_editor = self
            .instruments
            .iter_mut()
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
            #[allow(clippy::expect_used)]
            let patch_editor = self
                .instruments
                .iter()
                .find(|i| i.id == active_id)
                .map(|i| &i.patch_editor)
                .expect("Active instrument not found");

            for module_id in patch_editor.module_ids() {
                let category = patch_editor
                    .module_descriptor(module_id)
                    .map(|d| d.category);
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

/// Draw effect parameters with compact sliders.
#[allow(clippy::too_many_lines)]
fn draw_effect_params(
    ui: &mut egui::Ui,
    idx: usize,
    effect_type: EffectType,
    params: &MasterEffectParams,
    effects: &mut [MasterEffectUiState],
    param_changes: &mut Vec<(EffectType, Param)>,
) {
    use crate::types::{Decibels, Hertz, Milliseconds, NormalizedValue, Ratio, Seconds};

    // Use push_id to give unique IDs to widgets within each effect
    ui.push_id(format!("fx_{}", idx), |ui| {
        match params {
            MasterEffectParams::Compressor {
                threshold,
                ratio,
                attack,
                release,
                makeup,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Compressor {
                    threshold: t,
                    ratio: r,
                    attack: a,
                    release: rel,
                    makeup: m,
                    mix: mx,
                } = &mut effect.params
                {
                    // Threshold
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Thresh").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}dB", threshold))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *threshold;
                    if ui
                        .add(egui::Slider::new(&mut val, -60.0..=0.0).show_value(false))
                        .changed()
                    {
                        *t = val;
                        param_changes.push((
                            effect_type,
                            Param::Compressor(CompressorParam::Threshold(Decibels::new(val))),
                        ));
                    }

                    // Ratio
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Ratio").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.1}:1", ratio))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *ratio;
                    if ui
                        .add(egui::Slider::new(&mut val, 1.0..=20.0).show_value(false))
                        .changed()
                    {
                        *r = val;
                        param_changes.push((
                            effect_type,
                            Param::Compressor(CompressorParam::Ratio(Ratio::new(val))),
                        ));
                    }

                    // Attack
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Attack").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.1}ms", attack))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *attack;
                    if ui
                        .add(
                            egui::Slider::new(&mut val, 0.1..=100.0)
                                .show_value(false)
                                .logarithmic(true),
                        )
                        .changed()
                    {
                        *a = val;
                        param_changes.push((
                            effect_type,
                            Param::Compressor(CompressorParam::Attack(Milliseconds::new(val))),
                        ));
                    }

                    // Release
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Release").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}ms", release))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *release;
                    if ui
                        .add(
                            egui::Slider::new(&mut val, 10.0..=1000.0)
                                .show_value(false)
                                .logarithmic(true),
                        )
                        .changed()
                    {
                        *rel = val;
                        param_changes.push((
                            effect_type,
                            Param::Compressor(CompressorParam::Release(Milliseconds::new(val))),
                        ));
                    }

                    // Makeup
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Makeup").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.1}dB", makeup))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *makeup;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=24.0).show_value(false))
                        .changed()
                    {
                        *m = val;
                        param_changes.push((
                            effect_type,
                            Param::Compressor(CompressorParam::Makeup(Decibels::new(val))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mix").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *mix;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *mx = val;
                        param_changes.push((
                            effect_type,
                            Param::Compressor(CompressorParam::Mix(NormalizedValue::new(val))),
                        ));
                    }
                }
            }

            MasterEffectParams::Eq {
                low_gain,
                mid_gain,
                high_gain,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Eq {
                    low_gain: lg,
                    mid_gain: mg,
                    high_gain: hg,
                    mix: mx,
                } = &mut effect.params
                {
                    // Low Gain
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Low").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.1}dB", low_gain))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *low_gain;
                    if ui
                        .add(egui::Slider::new(&mut val, -12.0..=12.0).show_value(false))
                        .changed()
                    {
                        *lg = val;
                        param_changes
                            .push((effect_type, Param::Eq(EqParam::LowGain(Decibels::new(val)))));
                    }

                    // Mid Gain
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mid").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.1}dB", mid_gain))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *mid_gain;
                    if ui
                        .add(egui::Slider::new(&mut val, -12.0..=12.0).show_value(false))
                        .changed()
                    {
                        *mg = val;
                        param_changes
                            .push((effect_type, Param::Eq(EqParam::MidGain(Decibels::new(val)))));
                    }

                    // High Gain
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("High").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.1}dB", high_gain))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *high_gain;
                    if ui
                        .add(egui::Slider::new(&mut val, -12.0..=12.0).show_value(false))
                        .changed()
                    {
                        *hg = val;
                        param_changes.push((
                            effect_type,
                            Param::Eq(EqParam::HighGain(Decibels::new(val))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mix").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *mix;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *mx = val;
                        param_changes.push((
                            effect_type,
                            Param::Eq(EqParam::Mix(NormalizedValue::new(val))),
                        ));
                    }
                }
            }

            MasterEffectParams::Reverb {
                room_size,
                damping,
                width,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Reverb {
                    room_size: rs,
                    damping: d,
                    width: w,
                    mix: mx,
                } = &mut effect.params
                {
                    // Room Size
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Size").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", room_size * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *room_size;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *rs = val;
                        param_changes.push((
                            effect_type,
                            Param::Reverb(ReverbParam::RoomSize(NormalizedValue::new(val))),
                        ));
                    }

                    // Damping
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Damp").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", damping * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *damping;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *d = val;
                        param_changes.push((
                            effect_type,
                            Param::Reverb(ReverbParam::Damping(NormalizedValue::new(val))),
                        ));
                    }

                    // Width
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Width").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", width * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *width;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *w = val;
                        param_changes.push((
                            effect_type,
                            Param::Reverb(ReverbParam::Width(NormalizedValue::new(val))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mix").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *mix;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *mx = val;
                        param_changes.push((
                            effect_type,
                            Param::Reverb(ReverbParam::Mix(NormalizedValue::new(val))),
                        ));
                    }
                }
            }

            MasterEffectParams::Delay {
                time,
                feedback,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Delay {
                    time: t,
                    feedback: fb,
                    mix: mx,
                } = &mut effect.params
                {
                    // Time
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Time").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.2}s", time))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *time;
                    if ui
                        .add(
                            egui::Slider::new(&mut val, 0.01..=2.0)
                                .show_value(false)
                                .logarithmic(true),
                        )
                        .changed()
                    {
                        *t = val;
                        param_changes.push((
                            effect_type,
                            Param::Delay(DelayParam::Time(Seconds::new(val))),
                        ));
                    }

                    // Feedback
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Feedback").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", feedback * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *feedback;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=0.95).show_value(false))
                        .changed()
                    {
                        *fb = val;
                        param_changes.push((
                            effect_type,
                            Param::Delay(DelayParam::Feedback(NormalizedValue::new(val))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mix").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *mix;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *mx = val;
                        param_changes.push((
                            effect_type,
                            Param::Delay(DelayParam::Mix(NormalizedValue::new(val))),
                        ));
                    }
                }
            }

            MasterEffectParams::Chorus { rate, depth, mix } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Chorus {
                    rate: r,
                    depth: d,
                    mix: mx,
                } = &mut effect.params
                {
                    // Rate
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Rate").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.2}Hz", rate))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *rate;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.1..=5.0).show_value(false))
                        .changed()
                    {
                        *r = val;
                        param_changes.push((
                            effect_type,
                            Param::Chorus(ChorusParam::Rate(Hertz::new(val))),
                        ));
                    }

                    // Depth
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Depth").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", depth * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *depth;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *d = val;
                        param_changes.push((
                            effect_type,
                            Param::Chorus(ChorusParam::Depth(NormalizedValue::new(val))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mix").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *mix;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *mx = val;
                        param_changes.push((
                            effect_type,
                            Param::Chorus(ChorusParam::Mix(NormalizedValue::new(val))),
                        ));
                    }
                }
            }

            MasterEffectParams::Phaser {
                rate,
                depth,
                feedback,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Phaser {
                    rate: r,
                    depth: d,
                    feedback: fb,
                    mix: mx,
                } = &mut effect.params
                {
                    // Rate
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Rate").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.2}Hz", rate))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *rate;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.1..=5.0).show_value(false))
                        .changed()
                    {
                        *r = val;
                        param_changes.push((
                            effect_type,
                            Param::Phaser(PhaserParam::Rate(Hertz::new(val))),
                        ));
                    }

                    // Depth
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Depth").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", depth * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *depth;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *d = val;
                        param_changes.push((
                            effect_type,
                            Param::Phaser(PhaserParam::Depth(NormalizedValue::new(val))),
                        ));
                    }

                    // Feedback
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Feedback").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", feedback * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *feedback;
                    if ui
                        .add(egui::Slider::new(&mut val, -1.0..=1.0).show_value(false))
                        .changed()
                    {
                        *fb = val;
                        param_changes.push((
                            effect_type,
                            Param::Phaser(PhaserParam::Feedback(crate::types::BipolarValue::new(
                                val,
                            ))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mix").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *mix;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *mx = val;
                        param_changes.push((
                            effect_type,
                            Param::Phaser(PhaserParam::Mix(NormalizedValue::new(val))),
                        ));
                    }
                }
            }

            MasterEffectParams::Flanger {
                rate,
                depth,
                feedback,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Flanger {
                    rate: r,
                    depth: d,
                    feedback: fb,
                    mix: mx,
                } = &mut effect.params
                {
                    // Rate
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Rate").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.2}Hz", rate))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *rate;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.1..=5.0).show_value(false))
                        .changed()
                    {
                        *r = val;
                        param_changes.push((
                            effect_type,
                            Param::Flanger(FlangerParam::Rate(Hertz::new(val))),
                        ));
                    }

                    // Depth
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Depth").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", depth * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *depth;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *d = val;
                        param_changes.push((
                            effect_type,
                            Param::Flanger(FlangerParam::Depth(NormalizedValue::new(val))),
                        ));
                    }

                    // Feedback
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Feedback").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", feedback * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *feedback;
                    if ui
                        .add(egui::Slider::new(&mut val, -1.0..=1.0).show_value(false))
                        .changed()
                    {
                        *fb = val;
                        param_changes.push((
                            effect_type,
                            Param::Flanger(FlangerParam::Feedback(
                                crate::types::BipolarValue::new(val),
                            )),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mix").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *mix;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *mx = val;
                        param_changes.push((
                            effect_type,
                            Param::Flanger(FlangerParam::Mix(NormalizedValue::new(val))),
                        ));
                    }
                }
            }

            MasterEffectParams::Distortion { drive, tone, mix } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Distortion {
                    drive: dr,
                    tone: t,
                    mix: mx,
                } = &mut effect.params
                {
                    // Drive
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Drive").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", drive * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *drive;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *dr = val;
                        param_changes.push((
                            effect_type,
                            Param::Distortion(DistortionParam::Drive(NormalizedValue::new(val))),
                        ));
                    }

                    // Tone
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Tone").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", tone * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *tone;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *t = val;
                        param_changes.push((
                            effect_type,
                            Param::Distortion(DistortionParam::Tone(NormalizedValue::new(val))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Mix").color(colors::TEXT_DIM).size(9.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(colors::TEXT_SECONDARY)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *mix;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *mx = val;
                        param_changes.push((
                            effect_type,
                            Param::Distortion(DistortionParam::Mix(NormalizedValue::new(val))),
                        ));
                    }
                }
            }
        }
    });
}

/// Draw a vertical audio meter.
#[allow(dead_code)]
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
        [
            Pos2::new(rect.left(), peak_y),
            Pos2::new(rect.right(), peak_y),
        ],
        Stroke::new(2.0, colors::TEXT_PRIMARY),
    );
}

/// Draw a horizontal audio meter.
fn draw_meter_horizontal(ui: &mut egui::Ui, peak: f32, rms: f32, width: f32, height: f32) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 2.0, colors::BG_DARK);

    // RMS level (horizontal - grows from left)
    let rms_width = rect.width() * rms.clamp(0.0, 1.0);
    let rms_rect = egui::Rect::from_min_size(rect.min, Vec2::new(rms_width, rect.height()));

    let rms_color = if rms > 0.9 {
        colors::METER_RED
    } else if rms > 0.7 {
        colors::METER_YELLOW
    } else {
        colors::METER_GREEN
    };

    painter.rect_filled(rms_rect, 0.0, rms_color);

    // Peak indicator (vertical line)
    let peak_x = rect.left() + rect.width() * peak.clamp(0.0, 1.0);
    painter.line_segment(
        [
            Pos2::new(peak_x, rect.top()),
            Pos2::new(peak_x, rect.bottom()),
        ],
        Stroke::new(2.0, colors::TEXT_PRIMARY),
    );
}
