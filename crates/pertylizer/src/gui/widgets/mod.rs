//! Reusable UI widgets for the synthesizer GUI.
//!
//! This module provides custom widgets like knobs, sliders, meters,
//! and module frames that can be used across different module panels.

mod cable;
mod controls;
mod envelope;
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
    CABLE_SPREAD, cable_color, closest_point_on_cable, draw_cable, draw_cable_dragging,
    draw_cable_highlighted, draw_flow_particles, point_near_cable,
};
pub use controls::{
    DialogButton, dialog_button_row, icon_button, modal_window, section_header, toggle_button,
    toggle_button_colored,
};
pub use envelope::{EnvelopeChanges, EnvelopeEditor, draw_adsr_curve};
pub use knob::Knob;
pub use meter::{Meter, draw_level_meter, draw_stereo_meter, level_color};
pub use port::{PortWidget, WidgetPortDirection, WidgetPortType};
pub use scope::{draw_oscilloscope, draw_oscilloscope_with_trigger};
pub use spectrum::draw_spectrum_analyzer;
pub use tooltip::{draw_tooltip_above, draw_tooltip_right_of, draw_value_tooltip};
pub use waveform::{WaveformSelector, WaveformType};
