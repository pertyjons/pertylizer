//! RMS-driven point light — intensity tracks the audio level.

use bevy::prelude::*;

use super::RmsLight;
use super::theme::ThemeRuntime;
use crate::telemetry::SynthTelemetry;

/// Update point light intensity from RMS and flux telemetry.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    runtime: Res<ThemeRuntime>,
    mut query: Query<&mut PointLight, With<RmsLight>>,
) {
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;
    let intensity_multiplier = runtime.key_light_intensity;

    // Flux adds a transient brightness spike on spectral changes
    let flux_boost = 1.0 + telemetry.flux.clamp(0.0, 2.0) * 0.5;

    for mut light in &mut query {
        light.intensity = rms_mono * intensity_multiplier * flux_boost;
    }
}
