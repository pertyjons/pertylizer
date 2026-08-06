//! Dialog windows for the synthesizer GUI.
//!
//! This module provides reusable dialog components for settings,
//! about information, and patch management.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Pos2, RichText};
use egui_file_dialog::{FileDialog, FileFilter, Filter};

use super::egui_backend::{BUNDLED_FONTS, apply_fonts, resolve_font, setup_custom_style};
use super::theme::{ThemePreset, theme};
use super::widgets::{
    CaptionTone, DialogButton, caption, dialog_button_row, enum_combo, labeled_row, modal_window,
    strong_label,
};
use crate::io::settings::{AppSettings, settings_path};
use crate::io::{GroupTemplateInfo, GroupTemplateManager, GroupTemplateSource, PatchManager};
use crate::patch::{GroupCategory, GroupId, Patch, categorized_patches};
use crate::project;

/// Type of file dialog operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialogMode {
    /// Opening a patch file.
    OpenPatch,
    /// Saving a patch file.
    SavePatch,
    /// Opening a group template file.
    OpenGroupTemplate,
    /// Opening a project file.
    OpenProject,
    /// Saving a project file.
    SaveProject,
    /// Choosing output path for WAV export.
    ExportWav,
    /// Importing a WAV sample file.
    ImportSample,
    /// Exporting a sample as WAV file.
    ExportSample,
    /// Exporting the activity log to a text file.
    ExportActivityLog,
}

