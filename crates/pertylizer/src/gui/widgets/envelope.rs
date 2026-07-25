//! ADSR envelope visualization and editor widgets.

use eframe::egui::{self, Color32, Pos2, Sense, Shape, Stroke, Ui, Vec2};

use super::controls::{EnvelopeCurveDirection, envelope_curve_after_vertical_drag};
use crate::gui::theme::theme;
use synth_core::{BipolarValue, NormalizedValue, Seconds, TimeScale};
use synth_modules::EnvelopeStage;

const CURVE_STEPS: usize = 16;

/// Draw an ADSR envelope visualization (non-interactive).
pub fn draw_adsr_curve(
    ui: &mut Ui,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    width: f32,
    height: f32,
) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    // Read-only preview: expose so the egui-inspection MCP can locate it.
    super::controls::expose(&response, egui::WidgetType::Other, "ADSR curve", None);
    let t = theme();
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, t.style.corner_radius, t.colors.bg_dark);

    // Normalize times
    let total_time = attack + decay + release + 0.5; // 0.5 for sustain display
    let attack_x = rect.left() + (attack / total_time) * rect.width();
    let decay_x = attack_x + (decay / total_time) * rect.width();
    let sustain_x = decay_x + (0.5 / total_time) * rect.width();
    let release_x = rect.right();

    let top = rect.top() + 4.0;
    let bottom = rect.bottom() - 4.0;
    let sustain_y = bottom - sustain * (bottom - top);

    // Draw ADSR shape
    let points = [
        Pos2::new(rect.left(), bottom),  // Start
        Pos2::new(attack_x, top),        // Peak after attack
        Pos2::new(decay_x, sustain_y),   // Sustain level after decay
        Pos2::new(sustain_x, sustain_y), // Hold sustain
        Pos2::new(release_x, bottom),    // End of release
    ];

    // Draw envelope as single optimized line shape
    painter.add(Shape::line(
        points.to_vec(),
        Stroke::new(t.style.border_width_thick, t.colors.accent_cyan),
    ));

    // Draw dots at key points
    for point in &points[1..4] {
        painter.circle_filled(*point, 3.0, t.colors.accent_cyan);
    }
}

/// Which control point is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragPoint {
    Attack,
    Decay,
    Sustain,
    Release,
    AttackCurve,
    DecayCurve,
    ReleaseCurve,
}

impl DragPoint {
    #[must_use]
    const fn is_curve(self) -> bool {
        matches!(
            self,
            Self::AttackCurve | Self::DecayCurve | Self::ReleaseCurve
        )
    }
}

/// State captured at pointer-down so curve drags remain relative and stable.
#[derive(Debug, Clone, Copy)]
struct DragState {
    point: DragPoint,
    pointer_origin: Pos2,
    time_origin: Seconds,
    display_time: Seconds,
    curve_origin: BipolarValue,
}

/// Result of an envelope edit - contains changed values.
#[derive(Debug, Clone, Default)]
pub struct EnvelopeChanges {
    pub attack: Option<Seconds>,
    pub decay: Option<Seconds>,
    pub sustain: Option<NormalizedValue>,
    pub release: Option<Seconds>,
    pub attack_curve: Option<BipolarValue>,
    pub decay_curve: Option<BipolarValue>,
    pub release_curve: Option<BipolarValue>,
}

impl EnvelopeChanges {
    /// Returns true if any value changed.
    #[must_use]
    pub fn any_changed(&self) -> bool {
        self.attack.is_some()
            || self.decay.is_some()
            || self.sustain.is_some()
            || self.release.is_some()
            || self.attack_curve.is_some()
            || self.decay_curve.is_some()
            || self.release_curve.is_some()
    }
}

/// Format time value for display.
/// Uses seconds (s) for values >= 1.0, milliseconds (ms) for smaller values.
fn format_time(seconds: Seconds) -> String {
    if seconds.as_f32() >= 1.0 {
        format!("{:.2}s", seconds.as_f32())
    } else {
        format!("{:.0}ms", seconds.as_millis())
    }
}

