//! Automation system for parameter control over time.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use synth_core::{ModuleType, NormalizedValue, Semitones};

use super::ids::{InstrumentId, TrackId};
use super::time::PatternTick;

/// Strength of an exponential automation curve (`-127..=127`).
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct CurveStrength(#[schemars(range(min = -127, max = 127))] i8);

impl CurveStrength {
    pub const MIN: i8 = -127;
    pub const MAX: i8 = 127;
    pub const ZERO: Self = Self(0);
    pub const DEFAULT_EXPONENTIAL: Self = Self(64);

    /// Create a validated curve strength.
    pub const fn new(strength: i8) -> Option<Self> {
        if strength >= Self::MIN {
            Some(Self(strength))
        } else {
            None
        }
    }

    /// Return the raw curve strength.
    pub const fn as_i8(self) -> i8 {
        self.0
    }
}

impl<'de> Deserialize<'de> for CurveStrength {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let strength = i8::deserialize(deserializer)?;
        Self::new(strength)
            .ok_or_else(|| serde::de::Error::custom("curve strength must be in -127..=127"))
    }
}

impl std::fmt::Display for CurveStrength {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Interned-by-`Arc` parameter id for [`AutomationTarget::Module`].
///
/// Wraps `Arc<str>` so cloning the target on the audio thread (the per-tick
/// automation collection / event emission in `sequencer_engine`) is an atomic
/// refcount bump — RT-safe (no heap allocation, no lock), unlike the previous
/// `String` which allocated on every clone.
///
/// Chosen over a global intern pool + `Copy` handle (the roadmap's original
/// sketch): the engine dispatch matches the param id against each descriptor's
/// `type_id` **string**, so an intern handle would force a lock-taking
/// `as_str()` on the audio thread — trading an allocation for a lock. `Arc<str>`
/// keeps the dispatch's lock-free `&str` compare while making the clone cheap.
/// Serializes transparently as the id string, so existing projects load unchanged.
///
/// **Drop caveat (tracked follow-up):** dropping a clone on the audio thread is a
/// refcount *decrement*; only if the source lane was removed mid-playback (so the
/// engine's cached clone in `last_automation_values` / the event buffer is the
/// last ref) does the next `clear()` free on the audio thread. This is a strict
/// improvement over the prior `String` (which freed on *every* drop) but not yet
/// fully eliminated — the robust fix is to route cleared targets through the
/// engine's existing off-thread drop channel (`return_producer`), deferred here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct ParamId(Arc<str>);

impl ParamId {
    /// Borrow the id as a string slice (RT-safe — no lock, no allocation).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ParamId {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

impl From<String> for ParamId {
    fn from(s: String) -> Self {
        Self(Arc::from(s))
    }
}

impl std::fmt::Display for ParamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A single automation point.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
)]
pub enum CurveType {
    /// Linear interpolation to next point.
    #[default]
    Linear,
    /// Step (hold value until next point).
    Step,
    /// Exponential curve (parameter indicates strength, -127 to 127).
    Exponential(CurveStrength),
    /// S-curve (smoothstep).
    SCurve,
}

