//! The tempo map: musical time to plan position, and nothing else.
//!
//! ADR-0032 clause 27 makes anchoring the only place plan time and engine time meet, so
//! this module has **no** `SampleTime` in its API. It converts a musical position into a
//! [`PlanPosition`], and the session scheduler's anchor is what turns that into an engine
//! time — the master plan's "the tempo map produces plan positions; it never produces
//! engine times", enforced by the type rather than by convention.
//!
//! # Steps only, and a ramp is refused rather than approximated
//!
//! V1 ramps tempo linearly in tick space, which makes elapsed time the integral of
//! `60/bpm` over a linear `bpm` — a logarithm. **ADR-0032 clause 15 forbids that**: the
//! conversion law "must be expressible in operations whose results are identical on every
//! supported target — the four IEEE-754 arithmetic operations, comparison, and rounding",
//! because "a tempo ramp implemented through a transcendental function would make the
//! frame a note lands on depend on the platform's libm, which the determinism digest
//! cannot tolerate".
//!
//! A first draft of this module ported V1's `ln_1p` formula faithfully and was wrong for
//! exactly that reason. The choice clause 15 leaves — state the exact evaluation, or use a
//! ramp shape the four operations can express — is a durable one that changes either
//! delivered musical behaviour or the numeric law, so it is **deferred to its own record**
//! rather than settled here. Until then `TempoChange::ramp` does not exist and a map
//! cannot be built with one.
//!
//! Steps are what the corpus actually uses: no project in this repository sets a ramp.
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
    /// A tempo. Refuses anything that is not finite and above zero.
    ///
    /// Zero and negative are refused rather than clamped: both make the ramp integral
    /// meaningless, and a song that asked for one is a song whose author has to be told.
    pub fn new(bpm: f64) -> Result<Self, TempoError> {
        if !bpm.is_finite() || bpm <= 0.0 {
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
}

impl TempoChange {
    /// A change that steps to `bpm` at `tick` and holds until the next change.
    ///
    /// There is no ramp constructor. The module header says why: clause 15 forbids the
    /// transcendental V1's ramp shape needs, and choosing between an exactly stated
    /// evaluation and a different shape is a durable decision this module does not make.
    pub const fn step(tick: MusicalTick, bpm: Bpm) -> Self {
        Self { tick, bpm }
    }

    /// Where the change takes effect.
    pub const fn tick(self) -> MusicalTick {
        self.tick
    }

    /// The tempo it establishes.
    pub const fn bpm(self) -> Bpm {
        self.bpm
    }
}

/// Why a tempo map could not be built, or could not answer.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum TempoError {
    /// A tempo that is not a finite positive number.
    #[error("tempo {bpm} is not a finite tempo above zero")]
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
            segments.push(Segment {
                start_tick: point.tick().as_u64(),
                bpm: point.bpm().as_f64(),
                start_seconds: seconds,
            });
            if let Some(next) = points.get(index + 1) {
                seconds += segment_seconds(
                    point.tick().as_u64(),
                    point.bpm().as_f64(),
                    next.tick().as_u64(),
                );
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
        let seconds =
            segment.start_seconds + segment_seconds(segment.start_tick, segment.bpm, tick.as_u64());
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
    pub fn tempo_at(&self, tick: MusicalTick) -> Bpm {
        Bpm(self.segment_for(tick).bpm)
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
        })
    }
}

/// Seconds from a segment's start to `tick`, at a constant tempo.
///
/// Four operations and nothing else, which is what clause 15 requires of the conversion law.
fn segment_seconds(start_tick: u64, bpm: f64, tick: u64) -> f64 {
    let beats = tick.saturating_sub(start_tick) as f64 / f64::from(TICKS_PER_QUARTER);
    beats * 60.0 / bpm
}

#[cfg(test)]
#[path = "tests/tempo.rs"]
mod tests;
