//! Reservation identities and retained-result metadata. No persisted representation.

use crate::host::ConnectionGeneration;
use crate::quantities::PreparedBytes;
use crate::time::{SampleTime, StreamEpoch};
use thiserror::Error;

/// Non-reused identity, separate from the reusable storage position in a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct TakeId(pub(super) u64);
impl TakeId {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// A reservation capability. A stale or foreign ticket never addresses reused storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct TakeReservation {
    pub(super) id: TakeId,
    pub(super) slot: usize,
}
impl TakeReservation {
    pub const fn id(self) -> TakeId {
        self.id
    }
}

/// Explicit finite capture interval in one runtime epoch, before any projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct CaptureWindow {
    pub(super) epoch: StreamEpoch,
    pub(super) start: SampleTime,
    pub(super) end: SampleTime,
}
impl CaptureWindow {
    pub fn new(
        epoch: StreamEpoch,
        start: SampleTime,
        end: SampleTime,
    ) -> Result<Self, CaptureError> {
        if start > end {
            return Err(CaptureError::InvalidWindow);
        }
        Ok(Self { epoch, start, end })
    }
    pub const fn epoch(self) -> StreamEpoch {
        self.epoch
    }
    pub const fn start(self) -> SampleTime {
        self.start
    }
    pub const fn end(self) -> SampleTime {
        self.end
    }
}

/// Typed storage regions. All are charged independently of ordinary input records.
/// The payload type describes already validated fixed-size storage cells, not a MIDI
/// protocol. The first real publisher must supply its own semantic representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBuffer {
    Ordinary,
    TrackedInput,
    TerminalOccurrence,
    InitialSourceState,
    TerminalSourceState,
    Pass,
    Carry,
}
impl CaptureBuffer {
    pub(super) const COUNT: usize = 7;
    pub(super) const fn index(self) -> usize {
        match self {
            Self::Ordinary => 0,
            Self::TrackedInput => 1,
            Self::TerminalOccurrence => 2,
            Self::InitialSourceState => 3,
            Self::TerminalSourceState => 4,
            Self::Pass => 5,
            Self::Carry => 6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    Complete,
    Partial,
    Interrupted,
}

/// A checked coalescing diagnostic count; overflow remains visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[must_use]
pub struct DiagnosticCount {
    pub(super) value: u64,
    pub(super) saturated: bool,
}
impl DiagnosticCount {
    pub const fn as_u64(self) -> u64 {
        self.value
    }
    pub const fn saturated(self) -> bool {
        self.saturated
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LateCaptureInput {
    pub source: ConnectionGeneration,
    pub time: SampleTime,
}

/// Sticky, pre-reserved quality. Consumers read this beside the immutable sealed
/// outcome; a Complete result with a late fault is effectively Partial.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[must_use]
pub struct CaptureQuality {
    pub(super) first_late: Option<LateCaptureInput>,
    pub(super) first_uncertain_source: Option<ConnectionGeneration>,
    pub(super) late_count: DiagnosticCount,
}
impl CaptureQuality {
    pub const fn first_late(self) -> Option<LateCaptureInput> {
        self.first_late
    }
    /// A refused uncertainty interval overlapped this selected capture interval.
    /// This identifies its source without manufacturing an exact late timestamp.
    pub const fn first_uncertain_source(self) -> Option<ConnectionGeneration> {
        self.first_uncertain_source
    }
    pub const fn late_count(self) -> DiagnosticCount {
        self.late_count
    }
    pub const fn effective_outcome(self, sealed: CaptureOutcome) -> CaptureOutcome {
        if matches!(sealed, CaptureOutcome::Complete)
            && (self.first_late.is_some() || self.first_uncertain_source.is_some())
        {
            CaptureOutcome::Partial
        } else {
            sealed
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LateAttribution {
    TakeFaulted,
    OutsideSelectedInterval,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct SourceAttribution {
    pub generation: ConnectionGeneration,
    pub watermark: SampleTime,
    pub frontier: SampleTime,
    pub fenced: bool,
    pub quiescent: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TakeState {
    pub id: TakeId,
    pub requested_window: CaptureWindow,
    pub window: CaptureWindow,
    pub outcome: Option<CaptureOutcome>,
    pub quality: CaptureQuality,
    pub ordinary_len: usize,
    pub forced_partial: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[error("all eight additional recording settings are required before reservation")]
    MissingConfiguration,
    #[error("capture layout arithmetic or allocation extent is unrepresentable")]
    LayoutOverflow,
    #[error("capture storage needs {required}, exceeding max_capture_bytes {available}")]
    ByteBudget {
        required: PreparedBytes,
        available: PreparedBytes,
    },
    #[error("capture allocation failed before reservation")]
    Allocation,
    #[error("capture window end precedes its start")]
    InvalidWindow,
    #[error("capture sources must be nonempty, unique and within the admitted source count")]
    InvalidSources,
    #[error("all active or retained capture-result entitlements are occupied")]
    ResultsFull,
    #[error("take identities are exhausted")]
    IdentityExhausted,
    #[error("capture reservation is stale or foreign")]
    StaleReservation,
    #[error("capture storage is sealed")]
    Sealed,
    #[error("capture result has not been sealed")]
    NotSealed,
    #[error("ordinary capture capacity exhausted; finalization space remains reserved")]
    OrdinaryFull,
    #[error("capture stopped after a known gap or capacity exhaustion")]
    CaptureStopped,
    #[error("metadata cell is outside its reserved region")]
    MetadataBounds,
    #[error("source does not belong to this reservation")]
    ForeignSource,
    #[error("source generation is already quiescent")]
    SourceQuiescent,
    #[error(
        "a first source introduction must be newer than every previously introduced generation"
    )]
    SourceNotFresh,
    #[error("capture cannot start before an already consumed source frontier")]
    WindowBeforeFrontier,
    #[error("capture timestamp belongs to another epoch")]
    ForeignEpoch,
    #[error("source watermark or consumed frontier cannot retreat")]
    WatermarkRetreat,
    #[error("capture input is not below the acknowledged source watermark")]
    NotLate,
    #[error("the selected stop boundary is outside the reserved capture window")]
    InvalidStop,
    #[error("source fences have not reached the stop boundary")]
    AwaitingFences,
    #[error("source generations still own late-fault attribution")]
    SourcesLive,
    #[error("owner must acknowledge the current sticky capture quality")]
    QualityChanged,
}
