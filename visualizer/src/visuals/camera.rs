//! Orbital camera synced to tempo — rotates faster at higher BPM.

use bevy::prelude::*;

use crate::telemetry::SynthTelemetry;

/// Base orbit speed at 120 BPM (radians/sec).
const BASE_SPEED: f32 = 0.1;

/// Reference tempo for base speed.
const REFERENCE_BPM: f32 = 120.0;

/// Orbital camera state.
#[derive(Component)]
pub struct OrbitCamera {
    pub radius: f32,
    pub angle: f32,
}

/// Rotate the camera around the Y axis, speed scaled by tempo.
pub fn orbit(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    mut query: Query<(&mut Transform, &mut OrbitCamera)>,
) {
    let tempo_scale = if telemetry.tempo > 0.0 {
        telemetry.tempo / REFERENCE_BPM
    } else {
        1.0
    };
    let speed = BASE_SPEED * tempo_scale;

    for (mut transform, mut cam) in &mut query {
        cam.angle += speed * time.delta_secs();

        let x = cam.angle.cos() * cam.radius;
        let z = cam.angle.sin() * cam.radius;

        transform.translation = Vec3::new(x, 12.0, z);
        *transform = transform.looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y);
    }
}
