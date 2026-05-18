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
use crate::patch::{Author, AwePresetFile, GroupCategory, Patch, categorized_patches};
use crate::project::{self, GlobalProjectState, LoadedFile, ProjectFile};
use synth_core::{Describable, ModuleCategory};
use synth_core::{Seconds, Velocity};
use synth_engine::ModuleType as TypedModuleType;
use synth_engine::commands::PortId;
use synth_engine::instrument::{InstrumentId, MidiChannel};
use synth_engine::visualizers::{LevelMeter, Oscilloscope, SpectrumAnalyzer};
use synth_engine::{EngineCommand, EngineEvent, EngineHandle, ModuleId, SynthEngine};
use synth_sampler::SampleLibrary;

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
            .with_app_id("pertylizer")
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
    let mut style = (*ctx.global_style()).clone();
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

    ctx.set_global_style(style);
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
    active_instrument_id: Option<InstrumentId>,
    next_instrument_id: u64,

    // Navigation state
    active_view: AppView,

    // DEBUG: frame counter after project load (0 = not tracking)

    // AWE state
    awe_enabled: bool,
    awe_ui: crate::gui::awe_view::AweUiState,

    // Sequencer state
    song: std::sync::Arc<parking_lot::RwLock<synth_sequencer::Song>>,
    sequencer_view_state: crate::gui::sequencer::SequencerViewState,

    // MCP shared state
    #[cfg(feature = "mcp")]
    mcp_shared: Option<std::sync::Arc<crate::mcp_shared::McpSharedState>>,

    /// Last `McpSharedState::project_revision` value the GUI consumed.
    /// Lets the per-frame poll skip locking when no I/O has happened.
    #[cfg(feature = "mcp")]
    last_seen_project_revision: u64,

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

    /// Reusable buffers for master scope visualization (avoids per-frame allocation).
    scope_buf_l: Vec<f32>,
    scope_buf_r: Vec<f32>,

    /// Cached window title to avoid allocating every frame.
    last_title: String,

    /// Pending AWE preset to save (waiting for file dialog).
    pending_awe_preset_save: Option<crate::patch::AwePresetFile>,

    /// Shared sample library.
    sample_library: std::sync::Arc<std::sync::RwLock<SampleLibrary>>,

    /// Sample view state.
    sample_view_state: crate::gui::sample_view::SampleViewState,

    /// Audio input manager for recording.
    audio_input: crate::audio::input::AudioInputManager,

    /// Floating analyze window. Single instance, re-targeted to the active
    /// instrument each frame.
    analyze_window: crate::gui::analyze::AnalyzeWindow,

    /// Which instrument has its edit window open. `None` = closed.
    instrument_edit_target: Option<InstrumentId>,

    /// Instrument pending deletion (drives the confirmation modal).
    pending_instrument_delete: Option<InstrumentId>,

    /// Project info edit window — open state.
    project_edit_open: bool,
    /// Per-project author metadata. Initialized from `settings.author`
    /// for new projects, overridden from disk on load, persisted on save.
    current_project_author: Author,
}

impl SynthApp {
    #[allow(unused_variables)]
    fn new(
        handle: EngineHandle,
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
        let keyboard = PianoKeyboard::new();
        let glide_time = Seconds::new(0.0);

        // Start with no instruments — user creates them explicitly via "+ New Instrument"
        let active_instrument_id: Option<InstrumentId> = None;
        let next_instrument_id = 0;
        let instruments: Vec<InstrumentUiState> = vec![];
        let patch_name = String::new();

        // Initialize MIDI input (connects to first available port)
        // The MidiHandler gets a clone of the command sender, so both GUI and MIDI
        // can send commands to the engine.
        let midi_handler = MidiHandler::new(handle.command_sender());

        let mut dialog_state = DialogState::new();
        dialog_state.current_theme = settings.theme;

        // Clone the default author up front because `settings` is moved
        // into the struct literal below before `current_project_author`.
        let project_author = settings.author.clone();

        // Surface settings load warnings as a GUI toast so the user knows
        // defaults are being used (the file may be corrupt or missing).
        if let Some(warning) = &settings.load_warning {
            dialog_state.set_status(warning.clone());
        }

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
            #[cfg(feature = "mcp")]
            last_seen_project_revision: 0,
            #[cfg(feature = "osc")]
            osc_shared: config.osc_shared,
            settings,
            undo_manager: crate::undo::UndoManager::new(),
            dirty: false,
            unsaved_dialog: UnsavedChangesDialog::default(),
            clipboard: crate::gui::clipboard::ModuleClipboard::new(),
            scope_buf_l: Vec::new(),
            scope_buf_r: Vec::new(),
            last_title: String::new(),
            pending_awe_preset_save: None,
            sample_library: config.sample_library,
            sample_view_state: crate::gui::sample_view::SampleViewState::new(),
            audio_input: crate::audio::input::AudioInputManager::new(),
            analyze_window: crate::gui::analyze::AnalyzeWindow::new(),
            instrument_edit_target: None,
            pending_instrument_delete: None,
            project_edit_open: false,
            current_project_author: project_author,
        }
    }

    /// Mark the project as having unsaved changes.
    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// Dispatch a welcome-screen action to the appropriate existing flow.
    fn handle_welcome_action(&mut self, action: crate::gui::welcome_view::WelcomeAction) {
        use crate::gui::welcome_view::WelcomeAction;
        match action {
            WelcomeAction::NewInstrument => {
                self.add_new_instrument();
            }
            WelcomeAction::OpenProject => {
                if self.dirty {
                    self.unsaved_dialog.pending_action = Some(PendingAction::OpenProject);
                    self.unsaved_dialog.open = true;
                } else {
                    let initial_dir = self.resolve_project_dir();
                    self.dialog_state
                        .open_open_project_dialog(initial_dir.as_deref());
                }
            }
            WelcomeAction::OpenPatch => {
                let initial_dir = self.resolve_open_dir();
                self.dialog_state
                    .open_open_patch_dialog(initial_dir.as_deref());
            }
            WelcomeAction::LoadBuiltinPatch => {
                self.dialog_state.show_load_patch = true;
            }
            WelcomeAction::ImportSample => {
                self.dialog_state.open_import_sample_dialog(None);
            }
            WelcomeAction::OpenRecent(path) => {
                if self.dirty {
                    self.unsaved_dialog.pending_action = Some(PendingAction::LoadProject(path));
                    self.unsaved_dialog.open = true;
                } else {
                    self.load_recent_project(path);
                }
            }
        }
    }

    /// Create a new instrument, register it in the engine and UI, and make it active.
    /// Returns the new `InstrumentId` on success, or `None` if the session refused.
    /// Remove an instrument by id. Drops the UI mirror, tells the engine
    /// to release voices, and switches `active_instrument_id` to a
    /// neighbour when the active one was deleted. No-op for unknown ids.
    fn delete_instrument(&mut self, id: InstrumentId) {
        if !self.instruments.iter().any(|i| i.id == id) {
            return;
        }
        if let Err(e) = self.session.remove_instrument(id) {
            eprintln!("Failed to remove instrument {id:?}: {e}");
        }
        self.instruments.retain(|i| i.id != id);
        if self.active_instrument_id == Some(id) {
            self.active_instrument_id = self.instruments.first().map(|i| i.id);
            self.handle
                .set_focused_instrument(self.active_instrument_id);
        }
        if self.instrument_edit_target == Some(id) {
            self.instrument_edit_target = None;
        }
        self.mark_dirty();
    }

    fn add_new_instrument(&mut self) -> Option<InstrumentId> {
        let instrument_num = self.instruments.len() + 1;
        let new_name = format!("Instrument {instrument_num}");
        let new_channel =
            MidiChannel::from_one_indexed(instrument_num as u8).unwrap_or(MidiChannel::CH1);

        let new_id = self.session.add_instrument(&new_name).ok()?;

        let id_val = new_id.as_u64() + 1;
        if id_val > self.next_instrument_id {
            self.next_instrument_id = id_val;
        }

        self.handle.send(EngineCommand::SetInstrumentMidiChannel {
            instrument_id: new_id,
            channel: new_channel,
        });

        let new_ui_instrument = InstrumentUiState::new(new_id, &new_name).with_channel(new_channel);
        self.instruments.push(new_ui_instrument);
        self.active_instrument_id = Some(new_id);
        self.handle.set_focused_instrument(Some(new_id));
        self.mark_dirty();

        Some(new_id)
    }

    /// Add a module via session and register it in the active patch editor.
    /// Returns the assigned `ModuleId` and descriptor, or `None` on failure.
    fn session_add_module(
        &mut self,
        module_type: TypedModuleType,
    ) -> Option<(ModuleId, synth_core::ModuleDescriptor)> {
        let result = self
            .session
            .add_module(self.active_instrument_id?, module_type)
            .ok()?;
        Some(result)
    }

    /// Get the active instrument's patch editor.
    ///
    /// Returns `None` if active_instrument_id doesn't match any instrument.
    /// This can happen briefly during instrument deletion/creation.
    fn active_patch_editor(&mut self) -> Option<&mut PatchEditor> {
        let id = self.active_instrument_id?;
        self.instruments
            .iter_mut()
            .find(|i| i.id == id)
            .map(|i| &mut i.patch_editor)
    }

    /// Get the active instrument's patch editor (immutable).
    ///
    /// Returns `None` if active_instrument_id doesn't match any instrument.
    /// This can happen briefly during instrument deletion/creation.
    fn active_patch_editor_ref(&self) -> Option<&PatchEditor> {
        let id = self.active_instrument_id?;
        self.instruments
            .iter()
            .find(|i| i.id == id)
            .map(|i| &i.patch_editor)
    }
}

