//! Audio level meter widgets.

use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use crate::gui::theme::{Colors, theme};

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
        let t = theme();
        let desired_size = Vec2::new(self.width, self.height);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());

        let painter = ui.painter();

        // Background
        painter.rect_filled(rect, t.style.corner_radius_small, t.colors.bg_dark);

        // RMS level
        let rms_height = rect.height() * self.rms.clamp(0.0, 1.0);
        let rms_rect = Rect::from_min_size(
            Pos2::new(rect.left(), rect.bottom() - rms_height),
            Vec2::new(rect.width(), rms_height),
        );

        let rms_color = if self.rms > 0.9 {
            t.colors.meter_red
        } else if self.rms > 0.7 {
            t.colors.meter_yellow
        } else {
            t.colors.meter_green
        };

        painter.rect_filled(rms_rect, 0.0, rms_color);

        // Peak indicator
        let peak_y = rect.bottom() - rect.height() * self.peak.clamp(0.0, 1.0);
        painter.line_segment(
            [
                Pos2::new(rect.left(), peak_y),
                Pos2::new(rect.right(), peak_y),
            ],
            Stroke::new(t.style.border_width_thick, t.colors.text_primary),
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
    let t = theme();
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, t.style.corner_radius_small, t.colors.bg_dark);

    // Map magnitude to 0-1 range over -60dB to 0dB
    let peak_norm = synth_core::magnitude_to_normalized_db(peak, -60.0);
    let rms_norm = synth_core::magnitude_to_normalized_db(rms, -60.0);

    let padding = 2.0;
    let inner_rect = rect.shrink(padding);

    if horizontal {
        // RMS bar
        let rms_width = inner_rect.width() * rms_norm;
        let rms_rect =
            Rect::from_min_size(inner_rect.min, Vec2::new(rms_width, inner_rect.height()));

        // Gradient from green to yellow to red
        let rms_color = level_color(rms_norm, &t.colors);
        painter.rect_filled(rms_rect, t.style.border_width, rms_color);

        // Peak indicator line
        let peak_x = inner_rect.left() + inner_rect.width() * peak_norm;
        painter.line_segment(
            [
                Pos2::new(peak_x, inner_rect.top()),
                Pos2::new(peak_x, inner_rect.bottom()),
            ],
            Stroke::new(t.style.border_width_thick, t.colors.text_primary),
        );
    } else {
        // Vertical meter
        // RMS bar (from bottom up)
        let rms_height = inner_rect.height() * rms_norm;
        let rms_rect = Rect::from_min_max(
            Pos2::new(inner_rect.left(), inner_rect.bottom() - rms_height),
            inner_rect.max,
        );

        let rms_color = level_color(rms_norm, &t.colors);
        painter.rect_filled(rms_rect, t.style.border_width, rms_color);

        // Peak indicator line
        let peak_y = inner_rect.bottom() - inner_rect.height() * peak_norm;
        painter.line_segment(
            [
                Pos2::new(inner_rect.left(), peak_y),
                Pos2::new(inner_rect.right(), peak_y),
            ],
            Stroke::new(t.style.border_width_thick, t.colors.text_primary),
        );
    }

    // Clip indicator at top
    if peak_norm >= 1.0 {
        let clip_rect = if horizontal {
            Rect::from_min_size(
                Pos2::new(rect.right() - 6.0, rect.top()),
                Vec2::new(6.0, rect.height()),
            )
        } else {
            Rect::from_min_size(rect.min, Vec2::new(rect.width(), 4.0))
        };
        painter.rect_filled(clip_rect, t.style.border_width, t.colors.meter_red);
    }

    // Border
    painter.rect_stroke(
        rect,
        t.style.corner_radius_small,
        Stroke::new(t.style.border_width, t.colors.border),
        egui::StrokeKind::Outside,
    );
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
    let t = theme();
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, t.style.corner_radius, t.colors.bg_dark);

    let padding = 2.0;
    // Left gutter reserved for the dB scale ruler.
    let scale_w = 24.0;
    // Bottom band reserved for the L/R channel labels (matches draw_meter_bar,
    // which leaves 14px at the bottom of each bar).
    let label_band = 14.0;

    let bars_left = rect.left() + padding + scale_w;
    let bars_right = rect.right() - padding;
    let bars_w = (bars_right - bars_left).max(2.0);
    let bar_gap = 6.0;
    let bar_width = ((bars_w - bar_gap) / 2.0).max(1.0);

    // Left meter
    let left_rect = Rect::from_min_size(
        Pos2::new(bars_left, rect.top() + padding),
        Vec2::new(bar_width, height - padding * 2.0),
    );
    draw_meter_bar(painter, left_rect, peak_l, rms_l, &t);

    // Right meter
    let right_rect = Rect::from_min_size(
        Pos2::new(bars_left + bar_width + bar_gap, rect.top() + padding),
        Vec2::new(bar_width, height - padding * 2.0),
    );
    draw_meter_bar(painter, right_rect, peak_r, rms_r, &t);

    // dB scale ruler in the left gutter. Ticks align with the bars' segment area,
    // which spans [rect.top()+padding, rect.bottom()-padding-label_band] (norm 1..0)
    // — the same -60dB..0dB mapping draw_meter_bar uses for its segments.
    let seg_top = rect.top() + padding;
    let seg_bottom = rect.bottom() - padding - label_band;
    let seg_h = (seg_bottom - seg_top).max(1.0);
    for &db in &[0.0_f32, -6.0, -12.0, -24.0, -48.0] {
        let norm = (db + 60.0) / 60.0;
        let y = seg_bottom - norm * seg_h;
        painter.line_segment(
            [Pos2::new(bars_left - 4.0, y), Pos2::new(bars_left, y)],
            Stroke::new(0.5, t.colors.border),
        );
        painter.text(
            Pos2::new(bars_left - 6.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{}", db as i32),
            t.fonts.small(),
            t.colors.text_dim,
        );
    }

    // Labels
    let label_y = rect.bottom() - 12.0;
    painter.text(
        Pos2::new(left_rect.center().x, label_y),
        egui::Align2::CENTER_CENTER,
        "L",
        t.fonts.small(),
        t.colors.text_dim,
    );
    painter.text(
        Pos2::new(right_rect.center().x, label_y),
        egui::Align2::CENTER_CENTER,
        "R",
        t.fonts.small(),
        t.colors.text_dim,
    );

    // Border
    painter.rect_stroke(
        rect,
        t.style.corner_radius,
        Stroke::new(t.style.border_width, t.colors.border),
        egui::StrokeKind::Outside,
    );
}

