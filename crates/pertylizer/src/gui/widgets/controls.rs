//! Shared composite controls built from egui primitives.
//!
//! These capture UI idioms repeated across the views (themed labels, labeled
//! rows, numeric presets, toggle buttons, frameless icon buttons, section
//! headers, modal dialog scaffolding) so call sites stay short and visually
//! consistent, and so the styling lives in one place.

use std::ops::RangeInclusive;

use eframe::egui::{
    self, Button, Color32, DragValue, InnerResponse, Rect, Response, RichText, Slider, Ui, Vec2,
    WidgetText,
};

use egui_remixicon::icons as ri;

use super::ModMarkers;
use crate::gui::theme::theme;

/// Paint each active modulation marker in its fixed corner of `rect`, each with its
/// own hover tooltip. Painter-based so the glyphs never change the widget's
/// footprint. Each marker kind owns one corner (see
/// [`ModMarker::corner`](super::ModMarker::corner)), so two markers never collide
/// and a glyph's position reinforces its kind. With `outside` the glyphs sit just
/// clear of the widget body (knobs); otherwise they tuck into the corners (ports).
/// `interact` uses `&Ui`, so this composes even while a `Painter` borrow is live.
pub fn paint_marker_corners(ui: &Ui, rect: Rect, markers: ModMarkers, outside: bool) {
    for m in markers.iter() {
        let (pos, align) = m.corner(rect, outside);
        let glyph_rect = ui.painter().text(
            pos,
            align,
            m.glyph(),
            egui::FontId::proportional(11.0),
            m.color(),
        );
        ui.interact(
            glyph_rect,
            ui.id().with(("mod_marker", m as u8)),
            egui::Sense::hover(),
        )
        .on_hover_text(m.tooltip());
    }
}

// --- AccessKit exposure -----------------------------------------------------
//
// Custom-painted widgets (knobs, ports, node cards, canvases, meters) never fill
// their AccessKit node, so the egui-inspection MCP (`query_tree`,
// `content_contains`, `click`, `drag`) sees them as `Unknown` with no label or
// value. These helpers wrap `Response::widget_info`, which fills the node every
// frame (present in the tree even without interaction). Call once per frame, right
// before returning the `Response`.
//
// Cost: `widget_info`'s node-fill closure is skipped when AccessKit is off (a
// release build without the `egui-inspection` feature never runs it), so the
// per-frame overhead there is near-zero. But the `label` you pass is still built
// by the caller every frame — a `format!(...)` label allocates whether or not
// AccessKit is on. Keep labels cheap on hot paths; don't reach for heavier
// lazy-string machinery, a short `format!` per visible widget is fine on the GUI
// thread.
//
// `WidgetType` convention for full-surface responses: use `Panel` for a
// background container whose contents are separate AccessKit nodes drawn on top
// (the patch canvas has node cards, the tempo lane has points, a group box has
// its module body); use `Other` for a monolithic custom surface with no per-
// element child nodes in the tree (piano roll, arrangement, tracker, AWE room,
// sample waveform, keyboard).
//
// (An `expose_painted` escape hatch for pure-paint elements with no `Response`,
// e.g. cables, would need `accesskit::` types that are only linked under the
// opt-in `egui-inspection` feature; it is deferred with its only consumer.)

/// Expose a custom-painted, `Response`-backed widget to AccessKit with a stable,
/// meaningful `label` (the human/MCP name — module name, port name+type, tab
/// name). `value` lands in the node's `value` field (matched by the MCP's
/// `value_contains`); pass `None` for non-numeric widgets. Prefer the specific
/// [`egui::WidgetType`] (`Slider`, `Button`, …) over `Other`.
pub fn expose(
    response: &Response,
    typ: egui::WidgetType,
    label: impl Into<String>,
    value: Option<f64>,
) {
    let label = label.into();
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(typ, response.enabled(), label.clone());
        info.value = value;
        info
    });
}

/// Variant for on/off / active-tab widgets: reports `selected` state so the MCP
/// can read which tab/toggle is active.
pub fn expose_selected(
    response: &Response,
    typ: egui::WidgetType,
    label: impl Into<String>,
    selected: bool,
) {
    let label = label.into();
    response.widget_info(|| {
        egui::WidgetInfo::selected(typ, response.enabled(), selected, label.clone())
    });
}

