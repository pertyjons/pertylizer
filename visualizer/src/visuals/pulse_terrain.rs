//! Pulse Terrain — GPU-driven landscape that breathes with bass.
//!
//! A single subdivided plane mesh with vertex displacement handled entirely
//! on the GPU. Ripple waves, noise, and bass displacement are computed in the
//! vertex shader. CPU only smooths scalar audio signals and uploads uniforms.
//! Replaces the previous 400-entity CPU approach with 1 entity / 1 draw call.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const GRID_SIZE: usize = 40;
const GRID_SPACING: f32 = 1.5;
const EMISSIVE_STRENGTH: f32 = 2.0;

/// Input signal smoothing — attack (signal rising).
const SIGNAL_ATTACK_RATE: f32 = 20.0;
/// Input signal smoothing — decay (signal falling).
const SIGNAL_DECAY_RATE: f32 = 5.0;

/// Uniform data sent to the pulse terrain shader.
#[derive(ShaderType, Clone, Copy, Default)]
pub struct PulseTerrainUniforms {
    pub time: f32,
    pub smoothed_bass: f32,
    pub smoothed_rms: f32,
    pub beat_pulse: f32,
    pub velocity_boost: f32,
    pub fade: f32,
    pub grid_half_extent: f32,
    pub max_dist: f32,
    pub hue_base: f32,
    pub saturation: f32,
    pub lightness: f32,
    pub emissive_strength: f32,
    pub flux_boost: f32,
    pub _padding: f32,
}

/// Custom material for pulse terrain — all wave math on GPU.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct PulseTerrainMaterial {
    #[uniform(0)]
    pub uniforms: PulseTerrainUniforms,
}

impl Material for PulseTerrainMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/pulse_terrain.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/pulse_terrain.wgsl".into()
    }
}

/// Tracks smoothed audio state and material handle.
#[derive(Resource)]
pub struct PulseTerrainState {
    smoothed_bass: f32,
    smoothed_rms: f32,
    material_handle: Handle<PulseTerrainMaterial>,
}

impl Default for PulseTerrainState {
    fn default() -> Self {
        Self {
            smoothed_bass: 0.0,
            smoothed_rms: 0.0,
            material_handle: Handle::default(),
        }
    }
}

/// Generate a subdivided plane mesh for the terrain.
fn create_terrain_mesh() -> Mesh {
    let cols = GRID_SIZE;
    let rows = GRID_SIZE;
    let half = (GRID_SIZE as f32 * GRID_SPACING) / 2.0;

    let num_verts = (cols + 1) * (rows + 1);
    let mut positions = Vec::with_capacity(num_verts);
    let mut normals = Vec::with_capacity(num_verts);
    let mut uvs = Vec::with_capacity(num_verts);

    for row in 0..=rows {
        for col in 0..=cols {
            let x = col as f32 / cols as f32 * 2.0 * half - half;
            let z = row as f32 / rows as f32 * 2.0 * half - half;
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
    mut materials: ResMut<Assets<PulseTerrainMaterial>>,
    mut state: ResMut<PulseTerrainState>,
    policy: Res<ThemeMaterialPolicy>,
) {
    let mesh = meshes.add(create_terrain_mesh());

    let half = (GRID_SIZE as f32 * GRID_SPACING) / 2.0;
    let max_dist = (half * half * 2.0).sqrt();

    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let material = materials.add(PulseTerrainMaterial {
        uniforms: PulseTerrainUniforms {
            time: 0.0,
            smoothed_bass: 0.0,
            smoothed_rms: 0.0,
            beat_pulse: 0.0,
            velocity_boost: 0.0,
            fade: 1.0,
            grid_half_extent: half,
            max_dist,
            hue_base: 140.0,
            saturation: sat,
            lightness: lit,
            emissive_strength: emissive,
            flux_boost: 1.0,
            _padding: 0.0,
        },
    });

    state.material_handle = material.clone();

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        Visibility::Hidden,
        EffectLayer(EffectId::PulseTerrain),
    ));
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    policy: Res<ThemeMaterialPolicy>,
    mut state: ResMut<PulseTerrainState>,
    mut materials: ResMut<Assets<PulseTerrainMaterial>>,
) {
    let dt = time.delta_secs();
    let fade = effect_state.fade;

    // Transport stopped → slow down ripple animation
    let time_scale = if telemetry.playing { 1.0 } else { 0.15 };
    let t = time.elapsed_secs() * time_scale;

    // Compute raw bass from lower FFT bands
    let mut raw_bass = 0.0;
    for i in 0..10 {
        raw_bass += telemetry.fft[i];
    }
    raw_bass = (raw_bass / 10.0) * 15.0;

    // Compute raw RMS
    let raw_rms = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;

    // Smooth bass with asymmetric easing
    let bass_rate = if raw_bass > state.smoothed_bass {
        SIGNAL_ATTACK_RATE
    } else {
        SIGNAL_DECAY_RATE
    };
    state.smoothed_bass += (raw_bass - state.smoothed_bass) * (1.0 - (-bass_rate * dt).exp());

    // Smooth RMS with asymmetric easing
    let rms_rate = if raw_rms > state.smoothed_rms {
        SIGNAL_ATTACK_RATE
    } else {
        SIGNAL_DECAY_RATE
    };
    state.smoothed_rms += (raw_rms - state.smoothed_rms) * (1.0 - (-rms_rate * dt).exp());

    let beat_pulse = telemetry_color::beat_pulse_factor(telemetry.beat_phase, &policy);

    // Recent velocity boosts ground intensity
    let velocity_boost = if telemetry.note_age_frames < 6 {
        if let Some(note) = telemetry.last_note_on {
            let vel_norm = note.velocity as f32 / 127.0;
            let age_decay = 1.0 - (telemetry.note_age_frames as f32 / 6.0);
            vel_norm * age_decay
        } else {
            0.0
        }
    } else {
        0.0
    };

    let hue_base = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;
    let flux_boost = 1.0 + telemetry_color::flux_emissive_boost(telemetry.flux, &policy);

    let half = (GRID_SIZE as f32 * GRID_SPACING) / 2.0;
    let max_dist = (half * half * 2.0).sqrt();

    if let Some(mat) = materials.get_mut(&state.material_handle) {
        mat.uniforms = PulseTerrainUniforms {
            time: t,
            smoothed_bass: state.smoothed_bass,
            smoothed_rms: state.smoothed_rms,
            beat_pulse,
            velocity_boost,
            fade,
            grid_half_extent: half,
            max_dist,
            hue_base,
            saturation: sat,
            lightness: lit,
            emissive_strength: emissive,
            flux_boost,
            _padding: 0.0,
        };
    }
}
