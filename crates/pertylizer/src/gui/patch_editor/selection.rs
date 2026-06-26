//! Selection and module-interaction subsystem for the patch editor.
//!
//! Holds the `PatchEditor` methods that manage z-order, module selection, and
//! per-module Area interaction (click/drag select, multi-select, drag-to-group
//! re-parenting). Extracted verbatim from `patch_editor.rs` — no behavior
//! change.

use std::collections::HashSet;

use eframe::egui::{self, Ui};

use synth_engine::ModuleId;

use crate::patch::GroupId;

use super::{GroupLayout, PatchEditor};

impl PatchEditor {
    /// Bring a module to front.
    pub fn bring_to_front(&mut self, id: ModuleId) {
        self.z_order.retain(|&mid| mid != id);
        self.z_order.push(id);
    }

    /// Draw a vertical column of ports (input or output side).
    /// Handle a module Area's interaction: click/drag selects it (shift/ctrl
    /// toggles multi-select) and brings it to front; drag-stop re-parents it to
    /// the group under its center.
    pub(super) fn handle_module_interaction(
        &mut self,
        ui: &mut Ui,
        response: &egui::Response,
        module_id: ModuleId,
        group_layout: &GroupLayout,
        bring_to_front: &mut Option<ModuleId>,
    ) {
        // Bring to front + update selection on click / drag-start.
        if response.clicked() || response.drag_started() {
            let modifiers = ui.input(|i| i.modifiers);
            if modifiers.shift || modifiers.ctrl {
                if self.selected_modules.contains(&module_id) {
                    self.selected_modules.remove(&module_id);
                } else {
                    self.selected_modules.insert(module_id);
                }
            } else if response.drag_started() && self.selected_modules.contains(&module_id) {
                // Dragging a card that's already part of the (multi-)selection:
                // keep the whole selection so the group moves together (see the
                // `DraggingNodes` handler). A plain click still collapses to one.
            } else {
                self.selected_modules.clear();
                self.selected_modules.insert(module_id);
            }
            self.selected_module = Some(module_id);
            self.selected_group = None;
            *bring_to_front = Some(module_id);
        }

        // On drag-stop, re-parent every card that moved to whatever group its own
        // center now sits in. The drag set is the current multi-selection when the
        // grabbed card is part of it (preserved above), else just the grabbed card
        // — mirroring the `DraggingNodes` set so a multi-drag re-parents the whole
        // group, not only the grabbed card.
        if response.drag_stopped() {
            let drag_set: Vec<ModuleId> = if self.selected_modules.contains(&module_id) {
                self.selected_modules.iter().copied().collect()
            } else {
                vec![module_id]
            };
            for mid in drag_set {
                let Some(panel_state) = self.panels.get(&mid) else {
                    continue;
                };
                let center = panel_state.position + panel_state.size / 2.0;
                let mut target_group: Option<GroupId> = None;
                for (gid, rect) in &group_layout.rects_world {
                    if rect.contains(center) {
                        target_group = Some(*gid);
                        break;
                    }
                }
                match target_group {
                    Some(gid) => {
                        if self.group_of(mid) != Some(gid) {
                            self.remove_from_group(mid);
                            self.add_module_to_group(gid, mid);
                        }
                    }
                    None => {
                        if self.group_of(mid).is_some() {
                            self.remove_from_group(mid);
                        }
                    }
                }
            }
        }
    }

    /// Select a set of modules (replaces current multi-selection).
    pub fn select_modules(&mut self, ids: &HashSet<ModuleId>) {
        self.selected_modules = ids.clone();
        self.selected_module = ids.iter().next().copied();
    }
}