fn scaled_stage_time(time: Seconds, scale: TimeScale) -> Seconds {
    time * scale.as_f32()
}

fn timed_node_tooltip(time: Seconds, level: NormalizedValue) -> String {
    format!("{}  •  {:.0}%", format_time(time), level.as_f32() * 100.0)
}

fn attack_node_tooltip(time: Seconds) -> String {
    format!("{}  •  100% peak (fixed)", format_time(time))
}

fn curve_handle_position(from: Pos2, to: Pos2, curve: BipolarValue) -> Pos2 {
    let phase = 0.5;
    let y = synth_modules::math::interpolate_with_curve(from.y, to.y, phase, curve.as_f32());
    Pos2::new(egui::lerp(from.x..=to.x, phase), y)
}

fn append_curved_segment(points: &mut Vec<Pos2>, from: Pos2, to: Pos2, curve: BipolarValue) {
    if points.is_empty() {
        points.push(from);
    }
    for step in 1..=CURVE_STEPS {
        let phase = step as f32 / CURVE_STEPS as f32;
        let y = synth_modules::math::interpolate_with_curve(from.y, to.y, phase, curve.as_f32());
        points.push(Pos2::new(egui::lerp(from.x..=to.x, phase), y));
    }
}

fn stage_time_after_drag(
    origin: Seconds,
    delta_x: f32,
    width: f32,
    display_time: Seconds,
    maximum: Seconds,
) -> Seconds {
    let requested = origin + display_time * (delta_x / width.max(1.0));
    Seconds::new(requested.as_f32().clamp(0.0, maximum.as_f32()))
}

fn drag_cursor(point: DragPoint) -> egui::CursorIcon {
    match point {
        DragPoint::Attack | DragPoint::Release => egui::CursorIcon::ResizeHorizontal,
        DragPoint::Sustain
        | DragPoint::AttackCurve
        | DragPoint::DecayCurve
        | DragPoint::ReleaseCurve => egui::CursorIcon::ResizeVertical,
        DragPoint::Decay => egui::CursorIcon::Move,
    }
}

/// ADSR Envelope editor widget with interactive grid and tooltips.
///
/// Allows visual editing of Attack, Decay, Sustain, and Release values
/// by dragging control points directly on the curve.
///
/// Each parameter can be adjusted independently within its full range (0 to max_time
/// for time parameters, 0-100% for sustain).
pub struct EnvelopeEditor<'a> {
    attack: &'a mut f32,
    decay: &'a mut f32,
    sustain: &'a mut f32,
    release: &'a mut f32,
    attack_curve: &'a mut f32,
    decay_curve: &'a mut f32,
    release_curve: &'a mut f32,
    accent_color: Color32,
    width: f32,
    height: f32,
    /// Maximum time value for A/D/R (seconds).
    max_time: f32,
    /// Global multiplier applied to all three timed stages.
    time_scale: TimeScale,
    /// Current playback position for visualization (stage, time_in_stage_seconds).
    playback_position: Option<(EnvelopeStage, Seconds)>,
}

impl<'a> EnvelopeEditor<'a> {
    /// Create a new envelope editor.
    #[must_use]
    pub fn new(
        attack: &'a mut f32,
        decay: &'a mut f32,
        sustain: &'a mut f32,
        release: &'a mut f32,
        attack_curve: &'a mut f32,
        decay_curve: &'a mut f32,
        release_curve: &'a mut f32,
    ) -> Self {
        Self {
            attack,
            decay,
            sustain,
            release,
            attack_curve,
            decay_curve,
            release_curve,
            accent_color: theme().colors.accent_green,
            width: 200.0,
            height: 100.0,
            max_time: 10.0,
            time_scale: TimeScale::UNITY,
            playback_position: None,
        }
    }

    /// Set the accent color.
    #[must_use]
    pub fn accent_color(mut self, color: Color32) -> Self {
        self.accent_color = color;
        self
    }

    /// Set the widget size.
    #[must_use]
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Set the maximum time value for A/D/R parameters.
    #[must_use]
    pub fn max_time(mut self, max: f32) -> Self {
        self.max_time = max;
        self
    }

