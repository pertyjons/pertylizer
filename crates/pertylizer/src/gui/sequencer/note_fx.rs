//! The Note FX panel — a right-docked panel binding a pooled note graph to a
//! pattern. Pick a graph (or "None"), fork a shared one ("make unique"), jump
//! into the Note Grid view to edit its nodes, or freeze the result into plain
//! notes. Node/processor editing lives in the Note Grid view (`note_grid_view`),
//! which reuses this module's per-type editor bodies ([`edit_scale_quantize`]
//! and friends).
//!
//! The binding lives on the shared `Song` (`Pattern.note_graph`); the audio
//! thread re-reads it each tick, so edits go through `song.write()` + the undo
//! manager, never an `EngineCommand`.

use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
use egui_remixicon::icons as ri;
use synth_sequencer::{
    ArpMode, ArpRate, ArpVelocity, Arpeggiator, Chord, Duration as SeqDuration, Humanize,
    MAX_ARP_OFFSETS, NoteGraphId, NoteName, NoteProcessor, PatternId, PitchClass, ScaleMask,
    ScaleQuantize, StrumDirection,
};

use crate::gui::theme::theme;
use crate::gui::widgets::{
    dim_label, enum_combo, icon_button, knob_normalized, labeled_row, seed_reroll, strong_label,
    unit_drag_value,
};
use crate::undo::UndoAction;

use super::SequencerViewState;

/// A panel edit deferred until after the snapshot has been drawn, so we never
/// mutate the `Song` while reading the cloned snapshot.
enum PanelEdit {
    /// Bake the bound note graph — or, with no binding, per-note ornaments —
    /// into plain notes and clear the source.
    Freeze,
    /// Bind (or clear) the pattern's pooled note graph.
    BindGraph(Option<NoteGraphId>),
    /// Fork the bound shared graph into a copy and repoint this pattern
    /// (plan §1.2 "make unique" — the instrument-duplication move).
    MakeUnique,
}

/// Snapshot of the pattern's bound note graph for the binding row.
struct BoundGraph {
    id: NoteGraphId,
    /// `None` = the binding dangles (graph deleted elsewhere).
    name: Option<String>,
    /// How many patterns bind this graph (drives "make unique").
    usage: usize,
}

