//! Camera system with multiple modes, audio-reactive shake, and beat-synced cuts.
//!
//! Modes: Orbit, Top-Down, Front, Fly-Through, Free Orbit.
//! C cycles modes, Shift+C cycles backwards.
//! Up/Down arrows zoom in/out (orbit modes).
//! Auto-cut mode: beat-synced camera switching on downbeats.

use bevy::prelude::*;

use super::telemetry_color;
use crate::telemetry::SynthTelemetry;

/// Base orbit speed at 120 BPM (radians/sec).
const BASE_SPEED: f32 = 0.1;

/// Reference tempo for base speed.
const REFERENCE_BPM: f32 = 120.0;

/// Zoom speed (units/sec).
const ZOOM_SPEED: f32 = 15.0;

/// Minimum camera distance.
const MIN_RADIUS: f32 = 5.0;

/// Maximum camera distance.
const MAX_RADIUS: f32 = 60.0;

/// Shake decay rate (per second).
const SHAKE_DECAY: f32 = 12.0;

/// Peak threshold for triggering camera shake.
const SHAKE_PEAK_THRESHOLD: f32 = 0.75;

/// Maximum shake displacement in units.
const MAX_SHAKE: f32 = 0.4;

/// Default vertical FOV in radians (π/4 = 45°).
const DEFAULT_FOV: f32 = std::f32::consts::FRAC_PI_4;

/// Maximum additional FOV for dolly-zoom (radians).
const DOLLY_ZOOM_FOV_RANGE: f32 = 0.35;

/// Bass energy threshold to trigger dolly-zoom.
const DOLLY_BASS_THRESHOLD: f32 = 0.4;

/// Number of low FFT bins to average for bass detection.
const DOLLY_BASS_BINS: usize = 8;

/// Dolly-zoom decay rate (per second).
const DOLLY_DECAY: f32 = 4.0;

/// Dolly-zoom radius reduction factor (how much closer the camera moves).
const DOLLY_RADIUS_FACTOR: f32 = 0.3;

/// Available camera modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraMode {
    /// Slow orbit around center, tempo-synced.
    Orbit,
    /// Top-down looking straight down.
    TopDown,
    /// Fixed front view.
    Front,
    /// Fly-through: camera moves forward along Z, looping back.
    FlyThrough,
    /// Free orbit with slightly different angle/height.
    FreeOrbit,
}

impl CameraMode {
    const ALL: [Self; 5] = [
        Self::Orbit,
        Self::TopDown,
        Self::Front,
        Self::FlyThrough,
        Self::FreeOrbit,
    ];

    #[must_use]
    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    #[must_use]
    fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Orbit => "Orbit",
            Self::TopDown => "Top-Down",
            Self::Front => "Front",
            Self::FlyThrough => "Fly-Through",
            Self::FreeOrbit => "Free Orbit",
        }
    }

    #[must_use]
    pub fn from_name(s: &str) -> Option<Self> {
        // Match canonical name() and common variants (no heap allocation)
        Self::ALL.iter().copied().find(|m| s.eq_ignore_ascii_case(m.name()))
            .or_else(|| {
                // Handle underscore/no-separator variants
                if s.eq_ignore_ascii_case("topdown") || s.eq_ignore_ascii_case("top_down") {
                    Some(Self::TopDown)
                } else if s.eq_ignore_ascii_case("flythrough") || s.eq_ignore_ascii_case("fly_through") {
                    Some(Self::FlyThrough)
                } else if s.eq_ignore_ascii_case("freeorbit") || s.eq_ignore_ascii_case("free_orbit")
                    || s.eq_ignore_ascii_case("free-orbit") {
                    Some(Self::FreeOrbit)
                } else {
                    None
                }
            })
    }
}

