//! Port widget for module connections.

use eframe::egui::{Color32, Pos2, Response, Sense, Stroke, Ui, Vec2};

use crate::gui::theme::theme;

/// A port widget for connections.
pub struct PortWidget {
    port_type: WidgetPortType,
    #[allow(dead_code)] // Semantic field for future connection logic
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

    pub fn show(self, ui: &mut Ui) -> (Response, Pos2) {
        let size = 20.0;
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());

        let painter = ui.painter();
        let center = rect.center();
        let radius = size / 2.0 - 2.0;

        // Glowing highlight for valid connection targets
        if self.highlighted {
            // Outer glow
            let glow_color = Color32::from_rgba_unmultiplied(
                self.color().r(),
                self.color().g(),
                self.color().b(),
                80,
            );
            painter.circle_filled(center, radius + 6.0, glow_color);

            // Inner glow
            let inner_glow = Color32::from_rgba_unmultiplied(
                self.color().r(),
                self.color().g(),
                self.color().b(),
                120,
            );
            painter.circle_filled(center, radius + 3.0, inner_glow);
        }

        // Outer ring
        painter.circle_stroke(center, radius, Stroke::new(2.0, self.color()));

        // Inner fill (if connected)
        if self.connected {
            painter.circle_filled(center, radius - 3.0, self.color());
        } else {
            painter.circle_filled(center, radius - 3.0, theme().colors.bg_dark);
        }

        // Highlight on hover
        if response.hovered() {
            painter.circle_stroke(
                center,
                radius + 2.0,
                Stroke::new(1.0, self.color().gamma_multiply(0.5)),
            );
        }

        (response, center)
    }
}
