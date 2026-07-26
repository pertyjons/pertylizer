//! The single [`ModuleParam`] implementation for every fixed module-parameter
//! enum and the aggregate [`Param`].
//!
//! Each implementation owns the complete method contract: value conversion,
//! same-kind comparison, display naming, and type metadata. The value field is
//! dispatched through [`ScalarParam`] so kind/unit follow its domain type.
//!
//! Two-field `Variant(u8, T)` binds the value field `v` (the leading index is
//! structural). See `docs/param-kinds.md` for the authoritative mapping.

use super::mod_matrix::{
    SLOT_NAMES_AMOUNT, SLOT_NAMES_DEST, SLOT_NAMES_ENABLED, SLOT_NAMES_SOURCE,
};
use super::*;
use crate::module_traits::{ModuleParam, ParamKind, ParameterUnit, ResponseCurve, ScalarParam};
use crate::types::*;

/// Generate the three pieces of scalar metadata from one variant dispatch.
///
/// Keeping the pattern-to-value mapping in one place prevents kind, unit, and
/// response-curve metadata from drifting apart when a parameter is added.
macro_rules! scalar_metadata_methods {
    ($($pattern:pat => $value:expr),+ $(,)?) => {
        fn kind(&self) -> ParamKind {
            match self {
                $($pattern => ($value).scalar_kind()),+
            }
        }

        fn unit(&self) -> ParameterUnit {
            match self {
                $($pattern => ($value).scalar_unit()),+
            }
        }

        fn default_curve(&self) -> ResponseCurve {
            match self {
                $($pattern => ($value).scalar_curve()),+
            }
        }
    };
}

impl ModuleParam for AdditiveParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Tilt(_) => "Tilt",
            Self::OddEven(_) => "Odd/Even",
            Self::Brightness(_) => "Brightness",
            Self::Stretch(_) => "Stretch",
            Self::Randomize(_) => "Randomize",
            Self::Level(_) => "Level",
            Self::GlideTime(_) => "Glide",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Tilt(v)
            | Self::OddEven(v)
            | Self::Brightness(v)
            | Self::Stretch(v)
            | Self::Randomize(v)
            | Self::Level(v) => v.as_f32(),
            Self::GlideTime(s) => s.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Tilt(_) => Self::Tilt(NormalizedValue::new(value)),
            Self::OddEven(_) => Self::OddEven(NormalizedValue::new(value)),
            Self::Brightness(_) => Self::Brightness(NormalizedValue::new(value)),
            Self::Stretch(_) => Self::Stretch(NormalizedValue::new(value)),
            Self::Randomize(_) => Self::Randomize(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
            Self::GlideTime(_) => Self::GlideTime(Seconds::new(value.max(0.0))),
        }
    }

    scalar_metadata_methods! {
        Self::Tilt(v)
        | Self::OddEven(v)
        | Self::Brightness(v)
        | Self::Stretch(v)
        | Self::Randomize(v)
        | Self::Level(v) => v,
        Self::GlideTime(v) => v,
    }
}

impl ModuleParam for AmFormantParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Vowel(_) => "Vowel",
            Self::CarrierRatio(_) => "Carrier",
            Self::Depth(_) => "Depth",
            Self::Level(_) => "Level",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Vowel(v) | Self::CarrierRatio(v) | Self::Depth(v) | Self::Level(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Vowel(_) => Self::Vowel(NormalizedValue::new(value)),
            Self::CarrierRatio(_) => Self::CarrierRatio(NormalizedValue::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Vowel(v) | Self::CarrierRatio(v) | Self::Depth(v) | Self::Level(v) => {
            v
        }
    }
}

impl ModuleParam for BeatDetectorParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Sensitivity(_) => "Sensitivity",
            Self::FilterFreq(_) => "Filter",
            Self::HoldTime(_) => "Hold",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Sensitivity(v) => v.as_f32(),
            Self::FilterFreq(hz) => hz.as_f32(),
            Self::HoldTime(ms) => ms.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Sensitivity(_) => Self::Sensitivity(NormalizedValue::new(value)),
            Self::FilterFreq(_) => Self::FilterFreq(Hertz::new(value)),
            Self::HoldTime(_) => Self::HoldTime(Milliseconds::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Sensitivity(v) => v,
        Self::FilterFreq(v) => v,
        Self::HoldTime(v) => v,
    }
}

impl ModuleParam for ChaoticOscParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::System(_) => "System",
            Self::Rate(_) => "Rate",
            Self::Chaos(_) => "Chaos",
            Self::Depth(_) => "Depth",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::System(s) => s.index() as f32,
            Self::Rate(hz) => hz.as_f32(),
            Self::Chaos(c) => c.as_f32(),
            Self::Depth(d) => d.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::System(_) => {
                Self::System(ChaoticSystem::from_index(value as usize).unwrap_or_default())
            }
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Chaos(_) => Self::Chaos(NormalizedValue::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::System(v) => v,
        Self::Rate(v) => v,
        Self::Chaos(v) | Self::Depth(v) => v,
    }
}

impl ModuleParam for ConvolverParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Ir(_) => "IR Type",
            Self::Mix(_) => "Mix",
            Self::PreDelay(_) => "Pre-Delay",
            Self::DecayTrim(_) => "Decay",
            Self::Brightness(_) => "Brightness",
            Self::DynamicMode(_) => "Dynamic",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Ir(ir) => ir.index() as f32,
            Self::Mix(v) | Self::Brightness(v) | Self::DynamicMode(v) => v.as_f32(),
            Self::DecayTrim(d) => d.as_f32(),
            Self::PreDelay(ms) => ms.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Ir(_) => Self::Ir(ImpulseResponse::from_index(value as usize)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::PreDelay(_) => Self::PreDelay(Milliseconds::new(value)),
            Self::DecayTrim(_) => Self::DecayTrim(DecayTrim::new(value)),
            Self::Brightness(_) => Self::Brightness(NormalizedValue::new(value)),
            Self::DynamicMode(_) => Self::DynamicMode(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Ir(v) => v,
        Self::Mix(v) | Self::Brightness(v) | Self::DynamicMode(v) => v,
        Self::PreDelay(v) => v,
        Self::DecayTrim(v) => v,
    }
}

impl ModuleParam for DriftGeneratorParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Smoothness(_) => "Smoothness",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(d) => d.as_f32(),
            Self::Smoothness(s) => s.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Smoothness(_) => Self::Smoothness(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Rate(v) => v,
        Self::Depth(v) | Self::Smoothness(v) => v,
    }
}

impl ModuleParam for DelayParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Mode(_) => "Mode",
            Self::Time(_) => "Time",
            Self::TimeLeft(_) => "Time L",
            Self::TimeRight(_) => "Time R",
            Self::Feedback(_) => "Feedback",
            Self::Mix(_) => "Mix",
            Self::Damping(_) => "Damping",
            Self::TempoSync(_) => "Tempo Sync",
            Self::SyncDivision(_) => "Division",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Mode(m) => m.index() as f32,
            Self::Time(t) | Self::TimeLeft(t) | Self::TimeRight(t) => t.as_f32(),
            Self::Feedback(v) | Self::Mix(v) | Self::Damping(v) => v.as_f32(),
            Self::TempoSync(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::SyncDivision(d) => d.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Mode(_) => Self::Mode(DelayMode::from_index(value as usize).unwrap_or_default()),
            Self::Time(_) => Self::Time(Seconds::new(value)),
            Self::TimeLeft(_) => Self::TimeLeft(Seconds::new(value)),
            Self::TimeRight(_) => Self::TimeRight(Seconds::new(value)),
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Damping(_) => Self::Damping(NormalizedValue::new(value)),
            Self::TempoSync(_) => Self::TempoSync(value > 0.5),
            Self::SyncDivision(_) => Self::SyncDivision(BeatDivision::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Mode(v) => v,
        Self::Time(v) | Self::TimeLeft(v) | Self::TimeRight(v) => v,
        Self::Feedback(v) | Self::Mix(v) | Self::Damping(v) => v,
        Self::TempoSync(v) => v,
        Self::SyncDivision(v) => v,
    }
}

impl ModuleParam for ReverbParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::RoomSize(_) => "Room Size",
            Self::PreDelay(_) => "Pre-Delay",
            Self::Damping(_) => "Damping",
            Self::Width(_) => "Width",
            Self::Mix(_) => "Mix",
            Self::Decay(_) => "Decay",
            Self::Diffusion(_) => "Diffusion",
            Self::LowCut(_) => "Low Cut",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::RoomSize(v)
            | Self::Damping(v)
            | Self::Width(v)
            | Self::Mix(v)
            | Self::Decay(v)
            | Self::Diffusion(v) => v.as_f32(),
            Self::PreDelay(t) => t.as_f32(),
            Self::LowCut(hz) => hz.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::RoomSize(_) => Self::RoomSize(NormalizedValue::new(value)),
            Self::PreDelay(_) => Self::PreDelay(Seconds::new(value)),
            Self::Damping(_) => Self::Damping(NormalizedValue::new(value)),
            Self::Width(_) => Self::Width(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Decay(_) => Self::Decay(NormalizedValue::new(value)),
            Self::Diffusion(_) => Self::Diffusion(NormalizedValue::new(value)),
            Self::LowCut(_) => Self::LowCut(Hertz::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::RoomSize(v)
        | Self::Damping(v)
        | Self::Width(v)
        | Self::Mix(v)
        | Self::Decay(v)
        | Self::Diffusion(v) => v,
        Self::PreDelay(v) => v,
        Self::LowCut(v) => v,
    }
}

impl ModuleParam for DistortionParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Mode(_) => "Mode",
            Self::Drive(_) => "Drive",
            Self::Tone(_) => "Tone",
            Self::Mix(_) => "Mix",
            Self::BitDepth(_) => "BitDepth",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Mode(m) => m.index() as f32,
            Self::Drive(v) | Self::Tone(v) | Self::Mix(v) => v.as_f32(),
            Self::BitDepth(b) => b.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Mode(_) => {
                Self::Mode(DistortionMode::from_index(value as usize).unwrap_or_default())
            }
            Self::Drive(_) => Self::Drive(NormalizedValue::new(value)),
            Self::Tone(_) => Self::Tone(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::BitDepth(_) => Self::BitDepth(BitDepth::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Mode(v) => v,
        Self::Drive(v) | Self::Tone(v) | Self::Mix(v) => v,
        Self::BitDepth(v) => v,
    }
}

impl ModuleParam for ChorusParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Delay(_) => "Delay",
            Self::Feedback(_) => "Feedback",
            Self::Mix(_) => "Mix",
            Self::Voices(_) => "Voices",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v.as_f32(),
            Self::Delay(ms) => ms.as_f32(),
            Self::Voices(v) => v.as_u8() as f32,
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Delay(_) => Self::Delay(Milliseconds::new(value)),
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Voices(_) => Self::Voices(VoiceCount::new(value.round() as u8)),
        }
    }

    scalar_metadata_methods! {
        Self::Rate(v) => v,
        Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v,
        Self::Delay(v) => v,
        Self::Voices(v) => v,
    }
}

impl ModuleParam for PhaserParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Feedback(_) => "Feedback",
            Self::Stages(_) => "Stages",
            Self::CenterFreq(_) => "Center Freq",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) | Self::CenterFreq(hz) => hz.as_f32(),
            Self::Depth(v) | Self::Mix(v) => v.as_f32(),
            Self::Feedback(b) => b.as_f32(),
            Self::Stages(s) => *s as f32,
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Feedback(_) => Self::Feedback(BipolarValue::new(value)),
            Self::Stages(_) => Self::Stages(value.round() as u8),
            Self::CenterFreq(_) => Self::CenterFreq(Hertz::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Rate(v) | Self::CenterFreq(v) => v,
        Self::Depth(v) | Self::Mix(v) => v,
        Self::Feedback(v) => v,
        Self::Stages(v) => v,
    }
}

impl ModuleParam for FlangerParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Feedback(_) => "Feedback",
            Self::Delay(_) => "Delay",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(v) | Self::Mix(v) => v.as_f32(),
            Self::Feedback(b) => b.as_f32(),
            Self::Delay(ms) => ms.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Feedback(_) => Self::Feedback(BipolarValue::new(value)),
            Self::Delay(_) => Self::Delay(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Rate(v) => v,
        Self::Depth(v) | Self::Mix(v) => v,
        Self::Feedback(v) => v,
        Self::Delay(v) => v,
    }
}

impl ModuleParam for CompressorParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Threshold(_) => "Threshold",
            Self::Ratio(_) => "Ratio",
            Self::Attack(_) => "Attack",
            Self::Release(_) => "Release",
            Self::Makeup(_) => "Makeup",
            Self::Mix(_) => "Mix",
            Self::SidechainEnabled(_) => "Sidechain",
            Self::SidechainFilter(_) => "SC Filter",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Threshold(db) | Self::Makeup(db) => db.as_f32(),
            Self::Ratio(r) => r.as_f32(),
            Self::Attack(ms) | Self::Release(ms) => ms.as_f32(),
            Self::Mix(v) => v.as_f32(),
            Self::SidechainEnabled(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::SidechainFilter(hz) => hz.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Threshold(_) => Self::Threshold(Decibels::new(value)),
            Self::Ratio(_) => Self::Ratio(Ratio::new(value)),
            Self::Attack(_) => Self::Attack(Milliseconds::new(value)),
            Self::Release(_) => Self::Release(Milliseconds::new(value)),
            Self::Makeup(_) => Self::Makeup(Decibels::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::SidechainEnabled(_) => Self::SidechainEnabled(value > 0.5),
            Self::SidechainFilter(_) => Self::SidechainFilter(Hertz::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Threshold(v) | Self::Makeup(v) => v,
        Self::Ratio(v) => v,
        Self::Attack(v) | Self::Release(v) => v,
        Self::Mix(v) => v,
        Self::SidechainEnabled(v) => v,
        Self::SidechainFilter(v) => v,
    }
}

impl ModuleParam for TransientShaperParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Attack(_) => "Attack",
            Self::Sustain(_) => "Sustain",
            Self::Sensitivity(_) => "Sensitivity",
            Self::Window(_) => "Window",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Attack(db) | Self::Sustain(db) => db.as_f32(),
            Self::Sensitivity(v) | Self::Mix(v) => v.as_f32(),
            Self::Window(ms) => ms.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Attack(_) => Self::Attack(Decibels::new(value)),
            Self::Sustain(_) => Self::Sustain(Decibels::new(value)),
            Self::Sensitivity(_) => Self::Sensitivity(NormalizedValue::new(value)),
            Self::Window(_) => Self::Window(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Attack(v) | Self::Sustain(v) => v,
        Self::Sensitivity(v) | Self::Mix(v) => v,
        Self::Window(v) => v,
    }
}

