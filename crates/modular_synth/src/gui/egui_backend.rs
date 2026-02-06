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
};
use crate::gui::theme::theme;
use crate::gui::views::{MasterEffectParams, MasterEffectUiState, draw_meter_horizontal};
use crate::gui::{GuiBackend, GuiResult, SynthGuiConfig};
use crate::io::MidiHandler;
use crate::io::import;
use crate::patch::{Patch, example_patches};
use synth_core::{
    AmplifierParam, ChorusParam, CompressorParam, DelayParam, DistortionParam, EnvelopeParam,
    EqParam, FlangerParam, Param, PhaserParam, ReverbParam, SamplePlayerParam,
};
use synth_core::{Describable, ModuleCategory, PolyModule};
use synth_core::{FadeoutRate, Gain, LoopMode, NormalizedValue, ReleaseMode, Seconds, Velocity};
use synth_engine::ModuleType as TypedModuleType;
use synth_engine::commands::PortId;
use synth_engine::graph::Connection;
use synth_engine::instrument::{Instrument, InstrumentId, MidiChannel};
use synth_engine::visualizers::{LevelMeter, Oscilloscope};
use synth_engine::{
    AllocationMode, AllocatorConfig, EngineCommand, EngineEvent, EngineHandle, ModuleId,
    SynthEngine,
};
use synth_modules::effects::{Chorus, Compressor, Delay, Distortion, Eq, Flanger, Phaser, Reverb};
use synth_modules::{
    Amplifier, Envelope, Filter, Lfo, MathOscillator, Mixer, MultiPointEnvelope, NoiseGenerator,
    Oscillator, SamplePlayer, StereoOutput, SubOscillator,
};
use synth_sequencer::Song;

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

    // Navigation state
    active_view: AppView,

    // Tracker/Sequencer view state
    tracker_state: synth_sequencer::view::TrackerViewState,

    // Loaded song for sequencer
    song: Option<Song>,

    // Pending import file (processed on first update)
    pending_import: Option<PathBuf>,
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
        let startup_patch = crate::patches::default_patch();
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
            active_view: AppView::default(),
            tracker_state: synth_sequencer::view::TrackerViewState::default(),
            song: None,
            pending_import: config.import_file,
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
        // Handle pending import from CLI (first update only)
        if let Some(path) = self.pending_import.take() {
            self.import_song_file(&path);
        }

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
                    if ui.button("🎵 Import Song...").clicked() {
                        self.dialog_state.open_import_song_dialog();
                        ui.close();
                    }
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
                        ui.separator();
                        if ui.button("🔄 Refresh").clicked() {
                            ui.close();
                        }
                    });
                    ui.separator();
                    // Current patch name
                    ui.label(
                        RichText::new(format!("Patch: {}", self.current_patch_name))
                            .color(theme().colors.accent_cyan),
                    );
                    ui.separator();

                    // View selector tabs (rightmost, before status)
                    for view in [AppView::Mixer, AppView::Sequencer, AppView::Rack] {
                        let is_selected = self.active_view == view;
                        let label = format!("{} {}", view.icon(), view.label());
                        let text = if is_selected {
                            RichText::new(label).color(theme().colors.accent_primary)
                        } else {
                            RichText::new(label).color(theme().colors.text_secondary)
                        };
                        if ui.selectable_label(is_selected, text).clicked() {
                            self.active_view = view;
                            // Manage focused instrument based on view
                            match view {
                                AppView::Sequencer => {
                                    // Clear focused instrument so all instruments play
                                    self.handle.set_focused_instrument(None);
                                }
                                AppView::Rack => {
                                    // Set focused instrument to active one for solo preview
                                    self.handle
                                        .set_focused_instrument(Some(self.active_instrument_id));
                                }
                                _ => {}
                            }
                        }
                    }
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
                            PaletteSelection::Effect(effect_type) => {
                                self.add_effect_module(effect_type);
                            }
                            PaletteSelection::Visualizer(viz_type) => {
                                self.add_visualizer_module(viz_type);
                            }
                            PaletteSelection::StereoOutput => {
                                self.add_stereo_output_module();
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

        // Main content - view routing
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
                patch_editor.add_connection(connection);

                // Send Connect command to engine (active instrument's voice graph)
                self.handle.send(EngineCommand::Connect {
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

            // Handle auto-layout request
            if result.request_auto_layout
                && let Some(canvas_rect) = result.canvas_rect
            {
                patch_editor.apply_auto_layout(canvas_rect);
            }
                });
            }
            AppView::Sequencer => {
                // Get playback position from engine
                let playback_tick = if self.handle.is_playing() {
                    Some(synth_sequencer::Tick(self.handle.playback_ticks()))
                } else {
                    None
                };

                // Show sequencer view with tracker state and loaded song
                let result = crate::gui::views::sequencer::show(
                    ctx,
                    &mut self.tracker_state,
                    self.song.as_ref(),
                    playback_tick,
                );

                // Handle transport actions
                if let Some(action) = result.transport {
                    use crate::gui::views::sequencer::TransportAction;
                    match action {
                        TransportAction::Play => {
                            // Start from current pattern position
                            if let Some(pattern_id) = self.tracker_state.active_pattern {
                                self.handle
                                    .send(EngineCommand::PlayFromPattern { pattern_id });
                            } else {
                                self.handle.send(EngineCommand::Play);
                            }
                        }
                        TransportAction::Stop => {
                            self.handle.send(EngineCommand::Stop);
                        }
                        TransportAction::Rewind => {
                            self.handle.send(EngineCommand::Rewind);
                        }
                        TransportAction::PlayPattern => {
                            // Loop only the active pattern
                            if let Some(pattern_id) = self.tracker_state.active_pattern {
                                self.handle.send(EngineCommand::PlayPattern { pattern_id });
                            }
                        }
                    }
                }

                // Handle seek requests (pattern navigation during playback)
                if let Some(tick) = result.seek_to {
                    self.handle.send(EngineCommand::Seek { tick });
                }

                // Handle solo track changes
                if let Some(solo) = result.solo_track_changed {
                    self.handle.set_solo_track(solo);
                }
            }
            AppView::Mixer => {
                crate::gui::views::mixer::show(ctx);
            }
        }

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
        let (module, descriptor, module_type, envelope_pos): (
            Box<dyn synth_core::PolyModule>,
            _,
            TypedModuleType,
            Option<std::sync::Arc<synth_modules::EnvelopePositionBuffer>>,
        ) = match category {
            ModuleCategory::Oscillator => {
                let m = Oscillator::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Oscillator, None)
            }
            ModuleCategory::Filter => {
                let m = Filter::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Filter, None)
            }
            ModuleCategory::Envelope => {
                let m = Envelope::new();
                let d = m.descriptor();
                let pos_buf = m.position_buffer();
                (Box::new(m), d, TypedModuleType::Envelope, Some(pos_buf))
            }
            ModuleCategory::LFO => {
                let m = Lfo::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Lfo, None)
            }
            ModuleCategory::Amplifier => {
                let m = Amplifier::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Amplifier, None)
            }
            ModuleCategory::Mixer => {
                let m = Mixer::new();
                let d = m.descriptor();
                (Box::new(m), d, TypedModuleType::Mixer, None)
            }
            _ => return, // Effects handled separately
        };

        let next_id = self.next_module_id(module_type);
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

        // Set envelope position buffer for visualization
        if let Some(pos_buf) = envelope_pos {
            editor.set_module_envelope_position(next_id, pos_buf);
        }

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
        let module: Box<dyn synth_core::PolyModule> = Box::new(m);

        let next_id = self.next_module_id(TypedModuleType::MathOscillator);
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    fn add_sub_oscillator_module(&mut self) {
        let m = SubOscillator::new();
        let descriptor = m.descriptor();
        let module: Box<dyn synth_core::PolyModule> = Box::new(m);

        let next_id = self.next_module_id(TypedModuleType::SubOscillator);
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    fn add_noise_module(&mut self) {
        let m = NoiseGenerator::new();
        let descriptor = m.descriptor();
        let module: Box<dyn synth_core::PolyModule> = Box::new(m);

        let next_id = self.next_module_id(TypedModuleType::Noise);
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    fn add_keyboard_panner_module(&mut self) {
        let m = synth_modules::KeyboardPanner::new();
        let descriptor = m.descriptor();
        let module: Box<dyn synth_core::PolyModule> = Box::new(m);

        let next_id = self.next_module_id(TypedModuleType::KeyboardPanner);
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    fn add_body_resonance_module(&mut self) {
        let m = synth_modules::BodyResonance::new();
        let descriptor = m.descriptor();
        let module: Box<dyn synth_core::PolyModule> = Box::new(m);

        let next_id = self.next_module_id(TypedModuleType::BodyResonance);
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    fn add_mechanical_noise_module(&mut self) {
        let m = synth_modules::MechanicalNoise::new();
        let descriptor = m.descriptor();
        let module: Box<dyn synth_core::PolyModule> = Box::new(m);

        let next_id = self.next_module_id(TypedModuleType::MechanicalNoise);
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

        self.handle.send(EngineCommand::AddModuleInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            module,
        });
    }

    fn add_effect_module(&mut self, effect_type: EffectType) {
        // Create effect in GUI thread (real-time safe allocation)
        let (effect, descriptor, module_type): (
            Box<dyn synth_core::AudioEffect>,
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
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

        // Send pre-created effect to active instrument's effect chain
        self.handle.send(EngineCommand::AddEffectInstance {
            instrument_id: Some(self.active_instrument_id),
            id: next_id,
            effect,
        });
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
        };

        let next_id = self.next_module_id(module_type);
        // Visualizers are added to the active instrument's patch editor for visual display
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
        let Some(editor) = self.active_patch_editor() else {
            return;
        };
        editor.add_module(next_id, descriptor);

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

        ui.horizontal(|ui| {
            // Panic button (moved here since keyboard handles its own header)
            if ui
                .add(egui::Button::new(
                    RichText::new("PANIC").color(theme().colors.accent_red),
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
                RichText::new(format!("Playing: {}", active_name))
                    .color(theme().colors.accent_orange),
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
                .note_on_channel(note, Velocity::new(0.8), active_channel);
        }
        for note in event.note_off {
            self.handle.note_off_channel(note, active_channel);
            // Note release will be reflected via NoteReleased event from engine
        }
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
                FileDialogResult::Picked(path, Some(FileDialogMode::ImportSong)) => {
                    self.import_song_file(&path);
                }
                FileDialogResult::Picked(path, Some(FileDialogMode::OpenSample)) => {
                    // TODO: Handle sample loading when sample player UI is ready
                    self.dialog_state
                        .set_status(format!("Sample selected: {}", path.display()));
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
            &mut self.instance_counters,
            &mut self.handle,
            &mut self.keyboard,
            &mut self.glide_time,
            active_id,
        );
    }

    /// Import a song from a tracker file (MOD, XM, S3M).
    fn import_song_file(&mut self, path: &std::path::Path) {
        match import::import_song(path) {
            Ok(mut imported) => {
                // === Step 1: Clear existing state ===
                // Remove ALL existing instruments from engine (including the default one)
                for inst in &self.instruments {
                    self.handle.send_blocking(EngineCommand::RemoveInstrument {
                        instrument_id: inst.id,
                    });
                }
                // Clear GUI state
                self.instruments.clear();
                self.instance_counters.clear();
                self.handle.visualization_buffers.clear();

                // === Step 2: Sync grid from notes for tracker view ===
                // Collect pattern IDs first to avoid borrow issues
                let pattern_ids: Vec<_> = imported.song.patterns().map(|p| p.id).collect();
                for pattern_id in pattern_ids {
                    if let Some(pattern) = imported.song.pattern_mut(pattern_id) {
                        pattern.sync_grid_from_notes();
                    }
                }

                // === Step 3: Create instruments ===
                let instrument_count = imported.instruments.len();
                if instrument_count == 0 {
                    // No instruments - create a default instrument with just StereoOutput
                    let inst_id = InstrumentId::new(self.next_instrument_id);
                    self.next_instrument_id += 1;

                    let mut ui_state = InstrumentUiState::new(inst_id, "Default");
                    ui_state.channel = MidiChannel::CH1;

                    // Add StereoOutput to patch editor
                    let output = StereoOutput::new();
                    let output_desc = output.descriptor();
                    let output_id = self.next_module_id(TypedModuleType::StereoOutput);
                    ui_state.patch_editor.add_module(output_id, output_desc);

                    // Create engine instrument with tracker config
                    let voice_config = AllocatorConfig {
                        max_voices: imported.min_voices.unwrap_or(synth_core::VoiceCount::OCTO),
                        mode: AllocationMode::Tracker,
                        ..Default::default()
                    };
                    let mut engine_inst = Instrument::with_config(inst_id, "Default", voice_config);
                    engine_inst.set_midi_channel(MidiChannel::CH1);
                    self.handle.send_blocking(EngineCommand::AddInstrument {
                        instrument: Box::new(engine_inst),
                    });

                    // Add StereoOutput module to engine
                    self.handle.send_blocking(EngineCommand::AddModuleInstance {
                        instrument_id: Some(inst_id),
                        id: output_id,
                        module: Box::new(output),
                    });

                    self.instruments.push(ui_state);
                    self.active_instrument_id = inst_id;
                    // Set focused instrument for keyboard routing
                    self.handle.set_focused_instrument(Some(inst_id));
                } else {
                    // Create one synth instrument per imported instrument
                    for (idx, inst_meta) in imported.instruments.iter().enumerate() {
                        // Get sample for this instrument (if any)
                        let sample = inst_meta
                            .sample_index
                            .and_then(|i| imported.samples.get(i))
                            .cloned();

                        // Check if instrument has a valid (non-empty) sample
                        let has_valid_sample = sample.as_ref().is_some_and(|s| s.len().0 > 0);

                        let inst_id = InstrumentId::new(self.next_instrument_id);
                        self.next_instrument_id += 1;

                        // Use instrument name or generate one
                        let inst_name = if inst_meta.name.is_empty() {
                            format!("Instrument {}", idx + 1)
                        } else {
                            inst_meta.name.clone()
                        };

                        // Assign MIDI channel (1-16, wrap around)
                        let channel = MidiChannel::from_one_indexed(((idx % 16) + 1) as u8)
                            .unwrap_or(MidiChannel::CH1);

                        let mut ui_state = InstrumentUiState::new(inst_id, &inst_name);
                        ui_state.channel = channel;

                        // === Create engine instrument (always, to preserve indexing) ===
                        let min_voices =
                            imported.min_voices.unwrap_or(synth_core::VoiceCount::OCTO);
                        let voice_config = AllocatorConfig {
                            max_voices: min_voices,
                            mode: AllocationMode::Tracker,
                            ..Default::default()
                        };
                        let mut engine_inst =
                            Instrument::with_config(inst_id, &inst_name, voice_config);
                        engine_inst.set_midi_channel(channel);
                        self.handle.send_blocking(EngineCommand::AddInstrument {
                            instrument: Box::new(engine_inst),
                        });

                        // === Only create modules if instrument has a valid sample ===
                        if !has_valid_sample {
                            // Empty instrument - just add to UI and continue
                            self.instruments.push(ui_state);
                            if idx == 0 {
                                self.active_instrument_id = inst_id;
                                self.handle.set_focused_instrument(Some(inst_id));
                            }
                            continue;
                        }

                        // Generate waveform overview for GUI visualization
                        if let Some(ref smp) = sample {
                            ui_state.waveform_overview =
                                Some(synth_core::WaveformOverview::generate(smp, 200));
                        }

                        // === Create SamplePlayer module ===
                        let mut sample_player = SamplePlayer::new();

                        // Pre-configure SamplePlayer from sample metadata (if available)
                        // This ensures GUI and engine have matching parameters
                        if let Some(ref smp) = sample {
                            let sample_len = smp.len().as_usize();
                            // Apply loop settings from sample metadata
                            if let Some(ref loop_info) = smp.loop_info {
                                // Convert exact sample positions to normalized values for GUI
                                sample_player.set_param(Param::SamplePlayer(
                                    SamplePlayerParam::LoopStart(NormalizedValue::new(
                                        loop_info.normalized_start(sample_len),
                                    )),
                                ));
                                sample_player.set_param(Param::SamplePlayer(
                                    SamplePlayerParam::LoopEnd(NormalizedValue::new(
                                        loop_info.normalized_end(sample_len),
                                    )),
                                ));
                                if loop_info.enabled {
                                    let loop_mode = if loop_info.ping_pong {
                                        LoopMode::PingPong
                                    } else {
                                        LoopMode::Forward
                                    };
                                    sample_player.set_param(Param::SamplePlayer(
                                        SamplePlayerParam::LoopMode(loop_mode),
                                    ));
                                }
                            }

                            // Apply default volume from sample metadata
                            if let Some(volume) = smp.default_volume {
                                sample_player.set_param(Param::SamplePlayer(
                                    SamplePlayerParam::Level(Gain::new(volume)),
                                ));
                            }

                            // Set release mode based on loop settings:
                            // - Looped samples: Immediate (stop at note-off)
                            // - Non-looped samples: PlayToEnd (let sample finish)
                            let has_loop =
                                smp.loop_info.as_ref().map(|li| li.enabled).unwrap_or(false);
                            let release_mode = if has_loop {
                                ReleaseMode::Immediate
                            } else {
                                ReleaseMode::PlayToEnd
                            };
                            sample_player.set_param(Param::SamplePlayer(
                                SamplePlayerParam::ReleaseMode(release_mode),
                            ));
                        }

                        let sample_player_desc = sample_player.descriptor();
                        let sample_player_id = self.next_module_id(TypedModuleType::SamplePlayer);
                        ui_state
                            .patch_editor
                            .add_module(sample_player_id, sample_player_desc);

                        // Sync GUI parameter values with pre-configured SamplePlayer
                        // (add_module uses descriptor defaults, so we need to override)
                        if let Some(ref smp) = sample {
                            let sample_len = smp.len().as_usize();
                            if let Some(ref loop_info) = smp.loop_info {
                                // Convert exact sample positions to normalized values for GUI
                                ui_state.patch_editor.set_parameter_by_name(
                                    sample_player_id,
                                    "Loop Start",
                                    loop_info.normalized_start(sample_len),
                                );
                                ui_state.patch_editor.set_parameter_by_name(
                                    sample_player_id,
                                    "Loop End",
                                    loop_info.normalized_end(sample_len),
                                );
                                if loop_info.enabled {
                                    let loop_mode_idx = if loop_info.ping_pong {
                                        3.0 // PingPong = index 3
                                    } else {
                                        1.0 // Forward = index 1
                                    };
                                    ui_state.patch_editor.set_parameter_by_name(
                                        sample_player_id,
                                        "Loop",
                                        loop_mode_idx,
                                    );
                                }
                            }

                            if let Some(volume) = smp.default_volume {
                                ui_state.patch_editor.set_parameter_by_name(
                                    sample_player_id,
                                    "Level",
                                    volume,
                                );
                            }

                            // Set release mode in GUI
                            let has_loop =
                                smp.loop_info.as_ref().map(|li| li.enabled).unwrap_or(false);
                            let release_mode_idx = if has_loop {
                                0.0 // Immediate = index 0
                            } else {
                                1.0 // PlayToEnd = index 1
                            };
                            ui_state.patch_editor.set_parameter_by_name(
                                sample_player_id,
                                "Release",
                                release_mode_idx,
                            );
                        }

                        // Copy waveform overview to the module's panel state
                        if let Some(ref waveform) = ui_state.waveform_overview {
                            ui_state
                                .patch_editor
                                .set_module_waveform(sample_player_id, waveform.clone());
                        }

                        // Set position buffer for real-time playback position display
                        let position_buffer = sample_player.position_buffer();
                        ui_state
                            .patch_editor
                            .set_module_position_buffer(sample_player_id, position_buffer);

                        // === Create Envelope + Amplifier ===
                        // All instruments get an envelope + amplifier to ensure NoteOff works.
                        // - With envelope points: use MultiPointEnvelope
                        // - With ADSR (enabled but no points): use ADSR Envelope
                        // - Without envelope: use minimal gate envelope (instant on/off)

                        // Calculate tick rate from song BPM
                        let song_bpm = imported.song.default_tempo.as_f32();
                        let tick_rate = song_bpm * 2.0 / 5.0;

                        // Create the volume envelope module
                        let (envelope_id, envelope_module): (ModuleId, Box<dyn PolyModule>) =
                            if inst_meta.volume_envelope.enabled
                                && !inst_meta.envelope_points.is_empty()
                            {
                                // Use MultiPointEnvelope for accurate XM/IT envelope playback
                                let mut mp_env =
                                    MultiPointEnvelope::with_points(&inst_meta.envelope_points);

                                // Set sustain point (holds until note-off)
                                mp_env.set_sustain_point(inst_meta.envelope_sustain);

                                // Set loop region (loops while sustained)
                                if let Some((loop_start, loop_end)) = inst_meta.envelope_loop {
                                    mp_env.set_loop(Some(loop_start), Some(loop_end));
                                }

                                // Set fadeout rate from instrument metadata
                                // XM fadeout is stored as f32 (0.0-65535.0 range)
                                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                                let fadeout_rate = FadeoutRate::new(inst_meta.fadeout as u16);
                                mp_env.set_fadeout_rate(fadeout_rate);

                                // Set tick rate from song BPM
                                mp_env.set_tick_rate(tick_rate);

                                let envelope_desc = mp_env.descriptor();
                                let envelope_id =
                                    self.next_module_id(TypedModuleType::MultiPointEnvelope);
                                ui_state.patch_editor.add_module(envelope_id, envelope_desc);

                                (envelope_id, Box::new(mp_env))
                            } else if inst_meta.volume_envelope.enabled {
                                // Fall back to ADSR Envelope for simple cases
                                let mut envelope = Envelope::new();
                                envelope.set_param(Param::Envelope(EnvelopeParam::Attack(
                                    inst_meta.volume_envelope.attack,
                                )));
                                envelope.set_param(Param::Envelope(EnvelopeParam::Decay(
                                    inst_meta.volume_envelope.decay,
                                )));
                                envelope.set_param(Param::Envelope(EnvelopeParam::Sustain(
                                    inst_meta.volume_envelope.sustain,
                                )));
                                envelope.set_param(Param::Envelope(EnvelopeParam::Release(
                                    inst_meta.volume_envelope.release,
                                )));

                                let envelope_desc = envelope.descriptor();
                                let position_buffer = envelope.position_buffer();
                                let envelope_id = self.next_module_id(TypedModuleType::Envelope);
                                ui_state.patch_editor.add_module(envelope_id, envelope_desc);
                                ui_state
                                    .patch_editor
                                    .set_module_envelope_position(envelope_id, position_buffer);

                                (envelope_id, Box::new(envelope))
                            } else {
                                // Gate envelope: instant on/off for instruments without volume envelope
                                // This ensures NoteOff can silence looped samples (MOD/S3M)
                                let mut envelope = Envelope::new();
                                envelope.set_param(Param::Envelope(EnvelopeParam::Attack(
                                    Seconds::new(0.001),
                                )));
                                envelope.set_param(Param::Envelope(EnvelopeParam::Decay(
                                    Seconds::new(0.001),
                                )));
                                envelope.set_param(Param::Envelope(EnvelopeParam::Sustain(
                                    NormalizedValue::MAX,
                                )));
                                envelope.set_param(Param::Envelope(EnvelopeParam::Release(
                                    Seconds::new(0.005),
                                )));

                                let envelope_desc = envelope.descriptor();
                                let envelope_id = self.next_module_id(TypedModuleType::Envelope);
                                ui_state.patch_editor.add_module(envelope_id, envelope_desc);

                                (envelope_id, Box::new(envelope))
                            };

                        // Create Amplifier (VCA) with global volume and pan
                        let mut amplifier = Amplifier::new();
                        amplifier.set_param(Param::Amplifier(AmplifierParam::Level(
                            inst_meta.global_volume,
                        )));
                        amplifier.set_param(Param::Amplifier(AmplifierParam::Pan(
                            inst_meta.default_pan,
                        )));

                        let amplifier_desc = amplifier.descriptor();
                        let amplifier_id = self.next_module_id(TypedModuleType::Amplifier);
                        ui_state
                            .patch_editor
                            .add_module(amplifier_id, amplifier_desc);

                        // Create panning envelope if instrument has panning envelope points
                        let panning_envelope = if !inst_meta.panning_envelope_points.is_empty() {
                            let mut pan_env =
                                MultiPointEnvelope::with_points(&inst_meta.panning_envelope_points);

                            // Set sustain point
                            pan_env.set_sustain_point(inst_meta.panning_envelope_sustain);

                            // Set loop region
                            if let Some((loop_start, loop_end)) = inst_meta.panning_envelope_loop {
                                pan_env.set_loop(Some(loop_start), Some(loop_end));
                            }

                            // Panning envelope uses same fadeout as volume
                            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                            let fadeout_rate = FadeoutRate::new(inst_meta.fadeout as u16);
                            pan_env.set_fadeout_rate(fadeout_rate);

                            // Set tick rate from song BPM
                            pan_env.set_tick_rate(tick_rate);

                            // Bipolar output: 0.0-1.0 → -1.0 to +1.0
                            pan_env.set_output_bipolar(true);

                            let pan_env_desc = pan_env.descriptor();
                            let pan_env_id =
                                self.next_module_id(TypedModuleType::MultiPointEnvelope);
                            ui_state.patch_editor.add_module(pan_env_id, pan_env_desc);

                            Some((pan_env_id, Box::new(pan_env) as Box<dyn PolyModule>))
                        } else {
                            None
                        };

                        // === Create StereoOutput module ===
                        let stereo_output = StereoOutput::new();
                        let stereo_output_desc = stereo_output.descriptor();
                        let stereo_output_id = self.next_module_id(TypedModuleType::StereoOutput);
                        ui_state
                            .patch_editor
                            .add_module(stereo_output_id, stereo_output_desc);

                        // === Create connections in GUI ===
                        // All instruments: SamplePlayer -> Amplifier -> StereoOutput
                        //                  Envelope -> Amplifier CV
                        ui_state.patch_editor.add_connection(Connection::new(
                            sample_player_id,
                            "out",
                            amplifier_id,
                            "in",
                        ));
                        ui_state.patch_editor.add_connection(Connection::new(
                            envelope_id,
                            "out",
                            amplifier_id,
                            "cv",
                        ));
                        // Panning envelope -> Amplifier pan_cv
                        if let Some((pan_env_id, _)) = &panning_envelope {
                            ui_state.patch_editor.add_connection(Connection::new(
                                *pan_env_id,
                                "out",
                                amplifier_id,
                                "pan_cv",
                            ));
                        }
                        ui_state.patch_editor.add_connection(Connection::new(
                            amplifier_id,
                            "left",
                            stereo_output_id,
                            "in_l",
                        ));
                        ui_state.patch_editor.add_connection(Connection::new(
                            amplifier_id,
                            "right",
                            stereo_output_id,
                            "in_r",
                        ));

                        // === Send modules to engine (all modules BEFORE connections) ===
                        self.handle.send_blocking(EngineCommand::AddModuleInstance {
                            instrument_id: Some(inst_id),
                            id: sample_player_id,
                            module: Box::new(sample_player),
                        });

                        self.handle.send_blocking(EngineCommand::AddModuleInstance {
                            instrument_id: Some(inst_id),
                            id: stereo_output_id,
                            module: Box::new(stereo_output),
                        });

                        self.handle.send_blocking(EngineCommand::AddModuleInstance {
                            instrument_id: Some(inst_id),
                            id: envelope_id,
                            module: envelope_module, // Already boxed
                        });
                        self.handle.send_blocking(EngineCommand::AddModuleInstance {
                            instrument_id: Some(inst_id),
                            id: amplifier_id,
                            module: Box::new(amplifier),
                        });

                        // Extract panning envelope ID before consuming the module
                        let pan_env_id_opt = panning_envelope.as_ref().map(|(id, _)| *id);

                        if let Some((pan_env_id, pan_env_module)) = panning_envelope {
                            self.handle.send_blocking(EngineCommand::AddModuleInstance {
                                instrument_id: Some(inst_id),
                                id: pan_env_id,
                                module: pan_env_module,
                            });
                        }

                        // Send connections
                        self.handle.send_blocking(EngineCommand::Connect {
                            instrument_id: Some(inst_id),
                            from: PortId::new(sample_player_id, "out"),
                            to: PortId::new(amplifier_id, "in"),
                        });
                        self.handle.send_blocking(EngineCommand::Connect {
                            instrument_id: Some(inst_id),
                            from: PortId::new(envelope_id, "out"),
                            to: PortId::new(amplifier_id, "cv"),
                        });
                        if let Some(pan_env_id) = pan_env_id_opt {
                            self.handle.send_blocking(EngineCommand::Connect {
                                instrument_id: Some(inst_id),
                                from: PortId::new(pan_env_id, "out"),
                                to: PortId::new(amplifier_id, "pan_cv"),
                            });
                        }
                        self.handle.send_blocking(EngineCommand::Connect {
                            instrument_id: Some(inst_id),
                            from: PortId::new(amplifier_id, "left"),
                            to: PortId::new(stereo_output_id, "in_l"),
                        });
                        self.handle.send_blocking(EngineCommand::Connect {
                            instrument_id: Some(inst_id),
                            from: PortId::new(amplifier_id, "right"),
                            to: PortId::new(stereo_output_id, "in_r"),
                        });

                        // === Load sample(s) into SamplePlayer ===
                        // For multisample instruments (with keymap), load the full sample bank
                        // For single-sample instruments, use the simpler LoadSample command
                        if let Some(keymap) = &inst_meta.sample_keymap {
                            // Multisample instrument: collect all samples for this instrument
                            if let Some(first_idx) = inst_meta.sample_index {
                                let bank_samples: Vec<_> = (0..inst_meta.sample_count)
                                    .filter_map(|offset| {
                                        imported.samples.get(first_idx + offset).cloned()
                                    })
                                    .collect();

                                if !bank_samples.is_empty() {
                                    self.handle.send_blocking(EngineCommand::LoadSampleBank {
                                        instrument_id: Some(inst_id),
                                        module_id: sample_player_id,
                                        samples: bank_samples,
                                        keymap: keymap.clone(),
                                    });
                                }
                            }
                        } else if let Some(smp) = sample {
                            // Single sample instrument
                            self.handle.send_blocking(EngineCommand::LoadSample {
                                instrument_id: Some(inst_id),
                                module_id: sample_player_id,
                                sample: smp,
                            });
                        }

                        self.instruments.push(ui_state);

                        // Set first instrument as active for keyboard input
                        if idx == 0 {
                            self.active_instrument_id = inst_id;
                            // Set focused instrument for keyboard routing
                            self.handle.set_focused_instrument(Some(inst_id));
                        }
                    }
                }

                // === Step 4: Store the song and set tempo ===
                self.handle
                    .send_blocking(EngineCommand::SetTempo(imported.song.default_tempo));

                // Set the active pattern for the tracker view (first pattern in arrangement)
                let first_pattern_id = imported.song.arrangement().first().map(|p| p.pattern_id);
                self.tracker_state.active_pattern = first_pattern_id;

                // === Step 5: Send song to engine for playback ===
                // Create Arc<RwLock<Song>> and share with engine
                let song_arc = std::sync::Arc::new(std::sync::RwLock::new(imported.song.clone()));
                self.handle
                    .send_blocking(EngineCommand::SetSong { song: song_arc });

                // Store the song in GUI state
                self.song = Some(imported.song);

                // === Step 6: Reset transport ===
                self.handle.send_blocking(EngineCommand::Stop);
                self.handle.send_blocking(EngineCommand::Rewind);

                // Show success status
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                let sample_count = imported.samples.len();
                self.dialog_state.set_status(format!(
                    "Imported: {} ({} instruments, {} samples)",
                    file_name, instrument_count, sample_count
                ));

                // Switch to sequencer view to show the imported song
                self.active_view = AppView::Sequencer;
                // Keep focused instrument set - keyboard input goes to active instrument,
                // sequencer plays all instruments regardless of focus
            }
            Err(e) => {
                self.dialog_state.set_status(format!("Import failed: {e}"));
            }
        }
    }

    /// Reset the active instrument to a new empty patch.
    /// Clears all modules and adds a default StereoOutput for immediate sound.
    fn reset_to_new_patch(&mut self) {
        // 1. Clear active instrument's GUI state
        let active_id = self.active_instrument_id;

        // Clear all modules from the active instrument in the engine
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
        if let Some(editor) = self.active_patch_editor() {
            editor.clear();
        }
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
        let editor = self.active_patch_editor_ref()?;
        patch_bridge::create_patch_from_rack(
            &self.dialog_state.patch_save_name,
            editor,
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
        }
    });
}