    /// Set the global A/D/R time multiplier used by playback and readouts.
    #[must_use]
    pub fn time_scale(mut self, scale: TimeScale) -> Self {
        self.time_scale = scale;
        self
    }

    /// Set the current playback position for visualization.
    ///
    /// A vertical time indicator line will be drawn at the position
    /// corresponding to the current envelope stage and elapsed time.
    #[must_use]
    pub fn playback_position(mut self, stage: EnvelopeStage, time_in_stage: Seconds) -> Self {
        self.playback_position = Some((stage, time_in_stage));
        self
    }

    /// Show the envelope editor.
    ///
    /// Returns `Some(EnvelopeChanges)` if any values changed, `None` otherwise.
    #[must_use]
    pub fn show(self, ui: &mut Ui) -> Option<EnvelopeChanges> {
        let id = ui.id().with("envelope_editor");
        let mut changes = EnvelopeChanges::default();

        // Get or initialize drag state
        let dragging: Option<DragState> = ui.memory(|m| m.data.get_temp(id));

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(self.width, self.height), Sense::click_and_drag());

        // Expose the editor container to AccessKit / the egui-inspection MCP. Per-
        // handle drivability (dragging an individual ADSR node) is out of scope
        // for v1 — this makes the editor locatable in the tree.
        super::controls::expose(&response, egui::WidgetType::Other, "ADSR editor", None);

        let painter = ui.painter();
        let t = theme();

        // Background
        painter.rect_filled(rect, t.style.corner_radius, t.colors.bg_widget);
        painter.rect_stroke(
            rect,
            t.style.corner_radius,
            Stroke::new(t.style.border_width, t.colors.text_dim.gamma_multiply(0.5)),
            egui::StrokeKind::Inside,
        );

        let inner = rect.shrink(6.0);
        let bottom = inner.bottom();
        let top = inner.top();
        let left = inner.left();
        let right = inner.right();
        let width = inner.width();
        let height = inner.height();

        // Draw grid (5x5 lines)
        let grid_color = t.colors.text_dim.gamma_multiply(0.15);
        for i in 1..5 {
            let x = left + (i as f32 / 5.0) * width;
            let y = top + (i as f32 / 5.0) * height;

            painter.line_segment(
                [Pos2::new(x, top), Pos2::new(x, bottom)],
                Stroke::new(1.0, grid_color),
            );
            painter.line_segment(
                [Pos2::new(left, y), Pos2::new(right, y)],
                Stroke::new(1.0, grid_color),
            );
        }

        let effective_attack = scaled_stage_time(Seconds::new(*self.attack), self.time_scale);
        let effective_decay = scaled_stage_time(Seconds::new(*self.decay), self.time_scale);
        let effective_release = scaled_stage_time(Seconds::new(*self.release), self.time_scale);

        // Draw effective total time in upper right corner (A + D + R).
        let total_sound_time = effective_attack + effective_decay + effective_release;
        let total_text = format!("Σ {}", format_time(total_sound_time));
        painter.text(
            Pos2::new(right - 2.0, top + 2.0),
            egui::Align2::RIGHT_TOP,
            total_text,
            t.fonts.small(),
            t.colors.text_dim,
        );

        // Calculate positions based on actual time values
        // Use dynamic scaling: total display time = A + D + sustain_hold + R
        let sustain_hold = 0.3 * self.max_time; // Fixed sustain display section
        let total_time = *self.attack + *self.decay + sustain_hold + *self.release;
        let scale = if total_time > 0.001 {
            width / total_time
        } else {
            width / self.max_time
        };

        // X positions based on actual times
        let attack_x = left + *self.attack * scale;
        let decay_x = attack_x + *self.decay * scale;
        let sustain_x = decay_x + sustain_hold * scale;
        let release_x = (sustain_x + *self.release * scale).min(right);

        // Y position for sustain level
        let sustain_norm = (*self.sustain).clamp(0.0, 1.0);
        let sustain_y = top + (1.0 - sustain_norm) * height;