/// Draw the Note FX panel for `pattern_id`: bind a pooled note graph (or clear
/// it), fork a shared graph, jump into the Note Grid view to edit it, or freeze
/// the result into plain notes. Applies at most one edit per frame.
pub(crate) fn draw_note_fx_panel(
    ui: &mut egui::Ui,
    song: &Arc<synth_sequencer::SharedSong>,
    view_state: &mut SequencerViewState,
    undo_manager: &mut crate::undo::UndoManager,
    pattern_id: PatternId,
) {
    let t = theme();

    // Header row: title + close.
    ui.horizontal(|ui| {
        strong_label(ui, "NOTE FX", Some(t.colors.accent_purple));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("✖")
                .on_hover_text("Hide the Note FX rack")
                .clicked()
            {
                view_state.note_fx_panel_open = false;
            }
        });
    });
    ui.separator();

    let mut edit: Option<PanelEdit> = None;

    // Snapshot ornament presence, the graph binding, and the graph pool so the
    // lock is held only briefly.
    let Some((has_bakeable_notes, bound_graph, graph_pool)) = song.try_read().and_then(|s| {
        s.pattern(pattern_id).map(|p| {
            let bound = p.note_graph().map(|gid| BoundGraph {
                id: gid,
                name: s.note_graph(gid).map(|g| g.name.clone()),
                usage: s.note_graph_usage(gid),
            });
            (
                // Match the freeze condition: a count < 2 ornament is a no-op the
                // bake skips, so it must not enable the button. Note-scope
                // bindings also bake (plan §2.1), so they count too.
                p.notes()
                    .iter()
                    .any(|n| n.ornament.is_some_and(|o| o.count >= 2))
                    || p.notes().iter().any(|n| n.note_graph.is_some()),
                bound,
                s.note_graphs()
                    .map(|g| (g.id, g.name.clone()))
                    .collect::<Vec<_>>(),
            )
        })
    }) else {
        dim_label(ui, "Pattern unavailable…");
        return;
    };

    // ── Note graph binding (plan §8.2): one pooled graph per pattern. A bound
    // graph processes the pattern's notes at playback; with none, the raw notes
    // (+ ornaments) play. ──
    labeled_row(ui, "Graph", |ui| {
        let selected_label = match &bound_graph {
            Some(bg) => bg
                .name
                .clone()
                .unwrap_or_else(|| format!("missing graph {}", bg.id.0)),
            None => "None".to_owned(),
        };
        egui::ComboBox::from_id_salt("note_fx_graph_binding")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                let bound_id = bound_graph.as_ref().map(|bg| bg.id);
                if ui.selectable_label(bound_id.is_none(), "None").clicked() && bound_id.is_some() {
                    edit = Some(PanelEdit::BindGraph(None));
                }
                for (gid, name) in &graph_pool {
                    if ui.selectable_label(bound_id == Some(*gid), name).clicked()
                        && bound_id != Some(*gid)
                    {
                        edit = Some(PanelEdit::BindGraph(Some(*gid)));
                    }
                }
            });
        if let Some(bg) = &bound_graph {
            if bg.name.is_none() {
                // Dangling binding: the graph is gone from the pool, so the
                // edit/fork affordances would act on nothing (playback falls
                // back to the rack). Offer only the way out.
                dim_label(ui, "graph missing — playback uses the rack");
            } else {
                if icon_button(
                    ui,
                    ri::EDIT_LINE,
                    t.colors.text_secondary,
                    "Edit in the Note Grid view",
                )
                .clicked()
                {
                    view_state.jump_to_note_graph = Some(bg.id);
                }
                if bg.usage > 1 {
                    if ui
                        .small_button("Make unique")
                        .on_hover_text(format!(
                            "Shared by {} patterns — clone the graph and point only this pattern at the copy.",
                            bg.usage
                        ))
                        .clicked()
                    {
                        edit = Some(PanelEdit::MakeUnique);
                    }
                } else {
                    dim_label(ui, "only user");
                }
            }
        }
    });
    // A dangling binding resolves to nothing at playback, so it counts as "no
    // graph". With a graph bound the pattern is processed by that graph (plan
    // §2.2); with none, its raw notes play (plus any per-note ornaments).
    let graph_resolves = bound_graph.as_ref().is_some_and(|bg| bg.name.is_some());
    ui.add_space(4.0);

    // Freeze: bake the bound graph (or per-note ornaments / note-scope
    // articulation) into plain notes. Enabled when there is anything to bake.
    ui.add_enabled_ui(graph_resolves || has_bakeable_notes, |ui| {
        ui.menu_button("Freeze", |ui| {
            let what = if graph_resolves {
                "Bakes the bound note graph into plain notes, then clears the binding (the pooled graph survives). Undoable."
            } else {
                "Bakes per-note ornaments and note-scope articulation into plain notes. Undoable."
            };
            ui.label(RichText::new(what).color(t.colors.text_dim));
            if ui.button("Freeze now").clicked() {
                edit = Some(PanelEdit::Freeze);
                ui.close();
            }
        });
    });
    ui.add_space(6.0);

    // Node editing lives in the Note Grid view; this panel only binds a pooled
    // graph to the pattern.
    if !graph_resolves {
        ui.label(
            RichText::new(
                "Bind a note graph above to arpeggiate, build chords, snap to a \
                 scale, or humanize this pattern. Create and edit graphs in the \
                 Note Grid view.",
            )
            .color(t.colors.text_dim),
        );
    }

    // Apply the deferred edit.
    match edit {
        Some(PanelEdit::Freeze) => {
            let before;
            {
                let mut song_w = song.write();
                let bpm = song_w.tempo_at(synth_sequencer::Tick(0));
                before = song_w.pattern(pattern_id).cloned();
                // Song::freeze_pattern owns the graph-over-rack precedence, so
                // freeze always bakes what playback plays.
                let stats = song_w.freeze_pattern(pattern_id, bpm);
                // Surface an overflow (a graph node hit the 128-event cap) to the
                // activity log — the non-RT drop report (plan §7).
                if stats.dropped > 0 {
                    tracing::warn!(
                        target: "pertylizer::note_grid",
                        dropped = stats.dropped,
                        "Freeze dropped {} events — a note-graph node hit the 128-event cap",
                        stats.dropped
                    );
                }
            }
            if let Some(before) = before {
                undo_manager.push(UndoAction::FreezePattern { pattern_id, before });
            }
        }
        Some(PanelEdit::BindGraph(new)) => {
            let mut old = None;
            {
                let mut song_w = song.write();
                if let Some(pattern) = song_w.pattern_mut(pattern_id) {
                    old = Some(pattern.note_graph());
                    pattern.set_note_graph(new);
                }
            }
            if let Some(old) = old
                && old != new
            {
                undo_manager.push(UndoAction::SetPatternNoteGraph {
                    pattern_id,
                    old,
                    new,
                });
            }
        }
        Some(PanelEdit::MakeUnique) => {
            // Clone the shared graph, repoint this pattern at the copy; one
            // Composite so undo removes the clone and restores the binding.
            let mut result = None;
            {
                let mut song_w = song.write();
                let old_gid = song_w.pattern(pattern_id).and_then(|p| p.note_graph());
                if let Some(old_gid) = old_gid
                    && let Some(clone) = song_w.duplicate_note_graph(old_gid)
                {
                    if let Some(pattern) = song_w.pattern_mut(pattern_id) {
                        pattern.set_note_graph(Some(clone.id));
                    }
                    result = Some((old_gid, clone.id, clone));
                }
            }
            if let Some((old_gid, new_gid, clone)) = result {
                undo_manager.push(UndoAction::Composite(vec![
                    UndoAction::SetNoteGraph {
                        graph_id: new_gid,
                        old: None,
                        new: Some(clone),
                    },
                    UndoAction::SetPatternNoteGraph {
                        pattern_id,
                        old: Some(old_gid),
                        new: Some(new_gid),
                    },
                ]));
            }
        }
        None => {}
    }
}