impl ModuleParam for EqParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::LowFreq(_) => "Low Freq",
            Self::LowGain(_) => "Low Gain",
            Self::MidFreq(_) => "Mid Freq",
            Self::MidGain(_) => "Mid Gain",
            Self::MidQ(_) => "Mid Q",
            Self::HighFreq(_) => "High Freq",
            Self::HighGain(_) => "High Gain",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::LowFreq(hz) | Self::MidFreq(hz) | Self::HighFreq(hz) => hz.as_f32(),
            Self::LowGain(db) | Self::MidGain(db) | Self::HighGain(db) => db.as_f32(),
            Self::MidQ(v) | Self::Mix(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::LowFreq(_) => Self::LowFreq(Hertz::new(value)),
            Self::LowGain(_) => Self::LowGain(Decibels::new(value)),
            Self::MidFreq(_) => Self::MidFreq(Hertz::new(value)),
            Self::MidGain(_) => Self::MidGain(Decibels::new(value)),
            Self::MidQ(_) => Self::MidQ(NormalizedValue::new(value)),
            Self::HighFreq(_) => Self::HighFreq(Hertz::new(value)),
            Self::HighGain(_) => Self::HighGain(Decibels::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::LowFreq(v) | Self::MidFreq(v) | Self::HighFreq(v) => v,
        Self::LowGain(v) | Self::MidGain(v) | Self::HighGain(v) => v,
        Self::MidQ(v) | Self::Mix(v) => v,
    }
}

impl ModuleParam for BbdDelayParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Time(_) => "Time",
            Self::Feedback(_) => "Feedback",
            Self::Tone(_) => "Tone",
            Self::WowFlutter(_) => "Wow/Flutter",
            Self::ClockNoise(_) => "Clock Noise",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Time(t) => t.as_f32(),
            Self::Feedback(v)
            | Self::Tone(v)
            | Self::WowFlutter(v)
            | Self::ClockNoise(v)
            | Self::Mix(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Time(_) => Self::Time(Seconds::new(value)),
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Tone(_) => Self::Tone(NormalizedValue::new(value)),
            Self::WowFlutter(_) => Self::WowFlutter(NormalizedValue::new(value)),
            Self::ClockNoise(_) => Self::ClockNoise(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Time(v) => v,
        Self::Feedback(v)
        | Self::Tone(v)
        | Self::WowFlutter(v)
        | Self::ClockNoise(v)
        | Self::Mix(v) => v,
    }
}

impl ModuleParam for MidSideParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Width(_) => "Width",
            Self::MidGain(_) => "Mid Gain",
            Self::SideGain(_) => "Side Gain",
            Self::Rotation(_) => "Rotation",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Width(v) | Self::Mix(v) => v.as_f32(),
            Self::MidGain(db) | Self::SideGain(db) => db.as_f32(),
            Self::Rotation(b) => b.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Width(_) => Self::Width(NormalizedValue::new(value)),
            Self::MidGain(_) => Self::MidGain(Decibels::new(value)),
            Self::SideGain(_) => Self::SideGain(Decibels::new(value)),
            Self::Rotation(_) => Self::Rotation(BipolarValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Width(v) | Self::Mix(v) => v,
        Self::MidGain(v) | Self::SideGain(v) => v,
        Self::Rotation(v) => v,
    }
}

impl ModuleParam for LimiterParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Ceiling(_) => "Ceiling",
            Self::LookAhead(_) => "Look-Ahead",
            Self::Release(_) => "Release",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Ceiling(db) => db.as_f32(),
            Self::LookAhead(ms) | Self::Release(ms) => ms.as_f32(),
            Self::Mix(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Ceiling(_) => Self::Ceiling(Decibels::new(value)),
            Self::LookAhead(_) => Self::LookAhead(Milliseconds::new(value)),
            Self::Release(_) => Self::Release(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Ceiling(v) => v,
        Self::LookAhead(v) | Self::Release(v) => v,
        Self::Mix(v) => v,
    }
}

impl ModuleParam for EnsembleChorusParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::BaseDelay(_) => "Base Delay",
            Self::Mix(_) => "Mix",
            Self::Tone(_) => "Tone",
            Self::Noise(_) => "Noise",
            Self::StereoWidth(_) => "Stereo Width",
            Self::Voices(_) => "Voices",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(ms) | Self::BaseDelay(ms) => ms.as_f32(),
            Self::Mix(v) | Self::Tone(v) | Self::Noise(v) | Self::StereoWidth(v) => v.as_f32(),
            Self::Voices(v) => v.as_u8() as f32,
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(Milliseconds::new(value)),
            Self::BaseDelay(_) => Self::BaseDelay(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Tone(_) => Self::Tone(NormalizedValue::new(value)),
            Self::Noise(_) => Self::Noise(NormalizedValue::new(value)),
            Self::StereoWidth(_) => Self::StereoWidth(NormalizedValue::new(value)),
            Self::Voices(_) => Self::Voices(VoiceCount::new(value.round() as u8)),
        }
    }

    scalar_metadata_methods! {
        Self::Rate(v) => v,
        Self::Depth(v) | Self::BaseDelay(v) => v,
        Self::Mix(v) | Self::Tone(v) | Self::Noise(v) | Self::StereoWidth(v) => v,
        Self::Voices(v) => v,
    }
}

impl ModuleParam for ShimmerReverbParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::RoomSize(_) => "Room Size",
            Self::Decay(_) => "Decay",
            Self::Damping(_) => "Damping",
            Self::PreDelay(_) => "Pre-Delay",
            Self::PitchSemitones(_) => "Pitch",
            Self::ShimmerMix(_) => "Shimmer",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::RoomSize(v)
            | Self::Decay(v)
            | Self::Damping(v)
            | Self::ShimmerMix(v)
            | Self::Mix(v) => v.as_f32(),
            Self::PreDelay(t) => t.as_f32(),
            Self::PitchSemitones(s) => s.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::RoomSize(_) => Self::RoomSize(NormalizedValue::new(value)),
            Self::Decay(_) => Self::Decay(NormalizedValue::new(value)),
            Self::Damping(_) => Self::Damping(NormalizedValue::new(value)),
            Self::PreDelay(_) => Self::PreDelay(Seconds::new(value)),
            Self::PitchSemitones(_) => Self::PitchSemitones(Semitones::new(value)),
            Self::ShimmerMix(_) => Self::ShimmerMix(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::RoomSize(v)
        | Self::Decay(v)
        | Self::Damping(v)
        | Self::ShimmerMix(v)
        | Self::Mix(v) => v,
        Self::PreDelay(v) => v,
        Self::PitchSemitones(v) => v,
    }
}

impl ModuleParam for GranularFxParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::BufferTime(_) => "Buffer",
            Self::GrainSize(_) => "Grain Size",
            Self::Density(_) => "Density",
            Self::Position(_) => "Position",
            Self::PositionSpread(_) => "Pos Spread",
            Self::PitchSpread(_) => "Pitch Spread",
            Self::PanSpread(_) => "Pan Spread",
            Self::Freeze(_) => "Freeze",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::BufferTime(t) => t.as_f32(),
            Self::GrainSize(ms) => ms.as_f32(),
            Self::Density(v)
            | Self::Position(v)
            | Self::PositionSpread(v)
            | Self::PitchSpread(v)
            | Self::PanSpread(v)
            | Self::Mix(v) => v.as_f32(),
            Self::Freeze(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::BufferTime(_) => Self::BufferTime(Seconds::new(value)),
            Self::GrainSize(_) => Self::GrainSize(Milliseconds::new(value)),
            Self::Density(_) => Self::Density(NormalizedValue::new(value)),
            Self::Position(_) => Self::Position(NormalizedValue::new(value)),
            Self::PositionSpread(_) => Self::PositionSpread(NormalizedValue::new(value)),
            Self::PitchSpread(_) => Self::PitchSpread(NormalizedValue::new(value)),
            Self::PanSpread(_) => Self::PanSpread(NormalizedValue::new(value)),
            Self::Freeze(_) => Self::Freeze(value > 0.5),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::BufferTime(v) => v,
        Self::GrainSize(v) => v,
        Self::Density(v)
        | Self::Position(v)
        | Self::PositionSpread(v)
        | Self::PitchSpread(v)
        | Self::PanSpread(v)
        | Self::Mix(v) => v,
        Self::Freeze(v) => v,
    }
}

impl ModuleParam for SpectralBlurParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::FftSize(_) => "FFT Size",
            Self::BlurTime(_) => "Blur Time",
            Self::BlurFreq(_) => "Blur Freq",
            Self::Freeze(_) => "Freeze",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::FftSize(f) => f.index() as f32,
            Self::BlurTime(v) | Self::BlurFreq(v) | Self::Mix(v) => v.as_f32(),
            Self::Freeze(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::FftSize(_) => Self::FftSize(FftSizeOption::from_index(value as usize)),
            Self::BlurTime(_) => Self::BlurTime(NormalizedValue::new(value)),
            Self::BlurFreq(_) => Self::BlurFreq(NormalizedValue::new(value)),
            Self::Freeze(_) => Self::Freeze(value > 0.5),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::FftSize(v) => v,
        Self::BlurTime(v) | Self::BlurFreq(v) | Self::Mix(v) => v,
        Self::Freeze(v) => v,
    }
}

impl ModuleParam for ModalResonatorParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::BaseNote(_) => "Base Note",
            Self::Spread(_) => "Spread",
            Self::Modes(_) => "Modes",
            Self::Decay(_) => "Decay",
            Self::Brightness(_) => "Brightness",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::BaseNote(n) => n.as_u8() as f32,
            Self::Spread(v) | Self::Decay(v) | Self::Brightness(v) | Self::Mix(v) => v.as_f32(),
            Self::Modes(m) => m.as_u8() as f32,
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::BaseNote(_) => Self::BaseNote(MidiNote::new(value.round() as u8)),
            Self::Spread(_) => Self::Spread(NormalizedValue::new(value)),
            Self::Modes(_) => Self::Modes(VoiceCount::new(value.round() as u8)),
            Self::Decay(_) => Self::Decay(NormalizedValue::new(value)),
            Self::Brightness(_) => Self::Brightness(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::BaseNote(v) => v,
        Self::Spread(v) | Self::Decay(v) | Self::Brightness(v) | Self::Mix(v) => {
            v
        },
        Self::Modes(v) => v,
    }
}

impl ModuleParam for ReverseGateReverbParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::WindowTime(_) => "Window",
            Self::Mode(_) => "Mode",
            Self::Trigger(_) => "Trigger",
            Self::Threshold(_) => "Threshold",
            Self::GateTime(_) => "Gate Time",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::WindowTime(ms) | Self::GateTime(ms) => ms.as_f32(),
            Self::Mode(m) => m.index() as f32,
            Self::Trigger(t) => t.index() as f32,
            Self::Threshold(db) => db.as_f32(),
            Self::Mix(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::WindowTime(_) => Self::WindowTime(Milliseconds::new(value)),
            Self::Mode(_) => {
                Self::Mode(ReverseGateMode::from_index(value as usize).unwrap_or_default())
            }
            Self::Trigger(_) => {
                Self::Trigger(ReverseGateTrigger::from_index(value as usize).unwrap_or_default())
            }
            Self::Threshold(_) => Self::Threshold(Decibels::new(value)),
            Self::GateTime(_) => Self::GateTime(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::WindowTime(v) | Self::GateTime(v) => v,
        Self::Mode(v) => v,
        Self::Trigger(v) => v,
        Self::Threshold(v) => v,
        Self::Mix(v) => v,
    }
}

impl ModuleParam for TiltEqParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Tilt(_) => "Tilt",
            Self::CenterFreq(_) => "Center",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Tilt(b) => b.as_f32(),
            Self::CenterFreq(hz) => hz.as_f32(),
            Self::Mix(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Tilt(_) => Self::Tilt(BipolarValue::new(value)),
            Self::CenterFreq(_) => Self::CenterFreq(Hertz::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Tilt(v) => v,
        Self::CenterFreq(v) => v,
        Self::Mix(v) => v,
    }
}

impl ModuleParam for UnivibeParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Feedback(_) => "Feedback",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Rate(v) => v,
        Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v,
    }
}

impl ModuleParam for CrossoverParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Frequency(_) => "Frequency",
            Self::LowGain(_) => "Low Gain",
            Self::HighGain(_) => "High Gain",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Frequency(hz) => hz.as_f32(),
            Self::LowGain(v) | Self::HighGain(v) | Self::Mix(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Frequency(_) => Self::Frequency(Hertz::new(value)),
            Self::LowGain(_) => Self::LowGain(NormalizedValue::new(value)),
            Self::HighGain(_) => Self::HighGain(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Frequency(v) => v,
        Self::LowGain(v) | Self::HighGain(v) | Self::Mix(v) => v,
    }
}

impl ModuleParam for VocoderParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Order(_) => "Order",
            Self::WindowSize(_) => "Window",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Order(v) | Self::Mix(v) => v.as_f32(),
            Self::WindowSize(ms) => ms.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Order(_) => Self::Order(NormalizedValue::new(value)),
            Self::WindowSize(_) => Self::WindowSize(Milliseconds::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Order(v) | Self::Mix(v) => v,
        Self::WindowSize(v) => v,
    }
}

impl ModuleParam for EnvelopeFollowerParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Attack(_) => "Attack",
            Self::Release(_) => "Release",
            Self::Sensitivity(_) => "Sensitivity",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Attack(ms) => ms.as_f32(),
            Self::Release(ms) => ms.as_f32(),
            Self::Sensitivity(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Attack(_) => Self::Attack(Milliseconds::new(value)),
            Self::Release(_) => Self::Release(Milliseconds::new(value)),
            Self::Sensitivity(_) => Self::Sensitivity(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Attack(v) | Self::Release(v) => v,
        Self::Sensitivity(v) => v,
    }
}

impl ModuleParam for EnvelopeParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Attack(_) => "Attack",
            Self::Decay(_) => "Decay",
            Self::Sustain(_) => "Sustain",
            Self::Release(_) => "Release",
            Self::TimeScale(_) => "Time Scale",
            Self::AttackCurve(_) => "Attack Curve",
            Self::DecayCurve(_) => "Decay Curve",
            Self::ReleaseCurve(_) => "Release Curve",
            Self::VelocitySensitivity(_) => "Velocity Sens",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Attack(t) => t.as_f32(),
            Self::Decay(t) => t.as_f32(),
            Self::Sustain(v) => v.as_f32(),
            Self::Release(t) => t.as_f32(),
            Self::TimeScale(v) => v.as_f32(),
            Self::AttackCurve(c) => c.as_f32(),
            Self::DecayCurve(c) => c.as_f32(),
            Self::ReleaseCurve(c) => c.as_f32(),
            Self::VelocitySensitivity(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Attack(_) => Self::Attack(Seconds::new(value)),
            Self::Decay(_) => Self::Decay(Seconds::new(value)),
            Self::Sustain(_) => Self::Sustain(NormalizedValue::new(value)),
            Self::Release(_) => Self::Release(Seconds::new(value)),
            Self::TimeScale(_) => Self::TimeScale(TimeScale::new(value)),
            Self::AttackCurve(_) => Self::AttackCurve(BipolarValue::new(value)),
            Self::DecayCurve(_) => Self::DecayCurve(BipolarValue::new(value)),
            Self::ReleaseCurve(_) => Self::ReleaseCurve(BipolarValue::new(value)),
            Self::VelocitySensitivity(_) => Self::VelocitySensitivity(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Attack(v) | Self::Decay(v) | Self::Release(v) => v,
        Self::TimeScale(v) => v,
        Self::Sustain(v) | Self::VelocitySensitivity(v) => v,
        Self::AttackCurve(v) | Self::DecayCurve(v) | Self::ReleaseCurve(v) => v,
    }
}

