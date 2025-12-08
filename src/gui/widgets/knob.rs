//! Rotary knob widget.

use eframe::egui::{self, Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};

use crate::gui::theme::theme;
use crate::modules::core::{ParameterDescriptor, ParameterUnit, ResponseCurve};

/// A rotary knob widget.
pub struct Knob<'a> {
    value: &'a mut f32,
    min: f32,
    max: f32,
    default: f32,
    response_curve: ResponseCurve,
    unit: ParameterUnit,
    label: String,
    size: f32,
    accent_color: Color32,
}

impl<'a> Knob<'a> {
    pub fn new(value: &'a mut f32, min: f32, max: f32) -> Self {
        Self {
            value,
            min,
            max,
            default: (min + max) / 2.0,
            response_curve: ResponseCurve::Linear,
            unit: ParameterUnit::None,
            label: String::new(),
            size: 80.0,
            accent_color: theme().colors.accent_orange,
        }
    }

    pub fn from_descriptor(value: &'a mut f32, descriptor: &ParameterDescriptor) -> Self {
        Self {
            value,
            min: descriptor.min,
            max: descriptor.max,
            default: descriptor.default,
            response_curve: descriptor.response_curve,
            unit: descriptor.unit,
            label: descriptor.name.clone(),
            size: 80.0,
            accent_color: theme().colors.accent_orange,
        }
    }

    pub fn unit(mut self, unit: ParameterUnit) -> Self {
        self.unit = unit;
        self
    }

    /// Format the value with the appropriate unit suffix
    fn format_value(&self) -> String {
        self.unit.format(*self.value)
    }

    pub fn default(mut self, default: f32) -> Self {
        self.default = default;
        self
    }

    pub fn response_curve(mut self, curve: ResponseCurve) -> Self {
        self.response_curve = curve;
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn accent_color(mut self, color: Color32) -> Self {
        self.accent_color = color;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let t = theme();
        let padding = 6.0;
        let label_height = if self.label.is_empty() {
            0.0
        } else {
            t.sizes.knob_label_height
        };

        // Total widget size includes padding and label
        let widget_width = self.size + padding * 2.0;
        let widget_height = self.size + label_height + padding * 2.0;
        let desired_size = Vec2::new(widget_width, widget_height);

        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

        // The knob circle area is centered in the top portion
        let knob_size = self.size - padding;
        let knob_rect = egui::Rect::from_center_size(
            Pos2::new(rect.center().x, rect.top() + padding + knob_size / 2.0),
            Vec2::splat(knob_size),
        );

        if response.double_clicked() {
            *self.value = self.default;
        }

        if response.dragged() {
            let delta = -response.drag_delta().y;
            let sensitivity = 0.005;
            let normalized = self
                .response_curve
                .normalize(*self.value, self.min, self.max);
            let new_normalized = (normalized + delta * sensitivity).clamp(0.0, 1.0);
            *self.value = self
                .response_curve
                .denormalize(new_normalized, self.min, self.max);
        }

        let painter = ui.painter();

        // Draw frame around entire widget
        painter.rect_filled(rect, 4.0, theme().colors.bg_dark);
        painter.rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, theme().colors.bg_widget),
            egui::StrokeKind::Inside,
        );

        // Draw knob circle
        let center = knob_rect.center();
        let radius = knob_rect.width() / 2.0 - 2.0;

        // Background circle
        painter.circle_filled(center, radius, theme().colors.bg_widget);
        painter.circle_stroke(center, radius, Stroke::new(1.5, theme().colors.bg_panel));

        // Value arc
        let normalized = self
            .response_curve
            .normalize(*self.value, self.min, self.max);
        let start_angle = std::f32::consts::PI * 0.75;
        let end_angle = std::f32::consts::PI * 2.25;
        let value_angle = start_angle + normalized * (end_angle - start_angle);

        // Draw arc segments
        let segments = 32;
        let value_segments = ((normalized * segments as f32) as usize).min(segments);

        for i in 0..value_segments {
            let t0 = i as f32 / segments as f32;
            let t1 = (i + 1) as f32 / segments as f32;
            let a0 = start_angle + t0 * (end_angle - start_angle);
            let a1 = start_angle + t1 * (end_angle - start_angle);

            let inner_radius = radius - 5.0;
            let p0 = center + Vec2::new(a0.cos(), a0.sin()) * inner_radius;
            let p1 = center + Vec2::new(a1.cos(), a1.sin()) * inner_radius;

            painter.line_segment([p0, p1], Stroke::new(3.0, self.accent_color));
        }

        // Indicator dot at the edge
        let indicator_radius = radius - 8.0;
        let indicator_pos =
            center + Vec2::new(value_angle.cos(), value_angle.sin()) * indicator_radius;
        painter.circle_filled(indicator_pos, 3.0, theme().colors.text_primary);

        // Value text in center of knob
        let value_text = self.format_value();
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            &value_text,
            t.fonts.small(),
            theme().colors.text_primary,
        );

        // Label below knob circle, inside frame
        if !self.label.is_empty() {
            let label_pos = Pos2::new(rect.center().x, knob_rect.bottom() + 4.0);
            painter.text(
                label_pos,
                egui::Align2::CENTER_TOP,
                &self.label,
                t.fonts.small(),
                theme().colors.text_secondary,
            );
        }

        response
    }
}
