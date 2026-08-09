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
use crate::gui::patch_editor::{GroupTemplateAction, PatchEditor, QuickAddRequest};
use crate::gui::shortcuts;
use crate::gui::theme::theme;
use crate::gui::widgets::{
    attention_badge, danger_button, dim_label, draw_oscilloscope, draw_stereo_meter, empty_state,
    expose_selected, menu_entry, stepper, submenu_button,
};
use crate::gui::{GuiBackend, GuiResult, SynthGuiConfig};
use crate::io::settings::AppSettings;
use crate::io::{GroupTemplateManager, MidiHandler, PatchManager};
use crate::patch::{Author, GroupCategory, Patch, categorized_patches};
use crate::project::{self, GlobalProjectState, LoadedFile, ProjectFile};
use synth_core::ModuleCategory;
use synth_core::{Seconds, Velocity};
use synth_engine::ModuleType as TypedModuleType;
use synth_engine::commands::PortId;
use synth_engine::instrument::{InstrumentId, MidiChannelSelection};
use synth_engine::{EngineCommand, EngineEvent, EngineHandle, ModuleId, SynthEngine};
use synth_sampler::SampleLibrary;

mod autosave_flow;
mod dialog_flow;
mod dialog_state;
mod engine_events;
mod project_flow;
mod undo_flow;

use dialog_state::{PendingAction, UnsavedChangesDialog};

#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceVersion(u64);

impl SourceVersion {
    const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct VersionTracker {
    current: Option<SourceVersion>,
}

impl VersionTracker {
    const fn at(version: u64) -> Self {
        Self {
            current: Some(SourceVersion::new(version)),
        }
    }

    fn observe(&mut self, version: u64) -> bool {
        let version = SourceVersion::new(version);
        if self.current == Some(version) {
            false
        } else {
            self.current = Some(version);
            true
        }
    }

    fn is_stale(&self, version: u64) -> bool {
        self.current != Some(SourceVersion::new(version))
    }

    fn invalidate(&mut self) {
        self.current = None;
    }
}

#[derive(Debug)]
struct VersionedCache<T> {
    version: VersionTracker,
    value: T,
}

impl<T: Default> Default for VersionedCache<T> {
    fn default() -> Self {
        Self {
            version: VersionTracker::default(),
            value: T::default(),
        }
    }
}

impl<T> VersionedCache<T> {
    fn is_stale(&self, version: u64) -> bool {
        self.version.is_stale(version)
    }

    fn replace(&mut self, version: u64, value: T) {
        let _changed = self.version.observe(version);
        self.value = value;
    }

    const fn value(&self) -> &T {
        &self.value
    }
}

#[cfg(feature = "mcp")]
#[derive(Debug)]
struct McpSyncState {
    project: VersionTracker,
    gui: VersionTracker,
    graph: VersionTracker,
}

#[cfg(feature = "mcp")]
impl Default for McpSyncState {
    fn default() -> Self {
        Self {
            project: VersionTracker::at(0),
            gui: VersionTracker::at(0),
            graph: VersionTracker::at(0),
        }
    }
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
        let startup_font = resolve_font(config.settings.font.as_deref()).to_owned();

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
                apply_fonts(&cc.egui_ctx, &startup_font);
                egui_extras::install_image_loaders(&cc.egui_ctx);
                startup_theme.apply();
                setup_custom_style(&cc.egui_ctx);
                Ok(Box::new(app))
            }),
        )
        .map_err(|e| format!("Egui error: {e}"))?;

        Ok(())
    }
}

// Generated by build.rs: `pub static BUNDLED_FONTS: &[(&str, &[u8])]`, one entry
// per font file in `assets/fonts/` (display name = the font's own family name).
include!(concat!(env!("OUT_DIR"), "/bundled_fonts.rs"));

/// Default monospace font family, used when no valid selection is stored.
pub const DEFAULT_FONT: &str = "Share Tech Mono";

/// How long taking a clean baseline waits for queued engine commands to be
/// applied, so it describes the state the engine actually reached.
///
/// Draining normally costs one audio block; this leaves room for many while
/// staying far below a perceptible stall on the UI thread. See
/// `SynthApp::capture_clean_baseline`.
const BASELINE_SETTLE_TIMEOUT_MS: u64 = 100;

/// Resolve a possibly-missing or unknown font selection to a valid bundled
/// family name. Falls back to [`DEFAULT_FONT`], then to the first bundled font.
#[must_use]
pub fn resolve_font(selected: Option<&str>) -> &'static str {
    if let Some(sel) = selected
        && let Some(font) = BUNDLED_FONTS.iter().find(|f| f.0 == sel)
    {
        return font.0;
    }
    BUNDLED_FONTS
        .iter()
        .find(|f| f.0 == DEFAULT_FONT)
        .or_else(|| BUNDLED_FONTS.first())
        .map_or(DEFAULT_FONT, |f| f.0)
}

/// Install the app font set with `selected` as the primary family. The whole UI
/// is monospace, so the same ordered list drives both the Proportional and
/// Monospace families: the selected font first, every other bundled font after
/// it as a glyph-coverage fallback, then the Remix Icon glyphs. Safe to call at
/// runtime to switch fonts live (`ctx.set_fonts` re-lays out the next frame).
pub fn apply_fonts(ctx: &egui::Context, selected: &str) {
    let mut fonts = egui::FontDefinitions::default();

    // Register every bundled font under its family name.
    for font in BUNDLED_FONTS {
        fonts.font_data.insert(
            font.0.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(font.1)),
        );
    }

    // Priority order: the selected font first, then the rest as fallbacks.
    let primary = resolve_font(Some(selected));
    let mut order: Vec<&str> = vec![primary];
    order.extend(BUNDLED_FONTS.iter().map(|f| f.0).filter(|n| *n != primary));

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let entry = fonts.families.entry(family).or_default();
        for (i, name) in order.iter().enumerate() {
            entry.insert(i, (*name).to_owned());
        }
    }

    // Register Remix Icon font (icon glyphs as fallback).
    egui_remixicon::add_to_fonts(&mut fonts);

    ctx.set_fonts(fonts);
}

