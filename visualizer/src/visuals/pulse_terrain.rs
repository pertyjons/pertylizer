//! Pulse Terrain — Landscape breathes with bass.
//!
//! A grid of 3D geometry that displaces vertically based on lower FFT frequencies and RMS.
//! Like a pulsing dance floor/terrain. Input signals are smoothed with asymmetric
//! attack/decay easing to prevent teleportation on transients.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const GRID_SIZE: usize = 20; // 20x20 grid
const GRID_SPACING: f32 = 1.5;
const EMISSIVE_STRENGTH: f32 = 2.0;

/// Per-second terrain height smoothing rate. Responsive but smooth.
const TERRAIN_SMOOTH_RATE: f32 = 10.0;

/// Input signal smoothing — attack (signal rising).
const SIGNAL_ATTACK_RATE: f32 = 20.0;
/// Input signal smoothing — decay (signal falling).
const SIGNAL_DECAY_RATE: f32 = 5.0;

#[derive(Component)]
pub struct TerrainTile {
    pub x: usize,
    pub z: usize,
    pub base_y: f32,
    pub dist_from_center: f32,
}

const NUM_MATERIAL_BUCKETS: usize = 16;
const BASE_SATURATION: f32 = 0.8;

#[derive(Resource)]
pub struct TerrainMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

/// Computed per-bucket color properties for the terrain gradient.
struct BucketColor {
    color: Color,
    emissive: LinearRgba,
}

