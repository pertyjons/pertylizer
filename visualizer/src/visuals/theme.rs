//! Theme system — swaps palette, lighting, bloom, and material policy across all effects.
//!
//! Keyboard controls:
//! - `T`: next theme
//! - `Shift+T`: previous theme

use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use std::collections::HashMap;

use super::RmsLight;

// ---------------------------------------------------------------------------
// ThemeId
// ---------------------------------------------------------------------------

/// Identifies one of the built-in visual themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeId {
    #[default]
    Neon,
    Metal,
    Glass,
    Space,
    Synthwave,
    Ember,
    Arctic,
    Void,
}

impl ThemeId {
    /// All themes in presentation order.
    pub const ALL: &[Self] = &[
        Self::Neon,
        Self::Metal,
        Self::Glass,
        Self::Space,
        Self::Synthwave,
        Self::Ember,
        Self::Arctic,
        Self::Void,
    ];

    /// Human-readable name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Neon => "Neon",
            Self::Metal => "Metal",
            Self::Glass => "Glass",
            Self::Space => "Space",
            Self::Synthwave => "Synthwave",
            Self::Ember => "Ember",
            Self::Arctic => "Arctic",
            Self::Void => "Void",
        }
    }

    /// Cycle to the next theme, wrapping around.
    #[must_use]
    pub fn next(self) -> Self {
        let all = Self::ALL;
        let idx = all.iter().position(|&t| t == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }

    /// Cycle to the previous theme, wrapping around.
    #[must_use]
    pub fn prev(self) -> Self {
        let all = Self::ALL;
        let idx = all.iter().position(|&t| t == self).unwrap_or(0);
        if idx == 0 {
            all[all.len() - 1]
        } else {
            all[idx - 1]
        }
    }
}

// ---------------------------------------------------------------------------
// ThemeConfig
// ---------------------------------------------------------------------------

/// Complete visual configuration for a theme.
#[allow(dead_code)]
pub struct ThemeConfig {
    pub name: &'static str,

    // Lighting
    pub ambient_color: Color,
    pub ambient_brightness: f32,
    pub key_light_color: Color,
    pub key_light_intensity: f32,
    pub rim_light_color: Color,
    pub rim_light_intensity: f32,

    // Bloom
    pub bloom_intensity: f32,
    pub bloom_low_freq_boost: f32,

    // Materials
    pub emissive_multiplier: f32,
    pub saturation_offset: f32,
    pub lightness_offset: f32,
    pub metallic: f32,
    pub roughness: f32,

    // Telemetry-reactive parameters
    /// Hue when spectral centroid is low (bass-heavy).
    pub centroid_hue_low: f32,
    /// Hue when spectral centroid is high (bright/treble).
    pub centroid_hue_high: f32,
    /// Hue for flux spike accents.
    pub flux_burst_hue: f32,
    /// How much flux amplifies emissive (1.0 = normal).
    pub flux_intensity_scale: f32,
    /// How much beat phase modulates brightness (0.0-1.0).
    pub beat_pulse_strength: f32,
    /// Hue for transient/peak flashes.
    pub peak_flash_hue: f32,
    /// How much RMS scales emissive (0.5-2.0).
    pub rms_emissive_scale: f32,

    // Environment
    pub floor_color: Color,
    pub floor_metallic: f32,
    pub floor_roughness: f32,
}

// ---------------------------------------------------------------------------
// ThemeRegistry
// ---------------------------------------------------------------------------

/// Stores all theme configurations keyed by [`ThemeId`].
#[derive(Resource)]
pub struct ThemeRegistry {
    themes: HashMap<ThemeId, ThemeConfig>,
}

impl ThemeRegistry {
    /// Look up a theme config. Panics only during init if a theme is missing.
    #[must_use]
    pub fn get(&self, id: ThemeId) -> &ThemeConfig {
        self.themes
            .get(&id)
            .unwrap_or_else(|| self.themes.get(&ThemeId::Neon).expect("Neon theme missing"))
    }
}

