//! Custom tooltip widget that renders on top of other widgets.

use eframe::egui::{self, Color32, LayerId, Order, Pos2, Stroke, Ui, Vec2};

use crate::gui::theme::theme;

/// Draw a custom tooltip at the specified position.
///
/// This tooltip is drawn on a foreground layer so it appears above all other widgets.
/// Use this instead of egui's built-in tooltips for consistent styling.
///
/// # Arguments
/// * `ui` - The UI context
/// * `pos` - Position for the tooltip (top-left corner)
/// * `text` - Text to display
/// * `accent_color` - Color for the border accent
pub fn draw_value_tooltip(ui: &Ui, pos: Pos2, text: &str, accent_color: Color32) {
    render_tooltip(ui, pos, text, accent_color);
}

/// Draw a tooltip to the right of a center point (useful for knobs).
pub fn draw_tooltip_right_of(
    ui: &Ui,
    center: Pos2,
    radius: f32,
    text: &str,
    accent_color: Color32,
) {
    let pos = Pos2::new(center.x + radius + 6.0, center.y - 10.0);
    draw_value_tooltip(ui, pos, text, accent_color);
}

/// Draw a tooltip above a point (useful for envelope control points).
pub fn draw_tooltip_above(ui: &Ui, point: Pos2, text: &str, accent_color: Color32) {
    let t = theme();
    let painter = ui
        .ctx()
        .layer_painter(LayerId::new(Order::Tooltip, egui::Id::new("value_tooltip")));

    let galley = painter.layout_no_wrap(text.to_string(), t.fonts.small(), t.colors.text_primary);
    let padding = Vec2::new(6.0, 3.0);
    let size = galley.size() + padding * 2.0;

    // Position above and slightly to the right
    let pos = Pos2::new(point.x + 8.0, point.y - size.y - 4.0);
    render_tooltip(ui, pos, text, accent_color);
}

/// Shared tooltip rendering logic: background, border, and text on the tooltip layer.
fn render_tooltip(ui: &Ui, pos: Pos2, text: &str, accent_color: Color32) {
    let t = theme();
    let painter = ui
        .ctx()
        .layer_painter(LayerId::new(Order::Tooltip, egui::Id::new("value_tooltip")));

    let bg = t.colors.bg_dark;
    let tooltip_bg = Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), 240);

    let galley = painter.layout_no_wrap(text.to_string(), t.fonts.small(), t.colors.text_primary);
    let padding = Vec2::new(6.0, 3.0);
    let text_rect = egui::Rect::from_min_size(pos, galley.size() + padding * 2.0);

    // Background
    painter.rect_filled(text_rect, t.style.corner_radius_small, tooltip_bg);

    // Border with accent color
    painter.rect_stroke(
        text_rect,
        t.style.corner_radius_small,
        Stroke::new(t.style.border_width, accent_color.gamma_multiply(0.6)),
        egui::StrokeKind::Inside,
    );

    // Text
    painter.galley(pos + padding, galley, t.colors.text_primary);
}
