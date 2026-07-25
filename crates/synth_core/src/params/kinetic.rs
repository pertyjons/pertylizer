//! Kinetic Modulator parameter types.
//!
//! The Kinetic Modulator is a gate-triggered control module that generates
//! Position/Velocity/Acceleration outputs via Penner easing curves.
//! It fills the gap between ADSR (fixed form), LFO (cyclic), and MSEG (complex editor).

use serde::{Deserialize, Serialize};

use crate::types::{NormalizedValue, Seconds};

// ============================================================================
// EASING CURVE ENUM
// ============================================================================

/// Easing curve type (Penner-style "ease out" curves).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EasingCurve {
    #[default]
    Linear,
    QuadOut,
    CubicOut,
    QuartOut,
    QuintOut,
    ExpoOut,
    CircOut,
    BackOut,
    ElasticOut,
    BounceOut,
}

impl EasingCurve {
    pub const ALL: [Self; 10] = [
        Self::Linear,
        Self::QuadOut,
        Self::CubicOut,
        Self::QuartOut,
        Self::QuintOut,
        Self::ExpoOut,
        Self::CircOut,
        Self::BackOut,
        Self::ElasticOut,
        Self::BounceOut,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::QuadOut => "Quad Out",
            Self::CubicOut => "Cubic Out",
            Self::QuartOut => "Quart Out",
            Self::QuintOut => "Quint Out",
            Self::ExpoOut => "Expo Out",
            Self::CircOut => "Circ Out",
            Self::BackOut => "Back Out",
            Self::ElasticOut => "Elastic Out",
            Self::BounceOut => "Bounce Out",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::Linear => "Constant-rate ramp — a straight line from start to end.",
            Self::QuadOut => "Quadratic ease-out — fast start, gentle settle.",
            Self::CubicOut => "Cubic ease-out — stronger deceleration than quadratic.",
            Self::QuartOut => "Quartic ease-out — sharp start, long tail.",
            Self::QuintOut => "Quintic ease-out — very sharp start, very long tail.",
            Self::ExpoOut => {
                "Exponential ease-out — near-instant start, smooth exponential settle."
            }
            Self::CircOut => "Circular ease-out — a rounded, gradually slowing arc.",
            Self::BackOut => "Overshoots past the target, then settles back.",
            Self::ElasticOut => "Springs past the target and oscillates into place.",
            Self::BounceOut => "Bounces against the target before coming to rest.",
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::QuadOut => "quad_out",
            Self::CubicOut => "cubic_out",
            Self::QuartOut => "quart_out",
            Self::QuintOut => "quint_out",
            Self::ExpoOut => "expo_out",
            Self::CircOut => "circ_out",
            Self::BackOut => "back_out",
            Self::ElasticOut => "elastic_out",
            Self::BounceOut => "bounce_out",
        }
    }

    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|c| {
                crate::module_traits::ChoiceOption::new(c.id(), c.name())
                    .with_description(c.description())
            })
            .collect()
    }
}

// ============================================================================
// LOOP MODE ENUM
// ============================================================================

/// Loop mode for the kinetic modulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KineticLoopMode {
    /// Play once and hold at end value.
    #[default]
    OneShot,
    /// Loop back to start when reaching end.
    Loop,
    /// Alternate direction at each end.
    PingPong,
}

impl KineticLoopMode {
    pub const ALL: [Self; 3] = [Self::OneShot, Self::Loop, Self::PingPong];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::OneShot => "One Shot",
            Self::Loop => "Loop",
            Self::PingPong => "Ping Pong",
        }
    }

    /// One-line description for tooltips, JSON schema, and MCP discovery.
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::OneShot => "Plays the curve once and holds at the end value.",
            Self::Loop => "Restarts from the beginning each time it reaches the end.",
            Self::PingPong => "Reverses direction at each end for back-and-forth motion.",
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::OneShot => "one_shot",
            Self::Loop => "loop",
            Self::PingPong => "ping_pong",
        }
    }

    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }

    pub fn to_choices() -> Vec<crate::module_traits::ChoiceOption> {
        Self::ALL
            .iter()
            .map(|m| {
                crate::module_traits::ChoiceOption::new(m.id(), m.name())
                    .with_description(m.description())
            })
            .collect()
    }
}

