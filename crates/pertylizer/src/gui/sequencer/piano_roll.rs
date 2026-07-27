//! Piano-roll view: note grid, selection inspector, automation-target
//! selector, per-pattern instrument transport, and all mouse interaction
//! (draw/select/move/resize/delete notes, velocity + expression editing).
//!
//! Snapshot DTOs (`PianoRollData` et al.) and the tick helpers
//! (`snap_to_step`, `quantize_tick`) live in the parent module.

use super::*;
use crate::gui::widgets::{expose, icon_button, selectable_toggle, solo_toggle, submenu_button};

/// Ordered automation targets shown as stacked zones: every existing lane,
/// plus the edit-selected target if it has no lane yet (so a brand-new lane
/// still gets a zone to draw into). Lane order first, the new target last.
pub(super) fn displayed_automation_targets(
    data: &PianoRollData,
    view_state: &SequencerViewState,
) -> Vec<AutomationTarget> {
    let mut targets: Vec<AutomationTarget> = data
        .automation_lanes
        .iter()
        .map(|l| l.target.clone())
        .collect();
    if let Some(sel) = &view_state.selected_automation
        && !targets.iter().any(|t| t == sel)
    {
        targets.push(sel.clone());
    }
    targets
}

/// Collect piano roll data from song (short read-lock, then release).
pub(crate) fn collect_piano_roll_data(
    song: &Arc<synth_sequencer::SharedSong>,
    pattern_id: PatternId,
) -> Option<PianoRollData> {
    let song = song.try_read()?;
    let pattern = song.pattern(pattern_id)?;

    let mut pitch_min = Pitch::MAX;
    let mut pitch_max = Pitch::MIN;

    let notes: Vec<PianoRollNote> = pattern
        .notes()
        .iter()
        .map(|n| {
            if n.pitch < pitch_min {
                pitch_min = n.pitch;
            }
            if n.pitch > pitch_max {
                pitch_max = n.pitch;
            }
            PianoRollNote {
                note_id: n.id,
                pitch: n.pitch,
                start_tick: n.start,
                end_tick: n.end(),
                velocity: n.velocity,
                legato: n.legato,
                glide: n.glide,
                expression: n.expression,
                note_graph: n.note_graph,
                ornament: n.ornament,
                lane: n.lane,
            }
        })
        .collect();

    // Default range if no notes
    if pitch_min > pitch_max {
        pitch_min = Pitch::new(48).unwrap_or(Pitch::MIDDLE_C); // C3
        pitch_max = Pitch::new(72).unwrap_or(Pitch::MIDDLE_C); // C5
    }

    let automation_lanes: Vec<AutomationLaneSnapshot> = pattern
        .automation
        .iter()
        .map(|lane| AutomationLaneSnapshot {
            target: lane.target.clone(),
            points: lane
                .points()
                .iter()
                .map(|p| AutomationPointSnapshot {
                    tick: p.tick,
                    value: p.value,
                    curve: p.curve,
                })
                .collect(),
        })
        .collect();

    let time_sig = song.default_time_signature;

    // Collect the distinct instruments this pattern's placements route through
    // (one per track). Used to show which instrument(s) actually play the
    // pattern. (Per-note instrument is no longer consulted at playback.)
    let mut track_overrides: Vec<InstrumentId> = Vec::new();
    for placement in song.arrangement() {
        if placement.pattern_id == pattern_id
            && let Some(track) = song.track(placement.track_id)
            && !track_overrides.contains(&track.instrument)
        {
            track_overrides.push(track.instrument);
        }
    }
    // Every track (id, name) — the cross-track lane targets in the picker.
    let all_tracks: Vec<(TrackId, String)> =
        song.tracks().map(|t| (t.id, t.name.clone())).collect();

    Some(PianoRollData {
        pattern_name: if pattern.name.is_empty() {
            format!("Pattern {}", pattern_id.0)
        } else {
            pattern.name.clone()
        },
        pattern_description: pattern.description.clone(),
        pattern_id,
        length_ticks: pattern.length,
        ticks_per_row: song.row_resolution.ticks_per_row.as_u16(),
        notes,
        pitch_min,
        pitch_max,
        automation_lanes,
        time_sig,
        track_overrides,
        all_tracks,
    })
}

/// Find the note at the given position, returning its ID and which zone was hit.
#[allow(clippy::too_many_arguments)]
fn note_at_pos(
    notes: &[PianoRollNote],
    pos: Pos2,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
    pitch_to_y: &dyn Fn(Pitch) -> f32,
    length_ticks: SeqDuration,
    view_pitch_min: Pitch,
    view_pitch_max: Pitch,
    note_row_height: f32,
) -> Option<(NoteId, HitZone)> {
    // Iterate in reverse so top-most (last drawn) notes are checked first
    for note in notes.iter().rev() {
        if note.pitch < view_pitch_min || note.pitch > view_pitch_max {
            continue;
        }

        let y = pitch_to_y(note.pitch);
        let x_start = tick_to_x(note.start_tick);
        let x_end = match note.end_tick {
            Some(end) => tick_to_x(end),
            None => tick_to_x(length_ticks.as_pattern_tick()),
        };
        let note_width = (x_end - x_start).max(3.0);

        let note_rect = Rect::from_min_size(
            Pos2::new(x_start, y + 1.0),
            Vec2::new(note_width, note_row_height - 2.0),
        );

        // Expand tiny notes so they're easier to click
        let hit_rect = if note_width < RESIZE_GRAB_ZONE {
            note_rect.expand2(Vec2::new(2.0, 1.0))
        } else {
            note_rect
        };

        if hit_rect.contains(pos) {
            // Proportional grab zone: at most 30% of note width, so short notes remain movable
            let grab_zone = RESIZE_GRAB_ZONE.min(note_width * 0.3);
            let zone = if pos.x >= note_rect.max.x - grab_zone {
                HitZone::RightEdge
            } else {
                HitZone::Body
            };
            return Some((note.note_id, zone));
        }
    }
    None
}

/// Check if a note already exists at the given tick and pitch.
fn has_note_at(notes: &[PianoRollNote], tick: PatternTick, pitch: Pitch) -> bool {
    notes.iter().any(|n| {
        n.pitch == pitch
            && n.start_tick <= tick
            && n.end_tick.unwrap_or(PatternTick(u32::MAX)) > tick
    })
}

/// Horizontal pick radius (points) for hitting a note's velocity bar.
const VELOCITY_HIT_PX: f32 = 15.0;

/// The velocity-edit zone rect, directly below the note grid.
fn velocity_zone_rect(grid_rect: Rect) -> Rect {
    Rect::from_min_size(
        Pos2::new(grid_rect.min.x, grid_rect.max.y),
        Vec2::new(grid_rect.width(), VELOCITY_ZONE_HEIGHT),
    )
}

/// Map a pointer Y inside the velocity zone (top = `vel_zone_top`) to a
/// normalized velocity — top ≈ 1.0, bottom ≈ 0. Shared by the velocity click
/// and drag paths so they stay in lockstep.
fn velocity_from_pos_y(pos_y: f32, vel_zone_top: f32) -> f32 {
    (1.0 - (pos_y - vel_zone_top) / VELOCITY_ZONE_HEIGHT).clamp(0.01, 1.0)
}

/// The note whose velocity bar is nearest the pointer X, if within
/// [`VELOCITY_HIT_PX`]. Bars are drawn at each note's start-tick x, so distance
/// is measured there — matching the bar rendering and both edit paths.
fn nearest_velocity_note<'a>(
    notes: &'a [PianoRollNote],
    pos_x: f32,
    tick_to_x: &dyn Fn(PatternTick) -> f32,
) -> Option<&'a PianoRollNote> {
    notes
        .iter()
        .map(|n| (n, (tick_to_x(n.start_tick) - pos_x).abs()))
        .filter(|(_, dist)| *dist < VELOCITY_HIT_PX)
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(n, _)| n)
}

/// Selection inspector row: shows pitch / start / length of the selected notes
/// (or "—" when they differ) and an editable velocity DragValue that batches an
/// undo entry on drag/focus release.
/// Default per-note vibrato installed when the inspector's Vibrato toggle is enabled.
fn default_inspector_vibrato() -> Vibrato {
    Vibrato {
        depth: Semitones::new(0.3),
        rate: Hertz::new(5.5),
        delay: Milliseconds::new(0.0),
        shape: VibratoShape::Sine,
    }
}

/// Apply a per-note expression edit to the selected notes (preserving each
/// note's *other* fields) and push one `SetExpressionBatch` undo entry. For
/// discrete (toggle) edits.
fn apply_expression_edit(
    song: &Arc<synth_sequencer::SharedSong>,
    undo: &mut crate::undo::UndoManager,
    pid: PatternId,
    selected: &[&PianoRollNote],
    edit: impl Fn(&mut NoteExpression),
) {
    let mut changes: Vec<(NoteId, Option<NoteExpression>, Option<NoteExpression>)> = Vec::new();
    {
        let mut song_w = song.write();
        if let Some(pattern) = song_w.pattern_mut(pid) {
            for n in selected {
                let old = n.expression;
                let mut e = old.unwrap_or_default();
                edit(&mut e);
                // Collapse an all-default block back to None (no pointless storage/dot).
                let new = e.normalized();
                if new != old {
                    pattern.set_note_expression(n.note_id, new);
                    changes.push((n.note_id, old, new));
                }
            }
        }
    }
    if !changes.is_empty() {
        undo.push(crate::undo::UndoAction::SetExpressionBatch {
            pattern_id: pid,
            changes,
        });
    }
}

/// Live-apply a per-note expression edit (no undo) — used while an expression
/// DragValue is dragging; the drag collapses into one undo entry on release via
/// [`finish_expression_drag`]. Preserves each note's other fields.
fn live_expression_edit(
    song: &Arc<synth_sequencer::SharedSong>,
    pid: PatternId,
    selected: &[&PianoRollNote],
    edit: impl Fn(&mut NoteExpression),
) {
    let mut song_w = song.write();
    if let Some(pattern) = song_w.pattern_mut(pid) {
        for n in selected {
            let mut e = n.expression.unwrap_or_default();
            edit(&mut e);
            // Collapse an all-default block back to None (no pointless storage/dot).
            pattern.set_note_expression(n.note_id, e.normalized());
        }
    }
}

/// On expression-DragValue release, diff the pre-drag snapshot against the now
/// current pattern state and push one `SetExpressionBatch` undo entry.
fn finish_expression_drag(
    song: &Arc<synth_sequencer::SharedSong>,
    undo: &mut crate::undo::UndoManager,
    pid: PatternId,
    before: Vec<(NoteId, Option<NoteExpression>)>,
) {
    let mut changes = Vec::new();
    {
        let song_r = song.read();
        if let Some(pattern) = song_r.pattern(pid) {
            for (id, old) in before {
                let new = pattern.note(id).and_then(|n| n.expression);
                if new != old {
                    changes.push((id, old, new));
                }
            }
        }
    }
    if !changes.is_empty() {
        undo.push(crate::undo::UndoAction::SetExpressionBatch {
            pattern_id: pid,
            changes,
        });
    }
}

