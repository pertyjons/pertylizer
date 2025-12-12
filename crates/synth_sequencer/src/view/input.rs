//! Tracker input handling.
//!
//! This module provides keyboard input handling for the tracker view.
//! It follows the Command pattern - input is converted to TrackerCommand
//! which can be sent to the engine for processing.
//!
//! ## Focus Management
//!
//! The tracker requires keyboard focus to work properly. The `TrackerFocus`
//! struct manages focus state and ensures keyboard input is captured when
//! the tracker is active.

use super::state::{RowIndex, TrackerColumn, TrackerViewState};

/// Commands generated from tracker input.
///
/// These commands should be sent to the engine for processing.
/// The GUI should not modify pattern data directly.
#[derive(Debug, Clone, PartialEq)]
pub enum TrackerCommand {
    /// Set a note at the current cursor position.
    SetNote {
        row: u16,
        track: u8,
        midi_note: u8,
        instrument: u8,
        velocity: u8,
    },
    /// Set a note-off at the current cursor position.
    SetNoteOff { row: u16, track: u8 },
    /// Clear the cell at the current cursor position.
    ClearCell { row: u16, track: u8 },
    /// Set instrument number at current position.
    SetInstrument { row: u16, track: u8, instrument: u8 },
    /// Set volume at current position.
    SetVolume { row: u16, track: u8, volume: u8 },
    /// Set effect at current position.
    SetEffect {
        row: u16,
        track: u8,
        effect_type: u8,
        effect_value: u8,
    },
    /// Play a note (for preview/audition).
    PlayNote { midi_note: u8, velocity: u8 },
    /// Stop a note (for preview/audition).
    StopNote { midi_note: u8 },
}

/// Cursor position in the tracker grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrackerCursor {
    /// Row index.
    pub row: RowIndex,
    /// Track (channel) index.
    pub track: u8,
    /// Column within the track.
    pub column: TrackerColumn,
}

impl TrackerCursor {
    /// Create a new cursor at the given position.
    #[must_use]
    pub const fn new(row: u16, track: u8, column: TrackerColumn) -> Self {
        Self {
            row: RowIndex(row as usize),
            track,
            column,
        }
    }

    /// Get the row as u16.
    #[must_use]
    pub fn row_u16(&self) -> u16 {
        self.row.get() as u16
    }
}

/// Result of input handling.
#[derive(Debug, Default)]
pub struct InputResult {
    /// Commands to send to the engine.
    pub commands: Vec<TrackerCommand>,
    /// Whether the cursor position changed.
    pub cursor_moved: bool,
}

impl InputResult {
    /// Create an empty result.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            commands: Vec::new(),
            cursor_moved: false,
        }
    }

    /// Add a command.
    pub fn add_command(&mut self, cmd: TrackerCommand) {
        self.commands.push(cmd);
    }
}

/// Context for tracker input handling.
pub struct TrackerInputContext {
    /// Maximum number of rows in the pattern.
    pub max_rows: usize,
    /// Number of tracks in the pattern.
    pub num_tracks: usize,
    /// Current octave for note entry.
    pub octave: u8,
    /// Step size (rows to advance after entry).
    pub step_size: usize,
    /// Current instrument number.
    pub current_instrument: u8,
}

impl Default for TrackerInputContext {
    fn default() -> Self {
        Self {
            max_rows: 64,
            num_tracks: 4,
            octave: 4,
            step_size: 1,
            current_instrument: 0,
        }
    }
}

