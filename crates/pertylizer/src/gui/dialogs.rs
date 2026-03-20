//! Dialog windows for the synthesizer GUI.
//!
//! This module provides reusable dialog components for settings,
//! about information, and patch management.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Pos2, RichText};
use egui_file_dialog::FileDialog;

use super::egui_backend::setup_custom_style;
use super::theme::{ThemePreset, theme};
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
    /// Opening an AWE preset file.
    OpenAwePreset,
    /// Saving an AWE preset file.
    SaveAwePreset,
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
    /// File dialog instance.
    file_dialog: FileDialog,
    /// Current file dialog mode.
    file_dialog_mode: Option<FileDialogMode>,
    /// Show WAV export dialog.
    pub show_export_wav: bool,
    /// Export dialog state.
    pub export_state: crate::gui::export_dialog::ExportDialogState,
    /// Show save AWE preset dialog.
    pub show_save_awe_preset: bool,
    /// AWE preset name when saving.
    pub awe_preset_save_name: String,
    /// AWE preset description when saving.
    pub awe_preset_save_description: String,
    /// AWE preset tags when saving (comma-separated).
    pub awe_preset_save_tags: String,
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
            file_dialog: FileDialog::new(),
            file_dialog_mode: None,
            show_export_wav: false,
            export_state: crate::gui::export_dialog::ExportDialogState::default(),
            show_save_awe_preset: false,
            awe_preset_save_name: String::new(),
            awe_preset_save_description: String::new(),
            awe_preset_save_tags: String::new(),
        }
    }
}

