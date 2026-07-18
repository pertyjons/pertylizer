//! Shared Rack-led module port presentation and column rendering.

use std::borrow::Cow;

use eframe::egui::{self, Pos2, Ui, Vec2};

use super::{
    CaptionTone, ModMarkers, PortWidget, WidgetPortDirection, WidgetPortType, caption, expose,
};
use crate::gui::theme::theme;

/// Endpoint contract needed by the domain-neutral port presentation.
pub(crate) trait ModulePortEndpoint: Copy {
    fn widget_port_type(self) -> WidgetPortType;
}

/// Build a stable, descriptive accessibility name for a module port.
pub(crate) fn module_port_accessible_label(
    owner: &str,
    port_id: &str,
    label: &str,
    port_type: WidgetPortType,
    direction: WidgetPortDirection,
) -> String {
    let type_label = port_type_label(port_type);
    let direction_label = match direction {
        WidgetPortDirection::Input => "input",
        WidgetPortDirection::Output => "output",
    };
    format!("{owner}, {label}, {type_label} {direction_label}, port {port_id}")
}

/// Domain-neutral presentation of one module port.
pub(crate) struct ModulePort<'a, Endpoint> {
    endpoint: Endpoint,
    label: Cow<'a, str>,
    accessible_label: Cow<'a, str>,
    description: Cow<'a, str>,
    connected: bool,
    highlighted: bool,
    markers: ModMarkers,
}

impl<'a, Endpoint: ModulePortEndpoint> ModulePort<'a, Endpoint> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        endpoint: Endpoint,
        label: impl Into<Cow<'a, str>>,
        accessible_label: impl Into<Cow<'a, str>>,
        description: impl Into<Cow<'a, str>>,
        connected: bool,
        highlighted: bool,
        markers: ModMarkers,
    ) -> Self {
        Self {
            endpoint,
            label: label.into(),
            accessible_label: accessible_label.into(),
            description: description.into(),
            connected,
            highlighted,
            markers,
        }
    }

    pub(crate) const fn endpoint(&self) -> Endpoint {
        self.endpoint
    }

    pub(crate) fn port_type(&self) -> WidgetPortType {
        self.endpoint.widget_port_type()
    }
}

/// Draw one fixed-width Rack-style IN or OUT port column.
pub(crate) fn draw_module_port_column<Endpoint: ModulePortEndpoint>(
    ui: &mut Ui,
    direction: WidgetPortDirection,
    ports: &[ModulePort<'_, Endpoint>],
    mut on_port: impl FnMut(&ModulePort<Endpoint>, Pos2, &egui::Response),
) {
    let t = theme();
    let col_width = t.sizes.port_column_width;
    let spacing = t.sizes.port_vertical_spacing;

    ui.vertical(|ui| {
        ui.set_width(col_width);
        if ports.is_empty() {
            return;
        }

        ui.vertical_centered(|ui| {
            caption(
                ui,
                match direction {
                    WidgetPortDirection::Input => "IN",
                    WidgetPortDirection::Output => "OUT",
                },
                CaptionTone::Dim,
            );
        });

        let rail_x = ui.cursor().min.x + col_width * 0.5;
        let rail_top = ui.cursor().min.y + 3.0;
        let rail_bottom = rail_top + ports.len() as f32 * spacing - 6.0;
        ui.painter().line_segment(
            [Pos2::new(rail_x, rail_top), Pos2::new(rail_x, rail_bottom)],
            egui::Stroke::new(1.0, t.colors.border.gamma_multiply(0.55)),
        );

        for port in ports {
            ui.vertical_centered(|ui| {
                ui.allocate_ui(Vec2::new(col_width, spacing), |ui| {
                    ui.centered_and_justified(|ui| {
                        let port_type = port.port_type();
                        let (response, center) = PortWidget::new(port_type)
                            .connected(port.connected)
                            .highlighted(port.highlighted)
                            .markers(port.markers)
                            .show(ui);
                        on_port(port, center, &response);

                        let type_label = port_type_label(port_type);
                        expose(
                            &response,
                            egui::WidgetType::Other,
                            port.accessible_label.as_ref(),
                            None,
                        );
                        response.on_hover_ui(|ui| {
                            let mut tooltip = format!("{} ({type_label})", port.label);
                            if !port.description.is_empty() {
                                tooltip.push('\n');
                                tooltip.push_str(&port.description);
                            }
                            ui.label(tooltip);
                        });
                    });
                });
            });
        }
    });
}

const fn port_type_label(port_type: WidgetPortType) -> &'static str {
    match port_type {
        WidgetPortType::Audio => "audio",
        WidgetPortType::Control => "cv",
        WidgetPortType::Gate => "gate",
        WidgetPortType::Midi => "midi",
        WidgetPortType::NoteStream => "notes",
    }
}

/// Compose the shared Rack-style `IN | body | OUT` row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModuleColumn {
    Input,
    Body,
    Output,
}

pub(crate) fn draw_module_port_layout(
    ui: &mut Ui,
    body_width: f32,
    mut draw: impl FnMut(ModuleColumn, &mut Ui),
) {
    let sizes = theme().sizes;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = sizes.module_port_gap;
        ui.vertical(|ui| {
            ui.set_width(sizes.port_column_width);
            draw(ModuleColumn::Input, ui);
        });
        ui.vertical(|ui| {
            ui.set_width(body_width.max(0.0));
            draw(ModuleColumn::Body, ui);
        });
        ui.vertical(|ui| {
            ui.set_width(sizes.port_column_width);
            draw(ModuleColumn::Output, ui);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessible_label_contains_owner_type_direction_and_stable_port_id() {
        let label = module_port_accessible_label(
            "osc-1",
            "out",
            "Signal",
            WidgetPortType::Audio,
            WidgetPortDirection::Output,
        );

        assert_eq!(label, "osc-1, Signal, audio output, port out");
    }
}
