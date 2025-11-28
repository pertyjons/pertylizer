//! Reusable UI widgets for the synthesizer GUI.
//!
//! This module provides custom widgets like knobs, sliders, meters,
//! and module frames that can be used across different module panels.

use eframe::egui::{self, Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use crate::modules::core::{ParameterDescriptor, ResponseCurve};

// Re-export theme for convenience
pub use super::theme::{theme, with_theme_mut, set_theme, Theme};

// Re-export colors for backwards compatibility
pub use super::theme::colors;

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
        let knob_rect = Rect::from_min_size(rect.min, Vec2::splat(self.size));

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

/// A vertical meter widget for audio levels.
pub struct Meter {
    peak: f32,
    rms: f32,
    width: f32,
    height: f32,
}

impl Meter {
    pub fn new(peak: f32, rms: f32) -> Self {
        Self {
            peak,
            rms,
            width: 12.0,
            height: 100.0,
        }
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let desired_size = Vec2::new(self.width, self.height);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::hover());

        let painter = ui.painter();

        // Background
        painter.rect_filled(rect, 2.0, colors::BG_DARK);

        // RMS level
        let rms_height = rect.height() * self.rms.clamp(0.0, 1.0);
        let rms_rect = Rect::from_min_size(
            Pos2::new(rect.left(), rect.bottom() - rms_height),
            Vec2::new(rect.width(), rms_height),
        );

        let rms_color = if self.rms > 0.9 {
            colors::METER_RED
        } else if self.rms > 0.7 {
            colors::METER_YELLOW
        } else {
            colors::METER_GREEN
        };

        painter.rect_filled(rms_rect, 0.0, rms_color);

        // Peak indicator
        let peak_y = rect.bottom() - rect.height() * self.peak.clamp(0.0, 1.0);
        painter.line_segment(
            [
                Pos2::new(rect.left(), peak_y),
                Pos2::new(rect.right(), peak_y),
            ],
            Stroke::new(2.0, colors::TEXT_PRIMARY),
        );

        response
    }
}

/// A port widget for connections.
pub struct Port {
    port_type: PortType,
    direction: PortDirection,
    connected: bool,
    label: String,
}

/// Port type enum (mirrors the one in modules::core).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortType {
    Audio,
    Control,
    Gate,
    Midi,
}

/// Port direction enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortDirection {
    Input,
    Output,
}

impl Port {
    pub fn new(port_type: PortType, direction: PortDirection) -> Self {
        Self {
            port_type,
            direction,
            connected: false,
            label: String::new(),
        }
    }

    pub fn connected(mut self, connected: bool) -> Self {
        self.connected = connected;
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn color(&self) -> Color32 {
        match self.port_type {
            PortType::Audio => colors::PORT_AUDIO,
            PortType::Control => colors::PORT_CONTROL,
            PortType::Gate => colors::PORT_GATE,
            PortType::Midi => colors::PORT_MIDI,
        }
    }

    pub fn show(self, ui: &mut Ui) -> (Response, Pos2) {
        let size = 14.0;
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());

        let painter = ui.painter();
        let center = rect.center();
        let radius = size / 2.0 - 1.0;

        // Outer ring
        painter.circle_stroke(center, radius, Stroke::new(2.0, self.color()));

        // Inner fill (if connected)
        if self.connected {
            painter.circle_filled(center, radius - 3.0, self.color());
        } else {
            painter.circle_filled(center, radius - 3.0, colors::BG_DARK);
        }

        // Highlight on hover
        if response.hovered() {
            painter.circle_stroke(center, radius + 2.0, Stroke::new(1.0, self.color().gamma_multiply(0.5)));
        }

        (response, center)
    }
}

/// Draw a module frame with title and accent color.
pub fn module_frame(
    ui: &mut Ui,
    title: &str,
    accent_color: Color32,
    add_contents: impl FnOnce(&mut Ui),
) {
    egui::Frame::new()
        .fill(colors::BG_MODULE)
        .corner_radius(8.0)
        .stroke(Stroke::new(1.0, accent_color.gamma_multiply(0.5)))
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Colored accent bar
                let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 16.0), Sense::hover());
                ui.painter().rect_filled(rect, 2.0, accent_color);

                ui.label(egui::RichText::new(title).color(accent_color).strong());
            });

            ui.add_space(8.0);
            add_contents(ui);
        });
}

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
    painter.rect_filled(rect, 4.0, colors::BG_DARK);

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
        Pos2::new(rect.left(), bottom),           // Start
        Pos2::new(attack_x, top),                 // Peak after attack
        Pos2::new(decay_x, sustain_y),            // Sustain level after decay
        Pos2::new(sustain_x, sustain_y),          // Hold sustain
        Pos2::new(release_x, bottom),             // End of release
    ];

    for i in 0..points.len() - 1 {
        painter.line_segment(
            [points[i], points[i + 1]],
            Stroke::new(2.0, colors::ACCENT_CYAN),
        );
    }

    // Draw dots at key points
    for point in &points[1..4] {
        painter.circle_filled(*point, 3.0, colors::ACCENT_CYAN);
    }
}

