//! Oscilloscope display widget.

use eframe::egui::{Color32, Pos2, Sense, Shape, Stroke, Ui, Vec2};

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

        // Draw as single optimized line shape (batched GPU draw call)
        if points.len() >= 2 {
            painter.add(Shape::line(points, Stroke::new(1.5, color)));
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

/// Draw an oscilloscope display with an optional trigger level line.
///
/// Same as `draw_oscilloscope` but draws a thin horizontal line at the
/// trigger threshold when `trigger_level` is `Some`.
pub fn draw_oscilloscope_with_trigger(
    ui: &mut Ui,
    samples: &[f32],
    width: f32,
    height: f32,
    gain: f32,
    color: Color32,
    trigger_level: Option<f32>,
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

    // Draw trigger level line
    if let Some(level) = trigger_level {
        // Map trigger level (0.0-1.0) to bipolar (-1.0 to 1.0) then to screen Y
        let bipolar = level * 2.0 - 1.0;
        let half_height = (height - 8.0) / 2.0;
        let trigger_y = center_y - bipolar.clamp(-1.0, 1.0) * half_height;
        let trigger_color = Color32::from_rgba_premultiplied(255, 200, 50, 120);
        painter.line_segment(
            [
                Pos2::new(rect.left(), trigger_y),
                Pos2::new(rect.right(), trigger_y),
            ],
            Stroke::new(1.0, trigger_color),
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

        // Draw as single optimized line shape (batched GPU draw call)
        if points.len() >= 2 {
            painter.add(Shape::line(points, Stroke::new(1.5, color)));
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
