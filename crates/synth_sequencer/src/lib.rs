#![forbid(unsafe_code)]

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

pub mod automation;
pub mod events;
pub mod ids;
pub mod input;
pub mod mod_grid;
pub mod note;
pub mod note_graph;
pub mod note_processor;
pub mod pattern;
pub mod pitch;
pub mod shared_song;
pub mod song;
pub mod time;
pub mod track;

// Re-export commonly used types
pub use automation::{
    AutoInstrumentParam, GlobalParam, TRACK_PITCH_RANGE, TrackParam, track_pitch_normalized,
    track_pitch_semitones,
};
pub use automation::{
    AutomationLane, AutomationPoint, AutomationTarget, CurveStrength, CurveType, ParamId,
};
pub use events::SequencerEvent;
pub use ids::{
    InstrumentId, ModGraphId, ModNodeId, NoteGraphId, NoteId, NoteLane, NoteModuleId, PatternId,
    ReturnBusId, RowCount, RowIndex, SectionId, TicksPerRow, TrackCount, TrackId, TrackIndex,
};
pub use input::{InputCommand, InputMultiplexer, InputSource, KeyboardInputSource};
pub use mod_grid::{
    AudioTapNode, AudioTapSource, CombineMode, MAX_MOD_GRID_NODES, MacroNode, MidiCcNode,
    ModConnection, ModGraph, ModGraphError, ModGraphScope, ModNodeConfig, ModTarget,
    ModulationAmount, ModuleNode, TARGET_INPUT_PORT, TransportNode, TransportSource,
};
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
    MAX_RATCHET_SUBDIVISIONS, NoteProcessor, PitchClass, ScaleMask, ScaleQuantize, ScaleTieBreak,
    StrumDirection, lookback_pool,
};
pub use pattern::{HiddenEventSummary, Pattern, RowResolution};
pub use pitch::{NoteName, Pitch, Velocity};
pub use shared_song::{SharedSong, SharedSongWriteGuard};
pub use song::{
    ArrangementSection, FreezeStats, LoopRegion, PatternPlacement, PlacementLoopMode, SectionColor,
    SectionKind, Song, TempoChange, TimeSignatureChange,
};
pub use time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
pub use track::{ReturnBus, ReturnSend, SequencerTrack, TrackColor, TrackMode, TrackSend};

/// Prelude module for convenient imports.
pub mod prelude {
    pub use super::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveType};
    pub use super::events::SequencerEvent;
    pub use super::ids::{
        InstrumentId, NoteId, NoteLane, PatternId, ReturnBusId, RowCount, RowIndex, SectionId,
        TicksPerRow, TrackCount, TrackId, TrackIndex,
    };
    pub use super::input::{InputCommand, InputSource};
    pub use super::note::Note;
    pub use super::pattern::{Pattern, RowResolution};
    pub use super::pitch::{NoteName, Pitch, Velocity};
    pub use super::song::Song;
    pub use super::time::{Duration, PatternTick, TICKS_PER_QUARTER, Tick, TimeSignature};
    pub use super::track::SequencerTrack;
}
