//! Note particles — burst of small spheres on note-on events.
//!
//! Each note-on spawns a cluster of particles that fly outward and fade.
//! Hue is mapped from MIDI note, speed from velocity.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use crate::telemetry::SynthTelemetry;

/// Maximum live particles to prevent unbounded growth.
const MAX_PARTICLES: usize = 512;

/// Particle lifetime in seconds.
const LIFETIME: f32 = 2.0;

/// Number of particles per note-on.
const PARTICLES_PER_NOTE: usize = 16;

/// Emissive intensity multiplier (needs to be high for bloom to pick it up).
const EMISSIVE_STRENGTH: f32 = 15.0;

/// A single note particle.
#[derive(Component)]
pub struct NoteParticle {
    /// Velocity vector (world units per second).
    velocity: Vec3,
    /// Remaining lifetime in seconds.
    life: f32,
    /// Initial lifetime (for alpha computation).
    max_life: f32,
    /// Hue for this particle (degrees).
    hue: f32,
}

/// Tracks particle count to enforce the cap.
#[derive(Resource, Default)]
pub struct ParticleCount {
    pub count: usize,
}

/// Spawn particles on note-on events.
pub fn spawn(
    mut commands: Commands,
    telemetry: Res<SynthTelemetry>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut particle_count: ResMut<ParticleCount>,
) {
    // Only trigger on fresh note-on
    if telemetry.note_age_frames >= 2 {
        return;
    }
    let Some((note, velocity, _channel)) = telemetry.last_note_on else {
        return;
    };

    // Respect particle cap
    if particle_count.count >= MAX_PARTICLES {
        return;
    }

    let hue = (note as f32 / 127.0) * 360.0;
    let speed = 2.0 + (velocity as f32 / 127.0) * 6.0;
    let mesh = meshes.add(Sphere::new(0.18));

    let spawn_count = PARTICLES_PER_NOTE.min(MAX_PARTICLES - particle_count.count);

    for i in 0..spawn_count {
        // Distribute directions using golden angle for even spread
        let golden_angle = 2.399_963; // pi * (3 - sqrt(5))
        let theta = golden_angle * i as f32;
        let phi = (1.0 - 2.0 * (i as f32 + 0.5) / spawn_count as f32).acos();

        let dir = Vec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos())
            .normalize_or_zero();

        // Slight randomization via index-based variation
        let speed_var = speed * (0.8 + 0.4 * ((i * 7 + 3) % 10) as f32 / 10.0);

        let color = Color::hsl(hue, 0.9, 0.5);
        let material = materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * EMISSIVE_STRENGTH,
            ..default()
        });

        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, 5.0, -5.0),
            NoteParticle {
                velocity: dir * speed_var,
                life: LIFETIME,
                max_life: LIFETIME,
                hue,
            },
        ));
    }

    particle_count.count += spawn_count;
}

/// Update particle positions, fade, and despawn dead particles.
pub fn update(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(
        Entity,
        &mut NoteParticle,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut particle_count: ResMut<ParticleCount>,
) {
    let dt = time.delta_secs();

    for (entity, mut particle, mut transform, material_handle) in &mut query {
        particle.life -= dt;

        if particle.life <= 0.0 {
            commands.entity(entity).despawn();
            particle_count.count = particle_count.count.saturating_sub(1);
            continue;
        }

        // Apply velocity with gravity
        particle.velocity.y -= 3.0 * dt;
        transform.translation += particle.velocity * dt;

        // Fade emissive based on remaining life
        let alpha = particle.life / particle.max_life;
        if let Some(material) = materials.get_mut(&material_handle.0) {
            let base = Color::hsl(particle.hue, 0.9, 0.3 * alpha);
            material.base_color = base;
            material.emissive = LinearRgba::from(base) * EMISSIVE_STRENGTH * alpha;
        }
    }
}
