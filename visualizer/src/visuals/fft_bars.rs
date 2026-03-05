//! FFT bar visualization — 128 cubes driven by frequency band magnitudes.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use crate::telemetry::{NUM_FFT_BANDS, SynthTelemetry};

/// Marker component with the FFT band index.
#[derive(Component)]
pub struct FftBar(pub usize);

/// Total width of the bar array.
const TOTAL_WIDTH: f32 = 30.0;

/// Spacing between bars.
const BAR_WIDTH: f32 = TOTAL_WIDTH / NUM_FFT_BANDS as f32;

/// Maximum bar height.
const MAX_HEIGHT: f32 = 8.0;

/// Emissive intensity multiplier for bloom visibility.
const EMISSIVE_STRENGTH: f32 = 5.0;

/// Spawn 128 cubes spread along the X axis.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(BAR_WIDTH * 0.85, 1.0, BAR_WIDTH * 0.85));

    for i in 0..NUM_FFT_BANDS {
        let x = (i as f32 - NUM_FFT_BANDS as f32 / 2.0) * BAR_WIDTH;

        // Color: hue gradient from low (red/warm) to high (blue/cool)
        let hue = (i as f32 / NUM_FFT_BANDS as f32) * 270.0;
        let color = Color::hsl(hue, 0.8, 0.5);

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: color,
                emissive: LinearRgba::from(color) * EMISSIVE_STRENGTH,
                ..default()
            })),
            Transform::from_xyz(x, 0.0, 0.0).with_scale(Vec3::new(1.0, 0.01, 1.0)),
            FftBar(i),
            Visibility::Inherited,
            EffectLayer(EffectId::FftBars),
        ));
    }
}

/// Update bar heights from FFT telemetry with smooth lerp.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&mut Transform, &FftBar, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !effect_state.active.is_active(EffectId::FftBars) {
        return;
    }

    let fade = effect_state.fade;

    for (mut transform, bar, material_handle) in &mut query {
        let target_height = telemetry.fft[bar.0] * MAX_HEIGHT;
        let target_height = target_height.max(0.01); // Minimum visible height

        // Smooth lerp — fast attack, slow decay
        let current = transform.scale.y;
        let speed = if target_height > current { 0.4 } else { 0.1 };
        let new_height = current + (target_height - current) * speed;

        transform.scale.y = new_height;
        // Move bar up so base stays on the ground
        transform.translation.y = new_height / 2.0;

        // Apply fade to emissive
        if fade < 1.0
            && let Some(material) = materials.get_mut(&material_handle.0)
        {
            let hue = (bar.0 as f32 / NUM_FFT_BANDS as f32) * 270.0;
            let color = Color::hsl(hue, 0.8, 0.5 * fade);
            material.emissive = LinearRgba::from(color) * EMISSIVE_STRENGTH * fade;
        }
    }
}
