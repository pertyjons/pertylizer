//! Shared building blocks for the per-view context top-bars (Transport, Sample,
//! AWE, Pattern, Patch) so they all share one uniform look: a panel docked
//! directly under the global menu bar with a subtle panel fill, a uniform inner
//! margin, and consistent 8 px spacing between items.
//!
//! This is the sibling of [`super::list_panel`] (which unifies the left-hand
//! browser lists). Keeping every context bar behind this one helper means a
//! style tweak — fill, margin, spacing — happens in exactly one place.

use super::theme::theme;

/// The frame every context bar uses: a subtle `bg_panel` fill so the bar reads
/// as a toolbar distinct from the content below, plus an 8 × 4 px inner margin.
fn bar_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(theme().colors.bg_panel)
        .inner_margin(egui::Margin::symmetric(8, 4))
}

/// Standard horizontal spacing between items in any context bar.
fn item_spacing() -> f32 {
    theme().spacing.md // 8 px
}

/// Draw a view's context bar as a panel docked at the top of the current region
/// (i.e. directly under the global menu bar). `id` must be unique per view.
///
/// Returns whatever `add_contents` returns (e.g. the transport bar returns its
/// `is_playing` flag).
pub fn top<R>(
    ui: &mut egui::Ui,
    id: impl Into<egui::Id>,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Panel::top(id)
        .frame(bar_frame())
        .show_inside(ui, |ui| row(ui, add_contents))
        .inner
}

/// A secondary toolbar row stacked under a [`top`] bar (e.g. the piano-roll
/// tools/grid row): same item spacing as the bar above, but no panel of its own
/// so it sits inside the same region with a hairline gap above it.
pub fn secondary_row<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.add_space(theme().spacing.xxs);
    row(ui, add_contents)
}

/// Shared horizontal layout used by both bar kinds: a single `ui.horizontal`
/// row with the canonical item spacing applied.
fn row<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = item_spacing();
        add_contents(ui)
    })
    .inner
}
