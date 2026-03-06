//! Spectral waterfall — scrolling 3D spectrogram / terrain.
//!
//! A grid of cubes where each row represents a moment in time.
//! Rows physically scroll backward along the Z axis.
//! To maintain high performance (batching), we only use 64 materials (one per band)
//! and represent the magnitude via the Y-scale (height) of the cubes, rather than
//! unique colors per cube which would cause 2000+ draw calls and massive lag.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// Number of frequency bands per row (half of full FFT for manageable entity count).
const BANDS: usize = 64;

/// Number of history rows.
const ROWS: usize = 32;

/// Width of the entire waterfall.
const TOTAL_WIDTH: f32 = 28.0;

/// Depth spacing between rows.
const ROW_DEPTH: f32 = 0.6;

/// Cell size.
const CELL_SIZE: f32 = TOTAL_WIDTH / BANDS as f32;

/// Emissive intensity multiplier.
const EMISSIVE_STRENGTH: f32 = 8.0;

/// How often to scroll (seconds between row shifts).
const SCROLL_INTERVAL: f32 = 0.05;

/// Marker with (band_index, row_index).
#[derive(Component)]
pub struct WaterfallCell {
    band: usize,
    row: usize,
}

/// Shared materials per band (to allow Bevy to batch rendering into 64 draw calls instead of 2048).
#[derive(Resource)]
pub struct WaterfallMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

/// Tracks which logical row is at the front and scroll timing.
#[derive(Resource)]
pub struct WaterfallState {
    /// Logical front row index (wraps around).
    front_row: usize,
    /// Timer for scroll advancement.
    timer: f32,
    /// Set to true during crossfades to update all materials with the new fade value.
    full_update_needed: bool,
    /// FFT history: [row][band] magnitudes.
    history: [[f32; BANDS]; ROWS],
}

impl Default for WaterfallState {
    fn default() -> Self {
        Self {
            front_row: 0,
            timer: 0.0,
            full_update_needed: true,
            history: [[0.0; BANDS]; ROWS],
        }
    }
}

/// Spawn the waterfall grid (initially hidden).
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(Cuboid::new(CELL_SIZE * 0.9, 1.0, ROW_DEPTH * 0.85));

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    // Create 64 shared materials (one for each frequency band)
    let mut shared_mats = Vec::with_capacity(BANDS);
    for band in 0..BANDS {
        let hue = (band as f32 / BANDS as f32) * 270.0;
        let color = Color::hsl(hue, sat, lit);
        shared_mats.push(materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * emissive,
            ..default()
        }));
    }
    commands.insert_resource(WaterfallMaterials {
        materials: shared_mats.clone(),
    });

    for row in 0..ROWS {
        for (band, shared_mat) in shared_mats.iter().enumerate().take(BANDS) {
            let x = (band as f32 - BANDS as f32 / 2.0) * CELL_SIZE;

            // Z will be set properly in the first update

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(shared_mat.clone()),
                Transform::from_xyz(x, 0.05, -8.0).with_scale(Vec3::new(1.0, 0.01, 1.0)),
                Visibility::Hidden,
                WaterfallCell { band, row },
                EffectLayer(EffectId::SpectralWaterfall),
            ));
        }
    }
}

/// Downsample FFT bands to the waterfall band count.
fn downsample_fft(fft: &[f32], bin_count: usize) -> [f32; BANDS] {
    let mut out = [0.0; BANDS];
    if bin_count == 0 {
        return out;
    }
    let ratio = bin_count / BANDS;
    if ratio == 0 {
        // Fewer bins than waterfall bands — map directly
        for (i, val) in out.iter_mut().enumerate() {
            let src = i * bin_count / BANDS;
            *val = fft.get(src).copied().unwrap_or(0.0);
        }
    } else {
        for (i, val) in out.iter_mut().enumerate() {
            let start = i * ratio;
            let mut sum = 0.0;
            for j in 0..ratio {
                sum += fft.get(start + j).copied().unwrap_or(0.0);
            }
            *val = sum / ratio as f32;
        }
    }
    out
}

/// Advance the waterfall history by shifting the rows physically and updating their height scale.
pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut state: ResMut<WaterfallState>,
    mut query: Query<(&WaterfallCell, &mut Transform)>,
) {
    // Advance scroll on timer
    state.timer += time.delta_secs();
    let mut scrolled = false;

    if state.timer >= SCROLL_INTERVAL {
        state.timer -= SCROLL_INTERVAL;

        // Advance front row (ring buffer)
        state.front_row = (state.front_row + 1) % ROWS;

        // Write new FFT data to the new front row
        let front = state.front_row;
        let bin_count = telemetry.fft_bin_count;
        state.history[front] = downsample_fft(&telemetry.fft[..bin_count], bin_count);
        scrolled = true;
    }

    // We only need to physically move transforms if we scrolled,
    // OR if we are updating the active crossfade animation
    let fade = effect_state.fade;

    if !scrolled && fade == 1.0 && !state.full_update_needed {
        return;
    }

    if fade == 1.0 {
        state.full_update_needed = false;
    }

    // Since we use shared materials, we apply the "magnitude" and "fade" entirely via scale!
    for (cell, mut transform) in &mut query {
        // Calculate how far back this row is from the front
        let age = (state.front_row + ROWS - cell.row) % ROWS;

        if scrolled {
            // Shift physical position instead of shifting data through materials
            transform.translation.z = -(age as f32) * ROW_DEPTH - 8.0;
        }

        // Only scale if it just reached the front OR we are doing a crossfade
        if age == 0 || fade < 1.0 || state.full_update_needed {
            let magnitude = state.history[cell.row][cell.band];

            // Height = magnitude * fade multiplier. Max height ~4.0
            let target_height = (magnitude * 4.0 * fade).max(0.01);
            transform.scale.y = target_height;
            // Adjust translation so base stays flat on ground
            transform.translation.y = target_height / 2.0;
        }
    }
}

/// Handles the global fade of the shared materials during crossfades
pub fn update_materials(
    effect_state: Res<EffectState>,
    state: Res<WaterfallState>,
    waterfall_materials: Res<WaterfallMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut last_policy_version: Local<u64>,
) {
    let fade = effect_state.fade;

    // Only update materials if fade is active
    if fade == 1.0 && !state.full_update_needed && *last_policy_version == policy.version {
        return;
    }

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;
    let metallic = policy.metallic;
    let roughness = policy.roughness;

    // Update the 64 shared materials once per frame during crossfade
    for (band, handle) in waterfall_materials.materials.iter().enumerate() {
        if let Some(material) = materials.get_mut(handle) {
            let hue = (band as f32 / BANDS as f32) * 270.0;
            let lightness = (0.5 + policy.lightness_offset).clamp(0.0, 1.0) * fade;
            let color = Color::hsl(hue, sat, lightness);
            material.base_color = color;
            material.emissive = LinearRgba::from(color) * emissive * fade;
            material.metallic = metallic;
            material.perceptual_roughness = roughness;
        }
    }
    *last_policy_version = policy.version;
}
