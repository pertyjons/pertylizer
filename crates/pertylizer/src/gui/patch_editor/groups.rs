//! Module-group subsystem for the patch editor.
//!
//! Holds the free helpers and `PatchEditor` methods that create, lay out,
//! draw, and interact with module groups (collapsed boxes, group frames,
//! exposed-port columns, and the group context menu). Extracted verbatim from
//! `patch_editor.rs` — no behavior change.

use std::collections::{HashMap, HashSet};

use eframe::egui::{self, Color32, Id, LayerId, Pos2, Rect, Sense, Ui, Vec2};
use egui_remixicon::icons as ri;

use synth_engine::ModuleId;

use crate::gui::module_panel::PortPosition;
use crate::gui::theme::theme;
use crate::gui::widgets::{WidgetPortDirection, draw_module_header, icon_button, labeled_row};
use crate::patch::{GroupId, HexColor};

use super::{
    GROUP_HEADER_HEIGHT, GROUP_PADDING, GROUP_PORT_MARGIN, GroupContextMenuState, GroupLayout,
    GroupPortKey, GroupTemplateAction, ModuleGroup, PatchEditor, PatchEditorResult, PortRenderInfo,
    color32_to_hex, parse_hex_color, screen_to_world, snap_to_grid,
};

pub(super) fn collapsed_group_size(group: &ModuleGroup) -> Vec2 {
    let port_rows = group
        .exposed_inputs
        .len()
        .max(group.exposed_outputs.len())
        .max(1);
    let t = theme();
    let height = GROUP_HEADER_HEIGHT
        + GROUP_PORT_MARGIN * 2.0
        + port_rows as f32 * t.sizes.port_vertical_spacing;
    let height = height.max(t.sizes.module_min_height);
    // Collapsed groups only show "N modules" text — use a smaller content width
    let collapsed_content_width = 40.0;
    let ports_width = t.sizes.port_column_width * 2.0 + collapsed_content_width;
    let title_chars = group.name.chars().count() as f32;
    let title_width = 14.0 + title_chars * 7.0; // accent + approximate glyph width
    let header_actions = 2.0 * 20.0 + 8.0; // expand + delete buttons + spacing
    let header_width = title_width + header_actions;
    let width = ports_width.max(header_width);
    Vec2::new(width, height)
}

pub(super) fn group_toggle_icon_rect(rect: Rect) -> Rect {
    let size = Vec2::splat(16.0);
    Rect::from_min_size(Pos2::new(rect.max.x - size.x - 6.0, rect.min.y + 4.0), size)
}

pub(super) fn group_menu_icon_rect(rect: Rect) -> Rect {
    let size = Vec2::splat(16.0);
    let toggle = group_toggle_icon_rect(rect);
    Rect::from_min_size(
        Pos2::new(toggle.min.x - size.x - 4.0, rect.min.y + 4.0),
        size,
    )
}

impl PatchEditor {
    pub(super) fn remove_from_group(&mut self, module_id: ModuleId) {
        if let Some(group_id) = self.module_to_group.remove(&module_id)
            && let Some(group) = self.groups.get_mut(&group_id)
        {
            group.members.retain(|mid| *mid != module_id);
            group.exposed_inputs.retain(|p| p.module_id != module_id);
            group.exposed_outputs.retain(|p| p.module_id != module_id);
            if group.members.is_empty() {
                self.groups.remove(&group_id);
                if self.selected_group == Some(group_id) {
                    self.selected_group = None;
                }
                if self
                    .group_context_menu
                    .as_ref()
                    .is_some_and(|m| m.group_id == group_id)
                {
                    self.group_context_menu = None;
                }
            }
        }
        self.refresh_exposed_for_module(module_id);
    }

    pub(super) fn create_group_from_selection(&mut self) -> Option<GroupId> {
        if self.selected_modules.is_empty() {
            return None;
        }
        let id = self.allocate_group_id();
        let mut members: Vec<ModuleId> = self.selected_modules.iter().copied().collect();
        members.sort_by_key(|m| m.to_string());

        for mid in &members {
            self.remove_from_group(*mid);
        }
        for mid in &members {
            self.module_to_group.insert(*mid, id);
        }

        let position = self.compute_group_default_position(&members);
        let members_for_refresh = members.clone();

        let group = ModuleGroup {
            id,
            name: format!("Group {}", id.0),
            color: None,
            members,
            collapsed: false,
            position,
            exposed_inputs: Vec::new(),
            exposed_outputs: Vec::new(),
        };
        self.groups.insert(id, group);
        for mid in &members_for_refresh {
            self.refresh_exposed_for_module(*mid);
        }
        self.selected_group = Some(id);
        Some(id)
    }

