//! The tempo map: musical time to plan position, and nothing else.
//!
//! ADR-0032 clause 27 makes anchoring the only place plan time and engine time meet, so
//! this module has **no** `SampleTime` in its API. It converts a musical position into a
//! [`PlanPosition`], and the session scheduler's anchor is what turns that into an engine
//! time — the master plan's "the tempo map produces plan positions; it never produces
//! engine times", enforced by the type rather than by convention.
//!
//! # A ramp interpolates the beat's length, not the tempo number
//!
//! `SOUND-INV-019`, from [ADR-0049](../../../plans/v2/decisions/ADR-0049-tempo-ramp-law.md).
//! V1 ramps tempo linearly in tick space, which makes elapsed time the integral of
//! `60/bpm` over a linear `bpm` — a logarithm. **ADR-0032 clause 15 forbids that**: the
//! conversion law "must be expressible in operations whose results are identical on every
//! supported target — the four IEEE-754 arithmetic operations, comparison, and rounding",
//! because "a tempo ramp implemented through a transcendental function would make the
//! frame a note lands on depend on the platform's libm, which the determinism digest
//! cannot tolerate".
//!
//! A first draft of this module ported V1's `ln_1p` formula faithfully and was wrong for
//! exactly that reason. ADR-0049 takes the other way clause 15 leaves open: the **period**
//! — seconds per beat — is what moves linearly, so elapsed time is a quadratic the four
//! operations express exactly. The cost is delivered timing, and it is real rather than
//! nominal: the reference corpus's `tempo-map-arrangement` ramps 90 to 180 BPM over two
//! beats, which lasts 1.000 s here against V1's 0.924 s, and everything after it moves by
//! that difference. Phase 4's A/B owns that as an intentional difference with its own
//! comparison category, the way ADR-0042's envelope shape is.
//!
//! **A ramp shares the step's linear term rather than recomputing it**, which is what makes
//! an equal-endpoint ramp bit-identical to a step and removes V1's `1e-5` near-flat branch
//! instead of reproducing it. Sharing is the property; the particular order is not. Writing
//! the term as `period * beats` instead rounds differently in about a third of argument
//! pairs, but four million random musical `(tempo, tick)` pairs produced no case where the
//! two orders reached different frames, so this module does not claim the order is
//! load-bearing.
//!
//! # One rounding, and where the error lives
//!
//! Clause 15 also says musical time is "rounded to a frame exactly once", using
//! round-half-away-from-zero. [`TempoMap::position_of`] therefore accumulates **seconds**
//! in `f64` — the stored per-segment prefix plus the offset inside the segment — and
//! rounds that sum once. It does not round each boundary and add integers, which would
//! accumulate up to half a frame per tempo change.
//!
//! What does accumulate is `f64` addition over the prefix, and the honest bound is small
//! rather than absent: `f64` carries about 16 significant digits, so a ten-minute plan's
//! 600-second prefix accrues on the order of `1e-12` seconds over any plausible number of
//! changes, which is around `5e-8` frames. The rounding decision it could flip is the one
//! where the exact position already sits within `5e-8` of a half-frame.

use thiserror::Error;

use crate::quantities::SampleRate;
use crate::time::PlanPosition;

/// Ticks in one quarter note.
///
/// V1's value, and matching it is the point: Phase 4 lowers existing projects through this
/// map, and a different resolution would silently move every note it converted.
pub const TICKS_PER_QUARTER: u32 = 960;

/// The largest tick and the largest plan position this map will convert.
///
/// `2^53` is where `f64` stops representing consecutive integers, so beyond it two distinct
/// ticks convert to one position and two distinct positions become one number. The map
/// refuses rather than answering: a silent collapse of sample identity is the one failure a
/// timing law must not have. At 48 kHz and 960 ticks per quarter this bound is thousands of
/// years in either unit, so nothing musical reaches it.
const EXACT_INTEGER_LIMIT: u64 = 1 << 53;

/// A musical position, in ticks from the start of the plan.
///
/// **Not `synth_sequencer::Tick`.** V2 owns its own quantities, and ADR-0032 clause 5
/// refused reusing a name whose other meaning is one import away. Not a [`PlanPosition`]
/// either: that is frames, this is musical time, and the whole purpose of this module is
/// that converting between them takes a tempo map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct MusicalTick(u64);

