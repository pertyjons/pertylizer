//! Cable drawing utilities with orthogonal routing and animated flow particles.
//!
//! Cables are drawn as right-angle lines (horizontal→vertical→horizontal)
//! matching the left-to-right signal flow of the auto-layout. Cables are
//! rendered behind modules and take the shortest orthogonal path.
//! Animated particles along the cables show signal direction and activity.

use eframe::egui::{Color32, Pos2, Stroke, Vec2};

use super::port::WidgetPortType;
use crate::gui::theme::theme;

// --- Cable palette ---
/// Drop shadow behind cables.
const CABLE_SHADOW: Color32 = Color32::from_rgba_unmultiplied_const(0, 0, 0, 40);

/// Horizontal clearance from port before turning vertical.
const CLEARANCE: f32 = 25.0;

/// Radius for rounded corners at bends.
const CORNER_RADIUS: f32 = 5.0;

/// Y-distance threshold below which a forward cable is drawn as a straight line.
const STRAIGHT_THRESHOLD: f32 = 8.0;

/// Vertical margin above/below modules when routing around them.
const BYPASS_MARGIN: f32 = 20.0;

/// Spacing between parallel cables (pixels per spread unit).
pub const CABLE_SPREAD: f32 = 8.0;

/// Get cable color from theme based on port type.
#[must_use]
pub fn cable_color(port_type: WidgetPortType, alpha: u8) -> Color32 {
    let base = match port_type {
        WidgetPortType::Audio => theme().colors.cable_audio,
        WidgetPortType::Control => theme().colors.cable_control,
        WidgetPortType::Gate => theme().colors.cable_gate,
        WidgetPortType::Midi => theme().colors.port_midi,
        WidgetPortType::NoteStream => theme().colors.accent_purple,
    };
    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha)
}

/// Calculate orthogonal route waypoints from output port to input port.
///
/// Output always exits RIGHT, input always enters from LEFT. Cables take
/// the shortest orthogonal path (they render behind modules so no obstacle
/// avoidance is needed).
///
/// `spread` offsets the vertical segment so parallel cables don't overlap.
/// Positive values shift right (forward) or lower (backward).
#[must_use]
fn calculate_route(from: Pos2, to: Pos2, spread: f32) -> Vec<Pos2> {
    let x_after_out = from.x + CLEARANCE;
    let x_before_in = to.x - CLEARANCE;

    if x_after_out < x_before_in {
        // Forward routing: destination is to the right of source.
        let dy = (to.y - from.y).abs();
        if dy < STRAIGHT_THRESHOLD {
            return vec![from, to];
        }

        // Vertical segment at midpoint, offset by spread.
        let mid_x = (x_after_out + x_before_in) * 0.5 + spread;
        vec![from, Pos2::new(mid_x, from.y), Pos2::new(mid_x, to.y), to]
    } else {
        // Backward or overlapping: U-shape above both endpoints.
        let right_x = from.x + CLEARANCE + spread;
        let left_x = to.x - CLEARANCE + spread;
        let bypass_y = from.y.min(to.y) - BYPASS_MARGIN - spread.abs();

        vec![
            from,
            Pos2::new(right_x, from.y),
            Pos2::new(right_x, bypass_y),
            Pos2::new(left_x, bypass_y),
            Pos2::new(left_x, to.y),
            to,
        ]
    }
}

// ── Drawing helpers ──────────────────────────────────────────────────

/// Total path length along a set of waypoints.
fn path_length(points: &[Pos2]) -> f32 {
    points
        .array_windows()
        .map(|&[a, b]| {
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            (dx * dx + dy * dy).sqrt()
        })
        .sum()
}

/// Map a distance along the path to an (x, y) coordinate.
fn point_at_distance(points: &[Pos2], dist: f32) -> Option<Pos2> {
    let mut remaining = dist;
    for &[a, b] in points.array_windows() {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let seg_len = (dx * dx + dy * dy).sqrt();
        if seg_len < 0.001 {
            continue;
        }
        if remaining <= seg_len {
            let t = remaining / seg_len;
            return Some(Pos2::new(a.x + dx * t, a.y + dy * t));
        }
        remaining -= seg_len;
    }
    points.last().copied()
}

