//! Audio meter drawing utilities.
//!
//! This module provides functions for drawing peak and RMS audio level meters
//! in both vertical and horizontal orientations.

use eframe::egui::{self, Pos2, Stroke, Vec2};

use crate::gui::theme::theme;

/// Draw a vertical audio meter.
///
/// # Arguments
/// * `ui` - The egui UI context
/// * `peak` - Peak level (0.0 to 1.0+)
/// * `rms` - RMS level (0.0 to 1.0+)
/// * `width` - Meter width in pixels
/// * `height` - Meter height in pixels
#[allow(dead_code)]
pub fn draw_meter(ui: &mut egui::Ui, peak: f32, rms: f32, width: f32, height: f32) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 2.0, theme().colors.bg_dark);

    // RMS level
    let rms_height = rect.height() * rms.clamp(0.0, 1.0);
    let rms_rect = egui::Rect::from_min_size(
        Pos2::new(rect.left(), rect.bottom() - rms_height),
        Vec2::new(rect.width(), rms_height),
    );

    let rms_color = if rms > 0.9 {
        theme().colors.meter_red
    } else if rms > 0.7 {
        theme().colors.meter_yellow
    } else {
        theme().colors.meter_green
    };

    painter.rect_filled(rms_rect, 0.0, rms_color);

    // Peak indicator
    let peak_y = rect.bottom() - rect.height() * peak.clamp(0.0, 1.0);
    painter.line_segment(
        [
            Pos2::new(rect.left(), peak_y),
            Pos2::new(rect.right(), peak_y),
        ],
        Stroke::new(2.0, theme().colors.text_primary),
    );
}

/// Draw a horizontal audio meter.
///
/// # Arguments
/// * `ui` - The egui UI context
/// * `peak` - Peak level (0.0 to 1.0+)
/// * `rms` - RMS level (0.0 to 1.0+)
/// * `width` - Meter width in pixels
/// * `height` - Meter height in pixels
pub fn draw_meter_horizontal(ui: &mut egui::Ui, peak: f32, rms: f32, width: f32, height: f32) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 2.0, theme().colors.bg_dark);

    // RMS level (horizontal - grows from left)
    let rms_width = rect.width() * rms.clamp(0.0, 1.0);
    let rms_rect = egui::Rect::from_min_size(rect.min, Vec2::new(rms_width, rect.height()));

    let rms_color = if rms > 0.9 {
        theme().colors.meter_red
    } else if rms > 0.7 {
        theme().colors.meter_yellow
    } else {
        theme().colors.meter_green
    };

    painter.rect_filled(rms_rect, 0.0, rms_color);

    // Peak indicator (vertical line)
    let peak_x = rect.left() + rect.width() * peak.clamp(0.0, 1.0);
    painter.line_segment(
        [
            Pos2::new(peak_x, rect.top()),
            Pos2::new(peak_x, rect.bottom()),
        ],
        Stroke::new(2.0, theme().colors.text_primary),
    );
}
