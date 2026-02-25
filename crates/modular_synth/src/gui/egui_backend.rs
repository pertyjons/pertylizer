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

use eframe::egui::{self, Color32, RichText, Stroke, Vec2};

use crate::audio::AudioHostTrait;
use crate::gui::app::state::AppView;
use crate::gui::dialogs::{
    DialogState, FileDialogMode, FileDialogResult, LoadPatchResult, show_about_dialog,
    show_load_patch_dialog, show_settings_dialog, show_status_toast,
};
use crate::gui::input::handle_keyboard_input;
use crate::gui::instrument_rack::{InstrumentUiState, show_instrument_rack};
use crate::gui::keyboard::PianoKeyboard;
use crate::gui::patch_bridge;
use crate::gui::patch_editor::{
    EffectType, ModulePalette, PaletteSelection, PaletteVisualizerType, PatchEditor,
    QuickAddRequest,
};
use crate::gui::theme::theme;
use crate::gui::views::{MasterEffectParams, MasterEffectUiState, draw_meter_horizontal};
use crate::gui::widgets::draw_oscilloscope;
use crate::gui::{GuiBackend, GuiResult, SynthGuiConfig};
use crate::io::MidiHandler;
use crate::patch::{Patch, categorized_patches};
use synth_core::Velocity;
#[cfg(feature = "mcp")]
use synth_core::{BipolarValue, Gain};
use synth_core::{
    ChorusParam, CompressorParam, DelayParam, DistortionParam, EqParam, FlangerParam, MidSideParam,
    Param, PhaserParam, ReverbParam, WaveshaperParam,
};
use synth_core::{Describable, ModuleCategory};
use synth_engine::ModuleType as TypedModuleType;
use synth_engine::commands::PortId;
use synth_engine::instrument::{InstrumentId, MidiChannel};
use synth_engine::visualizers::{LevelMeter, Oscilloscope, SpectrumAnalyzer};
use synth_engine::{EngineCommand, EngineEvent, EngineHandle, ModuleId, SynthEngine};
use synth_modules::effects::{
    BbdDelay, Chorus, Compressor, Convolver, Delay, Distortion, Eq, Flanger, FrequencyShifter,
    Limiter, MidSide, PhaseVocoder, Phaser, Reverb, Waveshaper,
};

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

        let window_title = config.title.clone();
        let window_width = config.width as f32;
        let window_height = config.height as f32;

        let app = SynthApp::new(handle, host, config, stream_info.output_latency);

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([window_width, window_height])
                .with_title(&window_title)
                .with_min_inner_size([800.0, 600.0]),
            ..Default::default()
        };

        eframe::run_native(
            &window_title,
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
/// Reads colors from the current theme, so call this after changing theme.
pub fn setup_custom_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let t = theme();
    let colors = &t.colors;

    // Dark theme with synth colors
    style.visuals.dark_mode = true;
    style.visuals.override_text_color = Some(colors.text_primary);
    style.visuals.panel_fill = colors.bg_panel;
    style.visuals.window_fill = colors.bg_module;
    style.visuals.faint_bg_color = colors.bg_widget;

    // Widget styling - use bg_dark for slider backgrounds (darker contrast)
    style.visuals.widgets.inactive.bg_fill = colors.bg_dark;
    style.visuals.widgets.inactive.weak_bg_fill = colors.bg_dark;
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, colors.text_secondary);
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, colors.border);

    // Hovered: slightly lighter than bg_dark
    let hovered_bg = Color32::from_rgb(
        colors.bg_dark.r().saturating_add(20),
        colors.bg_dark.g().saturating_add(20),
        colors.bg_dark.b().saturating_add(20),
    );
    style.visuals.widgets.hovered.bg_fill = hovered_bg;
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, colors.text_primary);
    style.visuals.widgets.hovered.bg_stroke =
        Stroke::new(1.0, colors.accent_primary.gamma_multiply(0.5));

    style.visuals.widgets.active.bg_fill = colors.accent_primary;
    style.visuals.widgets.active.fg_stroke = Stroke::new(2.0, colors.bg_dark);

    style.visuals.selection.bg_fill = colors.accent_primary.gamma_multiply(0.4);
    style.visuals.selection.stroke = Stroke::new(1.0, colors.accent_primary);

    // Rounded corners
    style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(4);
    style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(4);
    style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(4);

    // Spacing - make sliders thicker and easier to interact with
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    style.spacing.slider_rail_height = t.style.slider_rail_height;
    style.spacing.interact_size.y = 18.0; // Taller click area for sliders

    ctx.set_style(style);
}

/// Main application state.
struct SynthApp {
    handle: EngineHandle,
    host: Option<Box<dyn AudioHostTrait>>,
    latency: std::time::Duration,

    // Shared session for module lifecycle
    session: std::sync::Arc<crate::session::SynthSession>,

    // MIDI input handler
    midi_handler: MidiHandler,

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

    // Navigation state
    active_view: AppView,

    // AWE state
    awe_enabled: bool,
    awe_ui: crate::gui::awe_view::AweUiState,

    // MCP shared state
    #[cfg(feature = "mcp")]
    mcp_shared: Option<std::sync::Arc<crate::mcp_shared::McpSharedState>>,
}

impl SynthApp {
    #[allow(unused_variables)]
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

        let session = config.session.clone();
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
        let startup_patch = crate::patches::default_patch();
        let patch_name = startup_patch.name.clone();

        patch_bridge::load_patch(
            &startup_patch,
            &mut default_instrument.patch_editor,
            &session,
            &mut handle,
            &mut keyboard,
            &mut glide_time,
            active_instrument_id,
        );

        let instruments = vec![default_instrument];

        // Set the focused instrument for keyboard routing (only this instrument receives keyboard input)
        handle.set_focused_instrument(Some(active_instrument_id));

        // Initialize MIDI input (connects to first available port)
        // The MidiHandler gets a clone of the command sender, so both GUI and MIDI
        // can send commands to the engine.
        let midi_handler = MidiHandler::new(handle.command_sender());