impl eframe::App for SynthApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        use egui_remixicon::icons as ri;
        let ctx = ui.ctx().clone();
        let ctx = &ctx;

        // Drain audio input ring buffer every frame (needed for peak metering and recording).
        // Must run regardless of active view so recording works from the patch editor too.
        self.audio_input.drain_gui_buffer();

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
                    // Write recorded notes into the pattern (on UI thread, safe to lock).
                    // Known limitation: if the song lock is poisoned, recorded notes are
                    // silently dropped. Buffering for retry adds complexity for a scenario
                    // that is extremely unlikely since only the UI thread writes.
                    {
                        let mut song = self.song.write();
                        if let Some(pattern) = song.pattern_mut(pattern_id) {
                            if !overdub {
                                pattern.clear_notes();
                            }
                            let instrument = self.sequencer_view_state.recording_instrument;
                            for note in &notes {
                                let nid = pattern.add_note(
                                    note.start,
                                    note.pitch,
                                    note.velocity,
                                    instrument,
                                );
                                if let Some(n) = pattern.note_mut(nid) {
                                    n.duration = Some(note.duration);
                                }
                            }
                        } else {
                            eprintln!(
                                "RecordedNotesFlushed: pattern {pattern_id:?} not found, {} notes dropped",
                                notes.len()
                            );
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

        // Poll MCP pending auto-layout.
        //
        // Non-destructive load: a request stays set until the Rack view
        // actually applies it (see apply site below). This means a
        // request that arrives while the user is in AcousticWorld /
        // Sequencer / Sample survives until they switch back to Rack.
        #[cfg(feature = "mcp")]
        let mcp_auto_layout = self.mcp_shared.as_ref().is_some_and(|shared| {
            shared
                .pending_auto_layout
                .load(std::sync::atomic::Ordering::Relaxed)
        });
        #[cfg(not(feature = "mcp"))]
        let mcp_auto_layout = false;

        // Single revision-gated drain for everything MCP project I/O
        // pushes back to the GUI: the refresh queue, source path, and
        // status line. `project_revision` is the lock-free fast path —
        // when nothing has happened we don't touch any mutex.
        #[cfg(feature = "mcp")]
        if let Some(shared) = self.mcp_shared.as_ref().map(std::sync::Arc::clone) {
            let current_rev = shared
                .project_revision
                .load(std::sync::atomic::Ordering::Acquire);
            if current_rev != self.last_seen_project_revision {
                use crate::mcp_shared::ProjectRefresh;

                // Load / new stash a refresh; save leaves it empty.
                let refresh = shared
                    .pending_project_refresh
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                if let Some(refresh) = refresh {
                    match refresh {
                        ProjectRefresh::Loaded(project) => {
                            self.current_project_path = shared
                                .last_loaded_project_path
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .clone();
                            self.refresh_ui_from_project(&project);
                            self.dirty = false;
                        }
                        ProjectRefresh::Reset => {
                            self.refresh_ui_after_reset();
                            self.current_project_path = None;
                            self.current_patch_name = "Init".to_string();
                            self.current_patch_path = None;
                            self.dirty = false;
                        }
                    }
                }

                // Surface the most recent I/O outcome (success or error)
                // in the status line.
                let status = shared
                    .last_project_io_status
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if let Some(status) = status {
                    let msg = match status {
                        Ok(m) => m,
                        Err(e) => format!("Error: {e}"),
                    };
                    self.dialog_state.set_status(msg);
                }

                self.last_seen_project_revision = current_rev;
            }
        }

        // Poll MCP pending AWE state
        #[cfg(feature = "mcp")]
        {
            let pending_awe = self.mcp_shared.as_ref().and_then(|shared| {
                shared
                    .pending_awe_state
                    .lock()
                    .ok()
                    .and_then(|mut pending| pending.take())
            });
            if let Some(awe_state) = pending_awe {
                self.awe_enabled = awe_state.enabled;
                self.awe_ui.restore_from(&awe_state);
                // Send engine commands so audio matches
                self.handle
                    .send(synth_engine::EngineCommand::SetAweEnabled {
                        enabled: awe_state.enabled,
                    });
                self.handle
                    .send(synth_engine::EngineCommand::SetAweParameter {
                        param: synth_awe::AweParam::RoomShape(awe_state.room),
                    });
                self.handle
                    .send(synth_engine::EngineCommand::SetAweParameter {
                        param: synth_awe::AweParam::Material(awe_state.material),
                    });
                self.handle.send(synth_engine::EngineCommand::SetAweState {
                    snapshot: awe_state.snapshot,
                });
                self.handle
                    .send(synth_engine::EngineCommand::SetAweParameter {
                        param: synth_awe::AweParam::SpatialEnabled(awe_state.spatial_enabled),
                    });
                self.handle
                    .send(synth_engine::EngineCommand::SetAweParameter {
                        param: synth_awe::AweParam::NoteMapping(awe_state.note_mapping),
                    });
            }
        }

        // Sync AWE state to MCP shared state
        #[cfg(feature = "mcp")]
        if let Some(shared) = &self.mcp_shared {
            if let Ok(mut awe_state) = shared.awe_state.lock() {
                *awe_state = self.awe_ui.to_awe_state(self.awe_enabled);
            }
            // The edit-in-progress flag (set last frame in draw_controls) decides
            // direction: while editing, GUI is source of truth; otherwise let MCP
            // writes flow back.
            if let Ok(mut desc) = shared.awe_description.lock()
                && *desc != self.awe_ui.description
            {
                if self.awe_ui.description_edit_in_progress {
                    desc.clone_from(&self.awe_ui.description);
                } else {
                    self.awe_ui.description.clone_from(&desc);
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

        // ── Analyze window shortcut (Ctrl/Cmd + Shift + A) ──
        self.handle_analyze_shortcut(ctx);

        // ── Analyze window — runs every frame so the worker-thread poll can
        //    drain even when the window itself is closed. ──
        self.render_analyze_window(ctx);

        // ── Instrument edit window (per-instrument basic settings). ──
        self.render_instrument_edit_window(ctx);
        self.render_instrument_delete_confirm(ctx);

        // ── Project info edit window (song metadata + project author). ──
        self.render_project_edit_window(ctx);

        // Request continuous repaint for meters
        ctx.request_repaint();

        // Top menu bar
        egui::Panel::top("menu_bar").show_inside(ui, |ui| {
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
                        self.save_current_project();
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
                        let song_secs = self.song.read().length_seconds();
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
                        .add_enabled(
                            self.undo_manager.can_undo(),
                            egui::Button::new(&undo_label).shortcut_text("Ctrl+Z"),
                        )
                        .clicked()
                    {
                        self.execute_undo();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.undo_manager.can_redo(),
                            egui::Button::new(&redo_label).shortcut_text("Ctrl+Shift+Z"),
                        )
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
                            egui::Button::new(format!("{} Copy", ri::FILE_COPY_LINE))
                                .shortcut_text("Ctrl+C"),
                        )
                        .clicked()
                    {
                        self.copy_selected_modules();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            !self.clipboard.is_empty(),
                            egui::Button::new(format!("{} Paste", ri::CLIPBOARD_LINE))
                                .shortcut_text("Ctrl+V"),
                        )
                        .clicked()
                    {
                        self.paste_modules_at_offset();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            has_selection,
                            egui::Button::new(format!("{} Duplicate", ri::FILE_COPY_2_LINE))
                                .shortcut_text("Ctrl+D"),
                        )
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
                    ui.separator();
                    if ui
                        .add(
                            egui::Button::new(format!("{} Analyze Patch…", ri::FILE_SEARCH_LINE))
                                .shortcut_text("Ctrl+Shift+A"),
                        )
                        .on_hover_text("Open the offline analyze view for the active instrument")
                        .clicked()
                    {
                        self.analyze_window.open();
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
                    let views: [(AppView, &str); 4] = [
                        (AppView::Rack, &format!("{} Rack", ri::LAYOUT_GRID_FILL)),
                        (
                            AppView::AcousticWorld,
                            &format!("{} AWE", ri::SURROUND_SOUND_FILL),
                        ),
                        (AppView::Sequencer, &format!("{} Seq", ri::PLAY_LIST_FILL)),
                        (AppView::Sample, &format!("{} Sample", ri::MUSIC_FILL)),
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
                            RichText::new(format!("{} PANIC", ri::ALARM_WARNING_FILL))
                                .color(theme().colors.accent_red),
                        ))
                        .clicked()
                    {
                        self.handle.send(EngineCommand::AllNotesOff);
                        self.pressed_keys.clear();
                        self.keyboard.clear_pressed();
                    }
                    // Sample memory indicator
                    if let Ok(lib) = self.sample_library.read() {
                        let count = lib.len();
                        if count > 0 {
                            let total_bytes: usize = lib
                                .list()
                                .iter()
                                .map(|m| {
                                    m.frame_count.as_usize()
                                        * m.channels.count() as usize
                                        * std::mem::size_of::<f32>()
                                })
                                .sum();
                            let mb = total_bytes as f64 / (1024.0 * 1024.0);
                            ui.separator();
                            ui.label(
                                RichText::new(format!("Samples: {count} ({mb:.1}MB)"))
                                    .color(theme().colors.text_dim),
                            );
                        }
                    }
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
                    // Instrument edit pencil — sits visually to the right of
                    // the dropdown (right-to-left layout). Opens the active
                    // instrument's edit window from any view; disabled when
                    // no instrument is selected.
                    let edit_btn = ui
                        .add_enabled(
                            self.active_instrument_id.is_some(),
                            egui::Button::new(
                                RichText::new(ri::EDIT_LINE).color(theme().colors.text_dim),
                            )
                            .frame(false)
                            .small(),
                        )
                        .on_hover_text("Edit active instrument…");
                    if edit_btn.clicked() {
                        self.instrument_edit_target = self.active_instrument_id;
                    }

                    // Instrument selector dropdown (lives in the toolbar so
                    // the active instrument can be switched independently of
                    // the keyboard strip).
                    let active_name = self
                        .active_instrument_id
                        .and_then(|id| self.instruments.iter().find(|i| i.id == id))
                        .map(|i| i.name.as_str())
                        .unwrap_or("(none)");
                    let menu_label = RichText::new(format!(
                        "{} {active_name} {}",
                        ri::MUSIC_2_FILL,
                        ri::ARROW_DOWN_S_FILL
                    ))
                    .color(theme().colors.accent_cyan);
                    ui.menu_button(menu_label, |ui| {
                        if ui
                            .button(
                                RichText::new(format!("{} New Instrument", ri::ADD_LINE))
                                    .color(theme().colors.accent_green),
                            )
                            .clicked()
                        {
                            self.add_new_instrument();
                            ui.close();
                        }
                        ui.separator();
                        if self.instruments.is_empty() {
                            ui.label(
                                RichText::new("No instruments").color(theme().colors.text_dim),
                            );
                        } else {
                            // Capture id/name pairs first so we can mutate
                            // state inside the menu without borrowing
                            // `self.instruments` immutably.
                            let rows: Vec<(InstrumentId, String, bool)> = self
                                .instruments
                                .iter()
                                .map(|inst| {
                                    (
                                        inst.id,
                                        inst.name.clone(),
                                        Some(inst.id) == self.active_instrument_id,
                                    )
                                })
                                .collect();
                            for (id, name, is_active) in rows {
                                ui.horizontal(|ui| {
                                    let label = if is_active {
                                        RichText::new(format!(
                                            "{} {}",
                                            ri::CHECKBOX_BLANK_CIRCLE_FILL,
                                            name
                                        ))
                                        .color(theme().colors.accent_cyan)
                                    } else {
                                        RichText::new(format!("  {}", name))
                                    };
                                    if ui.button(label).clicked() {
                                        self.active_instrument_id = Some(id);
                                        self.handle.set_focused_instrument(Some(id));
                                        ui.close();
                                    }
                                    // Per-row actions menu.
                                    ui.menu_button(
                                        RichText::new(ri::MORE_FILL).color(theme().colors.text_dim),
                                        |ui| {
                                            if ui
                                                .button(format!("{} Rename / edit…", ri::EDIT_LINE))
                                                .clicked()
                                            {
                                                self.instrument_edit_target = Some(id);
                                                ui.close();
                                            }
                                            ui.separator();
                                            if ui
                                                .button(
                                                    RichText::new(format!(
                                                        "{} Delete…",
                                                        ri::DELETE_BIN_LINE
                                                    ))
                                                    .color(theme().colors.accent_red),
                                                )
                                                .clicked()
                                            {
                                                self.pending_instrument_delete = Some(id);
                                                ui.close();
                                            }
                                        },
                                    );
                                });
                            }
                        }
                    });
                    ui.separator();

                    // Pencil edit icon (sits visually to the right of the title
                    // because right-to-left layout places later widgets further
                    // left).
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(ri::EDIT_LINE).color(theme().colors.text_dim),
                            )
                            .frame(false)
                            .small(),
                        )
                        .on_hover_text("Edit project info")
                        .clicked()
                    {
                        self.project_edit_open = true;
                    }

                    // Project / song title — shows `Song.name`. Double-click
                    // opens the project edit window.
                    let title_text = {
                        let song = self.song.read();
                        if song.name.is_empty() {
                            "Untitled".to_string()
                        } else {
                            song.name.clone()
                        }
                    };
                    let dirty_marker = if self.dirty { " *" } else { "" };
                    let title_resp = ui
                        .add(
                            egui::Label::new(
                                RichText::new(format!("{title_text}{dirty_marker}"))
                                    .color(theme().colors.text_secondary),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Double-click to edit project info");
                    if title_resp.double_clicked() {
                        self.project_edit_open = true;
                    }
                    ui.separator();
                });
            });
        });

        // Status bar at the very bottom: Glide + Octave on the left,
        // Latency / Voices / CPU on the right.
        // Declared before keyboard_panel so it ends up below the keyboard.
        egui::Panel::bottom("status_bar")
            .min_size(22.0)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let t = theme();

                    // ── Left side: Octave +/- and Glide slider ──
                    ui.label(
                        RichText::new(format!("Octave: {:+}", self.keyboard.octave_offset()))
                            .color(t.colors.text_secondary)
                            .small(),
                    );
                    if ui.small_button("-").clicked() {
                        let new_offset = self.keyboard.octave_offset() - 1;
                        self.keyboard.set_octave_offset(new_offset);
                    }
                    if ui.small_button("+").clicked() {
                        let new_offset = self.keyboard.octave_offset() + 1;
                        self.keyboard.set_octave_offset(new_offset);
                    }
                    ui.separator();

                    ui.label(RichText::new("Glide:").color(t.colors.text_dim).small());
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

                    // ── Right side: CPU / Voices / Latency ──
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let cpu = self.handle.cpu_usage();
                        let cpu_color = if cpu > 0.8 {
                            t.colors.meter_red
                        } else if cpu > 0.5 {
                            t.colors.meter_yellow
                        } else {
                            t.colors.meter_green
                        };
                        ui.label(
                            RichText::new(format!("CPU: {:>3.0}%", cpu * 100.0)).color(cpu_color),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(format!("Voices: {:>3}", self.handle.voice_count()))
                                .color(t.colors.text_secondary)
                                .family(egui::FontFamily::Monospace),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "Latency: {:.1}ms",
                                self.latency.as_secs_f64() * 1000.0
                            ))
                            .color(t.colors.text_dim),
                        );
                    });
                });
            });

        // Bottom panel with keyboard (always visible)
        // Render keyboard content in Order::Middle so it has input priority over
        // module Areas (Order::Background) that may extend into the panel area.
        egui::Panel::bottom("keyboard_panel")
            .min_size(100.0)
            .show_inside(ui, |ui| {
                let layer_id =
                    egui::LayerId::new(egui::Order::Middle, egui::Id::new("keyboard_layer"));
                ui.scope_builder(egui::UiBuilder::new().layer_id(layer_id), |ui| {
                    self.draw_keyboard(ui);
                });
            });

        // Main content - CentralPanel rendered LAST (normal egui order)
        // Module Areas are clipped to visible_rect in patch_editor.rs
        // Wrap in a labeled block so the welcome path can skip the rest of
        // the Rack arm without `return;` — that would also skip show_dialogs
        // below and prevent welcome-screen file dialogs from ever appearing.
        'view: {
            match self.active_view {
                AppView::Rack => {
                    // Rack view: show the active instrument's patch editor.
                    // When no instrument exists yet, render the welcome landing
                    // screen so the user can create one or open a project/patch
                    // without first hunting for the "+ New Instrument" button.
                    let Some(active_id) = self.active_instrument_id else {
                        let recent = self.settings.recent_projects.clone();
                        let mut welcome_action = None;
                        egui::CentralPanel::default().show_inside(ui, |ui| {
                            let t = theme();
                            welcome_action = crate::gui::welcome_view::show(ui, &t, &recent);
                        });
                        if let Some(action) = welcome_action {
                            self.handle_welcome_action(action);
                        }
                        break 'view;
                    };

                    // Left side panel: instrument list (mirrors sample_view layout).
                    // Acts as a primary picker; the dropdown above the keyboard
                    // stays as a backup.
                    egui::Panel::left("instruments_panel")
                        .default_size(180.0)
                        .min_size(140.0)
                        .show_inside(ui, |ui| {
                            use egui_remixicon::icons as ri;
                            let t = theme();

                            ui.heading(
                                RichText::new(format!("{} Instruments", ri::MUSIC_2_FILL))
                                    .color(t.colors.text_primary),
                            );
                            ui.separator();

                            let mut clicked: Option<InstrumentId> = None;
                            let mut edit_requested: Option<InstrumentId> = None;
                            egui::ScrollArea::vertical()
                                .content_margin(egui::Margin::same(6))
                                .show(ui, |ui| {
                                    for inst in &self.instruments {
                                        let is_active = inst.id == active_id;
                                        let text_color = if is_active {
                                            t.colors.text_primary
                                        } else {
                                            t.colors.text_secondary
                                        };
                                        ui.horizontal(|ui| {
                                            let resp = ui.selectable_label(
                                                is_active,
                                                RichText::new(&inst.name).color(text_color),
                                            );
                                            if resp.clicked() && !is_active {
                                                clicked = Some(inst.id);
                                            }
                                            if resp.double_clicked() {
                                                edit_requested = Some(inst.id);
                                            }
                                            // Pencil icon on the right opens the
                                            // edit window without changing the
                                            // active instrument.
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    let btn = ui.add(
                                                        egui::Button::new(
                                                            RichText::new(ri::EDIT_LINE)
                                                                .color(t.colors.text_dim),
                                                        )
                                                        .frame(false)
                                                        .small(),
                                                    );
                                                    if btn
                                                        .on_hover_text("Edit instrument…")
                                                        .clicked()
                                                    {
                                                        edit_requested = Some(inst.id);
                                                    }
                                                },
                                            );
                                        });
                                    }
                                });

                            if let Some(id) = clicked {
                                self.active_instrument_id = Some(id);
                                self.handle.set_focused_instrument(Some(id));
                            }
                            if let Some(id) = edit_requested {
                                self.instrument_edit_target = Some(id);
                            }

                            ui.add_space(8.0);
                            if ui
                                .button(
                                    RichText::new(format!("{} New Instrument", ri::ADD_LINE))
                                        .color(t.colors.accent_green),
                                )
                                .clicked()
                            {
                                self.add_new_instrument();
                            }
                        });

                    egui::CentralPanel::default().show_inside(ui, |ui| {
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

                        // Update sample list for sampler module dropdowns
                        if let Ok(lib) = self.sample_library.read() {
                            let list: Vec<(u64, String)> = lib
                                .list()
                                .iter()
                                .map(|m| (m.id.0, m.name.clone()))
                                .collect();
                            patch_editor.set_sample_list(list);
                        }

                        // Get effect chain order from shared state
                        let effect_chain_order: Vec<synth_engine::ModuleId> = self
                            .session
                            .list_instruments()
                            .iter()
                            .find(|s| s.id == active_id)
                            .map(|s| s.effect_chain_order.clone())
                            .unwrap_or_default();

                        let audio_input_snapshot = crate::gui::patch_editor::AudioInputSnapshot {
                            state: self.audio_input.state(),
                            peak_level: self.audio_input.peak_level(),
                            recorded_seconds: self.audio_input.recorded_seconds(),
                        };
                        let result = patch_editor.show(
                            ui,
                            &self.handle,
                            active_id.as_u64(),
                            &effect_chain_order,
                            &audio_input_snapshot,
                        );
                        let had_mutations = result.has_mutations();

                        // Handle parameter changes - send Param directly (carries its own value)
                        for (module_id, param) in result.param_changes {
                            // Check module category
                            let category = patch_editor
                                .module_descriptor(module_id)
                                .map(|d| d.category);

                            match category {
                                Some(ModuleCategory::Effect) => {
                                    // Effect module - use SetEffectParameter (targets active instrument).
                                    // Routing is by ModuleId so duplicate effects of the same type
                                    // can be tuned independently.
                                    self.handle.send(EngineCommand::SetEffectParameter {
                                        instrument_id: Some(active_id),
                                        module_id,
                                        param,
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
                                    | ModuleCategory::Sampler,
                                ) => {
                                    // Voice/modular module - send to active instrument's voice graph
                                    // SetModuleParameter updates both the template AND all active voices
                                    self.handle.send(EngineCommand::SetModuleParameter {
                                        instrument_id: Some(active_id),
                                        module_id,
                                        param,
                                    });

                                    // If a SampleSelect param was changed, also load the sample data
                                    if let synth_core::Param::Sampler(
                                        synth_core::SamplerParam::SampleSelect(sample_id),
                                    ) = param
                                        && let Ok(lib) = self.sample_library.read()
                                        && let Some(sample) =
                                            lib.get(synth_sampler::SampleId::new(sample_id.0))
                                    {
                                        self.handle.send(EngineCommand::LoadSampleData {
                                            instrument_id: active_id,
                                            module_id,
                                            data: std::sync::Arc::clone(&sample.data),
                                            channels: sample.meta.channels,
                                            frame_count: sample.meta.frame_count.as_usize(),
                                            root_note: sample
                                                .meta
                                                .root_note
                                                .unwrap_or(synth_core::MidiNote(60)),
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }

                        // Handle module removal
                        for module_id in result.modules_to_remove {
                            // Check if this module has a visualization buffer to clean up
                            let has_vis_buffer =
                                patch_editor.module_descriptor(module_id).is_some_and(|d| {
                                    d.category == ModuleCategory::Visualizer
                                        || d.type_id.0 == "signal_monitor"
                                        || d.type_id.0 == "inline_signal_monitor"
                                });

                            // Remove from session (registry + engine command)
                            if let Err(e) = self.session.remove_module(active_id, module_id) {
                                eprintln!("Failed to remove module {module_id:?}: {e}");
                                continue;
                            }

                            patch_editor.remove_module(module_id);

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
                                instrument_id: Some(active_id),
                                module: module_id,
                                bypass: new_bypass_state,
                            });
                        }

                        // Handle effect chain reorder requests
                        for (module_id, direction) in result.reorder_effects {
                            self.handle.send(EngineCommand::ReorderEffect {
                                instrument_id: Some(active_id),
                                module_id,
                                direction,
                            });
                        }

                        // Handle audio input actions from patch module
                        if let Some(action) = result.audio_input_action {
                            use crate::gui::patch_editor::AudioInputAction;
                            match action {
                                AudioInputAction::StartMonitoring => {
                                    if let Some(host) = &self.host {
                                        let device =
                                            self.sample_view_state.selected_input_device.as_deref();
                                        let config = synth_core::StreamConfig {
                                            sample_rate: synth_core::audio::SampleRate::DVD_QUALITY,
                                            buffer_size: synth_core::BufferSize::MEDIUM,
                                            channels: synth_core::ChannelCount::Stereo,
                                        };
                                        match self.audio_input.start_monitoring(
                                            host.as_ref(),
                                            device,
                                            &config,
                                        ) {
                                            Ok(()) => {
                                                if let Some(consumer) =
                                                    self.audio_input.take_engine_consumer()
                                                {
                                                    self.handle.send(
                                                        EngineCommand::SetAudioInputConsumer {
                                                            consumer,
                                                        },
                                                    );
                                                }
                                            }
                                            Err(e) => {
                                                self.dialog_state
                                                    .set_status(format!("Input error: {e}"));
                                            }
                                        }
                                    }
                                }
                                AudioInputAction::StopMonitoring => {
                                    self.handle.send(EngineCommand::ClearAudioInputConsumer);
                                    self.audio_input.stop_monitoring();
                                }
                                AudioInputAction::StartRecording => {
                                    self.audio_input.start_recording();
                                }
                                AudioInputAction::StopRecording => {
                                    if let Some(data) = self.audio_input.stop_recording() {
                                        let channels = self.audio_input.channels();
                                        let sample_rate = self.audio_input.sample_rate();
                                        let frame_count = if channels > 0 {
                                            data.len() / channels as usize
                                        } else {
                                            0
                                        };
                                        let sample = synth_sampler::Sample::new(
                                            synth_sampler::SampleMeta {
                                                id: synth_sampler::SampleId::new(0),
                                                name: format!(
                                                    "Recording {:.1}s",
                                                    frame_count as f64 / f64::from(sample_rate.0)
                                                ),
                                                sample_rate,
                                                channels: synth_core::ChannelCount::from(channels),
                                                frame_count: synth_core::SampleCount::new(
                                                    frame_count,
                                                ),
                                                root_note: None,
                                                loop_region: None,
                                                crop: None,
                                                source: synth_sampler::SampleSource::Recorded,
                                            },
                                            data.into(),
                                        );
                                        if let Ok(mut lib) = self.sample_library.write() {
                                            let id = lib.add(sample);
                                            self.sample_view_state.selected_sample = Some(id);
                                            self.sample_view_state.invalidate_peaks();
                                        }
                                        self.dialog_state.set_status("Recording saved");
                                    }
                                }
                            }
                        }

                        // Handle signal monitor insertions — create inline monitor and rewire
                        for connection in result.insert_signal_monitor_at {
                            // Create signal monitor module (same DSP, different GUI descriptor)
                            let mut m = synth_modules::SignalMonitor::new();

                            // Build an inline descriptor: compact type_id, no parameters, just ports
                            let inline_descriptor =
                                synth_core::ModuleDescriptor::new("inline_signal_monitor", "Mon")
                                    .description("Inline signal monitor (compact pass-through)")
                                    .category(synth_core::ModuleCategory::Utility)
                                    .port(synth_core::PortDescriptor::audio_input("in", "In"))
                                    .port(synth_core::PortDescriptor::audio_output("out", "Out"));

                            let monitor_id = {
                                let mut counters = self.session.counters_lock();
                                let counter = counters
                                    .entry((active_id, synth_core::ModuleType::SignalMonitor))
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
                                (Some(a), Some(b)) => {
                                    egui::Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
                                }
                                (Some(a), None) => egui::Pos2::new(a.x + 200.0, a.y),
                                _ => egui::Pos2::new(100.0, 100.0),
                            };

                            patch_editor.add_module_at(
                                monitor_id,
                                inline_descriptor.clone(),
                                mid_pos,
                            );

                            // Register in session so reconciliation doesn't remove it
                            self.session.register_descriptor(
                                active_id,
                                monitor_id,
                                inline_descriptor,
                            );

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
                                active_id,
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
                                    self.dialog_state.group_template_save_category =
                                        GroupCategory::default();
                                }
                            }
                        }

                        // Handle auto-layout request (from GUI menu or MCP).
                        //
                        // Only here do we clear the MCP pending flag —
                        // a request issued while another view was
                        // active waits until the next Rack frame.
                        if result.request_auto_layout || mcp_auto_layout {
                            patch_editor.apply_auto_layout(&effect_chain_order);
                            self.mark_dirty();
                            #[cfg(feature = "mcp")]
                            if mcp_auto_layout && let Some(shared) = self.mcp_shared.as_ref() {
                                shared
                                    .pending_auto_layout
                                    .store(false, std::sync::atomic::Ordering::Relaxed);
                            }
                        }

                        // Mark dirty if any mutations occurred
                        if had_mutations {
                            self.mark_dirty();
                        }
                    });
                }
                AppView::AcousticWorld => {
                    // Scan user presets on first view
                    if !self.awe_ui.user_presets_loaded {
                        let dir = self
                            .settings
                            .directories
                            .awe_presets_dir
                            .clone()
                            .or_else(|| crate::project::default_awe_presets_dir().ok());
                        if let Some(dir) = dir {
                            self.awe_ui.scan_user_presets(&dir);
                        } else {
                            self.awe_ui.user_presets_loaded = true;
                        }
                    }

                    let awe_action = crate::gui::awe_view::draw_awe_view(
                        ui,
                        &mut self.handle,
                        &mut self.awe_enabled,
                        &mut self.awe_ui,
                    );
                    match awe_action {
                        crate::gui::awe_view::AweViewAction::SavePreset => {
                            self.dialog_state.show_save_awe_preset = true;
                            self.dialog_state.awe_preset_save_name.clear();
                            self.dialog_state.awe_preset_save_description.clear();
                            self.dialog_state.awe_preset_save_tags.clear();
                        }
                        crate::gui::awe_view::AweViewAction::LoadPreset => {
                            let dir = self
                                .settings
                                .directories
                                .awe_presets_dir
                                .clone()
                                .or_else(|| crate::project::default_awe_presets_dir().ok());
                            self.dialog_state
                                .open_open_awe_preset_dialog(dir.as_deref());
                        }
                        crate::gui::awe_view::AweViewAction::None => {}
                    }
                }
                AppView::Sequencer => {
                    crate::gui::sequencer::draw_sequencer_view(
                        ui,
                        &mut self.handle,
                        &self.song,
                        &mut self.sequencer_view_state,
                        &self.instruments,
                        &mut self.undo_manager,
                    );
                }
                AppView::Sample => {
                    // Refresh input device cache on demand (not every frame)
                    if self.sample_view_state.devices_dirty {
                        self.sample_view_state.cached_input_devices = self
                            .host
                            .as_ref()
                            .and_then(|h| h.devices().ok())
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|d| {
                                matches!(
                                    d.device_type,
                                    synth_core::DeviceType::Input | synth_core::DeviceType::Duplex
                                )
                            })
                            .collect();
                        self.sample_view_state.devices_dirty = false;
                    }

                    let action = crate::gui::sample_view::draw_sample_view(
                        ui,
                        &self.sample_library,
                        &mut self.sample_view_state,
                        &mut self.audio_input,
                    );
                    match action {
                        crate::gui::sample_view::SampleViewAction::None => {}
                        crate::gui::sample_view::SampleViewAction::ImportWav => {
                            self.dialog_state.open_import_sample_dialog(None);
                        }
                        crate::gui::sample_view::SampleViewAction::ExportWav { name } => {
                            let wav_name = format!("{name}.wav");
                            self.dialog_state.open_export_sample_dialog(&wav_name, None);
                        }
                        crate::gui::sample_view::SampleViewAction::StartMonitoring => {
                            if let Some(host) = &self.host {
                                let device =
                                    self.sample_view_state.selected_input_device.as_deref();
                                let config = synth_core::StreamConfig {
                                    sample_rate: synth_core::audio::SampleRate::DVD_QUALITY,
                                    buffer_size: synth_core::BufferSize::MEDIUM,
                                    channels: synth_core::ChannelCount::Stereo,
                                };
                                match self.audio_input.start_monitoring(
                                    host.as_ref(),
                                    device,
                                    &config,
                                ) {
                                    Ok(()) => {
                                        // Send engine consumer so ProcessContext::audio_input works
                                        if let Some(consumer) =
                                            self.audio_input.take_engine_consumer()
                                        {
                                            self.handle.send(
                                                EngineCommand::SetAudioInputConsumer { consumer },
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        self.dialog_state.set_status(format!("Input error: {e}"));
                                    }
                                }
                            }
                        }
                        crate::gui::sample_view::SampleViewAction::StopMonitoring => {
                            self.handle.send(EngineCommand::ClearAudioInputConsumer);
                            self.audio_input.stop_monitoring();
                        }
                        crate::gui::sample_view::SampleViewAction::StartRecording => {
                            self.audio_input.start_recording();
                        }
                        crate::gui::sample_view::SampleViewAction::StopRecording => {
                            if let Some(data) = self.audio_input.stop_recording() {
                                let channels = self.audio_input.channels();
                                let sample_rate = self.audio_input.sample_rate();
                                let frame_count = if channels > 0 {
                                    data.len() / channels as usize
                                } else {
                                    0
                                };
                                let sample = synth_sampler::Sample::new(
                                    synth_sampler::SampleMeta {
                                        id: synth_sampler::SampleId::new(0),
                                        name: format!(
                                            "Recording {:.1}s",
                                            frame_count as f64 / f64::from(sample_rate.0)
                                        ),
                                        sample_rate,
                                        channels: synth_core::ChannelCount::from(channels),
                                        frame_count: synth_core::SampleCount::new(frame_count),
                                        root_note: None,
                                        loop_region: None,
                                        crop: None,
                                        source: synth_sampler::SampleSource::Recorded,
                                    },
                                    data.into(),
                                );
                                if let Ok(mut lib) = self.sample_library.write() {
                                    let id = lib.add(sample);
                                    self.sample_view_state.selected_sample = Some(id);
                                    self.sample_view_state.invalidate_peaks();
                                }
                                self.dialog_state.set_status("Recording saved");
                            }
                        }
                    }
                }
            }
        } // end 'view block

        // Dialogs
        self.show_dialogs(ctx);

        // Write current UI layout to MCP shared state
        #[cfg(feature = "mcp")]
        self.write_mcp_layout(ctx);

        // Update window title to reflect dirty state (only when changed)
        {
            let project_name = self
                .current_project_path
                .as_ref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .unwrap_or("Untitled");
            let title = format!(
                "Pertylizer v{} ({}) - {project_name}{}",
                env!("CARGO_PKG_VERSION"),
                env!("BUILD_DATE"),
                if self.dirty { " *" } else { "" },
            );
            if title != self.last_title {
                self.last_title = title.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
            }
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
            PaletteSelection::Sampler => Some(TypedModuleType::Sampler),
            PaletteSelection::AudioInput => Some(TypedModuleType::AudioInput),
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

        if let Err(e) = session.connect(
            instrument_id,
            connection.from_module,
            connection.from_port,
            connection.to_module,
            connection.to_port,
        ) {
            eprintln!("Failed to connect {connection:?} in quick-add: {e}");
        } else {
            editor.add_connection(connection);
        }
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
                    PaletteSelection::Sampler => Some(TypedModuleType::Sampler),
                    PaletteSelection::AudioInput => Some(TypedModuleType::AudioInput),
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

        // Layout: [Left Scope] [Piano Keys] [Right Scope] [Meter]
        // The horizontal row sets item_spacing to 0 so scopes sit flush
        // against the keyboard and the meter — no extra slack to budget for.
        //
        // Sizing strategy: reserve a minimum width for each scope first
        // and give the piano only the leftover. Whatever the piano
        // doesn't claim (rounding down to whole octaves) flows back to
        // the scopes as extra width.
        let available_width = ui.available_width();
        let meter_width = 30.0;
        let scope_min_width = 128.0;
        let keys_height = 100.0;

        // Piano: calculate octaves that fit in the leftover after reserving
        // space for both scopes and the meter.
        use crate::gui::keyboard::PianoKeyboard;
        let piano_budget = available_width - meter_width - 2.0 * scope_min_width;
        let num_octaves = PianoKeyboard::octaves_for_width(piano_budget.max(0.0));
        let piano_width = PianoKeyboard::width_for_octaves(num_octaves);

        // Scopes share everything that's left equally — they grow beyond
        // scope_min_width whenever the piano didn't use its full budget.
        let scope_total = (available_width - piano_width - meter_width).max(0.0);
        let min_scope_width = 40.0;
        let show_scopes = scope_total >= min_scope_width * 2.0;
        let scope_width = if show_scopes { scope_total / 2.0 } else { 0.0 };

        // Read samples into reusable buffers, then take ownership for this frame.
        // The buffers are returned to self at the end to preserve capacity.
        if show_scopes {
            self.handle
                .state
                .master_scope
                .read_samples_into(&mut self.scope_buf_l, &mut self.scope_buf_r);
        } else {
            self.scope_buf_l.clear();
            self.scope_buf_r.clear();
        }
        let mut samples_l = std::mem::take(&mut self.scope_buf_l);
        let mut samples_r = std::mem::take(&mut self.scope_buf_r);

        let (peak_l, peak_r) = self.handle.peak_meters();
        let (peak_l, peak_r) = (peak_l.as_f32(), peak_r.as_f32());
        let (rms_l, rms_r) = self.handle.rms_meters();
        let (rms_l, rms_r) = (rms_l.as_f32(), rms_r.as_f32());

        ui.horizontal(|ui| {
            // Keep scopes and meter flush against the keyboard — no item spacing
            // between widgets in this row.
            ui.spacing_mut().item_spacing.x = 0.0;

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
        });

        // Return buffers to self so their capacity is reused next frame
        samples_l.clear();
        samples_r.clear();
        self.scope_buf_l = samples_l;
        self.scope_buf_r = samples_r;
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

    /// Handle Ctrl/Cmd+Shift+A — toggle the analyze window.
    fn handle_analyze_shortcut(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused() {
            return;
        }
        let toggle =
            ctx.input(|i| i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::A));
        if toggle {
            self.analyze_window.toggle_open();
        }
    }

    /// Re-target the analyze window to the active instrument, then render it.
    /// Always runs (even when closed) so the worker-thread poll can drain a
    /// finished render if the user closed the window mid-flight.
    fn render_analyze_window(&mut self, ctx: &egui::Context) {
        if let Some(active_id) = self.active_instrument_id
            && let Some(name) = self
                .instruments
                .iter()
                .find(|i| i.id == active_id)
                .map(|i| i.name.clone())
        {
            self.analyze_window.set_target(active_id, name);
        }
        self.analyze_window
            .show(ctx, &self.session, &self.sample_library);
    }

    /// Floating window for editing project / song metadata: song name,
    /// song author, default tempo, default time signature, and the
    /// project-level `Author` (name / email / website / license).
    /// Live-mutates `self.song` and `self.current_project_author`.
    fn render_project_edit_window(&mut self, ctx: &egui::Context) {
        use egui_remixicon::icons as ri;

        if !self.project_edit_open {
            return;
        }

        let t = theme();
        let mut open = true;
        let mut song_changed = false;
        let mut author_changed = false;
        let mut send_tempo: Option<synth_core::Bpm> = None;

        egui::Window::new(format!("{} Project Info", ri::EDIT_LINE))
            .id(egui::Id::new("project_edit_window"))
            .open(&mut open)
            .resizable(true)
            .vscroll(true)
            .default_size([420.0, 520.0])
            .show(ctx, |ui| {
                // Song-level fields ----------------------------------------------------
                {
                    let mut song = self.song.write();

                    ui.label(RichText::new("Song name").color(t.colors.text_dim));
                    if ui.text_edit_singleline(&mut song.name).changed() {
                        song_changed = true;
                    }

                    ui.add_space(8.0);

                    ui.label(RichText::new("Song author").color(t.colors.text_dim));
                    if ui.text_edit_singleline(&mut song.author).changed() {
                        song_changed = true;
                    }

                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Tempo (BPM)").color(t.colors.text_dim));
                        let mut bpm = song.default_tempo.as_f32();
                        if ui
                            .add(
                                egui::DragValue::new(&mut bpm)
                                    .range(20.0..=300.0)
                                    .speed(0.5)
                                    .fixed_decimals(1),
                            )
                            .changed()
                        {
                            let clamped = bpm.clamp(20.0, 300.0);
                            let new_bpm = synth_core::Bpm::new(clamped);
                            song.default_tempo = new_bpm;
                            send_tempo = Some(new_bpm);
                            song_changed = true;
                        }
                    });

                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Time signature").color(t.colors.text_dim));
                        let mut num = song.default_time_signature.numerator as i32;
                        if ui
                            .add(egui::DragValue::new(&mut num).range(1..=32).speed(0.1))
                            .changed()
                        {
                            song.default_time_signature.numerator = num.clamp(1, 32) as u8;
                            song_changed = true;
                        }
                        ui.label("/");
                        let current_den = song.default_time_signature.denominator;
                        egui::ComboBox::from_id_salt("project_edit_time_sig_den")
                            .selected_text(current_den.to_string())
                            .width(60.0)
                            .show_ui(ui, |ui| {
                                for d in [1u8, 2, 4, 8, 16, 32] {
                                    if ui
                                        .selectable_label(current_den == d, d.to_string())
                                        .clicked()
                                        && song.default_time_signature.denominator != d
                                    {
                                        song.default_time_signature.denominator = d;
                                        song_changed = true;
                                    }
                                }
                            });
                    });
                } // <- release song write lock before touching other self fields

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);

                // Project author (Author struct) --------------------------------------
                ui.label(RichText::new("Project Author").color(t.colors.text_primary));
                ui.add_space(4.0);

                ui.label(RichText::new("Name").color(t.colors.text_dim));
                if ui
                    .text_edit_singleline(&mut self.current_project_author.name)
                    .changed()
                {
                    author_changed = true;
                }

                ui.add_space(6.0);

                ui.label(RichText::new("Email").color(t.colors.text_dim));
                if ui
                    .text_edit_singleline(&mut self.current_project_author.email)
                    .changed()
                {
                    author_changed = true;
                }

                ui.add_space(6.0);

                ui.label(RichText::new("Website").color(t.colors.text_dim));
                if ui
                    .text_edit_singleline(&mut self.current_project_author.website)
                    .changed()
                {
                    author_changed = true;
                }

                ui.add_space(6.0);

                ui.label(RichText::new("License").color(t.colors.text_dim));
                if ui
                    .text_edit_singleline(&mut self.current_project_author.license)
                    .changed()
                {
                    author_changed = true;
                }
            });

        if !open {
            self.project_edit_open = false;
        }
        if let Some(tempo) = send_tempo {
            self.handle.send(EngineCommand::SetTempo(tempo));
        }
        if song_changed || author_changed {
            self.mark_dirty();
        }
    }

    /// Floating window for editing an instrument's basic settings
    /// (identity + the performance controls also shown in the strip
    /// above the keyboard). Live-mutates the `InstrumentUiState` and
    /// forwards engine-relevant changes to the session/handle. Closed
    /// via the window X or when the target instrument no longer exists.
    /// Modal confirmation for instrument deletion. Instrument-level undo
    /// isn't wired into `UndoManager` yet, so a confirm step prevents
    /// accidental unrecoverable deletes.
    fn render_instrument_delete_confirm(&mut self, ctx: &egui::Context) {
        let Some(target_id) = self.pending_instrument_delete else {
            return;
        };
        let name = self
            .instruments
            .iter()
            .find(|i| i.id == target_id)
            .map(|i| i.name.clone());
        let Some(name) = name else {
            // Instrument vanished out from under us — clear and bail.
            self.pending_instrument_delete = None;
            return;
        };
        let t = theme();
        let mut open = true;
        let mut decision: Option<bool> = None;
        egui::Window::new("Delete instrument?")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Delete \"{name}\"? Notes referencing it remain on tracks but go silent until \
                     reassigned. This cannot be undone."
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                    if ui
                        .button(
                            RichText::new(format!(
                                "{} Delete",
                                egui_remixicon::icons::DELETE_BIN_LINE
                            ))
                            .color(t.colors.accent_red),
                        )
                        .clicked()
                    {
                        decision = Some(true);
                    }
                });
            });
        match decision {
            Some(true) => {
                self.delete_instrument(target_id);
                self.pending_instrument_delete = None;
            }
            Some(false) => self.pending_instrument_delete = None,
            None if !open => self.pending_instrument_delete = None,
            None => {}
        }
    }

    fn render_instrument_edit_window(&mut self, ctx: &egui::Context) {
        use egui_remixicon::icons as ri;
        use synth_engine::InstrumentCategory;

        let Some(target_id) = self.instrument_edit_target else {
            return;
        };
        let Some(idx) = self.instruments.iter().position(|i| i.id == target_id) else {
            self.instrument_edit_target = None;
            return;
        };

        let t = theme();
        let title = format!(
            "{} Edit instrument: {}",
            ri::EDIT_LINE,
            self.instruments[idx].name
        );

        let mut open = true;
        let mut name_changed = false;
        let mut category_changed = false;
        let mut other_changed = false;
        let mut send_midi_channel: Option<MidiChannel> = None;
        let mut send_volume = false;
        let mut send_pan = false;
        let mut send_transpose = false;
        let mut send_oversampling = false;
        let mut send_alloc_mode = false;
        let mut send_vel_amp = false;
        let mut send_vel_filter = false;
        let mut send_description = false;
        let mut send_patch_description = false;
        let mut send_sidechain: Option<Option<InstrumentId>> = None;
        // Other-instrument list captured before the mutable borrow.
        let other_instruments: Vec<(InstrumentId, String)> = self
            .instruments
            .iter()
            .filter(|i| i.id != target_id)
            .map(|i| (i.id, i.name.clone()))
            .collect();

        egui::Window::new(title)
            .id(egui::Id::new((
                "instrument_edit_window",
                target_id.as_u64(),
            )))
            .open(&mut open)
            .resizable(true)
            .vscroll(true)
            .default_size([400.0, 560.0])
            .show(ctx, |ui| {
                let inst = &mut self.instruments[idx];

                ui.label(RichText::new("Name").color(t.colors.text_dim));
                if ui.text_edit_singleline(&mut inst.name).changed() {
                    name_changed = true;
                }

                ui.add_space(8.0);

                ui.label(RichText::new("Category").color(t.colors.text_dim));
                let mut cat = inst.category;
                egui::ComboBox::from_id_salt("instrument_edit_category")
                    .selected_text(cat.name())
                    .show_ui(ui, |ui| {
                        for variant in [
                            InstrumentCategory::Uncategorized,
                            InstrumentCategory::Drums,
                            InstrumentCategory::Bass,
                            InstrumentCategory::Pad,
                            InstrumentCategory::Lead,
                            InstrumentCategory::Arp,
                            InstrumentCategory::Keys,
                            InstrumentCategory::FX,
                        ] {
                            ui.selectable_value(&mut cat, variant, variant.name());
                        }
                    });
                if cat != inst.category {
                    inst.category = cat;
                    category_changed = true;
                }

                ui.add_space(8.0);

                ui.label(RichText::new("Description").color(t.colors.text_dim))
                    .on_hover_text(
                        "Per-instance song-role intent (e.g. \"chorus pad\", \"sub layer\"). \
                        Distinct from Patch description below.",
                    );
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut inst.description)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    other_changed = true;
                    send_description = true;
                }

                ui.add_space(6.0);

                ui.label(RichText::new("Patch description").color(t.colors.text_dim))
                    .on_hover_text(
                        "Sound-design intent for this patch (how it works, what it's good for). \
                        Travels with the patch on save and is read back via MCP.",
                    );
                if ui
                    .add(
                        egui::TextEdit::multiline(&mut inst.patch_description)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    )
                    .changed()
                {
                    other_changed = true;
                    send_patch_description = true;
                }

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Color").color(t.colors.text_dim));
                    let mut color = inst
                        .color
                        .as_deref()
                        .and_then(crate::gui::patch_editor::parse_hex_color)
                        .unwrap_or(Color32::from_rgba_unmultiplied(128, 128, 128, 255));
                    if egui::color_picker::color_edit_button_srgba(
                        ui,
                        &mut color,
                        egui::color_picker::Alpha::BlendOrAdditive,
                    )
                    .changed()
                    {
                        inst.color = Some(crate::gui::patch_editor::color32_to_hex(color));
                        other_changed = true;
                    }
                    if inst.color.is_some() && ui.button("Clear").clicked() {
                        inst.color = None;
                        other_changed = true;
                    }
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);

                // ── Performance controls (mirrors the strip above the keyboard) ──

                // MIDI channel
                ui.horizontal(|ui| {
                    ui.label(RichText::new("MIDI Channel").color(t.colors.text_dim));
                    let channel = inst.channel;
                    let channel_label = if channel.is_omni() {
                        "Omni".to_string()
                    } else {
                        format!("Ch {}", channel.as_one_indexed())
                    };
                    egui::ComboBox::from_id_salt("instrument_edit_midi_ch")
                        .selected_text(channel_label)
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(channel.is_omni(), "Omni").clicked() {
                                inst.channel = MidiChannel::OMNI;
                                send_midi_channel = Some(MidiChannel::OMNI);
                            }
                            for ch in 1..=16u8 {
                                let Some(midi_ch) = MidiChannel::from_one_indexed(ch) else {
                                    continue;
                                };
                                let is_selected =
                                    !channel.is_omni() && channel.as_one_indexed() == ch;
                                if ui
                                    .selectable_label(is_selected, format!("Ch {ch}"))
                                    .clicked()
                                {
                                    inst.channel = midi_ch;
                                    send_midi_channel = Some(midi_ch);
                                }
                            }
                        });
                });

                ui.add_space(6.0);

                // Volume + Mute + Solo on one row
                let muted = inst.muted;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Volume").color(t.colors.text_dim));
                    let mut vol = inst.volume.as_f32();
                    let vol_resp = ui
                        .add(
                            egui::Slider::new(&mut vol, 0.0..=1.0)
                                .fixed_decimals(2)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                        )
                        .on_hover_text("Instrument volume");
                    if vol_resp.changed() && !muted {
                        inst.set_volume(synth_core::Gain::new(vol));
                        send_volume = true;
                    }
                });

                ui.add_space(6.0);

                // Pan slider
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Pan").color(t.colors.text_dim));
                    let mut pan = inst.pan.as_f32();
                    let resp = ui
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
                    if resp.changed() {
                        inst.pan = synth_core::BipolarValue::new(pan);
                        send_pan = true;
                    }
                });

                ui.add_space(6.0);

                // Transpose
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Transpose").color(t.colors.text_dim));
                    let mut transpose = inst.transpose.as_f32().round() as i32;
                    let resp = ui
                        .add(
                            egui::DragValue::new(&mut transpose)
                                .range(-24..=24)
                                .speed(0.1)
                                .suffix(" st"),
                        )
                        .on_hover_text("Transpose in semitones (-24 to +24)");
                    if resp.changed() {
                        let new_transpose =
                            synth_core::Semitones::new(transpose.clamp(-24, 24) as f32);
                        inst.transpose = new_transpose;
                        send_transpose = true;
                    }
                });

                ui.add_space(6.0);

                // Oversampling
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Oversampling").color(t.colors.text_dim));
                    let current_os = inst.oversampling;
                    egui::ComboBox::from_id_salt("instrument_edit_os")
                        .selected_text(current_os.name())
                        .width(80.0)
                        .show_ui(ui, |ui| {
                            for factor in synth_dsp::OversamplingFactor::ALL {
                                if ui
                                    .selectable_label(current_os == factor, factor.name())
                                    .clicked()
                                {
                                    inst.oversampling = factor;
                                    send_oversampling = true;
                                }
                            }
                        })
                        .response
                        .on_hover_text("Oversampling factor (reduces aliasing)");
                });

                ui.add_space(6.0);

                // Polyphony / voice count. Changing this only takes effect
                // after the project is reloaded — the engine can't resize
                // the voice pool at runtime.
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Max voices").color(t.colors.text_dim));
                    let mut voices = inst.max_voices.0 as i32;
                    let resp = ui
                        .add(egui::DragValue::new(&mut voices).range(1..=128).speed(0.2))
                        .on_hover_text(
                            "Maximum polyphony (1–128). Takes effect on project reload.",
                        );
                    if resp.changed() {
                        inst.max_voices = synth_core::VoiceCount::new(voices.clamp(1, 128) as u8);
                        other_changed = true;
                    }
                });

                ui.add_space(6.0);

                // Allocation mode
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Mode").color(t.colors.text_dim));
                    let current = inst.allocation_mode;
                    egui::ComboBox::from_id_salt("instrument_edit_alloc_mode")
                        .selected_text(format!("{current:?}"))
                        .width(110.0)
                        .show_ui(ui, |ui| {
                            for mode in [
                                synth_engine::voice_allocator::AllocationMode::Polyphonic,
                                synth_engine::voice_allocator::AllocationMode::Mono,
                                synth_engine::voice_allocator::AllocationMode::Legato,
                                synth_engine::voice_allocator::AllocationMode::Unison,
                            ] {
                                if ui
                                    .selectable_label(current == mode, format!("{mode:?}"))
                                    .clicked()
                                {
                                    inst.allocation_mode = mode;
                                    send_alloc_mode = true;
                                }
                            }
                        });
                });

                ui.add_space(6.0);

                // Velocity → amp sensitivity
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Vel {} Amp",
                            egui_remixicon::icons::ARROW_RIGHT_S_LINE
                        ))
                        .color(t.colors.text_dim),
                    );
                    let mut s = inst.velocity_amp_sensitivity.as_f32();
                    let resp = ui
                        .add(
                            egui::Slider::new(&mut s, 0.0..=1.0)
                                .fixed_decimals(2)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                        )
                        .on_hover_text("Velocity to amplitude sensitivity (0 = flat, 1 = full)");
                    if resp.changed() {
                        inst.velocity_amp_sensitivity = synth_core::NormalizedValue::new(s);
                        send_vel_amp = true;
                    }
                });

                ui.add_space(6.0);

                // Velocity → filter sensitivity
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Vel {} Filter",
                            egui_remixicon::icons::ARROW_RIGHT_S_LINE
                        ))
                        .color(t.colors.text_dim),
                    );
                    let mut s = inst.velocity_filter_sensitivity.as_f32();
                    let resp = ui
                        .add(
                            egui::Slider::new(&mut s, 0.0..=1.0)
                                .fixed_decimals(2)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                        )
                        .on_hover_text(
                            "Velocity to filter cutoff sensitivity (0 = none, 1 = full)",
                        );
                    if resp.changed() {
                        inst.velocity_filter_sensitivity = synth_core::NormalizedValue::new(s);
                        send_vel_filter = true;
                    }
                });

                ui.add_space(6.0);

                // Sidechain source
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Sidechain").color(t.colors.text_dim));
                    let current = inst.sidechain_source_id;
                    let label = match current {
                        Some(src) => other_instruments
                            .iter()
                            .find(|(id, _)| *id == src)
                            .map(|(_, n)| n.clone())
                            .unwrap_or_else(|| format!("#{}", src.as_u64())),
                        None => "— None —".to_owned(),
                    };
                    egui::ComboBox::from_id_salt("instrument_edit_sidechain")
                        .selected_text(label)
                        .width(160.0)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(current.is_none(), "— None —").clicked()
                                && current.is_some()
                            {
                                inst.sidechain_source_id = None;
                                send_sidechain = Some(None);
                            }
                            for (id, name) in &other_instruments {
                                let selected = current == Some(*id);
                                if ui.selectable_label(selected, name).clicked()
                                    && current != Some(*id)
                                {
                                    inst.sidechain_source_id = Some(*id);
                                    send_sidechain = Some(Some(*id));
                                }
                            }
                        })
                        .response
                        .on_hover_text(
                            "Route another instrument's audio output into this \
                            instrument's compressors for classic pumping/ducking. \
                            The compressor's Sidechain Enabled parameter must also \
                            be on. Detection latency is ~1 audio buffer.",
                        );
                });
            });

        if !open {
            self.instrument_edit_target = None;
        }

        if name_changed {
            let new_name = self.instruments[idx].name.clone();
            if let Err(e) = self.session.rename_instrument(target_id, &new_name) {
                eprintln!("Failed to rename instrument {target_id:?}: {e}");
            }
        }
        if category_changed {
            let cat = self.instruments[idx].category;
            if let Err(e) = self.session.set_instrument_category(target_id, cat) {
                eprintln!("Failed to set instrument category {target_id:?}: {e}");
            }
        }
        if let Some(channel) = send_midi_channel {
            self.handle.send(EngineCommand::SetInstrumentMidiChannel {
                instrument_id: target_id,
                channel,
            });
        }
        if send_volume {
            let volume = self.instruments[idx].volume;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: target_id,
                param: synth_engine::InstrumentParam::Volume(volume),
            });
        }
        if send_pan {
            let pan = self.instruments[idx].pan;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: target_id,
                param: synth_engine::InstrumentParam::Pan(pan),
            });
        }
        if send_transpose {
            let transpose = self.instruments[idx].transpose;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: target_id,
                param: synth_engine::InstrumentParam::Transpose(transpose),
            });
        }
        if send_oversampling {
            let os = self.instruments[idx].oversampling;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: target_id,
                param: synth_engine::InstrumentParam::OversamplingFactor(os),
            });
        }
        if send_alloc_mode {
            let mode = self.instruments[idx].allocation_mode;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: target_id,
                param: synth_engine::InstrumentParam::AllocationMode(mode),
            });
        }
        if send_vel_amp {
            let s = self.instruments[idx].velocity_amp_sensitivity;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: target_id,
                param: synth_engine::InstrumentParam::VelocityAmpSensitivity(s),
            });
        }
        if send_vel_filter {
            let s = self.instruments[idx].velocity_filter_sensitivity;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: target_id,
                param: synth_engine::InstrumentParam::VelocityFilterSensitivity(s),
            });
        }
        if send_description {
            let description = self.instruments[idx].description.clone();
            if let Err(e) = self
                .session
                .set_instrument_description(target_id, &description)
            {
                eprintln!("Failed to set instrument description {target_id:?}: {e}");
            }
        }
        if send_patch_description {
            let description = self.instruments[idx].patch_description.clone();
            let value = if description.is_empty() {
                None
            } else {
                Some(description.as_str())
            };
            if let Err(e) = self.session.set_patch_description(target_id, value) {
                eprintln!("Failed to set patch description {target_id:?}: {e}");
            }
        }
        if let Some(source) = send_sidechain
            && let Err(e) = self.session.set_sidechain_source(target_id, source)
        {
            eprintln!("Failed to set sidechain source for {target_id:?}: {e}");
        }
        if name_changed
            || category_changed
            || other_changed
            || send_midi_channel.is_some()
            || send_volume
            || send_pan
            || send_transpose
            || send_oversampling
            || send_alloc_mode
            || send_vel_amp
            || send_vel_filter
            || send_description
            || send_patch_description
            || send_sidechain.is_some()
        {
            self.mark_dirty();
        }
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
        if ctx.text_edit_focused() {
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

        let Some(instrument_id) = self.active_instrument_id else {
            return;
        };

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
            let mut song = self.song.write();
            song.remove_unused()
        };

        // Remove instruments not referenced by remaining tracks/notes
        let to_remove: Vec<_> = self
            .instruments
            .iter()
            .filter(|inst| {
                #[allow(clippy::cast_possible_truncation)]
                let seq_id = synth_sequencer::SeqInstrumentId(inst.id.as_u64() as u16);
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
        if self
            .active_instrument_id
            .is_some_and(|id| !self.instruments.iter().any(|i| i.id == id))
        {
            self.active_instrument_id = self.instruments.first().map(|i| i.id);
            self.handle
                .set_focused_instrument(self.active_instrument_id);
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
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
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
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
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
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.move_note(*note_id, *new_start);
                }
            }
            UndoAction::ResizeNote {
                pattern_id,
                note_id,
                new_duration,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
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
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id)
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
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.set_note_velocity(*note_id, *new_velocity);
                }
            }
            UndoAction::SetVelocitiesBatch {
                pattern_id,
                changes,
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    for (note_id, _old, new) in changes {
                        pattern.set_note_velocity(*note_id, *new);
                    }
                }
            }
            UndoAction::RenamePattern {
                pattern_id,
                new_name,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.name = new_name.clone();
                }
            }
            UndoAction::RenameTrack {
                track_id, new_name, ..
            } => {
                let mut song_w = self.song.write();
                if let Some(track) = song_w.track_mut(*track_id) {
                    track.name = new_name.clone();
                }
            }
            UndoAction::SetPatternLength {
                pattern_id,
                new_length,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.length = *new_length;
                }
            }
            UndoAction::SetTempo { tick, new_bpm, .. } => {
                let mut song_w = self.song.write();
                if let Some(bpm) = *new_bpm {
                    song_w.set_tempo_at(*tick, bpm);
                } else {
                    song_w.remove_tempo_change(*tick);
                }
            }
            UndoAction::MovePlacement {
                pattern_id,
                old_track_id,
                old_start,
                new_track_id,
                new_start,
            } => {
                let mut song_w = self.song.write();
                song_w.move_placement(
                    *pattern_id,
                    *old_track_id,
                    *old_start,
                    *new_track_id,
                    *new_start,
                );
            }
            UndoAction::AddAutomationPoint {
                pattern_id,
                target,
                tick,
                value,
                curve,
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    let lane = pattern.get_or_create_automation(target.clone());
                    let mut point = synth_sequencer::AutomationPoint::new(*tick, *value);
                    point.curve = *curve;
                    lane.add_point(point);
                }
            }
            UndoAction::RemoveAutomationPoint {
                pattern_id,
                target,
                tick,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id)
                    && let Some(lane) = pattern.automation.iter_mut().find(|l| &l.target == target)
                {
                    lane.remove_point(*tick);
                }
            }
            UndoAction::DeleteTrack {
                track, placements, ..
            } => {
                let mut song_w = self.song.write();
                song_w.delete_track(track.id);
                let _ = placements;
            }
            UndoAction::AddTrack {
                track,
                track_index,
                placements,
            } => {
                let mut song_w = self.song.write();
                song_w.insert_track(track.clone(), Some(*track_index));
                for p in placements {
                    song_w.insert_placement(p.clone());
                }
            }
            UndoAction::DeletePattern {
                pattern,
                placements,
            } => {
                let mut song_w = self.song.write();
                song_w.delete_pattern(pattern.id);
                let _ = placements;
            }
            UndoAction::AddPattern {
                pattern,
                placements,
            } => {
                let mut song_w = self.song.write();
                song_w.insert_pattern(pattern.clone());
                for p in placements {
                    song_w.insert_placement(p.clone());
                }
            }
            UndoAction::RemovePlacement { placement } => {
                let mut song_w = self.song.write();
                song_w.remove_placement(placement.pattern_id, placement.track_id, placement.start);
            }
            UndoAction::InsertPlacement { placement } => {
                let mut song_w = self.song.write();
                song_w.insert_placement(placement.clone());
            }
            UndoAction::SetPlacementLength {
                pattern_id,
                track_id,
                start,
                new_length,
                ..
            } => {
                let mut song_w = self.song.write();
                song_w.set_placement_length(*pattern_id, *track_id, *start, *new_length);
            }
            UndoAction::MoveAutomationPoint {
                pattern_id,
                target,
                old_tick,
                new_tick,
                new_value,
                curve,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    let lane = pattern.get_or_create_automation(target.clone());
                    lane.remove_point(*old_tick);
                    let mut point = synth_sequencer::AutomationPoint::new(*new_tick, *new_value);
                    point.curve = *curve;
                    lane.add_point(point);
                }
            }
            UndoAction::AddModule {
                instrument_id,
                module_state,
                connections,
            } => {
                // TODO: Full undo requires recreating the module from ModuleState
                // via the session, rebuilding connections, and updating the patch
                // editor. This needs significant refactoring of the session API
                // to support module reconstruction from serialized state.
                let _ = (instrument_id, module_state, connections);
                eprintln!("Undo: AddModule not yet implemented — requires session refactoring");
            }
            UndoAction::RemoveModule {
                instrument_id,
                module_state,
                connections: _,
            } => {
                // TODO: Full undo requires re-adding the removed module from its
                // saved ModuleState and restoring all connections. Same session
                // refactoring needed as AddModule above.
                let _ = (instrument_id, module_state);
                eprintln!("Undo: RemoveModule not yet implemented — requires session refactoring");
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

        // Save AWE preset dialog
        if self.dialog_state.show_save_awe_preset {
            match crate::gui::dialogs::show_save_awe_preset_dialog(
                ctx,
                &mut self.dialog_state.show_save_awe_preset,
                &mut self.dialog_state.awe_preset_save_name,
                &mut self.dialog_state.awe_preset_save_description,
                &mut self.dialog_state.awe_preset_save_tags,
                &self.settings.author,
            ) {
                crate::gui::dialogs::SaveAwePresetResult::Save => {
                    let state = self.awe_ui.to_awe_state(self.awe_enabled);
                    let mut preset =
                        AwePresetFile::new(self.dialog_state.awe_preset_save_name.trim(), state);
                    preset.description = self
                        .dialog_state
                        .awe_preset_save_description
                        .trim()
                        .to_string();
                    preset.tags = self
                        .dialog_state
                        .awe_preset_save_tags
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                    if !self.settings.author.is_empty() {
                        preset.author = Some(self.settings.author.clone());
                    }

                    // Open file save dialog
                    let default_name =
                        format!("{}.json", preset.name.to_lowercase().replace(' ', "_"));
                    let dir = self
                        .settings
                        .directories
                        .awe_presets_dir
                        .clone()
                        .or_else(|| crate::project::default_awe_presets_dir().ok());
                    // Ensure directory exists
                    if let Some(ref d) = dir {
                        let _ = std::fs::create_dir_all(d);
                    }
                    self.pending_awe_preset_save = Some(preset);
                    self.dialog_state
                        .open_save_awe_preset_dialog(&default_name, dir.as_deref());
                }
                crate::gui::dialogs::SaveAwePresetResult::Cancelled
                | crate::gui::dialogs::SaveAwePresetResult::None => {}
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
                            self.load_project_data(*proj);
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
                        Ok(LoadedFile::AwePreset(preset)) => {
                            self.load_awe_preset_data(&preset);
                            self.dialog_state
                                .set_status(format!("Loaded AWE preset: {}", preset.name));
                        }
                        Ok(LoadedFile::Bundle(bundle_path)) => {
                            match self.load_bundle_file(&bundle_path) {
                                Ok(msg) => {
                                    self.current_project_path = Some(path.clone());
                                    self.dirty = false;
                                    self.settings.add_recent_project(path.clone());
                                    self.settings.save();
                                    self.dialog_state.set_status(msg);
                                }
                                Err(e) => {
                                    self.dialog_state
                                        .set_status(format!("Bundle load error: {e}"));
                                }
                            }
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
                    // Use bundle format if there are samples, plain JSON otherwise
                    let has_samples = self.sample_library.read().is_ok_and(|lib| !lib.is_empty());
                    let save_result = if has_samples {
                        if let Ok(lib) = self.sample_library.read() {
                            crate::bundle::save_bundle(&proj, &lib, &path)
                        } else {
                            proj.save(&path)
                        }
                    } else {
                        proj.save(&path)
                    };
                    match save_result {
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
                FileDialogResult::Picked(path, Some(FileDialogMode::OpenAwePreset)) => {
                    match AwePresetFile::load(&path) {
                        Ok(preset) => {
                            self.load_awe_preset_data(&preset);
                            self.dialog_state
                                .set_status(format!("Loaded AWE preset: {}", preset.name));
                            // Refresh user presets list
                            self.awe_ui.user_presets_loaded = false;
                        }
                        Err(e) => {
                            self.dialog_state
                                .set_status(format!("Error loading AWE preset: {e}"));
                        }
                    }
                }
                FileDialogResult::Saved(path, Some(FileDialogMode::SaveAwePreset)) => {
                    if let Some(preset) = self.pending_awe_preset_save.take() {
                        match preset.save(&path) {
                            Ok(()) => {
                                self.dialog_state
                                    .set_status(format!("AWE preset saved: {}", path.display()));
                                // Refresh user presets list
                                self.awe_ui.user_presets_loaded = false;
                            }
                            Err(e) => {
                                self.dialog_state
                                    .set_status(format!("Error saving AWE preset: {e}"));
                            }
                        }
                    }
                }
                FileDialogResult::Picked(path, Some(FileDialogMode::ImportSample)) => {
                    let target_rate = synth_core::audio::SampleRate::DVD_QUALITY;
                    match synth_sampler::load_wav(&path, target_rate) {
                        Ok(sample) => {
                            let name = sample.meta.name.clone();
                            if let Ok(mut lib) = self.sample_library.write() {
                                let id = lib.add(sample);
                                self.sample_view_state.selected_sample = Some(id);
                                self.sample_view_state.invalidate_peaks();
                            }
                            self.dialog_state
                                .set_status(format!("Imported sample: {name}"));
                        }
                        Err(e) => {
                            self.dialog_state.set_status(format!("Import failed: {e}"));
                        }
                    }
                }
                FileDialogResult::Saved(path, Some(FileDialogMode::ExportSample)) => {
                    if let Some(id) = self.sample_view_state.selected_sample
                        && let Ok(lib) = self.sample_library.read()
                        && let Some(sample) = lib.get(id)
                    {
                        let save_path = if path.extension().is_none() {
                            path.with_extension("wav")
                        } else {
                            path
                        };
                        match synth_sampler::save_wav(
                            sample,
                            &save_path,
                            synth_sampler::BitDepth::Int16,
                        ) {
                            Ok(()) => {
                                self.dialog_state
                                    .set_status(format!("Exported: {}", save_path.display()));
                            }
                            Err(e) => {
                                self.dialog_state.set_status(format!("Export failed: {e}"));
                            }
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
        let Some(active_id) = self.active_instrument_id else {
            return;
        };
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
    /// If no instrument is active, one is auto-created so the user can load a
    /// patch into a fresh project without first clicking "+ New Instrument".
    fn load_patch_data(&mut self, patch: &Patch) {
        self.mark_dirty();

        // Delegate to patch_bridge for the main loading logic
        // Load into the active instrument's patch editor (auto-create if none)
        let active_id = match self.active_instrument_id {
            Some(id) => id,
            None => match self.add_new_instrument() {
                Some(id) => id,
                None => {
                    self.dialog_state
                        .set_status("Cannot load patch: failed to create instrument".to_string());
                    return;
                }
            },
        };
        let Some(instrument) = self.instruments.iter_mut().find(|i| i.id == active_id) else {
            self.dialog_state
                .set_status("Cannot load patch: active instrument missing".to_string());
            return;
        };

        // Clear visualization buffers only for the active instrument's modules
        for module_id in instrument.patch_editor.module_ids() {
            self.handle.visualization_buffers.remove(&module_id);
        }

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
            false,
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

            // Fetch live module state so newly-added modules can have their
            // current parameter values copied into the GUI's patch_editor —
            // without this, MCP-applied patches would render with descriptor
            // defaults regardless of what the engine actually runs.
            let module_snapshots = self
                .session
                .state()
                .shared_graph
                .get_modules_for_instrument(inst_id);

            for (module_id, descriptor) in to_add {
                let position = eframe::egui::Pos2::new(100.0, 100.0);
                patch_editor.add_module_at(module_id, descriptor, position);

                if let Some(snap) = module_snapshots.iter().find(|s| s.id == module_id) {
                    for param in &snap.parameters {
                        patch_editor.set_parameter_by_name(module_id, param.name(), param.as_f32());
                    }
                }
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

        // Remove GUI instruments that the engine no longer has, BUT only if
        // their PatchEditor is empty. During project loading, engine processes
        // AddInstrument commands asynchronously — GUI instruments with loaded
        // modules may not yet appear in shared state. Keeping them prevents
        // reconciliation from destroying loaded PatchEditors.
        self.instruments
            .retain(|i| engine_ids.contains(&i.id) || !i.patch_editor.module_ids().is_empty());

        // If active instrument was removed, switch to first available
        if self
            .active_instrument_id
            .is_some_and(|id| !self.instruments.iter().any(|i| i.id == id))
        {
            self.active_instrument_id = self.instruments.first().map(|i| i.id);
            self.handle
                .set_focused_instrument(self.active_instrument_id);
        }

        // Update metadata for existing instruments from the engine snapshot.
        // MCP is the source of truth for these fields when changed, so we
        // mirror them here so create_project_from_app (which reads from the
        // GUI-side InstrumentUiState) sees the latest values at save time.
        for snap in &snapshots {
            if let Some(ui_inst) = self.instruments.iter_mut().find(|i| i.id == snap.id) {
                ui_inst.name = snap.name.clone();
                if !ui_inst.muted {
                    ui_inst.volume = snap.volume;
                }
                ui_inst.pan = snap.pan;
                ui_inst.muted = snap.muted;
                ui_inst.solo = snap.solo;
                ui_inst.channel = synth_engine::MidiChannel::from_zero_indexed(
                    snap.midi_channel.as_u8().saturating_sub(1),
                )
                .unwrap_or(ui_inst.channel);
                ui_inst.description = snap.description.clone();
                ui_inst.patch_description = snap.patch_description.clone().unwrap_or_default();
                ui_inst.category = snap.category;
                ui_inst.sidechain_source_id = snap.sidechain_source_id;
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
        let Some(active_id) = self.active_instrument_id else {
            return;
        };

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

        // Clear all non-visualizer modules via session.
        // Failure is non-fatal during load — the instrument may already be empty.
        if let Err(e) = self.session.clear_graph(active_id) {
            eprintln!("clear_graph({active_id:?}) failed during load: {e}");
        }

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
        let active_id = self.active_instrument_id?;
        let instrument = self.instruments.iter().find(|i| i.id == active_id)?;
        let engine_state = Some((self.session.state().as_ref(), active_id));
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

    /// Build a `ProjectFile` from the current application state — the
    /// canonical engine-side builder plus a GUI overlay for fields the
    /// engine doesn't track (color, module positions, group metadata,
    /// canvas size, visualizer modules).
    fn create_project_from_app(&self) -> ProjectFile {
        let opts = self.build_save_options();
        let mut project = crate::project_apply::build_project_from_engine(
            &self.session,
            &self.song,
            &self.sample_library,
            opts,
        );
        // Snapshot ordering may differ from UI-state ordering; honour
        // whichever active id the GUI thinks it has so the file matches
        // the user's focused instrument.
        project.active_instrument_id = self.active_instrument_id.map_or(0, |id| id.as_u64());
        self.overlay_ui_metadata(&mut project);
        project
    }

    /// Build save options from current GUI state (author, AWE, glide,
    /// octave). MCP uses the same `ProjectBuildOptions` type with its
    /// own field sources (`McpSharedState`); this is the GUI's mirror.
    fn build_save_options(&self) -> crate::project_apply::ProjectBuildOptions {
        let author = if self.current_project_author.is_empty() {
            None
        } else {
            Some(self.current_project_author.clone())
        };

        // Prefer the MCP-shared `awe_description` slot when present so a
        // recent MCP write survives a save in the same frame (the
        // bi-directional sync at the top of update() runs *after* the
        // refresh handler, so self.awe_ui.description can be stale).
        #[cfg(feature = "mcp")]
        let awe_description = self
            .mcp_shared
            .as_ref()
            .map(|shared| {
                shared
                    .awe_description
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
            })
            .filter(|s| !s.is_empty())
            .or_else(|| {
                if self.awe_ui.description.is_empty() {
                    None
                } else {
                    Some(self.awe_ui.description.clone())
                }
            });
        #[cfg(not(feature = "mcp"))]
        let awe_description = if self.awe_ui.description.is_empty() {
            None
        } else {
            Some(self.awe_ui.description.clone())
        };

        let awe = if self.awe_enabled {
            Some(self.awe_ui.to_awe_state(true))
        } else {
            None
        };

        crate::project_apply::ProjectBuildOptions {
            author,
            awe,
            awe_description,
            glide_time: Some(self.glide_time),
            octave_offset: Some(self.keyboard.octave_offset()),
        }
    }

    /// Overlay GUI-only patch metadata onto a project built from engine
    /// state: instrument color, module positions, group metadata,
    /// canvas size, and visualizer modules (the engine doesn't track
    /// them — they live only in `PatchEditor`).
    fn overlay_ui_metadata(&self, project: &mut ProjectFile) {
        use std::collections::HashSet;
        for inst_state in &mut project.instruments {
            let Some(ui_inst) = self.instruments.iter().find(|i| i.id == inst_state.id) else {
                continue;
            };

            inst_state.color = ui_inst.color.clone();

            let patch = &mut inst_state.patch;

            let content_size = ui_inst.patch_editor.content_size();
            patch.settings.canvas_size = Some(crate::patch::CanvasSize::new(
                content_size.x,
                content_size.y,
            ));
            patch.groups = ui_inst.patch_editor.group_states();

            // Overwrite default `(0, 0)` positions on engine-tracked
            // modules with the canvas positions from the patch editor.
            for module in &mut patch.modules {
                if let Ok(module_id) = module.id.parse::<ModuleId>()
                    && let Some((_, pos, _)) = ui_inst.patch_editor.get_module_data(module_id)
                {
                    module.position = crate::patch::Position::new(pos.x, pos.y);
                }
            }

            // Append visualizer modules that exist in the PatchEditor
            // but not in the engine (apply_project skips visualizers,
            // so build_project_from_engine doesn't emit them either).
            let engine_ids: HashSet<String> = patch.modules.iter().map(|m| m.id.clone()).collect();
            for module_id in ui_inst.patch_editor.module_ids() {
                let id_str = module_id.to_string();
                if engine_ids.contains(&id_str) {
                    continue;
                }
                let Some((descriptor, position, gui_params)) =
                    ui_inst.patch_editor.get_module_data(module_id)
                else {
                    continue;
                };
                let name_to_type_id: std::collections::HashMap<&str, &str> = descriptor
                    .parameters
                    .iter()
                    .map(|p| (p.name.as_str(), p.type_id.as_str()))
                    .collect();
                let mut param_map = std::collections::BTreeMap::new();
                for (display_name, value) in &gui_params {
                    let key = name_to_type_id
                        .get(display_name.as_str())
                        .map_or_else(|| display_name.clone(), |tid| (*tid).to_string());
                    param_map.insert(key, crate::patch::ParamValue::Float(*value));
                }
                patch.modules.push(crate::patch::ModuleState {
                    id: id_str,
                    module_type: module_id.module_type,
                    position: crate::patch::Position::new(position.x, position.y),
                    parameters: param_map,
                });
            }
        }
    }

    /// Load a ZIP bundle project file with embedded samples.
    fn load_bundle_file(&mut self, path: &std::path::Path) -> Result<String, String> {
        let library_clone = std::sync::Arc::clone(&self.sample_library);
        let (project, sample_count) = {
            let mut lib = library_clone
                .write()
                .map_err(|_| "Failed to acquire sample library lock".to_string())?;
            let project = crate::bundle::load_bundle(path, &mut lib).map_err(|e| format!("{e}"))?;
            let count = lib.len();
            (project, count)
        };
        self.load_project_data(project);
        self.sample_view_state.invalidate_peaks();
        Ok(format!(
            "Bundle loaded: {} ({sample_count} samples)",
            path.display()
        ))
    }

    /// Rebuild the GUI's UI mirrors against `project`. Assumes engine
    /// state has already been written via `project_apply::apply_project`;
    /// this only touches `InstrumentUiState`, `PatchEditor` canvases,
    /// visualization buffers, glide / awe / keyboard widgets, and the
    /// active-instrument bookkeeping.
    fn refresh_ui_from_project(&mut self, project: &ProjectFile) {
        // 1. Tear down old UI mirrors. The engine instruments are already
        //    gone (apply_project ran tear_down_all_instruments); we just
        //    need to drop the GUI's visualization buffers and the
        //    `InstrumentUiState` vector.
        let old_ids: Vec<InstrumentId> = self.instruments.iter().map(|i| i.id).collect();
        for inst_id in &old_ids {
            self.remove_visualizers_for_instrument(*inst_id);
        }
        self.instruments.clear();
        self.active_instrument_id = None;
        self.handle.visualization_buffers.clear();

        // 2. Build fresh InstrumentUiState per instrument and populate the
        //    patch editor canvases. Engine state is already loaded; the
        //    patch_editor population reads descriptors from `session`.
        let mut max_id: u64 = 0;
        for inst_state in &project.instruments {
            let inst_id = inst_state.id;
            if inst_id.as_u64() > max_id {
                max_id = inst_id.as_u64();
            }

            let channel =
                MidiChannel::from_one_indexed(inst_state.channel).unwrap_or(MidiChannel::CH1);
            let mut ui_inst =
                InstrumentUiState::new(inst_id, &inst_state.name).with_channel(channel);

            patch_bridge::populate_editor_from_patch(
                &inst_state.patch,
                &mut ui_inst.patch_editor,
                &self.session,
                &mut self.handle,
                inst_id,
            );

            if let Some(cs) = inst_state.patch.settings.canvas_size {
                ui_inst
                    .patch_editor
                    .set_min_canvas_size(eframe::egui::Vec2::new(cs.width, cs.height));
            }

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
            ui_inst.category = synth_engine::InstrumentCategory::from_u8(inst_state.category);
            ui_inst.description = inst_state.description.clone();
            ui_inst.color = inst_state.color.clone();
            ui_inst.allocation_mode = inst_state.allocation_mode;
            ui_inst.stealing_strategy = inst_state.stealing_strategy;
            ui_inst.max_voices = inst_state.max_voices;
            ui_inst.velocity_amp_sensitivity = inst_state.velocity_amp_sensitivity;
            ui_inst.velocity_filter_sensitivity = inst_state.velocity_filter_sensitivity;
            ui_inst.sidechain_source_id = inst_state.sidechain_source_id.map(InstrumentId::new);
            ui_inst.patch_description = inst_state.patch.description.clone().unwrap_or_default();

            self.instruments.push(ui_inst);
        }

        // 3. Global UI mirrors.
        self.current_project_author = project
            .author
            .clone()
            .unwrap_or_else(|| self.settings.author.clone());
        // Mirror the loaded author into the MCP-shared slot so an
        // MCP-side save reads the loaded value, not a stale one.
        #[cfg(feature = "mcp")]
        if let Some(shared) = &self.mcp_shared {
            *shared.author.lock().unwrap_or_else(|e| e.into_inner()) = project.author.clone();
        }
        self.glide_time = project.global.glide_time;
        self.keyboard
            .set_octave_offset(project.global.octave_offset);

        // AWE description goes into the UI buffer and the MCP-shared slot.
        // Seeding both prevents the next frame's two-way sync from pulling
        // a stale empty value back over the just-loaded one.
        self.awe_ui.description = project.global.awe_description.clone().unwrap_or_default();
        #[cfg(feature = "mcp")]
        if let Some(shared) = &self.mcp_shared {
            shared
                .awe_description
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone_from(&self.awe_ui.description);
        }

        if let Some(awe) = &project.global.awe {
            self.awe_enabled = awe.enabled;
            self.awe_ui.restore_from(awe);
        } else {
            self.awe_enabled = false;
            self.awe_ui = crate::gui::awe_view::AweUiState::default();
        }

        // 4. Active-instrument bookkeeping. `apply_project` already sent
        //    `SetFocusedInstrument` to the engine; we only mirror it
        //    locally.
        self.next_instrument_id = max_id + 1;
        let target_id = InstrumentId::new(project.active_instrument_id);
        self.active_instrument_id = if self.instruments.iter().any(|i| i.id == target_id) {
            Some(target_id)
        } else {
            self.instruments.first().map(|i| i.id)
        };
    }

    /// Refresh UI mirrors after `project_apply::reset_to_new_project`
    /// has cleared the engine. Equivalent to a refresh against an empty
    /// `ProjectFile`.
    fn refresh_ui_after_reset(&mut self) {
        let empty = ProjectFile::new(
            Vec::new(),
            0,
            None,
            synth_sequencer::Song::new("Untitled"),
            GlobalProjectState::default(),
        );
        self.refresh_ui_from_project(&empty);
    }

    /// Load a project file, replacing all current state.
    fn load_project_data(&mut self, project: ProjectFile) {
        if let Err(e) = crate::project_apply::apply_project(
            &project,
            &self.session,
            &self.song,
            &self.sample_library,
        ) {
            eprintln!("apply_project failed during GUI load: {e}");
        }
        self.refresh_ui_from_project(&project);
    }

    /// Reset to a new empty project, clearing all instruments and song data.
    fn reset_to_new_project(&mut self) {
        if let Err(e) = crate::project_apply::reset_to_new_project(
            &self.session,
            &self.song,
            &self.sample_library,
        ) {
            eprintln!("reset_to_new_project failed: {e}");
        }
        self.sample_view_state.invalidate_peaks();
        self.refresh_ui_after_reset();
        self.current_project_path = None;
        self.current_patch_name = "Init".to_string();
        self.current_patch_path = None;
        self.dirty = false;
    }

    /// Load an AWE preset file, applying its state to the engine and UI.
    fn load_awe_preset_data(&mut self, preset: &crate::patch::AwePresetFile) {
        let state = &preset.state;
        self.awe_enabled = state.enabled;
        self.awe_ui.restore_from(state);
        self.awe_ui.selected_preset = None;
        self.awe_ui.current_preset_name = preset.name.clone();
        self.awe_ui.current_preset_description = preset.description.clone();
        self.awe_ui.current_preset_tags = preset.tags.clone();
        self.handle.send(EngineCommand::SetAweEnabled {
            enabled: state.enabled,
        });
        self.handle.send(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::RoomShape(state.room),
        });
        self.handle.send(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::Material(state.material),
        });
        self.handle.send(EngineCommand::SetAweState {
            snapshot: state.to_snapshot(),
        });
        self.handle.send(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::SpatialEnabled(state.spatial_enabled),
        });
        self.handle.send(EngineCommand::SetAweParameter {
            param: synth_awe::AweParam::NoteMapping(state.note_mapping),
        });
    }

    /// Load a project from a recent-projects path.
    fn load_recent_project(&mut self, path: PathBuf) {
        match project::load_file(&path) {
            Ok(LoadedFile::Project(proj)) => {
                self.load_project_data(*proj);
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
            Ok(LoadedFile::AwePreset(preset)) => {
                self.load_awe_preset_data(&preset);
                self.dialog_state
                    .set_status(format!("Loaded AWE preset: {}", preset.name));
            }
            Ok(LoadedFile::Bundle(bundle_path)) => match self.load_bundle_file(&bundle_path) {
                Ok(msg) => {
                    self.current_project_path = Some(path.clone());
                    self.dirty = false;
                    self.settings.add_recent_project(path.clone());
                    self.settings.save();
                    self.dialog_state.set_status(msg);
                }
                Err(e) => {
                    self.dialog_state
                        .set_status(format!("Bundle load error: {e}"));
                }
            },
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
            // Bundle format whenever samples exist — otherwise the WAV data
            // is silently dropped and the project loads back without audio.
            let has_samples = self.sample_library.read().is_ok_and(|lib| !lib.is_empty());
            let save_result = if has_samples {
                if let Ok(lib) = self.sample_library.read() {
                    crate::bundle::save_bundle(&proj, &lib, &path)
                } else {
                    proj.save(&path)
                }
            } else {
                proj.save(&path)
            };
            match save_result {
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