/// One entry in a mode's filter dropdown: `(display name, matching extensions)`.
/// An empty extension slice matches every file (the "All files" escape hatch).
type FilterSpec = (&'static str, &'static [&'static str]);

/// The conventional unfiltered entry, offered by every *pick* mode so a file with
/// an unexpected extension can still be reached.
const ANY_FILE: FilterSpec = ("All files", &[]);

impl FileDialogMode {
    /// The filters this mode offers in its dropdown.
    ///
    /// Data, not a builder closure, so every mode's filters read as one table and
    /// the shared dialog can swap them on each open (see
    /// [`DialogState::open_file_dialog`]).
    const fn file_filters(self) -> &'static [FilterSpec] {
        match self {
            Self::OpenPatch => &[("Patch files", &["json"]), ANY_FILE],
            Self::SavePatch => &[("Patch files", &["json"])],
            Self::OpenGroupTemplate => &[("Group templates", &["json"]), ANY_FILE],
            Self::OpenProject => &[("Project files", &["ptz", "json", "zip"]), ANY_FILE],
            Self::SaveProject => &[("Project files", &["ptz", "json", "zip"])],
            Self::ExportWav | Self::ExportSample => &[("WAV files", &["wav"])],
            Self::ImportSample => &[("WAV files", &["wav"]), ANY_FILE],
            Self::ExportActivityLog => &[("Text / log files", &["txt", "log"]), ANY_FILE],
        }
    }

    /// Whether this mode *writes* a file.
    ///
    /// The single source of truth for both halves of a save: it picks
    /// `save_file()` over `pick_file()` when opening, and
    /// [`FileDialogResult::Saved`] over [`FileDialogResult::Picked`] when the user
    /// confirms — so the two can no longer disagree and drop a result on the floor.
    const fn is_save(self) -> bool {
        matches!(
            self,
            Self::SavePatch
                | Self::SaveProject
                | Self::ExportWav
                | Self::ExportSample
                | Self::ExportActivityLog
        )
    }
}

/// State for all application dialogs.
pub struct DialogState {
    /// Show settings dialog.
    pub show_settings: bool,
    /// Show about dialog.
    pub show_about: bool,
    /// Show load patch dialog (built-in patches).
    pub show_load_patch: bool,
    /// Search filter for built-in patch browser.
    pub load_patch_search: String,
    /// Show group template browser dialog.
    pub show_group_templates: bool,
    /// Search filter for group template browser.
    pub group_template_search: String,
    /// Currently selected group template in browser.
    pub group_template_selected: Option<GroupTemplateSource>,
    /// Drop position for inserting a template.
    pub group_template_drop_pos: Option<Pos2>,
    /// Show save group template dialog.
    pub show_save_group_template: bool,
    /// Group ID to save as a template.
    pub group_template_save_group: Option<GroupId>,
    /// Template name when saving.
    pub group_template_save_name: String,
    /// Template description when saving.
    pub group_template_save_description: String,
    /// Template category when saving.
    pub group_template_save_category: GroupCategory,
    /// Name for saving patch.
    pub patch_save_name: String,
    /// Status message with timestamp.
    pub status_message: Option<(String, std::time::Instant)>,
    /// Currently selected theme preset.
    pub current_theme: ThemePreset,
    /// The one file dialog instance, shared by every [`FileDialogMode`]. Never
    /// rebuilt: its retained directory and highlighted entry live inside the
    /// instance, so reusing it is what makes them survive — including across a
    /// switch from Open Patch to Save Project.
    file_dialog: FileDialog,
    /// Current in-flight file dialog mode (for routing the picked result).
    file_dialog_mode: Option<FileDialogMode>,
    /// Show WAV export dialog.
    pub show_export_wav: bool,
    /// Export dialog state.
    pub export_state: crate::gui::export_dialog::ExportDialogState,
}

impl Default for DialogState {
    fn default() -> Self {
        Self {
            show_settings: false,
            show_about: false,
            show_load_patch: false,
            load_patch_search: String::new(),
            show_group_templates: false,
            group_template_search: String::new(),
            group_template_selected: None,
            group_template_drop_pos: None,
            show_save_group_template: false,
            group_template_save_group: None,
            group_template_save_name: String::new(),
            group_template_save_description: String::new(),
            group_template_save_category: GroupCategory::default(),
            patch_save_name: String::new(),
            status_message: None,
            current_theme: ThemePreset::default(),
            file_dialog: Self::new_file_dialog(),
            file_dialog_mode: None,
            show_export_wav: false,
            export_state: crate::gui::export_dialog::ExportDialogState::default(),
        }
    }
}

impl DialogState {
    /// Create a new dialog state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the file dialog with project-wide defaults. Called once, from
    /// [`Default`] — see [`Self::file_dialog`] for why it is never rebuilt.
    ///
    /// `as_modal` is disabled on purpose. Under egui 0.35 `move_to_top` no longer
    /// means "last call wins" — it sets a `wants_to_be_on_top` flag resolved by a
    /// stable sort, so the dialog's dim backdrop overlay and its window tie and the
    /// overlay can end up on top, swallowing every click and freezing the dialog.
    /// A plain top-level window has no overlay and stays fully interactive.
    fn new_file_dialog() -> FileDialog {
        FileDialog::new()
            .as_modal(false)
            .retain_selected_entry(true)
    }

    /// Set a status message that will auto-dismiss.
    ///
    /// The 3-second toast is transient, so the message is also emitted through
    /// `tracing` (target `pertylizer::status`) — that lands it permanently in
    /// the activity-log console and on stderr, where it can be reviewed after
    /// the toast fades.
    pub fn set_status(&mut self, message: impl Into<String>) {
        let message = message.into();
        tracing::info!(target: "pertylizer::status", "{message}");
        self.status_message = Some((message, std::time::Instant::now()));
    }

    /// Clear expired status message.
    pub fn update(&mut self) {
        if let Some((_, instant)) = &self.status_message
            && instant.elapsed().as_secs() >= 3
        {
            self.status_message = None;
        }
    }

    /// Open the shared file dialog in `kind`'s mode.
    ///
    /// Only the per-kind configuration is re-applied — the filters, the default
    /// file name, and the fallback directory — so the instance keeps its own
    /// memory across kinds. `default_name` is meaningful only for save modes
    /// (`FileDialogMode::is_save`); pass `None` when picking. `initial_dir` is a
    /// *fallback*: under the default `OpeningMode::LastPickedDir` the dialog
    /// reopens wherever the user last picked, and only falls back to this
    /// directory before there is such a place.
    pub fn open_file_dialog(
        &mut self,
        kind: FileDialogMode,
        default_name: Option<&str>,
        initial_dir: Option<&Path>,
    ) {
        let config = self.file_dialog.config_mut();
        config.file_filters = kind
            .file_filters()
            .iter()
            .map(|&(name, extensions)| FileFilter {
                id: egui::Id::new(name),
                name: name.to_owned(),
                filter: if extensions.is_empty() {
                    Filter::new(|_: &Path| true)
                } else {
                    Filter::new(move |p: &Path| {
                        p.extension()
                            .and_then(OsStr::to_str)
                            .is_some_and(|ext| extensions.contains(&ext))
                    })
                },
            })
            .collect();
        config.default_file_name = default_name.unwrap_or_default().to_string();
        if let Some(dir) = initial_dir {
            config.initial_directory = dir.to_path_buf();
        }

        self.file_dialog_mode = Some(kind);
        // Opening clears any filter the user had selected for the previous kind,
        // so the swapped-in filters above can never leave a dangling selection.
        if kind.is_save() {
            self.file_dialog.save_file();
        } else {
            self.file_dialog.pick_file();
        }
    }

    /// Update the file dialog and return any completed result.
    ///
    /// Reports a cancellation as well as a pick. `take_picked` yields nothing
    /// when the user backs out, so without the second branch `file_dialog_mode`
    /// would stay set for the rest of the session — and since
    /// [`Self::is_file_dialog_open`] reads it, one cancelled Open Project left
    /// the input gate permanently closed: no application shortcuts and no
    /// computer-keyboard piano until the app was restarted.
    pub fn update_file_dialog(&mut self, ctx: &egui::Context) -> Option<FileDialogResult> {
        self.file_dialog.update(ctx);

        if let Some(path) = self.file_dialog.take_picked() {
            let mode = self.file_dialog_mode.take();
            return Some(match mode {
                Some(kind) if kind.is_save() => FileDialogResult::Saved(path, mode),
                _ => FileDialogResult::Picked(path, mode),
            });
        }
        // Anything that is not still open, without a path to take, is the user
        // backing out. `take_picked` above already closed the picked case, so a
        // mode still set here cannot be one.
        if self.file_dialog_mode.is_some()
            && *self.file_dialog.state() != egui_file_dialog::DialogState::Open
        {
            return Some(FileDialogResult::Cancelled(self.file_dialog_mode.take()));
        }
        None
    }

    /// Check if a file dialog is currently open.
    pub fn is_file_dialog_open(&self) -> bool {
        self.file_dialog_mode.is_some()
    }
}

/// Every dialog that owns keyboard input while it is up.
///
/// A struct rather than a boolean expression at the call site, because
/// [`Self::any_open`] destructures it **exhaustively**: adding a field without
/// adding it to the disjunction is a compile error. That is what the old
/// hand-maintained `||` chain could not do — the instrument-delete confirmation
/// was simply left out of it, so a window whose own text says the action cannot
/// be undone let a bare space start playback and Ctrl+N reach the project
/// behind it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModalDialogs {
    /// The post-crash "recover unsaved work?" offer.
    pub recovery_prompt: bool,
    /// The open/save/import file picker, in any mode.
    ///
    /// Included because it is the one dialog the user routinely leaves without
    /// a focused text field, so without it a bare space would start playback
    /// and the letter keys would play the piano while they browse.
    pub file_dialog: bool,
    /// The unsaved-changes prompt.
    pub unsaved_changes: bool,
    /// Application settings.
    pub settings: bool,
    /// The about box.
    pub about: bool,
    /// The built-in patch browser.
    pub load_patch: bool,
    /// The group-template browser.
    pub group_templates: bool,
    /// The save-group-template form.
    pub save_group_template: bool,
    /// The WAV export dialog.
    pub export_wav: bool,
    /// The "Delete instrument?" confirmation.
    pub instrument_delete: bool,
}

