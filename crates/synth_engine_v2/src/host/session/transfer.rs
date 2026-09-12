//! Owning command handoff between a compiled session's control and audio halves.
//! The host must retain both halves and queue backing until its backend is quiescent.

mod hot;
pub mod replacement;
#[cfg(test)]
mod tests;

use super::*;
use crate::host::HostError;
use crate::plan::{CompiledPlan, PlanId};
use crate::render::PreparedRenderer;
use crate::time::{PlanPosition, StreamAnchor, StreamEpoch};
use thiserror::Error;

/// Runtime command storage and the controller's separate fixed credit ledger.
/// A host additionally budgets its concrete command and completion queues.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct SessionTransferLimits {
    pub session: SessionLimits,
    pub control_bytes: PreparedBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Origin {
    generation: ConnectionGeneration,
    epoch: StreamEpoch,
    plan: PlanId,
    table: crate::identity::TableId,
}

/// A command reached its destination but cannot take the requested musical effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SessionDeliveryError {
    #[error("command arrived after its boundary; renderer was at {observed}")]
    Late { observed: SampleTime },
    #[error("Play's prepared stopped position changed to {actual}")]
    PositionChanged { actual: PlanPosition },
    #[error("Play reached a session that is no longer stopped")]
    NotStopped,
}

/// A packet could not enter this runtime/collector. The owning packet is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SessionTransferError {
    #[error("packet belongs to another session, epoch, plan or identity table")]
    Origin,
    #[error("packet identity or time is out of order")]
    Order,
    #[error("no runtime command slot is available")]
    Full,
    #[error("session has closed admission")]
    Closed,
    #[error("packet has no matching controller credit")]
    UnknownCommand,
}

/// An observation attached to a completed command, not a hardware timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct SessionSnapshot {
    pub clock: SampleTime,
    pub playback: PlaybackState,
}

/// Prepared off-thread. Ownership must either reach the audio half or return to
/// `SessionControl::cancel`; dropping a packet does not return its reserved credit.
#[derive(Debug)]
#[must_use]
pub struct PreparedSessionCommand {
    origin: Origin,
    entry: Box<CommandEntry>,
}

impl PreparedSessionCommand {
    pub const fn boundary(&self) -> SessionBoundary {
        self.entry.boundary
    }
}

/// The original command and activation, retained until off-thread collection.
#[derive(Debug)]
#[must_use]
pub struct CompletedSessionCommand {
    origin: Origin,
    entry: Box<CommandEntry>,
    snapshot: SessionSnapshot,
}

impl CompletedSessionCommand {
    pub const fn boundary(&self) -> SessionBoundary {
        self.entry.boundary
    }
    pub const fn outcome(&self) -> Option<SessionOutcome> {
        self.entry.outcome
    }
    pub const fn snapshot(&self) -> SessionSnapshot {
        self.snapshot
    }
}

/// Sole mutable owner of the minter and command credits. No method runs on audio.
#[must_use]
pub struct SessionControl {
    control: StreamControl,
    stream: AdmittedCompiledStream,
    origin: Origin,
    credits: Box<[Option<SessionBoundary>]>,
    held: usize,
    serial: u64,
    last_offer: Option<SampleTime>,
    acknowledged_serial: u64,
    acknowledged: SessionSnapshot,
    session_share: EventCount,
    play_cost: EventCount,
    closed: bool,
    bytes: PreparedBytes,
    replacement_credits: [Option<replacement::PlanPublicationId>; replacement::REPLACEMENT_CREDITS],
    publication_serial: u64,
}

/// Sole mutable owner of the scheduler, render state and retained command slots.
/// Moving this into a backend callback alone is insufficient custody: the concrete
/// host must keep backing ownership until that backend's callback worker joins.
#[must_use]
pub struct SessionAudio {
    renderer: PreparedRenderer,
    runtime: SessionRuntime,
    origin: Origin,
    fault: Option<crate::schedule::ScheduledRenderError>,
    publication_serial: u64,
    prepared_minter_obligations: bool,
}

impl SessionControl {
    /// Prepare a fresh compiled-only session. `profile` must match the plan's
    /// device geometry and publication limits. Capture and live ingress have no
    /// constructor on these halves; their serial owners remain separate.
    pub fn prepare(
        plan: CompiledPlan,
        stream: AdmittedCompiledStream,
        profile: HostProfile,
        limits: SessionTransferLimits,
    ) -> Result<(Self, SessionAudio), HostError> {
        Self::prepare_pair(plan, stream, profile, limits, None)
    }