/// The single hit-target size for every frameless icon button (and status badge).
/// Exposed so callers that must build a raw `egui::Button` (e.g. a `MenuButton`)
/// can match it instead of repeating the literal.
pub const ICON_BUTTON_SIZE: Vec2 = Vec2::new(18.0, 20.0);

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

/// A frameless icon button — one uniform look for every icon: the
/// `ICON_BUTTON_SIZE` (18×20) hit target, `size_normal` glyph, the given hover
/// `tooltip`, and the arrow cursor. Used both for interactive controls (read
/// `.clicked()`) and for non-interactive status badges (ignore the response).
/// Returns the [`Response`].
pub fn icon_button(ui: &mut Ui, icon: &str, color: Color32, tooltip: &str) -> Response {
    // Force the arrow cursor: icon buttons never use the pointing-hand. This also
    // overrides any cursor an enclosing draggable container paints over itself
    // (e.g. the patch-editor module card's `Grab`), since the button draws after.
    let response = ui.add(
        Button::new(
            RichText::new(icon)
                .color(color)
                .size(theme().fonts.size_normal),
        )
        .frame(false)
        .min_size(ICON_BUTTON_SIZE),
    );
    // The glyph is a raw Remix Icon codepoint (e.g. `\u{EE29}`), which egui would
    // otherwise register as the AccessKit label — unmatchable by the MCP. Override
    // it with the tooltip's first line (human-readable, state-aware for toggles:
    // "Muted"/"Audible"), so every mute/solo/bypass/close/toolbar button is
    // queryable and clickable by name.
    let clean_label = tooltip.lines().next().unwrap_or(tooltip);
    expose(&response, egui::WidgetType::Button, clean_label, None);
    response
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::Default)
}

// --- Audible on/off toggles -------------------------------------------------
//
// One canonical look per concept, reused everywhere an audible source is
// silenced, isolated, or bypassed. Each is a frameless [`icon_button`], so the
// hit target (18×20), glyph size, and cursor match across the mixer, the
// arrangement, the piano roll, and the patch/effect module headers. Filled glyph
// + accent color = the "engaged" state; line glyph + `text_secondary` = the
// neutral resting state. Keeping icon, color, and tooltip here means a mute looks
// identical wherever it appears, and mute / solo / bypass stay visually distinct.

/// Mute toggle — red `volume-mute` when silenced, neutral `volume-up` when
/// audible. Returns the [`Response`]; read `.clicked()` to flip the state.
pub fn mute_toggle(ui: &mut Ui, muted: bool) -> Response {
    let t = theme();
    let (icon, color, tip) = if muted {
        (
            ri::VOLUME_MUTE_FILL,
            t.colors.accent_red,
            "Muted\nOutput is silenced.\nClick to unmute.",
        )
    } else {
        (
            ri::VOLUME_UP_FILL,
            t.colors.text_secondary,
            "Audible\nOutput is playing.\nClick to mute.",
        )
    };
    icon_button(ui, icon, color, tip)
}

/// Solo toggle — yellow filled headphone when isolating, neutral outline
/// headphone otherwise. The DAW convention for solo. Returns the [`Response`].
pub fn solo_toggle(ui: &mut Ui, soloed: bool) -> Response {
    let t = theme();
    let (icon, color, tip) = if soloed {
        (
            ri::HEADPHONE_FILL,
            t.colors.accent_yellow,
            "Soloed\nOnly soloed sources are heard.\nClick to unsolo.",
        )
    } else {
        (
            ri::HEADPHONE_LINE,
            t.colors.text_secondary,
            "Solo\nIsolate this source.\nClick to solo.",
        )
    };
    icon_button(ui, icon, color, tip)
}

