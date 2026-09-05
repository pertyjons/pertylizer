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
    /// A float outside the closed range its domain admits.
    ///
    /// Distinct from [`Self::OutOfRange`], which is an *index* outside a set: this is a
    /// continuous value outside an interval, and the two carry different numbers.
    #[error("{quantity} must be within {minimum} to {maximum}, and {value} is not")]
    OutsideInterval {
        /// The type that refused.
        quantity: &'static str,
        /// The rejected value.
        value: f32,
        /// The lowest admissible value.
        minimum: f32,
        /// The highest admissible value.
        maximum: f32,
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
    /// An index outside the set it indexes.
    #[error("{quantity} {value} is outside a set of {maximum}")]
    OutOfRange {
        /// The type that refused.
        quantity: &'static str,
        /// The rejected index.
        value: u64,
        /// How many elements the set has.
        maximum: u64,
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
counted_quantity!(
    RecordCount,
    u32,
    "records",
    "How many node records a plan schedules.

Not the node count, and the difference is what makes it its own quantity: the output node
has no kernel and no prepared data, so it schedules no record, while every operation the
compiler inserts does. It is therefore the length of the prepared, state and
control-range tables, and the figure every *per scheduled record* row and allocation is
over."
);
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

impl EventCount {
    /// The sum, or `None` where it cannot be represented.
    ///
    /// ADR-0046 clause 1 requires the producer-share sum to be checked. Six capacities
    /// can sum past what one can hold, and a wrapped sum would report a fitting
    /// partition for one that does not fit — the exact failure the partition exists to
    /// prevent.
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(total) => Some(Self(total)),
            None => None,
        }
    }

    /// This many events in each of `quanta` quanta, or `None` where the product cannot
    /// be represented.
    ///
    /// Written as the unit equation rather than as a bare multiplication: a per-quantum
    /// count times a number of quanta is a number of events. That is the whole reason
    /// [`QuantumCount`] exists — ADR-0046 clause 1 multiplies event capacities by it and
    /// requires the operand to carry its unit rather than arrive as a raw count.
    pub const fn checked_over(self, quanta: QuantumCount) -> Option<Self> {
        match self.0.checked_mul(quanta.0) {
            Some(total) => Some(Self(total)),
            None => None,
        }
    }
}

