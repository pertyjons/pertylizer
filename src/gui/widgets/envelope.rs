//! ADSR envelope visualization and editor widgets.

use eframe::egui::{self, Color32, Pos2, Sense, Shape, Stroke, Ui, Vec2};

use crate::gui::theme::theme;

/// Draw an ADSR envelope visualization.
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

/// ADSR Envelope editor widget.
/// Allows visual editing of Attack, Decay, Sustain, and Release values.
pub struct EnvelopeEditor<'a> {
    attack: &'a mut f32,
    decay: &'a mut f32,
    sustain: &'a mut f32,
    release: &'a mut f32,
    accent_color: Color32,
    width: f32,
    height: f32,
}

impl<'a> EnvelopeEditor<'a> {
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
            width: 150.0,
            height: 60.0,
        }
    }

    pub fn accent_color(mut self, color: Color32) -> Self {
        self.accent_color = color;
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Show the envelope editor. Returns true if any value changed.
    pub fn show(self, ui: &mut Ui) -> bool {
        let mut changed = false;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(self.width, self.height), Sense::click_and_drag());

        let painter = ui.painter();

        // Background
        painter.rect_filled(rect, 4.0, theme().colors.bg_widget);
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, theme().colors.text_dim),
            egui::StrokeKind::Outside,
        );

        let inner = rect.shrink(4.0);
        let bottom = inner.bottom();
        let top = inner.top();
        let left = inner.left();
        let right = inner.right();
        let width = inner.width();
        let height = inner.height();

        // Normalize times for display (assuming max 2 seconds each)
        let max_time = 2.0;
        let attack_norm = (*self.attack / max_time).min(1.0);
        let decay_norm = (*self.decay / max_time).min(1.0);
        let release_norm = (*self.release / max_time).min(1.0);
        let sustain_norm = *self.sustain;

        // Calculate x positions
        let total_time = attack_norm + decay_norm + 0.2 + release_norm; // 0.2 for sustain hold
        let scale = if total_time > 0.0 {
            1.0 / total_time
        } else {
            1.0
        };

        let attack_x = left + attack_norm * scale * width;
        let decay_x = attack_x + decay_norm * scale * width;
        let sustain_x = decay_x + 0.2 * scale * width;
        let release_x = right.min(sustain_x + release_norm * scale * width);

        // Y positions
        let sustain_y = top + (1.0 - sustain_norm) * height;

        // Draw envelope shape
        let points = [
            Pos2::new(left, bottom),         // Start
            Pos2::new(attack_x, top),        // Peak after attack
            Pos2::new(decay_x, sustain_y),   // After decay
            Pos2::new(sustain_x, sustain_y), // Sustain hold
            Pos2::new(release_x, bottom),    // End
        ];

        // Fill
        let fill_points: Vec<_> = points
            .iter()
            .copied()
            .chain(std::iter::once(Pos2::new(left, bottom)))
            .collect();

        painter.add(egui::Shape::convex_polygon(
            fill_points,
            self.accent_color.gamma_multiply(0.2),
            Stroke::NONE,
        ));

        // Lines (single optimized line shape)
        painter.add(Shape::line(
            points.to_vec(),
            Stroke::new(2.0, self.accent_color),
        ));

        // Control points (draggable)
        let control_points = [
            (Pos2::new(attack_x, top), "A"),
            (Pos2::new(decay_x, sustain_y), "D"),
            (Pos2::new(sustain_x, sustain_y), "S"),
        ];

        for (point, label) in &control_points {
            let is_hovered = response.hovered()
                && (ui
                    .input(|i| i.pointer.hover_pos())
                    .map(|p| (p - *point).length() < 10.0)
                    .unwrap_or(false));

            let radius = if is_hovered { 6.0 } else { 4.0 };
            painter.circle_filled(*point, radius, self.accent_color);

            // Label
            painter.text(
                *point + Vec2::new(0.0, -10.0),
                egui::Align2::CENTER_BOTTOM,
                *label,
                theme().fonts.small(),
                theme().colors.text_dim,
            );
        }

        // Handle dragging
        if response.dragged()
            && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        {
            // Find nearest control point
            let distances = [
                ((pos - Pos2::new(attack_x, top)).length(), 0),
                ((pos - Pos2::new(decay_x, sustain_y)).length(), 1),
                ((pos - Pos2::new(sustain_x, sustain_y)).length(), 2),
            ];

            let nearest = distances
                .iter()
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(_, idx)| *idx)
                .unwrap_or(0);

            match nearest {
                0 => {
                    // Attack - horizontal drag
                    let new_attack =
                        ((pos.x - left) / width * max_time / scale).clamp(0.001, max_time);
                    if (new_attack - *self.attack).abs() > 0.001 {
                        *self.attack = new_attack;
                        changed = true;
                    }
                }
                1 => {
                    // Decay - horizontal and vertical
                    let new_decay =
                        ((pos.x - attack_x) / width * max_time / scale).clamp(0.001, max_time);
                    let new_sustain = 1.0 - ((pos.y - top) / height).clamp(0.0, 1.0);
                    if (new_decay - *self.decay).abs() > 0.001 {
                        *self.decay = new_decay;
                        changed = true;
                    }
                    if (new_sustain - *self.sustain).abs() > 0.001 {
                        *self.sustain = new_sustain;
                        changed = true;
                    }
                }
                2 => {
                    // Sustain - vertical only
                    let new_sustain = 1.0 - ((pos.y - top) / height).clamp(0.0, 1.0);
                    if (new_sustain - *self.sustain).abs() > 0.001 {
                        *self.sustain = new_sustain;
                        changed = true;
                    }
                }
                _ => {}
            }
        }

        changed
    }
}
