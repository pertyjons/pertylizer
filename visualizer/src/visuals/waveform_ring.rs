//! Waveform ring — FFT bands arranged in a circle.
//!
//! Up to `MAX_FFT_BANDS` thin bars positioned on a ring, with height driven by FFT magnitude.
//! Only the first `fft_bin_count` bars are visible.
//! Uses shared materials per hue-group to allow Bevy batching.

use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
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
    frequency_mapped: false,
};

/// Shared materials per hue bucket.
#[derive(Resource)]
pub struct RingBarMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

/// Pre-computed ring positions and rotations per bin count, avoiding per-frame trig.
#[derive(Resource, Default)]
pub(crate) struct RingPositionCache {
    /// The bin count these positions were computed for.
    bin_count: usize,
    /// (x, z, rotation_quat) for each bar index.
    positions: Vec<(f32, f32, Quat)>,
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
    mut tracker: Local<effects::HueMaterialTracker>,
    mut pos_cache: Local<RingPositionCache>,
) {
    let bin_count = telemetry.fft_bin_count;
    let is_active = effect_state.active.is_active(EffectId::WaveformRing);
    let dt = time.delta_secs();

    // Recompute ring positions only when bin count changes
    if pos_cache.bin_count != bin_count {
        pos_cache.bin_count = bin_count;
        pos_cache.positions.clear();
        let bc = bin_count.max(1) as f32;
        for i in 0..MAX_FFT_BANDS {
            let angle = (i as f32 / bc) * std::f32::consts::TAU;
            pos_cache.positions.push((
                angle.cos() * RING_RADIUS,
                angle.sin() * RING_RADIUS,
                Quat::from_rotation_y(-angle),
            ));
        }
    }

    // Pre-compute smoothing alphas once (same for all bars)
    let attack_alpha = 1.0 - (-ATTACK_RATE * dt).exp();
    let decay_alpha = 1.0 - (-DECAY_RATE * dt).exp();

    for (mut transform, mut vis, bar) in &mut query {
        if bar.0 >= bin_count {
            *vis = Visibility::Hidden;
            continue;
        }
        if is_active {
            *vis = Visibility::Inherited;
        }

        // Use cached position/rotation (no per-frame trig)
        let (x, z, rot) = pos_cache.positions[bar.0];
        transform.translation.x = x;
        transform.translation.z = z;
        transform.rotation = rot;

        let target_height = (telemetry.fft[bar.0] * MAX_HEIGHT).max(0.01);

        // Smooth lerp with pre-computed alpha
        let current = transform.scale.y;
        let alpha = if target_height > current {
            attack_alpha
        } else {
            decay_alpha
        };
        let new_height = current + (target_height - current) * alpha;

        transform.scale.y = new_height;
        transform.translation.y = new_height / 2.0;
    }

    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let emissive_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    effects::update_hue_materials_for_fade(
        &mut materials,
        &ring_materials.materials,
        &MAT_CONFIG,
        &policy,
        effect_state.fade,
        hue_offset,
        emissive_boost,
        &mut tracker,
    );
}
