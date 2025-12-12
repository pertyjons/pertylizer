//! Sequencer module for the modular synthesizer.
//!
//! This crate provides a flexible sequencer that supports both tracker-style composition
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

#![allow(clippy::must_use_candidate)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::too_many_lines)]

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
pub use automation::{AutoInstrumentParam, GlobalParam, TrackParam};
pub use automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveType};
pub use effects::{EffectCommand, EffectWaveform};
pub use events::{EventSorting, SequencerEvent};
pub use ids::{NoteId, PatternId, SeqInstrumentId, TrackId};
pub use input::{InputCommand, InputMultiplexer, InputSource, KeyboardInputSource};
pub use note::Note;
pub use pattern::{Pattern, RowResolution, TrackCell, TrackerGrid};
pub use pitch::{NoteName, Pitch, Velocity};
pub use song::{PatternPlacement, Song, TempoChange, TimeSignatureChange};
pub use time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
pub use track::{SequencerTrack, TrackColor, TrackMode};
pub use view::tracker::{
    PatternTrackerView, TrackerCell, TrackerNoteDisplay, TrackerRow, TrackerViewConfig,
};

/// Prelude module for convenient imports.
pub mod prelude {
    pub use super::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveType};
    pub use super::effects::EffectCommand;
    pub use super::events::SequencerEvent;
    pub use super::ids::{NoteId, PatternId, SeqInstrumentId, TrackId};
    pub use super::input::{InputCommand, InputSource};
    pub use super::note::Note;
    pub use super::pattern::{Pattern, RowResolution};
    pub use super::pitch::{NoteName, Pitch, Velocity};
    pub use super::song::Song;
    pub use super::time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
    pub use super::track::SequencerTrack;
    pub use super::view::tracker::{PatternTrackerView, TrackerRow, TrackerViewConfig};
}