impl ModalDialogs {
    /// Whether any dialog currently owns keyboard input.
    #[must_use]
    pub const fn any_open(self) -> bool {
        // Destructured without `..` on purpose: see the type's documentation.
        let Self {
            recovery_prompt,
            file_dialog,
            unsaved_changes,
            settings,
            about,
            load_patch,
            group_templates,
            save_group_template,
            export_wav,
            instrument_delete,
        } = self;
        recovery_prompt
            || file_dialog
            || unsaved_changes
            || settings
            || about
            || load_patch
            || group_templates
            || save_group_template
            || export_wav
            || instrument_delete
    }
}

/// Result from the file dialog.
pub enum FileDialogResult {
    /// User picked a file.
    Picked(PathBuf, Option<FileDialogMode>),
    /// User saved to a file.
    Saved(PathBuf, Option<FileDialogMode>),
    /// User closed the dialog without choosing anything.
    ///
    /// Reported rather than swallowed because a caller may be holding work that
    /// was waiting on the path — see the unsaved-changes prompt's pending
    /// action, which has to be dropped when the save it deferred to never
    /// happens.
    Cancelled(Option<FileDialogMode>),
}

/// Result from showing the load patch dialog (built-in patches).
pub enum LoadPatchResult {
    /// No action taken.
    None,
    /// User selected a built-in patch to load.
    LoadBuiltin(Box<Patch>),
    /// User cancelled.
    Cancelled,
}

