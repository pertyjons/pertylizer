//! Shared composite controls built from egui primitives.
//!
//! These capture UI idioms repeated across the views (themed labels, labeled
//! rows, numeric presets, toggle buttons, frameless icon buttons, section
//! headers, modal dialog scaffolding) so call sites stay short and visually
//! consistent, and so the styling lives in one place.

use std::ops::RangeInclusive;

use eframe::egui::{
    self, Button, Color32, DragValue, InnerResponse, Response, RichText, Slider, Ui, Vec2,
    WidgetText,
};

use crate::gui::theme::theme;

/// Minimum hit-target size for a frameless icon button.
const ICON_BUTTON_MIN_SIZE: f32 = 20.0;

/// A text button that reads as "active" (accent) or "inactive" (dimmed).
///
/// Returns the [`Response`] so callers can chain `.on_hover_text(..)` and
/// `.clicked()`. Active uses `accent_primary`, inactive uses `text_dim`.
pub fn toggle_button(ui: &mut Ui, label: impl Into<WidgetText>, active: bool) -> Response {
    toggle_button_colored(ui, label, active, theme().colors.accent_primary)
}

/// Like [`toggle_button`] but with a caller-chosen active color.
pub fn toggle_button_colored(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    active: bool,
    active_color: Color32,
) -> Response {
    let color = if active {
        active_color
    } else {
        theme().colors.text_dim
    };
    ui.button(label.into().strong().color(color))
}

/// A frameless icon button with a square, consistent hit target.
///
/// Returns the [`Response`] so callers can chain `.on_hover_text(..)`.
pub fn icon_button(ui: &mut Ui, icon: &str, color: Color32, icon_size: f32) -> Response {
    icon_button_sized(
        ui,
        icon,
        color,
        icon_size,
        Vec2::splat(ICON_BUTTON_MIN_SIZE),
    )
}

/// A frameless icon button with a caller-chosen minimum hit target. Use this for
/// tightly-packed rows (e.g. narrow status badges) where the default square
/// target is too wide; otherwise prefer [`icon_button`].
///
/// Returns the [`Response`] so callers can chain `.on_hover_text(..)`.
pub fn icon_button_sized(
    ui: &mut Ui,
    icon: &str,
    color: Color32,
    icon_size: f32,
    min_size: Vec2,
) -> Response {
    ui.add(
        Button::new(RichText::new(icon).color(color).size(icon_size))
            .frame(false)
            .min_size(min_size),
    )
}

/// A section heading: a strong, accent-colored label at the theme heading size,
/// an optional leading icon, an optional small dimmed description line below it,
/// then a small vertical gap.
///
/// Pass `None` for `icon`/`description` to omit them. Returns the title label's
/// [`Response`] so callers can chain `.on_hover_text(..)`.
pub fn section_header(
    ui: &mut Ui,
    title: &str,
    description: Option<&str>,
    color: Color32,
    icon: Option<&str>,
) -> Response {
    let t = theme();
    let text = match icon {
        Some(ic) => format!("{ic}  {title}"),
        None => title.to_string(),
    };
    let res = ui.label(
        RichText::new(text)
            .color(color)
            .size(t.fonts.size_heading)
            .strong(),
    );
    if let Some(desc) = description {
        caption(ui, desc, CaptionTone::Dim);
    }
    ui.add_space(t.spacing.widget_spacing);
    res
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

/// A dim label at the **normal** text size, in the theme's dim colour. Replaces
/// the hand-written `ui.label(RichText::new(t).color(theme().colors.text_dim))`.
/// For a *smaller* (`size_small`) dim sub-label, use [`caption`] with
/// [`CaptionTone::Dim`] instead — that is the only difference between the two.
pub fn dim_label(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    ui.label(text.into().color(theme().colors.text_dim))
}

/// Colour tone for a [`caption`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionTone {
    /// Dimmed (`theme().colors.text_dim`).
    Dim,
    /// Secondary (`theme().colors.text_secondary`).
    Secondary,
    /// An explicit caller-chosen colour (accent/runtime tints). Prefer a
    /// semantic variant when one fits — `Color` is not re-themeable.
    Color(Color32),
}

/// A small caption label: the theme's `size_small` (smaller than [`dim_label`])
/// plus the chosen tone colour. Folds the repeated
/// `RichText::new(x).size(size_small).color(..)` pattern (section sub-labels,
/// compact value read-outs, accent-tinted chips, …).
pub fn caption(ui: &mut Ui, text: impl Into<String>, tone: CaptionTone) -> Response {
    let t = theme();
    let color = match tone {
        CaptionTone::Dim => t.colors.text_dim,
        CaptionTone::Secondary => t.colors.text_secondary,
        CaptionTone::Color(c) => c,
    };
    ui.label(RichText::new(text).size(t.fonts.size_small).color(color))
}

