//! Blanket [`ModuleParam`] impls for every `*Param` enum and the aggregate
//! [`Param`] (Phase 7). Each impl *delegates* to the type's existing inherent
//! methods, so the inherent methods remain the single definition and every call
//! site (`param.as_f32()`, …) is untouched — inherent methods win name resolution,
//! so `Self::as_f32(self)` here resolves to the inherent fn (no recursion).
//!
//! The macro forces the full contract on every listed type: a `*Param` enum
//! missing any method is a compile error here.

use super::*;
use crate::module_traits::{ModuleParam, ParamKind, ParameterUnit, ResponseCurve};

/// Implement [`ModuleParam`] for each type by delegating to its inherent methods.
macro_rules! impl_module_param {
    ($($t:ty),* $(,)?) => {
        $(
            impl ModuleParam for $t {
                #[inline]
                fn as_f32(&self) -> f32 {
                    Self::as_f32(self)
                }
                #[inline]
                fn with_f32(&self, value: f32) -> Self {
                    Self::with_f32(self, value)
                }
                #[inline]
                fn same_kind(&self, other: &Self) -> bool {
                    Self::same_kind(self, other)
                }
                #[inline]
                fn name(&self) -> &'static str {
                    Self::name(self)
                }
                #[inline]
                fn kind(&self) -> ParamKind {
                    Self::kind(self)
                }
                #[inline]
                fn unit(&self) -> ParameterUnit {
                    Self::unit(self)
                }
                #[inline]
                fn default_curve(&self) -> ResponseCurve {
                    Self::default_curve(self)
                }
            }
        )*
    };
}

impl_module_param!(
    Param,
    AdditiveParam,
    AmFormantParam,
    BeatDetectorParam,
    ChaoticOscParam,
    ConvolverParam,
    DriftGeneratorParam,
    BbdDelayParam,
    ChorusParam,
    CompressorParam,
    CrossoverParam,
    DelayParam,
    DistortionParam,
    EnsembleChorusParam,
    EqParam,
    FlangerParam,
    GranularFxParam,
    LimiterParam,
    MidSideParam,
    ModalResonatorParam,
    PhaserParam,
    ReverbParam,
    ReverseGateReverbParam,
    ShimmerReverbParam,
    SpectralBlurParam,
    TiltEqParam,
    UnivibeParam,
    VocoderParam,
    EnvelopeFollowerParam,
    EnvelopeParam,
    FilterParam,
    FofParam,
    FooglersParam,
    FormantFilterParam,
    FractalOscParam,
    FrequencyShifterParam,
    EuclideanParam,
    RandomGatesParam,
    TuringMachineParam,
    GranularParam,
    KineticParam,
    LaSynthParam,
    LfoParam,
    ModMatrixParam,
    AmplifierParam,
    LevelMeterParam,
    MixerParam,
    OscilloscopeParam,
    MsegParam,
    NoiseParam,
    MathOscillatorParam,
    OscillatorParam,
    PadSynthParam,
    PhaseVocoderParam,
    BodyResonanceParam,
    KeyboardPannerParam,
    SpatialPannerParam,
    MechanicalNoiseParam,
    PitchTrackerParam,
    RingModParam,
    SamplerParam,
    SidOscillatorParam,
    SignalMonitorParam,
    SpectrumAnalyzerParam,
    SubOscParam,
    VectorMixerParam,
    VocalTractParam,
    VoiceSynthParam,
    WaveshaperParam,
    WavetableParam,
);

#[cfg(test)]
mod tests {
    use super::*;

    /// A generic helper over `ModuleParam` compiles and runs — the whole point of
    /// the trait (generic code over any param enum or the aggregate).
    fn round_trip<P: ModuleParam>(p: &P) -> P {
        p.with_f32(p.as_f32())
    }

    #[test]
    fn generic_over_module_param() {
        let osc = OscillatorParam::Frequency(crate::Hertz::new(440.0));
        // Used generically through the trait (not the inherent method).
        assert_eq!(ModuleParam::kind(&osc), ParamKind::Continuous);
        assert!(round_trip(&osc).same_kind(&osc));

        // The aggregate `Param` also implements the trait.
        let p = Param::Oscillator(osc);
        assert_eq!(<Param as ModuleParam>::kind(&p), ParamKind::Continuous);
    }
}