/// Result from group template browser dialog.
pub enum GroupTemplateBrowserResult {
    /// No action taken.
    None,
    /// User cancelled.
    Cancelled,
    /// User selected a template (built-in or file-based).
    Selected(GroupTemplateSource),
    /// User wants to browse for a template file.
    Browse,
}

/// Result from save group template dialog.
pub enum SaveGroupTemplateResult {
    /// No action taken.
    None,
    /// User cancelled.
    Cancelled,
    /// User confirmed save.
    Save,
}

/// Result from the settings dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    /// No action (dialog still open, nothing changed).
    None,
    /// A setting was changed live (e.g. theme) — save immediately.
    LiveChange,
    /// User clicked "Save & Close".
    SaveAndClose,
    /// User clicked "Close" without saving.
    CloseWithoutSave,
}

/// Show the settings dialog.
///
/// Returns a [`SettingsAction`] indicating what happened.
pub fn show_settings_dialog(
    ctx: &egui::Context,
    open: &mut bool,
    current_theme: &mut ThemePreset,
    settings: &mut AppSettings,
) -> SettingsAction {
    if !*open {
        return SettingsAction::None;
    }

    let mut changed = false;
    let mut action = SettingsAction::None;

    egui::Window::new("Settings")
        .collapsible(false)
        .resizable(true)
        .min_width(360.0)
        .show(ctx, |ui| {
            // --- Theme ---
            ui.heading("Theme");
            ui.add_space(theme().spacing.xs);

            egui::Grid::new("theme_grid")
                .num_columns(4)
                .spacing([8.0, 8.0])
                .show(ui, |ui| {
                    for (i, preset) in ThemePreset::ALL.iter().enumerate() {
                        let is_selected = *current_theme == *preset;
                        let button = egui::Button::new(preset.name())
                            .selected(is_selected)
                            .min_size(egui::vec2(70.0, 28.0));

                        if ui.add(button).clicked() {
                            *current_theme = *preset;
                            settings.theme = *preset;
                            preset.apply();
                            setup_custom_style(ctx);
                            changed = true;
                        }

                        if (i + 1) % 4 == 0 {
                            ui.end_row();
                        }
                    }
                });

            ui.add_space(theme().spacing.lg);
            ui.separator();

            // --- Font ---
            ui.heading("Font");
            ui.add_space(theme().spacing.xs);

            // The whole UI is monospace; this picks which bundled mono font.
            // Options come straight from the generated `BUNDLED_FONTS` table.
            let current_font = resolve_font(settings.font.as_deref());
            let mut chosen = current_font;
            let font_options: Vec<(&str, &str)> =
                BUNDLED_FONTS.iter().map(|f| (f.0, f.0)).collect();
            enum_combo(ui, "font_combo", &mut chosen, &font_options);
            if chosen != current_font {
                settings.font = Some(chosen.to_owned());
                apply_fonts(ctx, chosen);
                changed = true;
            }

            ui.add_space(theme().spacing.lg);
            ui.separator();

            // --- Author / Composer ---
            ui.heading("Author");
            ui.add_space(theme().spacing.xs);

            egui::Grid::new("author_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(80.0)
                .show(ui, |ui| {
                    ui.label("Name:");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut settings.author.name)
                                .desired_width(220.0)
                                .hint_text("Your name"),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.end_row();

                    ui.label("Email:");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut settings.author.email)
                                .desired_width(220.0)
                                .hint_text("email@example.com"),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.end_row();

                    ui.label("Website:");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut settings.author.website)
                                .desired_width(220.0)
                                .hint_text("https://"),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.end_row();

                    ui.label("License:");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut settings.author.license)
                                .desired_width(220.0)
                                .hint_text("CC BY 4.0"),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.end_row();
                });

            ui.add_space(theme().spacing.lg);
            ui.separator();

            // --- Directories ---
            ui.heading("Directories");
            ui.add_space(theme().spacing.xs);

            egui::Grid::new("directories_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .min_col_width(80.0)
                .show(ui, |ui| {
                    // Patches directory
                    ui.label("Patches dir:");
                    ui.horizontal(|ui| {
                        let default_dir =
                            PatchManager::default_patches_dir().ok().unwrap_or_default();
                        let is_custom = settings.directories.patches_dir.is_some();
                        let display_path = settings
                            .directories
                            .patches_dir
                            .as_ref()
                            .unwrap_or(&default_dir);
                        let label = if is_custom {
                            display_path.display().to_string()
                        } else {
                            format!("{} (default)", display_path.display())
                        };
                        caption(ui, label, CaptionTone::Dim);
                        if is_custom && ui.small_button("Reset").clicked() {
                            settings.directories.patches_dir = None;
                            changed = true;
                        }
                    });
                    ui.end_row();

                    // Projects directory
                    ui.label("Projects dir:");
                    ui.horizontal(|ui| {
                        let default_dir = project::default_projects_dir().ok().unwrap_or_default();
                        let is_custom = settings.directories.projects_dir.is_some();
                        let display_path = settings
                            .directories
                            .projects_dir
                            .as_ref()
                            .unwrap_or(&default_dir);
                        let label = if is_custom {
                            display_path.display().to_string()
                        } else {
                            format!("{} (default)", display_path.display())
                        };
                        caption(ui, label, CaptionTone::Dim);
                        if is_custom && ui.small_button("Reset").clicked() {
                            settings.directories.projects_dir = None;
                            changed = true;
                        }
                    });
                    ui.end_row();

                    // Group templates directory
                    ui.label("Group templates dir:");
                    ui.horizontal(|ui| {
                        let default_dir = GroupTemplateManager::default_templates_dir()
                            .ok()
                            .unwrap_or_default();
                        let label = format!("{} (default)", default_dir.display());
                        caption(ui, label, CaptionTone::Dim);
                    });
                    ui.end_row();

                    // Settings file path (read-only)
                    ui.label("Settings file:");
                    let settings_display = settings_path()
                        .map_or_else(|_| "(unknown)".to_string(), |p| p.display().to_string());
                    caption(ui, settings_display, CaptionTone::Dim);
                    ui.end_row();
                });

            ui.add_space(theme().spacing.lg);
            ui.separator();

            // --- Keyboard ---
            ui.heading("Keyboard Layout");
            ui.label("Lower row (Z-M): C3-B3");
            ui.label("Upper row (Q-I): C4-C5");
            ui.label("-/+ keys: Change octave");

            ui.add_space(theme().spacing.xl);
            ui.horizontal(|ui| {
                if ui.button("Save & Close").clicked() {
                    action = SettingsAction::SaveAndClose;
                    *open = false;
                }
                if ui.button("Close").clicked() {
                    action = SettingsAction::CloseWithoutSave;
                    *open = false;
                }
            });
        });

    if changed && action == SettingsAction::None {
        return SettingsAction::LiveChange;
    }
    action
}

