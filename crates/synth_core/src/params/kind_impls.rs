//! Per-enum `kind()` / `unit()` methods + the aggregate `Param` delegation
//! (Phase 1, step 2 of the parameter type-system plan). Generated to mirror
//! each enum's variant structure; the value field is dispatched through
//! [`ScalarParam`] on the bound value, so kind/unit follow the field type.
//!
//! Two-field `Variant(u8, T)` binds the value field `v` (the leading index is
//! structural). See `docs/param-kinds.md` for the authoritative mapping.

use super::*;
use crate::module_traits::{ParamKind, ParameterUnit, ResponseCurve, ScalarParam};

impl AdditiveParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Tilt(v)
            | Self::OddEven(v)
            | Self::Brightness(v)
            | Self::Stretch(v)
            | Self::Randomize(v)
            | Self::Level(v) => v.scalar_kind(),
            Self::GlideTime(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Tilt(v)
            | Self::OddEven(v)
            | Self::Brightness(v)
            | Self::Stretch(v)
            | Self::Randomize(v)
            | Self::Level(v) => v.scalar_unit(),
            Self::GlideTime(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Tilt(v)
            | Self::OddEven(v)
            | Self::Brightness(v)
            | Self::Stretch(v)
            | Self::Randomize(v)
            | Self::Level(v) => v.scalar_curve(),
            Self::GlideTime(v) => v.scalar_curve(),
        }
    }
}

impl AmFormantParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Vowel(v) | Self::CarrierRatio(v) | Self::Depth(v) | Self::Level(v) => {
                v.scalar_kind()
            }
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Vowel(v) | Self::CarrierRatio(v) | Self::Depth(v) | Self::Level(v) => {
                v.scalar_unit()
            }
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Vowel(v) | Self::CarrierRatio(v) | Self::Depth(v) | Self::Level(v) => {
                v.scalar_curve()
            }
        }
    }
}

impl BeatDetectorParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Sensitivity(v) => v.scalar_kind(),
            Self::FilterFreq(v) => v.scalar_kind(),
            Self::HoldTime(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Sensitivity(v) => v.scalar_unit(),
            Self::FilterFreq(v) => v.scalar_unit(),
            Self::HoldTime(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Sensitivity(v) => v.scalar_curve(),
            Self::FilterFreq(v) => v.scalar_curve(),
            Self::HoldTime(v) => v.scalar_curve(),
        }
    }
}

impl ChaoticOscParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::System(v) => v.scalar_kind(),
            Self::Rate(v) => v.scalar_kind(),
            Self::Chaos(v) | Self::Depth(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::System(v) => v.scalar_unit(),
            Self::Rate(v) => v.scalar_unit(),
            Self::Chaos(v) | Self::Depth(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::System(v) => v.scalar_curve(),
            Self::Rate(v) => v.scalar_curve(),
            Self::Chaos(v) | Self::Depth(v) => v.scalar_curve(),
        }
    }
}

impl ConvolverParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Ir(v) => v.scalar_kind(),
            Self::Mix(v) | Self::Brightness(v) | Self::DynamicMode(v) => v.scalar_kind(),
            Self::PreDelay(v) => v.scalar_kind(),
            Self::DecayTrim(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Ir(v) => v.scalar_unit(),
            Self::Mix(v) | Self::Brightness(v) | Self::DynamicMode(v) => v.scalar_unit(),
            Self::PreDelay(v) => v.scalar_unit(),
            Self::DecayTrim(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Ir(v) => v.scalar_curve(),
            Self::Mix(v) | Self::Brightness(v) | Self::DynamicMode(v) => v.scalar_curve(),
            Self::PreDelay(v) => v.scalar_curve(),
            Self::DecayTrim(v) => v.scalar_curve(),
        }
    }
}

impl DriftGeneratorParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Rate(v) => v.scalar_kind(),
            Self::Depth(v) | Self::Smoothness(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Rate(v) => v.scalar_unit(),
            Self::Depth(v) | Self::Smoothness(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Rate(v) => v.scalar_curve(),
            Self::Depth(v) | Self::Smoothness(v) => v.scalar_curve(),
        }
    }
}

impl DelayParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Mode(v) => v.scalar_kind(),
            Self::Time(v) | Self::TimeLeft(v) | Self::TimeRight(v) => v.scalar_kind(),
            Self::Feedback(v) | Self::Mix(v) | Self::Damping(v) => v.scalar_kind(),
            Self::TempoSync(v) => v.scalar_kind(),
            Self::SyncDivision(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Mode(v) => v.scalar_unit(),
            Self::Time(v) | Self::TimeLeft(v) | Self::TimeRight(v) => v.scalar_unit(),
            Self::Feedback(v) | Self::Mix(v) | Self::Damping(v) => v.scalar_unit(),
            Self::TempoSync(v) => v.scalar_unit(),
            Self::SyncDivision(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Mode(v) => v.scalar_curve(),
            Self::Time(v) | Self::TimeLeft(v) | Self::TimeRight(v) => v.scalar_curve(),
            Self::Feedback(v) | Self::Mix(v) | Self::Damping(v) => v.scalar_curve(),
            Self::TempoSync(v) => v.scalar_curve(),
            Self::SyncDivision(v) => v.scalar_curve(),
        }
    }
}

impl ReverbParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::RoomSize(v)
            | Self::Damping(v)
            | Self::Width(v)
            | Self::Mix(v)
            | Self::Decay(v)
            | Self::Diffusion(v) => v.scalar_kind(),
            Self::PreDelay(v) => v.scalar_kind(),
            Self::LowCut(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::RoomSize(v)
            | Self::Damping(v)
            | Self::Width(v)
            | Self::Mix(v)
            | Self::Decay(v)
            | Self::Diffusion(v) => v.scalar_unit(),
            Self::PreDelay(v) => v.scalar_unit(),
            Self::LowCut(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::RoomSize(v)
            | Self::Damping(v)
            | Self::Width(v)
            | Self::Mix(v)
            | Self::Decay(v)
            | Self::Diffusion(v) => v.scalar_curve(),
            Self::PreDelay(v) => v.scalar_curve(),
            Self::LowCut(v) => v.scalar_curve(),
        }
    }
}

impl DistortionParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Mode(v) => v.scalar_kind(),
            Self::Drive(v) | Self::Tone(v) | Self::Mix(v) => v.scalar_kind(),
            Self::BitDepth(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Mode(v) => v.scalar_unit(),
            Self::Drive(v) | Self::Tone(v) | Self::Mix(v) => v.scalar_unit(),
            Self::BitDepth(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Mode(v) => v.scalar_curve(),
            Self::Drive(v) | Self::Tone(v) | Self::Mix(v) => v.scalar_curve(),
            Self::BitDepth(v) => v.scalar_curve(),
        }
    }
}

impl ChorusParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Rate(v) => v.scalar_kind(),
            Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v.scalar_kind(),
            Self::Delay(v) => v.scalar_kind(),
            Self::Voices(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Rate(v) => v.scalar_unit(),
            Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v.scalar_unit(),
            Self::Delay(v) => v.scalar_unit(),
            Self::Voices(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Rate(v) => v.scalar_curve(),
            Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v.scalar_curve(),
            Self::Delay(v) => v.scalar_curve(),
            Self::Voices(v) => v.scalar_curve(),
        }
    }
}

