//! Input abstraction for the sequencer.
//!
//! Provides a unified interface for all input sources (MIDI, keyboard, GUI, etc.).

use super::ids::{InstrumentId, NoteId, PatternId};
use super::pitch::{Pitch, Velocity};
use super::time::{Duration, PatternTick, Tick, TimeSignature};
use synth_core::{Bpm, NormalizedValue, Octaves, Semitones};

/// Input commands from any source.
#[derive(Debug, Clone)]
pub enum InputCommand {
    // === Real-time input ===
    /// Note on (from MIDI, keyboard, etc.)
    NoteOn {
        pitch: Pitch,
        velocity: Velocity,
        instrument: Option<InstrumentId>,
    },
    /// Note off.
    NoteOff { pitch: Pitch },

    // === Pattern editing ===
    /// Add note to pattern.
    AddNote {
        pattern: PatternId,
        start: PatternTick,
        duration: Option<Duration>,
        pitch: Pitch,
        velocity: Velocity,
    },
    /// Remove note.
    RemoveNote { pattern: PatternId, note_id: NoteId },
    /// Move note to new position.
    MoveNote {
        pattern: PatternId,
        note_id: NoteId,
        new_start: PatternTick,
    },
    /// Resize note duration.
    ResizeNote {
        pattern: PatternId,
        note_id: NoteId,
        new_duration: Duration,
    },
    /// Transpose note.
    TransposeNote {
        pattern: PatternId,
        note_id: NoteId,
        semitones: Semitones,
    },
    /// Set note velocity.
    SetNoteVelocity {
        pattern: PatternId,
        note_id: NoteId,
        velocity: Velocity,
    },
    // === Selection ===
    /// Select specific notes.
    SelectNotes {
        pattern: PatternId,
        note_ids: Vec<NoteId>,
    },
    /// Select notes in range.
    SelectRange {
        pattern: PatternId,
        start: PatternTick,
        end: PatternTick,
        pitch_min: Option<Pitch>,
        pitch_max: Option<Pitch>,
    },
    /// Add notes to selection.
    AddToSelection {
        pattern: PatternId,
        note_ids: Vec<NoteId>,
    },
    /// Remove notes from selection.
    RemoveFromSelection {
        pattern: PatternId,
        note_ids: Vec<NoteId>,
    },
    /// Clear selection.
    ClearSelection,
    /// Select all notes in pattern.
    SelectAll { pattern: PatternId },

    // === Clipboard operations ===
    /// Delete selected notes.
    DeleteSelection,
    /// Copy selection to clipboard.
    CopySelection,
    /// Cut selection to clipboard.
    CutSelection,
    /// Paste from clipboard.
    PasteAt {
        pattern: PatternId,
        tick: PatternTick,
    },
    /// Duplicate selection.
    DuplicateSelection {
        pattern: PatternId,
        offset: PatternTick,
    },

    // === Quantization ===
    /// Quantize notes to grid.
    Quantize {
        pattern: PatternId,
        strength: NormalizedValue,
    },
    /// Quantize selected notes only.
    QuantizeSelection { strength: NormalizedValue },

    // === Bulk operations ===
    /// Transpose selection.
    TransposeSelection { semitones: Semitones },
    /// Scale velocities.
    ScaleVelocities { pattern: PatternId, factor: f32 },
    /// Humanize timing.
    Humanize {
        pattern: PatternId,
        timing_amount: NormalizedValue,
        velocity_amount: NormalizedValue,
    },

    // === Transport ===
    /// Start playback.
    Play,
    /// Stop playback.
    Stop,
    /// Pause playback.
    Pause,
    /// Toggle between play and pause.
    TogglePlayPause,
    /// Toggle record mode.
    ToggleRecord,
    /// Set playback position.
    SetPosition(Tick),
    /// Step forward/backward by ticks.
    StepPosition(i64),
    /// Go to start.
    GoToStart,
    /// Go to end.
    GoToEnd,

    // === Loop ===
    /// Set loop region.
    SetLoop {
        start: Tick,
        end: Tick,
        enabled: bool,
    },
    /// Toggle loop on/off.
    ToggleLoop,
    /// Set loop to selection.
    LoopSelection,

    // === Tempo and time signature ===
    /// Set tempo (BPM).
    SetTempo(Bpm),
    /// Tap tempo.
    TapTempo,
    /// Set time signature.
    SetTimeSignature(TimeSignature),

    // === Pattern management ===
    /// Create new pattern.
    CreatePattern { length: Duration },
    /// Delete pattern.
    DeletePattern { pattern: PatternId },
    /// Duplicate pattern.
    DuplicatePattern { pattern: PatternId },
    /// Rename pattern.
    RenamePattern { pattern: PatternId, name: String },

    // === Undo/Redo ===
    /// Undo last action.
    Undo,
    /// Redo last undone action.
    Redo,
}

