//! Spectral Origami — Folded planes open with harmonics.
//!
//! A grid of triangles/planes that rotate (fold/unfold) based on Spectral Centroid and FFT.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const GRID_SIZE: usize = 12;
const SPACING: f32 = 2.5;
const EMISSIVE_STRENGTH: f32 = 2.5;
const NUM_MATERIAL_BUCKETS: usize = 32;
const MAT_CONFIG: effects::HueMaterialConfig = effects::HueMaterialConfig {
    hue_range: 360.0,
    saturation: 0.8,
    lightness: 0.5,
    emissive_strength: EMISSIVE_STRENGTH,
    frequency_mapped: false,
};

/// Per-second smoothing rate for fold angles (frame-rate independent).
const ANGLE_SMOOTH_RATE: f32 = 8.0;
/// Per-second smoothing rate for vertical position.
const Y_SMOOTH_RATE: f32 = 8.0;

#[derive(Component)]
pub struct OrigamiFold {
    pub base_pos: Vec3,
    pub index: usize,
}

#[derive(Resource)]
pub struct OrigamiMaterials(Vec<Handle<StandardMaterial>>);

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    // A simple plane that we can rotate to look like it's folding
    let mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(SPACING * 0.45)));

    let material_handles =
        effects::create_hue_materials(&mut materials, NUM_MATERIAL_BUCKETS, &MAT_CONFIG, &policy);

    // Double sided so we see it when it folds over
    for handle in &material_handles {
        if let Some(mat) = materials.get_mut(handle) {
            mat.double_sided = true;
        }
    }

    commands.insert_resource(OrigamiMaterials(material_handles.clone()));

    let offset = (GRID_SIZE as f32 * SPACING) / 2.0;

    let mut i = 0;
    for x in 0..GRID_SIZE {
        for z in 0..GRID_SIZE {
            let px = x as f32 * SPACING - offset;
            let pz = z as f32 * SPACING - offset;
            let pos = Vec3::new(px, 0.0, pz);
            let mat_idx = i % NUM_MATERIAL_BUCKETS;

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material_handles[mat_idx].clone()),
                Transform::from_translation(pos),
                Visibility::Hidden,
                OrigamiFold {
                    base_pos: pos,
                    index: i,
                },
                EffectLayer(EffectId::SpectralOrigami),
            ));
            i += 1;
        }
    }
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(&OrigamiFold, &mut Transform)>,
    mut smoothed_values: Local<Vec<[f32; 3]>>,
) {
    // Transport stopped → slow down flutter
    let time_scale = if telemetry.playing { 1.0 } else { 0.15 };
    let t = time.elapsed_secs() * time_scale;
    let dt = time.delta_secs();
    let fade = effect_state.fade;
    let angle_alpha = 1.0 - (-ANGLE_SMOOTH_RATE * dt).exp();
    let y_alpha = 1.0 - (-Y_SMOOTH_RATE * dt).exp();

    // Use centroid to drive how "open" the folds are globally
    let centroid_norm = (telemetry.centroid_hz / 5000.0).clamp(0.0, 1.0);

    // Overall energy to drive chaotic fluttering
    let energy = ((telemetry.rms[0] + telemetry.rms[1]) * 0.5 * 5.0).clamp(0.0, 2.0);

    // Ensure smoothed storage is large enough
    let total = GRID_SIZE * GRID_SIZE;
    if smoothed_values.len() < total {
        smoothed_values.resize(total, [0.0; 3]);
    }

    let bin_count = telemetry.fft_bin_count.max(1);

    for (fold, mut transform) in &mut query {
        // Tie to a specific FFT band (looping through available bands)
        let band_idx = fold.index % bin_count;
        let mag = telemetry.fft[band_idx];

        // Pitch bend adds extra fold angle
        let bend_fold = telemetry.pitch_bend * 1.5;

        // Base fold angle driven by centroid + individual band magnitude + pitch bend
        let target_angle_x = (mag * 3.0 + centroid_norm * 2.0 + bend_fold) * fade;
        let target_angle_z = (mag * 2.0 + (fold.index as f32 * 0.1 + t).sin() * energy) * fade;
        let target_y = fold.base_pos.y + (mag * 2.0 * fade);

        // Smooth angles and Y position with exponential interpolation
        if let Some(stored) = smoothed_values.get_mut(fold.index) {
            stored[0] += (target_angle_x - stored[0]) * angle_alpha;
            stored[1] += (target_angle_z - stored[1]) * angle_alpha;
            stored[2] += (target_y - stored[2]) * y_alpha;

            // Apply smoothed rotation
            transform.rotation =
                Quat::from_euler(EulerRot::XYZ, stored[0], 0.0, stored[1]);
            // Apply smoothed Y position
            transform.translation.y = stored[2];
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_material(
    time: Res<Time>,
    effect_state: Res<EffectState>,
    telemetry: Res<SynthTelemetry>,
    origami_materials: Res<OrigamiMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_hue: Local<f32>,
    mut frame_counter: Local<u32>,
    mut last_policy_version: Local<u64>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let fade = effect_state.fade;

    // Base hue from centroid theme mapping, with time-based drift (frame-rate independent)
    let base_hue = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    *last_hue = (base_hue + time.elapsed_secs() * 15.0) % 360.0;

    // Only update material every 3rd frame to reduce GPU re-uploads,
    // but always update during crossfades or policy changes
    *frame_counter = frame_counter.wrapping_add(1);
    let policy_changed = *last_policy_version != policy.version;
    if (*frame_counter).is_multiple_of(3) || fade < 1.0 || policy_changed {
        // proceed with update
    } else {
        return;
    }
    *last_policy_version = policy.version;

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let flux_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier * flux_boost;
    let metallic = policy.metallic;
    let roughness = policy.roughness;

    for (i, handle) in origami_materials.0.iter().enumerate() {
        if let Some(material) = materials.get_mut(handle) {
            // Vary hue slightly per material bucket to give each tile a distinct look
            let tile_hue = (*last_hue + (i as f32 * 11.0)) % 360.0;
            let color = Color::hsl(tile_hue, sat, lit * fade);
            material.base_color = color;
            material.emissive = LinearRgba::from(color) * emissive * fade;
            material.metallic = metallic;
            material.perceptual_roughness = roughness;
        }
    }
}
