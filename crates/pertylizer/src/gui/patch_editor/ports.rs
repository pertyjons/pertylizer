//! Inline-monitor and port-column subsystem for the patch editor.
//!
//! Holds the `PatchEditor` methods that render the inline signal-monitor module
//! (compact oscilloscope with tiny IN/OUT ports) and the generic vertical port
//! column. Extracted verbatim from `patch_editor.rs` — no behavior change.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Ui, Vec2};
use egui_remixicon::icons as ri;

use synth_core::ModuleDescriptor;
use synth_engine::{EngineHandle, ModuleId};

use crate::gui::module_panel::PortPosition;
use crate::gui::theme::theme;
use crate::gui::widgets::{WidgetPortDirection, WidgetPortType, draw_oscilloscope, expose};

use crate::gui::node_canvas;

use super::{
    CLOSE_BUTTON_HOVER_RED, CLOSE_BUTTON_IDLE, PatchEditor, PatchEditorResult, PatchPort,
    trim_sweep_to_complete_cycles,
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
}
