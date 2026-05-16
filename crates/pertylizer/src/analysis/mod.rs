//! Cross-cutting analysis utilities that derive higher-level facts about a
//! `Song`/engine state pair. Pure (no audio render, no lock holding) so they
//! can run from any thread.

pub mod bass_drum_lock;
pub mod drum_groove;
pub mod harmonic_function;
pub mod instrument_profile;
pub mod pattern_analysis;
pub(crate) mod repetition;

pub use bass_drum_lock::{
    BassDrumAlignment, BassDrumLockAnalysis, BassOnset, BassPitchStability, KickOnset,
};
pub use drum_groove::{
    DrumBackbeat, DrumComponent, DrumComposition, DrumFills, DrumGhostNotes, DrumGrooveAnalysis,
    DrumHat, DrumNote, DrumRepetition,
};
pub use harmonic_function::{
    CadenceEvent, CadenceKind, ChordFunction, ChordInput, FunctionDistribution, HarmonicFunction,
    HarmonicFunctionAnalysis, KeyMode, TensionStats,
};
pub use instrument_profile::{
    InstrumentProfile, ProfileSignal, Role, RoleInference, SignalAxis, infer_all_profiles,
};
pub use pattern_analysis::{
    DensityStats, PatternAnalysis, PitchStats, RepetitionStats, RhythmStats, VelocityStats,
};
