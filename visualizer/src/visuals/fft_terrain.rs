//! FFT Terrain — GPU-driven frequency landscape.
//!
//! A single subdivided plane mesh with vertex displacement driven by FFT
//! magnitudes. CPU computes smoothed per-cell heights (instant attack, smooth
//! decay) and uploads via storage buffer. Shader handles displacement and
//! frequency-based coloring with Z-depth fade.
//! Replaces the previous 128-entity CPU approach with 1 entity / 1 draw call.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderStorageBuffer;
use bevy::shader::ShaderRef;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// Columns along the frequency axis.
const COLS: usize = 32;
/// Rows (depth) for visual fullness.
const ROWS: usize = 16;
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

/// Uniform data sent to the FFT terrain shader.
#[derive(ShaderType, Clone, Copy, Default)]
pub struct FftTerrainUniforms {
    pub num_cols: u32,
    pub num_rows: u32,
    pub fade: f32,
    pub hue_offset: f32,
    pub saturation: f32,
    pub lightness: f32,
    pub emissive_strength: f32,
    pub flux_boost: f32,
}

/// Custom material for FFT terrain — displacement from storage buffer.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct FftTerrainMaterial {
    #[uniform(0)]
    pub uniforms: FftTerrainUniforms,
    #[storage(1, read_only)]
    pub heights: Handle<ShaderStorageBuffer>,
}

impl Material for FftTerrainMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/fft_terrain.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/fft_terrain.wgsl".into()
    }
}

/// Tracks smoothed heights and GPU handles.
#[derive(Resource)]
pub struct FftTerrainState {
    /// Smoothed heights per cell (COLS * ROWS).
    smoothed_heights: Vec<f32>,
    material_handle: Handle<FftTerrainMaterial>,
    buffer_handle: Handle<ShaderStorageBuffer>,
}

impl Default for FftTerrainState {
    fn default() -> Self {
        Self {
            smoothed_heights: vec![0.0; COLS * ROWS],
            material_handle: Handle::default(),
            buffer_handle: Handle::default(),
        }
    }
}

/// Generate a subdivided plane mesh for the FFT terrain.
fn create_fft_terrain_mesh() -> Mesh {
    let cols = COLS;
    let rows = ROWS;
    let width = COLS as f32 * COL_SPACING;
    let depth = ROWS as f32 * ROW_SPACING;

    let num_verts = (cols + 1) * (rows + 1);
    let mut positions = Vec::with_capacity(num_verts);
    let mut normals = Vec::with_capacity(num_verts);
    let mut uvs = Vec::with_capacity(num_verts);

    for row in 0..=rows {
        for col in 0..=cols {
            let x = col as f32 / cols as f32 * width - width / 2.0;
            let z = row as f32 / rows as f32 * depth - depth / 2.0;
            positions.push([x, 0.0, z]);
            normals.push([0.0, 1.0, 0.0]);
            uvs.push([col as f32 / cols as f32, row as f32 / rows as f32]);
        }
    }

    let mut indices = Vec::with_capacity(cols * rows * 6);
    for row in 0..rows {
        for col in 0..cols {
            let i = (row * (cols + 1) + col) as u32;
            let stride = (cols + 1) as u32;
            indices.push(i);
            indices.push(i + stride);
            indices.push(i + 1);
            indices.push(i + 1);
            indices.push(i + stride);
            indices.push(i + stride + 1);
        }
    }

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FftTerrainMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut state: ResMut<FftTerrainState>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(create_fft_terrain_mesh());

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let buffer = buffers.add(ShaderStorageBuffer::from(vec![0.0_f32; COLS * ROWS]));
    state.buffer_handle = buffer.clone();

    let material = materials.add(FftTerrainMaterial {
        uniforms: FftTerrainUniforms {
            num_cols: COLS as u32,
            num_rows: ROWS as u32,
            fade: 1.0,
            hue_offset: 0.0,
            saturation: sat,
            lightness: lit,
            emissive_strength: emissive,
            flux_boost: 1.0,
        },
        heights: buffer,
    });

    state.material_handle = material.clone();

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        Visibility::Hidden,
        EffectLayer(EffectId::FftTerrain),
    ));
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    policy: Res<ThemeMaterialPolicy>,
    mut state: ResMut<FftTerrainState>,
    mut materials: ResMut<Assets<FftTerrainMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    let fade = effect_state.fade;
    let dt = time.delta_secs();
    let decay_alpha = 1.0 - (-DECAY_RATE * dt).exp();
    let beat_pulse = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy);

    // Update smoothed heights per cell (instant attack, smooth decay)
    for col in 0..COLS {
        for row in 0..ROWS {
            let fft_bin = col * 4 + row % 4;
            let mag = if fft_bin < telemetry.fft_bin_count {
                telemetry.fft[fft_bin]
            } else {
                0.0
            };

            let target = (mag * MAX_HEIGHT * (1.0 + beat_pulse * 0.3)).max(0.1) * fade;
            let idx = col * ROWS + row;
            let current = state.smoothed_heights[idx];

            state.smoothed_heights[idx] = if target > current {
                target
            } else {
                current + (target - current) * decay_alpha
            };
        }
    }

    // Upload heights to GPU
    if let Some(buf) = buffers.get_mut(&state.buffer_handle) {
        buf.set_data(state.smoothed_heights.clone());
    }

    // Update material uniforms
    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let flux_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    if let Some(mat) = materials.get_mut(&state.material_handle) {
        mat.uniforms = FftTerrainUniforms {
            num_cols: COLS as u32,
            num_rows: ROWS as u32,
            fade,
            hue_offset,
            saturation: sat,
            lightness: lit,
            emissive_strength: emissive,
            flux_boost,
        };
    }
}
