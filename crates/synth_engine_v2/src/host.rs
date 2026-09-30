//! Stopped-only host lifecycle over a deterministic simulated output backend (P09-S001).
//!
//! This is host-side code, separate from the render plan. It opens no platform device.
//! The simulator serializes invocations and exposes delayed quiescence explicitly; it
//! does not prove a real backend's callback fence or concurrent publication mechanism.
//! P09-S005 attaches serial exact-input note capture and retains it across output loss.
//! P09-S006 orders explicit capture boundaries through the bounded serial session lane.
//! P09-S007 adds ordered compiled Play/Stop coupled to retained exact-input note capture.
//! Physical input, live ingress and automatic retries retain their
//! first-consumer IO/TAKE and ADR-0022/0050/0054 gates.

#[cfg(feature = "simulated-ingress")]
mod capture;
#[cfg(feature = "simulated-ingress")]
pub use crate::recording::notes::loop_capture::ordered::input;
#[cfg(feature = "simulated-ingress")]
pub use capture::NoteCaptureControl;

#[cfg(feature = "simulated-ingress")]
pub mod audio_input;
mod hot;
#[cfg(feature = "simulated-ingress")]
pub mod live;
pub mod mixed_targets;
#[cfg(feature = "simulated-ingress")]
pub mod pulse;
#[cfg(feature = "simulated-ingress")]
pub mod session;
mod types;
pub use types::*;

use std::sync::atomic::{AtomicU64, Ordering};

use crate::compile::{RenderConfig, compile};
use crate::ir::GraphIr;
use crate::plan::CompiledPlan;
use crate::profile::HostProfile;
use crate::render::PreparedRenderer;
use crate::stream::StreamControl;
use crate::time::{PlanPosition, SampleTime, StreamAnchor};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(0);

fn issue_generation(counter: &AtomicU64) -> Result<ConnectionGeneration, HostError> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            last.checked_add(1)
        })
        .map(|last| ConnectionGeneration(last + 1))
        .map_err(|_| HostError::GenerationExhausted)
}

#[cfg(feature = "simulated-ingress")]
pub(crate) fn issue_capture_source_generation() -> Result<ConnectionGeneration, HostError> {
    issue_generation(&NEXT_GENERATION)
}

struct PreparedOutput {
    control: StreamControl,
    renderer: PreparedRenderer,
    #[cfg(feature = "simulated-ingress")]
    session: Option<session::SessionRuntime>,
}

struct Connection {
    status: ConnectionStatus,
    prepared: Option<PreparedOutput>,
}

/// A bounded simulated host: one active slot and one candidate slot.
///
/// Except for `callback`, methods run on the simulated control thread. A callback
/// borrows its connection for the complete invocation. `acknowledge_quiescence` models
/// the backend fence, not an audio acknowledgement; it works without a final callback.
/// No callback owns a resource that it could finally drop. Replacement waits for that
/// fence, so slow retirement cannot create an unbounded list of retained renderers.
#[must_use]
#[derive(Default)]
pub struct SimulatedHost {
    selected: Option<OutputRequest>,
    active: Option<Connection>,
    candidate: Option<Connection>,
    last_valid: Option<StreamControl>,
    #[cfg(feature = "simulated-ingress")]
    note_capture: Option<crate::recording::notes::SimulatedNoteRecorder>,
}

impl SimulatedHost {
    /// No stream, request or epoch exists initially.
    pub fn new() -> Self {
        Self::default()
    }

    /// The user's intent survives negotiation failure and disconnect.
    pub const fn selected_request(&self) -> Option<&OutputRequest> {
        self.selected.as_ref()
    }

    /// Active connection status; candidate preparation never overwrites it.
    pub fn active(&self) -> Option<&ConnectionStatus> {
        self.active.as_ref().map(|connection| &connection.status)
    }

    /// The current preparation, including a failed candidate's diagnosis.
    pub fn candidate(&self) -> Option<&ConnectionStatus> {
        self.candidate.as_ref().map(|connection| &connection.status)
    }

    /// The most recently activated plan remains owned even after device retirement.
    pub fn last_valid_plan(&self) -> Option<&CompiledPlan> {
        self.active
            .as_ref()
            .and_then(|connection| connection.prepared.as_ref())
            .map(|prepared| prepared.control.plan())
            .or_else(|| self.last_valid.as_ref().map(StreamControl::plan))
    }

    fn require_stopped(&self) -> Result<(), HostError> {
        if self
            .active()
            .is_some_and(|status| status.state == ConnectionState::Running)
        {
            Err(HostError::TransportRunning)
        } else {
            Ok(())
        }
    }

