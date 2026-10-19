//! Parameter-only and node-linked first consumers for mixed load controls.
use super::{NodeId, ParameterId};
use crate::{
    quantities::{EventCount, ParameterValue},
    time::{FrameCount, QUANTUM_FRAMES},
};
use thiserror::Error;

mod hot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct RuntimeProgramId(u32);
impl RuntimeProgramId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProducerProgramError {
    #[error("period must be positive and every phase must lie inside it")]
    Phase,
    #[error("a runtime parameter program must have entries")]
    Empty,
    #[error("duplicate parameter target at one phase")]
    Duplicate,
    #[error("the producer bound cannot be represented")]
    Bound,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct RuntimeParameterEntry {
    node: NodeId,
    parameter: ParameterId,
    phase: FrameCount,
    value: ParameterValue,
}
impl RuntimeParameterEntry {
    pub const fn node(self) -> NodeId {
        self.node
    }
    pub const fn parameter(self) -> ParameterId {
        self.parameter
    }

    pub const fn new(
        node: NodeId,
        parameter: ParameterId,
        phase: FrameCount,
        value: ParameterValue,
    ) -> Self {
        Self {
            node,
            parameter,
            phase,
            value,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct RuntimeParameterProgram {
    id: RuntimeProgramId,
    period: FrameCount,
    entries: Box<[RuntimeParameterEntry]>,
    bound: EventCount,
}
impl RuntimeParameterProgram {
    pub const fn id(&self) -> RuntimeProgramId {
        self.id
    }

    pub fn new(
        id: RuntimeProgramId,
        period: FrameCount,
        mut entries: Vec<RuntimeParameterEntry>,
    ) -> Result<Self, ProducerProgramError> {
        if period.as_u64() == 0
            || entries
                .iter()
                .any(|entry| entry.phase.as_u64() >= period.as_u64())
        {
            return Err(ProducerProgramError::Phase);
        }
        if entries.is_empty() {
            return Err(ProducerProgramError::Empty);
        }
        entries.sort_by_key(|entry| (entry.phase.as_u64(), entry.node, entry.parameter));
        if entries.windows(2).any(|pair| {
            pair[0].phase == pair[1].phase
                && pair[0].node == pair[1].node
                && pair[0].parameter == pair[1].parameter
        }) {
            return Err(ProducerProgramError::Duplicate);
        }
        let targets = entries
            .iter()
            .enumerate()
            .filter(|(i, entry)| {
                !entries[..*i]
                    .iter()
                    .any(|old| old.node == entry.node && old.parameter == entry.parameter)
            })
            .count();
        let repeats = u64::from(QUANTUM_FRAMES).div_ceil(period.as_u64());
        let bound = u64::try_from(entries.len())
            .ok()
            .and_then(|length| length.checked_mul(repeats))
            .and_then(|events| events.checked_add(u64::try_from(targets).ok()?))
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(ProducerProgramError::Bound)?;
        Ok(Self {
            id,
            period,
            entries: entries.into_boxed_slice(),
            bound: EventCount::measured(bound),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct InternalProducerDeclaration {
    node: NodeId,
    period: FrameCount,
    phase: FrameCount,
}
impl InternalProducerDeclaration {
    pub const fn node(self) -> NodeId {
        self.node
    }

    pub fn periodic_delay_clear(
        node: NodeId,
        period: FrameCount,
        phase: FrameCount,
    ) -> Result<Self, ProducerProgramError> {
        if period.as_u64() == 0 || phase.as_u64() >= period.as_u64() {
            return Err(ProducerProgramError::Phase);
        }
        Ok(Self {
            node,
            period,
            phase,
        })
    }
    pub fn per_quantum(self) -> EventCount {
        // Q is a u32 and a positive period makes this quotient at most Q.
        EventCount::measured(
            u32::try_from(u64::from(QUANTUM_FRAMES).div_ceil(self.period.as_u64()))
                .unwrap_or(QUANTUM_FRAMES),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_periods_and_duplicate_targets_refuse_and_bounds_include_initialization() {
        let entry = RuntimeParameterEntry::new(
            NodeId::new(1),
            ParameterId::FIRST,
            FrameCount::new(0),
            ParameterValue::ONE,
        );
        let program = |period, entries| {
            RuntimeParameterProgram::new(RuntimeProgramId::new(1), FrameCount::new(period), entries)
        };
        assert!(matches!(
            program(0, vec![entry]),
            Err(ProducerProgramError::Phase)
        ));
        assert!(matches!(
            program(1, vec![entry, entry]),
            Err(ProducerProgramError::Duplicate)
        ));
        assert!(matches!(
            program(1, vec![]),
            Err(ProducerProgramError::Empty)
        ));
        assert!(
            InternalProducerDeclaration::periodic_delay_clear(
                NodeId::new(1),
                FrameCount::new(0),
                FrameCount::new(0)
            )
            .is_err()
        );
        assert!(
            InternalProducerDeclaration::periodic_delay_clear(
                NodeId::new(1),
                FrameCount::new(2),
                FrameCount::new(2)
            )
            .is_err()
        );
        assert_eq!(
            program(1, vec![entry]).unwrap().destination_occupancy(),
            EventCount::measured(65)
        );
        assert_eq!(
            InternalProducerDeclaration::periodic_delay_clear(
                NodeId::new(1),
                FrameCount::new(22),
                FrameCount::new(21)
            )
            .unwrap()
            .per_quantum(),
            EventCount::measured(3)
        );
    }
}
