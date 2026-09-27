//! Reduced two-source custody model with one local raw-input/recorder bridge probe.

use std::collections::VecDeque;

use super::{
    EventPayload, MixedCollection, MixedCollectionEnd, MixedIngressCommandId, MixedIngressOriginId,
    MixedIngressOutcome, MixedIngressRequest, MixedIngressResult, MixedIngressSubmitError,
    MixedOneShotControl, MixedOneShotRenderError, history_tests,
};
use crate::identity::NoteIdentity;
use crate::ingress::{ExhaustedResource, IngressRefused};
use crate::{
    host::{
        ConnectionGeneration, EndpointId,
        input::{
            InputCapacity, InputCaptureSession, InputError, InputEventId, InputLimits,
            InputObservation, InputOutcome, InputRate, InputReceipt, InputTick, InputTickSpan,
            SimulatedInputClock, SimulatedNoteInput,
        },
        session::{
            LoopRecordingSession, SessionCommand, SessionCommandCapacity, SessionLimits,
            SessionSourceCapacity, SessionSourceLimits, SessionSourceOutcome,
        },
    },
    profile::{CaptureLimits, CaptureLimitsInput, RecordingLimits},
    quantities::{
        CapturePassCount, CaptureResultCount, CaptureSourceCount, ChannelLayout, EventCount,
        HeldNoteCount, KeyIdentity, NoteVelocity, PreparedBytes, ProjectionTickCount, SampleRate,
        TrackedInputNoteCount,
    },
    recording::CaptureOutcome,
    recording::notes::{
        AuditionTrace, CaptureDisposition, CaptureMode, CaptureQuantization, CaptureStamp,
        ControllerSnapshot, FixtureRevision, FixtureTargetId, Midi1Event, Midi1Input,
        MusicalInterval, NoteArmContext, NoteArmInput, SimulatedNoteRecorder,
        loop_capture::{LoopCaptureSession, tests as loop_test_support},
    },
    render::{AudioBlockMut, NoteEdge},
    tempo::{Bpm, MusicalTick, TempoMap},
    time::{FrameCount, PlanPosition, SampleTime, StreamAnchor, StreamEpoch, issue_epoch},
};
use synth_core::MidiChannel;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelSource {
    First,
    Second,
}

