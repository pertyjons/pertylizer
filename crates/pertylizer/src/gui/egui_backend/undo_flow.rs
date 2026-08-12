//! undo flow responsibilities for the egui application.

use super::*;
use crate::app_services::{SongMutationService, TempoPointEdit};

impl SynthApp {
    /// Handle Ctrl+C (copy), Ctrl+V (paste), Ctrl+X (cut) keyboard shortcuts.
    ///
    /// These stay view-local — they act on the patch canvas selection, which
    /// only the Rack view has — but they take the same input gate as the
    /// application shortcuts so a modal or a focused text field silences them
    /// too.
    pub(super) fn handle_clipboard_shortcuts(
        &mut self,
        ctx: &egui::Context,
        gate: crate::gui::shortcuts::InputGate,
    ) {
        // Only handle clipboard shortcuts in Rack view
        if self.active_view != AppView::Rack {
            return;
        }

        if !gate.allows_app_shortcuts() {
            return;
        }

        let (ctrl_c, ctrl_v, ctrl_x) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            (
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::C),
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::V),
                cmd && !i.modifiers.shift && i.key_pressed(egui::Key::X),
            )
        });

        if ctrl_c {
            self.copy_selected_modules();
        } else if ctrl_v {
            self.paste_modules_at_offset();
        } else if ctrl_x {
            self.cut_selected_modules();
        }
    }

    /// Copy selected modules and their internal connections to the clipboard.
    pub(super) fn copy_selected_modules(&mut self) {
        let Some(editor) = self.active_patch_editor_ref() else {
            return;
        };
        let selection = editor.effective_selection();
        if selection.is_empty() {
            return;
        }
        let module_states = editor.extract_module_states(&selection);
        let connection_states = editor.internal_connections(&selection);
        self.clipboard
            .copy_modules(&module_states, &connection_states);
    }

    /// Paste clipboard contents at an offset from the original position.
    pub(super) fn paste_modules_at_offset(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        let (modules, connections, ref_pos) = self.clipboard.contents();
        let modules = modules.to_vec();
        let connections = connections.to_vec();
        let offset = crate::gui::clipboard::ModuleClipboard::paste_offset();
        let paste_pos = (ref_pos.0 + offset, ref_pos.1 + offset);

        let Some(instrument_id) = self.active_instrument_id else {
            return;
        };

        // Access the patch editor by index to avoid borrowing all of self
        let Some(inst_idx) = self.instruments.iter().position(|i| i.id == instrument_id) else {
            return;
        };

        let new_ids = patch_bridge::paste_clipboard_modules(
            &modules,
            &connections,
            ref_pos,
            paste_pos,
            &mut self.instruments[inst_idx].patch_editor,
            &self.session,
            &mut self.handle,
            instrument_id,
        );
        self.instruments[inst_idx]
            .patch_editor
            .select_modules(&new_ids);
        self.mark_dirty();
    }

    /// Cut selected modules: copy them to the clipboard, then remove them. Mirrors
    /// the removal path in `render_rack_central`, including the automation-lane
    /// guard so a lane is never silently orphaned.
    pub(super) fn cut_selected_modules(&mut self) {
        self.copy_selected_modules();
        let Some(active_id) = self.active_instrument_id else {
            return;
        };
        let Some(idx) = self.instruments.iter().position(|i| i.id == active_id) else {
            return;
        };
        let selection = self.instruments[idx].patch_editor.effective_selection();
        if selection.is_empty() {
            return;
        }
        let seq_id = active_id;
        let mut removed_any = false;
        for module_id in selection {
            // Don't orphan an automation lane that still targets this module.
            let referenced = self.song.read().is_module_automated(
                seq_id,
                module_id.module_type,
                module_id.instance,
            );
            if referenced {
                self.dialog_state.set_status(format!(
                    "Can't cut {:?} #{}: an automation lane targets it. Delete the lane first.",
                    module_id.module_type, module_id.instance
                ));
                continue;
            }
            let has_vis_buffer = self.instruments[idx]
                .patch_editor
                .module_descriptor(module_id)
                .is_some_and(|d| {
                    d.category == ModuleCategory::Visualizer
                        || d.type_id.as_str() == "signal_monitor"
                        || d.type_id.as_str() == "inline_signal_monitor"
                });
            if let Err(e) = self.session.remove_module(active_id, module_id) {
                eprintln!("Failed to remove module {module_id:?}: {e}");
                continue;
            }
            self.instruments[idx].patch_editor.remove_module(module_id);
            if has_vis_buffer {
                self.handle.remove_visualization_buffer(module_id);
            }
            removed_any = true;
        }
        if removed_any {
            self.mark_dirty();
        }
    }

    /// Optimize the project by removing unused patterns, tracks, and instruments.
    pub(super) fn optimize_project(&mut self) {
        // Remove unused patterns and tracks from the song
        let (removed_patterns, removed_tracks, used_instrument_ids) = {
            let mut song = self.song.write();
            song.remove_unused()
        };

        // Remove instruments not referenced by remaining tracks/notes
        let to_remove: Vec<_> = self
            .instruments
            .iter()
            .filter(|inst| {
                let seq_id = inst.id;
                !used_instrument_ids.contains(&seq_id)
            })
            .map(|inst| inst.id)
            .collect();
        let removed_instruments = to_remove.len();
        for inst_id in &to_remove {
            if let Err(e) = self.session.remove_instrument(*inst_id) {
                eprintln!("Failed to remove instrument {inst_id:?}: {e}");
            }
        }
        self.instruments
            .retain(|inst| !to_remove.contains(&inst.id));

        // If active instrument was removed, select first remaining
        if self
            .active_instrument_id
            .is_some_and(|id| !self.instruments.iter().any(|i| i.id == id))
        {
            self.active_instrument_id = self.instruments.first().map(|i| i.id);
            self.handle
                .set_focused_instrument(self.active_instrument_id);
        }

        // Drop samples no remaining Sampler references — keeps the
        // library empty when nothing uses it so the next save can stay
        // on plain JSON instead of being forced into bundle format.
        let removed_samples =
            crate::project_apply::prune_unused_samples(&self.session, &self.sample_library);

        let total = removed_patterns.len()
            + removed_tracks.len()
            + removed_instruments
            + removed_samples.len();
        if total > 0 {
            eprintln!(
                "Optimized project: removed {} patterns, {} tracks, {removed_instruments} instruments, {} samples",
                removed_patterns.len(),
                removed_tracks.len(),
                removed_samples.len(),
            );
        }
    }

    /// Execute an undo operation by popping the undo stack and applying the inverse.
    pub(super) fn execute_undo(&mut self) {
        self.refresh_added_effect_snapshot();
        if let Some(action) = self.undo_manager.undo() {
            self.apply_undo_action(&action);
        }
    }

    /// Bring the top entry's effect snapshot up to date, if it is an addition.
    ///
    /// An addition records the effect as it was *created*, carrying the
    /// descriptor's defaults, because that is all undoing an addition needs:
    /// take it off the chain again. Redo replays that same entry, though, so
    /// anything changed on the effect afterwards outside this manager — an MCP
    /// `set_master_effect_parameter`, a chain reorder — would come back as
    /// defaults appended to the end. Reading the live chain at the moment the
    /// addition is undone makes the entry describe the effect as it actually
    /// was when it went away.
    ///
    /// Peeked twice rather than once so the live read sits between the two
    /// borrows of the manager.
    fn refresh_added_effect_snapshot(&mut self) {
        use crate::undo::UndoAction;

        let Some((chain, module_id)) =
            self.undo_manager
                .peek_undo_mut()
                .and_then(|action| match action {
                    UndoAction::SetChainEffect {
                        chain,
                        old: None,
                        new: Some(snapshot),
                    } => Some((*chain, snapshot.module_id)),
                    _ => None,
                })
        else {
            return;
        };
        let Some(fresh) = self.live_chain_effect(chain, module_id) else {
            // Something already took the effect off the chain, so there is no
            // live state to read; the entry keeps what it captured and its
            // removal command lands as a no-op.
            return;
        };
        if let Some(UndoAction::SetChainEffect {
            new: Some(snapshot),
            ..
        }) = self.undo_manager.peek_undo_mut()
        {
            **snapshot = fresh;
        }
    }

    /// What an effect on a return-bus or master chain looks like right now,
    /// including the chain's slot order so a restore puts it back where it sat.
    fn live_chain_effect(
        &self,
        chain: crate::undo::EffectChain,
        module_id: synth_engine::ModuleId,
    ) -> Option<crate::undo::EffectSnapshot> {
        fn snapshot_of(
            effects: &[synth_engine::ReturnEffectSnapshot],
            module_id: synth_engine::ModuleId,
        ) -> Option<crate::undo::EffectSnapshot> {
            let effect = effects.iter().find(|e| e.module_id == module_id)?;
            Some(crate::undo::EffectSnapshot {
                chain_order: effects.iter().map(|e| e.module_id).collect(),
                module_id,
                module_type: effect.module_type,
                params: effect.parameters.clone(),
                bypassed: effect.bypassed,
            })
        }

        match chain {
            crate::undo::EffectChain::Master => {
                snapshot_of(&self.handle.state.master_effects.read(), module_id)
            }
            crate::undo::EffectChain::Return(return_id) => {
                let busses = self.handle.state.return_bus_effects.read();
                let bus = busses.iter().find(|bus| bus.id == return_id)?;
                snapshot_of(&bus.effects, module_id)
            }
        }
    }

    /// Execute a redo operation by popping the redo stack and re-applying.
    pub(super) fn execute_redo(&mut self) {
        if let Some(action) = self.undo_manager.redo() {
            self.apply_undo_action(&action);
        }
    }

    /// Drop one cable from both the patch editor and the engine.
    ///
    /// `index` is the instrument's position in `self.instruments`, already
    /// resolved by the caller.
    fn disconnect_rack_cable(
        &mut self,
        instrument_id: InstrumentId,
        index: usize,
        connection: synth_engine::graph::Connection,
    ) {
        self.instruments[index]
            .patch_editor
            .remove_connection(&connection);
        self.handle.send(EngineCommand::Disconnect {
            instrument_id: Some(instrument_id),
            from: PortId::new(connection.from_module, connection.from_port),
            to: PortId::new(connection.to_module, connection.to_port),
        });
    }

    /// Apply an undo/redo action to the current state.
    #[allow(clippy::too_many_lines)]
    pub(super) fn apply_undo_action(&mut self, action: &crate::undo::UndoAction) {
        use crate::undo::UndoAction;
        match action {
            UndoAction::SetNoteGraph { graph_id, new, .. } => {
                let mut song_w = self.song.write();
                match new {
                    Some(graph) => {
                        if let Some(existing) = song_w.note_graph_mut(*graph_id) {
                            *existing = graph.clone();
                        } else {
                            song_w.insert_note_graph(graph.clone());
                        }
                    }
                    None => {
                        song_w.remove_note_graph(*graph_id);
                    }
                }
            }
            UndoAction::SetModGraph { graph_id, new, .. } => {
                let mut song_w = self.song.write();
                match new {
                    Some(graph) => {
                        if let Some(existing) = song_w.mod_graph_mut(*graph_id) {
                            *existing = graph.clone();
                        } else {
                            song_w.insert_mod_graph(graph.clone());
                        }
                    }
                    None => {
                        song_w.remove_mod_graph(*graph_id);
                    }
                }
            }
            UndoAction::SetPatternNoteGraph {
                pattern_id, new, ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.set_note_graph(*new);
                }
            }
            UndoAction::AddNote { pattern_id, note } => {
                // `restore_note`, not `add_note`: the note comes back under its
                // own id, so a `NoteId` held across the undo still resolves and
                // `next_note_id` does not climb once per delete/undo cycle. It
                // also carries every field, including the per-note ornament,
                // glide, expression, legato flag and note-scope graph that the
                // old field-by-field re-add dropped.
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.restore_note(note.clone());
                }
            }
            UndoAction::RemoveNote { pattern_id, note } => {
                // Remove the note from the pattern.
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.remove_note(note.id);
                }
            }
            UndoAction::MoveNote {
                pattern_id,
                note_id,
                new_start,
                ..
            } => {
                // Move note to the target position (new_start is the destination).
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.move_note(*note_id, *new_start);
                }
            }
            UndoAction::ResizeNote {
                pattern_id,
                note_id,
                new_duration,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    if let Some(dur) = new_duration {
                        pattern.resize_note(*note_id, *dur);
                    } else if let Some(n) = pattern.note_mut(*note_id) {
                        n.duration = None;
                    }
                }
            }
            UndoAction::TransposeNote {
                pattern_id,
                note_id,
                new_pitch,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id)
                    && let Some(n) = pattern.note_mut(*note_id)
                {
                    n.pitch = *new_pitch;
                }
            }
            UndoAction::SetNoteVelocity {
                pattern_id,
                note_id,
                new_velocity,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.set_note_velocity(*note_id, *new_velocity);
                }
            }
            UndoAction::SetVelocitiesBatch {
                pattern_id,
                changes,
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    for (note_id, _old, new) in changes {
                        pattern.set_note_velocity(*note_id, *new);
                    }
                }
            }
            UndoAction::SetLegatoBatch {
                pattern_id,
                changes,
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    for (note_id, _old, new) in changes {
                        pattern.set_note_legato(*note_id, *new);
                    }
                }
            }
            UndoAction::SetGlideBatch {
                pattern_id,
                changes,
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    for (note_id, _old, new) in changes {
                        pattern.set_note_glide(*note_id, *new);
                    }
                }
            }
            UndoAction::SetExpressionBatch {
                pattern_id,
                changes,
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    for (note_id, _old, new) in changes {
                        pattern.set_note_expression(*note_id, *new);
                    }
                }
            }
            UndoAction::SetLaneBatch {
                pattern_id,
                changes,
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    for (note_id, _old, new) in changes {
                        pattern.set_note_lane(*note_id, *new);
                    }
                }
            }
            UndoAction::SetNoteGraphBindingBatch {
                pattern_id,
                changes,
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    for (note_id, _old, new) in changes {
                        pattern.set_note_note_graph(*note_id, *new);
                    }
                }
            }
            UndoAction::FreezePattern { pattern_id, .. } => {
                let mut song_w = self.song.write();
                let bpm = song_w.tempo_at(synth_sequencer::Tick(0));
                // Song::freeze_pattern owns the graph-over-rack precedence.
                song_w.freeze_pattern(*pattern_id, bpm);
            }
            UndoAction::RestorePattern {
                pattern_id,
                snapshot,
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    *pattern = snapshot.clone();
                }
            }
            UndoAction::SetNoteOrnament {
                pattern_id,
                note_id,
                new,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(note) = song_w
                    .pattern_mut(*pattern_id)
                    .and_then(|p| p.note_mut(*note_id))
                {
                    note.ornament = *new;
                }
            }
            UndoAction::RenamePattern {
                pattern_id,
                new_name,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.name = new_name.clone();
                }
            }
            UndoAction::RenameTrack {
                track_id, new_name, ..
            } => {
                let mut song_w = self.song.write();
                if let Some(track) = song_w.track_mut(*track_id) {
                    track.name = new_name.clone();
                }
            }
            UndoAction::SetPatternLength {
                pattern_id,
                new_length,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.length = *new_length;
                }
            }
            UndoAction::SetTempo { tick, new, .. } => {
                SongMutationService::new(&self.song).apply_tempo_point(*tick, *new);
            }
            UndoAction::MoveTempo { old, new } => {
                SongMutationService::new(&self.song)
                    .move_tempo_point(old.0, TempoPointEdit::new(new.0, new.1, new.2));
            }
            UndoAction::MovePlacement {
                pattern_id,
                old_track_id,
                old_start,
                new_track_id,
                new_start,
            } => {
                let mut song_w = self.song.write();
                song_w.move_placement(
                    *pattern_id,
                    *old_track_id,
                    *old_start,
                    *new_track_id,
                    *new_start,
                );
            }
            UndoAction::AddAutomationPoint {
                pattern_id,
                target,
                tick,
                value,
                curve,
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    let lane = pattern.get_or_create_automation(target.clone());
                    let mut point = synth_sequencer::AutomationPoint::new(*tick, *value);
                    point.curve = *curve;
                    lane.add_point(point);
                }
            }
            UndoAction::RemoveAutomationPoint {
                pattern_id,
                target,
                tick,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id)
                    && let Some(lane) = pattern.automation.iter_mut().find(|l| &l.target == target)
                {
                    lane.remove_point(*tick);
                }
            }
            UndoAction::DeleteTrack {
                track, placements, ..
            } => {
                let mut song_w = self.song.write();
                song_w.delete_track(track.id);
                let _ = placements;
            }
            UndoAction::AddTrack {
                track,
                track_index,
                placements,
            } => {
                let mut song_w = self.song.write();
                song_w.insert_track(track.clone(), Some(*track_index));
                for p in placements {
                    song_w.insert_placement(p.clone());
                }
            }
            UndoAction::DeletePattern {
                pattern,
                placements,
            } => {
                let mut song_w = self.song.write();
                song_w.delete_pattern(pattern.id);
                let _ = placements;
            }
            UndoAction::AddPattern {
                pattern,
                placements,
            } => {
                let mut song_w = self.song.write();
                song_w.insert_pattern(pattern.clone());
                for p in placements {
                    song_w.insert_placement(p.clone());
                }
            }
            UndoAction::RemovePlacement { placement } => {
                let mut song_w = self.song.write();
                song_w.remove_placement(placement.pattern_id, placement.track_id, placement.start);
            }
            UndoAction::InsertPlacement { placement } => {
                let mut song_w = self.song.write();
                song_w.insert_placement(placement.clone());
            }
            UndoAction::SetPlacementLength {
                pattern_id,
                track_id,
                start,
                new_length,
                ..
            } => {
                let mut song_w = self.song.write();
                song_w.set_placement_length(*pattern_id, *track_id, *start, *new_length);
            }
            UndoAction::SetPlacementLoopMode {
                pattern_id,
                track_id,
                start,
                new_mode,
                ..
            } => {
                let mut song_w = self.song.write();
                song_w.set_placement_loop_mode(*pattern_id, *track_id, *start, *new_mode);
            }
            UndoAction::SetArrangementSections { new, .. } => {
                self.song.write().replace_sections(new.clone());
            }
            UndoAction::MoveAutomationPoint {
                pattern_id,
                target,
                old_tick,
                new_tick,
                new_value,
                curve,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    let lane = pattern.get_or_create_automation(target.clone());
                    lane.remove_point(*old_tick);
                    let mut point = synth_sequencer::AutomationPoint::new(*new_tick, *new_value);
                    point.curve = *curve;
                    lane.add_point(point);
                }
            }
            UndoAction::SetAutomationPointCurve {
                pattern_id,
                target,
                tick,
                value,
                new_curve,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    // add_point replaces the point at the same tick in place.
                    let lane = pattern.get_or_create_automation(target.clone());
                    lane.add_point(
                        synth_sequencer::AutomationPoint::new(*tick, *value).with_curve(*new_curve),
                    );
                }
            }
            UndoAction::AddAutomationLane { pattern_id, lane } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.add_automation_lane(lane.clone());
                }
            }
            UndoAction::RemoveAutomationLane { pattern_id, lane } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    pattern.remove_automation_lane(&lane.target);
                }
            }
            UndoAction::AddConnection {
                instrument_id,
                connection,
            } => {
                if let Some(editor) = self.active_patch_editor() {
                    editor.add_connection(*connection);
                }
                self.handle.send(EngineCommand::Connect {
                    instrument_id: Some(*instrument_id),
                    from: synth_engine::commands::PortId::new(
                        connection.from_module,
                        connection.from_port,
                    ),
                    to: synth_engine::commands::PortId::new(
                        connection.to_module,
                        connection.to_port,
                    ),
                });
            }
            UndoAction::RemoveConnection {
                instrument_id,
                connection,
            } => {
                if let Some(editor) = self.active_patch_editor() {
                    editor.remove_connection(connection);
                }
                self.handle.send(EngineCommand::Disconnect {
                    instrument_id: Some(*instrument_id),
                    from: synth_engine::commands::PortId::new(
                        connection.from_module,
                        connection.from_port,
                    ),
                    to: synth_engine::commands::PortId::new(
                        connection.to_module,
                        connection.to_port,
                    ),
                });
            }
            // ── Mixer ──
            //
            // Faders, pans, mutes, solos and sends all live in the `Song`, which
            // the engine reads through its lock-free snapshot, so those need no
            // separate engine command — the write guard republishes on drop.
            // The *existence* of a return bus is the exception: the engine keeps
            // its own registry (its effect chain and mix buffer), so
            // creating/removing one has to be mirrored with a command, exactly
            // as `mixer_view::apply_mutation` does on the forward path.
            UndoAction::SetTrackMixer {
                track_id,
                param,
                new,
                ..
            } => {
                if let Some(track) = self.song.write().track_mut(*track_id) {
                    apply_mixer_value(
                        *param,
                        *new,
                        &mut track.volume,
                        &mut track.pan,
                        &mut track.mute,
                        &mut track.solo,
                    );
                }
            }
            UndoAction::SetTrackSend {
                track_id,
                return_bus,
                new,
                ..
            } => {
                let mut song_w = self.song.write();
                if let Some(track) = song_w.track_mut(*track_id) {
                    track.sends.retain(|send| send.target != *return_bus);
                    if let Some(send) = new {
                        track.sends.push(*send);
                    }
                }
            }
            UndoAction::SetReturnBusMixer {
                bus_id, param, new, ..
            } => {
                if let Some(bus) = self.song.write().return_bus_mut(*bus_id) {
                    apply_mixer_value(
                        *param,
                        *new,
                        &mut bus.volume,
                        &mut bus.pan,
                        &mut bus.mute,
                        &mut bus.solo,
                    );
                }
            }
            UndoAction::SetMasterVolume { new, .. } => {
                if let crate::undo::MixerValue::Level(level) = new {
                    self.handle
                        .send(EngineCommand::SetMasterVolume(synth_core::Gain::new(
                            level.as_f32(),
                        )));
                } else {
                    tracing::warn!(
                        target: "pertylizer::undo",
                        ?new,
                        "master-volume undo entry does not carry a level",
                    );
                }
            }
            UndoAction::SetReturnSend {
                from, target, new, ..
            } => {
                let mut song_w = self.song.write();
                if let Some(bus) = song_w.return_bus_mut(*from) {
                    bus.sends.retain(|send| send.target != *target);
                    if let Some(send) = new {
                        bus.sends.push(*send);
                    }
                }
            }
            UndoAction::SetReturnBus {
                effects,
                bus_id,
                index,
                new,
                ..
            } => {
                {
                    let mut song_w = self.song.write();
                    match new {
                        // Restoring a deleted bus puts back the whole thing —
                        // name, colour, fader, sends — at the position it held,
                        // not a blank bus appended to the end.
                        Some(bus) => song_w.restore_return_bus(*index, (**bus).clone()),
                        None => {
                            song_w.delete_return_bus(*bus_id);
                        }
                    }
                }
                // The engine owns the bus itself (its effect chain and mix
                // buffer), so without this the restored bus exists only in the
                // song: every send routed to it would produce silence, and an
                // undone *creation* would leave a phantom bus in the engine.
                if new.is_some() {
                    self.handle
                        .send(EngineCommand::CreateReturnBus { id: *bus_id });
                    // The insert chain is engine-side too, and went down with
                    // the bus. Rebuild it after the bus exists to route to.
                    for effect in effects {
                        for command in
                            effect.restore_commands(crate::undo::EffectChain::Return(*bus_id))
                        {
                            self.handle.send(command);
                        }
                    }
                } else {
                    // Removing the bus takes its chain with it, so the effects
                    // need no separate teardown.
                    self.handle
                        .send(EngineCommand::RemoveReturnBus { id: *bus_id });
                }
            }

            UndoAction::SwapPattern {
                pattern_id, new, ..
            } => {
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    *pattern = (**new).clone();
                }
            }
            UndoAction::SetTimeSignature { new, .. } => {
                self.song.write().default_time_signature = *new;
            }

            // ── Instrument ──
            //
            // Two writes, because the state is mirrored: the GUI's
            // `InstrumentUiState` (which the save path reads) and the engine's
            // own instrument. Writing only the first would restore what the
            // user sees while leaving the sound unchanged.
            UndoAction::SetInstrumentSettings {
                instrument_id, new, ..
            } => {
                if let Some(inst) = self.instruments.iter_mut().find(|i| i.id == *instrument_id) {
                    inst.apply_settings(new);
                }
                self.sync_instrument_settings_to_engine(*instrument_id);
            }

            // ── Return / master effect chains ──
            //
            // Entirely engine-owned: there is no GUI mirror to write, because
            // the mixer reads these chains out of shared state each frame.
            UndoAction::SetChainEffect { chain, old, new } => match new {
                Some(snapshot) => {
                    for command in snapshot.restore_commands(*chain) {
                        self.handle.send(command);
                    }
                }
                None => {
                    // Undoing an addition, or redoing a removal: either way the
                    // effect to take off the chain is the one `old` describes.
                    if let Some(snapshot) = old {
                        self.handle.send(chain.remove(snapshot.module_id));
                    }
                }
            },
            UndoAction::SetChainEffectParameter {
                chain,
                module_id,
                new,
                ..
            } => {
                self.handle.send(chain.set_param(*module_id, *new));
            }
            UndoAction::SetChainEffectBypass {
                chain,
                module_id,
                new,
                ..
            } => {
                // The command takes *enabled*, the inverse of bypassed.
                self.handle.send(chain.set_enabled(*module_id, !*new));
            }
            UndoAction::SetEffectChainOrder {
                instrument_id, new, ..
            } => {
                self.handle.send(EngineCommand::SetEffectChainOrder {
                    instrument_id: *instrument_id,
                    order: new.clone(),
                });
            }

            // Rack structure. Restoring goes through the same id-preserving
            // primitive project loading uses, so a brought-back module answers
            // to the id its cables and automation lanes still reference.
            UndoAction::SetRackModules {
                instrument_id,
                modules,
                connections,
                severed,
                restore,
            } => {
                let Some(index) = self.instruments.iter().position(|i| i.id == *instrument_id)
                else {
                    return;
                };
                if *restore {
                    for module_state in modules {
                        patch_bridge::populate_editor_module(
                            module_state,
                            &mut self.instruments[index].patch_editor,
                            &self.session,
                            &mut self.handle,
                            *instrument_id,
                        );
                    }
                    // Whatever the addition displaced goes first: an inline
                    // insert sits on a cable, and leaving the old cable in place
                    // would route the source to both the new module and the old
                    // destination.
                    for connection in severed {
                        self.disconnect_rack_cable(*instrument_id, index, *connection);
                    }
                    // Cables come back after the modules they attach to, or the
                    // engine would reject an edge to a port that does not exist
                    // yet.
                    for connection in connections {
                        self.instruments[index]
                            .patch_editor
                            .add_connection(*connection);
                        self.handle.send(EngineCommand::Connect {
                            instrument_id: Some(*instrument_id),
                            from: PortId::new(connection.from_module, connection.from_port),
                            to: PortId::new(connection.to_module, connection.to_port),
                        });
                    }
                } else {
                    for module_state in modules {
                        let Ok(module_id) = module_state.id.parse::<ModuleId>() else {
                            continue;
                        };
                        if let Err(e) = self.session.remove_module(*instrument_id, module_id) {
                            tracing::warn!(
                                target: "pertylizer::undo",
                                module = %module_id,
                                error = %e,
                                "could not remove a module while redoing",
                            );
                            continue;
                        }
                        self.instruments[index]
                            .patch_editor
                            .remove_module(module_id);
                        self.handle.remove_visualization_buffer(module_id);
                    }
                    // Removing the modules took their cables with them; the
                    // cable the addition had cut goes back down, or undoing an
                    // inline insert would leave the chain it sat in broken.
                    for connection in severed {
                        self.instruments[index]
                            .patch_editor
                            .add_connection(*connection);
                        self.handle.send(EngineCommand::Connect {
                            instrument_id: Some(*instrument_id),
                            from: PortId::new(connection.from_module, connection.from_port),
                            to: PortId::new(connection.to_module, connection.to_port),
                        });
                    }
                }
            }

            // Module parameters live in the engine, so the GUI's cached
            // value has to be written too — otherwise the knob would snap back
            // on the next frame's sync and undo the undo.
            UndoAction::SetModuleParameter {
                instrument_id,
                module_id,
                new,
                ..
            } => {
                if module_id.module_type.is_effect() {
                    self.handle.send(EngineCommand::SetEffectParameter {
                        instrument_id: Some(*instrument_id),
                        module_id: *module_id,
                        param: *new,
                    });
                } else {
                    self.handle.send(EngineCommand::SetModuleParameter {
                        instrument_id: Some(*instrument_id),
                        module_id: *module_id,
                        param: *new,
                    });
                }
                if let Some(editor) = self
                    .instruments
                    .iter_mut()
                    .find(|i| i.id == *instrument_id)
                    .map(|i| &mut i.patch_editor)
                {
                    editor.sync_module_params(*module_id, std::slice::from_ref(new));
                }
            }

            // ── Sample library ──
            //
            // The engine reads sample audio through `Arc` handles it was given
            // when the sampler module was built, so writing the library is not
            // enough on its own — a sampler already holding the old buffer
            // keeps playing it. `invalidate_peaks` refreshes the waveform view;
            // re-arming the engine's handles is the sampler's own reload path,
            // which runs when the graph next syncs.
            UndoAction::SetSample { id, new, .. } => {
                if let Ok(mut lib) = self.sample_library.write() {
                    match new {
                        Some(sample) => {
                            lib.add_with_id((**sample).clone(), *id);
                        }
                        None => {
                            lib.remove(*id);
                        }
                    }
                }
                self.sample_view_state.invalidate_peaks();
            }
            UndoAction::SetSampleMeta { id, new, .. } => {
                if let Ok(mut lib) = self.sample_library.write() {
                    lib.update_meta(*id, (**new).clone());
                }
                self.sample_view_state.invalidate_peaks();
            }
            UndoAction::SetSampleData { id, new, .. } => {
                if let Ok(mut lib) = self.sample_library.write() {
                    lib.replace_data(*id, std::sync::Arc::clone(new));
                }
                self.sample_view_state.invalidate_peaks();
            }

            UndoAction::Composite(actions) => {
                for sub_action in actions {
                    self.apply_undo_action(sub_action);
                }
            }
        }
    }
}