// ============================================================================
// KINETIC PARAMETER ENUM
// ============================================================================

/// Kinetic modulator parameter with typed value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum KineticParam {
    /// Curve duration in seconds (0.01 to 10.0, default 0.5).
    Duration(Seconds),
    /// Easing curve type.
    CurveType(EasingCurve),
    /// Overshoot strength for Back/Elastic curves (0.0 to 1.0, default 0.5).
    Overshoot(NormalizedValue),
    /// Bipolar mode: false = [0,1], true = [-1,1].
    Bipolar(bool),
    /// Loop mode.
    LoopMode(KineticLoopMode),
    /// Retrigger on new note-on (true = restart, false = continue).
    Retrigger(bool),
    /// Read-only: current position output (for voice.rs to read via get_param).
    OutputVel(f32),
    /// Read-only: current acceleration output (for voice.rs to read via get_param).
    OutputAcc(f32),
}

impl Default for KineticParam {
    fn default() -> Self {
        Self::Duration(Seconds::new(0.5))
    }
}

// ============================================================================
// EASING FUNCTIONS
// ============================================================================

/// Compute easing position for t in [0, 1].
///
/// Returns a value typically in [0, 1], though BackOut and ElasticOut may overshoot.
#[must_use]
pub fn easing_position(curve: EasingCurve, t: f32, overshoot: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        EasingCurve::Linear => t,
        EasingCurve::QuadOut => {
            let u = 1.0 - t;
            1.0 - u * u
        }
        EasingCurve::CubicOut => {
            let u = 1.0 - t;
            1.0 - u * u * u
        }
        EasingCurve::QuartOut => {
            let u = 1.0 - t;
            1.0 - u * u * u * u
        }
        EasingCurve::QuintOut => {
            let u = 1.0 - t;
            1.0 - u * u * u * u * u
        }
        EasingCurve::ExpoOut => {
            if t >= 1.0 {
                1.0
            } else {
                1.0 - 2.0_f32.powf(-10.0 * t)
            }
        }
        EasingCurve::CircOut => {
            let u = t - 1.0;
            (1.0 - u * u).sqrt()
        }
        EasingCurve::BackOut => {
            let c1 = overshoot * 3.0;
            let c3 = c1 + 1.0;
            let u = t - 1.0;
            1.0 + c3 * u * u * u + c1 * u * u
        }
        EasingCurve::ElasticOut => {
            if t <= 0.0 {
                0.0
            } else if t >= 1.0 {
                1.0
            } else {
                let amplitude = 1.0 + overshoot * 0.5;
                let period = 0.3;
                let s = period / 4.0;
                amplitude
                    * 2.0_f32.powf(-10.0 * t)
                    * ((t - s) * std::f32::consts::TAU / period).sin()
                    + 1.0
            }
        }
        EasingCurve::BounceOut => bounce_out(t),
    }
}