    pub(super) fn group_bounds_world(&self, group: &ModuleGroup) -> Option<Rect> {
        let mut rect: Option<Rect> = None;
        for mid in &group.members {
            if let Some(panel) = self.panels.get(mid) {
                let r = Rect::from_min_size(panel.position, panel.size);
                rect = Some(rect.map_or(r, |acc| acc.union(r)));
            }
        }
        let mut rect = rect?;
        rect.min.x -= GROUP_PADDING;
        rect.max.x += GROUP_PADDING;
        rect.min.y -= GROUP_PADDING + GROUP_HEADER_HEIGHT;
        rect.max.y += GROUP_PADDING;
        Some(rect)
    }

    pub(super) fn compute_group_layout(&self) -> GroupLayout {
        let mut rects_world = HashMap::new();
        let mut hidden_modules: HashSet<ModuleId> = HashSet::new();

        for group in self.groups.values() {
            let world_rect = if group.collapsed {
                Rect::from_min_size(group.position, collapsed_group_size(group))
            } else {
                match self.group_bounds_world(group) {
                    Some(r) => r,
                    None => continue,
                }
            };
            rects_world.insert(group.id, world_rect);

            if group.collapsed {
                hidden_modules.extend(group.members.iter().copied());
            }
        }

        GroupLayout {
            rects_world,
            hidden_modules,
        }
    }

    /// Move a collapsed group's box to `new_position` and shift every member
    /// panel by the same delta so the group's internal relative layout is
    /// preserved when the user expands it again.
    pub(super) fn move_collapsed_group(&mut self, group_id: GroupId, new_position: Pos2) {
        let Some(group) = self.groups.get_mut(&group_id) else {
            return;
        };
        let delta = new_position - group.position;
        if delta == Vec2::ZERO {
            return;
        }
        group.position = new_position;
        let members = group.members.clone();
        for mid in members {
            if let Some(panel) = self.panels.get_mut(&mid) {
                panel.position += delta;
                self.needs_reposition.insert(mid);
            }
        }
    }

    pub(super) fn delete_group(&mut self, group_id: GroupId, result: &mut PatchEditorResult) {
        let Some(group) = self.groups.get(&group_id).cloned() else {
            return;
        };
        for mid in group.members {
            result.modules_to_remove.push(mid);
            self.remove_module(mid);
        }
        self.groups.remove(&group_id);
        if self.selected_group == Some(group_id) {
            self.selected_group = None;
        }
        if self
            .group_context_menu
            .as_ref()
            .is_some_and(|m| m.group_id == group_id)
        {
            self.group_context_menu = None;
        }
    }

    pub(super) fn ungroup(&mut self, group_id: GroupId) {
        let Some(group) = self.groups.remove(&group_id) else {
            return;
        };
        for mid in group.members {
            self.module_to_group.remove(&mid);
        }
        if self.selected_group == Some(group_id) {
            self.selected_group = None;
        }
        if self
            .group_context_menu
            .as_ref()
            .is_some_and(|m| m.group_id == group_id)
        {
            self.group_context_menu = None;
        }
    }

    pub(super) fn group_color(&self, group: &ModuleGroup) -> Color32 {
        group
            .color
            .as_ref()
            .and_then(|c| parse_hex_color(c))
            .unwrap_or_else(|| theme().colors.accent_cyan)
    }

    pub(super) fn draw_group_port_column(
        &mut self,
        ui: &mut Ui,
        group: &ModuleGroup,
        direction: WidgetPortDirection,
        new_positions: &mut HashMap<GroupPortKey, PortPosition>,
    ) {
        let ports = match direction {
            WidgetPortDirection::Input => &group.exposed_inputs,
            WidgetPortDirection::Output => &group.exposed_outputs,
        };

        let ports: Vec<PortRenderInfo> = ports
            .iter()
            .map(|p| PortRenderInfo {
                module_id: p.module_id,
                port_name: p.port_name,
                label: p.label.clone(),
                description: String::new(),
                port_type: self.port_widget_type(p.module_id, p.port_name),
                is_connected: self.has_external_connection_for_port(
                    group.id,
                    p.module_id,
                    p.port_name,
                    direction,
                ),
                // Collapsed group ports don't carry modulation markers yet — the
                // per-module ports (see node.rs) do. Follow-up if groups need them.
                markers: crate::gui::widgets::ModMarkers::default(),
            })
            .collect();

        let pending_info = self
            .pending_connection()
            .map(|p| (p.from_module, p.from_type, p.from_direction));
        // Group columns expose ports from several member modules; the shared
        // per-frame set already covers them all.
        let cycle_blocked = &self.drag_cycle_blocked;
        Self::draw_port_column_with(
            ui,
            direction,
            &ports,
            pending_info,
            cycle_blocked,
            |port, center| {
                new_positions.insert(
                    GroupPortKey {
                        group_id: group.id,
                        module_id: port.module_id,
                        port_name: port.port_name,
                        direction,
                    },
                    PortPosition {
                        module_id: port.module_id,
                        port_name: port.port_name,
                        position: center,
                        port_type: port.port_type,
                        direction,
                    },
                );
            },
        );
    }

