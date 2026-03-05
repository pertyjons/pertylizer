//! Waveform ring — FFT bands arranged in a circle.
//!
//! 128 thin bars positioned on a ring, with height driven by FFT magnitude.
//! Uses shared materials per hue-group to allow Bevy batching.

use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
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

/// Number of shared material buckets.
const NUM_MATERIAL_BUCKETS: usize = 16;

const MAT_CONFIG: effects::HueMaterialConfig = effects::HueMaterialConfig {
    hue_range: 360.0,
    saturation: 0.85,
    lightness: 0.5,
    emissive_strength: EMISSIVE_STRENGTH,
};

/// Shared materials per hue bucket.
#[derive(Resource)]
pub struct RingBarMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

/// Spawn 128 bars arranged in a circle (initially hidden).
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(BAR_THICKNESS, 1.0, BAR_THICKNESS));

    let shared_mats =
        effects::create_hue_materials(&mut materials, NUM_MATERIAL_BUCKETS, &MAT_CONFIG);
    commands.insert_resource(RingBarMaterials {
        materials: shared_mats.clone(),
    });

    for i in 0..NUM_FFT_BANDS {
        let angle = (i as f32 / NUM_FFT_BANDS as f32) * std::f32::consts::TAU;
        let x = angle.cos() * RING_RADIUS;
        let z = angle.sin() * RING_RADIUS;

        let bucket = (i * NUM_MATERIAL_BUCKETS / NUM_FFT_BANDS).min(NUM_MATERIAL_BUCKETS - 1);

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(shared_mats[bucket].clone()),
            Transform::from_xyz(x, 0.0, z)
                .with_scale(Vec3::new(1.0, 0.01, 1.0))
                .with_rotation(Quat::from_rotation_y(-angle)),
            Visibility::Hidden,
            RingBar(i),
            EffectLayer(EffectId::WaveformRing),
        ));
    }
}

/// Update ring bar heights from FFT.
pub fn update(
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&mut Transform, &RingBar)>,
    ring_materials: Res<RingBarMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_fade: Local<f32>,
) {
    if !effect_state.active.is_active(EffectId::WaveformRing) {
        return;
    }

    for (mut transform, bar) in &mut query {
        let target_height = (telemetry.fft[bar.0] * MAX_HEIGHT).max(0.01);

        // Smooth lerp
        let current = transform.scale.y;
        let speed = if target_height > current { 0.4 } else { 0.1 };
        let new_height = current + (target_height - current) * speed;

        transform.scale.y = new_height;
        // Keep bar base on the ground
        transform.translation.y = new_height / 2.0;
    }

    effects::update_hue_materials_for_fade(
        &mut materials,
        &ring_materials.materials,
        &MAT_CONFIG,
        effect_state.fade,
        &mut last_fade,
    );
}
