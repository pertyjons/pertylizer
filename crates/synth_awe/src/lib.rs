//! Acoustic World Engine (AWE) - real-time room simulation.
//!
//! AWE provides physics-based room acoustics processing that sits
//! after master effects in the audio chain. It models:
//! - Room geometry and material properties
//! - Early reflections via image-source method (future)
//! - Room modes / standing waves (future)
//! - Late reverb via FDN (future)
//!
//! ## Fas 0 (current)
//! Infrastructure only — pass-through processing with all parameter
//! types and persistence in place.

pub mod awe_engine;
pub mod params;
pub mod room;

pub use awe_engine::AweEngine;
pub use params::{AweParam, AweSnapshot, AweState};
pub use room::{Material, RoomShape};
