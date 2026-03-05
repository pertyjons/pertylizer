//! Waveform ring — FFT bands arranged in a circle.
//!
//! 128 thin bars positioned on a ring, with height driven by FFT magnitude.
//! Creates a pulsing circular spectrum visualizer.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use crate::telemetry::{NUM_FFT_BANDS, SynthTelemetry};

/// Marker with FFT band index.
#[derive(Component)]
pub struct RingBar(pub usize);

/// Ring radius.
const RING_RADIUS: f32 = 10.0;

/// Maximum bar height.
const MAX_HEIGHT: f32 = 8.0;

/// Bar thickness.
const BAR_THICKNESS: f32 = 0.15;

/// Emissive intensity multiplier.
const EMISSIVE_STRENGTH: f32 = 6.0;

/// Spawn 128 bars arranged in a circle (initially hidden).
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(BAR_THICKNESS, 1.0, BAR_THICKNESS));

    for i in 0..NUM_FFT_BANDS {
        let angle = (i as f32 / NUM_FFT_BANDS as f32) * std::f32::consts::TAU;
        let x = angle.cos() * RING_RADIUS;
        let z = angle.sin() * RING_RADIUS;

        let hue = (i as f32 / NUM_FFT_BANDS as f32) * 360.0;
        let color = Color::hsl(hue, 0.85, 0.5);

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: color,
                emissive: LinearRgba::from(color) * EMISSIVE_STRENGTH,
                ..default()
            })),
            Transform::from_xyz(x, 0.0, z)
                .with_scale(Vec3::new(1.0, 0.01, 1.0))
                .with_rotation(Quat::from_rotation_y(-angle)),
            Visibility::Hidden,
            RingBar(i),
            EffectLayer(EffectId::WaveformRing),
        ));
    }
}

/// Update ring bar heights and emissive from FFT.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&mut Transform, &RingBar, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !effect_state.active.is_active(EffectId::WaveformRing) {
        return;
    }

    let fade = effect_state.fade;

    for (mut transform, bar, material_handle) in &mut query {
        let target_height = (telemetry.fft[bar.0] * MAX_HEIGHT).max(0.01);

        // Smooth lerp
        let current = transform.scale.y;
        let speed = if target_height > current { 0.4 } else { 0.1 };
        let new_height = current + (target_height - current) * speed;

        transform.scale.y = new_height;
        // Keep bar base on the ground
        transform.translation.y = new_height / 2.0;

        // Update emissive only during crossfade
        if fade < 1.0
            && let Some(material) = materials.get_mut(&material_handle.0)
        {
            let hue = (bar.0 as f32 / NUM_FFT_BANDS as f32) * 360.0;
            let color = Color::hsl(hue, 0.85, 0.5 * fade);
            material.emissive = LinearRgba::from(color) * EMISSIVE_STRENGTH * fade;
        }
    }
}