impl ModelSource {
    const fn index(self) -> usize {
        match self {
            Self::First => 0,
            Self::Second => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OccurrenceId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Key {
    source: ModelSource,
    channel: MidiChannel,
    note: KeyIdentity,
}

#[derive(Debug)]
struct Attempt {
    id: OccurrenceId,
    key: Key,
    input: Midi1Input,
    mapped_at: Option<SampleTime>,
}

enum RingPacket {
    Onset(Attempt),
    Ordinary,
    Release {
        id: OccurrenceId,
        key: Key,
        input: Midi1Input,
        mapped_at: Option<SampleTime>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoCreditReason {
    SourceReserve,
    RawCapture,
    Tracker,
    Ingress,
    Results,
    Ledger,
    OccurrenceIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminalReason {
    NoCredit(NoCreditReason),
    Order,
    ReleaseIdentity,
    MissingReleaseStamp,
    IngressRelease(IngressRefused),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CauseOrigin {
    SourceLocal,
    SharedState,
}

impl TerminalReason {
    const fn origin(self) -> CauseOrigin {
        match self {
            Self::NoCredit(NoCreditReason::SourceReserve | NoCreditReason::RawCapture)
            | Self::Order
            | Self::MissingReleaseStamp => CauseOrigin::SourceLocal,
            Self::NoCredit(_) | Self::ReleaseIdentity | Self::IngressRelease(_) => {
                CauseOrigin::SharedState
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerminalFault {
    original: Midi1Input,
    reason: TerminalReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OnsetPreflight {
    Ready,
    Retry,
    NoCredit(NoCreditReason),
    Order,
    Halted,
}

struct SourceHold {
    id: OccurrenceId,
    key: Key,
    mapped_at: Option<SampleTime>,
    release_queued: bool,
    retired: bool,
}

#[derive(Debug, PartialEq)]
struct RetryToken {
    id: OccurrenceId,
    source: ModelSource,
}

#[derive(Debug, PartialEq)]
enum RetryError {
    Stale(RetryToken),
    Blocked(RetryToken),
    Halted(RetryToken),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReleaseAttemptId(u64);

#[derive(Debug, PartialEq)]
struct ReleaseRetryToken {
    id: ReleaseAttemptId,
    source: ModelSource,
}

#[derive(Debug, PartialEq)]
enum ReleaseRetryError {
    Stale(ReleaseRetryToken),
    Blocked(ReleaseRetryToken),
    Halted(ReleaseRetryToken),
    MissingStamp(ReleaseRetryToken),
}

struct PendingRelease {
    id: ReleaseAttemptId,
    input: Midi1Input,
    mapped_at: Option<SampleTime>,
}

struct PendingRetry {
    attempt: Attempt,
    charge: Counts,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Retirement {
    id: OccurrenceId,
    key: Key,
    input: Midi1Input,
    returned: Counts,
}

#[derive(Debug)]
enum SubmitError {
    NoCredit(Midi1Input, NoCreditReason),
    Retry(RetryToken),
    Order(Midi1Input),
    Halted(Midi1Input),
}

#[derive(Debug, PartialEq)]
enum ReleaseOfferError {
    Blocked(ReleaseRetryToken),
    Order(Midi1Input),
    Halted(Midi1Input),
    Unmatched(Midi1Input),
    IdentityExhausted(Midi1Input),
    MissingStamp(Midi1Input),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ReleaseOffer {
    Queued(OccurrenceId),
    Retired(OccurrenceId),
    Unmatched(Midi1Input),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReleaseRoute {
    id: OccurrenceId,
    raw: bool,
    ingress: bool,
}

struct Entry {
    id: OccurrenceId,
    key: Key,
    input: Midi1Input,
    mapped_at: Option<SampleTime>,
    source_release: Option<(Midi1Input, Option<SampleTime>)>,
    released: bool,
    raw_held: bool,
    raw_onset: Option<InputEventId>,
    raw_release: Option<InputEventId>,
    ingress_held: bool,
    ingress_disposition: IngressDisposition,
    ingress_command: Option<(MixedIngressCommandId, MixedIngressRequest)>,
    ingress_release_command: Option<(MixedIngressCommandId, MixedIngressRequest)>,
    ingress_release_result: Option<Result<(), IngressRefused>>,
    result_held: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IngressResultBindError {
    State,
    WrongCommand,
    WrongRequest,
    WrongOutcome,
    DuplicateIdentity,
}

#[derive(Debug, PartialEq)]
enum IngressReleaseSubmitError {
    State,
    TerminalRefused(IngressRefused),
    Submit(MixedIngressSubmitError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IngressDisposition {
    Pending,
    Accepted(NoteIdentity),
    Refused(IngressRefused),
    FakeRefused,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RawReceiptSettlementError {
    Unreleased,
    BoundToRealInput,
    UnboundOnset,
    WrongOnset,
    WrongRelease,
    Undelivered,
    WrongOccurrence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RawOnsetBindError {
    State,
    Duplicate,
    SourceGeneration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Counts {
    tracker: usize,
    ingress: usize,
    results: usize,
    ledger: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RawShadow {
    occupied: usize,
    queued_onsets: usize,
    queued_ordinary: usize,
    release_reservations: usize,
    capacity: InputCapacity,
}

impl RawShadow {
    fn new(capacity: InputCapacity) -> Self {
        Self {
            occupied: 1,
            queued_onsets: 0,
            queued_ordinary: 0,
            release_reservations: 0,
            capacity,
        }
    }

    fn capacity_size(self) -> usize {
        usize::try_from(self.capacity.as_u32()).expect("fixture target represents input cells")
    }

    fn can_charge_onset(self) -> bool {
        self.occupied
            .checked_add(self.queued_onsets)
            .and_then(|count| count.checked_add(self.queued_ordinary))
            .and_then(|count| count.checked_add(self.release_reservations))
            .zip(self.capacity_size().checked_sub(4))
            .is_some_and(|(claimed, limit)| claimed <= limit)
    }

    fn can_charge_ordinary(self) -> bool {
        let Some(claimed) = self
            .occupied
            .checked_add(self.queued_onsets)
            .and_then(|count| count.checked_add(self.queued_ordinary))
            .and_then(|count| count.checked_add(self.release_reservations))
        else {
            return false;
        };
        let protected = if self.release_reservations > 0 { 3 } else { 2 };
        self.capacity_size()
            .checked_sub(protected)
            .is_some_and(|limit| claimed <= limit)
    }
}

/// Onset queue custody acquires four shared credits, its ring cell and one
/// future release cell. Consumers return only their own credits after their
/// outcomes; removing a source packet does not redeem its release reserve.
struct Model {
    rings: [VecDeque<RingPacket>; 2],
    merge_frontiers: [Option<SampleTime>; 2],
    merge_last_submitted: [Option<SampleTime>; 2],
    release_reservations: [usize; 2],
    source_holds: Vec<SourceHold>,
    retry_pending: [Option<PendingRetry>; 2],
    release_pending: [Option<PendingRelease>; 2],
    host_halted: bool,
    terminal_fault: Option<(ModelSource, TerminalFault)>,
    entries: Vec<Entry>,
    raw_binding: [Option<(ConnectionGeneration, u64)>; 2],
    raw_shadow: Option<[RawShadow; 2]>,
    next: u64,
    next_release: u64,
    used: Counts,
    limits: Counts,
    ring_limit: usize,
}

impl Model {
    fn new(limits: Counts, ring_limit: usize) -> Self {
        Self {
            rings: [VecDeque::new(), VecDeque::new()],
            merge_frontiers: [None; 2],
            merge_last_submitted: [None; 2],
            release_reservations: [0; 2],
            source_holds: Vec::new(),
            retry_pending: [None, None],
            release_pending: [None, None],
            host_halted: false,
            terminal_fault: None,
            entries: Vec::new(),
            raw_binding: [None, None],
            raw_shadow: None,
            next: 0,
            next_release: 0,
            used: Counts {
                tracker: 0,
                ingress: 0,
                results: 0,
                ledger: 0,
            },
            limits,
            ring_limit,
        }
    }

    fn with_raw_capacity(mut self, capacity: InputCapacity) -> Self {
        self.raw_shadow = Some([RawShadow::new(capacity); 2]);
        self
    }

    fn onset_preflight(&self, source: ModelSource) -> OnsetPreflight {
        let index = source.index();
        if self.host_halted {
            return OnsetPreflight::Halted;
        }
        if self.retry_pending[index].is_some() || self.release_pending[index].is_some() {
            return OnsetPreflight::Order;
        }
        if self.release_reservations[index] + 2 > self.ring_limit {
            return OnsetPreflight::NoCredit(NoCreditReason::SourceReserve);
        }
        if self
            .raw_shadow
            .is_some_and(|raw| !raw[index].can_charge_onset())
        {
            return OnsetPreflight::NoCredit(NoCreditReason::RawCapture);
        }
        for (used, limit, reason) in [
            (
                self.used.tracker,
                self.limits.tracker,
                NoCreditReason::Tracker,
            ),
            (
                self.used.ingress,
                self.limits.ingress,
                NoCreditReason::Ingress,
            ),
            (
                self.used.results,
                self.limits.results,
                NoCreditReason::Results,
            ),
            (self.used.ledger, self.limits.ledger, NoCreditReason::Ledger),
        ] {
            if used >= limit {
                return OnsetPreflight::NoCredit(reason);
            }
        }
        if self.next == u64::MAX {
            return OnsetPreflight::NoCredit(NoCreditReason::OccurrenceIdentity);
        }
        if self.rings[index].len() + self.release_reservations[index] + 2 > self.ring_limit {
            OnsetPreflight::Retry
        } else {
            OnsetPreflight::Ready
        }
    }

    fn fault(&mut self, source: ModelSource, original: Midi1Input, reason: TerminalReason) {
        if self.host_halted {
            return;
        }
        self.terminal_fault = Some((source, TerminalFault { original, reason }));
        self.host_halted = true;
    }

    fn submit(
        &mut self,
        source: ModelSource,
        input: Midi1Input,
    ) -> Result<OccurrenceId, SubmitError> {
        self.submit_at(source, input, None)
    }

    fn submit_at(
        &mut self,
        source: ModelSource,
        input: Midi1Input,
        mapped_at: Option<SampleTime>,
    ) -> Result<OccurrenceId, SubmitError> {
        let Midi1Event::NoteOn { key, .. } = input.event() else {
            panic!("the reduced submit operation accepts only note onsets");
        };
        let index = source.index();
        if self.host_halted {
            return Err(SubmitError::Halted(input));
        }
        if mapped_at.is_some_and(|at| {
            self.merge_frontiers[index].is_some_and(|last| at <= last)
                || self.merge_last_submitted[index].is_some_and(|last| at < last)
        }) {
            self.fault(source, input, TerminalReason::Order);
            return Err(SubmitError::Order(input));
        }
        let needs_retry = match self.onset_preflight(source) {
            OnsetPreflight::Halted => return Err(SubmitError::Halted(input)),
            OnsetPreflight::Order => {
                self.fault(source, input, TerminalReason::Order);
                return Err(SubmitError::Order(input));
            }
            OnsetPreflight::NoCredit(reason) => {
                self.fault(source, input, TerminalReason::NoCredit(reason));
                return Err(SubmitError::NoCredit(input, reason));
            }
            OnsetPreflight::Retry => true,
            OnsetPreflight::Ready => false,
        };
        let next = self.next + 1;
        self.next = next;
        if mapped_at.is_some() {
            self.merge_last_submitted[index] = mapped_at;
        }
        self.used.tracker += 1;
        self.used.ingress += 1;
        self.used.results += 1;
        self.used.ledger += 1;
        if let Some(raw) = self.raw_shadow.as_mut() {
            raw[index].queued_onsets += 1;
            raw[index].release_reservations += 1;
        }
        let attempt = Attempt {
            id: OccurrenceId(next),
            key: Key {
                source,
                channel: input.channel(),
                note: key,
            },
            input,
            mapped_at,
        };
        if needs_retry {
            let token = RetryToken {
                id: attempt.id,
                source,
            };
            self.retry_pending[index] = Some(PendingRetry {
                attempt,
                charge: limits(1),
            });
            return Err(SubmitError::Retry(token));
        }
        let id = attempt.id;
        self.source_holds.push(SourceHold {
            id,
            key: attempt.key,
            mapped_at: attempt.mapped_at,
            release_queued: false,
            retired: false,
        });
        self.release_reservations[index] += 1;
        self.rings[index].push_back(RingPacket::Onset(attempt));
        Ok(id)
    }

    fn retry(&mut self, token: RetryToken) -> Result<OccurrenceId, RetryError> {
        let index = token.source.index();
        if self.retry_pending[index]
            .as_ref()
            .is_none_or(|pending| pending.attempt.id != token.id)
        {
            return Err(RetryError::Stale(token));
        }
        if self.host_halted {
            return Err(RetryError::Halted(token));
        }
        if self.rings[index].len() + self.release_reservations[index] + 2 > self.ring_limit {
            return Err(RetryError::Blocked(token));
        }
        let pending = self.retry_pending[index]
            .take()
            .expect("matching pending retry");
        let id = pending.attempt.id;
        self.source_holds.push(SourceHold {
            id,
            key: pending.attempt.key,
            mapped_at: pending.attempt.mapped_at,
            release_queued: false,
            retired: false,
        });
        self.release_reservations[index] += 1;
        self.rings[index].push_back(RingPacket::Onset(pending.attempt));
        Ok(id)
    }

    fn retire(&mut self, token: RetryToken) -> Result<Retirement, RetryError> {
        let index = token.source.index();
        if self.retry_pending[index]
            .as_ref()
            .is_none_or(|pending| pending.attempt.id != token.id)
        {
            return Err(RetryError::Stale(token));
        }
        if self.host_halted {
            return Err(RetryError::Halted(token));
        }
        let pending = self.retry_pending[index]
            .take()
            .expect("matching pending retry");
        self.used.tracker -= pending.charge.tracker;
        self.used.ingress -= pending.charge.ingress;
        self.used.results -= pending.charge.results;
        if let Some(raw) = self.raw_shadow.as_mut() {
            raw[index].queued_onsets -= 1;
            raw[index].release_reservations -= 1;
        }
        self.source_holds.push(SourceHold {
            id: pending.attempt.id,
            key: pending.attempt.key,
            mapped_at: pending.attempt.mapped_at,
            release_queued: false,
            retired: true,
        });
        Ok(Retirement {
            id: pending.attempt.id,
            key: pending.attempt.key,
            input: pending.attempt.input,
            returned: Counts {
                tracker: pending.charge.tracker,
                ingress: pending.charge.ingress,
                results: pending.charge.results,
                ledger: 0,
            },
        })
    }

    fn service(&mut self, source: ModelSource, raw: bool, ingress: bool) -> OccurrenceId {
        self.service_with_input(source, raw, ingress).0
    }

    /// An empty source can rule out later stamped onsets and releases at or
    /// before this time. Ordinary packets are outside this selector model.
    /// This models a serviced frontier, not one still in its source ring.
    fn advance_merge_frontier(&mut self, source: ModelSource, at: SampleTime) -> bool {
        let index = source.index();
        if self.host_halted
            || !self.rings[index].is_empty()
            || self.retry_pending[index].is_some()
            || self.release_pending[index].is_some()
            || self.merge_frontiers[index].is_some_and(|last| at <= last)
            || self.merge_last_submitted[index].is_some_and(|last| at <= last)
        {
            return false;
        }
        self.merge_frontiers[index] = Some(at);
        true
    }

    /// Choose only a stamped head whose earlier competitors are already known.
    /// Source index fixes ties; an empty peer needs a serviced frontier.
    fn next_merged_onset(&self) -> Option<ModelSource> {
        if self.host_halted {
            return None;
        }
        let head = |source: ModelSource| match self.rings[source.index()].front()? {
            RingPacket::Onset(attempt) => attempt.mapped_at,
            RingPacket::Ordinary | RingPacket::Release { .. } => None,
        };
        let candidates = [ModelSource::First, ModelSource::Second];
        let candidate = candidates
            .into_iter()
            .filter_map(|source| head(source).map(|at| (at, source)))
            .min_by_key(|(at, source)| (*at, source.index()))?;
        let peer = candidates[1 - candidate.1.index()];
        if head(peer).is_some_and(|at| at >= candidate.0)
            || (self.rings[peer.index()].is_empty()
                && self.merge_frontiers[peer.index()].is_some_and(|at| at >= candidate.0))
        {
            Some(candidate.1)
        } else {
            None
        }
    }

    fn service_next_merged_onset(
        &mut self,
        raw: bool,
        ingress: bool,
    ) -> Option<(ModelSource, OccurrenceId)> {
        let source = self.next_merged_onset()?;
        Some((source, self.service(source, raw, ingress)))
    }

    fn service_with_input(
        &mut self,
        source: ModelSource,
        raw: bool,
        ingress: bool,
    ) -> (OccurrenceId, Key, Midi1Input) {
        assert!(!self.host_halted, "halted source service needs teardown");
        let RingPacket::Onset(attempt) = self.rings[source.index()]
            .pop_front()
            .expect("queued source occurrence")
        else {
            panic!("onset service cannot skip an earlier source packet");
        };
        if !raw {
            self.used.tracker -= 1;
            if let Some(shadow) = self.raw_shadow.as_mut() {
                shadow[source.index()].queued_onsets -= 1;
                shadow[source.index()].release_reservations -= 1;
            }
        }
        if !ingress {
            self.used.ingress -= 1;
        }
        self.entries.push(Entry {
            id: attempt.id,
            key: attempt.key,
            input: attempt.input,
            mapped_at: attempt.mapped_at,
            source_release: None,
            released: false,
            raw_held: raw,
            raw_onset: None,
            raw_release: None,
            ingress_held: ingress,
            ingress_disposition: if ingress {
                IngressDisposition::Pending
            } else {
                IngressDisposition::FakeRefused
            },
            ingress_command: None,
            ingress_release_command: None,
            ingress_release_result: None,
            result_held: true,
        });
        (attempt.id, attempt.key, attempt.input)
    }

    fn can_offer_ordinary(&self, source: ModelSource) -> bool {
        let index = source.index();
        !self.host_halted
            && self.retry_pending[index].is_none()
            && self.release_pending[index].is_none()
            && self.rings[index].len() + self.release_reservations[index] < self.ring_limit
            && self
                .raw_shadow
                .is_none_or(|raw| raw[index].can_charge_ordinary())
    }

    fn offer_ordinary(&mut self, source: ModelSource) {
        assert!(self.can_offer_ordinary(source));
        let index = source.index();
        if let Some(raw) = self.raw_shadow.as_mut() {
            raw[index].queued_ordinary += 1;
        }
        self.rings[index].push_back(RingPacket::Ordinary);
    }

    fn service_ordinary(&mut self, source: ModelSource) {
        assert!(!self.host_halted, "halted source service needs teardown");
        assert!(matches!(
            self.rings[source.index()].pop_front(),
            Some(RingPacket::Ordinary)
        ));
        if let Some(raw) = self.raw_shadow.as_mut() {
            let pressure = &mut raw[source.index()];
            assert!(pressure.queued_ordinary > 0);
            pressure.queued_ordinary -= 1;
            pressure.occupied += 1;
        }
    }

    fn offer_release(
        &mut self,
        source: ModelSource,
        input: Midi1Input,
    ) -> Result<ReleaseOffer, ReleaseOfferError> {
        self.offer_release_at(source, input, None)
    }

    fn offer_release_at(
        &mut self,
        source: ModelSource,
        input: Midi1Input,
        mapped_at: Option<SampleTime>,
    ) -> Result<ReleaseOffer, ReleaseOfferError> {
        assert!(matches!(input.event(), Midi1Event::KeyRelease { .. }));
        let index = source.index();
        if self.host_halted {
            return Err(ReleaseOfferError::Halted(input));
        }
        if mapped_at.is_some_and(|at| {
            self.merge_frontiers[index].is_some_and(|last| at <= last)
                || self.merge_last_submitted[index].is_some_and(|last| at < last)
        }) {
            self.fault(source, input, TerminalReason::Order);
            return Err(ReleaseOfferError::Order(input));
        }
        if self.release_pending[index].is_some() {
            self.fault(source, input, TerminalReason::Order);
            return Err(ReleaseOfferError::Order(input));
        }
        if self.retry_pending[index].is_some() {
            let Some(next) = self.next_release.checked_add(1) else {
                self.fault(source, input, TerminalReason::ReleaseIdentity);
                return Err(ReleaseOfferError::IdentityExhausted(input));
            };
            self.next_release = next;
            self.release_pending[index] = Some(PendingRelease {
                id: ReleaseAttemptId(next),
                input,
                mapped_at,
            });
            if mapped_at.is_some() {
                self.merge_last_submitted[index] = mapped_at;
            }
            return Err(ReleaseOfferError::Blocked(ReleaseRetryToken {
                id: ReleaseAttemptId(next),
                source,
            }));
        }
        let result = self.enqueue_release(source, input, mapped_at);
        if result.is_ok() && mapped_at.is_some() {
            self.merge_last_submitted[index] = mapped_at;
        }
        result
    }

    fn retry_release(
        &mut self,
        token: ReleaseRetryToken,
    ) -> Result<ReleaseOffer, ReleaseRetryError> {
        let index = token.source.index();
        if self.release_pending[index]
            .as_ref()
            .is_none_or(|pending| pending.id != token.id)
        {
            return Err(ReleaseRetryError::Stale(token));
        }
        if self.host_halted {
            return Err(ReleaseRetryError::Halted(token));
        }
        if self.retry_pending[index].is_some() {
            return Err(ReleaseRetryError::Blocked(token));
        }
        let pending = self.release_pending[index]
            .as_ref()
            .expect("matching pending release");
        let (input, mapped_at) = (pending.input, pending.mapped_at);
        let outcome = match self.enqueue_release(token.source, input, mapped_at) {
            Ok(outcome) => outcome,
            Err(ReleaseOfferError::Unmatched(original)) => ReleaseOffer::Unmatched(original),
            Err(ReleaseOfferError::MissingStamp(_)) => {
                self.release_pending[index] = None;
                return Err(ReleaseRetryError::MissingStamp(token));
            }
            Err(_) => return Err(ReleaseRetryError::Blocked(token)),
        };
        self.release_pending[index] = None;
        Ok(outcome)
    }

    fn enqueue_release(
        &mut self,
        source: ModelSource,
        input: Midi1Input,
        mapped_at: Option<SampleTime>,
    ) -> Result<ReleaseOffer, ReleaseOfferError> {
        let Midi1Event::KeyRelease { key, .. } = input.event() else {
            panic!("the reduced release operation accepts only key releases");
        };
        let index = source.index();
        let key = Key {
            source,
            channel: input.channel(),
            note: key,
        };
        let hold_index = self
            .source_holds
            .iter()
            .position(|hold| hold.key == key && !hold.release_queued)
            .ok_or(ReleaseOfferError::Unmatched(input))?;
        if self.source_holds[hold_index].mapped_at.is_some() && mapped_at.is_none() {
            self.fault(source, input, TerminalReason::MissingReleaseStamp);
            return Err(ReleaseOfferError::MissingStamp(input));
        }
        let hold = &mut self.source_holds[hold_index];
        if hold.retired {
            let id = hold.id;
            self.source_holds.remove(hold_index);
            self.used.ledger -= 1;
            return Ok(ReleaseOffer::Retired(id));
        }
        assert!(self.release_reservations[index] > 0);
        assert!(self.rings[index].len() < self.ring_limit);
        hold.release_queued = true;
        self.release_reservations[index] -= 1;
        self.rings[index].push_back(RingPacket::Release {
            id: hold.id,
            key,
            input,
            mapped_at,
        });
        Ok(ReleaseOffer::Queued(hold.id))
    }

    fn service_release(&mut self, source: ModelSource) -> ReleaseRoute {
        self.service_release_with_input(source).0
    }

    fn service_release_with_input(
        &mut self,
        source: ModelSource,
    ) -> (ReleaseRoute, Key, Midi1Input) {
        assert!(!self.host_halted, "halted source service needs teardown");
        let Some(RingPacket::Release {
            id,
            key,
            input,
            mapped_at,
        }) = self.rings[source.index()].pop_front()
        else {
            panic!("release service cannot skip an earlier source packet");
        };
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.key == key && !entry.released)
            .expect("queued onset precedes its release");
        assert_eq!(entry.id, id, "release must preserve same-key FIFO");
        entry.released = true;
        entry.source_release = Some((input, mapped_at));
        let route = ReleaseRoute {
            id: entry.id,
            raw: entry.raw_held,
            ingress: entry.ingress_held,
        };
        let hold_index = self
            .source_holds
            .iter()
            .position(|hold| hold.id == id)
            .expect("release retains its source hold");
        self.source_holds.remove(hold_index);
        self.reap(route.id);
        (route, key, input)
    }

    fn release(&mut self, source: ModelSource, input: Midi1Input) -> Option<ReleaseRoute> {
        match self.offer_release(source, input) {
            Ok(ReleaseOffer::Queued(_)) => Some(self.service_release(source)),
            Ok(ReleaseOffer::Retired(_)) => {
                panic!("direct release helper requires a queued onset")
            }
            Ok(ReleaseOffer::Unmatched(_)) => {
                panic!("direct release helper cannot consume a pending stray release")
            }
            Err(ReleaseOfferError::Unmatched(_)) => None,
            Err(
                ReleaseOfferError::Blocked(_)
                | ReleaseOfferError::Order(_)
                | ReleaseOfferError::Halted(_)
                | ReleaseOfferError::IdentityExhausted(_)
                | ReleaseOfferError::MissingStamp(_),
            ) => {
                panic!("direct release helper cannot skip pending source custody")
            }
        }
    }

    fn redeem_raw_release(&mut self, id: OccurrenceId) {
        let entry = self.entry_mut(id);
        assert!(entry.released && entry.raw_held);
        entry.raw_held = false;
        self.used.tracker -= 1;
        self.reap(id);
    }

    fn settle_fake_raw_release(
        &mut self,
        id: OccurrenceId,
    ) -> Result<(), RawReceiptSettlementError> {
        let entry = self.entry_mut(id);
        if entry.raw_onset.is_some() {
            return Err(RawReceiptSettlementError::BoundToRealInput);
        }
        if !entry.released || !entry.raw_held {
            return Err(RawReceiptSettlementError::Unreleased);
        }
        self.redeem_raw_release(id);
        Ok(())
    }

    fn bind_raw_onset(
        &mut self,
        id: OccurrenceId,
        raw_id: InputEventId,
    ) -> Result<(), RawOnsetBindError> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or(RawOnsetBindError::State)?;
        if !entry.raw_held || entry.raw_onset.is_some() {
            return Err(RawOnsetBindError::State);
        }
        if self
            .entries
            .iter()
            .any(|entry| entry.raw_onset == Some(raw_id))
        {
            return Err(RawOnsetBindError::Duplicate);
        }
        let source = entry.key.source.index();
        let generation = raw_id.generation();
        let serial = raw_id.serial();
        if let Some((bound_generation, last_serial)) = self.raw_binding[source] {
            if generation != bound_generation {
                return Err(RawOnsetBindError::SourceGeneration);
            }
            if serial <= last_serial {
                return Err(RawOnsetBindError::Duplicate);
            }
        } else if self.raw_binding[1 - source]
            .is_some_and(|(other_generation, _)| generation == other_generation)
        {
            return Err(RawOnsetBindError::SourceGeneration);
        }
        self.entry_mut(id).raw_onset = Some(raw_id);
        self.raw_binding[source] = Some((generation, serial));
        if let Some(raw) = self.raw_shadow.as_mut() {
            assert!(raw[source].queued_onsets > 0);
            raw[source].queued_onsets -= 1;
            raw[source].occupied += 1;
        }
        Ok(())
    }

    fn admit_raw_release(&mut self, id: OccurrenceId, raw_id: InputEventId) {
        let entry = self.entry_mut(id);
        assert!(entry.released && entry.raw_held && entry.raw_release.is_none());
        let onset = entry.raw_onset.expect("raw onset must precede release");
        assert_eq!(raw_id.generation(), onset.generation());
        assert!(raw_id.serial() > onset.serial());
        let source = entry.key.source.index();
        entry.raw_release = Some(raw_id);
        if let Some(raw) = self.raw_shadow.as_mut() {
            assert!(raw[source].release_reservations > 0);
            raw[source].release_reservations -= 1;
            raw[source].occupied += 1;
        }
    }

    fn admit_raw_frontier(&mut self, source: ModelSource) {
        if let Some(raw) = self.raw_shadow.as_mut() {
            raw[source.index()].occupied += 1;
        }
    }

    fn collect_raw_receipts(&mut self, source: ModelSource, count: usize) {
        if let Some(raw) = self.raw_shadow.as_mut() {
            assert!(raw[source.index()].occupied >= count);
            raw[source.index()].occupied -= count;
        }
    }

    fn raw_pressure(&self, source: ModelSource) -> RawShadow {
        self.raw_shadow.expect("raw-capacity fixture")[source.index()]
    }

    fn settle_delivered_raw_release(
        &mut self,
        id: OccurrenceId,
        onset: &InputReceipt,
        release: &InputReceipt,
    ) -> Result<(), RawReceiptSettlementError> {
        let checked_raw = self.raw_shadow.is_some();
        let entry = self.entry_mut(id);
        if !entry.released || !entry.raw_held {
            return Err(RawReceiptSettlementError::Unreleased);
        }
        let raw_onset = entry
            .raw_onset
            .ok_or(RawReceiptSettlementError::UnboundOnset)?;
        if onset.id != raw_onset || onset.matched_onset.is_some() {
            return Err(RawReceiptSettlementError::WrongOnset);
        }
        if !matches!(
            onset.observation,
            InputObservation::Message { input, .. }
                if input.channel() == entry.key.channel
                    && matches!(input.event(), Midi1Event::NoteOn { key, .. } if key == entry.key.note)
        ) {
            return Err(RawReceiptSettlementError::WrongOnset);
        }
        if release.matched_onset != Some(raw_onset)
            || release.id.generation() != raw_onset.generation()
            || release.id.serial() <= raw_onset.serial()
            || (checked_raw && entry.raw_release != Some(release.id))
        {
            return Err(RawReceiptSettlementError::WrongRelease);
        }
        if !matches!(
            release.observation,
            InputObservation::Message { input, .. }
                if input.channel() == entry.key.channel
                    && matches!(input.event(), Midi1Event::KeyRelease { key, .. } if key == entry.key.note)
        ) {
            return Err(RawReceiptSettlementError::WrongRelease);
        }
        let occurrence = |receipt: &InputReceipt| match &receipt.outcome {
            InputOutcome::Delivered(SessionSourceOutcome::Published(record)) => record.occurrence,
            _ => None,
        };
        let onset_occurrence = occurrence(onset).ok_or(RawReceiptSettlementError::Undelivered)?;
        let release_occurrence =
            occurrence(release).ok_or(RawReceiptSettlementError::Undelivered)?;
        if onset_occurrence != release_occurrence {
            return Err(RawReceiptSettlementError::WrongOccurrence);
        }
        self.redeem_raw_release(id);
        Ok(())
    }

    fn settle_ingress_release(&mut self, id: OccurrenceId) -> Result<(), IngressResultBindError> {
        if self.host_halted {
            return Err(IngressResultBindError::State);
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) else {
            return Err(IngressResultBindError::State);
        };
        if !entry.released
            || !entry.ingress_held
            || entry.ingress_command.is_some()
            || entry.ingress_release_command.is_some()
            || !matches!(entry.ingress_disposition, IngressDisposition::Accepted(_))
            || entry.ingress_release_result != Some(Ok(()))
        {
            return Err(IngressResultBindError::State);
        }
        entry.ingress_held = false;
        self.used.ingress -= 1;
        self.reap(id);
        Ok(())
    }

    fn settle_fake_ingress_release(&mut self, id: OccurrenceId) {
        assert!(!self.host_halted);
        let entry = self.entry_mut(id);
        assert!(
            entry.released
                && entry.ingress_held
                && entry.ingress_disposition == IngressDisposition::Pending
                && entry.mapped_at.is_none()
                && entry.ingress_command.is_none()
                && entry.ingress_release_command.is_none()
        );
        entry.ingress_held = false;
        self.used.ingress -= 1;
        self.reap(id);
    }

    fn submit_ingress_onset(
        &mut self,
        control: &mut MixedOneShotControl,
        occurrence: OccurrenceId,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), MixedIngressSubmitError> {
        let entry = self.entry_mut(occurrence);
        assert!(
            entry.ingress_held
                && entry.ingress_disposition == IngressDisposition::Pending
                && entry.ingress_command.is_none()
        );
        let Midi1Event::NoteOn { key, velocity } = entry.input.event() else {
            panic!("source onset entry must retain an onset");
        };
        let at = entry
            .mapped_at
            .expect("source onset must retain mapped time");
        let request = MixedIngressRequest::Onset {
            origin: MixedIngressOriginId(entry.id.0),
            at,
            key,
            velocity,
        };
        let command = control.submit_ingress(request)?;
        entry.ingress_command = Some((command, request));
        Ok((command, request))
    }

    fn apply_ingress_onset_result(
        &mut self,
        occurrence: OccurrenceId,
        result: MixedIngressResult,
    ) -> Result<(), IngressResultBindError> {
        let Some(index) = self.entries.iter().position(|entry| entry.id == occurrence) else {
            return Err(IngressResultBindError::State);
        };
        let entry = &self.entries[index];
        let Some((command, request)) = entry.ingress_command else {
            return Err(IngressResultBindError::State);
        };
        if !entry.ingress_held || entry.ingress_disposition != IngressDisposition::Pending {
            return Err(IngressResultBindError::State);
        }
        if result.id != command {
            return Err(IngressResultBindError::WrongCommand);
        }
        if result.request != request {
            return Err(IngressResultBindError::WrongRequest);
        }
        let MixedIngressOutcome::Onset(outcome) = result.outcome else {
            return Err(IngressResultBindError::WrongOutcome);
        };
        if let Ok(identity) = outcome
            && self
                .entries
                .iter()
                .any(|entry| entry.ingress_disposition == IngressDisposition::Accepted(identity))
        {
            return Err(IngressResultBindError::DuplicateIdentity);
        }
        let entry = &mut self.entries[index];
        entry.ingress_command = None;
        match outcome {
            Ok(identity) => entry.ingress_disposition = IngressDisposition::Accepted(identity),
            Err(reason) => {
                entry.ingress_disposition = IngressDisposition::Refused(reason);
                if !self.host_halted {
                    entry.ingress_held = false;
                    self.used.ingress -= 1;
                    self.reap(occurrence);
                }
            }
        }
        Ok(())
    }

    fn submit_ingress_release(
        &mut self,
        control: &mut MixedOneShotControl,
        occurrence: OccurrenceId,
    ) -> Result<(MixedIngressCommandId, MixedIngressRequest), IngressReleaseSubmitError> {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == occurrence) else {
            return Err(IngressReleaseSubmitError::State);
        };
        if let Some(Err(reason)) = entry.ingress_release_result {
            return Err(IngressReleaseSubmitError::TerminalRefused(reason));
        }
        if self.host_halted {
            return Err(IngressReleaseSubmitError::State);
        }
        let IngressDisposition::Accepted(identity) = entry.ingress_disposition else {
            return Err(IngressReleaseSubmitError::State);
        };
        if !entry.released
            || !entry.ingress_held
            || entry.ingress_command.is_some()
            || entry.ingress_release_command.is_some()
            || entry.ingress_release_result.is_some()
        {
            return Err(IngressReleaseSubmitError::State);
        }
        let Some((_, Some(at))) = entry.source_release else {
            return Err(IngressReleaseSubmitError::State);
        };
        let request = MixedIngressRequest::Release {
            origin: MixedIngressOriginId(entry.id.0),
            at,
            identity,
        };
        let command = control
            .submit_ingress(request)
            .map_err(IngressReleaseSubmitError::Submit)?;
        entry.ingress_release_command = Some((command, request));
        Ok((command, request))
    }

    fn apply_ingress_release_result(
        &mut self,
        occurrence: OccurrenceId,
        result: MixedIngressResult,
    ) -> Result<(), IngressResultBindError> {
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == occurrence) else {
            return Err(IngressResultBindError::State);
        };
        let Some((command, request)) = entry.ingress_release_command else {
            return Err(IngressResultBindError::State);
        };
        if result.id != command {
            return Err(IngressResultBindError::WrongCommand);
        }
        if result.request != request {
            return Err(IngressResultBindError::WrongRequest);
        }
        let MixedIngressOutcome::Release(outcome) = result.outcome else {
            return Err(IngressResultBindError::WrongOutcome);
        };
        entry.ingress_release_command = None;
        entry.ingress_release_result = Some(outcome);
        if let Err(reason) = outcome {
            let source = entry.key.source;
            let original = entry.source_release.expect("owned release input").0;
            self.fault(source, original, TerminalReason::IngressRelease(reason));
        }
        Ok(())
    }

    fn collect_result(&mut self, id: OccurrenceId) -> Result<(), IngressResultBindError> {
        if self.host_halted {
            return Err(IngressResultBindError::State);
        }
        let entry = self.entry_mut(id);
        assert!(entry.result_held);
        entry.result_held = false;
        self.used.results -= 1;
        self.reap(id);
        Ok(())
    }

    fn entry_mut(&mut self, id: OccurrenceId) -> &mut Entry {
        self.entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .expect("source occurrence remains owned")
    }

    fn reap(&mut self, id: OccurrenceId) {
        if self.host_halted {
            return;
        }
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.id == id
                && entry.released
                && !entry.raw_held
                && !entry.ingress_held
                && !entry.result_held
        }) {
            self.entries.remove(index);
            self.used.ledger -= 1;
        }
    }
}

fn input(status: u8, key: u8, velocity: u8) -> Midi1Input {
    Midi1Input::from_bytes([status, key, velocity]).expect("valid test MIDI")
}

fn limits(count: usize) -> Counts {
    Counts {
        tracker: count,
        ingress: count,
        results: count,
        ledger: count,
    }
}

fn bridge_recorder_limits() -> RecordingLimits {
    RecordingLimits::new(
        HeldNoteCount::limit(4).unwrap(),
        EventCount::limit(16).unwrap(),
    )
    .unwrap()
    .with_capture(
        CaptureLimits::new(CaptureLimitsInput {
            max_tracked_input_notes: TrackedInputNoteCount::limit(6).unwrap(),
            max_capture_sources: CaptureSourceCount::limit(2).unwrap(),
            max_capture_passes: CapturePassCount::limit(4).unwrap(),
            max_pending_capture_results: CaptureResultCount::limit(1).unwrap(),
            max_capture_bytes: PreparedBytes::limit(1_048_576).unwrap(),
            max_audio_capture_frames: FrameCount::new(1),
            max_projection_ticks: ProjectionTickCount::limit(100).unwrap(),
            capture_lateness_allowance: FrameCount::ZERO,
        })
        .unwrap(),
    )
    .unwrap()
}

fn bridge_raw_input(
    epoch: StreamEpoch,
    source: usize,
) -> (SimulatedNoteInput, ConnectionGeneration) {
    let (mut raw, generation) = bridge_ready_input(epoch, source);
    raw.start(generation).unwrap();
    (raw, generation)
}

fn bridge_ready_input(
    epoch: StreamEpoch,
    source: usize,
) -> (SimulatedNoteInput, ConnectionGeneration) {
    let mut raw = SimulatedNoteInput::new(
        EndpointId::new(format!("bridge-{source}")).unwrap(),
        InputLimits {
            cells: InputCapacity::new(8).unwrap(),
            bytes: PreparedBytes::measured(65_536),
        },
    )
    .unwrap();
    let generation = raw.begin().unwrap();
    raw.prepare(
        generation,
        SimulatedInputClock::new(
            epoch,
            SampleTime::ZERO,
            InputTick::new(0),
            InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
            InputTickSpan::new(0),
        ),
    )
    .unwrap();
    (raw, generation)
}

#[test]
fn queued_two_source_releases_preserve_payload_and_recorder_pairing() {
    let epoch = issue_epoch().unwrap();
    let mut recorder =
        SimulatedNoteRecorder::prepare_fixture(epoch, bridge_recorder_limits()).unwrap();
    let recorder_sources = [
        recorder
            .bind_fixture_source(Some(ControllerSnapshot::neutral()))
            .unwrap(),
        recorder
            .bind_fixture_source(Some(ControllerSnapshot::neutral()))
            .unwrap(),
    ];
    let context = NoteArmContext::prepare(NoteArmInput {
        target: FixtureTargetId::new(1).unwrap(),
        expected_revision: FixtureRevision::new(0),
        interval: MusicalInterval::new(MusicalTick::ZERO, MusicalTick::new(8)).unwrap(),
        mode: CaptureMode::Overdub,
        quantization: CaptureQuantization::Off,
        epoch,
        anchor: StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        tempo: TempoMap::new(
            Bpm::new(120.0).unwrap(),
            &[],
            SampleRate::new(48_000.0).unwrap(),
        )
        .unwrap(),
    })
    .unwrap();
    let ticket = recorder.arm(context, &recorder_sources).unwrap();
    for source in recorder_sources {
        recorder
            .fence(
                source,
                epoch,
                SampleTime::ZERO,
                recorder.source_sequence(source).unwrap(),
            )
            .unwrap();
    }
    recorder.start(ticket).unwrap();

    let mut raw = [bridge_raw_input(epoch, 0), bridge_raw_input(epoch, 1)];
    let mut model = Model::new(limits(3), 4);
    let onsets = [
        (ModelSource::First, input(0x90, 60, 100), 10),
        (ModelSource::Second, input(0x90, 60, 110), 11),
        (ModelSource::First, input(0x90, 60, 120), 12),
    ];
    let ids = onsets.map(|(source, note, _)| model.submit(source, note).unwrap());
    let mut identities = Vec::new();
    for ((source, onset, time), expected_id) in onsets.into_iter().zip(ids) {
        let (id, key, queued) = model.service_with_input(source, true, false);
        assert_eq!(id, expected_id);
        assert_eq!(key.source, source);
        assert_eq!(key.channel, queued.channel());
        assert_eq!(
            key.note,
            match queued.event() {
                Midi1Event::NoteOn { key, .. } => key,
                _ => panic!("queued onset must remain an onset"),
            }
        );
        assert_eq!(queued, onset);
        let index = key.source.index();
        let at = SampleTime::new(time);
        let observation = crate::host::input::InputObservation::Message {
            tick: InputTick::new(at.as_u64()),
            arrival: at,
            input: queued,
        };
        let (owner, generation) = &mut raw[index];
        assert_eq!(
            owner.preflight_observation(*generation, observation),
            Ok(())
        );
        let raw_id = owner
            .offer_message(*generation, InputTick::new(at.as_u64()), at, queued)
            .unwrap();
        assert_eq!(raw_id.generation(), *generation);
        let receipt = recorder
            .publish(
                recorder_sources[index],
                CaptureStamp::exact_fixture(epoch, at, at).unwrap(),
                queued,
                AuditionTrace::NotOffered,
            )
            .unwrap();
        assert_eq!(receipt.capture, CaptureDisposition::Recorded);
        let recorder_id = receipt.occurrence.unwrap();
        assert_eq!(recorder_id.source(), recorder_sources[index]);
        identities.push((id, key, raw_id, recorder_id));
    }
    let releases = [
        (ModelSource::First, input(0x90, 60, 0), ids[0], 20),
        (ModelSource::Second, input(0x90, 60, 0), ids[1], 21),
        (ModelSource::First, input(0x80, 60, 0), ids[2], 22),
    ];
    for (source, release, id, _) in releases {
        assert_eq!(
            model.offer_release(source, release),
            Ok(ReleaseOffer::Queued(id))
        );
    }
    for (source, release, expected_id, time) in releases {
        let (route, key, queued) = model.service_release_with_input(source);
        assert_eq!(route.id, expected_id);
        assert_eq!(key.source, source);
        assert!(route.raw);
        assert!(!route.ingress);
        assert_eq!(queued, release);
        let (model_id, original_key, raw_onset, recorder_onset) = identities
            .iter()
            .find(|(id, _, _, _)| *id == route.id)
            .unwrap();
        assert_eq!(*model_id, route.id);
        assert_eq!(*original_key, key);
        assert_eq!(key.channel, queued.channel());
        assert_eq!(
            key.note,
            match queued.event() {
                Midi1Event::KeyRelease { key, .. } => key,
                _ => panic!("queued release must remain a release"),
            }
        );
        let index = key.source.index();
        let at = SampleTime::new(time);
        let observation = crate::host::input::InputObservation::Message {
            tick: InputTick::new(at.as_u64()),
            arrival: at,
            input: queued,
        };
        let (owner, generation) = &mut raw[index];
        assert_eq!(raw_onset.generation(), *generation);
        assert_eq!(
            owner.preflight_observation(*generation, observation),
            Ok(())
        );
        let raw_release = owner
            .offer_message(*generation, InputTick::new(at.as_u64()), at, queued)
            .unwrap();
        assert_eq!(raw_release.generation(), raw_onset.generation());
        assert!(raw_release.serial() > raw_onset.serial());
        let receipt = recorder
            .publish(
                recorder_sources[index],
                CaptureStamp::exact_fixture(epoch, at, at).unwrap(),
                queued,
                AuditionTrace::NotOffered,
            )
            .unwrap();
        assert_eq!(receipt.capture, CaptureDisposition::Recorded);
        assert_eq!(receipt.occurrence, Some(*recorder_onset));
    }
    // Pairing is observed in the recorder. The model's raw, result and ledger
    // credits stay held until outcomes and raw input receipts can be joined.
    assert_eq!(
        model.used,
        Counts {
            tracker: 3,
            ingress: 0,
            results: 3,
            ledger: 3,
        }
    );
    for (owner, _) in &mut raw {
        assert_eq!(owner.state(), crate::host::ConnectionState::Running);
        assert_eq!(owner.discontinuity(), None);
        assert!(owner.collect().is_none());
    }
}

fn bridge_serial_owner(stop: u64) -> (InputCaptureSession, [ConnectionGeneration; 2]) {
    let mut capture = LoopCaptureSession::prepare(
        loop_test_support::stream(0, 2048),
        loop_test_support::limits(3, 64, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let epoch = capture.initial().epoch;
    let mut inputs = [bridge_ready_input(epoch, 0), bridge_ready_input(epoch, 1)];
    let generations = [inputs[0].1, inputs[1].1];
    let sources = [
        inputs[0]
            .0
            .bind_capture(generations[0], &mut capture, ControllerSnapshot::neutral())
            .unwrap(),
        inputs[1]
            .0
            .bind_capture(generations[1], &mut capture, ControllerSnapshot::neutral())
            .unwrap(),
    ];
    let _ticket = capture.arm(loop_test_support::input(), &sources).unwrap();
    let session = LoopRecordingSession::prepare(
        capture,
        SessionLimits {
            commands: SessionCommandCapacity::new(4).unwrap(),
            command_bytes: PreparedBytes::measured(16_384),
        },
        SessionSourceLimits {
            actions: SessionSourceCapacity::new(32).unwrap(),
            bytes: PreparedBytes::measured(32_768),
        },
    )
    .unwrap();
    let mut owner = InputCaptureSession::prepare(
        session,
        inputs.map(|(input, _)| input).into(),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    for generation in generations {
        owner.start_input(generation).unwrap();
    }
    let _play = owner.offer(SampleTime::ZERO, SessionCommand::Play).unwrap();
    let _stop = owner
        .offer(SampleTime::new(stop), SessionCommand::Stop)
        .unwrap();
    (owner, generations)
}

fn assert_raw_pressure(
    model: &Model,
    owner: &InputCaptureSession,
    generations: [ConnectionGeneration; 2],
) {
    for (source, generation) in [ModelSource::First, ModelSource::Second]
        .into_iter()
        .zip(generations)
    {
        let shadow = model.raw_pressure(source);
        let actual = owner.input(generation).unwrap().pressure();
        assert_eq!(shadow.queued_onsets, 0);
        assert_eq!(shadow.queued_ordinary, 0);
        assert_eq!(shadow.occupied, actual.occupied().as_usize());
        assert_eq!(
            shadow.release_reservations,
            actual.release_reservations().as_usize()
        );
        assert_eq!(shadow.capacity, actual.capacity());
    }
}

#[test]
fn raw_capacity_preflight_matches_the_source_capture_boundary() {
    let epoch = issue_epoch().unwrap();
    let (mut raw, generation) = bridge_raw_input(epoch, 0);
    let mut model = Model::new(limits(3), 4).with_raw_capacity(InputCapacity::new(8).unwrap());
    for (key, time) in [(60, 10), (61, 20)] {
        let note = input(0x90, key, 100);
        let expected = model.submit(ModelSource::First, note).unwrap();
        let (id, _, queued) = model.service_with_input(ModelSource::First, true, false);
        assert_eq!(id, expected);
        let raw_id = raw
            .offer_message(
                generation,
                InputTick::new(time),
                SampleTime::new(time),
                queued,
            )
            .unwrap();
        model.bind_raw_onset(id, raw_id).unwrap();
    }
    let pressure = raw.pressure();
    let shadow = model.raw_pressure(ModelSource::First);
    assert_eq!(shadow.occupied, pressure.occupied().as_usize());
    assert_eq!(
        shadow.release_reservations,
        pressure.release_reservations().as_usize()
    );
    assert_eq!(shadow.capacity, pressure.capacity());

    let third = input(0x90, 62, 100);
    assert_eq!(
        model.onset_preflight(ModelSource::First),
        OnsetPreflight::NoCredit(NoCreditReason::RawCapture)
    );
    assert_eq!(
        raw.preflight_observation(
            generation,
            InputObservation::Message {
                tick: InputTick::new(30),
                arrival: SampleTime::new(30),
                input: third,
            },
        ),
        Err(InputError::ProtectedCapacity)
    );
}

#[test]
fn queued_ordinary_packets_count_against_later_raw_onset_admission() {
    let epoch = issue_epoch().unwrap();
    let (mut raw, generation) = bridge_raw_input(epoch, 0);
    let mut model = Model::new(limits(1), 4).with_raw_capacity(InputCapacity::new(8).unwrap());
    let ordinary = input(0xe0, 0, 64);
    for _ in 0..4 {
        model.offer_ordinary(ModelSource::First);
    }
    assert_eq!(model.raw_pressure(ModelSource::First).queued_ordinary, 4);
    assert_eq!(
        model.onset_preflight(ModelSource::First),
        OnsetPreflight::NoCredit(NoCreditReason::RawCapture)
    );
    for time in 10..14 {
        let _id = raw
            .offer_message(
                generation,
                InputTick::new(time),
                SampleTime::new(time),
                ordinary,
            )
            .unwrap();
        model.service_ordinary(ModelSource::First);
    }
    let shadow = model.raw_pressure(ModelSource::First);
    let pressure = raw.pressure();
    assert_eq!(shadow.occupied, pressure.occupied().as_usize());
    assert_eq!(shadow.queued_ordinary, 0);
    assert_eq!(
        shadow.release_reservations,
        pressure.release_reservations().as_usize()
    );
    assert_eq!(
        model.onset_preflight(ModelSource::First),
        OnsetPreflight::NoCredit(NoCreditReason::RawCapture)
    );
    assert_eq!(
        raw.preflight_observation(
            generation,
            InputObservation::Message {
                tick: InputTick::new(20),
                arrival: SampleTime::new(20),
                input: input(0x90, 60, 100),
            },
        ),
        Err(InputError::ProtectedCapacity)
    );
}

#[test]
fn retiring_a_source_retry_returns_its_pending_raw_charge() {
    let mut model = Model::new(limits(1), 4).with_raw_capacity(InputCapacity::new(8).unwrap());
    for _ in 0..3 {
        model.offer_ordinary(ModelSource::First);
    }
    let retry = match model.submit(ModelSource::First, input(0x90, 60, 100)) {
        Err(SubmitError::Retry(token)) => token,
        other => panic!("expected pending retry: {other:?}"),
    };
    let pending = model.raw_pressure(ModelSource::First);
    assert_eq!(
        (
            pending.occupied,
            pending.queued_onsets,
            pending.release_reservations
        ),
        (1, 1, 1)
    );
    model.retire(retry).unwrap();
    let retired = model.raw_pressure(ModelSource::First);
    assert_eq!(
        (
            retired.occupied,
            retired.queued_onsets,
            retired.release_reservations
        ),
        (1, 0, 0)
    );
}

#[test]
fn modeled_ring_releases_settle_raw_credit_from_delivered_serial_receipts() {
    let (mut owner, generations) = bridge_serial_owner(128);

    let mut model = Model::new(limits(3), 4).with_raw_capacity(InputCapacity::new(8).unwrap());
    let onsets = [
        (ModelSource::First, input(0x90, 60, 100), 10),
        (ModelSource::Second, input(0x90, 60, 110), 11),
        (ModelSource::First, input(0x90, 60, 120), 12),
    ];
    let ids = onsets.map(|(source, note, _)| model.submit(source, note).unwrap());
    let mut admitted: Vec<(OccurrenceId, Key, InputEventId)> = Vec::new();
    for (source, note, time) in onsets {
        let (id, key, queued) = model.service_with_input(source, true, false);
        assert_eq!(queued, note);
        let generation = generations[source.index()];
        let at = SampleTime::new(time);
        let raw_id = owner
            .offer_message(generation, InputTick::new(time), at, queued)
            .unwrap();
        if id == ids[1] {
            assert_eq!(
                model.bind_raw_onset(id, admitted[0].2),
                Err(RawOnsetBindError::Duplicate)
            );
        }
        model.bind_raw_onset(id, raw_id).unwrap();
        admitted.push((id, key, raw_id));
    }
    assert_raw_pressure(&model, &owner, generations);
    let releases = [
        (ModelSource::First, input(0x90, 60, 0), ids[0], 20),
        (ModelSource::Second, input(0x90, 60, 0), ids[1], 21),
        (ModelSource::First, input(0x80, 60, 0), ids[2], 22),
    ];
    for (source, release, expected, _) in releases {
        assert_eq!(
            model.offer_release(source, release),
            Ok(ReleaseOffer::Queued(expected))
        );
    }
    let mut routed = Vec::new();
    for (source, release, expected, time) in releases {
        let (route, key, queued) = model.service_release_with_input(source);
        assert_eq!(route.id, expected);
        assert!(route.raw);
        assert!(!route.ingress);
        assert_eq!(queued, release);
        let generation = generations[source.index()];
        let at = SampleTime::new(time);
        let raw_id = owner
            .offer_message(generation, InputTick::new(time), at, queued)
            .unwrap();
        model.admit_raw_release(route.id, raw_id);
        routed.push((route.id, key, raw_id));
    }
    assert_raw_pressure(&model, &owner, generations);
    assert_eq!(
        model.used,
        Counts {
            tracker: 3,
            ingress: 0,
            results: 3,
            ledger: 3
        }
    );
    for (source, generation) in [ModelSource::First, ModelSource::Second]
        .into_iter()
        .zip(generations)
    {
        let _frontier = owner
            .advance_frontier(generation, InputTick::new(128))
            .unwrap();
        model.admit_raw_frontier(source);
    }
    assert_raw_pressure(&model, &owner, generations);
    owner.pump().unwrap();
    let mut audio = vec![0.0; 512];
    assert_eq!(
        crate::render_allocation::count_allocs(|| owner
            .render(AudioBlockMut::new(&mut audio, 512, ChannelLayout::Mono).unwrap())
            .unwrap()),
        0
    );
    owner.pump().unwrap();
    let receipts: Vec<_> = generations
        .into_iter()
        .flat_map(|generation| {
            std::iter::from_fn(|| owner.collect_input(generation).unwrap()).collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(receipts.len(), 10);
    for (source, generation) in [ModelSource::First, ModelSource::Second]
        .into_iter()
        .zip(generations)
    {
        model.collect_raw_receipts(
            source,
            receipts
                .iter()
                .filter(|receipt| receipt.id.generation() == generation)
                .count(),
        );
    }
    assert_raw_pressure(&model, &owner, generations);
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(model.used.tracker, 3);
    let receipt = |raw_id| {
        receipts
            .iter()
            .find(|receipt| receipt.id == raw_id)
            .unwrap()
    };
    assert_eq!(
        model.settle_delivered_raw_release(ids[0], receipt(admitted[0].2), receipt(routed[1].2)),
        Err(RawReceiptSettlementError::WrongRelease)
    );
    let original_release = receipt(routed[0].2);
    let cancelled_release = InputReceipt {
        audition: original_release.audition,
        id: original_release.id,
        matched_onset: original_release.matched_onset,
        observation: original_release.observation,
        clock: original_release.clock,
        outcome: InputOutcome::Cancelled,
    };
    assert_eq!(
        model.settle_delivered_raw_release(ids[0], receipt(admitted[0].2), &cancelled_release),
        Err(RawReceiptSettlementError::Undelivered)
    );
    assert_eq!(
        model.settle_fake_raw_release(ids[0]),
        Err(RawReceiptSettlementError::BoundToRealInput)
    );
    assert_eq!(model.used.tracker, 3);
    let foreign_release_raw = routed[1].2;
    let mut recorded = Vec::new();
    for (id, key, release_raw) in routed {
        let (_, original_key, onset_raw) = admitted
            .iter()
            .find(|(candidate, _, _)| *candidate == id)
            .unwrap();
        assert_eq!(*original_key, key);
        let onset_receipt = receipt(*onset_raw);
        let release_receipt = receipt(release_raw);
        assert_eq!(
            model.settle_delivered_raw_release(id, onset_receipt, release_receipt),
            Ok(())
        );
        let InputOutcome::Delivered(SessionSourceOutcome::Published(record)) =
            &onset_receipt.outcome
        else {
            panic!("settled onset must have a recorder publication");
        };
        recorded.push(record.occurrence.unwrap());
    }
    assert_ne!(recorded[0], recorded[1]);
    assert_ne!(recorded[0], recorded[2]);
    assert_ne!(recorded[1], recorded[2]);
    // The receipt-checked operation settles tracker credit. The model already
    // returned ingress credit for its refused disposition at source service;
    // no real mixed ingress or combined result is involved.
    assert_eq!(
        model.used,
        Counts {
            tracker: 0,
            ingress: 0,
            results: 3,
            ledger: 3
        }
    );
    model.collect_result(ids[0]).unwrap();
    assert!(model.entries.iter().all(|entry| entry.id != ids[0]));
    let replay = model
        .submit(ModelSource::First, input(0x90, 62, 100))
        .unwrap();
    assert_eq!(model.service(ModelSource::First, true, false), replay);
    assert_eq!(
        model.bind_raw_onset(replay, admitted[0].2),
        Err(RawOnsetBindError::Duplicate)
    );
    assert_eq!(
        model.bind_raw_onset(replay, foreign_release_raw),
        Err(RawOnsetBindError::SourceGeneration)
    );
}

#[test]
fn mixed_audio_queue_returns_exact_ingress_results_without_audio_service_allocation() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let first = MixedIngressRequest::Onset {
        origin: MixedIngressOriginId(1),
        at: SampleTime::new(140),
        key: KeyIdentity::new(60).unwrap(),
        velocity: NoteVelocity::FULL,
    };
    let second = MixedIngressRequest::Onset {
        origin: MixedIngressOriginId(2),
        at: SampleTime::new(150),
        key: KeyIdentity::new(60).unwrap(),
        velocity: NoteVelocity::FULL,
    };
    let first_id = control.submit_ingress(first).unwrap();
    let second_id = control.submit_ingress(second).unwrap();
    assert!(control.collect_ingress_result().is_none());
    audio = std::thread::spawn(move || {
        assert_eq!(
            crate::render_allocation::count_allocs(|| audio.service_test_ingress_queue()),
            0
        );
        audio
    })
    .join()
    .unwrap();
    let first_result = control.collect_ingress_result().unwrap();
    let second_result = control.collect_ingress_result().unwrap();
    assert_eq!((first_result.id, first_result.request), (first_id, first));
    assert_eq!(
        (second_result.id, second_result.request),
        (second_id, second)
    );
    let MixedIngressOutcome::Onset(Ok(first_identity)) = first_result.outcome else {
        panic!("first ingress result must retain its minted identity");
    };
    let MixedIngressOutcome::Onset(Ok(second_identity)) = second_result.outcome else {
        panic!("second ingress result must retain its minted identity");
    };
    assert_ne!(first_identity, second_identity);
    assert!(control.collect_ingress_result().is_none());

    let first_release = MixedIngressRequest::Release {
        origin: MixedIngressOriginId(1),
        at: SampleTime::new(160),
        identity: first_identity,
    };
    let second_release = MixedIngressRequest::Release {
        origin: MixedIngressOriginId(2),
        at: SampleTime::new(170),
        identity: second_identity,
    };
    let first_release_id = control.submit_ingress(first_release).unwrap();
    let second_release_id = control.submit_ingress(second_release).unwrap();
    audio = std::thread::spawn(move || {
        assert_eq!(
            crate::render_allocation::count_allocs(|| audio.service_test_ingress_queue()),
            0
        );
        audio
    })
    .join()
    .unwrap();
    assert_eq!(
        control.collect_ingress_result(),
        Some(super::MixedIngressResult {
            id: first_release_id,
            request: first_release,
            outcome: MixedIngressOutcome::Release(Ok(())),
        })
    );
    assert_eq!(
        control.collect_ingress_result(),
        Some(super::MixedIngressResult {
            id: second_release_id,
            request: second_release,
            outcome: MixedIngressOutcome::Release(Ok(())),
        })
    );
    assert_eq!(audio.test_ingress.holds_outstanding(), EventCount::NONE);
    assert!(control.collect_ingress_result().is_none());
}

#[test]
fn mixed_ingress_result_reservation_returns_full_request_until_collected() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let request = MixedIngressRequest::Onset {
        origin: MixedIngressOriginId(1),
        at: SampleTime::new(140),
        key: KeyIdentity::new(60).unwrap(),
        velocity: NoteVelocity::FULL,
    };
    let capacity = control.ingress_capacity.as_usize();
    assert!(capacity > 0);
    for _ in 0..capacity {
        let _id = control.submit_ingress(request).unwrap();
    }
    assert_eq!(
        control.submit_ingress(request),
        Err(MixedIngressSubmitError::Full(request))
    );
    audio.service_test_ingress_queue();
    assert_eq!(
        control.submit_ingress(request),
        Err(MixedIngressSubmitError::Full(request))
    );
    let first = control.collect_ingress_result().unwrap();
    assert_eq!(first.request, request);
    let next = control.submit_ingress(request).unwrap();
    audio.service_test_ingress_queue();
    let mut found = false;
    while let Some(result) = control.collect_ingress_result() {
        if result.id == next {
            found = true;
            assert_eq!(result.request, request);
        }
    }
    assert!(found);
}

#[test]
fn mixed_ingress_command_id_exhaustion_retains_original_request() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let request = MixedIngressRequest::Onset {
        origin: MixedIngressOriginId(1),
        at: SampleTime::new(140),
        key: KeyIdentity::new(60).unwrap(),
        velocity: NoteVelocity::FULL,
    };
    control.last_ingress_id = Some(super::MixedIngressCommandId(u64::MAX));
    assert_eq!(
        control.submit_ingress(request),
        Err(MixedIngressSubmitError::IdentityExhausted(request))
    );
    assert_eq!(
        control.ingress_outstanding,
        super::MixedIngressQueueCount::NONE
    );
    audio.service_test_ingress_queue();
    assert!(control.collect_ingress_result().is_none());
    assert_eq!(
        control.submit_ingress(request),
        Err(MixedIngressSubmitError::IdentityExhausted(request))
    );
}

#[test]
fn mixed_ingress_result_channel_returns_non_monotone_refusal_with_original() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let later = MixedIngressRequest::Onset {
        origin: MixedIngressOriginId(1),
        at: SampleTime::new(150),
        key: KeyIdentity::new(60).unwrap(),
        velocity: NoteVelocity::FULL,
    };
    let earlier = MixedIngressRequest::Onset {
        origin: MixedIngressOriginId(2),
        at: SampleTime::new(140),
        key: KeyIdentity::new(60).unwrap(),
        velocity: NoteVelocity::FULL,
    };
    let _later_id = control.submit_ingress(later).unwrap();
    let earlier_id = control.submit_ingress(earlier).unwrap();
    audio.service_test_ingress_queue();
    let _accepted = control.collect_ingress_result().unwrap();
    assert_eq!(
        control.collect_ingress_result(),
        Some(super::MixedIngressResult {
            id: earlier_id,
            request: earlier,
            outcome: MixedIngressOutcome::Onset(Err(IngressRefused::NonMonotoneStamp {
                time: SampleTime::new(140),
                last: SampleTime::new(150),
            })),
        })
    );
}

#[test]
fn mixed_collection_retains_unresolved_ingress_command_and_result_owners() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let request = MixedIngressRequest::Onset {
        origin: MixedIngressOriginId(1),
        at: SampleTime::new(140),
        key: KeyIdentity::new(60).unwrap(),
        velocity: NoteVelocity::FULL,
    };
    let id = control.submit_ingress(request).unwrap();
    let refusal = control.collect(audio).unwrap_err();
    assert_eq!(
        refusal.reason,
        super::MixedCollectionError::IngressResultPending
    );
    let super::MixedCollectionRefusal {
        control, mut audio, ..
    } = *refusal;
    audio.service_test_ingress_queue();
    let refusal = control.collect(audio).unwrap_err();
    assert_eq!(
        refusal.reason,
        super::MixedCollectionError::IngressResultPending
    );
    let super::MixedCollectionRefusal {
        mut control, audio, ..
    } = *refusal;
    let result = control.collect_ingress_result().unwrap();
    assert_eq!((result.id, result.request), (id, request));
    assert!(matches!(result.outcome, MixedIngressOutcome::Onset(Ok(_))));
    assert!(matches!(
        control.collect(audio).unwrap(),
        MixedCollection::Ended(_)
    ));
}

#[test]
fn merger_credit_alone_cannot_admit_out_of_order_mixed_ingress() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut mixed) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let epoch = issue_epoch().unwrap();
    let (mut first_raw, first_generation) = bridge_raw_input(epoch, 0);
    let (mut second_raw, second_generation) = bridge_raw_input(epoch, 1);
    let mut model = Model::new(limits(2), 4).with_raw_capacity(InputCapacity::new(8).unwrap());

    let first_note = input(0x90, 60, 100);
    let first_id = model
        .submit_at(ModelSource::First, first_note, Some(SampleTime::new(150)))
        .unwrap();
    let (served, _first_key, first_input) =
        model.service_with_input(ModelSource::First, true, true);
    assert_eq!(served, first_id);
    let first_raw_id = first_raw
        .offer_message(
            first_generation,
            InputTick::new(150),
            SampleTime::new(150),
            first_input,
        )
        .unwrap();
    model.bind_raw_onset(first_id, first_raw_id).unwrap();
    let (first_command, first_request) =
        model.submit_ingress_onset(&mut control, first_id).unwrap();

    assert_eq!(
        model.onset_preflight(ModelSource::Second),
        OnsetPreflight::Ready
    );
    let second_note = input(0x90, 60, 110);
    let second_id = model
        .submit_at(ModelSource::Second, second_note, Some(SampleTime::new(140)))
        .unwrap();
    let (served, _second_key, second_input) =
        model.service_with_input(ModelSource::Second, true, true);
    assert_eq!(served, second_id);
    let second_raw_id = second_raw
        .offer_message(
            second_generation,
            InputTick::new(140),
            SampleTime::new(140),
            second_input,
        )
        .unwrap();
    model.bind_raw_onset(second_id, second_raw_id).unwrap();
    let (second_command, second_request) =
        model.submit_ingress_onset(&mut control, second_id).unwrap();
    mixed.service_test_ingress_queue();
    let first_result = control.collect_ingress_result().unwrap();
    let second_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (first_result.id, first_result.request),
        (first_command, first_request)
    );
    assert_eq!(
        (second_result.id, second_result.request),
        (second_command, second_request)
    );
    assert_eq!(
        model.apply_ingress_onset_result(second_id, first_result),
        Err(IngressResultBindError::WrongCommand)
    );
    assert_eq!(
        model.apply_ingress_onset_result(
            first_id,
            MixedIngressResult {
                request: second_request,
                ..first_result
            }
        ),
        Err(IngressResultBindError::WrongRequest)
    );
    assert_eq!(
        model.apply_ingress_onset_result(
            first_id,
            MixedIngressResult {
                outcome: MixedIngressOutcome::Release(Ok(())),
                ..first_result
            }
        ),
        Err(IngressResultBindError::WrongOutcome)
    );
    assert_eq!(
        model.entry_mut(second_id).ingress_disposition,
        IngressDisposition::Pending
    );
    assert_eq!(
        model.entry_mut(first_id).ingress_disposition,
        IngressDisposition::Pending
    );
    model
        .apply_ingress_onset_result(first_id, first_result)
        .unwrap();
    let MixedIngressOutcome::Onset(Ok(first_identity)) = first_result.outcome else {
        panic!("first source must keep its accepted mixed identity");
    };
    assert_eq!(
        model.entry_mut(first_id).ingress_disposition,
        IngressDisposition::Accepted(first_identity)
    );
    assert_eq!(
        model.apply_ingress_onset_result(first_id, first_result),
        Err(IngressResultBindError::State)
    );
    assert_eq!(
        second_result.outcome,
        MixedIngressOutcome::Onset(Err(IngressRefused::NonMonotoneStamp {
            time: SampleTime::new(140),
            last: SampleTime::new(150),
        }))
    );
    model
        .apply_ingress_onset_result(second_id, second_result)
        .unwrap();
    assert_eq!(
        model.entry_mut(second_id).ingress_disposition,
        IngressDisposition::Refused(IngressRefused::NonMonotoneStamp {
            time: SampleTime::new(140),
            last: SampleTime::new(150),
        })
    );
    assert_eq!(
        mixed.test_ingress.holds_outstanding(),
        EventCount::measured(1)
    );
    assert_eq!(model.used.tracker, 2);
    assert_eq!(model.used.ingress, 1);
}

#[test]
fn serviced_source_frontier_orders_two_source_mixed_commands() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut mixed) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut model = Model::new(limits(2), 4);
    let late = model
        .submit_at(
            ModelSource::First,
            input(0x90, 60, 100),
            Some(SampleTime::new(150)),
        )
        .unwrap();
    assert_eq!(model.next_merged_onset(), None);
    let early = model
        .submit_at(
            ModelSource::Second,
            input(0x90, 62, 100),
            Some(SampleTime::new(140)),
        )
        .unwrap();
    assert_eq!(
        model.service_next_merged_onset(false, true),
        Some((ModelSource::Second, early))
    );
    let (early_command, early_request) = model.submit_ingress_onset(&mut control, early).unwrap();
    assert_eq!(model.next_merged_onset(), None);
    assert!(model.advance_merge_frontier(ModelSource::Second, SampleTime::new(151)));
    assert_eq!(
        model.service_next_merged_onset(false, true),
        Some((ModelSource::First, late))
    );
    let (late_command, late_request) = model.submit_ingress_onset(&mut control, late).unwrap();
    mixed.service_test_ingress_queue();
    for (id, command, request) in [
        (early, early_command, early_request),
        (late, late_command, late_request),
    ] {
        let result = control.collect_ingress_result().unwrap();
        assert_eq!((result.id, result.request), (command, request));
        assert!(matches!(result.outcome, MixedIngressOutcome::Onset(Ok(_))));
        model.apply_ingress_onset_result(id, result).unwrap();
    }
}

#[test]
fn merged_source_heads_require_frontiers_and_reject_late_packets() {
    let mut model = Model::new(limits(3), 4);
    let second = model
        .submit_at(
            ModelSource::Second,
            input(0x90, 62, 100),
            Some(SampleTime::new(150)),
        )
        .unwrap();
    let first = model
        .submit_at(
            ModelSource::First,
            input(0x90, 60, 100),
            Some(SampleTime::new(150)),
        )
        .unwrap();
    assert_eq!(model.next_merged_onset(), Some(ModelSource::First));
    assert!(!model.advance_merge_frontier(ModelSource::First, SampleTime::new(151)));
    assert_eq!(model.service(ModelSource::First, false, true), first);
    assert_eq!(model.next_merged_onset(), None);
    assert!(!model.advance_merge_frontier(ModelSource::First, SampleTime::new(150)));
    assert!(model.advance_merge_frontier(ModelSource::First, SampleTime::new(151)));
    assert_eq!(model.next_merged_onset(), Some(ModelSource::Second));
    assert_eq!(model.service(ModelSource::Second, false, true), second);
    assert!(!model.advance_merge_frontier(ModelSource::Second, SampleTime::new(150)));
    assert!(model.advance_merge_frontier(ModelSource::Second, SampleTime::new(151)));
    assert!(!model.advance_merge_frontier(ModelSource::Second, SampleTime::new(151)));
    let original = input(0x90, 64, 100);
    assert!(matches!(
        model.submit_at(ModelSource::Second, original, Some(SampleTime::new(150))),
        Err(SubmitError::Order(found)) if found == original
    ));
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::Second,
            TerminalFault {
                original,
                reason: TerminalReason::Order,
            }
        ))
    );
    let after_halt = input(0x90, 65, 100);
    assert!(matches!(
        model.submit_at(ModelSource::First, after_halt, Some(SampleTime::new(149))),
        Err(SubmitError::Halted(found)) if found == after_halt
    ));
}

#[test]
fn merged_frontier_rejects_stale_release_and_halted_selector() {
    let mut model = Model::new(limits(2), 4);
    let held = model
        .submit_at(
            ModelSource::First,
            input(0x90, 60, 100),
            Some(SampleTime::new(160)),
        )
        .unwrap();
    assert_eq!(model.service(ModelSource::First, false, true), held);
    assert!(model.advance_merge_frontier(ModelSource::First, SampleTime::new(181)));
    let waiting = model
        .submit_at(
            ModelSource::Second,
            input(0x90, 62, 100),
            Some(SampleTime::new(180)),
        )
        .unwrap();
    assert_eq!(model.next_merged_onset(), Some(ModelSource::Second));
    let original = input(0x80, 60, 0);
    assert_eq!(
        model.offer_release_at(ModelSource::First, original, Some(SampleTime::new(180))),
        Err(ReleaseOfferError::Order(original))
    );
    assert_eq!(model.next_merged_onset(), None);
    assert!(matches!(
        model.rings[ModelSource::Second.index()].front(),
        Some(RingPacket::Onset(attempt)) if attempt.id == waiting
    ));

    let mut model = Model::new(limits(1), 4);
    let held = model
        .submit_at(
            ModelSource::First,
            input(0x90, 60, 100),
            Some(SampleTime::new(160)),
        )
        .unwrap();
    assert_eq!(model.service(ModelSource::First, false, true), held);
    assert_eq!(
        model.offer_release_at(ModelSource::First, original, Some(SampleTime::new(155))),
        Err(ReleaseOfferError::Order(original))
    );
}

#[test]
fn stale_release_fault_precedes_unmatched_or_retired_route() {
    let original = input(0x80, 60, 0);
    let mut unmatched = Model::new(limits(1), 3);
    assert!(unmatched.advance_merge_frontier(ModelSource::First, SampleTime::new(120)));
    assert_eq!(
        unmatched.offer_release_at(ModelSource::First, original, Some(SampleTime::new(119))),
        Err(ReleaseOfferError::Order(original))
    );
    assert_eq!(
        unmatched.terminal_fault,
        Some((
            ModelSource::First,
            TerminalFault {
                original,
                reason: TerminalReason::Order,
            }
        ))
    );

    let mut retired = Model::new(limits(2), 3);
    retired
        .submit_at(
            ModelSource::First,
            input(0x90, 65, 100),
            Some(SampleTime::new(100)),
        )
        .unwrap();
    let retry = match retired.submit_at(
        ModelSource::First,
        input(0x90, 60, 100),
        Some(SampleTime::new(110)),
    ) {
        Err(SubmitError::Retry(token)) => token,
        other => panic!("second onset must retain a retry, got {other:?}"),
    };
    retired.service(ModelSource::First, false, false);
    let tombstone = retired.retire(retry).unwrap();
    assert!(retired.advance_merge_frontier(ModelSource::First, SampleTime::new(120)));
    let ledger_before = retired.used.ledger;
    assert_eq!(
        retired.offer_release_at(ModelSource::First, original, Some(SampleTime::new(119))),
        Err(ReleaseOfferError::Order(original))
    );
    assert!(
        retired
            .source_holds
            .iter()
            .any(|hold| hold.id == tombstone.id && hold.retired)
    );
    assert_eq!(retired.used.ledger, ledger_before);
}

#[test]
fn source_stamp_regression_faults_without_a_frontier() {
    let mut onsets = Model::new(limits(3), 5);
    onsets
        .submit_at(
            ModelSource::First,
            input(0x90, 60, 100),
            Some(SampleTime::new(100)),
        )
        .unwrap();
    onsets
        .submit_at(
            ModelSource::First,
            input(0x90, 62, 100),
            Some(SampleTime::new(160)),
        )
        .unwrap();
    let stale_onset = input(0x90, 64, 100);
    assert_eq!(onsets.merge_frontiers[ModelSource::First.index()], None);
    assert!(matches!(
        onsets.submit_at(ModelSource::First, stale_onset, Some(SampleTime::new(150))),
        Err(SubmitError::Order(original)) if original == stale_onset
    ));
    assert_eq!(onsets.rings[ModelSource::First.index()].len(), 2);

    let mut release = Model::new(limits(2), 5);
    let older = release
        .submit_at(
            ModelSource::First,
            input(0x90, 60, 100),
            Some(SampleTime::new(100)),
        )
        .unwrap();
    release
        .submit_at(
            ModelSource::First,
            input(0x90, 62, 100),
            Some(SampleTime::new(160)),
        )
        .unwrap();
    let stale_release = input(0x80, 60, 0);
    assert_eq!(release.merge_frontiers[ModelSource::First.index()], None);
    assert_eq!(
        release.offer_release_at(
            ModelSource::First,
            stale_release,
            Some(SampleTime::new(150))
        ),
        Err(ReleaseOfferError::Order(stale_release))
    );
    assert!(
        release
            .source_holds
            .iter()
            .any(|hold| hold.id == older && !hold.release_queued)
    );
}

#[test]
fn mixed_onset_result_after_source_release_keeps_exact_release_owner() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut model = Model::new(limits(1), 2);
    let id = model
        .submit_at(
            ModelSource::First,
            input(0x90, 60, 100),
            Some(SampleTime::new(140)),
        )
        .unwrap();
    assert_eq!(model.service(ModelSource::First, false, true), id);
    let (command, request) = model.submit_ingress_onset(&mut control, id).unwrap();
    assert_eq!(
        model.offer_release_at(
            ModelSource::First,
            input(0x80, 60, 0),
            Some(SampleTime::new(150)),
        ),
        Ok(ReleaseOffer::Queued(id))
    );
    assert_eq!(
        model.service_release(ModelSource::First),
        ReleaseRoute {
            id,
            raw: false,
            ingress: true,
        }
    );
    assert_eq!(
        model.settle_ingress_release(id),
        Err(IngressResultBindError::State)
    );
    audio.service_test_ingress_queue();
    let result = control.collect_ingress_result().unwrap();
    assert_eq!((result.id, result.request), (command, request));
    model.apply_ingress_onset_result(id, result).unwrap();
    assert!(matches!(
        model.entry_mut(id).ingress_disposition,
        IngressDisposition::Accepted(_)
    ));
    let (release_command, release) = model.submit_ingress_release(&mut control, id).unwrap();
    assert_eq!(
        model.settle_ingress_release(id),
        Err(IngressResultBindError::State)
    );
    audio.service_test_ingress_queue();
    let release_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        release_result,
        MixedIngressResult {
            id: release_command,
            request: release,
            outcome: MixedIngressOutcome::Release(Ok(())),
        }
    );
    assert_eq!(
        model.apply_ingress_release_result(
            id,
            MixedIngressResult {
                id: command,
                ..release_result
            }
        ),
        Err(IngressResultBindError::WrongCommand)
    );
    assert_eq!(
        model.apply_ingress_release_result(
            id,
            MixedIngressResult {
                request,
                ..release_result
            }
        ),
        Err(IngressResultBindError::WrongRequest)
    );
    assert_eq!(
        model.settle_ingress_release(id),
        Err(IngressResultBindError::State)
    );
    model
        .apply_ingress_release_result(id, release_result)
        .unwrap();
    model.settle_ingress_release(id).unwrap();
    model.collect_result(id).unwrap();
    assert_eq!(model.used, limits(0));
    assert!(model.entries.is_empty());
    assert_eq!(
        model.apply_ingress_release_result(id, release_result),
        Err(IngressResultBindError::State)
    );
    assert_eq!(
        model.settle_ingress_release(id),
        Err(IngressResultBindError::State)
    );
}

#[test]
fn mixed_refusal_after_source_release_reaps_settled_entry() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let first = MixedIngressRequest::Onset {
        origin: MixedIngressOriginId(999),
        at: SampleTime::new(150),
        key: KeyIdentity::new(60).unwrap(),
        velocity: NoteVelocity::FULL,
    };
    let first_command = control.submit_ingress(first).unwrap();
    audio.service_test_ingress_queue();
    let first_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (first_result.id, first_result.request),
        (first_command, first)
    );
    assert!(matches!(
        first_result.outcome,
        MixedIngressOutcome::Onset(Ok(_))
    ));

    let mut model = Model::new(limits(1), 2);
    let id = model
        .submit_at(
            ModelSource::Second,
            input(0x90, 60, 110),
            Some(SampleTime::new(140)),
        )
        .unwrap();
    assert_eq!(model.service(ModelSource::Second, false, true), id);
    let (command, request) = model.submit_ingress_onset(&mut control, id).unwrap();
    assert_eq!(
        model.offer_release_at(
            ModelSource::Second,
            input(0x80, 60, 0),
            Some(SampleTime::new(160)),
        ),
        Ok(ReleaseOffer::Queued(id))
    );
    assert_eq!(
        model.service_release(ModelSource::Second),
        ReleaseRoute {
            id,
            raw: false,
            ingress: true,
        }
    );
    model.collect_result(id).unwrap();
    assert_eq!(model.used.ingress, 1);
    assert_eq!(model.used.ledger, 1);
    audio.service_test_ingress_queue();
    let result = control.collect_ingress_result().unwrap();
    assert_eq!((result.id, result.request), (command, request));
    assert_eq!(
        result.outcome,
        MixedIngressOutcome::Onset(Err(IngressRefused::NonMonotoneStamp {
            time: SampleTime::new(140),
            last: SampleTime::new(150),
        }))
    );
    model.apply_ingress_onset_result(id, result).unwrap();
    assert_eq!(model.used, limits(0));
    assert!(model.entries.is_empty());
}

#[test]
fn mixed_onset_refusal_after_halt_retains_ingress_credit() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let first = MixedIngressRequest::Onset {
        origin: MixedIngressOriginId(999),
        at: SampleTime::new(150),
        key: KeyIdentity::new(60).unwrap(),
        velocity: NoteVelocity::FULL,
    };
    let first_command = control.submit_ingress(first).unwrap();
    audio.service_test_ingress_queue();
    let first_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (first_result.id, first_result.request),
        (first_command, first)
    );
    assert!(matches!(
        first_result.outcome,
        MixedIngressOutcome::Onset(Ok(_))
    ));

    let mut model = Model::new(limits(1), 2);
    let id = model
        .submit_at(
            ModelSource::Second,
            input(0x90, 60, 110),
            Some(SampleTime::new(140)),
        )
        .unwrap();
    assert_eq!(model.service(ModelSource::Second, false, true), id);
    let (command, request) = model.submit_ingress_onset(&mut control, id).unwrap();
    let prior = input(0x91, 61, 100);
    model.fault(ModelSource::First, prior, TerminalReason::Order);
    audio.service_test_ingress_queue();
    let result = control.collect_ingress_result().unwrap();
    assert_eq!((result.id, result.request), (command, request));
    let reason = IngressRefused::NonMonotoneStamp {
        time: SampleTime::new(140),
        last: SampleTime::new(150),
    };
    assert_eq!(result.outcome, MixedIngressOutcome::Onset(Err(reason)));
    model.apply_ingress_onset_result(id, result).unwrap();
    assert_eq!(
        model.entry_mut(id).ingress_disposition,
        IngressDisposition::Refused(reason)
    );
    assert_eq!(model.used.ingress, 1);
    assert_eq!(model.used.results, 1);
    assert_eq!(model.used.ledger, 1);
    assert_eq!(model.collect_result(id), Err(IngressResultBindError::State));
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::First,
            TerminalFault {
                original: prior,
                reason: TerminalReason::Order,
            }
        ))
    );
}

