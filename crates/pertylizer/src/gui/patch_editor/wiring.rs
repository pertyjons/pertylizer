//! Cable/connection "wiring" subsystem for the patch editor.
//!
//! Holds the `PatchEditor` methods that resolve port endpoints, draw connection
//! cables (including the effect-chain cables), and handle port interactions and
//! the port context menu. Drag-to-connect now runs through the shared
//! [`node_canvas::wiring`] FSM (`handle_port_interactions` collects the frame's
//! port events and resolves them); the cable drawing and endpoint resolution
//! stay patch-editor-specific (groups, effect chains, foreground glow layer).

use std::collections::HashMap;

use eframe::egui::{self, LayerId, Order, Pos2, Rect, Ui, Vec2};

use synth_core::{ModuleCategory, PortName};
use synth_engine::ModuleId;
use synth_engine::graph::Connection;

use crate::gui::module_panel::PortPosition;
use crate::gui::theme::theme;
use crate::gui::widgets::{
    CABLE_SPREAD, WidgetPortDirection, WidgetPortType, cable_color, closest_point_on_cable,
    draw_cable, draw_cable_highlighted, draw_flow_particles, point_near_cable,
};

use crate::gui::node_canvas;

use super::{
    EFFECT_CHAIN_AMBER, EffectType, GroupPortKey, PaletteSelection, PatchEditor, PatchEditorResult,
    PatchPort, PortContextMenuState, screen_to_world,
};

impl PatchEditor {
    pub(super) fn is_hidden_internal_connection(&self, connection: &Connection) -> bool {
        let from_group = self.group_of(connection.from_module);
        let to_group = self.group_of(connection.to_module);
        if let Some(gid) = from_group
            && from_group == to_group
            && let Some(group) = self.groups.get(&gid)
        {
            return group.collapsed;
        }
        false
    }

    pub(super) fn resolve_connection_endpoint(
        &self,
        module_id: ModuleId,
        port_name: PortName,
        other_module: ModuleId,
        direction: WidgetPortDirection,
    ) -> Option<PortPosition> {
        if let Some(group_id) = self.group_of(module_id)
            && self.group_of(other_module) != Some(group_id)
            && let Some(group) = self.groups.get(&group_id)
            && group.collapsed
        {
            let key = GroupPortKey {
                group_id,
                module_id,
                port_name,
                direction,
            };
            if let Some(pos) = self.group_port_positions.get(&key) {
                return Some(pos.clone());
            }
        }
        self.port_positions.get(&(module_id, port_name)).cloned()
    }

