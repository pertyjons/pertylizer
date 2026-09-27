//! Reduced two-source custody model with one local raw-input/recorder bridge probe.

use std::collections::VecDeque;

use crate::{
    host::{
        ConnectionGeneration, EndpointId,
        input::{
            InputCapacity, InputCaptureSession, InputLimits, InputOutcome, InputRate, InputTick,
            InputTickSpan, SimulatedInputClock, SimulatedNoteInput,
        },
        session::{
            LoopRecordingSession, SessionCommand, SessionCommandCapacity, SessionLimits,
            SessionSourceCapacity, SessionSourceLimits, SessionSourceOutcome,
        },
    },
    profile::{CaptureLimits, CaptureLimitsInput, RecordingLimits},
    quantities::{
        CapturePassCount, CaptureResultCount, CaptureSourceCount, ChannelLayout, EventCount,
        HeldNoteCount, KeyIdentity, PreparedBytes, ProjectionTickCount, SampleRate,
        TrackedInputNoteCount,
    },
    recording::CaptureOutcome,
    recording::notes::{
        AuditionTrace, CaptureDisposition, CaptureMode, CaptureQuantization, CaptureStamp,
        ControllerSnapshot, FixtureRevision, FixtureTargetId, Midi1Event, Midi1Input,
        MusicalInterval, NoteArmContext, NoteArmInput, SimulatedNoteRecorder,
        loop_capture::{LoopCaptureSession, tests as loop_test_support},
    },
    render::AudioBlockMut,
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
}