    /// A new user selection cancels a candidate that owns no callback resources.
    /// A prepared candidate must first be shut down and fenced. Explicit calls are
    /// the only retry mechanism; there is no background retry or wall-clock catch-up.
    pub fn begin(&mut self, request: OutputRequest) -> Result<ConnectionGeneration, HostError> {
        self.require_stopped()?;
        if self
            .candidate
            .as_ref()
            .is_some_and(|connection| connection.prepared.is_some())
        {
            return Err(HostError::AwaitingQuiescence);
        }
        let generation = issue_generation(&NEXT_GENERATION)?;
        self.candidate = Some(Connection {
            status: ConnectionStatus::preparing(generation),
            prepared: None,
        });
        self.selected = Some(request);
        Ok(generation)
    }

    /// Complete one simulated backend open and compile under its actual configuration.
    /// Neither an estimate nor an observation is promoted to the required bound.
    /// A stale completion is rejected before reading or changing the current candidate.
    pub fn prepare(
        &mut self,
        generation: ConnectionGeneration,
        backend: &SimulatedBackend,
        graph: &GraphIr,
    ) -> Result<(), HostError> {
        self.require_stopped()?;
        let candidate = self.candidate.as_mut().ok_or(HostError::StaleGeneration)?;
        if candidate.status.generation != generation {
            return Err(HostError::StaleGeneration);
        }
        if candidate.status.state != ConnectionState::Preparing {
            return Err(HostError::WrongState);
        }
        let request = self.selected.as_ref().ok_or(HostError::WrongState)?;
        let result = prepare_output(request, backend, graph, &mut candidate.status);
        match result {
            Ok(prepared) => {
                candidate.status.identity = Some(PreparedIdentity {
                    epoch: prepared.renderer.epoch(),
                    plan: prepared.control.plan_id(),
                });
                candidate.prepared = Some(prepared);
                candidate.status.state = ConnectionState::Ready;
                Ok(())
            }
            Err(error) => {
                candidate.status.state = ConnectionState::Unavailable;
                candidate.status.failure = Some(error.failure());
                Err(error)
            }
        }
    }

    /// Publish all prepared state together, while stopped and after old callback access
    /// has been fenced. Activation stays Ready; only an explicit `start` plays audio.
    pub fn activate(&mut self, generation: ConnectionGeneration) -> Result<(), HostError> {
        self.require_stopped()?;
        let candidate = self.candidate.as_ref().ok_or(HostError::StaleGeneration)?;
        if candidate.status.generation != generation {
            return Err(HostError::StaleGeneration);
        }
        if candidate.status.state != ConnectionState::Ready {
            return Err(HostError::WrongState);
        }
        if self
            .active
            .as_ref()
            .is_some_and(|connection| connection.prepared.is_some())
        {
            return Err(HostError::AwaitingQuiescence);
        }
        self.active = self.candidate.take();
        Ok(())
    }

