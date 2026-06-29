//! Per-note ornament editor — the shared popup that edits a note's optional
//! timed-repeat ornament (flam / drag / ruff / roll / grace). Hosted by the
//! piano-roll selection inspector (and later the tracker ornament column); both
//! call [`draw_ornament_editor`], so there is one editing code path.

use std::sync::Arc;

use eframe::egui;
use parking_lot::RwLock;
use synth_core::{NormalizedValue, Semitones};
use synth_sequencer::{
    Duration as SeqDuration, MAX_ORNAMENT_HITS, NoteId, Ornament, OrnamentDynamics,
    OrnamentPlacement, OrnamentSpacing, PatternId, Song,
};

use super::SequencerViewState;
use crate::gui::widgets::{Knob, enum_combo, labeled_row};
use crate::undo::{UndoAction, UndoManager};

/// An in-progress ornament edit: which note, the pre-edit baseline (for one
/// coalesced undo entry per popup session), and the live working copy. Lives on
/// `SequencerViewState` while the popup is open.
#[derive(Clone)]
pub(crate) struct OrnamentEdit {
    pub pattern_id: PatternId,
    pub note_id: NoteId,
    pub before: Option<Ornament>,
    pub current: Option<Ornament>,
}

/// Draw the ornament editor body for `orn` (`None` = no ornament). Returns
/// whether the value changed this frame. Preset buttons set a canonical figure;
/// "None" clears it; the detail controls appear only when an ornament is present.
pub(crate) fn draw_ornament_editor(ui: &mut egui::Ui, orn: &mut Option<Ornament>) -> bool {
    let before = *orn;

    ui.horizontal(|ui| {
        if ui
            .selectable_label(orn.is_none(), "None")
            .on_hover_text("No ornament")
            .clicked()
        {
            *orn = None;
        }
        for (preset, label) in PRESETS {
            if ui.button(label).clicked() {
                *orn = Some(preset());
            }
        }
    });

    if let Some(o) = orn.as_mut() {
        ui.separator();
        labeled_row(ui, "Hits", |ui| {
            // Min 2: a count of 1 is a documented no-op (use "None" to clear).
            ui.add(egui::DragValue::new(&mut o.count).range(2..=MAX_ORNAMENT_HITS));
            ui.label("Spacing");
            let mut spacing = o.spacing.0;
            ui.add(
                egui::DragValue::new(&mut spacing)
                    .range(1..=960)
                    .suffix(" t"),
            );
            o.spacing = SeqDuration(spacing);
        });
        labeled_row(ui, "Curve", |ui| {
            enum_combo(
                ui,
                "orn_curve",
                &mut o.spacing_curve,
                &[
                    (OrnamentSpacing::Even, "Even"),
                    (OrnamentSpacing::Accelerate, "Accelerate"),
                    (OrnamentSpacing::Decelerate, "Decelerate"),
                ],
            );
            ui.label("Dynamics");
            enum_combo(
                ui,
                "orn_dyn",
                &mut o.dynamics,
                &[
                    (OrnamentDynamics::Flat, "Flat"),
                    (OrnamentDynamics::Crescendo, "Crescendo"),
                    (OrnamentDynamics::Decrescendo, "Decrescendo"),
                ],
            );
        });
        labeled_row(ui, "Placement", |ui| {
            enum_combo(
                ui,
                "orn_place",
                &mut o.placement,
                &[
                    (OrnamentPlacement::LeadIn, "Lead-in"),
                    (OrnamentPlacement::OnBeat, "On beat"),
                ],
            );
        });
        labeled_row(ui, "Pitch offset", |ui| {
            let mut po = o.pitch_offset.as_f32();
            ui.add(
                egui::DragValue::new(&mut po)
                    .range(-24.0..=24.0)
                    .suffix(" st"),
            );
            o.pitch_offset = Semitones::new(po);

            let mut gg = o.grace_gate.as_f32();
            Knob::new(&mut gg, 0.0, 1.0).label("Grace gate").show(ui);
            o.grace_gate = NormalizedValue::new(gg);
        });
    }

    *orn != before
}

/// The per-note ornament editor popup window, shared by the piano-roll selection
/// inspector and the tracker ornament column. Shown while
/// `view_state.editing_ornament` is set; applies edits live to the note and
/// pushes one coalesced `SetNoteOrnament` undo entry when the window is closed.
pub(crate) fn draw_ornament_popup(
    ui: &mut egui::Ui,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    undo_manager: &mut UndoManager,
) {
    let mut finalize = false;
    if let Some(edit) = view_state.editing_ornament.as_mut() {
        let mut open = true;
        let mut changed = false;
        egui::Window::new("Ornament")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                changed = draw_ornament_editor(ui, &mut edit.current);
            });
        if changed {
            let mut song_w = song.write();
            if let Some(note) = song_w
                .pattern_mut(edit.pattern_id)
                .and_then(|p| p.note_mut(edit.note_id))
            {
                note.ornament = edit.current;
            }
        }
        finalize = !open;
    }
    if finalize
        && let Some(edit) = view_state.editing_ornament.take()
        && edit.before != edit.current
    {
        undo_manager.push(UndoAction::SetNoteOrnament {
            pattern_id: edit.pattern_id,
            note_id: edit.note_id,
            old: edit.before,
            new: edit.current,
        });
    }
}

