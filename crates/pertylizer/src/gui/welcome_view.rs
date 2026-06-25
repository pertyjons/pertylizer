//! Welcome / empty-state landing view shown in the Rack `CentralPanel` when no
//! instruments exist yet.
//!
//! Replaces the bare "Select or create an instrument..." label with discoverable
//! action buttons that map to the existing File-menu items, so the user can
//! start a session without first knowing about the "+ New Instrument" button.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, RichText};
use egui_remixicon::icons as ri;

use crate::gui::theme::{Theme, theme};

/// Action the user picked from the welcome screen.
pub enum WelcomeAction {
    /// Create a new empty instrument.
    NewInstrument,
    /// Open the "Open Project…" file dialog.
    OpenProject,
    /// Open the "Open Patch…" file dialog.
    OpenPatch,
    /// Show the built-in patch browser.
    LoadBuiltinPatch,
    /// Open the "Import Sample…" file dialog.
    ImportSample,
    /// Load a project from the recent-projects list.
    OpenRecent(PathBuf),
}

/// Draw the welcome screen and return the action the user picked, if any.
#[must_use]
pub fn show(ui: &mut egui::Ui, t: &Theme, recent_projects: &[PathBuf]) -> Option<WelcomeAction> {
    let mut action: Option<WelcomeAction> = None;

    ui.vertical_centered(|ui| {
        ui.add_space(32.0);

        ui.add(
            egui::Image::new(egui::include_image!("../../assets/images/pertylizer.png"))
                .fit_to_exact_size(egui::vec2(640.0, 640.0)),
        );

        ui.add_space(theme().spacing.md);
        ui.label(
            RichText::new(format!(
                "v{}  ·  {}",
                env!("CARGO_PKG_VERSION"),
                env!("BUILD_DATE")
            ))
            .size(t.fonts.size_small)
            .color(t.colors.text_dim),
        );

        ui.add_space(24.0);
        ui.label(
            RichText::new("Get started by creating an instrument or loading a project.")
                .size(t.fonts.size_normal)
                .color(t.colors.text_secondary),
        );

        ui.add_space(28.0);

        // Primary actions row
        ui.horizontal(|ui| {
            ui.add_space(button_row_indent(ui));
            if primary_button(ui, t, ri::ADD_LINE, "New Instrument", t.colors.accent_green) {
                action = Some(WelcomeAction::NewInstrument);
            }
            ui.add_space(theme().spacing.md);
            if primary_button(
                ui,
                t,
                ri::FOLDER_OPEN_LINE,
                "Open Project…",
                t.colors.accent_cyan,
            ) {
                action = Some(WelcomeAction::OpenProject);
            }
            ui.add_space(theme().spacing.md);
            if primary_button(
                ui,
                t,
                ri::FOLDER_OPEN_LINE,
                "Open Patch…",
                t.colors.accent_cyan,
            ) {
                action = Some(WelcomeAction::OpenPatch);
            }
        });

        ui.add_space(10.0);

        // Secondary actions row
        ui.horizontal(|ui| {
            ui.add_space(button_row_indent(ui));
            if secondary_button(ui, t, ri::FILE_LIST_LINE, "Load Built-in Patch…") {
                action = Some(WelcomeAction::LoadBuiltinPatch);
            }
            ui.add_space(theme().spacing.md);
            if secondary_button(ui, t, ri::MUSIC_FILL, "Import Sample…") {
                action = Some(WelcomeAction::ImportSample);
            }
        });

        if !recent_projects.is_empty() {
            ui.add_space(36.0);
            ui.label(
                RichText::new("Recent Projects")
                    .size(t.fonts.size_normal)
                    .color(t.colors.text_secondary)
                    .strong(),
            );
            ui.add_space(theme().spacing.sm);

            for path in recent_projects.iter().take(5) {
                if recent_button(ui, t, path) {
                    action = Some(WelcomeAction::OpenRecent(path.clone()));
                }
            }
        }
    });

    action
}

/// Indent that roughly centers the button row when egui's `vertical_centered`
/// can't size a horizontal layout itself.
fn button_row_indent(ui: &egui::Ui) -> f32 {
    // Leave the centering to vertical_centered; an explicit 0 keeps buttons
    // left-anchored within the centered column without extra padding.
    let _ = ui;
    0.0
}

fn primary_button(ui: &mut egui::Ui, t: &Theme, icon: &str, label: &str, accent: Color32) -> bool {
    let icon = RichText::new(icon)
        .size(t.fonts.size_normal)
        .color(accent)
        .strong();
    let label = RichText::new(label)
        .size(t.fonts.size_normal)
        .color(accent)
        .strong();
    ui.add_sized([180.0, 36.0], egui::Button::new((icon, label)))
        .clicked()
}

fn secondary_button(ui: &mut egui::Ui, t: &Theme, icon: &str, label: &str) -> bool {
    let icon = RichText::new(icon)
        .size(t.fonts.size_normal)
        .color(t.colors.text_primary);
    let label = RichText::new(label)
        .size(t.fonts.size_normal)
        .color(t.colors.text_primary);
    ui.add_sized([180.0, 30.0], egui::Button::new((icon, label)))
        .clicked()
}

fn recent_button(ui: &mut egui::Ui, t: &Theme, path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("(unnamed)");
    let icon = RichText::new(ri::HISTORY_LINE)
        .size(t.fonts.size_normal)
        .color(t.colors.accent_cyan);
    let label = RichText::new(name)
        .size(t.fonts.size_normal)
        .color(t.colors.accent_cyan);
    ui.add_sized([280.0, 26.0], egui::Button::new((icon, label)))
        .on_hover_text(path.display().to_string())
        .clicked()
}