impl MusicalTick {
    /// The start of the plan.
    pub const ZERO: Self = Self(0);

    /// A position, in ticks.
    pub const fn new(ticks: u64) -> Self {
        Self(ticks)
    }

    /// The raw tick count.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for MusicalTick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tick {}", self.0)
    }
}

/// A tempo in beats per minute.
///
/// V2's own, for the reason `SampleRate` is: V1's `Bpm` is not this crate's to constrain,
/// and a clamping constructor would let an invalid persisted tempo reach the map as a
/// different, valid one.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct Bpm(f64);

impl Bpm {
    /// A tempo. Refuses anything that is not finite, not above zero, or whose **period**
    /// is not finite.
    ///
    /// Zero and negative are refused rather than clamped: both make the ramp integral
    /// meaningless, and a song that asked for one is a song whose author has to be told.
    ///
    /// **The period is checked here rather than where it is used**, and the reason is the
    /// defect an independent review found. A tempo below `60 / f64::MAX` — around
    /// `3.34e-307`, one beat per ten thousand times the age of the universe — has a period
    /// that overflows to infinity. ADR-0049's ramp then evaluates `infinity * 0` at the
    /// segment's own start and produces `NaN`, which reaches both a position and a
    /// [`Bpm`] built by [`TempoMap::tempo_at`] without passing through this constructor.
    /// Refusing the tempo is what keeps that construction sound by argument instead of by
    /// a check at every later site — which is where this repository puts a newtype's
    /// invariant.
    pub fn new(bpm: f64) -> Result<Self, TempoError> {
        if !bpm.is_finite() || bpm <= 0.0 || !(60.0 / bpm).is_finite() {
            return Err(TempoError::TempoOutsideDomain { bpm });
        }
        Ok(Self(bpm))
    }

    /// The raw tempo.
    pub const fn as_f64(self) -> f64 {
        self.0
    }
}

impl std::fmt::Display for Bpm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} BPM", self.0)
    }
}

/// One tempo change on the plan's timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct TempoChange {
    tick: MusicalTick,
    bpm: Bpm,
    ramp: bool,
}

impl TempoChange {
    /// A change that steps to `bpm` at `tick` and holds until the next change.
    pub const fn step(tick: MusicalTick, bpm: Bpm) -> Self {
        Self {
            tick,
            bpm,
            ramp: false,
        }
    }

    /// A change that starts at `bpm` and ramps toward the **next** change's tempo.
    ///
    /// ADR-0049 clause 4, and V1's declaration kept unchanged so that Phase 4's lowering
    /// translates nothing: the ramp reaches the next change's tempo exactly at that
    /// change, and a ramp with no change after it is a step at `bpm` rather than a ramp
    /// toward an invented destination.
    pub const fn ramp(tick: MusicalTick, bpm: Bpm) -> Self {
        Self {
            tick,
            bpm,
            ramp: true,
        }
    }

    /// Where the change takes effect.
    pub const fn tick(self) -> MusicalTick {
        self.tick
    }

    /// The tempo it establishes.
    pub const fn bpm(self) -> Bpm {
        self.bpm
    }

    /// Whether it ramps toward the next change rather than holding.
    pub const fn is_ramp(self) -> bool {
        self.ramp
    }
}

/// Why a tempo map could not be built, or could not answer.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum TempoError {
    /// A tempo that is not a finite positive number, or whose period is not finite.
    #[error("tempo {bpm} is not a finite tempo above zero whose period is also finite")]
    TempoOutsideDomain {
        /// The value as given.
        bpm: f64,
    },

    /// Changes were not in ascending tick order, or repeated a tick.
    ///
    /// Refused rather than sorted: two changes at one tick have no defined order, and
    /// sorting here would pick one silently.
    #[error("tempo change {index} at {tick} does not follow {previous}")]
    ChangesOutOfOrder {
        /// Position in the supplied list.
        index: usize,
        /// The change's tick.
        tick: MusicalTick,
        /// The tick it had to follow.
        previous: MusicalTick,
    },

    /// A tick or the position it maps to is past exact `f64` integer range.
    ///
    /// Refused rather than answered approximately: past `2^53` two distinct ticks convert
    /// to one position, and a timing law that silently merges two samples is worse than one
    /// that says it cannot.
    #[error("tick {tick} or its position is past the exact conversion range")]
    PositionNotExactlyRepresentable {
        /// The position asked for.
        tick: MusicalTick,
    },
}

