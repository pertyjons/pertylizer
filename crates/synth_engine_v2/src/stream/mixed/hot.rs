//! The bound mixed audio half's quantum-boundary operation.
//! The renderer, ended-note storage, both range minters and the compiled token table
//! are prepared off-thread. This path mints and releases compiled occurrences in that
//! prepared storage only; it does not allocate, lock, perform I/O, log or drop an
//! owning candidate.

#[cfg(all(test, feature = "simulated-ingress"))]
use super::EmbeddedQueues;
#[cfg(feature = "mixed-transfer-rehearsal")]
use super::transfer::{
    MixedTransferAudioPort, MixedTransferCommand, MixedTransferConsumer, MixedTransferProducer,
    MixedTransferResult,
};
use super::{
    MixedCandidateOffer, MixedCompiledEvent, MixedCompiledPayload, MixedCompiledTokenState,
    MixedCompiledTransport, MixedEffectiveEvents, MixedEffectiveTimeError, MixedLoopCursor,
    MixedNoteToken, MixedOfferAdopted, MixedOfferCell, MixedOfferKind, MixedOfferOutcome,
    MixedOfferReceipt, MixedOfferRefused, MixedOfferSlot, MixedOfferSlotState, MixedOneShotAudio,
    MixedOneShotRenderError, MixedOneShotRenderReport, MixedRetiredOffer, MixedStopAccepted,
    MixedStopRefused, MixedStreamAudio,
};
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use super::{MixedIngressCommand, MixedIngressCommandFault, MixedIngressResult};
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use crate::ingress::{RELEASE_GROUP_CAPACITY, ReleaseCause, ReleaseGroup};
use crate::{
    identity::{CompiledRangeMinter, EndedNote, NoteIdentity, Resolution},
    looping::LoopPassId,
    publish::ProducerClass,
    quantities::{EventCount, HeldNoteCount, QuantumCount},
    render::{AudioBlockMut, EventEnvelope, EventPayload, NoteEdge, Renderer, TimedEvent},
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor, TimeSource},
    transport::LoopInterval,
};
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
use std::sync::atomic::Ordering;

/// How the audio half reaches its command consumer and result producer at a quantum
/// boundary: no queue, the private rehearsal's own rings, or a host's lent endpoint.
/// Every implementation is bounded, nonblocking, allocation-free and panic-free.
pub(crate) trait MixedAudioQueues {
    /// Whether ADR-0078's transfer rules fault a packet stamped before the last served
    /// one or beyond the forward horizon, rather than leaving the store to refuse it.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    const TRANSFER_RULES: bool;

    /// Whether this endpoint was bound to `audio` at arm. Checked before any queue use.
    fn bound(&self, audio: &MixedOneShotAudio) -> bool;

    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    fn pop_command(&mut self, audio: &mut MixedOneShotAudio) -> Option<MixedIngressCommand>;

    /// Push one result, or give back the exact result the queue refused.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    fn push_result(
        &mut self,
        audio: &mut MixedOneShotAudio,
        result: MixedIngressResult,
    ) -> Option<MixedIngressResult>;
}

/// No command source: the renderer-only private path.
#[cfg(not(all(test, feature = "simulated-ingress")))]
pub(crate) struct NoMixedQueues;

#[cfg(not(all(test, feature = "simulated-ingress")))]
impl MixedAudioQueues for NoMixedQueues {
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    const TRANSFER_RULES: bool = false;

    fn bound(&self, _audio: &MixedOneShotAudio) -> bool {
        true
    }

    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    fn pop_command(&mut self, _audio: &mut MixedOneShotAudio) -> Option<MixedIngressCommand> {
        None
    }

    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    fn push_result(
        &mut self,
        _audio: &mut MixedOneShotAudio,
        result: MixedIngressResult,
    ) -> Option<MixedIngressResult> {
        Some(result)
    }
}

/// A host's lent audio endpoint. Its binding is compared before either half is touched;
/// each packet moves through the host's qualified queue exactly once.
#[cfg(feature = "mixed-transfer-rehearsal")]
impl<C, R> MixedAudioQueues for MixedTransferAudioPort<C, R>
where
    C: MixedTransferConsumer<MixedTransferCommand>,
    R: MixedTransferProducer<MixedTransferResult>,
{
    const TRANSFER_RULES: bool = true;

    fn bound(&self, audio: &MixedOneShotAudio) -> bool {
        audio.transfer_binding == Some(self.binding)
    }

    fn pop_command(&mut self, _audio: &mut MixedOneShotAudio) -> Option<MixedIngressCommand> {
        self.commands
            .try_pop()
            .map(MixedTransferCommand::into_inner)
    }

    fn push_result(
        &mut self,
        audio: &mut MixedOneShotAudio,
        result: MixedIngressResult,
    ) -> Option<MixedIngressResult> {
        // The boundary serving this command; the result carries it to the report.
        let served_at = audio.audio.renderer.clock();
        match self
            .results
            .try_push(MixedTransferResult::new(result, served_at))
        {
            Ok(()) => None,
            Err(refused) => Some(refused.into_inner()),
        }
    }
}

impl MixedNoteToken {
    pub(crate) fn as_usize(self) -> usize {
        usize::from(self.0)
    }
}

impl MixedCompiledEvent {
    pub(crate) const fn new(envelope: EventEnvelope, payload: MixedCompiledPayload) -> Self {
        Self { envelope, payload }
    }

    pub(crate) const fn envelope(&self) -> EventEnvelope {
        self.envelope
    }

    pub(crate) const fn payload(&self) -> MixedCompiledPayload {
        self.payload
    }
}

impl MixedEffectiveEvents<'_> {
    /// The callback's checked read stays inside the purity scan's hot region.
    pub(super) fn shifted(
        &self,
        event_index: usize,
        event: MixedCompiledEvent,
    ) -> Result<MixedCompiledEvent, MixedEffectiveTimeError> {
        let envelope = event.envelope();
        let time = envelope.time().checked_add(self.shift).map_err(|_| {
            MixedEffectiveTimeError::EventTimeUnrepresentable {
                event_index,
                time: envelope.time(),
                shift: self.shift,
            }
        })?;
        Ok(MixedCompiledEvent::new(
            EventEnvelope::new(envelope.epoch(), time, envelope.source()),
            event.payload(),
        ))
    }

    pub(crate) fn get(
        &self,
        event_index: usize,
    ) -> Result<Option<MixedCompiledEvent>, MixedEffectiveTimeError> {
        match self.events.get(event_index).copied() {
            Some(event) => self.shifted(event_index, event).map(Some),
            None => Ok(None),
        }
    }
}

/// What publishing one compiled event produced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum MixedCompiledPublished {
    Event(TimedEvent),
    /// A Panic had ended the token's occurrence; its `Off` freed the token only.
    ReleasedAfterPanic,
    /// A Panic had ended the token's occurrence; its expression or bend went nowhere.
    ControlAfterPanic,
}

#[cfg(test)]
impl MixedCompiledPublished {
    pub(super) const fn event(self) -> Option<TimedEvent> {
        match self {
            Self::Event(event) => Some(event),
            Self::ReleasedAfterPanic | Self::ControlAfterPanic => None,
        }
    }
}

