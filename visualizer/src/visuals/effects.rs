//! Effect rack — switchable visual layers with fade-through-black crossfade.
//!
//! Keyboard controls:
//! - Left/Right arrows: previous/next preset scene
//! - R: generate random scene
//!
//! Effects are grouped into 4 categories to prevent Z-fighting and visual clutter.

use bevy::color::LinearRgba;
use bevy::prelude::*;
use rand::Rng;
use rand::seq::SliceRandom;
use std::sync::OnceLock;

/// Available visual effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectId {
    // Note Particles (was originally hardcoded, we make it an effect now)
    NoteParticles,

    // Terrain / Base
    BaseFloor,
    FftBars,
    WaveformRing,
    SpectralWaterfall,
    PulseTerrain,
    SpectralOrigami,

    // Hero / Centerpiece
    CpuOverdriveCore,
    FluxSupernova,
    FractalPulse,
    FerrofluidTendrils,

    // Ambient / Sky
    CentroidNebula,
    SpectralCathedral,

    // Transients / Actions
    VelocityMeteors,
    PhaseRings,
    HarmonicRibbons,
    ChordBloom,
    NeonCalligraphy,
    InstrumentCubes,

    // Generative Geometry
    VoronoiShatter,
    FftTerrain,
    ReactionDiffusion,
    NoteTree,
}

impl EffectId {
    pub const TERRAIN: &[Self] = &[
        Self::BaseFloor,
        Self::FftBars,
        Self::WaveformRing,
        Self::SpectralWaterfall,
        Self::PulseTerrain,
        Self::SpectralOrigami,
        Self::PhaseRings,
        Self::VoronoiShatter,
        Self::FftTerrain,
    ];

    pub const HERO: &[Self] = &[
        Self::CpuOverdriveCore,
        Self::FluxSupernova,
        Self::FractalPulse,
        Self::FerrofluidTendrils,
        Self::NoteTree,
    ];

    pub const AMBIENT: &[Self] = &[Self::CentroidNebula, Self::SpectralCathedral, Self::ReactionDiffusion];

    pub const TRANSIENTS: &[Self] = &[
        Self::NoteParticles,
        Self::VelocityMeteors,
        Self::HarmonicRibbons,
        Self::ChordBloom,
        Self::NeonCalligraphy,
        Self::InstrumentCubes,
    ];
}

/// Defines a complete visual scene.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneConfig {
    pub terrain: Option<EffectId>,
    pub hero: Option<EffectId>,
    pub ambient: Option<EffectId>,
    pub transients: Vec<EffectId>,
}

impl SceneConfig {
    /// Returns true if the given effect is active in this scene.
    pub fn is_active(&self, effect: EffectId) -> bool {
        self.terrain == Some(effect)
            || self.hero == Some(effect)
            || self.ambient == Some(effect)
            || self.transients.contains(&effect)
    }
}

/// Run condition: returns true when the given effect is active in the current scene.
pub fn effect_active(effect: EffectId) -> impl Fn(Res<EffectState>) -> bool + Clone {
    move |state: Res<EffectState>| state.active.is_active(effect)
}

/// Run condition: returns true when the given effect is active OR pending during a crossfade.
pub fn effect_active_or_pending(effect: EffectId) -> impl Fn(Res<EffectState>) -> bool + Clone {
    move |state: Res<EffectState>| {
        state.active.is_active(effect)
            || state
                .pending
                .as_ref()
                .is_some_and(|scene| scene.is_active(effect))
    }
}

/// Marker component linking an entity to a specific effect layer.
#[derive(Component)]
pub struct EffectLayer(pub EffectId);

/// Global effect state — controls which scene is active and crossfade progress.
#[derive(Resource)]
pub struct EffectState {
    /// Currently visible scene.
    pub active: SceneConfig,
    /// Scene to switch to after fade-out completes.
    pending: Option<SceneConfig>,
    /// Fade multiplier: 1.0 = fully visible, 0.0 = black.
    pub fade: f32,
    /// True while fading out (toward 0), false while fading in (toward 1).
    fading_out: bool,
    /// Current preset index.
    preset_index: usize,
}

