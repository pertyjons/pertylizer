//! Sequencer view - Pattern and song sequencing.
//!
//! This view provides a FastTracker II-style tracker interface for:
//! - Pattern editing with keyboard input
//! - Song arrangement
//! - Transport controls

use eframe::egui::{self, Key, RichText, Ui};

use crate::gui::theme::theme;
use synth_sequencer::song::Song;
use synth_sequencer::view::{
    TrackerColumn, TrackerViewConfig, TrackerViewState, draw_tracker_grid,
};
use synth_sequencer::{PatternId, PatternTick, Tick, TrackId};

/// Transport action requested from the sequencer view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportAction {
    /// Play song from current pattern position.
    Play,
    /// Stop playback.
    Stop,
    /// Rewind to beginning.
    Rewind,
    /// Play only the active pattern in a loop.
    PlayPattern,
}

/// Result of sequencer view interaction.
#[derive(Debug, Default)]
pub struct SequencerResult {
    /// Note to play (MIDI note number).
    pub play_note: Option<u8>,
    /// Note to stop.
    pub stop_note: Option<u8>,
    /// Transport action requested.
    pub transport: Option<TransportAction>,
    /// Solo track changed (Some(Some(track)) = solo track, Some(None) = no solo).
    pub solo_track_changed: Option<Option<TrackId>>,
    /// Seek to this tick position (for pattern navigation during playback).
    pub seek_to: Option<Tick>,
}

/// Show the sequencer view.
pub fn show(
    ctx: &egui::Context,
    tracker_state: &mut TrackerViewState,
    song: Option<&Song>,
    playback_tick: Option<Tick>,
) -> SequencerResult {
    let mut result = SequencerResult::default();

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical(|ui| {
            // Toolbar
            result.transport = draw_toolbar(ui, tracker_state, song);

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            // Main content area
            match song {
                Some(song) => {
                    // Handle pattern navigation (if buttons were clicked)
                    if tracker_state.navigate_pattern_prev || tracker_state.navigate_pattern_next {
                        // Collect and sort pattern IDs from both regular and tracker patterns
                        let mut pattern_ids: Vec<_> = song.patterns().map(|p| p.id).collect();
                        pattern_ids.extend(song.tracker_patterns().map(|p| p.id()));
                        pattern_ids.sort_by_key(|id| id.0);
                        pattern_ids.dedup(); // Remove duplicates if any

                        if let Some(new_pattern) = tracker_state.navigate_to_pattern(&pattern_ids) {
                            // If playing, seek to the new pattern's start position
                            if playback_tick.is_some()
                                && let Some(start_tick) = find_pattern_start_tick(song, new_pattern)
                            {
                                result.seek_to = Some(start_tick);
                            }
                        }
                    }

                    // Handle keyboard input BEFORE drawing (to update state first)
                    handle_tracker_input(ui, tracker_state, song, &mut result);

                    // Calculate playback row and auto-switch pattern during playback
                    let playback_row = playback_tick.and_then(|tick| {
                        // Find which pattern is playing at this tick
                        if let Some((pattern_id, offset)) = song.pattern_at_tick(tick) {
                            // Auto-switch to the playing pattern if follow is enabled
                            if tracker_state.follow_playback
                                && tracker_state.active_pattern != Some(pattern_id)
                            {
                                tracker_state.active_pattern = Some(pattern_id);
                                // Reset scroll to top when switching patterns
                                tracker_state.scroll_offset = synth_sequencer::view::RowIndex(0);
                            }

                            // Calculate row within the pattern
                            if tracker_state.active_pattern == Some(pattern_id) {
                                // Try TrackerPattern first (native tracker format)
                                if let Some(tracker_pattern) = song.tracker_pattern(pattern_id) {
                                    let pattern_tick = PatternTick(offset.0 as u32);
                                    Some(tracker_pattern.tick_to_row(pattern_tick).as_usize())
                                } else if let Some(pattern) = song.pattern(pattern_id) {
                                    // Fall back to regular Pattern
                                    let pattern_tick = PatternTick(offset.0 as u32);
                                    Some(pattern.row_resolution.tick_to_row(pattern_tick) as usize)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            // No pattern at this position - maybe before first pattern or gap
                            None
                        }
                    });

                    // Draw the tracker grid (track solo changes)
                    let solo_before = tracker_state.solo_track;
                    let config = TrackerViewConfig::fasttracker();
                    draw_tracker_grid(ui, tracker_state, song, &config, playback_row);
                    if tracker_state.solo_track != solo_before {
                        result.solo_track_changed = Some(tracker_state.solo_track);
                    }
                }
                None => {
                    draw_no_song_placeholder(ui);
                }
            }
        });
    });

    result
}

