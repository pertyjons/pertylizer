//! Reusable UI widgets for the synthesizer GUI.
//!
//! This module provides custom widgets like knobs, sliders, meters,
//! and module frames that can be used across different module panels.

mod cable;
mod envelope;
mod frame;
mod knob;
mod meter;
mod port;
mod scope;
mod waveform;

// Re-export theme for convenience
pub use super::theme::{theme, with_theme_mut, set_theme, Theme};

// Re-export colors for backwards compatibility
pub use super::theme::colors;

// Re-export all widgets
pub use cable::draw_cable;
pub use envelope::{draw_adsr_curve, EnvelopeEditor};
pub use frame::module_frame;
pub use knob::Knob;
pub use meter::{draw_level_meter, draw_stereo_meter, level_color, Meter};
pub use port::{Port, PortDirection, PortType};
pub use scope::draw_oscilloscope;
pub use waveform::{WaveformSelector, WaveformType};