#[test]
fn stamped_onset_refuses_unstamped_release_before_source_queue_custody() {
    let mut model = Model::new(limits(1), 2);
    let id = model
        .submit_at(
            ModelSource::First,
            input(0x90, 60, 100),
            Some(SampleTime::new(140)),
        )
        .unwrap();
    assert_eq!(model.service(ModelSource::First, false, true), id);
    let original = input(0x80, 60, 0);
    assert_eq!(
        model.offer_release(ModelSource::First, original),
        Err(ReleaseOfferError::MissingStamp(original))
    );
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::First,
            TerminalFault {
                original,
                reason: TerminalReason::MissingReleaseStamp,
            },
        ))
    );
    assert!(model.rings[ModelSource::First.index()].is_empty());
    assert!(!model.source_holds[0].release_queued);
    assert_eq!(model.used.ingress, 1);
    assert_eq!(
        model.offer_release_at(ModelSource::First, original, Some(SampleTime::new(150))),
        Err(ReleaseOfferError::Halted(original))
    );
}

#[test]
fn retired_stamped_onset_refuses_unstamped_release_without_spending_tombstone() {
    let mut model = Model::new(limits(2), 3);
    model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .unwrap();
    let retry = match model.submit_at(
        ModelSource::First,
        input(0x90, 61, 100),
        Some(SampleTime::new(150)),
    ) {
        Err(SubmitError::Retry(token)) => token,
        other => panic!("stamped onset must wait behind first ring packet: {other:?}"),
    };
    let retired = model.retire(retry).unwrap();
    let original = input(0x80, 61, 0);
    assert_eq!(
        model.offer_release(ModelSource::First, original),
        Err(ReleaseOfferError::MissingStamp(original))
    );
    assert!(model.source_holds.iter().any(|hold| hold.id == retired.id));
    assert_eq!(model.used.ledger, 2);
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::First,
            TerminalFault {
                original,
                reason: TerminalReason::MissingReleaseStamp,
            },
        ))
    );
}

