//! The Note FX rack inspector — a right-docked panel listing a pattern's
//! note-processor rack (NP6.1). Add / remove / configure the processors that
//! expand the pattern's source notes at playback time.
//!
//! The rack lives on the shared `Song` (`Pattern.processors`); the audio thread
//! re-reads it each tick, so edits go through `song.write()` + the undo manager,
//! never an `EngineCommand`.
//!
//! ## Undo coalescing
//!
//! A processor's config is edited as a `cfg` working copy; when it differs from
//! the stored config the change is applied live (no undo) and a pre-edit baseline
//! is captured in [`SequencerViewState::note_fx_edit_drag_start`]. The single
//! `SetNoteProcessorConfig` undo entry is pushed once **no widget is being
//! dragged** — so a knob/slider drag collapses to one entry, while a discrete
//! combo/toggle change (never "dragged") captures and finalizes in the same
//! frame. Add/remove are discrete and push their own undo immediately.

use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
use parking_lot::RwLock;
use synth_core::NormalizedValue;
use synth_sequencer::{
    ArpMode, ArpRate, ArpVelocity, Arpeggiator, Chord, Duration as SeqDuration, Humanize,
    MAX_ARP_OFFSETS, NoteName, NoteProcessor, PatternId, PitchClass, ScaleMask, ScaleQuantize,
    Song, StrumDirection,
};

use crate::gui::theme::theme;
use crate::gui::widgets::{Knob, ModuleFrame, dim_label, draw_module_header, enum_combo};
use crate::undo::UndoAction;

use super::SequencerViewState;

/// A rack edit deferred until after the snapshot has been drawn, so we never
/// mutate the rack while iterating the cloned snapshot.
enum RackEdit {
    Add(NoteProcessor),
    Remove(usize),
    /// Live config edit at `index` — applied immediately; undo is coalesced
    /// separately (see the module docs).
    Set {
        index: usize,
        cfg: NoteProcessor,
    },
    /// Bake the rack (+ per-note ornaments) into plain notes and clear it.
    Freeze,
}

