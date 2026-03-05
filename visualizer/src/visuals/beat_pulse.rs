//! Beat-synced pulse — ground plane and ambient light pulse on each beat.

use bevy::prelude::*;

use crate::telemetry::SynthTelemetry;

/// Base ambient brightness — must match the value in `setup_scene`.
pub const BASE_AMBIENT_BRIGHTNESS: f32 = 50.0;

/// Decay rate in units per second (higher = faster fade).
const DECAY_RATE: f32 = 8.0;

/// Intensity below which we skip visual updates.
const INTENSITY_EPSILON: f32 = 0.001;

/// Marker for the ground plane that pulses on beats.
#[derive(Component)]
pub struct BeatPulseGround;

/// Tracks beat pulse state.
#[derive(Resource)]
pub struct BeatPulseState {
    /// Previous beat position (to detect beat crossings).
    prev_beat: f32,
    /// Current pulse intensity (decays over time).
    intensity: f32,
}

impl Default for BeatPulseState {
    fn default() -> Self {
        Self {
            prev_beat: 0.0,
            intensity: 0.0,
        }
    }
}

/// Detect beat crossings and update pulse intensity.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    time: Res<Time>,
    mut state: ResMut<BeatPulseState>,
    mut query: Query<&MeshMaterial3d<StandardMaterial>, With<BeatPulseGround>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ambient: Query<&mut AmbientLight>,
) {
    let dt = time.delta_secs();

    if !telemetry.playing {
        state.prev_beat = telemetry.beat_position;
    } else {
        // Detect beat crossing via integer part change
        let current_beat = telemetry.beat_position.floor() as i32;
        let prev_beat = state.prev_beat.floor() as i32;

        if current_beat != prev_beat {
            // Stronger pulse on downbeats (beat 0 of each bar, assuming 4/4)
            let beat_in_bar = current_beat.rem_euclid(4);
            state.intensity = if beat_in_bar == 0 { 1.0 } else { 0.6 };
        }

        state.prev_beat = telemetry.beat_position;
    }

    // Frame-rate-independent exponential decay
    state.intensity *= (-DECAY_RATE * dt).exp();

    // Skip visual updates when intensity has decayed to zero
    if state.intensity < INTENSITY_EPSILON {
        state.intensity = 0.0;
        return;
    }

    // Update ground plane emissive
    for material_handle in &mut query {
        if let Some(material) = materials.get_mut(&material_handle.0) {
            let glow = state.intensity * 0.3;
            material.emissive = Color::srgb(glow * 0.3, glow * 0.4, glow * 1.0).into();
        }
    }

    // Pulse ambient light brightness
    for mut ambient_light in &mut ambient {
        let pulse_boost = state.intensity * 80.0;
        ambient_light.brightness = BASE_AMBIENT_BRIGHTNESS + pulse_boost;
    }
}
