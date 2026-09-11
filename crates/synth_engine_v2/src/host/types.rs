//! Host request, simulated capabilities and observable lifecycle records.

use thiserror::Error;

use crate::diagnostics::{CompileError, RenderError};
use crate::plan::PlanId;
use crate::profile::ProfileError;
use crate::quantities::{ChannelLayout, SampleRate};
use crate::time::{FrameCount, SampleTime, StreamEpoch};

/// Process-wide connection identity, minted independently of renderer epochs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct ConnectionGeneration(pub(super) u64);

impl ConnectionGeneration {
    /// Diagnostic representation. There is no public minting constructor.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Backend-stable opaque identity. A display name or enumeration index is not an ID.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct EndpointId(String);

impl EndpointId {
    /// Validate the backend representation off-thread.
    pub fn new(value: String) -> Result<Self, HostError> {
        if value.trim().is_empty() {
            return Err(HostError::EmptyEndpoint);
        }
        Ok(Self(value))
    }

    /// Backend representation for diagnostics and equality-based lookup.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Explicit device selection; default resolution happens during this user request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointSelection {
    Exact(EndpointId),
    SystemDefault,
}

/// A supported rate/layout pair; fallback permission applies to the whole pair.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct OutputFormat {
    pub rate: SampleRate,
    pub layout: ChannelLayout,
}

/// Host settings, deliberately outside project data. Buffer preference is a hint,
/// distinct from the backend's required maximum; no preference means backend choice.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct OutputRequest {
    pub endpoint: EndpointSelection,
    pub format: OutputFormat,
    pub fallback_endpoints: Vec<EndpointId>,
    pub fallback_formats: Vec<OutputFormat>,
    buffer_preference: Option<FrameCount>,
}

impl OutputRequest {
    /// Exact format, no implicit fallback and no buffer preference.
    pub fn new(endpoint: EndpointSelection, format: OutputFormat) -> Self {
        Self {
            endpoint,
            format,
            fallback_endpoints: Vec::new(),
            fallback_formats: Vec::new(),
            buffer_preference: None,
        }
    }

    /// An optional nonzero hint, never used as a guaranteed callback bound.
    pub fn with_buffer_preference(mut self, frames: FrameCount) -> Result<Self, HostError> {
        if frames == FrameCount::ZERO {
            return Err(HostError::ZeroBuffer);
        }
        self.buffer_preference = Some(frames);
        Ok(self)
    }

    pub const fn buffer_preference(&self) -> Option<FrameCount> {
        self.buffer_preference
    }
}

/// A simulator declaration of capability, distinct from its callback observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackBound {
    Guaranteed(FrameCount),
    Unknown { estimate: Option<FrameCount> },
}

/// One enumerated simulated endpoint. Duplicate stable IDs refuse selection;
/// duplicate display names are legal and never participate in identity checks.
#[derive(Debug, Clone)]
pub struct SimulatedEndpoint {
    pub id: EndpointId,
    pub display_name: String,
    pub format: OutputFormat,
    pub callback_bound: CallbackBound,
    pub open_succeeds: bool,
}

/// Deterministic discovery/open responses. This type has no physical-device path.
/// Fallback lookup proceeds only past missing endpoints. An enumerated endpoint's
/// open/configuration failure is reported, not hidden by another attempt.
#[derive(Debug, Clone, Default)]
pub struct SimulatedBackend {
    pub endpoints: Vec<SimulatedEndpoint>,
    pub default_output: Option<EndpointId>,
}

impl SimulatedBackend {
    pub(super) fn negotiate(&self, request: &OutputRequest) -> Result<NegotiatedOutput, HostError> {
        let primary = match &request.endpoint {
            EndpointSelection::Exact(id) => Some(id),
            EndpointSelection::SystemDefault => self.default_output.as_ref(),
        };
        for id in primary.into_iter().chain(request.fallback_endpoints.iter()) {
            let mut matching = self.endpoints.iter().filter(|endpoint| &endpoint.id == id);
            let Some(endpoint) = matching.next() else {
                continue;
            };
            if matching.next().is_some() {
                return Err(HostError::AmbiguousEndpoint);
            }
            if !endpoint.open_succeeds {
                return Err(HostError::OpenFailed);
            }
            if endpoint.format != request.format
                && !request.fallback_formats.contains(&endpoint.format)
            {
                return Err(HostError::UnpermittedFormat);
            }
            let maximum_callback = match endpoint.callback_bound {
                CallbackBound::Guaranteed(frames) if frames != FrameCount::ZERO => frames,
                CallbackBound::Guaranteed(_) => return Err(HostError::ZeroBuffer),
                CallbackBound::Unknown { .. } => return Err(HostError::UnknownCallbackBound),
            };
            return Ok(NegotiatedOutput {
                endpoint: endpoint.id.clone(),
                format: endpoint.format,
                maximum_callback,
                endpoint_substituted: primary != Some(id),
                format_substituted: endpoint.format != request.format,
            });
        }
        Err(HostError::EndpointUnavailable)
    }
}

