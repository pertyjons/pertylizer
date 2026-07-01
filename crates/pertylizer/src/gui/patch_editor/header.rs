//! Module-header action-row subsystem for the patch editor.
//!
//! Holds the `PatchEditor` method that paints the trailing action controls in a
//! module's title bar (status badges, power/bypass, effect-chain reorder, info,
//! overflow menu, close). Extracted verbatim from `patch_editor.rs` — no
//! behavior change.

use eframe::egui::{self, Ui};
use egui_remixicon::icons as ri;

use synth_core::ModuleCategory;
use synth_engine::ModuleId;

use crate::gui::theme::theme;
use crate::gui::widgets::{ICON_BUTTON_SIZE, ModRole, icon_button};

use super::{
    DescriptionEditorState, EFFECT_CHAIN_AMBER, ModuleConnectivity, ModuleHeaderCtx, PatchAnalysis,
    PatchEditor, PatchEditorResult, STATUS_AUTOMATED_AMBER, STATUS_DISCONNECTED_GRAY,
    STATUS_OK_GREEN, STATUS_ORPHANED_YELLOW, STATUS_ROUTING_BLUE, STATUS_SINK_RED,
};

impl PatchEditor {
    /// Draw the non-interactive status badges for a module — source / sink /
    /// automation / connectivity / mod-matrix role — as informational
    /// [`icon_button`]s whose `.clicked()` is ignored (they only carry tooltips;
    /// the helper's forced arrow cursor keeps them from looking clickable).
    /// Rendered in the module's bottom status bar, not the header.
    pub(super) fn draw_module_status_badges(
        ui: &mut Ui,
        ctx: &ModuleHeaderCtx,
        analysis: &PatchAnalysis,
    ) {
        let &ModuleHeaderCtx {
            module_id,
            descriptor,
            is_source,
            is_sink,
            is_automated,
            is_global_module,
            connectivity,
            ..
        } = ctx;
        let t = theme();

        // Source indicator (no inputs)
        if is_source {
            icon_button(
                ui,
                ri::UPLOAD_2_FILL,
                STATUS_OK_GREEN,
                "Source Module\nGenerates signal (no incoming connections).",
            );
        }

        // Sink indicator (no outputs)
        if is_sink {
            icon_button(
                ui,
                ri::DOWNLOAD_2_FILL,
                STATUS_SINK_RED,
                "Sink Module\nConsumes signal (no outgoing connections).",
            );
        }

        // Automation indicator (referenced by a sequencer automation lane).
        if is_automated {
            icon_button(
                ui,
                ri::PULSE_FILL,
                STATUS_AUTOMATED_AMBER,
                "Automated\nA sequencer automation lane targets this module.",
            );
        }

        // Connectivity status indicator
        let (conn_icon, conn_color, conn_tooltip): (_, _, &str) = if is_global_module {
            if descriptor.category == ModuleCategory::Utility {
                (
                    ri::FLASHLIGHT_FILL,
                    STATUS_ROUTING_BLUE,
                    "Internal Routing\nRoutes modulation internally — no cables needed.",
                )
            } else {
                (
                    ri::FLASHLIGHT_FILL,
                    STATUS_ROUTING_BLUE,
                    "Global Module\nProcessed automatically via effect chain.",
                )
            }
        } else {
            match connectivity {
                ModuleConnectivity::Connected => (
                    ri::LINK,
                    STATUS_OK_GREEN,
                    "Routed to Output\nAudio from this module reaches the output.",
                ),
                ModuleConnectivity::Orphaned => (
                    ri::ERROR_WARNING_LINE,
                    STATUS_ORPHANED_YELLOW,
                    "Orphaned\nHas connections but signal doesn't reach output.\nConnect to a module that leads to Output.",
                ),
                ModuleConnectivity::Disconnected => (
                    ri::LINK_UNLINK,
                    STATUS_DISCONNECTED_GRAY,
                    "Disconnected\nNo cables connected.\nDrag from ports to create connections.",
                ),
            }
        };
        icon_button(ui, conn_icon, conn_color, conn_tooltip);

        let is_matrix_source = analysis.is_mod_matrix_source(module_id);
        let is_matrix_destination = analysis.is_mod_matrix_destination(module_id);
        if let Some(badge_role) = ModRole::from_flags(is_matrix_source, is_matrix_destination) {
            let badge_color = t.colors.accent_purple;
            // Share the icon mapping with the per-knob marker so the module
            // roll-up and its parameters never show conflicting arrows; the
            // tooltips stay module-level.
            let badge_icon = badge_role.glyph();
            let badge_tip = match (is_matrix_source, is_matrix_destination) {
                (true, true) => {
                    "Mod Matrix\nRouted as both source and destination via the Mod Matrix."
                }
                (true, false) => {
                    "Mod Matrix Source\nThis module drives one or more Mod Matrix slots.\nLook in the Mod Matrix module for slot details."
                }
                (false, true) => {
                    "Mod Matrix Destination\nA Mod Matrix slot modulates a parameter on this module.\nLook in the Mod Matrix module for slot details."
                }
                _ => "",
            };
            icon_button(ui, badge_icon, badge_color, badge_tip);
        }
    }

