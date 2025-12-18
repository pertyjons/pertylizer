//! Tracker grid rendering using egui_extras::TableBuilder.
//!
//! This module handles the visual rendering of the tracker grid with virtual scrolling
//! for performance with large patterns.
//!
//! ## View Adapter Pattern
//!
//! The rendering is separated from data via `CellRenderer`:
//! - GUI never reads `TrackCell` directly
//! - All text formatting goes through `render_cell_text()`
//! - Returns `Cow<str>` to avoid allocations for static strings

use std::borrow::Cow;

#[cfg(feature = "gui-egui")]
use eframe::egui::{self, Color32, RichText, Ui};
#[cfg(feature = "gui-egui")]
use egui_extras::{Column, TableBuilder};

use super::state::{TrackerColumn, TrackerViewState};
use super::tracker::TrackerViewConfig;
use crate::ids::TrackId;
use crate::pattern::TrackCell;
use crate::song::Song;

// ============================================================================
// Cell Renderer - View Adapter for TrackCell
// ============================================================================

/// Column type for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    /// Row index column.
    RowIdx,
    /// Note column (C-4, D#5, ---, ===).
    Note,
    /// Instrument number column (00-FF).
    Instrument,
    /// Volume column (00-40).
    Volume,
    /// Effect type column (0-F).
    EffectType,
    /// Effect value column (00-FF).
    EffectValue,
}

/// Static strings for empty cells (avoids allocations).
mod static_strings {
    pub const EMPTY_NOTE: &str = "---";
    pub const NOTE_OFF: &str = "===";
    pub const EMPTY_INST: &str = "..";
    pub const EMPTY_VOL: &str = "..";
    pub const EMPTY_EFFECT: &str = "...";
}

/// Render cell text based on column type.
///
/// Returns `Cow::Borrowed` for static strings (empty, note-off) to avoid allocations.
/// Returns `Cow::Owned` only when dynamic formatting is needed.
#[must_use]
pub fn render_cell_text(cell: &TrackCell, col: ColumnType) -> Cow<'static, str> {
    match (cell, col) {
        // Empty cell
        (TrackCell::Empty, ColumnType::Note) => Cow::Borrowed(static_strings::EMPTY_NOTE),
        (TrackCell::Empty, ColumnType::Instrument) => Cow::Borrowed(static_strings::EMPTY_INST),
        (TrackCell::Empty, ColumnType::Volume) => Cow::Borrowed(static_strings::EMPTY_VOL),
        (TrackCell::Empty, ColumnType::EffectType | ColumnType::EffectValue) => {
            Cow::Borrowed(static_strings::EMPTY_EFFECT)
        }

        // Note-off
        (TrackCell::NoteOff, ColumnType::Note) => Cow::Borrowed(static_strings::NOTE_OFF),
        (TrackCell::NoteOff, _) => Cow::Borrowed(static_strings::EMPTY_INST),

        // Note cell
        (TrackCell::Note { pitch, .. }, ColumnType::Note) => Cow::Owned(format_pitch(*pitch)),
        (TrackCell::Note { instrument, .. }, ColumnType::Instrument) => {
            Cow::Owned(format!("{instrument:02X}"))
        }
        (TrackCell::Note { velocity, .. }, ColumnType::Volume) => {
            // Convert normalized velocity (0.0-1.0) to tracker volume (00-40)
            let vol = (velocity.as_f32() * 64.0) as u8;
            Cow::Owned(format!("{vol:02X}"))
        }
        (TrackCell::Note { .. }, ColumnType::EffectType | ColumnType::EffectValue) => {
            Cow::Borrowed(static_strings::EMPTY_EFFECT)
        }

        // Effect cell
        (TrackCell::Effect { command, value }, ColumnType::Note) => {
            Cow::Owned(format!("{command:X}{value:02X}"))
        }
        (TrackCell::Effect { .. }, _) => Cow::Borrowed(static_strings::EMPTY_INST),

        // Row index (handled separately, but included for completeness)
        (_, ColumnType::RowIdx) => Cow::Borrowed(""),
    }
}

/// Format a pitch as a tracker string (e.g., "C-4", "C#5").
fn format_pitch(pitch: crate::pitch::Pitch) -> String {
    let note_names = [
        "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
    ];
    let note_idx = (pitch.as_midi() % 12) as usize;
    let octave = pitch.octave();
    format!("{}{}", note_names[note_idx], octave)
}

/// Format a row number.
#[must_use]
pub fn format_row_number(row: u16, hex: bool) -> Cow<'static, str> {
    if hex {
        Cow::Owned(format!("{row:02X}"))
    } else {
        Cow::Owned(format!("{row:3}"))
    }
}