/// Show the about dialog.
pub fn show_about_dialog(ctx: &egui::Context, open: &mut bool) {
    if !*open {
        return;
    }

    egui::Window::new("About")
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.heading("Pertylizer");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.add_space(theme().spacing.md);
            ui.label("A modular synthesizer written in Rust.");
            ui.add_space(theme().spacing.md);

            ui.separator();
            strong_label(ui, "Keyboard Controls", None);
            ui.label("  Z-M: Lower octave (C3-B3)");
            ui.label("  Q-I: Upper octave (C4-C5)");
            ui.label("  -/+: Change octave");
            ui.label("  PANIC button (top bar): hard-stop all sound");

            ui.add_space(theme().spacing.md);
            ui.separator();
            strong_label(ui, "Features", None);
            ui.label("• Modular synthesis with patch cables");
            ui.label("• Multiple waveforms and filters");
            ui.label("• Effects: Delay, Reverb, Chorus, etc.");
            ui.label("• MIDI support");
            ui.label("• Save/Load patches");

            ui.add_space(theme().spacing.xl);
            if ui.button("Close").clicked() {
                *open = false;
            }
        });
}

/// Show the load patch dialog with search and categories.
///
/// Returns the action the user wants to take.
pub fn show_load_patch_dialog(
    ctx: &egui::Context,
    open: &mut bool,
    search: &mut String,
) -> LoadPatchResult {
    if !*open {
        return LoadPatchResult::None;
    }

    let mut result = LoadPatchResult::None;

    egui::Window::new("Load Built-in Patch")
        .collapsible(false)
        .resizable(true)
        .default_width(450.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            // Search field
            labeled_row(ui, egui_remixicon::icons::SEARCH_LINE, |ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(search)
                        .hint_text("Search name, description, tags...")
                        .desired_width(f32::INFINITY),
                );
                // Auto-focus the search field when dialog opens
                if search.is_empty() {
                    response.request_focus();
                }
            });
            ui.add_space(theme().spacing.xs);

            let query = search.trim().to_ascii_lowercase();
            let categories = categorized_patches();

            egui::ScrollArea::vertical()
                .max_height(400.0)
                .content_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    let dim = theme().colors.text_dim;
                    let mut any_match = false;

                    for (category, patches) in &categories {
                        // Filter patches matching the search query
                        let matching: Vec<&Patch> = patches
                            .iter()
                            .filter(|p| {
                                if query.is_empty() {
                                    return true;
                                }
                                let name = p.name.to_ascii_lowercase();
                                if name.contains(&query) {
                                    return true;
                                }
                                if let Some(ref desc) = p.description
                                    && desc.to_ascii_lowercase().contains(&query)
                                {
                                    return true;
                                }
                                p.tags
                                    .iter()
                                    .any(|t| t.to_ascii_lowercase().contains(&query))
                            })
                            .collect();

                        if matching.is_empty() {
                            continue;
                        }
                        any_match = true;

                        ui.add_space(theme().spacing.xs);
                        strong_label(ui, category, None);
                        ui.separator();

                        for patch in matching {
                            ui.horizontal(|ui| {
                                if ui.button(&patch.name).clicked() {
                                    result = LoadPatchResult::LoadBuiltin(Box::new(patch.clone()));
                                    *open = false;
                                    search.clear();
                                }
                                if let Some(ref desc) = patch.description {
                                    caption(ui, desc, CaptionTone::Dim);
                                }
                            });
                        }
                    }

                    if !any_match {
                        ui.add_space(theme().spacing.xl);
                        ui.label(RichText::new("No patches match your search.").color(dim));
                    }
                });

            ui.add_space(theme().spacing.md);
            if ui.button("Cancel").clicked() {
                result = LoadPatchResult::Cancelled;
                *open = false;
                search.clear();
            }
        });

    result
}

