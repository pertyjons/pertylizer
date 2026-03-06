//! Voronoi Shatter — Ground cells split and shatter with spectral flux.
//!
//! A grid of flat quads at ground level. High spectral flux causes them to
//! rotate, displace outward, and separate. The shatter decays over time,
//! reassembling the floor when the music calms down.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const GRID_SIZE: usize = 10;
const SPACING: f32 = 2.8;
const EMISSIVE_STRENGTH: f32 = 5.0;

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
pub struct VoronoiMaterial(Handle<StandardMaterial>);

#[derive(Resource, Default)]
pub struct VoronoiState {
    /// Current shatter intensity (0 = assembled, 1 = fully shattered).
    pub shatter: f32,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(SPACING * 0.42)));

    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.6, 0.3, 0.9),
        emissive: LinearRgba::from(Color::srgb(0.6, 0.3, 0.9)) * EMISSIVE_STRENGTH,
        double_sided: true,
        ..default()
    });

    commands.insert_resource(VoronoiMaterial(material.clone()));
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
            let dir_xz = (pos - center).normalize_or_else(|| Vec3::new(1.0, 0.0, 0.0));
            let shatter_dir = Vec3::new(dir_xz.x, 0.0, dir_xz.z).normalize();

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
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

        transform.translation = cell.base_pos + displacement * fade + Vec3::new(0.0, hop + pulse_y, 0.0) * fade;
        transform.rotation = Quat::from_euler(EulerRot::XYZ, rot_x * fade, 0.0, rot_z * fade);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_material(
    effect_state: Res<EffectState>,
    telemetry: Res<SynthTelemetry>,
    voronoi_material: Res<VoronoiMaterial>,
    voronoi_state: Res<VoronoiState>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_hue: Local<f32>,
    mut last_fade: Local<f32>,
    mut last_shatter: Local<f32>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let fade = effect_state.fade;
    let hue = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let shatter = voronoi_state.shatter;

    if (hue - *last_hue).abs() < 1.0
        && (fade - *last_fade).abs() < super::effects::FADE_EPSILON
        && (shatter - *last_shatter).abs() < 0.01
    {
        return;
    }
    *last_hue = hue;
    *last_fade = fade;
    *last_shatter = shatter;

    let sat = (0.7 + policy.saturation_offset).clamp(0.0, 1.0);
    // Shatter makes cells brighter
    let shatter_boost = 1.0 + voronoi_state.shatter * 0.5;
    let lit = ((0.4 + policy.lightness_offset) * shatter_boost).clamp(0.0, 1.0);
    let flux_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier * flux_boost;

    if let Some(material) = materials.get_mut(&voronoi_material.0) {
        let color = Color::hsl(hue, sat, lit * fade);
        material.base_color = color;
        material.emissive = LinearRgba::from(color) * emissive * fade;
        material.metallic = policy.metallic;
        material.perceptual_roughness = policy.roughness;
    }
}