/// Get the color for a cell based on its type and cursor state.
#[cfg(feature = "gui-egui")]
#[must_use]
pub fn cell_color(
    cell: &TrackCell,
    col: ColumnType,
    is_cursor: bool,
    colors: &TrackerColors,
) -> Color32 {
    if is_cursor {
        return Color32::WHITE;
    }

    match (cell, col) {
        (TrackCell::Empty, _) => colors.empty,
        (TrackCell::NoteOff, ColumnType::Note) => colors.note_off,
        (TrackCell::NoteOff, _) => colors.empty,
        (TrackCell::Note { .. }, ColumnType::Note) => colors.note,
        (TrackCell::Note { .. }, ColumnType::Instrument) => colors.instrument,
        (TrackCell::Note { .. }, ColumnType::Volume) => colors.volume,
        (TrackCell::Note { .. }, _) => colors.effect,
        (TrackCell::Effect { .. }, _) => colors.effect,
    }
}

/// Colors for the tracker display.
#[cfg(feature = "gui-egui")]
pub struct TrackerColors {
    pub background: Color32,
    pub row_number: Color32,
    pub row_highlight: Color32,
    pub cursor_row: Color32,
    pub cursor_cell: Color32,
    pub playback_row: Color32,
    pub note: Color32,
    pub note_off: Color32,
    pub instrument: Color32,
    pub volume: Color32,
    pub effect: Color32,
    pub empty: Color32,
    pub separator: Color32,
}

#[cfg(feature = "gui-egui")]
impl Default for TrackerColors {
    fn default() -> Self {
        Self {
            background: Color32::from_rgb(24, 24, 32),
            row_number: Color32::from_rgb(100, 100, 120),
            row_highlight: Color32::from_rgb(32, 32, 48),
            cursor_row: Color32::from_rgb(48, 48, 80),
            cursor_cell: Color32::from_rgb(80, 80, 160),
            playback_row: Color32::from_rgb(60, 100, 60),
            note: Color32::from_rgb(220, 220, 255),
            note_off: Color32::from_rgb(255, 100, 100),
            instrument: Color32::from_rgb(180, 180, 100),
            volume: Color32::from_rgb(100, 200, 100),
            effect: Color32::from_rgb(200, 150, 100),
            empty: Color32::from_rgb(60, 60, 80),
            separator: Color32::from_rgb(60, 60, 80),
        }
    }
}

/// Row height in pixels.
#[cfg(feature = "gui-egui")]
const ROW_HEIGHT: f32 = 18.0;

