//! Automation-lane zone under the piano roll: point hit-testing and the
//! draggable automation curve editor. Called by `piano_roll::draw_piano_roll`.

use super::*;

/// Find automation point at the given position (within hit radius).
pub(super) fn automation_point_at_pos(
    lane: &AutomationLaneSnapshot,
    pos: Pos2,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
    auto_y: f32,
) -> Option<usize> {
    let value_to_y =
        |val: f32| -> f32 { auto_y + AUTOMATION_ZONE_HEIGHT * (1.0 - val.clamp(0.0, 1.0)) };

    for (i, pt) in lane.points.iter().enumerate() {
        let px = tick_to_x(pt.tick);
        let py = value_to_y(pt.value.as_f32());
        let dist = ((pos.x - px).powi(2) + (pos.y - py).powi(2)).sqrt();
        if dist <= AUTOMATION_HIT_RADIUS {
            return Some(i);
        }
    }
    None
}

/// Draw one automation lane's zone. Zones are stacked (one per lane); the
/// caller positions each at its own `auto_y`. The zone shows a dimmed caption
/// naming its `target`, and the currently-selected (edit-focused) lane is
/// highlighted so it reads as the one the tools act on.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_automation_zone(
    painter: &egui::Painter,
    data: &PianoRollData,
    view_state: &SequencerViewState,
    selected_target: &AutomationTarget,
    grid_x: f32,
    auto_y: f32,
    grid_width: f32,
    viewport_left: f32,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
    t: &crate::gui::theme::Theme,
    is_selected: bool,
) {
    // The edit-focused lane draws in full accent; the others are dimmed so the
    // focused lane stands out (but every lane's curve stays legible).
    let auto_color = if is_selected {
        AUTOMATION_ORANGE
    } else {
        AUTOMATION_ORANGE.gamma_multiply(0.55)
    };

    // Background — the focused lane gets a subtly brighter fill.
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(grid_x, auto_y),
            Vec2::new(grid_width, AUTOMATION_ZONE_HEIGHT),
        ),
        0.0,
        if is_selected {
            AUTOMATION_ZONE_BG.gamma_multiply(1.6)
        } else {
            AUTOMATION_ZONE_BG
        },
    );

    // Separator line
    painter.line_segment(
        [
            Pos2::new(grid_x, auto_y),
            Pos2::new(grid_x + grid_width, auto_y),
        ],
        Stroke::new(1.0, t.colors.border),
    );

    // Every zone's target long-form name, pinned to the visible left edge of the
    // lane (clamped so it never drifts left of the lane's own start) so it stays
    // on screen no matter how far the pattern is scrolled — it used to sit at the
    // far right and was only visible when the whole pattern fit. Dimmed and drawn
    // behind the curve; the "AUT" + short-type gutter tag is drawn by the pinned
    // keyboard strip in `draw_pr_keyboard_gutter`.
    let label_x = viewport_left.max(grid_x) + 4.0;
    painter.text(
        Pos2::new(label_x, auto_y + 2.0),
        egui::Align2::LEFT_TOP,
        selected_target.display_name(),
        egui::FontId::proportional(9.0),
        t.colors
            .text_dim
            .gamma_multiply(if is_selected { 1.0 } else { 0.7 }),
    );

    // Reference lines (25%, 50%, 75%)
    for frac in [0.25, 0.5, 0.75] {
        let ry = auto_y + AUTOMATION_ZONE_HEIGHT * (1.0 - frac);
        painter.line_segment(
            [Pos2::new(grid_x, ry), Pos2::new(grid_x + grid_width, ry)],
            Stroke::new(0.3, t.colors.border.gamma_multiply(0.3)),
        );
    }

    // Coordinate helpers
    let value_to_y =
        |val: f32| -> f32 { auto_y + AUTOMATION_ZONE_HEIGHT * (1.0 - val.clamp(0.0, 1.0)) };

    // Find the lane matching the selected target
    let lane = data
        .automation_lanes
        .iter()
        .find(|l| l.target == *selected_target);

    if let Some(lane) = lane {
        let points = &lane.points;

        if !points.is_empty() {
            // Draw flat extension before first point
            if let Some(first) = points.first() {
                let first_x = tick_to_x(first.tick);
                if first_x > grid_x {
                    let y = value_to_y(first.value.as_f32());
                    painter.line_segment(
                        [Pos2::new(grid_x, y), Pos2::new(first_x, y)],
                        Stroke::new(1.0, auto_color.gamma_multiply(0.5)),
                    );
                }
            }

            // Draw curves between consecutive points
            for [from, to] in points.array_windows() {
                let x_start = tick_to_x(from.tick);
                let x_end = tick_to_x(to.tick);
                let pixel_width = (x_end - x_start).max(1.0);

                // Sample the curve pixel by pixel
                let steps = (pixel_width as u32).max(2);
                let mut prev_pos = Pos2::new(x_start, value_to_y(from.value.as_f32()));

                for step in 1..=steps {
                    #[allow(clippy::cast_precision_loss)]
                    let frac = step as f32 / steps as f32;
                    let x = x_start + frac * (x_end - x_start);
                    let val =
                        from.curve
                            .interpolate(from.value, to.value, NormalizedValue::new(frac));
                    let y = value_to_y(val.as_f32());
                    let cur_pos = Pos2::new(x, y);

                    painter.line_segment([prev_pos, cur_pos], Stroke::new(1.5, auto_color));
                    prev_pos = cur_pos;
                }
            }

            // Draw flat extension after last point
            if let Some(last) = points.last() {
                let last_x = tick_to_x(last.tick);
                let grid_end_x = grid_x + grid_width;
                if last_x < grid_end_x {
                    let y = value_to_y(last.value.as_f32());
                    painter.line_segment(
                        [Pos2::new(last_x, y), Pos2::new(grid_end_x, y)],
                        Stroke::new(1.0, auto_color.gamma_multiply(0.5)),
                    );
                }
            }

            // Draw points
            for pt in points {
                let px = tick_to_x(pt.tick);
                let py = value_to_y(pt.value.as_f32());
                painter.circle_filled(Pos2::new(px, py), AUTOMATION_POINT_RADIUS, auto_color);
                painter.circle_stroke(
                    Pos2::new(px, py),
                    AUTOMATION_POINT_RADIUS,
                    Stroke::new(1.0, Color32::WHITE),
                );
            }
        }
    }

    // Draw drag preview ghost point
    if let Some(DragState::DragAutomationPoint {
        current_tick,
        current_value,
        target,
        ..
    }) = &view_state.drag
        && target == selected_target
    {
        let px = tick_to_x(*current_tick);
        let py = value_to_y(current_value.as_f32());
        painter.circle_filled(
            Pos2::new(px, py),
            AUTOMATION_POINT_RADIUS + 1.0,
            AUTOMATION_ORANGE_FILL,
        );
        painter.circle_stroke(
            Pos2::new(px, py),
            AUTOMATION_POINT_RADIUS + 1.0,
            Stroke::new(1.0, Color32::WHITE),
        );
    }

    // Focus outline on the edit-selected lane.
    if is_selected {
        painter.rect_stroke(
            Rect::from_min_size(
                Pos2::new(grid_x, auto_y),
                Vec2::new(grid_width, AUTOMATION_ZONE_HEIGHT),
            ),
            0.0,
            Stroke::new(1.0, AUTOMATION_ORANGE.gamma_multiply(0.8)),
            egui::StrokeKind::Inside,
        );
    }
}
