//! Cross-cutting analysis utilities that derive higher-level facts about a
//! `Song`/engine state pair. Pure (no audio render, no lock holding) so they
//! can run from any thread.

pub mod instrument_profile;

pub use instrument_profile::{
    InstrumentProfile, ProfileSignal, Role, RoleInference, SignalAxis, infer_all_profiles,
};
