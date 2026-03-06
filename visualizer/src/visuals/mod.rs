//! Visual systems driven by synth telemetry.

pub mod base_floor;
pub mod beat_pulse;
pub mod camera;
pub mod centroid_nebula;
pub mod chord_bloom;
pub mod cpu_overdrive;
pub mod debug_hud;
pub mod effects;
pub mod fft_terrain;
pub mod ferrofluid_tendrils;
pub mod fft_bars;
pub mod flux_supernova;
pub mod fractal_pulse;
pub mod harmonic_ribbons;
pub mod instrument_cubes;
pub mod neon_calligraphy;
pub mod note_tree;
pub mod particles;
pub mod phase_rings;
pub mod pulse_terrain;
pub mod reaction_diffusion;
pub mod rms_light;
pub mod spectral_cathedral;
pub mod spectral_origami;
pub mod spectral_waterfall;
pub mod telemetry_color;
pub mod theme;
pub mod velocity_meteors;
pub mod voronoi_shatter;
pub mod waiting_indicator;
pub mod waveform_ring;

use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::Hdr;
use bevy::ui::IsDefaultUiCamera;

/// System set for effect switching (runs before visual updates).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct EffectSwitch;

/// System set for theme updates (runs after effect switching, before visual updates).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct ThemeUpdate;

/// Plugin that registers all visual systems.
pub struct VisualsPlugin;

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .init_resource::<debug_hud::DebugHudState>()
            .init_resource::<beat_pulse::BeatPulseState>()
            .init_resource::<particles::ParticleCount>()
            .init_resource::<effects::EffectState>()
            .init_resource::<spectral_waterfall::WaterfallState>()
            .init_resource::<phase_rings::RingState>()
            .init_resource::<flux_supernova::SupernovaState>()
            .init_resource::<instrument_cubes::CubeCount>()
            .init_resource::<theme::ThemeState>()
            .init_resource::<theme::ThemeRegistry>()
            .init_resource::<theme::ThemeMaterialPolicy>()
            .init_resource::<theme::ThemeRuntime>()
            .init_resource::<camera::CameraState>()
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
                    voronoi_shatter::setup,
                    fft_terrain::setup,
                    reaction_diffusion::setup,
                    note_tree::setup,
                    waiting_indicator::setup,
                    debug_hud::setup,
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
                effects::init_visibility_once
                    .in_set(EffectSwitch)
                    .before(effects::input),
            )
            // Theme input → transition → apply must run after effect switch
            .add_systems(
                Update,
                (
                    theme::theme_input,
                    theme::theme_transition,
                    theme::apply_theme,
                )
                    .chain()
                    .in_set(ThemeUpdate)
                    .after(EffectSwitch),
            )
            .add_systems(
                Update,
                (
                    base_floor::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::BaseFloor,
                    )),
                    fft_bars::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::FftBars,
                    )),
                    waveform_ring::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::WaveformRing,
                    )),
                    spectral_waterfall::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::SpectralWaterfall,
                    )),
                    spectral_waterfall::update_materials.run_if(effects::effect_active_or_pending(
                        effects::EffectId::SpectralWaterfall,
                    )),
                    velocity_meteors::spawn
                        .run_if(effects::effect_active(effects::EffectId::VelocityMeteors)),
                    velocity_meteors::update,
                    phase_rings::spawn_and_update,
                    centroid_nebula::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::CentroidNebula,
                    )),
                    centroid_nebula::update_material.run_if(effects::effect_active_or_pending(
                        effects::EffectId::CentroidNebula,
                    )),
                    flux_supernova::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::FluxSupernova,
                    )),
                    cpu_overdrive::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::CpuOverdriveCore,
                    )),
                    fractal_pulse::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::FractalPulse,
                    )),
                )
                    .after(ThemeUpdate),
            )
            .add_systems(
                Update,
                (
                    spectral_cathedral::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::SpectralCathedral,
                    )),
                    harmonic_ribbons::spawn_and_update,
                    chord_bloom::spawn_and_update,
                    pulse_terrain::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::PulseTerrain,
                    )),
                    pulse_terrain::update_material.run_if(effects::effect_active_or_pending(
                        effects::EffectId::PulseTerrain,
                    )),
                    spectral_origami::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::SpectralOrigami,
                    )),
                    spectral_origami::update_material.run_if(effects::effect_active_or_pending(
                        effects::EffectId::SpectralOrigami,
                    )),
                    ferrofluid_tendrils::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::FerrofluidTendrils,
                    )),
                    ferrofluid_tendrils::update_material.run_if(effects::effect_active_or_pending(
                        effects::EffectId::FerrofluidTendrils,
                    )),
                    neon_calligraphy::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::NeonCalligraphy,
                    )),
                )
                    .after(ThemeUpdate),
            )
            .add_systems(
                Update,
                (
                    voronoi_shatter::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::VoronoiShatter,
                    )),
                    voronoi_shatter::update_material.run_if(effects::effect_active_or_pending(
                        effects::EffectId::VoronoiShatter,
                    )),
                    fft_terrain::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::FftTerrain,
                    )),
                    reaction_diffusion::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::ReactionDiffusion,
                    )),
                    note_tree::update.run_if(effects::effect_active_or_pending(
                        effects::EffectId::NoteTree,
                    )),
                )
                    .after(ThemeUpdate),
            )
            .add_systems(
                Update,
                (
                    rms_light::update,
                    beat_pulse::update,
                    camera::input,
                    camera::update,
                    particles::spawn
                        .run_if(effects::effect_active(effects::EffectId::NoteParticles)),
                    particles::update,
                    instrument_cubes::spawn
                        .run_if(effects::effect_active(effects::EffectId::InstrumentCubes)),
                    instrument_cubes::update,
                    waiting_indicator::update,
                    debug_hud::toggle,
                    debug_hud::update,
                    debug_hud::screenshot,
                    debug_hud::fullscreen,
                )
                    .after(ThemeUpdate),
            );
    }
}

