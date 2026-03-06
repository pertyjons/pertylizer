//! Waveform ring — FFT bands arranged in a circle.
//!
//! Up to `MAX_FFT_BANDS` thin bars positioned on a ring, with height driven by FFT magnitude.
//! Only the first `fft_bin_count` bars are visible.
//! Uses shared materials per hue-group to allow Bevy batching.

use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::{MAX_FFT_BANDS, SynthTelemetry};

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

/// Per-second smoothing rates (frame-rate independent).
const ATTACK_RATE: f32 = 30.0;
const DECAY_RATE: f32 = 6.0;

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

/// Spawn `MAX_FFT_BANDS` bars arranged in a circle (initially hidden).
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Cuboid::new(BAR_THICKNESS, 1.0, BAR_THICKNESS));

    let shared_mats =
        effects::create_hue_materials(&mut materials, NUM_MATERIAL_BUCKETS, &MAT_CONFIG, &policy);
    commands.insert_resource(RingBarMaterials {
        materials: shared_mats.clone(),
    });

    for i in 0..MAX_FFT_BANDS {
        let bucket = (i * NUM_MATERIAL_BUCKETS / MAX_FFT_BANDS).min(NUM_MATERIAL_BUCKETS - 1);

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(shared_mats[bucket].clone()),
            Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::new(1.0, 0.01, 1.0)),
            Visibility::Hidden,
            RingBar(i),
            EffectLayer(EffectId::WaveformRing),
        ));
    }
}

/// Update ring bar heights and positions from FFT.
#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&mut Transform, &mut Visibility, &RingBar)>,
    ring_materials: Res<RingBarMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut last_fade: Local<f32>,
) {
    let bin_count = telemetry.fft_bin_count;
    let is_active = effect_state.active.is_active(EffectId::WaveformRing);
    let dt = time.delta_secs();

    for (mut transform, mut vis, bar) in &mut query {
        if bar.0 >= bin_count {
            *vis = Visibility::Hidden;
            continue;
        }
        if is_active {
            *vis = Visibility::Inherited;
        }

        // Reposition on the ring based on active bin count
        let angle = (bar.0 as f32 / bin_count as f32) * std::f32::consts::TAU;
        let x = angle.cos() * RING_RADIUS;
        let z = angle.sin() * RING_RADIUS;
        transform.translation.x = x;
        transform.translation.z = z;
        transform.rotation = Quat::from_rotation_y(-angle);

        let target_height = (telemetry.fft[bar.0] * MAX_HEIGHT).max(0.01);

        // Smooth lerp (time-based)
        let current = transform.scale.y;
        let rate = if target_height > current {
            ATTACK_RATE
        } else {
            DECAY_RATE
        };
        let alpha = 1.0 - (-rate * dt).exp();
        let new_height = current + (target_height - current) * alpha;

        transform.scale.y = new_height;
        // Keep bar base on the ground
        transform.translation.y = new_height / 2.0;
    }

    effects::update_hue_materials_for_fade(
        &mut materials,
        &ring_materials.materials,
        &MAT_CONFIG,
        &policy,
        effect_state.fade,
        &mut last_fade,
    );
}