    fn active_mut(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<&mut Connection, HostError> {
        self.active
            .as_mut()
            .filter(|connection| connection.status.generation == generation)
            .ok_or(HostError::StaleGeneration)
    }

    fn connection_mut(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<&mut Connection, HostError> {
        self.active
            .iter_mut()
            .chain(self.candidate.iter_mut())
            .find(|connection| connection.status.generation == generation)
            .ok_or(HostError::StaleGeneration)
    }

    /// Start/resume the prepared simulator. No live ingress or session activation is
    /// offered; stopped callbacks freeze this fixture's musical position.
    pub fn start(&mut self, generation: ConnectionGeneration) -> Result<(), HostError> {
        let connection = self.active_mut(generation)?;
        if connection.status.state != ConnectionState::Ready {
            return Err(HostError::WrongState);
        }
        connection.status.state = ConnectionState::Running;
        Ok(())
    }

    /// Stop transport without retiring the device or creating another epoch.
    pub fn stop(&mut self, generation: ConnectionGeneration) -> Result<(), HostError> {
        #[cfg(feature = "simulated-ingress")]
        if self
            .active_mut(generation)?
            .prepared
            .as_ref()
            .is_some_and(|prepared| prepared.session.is_some())
        {
            return Err(session::SessionError::OrderedTransport.into());
        }
        #[cfg(feature = "simulated-ingress")]
        if self.note_capture.as_ref().is_some_and(|capture| {
            capture.host_generation == Some(generation) && capture.is_active()
        }) {
            return Err(HostError::CaptureActive);
        }
        let connection = self.active_mut(generation)?;
        if !matches!(
            connection.status.state,
            ConnectionState::Ready | ConnectionState::Running
        ) {
            return Err(HostError::WrongState);
        }
        connection.status.state = ConnectionState::Ready;
        Ok(())
    }

    /// Request off-thread shutdown. The simulator never requires hardware pause:
    /// even a backend without pause support retires through the same fence.
    pub fn shutdown(&mut self, generation: ConnectionGeneration) -> Result<(), HostError> {
        let connection = self.connection_mut(generation)?;
        if connection.status.state == ConnectionState::Preparing {
            connection.status.state = ConnectionState::Stopped;
            return Ok(());
        }
        if !matches!(
            connection.status.state,
            ConnectionState::Ready | ConnectionState::Running
        ) {
            return Err(HostError::WrongState);
        }
        connection.status.state = ConnectionState::Quiescing;
        #[cfg(feature = "simulated-ingress")]
        self.close_ordered_session(generation)?;
        #[cfg(feature = "simulated-ingress")]
        self.interrupt_note_capture(generation)?;
        Ok(())
    }

    /// An asynchronous output-device error may be delivered without another callback.
    /// The first failure persists until this connection is replaced.
    pub fn device_lost(&mut self, generation: ConnectionGeneration) -> Result<(), HostError> {
        self.fail_device(generation, HostFailure::DeviceLost, false)
    }

    /// The device changed its own rate, layout or callback bound. The prepared plan was
    /// compiled for the old configuration and never renders under the new one: the
    /// connection quiesces as for a loss, needs no further callback, and reports
    /// `needs_reprepare`. Recovery is an explicit preparation against the backend's new
    /// configuration; a request that does not permit it fails visibly on that candidate.
    pub fn device_reconfigured(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<(), HostError> {
        self.fail_device(generation, HostFailure::DeviceReconfigured, true)
    }

    fn fail_device(
        &mut self,
        generation: ConnectionGeneration,
        failure: HostFailure,
        reprepare: bool,
    ) -> Result<(), HostError> {
        let connection = self.connection_mut(generation)?;
        if connection.status.state == ConnectionState::Preparing {
            connection.status.failure = Some(failure);
            connection.status.state = ConnectionState::Unavailable;
            connection.status.needs_reprepare |= reprepare;
            return Ok(());
        }
        if !matches!(
            connection.status.state,
            ConnectionState::Ready | ConnectionState::Running | ConnectionState::Quiescing
        ) {
            return Err(HostError::WrongState);
        }
        connection.status.failure.get_or_insert(failure);
        connection.status.needs_reprepare |= reprepare;
        connection.status.state = ConnectionState::Quiescing;
        #[cfg(feature = "simulated-ingress")]
        self.close_ordered_session(generation)?;
        #[cfg(feature = "simulated-ingress")]
        self.interrupt_note_capture(generation)?;
        Ok(())
    }

    /// Backend-side quiescence, deliberately independent of an audio callback. The
    /// simulator serializes calls with exclusive borrows, so no invocation is in flight.
    /// A physical backend must provide its own proven fence before using this model.
    pub fn acknowledge_quiescence(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<(), HostError> {
        if self.connection_mut(generation)?.status.state != ConnectionState::Quiescing {
            return Err(HostError::WrongState);
        }
        #[cfg(feature = "simulated-ingress")]
        self.close_ordered_session(generation)?;
        #[cfg(feature = "simulated-ingress")]
        self.require_session_collected(generation)?;
        #[cfg(feature = "simulated-ingress")]
        self.interrupt_note_capture(generation)?;
        #[cfg(feature = "simulated-ingress")]
        if self.note_capture.as_ref().is_some_and(|capture| {
            capture.host_generation == Some(generation) && !capture.host_quiescent()
        }) {
            return Err(HostError::AwaitingCaptureQuiescence);
        }
        let was_active = self
            .active()
            .is_some_and(|status| status.generation == generation);
        let connection = self.connection_mut(generation)?;
        if connection.status.state != ConnectionState::Quiescing {
            return Err(HostError::WrongState);
        }
        connection.status.state = if connection.status.failure.is_some() {
            ConnectionState::Unavailable
        } else {
            ConnectionState::Stopped
        };
        connection.status.identity = None;
        // A canceled or failed candidate never becomes the last valid active plan.
        let retired = connection.prepared.take();
        if let Some(prepared) = retired.filter(|_| was_active) {
            self.last_valid = Some(prepared.control);
            // Renderer, buffers and the superseded retained control drop here, off-thread.
        }
        Ok(())
    }
}

fn prepare_output(
    request: &OutputRequest,
    backend: &SimulatedBackend,
    graph: &GraphIr,
    status: &mut ConnectionStatus,
) -> Result<PreparedOutput, HostError> {
    let negotiated = backend.negotiate(request)?;
    let profile = HostProfile::harness(
        negotiated.format.rate,
        negotiated.maximum_callback,
        negotiated.format.layout,
    )?;
    status.negotiated = Some(negotiated);
    let plan = compile(graph, &RenderConfig::new(profile)).into_plan()?;
    let (control, renderer) = StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )?;
    Ok(PreparedOutput {
        control,
        renderer,
        #[cfg(feature = "simulated-ingress")]
        session: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callbacks_neither_allocate_nor_reclaim_even_during_fault_and_retirement() {
        use crate::ir::{ExecutionScope, IrNodeKind, NodeId, PortId, SignalDomain};
        use crate::quantities::{Amplitude, ChannelLayout, SampleRate};
        use crate::render::AudioBlockMut;
        use crate::time::FrameCount;

        let endpoint = EndpointId::new("test-output".to_owned()).unwrap();
        let format = OutputFormat {
            rate: SampleRate::new(48_000.0).unwrap(),
            layout: ChannelLayout::Mono,
        };
        let request = OutputRequest::new(EndpointSelection::Exact(endpoint.clone()), format);
        let backend = SimulatedBackend {
            endpoints: vec![SimulatedEndpoint {
                id: endpoint,
                display_name: "Output".to_owned(),
                format,
                callback_bound: CallbackBound::Guaranteed(FrameCount::new(64)),
                open_succeeds: true,
            }],
            default_output: None,
        };
        let graph = GraphIr::builder()
            .node(
                NodeId::new(1),
                IrNodeKind::Constant {
                    level: Amplitude::new(0.25).unwrap(),
                },
                ExecutionScope::Global,
            )
            .node(NodeId::new(2), IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (NodeId::new(1), PortId::FIRST),
                (NodeId::new(2), PortId::FIRST),
                SignalDomain::Audio,
            )
            .build()
            .unwrap();
        let mut host = SimulatedHost::new();
        let old = host.begin(request.clone()).unwrap();
        host.prepare(old, &backend, &graph).unwrap();
        host.activate(old).unwrap();
        host.start(old).unwrap();
        let mut samples = [1.0; 64];
        let mut oversized = [1.0; 65];
        let events = crate::render_allocation::count_allocs(|| {
            host.callback(
                old,
                AudioBlockMut::new(&mut samples, 64, format.layout).unwrap(),
            )
            .unwrap();
            assert!(
                host.callback(
                    old,
                    AudioBlockMut::new(&mut oversized, 65, format.layout).unwrap()
                )
                .is_err()
            );
            host.callback(
                old,
                AudioBlockMut::new(&mut samples, 64, format.layout).unwrap(),
            )
            .unwrap();
        });
        assert_eq!(events, 0);
        assert!(host.active.as_ref().unwrap().prepared.is_some());
        // Retirement is stalled. Repeated user requests replace only the candidate;
        // they cannot reclaim the old callback state or activate alongside it.
        for _ in 0..8 {
            let candidate = host.begin(request.clone()).unwrap();
            host.prepare(candidate, &backend, &graph).unwrap();
            let events = crate::render_allocation::count_allocs(|| {
                host.callback(
                    candidate,
                    AudioBlockMut::new(&mut samples, 64, format.layout).unwrap(),
                )
                .unwrap();
            });
            assert_eq!(events, 0);
            assert!(matches!(
                host.activate(candidate),
                Err(HostError::AwaitingQuiescence)
            ));
            assert!(host.active.as_ref().unwrap().prepared.is_some());
            assert!(matches!(
                host.begin(request.clone()),
                Err(HostError::AwaitingQuiescence)
            ));
            host.shutdown(candidate).unwrap();
            host.acknowledge_quiescence(candidate).unwrap();
        }
        host.acknowledge_quiescence(old).unwrap();
        assert!(host.active.as_ref().unwrap().prepared.is_none());
        assert!(host.last_valid.is_some());
        let new = host.begin(request).unwrap();
        host.prepare(new, &backend, &graph).unwrap();
        host.activate(new).unwrap();
        let events = crate::render_allocation::count_allocs(|| {
            assert_eq!(
                host.callback(
                    old,
                    AudioBlockMut::new(&mut samples, 64, format.layout).unwrap()
                ),
                Err(CallbackError::StaleGeneration)
            );
            host.callback(
                new,
                AudioBlockMut::new(&mut samples, 64, format.layout).unwrap(),
            )
            .unwrap();
        });
        assert_eq!(events, 0);
    }

    #[test]
    fn generation_exhaustion_never_wraps_or_reuses() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(issue_generation(&counter).unwrap().as_u64(), u64::MAX);
        assert!(matches!(
            issue_generation(&counter),
            Err(HostError::GenerationExhausted)
        ));
        assert!(matches!(
            issue_generation(&counter),
            Err(HostError::GenerationExhausted)
        ));
    }
}
