//! Reusable UI widgets for the synthesizer GUI.
//!
//! This module provides custom widgets like knobs, sliders, meters,
//! and module frames that can be used across different module panels.

mod cable;
mod controls;
mod envelope;
mod frame;
mod knob;
mod meter;
mod param_grid;
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
    CaptionTone, DialogButton, ICON_BUTTON_SIZE, InlineEdit, bypass_toggle, caption,
    clickable_label, count_drag_value, danger_button, dialog_button_row, dim_label, empty_state,
    enum_combo, expose, expose_selected, icon_button, inline_editable_text, knob_normalized,
    labeled_row, log_suffix_slider, modal_window, mute_toggle, paint_marker_corners,
    right_aligned_row, section_header, seed_reroll, solo_toggle, stepper, strong_label,
    suffix_slider, time_drag_value, toggle_button, toggle_button_colored, tree_picker_button,
    unit_drag_value,
};
pub use envelope::{EnvelopeChanges, EnvelopeEditor, draw_adsr_curve};
pub use frame::{ModuleFrame, blend_rgb, draw_module_footer, draw_module_header};
pub use knob::Knob;
pub use meter::{Meter, draw_level_meter, draw_stereo_meter, level_color};
pub use param_grid::{ModMarker, ModMarkers, ParamChange, draw_knobs, draw_parameter_grid};
pub use port::{PortWidget, WidgetPortDirection, WidgetPortType};
pub use scope::{draw_oscilloscope, draw_oscilloscope_with_trigger};
pub use spectrum::draw_spectrum_analyzer;
pub use tooltip::{draw_tooltip_above, draw_tooltip_right_of, draw_value_tooltip};
pub(crate) use waveform::waveform_button;
pub use waveform::{WaveformSelector, WaveformType};
