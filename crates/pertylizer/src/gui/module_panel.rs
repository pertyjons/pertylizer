//! Module panel rendering.
//!
//! This module handles rendering individual synthesizer modules
//! with their parameters and ports.

use eframe::egui::{self, Color32, Pos2, Response, Ui, Vec2};
use std::collections::HashMap;
use std::sync::Arc;

use synth_core::{
    ModuleCategory, ModuleDescriptor, ParameterDescriptor, PortDirection as CorePortDirection,
    PortType as CorePortType, WidgetHint,
};
use synth_core::{Param, PortName};
use synth_engine::ModuleId;
use synth_modules::EnvelopePositionBuffer;

use super::theme::theme;
use super::widgets::{Knob, PortWidget, WidgetPortDirection, WidgetPortType};

/// State for a module panel in the UI.
#[derive(Clone)]
pub struct ModulePanelState {
    /// Module ID.
    pub id: ModuleId,
    /// Position in the rack.
    pub position: Pos2,
    /// Last rendered size (used by auto-layout to avoid overlaps).
    pub size: Vec2,
    /// Cached parameter values (indexed by parameter name for lookup).
    /// Key is the parameter name from the descriptor.
    pub param_values: HashMap<String, f32>,
    /// Is this panel selected?
    #[allow(dead_code)]
    pub selected: bool,
    /// Is this panel being dragged?
    #[allow(dead_code)]
    pub dragging: bool,
    /// Envelope position buffer for envelope modules (lock-free GUI sync).
    pub envelope_position: Option<Arc<EnvelopePositionBuffer>>,
}

impl ModulePanelState {
    pub fn new(id: ModuleId, position: Pos2) -> Self {
        Self {
            id,
            position,
            size: Vec2::new(250.0, 200.0),
            param_values: HashMap::new(),
            selected: false,
            dragging: false,
            envelope_position: None,
        }
    }
}

/// Port position information for cable routing.
#[derive(Clone, Debug)]
pub struct PortPosition {
    pub module_id: ModuleId,
    pub port_name: PortName,
    pub position: Pos2,
    pub port_type: WidgetPortType,
    pub direction: WidgetPortDirection,
}

/// Result of drawing a module panel.
pub struct ModulePanelResult {
    /// The panel response for interaction.
    pub response: Response,
    /// Port positions for cable routing.
    pub port_positions: Vec<PortPosition>,
    /// Parameter changes that were made.
    /// Each Param carries its own value.
    pub param_changes: Vec<Param>,
}

