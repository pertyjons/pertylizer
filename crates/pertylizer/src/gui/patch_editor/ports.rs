//! Inline-monitor and port-column subsystem for the patch editor.
//!
//! Holds the `PatchEditor` methods that render the inline signal-monitor module
//! (compact oscilloscope with tiny IN/OUT ports) and the generic vertical port
//! column. Extracted verbatim from `patch_editor.rs` — no behavior change.

use std::collections::HashSet;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Ui, Vec2};
use egui_remixicon::icons as ri;

use synth_core::ModuleDescriptor;
use synth_engine::{EngineHandle, ModuleId};

use crate::gui::module_panel::PortPosition;
use crate::gui::theme::theme;
use crate::gui::widgets::{
    PortWidget, WidgetPortDirection, WidgetPortType, draw_oscilloscope, expose,
};

use crate::gui::node_canvas;

use super::{
    CLOSE_BUTTON_HOVER_RED, CLOSE_BUTTON_IDLE, PatchEditor, PatchEditorResult, PatchPort,
    PortRenderInfo, trim_sweep_to_complete_cycles,
};

impl PatchEditor {
    /// Handle clicks on empty canvas: left-click deselects, right-click opens
    /// the background/cable context menu, and Escape cancels a pending
    /// connection and closes any open context menus.
    /// Render an inline signal-monitor module: a compact 100×50 oscilloscope
    /// with tiny IN/OUT ports and a close button. Registers port positions for
    /// cables and starts pending connections on port drag.
    pub(super) fn draw_inline_monitor(
        &mut self,
        ui: &mut Ui,
        module_id: ModuleId,
        descriptor: &ModuleDescriptor,
        dimmed_accent: Color32,
        handle: &EngineHandle,
        result: &mut PatchEditorResult,
    ) {
        let inline_frame = egui::Frame::new()
            .fill(theme().colors.bg_dark)
            .stroke(egui::Stroke::new(1.0, dimmed_accent.gamma_multiply(0.5)))
            .corner_radius(4.0);
        inline_frame.show(ui, |ui| {
            let panel_width = 100.0;
            let panel_height = 50.0;
            ui.set_min_size(Vec2::new(panel_width, panel_height));
            ui.set_max_size(Vec2::new(panel_width, panel_height));

            // Draw ports at left/right edges
            let port_col_w = 8.0;
            ui.horizontal(|ui| {
                // Left port (IN) — tiny dot
                ui.vertical(|ui| {
                    ui.set_width(port_col_w);
                    ui.set_height(panel_height);
                    let center = ui.available_rect_before_wrap().center();
                    for port in &descriptor.ports {
                        if port.direction == synth_core::PortDirection::Input {
                            let port_rect = egui::Rect::from_center_size(center, Vec2::splat(8.0));
                            let port_resp = ui.allocate_rect(port_rect, Sense::click_and_drag());
                            expose(
                                &port_resp,
                                egui::WidgetType::Other,
                                format!("{} in", port.name),
                                None,
                            );
                            let port_color = if port_resp.hovered() {
                                theme().colors.accent_cyan
                            } else {
                                theme().colors.accent_cyan.gamma_multiply(0.6)
                            };
                            ui.painter().circle_filled(center, 3.0, port_color);

                            // Register port position for cables
                            let screen_pos = center;
                            let in_port_type = if port.port_type == synth_core::PortType::Audio {
                                WidgetPortType::Audio
                            } else {
                                WidgetPortType::Control
                            };
                            self.port_positions.insert(
                                (module_id, port.name),
                                PortPosition {
                                    module_id,
                                    port_name: port.name,
                                    position: screen_pos,
                                    direction: WidgetPortDirection::Input,
                                    port_type: in_port_type,
                                },
                            );

                            // Feed the shared wire FSM (drag-to-connect / click-click).
                            let pp = PatchPort {
                                module: module_id,
                                port: port.name,
                                direction: WidgetPortDirection::Input,
                                port_type: in_port_type,
                            };
                            node_canvas::push_port_event(
                                &mut self.wire_events,
                                &port_resp,
                                pp,
                                screen_pos,
                            );
                        }
                    }
                });

                // Oscilloscope content area
                let scope_width = panel_width - port_col_w * 2.0;
                let scope_height = panel_height;
                ui.vertical(|ui| {
                    ui.set_width(scope_width);
                    let vis_buffer = handle.get_visualization_buffer(module_id);
                    let samples = if let Some(buffer) = vis_buffer {
                        buffer.read_sweep().unwrap_or_default()
                    } else {
                        (0..128)
                            .map(|i| {
                                let t = i as f32 / 128.0;
                                (t * std::f32::consts::TAU * 3.0).sin() * 0.5
                            })
                            .collect()
                    };
                    let samples = trim_sweep_to_complete_cycles(&samples, 0.0);

                    draw_oscilloscope(
                        ui,
                        samples,
                        scope_width,
                        scope_height,
                        1.0,
                        theme().colors.accent_cyan,
                    );
                });

                // Right port (OUT) — tiny dot
                ui.vertical(|ui| {
                    ui.set_width(port_col_w);
                    ui.set_height(panel_height);
                    let center = ui.available_rect_before_wrap().center();
                    for port in &descriptor.ports {
                        if port.direction == synth_core::PortDirection::Output {
                            let port_rect = egui::Rect::from_center_size(center, Vec2::splat(8.0));
                            let port_resp = ui.allocate_rect(port_rect, Sense::click_and_drag());
                            expose(
                                &port_resp,
                                egui::WidgetType::Other,
                                format!("{} out", port.name),
                                None,
                            );
                            let port_color = if port_resp.hovered() {
                                theme().colors.accent_green
                            } else {
                                theme().colors.accent_green.gamma_multiply(0.6)
                            };
                            ui.painter().circle_filled(center, 3.0, port_color);

                            let screen_pos = center;
                            let out_port_type = if port.port_type == synth_core::PortType::Audio {
                                WidgetPortType::Audio
                            } else {
                                WidgetPortType::Control
                            };
                            self.port_positions.insert(
                                (module_id, port.name),
                                PortPosition {
                                    module_id,
                                    port_name: port.name,
                                    position: screen_pos,
                                    direction: WidgetPortDirection::Output,
                                    port_type: out_port_type,
                                },
                            );

                            let pp = PatchPort {
                                module: module_id,
                                port: port.name,
                                direction: WidgetPortDirection::Output,
                                port_type: out_port_type,
                            };
                            node_canvas::push_port_event(
                                &mut self.wire_events,
                                &port_resp,
                                pp,
                                screen_pos,
                            );
                        }
                    }
                });
            });

            // Close button overlay (×) in top-right corner
            let panel_rect = ui.min_rect();
            let close_size = Vec2::new(14.0, 14.0);
            let close_pos = Pos2::new(
                panel_rect.right() - close_size.x - 1.0,
                panel_rect.top() + 1.0,
            );
            let close_rect = Rect::from_min_size(close_pos, close_size);
            let close_resp = ui.allocate_rect(close_rect, Sense::click());
            expose(
                &close_resp,
                egui::WidgetType::Button,
                format!("close {}", descriptor.name),
                None,
            );
            let close_color = if close_resp.hovered() {
                CLOSE_BUTTON_HOVER_RED
            } else {
                CLOSE_BUTTON_IDLE
            };
            ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                ri::CLOSE_LINE,
                egui::FontId::proportional(10.0),
                close_color,
            );

            if close_resp.clicked() {
                self.bypass_and_remove(module_id, result);
            }
        });

