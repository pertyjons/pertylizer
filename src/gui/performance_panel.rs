//! Performance controls panel.
//!
//! Provides real-time performance controls for:
//! - Pitch Bend (spring-loaded, returns to center)
//! - Mod Wheel (stays where you leave it)
//! - Velocity sensitivity mapping

use eframe::egui::{self, Color32, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::engine::{EngineCommand, EngineHandle};
use crate::engine::commands::PartParam;
use crate::engine::part::{MidiChannel, PartId};
use crate::types::{BipolarValue, NormalizedValue};
use crate::gui::widgets::{colors, module_frame, Knob};

/// State for the performance panel.
#[derive(Debug, Clone)]
pub struct PerformanceState {
    /// Current pitch bend value (-1.0 to 1.0).
    pub pitch_bend: f32,
    /// Current mod wheel value (0.0 to 1.0).
    pub mod_wheel: f32,
    /// Velocity to amplitude sensitivity (0.0 to 1.0).
    pub velocity_amp_sens: f32,
    /// Velocity to filter sensitivity (0.0 to 1.0).
    pub velocity_filter_sens: f32,
}

impl Default for PerformanceState {
    fn default() -> Self {
        Self {
            pitch_bend: 0.0,
            mod_wheel: 0.0,
            velocity_amp_sens: 1.0,
            velocity_filter_sens: 0.5,
        }
    }
}

/// Draw the performance panel.
pub fn show(ui: &mut Ui, state: &mut PerformanceState, handle: &mut EngineHandle) {
    module_frame(ui, "Performance", colors::ACCENT_PURPLE, |ui| {
        ui.vertical(|ui| {
            draw_macro_controllers(ui, state, handle);
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);
            draw_velocity_mapping(ui, state, handle);
        });
    });
}

/// Draw pitch bend and mod wheel controls.
fn draw_macro_controllers(ui: &mut Ui, state: &mut PerformanceState, handle: &mut EngineHandle) {
    ui.label(egui::RichText::new("Macro Controllers").color(colors::TEXT_SECONDARY).small());
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        // Pitch Bend wheel
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("Pitch").color(colors::TEXT_DIM).small());
            let response = vertical_slider(ui, &mut state.pitch_bend, -1.0..=1.0, colors::ACCENT_CYAN);

            // Spring back to center when released
            if response.drag_stopped() {
                state.pitch_bend = 0.0;
            }

            // Send pitch bend on any change
            if response.changed() || response.drag_stopped() {
                handle.send(EngineCommand::PitchBend {
                    value: BipolarValue::new(state.pitch_bend),
                    channel: MidiChannel::OMNI,
                });
            }

            // Show current value
            let display = if state.pitch_bend.abs() < 0.01 {
                "0".to_string()
            } else {
                format!("{:+.0}", state.pitch_bend * 100.0)
            };
            ui.label(egui::RichText::new(display).color(colors::TEXT_DIM).small());
        });

        ui.add_space(8.0);

        // Mod Wheel
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("Mod").color(colors::TEXT_DIM).small());
            let response = vertical_slider(ui, &mut state.mod_wheel, 0.0..=1.0, colors::ACCENT_ORANGE);

            // Send mod wheel on change (stays where you leave it)
            if response.changed() {
                handle.send(EngineCommand::ModWheel {
                    value: NormalizedValue::new(state.mod_wheel),
                    channel: MidiChannel::OMNI,
                });
            }

            // Show current value
            ui.label(egui::RichText::new(format!("{:.0}%", state.mod_wheel * 100.0))
                .color(colors::TEXT_DIM).small());
        });
    });
}