impl ModuleParam for FilterParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Mode(_) => "Mode",
            Self::Cutoff(_) => "Cutoff",
            Self::Resonance(_) => "Resonance",
            Self::KeyTracking(_) => "Key Tracking",
            Self::Drive(_) => "Drive",
            Self::EnvAmount(_) => "Env Amount",
            Self::CutoffMod(_) => "Cutoff Mod",
            Self::Model(_) => "Model",
            Self::Morph(_) => "Morph",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Mode(m) => m.index() as f32,
            Self::Cutoff(hz) => hz.as_f32(),
            Self::Resonance(r) => r.as_f32(),
            Self::KeyTracking(kt) => kt.as_f32(),
            Self::Drive(d) => d.as_f32(),
            Self::EnvAmount(e) => e.as_f32(),
            Self::CutoffMod(c) => c.as_f32(),
            Self::Model(m) => m.index() as f32,
            Self::Morph(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Mode(_) => Self::Mode(FilterMode::from_index(value as usize).unwrap_or_default()),
            Self::Cutoff(_) => Self::Cutoff(Hertz::new(value)),
            Self::Resonance(_) => Self::Resonance(NormalizedValue::new(value)),
            Self::KeyTracking(_) => Self::KeyTracking(NormalizedValue::new(value)),
            Self::Drive(_) => Self::Drive(Gain::new(value)),
            Self::EnvAmount(_) => Self::EnvAmount(BipolarValue::new(value)),
            Self::CutoffMod(_) => Self::CutoffMod(BipolarValue::new(value)),
            Self::Model(_) => {
                Self::Model(FilterModel::from_index(value as usize).unwrap_or_default())
            }
            Self::Morph(_) => Self::Morph(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Mode(v) => v,
        Self::Cutoff(v) => v,
        Self::Resonance(v) | Self::KeyTracking(v) | Self::Morph(v) => v,
        Self::Drive(v) => v,
        Self::EnvAmount(v) | Self::CutoffMod(v) => v,
        Self::Model(v) => v,
    }
}

impl ModuleParam for FofParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Vowel(_) => "Vowel",
            Self::FormantShift(_) => "Formant Shift",
            Self::Skirt(_) => "Skirt",
            Self::Bandwidth(_) => "Bandwidth",
            Self::Breathiness(_) => "Breathiness",
            Self::VibratoRate(_) => "Vibrato Rate",
            Self::VibratoDepth(_) => "Vibrato Depth",
            Self::UnisonVoices(_) => "Unison Voices",
            Self::UnisonDetune(_) => "Unison Detune",
            Self::UnisonSpread(_) => "Unison Spread",
            Self::Level(_) => "Level",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Vowel(v)
            | Self::FormantShift(v)
            | Self::Skirt(v)
            | Self::Bandwidth(v)
            | Self::Breathiness(v)
            | Self::UnisonVoices(v)
            | Self::UnisonSpread(v)
            | Self::Level(v) => v.as_f32(),
            Self::VibratoRate(hz) => hz.as_f32(),
            Self::VibratoDepth(c) | Self::UnisonDetune(c) => c.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Vowel(_) => Self::Vowel(NormalizedValue::new(value)),
            Self::FormantShift(_) => Self::FormantShift(NormalizedValue::new(value)),
            Self::Skirt(_) => Self::Skirt(NormalizedValue::new(value)),
            Self::Bandwidth(_) => Self::Bandwidth(NormalizedValue::new(value)),
            Self::Breathiness(_) => Self::Breathiness(NormalizedValue::new(value)),
            Self::VibratoRate(_) => Self::VibratoRate(Hertz::new(value)),
            Self::VibratoDepth(_) => Self::VibratoDepth(Cents::new(value)),
            Self::UnisonVoices(_) => Self::UnisonVoices(NormalizedValue::new(value)),
            Self::UnisonDetune(_) => Self::UnisonDetune(Cents::new(value)),
            Self::UnisonSpread(_) => Self::UnisonSpread(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Vowel(v)
        | Self::FormantShift(v)
        | Self::Skirt(v)
        | Self::Bandwidth(v)
        | Self::Breathiness(v)
        | Self::UnisonVoices(v)
        | Self::UnisonSpread(v)
        | Self::Level(v) => v,
        Self::VibratoRate(v) => v,
        Self::VibratoDepth(v) | Self::UnisonDetune(v) => v,
    }
}

impl ModuleParam for FooglersParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Tap1(_) => "Tap 1",
            Self::Tap2(_) => "Tap 2",
            Self::Feedback(_) => "Feedback",
            Self::Damping(_) => "Damping",
            Self::Level(_) => "Level",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Tap1(v)
            | Self::Tap2(v)
            | Self::Feedback(v)
            | Self::Damping(v)
            | Self::Level(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Tap1(_) => Self::Tap1(NormalizedValue::new(value)),
            Self::Tap2(_) => Self::Tap2(NormalizedValue::new(value)),
            Self::Feedback(_) => Self::Feedback(NormalizedValue::new(value)),
            Self::Damping(_) => Self::Damping(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Tap1(v)
        | Self::Tap2(v)
        | Self::Feedback(v)
        | Self::Damping(v)
        | Self::Level(v) => v,
    }
}

impl ModuleParam for FormantFilterParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Vowel(_) => "Vowel",
            Self::Cutoff(_) => "Cutoff",
            Self::Resonance(_) => "Resonance",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Vowel(v) => v.as_f32(),
            Self::Cutoff(hz) => hz.as_f32(),
            Self::Resonance(r) => r.as_f32(),
            Self::Mix(m) => m.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Vowel(_) => Self::Vowel(NormalizedValue::new(value)),
            Self::Cutoff(_) => Self::Cutoff(Hertz::new(value)),
            Self::Resonance(_) => Self::Resonance(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Vowel(v) | Self::Resonance(v) | Self::Mix(v) => v,
        Self::Cutoff(v) => v,
    }
}

impl ModuleParam for FractalOscParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Roughness(_) => "Roughness",
            Self::FractalSpacing(_) => "Spacing",
            Self::Dispersion(_) => "Dispersion",
            Self::Spread(_) => "Spread",
            Self::Level(_) => "Level",
            Self::GlideTime(_) => "Glide",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Roughness(v)
            | Self::FractalSpacing(v)
            | Self::Dispersion(v)
            | Self::Spread(v)
            | Self::Level(v) => v.as_f32(),
            Self::GlideTime(s) => s.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Roughness(_) => Self::Roughness(NormalizedValue::new(value)),
            Self::FractalSpacing(_) => Self::FractalSpacing(NormalizedValue::new(value)),
            Self::Dispersion(_) => Self::Dispersion(NormalizedValue::new(value)),
            Self::Spread(_) => Self::Spread(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
            Self::GlideTime(_) => Self::GlideTime(Seconds::new(value.max(0.0))),
        }
    }

    scalar_metadata_methods! {
        Self::Roughness(v)
        | Self::FractalSpacing(v)
        | Self::Dispersion(v)
        | Self::Spread(v)
        | Self::Level(v) => v,
        Self::GlideTime(v) => v,
    }
}

impl ModuleParam for FrequencyShifterParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Shift(_) => "Shift",
            Self::Mix(_) => "Mix",
            Self::Mode(_) => "Mode",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Shift(h) => h.as_f32(),
            Self::Mix(v) => v.as_f32(),
            Self::Mode(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Shift(_) => Self::Shift(Hertz::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Mode(_) => Self::Mode(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Shift(v) => v,
        Self::Mix(v) | Self::Mode(v) => v,
    }
}

impl ModuleParam for EuclideanParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Steps(_) => "Steps",
            Self::Pulses(_) => "Pulses",
            Self::Rotation(_) => "Rotation",
            Self::Swing(_) => "Swing",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Steps(n) | Self::Pulses(n) | Self::Rotation(n) => n.as_f32(),
            Self::Swing(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Steps(_) => Self::Steps(StepCount::new((value.round() as u8).clamp(1, 32))),
            Self::Pulses(_) => Self::Pulses(StepCount::new((value.round() as u8).min(32))),
            Self::Rotation(_) => Self::Rotation(StepCount::new((value.round() as u8).min(31))),
            Self::Swing(_) => Self::Swing(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Steps(v) | Self::Pulses(v) | Self::Rotation(v) => v,
        Self::Swing(v) => v,
    }
}

impl ModuleParam for TuringMachineParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::MutationRate(_) => "Mutation",
            Self::Range(_) => "Range",
            Self::Scale(_) => "Scale",
            Self::Length(_) => "Length",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::MutationRate(v) | Self::Range(v) => v.as_f32(),
            Self::Scale(s) => s.index() as f32,
            Self::Length(n) => n.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::MutationRate(_) => Self::MutationRate(NormalizedValue::new(value)),
            Self::Range(_) => Self::Range(NormalizedValue::new(value)),
            Self::Scale(_) => {
                Self::Scale(TuringScale::from_index(value as usize).unwrap_or_default())
            }
            Self::Length(_) => Self::Length(StepCount::new(if value > 12.0 { 16 } else { 8 })),
        }
    }

    scalar_metadata_methods! {
        Self::MutationRate(v) | Self::Range(v) => v,
        Self::Scale(v) => v,
        Self::Length(v) => v,
    }
}

impl ModuleParam for RandomGatesParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Density(_) => "Density",
            Self::Seed(_) => "Seed",
            Self::BurstProbability(_) => "Burst",
            Self::GateLength(_) => "Gate Len",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Density(v) | Self::BurstProbability(v) | Self::GateLength(v) => v.as_f32(),
            Self::Seed(s) => *s as f32,
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Density(_) => Self::Density(NormalizedValue::new(value)),
            Self::Seed(_) => Self::Seed(value.round() as u32),
            Self::BurstProbability(_) => Self::BurstProbability(NormalizedValue::new(value)),
            Self::GateLength(_) => Self::GateLength(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Density(v) | Self::BurstProbability(v) | Self::GateLength(v) => v,
        Self::Seed(v) => v,
    }
}

impl ModuleParam for GranularParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::GrainSize(_) => "Grain Size",
            Self::Density(_) => "Density",
            Self::Position(_) => "Position",
            Self::PositionSpread(_) => "Pos Spread",
            Self::PitchSpread(_) => "Pitch Spread",
            Self::PanSpread(_) => "Pan Spread",
            Self::Freeze(_) => "Freeze",
            Self::Window(_) => "Window",
            Self::Source(_) => "Source",
            Self::Level(_) => "Level",
            Self::GlideTime(_) => "Glide",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::GrainSize(ms) => ms.as_f32(),
            Self::Density(v)
            | Self::Position(v)
            | Self::PositionSpread(v)
            | Self::PitchSpread(v)
            | Self::PanSpread(v) => v.as_f32(),
            Self::Freeze(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Window(w) => w.index() as f32,
            Self::Source(s) => s.index() as f32,
            Self::Level(g) => g.as_f32(),
            Self::GlideTime(s) => s.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::GrainSize(_) => Self::GrainSize(Milliseconds::new(value)),
            Self::Density(_) => Self::Density(NormalizedValue::new(value)),
            Self::Position(_) => Self::Position(NormalizedValue::new(value)),
            Self::PositionSpread(_) => Self::PositionSpread(NormalizedValue::new(value)),
            Self::PitchSpread(_) => Self::PitchSpread(NormalizedValue::new(value)),
            Self::PanSpread(_) => Self::PanSpread(NormalizedValue::new(value)),
            Self::Freeze(_) => Self::Freeze(value > 0.5),
            Self::Window(_) => Self::Window(GrainWindow::from_index(value as usize)),
            Self::Source(_) => Self::Source(GrainSource::from_index(value as usize)),
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::GlideTime(_) => Self::GlideTime(Seconds::new(value.max(0.0))),
        }
    }

    scalar_metadata_methods! {
        Self::GrainSize(v) => v,
        Self::Density(v)
        | Self::Position(v)
        | Self::PositionSpread(v)
        | Self::PitchSpread(v)
        | Self::PanSpread(v) => v,
        Self::Freeze(v) => v,
        Self::Window(v) => v,
        Self::Source(v) => v,
        Self::Level(v) => v,
        Self::GlideTime(v) => v,
    }
}

impl ModuleParam for KineticParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Duration(_) => "Duration",
            Self::CurveType(_) => "Curve",
            Self::Overshoot(_) => "Overshoot",
            Self::Bipolar(_) => "Bipolar",
            Self::LoopMode(_) => "Loop Mode",
            Self::Retrigger(_) => "Retrigger",
            Self::OutputVel(_) => "Out Vel",
            Self::OutputAcc(_) => "Out Acc",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Duration(s) => s.as_f32(),
            Self::CurveType(c) => c.index() as f32,
            Self::Overshoot(v) => v.as_f32(),
            Self::Bipolar(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::LoopMode(m) => m.index() as f32,
            Self::Retrigger(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::OutputVel(v) | Self::OutputAcc(v) => *v,
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Duration(_) => Self::Duration(Seconds::new(value)),
            Self::CurveType(_) => Self::CurveType(EasingCurve::from_index(value as usize)),
            Self::Overshoot(_) => Self::Overshoot(NormalizedValue::new(value)),
            Self::Bipolar(_) => Self::Bipolar(value > 0.5),
            Self::LoopMode(_) => Self::LoopMode(KineticLoopMode::from_index(value as usize)),
            Self::Retrigger(_) => Self::Retrigger(value > 0.5),
            Self::OutputVel(_) => Self::OutputVel(value),
            Self::OutputAcc(_) => Self::OutputAcc(value),
        }
    }

    scalar_metadata_methods! {
        Self::Duration(v) => v,
        Self::CurveType(v) => v,
        Self::Overshoot(v) => v,
        Self::Bipolar(v) | Self::Retrigger(v) => v,
        Self::LoopMode(v) => v,
        Self::OutputVel(v) | Self::OutputAcc(v) => v,
    }
}