/// Camera state resource.
#[derive(Resource)]
pub struct CameraState {
    pub mode: CameraMode,
    /// Whether timed auto-cuts are enabled.
    pub auto_cut: bool,
    /// Time since last auto-cut (seconds).
    auto_cut_timer: f32,
    /// Previous beat (integer) for detecting beat crossings.
    /// Current shake intensity (decays over time).
    shake_intensity: f32,
    /// Orbit angle (shared across orbit modes).
    angle: f32,
    /// Current radius (affected by zoom).
    radius: f32,
    /// Fly-through Z position.
    fly_z: f32,
    /// Dolly-zoom intensity (0.0 = none, 1.0 = full).
    dolly_zoom: f32,
    /// Previous bass energy for drop detection.
    prev_bass: f32,
    /// Pending camera mode requested via OSC (consumed by input system).
    pub osc_requested_mode: Option<CameraMode>,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            mode: CameraMode::Orbit,
            auto_cut: false,
            auto_cut_timer: 0.0,
            shake_intensity: 0.0,
            angle: 0.0,
            radius: 25.0,
            fly_z: 30.0,
            dolly_zoom: 0.0,
            prev_bass: 0.0,
            osc_requested_mode: None,
        }
    }
}

/// Orbital camera component (attached to the camera entity).
#[derive(Component)]
pub struct OrbitCamera {
    pub radius: f32,
    pub angle: f32,
}

/// Handle camera mode switching (C / Shift+C) and auto-cut toggle (V).
pub fn input(keys: Res<ButtonInput<KeyCode>>, mut state: ResMut<CameraState>) {
    if keys.just_pressed(KeyCode::KeyC) {
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        state.mode = if shift {
            state.mode.prev()
        } else {
            state.mode.next()
        };
        info!("Camera mode: {}", state.mode.name());
    }

    if keys.just_pressed(KeyCode::KeyV) {
        state.auto_cut = !state.auto_cut;
        info!("Auto-cut: {}", if state.auto_cut { "ON" } else { "OFF" });
    }

    // Handle OSC-requested camera mode
    if let Some(mode) = state.osc_requested_mode.take() {
        state.mode = mode;
        info!("Camera mode (OSC): {}", mode.name());
    }
}