// ============================================================================
// Per-type editors
// ============================================================================

pub(crate) fn edit_scale_quantize(ui: &mut egui::Ui, index: usize, q: &mut ScaleQuantize) {
    labeled_row(ui, "Root", |ui| {
        let cur = q.root.as_u8();
        egui::ComboBox::from_id_salt((index, "root"))
            .selected_text(NoteName::from_midi(cur).to_string())
            .show_ui(ui, |ui| {
                for i in 0..12u8 {
                    if ui
                        .selectable_label(cur == i, NoteName::from_midi(i).to_string())
                        .clicked()
                    {
                        q.root = PitchClass::new(i);
                    }
                }
            })
            .response
            .on_hover_text("Tonic the scale is built on.");
        ui.label("Scale");
        egui::ComboBox::from_id_salt((index, "scale"))
            .selected_text(scale_mask_name(q.mask))
            .show_ui(ui, |ui| {
                for (mask, label) in SCALES {
                    if ui.selectable_label(q.mask == mask, label).clicked() {
                        q.mask = mask;
                    }
                }
            })
            .response
            .on_hover_text("Scale the incoming pitches are snapped to.");
    });
    // Custom 12-pill mask row — toggle individual pitch classes.
    ui.horizontal_wrapped(|ui| {
        for i in 0..12u8 {
            let on = q.mask.contains_interval(i);
            if ui
                .selectable_label(on, NoteName::from_midi(i).to_string())
                .on_hover_text("Toggle this pitch class in/out of the scale.")
                .clicked()
            {
                let mut ivs: Vec<u8> = (0..12u8).filter(|&j| q.mask.contains_interval(j)).collect();
                if on {
                    ivs.retain(|&j| j != i);
                } else {
                    ivs.push(i);
                }
                q.mask = ScaleMask::from_intervals(&ivs);
            }
        }
    });
}