impl ModuleParam for LaSynthParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::AttackType(_) => "Attack Type",
            Self::AttackTime(_) => "Attack Time",
            Self::AttackLevel(_) => "Attack Level",
            Self::CrossfadeTime(_) => "X-Fade Time",
            Self::Brightness(_) => "Brightness",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::AttackType(v) => v.as_f32(),
            Self::AttackTime(v) => v.as_f32(),
            Self::AttackLevel(v) => v.as_f32(),
            Self::CrossfadeTime(v) => v.as_f32(),
            Self::Brightness(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::AttackType(_) => Self::AttackType(NormalizedValue::new(value)),
            Self::AttackTime(_) => Self::AttackTime(Milliseconds::new(value)),
            Self::AttackLevel(_) => Self::AttackLevel(NormalizedValue::new(value)),
            Self::CrossfadeTime(_) => Self::CrossfadeTime(Milliseconds::new(value)),
            Self::Brightness(_) => Self::Brightness(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::AttackType(v) | Self::AttackLevel(v) | Self::Brightness(v) => v,
        Self::AttackTime(v) | Self::CrossfadeTime(v) => v,
    }
}

impl ModuleParam for LfoParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Waveform(_) => "Waveform",
            Self::Rate(_) => "Rate",
            Self::Depth(_) => "Depth",
            Self::Phase(_) => "Phase",
            Self::TempoSync(_) => "Tempo Sync",
            Self::SyncDivision(_) => "Division",
            Self::Retrigger(_) => "Retrigger",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Waveform(w) => w.index() as f32,
            Self::Rate(hz) => hz.as_f32(),
            Self::Depth(d) => d.as_f32(),
            Self::Phase(p) => p.as_f32(),
            Self::TempoSync(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::SyncDivision(d) => d.as_f32(),
            Self::Retrigger(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Waveform(_) => {
                Self::Waveform(LfoWaveform::from_index(value as usize).unwrap_or_default())
            }
            Self::Rate(_) => Self::Rate(Hertz::new(value)),
            Self::Depth(_) => Self::Depth(NormalizedValue::new(value)),
            Self::Phase(_) => Self::Phase(Phase::new(value)),
            Self::TempoSync(_) => Self::TempoSync(value > 0.5),
            Self::SyncDivision(_) => Self::SyncDivision(BeatDivision::new(value)),
            Self::Retrigger(_) => Self::Retrigger(value > 0.5),
        }
    }

    scalar_metadata_methods! {
        Self::Waveform(v) => v,
        Self::Rate(v) => v,
        Self::Depth(v) => v,
        Self::Phase(v) => v,
        Self::TempoSync(v) | Self::Retrigger(v) => v,
        Self::SyncDivision(v) => v,
    }
}

impl ModuleParam for ModMatrixParam {
    fn same_kind(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::GridSize(_), Self::GridSize(_)) => true,
            (Self::SlotSource(a, _), Self::SlotSource(b, _)) => a == b,
            (Self::SlotDestination(a, _), Self::SlotDestination(b, _)) => a == b,
            (Self::SlotAmount(a, _), Self::SlotAmount(b, _)) => a == b,
            (Self::SlotEnabled(a, _), Self::SlotEnabled(b, _)) => a == b,
            _ => false,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::GridSize(_) => "Grid Size",
            Self::SlotSource(i, _) => SLOT_NAMES_SOURCE
                .get(*i as usize)
                .copied()
                .unwrap_or("Slot Source"),
            Self::SlotDestination(i, _) => SLOT_NAMES_DEST
                .get(*i as usize)
                .copied()
                .unwrap_or("Slot Dest"),
            Self::SlotAmount(i, _) => SLOT_NAMES_AMOUNT
                .get(*i as usize)
                .copied()
                .unwrap_or("Slot Amount"),
            Self::SlotEnabled(i, _) => SLOT_NAMES_ENABLED
                .get(*i as usize)
                .copied()
                .unwrap_or("Slot Enabled"),
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::GridSize(g) => g.index() as f32,
            Self::SlotSource(_, s) => s.map_or(0, |a| a.legacy_index()) as f32,
            Self::SlotDestination(_, d) => d.map_or(0, |a| a.legacy_index()) as f32,
            Self::SlotAmount(_, a) => a.as_f32(),
            Self::SlotEnabled(_, e) => {
                if *e {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::GridSize(_) => Self::GridSize(ModMatrixGridSize::from_index(value as usize)),
            Self::SlotSource(slot, _) => Self::SlotSource(
                *slot,
                SrcAddr::from_mod_source(ModSource::from_index(value as usize)),
            ),
            Self::SlotDestination(slot, _) => Self::SlotDestination(
                *slot,
                DestAddr::from_mod_destination(ModDestination::from_index(value as usize)),
            ),
            Self::SlotAmount(slot, _) => Self::SlotAmount(*slot, BipolarValue::new(value)),
            Self::SlotEnabled(slot, _) => Self::SlotEnabled(*slot, value > 0.5),
        }
    }

    scalar_metadata_methods! {
        Self::GridSize(v) => v,
        Self::SlotSource(_, v) => v,
        Self::SlotDestination(_, v) => v,
        Self::SlotAmount(_, v) => v,
        Self::SlotEnabled(_, v) => v,
    }
}

impl ModuleParam for AmplifierParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Level(_) => "Level",
            Self::Pan(_) => "Pan",
            Self::CvBipolar(_) => "CV Bipolar",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Level(g) => g.as_f32(),
            Self::Pan(p) => p.as_f32(),
            Self::CvBipolar(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::Pan(_) => Self::Pan(BipolarValue::new(value)),
            Self::CvBipolar(_) => Self::CvBipolar(value > 0.5),
        }
    }

    scalar_metadata_methods! {
        Self::Level(v) => v,
        Self::Pan(v) => v,
        Self::CvBipolar(v) => v,
    }
}

impl ModuleParam for MixerParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Input1(_) => "Input 1",
            Self::Input2(_) => "Input 2",
            Self::Input3(_) => "Input 3",
            Self::Input4(_) => "Input 4",
            Self::Input5(_) => "Input 5",
            Self::Input6(_) => "Input 6",
            Self::Input7(_) => "Input 7",
            Self::Input8(_) => "Input 8",
            Self::Master(_) => "Master",
            Self::Mute(_) => "Mute",
            Self::Limit(_) => "Limit",
            Self::Dither(_) => "Dither",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Input1(g)
            | Self::Input2(g)
            | Self::Input3(g)
            | Self::Input4(g)
            | Self::Input5(g)
            | Self::Input6(g)
            | Self::Input7(g)
            | Self::Input8(g)
            | Self::Master(g) => g.as_f32(),
            Self::Mute(b) | Self::Limit(b) | Self::Dither(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Input1(_) => Self::Input1(Gain::new(value)),
            Self::Input2(_) => Self::Input2(Gain::new(value)),
            Self::Input3(_) => Self::Input3(Gain::new(value)),
            Self::Input4(_) => Self::Input4(Gain::new(value)),
            Self::Input5(_) => Self::Input5(Gain::new(value)),
            Self::Input6(_) => Self::Input6(Gain::new(value)),
            Self::Input7(_) => Self::Input7(Gain::new(value)),
            Self::Input8(_) => Self::Input8(Gain::new(value)),
            Self::Master(_) => Self::Master(Gain::new(value)),
            Self::Mute(_) => Self::Mute(value > 0.5),
            Self::Limit(_) => Self::Limit(value > 0.5),
            Self::Dither(_) => Self::Dither(value > 0.5),
        }
    }

    scalar_metadata_methods! {
        Self::Input1(v)
        | Self::Input2(v)
        | Self::Input3(v)
        | Self::Input4(v)
        | Self::Input5(v)
        | Self::Input6(v)
        | Self::Input7(v)
        | Self::Input8(v)
        | Self::Master(v) => v,
        Self::Mute(v) | Self::Limit(v) | Self::Dither(v) => v,
    }
}

impl ModuleParam for OscilloscopeParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Time(_) => "Time",
            Self::Gain(_) => "Gain",
            Self::Trigger(_) => "Trigger",
            Self::Frozen(_) => "Frozen",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Time(t) => t.as_f32(),
            Self::Gain(g) => g.as_f32(),
            Self::Trigger(v) => v.as_f32(),
            Self::Frozen(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Time(_) => Self::Time(Seconds::new(value)),
            Self::Gain(_) => Self::Gain(Gain::new(value)),
            Self::Trigger(_) => Self::Trigger(NormalizedValue::new(value)),
            Self::Frozen(_) => Self::Frozen(value > 0.5),
        }
    }

    scalar_metadata_methods! {
        Self::Time(v) => v,
        Self::Gain(v) => v,
        Self::Trigger(v) => v,
        Self::Frozen(v) => v,
    }
}

impl ModuleParam for LevelMeterParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::PeakHold(_) => "Peak Hold",
            Self::DecayRate(_) => "Decay Rate",
            Self::ShowRms(_) => "Show RMS",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::PeakHold(t) => t.as_f32(),
            Self::DecayRate(v) => v.as_f32(),
            Self::ShowRms(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::PeakHold(_) => Self::PeakHold(Seconds::new(value)),
            Self::DecayRate(_) => Self::DecayRate(NormalizedValue::new(value)),
            Self::ShowRms(_) => Self::ShowRms(value > 0.5),
        }
    }

    scalar_metadata_methods! {
        Self::PeakHold(v) => v,
        Self::DecayRate(v) => v,
        Self::ShowRms(v) => v,
    }
}

impl ModuleParam for MsegParam {
    fn same_kind(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::SegmentCount(_), Self::SegmentCount(_))
            | (Self::SustainSegment(_), Self::SustainSegment(_))
            | (Self::LoopStart(_), Self::LoopStart(_))
            | (Self::LoopEnd(_), Self::LoopEnd(_))
            | (Self::LoopEnabled(_), Self::LoopEnabled(_))
            | (Self::TimeScale(_), Self::TimeScale(_)) => true,
            (Self::SegmentTime(a, _), Self::SegmentTime(b, _)) => a == b,
            (Self::SegmentLevel(a, _), Self::SegmentLevel(b, _)) => a == b,
            (Self::SegmentCurve(a, _), Self::SegmentCurve(b, _)) => a == b,
            _ => false,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::SegmentCount(_) => "Segments",
            Self::SustainSegment(_) => "Sustain Seg",
            Self::LoopStart(_) => "Loop Start",
            Self::LoopEnd(_) => "Loop End",
            Self::LoopEnabled(_) => "Loop",
            Self::TimeScale(_) => "Time Scale",
            Self::SegmentTime(_, _) => "Seg Time",
            Self::SegmentLevel(_, _) => "Seg Level",
            Self::SegmentCurve(_, _) => "Seg Curve",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::SegmentCount(n) => *n as f32,
            Self::SustainSegment(n) | Self::LoopStart(n) | Self::LoopEnd(n) => *n as f32,
            Self::LoopEnabled(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::TimeScale(v) => v.as_f32(),
            Self::SegmentTime(_, t) => t.as_f32(),
            Self::SegmentLevel(_, v) => v.as_f32(),
            Self::SegmentCurve(_, v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::SegmentCount(_) => Self::SegmentCount((value.round() as u8).clamp(1, 16)),
            Self::SustainSegment(_) => Self::SustainSegment((value.round() as u8).min(15)),
            Self::LoopStart(_) => Self::LoopStart((value.round() as u8).min(15)),
            Self::LoopEnd(_) => Self::LoopEnd((value.round() as u8).min(15)),
            Self::LoopEnabled(_) => Self::LoopEnabled(value > 0.5),
            Self::TimeScale(_) => Self::TimeScale(TimeScale::new(value)),
            Self::SegmentTime(idx, _) => Self::SegmentTime(*idx, Seconds::new(value)),
            Self::SegmentLevel(idx, _) => Self::SegmentLevel(*idx, NormalizedValue::new(value)),
            Self::SegmentCurve(idx, _) => Self::SegmentCurve(*idx, BipolarValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::SegmentCount(v)
        | Self::SustainSegment(v)
        | Self::LoopStart(v)
        | Self::LoopEnd(v) => v,
        Self::LoopEnabled(v) => v,
        Self::TimeScale(v) => v,
        Self::SegmentTime(_, v) => v,
        Self::SegmentLevel(_, v) => v,
        Self::SegmentCurve(_, v) => v,
    }
}

impl ModuleParam for NoiseParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Type(_) => "Type",
            Self::Level(_) => "Level",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Type(t) => t.index() as f32,
            Self::Level(g) => g.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Type(_) => Self::Type(NoiseType::from_index(value as usize).unwrap_or_default()),
            Self::Level(_) => Self::Level(Gain::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Type(v) => v,
        Self::Level(v) => v,
    }
}

impl ModuleParam for OscillatorParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Waveform(_) => "Waveform",
            Self::Frequency(_) => "Frequency",
            Self::Detune(_) => "Detune",
            Self::Octave(_) => "Octave",
            Self::PulseWidth(_) => "Pulse Width",
            Self::Level(_) => "Level",
            Self::Phase(_) => "Phase",
            Self::FmMode(_) => "FM Mode",
            Self::FmAmount(_) => "FM Amount",
            Self::UnisonVoices(_) => "Unison",
            Self::UnisonDetune(_) => "Uni Detune",
            Self::UnisonSpread(_) => "Uni Spread",
            Self::UnisonPhaseRandom(_) => "Uni Phase",
            Self::CrossModAmount(_) => "X-Mod",
            Self::AntiAlias(_) => "Anti-Alias",
            Self::GlideTime(_) => "Glide",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Waveform(w) => w.index() as f32,
            Self::Frequency(hz) => hz.as_f32(),
            Self::Detune(c) => c.as_f32(),
            Self::Octave(s) => s.as_f32(),
            Self::PulseWidth(pw) => pw.as_f32(),
            Self::Level(g) => g.as_f32(),
            Self::Phase(p) => p.as_f32(),
            Self::FmMode(m) => m.index() as f32,
            Self::FmAmount(a) => a.as_f32(),
            Self::UnisonVoices(n) => f32::from(*n),
            Self::UnisonDetune(c) => c.as_f32(),
            Self::UnisonSpread(v) => v.as_f32(),
            Self::UnisonPhaseRandom(v) => v.as_f32(),
            Self::CrossModAmount(v) => v.as_f32(),
            #[allow(clippy::cast_precision_loss)]
            Self::AntiAlias(m) => m.index() as f32,
            Self::GlideTime(s) => s.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Waveform(_) => {
                Self::Waveform(Waveform::from_index(value as usize).unwrap_or_default())
            }
            Self::Frequency(_) => Self::Frequency(Hertz::new(value)),
            // Clamp via the single-source detune preset; mirrors the
            // `UnisonDetune` clamp below so every apply path stays in range.
            Self::Detune(_) => Self::Detune(Cents::new(Cents::DETUNE_RANGE.clamp(value))),
            Self::Octave(_) => Self::Octave(Semitones::new(value)),
            Self::PulseWidth(_) => Self::PulseWidth(PulseWidth::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::Phase(_) => Self::Phase(Phase::new(value)),
            Self::FmMode(_) => Self::FmMode(FmMode::from_index(value as usize).unwrap_or_default()),
            Self::FmAmount(_) => Self::FmAmount(BipolarValue::new(value)),
            #[allow(clippy::cast_possible_truncation)]
            Self::UnisonVoices(_) => Self::UnisonVoices((value.round() as u8).clamp(1, 7)),
            Self::UnisonDetune(_) => {
                Self::UnisonDetune(Cents::new(Cents::UNISON_DETUNE_RANGE.clamp(value)))
            }
            Self::UnisonSpread(_) => Self::UnisonSpread(NormalizedValue::new(value)),
            Self::UnisonPhaseRandom(_) => Self::UnisonPhaseRandom(NormalizedValue::new(value)),
            Self::CrossModAmount(_) => Self::CrossModAmount(NormalizedValue::new(value)),
            #[allow(clippy::cast_possible_truncation)]
            Self::AntiAlias(_) => Self::AntiAlias(AntiAliasMode::from_index(value as usize)),
            Self::GlideTime(_) => Self::GlideTime(Seconds::new(value.max(0.0))),
        }
    }

    scalar_metadata_methods! {
        Self::Waveform(v) => v,
        Self::Frequency(v) => v,
        Self::Detune(v) | Self::UnisonDetune(v) => v,
        Self::Octave(v) => v,
        Self::PulseWidth(v) => v,
        Self::Level(v) => v,
        Self::Phase(v) => v,
        Self::FmMode(v) => v,
        Self::FmAmount(v) => v,
        Self::UnisonVoices(v) => v,
        Self::UnisonSpread(v) | Self::UnisonPhaseRandom(v) | Self::CrossModAmount(v) => {
            v
        },
        Self::AntiAlias(v) => v,
        Self::GlideTime(v) => v,
    }
}

