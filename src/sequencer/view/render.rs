//! Tracker grid rendering using egui_extras::TableBuilder.
//!
//! This module handles the visual rendering of the tracker grid with virtual scrolling
//! for performance with large patterns.

#[cfg(feature = "gui-egui")]
use eframe::egui::{self, Color32, RichText, Ui};
#[cfg(feature = "gui-egui")]
use egui_extras::{Column, TableBuilder};

use super::state::{TrackerColumn, TrackerViewState};
use super::tracker::TrackerViewConfig;
use crate::sequencer::song::Song;

/// Colors for the tracker display.
#[cfg(feature = "gui-egui")]
pub struct TrackerColors {
    pub background: Color32,
    pub row_number: Color32,
    pub row_highlight: Color32,
    pub cursor_row: Color32,
    pub cursor_cell: Color32,
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

    // Convert pattern to tracker rows
    let rows = super::tracker::to_tracker_rows(pattern, config);
    let num_rows = rows.len();

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

    let interaction = false;

    // Build the table
    TableBuilder::new(ui)
        .striped(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(30.0)) // Row number
        .columns(
            Column::exact(calculate_track_width(config)),
            config.num_channels.min(state.visible_tracks),
        )
        .header(ROW_HEIGHT, |mut header| {
            header.col(|ui| {
                ui.label(RichText::new("Row").color(colors.row_number).small());
            });
            for track_idx in 0..config.num_channels.min(state.visible_tracks) {
                header.col(|ui| {
                    let track_num = state.first_visible_track + track_idx;
                    ui.label(
                        RichText::new(format!("Track {}", track_num + 1))
                            .color(colors.row_number)
                            .small(),
                    );
                });
            }
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, num_rows, |mut row| {
                let row_idx = row.index();
                let tracker_row = &rows[row_idx];
                let is_cursor_row = state.cursor_row.get() == row_idx;
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

                    ui.label(
                        RichText::new(tracker_row.format_row_number(config.hex_row_numbers))
                            .color(colors.row_number)
                            .monospace(),
                    );
                });

                // Track columns
                for track_idx in 0..config.num_channels.min(state.visible_tracks) {
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

                        // Get cell data
                        let cell = tracker_row.columns.get(actual_track);

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