/// Marker for the RMS-driven point light.
#[derive(Component)]
pub struct RmsLight;

/// Set up the base scene: ground plane, camera, lights.
///
/// Initial values match the default Neon theme.
fn setup_scene(mut commands: Commands) {
    // Ambient light (Neon theme defaults)
    commands.spawn(AmbientLight {
        color: Color::srgb(0.05, 0.02, 0.1),
        brightness: 30.0,
        ..default()
    });

    // RMS-driven point light (Neon theme defaults)
    commands.spawn((
        PointLight {
            intensity: 0.0,
            range: 40.0,
            color: Color::srgb(1.0, 0.9, 0.8),
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(0.0, 10.0, 0.0),
        RmsLight,
    ));

    // Soft rim light to give depth to silhouettes (Neon theme defaults)
    commands.spawn((
        PointLight {
            intensity: 40_000.0,
            range: 80.0,
            color: Color::srgb(0.2, 0.4, 1.0),
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(-20.0, 18.0, -15.0),
        theme::RimLight,
    ));

    // Camera with bloom post-processing (Neon theme defaults)
    // In Bevy 0.18+, we use a single camera for both 3D and UI to avoid render-pass conflicts.
    // HDR is now a standalone component (Hdr) instead of a field on Camera.
    commands.spawn((
        Camera3d::default(),
        Hdr,
        Transform::from_xyz(0.0, 12.0, 25.0).looking_at(Vec3::new(0.0, 2.0, 0.0), Vec3::Y),
        camera::OrbitCamera {
            radius: 25.0,
            angle: 0.0,
        },
        Bloom {
            intensity: 0.4,
            low_frequency_boost: 0.6,
            low_frequency_boost_curvature: 0.5,
            high_pass_frequency: 0.7,
            ..default()
        },
        IsDefaultUiCamera,
    ));
}