#[test]
fn blocked_unstamped_release_faults_after_stamped_onset_retirement() {
    let mut model = Model::new(limits(2), 3);
    model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .unwrap();
    let retry = match model.submit_at(
        ModelSource::First,
        input(0x90, 61, 100),
        Some(SampleTime::new(150)),
    ) {
        Err(SubmitError::Retry(token)) => token,
        other => panic!("stamped onset must wait behind first ring packet: {other:?}"),
    };
    let original = input(0x80, 61, 0);
    let release_token = match model.offer_release(ModelSource::First, original) {
        Err(ReleaseOfferError::Blocked(token)) => token,
        other => panic!("release must wait behind stamped onset retry: {other:?}"),
    };
    let retired = model.retire(retry).unwrap();
    let expected_token = ReleaseRetryToken {
        id: release_token.id,
        source: release_token.source,
    };
    assert_eq!(
        model.retry_release(release_token),
        Err(ReleaseRetryError::MissingStamp(expected_token))
    );
    assert!(model.source_holds.iter().any(|hold| hold.id == retired.id));
    assert!(model.release_pending[ModelSource::First.index()].is_none());
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::First,
            TerminalFault {
                original,
                reason: TerminalReason::MissingReleaseStamp,
            },
        ))
    );
}

#[test]
fn terminal_halt_keeps_ledger_cell_when_raw_credit_settles_later() {
    let mut model = Model::new(limits(1), 2);
    let id = model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .unwrap();
    assert_eq!(model.service(ModelSource::First, true, false), id);
    model.collect_result(id).unwrap();
    assert_eq!(
        model.offer_release(ModelSource::First, input(0x80, 60, 0)),
        Ok(ReleaseOffer::Queued(id))
    );
    assert_eq!(model.service_release(ModelSource::First).id, id);
    model.fault(
        ModelSource::Second,
        input(0x91, 61, 100),
        TerminalReason::Order,
    );
    model.settle_fake_raw_release(id).unwrap();
    assert_eq!(model.used.tracker, 0);
    assert_eq!(model.used.ledger, 1);
    assert_eq!(model.entries.len(), 1);
}

