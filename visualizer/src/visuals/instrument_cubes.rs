//! Per-instrument metallic cube particles.
//!
//! Each instrument gets its own emitter position arranged in a circle.
//! Note-on events spawn small metallic cubes that fly outward and tumble.
//! Color is determined by instrument category.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer};
use super::theme::ThemeMaterialPolicy;
use crate::telemetry::SynthTelemetry;

/// Maximum live cubes across all instruments.
const MAX_CUBES: usize = 1024;

/// Cube lifetime in seconds.
const LIFETIME: f32 = 2.5;

/// Cubes spawned per note-on.
const CUBES_PER_NOTE: usize = 8;

/// Emissive intensity for bloom pickup.
const EMISSIVE_STRENGTH: f32 = 12.0;

/// Radius of the emitter circle.
const EMITTER_RADIUS: f32 = 8.0;

/// Height of emitter positions.
const EMITTER_Y: f32 = 4.0;

/// Golden angle for even spherical distribution.
const GOLDEN_ANGLE: f32 = 2.399_963;

/// Category constants matching `InstrumentCategory` repr(u8).
mod category {
    pub const DRUMS: u8 = 1;
    pub const BASS: u8 = 2;
    pub const PAD: u8 = 3;
    pub const LEAD: u8 = 4;
    pub const ARP: u8 = 5;
    pub const KEYS: u8 = 6;
    pub const FX: u8 = 7;
}

/// A single instrument cube particle.
#[derive(Component)]
pub struct InstrumentCube {
    velocity: Vec3,
    /// Angular velocity for tumbling (radians/sec per axis).
    spin: Vec3,
    life: f32,
    max_life: f32,
}

/// Shared cube mesh resource.
#[derive(Resource)]
pub struct CubeMesh(Handle<Mesh>);

/// Pre-built materials per category (indices 0..8).
#[derive(Resource)]
pub struct CubeMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

/// Tracks emitter positions keyed by instrument_id.
#[derive(Resource, Default)]
pub struct EmitterRegistry {
    /// (instrument_id, angle) — angle on the emitter circle.
    instruments: Vec<(u32, f32)>,
}

/// Tracks total live cube count.
#[derive(Resource, Default)]
pub struct CubeCount {
    pub count: usize,
}

impl EmitterRegistry {
    /// Get or create an emitter position for an instrument.
    fn position_for(&mut self, instrument_id: u32) -> Vec3 {
        let angle = if let Some((_, angle)) =
            self.instruments.iter().find(|(id, _)| *id == instrument_id)
        {
            *angle
        } else {
            // Assign next angle evenly spaced
            let angle = self.instruments.len() as f32 * GOLDEN_ANGLE;
            self.instruments.push((instrument_id, angle));
            angle
        };

        Vec3::new(
            EMITTER_RADIUS * angle.cos(),
            EMITTER_Y,
            EMITTER_RADIUS * angle.sin() - 5.0,
        )
    }
}

/// Returns the (hue, base_saturation, base_lightness) for an instrument category.
fn category_hsl(cat: u8) -> (f32, f32, f32) {
    match cat {
        category::DRUMS => (0.0, 0.85, 0.45), // red
        category::BASS => (230.0, 0.8, 0.35), // deep blue
        category::PAD => (280.0, 0.7, 0.5),   // purple
        category::LEAD => (50.0, 0.95, 0.55), // golden yellow
        category::ARP => (180.0, 0.85, 0.5),  // cyan
        category::KEYS => (130.0, 0.7, 0.45), // green
        category::FX => (30.0, 0.9, 0.5),     // orange
        _ => (0.0, 0.0, 0.6),                 // silver/gray for uncategorized
    }
}

/// Create cube mesh and category materials at startup.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
) {
    commands.insert_resource(CubeMesh(meshes.add(Cuboid::new(0.12, 0.12, 0.12))));

    let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;

    let mut mats = Vec::with_capacity(8);
    for cat in 0..8u8 {
        let (hue, base_sat, base_lit) = category_hsl(cat);
        let sat = (base_sat + policy.saturation_offset).clamp(0.0, 1.0);
        let lit = (base_lit + policy.lightness_offset).clamp(0.0, 1.0);
        let color = Color::hsl(hue, sat, lit);
        mats.push(materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * emissive,
            metallic: 0.95,
            perceptual_roughness: 0.15,
            reflectance: 0.8,
            ..default()
        }));
    }

    commands.insert_resource(CubeMaterials { materials: mats });
    commands.insert_resource(EmitterRegistry::default());
    commands.insert_resource(CubeCount::default());
}

