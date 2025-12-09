//! Cable drawing utilities with gravity, shadows, and theme colors.

use eframe::egui::{Color32, Pos2, Stroke};

use super::port::PortType;
use crate::gui::theme::theme;

/// Get cable color from theme based on port type.
pub fn cable_color(port_type: PortType, alpha: u8) -> Color32 {
    let base = match port_type {
        PortType::Audio => theme().colors.cable_audio,
        PortType::Control => theme().colors.cable_control,
        PortType::Gate => theme().colors.cable_gate,
        PortType::Midi => theme().colors.port_midi,
    };
    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha)
}

/// Calculate bezier control points with gravity (sag).
/// Returns (ctrl1, ctrl2) for a cubic bezier curve.
fn bezier_control_points_with_sag(from: Pos2, to: Pos2, sag_factor: f32) -> (Pos2, Pos2) {
    let dx = (to.x - from.x).abs();
    let dy = to.y - from.y;

    // Horizontal control offset (how "wide" the curve is)
    let control_offset = dx * 0.4;

    // Calculate sag - cable hangs down due to gravity
    // Sag is proportional to horizontal distance, capped at max_sag
    let max_sag = 100.0;
    let sag = (dx * sag_factor).min(max_sag);

    // If destination is below source, reduce sag effect
    let sag_adjustment = if dy > 0.0 { sag * 0.5 } else { sag };

    let ctrl1 = Pos2::new(from.x + control_offset, from.y + sag_adjustment);
    let ctrl2 = Pos2::new(to.x - control_offset, to.y + sag_adjustment);

    (ctrl1, ctrl2)
}

/// Evaluate cubic bezier at parameter t.
#[inline]
fn bezier_point(from: Pos2, ctrl1: Pos2, ctrl2: Pos2, to: Pos2, t: f32) -> Pos2 {
    let t2 = t * t;
    let t3 = t2 * t;
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let mt3 = mt2 * mt;

    let x = mt3 * from.x + 3.0 * mt2 * t * ctrl1.x + 3.0 * mt * t2 * ctrl2.x + t3 * to.x;
    let y = mt3 * from.y + 3.0 * mt2 * t * ctrl1.y + 3.0 * mt * t2 * ctrl2.y + t3 * to.y;

    Pos2::new(x, y)
}

/// Draw a bezier cable with the given control points.
fn draw_bezier_cable(
    painter: &eframe::egui::Painter,
    from: Pos2,
    ctrl1: Pos2,
    ctrl2: Pos2,
    to: Pos2,
    stroke: Stroke,
) {
    let segments = 32;
    let mut prev = from;

    for i in 1..=segments {
        let t = i as f32 / segments as f32;
        let curr = bezier_point(from, ctrl1, ctrl2, to, t);
        painter.line_segment([prev, curr], stroke);
        prev = curr;
    }
}

/// Draw a cable between two points with gravity (sag), shadow, and optional highlight.
pub fn draw_cable(painter: &eframe::egui::Painter, from: Pos2, to: Pos2, color: Color32) {
    // Control points with 15% sag
    let (ctrl1, ctrl2) = bezier_control_points_with_sag(from, to, 0.15);

    // Shadow (offset down and slightly right)
    let shadow_offset = Pos2::new(2.0, 3.0);
    let shadow_color = Color32::from_rgba_unmultiplied(0, 0, 0, 50);
    draw_bezier_cable(
        painter,
        from + shadow_offset.to_vec2(),
        ctrl1 + shadow_offset.to_vec2(),
        ctrl2 + shadow_offset.to_vec2(),
        to + shadow_offset.to_vec2(),
        Stroke::new(4.0, shadow_color),
    );

    // Main cable with semi-transparency
    let cable_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 200);
    draw_bezier_cable(
        painter,
        from,
        ctrl1,
        ctrl2,
        to,
        Stroke::new(3.0, cable_color),
    );

    // Subtle highlight (thinner, lighter line on top)
    let highlight_color = Color32::from_rgba_unmultiplied(255, 255, 255, 30);
    draw_bezier_cable(
        painter,
        from + Pos2::new(0.0, -0.5).to_vec2(),
        ctrl1 + Pos2::new(0.0, -0.5).to_vec2(),
        ctrl2 + Pos2::new(0.0, -0.5).to_vec2(),
        to + Pos2::new(0.0, -0.5).to_vec2(),
        Stroke::new(1.0, highlight_color),
    );
}

/// Draw a cable being dragged (less sag, no shadow for responsiveness).
pub fn draw_cable_dragging(painter: &eframe::egui::Painter, from: Pos2, to: Pos2, color: Color32) {
    // Control points with reduced sag (5%) for more responsive feel
    let (ctrl1, ctrl2) = bezier_control_points_with_sag(from, to, 0.05);

    // Semi-transparent cable, no shadow for cleaner dragging
    let cable_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 180);
    draw_bezier_cable(
        painter,
        from,
        ctrl1,
        ctrl2,
        to,
        Stroke::new(3.0, cable_color),
    );
}

/// Draw a highlighted cable (hovered or selected) with glow effect.
pub fn draw_cable_highlighted(painter: &eframe::egui::Painter, from: Pos2, to: Pos2) {
    let highlight_color = theme().colors.accent_red;

    // Control points with standard sag
    let (ctrl1, ctrl2) = bezier_control_points_with_sag(from, to, 0.15);

    // Outer glow
    let glow_color = Color32::from_rgba_unmultiplied(
        highlight_color.r(),
        highlight_color.g(),
        highlight_color.b(),
        60,
    );
    draw_bezier_cable(
        painter,
        from,
        ctrl1,
        ctrl2,
        to,
        Stroke::new(8.0, glow_color),
    );

    // Inner glow
    let inner_glow = Color32::from_rgba_unmultiplied(
        highlight_color.r(),
        highlight_color.g(),
        highlight_color.b(),
        120,
    );
    draw_bezier_cable(
        painter,
        from,
        ctrl1,
        ctrl2,
        to,
        Stroke::new(5.0, inner_glow),
    );

    // Core cable
    draw_bezier_cable(
        painter,
        from,
        ctrl1,
        ctrl2,
        to,
        Stroke::new(3.0, highlight_color),
    );
}

/// Check if a point is near a bezier cable (for hit testing).
/// Uses the same control points as draw_cable.
pub fn point_near_cable(point: Pos2, from: Pos2, to: Pos2, threshold: f32) -> bool {
    let (ctrl1, ctrl2) = bezier_control_points_with_sag(from, to, 0.15);

    // Sample points along the curve and check distance
    let segments = 16;
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        let curve_point = bezier_point(from, ctrl1, ctrl2, to, t);
        let dist = ((point.x - curve_point.x).powi(2) + (point.y - curve_point.y).powi(2)).sqrt();
        if dist < threshold {
            return true;
        }
    }
    false
}