pub(crate) fn edit_chord(ui: &mut egui::Ui, index: usize, c: &mut Chord, any_dragged: &mut bool) {
    labeled_row(ui, "Type", |ui| {
        egui::ComboBox::from_id_salt((index, "chordtype"))
            .selected_text(chord_preset_name(c))
            .show_ui(ui, |ui| {
                for (intervals, label) in CHORD_PRESETS {
                    if ui
                        .selectable_label(c.intervals() == intervals, label)
                        .clicked()
                    {
                        *c = rebuild_chord(intervals, c.strum(), c.direction());
                    }
                }
            })
            .response
            .on_hover_text("Chord preset — sets the intervals stacked on each note.");
    });

    // Interval chips (click to remove) + a menu to append common intervals.
    ui.horizontal_wrapped(|ui| {
        let ivs: Vec<i8> = c.intervals().to_vec();
        let mut remove_idx = None;
        for (i, &iv) in ivs.iter().enumerate() {
            if ui
                .button(format!("{iv:+} ✖"))
                .on_hover_text("Remove interval")
                .clicked()
            {
                remove_idx = Some(i);
            }
        }
        if let Some(i) = remove_idx {
            let mut nv = ivs.clone();
            nv.remove(i);
            *c = rebuild_chord(&nv, c.strum(), c.direction());
        }
        ui.menu_button("+ interval", |ui| {
            for (semi, label) in CHORD_INTERVAL_CHOICES {
                if ui.button(label).clicked() {
                    let mut nv = c.intervals().to_vec();
                    if nv.len() < synth_sequencer::MAX_CHORD_INTERVALS {
                        nv.push(semi);
                        *c = rebuild_chord(&nv, c.strum(), c.direction());
                    }
                    ui.close();
                }
            }
        });
    });

    // Strum spread + direction (direction only meaningful while strumming).
    ui.horizontal(|ui| {
        let mut spread = c.strum().0;
        let resp = ui
            .add(egui::Slider::new(&mut spread, 0..=480).text("Strum"))
            .on_hover_text("Spread between chord tones, in ticks (0 = block chord).");
        *any_dragged |= resp.dragged();
        if spread != c.strum().0 {
            let ivs: Vec<i8> = c.intervals().to_vec();
            *c = rebuild_chord(&ivs, SeqDuration(spread), c.direction());
        }
        ui.add_enabled_ui(spread > 0, |ui| {
            let mut dir = c.direction();
            enum_combo(
                ui,
                (index, "dir"),
                &mut dir,
                &[(StrumDirection::Up, "Up"), (StrumDirection::Down, "Down")],
            )
            .on_hover_text("Strum direction (low→high or high→low).");
            if dir != c.direction() {
                let ivs: Vec<i8> = c.intervals().to_vec();
                *c = rebuild_chord(&ivs, c.strum(), dir);
            }
        });
    });
}

pub(crate) fn edit_arpeggiator(
    ui: &mut egui::Ui,
    index: usize,
    a: &mut Arpeggiator,
    any_dragged: &mut bool,
) {
    labeled_row(ui, "Mode", |ui| {
        enum_combo(ui, (index, "mode"), &mut a.mode, &ARP_MODES)
            .on_hover_text("Order the held notes are cycled in.");
        ui.label("Rate");
        arp_rate_widget(ui, index, a, any_dragged);
    });
    labeled_row(ui, "Octaves", |ui| {
        let resp = ui
            .add(egui::DragValue::new(&mut a.octaves).range(1..=4))
            .on_hover_text("How many octaves the figure spans.");
        *any_dragged |= resp.dragged();
        ui.label("Vel");
        enum_combo(ui, (index, "vel"), &mut a.velocity, &ARP_VELS)
            .on_hover_text("Velocity profile across the arp steps.");
        ui.checkbox(&mut a.latch, "Latch")
            .on_hover_text("Keep arpeggiating held notes after they are released.");
        ui.checkbox(&mut a.legato, "Legato").on_hover_text(
            "Hold one envelope across the figure (glide between steps). Use a high Gate.",
        );
        ui.checkbox(&mut a.continuous_phase, "Free-run")
            .on_hover_text("Keep the step cycle aligned to absolute pattern time.");
    });
    if a.mode == ArpMode::Custom {
        edit_arp_offsets(ui, &mut a.custom, any_dragged);
    }
    ui.horizontal(|ui| {
        knob_normalized(ui, "Gate", &mut a.gate, any_dragged)
            .on_hover_text("Note length as a fraction of the step (0..1).");
        knob_normalized(ui, "Swing", &mut a.swing, any_dragged)
            .on_hover_text("Delay every other step for a shuffle feel (0..1).");
    });
}

