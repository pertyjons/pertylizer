//! Velocity Meteors — Spheres falling with size and brightness based on impact velocity.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer};
use crate::telemetry::SynthTelemetry;

/// How fast the meteors fall.
const FALL_SPEED: f32 = 25.0;

/// Distance before despawn.
const FALL_DISTANCE: f32 = 40.0;

/// Start height.
const START_HEIGHT: f32 = 25.0;

/// A falling meteor.
#[derive(Component)]
pub struct Meteor {
    /// Distance fallen so far.
    pub fallen: f32,
}

/// Cached mesh handle.
#[derive(Resource)]
pub struct MeteorMesh(Handle<Mesh>);

/// Shared materials to avoid asset churn.
#[derive(Resource)]
pub struct MeteorMaterials {
    materials: Vec<Handle<StandardMaterial>>,
}

/// Create the shared meteor mesh and materials.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(MeteorMesh(meshes.add(Sphere::new(1.0))));

    let mut shared_mats = Vec::with_capacity(128);
    for note in 0..128 {
        let hue = (note as f32 / 127.0) * 360.0;
        let color = Color::hsl(hue, 0.9, 0.6);
        shared_mats.push(materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * 15.0, // High emissive for bloom
            ..default()
        }));
    }
    commands.insert_resource(MeteorMaterials {
        materials: shared_mats,
    });
}

/// Spawn meteors on note-on.
pub fn spawn(
    mut commands: Commands,
    telemetry: Res<SynthTelemetry>,
    meteor_mesh: Res<MeteorMesh>,
    meteor_materials: Res<MeteorMaterials>,
) {
    if telemetry.note_age_frames >= 2 {
        return;
    }
    let Some(note_event) = telemetry.last_note_on else {
        return;
    };

    let note_idx = (note_event.midi_note as usize).min(127);

    // Scale by velocity (quieter notes are smaller)
    let vel_scale = (note_event.velocity as f32 / 127.0).max(0.1);

    // Spread X based on note pitch (low notes left, high notes right)
    let spread_x = ((note_event.midi_note as f32 / 127.0) * 2.0 - 1.0) * 20.0;

    // Randomize Z slightly for depth
    let spread_z = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as f32
        / 1_000_000_000.0)
        * 10.0
        - 10.0;

    let material = meteor_materials.materials[note_idx].clone();

    commands.spawn((
        Mesh3d(meteor_mesh.0.clone()),
        MeshMaterial3d(material),
        Transform::from_xyz(spread_x, START_HEIGHT, spread_z)
            .with_scale(Vec3::splat(vel_scale * 2.0)),
        Meteor { fallen: 0.0 },
        EffectLayer(EffectId::VelocityMeteors),
    ));
}

/// Update falling and despawn when hitting bottom.
pub fn update(
    mut commands: Commands,
    time: Res<Time>,
    mut query: Query<(Entity, &mut Meteor, &mut Transform)>,
) {
    let dt = time.delta_secs();

    for (entity, mut meteor, mut transform) in &mut query {
        let fall_amt = FALL_SPEED * dt;
        meteor.fallen += fall_amt;
        transform.translation.y -= fall_amt;

        // Shrink as it falls, like it's burning up
        let life_pct = (1.0 - (meteor.fallen / FALL_DISTANCE)).max(0.01);
        transform.scale = Vec3::splat(life_pct);

        if meteor.fallen >= FALL_DISTANCE {
            commands.entity(entity).despawn();
        }
    }
}