impl CurveType {
    /// Short display name for GUI labels (ignores the `Exponential` strength).
    #[must_use]
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Step => "Step",
            Self::Exponential(_) => "Exponential",
            Self::SCurve => "S-Curve",
        }
    }

    /// Representative variants for GUI menus. `Exponential` carries a moderate
    /// default strength (positive = ease-out) so a menu pick is usable as-is.
    pub const MENU_KINDS: &[Self] = &[
        Self::Linear,
        Self::Step,
        Self::Exponential(CurveStrength::DEFAULT_EXPONENTIAL),
        Self::SCurve,
    ];

    /// Same *kind* as `other`, ignoring the `Exponential` strength — so a menu
    /// highlights the active kind regardless of the stored strength value.
    #[must_use]
    pub fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Linear, Self::Linear)
                | (Self::Step, Self::Step)
                | (Self::Exponential(_), Self::Exponential(_))
                | (Self::SCurve, Self::SCurve)
        )
    }

    /// Interpolate between two values using this curve type.
    /// `t` is the normalized position (0.0 to 1.0).
    #[must_use]
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
                let strength = strength.as_i8();
                let exp_t = if strength >= 0 {
                    t.powf(1.0 + f32::from(strength) * 0.02)
                } else {
                    1.0 - (1.0 - t).powf(1.0 - f32::from(strength) * 0.02)
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AutomationLane {
    /// The parameter being automated.
    pub target: AutomationTarget,
    /// Automation points, sorted by tick.
    points: Vec<AutomationPoint>,
}

impl AutomationLane {
    /// Create a new empty automation lane.
    #[must_use]
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
    #[must_use]
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

        // Guard against division by zero when both points share the same tick
        if after.tick.0 == before.tick.0 {
            return Some(before.value);
        }

        // Calculate interpolation position
        let t = NormalizedValue::new(
            (tick.0 - before.tick.0) as f32 / (after.tick.0 - before.tick.0) as f32,
        );

        Some(before.curve.interpolate(before.value, after.value, t))
    }

    /// Get the value at a tick, or a default if no points exist.
    #[must_use]
    pub fn value_at_or(&self, tick: PatternTick, default: NormalizedValue) -> NormalizedValue {
        self.value_at(tick).unwrap_or(default)
    }

    /// Check if the lane is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Get number of points.
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Clear all points.
    pub fn clear(&mut self) {
        self.points.clear();
    }
}

/// Target for automation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub enum AutomationTarget {
    /// Instrument parameter.
    Instrument {
        instrument: InstrumentId,
        param: AutoInstrumentParam,
    },
    /// Track parameter.
    ///
    /// `track: None` means "the track hosting the placement": the lane follows
    /// whatever track its pattern is placed on, resolved by the sequencer
    /// during the placement walk. This is the default authoring form — a
    /// pooled pattern should not hard-code a track. `Some(id)` deliberately
    /// automates a specific track regardless of where the pattern is placed
    /// (cross-track automation, e.g. song-spanning fades hosted on a
    /// dedicated automation track).
    Track {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track: Option<TrackId>,
        param: TrackParam,
    },
    /// Global parameter.
    Global(GlobalParam),
    /// A parameter on a specific module within an instrument's graph (the
    /// generic A2 target — any continuous, RT-safe module parameter).
    ///
    /// Module identity is **positional** for this first cut: `module_type` +
    /// `instance` mirror the structure of `synth_engine::ModuleId` (a stable,
    /// non-positional `ModuleId` is a deferred TODO). `synth_sequencer` cannot
    /// depend on `synth_engine`, so the engine reconstructs the `ModuleId` from
    /// these two fields when dispatching.
    ///
    /// The parameter is identified by its descriptor `type_id` (e.g. `"cutoff"`)
    /// rather than a [`synth_core::Param`] value: `Param` carries an `f32` and so
    /// is neither `Eq` nor `Hash`, but `AutomationTarget` is used as a map key.
    /// The dispatch resolves the descriptor by `param_id`, denormalizes the
    /// `0..1` lane value through its range, and rebuilds the concrete `Param`
    /// with `Param::with_f32`.
    Module {
        /// Instrument owning the module.
        instrument: InstrumentId,
        /// Module type (with `instance`, the positional module identity).
        module_type: ModuleType,
        /// 1-based instance index within the module type (as in `ModuleId`).
        instance: u16,
        /// Descriptor `type_id` of the target parameter (e.g. `"cutoff"`).
        /// `Arc`-interned so cloning this target is RT-safe (see [`ParamId`]).
        param_id: ParamId,
    },
}

impl AutomationTarget {
    /// Display name for GUI labels.
    #[must_use]
    pub fn display_name(&self) -> String {
        match self {
            Self::Instrument { instrument, param } => {
                format!("Inst {} {}", instrument.as_u64(), param.display_name())
            }
            Self::Track { track, param } => match track {
                Some(track) => format!("Track {} {}", track.0, param.display_name()),
                None => format!("This Track {}", param.display_name()),
            },
            Self::Global(param) => format!("{param:?}"),
            Self::Module {
                instrument,
                module_type,
                instance,
                param_id,
            } => {
                format!(
                    "Inst {} {module_type:?} {instance} {param_id}",
                    instrument.as_u64()
                )
            }
        }
    }