impl PhaserParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Rate(v) | Self::CenterFreq(v) => v.scalar_kind(),
            Self::Depth(v) | Self::Mix(v) => v.scalar_kind(),
            Self::Feedback(v) => v.scalar_kind(),
            Self::Stages(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Rate(v) | Self::CenterFreq(v) => v.scalar_unit(),
            Self::Depth(v) | Self::Mix(v) => v.scalar_unit(),
            Self::Feedback(v) => v.scalar_unit(),
            Self::Stages(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Rate(v) | Self::CenterFreq(v) => v.scalar_curve(),
            Self::Depth(v) | Self::Mix(v) => v.scalar_curve(),
            Self::Feedback(v) => v.scalar_curve(),
            Self::Stages(v) => v.scalar_curve(),
        }
    }
}

impl FlangerParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Rate(v) => v.scalar_kind(),
            Self::Depth(v) | Self::Mix(v) => v.scalar_kind(),
            Self::Feedback(v) => v.scalar_kind(),
            Self::Delay(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Rate(v) => v.scalar_unit(),
            Self::Depth(v) | Self::Mix(v) => v.scalar_unit(),
            Self::Feedback(v) => v.scalar_unit(),
            Self::Delay(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Rate(v) => v.scalar_curve(),
            Self::Depth(v) | Self::Mix(v) => v.scalar_curve(),
            Self::Feedback(v) => v.scalar_curve(),
            Self::Delay(v) => v.scalar_curve(),
        }
    }
}

impl CompressorParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Threshold(v) | Self::Makeup(v) => v.scalar_kind(),
            Self::Ratio(v) => v.scalar_kind(),
            Self::Attack(v) | Self::Release(v) => v.scalar_kind(),
            Self::Mix(v) => v.scalar_kind(),
            Self::SidechainEnabled(v) => v.scalar_kind(),
            Self::SidechainFilter(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Threshold(v) | Self::Makeup(v) => v.scalar_unit(),
            Self::Ratio(v) => v.scalar_unit(),
            Self::Attack(v) | Self::Release(v) => v.scalar_unit(),
            Self::Mix(v) => v.scalar_unit(),
            Self::SidechainEnabled(v) => v.scalar_unit(),
            Self::SidechainFilter(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Threshold(v) | Self::Makeup(v) => v.scalar_curve(),
            Self::Ratio(v) => v.scalar_curve(),
            Self::Attack(v) | Self::Release(v) => v.scalar_curve(),
            Self::Mix(v) => v.scalar_curve(),
            Self::SidechainEnabled(v) => v.scalar_curve(),
            Self::SidechainFilter(v) => v.scalar_curve(),
        }
    }
}

impl TransientShaperParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Attack(v) | Self::Sustain(v) => v.scalar_kind(),
            Self::Sensitivity(v) | Self::Mix(v) => v.scalar_kind(),
            Self::Window(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Attack(v) | Self::Sustain(v) => v.scalar_unit(),
            Self::Sensitivity(v) | Self::Mix(v) => v.scalar_unit(),
            Self::Window(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Attack(v) | Self::Sustain(v) => v.scalar_curve(),
            Self::Sensitivity(v) | Self::Mix(v) => v.scalar_curve(),
            Self::Window(v) => v.scalar_curve(),
        }
    }
}

impl EqParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::LowFreq(v) | Self::MidFreq(v) | Self::HighFreq(v) => v.scalar_kind(),
            Self::LowGain(v) | Self::MidGain(v) | Self::HighGain(v) => v.scalar_kind(),
            Self::MidQ(v) | Self::Mix(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::LowFreq(v) | Self::MidFreq(v) | Self::HighFreq(v) => v.scalar_unit(),
            Self::LowGain(v) | Self::MidGain(v) | Self::HighGain(v) => v.scalar_unit(),
            Self::MidQ(v) | Self::Mix(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::LowFreq(v) | Self::MidFreq(v) | Self::HighFreq(v) => v.scalar_curve(),
            Self::LowGain(v) | Self::MidGain(v) | Self::HighGain(v) => v.scalar_curve(),
            Self::MidQ(v) | Self::Mix(v) => v.scalar_curve(),
        }
    }
}

impl BbdDelayParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Time(v) => v.scalar_kind(),
            Self::Feedback(v)
            | Self::Tone(v)
            | Self::WowFlutter(v)
            | Self::ClockNoise(v)
            | Self::Mix(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Time(v) => v.scalar_unit(),
            Self::Feedback(v)
            | Self::Tone(v)
            | Self::WowFlutter(v)
            | Self::ClockNoise(v)
            | Self::Mix(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Time(v) => v.scalar_curve(),
            Self::Feedback(v)
            | Self::Tone(v)
            | Self::WowFlutter(v)
            | Self::ClockNoise(v)
            | Self::Mix(v) => v.scalar_curve(),
        }
    }
}

impl MidSideParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Width(v) | Self::Mix(v) => v.scalar_kind(),
            Self::MidGain(v) | Self::SideGain(v) => v.scalar_kind(),
            Self::Rotation(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Width(v) | Self::Mix(v) => v.scalar_unit(),
            Self::MidGain(v) | Self::SideGain(v) => v.scalar_unit(),
            Self::Rotation(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Width(v) | Self::Mix(v) => v.scalar_curve(),
            Self::MidGain(v) | Self::SideGain(v) => v.scalar_curve(),
            Self::Rotation(v) => v.scalar_curve(),
        }
    }
}

impl LimiterParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Ceiling(v) => v.scalar_kind(),
            Self::LookAhead(v) | Self::Release(v) => v.scalar_kind(),
            Self::Mix(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Ceiling(v) => v.scalar_unit(),
            Self::LookAhead(v) | Self::Release(v) => v.scalar_unit(),
            Self::Mix(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Ceiling(v) => v.scalar_curve(),
            Self::LookAhead(v) | Self::Release(v) => v.scalar_curve(),
            Self::Mix(v) => v.scalar_curve(),
        }
    }
}

impl EnsembleChorusParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Rate(v) => v.scalar_kind(),
            Self::Depth(v) | Self::BaseDelay(v) => v.scalar_kind(),
            Self::Mix(v) | Self::Tone(v) | Self::Noise(v) | Self::StereoWidth(v) => v.scalar_kind(),
            Self::Voices(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Rate(v) => v.scalar_unit(),
            Self::Depth(v) | Self::BaseDelay(v) => v.scalar_unit(),
            Self::Mix(v) | Self::Tone(v) | Self::Noise(v) | Self::StereoWidth(v) => v.scalar_unit(),
            Self::Voices(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Rate(v) => v.scalar_curve(),
            Self::Depth(v) | Self::BaseDelay(v) => v.scalar_curve(),
            Self::Mix(v) | Self::Tone(v) | Self::Noise(v) | Self::StereoWidth(v) => {
                v.scalar_curve()
            }
            Self::Voices(v) => v.scalar_curve(),
        }
    }
}

