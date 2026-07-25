//! Canvas background subsystem for the patch editor.
//!
//! Holds the `PatchEditor` methods that paint the canvas background (grid and
//! the tinted category/mod-matrix zones), handle empty-grid input (marquee
//! select, deselect, escape), and draw the background right-click context menu
//! (also reused by the module body menu). Extracted verbatim from
//! `patch_editor.rs` — no behavior change.

use eframe::egui::{self, Pos2, Rect, Ui};
use egui_remixicon::icons as ri;

use synth_core::{ModuleCategory, ModuleType};
use synth_engine::graph::Connection;

use crate::gui::module_panel::category_color;
use crate::gui::theme::theme;
use crate::gui::widgets::submenu_button;

use super::{
    BgContextMenuState, CanvasInteraction, GroupTemplateAction, PaletteSelection, PatchAnalysis,
    PatchEditor, PatchEditorResult, category_icon, draw_module_zone, module_catalog,
    screen_to_world,
};

impl PatchEditor {
    pub(super) fn handle_canvas_background_input(
        &mut self,
        ui: &mut Ui,
        canvas_response: &Option<egui::Response>,
    ) {
        // Rubber-band marquee selection (plan 4c): LEFT-drag on the empty grid.
        // Tracked in WORLD coords so the rectangle survives a mid-drag pan/zoom.
        //
        // We read RAW pointer state rather than a drag-sensing widget: a
        // drag-sensing background would capture the pointer and starve the
        // Scene's own secondary-drag pan, and calling `ui.response()` here would
        // double-register the scene Ui's id (the Scene already calls it for its
        // pan response) → an id clash. The click-only `canvas_response` tells us
        // (via `hovered`) that the press landed on empty grid and not on a
        // module/port that occludes it.
        //
        // `rubber_band_committed` makes the marquee path and the click-deselect
        // path below mutually exclusive: egui still reports `clicked()` for a
        // short drag, which would otherwise wipe a just-made marquee selection.
        let mut rubber_band_committed = false;
        {
            let world_pointer = ui
                .input(|i| i.pointer.interact_pos())
                .map(|p| screen_to_world(ui, p));
            let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
            let primary_down = ui.input(|i| i.pointer.primary_down());
            let shift = ui.input(|i| i.modifiers.shift);
            // `hovered()` (not `contains_pointer()`) so an occluding module/port
            // on top of the full-canvas bg widget correctly suppresses the start —
            // `contains_pointer` is purely geometric and would be true everywhere.
            let on_empty_grid = canvas_response.as_ref().is_some_and(|r| r.hovered());

            // Start: primary press on empty grid (and not mid wire-drag).
            if primary_pressed
                && on_empty_grid
                && self.pending_wire.is_none()
                && let Some(p) = world_pointer
            {
                self.canvas_interaction = CanvasInteraction::RubberBand {
                    start: p,
                    current: p,
                };
            }

            // While the marquee is live, track the cursor and commit on release.
            // Copy the corners out via a borrow first so the block can reassign
            // `self.canvas_interaction` (the enum is no longer `Copy`).
            let active_marquee = match &self.canvas_interaction {
                CanvasInteraction::RubberBand { start, current } => Some((*start, *current)),
                _ => None,
            };
            if let Some((start, current)) = active_marquee {
                let current = world_pointer.unwrap_or(current);
                if primary_down {
                    self.canvas_interaction = CanvasInteraction::RubberBand { start, current };
                } else {
                    // Released: this gesture owns the empty-grid press, so claim it
                    // regardless of size to keep the click-deselect below from
                    // double-handling it.
                    rubber_band_committed = true;
                    let rect = Rect::from_two_pos(start, current);
                    if rect.width() * rect.height() > 4.0 {
                        // Real marquee → select every module it touches.
                        // Shift adds to the selection; otherwise replace it.
                        if !shift {
                            self.selected_modules.clear();
                        }
                        for (id, panel) in &self.panels {
                            let panel_rect = Rect::from_min_size(panel.position, panel.size);
                            if rect.intersects(panel_rect) {
                                self.selected_modules.insert(*id);
                            }
                        }
                        self.selected_module = None;
                        self.selected_group = None;
                    } else if !shift {
                        // Tiny marquee == a plain click on empty grid → deselect.
                        self.selected_module = None;
                        self.selected_modules.clear();
                        self.selected_group = None;
                    }
                    self.canvas_interaction = CanvasInteraction::Idle;
                }
            }
        }

        // Draw the marquee while a rubber-band drag is active (on top of the
        // modules; world coords, so it pans/zooms with the canvas).
        if let CanvasInteraction::RubberBand { start, current } = &self.canvas_interaction {
            let rect = Rect::from_two_pos(*start, *current);
            let accent = theme().colors.accent_cyan;
            ui.painter()
                .rect_filled(rect, 0.0, accent.gamma_multiply(0.10));
            ui.painter().rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, accent.gamma_multiply(0.8)),
                egui::StrokeKind::Inside,
            );
        }

        // Handle click on empty space to deselect (unless the rubber-band path
        // already claimed this gesture — see `rubber_band_committed`).
        if !rubber_band_committed
            && let Some(response) = canvas_response
            && response.clicked()
        {
            self.selected_module = None;
            self.selected_modules.clear();
            self.selected_group = None;
        }

        // Right-click on background (or cable) → capture state for context menu
        if let Some(response) = canvas_response
            && response.secondary_clicked()
            && self.port_context_menu.is_none()
            && let Some(screen_pos) = ui.input(|i| i.pointer.interact_pos())
        {
            // Pointer is screen-space inside the Scene → convert to world.
            let world_pos = screen_to_world(ui, screen_pos);
            self.bg_context_menu = Some(BgContextMenuState {
                world_pos,
                cable: self.hovered_cable,
            });
        }

        // Safety net: if a node drag's stop frame was missed (e.g. the grabbed
        // card was deleted/bypassed mid-drag so its response never fired again),
        // don't get stuck in DraggingNodes once the button is up.
        if matches!(
            self.canvas_interaction,
            CanvasInteraction::DraggingNodes { .. }
        ) && !ui.input(|i| i.pointer.primary_down())
        {
            self.canvas_interaction = CanvasInteraction::Idle;
        }

        // Cancel any interaction (wire/rubber-band/node drag) + context menus with escape
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.canvas_interaction = CanvasInteraction::Idle;
            self.pending_wire = None;
            self.port_context_menu = None;
            self.bg_context_menu = None;
        }

        // Cursor follows the active interaction. Runs after the module loop, so a
        // live drag (Grabbing / Crosshair) overrides the per-card Grab-on-hover set
        // there. Idle leaves the hover-based cursors alone.
        match self.canvas_interaction {
            CanvasInteraction::DraggingNodes { .. } => {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            CanvasInteraction::RubberBand { .. } => {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            CanvasInteraction::Idle => {}
        }
        // A cable drag lives outside `canvas_interaction` (shared wire FSM), so
        // its crosshair is set separately — it can only coincide with `Idle`.
        if self.pending_wire.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
    }

    /// Draw a tinted background zone behind effect modules.
    pub(super) fn draw_effect_zone(&self, ui: &mut Ui) {
        let panels = self.panels.iter().filter_map(|(id, p)| {
            let category = self.descriptors.get(id).map(|d| d.category);
            matches!(category, Some(ModuleCategory::Effect)).then_some(p)
        });
        draw_module_zone(
            ui,
            panels,
            category_color(ModuleCategory::Effect),
            &format!("{} Effect Chain", ri::FLASHLIGHT_FILL),
        );
    }

    /// Draw a tinted background zone behind visualizer/monitor modules.
    /// Visualizers are passive taps on the final signal — they are not part of
    /// the effect chain (no ordering, no chain cables), so they get their own
    /// zone instead of being grouped with effects.
    pub(super) fn draw_monitors_zone(&self, ui: &mut Ui) {
        let panels = self.panels.iter().filter_map(|(id, p)| {
            let category = self.descriptors.get(id).map(|d| d.category);
            matches!(category, Some(ModuleCategory::Visualizer)).then_some(p)
        });
        draw_module_zone(
            ui,
            panels,
            category_color(ModuleCategory::Visualizer),
            &format!("{} Monitors", ri::PULSE_FILL),
        );
    }

    /// Draw a tinted framed zone behind any Mod Matrix modules. The bounding
    /// box expands to include cable-less matrix attachments (modules wired
    /// only through a matrix slot, auto-stacked beneath the matrix by
    /// `align_mod_matrix_attachments`) so they sit visibly inside the same
    /// frame. Modules that *also* have cables stay in the voice-graph flow
    /// and the frame does not engulf them.
    pub(super) fn draw_mod_matrix_zone(&self, ui: &mut Ui, analysis: &PatchAnalysis) {
        if analysis.count(ModuleType::ModMatrix) == 0 {
            return;
        }
        // `prev_mod_matrix_attachments` was refreshed earlier this frame by
        // `realign_mod_matrix_attachments_if_changed`, so it matches the
        // current attachment set without a second walk over all panels.
        let matrix_panels = self
            .panels
            .iter()
            .filter(|(id, _)| id.module_type == ModuleType::ModMatrix)
            .map(|(_, p)| p);
        let attachment_panels = self
            .prev_mod_matrix_attachments
            .iter()
            .filter_map(|id| self.panels.get(id));
        draw_module_zone(
            ui,
            matrix_panels.chain(attachment_panels),
            theme().colors.accent_purple,
            &format!("{} Mod Matrix", ri::PULSE_FILL),
        );
    }

    /// Draw the background right-click context menu for adding modules.
    pub(super) fn draw_bg_context_menu(
        &mut self,
        response: &egui::Response,
        result: &mut PatchEditorResult,
    ) {
        let menu_state = self.bg_context_menu;
        let world_pos = menu_state.map(|s| s.world_pos).unwrap_or_default();
        let menu_cable = menu_state.and_then(|s| s.cable);
        let mut selected: Option<PaletteSelection> = None;
        let mut cable_action_taken = false;

        response.context_menu(|ui| {
            let (s, c) = self.bg_context_menu_contents(ui, result, world_pos, menu_cable);
            selected = s;
            cable_action_taken = c;
        });

        if cable_action_taken {
            self.bg_context_menu = None;
            return;
        }

        if let Some(sel) = selected {
            result.context_add = Some((sel, world_pos, menu_cable));
            self.bg_context_menu = None;
        }

        // Clear stored state when context menu closes
        if !response.context_menu_opened() {
            self.bg_context_menu = None;
        }
    }

    /// The shared body of the rack context menu (Clipboard, groups, Insert
    /// Template, cable actions, and the Add-module submenus). Shown from both the
    /// canvas background and a module's empty body. Returns the picked palette
    /// selection (which the caller turns into `context_add` at `world_pos`) and
    /// whether a cable action was taken.
    #[allow(clippy::too_many_lines)]
    pub(super) fn bg_context_menu_contents(
        &mut self,
        ui: &mut egui::Ui,
        result: &mut PatchEditorResult,
        world_pos: Pos2,
        menu_cable: Option<Connection>,
    ) -> (Option<PaletteSelection>, bool) {
        use egui_remixicon::icons as ri;
        let mut selected: Option<PaletteSelection> = None;
        let mut cable_action_taken = false;

        {
            // --- Clipboard --- (the backend owns the clipboard; copy/cut act on
            // the current selection, paste is validated against clipboard state.)
            let has_selection = !self.effective_selection().is_empty();
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new((ri::FILE_COPY_LINE, "Copy")),
                )
                .clicked()
            {
                result.request_copy = true;
                ui.close();
            }
            if ui
                .add_enabled(
                    has_selection,
                    egui::Button::new((ri::SCISSORS_CUT_LINE, "Cut")),
                )
                .clicked()
            {
                result.request_cut = true;
                ui.close();
            }
            if ui.button((ri::CLIPBOARD_LINE, "Paste")).clicked() {
                result.request_paste = true;
                ui.close();
            }

            ui.separator();

            if !self.selected_modules.is_empty() {
                if ui.button("Create group from selection").clicked() {
                    self.create_group_from_selection();
                    ui.close();
                }
                ui.separator();
            }

            if ui
                .button((ri::FOLDER_ADD_LINE, "Insert Group Template"))
                .clicked()
            {
                result.group_template_action = Some(GroupTemplateAction::OpenBrowser {
                    drop_pos: world_pos,
                });
                ui.close();
            }

            ui.separator();

            // Cable actions (shown when right-clicking on a hovered cable)
            if let Some(connection) = menu_cable {
                if ui.button("Delete cable").clicked() {
                    self.connections.retain(|c| c != &connection);
                    result.connections_to_remove.push(connection);
                    self.calculate_connectivity();
                    cable_action_taken = true;
                    ui.close();
                }

                if ui.button("Insert Signal Monitor").clicked() {
                    self.connections.retain(|c| c != &connection);
                    result.connections_to_remove.push(connection);
                    result.insert_signal_monitor_at.push(connection);
                    self.calculate_connectivity();
                    cable_action_taken = true;
                    ui.close();
                }

                ui.separator();

                ui.label(
                    egui::RichText::new("Insert module...")
                        .color(theme().colors.text_secondary)
                        .size(11.0),
                );
            } else {
                ui.label(
                    egui::RichText::new("Add module")
                        .color(theme().colors.text_secondary)
                        .size(11.0),
                );
            }
            ui.separator();

            // Data-driven "Add module" menu: one submenu per category, populated
            // from the module catalog (which is built from `ALL_MODULE_TYPES`).
            // Adding a `ModuleType` variant and listing it in `ALL_MODULE_TYPES`
            // makes it appear here automatically — there is no hand-maintained
            // palette to drift out of sync with the enum. Every `ModuleCategory`
            // is listed below, so no category can be silently dropped either.
            const CATEGORY_ORDER: &[(ModuleCategory, &str)] = &[
                (ModuleCategory::Oscillator, "Oscillator"),
                (ModuleCategory::Filter, "Filter"),
                (ModuleCategory::Envelope, "Envelope"),
                (ModuleCategory::LFO, "LFO"),
                (ModuleCategory::Amplifier, "Amplifier"),
                (ModuleCategory::Mixer, "Mixer"),
                (ModuleCategory::Effect, "Effect"),
                (ModuleCategory::Sampler, "Sampler"),
                (ModuleCategory::Utility, "Modulation / Utility"),
                (ModuleCategory::Sequencer, "Generative"),
                (ModuleCategory::PhysicalModeling, "Physical"),
                (ModuleCategory::Visualizer, "Visualizer"),
                (ModuleCategory::Output, "Output"),
            ];
            for &(category, title) in CATEGORY_ORDER {
                // Skip a category with no modules rather than show an empty
                // submenu. (The closure below only runs when the submenu opens,
                // so presence must be checked up front.)
                if !module_catalog().iter().any(|(_, cat, _)| *cat == category) {
                    continue;
                }
                submenu_button(
                    ui,
                    egui::RichText::new(format!("{} {title}", category_icon(category)))
                        .color(category_color(category)),
                    |ui| {
                        for &(mt, cat, _) in module_catalog() {
                            if cat == category {
                                Self::bg_menu_item(ui, PaletteSelection::Module(mt), &mut selected);
                            }
                        }
                    },
                );
            }
        }

        (selected, cable_action_taken)
    }
}
