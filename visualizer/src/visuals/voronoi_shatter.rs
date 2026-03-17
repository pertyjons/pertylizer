//! Voronoi Shatter — Ground cells split and shatter with spectral flux.
//!
//! A grid of flat quads at ground level. High spectral flux causes them to
//! rotate, displace outward, and separate. The shatter decays over time,
//! reassembling the floor when the music calms down.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const GRID_SIZE: usize = 10;
const SPACING: f32 = 2.8;
const EMISSIVE_STRENGTH: f32 = 2.0;
const NUM_MATERIAL_BUCKETS: usize = 32;

/// Purple-ish hue-distributed materials for voronoi cells.
const MAT_CONFIG: effects::HueMaterialConfig = effects::HueMaterialConfig {
    hue_range: 120.0, // Purple spread (centered via hue_offset in setup)
    saturation: 0.75,
    lightness: 0.5,
    emissive_strength: EMISSIVE_STRENGTH,
    frequency_mapped: false,
};

/// Shatter decay rate (per second).
const SHATTER_DECAY: f32 = 3.0;

/// Flux threshold to trigger shatter.
const FLUX_THRESHOLD: f32 = 0.3;

#[derive(Component)]
pub struct VoronoiCell {
    pub base_pos: Vec3,
    pub index: usize,
    /// Direction this cell shatters toward (normalized XZ).
    pub shatter_dir: Vec3,
}

#[derive(Resource)]
pub struct VoronoiMaterials(Vec<Handle<StandardMaterial>>);

#[derive(Resource, Default)]
pub struct VoronoiState {
    /// Current shatter intensity (0 = assembled, 1 = fully shattered).
    pub shatter: f32,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(SPACING * 0.42)));

    // Purple-centered: offset by 270 so the hue range spans ~210..330
    let material_handles = effects::create_hue_materials_with_offset(
        &mut materials,
        NUM_MATERIAL_BUCKETS,
        &MAT_CONFIG,
        &policy,
        270.0,
    );

    // Voronoi cells are flat planes that tumble during shatter, so they need double-sided rendering.
    for handle in &material_handles {
        if let Some(mat) = materials.get_mut(handle) {
            mat.double_sided = true;
        }
    }

    commands.insert_resource(VoronoiMaterials(material_handles.clone()));
    commands.init_resource::<VoronoiState>();

    let offset = (GRID_SIZE as f32 * SPACING) / 2.0;
    let center = Vec3::ZERO;

    let mut i = 0;
    for x in 0..GRID_SIZE {
        for z in 0..GRID_SIZE {
            let px = x as f32 * SPACING - offset + SPACING * 0.5;
            let pz = z as f32 * SPACING - offset + SPACING * 0.5;
            let pos = Vec3::new(px, 0.05, pz);

            // Shatter direction: away from center in XZ
            let dir_xz = (pos - center).normalize_or(Vec3::X);
            let shatter_dir = Vec3::new(dir_xz.x, 0.0, dir_xz.z).normalize_or(Vec3::X);
            let mat_idx = i % NUM_MATERIAL_BUCKETS;

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material_handles[mat_idx].clone()),
                Transform::from_translation(pos),
                Visibility::Hidden,
                VoronoiCell {
                    base_pos: pos,
                    index: i,
                    shatter_dir,
                },
                EffectLayer(EffectId::VoronoiShatter),
            ));
            i += 1;
        }
    }
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    policy: Res<ThemeMaterialPolicy>,
    mut state: ResMut<VoronoiState>,
    mut query: Query<(&VoronoiCell, &mut Transform)>,
) {
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    let fade = effect_state.fade;

    // Flux triggers shatter
    if telemetry.flux > FLUX_THRESHOLD {
        state.shatter = (state.shatter + telemetry.flux * 0.8).min(1.0);
    }
    // Decay
    state.shatter *= (-SHATTER_DECAY * dt).exp();
    if state.shatter < 0.001 {
        state.shatter = 0.0;
    }

    let shatter = state.shatter;
    let beat_pulse = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy);

    for (cell, mut transform) in &mut query {
        // Displacement: outward from center, scaled by shatter
        let displacement = cell.shatter_dir * shatter * 4.0;

        // Vertical hop: cells lift off the ground during shatter
        let hop = shatter * 2.0 * (1.0 + (cell.index as f32 * 0.7 + t * 3.0).sin() * 0.5);

        // Rotation: tumble during shatter
        let rot_x = shatter * (cell.index as f32 * 0.3 + t * 2.0).sin() * 1.5;
        let rot_z = shatter * (cell.index as f32 * 0.5 + t * 1.7).cos() * 1.5;

        // Beat pulse adds subtle vertical bounce when assembled
        let pulse_y = beat_pulse * 0.3 * (1.0 - shatter);

        transform.translation =
            cell.base_pos + displacement * fade + Vec3::new(0.0, hop + pulse_y, 0.0) * fade;
        transform.rotation = Quat::from_euler(EulerRot::XYZ, rot_x * fade, 0.0, rot_z * fade);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_material(
    effect_state: Res<EffectState>,
    telemetry: Res<SynthTelemetry>,
    voronoi_materials: Res<VoronoiMaterials>,
    voronoi_state: Res<VoronoiState>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_hue: Local<f32>,
    mut last_fade: Local<f32>,
    mut last_shatter: Local<f32>,
    mut last_policy_version: Local<u64>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let fade = effect_state.fade;
    let hue = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let shatter = voronoi_state.shatter;

    if (hue - *last_hue).abs() < 1.0
        && (fade - *last_fade).abs() < super::effects::FADE_EPSILON
        && (shatter - *last_shatter).abs() < 0.01
        && *last_policy_version == policy.version
    {
        return;
    }
    *last_hue = hue;
    *last_fade = fade;
    *last_shatter = shatter;
    *last_policy_version = policy.version;

    let sat_base = (0.7 + policy.saturation_offset).clamp(0.0, 1.0);
    // Shatter makes cells brighter
    let shatter_boost = 1.0 + voronoi_state.shatter * 0.5;
    let lit_base = ((0.4 + policy.lightness_offset) * shatter_boost).clamp(0.0, 1.0);
    let flux_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier * flux_boost;

    for (i, handle) in voronoi_materials.0.iter().enumerate() {
        if let Some(material) = materials.get_mut(handle) {
            // Vary lightness per bucket
            let lightness_variation = (i as f32 / NUM_MATERIAL_BUCKETS as f32) * 0.3 - 0.15; // +/- 0.15
            let lit = (lit_base + lightness_variation).clamp(0.0, 1.0);
            
            // Vary saturation per bucket
            let sat_variation = ((i * 7) % 11) as f32 * 0.02; // deterministic pseudo-random variation
            let sat = (sat_base + sat_variation).clamp(0.0, 1.0);

            let color = Color::hsl(hue, sat, lit * fade);
            material.base_color = color;
            material.emissive = LinearRgba::from(color) * emissive * fade;
            material.metallic = policy.metallic;
            material.perceptual_roughness = policy.roughness;
        }
    }
}
