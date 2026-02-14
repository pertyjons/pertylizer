//! AWE (Acoustic World Engine) view — placeholder for Fas 0.
//!
//! Displays an enable/disable toggle and basic status info.
//! Full room editor will be added in later phases.

use eframe::egui;

use crate::gui::app::state::AppView;
use crate::gui::theme::theme;
use synth_engine::{EngineCommand, EngineHandle};

/// Draw the AWE view.
///
/// Provides a toolbar with enable/disable toggle and a "Back to Rack" button,
/// plus a central panel with placeholder text.
pub fn draw_awe_view(
    ctx: &egui::Context,
    handle: &mut EngineHandle,
    awe_enabled: &mut bool,
    active_view: &mut AppView,
) {
    egui::TopBottomPanel::top("awe_toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("← Rack").clicked() {
                *active_view = AppView::Rack;
            }

            ui.separator();

            let toggle_label = if *awe_enabled { "AWE: ON" } else { "AWE: OFF" };
            let toggle_color = if *awe_enabled {
                theme().colors.meter_green
            } else {
                theme().colors.text_dim
            };
            if ui
                .button(egui::RichText::new(toggle_label).color(toggle_color))
                .clicked()
            {
                *awe_enabled = !*awe_enabled;
                handle.send(EngineCommand::SetAweEnabled {
                    enabled: *awe_enabled,
                });
            }
        });
    });

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.heading(
                egui::RichText::new("AWE — Acoustic World Engine")
                    .color(theme().colors.accent_cyan)
                    .size(24.0),
            );
            ui.add_space(20.0);

            let status = if *awe_enabled {
                "Enabled (pass-through in Fas 0)"
            } else {
                "Disabled"
            };
            ui.label(
                egui::RichText::new(status)
                    .color(theme().colors.text_dim)
                    .size(16.0),
            );

            ui.add_space(40.0);
            ui.label(
                egui::RichText::new("Room simulation controls will appear here in future phases.")
                    .color(theme().colors.text_dim),
            );
        });
    });
}
