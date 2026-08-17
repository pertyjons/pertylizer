//! The typed quantities a host profile is made of.
//!
//! `HOST-INV-018` governs this module: every profile field carrying a
//! **quantity** is a newtype with a private field and a fallible constructor, no
//! such field is a bare `usize`, `u32`, or `f32`, and no two fields whose units
//! differ share a type. The two fields carrying a **kind** rather than a
//! quantity — [`ChannelLayout`] and the capability source — are closed enums
//! instead, because an enum admits no invalid value in the first place.
//!
//! # Two constructors, two invariants
//!
//! Each quantity has both:
//!
//! - `limit` builds a **capacity**. Zero is refused: a capacity of zero admits
//!   nothing, so a profile carrying one is a profile that can compile no plan.
//! - `measured` builds an **observed amount** for the resource report. Zero is
//!   ordinary — a plan may request no voices — but an invalid *representation* is
//!   still refused, which for a float means non-finite.
//!
//! Which constructor ran is itself part of the guarantee, in the same way
//! `HostCapabilities`' two constructors are.

use synth_core::audio::DeviceSampleRate;
use thiserror::Error;

/// A quantity that refused to be built.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum QuantityError {
    /// A capacity of zero, which would admit nothing.
    #[error("{quantity} may not be zero: a capacity of zero admits nothing")]
    ZeroCapacity {
        /// The type that refused.
        quantity: &'static str,
    },
    /// A non-finite float.
    #[error("{quantity} must be finite, not {value}")]
    NotFinite {
        /// The type that refused.
        quantity: &'static str,
        /// The rejected value.
        value: f32,
    },
    /// A float at or below zero where the domain is positive.
    #[error("{quantity} must be greater than zero, not {value}")]
    NotPositive {
        /// The type that refused.
        quantity: &'static str,
        /// The rejected value.
        value: f32,
    },
    /// A float below zero where the domain is non-negative.
    #[error("{quantity} may not be negative, and {value} is")]
    Negative {
        /// The type that refused.
        quantity: &'static str,
        /// The rejected value.
        value: f32,
    },
    /// A range whose endpoints are the wrong way round.
    #[error("sample-rate range {minimum} Hz to {maximum} Hz is inverted")]
    InvertedRange {
        /// The lower endpoint as given.
        minimum: f32,
        /// The upper endpoint as given.
        maximum: f32,
    },
    /// A rate range reaching above the engine's own ceiling.
    ///
    /// `LIMIT-0004` is why this is refused rather than accepted as an operator's
    /// choice: real-time look-ahead and scratch buffers are sized from the engine
    /// ceiling, so a stream above it gets less DSP than its parameters advertise
    /// with no diagnostic. Moving the ceiling is an engine change.
    #[error("sample-rate maximum {maximum} Hz is above the engine ceiling of {ceiling} Hz")]
    AboveEngineCeiling {
        /// The rejected upper endpoint.
        maximum: f32,
        /// The engine-wide ceiling.
        ceiling: f32,
    },
}