counted_quantity!(
    QuantumCount,
    u32,
    "quanta",
    "A number of render quanta.\n\nADR-0046 clause 1 requires the derived `max_quanta_per_callback` to be one of these rather than a raw count: it multiplies event capacities, and a bare integer there is the unit confusion `HOST-INV-018` exists to prevent. Distinct from [`EventCount`] and from [`crate::time::FrameCount`], and deliberately not convertible to either — a quantum is neither an event nor a frame."
);
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
counted_quantity!(
    QuantumBytes,
    u64,
    "bytes per quantum",
    "The bytes one quantum of a signal occupies — an observation tap's declared cost (`SOUND-INV-022`).\n\nA rate rather than a total, and its own type so that a subscription's admission cannot take it for one of the prepared or mutable byte totals."
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

impl SampleRate {
    /// Half this rate: the highest frequency a stream at it can represent.
    ///
    /// Typed, because it is what a node's frequency is *compared against* and a bare
    /// `f32` there would let a diagnostic carry a negative or non-finite bound. The rate
    /// is validated positive and finite, so half of it is too.
    pub fn nyquist(self) -> Frequency {
        Frequency::new(self.0 / 2.0).unwrap_or(Frequency::ZERO)
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

    /// Concert A, 440 Hz — the resting pitch a declaration presents for a pitched source
    /// and the frequency the lowerer prepares an oscillator with before a note reaches it.
    pub const A4: Self = Self(440.0);

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

/// A linear gain factor: what a signal is multiplied *by*.
///
/// Separate from [`Amplitude`], which is how loud a signal *is*. They share a
/// representation and nothing else: unity is the identity here and a peak there,
/// values above one are ordinary here and clipping there, and no arithmetic converts
/// one into the other. Keeping them apart is what stops a node's output level being
/// passed where its multiplier belongs.
///
/// Non-finite is refused, for the same reason [`Frequency`] refuses it. Negative is
/// legal and inverts the signal.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct GainFactor(f32);

impl GainFactor {
    /// Silence: everything multiplied to zero.
    pub const SILENT: Self = Self(0.0);

    /// The identity.
    pub const UNITY: Self = Self(1.0);

    /// A gain factor. Must be finite.
    pub fn new(linear: f32) -> Result<Self, QuantityError> {
        if linear.is_finite() {
            Ok(Self(linear))
        } else {
            Err(QuantityError::NotFinite {
                quantity: "GainFactor",
                value: linear,
            })
        }
    }

    /// The raw factor.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for GainFactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "x{}", self.0)
    }
}

impl std::fmt::Display for Amplitude {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// How many frames one envelope segment lasts.
///
/// A count of frames, not a position on the stream: [`crate::time::FrameCount`] is that,
/// and it is a `u64` because a stream outlives any segment. This is bounded at
/// `u32::MAX` — a little over a day at 48 kHz — and the bound is enforced where a
/// duration becomes a count, because that is the only place a diagnostic can be produced.
/// Zero is legal and means instantaneous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct SegmentFrames(u32);

impl SegmentFrames {
    /// Instantaneous.
    pub const NONE: Self = Self(0);

    /// A count of frames.
    pub const fn new(frames: u32) -> Self {
        Self(frames)
    }

    /// The count.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// One frame fewer, stopping at zero.
    pub const fn spent(self) -> Self {
        Self(self.0.saturating_sub(1))
    }

    /// Whether the segment has no frames left.
    pub const fn is_finished(self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for SegmentFrames {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} frames", self.0)
    }
}

/// A level in `[0, 1]`.
///
/// Separate from [`Amplitude`], which deliberately admits negative values — an inverted
/// signal is an ordinary thing — and values above one, which are ordinary too. A
/// *level* is neither: an envelope's sustain outside `[0, 1]` would invert or amplify
/// whatever reads it, and a negative one would make the segment that falls towards it
/// rise instead.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct NormalizedLevel(f32);

impl NormalizedLevel {
    /// The bottom of the range.
    pub const ZERO: Self = Self(0.0);

    /// The top of the range.
    pub const FULL: Self = Self(1.0);

    /// A level. Must be finite and within `[0, 1]`.
    pub fn new(level: f32) -> Result<Self, QuantityError> {
        if !level.is_finite() {
            return Err(QuantityError::NotFinite {
                quantity: "NormalizedLevel",
                value: level,
            });
        }
        if !(0.0..=1.0).contains(&level) {
            return Err(QuantityError::OutsideInterval {
                quantity: "NormalizedLevel",
                value: level,
                minimum: 0.0,
                maximum: 1.0,
            });
        }
        Ok(Self(level))
    }

    /// The raw level.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for NormalizedLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A duration in seconds.
///
/// The unit an authored envelope segment is written in. It is **not** a position: this
/// crate's positions and spans in frames are [`crate::time::FrameCount`] and
/// [`crate::time::PlanPosition`], and the two kinds of time meet exactly once, where a
/// segment's duration becomes a per-sample increment against a rate.
///
/// Zero is legal and means instantaneous. Negative is not: a segment that runs backwards
/// has no meaning, and admitting one would put a negative increment into an envelope.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct Seconds(f32);

impl Seconds {
    /// Instantaneous.
    pub const ZERO: Self = Self(0.0);

    /// A duration. Must be finite and not negative.
    pub fn new(seconds: f32) -> Result<Self, QuantityError> {
        if !seconds.is_finite() {
            return Err(QuantityError::NotFinite {
                quantity: "Seconds",
                value: seconds,
            });
        }
        if seconds < 0.0 {
            return Err(QuantityError::Negative {
                quantity: "Seconds",
                value: seconds,
            });
        }
        Ok(Self(seconds))
    }

    /// The raw duration.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for Seconds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} s", self.0)
    }
}

/// A filter's corner frequency.
///
/// Separate from [`Frequency`], which a phase accumulator turns into a waveform and which
/// is legally negative — a sine running backwards is an ordinary thing. A corner
/// frequency is not: zero or negative has no filter, and the coefficient formula that
/// consumes it divides by a sine of it. The type refuses both, and the *upper* bound is
/// refused where the stream is known, because Nyquist is a property of the rate rather
/// than of the value.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct CutoffFrequency(f32);

impl CutoffFrequency {
    /// A corner frequency. Must be finite and above zero.
    pub fn new(hz: f32) -> Result<Self, QuantityError> {
        if !hz.is_finite() {
            return Err(QuantityError::NotFinite {
                quantity: "CutoffFrequency",
                value: hz,
            });
        }
        if hz <= 0.0 {
            return Err(QuantityError::NotPositive {
                quantity: "CutoffFrequency",
                value: hz,
            });
        }
        Ok(Self(hz))
    }

    /// The raw frequency.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for CutoffFrequency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} Hz", self.0)
    }
}

/// A filter's quality factor.
///
/// Must be finite and above zero: the coefficient formula divides by twice this value, so
/// zero is not a flat filter but a division. Values below `0.5` are overdamped and legal;
/// values well above it resonate, which is the point.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct Resonance(f32);

impl Resonance {
    /// The neutral, maximally flat value.
    pub const BUTTERWORTH: Self = Self(std::f32::consts::FRAC_1_SQRT_2);

    /// A quality factor. Must be finite and above zero.
    pub fn new(q: f32) -> Result<Self, QuantityError> {
        if !q.is_finite() {
            return Err(QuantityError::NotFinite {
                quantity: "Resonance",
                value: q,
            });
        }
        if q <= 0.0 {
            return Err(QuantityError::NotPositive {
                quantity: "Resonance",
                value: q,
            });
        }
        Ok(Self(q))
    }

    /// The raw quality factor.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for Resonance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Q {}", self.0)
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

    /// One.
    ///
    /// Named because a gate edge has to become a value somewhere, and a literal `1.0`
    /// built through the fallible constructor on the audio thread would need a fallback
    /// nobody could justify. A raised gate is any value above zero; this is the one the
    /// note payload chooses.
    pub const ONE: Self = Self(1.0);

    /// This value as a frequency. Infallible: both types admit exactly the finite floats.
    pub const fn into_frequency(self) -> Frequency {
        Frequency(self.0)
    }

    /// A resolved frequency as the value a control write carries.
    ///
    /// The inverse of [`Self::into_frequency`], and infallible for the same reason: a
    /// [`Frequency`] is finite by construction and this type admits exactly the finite
    /// floats. `SOUND-INV-021`'s pitch expansion needs it on the **audio thread**, where a
    /// fallible constructor would need a fallback nobody could justify — a substituted
    /// frequency is a substituted note.
    pub const fn from_frequency(frequency: Frequency) -> Self {
        Self(frequency.0)
    }

    /// A note's velocity as the value a control write carries.
    ///
    /// Infallible on the same ground: `[0, 1]` is inside the finite floats, so widening
    /// cannot fail. The narrowing direction is not, which is why it is
    /// [`NoteVelocity::saturating`] and lives on that type.
    pub const fn from_note_velocity(velocity: NoteVelocity) -> Self {
        Self(velocity.0)
    }

    /// This value as an amplitude. Infallible, for the same reason.
    pub const fn into_amplitude(self) -> Amplitude {
        Amplitude(self.0)
    }

    /// A frame count carried as a control value, the inverse of [`Self::as_frames`].
    #[allow(
        clippy::cast_precision_loss,
        reason = "a fade length is far below 2^24 frames, where every count is exact in f32"
    )]
    pub fn from_frames(frames: crate::time::FrameCount) -> Self {
        Self(frames.as_u64().min(u64::from(u32::MAX)) as f32)
    }

    /// The value read as a count of frames, for a control that carries one (ADR-0058's
    /// fade length). Negative and fractional parts are dropped; a count above `2^24` is
    /// not exact in `f32` and no fade is that long.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a frame count carried through a control value; clamped to the type's range \
                  first, so the conversion is saturating rather than truncating"
    )]
    pub fn as_frames(self) -> u32 {
        self.0.max(0.0).min(u32::MAX as f32) as u32
    }

    /// An amplitude as the value a control write carries. Infallible: an [`Amplitude`] is
    /// finite by construction.
    pub const fn from_amplitude(amplitude: Amplitude) -> Self {
        Self(amplitude.0)
    }

    /// A level as the value a control write carries. Infallible: `[0, 1]` is inside the
    /// finite floats.
    pub const fn from_level(level: NormalizedLevel) -> Self {
        Self(level.0)
    }

    /// A value from arithmetic that may have left the finite domain, by the documented
    /// saturating policy: an overflow becomes the widest finite value of its sign, and a
    /// `NaN` — which only `0 × ∞` produces where the arithmetic is a law's — becomes zero,
    /// because a zero base is zero under every law.
    ///
    /// The one place a parameter value is built from arithmetic rather than from a typed
    /// quantity: `SOUND-INV-023`'s composition runs on the audio thread, where nothing can
    /// refuse, and a law's exponential can overflow for a modulation sum nothing bounds
    /// yet. `SOUND-INV-024`'s segment then only ever interpolates between two of these,
    /// which cannot leave the domain.
    pub fn saturating(value: f32) -> Self {
        if value.is_nan() {
            Self(0.0)
        } else {
            Self(value.clamp(f32::MIN, f32::MAX))
        }
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

/// How many control writes one note-on expands to, its gate included.
///
/// `SOUND-INV-021` makes a note-on more than one write, and the sample-positioned control
/// scratch is sized on this: a budget stated over one write per event is overrun by the first
/// note whose scope declares a pitch and a velocity destination. It is a **capacity**, not a
/// count of anything observed, which is why it crosses the admission and preparation boundary
/// as its own type rather than as a `u32` interchangeable with an event count.
///
/// **Zero is unrepresentable rather than refused.** There is no fallible constructor to
/// unwrap on a derivation path: every note writes at least its gate, so the type is built from
/// [`Self::GATE_ONLY`] upward and a count of zero cannot be spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct WritesPerNote(u32);

impl WritesPerNote {
    /// A note that moves its gate and nothing else.
    ///
    /// Every plan's floor, and what a plan with no magnitude destination charges.
    pub const GATE_ONLY: Self = Self(1);

    /// The gate plus this many magnitude destinations.
    /// The wider of this and a write that fans out over `instances` voice rows: since
    /// `P06-S001` a sample-positioned `SetParameter` to a voice-scope parameter writes one
    /// control per instance, and the scratch a quantum's events need is sized by the widest
    /// write any one event can make.
    pub const fn fanned_out(self, instances: VoiceCount) -> Self {
        if instances.get() > self.0 {
            Self(instances.get())
        } else {
            self
        }
    }

    /// The wider of two figures.
    pub const fn widest(self, other: Self) -> Self {
        if other.0 > self.0 { other } else { self }
    }

    /// A count of writes, floored at one.
    pub const fn at_least(writes: u32) -> Self {
        Self(if writes == 0 { 1 } else { writes })
    }

    pub const fn with_magnitudes(magnitudes: u32) -> Self {
        Self(magnitudes.saturating_add(1))
    }

    /// The raw count.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl std::fmt::Display for WritesPerNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} writes per note", self.0)
    }
}

