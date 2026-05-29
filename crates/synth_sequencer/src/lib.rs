//! Sequencer module for Pertylizer.
//!
//! This crate provides a flexible sequencer that supports piano roll-style editing.
//!
//! ## Core Principles
//!
//! - **Storage vs Runtime**: Notes are stored as objects with start time and duration.
//!   At playback, they are converted to NoteOn/NoteOff event streams.
//! - **Type Safety**: Uses newtypes and enums for all domain concepts.
//! - **960 PPQN**: All time is measured in ticks with 960 ticks per quarter note.
//!
//! ## Module Structure
//!
//! - [`time`] - Time types (Tick, PatternTick, Duration, TimeSignature)
//! - [`pitch`] - Pitch and velocity types
//! - [`ids`] - Type-safe identifiers
//! - [`note`] - Note storage
//! - [`automation`] - Parameter automation
//! - [`pattern`] - Pattern container
//! - [`track`] - Track definition
//! - [`song`] - Song arrangement
//! - [`events`] - Runtime events for playback
//! - [`input`] - Input command abstraction

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::too_many_lines)]

pub mod automation;
pub mod events;
pub mod ids;
pub mod input;
pub mod note;
pub mod pattern;
pub mod pitch;
pub mod song;
pub mod time;
pub mod track;

// Re-export commonly used types
pub use automation::{AutoInstrumentParam, GlobalParam, TrackParam};
pub use automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveType};
pub use events::SequencerEvent;
pub use ids::{
    NoteId, PatternId, RowCount, RowIndex, SeqInstrumentId, TicksPerRow, TrackCount, TrackId,
    TrackIndex,
};
pub use input::{InputCommand, InputMultiplexer, InputSource, KeyboardInputSource};
pub use note::{Glide, GlideFrom, GlideInterp, Note};
pub use pattern::{Pattern, RowResolution};
pub use pitch::{NoteName, Pitch, Velocity};
pub use song::{PatternPlacement, Song, TempoChange, TimeSignatureChange};
pub use time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
pub use track::{SequencerTrack, TrackColor, TrackMode};

/// Prelude module for convenient imports.
pub mod prelude {
    pub use super::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveType};
    pub use super::events::SequencerEvent;
    pub use super::ids::{
        NoteId, PatternId, RowCount, RowIndex, SeqInstrumentId, TicksPerRow, TrackCount, TrackId,
        TrackIndex,
    };
    pub use super::input::{InputCommand, InputSource};
    pub use super::note::Note;
    pub use super::pattern::{Pattern, RowResolution};
    pub use super::pitch::{NoteName, Pitch, Velocity};
    pub use super::song::Song;
    pub use super::time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
    pub use super::track::SequencerTrack;
}