    /// Resolve this target against the track hosting the placement being
    /// played. A host-track lane (`Track { track: None, .. }`) becomes a
    /// concrete `Track { track: Some(host), .. }`; with no host to resolve
    /// against (`host_track: None` — the placement-less preview) it yields
    /// `None` so the caller drops the lane. Every other target passes through
    /// unchanged. This method is the single source of host-track semantics —
    /// collection sites must not re-derive it inline.
    ///
    /// RT-safe: the `Track` arm is stack-only; the pass-through clone is at
    /// worst an `Arc` refcount bump (see [`ParamId`]).
    #[must_use]
    pub fn resolved(&self, host_track: Option<TrackId>) -> Option<Self> {
        match self {
            Self::Track { track: None, param } => host_track.map(|host| Self::Track {
                track: Some(host),
                param: *param,
            }),
            other => Some(other.clone()),
        }
    }
}

/// Automatable instrument parameters for sequencer automation lanes.
///
/// These are parameter identifiers (no values) used in automation.
/// For engine commands with values, see `engine::commands::InstrumentParam`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub enum TrackParam {
    Volume,
    Pan,
    Mute,
    /// Continuous track-pitch offset applied to the voices playing on the
    /// track — channel-scoped pitch state (portamento, arpeggio, track
    /// vibrato) that may begin, change, or stop while a note is held. Lane
    /// values use the bipolar encoding: see [`TRACK_PITCH_RANGE`],
    /// [`track_pitch_semitones`], and [`track_pitch_normalized`].
    Pitch,
}

impl TrackParam {
    /// Display name for GUI labels.
    #[must_use]
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Pan => "Pan",
            Self::Mute => "Mute",
            Self::Pitch => "Pitch",
        }
    }

    /// All variants for GUI enumeration.
    pub const ALL: &[Self] = &[Self::Volume, Self::Pan, Self::Mute, Self::Pitch];
}

/// Semitone span of the bipolar [`TrackParam::Pitch`] lane encoding: lane
/// value 0.0 = −48 st, 0.5 = 0 st (no offset), 1.0 = +48 st. ±48 st covers
/// the largest tracker portamento spans (four octaves each way).
pub const TRACK_PITCH_RANGE: Semitones = Semitones::new(48.0);

/// Decode a normalized [`TrackParam::Pitch`] lane value into a semitone
/// offset (bipolar around 0 at lane value 0.5).
#[must_use]
pub fn track_pitch_semitones(value: NormalizedValue) -> Semitones {
    Semitones::new(value.to_bipolar().as_f32() * TRACK_PITCH_RANGE.as_f32())
}

/// Encode a semitone offset into the normalized [`TrackParam::Pitch`] lane
/// form, clamped to the representable ±[`TRACK_PITCH_RANGE`].
#[must_use]
pub fn track_pitch_normalized(semitones: Semitones) -> NormalizedValue {
    NormalizedValue::from_range(
        semitones.as_f32(),
        -TRACK_PITCH_RANGE.as_f32(),
        TRACK_PITCH_RANGE.as_f32(),
    )
}

