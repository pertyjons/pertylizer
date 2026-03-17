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
    direction: WidgetPortDirection,
    connected: bool,
    highlighted: bool,
    label: String,
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

/// Port direction for widget rendering.
///
/// For signal routing definitions, see `modules::core::PortDirection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetPortDirection {
    Input,
    Output,
}

impl PortWidget {
    pub fn new(port_type: WidgetPortType, direction: WidgetPortDirection) -> Self {
        Self {
            port_type,
            direction,
            connected: false,
            highlighted: false,
            label: String::new(),
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

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
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

    /// Short type label shown on hover.
    fn type_label(&self) -> &'static str {
        match self.port_type {
            WidgetPortType::Audio => "audio",
            WidgetPortType::Control => "cv",
            WidgetPortType::Gate => "gate",
            WidgetPortType::Midi => "midi",
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

        // Hover: brighter glow ring + port label
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

            // Draw port label on the tooltip layer so it renders on top of everything
            if !self.label.is_empty() {
                let label_text = format!("{} ({})", self.label, self.type_label());
                let label_pos = match self.direction {
                    WidgetPortDirection::Input => Pos2::new(center.x + radius + 8.0, center.y),
                    WidgetPortDirection::Output => Pos2::new(center.x - radius - 8.0, center.y),
                };
                let anchor = match self.direction {
                    WidgetPortDirection::Input => egui::Align2::LEFT_CENTER,
                    WidgetPortDirection::Output => egui::Align2::RIGHT_CENTER,
                };

                // Paint on tooltip layer to render above all modules
                let top_painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::new("port_label"),
                ));

                let galley = top_painter.layout_no_wrap(
                    label_text.clone(),
                    egui::FontId::proportional(10.0),
                    color,
                );
                let text_rect = anchor.anchor_size(label_pos, galley.size());
                let bg_rect = text_rect.expand(3.0);
                top_painter.rect_filled(bg_rect, 3.0, Color32::from_black_alpha(200));

                top_painter.text(
                    label_pos,
                    anchor,
                    label_text,
                    egui::FontId::proportional(10.0),
                    color,
                );
            }
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
