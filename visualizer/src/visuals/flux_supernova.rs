//! Flux Supernova — Star that explodes on sudden spectral changes (high flux).
//!
//! A central core with spikes extending and glowing brightly based on Spectral Flux and RMS.
//! High flux triggers "eruptions".

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};
use crate::telemetry::SynthTelemetry;

/// The threshold of spectral flux to trigger an explosion.
const FLUX_THRESHOLD: f32 = 0.5;

/// Decay rate of the explosion.
const DECAY_RATE: f32 = 4.0;

/// Base emissive strength.
const BASE_EMISSIVE: f32 = 5.0;

/// Peak emissive strength during supernova.
const PEAK_EMISSIVE: f32 = 50.0;

#[derive(Component)]
pub struct SupernovaCore;

#[derive(Resource, Default)]
pub struct SupernovaState {
    /// Current explosion intensity (decays over time).
    pub explosion: f32,
    /// Previous flux to detect spikes.
    pub prev_flux: f32,
    /// Has material been fully restored to idle state
    pub is_idle: bool,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Create an icosphere for the core
    let core_mesh = meshes.add(Sphere::new(2.5).mesh().ico(3).unwrap());

    let core_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.5, 0.0),
        emissive: LinearRgba::from(Color::srgb(1.0, 0.5, 0.0)) * BASE_EMISSIVE,
        ..default()
    });

    commands.spawn((
        Mesh3d(core_mesh),
        MeshMaterial3d(core_mat),
        Transform::from_xyz(0.0, 6.0, 0.0),
        Visibility::Hidden,
        SupernovaCore,
        EffectLayer(EffectId::FluxSupernova),
    ));
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    effect_state: Res<EffectState>,
    mut state: ResMut<SupernovaState>,
    mut query: Query<(&mut Transform, &MeshMaterial3d<StandardMaterial>), With<SupernovaCore>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs();

    // Detect flux spike (sudden onset)
    let flux = telemetry.flux;
    if flux > state.prev_flux + FLUX_THRESHOLD {
        // Add to explosion, capped at 1.0
        state.explosion = (state.explosion + (flux * 0.5)).min(1.0);
        state.is_idle = false;
    }
    state.prev_flux = flux;

    // Decay the explosion
    if state.explosion > 0.0 {
        state.explosion *= (-DECAY_RATE * dt).exp();
        if state.explosion < 0.001 {
            state.explosion = 0.0;
        }
    }

    // RMS controls base size/pulsing
    let energy = ((telemetry.rms[0] + telemetry.rms[1]) * 0.5 * 3.0).clamp(0.1, 1.0);
    let scale = 1.0 + (energy * 0.5) + (state.explosion * 1.5);

    let fade = effect_state.fade;

    for (mut transform, material_handle) in &mut query {
        transform.scale = Vec3::splat(scale);
        // Slowly rotate
        transform.rotate_local_y(0.2 * dt);
        transform.rotate_local_x(0.1 * dt);

        // Only mutate material if exploding, fading, or just entered idle state
        if state.explosion > 0.0 || fade < 1.0 || !state.is_idle {
            if let Some(material) = materials.get_mut(&material_handle.0) {
                // Color shifts from orange/red to bright yellow/white on explosion
                let hue = 15.0 + (state.explosion * 45.0);
                let lightness = 0.5 + (state.explosion * 0.4);
                let color = Color::hsl(hue, 0.9, lightness * fade);

                let emissive_strength =
                    BASE_EMISSIVE + (state.explosion * (PEAK_EMISSIVE - BASE_EMISSIVE));

                material.base_color = color;
                material.emissive = LinearRgba::from(color) * emissive_strength * fade;
            }

            if state.explosion == 0.0 && fade == 1.0 {
                state.is_idle = true;
            }
        }
    }
}
