//! Egui-based graphical GUI backend.
//!
//! This provides a full graphical interface using egui/eframe,
//! with a Pertylizer-style layout featuring:
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
    DialogState, FileDialogMode, FileDialogResult, GroupTemplateBrowserResult, LoadPatchResult,
    SaveGroupTemplateResult, SettingsAction, show_about_dialog, show_group_template_browser,
    show_load_patch_dialog, show_save_group_template_dialog, show_settings_dialog,
    show_status_toast,
};
use crate::gui::input::handle_keyboard_input;
use crate::gui::instrument_rack::InstrumentUiState;
use crate::gui::keyboard::PianoKeyboard;
use crate::gui::patch_bridge;
use crate::gui::patch_editor::{
    EffectType, GroupTemplateAction, PaletteSelection, PaletteVisualizerType, PatchEditor,
    QuickAddRequest,
};
use crate::gui::theme::theme;
use crate::gui::widgets::{draw_oscilloscope, draw_stereo_meter};
use crate::gui::{GuiBackend, GuiResult, SynthGuiConfig};
use crate::io::settings::AppSettings;
use crate::io::{GroupTemplateManager, MidiHandler, PatchManager};
use crate::patch::{GroupCategory, InstrumentState, Patch, categorized_patches};
use crate::project::{self, GlobalProjectState, LoadedFile, ProjectFile};
use synth_core::{Describable, ModuleCategory};
use synth_core::{Seconds, Velocity};
use synth_engine::ModuleType as TypedModuleType;
use synth_engine::commands::PortId;
use synth_engine::instrument::{InstrumentId, MidiChannel};
use synth_engine::visualizers::{LevelMeter, Oscilloscope, SpectrumAnalyzer};
use synth_engine::{EngineCommand, EngineEvent, EngineHandle, ModuleId, SynthEngine};

/// Action deferred until the user responds to the unsaved-changes dialog.
enum PendingAction {
    /// Create a new project.
    NewProject,
    /// Open a project via file dialog.
    OpenProject,
    /// Load a specific project file.
    LoadProject(PathBuf),
    /// Quit the application.
    Quit,
}