/// Automatable global parameters.
///
/// Tempo and swing are deliberately **not** here: both alter the playback time
/// grid itself rather than a per-block value. Tempo lives in the song's
/// dedicated tempo map (`Song::set_tempo_at` / `tempo_at`) — the single source
/// of truth that drives the sequencer's tick rate. Swing is a note-timing edit
/// (`Pattern::apply_swing`), not a continuously-modulatable scalar; a generic
/// automation lane for either would be a silent second source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub enum GlobalParam {
    MasterVolume,
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
        let mut lane = AutomationLane::new(AutomationTarget::Global(GlobalParam::MasterVolume));

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
        let mut lane = AutomationLane::new(AutomationTarget::Global(GlobalParam::MasterVolume));

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

    #[test]
    fn track_pitch_encoding_round_trips_and_clamps() {
        // Midpoint = no offset.
        assert!(
            track_pitch_semitones(NormalizedValue::new(0.5))
                .as_f32()
                .abs()
                < 1e-4
        );
        // Round trips inside the range.
        for st in [-48.0, -12.0, 0.0, 7.0, 48.0] {
            let decoded = track_pitch_semitones(track_pitch_normalized(Semitones::new(st)));
            assert!(
                (decoded.as_f32() - st).abs() < 1e-3,
                "round trip {st} st, got {}",
                decoded.as_f32()
            );
        }
        // Beyond the range clamps to the boundary.
        let clamped = track_pitch_semitones(track_pitch_normalized(Semitones::new(120.0)));
        assert!((clamped.as_f32() - TRACK_PITCH_RANGE.as_f32()).abs() < 1e-3);
        // The extremes decode to the documented boundaries.
        assert!((track_pitch_semitones(NormalizedValue::new(1.0)).as_f32() - 48.0).abs() < 1e-3);
        assert!((track_pitch_semitones(NormalizedValue::new(0.0)).as_f32() + 48.0).abs() < 1e-3);
    }

    #[test]
    fn track_target_serde_round_trip_and_legacy_form() {
        // Host-track ("this track") form round-trips.
        let host = AutomationTarget::Track {
            track: None,
            param: TrackParam::Volume,
        };
        let json = serde_json::to_string(&host).unwrap();
        // The host form omits the field entirely (no `"track": null`), keeping
        // the on-disk shape minimal.
        assert!(
            !json.contains("track"),
            "host-track form must omit the track field, got: {json}"
        );
        let decoded: AutomationTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, host);

        // Pre-platform files stored a bare track id — still loads, now Some.
        let legacy = r#"{"Track":{"track":2,"param":"Volume"}}"#;
        let decoded: AutomationTarget = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            decoded,
            AutomationTarget::Track {
                track: Some(TrackId(2)),
                param: TrackParam::Volume,
            }
        );

        // A file omitting the field entirely defaults to the host track.
        let missing = r#"{"Track":{"param":"Volume"}}"#;
        let decoded: AutomationTarget = serde_json::from_str(missing).unwrap();
        assert_eq!(decoded, host);
    }

    #[test]
    fn test_module_target_serde_round_trip() {
        let target = AutomationTarget::Module {
            instrument: InstrumentId::new(3),
            module_type: ModuleType::Filter,
            instance: 1,
            param_id: "cutoff".into(),
        };

        let json = serde_json::to_string(&target).unwrap();
        // param_id serializes as a bare string (serde-transparent), so the on-disk
        // form is unchanged from the pre-interning `String` field — pre-F1 projects
        // load unchanged.
        assert!(
            json.contains(r#""param_id":"cutoff""#),
            "param_id must serialize as a bare string, got: {json}"
        );
        // Round-trip from the bare-string form == the pre-F1 on-disk form, so a
        // legacy project (param_id stored as a plain string) loads unchanged.
        let decoded: AutomationTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, target);

        // The variant is usable as a map key (Eq + Hash), like the others.
        let mut map = std::collections::HashMap::new();
        map.insert(target.clone(), NormalizedValue::new(0.5));
        assert_eq!(map.get(&target), Some(&NormalizedValue::new(0.5)));
    }

    #[test]
    fn curve_strength_serde_rejects_negative_128() {
        assert!(matches!(
            serde_json::from_str::<CurveStrength>("-127").map(CurveStrength::as_i8),
            Ok(-127)
        ));
        assert!(serde_json::from_str::<CurveStrength>("-128").is_err());
        assert!(matches!(
            serde_json::from_str::<CurveStrength>("127").map(CurveStrength::as_i8),
            Ok(127)
        ));
        let schema = serde_json::to_value(schemars::schema_for!(CurveStrength)).unwrap_or_default();
        assert_eq!(schema["minimum"], -127);
        assert_eq!(schema["maximum"], 127);
    }
}
