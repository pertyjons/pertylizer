//! Tracker view helpers.
//!
//! Structures for rendering Pattern data as tracker-style rows and columns.

use crate::sequencer::effects::EffectCommand;
use crate::sequencer::ids::InstrumentId;
use crate::sequencer::pattern::Pattern;
use crate::sequencer::pitch::{Pitch, Velocity};

/// Display representation of a note in the tracker.
#[derive(Debug, Clone)]
pub struct TrackerNoteDisplay {
    /// The pitch of the note.
    pub pitch: Pitch,
    /// Whether this is a note-off marker.
    pub is_note_off: bool,
}

impl TrackerNoteDisplay {
    /// Format the note as a tracker string (e.g., "C-4", "C#5", "---").
    pub fn as_string(&self) -> String {
        if self.is_note_off {
            return "===".to_string();
        }

        let note_names = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];
        let note_idx = (self.pitch.as_midi() % 12) as usize;
        let octave = self.pitch.octave();
        format!("{}{}", note_names[note_idx], octave)
    }
}

/// A cell in the tracker view.
#[derive(Debug, Clone, Default)]
pub struct TrackerCell {
    /// Note in this cell (if any).
    pub note: Option<TrackerNoteDisplay>,
    /// Instrument number (if any).
    pub instrument: Option<InstrumentId>,
    /// Volume (0-64 tracker style, if any).
    pub volume: Option<u8>,
    /// Effects in this cell.
    pub effects: Vec<EffectCommand>,
}

impl TrackerCell {
    /// Create a new empty cell.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a cell with a note.
    pub fn with_note(pitch: Pitch, instrument: InstrumentId, velocity: Velocity) -> Self {
        Self {
            note: Some(TrackerNoteDisplay {
                pitch,
                is_note_off: false,
            }),
            instrument: Some(instrument),
            volume: Some((velocity.as_f32() * 64.0) as u8),
            effects: Vec::new(),
        }
    }

    /// Create a note-off cell.
    pub fn note_off(pitch: Pitch) -> Self {
        Self {
            note: Some(TrackerNoteDisplay {
                pitch,
                is_note_off: true,
            }),
            instrument: None,
            volume: None,
            effects: Vec::new(),
        }
    }

    /// Check if this cell is empty.
    pub fn is_empty(&self) -> bool {
        self.note.is_none() && self.instrument.is_none() && self.volume.is_none() && self.effects.is_empty()
    }

    /// Format the cell as a tracker string.
    pub fn as_string(&self, config: &TrackerViewConfig) -> String {
        let mut parts = Vec::new();

        // Note
        if let Some(note) = &self.note {
            parts.push(note.as_string());
        } else {
            parts.push("---".to_string());
        }

        // Instrument
        if config.show_instrument {
            if let Some(inst) = self.instrument {
                parts.push(format!("{:02X}", inst.0));
            } else {
                parts.push("..".to_string());
            }
        }

        // Volume
        if config.show_volume {
            if let Some(vol) = self.volume {
                parts.push(format!("{:02X}", vol));
            } else {
                parts.push("..".to_string());
            }
        }

        // Effects
        for i in 0..config.effect_columns as usize {
            if let Some(effect) = self.effects.get(i) {
                parts.push(format_effect(effect));
            } else {
                parts.push("...".to_string());
            }
        }

        parts.join(" ")
    }
}

/// A row in the tracker view.
#[derive(Debug, Clone)]
pub struct TrackerRow {
    /// Row index (0-based).
    pub row_index: u16,
    /// Columns (one per channel/voice).
    pub columns: Vec<TrackerCell>,
}

impl TrackerRow {
    /// Create a new empty row.
    pub fn new(row_index: u16) -> Self {
        Self {
            row_index,
            columns: vec![TrackerCell::default()],
        }
    }

    /// Create a row with a specific number of columns.
    pub fn with_columns(row_index: u16, num_columns: usize) -> Self {
        Self {
            row_index,
            columns: vec![TrackerCell::default(); num_columns],
        }
    }

    /// Get the number of columns.
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Format the row number.
    pub fn format_row_number(&self, hex: bool) -> String {
        if hex {
            format!("{:02X}", self.row_index)
        } else {
            format!("{:3}", self.row_index)
        }
    }

    /// Format the entire row as a string.
    pub fn as_string(&self, config: &TrackerViewConfig) -> String {
        let row_num = self.format_row_number(config.hex_row_numbers);
        let cells: Vec<String> = self.columns.iter().map(|c| c.as_string(config)).collect();
        format!("{} | {}", row_num, cells.join(" | "))
    }
}