impl ShimmerReverbParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::RoomSize(v)
            | Self::Decay(v)
            | Self::Damping(v)
            | Self::ShimmerMix(v)
            | Self::Mix(v) => v.scalar_kind(),
            Self::PreDelay(v) => v.scalar_kind(),
            Self::PitchSemitones(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::RoomSize(v)
            | Self::Decay(v)
            | Self::Damping(v)
            | Self::ShimmerMix(v)
            | Self::Mix(v) => v.scalar_unit(),
            Self::PreDelay(v) => v.scalar_unit(),
            Self::PitchSemitones(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::RoomSize(v)
            | Self::Decay(v)
            | Self::Damping(v)
            | Self::ShimmerMix(v)
            | Self::Mix(v) => v.scalar_curve(),
            Self::PreDelay(v) => v.scalar_curve(),
            Self::PitchSemitones(v) => v.scalar_curve(),
        }
    }
}

impl GranularFxParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::BufferTime(v) => v.scalar_kind(),
            Self::GrainSize(v) => v.scalar_kind(),
            Self::Density(v)
            | Self::Position(v)
            | Self::PositionSpread(v)
            | Self::PitchSpread(v)
            | Self::PanSpread(v)
            | Self::Mix(v) => v.scalar_kind(),
            Self::Freeze(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::BufferTime(v) => v.scalar_unit(),
            Self::GrainSize(v) => v.scalar_unit(),
            Self::Density(v)
            | Self::Position(v)
            | Self::PositionSpread(v)
            | Self::PitchSpread(v)
            | Self::PanSpread(v)
            | Self::Mix(v) => v.scalar_unit(),
            Self::Freeze(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::BufferTime(v) => v.scalar_curve(),
            Self::GrainSize(v) => v.scalar_curve(),
            Self::Density(v)
            | Self::Position(v)
            | Self::PositionSpread(v)
            | Self::PitchSpread(v)
            | Self::PanSpread(v)
            | Self::Mix(v) => v.scalar_curve(),
            Self::Freeze(v) => v.scalar_curve(),
        }
    }
}

impl SpectralBlurParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::FftSize(v) => v.scalar_kind(),
            Self::BlurTime(v) | Self::BlurFreq(v) | Self::Mix(v) => v.scalar_kind(),
            Self::Freeze(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::FftSize(v) => v.scalar_unit(),
            Self::BlurTime(v) | Self::BlurFreq(v) | Self::Mix(v) => v.scalar_unit(),
            Self::Freeze(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::FftSize(v) => v.scalar_curve(),
            Self::BlurTime(v) | Self::BlurFreq(v) | Self::Mix(v) => v.scalar_curve(),
            Self::Freeze(v) => v.scalar_curve(),
        }
    }
}

impl ModalResonatorParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::BaseNote(v) => v.scalar_kind(),
            Self::Spread(v) | Self::Decay(v) | Self::Brightness(v) | Self::Mix(v) => {
                v.scalar_kind()
            }
            Self::Modes(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::BaseNote(v) => v.scalar_unit(),
            Self::Spread(v) | Self::Decay(v) | Self::Brightness(v) | Self::Mix(v) => {
                v.scalar_unit()
            }
            Self::Modes(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::BaseNote(v) => v.scalar_curve(),
            Self::Spread(v) | Self::Decay(v) | Self::Brightness(v) | Self::Mix(v) => {
                v.scalar_curve()
            }
            Self::Modes(v) => v.scalar_curve(),
        }
    }
}

impl ReverseGateReverbParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::WindowTime(v) | Self::GateTime(v) => v.scalar_kind(),
            Self::Mode(v) => v.scalar_kind(),
            Self::Trigger(v) => v.scalar_kind(),
            Self::Threshold(v) => v.scalar_kind(),
            Self::Mix(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::WindowTime(v) | Self::GateTime(v) => v.scalar_unit(),
            Self::Mode(v) => v.scalar_unit(),
            Self::Trigger(v) => v.scalar_unit(),
            Self::Threshold(v) => v.scalar_unit(),
            Self::Mix(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::WindowTime(v) | Self::GateTime(v) => v.scalar_curve(),
            Self::Mode(v) => v.scalar_curve(),
            Self::Trigger(v) => v.scalar_curve(),
            Self::Threshold(v) => v.scalar_curve(),
            Self::Mix(v) => v.scalar_curve(),
        }
    }
}

impl TiltEqParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Tilt(v) => v.scalar_kind(),
            Self::CenterFreq(v) => v.scalar_kind(),
            Self::Mix(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Tilt(v) => v.scalar_unit(),
            Self::CenterFreq(v) => v.scalar_unit(),
            Self::Mix(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Tilt(v) => v.scalar_curve(),
            Self::CenterFreq(v) => v.scalar_curve(),
            Self::Mix(v) => v.scalar_curve(),
        }
    }
}

impl UnivibeParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Rate(v) => v.scalar_kind(),
            Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Rate(v) => v.scalar_unit(),
            Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Rate(v) => v.scalar_curve(),
            Self::Depth(v) | Self::Feedback(v) | Self::Mix(v) => v.scalar_curve(),
        }
    }
}

impl CrossoverParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Frequency(v) => v.scalar_kind(),
            Self::LowGain(v) | Self::HighGain(v) | Self::Mix(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Frequency(v) => v.scalar_unit(),
            Self::LowGain(v) | Self::HighGain(v) | Self::Mix(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Frequency(v) => v.scalar_curve(),
            Self::LowGain(v) | Self::HighGain(v) | Self::Mix(v) => v.scalar_curve(),
        }
    }
}

impl VocoderParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Order(v) | Self::Mix(v) => v.scalar_kind(),
            Self::WindowSize(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Order(v) | Self::Mix(v) => v.scalar_unit(),
            Self::WindowSize(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Order(v) | Self::Mix(v) => v.scalar_curve(),
            Self::WindowSize(v) => v.scalar_curve(),
        }
    }
}

impl EnvelopeFollowerParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Attack(v) | Self::Release(v) => v.scalar_kind(),
            Self::Sensitivity(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Attack(v) | Self::Release(v) => v.scalar_unit(),
            Self::Sensitivity(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Attack(v) | Self::Release(v) => v.scalar_curve(),
            Self::Sensitivity(v) => v.scalar_curve(),
        }
    }
}

impl EnvelopeParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Attack(v) | Self::Decay(v) | Self::Release(v) => v.scalar_kind(),
            Self::Sustain(v) | Self::VelocitySensitivity(v) => v.scalar_kind(),
            Self::AttackCurve(v) | Self::DecayCurve(v) | Self::ReleaseCurve(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Attack(v) | Self::Decay(v) | Self::Release(v) => v.scalar_unit(),
            Self::Sustain(v) | Self::VelocitySensitivity(v) => v.scalar_unit(),
            Self::AttackCurve(v) | Self::DecayCurve(v) | Self::ReleaseCurve(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Attack(v) | Self::Decay(v) | Self::Release(v) => v.scalar_curve(),
            Self::Sustain(v) | Self::VelocitySensitivity(v) => v.scalar_curve(),
            Self::AttackCurve(v) | Self::DecayCurve(v) | Self::ReleaseCurve(v) => v.scalar_curve(),
        }
    }
}

