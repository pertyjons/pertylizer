//! Floating popups and windows for the patch editor.
//!
//! Holds the `PatchEditor` method that draws the module info / description
//! popups, plus the free helpers that draw the shared YAMS slot expression
//! editor and its reference window. Extracted verbatim from `patch_editor.rs`
//! — no behavior change.

use std::collections::HashSet;

use eframe::egui::{self, Ui};
use egui_remixicon::icons as ri;

use synth_engine::ModuleId;

use crate::gui::module_panel::ModulePanelState;
use crate::gui::theme::theme;
use crate::gui::widgets::{CaptionTone, caption, strong_label};

use super::{
    ModAddrCatalog, PatchEditor, PatchEditorResult, PickedInput, ScriptDepGraph,
    draw_select_input_menu, insert_at_cursor, insert_module_source, script_refs_from_inputs,
    set_text_caret,
};

impl PatchEditor {
    /// Draw the read-only info popup (ⓘ) and the "Edit description" editor when
    /// open. Both are floating windows anchored beside their module (via the
    /// per-frame `module_rects`); the editor pushes its result through
    /// `module_description_actions` on OK, the info popup is read-only.
    pub(super) fn draw_module_popups(&mut self, ui: &mut Ui, result: &mut PatchEditorResult) {
        let ctx = ui.ctx().clone();
        let t = theme();

        // Read-only info popup: module id, type name + type documentation, and
        // this instance's note.
        if let Some(mid) = self.info_popup {
            let anchor = self
                .module_rects
                .get(&mid)
                .map(|r| r.right_top() + egui::vec2(12.0, 0.0));
            let (type_name, type_desc) = self
                .descriptors
                .get(&mid)
                .map(|d| (d.name.to_string(), d.description.to_string()))
                .unwrap_or_default();
            let instance_desc = self
                .panels
                .get(&mid)
                .map(|p| p.description.clone())
                .unwrap_or_default();
            let mut keep_open = true;
            let mut win = egui::Window::new(format!("{}  {mid}", ri::INFORMATION_LINE))
                .id(egui::Id::new(("module_info_popup", mid)))
                .collapsible(false)
                .resizable(false)
                .open(&mut keep_open);
            if let Some(pos) = anchor {
                win = win.default_pos(pos);
            }
            win.show(&ctx, |ui| {
                strong_label(
                    ui,
                    format!("Type: {type_name}"),
                    Some(t.colors.text_secondary),
                );
                ui.separator();
                caption(ui, "Type documentation", CaptionTone::Dim);
                ui.label(if type_desc.is_empty() {
                    "(none)".to_string()
                } else {
                    type_desc
                });
                ui.separator();
                caption(ui, "Instance note", CaptionTone::Dim);
                ui.label(if instance_desc.is_empty() {
                    "(no description set)".to_string()
                } else {
                    instance_desc
                });
            });
            if !keep_open {
                self.info_popup = None;
            }
        }

        // "Edit description" editor: a small popup near the module with OK /
        // Cancel, styled like the expression editor.
        if let Some(mut editor) = self.description_editor.take() {
            let mid = editor.module_id;
            let anchor = self
                .module_rects
                .get(&mid)
                .map(|r| r.right_top() + egui::vec2(12.0, 0.0));
            let mut keep_open = true;
            let mut closed = false;
            let mut win = egui::Window::new(format!("{}  Edit description - {mid}", ri::EDIT_LINE))
                .id(egui::Id::new(("module_desc_editor", mid)))
                .collapsible(false)
                .resizable(true)
                .default_size(egui::vec2(360.0, 160.0))
                .min_width(260.0)
                .min_height(120.0)
                .open(&mut keep_open);
            if let Some(pos) = anchor {
                win = win.default_pos(pos);
            }
            win.show(&ctx, |ui| {
                caption(
                    ui,
                    "Per-instance note — what this specific module is for.",
                    CaptionTone::Secondary,
                );
                let reserved = 40.0;
                let editor_height = (ui.available_height() - reserved).max(60.0);
                ui.add_sized(
                    egui::vec2(ui.available_width(), editor_height),
                    egui::TextEdit::multiline(&mut editor.draft).desired_rows(3),
                );
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        result
                            .module_description_actions
                            .push((mid, editor.draft.clone()));
                        closed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        closed = true;
                    }
                });
            });
            // Persist the in-progress draft across frames unless dismissed.
            if keep_open && !closed {
                self.description_editor = Some(editor);
            }
        }
    }
}

