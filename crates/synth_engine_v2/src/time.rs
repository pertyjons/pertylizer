//! Time, quantum, and epoch types.
//!
//! Every type here is fixed by ADR-0032, and the quantum is ADR-0001's. Two
//! properties are worth stating before the definitions, because both are the
//! kind of thing an implementation loses first:
//!
//! - **No time type is a float, and none converts from one.** A `Seconds` may
//!   describe a duration a user typed; it may never carry a position
//!   (ADR-0032 clause 6).
//! - **Positions do not subtract into themselves.** The difference of two
//!   positions is a signed [`FrameDelta`], because an event one frame early
//!   becoming an event eighteen quintillion frames late is the classic form
//!   this defect takes (clause 3).

use thiserror::Error;

use std::sync::atomic::{AtomicU32, Ordering};

/// The internal render quantum in frames — ADR-0001's `Q`.
///
/// ADR-0037 fixes the value at 64 **provisionally**, under its rule 1: the V1
/// proxy measurement came back inconclusive, and the Phase 2 exit gate
/// re-measures against real V2 nodes and either confirms or supersedes it.
/// Until then nothing may be tuned to this number — no hand-unrolled kernel, no
/// `Q`-specific buffer layout, and no test asserting a control rate in Hz.
///
/// It is a `u32` while [`QuantumOffset`] is a `u16`, deliberately: a frame count is
/// naturally the wider type, and the width difference is what makes the assertion
/// below a real constraint instead of a tautology. Declaring both the same width
/// would make `Q <= QuantumOffset::MAX` unfailable, which is the opposite of what
/// ADR-0032 clause 4 asks for.
pub const QUANTUM_FRAMES: u32 = 64;

/// ADR-0032 clause 4's compile-time assertion.
///
/// [`QuantumOffset`] is a `u16` so that raising `Q` past 256 does not silently
/// truncate an offset. This fails the build instead, which is the point: the
/// Phase 2 re-measurement may change `Q`, and a type sized to the old value
/// must not survive it quietly.
const _: () = assert!(QUANTUM_FRAMES <= QuantumOffset::MAX as u32);

/// Something a time type refused to represent.
///
/// Every variant is a fault rather than a value to clamp: ADR-0032 clause 28
/// makes exhaustion terminal for the epoch precisely so that an unreachable case
/// cannot become a debug panic on the audio thread or a release wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TimeError {
    /// A quantum-local offset at or beyond `Q`.
    #[error("quantum offset {offset} is outside 0..{quantum}")]
    OffsetOutsideQuantum {
        /// The rejected offset.
        offset: u16,
        /// `Q`, in frames.
        quantum: u32,
    },
    /// [`SampleTime`] cannot advance or add without wrapping.
    ///
    /// Unreachable in practice — three million years at 192 kHz — which is
    /// exactly why it has a defined behaviour rather than an assertion.
    #[error("sample time {position} cannot advance by {frames} frames without wrapping")]
    ClockExhausted {
        /// The position that could not advance.
        position: u64,
        /// The advance that was refused.
        frames: u64,
    },
    /// A plan position cannot advance without wrapping.
    #[error("plan position {position} cannot advance by {frames} frames without wrapping")]
    PlanPositionExhausted {
        /// The position that could not advance.
        position: u64,
        /// The advance that was refused.
        frames: u64,
    },
    /// Two positions are further apart than a signed difference can hold.
    ///
    /// ADR-0032 clause 3: a difference `FrameDelta` cannot represent is a **fault**, not
    /// a wrap. Unreachable inside one epoch by the margin in the range analysis — it
    /// takes positions more than nine quintillion frames apart — and refusing is what
    /// keeps the alternative from being a silent sign flip, where a forward position
    /// reads as one frame in the past.
    #[error("positions {later} and {earlier} are too far apart for a signed difference")]
    DifferenceUnrepresentable {
        /// The later position.
        later: u64,
        /// The earlier position.
        earlier: u64,
    },
    /// The process has issued every [`StreamEpoch`] identifier.
    ///
    /// ADR-0032 clause 12 requires identifiers to be strictly increasing and
    /// never reused, so a preparation that would exhaust the space fails rather
    /// than wrapping into a live producer's epoch.
    #[error("stream epoch identifiers are exhausted for this process")]
    EpochSpaceExhausted,
}

/// A non-negative duration in frames — a distance, a length, a horizon.
///
/// ADR-0032 clause 2. This is what a quantum, a latency contribution, and a
/// horizon are measured in. It is not a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct FrameCount(u64);