impl FilterParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Mode(v) => v.scalar_kind(),
            Self::Cutoff(v) => v.scalar_kind(),
            Self::Resonance(v) | Self::KeyTracking(v) | Self::Morph(v) => v.scalar_kind(),
            Self::Drive(v) => v.scalar_kind(),
            Self::EnvAmount(v) | Self::CutoffMod(v) => v.scalar_kind(),
            Self::Model(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Mode(v) => v.scalar_unit(),
            Self::Cutoff(v) => v.scalar_unit(),
            Self::Resonance(v) | Self::KeyTracking(v) | Self::Morph(v) => v.scalar_unit(),
            Self::Drive(v) => v.scalar_unit(),
            Self::EnvAmount(v) | Self::CutoffMod(v) => v.scalar_unit(),
            Self::Model(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Mode(v) => v.scalar_curve(),
            Self::Cutoff(v) => v.scalar_curve(),
            Self::Resonance(v) | Self::KeyTracking(v) | Self::Morph(v) => v.scalar_curve(),
            Self::Drive(v) => v.scalar_curve(),
            Self::EnvAmount(v) | Self::CutoffMod(v) => v.scalar_curve(),
            Self::Model(v) => v.scalar_curve(),
        }
    }
}

impl FofParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Vowel(v)
            | Self::FormantShift(v)
            | Self::Skirt(v)
            | Self::Bandwidth(v)
            | Self::Breathiness(v)
            | Self::UnisonVoices(v)
            | Self::UnisonSpread(v)
            | Self::Level(v) => v.scalar_kind(),
            Self::VibratoRate(v) => v.scalar_kind(),
            Self::VibratoDepth(v) | Self::UnisonDetune(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Vowel(v)
            | Self::FormantShift(v)
            | Self::Skirt(v)
            | Self::Bandwidth(v)
            | Self::Breathiness(v)
            | Self::UnisonVoices(v)
            | Self::UnisonSpread(v)
            | Self::Level(v) => v.scalar_unit(),
            Self::VibratoRate(v) => v.scalar_unit(),
            Self::VibratoDepth(v) | Self::UnisonDetune(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Vowel(v)
            | Self::FormantShift(v)
            | Self::Skirt(v)
            | Self::Bandwidth(v)
            | Self::Breathiness(v)
            | Self::UnisonVoices(v)
            | Self::UnisonSpread(v)
            | Self::Level(v) => v.scalar_curve(),
            Self::VibratoRate(v) => v.scalar_curve(),
            Self::VibratoDepth(v) | Self::UnisonDetune(v) => v.scalar_curve(),
        }
    }
}

impl FooglersParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Tap1(v)
            | Self::Tap2(v)
            | Self::Feedback(v)
            | Self::Damping(v)
            | Self::Level(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Tap1(v)
            | Self::Tap2(v)
            | Self::Feedback(v)
            | Self::Damping(v)
            | Self::Level(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Tap1(v)
            | Self::Tap2(v)
            | Self::Feedback(v)
            | Self::Damping(v)
            | Self::Level(v) => v.scalar_curve(),
        }
    }
}

impl FormantFilterParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Vowel(v) | Self::Resonance(v) | Self::Mix(v) => v.scalar_kind(),
            Self::Cutoff(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Vowel(v) | Self::Resonance(v) | Self::Mix(v) => v.scalar_unit(),
            Self::Cutoff(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Vowel(v) | Self::Resonance(v) | Self::Mix(v) => v.scalar_curve(),
            Self::Cutoff(v) => v.scalar_curve(),
        }
    }
}

impl FractalOscParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Roughness(v)
            | Self::FractalSpacing(v)
            | Self::Dispersion(v)
            | Self::Spread(v)
            | Self::Level(v) => v.scalar_kind(),
            Self::GlideTime(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Roughness(v)
            | Self::FractalSpacing(v)
            | Self::Dispersion(v)
            | Self::Spread(v)
            | Self::Level(v) => v.scalar_unit(),
            Self::GlideTime(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Roughness(v)
            | Self::FractalSpacing(v)
            | Self::Dispersion(v)
            | Self::Spread(v)
            | Self::Level(v) => v.scalar_curve(),
            Self::GlideTime(v) => v.scalar_curve(),
        }
    }
}

impl FrequencyShifterParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Shift(v) => v.scalar_kind(),
            Self::Mix(v) | Self::Mode(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Shift(v) => v.scalar_unit(),
            Self::Mix(v) | Self::Mode(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Shift(v) => v.scalar_curve(),
            Self::Mix(v) | Self::Mode(v) => v.scalar_curve(),
        }
    }
}

impl EuclideanParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Steps(v) | Self::Pulses(v) | Self::Rotation(v) => v.scalar_kind(),
            Self::Swing(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Steps(v) | Self::Pulses(v) | Self::Rotation(v) => v.scalar_unit(),
            Self::Swing(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Steps(v) | Self::Pulses(v) | Self::Rotation(v) => v.scalar_curve(),
            Self::Swing(v) => v.scalar_curve(),
        }
    }
}

impl TuringMachineParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::MutationRate(v) | Self::Range(v) => v.scalar_kind(),
            Self::Scale(v) => v.scalar_kind(),
            Self::Length(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::MutationRate(v) | Self::Range(v) => v.scalar_unit(),
            Self::Scale(v) => v.scalar_unit(),
            Self::Length(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::MutationRate(v) | Self::Range(v) => v.scalar_curve(),
            Self::Scale(v) => v.scalar_curve(),
            Self::Length(v) => v.scalar_curve(),
        }
    }
}

impl RandomGatesParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Density(v) | Self::BurstProbability(v) | Self::GateLength(v) => v.scalar_kind(),
            Self::Seed(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Density(v) | Self::BurstProbability(v) | Self::GateLength(v) => v.scalar_unit(),
            Self::Seed(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Density(v) | Self::BurstProbability(v) | Self::GateLength(v) => v.scalar_curve(),
            Self::Seed(v) => v.scalar_curve(),
        }
    }
}

impl GranularParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::GrainSize(v) => v.scalar_kind(),
            Self::Density(v)
            | Self::Position(v)
            | Self::PositionSpread(v)
            | Self::PitchSpread(v)
            | Self::PanSpread(v) => v.scalar_kind(),
            Self::Freeze(v) => v.scalar_kind(),
            Self::Window(v) => v.scalar_kind(),
            Self::Source(v) => v.scalar_kind(),
            Self::Level(v) => v.scalar_kind(),
            Self::GlideTime(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::GrainSize(v) => v.scalar_unit(),
            Self::Density(v)
            | Self::Position(v)
            | Self::PositionSpread(v)
            | Self::PitchSpread(v)
            | Self::PanSpread(v) => v.scalar_unit(),
            Self::Freeze(v) => v.scalar_unit(),
            Self::Window(v) => v.scalar_unit(),
            Self::Source(v) => v.scalar_unit(),
            Self::Level(v) => v.scalar_unit(),
            Self::GlideTime(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::GrainSize(v) => v.scalar_curve(),
            Self::Density(v)
            | Self::Position(v)
            | Self::PositionSpread(v)
            | Self::PitchSpread(v)
            | Self::PanSpread(v) => v.scalar_curve(),
            Self::Freeze(v) => v.scalar_curve(),
            Self::Window(v) => v.scalar_curve(),
            Self::Source(v) => v.scalar_curve(),
            Self::Level(v) => v.scalar_curve(),
            Self::GlideTime(v) => v.scalar_curve(),
        }
    }
}