/// Crossfade speed (units per second).
const FADE_SPEED: f32 = 4.0;

/// Minimum fade change to trigger a material re-upload.
pub const FADE_EPSILON: f32 = 0.005;

impl Default for EffectState {
    fn default() -> Self {
        // Default to a sensible base scene
        let default_scene = SceneConfig {
            terrain: Some(EffectId::BaseFloor),
            hero: None,
            ambient: None,
            transients: vec![EffectId::NoteParticles, EffectId::InstrumentCubes],
        };

        Self {
            active: default_scene,
            pending: None,
            fade: 1.0,
            fading_out: false,
            preset_index: 0,
        }
    }
}

/// Returns a cached list of hand-crafted scenes.
fn presets() -> &'static [SceneConfig] {
    static PRESETS: OnceLock<Vec<SceneConfig>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        // 0: Classic Pertylizer
        vec![
            SceneConfig {
                terrain: Some(EffectId::SpectralWaterfall),
                hero: None,
                ambient: None,
                transients: vec![EffectId::NoteParticles],
            },
            // 1: The Matrix
            SceneConfig {
                terrain: Some(EffectId::PulseTerrain),
                hero: Some(EffectId::CpuOverdriveCore),
                ambient: Some(EffectId::CentroidNebula),
                transients: vec![EffectId::VelocityMeteors],
            },
            // 2: Sacred Geometry
            SceneConfig {
                terrain: Some(EffectId::SpectralOrigami),
                hero: Some(EffectId::FractalPulse),
                ambient: Some(EffectId::SpectralCathedral),
                transients: vec![EffectId::ChordBloom],
            },
            // 3: Magnetic Storm
            SceneConfig {
                terrain: Some(EffectId::WaveformRing),
                hero: Some(EffectId::FerrofluidTendrils),
                ambient: Some(EffectId::CentroidNebula),
                transients: vec![EffectId::PhaseRings, EffectId::HarmonicRibbons],
            },
            // 4: The Exploding Sun
            SceneConfig {
                terrain: Some(EffectId::FftBars),
                hero: Some(EffectId::FluxSupernova),
                ambient: None,
                transients: vec![EffectId::NeonCalligraphy, EffectId::NoteParticles],
            },
            // 5: Metallic Orchestra
            SceneConfig {
                terrain: Some(EffectId::BaseFloor),
                hero: Some(EffectId::FractalPulse),
                ambient: Some(EffectId::CentroidNebula),
                transients: vec![EffectId::InstrumentCubes],
            },
            // 6: Earthquake
            SceneConfig {
                terrain: Some(EffectId::VoronoiShatter),
                hero: Some(EffectId::FerrofluidTendrils),
                ambient: None,
                transients: vec![EffectId::VelocityMeteors, EffectId::NoteParticles],
            },
            // 7: Spectrum City
            SceneConfig {
                terrain: Some(EffectId::FftTerrain),
                hero: Some(EffectId::NoteTree),
                ambient: Some(EffectId::ReactionDiffusion),
                transients: vec![EffectId::ChordBloom],
            },
            // 8: Living Forest
            SceneConfig {
                terrain: Some(EffectId::PulseTerrain),
                hero: Some(EffectId::NoteTree),
                ambient: Some(EffectId::CentroidNebula),
                transients: vec![EffectId::HarmonicRibbons, EffectId::NoteParticles],
            },
        ]
    })
}

/// Generate a completely random scene using the slot rules.
fn generate_random_scene() -> SceneConfig {
    let mut rng = rand::thread_rng();

    // 80% chance to have a terrain
    let terrain = if rng.gen_bool(0.8) {
        Some(*EffectId::TERRAIN.choose(&mut rng).unwrap())
    } else {
        None
    };

    // 70% chance to have a hero
    let hero = if rng.gen_bool(0.7) {
        Some(*EffectId::HERO.choose(&mut rng).unwrap())
    } else {
        None
    };

    // 50% chance to have an ambient effect
    let ambient = if rng.gen_bool(0.5) {
        Some(*EffectId::AMBIENT.choose(&mut rng).unwrap())
    } else {
        None
    };

    // Pick 1 to 3 random transients
    let mut transients = Vec::new();
    let num_transients = rng.gen_range(1..=3);
    let mut available_transients = EffectId::TRANSIENTS.to_vec();
    available_transients.shuffle(&mut rng);
    for transient in available_transients.iter().take(num_transients) {
        transients.push(*transient);
    }

    SceneConfig {
        terrain,
        hero,
        ambient,
        transients,
    }
}