impl ModuleParam for MathOscillatorParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Algorithm(_) => "Algorithm",
            Self::Frequency(_) => "Frequency",
            Self::ParamA(_) => "Param A",
            Self::ParamB(_) => "Param B",
            Self::ParamC(_) => "Param C",
            Self::Level(_) => "Level",
            Self::GlideTime(_) => "Glide",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Algorithm(a) => a.index() as f32,
            Self::Frequency(hz) => hz.as_f32(),
            Self::ParamA(v) => v.as_f32(),
            Self::ParamB(v) => v.as_f32(),
            Self::ParamC(v) => v.as_f32(),
            Self::Level(g) => g.as_f32(),
            Self::GlideTime(s) => s.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Algorithm(_) => {
                Self::Algorithm(MathAlgo::from_index(value as usize).unwrap_or_default())
            }
            Self::Frequency(_) => Self::Frequency(Hertz::new(value)),
            Self::ParamA(_) => Self::ParamA(NormalizedValue::new(value)),
            Self::ParamB(_) => Self::ParamB(NormalizedValue::new(value)),
            Self::ParamC(_) => Self::ParamC(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::GlideTime(_) => Self::GlideTime(Seconds::new(value.max(0.0))),
        }
    }

    scalar_metadata_methods! {
        Self::Algorithm(v) => v,
        Self::Frequency(v) => v,
        Self::ParamA(v) | Self::ParamB(v) | Self::ParamC(v) => v,
        Self::Level(v) => v,
        Self::GlideTime(v) => v,
    }
}

impl ModuleParam for PadSynthParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Bandwidth(_) => "Bandwidth",
            Self::Tilt(_) => "Tilt",
            Self::Detune(_) => "Detune",
            Self::BaseFreq(_) => "Base Freq",
            Self::Level(_) => "Level",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Bandwidth(v) | Self::Tilt(v) | Self::Detune(v) | Self::Level(v) => v.as_f32(),
            Self::BaseFreq(hz) => hz.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Bandwidth(_) => Self::Bandwidth(NormalizedValue::new(value)),
            Self::Tilt(_) => Self::Tilt(NormalizedValue::new(value)),
            Self::Detune(_) => Self::Detune(NormalizedValue::new(value)),
            Self::BaseFreq(_) => Self::BaseFreq(Hertz::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Bandwidth(v) | Self::Tilt(v) | Self::Detune(v) | Self::Level(v) => {
            v
        },
        Self::BaseFreq(v) => v,
    }
}

impl ModuleParam for PhaseVocoderParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::PitchShift(_) => "Pitch Shift",
            Self::Freeze(_) => "Freeze",
            Self::FftSize(_) => "FFT Size",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::PitchShift(st) => st.as_f32(),
            Self::Freeze(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::FftSize(f) => f.index() as f32,
            Self::Mix(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::PitchShift(_) => Self::PitchShift(Semitones::new(value)),
            Self::Freeze(_) => Self::Freeze(value > 0.5),
            Self::FftSize(_) => Self::FftSize(FftSizeOption::from_index(value as usize)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::PitchShift(v) => v,
        Self::Freeze(v) => v,
        Self::FftSize(v) => v,
        Self::Mix(v) => v,
    }
}

impl ModuleParam for KeyboardPannerParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Spread(_) => "Spread",
            Self::CenterNote(_) => "Center",
            Self::Curve(_) => "Curve",
            Self::Invert(_) => "Invert",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Spread(v) => v.as_f32(),
            Self::CenterNote(n) => f32::from(n.as_u8()),
            Self::Curve(c) => c.as_f32(),
            Self::Invert(p) => {
                if p.is_inverted() {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Spread(_) => Self::Spread(NormalizedValue::new(value)),
            Self::CenterNote(_) => {
                Self::CenterNote(MidiNote::new(value.round().clamp(0.0, 127.0) as u8))
            }
            Self::Curve(_) => Self::Curve(BipolarValue::new(value)),
            Self::Invert(_) => Self::Invert(Polarity::from(value > 0.5)),
        }
    }

    scalar_metadata_methods! {
        Self::Spread(v) => v,
        Self::CenterNote(v) => v,
        Self::Curve(v) => v,
        Self::Invert(v) => v,
    }
}

impl ModuleParam for SpatialPannerParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::X(_) => "X",
            Self::Y(_) => "Y",
            Self::Z(_) => "Z",
            Self::Diffusion(_) => "Diffusion",
            Self::ErLevel(_) => "ER Level",
            Self::DirectLevel(_) => "Direct Level",
            Self::Absorption(_) => "Absorption",
            Self::AirAbsorption(_) => "Air Absorption",
            Self::Distance(_) => "Distance",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::X(v) | Self::Y(v) | Self::Z(v) => v.as_f32(),
            Self::Diffusion(v)
            | Self::ErLevel(v)
            | Self::DirectLevel(v)
            | Self::Absorption(v)
            | Self::AirAbsorption(v)
            | Self::Distance(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::X(_) => Self::X(BipolarValue::new(value)),
            Self::Y(_) => Self::Y(BipolarValue::new(value)),
            Self::Z(_) => Self::Z(BipolarValue::new(value)),
            Self::Diffusion(_) => Self::Diffusion(NormalizedValue::new(value)),
            Self::ErLevel(_) => Self::ErLevel(NormalizedValue::new(value)),
            Self::DirectLevel(_) => Self::DirectLevel(NormalizedValue::new(value)),
            Self::Absorption(_) => Self::Absorption(NormalizedValue::new(value)),
            Self::AirAbsorption(_) => Self::AirAbsorption(NormalizedValue::new(value)),
            Self::Distance(_) => Self::Distance(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::X(v) | Self::Y(v) | Self::Z(v) => v,
        Self::Diffusion(v)
        | Self::ErLevel(v)
        | Self::DirectLevel(v)
        | Self::Absorption(v)
        | Self::AirAbsorption(v)
        | Self::Distance(v) => v,
    }
}

impl ModuleParam for BodyResonanceParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Frequency(_) => "Freq",
            Self::Resonance(_) => "Resonance",
            Self::Size(_) => "Size",
            Self::Brightness(_) => "Bright",
            Self::Mix(_) => "Mix",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Frequency(f) => f.as_f32(),
            Self::Resonance(v) | Self::Size(v) | Self::Brightness(v) | Self::Mix(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Frequency(_) => Self::Frequency(Hertz::new(value.clamp(50.0, 2000.0))),
            Self::Resonance(_) => Self::Resonance(NormalizedValue::new(value)),
            Self::Size(_) => Self::Size(NormalizedValue::new(value)),
            Self::Brightness(_) => Self::Brightness(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Frequency(v) => v,
        Self::Resonance(v) | Self::Size(v) | Self::Brightness(v) | Self::Mix(v) => {
            v
        }
    }
}

impl ModuleParam for MechanicalNoiseParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::NoiseType(_) => "Type",
            Self::Duration(_) => "Duration",
            Self::Cutoff(_) => "Cutoff",
            Self::VelocitySens(_) => "Vel Sens",
            Self::Level(_) => "Level",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::NoiseType(t) => t.index() as f32,
            Self::Duration(d) => d.as_f32(),
            Self::Cutoff(f) => f.as_f32(),
            Self::VelocitySens(v) => v.as_f32(),
            Self::Level(g) => g.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::NoiseType(_) => {
                Self::NoiseType(MechanicalNoiseType::from_index(value as usize).unwrap_or_default())
            }
            Self::Duration(_) => Self::Duration(Milliseconds::new(value.clamp(1.0, 100.0))),
            Self::Cutoff(_) => Self::Cutoff(Hertz::new(value.clamp(100.0, 10000.0))),
            Self::VelocitySens(_) => Self::VelocitySens(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(Gain::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::NoiseType(v) => v,
        Self::Duration(v) => v,
        Self::Cutoff(v) => v,
        Self::VelocitySens(v) => v,
        Self::Level(v) => v,
    }
}

impl ModuleParam for PitchTrackerParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Sensitivity(_) => "Sensitivity",
            Self::MinFreq(_) => "Min Freq",
            Self::MaxFreq(_) => "Max Freq",
            Self::Smoothing(_) => "Smoothing",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Sensitivity(v) => v.as_f32(),
            Self::MinFreq(h) => h.as_f32(),
            Self::MaxFreq(h) => h.as_f32(),
            Self::Smoothing(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Sensitivity(_) => Self::Sensitivity(NormalizedValue::new(value)),
            Self::MinFreq(_) => Self::MinFreq(Hertz::new(value)),
            Self::MaxFreq(_) => Self::MaxFreq(Hertz::new(value)),
            Self::Smoothing(_) => Self::Smoothing(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Sensitivity(v) | Self::Smoothing(v) => v,
        Self::MinFreq(v) | Self::MaxFreq(v) => v,
    }
}

impl ModuleParam for RingModParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::CarrierFreq(_) => "Carrier Freq",
            Self::CarrierWaveform(_) => "Carrier Wave",
            Self::Mix(_) => "Mix",
            Self::FreqRatio(_) => "Freq Ratio",
            Self::TrackKeyboard(_) => "Key Track",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::CarrierFreq(h) => h.as_f32(),
            Self::CarrierWaveform(w) => w.index() as f32,
            Self::Mix(v) => v.as_f32(),
            Self::FreqRatio(v) => v.as_f32(),
            Self::TrackKeyboard(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::CarrierFreq(_) => Self::CarrierFreq(Hertz::new(value)),
            Self::CarrierWaveform(_) => {
                Self::CarrierWaveform(Waveform::from_index(value as usize).unwrap_or_default())
            }
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::FreqRatio(_) => Self::FreqRatio(NormalizedValue::new(value)),
            Self::TrackKeyboard(_) => Self::TrackKeyboard(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::CarrierFreq(v) => v,
        Self::CarrierWaveform(v) => v,
        Self::Mix(v) | Self::FreqRatio(v) | Self::TrackKeyboard(v) => v,
    }
}