impl KineticParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Duration(v) => v.scalar_kind(),
            Self::CurveType(v) => v.scalar_kind(),
            Self::Overshoot(v) => v.scalar_kind(),
            Self::Bipolar(v) | Self::Retrigger(v) => v.scalar_kind(),
            Self::LoopMode(v) => v.scalar_kind(),
            Self::OutputVel(v) | Self::OutputAcc(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Duration(v) => v.scalar_unit(),
            Self::CurveType(v) => v.scalar_unit(),
            Self::Overshoot(v) => v.scalar_unit(),
            Self::Bipolar(v) | Self::Retrigger(v) => v.scalar_unit(),
            Self::LoopMode(v) => v.scalar_unit(),
            Self::OutputVel(v) | Self::OutputAcc(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Duration(v) => v.scalar_curve(),
            Self::CurveType(v) => v.scalar_curve(),
            Self::Overshoot(v) => v.scalar_curve(),
            Self::Bipolar(v) | Self::Retrigger(v) => v.scalar_curve(),
            Self::LoopMode(v) => v.scalar_curve(),
            Self::OutputVel(v) | Self::OutputAcc(v) => v.scalar_curve(),
        }
    }
}

impl LaSynthParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::AttackType(v) | Self::AttackLevel(v) | Self::Brightness(v) => v.scalar_kind(),
            Self::AttackTime(v) | Self::CrossfadeTime(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::AttackType(v) | Self::AttackLevel(v) | Self::Brightness(v) => v.scalar_unit(),
            Self::AttackTime(v) | Self::CrossfadeTime(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::AttackType(v) | Self::AttackLevel(v) | Self::Brightness(v) => v.scalar_curve(),
            Self::AttackTime(v) | Self::CrossfadeTime(v) => v.scalar_curve(),
        }
    }
}

impl LfoParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Waveform(v) => v.scalar_kind(),
            Self::Rate(v) => v.scalar_kind(),
            Self::Depth(v) => v.scalar_kind(),
            Self::Phase(v) => v.scalar_kind(),
            Self::TempoSync(v) | Self::Retrigger(v) => v.scalar_kind(),
            Self::SyncDivision(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Waveform(v) => v.scalar_unit(),
            Self::Rate(v) => v.scalar_unit(),
            Self::Depth(v) => v.scalar_unit(),
            Self::Phase(v) => v.scalar_unit(),
            Self::TempoSync(v) | Self::Retrigger(v) => v.scalar_unit(),
            Self::SyncDivision(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Waveform(v) => v.scalar_curve(),
            Self::Rate(v) => v.scalar_curve(),
            Self::Depth(v) => v.scalar_curve(),
            Self::Phase(v) => v.scalar_curve(),
            Self::TempoSync(v) | Self::Retrigger(v) => v.scalar_curve(),
            Self::SyncDivision(v) => v.scalar_curve(),
        }
    }
}

impl ModMatrixParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::GridSize(v) => v.scalar_kind(),
            Self::SlotSource(_, v) => v.scalar_kind(),
            Self::SlotDestination(_, v) => v.scalar_kind(),
            Self::SlotAmount(_, v) => v.scalar_kind(),
            Self::SlotEnabled(_, v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::GridSize(v) => v.scalar_unit(),
            Self::SlotSource(_, v) => v.scalar_unit(),
            Self::SlotDestination(_, v) => v.scalar_unit(),
            Self::SlotAmount(_, v) => v.scalar_unit(),
            Self::SlotEnabled(_, v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::GridSize(v) => v.scalar_curve(),
            Self::SlotSource(_, v) => v.scalar_curve(),
            Self::SlotDestination(_, v) => v.scalar_curve(),
            Self::SlotAmount(_, v) => v.scalar_curve(),
            Self::SlotEnabled(_, v) => v.scalar_curve(),
        }
    }
}

impl AmplifierParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Level(v) => v.scalar_kind(),
            Self::Pan(v) => v.scalar_kind(),
            Self::CvBipolar(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Level(v) => v.scalar_unit(),
            Self::Pan(v) => v.scalar_unit(),
            Self::CvBipolar(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Level(v) => v.scalar_curve(),
            Self::Pan(v) => v.scalar_curve(),
            Self::CvBipolar(v) => v.scalar_curve(),
        }
    }
}

impl MixerParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Input1(v)
            | Self::Input2(v)
            | Self::Input3(v)
            | Self::Input4(v)
            | Self::Input5(v)
            | Self::Input6(v)
            | Self::Input7(v)
            | Self::Input8(v)
            | Self::Master(v) => v.scalar_kind(),
            Self::Mute(v) | Self::Limit(v) | Self::Dither(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Input1(v)
            | Self::Input2(v)
            | Self::Input3(v)
            | Self::Input4(v)
            | Self::Input5(v)
            | Self::Input6(v)
            | Self::Input7(v)
            | Self::Input8(v)
            | Self::Master(v) => v.scalar_unit(),
            Self::Mute(v) | Self::Limit(v) | Self::Dither(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Input1(v)
            | Self::Input2(v)
            | Self::Input3(v)
            | Self::Input4(v)
            | Self::Input5(v)
            | Self::Input6(v)
            | Self::Input7(v)
            | Self::Input8(v)
            | Self::Master(v) => v.scalar_curve(),
            Self::Mute(v) | Self::Limit(v) | Self::Dither(v) => v.scalar_curve(),
        }
    }
}

impl OscilloscopeParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Time(v) => v.scalar_kind(),
            Self::Gain(v) => v.scalar_kind(),
            Self::Trigger(v) => v.scalar_kind(),
            Self::Frozen(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Time(v) => v.scalar_unit(),
            Self::Gain(v) => v.scalar_unit(),
            Self::Trigger(v) => v.scalar_unit(),
            Self::Frozen(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Time(v) => v.scalar_curve(),
            Self::Gain(v) => v.scalar_curve(),
            Self::Trigger(v) => v.scalar_curve(),
            Self::Frozen(v) => v.scalar_curve(),
        }
    }
}

impl LevelMeterParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::PeakHold(v) => v.scalar_kind(),
            Self::DecayRate(v) => v.scalar_kind(),
            Self::ShowRms(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::PeakHold(v) => v.scalar_unit(),
            Self::DecayRate(v) => v.scalar_unit(),
            Self::ShowRms(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::PeakHold(v) => v.scalar_curve(),
            Self::DecayRate(v) => v.scalar_curve(),
            Self::ShowRms(v) => v.scalar_curve(),
        }
    }
}

