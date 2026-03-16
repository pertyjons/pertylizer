//! Reaction Diffusion — Color patterns driven by spectral centroid.
//!
//! A 2D grid of small spheres with two simulated "chemical" concentrations.
//! Spectral centroid controls the feed rate (warm=slow, cool=fast),
//! RMS drives the kill rate, creating organic Turing-like patterns.
//!
//! Uses a pool of shared materials (buckets) instead of one per cell to reduce
//! GPU material re-uploads. Cells are reassigned to the nearest bucket each frame.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

const GRID_SIZE: usize = 12;
const SPACING: f32 = 2.2;
const SPHERE_RADIUS: f32 = 0.5;
const EMISSIVE_STRENGTH: f32 = 2.0;

/// Simulation speed (iterations per second).
const SIM_RATE: f32 = 15.0;

/// Number of shared material buckets. Cells are assigned to the nearest bucket
/// based on their chemical-B concentration, dramatically reducing unique materials
/// from 144 down to this count.
const MATERIAL_BUCKETS: usize = 32;

#[derive(Component)]
pub struct DiffusionCell {
    pub grid_x: usize,
    pub grid_z: usize,
    /// Chemical A concentration (0.0-1.0).
    pub chem_a: f32,
    /// Chemical B concentration (0.0-1.0).
    pub chem_b: f32,
}

/// Resource holding the shared material handles for all diffusion cells.
#[derive(Resource)]
pub struct DiffusionMaterials {
    handles: Vec<Handle<StandardMaterial>>,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Sphere::new(SPHERE_RADIUS));

    // Create shared material buckets
    let mut handles = Vec::with_capacity(MATERIAL_BUCKETS);
    for i in 0..MATERIAL_BUCKETS {
        let b = i as f32 / (MATERIAL_BUCKETS - 1) as f32;
        let hue = (200.0 + b * 120.0) % 360.0;
        let lit = 0.3 + b * 0.4;
        let color = Color::hsl(hue, 0.7, lit);
        let mat = materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * EMISSIVE_STRENGTH,
            ..default()
        });
        handles.push(mat);
    }

    commands.insert_resource(DiffusionMaterials {
        handles: handles.clone(),
    });

    let offset = (GRID_SIZE as f32 * SPACING) / 2.0;

    for x in 0..GRID_SIZE {
        for z in 0..GRID_SIZE {
            let px = x as f32 * SPACING - offset + SPACING * 0.5;
            let pz = z as f32 * SPACING - offset + SPACING * 0.5;
            let pos = Vec3::new(px, 6.0, pz);

            // Initial conditions: seed a few spots with chemical B
            let dist_from_center = ((x as f32 - GRID_SIZE as f32 / 2.0).powi(2)
                + (z as f32 - GRID_SIZE as f32 / 2.0).powi(2))
            .sqrt();
            let chem_b = if dist_from_center < 2.5 { 0.8 } else { 0.0 };

            // Assign initial material bucket based on chem_b
            let bucket = chem_b_to_bucket(chem_b);
            let mat = handles[bucket].clone();

            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(mat),
                Transform::from_translation(pos),
                Visibility::Hidden,
                DiffusionCell {
                    grid_x: x,
                    grid_z: z,
                    chem_a: 1.0,
                    chem_b,
                },
                EffectLayer(EffectId::ReactionDiffusion),
            ));
        }
    }
}

/// Map a chemical-B concentration (0.0-1.0) to a material bucket index.
fn chem_b_to_bucket(b: f32) -> usize {
    let idx = (b * (MATERIAL_BUCKETS - 1) as f32).round() as usize;
    idx.min(MATERIAL_BUCKETS - 1)
}