/// One segment's start, so a query adds a stored prefix rather than re-walking the map.
#[derive(Debug, Clone, Copy)]
struct Segment {
    start_tick: u64,
    bpm: f64,
    /// Seconds from the plan's start to this segment's start. Computed once, in `f64`, and
    /// never rounded — clause 15 rounds once, at the query.
    start_seconds: f64,
    /// Where a ramp is heading, when this segment ramps.
    ///
    /// `None` is a step, and it is also what a declared ramp becomes when no change
    /// follows it: ADR-0049 clause 4 refuses to invent a destination tempo, so the
    /// distinction disappears here rather than being carried and re-decided at every
    /// query.
    ramp: Option<RampEnd>,
}

/// The far end of a ramp segment.
#[derive(Debug, Clone, Copy)]
struct RampEnd {
    end_tick: u64,
    end_bpm: f64,
}

/// A plan's tempo over musical time.
#[derive(Debug, Clone)]
#[must_use]
pub struct TempoMap {
    segments: Vec<Segment>,
    sample_rate: SampleRate,
}

impl TempoMap {
    /// Build a map from an initial tempo and the changes that follow it.
    ///
    /// The initial tempo governs from tick zero. A change at tick zero replaces it rather
    /// than following it, so a song whose first change is at the start has one segment and
    /// no empty leading one.
    pub fn new(
        initial: Bpm,
        changes: &[TempoChange],
        sample_rate: SampleRate,
    ) -> Result<Self, TempoError> {
        let mut previous: Option<MusicalTick> = None;
        for (index, change) in changes.iter().enumerate() {
            if let Some(previous_tick) = previous
                && change.tick() <= previous_tick
            {
                return Err(TempoError::ChangesOutOfOrder {
                    index,
                    tick: change.tick(),
                    previous: previous_tick,
                });
            }
            if change.tick().as_u64() > EXACT_INTEGER_LIMIT {
                return Err(TempoError::PositionNotExactlyRepresentable {
                    tick: change.tick(),
                });
            }
            previous = Some(change.tick());
        }

        let mut points: Vec<TempoChange> = Vec::with_capacity(changes.len() + 1);
        if changes.first().map(|change| change.tick()) != Some(MusicalTick::ZERO) {
            points.push(TempoChange::step(MusicalTick::ZERO, initial));
        }
        points.extend_from_slice(changes);

        let mut segments: Vec<Segment> = Vec::with_capacity(points.len());
        let mut seconds = 0.0_f64;
        for (index, point) in points.iter().enumerate() {
            let next = points.get(index + 1);
            // ADR-0049 clause 4. A ramp needs a change after it to have a destination at
            // all, so the last point is a step whatever it declared.
            let ramp = match (point.is_ramp(), next) {
                (true, Some(next)) => Some(RampEnd {
                    end_tick: next.tick().as_u64(),
                    end_bpm: next.bpm().as_f64(),
                }),
                _ => None,
            };
            let segment = Segment {
                start_tick: point.tick().as_u64(),
                bpm: point.bpm().as_f64(),
                start_seconds: seconds,
                ramp,
            };
            segments.push(segment);
            if let Some(next) = next {
                // Clause 2: the prefix is this same expression evaluated at the segment's
                // own end, never a separately derived trapezoid, so a boundary cannot
                // disagree with a query just inside the segment.
                seconds += segment_seconds(&segment, next.tick().as_u64());
            }
        }

        Ok(Self {
            segments,
            sample_rate,
        })
    }