/// One applied compiled Panic cut.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MixedCompiledPanicCut {
    pub(super) ended: HeldNoteCount,
    pub(super) groups: EventCount,
}

/// What one loop wrap ended in the compiled token table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MixedCompiledWrap {
    /// Open occurrences ended by a Session-charged `Off` at the wrap.
    pub(super) ended: HeldNoteCount,
    /// Panic tombstones freed, their `Off` consumed rather than published.
    pub(super) after_panic: EventCount,
}

/// The plan position a running loop plays at engine time `at`: linear from the entry
/// anchor `(B, P)` up to the first wrap at `B + end - P`, then `start` plus the time
/// since that wrap modulo the loop's length. `None` before the anchor or where the
/// arithmetic cannot be represented. Pure, so audio and a host preparing an exit agree.
pub(super) fn loop_position(
    anchor: StreamAnchor,
    interval: LoopInterval,
    at: SampleTime,
) -> Option<PlanPosition> {
    let start = interval.start().as_u64();
    let length = interval.length().as_u64();
    if length == 0 {
        return None;
    }
    let elapsed = at.as_u64().checked_sub(anchor.time().as_u64())?;
    let into = anchor
        .position()
        .as_u64()
        .checked_sub(start)?
        .checked_add(elapsed)?;
    start.checked_add(into % length).map(PlanPosition::new)
}

/// One quantum's loop-mapped plan positions from `clock`, on the stack.
fn loop_positions(
    anchor: StreamAnchor,
    interval: LoopInterval,
    clock: SampleTime,
) -> Option<[PlanPosition; QUANTUM_FRAMES as usize]> {
    let mut positions = [interval.start(); QUANTUM_FRAMES as usize];
    for frame in 0..QUANTUM_FRAMES as usize {
        let at = clock.checked_add(FrameCount::new(frame as u64)).ok()?;
        *positions.get_mut(frame)? = loop_position(anchor, interval, at)?;
    }
    Some(positions)
}

/// Whether `token` is a Panic tombstone, or the occurrence an open one names.
enum CompiledTokenUse {
    Open(NoteIdentity),
    PanicEnded,
}

impl MixedStreamAudio {
    /// What a compiled edge other than a note-on finds at its token. A free token is a
    /// pairing defect.
    fn compiled_token_use(
        &self,
        token: MixedNoteToken,
    ) -> Result<CompiledTokenUse, MixedOneShotRenderError> {
        match self.compiled_tokens.get(token.as_usize()).copied() {
            Some(MixedCompiledTokenState::Open(identity)) => Ok(CompiledTokenUse::Open(identity)),
            Some(MixedCompiledTokenState::PanicEnded) => Ok(CompiledTokenUse::PanicEnded),
            Some(MixedCompiledTokenState::Free) | None => {
                Err(MixedOneShotRenderError::CompiledToken { token })
            }
        }
    }