/// State for the "unsaved changes" confirmation dialog.
#[derive(Default)]
struct UnsavedChangesDialog {
    /// Whether the dialog is currently visible.
    open: bool,
    /// The action to perform once the user responds.
    pending_action: Option<PendingAction>,
}

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

    // Project state
    current_project_path: Option<PathBuf>,

    // Global synth settings
    glide_time: synth_core::Seconds,

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

    // OSC shared state
    #[cfg(feature = "osc")]
    osc_shared: Option<synth_osc::OscSharedState>,

    // Persistent application settings
    settings: crate::io::settings::AppSettings,

    // Undo/redo manager
    undo_manager: crate::undo::UndoManager,

    /// Whether the current project has unsaved changes.
    dirty: bool,

    /// Unsaved changes confirmation dialog state.
    unsaved_dialog: UnsavedChangesDialog,

    /// Module clipboard for copy/paste.
    clipboard: crate::gui::clipboard::ModuleClipboard,

    /// Flag to clear egui Area positions on the next frame.
    /// Set when loading a project so that module positions are correctly
    /// restored from the saved layout instead of using stale cached positions.
    needs_area_reset: bool,
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
        let mut glide_time = Seconds::new(0.0);

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

        // Set instrument name to match the loaded patch (both UI and engine)
        default_instrument.name = patch_name.clone();
        handle.send_blocking(EngineCommand::RenameInstrument {
            instrument_id: active_instrument_id,
            name: patch_name.clone(),
        });

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
            current_project_path: None,
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
            #[cfg(feature = "osc")]
            osc_shared: config.osc_shared,
            settings,
            undo_manager: crate::undo::UndoManager::new(),
            dirty: false,
            unsaved_dialog: UnsavedChangesDialog::default(),
            clipboard: crate::gui::clipboard::ModuleClipboard::new(),
            needs_area_reset: false,
        }
    }

    /// Mark the project as having unsaved changes.
    fn mark_dirty(&mut self) {
        self.dirty = true;
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

        // If a project was just loaded, clear cached egui Area positions so that
        // module windows are placed at their saved positions, not stale ones.
        if self.needs_area_reset {
            self.needs_area_reset = false;
            ctx.memory_mut(|mem| mem.reset_areas());
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
                EngineEvent::RecordingPreview {
                    completed,
                    held,
                    pattern_length,
                } => {
                    self.sequencer_view_state.recording_preview_completed = completed;
                    self.sequencer_view_state.recording_preview_held = held;
                    self.sequencer_view_state.recording_preview_pattern_length = pattern_length;
                }
                EngineEvent::RecordedNotesFlushed {
                    pattern_id,
                    notes,
                    overdub,
                } => {
                    // Write recorded notes into the pattern (on UI thread, safe to lock)
                    if let Ok(mut song) = self.song.write()
                        && let Some(pattern) = song.pattern_mut(pattern_id)
                    {
                        if !overdub {
                            pattern.clear_notes();
                        }
                        for note in &notes {
                            let nid = pattern.add_note(
                                note.start,
                                note.pitch,
                                note.velocity,
                                synth_sequencer::SeqInstrumentId(0),
                            );
                            if let Some(n) = pattern.note_mut(nid) {
                                n.duration = Some(note.duration);
                            }
                        }
                    }
                    self.mark_dirty();

                    // Clear preview — notes are now committed
                    self.sequencer_view_state
                        .recording_preview_completed
                        .clear();
                    self.sequencer_view_state.recording_preview_held.clear();
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

        // Poll MCP pending auto-layout
        #[cfg(feature = "mcp")]
        let mcp_auto_layout = self.mcp_shared.as_ref().is_some_and(|shared| {
            shared
                .pending_auto_layout
                .swap(false, std::sync::atomic::Ordering::Relaxed)
        });
        #[cfg(not(feature = "mcp"))]
        let mcp_auto_layout = false;

        // Poll MCP pending project action
        #[cfg(feature = "mcp")]
        {
            use crate::mcp_shared::ProjectAction;

            let action = self.mcp_shared.as_ref().and_then(|shared| {
                shared
                    .pending_project_action
                    .lock()
                    .ok()
                    .and_then(|mut a| a.take())
            });
            if let Some(action) = action {
                let result = match action {
                    ProjectAction::New => {
                        self.reset_to_new_project();
                        Ok("New project created".to_string())
                    }
                    ProjectAction::Save(path) => {
                        let proj = self.create_project_from_app();
                        proj.save(&path)
                            .map(|()| format!("Saved to {}", path.display()))
                            .map_err(|e| e.to_string())
                    }
                    ProjectAction::Load(path) => match project::load_file(&path) {
                        Ok(LoadedFile::Project(proj)) => {
                            self.load_project_data(proj);
                            self.current_project_path = Some(path.clone());
                            Ok(format!("Loaded {}", path.display()))
                        }
                        Ok(LoadedFile::Patch(patch)) => {
                            self.current_patch_name = patch.name.clone();
                            self.current_patch_path = Some(path.clone());
                            self.load_patch_data(&patch);
                            Ok(format!("Loaded patch from {}", path.display()))
                        }
                        Err(e) => Err(e.to_string()),
                    },
                };
                if let Some(shared) = &self.mcp_shared {
                    let (lock, cvar) = &shared.project_action_result;
                    if let Ok(mut guard) = lock.lock() {
                        *guard = Some(result);
                        cvar.notify_one();
                    }
                }
            }
        }

        // Reconcile with session: detect modules added/removed by MCP
        #[cfg(feature = "mcp")]
        self.reconcile_with_session();

        // Handle keyboard input
        self.process_keyboard_input(ctx);

        // ── Undo/Redo keyboard shortcuts ──
        self.handle_undo_redo_shortcuts(ctx);

        // ── Copy/Paste/Duplicate keyboard shortcuts ──
        self.handle_clipboard_shortcuts(ctx);

        // Request continuous repaint for meters
        ctx.request_repaint();

        // Top menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    // --- Project ---
                    if ui
                        .button(format!("{} New Project", ri::FILE_ADD_LINE))
                        .clicked()
                    {
                        if self.dirty {
                            self.unsaved_dialog.pending_action = Some(PendingAction::NewProject);
                            self.unsaved_dialog.open = true;
                        } else {
                            self.reset_to_new_project();
                            self.dirty = false;
                            self.dialog_state
                                .set_status("New project created".to_string());
                        }
                        ui.close();
                    }
                    if ui
                        .button(format!("{} Open Project...", ri::FOLDER_OPEN_LINE))
                        .clicked()
                    {
                        if self.dirty {
                            self.unsaved_dialog.pending_action = Some(PendingAction::OpenProject);
                            self.unsaved_dialog.open = true;
                        } else {
                            let initial_dir = self.resolve_project_dir();
                            self.dialog_state
                                .open_open_project_dialog(initial_dir.as_deref());
                        }
                        ui.close();
                    }
                    if ui
                        .button(format!("{} Save Project", ri::SAVE_LINE))
                        .clicked()
                    {
                        if let Some(path) = self.current_project_path.clone() {
                            let proj = self.create_project_from_app();
                            match proj.save(&path) {
                                Ok(()) => {
                                    self.dirty = false;
                                    self.settings.add_recent_project(path.clone());
                                    self.settings.save();
                                    self.dialog_state
                                        .set_status(format!("Project saved: {}", path.display()));
                                }
                                Err(e) => {
                                    self.dialog_state
                                        .set_status(format!("Error saving project: {e}"));
                                }
                            }
                        } else {
                            // No path yet — open Save As dialog
                            let default_name = "project.json".to_string();
                            let initial_dir = self.resolve_project_dir();
                            self.dialog_state
                                .open_save_project_dialog(&default_name, initial_dir.as_deref());
                        }
                        ui.close();
                    }
                    if ui
                        .button(format!("{} Save Project As...", ri::SAVE_LINE))
                        .clicked()
                    {
                        let default_name = self
                            .current_project_path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .and_then(|n| n.to_str())
                            .unwrap_or("project.json")
                            .to_string();
                        let initial_dir = self.resolve_project_dir();
                        self.dialog_state
                            .open_save_project_dialog(&default_name, initial_dir.as_deref());
                        ui.close();
                    }
                    // --- Recent Projects ---
                    ui.menu_button(format!("{} Recent Projects", ri::HISTORY_LINE), |ui| {
                        let projects = self.settings.recent_projects.clone();
                        if projects.is_empty() {
                            ui.label("(none)");
                        } else {
                            for path in &projects {
                                let label =
                                    path.file_name().and_then(|n| n.to_str()).unwrap_or("???");
                                let btn =
                                    ui.button(label).on_hover_text(path.display().to_string());
                                if btn.clicked() {
                                    if self.dirty {
                                        self.unsaved_dialog.pending_action =
                                            Some(PendingAction::LoadProject(path.clone()));
                                        self.unsaved_dialog.open = true;
                                    } else {
                                        self.load_recent_project(path.clone());
                                    }
                                    ui.close();
                                }
                            }
                            ui.separator();
                            if ui.button("Clear Recent").clicked() {
                                self.settings.clear_recent_projects();
                                self.settings.save();
                                ui.close();
                            }
                        }
                    });
                    ui.separator();

                    // --- Patch ---
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
                        let initial_dir = self.resolve_open_dir();
                        self.dialog_state
                            .open_open_patch_dialog(initial_dir.as_deref());
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
                        let initial_dir = self.resolve_save_dir();
                        self.dialog_state
                            .open_save_patch_dialog(&default_name, initial_dir.as_deref());
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
                        .button(format!("{} Export WAV...", ri::DOWNLOAD_LINE))
                        .clicked()
                    {
                        // Pre-fill duration from song length
                        let song_secs =
                            self.song.read().map(|s| s.length_seconds()).unwrap_or(60.0);
                        self.dialog_state
                            .export_state
                            .set_duration_from_song(song_secs);
                        // Open file dialog to choose WAV path
                        let default_name = "export.wav".to_string();
                        let initial_dir = self.resolve_project_dir();
                        self.dialog_state
                            .open_export_wav_dialog(&default_name, initial_dir.as_deref());
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(format!("{} Settings...", ri::SETTINGS_LINE))
                        .clicked()
                    {
                        // Reload settings from disk to pick up changes
                        // made outside the dialog (e.g. last_open_dir)
                        self.settings = AppSettings::load();
                        self.dialog_state.show_settings = true;
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(format!("{} Quit", ri::SHUT_DOWN_LINE)).clicked() {
                        if self.dirty {
                            self.unsaved_dialog.pending_action = Some(PendingAction::Quit);
                            self.unsaved_dialog.open = true;
                        } else {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        ui.close();
                    }
                });

                ui.menu_button("Edit", |ui| {
                    let undo_label = format!("{} Undo", ri::ARROW_GO_BACK_LINE);
                    let redo_label = format!("{} Redo", ri::ARROW_GO_FORWARD_LINE);
                    if ui
                        .add_enabled(self.undo_manager.can_undo(), egui::Button::new(&undo_label))
                        .on_hover_text("Ctrl+Z")
                        .clicked()
                    {
                        self.execute_undo();
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.undo_manager.can_redo(), egui::Button::new(&redo_label))
                        .on_hover_text("Ctrl+Shift+Z")
                        .clicked()
                    {
                        self.execute_redo();
                        ui.close();
                    }
                    ui.separator();
                    let has_selection = self
                        .active_patch_editor_ref()
                        .is_some_and(|e| !e.effective_selection().is_empty());
                    if ui
                        .add_enabled(
                            has_selection,
                            egui::Button::new(format!("{} Copy", ri::FILE_COPY_LINE)),
                        )
                        .on_hover_text("Ctrl+C")
                        .clicked()
                    {
                        self.copy_selected_modules();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            !self.clipboard.is_empty(),
                            egui::Button::new(format!("{} Paste", ri::CLIPBOARD_LINE)),
                        )
                        .on_hover_text("Ctrl+V")
                        .clicked()
                    {
                        self.paste_modules_at_offset();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            has_selection,
                            egui::Button::new(format!("{} Duplicate", ri::FILE_COPY_2_LINE)),
                        )
                        .on_hover_text("Ctrl+D")
                        .clicked()
                    {
                        self.duplicate_selected_modules();
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(format!("{} Optimize Project", ri::DELETE_BIN_LINE))
                        .on_hover_text("Remove unused patterns, tracks, and instruments")
                        .clicked()
                    {
                        self.optimize_project();
                        ui.close();
                    }
                });

                ui.menu_button("Help", |ui| {
                    if ui.button("About").clicked() {
                        self.dialog_state.show_about = true;
                        ui.close();
                    }
                });

                // View selector — segmented control (right after Help menu)
                ui.separator();
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
                    let (outer_rect, _) =
                        ui.allocate_exact_size(egui::vec2(total_w, seg_h), egui::Sense::hover());
                    let painter = ui.painter_at(outer_rect);

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

                        if is_active {
                            painter.rect_filled(
                                seg_rect,
                                seg_rounding,
                                t.colors.accent_primary.gamma_multiply(0.55),
                            );
                        }

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
                    // MIDI status indicator (with port selector on click)
                    {
                        let (icon, color, hover_text) = if self.midi_handler.is_connected() {
                            let port_name = self
                                .midi_handler
                                .port_name()
                                .unwrap_or("Unknown")
                                .to_owned();
                            (
                                ri::PIANO_FILL,
                                theme().colors.meter_green,
                                format!("MIDI: connected to {port_name}"),
                            )
                        } else {
                            (
                                ri::PIANO_LINE,
                                theme().colors.text_dim,
                                "MIDI: not connected".to_owned(),
                            )
                        };
                        let arrow = ri::ARROW_DOWN_S_FILL;
                        let midi_label = RichText::new(format!("{icon} MIDI {arrow}")).color(color);
                        let resp = ui.menu_button(midi_label, |ui| {
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
                                        RichText::new(format!("  {port}"))
                                    };
                                    if ui.button(label).clicked() {
                                        if let Err(e) = self.midi_handler.connect_to(port) {
                                            eprintln!("MIDI connection error: {e}");
                                        }
                                        ui.close();
                                    }
                                }
                            }
                        });
                        resp.response.on_hover_text(hover_text);
                    }
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
                            let tooltip = if sessions > 0 {
                                let session_list = mcp.mcp_sessions.sessions();
                                if session_list.is_empty() {
                                    format!("MCP: {sessions} active session(s)")
                                } else {
                                    let mut text =
                                        format!("MCP: {} active session(s)\n", session_list.len());
                                    for s in &session_list {
                                        text.push_str(&format!(
                                            "\n  {} v{} (MCP {})",
                                            s.client_name, s.client_version, s.protocol_version
                                        ));
                                    }
                                    text
                                }
                            } else if listening {
                                "MCP: listening (no active sessions)".to_owned()
                            } else {
                                "MCP: not running".to_owned()
                            };
                            resp.on_hover_text(tooltip);
                        }
                        ui.separator();
                    }
                    // OSC telemetry status indicator
                    #[cfg(feature = "osc")]
                    {
                        let osc_status = self
                            .osc_shared
                            .as_ref()
                            .map_or(synth_osc::OscStatus::Off, |s| s.status());
                        let (icon, label, color) = match osc_status {
                            synth_osc::OscStatus::Connected => {
                                (ri::BROADCAST_FILL, "OSC", theme().colors.meter_green)
                            }
                            synth_osc::OscStatus::Idle => {
                                (ri::BROADCAST_LINE, "OSC", theme().colors.text_dim)
                            }
                            synth_osc::OscStatus::Off => {
                                (ri::BROADCAST_LINE, "OSC", theme().colors.accent_red)
                            }
                        };
                        let resp = ui.label(RichText::new(format!("{icon} {label}")).color(color));
                        if resp.hovered() {
                            resp.on_hover_text(match osc_status {
                                synth_osc::OscStatus::Connected => "OSC: visualizer connected",
                                synth_osc::OscStatus::Idle => "OSC: sending beacon (no visualizer)",
                                synth_osc::OscStatus::Off => "OSC: disabled",
                            });
                        }
                        ui.separator();
                    }
                    // AWE (Acoustic World Engine) status indicator with preset menu
                    {
                        let presets = synth_awe::presets::awe_presets();
                        let preset_name = self
                            .awe_ui
                            .selected_preset
                            .and_then(|i| presets.get(i).map(|p| p.name.to_owned()));
                        let (icon, color, hover_text) = if self.awe_enabled {
                            let name = preset_name.as_deref().unwrap_or("Custom");
                            (
                                ri::SURROUND_SOUND_FILL,
                                theme().colors.meter_green,
                                format!("AWE: {name}"),
                            )
                        } else {
                            (
                                ri::SURROUND_SOUND_LINE,
                                theme().colors.text_dim,
                                "AWE: off".to_owned(),
                            )
                        };
                        let arrow = ri::ARROW_DOWN_S_FILL;
                        let awe_label = RichText::new(format!("{icon} AWE {arrow}")).color(color);
                        let resp = ui.menu_button(awe_label, |ui| {
                            ui.set_min_width(250.0);
                            // Off option
                            let is_off = !self.awe_enabled;
                            let off_label = if is_off {
                                RichText::new(format!("{} Off", ri::CHECKBOX_BLANK_CIRCLE_FILL))
                                    .color(theme().colors.text_dim)
                            } else {
                                RichText::new("  Off")
                            };
                            if ui.button(off_label).clicked() {
                                self.awe_enabled = false;
                                self.awe_ui.selected_preset = None;
                                self.handle
                                    .send(EngineCommand::SetAweEnabled { enabled: false });
                                self.mark_dirty();
                                ui.close();
                            }
                            ui.separator();
                            // Standard presets
                            let standard: Vec<usize> = presets
                                .iter()
                                .enumerate()
                                .filter(|(_, p)| !p.name.starts_with("EXT:"))
                                .map(|(i, _)| i)
                                .collect();
                            let extreme: Vec<usize> = presets
                                .iter()
                                .enumerate()
                                .filter(|(_, p)| p.name.starts_with("EXT:"))
                                .map(|(i, _)| i)
                                .collect();
                            if !standard.is_empty() {
                                for i in &standard {
                                    let preset = &presets[*i];
                                    let is_current =
                                        self.awe_enabled && self.awe_ui.selected_preset == Some(*i);
                                    let label = if is_current {
                                        RichText::new(format!(
                                            "{} {}",
                                            ri::CHECKBOX_BLANK_CIRCLE_FILL,
                                            preset.name
                                        ))
                                        .color(theme().colors.meter_green)
                                    } else {
                                        RichText::new(format!("  {}", preset.name))
                                    };
                                    if ui.button(label).on_hover_text(preset.description).clicked()
                                    {
                                        crate::gui::awe_view::apply_awe_preset(
                                            *i,
                                            preset,
                                            &mut self.handle,
                                            &mut self.awe_enabled,
                                            &mut self.awe_ui,
                                        );
                                        self.mark_dirty();
                                        ui.close();
                                    }
                                }
                            }
                            if !extreme.is_empty() {
                                ui.separator();
                                ui.label(RichText::new("Extreme").color(theme().colors.text_dim));
                                for i in &extreme {
                                    let preset = &presets[*i];
                                    let is_current =
                                        self.awe_enabled && self.awe_ui.selected_preset == Some(*i);
                                    let label = if is_current {
                                        RichText::new(format!(
                                            "{} {}",
                                            ri::CHECKBOX_BLANK_CIRCLE_FILL,
                                            preset.name.trim_start_matches("EXT: ")
                                        ))
                                        .color(theme().colors.meter_green)
                                    } else {
                                        RichText::new(format!(
                                            "  {}",
                                            preset.name.trim_start_matches("EXT: ")
                                        ))
                                    };
                                    if ui.button(label).on_hover_text(preset.description).clicked()
                                    {
                                        crate::gui::awe_view::apply_awe_preset(
                                            *i,
                                            preset,
                                            &mut self.handle,
                                            &mut self.awe_enabled,
                                            &mut self.awe_ui,
                                        );
                                        self.mark_dirty();
                                        ui.close();
                                    }
                                }
                            }
                        });
                        resp.response.on_hover_text(hover_text);
                        ui.separator();
                    }
                    // Project name
                    let project_name = self
                        .current_project_path
                        .as_ref()
                        .and_then(|p| p.file_stem())
                        .and_then(|n| n.to_str())
                        .unwrap_or("Untitled");
                    let dirty_marker = if self.dirty { " *" } else { "" };
                    ui.label(
                        RichText::new(format!("{project_name}{dirty_marker}"))
                            .color(theme().colors.text_secondary),
                    );
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
            let had_mutations = result.has_mutations();

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

                self.undo_manager
                    .push(crate::undo::UndoAction::AddConnection {
                        instrument_id: active_id,
                        connection,
                    });

                // Send Connect command to engine (active instrument's voice graph)
                self.handle.send(EngineCommand::Connect {
                    instrument_id: Some(active_id),
                    from: PortId::new(connection.from_module, connection.from_port),
                    to: PortId::new(connection.to_module, connection.to_port),
                });
            }

            // Handle removed connections - send Disconnect commands to engine
            for connection in result.connections_to_remove {
                self.undo_manager
                    .push(crate::undo::UndoAction::RemoveConnection {
                        instrument_id: active_id,
                        connection,
                    });

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

            // Handle group template actions (open browser / save template)
            if let Some(action) = result.group_template_action {
                match action {
                    GroupTemplateAction::OpenBrowser { drop_pos } => {
                        self.dialog_state.show_group_templates = true;
                        self.dialog_state.group_template_drop_pos = Some(drop_pos);
                        self.dialog_state.group_template_selected = None;
                    }
                    GroupTemplateAction::SaveGroup { group_id } => {
                        self.dialog_state.show_save_group_template = true;
                        self.dialog_state.group_template_save_group = Some(group_id);
                        if let Some(name) = patch_editor.group_name(group_id) {
                            self.dialog_state.group_template_save_name = name;
                        }
                        self.dialog_state.group_template_save_description.clear();
                        self.dialog_state.group_template_save_category = GroupCategory::default();
                    }
                }
            }

            // Handle auto-layout request (from GUI menu or MCP)
            if (result.request_auto_layout || mcp_auto_layout)
                && let Some(canvas_rect) = result.canvas_rect
            {
                patch_editor.apply_auto_layout(canvas_rect);
            }

            // Mark dirty if any mutations occurred
            if had_mutations {
                self.mark_dirty();
            }
                });
            }
            AppView::AcousticWorld => {
                crate::gui::awe_view::draw_awe_view(
                    ctx,
                    &mut self.handle,
                    &mut self.awe_enabled,
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
                    &mut self.undo_manager,
                );
            }
        }

        // Dialogs
        self.show_dialogs(ctx);

        // Write current UI layout to MCP shared state
        #[cfg(feature = "mcp")]
        self.write_mcp_layout(ctx);

        // Update window title to reflect dirty state
        {
            let project_name = self
                .current_project_path
                .as_ref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled");
            let title = if self.dirty {
                format!("Pertylizer - {project_name} *")
            } else {
                format!("Pertylizer - {project_name}")
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // Intercept close request when there are unsaved changes
        if ctx.input(|i| i.viewport().close_requested()) && self.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.unsaved_dialog.pending_action = Some(PendingAction::Quit);
            self.unsaved_dialog.open = true;
        }

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
            PaletteSelection::FractalOsc => Some(TypedModuleType::FractalOsc),
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
                    PaletteSelection::FractalOsc => Some(TypedModuleType::FractalOsc),
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

    /// Draw the instrument control strip above the piano keyboard.
    /// Contains: instrument selector, MIDI channel, vol, pan, solo, mute, glide, module/connection counts.
    fn draw_instrument_strip(&mut self, ui: &mut egui::Ui) {
        use egui_remixicon::icons as ri;

        let t = theme();

        ui.horizontal(|ui| {
            // Use small font for all widgets in the strip
            ui.style_mut().text_styles.insert(
                egui::TextStyle::Body,
                egui::FontId::proportional(t.fonts.size_small),
            );
            ui.style_mut().text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::proportional(t.fonts.size_small),
            );

            // Instrument selector dropdown
            let active_name = self
                .instruments
                .iter()
                .find(|i| i.id == self.active_instrument_id)
                .map(|i| i.name.as_str())
                .unwrap_or("(none)");
            let menu_label = RichText::new(format!(
                "{} {active_name} {}",
                ri::MUSIC_2_FILL,
                ri::ARROW_DOWN_S_FILL
            ))
            .color(t.colors.accent_cyan)
            .size(t.fonts.size_small);
            ui.menu_button(menu_label, |ui| {
                // New Instrument option at the top
                if ui
                    .button(
                        RichText::new(format!("{} New Instrument", ri::ADD_LINE))
                            .color(t.colors.accent_green),
                    )
                    .clicked()
                {
                    let new_id = InstrumentId::new(self.next_instrument_id);
                    self.next_instrument_id += 1;

                    let instrument_num = self.instruments.len() + 1;
                    let new_name = format!("Instrument {instrument_num}");
                    let new_channel = MidiChannel::from_one_indexed(instrument_num as u8)
                        .unwrap_or(MidiChannel::CH1);

                    let new_ui_instrument =
                        InstrumentUiState::new(new_id, &new_name).with_channel(new_channel);

                    let mut engine_instrument =
                        synth_engine::instrument::Instrument::new(new_id, &new_name);
                    engine_instrument.set_midi_channel(new_channel);

                    self.handle.send(EngineCommand::AddInstrument {
                        instrument: Box::new(engine_instrument),
                    });

                    self.instruments.push(new_ui_instrument);
                    self.active_instrument_id = new_id;
                    self.handle.set_focused_instrument(Some(new_id));
                    self.mark_dirty();
                    ui.close();
                }

                ui.separator();

                if self.instruments.is_empty() {
                    ui.label(RichText::new("No instruments").color(t.colors.text_dim));
                } else {
                    for inst in &self.instruments {
                        let is_active = inst.id == self.active_instrument_id;
                        let label = if is_active {
                            RichText::new(format!(
                                "{} {}",
                                ri::CHECKBOX_BLANK_CIRCLE_FILL,
                                inst.name
                            ))
                            .color(t.colors.accent_cyan)
                        } else {
                            RichText::new(format!("  {}", inst.name))
                        };
                        if ui.button(label).clicked() {
                            self.active_instrument_id = inst.id;
                            self.handle.set_focused_instrument(Some(inst.id));
                            ui.close();
                        }
                    }
                }
            });
            ui.separator();

            // Find the active instrument index for modifying controls
            let active_idx = self
                .instruments
                .iter()
                .position(|i| i.id == self.active_instrument_id);

            if let Some(idx) = active_idx {
                let instrument_id = self.instruments[idx].id;

                // MIDI Channel dropdown
                let channel = self.instruments[idx].channel;
                let channel_label = if channel.is_omni() {
                    "Omni".to_string()
                } else {
                    format!("Ch {}", channel.as_one_indexed())
                };
                egui::ComboBox::from_id_salt("strip_midi_ch")
                    .selected_text(RichText::new(&channel_label).size(t.fonts.size_small))
                    .width(50.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(channel.is_omni(), "Omni").clicked() {
                            self.instruments[idx].channel = MidiChannel::OMNI;
                            self.handle.send(EngineCommand::SetInstrumentMidiChannel {
                                instrument_id,
                                channel: MidiChannel::OMNI,
                            });
                        }
                        for ch in 1..=16u8 {
                            let Some(midi_ch) = MidiChannel::from_one_indexed(ch) else {
                                continue;
                            };
                            let is_selected = !channel.is_omni() && channel.as_one_indexed() == ch;
                            if ui
                                .selectable_label(is_selected, format!("Ch {ch}"))
                                .clicked()
                            {
                                self.instruments[idx].channel = midi_ch;
                                self.handle.send(EngineCommand::SetInstrumentMidiChannel {
                                    instrument_id,
                                    channel: midi_ch,
                                });
                            }
                        }
                    })
                    .response
                    .on_hover_text("MIDI channel for this instrument");
                ui.separator();

                // Volume slider
                let muted = self.instruments[idx].muted;
                ui.label(
                    RichText::new("Vol:")
                        .color(t.colors.text_dim)
                        .size(t.fonts.size_small),
                );
                let mut vol = self.instruments[idx].volume.as_f32();
                let vol_response = ui
                    .add(
                        egui::Slider::new(&mut vol, 0.0..=1.0)
                            .fixed_decimals(2)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    )
                    .on_hover_text("Instrument volume");
                if vol_response.changed() && !muted {
                    self.instruments[idx].set_volume(synth_core::Gain::new(vol));
                    self.handle.send(EngineCommand::SetInstrumentParameter {
                        instrument_id,
                        param: synth_engine::InstrumentParam::Volume(self.instruments[idx].volume),
                    });
                }

                // Pan slider
                ui.label(
                    RichText::new("Pan:")
                        .color(t.colors.text_dim)
                        .size(t.fonts.size_small),
                );
                let current_pan = self.instruments[idx].pan.as_f32();
                let mut pan = current_pan;
                let pan_response = ui
                    .add(
                        egui::Slider::new(&mut pan, -1.0..=1.0)
                            .fixed_decimals(2)
                            .custom_formatter(|v, _| {
                                if v.abs() < 0.01 {
                                    "C".to_string()
                                } else if v < 0.0 {
                                    format!("L{:.0}", -v * 100.0)
                                } else {
                                    format!("R{:.0}", v * 100.0)
                                }
                            }),
                    )
                    .on_hover_text("Stereo pan position");
                if pan_response.changed() {
                    self.instruments[idx].pan = synth_core::BipolarValue::new(pan);
                    self.handle.send(EngineCommand::SetInstrumentParameter {
                        instrument_id,
                        param: synth_engine::InstrumentParam::Pan(self.instruments[idx].pan),
                    });
                }

                // Solo button
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("S")
                                .color(t.colors.accent_yellow)
                                .size(t.fonts.size_small),
                        )
                        .min_size(egui::vec2(24.0, 24.0)),
                    )
                    .on_hover_text("Solo: mute all other instruments")
                    .clicked()
                {
                    for (i, inst) in self.instruments.iter_mut().enumerate() {
                        if i == idx {
                            if inst.muted {
                                let vol = inst.toggle_mute();
                                self.handle.send(EngineCommand::SetInstrumentParameter {
                                    instrument_id: inst.id,
                                    param: synth_engine::InstrumentParam::Volume(vol),
                                });
                            }
                        } else if !inst.muted {
                            let vol = inst.toggle_mute();
                            self.handle.send(EngineCommand::SetInstrumentParameter {
                                instrument_id: inst.id,
                                param: synth_engine::InstrumentParam::Volume(vol),
                            });
                        }
                    }
                }

                // Mute button
                let mute_color = if muted {
                    t.colors.accent_red
                } else {
                    t.colors.text_dim
                };
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("M")
                                .color(mute_color)
                                .size(t.fonts.size_small),
                        )
                        .min_size(egui::vec2(24.0, 24.0)),
                    )
                    .on_hover_text("Mute this instrument")
                    .clicked()
                {
                    let new_volume = self.instruments[idx].toggle_mute();
                    self.handle.send(EngineCommand::SetInstrumentParameter {
                        instrument_id,
                        param: synth_engine::InstrumentParam::Volume(new_volume),
                    });
                }

                // Transpose
                ui.label(
                    RichText::new("Trans:")
                        .color(t.colors.text_dim)
                        .size(t.fonts.size_small),
                );
                let mut transpose = self.instruments[idx].transpose.as_f32().round() as i32;
                let trans_response = ui
                    .add(
                        egui::DragValue::new(&mut transpose)
                            .range(-24..=24)
                            .speed(0.1)
                            .suffix(" st"),
                    )
                    .on_hover_text("Transpose in semitones (-24 to +24)");
                if trans_response.changed() {
                    let new_transpose = synth_core::Semitones::new(transpose.clamp(-24, 24) as f32);
                    self.instruments[idx].transpose = new_transpose;
                    self.handle.send(EngineCommand::SetInstrumentParameter {
                        instrument_id,
                        param: synth_engine::InstrumentParam::Transpose(new_transpose),
                    });
                }

                // Oversampling
                ui.label(
                    RichText::new("OS:")
                        .color(t.colors.text_dim)
                        .size(t.fonts.size_small),
                );
                let current_os = self.instruments[idx].oversampling;
                egui::ComboBox::from_id_salt("strip_os")
                    .selected_text(RichText::new(current_os.name()).size(t.fonts.size_small))
                    .width(40.0)
                    .show_ui(ui, |ui| {
                        for factor in synth_dsp::OversamplingFactor::ALL {
                            if ui
                                .selectable_label(current_os == factor, factor.name())
                                .clicked()
                            {
                                self.instruments[idx].oversampling = factor;
                                self.handle.send(EngineCommand::SetInstrumentParameter {
                                    instrument_id,
                                    param: synth_engine::InstrumentParam::OversamplingFactor(
                                        factor,
                                    ),
                                });
                            }
                        }
                    })
                    .response
                    .on_hover_text("Oversampling factor (reduces aliasing)");

                ui.separator();

                // Glide slider
                ui.label(
                    RichText::new("Glide:")
                        .color(t.colors.text_dim)
                        .size(t.fonts.size_small),
                );
                let mut glide_val = self.glide_time.as_f32();
                let glide_response = ui
                    .add(
                        egui::Slider::new(&mut glide_val, 0.0..=2.0)
                            .suffix(" s")
                            .fixed_decimals(2)
                            .custom_formatter(|v, _| {
                                if v < 0.001 {
                                    "Off".to_string()
                                } else {
                                    format!("{v:.2}s")
                                }
                            }),
                    )
                    .on_hover_text("Portamento glide time between notes");
                if glide_response.changed() {
                    self.glide_time = synth_core::Seconds::new(glide_val);
                    self.handle
                        .send(EngineCommand::SetGlideTime(self.glide_time));
                }

                ui.separator();

                // Module and connection counts (only meaningful in Rack view)
                let (conn_count, module_count) = self
                    .active_patch_editor_ref()
                    .map(|e| (e.connections().len(), e.module_ids().len()))
                    .unwrap_or((0, 0));
                ui.label(
                    RichText::new(format!("M:{module_count} C:{conn_count}"))
                        .color(t.colors.text_dim)
                        .size(t.fonts.size_small),
                )
                .on_hover_text("Modules and connections in current patch");
            }
        });
    }

    fn draw_keyboard(&mut self, ui: &mut egui::Ui) {
        // Always use CH1 for keyboard input - focused_instrument handles routing
        let active_channel = MidiChannel::CH1;

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
        let (peak_l, peak_r) = (peak_l.as_f32(), peak_r.as_f32());
        let (rms_l, rms_r) = self.handle.rms_meters();
        let (rms_l, rms_r) = (rms_l.as_f32(), rms_r.as_f32());

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
                    // Instrument strip + octave controls above piano keys
                    ui.horizontal(|ui| {
                        self.draw_instrument_strip(ui);
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

    /// Handle Ctrl+Z (undo) and Ctrl+Shift+Z (redo) keyboard shortcuts.
    fn handle_undo_redo_shortcuts(&mut self, ctx: &egui::Context) {
        let (ctrl_z, ctrl_shift_z) = ctx.input(|i| {
            (
                i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z),
            )
        });
        if ctrl_shift_z {
            self.execute_redo();
        } else if ctrl_z {
            self.execute_undo();
        }
    }

    /// Handle Ctrl+C (copy), Ctrl+V (paste), Ctrl+D (duplicate) keyboard shortcuts.
    fn handle_clipboard_shortcuts(&mut self, ctx: &egui::Context) {
        // Only handle clipboard shortcuts in Rack view
        if self.active_view != AppView::Rack {
            return;
        }

        // Skip if any text edit is focused (avoids intercepting text input)
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }

        let (ctrl_c, ctrl_v, ctrl_d) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            (
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::C),
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::V),
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::D),
            )
        });

        if ctrl_c {
            self.copy_selected_modules();
        } else if ctrl_v {
            self.paste_modules_at_offset();
        } else if ctrl_d {
            self.duplicate_selected_modules();
        }
    }

    /// Copy selected modules and their internal connections to the clipboard.
    fn copy_selected_modules(&mut self) {
        let Some(editor) = self.active_patch_editor_ref() else {
            return;
        };
        let selection = editor.effective_selection();
        if selection.is_empty() {
            return;
        }
        let module_states = editor.extract_module_states(&selection);
        let connection_states = editor.internal_connections(&selection);
        self.clipboard
            .copy_modules(&module_states, &connection_states);
    }

    /// Paste clipboard contents at an offset from the original position.
    fn paste_modules_at_offset(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        let (modules, connections, ref_pos) = self.clipboard.contents();
        let modules = modules.to_vec();
        let connections = connections.to_vec();
        let offset = crate::gui::clipboard::ModuleClipboard::paste_offset();
        let paste_pos = (ref_pos.0 + offset, ref_pos.1 + offset);

        let instrument_id = self.active_instrument_id;

        // Access the patch editor by index to avoid borrowing all of self
        let Some(inst_idx) = self.instruments.iter().position(|i| i.id == instrument_id) else {
            return;
        };

        let new_ids = patch_bridge::paste_clipboard_modules(
            &modules,
            &connections,
            ref_pos,
            paste_pos,
            &mut self.instruments[inst_idx].patch_editor,
            &self.session,
            &mut self.handle,
            instrument_id,
        );
        self.instruments[inst_idx]
            .patch_editor
            .select_modules(&new_ids);
        self.mark_dirty();
    }

    /// Duplicate selected modules: copy + paste at an offset.
    fn duplicate_selected_modules(&mut self) {
        self.copy_selected_modules();
        self.paste_modules_at_offset();
    }

    /// Optimize the project by removing unused patterns, tracks, and instruments.
    fn optimize_project(&mut self) {
        // Remove unused patterns and tracks from the song
        let (removed_patterns, removed_tracks, used_instrument_ids) = {
            let mut song = self.song.write().unwrap_or_else(|e| e.into_inner());
            song.remove_unused()
        };

        // Remove instruments not referenced by remaining tracks/notes
        let to_remove: Vec<_> = self
            .instruments
            .iter()
            .filter(|inst| {
                #[allow(clippy::cast_possible_truncation)]
                let seq_id = inst.id.as_u64() as u16;
                !used_instrument_ids.contains(&seq_id)
            })
            .map(|inst| inst.id)
            .collect();
        let removed_instruments = to_remove.len();
        for inst_id in &to_remove {
            self.handle.send(EngineCommand::RemoveInstrument {
                instrument_id: *inst_id,
            });
        }
        self.instruments
            .retain(|inst| !to_remove.contains(&inst.id));

        // If active instrument was removed, select first remaining
        if !self.instruments.is_empty()
            && !self
                .instruments
                .iter()
                .any(|i| i.id == self.active_instrument_id)
        {
            self.active_instrument_id = self.instruments[0].id;
            self.handle
                .set_focused_instrument(Some(self.active_instrument_id));
        }

        let total = removed_patterns.len() + removed_tracks.len() + removed_instruments;
        if total > 0 {
            eprintln!(
                "Optimized project: removed {} patterns, {} tracks, {removed_instruments} instruments",
                removed_patterns.len(),
                removed_tracks.len(),
            );
        }
    }

    /// Execute an undo operation by popping the undo stack and applying the inverse.
    fn execute_undo(&mut self) {
        if let Some(action) = self.undo_manager.undo() {
            self.apply_undo_action(&action);
        }
    }

    /// Execute a redo operation by popping the redo stack and re-applying.
    fn execute_redo(&mut self) {
        if let Some(action) = self.undo_manager.redo() {
            self.apply_undo_action(&action);
        }
    }

    /// Apply an undo/redo action to the current state.
    #[allow(clippy::too_many_lines)]
    fn apply_undo_action(&mut self, action: &crate::undo::UndoAction) {
        use crate::undo::UndoAction;
        match action {
            UndoAction::AddNote { pattern_id, note } => {
                // Re-add the note to the pattern.
                if let Ok(mut song_w) = self.song.write()
                    && let Some(pattern) = song_w.pattern_mut(*pattern_id)
                {
                    let nid =
                        pattern.add_note(note.start, note.pitch, note.velocity, note.instrument);
                    if let Some(n) = pattern.note_mut(nid) {
                        n.duration = note.duration;
                        n.track = note.track;
                    }
                }
            }
            UndoAction::RemoveNote { pattern_id, note } => {
                // Remove the note from the pattern.
                if let Ok(mut song_w) = self.song.write()
                    && let Some(pattern) = song_w.pattern_mut(*pattern_id)
                {
                    pattern.remove_note(note.id);
                }
            }
            UndoAction::MoveNote {
                pattern_id,
                note_id,
                new_start,
                ..
            } => {
                // Move note to the target position (new_start is the destination).
                if let Ok(mut song_w) = self.song.write()
                    && let Some(pattern) = song_w.pattern_mut(*pattern_id)
                {
                    pattern.move_note(*note_id, *new_start);
                }
            }
            UndoAction::ResizeNote {
                pattern_id,
                note_id,
                new_duration,
                ..
            } => {
                if let Ok(mut song_w) = self.song.write()
                    && let Some(pattern) = song_w.pattern_mut(*pattern_id)
                {
                    if let Some(dur) = new_duration {
                        pattern.resize_note(*note_id, *dur);
                    } else if let Some(n) = pattern.note_mut(*note_id) {
                        n.duration = None;
                    }
                }
            }
            UndoAction::TransposeNote {
                pattern_id,
                note_id,
                new_pitch,
                ..
            } => {
                if let Ok(mut song_w) = self.song.write()
                    && let Some(pattern) = song_w.pattern_mut(*pattern_id)
                    && let Some(n) = pattern.note_mut(*note_id)
                {
                    n.pitch = *new_pitch;
                }
            }
            UndoAction::SetNoteVelocity {
                pattern_id,
                note_id,
                new_velocity,
                ..
            } => {
                if let Ok(mut song_w) = self.song.write()
                    && let Some(pattern) = song_w.pattern_mut(*pattern_id)
                {
                    pattern.set_note_velocity(*note_id, *new_velocity);
                }
            }
            UndoAction::AddModule {
                instrument_id,
                module_state,
            } => {
                // Re-add the module (via session + patch editor).
                // This is complex — for now we record it but full re-add
                // would require reconstructing the module from ModuleState.
                // Minimal: update patch editor position if module exists.
                let _ = (instrument_id, module_state);
            }
            UndoAction::RemoveModule {
                instrument_id,
                module_state,
                connections: _,
            } => {
                let _ = (instrument_id, module_state);
            }
            UndoAction::MoveModule {
                module_id, new_pos, ..
            } => {
                if let Some(editor) = self.active_patch_editor() {
                    editor.set_module_position(*module_id, egui::Pos2::new(new_pos.0, new_pos.1));
                }
            }
            UndoAction::SetParameter {
                module_id,
                param_name,
                new_value,
                ..
            } => {
                // Apply the parameter value to the patch editor UI state.
                // Note: full engine-side undo for parameters is not yet implemented
                // because reconstructing the Param enum variant from a name string
                // requires module-specific knowledge.
                if let crate::patch::ParamValue::Float(val) = new_value
                    && let Some(editor) = self.active_patch_editor()
                {
                    editor.set_parameter_by_name(*module_id, param_name, *val);
                }
            }
            UndoAction::AddConnection {
                instrument_id,
                connection,
            } => {
                if let Some(editor) = self.active_patch_editor() {
                    editor.add_connection(*connection);
                }
                self.handle.send(EngineCommand::Connect {
                    instrument_id: Some(*instrument_id),
                    from: synth_engine::commands::PortId::new(
                        connection.from_module,
                        connection.from_port,
                    ),
                    to: synth_engine::commands::PortId::new(
                        connection.to_module,
                        connection.to_port,
                    ),
                });
            }
            UndoAction::RemoveConnection {
                instrument_id,
                connection,
            } => {
                if let Some(editor) = self.active_patch_editor() {
                    editor.remove_connection(connection);
                }
                self.handle.send(EngineCommand::Disconnect {
                    instrument_id: Some(*instrument_id),
                    from: synth_engine::commands::PortId::new(
                        connection.from_module,
                        connection.from_port,
                    ),
                    to: synth_engine::commands::PortId::new(
                        connection.to_module,
                        connection.to_port,
                    ),
                });
            }
            UndoAction::Composite(actions) => {
                for sub_action in actions {
                    self.apply_undo_action(sub_action);
                }
            }
        }
    }

    fn show_dialogs(&mut self, ctx: &egui::Context) {
        // Unsaved changes confirmation dialog
        self.show_unsaved_changes_dialog(ctx);

        // Update dialog state (clears expired status messages)
        self.dialog_state.update();

        // Settings dialog
        match show_settings_dialog(
            ctx,
            &mut self.dialog_state.show_settings,
            &mut self.dialog_state.current_theme,
            &mut self.settings,
        ) {
            SettingsAction::LiveChange | SettingsAction::SaveAndClose => {
                self.settings.save();
            }
            SettingsAction::CloseWithoutSave => {
                // Discard unsaved edits by reloading from disk
                self.settings = AppSettings::load();
            }
            SettingsAction::None => {}
        }

        // About dialog
        show_about_dialog(ctx, &mut self.dialog_state.show_about);

        // Export WAV dialog
        if self.dialog_state.show_export_wav {
            let mut open = self.dialog_state.show_export_wav;
            match crate::gui::export_dialog::show_export_dialog(
                ctx,
                &mut self.dialog_state.export_state,
                &mut open,
            ) {
                crate::gui::export_dialog::ExportDialogResult::Completed(msg) => {
                    self.dialog_state.set_status(msg);
                    open = false;
                }
                crate::gui::export_dialog::ExportDialogResult::Failed(msg) => {
                    self.dialog_state
                        .set_status(format!("Export failed: {msg}"));
                    open = false;
                }
                crate::gui::export_dialog::ExportDialogResult::Closed => {
                    open = false;
                }
                crate::gui::export_dialog::ExportDialogResult::None => {}
            }
            self.dialog_state.show_export_wav = open;

            // Handle deferred export start (avoids borrow conflict)
            if self.dialog_state.export_state.wants_export {
                let project = self.create_project_from_app();
                self.dialog_state.export_state.begin_export(project);
            }
        }

        // Load built-in patch dialog
        match show_load_patch_dialog(
            ctx,
            &mut self.dialog_state.show_load_patch,
            &mut self.dialog_state.load_patch_search,
        ) {
            LoadPatchResult::LoadBuiltin(patch) => {
                self.load_patch_data(&patch);
                self.current_patch_name = patch.name.clone();
                self.dialog_state
                    .set_status(format!("Loaded: {}", patch.name));
            }
            LoadPatchResult::Cancelled | LoadPatchResult::None => {}
        }

        // Group template browser dialog
        if self.dialog_state.show_group_templates {
            let manager = GroupTemplateManager::default();
            let templates = manager.list_all_templates();
            match show_group_template_browser(
                ctx,
                &mut self.dialog_state.show_group_templates,
                &templates,
                &mut self.dialog_state.group_template_search,
                &mut self.dialog_state.group_template_selected,
            ) {
                GroupTemplateBrowserResult::Selected(source) => {
                    match Self::resolve_group_template(&manager, &source) {
                        Ok(template) => {
                            self.insert_group_template(&template);
                        }
                        Err(e) => {
                            self.dialog_state
                                .set_status(format!("Error loading template: {e}"));
                        }
                    }
                    self.dialog_state.group_template_selected = None;
                }
                GroupTemplateBrowserResult::Browse => {
                    let initial_dir = self.resolve_group_templates_dir();
                    self.dialog_state
                        .open_open_group_template_dialog(initial_dir.as_deref());
                    self.dialog_state.group_template_selected = None;
                }
                GroupTemplateBrowserResult::Cancelled => {
                    self.dialog_state.group_template_drop_pos = None;
                    self.dialog_state.group_template_selected = None;
                }
                GroupTemplateBrowserResult::None => {}
            }
        }

        // Save group template dialog
        if self.dialog_state.show_save_group_template {
            match show_save_group_template_dialog(
                ctx,
                &mut self.dialog_state.show_save_group_template,
                &mut self.dialog_state.group_template_save_name,
                &mut self.dialog_state.group_template_save_description,
                &mut self.dialog_state.group_template_save_category,
            ) {
                SaveGroupTemplateResult::Save => {
                    let group_id = self.dialog_state.group_template_save_group;
                    if let (Some(group_id), Some(editor)) =
                        (group_id, self.active_patch_editor_ref())
                    {
                        let author = self.settings.author.name.trim();
                        let author = if author.is_empty() {
                            None
                        } else {
                            Some(author.to_string())
                        };
                        let description = self
                            .dialog_state
                            .group_template_save_description
                            .trim()
                            .to_string();
                        let description = if description.is_empty() {
                            None
                        } else {
                            Some(description)
                        };
                        let template = editor.build_group_template(
                            group_id,
                            Some(self.dialog_state.group_template_save_category),
                            description,
                            author,
                        );
                        if let Some(template) = template {
                            let manager = GroupTemplateManager::default();
                            match manager.save_template(&template) {
                                Ok(path) => {
                                    self.dialog_state
                                        .set_status(format!("Saved template: {}", path.display()));
                                }
                                Err(e) => {
                                    self.dialog_state
                                        .set_status(format!("Error saving template: {e}"));
                                }
                            }
                        }
                    }
                    self.dialog_state.group_template_save_group = None;
                }
                SaveGroupTemplateResult::Cancelled => {
                    self.dialog_state.group_template_save_group = None;
                }
                SaveGroupTemplateResult::None => {}
            }
        }

        // File dialog (open/save/import)
        if let Some(result) = self.dialog_state.update_file_dialog(ctx) {
            match result {
                FileDialogResult::Picked(path, Some(FileDialogMode::OpenPatch)) => {
                    // Remember the directory for next time
                    if let Some(parent) = path.parent() {
                        self.settings.directories.last_open_dir = Some(parent.to_path_buf());
                        self.settings.save();
                    }
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
                    // Remember the directory for next time
                    if let Some(parent) = path.parent() {
                        self.settings.directories.last_save_dir = Some(parent.to_path_buf());
                        self.settings.save();
                    }
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
                FileDialogResult::Picked(path, Some(FileDialogMode::OpenProject)) => {
                    if let Some(parent) = path.parent() {
                        self.settings.directories.last_project_dir = Some(parent.to_path_buf());
                    }
                    // Smart open: auto-detect and load in a single read
                    match project::load_file(&path) {
                        Ok(LoadedFile::Project(proj)) => {
                            self.load_project_data(proj);
                            self.current_project_path = Some(path.clone());
                            self.dirty = false;
                            self.settings.add_recent_project(path.clone());
                            self.settings.save();
                            self.dialog_state
                                .set_status(format!("Project loaded: {}", path.display()));
                        }
                        Ok(LoadedFile::Patch(patch)) => {
                            self.load_patch_data(&patch);
                            self.current_patch_name = patch.name.clone();
                            self.current_patch_path = Some(path.clone());
                            self.dirty = false;
                            self.settings.add_recent_project(path.clone());
                            self.settings.save();
                            self.dialog_state
                                .set_status(format!("Loaded patch: {}", path.display()));
                        }
                        Err(e) => {
                            self.settings.save();
                            self.dialog_state
                                .set_status(format!("Error reading file: {e}"));
                        }
                    }
                }
                FileDialogResult::Saved(path, Some(FileDialogMode::SaveProject)) => {
                    if let Some(parent) = path.parent() {
                        self.settings.directories.last_project_dir = Some(parent.to_path_buf());
                    }
                    let proj = self.create_project_from_app();
                    match proj.save(&path) {
                        Ok(()) => {
                            self.current_project_path = Some(path.clone());
                            self.dirty = false;
                            self.settings.add_recent_project(path.clone());
                            self.settings.save();
                            self.dialog_state
                                .set_status(format!("Project saved: {}", path.display()));
                        }
                        Err(e) => {
                            self.settings.save();
                            self.dialog_state
                                .set_status(format!("Error saving project: {e}"));
                        }
                    }
                }
                FileDialogResult::Saved(path, Some(FileDialogMode::ExportWav)) => {
                    // User picked an export path — store it and open the export dialog
                    self.dialog_state.export_state.export_path = Some(path);
                    self.dialog_state.show_export_wav = true;
                }
                FileDialogResult::Picked(path, Some(FileDialogMode::OpenGroupTemplate)) => {
                    let manager = GroupTemplateManager::default();
                    match manager.load_template(&path) {
                        Ok(template) => {
                            self.insert_group_template(&template);
                        }
                        Err(e) => {
                            self.dialog_state
                                .set_status(format!("Error loading template: {e}"));
                        }
                    }
                }
                _ => {}
            }
        }

        // Status message toast
        show_status_toast(ctx, &mut self.dialog_state);
    }

    /// Resolve the initial directory for the Open file dialog.
    ///
    /// Priority: last open dir > custom patches dir > default patches dir.
    fn resolve_open_dir(&self) -> Option<PathBuf> {
        self.settings
            .directories
            .last_open_dir
            .clone()
            .or_else(|| self.settings.directories.patches_dir.clone())
            .or_else(|| PatchManager::default_patches_dir().ok())
    }

    /// Resolve the initial directory for the Save file dialog.
    ///
    /// Priority: last save dir > custom patches dir > default patches dir.
    fn resolve_save_dir(&self) -> Option<PathBuf> {
        self.settings
            .directories
            .last_save_dir
            .clone()
            .or_else(|| self.settings.directories.patches_dir.clone())
            .or_else(|| PatchManager::default_patches_dir().ok())
    }

    /// Resolve the initial directory for the Group Template file dialog.
    fn resolve_group_templates_dir(&self) -> Option<PathBuf> {
        GroupTemplateManager::default_templates_dir().ok()
    }

    /// Resolve a group template source to the actual template data.
    fn resolve_group_template(
        manager: &GroupTemplateManager,
        source: &crate::io::GroupTemplateSource,
    ) -> Result<crate::patch::GroupTemplate, crate::patch::PatchError> {
        match source {
            crate::io::GroupTemplateSource::BuiltIn(index) => {
                let all = crate::group_templates::builtin_group_templates();
                all.into_iter().nth(*index).ok_or_else(|| {
                    crate::patch::PatchError::Io(format!(
                        "Built-in template index {index} out of range"
                    ))
                })
            }
            crate::io::GroupTemplateSource::File(path) => manager.load_template(path),
        }
    }

    /// Insert a group template at the last remembered drop position.
    fn insert_group_template(&mut self, template: &crate::patch::GroupTemplate) {
        let drop_pos = self
            .dialog_state
            .group_template_drop_pos
            .take()
            .unwrap_or(Pos2::new(100.0, 100.0));
        let active_id = self.active_instrument_id;
        let session = self.session.clone();
        let (handle, instruments) = (&mut self.handle, &mut self.instruments);
        if let Some(editor) = instruments
            .iter_mut()
            .find(|i| i.id == active_id)
            .map(|i| &mut i.patch_editor)
        {
            match patch_bridge::insert_group_template(
                template, drop_pos, editor, &session, handle, active_id,
            ) {
                Ok(_) => {
                    self.dialog_state
                        .set_status(format!("Inserted template: {}", template.name));
                }
                Err(e) => {
                    self.dialog_state
                        .set_status(format!("Template insert failed: {e}"));
                }
            }
        }
    }

    /// Load a patch into the active instrument's rack view.
    fn load_patch_data(&mut self, patch: &Patch) {
        self.mark_dirty();

        // Clear visualization buffers (not handled by patch_bridge)
        self.handle.visualization_buffers.clear();

        // Delegate to patch_bridge for the main loading logic
        // Load into the active instrument's patch editor
        let active_id = self.active_instrument_id;
        let Some(instrument) = self.instruments.iter_mut().find(|i| i.id == active_id) else {
            eprintln!("Warning: Cannot load patch - no active instrument found");
            return;
        };

        // Update instrument name to match the loaded patch (both UI and engine)
        instrument.name = patch.name.clone();
        self.handle.send(EngineCommand::RenameInstrument {
            instrument_id: active_id,
            name: patch.name.clone(),
        });

        patch_bridge::load_patch(
            patch,
            &mut instrument.patch_editor,
            &self.session,
            &mut self.handle,
            &mut self.keyboard,
            &mut self.glide_time,
            active_id,
        );

        // Restore canvas size hint so the scroll area matches the original layout
        if let Some(cs) = patch.settings.canvas_size {
            instrument
                .patch_editor
                .set_min_canvas_size(eframe::egui::Vec2::new(cs.width, cs.height));
        }

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
                let channel = synth_engine::MidiChannel::from_zero_indexed(
                    snap.midi_channel.as_u8().saturating_sub(1),
                )
                .unwrap_or(synth_engine::MidiChannel::CH1);
                let mut ui_inst = InstrumentUiState::new(snap.id, &snap.name).with_channel(channel);
                ui_inst.volume = snap.volume;
                ui_inst.pan = snap.pan;
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
                    ui_inst.volume = snap.volume;
                }
                ui_inst.pan = snap.pan;
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
        self.mark_dirty();

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
        self.glide_time = synth_core::Seconds::new(0.0);

        // 3. Add default StereoOutput so user gets sound immediately
        self.add_stereo_output_module();

        // 4. Update patch name
        self.current_patch_name = "New Patch".to_string();
    }

    /// Create a patch from current rack state.
    fn create_patch_from_rack(&self) -> Option<Patch> {
        let instrument = self
            .instruments
            .iter()
            .find(|i| i.id == self.active_instrument_id)?;
        let engine_state = Some((self.session.state().as_ref(), self.active_instrument_id));
        patch_bridge::create_patch_from_rack(
            &instrument.name,
            &self.settings.author,
            &instrument.patch_editor,
            &self.keyboard,
            &self.handle,
            self.glide_time,
            self.awe_enabled,
            &self.awe_ui,
            engine_state,
        )
    }

    // ------------------------------------------------------------------
    // Project save/load
    // ------------------------------------------------------------------

    /// Build a `ProjectFile` from the current application state.
    fn create_project_from_app(&self) -> ProjectFile {
        let engine_state = self.session.state();
        let instrument_states: Vec<InstrumentState> = self
            .instruments
            .iter()
            .map(|inst| {
                let patch = patch_bridge::create_patch_from_editor(
                    &inst.name,
                    &inst.patch_editor,
                    Some((engine_state.as_ref(), inst.id)),
                );
                InstrumentState {
                    id: inst.id,
                    name: inst.name.clone(),
                    channel: inst.channel.as_one_indexed(),
                    volume: inst.volume,
                    pan: inst.pan,
                    muted: inst.muted,
                    solo: inst.solo,
                    key_range: (inst.key_range.low.as_u8(), inst.key_range.high.as_u8()),
                    transpose: inst.transpose,
                    oversampling: inst.oversampling.factor() as u8,
                    patch,
                }
            })
            .collect();

        let song = self.song.read().unwrap_or_else(|e| e.into_inner()).clone();

        let global = GlobalProjectState {
            master_volume: synth_core::Gain::new(self.handle.master_volume()),
            octave_offset: self.keyboard.octave_offset(),
            glide_time: self.glide_time,
            awe: if self.awe_enabled {
                Some(self.awe_ui.to_awe_state(true))
            } else {
                None
            },
        };

        ProjectFile::new(
            instrument_states,
            self.active_instrument_id.as_u64(),
            song,
            global,
        )
    }

    /// Load a project file, replacing all current state.
    fn load_project_data(&mut self, project: ProjectFile) {
        // 0. Request egui Area position reset so module positions are restored
        //    from the saved layout instead of using stale cached positions.
        self.needs_area_reset = true;

        // 1. Stop sequencer playback
        self.handle.send(EngineCommand::Stop);

        // 2. Remove visualizers and clear all instruments
        let all_ids: Vec<InstrumentId> = self.instruments.iter().map(|i| i.id).collect();
        for inst_id in &all_ids {
            self.remove_visualizers_for_instrument(*inst_id);
            let _ = self.session.clear_graph(*inst_id);
            let _ = self.session.remove_instrument(*inst_id);
        }

        // 3. Clear GUI state
        self.instruments.clear();
        self.handle.visualization_buffers.clear();

        // 4. Recreate instruments from project file
        let mut max_id: u64 = 0;
        for inst_state in &project.instruments {
            let inst_id = inst_state.id;
            if inst_id.as_u64() > max_id {
                max_id = inst_id.as_u64();
            }

            if inst_id != InstrumentId::FIRST
                && let Err(e) = self
                    .session
                    .add_instrument_with_id(inst_id, &inst_state.name)
            {
                eprintln!(
                    "Warning: failed to create instrument {}: {e}",
                    inst_state.name
                );
                continue;
            }

            // Reset counters before loading patch
            self.session.reset_counters_for_instrument(inst_id);

            // Create UI state
            let channel =
                MidiChannel::from_one_indexed(inst_state.channel).unwrap_or(MidiChannel::CH1);
            let mut ui_inst =
                InstrumentUiState::new(inst_id, &inst_state.name).with_channel(channel);

            // Load the patch into this instrument
            patch_bridge::load_patch(
                &inst_state.patch,
                &mut ui_inst.patch_editor,
                &self.session,
                &mut self.handle,
                &mut self.keyboard,
                &mut self.glide_time,
                inst_id,
            );

            // Restore canvas size so the scroll area matches the original layout
            if let Some(cs) = inst_state.patch.settings.canvas_size {
                ui_inst
                    .patch_editor
                    .set_min_canvas_size(eframe::egui::Vec2::new(cs.width, cs.height));
            }

            // Apply instrument-level settings
            ui_inst.volume = inst_state.volume;
            ui_inst.pan = inst_state.pan;
            ui_inst.muted = inst_state.muted;
            ui_inst.solo = inst_state.solo;
            ui_inst.key_range = synth_engine::instrument::KeyRange::new(
                synth_core::MidiNote::new(inst_state.key_range.0),
                synth_core::MidiNote::new(inst_state.key_range.1),
            );
            ui_inst.transpose = inst_state.transpose;
            ui_inst.oversampling = match inst_state.oversampling {
                2 => synth_dsp::OversamplingFactor::X2,
                4 => synth_dsp::OversamplingFactor::X4,
                _ => synth_dsp::OversamplingFactor::X1,
            };

            // Send engine commands for instrument parameters
            let _ = self.session.rename_instrument(inst_id, &inst_state.name);
            let _ = self
                .session
                .set_instrument_volume(inst_id, inst_state.volume);
            let _ = self.session.set_instrument_pan(inst_id, inst_state.pan);
            let _ = self.session.set_instrument_mute(inst_id, inst_state.muted);
            let _ = self.session.set_instrument_solo(inst_id, inst_state.solo);
            let _ = self.session.set_instrument_midi_channel(inst_id, channel);

            // Send oversampling, key range, transpose via engine commands
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: inst_id,
                param: synth_engine::InstrumentParam::OversamplingFactor(ui_inst.oversampling),
            });
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: inst_id,
                param: synth_engine::InstrumentParam::KeyRange(ui_inst.key_range),
            });
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: inst_id,
                param: synth_engine::InstrumentParam::Transpose(ui_inst.transpose),
            });

            self.instruments.push(ui_inst);
        }

        // 5. Replace song (move instead of clone since we own the project)
        {
            let mut song = self.song.write().unwrap_or_else(|e| e.into_inner());
            *song = project.song;
        }

        // 6. Restore global state
        self.handle
            .send(EngineCommand::SetMasterVolume(project.global.master_volume));
        self.keyboard
            .set_octave_offset(project.global.octave_offset);
        self.glide_time = project.global.glide_time;
        self.handle
            .send(EngineCommand::SetGlideTime(project.global.glide_time));

        if let Some(awe) = &project.global.awe {
            self.awe_enabled = awe.enabled;
            self.awe_ui.restore_from(awe);
            self.handle.send(EngineCommand::SetAweEnabled {
                enabled: awe.enabled,
            });
            self.handle.send(EngineCommand::SetAweParameter {
                param: synth_awe::AweParam::RoomShape(awe.room),
            });
            self.handle.send(EngineCommand::SetAweParameter {
                param: synth_awe::AweParam::Material(awe.material),
            });
            self.handle.send(EngineCommand::SetAweState {
                snapshot: awe.to_snapshot(),
            });
            self.handle.send(EngineCommand::SetAweParameter {
                param: synth_awe::AweParam::SpatialEnabled(awe.spatial_enabled),
            });
            self.handle.send(EngineCommand::SetAweParameter {
                param: synth_awe::AweParam::NoteMapping(awe.note_mapping),
            });
        } else {
            self.awe_enabled = false;
            self.awe_ui = crate::gui::awe_view::AweUiState::default();
        }

        // 7. Update instrument counter
        self.next_instrument_id = max_id + 1;

        // 8. Set active instrument
        let target_id = InstrumentId::new(project.active_instrument_id);
        if self.instruments.iter().any(|i| i.id == target_id) {
            self.active_instrument_id = target_id;
        } else if let Some(first) = self.instruments.first() {
            self.active_instrument_id = first.id;
        }
        self.handle
            .set_focused_instrument(Some(self.active_instrument_id));
    }

    /// Reset to a new empty project, clearing all instruments and song data.
    fn reset_to_new_project(&mut self) {
        let project = ProjectFile::new(
            vec![],
            0,
            synth_sequencer::Song::new("Untitled"),
            GlobalProjectState::default(),
        );
        self.load_project_data(project);
        self.current_project_path = None;
        self.current_patch_name = "Init".to_string();
        self.current_patch_path = None;
        self.dirty = false;
    }

    /// Load a project from a recent-projects path.
    fn load_recent_project(&mut self, path: PathBuf) {
        match project::load_file(&path) {
            Ok(LoadedFile::Project(proj)) => {
                self.load_project_data(proj);
                self.current_project_path = Some(path.clone());
                self.dirty = false;
                self.settings.add_recent_project(path.clone());
                self.settings.save();
                self.dialog_state
                    .set_status(format!("Project loaded: {}", path.display()));
            }
            Ok(LoadedFile::Patch(patch)) => {
                self.load_patch_data(&patch);
                self.current_patch_name = patch.name.clone();
                self.current_patch_path = Some(path.clone());
                self.dirty = false;
                self.settings.add_recent_project(path.clone());
                self.settings.save();
                self.dialog_state
                    .set_status(format!("Loaded patch: {}", path.display()));
            }
            Err(e) => {
                self.settings.remove_recent_project(&path);
                self.settings.save();
                self.dialog_state.set_status(format!("Error loading: {e}"));
            }
        }
    }

    /// Save the current project (returns true on success).
    fn save_current_project(&mut self) -> bool {
        if let Some(path) = self.current_project_path.clone() {
            let proj = self.create_project_from_app();
            match proj.save(&path) {
                Ok(()) => {
                    self.dirty = false;
                    self.settings.add_recent_project(path.clone());
                    self.settings.save();
                    self.dialog_state
                        .set_status(format!("Project saved: {}", path.display()));
                    true
                }
                Err(e) => {
                    self.dialog_state
                        .set_status(format!("Error saving project: {e}"));
                    false
                }
            }
        } else {
            let default_name = "project.json".to_string();
            let initial_dir = self.resolve_project_dir();
            self.dialog_state
                .open_save_project_dialog(&default_name, initial_dir.as_deref());
            false
        }
    }

    /// Show the unsaved-changes confirmation dialog.
    fn show_unsaved_changes_dialog(&mut self, ctx: &egui::Context) {
        if !self.unsaved_dialog.open {
            return;
        }
        let mut close = false;
        egui::Window::new("Unsaved Changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("You have unsaved changes. Save before continuing?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        let saved = self.save_current_project();
                        if saved {
                            self.execute_pending_action(ctx);
                        }
                        close = true;
                    }
                    if ui.button("Don't Save").clicked() {
                        self.dirty = false;
                        self.execute_pending_action(ctx);
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.unsaved_dialog.open = false;
            self.unsaved_dialog.pending_action = None;
        }
    }

    /// Execute the pending action from the unsaved-changes dialog.
    fn execute_pending_action(&mut self, ctx: &egui::Context) {
        let action = self.unsaved_dialog.pending_action.take();
        match action {
            Some(PendingAction::NewProject) => {
                self.reset_to_new_project();
                self.dialog_state
                    .set_status("New project created".to_string());
            }
            Some(PendingAction::OpenProject) => {
                let initial_dir = self.resolve_project_dir();
                self.dialog_state
                    .open_open_project_dialog(initial_dir.as_deref());
            }
            Some(PendingAction::LoadProject(path)) => {
                self.load_recent_project(path);
            }
            Some(PendingAction::Quit) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None => {}
        }
    }

    /// Remove all visualizer modules for a given instrument.
    fn remove_visualizers_for_instrument(&mut self, inst_id: InstrumentId) {
        if let Some(inst) = self.instruments.iter().find(|i| i.id == inst_id) {
            let vis_ids: Vec<_> = inst
                .patch_editor
                .module_ids()
                .into_iter()
                .filter(|mid| {
                    inst.patch_editor
                        .module_descriptor(*mid)
                        .map(|d| d.category)
                        == Some(synth_core::ModuleCategory::Visualizer)
                })
                .collect();
            for module_id in vis_ids {
                self.handle.send_blocking(EngineCommand::RemoveVisualizer {
                    instrument_id: Some(inst_id),
                    id: module_id,
                });
                self.handle.remove_visualization_buffer(module_id);
            }
        }
    }

    /// Resolve the initial directory for the Project file dialog.
    ///
    /// Priority: last project dir > custom projects dir > default projects dir.
    fn resolve_project_dir(&self) -> Option<PathBuf> {
        self.settings
            .directories
            .last_project_dir
            .clone()
            .or_else(|| self.settings.directories.projects_dir.clone())
            .or_else(|| project::default_projects_dir().ok())
    }
}