impl MsegParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::SegmentCount(v)
            | Self::SustainSegment(v)
            | Self::LoopStart(v)
            | Self::LoopEnd(v) => v.scalar_kind(),
            Self::LoopEnabled(v) => v.scalar_kind(),
            Self::TimeScale(v) => v.scalar_kind(),
            Self::SegmentTime(_, v) => v.scalar_kind(),
            Self::SegmentLevel(_, v) => v.scalar_kind(),
            Self::SegmentCurve(_, v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::SegmentCount(v)
            | Self::SustainSegment(v)
            | Self::LoopStart(v)
            | Self::LoopEnd(v) => v.scalar_unit(),
            Self::LoopEnabled(v) => v.scalar_unit(),
            Self::TimeScale(v) => v.scalar_unit(),
            Self::SegmentTime(_, v) => v.scalar_unit(),
            Self::SegmentLevel(_, v) => v.scalar_unit(),
            Self::SegmentCurve(_, v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::SegmentCount(v)
            | Self::SustainSegment(v)
            | Self::LoopStart(v)
            | Self::LoopEnd(v) => v.scalar_curve(),
            Self::LoopEnabled(v) => v.scalar_curve(),
            Self::TimeScale(v) => v.scalar_curve(),
            Self::SegmentTime(_, v) => v.scalar_curve(),
            Self::SegmentLevel(_, v) => v.scalar_curve(),
            Self::SegmentCurve(_, v) => v.scalar_curve(),
        }
    }
}

impl NoiseParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Type(v) => v.scalar_kind(),
            Self::Level(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Type(v) => v.scalar_unit(),
            Self::Level(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Type(v) => v.scalar_curve(),
            Self::Level(v) => v.scalar_curve(),
        }
    }
}

impl OscillatorParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Waveform(v) => v.scalar_kind(),
            Self::Frequency(v) => v.scalar_kind(),
            Self::Detune(v) | Self::UnisonDetune(v) => v.scalar_kind(),
            Self::Octave(v) => v.scalar_kind(),
            Self::PulseWidth(v) => v.scalar_kind(),
            Self::Level(v) => v.scalar_kind(),
            Self::Phase(v) => v.scalar_kind(),
            Self::FmMode(v) => v.scalar_kind(),
            Self::FmAmount(v) => v.scalar_kind(),
            Self::UnisonVoices(v) => v.scalar_kind(),
            Self::UnisonSpread(v) | Self::UnisonPhaseRandom(v) | Self::CrossModAmount(v) => {
                v.scalar_kind()
            }
            Self::AntiAlias(v) => v.scalar_kind(),
            Self::GlideTime(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Waveform(v) => v.scalar_unit(),
            Self::Frequency(v) => v.scalar_unit(),
            Self::Detune(v) | Self::UnisonDetune(v) => v.scalar_unit(),
            Self::Octave(v) => v.scalar_unit(),
            Self::PulseWidth(v) => v.scalar_unit(),
            Self::Level(v) => v.scalar_unit(),
            Self::Phase(v) => v.scalar_unit(),
            Self::FmMode(v) => v.scalar_unit(),
            Self::FmAmount(v) => v.scalar_unit(),
            Self::UnisonVoices(v) => v.scalar_unit(),
            Self::UnisonSpread(v) | Self::UnisonPhaseRandom(v) | Self::CrossModAmount(v) => {
                v.scalar_unit()
            }
            Self::AntiAlias(v) => v.scalar_unit(),
            Self::GlideTime(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Waveform(v) => v.scalar_curve(),
            Self::Frequency(v) => v.scalar_curve(),
            Self::Detune(v) | Self::UnisonDetune(v) => v.scalar_curve(),
            Self::Octave(v) => v.scalar_curve(),
            Self::PulseWidth(v) => v.scalar_curve(),
            Self::Level(v) => v.scalar_curve(),
            Self::Phase(v) => v.scalar_curve(),
            Self::FmMode(v) => v.scalar_curve(),
            Self::FmAmount(v) => v.scalar_curve(),
            Self::UnisonVoices(v) => v.scalar_curve(),
            Self::UnisonSpread(v) | Self::UnisonPhaseRandom(v) | Self::CrossModAmount(v) => {
                v.scalar_curve()
            }
            Self::AntiAlias(v) => v.scalar_curve(),
            Self::GlideTime(v) => v.scalar_curve(),
        }
    }
}

impl MathOscillatorParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Algorithm(v) => v.scalar_kind(),
            Self::Frequency(v) => v.scalar_kind(),
            Self::ParamA(v) | Self::ParamB(v) | Self::ParamC(v) => v.scalar_kind(),
            Self::Level(v) => v.scalar_kind(),
            Self::GlideTime(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Algorithm(v) => v.scalar_unit(),
            Self::Frequency(v) => v.scalar_unit(),
            Self::ParamA(v) | Self::ParamB(v) | Self::ParamC(v) => v.scalar_unit(),
            Self::Level(v) => v.scalar_unit(),
            Self::GlideTime(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Algorithm(v) => v.scalar_curve(),
            Self::Frequency(v) => v.scalar_curve(),
            Self::ParamA(v) | Self::ParamB(v) | Self::ParamC(v) => v.scalar_curve(),
            Self::Level(v) => v.scalar_curve(),
            Self::GlideTime(v) => v.scalar_curve(),
        }
    }
}

impl PadSynthParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Bandwidth(v) | Self::Tilt(v) | Self::Detune(v) | Self::Level(v) => {
                v.scalar_kind()
            }
            Self::BaseFreq(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Bandwidth(v) | Self::Tilt(v) | Self::Detune(v) | Self::Level(v) => {
                v.scalar_unit()
            }
            Self::BaseFreq(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Bandwidth(v) | Self::Tilt(v) | Self::Detune(v) | Self::Level(v) => {
                v.scalar_curve()
            }
            Self::BaseFreq(v) => v.scalar_curve(),
        }
    }
}

impl PhaseVocoderParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::PitchShift(v) => v.scalar_kind(),
            Self::Freeze(v) => v.scalar_kind(),
            Self::FftSize(v) => v.scalar_kind(),
            Self::Mix(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::PitchShift(v) => v.scalar_unit(),
            Self::Freeze(v) => v.scalar_unit(),
            Self::FftSize(v) => v.scalar_unit(),
            Self::Mix(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::PitchShift(v) => v.scalar_curve(),
            Self::Freeze(v) => v.scalar_curve(),
            Self::FftSize(v) => v.scalar_curve(),
            Self::Mix(v) => v.scalar_curve(),
        }
    }
}

impl KeyboardPannerParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Spread(v) => v.scalar_kind(),
            Self::CenterNote(v) => v.scalar_kind(),
            Self::Curve(v) => v.scalar_kind(),
            Self::Invert(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Spread(v) => v.scalar_unit(),
            Self::CenterNote(v) => v.scalar_unit(),
            Self::Curve(v) => v.scalar_unit(),
            Self::Invert(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Spread(v) => v.scalar_curve(),
            Self::CenterNote(v) => v.scalar_curve(),
            Self::Curve(v) => v.scalar_curve(),
            Self::Invert(v) => v.scalar_curve(),
        }
    }
}