/// Draw the shared per-slot YAMS expression-editor popup, reused by the Mod
/// Matrix, the control-rate Script module, and the per-sample AudioScript module
/// (`editor.audio_rate` selects the dialect for the live compile, title, hint,
/// help, and CPU-cost line). Compiles live for the status line (off the audio
/// thread) and pushes `(slot, Some(src))` / `(slot, None)` actions the caller
/// routes to `session.set_mod_script` / `clear_mod_script`. No-op when no slot's
/// editor is open. The window is keyed by `state.id`, so each module's editor is
/// independent.
pub(super) fn draw_slot_expression_editor(
    ui: &Ui,
    state: &mut ModulePanelState,
    script_graph: Option<&ScriptDepGraph>,
    catalog: &ModAddrCatalog,
    mod_script_actions: &mut Vec<(u8, Option<String>)>,
) {
    let Some(mut editor) = state.script_editor.take() else {
        return;
    };
    let module_id = state.id;
    let ctx = ui.ctx().clone();
    let mut keep_open = true;
    let mut closed_by_action = false;
    // AudioScript and the control-ports Script module each host a single program
    // rather than numbered Mod Matrix slots, so they get a program-centric title.
    let title = if editor.audio_rate {
        "Audio program - per-sample DSP".to_string()
    } else if editor.control_ports {
        "CV program - in1..in4 → out1..out4".to_string()
    } else {
        format!("Slot {} - Expression", editor.slot + 1)
    };
    egui::Window::new(title)
        .id(egui::Id::new(("mm_expr_editor", state.id, editor.slot)))
        .collapsible(false)
        .resizable(true)
        .default_width(520.0)
        .min_width(320.0)
        .open(&mut keep_open)
        .show(&ctx, |ui| {
            let hint = if editor.audio_rate {
                "YAMS audio program - per-sample DSP, e.g. `out = tanh(in * 4)` (stereo: `out.left`/`out.right`)"
            } else if editor.control_ports {
                "YAMS CV program - read in1..in4, write out1..out4, e.g. `out1 = in1 * in2`"
            } else {
                "YAMS expression - assign `out`, e.g. `out = lfo-1.out * velocity`"
            };
            crate::gui::script_editor::script_editor_header(
                ui,
                hint,
                "YAMS reference: how it runs, sources, functions",
                &mut editor.show_help,
            );
            // Fixed ~16-row editor: sizing it from `available_height` would feed
            // back into egui's auto-growing window (which never shrinks), so the
            // window crept taller whenever the warning line toggled. A constant
            // height breaks that loop; `add_sized` clamps it so long scripts scroll
            // inside the editor instead of growing the window. The stable id lets
            // the "Select input" picker move the caret, read back below.
            const EDITOR_ROWS: usize = 16;
            let te_id = egui::Id::new(("mm_expr_text", module_id, editor.slot));
            crate::gui::script_editor::script_code_editor(
                ui,
                te_id,
                &mut editor.draft,
                EDITOR_ROWS,
            );
            // Read the selection from the text edit's stored state: its live
            // `cursor_range` is only populated while focused, but opening the
            // "Select input" menu moves focus to the popup — so without this the
            // picker would append at the end instead of at the user's caret. A
            // non-empty range is replaced (like typing over a selection).
            let selection = egui::text_edit::TextEditState::load(ui.ctx(), te_id)
                .and_then(|st| st.cursor.char_range())
                .map(|r| {
                    // egui 0.35: cursor indices are now the strongly-typed
                    // `CharIndex(usize)`; unwrap to the raw offset.
                    let (a, b) = (r.primary.index.0, r.secondary.index.0);
                    (a.min(b), a.max(b))
                });

            // Live compile → status line (mirrors `session.set_mod_script`). The
            // same compile feeds the feedback-loop check below, so the draft is
            // compiled once per frame, not twice.
            let trimmed = editor.draft.trim();
            // `status`: compile result; `draft_refs`: module outputs the draft
            // reads (cycle check); `audio_ops`: compiled per-sample instruction
            // count, captured only for audio-rate programs (CPU-cost indicator).
            let (status, draft_refs, audio_ops) = if trimmed.is_empty() {
                (
                    Err("empty - Apply will clear the slot".to_string()),
                    HashSet::new(),
                    None,
                )
            } else {
                // Match the dialect `session::set_mod_script` will actually
                // compile with, so the live status never green-lights grammar the
                // install would reject (or vice-versa).
                let opts = synth_script::CompileOptions {
                    audio_rate: editor.audio_rate,
                    control_ports: editor.control_ports,
                    ..synth_script::CompileOptions::default()
                };
                let (program, diags) = synth_script::compile(&editor.draft, &opts);
                match program {
                    Some(p) => {
                        let bound = p.into_bound(String::new());
                        // The per-sample op count only matters for audio-rate
                        // programs (the cost indicator below).
                        let ops = editor.audio_rate.then(|| bound.script.code().len());
                        let refs = script_refs_from_inputs(&bound.inputs);
                        (Ok(()), refs, ops)
                    }
                    None => {
                        let msg = diags
                            .iter()
                            .filter(|d| d.is_error())
                            .map(|d| d.message.clone())
                            .collect::<Vec<_>>()
                            .join("; ");
                        (
                            Err(if msg.is_empty() {
                                "compile error".to_string()
                            } else {
                                msg
                            }),
                            HashSet::new(),
                            None,
                        )
                    }
                }
            };
            crate::gui::script_editor::script_status_line(ui, &status);

            // Live CPU-cost indicator (audio-rate only). YAMS runs every branch
            // every sample (eager evaluation), so the compiled instruction count
            // is the worst-case per-sample cost by construction — a faithful proxy
            // for CPU. We surface the exact op count (hard cap `MAX_INSTRUCTIONS`)
            // plus a rough core-fraction estimate anchored to the `yams_audio`
            // bench (~25 ops ≈ 9% of one core at 16 voices / 48 kHz). It is an
            // estimate, not a live measurement — per-module audio-thread timing is
            // out of scope (and would add forbidden syscalls to the hot path).
            if let Some(ops) = audio_ops {
                // Bench calibration: 9% core / 16 voices / 25 ops ≈ 0.36% per op.
                const EST_CORE_PCT_PER_OP_16V: f32 = 9.0 / 25.0;
                let pct16 = ops as f32 * EST_CORE_PCT_PER_OP_16V;
                // Tiers anchored to the bench: ≤24 ops ≈ light (<9%), ≤64 ≈
                // moderate (<23%), heavier programs warrant attention.
                let color = if ops <= 24 {
                    theme().colors.accent_green
                } else if ops <= 64 {
                    theme().colors.accent_yellow
                } else {
                    theme().colors.accent_orange
                };
                ui.label(
                    egui::RichText::new(format!(
                        "{}  ~{ops} ops/sample · ~{pct16:.0}% core @ 16 voices (est.)",
                        ri::CPU_LINE
                    ))
                    .size(theme().fonts.size_small)
                    .color(color),
                )
                .on_hover_text(format!(
                    "Per-sample instruction count (hard cap {cap}). Every branch \
                     runs every sample (eager evaluation), so this is the \
                     worst-case cost. The percentage is a rough estimate \
                     calibrated to the yams_audio benchmark at 48 kHz, not a live \
                     measurement; actual load scales with voice count and device \
                     sample rate. Heavy DSP inside a ternary branch is paid every \
                     sample — lift it into a `state` cell instead.",
                    cap = synth_core::script::MAX_INSTRUCTIONS,
                ));
            }

            // Feedback-loop notice (§3.5): a script reading its own or a
            // downstream script's output forms a one-block-delayed loop the cable
            // cycle-detection can't see. Purely informational — delayed feedback
            // (e.g. a leaky integrator) is sometimes intentional, so we warn
            // rather than block. Only Script-module slots can close such a loop.
            // The control-script dependency graph doesn't model the per-sample
            // AudioScript path (whose feedback is intentional `state`-cell IIR),
            // so the warning is suppressed there to avoid misleading notices.
            if !editor.audio_rate
                && let Some(warning) =
                    script_graph.and_then(|g| g.cycle_warning(module_id, editor.slot, &draft_refs))
            {
                ui.label(
                    egui::RichText::new(format!("{}  {warning}", ri::ALERT_LINE))
                        .size(theme().fonts.size_small)
                        .color(theme().colors.accent_yellow),
                );
            }

            ui.horizontal(|ui| {
                // "Select input" inserts a source reference at the caret: a bare
                // identifier for a macro/context var, or a `src <var> = <addr>`
                // binding + variable for a module output port.
                if let Some(pick) = draw_select_input_menu(ui, catalog) {
                    let new_caret = match pick {
                        PickedInput::Bare(name) => {
                            insert_at_cursor(&mut editor.draft, selection, &name)
                        }
                        PickedInput::ModuleSource(addr) => {
                            insert_module_source(&mut editor.draft, selection, &addr)
                        }
                    };
                    set_text_caret(&ctx, te_id, new_caret);
                }
                // Format runs the canonical yamsfmt formatter and replaces the
                // draft with its output. Enabled only when the script is valid
                // (the formatter parses first; a broken script can't be formatted).
                if ui
                    .add_enabled(status.is_ok(), egui::Button::new("Format"))
                    .on_hover_text("Reformat the expression (yamsfmt)")
                    .clicked()
                    && let Ok(formatted) = synth_script::format(&editor.draft)
                {
                    editor.draft = formatted;
                }
                if ui
                    .add_enabled(status.is_ok(), egui::Button::new("Apply"))
                    .on_hover_text("Install this expression on the slot (keeps editing)")
                    .clicked()
                {
                    // Install but leave the popup open so the user can keep
                    // iterating (Close / ✕ dismisses it).
                    mod_script_actions.push((editor.slot, Some(editor.draft.clone())));
                }
                if ui
                    .button("Clear")
                    .on_hover_text("Remove the expression from this slot")
                    .clicked()
                {
                    mod_script_actions.push((editor.slot, None));
                    closed_by_action = true;
                }
                if ui.button("Close").clicked() {
                    closed_by_action = true;
                }
            });
        });

    // YAMS reference panel, toggled by the Help button. A sibling window so it
    // can be read side-by-side with the editor; its ✕ mirrors back to the toggle.
    if editor.show_help {
        let mut help_open = true;
        draw_yams_help_window(
            &ctx,
            state.id,
            editor.slot,
            editor.audio_rate,
            &mut help_open,
        );
        if !help_open {
            editor.show_help = false;
        }
    }

    // Persist the editor (with its in-progress draft) across frames unless the
    // window was closed via its ✕ or an action button.
    if keep_open && !closed_by_action {
        state.script_editor = Some(editor);
    }
}

