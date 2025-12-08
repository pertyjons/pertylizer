//! Oscilloscope display widget.

use eframe::egui::{Color32, Pos2, Sense, Stroke, Ui, Vec2};

use crate::gui::theme::theme;

/// Draw an oscilloscope display.
/// Shows a waveform visualization with optional grid.
pub fn draw_oscilloscope(
    ui: &mut Ui,
    samples: &[f32],
    width: f32,
    height: f32,
    gain: f32,
    color: Color32,
) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 4.0, theme().colors.bg_dark);

    // Draw grid
    let grid_color = Color32::from_rgba_premultiplied(60, 65, 75, 100);

    // Horizontal center line
    let center_y = rect.center().y;
    painter.line_segment(
        [
            Pos2::new(rect.left(), center_y),
            Pos2::new(rect.right(), center_y),
        ],
        Stroke::new(1.0, grid_color),
    );

    // Vertical grid lines
    for i in 1..4 {
        let x = rect.left() + rect.width() * (i as f32 / 4.0);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, grid_color),
        );
    }

    // Draw waveform
    if samples.len() >= 2 {
        let step = samples.len() as f32 / width;
        let half_height = (height - 8.0) / 2.0;

        let mut points: Vec<Pos2> = Vec::with_capacity(width as usize);

        for x in 0..(width as usize) {
            let sample_idx = ((x as f32) * step) as usize;
            if sample_idx < samples.len() {
                let sample = samples[sample_idx] * gain;
                let y = center_y - sample.clamp(-1.0, 1.0) * half_height;
                points.push(Pos2::new(rect.left() + x as f32, y));
            }
        }

        // Draw as connected line segments
        for i in 0..points.len().saturating_sub(1) {
            painter.line_segment([points[i], points[i + 1]], Stroke::new(1.5, color));
        }
    } else {
        // No data - draw flat line
        painter.line_segment(
            [
                Pos2::new(rect.left() + 4.0, center_y),
                Pos2::new(rect.right() - 4.0, center_y),
            ],
            Stroke::new(1.5, color.gamma_multiply(0.5)),
        );
    }

    // Border
    painter.rect_stroke(
        rect,
        4.0,
        Stroke::new(1.0, theme().colors.border),
        eframe::egui::StrokeKind::Outside,
    );
}
