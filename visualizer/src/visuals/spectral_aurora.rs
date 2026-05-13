//! Spectral Aurora — Ambient sky effect showing soft waving curtains.
//!
//! A tall vertical plane (XY) positioned high up (y=20) like aurora borealis.
//! Vertex displacement on the GPU creates curtain-like waves driven by RMS
//! amplitude and spectral centroid. All wave math is in the shader; CPU only
//! smooths scalar audio signals and uploads uniforms.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const COLS: usize = 40;
const ROWS: usize = 20;
const WIDTH: f32 = 50.0;
const HEIGHT: f32 = 15.0;
const Y_OFFSET: f32 = 20.0;
const EMISSIVE_STRENGTH: f32 = 1.5;

/// Smoothing rates for audio signals.
const SIGNAL_ATTACK_RATE: f32 = 12.0;
const SIGNAL_DECAY_RATE: f32 = 3.0;

/// Uniform data sent to the spectral aurora shader.
#[derive(ShaderType, Clone, Copy, Default)]
pub struct SpectralAuroraUniforms {
    pub time: f32,
    pub rms: f32,
    pub fade: f32,
    pub hue_base: f32,
    pub saturation: f32,
    pub lightness: f32,
    pub emissive_strength: f32,
    pub wave_amplitude: f32,
    pub num_cols: u32,
    pub num_rows: u32,
    pub _padding0: u32,
    pub _padding1: u32,
}

/// Custom material for spectral aurora — vertex displacement + alpha blending.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct SpectralAuroraMaterial {
    #[uniform(0)]
    pub uniforms: SpectralAuroraUniforms,
}

impl Material for SpectralAuroraMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://pertylizer_visualizer/shaders/spectral_aurora.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://pertylizer_visualizer/shaders/spectral_aurora.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // Disable backface culling so the curtain is visible from both sides
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Tracks smoothed audio state and material handle.
#[derive(Resource)]
pub struct SpectralAuroraState {
    smoothed_rms: f32,
    smoothed_wave_amp: f32,
    material_handle: Handle<SpectralAuroraMaterial>,
}

impl Default for SpectralAuroraState {
    fn default() -> Self {
        Self {
            smoothed_rms: 0.0,
            smoothed_wave_amp: 0.0,
            material_handle: Handle::default(),
        }
    }
}

/// Generate a vertical plane mesh (XY) with COLS x ROWS subdivisions.
fn create_aurora_mesh() -> Mesh {
    let half_w = WIDTH / 2.0;
    let half_h = HEIGHT / 2.0;

    let num_verts = (COLS + 1) * (ROWS + 1);
    let mut positions = Vec::with_capacity(num_verts);
    let mut normals = Vec::with_capacity(num_verts);
    let mut uvs = Vec::with_capacity(num_verts);

    #[allow(clippy::cast_precision_loss)]
    for row in 0..=ROWS {
        for col in 0..=COLS {
            let u = col as f32 / COLS as f32;
            let v = row as f32 / ROWS as f32;
            let x = u * 2.0 * half_w - half_w;
            let y = v * 2.0 * half_h - half_h;
            positions.push([x, y, 0.0]);
            normals.push([0.0, 0.0, 1.0]);
            uvs.push([u, v]);
        }
    }

    let mut indices = Vec::with_capacity(COLS * ROWS * 6);
    #[allow(clippy::cast_possible_truncation)]
    for row in 0..ROWS {
        for col in 0..COLS {
            let i = (row * (COLS + 1) + col) as u32;
            let stride = (COLS + 1) as u32;
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
    mut materials: ResMut<Assets<SpectralAuroraMaterial>>,
    mut state: ResMut<SpectralAuroraState>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(create_aurora_mesh());

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.4 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    #[allow(clippy::cast_possible_truncation)]
    let material = materials.add(SpectralAuroraMaterial {
        uniforms: SpectralAuroraUniforms {
            time: 0.0,
            rms: 0.0,
            fade: 1.0,
            hue_base: 120.0,
            saturation: sat,
            lightness: lit,
            emissive_strength: emissive,
            wave_amplitude: 0.0,
            num_cols: COLS as u32,
            num_rows: ROWS as u32,
            _padding0: 0,
            _padding1: 0,
        },
    });

    state.material_handle = material.clone();

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, Y_OFFSET, 0.0),
        Visibility::Hidden,
        EffectLayer(EffectId::SpectralAurora),
    ));
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    policy: Res<ThemeMaterialPolicy>,
    mut state: ResMut<SpectralAuroraState>,
    mut materials: ResMut<Assets<SpectralAuroraMaterial>>,
) {
    let dt = time.delta_secs();
    let fade = effect_state.fade;

    // Compute raw RMS (average of stereo channels)
    let raw_rms = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;

    // Smooth RMS with asymmetric attack/decay
    let rms_rate = if raw_rms > state.smoothed_rms {
        SIGNAL_ATTACK_RATE
    } else {
        SIGNAL_DECAY_RATE
    };
    state.smoothed_rms += (raw_rms - state.smoothed_rms) * (1.0 - (-rms_rate * dt).exp());

    // Wave amplitude driven by RMS: base gentle motion + RMS boost
    let target_amp = 1.0 + state.smoothed_rms * 8.0;
    let amp_rate = if target_amp > state.smoothed_wave_amp {
        SIGNAL_ATTACK_RATE
    } else {
        SIGNAL_DECAY_RATE
    };
    state.smoothed_wave_amp +=
        (target_amp - state.smoothed_wave_amp) * (1.0 - (-amp_rate * dt).exp());

    // Centroid-driven hue
    let hue_base = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.4 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    #[allow(clippy::cast_possible_truncation)]
    if let Some(mat) = materials.get_mut(&state.material_handle) {
        mat.uniforms = SpectralAuroraUniforms {
            time: time.elapsed_secs(),
            rms: state.smoothed_rms,
            fade,
            hue_base,
            saturation: sat,
            lightness: lit,
            emissive_strength: emissive,
            wave_amplitude: state.smoothed_wave_amp,
            num_cols: COLS as u32,
            num_rows: ROWS as u32,
            _padding0: 0,
            _padding1: 0,
        };
    }
}
