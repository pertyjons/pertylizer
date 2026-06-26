//! Port widget for module connections.
//!
//! Each port type has a distinct shape for quick visual identification:
//! - **Audio** — circle
//! - **Control (CV)** — diamond
//! - **Gate** — square
//! - **MIDI** — hexagon

use eframe::egui::{self, Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};

use crate::gui::theme::theme;

/// A port widget for connections.
pub struct PortWidget {
    port_type: WidgetPortType,
    connected: bool,
    highlighted: bool,
}

/// Port type for widget rendering.
///
/// For signal routing definitions, see `modules::core::PortType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetPortType {
    Audio,
    Control,
    Gate,
    Midi,
}

impl WidgetPortType {
    /// The matching `synth_core::PortType` for routing-rule queries.
    fn to_core(self) -> synth_core::PortType {
        match self {
            Self::Audio => synth_core::PortType::Audio,
            Self::Control => synth_core::PortType::Control,
            Self::Gate => synth_core::PortType::Gate,
            Self::Midi => synth_core::PortType::Midi,
        }
    }

    /// Whether a signal from an output port of `self` may drive an input port
    /// of type `dest`. Delegates to the canonical
    /// [`synth_core::PortType::can_drive`] contract (shared with the MCP
    /// `check_connection` validator) so the patch editor and the engine-facing
    /// path never disagree about which connections are legal.
    #[must_use]
    pub fn can_drive(self, dest: Self) -> bool {
        self.to_core().can_drive(dest.to_core())
    }
}

/// Port direction for widget rendering.
///
/// For signal routing definitions, see `modules::core::PortDirection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetPortDirection {
    Input,
    Output,
}

impl PortWidget {
    pub fn new(port_type: WidgetPortType) -> Self {
        Self {
            port_type,
            connected: false,
            highlighted: false,
        }
    }

    pub fn connected(mut self, connected: bool) -> Self {
        self.connected = connected;
        self
    }

    /// Mark this port as a valid connection target (glowing highlight).
    pub fn highlighted(mut self, highlighted: bool) -> Self {
        self.highlighted = highlighted;
        self
    }

    pub fn color(&self) -> Color32 {
        let colors = &theme().colors;
        match self.port_type {
            WidgetPortType::Audio => colors.port_audio,
            WidgetPortType::Control => colors.port_control,
            WidgetPortType::Gate => colors.port_gate,
            WidgetPortType::Midi => colors.port_midi,
        }
    }

    pub fn show(self, ui: &mut Ui) -> (Response, Pos2) {
        let size = 20.0;
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());

        let painter = ui.painter();
        let center = rect.center();
        let radius = size / 2.0 - 2.0;
        let color = self.color();

        // Glowing highlight for valid connection targets
        if self.highlighted {
            let glow_color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 80);
            self.draw_shape(painter, center, radius + 6.0, glow_color, None);

