//! Bounded serial session ownership for compiled playback.

mod capture;
mod hot;
mod source;
pub use capture::SessionCaptureControl;
pub use source::*;
#[cfg(test)]
mod tests;
pub mod transfer;
mod types;
pub use types::*;

use super::{ConnectionGeneration, ConnectionState, HostError, SimulatedHost};
use crate::profile::HostProfile;
use crate::publish::PublicationArbiter;
use crate::quantities::{EventCount, PreparedBytes};
use crate::schedule::{AdmittedCompiledStream, CompiledEventScheduler};
use crate::stream::{ActivationRequest, StreamControl};
use crate::time::{QUANTUM_FRAMES, SampleTime};
use crate::transport::TransportActivation;

#[derive(Debug)]
struct CommandEntry {
    boundary: SessionBoundary,
    activation: Option<Box<TransportActivation>>,
    outcome: Option<SessionOutcome>,
    capture_outcome: Option<SessionCaptureOutcome>,
    expected_position: Option<crate::time::PlanPosition>,
    delivery_error: Option<transfer::SessionDeliveryError>,
}

pub(super) struct SessionRuntime {
    generation: ConnectionGeneration,
    sources: Option<source::SourceQueue>,
    stream: Option<AdmittedCompiledStream>,
    scheduler: CompiledEventScheduler,
    arbiter: PublicationArbiter,
    slots: Box<[Option<Box<CommandEntry>>]>,
    head: usize,
    held: usize,
    completed: usize,
    serial: u64,
    last_offer: Option<SampleTime>,
    state: PlaybackState,
    // Serial admission only; split admission is guarded by the control credit ledger.
    play_outstanding: bool,
    adopted_slot: Option<usize>,
    closed: bool,
    command_bytes: PreparedBytes,
    session_share: EventCount,
    play_cost: EventCount,
    operations: EventCount,
}

impl SessionRuntime {
    pub(super) fn has_retained_outcomes(&self) -> bool {
        self.held != 0
            || self
                .sources
                .as_ref()
                .is_some_and(source::SourceQueue::has_held)
    }