        // Control points positions
        let attack_point = Pos2::new(attack_x.clamp(left, right), top);
        let decay_point = Pos2::new(decay_x.clamp(left, right), sustain_y);
        let sustain_point = Pos2::new(sustain_x.clamp(left, right), sustain_y);
        let release_point = Pos2::new(release_x.clamp(left, right), bottom);

        let attack_curve = BipolarValue::new(*self.attack_curve);
        let decay_curve = BipolarValue::new(*self.decay_curve);
        let release_curve = BipolarValue::new(*self.release_curve);
        let start_point = Pos2::new(left, bottom);
        let attack_curve_point = curve_handle_position(start_point, attack_point, attack_curve);
        let decay_curve_point = curve_handle_position(attack_point, decay_point, decay_curve);
        let release_curve_point =
            curve_handle_position(sustain_point, release_point, release_curve);

        // Draw the shaped A/D/R segments and the level sustain section.
        let mut points = Vec::with_capacity(CURVE_STEPS * 3 + 3);
        append_curved_segment(&mut points, start_point, attack_point, attack_curve);
        append_curved_segment(&mut points, attack_point, decay_point, decay_curve);
        points.push(sustain_point);
        append_curved_segment(&mut points, sustain_point, release_point, release_curve);

        super::controls::paint_envelope_curve_fill(
            painter,
            &points,
            bottom,
            self.accent_color.gamma_multiply(0.15),
        );

        // Curve line
        painter.add(Shape::line(points, Stroke::new(2.5, self.accent_color)));

        // Draw playback time indicator (vertical line)
        if let Some((stage, time_in_stage)) = self.playback_position {
            let t = time_in_stage.as_f32();
            let playhead_x = match stage {
                EnvelopeStage::Idle => None,
                EnvelopeStage::Attack => {
                    // Time progress through attack phase
                    let attack_time = effective_attack.as_f32().max(0.001);
                    let progress = (t / attack_time).clamp(0.0, 1.0);
                    Some(left + progress * (attack_x - left))
                }
                EnvelopeStage::Decay => {
                    // Time progress through decay phase
                    let decay_time = effective_decay.as_f32().max(0.001);
                    let progress = (t / decay_time).clamp(0.0, 1.0);
                    Some(attack_x + progress * (decay_x - attack_x))
                }
                EnvelopeStage::Sustain => {
                    // Sustain holds at decay_x (or animate through sustain section)
                    let sustain_progress = (t / sustain_hold).clamp(0.0, 1.0);
                    Some(decay_x + sustain_progress * (sustain_x - decay_x))
                }
                EnvelopeStage::Release => {
                    // Time progress through release phase
                    let release_time = effective_release.as_f32().max(0.001);
                    let progress = (t / release_time).clamp(0.0, 1.0);
                    Some(sustain_x + progress * (release_x - sustain_x))
                }
            };

            if let Some(x) = playhead_x {
                let playhead_color = theme().colors.accent_yellow;
                // Vertical line from top to bottom
                painter.line_segment(
                    [Pos2::new(x, top), Pos2::new(x, bottom)],
                    Stroke::new(2.0, playhead_color),
                );
                // Small triangle indicator at top
                let tri_size = 5.0;
                painter.add(Shape::convex_polygon(
                    vec![
                        Pos2::new(x, top),
                        Pos2::new(x - tri_size, top - tri_size),
                        Pos2::new(x + tri_size, top - tri_size),
                    ],
                    playhead_color,
                    Stroke::NONE,
                ));
            }
        }

        // Control point info with correct formatting
        // A, D, R: time in seconds/ms
        // S: percentage
        let control_points = [
            (
                attack_point,
                DragPoint::Attack,
                "A",
                attack_node_tooltip(effective_attack),
            ),
            (
                decay_point,
                DragPoint::Decay,
                "D",
                timed_node_tooltip(effective_decay, NormalizedValue::new(sustain_norm)),
            ),
            (
                sustain_point,
                DragPoint::Sustain,
                "S",
                format!("{:.0}%", *self.sustain * 100.0),
            ),
            (
                release_point,
                DragPoint::Release,
                "R",
                timed_node_tooltip(effective_release, NormalizedValue::MIN),
            ),
            (
                attack_curve_point,
                DragPoint::AttackCurve,
                "",
                format!("Attack Curve {:+.2}", attack_curve.as_f32()),
            ),
            (
                decay_curve_point,
                DragPoint::DecayCurve,
                "",
                format!("Decay Curve {:+.2}", decay_curve.as_f32()),
            ),
            (
                release_curve_point,
                DragPoint::ReleaseCurve,
                "",
                format!("Release Curve {:+.2}", release_curve.as_f32()),
            ),
        ];

