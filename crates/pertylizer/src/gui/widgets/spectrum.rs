//! Spectrum analyzer display widget.

use eframe::egui::{Color32, Pos2, Sense, Shape, Stroke, Ui, Vec2};
use eframe::epaint::PathShape;

use crate::gui::theme::theme;

/// Draw a spectrum analyzer display.
/// Shows frequency spectrum with logarithmic x-axis (20Hz-20kHz) and dB y-axis.
///
/// `magnitudes` should be FFT magnitude bins (normalized 0.0-1.0),
/// `gain` scales the vertical display.
/// `sample_rate` is the audio sample rate in Hz used to map FFT bins to frequencies.
pub fn draw_spectrum_analyzer(
    ui: &mut Ui,
    magnitudes: &[f32],
    width: f32,
    height: f32,
    gain: f32,
    color: Color32,
    sample_rate: f32,
) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let t = theme();
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, t.style.corner_radius, t.colors.bg_dark);

    // Draw grid lines
    let grid_color = t.colors.border;
    let text_color = t.colors.text_dim;

    // Horizontal grid lines at -20dB intervals (mapped to 0.0-1.0: 0.8, 0.6, 0.4, 0.2)
    for i in 1..5 {
        let y_frac = i as f32 / 5.0;
        let y = rect.top() + y_frac * rect.height();
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(0.5, grid_color),
        );
    }

    // Vertical grid lines at decade frequencies: 100Hz, 1kHz, 10kHz
    // Using log scale: log10(freq/20) / log10(20000/20)
    let log_range = (20000.0_f32 / 20.0).log10(); // ~3.0
    let label_font = eframe::egui::FontId::proportional(8.0);
    let mut last_label_right = f32::NEG_INFINITY;
    for &freq in &[100.0_f32, 1000.0, 10000.0] {
        let x_frac = (freq / 20.0).log10() / log_range;
        let x = rect.left() + x_frac * rect.width();
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(0.5, grid_color),
        );
        // Frequency label — skip if it would overlap the previous one
        let label = if freq >= 1000.0 {
            format!("{}k", freq as u32 / 1000)
        } else {
            format!("{}", freq as u32)
        };
        let label_width = label.len() as f32 * 5.0;
        let label_left = x + 2.0;
        if label_left > last_label_right + 4.0 {
            painter.text(
                Pos2::new(label_left, rect.bottom() - 10.0),
                eframe::egui::Align2::LEFT_BOTTOM,
                label,
                label_font.clone(),
                text_color,
            );
            last_label_right = label_left + label_width;
        }
    }

    // Draw spectrum as filled area
    if magnitudes.len() >= 2 {
        let num_bins = magnitudes.len();
        // Map bins to log-frequency x positions using the provided sample rate
        let mut points: Vec<Pos2> = Vec::with_capacity(width as usize + 2);

        // Start from bottom-left
        points.push(Pos2::new(rect.left(), rect.bottom()));

        // Sample spectrum at each pixel column
        for px in 0..(width as usize) {
            let x_frac = px as f32 / width;
            // Convert pixel to frequency (log scale)
            let freq = 20.0 * 10.0_f32.powf(x_frac * log_range);

            // Find corresponding bin
            let fft_size = (num_bins - 1) * 2;
            let bin_f = freq * fft_size as f32 / sample_rate;
            let bin = (bin_f as usize).min(num_bins - 1);

            // Interpolate between bins for smoother display
            let frac = bin_f - bin as f32;
            let mag = if bin + 1 < num_bins {
                magnitudes[bin] * (1.0 - frac) + magnitudes[bin + 1] * frac
            } else {
                magnitudes[bin]
            };

            let y = rect.bottom() - (mag * gain).clamp(0.0, 1.0) * (rect.height() - 4.0);
            points.push(Pos2::new(rect.left() + px as f32, y));
        }

        // Close the shape at bottom-right
        points.push(Pos2::new(rect.right(), rect.bottom()));

        // Draw filled area using PathShape (the spectrum is non-convex)
        let fill_color =
            Color32::from_rgba_premultiplied(color.r() / 3, color.g() / 3, color.b() / 3, 80);
        painter.add(Shape::Path(PathShape {
            points: points.clone(),
            closed: true,
            fill: fill_color,
            stroke: Stroke::NONE.into(),
        }));

        // Draw outline on top (skip first and last points which are the bottom corners)
        if points.len() > 2 {
            let line_points: Vec<Pos2> = points[1..points.len() - 1].to_vec();
            painter.add(Shape::line(line_points, Stroke::new(1.5, color)));
        }
    } else {
        // No data - draw flat line at bottom
        painter.line_segment(
            [
                Pos2::new(rect.left() + 4.0, rect.bottom() - 4.0),
                Pos2::new(rect.right() - 4.0, rect.bottom() - 4.0),
            ],
            Stroke::new(1.5, color.gamma_multiply(0.5)),
        );
    }

    // Border
    painter.rect_stroke(
        rect,
        t.style.corner_radius,
        Stroke::new(t.style.border_width, t.colors.border),
        eframe::egui::StrokeKind::Outside,
    );
}