/// Setup custom egui style for synth look.
/// Reads colors from the current theme, so call this after changing theme.
pub fn setup_custom_style(ctx: &egui::Context) {
    let t = theme();
    let colors = &t.colors;

    // Which kind of palette this preset is. Everything below hangs off it, so
    // derive it once from the palette itself rather than assuming "dark".
    let egui_theme = if colors.is_dark() {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };

    let mut style = (*ctx.global_style()).clone();

    // Reset the visuals to egui's base for *this* palette's kind before applying
    // our overrides. Cloning whatever was there before is not enough: on the
    // first call it is egui's default for the OS theme (so a dark preset on a
    // light-mode desktop inherited light shadows/strokes), and on a later call it
    // is the previous preset's visuals. Only the fields we do not override below
    // are affected — shadows, hyperlink colour, code background, and so on.
    style.visuals = match egui_theme {
        egui::Theme::Dark => egui::Visuals::dark(),
        egui::Theme::Light => egui::Visuals::light(),
    };

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

    // Disable egui's debug-only "a rect changed its widget id" heuristic
    // (`warn_if_rect_changes_id`, on by default in debug builds). It paints a red
    // overlay when a screen rect is occupied by a different widget id than the
    // previous frame. The patch editor canvas legitimately triggers this: each
    // module card's id includes the instrument id, so switching instruments (or
    // dragging a card onto where another card sat) leaves the *same* world rect
    // holding a *different*, still-correct id — a false positive. The ids are
    // genuinely stable per instrument; this only silences the misfiring dev
    // overlay (no effect in release, where the check is compiled out).
    // `Style::debug` is itself `#[cfg(debug_assertions)]` in egui, so the field
    // does not exist in release builds — gate the assignment to match.
    #[cfg(debug_assertions)]
    {
        style.debug.warn_if_rect_changes_id = false;
    }

    // Write the style into *both* theme slots, not via `set_global_style` — that
    // only writes the slot of the currently active theme, so pinning the
    // preference afterwards would swap in egui's untouched defaults.
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style);

    // Pin the preference to match the palette. Without this egui stays on
    // `ThemePreference::System`, which is what the windowing layer syncs the OS
    // window decorations to — a light preset would otherwise get dark chrome
    // (and vice versa) purely from the desktop's own setting.
    ctx.set_theme(egui_theme);
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

    // Instrument-list search filter (Rack view).
    instrument_search: String,

    // Project state
    current_project_path: Option<PathBuf>,

    // Global synth settings
    glide_time: synth_core::Seconds,

    // Instrument rack state
    instruments: Vec<InstrumentUiState>,
    active_instrument_id: Option<InstrumentId>,

    // Navigation state
    active_view: AppView,

    // DEBUG: frame counter after project load (0 = not tracking)

    // Sequencer state
    song: std::sync::Arc<synth_sequencer::SharedSong>,
    sequencer_view_state: crate::gui::sequencer::SequencerViewState,

    // Pattern view state (tab-local UI bits; selection shared via sequencer_view_state)
    pattern_view_state: crate::gui::pattern_view::PatternViewState,

    /// Note Grid view state (selected graph, canvas cameras, node positions).
    note_grid_view_state: crate::gui::note_grid_view::NoteGridViewState,

    /// Mod Grid view state (selected graph, canvas cameras, node positions).
    mod_grid_view_state: crate::gui::mod_grid_view::ModGridViewState,

    /// Tracks the `Song::mod_grid_generation` shipped to the engine. An
    /// invalidated tracker forces a rebuild on the first frame and after load.
    mod_grid_version: VersionTracker,

    // MCP shared state
    #[cfg(feature = "mcp")]
    mcp_shared: Option<std::sync::Arc<crate::mcp_shared::McpSharedState>>,

    /// Revision gates for MCP project, GUI mirror, and graph synchronization.
    /// Keeping them together makes the ownership of MCP→GUI state explicit.
    #[cfg(feature = "mcp")]
    mcp_sync: McpSyncState,

    // OSC shared state
    #[cfg(feature = "osc")]
    osc_shared: Option<synth_osc::OscSharedState>,

    // Persistent application settings
    settings: crate::io::settings::AppSettings,

    // Undo/redo manager
    undo_manager: crate::undo::UndoManager,

    /// Gesture tracking for continuous controls, so one fader drag becomes one
    /// undo entry instead of one per frame.
    drag_coalescer: crate::undo::DragCoalescer,

    /// Edit counters as of the last load or successful save — the "clean"
    /// baseline that [`Self::is_dirty`] compares the live counters against.
    saved_revision: crate::dirty::ProjectRevision,

    /// Counter for patch-canvas state that no shared subsystem owns (module
    /// positions, group boxes, canvas size, instrument colour). Bumped by
    /// [`Self::mark_dirty`]; everything else is observed rather than reported.
    ui_revision: synth_core::ContentRevision,

    /// Undo-history position as of the last load or save.
    ///
    /// Undoing back to this position means the project is back at the state
    /// that was saved, so it should read clean again — see [`Self::is_dirty`].
    saved_undo_position: crate::undo::HistoryPosition,

    /// The revision and undo-mutation count seen on the previous frame, for
    /// spotting a project change that bypassed the undo manager.
    last_observed: (crate::dirty::ProjectRevision, u64),

    /// Set when a mutation since the last save did *not* go through the undo
    /// manager, which makes the undo-depth shortcut in [`Self::is_dirty`]
    /// unsafe: undoing everything undoable would still leave that change in
    /// place, and calling the project clean would invite discarding it.
    untracked_mutation_since_save: bool,

    /// Unsaved changes confirmation dialog state.
    unsaved_dialog: UnsavedChangesDialog,

    /// Debounced recovery-snapshot scheduling.
    autosave: autosave_flow::AutosaveState,

    /// Work found from a previous session, awaiting the user's decision.
    recovery_prompt: Option<crate::recovery::RecoveryEntry>,

    /// Module clipboard for copy/paste.
    clipboard: crate::gui::clipboard::ModuleClipboard,

    /// Reusable buffers for master scope visualization (avoids per-frame allocation).
    scope_buf_l: Vec<f32>,
    scope_buf_r: Vec<f32>,

    /// Cached window title to avoid allocating every frame.
    last_title: String,

    /// Shared sample library.
    sample_library: std::sync::Arc<std::sync::RwLock<SampleLibrary>>,

    /// Sample view state.
    sample_view_state: crate::gui::sample_view::SampleViewState,
    /// Cached sampler→sample reference counts for the Sample view, paired with the
    /// `shared_graph` version they were computed at. Recomputed only when that
    /// version changes (any module add/remove or parameter edit bumps it) rather
    /// than cloning every module snapshot each repaint while the tab is open.
    sample_ref_counts: VersionedCache<std::collections::HashMap<u64, usize>>,
    /// `shared_graph` version the Mod Grid view's per-instrument module-target
    /// groups were last built at. The groups themselves live in
    /// `mod_grid_view_state.module_groups`; this only tracks when to rebuild them
    /// (they derive from module structure/types, so only a graph change matters),
    /// instead of re-deriving from live descriptors for every instrument each frame.
    mod_target_groups_version: VersionTracker,

    /// Mixer view state (rename buffer + smoothed meter levels).
    mixer_view_state: crate::gui::mixer_view::MixerViewState,

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

    /// Shared in-memory activity log, fed by the tracing capture layer.
    activity_log: crate::activity_log::ActivityLog,
    /// UI state for the Home activity-log console.
    activity_log_view: crate::gui::activity_log_view::ActivityLogViewState,
    /// What the most recent project load could not reconstruct. Drives the
    /// status-bar badge; emptied by the next load or by the user following the
    /// badge to the Activity console.
    load_diagnostics: Vec<crate::project_diagnostics::ProjectApplyDiagnostic>,
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

        let mut app = Self {
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
            instrument_search: String::new(),
            current_project_path: None,
            glide_time,
            instruments,
            active_instrument_id,
            active_view: AppView::default(),
            song,
            mod_grid_version: VersionTracker::default(),
            sequencer_view_state: crate::gui::sequencer::SequencerViewState::new(),
            pattern_view_state: crate::gui::pattern_view::PatternViewState::default(),
            note_grid_view_state: crate::gui::note_grid_view::NoteGridViewState::default(),
            mod_grid_view_state: crate::gui::mod_grid_view::ModGridViewState::default(),
            #[cfg(feature = "mcp")]
            mcp_shared: config.mcp_shared,
            #[cfg(feature = "mcp")]
            mcp_sync: McpSyncState::default(),
            #[cfg(feature = "osc")]
            osc_shared: config.osc_shared,
            settings,
            undo_manager: crate::undo::UndoManager::new(),
            drag_coalescer: crate::undo::DragCoalescer::default(),
            // Provisional; replaced by the `mark_saved()` below once the struct
            // exists, so the baseline reflects whatever startup already put into
            // the session, song and sample library.
            saved_revision: crate::dirty::ProjectRevision::default(),
            ui_revision: synth_core::ContentRevision::INITIAL,
            saved_undo_position: (0, 0),
            last_observed: (crate::dirty::ProjectRevision::default(), 0),
            untracked_mutation_since_save: false,
            unsaved_dialog: UnsavedChangesDialog::default(),
            autosave: autosave_flow::AutosaveState::new(),
            recovery_prompt: None,
            clipboard: crate::gui::clipboard::ModuleClipboard::new(),
            scope_buf_l: Vec::new(),
            scope_buf_r: Vec::new(),
            last_title: String::new(),
            sample_library: config.sample_library,
            sample_view_state: crate::gui::sample_view::SampleViewState::new(),
            sample_ref_counts: VersionedCache::default(),
            mod_target_groups_version: VersionTracker::default(),
            mixer_view_state: crate::gui::mixer_view::MixerViewState::default(),
            audio_input: crate::audio::input::AudioInputManager::new(),
            analyze_window: crate::gui::analyze::AnalyzeWindow::new(),
            instrument_edit_target: None,
            pending_instrument_delete: None,
            project_edit_open: false,
            current_project_author: project_author,
            activity_log: config.activity_log,
            activity_log_view: crate::gui::activity_log_view::ActivityLogViewState::default(),
            load_diagnostics: Vec::new(),
        };

        // Whatever engine and song state startup produced is the clean baseline
        // — a freshly launched app has no unsaved changes. Reading the counters
        // here rather than assuming they are zero keeps that true when the app
        // is launched with a project on the command line, or when engine setup
        // has already published commands to the shared graph.
        app.capture_clean_baseline();
        // Ask about work from a previous session before the user starts editing
        // this one — the answer decides which document they are working in.
        app.check_for_recoverable_work();
        app
    }

    /// Record a change to patch-canvas state that no shared subsystem owns.
    ///
    /// Only needed for GUI-only data — module positions, group boxes, canvas
    /// size, instrument colour. Edits to the song, the engine graph or the
    /// sample library are picked up by [`Self::current_revision`] without the
    /// editor doing anything.
    fn mark_dirty(&mut self) {
        self.ui_revision = self.ui_revision.next();
    }

    /// The live edit counters of every subsystem holding part of the project.
    fn current_revision(&self) -> crate::dirty::ProjectRevision {
        crate::dirty::ProjectRevision {
            song: self.song.revision(),
            graph: synth_core::ContentRevision::new(self.session.state().shared_graph.version()),
            samples: self
                .sample_library
                .read()
                .map_or(synth_core::ContentRevision::INITIAL, |lib| lib.revision()),
            ui: self.ui_revision,
            // Summed rather than concatenated so instrument order does not
            // matter. Each term only ever changes when that instrument's
            // canvas changes, so a sum cannot silently cancel out.
            layout: self.instruments.iter().fold(0u64, |acc, inst| {
                acc.wrapping_add(inst.patch_editor.layout_fingerprint())
            }),
            global: crate::dirty::global_fingerprint(
                self.session.state(),
                self.keyboard.octave_offset(),
                self.glide_time,
            ),
            effect_order: crate::dirty::effect_order_fingerprint(self.session.state()),
        }
    }

    /// Whether the project has changes that are not in the file on disk.
    ///
    /// Two ways to be clean. The obvious one is that nothing has happened since
    /// the baseline. The second is that the user undid their way back to it:
    /// the counters are monotonic and cannot recognise a return to a previous
    /// point, so the undo stack answers that instead — standing at the same
    /// history position it did when the project was saved means the same edits
    /// have been undone as were made.
    ///
    /// The position must identify the *route*, not just the depth: undoing one
    /// edit and then making a different one comes back to the saved depth while
    /// leaving the project two edits away from the file, and calling that clean
    /// would drop the `*`, skip the quit prompt and stop autosave. See
    /// [`UndoManager::position`](crate::undo::UndoManager::position).
    ///
    /// The shortcut is only sound while *every* mutation is undoable. If
    /// something changed the project without going through the undo manager,
    /// undoing everything undoable would not restore the saved state, so
    /// `untracked_mutation_since_save` disables the shortcut and the project
    /// stays dirty. Erring toward dirty costs a redundant save prompt; erring
    /// the other way would discard work.
    ///
    /// The undo stack is consulted *before* the counters, not after. The two
    /// are independent observers and either one seeing a change is enough; the
    /// old order made the counters a gate that returned clean before the stack
    /// was ever read, so an edit the undo manager had recorded still reported
    /// clean if it happened to touch no counter — which is exactly what adding
    /// a master effect did.
    fn is_dirty(&self) -> bool {
        // Standing anywhere other than the saved position means recorded work
        // is not in the file, whatever the counters say.
        if self.undo_manager.position() != self.saved_undo_position {
            return true;
        }
        // Back at the save point on the undo stack. That is proof of clean only
        // while every mutation is undoable — otherwise fall through and let the
        // counters answer.
        if !self.untracked_mutation_since_save {
            return false;
        }
        self.current_revision().differs_from(self.saved_revision)
    }

    /// Notice a project change that did not pass through the undo manager.
    ///
    /// Called once per frame. A mutation the manager saw bumps its counter — a
    /// push (including one merged into the previous entry, which moves the
    /// project without changing the stack depth) or an undo/redo. So a revision
    /// that moved while that counter stood still is a mutation nothing recorded.
    fn observe_untracked_mutation(&mut self) {
        let observed = (self.current_revision(), self.undo_manager.mutation_count());
        let (previous_revision, previous_mutations) = self.last_observed;
        if observed.0 != previous_revision && observed.1 == previous_mutations {
            self.untracked_mutation_since_save = true;
        }
        self.last_observed = observed;
    }

    /// Take the current state as the clean baseline.
    ///
    /// Called after a successful save, after loading or resetting a project,
    /// and when the user discards changes — every point at which the current
    /// state becomes the reference. Note this reads the counters *now*, so an
    /// edit made while the save was in flight correctly stays part of the next
    /// dirty comparison.
    ///
    /// Retiring the recovery snapshot belongs here rather than at each call
    /// site: "this state is the baseline" and "there is no unsaved work to
    /// protect" are the same statement, and separating them is how a stale
    /// snapshot ends up being offered for work the user already saved.
    fn mark_saved(&mut self) {
        self.capture_clean_baseline();
        self.retire_recovery_snapshot();
    }

    /// Take the current state as the clean baseline *without* retiring the
    /// recovery snapshot.
    ///
    /// Only startup wants this. At launch there is nothing of this session's to
    /// protect, but there may well be a snapshot from the *previous* session
    /// waiting to be offered — and retiring it here would delete the crashed
    /// session's work before anyone was asked about it.
    fn capture_clean_baseline(&mut self) {
        // Let queued engine commands land first. Resetting or loading a project
        // sends the master volume, glide and effect chains to the audio thread,
        // which applies them a block or two later — so a baseline taken right
        // now describes a state the engine has not reached yet, and the next
        // frame reads the difference as an edit. That would mark a freshly
        // opened project dirty, and worse, `observe_untracked_mutation` would
        // see a revision move with no undo entry behind it and latch
        // `untracked_mutation_since_save` for the rest of the session, killing
        // the undo-back-to-clean shortcut. `project_apply` waits here for the
        // same reason; this bound is much tighter because it runs on the UI
        // thread, and timing out merely risks a spurious `*` — the safe way to
        // be wrong.
        let _ = self
            .session
            .wait_for_pending_commands(BASELINE_SETTLE_TIMEOUT_MS);
        self.saved_revision = self.current_revision();
        self.saved_undo_position = self.undo_manager.position();
        self.untracked_mutation_since_save = false;
        self.last_observed = (self.saved_revision, self.undo_manager.mutation_count());
    }

    /// Dispatch a welcome-screen action to the appropriate existing flow.
    fn handle_welcome_action(&mut self, action: crate::gui::welcome_view::WelcomeAction) {
        use crate::gui::welcome_view::WelcomeAction;
        match action {
            WelcomeAction::NewInstrument => {
                self.add_new_instrument();
                // Reveal the freshly created instrument in the rack.
                self.active_view = AppView::Rack;
            }
            WelcomeAction::OpenProject => {
                if self.is_dirty() {
                    self.unsaved_dialog.pending_action = Some(PendingAction::OpenProject);
                    self.unsaved_dialog.open = true;
                } else {
                    let initial_dir = self.resolve_project_dir();
                    self.dialog_state.open_file_dialog(
                        FileDialogMode::OpenProject,
                        None,
                        initial_dir.as_deref(),
                    );
                }
            }
            WelcomeAction::OpenPatch => {
                let initial_dir = self.resolve_open_dir();
                self.dialog_state.open_file_dialog(
                    FileDialogMode::OpenPatch,
                    None,
                    initial_dir.as_deref(),
                );
            }
            WelcomeAction::LoadBuiltinPatch => {
                self.dialog_state.show_load_patch = true;
            }
            WelcomeAction::ImportSample => {
                self.dialog_state
                    .open_file_dialog(FileDialogMode::ImportSample, None, None);
                self.active_view = AppView::Sample;
            }
            WelcomeAction::OpenRecent(path) => {
                if self.is_dirty() {
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
        let Some(name) = self
            .instruments
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.name.clone())
        else {
            return;
        };
        if let Err(e) = self.session.remove_instrument(id) {
            tracing::warn!(target: "pertylizer::instrument", error = %e, "failed to remove instrument '{name}'");
        } else {
            tracing::info!(target: "pertylizer::instrument", "deleted instrument '{name}'");
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
        let new_channel = MidiChannelSelection::from_one_indexed(instrument_num as u8)
            .unwrap_or(MidiChannelSelection::CH1);

        let new_id = self.session.add_instrument(&new_name).ok()?;
        tracing::info!(target: "pertylizer::instrument", "created instrument '{new_name}'");

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

        // Rebuild + ship the Mod Grid runtime when its pooled data changed.
        // Building allocates the control-rate DSP, so it happens here on the GUI
        // thread; the engine only swaps the pre-built runtime in. Cheap when
        // unchanged (a single `u64` compare under a short read lock).
        self.sync_mod_grid();

        // Poll for engine events (note feedback, etc.)
        // This ensures the GUI keyboard reflects what the engine is actually playing,
        // regardless of whether notes came from MIDI, sequencer, or GUI.
        self.poll_engine_events();

        // Poll MCP pending auto-layout.
        //
        // Non-destructive load: a request stays set until the Rack view
        // actually applies it (see apply site below). This means a
        // request that arrives while the user is in
        // Sequencer / Sample survives until they switch back to Rack.
        #[cfg(feature = "mcp")]
        let mcp_auto_layout = self.mcp_shared.as_ref().is_some_and(|shared| {
            shared
                .pending_auto_layout
                .load(std::sync::atomic::Ordering::Relaxed)
        });
        #[cfg(not(feature = "mcp"))]
        let mcp_auto_layout = false;

        // Drain MCP project I/O and GUI mirror payloads, then
        // reconcile module add/removes the MCP side performed.
        #[cfg(feature = "mcp")]
        {
            self.drain_mcp_state();
            self.reconcile_with_session();
        }

        // Compare against last frame before anything this frame mutates, so a
        // change is attributed to the frame that made it.
        self.observe_untracked_mutation();

        // Commit a background snapshot that finished since last frame, then
        // consider starting another. Debounced internally, so this is a cheap
        // comparison on all but one frame in ~1800.
        self.poll_autosave();
        self.tick_autosave();

        // ── Input routing ──
        //
        // The gate is read once and shared, so every consumer agrees on whether
        // a text field or a modal owns the keyboard this frame. Application
        // shortcuts are dispatched first and *consume* their keys, so a view
        // binding the same combination never sees it.
        let input_gate = shortcuts::InputGate::new(ctx, self.modal_is_open());
        self.handle_app_shortcuts(ctx, input_gate);

        // Handle keyboard input
        self.process_keyboard_input(ctx, input_gate);

        // ── Copy/Paste/Duplicate keyboard shortcuts ──
        self.handle_clipboard_shortcuts(ctx, input_gate);

        // ── Analyze window shortcut (Ctrl/Cmd + Shift + A) ──
        self.handle_analyze_shortcut(ctx, input_gate);

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
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                self.menu_file(ui, ctx);

                self.menu_edit(ui);
                self.menu_help(ui);

                // View selector — segmented control (right after Help menu)
                ui.separator();
                self.render_view_selector(ui);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new((
                            RichText::new(ri::ALARM_WARNING_FILL).color(theme().colors.accent_red),
                            RichText::new("PANIC").color(theme().colors.accent_red),
                        )))
                        .clicked()
                    {
                        // True panic = hard kill, not just release. AllNotesOff
                        // only triggers the release phase, so long releases,
                        // stuck voices, and reverb/delay tails keep ringing —
                        // the reason panic "didn't always work". Follow it with
                        // ResetDsp to zero all voices + effect tails instantly.
                        // AllNotesOff stays first for its AllNotesReleased event
                        // (clears on-screen keyboard state) + param-override reset.
                        self.handle.send(EngineCommand::AllNotesOff);
                        self.handle.send(EngineCommand::ResetDsp);
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
                    self.render_midi_status(ui);
                    ui.separator();
                    // MCP connection status indicator
                    #[cfg(feature = "mcp")]
                    self.render_mcp_status(ui);
                    // OSC telemetry status indicator
                    #[cfg(feature = "osc")]
                    self.render_osc_status(ui);
                    self.render_instrument_selector(ui);

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
                    let dirty_marker = if self.is_dirty() { " *" } else { "" };
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
        self.render_status_bar(ui);

        self.render_keyboard_panel(ui);

        // Main content - CentralPanel rendered LAST (normal egui order)
        // Module Areas are clipped to visible_rect in patch_editor.rs
        {
            match self.active_view {
                AppView::Home => {
                    // Right console: a live activity log (MCP calls, project /
                    // instrument ops, status messages). Rendered before the
                    // CentralPanel per the panel-order rule. Self-contained, so
                    // it can later be promoted to its own tab or bottom console.
                    let mut log_action = None;
                    egui::Panel::right("activity_log")
                        .resizable(true)
                        .default_size(380.0)
                        .min_size(240.0)
                        .show(ui, |ui| {
                            log_action = crate::gui::activity_log_view::show(
                                ui,
                                &self.activity_log,
                                &mut self.activity_log_view,
                            );
                        });
                    if log_action == Some(crate::gui::activity_log_view::ActivityLogAction::Export)
                    {
                        let default_name = "activity-log.txt";
                        let initial_dir = self.resolve_save_dir();
                        self.dialog_state.open_file_dialog(
                            FileDialogMode::ExportActivityLog,
                            Some(default_name),
                            initial_dir.as_deref(),
                        );
                    }

                    // Welcome / landing view — always available regardless of
                    // whether an instrument exists. Lets the user create an
                    // instrument or open a project/patch without hunting for
                    // the "+ New Instrument" button.
                    let recent = self.settings.recent_projects.clone();
                    let mut welcome_action = None;
                    egui::CentralPanel::default().show(ui, |ui| {
                        let t = theme();
                        welcome_action = crate::gui::welcome_view::show(ui, &t, &recent);
                    });
                    if let Some(action) = welcome_action {
                        self.handle_welcome_action(action);
                    }
                }
                AppView::Rack => {
                    // Rack view: always shows the rack. The instrument list panel
                    // is always present so the user can create/pick an instrument;
                    // the central area shows the active patch editor, or a hint
                    // when no instrument exists yet.
                    // Left side panel: instrument list (mirrors sample_view layout).
                    // Acts as a primary picker; the dropdown above the keyboard
                    // stays as a backup.
                    self.render_instruments_panel(ui, self.active_instrument_id);

                    if let Some(active_id) = self.active_instrument_id {
                        self.render_rack_central(ui, active_id, mcp_auto_layout);
                    } else {
                        egui::CentralPanel::default().show(ui, |ui| {
                            let t = theme();
                            ui.vertical_centered(|ui| {
                                ui.add_space(48.0);
                                ui.label(
                                    egui::RichText::new(
                                        "No instrument yet — create one to start patching.",
                                    )
                                    .size(t.fonts.size_normal)
                                    .color(t.colors.text_secondary),
                                );
                            });
                        });
                    }
                }
                AppView::NoteGraph => {
                    crate::gui::note_grid_view::draw_note_grid_view(
                        ui,
                        &self.song,
                        &mut self.note_grid_view_state,
                        &mut self.undo_manager,
                    );
                }
                AppView::ModGrid => {
                    let instruments: Vec<(synth_sequencer::InstrumentId, String)> = self
                        .instruments
                        .iter()
                        .map(|i| (i.id, i.name.clone()))
                        .collect();
                    // Per-instrument automatable module targets, from the live
                    // descriptors (shared enumeration → matches MCP + the lane
                    // picker). These derive from module structure/types, so rebuild
                    // only when the shared graph changed (a registry lock +
                    // descriptor clone per instrument each frame is wasteful on large
                    // projects); otherwise `None` keeps the view's existing groups.
                    let graph_version = self.session.state().shared_graph.version();
                    let module_groups: Option<
                        std::collections::HashMap<
                            synth_sequencer::InstrumentId,
                            Vec<crate::module_targets::ModuleTargetGroup>,
                        >,
                    > = if self.mod_target_groups_version.observe(graph_version) {
                        Some(
                            instruments
                                .iter()
                                .map(|(seq_id, _)| {
                                    let modules = self.session.all_modules_for_instrument(*seq_id);
                                    (
                                        *seq_id,
                                        crate::module_targets::module_target_groups(&modules),
                                    )
                                })
                                .collect(),
                        )
                    } else {
                        None
                    };
                    let cpu_profile = synth_engine::EngineHandle::cpu_profiling_enabled()
                        .then(|| self.handle.cpu_breakdown());
                    crate::gui::mod_grid_view::draw_mod_grid_view(
                        ui,
                        &self.song,
                        &mut self.mod_grid_view_state,
                        &mut self.undo_manager,
                        &instruments,
                        module_groups,
                        cpu_profile,
                    );
                }
                AppView::Pattern => {
                    crate::gui::pattern_view::draw_pattern_view(
                        ui,
                        &mut self.handle,
                        &self.song,
                        &mut self.sequencer_view_state,
                        &mut self.pattern_view_state,
                        &self.instruments,
                        &mut self.undo_manager,
                    );
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
                AppView::Mixer => {
                    let mut mixer_undo = crate::undo::MixerUndo {
                        undo: &mut self.undo_manager,
                        coalescer: &mut self.drag_coalescer,
                    };
                    let action = crate::gui::mixer_view::draw_mixer_view(
                        ui,
                        &mut self.handle,
                        &self.song,
                        &self.instruments,
                        &mut self.mixer_view_state,
                        &mut mixer_undo,
                    );
                    if let Some(crate::gui::mixer_view::MixerViewAction::EditChannelFx(seq_id)) =
                        action
                        && let Some(inst) = self
                            .instruments
                            .iter()
                            .find(|i| i.id.as_u64() == seq_id.as_u64())
                    {
                        // Channel inserts live on the instrument — open its
                        // patch editor in the Rack.
                        self.active_instrument_id = Some(inst.id);
                        self.active_view = AppView::Rack;
                    }
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

                    // How many sampler modules reference each sample (by raw id) —
                    // drives both dimming unused samples in the list and blocking
                    // deletion of an in-use sample. Recompute only when the shared
                    // graph changed since last frame; otherwise reuse the cache, so
                    // an open Sample tab doesn't clone every module snapshot ~60×/s.
                    let graph_version = self.session.state().shared_graph.version();
                    if self.sample_ref_counts.is_stale(graph_version) {
                        let mut counts: std::collections::HashMap<u64, usize> =
                            std::collections::HashMap::new();
                        for id in self
                            .session
                            .state()
                            .shared_graph
                            .get_all_modules()
                            .iter()
                            .flat_map(|m| &m.parameters)
                            .filter_map(|p| match p {
                                synth_core::params::Param::Sampler(
                                    synth_core::params::SamplerParam::SampleSelect(id),
                                ) => Some(id.as_u64()),
                                _ => None,
                            })
                        {
                            *counts.entry(id).or_insert(0) += 1;
                        }
                        self.sample_ref_counts.replace(graph_version, counts);
                    }
                    let mut sample_undo = crate::undo::SampleUndo {
                        undo: &mut self.undo_manager,
                        coalescer: &mut self.drag_coalescer,
                    };
                    let action = crate::gui::sample_view::draw_sample_view(
                        ui,
                        &self.sample_library,
                        &mut self.sample_view_state,
                        &mut self.audio_input,
                        self.sample_ref_counts.value(),
                        &mut sample_undo,
                    );
                    match action {
                        crate::gui::sample_view::SampleViewAction::None => {}
                        crate::gui::sample_view::SampleViewAction::ImportWav => {
                            self.dialog_state.open_file_dialog(
                                FileDialogMode::ImportSample,
                                None,
                                None,
                            );
                        }
                        crate::gui::sample_view::SampleViewAction::ExportWav { name } => {
                            let wav_name = format!("{name}.wav");
                            self.dialog_state.open_file_dialog(
                                FileDialogMode::ExportSample,
                                Some(&wav_name),
                                None,
                            );
                        }
                        crate::gui::sample_view::SampleViewAction::StartMonitoring => {
                            if let Some(host) = &self.host {
                                let device =
                                    self.sample_view_state.selected_input_device.as_deref();
                                let config = synth_core::StreamConfig {
                                    sample_rate: synth_core::audio::DeviceSampleRate::new(
                                        self.handle.state.sample_rate.load(),
                                    ),
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
                                        if let Some((consumer, sample_rate)) =
                                            self.audio_input.take_engine_consumer()
                                        {
                                            self.handle.send(
                                                EngineCommand::SetAudioInputConsumer {
                                                    consumer,
                                                    sample_rate,
                                                },
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
                            Self::commit_recording_as_sample(
                                &mut self.audio_input,
                                &self.sample_library,
                                &mut self.sample_view_state,
                                &mut self.dialog_state,
                                &mut self.undo_manager,
                                &mut self.drag_coalescer,
                            );
                        }
                    }
                }
            }
        } // end view block

        // Cross-view jump: the Note FX panel's "edit graph" affordance loads
        // the graph in the Note Grid view.
        if let Some(graph_id) = self.sequencer_view_state.jump_to_note_graph.take() {
            self.note_grid_view_state.selected = Some(graph_id);
            self.active_view = AppView::NoteGraph;
        }
        // Cross-view jump: an automation lane's provenance chip loads the writing
        // mod graph in the Mod Grid view.
        if let Some(graph_id) = self.sequencer_view_state.jump_to_mod_graph.take() {
            self.mod_grid_view_state.selected = Some(graph_id);
            self.active_view = AppView::ModGrid;
        }

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
                if self.is_dirty() { " *" } else { "" },
            );
            if title != self.last_title {
                self.last_title = title.clone();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
            }
        }

        // Intercept close request when there are unsaved changes
        if ctx.input(|i| i.viewport().close_requested()) && self.is_dirty() {
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
        let Some((next_id, descriptor)) = Self::create_palette_module(
            session,
            handle,
            instrument_id,
            editor,
            request.module_type,
            request.position,
        ) else {
            return;
        };

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

    /// Create `module_type` for `instrument_id` and place it in the editor at
    /// `position`, returning its id and descriptor.
    ///
    /// The single GUI-side add path, shared by quick-add (drag off a port) and
    /// context-add (right-click menu). Most types just go through
    /// [`SynthSession::add_module`], but two need GUI-owned setup that the
    /// session can't do: a **visualizer** is built here and registered with an
    /// `AddVisualizer` command (`add_module` rejects it with
    /// `VisualizerRequiresGui`), and the **signal monitor** needs its
    /// `VisualizationBuffer` installed as a sink before the instance is sent.
    /// Keeping that in one place is why quick-add now handles them too.
    fn create_palette_module(
        session: &crate::session::SynthSession,
        handle: &mut EngineHandle,
        instrument_id: InstrumentId,
        editor: &mut PatchEditor,
        module_type: TypedModuleType,
        position: Pos2,
    ) -> Option<(ModuleId, synth_core::ModuleDescriptor)> {
        patch_bridge::create_editor_module(
            session,
            handle,
            instrument_id,
            editor,
            module_type,
            position,
        )
        .ok()
    }

    /// Handle a context menu add: create a module and place it at the given position.
    /// If `inline_cable` is `Some`, the old cable is removed and the new module is
    /// wired inline: `from → new_module(first_input) → new_module(first_output) → to`.
    fn handle_context_add(
        session: &crate::session::SynthSession,
        handle: &mut EngineHandle,
        instrument_id: InstrumentId,
        editor: &mut PatchEditor,
        module_type: TypedModuleType,
        position: Pos2,
        inline_cable: Option<synth_engine::graph::Connection>,
    ) {
        let Some((new_id, descriptor)) = Self::create_palette_module(
            session,
            handle,
            instrument_id,
            editor,
            module_type,
            position,
        ) else {
            return;
        };

        let Some(cable) = inline_cable else {
            return;
        };

        // Splice the new module into the cable. Only when it can actually sit
        // there (audio in AND out): a module without an audio input — the Script
        // module, any generator — can't be inserted in-line, so leave the cable
        // intact rather than silently deleting it.
        let first_input = descriptor.ports.iter().find(|p| {
            p.direction == synth_core::PortDirection::Input
                && p.port_type == synth_core::PortType::Audio
        });
        let first_output = descriptor.ports.iter().find(|p| {
            p.direction == synth_core::PortDirection::Output
                && p.port_type == synth_core::PortType::Audio
        });
        let (Some(inp), Some(outp)) = (first_input, first_output) else {
            return;
        };

        // Replace the cable: drop it from the engine and the editor, then wire
        // from→new(in) and new(out)→to.
        handle.send(EngineCommand::Disconnect {
            instrument_id: Some(instrument_id),
            from: PortId::new(cable.from_module, cable.from_port),
            to: PortId::new(cable.to_module, cable.to_port),
        });
        editor.remove_connection(&cable);

        let conn_in = synth_engine::graph::Connection::new(
            cable.from_module,
            cable.from_port,
            new_id,
            inp.name,
        );
        let conn_out =
            synth_engine::graph::Connection::new(new_id, outp.name, cable.to_module, cable.to_port);
        for c in [conn_in, conn_out] {
            editor.add_connection(c);
            handle.send(EngineCommand::Connect {
                instrument_id: Some(instrument_id),
                from: PortId::new(c.from_module, c.from_port),
                to: PortId::new(c.to_module, c.to_port),
            });
        }
    }

    /// Finalize the current audio-input recording into a new library sample.
    /// Shared by the Rack audio-input view and the Sample view.
    ///
    /// Takes the individual fields rather than `&mut self` so it can be called
    /// from the Rack view while a `patch_editor` borrow of `self.instruments`
    /// is still live (disjoint-field borrows).
    fn commit_recording_as_sample(
        audio_input: &mut crate::audio::input::AudioInputManager,
        sample_library: &std::sync::RwLock<SampleLibrary>,
        sample_view_state: &mut crate::gui::sample_view::SampleViewState,
        dialog_state: &mut DialogState,
        undo_manager: &mut crate::undo::UndoManager,
        drag_coalescer: &mut crate::undo::DragCoalescer,
    ) {
        if let Some(data) = audio_input.stop_recording() {
            let channels = audio_input.channels();
            let sample_rate = audio_input.sample_rate();
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
                        frame_count as f64 / f64::from(sample_rate.as_u32())
                    ),
                    description: String::new(),
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
            let recorded = sample_library.write().ok().map(|mut lib| lib.add(sample));
            if let Some(id) = recorded {
                sample_view_state.selected_sample = Some(id);
                sample_view_state.invalidate_peaks();
                crate::undo::SampleUndo {
                    undo: undo_manager,
                    coalescer: drag_coalescer,
                }
                .record_import(sample_library, id);
            }
            dialog_state.set_status("Recording saved");
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
        let active_channel = MidiChannelSelection::CH1;

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

            // Vertical stereo level meter (no dB scale / L/R labels in the
            // cramped keyboard corner — bars fill the full width).
            draw_stereo_meter(
                ui,
                peak_l,
                peak_r,
                rms_l,
                rms_r,
                egui::vec2(meter_width, keys_height),
                false,
            );
        });

        // Return buffers to self so their capacity is reused next frame
        samples_l.clear();
        samples_r.clear();
        self.scope_buf_l = samples_l;
        self.scope_buf_r = samples_r;
    }

    fn process_keyboard_input(&mut self, ctx: &egui::Context, gate: shortcuts::InputGate) {
        // Always use CH1 for keyboard input - focused_instrument handles routing
        let active_channel = MidiChannelSelection::CH1;

        // Anything that stops keystrokes reaching the piano — a focused text
        // field, an open modal, the window losing focus — has to release what
        // is already held, or the matching key-up never arrives and the note
        // sustains forever.
        let has_window_focus = ctx.input(|i| i.focused);
        if !gate.allows_piano_keys() || !has_window_focus {
            crate::gui::input::release_all_keyboard_notes(
                &mut self.handle,
                &mut self.pressed_keys,
                active_channel,
            );
            return;
        }

        handle_keyboard_input(
            ctx,
            &mut self.handle,
            &mut self.keyboard,
            &mut self.pressed_keys,
            active_channel,
        );
    }

    /// Push every instrument property to the engine, unconditionally.
    ///
    /// The normal editing path sends one command per property the user actually
    /// touched. Undo has no such flags — it restores a whole snapshot — so this
    /// sends the lot. Redundant commands are harmless and this runs once per
    /// undo, not per frame.
    fn sync_instrument_settings_to_engine(&mut self, instrument_id: InstrumentId) {
        let Some(inst) = self.instruments.iter().find(|i| i.id == instrument_id) else {
            return;
        };
        // The whole snapshot, so this cannot drift from what
        // `InstrumentUiState::apply_settings` writes: every field restored on
        // the GUI side is pushed to the engine here. Leaving one out is how the
        // display and the sound end up disagreeing after an undo.
        let settings = inst.settings();

        use synth_engine::InstrumentParam;
        for param in [
            InstrumentParam::Volume(settings.volume),
            InstrumentParam::Pan(settings.pan),
            InstrumentParam::Solo(settings.solo),
            InstrumentParam::KeyRange(settings.key_range),
            InstrumentParam::Transpose(settings.transpose),
            InstrumentParam::OversamplingFactor(settings.oversampling),
            InstrumentParam::AllocationMode(settings.allocation_mode),
            InstrumentParam::StealingStrategy(settings.stealing_strategy),
            InstrumentParam::UnisonDetune(settings.unison_detune),
            InstrumentParam::UnisonSpread(settings.unison_spread),
            InstrumentParam::MaxVoices(settings.max_voices),
            InstrumentParam::VelocityAmpSensitivity(settings.velocity_amp_sensitivity),
            InstrumentParam::VelocityFilterSensitivity(settings.velocity_filter_sensitivity),
        ] {
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id,
                param,
            });
        }
        self.handle.send(EngineCommand::SetInstrumentMidiChannel {
            instrument_id,
            channel: settings.channel,
        });
        self.handle.send(EngineCommand::RenameInstrument {
            instrument_id,
            name: settings.name.clone(),
        });
        // These go through the session rather than a raw command because they
        // also update its control-side mirror. Mute comes after the volume
        // above, matching the order project loading uses — the UI's soft mute
        // and the engine's enable flag are separate switches.
        for result in [
            self.session
                .set_instrument_category(instrument_id, settings.category),
            self.session
                .set_instrument_color(instrument_id, settings.color.as_deref()),
            self.session
                .set_sidechain_source(instrument_id, settings.sidechain_source_id),
            self.session
                .set_instrument_mute(instrument_id, settings.muted),
            self.session
                .set_instrument_description(instrument_id, &settings.description),
            self.session.set_patch_description(
                instrument_id,
                // Empty means "no patch description", as the editor's own
                // send path treats it.
                Some(settings.patch_description.as_str()).filter(|d| !d.is_empty()),
            ),
        ] {
            if let Err(e) = result {
                tracing::warn!(
                    target: "pertylizer::undo",
                    instrument = ?instrument_id,
                    error = %e,
                    "could not restore an instrument property",
                );
            }
        }
    }

    /// Whether a dialog currently owns keyboard input.
    ///
    /// These are ordinary egui windows rather than true modals, so egui cannot
    /// answer this for us — but while one is up, keystrokes must not reach the
    /// document behind it. Collected into [`ModalDialogs`] so the disjunction is
    /// exhaustive by construction; a new dialog is caught by the compiler the
    /// moment it earns a field there.
    fn modal_is_open(&self) -> bool {
        crate::gui::dialogs::ModalDialogs {
            recovery_prompt: self.recovery_prompt.is_some(),
            file_dialog: self.dialog_state.is_file_dialog_open(),
            unsaved_changes: self.unsaved_dialog.open,
            settings: self.dialog_state.show_settings,
            about: self.dialog_state.show_about,
            load_patch: self.dialog_state.show_load_patch,
            group_templates: self.dialog_state.show_group_templates,
            save_group_template: self.dialog_state.show_save_group_template,
            export_wav: self.dialog_state.show_export_wav,
            instrument_delete: self.pending_instrument_delete.is_some(),
        }
        .any_open()
    }

    /// Run the application-wide shortcuts.
    ///
    /// Dispatched before any view input so a consumed key cannot also reach a
    /// view that binds it.
    fn handle_app_shortcuts(&mut self, ctx: &egui::Context, gate: shortcuts::InputGate) {
        use shortcuts::AppShortcut;

        for shortcut in shortcuts::pressed(ctx, gate) {
            match shortcut {
                AppShortcut::Save => {
                    self.save_current_project();
                }
                AppShortcut::SaveAs => self.open_save_project_as_dialog(),
                AppShortcut::New => self.request_new_project(),
                AppShortcut::Open => self.request_open_project(),
                AppShortcut::Undo => self.execute_undo(),
                AppShortcut::Redo => self.execute_redo(),
                AppShortcut::TogglePlayback => self.toggle_playback(),
            }
        }
    }

    /// Start playback, or pause it if already running.
    ///
    /// Lives here rather than in the transport widget so the spacebar works
    /// from every view, not just the sequencer editors that draw a transport.
    fn toggle_playback(&mut self) {
        if self.handle.state.transport.is_playing() {
            self.handle.send(EngineCommand::Pause);
        } else {
            self.handle.send(EngineCommand::Play);
            // Starting playback re-arms playhead following, matching what the
            // sequencer's own Play button does — the spacebar should not
            // behave differently just because it now lives here.
            self.sequencer_view_state.follow_playhead_on_play();
        }
    }

    /// Handle Ctrl/Cmd+Shift+A — toggle the analyze window.
    ///
    /// Not in the [`shortcuts`] table because it toggles a tool window rather
    /// than acting on the document, but it takes the same gate.
    fn handle_analyze_shortcut(&mut self, ctx: &egui::Context, gate: shortcuts::InputGate) {
        if !gate.allows_app_shortcuts() {
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
    /// Start a new project, asking about unsaved changes first.
    ///
    /// Shared by the File menu and the `Cmd+N` shortcut so the two cannot drift
    /// apart — in particular so the shortcut can never skip the unsaved-changes
    /// prompt the menu shows.
    fn request_new_project(&mut self) {
        if self.is_dirty() {
            self.unsaved_dialog.pending_action = Some(PendingAction::NewProject);
            self.unsaved_dialog.open = true;
        } else {
            self.reset_to_new_project();
            self.dialog_state
                .set_status("New project created".to_string());
        }
    }

    /// Open a project, asking about unsaved changes first. See
    /// [`Self::request_new_project`] for why this is shared.
    fn request_open_project(&mut self) {
        if self.is_dirty() {
            self.unsaved_dialog.pending_action = Some(PendingAction::OpenProject);
            self.unsaved_dialog.open = true;
        } else {
            let initial_dir = self.resolve_project_dir();
            self.dialog_state.open_file_dialog(
                FileDialogMode::OpenProject,
                None,
                initial_dir.as_deref(),
            );
        }
    }

    /// Open the Save As file dialog, pre-filled with the current filename.
    fn open_save_project_as_dialog(&mut self) {
        let has_samples = self.sample_library.read().is_ok_and(|lib| !lib.is_empty());
        let fallback = format!("project.{}", crate::project::project_extension(has_samples));
        let default_name = self
            .current_project_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map_or(fallback, ToString::to_string);
        let initial_dir = self.resolve_project_dir();
        self.dialog_state.open_file_dialog(
            FileDialogMode::SaveProject,
            Some(&default_name),
            initial_dir.as_deref(),
        );
    }

    /// The "File" menu (projects, patches, examples, export, settings, quit).
    fn menu_file(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use egui_remixicon::icons as ri;
        use shortcuts::AppShortcut;

        ui.menu_button("File", |ui| {
            // --- Project ---
            // Each entry renders the same binding the dispatcher acts on, so
            // the menu cannot advertise a shortcut that does something else.
            if menu_entry(ui, ri::FILE_ADD_LINE, AppShortcut::New) {
                self.request_new_project();
                ui.close();
            }
            if menu_entry(ui, ri::FOLDER_OPEN_LINE, AppShortcut::Open) {
                self.request_open_project();
                ui.close();
            }
            if menu_entry(ui, ri::SAVE_LINE, AppShortcut::Save) {
                self.save_current_project();
                ui.close();
            }
            if menu_entry(ui, ri::SAVE_LINE, AppShortcut::SaveAs) {
                self.open_save_project_as_dialog();
                ui.close();
            }
            // --- Recent Projects ---
            submenu_button(ui, (ri::HISTORY_LINE, "Recent Projects"), |ui| {
                let projects = self.settings.recent_projects.clone();
                if projects.is_empty() {
                    ui.label("(none)");
                } else {
                    for path in &projects {
                        let label = path.file_name().and_then(|n| n.to_str()).unwrap_or("???");
                        let btn = ui.button(label).on_hover_text(path.display().to_string());
                        if btn.clicked() {
                            if self.is_dirty() {
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
            if ui.button((ri::FILE_ADD_LINE, "New Patch")).clicked() {
                self.reset_to_new_patch();
                self.dialog_state
                    .set_status("New patch created".to_string());
                ui.close();
            }
            if ui.button((ri::FOLDER_OPEN_LINE, "Open Patch...")).clicked() {
                let initial_dir = self.resolve_open_dir();
                self.dialog_state.open_file_dialog(
                    FileDialogMode::OpenPatch,
                    None,
                    initial_dir.as_deref(),
                );
                ui.close();
            }
            if ui
                .button((ri::FOLDER_OPEN_LINE, "Load Built-in..."))
                .clicked()
            {
                self.dialog_state.show_load_patch = true;
                ui.close();
            }
            if ui.button((ri::SAVE_LINE, "Save Patch...")).clicked() {
                let default_name = format!(
                    "{}.json",
                    self.current_patch_name.to_lowercase().replace(' ', "_")
                );
                let initial_dir = self.resolve_save_dir();
                self.dialog_state.open_file_dialog(
                    FileDialogMode::SavePatch,
                    Some(&default_name),
                    initial_dir.as_deref(),
                );
                ui.close();
            }
            ui.separator();
            submenu_button(ui, (ri::FILE_LIST_LINE, "Example Patches"), |ui| {
                for (category, patches) in categorized_patches() {
                    submenu_button(ui, category, |ui| {
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
            if ui.button((ri::DOWNLOAD_LINE, "Export WAV...")).clicked() {
                // Pre-fill duration from song length
                let song_secs = self.song.read().length_seconds();
                self.dialog_state
                    .export_state
                    .set_duration_from_song(song_secs);
                // Open file dialog to choose WAV path
                let default_name = "export.wav".to_string();
                let initial_dir = self.resolve_project_dir();
                self.dialog_state.open_file_dialog(
                    FileDialogMode::ExportWav,
                    Some(&default_name),
                    initial_dir.as_deref(),
                );
                ui.close();
            }
            ui.separator();
            if ui.button((ri::SETTINGS_LINE, "Settings...")).clicked() {
                // Reload settings from disk to pick up changes
                // made outside the dialog (e.g. last_open_dir)
                self.settings = AppSettings::load();
                self.dialog_state.show_settings = true;
                ui.close();
            }
            ui.separator();
            if ui.button((ri::SHUT_DOWN_LINE, "Quit")).clicked() {
                if self.is_dirty() {
                    self.unsaved_dialog.pending_action = Some(PendingAction::Quit);
                    self.unsaved_dialog.open = true;
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                ui.close();
            }
        });
    }

    /// The "Edit" menu (undo/redo, clipboard, optimize, analyze).
    fn menu_edit(&mut self, ui: &mut egui::Ui) {
        use egui_remixicon::icons as ri;
        ui.menu_button("Edit", |ui| {
            if ui
                .add_enabled(
                    self.undo_manager.can_undo(),
                    egui::Button::new((ri::ARROW_GO_BACK_LINE, "Undo")).shortcut_text("Ctrl+Z"),
                )
                .clicked()
            {
                self.execute_undo();
                ui.close();
            }
            if ui
                .add_enabled(
                    self.undo_manager.can_redo(),
                    egui::Button::new((ri::ARROW_GO_FORWARD_LINE, "Redo"))
                        .shortcut_text("Ctrl+Shift+Z"),
                )
                .clicked()
            {
                self.execute_redo();
                ui.close();
            }
            ui.separator();
            // Copy / Paste / Cut live in the rack's right-click menu (and the
            // Ctrl+C/V/X shortcuts); Analyze Patch lives on the patch context bar.
            if ui
                .button((
                    RichText::new(ri::DELETE_BIN_LINE).color(theme().colors.accent_red),
                    "Optimize Project",
                ))
                .on_hover_text("Remove unused patterns, tracks, and instruments")
                .clicked()
            {
                self.optimize_project();
                ui.close();
            }
        });
    }

    /// The "Help" menu (About).
    fn menu_help(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Help", |ui| {
            if ui.button("About").clicked() {
                self.dialog_state.show_about = true;
                ui.close();
            }
        });
    }

    /// Patch/instrument context bar, docked over the rack editor (the left
    /// instrument list stays full-height to its left). Carries a compact subset
    /// of the active instrument's properties — color, name, category, MIDI
    /// channel, volume, pan, voice mode — edited inline, plus Auto Layout /
    /// Analyze Patch / a button that opens the full Edit-Instrument window for
    /// the long tail. Mirrors the mutation paths in `render_instrument_edit_window`
    /// (edit the `InstrumentUiState` here, push to session/engine afterwards).
    /// Returns `true` if Auto Layout was requested this frame.
    fn render_patch_toolbar(&mut self, ui: &mut egui::Ui, active_id: InstrumentId) -> bool {
        // Snapshot before the editor runs and compare after, rather than
        // recording each of the fifteen properties at its own call site. The
        // editor already collects its changes into `send_*` flags and applies
        // them at the end, so a per-property recording would have to duplicate
        // that bookkeeping — and a property added later would silently miss it.
        let settings_before = self
            .instruments
            .iter()
            .find(|i| i.id == active_id)
            .map(super::instrument_rack::InstrumentUiState::settings);
        let result = self.render_patch_toolbar_inner(ui, active_id);
        self.record_instrument_settings_change(active_id, settings_before);
        result
    }

    /// Record an instrument-property edit, if the editor changed anything.
    fn record_instrument_settings_change(
        &mut self,
        instrument_id: InstrumentId,
        before: Option<crate::gui::instrument_rack::InstrumentSettings>,
    ) {
        let (Some(before), Some(after)) = (
            before,
            self.instruments
                .iter()
                .find(|i| i.id == instrument_id)
                .map(crate::gui::instrument_rack::InstrumentUiState::settings),
        ) else {
            return;
        };
        if before != after {
            self.undo_manager
                .push(crate::undo::UndoAction::SetInstrumentSettings {
                    instrument_id,
                    old: Box::new(before),
                    new: Box::new(after),
                });
        }
    }

    fn render_patch_toolbar_inner(&mut self, ui: &mut egui::Ui, active_id: InstrumentId) -> bool {
        use egui_remixicon::icons as ri;
        use synth_engine::InstrumentCategory;
        let Some(idx) = self.instruments.iter().position(|i| i.id == active_id) else {
            return false;
        };

        let mut send_category = false;
        let mut send_midi: Option<MidiChannelSelection> = None;
        let mut send_volume = false;
        let mut send_pan = false;
        let mut send_transpose = false;
        let mut send_oversampling = false;
        let mut send_max_voices = false;
        let mut send_mode = false;
        let mut send_stealing = false;
        let mut send_unison_detune = false;
        let mut send_unison_spread = false;
        let mut send_vel_amp = false;
        let mut send_vel_filter = false;
        let mut send_sidechain: Option<Option<InstrumentId>> = None;
        let mut send_color = false;
        let mut open_edit = false;
        let mut open_analyze = false;
        let mut auto_layout = false;
        // Other-instrument list for the sidechain picker, captured before the
        // mutable borrow of the active instrument below.
        let other_instruments: Vec<(InstrumentId, String)> = self
            .instruments
            .iter()
            .filter(|i| i.id != active_id)
            .map(|i| (i.id, i.name.clone()))
            .collect();

        super::toolbar::top(ui, "patch_toolbar", |ui| {
            let inst = &mut self.instruments[idx];

            // Color swatch
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
            .on_hover_text("Instrument color")
            .changed()
            {
                inst.color = Some(crate::gui::patch_editor::color32_to_hex(color));
                send_color = true;
            }

            // Category
            let mut cat = inst.category;
            egui::ComboBox::from_id_salt("patch_bar_category")
                .selected_text(cat.name())
                .width(90.0)
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
                })
                .response
                .on_hover_text("Instrument category");
            if cat != inst.category {
                inst.category = cat;
                send_category = true;
            }

            ui.separator();

            // MIDI channel
            dim_label(ui, "Ch");
            let channel = inst.channel;
            let channel_label = if channel.is_omni() {
                "Omni".to_string()
            } else {
                format!("{}", channel.as_one_indexed())
            };
            egui::ComboBox::from_id_salt("patch_bar_midi_ch")
                .selected_text(channel_label)
                .width(60.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(channel.is_omni(), "Omni").clicked() {
                        inst.channel = MidiChannelSelection::OMNI;
                        send_midi = Some(MidiChannelSelection::OMNI);
                    }
                    for ch in 1..=16u8 {
                        let Some(midi_ch) = MidiChannelSelection::from_one_indexed(ch) else {
                            continue;
                        };
                        let is_selected = !channel.is_omni() && channel.as_one_indexed() == ch;
                        if ui
                            .selectable_label(is_selected, format!("Ch {ch}"))
                            .clicked()
                        {
                            inst.channel = midi_ch;
                            send_midi = Some(midi_ch);
                        }
                    }
                })
                .response
                .on_hover_text("MIDI input channel (Omni = respond to all channels)");

            // Volume
            let mut vol = inst.volume.as_f32();
            if ui
                .add(
                    egui::DragValue::new(&mut vol)
                        .range(0.0..=1.0)
                        .speed(0.005)
                        .prefix(RichText::new("Vol ").color(theme().colors.text_dim))
                        .custom_formatter(|v, _| format!("{:>3.0}%", v * 100.0)),
                )
                .on_hover_text("Instrument volume")
                .changed()
                && !inst.muted
            {
                inst.set_volume(synth_core::Gain::new(vol));
                send_volume = true;
            }

            // Pan
            let mut pan = inst.pan.as_f32();
            if ui
                .add(
                    egui::DragValue::new(&mut pan)
                        .range(-1.0..=1.0)
                        .speed(0.01)
                        .prefix(RichText::new("Pan ").color(theme().colors.text_dim))
                        .custom_formatter(|v, _| {
                            if v.abs() < 0.01 {
                                "C".to_string()
                            } else if v < 0.0 {
                                format!("L{:>3.0}", -v * 100.0)
                            } else {
                                format!("R{:>3.0}", v * 100.0)
                            }
                        }),
                )
                .on_hover_text("Stereo pan")
                .changed()
            {
                inst.pan = synth_core::BipolarValue::new(pan);
                send_pan = true;
            }

            // Voice mode
            let current = inst.allocation_mode;
            egui::ComboBox::from_id_salt("patch_bar_alloc_mode")
                .selected_text(format!("{current:?}"))
                .width(95.0)
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
                            send_mode = true;
                        }
                    }
                })
                .response
                .on_hover_text("Voice allocation mode (Polyphonic, Mono, Legato, Unison)");

            // Voice stealing strategy
            let current_steal = inst.stealing_strategy;
            egui::ComboBox::from_id_salt("patch_bar_stealing")
                .selected_text(format!("{current_steal:?}"))
                .width(95.0)
                .show_ui(ui, |ui| {
                    for strategy in [
                        synth_engine::voice_allocator::StealingStrategy::Oldest,
                        synth_engine::voice_allocator::StealingStrategy::Quietest,
                        synth_engine::voice_allocator::StealingStrategy::LowestPriority,
                        synth_engine::voice_allocator::StealingStrategy::SameNote,
                        synth_engine::voice_allocator::StealingStrategy::None,
                    ] {
                        if ui
                            .selectable_label(current_steal == strategy, format!("{strategy:?}"))
                            .clicked()
                        {
                            inst.stealing_strategy = strategy;
                            send_stealing = true;
                        }
                    }
                })
                .response
                .on_hover_text("Which voice is reused when all are busy");

            // Unison detune spread — only meaningful in Unison mode, so greyed out
            // otherwise.
            let is_unison =
                inst.allocation_mode == synth_engine::voice_allocator::AllocationMode::Unison;
            let mut detune_ct = inst.unison_detune.as_f32();
            if ui
                .add_enabled(
                    is_unison,
                    egui::DragValue::new(&mut detune_ct)
                        .range(0.0..=100.0)
                        .speed(0.5)
                        .suffix(" ct"),
                )
                .on_hover_text("Unison detune spread (total cents across all voices)")
                .changed()
            {
                inst.unison_detune = synth_core::Cents::new(detune_ct);
                send_unison_detune = true;
            }

            // Unison stereo spread (0..1 shown as 0..100 %), greyed out outside
            // Unison mode like the detune control above.
            let mut spread_pct = inst.unison_spread.as_f32() * 100.0;
            if ui
                .add_enabled(
                    is_unison,
                    egui::DragValue::new(&mut spread_pct)
                        .range(0.0..=100.0)
                        .speed(0.5)
                        .suffix(" %"),
                )
                .on_hover_text("Unison stereo spread (0 = centred, 100% = full L↔R width)")
                .changed()
            {
                inst.unison_spread = synth_core::NormalizedValue::new(spread_pct / 100.0);
                send_unison_spread = true;
            }

            ui.separator();

            // Transpose
            let mut transpose = inst.transpose.as_f32().round() as i32;
            if ui
                .add(
                    egui::DragValue::new(&mut transpose)
                        .range(-24..=24)
                        .speed(0.1)
                        .prefix(RichText::new("Tr ").color(theme().colors.text_dim))
                        .custom_formatter(|v, _| format!("{v:>3.0}"))
                        .suffix(" st"),
                )
                .on_hover_text("Transpose in semitones (-24 to +24)")
                .changed()
            {
                inst.transpose = synth_core::Semitones::new(transpose.clamp(-24, 24) as f32);
                send_transpose = true;
            }

            // Oversampling
            dim_label(ui, "OS");
            let current_os = inst.oversampling;
            egui::ComboBox::from_id_salt("patch_bar_os")
                .selected_text(current_os.name())
                .width(55.0)
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

            // Max voices
            let mut voices = i32::from(inst.max_voices.as_u8());
            if ui
                .add(
                    egui::DragValue::new(&mut voices)
                        .range(1..=128)
                        .speed(0.2)
                        .prefix(RichText::new("Voices ").color(theme().colors.text_dim))
                        .custom_formatter(|v, _| format!("{v:>3.0}")),
                )
                .on_hover_text("Maximum polyphony (1–128). Takes effect on project reload.")
                .changed()
            {
                inst.max_voices = synth_core::VoiceCount::new(voices.clamp(1, 128) as u8);
                send_max_voices = true;
            }

            ui.separator();

            // Velocity → amp
            let mut va = inst.velocity_amp_sensitivity.as_f32();
            if ui
                .add(
                    egui::DragValue::new(&mut va)
                        .range(0.0..=1.0)
                        .speed(0.005)
                        .prefix((
                            RichText::new("Vel").color(theme().colors.text_dim),
                            RichText::new(ri::ARROW_RIGHT_S_LINE).color(theme().colors.text_dim),
                            RichText::new("A ").color(theme().colors.text_dim),
                        ))
                        .custom_formatter(|v, _| format!("{:>3.0}%", v * 100.0)),
                )
                .on_hover_text("Velocity to amplitude sensitivity (0 = flat, 1 = full)")
                .changed()
            {
                inst.velocity_amp_sensitivity = synth_core::NormalizedValue::new(va);
                send_vel_amp = true;
            }

            // Velocity → filter
            let mut vf = inst.velocity_filter_sensitivity.as_f32();
            if ui
                .add(
                    egui::DragValue::new(&mut vf)
                        .range(0.0..=1.0)
                        .speed(0.005)
                        .prefix((
                            RichText::new("Vel").color(theme().colors.text_dim),
                            RichText::new(ri::ARROW_RIGHT_S_LINE).color(theme().colors.text_dim),
                            RichText::new("F ").color(theme().colors.text_dim),
                        ))
                        .custom_formatter(|v, _| format!("{:>3.0}%", v * 100.0)),
                )
                .on_hover_text("Velocity to filter cutoff sensitivity (0 = none, 1 = full)")
                .changed()
            {
                inst.velocity_filter_sensitivity = synth_core::NormalizedValue::new(vf);
                send_vel_filter = true;
            }

            ui.separator();

            // Sidechain source
            dim_label(ui, "SC");
            let current_sc = inst.sidechain_source_id;
            let sc_label = match current_sc {
                Some(src) => other_instruments
                    .iter()
                    .find(|(id, _)| *id == src)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| format!("#{}", src.as_u64())),
                None => "— None —".to_owned(),
            };
            egui::ComboBox::from_id_salt("patch_bar_sidechain")
                .selected_text(sc_label)
                .width(120.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(current_sc.is_none(), "— None —")
                        .clicked()
                        && current_sc.is_some()
                    {
                        inst.sidechain_source_id = None;
                        send_sidechain = Some(None);
                    }
                    for (id, name) in &other_instruments {
                        let selected = current_sc == Some(*id);
                        if ui.selectable_label(selected, name).clicked() && current_sc != Some(*id)
                        {
                            inst.sidechain_source_id = Some(*id);
                            send_sidechain = Some(Some(*id));
                        }
                    }
                })
                .response
                .on_hover_text(
                    "Route another instrument's audio output into this instrument's \
                    compressors for pumping/ducking (the compressor's Sidechain Enabled \
                    parameter must also be on).",
                );

            // Right-aligned actions: Auto Layout · Analyze · Edit…
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button((
                        RichText::new(ri::EDIT_LINE).color(theme().colors.text_secondary),
                        RichText::new("Edit…").color(theme().colors.text_secondary),
                    ))
                    .on_hover_text("Edit all instrument properties")
                    .clicked()
                {
                    open_edit = true;
                }
                if ui
                    .button((
                        RichText::new(ri::FILE_SEARCH_LINE).color(theme().colors.text_secondary),
                        RichText::new("Analyze").color(theme().colors.text_secondary),
                    ))
                    .on_hover_text("Open the offline analyze view for this patch")
                    .clicked()
                {
                    open_analyze = true;
                }
                if ui
                    .button((
                        RichText::new(ri::LAYOUT_GRID_FILL).color(theme().colors.text_secondary),
                        RichText::new("Auto Layout").color(theme().colors.text_secondary),
                    ))
                    .on_hover_text("Tidy the module layout")
                    .clicked()
                {
                    auto_layout = true;
                }
            });
        });

        // Apply collected changes (kept out of the UI closure so we never borrow
        // session/engine while the InstrumentUiState is borrowed for editing).
        if send_category {
            let cat = self.instruments[idx].category;
            if let Err(e) = self.session.set_instrument_category(active_id, cat) {
                eprintln!("Failed to set instrument category {active_id:?}: {e}");
            }
        }
        if let Some(channel) = send_midi {
            self.handle.send(EngineCommand::SetInstrumentMidiChannel {
                instrument_id: active_id,
                channel,
            });
        }
        if send_volume {
            let volume = self.instruments[idx].volume;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: active_id,
                param: synth_engine::InstrumentParam::Volume(volume),
            });
        }
        if send_pan {
            let pan = self.instruments[idx].pan;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: active_id,
                param: synth_engine::InstrumentParam::Pan(pan),
            });
        }
        if send_transpose {
            let transpose = self.instruments[idx].transpose;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: active_id,
                param: synth_engine::InstrumentParam::Transpose(transpose),
            });
        }
        if send_oversampling {
            let os = self.instruments[idx].oversampling;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: active_id,
                param: synth_engine::InstrumentParam::OversamplingFactor(os),
            });
        }
        if send_mode {
            let mode = self.instruments[idx].allocation_mode;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: active_id,
                param: synth_engine::InstrumentParam::AllocationMode(mode),
            });
        }
        if send_stealing {
            let strategy = self.instruments[idx].stealing_strategy;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: active_id,
                param: synth_engine::InstrumentParam::StealingStrategy(strategy),
            });
        }
        if send_unison_detune {
            let detune = self.instruments[idx].unison_detune;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: active_id,
                param: synth_engine::InstrumentParam::UnisonDetune(detune),
            });
        }
        if send_unison_spread {
            let spread = self.instruments[idx].unison_spread;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: active_id,
                param: synth_engine::InstrumentParam::UnisonSpread(spread),
            });
        }
        if send_vel_amp {
            let s = self.instruments[idx].velocity_amp_sensitivity;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: active_id,
                param: synth_engine::InstrumentParam::VelocityAmpSensitivity(s),
            });
        }
        if send_vel_filter {
            let s = self.instruments[idx].velocity_filter_sensitivity;
            self.handle.send(EngineCommand::SetInstrumentParameter {
                instrument_id: active_id,
                param: synth_engine::InstrumentParam::VelocityFilterSensitivity(s),
            });
        }
        if send_color {
            let color = self.instruments[idx].color.clone();
            if let Err(e) = self
                .session
                .set_instrument_color(active_id, color.as_deref())
            {
                eprintln!("Failed to set instrument color {active_id:?}: {e}");
            }
        }
        if let Some(source) = send_sidechain
            && let Err(e) = self.session.set_sidechain_source(active_id, source)
        {
            eprintln!("Failed to set sidechain source for {active_id:?}: {e}");
        }
        if open_edit {
            self.instrument_edit_target = Some(active_id);
        }
        if open_analyze {
            self.analyze_window.open();
        }
        if send_category
            || send_color
            || send_midi.is_some()
            || send_volume
            || send_pan
            || send_transpose
            || send_oversampling
            || send_max_voices
            || send_mode
            || send_stealing
            || send_vel_amp
            || send_vel_filter
            || send_sidechain.is_some()
        {
            self.mark_dirty();
        }

        auto_layout
    }

    /// Rack-view central panel: render the active instrument's patch editor and
    /// apply every result it returns (parameter/connection/bypass/reorder edits,
    /// module removal, audio-input actions, inline signal monitors, quick-add and
    /// context-add, group-template actions, and auto-layout).
    fn render_rack_central(
        &mut self,
        ui: &mut egui::Ui,
        active_id: InstrumentId,
        mcp_auto_layout: bool,
    ) {
        // Clipboard requests from the rack's right-click menu, applied after the
        // patch-editor borrow ends (copy/cut/paste need &mut self, not the editor).
        let mut do_copy = false;
        let mut do_cut = false;
        let mut do_paste = false;
        egui::CentralPanel::default().show(ui, |ui| {
            // Context bar docked over the rack editor; returns whether its Auto
            // Layout button was clicked this frame.
            let toolbar_auto_layout = self.render_patch_toolbar(ui, active_id);

            // Get the active instrument's patch editor
            let Some(patch_editor) = self
                .instruments
                .iter_mut()
                .find(|i| i.id == active_id)
                .map(|i| &mut i.patch_editor)
            else {
                // No active instrument - show error message
                empty_state(ui, "No active instrument selected");
                return;
            };

            // Update sample list for sampler module dropdowns
            if let Ok(lib) = self.sample_library.read() {
                let list: Vec<(u64, String)> = lib
                    .list()
                    .iter()
                    .map(|m| (m.id.as_u64(), m.name.clone()))
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

            // Modules referenced by an automation lane get an "automated" badge.
            // Built per frame from the sequencer reference index for this
            // instrument (positional module identity → ModuleId).
            let automated_modules: std::collections::HashSet<synth_engine::ModuleId> = {
                let seq_id = active_id;
                self.song
                    .read()
                    .automated_module_params()
                    .keys()
                    .filter(|(inst, _, _)| *inst == seq_id)
                    .map(|(_, module_type, instance)| {
                        synth_engine::ModuleId::new(*module_type, *instance)
                    })
                    .collect()
            };

            // Params this instrument's modules receive from a Mod Grid graph
            // (module → set of type_ids), for the per-param grid-dest marker. Both
            // explicit Module targets and module-backed Instrument targets count.
            let grid_dest_params: std::collections::HashMap<
                synth_engine::ModuleId,
                std::collections::HashSet<String>,
            > = {
                use synth_sequencer::{AutomationTarget, ModNodeConfig};
                let seq_id = active_id;
                let song = self.song.read();
                let mut map: std::collections::HashMap<
                    synth_engine::ModuleId,
                    std::collections::HashSet<String>,
                > = std::collections::HashMap::new();
                for graph in song.mod_graphs() {
                    for cfg in graph.nodes().values() {
                        let ModNodeConfig::Target(t) = cfg else {
                            continue;
                        };
                        let entry = match &t.target {
                            AutomationTarget::Module {
                                instrument,
                                module_type,
                                instance,
                                param_id,
                            } if *instrument == seq_id => Some((
                                synth_engine::ModuleId::new(*module_type, *instance),
                                param_id.as_str().to_string(),
                            )),
                            AutomationTarget::Instrument { instrument, param }
                                if *instrument == seq_id =>
                            {
                                crate::mod_grid_build::instrument_param_module(*param).map(
                                    |(mt, inst, tid)| {
                                        (synth_engine::ModuleId::new(mt, inst), tid.to_string())
                                    },
                                )
                            }
                            _ => None,
                        };
                        if let Some((module_id, type_id)) = entry {
                            map.entry(module_id).or_default().insert(type_id);
                        }
                    }
                }
                map
            };

            let result = patch_editor.show(
                ui,
                &self.handle,
                active_id.as_u64(),
                &effect_chain_order,
                &audio_input_snapshot,
                crate::gui::patch_editor::PatchModProvenance {
                    automated_modules,
                    grid_dest_params,
                },
            );
            let had_mutations = result.has_mutations();
            do_copy = result.request_copy;
            do_cut = result.request_cut;
            do_paste = result.request_paste;

            // Route on the engine-side distinction between effect-chain
            // modules (separate ordered chain) and voice-graph modules
            // (everything else). This mirrors `session.set_parameter`,
            // so utility modules like Mod Matrix — which previously fell
            // through a category whitelist and had their edits silently
            // dropped — now reach shared state like every other module.
            for (module_id, param) in result.param_changes {
                // The pre-edit value comes from the engine's snapshot, not
                // the patch editor's cache: the widget already wrote the new
                // value into that cache when it drew this frame. The snapshot
                // publishes synchronously when a command is *sent*, and this
                // loop has not sent one yet, so it still holds the old value.
                let previous = self
                    .session
                    .state()
                    .shared_graph
                    .get_module(active_id, module_id)
                    .and_then(|snapshot| {
                        snapshot
                            .parameters
                            .into_iter()
                            .find(|p| synth_core::ModuleParam::same_kind(p, &param))
                    });
                if let Some(old) = previous
                    && old != param
                {
                    self.undo_manager
                        .push(crate::undo::UndoAction::SetModuleParameter {
                            instrument_id: active_id,
                            module_id,
                            old,
                            new: param,
                        });
                }

                if module_id.module_type.is_effect() {
                    self.handle.send(EngineCommand::SetEffectParameter {
                        instrument_id: Some(active_id),
                        module_id,
                        param,
                    });
                } else {
                    self.handle.send(EngineCommand::SetModuleParameter {
                        instrument_id: Some(active_id),
                        module_id,
                        param,
                    });

                    if let synth_core::Param::Sampler(synth_core::SamplerParam::SampleSelect(
                        sample_id,
                    )) = param
                        && let Ok(lib) = self.sample_library.read()
                        && let Some(sample) =
                            lib.get(sample_id)
                    {
                        self.handle.send(EngineCommand::LoadSampleData {
                            instrument_id: active_id,
                            module_id,
                            data: std::sync::Arc::clone(&sample.data),
                            channels: sample.meta.channels,
                            frame_count: sample.meta.frame_count.as_usize(),
                            root_note: sample.meta.root_note.unwrap_or(synth_core::MidiNote::new(60)),
                        });
                    }
                }
            }

            // Handle Mod Matrix expression-editor actions (S2.4). Compilation
            // happens in the session (off the audio thread); a compile error is
            // surfaced to the status bar so the routing keeps its prior behaviour.
            for (module_id, slot, source) in result.mod_script_actions {
                let outcome = match &source {
                    Some(src) => self.session.set_mod_script(active_id, module_id, slot, src),
                    None => self.session.clear_mod_script(active_id, module_id, slot),
                };
                if let Err(e) = outcome {
                    self.dialog_state.set_status(format!(
                        "Expression on slot {} not applied: {e}",
                        slot + 1
                    ));
                }
            }

            // Module-instance description edits from the "Edit description" popup.
            // An empty string clears (the engine treats empty as None).
            for (module_id, description) in result.module_description_actions {
                if let Err(e) =
                    self.session
                        .set_module_description(active_id, module_id, Some(&description))
                {
                    self.dialog_state
                        .set_status(format!("Description on {module_id} not applied: {e}"));
                }
            }

            // Handle module removal
            for module_id in result.modules_to_remove {
                // Guard: block removing a module that an automation lane still
                // targets, so the user doesn't silently orphan that lane.
                // Module identity is positional (see AutomationTarget::Module);
                // the GUI treats the engine instrument id as the sequencer id.
                let seq_id = active_id;
                let referenced = self.song.read().is_module_automated(
                    seq_id,
                    module_id.module_type,
                    module_id.instance,
                );
                if referenced {
                    self.dialog_state.set_status(format!(
                        "Can't remove {:?} #{}: an automation lane targets it. Delete the lane first.",
                        module_id.module_type, module_id.instance
                    ));
                    continue;
                }

                // Check if this module has a visualization buffer to clean up
                let has_vis_buffer = patch_editor.module_descriptor(module_id).is_some_and(|d| {
                    d.category == ModuleCategory::Visualizer
                        || d.type_id.as_str() == "signal_monitor"
                        || d.type_id.as_str() == "inline_signal_monitor"
                });

                // Capture the module and its cables *before* removing them:
                // removing also drops every cable attached, so undo has to put
                // both back or the module returns disconnected.
                let removed = std::collections::HashSet::from([module_id]);
                let module_states = patch_editor.extract_module_states(&removed);
                let attached: Vec<synth_engine::graph::Connection> = patch_editor
                    .connections()
                    .iter()
                    .filter(|c| c.from_module == module_id || c.to_module == module_id)
                    .copied()
                    .collect();

                // Remove from session (registry + engine command)
                if let Err(e) = self.session.remove_module(active_id, module_id) {
                    eprintln!("Failed to remove module {module_id:?}: {e}");
                    continue;
                }

                patch_editor.remove_module(module_id);

                self.undo_manager
                    .push(crate::undo::UndoAction::SetRackModules {
                        instrument_id: active_id,
                        modules: module_states,
                        connections: attached,
                        // A removal cuts cables but never leaves one behind to
                        // restore — the module's own cables are `attached`.
                        severed: Vec::new(),
                        restore: false,
                    });

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

            // Handle effect chain reorder requests. Undo records the whole slot
            // order on both sides rather than the direction of the move — see
            // `UndoAction::SetEffectChainOrder` — so the running order is
            // threaded through the loop rather than re-read per click.
            let mut chain_order = effect_chain_order.clone();
            for (module_id, direction) in result.reorder_effects {
                let Some(reordered) =
                    crate::undo::reordered_chain(&chain_order, module_id, direction)
                else {
                    // Already at that end of the chain, or not on it at all:
                    // the engine's own swap would be a no-op, so there is
                    // nothing to undo either.
                    continue;
                };
                if !self.handle.send(EngineCommand::ReorderEffect {
                    instrument_id: Some(active_id),
                    module_id,
                    direction,
                }) {
                    // The engine never got the move, so the chain still stands
                    // as it was and an entry here would reorder it on Ctrl+Z.
                    continue;
                }
                let previous = std::mem::replace(&mut chain_order, reordered);
                self.undo_manager
                    .push(crate::undo::UndoAction::SetEffectChainOrder {
                        instrument_id: Some(active_id),
                        old: previous,
                        new: chain_order.clone(),
                    });
            }

            // Handle audio input actions from patch module
            if let Some(action) = result.audio_input_action {
                use crate::gui::patch_editor::AudioInputAction;
                match action {
                    AudioInputAction::StartMonitoring => {
                        if let Some(host) = &self.host {
                            let device = self.sample_view_state.selected_input_device.as_deref();
                            let config = synth_core::StreamConfig {
                                sample_rate: synth_core::audio::DeviceSampleRate::new(
                                    self.handle.state.sample_rate.load(),
                                ),
                                buffer_size: synth_core::BufferSize::MEDIUM,
                                channels: synth_core::ChannelCount::Stereo,
                            };
                            match self
                                .audio_input
                                .start_monitoring(host.as_ref(), device, &config)
                            {
                                Ok(()) => {
                                    if let Some((consumer, sample_rate)) =
                                        self.audio_input.take_engine_consumer()
                                    {
                                        self.handle.send(EngineCommand::SetAudioInputConsumer {
                                            consumer,
                                            sample_rate,
                                        });
                                    }
                                }
                                Err(e) => {
                                    self.dialog_state.set_status(format!("Input error: {e}"));
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
                        Self::commit_recording_as_sample(
                            &mut self.audio_input,
                            &self.sample_library,
                            &mut self.sample_view_state,
                            &mut self.dialog_state,
                            &mut self.undo_manager,
                            &mut self.drag_coalescer,
                        );
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
                    (Some(a), Some(b)) => egui::Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0),
                    (Some(a), None) => egui::Pos2::new(a.x + 200.0, a.y),
                    _ => egui::Pos2::new(100.0, 100.0),
                };

                patch_editor.add_module_at(monitor_id, inline_descriptor.clone(), mid_pos);

                // Register in session so reconciliation doesn't remove it
                self.session
                    .register_descriptor(active_id, monitor_id, inline_descriptor);

                // Create shared vis buffer and inject into module
                let buffer =
                    std::sync::Arc::new(synth_engine::visualizers::VisualizationBuffer::new(4096));
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

            // Module additions are recorded by diffing the editor's module
            // set around the add handlers, rather than inside each of them.
            // The handlers are static helpers shared by several entry points
            // and some of them add more than one module (a quick-add wires a
            // module in and may insert it inline on a cable) — diffing catches
            // all of it, including paths added later.
            let modules_before: std::collections::HashSet<ModuleId> =
                patch_editor.module_ids().into_iter().collect();
            // Cables are diffed too: an inline add splices the new module onto
            // an existing cable, which means *removing* that cable. Undo has to
            // put it back, so it cannot be left out of the entry.
            let cables_before: Vec<synth_engine::graph::Connection> =
                patch_editor.connections().to_vec();

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

            let added: std::collections::HashSet<ModuleId> = patch_editor
                .module_ids()
                .into_iter()
                .filter(|id| !modules_before.contains(id))
                .collect();
            if !added.is_empty() {
                let module_states = patch_editor.extract_module_states(&added);
                // Only cables touching the new modules belong to this entry —
                // a quick-add wires its module in, and an inline insert splices
                // it onto an existing cable.
                let attached: Vec<synth_engine::graph::Connection> = patch_editor
                    .connections()
                    .iter()
                    .filter(|c| added.contains(&c.from_module) || added.contains(&c.to_module))
                    .copied()
                    .collect();
                // The cable an inline insert replaced: present before, gone
                // after. Undoing has to lay it back down.
                let remaining = patch_editor.connections();
                let severed: Vec<synth_engine::graph::Connection> = cables_before
                    .into_iter()
                    .filter(|c| !remaining.contains(c))
                    .collect();
                self.undo_manager
                    .push(crate::undo::UndoAction::SetRackModules {
                        instrument_id: active_id,
                        modules: module_states,
                        connections: attached,
                        severed,
                        // This entry records an addition, so applying it adds.
                        restore: true,
                    });
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

            // Handle auto-layout request (from GUI menu or MCP).
            //
            // Only here do we clear the MCP pending flag —
            // a request issued while another view was
            // active waits until the next Rack frame.
            if result.request_auto_layout || mcp_auto_layout || toolbar_auto_layout {
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

        // Clipboard actions need &mut self (the patch-editor borrow above is now
        // released). Paste validates clipboard state internally.
        if do_copy {
            self.copy_selected_modules();
        }
        if do_cut {
            self.cut_selected_modules();
        }
        if do_paste {
            self.paste_modules_at_offset();
        }
    }

    /// Rack-view left panel: the instrument list (click to activate, double-click
    /// or the kebab menu to edit/delete, `+` in the header to add one). Unused
    /// instruments — those no track plays — render dimmed.
    fn render_instruments_panel(&mut self, ui: &mut egui::Ui, active_id: Option<InstrumentId>) {
        use crate::gui::list_panel;
        use egui_remixicon::icons as ri;

        egui::Panel::left("instruments_panel")
            .default_size(list_panel::DEFAULT_WIDTH)
            .min_size(list_panel::MIN_WIDTH)
            .show(ui, |ui| {
                // Header + search pinned to the top.
                let mut add_clicked = false;
                egui::Panel::top("instruments_head").show(ui, |ui| {
                    add_clicked =
                        list_panel::header(ui, ri::MUSIC_2_FILL, "Instruments", "New instrument");
                    list_panel::search_box(ui, &mut self.instrument_search);
                });
                if add_clicked {
                    self.add_new_instrument();
                }

                // Track-usage counts: an instrument is "used" when at least one
                // sequencer track routes to it. `None` = song busy, treat all as
                // used (don't dim) this frame.
                let usage: Option<std::collections::HashMap<InstrumentId, u32>> =
                    self.song.try_read().map(|song| {
                        let mut counts = std::collections::HashMap::new();
                        for track in song.tracks() {
                            *counts.entry(track.instrument).or_insert(0) += 1;
                        }
                        counts
                    });
                let mut clicked: Option<InstrumentId> = None;
                let mut edit_requested: Option<InstrumentId> = None;
                let mut delete_requested: Option<InstrumentId> = None;
                list_panel::browser_rows(
                    ui,
                    &self.instruments,
                    &self.instrument_search,
                    "No instruments",
                    |instrument| instrument.name.as_str(),
                    |ui, inst| {
                        let is_active = Some(inst.id) == active_id;
                        let track_count = usage
                            .as_ref()
                            .map_or(1, |m| m.get(&inst.id).copied().unwrap_or(0));
                        let used = track_count > 0;

                        let tip = if used {
                            format!(
                                "Used — {track_count} track{}",
                                if track_count == 1 { "" } else { "s" }
                            )
                        } else {
                            "Unused — no track plays this instrument".to_owned()
                        };
                        let outcome =
                            list_panel::browser_row(ui, is_active, used, &inst.name, tip, |ui| {
                                if ui.button((ri::EDIT_LINE, "Rename / edit…")).clicked() {
                                    edit_requested = Some(inst.id);
                                    ui.close();
                                }
                                ui.separator();
                                if danger_button(ui, format!("{} Delete…", ri::DELETE_BIN_LINE))
                                    .clicked()
                                {
                                    delete_requested = Some(inst.id);
                                    ui.close();
                                }
                            });
                        if outcome.clicked && !is_active {
                            clicked = Some(inst.id);
                        }
                        if outcome.double_clicked {
                            edit_requested = Some(inst.id);
                        }
                    },
                );

                if let Some(id) = clicked {
                    self.active_instrument_id = Some(id);
                    self.handle.set_focused_instrument(Some(id));
                }
                if let Some(id) = edit_requested {
                    self.instrument_edit_target = Some(id);
                }
                if let Some(id) = delete_requested {
                    self.pending_instrument_delete = Some(id);
                }
            });
    }

    /// Top-bar MIDI status indicator with a click-to-select port menu.
    fn render_midi_status(&mut self, ui: &mut egui::Ui) {
        use egui_remixicon::icons as ri;
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
                dim_label(ui, "No MIDI ports available");
            } else {
                for port in &ports {
                    let is_current = self.midi_handler.port_name() == Some(port.as_str());
                    let label = if is_current {
                        RichText::new(format!("{} {}", ri::CHECKBOX_BLANK_CIRCLE_FILL, port))
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

    /// Top-bar MCP connection status indicator (icon + session-count tooltip).
    #[cfg(feature = "mcp")]
    fn render_mcp_status(&self, ui: &mut egui::Ui) {
        use egui_remixicon::icons as ri;
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
                        let mut text = format!("MCP: {} active session(s)\n", session_list.len());
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
    }

    /// Top-bar OSC telemetry status indicator.
    #[cfg(feature = "osc")]
    fn render_osc_status(&self, ui: &mut egui::Ui) {
        use egui_remixicon::icons as ri;
        let osc_status = self
            .osc_shared
            .as_ref()
            .map_or(synth_osc::OscStatus::Off, |s| s.status());
        let (icon, label, color) = match osc_status {
            synth_osc::OscStatus::Connected => {
                (ri::BROADCAST_FILL, "OSC", theme().colors.meter_green)
            }
            synth_osc::OscStatus::Idle => (ri::BROADCAST_LINE, "OSC", theme().colors.text_dim),
            synth_osc::OscStatus::Off => (ri::BROADCAST_LINE, "OSC", theme().colors.accent_red),
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

    /// Top-bar instrument edit pencil + instrument selector dropdown
    /// (switch active instrument, add/rename/delete from the per-row menu).
    fn render_instrument_selector(&mut self, ui: &mut egui::Ui) {
        use egui_remixicon::icons as ri;
        // Instrument edit pencil — sits visually to the right of
        // the dropdown (right-to-left layout). Opens the active
        // instrument's edit window from any view; disabled when
        // no instrument is selected.
        let edit_btn = ui
            .add_enabled(
                self.active_instrument_id.is_some(),
                egui::Button::new(RichText::new(ri::EDIT_LINE).color(theme().colors.text_dim))
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
                .button((
                    RichText::new(ri::ADD_LINE).color(theme().colors.accent_green),
                    RichText::new("New Instrument").color(theme().colors.accent_green),
                ))
                .clicked()
            {
                self.add_new_instrument();
                ui.close();
            }
            ui.separator();
            if self.instruments.is_empty() {
                dim_label(ui, "No instruments");
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
                            RichText::new(format!("{} {}", ri::CHECKBOX_BLANK_CIRCLE_FILL, name))
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
                        submenu_button(
                            ui,
                            RichText::new(ri::MORE_FILL).color(theme().colors.text_dim),
                            |ui| {
                                if ui.button((ri::EDIT_LINE, "Rename / edit…")).clicked() {
                                    self.instrument_edit_target = Some(id);
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .button((
                                        RichText::new(ri::DELETE_BIN_LINE)
                                            .color(theme().colors.accent_red),
                                        "Delete…",
                                    ))
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
    }

    /// The top-bar segmented view selector (Rack / Pattern / Seq / Sample).
    fn render_view_selector(&mut self, ui: &mut egui::Ui) {
        use egui_remixicon::icons as ri;
        let t = theme();
        let views: [(AppView, &str); 8] = [
            (AppView::Home, &format!("{} Home", ri::HOME_FILL)),
            (AppView::Rack, &format!("{} Rack", ri::LAYOUT_GRID_FILL)),
            (AppView::NoteGraph, &format!("{} Notes", ri::MIND_MAP)),
            (AppView::ModGrid, &format!("{} Mod", ri::PULSE_LINE)),
            (AppView::Pattern, &format!("{} Pattern", ri::PIANO_FILL)),
            (AppView::Sequencer, &format!("{} Seq", ri::PLAY_LIST_FILL)),
            (AppView::Mixer, &format!("{} Mixer", ri::EQUALIZER_FILL)),
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
            // Expose each view tab to AccessKit / the egui-inspection MCP as a
            // selectable, so a driver can switch views by name (and read which is
            // active) instead of clicking by pixel. High navigation value.
            expose_selected(
                &resp,
                egui::WidgetType::SelectableLabel,
                label.to_string(),
                is_active,
            );
            if resp.clicked() {
                self.active_view = *view;
            }
        }
    }

    /// Bottom keyboard panel (always visible). Renders the keyboard content in
    /// `Order::Middle` so it takes input priority over module Areas
    /// (`Order::Background`) that may extend into the panel area.
    fn render_keyboard_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("keyboard_panel")
            .min_size(100.0)
            .show(ui, |ui| {
                let layer_id =
                    egui::LayerId::new(egui::Order::Middle, egui::Id::new("keyboard_layer"));
                ui.scope_builder(egui::UiBuilder::new().layer_id(layer_id), |ui| {
                    self.draw_keyboard(ui);
                });
            });
    }

    /// Bottom status bar: octave +/- and glide on the left, load warnings /
    /// CPU / voices / latency on the right.
    fn render_status_bar(&mut self, ui: &mut egui::Ui) {
        /// How many load diagnostics the badge's tooltip spells out before it
        /// counts the rest. A version-skewed project can produce one per
        /// parameter per module, and a tooltip taller than the window is a
        /// worse answer than a count plus the console.
        const BADGE_DIAGNOSTIC_LIMIT: usize = 8;

        egui::Panel::bottom("status_bar")
            .min_size(22.0)
            .show(ui, |ui| {
                // `horizontal_centered`, not `horizontal`: a plain `horizontal`
                // row starts `interact_size.y` (20 px) tall and centres widgets
                // as they are added — the first label lands on that band's
                // centreline, then the taller −/+ buttons grow the row downward
                // and everything after centres ~3 px lower. Allocating the full
                // panel height up front gives every widget the same centreline.
                ui.horizontal_centered(|ui| {
                    let t = theme();

                    // ── Left side: Octave +/- and Glide slider ──
                    let octave_step = stepper(
                        ui,
                        RichText::new(format!("Octave: {:+}", self.keyboard.octave_offset()))
                            .color(t.colors.text_secondary),
                    );
                    if octave_step != 0 {
                        let new_offset = self.keyboard.octave_offset() + octave_step;
                        self.keyboard.set_octave_offset(new_offset);
                    }
                    ui.separator();

                    dim_label(ui, "Glide:");
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

                    // ── Right side: load warnings / CPU / Voices / Latency ──
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // A load that dropped something has to say so where the
                        // user actually is. The full account goes to the
                        // Activity console — which lives on Home, while loading
                        // a project puts the user in the Rack — so without this
                        // a partial load looks exactly like a clean one. First
                        // in a right-to-left layout, i.e. the far corner.
                        if !self.load_diagnostics.is_empty() {
                            let count = self.load_diagnostics.len();
                            let mut tooltip = self
                                .load_diagnostics
                                .iter()
                                .take(BADGE_DIAGNOSTIC_LIMIT)
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join("\n");
                            if let Some(elided) = count.checked_sub(BADGE_DIAGNOSTIC_LIMIT)
                                && elided > 0
                            {
                                tooltip.push_str(&format!("\n… and {elided} more"));
                            }
                            tooltip.push_str("\n\nClick to open the Activity log.");
                            let label = format!("{count} not loaded");
                            if attention_badge(ui, &label, tooltip).clicked() {
                                self.active_view = AppView::Home;
                                // Following the badge is the acknowledgement:
                                // the entries stay in the console, which is now
                                // on screen, so keeping the badge up would only
                                // be a nag with nowhere further to go.
                                self.load_diagnostics.clear();
                            }
                            ui.separator();
                        }

                        if synth_engine::EngineHandle::cpu_profiling_enabled() {
                            let cpu = self.handle.cpu_usage().as_f32();
                            // Copy snapshot so the hover closure captures no borrow of `self`.
                            let bd = self.handle.cpu_breakdown();
                            let cpu_color = if cpu > 0.8 {
                                t.colors.meter_red
                            } else if cpu > 0.5 {
                                t.colors.meter_yellow
                            } else {
                                t.colors.meter_green
                            };
                            ui.label(
                                RichText::new(format!("CPU: {:>3.0}%", cpu * 100.0))
                                    .color(cpu_color),
                            )
                            .on_hover_ui(|ui| {
                                ui.strong("CPU breakdown (audio thread)");
                                ui.add_space(2.0);
                                egui::Grid::new("cpu_breakdown_grid")
                                    .num_columns(2)
                                    .spacing([14.0, 2.0])
                                    .show(ui, |ui| {
                                        let pct = |v: f32| format!("{:>5.1} %", v * 100.0);
                                        let total = bd.total.as_f32();
                                        let voices = bd.voices.as_f32();
                                        let module_graph = bd.module_graph.as_f32();
                                        let master_fx = bd.master_fx.as_f32();
                                        let mod_grid = bd.mod_grid.as_f32();
                                        let other =
                                            (total - voices - module_graph - master_fx - mod_grid)
                                                .max(0.0);
                                        for (label, value) in [
                                            ("Voices", voices),
                                            ("Module graph", module_graph),
                                            ("Mod Grid", mod_grid),
                                            ("Master FX", master_fx),
                                            ("Other", other),
                                        ] {
                                            ui.label(label);
                                            ui.monospace(pct(value));
                                            ui.end_row();
                                        }
                                        ui.separator();
                                        ui.end_row();
                                        ui.strong("Total");
                                        ui.monospace(pct(total));
                                        ui.end_row();
                                    });
                                ui.add_space(2.0);
                                ui.weak("Share of the per-buffer real-time budget.");
                            });
                        } else {
                            ui.label(RichText::new("CPU: OFF").color(t.colors.text_dim))
                                .on_hover_text(
                                    "Enable the rt-profiling feature for callback timing; \
                                     normal builds avoid wall-clock reads on the audio thread.",
                                );
                        }
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
    }

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
                // Snapshot under a short read lock, render against locals, then write
                // back only on change under a short write lock. Never hold the Song
                // lock across widget rendering — the audio thread polls it with
                // try_read(), and a held write lock (esp. across the ComboBox popup)
                // would starve it.
                {
                    let (
                        mut name,
                        mut song_author,
                        mut description,
                        mut bpm,
                        mut ts_num,
                        mut ts_den,
                    ) = {
                        let song = self.song.read();
                        (
                            song.name.clone(),
                            song.author.clone(),
                            song.description.clone(),
                            song.default_tempo.as_f32(),
                            i32::from(song.default_time_signature.numerator),
                            song.default_time_signature.denominator,
                        )
                    };

                    dim_label(ui, "Song name");
                    if ui.text_edit_singleline(&mut name).changed() {
                        song_changed = true;
                    }

                    ui.add_space(t.spacing.md);

                    dim_label(ui, "Song author");
                    if ui.text_edit_singleline(&mut song_author).changed() {
                        song_changed = true;
                    }

                    ui.add_space(t.spacing.md);

                    dim_label(ui, "Description");
                    if ui
                        .add(
                            egui::TextEdit::multiline(&mut description)
                                .desired_rows(2)
                                .desired_width(f32::INFINITY),
                        )
                        .changed()
                    {
                        song_changed = true;
                    }

                    ui.add_space(t.spacing.md);

                    ui.horizontal(|ui| {
                        dim_label(ui, "Tempo (BPM)");
                        if ui
                            .add(
                                egui::DragValue::new(&mut bpm)
                                    .range(20.0..=300.0)
                                    .speed(0.5)
                                    .fixed_decimals(1),
                            )
                            .changed()
                        {
                            bpm = bpm.clamp(20.0, 300.0);
                            send_tempo = Some(synth_core::Bpm::new(bpm));
                            song_changed = true;
                        }
                    });

                    ui.add_space(t.spacing.md);

                    ui.horizontal(|ui| {
                        dim_label(ui, "Time signature");
                        if ui
                            .add(egui::DragValue::new(&mut ts_num).range(1..=32).speed(0.1))
                            .changed()
                        {
                            ts_num = ts_num.clamp(1, 32);
                            song_changed = true;
                        }
                        ui.label("/");
                        egui::ComboBox::from_id_salt("project_edit_time_sig_den")
                            .selected_text(ts_den.to_string())
                            .width(60.0)
                            .show_ui(ui, |ui| {
                                for d in [1u8, 2, 4, 8, 16, 32] {
                                    if ui.selectable_label(ts_den == d, d.to_string()).clicked()
                                        && ts_den != d
                                    {
                                        ts_den = d;
                                        song_changed = true;
                                    }
                                }
                            });
                    });

                    if song_changed {
                        let mut song = self.song.write();
                        song.name = name;
                        song.author = song_author;
                        song.description = description;
                        song.default_tempo = synth_core::Bpm::new(bpm.clamp(20.0, 300.0));
                        song.default_time_signature.numerator = ts_num.clamp(1, 32) as u8;
                        song.default_time_signature.denominator = ts_den;
                    }
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(t.spacing.sm);

                // Project author (Author struct) --------------------------------------
                ui.label(RichText::new("Project Author").color(t.colors.text_primary));
                ui.add_space(t.spacing.xs);

                dim_label(ui, "Name");
                if ui
                    .text_edit_singleline(&mut self.current_project_author.name)
                    .changed()
                {
                    author_changed = true;
                }

                ui.add_space(t.spacing.sm);

                dim_label(ui, "Email");
                if ui
                    .text_edit_singleline(&mut self.current_project_author.email)
                    .changed()
                {
                    author_changed = true;
                }

                ui.add_space(t.spacing.sm);

                dim_label(ui, "Website");
                if ui
                    .text_edit_singleline(&mut self.current_project_author.website)
                    .changed()
                {
                    author_changed = true;
                }

                ui.add_space(t.spacing.sm);

                dim_label(ui, "License");
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
                ui.add_space(t.spacing.md);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                    if danger_button(
                        ui,
                        format!("{} Delete", egui_remixicon::icons::DELETE_BIN_LINE),
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
        // Same snapshot-and-compare as the patch toolbar; see there for why.
        let target = self.instrument_edit_target;
        let settings_before = target.and_then(|id| {
            self.instruments
                .iter()
                .find(|i| i.id == id)
                .map(crate::gui::instrument_rack::InstrumentUiState::settings)
        });
        self.render_instrument_edit_window_inner(ctx);
        if let Some(id) = target {
            self.record_instrument_settings_change(id, settings_before);
        }
    }

    fn render_instrument_edit_window_inner(&mut self, ctx: &egui::Context) {
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
        let mut send_description = false;
        let mut send_patch_description = false;
        let mut send_color = false;

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

                dim_label(ui, "Name");
                if ui.text_edit_singleline(&mut inst.name).changed() {
                    name_changed = true;
                }

                ui.add_space(t.spacing.md);

                dim_label(ui, "Category");
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

                ui.add_space(t.spacing.md);

                dim_label(ui, "Description").on_hover_text(
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

                ui.add_space(t.spacing.sm);

                dim_label(ui, "Patch description").on_hover_text(
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

                ui.add_space(t.spacing.md);

                ui.horizontal(|ui| {
                    dim_label(ui, "Color");
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
                        send_color = true;
                    }
                    if inst.color.is_some() && ui.button("Clear").clicked() {
                        inst.color = None;
                        other_changed = true;
                        send_color = true;
                    }
                });
            });

        if !open {
            self.instrument_edit_target = None;
        }

        if name_changed {
            let new_name = self.instruments[idx].name.clone();
            if let Err(e) = self.session.rename_instrument(target_id, &new_name) {
                tracing::warn!(target: "pertylizer::instrument", error = %e, "failed to rename instrument to '{new_name}'");
            } else {
                tracing::info!(target: "pertylizer::instrument", "renamed instrument to '{new_name}'");
            }
        }
        if category_changed {
            let cat = self.instruments[idx].category;
            if let Err(e) = self.session.set_instrument_category(target_id, cat) {
                eprintln!("Failed to set instrument category {target_id:?}: {e}");
            }
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
        if send_color {
            let color = self.instruments[idx].color.clone();
            if let Err(e) = self
                .session
                .set_instrument_color(target_id, color.as_deref())
            {
                eprintln!("Failed to set instrument color {target_id:?}: {e}");
            }
        }
        if name_changed
            || category_changed
            || other_changed
            || send_description
            || send_patch_description
            || send_color
        {
            self.mark_dirty();
        }
    }
}

#[cfg(test)]
mod version_tracking_tests {
    use super::{VersionTracker, VersionedCache};

    #[test]
    fn tracker_observes_each_version_once_and_can_be_invalidated() {
        let mut tracker = VersionTracker::default();

        assert!(tracker.observe(7));
        assert!(!tracker.observe(7));
        assert!(tracker.observe(8));

        tracker.invalidate();
        assert!(tracker.observe(8));
    }

    #[test]
    fn versioned_cache_keeps_value_until_source_changes() {
        let mut cache = VersionedCache::<Vec<u8>>::default();

        assert!(cache.is_stale(3));
        cache.replace(3, vec![1, 2]);
        assert!(!cache.is_stale(3));
        assert_eq!(cache.value(), &[1, 2]);
        assert!(cache.is_stale(4));
    }
}
