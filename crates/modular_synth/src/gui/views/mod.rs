//! GUI view components and drawing utilities.
//!
//! This module contains reusable drawing functions and UI state types
//! for various synthesizer views.
//!
//! # Main Views
//!
//! - `rack` - Instrument rack and patch editor (default view)
//! - `sequencer` - Pattern/song sequencer
//! - `mixer` - Mixer with channel strips
//!
//! # Utilities
//!
//! - `layout` - Top bar and drawer components
//! - `meters` - VU meters and level displays
//! - `master_effects` - Master effect chain UI

pub mod layout;
pub mod master_effects;
pub mod meters;
pub mod mixer;
pub mod rack;
pub mod sequencer;

pub use master_effects::{MasterEffectParams, MasterEffectUiState};
pub use meters::{draw_meter, draw_meter_horizontal};
