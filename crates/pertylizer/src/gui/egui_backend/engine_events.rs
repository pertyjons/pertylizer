//! engine events responsibilities for the egui application.

use super::*;

impl SynthApp {
    /// Drain MCP→GUI shared state once per frame: project I/O refresh/status
    /// and one-shot patch mirror payloads. Each section is revision-gated so
    /// an idle frame touches no mutex.
    #[cfg(feature = "mcp")]
    pub(super) fn drain_mcp_state(&mut self) {
        // Single revision-gated drain for everything MCP project I/O
        // pushes back to the GUI: the refresh queue, source path, and
        // status line. `project_revision` is the lock-free fast path —
        // when nothing has happened we don't touch any mutex.
        if let Some(shared) = self.mcp_shared.as_ref().map(std::sync::Arc::clone) {
            let current_rev = shared
                .project_revision
                .load(std::sync::atomic::Ordering::Acquire);
            if self.mcp_sync.project.observe(current_rev) {
                use crate::mcp_shared::ProjectRefresh;

                // Load / new stash a refresh; save leaves it empty.
                let refresh = shared
                    .pending_project_refresh
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                if let Some(refresh) = refresh {
                    match refresh {
                        ProjectRefresh::Loaded(project) => {
                            self.current_project_path = shared
                                .last_loaded_project_path
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .clone();
                            self.refresh_ui_from_project(&project);
                            self.mark_saved();
                        }
                        ProjectRefresh::Reset => {
                            self.refresh_ui_after_reset();
                            self.current_project_path = None;
                            self.current_patch_name = "Init".to_string();
                            self.current_patch_path = None;
                            self.mark_saved();
                        }
                    }
                }

                // Surface the most recent I/O outcome (success or error)
                // in the status line.
                let status = shared
                    .last_project_io_status
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if let Some(status) = status {
                    let msg = match status {
                        Ok(m) => m,
                        Err(e) => format!("Error: {e}"),
                    };
                    self.dialog_state.set_status(msg);
                }
            }
        }

        // Revision-gated drain of MCP→GUI one-shot mirror payloads
        // (`pending_patch`). Same shape as the
        // `project_revision` drain just above.
        if let Some(shared) = self.mcp_shared.as_ref().map(std::sync::Arc::clone) {
            let current_rev = shared
                .gui_revision
                .load(std::sync::atomic::Ordering::Acquire);
            if self.mcp_sync.gui.observe(current_rev) {
                let pending_patch = shared
                    .pending_patch
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                if let Some((patch, name)) = pending_patch {
                    self.current_patch_name = name;
                    self.current_patch_path = None;
                    self.load_patch_data(&patch);
                }
            }
        }
    }

    /// Drain engine→GUI events: mirror note on/off to the on-screen keyboard,
    /// apply learned key ranges, update the recording preview, and write flushed
    /// recorded notes into their pattern.
    /// Rebuild the engine's Mod Grid runtime when the pooled data changed and
    /// ship it via `SetModGrid`. No-op (one `u64` compare) when unchanged.
    pub(super) fn sync_mod_grid(&mut self) {
        let runtime = {
            let song = self.song.read();
            let generation = song.mod_grid_generation();
            if !self.mod_grid_version.observe(generation) {
                return;
            }
            crate::mod_grid_build::build_mod_grid_runtime(&song)
        };
        self.handle.send(synth_engine::EngineCommand::SetModGrid {
            runtime: Box::new(runtime),
        });
    }

    pub(super) fn poll_engine_events(&mut self) {
        while let Some(event) = self.handle.poll_event() {
            match event {
                EngineEvent::NoteTriggered { note, velocity, .. } => {
                    self.keyboard.set_note_on(note, velocity);
                }
                EngineEvent::NoteReleased { note, .. } => {
                    self.keyboard.set_note_off(note);
                    self.pressed_keys.remove(&note.as_u8());
                }
                EngineEvent::AllNotesReleased => {
                    self.keyboard.clear_pressed();
                    self.pressed_keys.clear();
                }
                EngineEvent::KeyRangeLearned {
                    instrument_id,
                    key_range,
                    learn_state,
                } => {
                    // Update the instrument's UI state with the learned key range
                    if let Some(inst) = self.instruments.iter_mut().find(|i| i.id == instrument_id)
                    {
                        inst.key_range = key_range;
                        inst.learn_state = learn_state;
                    }
                }
                EngineEvent::RecordingPreview {
                    completed,
                    held,
                    pattern_length,
                } => {
                    self.sequencer_view_state.recording_preview_completed = completed;
                    self.sequencer_view_state.recording_preview_held = held;
                    self.sequencer_view_state.recording_preview_pattern_length = pattern_length;
                }
                EngineEvent::RecordedNotesFlushed {
                    pattern_id,
                    notes,
                    overdub,
                } => {
                    // Write recorded notes into the pattern (on UI thread, safe to lock).
                    // Known limitation: if the song lock is poisoned, recorded notes are
                    // silently dropped. Buffering for retry adds complexity for a scenario
                    // that is extremely unlikely since only the UI thread writes.
                    // Committing a take is one undo step, whether it
                    // overdubbed or replaced: a whole-pattern swap, because a
                    // non-overdub take clears every note that was there.
                    let before = self.song.read().pattern(pattern_id).cloned();
                    {
                        let mut song = self.song.write();
                        if let Some(pattern) = song.pattern_mut(pattern_id) {
                            if !overdub {
                                pattern.clear_notes();
                            }
                            for note in &notes {
                                let nid = pattern.add_note(note.start, note.pitch, note.velocity);
                                if let Some(n) = pattern.note_mut(nid) {
                                    n.duration = Some(note.duration);
                                }
                            }
                        } else {
                            eprintln!(
                                "RecordedNotesFlushed: pattern {pattern_id:?} not found, {} notes dropped",
                                notes.len()
                            );
                        }
                    }
                    if let Some(before) = before
                        && let Some(after) = self.song.read().pattern(pattern_id).cloned()
                    {
                        self.undo_manager
                            .push(crate::undo::UndoAction::SwapPattern {
                                pattern_id,
                                old: Box::new(before),
                                new: Box::new(after),
                            });
                    }
                    self.mark_dirty();

                    // Clear preview — notes are now committed
                    self.sequencer_view_state
                        .recording_preview_completed
                        .clear();
                    self.sequencer_view_state.recording_preview_held.clear();
                }
                // Other events (meters, etc.) are handled elsewhere
                _ => {}
            }
        }
    }
}
