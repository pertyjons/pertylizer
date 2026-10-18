//! The plan positions visible to one fixed render quantum.

use super::{FrameCount, PlanPosition, QUANTUM_FRAMES, QuantumOffset};

#[derive(Debug, Clone, Copy)]
enum Mapping<'a> {
    Linear(Option<PlanPosition>),
    Mapped(&'a [PlanPosition; QUANTUM_FRAMES as usize]),
    Held(Option<PlanPosition>),
}

/// A kernel's plan timeline, independent of the callback's output partition.
/// A mapped quantum can visit a position several times without splitting DSP work.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct QuantumTimeline<'a> {
    mapping: Mapping<'a>,
}

impl<'a> QuantumTimeline<'a> {
    /// A linear quantum, or no position where the stream anchor does not reach it.
    pub const fn linear(start: Option<PlanPosition>) -> Self {
        Self {
            mapping: Mapping::Linear(start),
        }
    }

    /// Explicit positions for every frame. This kernel input does not change an
    /// engine anchor, publish events, mint notes or enable transport looping.
    pub const fn mapped(positions: &'a [PlanPosition; QUANTUM_FRAMES as usize]) -> Self {
        Self {
            mapping: Mapping::Mapped(positions),
        }
    }

    /// A stopped transport's quantum: every frame stands at `position`, and none
    /// visits it. This kernel input does not change an engine anchor either.
    pub(crate) const fn held(position: Option<PlanPosition>) -> Self {
        Self {
            mapping: Mapping::Held(position),
        }
    }

    /// The position at a validated quantum offset, if representable.
    pub fn position_at(self, offset: QuantumOffset) -> Option<PlanPosition> {
        match self.mapping {
            Mapping::Linear(start) => start?
                .checked_add(FrameCount::new(u64::from(offset.as_u16())))
                .ok(),
            Mapping::Mapped(positions) => positions.get(usize::from(offset.as_u16())).copied(),
            Mapping::Held(position) => position,
        }
    }

    pub(crate) const fn mapped_positions(
        self,
    ) -> Option<&'a [PlanPosition; QUANTUM_FRAMES as usize]> {
        match self.mapping {
            Mapping::Linear(_) | Mapping::Held(_) => None,
            Mapping::Mapped(positions) => Some(positions),
        }
    }

    /// Whether the transport stands still for this quantum, so no frame plays the
    /// position it reports. A mapped quantum that repeats a position still plays it.
    pub(crate) const fn is_held(self) -> bool {
        matches!(self.mapping, Mapping::Held(_))
    }
}

#[cfg(test)]
mod tests;