    /// The plan position of a musical tick.
    ///
    /// Clause 15's single rounding: the stored prefix and the offset inside the segment are
    /// summed in seconds and rounded once, half away from zero. Rounding each boundary and
    /// adding integers would instead accrue up to half a frame per tempo change.
    pub fn position_of(&self, tick: MusicalTick) -> Result<PlanPosition, TempoError> {
        if tick.as_u64() > EXACT_INTEGER_LIMIT {
            return Err(TempoError::PositionNotExactlyRepresentable { tick });
        }
        let segment = self.segment_for(tick);
        let seconds = segment.start_seconds + segment_seconds(&segment, tick.as_u64());
        let frames = seconds * f64::from(self.sample_rate.as_f32());

        // Bounded by the same `2^53` the tick is, and for the same reason: past it a
        // position is no longer one identifiable frame.
        #[allow(
            clippy::cast_precision_loss,
            reason = "the limit is a power of two and exact in f64"
        )]
        let limit = EXACT_INTEGER_LIMIT as f64;
        if !frames.is_finite() || frames < 0.0 || frames > limit {
            return Err(TempoError::PositionNotExactlyRepresentable { tick });
        }

        // `round` is round-half-away-from-zero, which clause 15 names. Truncation would
        // bias every position early, and a listener hears a uniform early bias as a drag.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the finite, non-negative, in-range value was just checked"
        )]
        Ok(PlanPosition::new(frames.round() as u64))
    }

    /// The tempo in force at a tick, for a report or a display.
    ///
    /// ADR-0049 clause 7: inside a ramp this is the **reciprocal of the interpolated
    /// period**, not a straight line between the two declared tempi. What moves linearly
    /// is the beat's length, so the tempo curve is a hyperbola, and a display that drew a
    /// ramp as a straight line between two BPM values would be drawing something the
    /// engine does not play.
    ///
    /// **The reciprocal is a valid tempo by construction, and it takes two things to make
    /// that argument true.** [`Bpm::new`]'s period check gives both endpoint periods as
    /// finite and positive — before it, a ramp toward a tempo below `60 / f64::MAX`
    /// produced an infinite period and a `NaN` tempo here, bypassing the constructor
    /// entirely, which an independent review found. And the interpolation below forms its
    /// fraction first, so the intermediate cannot overflow out of that range; the earlier
    /// order reached infinity and reported a tempo of zero, which this module's own check
    /// found.
    pub fn tempo_at(&self, tick: MusicalTick) -> Bpm {
        let segment = self.segment_for(tick);
        let Some(end) = segment.ramp else {
            return Bpm(segment.bpm);
        };
        let beats = beats_from(segment.start_tick, tick.as_u64());
        let period = 60.0 / segment.bpm;
        let end_period = 60.0 / end.end_bpm;
        let full = beats_from(segment.start_tick, end.end_tick);
        // **A weighted combination, then clamped to the interval its endpoints define**, and
        // every simpler form produced a `Bpm` this type's own constructor refuses. Three
        // were tried: `p0 + (p1 - p0) * beats / full` overflows to infinity for a ramp
        // toward a very slow tempo and reports zero; forming the fraction first leaves a
        // subtraction that cancels to exactly zero for a 6000-to-1e100 BPM ramp at
        // `fraction == 1.0` and reports infinity; and the weighted form alone can round one
        // unit in the last place below the endpoint period, whose reciprocal then overflows
        // for a tempo near `f64::MAX`. An independent review found each in turn.
        //
        // The clamp is not a repair applied to a value that might be wrong — it enforces
        // where the interpolated period mathematically **is**. `p(beta)` is a convex
        // combination of `p0` and `p1`, so it lies between them for every `beta` in the
        // segment; rounding is the only thing that can put it outside, and the reciprocal of
        // a value inside is a tempo by construction, bounded by the two the caller declared.
        let fraction = beats / full;
        let weighted = period * (1.0 - fraction) + end_period * fraction;
        let low = period.min(end_period);
        let high = period.max(end_period);
        Bpm(60.0 / weighted.clamp(low, high))
    }

    /// How many segments the map holds, for a test that has to know the shape it built.
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// The segment governing `tick`.
    ///
    /// Reverse linear: maps are short, and the same scan answers `tempo_at` and
    /// `position_of` so the two cannot disagree about which segment a tick is in.
    fn segment_for(&self, tick: MusicalTick) -> Segment {
        let ticks = tick.as_u64();
        for segment in self.segments.iter().rev() {
            if ticks >= segment.start_tick {
                return *segment;
            }
        }
        // The first segment starts at tick zero, so no tick precedes it. Returning it keeps
        // the function total rather than relying on that argument at an index.
        self.segments.first().copied().unwrap_or(Segment {
            start_tick: 0,
            bpm: 120.0,
            start_seconds: 0.0,
            ramp: None,
        })
    }
}