/// A keyboard position a note names.
///
/// `SOUND-INV-021`: the note payload's pitch limb is a **key**, not a frequency. What frequency
/// a key is belongs to the prepared tuning the plan resolves it through, so nothing here knows
/// about scales, reference pitches or equal temperament.
///
/// # Why it is not `synth_core::MidiNote`
///
/// That type's constructor replaces a value above 127 with 127. A key arriving from a saved
/// project or a live adapter is external input, and silently substituting a different note for
/// one out of range is the case `AGENTS.md`'s numeric rule exists to prevent — the render would
/// simply play a different note than the project says, with nothing reporting it. This refuses
/// instead, at the boundary where a diagnostic is still possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct KeyIdentity(u8);

impl KeyIdentity {
    /// The lowest key a keyboard names.
    pub const LOWEST: Self = Self(0);

    /// The highest.
    pub const HIGHEST: Self = Self(127);

    /// A key. Must be a keyboard position in `0..=127`.
    pub fn new(key: u8) -> Result<Self, QuantityError> {
        if key <= 127 {
            Ok(Self(key))
        } else {
            Err(QuantityError::OutsideInterval {
                quantity: "KeyIdentity",
                value: f32::from(key),
                minimum: 0.0,
                maximum: 127.0,
            })
        }
    }

