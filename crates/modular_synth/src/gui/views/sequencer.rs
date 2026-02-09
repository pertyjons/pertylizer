//! Sequencer view - Pattern and song sequencing.
//!
//! This view provides a FastTracker II-style tracker interface for:
//! - Pattern editing with keyboard input
//! - Song arrangement
//! - Transport controls

use std::collections::HashMap;

use eframe::egui::{self, Key, RichText, Ui};

use crate::gui::theme::theme;
use synth_sequencer::effects::EffectCommand;
use synth_sequencer::ids::{RowIndex, TrackIndex};
use synth_sequencer::pitch::Pitch;
use synth_sequencer::song::Song;
use synth_sequencer::tracker_pattern::{Cell, TrackerPattern};
use synth_sequencer::view::{
    TrackerColumn, TrackerViewConfig, TrackerViewState, draw_tracker_grid,
};
use synth_sequencer::{PatternId, PatternTick, Tick};

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
    /// Muted tracks changed — contains the new muted_tracks vec.
    pub muted_tracks_changed: Option<Vec<bool>>,
    /// Seek to this tick position (for pattern navigation during playback).
    pub seek_to: Option<Tick>,
}

/// Show the sequencer view.
pub fn show(
    ctx: &egui::Context,
    tracker_state: &mut TrackerViewState,
    song: Option<&Song>,
    playback_tick: Option<Tick>,
    playback_ticks_per_row: u32,
) -> SequencerResult {
    let mut result = SequencerResult::default();

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical(|ui| {
            // Snapshot muted state before any UI interaction (toolbar + grid)
            let muted_before = tracker_state.muted_tracks.clone();

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
                                if song.tracker_pattern(pattern_id).is_some() {
                                    // Use dynamic ticks_per_row from the engine (respects
                                    // SetSpeed effects) instead of the static pattern value.
                                    let pattern_tick = offset.0 as u32;
                                    if playback_ticks_per_row > 0 {
                                        Some((pattern_tick / playback_ticks_per_row) as usize)
                                    } else {
                                        Some(0)
                                    }
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

                    // Draw the tracker grid
                    let config = TrackerViewConfig::fasttracker();
                    draw_tracker_grid(ui, tracker_state, song, &config, playback_row);
                }
                None => {
                    draw_no_song_placeholder(ui);
                }
            }

            // Detect mute changes from toolbar or grid
            if tracker_state.muted_tracks != muted_before {
                result.muted_tracks_changed = Some(tracker_state.muted_tracks.clone());
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

        ui.add_space(10.0);
        if ui
            .button("Debug")
            .on_hover_text("Print pattern debug info to console")
            .clicked()
            && let Some(song) = song
        {
            print_debug_info(song, state);
        }

        ui.add_space(10.0);
        if ui
            .button("Unmute All")
            .on_hover_text("Unmute all tracks")
            .clicked()
        {
            state.unmute_all();
        }

        // Song name
        if let Some(song) = song
            && !song.name.is_empty()
        {
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            ui.label(RichText::new(&song.name).color(colors.text_dim));
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

/// Print debug info about the current song and pattern to the console.
#[allow(clippy::too_many_lines)]
fn print_debug_info(song: &Song, state: &TrackerViewState) {
    println!("=== Debug: Pattern Info ===");
    println!("Song: {}", song.name);
    println!("Author: {}", song.author);
    println!("Default BPM: {}", song.default_tempo);
    println!("Default speed: {}", song.default_tracker_speed);
    println!("Frequency mode: {:?}", song.tracker_frequency_mode);
    let track_count = song.track_count();
    println!("Tracks: {track_count}");

    // Track mute status
    print!("  Track status:\n    ");
    for i in 0..track_count {
        if i > 0 {
            print!(" | ");
        }
        let status = if state.is_muted(i) { "MUTED" } else { "active" };
        print!("T{}: {status:>6}", i + 1);
    }
    println!();

    println!(
        "Patterns: {} tracker + {} piano",
        song.tracker_pattern_count(),
        song.pattern_count()
    );
    println!("Instruments: {}", song.all_instrument_defaults().len());

    // Instrument defaults
    let defaults = song.all_instrument_defaults();
    if !defaults.is_empty() {
        println!("\nInstrument Defaults:");
        let mut ids: Vec<_> = defaults.keys().collect();
        ids.sort_by_key(|id| id.0);
        for id in ids {
            if let Some(d) = defaults.get(id) {
                println!(
                    "  Inst {:02X}: vol={:.3} pan={:+.3}",
                    id.0,
                    d.volume.as_f32(),
                    d.panning.as_f32(),
                );
            }
        }
    }

    // Active pattern info
    if let Some(pattern_id) = state.active_pattern {
        println!("\nActive pattern: {:?}", pattern_id);
        if let Some(tp) = song.tracker_pattern(pattern_id) {
            println!("  Name: {:?}", tp.name());
            println!("  Rows: {}", tp.num_rows().as_usize());
            println!("  Tracks/channels: {}", tp.num_tracks().as_usize());
            println!("  Ticks per row: {}", tp.ticks_per_row().as_u32());

            // Count non-empty rows, notes, effects
            let mut note_count = 0usize;
            let mut noteoff_count = 0usize;
            let mut effect_count = 0usize;
            let mut non_empty_count = 0usize;
            for (_, _, row) in tp.non_empty_rows() {
                non_empty_count += 1;
                if row.has_note() {
                    note_count += 1;
                }
                if row.has_note_off() {
                    noteoff_count += 1;
                }
                if row.has_effects() {
                    effect_count += row.effects.len();
                }
            }
            println!("\n  Non-empty rows: {non_empty_count}");
            println!("  Notes: {note_count}");
            println!("  Note-offs: {noteoff_count}");
            println!("  Effect commands: {effect_count}");

            // Effect usage summary
            print_effect_summary(tp);

            // Full pattern grid
            println!();
            print_pattern_grid(tp);
        }
    } else {
        println!("\nNo active pattern");
    }

    // Arrangement / pattern order
    let arrangement = song.arrangement();
    println!("\nArrangement: {} placements", arrangement.len());
    for (i, placement) in arrangement.iter().enumerate() {
        println!(
            "  [{:02}] Pattern {:?} on track {:?} at tick {}",
            i, placement.pattern_id, placement.track_id, placement.start.0
        );
    }
    println!("=== End Debug ===");
}

// =============================================================================
// Debug formatting helpers (matching analyze_tracker example output)
// =============================================================================

/// Print a tracker pattern as a full grid.
fn print_pattern_grid(pattern: &TrackerPattern) {
    let num_tracks = pattern.num_tracks().as_u8() as usize;
    let num_rows = pattern.num_rows().as_u16() as usize;

    println!(
        "=== Pattern {:02X}: \"{}\" ({} rows, {} tracks, {} ticks/row) ===",
        pattern.id().0,
        pattern.name(),
        num_rows,
        num_tracks,
        pattern.ticks_per_row().as_u32(),
    );

    // Header
    print!("    |");
    for ch in 0..num_tracks {
        print!(" T{:<2}            |", ch + 1);
    }
    println!();

    // Separator
    print!("----|");
    for _ in 0..num_tracks {
        print!("-----------------|");
    }
    println!();

    // Rows
    for row_idx in 0..num_rows {
        #[allow(clippy::cast_possible_truncation)]
        let ri = RowIndex::new(row_idx as u16);
        print!("{row_idx:02X}  |");
        for ch in 0..num_tracks {
            #[allow(clippy::cast_possible_truncation)]
            let ti = TrackIndex::new(ch as u8);
            let row = pattern.get(ti, ri);
            print!(" {} |", format_cell(&row.cell, &row.effects));
        }
        println!();
    }
}

/// Print a summary of effect usage in a pattern.
fn print_effect_summary(pattern: &TrackerPattern) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, _, row) in pattern.non_empty_rows() {
        for effect in &row.effects {
            let name = effect_variant_name(effect);
            *counts.entry(name).or_insert(0) += 1;
        }
    }

    if counts.is_empty() {
        return;
    }

    // Sort by count descending
    let mut sorted: Vec<_> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    println!("\nEffect usage in pattern:");
    for (name, count) in &sorted {
        println!("  {name:<20} x{count}");
    }
}

/// Get the variant name of an `EffectCommand` as a human-readable string.
fn effect_variant_name(effect: &EffectCommand) -> String {
    match effect {
        EffectCommand::Arpeggio { .. } => "Arpeggio",
        EffectCommand::PortamentoUp(_) => "PortamentoUp",
        EffectCommand::PortamentoDown(_) => "PortamentoDown",
        EffectCommand::TonePortamento { .. } => "TonePortamento",
        EffectCommand::Vibrato { .. } => "Vibrato",
        EffectCommand::VibratoWaveform(_) => "VibratoWaveform",
        EffectCommand::Glissando(_) => "Glissando",
        EffectCommand::FineTune(_) => "FineTune",
        EffectCommand::SetVolume(_) => "SetVolume",
        EffectCommand::VolumeSlide { .. } => "VolumeSlide",
        EffectCommand::FineVolumeSlide { .. } => "FineVolumeSlide",
        EffectCommand::Tremolo { .. } => "Tremolo",
        EffectCommand::TremoloWaveform(_) => "TremoloWaveform",
        EffectCommand::SetPanning(_) => "SetPanning",
        EffectCommand::PanningSlide { .. } => "PanningSlide",
        EffectCommand::SampleOffset(_) => "SampleOffset",
        EffectCommand::Retrigger { .. } => "Retrigger",
        EffectCommand::NoteCut(_) => "NoteCut",
        EffectCommand::NoteDelay(_) => "NoteDelay",
        EffectCommand::NoteFadeOut(_) => "NoteFadeOut",
        EffectCommand::Reverse => "Reverse",
        EffectCommand::SetTempo(_) => "SetTempo",
        EffectCommand::SetSpeed(_) => "SetSpeed",
        EffectCommand::PatternDelay(_) => "PatternDelay",
        EffectCommand::PatternLoop { .. } => "PatternLoop",
        EffectCommand::PatternJump(_) => "PatternJump",
        EffectCommand::PatternBreak(_) => "PatternBreak",
    }
    .to_string()
}

/// Format a cell + effects as a fixed-width string.
///
/// Format: "NOT IN VV EFX1" (up to 16 chars)
/// - NOT: note (3 chars: "C-4", "===", "---")
/// - IN: instrument (2 chars: hex or "..")
/// - VV: volume (2 chars: hex or "..")
/// - EFX: effects (variable)
fn format_cell(cell: &Cell, effects: &[EffectCommand]) -> String {
    let (note, inst, vel) = match cell {
        Cell::Empty => ("---".to_string(), "..".to_string(), None),
        Cell::NoteOff => ("===".to_string(), "..".to_string(), None),
        Cell::Note {
            pitch,
            instrument,
            velocity,
        } => (
            format_pitch(*pitch),
            instrument.map_or("..".to_string(), |i| format!("{:02X}", i.0)),
            *velocity,
        ),
    };

    // Separate volume (SetVolume) from other effects
    let mut vol_str = String::new();
    let mut other_effects = Vec::new();

    for effect in effects {
        if let EffectCommand::SetVolume(v) = effect {
            if vol_str.is_empty() {
                vol_str = format!("{v:02X}");
            } else {
                other_effects.push(format_effect(effect));
            }
        } else {
            other_effects.push(format_effect(effect));
        }
    }

    // If no SetVolume effect, check cell velocity
    if vol_str.is_empty() {
        if let Some(v) = vel {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let vol_byte = (v.as_f32() * 64.0).min(64.0) as u8;
            vol_str = format!("{vol_byte:02X}");
        } else {
            vol_str = "..".to_string();
        }
    }

    let efx_str = if other_effects.is_empty() {
        "....".to_string()
    } else {
        other_effects.join(" ")
    };

    format!("{note} {inst} {vol_str} {efx_str:<4}")
}

/// Format a Pitch as "C-4", "C#4", etc.
fn format_pitch(pitch: Pitch) -> String {
    let midi = pitch.as_midi();
    let octave = midi / 12;
    let semitone = midi % 12;
    let name = match semitone {
        0 => "C-",
        1 => "C#",
        2 => "D-",
        3 => "D#",
        4 => "E-",
        5 => "F-",
        6 => "F#",
        7 => "G-",
        8 => "G#",
        9 => "A-",
        10 => "A#",
        11 => "B-",
        _ => "??",
    };
    format!("{name}{octave}")
}

/// Format an `EffectCommand` as a short tracker-style code.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn format_effect(effect: &EffectCommand) -> String {
    match effect {
        EffectCommand::Arpeggio { x, y } => format!("0{x:X}{y:X}"),
        EffectCommand::PortamentoUp(s) => format!("1{s:02X}"),
        EffectCommand::PortamentoDown(s) => format!("2{s:02X}"),
        EffectCommand::TonePortamento { speed, .. } => format!("3{speed:02X}"),
        EffectCommand::Vibrato { speed, depth } => format!("4{speed:X}{depth:X}"),
        EffectCommand::VibratoWaveform(_) => "E4x".to_string(),
        EffectCommand::Glissando(on) => format!("E3{}", u8::from(*on)),
        EffectCommand::FineTune(c) => format!("E5{:02X}", *c as u8),
        EffectCommand::SetVolume(v) => format!("C{v:02X}"),
        EffectCommand::VolumeSlide { up, down } => {
            if *up > 0 {
                format!("A{up:X}0")
            } else {
                format!("A0{down:X}")
            }
        }
        EffectCommand::FineVolumeSlide { up, down } => {
            if *up > 0 {
                format!("EA{up:X}")
            } else {
                format!("EB{down:X}")
            }
        }
        EffectCommand::Tremolo { speed, depth } => format!("7{speed:X}{depth:X}"),
        EffectCommand::TremoloWaveform(_) => "E7x".to_string(),
        EffectCommand::SetPanning(p) => format!("8{p:02X}"),
        EffectCommand::PanningSlide { left, right } => {
            if *left > 0 {
                format!("P{left:X}0")
            } else {
                format!("P0{right:X}")
            }
        }
        EffectCommand::SampleOffset(o) => format!("9{:02X}", (*o >> 8) as u8),
        EffectCommand::Retrigger { interval, .. } => format!("R{interval:02X}"),
        EffectCommand::NoteCut(t) => format!("SC{t:X}"),
        EffectCommand::NoteDelay(t) => format!("SD{t:X}"),
        EffectCommand::NoteFadeOut(t) => format!("KF{t:X}"),
        EffectCommand::Reverse => "REV".to_string(),
        EffectCommand::SetTempo(t) => format!("F{t:02X}"),
        EffectCommand::SetSpeed(s) => format!("F{s:02X}"),
        EffectCommand::PatternDelay(r) => format!("EE{r:X}"),
        EffectCommand::PatternLoop { count } => format!("E6{count:X}"),
        EffectCommand::PatternJump(p) => format!("B{p:02X}"),
        EffectCommand::PatternBreak(r) => format!("D{r:02X}"),
    }
}