/// Define a counted quantity: private field, two constructors, one unit.
macro_rules! counted_quantity {
    ($name:ident, $inner:ty, $unit:literal, $doc:expr) => {
        #[doc = $doc]
        ///
        /// Built by `limit` as a capacity, where zero is refused, or by
        /// `measured` as an observed amount, where zero is ordinary.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[must_use]
        pub struct $name($inner);

        impl $name {
            /// No amount of this quantity. A measurement, never a capacity.
            pub const NONE: Self = Self(0);

            /// A capacity. Zero is refused.
            pub const fn limit(value: $inner) -> Result<Self, QuantityError> {
                if value == 0 {
                    Err(QuantityError::ZeroCapacity {
                        quantity: stringify!($name),
                    })
                } else {
                    Ok(Self(value))
                }
            }

            /// An observed amount. Zero is ordinary; there is no invalid integer.
            pub const fn measured(value: $inner) -> Self {
                Self(value)
            }

            /// The raw amount.
            pub const fn get(self) -> $inner {
                self.0
            }

            /// The amount as an index, or `None` where it does not fit one.
            pub const fn as_usize(self) -> Option<usize> {
                if self.0 as u128 <= usize::MAX as u128 {
                    Some(self.0 as usize)
                } else {
                    None
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{} {}", self.0, $unit)
            }
        }
    };
}

counted_quantity!(NodeCount, u32, "nodes", "A number of graph nodes.");
counted_quantity!(EdgeCount, u32, "edges", "A number of graph edges.");
counted_quantity!(
    FanOut,
    u32,
    "edges per port",
    "How many edges leave one output port."
);
counted_quantity!(
    VoiceCount,
    u32,
    "voices",
    "A number of voices.\n\nThe V2 type: V1's clamping `synth_core::VoiceCount` is replaced, not reused, per ADR-0021 part 3."
);
counted_quantity!(
    HeldNoteCount,
    u32,
    "held notes",
    "A number of held notes.\n\nDistinct from [`VoiceCount`] and deliberately not convertible to it: a held note is a source obligation and a voice is a resource, and more notes can be held than sounded. A sustain pedal, a stealing allocator, and an MPE source all do it."
);
counted_quantity!(
    EventCount,
    u32,
    "events",
    "A number of events — per quantum, per tick, or per queue."
);
counted_quantity!(TapCount, u32, "taps", "A number of observation taps.");
counted_quantity!(
    SlotCount,
    u32,
    "slots",
    "A number of slots: modulation, script host, script state, or script output."
);
counted_quantity!(
    InstructionCount,
    u32,
    "instructions",
    "A number of script instructions."
);
counted_quantity!(
    MixChannelCount,
    u32,
    "mix channels",
    "A number of mix channels.\n\nNot named `ChannelCount`: `synth_core::ChannelCount` already exists in this workspace and means a channel *layout*. Reusing the name is the hazard ADR-0032 clause 5 refused for `SampleOffset` — two unrelated meanings one import away."
);
counted_quantity!(BusCount, u32, "buses", "A number of mix buses.");
counted_quantity!(
    SendCount,
    u32,
    "sends",
    "A number of sends from one channel."
);
counted_quantity!(
    ScriptWorkPerQuantum,
    u64,
    "instructions per quantum",
    "Script instructions evaluated across a plan in one quantum.\n\nWider than [`InstructionCount`] because it is a **product**: instructions per program times evaluations per quantum overflows `u32` for values each of which is separately admissible, and saturating would make the report understate the work by orders of magnitude while looking precise."
);
counted_quantity!(
    PreparedBytes,
    u64,
    "bytes",
    "A number of bytes of prepared memory.\n\nThree separate profile fields use it: the type carries the unit and the field carries the kind."
);

/// A dimensionless ratio: predicted quantum cost over the quantum's real-time
/// budget.
///
/// The only non-count in the profile, and the only field the evidence genuinely
/// drives. It is **advisory**: it may warn and may never refuse, because the cost
/// model behind it is a prediction from measurements taken on V1, offline, on one
/// machine.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct CostRatio(f32);

impl CostRatio {
    /// A budget. Must be finite and above zero.
    pub fn limit(value: f32) -> Result<Self, QuantityError> {
        if !value.is_finite() {
            return Err(QuantityError::NotFinite {
                quantity: "CostRatio",
                value,
            });
        }
        if value <= 0.0 {
            return Err(QuantityError::NotPositive {
                quantity: "CostRatio",
                value,
            });
        }
        Ok(Self(value))
    }

    /// A predicted ratio. Zero is ordinary — an empty plan costs nothing — but
    /// non-finite and negative are not, and a `NaN` must never reach the report.
    pub fn measured(value: f32) -> Result<Self, QuantityError> {
        if !value.is_finite() {
            return Err(QuantityError::NotFinite {
                quantity: "CostRatio",
                value,
            });
        }
        if value < 0.0 {
            return Err(QuantityError::Negative {
                quantity: "CostRatio",
                value,
            });
        }
        Ok(Self(value))
    }

    /// The raw ratio.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for CostRatio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.4} of budget", self.0)
    }
}

/// A sample rate in hertz.
///
/// **V2's own type.** V1's `synth_core::SampleRate::new` clamps `NaN`, zero, and
/// negative to `1.0`, so it can offer no fallible constructor and an invalid
/// input is indistinguishable from a genuine one hertz. ADR-0021 part 3 replaces
/// V1's clamping `VoiceCount` on exactly this ground, and this type is the same
/// move.
///
/// The conversion is one-way: [`From<SampleRate>`] for the `synth_core` type is
/// infallible and needed by the DSP kernels, and there is no reverse conversion,
/// because a clamped `1.0` cannot be told from one hertz.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct SampleRate(f32);