/// Draw a cable between two points.
pub fn draw_cable(painter: &egui::Painter, from: Pos2, to: Pos2, color: Color32) {
    // Bezier curve for natural cable look
    let control_offset = (to.x - from.x).abs() * 0.5;
    let ctrl1 = Pos2::new(from.x + control_offset, from.y);
    let ctrl2 = Pos2::new(to.x - control_offset, to.y);

    // Approximate bezier with line segments
    let segments = 32;
    let mut prev = from;

    for i in 1..=segments {
        let t = i as f32 / segments as f32;
        let t2 = t * t;
        let t3 = t2 * t;
        let mt = 1.0 - t;
        let mt2 = mt * mt;
        let mt3 = mt2 * mt;

        let x = mt3 * from.x + 3.0 * mt2 * t * ctrl1.x + 3.0 * mt * t2 * ctrl2.x + t3 * to.x;
        let y = mt3 * from.y + 3.0 * mt2 * t * ctrl1.y + 3.0 * mt * t2 * ctrl2.y + t3 * to.y;
        let curr = Pos2::new(x, y);

        painter.line_segment([prev, curr], Stroke::new(3.0, color));
        prev = curr;
    }
}

/// Waveform selector widget with visual preview.
/// Shows a row of waveform buttons with visual representation of each waveform.
pub struct WaveformSelector<'a> {
    selected: &'a mut usize,
    waveforms: Vec<WaveformType>,
    accent_color: Color32,
}

/// Types of waveforms to display
#[derive(Clone, Copy, PartialEq)]
pub enum WaveformType {
    Sine,
    Triangle,
    Sawtooth,
    Square,
    Pulse,
    Noise,
}

impl WaveformType {
    /// Get all standard waveforms
    pub fn all() -> Vec<Self> {
        vec![
            Self::Sine,
            Self::Triangle,
            Self::Sawtooth,
            Self::Square,
            Self::Pulse,
            Self::Noise,
        ]
    }

    /// Get the name of the waveform
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Triangle => "Triangle",
            Self::Sawtooth => "Saw",
            Self::Square => "Square",
            Self::Pulse => "Pulse",
            Self::Noise => "Noise",
        }
    }

    /// Generate sample points for visualization (0 to 1 normalized x, -1 to 1 y)
    pub fn sample(&self, x: f32) -> f32 {
        match self {
            Self::Sine => (x * std::f32::consts::TAU).sin(),
            Self::Triangle => {
                if x < 0.5 {
                    4.0 * x - 1.0
                } else {
                    3.0 - 4.0 * x
                }
            }
            Self::Sawtooth => 2.0 * x - 1.0,
            Self::Square => if x < 0.5 { 1.0 } else { -1.0 },
            Self::Pulse => if x < 0.25 { 1.0 } else { -1.0 },
            Self::Noise => {
                // Pseudo-random based on x position for consistent visualization
                let seed = (x * 1000.0) as u32;
                let hash = seed.wrapping_mul(2654435761);
                ((hash as f32) / (u32::MAX as f32)) * 2.0 - 1.0
            }
        }
    }
}

impl<'a> WaveformSelector<'a> {
    pub fn new(selected: &'a mut usize) -> Self {
        Self {
            selected,
            waveforms: WaveformType::all(),
            accent_color: colors::ACCENT_ORANGE,
        }
    }

    pub fn waveforms(mut self, waveforms: Vec<WaveformType>) -> Self {
        self.waveforms = waveforms;
        self
    }

    pub fn accent_color(mut self, color: Color32) -> Self {
        self.accent_color = color;
        self
    }

