//! ADSR envelope visualization and editor widgets.

use eframe::egui::{self, Color32, Pos2, Sense, Shape, Stroke, Ui, Vec2};

use crate::gui::theme::theme;
use crate::types::{NormalizedValue, Seconds};

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
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 4.0, theme().colors.bg_dark);

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
        Stroke::new(2.0, theme().colors.accent_cyan),
    ));

    // Draw dots at key points
    for point in &points[1..4] {
        painter.circle_filled(*point, 3.0, theme().colors.accent_cyan);
    }
}

/// Which control point is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragPoint {
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Result of an envelope edit - contains changed values.
#[derive(Debug, Clone, Default)]
pub struct EnvelopeChanges {
    pub attack: Option<Seconds>,
    pub decay: Option<Seconds>,
    pub sustain: Option<NormalizedValue>,
    pub release: Option<Seconds>,
}

impl EnvelopeChanges {
    /// Returns true if any value changed.
    #[must_use]
    pub fn any_changed(&self) -> bool {
        self.attack.is_some()
            || self.decay.is_some()
            || self.sustain.is_some()
            || self.release.is_some()
    }
}

/// Format time value for display.
/// Uses seconds (s) for values >= 1.0, milliseconds (ms) for smaller values.
fn format_time(seconds: f32) -> String {
    if seconds >= 1.0 {
        format!("{:.2}s", seconds)
    } else {
        format!("{:.0}ms", seconds * 1000.0)
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
    accent_color: Color32,
    width: f32,
    height: f32,
    /// Maximum time value for A/D/R (seconds).
    max_time: f32,
}

impl<'a> EnvelopeEditor<'a> {
    /// Create a new envelope editor.
    #[must_use]
    pub fn new(
        attack: &'a mut f32,
        decay: &'a mut f32,
        sustain: &'a mut f32,
        release: &'a mut f32,
    ) -> Self {
        Self {
            attack,
            decay,
            sustain,
            release,
            accent_color: theme().colors.accent_green,
            width: 200.0,
            height: 100.0,
            max_time: 10.0,
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

    /// Show the envelope editor.
    ///
    /// Returns `Some(EnvelopeChanges)` if any values changed, `None` otherwise.
    #[must_use]
    pub fn show(self, ui: &mut Ui) -> Option<EnvelopeChanges> {
        let id = ui.id().with("envelope_editor");
        let mut changes = EnvelopeChanges::default();

        // Get or initialize drag state
        let dragging: Option<DragPoint> = ui.memory(|m| m.data.get_temp(id));

        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(self.width, self.height), Sense::click_and_drag());

        let painter = ui.painter();
        let t = theme();

        // Background
        painter.rect_filled(rect, 4.0, t.colors.bg_widget);
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, t.colors.text_dim.gamma_multiply(0.5)),
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

        // Draw total time in upper right corner (A + D + R)
        let total_sound_time = *self.attack + *self.decay + *self.release;
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

        // Draw envelope shape
        let points = [
            Pos2::new(left, bottom), // Start
            attack_point,            // Peak after attack
            decay_point,             // After decay
            sustain_point,           // Sustain hold
            release_point,           // End
        ];

        // Fill under curve
        let fill_points: Vec<_> = points
            .iter()
            .copied()
            .chain(std::iter::once(Pos2::new(right, bottom)))
            .chain(std::iter::once(Pos2::new(left, bottom)))
            .collect();

        painter.add(egui::Shape::convex_polygon(
            fill_points,
            self.accent_color.gamma_multiply(0.15),
            Stroke::NONE,
        ));

        // Curve line
        painter.add(Shape::line(
            points.to_vec(),
            Stroke::new(2.5, self.accent_color),
        ));

        // Control point info with correct formatting
        // A, D, R: time in seconds/ms
        // S: percentage
        let control_points = [
            (
                attack_point,
                DragPoint::Attack,
                "A",
                format_time(*self.attack),
            ),
            (decay_point, DragPoint::Decay, "D", format_time(*self.decay)),
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
                format_time(*self.release),
            ),
        ];

        // Detect hover
        let hover_pos = ui.input(|i| i.pointer.hover_pos());
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
            let is_dragging = dragging == Some(*drag_type);

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
            let label_offset = if *drag_type == DragPoint::Release {
                Vec2::new(-10.0, -12.0)
            } else {
                Vec2::new(0.0, -12.0)
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
        {
            ui.memory_mut(|m| m.data.insert_temp(id, hovered));
        }

        // Handle drag end
        if response.drag_stopped() {
            ui.memory_mut(|m| m.data.remove::<DragPoint>(id));
        }

        // Handle dragging - each point moves freely within its range
        if let Some(drag_point) = dragging
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        {
            match drag_point {
                DragPoint::Attack => {
                    // Attack: horizontal drag, 0 to max_time
                    // Position relative to left edge
                    let relative_x = (pos.x - left).max(0.0);
                    let new_attack = (relative_x / scale).clamp(0.0, self.max_time);
                    if (new_attack - *self.attack).abs() > 0.001 {
                        *self.attack = new_attack;
                        changes.attack = Some(Seconds::new(new_attack));
                    }
                }
                DragPoint::Decay => {
                    // Decay: horizontal drag for time (0 to max_time), vertical for sustain level
                    // Position relative to attack point
                    let relative_x = (pos.x - attack_x).max(0.0);
                    let new_decay = (relative_x / scale).clamp(0.0, self.max_time);
                    let new_sustain = (1.0 - (pos.y - top) / height).clamp(0.0, 1.0);

                    if (new_decay - *self.decay).abs() > 0.001 {
                        *self.decay = new_decay;
                        changes.decay = Some(Seconds::new(new_decay));
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
                    // Release: horizontal drag, 0 to max_time
                    // Position relative to sustain point
                    let relative_x = (pos.x - sustain_x).max(0.0);
                    let new_release = (relative_x / scale).clamp(0.0, self.max_time);
                    if (new_release - *self.release).abs() > 0.001 {
                        *self.release = new_release;
                        changes.release = Some(Seconds::new(new_release));
                    }
                }
            }
        }

        // Set cursor when hovering over a point
        if hovered_point.is_some() || dragging.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }

        if changes.any_changed() {
            Some(changes)
        } else {
            None
        }
    }
}