/// Compute easing velocity (first derivative) for t in [0, 1].
///
/// The result is normalized to approximately [-1, 1] using pre-calculated
/// max values for each curve type.
#[must_use]
pub fn easing_velocity(curve: EasingCurve, t: f32, overshoot: f32) -> f32 {
    let raw = easing_velocity_raw(curve, t, overshoot);
    let max_vel = max_velocity(curve, overshoot);
    if max_vel > 0.0 {
        (raw / max_vel).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Compute easing acceleration (second derivative) for t in [0, 1].
///
/// The result is normalized to approximately [-1, 1].
#[must_use]
pub fn easing_acceleration(curve: EasingCurve, t: f32, overshoot: f32) -> f32 {
    let raw = easing_acceleration_raw(curve, t, overshoot);
    let max_acc = max_acceleration(curve, overshoot);
    if max_acc > 0.0 {
        (raw / max_acc).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

// ============================================================================
// RAW DERIVATIVES (un-normalized)
// ============================================================================

fn easing_velocity_raw(curve: EasingCurve, t: f32, overshoot: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        EasingCurve::Linear => 1.0,
        EasingCurve::QuadOut => {
            // d/dt [1-(1-t)^2] = 2(1-t)
            2.0 * (1.0 - t)
        }
        EasingCurve::CubicOut => {
            // d/dt [1-(1-t)^3] = 3(1-t)^2
            let u = 1.0 - t;
            3.0 * u * u
        }
        EasingCurve::QuartOut => {
            // d/dt [1-(1-t)^4] = 4(1-t)^3
            let u = 1.0 - t;
            4.0 * u * u * u
        }
        EasingCurve::QuintOut => {
            // d/dt [1-(1-t)^5] = 5(1-t)^4
            let u = 1.0 - t;
            5.0 * u * u * u * u
        }
        EasingCurve::ExpoOut => {
            // d/dt [1 - 2^(-10t)] = 10 * ln(2) * 2^(-10t)
            10.0 * 2.0_f32.ln() * 2.0_f32.powf(-10.0 * t)
        }
        EasingCurve::CircOut => {
            // d/dt [sqrt(1-(t-1)^2)] = (1-t) / sqrt(1-(t-1)^2)
            let u = t - 1.0;
            let denom = (1.0 - u * u).sqrt();
            if denom > 1e-6 { -u / denom } else { 0.0 }
        }
        EasingCurve::BackOut => {
            // d/dt [1 + c3*(t-1)^3 + c1*(t-1)^2] = 3*c3*(t-1)^2 + 2*c1*(t-1)
            let c1 = overshoot * 3.0;
            let c3 = c1 + 1.0;
            let u = t - 1.0;
            3.0 * c3 * u * u + 2.0 * c1 * u
        }
        EasingCurve::ElasticOut => {
            if t <= 0.0 || t >= 1.0 {
                return 0.0;
            }
            let amplitude = 1.0 + overshoot * 0.5;
            let period = 0.3;
            let s = period / 4.0;
            let omega = std::f32::consts::TAU / period;
            let exp_part = 2.0_f32.powf(-10.0 * t);
            let sin_arg = (t - s) * omega;
            // d/dt [A * 2^(-10t) * sin(omega*(t-s)) + 1]
            // = A * [-10*ln2 * 2^(-10t) * sin(...) + 2^(-10t) * omega * cos(...)]
            amplitude * exp_part * (-10.0 * 2.0_f32.ln() * sin_arg.sin() + omega * sin_arg.cos())
        }
        EasingCurve::BounceOut => bounce_velocity(t),
    }
}

fn easing_acceleration_raw(curve: EasingCurve, t: f32, overshoot: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match curve {
        EasingCurve::Linear => 0.0,
        EasingCurve::QuadOut => {
            // d^2/dt^2 [1-(1-t)^2] = -2
            -2.0
        }
        EasingCurve::CubicOut => {
            // d^2/dt^2 [1-(1-t)^3] = -6(1-t)
            -6.0 * (1.0 - t)
        }
        EasingCurve::QuartOut => {
            // d^2/dt^2 [1-(1-t)^4] = -12(1-t)^2
            let u = 1.0 - t;
            -12.0 * u * u
        }
        EasingCurve::QuintOut => {
            // d^2/dt^2 [1-(1-t)^5] = -20(1-t)^3
            let u = 1.0 - t;
            -20.0 * u * u * u
        }
        EasingCurve::ExpoOut => {
            // d^2/dt^2 [1 - 2^(-10t)] = -100 * ln(2)^2 * 2^(-10t)
            let ln2 = 2.0_f32.ln();
            -100.0 * ln2 * ln2 * 2.0_f32.powf(-10.0 * t)
        }
        EasingCurve::CircOut => {
            // d^2/dt^2 [sqrt(1-(t-1)^2)] = -1 / (1-(t-1)^2)^(3/2)
            let u = t - 1.0;
            let denom = 1.0 - u * u;
            if denom > 1e-6 {
                -1.0 / (denom * denom.sqrt())
            } else {
                0.0
            }
        }
        EasingCurve::BackOut => {
            // d^2/dt^2 = 6*c3*(t-1) + 2*c1
            let c1 = overshoot * 3.0;
            let c3 = c1 + 1.0;
            let u = t - 1.0;
            6.0 * c3 * u + 2.0 * c1
        }
        EasingCurve::ElasticOut => {
            if t <= 0.0 || t >= 1.0 {
                return 0.0;
            }
            let amplitude = 1.0 + overshoot * 0.5;
            let period = 0.3;
            let s = period / 4.0;
            let omega = std::f32::consts::TAU / period;
            let ln2 = 2.0_f32.ln();
            let exp_part = 2.0_f32.powf(-10.0 * t);
            let sin_arg = (t - s) * omega;
            // Second derivative:
            // A * exp * [(100*ln2^2 - omega^2)*sin(...) - 20*ln2*omega*cos(...)]
            amplitude
                * exp_part
                * ((100.0 * ln2 * ln2 - omega * omega) * sin_arg.sin()
                    - 20.0 * ln2 * omega * sin_arg.cos())
        }
        EasingCurve::BounceOut => bounce_acceleration(t),
    }
}

// ============================================================================
// BOUNCE OUT (piecewise)
// ============================================================================

fn bounce_out(t: f32) -> f32 {
    let n1: f32 = 7.5625;
    let d1: f32 = 2.75;

    if t < 1.0 / d1 {
        n1 * t * t
    } else if t < 2.0 / d1 {
        let t = t - 1.5 / d1;
        n1 * t * t + 0.75
    } else if t < 2.5 / d1 {
        let t = t - 2.25 / d1;
        n1 * t * t + 0.9375
    } else {
        let t = t - 2.625 / d1;
        n1 * t * t + 0.984_375
    }
}

fn bounce_velocity(t: f32) -> f32 {
    let n1: f32 = 7.5625;
    let d1: f32 = 2.75;

    if t < 1.0 / d1 {
        2.0 * n1 * t
    } else if t < 2.0 / d1 {
        let t_local = t - 1.5 / d1;
        2.0 * n1 * t_local
    } else if t < 2.5 / d1 {
        let t_local = t - 2.25 / d1;
        2.0 * n1 * t_local
    } else {
        let t_local = t - 2.625 / d1;
        2.0 * n1 * t_local
    }
}

fn bounce_acceleration(t: f32) -> f32 {
    let n1: f32 = 7.5625;
    let d1: f32 = 2.75;

    // Each parabolic segment has constant acceleration 2*n1
    // At segment boundaries there are discontinuities
    if t < 1.0 / d1 || t < 2.0 / d1 || t < 2.5 / d1 || t < 1.0 {
        2.0 * n1
    } else {
        0.0
    }
}

// ============================================================================
// MAX VALUES FOR NORMALIZATION
// ============================================================================

/// Get the approximate max |velocity| for a curve type (for normalization).
#[must_use]
fn max_velocity(curve: EasingCurve, overshoot: f32) -> f32 {
    match curve {
        EasingCurve::Linear => 1.0,
        EasingCurve::QuadOut => 2.0,
        EasingCurve::CubicOut => 3.0,
        EasingCurve::QuartOut => 4.0,
        EasingCurve::QuintOut => 5.0,
        EasingCurve::ExpoOut => 10.0 * 2.0_f32.ln(),
        EasingCurve::CircOut => {
            // Peak near t=0: approaches infinity, but for practical purposes ~10
            10.0
        }
        EasingCurve::BackOut => {
            // Sample the curve to find max velocity
            let c1 = overshoot * 3.0;
            let c3 = c1 + 1.0;
            // At t=0: 3*c3 - 2*c1
            (3.0 * c3 - 2.0 * c1).max(3.0 * c3)
        }
        EasingCurve::ElasticOut => {
            // Elastic has high velocity at t near 0
            let amplitude = 1.0 + overshoot * 0.5;
            let period = 0.3;
            let omega = std::f32::consts::TAU / period;
            amplitude * (100.0_f32.sqrt() * 2.0_f32.ln().powi(2) + omega * omega).sqrt()
        }
        EasingCurve::BounceOut => {
            // Max at first impact: 2 * n1 * (1/d1)
            let n1: f32 = 7.5625;
            let d1: f32 = 2.75;
            2.0 * n1 / d1
        }
    }
}

/// Get the approximate max |acceleration| for a curve type (for normalization).
#[must_use]
fn max_acceleration(curve: EasingCurve, overshoot: f32) -> f32 {
    match curve {
        EasingCurve::Linear => 1.0, // Avoid div-by-zero
        EasingCurve::QuadOut => 2.0,
        EasingCurve::CubicOut => 6.0,
        EasingCurve::QuartOut => 12.0,
        EasingCurve::QuintOut => 20.0,
        EasingCurve::ExpoOut => {
            let ln2 = 2.0_f32.ln();
            100.0 * ln2 * ln2
        }
        EasingCurve::CircOut => 100.0, // Diverges near t=1, cap at practical value
        EasingCurve::BackOut => {
            let c1 = overshoot * 3.0;
            let c3 = c1 + 1.0;
            // Max at t=0: |6*c3*(-1) + 2*c1| = |2*c1 - 6*c3|
            (2.0 * c1 - 6.0 * c3).abs().max(2.0 * c1)
        }
        EasingCurve::ElasticOut => {
            let amplitude = 1.0 + overshoot * 0.5;
            let period = 0.3;
            let omega = std::f32::consts::TAU / period;
            let ln2 = 2.0_f32.ln();
            amplitude * (100.0 * ln2 * ln2 + omega * omega)
        }
        EasingCurve::BounceOut => {
            let n1: f32 = 7.5625;
            2.0 * n1
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module_traits::ModuleParam;

    #[test]
    fn test_easing_curve_endpoints() {
        // All curves should map 0 -> 0 and 1 -> 1 (approximately)
        for curve in EasingCurve::ALL {
            let start = easing_position(curve, 0.0, 0.5);
            let end = easing_position(curve, 1.0, 0.5);
            assert!(start.abs() < 0.01, "{:?}: start={start}", curve);
            assert!((end - 1.0).abs() < 0.01, "{:?}: end={end}", curve);
        }
    }

    #[test]
    fn test_easing_monotonic_simple_curves() {
        // Simple curves (not Back/Elastic/Bounce) should be monotonically increasing
        let simple = [
            EasingCurve::Linear,
            EasingCurve::QuadOut,
            EasingCurve::CubicOut,
            EasingCurve::QuartOut,
            EasingCurve::QuintOut,
            EasingCurve::ExpoOut,
            EasingCurve::CircOut,
        ];
        for curve in simple {
            let mut prev = 0.0;
            for i in 0..=100 {
                let t = i as f32 / 100.0;
                let pos = easing_position(curve, t, 0.5);
                assert!(
                    pos >= prev - 0.001,
                    "{:?} not monotonic at t={t}: {prev} -> {pos}",
                    curve
                );
                prev = pos;
            }
        }
    }

    #[test]
    fn test_easing_velocity_normalized() {
        // Velocity should be approximately in [-1, 1]
        for curve in EasingCurve::ALL {
            for i in 0..=100 {
                let t = i as f32 / 100.0;
                let vel = easing_velocity(curve, t, 0.5);
                assert!(
                    (-1.1..=1.1).contains(&vel),
                    "{:?} vel out of range at t={t}: {vel}",
                    curve
                );
            }
        }
    }

    #[test]
    fn test_kinetic_param_roundtrip() {
        let params = [
            KineticParam::Duration(Seconds::new(0.5)),
            KineticParam::CurveType(EasingCurve::CubicOut),
            KineticParam::Overshoot(NormalizedValue::new(0.7)),
            KineticParam::Bipolar(true),
            KineticParam::LoopMode(KineticLoopMode::PingPong),
            KineticParam::Retrigger(true),
        ];
        for param in &params {
            let f = param.as_f32();
            let reconstructed = param.with_f32(f);
            assert!(
                param.same_kind(&reconstructed),
                "Roundtrip failed for {:?}",
                param
            );
        }
    }

    #[test]
    fn test_easing_curve_index_roundtrip() {
        for (i, curve) in EasingCurve::ALL.iter().enumerate() {
            assert_eq!(curve.index(), i);
            assert_eq!(EasingCurve::from_index(i), *curve);
        }
    }

    #[test]
    fn test_loop_mode_index_roundtrip() {
        for (i, mode) in KineticLoopMode::ALL.iter().enumerate() {
            assert_eq!(mode.index(), i);
            assert_eq!(KineticLoopMode::from_index(i), *mode);
        }
    }
}
