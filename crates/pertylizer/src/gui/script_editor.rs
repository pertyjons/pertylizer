//! Shared building blocks for the YAMS expression editors (§10.4).
//!
//! The Mod-Matrix / control-rate `Script` / per-sample `AudioScript` popup
//! ([`patch_editor::popups`](crate::gui::patch_editor)) and the Note Grid's
//! `note_event` script popup ([`note_grid_view`](crate::gui::note_grid_view))
//! render the same editor core: a fixed-height code field and a live
//! compile-status line. Those two pieces live here so all three script variants
//! share one editor surface rather than each hand-rolling its own — the
//! dialect-specific chrome (source pickers, CPU cost, feedback-loop notices)
//! stays with each caller.

use eframe::egui::{self, Response, Ui};
use egui_remixicon::icons as ri;

use crate::gui::theme::theme;
use crate::gui::widgets::selectable_toggle;

/// The shared fixed-height YAMS code editor. The height is pinned to
/// `rows × monospace line height` rather than derived from the available
/// height, so it can't feed egui's auto-growing window height back on itself
/// (long scripts scroll inside the field instead of growing the window). `id`
/// must be stable across frames so the caller can read the caret back from the
/// stored [`egui::text_edit::TextEditState`].
pub(crate) fn script_code_editor(
    ui: &mut Ui,
    id: egui::Id,
    source: &mut String,
    rows: usize,
) -> Response {
    let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
    ui.add_sized(
        egui::vec2(ui.available_width(), rows as f32 * row_h),
        egui::TextEdit::multiline(source)
            .id(id)
            .code_editor()
            .desired_rows(rows),
    )
}

/// The shared editor-header row for a script popup: a wrapping dialect hint plus
/// a right-aligned Help toggle. The Help button sits on its own row so the hint
/// below can wrap to the full width without ever colliding with the button (all
/// three script popups shared the same overlap bug). Flips `show_help` on click.
pub(crate) fn script_editor_header(
    ui: &mut Ui,
    hint: &str,
    help_hover: &str,
    show_help: &mut bool,
) {
    let t = theme();
    // Help toggle on its own single-row band. The `ui.horizontal` bounds the row
    // height — a bare `right_to_left` directly in the window's top-down ui would
    // instead claim the full available height and center the button halfway down
    // the window. The hint then wraps freely on the row(s) below.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            selectable_toggle(
                ui,
                show_help,
                egui::RichText::new(format!("{}  Help", ri::QUESTION_LINE))
                    .size(t.fonts.size_small),
                help_hover,
            );
        });
    });
    ui.add(
        egui::Label::new(
            egui::RichText::new(hint)
                .size(t.fonts.size_small)
                .color(t.colors.text_secondary),
        )
        .wrap(),
    );
}

/// A focused reference window for the `note_event` script dialect (§R5): what it
/// reads and writes, the drop/sentinel rules, an example, and the function
/// catalog. Opened by the note-script popup's Help toggle; `id` keys the window,
/// `open` mirrors its ✕ back to the caller's toggle.
pub(crate) fn draw_note_script_help_window(ctx: &egui::Context, id: egui::Id, open: &mut bool) {
    let t = theme();
    let head = |ui: &mut Ui, text: &str| {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(text)
                .strong()
                .color(t.colors.accent_cyan),
        );
    };
    let body = |ui: &mut Ui, text: &str| {
        ui.label(egui::RichText::new(text).size(t.fonts.size_small));
    };
    let code = |ui: &mut Ui, text: &str| {
        ui.label(
            egui::RichText::new(text)
                .monospace()
                .size(t.fonts.size_small)
                .color(t.colors.text_secondary),
        );
    };

    egui::Window::new("Note script — note_event reference")
        .id(id)
        .collapsible(true)
        .resizable(true)
        .default_size(egui::vec2(430.0, 460.0))
        .min_width(300.0)
        .open(open)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                head(ui, "What it is");
                body(
                    ui,
                    "The note_event script runs once per note, 1-to-1 and \
                     stateless. It rewrites the note's fields; any field you \
                     don't assign passes through unchanged.",
                );

                head(ui, "Reads");
                code(ui, "note_pitch   MIDI pitch, 0..127");
                code(ui, "note_vel     velocity, 0..1");
                code(ui, "note_dur     duration in ticks (-1 = until cut)");
                code(ui, "tick         absolute tick of the note");
                code(ui, "in1 .. in4   Value inputs (0..1) from mod cables");

                head(ui, "Writes");
                code(ui, "out.pitch    rounded, clamped to 0..127");
                code(ui, "out.vel      clamped to 0..1");
                code(ui, "out.dur      clamped to >= 0 ticks");
                code(ui, "out.gate     gate fraction (staccato..tenuto)");

                head(ui, "Dropping & sentinels");
                body(
                    ui,
                    "A negative out.vel drops the note. A negative out.dur \
                     restores \"plays until cut\". There is no probability \
                     field — to drop notes at random, set out.vel = -1 behind a \
                     seeded rand().",
                );

                head(ui, "Example");
                code(ui, "out.pitch = note_pitch + 12");
                code(ui, "out.vel = note_vel * (0.5 + 0.5 * in1)");

                head(ui, "Functions");
                body(
                    ui,
                    "Stateless YAMS math: min, max, clamp, abs, floor, ceil, \
                     round, pow, sqrt, exp, log, sin, cos, tan, tanh, lerp, \
                     smoothstep, sigmoid, mtof, semis, unipolar, bipolar — plus \
                     a seeded rand(). Operators: + - * / and the ternary \
                     cond ? a : b.",
                );
            });
        });
}

/// A one-line, truncated preview of a YAMS source for a module/node panel row
/// (first non-blank line, clipped with an ellipsis). Shared by the rack's script
/// slot rows and the Note Grid's script node body so both preview identically.
pub(crate) fn script_preview(src: &str) -> String {
    const MAX: usize = 24;
    let line = src.lines().next().unwrap_or("").trim();
    if line.chars().count() > MAX {
        let head: String = line.chars().take(MAX).collect();
        format!("{head}…")
    } else {
        line.to_string()
    }
}

/// The shared live compile-status line: a green "compiled" on `Ok`, the joined
/// diagnostic (orange) on `Err`. The compile itself is the caller's job — the
/// dialects differ (control-rate / audio-rate / note-event) and some callers
/// need the compiled byproducts — so this only renders the verdict.
pub(crate) fn script_status_line(ui: &mut Ui, status: &Result<(), String>) {
    let t = theme();
    match status {
        Ok(()) => {
            ui.label(
                egui::RichText::new(format!("{}  compiled", ri::CHECKBOX_CIRCLE_LINE))
                    .size(t.fonts.size_small)
                    .color(t.colors.accent_green),
            );
        }
        Err(e) => {
            ui.label(
                egui::RichText::new(format!("{}  {e}", ri::ERROR_WARNING_LINE))
                    .size(t.fonts.size_small)
                    .color(t.colors.accent_orange),
            );
        }
    }
}