/// Handle keyboard input for scene switching.
pub fn input(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<EffectState>) {
    // Ignore input during crossfade
    if state.pending.is_some() {
        return;
    }

    let presets = presets();

    let mut next_scene = None;

    if keys.just_pressed(KeyCode::ArrowRight) {
        state.preset_index = (state.preset_index + 1) % presets.len();
        next_scene = Some(presets[state.preset_index].clone());
    } else if keys.just_pressed(KeyCode::ArrowLeft) {
        state.preset_index = if state.preset_index == 0 {
            presets.len() - 1
        } else {
            state.preset_index - 1
        };
        next_scene = Some(presets[state.preset_index].clone());
    } else if keys.just_pressed(KeyCode::KeyR) {
        next_scene = Some(generate_random_scene());
    }

    if let Some(target) = next_scene {
        state.pending = Some(target);
        state.fading_out = true;
    }
}

/// Update crossfade: fade out → swap visibility → fade in.
pub fn crossfade(
    time: Res<Time>,
    mut state: ResMut<EffectState>,
    mut query: Query<(&EffectLayer, &mut Visibility)>,
) {
    let dt = time.delta_secs();

    if state.fading_out {
        state.fade = (state.fade - FADE_SPEED * dt).max(0.0);
        if state.fade <= 0.0 {
            // Swap: hide old effects, show new effects
            if let Some(new_scene) = state.pending.take() {
                for (layer, mut vis) in &mut query {
                    // Turn on if it belongs to the new scene
                    if new_scene.is_active(layer.0) {
                        *vis = Visibility::Inherited;
                    }
                    // Turn off if it belonged to the old scene but not the new
                    else if state.active.is_active(layer.0) {
                        *vis = Visibility::Hidden;
                    }
                }
                state.active = new_scene;
            }
            state.fading_out = false;
        }
    } else if state.fade < 1.0 {
        state.fade = (state.fade + FADE_SPEED * dt).min(1.0);
    }
}

/// Initialize visibility once so the default scene is visible on startup.
pub fn init_visibility_once(
    mut initialized: Local<bool>,
    state: Res<EffectState>,
    mut query: Query<(&EffectLayer, &mut Visibility)>,
) {
    if *initialized {
        return;
    }

    for (layer, mut vis) in &mut query {
        *vis = if state.active.is_active(layer.0) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }

    *initialized = true;
}

/// Configuration for hue-bucketed shared materials.
pub struct HueMaterialConfig {
    pub hue_range: f32,
    pub saturation: f32,
    pub lightness: f32,
    pub emissive_strength: f32,
    /// When true, use nonlinear frequency→hue mapping (bass→red, mids→green, highs→blue)
    /// via `telemetry_color::band_frequency_hue` instead of linear hue distribution.
    pub frequency_mapped: bool,
}

use super::telemetry_color;
use super::theme::ThemeMaterialPolicy;

/// Create a set of hue-bucketed `StandardMaterial`s for use as shared materials.
///
/// The `ThemeMaterialPolicy` offsets are applied to saturation, lightness,
/// and emissive strength so that themes can shift the overall look.
/// `hue_offset` shifts the entire hue palette (e.g., from centroid mapping).
pub fn create_hue_materials(
    materials: &mut Assets<StandardMaterial>,
    num_buckets: usize,
    config: &HueMaterialConfig,
    policy: &ThemeMaterialPolicy,
) -> Vec<Handle<StandardMaterial>> {
    create_hue_materials_with_offset(materials, num_buckets, config, policy, 0.0)
}

