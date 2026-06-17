//! The Note FX rack inspector — a right-docked panel listing a pattern's
//! note-processor rack (NP6.1). Add / remove / (later) configure the processors
//! that expand the pattern's source notes at playback time.
//!
//! The rack lives on the shared `Song` (`Pattern.processors`); the audio thread
//! re-reads it each tick, so edits go through `song.write()` + the undo manager,
//! never an `EngineCommand`. This first slice covers the panel, add / remove, and
//! undo; per-processor parameter widgets and freeze land in follow-ups.

use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
use parking_lot::RwLock;
use synth_sequencer::{
    Arpeggiator, Chord, Humanize, NoteName, NoteProcessor, PatternId, ScaleMask, ScaleQuantize,
    Song,
};

use crate::gui::theme::theme;
use crate::gui::widgets::{ModuleFrame, draw_module_header};
use crate::undo::UndoAction;

use super::SequencerViewState;

/// A rack edit deferred until after the snapshot has been drawn, so we never
/// mutate the rack while iterating the cloned snapshot.
enum RackEdit {
    Add(NoteProcessor),
    Remove(usize),
}

/// Draw the note-processor rack for `pattern_id`. Reads a cloned snapshot of the
/// rack, renders one card per processor, and applies at most one add / remove
/// edit (with undo) after drawing.
pub(super) fn draw_note_fx_panel(
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
                .small_button("✕")
                .on_hover_text("Hide the Note FX rack")
                .clicked()
            {
                view_state.note_fx_panel_open = false;
            }
        });
    });
    ui.separator();

    // Snapshot the rack so the lock is held only briefly.
    let Some(rack) = song
        .try_read()
        .and_then(|s| s.pattern(pattern_id).map(|p| p.processors().to_vec()))
    else {
        ui.label(RichText::new("Pattern unavailable…").color(t.colors.text_dim));
        return;
    };

    let mut edit: Option<RackEdit> = None;

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
                        draw_module_header(ui, accent, processor_name(proc), None, |ui| {
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
                        ui.label(
                            RichText::new(processor_summary(proc))
                                .color(t.colors.text_secondary),
                        );
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

    // Apply the deferred edit under a write lock + record undo.
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
        None => {}
    }
}

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

/// One-line read-only summary of a processor's config (a placeholder until the
/// per-kind parameter widgets land in the next slice).
fn processor_summary(proc: &NoteProcessor) -> String {
    match proc {
        NoteProcessor::ScaleQuantize(q) => {
            format!(
                "Root {} · {}",
                NoteName::from_midi(q.root.as_u8()),
                scale_mask_name(q.mask)
            )
        }
        NoteProcessor::Chord(c) => {
            let n = c.intervals().len();
            format!("{n} tone{}", if n == 1 { "" } else { "s" })
        }
        NoteProcessor::Arpeggiator(a) => {
            format!(
                "{:?} · {:?} · {} oct",
                a.mode,
                a.rate,
                a.octaves.clamp(1, 4)
            )
        }
        NoteProcessor::Humanize(h) => {
            format!(
                "vel ±{:.0}% · gate ±{:.0}%",
                h.velocity.as_f32() * 100.0,
                h.gate.as_f32() * 100.0
            )
        }
    }
}

/// Name a scale mask if it matches a known preset, else "Custom".
fn scale_mask_name(mask: ScaleMask) -> &'static str {
    if mask == ScaleMask::MAJOR {
        "Major"
    } else if mask == ScaleMask::NATURAL_MINOR {
        "Natural minor"
    } else if mask == ScaleMask::HARMONIC_MINOR {
        "Harmonic minor"
    } else if mask == ScaleMask::PENTATONIC_MAJOR {
        "Pentatonic major"
    } else if mask == ScaleMask::PENTATONIC_MINOR {
        "Pentatonic minor"
    } else if mask == ScaleMask::CHROMATIC {
        "Chromatic"
    } else {
        "Custom"
    }
}