impl Default for ThemeRegistry {
    fn default() -> Self {
        let mut themes = HashMap::new();

        // 1. Neon (default)
        themes.insert(
            ThemeId::Neon,
            ThemeConfig {
                name: "Neon",
                ambient_color: Color::srgb(0.05, 0.02, 0.1),
                ambient_brightness: 1.5,
                key_light_color: Color::srgb(1.0, 0.9, 0.8),
                key_light_intensity: 35_000.0,
                rim_light_color: Color::srgb(0.2, 0.4, 1.0),
                rim_light_intensity: 8_000.0,
                bloom_intensity: 0.2,
                bloom_low_freq_boost: 0.3,
                emissive_multiplier: 0.5,
                saturation_offset: 0.1,
                lightness_offset: 0.0,
                metallic: 0.0,
                roughness: 0.5,
                centroid_hue_low: 270.0,
                centroid_hue_high: 360.0,
                flux_burst_hue: 60.0,
                flux_intensity_scale: 1.5,
                beat_pulse_strength: 0.8,
                peak_flash_hue: 120.0,
                rms_emissive_scale: 1.5,
                floor_color: Color::srgb(0.02, 0.02, 0.05),
                floor_metallic: 0.0,
                floor_roughness: 0.15,
            },
        );

        // 2. Metal
        themes.insert(
            ThemeId::Metal,
            ThemeConfig {
                name: "Metal",
                ambient_color: Color::srgb(0.15, 0.15, 0.18),
                ambient_brightness: 4.0,
                key_light_color: Color::srgb(0.9, 0.95, 1.0),
                key_light_intensity: 30_000.0,
                rim_light_color: Color::srgb(0.7, 0.7, 0.8),
                rim_light_intensity: 5_000.0,
                bloom_intensity: 0.08,
                bloom_low_freq_boost: 0.15,
                emissive_multiplier: 0.15,
                saturation_offset: -0.3,
                lightness_offset: 0.1,
                metallic: 0.9,
                roughness: 0.2,
                centroid_hue_low: 200.0,
                centroid_hue_high: 260.0,
                flux_burst_hue: 40.0,
                flux_intensity_scale: 0.8,
                beat_pulse_strength: 0.3,
                peak_flash_hue: 0.0,
                rms_emissive_scale: 0.8,
                floor_color: Color::srgb(0.08, 0.08, 0.1),
                floor_metallic: 0.9,
                floor_roughness: 0.25,
            },
        );

        // 3. Glass
        themes.insert(
            ThemeId::Glass,
            ThemeConfig {
                name: "Glass",
                ambient_color: Color::srgb(0.2, 0.25, 0.3),
                ambient_brightness: 5.0,
                key_light_color: Color::srgb(1.0, 1.0, 1.0),
                key_light_intensity: 20_000.0,
                rim_light_color: Color::srgb(0.5, 0.8, 0.9),
                rim_light_intensity: 3_000.0,
                bloom_intensity: 0.12,
                bloom_low_freq_boost: 0.2,
                emissive_multiplier: 0.25,
                saturation_offset: -0.1,
                lightness_offset: 0.15,
                metallic: 0.1,
                roughness: 0.1,
                centroid_hue_low: 180.0,
                centroid_hue_high: 300.0,
                flux_burst_hue: 200.0,
                flux_intensity_scale: 1.0,
                beat_pulse_strength: 0.5,
                peak_flash_hue: 180.0,
                rms_emissive_scale: 1.0,
                floor_color: Color::srgb(0.05, 0.08, 0.1),
                floor_metallic: 0.05,
                floor_roughness: 0.02,
            },
        );

        // 4. Space
        themes.insert(
            ThemeId::Space,
            ThemeConfig {
                name: "Space",
                ambient_color: Color::srgb(0.05, 0.02, 0.08),
                ambient_brightness: 0.8,
                key_light_color: Color::srgb(0.6, 0.7, 1.0),
                key_light_intensity: 45_000.0,
                rim_light_color: Color::srgb(0.5, 0.2, 0.8),
                rim_light_intensity: 7_000.0,
                bloom_intensity: 0.18,
                bloom_low_freq_boost: 0.25,
                emissive_multiplier: 0.6,
                saturation_offset: 0.0,
                lightness_offset: -0.05,
                metallic: 0.0,
                roughness: 0.5,
                centroid_hue_low: 220.0,
                centroid_hue_high: 280.0,
                flux_burst_hue: 270.0,
                flux_intensity_scale: 1.2,
                beat_pulse_strength: 0.6,
                peak_flash_hue: 60.0,
                rms_emissive_scale: 1.2,
                floor_color: Color::srgb(0.01, 0.01, 0.02),
                floor_metallic: 0.0,
                floor_roughness: 0.4,
            },
        );

        // 5. Synthwave
        themes.insert(
            ThemeId::Synthwave,
            ThemeConfig {
                name: "Synthwave",
                ambient_color: Color::srgb(0.1, 0.02, 0.08),
                ambient_brightness: 2.0,
                key_light_color: Color::srgb(1.0, 0.2, 0.6),
                key_light_intensity: 35_000.0,
                rim_light_color: Color::srgb(0.0, 0.8, 1.0),
                rim_light_intensity: 6_000.0,
                bloom_intensity: 0.22,
                bloom_low_freq_boost: 0.35,
                emissive_multiplier: 0.4,
                saturation_offset: 0.15,
                lightness_offset: 0.05,
                metallic: 0.0,
                roughness: 0.5,
                centroid_hue_low: 300.0,
                centroid_hue_high: 360.0,
                flux_burst_hue: 180.0,
                flux_intensity_scale: 1.4,
                beat_pulse_strength: 0.9,
                peak_flash_hue: 300.0,
                rms_emissive_scale: 1.4,
                floor_color: Color::srgb(0.03, 0.01, 0.06),
                floor_metallic: 0.1,
                floor_roughness: 0.1,
            },
        );

        // 6. Ember
        themes.insert(
            ThemeId::Ember,
            ThemeConfig {
                name: "Ember",
                ambient_color: Color::srgb(0.08, 0.03, 0.01),
                ambient_brightness: 3.0,
                key_light_color: Color::srgb(1.0, 0.6, 0.2),
                key_light_intensity: 30_000.0,
                rim_light_color: Color::srgb(0.8, 0.1, 0.05),
                rim_light_intensity: 4_000.0,
                bloom_intensity: 0.18,
                bloom_low_freq_boost: 0.25,
                emissive_multiplier: 0.35,
                saturation_offset: 0.0,
                lightness_offset: -0.05,
                metallic: 0.0,
                roughness: 0.5,
                centroid_hue_low: 0.0,
                centroid_hue_high: 60.0,
                flux_burst_hue: 30.0,
                flux_intensity_scale: 1.6,
                beat_pulse_strength: 0.7,
                peak_flash_hue: 45.0,
                rms_emissive_scale: 1.6,
                floor_color: Color::srgb(0.04, 0.03, 0.02),
                floor_metallic: 0.0,
                floor_roughness: 0.6,
            },
        );

        // 7. Arctic
        themes.insert(
            ThemeId::Arctic,
            ThemeConfig {
                name: "Arctic",
                ambient_color: Color::srgb(0.2, 0.25, 0.35),
                ambient_brightness: 6.0,
                key_light_color: Color::srgb(0.85, 0.9, 1.0),
                key_light_intensity: 25_000.0,
                rim_light_color: Color::srgb(0.4, 0.6, 0.9),
                rim_light_intensity: 2_000.0,
                bloom_intensity: 0.1,
                bloom_low_freq_boost: 0.15,
                emissive_multiplier: 0.2,
                saturation_offset: -0.2,
                lightness_offset: 0.2,
                metallic: 0.0,
                roughness: 0.5,
                centroid_hue_low: 180.0,
                centroid_hue_high: 240.0,
                flux_burst_hue: 200.0,
                flux_intensity_scale: 0.7,
                beat_pulse_strength: 0.4,
                peak_flash_hue: 210.0,
                rms_emissive_scale: 0.7,
                floor_color: Color::srgb(0.15, 0.18, 0.22),
                floor_metallic: 0.05,
                floor_roughness: 0.08,
            },
        );

        // 8. Void
        themes.insert(
            ThemeId::Void,
            ThemeConfig {
                name: "Void",
                ambient_color: Color::srgb(0.0, 0.0, 0.0),
                ambient_brightness: 0.3,
                key_light_color: Color::srgb(1.0, 1.0, 1.0),
                key_light_intensity: 50_000.0,
                rim_light_color: Color::srgb(0.0, 0.0, 0.0),
                rim_light_intensity: 0.0,
                bloom_intensity: 0.25,
                bloom_low_freq_boost: 0.4,
                emissive_multiplier: 0.8,
                saturation_offset: -0.6,
                lightness_offset: 0.0,
                metallic: 0.0,
                roughness: 0.5,
                centroid_hue_low: 0.0,
                centroid_hue_high: 0.0,
                flux_burst_hue: 0.0,
                flux_intensity_scale: 2.0,
                beat_pulse_strength: 0.2,
                peak_flash_hue: 0.0,
                rms_emissive_scale: 2.0,
                floor_color: Color::srgb(0.0, 0.0, 0.0),
                floor_metallic: 0.0,
                floor_roughness: 0.95,
            },
        );

        Self { themes }
    }
}

