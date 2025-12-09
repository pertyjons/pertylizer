//! GUI view components and drawing utilities.
//!
//! This module contains reusable drawing functions and UI state types
//! for various synthesizer views.

pub mod master_effects;
pub mod meters;

pub use master_effects::{MasterEffectParams, MasterEffectUiState};
pub use meters::{draw_meter, draw_meter_horizontal};