    /// The raw keyboard position.
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Its position in a table indexed by key.
    pub const fn as_index(self) -> usize {
        self.0 as usize
    }
}

impl std::fmt::Display for KeyIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "key {}", self.0)
    }
}

/// How hard a note was struck.
///
/// `SOUND-INV-021`: one validated normalized magnitude on the on edge, because V1 consumes a
/// single saved velocity at both its envelope and its voice output and two consumers of one
/// fact do not need two fields.
///
/// Zero is legal and means a note struck with no force — a silent note is representable, and
/// V1's own `Velocity` admits it. That is not a release: a release is the off edge, and this
/// travels beside the gate rather than being it.
///
/// # Why it is not `synth_core::Velocity`
///
/// That type's constructor clamps into `[0, 1]`. This refuses, for the reason [`KeyIdentity`]
/// gives: a value outside the range is external input that means something, and replacing it
/// silently renders a different note than the project asks for.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct NoteVelocity(f32);

impl NoteVelocity {
    /// Struck with no force.
    pub const SILENT: Self = Self(0.0);

    /// Struck as hard as the range admits.
    pub const FULL: Self = Self(1.0);

    /// A velocity at a destination that has no way to refuse one.
    ///
    /// **The saturating constructor, and the type owning that policy is the point of it.**
    /// `SOUND-INV-021` gives the envelope a velocity control that is addressable as an
    /// ordinary parameter, and `ParameterValue` admits every finite float — "what a
    /// particular parameter does with a finite value is that parameter's business". A `2.0`
    /// would over-amplify the envelope and a `-1.0` would **invert** it, and the audio
    /// thread can neither allocate a diagnostic nor drop the write without leaving the
    /// previous note's velocity in force.
    ///
    /// So the domain is enforced here, by the same two-constructor shape the rest of this
    /// module uses: [`Self::new`] refuses, and is what the **note payload** is built
    /// through — a key or a velocity arriving from a saved project or a live adapter is
    /// external input, and substituting for it silently is what that constructor exists to
    /// prevent. This one saturates, and is reachable only from the parameter path. V1
    /// applies the same policy at the same place: its own `Velocity` clamps.
    ///
    /// `NaN` becomes [`Self::SILENT`] rather than propagating. It cannot arrive from a
    /// `ParameterValue`, which is finite by construction, but a total function needs a total
    /// answer and `f32::clamp` returns `NaN` unchanged — which would then multiply every
    /// sample the envelope emits for the rest of the render.
    pub fn saturating(velocity: f32) -> Self {
        if velocity.is_nan() {
            return Self::SILENT;
        }
        Self(velocity.clamp(0.0, 1.0))
    }