/// Spawn cubes for each note-on event this frame.
pub fn spawn(
    mut commands: Commands,
    telemetry: Res<SynthTelemetry>,
    cube_mesh: Res<CubeMesh>,
    cube_materials: Res<CubeMaterials>,
    mut emitters: ResMut<EmitterRegistry>,
    mut cube_count: ResMut<CubeCount>,
) {
    for &event in &telemetry.pending_note_events {
        if cube_count.count >= MAX_CUBES {
            break;
        }

        let origin = emitters.position_for(event.instrument_id);
        let speed = 1.5 + (event.velocity as f32 / 127.0) * 4.0;
        let mat_idx = (event.category.as_u8() as usize).min(cube_materials.materials.len() - 1);
        let material = cube_materials.materials[mat_idx].clone();

        let spawn_count = CUBES_PER_NOTE.min(MAX_CUBES - cube_count.count);

        for i in 0..spawn_count {
            let theta = GOLDEN_ANGLE * i as f32;
            let phi = (1.0 - 2.0 * (i as f32 + 0.5) / spawn_count as f32).acos();

            let dir = Vec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos())
                .normalize_or_zero();

            // Vary speed by index
            let speed_var = speed * (0.7 + 0.6 * ((i * 7 + 3) % 10) as f32 / 10.0);

            // Vary initial rotation and spin based on note + index
            let seed = (event.midi_note as f32 * 0.1 + i as f32 * 0.7).sin();
            let spin = Vec3::new(
                seed * 4.0,
                (seed * 1.3).cos() * 3.0,
                (seed * 2.7).sin() * 5.0,
            );

            commands.spawn((
                Mesh3d(cube_mesh.0.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(origin).with_rotation(Quat::from_euler(
                    EulerRot::XYZ,
                    seed,
                    seed * 2.0,
                    seed * 0.5,
                )),
                InstrumentCube {
                    velocity: dir * speed_var,
                    spin,
                    life: LIFETIME,
                    max_life: LIFETIME,
                },
                EffectLayer(EffectId::InstrumentCubes),
            ));
        }

        cube_count.count += spawn_count;
    }
}

/// Update cube positions, rotation, shrink, and despawn.
#[allow(clippy::too_many_arguments)]
pub fn update(
    mut commands: Commands,
    time: Res<Time>,
    cube_materials: Res<CubeMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut last_policy_version: Local<u64>,
    mut query: Query<(Entity, &mut InstrumentCube, &mut Transform)>,
    mut cube_count: ResMut<CubeCount>,
) {
    let dt = time.delta_secs();

    if *last_policy_version != policy.version {
        let emissive = EMISSIVE_STRENGTH * policy.emissive_multiplier;
        for (cat, handle) in cube_materials.materials.iter().enumerate() {
            if let Some(material) = materials.get_mut(handle) {
                let (hue, base_sat, base_lit) = category_hsl(cat as u8);
                let sat = (base_sat + policy.saturation_offset).clamp(0.0, 1.0);
                let lit = (base_lit + policy.lightness_offset).clamp(0.0, 1.0);
                let color = Color::hsl(hue, sat, lit);
                material.base_color = color;
                material.emissive = LinearRgba::from(color) * emissive;
                material.metallic = policy.metallic;
                material.perceptual_roughness = policy.roughness;
            }
        }
        *last_policy_version = policy.version;
    }

    for (entity, mut cube, mut transform) in &mut query {
        cube.life -= dt;

        if cube.life <= 0.0 {
            commands.entity(entity).despawn();
            cube_count.count = cube_count.count.saturating_sub(1);
            continue;
        }

        // Apply velocity with gravity
        cube.velocity.y -= 2.5 * dt;
        transform.translation += cube.velocity * dt;

        // Tumble rotation
        let rot = Quat::from_euler(
            EulerRot::XYZ,
            cube.spin.x * dt,
            cube.spin.y * dt,
            cube.spin.z * dt,
        );
        transform.rotation *= rot;

        // Shrink over lifetime
        let t = cube.life / cube.max_life;
        transform.scale = Vec3::splat(t);
    }
}