impl SampleRate {
    /// A rate. Must be finite and above zero.
    pub fn new(hz: f32) -> Result<Self, QuantityError> {
        if !hz.is_finite() {
            return Err(QuantityError::NotFinite {
                quantity: "SampleRate",
                value: hz,
            });
        }
        if hz <= 0.0 {
            return Err(QuantityError::NotPositive {
                quantity: "SampleRate",
                value: hz,
            });
        }
        Ok(Self(hz))
    }

    /// The raw rate.
    pub const fn as_f32(self) -> f32 {
        self.0
    }

    /// How many whole frames one second is at this rate, rounded to nearest.
    ///
    /// Used where a budget's meaning is a duration: `HOST-INV-011` requires such
    /// a budget to be evaluated in seconds at the prepared rate, never in frames,
    /// because a block carries the same work at every rate while its real-time
    /// budget shrinks with the rate.
    pub fn frames_per_second(self) -> crate::time::FrameCount {
        // The constructor establishes finite and positive, and the engine ceiling
        // bounds it far below `u64::MAX`, so the rounded value is representable.
        crate::time::FrameCount::new(self.0.round() as u64)
    }
}

impl std::fmt::Display for SampleRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} Hz", self.0)
    }
}

impl From<SampleRate> for synth_core::SampleRate {
    /// Infallible: the value has already passed this type's validation, and the
    /// DSP kernels take `synth_core`'s type.
    fn from(rate: SampleRate) -> Self {
        Self::new_unchecked(rate.0)
    }
}

/// The inclusive range of rates a stream may be prepared at.
///
/// This is `accepted_sample_rates`, `LIMIT-0004`'s successor and a **render
/// limit** rather than a capability: its ledger entry is a configurable budget
/// owned by the profile, and admission — not preparation — is what refuses on it.
/// Both endpoints are inclusive, so a host supporting exactly one rate is a range
/// one rate wide.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct SampleRateRange {
    minimum: SampleRate,
    maximum: SampleRate,
}

impl SampleRateRange {
    /// The lowest rate this engine accepts, below telephone quality.
    ///
    /// Chosen: no rule produces 8 000, and nothing this engine ships is usable
    /// below it.
    pub const MINIMUM_HZ: f32 = 8_000.0;

    /// A range, or a refusal.
    ///
    /// Rejects an inverted range, and rejects a maximum above the engine's own
    /// ceiling — which is not the operator's to raise, because buffers are sized
    /// from it.
    pub fn new(minimum: SampleRate, maximum: SampleRate) -> Result<Self, QuantityError> {
        if minimum.as_f32() > maximum.as_f32() {
            return Err(QuantityError::InvertedRange {
                minimum: minimum.as_f32(),
                maximum: maximum.as_f32(),
            });
        }
        let ceiling = DeviceSampleRate::MAX_SUPPORTED.as_f32();
        if maximum.as_f32() > ceiling {
            return Err(QuantityError::AboveEngineCeiling {
                maximum: maximum.as_f32(),
                ceiling,
            });
        }
        Ok(Self { minimum, maximum })
    }

    /// The range this engine supports: 8 000 Hz to the engine ceiling.
    ///
    /// The maximum is **derived** from `DeviceSampleRate::MAX_SUPPORTED`, which is
    /// where V1's own render ceiling derives from after `LIMIT-0004`'s fix, so the
    /// two cannot drift apart again.
    pub fn engine_supported() -> Self {
        Self {
            minimum: SampleRate(Self::MINIMUM_HZ),
            maximum: SampleRate(DeviceSampleRate::MAX_SUPPORTED.as_f32()),
        }
    }

    /// The lower endpoint, inclusive.
    pub const fn minimum(self) -> SampleRate {
        self.minimum
    }

    /// The upper endpoint, inclusive.
    pub const fn maximum(self) -> SampleRate {
        self.maximum
    }

    /// Whether `rate` is inside the range, endpoints included.
    pub fn contains(self, rate: SampleRate) -> bool {
        rate.as_f32() >= self.minimum.as_f32() && rate.as_f32() <= self.maximum.as_f32()
    }
}

impl std::fmt::Display for SampleRateRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} Hz to {} Hz",
            self.minimum.as_f32(),
            self.maximum.as_f32()
        )
    }
}