fn draw_piano_roll_selection_inspector(
    ui: &mut egui::Ui,
    data: &PianoRollData,
    view_state: &mut SequencerViewState,
    song: &Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut crate::undo::UndoManager,
) {
    let t = theme();
    if !view_state.selected_notes.is_empty() {
        let selected: Vec<&PianoRollNote> = data
            .notes
            .iter()
            .filter(|n| view_state.selected_notes.contains(&n.note_id))
            .collect();
        if !selected.is_empty() {
            let first = selected[0];
            let pitches_equal = selected.iter().all(|n| n.pitch == first.pitch);
            let starts_equal = selected.iter().all(|n| n.start_tick == first.start_tick);
            let durations: Vec<Option<u32>> = selected
                .iter()
                .map(|n| n.end_tick.map(|e| e.0 - n.start_tick.0))
                .collect();
            let lengths_equal = durations.iter().all(|d| *d == durations[0]);
            let velocities_equal = selected
                .iter()
                .all(|n| (n.velocity.as_f32() - first.velocity.as_f32()).abs() < f32::EPSILON);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(
                    RichText::new(format!("Selection ({})", selected.len()))
                        .color(t.colors.accent_yellow)
                        .strong(),
                );
                ui.separator();

                // Pitch
                caption(ui, "Pitch", CaptionTone::Dim);
                let pitch_text = if pitches_equal {
                    let midi = first.pitch.as_midi();
                    let name = NoteName::from_midi(midi % 12);
                    let octave = (midi / 12) as i8 - 1;
                    format!("{name:?}{octave}")
                } else {
                    "—".to_owned()
                };
                ui.label(RichText::new(pitch_text).color(t.colors.text_primary));

                // Start
                caption(ui, "Start", CaptionTone::Dim);
                let start_text = if starts_equal {
                    let beats =
                        first.start_tick.0 as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;
                    format!("{beats:.2} beats")
                } else {
                    "—".to_owned()
                };
                ui.label(RichText::new(start_text).color(t.colors.text_primary));

                // Length
                caption(ui, "Len", CaptionTone::Dim);
                let length_text = if lengths_equal {
                    match durations[0] {
                        Some(d) => {
                            let beats = d as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;
                            format!("{beats:.2} beats")
                        }
                        None => "open".to_owned(),
                    }
                } else {
                    "—".to_owned()
                };
                ui.label(RichText::new(length_text).color(t.colors.text_primary));

                caption(ui, "Vel", CaptionTone::Dim);
                let mut vel_pct = if velocities_equal {
                    (first.velocity.as_f32() * 100.0).round()
                } else {
                    50.0
                };
                let vel_resp = unit_drag_value(ui, &mut vel_pct, 1.0..=100.0, 1.0, " %")
                    .on_hover_text(if velocities_equal {
                        "Velocity of selected notes"
                    } else {
                        "Velocities differ — drag to set all to the same value"
                    });
                if vel_resp.drag_started() || vel_resp.gained_focus() {
                    view_state.inspector_vel_drag_start = Some((
                        data.pattern_id,
                        selected.iter().map(|n| (n.note_id, n.velocity)).collect(),
                    ));
                }
                if vel_resp.changed() {
                    let new_vel = Velocity::new((vel_pct / 100.0).clamp(0.01, 1.0));
                    let pid = data.pattern_id;
                    let ids: Vec<NoteId> = selected.iter().map(|n| n.note_id).collect();
                    let mut song_w = song.write();
                    if let Some(pattern) = song_w.pattern_mut(pid) {
                        for nid in ids {
                            pattern.set_note_velocity(nid, new_vel);
                        }
                    }
                }
                if (vel_resp.drag_stopped() || vel_resp.lost_focus())
                    && let Some((pid, before)) = view_state.inspector_vel_drag_start.take()
                    && pid == data.pattern_id
                {
                    let new_vel = Velocity::new((vel_pct / 100.0).clamp(0.01, 1.0));
                    let changes: Vec<(NoteId, Velocity, Velocity)> = before
                        .into_iter()
                        .filter_map(|(id, old)| (old != new_vel).then_some((id, old, new_vel)))
                        .collect();
                    if !changes.is_empty() {
                        undo_manager.push(crate::undo::UndoAction::SetVelocitiesBatch {
                            pattern_id: data.pattern_id,
                            changes,
                        });
                    }
                }
            });

            // ── Per-note expression: tie/legato + glide (taxonomy primitive 2) ──
            let pid = data.pattern_id;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                // Tie / legato — discrete toggle, one undo entry per click.
                let mut legato = selected.iter().all(|n| n.legato);
                if ui
                    .checkbox(&mut legato, "Tie")
                    .on_hover_text("Legato: connect to the next note without retriggering")
                    .changed()
                {
                    let changes: Vec<(NoteId, bool, bool)> = selected
                        .iter()
                        .filter(|n| n.legato != legato)
                        .map(|n| (n.note_id, n.legato, legato))
                        .collect();
                    if !changes.is_empty() {
                        {
                            let mut song_w = song.write();
                            if let Some(pattern) = song_w.pattern_mut(pid) {
                                for (nid, _, new) in &changes {
                                    pattern.set_note_legato(*nid, *new);
                                }
                            }
                        }
                        undo_manager.push(crate::undo::UndoAction::SetLegatoBatch {
                            pattern_id: pid,
                            changes,
                        });
                    }
                }

                ui.separator();

                // Glide enable — discrete toggle; enabling installs a sensible default.
                let default_glide = Glide {
                    from: GlideFrom::Semitones(Semitones::new(-2.0)),
                    time: Milliseconds::new(100.0),
                    interp: GlideInterp::Continuous,
                };
                let mut glide_on =
                    !selected.is_empty() && selected.iter().all(|n| n.glide.is_some());
                if ui
                    .checkbox(&mut glide_on, "Glide")
                    .on_hover_text("Portamento/glissando into this note")
                    .changed()
                {
                    let new_glide = glide_on.then_some(default_glide);
                    let changes: Vec<(NoteId, Option<Glide>, Option<Glide>)> = selected
                        .iter()
                        .filter(|n| n.glide != new_glide)
                        .map(|n| (n.note_id, n.glide, new_glide))
                        .collect();
                    if !changes.is_empty() {
                        {
                            let mut song_w = song.write();
                            if let Some(pattern) = song_w.pattern_mut(pid) {
                                for (nid, _, new) in &changes {
                                    pattern.set_note_glide(*nid, *new);
                                }
                            }
                        }
                        undo_manager.push(crate::undo::UndoAction::SetGlideBatch {
                            pattern_id: pid,
                            changes,
                        });
                    }
                }

                // Glide parameters — shown when at least one selected note glides.
                if let Some(cur) = selected.iter().find_map(|n| n.glide) {
                    // The inspector expresses the source as a relative offset; an
                    // absolute Pitch source (MCP-set) collapses to the default.
                    let mut from_semis = match cur.from {
                        GlideFrom::Semitones(s) => s.as_f32(),
                        GlideFrom::Pitch(_) => -2.0,
                    };
                    let mut time_ms = cur.time.as_f32();
                    let mut stepped = matches!(cur.interp, GlideInterp::Stepped);

                    caption(ui, "From", CaptionTone::Dim);
                    let from_resp = unit_drag_value(ui, &mut from_semis, -24.0..=24.0, 0.5, " st")
                        .on_hover_text("Glide source, semitones relative to this note");

                    caption(ui, "Time", CaptionTone::Dim);
                    let time_resp = unit_drag_value(ui, &mut time_ms, 0.0..=2000.0, 2.0, " ms")
                        .on_hover_text("Glide time");

                    let make = |from_semis: f32, time_ms: f32, stepped: bool| Glide {
                        from: GlideFrom::Semitones(Semitones::new(from_semis)),
                        time: Milliseconds::new(time_ms),
                        interp: if stepped {
                            GlideInterp::Stepped
                        } else {
                            GlideInterp::Continuous
                        },
                    };

                    // Capture pre-drag glide once, collapse the whole drag into one
                    // undo entry on release (mirrors the velocity DragValue).
                    if from_resp.drag_started()
                        || from_resp.gained_focus()
                        || time_resp.drag_started()
                        || time_resp.gained_focus()
                    {
                        view_state.inspector_glide_drag_start =
                            Some((pid, selected.iter().map(|n| (n.note_id, n.glide)).collect()));
                    }
                    // From/Time edits only touch notes that *already* glide — they
                    // never force glide onto a non-gliding note in a mixed selection
                    // (use the Glide checkbox for that). Multi-edit shares the one
                    // representative value, like the velocity inspector.
                    if from_resp.changed() || time_resp.changed() {
                        let g = make(from_semis, time_ms, stepped);
                        let mut song_w = song.write();
                        if let Some(pattern) = song_w.pattern_mut(pid) {
                            for n in selected.iter().filter(|n| n.glide.is_some()) {
                                pattern.set_note_glide(n.note_id, Some(g));
                            }
                        }
                    }
                    if (from_resp.drag_stopped()
                        || from_resp.lost_focus()
                        || time_resp.drag_stopped()
                        || time_resp.lost_focus())
                        && let Some((dpid, before)) = view_state.inspector_glide_drag_start.take()
                        && dpid == pid
                    {
                        let g = make(from_semis, time_ms, stepped);
                        let changes: Vec<(NoteId, Option<Glide>, Option<Glide>)> = before
                            .into_iter()
                            .filter_map(|(id, old)| {
                                // Only notes that already glided were edited above.
                                (old.is_some() && old != Some(g)).then_some((id, old, Some(g)))
                            })
                            .collect();
                        if !changes.is_empty() {
                            undo_manager.push(crate::undo::UndoAction::SetGlideBatch {
                                pattern_id: pid,
                                changes,
                            });
                        }
                    }

                    // Stepped (glissando) toggle — discrete, one undo entry.
                    if ui
                        .checkbox(&mut stepped, "Stepped")
                        .on_hover_text(
                            "Glissando: hold at chromatic steps instead of a smooth ramp",
                        )
                        .changed()
                    {
                        let g = make(from_semis, time_ms, stepped);
                        let changes: Vec<(NoteId, Option<Glide>, Option<Glide>)> = selected
                            .iter()
                            .filter(|n| n.glide.is_some() && n.glide != Some(g))
                            .map(|n| (n.note_id, n.glide, Some(g)))
                            .collect();
                        if !changes.is_empty() {
                            {
                                let mut song_w = song.write();
                                if let Some(pattern) = song_w.pattern_mut(pid) {
                                    for (nid, _, new) in &changes {
                                        pattern.set_note_glide(*nid, *new);
                                    }
                                }
                            }
                            undo_manager.push(crate::undo::UndoAction::SetGlideBatch {
                                pattern_id: pid,
                                changes,
                            });
                        }
                    }
                }
            });

            // ── Per-note expression block: accent / gate / ghost / probability ──
            // (taxonomy primitive 3 note-shape scalars + primitive 1 vibrato).
            // Each control edits only its own field, preserving the others per note.
            // Multi-edit semantics match the velocity inspector: controls display
            // the first selected note's value and an edit writes that field to all
            // selected (it does not preserve per-note variation of the edited
            // field — only of the *other* fields).
            let cur = selected
                .iter()
                .find_map(|n| n.expression)
                .unwrap_or_default();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;

                // Accent (velocity ×). DragValue → live edit + one undo on release.
                caption(ui, "Accent", CaptionTone::Dim);
                let mut accent = cur.accent.unwrap_or(1.0);
                let r = ui
                    .add(
                        egui::DragValue::new(&mut accent)
                            .range(0.0..=4.0)
                            .speed(0.02)
                            .fixed_decimals(2),
                    )
                    .on_hover_text("Velocity multiplier (1.0 = unchanged)");
                if r.drag_started() || r.gained_focus() {
                    view_state.inspector_expr_drag_start = Some((
                        pid,
                        selected.iter().map(|n| (n.note_id, n.expression)).collect(),
                    ));
                }
                if r.changed() {
                    // Returning to the neutral value (1.0×) clears the field so the
                    // block can collapse to None (no lingering expression dot).
                    let accent_field = (accent != 1.0).then_some(accent);
                    live_expression_edit(song, pid, &selected, |e| e.accent = accent_field);
                }
                if (r.drag_stopped() || r.lost_focus())
                    && let Some((dpid, before)) = view_state.inspector_expr_drag_start.take()
                    && dpid == pid
                {
                    finish_expression_drag(song, undo_manager, pid, before);
                }

                // Gate (% of duration, staccato/tenuto).
                caption(ui, "Gate", CaptionTone::Dim);
                let mut gate_pct = cur.gate.map_or(100.0, |g| g.as_f32() * 100.0);
                let r = unit_drag_value(ui, &mut gate_pct, 1.0..=100.0, 1.0, " %")
                    .on_hover_text("Note length as a % of its duration (staccato)");
                if r.drag_started() || r.gained_focus() {
                    view_state.inspector_expr_drag_start = Some((
                        pid,
                        selected.iter().map(|n| (n.note_id, n.expression)).collect(),
                    ));
                }
                if r.changed() {
                    // Neutral gate (100% = full duration) clears the field.
                    let gate_field = (gate_pct < 100.0)
                        .then(|| NormalizedValue::new((gate_pct / 100.0).clamp(0.0, 1.0)));
                    live_expression_edit(song, pid, &selected, |e| e.gate = gate_field);
                }
                if (r.drag_stopped() || r.lost_focus())
                    && let Some((dpid, before)) = view_state.inspector_expr_drag_start.take()
                    && dpid == pid
                {
                    finish_expression_drag(song, undo_manager, pid, before);
                }

                // Probability (% chance to play).
                caption(ui, "Prob", CaptionTone::Dim);
                let mut prob_pct = cur.probability.map_or(100.0, |p| p.as_f32() * 100.0);
                let r = unit_drag_value(ui, &mut prob_pct, 0.0..=100.0, 1.0, " %")
                    .on_hover_text("Chance this note plays (resolved at playback)");
                if r.drag_started() || r.gained_focus() {
                    view_state.inspector_expr_drag_start = Some((
                        pid,
                        selected.iter().map(|n| (n.note_id, n.expression)).collect(),
                    ));
                }
                if r.changed() {
                    // Neutral probability (100% = always plays) clears the field.
                    let prob_field = (prob_pct < 100.0)
                        .then(|| NormalizedValue::new((prob_pct / 100.0).clamp(0.0, 1.0)));
                    live_expression_edit(song, pid, &selected, |e| e.probability = prob_field);
                }
                if (r.drag_stopped() || r.lost_focus())
                    && let Some((dpid, before)) = view_state.inspector_expr_drag_start.take()
                    && dpid == pid
                {
                    finish_expression_drag(song, undo_manager, pid, before);
                }

                // Ghost (forced-soft) — discrete toggle.
                let mut ghost = cur.ghost;
                if ui
                    .checkbox(&mut ghost, "Ghost")
                    .on_hover_text("Force a soft (ghost) velocity")
                    .changed()
                {
                    apply_expression_edit(song, undo_manager, pid, &selected, |e| e.ghost = ghost);
                }
            });

            // ── Vibrato mini-control (taxonomy primitive 1): enable + depth/rate ──
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let mut vib_on = cur.vibrato.is_some();
                if ui
                    .checkbox(&mut vib_on, "Vibrato")
                    .on_hover_text("Per-note pitch vibrato")
                    .changed()
                {
                    let v = vib_on.then(default_inspector_vibrato);
                    apply_expression_edit(song, undo_manager, pid, &selected, |e| e.vibrato = v);
                }
                if let Some(v) = cur.vibrato {
                    caption(ui, "Depth", CaptionTone::Dim);
                    let mut depth = v.depth.as_f32();
                    let rd = unit_drag_value(ui, &mut depth, 0.0..=2.0, 0.01, " st")
                        .on_hover_text("Vibrato depth (semitones)");
                    caption(ui, "Rate", CaptionTone::Dim);
                    let mut rate = v.rate.as_f32();
                    let rr = unit_drag_value(ui, &mut rate, 0.1..=20.0, 0.1, " Hz")
                        .on_hover_text("Vibrato rate");
                    if rd.drag_started()
                        || rd.gained_focus()
                        || rr.drag_started()
                        || rr.gained_focus()
                    {
                        view_state.inspector_expr_drag_start = Some((
                            pid,
                            selected.iter().map(|n| (n.note_id, n.expression)).collect(),
                        ));
                    }
                    if rd.changed() || rr.changed() {
                        // Only notes that already have vibrato get depth/rate edits.
                        let mut song_w = song.write();
                        if let Some(pattern) = song_w.pattern_mut(pid) {
                            for n in selected
                                .iter()
                                .filter(|n| n.expression.is_some_and(|e| e.vibrato.is_some()))
                            {
                                let mut e = n.expression.unwrap_or_default();
                                if let Some(vib) = e.vibrato.as_mut() {
                                    vib.depth = Semitones::new(depth);
                                    vib.rate = Hertz::new(rate);
                                }
                                pattern.set_note_expression(n.note_id, Some(e));
                            }
                        }
                    }
                    if (rd.drag_stopped()
                        || rd.lost_focus()
                        || rr.drag_stopped()
                        || rr.lost_focus())
                        && let Some((dpid, before)) = view_state.inspector_expr_drag_start.take()
                        && dpid == pid
                    {
                        finish_expression_drag(song, undo_manager, pid, before);
                    }
                }
            });
            ui.separator();

            // ── Per-note note-scope graph (plan §2.1): an articulation graph
            // bound to this note (strum / flam / arp of this one note), the
            // generalization of the ornament. Applies to the whole selection
            // like velocity; decorrelated per note by host key at playback. ──
            {
                let pid = data.pattern_id;
                let bindings_equal = selected.iter().all(|n| n.note_graph == first.note_graph);
                let common = bindings_equal.then_some(first.note_graph).flatten();
                let pool: Vec<(synth_sequencer::NoteGraphId, String)> = song
                    .try_read()
                    .map(|s| s.note_graphs().map(|g| (g.id, g.name.clone())).collect())
                    .unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    caption(ui, "Graph", CaptionTone::Dim);
                    let selected_label = if !bindings_equal {
                        "— (mixed)".to_owned()
                    } else {
                        match common {
                            Some(gid) => pool.iter().find(|(id, _)| *id == gid).map_or_else(
                                || format!("missing graph {}", gid.0),
                                |(_, n)| n.clone(),
                            ),
                            None => "None".to_owned(),
                        }
                    };
                    let mut chosen: Option<Option<synth_sequencer::NoteGraphId>> = None;
                    egui::ComboBox::from_id_salt("piano_note_graph_binding")
                        .selected_text(selected_label)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(bindings_equal && common.is_none(), "None")
                                .clicked()
                            {
                                chosen = Some(None);
                            }
                            for (gid, name) in &pool {
                                if ui.selectable_label(common == Some(*gid), name).clicked() {
                                    chosen = Some(Some(*gid));
                                }
                            }
                        })
                        .response
                        .on_hover_text(
                            "Per-note articulation graph — strum / flam / arp of this one note",
                        );
                    if let Some(new_graph) = chosen {
                        let changes: Vec<(
                            NoteId,
                            Option<synth_sequencer::NoteGraphId>,
                            Option<synth_sequencer::NoteGraphId>,
                        )> = selected
                            .iter()
                            .filter(|n| n.note_graph != new_graph)
                            .map(|n| (n.note_id, n.note_graph, new_graph))
                            .collect();
                        if !changes.is_empty() {
                            {
                                let mut song_w = song.write();
                                if let Some(pattern) = song_w.pattern_mut(pid) {
                                    for (nid, _, new) in &changes {
                                        pattern.set_note_note_graph(*nid, *new);
                                    }
                                }
                            }
                            undo_manager.push(crate::undo::UndoAction::SetNoteGraphBindingBatch {
                                pattern_id: pid,
                                changes,
                            });
                        }
                    }
                    // Jump into the Note Grid view for a single resolved binding.
                    if let Some(gid) = common
                        && pool.iter().any(|(id, _)| *id == gid)
                        && ui
                            .small_button("Edit…")
                            .on_hover_text("Open this graph in the Note Grid view")
                            .clicked()
                    {
                        view_state.jump_to_note_graph = Some(gid);
                    }
                });
                ui.separator();
            }

            // ── Per-note ornament (single selection) ──
            if selected.len() == 1 {
                let nid = selected[0].note_id;
                // Outer `Some` = lock acquired; inner = the note's ornament (or
                // `None`). The editor only opens from a successful read, so a
                // transient lock miss can't capture a wrong `None` baseline and
                // clobber an existing ornament.
                let read = song.try_read().map(|s| {
                    s.pattern(data.pattern_id)
                        .and_then(|p| p.note(nid))
                        .and_then(|n| n.ornament)
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    caption(ui, "Ornament", CaptionTone::Dim);
                    ui.label(
                        RichText::new(ornament_summary(read.flatten()))
                            .color(t.colors.text_secondary),
                    );
                    if ui.button("Edit…").clicked()
                        && let Some(current) = read
                    {
                        view_state.editing_ornament = Some(OrnamentEdit {
                            pattern_id: data.pattern_id,
                            note_id: nid,
                            before: current,
                            current,
                        });
                    }
                });
                ui.separator();
            }
        }
    }

    draw_ornament_popup(ui, song, view_state, undo_manager);
}

