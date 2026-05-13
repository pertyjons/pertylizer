//! Spectral waterfall — GPU-driven scrolling 3D spectrogram.
//!
//! A single subdivided plane mesh where vertex displacement is handled entirely
//! on the GPU via a custom shader. FFT history is uploaded as a storage buffer
//! and the vertex shader displaces each vertex's Y coordinate based on magnitude.
//! This replaces the previous 2048-entity CPU approach with 1 entity / 1 draw call.

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

/// Number of frequency bands per row.
const BANDS: usize = 64;

/// Number of history rows.
const ROWS: usize = 32;

/// Width of the entire waterfall in world units.
const TOTAL_WIDTH: f32 = 28.0;

/// Depth spacing between rows.
const ROW_DEPTH: f32 = 0.6;

/// Emissive intensity multiplier.
const EMISSIVE_STRENGTH: f32 = 3.0;

/// How often to scroll (seconds between row shifts).
const SCROLL_INTERVAL: f32 = 0.05;

/// Z offset to position the waterfall behind the camera focus.
const Z_OFFSET: f32 = -8.0;

/// Uniform data sent to the waterfall shader.
#[derive(ShaderType, Clone, Copy, Default)]
pub struct WaterfallUniforms {
    pub front_row: u32,
    pub fade: f32,
    pub hue_offset: f32,
    pub saturation: f32,
    pub lightness: f32,
    pub emissive_strength: f32,
    /// Fractional scroll progress within the current interval [0, 1).
    pub scroll_fraction: f32,
    pub row_depth: f32,
}

/// Custom material for the waterfall — drives vertex displacement on the GPU.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct WaterfallMaterial {
    #[uniform(0)]
    pub uniforms: WaterfallUniforms,
    #[storage(1, read_only)]
    pub fft_history: Handle<ShaderStorageBuffer>,
}

impl Material for WaterfallMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://pertylizer_visualizer/shaders/spectral_waterfall.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://pertylizer_visualizer/shaders/spectral_waterfall.wgsl".into()
    }
}

/// Tracks ring buffer state and scroll timing.
#[derive(Resource)]
pub struct WaterfallState {
    /// Logical front row index (wraps around).
    front_row: usize,
    /// Timer for scroll advancement.
    timer: f32,
    /// FFT history: [row][band] magnitudes stored flat for GPU upload.
    history: Vec<f32>,
    /// Handle to the waterfall material asset.
    material_handle: Handle<WaterfallMaterial>,
    /// Handle to the GPU storage buffer for FFT data.
    buffer_handle: Handle<ShaderStorageBuffer>,
}

impl Default for WaterfallState {
    fn default() -> Self {
        Self {
            front_row: 0,
            timer: 0.0,
            history: vec![0.0; BANDS * ROWS],
            material_handle: Handle::default(),
            buffer_handle: Handle::default(),
        }
    }
}

/// Generate a subdivided plane mesh for the waterfall.
///
/// UV.x maps to frequency band position [0, 1].
/// UV.y maps to time/row position [0, 1] (0 = newest, 1 = oldest).
fn create_waterfall_mesh() -> Mesh {
    let cols = BANDS;
    let rows = ROWS;
    let width = TOTAL_WIDTH;
    let depth = ROWS as f32 * ROW_DEPTH;

    let num_verts = (cols + 1) * (rows + 1);
    let mut positions = Vec::with_capacity(num_verts);
    let mut normals = Vec::with_capacity(num_verts);
    let mut uvs = Vec::with_capacity(num_verts);

    for row in 0..=rows {
        for col in 0..=cols {
            let x = (col as f32 / cols as f32 - 0.5) * width;
            let z = -(row as f32 / rows as f32) * depth;
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
            // Triangle 1
            indices.push(i);
            indices.push(i + stride);
            indices.push(i + 1);
            // Triangle 2
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

/// Spawn the waterfall as a single mesh entity with a custom GPU material.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<WaterfallMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut state: ResMut<WaterfallState>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(create_waterfall_mesh());

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    // Create storage buffer for FFT history data
    let fft_data = vec![0.0_f32; BANDS * ROWS];
    let buffer = buffers.add(ShaderStorageBuffer::from(fft_data));
    state.buffer_handle = buffer.clone();

    let material = materials.add(WaterfallMaterial {
        uniforms: WaterfallUniforms {
            front_row: 0,
            fade: 1.0,
            hue_offset: 0.0,
            saturation: sat,
            lightness: lit,
            emissive_strength: emissive,
            scroll_fraction: 0.0,
            row_depth: ROW_DEPTH,
        },
        fft_history: buffer,
    });

    state.material_handle = material.clone();

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, 0.0, Z_OFFSET),
        Visibility::Hidden,
        EffectLayer(EffectId::SpectralWaterfall),
    ));
}

/// Update FFT history ring buffer and upload new data to the GPU material.
#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    policy: Res<ThemeMaterialPolicy>,
    mut state: ResMut<WaterfallState>,
    mut materials: ResMut<Assets<WaterfallMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut last_policy_version: Local<u64>,
    mut last_hue_offset: Local<f32>,
) {
    // Advance scroll timer
    state.timer += time.delta_secs();
    let mut scrolled = false;

    if state.timer >= SCROLL_INTERVAL {
        state.timer -= SCROLL_INTERVAL;

        // Advance front row (ring buffer)
        state.front_row = (state.front_row + 1) % ROWS;

        // Write new FFT data to the new front row
        let front = state.front_row;
        let bin_count = telemetry.fft_bin_count;
        let row_data = downsample_fft(&telemetry.fft[..bin_count], bin_count);

        // Copy into flat history buffer
        let offset = front * BANDS;
        state.history[offset..offset + BANDS].copy_from_slice(&row_data);

        scrolled = true;
    }

    let fade = effect_state.fade;
    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);

    // Skip GPU upload if nothing changed
    if !scrolled
        && fade == 1.0
        && *last_policy_version == policy.version
        && (hue_offset - *last_hue_offset).abs() < 1.0
    {
        return;
    }

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let scroll_fraction = state.timer / SCROLL_INTERVAL;

    // Update the storage buffer with latest FFT history
    if let Some(buf) = buffers.get_mut(&state.buffer_handle) {
        buf.set_data(state.history.clone());
    }

    // Update material uniforms
    if let Some(mat) = materials.get_mut(&state.material_handle) {
        mat.uniforms = WaterfallUniforms {
            front_row: state.front_row as u32,
            fade,
            hue_offset,
            saturation: sat,
            lightness: lit,
            emissive_strength: emissive,
            scroll_fraction,
            row_depth: ROW_DEPTH,
        };
    }

    *last_policy_version = policy.version;
    *last_hue_offset = hue_offset;
}