impl ModuleParam for SamplerParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::SampleSelect(_) => "Sample",
            Self::PitchTracking(_) => "Pitch Track",
            Self::Level(_) => "Level",
            Self::PlayMode(_) => "Play Mode",
            Self::Direction(_) => "Direction",
            Self::VelocitySensitivity(_) => "Vel Sens",
            Self::FineTune(_) => "Fine Tune",
            Self::StartOffset(_) => "Start Offset",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::SampleSelect(_) => 0.0, // Not meaningful as f32
            Self::PitchTracking(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Level(g) => g.as_f32(),
            Self::PlayMode(m) => *m as u8 as f32,
            Self::Direction(d) => *d as u8 as f32,
            Self::VelocitySensitivity(v) => v.as_f32(),
            Self::FineTune(c) => c.as_f32(),
            Self::StartOffset(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::SampleSelect(_) => Self::SampleSelect(SampleId::new(value as u64)),
            Self::PitchTracking(_) => Self::PitchTracking(value > 0.5),
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::PlayMode(_) => {
                let mode = match value as u8 {
                    0 => SamplerPlayMode::OneShot,
                    1 => SamplerPlayMode::Sustain,
                    _ => SamplerPlayMode::Loop,
                };
                Self::PlayMode(mode)
            }
            Self::Direction(_) => {
                let dir = match value as u8 {
                    0 => PlayDirection::Forward,
                    1 => PlayDirection::Reverse,
                    _ => PlayDirection::PingPong,
                };
                Self::Direction(dir)
            }
            Self::VelocitySensitivity(_) => Self::VelocitySensitivity(NormalizedValue::new(value)),
            Self::FineTune(_) => Self::FineTune(Cents::new(value)),
            Self::StartOffset(_) => Self::StartOffset(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::SampleSelect(v) => v,
        Self::PitchTracking(v) => v,
        Self::Level(v) => v,
        Self::PlayMode(v) => v,
        Self::Direction(v) => v,
        Self::VelocitySensitivity(v) | Self::StartOffset(v) => v,
        Self::FineTune(v) => v,
    }
}

impl ModuleParam for SidOscillatorParam {
    fn same_kind(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::SeqStep(a, _), Self::SeqStep(b, _))
            | (Self::SeqStepFreq(a, _), Self::SeqStepFreq(b, _)) => a == b,
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Triangle(_) => "Triangle",
            Self::Sawtooth(_) => "Sawtooth",
            Self::Pulse(_) => "Pulse",
            Self::Noise(_) => "Noise",
            Self::NoiseSeed(_) => "Noise Seed",
            Self::FreqReg(_) => "Freq Reg",
            Self::TrackVoicePitch(_) => "Track Pitch",
            Self::PulseWidthReg(_) => "PW Reg",
            Self::Test(_) => "Test",
            Self::RingMod(_) => "Ring Mod",
            Self::HardSync(_) => "Hard Sync",
            Self::Model(_) => "Model",
            Self::Clock(_) => "Clock",
            Self::Quality(_) => "Quality",
            Self::DcBlock(_) => "DC Block",
            Self::Level(_) => "Level",
            Self::SeqLength(_) => "Seq Length",
            Self::SeqRate(_) => "Seq Rate",
            Self::SeqLoop(_) => "Seq Loop",
            Self::SeqStep(..) => "Seq Step",
            Self::SeqFreqMask(_) => "Seq Freq Mask",
            Self::SeqStepFreq(..) => "Seq Step Freq",
            Self::GlideTime(_) => "Glide",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Triangle(b)
            | Self::Sawtooth(b)
            | Self::Pulse(b)
            | Self::Noise(b)
            | Self::TrackVoicePitch(b)
            | Self::Test(b)
            | Self::RingMod(b)
            | Self::HardSync(b)
            | Self::DcBlock(b)
            | Self::SeqLoop(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            #[allow(clippy::cast_precision_loss)]
            Self::FreqReg(v) | Self::PulseWidthReg(v) | Self::SeqStepFreq(_, v) => *v as f32,
            Self::SeqFreqMask(mask) => *mask as f32,
            Self::NoiseSeed(seed) => seed.as_u32() as f32,
            #[allow(clippy::cast_precision_loss)]
            Self::Model(m) => m.index() as f32,
            #[allow(clippy::cast_precision_loss)]
            Self::Clock(c) => c.index() as f32,
            #[allow(clippy::cast_precision_loss)]
            Self::Quality(q) => q.index() as f32,
            Self::Level(g) => g.as_f32(),
            Self::SeqLength(n) | Self::SeqRate(n) | Self::SeqStep(_, n) => f32::from(*n),
            Self::GlideTime(s) => s.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        let as_bool = value > 0.5;
        match self {
            Self::Triangle(_) => Self::Triangle(as_bool),
            Self::Sawtooth(_) => Self::Sawtooth(as_bool),
            Self::Pulse(_) => Self::Pulse(as_bool),
            Self::Noise(_) => Self::Noise(as_bool),
            Self::NoiseSeed(_) => Self::NoiseSeed(SidNoiseSeed::new(value.round().max(1.0) as u32)),
            #[allow(clippy::cast_precision_loss)]
            Self::FreqReg(_) => {
                Self::FreqReg(value.round().clamp(0.0, SID_FREQ_REG_MAX as f32) as u32)
            }
            Self::TrackVoicePitch(_) => Self::TrackVoicePitch(as_bool),
            #[allow(clippy::cast_precision_loss)]
            Self::PulseWidthReg(_) => {
                Self::PulseWidthReg(value.round().clamp(0.0, SID_PW_REG_MAX as f32) as u32)
            }
            Self::Test(_) => Self::Test(as_bool),
            Self::RingMod(_) => Self::RingMod(as_bool),
            Self::HardSync(_) => Self::HardSync(as_bool),
            Self::Model(_) => Self::Model(SidModel::from_index(value.round().max(0.0) as usize)),
            Self::Clock(_) => Self::Clock(SidClock::from_index(value.round().max(0.0) as usize)),
            Self::Quality(_) => {
                Self::Quality(SidQuality::from_index(value.round().max(0.0) as usize))
            }
            Self::DcBlock(_) => Self::DcBlock(as_bool),
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::SeqLength(_) => {
                Self::SeqLength(value.round().clamp(0.0, SID_SEQ_STEPS as f32) as u8)
            }
            Self::SeqRate(_) => Self::SeqRate(value.round().clamp(1.0, 16.0) as u8),
            Self::SeqLoop(_) => Self::SeqLoop(as_bool),
            Self::SeqStep(i, _) => Self::SeqStep(*i, value.round().clamp(0.0, 15.0) as u8),
            Self::SeqFreqMask(_) => Self::SeqFreqMask(value.round().clamp(0.0, 65535.0) as u32),
            #[allow(clippy::cast_precision_loss)]
            Self::SeqStepFreq(i, _) => {
                Self::SeqStepFreq(*i, value.round().clamp(0.0, SID_FREQ_REG_MAX as f32) as u32)
            }
            Self::GlideTime(_) => Self::GlideTime(Seconds::new(value.max(0.0))),
        }
    }

    scalar_metadata_methods! {
        Self::Triangle(v)
        | Self::Sawtooth(v)
        | Self::Pulse(v)
        | Self::Noise(v)
        | Self::TrackVoicePitch(v)
        | Self::Test(v)
        | Self::RingMod(v)
        | Self::HardSync(v)
        | Self::DcBlock(v)
        | Self::SeqLoop(v) => v,
        Self::NoiseSeed(v) => v,
        Self::FreqReg(v) | Self::PulseWidthReg(v) | Self::SeqStepFreq(_, v) => v,
        Self::Model(v) => v,
        Self::Clock(v) => v,
        Self::Quality(v) => v,
        Self::Level(v) => v,
        Self::SeqLength(v) | Self::SeqRate(v) | Self::SeqStep(_, v) => v,
        Self::SeqFreqMask(v) => v,
        Self::GlideTime(v) => v,
    }
}

impl ModuleParam for SignalMonitorParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Time(_) => "Time",
            Self::Gain(_) => "Gain",
            Self::Trigger(_) => "Trigger",
            Self::Frozen(_) => "Frozen",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Time(t) => t.as_f32(),
            Self::Gain(g) => g.as_f32(),
            Self::Trigger(v) => v.as_f32(),
            Self::Frozen(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Time(_) => Self::Time(Seconds::new(value)),
            Self::Gain(_) => Self::Gain(Gain::new(value)),
            Self::Trigger(_) => Self::Trigger(NormalizedValue::new(value)),
            Self::Frozen(_) => Self::Frozen(value > 0.5),
        }
    }

    scalar_metadata_methods! {
        Self::Time(v) => v,
        Self::Gain(v) => v,
        Self::Trigger(v) => v,
        Self::Frozen(v) => v,
    }
}

impl ModuleParam for SpectrumAnalyzerParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Gain(_) => "Gain",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Gain(g) => g.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Gain(_) => Self::Gain(Gain::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Gain(v) => v,
    }
}

impl ModuleParam for SubOscParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Waveform(_) => "Waveform",
            Self::Octave(_) => "Octave",
            Self::Level(_) => "Level",
            Self::GlideTime(_) => "Glide",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Waveform(w) => w.index() as f32,
            Self::Octave(o) => o.index() as f32,
            Self::Level(g) => g.as_f32(),
            Self::GlideTime(s) => s.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Waveform(_) => {
                Self::Waveform(SubOscWaveform::from_index(value as usize).unwrap_or_default())
            }
            Self::Octave(_) => {
                Self::Octave(SubOscOctave::from_index(value as usize).unwrap_or_default())
            }
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::GlideTime(_) => Self::GlideTime(Seconds::new(value.max(0.0))),
        }
    }

    scalar_metadata_methods! {
        Self::Waveform(v) => v,
        Self::Octave(v) => v,
        Self::Level(v) => v,
        Self::GlideTime(v) => v,
    }
}

impl ModuleParam for VectorMixerParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::X(_) => "X",
            Self::Y(_) => "Y",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::X(v) => v.as_f32(),
            Self::Y(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::X(_) => Self::X(BipolarValue::new(value)),
            Self::Y(_) => Self::Y(BipolarValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::X(v) | Self::Y(v) => v,
    }
}

impl ModuleParam for VocalTractParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Tongue(_) => "Tongue",
            Self::Constriction(_) => "Constriction",
            Self::Lips(_) => "Lips",
            Self::Length(_) => "Length",
            Self::Nasality(_) => "Nasality",
            Self::Breathiness(_) => "Breathiness",
            Self::Level(_) => "Level",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Tongue(v)
            | Self::Constriction(v)
            | Self::Lips(v)
            | Self::Length(v)
            | Self::Nasality(v)
            | Self::Breathiness(v)
            | Self::Level(v) => v.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Tongue(_) => Self::Tongue(NormalizedValue::new(value)),
            Self::Constriction(_) => Self::Constriction(NormalizedValue::new(value)),
            Self::Lips(_) => Self::Lips(NormalizedValue::new(value)),
            Self::Length(_) => Self::Length(NormalizedValue::new(value)),
            Self::Nasality(_) => Self::Nasality(NormalizedValue::new(value)),
            Self::Breathiness(_) => Self::Breathiness(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Tongue(v)
        | Self::Constriction(v)
        | Self::Lips(v)
        | Self::Length(v)
        | Self::Nasality(v)
        | Self::Breathiness(v)
        | Self::Level(v) => v,
    }
}

impl ModuleParam for VoiceSynthParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Vowel(_) => "Vowel",
            Self::FormantShift(_) => "Formant Shift",
            Self::Breathiness(_) => "Breathiness",
            Self::OpenQuotient(_) => "Open Quotient",
            Self::Tilt(_) => "Tilt",
            Self::VibratoRate(_) => "Vibrato Rate",
            Self::VibratoDepth(_) => "Vibrato Depth",
            Self::UnisonVoices(_) => "Unison Voices",
            Self::UnisonDetune(_) => "Unison Detune",
            Self::UnisonSpread(_) => "Unison Spread",
            Self::Level(_) => "Level",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Vowel(v)
            | Self::FormantShift(v)
            | Self::Breathiness(v)
            | Self::OpenQuotient(v)
            | Self::Tilt(v)
            | Self::UnisonVoices(v)
            | Self::UnisonSpread(v)
            | Self::Level(v) => v.as_f32(),
            Self::VibratoRate(hz) => hz.as_f32(),
            Self::VibratoDepth(c) | Self::UnisonDetune(c) => c.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Vowel(_) => Self::Vowel(NormalizedValue::new(value)),
            Self::FormantShift(_) => Self::FormantShift(NormalizedValue::new(value)),
            Self::Breathiness(_) => Self::Breathiness(NormalizedValue::new(value)),
            Self::OpenQuotient(_) => Self::OpenQuotient(NormalizedValue::new(value)),
            Self::Tilt(_) => Self::Tilt(NormalizedValue::new(value)),
            Self::VibratoRate(_) => Self::VibratoRate(Hertz::new(value)),
            Self::VibratoDepth(_) => Self::VibratoDepth(Cents::new(value)),
            Self::UnisonVoices(_) => Self::UnisonVoices(NormalizedValue::new(value)),
            Self::UnisonDetune(_) => Self::UnisonDetune(Cents::new(value)),
            Self::UnisonSpread(_) => Self::UnisonSpread(NormalizedValue::new(value)),
            Self::Level(_) => Self::Level(NormalizedValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Vowel(v)
        | Self::FormantShift(v)
        | Self::Breathiness(v)
        | Self::OpenQuotient(v)
        | Self::Tilt(v)
        | Self::UnisonVoices(v)
        | Self::UnisonSpread(v)
        | Self::Level(v) => v,
        Self::VibratoRate(v) => v,
        Self::VibratoDepth(v) | Self::UnisonDetune(v) => v,
    }
}

impl ModuleParam for WaveshaperParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Curve(_) => "Curve",
            Self::Drive(_) => "Drive",
            Self::Mix(_) => "Mix",
            Self::Bias(_) => "Bias",
            Self::Symmetry(_) => "Symmetry",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Curve(c) => c.index() as f32,
            Self::Drive(v) | Self::Mix(v) => v.as_f32(),
            Self::Bias(b) | Self::Symmetry(b) => b.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Curve(_) => {
                Self::Curve(WaveshaperCurve::from_index(value as usize).unwrap_or_default())
            }
            Self::Drive(_) => Self::Drive(NormalizedValue::new(value)),
            Self::Mix(_) => Self::Mix(NormalizedValue::new(value)),
            Self::Bias(_) => Self::Bias(BipolarValue::new(value)),
            Self::Symmetry(_) => Self::Symmetry(BipolarValue::new(value)),
        }
    }

    scalar_metadata_methods! {
        Self::Curve(v) => v,
        Self::Drive(v) | Self::Mix(v) => v,
        Self::Bias(v) | Self::Symmetry(v) => v,
    }
}

impl ModuleParam for WavetableParam {
    fn same_kind(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Table(_) => "Table",
            Self::Position(_) => "Position",
            Self::Detune(_) => "Detune",
            Self::Octave(_) => "Octave",
            Self::Level(_) => "Level",
            Self::GlideTime(_) => "Glide",
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Table(t) => t.index() as f32,
            Self::Position(v) => v.as_f32(),
            Self::Detune(c) => c.as_f32(),
            Self::Octave(o) => o.as_i32() as f32,
            Self::Level(g) => g.as_f32(),
            Self::GlideTime(s) => s.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Table(_) => {
                Self::Table(WavetableSelect::from_index(value as usize).unwrap_or_default())
            }
            Self::Position(_) => Self::Position(NormalizedValue::new(value)),
            Self::Detune(_) => Self::Detune(Cents::new(value)),
            Self::Octave(_) => Self::Octave(Octaves::new(value.round() as i32)),
            Self::Level(_) => Self::Level(Gain::new(value)),
            Self::GlideTime(_) => Self::GlideTime(Seconds::new(value.max(0.0))),
        }
    }

    scalar_metadata_methods! {
        Self::Table(v) => v,
        Self::Position(v) => v,
        Self::Detune(v) => v,
        Self::Octave(v) => v,
        Self::Level(v) => v,
        Self::GlideTime(v) => v,
    }
}