/// Draw the orthogonal line segments with optional rounded corners.
fn draw_segments(
    painter: &eframe::egui::Painter,
    points: &[Pos2],
    stroke: Stroke,
    round_corners: bool,
) {
    if points.len() < 2 {
        return;
    }

    if !round_corners || points.len() <= 2 {
        for &[a, b] in points.array_windows() {
            painter.line_segment([a, b], stroke);
        }
        return;
    }

    // Draw segments with rounded corners at bends
    for i in 0..points.len() - 1 {
        let a = points[i];
        let b = points[i + 1];

        let start = if i > 0 {
            shorten_toward(a, b, CORNER_RADIUS)
        } else {
            a
        };
        let end = if i + 2 < points.len() {
            shorten_toward(b, a, CORNER_RADIUS)
        } else {
            b
        };

        painter.line_segment([start, end], stroke);

        if i + 2 < points.len() {
            let c = points[i + 2];
            draw_corner_arc(painter, b, a, c, stroke);
        }
    }
}

/// Move point `from` toward `to` by `amount`.
#[must_use]
fn shorten_toward(from: Pos2, to: Pos2, amount: f32) -> Pos2 {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < amount * 2.0 {
        return from;
    }
    let t = amount / len;
    Pos2::new(from.x + dx * t, from.y + dy * t)
}

/// Draw a small arc at a corner where the path bends.
fn draw_corner_arc(
    painter: &eframe::egui::Painter,
    corner: Pos2,
    prev: Pos2,
    next: Pos2,
    stroke: Stroke,
) {
    let r = CORNER_RADIUS;
    let to_prev = Vec2::new(prev.x - corner.x, prev.y - corner.y).normalized();
    let to_next = Vec2::new(next.x - corner.x, next.y - corner.y).normalized();

    let start = Pos2::new(corner.x + to_prev.x * r, corner.y + to_prev.y * r);
    let end = Pos2::new(corner.x + to_next.x * r, corner.y + to_next.y * r);

    let steps = 4;
    let mut prev_pt = start;
    for s in 1..=steps {
        let t = s as f32 / steps as f32;
        let pt = Pos2::new(
            start.x
                + (end.x - start.x) * t
                + (corner.x - (start.x + end.x) * 0.5) * 4.0 * t * (1.0 - t) * 0.4,
            start.y
                + (end.y - start.y) * t
                + (corner.y - (start.y + end.y) * 0.5) * 4.0 * t * (1.0 - t) * 0.4,
        );
        painter.line_segment([prev_pt, pt], stroke);
        prev_pt = pt;
    }
}

// ── Public API ───────────────────────────────────────────────────────

/// Draw a cable between two points with orthogonal routing and shadow.
pub fn draw_cable(
    painter: &eframe::egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    spread: f32,
) {
    let points = calculate_route(from, to, spread);
    let thickness = theme().sizes.cable_thickness;

    // Shadow
    let shadow_offset = Vec2::new(1.0, 2.0);
    let shadow_points: Vec<Pos2> = points.iter().map(|p| *p + shadow_offset).collect();
    let shadow_color = CABLE_SHADOW;
    draw_segments(
        painter,
        &shadow_points,
        Stroke::new(thickness + 1.0, shadow_color),
        false,
    );

    // Main cable
    let cable_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 160);
    draw_segments(painter, &points, Stroke::new(thickness, cable_color), true);
}

/// Draw a cable being dragged (simpler, no shadow).
pub fn draw_cable_dragging(painter: &eframe::egui::Painter, from: Pos2, to: Pos2, color: Color32) {
    let points = calculate_route(from, to, 0.0);
    let cable_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 160);
    draw_segments(
        painter,
        &points,
        Stroke::new(theme().sizes.cable_thickness, cable_color),
        true,
    );
}

/// Draw a highlighted cable (hovered) with glow effect using the cable's own color.
pub fn draw_cable_highlighted(
    painter: &eframe::egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    spread: f32,
) {
    let points = calculate_route(from, to, spread);
    let thickness = theme().sizes.cable_thickness;

    // Outer glow
    let glow_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 60);
    draw_segments(
        painter,
        &points,
        Stroke::new(thickness * 3.2, glow_color),
        false,
    );

    // Inner glow
    let inner_glow = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 120);
    draw_segments(
        painter,
        &points,
        Stroke::new(thickness * 2.0, inner_glow),
        false,
    );

    // Core cable (full brightness)
    let core = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 220);
    draw_segments(painter, &points, Stroke::new(thickness, core), true);
}