// ---------------------------------------------------------------------------
// ThemeState
// ---------------------------------------------------------------------------

/// Tracks the active theme and transition progress.
#[derive(Resource)]
pub struct ThemeState {
    pub active: ThemeId,
    pending: Option<ThemeId>,
    /// 0.0 = old theme fully applied, 1.0 = new theme fully applied.
    pub transition: f32,
    transitioning: bool,
}

impl Default for ThemeState {
    fn default() -> Self {
        Self {
            active: ThemeId::default(),
            pending: None,
            transition: 1.0,
            transitioning: false,
        }
    }
}

// ---------------------------------------------------------------------------
// ThemeMaterialPolicy
// ---------------------------------------------------------------------------

/// Current material adjustments derived from the active theme.
/// Effects that create hue-bucketed materials should consult this resource.
#[derive(Resource)]
pub struct ThemeMaterialPolicy {
    pub emissive_multiplier: f32,
    pub saturation_offset: f32,
    pub lightness_offset: f32,
    pub metallic: f32,
    pub roughness: f32,

    // Telemetry-reactive parameters (lerped during transitions)
    pub centroid_hue_low: f32,
    pub centroid_hue_high: f32,
    pub flux_burst_hue: f32,
    pub flux_intensity_scale: f32,
    pub beat_pulse_strength: f32,
    pub peak_flash_hue: f32,
    pub rms_emissive_scale: f32,

