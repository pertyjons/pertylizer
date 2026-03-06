//! RMS-driven point light — intensity tracks the audio level.

use bevy::prelude::*;

use super::RmsLight;
use super::theme::ThemeRuntime;
use crate::telemetry::SynthTelemetry;

/// Update point light intensity from RMS, flux, velocity, and peak telemetry.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    runtime: Res<ThemeRuntime>,
    mut query: Query<&mut PointLight, With<RmsLight>>,
) {
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;
    let intensity_multiplier = runtime.key_light_intensity;

    // Flux adds a transient brightness spike on spectral changes
    let flux_boost = 1.0 + telemetry.flux.clamp(0.0, 2.0) * 0.5;

    // High-velocity notes flash the light briefly
    let velocity_flash = if telemetry.note_age_frames < 4 {
        if let Some(note) = telemetry.last_note_on {
            let vel_norm = note.velocity as f32 / 127.0;
            let age_decay = 1.0 - (telemetry.note_age_frames as f32 / 4.0);
            vel_norm * age_decay * 0.5
        } else {
            0.0
        }
    } else {
        0.0
    };

    // Peak transient flash
    let peak_flash = if super::telemetry_color::peak_exceeds_threshold(
        telemetry.peak[0],
        telemetry.peak[1],
        0.8,
    ) {
        0.3
    } else {
        0.0
    };

    for mut light in &mut query {
        light.intensity = rms_mono * intensity_multiplier * flux_boost
            + (velocity_flash + peak_flash) * intensity_multiplier;
    }
}
