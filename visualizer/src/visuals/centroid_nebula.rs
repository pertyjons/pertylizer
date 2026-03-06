//! Centroid Nebula — Particle cloud shifting color and shape based on spectral brightness.

use bevy::color::LinearRgba;
use bevy::prelude::*;
use rand::Rng;

use super::effects::{self, EffectId, EffectLayer, EffectState};
use crate::telemetry::SynthTelemetry;

/// Number of particles in the nebula (reduced from 2000 for performance).
const NEBULA_PARTICLES: usize = 500;

/// Base bounds of the particle cloud.
const BOUNDS: f32 = 15.0;

/// Emissive multiplier.
const EMISSIVE_STRENGTH: f32 = 4.0;

/// A single particle in the nebula.
#[derive(Component)]
pub struct NebulaParticle {
    /// Base (original) position.
    base_pos: Vec3,
    /// Random offset phase for noise/movement.
    phase: f32,
    /// Base size.
    base_size: f32,
}

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Sphere::new(0.15));

    // Create one shared material for all particles since they share color state
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.1, 0.1, 0.3),
        emissive: LinearRgba::from(Color::srgb(0.1, 0.1, 0.3)) * EMISSIVE_STRENGTH,
        ..default()
    });

    let mut rng = rand::thread_rng();

    for _ in 0..NEBULA_PARTICLES {
        let x = rng.gen_range(-BOUNDS..BOUNDS);
        let y = rng.gen_range(-BOUNDS..BOUNDS) + 5.0;
        let z = rng.gen_range(-BOUNDS..BOUNDS) - 5.0;
        let pos = Vec3::new(x, y, z);

        let phase = rng.gen_range(0.0..std::f32::consts::TAU);
        let base_size = rng.gen_range(0.5..1.5);

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(pos).with_scale(Vec3::splat(base_size)),
            Visibility::Hidden,
            NebulaParticle {
                base_pos: pos,
                phase,
                base_size,
            },
            EffectLayer(EffectId::CentroidNebula),
        ));
    }
}

pub fn update(
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    mut query: Query<(&NebulaParticle, &mut Transform)>,
    mut last_centroid: Local<f32>,
) {
    let t = time.elapsed_secs();

    // Smooth the centroid value so it doesn't jitter violently
    // Typical centroid is 500Hz - 8000Hz
    let raw_centroid = telemetry.centroid_hz.clamp(200.0, 10000.0);
    let centroid = *last_centroid + (raw_centroid - *last_centroid) * 0.1;
    *last_centroid = centroid;

    // Normalize centroid to a 0.0 - 1.0 range (logarithmic is better for frequency)
    let centroid_norm = ((centroid.log2() - 200.0_f32.log2())
        / (10000.0_f32.log2() - 200.0_f32.log2()))
    .clamp(0.0, 1.0);

    // RMS controls overall energy/size
    let energy = ((telemetry.rms[0] + telemetry.rms[1]) * 0.5 * 5.0).clamp(0.1, 2.0);

    for (particle, mut transform) in &mut query {
        // High centroid = fast turbulent movement. Low centroid = slow rolling
        let speed = 0.5 + centroid_norm * 3.0;
        let p = particle.phase + t * speed;

        // Expansion based on energy
        let spread = 1.0 + energy * 0.5;

        // Simple pseudo-noise displacement using trig functions
        let offset = Vec3::new(
            (p * 1.3).sin() * 2.0 * centroid_norm,
            (p * 0.8).cos() * 2.0 * centroid_norm,
            (p * 1.1).sin() * 2.0 * centroid_norm,
        );

        transform.translation = particle.base_pos * spread + offset;

        // Scale based on energy
        transform.scale = Vec3::splat(particle.base_size * energy);
    }
}

pub fn update_material(
    effect_state: Res<EffectState>,
    telemetry: Res<SynthTelemetry>,
    query: Query<&MeshMaterial3d<StandardMaterial>, With<NebulaParticle>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_centroid: Local<f32>,
    mut last_hue: Local<f32>,
    mut last_fade: Local<f32>,
) {
    let fade = effect_state.fade;

    let raw_centroid = telemetry.centroid_hz.clamp(200.0, 10000.0);
    let centroid = if *last_centroid == 0.0 {
        raw_centroid
    } else {
        *last_centroid + (raw_centroid - *last_centroid) * 0.1
    };
    *last_centroid = centroid;
    let centroid_norm = ((centroid.max(1.0).log2() - 200.0_f32.log2())
        / (10000.0_f32.log2() - 200.0_f32.log2()))
    .clamp(0.0, 1.0);

    let hue = 330.0 - (centroid_norm * 130.0);

    // Only update material when hue or fade changes meaningfully
    if (hue - *last_hue).abs() < 1.0 && (fade - *last_fade).abs() < effects::FADE_EPSILON {
        return;
    }

    if let Some(handle) = query.iter().next()
        && let Some(material) = materials.get_mut(&handle.0)
    {
        let color = Color::hsl(hue, 0.8, 0.5 * fade);
        material.base_color = color;
        material.emissive = LinearRgba::from(color) * EMISSIVE_STRENGTH * fade;
    }

    *last_hue = hue;
    *last_fade = fade;
}