impl ModuleParam for Param {
    fn same_kind(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Oscillator(a), Self::Oscillator(b)) => a.same_kind(b),
            (Self::MathOscillator(a), Self::MathOscillator(b)) => a.same_kind(b),
            (Self::SubOsc(a), Self::SubOsc(b)) => a.same_kind(b),
            (Self::Noise(a), Self::Noise(b)) => a.same_kind(b),
            (Self::Filter(a), Self::Filter(b)) => a.same_kind(b),
            (Self::Envelope(a), Self::Envelope(b)) => a.same_kind(b),
            (Self::Lfo(a), Self::Lfo(b)) => a.same_kind(b),
            (Self::Amplifier(a), Self::Amplifier(b)) => a.same_kind(b),
            (Self::Mixer(a), Self::Mixer(b)) => a.same_kind(b),
            (Self::Delay(a), Self::Delay(b)) => a.same_kind(b),
            (Self::Reverb(a), Self::Reverb(b)) => a.same_kind(b),
            (Self::Distortion(a), Self::Distortion(b)) => a.same_kind(b),
            (Self::Chorus(a), Self::Chorus(b)) => a.same_kind(b),
            (Self::Phaser(a), Self::Phaser(b)) => a.same_kind(b),
            (Self::Flanger(a), Self::Flanger(b)) => a.same_kind(b),
            (Self::Compressor(a), Self::Compressor(b)) => a.same_kind(b),
            (Self::Eq(a), Self::Eq(b)) => a.same_kind(b),
            (Self::Waveshaper(a), Self::Waveshaper(b)) => a.same_kind(b),
            (Self::Oscilloscope(a), Self::Oscilloscope(b)) => a.same_kind(b),
            (Self::LevelMeter(a), Self::LevelMeter(b)) => a.same_kind(b),
            (Self::SpectrumAnalyzer(a), Self::SpectrumAnalyzer(b)) => a.same_kind(b),
            (Self::ModMatrix(a), Self::ModMatrix(b)) => a.same_kind(b),
            // Modulation / Utility
            (Self::RingMod(a), Self::RingMod(b)) => a.same_kind(b),
            (Self::EnvelopeFollower(a), Self::EnvelopeFollower(b)) => a.same_kind(b),
            (Self::WavetableOsc(a), Self::WavetableOsc(b)) => a.same_kind(b),
            (Self::Mseg(a), Self::Mseg(b)) => a.same_kind(b),
            (Self::AdditiveOsc(a), Self::AdditiveOsc(b)) => a.same_kind(b),
            // Effects (continued)
            (Self::BbdDelay(a), Self::BbdDelay(b)) => a.same_kind(b),
            (Self::MidSide(a), Self::MidSide(b)) => a.same_kind(b),
            (Self::Limiter(a), Self::Limiter(b)) => a.same_kind(b),
            // Generative
            (Self::Euclidean(a), Self::Euclidean(b)) => a.same_kind(b),
            (Self::TuringMachine(a), Self::TuringMachine(b)) => a.same_kind(b),
            (Self::RandomGates(a), Self::RandomGates(b)) => a.same_kind(b),
            // Physical modeling
            (Self::KeyboardPanner(a), Self::KeyboardPanner(b)) => a.same_kind(b),
            (Self::BodyResonance(a), Self::BodyResonance(b)) => a.same_kind(b),
            (Self::MechanicalNoise(a), Self::MechanicalNoise(b)) => a.same_kind(b),
            (Self::GranularOsc(a), Self::GranularOsc(b)) => a.same_kind(b),
            (Self::Convolver(a), Self::Convolver(b)) => a.same_kind(b),
            (Self::PhaseVocoder(a), Self::PhaseVocoder(b)) => a.same_kind(b),
            (Self::Kinetic(a), Self::Kinetic(b)) => a.same_kind(b),
            (Self::SignalMonitor(a), Self::SignalMonitor(b)) => a.same_kind(b),
            (Self::FrequencyShifter(a), Self::FrequencyShifter(b)) => a.same_kind(b),
            (Self::VectorMixer(a), Self::VectorMixer(b)) => a.same_kind(b),
            (Self::LaSynth(a), Self::LaSynth(b)) => a.same_kind(b),
            (Self::PitchTracker(a), Self::PitchTracker(b)) => a.same_kind(b),
            (Self::EnsembleChorus(a), Self::EnsembleChorus(b)) => a.same_kind(b),
            (Self::ShimmerReverb(a), Self::ShimmerReverb(b)) => a.same_kind(b),
            (Self::GranularFx(a), Self::GranularFx(b)) => a.same_kind(b),
            (Self::SpectralBlur(a), Self::SpectralBlur(b)) => a.same_kind(b),
            (Self::ModalResonator(a), Self::ModalResonator(b)) => a.same_kind(b),
            (Self::ReverseGateReverb(a), Self::ReverseGateReverb(b)) => a.same_kind(b),
            (Self::FractalOsc(a), Self::FractalOsc(b)) => a.same_kind(b),
            (Self::Sampler(a), Self::Sampler(b)) => a.same_kind(b),
            // MusicDSP additions
            (Self::DriftGenerator(a), Self::DriftGenerator(b)) => a.same_kind(b),
            (Self::ChaoticOsc(a), Self::ChaoticOsc(b)) => a.same_kind(b),
            (Self::FormantFilter(a), Self::FormantFilter(b)) => a.same_kind(b),
            (Self::Fooglers(a), Self::Fooglers(b)) => a.same_kind(b),
            (Self::BeatDetector(a), Self::BeatDetector(b)) => a.same_kind(b),
            (Self::PadSynth(a), Self::PadSynth(b)) => a.same_kind(b),
            (Self::AmFormant(a), Self::AmFormant(b)) => a.same_kind(b),
            (Self::TiltEq(a), Self::TiltEq(b)) => a.same_kind(b),
            (Self::Univibe(a), Self::Univibe(b)) => a.same_kind(b),
            (Self::Crossover(a), Self::Crossover(b)) => a.same_kind(b),
            (Self::Vocoder(a), Self::Vocoder(b)) => a.same_kind(b),
            (Self::TransientShaper(a), Self::TransientShaper(b)) => a.same_kind(b),
            (Self::VoiceSynth(a), Self::VoiceSynth(b)) => a.same_kind(b),
            (Self::VocalTract(a), Self::VocalTract(b)) => a.same_kind(b),
            (Self::Fof(a), Self::Fof(b)) => a.same_kind(b),
            (Self::SidOscillator(a), Self::SidOscillator(b)) => a.same_kind(b),
            (Self::SpatialPanner(a), Self::SpatialPanner(b)) => a.same_kind(b),
            (Self::Script(a), Self::Script(b)) => a.same_kind(b),
            _ => false,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Oscillator(p) => p.name(),
            Self::MathOscillator(p) => p.name(),
            Self::SubOsc(p) => p.name(),
            Self::Noise(p) => p.name(),
            Self::Filter(p) => p.name(),
            Self::Envelope(p) => p.name(),
            Self::Lfo(p) => p.name(),
            Self::Amplifier(p) => p.name(),
            Self::Mixer(p) => p.name(),
            Self::Delay(p) => p.name(),
            Self::Reverb(p) => p.name(),
            Self::Distortion(p) => p.name(),
            Self::Chorus(p) => p.name(),
            Self::Phaser(p) => p.name(),
            Self::Flanger(p) => p.name(),
            Self::Compressor(p) => p.name(),
            Self::Eq(p) => p.name(),
            Self::Waveshaper(p) => p.name(),
            Self::Oscilloscope(p) => p.name(),
            Self::LevelMeter(p) => p.name(),
            Self::SpectrumAnalyzer(p) => p.name(),
            Self::ModMatrix(p) => p.name(),
            // Modulation / Utility
            Self::RingMod(p) => p.name(),
            Self::EnvelopeFollower(p) => p.name(),
            Self::WavetableOsc(p) => p.name(),
            Self::Mseg(p) => p.name(),
            Self::AdditiveOsc(p) => p.name(),
            // Effects (continued)
            Self::BbdDelay(p) => p.name(),
            Self::MidSide(p) => p.name(),
            Self::Limiter(p) => p.name(),
            // Generative
            Self::Euclidean(p) => p.name(),
            Self::TuringMachine(p) => p.name(),
            Self::RandomGates(p) => p.name(),
            // Physical modeling
            Self::KeyboardPanner(p) => p.name(),
            Self::BodyResonance(p) => p.name(),
            Self::MechanicalNoise(p) => p.name(),
            Self::GranularOsc(p) => p.name(),
            Self::Convolver(p) => p.name(),
            Self::PhaseVocoder(p) => p.name(),
            Self::Kinetic(p) => p.name(),
            Self::SignalMonitor(p) => p.name(),
            Self::FrequencyShifter(p) => p.name(),
            Self::VectorMixer(p) => p.name(),
            Self::LaSynth(p) => p.name(),
            Self::PitchTracker(p) => p.name(),
            Self::EnsembleChorus(p) => p.name(),
            Self::ShimmerReverb(p) => p.name(),
            Self::GranularFx(p) => p.name(),
            Self::SpectralBlur(p) => p.name(),
            Self::ModalResonator(p) => p.name(),
            Self::ReverseGateReverb(p) => p.name(),
            Self::FractalOsc(p) => p.name(),
            Self::Sampler(p) => p.name(),
            // MusicDSP additions
            Self::DriftGenerator(p) => p.name(),
            Self::ChaoticOsc(p) => p.name(),
            Self::FormantFilter(p) => p.name(),
            Self::Fooglers(p) => p.name(),
            Self::BeatDetector(p) => p.name(),
            Self::PadSynth(p) => p.name(),
            Self::AmFormant(p) => p.name(),
            Self::TiltEq(p) => p.name(),
            Self::Univibe(p) => p.name(),
            Self::Crossover(p) => p.name(),
            Self::Vocoder(p) => p.name(),
            Self::TransientShaper(p) => p.name(),
            Self::VoiceSynth(p) => p.name(),
            Self::VocalTract(p) => p.name(),
            Self::Fof(p) => p.name(),
            Self::SidOscillator(p) => p.name(),
            Self::SpatialPanner(p) => p.name(),
            Self::Script(p) => p.name(),
        }
    }

    fn as_f32(&self) -> f32 {
        match self {
            Self::Oscillator(p) => p.as_f32(),
            Self::MathOscillator(p) => p.as_f32(),
            Self::SubOsc(p) => p.as_f32(),
            Self::Noise(p) => p.as_f32(),
            Self::Filter(p) => p.as_f32(),
            Self::Envelope(p) => p.as_f32(),
            Self::Lfo(p) => p.as_f32(),
            Self::Amplifier(p) => p.as_f32(),
            Self::Mixer(p) => p.as_f32(),
            Self::Delay(p) => p.as_f32(),
            Self::Reverb(p) => p.as_f32(),
            Self::Distortion(p) => p.as_f32(),
            Self::Chorus(p) => p.as_f32(),
            Self::Phaser(p) => p.as_f32(),
            Self::Flanger(p) => p.as_f32(),
            Self::Compressor(p) => p.as_f32(),
            Self::Eq(p) => p.as_f32(),
            Self::Waveshaper(p) => p.as_f32(),
            Self::Oscilloscope(p) => p.as_f32(),
            Self::LevelMeter(p) => p.as_f32(),
            Self::SpectrumAnalyzer(p) => p.as_f32(),
            Self::ModMatrix(p) => p.as_f32(),
            // Modulation / Utility
            Self::RingMod(p) => p.as_f32(),
            Self::EnvelopeFollower(p) => p.as_f32(),
            Self::WavetableOsc(p) => p.as_f32(),
            Self::Mseg(p) => p.as_f32(),
            Self::AdditiveOsc(p) => p.as_f32(),
            // Effects (continued)
            Self::BbdDelay(p) => p.as_f32(),
            Self::MidSide(p) => p.as_f32(),
            Self::Limiter(p) => p.as_f32(),
            // Generative
            Self::Euclidean(p) => p.as_f32(),
            Self::TuringMachine(p) => p.as_f32(),
            Self::RandomGates(p) => p.as_f32(),
            // Physical modeling
            Self::KeyboardPanner(p) => p.as_f32(),
            Self::BodyResonance(p) => p.as_f32(),
            Self::MechanicalNoise(p) => p.as_f32(),
            Self::GranularOsc(p) => p.as_f32(),
            Self::Convolver(p) => p.as_f32(),
            Self::PhaseVocoder(p) => p.as_f32(),
            Self::Kinetic(p) => p.as_f32(),
            Self::SignalMonitor(p) => p.as_f32(),
            Self::FrequencyShifter(p) => p.as_f32(),
            Self::VectorMixer(p) => p.as_f32(),
            Self::LaSynth(p) => p.as_f32(),
            Self::PitchTracker(p) => p.as_f32(),
            Self::EnsembleChorus(p) => p.as_f32(),
            Self::ShimmerReverb(p) => p.as_f32(),
            Self::GranularFx(p) => p.as_f32(),
            Self::SpectralBlur(p) => p.as_f32(),
            Self::ModalResonator(p) => p.as_f32(),
            Self::ReverseGateReverb(p) => p.as_f32(),
            Self::FractalOsc(p) => p.as_f32(),
            Self::Sampler(p) => p.as_f32(),
            // MusicDSP additions
            Self::DriftGenerator(p) => p.as_f32(),
            Self::ChaoticOsc(p) => p.as_f32(),
            Self::FormantFilter(p) => p.as_f32(),
            Self::Fooglers(p) => p.as_f32(),
            Self::BeatDetector(p) => p.as_f32(),
            Self::PadSynth(p) => p.as_f32(),
            Self::AmFormant(p) => p.as_f32(),
            Self::TiltEq(p) => p.as_f32(),
            Self::Univibe(p) => p.as_f32(),
            Self::Crossover(p) => p.as_f32(),
            Self::Vocoder(p) => p.as_f32(),
            Self::TransientShaper(p) => p.as_f32(),
            Self::VoiceSynth(p) => p.as_f32(),
            Self::VocalTract(p) => p.as_f32(),
            Self::Fof(p) => p.as_f32(),
            Self::SidOscillator(p) => p.as_f32(),
            Self::SpatialPanner(p) => p.as_f32(),
            Self::Script(p) => p.as_f32(),
        }
    }

    fn with_f32(&self, value: f32) -> Self {
        match self {
            Self::Oscillator(p) => Self::Oscillator(p.with_f32(value)),
            Self::MathOscillator(p) => Self::MathOscillator(p.with_f32(value)),
            Self::SubOsc(p) => Self::SubOsc(p.with_f32(value)),
            Self::Noise(p) => Self::Noise(p.with_f32(value)),
            Self::Filter(p) => Self::Filter(p.with_f32(value)),
            Self::Envelope(p) => Self::Envelope(p.with_f32(value)),
            Self::Lfo(p) => Self::Lfo(p.with_f32(value)),
            Self::Amplifier(p) => Self::Amplifier(p.with_f32(value)),
            Self::Mixer(p) => Self::Mixer(p.with_f32(value)),
            Self::Delay(p) => Self::Delay(p.with_f32(value)),
            Self::Reverb(p) => Self::Reverb(p.with_f32(value)),
            Self::Distortion(p) => Self::Distortion(p.with_f32(value)),
            Self::Chorus(p) => Self::Chorus(p.with_f32(value)),
            Self::Phaser(p) => Self::Phaser(p.with_f32(value)),
            Self::Flanger(p) => Self::Flanger(p.with_f32(value)),
            Self::Compressor(p) => Self::Compressor(p.with_f32(value)),
            Self::Eq(p) => Self::Eq(p.with_f32(value)),
            Self::Waveshaper(p) => Self::Waveshaper(p.with_f32(value)),
            Self::Oscilloscope(p) => Self::Oscilloscope(p.with_f32(value)),
            Self::LevelMeter(p) => Self::LevelMeter(p.with_f32(value)),
            Self::SpectrumAnalyzer(p) => Self::SpectrumAnalyzer(p.with_f32(value)),
            Self::ModMatrix(p) => Self::ModMatrix(p.with_f32(value)),
            // Modulation / Utility
            Self::RingMod(p) => Self::RingMod(p.with_f32(value)),
            Self::EnvelopeFollower(p) => Self::EnvelopeFollower(p.with_f32(value)),
            Self::WavetableOsc(p) => Self::WavetableOsc(p.with_f32(value)),
            Self::Mseg(p) => Self::Mseg(p.with_f32(value)),
            Self::AdditiveOsc(p) => Self::AdditiveOsc(p.with_f32(value)),
            // Effects (continued)
            Self::BbdDelay(p) => Self::BbdDelay(p.with_f32(value)),
            Self::MidSide(p) => Self::MidSide(p.with_f32(value)),
            Self::Limiter(p) => Self::Limiter(p.with_f32(value)),
            // Generative
            Self::Euclidean(p) => Self::Euclidean(p.with_f32(value)),
            Self::TuringMachine(p) => Self::TuringMachine(p.with_f32(value)),
            Self::RandomGates(p) => Self::RandomGates(p.with_f32(value)),
            // Physical modeling
            Self::KeyboardPanner(p) => Self::KeyboardPanner(p.with_f32(value)),
            Self::BodyResonance(p) => Self::BodyResonance(p.with_f32(value)),
            Self::MechanicalNoise(p) => Self::MechanicalNoise(p.with_f32(value)),
            Self::GranularOsc(p) => Self::GranularOsc(p.with_f32(value)),
            Self::Convolver(p) => Self::Convolver(p.with_f32(value)),
            Self::PhaseVocoder(p) => Self::PhaseVocoder(p.with_f32(value)),
            Self::Kinetic(p) => Self::Kinetic(p.with_f32(value)),
            Self::SignalMonitor(p) => Self::SignalMonitor(p.with_f32(value)),
            Self::FrequencyShifter(p) => Self::FrequencyShifter(p.with_f32(value)),
            Self::VectorMixer(p) => Self::VectorMixer(p.with_f32(value)),
            Self::LaSynth(p) => Self::LaSynth(p.with_f32(value)),
            Self::PitchTracker(p) => Self::PitchTracker(p.with_f32(value)),
            Self::EnsembleChorus(p) => Self::EnsembleChorus(p.with_f32(value)),
            Self::ShimmerReverb(p) => Self::ShimmerReverb(p.with_f32(value)),
            Self::GranularFx(p) => Self::GranularFx(p.with_f32(value)),
            Self::SpectralBlur(p) => Self::SpectralBlur(p.with_f32(value)),
            Self::ModalResonator(p) => Self::ModalResonator(p.with_f32(value)),
            Self::ReverseGateReverb(p) => Self::ReverseGateReverb(p.with_f32(value)),
            Self::FractalOsc(p) => Self::FractalOsc(p.with_f32(value)),
            Self::Sampler(p) => Self::Sampler(p.with_f32(value)),
            // MusicDSP additions
            Self::DriftGenerator(p) => Self::DriftGenerator(p.with_f32(value)),
            Self::ChaoticOsc(p) => Self::ChaoticOsc(p.with_f32(value)),
            Self::FormantFilter(p) => Self::FormantFilter(p.with_f32(value)),
            Self::Fooglers(p) => Self::Fooglers(p.with_f32(value)),
            Self::BeatDetector(p) => Self::BeatDetector(p.with_f32(value)),
            Self::PadSynth(p) => Self::PadSynth(p.with_f32(value)),
            Self::AmFormant(p) => Self::AmFormant(p.with_f32(value)),
            Self::TiltEq(p) => Self::TiltEq(p.with_f32(value)),
            Self::Univibe(p) => Self::Univibe(p.with_f32(value)),
            Self::Crossover(p) => Self::Crossover(p.with_f32(value)),
            Self::Vocoder(p) => Self::Vocoder(p.with_f32(value)),
            Self::TransientShaper(p) => Self::TransientShaper(p.with_f32(value)),
            Self::VoiceSynth(p) => Self::VoiceSynth(p.with_f32(value)),
            Self::VocalTract(p) => Self::VocalTract(p.with_f32(value)),
            Self::Fof(p) => Self::Fof(p.with_f32(value)),
            Self::SidOscillator(p) => Self::SidOscillator(p.with_f32(value)),
            Self::SpatialPanner(p) => Self::SpatialPanner(p.with_f32(value)),
            Self::Script(p) => Self::Script(p.with_f32(value)),
        }
    }

    /// Value-kind classifier for this parameter (delegates to the module enum).
    fn kind(&self) -> ParamKind {
        match self {
            Self::Oscillator(p) => p.kind(),
            Self::MathOscillator(p) => p.kind(),
            Self::SubOsc(p) => p.kind(),
            Self::Noise(p) => p.kind(),
            Self::Filter(p) => p.kind(),
            Self::Envelope(p) => p.kind(),
            Self::Lfo(p) => p.kind(),
            Self::Amplifier(p) => p.kind(),
            Self::Mixer(p) => p.kind(),
            Self::Delay(p) => p.kind(),
            Self::Reverb(p) => p.kind(),
            Self::Distortion(p) => p.kind(),
            Self::Chorus(p) => p.kind(),
            Self::Phaser(p) => p.kind(),
            Self::Flanger(p) => p.kind(),
            Self::Compressor(p) => p.kind(),
            Self::Eq(p) => p.kind(),
            Self::Waveshaper(p) => p.kind(),
            Self::Oscilloscope(p) => p.kind(),
            Self::LevelMeter(p) => p.kind(),
            Self::SpectrumAnalyzer(p) => p.kind(),
            Self::ModMatrix(p) => p.kind(),
            Self::RingMod(p) => p.kind(),
            Self::EnvelopeFollower(p) => p.kind(),
            Self::WavetableOsc(p) => p.kind(),
            Self::Mseg(p) => p.kind(),
            Self::AdditiveOsc(p) => p.kind(),
            Self::BbdDelay(p) => p.kind(),
            Self::MidSide(p) => p.kind(),
            Self::Limiter(p) => p.kind(),
            Self::Euclidean(p) => p.kind(),
            Self::TuringMachine(p) => p.kind(),
            Self::RandomGates(p) => p.kind(),
            Self::KeyboardPanner(p) => p.kind(),
            Self::BodyResonance(p) => p.kind(),
            Self::MechanicalNoise(p) => p.kind(),
            Self::GranularOsc(p) => p.kind(),
            Self::Convolver(p) => p.kind(),
            Self::PhaseVocoder(p) => p.kind(),
            Self::Kinetic(p) => p.kind(),
            Self::SignalMonitor(p) => p.kind(),
            Self::FrequencyShifter(p) => p.kind(),
            Self::VectorMixer(p) => p.kind(),
            Self::LaSynth(p) => p.kind(),
            Self::PitchTracker(p) => p.kind(),
            Self::EnsembleChorus(p) => p.kind(),
            Self::ShimmerReverb(p) => p.kind(),
            Self::GranularFx(p) => p.kind(),
            Self::SpectralBlur(p) => p.kind(),
            Self::ModalResonator(p) => p.kind(),
            Self::ReverseGateReverb(p) => p.kind(),
            Self::FractalOsc(p) => p.kind(),
            Self::Sampler(p) => p.kind(),
            Self::DriftGenerator(p) => p.kind(),
            Self::ChaoticOsc(p) => p.kind(),
            Self::FormantFilter(p) => p.kind(),
            Self::Fooglers(p) => p.kind(),
            Self::BeatDetector(p) => p.kind(),
            Self::PadSynth(p) => p.kind(),
            Self::AmFormant(p) => p.kind(),
            Self::TiltEq(p) => p.kind(),
            Self::Univibe(p) => p.kind(),
            Self::Crossover(p) => p.kind(),
            Self::Vocoder(p) => p.kind(),
            Self::TransientShaper(p) => p.kind(),
            Self::VoiceSynth(p) => p.kind(),
            Self::VocalTract(p) => p.kind(),
            Self::Fof(p) => p.kind(),
            Self::SidOscillator(p) => p.kind(),
            Self::SpatialPanner(p) => p.kind(),
            Self::Script(p) => p.kind(),
        }
    }

    /// Natural display unit for this parameter (delegates to the module enum).
    fn unit(&self) -> ParameterUnit {
        match self {
            Self::Oscillator(p) => p.unit(),
            Self::MathOscillator(p) => p.unit(),
            Self::SubOsc(p) => p.unit(),
            Self::Noise(p) => p.unit(),
            Self::Filter(p) => p.unit(),
            Self::Envelope(p) => p.unit(),
            Self::Lfo(p) => p.unit(),
            Self::Amplifier(p) => p.unit(),
            Self::Mixer(p) => p.unit(),
            Self::Delay(p) => p.unit(),
            Self::Reverb(p) => p.unit(),
            Self::Distortion(p) => p.unit(),
            Self::Chorus(p) => p.unit(),
            Self::Phaser(p) => p.unit(),
            Self::Flanger(p) => p.unit(),
            Self::Compressor(p) => p.unit(),
            Self::Eq(p) => p.unit(),
            Self::Waveshaper(p) => p.unit(),
            Self::Oscilloscope(p) => p.unit(),
            Self::LevelMeter(p) => p.unit(),
            Self::SpectrumAnalyzer(p) => p.unit(),
            Self::ModMatrix(p) => p.unit(),
            Self::RingMod(p) => p.unit(),
            Self::EnvelopeFollower(p) => p.unit(),
            Self::WavetableOsc(p) => p.unit(),
            Self::Mseg(p) => p.unit(),
            Self::AdditiveOsc(p) => p.unit(),
            Self::BbdDelay(p) => p.unit(),
            Self::MidSide(p) => p.unit(),
            Self::Limiter(p) => p.unit(),
            Self::Euclidean(p) => p.unit(),
            Self::TuringMachine(p) => p.unit(),
            Self::RandomGates(p) => p.unit(),
            Self::KeyboardPanner(p) => p.unit(),
            Self::BodyResonance(p) => p.unit(),
            Self::MechanicalNoise(p) => p.unit(),
            Self::GranularOsc(p) => p.unit(),
            Self::Convolver(p) => p.unit(),
            Self::PhaseVocoder(p) => p.unit(),
            Self::Kinetic(p) => p.unit(),
            Self::SignalMonitor(p) => p.unit(),
            Self::FrequencyShifter(p) => p.unit(),
            Self::VectorMixer(p) => p.unit(),
            Self::LaSynth(p) => p.unit(),
            Self::PitchTracker(p) => p.unit(),
            Self::EnsembleChorus(p) => p.unit(),
            Self::ShimmerReverb(p) => p.unit(),
            Self::GranularFx(p) => p.unit(),
            Self::SpectralBlur(p) => p.unit(),
            Self::ModalResonator(p) => p.unit(),
            Self::ReverseGateReverb(p) => p.unit(),
            Self::FractalOsc(p) => p.unit(),
            Self::Sampler(p) => p.unit(),
            Self::DriftGenerator(p) => p.unit(),
            Self::ChaoticOsc(p) => p.unit(),
            Self::FormantFilter(p) => p.unit(),
            Self::Fooglers(p) => p.unit(),
            Self::BeatDetector(p) => p.unit(),
            Self::PadSynth(p) => p.unit(),
            Self::AmFormant(p) => p.unit(),
            Self::TiltEq(p) => p.unit(),
            Self::Univibe(p) => p.unit(),
            Self::Crossover(p) => p.unit(),
            Self::Vocoder(p) => p.unit(),
            Self::TransientShaper(p) => p.unit(),
            Self::VoiceSynth(p) => p.unit(),
            Self::VocalTract(p) => p.unit(),
            Self::Fof(p) => p.unit(),
            Self::SidOscillator(p) => p.unit(),
            Self::SpatialPanner(p) => p.unit(),
            Self::Script(p) => p.unit(),
        }
    }

    /// Suggested response curve (advisory; delegates to the module enum).
    fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Oscillator(p) => p.default_curve(),
            Self::MathOscillator(p) => p.default_curve(),
            Self::SubOsc(p) => p.default_curve(),
            Self::Noise(p) => p.default_curve(),
            Self::Filter(p) => p.default_curve(),
            Self::Envelope(p) => p.default_curve(),
            Self::Lfo(p) => p.default_curve(),
            Self::Amplifier(p) => p.default_curve(),
            Self::Mixer(p) => p.default_curve(),
            Self::Delay(p) => p.default_curve(),
            Self::Reverb(p) => p.default_curve(),
            Self::Distortion(p) => p.default_curve(),
            Self::Chorus(p) => p.default_curve(),
            Self::Phaser(p) => p.default_curve(),
            Self::Flanger(p) => p.default_curve(),
            Self::Compressor(p) => p.default_curve(),
            Self::Eq(p) => p.default_curve(),
            Self::Waveshaper(p) => p.default_curve(),
            Self::Oscilloscope(p) => p.default_curve(),
            Self::LevelMeter(p) => p.default_curve(),
            Self::SpectrumAnalyzer(p) => p.default_curve(),
            Self::ModMatrix(p) => p.default_curve(),
            Self::RingMod(p) => p.default_curve(),
            Self::EnvelopeFollower(p) => p.default_curve(),
            Self::WavetableOsc(p) => p.default_curve(),
            Self::Mseg(p) => p.default_curve(),
            Self::AdditiveOsc(p) => p.default_curve(),
            Self::BbdDelay(p) => p.default_curve(),
            Self::MidSide(p) => p.default_curve(),
            Self::Limiter(p) => p.default_curve(),
            Self::Euclidean(p) => p.default_curve(),
            Self::TuringMachine(p) => p.default_curve(),
            Self::RandomGates(p) => p.default_curve(),
            Self::KeyboardPanner(p) => p.default_curve(),
            Self::BodyResonance(p) => p.default_curve(),
            Self::MechanicalNoise(p) => p.default_curve(),
            Self::GranularOsc(p) => p.default_curve(),
            Self::Convolver(p) => p.default_curve(),
            Self::PhaseVocoder(p) => p.default_curve(),
            Self::Kinetic(p) => p.default_curve(),
            Self::SignalMonitor(p) => p.default_curve(),
            Self::FrequencyShifter(p) => p.default_curve(),
            Self::VectorMixer(p) => p.default_curve(),
            Self::LaSynth(p) => p.default_curve(),
            Self::PitchTracker(p) => p.default_curve(),
            Self::EnsembleChorus(p) => p.default_curve(),
            Self::ShimmerReverb(p) => p.default_curve(),
            Self::GranularFx(p) => p.default_curve(),
            Self::SpectralBlur(p) => p.default_curve(),
            Self::ModalResonator(p) => p.default_curve(),
            Self::ReverseGateReverb(p) => p.default_curve(),
            Self::FractalOsc(p) => p.default_curve(),
            Self::Sampler(p) => p.default_curve(),
            Self::DriftGenerator(p) => p.default_curve(),
            Self::ChaoticOsc(p) => p.default_curve(),
            Self::FormantFilter(p) => p.default_curve(),
            Self::Fooglers(p) => p.default_curve(),
            Self::BeatDetector(p) => p.default_curve(),
            Self::PadSynth(p) => p.default_curve(),
            Self::AmFormant(p) => p.default_curve(),
            Self::TiltEq(p) => p.default_curve(),
            Self::Univibe(p) => p.default_curve(),
            Self::Crossover(p) => p.default_curve(),
            Self::Vocoder(p) => p.default_curve(),
            Self::TransientShaper(p) => p.default_curve(),
            Self::VoiceSynth(p) => p.default_curve(),
            Self::VocalTract(p) => p.default_curve(),
            Self::Fof(p) => p.default_curve(),
            Self::SidOscillator(p) => p.default_curve(),
            Self::SpatialPanner(p) => p.default_curve(),
            Self::Script(p) => p.default_curve(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<P: ModuleParam>(param: &P) -> P {
        param.with_f32(param.as_f32())
    }

    #[test]
    fn generic_module_param_round_trip() {
        let oscillator = OscillatorParam::Frequency(crate::Hertz::A4);
        assert_eq!(ModuleParam::kind(&oscillator), ParamKind::Continuous);
        assert!(round_trip(&oscillator).same_kind(&oscillator));

        let aggregate = Param::Oscillator(oscillator);
        assert_eq!(
            <Param as ModuleParam>::kind(&aggregate),
            ParamKind::Continuous
        );
    }
}
