//! Audio level meter widgets.

use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use super::{colors, theme};

/// A vertical meter widget for audio levels.
pub struct Meter {
    peak: f32,
    rms: f32,
    width: f32,
    height: f32,
}

impl Meter {
    pub fn new(peak: f32, rms: f32) -> Self {
        Self {
            peak,
            rms,
            width: 12.0,
            height: 100.0,
        }
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let desired_size = Vec2::new(self.width, self.height);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());

        let painter = ui.painter();

        // Background
        painter.rect_filled(rect, 2.0, colors::BG_DARK);

        // RMS level
        let rms_height = rect.height() * self.rms.clamp(0.0, 1.0);
        let rms_rect = Rect::from_min_size(
            Pos2::new(rect.left(), rect.bottom() - rms_height),
            Vec2::new(rect.width(), rms_height),
        );

        let rms_color = if self.rms > 0.9 {
            colors::METER_RED
        } else if self.rms > 0.7 {
            colors::METER_YELLOW
        } else {
            colors::METER_GREEN
        };

        painter.rect_filled(rms_rect, 0.0, rms_color);

        // Peak indicator
        let peak_y = rect.bottom() - rect.height() * self.peak.clamp(0.0, 1.0);
        painter.line_segment(
            [
                Pos2::new(rect.left(), peak_y),
                Pos2::new(rect.right(), peak_y),
            ],
            Stroke::new(2.0, colors::TEXT_PRIMARY),
        );

        response
    }
}

/// Draw a level meter (VU meter style).
/// Shows peak and RMS levels with optional peak hold.
pub fn draw_level_meter(
    ui: &mut Ui,
    peak: f32,
    rms: f32,
    width: f32,
    height: f32,
    horizontal: bool,
) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 2.0, colors::BG_DARK);

    // Calculate bar positions
    let peak_db = 20.0 * peak.max(0.0001).log10();
    let rms_db = 20.0 * rms.max(0.0001).log10();

    // Map dB to 0-1 range (-60dB to 0dB)
    let peak_norm = ((peak_db + 60.0) / 60.0).clamp(0.0, 1.0);
    let rms_norm = ((rms_db + 60.0) / 60.0).clamp(0.0, 1.0);

    let padding = 2.0;
    let inner_rect = rect.shrink(padding);

    if horizontal {
        // RMS bar
        let rms_width = inner_rect.width() * rms_norm;
        let rms_rect = Rect::from_min_size(
            inner_rect.min,
            Vec2::new(rms_width, inner_rect.height()),
        );

        // Gradient from green to yellow to red
        let rms_color = level_color(rms_norm);
        painter.rect_filled(rms_rect, 1.0, rms_color);

        // Peak indicator line
        let peak_x = inner_rect.left() + inner_rect.width() * peak_norm;
        painter.line_segment(
            [Pos2::new(peak_x, inner_rect.top()), Pos2::new(peak_x, inner_rect.bottom())],
            Stroke::new(2.0, colors::TEXT_PRIMARY),
        );
    } else {
        // Vertical meter
        // RMS bar (from bottom up)
        let rms_height = inner_rect.height() * rms_norm;
        let rms_rect = Rect::from_min_max(
            Pos2::new(inner_rect.left(), inner_rect.bottom() - rms_height),
            inner_rect.max,
        );

        let rms_color = level_color(rms_norm);
        painter.rect_filled(rms_rect, 1.0, rms_color);

        // Peak indicator line
        let peak_y = inner_rect.bottom() - inner_rect.height() * peak_norm;
        painter.line_segment(
            [Pos2::new(inner_rect.left(), peak_y), Pos2::new(inner_rect.right(), peak_y)],
            Stroke::new(2.0, colors::TEXT_PRIMARY),
        );
    }

    // Clip indicator at top
    if peak_norm > 0.98 {
        let clip_rect = if horizontal {
            Rect::from_min_size(
                Pos2::new(rect.right() - 6.0, rect.top()),
                Vec2::new(6.0, rect.height()),
            )
        } else {
            Rect::from_min_size(
                rect.min,
                Vec2::new(rect.width(), 4.0),
            )
        };
        painter.rect_filled(clip_rect, 1.0, Color32::RED);
    }

    // Border
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, colors::BORDER), egui::StrokeKind::Outside);
}