/// Draw the toolbar with controls.
/// Returns the transport action if a button was clicked.
fn draw_toolbar(
    ui: &mut Ui,
    state: &mut TrackerViewState,
    song: Option<&Song>,
) -> Option<TransportAction> {
    let colors = theme().colors;
    let mut transport_action = None;

    ui.horizontal(|ui| {
        // Transport controls
        ui.add_space(10.0);

        if ui
            .button(RichText::new("⏮").size(18.0))
            .on_hover_text("Rewind (Home)")
            .clicked()
        {
            state.goto_start();
            transport_action = Some(TransportAction::Rewind);
        }

        if ui
            .button(RichText::new("▶").size(18.0))
            .on_hover_text("Play (Space)")
            .clicked()
        {
            transport_action = Some(TransportAction::Play);
        }

        if ui
            .button(RichText::new("⏹").size(18.0))
            .on_hover_text("Stop (Space)")
            .clicked()
        {
            transport_action = Some(TransportAction::Stop);
        }

        if ui
            .button(RichText::new("⏺").size(18.0))
            .on_hover_text("Record")
            .clicked()
        {
            // TODO: Implement record
        }

        ui.add_space(10.0);

        // Play Pattern button (loops current pattern)
        if ui
            .button(RichText::new("🔁").size(18.0))
            .on_hover_text("Play Pattern (loop current pattern)")
            .clicked()
        {
            transport_action = Some(TransportAction::PlayPattern);
        }

        ui.add_space(20.0);
        ui.separator();
        ui.add_space(20.0);

        // Octave selector
        ui.label(RichText::new("Octave:").color(colors.text_dim));
        if ui
            .button("-")
            .on_hover_text("Decrease octave (F1)")
            .clicked()
        {
            state.octave_down();
        }
        ui.label(
            RichText::new(format!("{}", state.octave))
                .color(colors.text_primary)
                .monospace()
                .strong(),
        );
        if ui
            .button("+")
            .on_hover_text("Increase octave (F2)")
            .clicked()
        {
            state.octave_up();
        }

        ui.add_space(20.0);

        // Step size selector
        ui.label(RichText::new("Step:").color(colors.text_dim));
        if ui.button("-").on_hover_text("Decrease step").clicked() && state.step_size > 0 {
            state.step_size -= 1;
        }
        ui.label(
            RichText::new(format!("{}", state.step_size))
                .color(colors.text_primary)
                .monospace()
                .strong(),
        );
        if ui.button("+").on_hover_text("Increase step").clicked() && state.step_size < 16 {
            state.step_size += 1;
        }

        ui.add_space(20.0);

        // Follow playback toggle
        let follow_text = if state.follow_playback {
            "Follow: ON"
        } else {
            "Follow: OFF"
        };
        if ui
            .selectable_label(state.follow_playback, follow_text)
            .on_hover_text("Follow playback position")
            .clicked()
        {
            state.follow_playback = !state.follow_playback;
        }

        ui.add_space(20.0);
        ui.separator();
        ui.add_space(20.0);

        // Pattern selector with navigation and count
        ui.label(RichText::new("Pattern:").color(colors.text_dim));
        if ui.button("<").on_hover_text("Previous pattern").clicked() {
            state.navigate_pattern_prev = true;
        }

        // Show current/total pattern count (including tracker patterns)
        let pattern_count = song
            .map(|s| s.pattern_count() + s.tracker_pattern_count())
            .unwrap_or(0);
        let current_index = song
            .and_then(|s| {
                state.active_pattern.and_then(|active_id| {
                    let mut ids: Vec<_> = s.patterns().map(|p| p.id).collect();
                    ids.extend(s.tracker_patterns().map(|p| p.id()));
                    ids.sort_by_key(|id| id.0);
                    ids.dedup();
                    ids.iter().position(|&id| id == active_id)
                })
            })
            .map(|i| i + 1) // 1-based index for display
            .unwrap_or(0);

        let pattern_text = if pattern_count > 0 {
            format!("{}/{}", current_index, pattern_count)
        } else {
            "--/--".to_string()
        };
        ui.label(
            RichText::new(pattern_text)
                .color(colors.text_primary)
                .monospace()
                .strong(),
        );
        if ui.button(">").on_hover_text("Next pattern").clicked() {
            state.navigate_pattern_next = true;
        }

        // Cursor position display
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!(
                    "Row {:02X} Track {} Col {:?}",
                    state.cursor_row.get(),
                    state.cursor_track + 1,
                    state.cursor_column
                ))
                .color(colors.text_dim)
                .monospace()
                .small(),
            );
        });
    });

    transport_action
}