/// Convert a key character to a semitone offset (piano keyboard layout).
///
/// Returns `Some(semitone)` where semitone is 0-24 (two octaves).
/// Returns `None` if the key is not a note key.
#[must_use]
pub fn key_to_semitone(key_char: char) -> Option<u8> {
    // Lower row: Z=C, S=C#, X=D, etc.
    // Upper row: Q=C+12, 2=C#+12, W=D+12, etc.
    match key_char.to_ascii_lowercase() {
        // Lower octave (Z-M row)
        'z' => Some(0),  // C
        's' => Some(1),  // C#
        'x' => Some(2),  // D
        'd' => Some(3),  // D#
        'c' => Some(4),  // E
        'v' => Some(5),  // F
        'g' => Some(6),  // F#
        'b' => Some(7),  // G
        'h' => Some(8),  // G#
        'n' => Some(9),  // A
        'j' => Some(10), // A#
        'm' => Some(11), // B
        // Upper octave (Q-I row)
        'q' => Some(12), // C+1
        '2' => Some(13), // C#+1
        'w' => Some(14), // D+1
        '3' => Some(15), // D#+1
        'e' => Some(16), // E+1
        'r' => Some(17), // F+1
        '5' => Some(18), // F#+1
        't' => Some(19), // G+1
        '6' => Some(20), // G#+1
        'y' => Some(21), // A+1
        '7' => Some(22), // A#+1
        'u' => Some(23), // B+1
        'i' => Some(24), // C+2
        _ => None,
    }
}

/// Convert an octave and semitone to a MIDI note number.
///
/// Returns `None` if the result would be outside MIDI range (0-127).
#[must_use]
pub fn to_midi_note(octave: u8, semitone: u8) -> Option<u8> {
    let note = octave.saturating_mul(12).saturating_add(semitone);
    (note <= 127).then_some(note)
}

/// Handle note entry in the Note column.
///
/// Updates state and returns commands for the engine.
pub fn handle_note_entry(
    state: &mut TrackerViewState,
    ctx: &TrackerInputContext,
    semitone: u8,
) -> InputResult {
    let mut result = InputResult::empty();

    let Some(midi_note) = to_midi_note(state.octave, semitone) else {
        return result;
    };

    // Create command to set note
    result.add_command(TrackerCommand::SetNote {
        row: state.cursor_row.get() as u16,
        track: state.cursor_track as u8,
        midi_note,
        instrument: ctx.current_instrument,
        velocity: 100, // Default velocity
    });

    // Play the note for preview
    result.add_command(TrackerCommand::PlayNote {
        midi_note,
        velocity: 100,
    });

    // Advance cursor
    state.cursor_down(ctx.max_rows);
    result.cursor_moved = true;

    result
}

/// Handle hex digit entry for instrument/volume/effect columns.
#[must_use]
pub fn hex_char_to_value(c: char) -> Option<u8> {
    let upper = c.to_ascii_uppercase();
    match upper {
        '0'..='9' => Some(upper as u8 - b'0'),
        'A'..='F' => Some(upper as u8 - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_to_semitone() {
        assert_eq!(key_to_semitone('z'), Some(0)); // C
        assert_eq!(key_to_semitone('Z'), Some(0)); // Case insensitive
        assert_eq!(key_to_semitone('s'), Some(1)); // C#
        assert_eq!(key_to_semitone('m'), Some(11)); // B
        assert_eq!(key_to_semitone('q'), Some(12)); // C+1
        assert_eq!(key_to_semitone('i'), Some(24)); // C+2
        assert_eq!(key_to_semitone('p'), None); // Not a note key
    }

    #[test]
    fn test_to_midi_note() {
        assert_eq!(to_midi_note(4, 0), Some(48)); // C4
        assert_eq!(to_midi_note(5, 0), Some(60)); // C5 (middle C)
        assert_eq!(to_midi_note(9, 0), Some(108)); // C9
        assert_eq!(to_midi_note(10, 0), Some(120)); // C10
        assert_eq!(to_midi_note(11, 0), None); // Out of range
    }

    #[test]
    fn test_hex_char_to_value() {
        assert_eq!(hex_char_to_value('0'), Some(0));
        assert_eq!(hex_char_to_value('9'), Some(9));
        assert_eq!(hex_char_to_value('A'), Some(10));
        assert_eq!(hex_char_to_value('a'), Some(10));
        assert_eq!(hex_char_to_value('F'), Some(15));
        assert_eq!(hex_char_to_value('G'), None);
    }
}
