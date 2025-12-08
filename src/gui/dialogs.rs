//! Dialog windows for the synthesizer GUI.
//!
//! This module provides reusable dialog components for settings,
//! about information, and patch management.

use eframe::egui::{self, RichText};

use super::egui_backend::setup_custom_style;
use super::theme::{theme, ThemePreset};
use crate::patch::{Patch, example_patches};

/// State for all application dialogs.
#[derive(Debug, Default)]
pub struct DialogState {
    /// Show settings dialog.
    pub show_settings: bool,
    /// Show about dialog.
    pub show_about: bool,
    /// Show load patch dialog.
    pub show_load_patch: bool,
    /// Show save patch dialog.
    pub show_save_patch: bool,
    /// Name for saving patch.
    pub patch_save_name: String,
    /// Status message with timestamp.
    pub status_message: Option<(String, std::time::Instant)>,
    /// Currently selected theme preset.
    pub current_theme: ThemePreset,
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
}

/// Result from showing the load patch dialog.
pub enum LoadPatchResult {
    /// No action taken.
    None,
    /// User selected a built-in patch to load.
    LoadBuiltin(Box<Patch>),
    /// User cancelled.
    Cancelled,
}

/// Result from showing the save patch dialog.
pub enum SavePatchResult {
    /// No action taken.
    None,
    /// User wants to save with the given name.
    Save(String),
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
                                ui.label(RichText::new(desc).small().color(theme().colors.text_dim));
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

/// Show the save patch dialog.
///
/// Returns the action the user wants to take.
pub fn show_save_patch_dialog(
    ctx: &egui::Context,
    open: &mut bool,
    patch_name: &mut String,
) -> SavePatchResult {
    if !*open {
        return SavePatchResult::None;
    }

    let mut result = SavePatchResult::None;

    egui::Window::new("Save Patch")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label("Enter a name for your patch:");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(patch_name);
            });

            ui.add_space(16.0);
            ui.horizontal(|ui| {
                let can_save = !patch_name.trim().is_empty();
                if ui
                    .add_enabled(can_save, egui::Button::new("Save"))
                    .clicked()
                {
                    result = SavePatchResult::Save(patch_name.clone());
                    *open = false;
                }
                if ui.button("Cancel").clicked() {
                    result = SavePatchResult::Cancelled;
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