#[test]
fn refused_mixed_release_retains_original_and_halts_model_credit() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut model = Model::new(limits(2), 4);
    let id = model
        .submit_at(
            ModelSource::First,
            input(0x90, 60, 100),
            Some(SampleTime::new(140)),
        )
        .unwrap();
    assert_eq!(model.service(ModelSource::First, false, true), id);
    let (onset_command, onset_request) = model.submit_ingress_onset(&mut control, id).unwrap();
    audio.service_test_ingress_queue();
    let onset_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (onset_result.id, onset_result.request),
        (onset_command, onset_request)
    );
    model.apply_ingress_onset_result(id, onset_result).unwrap();

    let peer = model
        .submit_at(
            ModelSource::Second,
            input(0x90, 62, 100),
            Some(SampleTime::new(150)),
        )
        .unwrap();
    assert_eq!(model.next_merged_onset(), None);
    assert_eq!(model.service(ModelSource::Second, false, true), peer);
    let (peer_command, peer_request) = model.submit_ingress_onset(&mut control, peer).unwrap();
    audio.service_test_ingress_queue();
    let peer_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (peer_result.id, peer_result.request),
        (peer_command, peer_request)
    );
    model.apply_ingress_onset_result(peer, peer_result).unwrap();

    let original = input(0x80, 60, 0);
    assert_eq!(
        model.offer_release_at(ModelSource::First, original, Some(SampleTime::new(145))),
        Ok(ReleaseOffer::Queued(id))
    );
    assert_eq!(model.service_release(ModelSource::First).id, id);
    let (command, request) = model.submit_ingress_release(&mut control, id).unwrap();
    assert_eq!(
        request,
        MixedIngressRequest::Release {
            origin: MixedIngressOriginId(id.0),
            at: SampleTime::new(145),
            identity: match model.entry_mut(id).ingress_disposition {
                IngressDisposition::Accepted(identity) => identity,
                other => panic!("accepted identity required, got {other:?}"),
            },
        }
    );
    audio.service_test_ingress_queue();
    let result = control.collect_ingress_result().unwrap();
    assert_eq!((result.id, result.request), (command, request));
    let reason = IngressRefused::NonMonotoneStamp {
        time: SampleTime::new(145),
        last: SampleTime::new(150),
    };
    assert_eq!(result.outcome, MixedIngressOutcome::Release(Err(reason)));
    model.apply_ingress_release_result(id, result).unwrap();
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::First,
            TerminalFault {
                original,
                reason: TerminalReason::IngressRelease(reason),
            },
        ))
    );
    assert_eq!(
        model.submit_ingress_release(&mut control, id),
        Err(IngressReleaseSubmitError::TerminalRefused(reason))
    );
    assert_eq!(
        model.settle_ingress_release(id),
        Err(IngressResultBindError::State)
    );
    assert_eq!(model.used.ingress, 2);
    assert_eq!(model.used.ledger, 2);
    assert!(matches!(
        model.submit(ModelSource::Second, input(0x90, 61, 100)),
        Err(SubmitError::Halted(_))
    ));
}