/// Draw the note-processor rack for `pattern_id`. Reads a cloned snapshot of the
/// rack, renders one editable card per processor, applies at most one edit, and
/// finalizes a coalesced config-edit undo entry when no widget is being dragged.
pub(crate) fn draw_note_fx_panel(
    ui: &mut egui::Ui,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    undo_manager: &mut crate::undo::UndoManager,
    pattern_id: PatternId,
) {
    let t = theme();

    // Header row: title + close.
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("NOTE FX")
                .strong()
                .color(t.colors.accent_purple),
        );
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

    let mut edit: Option<RackEdit> = None;
    // Set true by any continuous widget (knob/slider/DragValue) currently dragged;
    // gates undo finalization so a drag yields one entry, not one per frame.
    let mut any_dragged = false;

    // Snapshot the rack (and whether any note has an ornament) so the lock is
    // held only briefly.
    let Some((rack, has_ornament)) = song.try_read().and_then(|s| {
        s.pattern(pattern_id).map(|p| {
            (
                p.processors().to_vec(),
                // Match the freeze condition: a count < 2 ornament is a no-op the
                // bake skips, so it must not enable the button.
                p.notes()
                    .iter()
                    .any(|n| n.ornament.is_some_and(|o| o.count >= 2)),
            )
        })
    }) else {
        dim_label(ui, "Pattern unavailable…");
        return;
    };

    // Freeze: bake the rack + per-note ornaments into plain notes. Enabled when
    // there is anything to bake (a rack or any ornament).
    ui.add_enabled_ui(!rack.is_empty() || has_ornament, |ui| {
        ui.menu_button("Freeze", |ui| {
            ui.label(
                RichText::new(
                    "Bakes the processor rack and per-note ornaments into plain notes, then clears the rack. Undoable.",
                )
                .color(t.colors.text_dim),
            );
            if ui.button("Freeze now").clicked() {
                edit = Some(RackEdit::Freeze);
                ui.close();
            }
        });
    });
    ui.add_space(4.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        if rack.is_empty() {
            ui.label(
                RichText::new("No note processors. Add one below to arpeggiate, build chords, snap to a scale, or humanize this pattern.")
                    .color(t.colors.text_dim),
            );
        }

        for (index, proc) in rack.iter().enumerate() {
            let accent = processor_accent(proc);
            let frame = ModuleFrame::new(accent)
                .inner_margin(6.0)
                .build(&ui.global_style());
            frame.show(ui, |ui| {
                ui.push_id(index, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width());
                        draw_module_header(ui, accent, processor_name(proc), None, false, |ui| {
                            if ui
                                .add(
                                    egui::Button::new("✖")
                                        .frame(false)
                                        .min_size(egui::vec2(18.0, 18.0)),
                                )
                                .on_hover_text("Remove processor")
                                .clicked()
                            {
                                edit = Some(RackEdit::Remove(index));
                            }
                        });

                        // Render widgets into a working copy; a diff against the
                        // stored config drives the live apply + undo capture.
                        let mut cfg = proc.clone();
                        match &mut cfg {
                            NoteProcessor::ScaleQuantize(q) => edit_scale_quantize(ui, index, q),
                            NoteProcessor::Chord(c) => edit_chord(ui, index, c, &mut any_dragged),
                            NoteProcessor::Arpeggiator(a) => {
                                edit_arpeggiator(ui, index, a, &mut any_dragged);
                            }
                            NoteProcessor::Humanize(h) => edit_humanize(ui, h, &mut any_dragged),
                        }
                        if cfg != *proc {
                            // Capture the pre-edit baseline for this (pattern,
                            // index). Replace any stale baseline left by an
                            // interrupted gesture on a different processor/pattern
                            // (e.g. the pattern was switched or removed mid-edit),
                            // so the current edit's undo is never keyed to the
                            // wrong slot.
                            let matches = view_state
                                .note_fx_edit_drag_start
                                .as_ref()
                                .is_some_and(|(p, i, _)| *p == pattern_id && *i == index);
                            if !matches {
                                view_state.note_fx_edit_drag_start =
                                    Some((pattern_id, index, proc.clone()));
                            }
                            edit = Some(RackEdit::Set { index, cfg });
                        }
                    });
                });
            });
            ui.add_space(4.0);
        }

        // ── Add menu: only stages not already present (≤1 per stage) ──
        let present: Vec<u8> = rack.iter().map(NoteProcessor::chain_stage).collect();
        let candidates = default_processors();
        let all_present = candidates
            .iter()
            .all(|p| present.contains(&p.chain_stage()));
        ui.add_enabled_ui(!all_present, |ui| {
            ui.menu_button("+ Add", |ui| {
                for candidate in candidates {
                    if present.contains(&candidate.chain_stage()) {
                        continue;
                    }
                    if ui.button(processor_name(&candidate)).clicked() {
                        edit = Some(RackEdit::Add(candidate));
                        ui.close();
                    }
                }
            });
        });
    });

    // Apply the deferred edit. Add/Remove push undo immediately (discrete);
    // Set applies live and leaves undo to the finalize step below.
    match edit {
        Some(RackEdit::Add(proc)) => {
            let mut index = None;
            {
                let mut song_w = song.write();
                if let Some(pattern) = song_w.pattern_mut(pattern_id) {
                    index = Some(pattern.add_processor(proc.clone()));
                }
            }
            if let Some(index) = index {
                undo_manager.push(UndoAction::AddNoteProcessor {
                    pattern_id,
                    index,
                    processor: proc,
                });
            }
        }
        Some(RackEdit::Remove(index)) => {
            let mut removed = None;
            {
                let mut song_w = song.write();
                if let Some(pattern) = song_w.pattern_mut(pattern_id) {
                    removed = pattern.remove_processor(index);
                }
            }
            if let Some(processor) = removed {
                undo_manager.push(UndoAction::RemoveNoteProcessor {
                    pattern_id,
                    index,
                    processor,
                });
            }
        }
        Some(RackEdit::Set { index, cfg }) => {
            let mut song_w = song.write();
            if let Some(pattern) = song_w.pattern_mut(pattern_id) {
                pattern.set_processor(index, cfg);
            }
        }
        Some(RackEdit::Freeze) => {
            let mut before = None;
            {
                let mut song_w = song.write();
                let bpm = song_w.tempo_at(synth_sequencer::Tick(0));
                if let Some(pattern) = song_w.pattern_mut(pattern_id) {
                    before = Some(pattern.clone());
                    pattern.freeze_processors(bpm);
                }
            }
            if let Some(before) = before {
                undo_manager.push(UndoAction::FreezePattern { pattern_id, before });
            }
        }
        None => {}
    }

    // Finalize a coalesced config edit once no widget is being dragged. Clear the
    // baseline whenever the lock is acquired (the gesture is over) — even if the
    // processor is gone (pattern removed/closed mid-edit) — so a stale baseline
    // can never wedge future edits. A transient lock miss keeps it for a retry.
    if !any_dragged
        && let Some((pid, idx, old)) = view_state.note_fx_edit_drag_start.clone()
        && let Some(song_r) = song.try_read()
    {
        let new = song_r
            .pattern(pid)
            .and_then(|p| p.processors().get(idx).cloned());
        drop(song_r);
        if let Some(new) = new
            && new != old
        {
            undo_manager.push(UndoAction::SetNoteProcessorConfig {
                pattern_id: pid,
                index: idx,
                old,
                new,
            });
        }
        view_state.note_fx_edit_drag_start = None;
    }
}

// ============================================================================
// Per-type editors
// ============================================================================

