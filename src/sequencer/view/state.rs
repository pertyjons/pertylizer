//! Tracker view state types.
//!
//! Contains strong types for navigation and editor state in the tracker view.
//! Follows the newtype pattern for type safety.

use serde::{Deserialize, Serialize};

use crate::sequencer::ids::PatternId;

/// Stark typ for row index in the tracker.
/// Prevents confusion between row indices and other integer values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub struct RowIndex(pub usize);

impl RowIndex {
    /// Create a new row index.
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// Get the underlying value.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }

    /// Saturating addition.
    #[must_use]
    pub fn saturating_add(self, delta: usize) -> Self {
        Self(self.0.saturating_add(delta))
    }

    /// Saturating subtraction.
    #[must_use]
    pub fn saturating_sub(self, delta: usize) -> Self {
        Self(self.0.saturating_sub(delta))
    }
}

/// Which sub-column within a track the cursor is positioned at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TrackerColumn {
    /// Note column (C-4, D#5, ---, ===).
    #[default]
    Note,
    /// Instrument number (00-FF).
    Instrument,
    /// Volume (00-40 in tracker style).
    Volume,
    /// Effect type (0-F).
    EffectType,
    /// Effect value (00-FF).
    EffectValue,
}

impl TrackerColumn {
    /// Move to the next column (wraps to Note).
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Note => Self::Instrument,
            Self::Instrument => Self::Volume,
            Self::Volume => Self::EffectType,
            Self::EffectType => Self::EffectValue,
            Self::EffectValue => Self::Note, // Wrap handled by caller
        }
    }

    /// Move to the previous column (wraps to EffectValue).
    #[must_use]
    pub const fn prev(self) -> Self {
        match self {
            Self::Note => Self::EffectValue, // Wrap handled by caller
            Self::Instrument => Self::Note,
            Self::Volume => Self::Instrument,
            Self::EffectType => Self::Volume,
            Self::EffectValue => Self::EffectType,
        }
    }

    /// Check if this is the first column.
    #[must_use]
    pub const fn is_first(self) -> bool {
        matches!(self, Self::Note)
    }

    /// Check if this is the last column.
    #[must_use]
    pub const fn is_last(self) -> bool {
        matches!(self, Self::EffectValue)
    }
}

/// Tracker view state (UI state, not song data).
///
/// This state controls navigation, editing mode, and display settings.
/// It is separate from the actual song/pattern data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerViewState {
    /// Currently active pattern for editing.
    pub active_pattern: Option<PatternId>,
    /// Cursor row position.
    pub cursor_row: RowIndex,
    /// Cursor track index (which track/channel).
    pub cursor_track: usize,
    /// Cursor column within the track.
    pub cursor_column: TrackerColumn,
    /// Scroll offset (first visible row).
    pub scroll_offset: RowIndex,
    /// Current octave for note entry (0-9).
    pub octave: u8,
    /// Step size - how many rows to advance after entering a note.
    pub step_size: usize,
    /// Whether to follow playback position.
    pub follow_playback: bool,
    /// Number of visible tracks (can be less than total tracks).
    pub visible_tracks: usize,
    /// First visible track index.
    pub first_visible_track: usize,
}

impl Default for TrackerViewState {
    fn default() -> Self {
        Self {
            active_pattern: None,
            cursor_row: RowIndex(0),
            cursor_track: 0,
            cursor_column: TrackerColumn::Note,
            scroll_offset: RowIndex(0),
            octave: 4,
            step_size: 1,
            follow_playback: true,
            visible_tracks: 4,
            first_visible_track: 0,
        }
    }
}

impl TrackerViewState {
    /// Create a new tracker view state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Move cursor down by step_size rows.
    pub fn cursor_down(&mut self, max_rows: usize) {
        let new_row = self.cursor_row.0 + self.step_size;
        if new_row < max_rows {
            self.cursor_row = RowIndex(new_row);
        }
    }

    /// Move cursor up by step_size rows.
    pub fn cursor_up(&mut self) {
        self.cursor_row = self.cursor_row.saturating_sub(self.step_size);
    }

