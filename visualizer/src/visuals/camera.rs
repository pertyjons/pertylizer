//! Simple orbital camera that slowly rotates around the scene.

use bevy::prelude::*;

/// Orbital camera state.
#[derive(Component)]
pub struct OrbitCamera {
    pub radius: f32,
    pub speed: f32,
    pub angle: f32,
}

/// Rotate the camera around the Y axis.
pub fn orbit(time: Res<Time>, mut query: Query<(&mut Transform, &mut OrbitCamera)>) {
    for (mut transform, mut cam) in &mut query {
        cam.angle += cam.speed * time.delta_secs();

        let x = cam.angle.cos() * cam.radius;
        let z = cam.angle.sin() * cam.radius;

        transform.translation = Vec3::new(x, 12.0, z);
        *transform = transform.looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y);
    }
}