/// An oscillator frequency in hertz.
///
/// A separate type from [`SampleRate`] although both are hertz: `HOST-INV-018`'s rule
/// that no two fields whose units differ share a type has a converse — two quantities
/// that share a unit are still two concepts, and one of these bounds a stream while the
/// other is a control value inside it.
///
/// Non-finite is refused. A `NaN` frequency would poison a phase accumulator permanently:
/// once the phase is `NaN` every later sample is, and no later event can recover it.
/// Negative is legal and means the phase runs backwards.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct Frequency(f32);

impl Frequency {
    /// Silence's frequency.
    pub const ZERO: Self = Self(0.0);

    /// A frequency. Must be finite.
    pub fn new(hz: f32) -> Result<Self, QuantityError> {
        if hz.is_finite() {
            Ok(Self(hz))
        } else {
            Err(QuantityError::NotFinite {
                quantity: "Frequency",
                value: hz,
            })
        }
    }

    /// The raw frequency.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for Frequency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} Hz", self.0)
    }
}

/// A linear amplitude.
///
/// Non-finite is refused, for the same reason [`Frequency`] refuses it. Negative is legal
/// and means an inverted signal.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct Amplitude(f32);

impl Amplitude {
    /// Silent.
    pub const SILENT: Self = Self(0.0);

    /// Unity.
    pub const UNITY: Self = Self(1.0);

    /// An amplitude. Must be finite.
    pub fn new(linear: f32) -> Result<Self, QuantityError> {
        if linear.is_finite() {
            Ok(Self(linear))
        } else {
            Err(QuantityError::NotFinite {
                quantity: "Amplitude",
                value: linear,
            })
        }
    }

    /// The raw amplitude.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for Amplitude {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A value an event carries to a parameter.
///
/// Validated at the boundary the event crosses, not where it lands: a non-finite value
/// must not reach the audio thread at all, because the render loop has no way to refuse
/// one — it cannot allocate a diagnostic, and clamping silently is what this contract
/// exists to remove. What a *particular* parameter does with a finite value is that
/// parameter's business.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct ParameterValue(f32);

impl ParameterValue {
    /// Zero.
    pub const ZERO: Self = Self(0.0);

    /// This value as a frequency. Infallible: both types admit exactly the finite floats.
    pub const fn into_frequency(self) -> Frequency {
        Frequency(self.0)
    }

    /// This value as an amplitude. Infallible, for the same reason.
    pub const fn into_amplitude(self) -> Amplitude {
        Amplitude(self.0)
    }

    /// A value. Must be finite.
    pub fn new(value: f32) -> Result<Self, QuantityError> {
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(QuantityError::NotFinite {
                quantity: "ParameterValue",
                value,
            })
        }
    }

    /// The raw value.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for ParameterValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What a stream's channels are — a **kind**, not a quantity.
///
/// Phase 1 carries the layouts it renders and no others. ADR-0002 owns what a
/// layout may be, and adding a `Multi(n)` variant here would be the claim rather
/// than the absence of one: it would assert that a layout *is* a channel count,
/// when ADR-0002 may just as well define speaker roles or a layout set. Phase 9
/// queries a real device, is the first phase with somewhere to put a multichannel
/// value, and adds the variant together with that decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelLayout {
    /// One channel.
    Mono,
    /// Two channels, left then right, interleaved.
    Stereo,
}

impl ChannelLayout {
    /// How many channels the layout has.
    #[must_use]
    pub const fn channels(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
        }
    }
}