/// Drum-rudiment classification of an ornament, derived from its shape. The
/// single source of truth shared by [`ornament_tag`] and [`ornament_kind_name`].
enum Rudiment {
    /// `count <= 1` — effectively no ornament.
    NoOp,
    /// Melodic grace note (a different pitch, count 2).
    Grace,
    Flam,
    Drag,
    Ruff,
    Roll,
}

fn classify(o: Ornament) -> Rudiment {
    if o.pitch_offset.as_f32() != 0.0 && o.count == 2 {
        Rudiment::Grace
    } else {
        match o.count {
            0 | 1 => Rudiment::NoOp,
            2 => Rudiment::Flam,
            3 => Rudiment::Drag,
            4 => Rudiment::Ruff,
            _ => Rudiment::Roll,
        }
    }
}

/// A compact tracker-cell tag for an ornament (e.g. "Fl2", "Ro12", "Gr+2").
pub(crate) fn ornament_tag(o: Ornament) -> String {
    match classify(o) {
        Rudiment::NoOp => "—".to_string(),
        Rudiment::Grace => format!("Gr{:+}", o.pitch_offset.as_f32() as i32),
        Rudiment::Flam => format!("Fl{}", o.count),
        Rudiment::Drag => format!("Dr{}", o.count),
        Rudiment::Ruff => format!("Rf{}", o.count),
        Rudiment::Roll => format!("Ro{}", o.count),
    }
}

fn spacing_name(s: OrnamentSpacing) -> &'static str {
    match s {
        OrnamentSpacing::Even => "Even",
        OrnamentSpacing::Accelerate => "Accelerate",
        OrnamentSpacing::Decelerate => "Decelerate",
    }
}

fn dynamics_name(d: OrnamentDynamics) -> &'static str {
    match d {
        OrnamentDynamics::Flat => "Flat",
        OrnamentDynamics::Crescendo => "Crescendo",
        OrnamentDynamics::Decrescendo => "Decrescendo",
    }
}

fn placement_name(p: OrnamentPlacement) -> &'static str {
    match p {
        OrnamentPlacement::LeadIn => "Lead-in",
        OrnamentPlacement::OnBeat => "On beat",
    }
}

/// A multi-line description of an ornament's full config, for a hover tooltip.
pub(crate) fn ornament_detail(o: Ornament) -> String {
    format!(
        "{} — {} hits, spacing {} ticks\ncurve {}, dynamics {}, {}\npitch offset {:+} st, grace gate {:.0}%",
        ornament_kind_name(o),
        o.count,
        o.spacing.0,
        spacing_name(o.spacing_curve),
        dynamics_name(o.dynamics),
        placement_name(o.placement),
        o.pitch_offset.as_f32() as i32,
        o.grace_gate.as_f32() * 100.0,
    )
}

/// A one-line read-only summary of a note's ornament for the inspector.
pub(crate) fn ornament_summary(orn: Option<Ornament>) -> String {
    match orn {
        None => "none".to_string(),
        Some(o) => format!("{} ({} hits)", ornament_kind_name(o), o.count),
    }
}

/// Drum-rudiment name for an ornament (matches the preset convention).
fn ornament_kind_name(o: Ornament) -> &'static str {
    match classify(o) {
        Rudiment::NoOp => "—",
        Rudiment::Grace => "Grace",
        Rudiment::Flam => "Flam",
        Rudiment::Drag => "Drag",
        Rudiment::Ruff => "Ruff",
        Rudiment::Roll => "Roll",
    }
}

/// A named ornament preset: a constructor plus its button label.
type Preset = (fn() -> Ornament, &'static str);

const PRESETS: [Preset; 5] = [
    (preset_flam, "Flam"),
    (preset_drag, "Drag"),
    (preset_ruff, "Ruff"),
    (preset_roll, "Roll"),
    (preset_grace, "Grace"),
];

/// Flam: the default ornament (one quiet grace crushed into the main hit).
fn preset_flam() -> Ornament {
    Ornament::default()
}

fn preset_drag() -> Ornament {
    Ornament {
        count: 3,
        ..Ornament::default()
    }
}

fn preset_ruff() -> Ornament {
    Ornament {
        count: 4,
        ..Ornament::default()
    }
}

fn preset_roll() -> Ornament {
    Ornament {
        count: 8,
        spacing: SeqDuration(60),
        dynamics: OrnamentDynamics::Flat,
        ..Ornament::default()
    }
}

/// Acciaccatura: a single grace a tone above, crushed before the beat.
fn preset_grace() -> Ornament {
    Ornament {
        count: 2,
        pitch_offset: Semitones::new(2.0),
        ..Ornament::default()
    }
}