    /// Move cursor to the next column, wrapping to next track if needed.
    pub fn cursor_right(&mut self, num_tracks: usize) {
        if self.cursor_column.is_last() {
            // Move to next track
            if self.cursor_track + 1 < num_tracks {
                self.cursor_track += 1;
                self.cursor_column = TrackerColumn::Note;
            }
        } else {
            self.cursor_column = self.cursor_column.next();
        }
    }

    /// Move cursor to the previous column, wrapping to previous track if needed.
    pub fn cursor_left(&mut self) {
        if self.cursor_column.is_first() {
            // Move to previous track
            if self.cursor_track > 0 {
                self.cursor_track -= 1;
                self.cursor_column = TrackerColumn::EffectValue;
            }
        } else {
            self.cursor_column = self.cursor_column.prev();
        }
    }

    /// Jump to start of pattern.
    pub fn goto_start(&mut self) {
        self.cursor_row = RowIndex(0);
        self.scroll_offset = RowIndex(0);
    }

    /// Jump to end of pattern.
    pub fn goto_end(&mut self, max_rows: usize) {
        if max_rows > 0 {
            self.cursor_row = RowIndex(max_rows - 1);
        }
    }

    /// Increase octave (max 9).
    pub fn octave_up(&mut self) {
        if self.octave < 9 {
            self.octave += 1;
        }
    }

    /// Decrease octave (min 0).
    pub fn octave_down(&mut self) {
        if self.octave > 0 {
            self.octave -= 1;
        }
    }

    /// Ensure cursor is visible by adjusting scroll.
    pub fn ensure_cursor_visible(&mut self, visible_rows: usize) {
        // Scroll down if cursor is below visible area
        if self.cursor_row.0 >= self.scroll_offset.0 + visible_rows {
            self.scroll_offset = RowIndex(self.cursor_row.0.saturating_sub(visible_rows - 1));
        }
        // Scroll up if cursor is above visible area
        if self.cursor_row.0 < self.scroll_offset.0 {
            self.scroll_offset = self.cursor_row;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_row_index_arithmetic() {
        let row = RowIndex(5);
        assert_eq!(row.saturating_add(3).get(), 8);
        assert_eq!(row.saturating_sub(3).get(), 2);
        assert_eq!(row.saturating_sub(10).get(), 0);
    }

    #[test]
    fn test_tracker_column_navigation() {
        let col = TrackerColumn::Note;
        assert_eq!(col.next(), TrackerColumn::Instrument);
        assert_eq!(TrackerColumn::EffectValue.next(), TrackerColumn::Note);

        assert_eq!(TrackerColumn::Note.prev(), TrackerColumn::EffectValue);
        assert_eq!(TrackerColumn::Volume.prev(), TrackerColumn::Instrument);
    }

    #[test]
    fn test_cursor_movement() {
        let mut state = TrackerViewState::default();
        state.step_size = 1;

        state.cursor_down(64);
        assert_eq!(state.cursor_row.get(), 1);

        state.cursor_up();
        assert_eq!(state.cursor_row.get(), 0);

        // Can't go below 0
        state.cursor_up();
        assert_eq!(state.cursor_row.get(), 0);
    }

    #[test]
    fn test_cursor_horizontal_movement() {
        let mut state = TrackerViewState::default();

        // Start at Note, track 0
        state.cursor_right(4);
        assert_eq!(state.cursor_column, TrackerColumn::Instrument);
        assert_eq!(state.cursor_track, 0);

        // Move all the way to EffectValue
        state.cursor_column = TrackerColumn::EffectValue;
        state.cursor_right(4);
        // Should wrap to next track
        assert_eq!(state.cursor_column, TrackerColumn::Note);
        assert_eq!(state.cursor_track, 1);
    }

    #[test]
    fn test_octave_limits() {
        let mut state = TrackerViewState::default();
        state.octave = 9;
        state.octave_up();
        assert_eq!(state.octave, 9);

        state.octave = 0;
        state.octave_down();
        assert_eq!(state.octave, 0);
    }
}