/// Helper to draw a single meter bar.
fn draw_meter_bar(
    painter: &egui::Painter,
    rect: Rect,
    peak: f32,
    rms: f32,
    t: &crate::gui::theme::Theme,
) {
    // Calculate levels over -60dB to 0dB
    let peak_norm = synth_core::magnitude_to_normalized_db(peak, -60.0);
    let rms_norm = synth_core::magnitude_to_normalized_db(rms, -60.0);

    // Background
    painter.rect_filled(rect, t.style.border_width, t.colors.bg_dark);

    // Draw segments
    let num_segments = t.style.meter_segments;
    let segment_height = (rect.height() - 14.0) / num_segments as f32;
    let gap = t.style.meter_segment_gap;

    for i in 0..num_segments {
        let level = (i + 1) as f32 / num_segments as f32;
        let y = rect.bottom() - 14.0 - (i + 1) as f32 * segment_height;

        let segment_rect = Rect::from_min_size(
            Pos2::new(rect.left() + t.style.border_width, y + gap),
            Vec2::new(
                rect.width() - t.style.border_width * 2.0,
                segment_height - gap * 2.0,
            ),
        );

        let color = if level <= rms_norm {
            level_color(level, &t.colors)
        } else {
            // Inactive segment - use the widget background from the theme
            t.colors.bg_widget
        };

        painter.rect_filled(segment_rect, 0.0, color);
    }

    // Peak indicator
    if peak_norm > 0.01 {
        let segment_area_height = rect.height() - 14.0;
        let peak_y = rect.bottom() - 14.0 - segment_area_height * peak_norm;
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.left() + t.style.border_width, peak_y),
                Vec2::new(
                    rect.width() - t.style.border_width * 2.0,
                    t.style.border_width_thick,
                ),
            ),
            0.0,
            t.colors.text_primary,
        );
    }
}

/// Get color for level value (green -> yellow -> red) from the theme palette.
/// Takes `&Colors` so callers that already hold the theme don't re-acquire the
/// global theme lock per meter segment.
pub fn level_color(level: f32, colors: &Colors) -> Color32 {
    if level < 0.6 {
        colors.meter_green
    } else if level < 0.85 {
        colors.meter_yellow
    } else {
        colors.meter_red
    }
}
