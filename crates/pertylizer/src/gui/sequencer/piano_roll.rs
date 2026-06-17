//! Piano-roll view: note grid, selection inspector, automation-target
//! selector, per-pattern instrument transport, and all mouse interaction
//! (draw/select/move/resize/delete notes, velocity + expression editing).
//!
//! Snapshot DTOs (`PianoRollData` et al.) and the tick helpers
//! (`snap_to_step`, `quantize_tick`) live in the parent module.

use super::*;

/// Collect piano roll data from song (short read-lock, then release).
pub(crate) fn collect_piano_roll_data(
    song: &Arc<RwLock<Song>>,
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
                has_ornament: n.ornament.is_some(),
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
    let mut track_overrides: Vec<SeqInstrumentId> = Vec::new();
    for placement in song.arrangement() {
        if placement.pattern_id == pattern_id
            && let Some(track) = song.track(placement.track_id)
            && !track_overrides.contains(&track.instrument)
        {
            track_overrides.push(track.instrument);
        }
    }

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
    song: &Arc<RwLock<Song>>,
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
    song: &Arc<RwLock<Song>>,
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
    song: &Arc<RwLock<Song>>,
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
    song: &Arc<RwLock<Song>>,
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
                ui.label(RichText::new("Pitch").color(t.colors.text_dim).size(10.0));
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
                ui.label(RichText::new("Start").color(t.colors.text_dim).size(10.0));
                let start_text = if starts_equal {
                    let beats =
                        first.start_tick.0 as f32 / synth_sequencer::TICKS_PER_QUARTER as f32;
                    format!("{beats:.2} beats")
                } else {
                    "—".to_owned()
                };
                ui.label(RichText::new(start_text).color(t.colors.text_primary));

                // Length
                ui.label(RichText::new("Len").color(t.colors.text_dim).size(10.0));
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

                ui.label(RichText::new("Vel").color(t.colors.text_dim).size(10.0));
                let mut vel_pct = if velocities_equal {
                    (first.velocity.as_f32() * 100.0).round()
                } else {
                    50.0
                };
                let vel_resp = ui
                    .add(
                        egui::DragValue::new(&mut vel_pct)
                            .range(1.0..=100.0)
                            .speed(1.0)
                            .suffix(" %"),
                    )
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

                    ui.label(RichText::new("From").color(t.colors.text_dim).size(10.0));
                    let from_resp = ui
                        .add(
                            egui::DragValue::new(&mut from_semis)
                                .range(-24.0..=24.0)
                                .speed(0.5)
                                .suffix(" st"),
                        )
                        .on_hover_text("Glide source, semitones relative to this note");

                    ui.label(RichText::new("Time").color(t.colors.text_dim).size(10.0));
                    let time_resp = ui
                        .add(
                            egui::DragValue::new(&mut time_ms)
                                .range(0.0..=2000.0)
                                .speed(2.0)
                                .suffix(" ms"),
                        )
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
                ui.label(RichText::new("Accent").color(t.colors.text_dim).size(10.0));
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
                ui.label(RichText::new("Gate").color(t.colors.text_dim).size(10.0));
                let mut gate_pct = cur.gate.map_or(100.0, |g| g.as_f32() * 100.0);
                let r = ui
                    .add(
                        egui::DragValue::new(&mut gate_pct)
                            .range(1.0..=100.0)
                            .speed(1.0)
                            .suffix(" %"),
                    )
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
                ui.label(RichText::new("Prob").color(t.colors.text_dim).size(10.0));
                let mut prob_pct = cur.probability.map_or(100.0, |p| p.as_f32() * 100.0);
                let r = ui
                    .add(
                        egui::DragValue::new(&mut prob_pct)
                            .range(0.0..=100.0)
                            .speed(1.0)
                            .suffix(" %"),
                    )
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
                    ui.label(RichText::new("Depth").color(t.colors.text_dim).size(10.0));
                    let mut depth = v.depth.as_f32();
                    let rd = ui
                        .add(
                            egui::DragValue::new(&mut depth)
                                .range(0.0..=2.0)
                                .speed(0.01)
                                .suffix(" st"),
                        )
                        .on_hover_text("Vibrato depth (semitones)");
                    ui.label(RichText::new("Rate").color(t.colors.text_dim).size(10.0));
                    let mut rate = v.rate.as_f32();
                    let rr = ui
                        .add(
                            egui::DragValue::new(&mut rate)
                                .range(0.1..=20.0)
                                .speed(0.1)
                                .suffix(" Hz"),
                        )
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
                    ui.label(
                        RichText::new("Ornament")
                            .color(t.colors.text_dim)
                            .size(10.0),
                    );
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

/// The per-note ornament editor popup window. Shown while
/// `view_state.editing_ornament` is set; applies edits live and pushes one
/// coalesced `SetNoteOrnament` undo entry when the window is closed.
fn draw_ornament_popup(
    ui: &mut egui::Ui,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    undo_manager: &mut crate::undo::UndoManager,
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
        undo_manager.push(crate::undo::UndoAction::SetNoteOrnament {
            pattern_id: edit.pattern_id,
            note_id: edit.note_id,
            old: edit.before,
            new: edit.current,
        });
    }
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
    ui.label(RichText::new("Auto:").color(theme().colors.text_dim));
    // Build label for ComboBox
    let auto_label = view_state
        .selected_automation
        .as_ref()
        .map_or_else(|| "None".to_owned(), AutomationTarget::display_name);

    egui::ComboBox::from_id_salt("auto_lane_select")
        .selected_text(&auto_label)
        .width(160.0)
        .show_ui(ui, |ui| {
            // "None" option to hide automation zone
            if ui
                .selectable_label(view_state.selected_automation.is_none(), "None")
                .clicked()
            {
                view_state.selected_automation = None;
            }

            // 1. Existing lanes with points (any instrument), with a
            //    badge when they belong to a different instrument.
            let mut shown_any_existing = false;
            for lane in &data.automation_lanes {
                if lane.points.is_empty() {
                    continue;
                }
                let target = &lane.target;
                let is_foreign = match target {
                    AutomationTarget::Instrument { instrument, .. }
                    | AutomationTarget::Module { instrument, .. } => {
                        *instrument != view_state.selected_instrument
                    }
                    _ => false,
                };
                let inst_name = match target {
                    AutomationTarget::Instrument { instrument, .. }
                    | AutomationTarget::Module { instrument, .. } => instruments
                        .iter()
                        .find(|inst| inst.id.0 == instrument.0 as u64)
                        .map(|inst| inst.name.clone()),
                    _ => None,
                };
                let base = target.display_name();
                let arrow = egui_remixicon::icons::ARROW_RIGHT_S_LINE;
                let label = if is_foreign {
                    match inst_name {
                        Some(name) => format!("* {base}  {arrow} {name}"),
                        None => format!("* {base}"),
                    }
                } else {
                    format!("* {base}")
                };
                let is_selected = view_state.selected_automation.as_ref() == Some(target);
                if ui.selectable_label(is_selected, &label).clicked() {
                    view_state.selected_automation = Some(target.clone());
                }
                shown_any_existing = true;
            }
            if shown_any_existing {
                ui.separator();
            }

            // 2. All instrument params for the currently selected
            //    instrument (empty + with-points alike), so the user
            //    can create new lanes.
            for param in AutoInstrumentParam::ALL {
                let target = AutomationTarget::Instrument {
                    instrument: view_state.selected_instrument,
                    param: *param,
                };
                let already_shown = data
                    .automation_lanes
                    .iter()
                    .any(|l| l.target == target && !l.points.is_empty());
                if already_shown {
                    continue;
                }
                let label = param.display_name().to_owned();
                let is_selected = view_state.selected_automation.as_ref() == Some(&target);
                if ui.selectable_label(is_selected, &label).clicked() {
                    view_state.selected_automation = Some(target);
                }
            }

            // 3. Per-module parameters of the selected instrument
            //    (generic A2 targets), filtered to the automatable
            //    allowlist. Lets the user automate any continuous,
            //    RT-safe module parameter, not just the fixed
            //    instrument-level set above.
            if let Some(inst) = instruments
                .iter()
                .find(|i| i.id.0 == view_state.selected_instrument.0 as u64)
            {
                let mut module_ids = inst.patch_editor.module_ids();
                module_ids.sort_unstable(); // deterministic (type, instance) order
                let mut shown_module_header = false;
                for module_id in module_ids {
                    let Some(desc) = inst.patch_editor.module_descriptor(module_id) else {
                        continue;
                    };
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
                        // Skip params already shown as an existing lane above.
                        let already_shown = data
                            .automation_lanes
                            .iter()
                            .any(|l| l.target == target && !l.points.is_empty());
                        if already_shown {
                            continue;
                        }
                        if !shown_module_header {
                            ui.separator();
                            shown_module_header = true;
                        }
                        let label =
                            format!("{} {} · {}", desc.name, module_id.instance, param.name);
                        let is_selected = view_state.selected_automation.as_ref() == Some(&target);
                        if ui.selectable_label(is_selected, &label).clicked() {
                            view_state.selected_automation = Some(target);
                        }
                    }
                }
            }
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
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    is_playing: bool,
) {
    let t = theme();

    // Instrument selector for new notes
    {
        let selected_label = instruments
            .iter()
            .find(|inst| inst.id.0 == view_state.selected_instrument.0 as u64)
            .map_or_else(|| "---".to_owned(), |inst| inst.name.clone());
        egui::ComboBox::from_id_salt(ui.id().with("piano_roll_instrument"))
            .selected_text(RichText::new(&selected_label).size(12.0))
            .width(100.0)
            .show_ui(ui, |ui| {
                for inst in instruments {
                    let seq_id = SeqInstrumentId::new(inst.id.0 as u16);
                    let selected = view_state.selected_instrument == seq_id;
                    if ui.selectable_label(selected, &inst.name).clicked() {
                        view_state.selected_instrument = seq_id;
                    }
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
                        .find(|inst| inst.id.0 == seq_id.0 as u64)
                        .map_or_else(|| format!("#{}", seq_id.0), |inst| inst.name.clone())
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
        // overlapping the pattern's time range also play.
        let solo_icon = if view_state.pattern_solo {
            RichText::new(ri::VOLUME_MUTE_FILL)
                .size(12.0)
                .color(t.colors.accent_yellow)
        } else {
            RichText::new(ri::VOLUME_UP_LINE)
                .size(12.0)
                .color(t.colors.text_dim)
        };
        let solo_resp = ui
            .button(solo_icon)
            .on_hover_text(if view_state.pattern_solo {
                "Solo: only this pattern plays — click to also hear other tracks"
            } else {
                "Other tracks audible — click to isolate this pattern"
            });
        if solo_resp.clicked() {
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
    song: &'a Arc<RwLock<Song>>,
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
        song: &'a Arc<RwLock<Song>>,
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

/// Draw the piano roll in a bottom panel.
/// Returns false if the close button was clicked.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
pub(crate) fn draw_piano_roll(
    ui: &mut egui::Ui,
    data: &PianoRollData,
    playhead_tick: Option<PatternTick>,
    is_playing: bool,
    handle: &mut EngineHandle,
    song: &Arc<RwLock<Song>>,
    view_state: &mut SequencerViewState,
    instruments: &[crate::gui::instrument_rack::InstrumentUiState],
    undo_manager: &mut crate::undo::UndoManager,
) -> bool {
    let t = theme();
    let keep_open = {
        let mut ctx = PianoRollCtx::new(data, song, view_state, handle, undo_manager, instruments);
        draw_piano_roll_toolbar(&mut ctx, ui, is_playing)
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
                    ui.label(
                        RichText::new("STEP ENTRY")
                            .strong()
                            .color(STEP_ENTRY_TEXT),
                    );
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
    let auto_height = if view_state.selected_automation.is_some() {
        AUTOMATION_ZONE_HEIGHT
    } else {
        0.0
    };
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

    // ── Scrollable piano roll area ──
    // Use all available height (panel is resizable via TopBottomPanel)
    let scroll_max_height = ui.available_height().max(100.0);

    // Auto-follow playhead in piano roll during playback
    let pr_scroll_salt = "piano_roll_scroll";
    let pr_scroll_id = ui.make_persistent_id(egui::Id::new(pr_scroll_salt));
    if is_playing
        && view_state.auto_follow_playhead
        && let Some(pt) = playhead_tick
        && ticks_per_beat > 0
    {
        let playhead_beats = pt.0 as f32 / ticks_per_beat as f32;
        let playhead_x = KEY_WIDTH + playhead_beats * pr_pixels_per_beat;
        let visible_width = ui.available_width();
        let target_offset = (playhead_x - visible_width * 0.5).max(0.0);

        if let Some(mut scroll_state) = egui::scroll_area::State::load(ui.ctx(), pr_scroll_id) {
            scroll_state.offset.x = target_offset;
            scroll_state.store(ui.ctx(), pr_scroll_id);
            view_state.pr_last_auto_scroll_offset = Some(target_offset);
        }
    }

    let scroll_output = egui::ScrollArea::both()
        .id_salt(pr_scroll_salt)
        .max_height(scroll_max_height)
        .scroll_source(egui::scroll_area::ScrollSource {
            scroll_bar: true,
            drag: false, // Don't steal drag events — we handle them for note editing
            mouse_wheel: true,
        })
        .show(ui, |ui| {
            let total_size = Vec2::new(KEY_WIDTH + grid_width, RULER_HEIGHT + total_content_height);

            // Use allocate_rect with click_and_drag sense for mouse interaction
            let alloc_rect = Rect::from_min_size(ui.cursor().min, total_size);
            let response = ui.allocate_rect(alloc_rect, Sense::click_and_drag());
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
            let grid_x = origin.x + KEY_WIDTH;
            // Reserve a ruler strip at the top; the grid starts below it.
            let grid_y = origin.y + RULER_HEIGHT;

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

            // ── Timeline ruler (bar numbers) ──
            // Top-left corner cell above the keyboard column.
            painter.rect_filled(
                Rect::from_min_size(origin, Vec2::new(KEY_WIDTH, RULER_HEIGHT)),
                0.0,
                t.colors.bg_dark,
            );
            let ruler_rect = Rect::from_min_size(
                Pos2::new(grid_x, origin.y),
                Vec2::new(grid_width, RULER_HEIGHT),
            );
            let ticks_per_bar = u64::from(data.time_sig.ticks_per_bar().max(1));
            let total_bars = effective_ticks.div_ceil(data.time_sig.ticks_per_bar().max(1)).max(1);
            draw_ruler_labels(&painter, &t, ruler_rect, total_bars, ticks_per_bar, |tick| {
                if ticks_per_beat == 0 {
                    grid_x
                } else {
                    grid_x + (tick as f32 / ticks_per_beat as f32) * pr_pixels_per_beat
                }
            });
            // Ruler bottom border.
            painter.line_segment(
                [
                    Pos2::new(grid_x, grid_y),
                    Pos2::new(grid_x + grid_width, grid_y),
                ],
                Stroke::new(1.0, t.colors.border),
            );

            // ── Keyboard (left column) ──
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(origin.x, grid_y), Vec2::new(KEY_WIDTH, grid_height)),
                0.0,
                t.colors.bg_dark,
            );

            for p in view_pitch_min.as_midi()..=view_pitch_max.as_midi() {
                let pitch = Pitch::new(p).unwrap_or(Pitch::MIDDLE_C);
                let y = pitch_to_y(pitch);
                let note_name = NoteName::from_midi(p % 12);
                let is_black = note_name.is_black_key();

                // Key background
                let key_color = if is_black {
                    PIANO_KEY_BLACK
                } else {
                    PIANO_KEY_WHITE
                };
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(origin.x, y),
                        Vec2::new(KEY_WIDTH, note_row_height),
                    ),
                    0.0,
                    key_color,
                );

                // Label on C notes
                if p % 12 == 0 {
                    let octave = (p / 12) as i8 - 1;
                    painter.text(
                        Pos2::new(origin.x + 4.0, y + 1.0),
                        egui::Align2::LEFT_TOP,
                        format!("C{octave}"),
                        egui::FontId::proportional(10.0),
                        t.colors.text_primary,
                    );
                }

                // Key border
                painter.line_segment(
                    [
                        Pos2::new(origin.x, y + note_row_height),
                        Pos2::new(origin.x + KEY_WIDTH, y + note_row_height),
                    ],
                    Stroke::new(0.5, t.colors.border.gamma_multiply(0.3)),
                );
            }

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
                    Rect::from_min_size(
                        Pos2::new(grid_x, y),
                        Vec2::new(grid_width, note_row_height),
                    ),
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

                let fill = Color32::from_rgba_unmultiplied(
                    inst_color.r(),
                    inst_color.g(),
                    inst_color.b(),
                    alpha,
                );

                let note_rect = Rect::from_min_size(
                    Pos2::new(x_start, y + 1.0),
                    Vec2::new(note_width, note_row_height - 2.0),
                );

                if is_selected {
                    // Soft glow halo behind the note (cyan tint).
                    let glow_rect = note_rect.expand(3.0);
                    painter.rect_filled(
                        glow_rect,
                        3.0,
                        NOTE_SELECTED_GLOW,
                    );
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
                if note.has_ornament && note_width > 5.0 {
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
                        painter.rect_filled(
                            held_rect,
                            2.0,
                            RECORDING_PREVIEW_HELD_FILL,
                        );
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
                    painter.rect_filled(
                        ghost_rect,
                        2.0,
                        MOVE_GHOST_FILL,
                    );
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
                painter.rect_filled(
                    draw_rect,
                    2.0,
                    DRAW_NOTE_FILL,
                );
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
                painter.rect_filled(
                    sel_rect,
                    0.0,
                    SELECTION_RECT_FILL,
                );
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

            // Velocity label
            painter.text(
                Pos2::new(origin.x + 2.0, vel_y + 2.0),
                egui::Align2::LEFT_TOP,
                "VEL",
                egui::FontId::proportional(9.0),
                t.colors.text_dim,
            );

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

            // ── Automation zone (below velocity) ──
            let auto_y = vel_y + VELOCITY_ZONE_HEIGHT;
            if let Some(selected_target) = &view_state.selected_automation {
                draw_automation_zone(
                    &painter,
                    data,
                    view_state,
                    selected_target,
                    grid_x,
                    auto_y,
                    grid_width,
                    &tick_to_x,
                    &t,
                );
            }

            // ── Playhead (only if this pattern is actually playing) ──
            if let Some(pattern_tick) = playhead_tick {
                let playhead_x = tick_to_x(pattern_tick);

                if playhead_x >= grid_x && playhead_x <= grid_x + grid_width {
                    // Line runs from the top of the ruler down through the grid.
                    painter.line_segment(
                        [
                            Pos2::new(playhead_x, origin.y),
                            Pos2::new(playhead_x, grid_y + total_content_height),
                        ],
                        Stroke::new(1.5, t.colors.accent_primary),
                    );
                    // Triangle marker in the ruler strip.
                    let tri_size = 6.0;
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            Pos2::new(playhead_x - tri_size, origin.y),
                            Pos2::new(playhead_x + tri_size, origin.y),
                            Pos2::new(playhead_x, origin.y + RULER_HEIGHT * 0.6),
                        ],
                        t.colors.accent_primary,
                        Stroke::NONE,
                    ));
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

            // ── Keyboard / grid separator ──
            painter.line_segment(
                [
                    Pos2::new(grid_x, grid_y),
                    Pos2::new(grid_x, grid_y + total_content_height),
                ],
                Stroke::new(1.0, t.colors.border),
            );

            // Automation zone rect (for hit-testing)
            let auto_rect = if view_state.selected_automation.is_some() {
                Some(Rect::from_min_size(
                    Pos2::new(grid_x, auto_y),
                    Vec2::new(grid_width, AUTOMATION_ZONE_HEIGHT),
                ))
            } else {
                None
            };

            // ── Hover and cursor ──
            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                // Check automation zone hover first
                if auto_rect.is_some_and(|r| r.contains(pos)) {
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
                                let beats = note.start_tick.0 as f32
                                    / synth_sequencer::TICKS_PER_QUARTER as f32;
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
                                    .find(|inst| inst.id.0 == note_instrument.0 as u64)
                                    .map_or_else(
                                        || format!("#{}", note_instrument.0),
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
                                        RichText::new(format!(
                                            "{pitch_name:?}{octave}  (MIDI {midi})"
                                        ))
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
                &coords,
            );
        });

    // Detect manual scrolling to disable auto-follow, mirroring the
    // arrangement timeline: if the offset after the scroll area differs from
    // what auto-follow set, the user dragged the scrollbar — stop fighting.
    if is_playing {
        let actual_offset = scroll_output.state.offset.x;
        if let Some(expected) = view_state.pr_last_auto_scroll_offset
            && (actual_offset - expected).abs() > 2.0
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
) -> bool {
    let data = ctx.data;
    let song = ctx.song;
    let view_state = &mut *ctx.view_state;
    let handle = &mut *ctx.handle;
    let undo_manager = &mut *ctx.undo_manager;
    let instruments = ctx.instruments;
    let t = theme();
    let mut keep_open = true;

    // ── Toolbar ──
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

        // Pattern name (editable via rename context menu)
        if view_state.editing_pattern_name.as_ref().map(|(id, _)| *id) == Some(data.pattern_id) {
            if let Some((_, ref mut name_buf)) = view_state.editing_pattern_name {
                let resp = ui.add(
                    egui::TextEdit::singleline(name_buf)
                        .desired_width(120.0)
                        .font(egui::FontId::proportional(14.0)),
                );
                if resp.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    commit_pattern_rename(song, undo_manager, data.pattern_id, name_buf.clone());
                    view_state.editing_pattern_name = None;
                } else if !resp.has_focus() {
                    resp.request_focus();
                }
            }
        } else {
            let name_resp = ui.add(
                egui::Label::new(
                    RichText::new(&data.pattern_name)
                        .size(14.0)
                        .color(t.colors.accent_cyan),
                )
                .sense(Sense::click()),
            );
            if name_resp.double_clicked() {
                view_state.editing_pattern_name =
                    Some((data.pattern_id, data.pattern_name.clone()));
            }
        }

        // Pattern description (editable inline; utility metadata, no undo)
        if view_state
            .editing_pattern_description
            .as_ref()
            .map(|(id, _)| *id)
            == Some(data.pattern_id)
        {
            if let Some((_, ref mut desc_buf)) = view_state.editing_pattern_description {
                let resp = ui.add(
                    egui::TextEdit::singleline(desc_buf)
                        .desired_width(180.0)
                        .hint_text("Description")
                        .font(egui::FontId::proportional(12.0)),
                );
                // Commit on every change so switching patterns mid-edit can't
                // strand the buffer; lost_focus (incl. Enter on a singleline)
                // ends the edit session.
                if resp.changed() {
                    commit_pattern_description(song, data.pattern_id, desc_buf.clone());
                }
                if resp.lost_focus() {
                    view_state.editing_pattern_description = None;
                } else if !resp.has_focus() {
                    resp.request_focus();
                }
            }
        } else {
            let desc_text = if data.pattern_description.is_empty() {
                RichText::new("+ description")
                    .size(12.0)
                    .italics()
                    .color(t.colors.text_dim)
            } else {
                RichText::new(&data.pattern_description)
                    .size(12.0)
                    .color(t.colors.text_secondary)
            };
            let desc_resp = ui
                .add(egui::Label::new(desc_text).sense(Sense::click()))
                .on_hover_text("Double-click to edit pattern description");
            if desc_resp.double_clicked() {
                view_state.editing_pattern_description =
                    Some((data.pattern_id, data.pattern_description.clone()));
            }
        }

        // Pattern length (whole bars)
        {
            let ticks_per_bar = data.time_sig.ticks_per_bar().max(1);
            let mut bars = (data.length_ticks.0 / ticks_per_bar).max(1) as i32;
            let bars_resp = ui
                .add(
                    egui::DragValue::new(&mut bars)
                        .range(1..=64)
                        .speed(0.1)
                        .suffix(" bars"),
                )
                .on_hover_text("Pattern length in bars");
            if bars_resp.drag_started() || bars_resp.gained_focus() {
                view_state.pattern_length_drag_start = Some((data.pattern_id, data.length_ticks));
            }
            if bars_resp.changed() {
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

        // Note FX rack toggle (badge = processor count for this pattern).
        let np_count = song
            .try_read()
            .and_then(|s| s.pattern(data.pattern_id).map(|p| p.processors().len()))
            .unwrap_or(0);
        if ui
            .selectable_label(
                view_state.note_fx_panel_open,
                format!("Note FX ({np_count})"),
            )
            .on_hover_text("Show/hide the note-processor rack for this pattern")
            .clicked()
        {
            view_state.note_fx_panel_open = !view_state.note_fx_panel_open;
        }
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
            if ui
                .button(RichText::new(egui_remixicon::icons::CLOSE_LINE).color(t.colors.accent_red))
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
                ui.label(RichText::new("Piano-roll keyboard shortcuts").strong());
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

    ui.add_space(t.spacing.xxs);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

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
        ui.label(RichText::new("Vel:").color(t.colors.text_dim).size(10.0));
        let mut vel_pct = (view_state.default_velocity.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut vel_pct)
                    .range(1.0..=100.0)
                    .speed(1.0)
                    .suffix(" %"),
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
        ui.label(RichText::new("Sw:").color(t.colors.text_dim).size(10.0));
        let mut sw_pct = (view_state.swing_amount.as_f32() * 100.0).round();
        if ui
            .add(
                egui::DragValue::new(&mut sw_pct)
                    .range(0.0..=100.0)
                    .speed(1.0)
                    .suffix(" %"),
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
            ui.label(RichText::new("Zoom").color(t.colors.text_dim).size(10.0));
        });
    });

    keep_open
}

fn handle_piano_roll_interaction(
    ctx: &mut PianoRollCtx<'_>,
    response: &egui::Response,
    ui: &egui::Ui,
    grid_rect: Rect,
    auto_rect: Option<Rect>,
    auto_y: f32,
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

        let new_point = AutomationPoint::new(tick, value);
        let curve = new_point.curve;
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

    // ── Automation right-click (delete point) ──
    if response.secondary_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(ar) = auto_rect
        && ar.contains(pos)
        && let Some(target) = &view_state.selected_automation
    {
        let target = target.clone();
        // Find the lane snapshot to hit-test against
        if let Some(lane) = data.automation_lanes.iter().find(|l| l.target == target)
            && let Some(idx) = automation_point_at_pos(lane, pos, tick_to_x, auto_y)
        {
            let pt = &lane.points[idx];
            let point_tick = pt.tick;
            let point_value = pt.value;
            let point_curve = pt.curve;
            {
                let mut song_w = song.write();
                if let Some(pattern) = song_w.pattern_mut(data.pattern_id)
                    && let Some(auto_lane) =
                        pattern.automation.iter_mut().find(|l| l.target == target)
                {
                    auto_lane.remove_point(point_tick);
                }
            }
            undo_manager.push(crate::undo::UndoAction::RemoveAutomationPoint {
                pattern_id: data.pattern_id,
                target,
                tick: point_tick,
                value: point_value,
                curve: point_curve,
            });
        }
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

    // ── Automation drag start ──
    if response.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        && let Some(ar) = auto_rect
        && ar.contains(pos)
        && let Some(target) = &view_state.selected_automation
    {
        let target = target.clone();
        if let Some(lane) = data.automation_lanes.iter().find(|l| l.target == target)
            && let Some(idx) = automation_point_at_pos(lane, pos, tick_to_x, auto_y)
        {
            let pt = &lane.points[idx];
            view_state.drag = Some(DragState::DragAutomationPoint {
                target,
                original_tick: pt.tick,
                original_value: pt.value,
                current_tick: pt.tick,
                current_value: pt.value,
            });
        }
    }

    // ── Velocity drag start ──
    if response.drag_started()
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
    {
        let vel_y = grid_rect.max.y;
        let vel_rect = Rect::from_min_size(
            Pos2::new(grid_rect.min.x, vel_y),
            Vec2::new(grid_rect.width(), VELOCITY_ZONE_HEIGHT),
        );
        if vel_rect.contains(pos) {
            view_state.drag = Some(DragState::DragVelocity {
                last_note_id: None,
                initial_velocities: std::collections::HashMap::new(),
            });
        }
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
                ..
            }) => {
                let raw_tick = x_to_tick(pos.x);
                *current_tick = quantize_tick(raw_tick, data.ticks_per_row);
                *current_value = NormalizedValue::new(
                    (1.0 - (pos.y - auto_y) / AUTOMATION_ZONE_HEIGHT).clamp(0.0, 1.0),
                );
            }
            Some(DragState::DragVelocity { .. }) => {
                // Velocity painting handled in real-time below
            }
            Some(DragState::DragPlacement { .. })
            | Some(DragState::ResizePlacement { .. })
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
        // vel_y is grid_rect.max.y, VELOCITY_ZONE_HEIGHT below
        let vel_y = grid_rect.max.y;
        let new_vel = (1.0 - (pos.y - vel_y) / VELOCITY_ZONE_HEIGHT).clamp(0.01, 1.0);
        // Find note at this x position
        let click_tick = x_to_tick(pos.x);
        let nearest_note = data.notes.iter().min_by_key(|n| {
            let mid = n.start_tick.0 + n.end_tick.map_or(0, |e| (e.0 - n.start_tick.0) / 2);
            (mid as i64 - click_tick.0 as i64).unsigned_abs()
        });
        if let Some(note) = nearest_note {
            let note_x = tick_to_x(note.start_tick);
            if (note_x - pos.x).abs() < 15.0 {
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
            } => {
                // Apply automation point move
                if current_tick != original_tick
                    || (current_value.as_f32() - original_value.as_f32()).abs() > f32::EPSILON
                {
                    let new_point = AutomationPoint::new(current_tick, current_value);
                    let curve = new_point.curve;
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
            // Placement drag + resize are handled in the arrangement view.
            DragState::DragPlacement { .. } | DragState::ResizePlacement { .. } => {}
        }
    }
}
