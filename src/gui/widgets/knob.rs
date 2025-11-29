//! Rotary knob widget.

use eframe::egui::{self, Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};

use crate::modules::core::{ParameterDescriptor, ResponseCurve};
use super::{colors, theme};

/// A rotary knob widget.
pub struct Knob<'a> {
    value: &'a mut f32,
    min: f32,
    max: f32,
    default: f32,
    response_curve: ResponseCurve,
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
            label: String::new(),
            size: 48.0,
            accent_color: colors::ACCENT_ORANGE,
        }
    }

    pub fn from_descriptor(value: &'a mut f32, descriptor: &ParameterDescriptor) -> Self {
        Self {
            value,
            min: descriptor.min,
            max: descriptor.max,
            default: descriptor.default,
            response_curve: descriptor.response_curve,
            label: descriptor.name.clone(),
            size: 48.0,
            accent_color: colors::ACCENT_ORANGE,
        }
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
        // Allocate extra height for the label if present
        let label_height = if self.label.is_empty() { 0.0 } else { theme().sizes.knob_label_height };
        let desired_size = Vec2::new(self.size, self.size + label_height);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

        // The knob circle area is the top square portion
        let knob_rect = egui::Rect::from_min_size(rect.min, Vec2::splat(self.size));

        if response.double_clicked() {
            *self.value = self.default;
        }

        if response.dragged() {
            let delta = -response.drag_delta().y;
            let sensitivity = 0.005;
            let normalized = self.response_curve.normalize(*self.value, self.min, self.max);
            let new_normalized = (normalized + delta * sensitivity).clamp(0.0, 1.0);
            *self.value = self.response_curve.denormalize(new_normalized, self.min, self.max);
        }

        // Draw knob
        let painter = ui.painter();
        let center = knob_rect.center();
        let radius = knob_rect.width() / 2.0 - 4.0;

        // Background circle
        painter.circle_filled(center, radius, colors::BG_WIDGET);
        painter.circle_stroke(center, radius, Stroke::new(2.0, colors::BG_DARK));

        // Value arc
        let normalized = self.response_curve.normalize(*self.value, self.min, self.max);
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

            let inner_radius = radius - 6.0;
            let p0 = center + Vec2::new(a0.cos(), a0.sin()) * inner_radius;
            let p1 = center + Vec2::new(a1.cos(), a1.sin()) * inner_radius;

            painter.line_segment([p0, p1], Stroke::new(4.0, self.accent_color));
        }

        // Indicator line
        let indicator_inner = radius * 0.3;
        let indicator_outer = radius * 0.7;
        let p_inner = center + Vec2::new(value_angle.cos(), value_angle.sin()) * indicator_inner;
        let p_outer = center + Vec2::new(value_angle.cos(), value_angle.sin()) * indicator_outer;
        painter.line_segment([p_inner, p_outer], Stroke::new(2.0, colors::TEXT_PRIMARY));

        // Label below knob circle
        if !self.label.is_empty() {
            let t = theme();
            let label_pos = Pos2::new(knob_rect.center().x, knob_rect.bottom() + t.spacing.label_spacing);
            painter.text(
                label_pos,
                egui::Align2::CENTER_TOP,
                &self.label,
                t.fonts.normal(),
                colors::TEXT_SECONDARY,
            );
        }

        response
    }
}