#[test]
fn later_mixed_release_refusal_keeps_prior_terminal_fault_and_local_original() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut model = Model::new(limits(2), 4);
    let id = model
        .submit_at(
            ModelSource::First,
            input(0x90, 60, 100),
            Some(SampleTime::new(140)),
        )
        .unwrap();
    assert_eq!(model.service(ModelSource::First, false, true), id);
    let (onset_command, onset_request) = model.submit_ingress_onset(&mut control, id).unwrap();
    audio.service_test_ingress_queue();
    let onset_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (onset_result.id, onset_result.request),
        (onset_command, onset_request)
    );
    model.apply_ingress_onset_result(id, onset_result).unwrap();
    let peer = model
        .submit_at(
            ModelSource::Second,
            input(0x90, 62, 100),
            Some(SampleTime::new(150)),
        )
        .unwrap();
    assert_eq!(model.next_merged_onset(), None);
    assert_eq!(model.service(ModelSource::Second, false, true), peer);
    let (peer_command, peer_request) = model.submit_ingress_onset(&mut control, peer).unwrap();
    audio.service_test_ingress_queue();
    let peer_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (peer_result.id, peer_result.request),
        (peer_command, peer_request)
    );
    model.apply_ingress_onset_result(peer, peer_result).unwrap();
    let original = input(0x80, 60, 0);
    assert_eq!(
        model.offer_release_at(ModelSource::First, original, Some(SampleTime::new(145))),
        Ok(ReleaseOffer::Queued(id))
    );
    assert_eq!(model.service_release(ModelSource::First).id, id);
    let (release_command, release_request) =
        model.submit_ingress_release(&mut control, id).unwrap();

    let earlier_fault = input(0x91, 61, 100);
    model.fault(ModelSource::Second, earlier_fault, TerminalReason::Order);
    audio.service_test_ingress_queue();
    let result = control.collect_ingress_result().unwrap();
    assert_eq!(
        (result.id, result.request),
        (release_command, release_request)
    );
    let reason = IngressRefused::NonMonotoneStamp {
        time: SampleTime::new(145),
        last: SampleTime::new(150),
    };
    assert_eq!(result.outcome, MixedIngressOutcome::Release(Err(reason)));
    model.apply_ingress_release_result(id, result).unwrap();
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::Second,
            TerminalFault {
                original: earlier_fault,
                reason: TerminalReason::Order,
            }
        ))
    );
    let entry = model.entry_mut(id);
    assert_eq!(
        entry.source_release,
        Some((original, Some(SampleTime::new(145))))
    );
    assert_eq!(entry.ingress_release_result, Some(Err(reason)));
    assert_eq!(model.collect_result(id), Err(IngressResultBindError::State));
    assert_eq!(
        model.settle_ingress_release(id),
        Err(IngressResultBindError::State)
    );
    assert_eq!(model.used.ingress, 2);
    assert_eq!(model.used.ledger, 2);
}

#[test]
fn equal_source_payloads_keep_distinct_mixed_origins_and_results() {
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut model = Model::new(limits(2), 4);
    let note = input(0x90, 60, 100);
    let at = Some(SampleTime::new(140));
    let first = model.submit_at(ModelSource::First, note, at).unwrap();
    let second = model.submit_at(ModelSource::Second, note, at).unwrap();
    assert_eq!(model.service(ModelSource::First, false, true), first);
    assert_eq!(model.service(ModelSource::Second, false, true), second);
    let (first_command, first_request) = model.submit_ingress_onset(&mut control, first).unwrap();
    let (second_command, second_request) =
        model.submit_ingress_onset(&mut control, second).unwrap();
    let MixedIngressRequest::Onset {
        origin: first_origin,
        at: first_at,
        key: first_key,
        velocity: first_velocity,
    } = first_request
    else {
        panic!("first source must submit an onset");
    };
    let MixedIngressRequest::Onset {
        origin: second_origin,
        at: second_at,
        key: second_key,
        velocity: second_velocity,
    } = second_request
    else {
        panic!("second source must submit an onset");
    };
    assert_eq!(
        (first_at, first_key, first_velocity),
        (second_at, second_key, second_velocity)
    );
    assert_eq!(first_origin, MixedIngressOriginId(first.0));
    assert_eq!(second_origin, MixedIngressOriginId(second.0));
    assert_ne!(first_origin, second_origin);
    assert_ne!(first_command, second_command);
    audio.service_test_ingress_queue();
    let first_result = control.collect_ingress_result().unwrap();
    let second_result = control.collect_ingress_result().unwrap();
    assert_eq!(
        model.apply_ingress_onset_result(second, first_result),
        Err(IngressResultBindError::WrongCommand)
    );
    model
        .apply_ingress_onset_result(first, first_result)
        .unwrap();
    model
        .apply_ingress_onset_result(second, second_result)
        .unwrap();
    let IngressDisposition::Accepted(first_identity) = model.entry_mut(first).ingress_disposition
    else {
        panic!("first equal-valued onset must be accepted");
    };
    let IngressDisposition::Accepted(second_identity) = model.entry_mut(second).ingress_disposition
    else {
        panic!("second equal-valued onset must be accepted");
    };
    assert_ne!(first_identity, second_identity);
}