/// Write one channel-strip value into whichever field it belongs to.
///
/// Tracks and return buses have the same four controls with the same types but
/// no shared trait, so the fields come in as separate `&mut`s rather than the
/// owning struct.
fn apply_mixer_value(
    param: crate::undo::TrackMixerParam,
    value: crate::undo::MixerValue,
    volume: &mut synth_core::NormalizedValue,
    pan: &mut synth_core::BipolarValue,
    muted: &mut bool,
    solo: &mut bool,
) {
    use crate::undo::{MixerValue, TrackMixerParam};

    // A mismatched pair (Volume with a Flag, say) can only come from a bug in
    // whoever built the action; ignoring it keeps undo from writing nonsense
    // into the mixer.
    match (param, value) {
        (TrackMixerParam::Volume, MixerValue::Level(v)) => *volume = v,
        (TrackMixerParam::Pan, MixerValue::Balance(v)) => *pan = v,
        (TrackMixerParam::Mute, MixerValue::Flag(v)) => *muted = v,
        (TrackMixerParam::Solo, MixerValue::Flag(v)) => *solo = v,
        (param, value) => {
            tracing::warn!(
                target: "pertylizer::undo",
                param = param.as_str(),
                ?value,
                "mixer undo entry has a value that does not match its parameter",
            );
        }
    }
}