        // Inside an `egui::Scene` the raw `ui.input` pointer is global/screen,
        // but our manual hit-tests below compare against the curve's world-space
        // `rect`/control points — egui only transforms the pointer for real
        // widget interactions (the knobs), not these lookups. Map screen → local
        // so hover/drag work at any Scene pan/zoom (identity outside a Scene).
        let to_local = ui
            .ctx()
            .layer_transform_to_global(ui.layer_id())
            .map(|t| t.inverse());
        let to_local_pos = |p: Pos2| to_local.map_or(p, |t| t * p);

        // Detect hover
        let hover_pos = ui.input(|i| i.pointer.hover_pos()).map(to_local_pos);
        let mut hovered_point: Option<DragPoint> = None;

        if let Some(pos) = hover_pos
            && rect.contains(pos)
        {
            // Find nearest point within threshold
            let threshold = 15.0;
            let mut min_dist = threshold;

            for (point, drag_type, _, _) in &control_points {
                let dist = (*point - pos).length();
                if dist < min_dist {
                    min_dist = dist;
                    hovered_point = Some(*drag_type);
                }
            }
        }

        // Draw control points
        for (point, drag_type, label, value_text) in &control_points {
            let is_hovered = hovered_point == Some(*drag_type);
            let is_dragging = dragging.map(|state| state.point) == Some(*drag_type);

            if drag_type.is_curve() {
                super::controls::paint_envelope_curve_handle(
                    painter,
                    *point,
                    self.accent_color,
                    is_hovered || is_dragging,
                );
                if is_hovered || is_dragging {
                    super::tooltip::draw_tooltip_above(ui, *point, value_text, self.accent_color);
                }
                continue;
            }

            let radius = if is_dragging {
                8.0
            } else if is_hovered {
                6.0
            } else {
                4.0
            };

            // Glow effect when hovered/dragging
            if is_hovered || is_dragging {
                painter.circle_filled(*point, radius + 4.0, self.accent_color.gamma_multiply(0.3));
            }

            // Main dot
            let point_color = if is_dragging {
                Color32::WHITE
            } else {
                self.accent_color
            };
            painter.circle_filled(*point, radius, point_color);

            // Label above point
            let label_offset = match *drag_type {
                DragPoint::Release => Vec2::new(-10.0, -12.0),
                DragPoint::Sustain => Vec2::new(-8.0, -12.0),
                _ => Vec2::new(0.0, -12.0),
            };

            painter.text(
                *point + label_offset,
                egui::Align2::CENTER_BOTTOM,
                *label,
                t.fonts.small(),
                t.colors.text_secondary,
            );

            // Tooltip with value when hovered or dragging
            if is_hovered || is_dragging {
                super::tooltip::draw_tooltip_above(ui, *point, value_text, self.accent_color);
            }
        }

        // Handle drag start
        if response.drag_started()
            && let Some(hovered) = hovered_point
            && let Some(pointer_origin) = hover_pos
        {
            let curve_origin = match hovered {
                DragPoint::AttackCurve => attack_curve,
                DragPoint::DecayCurve => decay_curve,
                DragPoint::ReleaseCurve => release_curve,
                _ => BipolarValue::CENTER,
            };
            let time_origin = match hovered {
                DragPoint::Attack => Seconds::new(*self.attack),
                DragPoint::Decay => Seconds::new(*self.decay),
                DragPoint::Release => Seconds::new(*self.release),
                _ => Seconds::ZERO,
            };
            ui.memory_mut(|m| {
                m.data.insert_temp(
                    id,
                    DragState {
                        point: hovered,
                        pointer_origin,
                        time_origin,
                        display_time: Seconds::new(total_time),
                        curve_origin,
                    },
                );
            });
        }

