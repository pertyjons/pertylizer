//! Note particles — burst of small spheres on note-on events.
//!
//! Each note-on spawns a cluster of particles that fly outward and shrink.
//! Hue is mapped from MIDI note, speed from velocity.
//! Uses a shared material pool to prevent asset churn and stuttering.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// Maximum live particles to prevent unbounded growth.
const MAX_PARTICLES: usize = 512;

/// Particle lifetime in seconds.
const LIFETIME: f32 = 2.0;

/// Number of particles per note-on.
const PARTICLES_PER_NOTE: usize = 16;

/// Emissive intensity multiplier (needs to be high for bloom to pick it up).
const EMISSIVE_STRENGTH: f32 = 15.0;

/// Golden angle in radians for even spherical distribution.
const GOLDEN_ANGLE: f32 = 2.399_963;

/// A single note particle.
#[derive(Component)]
pub struct NoteParticle {
    /// Velocity vector (world units per second).
    velocity: Vec3,
    /// Remaining lifetime in seconds.
    life: f32,
    /// Initial lifetime (for scale computation).
    max_life: f32,
}

/// Cached mesh handle to avoid creating a new mesh asset per note-on.
#[derive(Resource)]
pub struct ParticleMesh(Handle<Mesh>);

/// Shared materials for each MIDI note (0-127) to avoid asset churn.
#[derive(Resource)]
pub struct ParticleMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

/// Tracks particle count to enforce the cap.
#[derive(Resource, Default)]
pub struct ParticleCount {
    pub count: usize,
}

/// Create the shared particle mesh and 128 note materials at startup.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    commands.insert_resource(ParticleMesh(meshes.add(Sphere::new(0.18))));

    let sat = (0.9 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let mut shared_mats = Vec::with_capacity(128);
    for note in 0..128 {
        let hue = (note as f32 / 127.0) * 360.0;
        let color = Color::hsl(hue, sat, lit);
        shared_mats.push(materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * emissive,
            ..default()
        }));
    }
    commands.insert_resource(ParticleMaterials {
        materials: shared_mats,
    });
}

/// Spawn particles on note-on events using shared materials.
pub fn spawn(
    mut commands: Commands,
    telemetry: Res<SynthTelemetry>,
    particle_mesh: Res<ParticleMesh>,
    particle_materials: Res<ParticleMaterials>,
    mut particle_count: ResMut<ParticleCount>,
) {
    // Voice count scales burst size: more voices → more particles per note
    let voice_scale = 1.0 + (telemetry.voice_count as f32 / 32.0).clamp(0.0, 1.0);

    for note_event in &telemetry.pending_note_events {
        if particle_count.count >= MAX_PARTICLES {
            break;
        }

        let note_idx = (note_event.midi_note as usize).min(127);
        let speed = 2.0 + (note_event.velocity as f32 / 127.0) * 6.0;

        let scaled_count = (PARTICLES_PER_NOTE as f32 * voice_scale) as usize;
        let spawn_count = scaled_count.min(MAX_PARTICLES - particle_count.count);

        for i in 0..spawn_count {
            // Distribute directions using golden angle for even spread
            let theta = GOLDEN_ANGLE * i as f32;
            let phi = (1.0 - 2.0 * (i as f32 + 0.5) / spawn_count as f32).acos();

            let dir = Vec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos())
                .normalize_or_zero();

            // Slight randomization via index-based variation
            let speed_var = speed * (0.8 + 0.4 * ((i * 7 + 3) % 10) as f32 / 10.0);

            let material = particle_materials.materials[note_idx].clone();

            commands.spawn((
                Mesh3d(particle_mesh.0.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(0.0, 5.0, -5.0),
                NoteParticle {
                    velocity: dir * speed_var,
                    life: LIFETIME,
                    max_life: LIFETIME,
                },
            ));
        }

        particle_count.count += spawn_count;
    }
}

/// Update particle positions, shrink, and despawn dead particles.
pub fn update(
    mut commands: Commands,
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    particle_materials: Res<ParticleMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut last_policy_version: Local<u64>,
    mut last_hue_offset: Local<f32>,
    mut query: Query<(Entity, &mut NoteParticle, &mut Transform)>,
    mut particle_count: ResMut<ParticleCount>,
) {
    let dt = time.delta_secs();

    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    if *last_policy_version != policy.version || (hue_offset - *last_hue_offset).abs() > 1.0 {
        let sat = (0.9 + policy.saturation_offset).clamp(0.0, 1.0);
        let lit = (0.5 + policy.lightness_offset).clamp(0.0, 1.0);
        let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

        for (note, handle) in particle_materials.materials.iter().enumerate() {
            if let Some(material) = materials.get_mut(handle) {
                let hue = ((note as f32 / 127.0) * 360.0 + hue_offset) % 360.0;
                let color = Color::hsl(hue, sat, lit);
                material.base_color = color;
                material.emissive = LinearRgba::from(color) * emissive;
                material.metallic = policy.metallic;
                material.perceptual_roughness = policy.roughness;
            }
        }
        *last_policy_version = policy.version;
        *last_hue_offset = hue_offset;
    }

    for (entity, mut particle, mut transform) in &mut query {
        particle.life -= dt;

        if particle.life <= 0.0 {
            commands.entity(entity).despawn();
            particle_count.count = particle_count.count.saturating_sub(1);
            continue;
        }

        // Apply velocity with gravity
        particle.velocity.y -= 3.0 * dt;
        transform.translation += particle.velocity * dt;

        // Shrink instead of fading emissive to avoid material mutation
        let scale = (particle.life / particle.max_life).max(0.0);
        transform.scale = Vec3::splat(scale);
    }
}