/// The YAMS reference panel: a scrollable, read-only cheat-sheet covering the
/// execution model, source syntax, the predefined identifiers, and the function
/// catalog. Mirrors `docs/yams.md` so the in-app help stays close to the
/// canonical reference. The execution-model and sources sections branch on
/// `audio_rate` so an AudioScript author sees the per-sample dialect (`in`,
/// `out.left`/`out.right`, `first_sample`, `state` cells) rather than the
/// control-rate semantics.
pub(super) fn draw_yams_help_window(
    ctx: &egui::Context,
    module: ModuleId,
    slot: u8,
    audio_rate: bool,
    open: &mut bool,
) {
    let t = theme();
    // Section heading.
    let head = |ui: &mut Ui, text: &str| {
        ui.add_space(6.0);
        strong_label(ui, text, Some(t.colors.accent_cyan));
    };
    // Body paragraph.
    let body = |ui: &mut Ui, text: &str| {
        ui.label(egui::RichText::new(text).size(t.fonts.size_small));
    };
    // Monospace example line.
    let code = |ui: &mut Ui, text: &str| {
        ui.label(
            egui::RichText::new(text)
                .monospace()
                .size(t.fonts.size_small)
                .color(t.colors.text_secondary),
        );
    };

    let title = if audio_rate {
        "YAMS - audio-rate reference"
    } else {
        "YAMS - reference"
    };
    egui::Window::new(title)
        .id(egui::Id::new(("yams_help", module, slot)))
        .collapsible(true)
        .resizable(true)
        .default_size(egui::vec2(420.0, 460.0))
        .min_width(300.0)
        .open(open)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                head(ui, "What it is");
                if audio_rate {
                    body(
                        ui,
                        "YAMS audio-rate is a small per-sample DSP language. The \
                         program runs once per audio sample per voice, reading the \
                         incoming sample with `in` (or `in_l`/`in_r`) and writing \
                         the output sample to `out` (mono) or `out.left`/ \
                         `out.right` (stereo). You process the actual waveform.",
                    );
                } else {
                    body(
                        ui,
                        "YAMS is a small control-rate expression language. Each slot \
                         computes one value per voice, assigned to `out`. That value \
                         becomes a normalized modulation offset (output ports are ±1; a \
                         parameter maps to 0..1, or -1..1 if bipolar).",
                    );
                }

                if audio_rate {
                    head(ui, "How it runs (per sample)");
                    body(
                        ui,
                        "The program runs once per AUDIO SAMPLE (e.g. 48000 Hz), so \
                         `in` is the live input waveform, not a sampled point. Keep \
                         it cheap: this is the audio hot path. Use `state` cells for \
                         feedback / IIR filters (a `state` value persists across \
                         samples); `first_sample` is 1 only on the very first sample \
                         after note-on for one-shot init.",
                    );
                    body(
                        ui,
                        "Evaluation is eager: every node runs EVERY SAMPLE. `?:`, \
                         `&&`, `||` only SELECT an already-computed value - both arms \
                         are paid in full per sample, so do not hide a heavy biquad \
                         inside a ternary branch; lift it into a `state` cell. Math \
                         is NaN-free (x/0 = 0, safe log/sqrt).",
                    );
                } else {
                    head(ui, "How it runs (the key expectation)");
                    body(
                        ui,
                        "The script runs once per CONTROL BLOCK (~187-750 Hz), not per \
                         audio sample. Every source you reference is SAMPLED once per \
                         block - you read a value, you do not process the waveform. So \
                         `osc-1.out` gives one sampled point of that oscillator per \
                         block (great for a sub-audio osc used as an LFO; aliased for an \
                         audio-rate one), never the audible waveform. Per-sample audio \
                         processing is the separate AudioScript module.",
                    );
                    body(
                        ui,
                        "Sources resolve with a one-block latency, so a script reading \
                         its own or a downstream script's output is a delayed feedback \
                         path - allowed, but flagged below the editor.",
                    );
                    body(
                        ui,
                        "Evaluation is eager: every node runs every block. `?:`, `&&`, \
                         `||` only SELECT an already-computed value - stateful functions \
                         (lag, phasor, ...) keep ticking even on the untaken branch. \
                         Math is NaN-free (x/0 = 0, safe log/sqrt).",
                    );
                }

                head(ui, "Syntax");
                if audio_rate {
                    code(ui, "state y = 0          # persists across samples");
                    code(ui, "y = y + 0.1*(in - y) # one-pole lowpass");
                    code(ui, "out = tanh(y * 2)    # mono out");
                    code(ui, "# or stereo: out.left = ...  out.right = ...");
                    body(
                        ui,
                        "`state` declarations first, then `let`s and assignments in \
                         order, then `out` (or `out.left`/`out.right`). Numbers need \
                         a leading zero (0.5, not .5). Comments: # to end of line.",
                    );
                } else {
                    code(ui, "src lfo = lfo-1.out   # bind a source by address");
                    code(ui, "let depth = mod_wheel * 0.5");
                    code(ui, "out = lfo * depth     # exactly one `out`");
                    body(
                        ui,
                        "All `src` bindings first, then `let`s, then one `out`. \
                         Numbers need a leading zero (0.5, not .5). Durations: 50ms, \
                         1.5s. Comments: # to end of line. Use the \"Select input\" \
                         button to insert a source without typing the address.",
                    );
                }

                head(ui, "Sources you can reference");
                if audio_rate {
                    body(
                        ui,
                        "- Audio input (per sample): in (= left), in_l, in_r. \
                         first_sample is 1 on the first sample after note-on.",
                    );
                }
                body(
                    ui,
                    "- Any module output port by address: lfo-1.out, env-2.out, \
                     scr-1.out1 (bound with `src`).",
                );
                body(
                    ui,
                    "- Macros (bare names, 0..1 unless noted): velocity, mod_wheel, \
                     aftertouch, pitch_bend (-1..1), note, poly_at.",
                );
                // Generated from CONTEXT_CATALOG so a new context var appears here
                // automatically — no hand-maintained mirror to drift (TODO 2.6).
                let context_vars = synth_script::symbols::CONTEXT_CATALOG
                    .iter()
                    .map(|(name, label)| format!("{name} ({label})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                body(
                    ui,
                    &format!(
                        "- Context: {context_vars}. The transport vars (beat, \
                         bar_phase, tempo, playing) are shared by all voices - \
                         tempo-synced: out = sin(beat * tau)."
                    ),
                );
                body(ui, "- Constants: pi, tau (2*pi), e.");

                head(ui, "Functions - stateless");
                code(ui, "abs sign min max clamp");
                code(ui, "floor ceil round trunc quantize(x,step)");
                code(ui, "pow sqrt exp log");
                code(ui, "sin cos tan atan atan2");
                code(
                    ui,
                    "tanh (soft-clip)  lerp/mix(a,b,t) smoothstep sigmoid gauss",
                );
                code(ui, "semis(x)->ratio  mtof(x)->Hz");

                head(ui, "Functions - stateful (per-voice, reset on note-on)");
                code(ui, "lag(x,t)         one-pole smoothing");
                code(ui, "slew(x,up,down)  separate rise/fall rates");
                code(ui, "sah(x,trig)      sample & hold on rising trig");
                code(ui, "accum(x)         running sum   delta(x) change/block");
                code(ui, "phasor(rate)     own 0->1 ramp at rate Hz");
                code(ui, "edge(x) counter(trig)");
                code(ui, "rand([lo,hi])    white()   (decorrelated per voice)");
                body(
                    ui,
                    "A literal time arg (lag(x, 50ms)) precomputes its coefficient; \
                     an expression arg costs a per-block recompute.",
                );

                head(ui, "Operators (low -> high precedence)");
                code(ui, "?:   ||   &&   == !=   < > <= >=   + -   * / %");
                code(ui, "unary - !    ^ (power)    f(...)  ( )");
                body(
                    ui,
                    "Comparisons can't chain: a < b < c is a syntax error - \
                     parenthesize. Truthiness: any non-zero is true; comparisons \
                     yield 1 or 0.",
                );
            });
        });
}
