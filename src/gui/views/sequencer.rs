//! Sequencer view - Pattern and song sequencing.
//!
//! This view will contain:
//! - Pattern editor (piano roll / tracker)
//! - Song arrangement
//! - Transport controls

use eframe::egui::{self, RichText};

use crate::gui::theme::theme;

/// Show the sequencer view.
pub fn show(ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.heading("Sequencer");
            ui.add_space(20.0);
            ui.label(
                RichText::new("Pattern editor and song arrangement coming soon...")
                    .color(theme().colors.text_dim),
            );
            ui.add_space(40.0);

            // Placeholder transport
            ui.horizontal(|ui| {
                ui.add_space(ui.available_width() / 2.0 - 80.0);
                if ui.button(RichText::new("").size(24.0)).clicked() {
                    // Rewind
                }
                if ui.button(RichText::new("").size(24.0)).clicked() {
                    // Play
                }
                if ui.button(RichText::new("").size(24.0)).clicked() {
                    // Stop
                }
                if ui.button(RichText::new("").size(24.0)).clicked() {
                    // Record
                }
            });
        });
    });
}