/// Shared automation-target selector ComboBox (labelled "Auto:"). Sets
/// `view_state.selected_automation`. Lists existing lanes with points (badged when
/// they belong to another instrument), every instrument param, and the automatable
/// module params of the selected instrument. Used by both the piano-roll toolbar and
/// the tracker's "add automation column" control.
pub(crate) fn draw_automation_target_selector(
    ui: &mut egui::Ui,
    view_state: &mut SequencerViewState,
    data: &PianoRollData,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
) {
    dim_label(ui, "Auto:");
    // Button label = current selection (mirrors what a ComboBox's selected_text
    // would show). The dropdown itself is a nested `menu_button` tree so params
    // group under their module, like the Script ƒx "Select input" picker.
    let auto_label = view_state
        .selected_automation
        .as_ref()
        .map_or_else(|| "None".to_owned(), AutomationTarget::display_name);

    tree_picker_button(ui, "auto_lane_select", 200.0, auto_label, |ui| {
        // Keep the popup at least as wide as the button so long target names
        // ("This Track: Pitch", "module:flt:1:cutoff") don't collapse it.
        ui.set_min_width(200.0);
        // Grow to fit all rows (egui clamps the popup to the window, so a
        // generous cap effectively means "show everything, scroll only if the
        // menu is taller than the screen").
        egui::ScrollArea::vertical()
            .max_height(900.0)
            .show(ui, |ui| {
                // "None" clears the selection (hides the automation zone).
                if ui
                    .selectable_label(view_state.selected_automation.is_none(), "None")
                    .clicked()
                {
                    view_state.selected_automation = None;
                    ui.close();
                }

                // Category submenus at the top — Instrument / Track / Global —
                // then a separator, then the per-module parameters below. (Active
                // lanes are no longer flat-listed here: every lane is a clickable
                // stacked zone, so the list would duplicate them.)

                // Instrument-level macros for the selected instrument.
                submenu_button(ui, "Instrument", |ui| {
                    for param in AutoInstrumentParam::ALL {
                        let target = AutomationTarget::Instrument {
                            instrument: view_state.selected_instrument,
                            param: *param,
                        };
                        let label = param.display_name().to_owned();
                        let is_selected = view_state.selected_automation.as_ref() == Some(&target);
                        if ui.selectable_label(is_selected, &label).clicked() {
                            view_state.selected_automation = Some(target);
                            ui.close();
                        }
                    }
                });

                // Track params. The default authoring form is a host-track lane
                //    (`Track { None }`), which follows whatever track the pattern
                //    is placed on — offered flat as "This Track: <param>". A
                //    "Cross-track" submenu lists explicit tracks for the rare
                //    deliberate case of automating another track from here.
                submenu_button(ui, "Track", |ui| {
                    for param in TrackParam::ALL {
                        let target = AutomationTarget::Track {
                            track: None,
                            param: *param,
                        };
                        let label = format!("This Track: {}", param.display_name());
                        let is_selected = view_state.selected_automation.as_ref() == Some(&target);
                        if ui.selectable_label(is_selected, &label).clicked() {
                            view_state.selected_automation = Some(target);
                            ui.close();
                        }
                    }
                    // Cross-track: name a specific track. Only meaningful when
                    // the song has tracks to point at.
                    if !data.all_tracks.is_empty() {
                        ui.separator();
                        submenu_button(ui, "Cross-track", |ui| {
                            for (track_id, track_name) in &data.all_tracks {
                                submenu_button(ui, track_name, |ui| {
                                    for param in TrackParam::ALL {
                                        let target = AutomationTarget::Track {
                                            track: Some(*track_id),
                                            param: *param,
                                        };
                                        let is_selected = view_state.selected_automation.as_ref()
                                            == Some(&target);
                                        if ui
                                            .selectable_label(is_selected, param.display_name())
                                            .clicked()
                                        {
                                            view_state.selected_automation = Some(target);
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        });
                    }
                });

                // Global params (master volume). Song-spanning, so authoring
                //    one here hosts the lane on this pattern.
                submenu_button(ui, "Global", |ui| {
                    let target = AutomationTarget::Global(GlobalParam::MasterVolume);
                    let is_selected = view_state.selected_automation.as_ref() == Some(&target);
                    if ui
                        .selectable_label(is_selected, target.display_name())
                        .clicked()
                    {
                        view_state.selected_automation = Some(target);
                        ui.close();
                    }
                });

                ui.separator();

                // Per-module parameters of the selected instrument (generic
                //    module targets), filtered to the automatable allowlist and
                //    grouped into one submenu per module. Lets the user automate
                //    any continuous, RT-safe module parameter.
                if let Some(inst) = instruments
                    .iter()
                    .find(|i| i.id == view_state.selected_instrument)
                {
                    let mut module_ids = inst.patch_editor.module_ids();
                    module_ids.sort_unstable(); // deterministic (type, instance) order
                    for module_id in module_ids {
                        let Some(desc) = inst.patch_editor.module_descriptor(module_id) else {
                            continue;
                        };
                        // Skip a module with no automatable params so it doesn't
                        // show an empty submenu.
                        if !desc.parameters.iter().any(|p| p.is_automatable()) {
                            continue;
                        }
                        let module_label = format!("{} {}", desc.name, module_id.instance);
                        submenu_button(ui, module_label, |ui| {
                            for param in &desc.parameters {
                                if !param.is_automatable() {
                                    continue;
                                }
                                let target = AutomationTarget::Module {
                                    instrument: view_state.selected_instrument,
                                    module_type: module_id.module_type,
                                    instance: module_id.instance,
                                    param_id: param.type_id.as_str().into(),
                                };
                                let is_selected =
                                    view_state.selected_automation.as_ref() == Some(&target);
                                if ui.selectable_label(is_selected, &param.name).clicked() {
                                    view_state.selected_automation = Some(target);
                                    ui.close();
                                }
                            }
                        });
                    }
                }
            });
    });
}

/// Shared pattern-editor controls rendered inline into the caller's toolbar row:
/// the working-instrument selector, the "track plays" badge, and the mini-transport
/// (play/pause, stop, record arm/disarm, pattern-solo). Both the piano roll and the
/// tracker call this so they show the same row without duplicating the
/// recording-arm logic.
pub(super) fn draw_pattern_instrument_transport(
    ui: &mut egui::Ui,
    data: &PianoRollData,
    handle: &mut EngineHandle,
    song: &Arc<synth_sequencer::SharedSong>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    is_playing: bool,
) {
    let t = theme();

    // Snap the working instrument to the first instrument the pattern's tracks
    // play through (track_overrides is ordered by placement/track). Re-fires
    // whenever the open pattern OR that first-used instrument changes — so
    // placing/moving the pattern onto a track after it was already open updates
    // the pick too — but not every frame, so a manual pick sticks while the
    // placement is unchanged.
    let auto_key = (data.pattern_id, data.track_overrides.first().copied());
    if view_state.last_auto_instrument != Some(auto_key) {
        view_state.last_auto_instrument = Some(auto_key);
        if let Some(first) = auto_key.1 {
            view_state.selected_instrument = first;
        }
    }

    // Instrument selector for new notes
    {
        let selected_label = instruments
            .iter()
            .find(|inst| inst.id == view_state.selected_instrument)
            .map_or_else(|| "---".to_owned(), |inst| inst.name.clone());
        egui::ComboBox::from_id_salt(ui.id().with("piano_roll_instrument"))
            .selected_text(RichText::new(&selected_label).size(12.0))
            .width(100.0)
            .height(700.0)
            .show_ui(ui, |ui| {
                // Instruments the pattern actually plays through (its tracks'
                // instruments) float to the top, marked with a dot; the rest
                // follow after a separator.
                let used = &data.track_overrides;
                let render = |ui: &mut egui::Ui,
                              inst: &crate::gui::instrument_rack::InstrumentUiState,
                              view_state: &mut SequencerViewState,
                              mark: bool| {
                    let seq_id = inst.id;
                    let selected = view_state.selected_instrument == seq_id;
                    let label = if mark {
                        format!("• {}", inst.name)
                    } else {
                        inst.name.clone()
                    };
                    if ui.selectable_label(selected, label).clicked() {
                        view_state.selected_instrument = seq_id;
                    }
                };
                let mut any_used = false;
                for seq_id in used {
                    if let Some(inst) = instruments.iter().find(|i| i.id == *seq_id) {
                        render(ui, inst, view_state, true);
                        any_used = true;
                    }
                }
                if any_used {
                    ui.separator();
                }
                for inst in instruments {
                    let is_used = used.contains(&inst.id);
                    if is_used {
                        continue; // already shown at the top
                    }
                    render(ui, inst, view_state, false);
                }
            });

        // Badge: which instrument(s) the pattern's placements actually play
        // through. The selector above is only the working instrument (note
        // colour + preview); playback always routes via the track.
        if !data.track_overrides.is_empty() {
            let names: Vec<String> = data
                .track_overrides
                .iter()
                .map(|seq_id| {
                    instruments
                        .iter()
                        .find(|inst| inst.id == *seq_id)
                        .map_or_else(|| format!("#{}", seq_id.as_u64()), |inst| inst.name.clone())
                })
                .collect();
            let arrow = egui_remixicon::icons::ARROW_RIGHT_S_LINE;
            let badge_text = if names.len() == 1 {
                format!("{arrow} track plays: {}", names[0])
            } else {
                format!("{arrow} track plays: {}", names.join(", "))
            };
            ui.label(
                RichText::new(badge_text)
                    .size(10.0)
                    .color(t.colors.accent_yellow),
            )
            .on_hover_text(
                "Instrument(s) this pattern plays through where it is placed \
                    (set per track). The selector above is the working instrument \
                    for note colour and preview only.",
            );
        }
    }
    ui.separator();

    // Mini-transport controls
    {
        use egui_remixicon::icons as ri;
        ui.spacing_mut().item_spacing.x = 2.0;

        if is_playing {
            // Pause button
            if ui
                .button(
                    RichText::new(ri::PAUSE_FILL)
                        .size(12.0)
                        .color(t.colors.accent_yellow),
                )
                .on_hover_text("Pause")
                .clicked()
            {
                handle.send(EngineCommand::Pause);
            }
        } else {
            // Play pattern button — always loops the open pattern.
            // Pattern-solo toggle controls whether other tracks/patterns
            // are silenced during this playback.
            let play_tooltip = if view_state.pattern_solo {
                "Play pattern (solo — other tracks silent)"
            } else {
                "Play pattern (with other tracks)"
            };
            if ui
                .button(
                    RichText::new(ri::PLAY_FILL)
                        .size(12.0)
                        .color(t.colors.accent_green),
                )
                .on_hover_text(play_tooltip)
                .clicked()
            {
                // Only take the armed-Play path when recording is armed for
                // THIS pattern. Routing through `Play` runs the engine's
                // Armed → CountIn → Capturing flow (`PlayPattern` skips that
                // transition). If armed for a *different* pattern, fall
                // through and just audition the one on screen. Solo is not
                // applied while recording: the engine is either in orphan-
                // preview mode (solo ignored there) or the user wants to
                // hear surrounding context while overdubbing.
                let armed_for_this = handle.state.transport.recording_state()
                    == RecordingState::Armed
                    && view_state.recording_pattern == Some(data.pattern_id);
                if armed_for_this {
                    handle.send(EngineCommand::Play);
                } else {
                    let solo_target = if view_state.pattern_solo {
                        Some(data.pattern_id)
                    } else {
                        None
                    };
                    handle.send(EngineCommand::SetSoloPattern(solo_target));
                    handle.send(EngineCommand::PlayPattern {
                        pattern_id: data.pattern_id,
                        instrument: view_state.selected_instrument,
                    });
                }
                // Starting playback re-engages playhead follow (both editors), so a
                // prior manual scroll-away doesn't leave the view stuck off-screen.
                view_state.auto_follow_playhead = true;
            }
        }

        // Stop button — also clears the solo filter via the engine.
        if ui
            .button(
                RichText::new(ri::STOP_FILL)
                    .size(12.0)
                    .color(if is_playing {
                        t.colors.accent_red
                    } else {
                        t.colors.text_dim
                    }),
            )
            .on_hover_text("Stop")
            .clicked()
        {
            handle.send(EngineCommand::Stop);
        }

        // Mini record button — mirrors the global transport's record
        // arm/disarm behaviour so the user does not have to reach
        // back up while editing in the piano roll.
        let mini_rec_state = handle.state.transport.recording_state();
        let dim_red = DIM_RED;
        let mini_rec_color = match mini_rec_state {
            RecordingState::Capturing => t.colors.accent_red,
            RecordingState::CountIn | RecordingState::Armed => {
                let blink = ((ui.input(|i| i.time) * 2.0) as u64).is_multiple_of(2);
                if blink { t.colors.accent_red } else { dim_red }
            }
            RecordingState::Idle => dim_red,
        };
        if ui
            .button(
                RichText::new(ri::RECORD_CIRCLE_FILL)
                    .size(12.0)
                    .color(mini_rec_color),
            )
            .on_hover_text(match mini_rec_state {
                RecordingState::Idle => "Arm recording",
                _ => "Disarm recording",
            })
            .clicked()
        {
            if mini_rec_state != RecordingState::Idle {
                handle.send(EngineCommand::DisarmRecord);
            } else {
                arm_recording_for_pattern(handle, song, view_state, data.pattern_id);
            }
        }
        if matches!(
            mini_rec_state,
            RecordingState::Armed | RecordingState::CountIn
        ) {
            ui.ctx().request_repaint();
        }

        // Pattern solo toggle — when ON (default), pattern playback
        // isolates the open pattern. When OFF, other tracks/patterns
        // overlapping the pattern's time range also play. Shared solo
        // helper (headphone/yellow) so it matches the mixer + arrangement.
        if solo_toggle(ui, view_state.pattern_solo).clicked() {
            view_state.pattern_solo = !view_state.pattern_solo;
            if is_playing {
                let solo_target = if view_state.pattern_solo {
                    Some(data.pattern_id)
                } else {
                    None
                };
                handle.send(EngineCommand::SetSoloPattern(solo_target));
            }
        }

        ui.spacing_mut().item_spacing.x = 8.0;
    }
}

/// Piano-roll coordinate transforms between grid pixels and `(tick, pitch)`.
///
/// Bundles the seven view scalars that the four transform closures used to
/// capture, so they travel as one `Copy` value instead of four `&dyn Fn`
/// arguments plus three loose scalars (collapsing
/// `handle_piano_roll_interaction`'s parameter list). The methods reproduce the
/// former closures verbatim.
#[derive(Clone, Copy)]
struct PianoRollCoords {
    /// Left edge of the note grid (x of tick 0), in screen pixels.
    grid_x: f32,
    /// Top edge of the note grid (y of the top visible row), in screen pixels.
    grid_y: f32,
    /// Pattern ticks per beat (quarter note).
    ticks_per_beat: u32,
    /// Horizontal scale: screen pixels per beat.
    pr_pixels_per_beat: f32,
    /// Height of one pitch row, in screen pixels.
    note_row_height: f32,
    /// Lowest visible pitch (clamps `y_to_pitch`).
    view_pitch_min: Pitch,
    /// Highest visible pitch (grid row 0).
    view_pitch_max: Pitch,
}

impl PianoRollCoords {
    /// Tick → x screen position.
    fn tick_to_x(&self, tick_val: PatternTick) -> f32 {
        if self.ticks_per_beat == 0 {
            return self.grid_x;
        }
        let beats = tick_val.0 as f32 / self.ticks_per_beat as f32;
        self.grid_x + beats * self.pr_pixels_per_beat
    }

    /// Pitch → y screen position (higher pitch = lower y, piano style).
    fn pitch_to_y(&self, pitch: Pitch) -> f32 {
        let row = self
            .view_pitch_max
            .as_midi()
            .saturating_sub(pitch.as_midi());
        self.grid_y + row as f32 * self.note_row_height
    }

    /// x screen position → tick (clamped at 0).
    fn x_to_tick(&self, x: f32) -> PatternTick {
        #[allow(clippy::cast_possible_truncation)]
        let tick =
            ((x - self.grid_x) / self.pr_pixels_per_beat * self.ticks_per_beat as f32).max(0.0);
        PatternTick(tick as u32)
    }

    /// y screen position → pitch (clamped to the visible range).
    fn y_to_pitch(&self, y: f32) -> Pitch {
        #[allow(clippy::cast_possible_truncation)]
        let row = ((y - self.grid_y) / self.note_row_height).floor().max(0.0) as u8;
        let midi = self
            .view_pitch_max
            .as_midi()
            .saturating_sub(row)
            .clamp(self.view_pitch_min.as_midi(), self.view_pitch_max.as_midi());
        Pitch::new(midi).unwrap_or(Pitch::MIDDLE_C)
    }
}

/// Bundle of the long-lived locals every piano-roll sub-section threads through:
/// the song snapshot, live engine handle, view state, undo manager and instrument
/// list. Lets the extracted toolbar / shortcut / interaction helpers take one
/// `&mut ctx` instead of 6+ positional parameters. Each helper re-exposes the
/// fields under their original names, so the moved bodies stay byte-for-byte
/// unchanged (same trick `handle_piano_roll_interaction` uses for the bundled
/// `PianoRollCoords` transforms).
struct PianoRollCtx<'a> {
    data: &'a PianoRollData,
    song: &'a Arc<synth_sequencer::SharedSong>,
    view_state: &'a mut SequencerViewState,
    handle: &'a mut EngineHandle,
    undo_manager: &'a mut crate::undo::UndoManager,
    instruments: &'a [crate::gui::instrument_rack::InstrumentUiState],
}

impl<'a> PianoRollCtx<'a> {
    /// Bundle the threaded locals. Keeps the field list in one place so the
    /// reborrow-per-call sites stay one line each.
    fn new(
        data: &'a PianoRollData,
        song: &'a Arc<synth_sequencer::SharedSong>,
        view_state: &'a mut SequencerViewState,
        handle: &'a mut EngineHandle,
        undo_manager: &'a mut crate::undo::UndoManager,
        instruments: &'a [crate::gui::instrument_rack::InstrumentUiState],
    ) -> Self {
        Self {
            data,
            song,
            view_state,
            handle,
            undo_manager,
            instruments,
        }
    }
}

/// Provenance (which mod graphs write this lane's target) + a quick-assign menu
/// that wires an LFO to it in ~3 clicks. Only shown when a lane is focused.
fn draw_mod_grid_lane_tools(
    ui: &mut egui::Ui,
    song: &std::sync::Arc<synth_sequencer::SharedSong>,
    data: &PianoRollData,
    view_state: &mut SequencerViewState,
    undo_manager: &mut crate::undo::UndoManager,
) {
    let Some(sel) = view_state.selected_automation.clone() else {
        return;
    };
    // Provenance chips: the mod graphs whose Target nodes write this param.
    let writers: Vec<(synth_sequencer::ModGraphId, String)> = {
        let Some(s) = song.try_read() else {
            return;
        };
        s.mod_graphs()
            .filter(|g| {
                g.nodes().values().any(|n| match n {
                    synth_sequencer::ModNodeConfig::Target(t) => {
                        mod_target_matches(&t.target, &sel)
                    }
                    _ => false,
                })
            })
            .map(|g| (g.id, g.name.clone()))
            .collect()
    };
    for (gid, name) in &writers {
        if ui
            .small_button(format!("⬲ {name}"))
            .on_hover_text("This mod graph modulates the focused target — open it")
            .clicked()
        {
            view_state.jump_to_mod_graph = Some(*gid);
        }
    }

    // Quick-assign: add an LFO wired to this target, in a new or existing graph.
    ui.menu_button(
        format!("{} Mod Grid", egui_remixicon::icons::ADD_LINE),
        |ui| {
            dim_label(ui, "Add an LFO modulating this target");
            let graphs: Vec<(synth_sequencer::ModGraphId, String)> = song
                .try_read()
                .map(|s| s.mod_graphs().map(|g| (g.id, g.name.clone())).collect())
                .unwrap_or_default();
            if ui.button("New graph + LFO").clicked() {
                quick_assign_mod_grid(song, undo_manager, view_state, data, None, &sel);
                ui.close();
            }
            if !graphs.is_empty() {
                ui.separator();
            }
            for (gid, name) in graphs {
                if ui.button(name).clicked() {
                    quick_assign_mod_grid(song, undo_manager, view_state, data, Some(gid), &sel);
                    ui.close();
                }
            }
        },
    );
}

/// Loose provenance match: a grid Target writes a lane's target when they name
/// the same param (track/instrument/global), ignoring the specific track so a
/// relative "this track" lane still shows its modulators.
fn mod_target_matches(grid: &AutomationTarget, sel: &AutomationTarget) -> bool {
    use AutomationTarget::{Global, Instrument, Module, Track};
    match (grid, sel) {
        (Track { param: a, .. }, Track { param: b, .. }) => a == b,
        (Global(a), Global(b)) => a == b,
        (
            Instrument {
                instrument: ia,
                param: pa,
            },
            Instrument {
                instrument: ib,
                param: pb,
            },
        ) => ia == ib && pa == pb,
        (Module { .. }, Module { .. }) => grid == sel,
        _ => false,
    }
}

/// Create (or reuse) a mod graph with an LFO wired to `sel`, and jump to it. For
/// a relative track target the graph is Track-scoped and assigned to the
/// pattern's host track so it resolves and modulates immediately.
fn quick_assign_mod_grid(
    song: &std::sync::Arc<synth_sequencer::SharedSong>,
    undo_manager: &mut crate::undo::UndoManager,
    view_state: &mut SequencerViewState,
    data: &PianoRollData,
    existing: Option<synth_sequencer::ModGraphId>,
    sel: &AutomationTarget,
) {
    use synth_sequencer::{ModConnection, ModGraphScope, ModNodeConfig, ModTarget, ModuleNode};
    // The pattern's first host track — used to resolve a relative track target.
    let host_track = {
        let Some(s) = song.try_read() else {
            return;
        };
        s.arrangement()
            .iter()
            .find(|p| p.pattern_id == data.pattern_id)
            .map(|p| p.track_id)
    };
    let is_relative_track = matches!(sel, AutomationTarget::Track { track: None, .. });

    let (gid, before, after) = {
        let mut s = song.write();
        let gid = existing.unwrap_or_else(|| s.create_mod_graph("Quick Mod"));
        let before = existing.and(s.mod_graph(gid).cloned());
        // Auto-scope + assign so a relative track target resolves — but only for
        // a freshly-created graph, never re-scoping an existing one the user set
        // up (which could break its other routings).
        if existing.is_none()
            && is_relative_track
            && let Some(track) = host_track
        {
            s.set_mod_graph_scope(gid, ModGraphScope::Track);
            let mut assigned: Vec<_> = s
                .mod_graph(gid)
                .map(|g| g.assigned_tracks.clone())
                .unwrap_or_default();
            if !assigned.contains(&track) {
                assigned.push(track);
            }
            s.assign_mod_graph(gid, &assigned);
        }
        if let Some(g) = s.mod_graph_mut(gid) {
            let lfo = g.next_node_id();
            let _ = g.try_insert_node(
                lfo,
                ModNodeConfig::Module(ModuleNode {
                    module_type: synth_core::ModuleType::Lfo,
                    params: Default::default(),
                    seed: None,
                }),
            );
            let tgt = g.next_node_id();
            let _ = g.try_insert_node(
                tgt,
                ModNodeConfig::Target(ModTarget {
                    target: sel.clone(),
                    amount: synth_sequencer::ModulationAmount::new(0.25),
                    combine: synth_sequencer::CombineMode::default(),
                }),
            );
            let _ = g.try_connect(ModConnection::new(lfo, "out", tgt, "in"));
        }
        let after = s.mod_graph(gid).cloned();
        (gid, before, after)
    };
    if after.is_some() {
        undo_manager.push(crate::undo::UndoAction::SetModGraph {
            graph_id: gid,
            old: before,
            new: after,
        });
    }
    view_state.jump_to_mod_graph = Some(gid);
}

/// Draw the piano roll in a bottom panel.
/// Returns false if the close button was clicked.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub(crate) fn draw_piano_roll(
    ui: &mut egui::Ui,
    data: &PianoRollData,
    playhead_tick: Option<PatternTick>,
    is_playing: bool,
    handle: &mut EngineHandle,
    song: &Arc<synth_sequencer::SharedSong>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    undo_manager: &mut crate::undo::UndoManager,
    editor_mode: Option<&mut crate::gui::pattern_view::PatternEditorMode>,
) -> bool {
    let t = theme();
    let keep_open = {
        let mut ctx = PianoRollCtx::new(data, song, view_state, handle, undo_manager, instruments);
        draw_piano_roll_toolbar(&mut ctx, ui, is_playing, editor_mode)
    };

    ui.separator();

    // ── Step entry banner ──
    if view_state.step_entry_mode {
        let banner_color = STEP_ENTRY_BANNER_FILL;
        egui::Frame::new()
            .fill(banner_color)
            .inner_margin(egui::Margin::symmetric(6, 4))
            .corner_radius(2)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    strong_label(ui, "STEP ENTRY", Some(STEP_ENTRY_TEXT));
                    ui.label(
                        RichText::new(
                            "Press keys to insert notes  (A·W·S·E·D·F·T·G·Y·H·U·J = C·C#·D·D#·E·F·F#·G·G#·A·A#·B)",
                        )
                        .size(11.0)
                        .color(t.colors.text_secondary),
                    );
                    ui.label(
                        RichText::new("· Esc disables").size(11.0).color(t.colors.text_dim),
                    );
                });
            });
        ui.separator();
    }

    draw_piano_roll_selection_inspector(ui, data, view_state, song, undo_manager);

    // ── Pitch range with margin ──
    // Fold live recording-preview pitches into the range so notes recorded
    // outside the committed range are visible immediately, not only after the
    // next loop wrap flushes them into the pattern. Only do this when the
    // preview belongs to the pattern on screen — the preview buffers are
    // global, so without this guard a recording into another pattern would
    // widen this one's vertical range to an empty band.
    let mut pitch_min = data.pitch_min;
    let mut pitch_max = data.pitch_max;
    if view_state.recording_pattern == Some(data.pattern_id) {
        for note in &view_state.recording_preview_completed {
            if note.pitch < pitch_min {
                pitch_min = note.pitch;
            }
            if note.pitch > pitch_max {
                pitch_max = note.pitch;
            }
        }
        for (pitch, _) in &view_state.recording_preview_held {
            if *pitch < pitch_min {
                pitch_min = *pitch;
            }
            if *pitch > pitch_max {
                pitch_max = *pitch;
            }
        }
    }

    let margin = 6;
    let view_pitch_min = pitch_min.saturating_sub(margin);
    let view_pitch_max = pitch_max.saturating_add(margin);
    let pitch_range = view_pitch_max.as_midi() - view_pitch_min.as_midi() + 1;

    // Zoom-scaled grid units (1.0 zoom = default constants).
    let note_row_height = DEFAULT_NOTE_ROW_HEIGHT * view_state.pr_zoom_y;
    let pr_pixels_per_beat = DEFAULT_PR_PIXELS_PER_BEAT * view_state.pr_zoom_x;

    let grid_height = pitch_range as f32 * note_row_height;
    // Every automation lane gets its own stacked zone below the velocity zone.
    let auto_zone_targets = displayed_automation_targets(data, view_state);
    let auto_height = auto_zone_targets.len() as f32 * AUTOMATION_ZONE_HEIGHT;
    let total_content_height = grid_height + VELOCITY_ZONE_HEIGHT + auto_height;

    // Timeline width: use max of pattern length and furthest note end
    let ticks_per_beat = synth_sequencer::TICKS_PER_QUARTER;
    let max_note_end = data
        .notes
        .iter()
        .filter_map(|n| n.end_tick)
        .max()
        .map_or(0, |t| t.0);
    let effective_ticks = data.length_ticks.0.max(max_note_end);
    let beats_in_pattern = if ticks_per_beat > 0 {
        effective_ticks as f32 / ticks_per_beat as f32
    } else {
        4.0
    };
    let grid_width = (beats_in_pattern * pr_pixels_per_beat).max(200.0);

    // ── Pinned gutter + ruler, scrollable grid ──
    // The keyboard column (fixed left) and the bar-number ruler (fixed top) live
    // in their own pinned strips so they never scroll out of view; only the note
    // grid scrolls. Each strip mirrors the grid ScrollArea's offset on its own
    // axis (keyboard ← offset.y, ruler ← offset.x) so it stays locked to the song
    // position. Mirrors the arrangement's pinned track-header column.
    let pr_scroll_salt = "piano_roll_scroll";
    let pr_scroll_id = super::scroll_state_id(ui, pr_scroll_salt);

    // Auto-follow playhead during playback: pre-set the horizontal offset before
    // the grid ScrollArea reads it. Grid content x of tick 0 is 0 now that the
    // keyboard column lives outside the scroll area, and the visible width is the
    // grid viewport (panel width minus the pinned keyboard column).
    if is_playing
        && view_state.auto_follow_playhead
        && let Some(pt) = playhead_tick
        && ticks_per_beat > 0
    {
        let playhead_beats = pt.0 as f32 / ticks_per_beat as f32;
        let playhead_x = playhead_beats * pr_pixels_per_beat;
        let visible_width = (ui.available_width() - KEY_WIDTH).max(1.0);
        let target_offset = (playhead_x - visible_width * 0.5).max(0.0);

        if let Some(mut scroll_state) = egui::scroll_area::State::load(ui.ctx(), pr_scroll_id) {
            scroll_state.offset.x = target_offset;
            scroll_state.store(ui.ctx(), pr_scroll_id);
            view_state.pr_last_auto_scroll_offset = Some(target_offset);
        }
    }

    // The offset the grid will use this frame — read after auto-follow set it, so
    // the pinned strips track playback without a frame of lag.
    let pr_offset = egui::scroll_area::State::load(ui.ctx(), pr_scroll_id)
        .map(|s| s.offset)
        .unwrap_or_default();
    let selected_auto = view_state.selected_automation.clone();

    // Ghost-preview notes (note-processor expansion), computed before the painter
    // so the cache update's mutable borrow of `view_state` ends here.
    let ghost_notes = if view_state.show_note_fx_ghosts {
        view_state.ghost_notes(song, data.pattern_id)
    } else {
        Vec::new()
    };

    // Pinned keyboard column (fixed left edge, mirrors vertical scroll). No
    // frame margin so the key rows line up pixel-exactly with the grid rows.
    egui::Panel::left("pr_keyboard_gutter")
        .exact_size(KEY_WIDTH)
        .resizable(false)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            draw_pr_keyboard_gutter(
                ui,
                pr_offset.y,
                view_pitch_min,
                view_pitch_max,
                note_row_height,
                grid_height,
                &auto_zone_targets,
                selected_auto.as_ref(),
            );
        });

    // Pinned bar-number ruler (fixed top edge, mirrors horizontal scroll). No
    // frame margin so the bar labels line up pixel-exactly with the grid.
    egui::Panel::top("pr_ruler")
        .exact_size(RULER_HEIGHT)
        .resizable(false)
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            draw_pr_ruler_strip(
                ui,
                pr_offset.x,
                pr_pixels_per_beat,
                ticks_per_beat,
                data,
                effective_ticks,
                playhead_tick,
            );
        });

    let scroll_output = egui::ScrollArea::both()
        .id_salt(pr_scroll_salt)
        .auto_shrink([false, false])
        .scroll_source(egui::scroll_area::ScrollSource {
            scroll_bar: true,
            // egui 0.35: `drag` is now `DragScroll`; `Never` == old `false`.
            // Don't steal drag events — we handle them for note editing.
            drag: egui::scroll_area::DragScroll::Never,
            mouse_wheel: true,
        })
        .show(ui, |ui| {
            let mut ctx =
                PianoRollCtx::new(data, song, view_state, handle, undo_manager, instruments);
            draw_piano_roll_grid(
                &mut ctx,
                ui,
                playhead_tick,
                view_pitch_min,
                view_pitch_max,
                note_row_height,
                pr_pixels_per_beat,
                grid_height,
                grid_width,
                total_content_height,
                ticks_per_beat,
                beats_in_pattern,
                &ghost_notes,
            );
        });

    // Detect manual scrolling to disable auto-follow, mirroring the
    // arrangement timeline: if the offset after the scroll area differs from
    // what auto-follow set, the user dragged the scrollbar — stop fighting.
    if is_playing {
        if let Some(expected) = view_state.pr_last_auto_scroll_offset
            && super::user_scrolled_away(&scroll_output, expected)
        {
            view_state.auto_follow_playhead = false;
            view_state.pr_last_auto_scroll_offset = None;
        }
    } else {
        view_state.pr_last_auto_scroll_offset = None;
    }

    {
        let mut ctx = PianoRollCtx::new(data, song, view_state, handle, undo_manager, instruments);
        handle_piano_roll_shortcuts(&mut ctx, ui, is_playing, playhead_tick);
    }

    keep_open
}

