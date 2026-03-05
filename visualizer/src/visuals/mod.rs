//! Visual systems driven by synth telemetry.

pub mod beat_pulse;
pub mod camera;
pub mod fft_bars;
pub mod note_flash;
pub mod rms_light;

use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;

/// Plugin that registers all visual systems.
pub struct VisualsPlugin;

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<beat_pulse::BeatPulseState>()
            .add_systems(Startup, (setup_scene, fft_bars::setup, note_flash::setup))
            .add_systems(
                Update,
                (
                    fft_bars::update,
                    rms_light::update,
                    note_flash::update,
                    beat_pulse::update,
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
    // Ground plane (with BeatPulseGround marker for beat-synced glow)
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(50.0)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.05, 0.05, 0.08),
            perceptual_roughness: 0.9,
            ..default()
        })),
        beat_pulse::BeatPulseGround,
    ));

    // Ambient light
    commands.spawn(AmbientLight {
        color: Color::srgb(0.3, 0.3, 0.4),
        brightness: beat_pulse::BASE_AMBIENT_BRIGHTNESS,
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

    // Camera with bloom post-processing
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 12.0, 25.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
        camera::OrbitCamera {
            radius: 25.0,
            speed: 0.1,
            angle: 0.0,
        },
        Bloom {
            intensity: 0.3,
            low_frequency_boost: 0.5,
            low_frequency_boost_curvature: 0.5,
            high_pass_frequency: 0.7,
            ..default()
        },
    ));
}
