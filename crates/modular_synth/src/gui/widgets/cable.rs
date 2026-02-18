//! Cable drawing utilities with orthogonal routing and animated flow particles.
//!
//! Cables are drawn as right-angle lines (horizontal→vertical→horizontal)
//! matching the left-to-right signal flow of the auto-layout. The routing
//! algorithm avoids crossing module rectangles by placing vertical segments
//! in the gaps between module columns. Animated particles along the cables
//! show signal direction and activity.

use eframe::egui::{Color32, Pos2, Rect, Stroke, Vec2};

use super::port::WidgetPortType;
use crate::gui::theme::theme;

/// Horizontal clearance from port before turning vertical.
const CLEARANCE: f32 = 25.0;

/// Radius for rounded corners at bends.
const CORNER_RADIUS: f32 = 5.0;

/// Y-distance threshold below which a forward cable is drawn as a straight line.
const STRAIGHT_THRESHOLD: f32 = 8.0;

/// Vertical margin above/below modules when routing around them.
const BYPASS_MARGIN: f32 = 20.0;

/// Get cable color from theme based on port type.
#[must_use]
pub fn cable_color(port_type: WidgetPortType, alpha: u8) -> Color32 {
    let base = match port_type {
        WidgetPortType::Audio => theme().colors.cable_audio,
        WidgetPortType::Control => theme().colors.cable_control,
        WidgetPortType::Gate => theme().colors.cable_gate,
        WidgetPortType::Midi => theme().colors.port_midi,
    };
    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha)
}

