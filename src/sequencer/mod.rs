//! Sequencer module for the modular synthesizer.
//!
//! This module provides a flexible sequencer that supports both tracker-style composition
//! (inspired by ProTracker/FastTracker) and modern piano roll-style editing.
//!
//! ## Core Principles
//!
//! - **Storage vs Runtime**: Notes are stored as objects with start time and duration.
//!   At playback, they are converted to NoteOn/NoteOff event streams.
//! - **Type Safety**: Uses newtypes and enums for all domain concepts.
//! - **View Agnostic**: The same data can be rendered as tracker rows or piano roll.
//! - **960 PPQN**: All time is measured in ticks with 960 ticks per quarter note.
//!
//! ## Module Structure
//!
//! - [`time`] - Time types (Tick, PatternTick, Duration, TimeSignature)
//! - [`pitch`] - Pitch and velocity types
//! - [`ids`] - Type-safe identifiers
//! - [`effects`] - Tracker-style effect commands
//! - [`note`] - Note storage
//! - [`automation`] - Parameter automation
//! - [`pattern`] - Pattern container
//! - [`track`] - Track definition
//! - [`song`] - Song arrangement
//! - [`events`] - Runtime events for playback
//! - [`input`] - Input command abstraction
//! - [`view`] - View helpers (tracker, piano roll)
//!
//! ## Example
//!
//! ```rust
//! use modular_synth::sequencer::*;
//! use modular_synth::types::Bpm;
//!
//! // Create a new song
//! let mut song = Song::new("My Song")
//!     .with_author("Composer")
//!     .with_tempo(Bpm::new(140.0));
//!
//! // Create a pattern (4 bars at standard resolution)
//! let pattern_id = song.create_pattern(Duration::WHOLE * 4);
//! let pattern = song.pattern_mut(pattern_id).unwrap();
//!
//! // Add a note
//! let note_id = pattern.add_note(
//!     PatternTick(0),
//!     Pitch::new(60).unwrap(), // Middle C
//!     Velocity::MF,
//!     InstrumentId(0),
//! );
//!
//! // Create a track and place the pattern
//! let track_id = song.create_track("Lead");
//! song.place_pattern(pattern_id, track_id, Tick(0));
//! ```

pub mod automation;
pub mod effects;
pub mod events;
pub mod ids;
pub mod input;
pub mod note;
pub mod pattern;
pub mod pitch;
pub mod song;
pub mod time;
pub mod track;
pub mod view;

// Re-export commonly used types
pub use automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveType};
pub use automation::{GlobalParam, InstrumentParam, TrackParam};
pub use effects::{EffectCommand, EffectWaveform};
pub use events::{EventSorting, SequencerEvent};
pub use ids::{InstrumentId, NoteId, PatternId, TrackId};
pub use input::{InputCommand, InputMultiplexer, InputSource, KeyboardInputSource};
pub use note::Note;
pub use pattern::{Pattern, RowResolution};
pub use pitch::{NoteName, Pitch, Velocity};
pub use song::{PatternPlacement, Song, TempoChange, TimeSignatureChange};
pub use time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
pub use track::{SequencerTrack, TrackColor};
pub use view::tracker::{
    PatternTrackerView, TrackerCell, TrackerNoteDisplay, TrackerRow, TrackerViewConfig,
};

/// Prelude module for convenient imports.
pub mod prelude {
    pub use super::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveType};
    pub use super::effects::EffectCommand;
    pub use super::events::SequencerEvent;
    pub use super::ids::{InstrumentId, NoteId, PatternId, TrackId};
    pub use super::input::{InputCommand, InputSource};
    pub use super::note::Note;
    pub use super::pattern::{Pattern, RowResolution};
    pub use super::pitch::{NoteName, Pitch, Velocity};
    pub use super::song::Song;
    pub use super::time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
    pub use super::track::SequencerTrack;
    pub use super::view::tracker::{PatternTrackerView, TrackerRow, TrackerViewConfig};
}
