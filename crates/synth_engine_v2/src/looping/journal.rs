//! Finite retained render observations, independent of source fences or take sealing.

mod hot;

use super::{CompiledLoopStream, LoopBoundary, LoopFault, LoopPassId, LoopSnapshot};
use crate::{
    quantities::{LoopPassCount, PreparedBytes},
    time::{SampleTime, StreamEpoch},
    transport::LoopInterval,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LoopJournalPrepareError {
    #[error("a loop journal must admit at least its initial pass")]
    Empty,
    #[error("loop journal storage is not representable")]
    Layout,
    #[error("loop journal needs {required:?}, above the declared {available:?}")]
    Budget {
        required: PreparedBytes,
        available: PreparedBytes,
    },
    #[error("loop journal storage could not be reserved")]
    Allocation,
    #[error("a faulted loop cannot start a journal: {0}")]
    Faulted(LoopFault),
}

/// The first end of observation, which does not itself stop compiled playback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopJournalEndReason {
    Finished,
    PassLimit,
    RenderFault(LoopFault),
}

/// Exclusive observation endpoint in rendered engine time, never hardware time.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct LoopJournalEnd {
    pub epoch: StreamEpoch,
    pub at: SampleTime,
    pub pass: LoopPassId,
    pub reason: LoopJournalEndReason,
}

/// Owns all loop mutation and retains a finite append-only boundary journal.
/// Reading cannot renew pass entitlement. This is not a recorded take, a source
/// fence, or a concurrent queue; its owner must be reclaimed off-thread.
#[must_use]
pub struct JournaledLoopStream {
    stream: CompiledLoopStream,
    initial: LoopSnapshot,
    acknowledged: LoopSnapshot,
    boundaries: Box<[Option<LoopBoundary>]>,
    boundary_len: usize,
    end: Option<LoopJournalEnd>,
    storage_bytes: PreparedBytes,
}

impl JournaledLoopStream {
    #[cfg(feature = "simulated-ingress")]
    pub(crate) fn is_fresh(&self) -> bool {
        self.initial.clock == SampleTime::ZERO
            && self.acknowledged == self.initial
            && self.end.is_none()
            && !self.stream.source.started
            && self.stream.renderer.carry_frames() == crate::time::QUANTUM_FRAMES as usize
    }

    #[cfg(feature = "simulated-ingress")]
    pub(crate) const fn session_share(&self) -> crate::quantities::EventCount {
        self.stream.arbiter.session_share()
    }

    /// Take exclusive ownership off-thread, including a loop that has already
    /// rendered. The initial position may equal the end, with a wrap still pending.
    /// P passes reserve P-1 interior boundary cells and an inline terminal cell.
    /// The extra byte ceiling charges retained heap only; loop preparation has
    /// its own budget. No capture-session or capture-pass identity is issued here.
    pub fn prepare(
        stream: CompiledLoopStream,
        passes: LoopPassCount,
        budget: PreparedBytes,
    ) -> Result<Self, LoopJournalPrepareError> {
        if let Some(fault) = stream.fault() {
            return Err(LoopJournalPrepareError::Faulted(fault));
        }
        if passes.get() == 0 {
            return Err(LoopJournalPrepareError::Empty);
        }
        let cells = passes
            .as_usize()
            .and_then(|count| count.checked_sub(1))
            .ok_or(LoopJournalPrepareError::Layout)?;
        let bytes = cells
            .checked_mul(size_of::<Option<LoopBoundary>>())
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(LoopJournalPrepareError::Layout)?;
        let storage_bytes = PreparedBytes::measured(bytes);
        if storage_bytes > budget {
            return Err(LoopJournalPrepareError::Budget {
                required: storage_bytes,
                available: budget,
            });
        }
        let mut boundaries = Vec::new();
        boundaries
            .try_reserve_exact(cells)
            .map_err(|_| LoopJournalPrepareError::Allocation)?;
        boundaries.resize(cells, None);
        let initial = stream.snapshot();
        Ok(Self {
            stream,
            initial,
            acknowledged: initial,
            boundaries: boundaries.into_boxed_slice(),
            boundary_len: 0,
            end: None,
            storage_bytes,
        })
    }

    pub const fn initial(&self) -> LoopSnapshot {
        self.initial
    }

    /// Most recent successful whole-callback render snapshot. It can advance
    /// beyond the journal's terminal endpoint while playback continues.
    pub const fn acknowledged(&self) -> LoopSnapshot {
        self.acknowledged
    }

    pub const fn interval(&self) -> LoopInterval {
        self.stream.source.interval
    }

    pub const fn end(&self) -> Option<LoopJournalEnd> {
        self.end
    }

    pub const fn storage_bytes(&self) -> PreparedBytes {
        self.storage_bytes
    }

    /// Every admitted interior transition, retained until off-thread destruction.
    pub fn boundaries(&self) -> impl Iterator<Item = &LoopBoundary> {
        // Only a successful get_mut writes a cell and increments this length.
        self.boundaries[..self.boundary_len].iter().flatten()
    }
}