    pub version: u64,
}

impl Default for ThemeMaterialPolicy {
    fn default() -> Self {
        // Matches the Neon theme defaults
        Self {
            emissive_multiplier: 0.5,
            saturation_offset: 0.1,
            lightness_offset: 0.0,
            metallic: 0.0,
            roughness: 0.5,
            centroid_hue_low: 270.0,
            centroid_hue_high: 360.0,
            flux_burst_hue: 60.0,
            flux_intensity_scale: 1.5,
            beat_pulse_strength: 0.8,
            peak_flash_hue: 120.0,
            rms_emissive_scale: 1.5,
            version: 0,
        }
    }
}

impl ThemeMaterialPolicy {
    /// Update values and bump version if anything changed meaningfully.
    #[allow(clippy::too_many_arguments)]
    fn update_from_config(&mut self, cfg: &ThemePolicySnapshot) {
        const EPS: f32 = 0.0001;
        if (self.emissive_multiplier - cfg.emissive_multiplier).abs() > EPS
            || (self.saturation_offset - cfg.saturation_offset).abs() > EPS
            || (self.lightness_offset - cfg.lightness_offset).abs() > EPS
            || (self.metallic - cfg.metallic).abs() > EPS
            || (self.roughness - cfg.roughness).abs() > EPS
            || (self.centroid_hue_low - cfg.centroid_hue_low).abs() > EPS
            || (self.centroid_hue_high - cfg.centroid_hue_high).abs() > EPS
            || (self.flux_burst_hue - cfg.flux_burst_hue).abs() > EPS
            || (self.flux_intensity_scale - cfg.flux_intensity_scale).abs() > EPS
            || (self.beat_pulse_strength - cfg.beat_pulse_strength).abs() > EPS
            || (self.peak_flash_hue - cfg.peak_flash_hue).abs() > EPS
            || (self.rms_emissive_scale - cfg.rms_emissive_scale).abs() > EPS
        {
            self.emissive_multiplier = cfg.emissive_multiplier;
            self.saturation_offset = cfg.saturation_offset;
            self.lightness_offset = cfg.lightness_offset;
            self.metallic = cfg.metallic;
            self.roughness = cfg.roughness;
            self.centroid_hue_low = cfg.centroid_hue_low;
            self.centroid_hue_high = cfg.centroid_hue_high;
            self.flux_burst_hue = cfg.flux_burst_hue;
            self.flux_intensity_scale = cfg.flux_intensity_scale;
            self.beat_pulse_strength = cfg.beat_pulse_strength;
            self.peak_flash_hue = cfg.peak_flash_hue;
            self.rms_emissive_scale = cfg.rms_emissive_scale;
            self.version = self.version.wrapping_add(1);
        }
    }
}

