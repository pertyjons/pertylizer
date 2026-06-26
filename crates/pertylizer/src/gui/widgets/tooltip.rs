//! Custom tooltip widget that renders on top of other widgets.

use eframe::egui::emath::TSTransform;
use eframe::egui::{self, Color32, LayerId, Order, Pos2, Stroke, Ui, Vec2};

use crate::gui::theme::theme;

/// Draw a custom tooltip at the specified position.
///
/// This tooltip is drawn on a foreground layer so it appears above all other widgets.
/// Use this instead of egui's built-in tooltips for consistent styling.
///
/// # Arguments
/// * `ui` - The UI context
/// * `pos` - Position for the tooltip (top-left corner), in the caller's layer
///   (world) coordinates
/// * `text` - Text to display
/// * `accent_color` - Color for the border accent
pub fn draw_value_tooltip(ui: &Ui, pos: Pos2, text: &str, accent_color: Color32) {
    // `pos` is a bare position in the caller's coordinates — project it to screen.
    render_tooltip_at_screen(ui, layer_transform(ui) * pos, text, accent_color);
}

/// Draw a tooltip to the right of a center point (useful for knobs).
pub fn draw_tooltip_right_of(
    ui: &Ui,
    center: Pos2,
    radius: f32,
    text: &str,
    accent_color: Color32,
) {
    // Project the knob centre to screen, then offset in SCREEN space. `radius` is
    // a world length, so it scales with the Scene zoom; the 6/10 px gaps are fixed
    // screen pixels. Computing the offset in world space (and letting the tooltip
    // layer scale it) would misplace the tooltip at any zoom != 1.
    let tr = layer_transform(ui);
    let anchor = tr * center;
    let pos = anchor + Vec2::new(radius * tr.scaling + 6.0, -10.0);
    render_tooltip_at_screen(ui, pos, text, accent_color);
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

    // Project the anchor point to screen, then apply the screen-pixel offsets
    // (the box itself is drawn at fixed screen size, so the gap must be too).
    let anchor = layer_transform(ui) * point;
    let pos = Pos2::new(anchor.x + 8.0, anchor.y - size.y - 4.0);
    render_tooltip_at_screen(ui, pos, text, accent_color);
}

/// The caller's layer→global (world→screen) transform. Inside an `egui::Scene`
/// (the patch editor) this carries the pan/zoom; outside a Scene the layer has no
/// transform, so this is the identity (the mixer knobs keep working unchanged).
fn layer_transform(ui: &Ui) -> TSTransform {
    ui.ctx()
        .layer_transform_to_global(ui.layer_id())
        .unwrap_or(TSTransform::IDENTITY)
}

/// Shared tooltip rendering on the screen-space tooltip layer. `pos` is already in
/// screen coordinates (callers project their anchor first).
fn render_tooltip_at_screen(ui: &Ui, pos: Pos2, text: &str, accent_color: Color32) {
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