    fn prepare_pair(
        plan: CompiledPlan,
        stream: AdmittedCompiledStream,
        profile: HostProfile,
        limits: SessionTransferLimits,
        previous: Option<&Self>,
    ) -> Result<(Self, SessionAudio), HostError> {
        let caps = profile.capabilities();
        let events = profile.limits().events();
        if plan.sample_rate() != caps.sample_rate()
            || plan.channel_layout() != caps.channel_layout()
            || plan.maximum_block_size() != caps.maximum_block_size()
            || plan.max_events_per_quantum() != events.max_events_per_quantum()
            || plan.compiled_event_share() != events.shares().compiled_event_share()
            || plan.forward_event_horizon() != events.forward_event_horizon()
        {
            return Err(SessionError::TransferProfile.into());
        }
        let count =
            usize::try_from(limits.session.commands.as_u32()).map_err(|_| SessionError::Layout)?;
        let bytes = count
            .checked_mul(size_of::<Option<SessionBoundary>>())
            .and_then(|bytes| bytes.checked_add(size_of::<Self>()))
            .and_then(|bytes| {
                bytes.checked_add(
                    replacement::REPLACEMENT_CREDITS
                        .checked_mul(size_of::<replacement::PreparedPlanReplacement>())?,
                )
            })
            .filter(|bytes| *bytes <= isize::MAX as usize)
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(SessionError::Layout)?;
        let required = PreparedBytes::measured(bytes);
        if required > limits.control_bytes {
            return Err(SessionError::ByteBudget {
                required,
                available: limits.control_bytes,
            }
            .into());
        }
        let mut credits = Vec::new();
        credits
            .try_reserve_exact(count)
            .map_err(|_| SessionError::Allocation)?;
        credits.resize_with(count, || None);
        let (generation, mut control, renderer) = if let Some(previous) = previous {
            let (control, renderer) = previous.control.replace_compiled(plan)?;
            (previous.origin.generation, control, renderer)
        } else {
            let generation = super::super::issue_generation(&super::super::NEXT_GENERATION)?;
            let (control, renderer) = StreamControl::open(
                plan,
                StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
            )?;
            (generation, control, renderer)
        };
        let origin = Origin {
            generation,
            epoch: control.epoch(),
            plan: control.plan_id(),
            table: control.table_id(),
        };
        let runtime = SessionRuntime::prepare_detached(
            generation,
            &mut control,
            &stream,
            &profile,
            limits.session,
        )?;
        let prepared_minter_obligations = control.has_replacement_obligations();
        let owner = Self {
            control,
            stream,
            origin,
            credits: credits.into_boxed_slice(),
            held: 0,
            serial: 0,
            last_offer: None,
            acknowledged_serial: 0,
            acknowledged: SessionSnapshot {
                clock: SampleTime::ZERO,
                playback: runtime.state,
            },
            session_share: runtime.session_share,
            play_cost: runtime.play_cost,
            closed: false,
            bytes: required,
            replacement_credits: [None; replacement::REPLACEMENT_CREDITS],
            publication_serial: 0,
        };
        Ok((
            owner,
            SessionAudio {
                renderer,
                runtime,
                origin,
                fault: None,
                publication_serial: 0,
                prepared_minter_obligations,
            },
        ))
    }

    pub const fn snapshot(&self) -> SessionSnapshot {
        self.acknowledged
    }
    pub const fn generation(&self) -> ConnectionGeneration {
        self.origin.generation
    }
    pub const fn epoch(&self) -> StreamEpoch {
        self.origin.epoch
    }
    pub const fn plan_id(&self) -> PlanId {
        self.origin.plan
    }
    pub const fn control_bytes(&self) -> PreparedBytes {
        self.bytes
    }
    pub const fn has_outstanding(&self) -> bool {
        self.held != 0
    }

    pub fn close_admission(&mut self) {
        self.closed = true;
    }

    fn validate(
        &self,
        at: SampleTime,
        command: SessionCommand,
    ) -> Result<(usize, u64), SessionError> {
        if self.closed {
            return Err(SessionError::Closed);
        }
        if self.has_replacements() {
            return Err(SessionError::ReplacementOutstanding);
        }
        if at < self.acknowledged.clock
            || !at.as_u64().is_multiple_of(u64::from(QUANTUM_FRAMES))
            || self.last_offer.is_some_and(|last| at < last)
        {
            return Err(SessionError::Boundary);
        }
        let reserve = usize::from(command == SessionCommand::Play);
        if self.held >= self.credits.len() - reserve {
            return Err(SessionError::Full);
        }
        let mut charge = if command == SessionCommand::Play {
            u64::from(self.play_cost.get())
        } else {
            1
        };
        for entry in self.credits.iter().flatten() {
            if entry.command == SessionCommand::Play && command == SessionCommand::Play {
                return Err(SessionError::PlayOutstanding);
            }
            if entry.at == at {
                charge += if entry.command == SessionCommand::Play {
                    u64::from(self.play_cost.get())
                } else {
                    1
                };
            }
        }
        if charge > u64::from(self.session_share.get()) {
            return Err(SessionError::SessionShare);
        }
        let index = self
            .credits
            .iter()
            .position(Option::is_none)
            .ok_or(SessionError::Full)?;
        let serial = self
            .serial
            .checked_add(1)
            .ok_or(SessionError::IdentityExhausted)?;
        Ok((index, serial))
    }

