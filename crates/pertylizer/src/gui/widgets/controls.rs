//! Shared composite controls built from egui primitives.
//!
//! These capture UI idioms repeated across the views (toggle buttons, frameless
//! icon buttons, section headers, modal dialog scaffolding) so call sites stay
//! short and visually consistent, and so the styling lives in one place.

use eframe::egui::{self, Button, Color32, Response, RichText, Ui, Vec2};

use crate::gui::theme::theme;

/// Minimum hit-target size for a frameless icon button.
const ICON_BUTTON_MIN_SIZE: f32 = 20.0;

/// A text button that reads as "active" (accent) or "inactive" (dimmed).
///
/// Returns the [`Response`] so callers can chain `.on_hover_text(..)` and
/// `.clicked()`. Active uses `accent_primary`, inactive uses `text_dim`.
pub fn toggle_button(ui: &mut Ui, label: impl Into<String>, active: bool) -> Response {
    toggle_button_colored(ui, label, active, theme().colors.accent_primary)
}

/// Like [`toggle_button`] but with a caller-chosen active color.
pub fn toggle_button_colored(
    ui: &mut Ui,
    label: impl Into<String>,
    active: bool,
    active_color: Color32,
) -> Response {
    let color = if active {
        active_color
    } else {
        theme().colors.text_dim
    };
    ui.button(RichText::new(label.into()).strong().color(color))
}

/// A frameless icon button with a consistent hit target.
///
/// Returns the [`Response`] so callers can chain `.on_hover_text(..)`.
pub fn icon_button(ui: &mut Ui, icon: &str, color: Color32, icon_size: f32) -> Response {
    ui.add(
        Button::new(RichText::new(icon).color(color).size(icon_size))
            .frame(false)
            .min_size(Vec2::splat(ICON_BUTTON_MIN_SIZE)),
    )
}

/// A section heading: a strong, accent-colored label at the theme heading size,
/// followed by a small vertical gap.
pub fn section_header(ui: &mut Ui, label: &str, color: Color32) {
    let t = theme();
    ui.label(
        RichText::new(label)
            .color(color)
            .size(t.fonts.size_heading)
            .strong(),
    );
    ui.add_space(t.spacing.widget_spacing);
}

/// Outcome of a [`dialog_button_row`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogButton {
    /// Neither button was pressed this frame.
    None,
    /// The Cancel button was pressed.
    Cancel,
    /// The confirm/action button was pressed.
    Confirm,
}

/// A standard `[Cancel] [Confirm]` button row.
///
/// The confirm button is disabled unless `can_confirm` is true.
pub fn dialog_button_row(ui: &mut Ui, confirm_label: &str, can_confirm: bool) -> DialogButton {
    let mut result = DialogButton::None;
    ui.horizontal(|ui| {
        if ui.button("Cancel").clicked() {
            result = DialogButton::Cancel;
        }
        if ui
            .add_enabled(can_confirm, Button::new(confirm_label))
            .clicked()
        {
            result = DialogButton::Confirm;
        }
    });
    result
}

/// Standard centered, non-resizable modal window scaffold.
///
/// Returns the closure's value when the window is shown, `None` otherwise.
pub fn modal_window<R>(
    ctx: &egui::Context,
    title: &str,
    content: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, content)
        .and_then(|r| r.inner)
}