    /// Show the waveform selector and return true if selection changed
    pub fn show(self, ui: &mut Ui) -> bool {
        let mut changed = false;
        let button_size = Vec2::new(40.0, 30.0);
        
        ui.horizontal(|ui| {
            for (i, waveform) in self.waveforms.iter().enumerate() {
                let is_selected = i == *self.selected;
                
                let (rect, response) = ui.allocate_exact_size(button_size, Sense::click());
                
                if response.clicked() {
                    *self.selected = i;
                    changed = true;
                }

                // Draw button background
                let bg_color = if is_selected {
                    self.accent_color.gamma_multiply(0.3)
                } else if response.hovered() {
                    colors::BG_WIDGET.gamma_multiply(1.3)
                } else {
                    colors::BG_WIDGET
                };

                let stroke_color = if is_selected {
                    self.accent_color
                } else {
                    colors::TEXT_DIM
                };

                ui.painter().rect(
                    rect,
                    4.0,
                    bg_color,
                    Stroke::new(1.0, stroke_color),
                    egui::StrokeKind::Outside,
                );

                // Draw waveform preview
                let inner_rect = rect.shrink(4.0);
                self.draw_waveform(ui.painter(), inner_rect, *waveform, is_selected);

                // Tooltip with name
                response.on_hover_text(waveform.name());
            }
        });

        changed
    }

    fn draw_waveform(&self, painter: &egui::Painter, rect: Rect, waveform: WaveformType, is_selected: bool) {
        let color = if is_selected {
            self.accent_color
        } else {
            colors::TEXT_SECONDARY
        };

        let samples = 32;
        let mut points = Vec::with_capacity(samples);

        for i in 0..samples {
            let x_norm = i as f32 / (samples - 1) as f32;
            let y_norm = waveform.sample(x_norm);
            
            let x = rect.left() + x_norm * rect.width();
            let y = rect.center().y - y_norm * rect.height() * 0.4;
            
            points.push(Pos2::new(x, y));
        }

        // Draw the waveform line
        for i in 0..points.len() - 1 {
            painter.line_segment(
                [points[i], points[i + 1]],
                Stroke::new(1.5, color),
            );
        }
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
    pub fn new(attack: &'a mut f32, decay: &'a mut f32, sustain: &'a mut f32, release: &'a mut f32) -> Self {
        Self {
            attack,
            decay,
            sustain,
            release,
            accent_color: colors::ACCENT_GREEN,
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
        let (rect, response) = ui.allocate_exact_size(Vec2::new(self.width, self.height), Sense::click_and_drag());

        let painter = ui.painter();

        // Background
        painter.rect_filled(rect, 4.0, colors::BG_WIDGET);
        painter.rect_stroke(rect, 4.0, Stroke::new(1.0, colors::TEXT_DIM), egui::StrokeKind::Outside);

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
        let scale = if total_time > 0.0 { 1.0 / total_time } else { 1.0 };
        
        let attack_x = left + attack_norm * scale * width;
        let decay_x = attack_x + decay_norm * scale * width;
        let sustain_x = decay_x + 0.2 * scale * width;
        let release_x = right.min(sustain_x + release_norm * scale * width);

        // Y positions
        let sustain_y = top + (1.0 - sustain_norm) * height;

        // Draw envelope shape
        let points = [
            Pos2::new(left, bottom),          // Start
            Pos2::new(attack_x, top),         // Peak after attack
            Pos2::new(decay_x, sustain_y),    // After decay
            Pos2::new(sustain_x, sustain_y),  // Sustain hold
            Pos2::new(release_x, bottom),     // End
        ];

        // Fill
        let fill_points: Vec<_> = points.iter().copied()
            .chain(std::iter::once(Pos2::new(left, bottom)))
            .collect();
        
        painter.add(egui::Shape::convex_polygon(
            fill_points,
            self.accent_color.gamma_multiply(0.2),
            Stroke::NONE,
        ));

        // Lines
        for i in 0..points.len() - 1 {
            painter.line_segment([points[i], points[i + 1]], Stroke::new(2.0, self.accent_color));
        }

        // Control points (draggable)
        let control_points = [
            (Pos2::new(attack_x, top), "A"),
            (Pos2::new(decay_x, sustain_y), "D"),
            (Pos2::new(sustain_x, sustain_y), "S"),
        ];

        for (point, label) in &control_points {
            let is_hovered = response.hovered() && 
                (ui.input(|i| i.pointer.hover_pos())
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
                colors::TEXT_DIM,
            );
        }

        // Handle dragging
        if response.dragged() {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                // Find nearest control point
                let distances = [
                    ((pos - Pos2::new(attack_x, top)).length(), 0),
                    ((pos - Pos2::new(decay_x, sustain_y)).length(), 1),
                    ((pos - Pos2::new(sustain_x, sustain_y)).length(), 2),
                ];
                
                let nearest = distances.iter().min_by(|a, b| a.0.partial_cmp(&b.0).unwrap()).unwrap().1;

                match nearest {
                    0 => {
                        // Attack - horizontal drag
                        let new_attack = ((pos.x - left) / width * max_time / scale).clamp(0.001, max_time);
                        if (new_attack - *self.attack).abs() > 0.001 {
                            *self.attack = new_attack;
                            changed = true;
                        }
                    }
                    1 => {
                        // Decay - horizontal and vertical
                        let new_decay = ((pos.x - attack_x) / width * max_time / scale).clamp(0.001, max_time);
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
        }

        changed
    }
}

// ============================================================================
// VISUALIZATION WIDGETS
// ============================================================================

/// Draw an oscilloscope display.
/// Shows a waveform visualization with optional grid.
pub fn draw_oscilloscope(
    ui: &mut Ui,
    samples: &[f32],
    width: f32,
    height: f32,
    gain: f32,
    color: Color32,
) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 4.0, colors::BG_DARK);

    // Draw grid
    let grid_color = Color32::from_rgba_premultiplied(60, 65, 75, 100);
    
    // Horizontal center line
    let center_y = rect.center().y;
    painter.line_segment(
        [Pos2::new(rect.left(), center_y), Pos2::new(rect.right(), center_y)],
        Stroke::new(1.0, grid_color),
    );
    
    // Vertical grid lines
    for i in 1..4 {
        let x = rect.left() + rect.width() * (i as f32 / 4.0);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, grid_color),
        );
    }