    /// Draw effect chain cables showing signal flow between effects.
    ///
    /// These are drawn as vertical cables (top-to-bottom) between consecutive
    /// effects in chain order, with arrowheads showing direction.
    pub(super) fn draw_effect_chain_cables(
        &self,
        ui: &Ui,
        bg_layer: LayerId,
        clip_rect: Rect,
        effect_chain_order: &[ModuleId],
    ) {
        if effect_chain_order.len() < 2 {
            return;
        }

        let painter = eframe::egui::Painter::new(ui.ctx().clone(), bg_layer, clip_rect);

        // Warm amber color for chain cables
        let chain_color = EFFECT_CHAIN_AMBER;
        let chain_stroke = egui::Stroke::new(2.5, chain_color.gamma_multiply(0.7));
        let arrow_color = chain_color.gamma_multiply(0.85);

        // Draw cables between consecutive effects
        for pair in effect_chain_order.windows(2) {
            let from_id = pair[0];
            let to_id = pair[1];

            // Get module panel positions
            let (Some(from_panel), Some(to_panel)) =
                (self.panels.get(&from_id), self.panels.get(&to_id))
            else {
                continue;
            };

            // World positions: bottom-center of source, top-center of destination
            let from_screen = Pos2::new(
                from_panel.position.x + from_panel.size.x * 0.5,
                from_panel.position.y + from_panel.size.y,
            );
            let to_screen = Pos2::new(
                to_panel.position.x + to_panel.size.x * 0.5,
                to_panel.position.y,
            );

            // Draw vertical cable segments
            let mid_y = (from_screen.y + to_screen.y) * 0.5;
            if (from_screen.x - to_screen.x).abs() < 2.0 {
                // Straight vertical line
                painter.line_segment([from_screen, to_screen], chain_stroke);
            } else {
                // Orthogonal: down, across, down
                painter.line_segment([from_screen, Pos2::new(from_screen.x, mid_y)], chain_stroke);
                painter.line_segment(
                    [
                        Pos2::new(from_screen.x, mid_y),
                        Pos2::new(to_screen.x, mid_y),
                    ],
                    chain_stroke,
                );
                painter.line_segment([Pos2::new(to_screen.x, mid_y), to_screen], chain_stroke);
            }

            // Draw arrowhead at destination
            let arrow_size = 6.0;
            let arrow_tip = to_screen;
            let arrow_left = Pos2::new(arrow_tip.x - arrow_size, arrow_tip.y - arrow_size * 1.5);
            let arrow_right = Pos2::new(arrow_tip.x + arrow_size, arrow_tip.y - arrow_size * 1.5);
            painter.add(egui::Shape::convex_polygon(
                vec![arrow_tip, arrow_left, arrow_right],
                arrow_color,
                egui::Stroke::NONE,
            ));
        }

        // Draw "IN" label/arrow above the first effect
        if let Some(first_panel) = self.panels.get(&effect_chain_order[0]) {
            let pos = Pos2::new(
                first_panel.position.x + first_panel.size.x * 0.5,
                first_panel.position.y - 4.0,
            );
            let arrow_top = Pos2::new(pos.x, pos.y - 14.0);
            painter.line_segment([arrow_top, pos], chain_stroke);
            // Small arrowhead
            let s = 4.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos,
                    Pos2::new(pos.x - s, pos.y - s * 1.5),
                    Pos2::new(pos.x + s, pos.y - s * 1.5),
                ],
                arrow_color,
                egui::Stroke::NONE,
            ));
            painter.text(
                Pos2::new(pos.x, arrow_top.y - 2.0),
                egui::Align2::CENTER_BOTTOM,
                "IN",
                egui::FontId::proportional(9.0),
                chain_color.gamma_multiply(0.6),
            );
        }

        // Draw "OUT" label/arrow below the last effect
        if let Some(last_panel) = self
            .panels
            .get(effect_chain_order.last().unwrap_or(&effect_chain_order[0]))
        {
            let pos = Pos2::new(
                last_panel.position.x + last_panel.size.x * 0.5,
                last_panel.position.y + last_panel.size.y + 4.0,
            );
            let arrow_bottom = Pos2::new(pos.x, pos.y + 14.0);
            painter.line_segment([pos, arrow_bottom], chain_stroke);
            let s = 4.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    arrow_bottom,
                    Pos2::new(arrow_bottom.x - s, arrow_bottom.y - s * 1.5),
                    Pos2::new(arrow_bottom.x + s, arrow_bottom.y - s * 1.5),
                ],
                arrow_color,
                egui::Stroke::NONE,
            ));
            painter.text(
                Pos2::new(pos.x, arrow_bottom.y + 2.0),
                egui::Align2::CENTER_TOP,
                "OUT",
                egui::FontId::proportional(9.0),
                chain_color.gamma_multiply(0.6),
            );
        }
    }

    /// Draw cables behind modules (on the scroll area's layer). Hovered cables
    /// are drawn in the foreground layer so they appear above modules with glow.
    pub(super) fn draw_connections(
        &mut self,
        ui: &Ui,
        time: f64,
        bg_layer: LayerId,
        clip_rect: Rect,
        module_rects: &[Rect],
    ) {
        let bg_painter = eframe::egui::Painter::new(ui.ctx().clone(), bg_layer, clip_rect);
        // Highlighted cables glow on a sublayer ABOVE the scene content (so the
        // glow sits on top of the modules). It must carry the SAME pan/zoom
        // transform as the scene layer, otherwise the glow renders untransformed
        // at raw world coordinates (offset from the actual cable).
        let fg_layer = LayerId::new(Order::Background, egui::Id::new("cables_active_fg"));
        ui.ctx().set_sublayer(bg_layer, fg_layer);
        if let Some(transform) = ui.ctx().layer_transform_to_global(bg_layer) {
            ui.ctx().set_transform_layer(fg_layer, transform);
        }
        let fg_painter = eframe::egui::Painter::new(ui.ctx().clone(), fg_layer, clip_rect);

        let pointer_pos = ui
            .input(|i| i.pointer.hover_pos())
            .map(|p| screen_to_world(ui, p));

        // Track which cable the context menu targets so we highlight it
        let menu_target = self
            .bg_context_menu
            .as_ref()
            .and_then(|s| s.cable.as_ref())
            .cloned();

        // Compute spread offsets: group cables by destination module,
        // then spread cables within each group so they don't overlap.
        let mut dest_count: HashMap<ModuleId, usize> = HashMap::new();
        for c in &self.connections {
            if self.is_hidden_internal_connection(c) {
                continue;
            }
            *dest_count.entry(c.to_module).or_default() += 1;
        }
        let mut dest_index: HashMap<ModuleId, usize> = HashMap::new();

        // Pre-compute cable spreads and positions for nearest-cable detection
        struct CableInfo {
            index: usize,
            spread: f32,
            from_pos: Pos2,
            to_pos: Pos2,
            port_type: WidgetPortType,
        }
        let mut cable_infos: Vec<CableInfo> = Vec::new();

        for (i, connection) in self.connections.iter().enumerate() {
            if self.is_hidden_internal_connection(connection) {
                continue;
            }
            let idx = dest_index.entry(connection.to_module).or_default();
            let n = dest_count.get(&connection.to_module).copied().unwrap_or(1);
            let spread = (*idx as f32 - (n as f32 - 1.0) / 2.0) * CABLE_SPREAD;
            *dest_index.get_mut(&connection.to_module).unwrap_or(&mut 0) += 1;

            let from_pos = self.resolve_connection_endpoint(
                connection.from_module,
                connection.from_port,
                connection.to_module,
                WidgetPortDirection::Output,
            );
            let to_pos = self.resolve_connection_endpoint(
                connection.to_module,
                connection.to_port,
                connection.from_module,
                WidgetPortDirection::Input,
            );

            if let (Some(from_pos), Some(to_pos)) = (from_pos, to_pos) {
                cable_infos.push(CableInfo {
                    index: i,
                    spread,
                    from_pos: from_pos.position,
                    to_pos: to_pos.position,
                    port_type: from_pos.port_type,
                });
            }
        }

        // Find the single nearest cable to the pointer (exclusive hover)
        let nearest_cable_idx: Option<usize> = pointer_pos.and_then(|p| {
            let over_module = module_rects.iter().any(|r| r.contains(p));

            let mut best_idx: Option<usize> = None;
            let mut best_dist = f32::MAX;

            for info in &cable_infos {
                let near_port = {
                    let to_from = (p - info.from_pos).length();
                    let to_to = (p - info.to_pos).length();
                    to_from < 15.0 || to_to < 15.0
                };

                if !near_port && over_module {
                    continue;
                }

                if point_near_cable(p, info.from_pos, info.to_pos, 20.0, info.spread) {
                    let snap = closest_point_on_cable(p, info.from_pos, info.to_pos, info.spread);
                    let dist = (p - snap).length();
                    if dist < best_dist {
                        best_dist = dist;
                        best_idx = Some(info.index);
                    }
                }
            }

            best_idx
        });

        // Update hovered_cable state for right-click handling
        self.hovered_cable = nearest_cable_idx.map(|i| self.connections[i]);

        // Draw all cables
        for info in &cable_infos {
            let connection = &self.connections[info.index];
            let color = cable_color(info.port_type, 180);

            let is_nearest = nearest_cable_idx == Some(info.index);
            let show_highlight = is_nearest || menu_target.as_ref() == Some(connection);

            if show_highlight {
                // Highlighted cable in foreground (above modules)
                draw_cable_highlighted(&fg_painter, info.from_pos, info.to_pos, color, info.spread);
            } else {
                // Normal cable behind modules
                draw_cable(&bg_painter, info.from_pos, info.to_pos, color, info.spread);
            }

            // Animated flow particles behind modules
            draw_flow_particles(
                &bg_painter,
                info.from_pos,
                info.to_pos,
                color,
                info.port_type,
                time,
                info.spread,
            );
        }
    }

    /// Draw the port right-click context menu for quick-adding connected modules.
    #[allow(clippy::too_many_lines)]
    pub(super) fn draw_port_context_menu(&mut self, ui: &Ui, result: &mut PatchEditorResult) {
        let Some(ref state) = self.port_context_menu else {
            return;
        };

        let menu_id = egui::Id::new("port_context_menu");
        let mut close_menu = false;

        // Clone what we need before mutable borrow
        let target_module = state.module_id;
        let target_port = state.port_name;
        let target_direction = state.direction;
        let port_type = state.port_type;
        let menu_pos = state.menu_pos;

        // Position: new module to the left of input ports, right of output ports
        let offset_x = match target_direction {
            WidgetPortDirection::Input => -220.0,
            WidgetPortDirection::Output => 220.0,
        };
        let new_module_pos = Pos2::new(menu_pos.x + offset_x, menu_pos.y - 50.0);

        let mut open = true;
        egui::Popup::new(
            menu_id,
            ui.ctx().clone(),
            egui::PopupAnchor::Position(menu_pos),
            ui.layer_id(),
        )
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .style(egui::containers::menu::menu_style)
        .gap(0.0)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let header = match target_direction {
                WidgetPortDirection::Input => "Add source",
                WidgetPortDirection::Output => "Add target",
            };
            ui.label(
                egui::RichText::new(header)
                    .color(theme().colors.text_secondary)
                    .size(11.0),
            );
            ui.separator();

            // Group exposure controls (if the module belongs to a group)
            if let Some(group_id) = self.group_of(target_module) {
                let is_exposed =
                    self.is_port_exposed(group_id, target_module, target_port, target_direction);
                if is_exposed {
                    let can_hide = !self.has_external_connection_for_port(
                        group_id,
                        target_module,
                        target_port,
                        target_direction,
                    );
                    let resp = ui.add_enabled(can_hide, egui::Button::new("Hide group port"));
                    if resp.clicked() {
                        let _ =
                            self.hide_port(group_id, target_module, target_port, target_direction);
                        close_menu = true;
                    } else if !can_hide {
                        resp.on_hover_text("Port has external connections");
                    }
                } else if ui.button("Expose as group port").clicked() {
                    let _ =
                        self.expose_port(group_id, target_module, target_port, target_direction);
                    close_menu = true;
                }
                ui.separator();
            }

            // Build menu items based on port type + direction
            match target_direction {
                WidgetPortDirection::Input => {
                    // Input port: show source modules
                    match port_type {
                        WidgetPortType::Audio => {
                            self.port_menu_items(
                                ui,
                                result,
                                &mut close_menu,
                                target_module,
                                target_port,
                                target_direction,
                                new_module_pos,
                                &[
                                    PaletteSelection::Category(ModuleCategory::Oscillator),
                                    PaletteSelection::SubOscillator,
                                    PaletteSelection::WavetableOsc,
                                    PaletteSelection::MathOscillator,
                                    PaletteSelection::AdditiveOsc,
                                    PaletteSelection::GranularOsc,
                                    PaletteSelection::FractalOsc,
                                    PaletteSelection::Sampler,
                                    PaletteSelection::AudioInput,
                                    PaletteSelection::Noise,
                                    PaletteSelection::RingMod,
                                ],
                            );
                        }
                        WidgetPortType::Control => {
                            self.port_menu_items(
                                ui,
                                result,
                                &mut close_menu,
                                target_module,
                                target_port,
                                target_direction,
                                new_module_pos,
                                &[
                                    PaletteSelection::Category(ModuleCategory::LFO),
                                    PaletteSelection::Category(ModuleCategory::Envelope),
                                    PaletteSelection::Mseg,
                                    PaletteSelection::KineticModulator,
                                    PaletteSelection::EnvelopeFollower,
                                ],
                            );
                        }
                        WidgetPortType::Gate => {
                            self.port_menu_items(
                                ui,
                                result,
                                &mut close_menu,
                                target_module,
                                target_port,
                                target_direction,
                                new_module_pos,
                                &[
                                    PaletteSelection::Euclidean,
                                    PaletteSelection::TuringMachine,
                                    PaletteSelection::RandomGates,
                                ],
                            );
                        }
                        // MIDI has no quick-add sources; NoteStream never
                        // appears in the audio patch editor.
                        WidgetPortType::Midi | WidgetPortType::NoteStream => {}
                    }
                }
                WidgetPortDirection::Output => {
                    // Output port: show destination modules
                    match port_type {
                        WidgetPortType::Audio => {
                            self.port_menu_items(
                                ui,
                                result,
                                &mut close_menu,
                                target_module,
                                target_port,
                                target_direction,
                                new_module_pos,
                                &[
                                    PaletteSelection::Category(ModuleCategory::Filter),
                                    PaletteSelection::Category(ModuleCategory::Amplifier),
                                    PaletteSelection::Category(ModuleCategory::Mixer),
                                    PaletteSelection::SignalMonitor,
                                ],
                            );
                            ui.separator();
                            self.port_menu_items(
                                ui,
                                result,
                                &mut close_menu,
                                target_module,
                                target_port,
                                target_direction,
                                new_module_pos,
                                &[
                                    PaletteSelection::Effect(EffectType::Delay),
                                    PaletteSelection::Effect(EffectType::Reverb),
                                    PaletteSelection::Effect(EffectType::Distortion),
                                    PaletteSelection::Effect(EffectType::Chorus),
                                    PaletteSelection::Effect(EffectType::Flanger),
                                    PaletteSelection::Effect(EffectType::Phaser),
                                    PaletteSelection::Effect(EffectType::Compressor),
                                    PaletteSelection::Effect(EffectType::Eq),
                                    PaletteSelection::Effect(EffectType::Waveshaper),
                                ],
                            );
                        }
                        WidgetPortType::Control => {
                            self.port_menu_items(
                                ui,
                                result,
                                &mut close_menu,
                                target_module,
                                target_port,
                                target_direction,
                                new_module_pos,
                                &[
                                    PaletteSelection::Category(ModuleCategory::Amplifier),
                                    PaletteSelection::Category(ModuleCategory::Filter),
                                    PaletteSelection::Category(ModuleCategory::Oscillator),
                                ],
                            );
                        }
                        WidgetPortType::Gate => {
                            self.port_menu_items(
                                ui,
                                result,
                                &mut close_menu,
                                target_module,
                                target_port,
                                target_direction,
                                new_module_pos,
                                &[
                                    PaletteSelection::Category(ModuleCategory::Envelope),
                                    PaletteSelection::Category(ModuleCategory::Amplifier),
                                ],
                            );
                        }
                        WidgetPortType::Midi | WidgetPortType::NoteStream => {}
                    }
                }
            }
        });

        if close_menu || !open {
            self.port_context_menu = None;
        }
    }

    pub(super) fn handle_port_interactions(&mut self, ui: &mut Ui, result: &mut PatchEditorResult) {
        // Right-click still opens the port context menu via a direct hit-test
        // against this frame's anchors (normal ports first, then collapsed-group
        // exposed ports — matching the old two-loop precedence).
        if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary))
            && let Some(pos) = ui
                .input(|i| i.pointer.interact_pos())
                .map(|p| screen_to_world(ui, p))
        {
            for port_pos in self
                .port_positions
                .values()
                .chain(self.group_port_positions.values())
            {
                if Rect::from_center_size(port_pos.position, Vec2::splat(20.0)).contains(pos) {
                    self.port_context_menu = Some(PortContextMenuState {
                        module_id: port_pos.module_id,
                        port_name: port_pos.port_name,
                        port_type: port_pos.port_type,
                        direction: port_pos.direction,
                        menu_pos: pos,
                    });
                    break;
                }
            }
        }

        // Left drag/click wiring goes through the shared node-canvas FSM. Merge
        // the normal and collapsed-group anchors into one map keyed by the
        // self-describing `PatchPort` (a port is visible xor collapsed, so the
        // `(module, port)` keys never collide). The port widgets pushed
        // `wire_events` while the nodes were drawn; a drag releases onto the
        // nearest anchor inside the canvas that forms a valid connection.
        let anchors: HashMap<PatchPort, Pos2> = self
            .port_positions
            .values()
            .chain(self.group_port_positions.values())
            .map(|p| {
                (
                    PatchPort {
                        module: p.module_id,
                        port: p.port_name,
                        direction: p.direction,
                        port_type: p.port_type,
                    },
                    p.position,
                )
            })
            .collect();
        let world_rect = ui.clip_rect();
        let events = std::mem::take(&mut self.wire_events);
        let mut pending = self.pending_wire.take();
        {
            let connections = &self.connections;
            node_canvas::resolve_wire_events(
                ui,
                &mut pending,
                node_canvas::DropTargets {
                    anchors: &anchors,
                    world_rect,
                    drop_radius: 16.0,
                },
                events,
                |from, to| Self::open_patch_connection(connections, from, to),
                |connection| result.connections_to_add.push(connection),
            );
        }
        self.pending_wire = pending;
    }

    /// Recompute the set of modules the in-progress drag must not connect to
    /// because the edge would close a cycle. One graph traversal per frame
    /// (reachability from the drag's source) replaces the previous per-port,
    /// per-frame `would_create_cycle` DFS. Empty when no drag is active.
    ///
    /// Dragging from an *output* of `S` would add `S → m`, so any `m` that can
    /// already reach `S` (an ancestor of `S`) is blocked — walk edges backward.
    /// Dragging from an *input* of `S` would add `m → S`, so any `m` reachable
    /// from `S` (a descendant) is blocked — walk edges forward. `S` itself is
    /// always blocked (self-loop).
    pub(super) fn recompute_drag_cycle_blocked(&mut self) {
        self.drag_cycle_blocked.clear();
        // Pull the scalars out of the borrow before mutating `drag_cycle_blocked`.
        let Some((source, walk_forward)) = self.pending_wire.as_ref().map(|w| {
            (
                w.from.module,
                w.from.direction == WidgetPortDirection::Input,
            )
        }) else {
            return;
        };

        self.drag_cycle_blocked.insert(source);
        let mut stack = vec![source];
        while let Some(current) = stack.pop() {
            for conn in &self.connections {
                let neighbor = if walk_forward {
                    (conn.from_module == current).then_some(conn.to_module)
                } else {
                    (conn.to_module == current).then_some(conn.from_module)
                };
                if let Some(next) = neighbor
                    && self.drag_cycle_blocked.insert(next)
                {
                    stack.push(next);
                }
            }
        }
    }
}
