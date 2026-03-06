//! Visual systems driven by synth telemetry.

pub mod base_floor;
pub mod beat_pulse;
pub mod camera;
pub mod centroid_nebula;
pub mod chord_bloom;
pub mod cpu_overdrive;
pub mod effects;
pub mod ferrofluid_tendrils;
pub mod fft_bars;
pub mod flux_supernova;
pub mod fractal_pulse;
pub mod harmonic_ribbons;
pub mod instrument_cubes;
pub mod neon_calligraphy;
pub mod particles;
pub mod phase_rings;
pub mod pulse_terrain;
pub mod rms_light;
pub mod spectral_cathedral;
pub mod spectral_origami;
pub mod spectral_waterfall;
pub mod velocity_meteors;
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
            .init_resource::<phase_rings::RingState>()
            .init_resource::<flux_supernova::SupernovaState>()
            .init_resource::<instrument_cubes::CubeCount>()
            .add_systems(
                Startup,
                (
                    setup_scene,
                    base_floor::setup,
                    fft_bars::setup,
                    particles::setup,
                    waveform_ring::setup,
                    spectral_waterfall::setup,
                    velocity_meteors::setup,
                    phase_rings::setup,
                    centroid_nebula::setup,
                    flux_supernova::setup,
                    cpu_overdrive::setup,
                    fractal_pulse::setup,
                ),
            )
            .add_systems(
                Startup,
                (
                    spectral_cathedral::setup,
                    harmonic_ribbons::setup,
                    chord_bloom::setup,
                    pulse_terrain::setup,
                    spectral_origami::setup,
                    ferrofluid_tendrils::setup,
                    neon_calligraphy::setup,
                    instrument_cubes::setup,
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
                    base_floor::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::BaseFloor,
                    )),
                    fft_bars::update
                        .run_if(effects::effect_active_or_fading(effects::EffectId::FftBars)),
                    waveform_ring::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::WaveformRing,
                    )),
                    spectral_waterfall::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::SpectralWaterfall,
                    )),
                    spectral_waterfall::update_materials.run_if(effects::effect_active_or_fading(
                        effects::EffectId::SpectralWaterfall,
                    )),
                    velocity_meteors::spawn
                        .run_if(effects::effect_active(effects::EffectId::VelocityMeteors)),
                    velocity_meteors::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::VelocityMeteors,
                    )),
                    phase_rings::spawn_and_update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::PhaseRings,
                    )),
                    centroid_nebula::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::CentroidNebula,
                    )),
                    centroid_nebula::update_material.run_if(effects::effect_active_or_fading(
                        effects::EffectId::CentroidNebula,
                    )),
                    flux_supernova::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::FluxSupernova,
                    )),
                    cpu_overdrive::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::CpuOverdriveCore,
                    )),
                    fractal_pulse::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::FractalPulse,
                    )),
                )
                    .after(EffectSwitch),
            )
            .add_systems(
                Update,
                (
                    spectral_cathedral::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::SpectralCathedral,
                    )),
                    harmonic_ribbons::spawn_and_update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::HarmonicRibbons,
                    )),
                    chord_bloom::spawn_and_update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::ChordBloom,
                    )),
                    pulse_terrain::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::PulseTerrain,
                    )),
                    pulse_terrain::update_material.run_if(effects::effect_active_or_fading(
                        effects::EffectId::PulseTerrain,
                    )),
                    spectral_origami::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::SpectralOrigami,
                    )),
                    spectral_origami::update_material.run_if(effects::effect_active_or_fading(
                        effects::EffectId::SpectralOrigami,
                    )),
                    ferrofluid_tendrils::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::FerrofluidTendrils,
                    )),
                    ferrofluid_tendrils::update_material.run_if(effects::effect_active_or_fading(
                        effects::EffectId::FerrofluidTendrils,
                    )),
                    neon_calligraphy::update.run_if(effects::effect_active_or_fading(
                        effects::EffectId::NeonCalligraphy,
                    )),
                )
                    .after(EffectSwitch),
            )
            .add_systems(
                Update,
                (
                    rms_light::update,
                    beat_pulse::update,
                    camera::orbit,
                    particles::spawn
                        .run_if(effects::effect_active(effects::EffectId::NoteParticles)),
                    particles::update,
                    instrument_cubes::spawn
                        .run_if(effects::effect_active(effects::EffectId::InstrumentCubes)),
                    instrument_cubes::update,
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
fn setup_scene(mut commands: Commands) {
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
            shadows_enabled: false,
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
