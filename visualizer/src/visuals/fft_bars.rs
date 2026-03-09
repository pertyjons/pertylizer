//! FFT bar visualization — cubes driven by frequency band magnitudes.
//!
//! Spawns up to `MAX_FFT_BANDS` bars; only the first `fft_bin_count` are visible.
//! Uses shared materials per hue-group to allow Bevy batching.

use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::{MAX_FFT_BANDS, SynthTelemetry};

/// Marker component with the FFT band index.
#[derive(Component)]
pub struct FftBar(pub usize);

/// Total width of the bar array.
const TOTAL_WIDTH: f32 = 30.0;

/// Bar width at maximum bin count (used for mesh sizing).
const MIN_BAR_WIDTH: f32 = TOTAL_WIDTH / MAX_FFT_BANDS as f32;

/// Maximum bar height.
const MAX_HEIGHT: f32 = 8.0;

/// Emissive intensity multiplier for bloom visibility.
const EMISSIVE_STRENGTH: f32 = 5.0;

/// Per-second decay smoothing rate (frame-rate independent). Attack is instant.
const DECAY_RATE: f32 = 6.0;

/// Number of shared material buckets (bars grouped by hue).
const NUM_MATERIAL_BUCKETS: usize = 16;

const MAT_CONFIG: effects::HueMaterialConfig = effects::HueMaterialConfig {
    hue_range: 270.0,
    saturation: 0.8,
    lightness: 0.5,
    emissive_strength: EMISSIVE_STRENGTH,
    frequency_mapped: true,
};

/// Shared materials per hue bucket.
#[derive(Resource)]
pub struct FftBarMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

/// Spawn `MAX_FFT_BANDS` cubes spread along the X axis.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Cuboid::new(MIN_BAR_WIDTH * 0.85, 1.0, MIN_BAR_WIDTH * 0.85));

    let shared_mats =
        effects::create_hue_materials(&mut materials, NUM_MATERIAL_BUCKETS, &MAT_CONFIG, &policy);
    commands.insert_resource(FftBarMaterials {
        materials: shared_mats.clone(),
    });

    for i in 0..MAX_FFT_BANDS {
        let bucket = (i * NUM_MATERIAL_BUCKETS / MAX_FFT_BANDS).min(NUM_MATERIAL_BUCKETS - 1);

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(shared_mats[bucket].clone()),
            Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::new(1.0, 0.01, 1.0)),
            FftBar(i),
            Visibility::Inherited,
            EffectLayer(EffectId::FftBars),
        ));
    }
}

/// Update bar heights and positions from FFT telemetry with smooth lerp.
#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&mut Transform, &mut Visibility, &FftBar)>,
    fft_materials: Res<FftBarMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut tracker: Local<effects::HueMaterialTracker>,
) {
    let bin_count = telemetry.fft_bin_count;
    let bar_width = TOTAL_WIDTH / bin_count.max(1) as f32;
    let dt = time.delta_secs();

    // Pre-compute values once instead of per-bar
    let beat_pulse = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy);
    let height_scale = MAX_HEIGHT * (1.0 + beat_pulse * 0.15);
    let width_scale = bar_width / MIN_BAR_WIDTH;
    let decay_alpha = 1.0 - (-DECAY_RATE * dt).exp();

    for (mut transform, mut vis, bar) in &mut query {
        if bar.0 >= bin_count {
            *vis = Visibility::Hidden;
            continue;
        }
        *vis = Visibility::Inherited;

        // Reposition based on active bin count
        transform.translation.x = (bar.0 as f32 - bin_count as f32 / 2.0) * bar_width;
        transform.scale.x = width_scale;
        transform.scale.z = width_scale;

        let target_height = (telemetry.fft[bar.0] * height_scale).max(0.01);

        // Instant attack, smooth decay
        let current = transform.scale.y;
        let new_height = if target_height > current {
            target_height
        } else {
            current + (target_height - current) * decay_alpha
        };

        transform.scale.y = new_height;
        transform.translation.y = new_height / 2.0;
    }

    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let emissive_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    effects::update_hue_materials_for_fade(
        &mut materials,
        &fft_materials.materials,
        &MAT_CONFIG,
        &policy,
        effect_state.fade,
        hue_offset,
        emissive_boost,
        &mut tracker,
    );
}