impl InputCommand {
    /// Check if this command requires a pattern context.
    #[must_use]
    pub fn requires_pattern(&self) -> bool {
        matches!(
            self,
            Self::AddNote { .. }
                | Self::RemoveNote { .. }
                | Self::MoveNote { .. }
                | Self::ResizeNote { .. }
                | Self::TransposeNote { .. }
                | Self::SetNoteVelocity { .. }
                | Self::SelectNotes { .. }
                | Self::SelectRange { .. }
                | Self::AddToSelection { .. }
                | Self::RemoveFromSelection { .. }
                | Self::SelectAll { .. }
                | Self::PasteAt { .. }
                | Self::DuplicateSelection { .. }
                | Self::Quantize { .. }
                | Self::ScaleVelocities { .. }
                | Self::Humanize { .. }
                | Self::DeletePattern { .. }
                | Self::DuplicatePattern { .. }
                | Self::RenamePattern { .. }
        )
    }

    /// Check if this is a transport command.
    #[must_use]
    pub fn is_transport(&self) -> bool {
        matches!(
            self,
            Self::Play
                | Self::Stop
                | Self::Pause
                | Self::TogglePlayPause
                | Self::ToggleRecord
                | Self::SetPosition(_)
                | Self::StepPosition(_)
                | Self::GoToStart
                | Self::GoToEnd
        )
    }

    /// Check if this is an editing command.
    #[must_use]
    pub fn is_editing(&self) -> bool {
        matches!(
            self,
            Self::AddNote { .. }
                | Self::RemoveNote { .. }
                | Self::MoveNote { .. }
                | Self::ResizeNote { .. }
                | Self::TransposeNote { .. }
                | Self::SetNoteVelocity { .. }
                | Self::DeleteSelection
                | Self::CutSelection
                | Self::PasteAt { .. }
                | Self::DuplicateSelection { .. }
                | Self::Quantize { .. }
                | Self::QuantizeSelection { .. }
                | Self::TransposeSelection { .. }
                | Self::ScaleVelocities { .. }
                | Self::Humanize { .. }
        )
    }

    /// Check if this command should be recorded for undo.
    #[must_use]
    pub fn is_undoable(&self) -> bool {
        self.is_editing()
            || matches!(
                self,
                Self::CreatePattern { .. }
                    | Self::DeletePattern { .. }
                    | Self::DuplicatePattern { .. }
                    | Self::RenamePattern { .. }
            )
    }

    /// Check if this is a real-time input (note on/off).
    #[must_use]
    pub fn is_realtime_input(&self) -> bool {
        matches!(self, Self::NoteOn { .. } | Self::NoteOff { .. })
    }
}

/// Trait for input sources.
pub trait InputSource: Send {
    /// Poll for new commands.
    fn poll(&mut self) -> Vec<InputCommand>;

    /// Name for UI display.
    fn name(&self) -> &str;

    /// Check if the source is connected/active.
    fn is_active(&self) -> bool;

    /// Enable or disable the source.
    fn set_enabled(&mut self, enabled: bool);

    /// Check if the source is enabled.
    fn is_enabled(&self) -> bool;
}

/// A keyboard input source (computer keyboard as piano).
#[derive(Debug, Default)]
pub struct KeyboardInputSource {
    name: String,
    enabled: bool,
    pending_commands: Vec<InputCommand>,
    base_octave: Octaves,
    velocity: Velocity,
}

impl KeyboardInputSource {
    /// Create a new keyboard input source.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            enabled: true,
            pending_commands: Vec::new(),
            base_octave: Octaves::new(4),
            velocity: Velocity::MF,
        }
    }

    /// Set the base octave for keyboard input.
    pub fn set_base_octave(&mut self, octave: Octaves) {
        self.base_octave = Octaves::new(octave.as_i32().clamp(-1, 8));
    }

    /// Get the current base octave.
    #[must_use]
    pub fn base_octave(&self) -> Octaves {
        self.base_octave
    }

    /// Set the velocity for keyboard input.
    pub fn set_velocity(&mut self, velocity: Velocity) {
        self.velocity = velocity;
    }

    /// Get the current velocity.
    #[must_use]
    pub fn velocity(&self) -> Velocity {
        self.velocity
    }

    /// Push a command to be polled.
    pub fn push_command(&mut self, command: InputCommand) {
        if self.enabled {
            self.pending_commands.push(command);
        }
    }

    /// Handle a key press (note on).
    pub fn key_down(&mut self, note_offset: u8, instrument: Option<InstrumentId>) {
        let midi_note = (self.base_octave.as_i32() + 1) * 12 + i32::from(note_offset);
        let Ok(midi_note) = u8::try_from(midi_note) else {
            return;
        };
        if let Some(pitch) = Pitch::new(midi_note) {
            self.push_command(InputCommand::NoteOn {
                pitch,
                velocity: self.velocity,
                instrument,
            });
        }
    }

    /// Handle a key release (note off).
    pub fn key_up(&mut self, note_offset: u8) {
        let midi_note = (self.base_octave.as_i32() + 1) * 12 + i32::from(note_offset);
        let Ok(midi_note) = u8::try_from(midi_note) else {
            return;
        };
        if let Some(pitch) = Pitch::new(midi_note) {
            self.push_command(InputCommand::NoteOff { pitch });
        }
    }
}

