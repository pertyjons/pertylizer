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
mod spectrum;
mod tooltip;
mod waveform;

// Re-export theme for convenience
pub use super::theme::{Theme, set_theme, theme, with_theme_mut};

// Re-export all widgets
pub use cable::{
    cable_color, draw_cable, draw_cable_dragging, draw_cable_highlighted, point_near_cable,
};
pub use envelope::{EnvelopeChanges, EnvelopeEditor, draw_adsr_curve};
pub use frame::module_frame;
pub use knob::Knob;
pub use meter::{Meter, draw_level_meter, draw_stereo_meter, level_color};
pub use port::{PortWidget, WidgetPortDirection, WidgetPortType};
pub use scope::draw_oscilloscope;
pub use spectrum::draw_spectrum_analyzer;
pub use tooltip::{draw_tooltip_above, draw_tooltip_right_of, draw_value_tooltip};
pub use waveform::{WaveformSelector, WaveformType};
