//! RMS-driven point light — intensity tracks the audio level.

use bevy::prelude::*;

use super::RmsLight;
use crate::telemetry::SynthTelemetry;

/// Update point light intensity from RMS telemetry.
pub fn update(telemetry: Res<SynthTelemetry>, mut query: Query<&mut PointLight, With<RmsLight>>) {
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;

    for mut light in &mut query {
        // Scale RMS to Bevy lumens (smooth via telemetry's own smoothing)
        light.intensity = rms_mono * 200_000.0;
    }
}