impl FrameCount {
    /// No frames.
    pub const ZERO: Self = Self(0);

    /// One render quantum.
    pub const QUANTUM: Self = Self(QUANTUM_FRAMES as u64);

    /// A count of frames. Every `u64` is a valid duration.
    pub const fn new(frames: u64) -> Self {
        Self(frames)
    }

    /// The raw count.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// The count as a `usize`, for indexing a buffer that was allocated from it.
    ///
    /// Returns `None` where the count does not fit the platform's index type,
    /// which keeps the conversion out of `as`-cast territory at a boundary where
    /// a truncation would silently shorten a buffer.
    pub const fn as_usize(self) -> Option<usize> {
        // The comparison is what makes the cast total; `usize::try_from` is not
        // available in a `const fn`.
        if self.0 <= usize::MAX as u64 {
            Some(self.0 as usize)
        } else {
            None
        }
    }

    /// The sum, or `None` on overflow.
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(sum) => Some(Self(sum)),
            None => None,
        }
    }

    /// How many whole quanta this many frames spans, rounded up.
    pub const fn quanta_spanned(self) -> u64 {
        self.0.div_ceil(QUANTUM_FRAMES as u64)
    }
}

impl std::fmt::Display for FrameCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} frames", self.0)
    }
}

/// The signed difference of two positions of the same kind.
///
/// ADR-0032 clause 3: this is the *only* way to subtract two [`SampleTime`]s or
/// two [`PlanPosition`]s, and subtracting across the two kinds is not defined at
/// all. Neither position type implements `Sub`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct FrameDelta(i64);

impl FrameDelta {
    /// No difference.
    pub const ZERO: Self = Self(0);

    /// A signed frame difference.
    pub const fn new(frames: i64) -> Self {
        Self(frames)
    }

    /// The raw difference.
    pub const fn as_i64(self) -> i64 {
        self.0
    }

    /// Whether the difference points backwards.
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }
}

impl std::fmt::Display for FrameDelta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:+} frames", self.0)
    }
}

/// The position within a quantum, `0 <= offset < Q`.
///
/// ADR-0032 clause 4. Construction from an out-of-range value **fails rather
/// than clamping**, and the width is `u16` rather than `u8` so that the type is
/// not sized to ADR-0037's provisional `Q`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct QuantumOffset(u16);

impl QuantumOffset {
    /// The first sample of a quantum.
    pub const ZERO: Self = Self(0);

    /// The largest offset the representation can hold.
    ///
    /// This is the bound the compile-time assertion at the top of this module
    /// pins `Q` against. It is not the largest *valid* offset, which is `Q - 1`.
    pub const MAX: u16 = u16::MAX;

    /// An offset inside the quantum, or [`TimeError::OffsetOutsideQuantum`].
    pub const fn new(offset: u16) -> Result<Self, TimeError> {
        if (offset as u32) < QUANTUM_FRAMES {
            Ok(Self(offset))
        } else {
            Err(TimeError::OffsetOutsideQuantum {
                offset,
                quantum: QUANTUM_FRAMES,
            })
        }
    }

    /// The raw offset.
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// The offset as a buffer index.
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// The engine's absolute time: input frames consumed since the stream was
/// prepared.
///
/// ADR-0032 clause 1 over ADR-0001 clause 11's epoch. It is advanced only by
/// rendering — by `Q` per rendered quantum and by nothing else — and no musical
/// operation moves it. It is never persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct SampleTime(u64);

impl SampleTime {
    /// The first input frame of the epoch.
    pub const ZERO: Self = Self(0);

    /// A position in the current epoch.
    pub const fn new(frames: u64) -> Self {
        Self(frames)
    }

    /// The raw position.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// This position plus a duration, or [`TimeError::ClockExhausted`].
    ///
    /// ADR-0032 clause 28: checked, never wrapping, and never panicking, because
    /// both this and the per-quantum advance run on the audio thread.
    pub const fn checked_add(self, frames: FrameCount) -> Result<Self, TimeError> {
        match self.0.checked_add(frames.as_u64()) {
            Some(sum) => Ok(Self(sum)),
            None => Err(TimeError::ClockExhausted {
                position: self.0,
                frames: frames.as_u64(),
            }),
        }
    }

    /// This position advanced by one quantum.
    pub const fn checked_advance_quantum(self) -> Result<Self, TimeError> {
        self.checked_add(FrameCount::QUANTUM)
    }