impl std::fmt::Display for ChannelLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mono => f.write_str("mono"),
            Self::Stereo => f.write_str("stereo"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capacity_of_zero_is_refused_and_a_measurement_of_zero_is_not() {
        assert_eq!(
            NodeCount::limit(0),
            Err(QuantityError::ZeroCapacity {
                quantity: "NodeCount"
            })
        );
        assert_eq!(NodeCount::measured(0).get(), 0);
        assert_eq!(NodeCount::limit(1).map(NodeCount::get), Ok(1));
    }

    #[test]
    fn every_counted_quantity_refuses_a_zero_capacity() {
        // One assertion per type, so a type added without the rule fails here
        // rather than silently accepting a capacity that admits nothing.
        assert!(NodeCount::limit(0).is_err());
        assert!(EdgeCount::limit(0).is_err());
        assert!(FanOut::limit(0).is_err());
        assert!(VoiceCount::limit(0).is_err());
        assert!(HeldNoteCount::limit(0).is_err());
        assert!(EventCount::limit(0).is_err());
        assert!(TapCount::limit(0).is_err());
        assert!(SlotCount::limit(0).is_err());
        assert!(InstructionCount::limit(0).is_err());
        assert!(MixChannelCount::limit(0).is_err());
        assert!(BusCount::limit(0).is_err());
        assert!(SendCount::limit(0).is_err());
        assert!(PreparedBytes::limit(0).is_err());
        assert!(CostRatio::limit(0.0).is_err());
    }

    #[test]
    fn a_cost_ratio_refuses_nan_from_both_constructors() {
        assert!(CostRatio::limit(f32::NAN).is_err());
        // The measurement path is the one that would otherwise put a `NaN` in the
        // resource report, where it compares false against everything.
        assert!(CostRatio::measured(f32::NAN).is_err());
        assert!(CostRatio::measured(f32::INFINITY).is_err());
        assert!(CostRatio::measured(-0.5).is_err());
        assert_eq!(CostRatio::measured(0.0).map(CostRatio::as_f32), Ok(0.0));
    }

    #[test]
    fn a_sample_rate_refuses_what_v1_would_have_clamped() {
        // Each of these is `1.0` after `synth_core::SampleRate::new`, which is
        // why V2 has its own type.
        assert!(SampleRate::new(f32::NAN).is_err());
        assert!(SampleRate::new(0.0).is_err());
        assert!(SampleRate::new(-48_000.0).is_err());
        assert!(SampleRate::new(48_000.0).is_ok());
    }

    #[test]
    fn a_rate_range_refuses_inversion_and_the_engine_ceiling() {
        let low = SampleRate::new(8_000.0).expect("valid rate");
        let high = SampleRate::new(48_000.0).expect("valid rate");
        assert!(SampleRateRange::new(low, high).is_ok());
        assert!(matches!(
            SampleRateRange::new(high, low),
            Err(QuantityError::InvertedRange { .. })
        ));

        // 384 kHz against a 192 kHz engine is `LIMIT-0004` exactly: V1 accepted
        // it and silently halved the limiter's advertised look-ahead.
        let above = SampleRate::new(384_000.0).expect("valid rate");
        assert!(matches!(
            SampleRateRange::new(low, above),
            Err(QuantityError::AboveEngineCeiling { .. })
        ));
    }

    #[test]
    fn a_range_may_be_one_rate_wide() {
        let fixed = SampleRate::new(44_100.0).expect("valid rate");
        let range = SampleRateRange::new(fixed, fixed).expect("equal endpoints are legal");
        assert!(range.contains(fixed));
    }

    #[test]
    fn the_engine_range_includes_both_endpoints_and_excludes_neighbours() {
        let range = SampleRateRange::engine_supported();
        assert!(range.contains(range.minimum()));
        assert!(range.contains(range.maximum()));
        assert!(!range.contains(SampleRate::new(7_999.0).expect("valid rate")));
        assert!(!range.contains(SampleRate::new(192_001.0).expect("valid rate")));
    }

    #[test]
    fn frames_per_second_is_the_rate_rounded() {
        let rate = SampleRate::new(44_100.0).expect("valid rate");
        assert_eq!(rate.frames_per_second().as_u64(), 44_100);
    }

    #[test]
    fn the_dsp_control_types_refuse_what_would_poison_a_phase() {
        // A `NaN` frequency makes every later sample `NaN`, and no later event recovers
        // it — which is why the refusal is at construction and not at the parameter.
        assert!(Frequency::new(f32::NAN).is_err());
        assert!(Frequency::new(f32::INFINITY).is_err());
        assert!(Amplitude::new(f32::NAN).is_err());
        assert!(ParameterValue::new(f32::NAN).is_err());

        // Negative is legal for both: a reversed phase and an inverted signal.
        assert!(Frequency::new(-440.0).is_ok());
        assert!(Amplitude::new(-1.0).is_ok());
        assert!(ParameterValue::new(-1.0).is_ok());
    }

    #[test]
    fn a_layout_reports_its_channel_count() {
        assert_eq!(ChannelLayout::Mono.channels(), 1);
        assert_eq!(ChannelLayout::Stereo.channels(), 2);
    }
}
