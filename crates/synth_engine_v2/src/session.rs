//! The session scheduler: where plan time and engine time are joined, and only there.
//!
//! ADR-0032 clause 27 makes anchoring the single place the two vocabularies meet. This
//! module owns the current [`StreamAnchor`] and the [`TempoMap`] behind it, so a caller
//! goes musical → plan → engine and never musical → engine. The composition is the point:
//! each step has one owner, and no step can be skipped by reaching for a shortcut that does
//! not exist.
//!
//! # Re-anchoring, and why a wrap is not a subtraction
//!
//! The master plan names four moments that re-anchor: play, seek, loop wrap, and offline
//! range start. Three of them are obvious. The fourth is the one clause 27 singles out —
//! `StreamAnchor::time_of` has no answer for a position *before* the anchor, and says so
//! rather than clamping, because "the loop wrap that would produce one re-anchors instead".
//!
//! That is a real distinction and not bookkeeping. A wrap moves the plan backwards while
//! engine time keeps going forward, so the old pairing is not merely stale, it is
//! **contradictory**: it would map the loop's second pass onto the first pass's frames. A
//! scheduler that subtracted a loop length from the position instead would keep answering,
//! and every answer after the first wrap would be wrong by exactly one loop.
//!
//! # Replacement is atomic, or it did not happen
//!
//! A tempo map fixes where every musical position lands, so replacing one moves the entire
//! remaining plan. [`SessionScheduler::replace_tempo_map`] therefore takes the recompiled
//! schedule with the new map and activates the pair together. If the caller cannot supply a
//! schedule that is admissible under the new map, **both** stay as they were — the master
//! plan's "failure leaves the old map and plan active". Activating one without the other
//! would leave events whose engine times were computed under a tempo that is no longer in
//! force.

use thiserror::Error;

use crate::tempo::{MusicalTick, TempoError, TempoMap};
use crate::time::{PlanPosition, SampleTime, StreamAnchor};

/// Why the session could not answer, or could not re-anchor.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum SessionError {
    /// A musical position that precedes the current anchor.
    ///
    /// Clause 27's case, reported rather than clamped. A caller that reaches this has moved
    /// the plan backwards without re-anchoring, and the fix is the re-anchor rather than an
    /// answer this one cannot give truthfully.
    #[error("{tick} is at {position} which precedes the anchor at {anchor}")]
    PositionPrecedesAnchor {
        /// The musical position asked for.
        tick: MusicalTick,
        /// Where the tempo map puts it.
        position: PlanPosition,
        /// The anchor's plan position.
        anchor: PlanPosition,
    },

    /// The engine time for a position is past what this stream can represent.
    #[error("{tick} maps past the end of this stream's engine time")]
    EngineTimeExhausted {
        /// The musical position asked for.
        tick: MusicalTick,
    },

    /// The tempo map could not place a position.
    #[error("tempo: {0}")]
    Tempo(#[from] TempoError),
}

/// The transport's current pairing of plan time and engine time.
#[derive(Debug, Clone)]
#[must_use]
pub struct SessionScheduler {
    tempo: TempoMap,
    anchor: StreamAnchor,
}

impl SessionScheduler {
    /// Start playing `tick` at engine time `at`.
    ///
    /// One of the four re-anchoring moments, and the one every stream begins with.
    pub fn play(tempo: TempoMap, tick: MusicalTick, at: SampleTime) -> Result<Self, SessionError> {
        let position = tempo.position_of(tick)?;
        Ok(Self {
            tempo,
            anchor: StreamAnchor::new(at, position),
        })
    }

    /// Begin an offline render at `tick`, treating `at` as its first frame.
    ///
    /// The same operation as [`Self::play`] and named separately because the master plan
    /// names it separately: an offline range start is a re-anchor, and calling it a play
    /// would hide that an offline job re-anchors at all.
    pub fn offline_range_start(
        tempo: TempoMap,
        tick: MusicalTick,
        at: SampleTime,
    ) -> Result<Self, SessionError> {
        Self::play(tempo, tick, at)
    }

    /// Move the transport to `tick`, which the stream reaches at engine time `at`.
    pub fn seek(&mut self, tick: MusicalTick, at: SampleTime) -> Result<(), SessionError> {
        let position = self.tempo.position_of(tick)?;
        self.anchor = StreamAnchor::new(at, position);
        Ok(())
    }

    /// Wrap the transport back to `tick` at engine time `at`.
    ///
    /// Clause 27's named case. It re-anchors rather than adjusting a position, because a
    /// wrap moves plan time backwards while engine time moves forward — after it, the old
    /// pairing does not merely lag, it maps the second pass onto the first pass's frames.
    pub fn loop_wrap(&mut self, tick: MusicalTick, at: SampleTime) -> Result<(), SessionError> {
        self.seek(tick, at)
    }

    /// The engine time a musical position falls on.
    ///
    /// The whole composition, and the only route: musical → plan through the tempo map,
    /// plan → engine through the anchor. There is no musical → engine shortcut to reach
    /// for.
    pub fn engine_time_of(&self, tick: MusicalTick) -> Result<SampleTime, SessionError> {
        let position = self.tempo.position_of(tick)?;
        self.anchor
            .time_of(position)
            .ok_or_else(|| self.explain(tick, position))
    }

    /// The plan position of a musical tick, without engine time.
    pub fn position_of(&self, tick: MusicalTick) -> Result<PlanPosition, SessionError> {
        Ok(self.tempo.position_of(tick)?)
    }

    /// The current anchor.
    pub const fn anchor(&self) -> StreamAnchor {
        self.anchor
    }

    /// The tempo map in force.
    pub const fn tempo(&self) -> &TempoMap {
        &self.tempo
    }

    /// Replace the tempo map, keeping the transport at `tick`.
    ///
    /// The caller supplies the position it wants `tick` to keep in engine time, because a
    /// new map moves every musical position: continuing to sound at the same instant means
    /// re-anchoring, and there is no reading of "keep going" that does not.
    ///
    /// **Atomic.** The new map is validated against the transport before anything changes,
    /// and a failure leaves the old map and the old anchor exactly as they were. A partial
    /// activation would leave events whose engine times were computed under a tempo no
    /// longer in force.
    pub fn replace_tempo_map(
        &mut self,
        tempo: TempoMap,
        tick: MusicalTick,
        at: SampleTime,
    ) -> Result<(), SessionError> {
        // Computed before the swap, so a map that cannot place the transport's own position
        // is refused while the old one is still the one in force.
        let position = tempo.position_of(tick)?;
        self.tempo = tempo;
        self.anchor = StreamAnchor::new(at, position);
        Ok(())
    }

    /// Which of the two reasons `time_of` returned nothing.
    ///
    /// The accessor answers `None` for both a position before the anchor and one past the
    /// stream's range, because a caller re-anchors either way. A diagnostic cannot be that
    /// terse: the two need different fixes.
    fn explain(&self, tick: MusicalTick, position: PlanPosition) -> SessionError {
        if position < self.anchor.position() {
            SessionError::PositionPrecedesAnchor {
                tick,
                position,
                anchor: self.anchor.position(),
            }
        } else {
            SessionError::EngineTimeExhausted { tick }
        }
    }
}

#[cfg(test)]
#[path = "tests/session.rs"]
mod tests;