    /// The signed difference `self - earlier`, per ADR-0032 clause 3.
    ///
    /// Fails rather than wrapping where the difference does not fit, which is the only
    /// way a `u64` pair can be subtracted without a sign flip at the extremes.
    pub const fn difference(self, earlier: Self) -> Result<FrameDelta, TimeError> {
        signed_difference(self.0, earlier.0)
    }

    /// The index of the quantum containing this position.
    ///
    /// Derived, never stored (ADR-0032 clause 16).
    pub const fn quantum_index(self) -> u64 {
        self.0 / QUANTUM_FRAMES as u64
    }

    /// The offset of this position inside its quantum.
    pub const fn quantum_offset(self) -> QuantumOffset {
        // A remainder modulo `Q` is below `Q`, and the compile-time assertion at the
        // top of this module keeps `Q` inside `u16`, so the cast cannot truncate.
        QuantumOffset((self.0 % QUANTUM_FRAMES as u64) as u16)
    }

    /// The first position of the quantum containing this one.
    pub const fn quantum_start(self) -> Self {
        Self(self.quantum_index() * QUANTUM_FRAMES as u64)
    }
}

impl std::fmt::Display for SampleTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sample {}", self.0)
    }
}

/// Musical time resolved to frames, measured from plan sample zero.
///
/// ADR-0032 clause 26. It says *where in the piece* an event is, not *when the
/// renderer reaches it*, so it survives seek, loop wrap, and re-preparation —
/// and it is valid only against the tempo map that produced it, which is why it
/// is never persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct PlanPosition(u64);

impl PlanPosition {
    /// Plan sample zero.
    pub const ZERO: Self = Self(0);

    /// A position in the plan.
    pub const fn new(frames: u64) -> Self {
        Self(frames)
    }

    /// The raw position.
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// This position plus a duration, or [`TimeError::PlanPositionExhausted`].
    pub const fn checked_add(self, frames: FrameCount) -> Result<Self, TimeError> {
        match self.0.checked_add(frames.as_u64()) {
            Some(sum) => Ok(Self(sum)),
            None => Err(TimeError::PlanPositionExhausted {
                position: self.0,
                frames: frames.as_u64(),
            }),
        }
    }

    /// The signed difference `self - earlier`, per ADR-0032 clause 3.
    ///
    /// Fails rather than wrapping, for the reason [`SampleTime::difference`] gives.
    pub const fn difference(self, earlier: Self) -> Result<FrameDelta, TimeError> {
        signed_difference(self.0, earlier.0)
    }
}

impl std::fmt::Display for PlanPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "plan sample {}", self.0)
    }
}

/// `later - earlier` as a signed difference, or the fault of ADR-0032 clause 3.
///
/// Shared by both position types and by neither's `Sub`, because there is no `Sub`: the
/// operator would have to pick between wrapping and panicking, and the contract wants
/// neither.
const fn signed_difference(later: u64, earlier: u64) -> Result<FrameDelta, TimeError> {
    if later >= earlier {
        let forward = later - earlier;
        if forward > i64::MAX as u64 {
            return Err(TimeError::DifferenceUnrepresentable { later, earlier });
        }
        Ok(FrameDelta::new(forward as i64))
    } else {
        let backward = earlier - later;
        // `i64::MIN`'s magnitude is one larger than `i64::MAX`'s, so the boundary is not
        // symmetric and comparing against `MAX` alone would refuse one valid value.
        if backward > i64::MAX as u64 + 1 {
            return Err(TimeError::DifferenceUnrepresentable { later, earlier });
        }
        Ok(FrameDelta::new((backward as i64).wrapping_neg()))
    }
}

/// The pair that relates plan time to engine time.
///
/// ADR-0032 clause 27: anchoring is the **only** place the two vocabularies
/// meet, and in Phase 1 the only anchor is the one preparation establishes.
/// The session scheduler that re-anchors on play, seek, and loop wrap is
/// Phase 3's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct StreamAnchor {
    time: SampleTime,
    position: PlanPosition,
}

impl StreamAnchor {
    /// An anchor equating `time` with `position`.
    pub const fn new(time: SampleTime, position: PlanPosition) -> Self {
        Self { time, position }
    }

    /// The engine time this anchor is fixed at.
    pub const fn time(self) -> SampleTime {
        self.time
    }

    /// The plan position this anchor is fixed at.
    pub const fn position(self) -> PlanPosition {
        self.position
    }