impl InputSource for KeyboardInputSource {
    fn poll(&mut self) -> Vec<InputCommand> {
        std::mem::take(&mut self.pending_commands)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_active(&self) -> bool {
        self.enabled
    }

    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.pending_commands.clear();
        }
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// An input multiplexer that combines multiple input sources.
pub struct InputMultiplexer {
    sources: Vec<Box<dyn InputSource>>,
}

impl InputMultiplexer {
    /// Create a new input multiplexer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    /// Add an input source.
    pub fn add_source(&mut self, source: Box<dyn InputSource>) {
        self.sources.push(source);
    }

    /// Remove an input source by name.
    pub fn remove_source(&mut self, name: &str) -> Option<Box<dyn InputSource>> {
        let pos = self.sources.iter().position(|s| s.name() == name)?;
        Some(self.sources.remove(pos))
    }

    /// Poll all sources for commands.
    pub fn poll_all(&mut self) -> Vec<InputCommand> {
        let mut commands = Vec::new();
        for source in &mut self.sources {
            if source.is_active() {
                commands.extend(source.poll());
            }
        }
        commands
    }

    /// Get list of source names.
    #[must_use]
    pub fn source_names(&self) -> Vec<&str> {
        self.sources.iter().map(|s| s.name()).collect()
    }

    /// Get number of sources.
    #[must_use]
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }
}

impl Default for InputMultiplexer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::assert_matches;

    #[test]
    fn test_input_command_categories() {
        let play = InputCommand::Play;
        assert!(play.is_transport());
        assert!(!play.is_editing());
        assert!(!play.is_undoable());

        let add_note = InputCommand::AddNote {
            pattern: PatternId(0),
            start: PatternTick(0),
            duration: Some(Duration::QUARTER),
            pitch: Pitch::new(60).unwrap(),
            velocity: Velocity::MF,
        };
        assert!(!add_note.is_transport());
        assert!(add_note.is_editing());
        assert!(add_note.is_undoable());

        let note_on = InputCommand::NoteOn {
            pitch: Pitch::new(60).unwrap(),
            velocity: Velocity::MF,
            instrument: None,
        };
        assert!(note_on.is_realtime_input());
    }

    #[test]
    fn test_keyboard_input_source() {
        let mut keyboard = KeyboardInputSource::new("Test Keyboard");
        assert!(keyboard.is_active());
        assert!(keyboard.is_enabled());

        // Key down
        keyboard.key_down(0, None); // C at base octave
        let commands = keyboard.poll();
        assert_eq!(commands.len(), 1);
        assert_matches!(commands[0], InputCommand::NoteOn { .. });

        // Key up
        keyboard.key_up(0);
        let commands = keyboard.poll();
        assert_eq!(commands.len(), 1);
        assert_matches!(commands[0], InputCommand::NoteOff { .. });
    }

    #[test]
    fn test_keyboard_octave() {
        let mut keyboard = KeyboardInputSource::new("Test");
        keyboard.set_base_octave(Octaves::new(5));
        assert_eq!(keyboard.base_octave(), Octaves::new(5));

        keyboard.key_down(0, None);
        let commands = keyboard.poll();
        if let InputCommand::NoteOn { pitch, .. } = &commands[0] {
            // Octave 5 * 12 + C = 72
            assert_eq!(pitch.as_midi(), 72);
        } else {
            panic!("Expected NoteOn");
        }
    }

    #[test]
    fn test_input_multiplexer() {
        let mut mux = InputMultiplexer::new();

        let mut kb1 = KeyboardInputSource::new("Keyboard 1");
        kb1.key_down(0, None);
        mux.add_source(Box::new(kb1));

        let mut kb2 = KeyboardInputSource::new("Keyboard 2");
        kb2.key_down(12, None);
        mux.add_source(Box::new(kb2));

        assert_eq!(mux.source_count(), 2);

        let commands = mux.poll_all();
        assert_eq!(commands.len(), 2);
    }

    #[test]
    fn test_disabled_source() {
        let mut keyboard = KeyboardInputSource::new("Test");
        keyboard.set_enabled(false);

        keyboard.key_down(0, None);
        let commands = keyboard.poll();
        assert!(commands.is_empty());
    }
}