        Self {
            handle,
            host: Some(host),
            latency,
            session,
            midi_handler,
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
            active_view: AppView::default(),
            awe_enabled: false,
            awe_ui: crate::gui::awe_view::AweUiState::default(),
            #[cfg(feature = "mcp")]
            mcp_shared: config.mcp_shared,
        }
    }

    /// Add a module via session and register it in the active patch editor.
    /// Returns the assigned `ModuleId` and descriptor, or `None` on failure.
    fn session_add_module(
        &mut self,
        module_type: TypedModuleType,
    ) -> Option<(ModuleId, synth_core::ModuleDescriptor)> {
        let result = self
            .session
            .add_module(self.active_instrument_id, module_type)
            .ok()?;
        Some(result)
    }

    /// Get the active instrument's patch editor.
    ///
    /// Returns `None` if active_instrument_id doesn't match any instrument.
    /// This can happen briefly during instrument deletion/creation.
    fn active_patch_editor(&mut self) -> Option<&mut PatchEditor> {
        self.instruments
            .iter_mut()
            .find(|i| i.id == self.active_instrument_id)
            .map(|i| &mut i.patch_editor)
    }

    /// Get the active instrument's patch editor (immutable).
    ///
    /// Returns `None` if active_instrument_id doesn't match any instrument.
    /// This can happen briefly during instrument deletion/creation.
    fn active_patch_editor_ref(&self) -> Option<&PatchEditor> {
        self.instruments
            .iter()
            .find(|i| i.id == self.active_instrument_id)
            .map(|i| &i.patch_editor)
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
                EngineEvent::KeyRangeLearned {
                    instrument_id,
                    key_range,
                    learn_state,
                } => {
                    // Update the instrument's UI state with the learned key range
                    if let Some(inst) = self.instruments.iter_mut().find(|i| i.id == instrument_id)
                    {
                        inst.key_range = key_range;
                        inst.learn_state = learn_state;
                    }
                }
                // Other events (meters, etc.) are handled elsewhere
                _ => {}
            }
        }

        // Poll MCP pending patch
        #[cfg(feature = "mcp")]
        {
            let pending_patch = self.mcp_shared.as_ref().and_then(|shared| {
                shared
                    .pending_patch
                    .lock()
                    .ok()
                    .and_then(|mut pending| pending.take())
            });
            if let Some((patch, name)) = pending_patch {
                self.current_patch_name = name;
                self.current_patch_path = None;
                self.load_patch_data(&patch);
            }
        }

        // Reconcile with session: detect modules added/removed by MCP
        #[cfg(feature = "mcp")]
        self.reconcile_with_session();

        // Handle keyboard input
        self.process_keyboard_input(ctx);

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
                    if ui.button("📂 Open Patch...").clicked() {
                        self.dialog_state.open_open_patch_dialog();
                        ui.close();
                    }
                    if ui.button("📋 Load Built-in...").clicked() {
                        self.dialog_state.show_load_patch = true;
                        ui.close();
                    }
                    if ui.button("💾 Save Patch...").clicked() {
                        let default_name = format!(
                            "{}.json",
                            self.current_patch_name.to_lowercase().replace(' ', "_")
                        );
                        self.dialog_state.open_save_patch_dialog(&default_name);
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button("📋 Example Patches", |ui| {
                        for (category, patches) in categorized_patches() {
                            ui.menu_button(category, |ui| {
                                for patch in patches {
                                    if ui.button(&patch.name).clicked() {
                                        self.load_patch_data(&patch);
                                        self.current_patch_name = patch.name.clone();
                                        self.dialog_state
                                            .set_status(format!("Loaded: {}", patch.name));
                                        ui.close();
                                    }
                                }
                            });
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
                        theme().colors.meter_red
                    } else if cpu > 0.5 {
                        theme().colors.meter_yellow
                    } else {
                        theme().colors.meter_green
                    };
                    ui.label(RichText::new(format!("CPU: {:>3.0}%", cpu * 100.0)).color(cpu_color));
                    ui.separator();
                    ui.label(
                        RichText::new(format!("Voices: {}", self.handle.voice_count()))
                            .color(theme().colors.text_secondary),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "Latency: {:.1}ms",
                            self.latency.as_secs_f64() * 1000.0
                        ))
                        .color(theme().colors.text_dim),
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
                        RichText::new(format!("🎹 {} ▼", short_name))
                            .color(theme().colors.meter_green)
                    } else {
                        RichText::new("🎹 MIDI ▼").color(theme().colors.text_dim)
                    };

                    ui.menu_button(midi_label, |ui| {
                        ui.set_min_width(250.0);
                        let ports = MidiHandler::list_ports();
                        if ports.is_empty() {
                            ui.label(
                                RichText::new("No MIDI ports available")
                                    .color(theme().colors.text_dim),
                            );
                        } else {
                            for port in &ports {
                                let is_current =
                                    self.midi_handler.port_name() == Some(port.as_str());
                                let label = if is_current {
                                    RichText::new(format!("● {}", port))
                                        .color(theme().colors.meter_green)
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
                    });
                    ui.separator();
                    // Current patch name
                    ui.label(
                        RichText::new(format!("Patch: {}", self.current_patch_name))
                            .color(theme().colors.accent_cyan),
                    );
                    ui.separator();

                    // AWE view toggle with on/off indicator
                    let awe_indicator_color = if self.awe_enabled {
                        theme().colors.meter_green
                    } else {
                        theme().colors.text_dim
                    };
                    let view_label = match self.active_view {
                        AppView::Rack => "● AWE",
                        AppView::AcousticWorld => "● Rack",
                    };
                    if ui
                        .button(RichText::new(view_label).color(awe_indicator_color))
                        .clicked()
                    {
                        self.active_view = match self.active_view {
                            AppView::Rack => AppView::AcousticWorld,
                            AppView::AcousticWorld => AppView::Rack,
                        };
                    }
                    ui.separator();
                });
            });
        });

        // Toolbar for adding modules (only show in Rack view)
        if self.active_view == AppView::Rack {
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
                            PaletteSelection::ModMatrix => {
                                self.add_mod_matrix_module();
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
                            // Modulation / Utility
                            PaletteSelection::RingMod => {
                                self.add_ring_mod_module();
                            }
                            PaletteSelection::EnvelopeFollower => {
                                self.add_envelope_follower_module();
                            }
                            PaletteSelection::WavetableOsc => {
                                self.add_wavetable_osc_module();
                            }
                            // Physical modeling
                            PaletteSelection::KeyboardPanner => {
                                self.add_keyboard_panner_module();
                            }
                            PaletteSelection::BodyResonance => {
                                self.add_body_resonance_module();
                            }
                            PaletteSelection::MechanicalNoise => {
                                self.add_mechanical_noise_module();
                            }
                            // New modules
                            PaletteSelection::Mseg => {
                                self.add_mseg_module();
                            }
                            PaletteSelection::AdditiveOsc => {
                                self.add_additive_osc_module();
                            }
                            PaletteSelection::Euclidean => {
                                self.add_euclidean_module();
                            }
                            PaletteSelection::TuringMachine => {
                                self.add_turing_machine_module();
                            }
                            PaletteSelection::RandomGates => {
                                self.add_random_gates_module();
                            }
                            PaletteSelection::GranularOsc => {
                                self.add_granular_osc_module();
                            }
                            PaletteSelection::KineticModulator => {
                                self.add_kinetic_modulator_module();
                            }
                            PaletteSelection::SignalMonitor => {
                                self.add_signal_monitor_module();
                            }
                        }
                    }

                    ui.separator();

                    // Glide/Portamento control
                    ui.label(RichText::new("Glide:").color(theme().colors.text_dim));
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
                            .send(EngineCommand::SetGlideTime(synth_core::Seconds::new(
                                self.glide_time,
                            )));
                    }

                    ui.separator();

                    // Connection info (from active instrument's patch editor)
                    let (conn_count, module_count) = self
                        .active_patch_editor_ref()
                        .map(|e| (e.connections().len(), e.module_ids().len()))
                        .unwrap_or((0, 0));
                    ui.label(
                        RichText::new(format!(
                            "Modules: {} | Connections: {}",
                            module_count, conn_count
                        ))
                        .color(theme().colors.text_dim),
                    );
                });
            });
        } // end if Rack view - toolbar

        // Bottom panel with keyboard (always visible)
        egui::TopBottomPanel::bottom("keyboard_panel")
            .min_height(120.0)
            .show(ctx, |ui| {
                self.draw_keyboard(ui);
            });

        // Side panels only visible in Rack view
        if self.active_view == AppView::Rack {
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
        }

        // Main content - CentralPanel rendered LAST (normal egui order)
        // Module Areas are clipped to visible_rect in patch_editor.rs
        match self.active_view {
            AppView::Rack => {
                // Rack view: show the active instrument's patch editor
                let active_id = self.active_instrument_id;
                egui::CentralPanel::default().show(ctx, |ui| {
            // Get the active instrument's patch editor
            let Some(patch_editor) = self
                .instruments
                .iter_mut()
                .find(|i| i.id == active_id)
                .map(|i| &mut i.patch_editor)
            else {
                // No active instrument - show error message
                ui.centered_and_justified(|ui| {
                    ui.label("No active instrument selected");
                });
                return;
            };

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
                        // SetModuleParameter updates both the template AND all active voices
                        self.handle.send(EngineCommand::SetModuleParameter {
                            instrument_id: Some(active_id),
                            module_id,
                            param,
                        });
                    }
                    _ => {}
                }
            }

            // Handle module removal
            for module_id in result.modules_to_remove {
                // Get module descriptor to determine type (before removing from editor)
                let category = patch_editor
                    .module_descriptor(module_id)
                    .map(|d| d.category);
                let type_id = patch_editor
                    .module_descriptor(module_id)
                    .map(|d| d.type_id.0.clone());

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
                        | ModuleCategory::Output
                        | ModuleCategory::Utility
                        | ModuleCategory::PhysicalModeling,
                    ) => {
                        // Remove from active instrument's voice graph
                        self.handle.send(EngineCommand::RemoveModule {
                            instrument_id: Some(active_id),
                            id: module_id,
                        });
                        // Signal monitor has a vis buffer that needs cleanup
                        if type_id.as_deref() == Some("signal_monitor") {
                            self.handle.remove_visualization_buffer(module_id);
                        }
                    }
                    _ => {}
                }
            }

            // Handle new connections - now synced with engine
            for connection in result.connections_to_add {
                patch_editor.add_connection(connection);

                // Send Connect command to engine (active instrument's voice graph)
                self.handle.send(EngineCommand::Connect {
                    instrument_id: Some(active_id),
                    from: PortId::new(connection.from_module, connection.from_port),
                    to: PortId::new(connection.to_module, connection.to_port),
                });
            }

            // Handle removed connections - send Disconnect commands to engine
            for connection in result.connections_to_remove {
                self.handle.send(EngineCommand::Disconnect {
                    instrument_id: Some(active_id),
                    from: PortId::new(connection.from_module, connection.from_port),
                    to: PortId::new(connection.to_module, connection.to_port),
                });
            }

            // Handle bypass toggles - send SetBypass commands to engine
            for (module_id, new_bypass_state) in result.bypass_toggles {
                self.handle.send(EngineCommand::SetBypass {
                    module: module_id,
                    bypass: new_bypass_state,
                });
            }

            // Handle signal monitor insertions — create inline monitor and rewire
            for connection in result.insert_signal_monitor_at {
                // Create signal monitor module (same DSP, different GUI descriptor)
                let mut m = synth_modules::SignalMonitor::new();

                // Build an inline descriptor: compact type_id, no parameters, just ports
                let inline_descriptor = synth_core::ModuleDescriptor::new(
                    "inline_signal_monitor",
                    "Mon",
                )
                .description("Inline signal monitor (compact pass-through)")
                .category(synth_core::ModuleCategory::Utility)
                .port(synth_core::PortDescriptor::audio_input("in", "In"))
                .port(synth_core::PortDescriptor::audio_output("out", "Out"));

                let monitor_id = {
                    let mut counters = self.session.counters_lock();
                    let counter = counters
                        .entry((self.active_instrument_id, synth_core::ModuleType::SignalMonitor))
                        .or_insert(0);
                    *counter += 1;
                    ModuleId::new(TypedModuleType::SignalMonitor, *counter)
                };

                // Position between the two connected modules
                let from_pos = patch_editor
                    .get_module_data(connection.from_module)
                    .map(|(_, pos, _)| pos);
                let to_pos = patch_editor
                    .get_module_data(connection.to_module)
                    .map(|(_, pos, _)| pos);
                let mid_pos = match (from_pos, to_pos) {
                    (Some(a), Some(b)) => egui::Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0),
                    (Some(a), None) => egui::Pos2::new(a.x + 200.0, a.y),
                    _ => egui::Pos2::new(100.0, 100.0),
                };

                patch_editor.add_module_at(monitor_id, inline_descriptor, mid_pos);

                // Create shared vis buffer and inject into module
                let buffer = std::sync::Arc::new(
                    synth_engine::visualizers::VisualizationBuffer::new(4096),
                );
                self.handle
                    .add_visualization_buffer(monitor_id, buffer.clone());
                m.set_vis_sink(buffer);

                let module: Box<dyn synth_core::PolyModule> = Box::new(m);
                self.handle.send(EngineCommand::AddModuleInstance {
                    instrument_id: Some(active_id),
                    id: monitor_id,
                    module,
                });

                // Wire: original_from → monitor "in", monitor "out" → original_to
                let conn_in = synth_engine::graph::Connection::new(
                    connection.from_module,
                    connection.from_port,
                    monitor_id,
                    "in",
                );
                let conn_out = synth_engine::graph::Connection::new(
                    monitor_id,
                    "out",
                    connection.to_module,
                    connection.to_port,
                );

                for c in [conn_in, conn_out] {
                    patch_editor.add_connection(c);
                    self.handle.send(EngineCommand::Connect {
                        instrument_id: Some(active_id),
                        from: PortId::new(c.from_module, c.from_port),
                        to: PortId::new(c.to_module, c.to_port),
                    });
                }
            }

            // Handle quick-add requests (right-click on port → add module)
            for request in result.quick_add_requests {
                Self::handle_quick_add(
                    &self.session,
                    &mut self.handle,
                    active_id,
                    patch_editor,
                    request,
                );
            }

            // Handle auto-layout request
            if result.request_auto_layout
                && let Some(canvas_rect) = result.canvas_rect
            {
                patch_editor.apply_auto_layout(canvas_rect);
            }
                });
            }
            AppView::AcousticWorld => {
                crate::gui::awe_view::draw_awe_view(
                    ctx,
                    &mut self.handle,
                    &mut self.awe_enabled,
                    &mut self.active_view,
                    &mut self.awe_ui,
                );
            }
        }

        // Dialogs
        self.show_dialogs(ctx);

        // Write current UI layout to MCP shared state
        #[cfg(feature = "mcp")]
        self.write_mcp_layout(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Stop audio
        if let Some(ref mut host) = self.host {
            let _ = host.stop();
        }
    }
}