/// Intermediate snapshot for computing lerped policy values.
struct ThemePolicySnapshot {
    emissive_multiplier: f32,
    saturation_offset: f32,
    lightness_offset: f32,
    metallic: f32,
    roughness: f32,
    centroid_hue_low: f32,
    centroid_hue_high: f32,
    flux_burst_hue: f32,
    flux_intensity_scale: f32,
    beat_pulse_strength: f32,
    peak_flash_hue: f32,
    rms_emissive_scale: f32,
}

impl ThemePolicySnapshot {
    fn from_config(cfg: &ThemeConfig) -> Self {
        Self {
            emissive_multiplier: cfg.emissive_multiplier,
            saturation_offset: cfg.saturation_offset,
            lightness_offset: cfg.lightness_offset,
            metallic: cfg.metallic.clamp(0.0, 1.0),
            roughness: cfg.roughness.clamp(0.0, 1.0),
            centroid_hue_low: cfg.centroid_hue_low,
            centroid_hue_high: cfg.centroid_hue_high,
            flux_burst_hue: cfg.flux_burst_hue,
            flux_intensity_scale: cfg.flux_intensity_scale,
            beat_pulse_strength: cfg.beat_pulse_strength,
            peak_flash_hue: cfg.peak_flash_hue,
            rms_emissive_scale: cfg.rms_emissive_scale,
        }
    }

    fn lerp(a: &ThemeConfig, b: &ThemeConfig, t: f32) -> Self {
        Self {
            emissive_multiplier: lerp_f32(a.emissive_multiplier, b.emissive_multiplier, t),
            saturation_offset: lerp_f32(a.saturation_offset, b.saturation_offset, t),
            lightness_offset: lerp_f32(a.lightness_offset, b.lightness_offset, t),
            metallic: lerp_f32(a.metallic, b.metallic, t).clamp(0.0, 1.0),
            roughness: lerp_f32(a.roughness, b.roughness, t).clamp(0.0, 1.0),
            centroid_hue_low: lerp_f32(a.centroid_hue_low, b.centroid_hue_low, t),
            centroid_hue_high: lerp_f32(a.centroid_hue_high, b.centroid_hue_high, t),
            flux_burst_hue: lerp_f32(a.flux_burst_hue, b.flux_burst_hue, t),
            flux_intensity_scale: lerp_f32(a.flux_intensity_scale, b.flux_intensity_scale, t),
            beat_pulse_strength: lerp_f32(a.beat_pulse_strength, b.beat_pulse_strength, t),
            peak_flash_hue: lerp_f32(a.peak_flash_hue, b.peak_flash_hue, t),
            rms_emissive_scale: lerp_f32(a.rms_emissive_scale, b.rms_emissive_scale, t),
        }
    }
}