impl SpatialPannerParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::X(v) | Self::Y(v) | Self::Z(v) => v.scalar_kind(),
            Self::Diffusion(v)
            | Self::ErLevel(v)
            | Self::DirectLevel(v)
            | Self::Absorption(v)
            | Self::AirAbsorption(v)
            | Self::Distance(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::X(v) | Self::Y(v) | Self::Z(v) => v.scalar_unit(),
            Self::Diffusion(v)
            | Self::ErLevel(v)
            | Self::DirectLevel(v)
            | Self::Absorption(v)
            | Self::AirAbsorption(v)
            | Self::Distance(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type.
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::X(v) | Self::Y(v) | Self::Z(v) => v.scalar_curve(),
            Self::Diffusion(v)
            | Self::ErLevel(v)
            | Self::DirectLevel(v)
            | Self::Absorption(v)
            | Self::AirAbsorption(v)
            | Self::Distance(v) => v.scalar_curve(),
        }
    }
}

impl BodyResonanceParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Frequency(v) => v.scalar_kind(),
            Self::Resonance(v) | Self::Size(v) | Self::Brightness(v) | Self::Mix(v) => {
                v.scalar_kind()
            }
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Frequency(v) => v.scalar_unit(),
            Self::Resonance(v) | Self::Size(v) | Self::Brightness(v) | Self::Mix(v) => {
                v.scalar_unit()
            }
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Frequency(v) => v.scalar_curve(),
            Self::Resonance(v) | Self::Size(v) | Self::Brightness(v) | Self::Mix(v) => {
                v.scalar_curve()
            }
        }
    }
}

impl MechanicalNoiseParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::NoiseType(v) => v.scalar_kind(),
            Self::Duration(v) => v.scalar_kind(),
            Self::Cutoff(v) => v.scalar_kind(),
            Self::VelocitySens(v) => v.scalar_kind(),
            Self::Level(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::NoiseType(v) => v.scalar_unit(),
            Self::Duration(v) => v.scalar_unit(),
            Self::Cutoff(v) => v.scalar_unit(),
            Self::VelocitySens(v) => v.scalar_unit(),
            Self::Level(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::NoiseType(v) => v.scalar_curve(),
            Self::Duration(v) => v.scalar_curve(),
            Self::Cutoff(v) => v.scalar_curve(),
            Self::VelocitySens(v) => v.scalar_curve(),
            Self::Level(v) => v.scalar_curve(),
        }
    }
}

impl PitchTrackerParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Sensitivity(v) | Self::Smoothing(v) => v.scalar_kind(),
            Self::MinFreq(v) | Self::MaxFreq(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Sensitivity(v) | Self::Smoothing(v) => v.scalar_unit(),
            Self::MinFreq(v) | Self::MaxFreq(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Sensitivity(v) | Self::Smoothing(v) => v.scalar_curve(),
            Self::MinFreq(v) | Self::MaxFreq(v) => v.scalar_curve(),
        }
    }
}

impl RingModParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::CarrierFreq(v) => v.scalar_kind(),
            Self::CarrierWaveform(v) => v.scalar_kind(),
            Self::Mix(v) | Self::FreqRatio(v) | Self::TrackKeyboard(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::CarrierFreq(v) => v.scalar_unit(),
            Self::CarrierWaveform(v) => v.scalar_unit(),
            Self::Mix(v) | Self::FreqRatio(v) | Self::TrackKeyboard(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::CarrierFreq(v) => v.scalar_curve(),
            Self::CarrierWaveform(v) => v.scalar_curve(),
            Self::Mix(v) | Self::FreqRatio(v) | Self::TrackKeyboard(v) => v.scalar_curve(),
        }
    }
}

impl SamplerParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::SampleSelect(v) => v.scalar_kind(),
            Self::PitchTracking(v) => v.scalar_kind(),
            Self::Level(v) => v.scalar_kind(),
            Self::PlayMode(v) => v.scalar_kind(),
            Self::Direction(v) => v.scalar_kind(),
            Self::VelocitySensitivity(v) | Self::StartOffset(v) => v.scalar_kind(),
            Self::FineTune(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::SampleSelect(v) => v.scalar_unit(),
            Self::PitchTracking(v) => v.scalar_unit(),
            Self::Level(v) => v.scalar_unit(),
            Self::PlayMode(v) => v.scalar_unit(),
            Self::Direction(v) => v.scalar_unit(),
            Self::VelocitySensitivity(v) | Self::StartOffset(v) => v.scalar_unit(),
            Self::FineTune(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::SampleSelect(v) => v.scalar_curve(),
            Self::PitchTracking(v) => v.scalar_curve(),
            Self::Level(v) => v.scalar_curve(),
            Self::PlayMode(v) => v.scalar_curve(),
            Self::Direction(v) => v.scalar_curve(),
            Self::VelocitySensitivity(v) | Self::StartOffset(v) => v.scalar_curve(),
            Self::FineTune(v) => v.scalar_curve(),
        }
    }
}

impl SidOscillatorParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Triangle(v)
            | Self::Sawtooth(v)
            | Self::Pulse(v)
            | Self::Noise(v)
            | Self::TrackVoicePitch(v)
            | Self::Test(v)
            | Self::RingMod(v)
            | Self::HardSync(v)
            | Self::DcBlock(v)
            | Self::SeqLoop(v) => v.scalar_kind(),
            Self::NoiseSeed(v) => v.scalar_kind(),
            Self::FreqReg(v) | Self::PulseWidthReg(v) | Self::SeqStepFreq(_, v) => v.scalar_kind(),
            Self::Model(v) => v.scalar_kind(),
            Self::Clock(v) => v.scalar_kind(),
            Self::Quality(v) => v.scalar_kind(),
            Self::Level(v) => v.scalar_kind(),
            Self::SeqLength(v) | Self::SeqRate(v) | Self::SeqStep(_, v) => v.scalar_kind(),
            Self::SeqFreqMask(v) => v.scalar_kind(),
            Self::GlideTime(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Triangle(v)
            | Self::Sawtooth(v)
            | Self::Pulse(v)
            | Self::Noise(v)
            | Self::TrackVoicePitch(v)
            | Self::Test(v)
            | Self::RingMod(v)
            | Self::HardSync(v)
            | Self::DcBlock(v)
            | Self::SeqLoop(v) => v.scalar_unit(),
            Self::NoiseSeed(v) => v.scalar_unit(),
            Self::FreqReg(v) | Self::PulseWidthReg(v) | Self::SeqStepFreq(_, v) => v.scalar_unit(),
            Self::Model(v) => v.scalar_unit(),
            Self::Clock(v) => v.scalar_unit(),
            Self::Quality(v) => v.scalar_unit(),
            Self::Level(v) => v.scalar_unit(),
            Self::SeqLength(v) | Self::SeqRate(v) | Self::SeqStep(_, v) => v.scalar_unit(),
            Self::SeqFreqMask(v) => v.scalar_unit(),
            Self::GlideTime(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Triangle(v)
            | Self::Sawtooth(v)
            | Self::Pulse(v)
            | Self::Noise(v)
            | Self::TrackVoicePitch(v)
            | Self::Test(v)
            | Self::RingMod(v)
            | Self::HardSync(v)
            | Self::DcBlock(v)
            | Self::SeqLoop(v) => v.scalar_curve(),
            Self::NoiseSeed(v) => v.scalar_curve(),
            Self::FreqReg(v) | Self::PulseWidthReg(v) | Self::SeqStepFreq(_, v) => v.scalar_curve(),
            Self::Model(v) => v.scalar_curve(),
            Self::Clock(v) => v.scalar_curve(),
            Self::Quality(v) => v.scalar_curve(),
            Self::Level(v) => v.scalar_curve(),
            Self::SeqLength(v) | Self::SeqRate(v) | Self::SeqStep(_, v) => v.scalar_curve(),
            Self::SeqFreqMask(v) => v.scalar_curve(),
            Self::GlideTime(v) => v.scalar_curve(),
        }
    }
}

