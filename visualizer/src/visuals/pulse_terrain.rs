//! Pulse Terrain — Landscape breathes with bass.
//!
//! A grid of 3D geometry that displaces vertically based on lower FFT frequencies and RMS.
//! Like a pulsing dance floor/terrain.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use crate::telemetry::SynthTelemetry;

const GRID_SIZE: usize = 20; // 20x20 grid
const GRID_SPACING: f32 = 1.5;
const EMISSIVE_STRENGTH: f32 = 5.0;

#[derive(Component)]
pub struct TerrainTile {
    pub x: usize,
    pub z: usize,
    pub base_y: f32,
    pub dist_from_center: f32,
}

#[derive(Resource)]
pub struct TerrainMaterial(Handle<StandardMaterial>);

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(GRID_SPACING * 0.9, 0.5, GRID_SPACING * 0.9));

    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.8, 0.4),
        emissive: LinearRgba::from(Color::srgb(0.2, 0.8, 0.4)) * EMISSIVE_STRENGTH,
        ..default()
    });

    commands.insert_resource(TerrainMaterial(material.clone()));

    let offset = (GRID_SIZE as f32 * GRID_SPACING) / 2.0;

    for x in 0..GRID_SIZE {
        for z in 0..GRID_SIZE {
            let px = x as f32 * GRID_SPACING - offset;
            let pz = z as f32 * GRID_SPACING - offset;
            let dist = (px * px + pz * pz).sqrt();

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(px, 0.0, pz),
                Visibility::Hidden,
                TerrainTile {
                    x,
                    z,
                    base_y: 0.0,
                    dist_from_center: dist,
                },
                EffectLayer(EffectId::PulseTerrain),
            ));
        }
    }
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&TerrainTile, &mut Transform)>,
) {
    let t = time.elapsed_secs();
    let fade = effect_state.fade;

    // Use lower FFT bands to drive the terrain height
    let mut bass = 0.0;
    for i in 0..10 {
        bass += telemetry.fft[i];
    }
    bass = (bass / 10.0) * 15.0;

    // Use RMS for global wave
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;

    for (tile, mut transform) in &mut query {
        // Create an expanding ripple effect using distance from center
        let ripple = ((tile.dist_from_center * 0.5) - t * 5.0).sin();

        // Add some noise based on X/Z coordinates
        let noise = ((tile.x as f32 * 0.3 + t).sin() * (tile.z as f32 * 0.3 + t).cos()) * 2.0;

        // Height is a combination of base wave, ripple driven by RMS, and global bass pump
        let target_y = tile.base_y
            + noise
            + (ripple * rms_mono * 10.0)
            + (bass * (1.0 / (1.0 + tile.dist_from_center * 0.1)));

        transform.translation.y = target_y * fade;

        // Scale down slightly when fading out
        let scale = 1.0 * fade;
        transform.scale = Vec3::new(scale, 1.0, scale);
    }
}

pub fn update_material(
    effect_state: Res<EffectState>,
    terrain_material: Res<TerrainMaterial>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_fade: Local<f32>,
) {
    let fade = effect_state.fade;

    // Only update material when fade actually changes
    if (fade - *last_fade).abs() < effects::FADE_EPSILON {
        return;
    }

    if let Some(material) = materials.get_mut(&terrain_material.0) {
        let color = Color::hsl(140.0, 0.8, 0.5 * fade);
        material.base_color = color;
        material.emissive = LinearRgba::from(color) * EMISSIVE_STRENGTH * fade;
    }
    *last_fade = fade;
}