/// Like [`create_hue_materials`] but with an explicit hue offset.
pub fn create_hue_materials_with_offset(
    materials: &mut Assets<StandardMaterial>,
    num_buckets: usize,
    config: &HueMaterialConfig,
    policy: &ThemeMaterialPolicy,
    hue_offset: f32,
) -> Vec<Handle<StandardMaterial>> {
    let sat = (config.saturation + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (config.lightness + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = config.emissive_strength * policy.emissive_multiplier;
    let metallic = policy.metallic;
    let roughness = policy.roughness;

    let mut result = Vec::with_capacity(num_buckets);
    for bucket in 0..num_buckets {
        #[allow(clippy::cast_precision_loss)]
        let base_hue = if config.frequency_mapped {
            telemetry_color::band_frequency_hue(bucket as f32 / num_buckets as f32)
        } else {
            (bucket as f32 / num_buckets as f32) * config.hue_range
        };
        let hue = (base_hue + hue_offset) % 360.0;
        let color = Color::hsl(hue, sat, lit);
        result.push(materials.add(StandardMaterial {
            base_color: color,
            emissive: LinearRgba::from(color) * emissive,
            metallic,
            perceptual_roughness: roughness,
            ..default()
        }));
    }
    result
}

/// Tracked state for `update_hue_materials_for_fade` to avoid redundant GPU re-uploads.
pub struct HueMaterialTracker {
    pub last_fade: f32,
    pub last_hue_offset: f32,
    pub last_emissive_boost: f32,
    pub last_policy_version: u64,
}

impl Default for HueMaterialTracker {
    fn default() -> Self {
        Self {
            last_fade: 0.0,
            last_hue_offset: 0.0,
            last_emissive_boost: 1.0,
            last_policy_version: 0,
        }
    }
}

/// Update hue-bucketed shared materials for crossfade, telemetry-driven hue shifts,
/// and emissive boost (e.g., from spectral flux).
///
/// `hue_offset` shifts the entire palette (e.g., centroid-driven).
/// `emissive_boost` multiplies emissive (1.0 = no change, >1.0 = brighter on flux spikes).
#[allow(clippy::too_many_arguments)]
pub fn update_hue_materials_for_fade(
    materials: &mut Assets<StandardMaterial>,
    handles: &[Handle<StandardMaterial>],
    config: &HueMaterialConfig,
    policy: &ThemeMaterialPolicy,
    fade: f32,
    hue_offset: f32,
    emissive_boost: f32,
    tracker: &mut HueMaterialTracker,
) {
    if (fade - tracker.last_fade).abs() < FADE_EPSILON
        && (hue_offset - tracker.last_hue_offset).abs() < 1.0
        && (emissive_boost - tracker.last_emissive_boost).abs() < 0.05
        && tracker.last_policy_version == policy.version
    {
        return;
    }

    let sat = (config.saturation + policy.saturation_offset).clamp(0.0, 1.0);
    let lit = (config.lightness + policy.lightness_offset).clamp(0.0, 1.0);
    let emissive = config.emissive_strength * policy.emissive_multiplier * emissive_boost;
    let metallic = policy.metallic;
    let roughness = policy.roughness;

    let num_buckets = handles.len();
    for (bucket, handle) in handles.iter().enumerate() {
        if let Some(material) = materials.get_mut(handle) {
            #[allow(clippy::cast_precision_loss)]
            let base_hue = if config.frequency_mapped {
                telemetry_color::band_frequency_hue(bucket as f32 / num_buckets as f32)
            } else {
                (bucket as f32 / num_buckets as f32) * config.hue_range
            };
            let hue = (base_hue + hue_offset) % 360.0;
            let color = Color::hsl(hue, sat, lit * fade);
            material.base_color = color;
            material.emissive = LinearRgba::from(color) * emissive * fade;
            material.metallic = metallic;
            material.perceptual_roughness = roughness;
        }
    }
    tracker.last_fade = fade;
    tracker.last_hue_offset = hue_offset;
    tracker.last_emissive_boost = emissive_boost;
    tracker.last_policy_version = policy.version;
}