/// Draw the pinned keyboard column — a fixed-x left strip that mirrors the grid
/// `ScrollArea`'s vertical `offset_y` so the piano keys stay aligned with their
/// note rows no matter how far the grid is scrolled sideways. Also paints the
/// "VEL" / "AUTO" gutter tags at their zone positions. Counterpart of the
/// arrangement's pinned track-header column.
#[allow(clippy::too_many_arguments)]
fn draw_pr_keyboard_gutter(
    ui: &mut egui::Ui,
    offset_y: f32,
    view_pitch_min: Pitch,
    view_pitch_max: Pitch,
    note_row_height: f32,
    grid_height: f32,
    auto_zone_targets: &[AutomationTarget],
    selected_automation: Option<&AutomationTarget>,
) {
    let t = theme();
    let area = ui.max_rect();
    let corner = Rect::from_min_size(area.min, Vec2::new(area.width(), RULER_HEIGHT));
    let gutter = Rect::from_min_max(Pos2::new(area.left(), area.top() + RULER_HEIGHT), area.max);

    // Corner cell + gutter background.
    let painter = ui.painter().with_clip_rect(area);
    painter.rect_filled(corner, 0.0, t.colors.bg_dark);
    painter.rect_filled(gutter, 0.0, t.colors.bg_dark);

    // Keys are clipped to the gutter so a row scrolled up under the corner does
    // not bleed over it.
    let gp = ui.painter().with_clip_rect(gutter);
    // Screen y of the top visible grid row (`view_pitch_max`), scrolled by the
    // grid's vertical offset — the gutter analogue of `PianoRollCoords::grid_y`.
    let row0_y = gutter.top() - offset_y;

    for p in view_pitch_min.as_midi()..=view_pitch_max.as_midi() {
        let row = view_pitch_max.as_midi().saturating_sub(p);
        let y = row0_y + f32::from(row) * note_row_height;
        if y + note_row_height < gutter.top() || y > gutter.bottom() {
            continue;
        }
        let is_black = NoteName::from_midi(p % 12).is_black_key();
        let key_color = if is_black {
            PIANO_KEY_BLACK
        } else {
            PIANO_KEY_WHITE
        };
        gp.rect_filled(
            Rect::from_min_size(
                Pos2::new(area.left(), y),
                Vec2::new(area.width(), note_row_height),
            ),
            0.0,
            key_color,
        );
        if p % 12 == 0 {
            let octave = (p / 12) as i8 - 1;
            gp.text(
                Pos2::new(area.left() + 4.0, y + 1.0),
                egui::Align2::LEFT_TOP,
                format!("C{octave}"),
                egui::FontId::proportional(10.0),
                t.colors.text_primary,
            );
        }
        gp.line_segment(
            [
                Pos2::new(area.left(), y + note_row_height),
                Pos2::new(area.right(), y + note_row_height),
            ],
            Stroke::new(0.5, t.colors.border.gamma_multiply(0.3)),
        );
    }

    // "VEL" tag over the velocity zone, and "AUTO" over the focused automation
    // lane — the pinned gutter counterparts of the tags the zones used to draw.
    let vel_y = row0_y + grid_height;
    gp.text(
        Pos2::new(area.left() + 2.0, vel_y + 2.0),
        egui::Align2::LEFT_TOP,
        "VEL",
        egui::FontId::proportional(9.0),
        t.colors.text_dim,
    );
    let auto_base_y = vel_y + VELOCITY_ZONE_HEIGHT;
    for (i, target) in auto_zone_targets.iter().enumerate() {
        let zone_y = auto_base_y + i as f32 * AUTOMATION_ZONE_HEIGHT;
        // Every lane gets an "AUT" tag plus its short type below it (the pinned
        // gutter counterpart of the "VEL" tag). The edit-focused lane is drawn
        // brighter so it stands out.
        let focused = selected_automation == Some(target);
        let label_color = if focused {
            t.colors.text_secondary
        } else {
            t.colors.text_dim
        };
        gp.text(
            Pos2::new(area.left() + 2.0, zone_y + 2.0),
            egui::Align2::LEFT_TOP,
            "AUT",
            egui::FontId::proportional(9.0),
            label_color,
        );
        // Short type (the parameter name) below the tag — clipped to the gutter.
        gp.text(
            Pos2::new(area.left() + 2.0, zone_y + 13.0),
            egui::Align2::LEFT_TOP,
            target.short_name(),
            egui::FontId::proportional(8.0),
            label_color.gamma_multiply(0.85),
        );
    }

    // Right edge of the gutter (the old keyboard/grid separator).
    painter.line_segment(
        [
            Pos2::new(area.right(), area.top()),
            Pos2::new(area.right(), area.bottom()),
        ],
        Stroke::new(1.0, t.colors.border),
    );
}

/// Draw the pinned bar-number ruler — a fixed-y top strip that mirrors the grid
/// `ScrollArea`'s horizontal `offset_x` so bar numbers track the song position,
/// and paints the playhead triangle marker. Never scrolls out of view.
fn draw_pr_ruler_strip(
    ui: &mut egui::Ui,
    offset_x: f32,
    pr_pixels_per_beat: f32,
    ticks_per_beat: u32,
    data: &PianoRollData,
    effective_ticks: u32,
    playhead_tick: Option<PatternTick>,
) {
    let t = theme();
    let area = ui.max_rect();
    let painter = ui.painter().with_clip_rect(area);

    let ticks_per_bar = u64::from(data.time_sig.ticks_per_bar().max(1));
    let total_bars = effective_ticks
        .div_ceil(data.time_sig.ticks_per_bar().max(1))
        .max(1);
    // Screen x of a content tick: the grid's left edge minus the horizontal
    // scroll offset, plus the tick's grid position.
    let tick_to_x = |tick: u64| {
        if ticks_per_beat == 0 {
            area.left() - offset_x
        } else {
            area.left() - offset_x + (tick as f32 / ticks_per_beat as f32) * pr_pixels_per_beat
        }
    };
    // `draw_ruler_labels` fills the strip background before painting the labels.
    draw_ruler_labels(&painter, &t, area, total_bars, ticks_per_bar, tick_to_x);

    // Ruler bottom border.
    painter.line_segment(
        [
            Pos2::new(area.left(), area.bottom()),
            Pos2::new(area.right(), area.bottom()),
        ],
        Stroke::new(1.0, t.colors.border),
    );

    // Playhead triangle marker (matches the grid's vertical playhead line).
    if let Some(pt) = playhead_tick
        && ticks_per_beat > 0
    {
        let x = area.left() - offset_x + (pt.0 as f32 / ticks_per_beat as f32) * pr_pixels_per_beat;
        if x >= area.left() && x <= area.right() {
            let tri = 6.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(x - tri, area.top()),
                    Pos2::new(x + tri, area.top()),
                    Pos2::new(x, area.top() + RULER_HEIGHT * 0.6),
                ],
                t.colors.accent_primary,
                Stroke::NONE,
            ));
        }
    }
}

