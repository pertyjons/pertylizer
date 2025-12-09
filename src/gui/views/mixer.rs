//! Mixer view - Channel strips and master section.
//!
//! This view will contain:
//! - Channel strips for each instrument
//! - Master fader and meters
//! - Send/return routing

use eframe::egui::{self, RichText};

use crate::gui::theme::theme;

/// Show the mixer view.
pub fn show(ctx: &egui::Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.heading("Mixer");
            ui.add_space(20.0);
            ui.label(
                RichText::new("Channel strips and master section coming soon...")
                    .color(theme().colors.text_dim),
            );
            ui.add_space(40.0);

            // Placeholder channel strips
            ui.horizontal(|ui| {
                for i in 1..=8 {
                    ui.group(|ui| {
                        ui.set_min_width(60.0);
                        ui.set_min_height(200.0);
                        ui.vertical_centered(|ui| {
                            ui.label(format!("CH{}", i));
                            ui.add_space(10.0);

                            // Placeholder fader
                            let mut level = 0.8;
                            ui.add(
                                egui::Slider::new(&mut level, 0.0..=1.0)
                                    .vertical()
                                    .show_value(false),
                            );

                            ui.add_space(10.0);
                            ui.label(RichText::new("0 dB").small());
                        });
                    });
                }
            });
        });
    });
}