/// Draw the tracker grid.
///
/// Returns `true` if any interaction occurred.
#[cfg(feature = "gui-egui")]
pub fn draw_tracker_grid(
    ui: &mut Ui,
    state: &mut TrackerViewState,
    song: &Song,
    config: &TrackerViewConfig,
    playback_row: Option<usize>,
) -> bool {
    let colors = TrackerColors::default();

    // Get the active pattern
    let pattern = match state.active_pattern {
        Some(id) => match song.pattern(id) {
            Some(p) => p,
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Pattern not found").color(colors.empty));
                });
                return false;
            }
        },
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("No Pattern Selected")
                        .color(colors.empty)
                        .size(16.0),
                );
            });
            return false;
        }
    };

    // Get number of tracks from pattern (dynamic)
    let num_tracks = pattern.num_tracks() as usize;

    // Convert pattern to tracker rows using pattern's track count
    let mut config_with_tracks = config.clone();
    config_with_tracks.num_channels = num_tracks;
    let rows = super::tracker::to_tracker_rows(pattern, &config_with_tracks);
    let num_rows = rows.len();

    if num_rows == 0 {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("Empty pattern").color(colors.empty));
        });
        return false;
    }

    // Determine which row to scroll to
    let scroll_to = if state.follow_playback {
        // Follow playback position when playing
        playback_row
    } else {
        // Follow cursor when not playing
        Some(state.cursor_row.get())
    };

    let interaction = false;

    // Build the table with dynamic track count
    let mut table = TableBuilder::new(ui)
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(30.0)) // Row number
        .columns(Column::exact(calculate_track_width(config)), num_tracks);

    // Scroll to keep the target row visible
    if let Some(row) = scroll_to {
        table = table.scroll_to_row(row, Some(egui::Align::Center));
    }

    table
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.label(RichText::new("Row").color(colors.row_number).small());
            });
            for track_idx in 0..num_tracks {
                header.col(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("T{}", track_idx + 1))
                                .color(colors.row_number)
                                .small(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let track_id = TrackId::new(track_idx as u16);
                            let is_solo = state.solo_track == Some(track_id);
                            let (btn_fill, btn_text_color) = if is_solo {
                                (Color32::from_rgb(255, 180, 60), Color32::BLACK)
                            } else {
                                (Color32::from_rgb(60, 60, 60), colors.row_number)
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("S").color(btn_text_color).small(),
                                    )
                                    .fill(btn_fill)
                                    .corner_radius(3.0)
                                    .min_size(egui::vec2(16.0, 14.0)),
                                )
                                .clicked()
                            {
                                if is_solo {
                                    state.solo_track = None;
                                } else {
                                    state.solo_track = Some(track_id);
                                }
                            }
                        });
                    });
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, num_rows, |mut row| {
                let row_idx = row.index();
                let tracker_row = &rows[row_idx];
                let is_cursor_row = state.cursor_row.get() == row_idx;
                let is_playback_row = playback_row == Some(row_idx);
                let is_highlight = config.should_highlight(row_idx as u16);

                // Row number column
                row.col(|ui| {
                    let bg = if is_playback_row && is_cursor_row {
                        // Both cursor and playback on same row - blend colors
                        Color32::from_rgb(70, 100, 90)
                    } else if is_playback_row {
                        colors.playback_row
                    } else if is_cursor_row {
                        colors.cursor_row
                    } else if is_highlight {
                        colors.row_highlight
                    } else {
                        colors.background
                    };

                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, bg);

                    ui.label(
                        RichText::new(tracker_row.format_row_number(config.hex_row_numbers))
                            .color(colors.row_number)
                            .monospace(),
                    );
                });

                // Track columns
                for track_idx in 0..num_tracks {
                    row.col(|ui| {
                        let is_cursor_track = state.cursor_track == track_idx;

                        let bg = if is_cursor_row && is_cursor_track {
                            colors.cursor_cell
                        } else if is_playback_row && is_cursor_row {
                            Color32::from_rgb(70, 100, 90)
                        } else if is_playback_row {
                            colors.playback_row
                        } else if is_cursor_row {
                            colors.cursor_row
                        } else if is_highlight {
                            colors.row_highlight
                        } else {
                            colors.background
                        };

                        let rect = ui.available_rect_before_wrap();
                        ui.painter().rect_filled(rect, 0.0, bg);

                        // Get cell data
                        let cell = tracker_row.columns.get(track_idx);

                        // Draw cell contents
                        ui.horizontal(|ui| {
                            draw_cell(
                                ui,
                                cell,
                                config,
                                &colors,
                                is_cursor_row && is_cursor_track,
                                state.cursor_column,
                            );
                        });
                    });
                }
            });
        });

    interaction
}

/// Calculate the width needed for a track column.
#[cfg(feature = "gui-egui")]
fn calculate_track_width(config: &TrackerViewConfig) -> f32 {
    let mut width = 30.0; // Note (C-4)

    if config.show_instrument {
        width += 20.0; // Instrument (00)
    }
    if config.show_volume {
        width += 20.0; // Volume (00)
    }

    width += config.effect_columns as f32 * 30.0; // Effects (000)
    width += 10.0; // Padding

    width
}

/// Draw a single tracker cell.
#[cfg(feature = "gui-egui")]
fn draw_cell(
    ui: &mut Ui,
    cell: Option<&super::tracker::TrackerCell>,
    config: &TrackerViewConfig,
    colors: &TrackerColors,
    is_cursor: bool,
    cursor_column: TrackerColumn,
) {
    let cell = match cell {
        Some(c) => c,
        None => {
            // Empty cell
            draw_empty_cell(ui, config, colors, is_cursor, cursor_column);
            return;
        }
    };

    // Note
    let note_color = if is_cursor && cursor_column == TrackerColumn::Note {
        Color32::WHITE
    } else if cell.note.as_ref().is_some_and(|n| n.is_note_off) {
        colors.note_off
    } else {
        colors.note
    };

    let note_text = cell
        .note
        .as_ref()
        .map(|n| n.as_string())
        .unwrap_or_else(|| "---".to_string());

    ui.label(RichText::new(note_text).color(note_color).monospace());

    // Instrument
    if config.show_instrument {
        let inst_color = if is_cursor && cursor_column == TrackerColumn::Instrument {
            Color32::WHITE
        } else {
            colors.instrument
        };

        let inst_text = cell
            .instrument
            .map(|i| format!("{:02X}", i.0))
            .unwrap_or_else(|| "..".to_string());

        ui.label(RichText::new(inst_text).color(inst_color).monospace());
    }

    // Volume
    if config.show_volume {
        let vol_color = if is_cursor && cursor_column == TrackerColumn::Volume {
            Color32::WHITE
        } else {
            colors.volume
        };

        let vol_text = cell
            .volume
            .map(|v| format!("{:02X}", v))
            .unwrap_or_else(|| "..".to_string());

        ui.label(RichText::new(vol_text).color(vol_color).monospace());
    }

    // Effects
    for i in 0..config.effect_columns as usize {
        let is_effect_cursor = is_cursor
            && (cursor_column == TrackerColumn::EffectType
                || cursor_column == TrackerColumn::EffectValue);
        let effect_color = if is_effect_cursor {
            Color32::WHITE
        } else {
            colors.effect
        };

        let effect_text = cell
            .effects
            .get(i)
            .map(super::tracker::format_effect_command)
            .unwrap_or_else(|| "...".to_string());

        ui.label(RichText::new(effect_text).color(effect_color).monospace());
    }
}