/// Draw a module panel.
#[allow(dead_code)]
pub fn draw_module_panel(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    descriptor: &ModuleDescriptor,
    connected_ports: &[PortName],
) -> ModulePanelResult {
    let accent_color = category_color(descriptor.category);
    let mut port_positions = Vec::new();
    let mut param_changes = Vec::new();

    // Calculate panel size based on content - kompakta storlekar
    let panel_width = 140.0;
    let min_height = 80.0;

    let frame_response = egui::Frame::new()
        .fill(theme().colors.bg_module)
        .corner_radius(8.0)
        .stroke(egui::Stroke::new(
            if state.selected { 2.0 } else { 1.0 },
            if state.selected {
                accent_color
            } else {
                accent_color.gamma_multiply(0.5)
            },
        ))
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_min_width(panel_width);
            ui.set_min_height(min_height);

            // Header
            ui.horizontal(|ui| {
                // Colored accent bar
                let (rect, _) = ui.allocate_exact_size(Vec2::new(3.0, 16.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, accent_color);

                ui.label(
                    egui::RichText::new(&descriptor.name)
                        .color(accent_color)
                        .strong(),
                );
            });

            ui.add_space(4.0);

            // Ports section
            ui.horizontal(|ui| {
                // Input ports on the left
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("IN")
                            .small()
                            .color(theme().colors.text_dim),
                    );
                    for port in descriptor
                        .ports
                        .iter()
                        .filter(|p| p.direction == CorePortDirection::Input)
                    {
                        let port_type = convert_port_type(port.port_type);
                        let is_connected = connected_ports.contains(&port.name);

                        ui.horizontal(|ui| {
                            let (response, center) =
                                PortWidget::new(port_type, WidgetPortDirection::Input)
                                    .connected(is_connected)
                                    .show(ui);

                            // Convert local position to absolute
                            let abs_pos = center;

                            port_positions.push(PortPosition {
                                module_id: state.id,
                                port_name: port.name,
                                position: abs_pos,
                                port_type,
                                direction: WidgetPortDirection::Input,
                            });

                            ui.label(
                                egui::RichText::new(&port.label)
                                    .small()
                                    .color(theme().colors.text_secondary),
                            );

                            // Show tooltip on hover
                            if response.hovered() && !port.description.is_empty() {
                                response.on_hover_text(&port.description);
                            }
                        });
                    }
                });

                ui.add_space(16.0);

                // Output ports on the right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("OUT")
                                .small()
                                .color(theme().colors.text_dim),
                        );
                        for port in descriptor
                            .ports
                            .iter()
                            .filter(|p| p.direction == CorePortDirection::Output)
                        {
                            let port_type = convert_port_type(port.port_type);
                            let is_connected = connected_ports.contains(&port.name);

                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&port.label)
                                        .small()
                                        .color(theme().colors.text_secondary),
                                );

                                let (response, center) =
                                    PortWidget::new(port_type, WidgetPortDirection::Output)
                                        .connected(is_connected)
                                        .show(ui);

                                let abs_pos = center;

                                port_positions.push(PortPosition {
                                    module_id: state.id,
                                    port_name: port.name,
                                    position: abs_pos,
                                    port_type,
                                    direction: WidgetPortDirection::Output,
                                });

                                if response.hovered() && !port.description.is_empty() {
                                    response.on_hover_text(&port.description);
                                }
                            });
                        }
                    });
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);

            // Parameters section
            draw_parameters(ui, state, descriptor, accent_color, &mut param_changes);
        });

    ModulePanelResult {
        response: frame_response.response,
        port_positions,
        param_changes,
    }
}

/// Draw parameters for a module.
fn draw_parameters(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    descriptor: &ModuleDescriptor,
    accent_color: Color32,
    param_changes: &mut Vec<Param>,
) {
    // Group parameters by widget hint for layout
    let knob_params: Vec<_> = descriptor
        .parameters
        .iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Knob))
        .collect();

    let slider_params: Vec<_> = descriptor
        .parameters
        .iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Slider))
        .collect();

    let dropdown_params: Vec<_> = descriptor
        .parameters
        .iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Dropdown))
        .collect();

    let toggle_params: Vec<_> = descriptor
        .parameters
        .iter()
        .filter(|p| matches!(p.widget_hint, WidgetHint::Toggle))
        .collect();

    // Draw dropdowns first
    for param in &dropdown_params {
        draw_dropdown_param(ui, state, param, param_changes);
    }

    // Draw toggles
    if !toggle_params.is_empty() {
        ui.horizontal(|ui| {
            for param in &toggle_params {
                draw_toggle_param(ui, state, param, param_changes);
            }
        });
    }

    // Draw knobs in a responsive grid
    if !knob_params.is_empty() {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        ui.horizontal_wrapped(|ui| {
            for param in &knob_params {
                draw_knob_param(ui, state, param, accent_color, param_changes);
            }
        });
    }

    // Draw sliders
    for param in &slider_params {
        draw_slider_param(ui, state, param, accent_color, param_changes);
    }
}

fn draw_knob_param(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    param: &ParameterDescriptor,
    accent_color: Color32,
    param_changes: &mut Vec<Param>,
) {
    ui.vertical(|ui| {
        let current_value = state
            .param_values
            .get(&param.name)
            .copied()
            .unwrap_or(param.range.default);

        let mut value = current_value;
        let size = 48.0;

        Knob::from_descriptor(&mut value, param)
            .size(size)
            .accent_color(accent_color)
            .show(ui);

        if (value - current_value).abs() > f32::EPSILON {
            state.param_values.insert(param.name.clone(), value);
            // Create new param with updated value
            param_changes.push(param.id.with_f32(value));
        }
    });
}