impl SynthApp {
    /// Add a new module of the given category via session.
    fn add_module_of_category(&mut self, category: ModuleCategory) {
        let module_type = match category {
            ModuleCategory::Oscillator => TypedModuleType::Oscillator,
            ModuleCategory::Filter => TypedModuleType::Filter,
            ModuleCategory::Envelope => TypedModuleType::Envelope,
            ModuleCategory::LFO => TypedModuleType::Lfo,
            ModuleCategory::Amplifier => TypedModuleType::Amplifier,
            ModuleCategory::Mixer => TypedModuleType::Mixer,
            _ => return,
        };

        let Some((next_id, descriptor)) = self.session_add_module(module_type) else {
            return;
        };
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);
    }

    /// Add a voice module of the given type via session.
    fn add_voice_module_via_session(&mut self, module_type: TypedModuleType) {
        let Some((next_id, descriptor)) = self.session_add_module(module_type) else {
            return;
        };
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);
    }

    fn add_math_oscillator_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::MathOscillator);
    }

    fn add_sub_oscillator_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::SubOscillator);
    }

    fn add_noise_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::Noise);
    }

    fn add_mod_matrix_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::ModMatrix);
    }

    fn add_keyboard_panner_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::KeyboardPanner);
    }

    fn add_body_resonance_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::BodyResonance);
    }

    fn add_mechanical_noise_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::MechanicalNoise);
    }

    fn add_ring_mod_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::RingMod);
    }

    fn add_envelope_follower_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::EnvelopeFollower);
    }

    fn add_wavetable_osc_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::WavetableOsc);
    }

    fn add_mseg_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::Mseg);
    }

    fn add_additive_osc_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::AdditiveOsc);
    }

    fn add_euclidean_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::Euclidean);
    }

    fn add_turing_machine_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::TuringMachine);
    }

    fn add_random_gates_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::RandomGates);
    }

    fn add_granular_osc_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::GranularOsc);
    }

    fn add_kinetic_modulator_module(&mut self) {
        self.add_voice_module_via_session(TypedModuleType::KineticModulator);
    }

    fn add_signal_monitor_module(&mut self) {
        // SignalMonitor needs GUI-specific VisualizationBuffer — create directly
        let mut m = synth_modules::SignalMonitor::new();
        let descriptor = m.descriptor();

        // Use session for ID generation only (not add_module since it would create a second instance)
        let next_id = {
            use synth_core::ModuleType;
            let mut counters = self.session.counters_lock();
            let counter = counters
                .entry((self.active_instrument_id, ModuleType::SignalMonitor))
                .or_insert(0);
            *counter += 1;
            ModuleId::new(TypedModuleType::SignalMonitor, *counter)
        };
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

        // Create shared vis buffer and inject into module as trait object
        let buffer = std::sync::Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
        self.handle
            .add_visualization_buffer(next_id, buffer.clone());
        m.set_vis_sink(buffer);

        let module: Box<dyn synth_core::PolyModule> = Box::new(m);
        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    /// Handle a quick-add request: create module via session, place it, and auto-connect.
    fn handle_quick_add(
        session: &crate::session::SynthSession,
        handle: &mut EngineHandle,
        instrument_id: InstrumentId,
        editor: &mut PatchEditor,
        request: QuickAddRequest,
    ) {
        // Determine module type from selection
        let module_type: Option<TypedModuleType> = match request.selection {
            PaletteSelection::Category(category) => match category {
                ModuleCategory::Oscillator => Some(TypedModuleType::Oscillator),
                ModuleCategory::Filter => Some(TypedModuleType::Filter),
                ModuleCategory::Envelope => Some(TypedModuleType::Envelope),
                ModuleCategory::LFO => Some(TypedModuleType::Lfo),
                ModuleCategory::Amplifier => Some(TypedModuleType::Amplifier),
                ModuleCategory::Mixer => Some(TypedModuleType::Mixer),
                _ => None,
            },
            PaletteSelection::MathOscillator => Some(TypedModuleType::MathOscillator),
            PaletteSelection::SubOscillator => Some(TypedModuleType::SubOscillator),
            PaletteSelection::Noise => Some(TypedModuleType::Noise),
            PaletteSelection::WavetableOsc => Some(TypedModuleType::WavetableOsc),
            PaletteSelection::AdditiveOsc => Some(TypedModuleType::AdditiveOsc),
            PaletteSelection::GranularOsc => Some(TypedModuleType::GranularOsc),
            PaletteSelection::RingMod => Some(TypedModuleType::RingMod),
            PaletteSelection::EnvelopeFollower => Some(TypedModuleType::EnvelopeFollower),
            PaletteSelection::Mseg => Some(TypedModuleType::Mseg),
            PaletteSelection::KineticModulator => Some(TypedModuleType::KineticModulator),
            PaletteSelection::Euclidean => Some(TypedModuleType::Euclidean),
            PaletteSelection::TuringMachine => Some(TypedModuleType::TuringMachine),
            PaletteSelection::RandomGates => Some(TypedModuleType::RandomGates),
            PaletteSelection::Effect(effect_type) => Some(match effect_type {
                EffectType::Delay => TypedModuleType::Delay,
                EffectType::Reverb => TypedModuleType::Reverb,
                EffectType::Distortion => TypedModuleType::Distortion,
                EffectType::Chorus => TypedModuleType::Chorus,
                EffectType::Phaser => TypedModuleType::Phaser,
                EffectType::Flanger => TypedModuleType::Flanger,
                EffectType::Compressor => TypedModuleType::Compressor,
                EffectType::Eq => TypedModuleType::Eq,
                EffectType::Waveshaper => TypedModuleType::Waveshaper,
                EffectType::MidSide => TypedModuleType::MidSide,
                EffectType::BbdDelay => TypedModuleType::BbdDelay,
                EffectType::Limiter => TypedModuleType::Limiter,
                EffectType::Convolver => TypedModuleType::Convolver,
                EffectType::PhaseVocoder => TypedModuleType::PhaseVocoder,
                EffectType::FrequencyShifter => TypedModuleType::FrequencyShifter,
            }),
            PaletteSelection::SignalMonitor => {
                // SignalMonitor needs GUI-specific VisualizationBuffer
                let mut m = synth_modules::SignalMonitor::new();
                let d = m.descriptor();
                let id = {
                    let mut counters = session.counters_lock();
                    let counter = counters
                        .entry((instrument_id, synth_core::ModuleType::SignalMonitor))
                        .or_insert(0);
                    *counter += 1;
                    ModuleId::new(TypedModuleType::SignalMonitor, *counter)
                };
                let dc = d.clone();
                editor.add_module_at(id, d, request.position);

                let buffer =
                    std::sync::Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(id, buffer.clone());
                m.set_vis_sink(buffer);

                handle.send(EngineCommand::AddModuleInstance {
                    instrument_id: Some(instrument_id),
                    id,
                    module: Box::new(m),
                });
                // Continue to connection logic below
                Self::quick_add_connect(session, handle, instrument_id, editor, &request, id, &dc);
                return;
            }
            _ => None,
        };

        let Some(module_type) = module_type else {
            return;
        };

        let Ok((next_id, descriptor)) = session.add_module(instrument_id, module_type) else {
            return;
        };
        editor.add_module_at(next_id, descriptor.clone(), request.position);

        Self::quick_add_connect(
            session,
            handle,
            instrument_id,
            editor,
            &request,
            next_id,
            &descriptor,
        );
    }

    /// Create the auto-connection for a quick-add operation.
    fn quick_add_connect(
        session: &crate::session::SynthSession,
        _handle: &mut EngineHandle,
        instrument_id: InstrumentId,
        editor: &mut PatchEditor,
        request: &QuickAddRequest,
        next_id: ModuleId,
        descriptor: &synth_core::ModuleDescriptor,
    ) {
        use crate::gui::widgets::WidgetPortDirection;
        use synth_core::PortDirection;

        let needed_dir = match request.target_direction {
            WidgetPortDirection::Input => PortDirection::Output,
            WidgetPortDirection::Output => PortDirection::Input,
        };

        let Some(new_port_name) = descriptor
            .ports
            .iter()
            .find(|p| p.direction == needed_dir)
            .map(|p| p.name.clone())
        else {
            return;
        };

        let connection = match request.target_direction {
            WidgetPortDirection::Input => synth_engine::graph::Connection::new(
                next_id,
                &new_port_name,
                request.target_module,
                &request.target_port,
            ),
            WidgetPortDirection::Output => synth_engine::graph::Connection::new(
                request.target_module,
                &request.target_port,
                next_id,
                &new_port_name,
            ),
        };

        editor.add_connection(connection);
        let _ = session.connect(
            instrument_id,
            connection.from_module,
            connection.from_port.to_string(),
            connection.to_module,
            connection.to_port.to_string(),
        );
    }

    fn add_effect_module(&mut self, effect_type: EffectType) {
        let module_type = match effect_type {
            EffectType::Delay => TypedModuleType::Delay,
            EffectType::Reverb => TypedModuleType::Reverb,
            EffectType::Distortion => TypedModuleType::Distortion,
            EffectType::Chorus => TypedModuleType::Chorus,
            EffectType::Phaser => TypedModuleType::Phaser,
            EffectType::Flanger => TypedModuleType::Flanger,
            EffectType::Compressor => TypedModuleType::Compressor,
            EffectType::Eq => TypedModuleType::Eq,
            EffectType::Waveshaper => TypedModuleType::Waveshaper,
            EffectType::MidSide => TypedModuleType::MidSide,
            EffectType::BbdDelay => TypedModuleType::BbdDelay,
            EffectType::Limiter => TypedModuleType::Limiter,
            EffectType::Convolver => TypedModuleType::Convolver,
            EffectType::PhaseVocoder => TypedModuleType::PhaseVocoder,
            EffectType::FrequencyShifter => TypedModuleType::FrequencyShifter,
        };

        let Some((next_id, descriptor)) = self.session_add_module(module_type) else {
            return;
        };
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);
    }

    fn add_visualizer_module(&mut self, viz_type: PaletteVisualizerType) {
        let (descriptor, module_type) = match viz_type {
            PaletteVisualizerType::Oscilloscope => (
                Oscilloscope::new().descriptor(),
                TypedModuleType::Oscilloscope,
            ),
            PaletteVisualizerType::LevelMeter => {
                (LevelMeter::new().descriptor(), TypedModuleType::LevelMeter)
            }
            PaletteVisualizerType::SpectrumAnalyzer => (
                SpectrumAnalyzer::new().descriptor(),
                TypedModuleType::SpectrumAnalyzer,
            ),
        };

        // Visualizers need GUI-specific VisualizationBuffer — use session for ID only
        let next_id = {
            use synth_core::ModuleType;
            let mt = match viz_type {
                PaletteVisualizerType::Oscilloscope => ModuleType::Oscilloscope,
                PaletteVisualizerType::LevelMeter => ModuleType::LevelMeter,
                PaletteVisualizerType::SpectrumAnalyzer => ModuleType::SpectrumAnalyzer,
            };
            let mut counters = self.session.counters_lock();
            let counter = counters.entry((self.active_instrument_id, mt)).or_insert(0);
            *counter += 1;
            ModuleId::new(module_type, *counter)
        };
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

        // Create shared visualization buffer wrapped in Arc
        let buffer = std::sync::Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));

        // Store Arc clone in our handle for GUI access (same buffer!)
        self.handle
            .add_visualization_buffer(next_id, buffer.clone());

        // Convert GUI PaletteVisualizerType to engine VisualizerType
        let engine_viz_type = match viz_type {
            PaletteVisualizerType::Oscilloscope => {
                synth_engine::commands::VisualizerType::Oscilloscope
            }
            PaletteVisualizerType::LevelMeter => synth_engine::commands::VisualizerType::LevelMeter,
            PaletteVisualizerType::SpectrumAnalyzer => {
                synth_engine::commands::VisualizerType::SpectrumAnalyzer
            }
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
        let Some((next_id, descriptor)) = self.session_add_module(TypedModuleType::StereoOutput)
        else {
            return;
        };
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);
    }

    /// Add an effect to the master bus.
    fn add_master_effect(&mut self, effect_type: EffectType) {
        // Create effect in GUI thread (real-time safe allocation)
        let (effect, module_type): (Box<dyn synth_core::AudioEffect>, TypedModuleType) =
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
                EffectType::Waveshaper => {
                    (Box::new(Waveshaper::new()), TypedModuleType::Waveshaper)
                }
                EffectType::MidSide => (Box::new(MidSide::new()), TypedModuleType::MidSide),
                EffectType::BbdDelay => (Box::new(BbdDelay::new()), TypedModuleType::BbdDelay),
                EffectType::Limiter => (Box::new(Limiter::new()), TypedModuleType::Limiter),
                EffectType::Convolver => (Box::new(Convolver::new()), TypedModuleType::Convolver),
                EffectType::PhaseVocoder => {
                    (Box::new(PhaseVocoder::new()), TypedModuleType::PhaseVocoder)
                }
                EffectType::FrequencyShifter => (
                    Box::new(FrequencyShifter::new()),
                    TypedModuleType::FrequencyShifter,
                ),
            };

        // Master effects use session counters for ID but send directly (instrument_id: None)
        let next_id = {
            let mut counters = self.session.counters_lock();
            let counter = counters
                .entry((InstrumentId::MASTER, module_type))
                .or_insert(0);
            *counter += 1;
            ModuleId::new(module_type, *counter)
        };

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
            ui.label(
                RichText::new("OUTPUT")
                    .color(theme().colors.text_dim)
                    .small(),
            );
            ui.add_space(4.0);

            let (peak_l, peak_r) = self.handle.peak_meters();
            let (rms_l, rms_r) = self.handle.rms_meters();
            let meter_width = ui.available_width() - 30.0;

            // Left channel - horizontal
            ui.horizontal(|ui| {
                ui.label(RichText::new("L").color(theme().colors.text_dim).size(10.0));
                draw_meter_horizontal(ui, peak_l, rms_l, meter_width, 12.0);
            });

            // Right channel - horizontal
            ui.horizontal(|ui| {
                ui.label(RichText::new("R").color(theme().colors.text_dim).size(10.0));
                draw_meter_horizontal(ui, peak_r, rms_r, meter_width, 12.0);
            });

            // dB readout
            ui.horizontal(|ui| {
                let db_l = 20.0 * peak_l.max(0.0001).log10();
                let db_r = 20.0 * peak_r.max(0.0001).log10();
                ui.label(
                    RichText::new(format!("{:+.1} / {:+.1} dB", db_l, db_r))
                        .color(theme().colors.text_dim)
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
            ui.label(
                RichText::new("MASTER FX")
                    .color(theme().colors.text_dim)
                    .small(),
            );
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
                        // Effect header frame - use bg_module (lighter) for contrast with sliders
                        let frame_color = if *is_bypassed {
                            theme().colors.bg_module.gamma_multiply(0.7)
                        } else {
                            theme().colors.bg_module
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
                                                    .color(theme().colors.text_dim)
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
                                        theme().colors.text_dim
                                    } else {
                                        theme().colors.text_primary
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
                                                            .color(theme().colors.text_dim)
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
                                                theme().colors.accent_yellow
                                            } else {
                                                theme().colors.text_dim
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
                        (EffectType::Waveshaper, "Waveshaper"),
                        (EffectType::MidSide, "Mid/Side"),
                        (EffectType::BbdDelay, "BBD Delay"),
                        (EffectType::Limiter, "Limiter"),
                        (EffectType::FrequencyShifter, "Freq Shifter"),
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
        // Always use CH1 for keyboard input - focused_instrument handles routing
        let active_channel = MidiChannel::CH1;

        // Header row: PANIC, Playing instrument, KEYBOARD label, Octave, Center
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(
                    RichText::new("PANIC").color(theme().colors.accent_red),
                ))
                .clicked()
            {
                self.handle.send(EngineCommand::AllNotesOff);
                self.pressed_keys.clear();
            }

            let active_name = self
                .instruments
                .iter()
                .find(|p| p.id == self.active_instrument_id)
                .map(|p| p.name.as_str())
                .unwrap_or("Instrument 1");
            ui.separator();
            ui.label(
                RichText::new(format!("Playing: {}", active_name))
                    .color(theme().colors.accent_orange),
            );

            ui.separator();
            self.keyboard.show_header(ui);
        });

        // Layout: [Left Scope] [Piano Keys] [Right Scope]
        let available_width = ui.available_width();
        let piano_width = 52.0 * 24.0; // 52 white keys × 24px
        let remaining = available_width - piano_width;
        let min_scope_width = 60.0;
        let show_scopes = remaining > min_scope_width * 2.0;

        let (samples_l, samples_r) = if show_scopes {
            self.handle.state.master_scope.read_samples()
        } else {
            (Vec::new(), Vec::new())
        };

        let keys_height = 110.0;

        ui.horizontal(|ui| {
            if show_scopes {
                let scope_width = remaining / 2.0;
                draw_oscilloscope(
                    ui,
                    &samples_l,
                    scope_width,
                    keys_height,
                    1.0,
                    theme().colors.accent_cyan,
                );
            }

            let piano_max = available_width.min(piano_width + 10.0);
            ui.allocate_ui(Vec2::new(piano_max, keys_height), |ui| {
                let event = self.keyboard.show_keys(ui);

                if let Some(note) = event.note_on {
                    self.handle
                        .note_on_channel(note, Velocity::new(0.8), active_channel);
                }
                for note in event.note_off {
                    self.handle.note_off_channel(note, active_channel);
                }
            });

            if show_scopes {
                let scope_width = remaining / 2.0;
                draw_oscilloscope(
                    ui,
                    &samples_r,
                    scope_width,
                    keys_height,
                    1.0,
                    theme().colors.meter_green,
                );
            }
        });
    }

    fn process_keyboard_input(&mut self, ctx: &egui::Context) {
        // Always use CH1 for keyboard input - focused_instrument handles routing
        let active_channel = MidiChannel::CH1;

        handle_keyboard_input(
            ctx,
            &mut self.handle,
            &mut self.keyboard,
            &mut self.pressed_keys,
            active_channel,
        );
    }

    fn show_dialogs(&mut self, ctx: &egui::Context) {
        // Update dialog state (clears expired status messages)
        self.dialog_state.update();

        // Settings dialog
        show_settings_dialog(
            ctx,
            &mut self.dialog_state.show_settings,
            &mut self.dialog_state.current_theme,
        );

        // About dialog
        show_about_dialog(ctx, &mut self.dialog_state.show_about);

        // Load built-in patch dialog
        match show_load_patch_dialog(ctx, &mut self.dialog_state.show_load_patch) {
            LoadPatchResult::LoadBuiltin(patch) => {
                self.load_patch_data(&patch);
                self.current_patch_name = patch.name.clone();
                self.dialog_state
                    .set_status(format!("Loaded: {}", patch.name));
            }
            LoadPatchResult::Cancelled | LoadPatchResult::None => {}
        }

        // File dialog (open/save/import)
        if let Some(result) = self.dialog_state.update_file_dialog(ctx) {
            match result {
                FileDialogResult::Picked(path, Some(FileDialogMode::OpenPatch)) => {
                    match Patch::load(&path) {
                        Ok(patch) => {
                            self.load_patch_data(&patch);
                            self.current_patch_name = patch.name.clone();
                            self.current_patch_path = Some(path.clone());
                            self.dialog_state
                                .set_status(format!("Loaded: {}", path.display()));
                        }
                        Err(e) => {
                            self.dialog_state.set_status(format!("Error loading: {e}"));
                        }
                    }
                }
                FileDialogResult::Saved(path, Some(FileDialogMode::SavePatch)) => {
                    if let Some(patch) = self.create_patch_from_rack() {
                        if let Err(e) = patch.save(&path) {
                            self.dialog_state.set_status(format!("Error saving: {e}"));
                        } else {
                            // Extract name from path
                            let name = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("patch")
                                .to_string();
                            self.current_patch_name = name;
                            self.current_patch_path = Some(path.clone());
                            self.dialog_state
                                .set_status(format!("Saved: {}", path.display()));
                        }
                    }
                }
                _ => {}
            }
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
        let Some(patch_editor) = self
            .instruments
            .iter_mut()
            .find(|i| i.id == active_id)
            .map(|i| &mut i.patch_editor)
        else {
            eprintln!("Warning: Cannot load patch - no active instrument found");
            return;
        };

        patch_bridge::load_patch(
            patch,
            patch_editor,
            &self.session,
            &mut self.handle,
            &mut self.keyboard,
            &mut self.glide_time,
            active_id,
        );

        // Auto-layout after loading a new patch
        patch_editor.request_initial_layout();

        // Restore AWE UI state from loaded patch
        if let Some(awe) = &patch.settings.awe {
            self.awe_enabled = awe.enabled;
            self.awe_ui.restore_from(awe);
        } else {
            self.awe_enabled = false;
            self.awe_ui = crate::gui::awe_view::AweUiState::default();
        }
    }

    /// Reconcile GUI state with session: detect modules added/removed by MCP.
    ///
    /// Loops over ALL instruments (not just the active one) so that modules
    /// and connections created by MCP on any instrument appear immediately
    /// when the user switches to that instrument.
    #[cfg(feature = "mcp")]
    fn reconcile_with_session(&mut self) {
        // --- Instrument-level reconciliation ---
        // Detect instruments added/removed/changed by MCP.
        self.reconcile_instruments();

        // --- Module-level reconciliation (all instruments) ---
        let instrument_ids: Vec<InstrumentId> = self.instruments.iter().map(|i| i.id).collect();

        for inst_id in instrument_ids {
            let session_modules = self.session.all_modules_for_instrument(inst_id);
            let engine_connections = self
                .session
                .state()
                .shared_graph
                .get_connections_for_instrument(inst_id);

            let Some(patch_editor) = self
                .instruments
                .iter_mut()
                .find(|i| i.id == inst_id)
                .map(|i| &mut i.patch_editor)
            else {
                continue;
            };

            let editor_ids: std::collections::HashSet<ModuleId> =
                patch_editor.module_ids().into_iter().collect();
            let session_ids: std::collections::HashSet<ModuleId> =
                session_modules.keys().copied().collect();

            // Modules added by MCP (in session but not in editor)
            let to_add: Vec<(ModuleId, synth_core::ModuleDescriptor)> = session_ids
                .difference(&editor_ids)
                .filter_map(|id| session_modules.get(id).map(|d| (*id, d.clone())))
                .collect();

            // Modules removed by MCP (in editor but not in session)
            let to_remove: Vec<ModuleId> = editor_ids
                .difference(&session_ids)
                .copied()
                // Skip visualizers — they are GUI-only and not tracked by session
                .filter(|id| {
                    !patch_editor
                        .module_descriptor(*id)
                        .is_some_and(|d| d.category == synth_core::ModuleCategory::Visualizer)
                })
                .collect();

            // Check connections too
            let editor_connections = patch_editor.connections().to_vec();
            let new_connections: Vec<synth_engine::graph::Connection> = engine_connections
                .iter()
                .filter_map(|snap| {
                    let conn = synth_engine::graph::Connection::new(
                        snap.from_module,
                        &snap.from_port,
                        snap.to_module,
                        &snap.to_port,
                    );
                    if !editor_connections.contains(&conn) {
                        Some(conn)
                    } else {
                        None
                    }
                })
                .collect();

            if to_add.is_empty() && to_remove.is_empty() && new_connections.is_empty() {
                continue;
            }

            for (module_id, descriptor) in to_add {
                let position = eframe::egui::Pos2::new(100.0, 100.0);
                patch_editor.add_module_at(module_id, descriptor, position);
            }

            for module_id in to_remove {
                patch_editor.remove_module(module_id);
            }

            for conn in new_connections {
                patch_editor.add_connection(conn);
            }
        }
    }

    /// Reconcile GUI instrument list with engine's `instrument_snapshots`.
    ///
    /// Detects instruments added or removed by MCP and syncs the GUI.
    #[cfg(feature = "mcp")]
    fn reconcile_instruments(&mut self) {
        let snapshots = self.session.list_instruments();

        let gui_ids: std::collections::HashSet<InstrumentId> =
            self.instruments.iter().map(|i| i.id).collect();
        let engine_ids: std::collections::HashSet<InstrumentId> =
            snapshots.iter().map(|s| s.id).collect();

        // Instruments added by MCP (in engine but not in GUI)
        for snap in &snapshots {
            if !gui_ids.contains(&snap.id) {
                let channel =
                    MidiChannel::from_one_indexed(snap.midi_channel).unwrap_or(MidiChannel::CH1);
                let mut ui_inst = InstrumentUiState::new(snap.id, &snap.name).with_channel(channel);
                ui_inst.volume = Gain::new(snap.volume);
                ui_inst.pan = BipolarValue::new(snap.pan);
                ui_inst.muted = snap.muted;
                ui_inst.solo = snap.solo;
                self.instruments.push(ui_inst);

                // Keep next_instrument_id above any MCP-created IDs
                let id_val = snap.id.as_u64() + 1;
                if id_val > self.next_instrument_id {
                    self.next_instrument_id = id_val;
                }
            }
        }

        // Instruments removed by MCP (in GUI but not in engine)
        // Don't remove the default instrument (ID 0) as a safety measure
        self.instruments
            .retain(|i| engine_ids.contains(&i.id) || i.id == InstrumentId::FIRST);

        // If active instrument was removed, switch to first available
        if !self
            .instruments
            .iter()
            .any(|i| i.id == self.active_instrument_id)
            && let Some(first) = self.instruments.first()
        {
            self.active_instrument_id = first.id;
            self.handle
                .set_focused_instrument(Some(self.active_instrument_id));
        }

        // Update metadata for existing instruments (name, volume, pan, mute, solo)
        for snap in &snapshots {
            if let Some(ui_inst) = self.instruments.iter_mut().find(|i| i.id == snap.id) {
                ui_inst.name = snap.name.clone();
                // Only update volume/pan/mute/solo if not currently being edited by GUI
                // For now, always sync from engine (MCP is source of truth when changed)
                if !ui_inst.muted {
                    ui_inst.volume = Gain::new(snap.volume);
                }
                ui_inst.pan = BipolarValue::new(snap.pan);
                ui_inst.muted = snap.muted;
                ui_inst.solo = snap.solo;
            }
        }
    }

    /// Write the current UI layout to MCP shared state (called each frame).
    #[cfg(feature = "mcp")]
    fn write_mcp_layout(&self, ctx: &egui::Context) {
        let Some(shared) = &self.mcp_shared else {
            return;
        };
        let Some(patch_editor) = self.active_patch_editor_ref() else {
            return;
        };

        let rect = ctx.input(|i| i.viewport_rect());
        let window_size = (rect.width(), rect.height());

        let mut modules = Vec::new();
        for module_id in patch_editor.module_ids() {
            if let Some((descriptor, pos, param_values)) = patch_editor.get_module_data(module_id) {
                let panel_size = patch_editor
                    .module_panel_size(module_id)
                    .unwrap_or(egui::Vec2::new(250.0, 200.0));
                let params: Vec<(String, String)> = param_values
                    .iter()
                    .map(|(name, val)| (name.clone(), format!("{val:.3}")))
                    .collect();
                modules.push(crate::mcp_shared::ModuleLayout {
                    id: module_id.to_string(),
                    module_type: descriptor.type_id.0.clone(),
                    name: descriptor.name.clone(),
                    position: (pos.x, pos.y),
                    size: (panel_size.x, panel_size.y),
                    parameters: params,
                });
            }
        }

        let connections: Vec<crate::mcp_shared::ConnectionLayout> = patch_editor
            .connections()
            .iter()
            .map(|c| crate::mcp_shared::ConnectionLayout {
                from_module: c.from_module.to_string(),
                from_port: c.from_port.to_string(),
                to_module: c.to_module.to_string(),
                to_port: c.to_port.to_string(),
            })
            .collect();

        if let Ok(mut layout) = shared.ui_layout.lock() {
            *layout = crate::mcp_shared::UiLayoutData {
                patch_name: self.current_patch_name.clone(),
                modules,
                connections,
                window_size,
            };
        }
    }

    /// Reset the active instrument to a new empty patch.
    /// Clears all modules and adds a default StereoOutput for immediate sound.
    fn reset_to_new_patch(&mut self) {
        // 1. Clear active instrument's GUI state
        let active_id = self.active_instrument_id;

        // Remove visualizer buffers before clearing (session doesn't track these)
        if let Some(patch_editor) = self
            .instruments
            .iter()
            .find(|i| i.id == active_id)
            .map(|i| &i.patch_editor)
        {
            for module_id in patch_editor.module_ids() {
                let category = patch_editor
                    .module_descriptor(module_id)
                    .map(|d| d.category);
                if matches!(category, Some(ModuleCategory::Visualizer)) {
                    self.handle.send_blocking(EngineCommand::RemoveVisualizer {
                        instrument_id: Some(active_id),
                        id: module_id,
                    });
                    self.handle.remove_visualization_buffer(module_id);
                }
            }
        }

        // Clear all non-visualizer modules via session
        let _ = self.session.clear_graph(active_id);

        // Clear the patch editor GUI state
        if let Some(editor) = self.active_patch_editor() {
            editor.clear();
        }
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
        let editor = self.active_patch_editor_ref()?;
        patch_bridge::create_patch_from_rack(
            &self.dialog_state.patch_save_name,
            editor,
            &self.keyboard,
            &self.handle,
            self.glide_time,
            self.awe_enabled,
            &self.awe_ui,
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
    use synth_core::{Decibels, Hertz, Milliseconds, NormalizedValue, Ratio, Seconds};

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
                        ui.label(
                            RichText::new("Thresh")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}dB", threshold))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Ratio")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.1}:1", ratio))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Attack")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.1}ms", attack))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Release")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}ms", release))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Makeup")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.1}dB", makeup))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Low")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.1}dB", low_gain))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Mid")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.1}dB", mid_gain))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("High")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.1}dB", high_gain))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Size")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", room_size * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Damp")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", damping * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Width")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", width * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Time")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.2}s", time))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Feedback")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", feedback * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Rate")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.2}Hz", rate))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Depth")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", depth * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Rate")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.2}Hz", rate))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Depth")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", depth * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Feedback")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", feedback * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::Phaser(PhaserParam::Feedback(synth_core::BipolarValue::new(
                                val,
                            ))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Rate")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.2}Hz", rate))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Depth")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", depth * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Feedback")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", feedback * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::Flanger(FlangerParam::Feedback(synth_core::BipolarValue::new(
                                val,
                            ))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Drive")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", drive * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Tone")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", tone * 100.0))
                                    .color(theme().colors.text_secondary)
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
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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

            MasterEffectParams::Waveshaper { drive, mix, bias } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Waveshaper {
                    drive: dr,
                    mix: mx,
                    bias: bi,
                } = &mut effect.params
                {
                    // Drive
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Drive")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", drive * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::Waveshaper(WaveshaperParam::Drive(NormalizedValue::new(val))),
                        ));
                    }

                    // Bias
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Bias")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.2}", bias))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *bias;
                    if ui
                        .add(egui::Slider::new(&mut val, -1.0..=1.0).show_value(false))
                        .changed()
                    {
                        *bi = val;
                        param_changes.push((
                            effect_type,
                            Param::Waveshaper(WaveshaperParam::Bias(
                                synth_core::BipolarValue::new(val),
                            )),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::Waveshaper(WaveshaperParam::Mix(NormalizedValue::new(val))),
                        ));
                    }
                }
            }

            MasterEffectParams::MidSide {
                width,
                mid_gain,
                side_gain,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::MidSide {
                    width: w,
                    mid_gain: mg,
                    side_gain: sg,
                    mix: mx,
                } = &mut effect.params
                {
                    // Width
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Width")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let actual_width = width * 2.0;
                            ui.label(
                                RichText::new(format!("{actual_width:.1}"))
                                    .color(theme().colors.text_secondary)
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
                            Param::MidSide(MidSideParam::Width(NormalizedValue::new(val))),
                        ));
                    }

                    // Mid Gain
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Mid Gain")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{mid_gain:+.1} dB"))
                                    .color(theme().colors.text_secondary)
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
                        param_changes.push((
                            effect_type,
                            Param::MidSide(MidSideParam::MidGain(synth_core::Decibels::new(val))),
                        ));
                    }

                    // Side Gain
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Side Gain")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{side_gain:+.1} dB"))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *side_gain;
                    if ui
                        .add(egui::Slider::new(&mut val, -12.0..=12.0).show_value(false))
                        .changed()
                    {
                        *sg = val;
                        param_changes.push((
                            effect_type,
                            Param::MidSide(MidSideParam::SideGain(synth_core::Decibels::new(val))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::MidSide(MidSideParam::Mix(NormalizedValue::new(val))),
                        ));
                    }
                }
            }

            MasterEffectParams::BbdDelay {
                time,
                feedback,
                tone,
                wow_flutter,
                clock_noise,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::BbdDelay {
                    time: t,
                    feedback: fb,
                    tone: tn,
                    wow_flutter: wf,
                    clock_noise: cn,
                    mix: mx,
                } = &mut effect.params
                {
                    // Time
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Time")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}ms", time * 1000.0))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *time;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.01..=1.0).show_value(false))
                        .changed()
                    {
                        *t = val;
                        param_changes.push((
                            effect_type,
                            Param::BbdDelay(synth_core::BbdDelayParam::Time(Seconds::new(val))),
                        ));
                    }

                    // Feedback
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Feedback")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", feedback * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::BbdDelay(synth_core::BbdDelayParam::Feedback(
                                NormalizedValue::new(val),
                            )),
                        ));
                    }

                    // Tone
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Tone")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", tone * 100.0))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *tone;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *tn = val;
                        param_changes.push((
                            effect_type,
                            Param::BbdDelay(synth_core::BbdDelayParam::Tone(NormalizedValue::new(
                                val,
                            ))),
                        ));
                    }

                    // Wow & Flutter
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("W&F")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", wow_flutter * 100.0))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *wow_flutter;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *wf = val;
                        param_changes.push((
                            effect_type,
                            Param::BbdDelay(synth_core::BbdDelayParam::WowFlutter(
                                NormalizedValue::new(val),
                            )),
                        ));
                    }

                    // Clock Noise
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Noise")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", clock_noise * 100.0))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *clock_noise;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *cn = val;
                        param_changes.push((
                            effect_type,
                            Param::BbdDelay(synth_core::BbdDelayParam::ClockNoise(
                                NormalizedValue::new(val),
                            )),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::BbdDelay(synth_core::BbdDelayParam::Mix(NormalizedValue::new(
                                val,
                            ))),
                        ));
                    }
                }
            }

            MasterEffectParams::Limiter {
                ceiling,
                look_ahead,
                release,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Limiter {
                    ceiling: c,
                    look_ahead: la,
                    release: rel,
                    mix: mx,
                } = &mut effect.params
                {
                    // Ceiling
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Ceiling")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{ceiling:.1} dB"))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *ceiling;
                    if ui
                        .add(egui::Slider::new(&mut val, -12.0..=0.0).show_value(false))
                        .changed()
                    {
                        *c = val;
                        param_changes.push((
                            effect_type,
                            Param::Limiter(synth_core::LimiterParam::Ceiling(Decibels::new(val))),
                        ));
                    }

                    // Look-ahead
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Look-ahead")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{look_ahead:.1}ms"))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *look_ahead;
                    if ui
                        .add(egui::Slider::new(&mut val, 1.0..=5.0).show_value(false))
                        .changed()
                    {
                        *la = val;
                        param_changes.push((
                            effect_type,
                            Param::Limiter(synth_core::LimiterParam::LookAhead(Milliseconds::new(
                                val,
                            ))),
                        ));
                    }

                    // Release
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Release")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{release:.0}ms"))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *release;
                    if ui
                        .add(egui::Slider::new(&mut val, 10.0..=500.0).show_value(false))
                        .changed()
                    {
                        *rel = val;
                        param_changes.push((
                            effect_type,
                            Param::Limiter(synth_core::LimiterParam::Release(Milliseconds::new(
                                val,
                            ))),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::Limiter(synth_core::LimiterParam::Mix(NormalizedValue::new(
                                val,
                            ))),
                        ));
                    }
                }
            }
            MasterEffectParams::Convolver {
                ir_type: _,
                pre_delay,
                decay_trim,
                brightness,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::Convolver {
                    ir_type: _it,
                    pre_delay: pd,
                    decay_trim: dt,
                    brightness: br,
                    mix: mx,
                } = &mut effect.params
                {
                    // Pre-Delay
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Pre-Delay")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}ms", pre_delay))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *pre_delay;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=200.0).show_value(false))
                        .changed()
                    {
                        *pd = val;
                        param_changes.push((
                            effect_type,
                            Param::Convolver(synth_core::ConvolverParam::PreDelay(
                                Milliseconds::new(val),
                            )),
                        ));
                    }

                    // Decay
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Decay")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", decay_trim * 100.0))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *decay_trim;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.1..=1.0).show_value(false))
                        .changed()
                    {
                        *dt = val;
                        param_changes.push((
                            effect_type,
                            Param::Convolver(synth_core::ConvolverParam::DecayTrim(
                                NormalizedValue::new(val),
                            )),
                        ));
                    }

                    // Brightness
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Bright")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", brightness * 100.0))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *brightness;
                    if ui
                        .add(egui::Slider::new(&mut val, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        *br = val;
                        param_changes.push((
                            effect_type,
                            Param::Convolver(synth_core::ConvolverParam::Brightness(
                                NormalizedValue::new(val),
                            )),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::Convolver(synth_core::ConvolverParam::Mix(
                                NormalizedValue::new(val),
                            )),
                        ));
                    }
                }
            }
            MasterEffectParams::PhaseVocoder {
                pitch_shift,
                freeze: _,
                fft_size: _,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::PhaseVocoder {
                    pitch_shift: ps,
                    freeze: _fr,
                    fft_size: _fs,
                    mix: mx,
                } = &mut effect.params
                {
                    // Pitch Shift
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Pitch")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.1}st", pitch_shift))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *pitch_shift;
                    if ui
                        .add(egui::Slider::new(&mut val, -24.0..=24.0).show_value(false))
                        .changed()
                    {
                        *ps = val;
                        param_changes.push((
                            effect_type,
                            Param::PhaseVocoder(synth_core::PhaseVocoderParam::PitchShift(
                                synth_core::Semitones(val),
                            )),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::PhaseVocoder(synth_core::PhaseVocoderParam::Mix(
                                NormalizedValue::new(val),
                            )),
                        ));
                    }
                }
            }
            MasterEffectParams::FrequencyShifter {
                shift,
                mode: _,
                mix,
            } => {
                let effect = &mut effects[idx];
                if let MasterEffectParams::FrequencyShifter {
                    shift: sh,
                    mode: _md,
                    mix: mx,
                } = &mut effect.params
                {
                    // Shift
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Shift")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:+.1}Hz", shift))
                                    .color(theme().colors.text_secondary)
                                    .size(9.0),
                            );
                        });
                    });
                    let mut val = *shift;
                    if ui
                        .add(egui::Slider::new(&mut val, -1000.0..=1000.0).show_value(false))
                        .changed()
                    {
                        *sh = val;
                        param_changes.push((
                            effect_type,
                            Param::FrequencyShifter(synth_core::FrequencyShifterParam::Shift(
                                Hertz::new(val),
                            )),
                        ));
                    }

                    // Mix
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Mix")
                                .color(theme().colors.text_dim)
                                .size(9.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}%", mix * 100.0))
                                    .color(theme().colors.text_secondary)
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
                            Param::FrequencyShifter(synth_core::FrequencyShifterParam::Mix(
                                NormalizedValue::new(val),
                            )),
                        ));
                    }
                }
            }
        }
    });
}
