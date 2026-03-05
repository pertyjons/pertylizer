//! Visual systems driven by synth telemetry.

pub mod camera;
pub mod fft_bars;
pub mod note_flash;
pub mod rms_light;

use bevy::prelude::*;

/// Plugin that registers all visual systems.
pub struct VisualsPlugin;

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_scene, fft_bars::setup, note_flash::setup))
            .add_systems(
                Update,
                (
                    fft_bars::update,
                    rms_light::update,
                    note_flash::update,
                    camera::orbit,
                ),
            );
    }
}

/// Marker for the RMS-driven point light.
#[derive(Component)]
pub struct RmsLight;

/// Set up the base scene: ground plane, camera, lights.
fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Ground plane
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(50.0)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.05, 0.05, 0.08),
            perceptual_roughness: 0.9,
            ..default()
        })),
    ));

    // Ambient light
    commands.spawn(AmbientLight {
        color: Color::srgb(0.3, 0.3, 0.4),
        brightness: 50.0,
        ..default()
    });

    // RMS-driven point light
    commands.spawn((
        PointLight {
            intensity: 0.0,
            range: 40.0,
            color: Color::srgb(1.0, 0.8, 0.5),
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(0.0, 10.0, 0.0),
        RmsLight,
    ));

    // Camera
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 12.0, 25.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
        camera::OrbitCamera {
            radius: 25.0,
            speed: 0.1,
            angle: 0.0,
        },
    ));
}