/// A seconds `DragValue` preset (`" s"` suffix, caller-chosen drag `speed`). For
/// controls that are genuinely a time entry; for unit sliders use [`suffix_slider`].
pub fn time_drag_value(
    ui: &mut Ui,
    secs: &mut f32,
    range: RangeInclusive<f32>,
    speed: f64,
) -> Response {
    ui.add(DragValue::new(secs).speed(speed).range(range).suffix(" s"))
}

/// A `DragValue` carrying a unit suffix (e.g. `" %"`, `" st"`, `" ms"`) — the
/// non-seconds counterpart of [`time_drag_value`]. Generic over any numeric type.
/// Pass `""` for no suffix. For a styled prefix or custom formatter, use the raw
/// `egui::DragValue`.
pub fn unit_drag_value<T: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut T,
    range: RangeInclusive<T>,
    speed: f64,
    suffix: &str,
) -> Response {
    ui.add(
        DragValue::new(value)
            .range(range)
            .speed(speed)
            .suffix(suffix),
    )
}

/// A `Slider` carrying a unit suffix (e.g. `" Hz"`, `"m"`). Generic over any
/// numeric type so it also covers `i32`/`usize` state. Pass `""` for no suffix.
pub fn suffix_slider<T: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut T,
    range: RangeInclusive<T>,
    suffix: &str,
) -> Response {
    ui.add(Slider::new(value, range).suffix(suffix))
}

/// Like [`suffix_slider`] but on a logarithmic scale — for wide, frequency-like
/// ranges where low values need as much slider travel as high ones (e.g. a Hz
/// cutoff). A separate fn rather than a flag so the common linear case stays
/// argument-light.
pub fn log_suffix_slider<T: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut T,
    range: RangeInclusive<T>,
    suffix: &str,
) -> Response {
    ui.add(Slider::new(value, range).logarithmic(true).suffix(suffix))
}

/// A `ComboBox` over a fixed `(variant, label)` table: the selected variant's
/// label is shown, and picking a row writes it back to `current`. Folds the
/// repeated `from_id_salt(..).selected_text(..).show_ui(|ui| for .. {
/// selectable_value })` idiom. `id_salt` must be caller-supplied (and unique per
/// combo) — egui 0.35's `AsIdSalt` requires `Debug`. Returns the combo button's
/// [`Response`].
pub fn enum_combo<T: PartialEq + Copy>(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    current: &mut T,
    options: &[(T, &str)],
) -> Response {
    let selected = options
        .iter()
        .find(|(v, _)| v == current)
        .map_or("", |(_, l)| *l);
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (v, l) in options {
                ui.selectable_value(current, *v, *l);
            }
        })
        .response
}

/// A destructive text button tinted with the theme's `accent_red` (Delete, Clear,
/// Remove …). Folds the repeated `ui.button(RichText::new(x).color(accent_red))`
/// idiom so the destructive colour lives in one place.
pub fn danger_button(ui: &mut Ui, label: impl Into<WidgetText>) -> Response {
    ui.button(label.into().color(theme().colors.accent_red))
}

/// A bold (`.strong()`) inline label, optionally tinted — for lightweight
/// section/category titles that are NOT full [`section_header`]s (no
/// heading size, no trailing gap). Folds `ui.label(RichText::new(x).strong()
/// [.color(c)])`.
pub fn strong_label(ui: &mut Ui, text: impl Into<WidgetText>, color: Option<Color32>) -> Response {
    let text = text.into().strong();
    let text = match color {
        Some(c) => text.color(c),
        None => text,
    };
    ui.label(text)
}

/// A centered, dimmed empty-state placeholder filling the available area — the
/// "nothing selected / nothing here yet" message for a main panel. (For an inline
/// sidebar-list empty note, `list_panel::empty` stays its own italic variant.)
pub fn empty_state(ui: &mut Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(RichText::new(text).color(theme().colors.text_dim));
    });
}

/// A label that responds to clicks (`Sense::click()`) — for inline-editable /
/// selectable titles. Caller supplies the (optionally styled) text; the helper
/// adds the click sense. Returns the [`Response`] so callers read `.clicked()` /
/// `.double_clicked()`.
pub fn clickable_label(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    ui.add(egui::Label::new(text).sense(egui::Sense::click()))
}

/// A horizontal `label + widget` form row: a plain label, then the caller's
/// widget(s). Folds `ui.horizontal(|ui| { ui.label(label); … })` so label
/// styling/column behaviour lives in one place. Returns the closure's
/// [`InnerResponse`] (the row), so a caller can read `.inner` if needed.
pub fn labeled_row<R>(
    ui: &mut Ui,
    label: impl Into<WidgetText>,
    widget: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    ui.horizontal(|ui| {
        ui.label(label);
        widget(ui)
    })
}