/// Configuration for the tracker view.
#[derive(Debug, Clone)]
pub struct TrackerViewConfig {
    /// Number of effect columns to display (1-8).
    pub effect_columns: u8,
    /// Show volume column.
    pub show_volume: bool,
    /// Show instrument column.
    pub show_instrument: bool,
    /// Display row numbers in hex.
    pub hex_row_numbers: bool,
    /// Number of channels/columns.
    pub num_channels: usize,
    /// Highlight every N rows (typically 4 or 16).
    pub highlight_interval: u16,
}

impl Default for TrackerViewConfig {
    fn default() -> Self {
        Self {
            effect_columns: 2,
            show_volume: true,
            show_instrument: true,
            hex_row_numbers: true,
            num_channels: 4,
            highlight_interval: 16,
        }
    }
}

impl TrackerViewConfig {
    /// Create a minimal config (just notes).
    pub fn minimal() -> Self {
        Self {
            effect_columns: 0,
            show_volume: false,
            show_instrument: false,
            hex_row_numbers: true,
            num_channels: 1,
            highlight_interval: 16,
        }
    }

    /// Create a config for ProTracker-style display.
    pub fn protracker() -> Self {
        Self {
            effect_columns: 1,
            show_volume: false,
            show_instrument: true,
            hex_row_numbers: true,
            num_channels: 4,
            highlight_interval: 16,
        }
    }

    /// Create a config for FastTracker-style display.
    pub fn fasttracker() -> Self {
        Self {
            effect_columns: 2,
            show_volume: true,
            show_instrument: true,
            hex_row_numbers: true,
            num_channels: 8,
            highlight_interval: 4,
        }
    }

    /// Check if a row should be highlighted.
    pub fn should_highlight(&self, row: u16) -> bool {
        self.highlight_interval > 0 && row % self.highlight_interval == 0
    }
}

/// Format an effect command as a tracker string.
fn format_effect(effect: &EffectCommand) -> String {
    match effect {
        EffectCommand::Arpeggio { x, y } => format!("0{:X}{:X}", x, y),
        EffectCommand::PortamentoUp(speed) => format!("1{:02X}", speed),
        EffectCommand::PortamentoDown(speed) => format!("2{:02X}", speed),
        EffectCommand::TonePortamento { speed, .. } => format!("3{:02X}", speed),
        EffectCommand::Vibrato { speed, depth } => format!("4{:X}{:X}", speed, depth),
        EffectCommand::VolumeSlide { up, down } => format!("A{:X}{:X}", up, down),
        EffectCommand::SetVolume(vol) => format!("C{:02X}", vol),
        EffectCommand::PatternBreak(row) => format!("D{:02X}", row),
        EffectCommand::SetTempo(bpm) => format!("F{:02X}", bpm.min(&255)),
        EffectCommand::SetSpeed(speed) => format!("F{:02X}", speed),
        EffectCommand::SampleOffset(offset) => format!("9{:02X}", offset & 0xFF),
        EffectCommand::Retrigger { interval, .. } => format!("E9{:X}", interval),
        EffectCommand::NoteCut(ticks) => format!("EC{:X}", ticks),
        EffectCommand::NoteDelay(ticks) => format!("ED{:X}", ticks),
        EffectCommand::SetPanning(pan) => format!("8{:02X}", pan),
        EffectCommand::Tremolo { speed, depth } => format!("7{:X}{:X}", speed, depth),
        _ => "...".to_string(),
    }
}

/// Convert a Pattern to tracker rows for rendering.
pub fn to_tracker_rows(pattern: &Pattern, config: &TrackerViewConfig) -> Vec<TrackerRow> {
    let num_rows = pattern.row_resolution.rows as usize;
    let mut rows: Vec<TrackerRow> = (0..num_rows)
        .map(|i| TrackerRow::with_columns(i as u16, config.num_channels))
        .collect();

    for note in pattern.notes() {
        let row_idx = pattern.row_resolution.tick_to_row(note.start) as usize;
        if row_idx >= num_rows {
            continue;
        }

        let row = &mut rows[row_idx];

        // Find an empty column or create a new one
        let col_idx = row
            .columns
            .iter()
            .position(|c| c.note.is_none())
            .unwrap_or_else(|| {
                row.columns.push(TrackerCell::default());
                row.columns.len() - 1
            });

        let cell = &mut row.columns[col_idx];
        cell.note = Some(TrackerNoteDisplay {
            pitch: note.pitch,
            is_note_off: false,
        });
        cell.instrument = Some(note.instrument);
        cell.volume = Some((note.velocity.as_f32() * 64.0) as u8);

        // Copy effects (up to configured limit)
        cell.effects = note
            .effects
            .iter()
            .take(config.effect_columns as usize)
            .copied()
            .collect();

        // Add note-off at end row
        if let Some(end) = note.end() {
            let end_row_idx = pattern.row_resolution.tick_to_row(end) as usize;
            if end_row_idx < num_rows && end_row_idx > row_idx {
                let end_row = &mut rows[end_row_idx];

                // Ensure we have enough columns
                while end_row.columns.len() <= col_idx {
                    end_row.columns.push(TrackerCell::default());
                }

                if end_row.columns[col_idx].note.is_none() {
                    end_row.columns[col_idx].note = Some(TrackerNoteDisplay {
                        pitch: note.pitch,
                        is_note_off: true,
                    });
                }
            }
        }
    }

    rows
}

