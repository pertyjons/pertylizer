//! Explicit source actions for the serial session owner.
use super::super::SessionError;
use crate::host::ConnectionGeneration;
use crate::quantities::PreparedBytes;
use crate::recording::notes::{
    AuditionTrace, CaptureStamp, Midi1Input, NoteCaptureError, PublicationReceipt,
};
use crate::time::{SampleTime, StreamEpoch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SessionSourceCapacity(u32);
impl SessionSourceCapacity {
    pub fn new(slots: u32) -> Result<Self, SessionError> {
        if slots == 0 {
            return Err(SessionError::SourceCapacity);
        }
        Ok(Self(slots))
    }
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct SessionSourceLimits {
    pub actions: SessionSourceCapacity,
    pub bytes: PreparedBytes,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SessionSourceId {
    pub(super) generation: ConnectionGeneration,
    pub(super) serial: u64,
}
impl SessionSourceId {
    pub const fn generation(self) -> ConnectionGeneration {
        self.generation
    }
    pub const fn serial(self) -> u64 {
        self.serial
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub enum SessionSourceAction {
    Publish {
        source: ConnectionGeneration,
        stamp: CaptureStamp,
        input: Midi1Input,
        audition: AuditionTrace,
    },
    /// Explicit prefix frontier; the serial owner supplies its actually consumed
    /// publication sequence when dispatching this fence, never guessed backend progress.
    Fence {
        source: ConnectionGeneration,
        epoch: StreamEpoch,
        frontier: SampleTime,
    },
}
impl SessionSourceAction {
    pub const fn at(self) -> SampleTime {
        match self {
            Self::Publish { stamp, .. } => stamp.published_at(),
            Self::Fence { frontier, .. } => frontier,
        }
    }
    pub const fn source(self) -> ConnectionGeneration {
        match self {
            Self::Publish { source, .. } | Self::Fence { source, .. } => source,
        }
    }
    pub const fn epoch(self) -> StreamEpoch {
        match self {
            Self::Publish { stamp, .. } => stamp.epoch(),
            Self::Fence { epoch, .. } => epoch,
        }
    }
}
#[derive(Debug, PartialEq)]
pub enum SessionSourceOutcome {
    Published(PublicationReceipt),
    Fenced,
    Refused(NoteCaptureError),
    Cancelled,
}
#[derive(Debug, PartialEq)]
#[must_use]
pub struct SessionSourceReceipt {
    pub id: SessionSourceId,
    pub action: SessionSourceAction,
    pub outcome: SessionSourceOutcome,
}