    // Draw waveform
    if samples.len() >= 2 {
        let step = samples.len() as f32 / width;
        let half_height = (height - 8.0) / 2.0;
        
        let mut points: Vec<Pos2> = Vec::with_capacity(width as usize);
        
        for x in 0..(width as usize) {
            let sample_idx = ((x as f32) * step) as usize;
            if sample_idx < samples.len() {
                let sample = samples[sample_idx] * gain;
                let y = center_y - sample.clamp(-1.0, 1.0) * half_height;
                points.push(Pos2::new(rect.left() + x as f32, y));
            }
        }
        
        // Draw as connected line segments
        for i in 0..points.len().saturating_sub(1) {
            painter.line_segment([points[i], points[i + 1]], Stroke::new(1.5, color));
        }
    } else {
        // No data - draw flat line
        painter.line_segment(
            [Pos2::new(rect.left() + 4.0, center_y), Pos2::new(rect.right() - 4.0, center_y)],
            Stroke::new(1.5, color.gamma_multiply(0.5)),
        );
    }

    // Border
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, colors::BORDER), egui::StrokeKind::Outside);
}

/// Draw a level meter (VU meter style).
/// Shows peak and RMS levels with optional peak hold.
pub fn draw_level_meter(
    ui: &mut Ui,
    peak: f32,
    rms: f32,
    width: f32,
    height: f32,
    horizontal: bool,
) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 2.0, colors::BG_DARK);

    // Calculate bar positions
    let peak_db = 20.0 * peak.max(0.0001).log10();
    let rms_db = 20.0 * rms.max(0.0001).log10();
    
    // Map dB to 0-1 range (-60dB to 0dB)
    let peak_norm = ((peak_db + 60.0) / 60.0).clamp(0.0, 1.0);
    let rms_norm = ((rms_db + 60.0) / 60.0).clamp(0.0, 1.0);

    let padding = 2.0;
    let inner_rect = rect.shrink(padding);

    if horizontal {
        // RMS bar
        let rms_width = inner_rect.width() * rms_norm;
        let rms_rect = Rect::from_min_size(
            inner_rect.min,
            Vec2::new(rms_width, inner_rect.height()),
        );
        
        // Gradient from green to yellow to red
        let rms_color = level_color(rms_norm);
        painter.rect_filled(rms_rect, 1.0, rms_color);
        
        // Peak indicator line
        let peak_x = inner_rect.left() + inner_rect.width() * peak_norm;
        painter.line_segment(
            [Pos2::new(peak_x, inner_rect.top()), Pos2::new(peak_x, inner_rect.bottom())],
            Stroke::new(2.0, colors::TEXT_PRIMARY),
        );
    } else {
        // Vertical meter
        // RMS bar (from bottom up)
        let rms_height = inner_rect.height() * rms_norm;
        let rms_rect = Rect::from_min_max(
            Pos2::new(inner_rect.left(), inner_rect.bottom() - rms_height),
            inner_rect.max,
        );
        
        let rms_color = level_color(rms_norm);
        painter.rect_filled(rms_rect, 1.0, rms_color);
        
        // Peak indicator line
        let peak_y = inner_rect.bottom() - inner_rect.height() * peak_norm;
        painter.line_segment(
            [Pos2::new(inner_rect.left(), peak_y), Pos2::new(inner_rect.right(), peak_y)],
            Stroke::new(2.0, colors::TEXT_PRIMARY),
        );
    }

    // Clip indicator at top
    if peak_norm > 0.98 {
        let clip_rect = if horizontal {
            Rect::from_min_size(
                Pos2::new(rect.right() - 6.0, rect.top()),
                Vec2::new(6.0, rect.height()),
            )
        } else {
            Rect::from_min_size(
                rect.min,
                Vec2::new(rect.width(), 4.0),
            )
        };
        painter.rect_filled(clip_rect, 1.0, Color32::RED);
    }

    // Border
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, colors::BORDER), egui::StrokeKind::Outside);
}