    fn packet(
        &mut self,
        index: usize,
        serial: u64,
        at: SampleTime,
        command: SessionCommand,
        activation: Option<Box<TransportActivation>>,
        expected_position: Option<PlanPosition>,
    ) -> PreparedSessionCommand {
        let boundary = SessionBoundary {
            id: SessionCommandId {
                generation: self.origin.generation,
                serial,
            },
            at,
            command,
            capture: None,
        };
        let packet = PreparedSessionCommand {
            origin: self.origin,
            entry: Box::new(CommandEntry {
                boundary,
                activation,
                outcome: None,
                capture_outcome: None,
                expected_position,
                delivery_error: None,
            }),
        };
        // validate selected this private free slot before any minter mutation.
        self.credits[index] = Some(boundary);
        self.held += 1;
        self.serial = serial;
        self.last_offer = Some(at);
        packet
    }

    pub fn prepare_play(&mut self, at: SampleTime) -> Result<PreparedSessionCommand, SessionError> {
        let (index, serial) = self.validate(at, SessionCommand::Play)?;
        let PlaybackState::Stopped(position) = self.acknowledged.playback else {
            return Err(SessionError::AlreadyPlaying);
        };
        let candidate = self.control.plan_activation(
            &self.stream,
            ActivationRequest {
                at,
                position,
                loop_interval: None,
            },
        )?;
        Ok(self.packet(
            index,
            serial,
            at,
            SessionCommand::Play,
            Some(candidate),
            Some(position),
        ))
    }

    pub fn prepare_stop(&mut self, at: SampleTime) -> Result<PreparedSessionCommand, SessionError> {
        let (index, serial) = self.validate(at, SessionCommand::Stop)?;
        Ok(self.packet(index, serial, at, SessionCommand::Stop, None, None))
    }

    fn credit(&self, origin: Origin, boundary: SessionBoundary) -> Result<usize, SessionError> {
        if origin != self.origin {
            return Err(SessionTransferError::Origin.into());
        }
        self.credits
            .iter()
            .position(|entry| *entry == Some(boundary))
            .ok_or_else(|| SessionTransferError::UnknownCommand.into())
    }

    fn resolve(&mut self, entry: &mut CommandEntry) -> Result<(), SessionError> {
        if let Some(candidate) = entry.activation.take() {
            let result = if candidate.effective().is_some() {
                self.control.adopted(candidate)
            } else {
                self.control.withdraw(candidate)
            };
            if let Err((candidate, error)) = result {
                entry.activation = Some(candidate);
                return Err(SessionError::Collection(error));
            }
        }
        Ok(())
    }

    /// Return a packet that never entered the audio runtime, including a failed send.
    pub fn cancel(
        &mut self,
        mut packet: PreparedSessionCommand,
    ) -> Result<SessionReceipt, (PreparedSessionCommand, SessionError)> {
        let index = match self.credit(packet.origin, packet.entry.boundary) {
            Ok(index) => index,
            Err(error) => return Err((packet, error)),
        };
        if let Err(error) = self.resolve(&mut packet.entry) {
            return Err((packet, error));
        }
        self.credits[index] = None;
        self.held -= 1;
        Ok(SessionReceipt {
            boundary: packet.entry.boundary,
            outcome: SessionOutcome::Cancelled,
            capture: None,
        })
    }

    /// Resolve the activation off-thread before releasing its command credit.
    /// Out-of-order collection cannot rewind the acknowledged playback snapshot.
    pub fn collect(
        &mut self,
        mut packet: CompletedSessionCommand,
    ) -> Result<SessionReceipt, (CompletedSessionCommand, SessionError)> {
        let index = match self.credit(packet.origin, packet.entry.boundary) {
            Ok(index) => index,
            Err(error) => return Err((packet, error)),
        };
        let Some(outcome) = packet.entry.outcome else {
            return Err((packet, SessionTransferError::UnknownCommand.into()));
        };
        if let Err(error) = self.resolve(&mut packet.entry) {
            return Err((packet, error));
        }
        self.credits[index] = None;
        self.held -= 1;
        if packet.entry.boundary.id.serial() > self.acknowledged_serial {
            self.acknowledged_serial = packet.entry.boundary.id.serial();
            self.acknowledged = packet.snapshot;
        }
        Ok(SessionReceipt {
            boundary: packet.entry.boundary,
            outcome,
            capture: packet.entry.capture_outcome.take(),
        })
    }
}

impl SessionAudio {
    /// Call only after the host has fenced callback access. All unconsumed
    /// commands become retained cancellations; collection remains available.
    pub fn close_after_quiescence(&mut self) -> Result<(), SessionError> {
        self.runtime.close(self.renderer.clock())
    }
}