/// Rate picker: the named divisions plus the two data-carrying chiptune rates
/// (`Ticks`, `Hz`). When a data rate is selected an adjacent `DragValue` edits
/// its tick count / frequency.
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn arp_rate_widget(ui: &mut egui::Ui, index: usize, a: &mut Arpeggiator, any_dragged: &mut bool) {
    let selected = match a.rate {
        ArpRate::Ticks(_) => "Ticks",
        ArpRate::MilliHz(_) => "Hz",
        other => ARP_RATES
            .iter()
            .find(|(v, _)| *v == other)
            .map_or("", |(_, l)| l),
    };
    egui::ComboBox::from_id_salt((index, "rate"))
        .selected_text(selected)
        .show_ui(ui, |ui| {
            for (v, l) in ARP_RATES {
                ui.selectable_value(&mut a.rate, v, l);
            }
            // Switching *into* a data rate seeds a musical default (one PAL
            // frame @125 BPM / 50 Hz); staying on it keeps the edited value.
            if ui
                .selectable_label(matches!(a.rate, ArpRate::Ticks(_)), "Ticks (sub-grid)")
                .clicked()
                && !matches!(a.rate, ArpRate::Ticks(_))
            {
                a.rate = ArpRate::Ticks(40);
            }
            if ui
                .selectable_label(matches!(a.rate, ArpRate::MilliHz(_)), "Hz (frame-locked)")
                .clicked()
                && !matches!(a.rate, ArpRate::MilliHz(_))
            {
                a.rate = ArpRate::MilliHz(50_000);
            }
        });
    match &mut a.rate {
        ArpRate::Ticks(n) => {
            let resp = ui.add(egui::DragValue::new(n).range(1..=3840).suffix(" tk"));
            *any_dragged |= resp.dragged();
        }
        ArpRate::MilliHz(m) => {
            // Authored in Hz; stored as integer millihertz.
            let mut hz = *m as f32 / 1000.0;
            let resp = unit_drag_value(ui, &mut hz, 0.1..=240.0, 0.1, " Hz");
            if resp.changed() {
                *m = (hz * 1000.0).round().max(1.0) as u32;
            }
            *any_dragged |= resp.dragged();
        }
        _ => {}
    }
}

/// Inline editor for the `Custom` offset cycle: a row of semitone `DragValue`s
/// plus add/remove buttons, capped at [`MAX_ARP_OFFSETS`].
fn edit_arp_offsets(
    ui: &mut egui::Ui,
    custom: &mut synth_sequencer::ArpOffsets,
    any_dragged: &mut bool,
) {
    // Wrap so a full 16-offset cycle fits any card width instead of forcing the
    // panel/node wider (a `labeled_row` is a single non-wrapping horizontal).
    ui.horizontal_wrapped(|ui| {
        dim_label(ui, "Offsets");
        let len = custom.len();
        for i in 0..len {
            let resp = ui.add(
                egui::DragValue::new(&mut custom.values[i])
                    .range(-48..=48)
                    .speed(0.2),
            );
            *any_dragged |= resp.dragged();
        }
        if len < MAX_ARP_OFFSETS && ui.small_button("+").on_hover_text("Add step").clicked() {
            custom.values[len] = 0;
            custom.len += 1;
        }
        if len > 0 && ui.small_button("−").on_hover_text("Remove step").clicked() {
            custom.len -= 1;
            // Zero the vacated slot so the beyond-`len` bytes stay 0 — keeps the
            // derived `PartialEq` in step with the live offsets (no stale-byte
            // false "dirty" diff after add→edit→remove).
            custom.values[custom.len as usize] = 0;
        }
    });
}

pub(crate) fn edit_humanize(ui: &mut egui::Ui, h: &mut Humanize, any_dragged: &mut bool) {
    ui.horizontal(|ui| {
        knob_normalized(ui, "Vel ±", &mut h.velocity, any_dragged)
            .on_hover_text("Random velocity variation per note (0..1).");
        knob_normalized(ui, "Gate ±", &mut h.gate, any_dragged)
            .on_hover_text("Random note-length variation per note (0..1).");
    });
    labeled_row(ui, "Seed", |ui| {
        seed_reroll(ui, &mut h.seed, any_dragged);
    })
    .response
    .on_hover_text("Seed for the reproducible variation — reroll for a fresh feel.");
}