enum RingPacket {
    Onset(Attempt),
    Ordinary,
    Release {
        id: OccurrenceId,
        key: Key,
        input: Midi1Input,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoCreditReason {
    SourceReserve,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CauseOrigin {
    SourceLocal,
    SharedState,
}

impl TerminalReason {
    const fn origin(self) -> CauseOrigin {
        match self {
            Self::NoCredit(NoCreditReason::SourceReserve) | Self::Order => CauseOrigin::SourceLocal,
            Self::NoCredit(_) | Self::ReleaseIdentity => CauseOrigin::SharedState,
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
}

struct PendingRelease {
    id: ReleaseAttemptId,
    input: Midi1Input,
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
    released: bool,
    raw_held: bool,
    ingress_held: bool,
    result_held: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Counts {
    tracker: usize,
    ingress: usize,
    results: usize,
    ledger: usize,
}

/// Onset queue custody acquires four shared credits, its ring cell and one
/// future release cell. Consumers return only their own credits after their
/// outcomes; removing a source packet does not redeem its release reserve.
struct Model {
    rings: [VecDeque<RingPacket>; 2],
    release_reservations: [usize; 2],
    source_holds: Vec<SourceHold>,
    retry_pending: [Option<PendingRetry>; 2],
    release_pending: [Option<PendingRelease>; 2],
    host_halted: bool,
    terminal_fault: Option<(ModelSource, TerminalFault)>,
    entries: Vec<Entry>,
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
            release_reservations: [0; 2],
            source_holds: Vec::new(),
            retry_pending: [None, None],
            release_pending: [None, None],
            host_halted: false,
            terminal_fault: None,
            entries: Vec::new(),
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
        let Midi1Event::NoteOn { key, .. } = input.event() else {
            panic!("the reduced submit operation accepts only note onsets");
        };
        let index = source.index();
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
        self.used.tracker += 1;
        self.used.ingress += 1;
        self.used.results += 1;
        self.used.ledger += 1;
        let attempt = Attempt {
            id: OccurrenceId(next),
            key: Key {
                source,
                channel: input.channel(),
                note: key,
            },
            input,
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
        self.source_holds.push(SourceHold {
            id: pending.attempt.id,
            key: pending.attempt.key,
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

    fn service_with_input(
        &mut self,
        source: ModelSource,
        raw: bool,
        ingress: bool,
    ) -> (OccurrenceId, Key, Midi1Input) {
        let RingPacket::Onset(attempt) = self.rings[source.index()]
            .pop_front()
            .expect("queued source occurrence")
        else {
            panic!("onset service cannot skip an earlier source packet");
        };
        if !raw {
            self.used.tracker -= 1;
        }
        if !ingress {
            self.used.ingress -= 1;
        }
        self.entries.push(Entry {
            id: attempt.id,
            key: attempt.key,
            released: false,
            raw_held: raw,
            ingress_held: ingress,
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
    }

    fn offer_ordinary(&mut self, source: ModelSource) {
        assert!(self.can_offer_ordinary(source));
        let index = source.index();
        self.rings[index].push_back(RingPacket::Ordinary);
    }

    fn service_ordinary(&mut self, source: ModelSource) {
        assert!(matches!(
            self.rings[source.index()].pop_front(),
            Some(RingPacket::Ordinary)
        ));
    }

    fn offer_release(
        &mut self,
        source: ModelSource,
        input: Midi1Input,
    ) -> Result<ReleaseOffer, ReleaseOfferError> {
        assert!(matches!(input.event(), Midi1Event::KeyRelease { .. }));
        let index = source.index();
        if self.host_halted {
            return Err(ReleaseOfferError::Halted(input));
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
            });
            return Err(ReleaseOfferError::Blocked(ReleaseRetryToken {
                id: ReleaseAttemptId(next),
                source,
            }));
        }
        self.enqueue_release(source, input)
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
        let input = self.release_pending[index]
            .as_ref()
            .expect("matching pending release")
            .input;
        let outcome = match self.enqueue_release(token.source, input) {
            Ok(outcome) => outcome,
            Err(ReleaseOfferError::Unmatched(original)) => ReleaseOffer::Unmatched(original),
            Err(_) => return Err(ReleaseRetryError::Blocked(token)),
        };
        self.release_pending[index] = None;
        Ok(outcome)
    }

    fn enqueue_release(
        &mut self,
        source: ModelSource,
        input: Midi1Input,
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
        let Some(RingPacket::Release { id, key, input }) = self.rings[source.index()].pop_front()
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
                | ReleaseOfferError::IdentityExhausted(_),
            ) => {
                panic!("direct release helper cannot skip pending source custody")
            }
        }
    }

    fn settle_raw_release(&mut self, id: OccurrenceId) {
        let entry = self.entry_mut(id);
        assert!(entry.released && entry.raw_held);
        entry.raw_held = false;
        self.used.tracker -= 1;
        self.reap(id);
    }

    fn settle_ingress_release(&mut self, id: OccurrenceId) {
        let entry = self.entry_mut(id);
        assert!(entry.released && entry.ingress_held);
        entry.ingress_held = false;
        self.used.ingress -= 1;
        self.reap(id);
    }

    fn collect_result(&mut self, id: OccurrenceId) {
        let entry = self.entry_mut(id);
        assert!(entry.result_held);
        entry.result_held = false;
        self.used.results -= 1;
        self.reap(id);
    }

    fn entry_mut(&mut self, id: OccurrenceId) -> &mut Entry {
        self.entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .expect("source occurrence remains owned")
    }

    fn reap(&mut self, id: OccurrenceId) {
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

#[test]
fn modeled_ring_releases_settle_raw_credit_from_delivered_serial_receipts() {
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
        .offer(SampleTime::new(128), SessionCommand::Stop)
        .unwrap();

    let mut model = Model::new(limits(3), 4);
    let onsets = [
        (ModelSource::First, input(0x90, 60, 100), 10),
        (ModelSource::Second, input(0x90, 60, 110), 11),
        (ModelSource::First, input(0x90, 60, 120), 12),
    ];
    let ids = onsets.map(|(source, note, _)| model.submit(source, note).unwrap());
    let mut admitted = Vec::new();
    for (source, note, time) in onsets {
        let (id, key, queued) = model.service_with_input(source, true, false);
        assert_eq!(queued, note);
        let generation = generations[source.index()];
        let at = SampleTime::new(time);
        let raw_id = owner
            .offer_message(generation, InputTick::new(time), at, queued)
            .unwrap();
        admitted.push((id, key, raw_id));
    }
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
        routed.push((route.id, key, raw_id));
    }
    assert_eq!(
        model.used,
        Counts {
            tracker: 3,
            ingress: 0,
            results: 3,
            ledger: 3
        }
    );
    for generation in generations {
        let _frontier = owner
            .advance_frontier(generation, InputTick::new(128))
            .unwrap();
    }
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
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(model.used.tracker, 3);
    let published = |raw_id| {
        let receipt = receipts
            .iter()
            .find(|receipt| receipt.id == raw_id)
            .unwrap();
        match &receipt.outcome {
            InputOutcome::Delivered(SessionSourceOutcome::Published(record)) => {
                (receipt.matched_onset, record.occurrence.unwrap())
            }
            _ => panic!("raw message must be delivered to the recorder"),
        }
    };
    let mut recorded = Vec::new();
    for (id, key, release_raw) in routed {
        let (_, original_key, onset_raw) = admitted
            .iter()
            .find(|(candidate, _, _)| *candidate == id)
            .unwrap();
        assert_eq!(*original_key, key);
        let (matched, recorded_release) = published(release_raw);
        let (_, recorded_onset) = published(*onset_raw);
        assert_eq!(matched, Some(*onset_raw));
        assert_eq!(recorded_release, recorded_onset);
        recorded.push(recorded_onset);
        model.settle_raw_release(id);
    }
    assert_ne!(recorded[0], recorded[1]);
    assert_ne!(recorded[0], recorded[2]);
    assert_ne!(recorded[1], recorded[2]);
    // This fixture settles tracker credit after checking each delivered raw
    // release and recorder occurrence. The model already returned ingress
    // credit for its refused ingress disposition at source service; no real
    // mixed ingress or combined result is involved.
    assert_eq!(
        model.used,
        Counts {
            tracker: 0,
            ingress: 0,
            results: 3,
            ledger: 3
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
    model.settle_raw_release(raw_only);
    model.settle_ingress_release(ingress_only);
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
    model.collect_result(earlier);
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
    model.collect_result(held);
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
    model.settle_raw_release(held);
    assert_eq!(
        model.onset_preflight(ModelSource::Second),
        OnsetPreflight::NoCredit(NoCreditReason::Ingress)
    );
    model.settle_ingress_release(held);
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
    model.collect_result(refused);
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