/// Draw velocity sensitivity knobs.
fn draw_velocity_mapping(ui: &mut Ui, state: &mut PerformanceState, handle: &mut EngineHandle) {
    ui.label(egui::RichText::new("Velocity Mapping").color(colors::TEXT_SECONDARY).small());
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        // Amp sensitivity knob
        ui.vertical(|ui| {
            let response = Knob::new(&mut state.velocity_amp_sens, 0.0, 1.0)
                .label("Amp")
                .size(48.0)
                .default(1.0)
                .accent_color(colors::ACCENT_GREEN)
                .show(ui);

            if response.changed() {
                // Send to first part (applies to all voices in that part)
                handle.send(EngineCommand::SetPartParameter {
                    part_id: PartId::FIRST,
                    param: PartParam::VelocityAmpSensitivity(
                        NormalizedValue::new(state.velocity_amp_sens)
                    ),
                });
            }
        });

        ui.add_space(4.0);

        // Filter sensitivity knob
        ui.vertical(|ui| {
            let response = Knob::new(&mut state.velocity_filter_sens, 0.0, 1.0)
                .label("Filter")
                .size(48.0)
                .default(0.5)
                .accent_color(colors::ACCENT_YELLOW)
                .show(ui);

            if response.changed() {
                // Send to first part (applies to all voices in that part)
                handle.send(EngineCommand::SetPartParameter {
                    part_id: PartId::FIRST,
                    param: PartParam::VelocityFilterSensitivity(
                        NormalizedValue::new(state.velocity_filter_sens)
                    ),
                });
            }
        });
    });
}

/// Draw a vertical slider styled like a pitch wheel.
fn vertical_slider(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    accent_color: Color32,
) -> Response {
    let desired_size = Vec2::new(32.0, 100.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click_and_drag());

    let min = *range.start();
    let max = *range.end();

    // Handle dragging
    if response.dragged() {
        let delta = -response.drag_delta().y;
        let sensitivity = 0.01;
        *value = (*value + delta * sensitivity).clamp(min, max);
    }

    // Draw the slider
    let painter = ui.painter();

    // Track background
    let track_width = 8.0;
    let track_rect = egui::Rect::from_center_size(
        rect.center(),
        Vec2::new(track_width, rect.height() - 16.0),
    );
    painter.rect_filled(track_rect, 4.0, colors::BG_WIDGET);
    painter.rect_stroke(track_rect, 4.0, Stroke::new(1.0, colors::BG_DARK), StrokeKind::Outside);

    // Calculate thumb position
    let normalized = (*value - min) / (max - min);
    let thumb_y = track_rect.bottom() - normalized * track_rect.height();

    // Draw center line for bipolar sliders
    if min < 0.0 {
        let center_y = track_rect.center().y;
        painter.line_segment(
            [
                egui::pos2(track_rect.left(), center_y),
                egui::pos2(track_rect.right(), center_y),
            ],
            Stroke::new(1.0, colors::TEXT_DIM),
        );
    }

    // Draw value fill
    let fill_color = accent_color.gamma_multiply(0.6);
    if min < 0.0 {
        // Bipolar: fill from center
        let center_y = track_rect.center().y;
        let fill_rect = if *value >= 0.0 {
            egui::Rect::from_min_max(
                egui::pos2(track_rect.left() + 1.0, thumb_y),
                egui::pos2(track_rect.right() - 1.0, center_y),
            )
        } else {
            egui::Rect::from_min_max(
                egui::pos2(track_rect.left() + 1.0, center_y),
                egui::pos2(track_rect.right() - 1.0, thumb_y),
            )
        };
        painter.rect_filled(fill_rect, 2.0, fill_color);
    } else {
        // Unipolar: fill from bottom
        let fill_rect = egui::Rect::from_min_max(
            egui::pos2(track_rect.left() + 1.0, thumb_y),
            egui::pos2(track_rect.right() - 1.0, track_rect.bottom() - 1.0),
        );
        painter.rect_filled(fill_rect, 2.0, fill_color);
    }

    // Draw thumb
    let thumb_size = Vec2::new(24.0, 12.0);
    let thumb_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, thumb_y),
        thumb_size,
    );

    let thumb_color = if response.hovered() || response.dragged() {
        accent_color
    } else {
        colors::TEXT_SECONDARY
    };

    painter.rect_filled(thumb_rect, 3.0, thumb_color);
    painter.rect_stroke(thumb_rect, 3.0, Stroke::new(1.0, colors::BG_DARK), StrokeKind::Outside);

    // Grip lines on thumb
    let line_color = colors::BG_DARK;
    for i in [-1, 0, 1] {
        let y = thumb_rect.center().y + i as f32 * 2.5;
        painter.line_segment(
            [
                egui::pos2(thumb_rect.left() + 4.0, y),
                egui::pos2(thumb_rect.right() - 4.0, y),
            ],
            Stroke::new(1.0, line_color),
        );
    }

    response
}