        // Handle drag end
        if response.drag_stopped() {
            ui.memory_mut(|m| m.data.remove::<DragState>(id));
        }

        // Handle dragging - each point moves freely within its range
        if let Some(drag) = dragging
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos()).map(to_local_pos)
        {
            match drag.point {
                DragPoint::Attack => {
                    let new_attack = stage_time_after_drag(
                        drag.time_origin,
                        pos.x - drag.pointer_origin.x,
                        width,
                        drag.display_time,
                        Seconds::new(self.max_time),
                    );
                    if (new_attack.as_f32() - *self.attack).abs() > 0.001 {
                        *self.attack = new_attack.as_f32();
                        changes.attack = Some(new_attack);
                    }
                }
                DragPoint::Decay => {
                    let new_decay = stage_time_after_drag(
                        drag.time_origin,
                        pos.x - drag.pointer_origin.x,
                        width,
                        drag.display_time,
                        Seconds::new(self.max_time),
                    );
                    let new_sustain = (1.0 - (pos.y - top) / height).clamp(0.0, 1.0);

                    if (new_decay.as_f32() - *self.decay).abs() > 0.001 {
                        *self.decay = new_decay.as_f32();
                        changes.decay = Some(new_decay);
                    }
                    if (new_sustain - *self.sustain).abs() > 0.005 {
                        *self.sustain = new_sustain;
                        changes.sustain = Some(NormalizedValue::new(new_sustain));
                    }
                }
                DragPoint::Sustain => {
                    // Sustain: vertical drag only (0-100%)
                    let new_sustain = (1.0 - (pos.y - top) / height).clamp(0.0, 1.0);
                    if (new_sustain - *self.sustain).abs() > 0.005 {
                        *self.sustain = new_sustain;
                        changes.sustain = Some(NormalizedValue::new(new_sustain));
                    }
                }
                DragPoint::Release => {
                    let new_release = stage_time_after_drag(
                        drag.time_origin,
                        pos.x - drag.pointer_origin.x,
                        width,
                        drag.display_time,
                        Seconds::new(self.max_time),
                    );
                    if (new_release.as_f32() - *self.release).abs() > 0.001 {
                        *self.release = new_release.as_f32();
                        changes.release = Some(new_release);
                    }
                }
                DragPoint::AttackCurve => {
                    let curve = envelope_curve_after_vertical_drag(
                        drag.curve_origin,
                        pos.y - drag.pointer_origin.y,
                        height,
                        EnvelopeCurveDirection::Rising,
                    );
                    if (curve.as_f32() - *self.attack_curve).abs() > f32::EPSILON {
                        *self.attack_curve = curve.as_f32();
                        changes.attack_curve = Some(curve);
                    }
                }
                DragPoint::DecayCurve => {
                    let curve = envelope_curve_after_vertical_drag(
                        drag.curve_origin,
                        pos.y - drag.pointer_origin.y,
                        height,
                        EnvelopeCurveDirection::Falling,
                    );
                    if (curve.as_f32() - *self.decay_curve).abs() > f32::EPSILON {
                        *self.decay_curve = curve.as_f32();
                        changes.decay_curve = Some(curve);
                    }
                }
                DragPoint::ReleaseCurve => {
                    let curve = envelope_curve_after_vertical_drag(
                        drag.curve_origin,
                        pos.y - drag.pointer_origin.y,
                        height,
                        EnvelopeCurveDirection::Falling,
                    );
                    if (curve.as_f32() - *self.release_curve).abs() > f32::EPSILON {
                        *self.release_curve = curve.as_f32();
                        changes.release_curve = Some(curve);
                    }
                }
            }
        }

        // Match the cursor to each point's editable axes. The ADSR attack peak
        // is fixed at 100%, so its node intentionally exposes horizontal time
        // adjustment only.
        if let Some(point) = dragging.map(|state| state.point).or(hovered_point) {
            ui.ctx().set_cursor_icon(drag_cursor(point));
        }

        if changes.any_changed() {
            Some(changes)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_handle_tracks_the_shaped_segment_midpoint() {
        let from = Pos2::new(0.0, 100.0);
        let to = Pos2::new(100.0, 0.0);

        let linear = curve_handle_position(from, to, BipolarValue::CENTER);
        let exponential = curve_handle_position(from, to, BipolarValue::MAX);
        let logarithmic = curve_handle_position(from, to, BipolarValue::MIN);

        assert_eq!(linear, Pos2::new(50.0, 50.0));
        assert_eq!(exponential.x, 50.0);
        assert!(exponential.y > linear.y);
        assert_eq!(logarithmic.x, 50.0);
        assert!(logarithmic.y < linear.y);
    }

    #[test]
    fn curve_drag_uses_and_clamps_the_full_bipolar_range() {
        assert_eq!(
            envelope_curve_after_vertical_drag(
                BipolarValue::CENTER,
                -25.0,
                100.0,
                EnvelopeCurveDirection::Falling,
            ),
            BipolarValue::MAX
        );
        assert_eq!(
            envelope_curve_after_vertical_drag(
                BipolarValue::CENTER,
                25.0,
                100.0,
                EnvelopeCurveDirection::Falling,
            ),
            BipolarValue::MIN
        );
        assert_eq!(
            envelope_curve_after_vertical_drag(
                BipolarValue::CENTER,
                -100.0,
                100.0,
                EnvelopeCurveDirection::Falling,
            ),
            BipolarValue::MAX
        );
    }

    #[test]
    fn rising_curve_drag_follows_the_pointer_direction() {
        let upward = envelope_curve_after_vertical_drag(
            BipolarValue::CENTER,
            -25.0,
            100.0,
            EnvelopeCurveDirection::Rising,
        );
        let downward = envelope_curve_after_vertical_drag(
            BipolarValue::CENTER,
            25.0,
            100.0,
            EnvelopeCurveDirection::Rising,
        );

        assert_eq!(upward, BipolarValue::MIN);
        assert_eq!(downward, BipolarValue::MAX);
    }

    #[test]
    fn stage_time_drag_can_lower_and_raise_from_its_origin() {
        let origin = Seconds::new(0.5);
        let display_time = Seconds::new(2.0);
        let maximum = Seconds::new(10.0);

        assert_eq!(
            stage_time_after_drag(origin, -10.0, 100.0, display_time, maximum),
            Seconds::new(0.3)
        );
        assert_eq!(
            stage_time_after_drag(origin, 10.0, 100.0, display_time, maximum),
            Seconds::new(0.7)
        );
    }

    #[test]
    fn timed_node_tooltip_includes_time_and_level() {
        assert_eq!(
            timed_node_tooltip(Seconds::new(0.25), NormalizedValue::new(0.7)),
            "250ms  •  70%"
        );
    }

    #[test]
    fn attack_tooltip_marks_the_peak_as_fixed() {
        assert_eq!(
            attack_node_tooltip(Seconds::new(0.25)),
            "250ms  •  100% peak (fixed)"
        );
    }

    #[test]
    fn adsr_cursors_match_the_editable_axes() {
        assert_eq!(
            drag_cursor(DragPoint::Attack),
            egui::CursorIcon::ResizeHorizontal
        );
        assert_eq!(
            drag_cursor(DragPoint::Sustain),
            egui::CursorIcon::ResizeVertical
        );
        assert_eq!(drag_cursor(DragPoint::Decay), egui::CursorIcon::Move);
    }

    #[test]
    fn curve_edits_are_reported_as_envelope_changes() {
        let changes = EnvelopeChanges {
            attack_curve: Some(BipolarValue::new(0.5)),
            ..EnvelopeChanges::default()
        };

        assert!(changes.any_changed());
    }

    #[test]
    fn time_scale_changes_effective_point_readouts() {
        let raw = Seconds::new(0.25);

        assert_eq!(scaled_stage_time(raw, TimeScale::UNITY), Seconds::new(0.25));
        assert_eq!(
            scaled_stage_time(raw, TimeScale::new(2.0)),
            Seconds::new(0.5)
        );
    }
}