/// Show the group template browser dialog.
pub fn show_group_template_browser(
    ctx: &egui::Context,
    open: &mut bool,
    templates: &[GroupTemplateInfo],
    search: &mut String,
    selected: &mut Option<GroupTemplateSource>,
) -> GroupTemplateBrowserResult {
    if !*open {
        return GroupTemplateBrowserResult::None;
    }

    let mut result = GroupTemplateBrowserResult::None;

    egui::Window::new("Group Templates")
        .collapsible(false)
        .resizable(true)
        .min_width(420.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            labeled_row(ui, "Search", |ui| {
                ui.text_edit_singleline(search);
                if ui.button("Clear").clicked() {
                    search.clear();
                }
            });
            ui.add_space(theme().spacing.md);

            let needle = search.to_lowercase();
            let mut shown = 0;

            egui::ScrollArea::vertical()
                .max_height(380.0)
                .content_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    // Group by category
                    for category in GroupCategory::ALL {
                        let category_templates: Vec<_> = templates
                            .iter()
                            .filter(|t| {
                                t.category == Some(category) && {
                                    let hay_name = t.name.to_lowercase();
                                    let hay_desc = t
                                        .description
                                        .as_ref()
                                        .map(|d| d.to_lowercase())
                                        .unwrap_or_default();
                                    needle.is_empty()
                                        || hay_name.contains(&needle)
                                        || hay_desc.contains(&needle)
                                }
                            })
                            .collect();

                        if category_templates.is_empty() {
                            continue;
                        }

                        ui.add_space(theme().spacing.xs);
                        strong_label(ui, category.label(), None);
                        ui.add_space(theme().spacing.xxs);

                        for template in &category_templates {
                            shown += 1;
                            ui.horizontal(|ui| {
                                let is_selected =
                                    selected.as_ref().is_some_and(|s| *s == template.source);
                                if ui.selectable_label(is_selected, &template.name).clicked() {
                                    *selected = Some(template.source.clone());
                                }
                                if matches!(template.source, GroupTemplateSource::BuiltIn(_)) {
                                    caption(
                                        ui,
                                        "Built-in",
                                        CaptionTone::Color(theme().colors.accent_green),
                                    );
                                }
                                if let Some(ref desc) = template.description {
                                    caption(ui, desc, CaptionTone::Dim);
                                }
                            });
                        }
                    }

                    // Uncategorized templates (file-based with no category)
                    let uncategorized: Vec<_> = templates
                        .iter()
                        .filter(|t| {
                            t.category.is_none() && {
                                let hay_name = t.name.to_lowercase();
                                let hay_desc = t
                                    .description
                                    .as_ref()
                                    .map(|d| d.to_lowercase())
                                    .unwrap_or_default();
                                needle.is_empty()
                                    || hay_name.contains(&needle)
                                    || hay_desc.contains(&needle)
                            }
                        })
                        .collect();

                    if !uncategorized.is_empty() {
                        ui.add_space(theme().spacing.xs);
                        strong_label(ui, "Other", None);
                        ui.add_space(theme().spacing.xxs);

                        for template in &uncategorized {
                            shown += 1;
                            ui.horizontal(|ui| {
                                let is_selected =
                                    selected.as_ref().is_some_and(|s| *s == template.source);
                                if ui.selectable_label(is_selected, &template.name).clicked() {
                                    *selected = Some(template.source.clone());
                                }
                                if let Some(ref desc) = template.description {
                                    caption(ui, desc, CaptionTone::Dim);
                                }
                            });
                        }
                    }
                });

            if shown == 0 {
                caption(ui, "No templates found", CaptionTone::Dim);
            }

            ui.add_space(theme().spacing.lg);
            ui.horizontal(|ui| {
                let can_load = selected.is_some();
                if ui
                    .add_enabled(can_load, egui::Button::new("Load"))
                    .clicked()
                    && let Some(source) = selected.clone()
                {
                    result = GroupTemplateBrowserResult::Selected(source);
                    *open = false;
                }
                if ui.button("Browse...").clicked() {
                    result = GroupTemplateBrowserResult::Browse;
                    *open = false;
                }
                if ui.button("Cancel").clicked() {
                    result = GroupTemplateBrowserResult::Cancelled;
                    *open = false;
                }
            });
        });

    result
}

