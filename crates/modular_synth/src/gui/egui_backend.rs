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
    EffectType, PaletteSelection, PaletteVisualizerType, PatchEditor, QuickAddRequest,
};
use crate::gui::theme::theme;
use crate::gui::widgets::{draw_oscilloscope, draw_stereo_meter};
use crate::gui::{GuiBackend, GuiResult, SynthGuiConfig};
use crate::io::MidiHandler;
use crate::patch::{Patch, categorized_patches};
use synth_core::Velocity;
#[cfg(feature = "mcp")]
use synth_core::{BipolarValue, Gain};
use synth_core::{Describable, ModuleCategory};
use synth_engine::ModuleType as TypedModuleType;
use synth_engine::commands::PortId;
use synth_engine::instrument::{InstrumentId, MidiChannel};
use synth_engine::visualizers::{LevelMeter, Oscilloscope, SpectrumAnalyzer};
use synth_engine::{EngineCommand, EngineEvent, EngineHandle, ModuleId, SynthEngine};

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
        let window_width = config.settings.window.width as f32;
        let window_height = config.settings.window.height as f32;
        let startup_theme = config.settings.theme;

        let mut viewport = egui::ViewportBuilder::default()
            .with_inner_size([window_width, window_height])
            .with_title(&window_title)
            .with_min_inner_size([800.0, 600.0]);

        if let (Some(x), Some(y)) = (config.settings.window.x, config.settings.window.y) {
            viewport = viewport.with_position([x as f32, y as f32]);
        }

        let app = SynthApp::new(handle, host, config, stream_info.output_latency);

        let options = eframe::NativeOptions {
            viewport,
            ..Default::default()
        };

        eframe::run_native(
            &window_title,
            options,
            Box::new(move |cc| {
                setup_custom_fonts(&cc.egui_ctx);
                startup_theme.apply();
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

    // Register Remix Icon font (icon glyphs as fallback)
    egui_remixicon::add_to_fonts(&mut fonts);

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

    // Navigation state
    active_view: AppView,

    // AWE state
    awe_enabled: bool,
    awe_ui: crate::gui::awe_view::AweUiState,

    // Sequencer state
    song: std::sync::Arc<std::sync::RwLock<synth_sequencer::Song>>,
    sequencer_view_state: crate::gui::sequencer::SequencerViewState,

    // MCP shared state
    #[cfg(feature = "mcp")]
    mcp_shared: Option<std::sync::Arc<crate::mcp_shared::McpSharedState>>,

    // Persistent application settings
    settings: crate::io::settings::AppSettings,
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

        let settings = config.settings.clone();
        let session = config.session.clone();
        let song = config.song.clone();
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

        let mut dialog_state = DialogState::new();
        dialog_state.current_theme = settings.theme;

        Self {
            handle,
            host: Some(host),
            latency,
            session,
            midi_handler,
            keyboard,
            pressed_keys: HashMap::new(),
            dialog_state,
            current_patch_name: patch_name,
            current_patch_path: None,
            glide_time,
            instruments,
            active_instrument_id,
            next_instrument_id,
            active_view: AppView::default(),
            awe_enabled: false,
            awe_ui: crate::gui::awe_view::AweUiState::default(),
            song,
            sequencer_view_state: crate::gui::sequencer::SequencerViewState::new(),
            #[cfg(feature = "mcp")]
            mcp_shared: config.mcp_shared,
            settings,
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
        use egui_remixicon::icons as ri;
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
                    if ui
                        .button(format!("{} New Patch", ri::FILE_ADD_LINE))
                        .clicked()
                    {
                        self.reset_to_new_patch();
                        self.dialog_state
                            .set_status("New patch created".to_string());
                        ui.close();
                    }
                    if ui
                        .button(format!("{} Open Patch...", ri::FOLDER_OPEN_LINE))
                        .clicked()
                    {
                        self.dialog_state.open_open_patch_dialog();
                        ui.close();
                    }
                    if ui
                        .button(format!("{} Load Built-in...", ri::FOLDER_OPEN_LINE))
                        .clicked()
                    {
                        self.dialog_state.show_load_patch = true;
                        ui.close();
                    }
                    if ui
                        .button(format!("{} Save Patch...", ri::SAVE_LINE))
                        .clicked()
                    {
                        let default_name = format!(
                            "{}.json",
                            self.current_patch_name.to_lowercase().replace(' ', "_")
                        );
                        self.dialog_state.open_save_patch_dialog(&default_name);
                        ui.close();
                    }
                    ui.separator();
                    ui.menu_button(format!("{} Example Patches", ri::FILE_LIST_LINE), |ui| {
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
                    if ui
                        .button(format!("{} Settings...", ri::SETTINGS_LINE))
                        .clicked()
                    {
                        self.dialog_state.show_settings = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(format!("{} Quit", ri::SHUT_DOWN_LINE)).clicked() {
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
                    if ui
                        .add(egui::Button::new(
                            RichText::new("PANIC").color(theme().colors.accent_red),
                        ))
                        .clicked()
                    {
                        self.handle.send(EngineCommand::AllNotesOff);
                        self.pressed_keys.clear();
                        self.keyboard.clear_pressed();
                    }
                    ui.separator();

                    // Glide + module/connection counts (Rack view only)
                    if self.active_view == AppView::Rack {
                        let (conn_count, module_count) = self
                            .active_patch_editor_ref()
                            .map(|e| (e.connections().len(), e.module_ids().len()))
                            .unwrap_or((0, 0));
                        ui.label(
                            RichText::new(format!("M:{} C:{}", module_count, conn_count))
                                .color(theme().colors.text_dim),
                        );
                        ui.separator();

                        ui.label(RichText::new("Glide:").color(theme().colors.text_dim));
                        let glide_response = ui.add(
                            egui::Slider::new(&mut self.glide_time, 0.0..=2.0)
                                .suffix(" s")
                                .fixed_decimals(2)
                                .custom_formatter(|v, _| {
                                    if v < 0.001 {
                                        "Off".to_string()
                                    } else {
                                        format!("{v:.2}s")
                                    }
                                }),
                        );
                        if glide_response.changed() {
                            self.handle.send(EngineCommand::SetGlideTime(
                                synth_core::Seconds::new(self.glide_time),
                            ));
                        }
                        ui.separator();
                    }

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
                        RichText::new(format!("{} {} ▼", ri::PIANO_FILL, short_name))
                            .color(theme().colors.meter_green)
                    } else {
                        RichText::new(format!("{} MIDI ▼", ri::PIANO_LINE))
                            .color(theme().colors.text_dim)
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
                                    RichText::new(format!(
                                        "{} {}",
                                        ri::CHECKBOX_BLANK_CIRCLE_FILL,
                                        port
                                    ))
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
                    // MCP connection status indicator
                    #[cfg(feature = "mcp")]
                    if let Some(ref mcp) = self.mcp_shared {
                        let listening = mcp.is_listening();
                        let sessions = mcp.active_sessions();
                        let (icon, label, color) = if sessions > 0 {
                            (
                                ri::ROBOT_2_FILL,
                                format!("MCP ({})", sessions),
                                theme().colors.meter_green,
                            )
                        } else if listening {
                            (ri::ROBOT_2_LINE, "MCP".to_owned(), theme().colors.text_dim)
                        } else {
                            (
                                ri::ROBOT_2_LINE,
                                "MCP".to_owned(),
                                theme().colors.accent_red,
                            )
                        };
                        let resp = ui.label(RichText::new(format!("{icon} {label}")).color(color));
                        if resp.hovered() {
                            resp.on_hover_text(if sessions > 0 {
                                format!("MCP: {sessions} active session(s)")
                            } else if listening {
                                "MCP: listening (no active sessions)".to_owned()
                            } else {
                                "MCP: not running".to_owned()
                            });
                        }
                        ui.separator();
                    }
                    // Current patch name
                    ui.label(
                        RichText::new(format!("Patch: {}", self.current_patch_name))
                            .color(theme().colors.accent_cyan),
                    );
                    ui.separator();

                    // View selector — segmented control
                    {
                        let t = theme();
                        let views: [(AppView, &str); 3] = [
                            (AppView::Rack, &format!("{} Rack", ri::LAYOUT_GRID_FILL)),
                            (
                                AppView::AcousticWorld,
                                &format!("{} AWE", ri::SURROUND_SOUND_FILL),
                            ),
                            (AppView::Sequencer, &format!("{} Seq", ri::PLAY_LIST_FILL)),
                        ];
                        let seg_w = 80.0_f32;
                        let seg_h = 22.0_f32;
                        let rounding: u8 = 5;
                        let total_w = seg_w * views.len() as f32;
                        let (outer_rect, _) = ui
                            .allocate_exact_size(egui::vec2(total_w, seg_h), egui::Sense::hover());
                        let painter = ui.painter_at(outer_rect);

                        // Outer border with pill rounding
                        painter.rect_stroke(
                            outer_rect,
                            egui::CornerRadius::same(rounding),
                            egui::Stroke::new(1.0, t.colors.border),
                            egui::StrokeKind::Inside,
                        );

                        for (i, (view, label)) in views.iter().enumerate() {
                            let is_active = self.active_view == *view;
                            let x = outer_rect.left() + seg_w * i as f32;
                            let seg_rect = egui::Rect::from_min_size(
                                egui::pos2(x, outer_rect.top()),
                                egui::vec2(seg_w, seg_h),
                            );

                            // Rounding: first segment left-rounded, last right-rounded
                            let seg_rounding = if i == 0 {
                                egui::CornerRadius {
                                    nw: rounding,
                                    sw: rounding,
                                    ne: 0,
                                    se: 0,
                                }
                            } else if i == views.len() - 1 {
                                egui::CornerRadius {
                                    nw: 0,
                                    sw: 0,
                                    ne: rounding,
                                    se: rounding,
                                }
                            } else {
                                egui::CornerRadius::ZERO
                            };

                            // Active fill
                            if is_active {
                                painter.rect_filled(
                                    seg_rect,
                                    seg_rounding,
                                    t.colors.accent_primary.gamma_multiply(0.55),
                                );
                            }

                            // Divider lines between inactive segments
                            if i > 0 {
                                let prev_active = self.active_view == views[i - 1].0;
                                if !is_active && !prev_active {
                                    let top = seg_rect.top() + 4.0;
                                    let bot = seg_rect.bottom() - 4.0;
                                    painter.line_segment(
                                        [egui::pos2(x, top), egui::pos2(x, bot)],
                                        egui::Stroke::new(1.0, t.colors.border),
                                    );
                                }
                            }

                            // Text
                            let text_color = if is_active {
                                t.colors.text_primary
                            } else {
                                t.colors.text_dim
                            };
                            painter.text(
                                seg_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                label,
                                egui::FontId::proportional(13.0),
                                text_color,
                            );

                            // Click interaction
                            let resp = ui.interact(
                                seg_rect,
                                ui.id().with(("view_seg", i)),
                                egui::Sense::click(),
                            );
                            if resp.clicked() {
                                self.active_view = *view;
                            }
                        }
                    }
                    ui.separator();
                });
            });
        });

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
                // Check if this module has a visualization buffer to clean up
                let has_vis_buffer = patch_editor
                    .module_descriptor(module_id)
                    .is_some_and(|d| {
                        d.category == ModuleCategory::Visualizer
                            || d.type_id.0 == "signal_monitor"
                            || d.type_id.0 == "inline_signal_monitor"
                    });

                patch_editor.remove_module(module_id);

                // Remove from session (registry + engine command)
                let _ = self.session.remove_module(active_id, module_id);

                // Clean up visualization buffer if needed
                if has_vis_buffer {
                    self.handle.remove_visualization_buffer(module_id);
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

                patch_editor.add_module_at(monitor_id, inline_descriptor.clone(), mid_pos);

                // Register in session so reconciliation doesn't remove it
                self.session
                    .register_descriptor(active_id, monitor_id, inline_descriptor);

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

            // Handle background context menu add (right-click on empty space or cable)
            if let Some((selection, world_pos, inline_cable)) = result.context_add {
                Self::handle_context_add(
                    &self.session,
                    &mut self.handle,
                    self.active_instrument_id,
                    patch_editor,
                    selection,
                    world_pos,
                    inline_cable,
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
            AppView::Sequencer => {
                crate::gui::sequencer::draw_sequencer_view(
                    ctx,
                    &mut self.handle,
                    &self.song,
                    &mut self.sequencer_view_state,
                    &self.instruments,
                );
            }
        }

        // Dialogs
        self.show_dialogs(ctx);

        // Write current UI layout to MCP shared state
        #[cfg(feature = "mcp")]
        self.write_mcp_layout(ctx);

        // Track window geometry for saving on exit
        ctx.input(|i| {
            if let Some(rect) = i.viewport().inner_rect {
                self.settings.window.width = rect.width() as u32;
                self.settings.window.height = rect.height() as u32;
            }
            if let Some(pos) = i.viewport().outer_rect {
                self.settings.window.x = Some(pos.min.x as i32);
                self.settings.window.y = Some(pos.min.y as i32);
            }
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Save window geometry
        self.settings.save();

        // Stop audio
        if let Some(ref mut host) = self.host {
            let _ = host.stop();
        }
    }
}

impl SynthApp {
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
                EffectType::EnsembleChorus => TypedModuleType::EnsembleChorus,
                EffectType::ShimmerReverb => TypedModuleType::ShimmerReverb,
                EffectType::GranularFx => TypedModuleType::GranularFx,
                EffectType::SpectralBlur => TypedModuleType::SpectralBlur,
                EffectType::ModalResonator => TypedModuleType::ModalResonator,
                EffectType::ReverseGateReverb => TypedModuleType::ReverseGateReverb,
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
                session.register_descriptor(instrument_id, id, d.clone());
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
            .map(|p| p.name)
        else {
            return;
        };

        let connection = match request.target_direction {
            WidgetPortDirection::Input => synth_engine::graph::Connection::new(
                next_id,
                new_port_name,
                request.target_module,
                request.target_port,
            ),
            WidgetPortDirection::Output => synth_engine::graph::Connection::new(
                request.target_module,
                request.target_port,
                next_id,
                new_port_name,
            ),
        };

        editor.add_connection(connection);
        let _ = session.connect(
            instrument_id,
            connection.from_module,
            connection.from_port,
            connection.to_module,
            connection.to_port,
        );
    }

    /// Handle a context menu add: create a module and place it at the given position.
    /// If `inline_cable` is `Some`, the old cable is removed and the new module is
    /// wired inline: `from → new_module(first_input) → new_module(first_output) → to`.
    #[allow(clippy::too_many_lines)]
    fn handle_context_add(
        session: &crate::session::SynthSession,
        handle: &mut EngineHandle,
        instrument_id: InstrumentId,
        editor: &mut PatchEditor,
        selection: PaletteSelection,
        position: Pos2,
        inline_cable: Option<synth_engine::graph::Connection>,
    ) {
        // Helper: wire a newly created module inline on an existing cable.
        // Removes the old cable, then connects from→new(first_input), new(first_output)→to.
        let wire_inline = |handle: &mut EngineHandle,
                           editor: &mut PatchEditor,
                           new_id: ModuleId,
                           descriptor: &synth_core::ModuleDescriptor,
                           cable: synth_engine::graph::Connection| {
            // Remove old cable from engine
            handle.send(EngineCommand::Disconnect {
                instrument_id: Some(instrument_id),
                from: PortId::new(cable.from_module, cable.from_port),
                to: PortId::new(cable.to_module, cable.to_port),
            });

            // Find first audio input and output on the new module
            let first_input = descriptor.ports.iter().find(|p| {
                p.direction == synth_core::PortDirection::Input
                    && p.port_type == synth_core::PortType::Audio
            });
            let first_output = descriptor.ports.iter().find(|p| {
                p.direction == synth_core::PortDirection::Output
                    && p.port_type == synth_core::PortType::Audio
            });

            if let (Some(inp), Some(outp)) = (first_input, first_output) {
                let conn_in = synth_engine::graph::Connection::new(
                    cable.from_module,
                    cable.from_port,
                    new_id,
                    inp.name,
                );
                let conn_out = synth_engine::graph::Connection::new(
                    new_id,
                    outp.name,
                    cable.to_module,
                    cable.to_port,
                );
                for c in [conn_in, conn_out] {
                    editor.add_connection(c);
                    handle.send(EngineCommand::Connect {
                        instrument_id: Some(instrument_id),
                        from: PortId::new(c.from_module, c.from_port),
                        to: PortId::new(c.to_module, c.to_port),
                    });
                }
            }
        };

        match selection {
            PaletteSelection::Visualizer(viz_type) => {
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
                let next_id = {
                    use synth_core::ModuleType;
                    let mt = match viz_type {
                        PaletteVisualizerType::Oscilloscope => ModuleType::Oscilloscope,
                        PaletteVisualizerType::LevelMeter => ModuleType::LevelMeter,
                        PaletteVisualizerType::SpectrumAnalyzer => ModuleType::SpectrumAnalyzer,
                    };
                    let mut counters = session.counters_lock();
                    let counter = counters.entry((instrument_id, mt)).or_insert(0);
                    *counter += 1;
                    ModuleId::new(module_type, *counter)
                };
                editor.add_module_at(next_id, descriptor, position);

                let buffer =
                    std::sync::Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(next_id, buffer.clone());
                let engine_viz_type = match viz_type {
                    PaletteVisualizerType::Oscilloscope => {
                        synth_engine::commands::VisualizerType::Oscilloscope
                    }
                    PaletteVisualizerType::LevelMeter => {
                        synth_engine::commands::VisualizerType::LevelMeter
                    }
                    PaletteVisualizerType::SpectrumAnalyzer => {
                        synth_engine::commands::VisualizerType::SpectrumAnalyzer
                    }
                };
                handle.send(EngineCommand::AddVisualizer {
                    instrument_id: Some(instrument_id),
                    id: next_id,
                    visualizer_type: engine_viz_type,
                    buffer,
                });
            }
            PaletteSelection::SignalMonitor => {
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
                session.register_descriptor(instrument_id, id, d.clone());
                editor.add_module_at(id, d, position);

                let buffer =
                    std::sync::Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
                handle.add_visualization_buffer(id, buffer.clone());
                m.set_vis_sink(buffer);
                handle.send(EngineCommand::AddModuleInstance {
                    instrument_id: Some(instrument_id),
                    id,
                    module: Box::new(m),
                });
            }
            _ => {
                let module_type = match selection {
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
                    PaletteSelection::ModMatrix => Some(TypedModuleType::ModMatrix),
                    PaletteSelection::StereoOutput => Some(TypedModuleType::StereoOutput),
                    PaletteSelection::KeyboardPanner => Some(TypedModuleType::KeyboardPanner),
                    PaletteSelection::BodyResonance => Some(TypedModuleType::BodyResonance),
                    PaletteSelection::MechanicalNoise => Some(TypedModuleType::MechanicalNoise),
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
                        EffectType::EnsembleChorus => TypedModuleType::EnsembleChorus,
                        EffectType::ShimmerReverb => TypedModuleType::ShimmerReverb,
                        EffectType::GranularFx => TypedModuleType::GranularFx,
                        EffectType::SpectralBlur => TypedModuleType::SpectralBlur,
                        EffectType::ModalResonator => TypedModuleType::ModalResonator,
                        EffectType::ReverseGateReverb => TypedModuleType::ReverseGateReverb,
                    }),
                    _ => None,
                };

                if let Some(mt) = module_type {
                    let Ok((next_id, descriptor)) = session.add_module(instrument_id, mt) else {
                        return;
                    };
                    editor.add_module_at(next_id, descriptor.clone(), position);

                    // Wire inline if this was a cable context menu action
                    if let Some(cable) = inline_cable {
                        wire_inline(handle, editor, next_id, &descriptor, cable);
                    }
                }
            }
        }
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

    fn draw_keyboard(&mut self, ui: &mut egui::Ui) {
        // Always use CH1 for keyboard input - focused_instrument handles routing
        let active_channel = MidiChannel::CH1;

        let active_name = self
            .instruments
            .iter()
            .find(|p| p.id == self.active_instrument_id)
            .map(|p| p.name.as_str())
            .unwrap_or("Instrument 1");

        // Layout: [Left Scope] [Piano Keys] [Right Scope] [Meter] [margin]
        let available_width = ui.available_width();
        let meter_width = 30.0;
        let meter_margin = 24.0;
        let item_spacing = ui.spacing().item_spacing.x;
        let keys_height = 130.0;

        // Estimate spacings (max 3 gaps: between ScopeL-Piano, Piano-ScopeR, ScopeR-Meter)
        let max_spacings = 3.0 * item_spacing;

        // Piano: calculate octaves that fit, then its exact width
        use crate::gui::keyboard::PianoKeyboard;
        let piano_budget = available_width - meter_width - meter_margin - max_spacings;
        let num_octaves = PianoKeyboard::octaves_for_width(piano_budget.max(0.0));
        let piano_width = PianoKeyboard::width_for_octaves(num_octaves);

        // Scopes share remaining space equally
        let scope_total =
            (available_width - piano_width - meter_width - meter_margin - max_spacings).max(0.0);
        let min_scope_width = 40.0;
        let show_scopes = scope_total >= min_scope_width * 2.0;
        let scope_width = if show_scopes { scope_total / 2.0 } else { 0.0 };

        let (samples_l, samples_r) = if show_scopes {
            self.handle.state.master_scope.read_samples()
        } else {
            (Vec::new(), Vec::new())
        };

        let (peak_l, peak_r) = self.handle.peak_meters();
        let (rms_l, rms_r) = self.handle.rms_meters();

        ui.horizontal(|ui| {
            if show_scopes {
                draw_oscilloscope(
                    ui,
                    &samples_l,
                    scope_width,
                    keys_height,
                    1.0,
                    theme().colors.accent_cyan,
                );
            }

            ui.allocate_ui(Vec2::new(piano_width, keys_height), |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("Active: {}", active_name))
                                .color(theme().colors.accent_orange),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.keyboard.show_header(ui);
                        });
                    });

                    let event = self.keyboard.show_keys(ui);

                    if let Some(note) = event.note_on {
                        self.handle
                            .note_on_channel(note, Velocity::new(0.8), active_channel);
                    }
                    for note in event.note_off {
                        self.handle.note_off_channel(note, active_channel);
                    }
                });
            });

            if show_scopes {
                draw_oscilloscope(
                    ui,
                    &samples_r,
                    scope_width,
                    keys_height,
                    1.0,
                    theme().colors.meter_green,
                );
            }

            // Vertical stereo level meter
            draw_stereo_meter(ui, peak_l, peak_r, rms_l, rms_r, meter_width, keys_height);
            ui.add_space(meter_margin);
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
        if show_settings_dialog(
            ctx,
            &mut self.dialog_state.show_settings,
            &mut self.dialog_state.current_theme,
            &mut self.settings,
        ) {
            self.settings.save();
        }

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
                        snap.from_port,
                        snap.to_module,
                        snap.to_port,
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

            // Trigger auto-layout so MCP-added modules get proper positions
            patch_editor.request_initial_layout();
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