/// Draw the piano-roll note grid (grid lines, notes, velocity and automation
/// zones) and dispatch its pointer interaction — the body of the note-grid
/// `ScrollArea`, split out of [`draw_piano_roll`]. The keyboard column and bar
/// ruler are drawn separately by the pinned strips. Builds its `PianoRollCoords`
/// from the painter rect and delegates editing to
/// [`handle_piano_roll_interaction`].
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn draw_piano_roll_grid(
    ctx: &mut PianoRollCtx<'_>,
    ui: &mut egui::Ui,
    playhead_tick: Option<PatternTick>,
    view_pitch_min: Pitch,
    view_pitch_max: Pitch,
    note_row_height: f32,
    pr_pixels_per_beat: f32,
    grid_height: f32,
    grid_width: f32,
    total_content_height: f32,
    ticks_per_beat: u32,
    beats_in_pattern: f32,
    ghost_notes: &[GhostNote],
) {
    let data = ctx.data;
    let song = ctx.song;
    let view_state = &mut *ctx.view_state;
    let handle = &mut *ctx.handle;
    let undo_manager = &mut *ctx.undo_manager;
    let instruments = ctx.instruments;
    let t = theme();

    let total_size = Vec2::new(grid_width, total_content_height);

    // Use allocate_rect with click_and_drag sense for mouse interaction
    let alloc_rect = Rect::from_min_size(ui.cursor().min, total_size);
    let response = ui.allocate_rect(alloc_rect, Sense::click_and_drag());
    // Expose the canvas container to AccessKit / the egui-inspection MCP. Per-note
    // drivability is out of scope for v1 — this makes the canvas locatable.
    expose(
        &response,
        egui::WidgetType::Other,
        "piano roll canvas",
        None,
    );
    let rect = response.rect;
    let painter = ui.painter_at(rect);

    // ── Ctrl+scroll → zoom (consumed before scroll area sees it) ──
    if response.hovered() {
        let (scroll_dy, ctrl, shift) = ui.input(|i| {
            (
                i.smooth_scroll_delta.y,
                i.modifiers.ctrl || i.modifiers.command,
                i.modifiers.shift,
            )
        });
        if ctrl && scroll_dy != 0.0 {
            let factor = 1.0 + scroll_dy * 0.002;
            if shift {
                view_state.pr_zoom_y = (view_state.pr_zoom_y * factor).clamp(0.5, 3.0);
            } else {
                view_state.pr_zoom_x = (view_state.pr_zoom_x * factor).clamp(0.25, 4.0);
            }
        }
    }

    let origin = rect.min;
    // Keyboard column and bar ruler are pinned in their own strips outside this
    // scroll area, so the grid content starts at the scroll-content origin on
    // both axes (no keyboard/ruler inset here anymore).
    let grid_x = origin.x;
    let grid_y = origin.y;

    // All four grid↔(tick,pitch) transforms live on this one value (see
    // `PianoRollCoords`). The closures below just delegate so the rest of
    // this function — and the interaction handler — keep their call shape.
    let coords = PianoRollCoords {
        grid_x,
        grid_y,
        ticks_per_beat,
        pr_pixels_per_beat,
        note_row_height,
        view_pitch_min,
        view_pitch_max,
    };
    let tick_to_x = |tick_val: PatternTick| coords.tick_to_x(tick_val);
    let pitch_to_y = |pitch: Pitch| coords.pitch_to_y(pitch);

    // Grid rect for checking if pointer is in the note grid area
    let grid_rect = Rect::from_min_size(
        Pos2::new(grid_x, grid_y),
        Vec2::new(grid_width, grid_height),
    );

    // The bar-number ruler and the piano keyboard column are drawn by the pinned
    // strips in `draw_piano_roll` (`draw_pr_ruler_strip` / `draw_pr_keyboard_gutter`),
    // so this canvas paints only the scrollable grid, notes and zones.

    // ── Note grid background ──
    for p in view_pitch_min.as_midi()..=view_pitch_max.as_midi() {
        let pitch = Pitch::new(p).unwrap_or(Pitch::MIDDLE_C);
        let y = pitch_to_y(pitch);
        let note_name = NoteName::from_midi(p % 12);
        let is_black = note_name.is_black_key();
        let is_c = p % 12 == 0;

        let bg = if is_c {
            GRID_BG_C
        } else if is_black {
            GRID_BG_BLACK
        } else {
            GRID_BG_WHITE
        };

        painter.rect_filled(
            Rect::from_min_size(Pos2::new(grid_x, y), Vec2::new(grid_width, note_row_height)),
            0.0,
            bg,
        );

        // Horizontal pitch row separator
        painter.line_segment(
            [
                Pos2::new(grid_x, y + note_row_height),
                Pos2::new(grid_x + grid_width, y + note_row_height),
            ],
            Stroke::new(
                if is_c { 0.8 } else { 0.3 },
                t.colors.border.gamma_multiply(if is_c { 0.6 } else { 0.2 }),
            ),
        );
    }

    // ── Vertical beat/sub-beat lines ──
    let beats_total = beats_in_pattern.ceil() as u32;
    let beats_per_bar_val = data.time_sig.numerator.max(1) as u32;
    for beat_idx in 0..=beats_total {
        let beat_tick = beat_idx * ticks_per_beat;
        let x = tick_to_x(PatternTick(beat_tick));
        let is_bar_line = beat_idx % beats_per_bar_val == 0;

        painter.line_segment(
            [Pos2::new(x, grid_y), Pos2::new(x, grid_y + grid_height)],
            Stroke::new(
                if is_bar_line { 1.0 } else { 0.5 },
                t.colors
                    .border
                    .gamma_multiply(if is_bar_line { 0.8 } else { 0.3 }),
            ),
        );

        // Sub-beat lines (based on ticks_per_row)
        if data.ticks_per_row > 0 && !is_bar_line {
            // Already drawing at beat level; skip sub-beats for readability
        }
    }

    // Sub-beat grid lines based on row resolution
    if data.ticks_per_row > 0 {
        let total_rows = data.length_ticks.0 / data.ticks_per_row as u32;
        for row in 0..total_rows {
            let row_tick = row * data.ticks_per_row as u32;
            // Skip if this aligns with a beat line (already drawn)
            if row_tick.is_multiple_of(ticks_per_beat) {
                continue;
            }
            let x = tick_to_x(PatternTick(row_tick));
            painter.line_segment(
                [Pos2::new(x, grid_y), Pos2::new(x, grid_y + grid_height)],
                Stroke::new(0.3, t.colors.border.gamma_multiply(0.15)),
            );
        }
    }

    // ── Notes ──
    let default_note_color = DEFAULT_NOTE_BLUE;
    // One-shot per-frame colour cache so each note's lookup is O(1).
    let note_color_cache = build_instrument_colour_cache(instruments);

    // Ghost-preview overlay: the note-processor expansion, painted faintly
    // behind the source notes so the user sees what actually plays.
    // Read-only, non-interactive.
    for ghost in ghost_notes {
        if ghost.pitch < view_pitch_min || ghost.pitch > view_pitch_max {
            continue;
        }
        let gy = pitch_to_y(ghost.pitch);
        let gx_start = tick_to_x(ghost.start);
        let gx_end = match ghost.duration {
            Some(d) => tick_to_x(PatternTick(ghost.start.0.saturating_add(d.0))),
            None => tick_to_x(data.length_ticks.as_pattern_tick()).min(gx_start + grid_width),
        };
        let gw = (gx_end - gx_start).max(2.0);
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(gx_start, gy + 1.0),
                Vec2::new(gw, note_row_height - 2.0),
            ),
            2.0,
            GHOST_NOTE_COLOR,
        );
    }

    for note in &data.notes {
        if note.pitch < view_pitch_min || note.pitch > view_pitch_max {
            continue;
        }

        // Skip notes that are being dragged (draw ghost instead)
        let is_being_moved = matches!(
            &view_state.drag,
            Some(DragState::MoveNote { note_id, .. }) if *note_id == note.note_id
        );
        if is_being_moved {
            continue;
        }

        let y = pitch_to_y(note.pitch);
        let x_start = tick_to_x(note.start_tick);

        let x_end = match note.end_tick {
            Some(end) => tick_to_x(end),
            None => {
                // Open-ended: draw to pattern end or at least a visible width
                tick_to_x(data.length_ticks.as_pattern_tick()).min(x_start + grid_width)
            }
        };

        // Apply resize preview if this note is being resized
        let x_end = match &view_state.drag {
            Some(DragState::ResizeNote {
                note_id,
                current_end_tick,
                ..
            }) if *note_id == note.note_id => tick_to_x(*current_end_tick),
            _ => x_end,
        };

        let note_width = (x_end - x_start).max(3.0);
        let alpha = (note.velocity.as_f32() * 200.0 + 55.0).min(255.0) as u8;

        // Notes have no per-note instrument; colour by the instrument
        // the pattern plays through (its placement's track), falling
        // back to the working instrument for an unplaced pattern.
        let note_instrument = data
            .track_overrides
            .first()
            .copied()
            .unwrap_or(view_state.selected_instrument);
        let inst_color =
            cached_instrument_color(&note_color_cache, note_instrument, default_note_color);

        let is_selected = view_state.selected_notes.contains(&note.note_id);

        let fill =
            Color32::from_rgba_unmultiplied(inst_color.r(), inst_color.g(), inst_color.b(), alpha);

        let note_rect = Rect::from_min_size(
            Pos2::new(x_start, y + 1.0),
            Vec2::new(note_width, note_row_height - 2.0),
        );

        if is_selected {
            // Soft glow halo behind the note (cyan tint).
            let glow_rect = note_rect.expand(3.0);
            painter.rect_filled(glow_rect, 3.0, NOTE_SELECTED_GLOW);
        }

        painter.rect_filled(note_rect, 2.0, fill);

        if is_selected {
            // High-contrast outer outline (white) — clearly visible
            // against any instrument colour or grid background.
            painter.rect_stroke(
                note_rect.expand(1.0),
                3.0,
                Stroke::new(2.0, Color32::WHITE),
                egui::StrokeKind::Outside,
            );
            // Cyan inner accent for a recognisable selection colour.
            painter.rect_stroke(
                note_rect,
                2.0,
                Stroke::new(1.0, t.colors.accent_cyan),
                egui::StrokeKind::Inside,
            );
        } else {
            painter.rect_stroke(
                note_rect,
                2.0,
                Stroke::new(0.5, inst_color),
                egui::StrokeKind::Inside,
            );
        }

        // Per-note expression markers (B4): a tie underline for legato
        // and a small ramp glyph at the left edge for glide.
        if note.legato {
            let y_line = note_rect.max.y - 1.0;
            painter.line_segment(
                [
                    Pos2::new(note_rect.min.x + 1.0, y_line),
                    Pos2::new(note_rect.max.x - 1.0, y_line),
                ],
                Stroke::new(1.5, t.colors.accent_yellow),
            );
        }
        if note.glide.is_some() {
            // Diagonal ramp from bottom-left up into the note's leading edge.
            let gx = note_rect.min.x;
            painter.line_segment(
                [
                    Pos2::new(gx + 0.5, note_rect.max.y - 1.0),
                    Pos2::new((gx + 6.0).min(note_rect.max.x), note_rect.min.y + 1.0),
                ],
                Stroke::new(1.5, t.colors.accent_cyan),
            );
        }
        // Per-note expression (C5): a small dot at the top-right corner.
        if note.expression.is_some() && note_width > 5.0 {
            painter.circle_filled(
                Pos2::new(note_rect.max.x - 2.5, note_rect.min.y + 2.5),
                1.5,
                t.colors.accent_yellow,
            );
        }
        // Per-note ornament (NP6): a small marker at the top-left corner,
        // in the Note FX accent colour.
        if note.ornament.is_some() && note_width > 5.0 {
            painter.circle_filled(
                Pos2::new(note_rect.min.x + 2.5, note_rect.min.y + 2.5),
                1.5,
                t.colors.accent_purple,
            );
        }

        // Open-ended indicator (gradient fade-out at right edge)
        if note.end_tick.is_none() && note_width > 8.0 {
            let fade_rect = Rect::from_min_max(
                Pos2::new(note_rect.max.x - 6.0, note_rect.min.y),
                note_rect.max,
            );
            painter.rect_filled(
                fade_rect,
                0.0,
                Color32::from_rgba_unmultiplied(
                    inst_color.r(),
                    inst_color.g(),
                    inst_color.b(),
                    alpha / 3,
                ),
            );
        }
    }

    // ── Recording preview notes (orange) ──
    if !view_state.recording_preview_completed.is_empty()
        || !view_state.recording_preview_held.is_empty()
    {
        let preview_color = RECORDING_PREVIEW_ORANGE;

        // Draw completed preview notes
        for note in &view_state.recording_preview_completed {
            if note.pitch < view_pitch_min || note.pitch > view_pitch_max {
                continue;
            }
            let y = pitch_to_y(note.pitch);
            let x_start = tick_to_x(note.start);
            let x_end = tick_to_x(note.start + note.duration);
            let note_width = (x_end - x_start).max(3.0);
            let alpha = 180_u8;

            let preview_rect = Rect::from_min_size(
                Pos2::new(x_start, y + 1.0),
                Vec2::new(note_width, note_row_height - 2.0),
            );
            painter.rect_filled(
                preview_rect,
                2.0,
                Color32::from_rgba_unmultiplied(
                    preview_color.r(),
                    preview_color.g(),
                    preview_color.b(),
                    alpha,
                ),
            );
            painter.rect_stroke(
                preview_rect,
                2.0,
                Stroke::new(0.5, preview_color),
                egui::StrokeKind::Inside,
            );
        }

        // Draw held notes extending from start to current playhead
        if !view_state.recording_preview_held.is_empty() {
            // Compute playhead position within pattern
            let playhead_in_pattern = playhead_tick.map_or(0, |pt| pt.0);

            for (pitch, start_tick) in &view_state.recording_preview_held {
                if *pitch < view_pitch_min || *pitch > view_pitch_max {
                    continue;
                }
                let y = pitch_to_y(*pitch);
                let x_start = tick_to_x(*start_tick);
                let end = if playhead_in_pattern >= start_tick.0 {
                    PatternTick(playhead_in_pattern)
                } else {
                    // Note wraps around pattern — draw to end
                    data.length_ticks.as_pattern_tick()
                };
                let x_end = tick_to_x(end);
                let note_width = (x_end - x_start).max(3.0);

                let held_rect = Rect::from_min_size(
                    Pos2::new(x_start, y + 1.0),
                    Vec2::new(note_width, note_row_height - 2.0),
                );
                painter.rect_filled(held_rect, 2.0, RECORDING_PREVIEW_HELD_FILL);
                painter.rect_stroke(
                    held_rect,
                    2.0,
                    Stroke::new(0.5, preview_color),
                    egui::StrokeKind::Inside,
                );
            }
        }

        // Request repaint during recording for live updates
        ui.request_repaint();
    }

    // ── Ghost note for MoveNote drag ──
    if let Some(DragState::MoveNote {
        note_id,
        current_tick: drag_tick,
        current_pitch: drag_pitch,
        ..
    }) = &view_state.drag
    {
        // Find the original note data for velocity/duration
        if let Some(note) = data.notes.iter().find(|n| n.note_id == *note_id) {
            let duration_ticks = note.end_tick.map_or(
                data.length_ticks.as_pattern_tick() - note.start_tick,
                |end| end - note.start_tick,
            );
            let y = pitch_to_y(*drag_pitch);
            let x_start = tick_to_x(*drag_tick);
            let x_end = tick_to_x(*drag_tick + duration_ticks);
            let note_width = (x_end - x_start).max(3.0);

            let ghost_rect = Rect::from_min_size(
                Pos2::new(x_start, y + 1.0),
                Vec2::new(note_width, note_row_height - 2.0),
            );

            // Semi-transparent ghost
            painter.rect_filled(ghost_rect, 2.0, MOVE_GHOST_FILL);
            painter.rect_stroke(
                ghost_rect,
                2.0,
                Stroke::new(1.0, MOVE_GHOST_STROKE),
                egui::StrokeKind::Inside,
            );
        }
    }

    // ── DrawNote preview ──
    if let Some(DragState::DrawNote {
        start_tick,
        pitch,
        current_end_tick,
    }) = &view_state.drag
    {
        let y = pitch_to_y(*pitch);
        let x_start = tick_to_x(*start_tick);
        let x_end = tick_to_x(*current_end_tick);
        let note_width = (x_end - x_start).max(3.0);

        let draw_rect = Rect::from_min_size(
            Pos2::new(x_start, y + 1.0),
            Vec2::new(note_width, note_row_height - 2.0),
        );
        painter.rect_filled(draw_rect, 2.0, DRAW_NOTE_FILL);
        painter.rect_stroke(
            draw_rect,
            2.0,
            Stroke::new(1.0, DRAW_NOTE_STROKE),
            egui::StrokeKind::Inside,
        );
    }

    // ── Selection rectangle ──
    if let Some(DragState::SelectRect {
        start_pos,
        current_pos,
    }) = &view_state.drag
    {
        let sel_rect = Rect::from_two_pos(*start_pos, *current_pos);
        painter.rect_filled(sel_rect, 0.0, SELECTION_RECT_FILL);
        painter.rect_stroke(
            sel_rect,
            0.0,
            Stroke::new(1.0, SELECTION_RECT_STROKE),
            egui::StrokeKind::Inside,
        );
    }

    // ── Velocity bars (below grid) ──
    let vel_y = grid_y + grid_height;
    // Background
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(grid_x, vel_y),
            Vec2::new(grid_width, VELOCITY_ZONE_HEIGHT),
        ),
        0.0,
        VELOCITY_ZONE_BG,
    );

    // Separator line
    painter.line_segment(
        [
            Pos2::new(grid_x, vel_y),
            Pos2::new(grid_x + grid_width, vel_y),
        ],
        Stroke::new(1.0, t.colors.border),
    );

    // (The "VEL" gutter tag is drawn by the pinned keyboard strip.)

    // Velocity bars
    for note in &data.notes {
        let x = tick_to_x(note.start_tick);
        let bar_height = note.velocity.as_f32() * (VELOCITY_ZONE_HEIGHT - 4.0);
        let bar_y = vel_y + VELOCITY_ZONE_HEIGHT - bar_height - 2.0;

        let is_selected = view_state.selected_notes.contains(&note.note_id);
        let (vel_color, bar_w) = if is_selected {
            (VELOCITY_BAR_SELECTED, 5.0)
        } else {
            (velocity_color(note.velocity.as_f32()), 3.0)
        };
        let bar_rect = Rect::from_min_size(
            Pos2::new(x - bar_w * 0.5, bar_y),
            Vec2::new(bar_w, bar_height),
        );
        painter.rect_filled(bar_rect, 1.0, vel_color);
        if is_selected {
            painter.rect_stroke(
                bar_rect,
                1.0,
                Stroke::new(1.0, Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        }
    }

    // ── Automation zones (stacked below velocity, one per lane) ──
    // Same ordered list the layout used for `auto_height` (pure helper).
    let auto_zone_targets = displayed_automation_targets(data, view_state);
    let auto_height = auto_zone_targets.len() as f32 * AUTOMATION_ZONE_HEIGHT;
    let auto_base_y = vel_y + VELOCITY_ZONE_HEIGHT;
    // Left edge of the visible scroll viewport, so each lane can pin its
    // description there and keep it on screen regardless of horizontal scroll.
    let viewport_left = ui.clip_rect().min.x;
    for (i, target) in auto_zone_targets.iter().enumerate() {
        let zone_y = auto_base_y + i as f32 * AUTOMATION_ZONE_HEIGHT;
        let is_selected = view_state.selected_automation.as_ref() == Some(target);
        draw_automation_zone(
            &painter,
            data,
            view_state,
            target,
            grid_x,
            zone_y,
            grid_width,
            viewport_left,
            &tick_to_x,
            &t,
            is_selected,
        );
    }
    // The edit-focused lane's zone top — the interaction handler resolves
    // pointer Y against every band, but this is the y the tools draw at.
    let selected_zone_index = view_state
        .selected_automation
        .as_ref()
        .and_then(|sel| auto_zone_targets.iter().position(|t| t == sel));
    let auto_y = selected_zone_index.map_or(auto_base_y, |i| {
        auto_base_y + i as f32 * AUTOMATION_ZONE_HEIGHT
    });

    // ── Playhead (only if this pattern is actually playing) ──
    if let Some(pattern_tick) = playhead_tick {
        let playhead_x = tick_to_x(pattern_tick);

        if playhead_x >= grid_x && playhead_x <= grid_x + grid_width {
            // Vertical line spanning the grid + velocity/automation zones. The
            // matching ruler triangle marker is drawn by `draw_pr_ruler_strip`.
            painter.line_segment(
                [
                    Pos2::new(playhead_x, grid_y),
                    Pos2::new(playhead_x, grid_y + total_content_height),
                ],
                Stroke::new(1.5, t.colors.accent_primary),
            );
        }
    }

    // ── Step cursor ──
    if view_state.step_entry_mode {
        let cursor_x = tick_to_x(view_state.step_cursor_tick);
        if cursor_x >= grid_x && cursor_x <= grid_x + grid_width {
            painter.line_segment(
                [
                    Pos2::new(cursor_x, grid_y),
                    Pos2::new(cursor_x, grid_y + grid_height),
                ],
                Stroke::new(2.0, STEP_CURSOR),
            );
        }
    }

    // (The keyboard/grid separator is now the pinned keyboard strip's edge.)

    // Hit-test rect for the edit-focused lane's zone (at its stacked y).
    let auto_rect = selected_zone_index.map(|_| {
        Rect::from_min_size(
            Pos2::new(grid_x, auto_y),
            Vec2::new(grid_width, AUTOMATION_ZONE_HEIGHT),
        )
    });
    // Full stacked-zones band, for resolving focus-click on any lane.
    let auto_stack_rect = (!auto_zone_targets.is_empty()).then(|| {
        Rect::from_min_size(
            Pos2::new(grid_x, auto_base_y),
            Vec2::new(grid_width, auto_height),
        )
    });

    // ── Hover and cursor ──
    if let Some(pos) = ui.ctx().pointer_hover_pos() {
        // Crosshair over any stacked automation zone.
        if auto_stack_rect.is_some_and(|r| r.contains(pos)) {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        } else if grid_rect.contains(pos) {
            let vp_min = view_pitch_min;
            let vp_max = view_pitch_max;
            let hit = note_at_pos(
                &data.notes,
                pos,
                &tick_to_x,
                &pitch_to_y,
                data.length_ticks,
                vp_min,
                vp_max,
                note_row_height,
            );

            match hit {
                Some((_, HitZone::RightEdge)) => {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeEast);
                }
                Some((note_id, HitZone::Body)) => {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);

                    // Subtle hover highlight
                    if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                        let y = pitch_to_y(note.pitch);
                        let x_start = tick_to_x(note.start_tick);
                        let x_end = match note.end_tick {
                            Some(end) => tick_to_x(end),
                            None => tick_to_x(data.length_ticks.as_pattern_tick()),
                        };
                        let hover_rect = Rect::from_min_size(
                            Pos2::new(x_start, y + 1.0),
                            Vec2::new((x_end - x_start).max(3.0), note_row_height - 2.0),
                        );
                        painter.rect_stroke(
                            hover_rect,
                            2.0,
                            Stroke::new(1.0, NOTE_HOVER_OUTLINE),
                            egui::StrokeKind::Outside,
                        );

                        let midi = note.pitch.as_midi();
                        let pitch_name = NoteName::from_midi(midi % 12);
                        let octave = (midi / 12) as i8 - 1;
                        let beats =
                            note.start_tick.0 as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;
                        let (bar0, beat0, tick0) =
                            Tick(note.start_tick.0 as u64).to_bar_beat_tick(data.time_sig);
                        let bar = bar0 + 1;
                        let beat_in_bar = beat0 as f32
                            + tick0 as f32 / synth_sequencer::TICKS_PER_QUARTER as f32
                            + 1.0;
                        let length_text = match note.end_tick {
                            Some(end) => {
                                let dur_beats = (end.0 - note.start_tick.0) as f32
                                    / synth_sequencer::TICKS_PER_QUARTER as f32;
                                format!("{dur_beats:.2} beats")
                            }
                            None => "open".to_owned(),
                        };
                        let vel_pct = (note.velocity.as_f32() * 100.0).round();
                        // Notes route through their placement's track instrument.
                        let note_instrument =
                            data.track_overrides.first().copied().unwrap_or_default();
                        let inst_name = instruments
                            .iter()
                            .find(|inst| inst.id == note_instrument)
                            .map_or_else(
                                || format!("#{}", note_instrument.as_u64()),
                                |inst| inst.name.clone(),
                            );
                        let tooltip_id = ui.id().with(("note_tip", note_id.0));
                        egui::Tooltip::always_open(
                            ui.ctx().clone(),
                            ui.layer_id(),
                            tooltip_id,
                            pos,
                        )
                        .at_pointer()
                        .show(|ui| {
                            ui.label(
                                RichText::new(format!("{pitch_name:?}{octave}  (MIDI {midi})"))
                                    .strong(),
                            );
                            ui.label(format!(
                                "Bar {bar}, beat {beat_in_bar:.2}  ({beats:.2} beats from start)"
                            ));
                            ui.label(format!("Length: {length_text}"));
                            ui.label(format!("Velocity: {vel_pct}%"));
                            ui.label(format!("Instrument: {inst_name}"));
                        });
                    }
                }
                None => {
                    if view_state.edit_tool == EditTool::Draw {
                        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                    }
                }
            }
        }
    }

    // ── Mouse interaction ──
    let mut ctx = PianoRollCtx::new(data, song, view_state, handle, undo_manager, instruments);
    handle_piano_roll_interaction(
        &mut ctx,
        &response,
        ui,
        grid_rect,
        auto_rect,
        auto_y,
        auto_base_y,
        &auto_zone_targets,
        &coords,
    );
}