/// Show the save group template dialog.
pub fn show_save_group_template_dialog(
    ctx: &egui::Context,
    open: &mut bool,
    name: &mut String,
    description: &mut String,
    category: &mut GroupCategory,
) -> SaveGroupTemplateResult {
    if !*open {
        return SaveGroupTemplateResult::None;
    }

    let mut result = SaveGroupTemplateResult::None;

    modal_window(ctx, "Save Group Template", |ui| {
        ui.label("Template name");
        ui.text_edit_singleline(name);

        ui.add_space(theme().spacing.sm);
        ui.label("Category");
        egui::ComboBox::from_id_salt("group_template_category")
            .selected_text(category.label())
            .show_ui(ui, |ui| {
                for cat in GroupCategory::ALL {
                    ui.selectable_value(category, cat, cat.label());
                }
            });

        ui.add_space(theme().spacing.sm);
        ui.label("Description");
        ui.text_edit_multiline(description);

        ui.add_space(theme().spacing.lg);
        match dialog_button_row(ui, "Save", !name.trim().is_empty()) {
            DialogButton::Cancel => {
                result = SaveGroupTemplateResult::Cancelled;
                *open = false;
            }
            DialogButton::Confirm => {
                result = SaveGroupTemplateResult::Save;
                *open = false;
            }
            DialogButton::None => {}
        }
    });

    result
}

/// Show a status toast message.
pub fn show_status_toast(ctx: &egui::Context, state: &mut DialogState) {
    if let Some((ref msg, instant)) = state.status_message {
        if instant.elapsed().as_secs() < 3 {
            egui::Window::new("Status")
                .collapsible(false)
                .resizable(false)
                .title_bar(false)
                .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -50.0])
                .show(ctx, |ui| {
                    ui.label(RichText::new(msg).color(theme().colors.accent_green));
                });
        } else {
            state.status_message = None;
        }
    }
}