/// Draw a stereo level meter (two vertical bars side by side).
pub fn draw_stereo_meter(
    ui: &mut Ui,
    peak_l: f32,
    peak_r: f32,
    rms_l: f32,
    rms_r: f32,
    width: f32,
    height: f32,
) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 4.0, colors::BG_DARK);

    let bar_width = (width - 10.0) / 2.0;
    let padding = 2.0;

    // Left meter
    let left_rect = Rect::from_min_size(
        Pos2::new(rect.left() + padding, rect.top() + padding),
        Vec2::new(bar_width, height - padding * 2.0),
    );
    draw_meter_bar(painter, left_rect, peak_l, rms_l);

    // Right meter
    let right_rect = Rect::from_min_size(
        Pos2::new(rect.left() + bar_width + 6.0, rect.top() + padding),
        Vec2::new(bar_width, height - padding * 2.0),
    );
    draw_meter_bar(painter, right_rect, peak_r, rms_r);

    // Labels
    let t = theme();
    let label_y = rect.bottom() - 12.0;
    painter.text(
        Pos2::new(left_rect.center().x, label_y),
        egui::Align2::CENTER_CENTER,
        "L",
        t.fonts.small(),
        colors::TEXT_DIM,
    );
    painter.text(
        Pos2::new(right_rect.center().x, label_y),
        egui::Align2::CENTER_CENTER,
        "R",
        t.fonts.small(),
        colors::TEXT_DIM,
    );

    // Border
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, colors::BORDER), egui::StrokeKind::Outside);
}

/// Helper to draw a single meter bar.
fn draw_meter_bar(painter: &egui::Painter, rect: Rect, peak: f32, rms: f32) {
    // Calculate levels
    let peak_db = 20.0 * peak.max(0.0001).log10();
    let rms_db = 20.0 * rms.max(0.0001).log10();

    let peak_norm = ((peak_db + 60.0) / 60.0).clamp(0.0, 1.0);
    let rms_norm = ((rms_db + 60.0) / 60.0).clamp(0.0, 1.0);

    // Background
    painter.rect_filled(rect, 1.0, Color32::from_rgb(25, 28, 35));

    // Draw segments
    let num_segments = 20;
    let segment_height = (rect.height() - 14.0) / num_segments as f32;
    let gap = 1.0;

    for i in 0..num_segments {
        let level = (i + 1) as f32 / num_segments as f32;
        let y = rect.bottom() - 14.0 - (i + 1) as f32 * segment_height;

        let segment_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 1.0, y + gap),
            Vec2::new(rect.width() - 2.0, segment_height - gap * 2.0),
        );

        let color = if level <= rms_norm {
            level_color(level)
        } else {
            Color32::from_rgb(35, 38, 45)
        };

        painter.rect_filled(segment_rect, 0.0, color);
    }

    // Peak indicator
    if peak_norm > 0.01 {
        let peak_y = rect.bottom() - 14.0 - rect.height() * peak_norm + 7.0;
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.left() + 1.0, peak_y),
                Vec2::new(rect.width() - 2.0, 2.0),
            ),
            0.0,
            colors::TEXT_PRIMARY,
        );
    }
}

/// Get color for level value (green -> yellow -> red).
pub fn level_color(level: f32) -> Color32 {
    if level < 0.6 {
        // Green
        Color32::from_rgb(80, 200, 120)
    } else if level < 0.85 {
        // Yellow
        Color32::from_rgb(220, 180, 50)
    } else {
        // Red
        Color32::from_rgb(220, 80, 80)
    }
}
