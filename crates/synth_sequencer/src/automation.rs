//! Automation system for parameter control over time.

use serde::{Deserialize, Serialize};
use synth_core::NormalizedValue;

use super::ids::{SeqInstrumentId, TrackId};
use super::time::PatternTick;

/// A single automation point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AutomationPoint {
    /// Position within the pattern.
    pub tick: PatternTick,
    /// Normalized value (0.0 - 1.0).
    pub value: NormalizedValue,
    /// Curve type for interpolation to next point.
    pub curve: CurveType,
}

impl AutomationPoint {
    /// Create a new automation point with linear interpolation.
    pub fn new(tick: PatternTick, value: NormalizedValue) -> Self {
        Self {
            tick,
            value,
            curve: CurveType::Linear,
        }
    }

    /// Set the curve type (builder pattern).
    pub fn with_curve(mut self, curve: CurveType) -> Self {
        self.curve = curve;
        self
    }
}

/// Interpolation curve type between automation points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CurveType {
    /// Linear interpolation to next point.
    #[default]
    Linear,
    /// Step (hold value until next point).
    Step,
    /// Exponential curve (parameter indicates strength, -127 to 127).
    Exponential(i8),
    /// S-curve (smoothstep).
    SCurve,
}

impl CurveType {
    /// Interpolate between two values using this curve type.
    /// `t` is the normalized position (0.0 to 1.0).
    pub fn interpolate(
        &self,
        from: NormalizedValue,
        to: NormalizedValue,
        t: NormalizedValue,
    ) -> NormalizedValue {
        let t = t.as_f32().clamp(0.0, 1.0);
        let from_f = from.as_f32();
        let to_f = to.as_f32();
        let result = match self {
            Self::Linear => from_f + (to_f - from_f) * t,
            Self::Step => from_f,
            Self::Exponential(strength) => {
                let exp_t = if *strength >= 0 {
                    t.powf(1.0 + *strength as f32 * 0.02)
                } else {
                    1.0 - (1.0 - t).powf(1.0 - *strength as f32 * 0.02)
                };
                from_f + (to_f - from_f) * exp_t
            }
            Self::SCurve => {
                // Smoothstep: 3t² - 2t³
                let s = t * t * (3.0 - 2.0 * t);
                from_f + (to_f - from_f) * s
            }
        };
        NormalizedValue::new(result)
    }
}

/// An automation lane controlling a specific parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationLane {
    /// The parameter being automated.
    pub target: AutomationTarget,
    /// Automation points, sorted by tick.
    points: Vec<AutomationPoint>,
}

impl AutomationLane {
    /// Create a new empty automation lane.
    pub fn new(target: AutomationTarget) -> Self {
        Self {
            target,
            points: Vec::new(),
        }
    }

    /// Add a point, maintaining sort order.
    pub fn add_point(&mut self, point: AutomationPoint) {
        let pos = self.points.partition_point(|p| p.tick <= point.tick);

        // Remove existing point at same tick
        if pos > 0 && self.points[pos - 1].tick == point.tick {
            self.points[pos - 1] = point;
        } else {
            self.points.insert(pos, point);
        }
    }

    /// Remove a point at the given tick.
    pub fn remove_point(&mut self, tick: PatternTick) -> Option<AutomationPoint> {
        let pos = self.points.iter().position(|p| p.tick == tick)?;
        Some(self.points.remove(pos))
    }

    /// Get all points.
    pub fn points(&self) -> &[AutomationPoint] {
        &self.points
    }

    /// Get interpolated value at the given tick.
    pub fn value_at(&self, tick: PatternTick) -> Option<NormalizedValue> {
        if self.points.is_empty() {
            return None;
        }

        // Find surrounding points
        let idx = self.points.partition_point(|p| p.tick <= tick);

        if idx == 0 {
            // Before first point - return first value
            return Some(self.points[0].value);
        }
        if idx >= self.points.len() {
            // After last point - return last value
            return self.points.last().map(|p| p.value);
        }

        let before = &self.points[idx - 1];
        let after = &self.points[idx];

        // Calculate interpolation position
        let t = NormalizedValue::new(
            (tick.0 - before.tick.0) as f32 / (after.tick.0 - before.tick.0) as f32,
        );

        Some(before.curve.interpolate(before.value, after.value, t))
    }

    /// Get the value at a tick, or a default if no points exist.
    pub fn value_at_or(&self, tick: PatternTick, default: NormalizedValue) -> NormalizedValue {
        self.value_at(tick).unwrap_or(default)
    }

    /// Check if the lane is empty.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Get number of points.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Clear all points.
    pub fn clear(&mut self) {
        self.points.clear();
    }
}

/// Target for automation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutomationTarget {
    /// Instrument parameter.
    Instrument {
        instrument: SeqInstrumentId,
        param: AutoInstrumentParam,
    },
    /// Track parameter.
    Track { track: TrackId, param: TrackParam },
    /// Global parameter.
    Global(GlobalParam),
}

impl AutomationTarget {
    /// Display name for GUI labels.
    #[must_use]
    pub fn display_name(&self) -> String {
        match self {
            Self::Instrument { instrument, param } => {
                format!("Inst {} {}", instrument.0, param.display_name())
            }
            Self::Track { track, param } => {
                format!("Track {} {param:?}", track.0)
            }
            Self::Global(param) => format!("{param:?}"),
        }
    }
}