/// Calculate orthogonal route waypoints from output port to input port,
/// avoiding all module rectangles.
///
/// Output always exits RIGHT, input always enters from LEFT. The vertical
/// cable segment is placed in a gap between module columns so it never
/// crosses a module.
#[must_use]
fn calculate_route(from: Pos2, to: Pos2, obstacles: &[Rect]) -> Vec<Pos2> {
    let x_after_out = from.x + CLEARANCE;
    let x_before_in = to.x - CLEARANCE;

    if x_after_out < x_before_in {
        // Forward routing: destination is to the right of source.
        let dy = (to.y - from.y).abs();
        if dy < STRAIGHT_THRESHOLD {
            return vec![from, to];
        }

        // Find a clear x for the vertical segment in the gap between modules.
        let preferred_x = (x_after_out + x_before_in) * 0.5;
        let y_min = from.y.min(to.y);
        let y_max = from.y.max(to.y);
        let turn_x = find_clear_x(
            preferred_x,
            x_after_out,
            x_before_in,
            y_min,
            y_max,
            obstacles,
        );

        if let Some(x) = turn_x {
            vec![from, Pos2::new(x, from.y), Pos2::new(x, to.y), to]
        } else {
            // No clear vertical gap — route above all intermediate modules.
            route_over_obstacles(from, to, x_after_out, x_before_in, obstacles)
        }
    } else {
        // Backward or overlapping: route around both modules above.
        let right_x = from.x + CLEARANCE;
        let left_x = to.x - CLEARANCE;
        let y_min = from.y.min(to.y);
        let bypass_y = find_bypass_y_above(left_x, right_x, y_min, obstacles);

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

/// Find an x-coordinate in `[x_min, x_max]` for a vertical cable segment
/// that doesn't cross any obstacle in the y-range `[y_min, y_max]`.
///
/// Starts from `preferred` and searches outward. Returns `None` if the
/// entire x-range is blocked.
fn find_clear_x(
    preferred: f32,
    x_min: f32,
    x_max: f32,
    y_min: f32,
    y_max: f32,
    obstacles: &[Rect],
) -> Option<f32> {
    // Collect obstacles that overlap the cable's y-range and x-range
    let blocking: Vec<&Rect> = obstacles
        .iter()
        .filter(|r| {
            r.top() <= y_max && r.bottom() >= y_min && r.left() < x_max && r.right() > x_min
        })
        .collect();

    if blocking.is_empty() {
        return Some(preferred);
    }

    // Check if preferred x is clear
    if !is_x_blocked(preferred, &blocking) {
        return Some(preferred);
    }

    // Build sorted list of gap intervals between blocking rects
    let mut edges: Vec<f32> = Vec::new();
    edges.push(x_min);
    for r in &blocking {
        edges.push(r.left());
        edges.push(r.right());
    }
    edges.push(x_max);
    edges.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    edges.dedup();

    // Find gaps (intervals where no blocking rect covers)
    let mut best: Option<f32> = None;
    let mut best_dist = f32::MAX;

    for pair in edges.windows(2) {
        let mid = (pair[0] + pair[1]) * 0.5;
        if mid < x_min || mid > x_max {
            continue;
        }
        if !is_x_blocked(mid, &blocking) {
            let dist = (mid - preferred).abs();
            if dist < best_dist {
                best_dist = dist;
                best = Some(mid);
            }
        }
    }

    best
}

/// Check if a vertical line at `x` is blocked by any of the given rects.
fn is_x_blocked(x: f32, rects: &[&Rect]) -> bool {
    rects.iter().any(|r| x >= r.left() && x <= r.right())
}

/// Route a cable over all intermediate obstacles using an inverted-U shape.
///
/// Goes up from the source, across above all obstacles, then down to the
/// destination.
fn route_over_obstacles(
    from: Pos2,
    to: Pos2,
    x_after_out: f32,
    x_before_in: f32,
    obstacles: &[Rect],
) -> Vec<Pos2> {
    let bypass_y = find_bypass_y_above(x_after_out, x_before_in, from.y.min(to.y), obstacles);

    vec![
        from,
        Pos2::new(x_after_out, from.y),
        Pos2::new(x_after_out, bypass_y),
        Pos2::new(x_before_in, bypass_y),
        Pos2::new(x_before_in, to.y),
        to,
    ]
}

/// Find a y-coordinate above all obstacles in the x-range `[x_min, x_max]`
/// that is also above `y_ref`.
fn find_bypass_y_above(x_min: f32, x_max: f32, y_ref: f32, obstacles: &[Rect]) -> f32 {
    let (lo, hi) = if x_min < x_max {
        (x_min, x_max)
    } else {
        (x_max, x_min)
    };

    let min_top = obstacles
        .iter()
        .filter(|r| r.right() > lo && r.left() < hi)
        .map(|r| r.top())
        .fold(y_ref, f32::min);

    min_top - BYPASS_MARGIN
}

// ── Drawing helpers ──────────────────────────────────────────────────

/// Total path length along a set of waypoints.
fn path_length(points: &[Pos2]) -> f32 {
    points
        .windows(2)
        .map(|w| {
            let dx = w[1].x - w[0].x;
            let dy = w[1].y - w[0].y;
            (dx * dx + dy * dy).sqrt()
        })
        .sum()
}

/// Map a distance along the path to an (x, y) coordinate.
fn point_at_distance(points: &[Pos2], dist: f32) -> Option<Pos2> {
    let mut remaining = dist;
    for w in points.windows(2) {
        let dx = w[1].x - w[0].x;
        let dy = w[1].y - w[0].y;
        let seg_len = (dx * dx + dy * dy).sqrt();
        if seg_len < 0.001 {
            continue;
        }
        if remaining <= seg_len {
            let t = remaining / seg_len;
            return Some(Pos2::new(w[0].x + dx * t, w[0].y + dy * t));
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
        for w in points.windows(2) {
            painter.line_segment([w[0], w[1]], stroke);
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

/// Draw a cable between two points with orthogonal routing, shadow, and highlight.
/// Avoids crossing any module in `obstacles`.
pub fn draw_cable(
    painter: &eframe::egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    obstacles: &[Rect],
) {
    let points = calculate_route(from, to, obstacles);

    // Shadow
    let shadow_offset = Vec2::new(1.0, 2.0);
    let shadow_points: Vec<Pos2> = points.iter().map(|p| *p + shadow_offset).collect();
    let shadow_color = Color32::from_rgba_unmultiplied(0, 0, 0, 40);
    draw_segments(
        painter,
        &shadow_points,
        Stroke::new(3.5, shadow_color),
        false,
    );

    // Main cable
    let cable_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 160);
    draw_segments(painter, &points, Stroke::new(2.5, cable_color), true);
}

/// Draw a cable being dragged (simpler, no shadow, no obstacle avoidance).
pub fn draw_cable_dragging(painter: &eframe::egui::Painter, from: Pos2, to: Pos2, color: Color32) {
    let points = calculate_route(from, to, &[]);
    let cable_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 160);
    draw_segments(painter, &points, Stroke::new(2.5, cable_color), true);
}

/// Draw a highlighted cable (hovered) with glow effect using the cable's own color.
pub fn draw_cable_highlighted(
    painter: &eframe::egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    obstacles: &[Rect],
) {
    let points = calculate_route(from, to, obstacles);

    // Outer glow
    let glow_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 60);
    draw_segments(painter, &points, Stroke::new(8.0, glow_color), false);

    // Inner glow
    let inner_glow = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 120);
    draw_segments(painter, &points, Stroke::new(5.0, inner_glow), false);

    // Core cable (full brightness)
    let core = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 220);
    draw_segments(painter, &points, Stroke::new(2.5, core), true);
}

/// Draw animated flow particles along a cable.
pub fn draw_flow_particles(
    painter: &eframe::egui::Painter,
    from: Pos2,
    to: Pos2,
    color: Color32,
    port_type: WidgetPortType,
    time: f64,
    obstacles: &[Rect],
) {
    let points = calculate_route(from, to, obstacles);
    let total_len = path_length(&points);
    if total_len < 1.0 {
        return;
    }

    let (speed, spacing, radius) = match port_type {
        WidgetPortType::Audio => (120.0_f64, 30.0_f64, 1.8_f32),
        WidgetPortType::Control => (60.0, 50.0, 1.8),
        WidgetPortType::Gate => (80.0, 60.0, 2.0),
        WidgetPortType::Midi => (70.0, 45.0, 1.8),
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
pub fn point_near_cable(
    point: Pos2,
    from: Pos2,
    to: Pos2,
    threshold: f32,
    obstacles: &[Rect],
) -> bool {
    let points = calculate_route(from, to, obstacles);

    for w in points.windows(2) {
        if point_to_segment_distance(point, w[0], w[1]) < threshold {
            return true;
        }
    }
    false
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