impl DialogState {
    /// Create a new dialog state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a status message that will auto-dismiss.
    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = Some((message.into(), std::time::Instant::now()));
    }

    /// Clear expired status message.
    pub fn update(&mut self) {
        if let Some((_, instant)) = &self.status_message
            && instant.elapsed().as_secs() >= 3
        {
            self.status_message = None;
        }
    }

    /// Open the file dialog for opening a patch.
    pub fn open_open_patch_dialog(&mut self, initial_dir: Option<&Path>) {
        self.file_dialog_mode = Some(FileDialogMode::OpenPatch);
        let mut dialog = FileDialog::new()
            .add_file_filter(
                "Patch files",
                Arc::new(|p| p.extension().is_some_and(|e| e == "json")),
            )
            .add_file_filter("All files", Arc::new(|_| true));
        if let Some(dir) = initial_dir {
            dialog = dialog.initial_directory(dir.to_path_buf());
        }
        self.file_dialog = dialog;
        self.file_dialog.pick_file();
    }

    /// Open the file dialog for saving a patch.
    pub fn open_save_patch_dialog(&mut self, default_name: &str, initial_dir: Option<&Path>) {
        self.file_dialog_mode = Some(FileDialogMode::SavePatch);
        let mut dialog = FileDialog::new()
            .add_file_filter(
                "Patch files",
                Arc::new(|p| p.extension().is_some_and(|e| e == "json")),
            )
            .default_file_name(default_name);
        if let Some(dir) = initial_dir {
            dialog = dialog.initial_directory(dir.to_path_buf());
        }
        self.file_dialog = dialog;
        self.file_dialog.save_file();
    }

    /// Open the file dialog for opening a project.
    pub fn open_open_project_dialog(&mut self, initial_dir: Option<&Path>) {
        self.file_dialog_mode = Some(FileDialogMode::OpenProject);
        let mut dialog = FileDialog::new()
            .add_file_filter(
                "Project files",
                Arc::new(|p| p.extension().is_some_and(|e| e == "json")),
            )
            .add_file_filter("All files", Arc::new(|_| true));
        if let Some(dir) = initial_dir {
            dialog = dialog.initial_directory(dir.to_path_buf());
        }
        self.file_dialog = dialog;
        self.file_dialog.pick_file();
    }

    /// Open the file dialog for saving a project.
    pub fn open_save_project_dialog(&mut self, default_name: &str, initial_dir: Option<&Path>) {
        self.file_dialog_mode = Some(FileDialogMode::SaveProject);
        let mut dialog = FileDialog::new()
            .add_file_filter(
                "Project files",
                Arc::new(|p| p.extension().is_some_and(|e| e == "json")),
            )
            .default_file_name(default_name);
        if let Some(dir) = initial_dir {
            dialog = dialog.initial_directory(dir.to_path_buf());
        }
        self.file_dialog = dialog;
        self.file_dialog.save_file();
    }

    /// Open the file dialog for choosing a WAV export path.
    pub fn open_export_wav_dialog(&mut self, default_name: &str, initial_dir: Option<&Path>) {
        self.file_dialog_mode = Some(FileDialogMode::ExportWav);
        let mut dialog = FileDialog::new()
            .add_file_filter(
                "WAV files",
                Arc::new(|p| p.extension().is_some_and(|e| e == "wav")),
            )
            .default_file_name(default_name);
        if let Some(dir) = initial_dir {
            dialog = dialog.initial_directory(dir.to_path_buf());
        }
        self.file_dialog = dialog;
        self.file_dialog.save_file();
    }

    /// Open the file dialog for opening an AWE preset.
    pub fn open_open_awe_preset_dialog(&mut self, initial_dir: Option<&Path>) {
        self.file_dialog_mode = Some(FileDialogMode::OpenAwePreset);
        let mut dialog = FileDialog::new()
            .add_file_filter(
                "AWE presets",
                Arc::new(|p| p.extension().is_some_and(|e| e == "json")),
            )
            .add_file_filter("All files", Arc::new(|_| true));
        if let Some(dir) = initial_dir {
            dialog = dialog.initial_directory(dir.to_path_buf());
        }
        self.file_dialog = dialog;
        self.file_dialog.pick_file();
    }

    /// Open the file dialog for saving an AWE preset.
    pub fn open_save_awe_preset_dialog(&mut self, default_name: &str, initial_dir: Option<&Path>) {
        self.file_dialog_mode = Some(FileDialogMode::SaveAwePreset);
        let mut dialog = FileDialog::new()
            .add_file_filter(
                "AWE presets",
                Arc::new(|p| p.extension().is_some_and(|e| e == "json")),
            )
            .default_file_name(default_name);
        if let Some(dir) = initial_dir {
            dialog = dialog.initial_directory(dir.to_path_buf());
        }
        self.file_dialog = dialog;
        self.file_dialog.save_file();
    }

    /// Open the file dialog for opening a group template.
    pub fn open_open_group_template_dialog(&mut self, initial_dir: Option<&Path>) {
        self.file_dialog_mode = Some(FileDialogMode::OpenGroupTemplate);
        let mut dialog = FileDialog::new()
            .add_file_filter(
                "Group templates",
                Arc::new(|p| p.extension().is_some_and(|e| e == "json")),
            )
            .add_file_filter("All files", Arc::new(|_| true));
        if let Some(dir) = initial_dir {
            dialog = dialog.initial_directory(dir.to_path_buf());
        }
        self.file_dialog = dialog;
        self.file_dialog.pick_file();
    }

    /// Update the file dialog and return any completed result.
    pub fn update_file_dialog(&mut self, ctx: &egui::Context) -> Option<FileDialogResult> {
        self.file_dialog.update(ctx);

        if let Some(path) = self.file_dialog.take_picked() {
            let mode = self.file_dialog_mode.take();
            // Distinguish between picked and saved based on mode
            return match mode {
                Some(
                    FileDialogMode::SavePatch
                    | FileDialogMode::SaveProject
                    | FileDialogMode::ExportWav
                    | FileDialogMode::SaveAwePreset,
                ) => Some(FileDialogResult::Saved(path, mode)),
                _ => Some(FileDialogResult::Picked(path, mode)),
            };
        }

        None
    }

    /// Check if a file dialog is currently open.
    pub fn is_file_dialog_open(&self) -> bool {
        self.file_dialog_mode.is_some()
    }
}