#[test]
fn mixed_hold_refusal_and_later_fault_leave_serial_capture_receipts_independent() {
    let (mut owner, generations) = bridge_serial_owner(256);
    let (prepared, candidate) = history_tests::one_shot_with_boundary_on(true);
    let (mut control, mut mixed) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut model = Model::new(limits(3), 4).with_raw_capacity(InputCapacity::new(8).unwrap());
    let onsets = [
        (ModelSource::First, input(0x90, 60, 100), 140),
        (ModelSource::Second, input(0x90, 60, 110), 141),
        (ModelSource::First, input(0x90, 60, 120), 142),
    ];
    let ids = onsets.map(|(source, note, time)| {
        model
            .submit_at(source, note, Some(SampleTime::new(time)))
            .unwrap()
    });
    let mut admitted = Vec::new();
    for (source, note, time) in onsets {
        let (id, key, queued) = model.service_with_input(source, true, true);
        assert_eq!(queued, note);
        let at = SampleTime::new(time);
        let raw_id = owner
            .offer_message(
                generations[source.index()],
                InputTick::new(time),
                at,
                queued,
            )
            .unwrap();
        model.bind_raw_onset(id, raw_id).unwrap();
        admitted.push((id, key, raw_id));
        assert_eq!(queued.channel(), key.channel);
        let (command, request) = model.submit_ingress_onset(&mut control, id).unwrap();
        mixed.service_test_ingress_queue();
        let result = control.collect_ingress_result().unwrap();
        assert_eq!((result.id, result.request), (command, request));
        match result.outcome {
            MixedIngressOutcome::Onset(Ok(_)) => {
                assert_ne!(id, ids[2], "third onset must hit the mixed hold limit");
            }
            MixedIngressOutcome::Onset(Err(IngressRefused::Dropped {
                resource: ExhaustedResource::Hold,
            })) if id == ids[2] => {}
            other => panic!("unexpected mixed onset disposition: {other:?}"),
        }
        model.apply_ingress_onset_result(id, result).unwrap();
    }
    assert_raw_pressure(&model, &owner, generations);
    assert_eq!(mixed.test_ingress.counters().dropped_hold(), 1);
    assert_eq!(
        model.entry_mut(ids[2]).ingress_disposition,
        IngressDisposition::Refused(IngressRefused::Dropped {
            resource: ExhaustedResource::Hold,
        })
    );
    assert_eq!(
        model.used,
        Counts {
            tracker: 3,
            ingress: 2,
            results: 3,
            ledger: 3,
        }
    );
    let releases = [
        (ModelSource::First, input(0x90, 60, 0), ids[0], 150),
        (ModelSource::Second, input(0x80, 60, 0), ids[1], 151),
        (ModelSource::First, input(0x80, 60, 0), ids[2], 152),
    ];
    for (source, release, id, time) in releases {
        assert_eq!(
            model.offer_release_at(source, release, Some(SampleTime::new(time))),
            Ok(ReleaseOffer::Queued(id))
        );
    }
    let mut routed = Vec::new();
    for (source, release, expected, time) in releases {
        let (route, key, queued) = model.service_release_with_input(source);
        assert_eq!(route.id, expected);
        assert_eq!(queued, release);
        assert_eq!(queued.channel(), key.channel);
        assert!(
            matches!(queued.event(), Midi1Event::KeyRelease { key: released, .. } if released == key.note)
        );
        assert!(route.raw);
        assert_eq!(route.ingress, route.id != ids[2]);
        let at = SampleTime::new(time);
        let raw_id = owner
            .offer_message(
                generations[source.index()],
                InputTick::new(time),
                at,
                queued,
            )
            .unwrap();
        model.admit_raw_release(route.id, raw_id);
        if route.ingress {
            let (command, request) = model
                .submit_ingress_release(&mut control, route.id)
                .unwrap();
            mixed.service_test_ingress_queue();
            let result = control.collect_ingress_result().unwrap();
            assert_eq!((result.id, result.request), (command, request));
            assert_eq!(result.outcome, MixedIngressOutcome::Release(Ok(())));
            model
                .apply_ingress_release_result(route.id, result)
                .unwrap();
        }
        routed.push((route.id, key, raw_id));
    }
    assert_raw_pressure(&model, &owner, generations);
    assert_eq!(mixed.test_ingress.holds_outstanding(), EventCount::NONE);
    let mut first = [0.0_f32; 128];
    mixed
        .render_private(AudioBlockMut::new(&mut first, 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert!(!mixed.report().faulted);
    mixed.test_fail_after_ingress_at = Some(SampleTime::new(128));
    let mut second = [1.0_f32; 128];
    assert_eq!(
        mixed.render_private(AudioBlockMut::new(&mut second, 128, ChannelLayout::Mono).unwrap()),
        Err(MixedOneShotRenderError::InjectedAfterIngress)
    );
    assert!(second.iter().all(|sample| *sample == 0.0));
    let MixedCollection::Ended(ended) = control.collect(mixed).unwrap() else {
        panic!("faulted mixed owner must be classified at teardown");
    };
    assert_eq!(ended.end, MixedCollectionEnd::Faulted);
    assert!(ended.ingress.queued.is_empty());
    assert_eq!(ended.ingress.charged_in_faulted_callback.len(), 4);
    assert_eq!(ended.ingress.counters.dropped_hold(), 1);
    assert_eq!(ended.ingress.holds_outstanding, EventCount::NONE);
    assert!(ended.ingress.minted_live.is_empty());
    assert!(ended.sounding.live().is_empty());
    let mut ingress_identity = |id| {
        let IngressDisposition::Accepted(identity) = model.entry_mut(id).ingress_disposition else {
            panic!("accepted mixed onset lost its identity");
        };
        identity
    };
    assert_eq!(
        ended
            .ingress
            .charged_in_faulted_callback
            .iter()
            .map(|(event, redeems)| {
                let EventPayload::Note { identity, edge } = event.payload() else {
                    panic!("mixed fault journal contains only note edges");
                };
                (
                    event.envelope().time(),
                    identity,
                    matches!(edge, NoteEdge::Off),
                    *redeems,
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (SampleTime::new(140), ingress_identity(ids[0]), false, false),
            (SampleTime::new(141), ingress_identity(ids[1]), false, false),
            (SampleTime::new(150), ingress_identity(ids[0]), true, true),
            (SampleTime::new(151), ingress_identity(ids[1]), true, true),
        ]
    );

    for (source, generation) in [ModelSource::First, ModelSource::Second]
        .into_iter()
        .zip(generations)
    {
        let _frontier = owner
            .advance_frontier(generation, InputTick::new(256))
            .unwrap();
        model.admit_raw_frontier(source);
    }
    assert_raw_pressure(&model, &owner, generations);
    owner.pump().unwrap();
    let mut samples = [0.0_f32; 512];
    owner
        .render(AudioBlockMut::new(&mut samples, 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    owner.pump().unwrap();
    let receipts: Vec<_> = generations
        .into_iter()
        .flat_map(|generation| {
            std::iter::from_fn(|| owner.collect_input(generation).unwrap()).collect::<Vec<_>>()
        })
        .collect();
    for (source, generation) in [ModelSource::First, ModelSource::Second]
        .into_iter()
        .zip(generations)
    {
        model.collect_raw_receipts(
            source,
            receipts
                .iter()
                .filter(|receipt| receipt.id.generation() == generation)
                .count(),
        );
    }
    assert_raw_pressure(&model, &owner, generations);
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(receipts.len(), 10);
    for (id, key, release_raw) in routed {
        let (_, original_key, onset_raw) = admitted
            .iter()
            .find(|(candidate, _, _)| *candidate == id)
            .unwrap();
        assert_eq!(*original_key, key);
        let onset_receipt = receipts
            .iter()
            .find(|receipt| receipt.id == *onset_raw)
            .unwrap();
        let release_receipt = receipts
            .iter()
            .find(|receipt| receipt.id == release_raw)
            .unwrap();
        assert_eq!(
            model.settle_delivered_raw_release(id, onset_receipt, release_receipt),
            Ok(())
        );
    }
    // The private owner has no per-occurrence outcome after the fault. The
    // model retains these ingress credits without a joined redemption rule.
    assert_eq!(
        model.used,
        Counts {
            tracker: 0,
            ingress: 2,
            results: 3,
            ledger: 3,
        }
    );
}

#[test]
fn same_key_fifo_retains_accepted_then_refused_occurrences() {
    let mut model = Model::new(limits(3), 4);
    let first = model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("first queue custody");
    let second = model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("second queue custody");
    assert_eq!(model.service(ModelSource::First, true, true), first);
    assert_eq!(model.service(ModelSource::First, false, false), second);
    let first_route = model
        .release(ModelSource::First, input(0x90, 60, 0))
        .expect("velocity-zero release");
    assert_eq!(
        first_route,
        ReleaseRoute {
            id: first,
            raw: true,
            ingress: true
        }
    );
    let second_route = model
        .release(ModelSource::First, input(0x80, 60, 0))
        .expect("refused tombstone release");
    assert_eq!(
        second_route,
        ReleaseRoute {
            id: second,
            raw: false,
            ingress: false
        }
    );
    assert_eq!(model.release(ModelSource::First, input(0x80, 60, 0)), None);
}

#[test]
fn stray_release_from_other_source_cannot_spend_same_key_credit() {
    let mut model = Model::new(limits(1), 2);
    let held = model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("queue custody");
    model.service(ModelSource::First, true, true);
    let foreign = input(0x80, 60, 0);
    assert_eq!(
        model.offer_release(ModelSource::Second, foreign),
        Err(ReleaseOfferError::Unmatched(foreign))
    );
    assert_eq!(model.release(ModelSource::Second, input(0x80, 60, 0)), None);
    assert_eq!(model.used, limits(1));
    assert_eq!(
        model.release(ModelSource::First, input(0x80, 60, 0)),
        Some(ReleaseRoute {
            id: held,
            raw: true,
            ingress: true,
        })
    );
}

#[test]
fn partial_consumer_admission_keeps_each_release_path() {
    let mut model = Model::new(limits(2), 4);
    let raw_only = model
        .submit(ModelSource::First, input(0x90, 61, 100))
        .expect("first queue custody");
    let ingress_only = model
        .submit(ModelSource::First, input(0x90, 61, 100))
        .expect("second queue custody");
    model.service(ModelSource::First, true, false);
    model.service(ModelSource::First, false, true);
    assert_eq!(
        model.release(ModelSource::First, input(0x80, 61, 0)),
        Some(ReleaseRoute {
            id: raw_only,
            raw: true,
            ingress: false
        })
    );
    assert_eq!(
        model.release(ModelSource::First, input(0x80, 61, 0)),
        Some(ReleaseRoute {
            id: ingress_only,
            raw: false,
            ingress: true
        })
    );
    model.settle_fake_raw_release(raw_only).unwrap();
    model.settle_fake_ingress_release(ingress_only);
    assert_eq!(model.used.tracker, 0);
    assert_eq!(model.used.ingress, 0);
}

#[test]
fn reserved_release_traverses_full_source_fifo_after_ordinary_prefix() {
    let mut model = Model::new(limits(1), 4);
    let held = model
        .submit(ModelSource::First, input(0x90, 62, 100))
        .expect("onset and release reservation");
    model.service(ModelSource::First, true, true);
    for _ in 0..3 {
        assert!(model.can_offer_ordinary(ModelSource::First));
        model.offer_ordinary(ModelSource::First);
    }
    assert!(!model.can_offer_ordinary(ModelSource::First));
    assert_eq!(model.release_reservations, [1, 0]);
    assert_eq!(
        model.offer_release(ModelSource::First, input(0x80, 62, 0)),
        Ok(ReleaseOffer::Queued(held))
    );
    assert_eq!(model.release_reservations, [0, 0]);
    assert_eq!(model.rings[ModelSource::First.index()].len(), 4);
    for _ in 0..3 {
        model.service_ordinary(ModelSource::First);
    }
    assert_eq!(
        model.service_release(ModelSource::First),
        ReleaseRoute {
            id: held,
            raw: true,
            ingress: true,
        }
    );
    assert!(model.rings[ModelSource::First.index()].is_empty());
}

#[test]
fn two_same_key_release_reservations_redeem_in_source_order() {
    let mut model = Model::new(limits(2), 4);
    let first = model
        .submit(ModelSource::First, input(0x90, 62, 100))
        .expect("first reserved onset");
    let second = model
        .submit(ModelSource::First, input(0x90, 62, 100))
        .expect("second reserved onset");
    assert_eq!(model.release_reservations, [2, 0]);
    assert!(!model.can_offer_ordinary(ModelSource::First));
    model.service(ModelSource::First, true, true);
    model.service(ModelSource::First, false, false);
    assert!(model.can_offer_ordinary(ModelSource::First));
    model.offer_ordinary(ModelSource::First);
    assert!(model.can_offer_ordinary(ModelSource::First));
    model.offer_ordinary(ModelSource::First);
    assert!(!model.can_offer_ordinary(ModelSource::First));
    assert_eq!(
        model.offer_release(ModelSource::First, input(0x80, 62, 0)),
        Ok(ReleaseOffer::Queued(first))
    );
    assert_eq!(
        model.offer_release(ModelSource::First, input(0x90, 62, 0)),
        Ok(ReleaseOffer::Queued(second))
    );
    assert_eq!(model.release_reservations, [0, 0]);
    assert_eq!(model.rings[ModelSource::First.index()].len(), 4);
    model.service_ordinary(ModelSource::First);
    model.service_ordinary(ModelSource::First);
    assert_eq!(model.service_release(ModelSource::First).id, first);
    assert_eq!(model.service_release(ModelSource::First).id, second);
    assert!(model.rings[ModelSource::First.index()].is_empty());
}

#[test]
fn nondrainable_reservation_shortage_is_not_a_retry() {
    let mut model = Model::new(limits(3), 3);
    let first = model
        .submit(ModelSource::First, input(0x90, 70, 100))
        .expect("first onset");
    model.service(ModelSource::First, true, true);
    let second = model
        .submit(ModelSource::First, input(0x90, 71, 100))
        .expect("second onset");
    model.service(ModelSource::First, true, true);
    assert!(model.rings[ModelSource::First.index()].is_empty());
    assert_eq!(model.release_reservations, [2, 0]);
    let refused = input(0x90, 72, 100);
    assert!(matches!(
        model.submit(ModelSource::First, refused),
        Err(SubmitError::NoCredit(original, NoCreditReason::SourceReserve))
            if original == refused
    ));
    assert_eq!(model.used, limits(2));
    assert!(model.retry_pending[ModelSource::First.index()].is_none());
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::First,
            TerminalFault {
                original: refused,
                reason: TerminalReason::NoCredit(NoCreditReason::SourceReserve),
            }
        ))
    );
    assert_eq!(model.entries[0].id, first);
    assert_eq!(model.entries[1].id, second);
    assert_eq!(model.release_reservations, [2, 0]);
    assert!(model.host_halted);
    assert_eq!(
        TerminalReason::NoCredit(NoCreditReason::SourceReserve).origin(),
        CauseOrigin::SourceLocal
    );
}

#[test]
fn retry_retains_shared_charge_until_exact_retry_or_retirement() {
    let mut model = Model::new(limits(4), 3);
    let first = model
        .submit(ModelSource::First, input(0x90, 62, 100))
        .expect("first queue custody");
    let retry = match model.submit(ModelSource::First, input(0x90, 63, 100)) {
        Err(SubmitError::Retry(retry)) => retry,
        _ => panic!("full ring must return owned retry"),
    };
    assert_eq!(model.used, limits(2));
    let blocked_release = input(0x80, 62, 0);
    let blocked_token = match model.offer_release(ModelSource::First, blocked_release) {
        Err(ReleaseOfferError::Blocked(token)) => token,
        _ => panic!("release behind retry must retain an owned token"),
    };
    assert_eq!(
        model.release_pending[ModelSource::First.index()]
            .as_ref()
            .map(|pending| pending.input),
        Some(blocked_release)
    );
    let pending_release = input(0x80, 63, 0);
    assert!(!model.can_offer_ordinary(ModelSource::First));
    assert_eq!(model.release_reservations, [1, 0]);
    let retry = match model.retry(retry) {
        Err(RetryError::Blocked(token)) => token,
        _ => panic!("full ring retains blocked token"),
    };
    assert_eq!(model.service(ModelSource::First, true, true), first);
    let stale = RetryToken {
        id: first,
        source: ModelSource::First,
    };
    assert!(matches!(model.retry(stale), Err(RetryError::Stale(_))));
    let wrong_source = RetryToken {
        id: retry.id,
        source: ModelSource::Second,
    };
    assert!(matches!(
        model.retry(wrong_source),
        Err(RetryError::Stale(_))
    ));
    assert_eq!(
        model.retry_pending[ModelSource::First.index()]
            .as_ref()
            .map(|pending| pending.attempt.id),
        Some(retry.id)
    );
    let retried = model.retry(retry).expect("space available");
    assert_eq!(
        model.rings[ModelSource::First.index()]
            .front()
            .and_then(|packet| match packet {
                RingPacket::Onset(attempt) => Some(attempt.input),
                _ => None,
            }),
        Some(input(0x90, 63, 100))
    );
    let wrong_release_token = ReleaseRetryToken {
        id: ReleaseAttemptId(blocked_token.id.0 + 1),
        source: ModelSource::First,
    };
    assert!(matches!(
        model.retry_release(wrong_release_token),
        Err(ReleaseRetryError::Stale(_))
    ));
    let wrong_source_token = ReleaseRetryToken {
        id: blocked_token.id,
        source: ModelSource::Second,
    };
    assert!(matches!(
        model.retry_release(wrong_source_token),
        Err(ReleaseRetryError::Stale(_))
    ));
    assert_eq!(
        model.release_pending[ModelSource::First.index()]
            .as_ref()
            .map(|pending| pending.input),
        Some(blocked_release)
    );
    assert_eq!(
        model.retry_release(blocked_token),
        Ok(ReleaseOffer::Queued(first))
    );
    assert_eq!(model.service(ModelSource::First, false, false), retried);
    assert_eq!(model.service_release(ModelSource::First).id, first);
    assert_eq!(
        model.used,
        Counts {
            tracker: 1,
            ingress: 1,
            results: 2,
            ledger: 2,
        }
    );
    assert_eq!(
        model
            .release(ModelSource::First, pending_release)
            .map(|route| route.id),
        Some(retried)
    );

    let queued = model
        .submit(ModelSource::First, input(0x90, 65, 100))
        .expect("next queue custody");
    let (retired, older_token) = match model.submit(ModelSource::First, input(0x90, 66, 100)) {
        Err(SubmitError::Retry(retry)) => {
            let older_release = input(0x80, 65, 0);
            let older_token = match model.offer_release(ModelSource::First, older_release) {
                Err(ReleaseOfferError::Blocked(token)) => token,
                _ => panic!("older release must wait behind the retry"),
            };
            let stale = RetryToken {
                id: queued,
                source: ModelSource::First,
            };
            assert!(model.retire(stale).is_err());
            (model.retire(retry).expect("exact retirement"), older_token)
        }
        _ => panic!("full ring must return owned retry"),
    };
    assert_eq!(retired.id, OccurrenceId(queued.0 + 1));
    assert_eq!(retired.key.source, ModelSource::First);
    assert_eq!(retired.key.note, KeyIdentity::new(66).expect("test key"));
    assert_eq!(retired.input, input(0x90, 66, 100));
    assert_eq!(
        retired.returned,
        Counts {
            tracker: 1,
            ingress: 1,
            results: 1,
            ledger: 0,
        }
    );
    assert_eq!(
        model.used,
        Counts {
            tracker: 2,
            ingress: 2,
            results: 3,
            ledger: 4,
        }
    );
    assert_eq!(
        model.retry_release(older_token),
        Ok(ReleaseOffer::Queued(queued))
    );
    assert_eq!(model.service(ModelSource::First, false, false), queued);
    assert_eq!(model.service_release(ModelSource::First).id, queued);
    let retired_release = input(0x80, 66, 0);
    assert_eq!(
        model.offer_release(ModelSource::First, retired_release),
        Ok(ReleaseOffer::Retired(retired.id))
    );
    assert!(
        model
            .submit(ModelSource::First, input(0x90, 67, 100))
            .is_ok()
    );
}

#[test]
fn new_offer_behind_pending_custody_is_terminal_order_fault() {
    for later_is_onset in [false, true] {
        let mut model = Model::new(limits(3), 3);
        model
            .submit(ModelSource::First, input(0x90, 60, 100))
            .expect("earlier onset");
        let retry = match model.submit(ModelSource::First, input(0x90, 63, 100)) {
            Err(SubmitError::Retry(token)) => token,
            _ => panic!("pending onset must wait for ring headroom"),
        };
        let fault_original = if later_is_onset {
            let original = input(0x90, 64, 100);
            assert!(matches!(
                model.submit(ModelSource::First, original),
                Err(SubmitError::Order(returned)) if returned == original
            ));
            original
        } else {
            let first_release = input(0x80, 60, 0);
            let pending_token = match model.offer_release(ModelSource::First, first_release) {
                Err(ReleaseOfferError::Blocked(token)) => token,
                _ => panic!("first release must wait behind onset retry"),
            };
            let later_release = input(0x80, 63, 0);
            assert_eq!(
                model.offer_release(ModelSource::First, later_release),
                Err(ReleaseOfferError::Order(later_release))
            );
            assert!(matches!(
                model.retry_release(pending_token),
                Err(ReleaseRetryError::Halted(_))
            ));
            later_release
        };
        assert!(model.host_halted);
        let retained_fault = Some((
            ModelSource::First,
            TerminalFault {
                original: fault_original,
                reason: TerminalReason::Order,
            },
        ));
        assert_eq!(model.terminal_fault, retained_fault);
        assert_eq!(TerminalReason::Order.origin(), CauseOrigin::SourceLocal);
        let retry = match model.retry(retry) {
            Err(RetryError::Halted(token)) => token,
            _ => panic!("matching retry must report terminal halt"),
        };
        let charged = model.used;
        assert!(matches!(model.retire(retry), Err(RetryError::Halted(_))));
        assert_eq!(model.used, charged);
        assert!(model.retry_pending[ModelSource::First.index()].is_some());
        let after_halt = input(0x90, 65, 100);
        assert!(matches!(
            model.submit(ModelSource::First, after_halt),
            Err(SubmitError::Halted(returned)) if returned == after_halt
        ));
        let later_release = input(0x80, 65, 0);
        assert_eq!(
            model.offer_release(ModelSource::First, later_release),
            Err(ReleaseOfferError::Halted(later_release))
        );
        let peer = input(0x91, 65, 100);
        assert!(matches!(
            model.submit(ModelSource::Second, peer),
            Err(SubmitError::Halted(original)) if original == peer
        ));
        assert_eq!(model.terminal_fault, retained_fault);
    }
}

#[test]
fn onset_behind_pending_release_cannot_overtake_it() {
    let mut model = Model::new(limits(3), 3);
    model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("earlier onset");
    let retry = match model.submit(ModelSource::First, input(0x90, 63, 100)) {
        Err(SubmitError::Retry(token)) => token,
        _ => panic!("later onset must wait for ring headroom"),
    };
    let release_token = match model.offer_release(ModelSource::First, input(0x80, 60, 0)) {
        Err(ReleaseOfferError::Blocked(token)) => token,
        _ => panic!("release must wait behind the onset retry"),
    };
    model.service(ModelSource::First, true, true);
    let retried = model.retry(retry).expect("onset retry can now enter ring");
    assert_eq!(
        model.retry_pending[ModelSource::First.index()]
            .as_ref()
            .map(|pending| pending.attempt.id),
        None
    );
    assert!(model.release_pending[ModelSource::First.index()].is_some());
    let later = input(0x90, 65, 100);
    assert!(matches!(
        model.submit(ModelSource::First, later),
        Err(SubmitError::Order(original)) if original == later
    ));
    assert!(matches!(
        model.retry(RetryToken {
            id: retried,
            source: ModelSource::First,
        }),
        Err(RetryError::Stale(_))
    ));
    assert!(matches!(
        model.retry_release(release_token),
        Err(ReleaseRetryError::Halted(_))
    ));
    assert!(model.release_pending[ModelSource::First.index()].is_some());
}

#[test]
fn release_for_pending_onset_keeps_source_order_on_retry_and_retirement() {
    for retire in [false, true] {
        let mut model = Model::new(limits(2), 3);
        let earlier = model
            .submit(ModelSource::First, input(0x90, 60, 100))
            .expect("earlier queue custody");
        let retry = match model.submit(ModelSource::First, input(0x90, 63, 100)) {
            Err(SubmitError::Retry(token)) => token,
            _ => panic!("pending onset must wait for ring headroom"),
        };
        let pending_input = input(0x90, 63, 0);
        let release_token = match model.offer_release(ModelSource::First, pending_input) {
            Err(ReleaseOfferError::Blocked(token)) => token,
            _ => panic!("pending-key release must be owned"),
        };
        assert_eq!(
            model.release_pending[ModelSource::First.index()]
                .as_ref()
                .map(|pending| pending.input),
            Some(pending_input)
        );
        let release_token = match model.retry_release(release_token) {
            Err(ReleaseRetryError::Blocked(token)) => token,
            _ => panic!("release must wait for earlier onset disposition"),
        };
        if retire {
            let retired = model.retire(retry).expect("retire pending onset");
            let ledger_before = model.used.ledger;
            assert_eq!(
                model.retry_release(release_token),
                Ok(ReleaseOffer::Retired(retired.id))
            );
            assert_eq!(model.used.ledger, ledger_before - 1);
            assert_eq!(model.release_reservations, [1, 0]);
            assert_eq!(model.service(ModelSource::First, true, true), earlier);
        } else {
            assert_eq!(model.service(ModelSource::First, true, true), earlier);
            let retried = model.retry(retry).expect("drained ring headroom");
            assert_eq!(
                model.retry_release(release_token),
                Ok(ReleaseOffer::Queued(retried))
            );
            assert_eq!(model.service(ModelSource::First, true, true), retried);
            assert_eq!(model.service_release(ModelSource::First).id, retried);
        }
        assert!(model.release_pending[ModelSource::First.index()].is_none());
    }
}

#[test]
fn stray_release_waiting_behind_retry_gets_final_unmatched_result() {
    let mut model = Model::new(limits(2), 3);
    model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("earlier onset");
    let retry = match model.submit(ModelSource::First, input(0x90, 63, 100)) {
        Err(SubmitError::Retry(token)) => token,
        _ => panic!("pending onset must wait for ring headroom"),
    };
    let stray = input(0x80, 70, 0);
    let token = match model.offer_release(ModelSource::First, stray) {
        Err(ReleaseOfferError::Blocked(token)) => token,
        _ => panic!("stray release retains source order behind retry"),
    };
    model.service(ModelSource::First, true, true);
    model.retry(retry).expect("drained ring headroom");
    assert_eq!(
        model.retry_release(token),
        Ok(ReleaseOffer::Unmatched(stray))
    );
    assert!(model.release_pending[ModelSource::First.index()].is_none());
    model.service(ModelSource::First, true, true);
    assert!(model.can_offer_ordinary(ModelSource::First));
    model.offer_ordinary(ModelSource::First);
}

#[test]
fn exhausted_release_attempt_id_returns_original_without_queue_custody() {
    let mut model = Model::new(limits(2), 3);
    model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("earlier onset");
    let retry = match model.submit(ModelSource::First, input(0x90, 63, 100)) {
        Err(SubmitError::Retry(token)) => token,
        _ => panic!("pending onset must wait for ring headroom"),
    };
    model.next_release = u64::MAX;
    let original = input(0x80, 63, 0);
    let before = model.used;
    assert_eq!(
        model.offer_release(ModelSource::First, original),
        Err(ReleaseOfferError::IdentityExhausted(original))
    );
    assert_eq!(model.used, before);
    assert!(model.release_pending[ModelSource::First.index()].is_none());
    assert!(model.host_halted);
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::First,
            TerminalFault {
                original,
                reason: TerminalReason::ReleaseIdentity,
            }
        ))
    );
    assert_eq!(
        TerminalReason::ReleaseIdentity.origin(),
        CauseOrigin::SharedState
    );
    assert_eq!(
        model.retry_pending[ModelSource::First.index()]
            .as_ref()
            .map(|pending| pending.attempt.id),
        Some(retry.id)
    );
    assert!(matches!(model.retry(retry), Err(RetryError::Halted(_))));
}