/// Handle mouse clicks and drags in the piano roll.
#[allow(clippy::too_many_arguments)]
fn handle_piano_roll_shortcuts(
    ctx: &mut PianoRollCtx<'_>,
    ui: &mut egui::Ui,
    is_playing: bool,
    playhead_tick: Option<PatternTick>,
) {
    let data = ctx.data;
    let song = ctx.song;
    let view_state = &mut *ctx.view_state;
    let handle = &mut *ctx.handle;
    let undo_manager = &mut *ctx.undo_manager;

    // ── Keyboard shortcuts ──
    let egui_ctx = ui.ctx();
    if egui_ctx.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
        delete_selected_notes(
            song,
            data.pattern_id,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }
    if egui_ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        view_state.selected_notes.clear();
        view_state.drag = None;
        view_state.step_entry_mode = false;
    }

    // ── Ctrl+A — select all notes ──
    if egui_ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
        view_state.selected_notes.clear();
        for note in &data.notes {
            view_state.selected_notes.insert(note.note_id);
        }
    }

    // ── Ctrl+C — copy selected notes ──
    if egui_ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::C))
        && !view_state.selected_notes.is_empty()
    {
        copy_selected_notes(data, &view_state.selected_notes, &mut view_state.clipboard);
    }

    // ── Ctrl+X — cut selected notes ──
    if egui_ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::X))
        && !view_state.selected_notes.is_empty()
    {
        copy_selected_notes(data, &view_state.selected_notes, &mut view_state.clipboard);
        delete_selected_notes(
            song,
            data.pattern_id,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }

    // ── Ctrl+V — paste at playhead or start ──
    if egui_ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::V))
        && !view_state.clipboard.notes.is_empty()
    {
        let paste_tick = playhead_tick.unwrap_or(PatternTick(0));
        paste_clipboard_notes(
            song,
            data.pattern_id,
            &view_state.clipboard,
            paste_tick,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }

    // ── Ctrl+D — duplicate selected notes ──
    if egui_ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::D))
        && !view_state.selected_notes.is_empty()
    {
        copy_selected_notes(data, &view_state.selected_notes, &mut view_state.clipboard);
        // Find min tick of selection
        let min_tick = data
            .notes
            .iter()
            .filter(|n| view_state.selected_notes.contains(&n.note_id))
            .map(|n| n.start_tick)
            .min()
            .unwrap_or(PatternTick::ZERO);
        let paste_tick = min_tick + view_state.clipboard.selection_width;
        paste_clipboard_notes(
            song,
            data.pattern_id,
            &view_state.clipboard,
            paste_tick,
            &mut view_state.selected_notes,
            undo_manager,
        );
    }

    // ── Arrow Up/Down — transpose selected notes ──
    if !view_state.selected_notes.is_empty() {
        let shift = egui_ctx.input(|i| i.modifiers.shift);
        let up = egui_ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
        let down = egui_ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
        if up || down {
            let semitones = match (up, shift) {
                (true, false) => Semitones::new(1.0),
                (true, true) => Semitones::new(12.0),
                (false, false) => Semitones::new(-1.0),
                (false, true) => Semitones::new(-12.0),
            };
            {
                let mut song_w = song.write();
                if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                    let mut composite = Vec::new();
                    for note_id in &view_state.selected_notes {
                        if let Some(note) = pattern.note(*note_id) {
                            let old_pitch = note.pitch;
                            if pattern.transpose_note(*note_id, semitones)
                                && let Some(transposed) = pattern.note(*note_id)
                            {
                                composite.push(crate::undo::UndoAction::TransposeNote {
                                    pattern_id: data.pattern_id,
                                    note_id: *note_id,
                                    old_pitch,
                                    new_pitch: transposed.pitch,
                                });
                            }
                        }
                    }
                    if !composite.is_empty() {
                        undo_manager.push(crate::undo::UndoAction::Composite(composite));
                    }
                }
            }
        }
    }

    // ── Space — toggle play/pause ──
    if egui_ctx.input(|i| i.key_pressed(egui::Key::Space)) {
        if is_playing {
            handle.send(EngineCommand::Pause);
        } else {
            handle.send(EngineCommand::Play);
            view_state.auto_follow_playhead = true;
        }
    }

    // ── Step entry mode — keyboard piano inserts notes at cursor ──
    if view_state.step_entry_mode {
        let step_size = if view_state.draw_note_length > 0 {
            SeqDuration(view_state.draw_note_length)
        } else {
            view_state.effective_grid(data.ticks_per_row)
        };

        // Collect pressed key in a single input lock acquisition
        let pressed_note = egui_ctx.input(|i| {
            if i.modifiers.command || i.modifiers.ctrl {
                return None;
            }
            KEY_MAP
                .iter()
                .find(|(key, _)| i.key_pressed(*key))
                .map(|(_, note)| *note)
        });
        let mut inserted = false;
        if let Some(pitch_val) = pressed_note
            && let Some(pitch) = Pitch::new(pitch_val)
            && let mut song_w = song.write()
            && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
        {
            let note_id = pattern.add_note(
                view_state.step_cursor_tick,
                pitch,
                view_state.default_velocity,
            );
            pattern.resize_note(note_id, step_size);
            if let Some(note) = pattern.note(note_id) {
                undo_manager.push(crate::undo::UndoAction::AddNote {
                    pattern_id: data.pattern_id,
                    note: note.into(),
                });
            }
            view_state.selected_notes.clear();
            view_state.selected_notes.insert(note_id);
            preview_note(handle, pitch, view_state.default_velocity);
            inserted = true;
        }
        if inserted {
            view_state.step_cursor_tick = view_state.step_cursor_tick + step_size;
            // Wrap around at pattern end
            if view_state.step_cursor_tick.0 >= data.length_ticks.0 {
                view_state.step_cursor_tick = PatternTick::ZERO;
            }
        }
    }
}

