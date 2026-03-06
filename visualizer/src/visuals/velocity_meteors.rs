//! Velocity Meteors — Spheres falling with size and brightness based on impact velocity.

use bevy::color::LinearRgba;
use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer};
use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;
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
    /// Base scale derived from velocity (preserved across frames).
    pub base_scale: f32,
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
    policy: Res<ThemeMaterialPolicy>,
) {
    commands.insert_resource(MeteorMesh(meshes.add(Sphere::new(1.0))));

    let sat = (0.9 + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (0.6 + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = 15.0 * policy.emissive_multiplier;

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
    for note_event in &telemetry.pending_note_events {
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
            Meteor {
                fallen: 0.0,
                base_scale: vel_scale * 2.0,
            },
            EffectLayer(EffectId::VelocityMeteors),
        ));
    }
}

/// Update falling and despawn when hitting bottom.
pub fn update(
    mut commands: Commands,
    time: Res<Time>,
    telemetry: Res<SynthTelemetry>,
    meteor_materials: Res<MeteorMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    policy: Res<ThemeMaterialPolicy>,
    mut last_policy_version: Local<u64>,
    mut last_hue_offset: Local<f32>,
    mut query: Query<(Entity, &mut Meteor, &mut Transform)>,
) {
    let dt = time.delta_secs();

    let hue_offset = telemetry_color::centroid_to_hue(telemetry.centroid_hz, &policy);
    if *last_policy_version != policy.version || (hue_offset - *last_hue_offset).abs() > 1.0 {
        let sat = (0.9 + policy.saturation_offset).clamp(0.0, 1.0);
        let lit = (0.6 + policy.lightness_offset).clamp(0.0, 1.0);
        let emissive = 15.0 * policy.emissive_multiplier;

        for (note, handle) in meteor_materials.materials.iter().enumerate() {
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

    for (entity, mut meteor, mut transform) in &mut query {
        let fall_amt = FALL_SPEED * dt;
        meteor.fallen += fall_amt;
        transform.translation.y -= fall_amt;

        // Shrink as it falls, like it's burning up
        let life_pct = (1.0 - (meteor.fallen / FALL_DISTANCE)).max(0.01);
        transform.scale = Vec3::splat((meteor.base_scale * life_pct).max(0.01));

        if meteor.fallen >= FALL_DISTANCE {
            commands.entity(entity).despawn();
        }
    }
}