    /// Draw the interactive action controls in a module's header row: power/bypass,
    /// effect-chain reorder, info, overflow menu, and close. Pushes engine actions
    /// onto `result`; sets `*open = false` when the close button is clicked. Called
    /// as the `actions` closure of [`draw_module_header`]. (The non-interactive
    /// status badges live in the bottom status bar — see
    /// [`Self::draw_module_status_badges`].)
    pub(super) fn draw_module_header_actions(
        &mut self,
        ui: &mut Ui,
        ctx: &ModuleHeaderCtx,
        effect_chain_order: &[ModuleId],
        result: &mut PatchEditorResult,
        open: &mut bool,
    ) {
        let module_id = ctx.module_id;
        let is_bypassed = ctx.is_bypassed;
        let t = theme();

        // `draw_module_header` lays this row out right-to-left (its shared
        // `right_aligned_row`), so the first widget added lands rightmost. The
        // controls are therefore emitted in reverse reading order — close,
        // overflow, info, reorder (down before up), power — so they read
        // left→right as `power · up · down · info · overflow · close`.

        // Close/delete button (rightmost).
        if icon_button(ui, ri::CLOSE_LINE, t.colors.accent_primary, "Delete module").clicked() {
            *open = false;
        }

        // Overflow menu (⋯) — per-module actions. Currently just "Edit
        // description"; built to grow. Uses a frameless custom button so it
        // matches the other header icons (the default `menu_button` is boxed).
        let mut open_desc_editor = false;
        egui::containers::menu::MenuButton::from_button(
            egui::Button::new(
                egui::RichText::new(ri::MORE_FILL)
                    .color(t.colors.accent_primary)
                    .size(t.fonts.size_normal),
            )
            .frame(false)
            // The overflow menu needs a raw egui::Button (it can't go through
            // `icon_button`), so match the shared hit target explicitly.
            .min_size(ICON_BUTTON_SIZE),
        )
        .ui(ui, |ui| {
            if ui.button((ri::EDIT_LINE, "Edit description…")).clicked() {
                open_desc_editor = true;
                ui.close();
            }
        })
        .0
        .on_hover_cursor(egui::CursorIcon::Default);
        if open_desc_editor {
            let draft = self
                .panels
                .get(&module_id)
                .map(|p| p.description.clone())
                .unwrap_or_default();
            self.description_editor = Some(DescriptionEditorState { module_id, draft });
        }

        // Info (ⓘ) — toggles a read-only popup with the module's type
        // documentation + this instance's note.
        let info_open = self.info_popup == Some(module_id);
        if icon_button(
            ui,
            ri::INFORMATION_LINE,
            t.colors.accent_primary,
            "Module info\nType documentation + this instance's note.",
        )
        .clicked()
        {
            self.info_popup = if info_open { None } else { Some(module_id) };
        }

        // Effect chain reorder buttons (down/up arrows — added in RTL order so
        // they render up-left, down-right).
        if let Some(chain_pos) = effect_chain_order.iter().position(|id| *id == module_id) {
            let can_move_up = chain_pos > 0;
            let can_move_down = chain_pos + 1 < effect_chain_order.len();
            let chain_color = EFFECT_CHAIN_AMBER;

            // Down arrow
            let down_color = if can_move_down {
                chain_color
            } else {
                chain_color.gamma_multiply(0.3)
            };
            let down_resp = icon_button(
                ui,
                ri::ARROW_DOWN_S_LINE,
                down_color,
                "Move down in chain (process later)",
            );
            if down_resp.clicked() && can_move_down {
                result
                    .reorder_effects
                    .push((module_id, synth_engine::ReorderDirection::Down));
            }

            // Up arrow
            let up_color = if can_move_up {
                chain_color
            } else {
                chain_color.gamma_multiply(0.3)
            };
            let up_resp = icon_button(
                ui,
                ri::ARROW_UP_S_LINE,
                up_color,
                "Move up in chain (process earlier)",
            );
            if up_resp.clicked() && can_move_up {
                result
                    .reorder_effects
                    .push((module_id, synth_engine::ReorderDirection::Up));
            }
        }

        // Power/bypass button (leftmost).
        let (power_icon, power_color) = if is_bypassed {
            (ri::VOLUME_MUTE_FILL, t.colors.text_dim)
        } else {
            (ri::VOLUME_UP_FILL, t.colors.accent_green)
        };
        let power_tooltip = if is_bypassed {
            "Bypassed\nModule output is muted.\nClick to activate."
        } else {
            "Active\nModule is processing audio.\nClick to bypass."
        };
        if icon_button(ui, power_icon, power_color, power_tooltip).clicked() {
            let new_bypass_state = !is_bypassed;
            self.bypassed.insert(module_id, new_bypass_state);
            result.bypass_toggles.push((module_id, new_bypass_state));
        }
    }
}
