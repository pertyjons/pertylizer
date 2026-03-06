//! RMS-driven point light — intensity tracks the audio level.

use bevy::prelude::*;

use super::RmsLight;
use super::theme::ThemeRuntime;
use crate::telemetry::SynthTelemetry;

/// Update point light intensity from RMS telemetry.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    runtime: Res<ThemeRuntime>,
    mut query: Query<&mut PointLight, With<RmsLight>>,
) {
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;
    let intensity_multiplier = runtime.key_light_intensity;

    for mut light in &mut query {
        // Scale RMS to Bevy lumens using the active theme's intensity multiplier
        light.intensity = rms_mono * intensity_multiplier;
    }
}