/// Draw an empty cell with proper formatting.
#[cfg(feature = "gui-egui")]
fn draw_empty_cell(
    ui: &mut Ui,
    config: &TrackerViewConfig,
    colors: &TrackerColors,
    is_cursor: bool,
    cursor_column: TrackerColumn,
) {
    // Note
    let note_color = if is_cursor && cursor_column == TrackerColumn::Note {
        Color32::WHITE
    } else {
        colors.empty
    };
    ui.label(RichText::new("---").color(note_color).monospace());

    // Instrument
    if config.show_instrument {
        let inst_color = if is_cursor && cursor_column == TrackerColumn::Instrument {
            Color32::WHITE
        } else {
            colors.empty
        };
        ui.label(RichText::new("..").color(inst_color).monospace());
    }

    // Volume
    if config.show_volume {
        let vol_color = if is_cursor && cursor_column == TrackerColumn::Volume {
            Color32::WHITE
        } else {
            colors.empty
        };
        ui.label(RichText::new("..").color(vol_color).monospace());
    }

    // Effects
    for _ in 0..config.effect_columns as usize {
        let effect_color = if is_cursor
            && (cursor_column == TrackerColumn::EffectType
                || cursor_column == TrackerColumn::EffectValue)
        {
            Color32::WHITE
        } else {
            colors.empty
        };
        ui.label(RichText::new("...").color(effect_color).monospace());
    }
}

/// Draw a "No Pattern" placeholder.
#[cfg(feature = "gui-egui")]
pub fn draw_no_pattern(ui: &mut Ui) {
    let colors = TrackerColors::default();
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            ui.label(
                RichText::new("No Pattern")
                    .color(colors.empty)
                    .size(20.0)
                    .strong(),
            );
            ui.add_space(10.0);
            ui.label(
                RichText::new("Create a pattern or select one from the list")
                    .color(colors.row_number)
                    .size(14.0),
            );
        });
    });
}

// ============================================================================
// New TrackCell-based rendering (uses View Adapter pattern)
// ============================================================================

/// Draw a single TrackCell using the View Adapter.
///
/// This is the new rendering function that uses `render_cell_text()` and `cell_color()`
/// from the View Adapter to avoid direct coupling between GUI and data.
#[cfg(feature = "gui-egui")]
pub fn draw_track_cell(
    ui: &mut Ui,
    cell: &TrackCell,
    config: &TrackerViewConfig,
    colors: &TrackerColors,
    is_cursor: bool,
    cursor_column: TrackerColumn,
) {
    // Note column
    let note_is_cursor = is_cursor && cursor_column == TrackerColumn::Note;
    let note_text = render_cell_text(cell, ColumnType::Note);
    let note_color = cell_color(cell, ColumnType::Note, note_is_cursor, colors);
    ui.label(
        RichText::new(note_text.as_ref())
            .color(note_color)
            .monospace(),
    );

    // Instrument column
    if config.show_instrument {
        let inst_is_cursor = is_cursor && cursor_column == TrackerColumn::Instrument;
        let inst_text = render_cell_text(cell, ColumnType::Instrument);
        let inst_color = cell_color(cell, ColumnType::Instrument, inst_is_cursor, colors);
        ui.label(
            RichText::new(inst_text.as_ref())
                .color(inst_color)
                .monospace(),
        );
    }

    // Volume column
    if config.show_volume {
        let vol_is_cursor = is_cursor && cursor_column == TrackerColumn::Volume;
        let vol_text = render_cell_text(cell, ColumnType::Volume);
        let vol_color = cell_color(cell, ColumnType::Volume, vol_is_cursor, colors);
        ui.label(
            RichText::new(vol_text.as_ref())
                .color(vol_color)
                .monospace(),
        );
    }

    // Effect columns
    for _ in 0..config.effect_columns {
        let effect_is_cursor = is_cursor
            && (cursor_column == TrackerColumn::EffectType
                || cursor_column == TrackerColumn::EffectValue);
        let effect_text = render_cell_text(cell, ColumnType::EffectType);
        let effect_color = cell_color(cell, ColumnType::EffectType, effect_is_cursor, colors);
        ui.label(
            RichText::new(effect_text.as_ref())
                .color(effect_color)
                .monospace(),
        );
    }
}

