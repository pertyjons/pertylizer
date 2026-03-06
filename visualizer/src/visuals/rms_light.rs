//! RMS-driven point light — intensity tracks the audio level.

use bevy::prelude::*;

use super::RmsLight;
use super::theme::{ThemeRegistry, ThemeState};
use crate::telemetry::SynthTelemetry;

/// Update point light intensity from RMS telemetry.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    theme_state: Res<ThemeState>,
    theme_registry: Res<ThemeRegistry>,
    mut query: Query<&mut PointLight, With<RmsLight>>,
) {
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;
    let intensity_multiplier = theme_registry.get(theme_state.active).key_light_intensity;

    for mut light in &mut query {
        // Scale RMS to Bevy lumens using the active theme's intensity multiplier
        light.intensity = rms_mono * intensity_multiplier;
    }
}