fn draw_slider_param(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    param: &ParameterDescriptor,
    _accent_color: Color32,
    param_changes: &mut Vec<Param>,
) {
    let current_value = state
        .param_values
        .get(&param.name)
        .copied()
        .unwrap_or(param.range.default);

    let mut value = current_value;

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(&param.name)
                .size(theme().fonts.size_normal)
                .color(theme().colors.text_secondary),
        );

        let slider = egui::Slider::new(&mut value, param.range.min..=param.range.max)
            .show_value(true)
            .custom_formatter(|v, _| param.format(v as f32));

        ui.add(slider);
    });

    if (value - current_value).abs() > f32::EPSILON {
        state.param_values.insert(param.name.clone(), value);
        param_changes.push(param.id.with_f32(value));
    }
}

fn draw_dropdown_param(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    param: &ParameterDescriptor,
    param_changes: &mut Vec<Param>,
) {
    if let Some(ref choices) = param.choices {
        let current_value = state
            .param_values
            .get(&param.name)
            .copied()
            .unwrap_or(param.range.default);

        let mut selected = current_value.round() as usize;

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&param.name)
                    .size(theme().fonts.size_normal)
                    .color(theme().colors.text_secondary),
            );

            let selected_text = choices
                .get(selected)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "?".to_string());

            egui::ComboBox::from_id_salt(format!("{:?}", param.id))
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for (i, choice) in choices.iter().enumerate() {
                        if ui.selectable_label(selected == i, &choice.name).clicked() {
                            selected = i;
                        }
                    }
                });
        });

        if selected as f32 != current_value.round() {
            let new_value = selected as f32;
            state.param_values.insert(param.name.clone(), new_value);
            param_changes.push(param.id.with_f32(new_value));
        }
    }
}

fn draw_toggle_param(
    ui: &mut Ui,
    state: &mut ModulePanelState,
    param: &ParameterDescriptor,
    param_changes: &mut Vec<Param>,
) {
    let current_value = state
        .param_values
        .get(&param.name)
        .copied()
        .unwrap_or(param.range.default);

    let mut checked = current_value > 0.5;

    if ui.checkbox(&mut checked, &param.name).changed() {
        let new_value = if checked { 1.0 } else { 0.0 };
        state.param_values.insert(param.name.clone(), new_value);
        param_changes.push(param.id.with_f32(new_value));
    }
}

/// Get accent color for a module category.
pub fn category_color(category: ModuleCategory) -> Color32 {
    match category {
        ModuleCategory::Oscillator => theme().colors.accent_orange,
        ModuleCategory::Filter => theme().colors.accent_cyan,
        ModuleCategory::Envelope => theme().colors.accent_green,
        ModuleCategory::LFO => theme().colors.accent_purple,
        ModuleCategory::Amplifier => theme().colors.accent_yellow,
        ModuleCategory::Effect => theme().colors.accent_cyan,
        ModuleCategory::Utility => theme().colors.text_secondary,
        ModuleCategory::Sampler => theme().colors.accent_orange,
        ModuleCategory::Sequencer => theme().colors.accent_red,
        ModuleCategory::Mixer => theme().colors.accent_yellow,
        ModuleCategory::Output => theme().colors.accent_green,
        ModuleCategory::Visualizer => theme().colors.accent_purple,
        ModuleCategory::PhysicalModeling => theme().colors.accent_red,
    }
}

/// Convert from core PortType to widget PortType.
fn convert_port_type(port_type: CorePortType) -> WidgetPortType {
    match port_type {
        CorePortType::Audio => WidgetPortType::Audio,
        CorePortType::Control => WidgetPortType::Control,
        CorePortType::Gate => WidgetPortType::Gate,
        CorePortType::Midi => WidgetPortType::Midi,
    }
}
