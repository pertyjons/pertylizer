//! Sample waveform display widget.
//!
//! Displays a pre-computed waveform overview for samples.
//! Supports both mono and stereo visualization.

use eframe::egui::{Color32, Pos2, Sense, Stroke, Ui, Vec2};

use crate::gui::theme::theme;
use synth_core::WaveformOverview;

/// Draw a sample waveform display.
///
/// # Arguments
/// * `ui` - The egui UI context
/// * `overview` - The pre-computed waveform overview
/// * `playback_position` - Current playback position (0.0 to 1.0), or None
/// * `loop_region` - Optional loop region as (start, end) normalized (0.0 to 1.0)
/// * `width` - Display width in pixels
/// * `height` - Display height in pixels
/// * `color` - Waveform color
pub fn draw_sample_waveform(
    ui: &mut Ui,
    overview: &WaveformOverview,
    playback_position: Option<f32>,
    loop_region: Option<(f32, f32)>,
    width: f32,
    height: f32,
    color: Color32,
) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 4.0, theme().colors.bg_dark);

    // Draw loop region if enabled
    if let Some((loop_start, loop_end)) = loop_region {
        let loop_start = loop_start.clamp(0.0, 1.0);
        let loop_end = loop_end.clamp(0.0, 1.0);

        if loop_start < loop_end {
            // Semi-transparent loop region background
            let loop_x_start = rect.left() + loop_start * width;
            let loop_x_end = rect.left() + loop_end * width;
            let loop_rect = eframe::egui::Rect::from_min_max(
                Pos2::new(loop_x_start, rect.top()),
                Pos2::new(loop_x_end, rect.bottom()),
            );
            let loop_color = Color32::from_rgba_unmultiplied(100, 180, 100, 40);
            painter.rect_filled(loop_rect, 0.0, loop_color);

            // Loop start marker (dashed vertical line)
            let marker_color = Color32::from_rgb(100, 200, 100);
            draw_dashed_vline(
                painter,
                loop_x_start,
                rect.top(),
                rect.bottom(),
                marker_color,
            );

            // Loop end marker
            draw_dashed_vline(painter, loop_x_end, rect.top(), rect.bottom(), marker_color);

            // Small labels at bottom
            let label_color = Color32::from_rgba_premultiplied(100, 200, 100, 180);
            painter.text(
                Pos2::new(loop_x_start + 2.0, rect.bottom() - 10.0),
                eframe::egui::Align2::LEFT_BOTTOM,
                "L",
                eframe::egui::FontId::monospace(8.0),
                label_color,
            );
            painter.text(
                Pos2::new(loop_x_end - 2.0, rect.bottom() - 10.0),
                eframe::egui::Align2::RIGHT_BOTTOM,
                "L",
                eframe::egui::FontId::monospace(8.0),
                label_color,
            );
        }
    }

    // Grid color
    let grid_color = Color32::from_rgba_premultiplied(60, 65, 75, 100);

    if overview.is_stereo {
        // Stereo: draw left channel in top half, right in bottom half
        let half_height = height / 2.0;
        let center_l = rect.top() + half_height / 2.0;
        let center_r = rect.top() + half_height + half_height / 2.0;

        // Horizontal center lines for each channel
        painter.line_segment(
            [
                Pos2::new(rect.left(), center_l),
                Pos2::new(rect.right(), center_l),
            ],
            Stroke::new(1.0, grid_color),
        );
        painter.line_segment(
            [
                Pos2::new(rect.left(), center_r),
                Pos2::new(rect.right(), center_r),
            ],
            Stroke::new(1.0, grid_color),
        );

        // Divider line between channels
        let divider_y = rect.top() + half_height;
        painter.line_segment(
            [
                Pos2::new(rect.left(), divider_y),
                Pos2::new(rect.right(), divider_y),
            ],
            Stroke::new(1.0, theme().colors.border),
        );

        // Draw left channel (top half)
        draw_channel_waveform(
            painter,
            &overview.peaks_left,
            rect.left(),
            center_l,
            width,
            half_height * 0.9 / 2.0,
            color,
        );

        // Draw right channel (bottom half)
        draw_channel_waveform(
            painter,
            &overview.peaks_right,
            rect.left(),
            center_r,
            width,
            half_height * 0.9 / 2.0,
            color,
        );

        // Channel labels
        let label_color = Color32::from_rgba_premultiplied(150, 150, 150, 180);
        painter.text(
            Pos2::new(rect.left() + 4.0, rect.top() + 2.0),
            eframe::egui::Align2::LEFT_TOP,
            "L",
            eframe::egui::FontId::monospace(10.0),
            label_color,
        );
        painter.text(
            Pos2::new(rect.left() + 4.0, divider_y + 2.0),
            eframe::egui::Align2::LEFT_TOP,
            "R",
            eframe::egui::FontId::monospace(10.0),
            label_color,
        );
    } else {
        // Mono: draw centered in full height
        let center_y = rect.center().y;

        // Horizontal center line
        painter.line_segment(
            [
                Pos2::new(rect.left(), center_y),
                Pos2::new(rect.right(), center_y),
            ],
            Stroke::new(1.0, grid_color),
        );

        // Draw mono waveform
        draw_channel_waveform(
            painter,
            &overview.peaks_left,
            rect.left(),
            center_y,
            width,
            (height - 8.0) / 2.0,
            color,
        );
    }

    // Draw playback position indicator
    if let Some(pos) = playback_position
        && (0.0..=1.0).contains(&pos)
    {
        let x = rect.left() + pos * width;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(2.0, Color32::from_rgb(255, 200, 50)),
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

/// Draw a dashed vertical line.
fn draw_dashed_vline(
    painter: &eframe::egui::Painter,
    x: f32,
    y_start: f32,
    y_end: f32,
    color: Color32,
) {
    let dash_length = 4.0;
    let gap_length = 3.0;
    let mut y = y_start;

    while y < y_end {
        let y_next = (y + dash_length).min(y_end);
        painter.line_segment(
            [Pos2::new(x, y), Pos2::new(x, y_next)],
            Stroke::new(1.5, color),
        );
        y = y_next + gap_length;
    }
}

/// Draw a single channel's waveform as filled area between min/max peaks.
fn draw_channel_waveform(
    painter: &eframe::egui::Painter,
    peaks: &[(synth_core::SampleValue, synth_core::SampleValue)],
    x_start: f32,
    center_y: f32,
    width: f32,
    half_height: f32,
    color: Color32,
) {
    if peaks.is_empty() {
        // Draw flat line for empty waveform
        painter.line_segment(
            [
                Pos2::new(x_start + 4.0, center_y),
                Pos2::new(x_start + width - 4.0, center_y),
            ],
            Stroke::new(1.0, color.gamma_multiply(0.5)),
        );
        return;
    }

    let num_peaks = peaks.len();

    // Draw filled waveform using vertical lines for each peak
    for (i, &(min_val, max_val)) in peaks.iter().enumerate() {
        let x = x_start + (i as f32 / num_peaks as f32) * width;
        let y_min = center_y - min_val.as_f32().clamp(-1.0, 1.0) * half_height;
        let y_max = center_y - max_val.as_f32().clamp(-1.0, 1.0) * half_height;

        // Draw vertical line from min to max
        painter.line_segment(
            [Pos2::new(x, y_min), Pos2::new(x, y_max)],
            Stroke::new(1.0, color),
        );
    }

    // Draw center line highlight (where waveform crosses zero)
    let zero_line_color = color.gamma_multiply(0.3);
    painter.line_segment(
        [
            Pos2::new(x_start, center_y),
            Pos2::new(x_start + width, center_y),
        ],
        Stroke::new(0.5, zero_line_color),
    );
}

/// Draw a compact sample waveform for list views.
///
/// Smaller version suitable for instrument list items or compact views.
pub fn draw_sample_waveform_compact(
    ui: &mut Ui,
    overview: &WaveformOverview,
    width: f32,
    height: f32,
    color: Color32,
) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 2.0, theme().colors.bg_dark);

    // Draw mono representation (combine stereo to mono for compact view)
    let center_y = rect.center().y;
    let half_height = (height - 4.0) / 2.0;

    if !overview.peaks_left.is_empty() {
        let num_peaks = overview.peaks_left.len();

        for (i, &(min_l, max_l)) in overview.peaks_left.iter().enumerate() {
            let x = rect.left() + (i as f32 / num_peaks as f32) * width;

            // For stereo, average left and right
            let (min_val, max_val) = if overview.is_stereo {
                if let Some(&(min_r, max_r)) = overview.peaks_right.get(i) {
                    (
                        (min_l.as_f32() + min_r.as_f32()) * 0.5,
                        (max_l.as_f32() + max_r.as_f32()) * 0.5,
                    )
                } else {
                    (min_l.as_f32(), max_l.as_f32())
                }
            } else {
                (min_l.as_f32(), max_l.as_f32())
            };

            let y_min = center_y - min_val.clamp(-1.0, 1.0) * half_height;
            let y_max = center_y - max_val.clamp(-1.0, 1.0) * half_height;

            painter.line_segment(
                [Pos2::new(x, y_min), Pos2::new(x, y_max)],
                Stroke::new(1.0, color),
            );
        }
    }

    // Border
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, theme().colors.border),
        eframe::egui::StrokeKind::Outside,
    );
}