    /// Give one tokenized compiled event the occurrence it names, as it publishes.
    ///
    /// A note-on mints by the compiled range's lowest-free rule at this point in the
    /// applied event order, so the identity depends on what has published, never on a
    /// candidate that was prepared, withdrawn or refused. Its release frees the index
    /// here, in the same order the renderer applies the edges. An edge for a token a
    /// Panic ended publishes nothing: its `Off` frees the token, and its expression or
    /// bend is consumed; both are counted, never faulted. A note-on can never meet such
    /// a token, since pairing puts the `Off` first.
    pub(super) fn publish_compiled(
        &mut self,
        event: MixedCompiledEvent,
    ) -> Result<MixedCompiledPublished, MixedOneShotRenderError> {
        let payload = match event.payload() {
            MixedCompiledPayload::Restore(scoped) => EventPayload::ScopedRestore(scoped),
            MixedCompiledPayload::On {
                token,
                slot,
                key,
                velocity,
            } => {
                if self.compiled_tokens.get(token.as_usize()).copied()
                    != Some(MixedCompiledTokenState::Free)
                {
                    return Err(MixedOneShotRenderError::CompiledToken { token });
                }
                let identity = self
                    .compiled_minter
                    .mint_keyed(slot, key)
                    .map_err(|source| MixedOneShotRenderError::CompiledMint { source })?;
                if let Some(cell) = self.compiled_tokens.get_mut(token.as_usize()) {
                    *cell = MixedCompiledTokenState::Open(identity);
                }
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::On {
                        slot,
                        key,
                        velocity,
                    },
                }
            }
            MixedCompiledPayload::Off(token) => {
                let used = self.compiled_token_use(token)?;
                if let Some(cell) = self.compiled_tokens.get_mut(token.as_usize()) {
                    *cell = MixedCompiledTokenState::Free;
                }
                let CompiledTokenUse::Open(identity) = used else {
                    return Ok(MixedCompiledPublished::ReleasedAfterPanic);
                };
                let resolution = self.compiled_minter.release(identity);
                if resolution != Resolution::Live {
                    return Err(MixedOneShotRenderError::CompiledRelease { resolution });
                }
                EventPayload::Note {
                    identity,
                    edge: NoteEdge::Off,
                }
            }
            MixedCompiledPayload::Expression(token, expression) => {
                match self.compiled_token_use(token)? {
                    CompiledTokenUse::Open(identity) => EventPayload::Expression {
                        identity,
                        expression,
                    },
                    CompiledTokenUse::PanicEnded => {
                        return Ok(MixedCompiledPublished::ControlAfterPanic);
                    }
                }
            }
            MixedCompiledPayload::Bend(token, cents) => match self.compiled_token_use(token)? {
                CompiledTokenUse::Open(identity) => EventPayload::Bend { identity, cents },
                CompiledTokenUse::PanicEnded => {
                    return Ok(MixedCompiledPublished::ControlAfterPanic);
                }
            },
        };
        Ok(MixedCompiledPublished::Event(TimedEvent::new(
            event.envelope(),
            payload,
        )))
    }

    /// End every open compiled occurrence at `at`, a Panic's execution sample, as
    /// Release-charged groups of at most eight in token order, and leave each token a
    /// Panic tombstone. The compiled minter frees each identity here, in applied order,
    /// so a later compiled onset may take an index at its next generation; the renderer
    /// releases the old generation at `at` before it admits the new one.
    ///
    /// Every charge is preflighted against the release share before any token, identity
    /// or charge changes. Arm proved the share covers one cut of the whole span beside
    /// the full live backlog, so a refusal here is a terminal defect, not a shortage.
    /// A stopped program holds no open token, so it publishes nothing.
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    pub(super) fn end_compiled_for_panic(
        &mut self,
        at: SampleTime,
        publication: &mut crate::publish::Publication<'_>,
    ) -> Result<MixedCompiledPanicCut, MixedOneShotRenderError> {
        let mut open = 0_usize;
        let mut first = None;
        for token in &self.compiled_tokens {
            if let MixedCompiledTokenState::Open(identity) = *token {
                open += 1;
                if first.is_none() {
                    first = Some(identity);
                }
            }
        }
        let groups = open.div_ceil(RELEASE_GROUP_CAPACITY);
        let class = ReleaseCause::CompiledPanic.class();
        let spent = publication.spent(crate::publish::WindowRow::FIRST, class);
        let share = publication.share(class);
        let needed = EventCount::measured(u32::try_from(groups).unwrap_or(u32::MAX));
        if spent.checked_add(needed).is_none_or(|total| total > share) {
            return Err(MixedOneShotRenderError::CompiledPanicShare {
                spent,
                groups: needed,
                share,
            });
        }
        let mut cut = MixedCompiledPanicCut {
            ended: HeldNoteCount::NONE,
            groups: EventCount::NONE,
        };
        let Some(first) = first else {
            return Ok(cut);
        };
        let envelope = EventEnvelope::new(self.renderer.epoch(), at, TimeSource::Compiled);
        let mut members = [first; RELEASE_GROUP_CAPACITY];
        let mut count = 0;
        for index in 0..self.compiled_tokens.len() {
            let Some(MixedCompiledTokenState::Open(identity)) =
                self.compiled_tokens.get(index).copied()
            else {
                continue;
            };
            let resolution = self.compiled_minter.release(identity);
            if resolution != Resolution::Live {
                return Err(MixedOneShotRenderError::CompiledRelease { resolution });
            }
            if let Some(token) = self.compiled_tokens.get_mut(index) {
                *token = MixedCompiledTokenState::PanicEnded;
            }
            if let Some(member) = members.get_mut(count) {
                *member = identity;
            }
            count += 1;
            cut.ended = HeldNoteCount::measured(cut.ended.get().saturating_add(1));
            if count == RELEASE_GROUP_CAPACITY {
                charge_compiled_panic_group(publication, envelope, members.get(..count))?;
                cut.groups = EventCount::measured(cut.groups.get().saturating_add(1));
                count = 0;
            }
        }
        if count > 0 {
            charge_compiled_panic_group(publication, envelope, members.get(..count))?;
            cut.groups = EventCount::measured(cut.groups.get().saturating_add(1));
        }
        Ok(cut)
    }

    /// End the pass at a loop wrap at `at`, as ADR-0065's exclusive loop does but scoped
    /// to the compiled producer: every open compiled occurrence gets a Session-charged
    /// `Off` at `at`, in token order, and leaves the compiled minter; every Panic
    /// tombstone is freed and its owed `Off` consumed. Then the wrap's one Session
    /// operation. The token table is empty afterwards, which the next pass's template
    /// assumes. No live occurrence, row, ramp or minter is visited, and nothing is
    /// seeded: the next pass's restoration writes compiled rows as ordinary writes.
    pub(super) fn wrap_compiled(
        &mut self,
        at: SampleTime,
        publication: &mut crate::publish::Publication<'_>,
    ) -> Result<MixedCompiledWrap, MixedOneShotRenderError> {
        let envelope = EventEnvelope::new(self.renderer.epoch(), at, TimeSource::Compiled);
        let mut wrap = MixedCompiledWrap {
            ended: HeldNoteCount::NONE,
            after_panic: EventCount::NONE,
        };
        for index in 0..self.compiled_tokens.len() {
            match self.compiled_tokens.get(index).copied() {
                Some(MixedCompiledTokenState::Open(identity)) => {
                    publication.charge(
                        ProducerClass::Session,
                        TimedEvent::new(
                            envelope,
                            EventPayload::Note {
                                identity,
                                edge: NoteEdge::Off,
                            },
                        ),
                    )?;
                    let resolution = self.compiled_minter.release(identity);
                    if resolution != Resolution::Live {
                        return Err(MixedOneShotRenderError::CompiledRelease { resolution });
                    }
                    wrap.ended = HeldNoteCount::measured(wrap.ended.get().saturating_add(1));
                }
                Some(MixedCompiledTokenState::PanicEnded) => {
                    wrap.after_panic =
                        EventCount::measured(wrap.after_panic.get().saturating_add(1));
                }
                Some(MixedCompiledTokenState::Free) | None => continue,
            }
            if let Some(token) = self.compiled_tokens.get_mut(index) {
                *token = MixedCompiledTokenState::Free;
            }
        }
        publication.charge_operation(ProducerClass::Session, at)?;
        Ok(wrap)
    }

    /// Compiled tokens a Panic ended whose `Off` has not published yet.
    fn compiled_panic_tombstones(&self) -> HeldNoteCount {
        let tombstones = self
            .compiled_tokens
            .iter()
            .filter(|token| **token == MixedCompiledTokenState::PanicEnded)
            .count();
        HeldNoteCount::measured(u32::try_from(tombstones).unwrap_or(u32::MAX))
    }

    /// Release, in the audio-owned compiled minter, exactly the occurrences the
    /// renderer's boundary release ended, and close every token of the retired list.
    ///
    /// The minter and the registry must then agree that no compiled occurrence remains:
    /// a token the registry did not end, or an ended note the minter did not hold, is a
    /// terminal custody fault. The successor list's tokens start from this empty table.
    pub(super) fn end_compiled_custody(
        &mut self,
        released: HeldNoteCount,
    ) -> Result<(), MixedOneShotRenderError> {
        end_compiled_custody(
            &mut self.compiled_minter,
            &mut self.compiled_tokens,
            &self.compiled_ended,
            released,
        )
    }

    /// Move only the compiled musical mapping at the current render boundary.
    /// Scratch was sized from the bound compiled span off-thread. The returned
    /// anchor belongs with the retired compiled list; a later schedule owner
    /// must also publish complete scoped restoration in this same quantum.
    #[allow(dead_code)] // Only the private mixed callback calls this so far.
    pub(crate) fn adopt_compiled_boundary(
        &mut self,
        effective: SampleTime,
        position: PlanPosition,
    ) -> Result<crate::render::MixedBoundaryAdopted, crate::render::MixedBoundaryReleaseError> {
        self.renderer.adopt_mixed_compiled_boundary(
            self.partition.compiled_producer(),
            effective,
            position,
            &mut self.compiled_ended,
        )
    }
}

/// Charge one group of a compiled Panic cut to the release share.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
fn charge_compiled_panic_group(
    publication: &mut crate::publish::Publication<'_>,
    envelope: EventEnvelope,
    members: Option<&[NoteIdentity]>,
) -> Result<(), MixedOneShotRenderError> {
    let group = members
        .and_then(ReleaseGroup::compiled_panic)
        .ok_or(MixedOneShotRenderError::CompiledPanicGroup)?;
    publication.charge(
        ReleaseCause::CompiledPanic.class(),
        TimedEvent::new(envelope, EventPayload::ReleaseGroup(group)),
    )?;
    Ok(())
}

/// Add one applied cut to the owner's cumulative compiled Panic counts.
#[cfg(any(
    all(test, feature = "simulated-ingress"),
    feature = "mixed-transfer-rehearsal"
))]
fn count_compiled_panic(
    ended: &mut HeldNoteCount,
    groups: &mut EventCount,
    cut: MixedCompiledPanicCut,
) {
    *ended = HeldNoteCount::measured(ended.get().saturating_add(cut.ended.get()));
    *groups = EventCount::measured(groups.get().saturating_add(cut.groups.get()));
}