    /// The engine time of `position`, or `None` where it is not representable in
    /// this stream.
    ///
    /// A position before the anchor is a scheduler error rather than a clamp
    /// (clause 27), so it has no time here; the loop wrap that would produce one
    /// re-anchors instead.
    pub const fn time_of(self, position: PlanPosition) -> Option<SampleTime> {
        let Ok(delta) = position.difference(self.position) else {
            // Unrepresentable is a fault, and this accessor's answer for "no time in this
            // stream" is the same either way: the caller re-anchors.
            return None;
        };
        if delta.is_negative() {
            return None;
        }
        // The negative case returned above, so the delta is non-negative.
        let forward = FrameCount::new(delta.as_i64() as u64);
        match self.time.checked_add(forward) {
            Ok(time) => Some(time),
            Err(_) => None,
        }
    }
}

/// A prepared stream's identity.
///
/// ADR-0032 clause 12: [`SampleTime`] restarts at zero in each epoch, so a time
/// without its epoch is ambiguous by construction. Identifiers are strictly
/// increasing and never reused within a process, which is stronger than
/// "different from the previous one" — that would permit `A -> B -> A` and let a
/// descheduled producer pass the staleness check with a timestamp from a dead
/// stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct StreamEpoch(u32);

impl StreamEpoch {
    /// The raw identifier.
    ///
    /// It means nothing outside the process that issued it: two processes both
    /// reach epoch 3, so a report publishing an `(epoch, time)` pair scopes it to
    /// the session that produced it.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Name an identifier this process did not issue.
    ///
    /// Two callers need it and neither is [`issue_epoch`]'s job: a protocol
    /// decoding an `(epoch, time)` pair it was published under ADR-0032 clause 7,
    /// and a test that has to name an epoch which is deliberately *not* the
    /// renderer's. It allocates nothing and confers nothing — issuing is what
    /// keeps identifiers increasing, and this only spells one out.
    pub const fn from_raw(id: u32) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for StreamEpoch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "epoch {}", self.0)
    }
}

/// The process's epoch counter. Starts at zero, only ever increments.
static NEXT_EPOCH: AtomicU32 = AtomicU32::new(0);

/// Issue the next [`StreamEpoch`], or fail because the space is exhausted.
///
/// Called once per preparation, off the audio thread. Exhaustion is unreachable
/// — 2^32 preparations in one process — and refusing rather than wrapping is what
/// keeps ADR-0032 clause 20's staleness check total.
pub fn issue_epoch() -> Result<StreamEpoch, TimeError> {
    NEXT_EPOCH
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map(StreamEpoch)
        .map_err(|_| TimeError::EpochSpaceExhausted)
}

/// Where a timestamp came from.
///
/// ADR-0032 clause 18. The distinction is what lets a report say how much a
/// timestamp is worth: a `Compiled` position is exact by construction, while an
/// `Arrival` one carries error the adapter itself declares unmeasured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeSource {
    /// From a driver or host, mapped through the epoch anchor.
    Hardware,
    /// The adapter had no timestamp, so the mapper stamped the event on arrival.
    Arrival,
    /// Generated from the plan and the timeline, where the timestamp is exact.
    Compiled,
}

