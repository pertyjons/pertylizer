//! Bounded allocation-free queries over an immutable admitted tempo map.
use super::{
    EXACT_INTEGER_LIMIT, MusicalTick, PlanPosition, Segment, TICKS_PER_QUARTER, TempoError,
    TempoMap,
};
impl TempoMap {
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

    /// The segment governing `tick`.
    ///
    /// Reverse linear: maps are short, and the same scan answers `tempo_at` and
    /// `position_of` so the two cannot disagree about which segment a tick is in.
    pub(super) fn segment_for(&self, tick: MusicalTick) -> Segment {
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
pub(super) fn segment_seconds(segment: &Segment, tick: u64) -> f64 {
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
pub(super) fn beats_from(start_tick: u64, tick: u64) -> f64 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "the tick is bounded by EXACT_INTEGER_LIMIT before any conversion"
    )]
    let ticks = tick.saturating_sub(start_tick) as f64;
    ticks / f64::from(TICKS_PER_QUARTER)
}