/// Result from the file dialog.
pub enum FileDialogResult {
    /// User picked a file.
    Picked(PathBuf, Option<FileDialogMode>),
    /// User saved to a file.
    Saved(PathBuf, Option<FileDialogMode>),
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
            ui.add_space(4.0);

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

            ui.add_space(12.0);
            ui.separator();

            // --- Author / Composer ---
            ui.heading("Author");
            ui.add_space(4.0);

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

            ui.add_space(12.0);
            ui.separator();

            // --- Directories ---
            ui.heading("Directories");
            ui.add_space(4.0);

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
                        ui.label(RichText::new(label).small().color(theme().colors.text_dim));
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
                        ui.label(RichText::new(label).small().color(theme().colors.text_dim));
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
                        ui.label(RichText::new(label).small().color(theme().colors.text_dim));
                    });
                    ui.end_row();

                    // Settings file path (read-only)
                    ui.label("Settings file:");
                    let settings_display = settings_path()
                        .map_or_else(|_| "(unknown)".to_string(), |p| p.display().to_string());
                    ui.label(
                        RichText::new(settings_display)
                            .small()
                            .color(theme().colors.text_dim),
                    );
                    ui.end_row();
                });

            ui.add_space(12.0);
            ui.separator();

            // --- Keyboard ---
            ui.heading("Keyboard Layout");
            ui.label("Lower row (Z-M): C3-B3");
            ui.label("Upper row (Q-I): C4-C5");
            ui.label("-/+ keys: Change octave");

            ui.add_space(16.0);
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
            ui.add_space(8.0);
            ui.label("A modular synthesizer written in Rust.");
            ui.add_space(8.0);

            ui.separator();
            ui.label(RichText::new("Keyboard Controls").strong());
            ui.label("  Z-M: Lower octave (C3-B3)");
            ui.label("  Q-I: Upper octave (C4-C5)");
            ui.label("  -/+: Change octave");
            ui.label("  Space: Panic (all notes off)");

            ui.add_space(8.0);
            ui.separator();
            ui.label(RichText::new("Features").strong());
            ui.label("• Modular synthesis with patch cables");
            ui.label("• Multiple waveforms and filters");
            ui.label("• Effects: Delay, Reverb, Chorus, etc.");
            ui.label("• MIDI support");
            ui.label("• Save/Load patches");

            ui.add_space(16.0);
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
            ui.horizontal(|ui| {
                ui.label(egui_remixicon::icons::SEARCH_LINE);
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
            ui.add_space(4.0);

            let query = search.trim().to_ascii_lowercase();
            let categories = categorized_patches();

            egui::ScrollArea::vertical()
                .max_height(400.0)
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

                        ui.add_space(4.0);
                        ui.label(RichText::new(category).strong());
                        ui.separator();

                        for patch in matching {
                            ui.horizontal(|ui| {
                                if ui.button(&patch.name).clicked() {
                                    result = LoadPatchResult::LoadBuiltin(Box::new(patch.clone()));
                                    *open = false;
                                    search.clear();
                                }
                                if let Some(ref desc) = patch.description {
                                    ui.label(RichText::new(desc).small().color(dim));
                                }
                            });
                        }
                    }

                    if !any_match {
                        ui.add_space(16.0);
                        ui.label(RichText::new("No patches match your search.").color(dim));
                    }
                });

            ui.add_space(8.0);
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
            ui.horizontal(|ui| {
                ui.label("Search");
                ui.text_edit_singleline(search);
                if ui.button("Clear").clicked() {
                    search.clear();
                }
            });
            ui.add_space(8.0);

            let needle = search.to_lowercase();
            let mut shown = 0;

            egui::ScrollArea::vertical()
                .max_height(380.0)
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

                        ui.add_space(4.0);
                        ui.label(RichText::new(category.label()).strong());
                        ui.add_space(2.0);

                        for template in &category_templates {
                            shown += 1;
                            ui.horizontal(|ui| {
                                let is_selected =
                                    selected.as_ref().is_some_and(|s| *s == template.source);
                                if ui.selectable_label(is_selected, &template.name).clicked() {
                                    *selected = Some(template.source.clone());
                                }
                                if matches!(template.source, GroupTemplateSource::BuiltIn(_)) {
                                    ui.label(
                                        RichText::new("Built-in")
                                            .small()
                                            .color(theme().colors.accent_green),
                                    );
                                }
                                if let Some(ref desc) = template.description {
                                    ui.label(
                                        RichText::new(desc).small().color(theme().colors.text_dim),
                                    );
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
                        ui.add_space(4.0);
                        ui.label(RichText::new("Other").strong());
                        ui.add_space(2.0);

                        for template in &uncategorized {
                            shown += 1;
                            ui.horizontal(|ui| {
                                let is_selected =
                                    selected.as_ref().is_some_and(|s| *s == template.source);
                                if ui.selectable_label(is_selected, &template.name).clicked() {
                                    *selected = Some(template.source.clone());
                                }
                                if let Some(ref desc) = template.description {
                                    ui.label(
                                        RichText::new(desc).small().color(theme().colors.text_dim),
                                    );
                                }
                            });
                        }
                    }
                });

            if shown == 0 {
                ui.label(
                    RichText::new("No templates found")
                        .small()
                        .color(theme().colors.text_dim),
                );
            }

            ui.add_space(12.0);
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

    egui::Window::new("Save Group Template")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Template name");
            ui.text_edit_singleline(name);

            ui.add_space(6.0);
            ui.label("Category");
            egui::ComboBox::from_id_salt("group_template_category")
                .selected_text(category.label())
                .show_ui(ui, |ui| {
                    for cat in GroupCategory::ALL {
                        ui.selectable_value(category, cat, cat.label());
                    }
                });

            ui.add_space(6.0);
            ui.label("Description");
            ui.text_edit_multiline(description);

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    result = SaveGroupTemplateResult::Cancelled;
                    *open = false;
                }

                let can_save = !name.trim().is_empty();
                if ui
                    .add_enabled(can_save, egui::Button::new("Save"))
                    .clicked()
                {
                    result = SaveGroupTemplateResult::Save;
                    *open = false;
                }
            });
        });

    result
}