/// Draw a stereo level meter (two vertical bars side by side).
pub fn draw_stereo_meter(
    ui: &mut Ui,
    peak_l: f32,
    peak_r: f32,
    rms_l: f32,
    rms_r: f32,
    width: f32,
    height: f32,
) {
    let (rect, _response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter();

    // Background
    painter.rect_filled(rect, 4.0, colors::BG_DARK);

    let bar_width = (width - 10.0) / 2.0;
    let padding = 2.0;

    // Left meter
    let left_rect = Rect::from_min_size(
        Pos2::new(rect.left() + padding, rect.top() + padding),
        Vec2::new(bar_width, height - padding * 2.0),
    );
    draw_meter_bar(painter, left_rect, peak_l, rms_l);

    // Right meter
    let right_rect = Rect::from_min_size(
        Pos2::new(rect.left() + bar_width + 6.0, rect.top() + padding),
        Vec2::new(bar_width, height - padding * 2.0),
    );
    draw_meter_bar(painter, right_rect, peak_r, rms_r);

    // Labels
    let t = theme();
    let label_y = rect.bottom() - 12.0;
    painter.text(
        Pos2::new(left_rect.center().x, label_y),
        egui::Align2::CENTER_CENTER,
        "L",
        t.fonts.small(),
        colors::TEXT_DIM,
    );
    painter.text(
        Pos2::new(right_rect.center().x, label_y),
        egui::Align2::CENTER_CENTER,
        "R",
        t.fonts.small(),
        colors::TEXT_DIM,
    );

    // Border
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, colors::BORDER), egui::StrokeKind::Outside);
}

/// Helper to draw a single meter bar.
fn draw_meter_bar(painter: &egui::Painter, rect: Rect, peak: f32, rms: f32) {
    // Calculate levels
    let peak_db = 20.0 * peak.max(0.0001).log10();
    let rms_db = 20.0 * rms.max(0.0001).log10();
    
    let peak_norm = ((peak_db + 60.0) / 60.0).clamp(0.0, 1.0);
    let rms_norm = ((rms_db + 60.0) / 60.0).clamp(0.0, 1.0);

    // Background
    painter.rect_filled(rect, 1.0, Color32::from_rgb(25, 28, 35));

    // Draw segments
    let num_segments = 20;
    let segment_height = (rect.height() - 14.0) / num_segments as f32;
    let gap = 1.0;

    for i in 0..num_segments {
        let level = (i + 1) as f32 / num_segments as f32;
        let y = rect.bottom() - 14.0 - (i + 1) as f32 * segment_height;
        
        let segment_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 1.0, y + gap),
            Vec2::new(rect.width() - 2.0, segment_height - gap * 2.0),
        );

        let color = if level <= rms_norm {
            level_color(level)
        } else {
            Color32::from_rgb(35, 38, 45)
        };

        painter.rect_filled(segment_rect, 0.0, color);
    }

    // Peak indicator
    if peak_norm > 0.01 {
        let peak_y = rect.bottom() - 14.0 - rect.height() * peak_norm + 7.0;
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.left() + 1.0, peak_y),
                Vec2::new(rect.width() - 2.0, 2.0),
            ),
            0.0,
            colors::TEXT_PRIMARY,
        );
    }
}

/// Get color for level value (green -> yellow -> red).
fn level_color(level: f32) -> Color32 {
    if level < 0.6 {
        // Green
        Color32::from_rgb(80, 200, 120)
    } else if level < 0.85 {
        // Yellow
        Color32::from_rgb(220, 180, 50)
    } else {
        // Red
        Color32::from_rgb(220, 80, 80)
    }
}