/// Extension trait for Pattern to add tracker view conversion.
pub trait PatternTrackerView {
    /// Convert this pattern to tracker rows.
    fn to_tracker_rows(&self, config: &TrackerViewConfig) -> Vec<TrackerRow>;
}

impl PatternTrackerView for Pattern {
    fn to_tracker_rows(&self, config: &TrackerViewConfig) -> Vec<TrackerRow> {
        to_tracker_rows(self, config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequencer::ids::PatternId;
    use crate::sequencer::pattern::RowResolution;
    use crate::sequencer::pitch::Velocity;
    use crate::sequencer::time::{Duration, PatternTick};

    #[test]
    fn test_note_display_string() {
        let note = TrackerNoteDisplay {
            pitch: Pitch::new(60).unwrap(), // C4
            is_note_off: false,
        };
        assert_eq!(note.as_string(), "C-4");

        let note_off = TrackerNoteDisplay {
            pitch: Pitch::new(60).unwrap(),
            is_note_off: true,
        };
        assert_eq!(note_off.as_string(), "===");
    }

    #[test]
    fn test_tracker_cell() {
        let cell = TrackerCell::with_note(
            Pitch::new(60).unwrap(),
            InstrumentId(1),
            Velocity::MF,
        );

        assert!(cell.note.is_some());
        assert_eq!(cell.instrument, Some(InstrumentId(1)));
        assert!(!cell.is_empty());

        let empty = TrackerCell::default();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_tracker_row_format() {
        let _config = TrackerViewConfig::default();
        let mut row = TrackerRow::new(16);
        row.columns[0] = TrackerCell::with_note(
            Pitch::new(60).unwrap(),
            InstrumentId(1),
            Velocity::MF,
        );

        let row_num = row.format_row_number(true);
        assert_eq!(row_num, "10"); // 16 in hex
    }

    #[test]
    fn test_pattern_to_tracker_rows() {
        let mut pattern = Pattern::new(PatternId(0), Duration(3840))
            .with_row_resolution(RowResolution::standard_64());

        // Add a note at row 0
        pattern.add_note(
            PatternTick(0),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            InstrumentId(0),
        );

        let config = TrackerViewConfig::default();
        let rows = pattern.to_tracker_rows(&config);

        assert_eq!(rows.len(), 64);
        assert!(rows[0].columns[0].note.is_some());
        assert_eq!(
            rows[0].columns[0].note.as_ref().unwrap().pitch.as_midi(),
            60
        );
    }

    #[test]
    fn test_tracker_row_with_note_off() {
        let mut pattern = Pattern::new(PatternId(0), Duration(3840))
            .with_row_resolution(RowResolution::standard_64());

        // Add a note with duration (4 rows = 960 ticks)
        use crate::sequencer::note::Note;
        let note = Note::new(
            crate::sequencer::ids::NoteId(0),
            PatternTick(0),
            Pitch::new(60).unwrap(),
            Velocity::MF,
            InstrumentId(0),
        )
        .with_duration(Duration(960)); // 4 rows

        pattern.insert_note(note);

        let config = TrackerViewConfig::default();
        let rows = pattern.to_tracker_rows(&config);

        // Note-on at row 0
        assert!(rows[0].columns[0].note.is_some());
        assert!(!rows[0].columns[0].note.as_ref().unwrap().is_note_off);

        // Note-off at row 4
        assert!(rows[4].columns[0].note.is_some());
        assert!(rows[4].columns[0].note.as_ref().unwrap().is_note_off);
    }

    #[test]
    fn test_highlight_interval() {
        let config = TrackerViewConfig {
            highlight_interval: 16,
            ..Default::default()
        };

        assert!(config.should_highlight(0));
        assert!(!config.should_highlight(4));
        assert!(config.should_highlight(16));
        assert!(config.should_highlight(32));
    }

    #[test]
    fn test_effect_formatting() {
        assert_eq!(format_effect(&EffectCommand::Arpeggio { x: 3, y: 7 }), "037");
        assert_eq!(format_effect(&EffectCommand::PortamentoUp(15)), "10F");
        assert_eq!(format_effect(&EffectCommand::SetVolume(64)), "C40");
    }
}