/// Compute the color for a single terrain material bucket.
///
/// `t` ranges from 0.0 (center) to 1.0 (edge). Center buckets are brighter
/// and more emissive; edge buckets are darker and dimmer.
fn bucket_color(
    t: f32,
    hue_base: f32,
    fade: f32,
    policy: &ThemeMaterialPolicy,
    flux_boost: f32,
) -> BucketColor {
    let lit = (0.7 - t * 0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let hue = (hue_base + t * 60.0) % 360.0;
    let sat = (BASE_SATURATION + policy.saturation_offset).clamp(0.0, 1.0);
    let color = Color::hsl(hue, sat, lit * fade);
    let emissive_strength =
        EMISSIVE_STRENGTH * (1.0 - t * 0.8) * policy.emissive_multiplier * flux_boost;
    let emissive = LinearRgba::from(color) * emissive_strength * fade;
    BucketColor { color, emissive }
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Cuboid::new(GRID_SPACING * 0.9, 0.5, GRID_SPACING * 0.9));

    // Create materials directly with per-bucket distance-based gradient.
    // Center buckets are bright; edge buckets are dark.
    let initial_hue = 140.0;
    let shared_mats: Vec<Handle<StandardMaterial>> = (0..NUM_MATERIAL_BUCKETS)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / (NUM_MATERIAL_BUCKETS - 1) as f32;
            let bc = bucket_color(t, initial_hue, 1.0, &policy, 1.0);
            materials.add(StandardMaterial {
                base_color: bc.color,
                emissive: bc.emissive,
                metallic: policy.metallic,
                perceptual_roughness: policy.roughness,
                ..default()
            })
        })
        .collect();

    commands.insert_resource(TerrainMaterials {
        materials: shared_mats.clone(),
    });

    let offset = (GRID_SIZE as f32 * GRID_SPACING) / 2.0;
    let max_dist = ((offset * offset) * 2.0).sqrt();

    for x in 0..GRID_SIZE {
        for z in 0..GRID_SIZE {
            let px = x as f32 * GRID_SPACING - offset + GRID_SPACING * 0.5;
            let pz = z as f32 * GRID_SPACING - offset + GRID_SPACING * 0.5;
            let dist = (px * px + pz * pz).sqrt();

            let bucket = ((dist / max_dist) * NUM_MATERIAL_BUCKETS as f32).floor() as usize;
            let bucket = bucket.min(NUM_MATERIAL_BUCKETS - 1);

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(shared_mats[bucket].clone()),
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
    policy: Res<ThemeMaterialPolicy>,
    mut query: Query<(&TerrainTile, &mut Transform)>,
    mut smoothed_bass: Local<f32>,
    mut smoothed_rms: Local<f32>,
) {
    // Transport stopped → slow down ripple animation
    let time_scale = if telemetry.playing { 1.0 } else { 0.15 };
    let t = time.elapsed_secs() * time_scale;
    let dt = time.delta_secs();
    let smooth_alpha = 1.0 - (-TERRAIN_SMOOTH_RATE * dt).exp();
    let fade = effect_state.fade;

    // Compute raw bass from lower FFT bands
    let mut raw_bass = 0.0;
    for i in 0..10 {
        raw_bass += telemetry.fft[i];
    }
    raw_bass = (raw_bass / 10.0) * 15.0;

    // Compute raw RMS
    let raw_rms = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;

    // Smooth bass with asymmetric easing
    let bass_rate = if raw_bass > *smoothed_bass {
        SIGNAL_ATTACK_RATE
    } else {
        SIGNAL_DECAY_RATE
    };
    *smoothed_bass += (raw_bass - *smoothed_bass) * (1.0 - (-bass_rate * dt).exp());

    // Smooth RMS with asymmetric easing
    let rms_rate = if raw_rms > *smoothed_rms {
        SIGNAL_ATTACK_RATE
    } else {
        SIGNAL_DECAY_RATE
    };
    *smoothed_rms += (raw_rms - *smoothed_rms) * (1.0 - (-rms_rate * dt).exp());

    let beat_pulse = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy);

    // Recent velocity boosts ground intensity
    let velocity_boost = if telemetry.note_age_frames < 6 {
        if let Some(note) = telemetry.last_note_on {
            let vel_norm = note.velocity as f32 / 127.0;
            let age_decay = 1.0 - (telemetry.note_age_frames as f32 / 6.0);
            vel_norm * age_decay
        } else {
            0.0
        }
    } else {
        0.0
    };

    for (tile, mut transform) in &mut query {
        // Create an expanding ripple effect using distance from center
        let ripple = ((tile.dist_from_center * 0.5) - t * 5.0).sin();

        // Add some noise based on X/Z coordinates
        let noise = ((tile.x as f32 * 0.3 + t).sin() * (tile.z as f32 * 0.3 + t).cos()) * 2.0;

        // Height uses smoothed signals instead of raw values
        let target_y = tile.base_y
            + noise
            + (ripple * (*smoothed_rms + beat_pulse * 0.3) * 10.0)
            + (*smoothed_bass
                * (1.0 + velocity_boost)
                * (1.0 / (1.0 + tile.dist_from_center * 0.1)));

        let target = target_y * fade;
        transform.translation.y += (target - transform.translation.y) * smooth_alpha;

        // Scale down slightly when fading out
        let scale = 1.0 * fade;
        transform.scale = Vec3::new(scale, 1.0, scale);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_material(
    effect_state: Res<EffectState>,
    telemetry: Res<SynthTelemetry>,
    terrain_materials: Res<TerrainMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_fade: Local<f32>,
    mut last_hue: Local<f32>,
    mut last_policy_version: Local<u64>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let fade = effect_state.fade;
    let hue_base = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);

    if (fade - *last_fade).abs() < effects::FADE_EPSILON
        && (hue_base - *last_hue).abs() < 1.0
        && *last_policy_version == policy.version
    {
        return;
    }

    let flux_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);

    for (i, handle) in terrain_materials.materials.iter().enumerate() {
        if let Some(material) = materials.get_mut(handle) {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / (NUM_MATERIAL_BUCKETS - 1) as f32;
            let bc = bucket_color(t, hue_base, fade, &policy, flux_boost);

            material.base_color = bc.color;
            material.emissive = bc.emissive;
            material.metallic = policy.metallic;
            material.perceptual_roughness = policy.roughness;
        }
    }
    
    *last_fade = fade;
    *last_hue = hue_base;
    *last_policy_version = policy.version;
}
