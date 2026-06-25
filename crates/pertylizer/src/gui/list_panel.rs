//! Shared building blocks for the left-hand browser lists (Instruments,
//! Patterns, Samples) so they share one uniform look: an icon + title header
//! with a right-aligned add button, a search field, frame-styled rows with a
//! per-row kebab (⋮) menu, and a consistent "used / unused" treatment (unused
//! rows are dimmed).
//!
//! The sequencer track-header list is intentionally NOT built from these — it
//! has to row-align with the arrangement timeline — but it borrows the same
//! colors so it stays tonally consistent.

use egui_remixicon::icons as ri;

use super::theme::theme;

/// Uniform panel width defaults shared by every browser list.
pub const DEFAULT_WIDTH: f32 = 200.0;
pub const MIN_WIDTH: f32 = 150.0;

/// Header row: `<icon> <title>` on the left, a right-aligned `+` add button.
///
/// Returns `true` on the frame the add button was clicked. Draws its own
/// trailing separator so callers don't repeat it.
#[must_use]
pub fn header(ui: &mut egui::Ui, icon: &str, title: &str, add_tooltip: &str) -> bool {
    let t = theme();
    let mut add_clicked = false;
    ui.horizontal(|ui| {
        // Normal (body) size + a little weight — matches the list rows so the
        // header doesn't tower over them; this also keeps the icon at row size.
        // Icon and title are separate atoms so the icon can take the accent
        // colour while the title stays in the primary text colour, sharing one
        // baseline and gap instead of being one pre-formatted string.
        egui::AtomLayout::new((
            egui::RichText::new(icon).color(t.colors.accent_primary),
            egui::RichText::new(title)
                .color(t.colors.text_primary)
                .strong(),
        ))
        .show(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            add_clicked = ui
                .button(
                    egui::RichText::new(ri::ADD_LINE.to_string()).color(t.colors.accent_primary),
                )
                .on_hover_text(add_tooltip)
                .clicked();
        });
    });
    ui.separator();
    add_clicked
}

/// Full-width search field with a `Search…` hint, followed by a little spacing.
pub fn search_box(ui: &mut egui::Ui, query: &mut String) {
    let t = theme();
    ui.add(
        egui::TextEdit::singleline(query)
            .hint_text("Search…")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(t.spacing.xs);
}

/// The text color a row's primary label should use, so every list colors its
/// rows the same way: cyan when selected, dimmed when unused, otherwise normal.
#[must_use]
pub fn row_text_color(selected: bool, used: bool) -> egui::Color32 {
    let t = theme();
    if selected {
        t.colors.accent_cyan
    } else if used {
        t.colors.text_primary
    } else {
        t.colors.text_dim
    }
}

/// Draw one uniform list row: the whole row is the click/selection surface (with
/// a hover highlight and pointer cursor), the name is painted on the left, and a
/// kebab (⋮) actions menu sits on the right.
///
/// `text`/`text_color` are the row's name (color via [`row_text_color`]).
/// `kebab` populates the per-row actions menu. Returns the row's response — use
/// it for selection (`clicked`/`double_clicked`) and tooltips.
///
/// The name is *painted* (not a widget) so it can't steal clicks or show a text
/// cursor, and the full-row response stays hovered across the whole row. The
/// kebab is a real widget drawn last, so on the overlap it wins its own clicks
/// (egui's hit-test breaks ties in favor of the top-most/last widget) while the
/// rest of the row selects.
pub fn row(
    ui: &mut egui::Ui,
    selected: bool,
    text: &str,
    text_color: egui::Color32,
    kebab: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let t = theme();
    let pad = 6.0;
    let kebab_w = 22.0;
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 8.0;

    // Full-row click surface, allocated first so it sits beneath the kebab.
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::click(),
    );

    // Selected / hover background (hover uses the rect directly so it lights up
    // even when the pointer is over the kebab).
    if selected {
        ui.painter().rect_filled(rect, 3.0, t.colors.bg_widget);
    } else if ui.rect_contains_pointer(rect) {
        ui.painter()
            .rect_filled(rect, 3.0, t.colors.bg_widget.gamma_multiply(0.5));
    }

    // Name, painted (clipped so it never overlaps the kebab).
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + pad, rect.top()),
        egui::pos2(rect.right() - kebab_w - pad, rect.bottom()),
    );
    ui.painter().with_clip_rect(label_rect).text(
        label_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::TextStyle::Body.resolve(ui.style()),
        text_color,
    );

    // Kebab, drawn last so it owns its clicks.
    let kebab_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - kebab_w - pad, rect.top()),
        egui::pos2(rect.right() - pad, rect.bottom()),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            // Salt with the row's own id so each kebab gets a unique BarState
            // (otherwise every row shares one menu id and the open-state visuals
            // bleed across rows).
            .id_salt(resp.id)
            .max_rect(kebab_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            ui.menu_button(
                egui::RichText::new(ri::MORE_FILL).color(t.colors.text_dim),
                kebab,
            );
        },
    );

    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Italic placeholder shown when a list has no entries.
pub fn empty(ui: &mut egui::Ui, text: &str) {
    let t = theme();
    ui.label(egui::RichText::new(text).color(t.colors.text_dim).italics());
}