const ARP_MODES: [(ArpMode, &str); 8] = [
    (ArpMode::Up, "Up"),
    (ArpMode::Down, "Down"),
    (ArpMode::UpDown, "Up-Down"),
    (ArpMode::DownUp, "Down-Up"),
    (ArpMode::AsPlayed, "As played"),
    (ArpMode::Random, "Random"),
    (ArpMode::Chord, "Chord"),
    (ArpMode::Custom, "Custom"),
];

const ARP_RATES: [(ArpRate, &str); 12] = [
    (ArpRate::Quarter, "1/4"),
    (ArpRate::QuarterDotted, "1/4."),
    (ArpRate::QuarterTriplet, "1/4T"),
    (ArpRate::Eighth, "1/8"),
    (ArpRate::EighthDotted, "1/8."),
    (ArpRate::EighthTriplet, "1/8T"),
    (ArpRate::Sixteenth, "1/16"),
    (ArpRate::SixteenthDotted, "1/16."),
    (ArpRate::SixteenthTriplet, "1/16T"),
    (ArpRate::ThirtySecond, "1/32"),
    (ArpRate::ThirtySecondDotted, "1/32."),
    (ArpRate::ThirtySecondTriplet, "1/32T"),
];

const ARP_VELS: [(ArpVelocity, &str); 3] = [
    (ArpVelocity::AsPlayed, "As played"),
    (ArpVelocity::RampUp, "Ramp up"),
    (ArpVelocity::RampDown, "Ramp down"),
];

const SCALES: [(ScaleMask, &str); 6] = [
    (ScaleMask::MAJOR, "Major"),
    (ScaleMask::NATURAL_MINOR, "Natural minor"),
    (ScaleMask::HARMONIC_MINOR, "Harmonic minor"),
    (ScaleMask::PENTATONIC_MAJOR, "Pentatonic major"),
    (ScaleMask::PENTATONIC_MINOR, "Pentatonic minor"),
    (ScaleMask::CHROMATIC, "Chromatic"),
];

const CHORD_PRESETS: [(&[i8], &str); 3] = [
    (&[0, 4, 7], "Major"),
    (&[0, 3, 7], "Minor"),
    (&[0, 4, 7, 10], "Dominant 7"),
];

const CHORD_INTERVAL_CHOICES: [(i8, &str); 6] = [
    (3, "min 3rd (+3)"),
    (4, "maj 3rd (+4)"),
    (7, "5th (+7)"),
    (10, "min 7th (+10)"),
    (11, "maj 7th (+11)"),
    (12, "octave (+12)"),
];

/// Rebuild a chord from `intervals`, carrying over its strum spread + direction.
/// `Chord::new` resets strum to zero, so every interval edit funnels through here
/// to preserve the strum settings.
fn rebuild_chord(intervals: &[i8], strum: SeqDuration, direction: StrumDirection) -> Chord {
    Chord::new(intervals).with_strum(strum, direction)
}

/// Name a chord by its interval set if it matches a preset, else "Custom".
fn chord_preset_name(c: &Chord) -> &'static str {
    CHORD_PRESETS
        .iter()
        .find(|(intervals, _)| c.intervals() == *intervals)
        .map_or("Custom", |(_, label)| label)
}

// ============================================================================
// Processor-kind helpers (shared with the Note Grid node bodies)
// ============================================================================

/// Per-kind accent colour, keyed to the same rack module-category meanings as
/// [`node_accent`](crate::gui::note_grid_view): `ScaleQuantize` shapes pitch like
/// a **filter** (cyan); `Chord` is a note **generator/source** (orange);
/// `Arpeggiator` turns held notes into a timed run like a **sequencer** (red);
/// `Humanize` is a **utility** modifier (grey).
pub(crate) fn processor_accent(proc: &NoteProcessor) -> Color32 {
    let t = theme();
    match proc {
        NoteProcessor::ScaleQuantize(_) => t.colors.accent_cyan,
        NoteProcessor::Chord(_) => t.colors.accent_orange,
        NoteProcessor::Arpeggiator(_) => t.colors.accent_red,
        NoteProcessor::Humanize(_) => t.colors.text_secondary,
    }
}

/// Name a scale mask if it matches a known preset, else "Custom".
fn scale_mask_name(mask: ScaleMask) -> &'static str {
    SCALES
        .iter()
        .find(|(m, _)| *m == mask)
        .map_or("Custom", |(_, label)| label)
}