    fn close(&mut self, clock: SampleTime) -> Result<(), SessionError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        if let Some(sources) = self.sources.as_mut() {
            sources.close();
        }
        if let Some(index) = self.adopted_slot.take()
            && let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut)
        {
            entry.activation = self.scheduler.close_exchange();
        }
        for offset in self.completed..self.held {
            let index = (self.head + offset) % self.slots.len();
            if let Some(entry) = self.slots.get_mut(index).and_then(Option::as_mut) {
                entry.outcome = Some(SessionOutcome::Cancelled);
                if entry.boundary.capture.is_some() && entry.capture_outcome.is_none() {
                    entry.capture_outcome = Some(SessionCaptureOutcome::Cancelled);
                }
            }
        }
        self.completed = self.held;
        if let Err(error) = self.stop_at(clock) {
            self.state = PlaybackState::Unavailable;
            return Err(error.into());
        }
        Ok(())
    }

    fn prepare(
        generation: ConnectionGeneration,
        control: &mut StreamControl,
        stream: AdmittedCompiledStream,
        profile: &HostProfile,
        limits: SessionLimits,
    ) -> Result<Self, SessionError> {
        let mut runtime = Self::prepare_detached(generation, control, &stream, profile, limits)?;
        runtime.stream = Some(stream);
        Ok(runtime)
    }

    fn prepare_detached(
        generation: ConnectionGeneration,
        control: &mut StreamControl,
        stream: &AdmittedCompiledStream,
        profile: &HostProfile,
        limits: SessionLimits,
    ) -> Result<Self, SessionError> {
        let count = usize::try_from(limits.commands.as_u32()).map_err(|_| SessionError::Layout)?;
        let bytes = count
            .checked_mul(size_of::<Option<Box<CommandEntry>>>() + size_of::<CommandEntry>())
            .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .ok_or(SessionError::Layout)?;
        let required = PreparedBytes::measured(bytes as u64);
        if required > limits.command_bytes {
            return Err(SessionError::ByteBudget {
                required,
                available: limits.command_bytes,
            });
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(count)
            .map_err(|_| SessionError::Allocation)?;
        slots.resize_with(count, || None);
        let arbiter = PublicationArbiter::prepare(profile)?;
        let play_cost = u32::try_from(control.plan().parameter_addresses().len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or(SessionError::Layout)?;
        // All other fallible preparation precedes the schedule's ownership latch.
        let scheduler = CompiledEventScheduler::prepare(control, stream)?;
        Ok(Self {
            generation,
            sources: None,
            stream: None,
            scheduler,
            arbiter,
            slots: slots.into_boxed_slice(),
            head: 0,
            held: 0,
            completed: 0,
            serial: 0,
            last_offer: None,
            state: PlaybackState::Stopped(control.anchor().position()),
            play_outstanding: false,
            adopted_slot: None,
            closed: false,
            command_bytes: required,
            session_share: profile.limits().events().shares().session_event_share(),
            play_cost: EventCount::measured(play_cost),
            operations: EventCount::measured(0),
        })
    }

    fn validate_offer(
        &self,
        clock: SampleTime,
        at: SampleTime,
        command: SessionCommand,
    ) -> Result<u64, SessionError> {
        if self.closed {
            return Err(SessionError::Closed);
        }
        if at < clock
            || !at.as_u64().is_multiple_of(u64::from(QUANTUM_FRAMES))
            || self.last_offer.is_some_and(|last| at < last)
        {
            return Err(SessionError::Boundary);
        }
        let reserve = usize::from(command == SessionCommand::Play);
        if self.held >= self.slots.len() - reserve {
            return Err(SessionError::Full);
        }
        let mut charge = if command == SessionCommand::Play {
            u64::from(self.play_cost.get())
        } else {
            1
        };
        for offset in self.completed..self.held {
            let index = (self.head + offset) % self.slots.len();
            if let Some(entry) = self.slots.get(index).and_then(Option::as_ref)
                && entry.boundary.at == at
            {
                charge += if entry.boundary.command == SessionCommand::Play {
                    u64::from(self.play_cost.get())
                } else {
                    1
                };
            }
        }
        if charge > u64::from(self.session_share.get()) {
            return Err(SessionError::SessionShare);
        }
        self.serial
            .checked_add(1)
            .ok_or(SessionError::IdentityExhausted)
    }

    fn insert(
        &mut self,
        serial: u64,
        at: SampleTime,
        command: SessionCommand,
        activation: Option<Box<TransportActivation>>,
        capture: Option<crate::recording::TakeReservation>,
    ) -> SessionCommandId {
        let id = SessionCommandId {
            generation: self.generation,
            serial,
        };
        let index = (self.head + self.held) % self.slots.len();
        // Validated capacity makes this the next free slot; this is off-thread.
        self.slots[index] = Some(Box::new(CommandEntry {
            boundary: SessionBoundary {
                id,
                at,
                command,
                capture,
            },
            activation,
            outcome: None,
            capture_outcome: None,
            expected_position: None,
            delivery_error: None,
        }));
        self.held += 1;
        self.serial = serial;
        self.last_offer = Some(at);
        id
    }

    fn offer_play(
        &mut self,
        control: &mut StreamControl,
        clock: SampleTime,
        at: SampleTime,
        capture: Option<crate::recording::TakeReservation>,
    ) -> Result<SessionCommandId, SessionError> {
        let serial = self.validate_offer(clock, at, SessionCommand::Play)?;
        if self.play_outstanding {
            return Err(SessionError::PlayOutstanding);
        }
        let PlaybackState::Stopped(position) = self.state else {
            return Err(SessionError::AlreadyPlaying);
        };
        let stream = self.stream.as_ref().ok_or(SessionError::NotEnabled)?;
        let candidate = control.plan_activation(
            stream,
            ActivationRequest {
                at,
                position,
                loop_interval: None,
            },
        )?;
        self.play_outstanding = true;
        Ok(self.insert(serial, at, SessionCommand::Play, Some(candidate), capture))
    }

    fn collect(
        &mut self,
        control: &mut StreamControl,
    ) -> Result<Option<SessionReceipt>, SessionError> {
        if self.completed == 0 {
            return Ok(None);
        }
        let Some(entry) = self.slots.get_mut(self.head).and_then(Option::as_mut) else {
            return Ok(None);
        };
        let Some(outcome) = entry.outcome else {
            return Ok(None);
        };
        if let Some(candidate) = entry.activation.take() {
            let result = if candidate.effective().is_some() {
                control.adopted(candidate)
            } else {
                control.withdraw(candidate)
            };
            if let Err((candidate, error)) = result {
                entry.activation = Some(candidate);
                return Err(SessionError::Collection(error));
            }
        }
        let receipt = SessionReceipt {
            boundary: entry.boundary,
            outcome,
            capture: entry.capture_outcome.take(),
        };
        if entry.boundary.command == SessionCommand::Play {
            self.play_outstanding = false;
        }
        self.slots[self.head] = None;
        self.head = (self.head + 1) % self.slots.len();
        self.held -= 1;
        self.completed -= 1;
        Ok(Some(receipt))
    }
}

impl SimulatedHost {
    pub(super) fn close_ordered_session(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<(), HostError> {
        let connection = self.connection_mut(generation)?;
        if let Some(prepared) = connection.prepared.as_mut()
            && let Some(session) = prepared.session.as_mut()
        {
            session.close(prepared.renderer.clock())?;
        }
        Ok(())
    }

    pub(super) fn require_session_collected(
        &self,
        generation: ConnectionGeneration,
    ) -> Result<(), HostError> {
        let retained = self
            .active
            .iter()
            .chain(self.candidate.iter())
            .filter(|connection| connection.status.generation == generation)
            .filter_map(|connection| connection.prepared.as_ref())
            .filter_map(|prepared| prepared.session.as_ref())
            .any(|session| {
                session.held != 0
                    || session
                        .sources
                        .as_ref()
                        .is_some_and(source::SourceQueue::has_held)
            });
        if retained {
            Err(SessionError::RetainedOutcomes.into())
        } else {
            Ok(())
        }
    }
    /// Enable ordered musical transport on a fresh Ready output. `start` then starts
    /// device pumping; Play/Stop in this lane control the musical state independently.
    pub fn enable_ordered_transport(
        &mut self,
        generation: ConnectionGeneration,
        stream: AdmittedCompiledStream,
        limits: SessionLimits,
    ) -> Result<(), HostError> {
        if self.note_capture.is_some() {
            return Err(SessionError::FreshStreamRequired.into());
        }
        let connection = self.active_mut(generation)?;
        if connection.status.state != ConnectionState::Ready {
            return Err(HostError::WrongState);
        }
        let prepared = connection.prepared.as_mut().ok_or(HostError::WrongState)?;
        if prepared.session.is_some() {
            return Err(SessionError::AlreadyEnabled.into());
        }
        if prepared.renderer.clock() != SampleTime::ZERO {
            return Err(SessionError::FreshStreamRequired.into());
        }
        let negotiated = connection
            .status
            .negotiated
            .as_ref()
            .ok_or(HostError::WrongState)?;
        let profile = HostProfile::harness(
            negotiated.format.rate,
            negotiated.maximum_callback,
            negotiated.format.layout,
        )?;
        prepared.session = Some(SessionRuntime::prepare(
            generation,
            &mut prepared.control,
            stream,
            &profile,
            limits,
        )?);
        Ok(())
    }

    pub fn session_state(&self) -> Option<PlaybackState> {
        self.active
            .as_ref()?
            .prepared
            .as_ref()?
            .session
            .as_ref()
            .map(|session| session.state)
    }

    pub fn session_command_bytes(&self) -> Option<PreparedBytes> {
        self.active
            .as_ref()?
            .prepared
            .as_ref()?
            .session
            .as_ref()
            .map(|session| session.command_bytes)
    }

    pub fn offer_session_play(
        &mut self,
        generation: ConnectionGeneration,
        at: SampleTime,
    ) -> Result<SessionCommandId, HostError> {
        let connection = self.active_mut(generation)?;
        if !matches!(
            connection.status.state,
            ConnectionState::Ready | ConnectionState::Running
        ) {
            return Err(HostError::WrongState);
        }
        let prepared = connection.prepared.as_mut().ok_or(HostError::WrongState)?;
        let session = prepared.session.as_mut().ok_or(SessionError::NotEnabled)?;
        Ok(session.offer_play(&mut prepared.control, prepared.renderer.clock(), at, None)?)
    }

    pub fn offer_session_stop(
        &mut self,
        generation: ConnectionGeneration,
        at: SampleTime,
    ) -> Result<SessionCommandId, HostError> {
        let capture_ticket = self
            .note_capture
            .as_ref()
            .filter(|recorder| recorder.host_generation == Some(generation))
            .and_then(crate::recording::notes::SimulatedNoteRecorder::active_ticket);
        let connection = self.active_mut(generation)?;
        if !matches!(
            connection.status.state,
            ConnectionState::Ready | ConnectionState::Running
        ) {
            return Err(HostError::WrongState);
        }
        let prepared = connection.prepared.as_mut().ok_or(HostError::WrongState)?;
        let session = prepared.session.as_mut().ok_or(SessionError::NotEnabled)?;
        let serial = session.validate_offer(prepared.renderer.clock(), at, SessionCommand::Stop)?;
        Ok(session.insert(serial, at, SessionCommand::Stop, None, capture_ticket))
    }

    /// Resolve ownership and reclaim the activation off-thread. A stalled reader
    /// does not block effects of commands whose result slots were already reserved.
    pub fn collect_session_receipt(
        &mut self,
        generation: ConnectionGeneration,
    ) -> Result<Option<SessionReceipt>, HostError> {
        if self.active().is_some_and(|status| {
            status.generation == generation && status.state == ConnectionState::Quiescing
        }) {
            self.close_ordered_session(generation)?;
        }
        let connection = self.active_mut(generation)?;
        let prepared = connection.prepared.as_mut().ok_or(HostError::WrongState)?;
        let session = prepared.session.as_mut().ok_or(SessionError::NotEnabled)?;
        Ok(session.collect(&mut prepared.control)?)
    }
}