    /// A velocity. Must be finite and within `[0, 1]`.
    pub fn new(velocity: f32) -> Result<Self, QuantityError> {
        if !velocity.is_finite() {
            return Err(QuantityError::NotFinite {
                quantity: "NoteVelocity",
                value: velocity,
            });
        }
        if !(0.0..=1.0).contains(&velocity) {
            return Err(QuantityError::OutsideInterval {
                quantity: "NoteVelocity",
                value: velocity,
                minimum: 0.0,
                maximum: 1.0,
            });
        }
        Ok(Self(velocity))
    }

    /// The raw magnitude.
    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl std::fmt::Display for NoteVelocity {
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

/// One channel's position within a layout.
///
/// Validated against the layout it indexes, so a channel a stream does not have is
/// unrepresentable rather than silently dropped by a bounds-checked write. ADR-0002
/// clause 4 makes ordering part of the layout — channel 0 of a stereo signal is the
/// left channel — which is why this is an index into an ordered set and not a count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct ChannelIndex(u16);

impl ChannelIndex {
    /// The first channel, which every layout has.
    pub const FIRST: Self = Self(0);

    /// A channel of `layout`.
    pub fn in_layout(index: usize, layout: ChannelLayout) -> Result<Self, QuantityError> {
        let raw = u16::try_from(index)
            .ok()
            .filter(|_| index < layout.channels());
        raw.map(Self).ok_or(QuantityError::OutOfRange {
            quantity: "ChannelIndex",
            value: index as u64,
            maximum: layout.channels() as u64,
        })
    }

    /// The index.
    pub const fn get(self) -> usize {
        self.0 as usize
    }
}

impl std::fmt::Display for ChannelIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "channel {}", self.0)
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
