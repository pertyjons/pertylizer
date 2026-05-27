//! Acoustic World Engine (AWE) - real-time room simulation.
//!
//! AWE provides physics-based room acoustics processing that sits
//! after master effects in the audio chain. It models:
//! - Early reflections via image-source method (ISM)
//! - Late reverb via FDN with geometry-driven parameters
//! - Room modes / standing waves as comb filters
//! - Stereo spatialisation (ITD + ILD)
//! - Internal control-rate LFOs for parameter modulation

pub mod awe_engine;
pub mod early_reflections;
pub mod lfo;
pub mod params;
pub mod presets;
pub mod room;
pub mod room_modes;
pub mod spatial_voice;
pub mod spatializer;
pub mod types;

pub use awe_engine::AweEngine;
pub use params::{AweLfoTarget, AweParam, AweSnapshot, AweState};
pub use presets::{AwePreset, awe_presets};
pub use room::{Material, MaterialKind, RoomShape, RoomShapeKind};
pub use spatial_voice::{
    MAX_SPATIAL_VOICES, NotePositionMapping, SpatialContext, SpatialVoiceBank, SpatialVoiceInfo,
};
pub use types::{
    Celsius, CubicMeters, Meters, MetersPerSecond, Position3, SampleOffset, SquareMeters,
    StretchFactor,
};
