//! [`ScalarParam`] impls for every value type a [`Param`](crate::params::Param)
//! variant can carry — the type-derived `ParamKind` / unit / curve foundation
//! (Phase 1 of the parameter type-system plan).
//!
//! The authoritative `value type → kind` mapping is `docs/param-kinds.md`. Keep
//! these impls in sync with it; the per-enum `kind()` methods dispatch through
//! `ScalarParam::scalar_kind` on bound values, so the kind follows the field type.

use crate::module_traits::{ParamKind, ParameterUnit, ResponseCurve, ScalarParam};
use crate::params::{
    AntiAliasMode, ChaoticSystem, DelayMode, DestAddr, DistortionMode, EasingCurve, FftSizeOption,
    FilterMode, FilterModel, FmMode, GrainSource, GrainWindow, ImpulseResponse, KineticLoopMode,
    LfoWaveform, MathAlgo, MechanicalNoiseType, ModMatrixGridSize, NoiseType, PlayDirection,
    ReverseGateMode, ReverseGateTrigger, SampleId, SamplerPlayMode, SidClock, SidModel,
    SidNoiseSeed, SidQuality, SrcAddr, SubOscOctave, SubOscWaveform, TuringScale, Waveform,
    WaveshaperCurve, WavetableSelect,
};
use crate::types::{
    BeatDivision, BipolarValue, BitDepth, Cents, DecayTrim, Decibels, Gain, Hertz, MidiNote,
    Milliseconds, NormalizedValue, Octaves, Phase, Polarity, PulseWidth, Ratio, Seconds, Semitones,
    StepCount, TimeScale, VoiceCount,
};

/// Implement [`ScalarParam`] for `$t` with the given kind / unit / curve.
macro_rules! scalar {
    ($t:ty => $k:ident, $u:ident, $c:ident) => {
        impl ScalarParam for $t {
            const KIND: ParamKind = ParamKind::$k;
            const UNIT: ParameterUnit = ParameterUnit::$u;
            const DEFAULT_CURVE: ResponseCurve = ResponseCurve::$c;
        }
    };
}

/// Implement [`ScalarParam`] for each choice enum (`Enum` / no unit / linear).
macro_rules! scalar_enum {
    ($($t:ty),* $(,)?) => {
        $( scalar!($t => Enum, None, Linear); )*
    };
}

// --- Continuous: f32-backed newtypes -----------------------------------------
scalar!(Hertz => Continuous, Hertz, Logarithmic);
scalar!(Decibels => Continuous, Decibels, Linear);
scalar!(Gain => Continuous, None, Squared);
scalar!(Ratio => Continuous, Ratio, Linear);
scalar!(Seconds => Continuous, Seconds, Exponential);
scalar!(Milliseconds => Continuous, Milliseconds, Exponential);
scalar!(Cents => Continuous, Cents, Linear);
scalar!(Semitones => Continuous, Semitones, Linear);
scalar!(NormalizedValue => Continuous, None, Linear);
scalar!(BipolarValue => Continuous, None, Linear);
scalar!(Phase => Continuous, None, Linear);
scalar!(BeatDivision => Continuous, Beats, Linear);
scalar!(TimeScale => Continuous, None, Linear);
scalar!(PulseWidth => Continuous, None, Linear);
scalar!(BitDepth => Continuous, None, Linear);
scalar!(DecayTrim => Continuous, None, Linear);
// Raw f32 (KineticParam read-out values) — should get a newtype later.
scalar!(f32 => Continuous, None, Linear);

// --- Integer: primitives + integer-backed newtypes ---------------------------
scalar!(u8 => Integer, None, Linear);
scalar!(u32 => Integer, None, Linear);
scalar!(i32 => Integer, None, Linear);
scalar!(usize => Integer, None, Linear);
scalar!(VoiceCount => Integer, None, Linear);
scalar!(StepCount => Integer, None, Linear);
scalar!(MidiNote => Integer, None, Linear);
scalar!(SidNoiseSeed => Integer, None, Linear);
scalar!(Octaves => Integer, Octaves, Linear);

// --- Bool --------------------------------------------------------------------
scalar!(bool => Bool, None, Linear);

// --- Reference: opaque ids / addresses ---------------------------------------
scalar!(SampleId => Reference, None, Linear);
scalar!(Option<SrcAddr> => Reference, None, Linear);
scalar!(Option<DestAddr> => Reference, None, Linear);

// --- Enum: choice / mode enums (the ~25 picker enums) ------------------------
scalar_enum!(
    Waveform,
    MathAlgo,
    FmMode,
    AntiAliasMode,
    SubOscWaveform,
    SubOscOctave,
    NoiseType,
    FilterMode,
    FilterModel,
    LfoWaveform,
    DelayMode,
    DistortionMode,
    ReverseGateMode,
    ReverseGateTrigger,
    FftSizeOption,
    GrainSource,
    GrainWindow,
    EasingCurve,
    KineticLoopMode,
    ChaoticSystem,
    MechanicalNoiseType,
    TuringScale,
    Polarity,
    ImpulseResponse,
    SamplerPlayMode,
    PlayDirection,
    WaveshaperCurve,
    WavetableSelect,
    ModMatrixGridSize,
    SidModel,
    SidClock,
    SidQuality,
);
