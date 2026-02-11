//! GUI view components and drawing utilities.
//!
//! This module contains reusable drawing functions and UI state types
//! for various synthesizer views.
//!
//! # Main Views
//!
//! - `rack` - Instrument rack and patch editor (default view)
//!
//! # Utilities
//!
//! - `meters` - VU meters and level displays
//! - `master_effects` - Master effect chain UI

pub mod master_effects;
pub mod meters;
pub mod rack;

pub use master_effects::{MasterEffectParams, MasterEffectUiState};
pub use meters::{draw_meter, draw_meter_horizontal};