fn draw_piano_roll_toolbar(
    ctx: &mut PianoRollCtx<'_>,
    ui: &mut egui::Ui,
    is_playing: bool,
    editor_mode: Option<&mut crate::gui::pattern_view::PatternEditorMode>,
) -> bool {
    let data = ctx.data;
    let song = ctx.song;
    let view_state = &mut *ctx.view_state;
    let handle = &mut *ctx.handle;
    let undo_manager = &mut *ctx.undo_manager;
    let instruments = ctx.instruments;
    let t = theme();
    let mut keep_open = true;
    // The Pattern view (Some) shows a read-only name and no description; the
    // sequencer's piano roll (None) keeps the inline rename + description editors
    // (the arrangement's "Rename Pattern" drives that inline name editor).
    let is_pattern_view = editor_mode.is_some();

    // ── Toolbar (row 1: docked context bar) ──
    crate::gui::toolbar::top(ui, "piano_roll_toolbar", |ui| {
        // Editor-mode selector. The Pattern view passes Some(editor_mode) so the
        // bar carries the Piano-roll/Tracker switch; the sequencer's bottom-panel
        // piano roll passes None (it has no mode switch).
        if let Some(mode) = editor_mode {
            ui.selectable_value(
                mode,
                crate::gui::pattern_view::PatternEditorMode::PianoRoll,
                "Piano roll",
            );
            ui.selectable_value(
                mode,
                crate::gui::pattern_view::PatternEditorMode::Tracker,
                "Tracker",
            );
            ui.separator();
        }

        // Pattern name. Read-only label in the Pattern view (rename happens there
        // via the browser's edit dialog); inline-editable in the sequencer's piano
        // roll, where the arrangement's "Rename Pattern" drives this editor.
        if is_pattern_view {
            ui.label(
                RichText::new(&data.pattern_name)
                    .size(14.0)
                    .color(t.colors.accent_cyan),
            );
        } else if view_state.editing_pattern_name.as_ref().map(|(id, _)| *id)
            == Some(data.pattern_id)
        {
            if let Some((_, ref mut name_buf)) = view_state.editing_pattern_name {
                let edit = inline_editable_text(ui, name_buf, false, |te| {
                    te.desired_width(120.0)
                        .font(egui::FontId::proportional(14.0))
                });
                if edit.ended {
                    commit_pattern_rename(song, undo_manager, data.pattern_id, name_buf.clone());
                    view_state.editing_pattern_name = None;
                }
            }
        } else {
            let name_resp = clickable_label(
                ui,
                RichText::new(&data.pattern_name)
                    .size(14.0)
                    .color(t.colors.accent_cyan),
            );
            if name_resp.double_clicked() {
                view_state.editing_pattern_name =
                    Some((data.pattern_id, data.pattern_name.clone()));
            }
        }

        // Pattern description — only in the sequencer's piano roll; the Pattern
        // view bar shows no description.
        if !is_pattern_view {
            if view_state
                .editing_pattern_description
                .as_ref()
                .map(|(id, _)| *id)
                == Some(data.pattern_id)
            {
                if let Some((_, ref mut desc_buf)) = view_state.editing_pattern_description {
                    // Commit on every change so switching patterns mid-edit can't
                    // strand the buffer; `ended` (lost focus, incl. Enter on a
                    // singleline) closes the edit session.
                    let edit = inline_editable_text(ui, desc_buf, false, |te| {
                        te.desired_width(180.0)
                            .hint_text("Description")
                            .font(egui::FontId::proportional(12.0))
                    });
                    if edit.response.changed() {
                        commit_pattern_description(song, data.pattern_id, desc_buf.clone());
                    }
                    if edit.ended {
                        view_state.editing_pattern_description = None;
                    }
                }
            } else {
                // Collapsed to an info icon so the description doesn't crowd the
                // toolbar: the text shows in the hover tooltip. Same glyph +
                // accent as the patch module header's info button. Double-click
                // still opens the inline editor.
                use egui_remixicon::icons as ri;
                let tooltip = if data.pattern_description.is_empty() {
                    "Double-click to add a pattern description".to_owned()
                } else {
                    format!("{}\n\nDouble-click to edit", data.pattern_description)
                };
                if icon_button(ui, ri::INFORMATION_LINE, t.colors.accent_primary, &tooltip)
                    .double_clicked()
                {
                    view_state.editing_pattern_description =
                        Some((data.pattern_id, data.pattern_description.clone()));
                }
            }
        }

        // Pattern length (whole bars).
        //
        // Two guards protect a pattern from being silently truncated by merely
        // *drawing* this control:
        //  1. The range's upper bound grows to fit the pattern's current length,
        //     so a pattern longer than the nominal cap (e.g. a 161-bar / 619560-
        //     tick SID import) is never clamped down. A fixed `1..=64` cap made
        //     egui clamp the shown value to 64 and report `changed()`, which wrote
        //     `64 * ticks_per_bar` (245760 ticks) straight back to the song —
        //     truncating the pattern the instant the piano roll rendered it and
        //     clobbering lengths set via MCP or project load.
        //  2. The song is written only while the user is actively editing *this*
        //     pattern's control (`pattern_length_drag_start` is armed for it), so a
        //     passive re-render's `changed()` can never mutate pattern data.
        {
            let ticks_per_bar = data.time_sig.ticks_per_bar().max(1);
            let mut bars = (data.length_ticks.0 / ticks_per_bar).max(1) as i32;
            // Never let the range clamp the pattern's own length; 256 bars keeps a
            // sane drag ceiling for short patterns (≈ the MCP 1024-beat cap).
            let max_bars = bars.max(256);
            let bars_resp = unit_drag_value(ui, &mut bars, 1..=max_bars, 0.1, " bars")
                .on_hover_text("Pattern length in bars");
            if bars_resp.drag_started() || bars_resp.gained_focus() {
                view_state.pattern_length_drag_start = Some((data.pattern_id, data.length_ticks));
            }
            let editing_this = matches!(
                view_state.pattern_length_drag_start,
                Some((pid, _)) if pid == data.pattern_id
            );
            if bars_resp.changed() && editing_this {
                let new_len = SeqDuration(bars.max(1) as u32 * ticks_per_bar);
                let pid = data.pattern_id;
                let mut song_w = song.write();
                if let Some(pat) = song_w.pattern_mut(pid) {
                    pat.length = new_len;
                }
            }
            if (bars_resp.drag_stopped() || bars_resp.lost_focus())
                && let Some((pid, old_len)) = view_state.pattern_length_drag_start.take()
            {
                let new_len = SeqDuration(bars.max(1) as u32 * ticks_per_bar);
                if pid == data.pattern_id && old_len != new_len {
                    undo_manager.push(crate::undo::UndoAction::SetPatternLength {
                        pattern_id: pid,
                        old_length: old_len,
                        new_length: new_len,
                    });
                }
            }
        }

        draw_pattern_instrument_transport(
            ui,
            data,
            handle,
            song,
            view_state,
            instruments,
            is_playing,
        );
        ui.separator();

        // Note FX toggle (badge = a dot when a note graph is bound to this pattern).
        let has_graph = song
            .try_read()
            .and_then(|s| s.pattern(data.pattern_id).map(|p| p.note_graph().is_some()))
            .unwrap_or(false);
        let note_fx_label = if has_graph { "Note FX ●" } else { "Note FX" };
        selectable_toggle(
            ui,
            &mut view_state.note_fx_panel_open,
            note_fx_label,
            "Show/hide the Note FX panel (bind a note graph to this pattern)",
        );
        selectable_toggle(
            ui,
            &mut view_state.show_note_fx_ghosts,
            "Ghosts",
            "Preview the note-graph / ornament expansion as faint ghost notes",
        );
        ui.separator();

        ui.label(
            RichText::new(format!("{} notes", data.notes.len())).color(t.colors.text_secondary),
        );

        if !view_state.selected_notes.is_empty() {
            ui.label(
                RichText::new(format!("{} selected", view_state.selected_notes.len()))
                    .color(t.colors.accent_yellow),
            );
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if danger_button(ui, egui_remixicon::icons::CLOSE_LINE)
                .on_hover_text("Close piano roll")
                .clicked()
            {
                keep_open = false;
            }

            let help_btn = ui
                .button(
                    RichText::new(egui_remixicon::icons::QUESTION_LINE).color(t.colors.text_dim),
                )
                .on_hover_text("Keyboard shortcuts");
            egui::Popup::from_toggle_button_response(&help_btn).show(|ui| {
                ui.set_min_width(320.0);
                strong_label(ui, "Piano-roll keyboard shortcuts", None);
                ui.add_space(t.spacing.xs);
                egui::Grid::new("pr_shortcuts_grid")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        let rows: &[(&str, &str)] = &[
                            ("Space", "Play / Pause"),
                            ("Delete · Backspace", "Delete selected notes"),
                            ("Escape", "Clear selection · exit step entry"),
                            ("Ctrl+A", "Select all notes"),
                            ("Ctrl+C", "Copy selection"),
                            ("Ctrl+X", "Cut selection"),
                            ("Ctrl+V", "Paste at playhead"),
                            ("Ctrl+D", "Duplicate selection after"),
                            ("↑ · ↓", "Transpose ±1 semitone"),
                            ("Shift+↑ · Shift+↓", "Transpose ±1 octave"),
                            ("Ctrl+scroll", "Horizontal zoom"),
                            ("Ctrl+Shift+scroll", "Vertical zoom"),
                            (
                                "STEP key letters",
                                concat!("A·W·S·E·D·F·T·G·Y·H·U·J ", "\u{ea6e}", " C..B"),
                            ),
                        ];
                        for (k, d) in rows {
                            ui.label(RichText::new(*k).monospace().color(t.colors.accent_cyan));
                            ui.label(*d);
                            ui.end_row();
                        }
                    });
            });
        });
    });

    // ── Toolbar (row 2: secondary tools/grid row) ──
    crate::gui::toolbar::secondary_row(ui, |ui| {
        // Tool selector
        let select_label = if view_state.edit_tool == EditTool::Select {
            RichText::new("Select").color(t.colors.accent_primary)
        } else {
            RichText::new("Select").color(t.colors.text_secondary)
        };
        if ui.button(select_label).clicked() {
            view_state.edit_tool = EditTool::Select;
        }

        let draw_label = if view_state.edit_tool == EditTool::Draw {
            RichText::new("Draw").color(t.colors.accent_primary)
        } else {
            RichText::new("Draw").color(t.colors.text_secondary)
        };
        if ui.button(draw_label).clicked() {
            view_state.edit_tool = EditTool::Draw;
        }

        ui.separator();

        // Automation lane selector (shared with the tracker view).
        draw_automation_target_selector(ui, view_state, data, instruments);
        // Mod Grid provenance + quick-assign for the focused lane's target.
        draw_mod_grid_lane_tools(ui, song, data, view_state, undo_manager);

        // Curve-type brush for newly drawn points (only while a lane is shown).
        // A point's type is changed after the fact from its right-click menu.
        if view_state.selected_automation.is_some() {
            egui::ComboBox::from_id_salt("auto_curve")
                .selected_text(format!("~{}", view_state.automation_curve.display_name()))
                .width(90.0)
                .show_ui(ui, |ui| {
                    for kind in CurveType::MENU_KINDS {
                        if ui
                            .selectable_label(
                                view_state.automation_curve.same_kind(kind),
                                kind.display_name(),
                            )
                            .clicked()
                        {
                            view_state.automation_curve = *kind;
                        }
                    }
                })
                .response
                .on_hover_text("Curve applied to newly drawn automation points");
        }

        ui.separator();

        // ── Grid resolution selector ──
        let grid_label = GRID_RESOLUTIONS
            .iter()
            .find(|(_, t)| *t == view_state.draw_grid_resolution)
            .map_or("Grid", |(l, _)| l);
        egui::ComboBox::from_id_salt("grid_res")
            .selected_text(grid_label)
            .width(50.0)
            .show_ui(ui, |ui| {
                for &(label, ticks) in GRID_RESOLUTIONS {
                    if ui
                        .selectable_label(view_state.draw_grid_resolution == ticks, label)
                        .clicked()
                    {
                        view_state.draw_grid_resolution = ticks;
                    }
                }
            });

        // ── Note length preset selector ──
        let len_label = GRID_RESOLUTIONS
            .iter()
            .find(|(_, t)| *t == view_state.draw_note_length)
            .map_or("L:Drag", |(l, _)| l);
        egui::ComboBox::from_id_salt("note_len")
            .selected_text(if view_state.draw_note_length == 0 {
                "L:Drag"
            } else {
                len_label
            })
            .width(55.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(view_state.draw_note_length == 0, "Drag")
                    .clicked()
                {
                    view_state.draw_note_length = 0;
                }
                for &(label, ticks) in &GRID_RESOLUTIONS[1..] {
                    if ui
                        .selectable_label(view_state.draw_note_length == ticks, label)
                        .clicked()
                    {
                        view_state.draw_note_length = ticks;
                    }
                }
            });

        // ── Default velocity ──
        let mut vel_pct = (view_state.default_velocity.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut vel_pct)
                    .range(1.0..=100.0)
                    .speed(1.0)
                    .suffix(" %")
                    .prefix("Vel:"),
            )
            .changed()
        {
            view_state.default_velocity = Velocity::new((vel_pct / 100.0).max(0.01));
        }

        // ── Quantize button ──
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty(),
                egui::Button::new(RichText::new("Q").color(t.colors.accent_primary)),
            )
            .on_hover_text(format!(
                "Quantize selected (strength {:.0}%)",
                view_state.quantize_strength.as_f32() * 100.0
            ))
            .clicked()
        {
            let grid = view_state.effective_grid(data.ticks_per_row);
            let strength = view_state.quantize_strength;
            let selected = view_state.selected_notes.clone();
            batch_transform_with_undo(song, data.pattern_id, &selected, undo_manager, |pattern| {
                pattern.quantize_selected(&selected, grid, strength)
            });
        }

        // ── Quantize strength (small drag value) ──
        let mut q_str_pct = (view_state.quantize_strength.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut q_str_pct)
                    .range(0.0..=100.0)
                    .speed(1.0)
                    .suffix(" %")
                    .prefix("Str:"),
            )
            .on_hover_text("Quantize strength")
            .changed()
        {
            view_state.quantize_strength = NormalizedValue::new(q_str_pct / 100.0);
        }

        // ── Humanize button ──
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty(),
                egui::Button::new(RichText::new("H").color(t.colors.text_secondary)),
            )
            .on_hover_text("Humanize selected notes (random timing/velocity)")
            .clicked()
        {
            let selected = view_state.selected_notes.clone();
            batch_transform_with_undo(song, data.pattern_id, &selected, undo_manager, |pattern| {
                pattern.humanize_notes(&selected, SeqDuration(15), NormalizedValue::new(0.05));
            });
        }

        ui.separator();

        // ── Swing control ──
        let mut sw_pct = (view_state.swing_amount.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut sw_pct)
                    .range(0.0..=100.0)
                    .speed(1.0)
                    .suffix(" %")
                    .prefix("Sw:"),
            )
            .on_hover_text("Swing amount (offset even subdivisions)")
            .changed()
        {
            view_state.swing_amount = NormalizedValue::new(sw_pct / 100.0);
        }
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty() && view_state.swing_amount.as_f32() > 0.0,
                egui::Button::new(RichText::new("Apply").color(t.colors.text_secondary)),
            )
            .on_hover_text("Apply swing to selected notes")
            .clicked()
        {
            let grid = view_state.effective_grid(data.ticks_per_row);
            let amount = view_state.swing_amount;
            let selected = view_state.selected_notes.clone();
            batch_transform_with_undo(song, data.pattern_id, &selected, undo_manager, |pattern| {
                pattern.apply_swing(&selected, grid, amount)
            });
        }

        // ── Scale velocities ──
        ui.add(
            egui::DragValue::new(&mut view_state.velocity_scale_pct)
                .range(1..=200_u32)
                .speed(1.0)
                .suffix(" %")
                .prefix("Vel×"),
        )
        .on_hover_text("Velocity scale factor");
        if ui
            .add_enabled(
                !view_state.selected_notes.is_empty() && view_state.velocity_scale_pct != 100,
                egui::Button::new(RichText::new("Scale").color(t.colors.text_secondary)),
            )
            .on_hover_text("Scale velocities of selected notes")
            .clicked()
        {
            let mut changes: Vec<(NoteId, Velocity, Velocity)> = Vec::new();
            {
                let mut song_w = song.write();
                if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                    for nid in &view_state.selected_notes {
                        if let Some(note) = pattern.note(*nid) {
                            changes.push((*nid, note.velocity, note.velocity));
                        }
                    }
                    pattern.scale_velocities(
                        &view_state.selected_notes,
                        view_state.velocity_scale_pct as f32 / 100.0,
                    );
                    for entry in &mut changes {
                        if let Some(note) = pattern.note(entry.0) {
                            entry.2 = note.velocity;
                        }
                    }
                }
            }
            if changes.iter().any(|(_, o, n)| o != n) {
                undo_manager.push(crate::undo::UndoAction::SetVelocitiesBatch {
                    pattern_id: data.pattern_id,
                    changes,
                });
            }
            view_state.velocity_scale_pct = 100;
        }

        // ── Step entry toggle ──
        let step_color = if view_state.step_entry_mode {
            t.colors.accent_primary
        } else {
            t.colors.text_dim
        };
        if ui
            .button(RichText::new("STEP").color(step_color))
            .on_hover_text("Step entry mode")
            .clicked()
        {
            view_state.step_entry_mode = !view_state.step_entry_mode;
            if view_state.step_entry_mode {
                view_state.step_cursor_tick = PatternTick::ZERO;
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("+").on_hover_text("Zoom in").clicked() {
                view_state.pr_zoom_x = (view_state.pr_zoom_x * 1.25).min(4.0);
                view_state.pr_zoom_y = (view_state.pr_zoom_y * 1.25).min(3.0);
            }
            if ui.small_button("1x").on_hover_text("Reset zoom").clicked() {
                view_state.pr_zoom_x = 1.0;
                view_state.pr_zoom_y = 1.0;
            }
            if ui
                .small_button("-")
                .on_hover_text("Zoom out (Ctrl+scroll = horizontal, Ctrl+Shift+scroll = vertical)")
                .clicked()
            {
                view_state.pr_zoom_x = (view_state.pr_zoom_x * 0.8).max(0.25);
                view_state.pr_zoom_y = (view_state.pr_zoom_y * 0.8).max(0.5);
            }
            caption(ui, "Zoom", CaptionTone::Dim);
        });
    });

    keep_open
}