    pub(super) fn draw_group_frames(
        &self,
        ui: &Ui,
        layout: &GroupLayout,
        layer_id: LayerId,
        clip_rect: Rect,
    ) {
        let painter = eframe::egui::Painter::new(ui.ctx().clone(), layer_id, clip_rect);
        for group in self.groups.values() {
            if group.collapsed {
                continue;
            }
            let Some(rect) = layout.rects_world.get(&group.id) else {
                continue;
            };
            let base_color = self.group_color(group);
            let stroke_width = if self.selected_group == Some(group.id) {
                2.0
            } else {
                1.0
            };
            let stroke = egui::Stroke::new(stroke_width, base_color.gamma_multiply(0.6));
            painter.rect(
                *rect,
                6.0,
                Color32::TRANSPARENT,
                stroke,
                egui::StrokeKind::Inside,
            );

            // Header strip
            let header_rect =
                Rect::from_min_size(rect.min, Vec2::new(rect.width(), GROUP_HEADER_HEIGHT));
            painter.rect_filled(header_rect, 6.0, base_color.gamma_multiply(0.15));
            painter.text(
                header_rect.min + Vec2::new(8.0, 4.0),
                egui::Align2::LEFT_TOP,
                &group.name,
                egui::FontId::proportional(12.0),
                base_color.gamma_multiply(0.9),
            );

            // Menu icon (⋯)
            let menu_rect = group_menu_icon_rect(*rect);
            painter.rect_stroke(
                menu_rect,
                3.0,
                egui::Stroke::new(1.0, base_color.gamma_multiply(0.5)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                menu_rect.center(),
                egui::Align2::CENTER_CENTER,
                ri::MORE_LINE,
                egui::FontId::proportional(12.0),
                base_color.gamma_multiply(0.9),
            );

            // Collapse icon
            let icon_rect = group_toggle_icon_rect(*rect);
            painter.rect_stroke(
                icon_rect,
                3.0,
                egui::Stroke::new(1.0, base_color.gamma_multiply(0.5)),
                egui::StrokeKind::Inside,
            );
            painter.text(
                icon_rect.center(),
                egui::Align2::CENTER_CENTER,
                ri::SUBTRACT_LINE,
                egui::FontId::proportional(12.0),
                base_color.gamma_multiply(0.9),
            );
        }
    }

    pub(super) fn draw_collapsed_groups(
        &mut self,
        ui: &mut Ui,
        instrument_id: u64,
        result: &mut PatchEditorResult,
        new_positions: &mut HashMap<GroupPortKey, PortPosition>,
    ) {
        let group_ids: Vec<GroupId> = self.groups.keys().copied().collect();
        for group_id in group_ids {
            let Some(group) = self.groups.get(&group_id).cloned() else {
                continue;
            };
            if !group.collapsed {
                continue;
            }

            // Drag/select hit-rect: the box rendered last frame (measured below),
            // falling back to the static estimate on the first frame. Reading the
            // real size back keeps the whole visible box grabbable even when the
            // estimate is off — the same one-frame-lag trick the module cards use.
            let est_size = collapsed_group_size(&group);
            let box_size = self
                .collapsed_group_sizes
                .get(&group_id)
                .copied()
                .unwrap_or(est_size);
            let world_rect = Rect::from_min_size(group.position, box_size);

            let area_id = Id::new((instrument_id, "group_box", group_id.0));

            // Own the drag in WORLD space — no `egui::Area` (an Area inside a
            // Scene is root-level / screen-space and ignores the layer transform,
            // so the box would not pan/zoom with the canvas). The background
            // interact is registered BEFORE the body so the header buttons (drawn
            // on top) keep their own clicks. `drag_delta()` is already world-space
            // inside a Scene.
            let box_response =
                ui.interact(world_rect, area_id.with("box"), Sense::click_and_drag());
            if box_response.dragged() {
                let new_pos = group.position + box_response.drag_delta();
                self.move_collapsed_group(group_id, new_pos);
            }
            if box_response.drag_stopped() {
                self.move_collapsed_group(group_id, snap_to_grid(group.position));
            }
            // Live position after any drag this frame, for placing the body.
            let world_min = self
                .groups
                .get(&group_id)
                .map_or(world_rect.min, |g| g.position);

            let mut toggle_clicked = false;
            let mut delete_clicked = false;
            let mut menu_clicked = false;
            let mut menu_pos = Pos2::ZERO;
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id(area_id)
                    // Width from the estimate (drives column wrapping); generous
                    // height so the content is never clipped — the frame sizes to
                    // its content and the real rect is measured back below.
                    .max_rect(Rect::from_min_size(world_min, Vec2::new(est_size.x, 600.0)))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            {
                let ui = &mut child;
                let base_color = self.group_color(&group);
                let stroke_width = if self.selected_group == Some(group_id) {
                    2.0
                } else {
                    1.0
                };
                let frame = egui::Frame::window(&ui.global_style())
                    .fill(ui.global_style().visuals.window_fill())
                    .stroke(egui::Stroke::new(
                        stroke_width,
                        base_color.gamma_multiply(0.6),
                    ))
                    .corner_radius(6.0);
                frame.show(ui, |ui| {
                    let t = theme();
                    draw_module_header(
                        ui,
                        base_color.gamma_multiply(0.9),
                        &group.name,
                        Some(format!("Group ID: {}", group_id.0)),
                        false,
                        |ui| {
                            // Emitted right-to-left (see `draw_module_header`), so
                            // reverse reading order: close · add · menu → renders
                            // menu · add · close.
                            if icon_button(ui, ri::CLOSE_LINE, t.colors.text_dim, "Delete group")
                                .clicked()
                            {
                                delete_clicked = true;
                            }

                            if icon_button(ui, ri::ADD_LINE, t.colors.text_dim, "Expand group")
                                .clicked()
                            {
                                toggle_clicked = true;
                            }

                            let menu_resp =
                                icon_button(ui, ri::MORE_LINE, t.colors.text_dim, "Group menu");
                            if menu_resp.clicked() {
                                menu_clicked = true;
                                menu_pos = menu_resp.rect.left_bottom();
                            }
                        },
                    );

                    ui.horizontal(|ui| {
                        // Left port column (IN)
                        ui.vertical(|ui| {
                            ui.set_width(t.sizes.port_column_width);
                            self.draw_group_port_column(
                                ui,
                                &group,
                                WidgetPortDirection::Input,
                                new_positions,
                            );
                        });

                        // Content column
                        ui.vertical(|ui| {
                            ui.set_min_width(40.0);
                            ui.label(
                                egui::RichText::new(format!("{} modules", group.members.len()))
                                    .size(10.0)
                                    .color(t.colors.text_dim),
                            );
                            if group.exposed_inputs.is_empty() && group.exposed_outputs.is_empty() {
                                ui.label(
                                    egui::RichText::new("No exposed ports")
                                        .size(9.0)
                                        .color(t.colors.text_dim.gamma_multiply(0.7)),
                                );
                            }
                        });

                        // Right port column (OUT)
                        ui.vertical(|ui| {
                            ui.set_width(t.sizes.port_column_width);
                            self.draw_group_port_column(
                                ui,
                                &group,
                                WidgetPortDirection::Output,
                                new_positions,
                            );
                        });
                    });
                });
            }

            // Store the rendered box size for next frame's interact rect.
            self.collapsed_group_sizes
                .insert(group_id, child.min_rect().size());

            if delete_clicked {
                self.delete_group(group_id, result);
                continue;
            }

            if toggle_clicked && let Some(group_mut) = self.groups.get_mut(&group_id) {
                group_mut.collapsed = false;
                continue;
            }

            if menu_clicked {
                self.group_context_menu = Some(GroupContextMenuState { group_id, menu_pos });
            }

            if box_response.clicked() {
                self.selected_group = Some(group_id);
                self.selected_modules.clear();
                self.selected_module = None;
            }
            if box_response.double_clicked()
                && let Some(group_mut) = self.groups.get_mut(&group_id)
            {
                group_mut.collapsed = false;
            }
        }
    }

    pub(super) fn handle_group_interactions(
        &mut self,
        ui: &Ui,
        layout: &GroupLayout,
        module_rects: &[Rect],
    ) {
        let pointer_pos = ui
            .input(|i| i.pointer.interact_pos())
            .map(|p| screen_to_world(ui, p));
        let Some(pos) = pointer_pos else {
            return;
        };
        let over_module = module_rects.iter().any(|r| r.contains(pos));
        if over_module {
            return;
        }

        let mut target_group: Option<GroupId> = None;
        for (gid, rect) in &layout.rects_world {
            if rect.contains(pos)
                && let Some(group) = self.groups.get(gid)
                && !group.collapsed
            {
                target_group = Some(*gid);
                break;
            }
        }

        let Some(group_id) = target_group else {
            return;
        };

        if let Some(rect) = layout.rects_world.get(&group_id) {
            let primary_clicked =
                ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));

            // Menu icon (⋯) click
            let menu_rect = group_menu_icon_rect(*rect);
            if menu_rect.contains(pos) && primary_clicked {
                self.group_context_menu = Some(GroupContextMenuState {
                    group_id,
                    menu_pos: Pos2::new(menu_rect.left(), menu_rect.bottom()),
                });
                return;
            }

            // Collapse icon click
            let icon_rect = group_toggle_icon_rect(*rect);
            if icon_rect.contains(pos) && primary_clicked {
                if let Some(group) = self.groups.get_mut(&group_id) {
                    group.collapsed = true;
                    if let Some(rect_world) = layout.rects_world.get(&group_id) {
                        group.position = rect_world.min;
                    }
                }
                return;
            }
        }

