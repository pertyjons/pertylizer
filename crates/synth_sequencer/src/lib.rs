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
pub mod note_graph;
pub mod note_processor;
pub mod pattern;
pub mod pitch;
pub mod song;
pub mod time;
pub mod track;

// Re-export commonly used types
pub use automation::{AutoInstrumentParam, GlobalParam, TrackParam};
pub use automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveType, ParamId};
pub use events::SequencerEvent;
pub use ids::{
    NoteGraphId, NoteId, NoteLane, NoteModuleId, PatternId, ReturnBusId, RowCount, RowIndex,
    SeqInstrumentId, TicksPerRow, TrackCount, TrackId, TrackIndex,
};
pub use input::{InputCommand, InputMultiplexer, InputSource, KeyboardInputSource};
pub use note::{
    Glide, GlideFrom, GlideInterp, Note, NoteExpression, Ornament, OrnamentDynamics,
    OrnamentPlacement, OrnamentSpacing, Vibrato, VibratoShape,
};
pub use note_graph::{
    EnvelopeTrigger, EuclideanGenerator, HostKey, LfoShape, MAX_NOTE_GRID_NODES,
    MAX_STEP_LFO_STEPS, MAX_VALUE_INPUTS, ModInputs, NodePosition, NoteConnection, NoteDelay,
    NoteEnvelope, NoteEventKey, NoteGraph, NoteGraphError, NoteLfo, NoteModuleConfig, NotePortType,
    NoteScopeCtx, NoteScriptTransform, ProbabilityGate, Ratchet, StepLfo,
};
pub use note_processor::{
    ArpMode, ArpOffsets, ArpRate, ArpVelocity, Arpeggiator, Chord, ExpandedNote, ExpansionBuffer,
    Humanize, MAX_ARP_HELD, MAX_ARP_OFFSETS, MAX_CHORD_INTERVALS, MAX_DELAY_REPEATS,
    MAX_EXPANSION_EVENTS_PER_TICK, MAX_LOOKBACK_DEPTH, MAX_NOTE_DELAY_TICKS, MAX_ORNAMENT_HITS,
    MAX_RATCHET_SUBDIVISIONS, NoteProcessor, PitchClass, ScaleMask, ScaleQuantize, StrumDirection,
    lookback_pool,
};
pub use pattern::{Pattern, RowResolution};
pub use pitch::{NoteName, Pitch, Velocity};
pub use song::{FreezeStats, LoopRegion, PatternPlacement, Song, TempoChange, TimeSignatureChange};
pub use time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
pub use track::{ReturnBus, ReturnSend, SequencerTrack, TrackColor, TrackMode, TrackSend};

/// Prelude module for convenient imports.
pub mod prelude {
    pub use super::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveType};
    pub use super::events::SequencerEvent;
    pub use super::ids::{
        NoteId, NoteLane, PatternId, ReturnBusId, RowCount, RowIndex, SeqInstrumentId, TicksPerRow,
        TrackCount, TrackId, TrackIndex,
    };
    pub use super::input::{InputCommand, InputSource};
    pub use super::note::Note;
    pub use super::pattern::{Pattern, RowResolution};
    pub use super::pitch::{NoteName, Pitch, Velocity};
    pub use super::song::Song;
    pub use super::time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
    pub use super::track::SequencerTrack;
}