#[allow(clippy::too_many_arguments)]
fn handle_piano_roll_interaction(
    ctx: &mut PianoRollCtx<'_>,
    response: &egui::Response,
    ui: &egui::Ui,
    grid_rect: Rect,
    auto_rect: Option<Rect>,
    auto_y: f32,
    auto_base_y: f32,
    auto_zone_targets: &[AutomationTarget],
    coords: &PianoRollCoords,
) {
    let data = ctx.data;
    let song = ctx.song;
    let view_state = &mut *ctx.view_state;
    let handle = &mut *ctx.handle;
    let undo_manager = &mut *ctx.undo_manager;
    // Re-expose the bundled transforms under the names the body already uses, so
    // the handler logic is untouched by the parameter consolidation. The `&dyn
    // Fn` bindings keep the call shape for the sub-helpers that take closures.
    let tick_to_x_impl = |t: PatternTick| coords.tick_to_x(t);
    let pitch_to_y_impl = |p: Pitch| coords.pitch_to_y(p);
    let x_to_tick_impl = |x: f32| coords.x_to_tick(x);
    let y_to_pitch_impl = |y: f32| coords.y_to_pitch(y);
    let tick_to_x: &dyn Fn(PatternTick) -> f32 = &tick_to_x_impl;
    let pitch_to_y: &dyn Fn(Pitch) -> f32 = &pitch_to_y_impl;
    let x_to_tick: &dyn Fn(f32) -> PatternTick = &x_to_tick_impl;
    let y_to_pitch: &dyn Fn(f32) -> Pitch = &y_to_pitch_impl;
    let view_pitch_min = coords.view_pitch_min;
    let view_pitch_max = coords.view_pitch_max;
    let note_row_height = coords.note_row_height;

    let shift_held = ui.input(|i| i.modifiers.shift);

    // Which stacked automation zone a Y coordinate falls in (0 = top zone).
    // Callers bound-check against `auto_zone_targets` via `.get(band)`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let band_at = |y: f32| ((y - auto_base_y) / AUTOMATION_ZONE_HEIGHT) as usize;

    // ── Focus-click: click a different stacked lane's zone to edit it ──
    // Only inside the stacked band's x/y extent, and never on the already-
    // focused lane's rect (that click falls through to the add-point handler
    // below) — so the two are mutually exclusive even at a shared zone edge.
    let auto_stack_bottom = auto_base_y + auto_zone_targets.len() as f32 * AUTOMATION_ZONE_HEIGHT;
    if response.clicked()
        && !auto_zone_targets.is_empty()
        && let Some(pos) = response.interact_pointer_pos()
        && (grid_rect.min.x..=grid_rect.max.x).contains(&pos.x)
        && (auto_base_y..auto_stack_bottom).contains(&pos.y)
        && !auto_rect.is_some_and(|r| r.contains(pos))
    {
        let band = band_at(pos.y);
        if let Some(target) = auto_zone_targets.get(band)
            && view_state.selected_automation.as_ref() != Some(target)
        {
            view_state.selected_automation = Some(target.clone());
        }
    }

    // ── Automation click handling ──
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(ar) = auto_rect
        && ar.contains(pos)
        && let Some(target) = &view_state.selected_automation
    {
        let target = target.clone();
        let raw_tick = x_to_tick(pos.x);
        let tick = quantize_tick(raw_tick, data.ticks_per_row);
        let value =
            NormalizedValue::new((1.0 - (pos.y - auto_y) / AUTOMATION_ZONE_HEIGHT).clamp(0.0, 1.0));

        let curve = view_state.automation_curve;
        let new_point = AutomationPoint::new(tick, value).with_curve(curve);
        {
            let mut song_w = song.write();
            if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                let lane = pattern.get_or_create_automation(target.clone());
                lane.add_point(new_point);
            }
        }
        undo_manager.push(crate::undo::UndoAction::AddAutomationPoint {
            pattern_id: data.pattern_id,
            target,
            tick,
            value,
            curve,
        });
    }

    // ── Automation right-click → context menu (works on ANY stacked lane) ──
    // Resolve which stacked zone the click landed in (not just the focused
    // lane), then whether it hit a point in that zone. `Some(tick)` → a point
    // (curve/delete-point items); `None` → empty zone (lane items only).
    if response.secondary_clicked() {
        view_state.automation_ctx_point = response
            .interact_pointer_pos()
            .filter(|pos| {
                (grid_rect.min.x..=grid_rect.max.x).contains(&pos.x)
                    && pos.y >= auto_base_y
                    && !auto_zone_targets.is_empty()
            })
            .and_then(|pos| {
                let band = band_at(pos.y);
                let target = auto_zone_targets.get(band)?;
                let zone_y = auto_base_y + band as f32 * AUTOMATION_ZONE_HEIGHT;
                let hit_tick = data
                    .automation_lanes
                    .iter()
                    .find(|l| &l.target == target)
                    .and_then(|lane| {
                        automation_point_at_pos(lane, pos, tick_to_x, zone_y)
                            .map(|idx| lane.points[idx].tick)
                    });
                Some((target.clone(), hit_tick))
            });
    }

    response.context_menu(|ui| {
        let Some((target, hit_tick)) = view_state.automation_ctx_point.clone() else {
            ui.close();
            return;
        };
        ui.set_min_width(140.0);

        // Point items (only when the right-click actually hit a point).
        if let Some(tick) = hit_tick
            && let Some(pt) = data
                .automation_lanes
                .iter()
                .find(|l| l.target == target)
                .and_then(|l| l.points.iter().find(|p| p.tick == tick))
        {
            let value = pt.value;
            let old_curve = pt.curve;
            ui.label("Curve");
            for kind in CurveType::MENU_KINDS {
                if ui
                    .selectable_label(old_curve.same_kind(kind), kind.display_name())
                    .clicked()
                {
                    if !old_curve.same_kind(kind) {
                        {
                            let mut song_w = song.write();
                            if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                                let lane = pattern.get_or_create_automation(target.clone());
                                lane.add_point(AutomationPoint::new(tick, value).with_curve(*kind));
                            }
                        }
                        undo_manager.push(crate::undo::UndoAction::SetAutomationPointCurve {
                            pattern_id: data.pattern_id,
                            target: target.clone(),
                            tick,
                            value,
                            old_curve,
                            new_curve: *kind,
                        });
                    }
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Delete point").clicked() {
                {
                    let mut song_w = song.write();
                    if let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                        && let Some(lane) =
                            pattern.automation.iter_mut().find(|l| l.target == target)
                    {
                        lane.remove_point(tick);
                    }
                }
                undo_manager.push(crate::undo::UndoAction::RemoveAutomationPoint {
                    pattern_id: data.pattern_id,
                    target: target.clone(),
                    tick,
                    value,
                    curve: old_curve,
                });
                ui.close();
            }
            ui.separator();
        }

        // Lane-level: delete the whole lane (captures the full lane for undo).
        if danger_button(ui, format!("Delete lane: {}", target.display_name())).clicked() {
            let removed = {
                let mut song_w = song.write();
                song_w
                    .pattern_mut(data.pattern_id)
                    .and_then(|pattern| pattern.remove_automation_lane(&target))
            };
            if let Some(lane) = removed {
                // If the deleted lane was the edit focus, clear it.
                if view_state.selected_automation.as_ref() == Some(&target) {
                    view_state.selected_automation = None;
                }
                undo_manager.push(crate::undo::UndoAction::RemoveAutomationLane {
                    pattern_id: data.pattern_id,
                    lane,
                });
            }
            ui.close();
        }
    });

    // ── Automation point hover tooltip (names the point's curve type) ──
    // `response.hovered()` is false when a higher layer (e.g. the point's
    // just-opened context menu) sits under the pointer, so the tooltip does
    // not overlap the menu on the right-click frame.
    if view_state.drag.is_none()
        && response.hovered()
        && let Some(pos) = ui.pointer_latest_pos()
        && let Some(ar) = auto_rect
        && ar.contains(pos)
        && let Some(target) = &view_state.selected_automation
        && let Some(lane) = data.automation_lanes.iter().find(|l| &l.target == target)
        && let Some(idx) = automation_point_at_pos(lane, pos, tick_to_x, auto_y)
    {
        let curve = lane.points[idx].curve;
        egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            egui::Id::new("auto_point_curve_tip"),
            pos,
        )
        .at_pointer()
        .show(|ui| ui.label(format!("{} point", curve.display_name())));
    }

    // ── Click handling ──
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && grid_rect.contains(pos)
    {
        let vp_min = view_pitch_min;
        let vp_max = view_pitch_max;
        let hit = note_at_pos(
            &data.notes,
            pos,
            tick_to_x,
            pitch_to_y,
            data.length_ticks,
            vp_min,
            vp_max,
            note_row_height,
        );

        match hit {
            Some((note_id, _)) => {
                // Clicked on a note — select it and preview its sound
                if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                    preview_note(handle, note.pitch, note.velocity);
                }
                if shift_held {
                    // Toggle in selection
                    if !view_state.selected_notes.remove(&note_id) {
                        view_state.selected_notes.insert(note_id);
                    }
                } else {
                    view_state.selected_notes.clear();
                    view_state.selected_notes.insert(note_id);
                }
            }
            None => {
                // Clicked on empty space
                if view_state.edit_tool == EditTool::Draw {
                    // Add a new note (only if no note already exists here)
                    let raw_tick = x_to_tick(pos.x);
                    let tick = quantize_tick(raw_tick, data.ticks_per_row);
                    let pitch_val = y_to_pitch(pos.y);

                    if !has_note_at(&data.notes, tick, pitch_val)
                        && let mut song_w = song.write()
                        && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                    {
                        let duration = if view_state.draw_note_length > 0 {
                            SeqDuration(view_state.draw_note_length)
                        } else {
                            SeqDuration((data.ticks_per_row as u32).max(1))
                        };
                        let note_id =
                            pattern.add_note(tick, pitch_val, view_state.default_velocity);
                        pattern.resize_note(note_id, duration);
                        if let Some(note) = pattern.note(note_id) {
                            undo_manager.push(crate::undo::UndoAction::AddNote {
                                pattern_id: data.pattern_id,
                                note: note.into(),
                            });
                        }
                        view_state.selected_notes.clear();
                        view_state.selected_notes.insert(note_id);
                        preview_note(handle, pitch_val, view_state.default_velocity);
                    }
                } else if !shift_held {
                    // Select tool on empty space — clear selection
                    view_state.selected_notes.clear();
                }
            }
        }
    }

    // ── Automation point drag start (any stacked lane, not just the focused
    // one) ── Resolve which zone the press landed in, hit-test a point there,
    // and start the drag against that zone's own top y. Also focus the lane so
    // its zone renders as selected while dragging.
    if response.drag_started()
        && !auto_zone_targets.is_empty()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        && (grid_rect.min.x..=grid_rect.max.x).contains(&pos.x)
        && (auto_base_y..auto_stack_bottom).contains(&pos.y)
    {
        let band = band_at(pos.y);
        let zone_y = auto_base_y + band as f32 * AUTOMATION_ZONE_HEIGHT;
        if let Some(target) = auto_zone_targets.get(band).cloned()
            && let Some(lane) = data.automation_lanes.iter().find(|l| l.target == target)
            && let Some(idx) = automation_point_at_pos(lane, pos, tick_to_x, zone_y)
        {
            let pt = &lane.points[idx];
            view_state.selected_automation = Some(target.clone());
            view_state.drag = Some(DragState::DragAutomationPoint {
                target,
                original_tick: pt.tick,
                original_value: pt.value,
                current_tick: pt.tick,
                current_value: pt.value,
                curve: pt.curve,
                zone_y,
            });
        }
    }

    // ── Velocity click: set the nearest note's velocity from the click height
    // (a plain click, no drag needed — mirrors the drag-paint apply below).
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let vel_rect = velocity_zone_rect(grid_rect);
        if vel_rect.contains(pos)
            && let Some(note) = nearest_velocity_note(&data.notes, pos.x, tick_to_x)
        {
            let old_velocity = note.velocity;
            let new_velocity = Velocity::new(velocity_from_pos_y(pos.y, vel_rect.min.y));
            // Skip a click that doesn't move the value — no silent no-op
            // undo entry.
            if new_velocity != old_velocity {
                {
                    let mut song_w = song.write();
                    if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                        pattern.set_note_velocity(note.note_id, new_velocity);
                    }
                }
                undo_manager.push(crate::undo::UndoAction::SetNoteVelocity {
                    pattern_id: data.pattern_id,
                    note_id: note.note_id,
                    old_velocity,
                    new_velocity,
                });
            }
        }
    }

    // ── Velocity drag start ──
    if response.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        && velocity_zone_rect(grid_rect).contains(pos)
    {
        view_state.drag = Some(DragState::DragVelocity {
            last_note_id: None,
            initial_velocities: std::collections::HashMap::new(),
        });
    }

    // ── Drag start ──
    // Use press_origin (where the click started) for hit-testing, not current pointer pos.
    // Without this, dragging straight up/down misses the note because the pointer
    // has already left the note rect by the time the drag threshold is reached.
    if response.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        && grid_rect.contains(pos)
    {
        let vp_min = view_pitch_min;
        let vp_max = view_pitch_max;
        let hit = note_at_pos(
            &data.notes,
            pos,
            tick_to_x,
            pitch_to_y,
            data.length_ticks,
            vp_min,
            vp_max,
            note_row_height,
        );

        match hit {
            Some((note_id, HitZone::Body)) => {
                // Start moving the note
                if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                    // If note has no explicit duration, lock it before move
                    // so the visual length is preserved
                    if note.end_tick.is_none() {
                        let implied_dur = (data.length_ticks.as_pattern_tick() - note.start_tick)
                            .max(SeqDuration(1));
                        {
                            let mut song_w = song.write();
                            if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                                pattern.resize_note(note_id, implied_dur);
                            }
                        }
                    }
                    // Calculate where on the note the user grabbed
                    let grab_tick = x_to_tick(pos.x);
                    let grab_offset = grab_tick - note.start_tick;
                    view_state.drag = Some(DragState::MoveNote {
                        note_id,
                        original_tick: note.start_tick,
                        original_pitch: note.pitch,
                        current_tick: note.start_tick,
                        current_pitch: note.pitch,
                        grab_offset_ticks: grab_offset,
                    });
                    // Ensure note is selected
                    if !shift_held {
                        view_state.selected_notes.clear();
                    }
                    view_state.selected_notes.insert(note_id);
                }
            }
            Some((note_id, HitZone::RightEdge)) => {
                // Start resizing the note
                if let Some(note) = data.notes.iter().find(|n| n.note_id == note_id) {
                    let end_tick = note.end_tick.unwrap_or(data.length_ticks.as_pattern_tick());
                    view_state.drag = Some(DragState::ResizeNote {
                        note_id,
                        original_end_tick: end_tick,
                        current_end_tick: end_tick,
                    });
                }
            }
            None => match view_state.edit_tool {
                EditTool::Select => {
                    // Start selection rectangle
                    view_state.drag = Some(DragState::SelectRect {
                        start_pos: pos,
                        current_pos: pos,
                    });
                    if !shift_held {
                        view_state.selected_notes.clear();
                    }
                }
                EditTool::Draw => {
                    // Start drawing a new note by dragging (only if position is free)
                    let raw_tick = x_to_tick(pos.x);
                    let start_tick = quantize_tick(raw_tick, data.ticks_per_row);
                    let pitch = y_to_pitch(pos.y);
                    if !has_note_at(&data.notes, start_tick, pitch) {
                        let end_tick =
                            PatternTick(start_tick.0 + (data.ticks_per_row as u32).max(1));
                        view_state.drag = Some(DragState::DrawNote {
                            start_tick,
                            pitch,
                            current_end_tick: end_tick,
                        });
                    }
                }
            },
        }
    }

    // ── Drag update ──
    // Use pointer_latest_pos for smooth tracking during drags
    if response.dragged()
        && let Some(pos) = ui.pointer_latest_pos()
    {
        match &mut view_state.drag {
            Some(DragState::MoveNote {
                current_tick,
                current_pitch,
                grab_offset_ticks,
                ..
            }) => {
                let raw_tick = PatternTick(x_to_tick(pos.x).0.saturating_sub(grab_offset_ticks.0));
                *current_tick = quantize_tick(raw_tick, data.ticks_per_row);
                *current_pitch = y_to_pitch(pos.y);
            }
            Some(DragState::ResizeNote {
                current_end_tick, ..
            }) => {
                let raw_tick = x_to_tick(pos.x);
                let quantized = quantize_tick(raw_tick, data.ticks_per_row);
                *current_end_tick = PatternTick(quantized.0.max(1));
            }
            Some(DragState::DrawNote {
                start_tick,
                current_end_tick,
                ..
            }) => {
                let raw_tick = x_to_tick(pos.x);
                let quantized = quantize_tick(raw_tick, data.ticks_per_row);
                // End tick must be at least one row past start
                *current_end_tick = PatternTick(
                    quantized
                        .0
                        .max(start_tick.0 + (data.ticks_per_row as u32).max(1)),
                );
            }
            Some(DragState::SelectRect { current_pos, .. }) => {
                *current_pos = pos;
            }
            Some(DragState::DragAutomationPoint {
                current_tick,
                current_value,
                zone_y,
                ..
            }) => {
                let raw_tick = x_to_tick(pos.x);
                *current_tick = quantize_tick(raw_tick, data.ticks_per_row);
                *current_value = NormalizedValue::new(
                    (1.0 - (pos.y - *zone_y) / AUTOMATION_ZONE_HEIGHT).clamp(0.0, 1.0),
                );
            }
            Some(DragState::DragVelocity { .. }) => {
                // Velocity painting handled in real-time below
            }
            Some(DragState::DragPlacement { .. })
            | Some(DragState::ResizePlacement { .. })
            | Some(DragState::MoveSection { .. })
            | Some(DragState::ResizeSection { .. })
            | None => {}
        }
    }

    // ── Velocity drag: apply velocity change in real-time ──
    if let Some(DragState::DragVelocity {
        last_note_id,
        initial_velocities,
    }) = &mut view_state.drag
        && let Some(pos) = ui.pointer_latest_pos()
    {
        let new_vel = velocity_from_pos_y(pos.y, grid_rect.max.y);
        if let Some(note) = nearest_velocity_note(&data.notes, pos.x, tick_to_x) {
            *last_note_id = Some(note.note_id);
            initial_velocities
                .entry(note.note_id)
                .or_insert(note.velocity);
            {
                let mut song_w = song.write();
                if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                    pattern.set_note_velocity(note.note_id, Velocity::new(new_vel));
                }
            }
        }
    }

    // ── Drag end ──
    if response.drag_stopped()
        && let Some(drag) = view_state.drag.take()
    {
        match drag {
            DragState::MoveNote {
                note_id,
                original_tick,
                original_pitch,
                current_tick,
                current_pitch,
                ..
            } => {
                // Apply move to song
                if (current_tick != original_tick || current_pitch != original_pitch)
                    && let mut song_w = song.write()
                    && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                {
                    let mut composite = Vec::new();
                    if current_tick != original_tick {
                        pattern.move_note(note_id, current_tick);
                        composite.push(crate::undo::UndoAction::MoveNote {
                            pattern_id: data.pattern_id,
                            note_id,
                            old_start: original_tick,
                            new_start: current_tick,
                        });
                    }
                    if current_pitch != original_pitch {
                        #[allow(clippy::cast_precision_loss)]
                        let delta =
                            current_pitch.as_midi() as f32 - original_pitch.as_midi() as f32;
                        pattern.transpose_note(note_id, Semitones::new(delta));
                        composite.push(crate::undo::UndoAction::TransposeNote {
                            pattern_id: data.pattern_id,
                            note_id,
                            old_pitch: original_pitch,
                            new_pitch: current_pitch,
                        });
                    }
                    if !composite.is_empty() {
                        undo_manager.push(crate::undo::UndoAction::Composite(composite));
                    }
                }
            }
            DragState::ResizeNote {
                note_id,
                original_end_tick,
                current_end_tick,
                ..
            } => {
                // Apply resize to song
                if current_end_tick != original_end_tick
                    && let Some(note) = data.notes.iter().find(|n| n.note_id == note_id)
                {
                    let new_duration = (current_end_tick - note.start_tick).max(SeqDuration(1));
                    let old_duration = note.end_tick.map(|e| e - note.start_tick);
                    {
                        let mut song_w = song.write();
                        if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                            pattern.resize_note(note_id, new_duration);
                            undo_manager.push(crate::undo::UndoAction::ResizeNote {
                                pattern_id: data.pattern_id,
                                note_id,
                                old_duration: old_duration.map(|d| SeqDuration(d.0)),
                                new_duration: Some(new_duration),
                            });
                        }
                    }
                }
            }
            DragState::DrawNote {
                start_tick,
                pitch,
                current_end_tick,
            } => {
                // Create the note with the dragged duration (only if no duplicate)
                let duration = (current_end_tick - start_tick).max(SeqDuration(1));
                if !has_note_at(&data.notes, start_tick, pitch)
                    && let mut song_w = song.write()
                    && let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                {
                    let note_id = pattern.add_note(start_tick, pitch, view_state.default_velocity);
                    pattern.resize_note(note_id, duration);
                    if let Some(added_note) = pattern.note(note_id) {
                        undo_manager.push(crate::undo::UndoAction::AddNote {
                            pattern_id: data.pattern_id,
                            note: added_note.into(),
                        });
                    }
                    view_state.selected_notes.clear();
                    view_state.selected_notes.insert(note_id);
                }
            }
            DragState::SelectRect {
                start_pos,
                current_pos,
                ..
            } => {
                // Resolve selection rectangle to notes
                let sel_rect = Rect::from_two_pos(start_pos, current_pos);
                let tick_start = x_to_tick(sel_rect.min.x);
                let tick_end = x_to_tick(sel_rect.max.x);
                let pitch_top = y_to_pitch(sel_rect.min.y);
                let pitch_bottom = y_to_pitch(sel_rect.max.y);
                let p_min = pitch_bottom.min(pitch_top);
                let p_max = pitch_bottom.max(pitch_top);

                for note in &data.notes {
                    let note_end = note.end_tick.unwrap_or(data.length_ticks.as_pattern_tick());
                    if note.start_tick < tick_end
                        && note_end > tick_start
                        && note.pitch >= p_min
                        && note.pitch <= p_max
                    {
                        view_state.selected_notes.insert(note.note_id);
                    }
                }
            }
            DragState::DragAutomationPoint {
                target,
                original_tick,
                original_value,
                current_tick,
                current_value,
                curve,
                zone_y: _,
            } => {
                // Apply automation point move — the point keeps its own curve.
                if current_tick != original_tick
                    || (current_value.as_f32() - original_value.as_f32()).abs() > f32::EPSILON
                {
                    let new_point =
                        AutomationPoint::new(current_tick, current_value).with_curve(curve);
                    {
                        let mut song_w = song.write();
                        if let Some(pattern) = song_w.pattern_mut(data.pattern_id) {
                            let lane = pattern.get_or_create_automation(target.clone());
                            lane.remove_point(original_tick);
                            lane.add_point(new_point);
                        }
                    }
                    undo_manager.push(crate::undo::UndoAction::MoveAutomationPoint {
                        pattern_id: data.pattern_id,
                        target,
                        old_tick: original_tick,
                        old_value: original_value,
                        new_tick: current_tick,
                        new_value: current_value,
                        curve,
                    });
                }
            }
            // DragVelocity — applied in real-time; on release, capture the
            // before/after pairs into a single composite undo entry.
            DragState::DragVelocity {
                initial_velocities, ..
            } => {
                if !initial_velocities.is_empty() {
                    let mut changes: Vec<(NoteId, Velocity, Velocity)> = Vec::new();
                    {
                        let song_r = song.read();
                        if let Some(pattern) = song_r.pattern(data.pattern_id) {
                            for (note_id, old_vel) in &initial_velocities {
                                if let Some(note) = pattern.note(*note_id)
                                    && note.velocity != *old_vel
                                {
                                    changes.push((*note_id, *old_vel, note.velocity));
                                }
                            }
                        }
                    }
                    if !changes.is_empty() {
                        undo_manager.push(crate::undo::UndoAction::SetVelocitiesBatch {
                            pattern_id: data.pattern_id,
                            changes,
                        });
                    }
                }
            }
            // Arrangement-only drags are handled in the arrangement view.
            DragState::DragPlacement { .. }
            | DragState::ResizePlacement { .. }
            | DragState::MoveSection { .. }
            | DragState::ResizeSection { .. } => {}
        }
    }
}
