//! Visual systems driven by synth telemetry.

pub mod beat_pulse;
pub mod camera;
pub mod effects;
pub mod fft_bars;
pub mod note_flash;
pub mod particles;
pub mod rms_light;
pub mod spectral_waterfall;
pub mod waiting_indicator;
pub mod waveform_ring;

use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;

/// System set for effect switching (runs before visual updates).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct EffectSwitch;

/// Plugin that registers all visual systems.
pub struct VisualsPlugin;

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<beat_pulse::BeatPulseState>()
            .init_resource::<particles::ParticleCount>()
            .init_resource::<effects::EffectState>()
            .init_resource::<spectral_waterfall::WaterfallState>()
            .add_systems(
                Startup,
                (
                    setup_scene,
                    fft_bars::setup,
                    note_flash::setup,
                    particles::setup,
                    waveform_ring::setup,
                    spectral_waterfall::setup,
                    waiting_indicator::setup,
                ),
            )
            // Effect input → crossfade must run before effect updates
            .add_systems(
                Update,
                (effects::input, effects::crossfade)
                    .chain()
                    .in_set(EffectSwitch),
            )
            .add_systems(
                Update,
                (
                    fft_bars::update,
                    waveform_ring::update,
                    spectral_waterfall::update,
                    rms_light::update,
                    note_flash::update,
                    beat_pulse::update,
                    camera::orbit,
                    particles::spawn,
                    particles::update,
                    waiting_indicator::update,
                )
                    .after(EffectSwitch),
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