/// Release in the compiled minter exactly the `released` prefix of `ended`, and close
/// every compiled token, including a Panic tombstone, whose `Off` belongs to the list
/// this boundary retires or suspends. The minter and the registry must then agree that
/// no compiled occurrence remains; disagreement is a terminal custody fault.
fn end_compiled_custody(
    minter: &mut CompiledRangeMinter,
    tokens: &mut [MixedCompiledTokenState],
    ended: &[Option<EndedNote>],
    released: HeldNoteCount,
) -> Result<(), MixedOneShotRenderError> {
    let ended = released
        .as_usize()
        .and_then(|count| ended.get(..count))
        .ok_or(MixedOneShotRenderError::CompiledCustody { live: released })?;
    for note in ended {
        let Some(note) = note else {
            return Err(MixedOneShotRenderError::CompiledCustody { live: released });
        };
        let resolution = minter.release(note.identity);
        if resolution != Resolution::Live {
            return Err(MixedOneShotRenderError::CompiledRelease { resolution });
        }
    }
    tokens.fill(MixedCompiledTokenState::Free);
    let live = minter.live();
    if live != 0 {
        return Err(MixedOneShotRenderError::CompiledCustody {
            live: HeldNoteCount::measured(live),
        });
    }
    Ok(())
}

#[cfg(feature = "mixed-transfer-rehearsal")]
impl MixedTransferCommand {
    pub(crate) fn into_inner(self) -> MixedIngressCommand {
        self.0
    }
}

#[cfg(feature = "mixed-transfer-rehearsal")]
impl MixedTransferResult {
    pub(crate) const fn new(result: MixedIngressResult, served_at: SampleTime) -> Self {
        Self(result, served_at)
    }

    pub(crate) fn into_inner(self) -> MixedIngressResult {
        self.0
    }
}

