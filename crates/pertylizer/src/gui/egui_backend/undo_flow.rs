//! undo flow responsibilities for the egui application.

use super::*;
use crate::app_services::{SongMutationService, TempoPointEdit};

impl SynthApp {
    /// Handle Ctrl+Z (undo) and Ctrl+Shift+Z (redo) keyboard shortcuts.
    pub(super) fn handle_undo_redo_shortcuts(&mut self, ctx: &egui::Context) {
        let (ctrl_z, ctrl_shift_z) = ctx.input(|i| {
            (
                i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z),
            )
        });
        if ctrl_shift_z {
            self.execute_redo();
        } else if ctrl_z {
            self.execute_undo();
        }
    }

    /// Handle Ctrl+C (copy), Ctrl+V (paste), Ctrl+X (cut) keyboard shortcuts.
    pub(super) fn handle_clipboard_shortcuts(&mut self, ctx: &egui::Context) {
        // Only handle clipboard shortcuts in Rack view
        if self.active_view != AppView::Rack {
            return;
        }

        // Skip if any text edit is focused (avoids intercepting text input)
        if ctx.text_edit_focused() {
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
        if let Some(action) = self.undo_manager.undo() {
            self.apply_undo_action(&action);
        }
    }

    /// Execute a redo operation by popping the redo stack and re-applying.
    pub(super) fn execute_redo(&mut self) {
        if let Some(action) = self.undo_manager.redo() {
            self.apply_undo_action(&action);
        }
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
                // Re-add the note to the pattern.
                let mut song_w = self.song.write();
                if let Some(pattern) = song_w.pattern_mut(*pattern_id) {
                    let nid = pattern.add_note(note.start, note.pitch, note.velocity);
                    if let Some(n) = pattern.note_mut(nid) {
                        n.duration = note.duration;
                        n.track = note.track;
                        n.lane = note.lane;
                    }
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
            UndoAction::AddModule {
                instrument_id,
                module_state,
                connections,
            } => {
                // TODO: Full undo requires recreating the module from ModuleState
                // via the session, rebuilding connections, and updating the patch
                // editor. This needs significant refactoring of the session API
                // to support module reconstruction from serialized state.
                let _ = (instrument_id, module_state, connections);
                eprintln!("Undo: AddModule not yet implemented — requires session refactoring");
            }
            UndoAction::RemoveModule {
                instrument_id,
                module_state,
                connections: _,
            } => {
                // TODO: Full undo requires re-adding the removed module from its
                // saved ModuleState and restoring all connections. Same session
                // refactoring needed as AddModule above.
                let _ = (instrument_id, module_state);
                eprintln!("Undo: RemoveModule not yet implemented — requires session refactoring");
            }
            UndoAction::MoveModule {
                module_id, new_pos, ..
            } => {
                if let Some(editor) = self.active_patch_editor() {
                    editor.set_module_position(*module_id, egui::Pos2::new(new_pos.0, new_pos.1));
                }
            }
            UndoAction::SetParameter {
                module_id,
                param_name,
                new_value,
                ..
            } => {
                // Apply the parameter value to the patch editor UI state.
                // Note: full engine-side undo for parameters is not yet implemented
                // because reconstructing the Param enum variant from a name string
                // requires module-specific knowledge.
                if let crate::patch::ParamValue::Float(val) = new_value
                    && let Some(editor) = self.active_patch_editor()
                {
                    editor.set_parameter_by_name(*module_id, param_name, *val);
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
            UndoAction::Composite(actions) => {
                for sub_action in actions {
                    self.apply_undo_action(sub_action);
                }
            }
        }
    }
}
