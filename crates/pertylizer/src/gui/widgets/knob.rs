//! Rotary knob widget.

use eframe::egui::{self, Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};

use crate::gui::theme::theme;
use synth_core::ValueRange;
use synth_core::{ParameterDescriptor, ParameterUnit, ResponseCurve};

/// A rotary knob widget.
pub struct Knob<'a> {
    value: &'a mut f32,
    range: ValueRange,
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
            range: ValueRange::symmetric(min, max),
            response_curve: ResponseCurve::Linear,
            unit: ParameterUnit::None,
            label: String::new(),
            size: 40.0, // Smaller default size
            accent_color: theme().colors.accent_orange,
        }
    }

    pub fn from_descriptor(value: &'a mut f32, descriptor: &ParameterDescriptor) -> Self {
        Self {
            value,
            range: descriptor.range,
            response_curve: descriptor.response_curve,
            unit: descriptor.unit,
            label: descriptor.name.clone(),
            size: 40.0, // Smaller default size
            accent_color: theme().colors.accent_orange,
        }
    }

    #[must_use]
    pub fn unit(mut self, unit: ParameterUnit) -> Self {
        self.unit = unit;
        self
    }

    /// Format the value with the appropriate unit suffix
    fn format_value(&self) -> String {
        self.unit.format(*self.value)
    }

    #[must_use]
    pub fn default(mut self, default: f32) -> Self {
        self.range = self.range.with_default(default);
        self
    }

    #[must_use]
    pub fn response_curve(mut self, curve: ResponseCurve) -> Self {
        self.response_curve = curve;
        self
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    #[must_use]
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn accent_color(mut self, color: Color32) -> Self {
        self.accent_color = color;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let t = theme();
        let padding = 2.0;
        let label_height = if self.label.is_empty() {
            0.0
        } else {
            12.0 // Compact label height
        };

        // Total widget size - minimal padding
        let widget_width = self.size + padding * 2.0;
        let widget_height = self.size + label_height + padding * 2.0;
        let desired_size = Vec2::new(widget_width, widget_height);

        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

        if response.double_clicked() {
            *self.value = self.range.default.clamp(self.range.min, self.range.max);
        }

        if response.dragged() {
            let delta = -response.drag_delta().y;
            let sensitivity = t.style.knob_sensitivity;
            let normalized = self.response_curve.normalize(*self.value, self.range);
            let new_normalized = (normalized + delta * sensitivity).clamp(0.0, 1.0);
            *self.value = self.response_curve.denormalize(new_normalized, self.range);
        }

        let painter = ui.painter();

        // The knob circle area
        let knob_size = self.size;
        let knob_rect = egui::Rect::from_center_size(
            Pos2::new(rect.center().x, rect.top() + padding + knob_size / 2.0),
            Vec2::splat(knob_size),
        );

        // Draw knob circle - no outer frame for compactness
        let center = knob_rect.center();
        let radius = knob_size / 2.0 - 1.0;

        // Background circle
        painter.circle_filled(center, radius, t.colors.bg_widget);
        painter.circle_stroke(center, radius, Stroke::new(1.0, t.colors.bg_panel));

        // Value arc
        let normalized = self.response_curve.normalize(*self.value, self.range);
        let start_angle = std::f32::consts::PI * 0.75;
        let end_angle = std::f32::consts::PI * 2.25;
        let value_angle = start_angle + normalized * (end_angle - start_angle);

        // Draw arc - scaled for smaller knob
        let arc_width = (self.size / 20.0).max(2.0);
        let segments = 24;
        let value_segments = ((normalized * segments as f32) as usize).min(segments);

        for i in 0..value_segments {
            let t0 = i as f32 / segments as f32;
            let t1 = (i + 1) as f32 / segments as f32;
            let a0 = start_angle + t0 * (end_angle - start_angle);
            let a1 = start_angle + t1 * (end_angle - start_angle);

            let inner_radius = radius - arc_width - 1.0;
            let p0 = center + Vec2::new(a0.cos(), a0.sin()) * inner_radius;
            let p1 = center + Vec2::new(a1.cos(), a1.sin()) * inner_radius;

            painter.line_segment([p0, p1], Stroke::new(arc_width, self.accent_color));
        }

        // Indicator dot at the edge - scaled
        let indicator_size = (self.size / 16.0).max(2.0);
        let indicator_radius = radius - arc_width - indicator_size - 1.0;
        let indicator_pos =
            center + Vec2::new(value_angle.cos(), value_angle.sin()) * indicator_radius;
        painter.circle_filled(indicator_pos, indicator_size, t.colors.text_primary);

        // Label below knob
        if !self.label.is_empty() {
            let label_pos = Pos2::new(rect.center().x, knob_rect.bottom() + 1.0);
            painter.text(
                label_pos,
                egui::Align2::CENTER_TOP,
                &self.label,
                egui::FontId::proportional(9.0),
                t.colors.text_secondary,
            );
        }

        // Show tooltip with value on hover or when dragging
        if response.hovered() || response.dragged() {
            let value_text = self.format_value();
            super::tooltip::draw_tooltip_right_of(
                ui,
                center,
                radius,
                &value_text,
                self.accent_color,
            );
        }

        response
    }
}