/// Draw animated flow particles along a cable.
pub fn draw_flow_particles(
    painter: &eframe::egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    port_type: WidgetPortType,
    time: f64,
    spread: f32,
) {
    let points = calculate_route(from, to, spread);
    let total_len = path_length(&points);
    if total_len < 1.0 {
        return;
    }

    let (speed, spacing, radius) = match port_type {
        WidgetPortType::Audio => (120.0_f64, 30.0_f64, 1.8_f32),
        WidgetPortType::Control => (60.0, 50.0, 1.8),
        WidgetPortType::Gate => (80.0, 60.0, 2.0),
        WidgetPortType::Midi | WidgetPortType::NoteStream => (70.0, 45.0, 1.8),
    };

    let particle_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 220);
    let total_len_f64 = f64::from(total_len);
    let num_particles = (total_len_f64 / spacing).ceil() as i32;
    let base_offset = (time * speed) % total_len_f64;

    for i in 0..num_particles {
        let dist = (base_offset + f64::from(i) * spacing) % total_len_f64;

        if let Some(pos) = point_at_distance(&points, dist as f32) {
            let draw_color = if port_type == WidgetPortType::Gate {
                let pulse = ((time * 6.0).sin() * 0.5 + 0.5) as f32;
                let alpha = (160.0 + pulse * 60.0) as u8;
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
            } else {
                particle_color
            };

            painter.circle_filled(pos, radius, draw_color);
        }
    }
}

/// Check if a point is near an orthogonal cable (for hit testing).
#[must_use]
pub fn point_near_cable(point: Pos2, from: Pos2, to: Pos2, threshold: f32, spread: f32) -> bool {
    let points = calculate_route(from, to, spread);

    for &[a, b] in points.array_windows() {
        if point_to_segment_distance(point, a, b) < threshold {
            return true;
        }
    }
    false
}

/// Find the closest point on an orthogonal cable to a given point.
#[must_use]
pub fn closest_point_on_cable(point: Pos2, from: Pos2, to: Pos2, spread: f32) -> Pos2 {
    let points = calculate_route(from, to, spread);
    let mut best = from;
    let mut best_dist = f32::MAX;

    for &[a, b] in points.array_windows() {
        let closest = closest_point_on_segment(point, a, b);
        let dx = point.x - closest.x;
        let dy = point.y - closest.y;
        let dist = dx * dx + dy * dy;
        if dist < best_dist {
            best_dist = dist;
            best = closest;
        }
    }
    best
}

/// Closest point on a line segment to a given point.
fn closest_point_on_segment(p: Pos2, a: Pos2, b: Pos2) -> Pos2 {
    let ab = Vec2::new(b.x - a.x, b.y - a.y);
    let ap = Vec2::new(p.x - a.x, p.y - a.y);

    let len_sq = ab.x * ab.x + ab.y * ab.y;
    if len_sq < 0.001 {
        return a;
    }

    let t = ((ap.x * ab.x + ap.y * ab.y) / len_sq).clamp(0.0, 1.0);
    Pos2::new(a.x + ab.x * t, a.y + ab.y * t)
}

/// Distance from a point to a line segment.
fn point_to_segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = Vec2::new(b.x - a.x, b.y - a.y);
    let ap = Vec2::new(p.x - a.x, p.y - a.y);

    let len_sq = ab.x * ab.x + ab.y * ab.y;
    if len_sq < 0.001 {
        return (ap.x * ap.x + ap.y * ap.y).sqrt();
    }

    let t = ((ap.x * ab.x + ap.y * ab.y) / len_sq).clamp(0.0, 1.0);
    let closest = Pos2::new(a.x + ab.x * t, a.y + ab.y * t);
    let dx = p.x - closest.x;
    let dy = p.y - closest.y;
    (dx * dx + dy * dy).sqrt()
}