impl TimeSource {
    /// Whether this provenance is external to the engine.
    ///
    /// ADR-0032 clause 21 binds the forward horizon to ingress provenance only:
    /// it exists to bound what an external producer can enqueue, and measuring
    /// the scheduler's own releases against it would reject most of a song.
    pub const fn is_ingress(self) -> bool {
        matches!(self, Self::Hardware | Self::Arrival)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantum_offset_refuses_a_value_at_or_beyond_the_quantum() {
        let last = u16::try_from(QUANTUM_FRAMES - 1).expect("Q fits u16 by the assertion above");
        assert!(QuantumOffset::new(last).is_ok());
        let at_q = u16::try_from(QUANTUM_FRAMES).expect("Q fits u16 by the assertion above");
        assert_eq!(
            QuantumOffset::new(at_q),
            Err(TimeError::OffsetOutsideQuantum {
                offset: at_q,
                quantum: QUANTUM_FRAMES,
            })
        );
        // The refusal is the point: a clamping constructor would place an event
        // at the wrong sample and type-check.
        assert!(QuantumOffset::new(u16::MAX).is_err());
    }

    #[test]
    fn sample_time_derives_its_quantum_index_and_offset() {
        let quantum = u64::from(QUANTUM_FRAMES);
        let time = SampleTime::new(quantum * 3 + 7);
        assert_eq!(time.quantum_index(), 3);
        assert_eq!(time.quantum_offset().as_u16(), 7);
        assert_eq!(time.quantum_start(), SampleTime::new(quantum * 3));
    }

    #[test]
    fn the_clock_refuses_to_wrap_rather_than_panicking() {
        let last = SampleTime::new(u64::MAX);
        assert_eq!(
            last.checked_advance_quantum(),
            Err(TimeError::ClockExhausted {
                position: u64::MAX,
                frames: u64::from(QUANTUM_FRAMES),
            })
        );
        assert!(SampleTime::ZERO.checked_advance_quantum().is_ok());
    }

    #[test]
    fn subtracting_positions_yields_a_signed_delta() {
        let early = SampleTime::new(10);
        let late = SampleTime::new(74);
        assert_eq!(late.difference(early), Ok(FrameDelta::new(64)));
        // The reversed order is where a `u64` subtraction would produce a
        // position eighteen quintillion frames in the future.
        assert_eq!(early.difference(late), Ok(FrameDelta::new(-64)));
        assert!(
            early
                .difference(late)
                .is_ok_and(|delta| delta.is_negative())
        );
    }

    #[test]
    fn plan_position_subtraction_is_also_signed() {
        let start = PlanPosition::new(48_000);
        let earlier = PlanPosition::new(0);
        assert_eq!(start.difference(earlier), Ok(FrameDelta::new(48_000)));
        assert_eq!(earlier.difference(start), Ok(FrameDelta::new(-48_000)));
    }

    #[test]
    fn a_difference_too_large_to_represent_is_a_fault_rather_than_a_sign_flip() {
        // The case the wrapping form got wrong: a position at the top of the range,
        // measured from zero, came back as one frame in the *past*.
        let far = SampleTime::new(u64::MAX);
        assert_eq!(
            far.difference(SampleTime::ZERO),
            Err(TimeError::DifferenceUnrepresentable {
                later: u64::MAX,
                earlier: 0,
            })
        );
        // The largest representable difference still works, in both directions.
        let edge = SampleTime::new(i64::MAX as u64);
        assert_eq!(
            edge.difference(SampleTime::ZERO),
            Ok(FrameDelta::new(i64::MAX))
        );
        assert_eq!(
            SampleTime::ZERO.difference(edge),
            Ok(FrameDelta::new(-i64::MAX))
        );
        // And `i64::MIN`'s asymmetric magnitude is representable rather than refused.
        let min_edge = SampleTime::new(i64::MAX as u64 + 1);
        assert_eq!(
            SampleTime::ZERO.difference(min_edge),
            Ok(FrameDelta::new(i64::MIN))
        );
    }

    #[test]
    fn plan_position_refuses_to_wrap() {
        assert_eq!(
            PlanPosition::new(u64::MAX).checked_add(FrameCount::QUANTUM),
            Err(TimeError::PlanPositionExhausted {
                position: u64::MAX,
                frames: u64::from(QUANTUM_FRAMES),
            })
        );
    }

    #[test]
    fn an_anchor_maps_plan_positions_forward_only() {
        let anchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::new(1_000));
        assert_eq!(
            anchor.time_of(PlanPosition::new(1_064)),
            Some(SampleTime::new(64))
        );
        assert_eq!(
            anchor.time_of(PlanPosition::new(1_000)),
            Some(SampleTime::ZERO)
        );
        // Before the anchor is a scheduler error, not a clamp to zero.
        assert_eq!(anchor.time_of(PlanPosition::new(999)), None);
    }

    #[test]
    fn epochs_strictly_increase_and_are_never_reused() {
        let first = issue_epoch().expect("epoch space is not exhausted in a test process");
        let second = issue_epoch().expect("epoch space is not exhausted in a test process");
        let third = issue_epoch().expect("epoch space is not exhausted in a test process");
        assert!(first < second && second < third);
    }

    #[test]
    fn ingress_provenance_is_what_the_forward_horizon_binds() {
        assert!(TimeSource::Hardware.is_ingress());
        assert!(TimeSource::Arrival.is_ingress());
        // A compiled list spans the whole piece; measuring it against a horizon
        // meant for live input would reject most of a song.
        assert!(!TimeSource::Compiled.is_ingress());
    }

    #[test]
    fn frame_counts_span_whole_quanta_rounding_up() {
        assert_eq!(FrameCount::ZERO.quanta_spanned(), 0);
        assert_eq!(FrameCount::new(1).quanta_spanned(), 1);
        assert_eq!(FrameCount::QUANTUM.quanta_spanned(), 1);
        assert_eq!(
            FrameCount::new(u64::from(QUANTUM_FRAMES) + 1).quanta_spanned(),
            2
        );
    }
}
