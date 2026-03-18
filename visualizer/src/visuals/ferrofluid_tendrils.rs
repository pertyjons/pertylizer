//! Ferrofluid Tendrils — Raymarched SDF with smooth blending.
//!
//! Renders a cohesive liquid-metal surface using signed distance fields and
//! raymarching. Vertical capsules arranged in a circle are driven by FFT
//! magnitudes and blended together with smooth minimum (smin) to create
//! an organic ferrofluid/mercury appearance.
//!
//! Replaces the previous 30-cylinder CPU approach with 1 cube bounding volume
//! and a raymarching fragment shader.

use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderStorageBuffer;
use bevy::shader::ShaderRef;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const NUM_TENDRILS: usize = 30;
const RADIUS: f32 = 8.0;
const MAX_HEIGHT: f32 = 12.0;
const EMISSIVE_STRENGTH: f32 = 2.5;

/// Uniform data for the ferrofluid raymarching shader.
#[derive(ShaderType, Clone, Copy, Default)]
pub struct FerrofluidUniforms {
    pub time: f32,
    pub fade: f32,
    pub num_tendrils: u32,
    pub hue_offset: f32,
    pub emissive_strength: f32,
    pub saturation: f32,
    pub lightness: f32,
    pub _padding: f32,
}

/// Raymarched ferrofluid material — fragment shader does all the heavy lifting.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct FerrofluidMaterial {
    #[uniform(0)]
    pub uniforms: FerrofluidUniforms,
    #[storage(1, read_only)]
    pub tendrils: Handle<ShaderStorageBuffer>,
}

impl Material for FerrofluidMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/ferrofluid_tendrils.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/ferrofluid_tendrils.wgsl".into()
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // Disable backface culling so rays can enter the bounding volume from any angle
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Tracks smoothed tendril heights and GPU handles.
#[derive(Resource)]
pub struct FerrofluidState {
    /// Smoothed heights per tendril.
    heights: Vec<f32>,
    /// Pre-computed XZ positions and hues per tendril.
    positions: Vec<(f32, f32, f32)>, // (x, z, base_hue)
    material_handle: Handle<FerrofluidMaterial>,
    buffer_handle: Handle<ShaderStorageBuffer>,
}

impl Default for FerrofluidState {
    fn default() -> Self {
        Self {
            heights: vec![0.01; NUM_TENDRILS],
            positions: Vec::new(),
            material_handle: Handle::default(),
            buffer_handle: Handle::default(),
        }
    }
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FerrofluidMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
    mut state: ResMut<FerrofluidState>,
    policy: Res<ThemeMaterialPolicy>,
) {
    // Bounding volume cube: must encompass all possible tendrils + max height
    let mesh = meshes.add(Cuboid::new(RADIUS * 3.0, MAX_HEIGHT + 2.0, RADIUS * 3.0));

    // Pre-compute tendril positions (same layout as original)
    let mut positions = Vec::with_capacity(NUM_TENDRILS);
    for i in 0..NUM_TENDRILS {
        let angle = (i as f32 / NUM_TENDRILS as f32) * std::f32::consts::TAU;
        let band_index = (i * 2) % 32;
        let x = angle.cos() * RADIUS + (i as f32 * 2.4).sin() * 2.0;
        let z = angle.sin() * RADIUS + (i as f32 * 2.4).sin() * 2.0;
        let hue = telemetry_color::band_frequency_hue(band_index as f32 / 32.0);
        positions.push((x, z, hue));
    }
    state.positions = positions;

    // Initialize storage buffer: vec4 per tendril (x, z, height, hue)
    let tendril_data: Vec<[f32; 4]> = state
        .positions
        .iter()
        .map(|(x, z, hue)| [*x, *z, 0.01, *hue])
        .collect();
    let buffer = buffers.add(ShaderStorageBuffer::from(tendril_data));
    state.buffer_handle = buffer.clone();

    let sat = (0.7 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.3 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let material = materials.add(FerrofluidMaterial {
        uniforms: FerrofluidUniforms {
            time: 0.0,
            fade: 1.0,
            num_tendrils: NUM_TENDRILS as u32,
            hue_offset: 0.0,
            emissive_strength: emissive,
            saturation: sat,
            lightness: lit,
            _padding: 0.0,
        },
        tendrils: buffer,
    });

    state.material_handle = material.clone();

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        // Position so the base of the bounding volume is at y=0
        Transform::from_xyz(0.0, (MAX_HEIGHT + 2.0) / 2.0 - 0.5, 0.0),
        Visibility::Hidden,
        EffectLayer(EffectId::FerrofluidTendrils),
    ));
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    policy: Res<ThemeMaterialPolicy>,
    mut state: ResMut<FerrofluidState>,
    mut materials: ResMut<Assets<FerrofluidMaterial>>,
    mut buffers: ResMut<Assets<ShaderStorageBuffer>>,
) {
    let dt = time.delta_secs();
    let fade = effect_state.fade;

    // Smooth tendril heights (asymmetric attack/decay)
    for i in 0..NUM_TENDRILS {
        let band_index = (i * 2) % 32;
        let mag = if band_index < telemetry.fft_bin_count {
            telemetry.fft[band_index]
        } else {
            0.0
        };

        let target = (mag * MAX_HEIGHT).max(0.1) * fade;
        let speed = if target > state.heights[i] { 5.0 } else { 2.0 };
        state.heights[i] += (target - state.heights[i]) * speed * dt;
    }

    // Pack tendril data (x, z, height, hue)
    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let tendril_data: Vec<[f32; 4]> = state
        .positions
        .iter()
        .zip(state.heights.iter())
        .map(|((x, z, hue), h)| [*x, *z, *h, *hue])
        .collect();

    if let Some(buf) = buffers.get_mut(&state.buffer_handle) {
        buf.set_data(tendril_data);
    }

    let sat = (0.7 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.3 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    if let Some(mat) = materials.get_mut(&state.material_handle) {
        mat.uniforms = FerrofluidUniforms {
            time: time.elapsed_secs(),
            fade,
            num_tendrils: NUM_TENDRILS as u32,
            hue_offset,
            emissive_strength: emissive,
            saturation: sat,
            lightness: lit,
            _padding: 0.0,
        };
    }
}