/// Seconds from a segment's start to `tick`.
///
/// `SOUND-INV-019`. Four operations and nothing else, which is what clause 15 requires of
/// the conversion law. A step contributes `beats * 60 / bpm`; a ramp adds ADR-0049
/// clause 1's quadratic term, which is the integral of a period moving linearly across the
/// segment.
///
/// **Clause 3's bit-identity comes from `linear` being one value rather than two.** Both
/// laws need it, and computing it once is what makes an equal-endpoint ramp — whose
/// quadratic term is then a signed zero — produce the step's own answer bit for bit. A
/// second copy of the expression could disagree with the first by a rounding; the sharing,
/// not any particular order, is the property under test.
///
/// **The rounded conversion is not guaranteed monotone, and ADR-0049 clause 6 says so rather
/// than claiming otherwise.** The step law is non-decreasing by composition — every operation
/// in it is monotone in its argument — and an accelerating ramp's is not: its exact function
/// is a positive linear term minus a positive quadratic one, so adjacent ticks can convert to
/// decreasing positions once the position's own rounding exceeds the per-tick increment.
///
/// A **monotone** rewriting does exist, and it is not adopted for a measured reason rather
/// than an unexamined one. Writing the rising case relative to the segment's end,
/// `S(B) - [p1 * u + (p0 - p1) * u * u / (2B)]` with `u = B - beats`, makes the bracket
/// non-increasing and the whole expression non-decreasing by composition. It also subtracts
/// two large quantities near `beats = 0`, where the accepted form is exact: over random
/// ramps that cancellation put the segment's own start as much as 128 seconds *before* its
/// prefix, which is a backwards step of its own and a far larger error than the one it
/// removes. The record carries both halves of that trade.
///
/// Inversions are constructible, and the record deliberately puts **no figure** on where
/// they start: every bound written here was refuted by a better search, and a threshold that
/// keeps moving is not a threshold. What is true and stable is the shape of the domain — a
/// position decades of audio out, together with a tempo ratio in the thousands. Nothing
/// musical approaches either.
///
/// `full` is positive by construction: [`TempoMap::new`] refuses changes that do not ascend,
/// so a ramp segment spans at least one tick and the division has no zero case to guard.
/// `beats <= full` likewise holds by construction — a tick at or past a ramp's end belongs to
/// the next segment, and the stored prefix is this function evaluated at exactly that
/// boundary.
fn segment_seconds(segment: &Segment, tick: u64) -> f64 {
    let beats = beats_from(segment.start_tick, tick);
    let linear = beats * 60.0 / segment.bpm;
    let Some(end) = segment.ramp else {
        return linear;
    };
    let period = 60.0 / segment.bpm;
    let end_period = 60.0 / end.end_bpm;
    let full = beats_from(segment.start_tick, end.end_tick);
    linear + (end_period - period) * beats * beats / (2.0 * full)
}

/// Beats from `start_tick` to `tick`, never negative.
///
/// One expression rather than three: [`TempoMap::position_of`], [`TempoMap::tempo_at`] and
/// [`segment_seconds`] all ask the same question, and three copies of a conversion are
/// three chances for one of them to disagree about what a beat is.
fn beats_from(start_tick: u64, tick: u64) -> f64 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "the tick is bounded by EXACT_INTEGER_LIMIT before any conversion"
    )]
    let ticks = tick.saturating_sub(start_tick) as f64;
    ticks / f64::from(TICKS_PER_QUARTER)
}

#[cfg(test)]
#[path = "tests/tempo.rs"]
mod tests;
