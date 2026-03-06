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

    // Environment
    pub floor_color: Color,
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
                ambient_brightness: 30.0,
                key_light_color: Color::srgb(1.0, 0.9, 0.8),
                key_light_intensity: 250_000.0,
                rim_light_color: Color::srgb(0.2, 0.4, 1.0),
                rim_light_intensity: 40_000.0,
                bloom_intensity: 0.4,
                bloom_low_freq_boost: 0.6,
                emissive_multiplier: 1.2,
                saturation_offset: 0.1,
                lightness_offset: 0.0,
                metallic: 0.0,
                roughness: 0.5,
                floor_color: Color::srgb(0.02, 0.02, 0.05),
            },
        );

        // 2. Metal
        themes.insert(
            ThemeId::Metal,
            ThemeConfig {
                name: "Metal",
                ambient_color: Color::srgb(0.15, 0.15, 0.18),
                ambient_brightness: 80.0,
                key_light_color: Color::srgb(0.9, 0.95, 1.0),
                key_light_intensity: 180_000.0,
                rim_light_color: Color::srgb(0.7, 0.7, 0.8),
                rim_light_intensity: 25_000.0,
                bloom_intensity: 0.15,
                bloom_low_freq_boost: 0.3,
                emissive_multiplier: 0.6,
                saturation_offset: -0.3,
                lightness_offset: 0.1,
                metallic: 0.9,
                roughness: 0.2,
                floor_color: Color::srgb(0.08, 0.08, 0.1),
            },
        );

        // 3. Glass
        themes.insert(
            ThemeId::Glass,
            ThemeConfig {
                name: "Glass",
                ambient_color: Color::srgb(0.2, 0.25, 0.3),
                ambient_brightness: 100.0,
                key_light_color: Color::srgb(1.0, 1.0, 1.0),
                key_light_intensity: 200_000.0,
                rim_light_color: Color::srgb(0.5, 0.8, 0.9),
                rim_light_intensity: 30_000.0,
                bloom_intensity: 0.25,
                bloom_low_freq_boost: 0.4,
                emissive_multiplier: 0.8,
                saturation_offset: -0.1,
                lightness_offset: 0.15,
                metallic: 0.1,
                roughness: 0.1,
                floor_color: Color::srgb(0.05, 0.08, 0.1),
            },
        );

        // 4. Space
        themes.insert(
            ThemeId::Space,
            ThemeConfig {
                name: "Space",
                ambient_color: Color::srgb(0.05, 0.02, 0.08),
                ambient_brightness: 20.0,
                key_light_color: Color::srgb(0.6, 0.7, 1.0),
                key_light_intensity: 150_000.0,
                rim_light_color: Color::srgb(0.5, 0.2, 0.8),
                rim_light_intensity: 20_000.0,
                bloom_intensity: 0.35,
                bloom_low_freq_boost: 0.5,
                emissive_multiplier: 1.0,
                saturation_offset: 0.0,
                lightness_offset: -0.05,
                metallic: 0.0,
                roughness: 0.5,
                floor_color: Color::srgb(0.01, 0.01, 0.02),
            },
        );

        // 5. Synthwave
        themes.insert(
            ThemeId::Synthwave,
            ThemeConfig {
                name: "Synthwave",
                ambient_color: Color::srgb(0.1, 0.02, 0.08),
                ambient_brightness: 40.0,
                key_light_color: Color::srgb(1.0, 0.2, 0.6),
                key_light_intensity: 200_000.0,
                rim_light_color: Color::srgb(0.0, 0.8, 1.0),
                rim_light_intensity: 35_000.0,
                bloom_intensity: 0.45,
                bloom_low_freq_boost: 0.7,
                emissive_multiplier: 1.3,
                saturation_offset: 0.15,
                lightness_offset: 0.05,
                metallic: 0.0,
                roughness: 0.5,
                floor_color: Color::srgb(0.03, 0.01, 0.06),
            },
        );

        // 6. Ember
        themes.insert(
            ThemeId::Ember,
            ThemeConfig {
                name: "Ember",
                ambient_color: Color::srgb(0.08, 0.03, 0.01),
                ambient_brightness: 35.0,
                key_light_color: Color::srgb(1.0, 0.6, 0.2),
                key_light_intensity: 220_000.0,
                rim_light_color: Color::srgb(0.8, 0.1, 0.05),
                rim_light_intensity: 25_000.0,
                bloom_intensity: 0.35,
                bloom_low_freq_boost: 0.5,
                emissive_multiplier: 1.1,
                saturation_offset: 0.0,
                lightness_offset: -0.05,
                metallic: 0.0,
                roughness: 0.5,
                floor_color: Color::srgb(0.04, 0.03, 0.02),
            },
        );

        // 7. Arctic
        themes.insert(
            ThemeId::Arctic,
            ThemeConfig {
                name: "Arctic",
                ambient_color: Color::srgb(0.2, 0.25, 0.35),
                ambient_brightness: 120.0,
                key_light_color: Color::srgb(0.85, 0.9, 1.0),
                key_light_intensity: 160_000.0,
                rim_light_color: Color::srgb(0.4, 0.6, 0.9),
                rim_light_intensity: 20_000.0,
                bloom_intensity: 0.2,
                bloom_low_freq_boost: 0.3,
                emissive_multiplier: 0.7,
                saturation_offset: -0.2,
                lightness_offset: 0.2,
                metallic: 0.0,
                roughness: 0.5,
                floor_color: Color::srgb(0.15, 0.18, 0.22),
            },
        );

        // 8. Void
        themes.insert(
            ThemeId::Void,
            ThemeConfig {
                name: "Void",
                ambient_color: Color::srgb(0.0, 0.0, 0.0),
                ambient_brightness: 5.0,
                key_light_color: Color::srgb(1.0, 1.0, 1.0),
                key_light_intensity: 300_000.0,
                rim_light_color: Color::srgb(0.0, 0.0, 0.0),
                rim_light_intensity: 0.0,
                bloom_intensity: 0.5,
                bloom_low_freq_boost: 0.8,
                emissive_multiplier: 1.5,
                saturation_offset: -0.6,
                lightness_offset: 0.0,
                metallic: 0.0,
                roughness: 0.5,
                floor_color: Color::srgb(0.0, 0.0, 0.0),
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
    pub version: u64,
}

impl Default for ThemeMaterialPolicy {
    fn default() -> Self {
        // Matches the Neon theme defaults
        Self {
            emissive_multiplier: 1.2,
            saturation_offset: 0.1,
            lightness_offset: 0.0,
            metallic: 0.0,
            roughness: 0.5,
            version: 0,
        }
    }
}

impl ThemeMaterialPolicy {
    /// Update values and bump version if anything changed meaningfully.
    fn update_if_changed(
        &mut self,
        emissive: f32,
        sat_offset: f32,
        lit_offset: f32,
        metallic: f32,
        roughness: f32,
    ) {
        const EPS: f32 = 0.0001;
        if (self.emissive_multiplier - emissive).abs() > EPS
            || (self.saturation_offset - sat_offset).abs() > EPS
            || (self.lightness_offset - lit_offset).abs() > EPS
            || (self.metallic - metallic).abs() > EPS
            || (self.roughness - roughness).abs() > EPS
        {
            self.emissive_multiplier = emissive;
            self.saturation_offset = sat_offset;
            self.lightness_offset = lit_offset;
            self.metallic = metallic;
            self.roughness = roughness;
            self.version = self.version.wrapping_add(1);
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
            ambient_brightness: 30.0,
            key_light_intensity: 250_000.0,
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
) {
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
                    lerp_f32(current_cfg.metallic, target_cfg.metallic, t).clamp(0.0, 1.0);
                material.perceptual_roughness =
                    lerp_f32(current_cfg.roughness, target_cfg.roughness, t).clamp(0.0, 1.0);
            }
        }

        // Material policy (lerped)
        let emissive = lerp_f32(
            current_cfg.emissive_multiplier,
            target_cfg.emissive_multiplier,
            t,
        );
        let sat_offset =
            lerp_f32(current_cfg.saturation_offset, target_cfg.saturation_offset, t);
        let lit_offset =
            lerp_f32(current_cfg.lightness_offset, target_cfg.lightness_offset, t);
        let metallic = lerp_f32(current_cfg.metallic, target_cfg.metallic, t).clamp(0.0, 1.0);
        let roughness =
            lerp_f32(current_cfg.roughness, target_cfg.roughness, t).clamp(0.0, 1.0);
        policy.update_if_changed(emissive, sat_offset, lit_offset, metallic, roughness);
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
                material.metallic = current_cfg.metallic.clamp(0.0, 1.0);
                material.perceptual_roughness = current_cfg.roughness.clamp(0.0, 1.0);
            }
        }

        policy.update_if_changed(
            current_cfg.emissive_multiplier,
            current_cfg.saturation_offset,
            current_cfg.lightness_offset,
            current_cfg.metallic.clamp(0.0, 1.0),
            current_cfg.roughness.clamp(0.0, 1.0),
        );
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