/// Actual preparation inputs, visible before activation.
#[derive(Debug, Clone, PartialEq)]
pub struct NegotiatedOutput {
    pub endpoint: EndpointId,
    pub format: OutputFormat,
    pub maximum_callback: FrameCount,
    pub endpoint_substituted: bool,
    pub format_substituted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Stopped,
    Preparing,
    Ready,
    Running,
    Quiescing,
    Unavailable,
}

/// Coherent prepared plan/epoch pair. Its containing slot determines whether it is
/// active or a candidate; this is not a message acknowledging a concurrent callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedIdentity {
    pub epoch: StreamEpoch,
    pub plan: PlanId,
}

/// Persistent status belongs to the connection, not a lossy telemetry queue.
/// Timing is simulated, with no measured latency, input backlog or hardware provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionStatus {
    pub generation: ConnectionGeneration,
    pub state: ConnectionState,
    pub negotiated: Option<NegotiatedOutput>,
    pub identity: Option<PreparedIdentity>,
    pub failure: Option<HostFailure>,
    pub last_callback: Option<FrameCount>,
    pub clock: SampleTime,
    pub needs_reprepare: bool,
}

impl ConnectionStatus {
    pub(super) fn preparing(generation: ConnectionGeneration) -> Self {
        Self {
            generation,
            state: ConnectionState::Preparing,
            negotiated: None,
            identity: None,
            failure: None,
            last_callback: None,
            clock: SampleTime::ZERO,
            needs_reprepare: false,
        }
    }
}

/// Fixed-size retained failure, readable repeatedly without consuming it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HostFailure {
    EndpointUnavailable,
    AmbiguousEndpoint,
    OpenFailed,
    UnpermittedFormat,
    UnknownCallbackBound,
    InvalidConfiguration,
    Compilation,
    DeviceLost,
    Render(RenderError),
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error("endpoint identity must be nonempty")]
    EmptyEndpoint,
    #[error("buffer frame count must be nonzero")]
    ZeroBuffer,
    #[error("connection generation space exhausted")]
    GenerationExhausted,
    #[error("connection generation is stale")]
    StaleGeneration,
    #[error("operation requires a different connection state")]
    WrongState,
    #[error("stop transport before preparing or activating a connection")]
    TransportRunning,
    #[error("old callback resources await backend quiescence")]
    AwaitingQuiescence,
    #[error("selected endpoint and explicit fallbacks are unavailable")]
    EndpointUnavailable,
    #[error("endpoint identity matches more than one enumerated device")]
    AmbiguousEndpoint,
    #[error("backend refused stream creation")]
    OpenFailed,
    #[error("negotiated rate/layout is outside the explicit fallback set")]
    UnpermittedFormat,
    #[error("backend has no guaranteed callback maximum; estimates are insufficient")]
    UnknownCallbackBound,
    #[error("host profile refused configuration: {0}")]
    Profile(#[from] ProfileError),
    #[error("plan preparation failed: {0}")]
    Compile(#[from] CompileError),
}

impl HostError {
    pub(super) const fn failure(&self) -> HostFailure {
        match self {
            Self::EndpointUnavailable => HostFailure::EndpointUnavailable,
            Self::AmbiguousEndpoint => HostFailure::AmbiguousEndpoint,
            Self::OpenFailed => HostFailure::OpenFailed,
            Self::UnpermittedFormat => HostFailure::UnpermittedFormat,
            Self::UnknownCallbackBound => HostFailure::UnknownCallbackBound,
            Self::Compile(_) => HostFailure::Compilation,
            _ => HostFailure::InvalidConfiguration,
        }
    }
}

/// Callback errors contain no allocating diagnostic payload.
#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum CallbackError {
    #[error("callback belongs to a stale connection")]
    StaleGeneration,
    #[error("running connection has no prepared renderer")]
    NotPrepared,
    #[error("renderer fault: {0}")]
    Render(RenderError),
}