/// Bypass/power toggle for an effect or module — a power glyph that glows green
/// (`shut-down` filled) while processing and dims to a neutral outline when
/// bypassed. A distinct glyph from [`mute_toggle`] so "effect off" never reads as
/// "channel muted". Returns the [`Response`].
pub fn bypass_toggle(ui: &mut Ui, bypassed: bool) -> Response {
    let t = theme();
    let (icon, color, tip) = if bypassed {
        (
            ri::SHUT_DOWN_LINE,
            t.colors.text_secondary,
            "Bypassed\nSignal passes through unprocessed.\nClick to activate.",
        )
    } else {
        (
            ri::SHUT_DOWN_FILL,
            t.colors.accent_green,
            "Active\nProcessing audio.\nClick to bypass.",
        )
    };
    icon_button(ui, icon, color, tip)
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

/// A `menu_button` styled as a fixed-width dropdown, for **hierarchical** (tree)
/// pickers built from nested `menu_button`s — the shape used by the Script ƒx
/// "Select input" picker, the Mod Matrix source/destination pickers, and the
/// piano-roll "Auto:" selector. Unlike a [`ComboBox`](egui::ComboBox) this lets
/// the dropdown contents nest submenus. `label` is the current selection shown on
/// the button; the justified layout stretches it to `width` so adjacent pickers
/// stay aligned; `id_salt` keeps buttons with identical labels from colliding.
/// `contents` builds the dropdown tree (typically mutating a captured result).
pub fn tree_picker_button(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
    label: impl Into<WidgetText>,
    contents: impl FnOnce(&mut Ui),
) {
    ui.push_id(id_salt, |ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(width, ui.spacing().interact_size.y),
            egui::Layout::top_down_justified(egui::Align::Min),
            |ui| {
                ui.menu_button(label, contents);
            },
        );
    });
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

/// Outcome of one frame of an [`inline_editable_text`] editor.
pub struct InlineEdit {
    /// The underlying widget response — read `.changed()` for live-commit
    /// callers, or any other field you need.
    pub response: Response,
    /// The edit session ended this frame: focus was lost, which for a
    /// singleline field also covers pressing Enter. Callers finalize the value
    /// and clear their "currently editing this row" state when this is `true`.
    pub ended: bool,
}

/// An auto-focused inline text editor. The field grabs keyboard focus on the
/// frame it first appears (so the caret lands without a second click) and keeps
/// it until focus is lost. This folds only the focus-grab + end-of-edit
/// detection that was copy-pasted across the pattern/track name and description
/// editors; the caller still owns the "am I editing this row?" state and the
/// commit policy (commit-on-`ended` for names, commit-on-`response.changed()`
/// for live-updating descriptions).
///
/// `multiline` picks singleline vs multiline; `configure` tweaks the builder
/// (width, font, hint, rows) for the specific call site.
pub fn inline_editable_text<'t>(
    ui: &mut Ui,
    buf: &'t mut String,
    multiline: bool,
    configure: impl FnOnce(egui::TextEdit<'t>) -> egui::TextEdit<'t>,
) -> InlineEdit {
    let base = if multiline {
        egui::TextEdit::multiline(buf)
    } else {
        egui::TextEdit::singleline(buf)
    };
    let response = ui.add(configure(base));
    let ended = response.lost_focus();
    // Grab focus on the first frame (and any frame it isn't yet focused),
    // except the frame the edit ends — mirroring the original `else if` so a
    // lost-focus commit isn't cancelled by an immediate re-focus.
    if !ended && !response.has_focus() {
        response.request_focus();
    }
    InlineEdit { response, ended }
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

/// Pin `content` to the **right edge** of the current row, laying its widgets
/// out **right-to-left**: the *first* widget added lands rightmost. Callers
/// therefore add their trailing widgets in reverse reading order (rightmost
/// first). Use inside a `ui.horizontal` where earlier content (e.g. a title)
/// takes the left slack.
pub fn right_aligned_row<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), content)
        .inner
}

/// A `leading  −  +` stepper: the label followed by two square −/+ buttons as
/// **plain widgets**, so egui centres all three on the row's shared centreline
/// like any neighbouring label. (An earlier `egui::AtomLayout` version sat the
/// label a few px above the centreline — the block's text atom does not centre
/// against the taller button slots.) Returns the net step: −1, 0, or +1.
pub fn stepper(ui: &mut Ui, leading: RichText) -> i32 {
    ui.label(leading);
    // Square buttons the size of a default interactive line.
    let slot = Vec2::splat(ui.spacing().interact_size.y);
    let mut delta = 0;
    if ui.add(Button::new("-").min_size(slot)).clicked() {
        delta -= 1;
    }
    if ui.add(Button::new("+").min_size(slot)).clicked() {
        delta += 1;
    }
    delta
}