            let inner_glow = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 120);
            self.draw_shape(painter, center, radius + 3.0, inner_glow, None);
        }

        // Outer ring (shape outline)
        self.draw_shape(
            painter,
            center,
            radius,
            Color32::TRANSPARENT,
            Some(Stroke::new(2.0, color)),
        );

        // Inner fill
        let fill = if self.connected {
            color
        } else {
            theme().colors.bg_dark
        };
        self.draw_shape(painter, center, radius - 3.0, fill, None);

        // Hover: brighter glow ring. The port name/type is shown via the
        // caller's egui `on_hover_text` tooltip (correctly positioned inside the
        // Scene transform) rather than a manually-painted Tooltip-layer label,
        // which used the world-space `center` and so landed off-screen.
        if response.hovered() {
            let hover_glow = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 60);
            self.draw_shape(painter, center, radius + 4.0, hover_glow, None);
            self.draw_shape(
                painter,
                center,
                radius + 4.0,
                Color32::TRANSPARENT,
                Some(Stroke::new(1.5, color.gamma_multiply(0.7))),
            );
        }

        (response, center)
    }

    /// Draw the port shape (filled and/or stroked) based on port type.
    fn draw_shape(
        &self,
        painter: &egui::Painter,
        center: Pos2,
        radius: f32,
        fill: Color32,
        stroke: Option<Stroke>,
    ) {
        let stroke = stroke.unwrap_or(Stroke::NONE);

        match self.port_type {
            WidgetPortType::Audio => {
                // Circle
                if fill != Color32::TRANSPARENT {
                    painter.circle_filled(center, radius, fill);
                }
                if stroke != Stroke::NONE {
                    painter.circle_stroke(center, radius, stroke);
                }
            }
            WidgetPortType::Control => {
                // Diamond (rotated square)
                let points = diamond_points(center, radius);
                draw_convex(painter, &points, fill, stroke);
            }
            WidgetPortType::Gate => {
                // Square
                let half = radius * 0.82; // visually match circle area
                let points = [
                    Pos2::new(center.x - half, center.y - half),
                    Pos2::new(center.x + half, center.y - half),
                    Pos2::new(center.x + half, center.y + half),
                    Pos2::new(center.x - half, center.y + half),
                ];
                draw_convex(painter, &points, fill, stroke);
            }
            WidgetPortType::Midi => {
                // Hexagon
                let points = hexagon_points(center, radius);
                draw_convex(painter, &points, fill, stroke);
            }
        }
    }
}

/// Generate diamond (rotated square) vertices.
fn diamond_points(center: Pos2, radius: f32) -> [Pos2; 4] {
    [
        Pos2::new(center.x, center.y - radius), // top
        Pos2::new(center.x + radius, center.y), // right
        Pos2::new(center.x, center.y + radius), // bottom
        Pos2::new(center.x - radius, center.y), // left
    ]
}

/// Generate hexagon vertices.
fn hexagon_points(center: Pos2, radius: f32) -> [Pos2; 6] {
    let mut points = [Pos2::ZERO; 6];
    for (i, point) in points.iter_mut().enumerate() {
        let angle = std::f32::consts::FRAC_PI_3 * i as f32 - std::f32::consts::FRAC_PI_6;
        *point = Pos2::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
    }
    points
}

/// Draw a filled and/or stroked convex polygon.
fn draw_convex(painter: &egui::Painter, points: &[Pos2], fill: Color32, stroke: Stroke) {
    use eframe::epaint::{PathShape, PathStroke};

    if fill != Color32::TRANSPARENT {
        // Vec allocation required by egui PathShape API
        painter.add(PathShape::convex_polygon(
            points.to_vec(),
            fill,
            PathStroke::NONE,
        ));
    }
    if stroke != Stroke::NONE {
        // Vec allocation required by egui PathShape API
        painter.add(PathShape::convex_polygon(
            points.to_vec(),
            Color32::TRANSPARENT,
            PathStroke::new(stroke.width, stroke.color),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::WidgetPortType::{Audio, Control, Gate, Midi};

    #[test]
    fn audio_and_control_are_interchangeable() {
        // The bug: dragging an audio output (e.g. an envelope) onto a control
        // input (e.g. a VCA cv) must be allowed, and vice versa.
        assert!(Audio.can_drive(Control));
        assert!(Control.can_drive(Audio));
        assert!(Audio.can_drive(Audio));
        assert!(Control.can_drive(Control));
    }

    #[test]
    fn gate_is_directional_with_control() {
        assert!(Gate.can_drive(Control));
        assert!(Gate.can_drive(Gate));
        // A control signal may not drive a gate input.
        assert!(!Control.can_drive(Gate));
        assert!(!Audio.can_drive(Gate));
    }

    #[test]
    fn midi_is_isolated() {
        assert!(Midi.can_drive(Midi));
        assert!(!Midi.can_drive(Audio));
        assert!(!Midi.can_drive(Control));
        assert!(!Audio.can_drive(Midi));
    }
}
