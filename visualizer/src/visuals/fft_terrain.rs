//! FFT Terrain — Landscape of columns deforming in real-time with FFT bands.
//!
//! A grid of vertical pillars whose height is driven by FFT bin magnitudes.
//! Creates a cityscape-like terrain that dances with the frequency spectrum.
//! Colors use nonlinear frequency-to-hue mapping (bass=red, mids=green, highs=blue).

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// Columns along the frequency axis.
const COLS: usize = 16;
/// Rows (depth) for visual fullness.
const ROWS: usize = 8;
/// Spacing between columns.
const COL_SPACING: f32 = 1.6;
/// Spacing between rows.
const ROW_SPACING: f32 = 2.0;
/// Maximum column height.
const MAX_HEIGHT: f32 = 12.0;
/// Base emissive strength.
const EMISSIVE_STRENGTH: f32 = 2.5;
/// Per-second decay smoothing rate (frame-rate independent). Attack is instant.
const DECAY_RATE: f32 = 6.0;

#[derive(Component)]
pub struct TerrainColumn {
    pub base_pos: Vec3,
    /// Which FFT bin this column represents.
    pub fft_bin: usize,
}

/// Shared material handles — one per frequency column.
#[derive(Resource)]
pub struct TerrainMaterials {
    pub materials: Vec<Handle<StandardMaterial>>,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Cuboid::new(COL_SPACING * 0.7, 1.0, ROW_SPACING * 0.7));

    let offset_x = (COLS as f32 * COL_SPACING) / 2.0;
    let offset_z = (ROWS as f32 * ROW_SPACING) / 2.0;

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let mut shared_mats = Vec::with_capacity(COLS * ROWS);

    for col in 0..COLS {
        for row in 0..ROWS {
            let band_pos = col as f32 / COLS as f32;
            let hue = telemetry_color::band_frequency_hue(band_pos);
            
            // Fade lightness backward in Z
            let z_fade = 1.0 - (row as f32 / ROWS as f32);
            let color_lit = lit * (0.2 + z_fade * 0.8);
            
            let color = Color::hsl(hue, sat, color_lit);
            let mat = materials.add(StandardMaterial {
                base_color: color,
                emissive: LinearRgba::from(color) * emissive * z_fade,
                metallic: policy.metallic,
                perceptual_roughness: policy.roughness,
                ..default()
            });

            shared_mats.push(mat.clone());

            let px = col as f32 * COL_SPACING - offset_x + COL_SPACING * 0.5;
            let pz = row as f32 * ROW_SPACING - offset_z + ROW_SPACING * 0.5;
            let pos = Vec3::new(px, 0.0, pz);

            // Map column to FFT bin (spread across available bins)
            let fft_bin = col * 4 + row % 4;

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(pos),
                Visibility::Hidden,
                TerrainColumn {
                    base_pos: pos,
                    fft_bin,
                },
                EffectLayer(EffectId::FftTerrain),
            ));
        }
    }

    commands.insert_resource(TerrainMaterials {
        materials: shared_mats,
    });
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    policy: Res<ThemeMaterialPolicy>,
    terrain_materials: Res<TerrainMaterials>,
    mut query: Query<(&TerrainColumn, &mut Transform)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_policy_version: Local<u64>,
    mut last_fade: Local<f32>,
    mut last_hue_offset: Local<f32>,
) {
    let fade = effect_state.fade;
    let dt = time.delta_secs();
    let decay_alpha = 1.0 - (-DECAY_RATE * dt).exp();

    let beat_pulse = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy);

    for (col, mut transform) in &mut query {
        // Get FFT magnitude for this column
        let mag = if col.fft_bin < telemetry.fft_bin_count {
            telemetry.fft[col.fft_bin]
        } else {
            0.0
        };

        // Height driven by FFT magnitude + beat pulse
        let target_height = (mag * MAX_HEIGHT * (1.0 + beat_pulse * 0.3)).max(0.1) * fade;

        // Instant attack, smooth decay (same pattern as fft_bars)
        let current = transform.scale.y;
        let height = if target_height > current {
            target_height
        } else {
            current + (target_height - current) * decay_alpha
        };

        // Scale Y for height, keep X/Z at 1
        transform.scale = Vec3::new(1.0, height, 1.0);
        // Position: base + half height (cube origin is center)
        transform.translation = Vec3::new(col.base_pos.x, height * 0.5, col.base_pos.z);
    }

    // Update shared materials once each (not per entity)
    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let policy_changed = *last_policy_version != policy.version;

    if !policy_changed
        && (fade - *last_fade).abs() < effects::FADE_EPSILON
        && (hue_offset - *last_hue_offset).abs() < 1.0
    {
        return;
    }
    *last_policy_version = policy.version;
    *last_fade = fade;
    *last_hue_offset = hue_offset;

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let flux_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier * flux_boost;

    for (i, handle) in terrain_materials.materials.iter().enumerate() {
        if let Some(material) = materials.get_mut(handle) {
            let col = i / ROWS;
            let row = i % ROWS;
            
            let band_pos = col as f32 / COLS as f32;
            let hue = telemetry_color::band_frequency_hue(band_pos) + hue_offset;
            
            // Fade lightness backward in Z
            let z_fade = 1.0 - (row as f32 / ROWS as f32);
            let color_lit = lit * (0.2 + z_fade * 0.8);
            
            let color = Color::hsl(hue % 360.0, sat, color_lit * fade);
            material.base_color = color;
            material.emissive = LinearRgba::from(color) * emissive * z_fade * fade;
            material.metallic = policy.metallic;
            material.perceptual_roughness = policy.roughness;
        }
    }
}