/// Show a confirmation dialog.
///
/// Returns true if confirmed, false if cancelled, None if still showing.
pub fn show_confirm_dialog(
    ctx: &egui::Context,
    title: &str,
    message: &str,
    open: &mut bool,
) -> Option<bool> {
    let mut result = None;

    modal_window(ctx, title, |ui| {
        ui.label(message);
        ui.add_space(theme().spacing.xl);
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                result = Some(true);
                *open = false;
            }
            if ui.button("Cancel").clicked() {
                result = Some(false);
                *open = false;
            }
        });
    });

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dialog_state_default() {
        let state = DialogState::new();
        assert!(!state.show_settings);
        assert!(!state.show_about);
        assert!(state.status_message.is_none());
    }

    #[test]
    fn test_dialog_state_status() {
        let mut state = DialogState::new();
        state.set_status("Test message");
        assert!(state.status_message.is_some());
    }

    /// Every dialog mode. `FileDialogMode::file_filters` matches exhaustively, so
    /// a new variant can't be forgotten there — this list only keeps the tests
    /// below covering it too.
    const ALL_MODES: [FileDialogMode; 9] = [
        FileDialogMode::OpenPatch,
        FileDialogMode::SavePatch,
        FileDialogMode::OpenGroupTemplate,
        FileDialogMode::OpenProject,
        FileDialogMode::SaveProject,
        FileDialogMode::ExportWav,
        FileDialogMode::ImportSample,
        FileDialogMode::ExportSample,
        FileDialogMode::ExportActivityLog,
    ];

    /// Nothing open is the only state in which keystrokes reach the document.
    #[test]
    fn no_dialog_open_leaves_the_gate_alone() {
        assert!(!ModalDialogs::default().any_open());
    }

    /// Each dialog on its own has to close the gate. Written out one field at a
    /// time rather than in a loop: this is the list that must be complete, and
    /// `any_open`'s exhaustive destructuring is what makes a *new* field fail to
    /// compile until it is handled — this test is what catches a field handled
    /// wrongly. The instrument-delete confirmation was the one that had been
    /// missed, on a window whose own text says the action cannot be undone.
    #[test]
    fn any_single_dialog_closes_the_gate() {
        type OpenOne = fn(&mut ModalDialogs);
        let cases: [(&str, OpenOne); 10] = [
            ("recovery_prompt", |m| m.recovery_prompt = true),
            ("file_dialog", |m| m.file_dialog = true),
            ("unsaved_changes", |m| m.unsaved_changes = true),
            ("settings", |m| m.settings = true),
            ("about", |m| m.about = true),
            ("load_patch", |m| m.load_patch = true),
            ("group_templates", |m| m.group_templates = true),
            ("save_group_template", |m| m.save_group_template = true),
            ("export_wav", |m| m.export_wav = true),
            ("instrument_delete", |m| m.instrument_delete = true),
        ];
        for (name, open) in cases {
            let mut modals = ModalDialogs::default();
            open(&mut modals);
            assert!(modals.any_open(), "{name} must own keyboard input");
        }
    }

    #[test]
    fn every_mode_offers_usable_filters() {
        for mode in ALL_MODES {
            let filters = mode.file_filters();
            assert!(!filters.is_empty(), "{mode:?} offers no filters");
            assert!(
                filters.iter().any(|(_, ext)| !ext.is_empty()),
                "{mode:?} only offers the unfiltered entry"
            );
            for (name, extensions) in filters {
                // The names become `egui::Id`s in the filter dropdown; a repeat
                // would silently overwrite the earlier entry.
                assert_eq!(
                    filters.iter().filter(|(n, _)| n == name).count(),
                    1,
                    "{mode:?} repeats the filter name {name:?}"
                );
                for ext in *extensions {
                    assert!(
                        !ext.starts_with('.') && ext.chars().all(char::is_lowercase),
                        "{mode:?}: {ext:?} must be a bare lowercase extension"
                    );
                }
            }
        }
    }

    #[test]
    fn writing_modes_are_marked_as_saves() {
        // Regression: `ExportActivityLog` opened the dialog with `save_file()` but
        // was reported as `Picked`, while the only handler for it matches `Saved`
        // — so the export silently did nothing. Both halves now read `is_save`.
        for mode in ALL_MODES {
            let expected = matches!(
                mode,
                FileDialogMode::SavePatch
                    | FileDialogMode::SaveProject
                    | FileDialogMode::ExportWav
                    | FileDialogMode::ExportSample
                    | FileDialogMode::ExportActivityLog
            );
            assert_eq!(mode.is_save(), expected, "{mode:?}");
        }
    }
}