impl SignalMonitorParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Time(v) => v.scalar_kind(),
            Self::Gain(v) => v.scalar_kind(),
            Self::Trigger(v) => v.scalar_kind(),
            Self::Frozen(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Time(v) => v.scalar_unit(),
            Self::Gain(v) => v.scalar_unit(),
            Self::Trigger(v) => v.scalar_unit(),
            Self::Frozen(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Time(v) => v.scalar_curve(),
            Self::Gain(v) => v.scalar_curve(),
            Self::Trigger(v) => v.scalar_curve(),
            Self::Frozen(v) => v.scalar_curve(),
        }
    }
}

impl SpectrumAnalyzerParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Gain(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Gain(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Gain(v) => v.scalar_curve(),
        }
    }
}

impl SubOscParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Waveform(v) => v.scalar_kind(),
            Self::Octave(v) => v.scalar_kind(),
            Self::Level(v) => v.scalar_kind(),
            Self::GlideTime(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Waveform(v) => v.scalar_unit(),
            Self::Octave(v) => v.scalar_unit(),
            Self::Level(v) => v.scalar_unit(),
            Self::GlideTime(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Waveform(v) => v.scalar_curve(),
            Self::Octave(v) => v.scalar_curve(),
            Self::Level(v) => v.scalar_curve(),
            Self::GlideTime(v) => v.scalar_curve(),
        }
    }
}

impl VectorMixerParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::X(v) | Self::Y(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::X(v) | Self::Y(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::X(v) | Self::Y(v) => v.scalar_curve(),
        }
    }
}

impl VocalTractParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Tongue(v)
            | Self::Constriction(v)
            | Self::Lips(v)
            | Self::Length(v)
            | Self::Nasality(v)
            | Self::Breathiness(v)
            | Self::Level(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Tongue(v)
            | Self::Constriction(v)
            | Self::Lips(v)
            | Self::Length(v)
            | Self::Nasality(v)
            | Self::Breathiness(v)
            | Self::Level(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Tongue(v)
            | Self::Constriction(v)
            | Self::Lips(v)
            | Self::Length(v)
            | Self::Nasality(v)
            | Self::Breathiness(v)
            | Self::Level(v) => v.scalar_curve(),
        }
    }
}

impl VoiceSynthParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Vowel(v)
            | Self::FormantShift(v)
            | Self::Breathiness(v)
            | Self::OpenQuotient(v)
            | Self::Tilt(v)
            | Self::UnisonVoices(v)
            | Self::UnisonSpread(v)
            | Self::Level(v) => v.scalar_kind(),
            Self::VibratoRate(v) => v.scalar_kind(),
            Self::VibratoDepth(v) | Self::UnisonDetune(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Vowel(v)
            | Self::FormantShift(v)
            | Self::Breathiness(v)
            | Self::OpenQuotient(v)
            | Self::Tilt(v)
            | Self::UnisonVoices(v)
            | Self::UnisonSpread(v)
            | Self::Level(v) => v.scalar_unit(),
            Self::VibratoRate(v) => v.scalar_unit(),
            Self::VibratoDepth(v) | Self::UnisonDetune(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Vowel(v)
            | Self::FormantShift(v)
            | Self::Breathiness(v)
            | Self::OpenQuotient(v)
            | Self::Tilt(v)
            | Self::UnisonVoices(v)
            | Self::UnisonSpread(v)
            | Self::Level(v) => v.scalar_curve(),
            Self::VibratoRate(v) => v.scalar_curve(),
            Self::VibratoDepth(v) | Self::UnisonDetune(v) => v.scalar_curve(),
        }
    }
}

impl WaveshaperParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Curve(v) => v.scalar_kind(),
            Self::Drive(v) | Self::Mix(v) => v.scalar_kind(),
            Self::Bias(v) | Self::Symmetry(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Curve(v) => v.scalar_unit(),
            Self::Drive(v) | Self::Mix(v) => v.scalar_unit(),
            Self::Bias(v) | Self::Symmetry(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Curve(v) => v.scalar_curve(),
            Self::Drive(v) | Self::Mix(v) => v.scalar_curve(),
            Self::Bias(v) | Self::Symmetry(v) => v.scalar_curve(),
        }
    }
}

impl WavetableParam {
    /// Value-kind of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn kind(&self) -> ParamKind {
        match self {
            Self::Table(v) => v.scalar_kind(),
            Self::Position(v) => v.scalar_kind(),
            Self::Detune(v) => v.scalar_kind(),
            Self::Octave(v) => v.scalar_kind(),
            Self::Level(v) => v.scalar_kind(),
            Self::GlideTime(v) => v.scalar_kind(),
        }
    }

    /// Display unit of this parameter, dispatched on the bound value.
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
        match self {
            Self::Table(v) => v.scalar_unit(),
            Self::Position(v) => v.scalar_unit(),
            Self::Detune(v) => v.scalar_unit(),
            Self::Octave(v) => v.scalar_unit(),
            Self::Level(v) => v.scalar_unit(),
            Self::GlideTime(v) => v.scalar_unit(),
        }
    }

    /// Suggested response curve for this parameter's value type
    /// (advisory; not auto-applied — see plan Phase 2b / §14.6).
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
        match self {
            Self::Table(v) => v.scalar_curve(),
            Self::Position(v) => v.scalar_curve(),
            Self::Detune(v) => v.scalar_curve(),
            Self::Octave(v) => v.scalar_curve(),
            Self::Level(v) => v.scalar_curve(),
            Self::GlideTime(v) => v.scalar_curve(),
        }
    }
}

impl Param {
    /// Value-kind classifier for this parameter (delegates to the module enum).
    #[must_use]
    pub fn kind(&self) -> ParamKind {
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
    #[must_use]
    pub fn unit(&self) -> ParameterUnit {
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
    #[must_use]
    pub fn default_curve(&self) -> ResponseCurve {
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
