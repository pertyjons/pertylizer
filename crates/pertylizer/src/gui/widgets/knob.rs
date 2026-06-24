//! Rotary knob widget.

use eframe::egui::{self, Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};

use crate::gui::theme::theme;
use synth_core::ValueRange;
use synth_core::{ParamKind, ParameterDescriptor, ParameterUnit, ResponseCurve};

/// Pixels of vertical drag per discrete step on a stepped (integer/quantized)
/// knob. Range-independent on purpose, so small-range integer knobs (e.g. MSEG
/// segment counts / loop points) aren't sluggish — every `step` is the same
/// drag distance regardless of how many steps the range spans.
const STEP_DRAG_PIXELS: f32 = 8.0;

/// Scroll units per discrete step on a stepped knob (≈ one mouse-wheel notch);
/// a smooth trackpad swipe accumulates toward it. Tunable feel constant.
const SCROLL_UNITS_PER_STEP: f32 = 40.0;

/// Continuous-knob scroll sensitivity (normalized value per scroll unit), so a
/// wheel notch nudges the value a couple of percent. Tunable feel constant.
const SCROLL_CONT_SENSITIVITY: f32 = 0.000_625;

/// A rotary knob widget.
pub struct Knob<'a> {
    value: &'a mut f32,
    range: ValueRange,
    response_curve: ResponseCurve,
    unit: ParameterUnit,
    /// Value-kind, driving display (decimal-free integers, `On`/`Off` bools) and
    /// the default integer snapping step.
    kind: ParamKind,
    label: String,
    size: f32,
    accent_color: Color32,
    /// Optional quantization step in value units (e.g. `1.0` for integer knobs).
    /// Drags snap to the nearest multiple of `step` offset from `range.min`.
    step: Option<f32>,
    /// Optional Mod Matrix marker `(glyph, color)` painted in the knob's corner —
    /// kept off the label so it never widens the cell (S1.5a).
    mod_marker: Option<(&'static str, Color32)>,
}

impl<'a> Knob<'a> {
    pub fn new(value: &'a mut f32, min: f32, max: f32) -> Self {
        Self {
            value,
            range: ValueRange::symmetric(min, max),
            response_curve: ResponseCurve::Linear,
            unit: ParameterUnit::None,
            kind: ParamKind::Continuous,
            label: String::new(),
            size: 40.0, // Smaller default size
            accent_color: theme().colors.accent_orange,
            step: None,
            mod_marker: None,
        }
    }

    pub fn from_descriptor(value: &'a mut f32, descriptor: &ParameterDescriptor) -> Self {
        Self {
            value,
            range: descriptor.range,
            response_curve: descriptor.response_curve,
            unit: descriptor.unit,
            kind: descriptor.kind,
            label: descriptor.name.clone(),
            size: 40.0, // Smaller default size
            accent_color: theme().colors.accent_orange,
            // Integer params snap to whole numbers — the kind supplies the step.
            step: (descriptor.kind == ParamKind::Integer).then_some(1.0),
            mod_marker: None,
        }
    }

    /// Set a quantization step in value units (drags snap to it).
    #[must_use]
    pub fn step(mut self, step: f32) -> Self {
        self.step = (step > 0.0).then_some(step);
        self
    }

    /// Snap a value to the nearest `step` multiple offset from `range.min`,
    /// clamped to the range. No-op when no step is set.
    fn snapped(&self, value: f32) -> f32 {
        match self.step {
            Some(step) if step > 0.0 => {
                let snapped = self.range.min + ((value - self.range.min) / step).round() * step;
                snapped.clamp(self.range.min, self.range.max)
            }
            _ => value,
        }
    }

    /// Paint a small Mod Matrix marker glyph in the knob's top-right corner.
    #[must_use]
    pub fn mod_marker(mut self, glyph: &'static str, color: Color32) -> Self {
        self.mod_marker = Some((glyph, color));
        self
    }

    #[must_use]
    pub fn unit(mut self, unit: ParameterUnit) -> Self {
        self.unit = unit;
        self
    }

    /// Format the value, kind-aware (decimal-free integers, `On`/`Off` bools).
    fn format_value(&self) -> String {
        self.kind.format(self.unit, *self.value)
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
            *self.value = self.snapped(self.range.default.clamp(self.range.min, self.range.max));
        }

        if response.dragged() {
            let delta = -response.drag_delta().y;
            if let Some(step) = self.step {
                // Stepped (integer/quantized) knob: accumulate an unsnapped value
                // across frames in egui memory so sub-step drag isn't discarded by
                // snapping each frame, and move in value space at a fixed
                // pixels-per-step rate (range-independent — small ranges stay
                // responsive). Snap only the value we write out.
                let acc_id = response.id.with("knob_drag_acc");
                let prev = if response.drag_started() {
                    *self.value
                } else {
                    ui.memory(|m| m.data.get_temp::<f32>(acc_id))
                        .unwrap_or(*self.value)
                };
                let acc = (prev + delta * (step / STEP_DRAG_PIXELS))
                    .clamp(self.range.min, self.range.max);
                ui.memory_mut(|m| m.data.insert_temp(acc_id, acc));
                *self.value = self.snapped(acc);
            } else {
                // Continuous knob: smooth movement in normalized (curve) space.
                let sensitivity = t.style.knob_sensitivity;
                let normalized = self.response_curve.normalize(*self.value, self.range);
                let new_normalized = (normalized + delta * sensitivity).clamp(0.0, 1.0);
                *self.value = self.response_curve.denormalize(new_normalized, self.range);
            }
        }

        // Mouse-wheel / trackpad scroll over the knob adjusts its value.
        if response.hovered() {
            // Negated: scroll down increases, scroll up decreases.
            let scroll = -ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(step) = self.step {
                    // Accumulate scroll units and emit whole steps, keeping the
                    // remainder so a smooth trackpad swipe ticks one step at a time
                    // (and a mouse notch ≈ one step).
                    let acc_id = response.id.with("knob_scroll_acc");
                    let acc = ui.memory(|m| m.data.get_temp::<f32>(acc_id)).unwrap_or(0.0) + scroll;
                    let steps = (acc / SCROLL_UNITS_PER_STEP).trunc();
                    if steps != 0.0 {
                        *self.value = self.snapped(
                            (*self.value + steps * step).clamp(self.range.min, self.range.max),
                        );
                    }
                    ui.memory_mut(|m| {
                        m.data
                            .insert_temp(acc_id, acc - steps * SCROLL_UNITS_PER_STEP)
                    });
                } else {
                    let normalized = self.response_curve.normalize(*self.value, self.range);
                    let new_normalized =
                        (normalized + scroll * SCROLL_CONT_SENSITIVITY).clamp(0.0, 1.0);
                    *self.value = self.response_curve.denormalize(new_normalized, self.range);
                }
                // Consume the vertical scroll so an enclosing ScrollArea (patch
                // editor / mixer) doesn't also scroll while adjusting the knob.
                ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
            }
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

        // Mod Matrix marker in the top-right corner (S1.5a) — same purple glyph
        // language as the module-header badge, painted over the knob so it never
        // changes the cell's footprint.
        if let Some((glyph, color)) = self.mod_marker {
            painter.text(
                Pos2::new(knob_rect.right(), knob_rect.top()),
                egui::Align2::RIGHT_TOP,
                glyph,
                egui::FontId::proportional(11.0),
                color,
            );
        }

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