/// Result from save AWE preset dialog.
pub enum SaveAwePresetResult {
    /// No action taken.
    None,
    /// User cancelled.
    Cancelled,
    /// User wants to save — returns (name, description, tags).
    Save,
}

/// Show the save AWE preset dialog.
pub fn show_save_awe_preset_dialog(
    ctx: &egui::Context,
    open: &mut bool,
    name: &mut String,
    description: &mut String,
    tags: &mut String,
    author: &crate::patch::Author,
) -> SaveAwePresetResult {
    if !*open {
        return SaveAwePresetResult::None;
    }

    let mut result = SaveAwePresetResult::None;

    egui::Window::new("Save AWE Preset")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Preset name");
            ui.text_edit_singleline(name);

            ui.add_space(6.0);
            ui.label("Description");
            ui.text_edit_multiline(description);

            ui.add_space(6.0);
            ui.label("Tags (comma-separated)");
            ui.text_edit_singleline(tags);

            ui.add_space(6.0);
            ui.separator();
            ui.label(
                RichText::new("Author")
                    .color(theme().colors.text_dim)
                    .small(),
            );
            if !author.name.is_empty() {
                ui.label(format!("Name: {}", author.name));
            }
            if !author.license.is_empty() {
                ui.label(format!("License: {}", author.license));
            }
            if author.name.is_empty() && author.license.is_empty() {
                ui.label(
                    RichText::new("(set in Settings)")
                        .color(theme().colors.text_dim)
                        .small(),
                );
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    result = SaveAwePresetResult::Cancelled;
                    *open = false;
                }

                let can_save = !name.trim().is_empty();
                if ui
                    .add_enabled(can_save, egui::Button::new("Save"))
                    .clicked()
                {
                    result = SaveAwePresetResult::Save;
                    *open = false;
                }
            });
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

    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(message);
            ui.add_space(16.0);
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
}