#[allow(dead_code)] // Private callback remains unreachable by production hosts.
impl MixedOneShotAudio {
    #[cfg(any(
        all(test, feature = "simulated-ingress"),
        feature = "mixed-transfer-rehearsal"
    ))]
    fn ingress_halt_error(&self) -> MixedOneShotRenderError {
        match self.ingress_command_fault {
            Some(MixedIngressCommandFault::ForeignEndpoint) => {
                MixedOneShotRenderError::IngressForeignEndpoint
            }
            Some(MixedIngressCommandFault::BackwardTime) => {
                MixedOneShotRenderError::IngressBackwardTime
            }
            Some(MixedIngressCommandFault::BeyondHorizon) => {
                MixedOneShotRenderError::IngressBeyondHorizon
            }
            Some(MixedIngressCommandFault::ForeignCommand | MixedIngressCommandFault::Sequence) => {
                MixedOneShotRenderError::IngressCommandOrder
            }
            None if self.invalid_ingress_command.is_some() => {
                MixedOneShotRenderError::IngressCommandOrder
            }
            None if self.ingress_result_pending.is_some() => {
                MixedOneShotRenderError::IngressResultPush
            }
            None => MixedOneShotRenderError::IngressTransferHalt,
        }
    }

    /// A lent endpoint of another transfer: terminal before any queue is touched. The
    /// endpoint stays with its host owner; the complete callback is silent.
    fn refuse_foreign_endpoint(
        &mut self,
        output: &mut AudioBlockMut<'_>,
    ) -> Result<(), MixedOneShotRenderError> {
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        self.refuse_ingress_service(MixedIngressCommandFault::ForeignEndpoint, None);
        let error = MixedOneShotRenderError::IngressForeignEndpoint;
        self.fault = Some(error);
        self.audio.renderer.terminal_mixed_fault(output);
        Err(error)
    }

    /// Read the private owner's state and charges without moving its capsule.
    pub(crate) fn report(&self) -> MixedOneShotRenderReport {
        MixedOneShotRenderReport {
            effective_anchor: self.effective_anchor,
            retired_anchor: self.capsule.retired_anchor,
            sequence: self.in_force,
            adopted: self.adopted,
            faulted: self.fault.is_some(),
            fault: self.fault,
            releases_charged: self.releases_charged,
            released_compiled: self.released_compiled,
            restoration_charged: self.restoration_charged,
            suffix_charged: self.suffix_charged,
            completed_quanta: self.completed_quanta,
            boundary_quantum_completed: self
                .adoption_after_quanta
                .is_some_and(|at| self.completed_quanta > at),
            transport: self.transport,
            stop_pending: self.stop_pending,
            stops_charged: self.stops_charged,
            offer: match &self.offer_slot {
                MixedOfferSlot::Empty => MixedOfferSlotState::Empty,
                MixedOfferSlot::Pending(offer) => MixedOfferSlotState::Pending {
                    supersedes: offer.0.candidate.supersedes,
                    effective: offer.0.timing.effective(),
                    kind: offer.0.kind,
                },
                MixedOfferSlot::Retired(retired) => MixedOfferSlotState::Retired(retired.receipt),
            },
            compiled_panic_ended: self.compiled_panic_ended,
            compiled_panic_groups: self.compiled_panic_groups,
            compiled_panic_tombstones: self.audio.compiled_panic_tombstones(),
            compiled_released_after_panic: self.compiled_released_after_panic,
            compiled_controls_after_panic: self.compiled_controls_after_panic,
            #[cfg(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            ))]
            panic_parked: self
                .parked_panic
                .as_ref()
                .map(|command| command.request.nominal()),
            #[cfg(not(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            )))]
            panic_parked: None,
            loop_interval: self.loop_program.as_deref().map(|program| program.interval),
            loop_pass: self.loop_cursor.map(|cursor| cursor.pass),
            loop_wraps: self.loop_wraps,
            loop_wrap_releases: self.loop_wrap_releases,
        }
    }

    /// Accept one sealed later candidate into the single offer slot, or give it back.
    ///
    /// Every check runs here, and again when the candidate's boundary opens, before
    /// anything changes: a refusal leaves the program in force, both minters, the
    /// registry and the slot as they were, returns the owned candidate, and is counted.
    /// An occupied slot refuses whether it holds a pending candidate or a retired one
    /// control has not taken, so one boundary takes at most one transition. Callable
    /// between callbacks on the audio thread: it moves one box and allocates nothing.
    pub(crate) fn offer_candidate(
        &mut self,
        offer: MixedCandidateOffer,
    ) -> Result<(), (MixedOfferRefused, MixedCandidateOffer)> {
        let clock = self.audio.renderer.clock();
        let refused = if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare()
        {
            Some(MixedOfferRefused::Faulted)
        } else if self.offer_is_foreign(&offer.0) {
            Some(MixedOfferRefused::ForeignCandidate)
        } else {
            match &self.offer_slot {
                MixedOfferSlot::Pending(_) => Some(MixedOfferRefused::AlreadyPending),
                MixedOfferSlot::Retired(_) => Some(MixedOfferRefused::RetiredUncollected),
                MixedOfferSlot::Empty => self.offer_boundary_refusal(&offer.0, clock),
            }
        };
        if let Some(reason) = refused {
            self.audio.renderer.count_refused_activation();
            return Err((reason, offer));
        }
        let mut offer = offer;
        offer.0.late = offer.0.candidate.anchor.time() < clock;
        self.offer_slot = MixedOfferSlot::Pending(offer);
        Ok(())
    }

    /// Move the retired offer and its receipt out of the slot, emptying it. Allocates
    /// and frees nothing; control drops the retired list off-thread.
    pub(crate) fn take_retired_offer(&mut self) -> Option<MixedRetiredOffer> {
        match core::mem::take(&mut self.offer_slot) {
            MixedOfferSlot::Retired(retired) => Some(retired),
            other => {
                self.offer_slot = other;
                None
            }
        }
    }

    fn offer_is_foreign(&self, cell: &MixedOfferCell) -> bool {
        let renderer = &self.audio.renderer;
        cell.candidate.plan != renderer.plan().id()
            || cell.candidate.epoch != renderer.epoch()
            || cell.candidate.table != renderer.table_id()
    }

    /// The sequence, timing and transport checks shared by the offer and its boundary.
    /// The sequence in force at the candidate's boundary is the armed one while that is
    /// still pending. A pending Stop at or before the boundary refuses the candidate
    /// in either arrival order: offered after the Stop here, or cancelled by it when it
    /// applies. A Start needs the program stopped at its boundary, which only an
    /// applied Stop gives, since any pending Stop is then after the boundary.
    fn offer_boundary_refusal(
        &self,
        cell: &MixedOfferCell,
        clock: SampleTime,
    ) -> Option<MixedOfferRefused> {
        let in_force = if self.adopted {
            self.in_force
        } else {
            self.capsule.sequence
        };
        let supersedes = cell.candidate.supersedes;
        let effective = cell.timing.effective();
        let boundary = self.timing.effective();
        if supersedes != in_force {
            Some(MixedOfferRefused::Superseded {
                supersedes,
                in_force,
            })
        } else if effective <= boundary {
            Some(MixedOfferRefused::NotAfterInForce {
                effective,
                boundary,
            })
        } else if effective < clock {
            Some(MixedOfferRefused::BoundaryPassed { effective, clock })
        } else if let Some(stop) = self.stop_pending
            && effective >= stop
        {
            Some(MixedOfferRefused::StopPending { stop, effective })
        } else if cell.kind == MixedOfferKind::Start
            && self.transport == MixedCompiledTransport::Running
        {
            Some(MixedOfferRefused::StartWhileRunning)
        } else {
            None
        }
    }

    /// Applying a Stop at `clock` cancels a candidate waiting at or after it (draft
    /// Item 2, step 3). The same allocation retires into the slot with a cancelled
    /// receipt; nothing is dropped, minted or released.
    fn cancel_pending_offer(&mut self, clock: SampleTime) {
        let cancels = matches!(
            &self.offer_slot,
            MixedOfferSlot::Pending(offer) if offer.0.timing.effective() >= clock
        );
        if !cancels {
            return;
        }
        match core::mem::take(&mut self.offer_slot) {
            MixedOfferSlot::Pending(offer) => {
                let cell = &offer.0;
                let receipt = MixedOfferReceipt {
                    supersedes: cell.candidate.supersedes,
                    sequence: cell.candidate.sequence,
                    late: cell.late,
                    kind: cell.kind,
                    outcome: MixedOfferOutcome::Cancelled { stop: clock },
                };
                self.offer_slot = MixedOfferSlot::Retired(MixedRetiredOffer {
                    cell: offer.0,
                    receipt,
                });
            }
            other => self.offer_slot = other,
        }
    }

    /// At a quantum boundary, take a pending candidate that is due: validate it again,
    /// then end the old program's sounding compiled occurrences, swap the program and
    /// anchor, and leave its scoped restoration at the head of the list in force, so
    /// it publishes before any destination edge. A Start then resumes the stopped
    /// program; a seek leaves the transport as it is, so a stopped program publishes
    /// only that restoration. Live occurrences, rows and ramps are outside the release.
    /// A refusal retires the candidate with a refused receipt and changes nothing
    /// else. A terminal fault leaves the candidate in its slot for off-thread teardown
    /// rather than dropping it here.
    fn take_due_offer(&mut self, clock: SampleTime) -> Result<(), MixedOneShotRenderError> {
        let due = matches!(
            &self.offer_slot,
            MixedOfferSlot::Pending(offer) if offer.0.timing.effective() <= clock
        );
        if !due {
            return Ok(());
        }
        let mut offer = match core::mem::take(&mut self.offer_slot) {
            MixedOfferSlot::Pending(offer) => offer,
            other => {
                self.offer_slot = other;
                return Ok(());
            }
        };
        let refused = if self.offer_is_foreign(&offer.0) {
            Some(MixedOfferRefused::ForeignCandidate)
        } else {
            self.offer_boundary_refusal(&offer.0, clock)
        };
        let candidate = &offer.0.candidate;
        let mut receipt = MixedOfferReceipt {
            supersedes: candidate.supersedes,
            sequence: candidate.sequence,
            late: offer.0.late,
            kind: offer.0.kind,
            outcome: MixedOfferOutcome::Refused(MixedOfferRefused::Faulted),
        };
        if let Some(reason) = refused {
            self.audio.renderer.count_refused_activation();
            receipt.outcome = MixedOfferOutcome::Refused(reason);
            self.offer_slot = MixedOfferSlot::Retired(MixedRetiredOffer {
                cell: offer.0,
                receipt,
            });
            return Ok(());
        }
        let effective = offer.0.timing.effective();
        let position = candidate.anchor.position();
        // A loop's first pass ends at `B + end - P`. Seal proved it representable; a
        // failure here is a defect, checked before anything moves.
        let loop_cursor = match offer.0.loop_program.as_deref() {
            None => None,
            Some(program) => {
                let first_wrap = program
                    .interval
                    .end()
                    .as_u64()
                    .checked_sub(position.as_u64())
                    .filter(|frames| *frames > 0)
                    .and_then(|frames| effective.checked_add(FrameCount::new(frames)).ok());
                let Some(next_wrap) = first_wrap else {
                    self.offer_slot = MixedOfferSlot::Pending(offer);
                    return Err(MixedOneShotRenderError::LoopTimeline);
                };
                Some(MixedLoopCursor {
                    pass: LoopPassId::FIRST,
                    next_wrap,
                    repeating: false,
                    pass_shift: FrameCount::ZERO,
                })
            }
        };
        let producer = self.audio.partition.compiled_producer();
        let adopted = match self.audio.renderer.adopt_mixed_compiled_boundary(
            producer,
            effective,
            position,
            &mut offer.0.ended,
        ) {
            Ok(adopted) => adopted,
            Err(error) => {
                self.offer_slot = MixedOfferSlot::Pending(offer);
                return Err(MixedOneShotRenderError::Boundary(error));
            }
        };
        offer.0.released = adopted.released();
        if let Err(error) = end_compiled_custody(
            &mut self.audio.compiled_minter,
            &mut self.audio.compiled_tokens,
            &offer.0.ended,
            adopted.released(),
        ) {
            self.offer_slot = MixedOfferSlot::Pending(offer);
            return Err(error);
        }
        let cell = &mut *offer.0;
        core::mem::swap(&mut self.events, &mut cell.candidate.events);
        // The retired loop moves into the cell with its list; nothing drops here.
        core::mem::swap(&mut self.loop_program, &mut cell.loop_program);
        let retired_cursor = self.loop_cursor;
        self.loop_cursor = loop_cursor;
        // A loop that wrapped had spent its entry list; the cursor read its template.
        let retired_next = match retired_cursor {
            Some(cursor) if cursor.repeating => cell.candidate.events.len(),
            _ => self.next,
        };
        self.next = 0;
        self.timing = cell.timing;
        let anchor = StreamAnchor::new(effective, position);
        self.effective_anchor = anchor;
        self.in_force = cell.candidate.sequence;
        self.in_force_restoration = cell.candidate.restoration_count;
        self.release_due = true;
        match (cell.kind, &mut self.transport) {
            (MixedOfferKind::Start, _) => self.transport = MixedCompiledTransport::Running,
            // A stopped seek moves the held playhead to its destination.
            (MixedOfferKind::Seek, MixedCompiledTransport::Stopped { position: held, .. }) => {
                *held = Some(position)
            }
            (MixedOfferKind::Seek, MixedCompiledTransport::Running) => {}
        }
        cell.candidate.retired_anchor = Some(adopted.retired_anchor());
        cell.candidate.retired_next = Some(retired_next);
        receipt.outcome = MixedOfferOutcome::Adopted(MixedOfferAdopted {
            effective,
            anchor,
            retired_anchor: adopted.retired_anchor(),
            retired_unconsumed_from: retired_next,
            retired_event_count: cell.candidate.events.len(),
            released: adopted.released(),
            stopped: self.transport != MixedCompiledTransport::Running,
            loop_interval: self.loop_program.as_deref().map(|program| program.interval),
            retired_pass: retired_cursor.map(|cursor| cursor.pass),
        });
        self.offer_slot = MixedOfferSlot::Retired(MixedRetiredOffer {
            cell: offer.0,
            receipt,
        });
        Ok(())
    }

    /// Schedule a compiled-scoped Stop at the quantum boundary `at`, applied before
    /// that quantum's compiled events by every callback partition alike. It ends only
    /// compiled custody; live notes keep sounding and may start while stopped.
    ///
    /// A later candidate pending before `at` adopts first, so a seek moves the program
    /// the Stop then ends and a Start resumes the program it then stops. One pending at
    /// or after `at` is cancelled when the Stop applies, since one boundary takes one
    /// transport change. A Stop while stopped is acknowledged and changes nothing,
    /// unless it has a pending candidate to cancel or a pending Start to follow.
    pub(crate) fn schedule_compiled_stop(
        &mut self,
        at: SampleTime,
    ) -> Result<MixedStopAccepted, MixedStopRefused> {
        if self.fault.is_some() || self.audio.renderer.diagnostics().needs_reprepare() {
            return Err(MixedStopRefused::Faulted);
        }
        if let Some(pending) = self.stop_pending {
            return Err(MixedStopRefused::Pending { pending });
        }
        if at.quantum_offset() != crate::time::QuantumOffset::ZERO {
            return Err(MixedStopRefused::NotBoundary { at });
        }
        let clock = self.audio.renderer.clock();
        if at < clock {
            return Err(MixedStopRefused::Past { at, clock });
        }
        let effective = self.timing.effective();
        if !self.adopted && at <= effective {
            return Err(MixedStopRefused::ActivationPending { at, effective });
        }
        if self.transport != MixedCompiledTransport::Running {
            let interacts = matches!(
                &self.offer_slot,
                MixedOfferSlot::Pending(offer)
                    if offer.0.timing.effective() >= at || offer.0.kind == MixedOfferKind::Start
            );
            if !interacts {
                return Ok(MixedStopAccepted::AlreadyStopped);
            }
        }
        self.stop_pending = Some(at);
        Ok(MixedStopAccepted::Scheduled)
    }

    /// Apply a Stop due at `clock`, before any later candidate's boundary. It cancels a
    /// candidate waiting at or after `clock`, then, if the program runs, ends its
    /// compiled custody and owes one Session operation. The renderer's release count is
    /// recorded before custody ends, so a fault there leaves teardown the record. A
    /// Stop that finds the program stopped only cancels.
    fn apply_compiled_stop(&mut self, clock: SampleTime) -> Result<(), MixedOneShotRenderError> {
        if self.stop_pending != Some(clock) {
            return Ok(());
        }
        if !self.adopted {
            return Err(MixedOneShotRenderError::StopBeforeAdoption);
        }
        self.cancel_pending_offer(clock);
        if self.transport == MixedCompiledTransport::Running {
            // The playhead this boundary plays, before anything moves: the loop's mapped
            // position while a loop runs, else the anchor's. A stopped loop never wraps;
            // the next adoption replaces its program.
            let position = match (self.loop_cursor, self.loop_program.as_deref()) {
                (Some(_), Some(program)) => Some(
                    loop_position(self.effective_anchor, program.interval, clock)
                        .ok_or(MixedOneShotRenderError::LoopTimeline)?,
                ),
                _ => self.audio.renderer.plan_position_at_clock(),
            };
            self.stop_released = HeldNoteCount::NONE;
            let released = self.audio.renderer.stop_mixed_compiled_boundary(
                self.audio.partition.compiled_producer(),
                clock,
                &mut self.stop_ended,
            )?;
            self.stop_released = released;
            end_compiled_custody(
                &mut self.audio.compiled_minter,
                &mut self.audio.compiled_tokens,
                &self.stop_ended,
                released,
            )?;
            self.transport = MixedCompiledTransport::Stopped {
                at: clock,
                released,
                position,
            };
            self.stop_due = true;
        }
        self.stop_pending = None;
        Ok(())
    }

    /// Render the sealed private schedule in at most one quantum per publication.
    /// Every terminal failure silences the caller's complete block and retains the box.
    /// The private rehearsal serves its own embedded rings; other builds have none.
    pub(crate) fn render_private(
        &mut self,
        output: AudioBlockMut<'_>,
    ) -> Result<(), MixedOneShotRenderError> {
        #[cfg(all(test, feature = "simulated-ingress"))]
        let mut queues = EmbeddedQueues;
        #[cfg(not(all(test, feature = "simulated-ingress")))]
        let mut queues = NoMixedQueues;
        self.render_with(&mut queues, output)
    }

    /// [`Self::render_private`] serving the command queue `queues` reaches. A foreign
    /// endpoint faults before any queue is touched, and every transfer fault this or the
    /// control half publishes silences the complete callback that observes it.
    pub(crate) fn render_with<Q: MixedAudioQueues>(
        &mut self,
        queues: &mut Q,
        mut output: AudioBlockMut<'_>,
    ) -> Result<(), MixedOneShotRenderError> {
        if self.fault.is_some() {
            output.silence();
            return Err(MixedOneShotRenderError::Faulted);
        }
        // The halt read precedes the endpoint check, as on the control half.
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        if self.ingress_halt.load(Ordering::Acquire) {
            let error = self.ingress_halt_error();
            self.fault = Some(error);
            self.audio.renderer.terminal_mixed_fault(&mut output);
            return Err(error);
        }
        if !queues.bound(self) {
            return self.refuse_foreign_endpoint(&mut output);
        }
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        if self.invalid_ingress_command.is_some() {
            self.fault = Some(MixedOneShotRenderError::IngressCommandOrder);
            self.audio.renderer.terminal_mixed_fault(&mut output);
            return Err(MixedOneShotRenderError::IngressCommandOrder);
        }
        let renderer = &self.audio.renderer;
        if output.layout() != renderer.plan().channel_layout()
            || u64::try_from(output.frames()).map_or(true, |frames| {
                frames > renderer.plan().maximum_block_size().as_u64()
            })
        {
            return Err(MixedOneShotRenderError::OutputShape);
        }
        if output.frames() == 0 {
            return Ok(());
        }
        #[cfg(all(test, feature = "simulated-ingress"))]
        {
            self.test_ingress_inflight_len = 0;
        }
        self.render_started = true;
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        let result = self.render_private_inner(queues, &mut output);
        #[cfg(not(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        )))]
        let result = self.render_private_inner(&mut output);
        if let Err(error) = result {
            self.fault = Some(error);
            match error {
                MixedOneShotRenderError::Publication(_) => {
                    self.audio.renderer.terminal_fault(&mut output);
                }
                MixedOneShotRenderError::Displacement(_) => {
                    self.audio.renderer.terminal_displacement_fault(&mut output);
                }
                _ => self.audio.renderer.terminal_mixed_fault(&mut output),
            }
            return Err(error);
        }
        #[cfg(all(test, feature = "simulated-ingress"))]
        {
            self.test_ingress_inflight_len = 0;
        }
        Ok(())
    }

    fn render_private_inner(
        &mut self,
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        queues: &mut impl MixedAudioQueues,
        output: &mut AudioBlockMut<'_>,
    ) -> Result<(), MixedOneShotRenderError> {
        let renderer = &self.audio.renderer;
        if renderer.plan().id() != self.capsule.plan
            || renderer.epoch() != self.capsule.epoch
            || renderer.table_id() != self.capsule.table
        {
            return Err(MixedOneShotRenderError::Pairing);
        }
        if renderer.diagnostics().needs_reprepare() {
            return Err(MixedOneShotRenderError::RendererFaulted);
        }
        let mut offset = 0;
        while offset < output.frames() {
            let carry = self.audio.renderer.carry_frames();
            let remaining = output.frames() - offset;
            let frames = remaining.min(if carry == 0 {
                QUANTUM_FRAMES as usize
            } else {
                carry
            });
            let Some(block) = output.window(offset, frames) else {
                return Err(MixedOneShotRenderError::InternalOutputWindow);
            };
            #[cfg(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            ))]
            self.render_private_step(queues, block, carry == 0)?;
            #[cfg(not(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            )))]
            self.render_private_step(block, carry == 0)?;
            offset += frames;
        }
        Ok(())
    }

    fn render_private_step(
        &mut self,
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        queues: &mut impl MixedAudioQueues,
        output: AudioBlockMut<'_>,
        opens_quantum: bool,
    ) -> Result<(), MixedOneShotRenderError> {
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        if self.ingress_halt.load(Ordering::Acquire) {
            return Err(self.ingress_halt_error());
        }
        let clock = self.audio.renderer.clock();
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        if opens_quantum {
            self.panic_current_service_at
                .store(clock.as_u64(), Ordering::Release);
        }
        if opens_quantum && !self.adopted && clock == self.timing.effective() {
            let adopted = self.audio.adopt_compiled_boundary(
                self.timing.effective(),
                self.effective_anchor.position(),
            )?;
            self.capsule.retired_anchor = Some(adopted.retired_anchor());
            self.capsule.retired_next = Some(self.next);
            self.released_compiled = adopted.released();
            self.audio.end_compiled_custody(adopted.released())?;
            core::mem::swap(&mut self.events, &mut self.capsule.events);
            self.next = 0;
            self.in_force = self.capsule.sequence;
            self.in_force_restoration = self.capsule.restoration_count;
            self.release_due = true;
            self.adopted = true;
            self.adoption_after_quanta = Some(self.completed_quanta);
        }
        // A Stop due here cancels any candidate due here before it could adopt.
        if opens_quantum {
            self.apply_compiled_stop(clock)?;
            self.take_due_offer(clock)?;
        }
        let stopped = matches!(self.transport, MixedCompiledTransport::Stopped { .. });

        // Serve only at a quantum boundary. The command prefix is fixed before
        // opening publication, which borrows the arbiter through the render.
        #[cfg(any(
            all(test, feature = "simulated-ingress"),
            feature = "mixed-transfer-rehearsal"
        ))]
        if opens_quantum {
            self.service_ingress_queue(queues);
            if self.ingress_halt.load(Ordering::Acquire) || self.invalid_ingress_command.is_some() {
                return Err(self.ingress_halt_error());
            }
        }
        let mut publication = self.arbiter.open(clock, usize::from(opens_quantum))?;
        let mut next = self.next;
        if opens_quantum {
            let restoration = self
                .in_force_restoration
                .as_usize()
                .ok_or(MixedOneShotRenderError::RestorationCount)?;
            // Each adoption's boundary release is one Session operation in its quantum.
            if self.release_due {
                publication.charge_operation(ProducerClass::Session, clock)?;
                self.release_due = false;
                self.releases_charged =
                    EventCount::measured(self.releases_charged.get().saturating_add(1));
            }
            // Arm proved one Session operation fits every quantum after adoption's, and
            // a Stop never shares its boundary with an adoption.
            if self.stop_due {
                publication.charge_operation(ProducerClass::Session, clock)?;
                self.stop_due = false;
                self.stops_charged =
                    EventCount::measured(self.stops_charged.get().saturating_add(1));
            }
            // A Panic served at this boundary ends the compiled producer's open
            // occurrences after every compiled event stamped at or before its sample and
            // before any later one, so equal stamps keep Item 2's compiled-then-live order.
            #[cfg(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            ))]
            let mut panic_cut = self.panic_cut.take();
            // A stopped program publishes only the scoped restoration at the head of a
            // list it adopted while stopped; every later event stays unconsumed.
            loop {
                if stopped && next >= restoration {
                    break;
                }
                // A running loop's cursor, read again after every wrap; a stopped
                // program never wraps.
                let looping = if stopped { None } else { self.loop_cursor };
                // After a wrap the cursor reads the template, shifted to that wrap. Both
                // lists start with the same restoration groups, so `restoration` serves
                // either.
                let read = match self.loop_cursor.filter(|cursor| cursor.repeating) {
                    Some(cursor) => MixedEffectiveEvents {
                        events: &self
                            .loop_program
                            .as_deref()
                            .ok_or(MixedOneShotRenderError::LoopProgram)?
                            .events,
                        shift: cursor.pass_shift,
                    }
                    .get(next)?,
                    None if self.adopted => MixedEffectiveEvents {
                        events: &self.events,
                        shift: self.timing.shift(),
                    }
                    .get(next)?,
                    None => self.events.get(next).copied(),
                };
                let Some(event) = read else {
                    // The pass is spent, so its wrap is the next compiled step. It runs
                    // at its own sample, after every earlier event and before the next
                    // pass's events at that sample.
                    let Some(cursor) = looping else {
                        break;
                    };
                    let at = cursor.next_wrap;
                    if at < clock {
                        return Err(MixedOneShotRenderError::MissedEvent { event: at, clock });
                    }
                    if at.quantum_index() != clock.quantum_index() {
                        break;
                    }
                    // A Panic before the wrap ends the pass's notes first; one at the
                    // wrap's sample runs after the next pass's events there.
                    #[cfg(any(
                        all(test, feature = "simulated-ingress"),
                        feature = "mixed-transfer-rehearsal"
                    ))]
                    if let Some(cut_at) = panic_cut
                        && at > cut_at
                    {
                        let cut = self
                            .audio
                            .end_compiled_for_panic(cut_at, &mut publication)?;
                        count_compiled_panic(
                            &mut self.compiled_panic_ended,
                            &mut self.compiled_panic_groups,
                            cut,
                        );
                        panic_cut = None;
                    }
                    let length = self
                        .loop_program
                        .as_deref()
                        .ok_or(MixedOneShotRenderError::LoopProgram)?
                        .interval
                        .length();
                    let pass = cursor
                        .pass
                        .next()
                        .ok_or(MixedOneShotRenderError::LoopPassExhausted)?;
                    let next_wrap = at
                        .checked_add(length)
                        .map_err(|_| MixedOneShotRenderError::LoopTimeline)?;
                    let wrap = self.audio.wrap_compiled(at, &mut publication)?;
                    self.loop_wraps = EventCount::measured(self.loop_wraps.get().saturating_add(1));
                    self.loop_wrap_releases = HeldNoteCount::measured(
                        self.loop_wrap_releases
                            .get()
                            .saturating_add(wrap.ended.get()),
                    );
                    self.compiled_released_after_panic = EventCount::measured(
                        self.compiled_released_after_panic
                            .get()
                            .saturating_add(wrap.after_panic.get()),
                    );
                    self.loop_cursor = Some(MixedLoopCursor {
                        pass,
                        next_wrap,
                        repeating: true,
                        pass_shift: FrameCount::new(at.as_u64()),
                    });
                    next = 0;
                    continue;
                };
                let at = event.envelope().time();
                if at < clock {
                    return Err(MixedOneShotRenderError::MissedEvent { event: at, clock });
                }
                if let Some(cursor) = looping
                    && at >= cursor.next_wrap
                {
                    return Err(MixedOneShotRenderError::LoopEventPastWrap {
                        event: at,
                        wrap: cursor.next_wrap,
                    });
                }
                if at.quantum_index() != clock.quantum_index() {
                    break;
                }
                #[cfg(any(
                    all(test, feature = "simulated-ingress"),
                    feature = "mixed-transfer-rehearsal"
                ))]
                if let Some(cut_at) = panic_cut
                    && at > cut_at
                {
                    let cut = self
                        .audio
                        .end_compiled_for_panic(cut_at, &mut publication)?;
                    count_compiled_panic(
                        &mut self.compiled_panic_ended,
                        &mut self.compiled_panic_groups,
                        cut,
                    );
                    panic_cut = None;
                }
                let event = match self.audio.publish_compiled(event)? {
                    MixedCompiledPublished::Event(event) => event,
                    MixedCompiledPublished::ReleasedAfterPanic => {
                        self.compiled_released_after_panic = EventCount::measured(
                            self.compiled_released_after_panic.get().saturating_add(1),
                        );
                        next += 1;
                        continue;
                    }
                    MixedCompiledPublished::ControlAfterPanic => {
                        self.compiled_controls_after_panic = EventCount::measured(
                            self.compiled_controls_after_panic.get().saturating_add(1),
                        );
                        next += 1;
                        continue;
                    }
                };
                let class = if next < restoration {
                    ProducerClass::Session
                } else {
                    ProducerClass::Compiled
                };
                publication.charge(class, event)?;
                if class == ProducerClass::Session {
                    self.restoration_charged =
                        EventCount::measured(self.restoration_charged.get().saturating_add(1));
                } else if self.adopted {
                    self.suffix_charged =
                        EventCount::measured(self.suffix_charged.get().saturating_add(1));
                }
                next += 1;
            }
            #[cfg(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            ))]
            if let Some(cut_at) = panic_cut {
                let cut = self
                    .audio
                    .end_compiled_for_panic(cut_at, &mut publication)?;
                count_compiled_panic(
                    &mut self.compiled_panic_ended,
                    &mut self.compiled_panic_groups,
                    cut,
                );
            }
            #[cfg(any(
                all(test, feature = "simulated-ingress"),
                feature = "mixed-transfer-rehearsal"
            ))]
            {
                #[cfg(all(test, feature = "simulated-ingress"))]
                let (offset, before) = {
                    let offset = self.test_ingress_inflight_len;
                    let destination = self
                        .test_ingress_inflight
                        .get_mut(offset..)
                        .ok_or(MixedOneShotRenderError::IngressJournal)?;
                    let before = self
                        .test_ingress
                        .copy_queue_for_test(destination)
                        .ok_or(MixedOneShotRenderError::IngressJournal)?;
                    (offset, before)
                };
                let bends_before = self.test_ingress.mixed_bend_queued();
                let charged = self.test_ingress.drain_into(
                    &mut publication,
                    self.audio.renderer.diagnostics_mut(),
                    clock,
                );
                #[cfg(all(test, feature = "simulated-ingress"))]
                {
                    self.test_ingress_inflight_len = offset.saturating_add(
                        before.saturating_sub(self.test_ingress.ring_len_for_test()),
                    );
                }
                // Each bend entry that left the queue returns its credit to the merger.
                self.bend_credit_return
                    .give_back(super::MixedIngressQueueCount(
                        bends_before.saturating_sub(self.test_ingress.mixed_bend_queued()),
                    ));
                charged?;
            }
            #[cfg(test)]
            if let Some(event) = self.test_live
                && !self.test_live_spent
            {
                let at = event.envelope().time();
                if at < clock {
                    return Err(MixedOneShotRenderError::MissedEvent { event: at, clock });
                }
                if at.quantum_index() == clock.quantum_index() {
                    publication.charge(ProducerClass::Live, event)?;
                    self.test_live_spent = true;
                }
            }
        }
        #[cfg(all(test, feature = "simulated-ingress"))]
        if opens_quantum && self.test_fail_after_ingress_at == Some(clock) {
            return Err(MixedOneShotRenderError::InjectedAfterIngress);
        }
        // A stopped program holds the plan timeline for the whole quantum, so no
        // callback partition lets it advance while live notes and tails render.
        match self.transport {
            // A running loop maps each new quantum's frames through its wraps, so the
            // plan timeline wraps at the same samples as its events. A step that opens no
            // quantum only delivers carry and reads no timeline. Live voices read this
            // transport-owned timeline too, as they do across a seek or a Stop; a wrap
            // moves no live note, row or ramp.
            MixedCompiledTransport::Running => match (opens_quantum, self.loop_cursor) {
                (true, Some(_)) => {
                    let interval = self
                        .loop_program
                        .as_deref()
                        .ok_or(MixedOneShotRenderError::LoopProgram)?
                        .interval;
                    let positions = loop_positions(self.effective_anchor, interval, clock)
                        .ok_or(MixedOneShotRenderError::LoopTimeline)?;
                    self.audio.renderer.render_loop_quantum(
                        output,
                        publication.seal().events(),
                        &positions,
                    )?
                }
                _ => self
                    .audio
                    .renderer
                    .render(output, publication.seal().events())?,
            },
            MixedCompiledTransport::Stopped { position, .. } => {
                self.audio
                    .renderer
                    .render_held(output, publication.seal().events(), position)?
            }
        }
        self.next = next;
        if opens_quantum {
            self.completed_quanta =
                QuantumCount::measured(self.completed_quanta.get().saturating_add(1));
        }
        Ok(())
    }
}