fn edit_scale_quantize(ui: &mut egui::Ui, index: usize, q: &mut ScaleQuantize) {
    ui.horizontal(|ui| {
        ui.label("Root");
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
            });
        ui.label("Scale");
        egui::ComboBox::from_id_salt((index, "scale"))
            .selected_text(scale_mask_name(q.mask))
            .show_ui(ui, |ui| {
                for (mask, label) in SCALES {
                    if ui.selectable_label(q.mask == mask, label).clicked() {
                        q.mask = mask;
                    }
                }
            });
    });
    // Custom 12-pill mask row — toggle individual pitch classes.
    ui.horizontal_wrapped(|ui| {
        for i in 0..12u8 {
            let on = q.mask.contains_interval(i);
            if ui
                .selectable_label(on, NoteName::from_midi(i).to_string())
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

fn edit_chord(ui: &mut egui::Ui, index: usize, c: &mut Chord, any_dragged: &mut bool) {
    ui.horizontal(|ui| {
        ui.label("Type");
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
            });
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
        let resp = ui.add(egui::Slider::new(&mut spread, 0..=480).text("Strum"));
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
            );
            if dir != c.direction() {
                let ivs: Vec<i8> = c.intervals().to_vec();
                *c = rebuild_chord(&ivs, c.strum(), dir);
            }
        });
    });
}

fn edit_arpeggiator(ui: &mut egui::Ui, index: usize, a: &mut Arpeggiator, any_dragged: &mut bool) {
    ui.horizontal(|ui| {
        ui.label("Mode");
        enum_combo(ui, (index, "mode"), &mut a.mode, &ARP_MODES);
        ui.label("Rate");
        arp_rate_widget(ui, index, a, any_dragged);
    });
    ui.horizontal(|ui| {
        ui.label("Octaves");
        let resp = ui.add(egui::DragValue::new(&mut a.octaves).range(1..=4));
        *any_dragged |= resp.dragged();
        ui.label("Vel");
        enum_combo(ui, (index, "vel"), &mut a.velocity, &ARP_VELS);
        ui.checkbox(&mut a.latch, "Latch");
        ui.checkbox(&mut a.legato, "Legato").on_hover_text(
            "Hold one envelope across the figure (glide between steps). Use a high Gate.",
        );
    });
    if a.mode == ArpMode::Custom {
        edit_arp_offsets(ui, &mut a.custom, any_dragged);
    }
    ui.horizontal(|ui| {
        knob_normalized(ui, "Gate", &mut a.gate, any_dragged);
        knob_normalized(ui, "Swing", &mut a.swing, any_dragged);
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
            let resp = ui.add(
                egui::DragValue::new(&mut hz)
                    .range(0.1..=240.0)
                    .speed(0.1)
                    .suffix(" Hz"),
            );
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
    ui.horizontal(|ui| {
        ui.label("Offsets");
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

fn edit_humanize(ui: &mut egui::Ui, h: &mut Humanize, any_dragged: &mut bool) {
    ui.horizontal(|ui| {
        knob_normalized(ui, "Vel ±", &mut h.velocity, any_dragged);
        knob_normalized(ui, "Gate ±", &mut h.gate, any_dragged);
    });
    ui.horizontal(|ui| {
        ui.label("Seed");
        let resp = ui.add(egui::DragValue::new(&mut h.seed).speed(1.0));
        *any_dragged |= resp.dragged();
        if ui.button("🎲").on_hover_text("Reroll seed").clicked() {
            // Golden-ratio step: a fresh, well-distributed seed each click.
            h.seed = h.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        }
    });
}

// ============================================================================
// Widget + label helpers
// ============================================================================

/// A 0–1 knob bound to a `NormalizedValue` field; ORs `any_dragged` while held.
fn knob_normalized(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut NormalizedValue,
    any_dragged: &mut bool,
) {
    let mut v = value.as_f32();
    let resp = Knob::new(&mut v, 0.0, 1.0).label(label).show(ui);
    *any_dragged |= resp.dragged();
    *value = NormalizedValue::new(v);
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
// Rack-level helpers
// ============================================================================

/// One default-configured instance of each processor kind, in chain order — the
/// "+ Add" menu source and the stage-coverage check.
fn default_processors() -> [NoteProcessor; 4] {
    [
        NoteProcessor::ScaleQuantize(ScaleQuantize::default()),
        NoteProcessor::Chord(Chord::default()),
        NoteProcessor::Arpeggiator(Arpeggiator::default()),
        NoteProcessor::Humanize(Humanize::default()),
    ]
}

/// Human-facing card title for a processor kind.
fn processor_name(proc: &NoteProcessor) -> &'static str {
    match proc {
        NoteProcessor::ScaleQuantize(_) => "Scale Quantize",
        NoteProcessor::Chord(_) => "Chord",
        NoteProcessor::Arpeggiator(_) => "Arpeggiator",
        NoteProcessor::Humanize(_) => "Humanize",
    }
}

/// Per-kind accent colour so the rack is scannable at a glance.
fn processor_accent(proc: &NoteProcessor) -> Color32 {
    let t = theme();
    match proc {
        NoteProcessor::ScaleQuantize(_) => t.colors.accent_cyan,
        NoteProcessor::Chord(_) => t.colors.accent_green,
        NoteProcessor::Arpeggiator(_) => t.colors.accent_purple,
        NoteProcessor::Humanize(_) => t.colors.accent_orange,
    }
}

/// Name a scale mask if it matches a known preset, else "Custom".
fn scale_mask_name(mask: ScaleMask) -> &'static str {
    SCALES
        .iter()
        .find(|(m, _)| *m == mask)
        .map_or("Custom", |(_, label)| label)
}