// ---------------------------------------------------------------------------
// ThemeRuntime
// ---------------------------------------------------------------------------

/// Runtime theme values (lerped during transitions) for systems that need them per-frame.
#[derive(Resource)]
pub struct ThemeRuntime {
    pub ambient_brightness: f32,
    pub key_light_intensity: f32,
}

impl Default for ThemeRuntime {
    fn default() -> Self {
        // Matches Neon defaults
        Self {
            ambient_brightness: 1.5,
            key_light_intensity: 35_000.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Marker components
// ---------------------------------------------------------------------------

/// Marker for the rim (fill) light so the theme system can find it.
#[derive(Component)]
pub struct RimLight;

/// Marker for the floor entity so the theme system can update its material.
#[derive(Component)]
pub struct FloorEntity;

// ---------------------------------------------------------------------------
// Transition speed
// ---------------------------------------------------------------------------

/// Transition speed in units per second.
const TRANSITION_SPEED: f32 = 3.0;

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Handle keyboard input for theme switching.
pub fn theme_input(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<ThemeState>) {
    // Ignore input during a transition
    if state.transitioning {
        return;
    }

    if keys.just_pressed(KeyCode::KeyT) {
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        let next = if shift {
            state.active.prev()
        } else {
            state.active.next()
        };
        state.pending = Some(next);
        state.transitioning = true;
        state.transition = 0.0;
        info!("Theme: {} -> {}", state.active.name(), next.name());
    }
}

/// Advance the theme transition timer.
pub fn theme_transition(time: Res<Time>, mut state: ResMut<ThemeState>) {
    if !state.transitioning {
        return;
    }

    state.transition = (state.transition + TRANSITION_SPEED * time.delta_secs()).min(1.0);

    if state.transition >= 1.0 {
        if let Some(next) = state.pending.take() {
            state.active = next;
        }
        state.transitioning = false;
    }
}

/// Apply theme properties to the scene (ambient light, key light, rim light, bloom, floor, material policy).
#[allow(clippy::too_many_arguments)]
pub fn apply_theme(
    state: Res<ThemeState>,
    registry: Res<ThemeRegistry>,
    mut policy: ResMut<ThemeMaterialPolicy>,
    mut runtime: ResMut<ThemeRuntime>,
    mut ambient_query: Query<&mut AmbientLight>,
    mut key_light_query: Query<&mut PointLight, (With<RmsLight>, Without<RimLight>)>,
    mut rim_light_query: Query<&mut PointLight, (With<RimLight>, Without<RmsLight>)>,
    mut bloom_query: Query<&mut Bloom>,
    mut floor_query: Query<&MeshMaterial3d<StandardMaterial>, With<FloorEntity>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last_applied: Local<Option<(ThemeId, bool)>>,
) {
    // Skip if theme was already applied and we're not transitioning.
    // The second tuple element tracks whether we were transitioning last frame,
    // so we run once more after transition ends to apply final values.
    let was_transitioning = last_applied.is_some_and(|(_, t)| t);
    let already_applied = last_applied.is_some_and(|(id, _)| id == state.active);
    if !state.transitioning && already_applied && !was_transitioning {
        return;
    }
    *last_applied = Some((state.active, state.transitioning));

    let current_cfg = registry.get(state.active);

    if state.transitioning {
        let pending_id = state.pending.unwrap_or(state.active);
        let target_cfg = registry.get(pending_id);
        let t = smooth_step(state.transition);

        // Ambient light
        let ambient_brightness = lerp_f32(
            current_cfg.ambient_brightness,
            target_cfg.ambient_brightness,
            t,
        );
        runtime.ambient_brightness = ambient_brightness;
        for mut ambient in &mut ambient_query {
            ambient.color = lerp_color(current_cfg.ambient_color, target_cfg.ambient_color, t);
            ambient.brightness = ambient_brightness;
        }

        // Key light (RMS-driven) — we set the color; intensity is handled by rms_light system
        runtime.key_light_intensity = lerp_f32(
            current_cfg.key_light_intensity,
            target_cfg.key_light_intensity,
            t,
        );
        for mut light in &mut key_light_query {
            light.color = lerp_color(current_cfg.key_light_color, target_cfg.key_light_color, t);
        }

        // Rim light
        for mut light in &mut rim_light_query {
            light.color = lerp_color(current_cfg.rim_light_color, target_cfg.rim_light_color, t);
            light.intensity = lerp_f32(
                current_cfg.rim_light_intensity,
                target_cfg.rim_light_intensity,
                t,
            );
        }

        // Bloom
        for mut bloom in &mut bloom_query {
            bloom.intensity = lerp_f32(current_cfg.bloom_intensity, target_cfg.bloom_intensity, t);
            bloom.low_frequency_boost = lerp_f32(
                current_cfg.bloom_low_freq_boost,
                target_cfg.bloom_low_freq_boost,
                t,
            );
        }

        // Floor material color
        for mat_handle in &mut floor_query {
            if let Some(material) = materials.get_mut(&mat_handle.0) {
                material.base_color =
                    lerp_color(current_cfg.floor_color, target_cfg.floor_color, t);
                material.metallic =
                    lerp_f32(current_cfg.floor_metallic, target_cfg.floor_metallic, t)
                        .clamp(0.0, 1.0);
                material.perceptual_roughness =
                    lerp_f32(current_cfg.floor_roughness, target_cfg.floor_roughness, t)
                        .clamp(0.0, 1.0);
            }
        }

        // Material policy (lerped)
        let snapshot = ThemePolicySnapshot::lerp(current_cfg, target_cfg, t);
        policy.update_from_config(&snapshot);
    } else {
        // No transition — just apply the current theme directly
        runtime.ambient_brightness = current_cfg.ambient_brightness;
        for mut ambient in &mut ambient_query {
            ambient.color = current_cfg.ambient_color;
            ambient.brightness = current_cfg.ambient_brightness;
        }

        runtime.key_light_intensity = current_cfg.key_light_intensity;
        for mut light in &mut key_light_query {
            light.color = current_cfg.key_light_color;
        }

        for mut light in &mut rim_light_query {
            light.color = current_cfg.rim_light_color;
            light.intensity = current_cfg.rim_light_intensity;
        }

        for mut bloom in &mut bloom_query {
            bloom.intensity = current_cfg.bloom_intensity;
            bloom.low_frequency_boost = current_cfg.bloom_low_freq_boost;
        }

        for mat_handle in &mut floor_query {
            if let Some(material) = materials.get_mut(&mat_handle.0) {
                material.base_color = current_cfg.floor_color;
                material.metallic = current_cfg.floor_metallic.clamp(0.0, 1.0);
                material.perceptual_roughness = current_cfg.floor_roughness.clamp(0.0, 1.0);
            }
        }

        let snapshot = ThemePolicySnapshot::from_config(current_cfg);
        policy.update_from_config(&snapshot);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Smoothstep for nicer transitions.
fn smooth_step(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Linearly interpolate between two f32 values.
fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Linearly interpolate between two colors in sRGB space.
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let a_srgba = Srgba::from(a);
    let b_srgba = Srgba::from(b);
    Color::srgba(
        lerp_f32(a_srgba.red, b_srgba.red, t),
        lerp_f32(a_srgba.green, b_srgba.green, t),
        lerp_f32(a_srgba.blue, b_srgba.blue, t),
        lerp_f32(a_srgba.alpha, b_srgba.alpha, t),
    )
}