#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut query: Query<(
        &mut DiffusionCell,
        &mut Transform,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    diffusion_mats: Option<Res<DiffusionMaterials>>,
    mut sim_accum: Local<f32>,
    mut frame_counter: Local<u32>,
) {
    let Some(diffusion_mats) = diffusion_mats else {
        return;
    };

    let dt = time.delta_secs();
    let fade = effect_state.fade;

    // Centroid drives feed rate: low centroid (bass) = slow organic, high = fast chaotic
    let centroid_norm = (telemetry.centroid_hz / 5000.0).clamp(0.0, 1.0);
    let feed = 0.03 + centroid_norm * 0.04;

    // RMS drives kill rate
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;
    let kill = 0.06 + rms_mono * 0.02;

    // Diffusion rates
    let d_a = 0.8;
    let d_b = 0.4;

    // Accumulate time for simulation steps
    *sim_accum += dt;
    let step_interval = 1.0 / SIM_RATE;

    // Read current state into a buffer for neighbor lookups
    let mut grid_a = [[0.0_f32; GRID_SIZE]; GRID_SIZE];
    let mut grid_b = [[0.0_f32; GRID_SIZE]; GRID_SIZE];

    for (cell, _, _) in &query {
        grid_a[cell.grid_x][cell.grid_z] = cell.chem_a;
        grid_b[cell.grid_x][cell.grid_z] = cell.chem_b;
    }

    // Run simulation steps
    let steps = (*sim_accum / step_interval) as u32;
    *sim_accum -= steps as f32 * step_interval;

    // Limit to prevent spiral of death
    let steps = steps.min(3);

    for _ in 0..steps {
        let mut new_a = grid_a;
        let mut new_b = grid_b;

        for x in 0..GRID_SIZE {
            for z in 0..GRID_SIZE {
                let a = grid_a[x][z];
                let b = grid_b[x][z];

                // Laplacian (5-point stencil with wrapping)
                let xp = (x + 1) % GRID_SIZE;
                let xm = (x + GRID_SIZE - 1) % GRID_SIZE;
                let zp = (z + 1) % GRID_SIZE;
                let zm = (z + GRID_SIZE - 1) % GRID_SIZE;

                let lap_a =
                    grid_a[xp][z] + grid_a[xm][z] + grid_a[x][zp] + grid_a[x][zm] - 4.0 * a;
                let lap_b =
                    grid_b[xp][z] + grid_b[xm][z] + grid_b[x][zp] + grid_b[x][zm] - 4.0 * b;

                // Gray-Scott reaction-diffusion
                let reaction = a * b * b;
                new_a[x][z] =
                    (a + d_a * lap_a * 0.2 - reaction + feed * (1.0 - a)).clamp(0.0, 1.0);
                new_b[x][z] =
                    (b + d_b * lap_b * 0.2 + reaction - (kill + feed) * b).clamp(0.0, 1.0);
            }
        }

        grid_a = new_a;
        grid_b = new_b;
    }

    // Update shared bucket materials every 3rd frame (or during fade)
    *frame_counter = frame_counter.wrapping_add(1);
    let update_materials = frame_counter.is_multiple_of(3) || fade < 1.0;

    if update_materials {
        let hue_base = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
        let sat = (0.8 + policy.saturation_offset).clamp(0.0, 1.0);
        let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

        for (i, handle) in diffusion_mats.handles.iter().enumerate() {
            let b = i as f32 / (MATERIAL_BUCKETS - 1) as f32;
            let hue = (hue_base + b * 120.0) % 360.0;
            let lit = ((0.3 + b * 0.4 + policy.lightness_offset) * fade).clamp(0.0, 1.0);

            if let Some(material) = materials.get_mut(handle) {
                let color = Color::hsl(hue, sat, lit);
                material.base_color = color;
                material.emissive = LinearRgba::from(color) * emissive * fade;
                material.metallic = policy.metallic;
                material.perceptual_roughness = policy.roughness;
            }
        }
    }

    // Write back state, update transforms, and reassign material buckets
    for (mut cell, mut transform, mut material_handle) in &mut query {
        cell.chem_a = grid_a[cell.grid_x][cell.grid_z];
        cell.chem_b = grid_b[cell.grid_x][cell.grid_z];

        // Scale sphere based on chemical B concentration
        let b = cell.chem_b;
        let scale = (0.3 + b * 1.5) * fade;
        transform.scale = Vec3::splat(scale);

        // Slight vertical movement based on concentration
        transform.translation.y = 6.0 + b * 2.0 * fade;

        // Reassign to nearest material bucket
        let bucket = chem_b_to_bucket(b);
        let target = &diffusion_mats.handles[bucket];
        if material_handle.0 != *target {
            material_handle.0 = target.clone();
        }
    }
}
