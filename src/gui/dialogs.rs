//! Dialog windows for the synthesizer GUI.
//!
//! This module provides reusable dialog components for settings,
//! about information, and patch management.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, RichText};
use egui_file_dialog::FileDialog;

use super::egui_backend::setup_custom_style;
use super::theme::{ThemePreset, theme};
use crate::patch::{Patch, example_patches};

/// Type of file dialog operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDialogMode {
    /// Opening a patch file.
    OpenPatch,
    /// Saving a patch file.
    SavePatch,
    /// Importing a song (tracker file).
    ImportSong,
    /// Opening a WAV sample.
    OpenSample,
}

/// State for all application dialogs.
pub struct DialogState {
    /// Show settings dialog.
    pub show_settings: bool,
    /// Show about dialog.
    pub show_about: bool,
    /// Show load patch dialog (built-in patches).
    pub show_load_patch: bool,
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
}

impl Default for DialogState {
    fn default() -> Self {
        Self {
            show_settings: false,
            show_about: false,
            show_load_patch: false,
            patch_save_name: String::new(),
            status_message: None,
            current_theme: ThemePreset::default(),
            file_dialog: FileDialog::new(),
            file_dialog_mode: None,
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

    /// Open the file dialog for importing a song.
    pub fn open_import_song_dialog(&mut self) {
        self.file_dialog_mode = Some(FileDialogMode::ImportSong);
        self.file_dialog = FileDialog::new()
            .add_file_filter(
                "Tracker files",
                Arc::new(|p| {
                    p.extension().and_then(|e| e.to_str()).is_some_and(|ext| {
                        matches!(ext.to_lowercase().as_str(), "mod" | "xm" | "s3m")
                    })
                }),
            )
            .add_file_filter("All files", Arc::new(|_| true));
        self.file_dialog.pick_file();
    }

    /// Open the file dialog for opening a patch.
    pub fn open_open_patch_dialog(&mut self) {
        self.file_dialog_mode = Some(FileDialogMode::OpenPatch);
        self.file_dialog = FileDialog::new()
            .add_file_filter(
                "Patch files",
                Arc::new(|p| p.extension().is_some_and(|e| e == "json")),
            )
            .add_file_filter("All files", Arc::new(|_| true));
        self.file_dialog.pick_file();
    }

    /// Open the file dialog for saving a patch.
    pub fn open_save_patch_dialog(&mut self, default_name: &str) {
        self.file_dialog_mode = Some(FileDialogMode::SavePatch);
        self.file_dialog = FileDialog::new()
            .add_file_filter(
                "Patch files",
                Arc::new(|p| p.extension().is_some_and(|e| e == "json")),
            )
            .default_file_name(default_name);
        self.file_dialog.save_file();
    }

    /// Open the file dialog for opening a sample.
    pub fn open_sample_dialog(&mut self) {
        self.file_dialog_mode = Some(FileDialogMode::OpenSample);
        self.file_dialog = FileDialog::new()
            .add_file_filter(
                "Audio files",
                Arc::new(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
                }),
            )
            .add_file_filter("All files", Arc::new(|_| true));
        self.file_dialog.pick_file();
    }

    /// Update the file dialog and return any completed result.
    pub fn update_file_dialog(&mut self, ctx: &egui::Context) -> Option<FileDialogResult> {
        self.file_dialog.update(ctx);

        if let Some(path) = self.file_dialog.take_picked() {
            let mode = self.file_dialog_mode.take();
            // Distinguish between picked and saved based on mode
            return match mode {
                Some(FileDialogMode::SavePatch) => Some(FileDialogResult::Saved(path, mode)),
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

/// Show the settings dialog.
pub fn show_settings_dialog(ctx: &egui::Context, open: &mut bool, current_theme: &mut ThemePreset) {
    if !*open {
        return;
    }

    egui::Window::new("Settings")
        .collapsible(false)
        .resizable(true)
        .min_width(300.0)
        .show(ctx, |ui| {
            // Theme section
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
                            preset.apply();
                            setup_custom_style(ctx);
                        }

                        // 4 kolumner per rad
                        if (i + 1) % 4 == 0 {
                            ui.end_row();
                        }
                    }
                });

            ui.add_space(12.0);
            ui.separator();

            // Audio section (placeholder)
            ui.heading("Audio");
            ui.label("Audio settings coming soon...");

            ui.add_space(12.0);
            ui.separator();

            // Keyboard section
            ui.heading("Keyboard Layout");
            ui.label("Lower row (Z-M): C3-B3");
            ui.label("Upper row (Q-I): C4-C5");
            ui.label("-/+ keys: Change octave");

            ui.add_space(16.0);
            if ui.button("Close").clicked() {
                *open = false;
            }
        });
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
            ui.heading("Modular Synthesizer");
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

/// Show the load patch dialog.
///
/// Returns the action the user wants to take.
pub fn show_load_patch_dialog(ctx: &egui::Context, open: &mut bool) -> LoadPatchResult {
    if !*open {
        return LoadPatchResult::None;
    }

    let mut result = LoadPatchResult::None;

    egui::Window::new("Load Patch")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Select a patch to load:");
            ui.add_space(8.0);

            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    ui.heading("Built-in Patches");
                    for patch in example_patches() {
                        ui.horizontal(|ui| {
                            if ui.button(&patch.name).clicked() {
                                result = LoadPatchResult::LoadBuiltin(Box::new(patch.clone()));
                                *open = false;
                            }
                            if let Some(ref desc) = patch.description {
                                ui.label(
                                    RichText::new(desc).small().color(theme().colors.text_dim),
                                );
                            }
                        });
                    }
                });

            ui.add_space(16.0);
            if ui.button("Cancel").clicked() {
                result = LoadPatchResult::Cancelled;
                *open = false;
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
