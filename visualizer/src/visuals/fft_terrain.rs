//! FFT Terrain — Landscape of columns deforming in real-time with FFT bands.
//!
//! A grid of vertical pillars whose height is driven by FFT bin magnitudes.
//! Creates a cityscape-like terrain that dances with the frequency spectrum.
//! Colors use nonlinear frequency-to-hue mapping (bass=red, mids=green, highs=blue).

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
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
const EMISSIVE_STRENGTH: f32 = 6.0;

#[derive(Component)]
pub struct TerrainColumn {
    pub base_pos: Vec3,
    /// Which FFT bin this column represents.
    pub fft_bin: usize,
    /// Column index for color mapping.
    pub col_index: usize,
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

    for col in 0..COLS {
        let band_pos = col as f32 / COLS as f32;
        let hue = telemetry_color::band_frequency_hue(band_pos);
        let color = Color::hsl(hue, sat, lit);
        let mat = materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * emissive,
            metallic: policy.metallic,
            perceptual_roughness: policy.roughness,
            ..default()
        });

        for row in 0..ROWS {
            let px = col as f32 * COL_SPACING - offset_x + COL_SPACING * 0.5;
            let pz = row as f32 * ROW_SPACING - offset_z + ROW_SPACING * 0.5;
            let pos = Vec3::new(px, 0.0, pz);

            // Map column to FFT bin (spread across available bins)
            let fft_bin = col * 4 + row % 4;

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(pos),
                Visibility::Hidden,
                TerrainColumn {
                    base_pos: pos,
                    fft_bin,
                    col_index: col,
                },
                EffectLayer(EffectId::FftTerrain),
            ));
        }
    }
}

pub fn update(
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    policy: Res<ThemeMaterialPolicy>,
    mut query: Query<(&TerrainColumn, &mut Transform, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_policy_version: Local<u64>,
) {
    let fade = effect_state.fade;

    let beat_pulse = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy);
    let policy_changed = *last_policy_version != policy.version;
    if policy_changed {
        *last_policy_version = policy.version;
    }

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    for (col, mut transform, material_handle) in &mut query {
        // Get FFT magnitude for this column
        let mag = if col.fft_bin < telemetry.fft_bin_count {
            telemetry.fft[col.fft_bin]
        } else {
            0.0
        };

        // Height driven by FFT magnitude + beat pulse
        let height = (mag * MAX_HEIGHT * (1.0 + beat_pulse * 0.3)).max(0.1) * fade;

        // Scale Y for height, keep X/Z at 1
        transform.scale = Vec3::new(1.0, height, 1.0);
        // Position: base + half height (cube origin is center)
        transform.translation = Vec3::new(col.base_pos.x, height * 0.5, col.base_pos.z);

        // Update material color when policy changes or during fade
        if policy_changed || fade < 1.0 {
            if let Some(material) = materials.get_mut(&material_handle.0) {
                let band_pos = col.col_index as f32 / COLS as f32;
                let hue = telemetry_color::band_frequency_hue(band_pos);
                let color = Color::hsl(hue, sat, lit * fade);
                material.base_color = color;
                material.emissive = LinearRgba::from(color) * emissive * fade;
                material.metallic = policy.metallic;
                material.perceptual_roughness = policy.roughness;
            }
        }
    }
}
