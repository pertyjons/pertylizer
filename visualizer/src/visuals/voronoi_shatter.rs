//! Voronoi Shatter — GPU-driven ground cells that split with spectral flux.
//!
//! A single mesh of separate (unconnected) quads at ground level. The vertex
//! shader computes per-cell displacement, rotation, and hop from uniforms.
//! Cell identity is derived from vertex grid position.
//! Replaces the previous 100-entity CPU approach with 1 entity / 1 draw call.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const GRID_SIZE: usize = 10;
const SPACING: f32 = 2.8;
const EMISSIVE_STRENGTH: f32 = 2.0;
const SHATTER_DECAY: f32 = 3.0;
const FLUX_THRESHOLD: f32 = 0.3;

/// Uniform data for the voronoi shatter shader.
#[derive(ShaderType, Clone, Copy, Default)]
pub struct VoronoiUniforms {
    pub time: f32,
    pub shatter: f32,
    pub fade: f32,
    pub beat_pulse: f32,
    pub hue_base: f32,
    pub saturation: f32,
    pub lightness: f32,
    pub emissive_strength: f32,
    pub flux_boost: f32,
    pub shatter_boost: f32,
    pub _pad0: f32,
    pub _pad1: f32,
}

/// Custom material for the voronoi shatter effect.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct VoronoiShatterMaterial {
    #[uniform(0)]
    pub uniforms: VoronoiUniforms,
}

impl Material for VoronoiShatterMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://pertylizer_visualizer/shaders/voronoi_shatter.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://pertylizer_visualizer/shaders/voronoi_shatter.wgsl".into()
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // Double-sided rendering for tumbling quads
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Tracks shatter state and material handle.
#[derive(Resource, Default)]
pub struct VoronoiState {
    pub shatter: f32,
    pub material_handle: Handle<VoronoiShatterMaterial>,
}

/// Generate a mesh with GRID_SIZE*GRID_SIZE separate (unconnected) quads.
fn create_voronoi_mesh() -> Mesh {
    let total_cells = GRID_SIZE * GRID_SIZE;
    let half_quad = SPACING * 0.42;
    let offset = (GRID_SIZE as f32 * SPACING) / 2.0;

    let mut positions = Vec::with_capacity(total_cells * 4);
    let mut normals = Vec::with_capacity(total_cells * 4);
    let mut uvs = Vec::with_capacity(total_cells * 4);
    let mut indices = Vec::with_capacity(total_cells * 6);

    for x in 0..GRID_SIZE {
        for z in 0..GRID_SIZE {
            let cx = x as f32 * SPACING - offset + SPACING * 0.5;
            let cz = z as f32 * SPACING - offset + SPACING * 0.5;
            let base_idx = positions.len() as u32;

            // 4 corners of the quad
            positions.push([cx - half_quad, 0.05, cz - half_quad]);
            positions.push([cx + half_quad, 0.05, cz - half_quad]);
            positions.push([cx + half_quad, 0.05, cz + half_quad]);
            positions.push([cx - half_quad, 0.05, cz + half_quad]);

            for _ in 0..4 {
                normals.push([0.0, 1.0, 0.0]);
                uvs.push([0.0, 0.0]);
            }

            // Two triangles
            indices.push(base_idx);
            indices.push(base_idx + 2);
            indices.push(base_idx + 1);
            indices.push(base_idx);
            indices.push(base_idx + 3);
            indices.push(base_idx + 2);
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
    mut materials: ResMut<Assets<VoronoiShatterMaterial>>,
    mut state: ResMut<VoronoiState>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(create_voronoi_mesh());

    let sat = (0.7 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.4 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let material = materials.add(VoronoiShatterMaterial {
        uniforms: VoronoiUniforms {
            time: 0.0,
            shatter: 0.0,
            fade: 1.0,
            beat_pulse: 0.0,
            hue_base: 270.0,
            saturation: sat,
            lightness: lit,
            emissive_strength: emissive,
            flux_boost: 1.0,
            shatter_boost: 1.0,
            _pad0: 0.0,
            _pad1: 0.0,
        },
    });

    state.material_handle = material.clone();

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        Visibility::Hidden,
        EffectLayer(EffectId::VoronoiShatter),
    ));
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    policy: Res<ThemeMaterialPolicy>,
    mut state: ResMut<VoronoiState>,
    mut materials: ResMut<Assets<VoronoiShatterMaterial>>,
) {
    let dt = time.delta_secs();
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

    let beat_pulse = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy);
    let hue_base = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy) + 270.0;
    let sat = (0.7 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.4 + policy.lightness_offset).clamp(0.0, 1.0);
    let flux_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;
    let shatter_boost = 1.0 + state.shatter * 0.5;

    if let Some(mat) = materials.get_mut(&state.material_handle) {
        mat.uniforms = VoronoiUniforms {
            time: time.elapsed_secs(),
            shatter: state.shatter,
            fade,
            beat_pulse,
            hue_base,
            saturation: sat,
            lightness: lit,
            emissive_strength: emissive,
            flux_boost,
            shatter_boost,
            _pad0: 0.0,
            _pad1: 0.0,
        };
    }
}