        if ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary)) {
            self.selected_group = Some(group_id);
            self.selected_modules.clear();
            self.selected_module = None;
        }
        if ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        }) && let Some(group) = self.groups.get_mut(&group_id)
        {
            group.collapsed = true;
            if let Some(rect) = layout.rects_world.get(&group_id) {
                group.position = rect.min;
            }
        }
    }

    pub(super) fn draw_group_context_menu(&mut self, ui: &Ui, result: &mut PatchEditorResult) {
        let Some(state) = self.group_context_menu.clone() else {
            return;
        };
        let Some(group) = self.groups.get(&state.group_id).cloned() else {
            self.group_context_menu = None;
            return;
        };

        let menu_id = egui::Id::new("group_context_menu");
        let mut open = true;
        let mut close_menu = false;
        let mut commit_name: Option<String> = None;
        let mut color_update: Option<Option<HexColor>> = None;
        let mut do_save_template = false;
        let mut do_ungroup = false;
        let mut do_delete = false;

        let mut name_buf = self
            .group_name_edit
            .take()
            .and_then(|(gid, name)| {
                if gid == state.group_id {
                    Some(name)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| group.name.clone());

        egui::Popup::new(
            menu_id,
            ui.ctx().clone(),
            egui::PopupAnchor::Position(state.menu_pos),
            ui.layer_id(),
        )
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .style(egui::containers::menu::menu_style)
        .gap(0.0)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.label(
                egui::RichText::new(&group.name)
                    .color(theme().colors.text_secondary)
                    .size(11.0),
            );
            ui.separator();

            labeled_row(ui, "Name", |ui| {
                let resp = ui.text_edit_singleline(&mut name_buf);
                if (resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)))
                    && !name_buf.trim().is_empty()
                {
                    commit_name = Some(name_buf.trim().to_string());
                }
            });

            labeled_row(ui, "Color", |ui| {
                let mut color = self.group_color(&group);
                let changed = egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut color,
                    egui::color_picker::Alpha::BlendOrAdditive,
                )
                .changed();
                if changed {
                    color_update = Some(Some(color32_to_hex(color)));
                }
                if ui.button("Clear").clicked() {
                    color_update = Some(None);
                }
            });

            ui.separator();

            if ui.button("Save group as template").clicked() {
                do_save_template = true;
                close_menu = true;
            }

            if ui.button("Ungroup").clicked() {
                do_ungroup = true;
                close_menu = true;
            }

            if ui.button("Delete group").clicked() {
                do_delete = true;
                close_menu = true;
            }
        });

        if let Some(new_name) = commit_name
            && let Some(g) = self.groups.get_mut(&state.group_id)
        {
            g.name = new_name;
        }

        if let Some(color_opt) = color_update
            && let Some(g) = self.groups.get_mut(&state.group_id)
        {
            g.color = color_opt;
        }

        if do_save_template {
            result.group_template_action = Some(GroupTemplateAction::SaveGroup {
                group_id: state.group_id,
            });
        }

        if do_ungroup {
            self.ungroup(state.group_id);
        }

        if do_delete {
            self.delete_group(state.group_id, result);
        }

        if !close_menu && open {
            self.group_name_edit = Some((state.group_id, name_buf));
        } else {
            self.group_name_edit = None;
        }

        if close_menu || !open {
            self.group_context_menu = None;
        }
    }
}
