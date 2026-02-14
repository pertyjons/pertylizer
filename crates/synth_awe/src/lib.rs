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
pub mod room;
pub mod room_modes;
pub mod spatializer;

pub use awe_engine::AweEngine;
pub use params::{AweLfoTarget, AweParam, AweSnapshot, AweState};
pub use room::{Material, RoomShape};
