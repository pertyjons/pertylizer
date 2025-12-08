//! Module frame widget.

use eframe::egui::{self, Color32, Sense, Stroke, Ui, Vec2};

use crate::gui::theme::theme;

/// Draw a module frame with title and accent color.
pub fn module_frame(
    ui: &mut Ui,
    title: &str,
    accent_color: Color32,
    add_contents: impl FnOnce(&mut Ui),
) {
    egui::Frame::new()
        .fill(theme().colors.bg_module)
        .corner_radius(8.0)
        .stroke(Stroke::new(1.0, accent_color.gamma_multiply(0.5)))
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Colored accent bar
                let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 16.0), Sense::hover());
                ui.painter().rect_filled(rect, 2.0, accent_color);

                ui.label(egui::RichText::new(title).color(accent_color).strong());
            });

            ui.add_space(8.0);
            add_contents(ui);
        });
}