#[test]
fn exhausted_occurrence_id_takes_no_shared_credit() {
    let mut model = Model::new(limits(2), 3);
    model.next = u64::MAX;
    let refused = input(0x90, 60, 100);
    assert!(matches!(
        model.submit(ModelSource::First, refused),
        Err(SubmitError::NoCredit(original, NoCreditReason::OccurrenceIdentity))
            if original == refused
    ));
    assert_eq!(model.used, limits(0));
    assert!(model.rings[ModelSource::First.index()].is_empty());
    assert!(model.retry_pending[ModelSource::First.index()].is_none());
    assert!(model.host_halted);
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::First,
            TerminalFault {
                original: refused,
                reason: TerminalReason::NoCredit(NoCreditReason::OccurrenceIdentity),
            }
        ))
    );
    assert_eq!(
        TerminalReason::NoCredit(NoCreditReason::OccurrenceIdentity).origin(),
        CauseOrigin::SharedState
    );
}

#[test]
fn shared_credit_refusal_halts_host_and_preserves_other_source_prefix() {
    let mut model = Model::new(limits(1), 2);
    let held = model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("first source owns shared credit");
    assert_eq!(
        model.onset_preflight(ModelSource::Second),
        OnsetPreflight::NoCredit(NoCreditReason::Tracker)
    );
    let refused = input(0x91, 60, 100);
    assert!(matches!(
        model.submit(ModelSource::Second, refused),
        Err(SubmitError::NoCredit(original, NoCreditReason::Tracker))
            if original == refused
    ));
    assert_eq!(model.used, limits(1));
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::Second,
            TerminalFault {
                original: refused,
                reason: TerminalReason::NoCredit(NoCreditReason::Tracker),
            }
        ))
    );
    assert_eq!(
        TerminalReason::NoCredit(NoCreditReason::Tracker).origin(),
        CauseOrigin::SharedState
    );
    assert!(matches!(
        model.rings[ModelSource::First.index()].front(),
        Some(RingPacket::Onset(attempt)) if attempt.id == held
    ));
    let later = input(0x90, 60, 100);
    assert!(matches!(
        model.submit(ModelSource::First, later),
        Err(SubmitError::Halted(original)) if original == later
    ));
    assert_eq!(
        model.offer_release(ModelSource::First, input(0x80, 60, 0)),
        Err(ReleaseOfferError::Halted(input(0x80, 60, 0)))
    );
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::Second,
            TerminalFault {
                original: refused,
                reason: TerminalReason::NoCredit(NoCreditReason::Tracker),
            }
        ))
    );
    assert_eq!(model.used, limits(1));
}

#[test]
fn peer_terminal_fault_keeps_both_pending_tokens_and_shared_charge() {
    let mut model = Model::new(limits(2), 3);
    model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("earlier first-source onset");
    let retry = match model.submit(ModelSource::First, input(0x90, 63, 100)) {
        Err(SubmitError::Retry(token)) => token,
        _ => panic!("later first-source onset must retain retry"),
    };
    let release = input(0x80, 60, 0);
    let release_token = match model.offer_release(ModelSource::First, release) {
        Err(ReleaseOfferError::Blocked(token)) => token,
        _ => panic!("release must remain behind onset retry"),
    };
    let refused = input(0x91, 70, 100);
    assert!(matches!(
        model.submit(ModelSource::Second, refused),
        Err(SubmitError::NoCredit(original, NoCreditReason::Tracker))
            if original == refused
    ));
    let charged = model.used;
    let retry = match model.retry(retry) {
        Err(RetryError::Halted(token)) => token,
        _ => panic!("peer halt must stop matching onset retry"),
    };
    assert!(matches!(model.retire(retry), Err(RetryError::Halted(_))));
    assert!(matches!(
        model.retry_release(release_token),
        Err(ReleaseRetryError::Halted(_))
    ));
    assert_eq!(model.used, charged);
    assert!(model.retry_pending[ModelSource::First.index()].is_some());
    assert_eq!(
        model.release_pending[ModelSource::First.index()]
            .as_ref()
            .map(|pending| pending.input),
        Some(release)
    );
    assert!(!model.can_offer_ordinary(ModelSource::First));
    assert!(!model.can_offer_ordinary(ModelSource::Second));
    assert_eq!(
        model.terminal_fault,
        Some((
            ModelSource::Second,
            TerminalFault {
                original: refused,
                reason: TerminalReason::NoCredit(NoCreditReason::Tracker),
            }
        ))
    );
}

#[test]
fn blocked_same_key_release_routes_earlier_ring_onset_before_retired_retry() {
    let mut model = Model::new(limits(2), 3);
    let earlier = model
        .submit(ModelSource::First, input(0x90, 60, 100))
        .expect("earlier same-key onset");
    let retry = match model.submit(ModelSource::First, input(0x90, 60, 100)) {
        Err(SubmitError::Retry(token)) => token,
        _ => panic!("second same-key onset must wait"),
    };
    let release_token = match model.offer_release(ModelSource::First, input(0x80, 60, 0)) {
        Err(ReleaseOfferError::Blocked(token)) => token,
        _ => panic!("same-key release must wait behind retry"),
    };
    let retired = model
        .retire(retry)
        .expect("retired retry retains tombstone");
    assert_eq!(
        model.retry_release(release_token),
        Ok(ReleaseOffer::Queued(earlier))
    );
    assert_eq!(model.service(ModelSource::First, false, false), earlier);
    model.collect_result(earlier).unwrap();
    assert_eq!(model.service_release(ModelSource::First).id, earlier);
    assert_eq!(model.used.ledger, 1);
    assert_eq!(
        model.offer_release(ModelSource::First, input(0x80, 60, 0)),
        Ok(ReleaseOffer::Retired(retired.id))
    );
    assert_eq!(model.used, limits(0));
}

#[test]
fn retired_retry_tombstone_does_not_release_later_same_key_onset() {
    let mut model = Model::new(limits(3), 3);
    let earlier = model
        .submit(ModelSource::First, input(0x90, 65, 100))
        .expect("earlier onset");
    let retry = match model.submit(ModelSource::First, input(0x90, 66, 100)) {
        Err(SubmitError::Retry(token)) => token,
        _ => panic!("second onset must wait for ring headroom"),
    };
    model.service(ModelSource::First, true, true);
    let retired = model.retire(retry).expect("retire pending onset");
    assert_eq!(model.used.ledger, 2);
    assert_eq!(
        model
            .release(ModelSource::First, input(0x80, 65, 0))
            .map(|route| route.id),
        Some(earlier)
    );
    let later = model
        .submit(ModelSource::First, input(0x90, 66, 100))
        .expect("later same-key onset");
    model.service(ModelSource::First, true, true);
    assert_eq!(model.used.ledger, 3);
    assert_eq!(
        model.offer_release(ModelSource::First, input(0x80, 66, 0)),
        Ok(ReleaseOffer::Retired(retired.id))
    );
    assert_eq!(model.used.ledger, 2);
    assert_eq!(model.release_reservations, [1, 0]);
    assert_eq!(
        model.offer_release(ModelSource::First, input(0x80, 66, 0)),
        Ok(ReleaseOffer::Queued(later))
    );
    assert_eq!(model.service_release(ModelSource::First).id, later);
}

#[test]
fn cross_source_credit_waits_for_consumer_settlement_after_result_collection() {
    let mut model = Model::new(limits(1), 2);
    let held = model
        .submit(ModelSource::First, input(0x90, 67, 100))
        .expect("first queue custody");
    model.service(ModelSource::First, true, true);
    model.collect_result(held).unwrap();
    assert_eq!(
        model.used,
        Counts {
            tracker: 1,
            ingress: 1,
            results: 0,
            ledger: 1,
        }
    );
    assert_eq!(
        model.onset_preflight(ModelSource::Second),
        OnsetPreflight::NoCredit(NoCreditReason::Tracker)
    );
    let route = model
        .release(ModelSource::First, input(0x80, 67, 0))
        .expect("held release");
    assert_eq!(route.id, held);
    assert_eq!(
        model.onset_preflight(ModelSource::Second),
        OnsetPreflight::NoCredit(NoCreditReason::Tracker)
    );
    model.settle_fake_raw_release(held).unwrap();
    assert_eq!(
        model.onset_preflight(ModelSource::Second),
        OnsetPreflight::NoCredit(NoCreditReason::Ingress)
    );
    model.settle_fake_ingress_release(held);
    assert_eq!(
        model.onset_preflight(ModelSource::Second),
        OnsetPreflight::Ready
    );
    assert!(
        model
            .submit(ModelSource::Second, input(0x91, 67, 100))
            .is_ok()
    );
}

#[test]
fn refused_onset_keeps_ledger_cell_after_result_collection() {
    let mut model = Model::new(limits(1), 2);
    let refused = model
        .submit(ModelSource::First, input(0x90, 68, 100))
        .expect("queue custody");
    model.service(ModelSource::First, false, false);
    assert_eq!(
        model.onset_preflight(ModelSource::Second),
        OnsetPreflight::NoCredit(NoCreditReason::Results)
    );
    model.collect_result(refused).unwrap();
    assert_eq!(
        model.used,
        Counts {
            tracker: 0,
            ingress: 0,
            results: 0,
            ledger: 1,
        }
    );
    assert_eq!(
        model.onset_preflight(ModelSource::Second),
        OnsetPreflight::NoCredit(NoCreditReason::Ledger)
    );
    assert_eq!(
        model.release(ModelSource::First, input(0x80, 68, 0)),
        Some(ReleaseRoute {
            id: refused,
            raw: false,
            ingress: false,
        })
    );
    assert_eq!(model.used, limits(0));
    assert_eq!(
        model.onset_preflight(ModelSource::Second),
        OnsetPreflight::Ready
    );
    assert!(
        model
            .submit(ModelSource::Second, input(0x91, 69, 100))
            .is_ok()
    );
}
