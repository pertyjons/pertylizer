//! Ordered compiled transport for the simulated host.

use crate::host::ConnectionGeneration;
use crate::quantities::PreparedBytes;
use crate::time::{PlanPosition, SampleTime};
use crate::transport::ActivationRefused;
use thiserror::Error;

/// Fixed command slots, including the slot reserved against Play traffic for Stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SessionCommandCapacity(u32);
impl SessionCommandCapacity {
    pub fn new(slots: u32) -> Result<Self, SessionError> {
        if slots < 2 {
            return Err(SessionError::Capacity);
        }
        Ok(Self(slots))
    }
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

/// Admission for command descriptors and their retained final outcomes.
/// Renderer/scheduler resources retain their own existing preparation admission.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct SessionLimits {
    pub commands: SessionCommandCapacity,
    pub command_bytes: PreparedBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SessionCommandId {
    pub(super) generation: ConnectionGeneration,
    pub(super) serial: u64,
}
impl SessionCommandId {
    pub(crate) fn after(
        generation: ConnectionGeneration,
        previous: u64,
    ) -> Result<Self, SessionError> {
        let serial = previous
            .checked_add(1)
            .ok_or(SessionError::IdentityExhausted)?;
        Ok(Self { generation, serial })
    }

    pub const fn generation(self) -> ConnectionGeneration {
        self.generation
    }
    pub const fn serial(self) -> u64 {
        self.serial
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCommand {
    Play,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SessionBoundary {
    pub id: SessionCommandId,
    pub at: SampleTime,
    pub command: SessionCommand,
    pub capture: Option<crate::recording::TakeReservation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOutcome {
    DeliveryRefused(super::transfer::SessionDeliveryError),
    CaptureRefused,
    Applied { position: PlanPosition },
    Cancelled,
    Refused(ActivationRefused),
}

/// The session's musical mapping; the renderer remains the sole engine clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum PlaybackState {
    /// Terminal closure could not represent the final playhead; no position is invented.
    Unavailable,
    Stopped(PlanPosition),
    Playing(crate::time::StreamAnchor),
}

#[derive(Debug, PartialEq)]
pub enum SessionCaptureOutcome {
    Applied,
    Refused(crate::recording::notes::NoteCaptureError),
    Cancelled,
}

#[derive(Debug, PartialEq)]
#[must_use]
pub struct SessionReceipt {
    pub boundary: SessionBoundary,
    pub outcome: SessionOutcome,
    pub capture: Option<SessionCaptureOutcome>,
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("prepared plan replacements must be resolved before transport commands")]
    ReplacementOutstanding,
    #[error("session profile differs from the compiled plan geometry or publication limits")]
    TransferProfile,
    #[error(transparent)]
    Transfer(#[from] super::transfer::SessionTransferError),
    #[error("source-action capacity must be positive")]
    SourceCapacity,
    #[error(
        "source actions must follow time order, with fences before publications at equal times"
    )]
    SourceOrder,
    #[error("ordered capture is not prepared")]
    NoCapture,
    #[error("capture context must match the stopped position and requested Play boundary")]
    CaptureContext,
    #[error("capture preparation or command validation failed: {0}")]
    Capture(#[from] crate::recording::notes::NoteCaptureError),
    #[error("same-boundary session commands and Play catch-up exceed the session share")]
    SessionShare,
    #[error("ordered transport owns Play/Stop; use its command lane")]
    OrderedTransport,
    #[error("ordered session outcomes must be collected before retirement")]
    RetainedOutcomes,
    #[error("ordered session timeline failed: {0}")]
    Timeline(#[from] crate::schedule::ScheduledRenderError),
    #[error("ordered transport requires a fresh renderer and no direct capture owner")]
    FreshStreamRequired,
    #[error("ordered session has closed admission")]
    Closed,
    #[error("ordered session command capacity must contain at least two slots")]
    Capacity,
    #[error("ordered session storage layout overflows the platform")]
    Layout,
    #[error("ordered session command storage requires {required:?}, budget is {available:?}")]
    ByteBudget {
        required: PreparedBytes,
        available: PreparedBytes,
    },
    #[error("ordered session allocation failed")]
    Allocation,
    #[error("ordered session is already enabled")]
    AlreadyEnabled,
    #[error("ordered session is not enabled")]
    NotEnabled,
    #[error("command must name this generation and a future quantum boundary in offer order")]
    Boundary,
    #[error("ordered session command and receipt slots are full")]
    Full,
    #[error("a previous Play still owns its activation; collect its outcome first")]
    PlayOutstanding,
    #[error("Play requires stopped transport")]
    AlreadyPlaying,
    #[error("ordered session command identities are exhausted")]
    IdentityExhausted,
    #[error("ordered session preparation failed: {0}")]
    Schedule(#[from] crate::schedule::SchedulePrepareError),
    #[error("ordered session publication preparation failed: {0}")]
    Profile(#[from] crate::profile::ProfileError),
    #[error("Play preparation failed: {0}")]
    Activation(#[from] crate::stream::ActivationBuildError),
    #[error("activation collection failed: {0}")]
    Collection(crate::stream::ActivationCollectError),
}