/// Draw the tracker grid using TrackerGrid directly.
///
/// This is the optimized version that reads from TrackerGrid
/// instead of converting to TrackerRow first.
#[cfg(feature = "gui-egui")]
pub fn draw_tracker_grid_from_pattern(
    ui: &mut Ui,
    state: &mut TrackerViewState,
    pattern: &mut crate::pattern::Pattern,
    config: &TrackerViewConfig,
) -> bool {
    let colors = TrackerColors::default();
    let grid = pattern.grid();
    let num_rows = grid.rows as usize;

    if num_rows == 0 {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("Empty pattern").color(colors.empty));
        });
        return false;
    }

    // Calculate visible rows
    let available_height = ui.available_height();
    let visible_rows = (available_height / ROW_HEIGHT) as usize;

    // Ensure cursor is visible
    state.ensure_cursor_visible(visible_rows);

    let num_tracks = grid.tracks as usize;
    let interaction = false;

    // Build the table with virtual scrolling
    TableBuilder::new(ui)
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(30.0)) // Row number
        .columns(
            Column::exact(calculate_track_width(config)),
            num_tracks.min(state.visible_tracks),
        )
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.label(RichText::new("Row").color(colors.row_number).small());
            });
            for track_idx in 0..num_tracks.min(state.visible_tracks) {
                header.col(|ui| {
                    let track_num = state.first_visible_track + track_idx;
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("T{}", track_num + 1))
                                .color(colors.row_number)
                                .small(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let track_id = TrackId::new(track_num as u16);
                            let is_solo = state.solo_track == Some(track_id);
                            let (btn_fill, btn_text_color) = if is_solo {
                                (Color32::from_rgb(255, 180, 60), Color32::BLACK)
                            } else {
                                (Color32::from_rgb(60, 60, 60), colors.row_number)
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("S").color(btn_text_color).small(),
                                    )
                                    .fill(btn_fill)
                                    .corner_radius(3.0)
                                    .min_size(egui::vec2(16.0, 14.0)),
                                )
                                .clicked()
                            {
                                if is_solo {
                                    state.solo_track = None;
                                } else {
                                    state.solo_track = Some(track_id);
                                }
                            }
                        });
                    });
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, num_rows, |mut row| {
                let row_idx = row.index();
                let is_cursor_row = state.cursor_row.get() == row_idx;
                // Highlight every 4th row (beat marker)
                let is_highlight = config.should_highlight(row_idx as u16);

                // Row number column
                row.col(|ui| {
                    let bg = if is_cursor_row {
                        colors.cursor_row
                    } else if is_highlight {
                        colors.row_highlight
                    } else {
                        colors.background
                    };

                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, bg);

                    let row_text = format_row_number(row_idx as u16, config.hex_row_numbers);
                    ui.label(
                        RichText::new(row_text.as_ref())
                            .color(colors.row_number)
                            .monospace(),
                    );
                });

                // Track columns
                for track_idx in 0..num_tracks.min(state.visible_tracks) {
                    row.col(|ui| {
                        let actual_track = state.first_visible_track + track_idx;
                        let is_cursor_track = state.cursor_track == actual_track;

                        let bg = if is_cursor_row && is_cursor_track {
                            colors.cursor_cell
                        } else if is_cursor_row {
                            colors.cursor_row
                        } else if is_highlight {
                            colors.row_highlight
                        } else {
                            colors.background
                        };

                        let rect = ui.available_rect_before_wrap();
                        ui.painter().rect_filled(rect, 0.0, bg);

                        // Get cell from grid
                        let cell = grid.get(row_idx as u16, actual_track as u8);

                        // Draw cell using View Adapter
                        ui.horizontal(|ui| {
                            draw_track_cell(
                                ui,
                                &cell,
                                config,
                                &colors,
                                is_cursor_row && is_cursor_track,
                                state.cursor_column,
                            );
                        });
                    });
                }
            });
        });

    interaction
}
