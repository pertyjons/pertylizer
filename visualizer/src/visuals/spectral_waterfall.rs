//! Spectral waterfall — scrolling 3D spectrogram.
//!
//! A grid of cubes where each row represents a moment in time.
//! Rows scroll backward along the Z axis, with the front row
//! receiving fresh FFT data. Color intensity = magnitude.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use crate::telemetry::{NUM_FFT_BANDS, SynthTelemetry};

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

/// Tracks which logical row is at the front and scroll timing.
#[derive(Resource)]
pub struct WaterfallState {
    /// Logical front row index (wraps around).
    front_row: usize,
    /// Timer for scroll advancement.
    timer: f32,
    /// Whether materials need updating (set when history changes or fade changes).
    dirty: bool,
    /// FFT history: [row][band] magnitudes.
    history: [[f32; BANDS]; ROWS],
}

impl Default for WaterfallState {
    fn default() -> Self {
        Self {
            front_row: 0,
            timer: 0.0,
            dirty: true,
            history: [[0.0; BANDS]; ROWS],
        }
    }
}

/// Spawn the waterfall grid (initially hidden).
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(CELL_SIZE * 0.9, 0.15, ROW_DEPTH * 0.85));

    for row in 0..ROWS {
        let z = -(row as f32) * ROW_DEPTH;

        for band in 0..BANDS {
            let x = (band as f32 - BANDS as f32 / 2.0) * CELL_SIZE;

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::BLACK,
                    emissive: LinearRgba::BLACK,
                    ..default()
                })),
                Transform::from_xyz(x, 0.05, z - 8.0),
                Visibility::Hidden,
                WaterfallCell { band, row },
                EffectLayer(EffectId::SpectralWaterfall),
            ));
        }
    }
}

/// Downsample 128-band FFT to 64 bands by averaging pairs.
fn downsample_fft(fft: &[f32; NUM_FFT_BANDS]) -> [f32; BANDS] {
    let mut out = [0.0; BANDS];
    let ratio = NUM_FFT_BANDS / BANDS;
    for (i, val) in out.iter_mut().enumerate() {
        let start = i * ratio;
        let mut sum = 0.0;
        for j in 0..ratio {
            sum += fft[start + j];
        }
        *val = sum / ratio as f32;
    }
    out
}

/// Advance the waterfall history and update cell colors.
pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut state: ResMut<WaterfallState>,
    mut query: Query<(&WaterfallCell, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if effect_state.active != EffectId::SpectralWaterfall {
        return;
    }

    let fade = effect_state.fade;

    // Mark dirty during crossfade (fade is changing)
    if fade < 1.0 {
        state.dirty = true;
    }

    // Advance scroll on timer
    state.timer += time.delta_secs();
    if state.timer >= SCROLL_INTERVAL {
        state.timer -= SCROLL_INTERVAL;

        // Advance front row (ring buffer)
        state.front_row = (state.front_row + 1) % ROWS;

        // Write new FFT data to the new front row
        let front = state.front_row;
        state.history[front] = downsample_fft(&telemetry.fft);
        state.dirty = true;
    }

    // Only update materials when history or fade changed
    if !state.dirty {
        return;
    }
    state.dirty = false;

    for (cell, material_handle) in &mut query {
        // Map visual row to logical row in ring buffer
        let logical_row = (state.front_row + ROWS - cell.row) % ROWS;
        let magnitude = state.history[logical_row][cell.band];

        if let Some(material) = materials.get_mut(&material_handle.0) {
            let hue = (cell.band as f32 / BANDS as f32) * 270.0;
            let lightness = magnitude * 0.5 * fade;
            let color = Color::hsl(hue, 0.8, lightness);
            material.base_color = color;
            material.emissive = LinearRgba::from(color) * EMISSIVE_STRENGTH * magnitude * fade;
        }
    }
}
