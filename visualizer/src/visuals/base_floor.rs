//! Base Floor — A simple floor that can be selected as a terrain.
//!
//! A large, dark grey floor plane.

use bevy::prelude::*;

use super::effects::{EffectId, EffectLayer, EffectState};

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

    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.05, 0.05, 0.08),
        perceptual_roughness: 0.9,
        ..default()
    });

    commands.spawn((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::from_xyz(0.0, 0.0, 0.0)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        Visibility::Hidden,
        BaseFloor,
        EffectLayer(EffectId::BaseFloor),
    ));
}

pub fn update(effect_state: Res<EffectState>, mut query: Query<&mut Transform, With<BaseFloor>>) {
    if !effect_state.active.is_active(EffectId::BaseFloor) && effect_state.fade == 0.0 {
        return;
    }

    // Since the floor has no animation, we just optionally scale it or fade it if we want,
    // but a static floor doesn't even need to move. We leave this empty or apply fade scaling if needed.

    for _transform in &mut query {
        // Floor is completely static
    }
}