/// Automatable instrument parameters for sequencer automation lanes.
///
/// These are parameter identifiers (no values) used in automation.
/// For engine commands with values, see `engine::commands::InstrumentParam`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AutoInstrumentParam {
    Volume,
    Pan,
    FilterCutoff,
    FilterResonance,
    Attack,
    Decay,
    Sustain,
    Release,
    // Can be extended to match PolyModule parameters
}

impl AutoInstrumentParam {
    /// Display name for GUI labels.
    #[must_use]
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Pan => "Pan",
            Self::FilterCutoff => "Filter Cutoff",
            Self::FilterResonance => "Filter Res",
            Self::Attack => "Attack",
            Self::Decay => "Decay",
            Self::Sustain => "Sustain",
            Self::Release => "Release",
        }
    }

    /// All variants for GUI enumeration.
    pub const ALL: &[Self] = &[
        Self::Volume,
        Self::Pan,
        Self::FilterCutoff,
        Self::FilterResonance,
        Self::Attack,
        Self::Decay,
        Self::Sustain,
        Self::Release,
    ];
}

/// Automatable track parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackParam {
    Volume,
    Pan,
    Mute,
    Solo,
}

/// Automatable global parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GlobalParam {
    Tempo,
    MasterVolume,
    Swing,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_automation_point_creation() {
        let point = AutomationPoint::new(PatternTick(480), NormalizedValue::new(0.5));
        assert_eq!(point.tick.0, 480);
        assert_eq!(point.value, NormalizedValue::new(0.5));
        assert_eq!(point.curve, CurveType::Linear);
    }

    #[test]
    fn test_curve_linear() {
        let curve = CurveType::Linear;
        let zero = NormalizedValue::new(0.0);
        let half = NormalizedValue::new(0.5);
        let one = NormalizedValue::new(1.0);
        assert_eq!(curve.interpolate(zero, one, zero), zero);
        assert_eq!(curve.interpolate(zero, one, half), half);
        assert_eq!(curve.interpolate(zero, one, one), one);
    }

    #[test]
    fn test_curve_step() {
        let curve = CurveType::Step;
        let zero = NormalizedValue::new(0.0);
        let one = NormalizedValue::new(1.0);
        assert_eq!(curve.interpolate(zero, one, zero), zero);
        assert_eq!(
            curve.interpolate(zero, one, NormalizedValue::new(0.5)),
            zero
        );
        assert_eq!(
            curve.interpolate(zero, one, NormalizedValue::new(0.99)),
            zero
        );
    }

    #[test]
    fn test_curve_scurve() {
        let curve = CurveType::SCurve;
        let mid = curve.interpolate(
            NormalizedValue::new(0.0),
            NormalizedValue::new(1.0),
            NormalizedValue::new(0.5),
        );
        assert!((mid.as_f32() - 0.5).abs() < 0.01); // S-curve passes through 0.5 at t=0.5
    }

    #[test]
    fn test_automation_lane_value_at() {
        let mut lane = AutomationLane::new(AutomationTarget::Global(GlobalParam::Tempo));

        lane.add_point(AutomationPoint::new(
            PatternTick(0),
            NormalizedValue::new(0.0),
        ));
        lane.add_point(AutomationPoint::new(
            PatternTick(960),
            NormalizedValue::new(1.0),
        ));

        // Before first point
        assert_eq!(
            lane.value_at(PatternTick(0)),
            Some(NormalizedValue::new(0.0))
        );

        // At first point
        assert!((lane.value_at(PatternTick(0)).unwrap().as_f32() - 0.0).abs() < 0.01);

        // Middle (linear interpolation)
        assert!((lane.value_at(PatternTick(480)).unwrap().as_f32() - 0.5).abs() < 0.01);

        // At last point
        assert!((lane.value_at(PatternTick(960)).unwrap().as_f32() - 1.0).abs() < 0.01);

        // After last point
        assert_eq!(
            lane.value_at(PatternTick(2000)),
            Some(NormalizedValue::new(1.0))
        );
    }

    #[test]
    fn test_automation_lane_add_remove() {
        let mut lane = AutomationLane::new(AutomationTarget::Global(GlobalParam::MasterVolume));

        lane.add_point(AutomationPoint::new(
            PatternTick(100),
            NormalizedValue::new(0.5),
        ));
        lane.add_point(AutomationPoint::new(
            PatternTick(50),
            NormalizedValue::new(0.3),
        )); // Should sort before
        lane.add_point(AutomationPoint::new(
            PatternTick(200),
            NormalizedValue::new(0.8),
        ));

        assert_eq!(lane.len(), 3);
        assert_eq!(lane.points()[0].tick.0, 50);
        assert_eq!(lane.points()[1].tick.0, 100);

        // Remove middle point
        let removed = lane.remove_point(PatternTick(100));
        assert!(removed.is_some());
        assert_eq!(lane.len(), 2);
    }

    #[test]
    fn test_automation_lane_replace_at_same_tick() {
        let mut lane = AutomationLane::new(AutomationTarget::Global(GlobalParam::Swing));

        lane.add_point(AutomationPoint::new(
            PatternTick(100),
            NormalizedValue::new(0.5),
        ));
        lane.add_point(AutomationPoint::new(
            PatternTick(100),
            NormalizedValue::new(0.8),
        )); // Same tick, should replace

        assert_eq!(lane.len(), 1);
        assert_eq!(lane.points()[0].value, NormalizedValue::new(0.8));
    }
}