/// Handle keyboard input for tracker navigation and editing.
fn handle_tracker_input(
    ui: &mut Ui,
    state: &mut TrackerViewState,
    song: &Song,
    result: &mut SequencerResult,
) {
    // Get pattern row count for bounds checking (check TrackerPattern first)
    let max_rows = state
        .active_pattern
        .and_then(|id| {
            song.tracker_pattern(id)
                .map(|p| p.num_rows().as_usize())
                .or_else(|| song.pattern(id).map(|p| p.row_resolution.rows as usize))
        })
        .unwrap_or(64);

    // Get track count from active pattern (check TrackerPattern first)
    let num_tracks = state
        .active_pattern
        .and_then(|id| {
            song.tracker_pattern(id)
                .map(|p| p.num_tracks().as_usize())
                .or_else(|| song.pattern(id).map(|p| p.num_tracks() as usize))
        })
        .unwrap_or(4);

    // Navigation keys
    ui.input(|i| {
        // Arrow keys for navigation
        if i.key_pressed(Key::ArrowDown) {
            state.cursor_down(max_rows);
        }
        if i.key_pressed(Key::ArrowUp) {
            state.cursor_up();
        }
        if i.key_pressed(Key::ArrowRight) {
            state.cursor_right(num_tracks);
        }
        if i.key_pressed(Key::ArrowLeft) {
            state.cursor_left();
        }

        // Page up/down
        if i.key_pressed(Key::PageDown) {
            for _ in 0..16 {
                state.cursor_down(max_rows);
            }
        }
        if i.key_pressed(Key::PageUp) {
            for _ in 0..16 {
                state.cursor_up();
            }
        }

        // Home/End
        if i.key_pressed(Key::Home) {
            state.goto_start();
        }
        if i.key_pressed(Key::End) {
            state.goto_end(max_rows);
        }

        // Tab to next track
        if i.key_pressed(Key::Tab) {
            if i.modifiers.shift {
                if state.cursor_track > 0 {
                    state.cursor_track -= 1;
                }
            } else if state.cursor_track + 1 < num_tracks {
                state.cursor_track += 1;
            }
        }

        // Octave change with F1/F2
        if i.key_pressed(Key::F1) {
            state.octave_down();
        }
        if i.key_pressed(Key::F2) {
            state.octave_up();
        }

        // Note entry (when in Note column)
        if state.cursor_column == TrackerColumn::Note {
            // Piano keyboard layout: Z=C, S=C#, X=D, D=D#, etc.
            let note_keys = [
                (Key::Z, 0),  // C
                (Key::S, 1),  // C#
                (Key::X, 2),  // D
                (Key::D, 3),  // D#
                (Key::C, 4),  // E
                (Key::V, 5),  // F
                (Key::G, 6),  // F#
                (Key::B, 7),  // G
                (Key::H, 8),  // G#
                (Key::N, 9),  // A
                (Key::J, 10), // A#
                (Key::M, 11), // B
                // Upper octave: Q=C, 2=C#, W=D, etc.
                (Key::Q, 12), // C+1
                (Key::Num2, 13),
                (Key::W, 14),
                (Key::Num3, 15),
                (Key::E, 16),
                (Key::R, 17),
                (Key::Num5, 18),
                (Key::T, 19),
                (Key::Num6, 20),
                (Key::Y, 21),
                (Key::Num7, 22),
                (Key::U, 23), // B+1
                (Key::I, 24), // C+2
            ];

            for (key, semitone) in note_keys {
                if i.key_pressed(key) {
                    let midi_note = (state.octave * 12) + semitone;
                    if midi_note <= 127 {
                        result.play_note = Some(midi_note);
                        // Advance cursor
                        state.cursor_down(max_rows);
                    }
                }
            }

            // Delete/Backspace to clear cell
            if i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace) {
                // TODO: Clear current cell
                state.cursor_down(max_rows);
            }
        }
    });
}

/// Draw placeholder when no song is loaded.
fn draw_no_song_placeholder(ui: &mut Ui) {
    let colors = theme().colors;

    ui.vertical_centered(|ui| {
        ui.add_space(100.0);
        ui.heading(RichText::new("No Song Loaded").color(colors.text_dim));
        ui.add_space(20.0);
        ui.label(
            RichText::new("Create a new song or load an existing one to start sequencing.")
                .color(colors.text_dim),
        );

        ui.add_space(40.0);

        // Placeholder transport (for visual consistency)
        ui.horizontal(|ui| {
            ui.add_space(ui.available_width() / 2.0 - 80.0);
            ui.add_enabled(false, egui::Button::new(RichText::new("⏮").size(24.0)));
            ui.add_enabled(false, egui::Button::new(RichText::new("▶").size(24.0)));
            ui.add_enabled(false, egui::Button::new(RichText::new("⏹").size(24.0)));
            ui.add_enabled(false, egui::Button::new(RichText::new("⏺").size(24.0)));
        });

        ui.add_space(40.0);

        // Keyboard shortcut hints
        ui.label(RichText::new("Keyboard Shortcuts:").color(colors.text_primary));
        ui.add_space(10.0);

        let shortcuts = [
            ("Arrow Keys", "Navigate"),
            ("Z, S, X, D, C...", "Play notes (piano layout)"),
            ("F1 / F2", "Octave down / up"),
            ("Tab / Shift+Tab", "Next / Previous track"),
            ("Home / End", "Go to start / end"),
            ("Page Up / Down", "Move 16 rows"),
        ];

        for (key, desc) in shortcuts {
            ui.horizontal(|ui| {
                ui.label(RichText::new(key).color(colors.accent_primary).monospace());
                ui.label(RichText::new(" - ").color(colors.text_dim));
                ui.label(RichText::new(desc).color(colors.text_dim));
            });
        }
    });
}

/// Find the start tick for a pattern's first occurrence in the arrangement.
fn find_pattern_start_tick(song: &Song, pattern_id: PatternId) -> Option<Tick> {
    song.arrangement()
        .iter()
        .find(|p| p.pattern_id == pattern_id)
        .map(|p| p.start)
}
