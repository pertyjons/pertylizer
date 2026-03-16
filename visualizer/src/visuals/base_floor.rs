//! Base Floor — A simple floor that can be selected as a terrain.
//!
//! A large, dark grey floor plane.

use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer};
use super::theme::FloorEntity;

#[derive(Component)]
pub struct BaseFloor;

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Use a very large Circle instead of a Plane3d (which is a square).
    // This prevents the corners from appearing as a "spinning black box" when the camera orbits.
    let mesh = meshes.add(Circle::new(200.0));

    // Floor color matches the default Neon theme
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.02, 0.02, 0.05),
        perceptual_roughness: 0.15,
        ..default()
    });

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, 0.0, 0.0)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        Visibility::Hidden,
        BaseFloor,
        FloorEntity,
        EffectLayer(EffectId::BaseFloor),
    ));
}

pub fn update(query: Query<&Transform, With<BaseFloor>>) {
    // Floor is completely static — immutable query to avoid triggering change detection
    let _ = query;
}