        // Request continuous repaint so the waveform updates
        ui.request_repaint();
    }

    pub(super) fn draw_port_column_with<F>(
        ui: &mut Ui,
        direction: WidgetPortDirection,
        ports: &[PortRenderInfo],
        pending_info: Option<(ModuleId, WidgetPortType, WidgetPortDirection)>,
        cycle_blocked: &HashSet<ModuleId>,
        mut store_position: F,
    ) where
        F: FnMut(&PortRenderInfo, Pos2, &egui::Response),
    {
        let t = theme();
        let col_width = t.sizes.port_column_width;
        let spacing = t.sizes.port_vertical_spacing;

        ui.vertical(|ui| {
            let label = match direction {
                WidgetPortDirection::Input => "IN",
                WidgetPortDirection::Output => "OUT",
            };
            if !ports.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(label)
                            .size(8.0)
                            .color(t.colors.text_dim),
                    );
                });

                let rail_x = ui.cursor().min.x + col_width * 0.5;
                let rail_top = ui.cursor().min.y + 3.0;
                let rail_bottom = rail_top + ports.len() as f32 * spacing - 6.0;
                ui.painter().line_segment(
                    [Pos2::new(rail_x, rail_top), Pos2::new(rail_x, rail_bottom)],
                    egui::Stroke::new(1.0, t.colors.border.gamma_multiply(0.55)),
                );
            }

            for port in ports {
                let is_highlighted = pending_info
                    .map(|(from_module, from_type, from_dir)| {
                        // Signal always flows output → input; pick whichever side
                        // is the output so directional compatibility is correct.
                        let (out_type, in_type) = if from_dir == WidgetPortDirection::Output {
                            (from_type, port.port_type)
                        } else {
                            (port.port_type, from_type)
                        };
                        from_module != port.module_id
                            && from_dir != direction
                            && out_type.can_drive(in_type)
                            && !cycle_blocked.contains(&port.module_id)
                    })
                    .unwrap_or(false);

                ui.vertical_centered(|ui| {
                    ui.allocate_ui(Vec2::new(col_width, spacing), |ui| {
                        ui.centered_and_justified(|ui| {
                            let (response, center) = PortWidget::new(port.port_type)
                                .connected(port.is_connected)
                                .highlighted(is_highlighted)
                                .markers(port.markers)
                                .show(ui);

                            store_position(port, center, &response);

                            // Expose to AccessKit / the egui-inspection MCP so a
                            // driver can locate a port by name+type+direction (the
                            // node name isn't threaded into this column, so the
                            // label is port-scoped).
                            let type_str = match port.port_type {
                                WidgetPortType::Audio => "audio",
                                WidgetPortType::Control => "cv",
                                WidgetPortType::Gate => "gate",
                                WidgetPortType::Midi => "midi",
                                WidgetPortType::NoteStream => "notes",
                            };
                            let dir_str = match direction {
                                WidgetPortDirection::Input => "in",
                                WidgetPortDirection::Output => "out",
                            };
                            expose(
                                &response,
                                egui::WidgetType::Other,
                                format!("{} {type_str} {dir_str}", port.label),
                                None,
                            );

                            // Single egui tooltip (positioned correctly inside the
                            // Scene transform): port name + signal type, plus the
                            // longer description when present. Built lazily inside
                            // `on_hover_ui` so we don't format a throwaway String
                            // for every port on every frame — only when hovered.
                            response.on_hover_ui(|ui| {
                                let mut tip = format!("{} ({type_str})", port.label);
                                if !port.description.is_empty() {
                                    tip.push('\n');
                                    tip.push_str(&port.description);
                                }
                                ui.label(tip);
                            });
                        });
                    });
                });
            }
        });
    }
}