/// Main camera update: position, shake, zoom, beat-synced cuts.
#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    telemetry: Res<SynthTelemetry>,
    mut state: ResMut<CameraState>,
    mut query: Query<(&mut Transform, &mut OrbitCamera, &mut Projection)>,
) {
    let dt = time.delta_secs();
    let t = time.elapsed_secs();

    // --- Dolly-zoom on bass drops ---
    let bass_energy = if telemetry.fft_bin_count > 0 {
        let bins = DOLLY_BASS_BINS.min(telemetry.fft_bin_count);
        let sum: f32 = telemetry.fft[..bins].iter().sum();
        sum / bins as f32
    } else {
        0.0
    };
    // Detect bass drop: sudden increase in bass energy
    let bass_delta = (bass_energy - state.prev_bass).max(0.0);
    state.prev_bass = bass_energy;
    if bass_delta > DOLLY_BASS_THRESHOLD {
        state.dolly_zoom = (state.dolly_zoom + bass_delta * 2.0).min(1.0);
    }
    // Decay dolly-zoom
    state.dolly_zoom *= (-DOLLY_DECAY * dt).exp();
    if state.dolly_zoom < 0.001 {
        state.dolly_zoom = 0.0;
    }

    // --- Audio-reactive shake ---
    // Trigger on peak transients
    if telemetry_color::peak_exceeds_threshold(
        telemetry.peak[0],
        telemetry.peak[1],
        SHAKE_PEAK_THRESHOLD,
    ) {
        let peak_mono = (telemetry.peak[0] + telemetry.peak[1]) * 0.5;
        state.shake_intensity = (state.shake_intensity + peak_mono * 0.5).min(1.0);
    }
    // Decay
    state.shake_intensity *= (-SHAKE_DECAY * dt).exp();
    if state.shake_intensity < 0.001 {
        state.shake_intensity = 0.0;
    }

    // --- Timed auto-cut (~20 seconds) ---
    const AUTO_CUT_INTERVAL: f32 = 20.0;
    if state.auto_cut {
        state.auto_cut_timer += dt;
        if state.auto_cut_timer >= AUTO_CUT_INTERVAL {
            state.auto_cut_timer = 0.0;
            let mut rng = rand::thread_rng();
            use rand::Rng;
            let mut next_idx = rng.gen_range(0..CameraMode::ALL.len());
            let cur_idx = CameraMode::ALL
                .iter()
                .position(|&m| m == state.mode)
                .unwrap_or(0);
            if next_idx == cur_idx {
                next_idx = (next_idx + 1) % CameraMode::ALL.len();
            }
            state.mode = CameraMode::ALL[next_idx];
        }
    } else {
        state.auto_cut_timer = 0.0;
    }

    // --- Zoom (all orbit-like modes) ---
    if keys.pressed(KeyCode::ArrowUp) {
        state.radius = (state.radius - ZOOM_SPEED * dt).max(MIN_RADIUS);
    }
    if keys.pressed(KeyCode::ArrowDown) {
        state.radius = (state.radius + ZOOM_SPEED * dt).min(MAX_RADIUS);
    }

    // --- Tempo-scaled rotation ---
    let tempo_scale = if telemetry.tempo > 0.0 {
        telemetry.tempo / REFERENCE_BPM
    } else {
        1.0
    };
    state.angle += BASE_SPEED * tempo_scale * dt;

    // --- Compute base position per mode ---
    let look_target = Vec3::new(0.0, 2.0, 0.0);

    for (mut transform, mut cam, mut projection) in &mut query {
        // Sync component with state
        cam.radius = state.radius;
        cam.angle = state.angle;

        // Apply dolly-zoom: widen FOV while pushing camera closer
        let dolly_radius_offset = state.dolly_zoom * DOLLY_RADIUS_FACTOR * state.radius;
        let effective_radius = state.radius - dolly_radius_offset;
        if let Projection::Perspective(ref mut persp) = *projection {
            persp.fov = DEFAULT_FOV + state.dolly_zoom * DOLLY_ZOOM_FOV_RANGE;
        }

        let base_pos = match state.mode {
            CameraMode::Orbit => {
                let x = state.angle.cos() * effective_radius;
                let z = state.angle.sin() * effective_radius;
                let y = 4.0 + effective_radius * 0.32;
                Vec3::new(x, y, z)
            }
            CameraMode::TopDown => {
                // Slow rotation overhead
                let x = (state.angle * 0.3).cos() * 2.0;
                let z = (state.angle * 0.3).sin() * 2.0;
                Vec3::new(x, effective_radius, z)
            }
            CameraMode::Front => {
                // Fixed front, slight sway
                let sway = (t * 0.2).sin() * 1.5;
                Vec3::new(sway, 8.0, effective_radius * 0.8)
            }
            CameraMode::FlyThrough => {
                // Move forward along Z, loop back
                state.fly_z -= 8.0 * tempo_scale * dt;
                if state.fly_z < -30.0 {
                    state.fly_z = 30.0;
                }
                let x = (t * 0.5).sin() * 6.0;
                let y = 3.0 + (t * 0.3).cos() * 2.0;
                Vec3::new(x, y, state.fly_z)
            }
            CameraMode::FreeOrbit => {
                // Tilted orbit with varying height
                let x = (state.angle * 0.7).cos() * effective_radius * 0.8;
                let z = (state.angle * 0.7).sin() * effective_radius;
                let y = 6.0 + (state.angle * 0.5).sin() * 4.0 + effective_radius * 0.2;
                Vec3::new(x, y, z)
            }
        };

        // Apply shake offset
        let shake_offset = if state.shake_intensity > 0.001 {
            let sx = (t * 47.0).sin() * state.shake_intensity * MAX_SHAKE;
            let sy = (t * 53.0).cos() * state.shake_intensity * MAX_SHAKE;
            Vec3::new(sx, sy, 0.0)
        } else {
            Vec3::ZERO
        };

        transform.translation = base_pos + shake_offset;
        *transform = transform.looking_at(look_target, Vec3::Y);
    }
}
