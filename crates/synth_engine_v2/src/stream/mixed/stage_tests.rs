//! Reduced bounded mixed lane after independent raw source service.

use std::collections::VecDeque;

use synth_core::MidiChannel;

use super::{
    EventPayload, MixedCollection, MixedCollectionEnd, MixedIngressOriginId, MixedIngressOutcome,
    MixedIngressRequest, MixedOneShotAudio, MixedOneShotControl, MixedOneShotRenderError,
    MixedOneShotTeardown, MixedPedalSubmitted, TimedEvent, history_tests, ledger_tests,
    occurrence::{
        LiveOccurrenceId, MixedLiveScope, MixedLiveSource, MixedOnsetSubmitError,
        MixedPedalSubmitError, MixedRetireCause, MixedRetired,
    },
};
use crate::{
    host::{
        ConnectionGeneration, EndpointId,
        input::{
            InputCapacity, InputCaptureError, InputCaptureHalt, InputCaptureSession,
            InputCustodyPreflight, InputError, InputEventId, InputLimits, InputObservation,
            InputOutcome, InputRate, InputReceipt, InputTick, InputTickSpan, SimulatedInputClock,
            SimulatedNoteInput,
        },
        session::SessionSourceOutcome,
    },
    identity::NoteIdentity,
    ingress::{ExhaustedResource, IngressRefused},
    quantities::{ChannelLayout, EventCount, PreparedBytes},
    recording::{
        CaptureOutcome,
        notes::{Midi1Event, Midi1Input, PerformedOccurrenceId},
    },
    render::{AudioBlockMut, NoteEdge},
    time::{FrameCount, SampleTime, issue_epoch},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Source {
    First,
    Second,
}

impl Source {
    const fn index(self) -> usize {
        match self {
            Self::First => 0,
            Self::Second => 1,
        }
    }
}

/// The private raw bridge maps each source and MIDI channel to one live scope.
fn source_scope(source: Source, original: Midi1Input) -> MixedLiveScope {
    let owner = match source {
        Source::First => MixedLiveSource::new(1),
        Source::Second => MixedLiveSource::new(2),
    };
    MixedLiveScope::new(owner, original.channel())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Occurrence(u64);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Packet {
    occurrence: Occurrence,
    original: Midi1Input,
    at: SampleTime,
    release: bool,
}

struct Entry {
    packet: Packet,
    frontier_after: Option<SampleTime>,
}

#[derive(Default)]
struct Lane {
    entries: VecDeque<Entry>,
    reserved_releases: Vec<Occurrence>,
    reserved_pedal_lifts: Vec<(MidiChannel, Occurrence)>,
    unstaged_pedal_downs: Vec<(MidiChannel, Occurrence)>,
    frontier: Option<SampleTime>,
    last_offered: Option<SampleTime>,
}

impl Lane {
    fn reserved_count(&self) -> usize {
        self.reserved_releases.len() + self.reserved_pedal_lifts.len()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StagePedalCharge {
    ReservedLift,
    RedeemedLift(Occurrence),
    Ordinary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StageRefusal {
    Order,
    NoPacketCredit,
    NoReleaseReservation,
    UnstagedPedal(Occurrence),
}

/// Each source owns packet cells independently; a frontier is attached to the
/// preceding packet or kept in the empty lane, so it spends no packet cell.
struct Stage {
    lanes: [Lane; 2],
    cells_per_source: usize,
}

impl Stage {
    fn new(cells_per_source: usize) -> Self {
        assert!(cells_per_source >= 2);
        Self {
            lanes: [Lane::default(), Lane::default()],
            cells_per_source,
        }
    }

    fn onset(&mut self, source: Source, packet: Packet) -> Result<(), StageRefusal> {
        assert!(!packet.release);
        let lane = &mut self.lanes[source.index()];
        if lane.last_offered.is_some_and(|last| packet.at < last) {
            return Err(StageRefusal::Order);
        }
        // The second cell is held until this onset's release enters the lane.
        if lane.entries.len() + lane.reserved_count() + 2 > self.cells_per_source {
            return Err(StageRefusal::NoPacketCredit);
        }
        lane.last_offered = Some(packet.at);
        lane.reserved_releases.push(packet.occurrence);
        lane.entries.push_back(Entry {
            packet,
            frontier_after: None,
        });
        Ok(())
    }

    fn release(&mut self, source: Source, packet: Packet) -> Result<(), StageRefusal> {
        assert!(packet.release);
        let lane = &mut self.lanes[source.index()];
        if lane.last_offered.is_some_and(|last| packet.at < last) {
            return Err(StageRefusal::Order);
        }
        let Some(position) = lane
            .reserved_releases
            .iter()
            .position(|id| *id == packet.occurrence)
        else {
            return Err(StageRefusal::NoReleaseReservation);
        };
        lane.reserved_releases.swap_remove(position);
        lane.last_offered = Some(packet.at);
        lane.entries.push_back(Entry {
            packet,
            frontier_after: None,
        });
        assert!(lane.entries.len() + lane.reserved_count() <= self.cells_per_source);
        Ok(())
    }

    /// An effective source pedal-down holds its own cell and one future lift
    /// cell. A raw lift redeems that stage reservation independently of its
    /// eventual mixed result. Repeated edges use an ordinary cell.
    fn pedal(&mut self, source: Source, packet: Packet) -> Result<StagePedalCharge, StageRefusal> {
        assert!(!packet.release);
        let Midi1Event::Sustain { down } = packet.original.event() else {
            panic!("a pedal stage entry carries a sustain edge");
        };
        let lane = &mut self.lanes[source.index()];
        if lane.last_offered.is_some_and(|last| packet.at < last) {
            return Err(StageRefusal::Order);
        }
        let channel = packet.original.channel();
        if let Some(index) = lane
            .unstaged_pedal_downs
            .iter()
            .position(|(held, _)| *held == channel)
        {
            lane.last_offered = Some(packet.at);
            let down_occurrence = if down {
                lane.unstaged_pedal_downs[index].1
            } else {
                lane.unstaged_pedal_downs.swap_remove(index).1
            };
            return Err(StageRefusal::UnstagedPedal(down_occurrence));
        }
        let reserved = lane
            .reserved_pedal_lifts
            .iter()
            .position(|(held, _)| *held == channel);
        let new_cells = match (down, reserved) {
            (true, None) => 2,
            (false, Some(_)) => 0,
            _ => 1,
        };
        if lane.entries.len() + lane.reserved_count() + new_cells > self.cells_per_source {
            if down && reserved.is_none() {
                lane.unstaged_pedal_downs.push((channel, packet.occurrence));
                lane.last_offered = Some(packet.at);
            }
            return Err(StageRefusal::NoPacketCredit);
        }
        let charge = match (down, reserved) {
            (true, None) => {
                lane.reserved_pedal_lifts.push((channel, packet.occurrence));
                StagePedalCharge::ReservedLift
            }
            (false, Some(index)) => {
                let (_, down_occurrence) = lane.reserved_pedal_lifts.swap_remove(index);
                StagePedalCharge::RedeemedLift(down_occurrence)
            }
            _ => StagePedalCharge::Ordinary,
        };
        lane.last_offered = Some(packet.at);
        lane.entries.push_back(Entry {
            packet,
            frontier_after: None,
        });
        assert!(lane.entries.len() + lane.reserved_count() <= self.cells_per_source);
        Ok(charge)
    }

    /// A bend opens no occurrence, so it needs only its own packet cell.
    fn bend(&mut self, source: Source, packet: Packet) -> Result<(), StageRefusal> {
        assert!(!packet.release && matches!(packet.original.event(), Midi1Event::PitchBend { .. }));
        let lane = &mut self.lanes[source.index()];
        if lane.last_offered.is_some_and(|last| packet.at < last) {
            return Err(StageRefusal::Order);
        }
        if lane.entries.len() + lane.reserved_count() >= self.cells_per_source {
            return Err(StageRefusal::NoPacketCredit);
        }
        lane.last_offered = Some(packet.at);
        lane.entries.push_back(Entry {
            packet,
            frontier_after: None,
        });
        Ok(())
    }

    fn frontier(&mut self, source: Source, at: SampleTime) -> Result<(), StageRefusal> {
        let lane = &mut self.lanes[source.index()];
        if lane.last_offered.is_some_and(|last| at <= last) {
            return Err(StageRefusal::Order);
        }
        lane.last_offered = Some(at);
        if let Some(tail) = lane.entries.back_mut() {
            tail.frontier_after = Some(at);
        } else {
            lane.frontier = Some(at);
        }
        Ok(())
    }

    fn next(&self) -> Option<Source> {
        let first = self.lanes[0].entries.front().map(|entry| entry.packet.at);
        let second = self.lanes[1].entries.front().map(|entry| entry.packet.at);
        match (first, second) {
            (Some(left), Some(right)) => Some(if left <= right {
                Source::First
            } else {
                Source::Second
            }),
            (Some(left), None)
                if self.lanes[1]
                    .frontier
                    .is_some_and(|frontier| frontier > left) =>
            {
                Some(Source::First)
            }
            (None, Some(right))
                if self.lanes[0]
                    .frontier
                    .is_some_and(|frontier| frontier > right) =>
            {
                Some(Source::Second)
            }
            _ => None,
        }
    }

    /// The time-ordered head, which stays staged until its consumer accepts it.
    fn peek_next(&self) -> Option<(Source, Packet)> {
        let source = self.next()?;
        let entry = self.lanes[source.index()].entries.front()?;
        Some((source, entry.packet))
    }

    /// Remove the head [`Self::peek_next`] returned, once its consumer accepted it.
    fn commit_next(&mut self, source: Source, packet: Packet) {
        assert_eq!(
            self.peek_next(),
            Some((source, packet)),
            "only the selected head is committed"
        );
        let lane = &mut self.lanes[source.index()];
        let entry = lane
            .entries
            .pop_front()
            .expect("the selected head is staged");
        if let Some(frontier) = entry.frontier_after {
            lane.frontier = Some(frontier);
        }
    }

    fn pop_next(&mut self) -> Option<(Source, Packet)> {
        let (source, packet) = self.peek_next()?;
        self.commit_next(source, packet);
        Some((source, packet))
    }

    fn pressure(&self, source: Source) -> (usize, usize) {
        let lane = &self.lanes[source.index()];
        (lane.entries.len(), lane.reserved_count())
    }
}

fn input(status: u8, note: u8, velocity: u8) -> Midi1Input {
    Midi1Input::from_bytes([status, note, velocity]).expect("valid MIDI fixture")
}

fn onset(id: u64, at: u64) -> Packet {
    let original = input(0x90, 60, 100);
    assert!(matches!(original.event(), Midi1Event::NoteOn { .. }));
    Packet {
        occurrence: Occurrence(id),
        original,
        at: SampleTime::new(at),
        release: false,
    }
}

fn release(id: u64, at: u64) -> Packet {
    Packet {
        occurrence: Occurrence(id),
        original: input(0x80, 60, 0),
        at: SampleTime::new(at),
        release: true,
    }
}

#[test]
fn saturated_waiting_source_cannot_spend_peer_progress_credit() {
    let mut stage = Stage::new(2);
    stage.onset(Source::First, onset(1, 150)).unwrap();
    assert_eq!(stage.pressure(Source::First), (1, 1));
    assert_eq!(
        stage.onset(Source::First, onset(2, 151)),
        Err(StageRefusal::NoPacketCredit)
    );
    assert_eq!(stage.next(), None);

    stage.onset(Source::Second, onset(3, 140)).unwrap();
    assert_eq!(stage.pop_next(), Some((Source::Second, onset(3, 140))));
    stage.release(Source::Second, release(3, 145)).unwrap();
    assert_eq!(stage.pop_next(), Some((Source::Second, release(3, 145))));
    assert_eq!(stage.next(), None);
    stage
        .frontier(Source::Second, SampleTime::new(150))
        .unwrap();
    assert_eq!(stage.next(), None);
    stage
        .frontier(Source::Second, SampleTime::new(151))
        .unwrap();
    assert_eq!(stage.pop_next(), Some((Source::First, onset(1, 150))));
    assert_eq!(stage.pressure(Source::First), (0, 1));
    stage.release(Source::First, release(1, 160)).unwrap();
    assert_eq!(stage.pressure(Source::First), (1, 0));
    assert_eq!(
        stage.release(Source::First, release(1, 161)),
        Err(StageRefusal::NoReleaseReservation)
    );
}

#[test]
fn consecutive_frontiers_coalesce_behind_a_full_waiting_lane() {
    let mut stage = Stage::new(2);
    stage.onset(Source::First, onset(1, 150)).unwrap();
    stage.release(Source::First, release(1, 160)).unwrap();
    assert_eq!(stage.pressure(Source::First), (2, 0));
    for at in 161..=260 {
        stage.frontier(Source::First, SampleTime::new(at)).unwrap();
        assert_eq!(stage.pressure(Source::First), (2, 0));
    }
    stage
        .frontier(Source::Second, SampleTime::new(151))
        .unwrap();
    assert_eq!(stage.pop_next(), Some((Source::First, onset(1, 150))));
    assert_eq!(stage.next(), None);
    stage
        .frontier(Source::Second, SampleTime::new(161))
        .unwrap();
    assert_eq!(stage.pop_next(), Some((Source::First, release(1, 160))));
    assert_eq!(stage.lanes[0].frontier, Some(SampleTime::new(260)));
    stage.onset(Source::First, onset(2, 260)).unwrap();
    assert_eq!(stage.pressure(Source::First), (1, 1));
    assert_eq!(stage.next(), None);
    stage
        .frontier(Source::Second, SampleTime::new(261))
        .unwrap();
    assert_eq!(stage.pop_next(), Some((Source::First, onset(2, 260))));
}

#[test]
fn frontier_between_packets_does_not_close_its_equal_time() {
    let mut stage = Stage::new(4);
    stage.onset(Source::First, onset(1, 140)).unwrap();
    stage.frontier(Source::First, SampleTime::new(150)).unwrap();
    stage.onset(Source::First, onset(2, 150)).unwrap();
    stage.onset(Source::Second, onset(3, 150)).unwrap();

    assert_eq!(stage.pop_next(), Some((Source::First, onset(1, 140))));
    assert_eq!(stage.lanes[0].frontier, Some(SampleTime::new(150)));
    assert_eq!(stage.pop_next(), Some((Source::First, onset(2, 150))));
    assert_eq!(stage.next(), None);
    stage.frontier(Source::First, SampleTime::new(151)).unwrap();
    assert_eq!(stage.pop_next(), Some((Source::Second, onset(3, 150))));
}

#[test]
fn bounded_stage_feeds_actual_mixed_command_results_in_time_order() {
    for compiled_first in [true, false] {
        let (prepared, candidate) = history_tests::one_shot_with_boundary_on(compiled_first);
        let (mut control, mut audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        let mut stage = Stage::new(2);
        stage.onset(Source::First, onset(1, 150)).unwrap();
        stage.onset(Source::Second, onset(2, 140)).unwrap();
        stage.release(Source::Second, release(2, 145)).unwrap();
        stage
            .frontier(Source::Second, SampleTime::new(151))
            .unwrap();
        assert_eq!(stage.pressure(Source::Second), (2, 0));

        let (source, first) = stage.pop_next().unwrap();
        assert_eq!(source, Source::Second);
        let Midi1Event::NoteOn { key, velocity } = first.original.event() else {
            panic!("first staged packet must be an onset");
        };
        let (first_command, first_request) = control
            .submit_onset(
                MixedIngressOriginId(first.occurrence.0),
                super::TEST_SCOPE,
                first.at,
                key,
                velocity,
            )
            .unwrap();
        let MixedIngressRequest::Onset {
            occurrence: first_handle,
            ..
        } = first_request
        else {
            panic!("onset request");
        };
        audio.service_test_ingress_queue();
        let first_result = control.collect_ingress_result().unwrap();
        assert_eq!(
            (first_result.id, first_result.request),
            (first_command, first_request)
        );
        let MixedIngressOutcome::Onset(Ok(first_identity)) = first_result.outcome else {
            panic!("the earlier staged onset must be accepted");
        };

        let (source, second) = stage.pop_next().unwrap();
        assert_eq!((source, second), (Source::Second, release(2, 145)));
        let (second_command, second_request) = control
            .submit_release(
                MixedIngressOriginId(second.occurrence.0),
                first_handle,
                second.at,
            )
            .unwrap();
        audio.service_test_ingress_queue();
        let second_result = control.collect_ingress_result().unwrap();
        assert_eq!(
            (second_result.id, second_result.request),
            (second_command, second_request)
        );
        assert_eq!(
            second_result.outcome,
            MixedIngressOutcome::Release(Ok(MixedRetired {
                occurrence: first_handle,
                cause: MixedRetireCause::Released(first_identity),
            }))
        );

        let (source, third) = stage.pop_next().unwrap();
        assert_eq!((source, third), (Source::First, onset(1, 150)));
        let Midi1Event::NoteOn { key, velocity } = third.original.event() else {
            panic!("waiting staged packet must be an onset");
        };
        let (third_command, third_request) = control
            .submit_onset(
                MixedIngressOriginId(third.occurrence.0),
                super::TEST_SCOPE,
                third.at,
                key,
                velocity,
            )
            .unwrap();
        let MixedIngressRequest::Onset {
            occurrence: third_handle,
            ..
        } = third_request
        else {
            panic!("onset request");
        };
        audio.service_test_ingress_queue();
        let third_result = control.collect_ingress_result().unwrap();
        assert_eq!(
            (third_result.id, third_result.request),
            (third_command, third_request)
        );
        let MixedIngressOutcome::Onset(Ok(third_identity)) = third_result.outcome else {
            panic!("waiting staged onset must be accepted");
        };
        assert_ne!(first_identity, third_identity);

        stage.release(Source::First, release(1, 160)).unwrap();
        stage
            .frontier(Source::Second, SampleTime::new(161))
            .unwrap();
        let (source, last) = stage.pop_next().unwrap();
        assert_eq!((source, last), (Source::First, release(1, 160)));
        let (last_command, last_request) = control
            .submit_release(
                MixedIngressOriginId(last.occurrence.0),
                third_handle,
                last.at,
            )
            .unwrap();
        audio.service_test_ingress_queue();
        let last_result = control.collect_ingress_result().unwrap();
        assert_eq!(
            (last_result.id, last_result.request),
            (last_command, last_request)
        );
        assert_eq!(
            last_result.outcome,
            MixedIngressOutcome::Release(Ok(MixedRetired {
                occurrence: third_handle,
                cause: MixedRetireCause::Released(third_identity),
            }))
        );
        assert!(control.collect_ingress_result().is_none());
        assert_eq!(stage.pressure(Source::First), (0, 0));
        assert_eq!(stage.pressure(Source::Second), (0, 0));
    }
}

/// One source's raw admission with the release link read before its cell can
/// be reaped.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RawHandoff {
    source: Source,
    raw: InputEventId,
    link: Option<InputEventId>,
    original: Midi1Input,
    at: SampleTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaneRefusal {
    Link(InputError),
    Stage(StageRefusal),
    UnmatchedRelease,
    UnstagedOnset,
    UnstagedPedal { down_raw: InputEventId },
    Ordinary,
}

/// Raw and mixed dispositions stay separate: a raw refusal never reaches the
/// stage, while a lane refusal retains the raw admission it follows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BridgeRefused {
    Raw {
        source: Source,
        original: Midi1Input,
        at: SampleTime,
        error: InputError,
    },
    Lane {
        handoff: RawHandoff,
        reason: LaneRefusal,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MixedDisposition {
    /// The actual audio-side result for this staged packet's request. A release
    /// names its onset's handle, so a refused onset's release is offered too and
    /// retired against the onset's tombstone.
    Result(MixedIngressOutcome),
}

/// A private split decision at a selected stage head. The bridge removes its
/// handoff copy and returns it here; the tombstone keeps only its raw ID.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SplitDisposition {
    Onset {
        reason: MixedOnsetSubmitError,
    },
    KeyUpAfterOnset {
        onset_raw: InputEventId,
        reason: MixedOnsetSubmitError,
    },
}

#[derive(Debug, PartialEq)]
struct SplitRefusal {
    handoff: RawHandoff,
    occurrence: Occurrence,
    disposition: SplitDisposition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RefusedOnsetTombstone {
    source: Source,
    occurrence: Occurrence,
    onset_raw: InputEventId,
    reason: MixedOnsetSubmitError,
}

#[derive(Debug, PartialEq)]
enum OnsetDecision {
    Admitted(CombinedResult),
    Refused(SplitRefusal),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CombinedResult {
    handoff: RawHandoff,
    occurrence: Occurrence,
    /// The merger-minted handle the request named.
    handle: LiveOccurrenceId,
    disposition: MixedDisposition,
}

#[derive(Debug, PartialEq)]
struct PedalCombinedResult {
    handoff: RawHandoff,
    occurrence: Occurrence,
    paired_down_raw: Option<InputEventId>,
    /// `None` means the selected edge was redundant at the mixed owner.
    outcome: Option<MixedIngressOutcome>,
}

impl CombinedResult {
    /// Both consumers accepted: raw admission produced `handoff`, and the
    /// audio-side mixed result for the same staged occurrence is positive.
    fn accepted(&self) -> bool {
        matches!(
            self.disposition,
            MixedDisposition::Result(
                MixedIngressOutcome::Onset(Ok(_))
                    | MixedIngressOutcome::Release(Ok(MixedRetired {
                        cause: MixedRetireCause::Released(_),
                        ..
                    }))
            )
        )
    }
}

struct RawSource {
    input: SimulatedNoteInput,
    generation: ConnectionGeneration,
}

fn raw_source(index: usize) -> RawSource {
    raw_source_with_cells(index, 12)
}

fn raw_source_with_cells(index: usize, cells: u32) -> RawSource {
    let mut input = SimulatedNoteInput::new(
        EndpointId::new(format!("stage-{index}")).unwrap(),
        InputLimits {
            cells: InputCapacity::new(cells).unwrap(),
            bytes: PreparedBytes::measured(65_536),
        },
    )
    .unwrap();
    let generation = input.begin().unwrap();
    input
        .prepare(
            generation,
            SimulatedInputClock::new(
                issue_epoch().unwrap(),
                SampleTime::ZERO,
                InputTick::new(0),
                InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
                InputTickSpan::new(0),
            ),
        )
        .unwrap();
    input.start(generation).unwrap();
    RawSource { input, generation }
}

/// The raw side of the bridge: either standalone raw owners or the raw inputs
/// of one serial capture session feeding the actual recorder.
trait RawBackend {
    fn preflight(
        &self,
        source: Source,
        at: SampleTime,
        original: Midi1Input,
    ) -> Result<InputCustodyPreflight, InputError>;

    fn offer(
        &mut self,
        source: Source,
        at: SampleTime,
        original: Midi1Input,
    ) -> Result<InputEventId, InputError>;
    fn frontier(&mut self, source: Source, at: SampleTime) -> InputEventId;
    fn matched_onset(
        &self,
        source: Source,
        id: InputEventId,
    ) -> Result<Option<InputEventId>, InputError>;
}

impl RawBackend for [RawSource; 2] {
    fn preflight(
        &self,
        source: Source,
        at: SampleTime,
        original: Midi1Input,
    ) -> Result<InputCustodyPreflight, InputError> {
        let owner = &self[source.index()];
        owner.input.preflight_observation_custody(
            owner.generation,
            InputObservation::Message {
                tick: InputTick::new(at.as_u64()),
                arrival: at,
                input: original,
            },
        )
    }

    fn offer(
        &mut self,
        source: Source,
        at: SampleTime,
        original: Midi1Input,
    ) -> Result<InputEventId, InputError> {
        let owner = &mut self[source.index()];
        owner
            .input
            .offer_message(owner.generation, InputTick::new(at.as_u64()), at, original)
    }

    fn frontier(&mut self, source: Source, at: SampleTime) -> InputEventId {
        let owner = &mut self[source.index()];
        owner
            .input
            .advance_frontier(owner.generation, InputTick::new(at.as_u64()))
            .unwrap()
    }

    fn matched_onset(
        &self,
        source: Source,
        id: InputEventId,
    ) -> Result<Option<InputEventId>, InputError> {
        self[source.index()].input.matched_onset(id)
    }
}

struct SerialRaw {
    owner: InputCaptureSession,
    generations: [ConnectionGeneration; 2],
}

/// A recording-session error other than raw admission is outside this fixture.
fn raw_admission(error: InputCaptureError) -> InputError {
    match error {
        InputCaptureError::Input(error) => error,
        other => panic!("serial capture failed outside raw admission: {other}"),
    }
}

impl RawBackend for SerialRaw {
    fn preflight(
        &self,
        source: Source,
        at: SampleTime,
        original: Midi1Input,
    ) -> Result<InputCustodyPreflight, InputError> {
        let generation = self.generations[source.index()];
        self.owner
            .input(generation)
            .ok_or(InputError::Stale)?
            .preflight_observation_custody(
                generation,
                InputObservation::Message {
                    tick: InputTick::new(at.as_u64()),
                    arrival: at,
                    input: original,
                },
            )
    }

    fn offer(
        &mut self,
        source: Source,
        at: SampleTime,
        original: Midi1Input,
    ) -> Result<InputEventId, InputError> {
        self.owner
            .offer_message(
                self.generations[source.index()],
                InputTick::new(at.as_u64()),
                at,
                original,
            )
            .map_err(raw_admission)
    }

    fn frontier(&mut self, source: Source, at: SampleTime) -> InputEventId {
        self.owner
            .advance_frontier(
                self.generations[source.index()],
                InputTick::new(at.as_u64()),
            )
            .unwrap()
    }

    fn matched_onset(
        &self,
        source: Source,
        id: InputEventId,
    ) -> Result<Option<InputEventId>, InputError> {
        self.owner
            .input(self.generations[source.index()])
            .ok_or(InputError::Stale)?
            .matched_onset(id)
    }
}

/// Actual raw owners serve each source independently. Notes and pedals enter
/// the bounded mixed stage; note releases use the raw owner's same-key link.
struct RawStageBridge<R = [RawSource; 2]> {
    stage: Stage,
    sources: R,
    staged_onsets: Vec<(Source, InputEventId, Occurrence)>,
    /// Source pedal-downs whose stage lift place has not yet been redeemed.
    pedal_downs: Vec<(Source, MidiChannel, Occurrence, InputEventId)>,
    /// A staged lift retains its raw down link until selected service or teardown.
    paired_pedal_lifts: Vec<(Occurrence, InputEventId)>,
    /// A raw-admitted down refused at stage still owns its source latch.
    unstaged_pedal_downs: Vec<(Source, MidiChannel, Occurrence, InputEventId)>,
    handoffs: Vec<(Occurrence, bool, RawHandoff)>,
    onset_results: Vec<(Occurrence, Result<NoteIdentity, IngressRefused>)>,
    /// The merger-minted handle of every staged onset whose release is not yet offered.
    handles: Vec<(Occurrence, LiveOccurrenceId)>,
    /// A refused onset has no mixed handle. Its raw handoff belongs to the
    /// returned split result, while this cell links its future key-up.
    refused_onsets: Vec<RefusedOnsetTombstone>,
    /// Every raw ID a raw owner issued to this bridge, with its admission link.
    raw_issued: Vec<(Source, InputEventId, Option<InputEventId>)>,
    next_occurrence: u64,
}

impl RawStageBridge {
    fn new(cells_per_source: usize) -> Self {
        Self::with_raw(cells_per_source, [raw_source(0), raw_source(1)])
    }
}

impl<R: RawBackend> RawStageBridge<R> {
    fn with_raw(cells_per_source: usize, sources: R) -> Self {
        Self {
            stage: Stage::new(cells_per_source),
            sources,
            staged_onsets: Vec::new(),
            pedal_downs: Vec::new(),
            paired_pedal_lifts: Vec::new(),
            unstaged_pedal_downs: Vec::new(),
            handoffs: Vec::new(),
            onset_results: Vec::new(),
            handles: Vec::new(),
            refused_onsets: Vec::new(),
            raw_issued: Vec::new(),
            next_occurrence: 1,
        }
    }

    /// A bent-scope onset remains staged, with the same raw handoff, if the
    /// command caller lacks an inherited-bend credit. This focused path keeps
    /// the ordinary note bridge's result and release bookkeeping intact.
    fn try_service_onset_in_scope(
        &mut self,
        control: &mut MixedOneShotControl,
        audio: &mut MixedOneShotAudio,
        scope: MixedLiveScope,
    ) -> Result<Option<CombinedResult>, MixedOnsetSubmitError> {
        let Some((source, packet)) = self.stage.peek_next() else {
            return Ok(None);
        };
        assert_eq!(scope, source_scope(source, packet.original));
        let Midi1Event::NoteOn { key, velocity } = packet.original.event() else {
            panic!("the focused bridge services only an onset");
        };
        let origin = MixedIngressOriginId(packet.occurrence.0);
        let (command, request) = control.submit_onset(origin, scope, packet.at, key, velocity)?;
        self.stage.commit_next(source, packet);
        let position = self
            .handoffs
            .iter()
            .position(|&(occurrence, release, _)| occurrence == packet.occurrence && !release)
            .unwrap();
        let (_, _, handoff) = self.handoffs.remove(position);
        assert_eq!(
            (handoff.source, handoff.original, handoff.at),
            (source, packet.original, packet.at)
        );
        let MixedIngressRequest::Onset {
            occurrence: handle, ..
        } = request
        else {
            unreachable!("an onset submit returns its onset request");
        };
        self.handles.push((packet.occurrence, handle));
        audio.service_test_ingress_queue();
        let result = control.collect_ingress_result().unwrap();
        assert_eq!((result.id, result.request), (command, request));
        if let MixedIngressOutcome::Onset(onset) = result.outcome {
            self.onset_results.push((packet.occurrence, onset));
        }
        Ok(Some(CombinedResult {
            handoff,
            occurrence: packet.occurrence,
            handle,
            disposition: MixedDisposition::Result(result.outcome),
        }))
    }

    /// At the stage's selected head, a settled command-ring shortage may be
    /// reported as a per-occurrence split refusal. This is a private policy
    /// choice at this service point, not a claim that a later lift cannot free
    /// room. The existing stage selector chooses the service head.
    fn service_onset_or_refuse_full(
        &mut self,
        control: &mut MixedOneShotControl,
        audio: &mut MixedOneShotAudio,
        scope: MixedLiveScope,
    ) -> Result<Option<OnsetDecision>, MixedOnsetSubmitError> {
        match self.try_service_onset_in_scope(control, audio, scope) {
            Ok(Some(result)) => Ok(Some(OnsetDecision::Admitted(result))),
            Ok(None) => Ok(None),
            Err(MixedOnsetSubmitError::Full)
                if control.ingress_outstanding == super::MixedIngressQueueCount::NONE
                    && control.ingress_room() < 2 =>
            {
                let (source, packet) = self
                    .stage
                    .peek_next()
                    .expect("a refused onset remains the selected stage head");
                let position = self
                    .handoffs
                    .iter()
                    .position(|&(occurrence, release, _)| {
                        occurrence == packet.occurrence && !release
                    })
                    .expect("the selected onset keeps its raw handoff");
                self.stage.commit_next(source, packet);
                let (_, _, handoff) = self.handoffs.remove(position);
                self.refused_onsets.push(RefusedOnsetTombstone {
                    source,
                    occurrence: packet.occurrence,
                    onset_raw: handoff.raw,
                    reason: MixedOnsetSubmitError::Full,
                });
                Ok(Some(OnsetDecision::Refused(SplitRefusal {
                    handoff,
                    occurrence: packet.occurrence,
                    disposition: SplitDisposition::Onset {
                        reason: MixedOnsetSubmitError::Full,
                    },
                })))
            }
            Err(error) => Err(error),
        }
    }

    /// Redeem one raw-linked key-up against its refused onset, in stage order.
    /// A key-up for an admitted onset stays on the ordinary mixed path.
    fn service_refused_key_up(&mut self) -> Option<SplitRefusal> {
        let (source, packet) = self.stage.peek_next()?;
        if !packet.release {
            return None;
        }
        let tombstone = self
            .refused_onsets
            .iter()
            .position(|cell| cell.source == source && cell.occurrence == packet.occurrence)?;
        let handoff = self
            .handoffs
            .iter()
            .position(|&(occurrence, release, handoff)| {
                occurrence == packet.occurrence
                    && release
                    && handoff.link == Some(self.refused_onsets[tombstone].onset_raw)
            })
            .expect("a refused onset's staged key-up keeps its raw handoff");
        self.stage.commit_next(source, packet);
        let (_, _, handoff) = self.handoffs.remove(handoff);
        let ended = self.refused_onsets.swap_remove(tombstone);
        Some(SplitRefusal {
            handoff,
            occurrence: packet.occurrence,
            disposition: SplitDisposition::KeyUpAfterOnset {
                onset_raw: ended.onset_raw,
                reason: ended.reason,
            },
        })
    }

    fn offer(
        &mut self,
        source: Source,
        at: u64,
        original: Midi1Input,
    ) -> Result<Occurrence, BridgeRefused> {
        let at = SampleTime::new(at);
        let raw = self
            .sources
            .offer(source, at, original)
            .map_err(|error| BridgeRefused::Raw {
                source,
                original,
                at,
                error,
            })?;
        let mut handoff = RawHandoff {
            source,
            raw,
            link: None,
            original,
            at,
        };
        let link = self.sources.matched_onset(source, raw);
        self.raw_issued
            .push((source, raw, link.as_ref().ok().copied().flatten()));
        handoff.link = link.map_err(|error| BridgeRefused::Lane {
            handoff,
            reason: LaneRefusal::Link(error),
        })?;
        let refuse = |reason| BridgeRefused::Lane { handoff, reason };
        match original.event() {
            Midi1Event::NoteOn { .. } => {
                let occurrence = Occurrence(self.next_occurrence);
                self.stage
                    .onset(
                        source,
                        Packet {
                            occurrence,
                            original,
                            at,
                            release: false,
                        },
                    )
                    .map_err(|refusal| refuse(LaneRefusal::Stage(refusal)))?;
                self.next_occurrence += 1;
                self.staged_onsets.push((source, raw, occurrence));
                self.handoffs.push((occurrence, false, handoff));
                Ok(occurrence)
            }
            Midi1Event::KeyRelease { .. } => {
                let onset = handoff.link.ok_or(refuse(LaneRefusal::UnmatchedRelease))?;
                let position = self
                    .staged_onsets
                    .iter()
                    .position(|&(owner, id, _)| owner == source && id == onset)
                    .ok_or(refuse(LaneRefusal::UnstagedOnset))?;
                let (_, _, occurrence) = self.staged_onsets[position];
                self.stage
                    .release(
                        source,
                        Packet {
                            occurrence,
                            original,
                            at,
                            release: true,
                        },
                    )
                    .map_err(|refusal| refuse(LaneRefusal::Stage(refusal)))?;
                let (_, redeemed, _) = self.staged_onsets.swap_remove(position);
                assert_eq!(redeemed, onset);
                self.handoffs.push((occurrence, true, handoff));
                Ok(occurrence)
            }
            Midi1Event::Sustain { down } => {
                assert_eq!(handoff.link, None);
                let occurrence = Occurrence(self.next_occurrence);
                self.next_occurrence += 1;
                let charge = match self.stage.pedal(
                    source,
                    Packet {
                        occurrence,
                        original,
                        at,
                        release: false,
                    },
                ) {
                    Ok(charge) => charge,
                    Err(StageRefusal::UnstagedPedal(down_occurrence)) => {
                        let position = self
                            .unstaged_pedal_downs
                            .iter()
                            .position(|&(owner, channel, id, _)| {
                                (owner, channel, id)
                                    == (source, original.channel(), down_occurrence)
                            })
                            .expect("an unstaged pedal edge names its raw down");
                        let down_raw = self.unstaged_pedal_downs[position].3;
                        if !down {
                            let (_, _, _, redeemed) =
                                self.unstaged_pedal_downs.swap_remove(position);
                            assert_eq!(redeemed, down_raw);
                        }
                        return Err(refuse(LaneRefusal::UnstagedPedal { down_raw }));
                    }
                    Err(refusal) => {
                        if down
                            && self.stage.lanes[source.index()]
                                .unstaged_pedal_downs
                                .iter()
                                .any(|&(_, id)| id == occurrence)
                        {
                            self.unstaged_pedal_downs.push((
                                source,
                                original.channel(),
                                occurrence,
                                raw,
                            ));
                        }
                        return Err(refuse(LaneRefusal::Stage(refusal)));
                    }
                };
                match charge {
                    StagePedalCharge::ReservedLift => {
                        self.pedal_downs
                            .push((source, original.channel(), occurrence, raw))
                    }
                    StagePedalCharge::RedeemedLift(down) => {
                        let position = self
                            .pedal_downs
                            .iter()
                            .position(|&(owner, channel, id, _)| {
                                (owner, channel, id) == (source, original.channel(), down)
                            })
                            .expect("a protected lift names its raw pedal-down");
                        let (_, _, _, down_raw) = self.pedal_downs.swap_remove(position);
                        assert!(self.raw_issued.contains(&(source, down_raw, None)));
                        self.paired_pedal_lifts.push((occurrence, down_raw));
                    }
                    StagePedalCharge::Ordinary => {}
                }
                self.handoffs.push((occurrence, false, handoff));
                Ok(occurrence)
            }
            _ => Err(refuse(LaneRefusal::Ordinary)),
        }
    }

    fn frontier(&mut self, source: Source, at: u64) {
        let frontier = self.sources.frontier(source, SampleTime::new(at));
        assert_eq!(self.sources.matched_onset(source, frontier), Ok(None));
        self.raw_issued.push((source, frontier, None));
        self.stage.frontier(source, SampleTime::new(at)).unwrap();
    }

    /// A pedal is submitted only from the selected stage head. A caller-side
    /// refusal leaves that head and its raw handoff available for retry.
    fn service_pedal(
        &mut self,
        control: &mut MixedOneShotControl,
        audio: &mut MixedOneShotAudio,
    ) -> Result<Option<PedalCombinedResult>, MixedPedalSubmitError> {
        let Some((source, packet)) = self.stage.peek_next() else {
            return Ok(None);
        };
        let Midi1Event::Sustain { down } = packet.original.event() else {
            panic!("the focused pedal service requires a selected pedal head");
        };
        let submitted = control.submit_pedal(
            MixedIngressOriginId(packet.occurrence.0),
            source_scope(source, packet.original),
            packet.at,
            down,
        )?;
        self.stage.commit_next(source, packet);
        let position = self
            .handoffs
            .iter()
            .position(|&(occurrence, release, _)| occurrence == packet.occurrence && !release)
            .expect("a selected pedal keeps its raw handoff");
        let (_, _, handoff) = self.handoffs.remove(position);
        assert_eq!(
            (handoff.source, handoff.original, handoff.at),
            (source, packet.original, packet.at)
        );
        let paired_down_raw = self
            .paired_pedal_lifts
            .iter()
            .position(|&(occurrence, _)| occurrence == packet.occurrence)
            .map(|position| self.paired_pedal_lifts.swap_remove(position).1);
        let outcome = match submitted {
            MixedPedalSubmitted::Redundant => None,
            MixedPedalSubmitted::Command(command, request) => {
                audio.service_test_ingress_queue();
                let result = control.collect_ingress_result().unwrap();
                assert_eq!((result.id, result.request), (command, request));
                Some(result.outcome)
            }
        };
        Ok(Some(PedalCombinedResult {
            handoff,
            occurrence: packet.occurrence,
            paired_down_raw,
            outcome,
        }))
    }

    /// Offer a selected note packet to mixed ingress. A selected pedal remains
    /// staged for [`Self::service_pedal`]. A note release names the handle its
    /// onset was staged under and never resolves through a key.
    fn service(
        &mut self,
        control: &mut MixedOneShotControl,
        audio: &mut MixedOneShotAudio,
    ) -> Option<CombinedResult> {
        let (source, packet) = self.stage.peek_next()?;
        if !matches!(
            packet.original.event(),
            Midi1Event::NoteOn { .. } | Midi1Event::KeyRelease { .. }
        ) {
            return None;
        }
        self.stage.commit_next(source, packet);
        let position = self
            .handoffs
            .iter()
            .position(|&(occurrence, release, _)| {
                occurrence == packet.occurrence && release == packet.release
            })
            .unwrap();
        let (occurrence, _, handoff) = self.handoffs.remove(position);
        assert_eq!(
            (handoff.source, handoff.original, handoff.at),
            (source, packet.original, packet.at)
        );
        let origin = MixedIngressOriginId(occurrence.0);
        let (handle, (command, request)) = match packet.original.event() {
            Midi1Event::NoteOn { key, velocity } => {
                let submitted = control
                    .submit_onset(
                        origin,
                        source_scope(source, packet.original),
                        packet.at,
                        key,
                        velocity,
                    )
                    .unwrap();
                let MixedIngressRequest::Onset {
                    occurrence: handle, ..
                } = submitted.1
                else {
                    unreachable!("an onset submit returns its onset request");
                };
                self.handles.push((occurrence, handle));
                (handle, submitted)
            }
            Midi1Event::KeyRelease { .. } => {
                let position = self
                    .handles
                    .iter()
                    .position(|&(id, _)| id == occurrence)
                    .expect("a lane selects each onset before its release");
                let (_, handle) = self.handles.swap_remove(position);
                // The synchronous lane collected the onset's result before this
                // release; the release does not read it.
                let position = self
                    .onset_results
                    .iter()
                    .position(|&(id, _)| id == occurrence)
                    .expect("the onset's result was collected before its release");
                let (_, _onset) = self.onset_results.swap_remove(position);
                (
                    handle,
                    control.submit_release(origin, handle, packet.at).unwrap(),
                )
            }
            _ => unreachable!("the selected packet is a note edge"),
        };
        audio.service_test_ingress_queue();
        let result = control.collect_ingress_result().unwrap();
        assert_eq!((result.id, result.request), (command, request));
        if let MixedIngressOutcome::Onset(onset) = result.outcome {
            self.onset_results.push((occurrence, onset));
        }
        Some(CombinedResult {
            handoff,
            occurrence,
            handle,
            disposition: MixedDisposition::Result(result.outcome),
        })
    }
}

/// Render one 256-frame callback, so every queued live edge stamped before it drains.
fn drain(audio: &mut MixedOneShotAudio) {
    let mut samples = [0.0_f32; 256];
    audio
        .render_private(AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).unwrap())
        .unwrap();
}

#[test]
fn raw_release_links_carry_source_handoffs_into_staged_mixed_results() {
    let on = input(0x90, 60, 100);
    let off = input(0x80, 60, 0);
    for compiled_first in [true, false] {
        let (prepared, candidate) = history_tests::one_shot_with_boundary_on(compiled_first);
        let (mut control, mut audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        let mut bridge = RawStageBridge::new(4);

        // Source A repeats key 60; raw pairing, not the stage, decides which
        // occurrence each release redeems.
        let a_first = bridge.offer(Source::First, 150, on).unwrap();
        let a_second = bridge.offer(Source::First, 152, on).unwrap();
        let b_onset = bridge.offer(Source::Second, 140, on).unwrap();
        let b_release = bridge.offer(Source::Second, 145, off).unwrap();
        assert_eq!(b_release, b_onset);
        assert_eq!(bridge.stage.pressure(Source::First), (2, 2));
        assert_eq!(bridge.stage.pressure(Source::Second), (2, 0));

        let first = bridge.service(&mut control, &mut audio).unwrap();
        assert_eq!(
            (first.handoff.source, first.handoff.at, first.handoff.link),
            (Source::Second, SampleTime::new(140), None)
        );
        assert!(first.accepted());
        let second = bridge.service(&mut control, &mut audio).unwrap();
        assert_eq!(second.occurrence, b_onset);
        assert_eq!(second.handoff.link, Some(first.handoff.raw));
        assert!(second.accepted());
        // B's pair drains: the bend-credit budget leaves `2H` ordinary units, which
        // A's two held notes need.
        drain(&mut audio);
        // B has no frontier beyond A's head yet.
        assert!(bridge.service(&mut control, &mut audio).is_none());

        bridge.frontier(Source::Second, 153);
        let third = bridge.service(&mut control, &mut audio).unwrap();
        let fourth = bridge.service(&mut control, &mut audio).unwrap();
        assert_eq!((third.occurrence, fourth.occurrence), (a_first, a_second));
        assert!(third.accepted() && fourth.accepted());
        assert_ne!(third.disposition, fourth.disposition);

        let a_release_first = bridge.offer(Source::First, 160, off).unwrap();
        let a_release_second = bridge.offer(Source::First, 161, off).unwrap();
        assert_eq!((a_release_first, a_release_second), (a_first, a_second));
        bridge.frontier(Source::Second, 162);
        let fifth = bridge.service(&mut control, &mut audio).unwrap();
        let sixth = bridge.service(&mut control, &mut audio).unwrap();
        assert_eq!(fifth.handoff.link, Some(third.handoff.raw));
        assert_eq!(sixth.handoff.link, Some(fourth.handoff.raw));
        assert!(fifth.accepted() && sixth.accepted());
        assert!(bridge.service(&mut control, &mut audio).is_none());
        assert!(control.collect_ingress_result().is_none());
        assert_eq!(bridge.stage.pressure(Source::First), (0, 0));
        assert_eq!(bridge.stage.pressure(Source::Second), (0, 0));
        assert!(bridge.handoffs.is_empty() && bridge.onset_results.is_empty());
    }
}

#[test]
fn lane_refusals_after_raw_admission_keep_the_raw_handoff_and_its_link() {
    let on = input(0x90, 60, 100);
    let off = input(0x80, 60, 0);
    let mut bridge = RawStageBridge::new(2);

    let staged = bridge.offer(Source::First, 150, on).unwrap();
    let Err(BridgeRefused::Lane {
        handoff: full,
        reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
    }) = bridge.offer(Source::First, 151, on)
    else {
        panic!("a saturated lane must refuse after raw admission");
    };
    assert_eq!((full.link, full.at), (None, SampleTime::new(151)));
    assert_eq!(bridge.stage.pressure(Source::First), (1, 1));

    // Raw FIFO pairs the first release with the staged onset and the second
    // with the onset the stage refused.
    assert_eq!(bridge.offer(Source::First, 160, off), Ok(staged));
    let Err(BridgeRefused::Lane {
        handoff: orphan,
        reason: LaneRefusal::UnstagedOnset,
    }) = bridge.offer(Source::First, 161, off)
    else {
        panic!("a release of a lane-refused onset must be refused exactly");
    };
    assert_eq!(orphan.link, Some(full.raw));
    assert_eq!(orphan.original, off);

    let Err(BridgeRefused::Lane {
        handoff: unmatched,
        reason: LaneRefusal::UnmatchedRelease,
    }) = bridge.offer(Source::First, 162, input(0x80, 61, 0))
    else {
        panic!("an unmatched raw release must not reach the stage");
    };
    assert_eq!(unmatched.link, None);
    let pedal = input(0xB0, 64, 127);
    let Err(BridgeRefused::Lane {
        handoff: split_pedal,
        reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
    }) = bridge.offer(Source::First, 163, pedal)
    else {
        panic!("a full lane keeps the raw pedal-down in its split latch");
    };
    assert_eq!((split_pedal.original, split_pedal.link), (pedal, None));
    assert_eq!(bridge.stage.pressure(Source::First), (2, 0));
    assert_eq!(bridge.unstaged_pedal_downs[0].3, split_pedal.raw);
    let wheel = input(0xe0, 0, 64);
    let Err(BridgeRefused::Lane {
        handoff: ordinary,
        reason: LaneRefusal::Ordinary,
    }) = bridge.offer(Source::First, 164, wheel)
    else {
        panic!("an ordinary bend has no note-bridge stage disposition");
    };
    assert_eq!((ordinary.original, ordinary.link), (wheel, None));

    // A raw refusal owns its original and never touches the stage.
    assert_eq!(
        bridge.offer(Source::First, 149, on),
        Err(BridgeRefused::Raw {
            source: Source::First,
            original: on,
            at: SampleTime::new(149),
            error: InputError::Order,
        })
    );
    assert_eq!(bridge.stage.pressure(Source::First), (2, 0));
    assert_eq!(bridge.stage.pressure(Source::Second), (0, 0));
}

#[test]
fn mixed_onset_refusal_retires_its_raw_linked_release_against_the_tombstone() {
    let on = input(0x90, 60, 100);
    let off = input(0x80, 60, 0);
    let hold = IngressRefused::Dropped {
        resource: ExhaustedResource::Hold,
    };
    for compiled_first in [true, false] {
        let (prepared, candidate) = history_tests::one_shot_with_boundary_on(compiled_first);
        let (mut control, mut audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        let mut bridge = RawStageBridge::new(4);

        // Two release holds: A's second key-60 onset is the third live note.
        let a_kept = bridge.offer(Source::First, 140, on).unwrap();
        let b_onset = bridge.offer(Source::Second, 141, on).unwrap();
        let a_dropped = bridge.offer(Source::First, 142, on).unwrap();
        bridge.frontier(Source::Second, 143);
        let kept = bridge.service(&mut control, &mut audio).unwrap();
        let peer = bridge.service(&mut control, &mut audio).unwrap();
        let dropped = bridge.service(&mut control, &mut audio).unwrap();
        assert_eq!(
            (kept.occurrence, peer.occurrence, dropped.occurrence),
            (a_kept, b_onset, a_dropped)
        );
        assert!(kept.accepted() && peer.accepted());
        assert_eq!(
            dropped.disposition,
            MixedDisposition::Result(MixedIngressOutcome::Onset(Err(hold)))
        );
        assert_eq!(audio.test_ingress.counters().dropped_hold(), 1);

        // Raw FIFO links the second release to the mixed-refused onset.
        assert_eq!(bridge.offer(Source::First, 150, off), Ok(a_kept));
        assert_eq!(bridge.offer(Source::First, 151, off), Ok(a_dropped));
        assert_eq!(bridge.offer(Source::Second, 152, off), Ok(b_onset));
        assert_eq!(bridge.stage.pressure(Source::First), (2, 0));
        let released = bridge.service(&mut control, &mut audio).unwrap();
        assert_eq!(released.handoff.link, Some(kept.handoff.raw));
        assert!(released.accepted());

        let disposed = bridge.service(&mut control, &mut audio).unwrap();
        assert_eq!(
            (
                disposed.occurrence,
                disposed.handoff.link,
                disposed.handoff.at
            ),
            (a_dropped, Some(dropped.handoff.raw), SampleTime::new(151))
        );
        // The release names the refused onset's handle; audio consumes it against
        // the tombstone and retires the cell without touching the ingress queue.
        assert_eq!(disposed.handle, dropped.handle);
        assert_eq!(
            disposed.disposition,
            MixedDisposition::Result(MixedIngressOutcome::Release(Ok(MixedRetired {
                occurrence: dropped.handle,
                cause: MixedRetireCause::AfterRefusal(hold),
            })))
        );
        assert!(!disposed.accepted());
        assert_eq!(
            audio.occurrences.released_after_refusal(),
            crate::quantities::EventCount::measured(1)
        );
        assert_eq!(
            audio.test_ingress.holds_outstanding(),
            crate::quantities::EventCount::measured(1)
        );
        assert!(control.collect_ingress_result().is_none());
        assert_eq!(bridge.stage.pressure(Source::First), (0, 0));

        // B's release still waits for A's frontier beyond its stamp.
        assert!(bridge.service(&mut control, &mut audio).is_none());
        bridge.frontier(Source::First, 153);
        let peer_released = bridge.service(&mut control, &mut audio).unwrap();
        assert_eq!(peer_released.handoff.link, Some(peer.handoff.raw));
        assert!(peer_released.accepted());

        // The actual releases returned both holds; once their edges drain from the
        // `2H` ordinary units, a later onset is accepted.
        drain(&mut audio);
        let later = bridge.offer(Source::First, 160, on).unwrap();
        bridge.frontier(Source::Second, 161);
        let reused = bridge.service(&mut control, &mut audio).unwrap();
        assert_eq!(reused.occurrence, later);
        assert!(reused.accepted());
        assert_eq!(audio.test_ingress.counters().dropped_hold(), 1);
        assert!(bridge.handoffs.is_empty());
        assert_eq!(bridge.onset_results.len(), 1);
        assert_eq!(bridge.stage.pressure(Source::First), (0, 1));
        assert_eq!(bridge.stage.pressure(Source::Second), (0, 0));
    }
}

/// Where an accepted mixed onset's identity is found in the stopped owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OnsetLocation {
    Queued,
    ChargedInFaultedCallback,
    Sounding,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum OnsetAtTeardown {
    Live {
        identity: NoteIdentity,
        location: OnsetLocation,
    },
    Refused(IngressRefused),
}

/// Stage packet cells, protected release/lift places and retained onset
/// results are distinct charges. An unstaged pedal latch owns only a raw link,
/// not a stage place. Refused-onset and staged-lift records cross-reference
/// existing charges; they are not additional credits.
#[derive(Debug, PartialEq)]
struct BridgeTeardown {
    raw_issued: Vec<(Source, InputEventId, Option<InputEventId>)>,
    unoffered: Vec<RawHandoff>,
    reservations: Vec<(Source, Occurrence, InputEventId)>,
    pedal_lifts: Vec<(Source, MidiChannel, Occurrence, InputEventId)>,
    /// Cross-references an unoffered lift's raw ID to its raw down ID.
    staged_pedal_lifts: Vec<(InputEventId, InputEventId)>,
    unstaged_pedals: Vec<(Source, MidiChannel, Occurrence, InputEventId)>,
    onsets: Vec<(Occurrence, OnsetAtTeardown)>,
    refused_onsets: Vec<(RefusedOnsetTombstone, RefusedOnsetPending)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RefusedOnsetPending {
    ReservedKeyUp,
    StagedKeyUp(InputEventId),
}

fn onset_edges(events: &[(TimedEvent, bool)], identity: NoteIdentity) -> usize {
    events
        .iter()
        .filter(|(event, _)| {
            matches!(
                event.payload(),
                EventPayload::Note { identity: id, edge: NoteEdge::On { .. } } if id == identity
            )
        })
        .count()
}

fn locate_onset(ended: &MixedOneShotTeardown, identity: NoteIdentity) -> OnsetLocation {
    let found = [
        (
            onset_edges(&ended.ingress.queued, identity),
            OnsetLocation::Queued,
        ),
        (
            onset_edges(&ended.ingress.charged_in_faulted_callback, identity),
            OnsetLocation::ChargedInFaultedCallback,
        ),
        (
            ended
                .sounding
                .live()
                .iter()
                .filter(|note| note.identity == identity)
                .count(),
            OnsetLocation::Sounding,
        ),
    ];
    // Counting, not presence: a duplicate within one location also fails.
    assert_eq!(
        found.iter().map(|(count, _)| count).sum::<usize>(),
        1,
        "exactly one owner copy per accepted onset"
    );
    found
        .iter()
        .find(|(count, _)| *count == 1)
        .map(|(_, location)| *location)
        .unwrap()
}

impl<R: RawBackend> RawStageBridge<R> {
    /// Stop the mixed owner and account for every bridge charge exactly once.
    /// The raw owners are returned with their cells; no raw receipt is settled.
    fn tear_down(
        mut self,
        control: MixedOneShotControl,
        audio: MixedOneShotAudio,
    ) -> (BridgeTeardown, R, MixedOneShotTeardown) {
        let MixedCollection::Ended(ended) = control.collect(audio).unwrap() else {
            panic!("a pending or faulted mixed owner must end at teardown");
        };
        let mut teardown = BridgeTeardown {
            raw_issued: std::mem::take(&mut self.raw_issued),
            unoffered: Vec::new(),
            reservations: Vec::new(),
            pedal_lifts: Vec::new(),
            staged_pedal_lifts: Vec::new(),
            unstaged_pedals: Vec::new(),
            onsets: Vec::new(),
            refused_onsets: Vec::new(),
        };
        for (lane, source) in self
            .stage
            .lanes
            .iter_mut()
            .zip([Source::First, Source::Second])
        {
            for entry in lane.entries.drain(..) {
                let position = self
                    .handoffs
                    .iter()
                    .position(|&(occurrence, release, _)| {
                        occurrence == entry.packet.occurrence && release == entry.packet.release
                    })
                    .expect("every staged packet has its raw handoff");
                let (_, _, handoff) = self.handoffs.remove(position);
                assert_eq!(handoff.source, source);
                if matches!(
                    entry.packet.original.event(),
                    Midi1Event::Sustain { down: false }
                ) && let Some(position) = self
                    .paired_pedal_lifts
                    .iter()
                    .position(|&(lift, _)| lift == entry.packet.occurrence)
                {
                    let (_, down_raw) = self.paired_pedal_lifts.swap_remove(position);
                    teardown.staged_pedal_lifts.push((handoff.raw, down_raw));
                }
                teardown.unoffered.push(handoff);
            }
            for occurrence in lane.reserved_releases.drain(..) {
                let position = self
                    .staged_onsets
                    .iter()
                    .position(|&(owner, _, id)| owner == source && id == occurrence)
                    .expect("every release reservation names a raw onset");
                let (_, raw, _) = self.staged_onsets.remove(position);
                teardown.reservations.push((source, occurrence, raw));
            }
            for (channel, occurrence) in lane.reserved_pedal_lifts.drain(..) {
                let position = self
                    .pedal_downs
                    .iter()
                    .position(|&(owner, held, id, _)| {
                        (owner, held, id) == (source, channel, occurrence)
                    })
                    .expect("every protected lift names a raw pedal-down");
                teardown
                    .pedal_lifts
                    .push(self.pedal_downs.swap_remove(position));
            }
            for (channel, occurrence) in lane.unstaged_pedal_downs.drain(..) {
                let position = self
                    .unstaged_pedal_downs
                    .iter()
                    .position(|&(owner, held, id, _)| {
                        (owner, held, id) == (source, channel, occurrence)
                    })
                    .expect("every unstaged pedal latch names a raw down");
                teardown
                    .unstaged_pedals
                    .push(self.unstaged_pedal_downs.swap_remove(position));
            }
        }
        for tombstone in self.refused_onsets.drain(..) {
            let reserved = teardown
                .reservations
                .iter()
                .any(|&(source, occurrence, raw)| {
                    (source, occurrence, raw)
                        == (tombstone.source, tombstone.occurrence, tombstone.onset_raw)
                });
            let staged = teardown.unoffered.iter().find(|handoff| {
                handoff.source == tombstone.source
                    && handoff.link == Some(tombstone.onset_raw)
                    && matches!(handoff.original.event(), Midi1Event::KeyRelease { .. })
            });
            let pending = match (reserved, staged) {
                (true, None) => RefusedOnsetPending::ReservedKeyUp,
                (false, Some(handoff)) => RefusedOnsetPending::StagedKeyUp(handoff.raw),
                _ => panic!("one stage charge for each refused onset"),
            };
            teardown.refused_onsets.push((tombstone, pending));
        }
        assert!(
            self.handoffs.is_empty()
                && self.staged_onsets.is_empty()
                && self.pedal_downs.is_empty()
                && self.paired_pedal_lifts.is_empty()
                && self.unstaged_pedal_downs.is_empty()
        );
        for (occurrence, result) in self.onset_results.drain(..) {
            let state = match result {
                Ok(identity) => OnsetAtTeardown::Live {
                    identity,
                    location: locate_onset(&ended, identity),
                },
                Err(refusal) => OnsetAtTeardown::Refused(refusal),
            };
            teardown.onsets.push((occurrence, state));
        }
        (teardown, self.sources, *ended)
    }
}

/// Raw receipts after ending both connections: those joined to a raw ID the
/// bridge was issued, and each owner's tick-0 frontier, which `prepare`
/// admits before the bridge sees any traffic.
struct RawSettlement {
    bridge: Vec<(Source, InputReceipt)>,
    prepared_frontiers: Vec<(Source, InputReceipt)>,
}

/// End each returned raw connection and join every raw ID the bridge was
/// issued to exactly one cancelled receipt carrying the same admission link.
fn settle_raw_by_cancellation(
    raw: [RawSource; 2],
    issued: &[(Source, InputEventId, Option<InputEventId>)],
) -> RawSettlement {
    let mut remaining = issued.to_vec();
    let mut settlement = RawSettlement {
        bridge: Vec::new(),
        prepared_frontiers: Vec::new(),
    };
    for (mut owner, source) in raw.into_iter().zip([Source::First, Source::Second]) {
        owner.input.device_lost(owner.generation).unwrap();
        while let Some(receipt) = owner.input.collect() {
            assert!(matches!(receipt.outcome, InputOutcome::Cancelled));
            if let Some(position) = remaining
                .iter()
                .position(|&(owner, id, _)| owner == source && id == receipt.id)
            {
                let (_, _, link) = remaining.swap_remove(position);
                assert_eq!(receipt.matched_onset, link);
                settlement.bridge.push((source, receipt));
                continue;
            }
            assert_eq!(
                (receipt.observation, receipt.matched_onset),
                (
                    InputObservation::Frontier {
                        tick: InputTick::new(0)
                    },
                    None
                ),
                "only the owner's prepared frontier may settle outside the bridge"
            );
            assert!(
                settlement
                    .prepared_frontiers
                    .iter()
                    .all(|(settled, _)| *settled != source),
                "one prepared frontier per raw owner"
            );
            settlement.prepared_frontiers.push((source, receipt));
        }
        assert_eq!(owner.input.pressure().occupied().as_usize(), 0);
    }
    assert!(remaining.is_empty(), "every issued raw ID must be settled");
    settlement
}

#[test]
fn joined_teardown_accounts_every_outstanding_bridge_credit() {
    let on = input(0x90, 60, 100);
    let off = input(0x80, 60, 0);
    let hold = IngressRefused::Dropped {
        resource: ExhaustedResource::Hold,
    };
    for compiled_first in [true, false] {
        for fault in [false, true] {
            let (prepared, candidate) = history_tests::one_shot_with_boundary_on(compiled_first);
            let (mut control, mut audio) = prepared
                .arm_one_shot(candidate, &history_tests::mixed_profile())
                .unwrap();
            let mut bridge = RawStageBridge::new(4);

            let a_live = bridge.offer(Source::First, 140, on).unwrap();
            let b_live = bridge.offer(Source::Second, 141, on).unwrap();
            let a_refused = bridge.offer(Source::First, 142, on).unwrap();
            bridge.frontier(Source::Second, 143);
            let serviced: Vec<_> =
                std::iter::from_fn(|| bridge.service(&mut control, &mut audio)).collect();
            assert_eq!(serviced.len(), 3);
            let raw_of = |occurrence| {
                serviced
                    .iter()
                    .find(|result| result.occurrence == occurrence)
                    .unwrap()
                    .handoff
                    .raw
            };

            // Credits outstanding when the owner stops: two raw-linked releases
            // and one onset staged but not offered, two release reservations,
            // and three retained onset results.
            assert_eq!(bridge.offer(Source::First, 150, off), Ok(a_live));
            assert_eq!(bridge.offer(Source::First, 151, off), Ok(a_refused));
            let b_unoffered = bridge.offer(Source::Second, 155, on).unwrap();
            let Err(BridgeRefused::Lane {
                handoff: unmatched,
                reason: LaneRefusal::UnmatchedRelease,
            }) = bridge.offer(Source::Second, 156, input(0x80, 61, 0))
            else {
                panic!("an unmatched raw release stays outside the stage");
            };
            assert_eq!(bridge.stage.pressure(Source::First), (2, 0));
            assert_eq!(bridge.stage.pressure(Source::Second), (1, 2));

            if fault {
                let mut first = [0.0_f32; 128];
                audio
                    .render_private(
                        AudioBlockMut::new(&mut first, 128, ChannelLayout::Mono).unwrap(),
                    )
                    .unwrap();
                audio.test_fail_after_ingress_at = Some(SampleTime::new(128));
                let mut second = [1.0_f32; 128];
                assert_eq!(
                    audio.render_private(
                        AudioBlockMut::new(&mut second, 128, ChannelLayout::Mono).unwrap()
                    ),
                    Err(MixedOneShotRenderError::InjectedAfterIngress)
                );
            }
            let (teardown, raw, ended) = bridge.tear_down(control, audio);
            let (end, location) = if fault {
                (
                    MixedCollectionEnd::Faulted,
                    OnsetLocation::ChargedInFaultedCallback,
                )
            } else {
                (MixedCollectionEnd::Pending, OnsetLocation::Queued)
            };
            assert_eq!(ended.end, end);
            assert_eq!(ended.ingress.holds_outstanding, EventCount::measured(2));

            assert_eq!(
                teardown
                    .unoffered
                    .iter()
                    .map(|handoff| (handoff.source, handoff.at, handoff.link))
                    .collect::<Vec<_>>(),
                vec![
                    (Source::First, SampleTime::new(150), Some(raw_of(a_live))),
                    (Source::First, SampleTime::new(151), Some(raw_of(a_refused))),
                    (Source::Second, SampleTime::new(155), None),
                ]
            );
            // The returned raw owners still hold each unoffered packet's cell.
            for handoff in &teardown.unoffered {
                assert_eq!(
                    raw[handoff.source.index()].input.matched_onset(handoff.raw),
                    Ok(handoff.link)
                );
            }
            let b_unoffered_raw = teardown.unoffered[2].raw;
            assert_eq!(
                teardown.reservations,
                vec![
                    (Source::Second, b_live, raw_of(b_live)),
                    (Source::Second, b_unoffered, b_unoffered_raw),
                ]
            );
            let identity_of = |occurrence| {
                let MixedDisposition::Result(MixedIngressOutcome::Onset(Ok(identity))) = serviced
                    .iter()
                    .find(|result| result.occurrence == occurrence)
                    .unwrap()
                    .disposition
                else {
                    panic!("serviced onset must have been accepted");
                };
                identity
            };
            assert_eq!(
                teardown.onsets,
                vec![
                    (
                        a_live,
                        OnsetAtTeardown::Live {
                            identity: identity_of(a_live),
                            location,
                        }
                    ),
                    (
                        b_live,
                        OnsetAtTeardown::Live {
                            identity: identity_of(b_live),
                            location,
                        }
                    ),
                    (a_refused, OnsetAtTeardown::Refused(hold)),
                ]
            );

            // Raw settlement covers serviced, disposed, unoffered and
            // lane-refused packets plus B's frontier: four IDs on each source.
            let issued = teardown.raw_issued.clone();
            assert_eq!(issued.len(), 8);
            assert!(issued.contains(&(Source::Second, unmatched.raw, None)));
            let settled = settle_raw_by_cancellation(raw, &issued);
            assert_eq!(settled.bridge.len(), issued.len());
            assert_eq!(
                settled
                    .prepared_frontiers
                    .iter()
                    .map(|(source, _)| *source)
                    .collect::<Vec<_>>(),
                vec![Source::First, Source::Second]
            );
            for handoff in &teardown.unoffered {
                let (_, receipt) = settled
                    .bridge
                    .iter()
                    .find(|(_, receipt)| receipt.id == handoff.raw)
                    .unwrap();
                assert_eq!(receipt.matched_onset, handoff.link);
            }
        }
    }
}

/// Publish the serial session and join every issued raw ID to exactly one
/// delivered receipt with its admission link; only each input's prepared
/// tick-0 frontier is extra. Capture must complete.
fn publish_serial(
    raw: &mut SerialRaw,
    mut remaining: Vec<(Source, InputEventId, Option<InputEventId>)>,
) -> Vec<InputReceipt> {
    raw.owner.pump().unwrap();
    let mut samples = [0.0_f32; 512];
    raw.owner
        .render(AudioBlockMut::new(&mut samples, 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    raw.owner.pump().unwrap();
    let mut delivered = Vec::new();
    let mut prepared_frontiers = Vec::new();
    for (source, generation) in [Source::First, Source::Second]
        .into_iter()
        .zip(raw.generations)
    {
        while let Some(receipt) = raw.owner.collect_input(generation).unwrap() {
            assert!(matches!(receipt.outcome, InputOutcome::Delivered(_)));
            if let Some(position) = remaining
                .iter()
                .position(|&(owner, id, _)| owner == source && id == receipt.id)
            {
                let (_, _, link) = remaining.swap_remove(position);
                assert_eq!(receipt.matched_onset, link);
                delivered.push(receipt);
            } else {
                assert_eq!(
                    receipt.observation,
                    InputObservation::Frontier {
                        tick: InputTick::new(0)
                    }
                );
                prepared_frontiers.push(source);
            }
        }
    }
    assert!(
        remaining.is_empty(),
        "every issued raw ID must be delivered"
    );
    assert_eq!(prepared_frontiers, vec![Source::First, Source::Second]);
    raw.owner.finalize().unwrap();
    assert_eq!(
        raw.owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    delivered
}

fn recorded_occurrence(delivered: &[InputReceipt], raw: InputEventId) -> PerformedOccurrenceId {
    let receipt = delivered.iter().find(|receipt| receipt.id == raw).unwrap();
    let InputOutcome::Delivered(SessionSourceOutcome::Published(record)) = &receipt.outcome else {
        panic!("a raw note must reach the recorder: {receipt:?}");
    };
    record.occurrence.unwrap()
}

#[test]
fn delivered_raw_receipts_join_bridge_results_to_recorder_occurrences() {
    let on = input(0x90, 60, 100);
    let off = input(0x80, 60, 0);
    let hold = IngressRefused::Dropped {
        resource: ExhaustedResource::Hold,
    };
    for compiled_first in [true, false] {
        let (prepared, candidate) = history_tests::one_shot_with_boundary_on(compiled_first);
        let (mut control, mut audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        let (owner, generations) = ledger_tests::bridge_serial_owner(256);
        let mut bridge = RawStageBridge::with_raw(4, SerialRaw { owner, generations });

        let a_live = bridge.offer(Source::First, 140, on).unwrap();
        let b_live = bridge.offer(Source::Second, 141, on).unwrap();
        let a_refused = bridge.offer(Source::First, 142, on).unwrap();
        bridge.frontier(Source::Second, 143);
        let onsets: Vec<_> =
            std::iter::from_fn(|| bridge.service(&mut control, &mut audio)).collect();
        assert_eq!(onsets.len(), 3);
        assert!(onsets[0].accepted() && onsets[1].accepted());
        assert_eq!(
            (onsets[2].occurrence, onsets[2].disposition),
            (
                a_refused,
                MixedDisposition::Result(MixedIngressOutcome::Onset(Err(hold)))
            )
        );

        assert_eq!(bridge.offer(Source::First, 150, off), Ok(a_live));
        assert_eq!(bridge.offer(Source::First, 151, off), Ok(a_refused));
        assert_eq!(bridge.offer(Source::Second, 152, off), Ok(b_live));
        bridge.frontier(Source::First, 153);
        let releases: Vec<_> =
            std::iter::from_fn(|| bridge.service(&mut control, &mut audio)).collect();
        let released = |onset: &CombinedResult| {
            let MixedDisposition::Result(MixedIngressOutcome::Onset(Ok(identity))) =
                onset.disposition
            else {
                panic!("an accepted onset");
            };
            MixedDisposition::Result(MixedIngressOutcome::Release(Ok(MixedRetired {
                occurrence: onset.handle,
                cause: MixedRetireCause::Released(identity),
            })))
        };
        assert_eq!(
            releases
                .iter()
                .map(|result| (result.occurrence, result.disposition))
                .collect::<Vec<_>>(),
            vec![
                (a_live, released(&onsets[0])),
                (
                    a_refused,
                    MixedDisposition::Result(MixedIngressOutcome::Release(Ok(MixedRetired {
                        occurrence: onsets[2].handle,
                        cause: MixedRetireCause::AfterRefusal(hold),
                    })))
                ),
                (b_live, released(&onsets[1])),
            ]
        );

        // The mixed side is fully settled before the raw session publishes.
        let (teardown, mut raw, ended) = bridge.tear_down(control, audio);
        assert_eq!(ended.end, MixedCollectionEnd::Pending);
        assert!(teardown.unoffered.is_empty() && teardown.reservations.is_empty());
        assert!(teardown.onsets.is_empty());
        let mut remaining = teardown.raw_issued.clone();
        assert_eq!(remaining.len(), 8);
        for (source, generation) in [Source::First, Source::Second]
            .into_iter()
            .zip(raw.generations)
        {
            let frontier = raw
                .owner
                .advance_frontier(generation, InputTick::new(256))
                .unwrap();
            remaining.push((source, frontier, None));
        }
        let delivered = publish_serial(&mut raw, remaining);

        // Join each combined result to its recorder occurrence. The onset that
        // mixed ingress refused is still recorded and paired by the recorder.
        let recorded =
            |result: &CombinedResult| recorded_occurrence(&delivered, result.handoff.raw);
        let mut occurrences = Vec::new();
        for (onset, release) in onsets
            .iter()
            .zip([&releases[0], &releases[2], &releases[1]])
        {
            assert_eq!(onset.occurrence, release.occurrence);
            assert_eq!(release.handoff.link, Some(onset.handoff.raw));
            assert_eq!(recorded(onset), recorded(release));
            occurrences.push(recorded(onset));
        }
        assert_ne!(occurrences[0], occurrences[1]);
        assert_ne!(occurrences[0], occurrences[2]);
        assert_ne!(occurrences[1], occurrences[2]);
    }
}

/// Split-outcome rule: raw capture is never refused for a mixed-lane shortage.
/// A combined positive outcome needs raw admission, stage credit and a positive
/// audio result; a note may be recorded while its mixed disposition is refused.
#[test]
fn a_silent_peer_saturating_the_stage_never_refuses_raw_capture() {
    let on = input(0x90, 60, 100);
    let off = input(0x80, 60, 0);
    for compiled_first in [true, false] {
        let (prepared, candidate) = history_tests::one_shot_with_boundary_on(compiled_first);
        let (mut control, mut audio) = prepared
            .arm_one_shot(candidate, &history_tests::mixed_profile())
            .unwrap();
        let (owner, generations) = ledger_tests::bridge_serial_owner(256);
        let mut bridge = RawStageBridge::with_raw(2, SerialRaw { owner, generations });

        // Source B stays silent, so A's staged onset waits and fills A's lane.
        let staged = bridge.offer(Source::First, 150, on).unwrap();
        assert_eq!(bridge.stage.pressure(Source::First), (1, 1));
        let Err(BridgeRefused::Lane {
            handoff: shortage,
            reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
        }) = bridge.offer(Source::First, 151, on)
        else {
            panic!("the saturated lane must refuse only its own admission");
        };
        assert_eq!(bridge.offer(Source::First, 160, off), Ok(staged));
        let Err(BridgeRefused::Lane {
            handoff: orphan,
            reason: LaneRefusal::UnstagedOnset,
        }) = bridge.offer(Source::First, 161, off)
        else {
            panic!("the shortage onset's release has no staged occurrence");
        };
        assert_eq!(orphan.link, Some(shortage.raw));
        bridge.frontier(Source::First, 256);
        assert!(bridge.service(&mut control, &mut audio).is_none());

        // B's first progress releases the waiting lane; only the staged note
        // reaches mixed ingress.
        bridge.frontier(Source::Second, 256);
        let onset = bridge.service(&mut control, &mut audio).unwrap();
        let release = bridge.service(&mut control, &mut audio).unwrap();
        assert!(bridge.service(&mut control, &mut audio).is_none());
        assert_eq!((onset.occurrence, release.occurrence), (staged, staged));
        assert!(onset.accepted() && release.accepted());

        let (teardown, mut raw, ended) = bridge.tear_down(control, audio);
        assert_eq!(ended.end, MixedCollectionEnd::Pending);
        assert!(teardown.unoffered.is_empty() && teardown.reservations.is_empty());
        assert!(teardown.onsets.is_empty());
        let delivered = publish_serial(&mut raw, teardown.raw_issued);

        // Both notes are recorded and paired, including the one whose mixed
        // lane refused it; their recorder occurrences are distinct.
        let combined = recorded_occurrence(&delivered, onset.handoff.raw);
        assert_eq!(
            recorded_occurrence(&delivered, release.handoff.raw),
            combined
        );
        let split = recorded_occurrence(&delivered, shortage.raw);
        assert_eq!(recorded_occurrence(&delivered, orphan.raw), split);
        assert_ne!(combined, split);
    }
}

#[test]
fn raw_pedal_lift_redeems_its_stage_place_behind_sustained_note() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut bridge = RawStageBridge::new(3);
    let down_id = bridge
        .offer(Source::First, 100, input(0xb0, 64, 127))
        .unwrap();
    assert_eq!(bridge.stage.pressure(Source::First), (1, 1));
    bridge.frontier(Source::Second, 105);
    let selected = bridge.stage.peek_next();
    assert!(bridge.service(&mut control, &mut audio).is_none());
    assert_eq!(bridge.stage.peek_next(), selected);
    let down = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(down.occurrence, down_id);
    assert_eq!(down.handoff.link, None);
    assert!(
        matches!(down.outcome, Some(MixedIngressOutcome::PedalDown(Ok(_)))),
        "unexpected pedal-down result: {:?}",
        down.outcome
    );
    assert_eq!(bridge.stage.pressure(Source::First), (0, 1));

    let note_id = bridge
        .offer(Source::First, 101, input(0x90, 60, 100))
        .unwrap();
    assert_eq!(
        bridge.offer(Source::First, 102, input(0x80, 60, 0)),
        Ok(note_id)
    );
    let lift_id = bridge
        .offer(Source::First, 103, input(0xb0, 64, 0))
        .unwrap();
    assert_eq!(bridge.stage.pressure(Source::First), (3, 0));
    assert!(bridge.pedal_downs.is_empty());
    let onset = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(onset.occurrence, note_id);
    assert!(onset.accepted());
    let key_up = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(key_up.occurrence, note_id);
    assert_eq!(
        key_up.disposition,
        MixedDisposition::Result(MixedIngressOutcome::Sustained(onset.handle))
    );
    let lift = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(lift.occurrence, lift_id);
    assert_eq!(lift.handoff.link, None);
    assert_eq!(lift.paired_down_raw, Some(down.handoff.raw));
    let Some(MixedIngressOutcome::Lift(Ok(lifted))) = lift.outcome else {
        panic!("the selected lift must retire the sustained note");
    };
    assert_eq!(lifted.retired().collect::<Vec<_>>(), vec![onset.handle]);
    assert_eq!(bridge.stage.pressure(Source::First), (0, 0));

    let (teardown, raw, ended) = bridge.tear_down(control, audio);
    assert!(
        teardown.unoffered.is_empty()
            && teardown.reservations.is_empty()
            && teardown.pedal_lifts.is_empty()
    );
    assert_eq!(ended.ingress.holds_outstanding, EventCount::measured(0));
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn raw_pedal_down_full_keeps_its_head_and_staged_lift_for_retry() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(
            candidate,
            &history_tests::profile_with_ingress_depth(EventCount::measured(3)),
        )
        .unwrap();
    let filler_scope = MixedLiveScope::new(MixedLiveSource::new(999), MidiChannel::CH1);
    assert!(matches!(
        control
            .submit_pedal(
                MixedIngressOriginId(90),
                filler_scope,
                SampleTime::new(90),
                true
            )
            .unwrap(),
        MixedPedalSubmitted::Command(..)
    ));
    audio.service_test_ingress_queue();
    assert!(matches!(
        control.collect_ingress_result().unwrap().outcome,
        MixedIngressOutcome::PedalDown(Ok(_))
    ));
    let (filler, filler_request) = control
        .submit_bend(
            MixedIngressOriginId(91),
            filler_scope,
            SampleTime::new(91),
            crate::quantities::Cents::ZERO,
        )
        .unwrap();
    assert_eq!(control.ingress_room(), 1);

    let mut bridge = RawStageBridge::new(2);
    let down_id = bridge
        .offer(Source::First, 100, input(0xb0, 64, 127))
        .unwrap();
    let lift_id = bridge
        .offer(Source::First, 101, input(0xb0, 64, 0))
        .unwrap();
    bridge.frontier(Source::Second, 102);
    assert_eq!(bridge.stage.pressure(Source::First), (2, 0));
    let selected = bridge.stage.peek_next();
    assert_eq!(selected.unwrap().1.occurrence, down_id);
    assert_eq!(control.last_ingress_id, Some(filler));
    assert_eq!(
        bridge.service_pedal(&mut control, &mut audio),
        Err(MixedPedalSubmitError::Full)
    );
    assert_eq!(bridge.stage.peek_next(), selected);
    assert_eq!(bridge.handoffs.len(), 2);
    assert_eq!(control.last_ingress_id, Some(filler));

    audio.service_test_ingress_queue();
    let drained = control.collect_ingress_result().unwrap();
    assert_eq!((drained.id, drained.request), (filler, filler_request));
    assert!(matches!(drained.outcome, MixedIngressOutcome::Bend(Ok(_))));
    assert_eq!(control.ingress_room(), 2);
    let down = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(down.occurrence, down_id);
    assert!(matches!(
        down.outcome,
        Some(MixedIngressOutcome::PedalDown(Ok(_)))
    ));
    let lift = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(lift.occurrence, lift_id);
    assert_eq!(lift.paired_down_raw, Some(down.handoff.raw));
    assert!(matches!(
        lift.outcome,
        Some(MixedIngressOutcome::Lift(Ok(_)))
    ));
    assert_eq!(bridge.stage.pressure(Source::First), (0, 0));

    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert!(
        teardown.unoffered.is_empty()
            && teardown.reservations.is_empty()
            && teardown.pedal_lifts.is_empty()
    );
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn joined_teardown_names_the_raw_down_behind_a_pending_stage_lift() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut bridge = RawStageBridge::new(2);
    let occurrence = bridge
        .offer(Source::First, 100, input(0xb0, 64, 127))
        .unwrap();
    bridge.frontier(Source::Second, 101);
    let down = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(down.occurrence, occurrence);
    assert!(matches!(
        down.outcome,
        Some(MixedIngressOutcome::PedalDown(Ok(_)))
    ));
    assert_eq!(bridge.stage.pressure(Source::First), (0, 1));

    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.unoffered.is_empty() && teardown.reservations.is_empty());
    assert_eq!(
        teardown.pedal_lifts,
        vec![(
            Source::First,
            MidiChannel::CH1,
            occurrence,
            down.handoff.raw
        )]
    );
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn joined_teardown_pairs_an_unoffered_raw_lift_with_its_down() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut bridge = RawStageBridge::new(2);
    let down_id = bridge
        .offer(Source::First, 100, input(0xb0, 64, 127))
        .unwrap();
    bridge.frontier(Source::Second, 103);
    let down = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(down.occurrence, down_id);
    let lift_id = bridge
        .offer(Source::First, 101, input(0xb0, 64, 0))
        .unwrap();
    assert_eq!(bridge.stage.pressure(Source::First), (1, 0));
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.pedal_lifts.is_empty());
    assert_eq!(teardown.unoffered.len(), 1);
    assert_eq!(teardown.unoffered[0].original, input(0xb0, 64, 0));
    assert_eq!(lift_id, Occurrence(2));
    assert_eq!(
        teardown.staged_pedal_lifts,
        vec![(teardown.unoffered[0].raw, down.handoff.raw)]
    );
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn repeated_raw_pedal_down_uses_one_cell_without_another_lift_reservation() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut bridge = RawStageBridge::new(2);
    let first = bridge
        .offer(Source::First, 100, input(0xb0, 64, 127))
        .unwrap();
    bridge.frontier(Source::Second, 104);
    let down = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(down.occurrence, first);
    assert!(matches!(
        down.outcome,
        Some(MixedIngressOutcome::PedalDown(Ok(_)))
    ));

    let repeated = bridge
        .offer(Source::First, 101, input(0xb0, 64, 127))
        .unwrap();
    assert_eq!(bridge.stage.pressure(Source::First), (1, 1));
    let lift = bridge
        .offer(Source::First, 102, input(0xb0, 64, 0))
        .unwrap();
    assert_eq!(bridge.stage.pressure(Source::First), (2, 0));
    let duplicate = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!((duplicate.occurrence, duplicate.outcome), (repeated, None));
    let lifted = bridge
        .service_pedal(&mut control, &mut audio)
        .unwrap()
        .unwrap();
    assert_eq!(lifted.occurrence, lift);
    assert_eq!(lifted.paired_down_raw, Some(down.handoff.raw));
    assert!(matches!(
        lifted.outcome,
        Some(MixedIngressOutcome::Lift(Ok(_)))
    ));
    assert_eq!(bridge.stage.pressure(Source::First), (0, 0));
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.pedal_lifts.is_empty() && teardown.unoffered.is_empty());
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn stage_refused_raw_pedal_down_links_repeated_down_and_lift_without_commands() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut bridge = RawStageBridge::new(2);
    let note = bridge
        .offer(Source::First, 100, input(0x90, 60, 100))
        .unwrap();
    assert_eq!(bridge.stage.pressure(Source::First), (1, 1));
    let Err(BridgeRefused::Lane {
        handoff: down,
        reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
    }) = bridge.offer(Source::First, 101, input(0xb0, 64, 127))
    else {
        panic!("a full stage splits the raw pedal-down");
    };
    assert_eq!(down.link, None);
    assert_eq!(bridge.unstaged_pedal_downs.len(), 1);
    for (at, original) in [(102, input(0xb0, 64, 127)), (103, input(0xb0, 64, 0))] {
        let Err(BridgeRefused::Lane {
            handoff,
            reason: LaneRefusal::UnstagedPedal { down_raw },
        }) = bridge.offer(Source::First, at, original)
        else {
            panic!("later raw pedal edges keep the split down's identity");
        };
        assert_eq!(
            (handoff.original, handoff.at, handoff.link),
            (original, SampleTime::new(at), None)
        );
        assert_eq!(down_raw, down.raw);
    }
    assert!(bridge.unstaged_pedal_downs.is_empty());
    assert_eq!(bridge.stage.pressure(Source::First), (1, 1));
    assert_eq!(
        bridge.offer(Source::First, 104, input(0x80, 60, 0)),
        Ok(note)
    );
    bridge.frontier(Source::Second, 105);
    assert!(bridge.service(&mut control, &mut audio).unwrap().accepted());
    assert!(bridge.service(&mut control, &mut audio).unwrap().accepted());
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.unstaged_pedals.is_empty() && teardown.pedal_lifts.is_empty());
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn notes_after_a_stage_refused_pedal_down_retire_without_mixed_sustain() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(2),
        EventCount::measured(2),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut bridge = RawStageBridge::new(2);
    let first = bridge
        .offer(Source::First, 100, input(0x90, 60, 100))
        .unwrap();
    let Err(BridgeRefused::Lane {
        handoff: down,
        reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
    }) = bridge.offer(Source::First, 101, input(0xb0, 64, 127))
    else {
        panic!("the raw pedal-down must split at a full stage");
    };
    assert_eq!(
        bridge.offer(Source::First, 102, input(0x80, 60, 0)),
        Ok(first)
    );
    bridge.frontier(Source::Second, 106);
    assert!(bridge.service(&mut control, &mut audio).unwrap().accepted());
    assert!(bridge.service(&mut control, &mut audio).unwrap().accepted());

    let later = bridge
        .offer(Source::First, 103, input(0x90, 61, 100))
        .unwrap();
    assert_eq!(
        bridge.offer(Source::First, 104, input(0x80, 61, 0)),
        Ok(later)
    );
    assert!(bridge.service(&mut control, &mut audio).unwrap().accepted());
    let later_release = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(later_release.occurrence, later);
    assert!(
        later_release.accepted(),
        "mixed has no pedal-down to sustain it"
    );
    let Err(BridgeRefused::Lane {
        handoff: lift,
        reason: LaneRefusal::UnstagedPedal { down_raw },
    }) = bridge.offer(Source::First, 105, input(0xb0, 64, 0))
    else {
        panic!("the raw lift closes the earlier stage split");
    };
    assert_eq!((down_raw, lift.link), (down.raw, None));
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.unstaged_pedals.is_empty() && teardown.pedal_lifts.is_empty());
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn joined_teardown_names_an_unstaged_raw_pedal_down() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (control, audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let mut bridge = RawStageBridge::new(2);
    let note = bridge
        .offer(Source::First, 100, input(0x90, 60, 100))
        .unwrap();
    let Err(BridgeRefused::Lane {
        handoff: down,
        reason: LaneRefusal::Stage(StageRefusal::NoPacketCredit),
    }) = bridge.offer(Source::First, 101, input(0xb0, 64, 127))
    else {
        panic!("raw-accepted pedal-down is split at the full stage");
    };
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert_eq!(teardown.reservations[0].1, note);
    assert_eq!(teardown.unoffered.len(), 1);
    assert_eq!(
        teardown.unstaged_pedals,
        vec![(Source::First, MidiChannel::CH1, Occurrence(2), down.raw)]
    );
    assert!(teardown.pedal_lifts.is_empty());
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn source_and_channel_scopes_keep_independent_live_pedals() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(3),
        EventCount::measured(3),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &history_tests::mixed_profile())
        .unwrap();
    let note = input(0x90, 60, 100);
    let first_scope = source_scope(Source::First, note);
    let second_scope = source_scope(Source::Second, note);
    assert_ne!(first_scope, second_scope);
    assert_ne!(
        first_scope,
        source_scope(Source::First, input(0x91, 60, 100))
    );
    let pedal = control
        .submit_pedal(
            MixedIngressOriginId(90),
            first_scope,
            SampleTime::new(90),
            true,
        )
        .unwrap();
    assert!(matches!(pedal, super::MixedPedalSubmitted::Command(..)));
    audio.service_test_ingress_queue();
    assert!(matches!(
        control.collect_ingress_result().unwrap().outcome,
        MixedIngressOutcome::PedalDown(Ok(_))
    ));

    let mut bridge = RawStageBridge::new(4);
    let first = bridge.offer(Source::First, 100, note).unwrap();
    let other_channel = bridge
        .offer(Source::First, 101, input(0x91, 61, 100))
        .unwrap();
    let second = bridge.offer(Source::Second, 102, note).unwrap();
    assert_eq!(
        bridge.offer(Source::First, 110, input(0x80, 60, 0)),
        Ok(first)
    );
    assert_eq!(
        bridge.offer(Source::First, 111, input(0x81, 61, 0)),
        Ok(other_channel)
    );
    assert_eq!(
        bridge.offer(Source::Second, 112, input(0x80, 60, 0)),
        Ok(second)
    );
    bridge.frontier(Source::First, 113);
    let first_on = bridge.service(&mut control, &mut audio).unwrap();
    let other_on = bridge.service(&mut control, &mut audio).unwrap();
    let second_on = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(
        (
            first_on.occurrence,
            other_on.occurrence,
            second_on.occurrence
        ),
        (first, other_channel, second)
    );
    assert!(first_on.accepted() && other_on.accepted() && second_on.accepted());
    assert_ne!(first_on.handle, second_on.handle);
    let first_off = bridge.service(&mut control, &mut audio).unwrap();
    let other_off = bridge.service(&mut control, &mut audio).unwrap();
    let second_off = bridge.service(&mut control, &mut audio).unwrap();
    assert_eq!(
        (
            first_off.occurrence,
            other_off.occurrence,
            second_off.occurrence
        ),
        (first, other_channel, second)
    );
    assert_eq!(first_off.handle, first_on.handle);
    assert_eq!(other_off.handle, other_on.handle);
    assert_eq!(second_off.handle, second_on.handle);
    assert_eq!(
        first_off.disposition,
        MixedDisposition::Result(MixedIngressOutcome::Sustained(first_on.handle))
    );
    assert!(other_off.accepted() && second_off.accepted());
    assert!(bridge.stage.peek_next().is_none());

    let (teardown, raw, ended) = bridge.tear_down(control, audio);
    assert_eq!(ended.ingress.holds_outstanding, EventCount::measured(1));
    assert!(teardown.unoffered.is_empty() && teardown.reservations.is_empty());
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn in_flight_full_keeps_the_source_head_for_retry() {
    let (prepared, candidate) = history_tests::one_shot_with_live(
        true,
        crate::quantities::HeldNoteCount::measured(1),
        EventCount::measured(1),
    );
    let (mut control, mut audio) = prepared
        .arm_one_shot(
            candidate,
            &history_tests::profile_with_ingress_depth(EventCount::measured(3)),
        )
        .unwrap();
    let _prior = control
        .submit_onset(
            MixedIngressOriginId(1),
            super::TEST_SCOPE,
            SampleTime::new(100),
            crate::quantities::KeyIdentity::new(60).unwrap(),
            crate::quantities::NoteVelocity::FULL,
        )
        .unwrap();
    assert_eq!(control.ingress_outstanding.as_usize(), 1);
    assert_eq!(control.ingress_room(), 1);
    let mut bridge = RawStageBridge::new(4);
    bridge.frontier(Source::Second, 200);
    let waiting = bridge
        .offer(Source::First, 101, input(0x90, 61, 100))
        .unwrap();
    let selected = bridge.stage.peek_next();
    let handoff = bridge.handoffs[0].2;
    let command = control.last_ingress_id;
    assert_eq!(
        bridge.service_onset_or_refuse_full(&mut control, &mut audio, super::TEST_SCOPE),
        Err(MixedOnsetSubmitError::Full)
    );
    assert_eq!(bridge.stage.peek_next(), selected);
    assert_eq!(bridge.handoffs[0].2, handoff);
    assert_eq!(control.last_ingress_id, command);
    assert!(bridge.refused_onsets.is_empty());
    audio.service_test_ingress_queue();
    let prior = control.collect_ingress_result().unwrap();
    assert!(matches!(prior.outcome, MixedIngressOutcome::Onset(Ok(_))));
    assert_eq!(control.ingress_room(), 2);
    let retried = bridge
        .service_onset_or_refuse_full(&mut control, &mut audio, super::TEST_SCOPE)
        .unwrap()
        .unwrap();
    let OnsetDecision::Admitted(retried) = retried else {
        panic!("draining the earlier command restores onset room");
    };
    assert_eq!(retried.occurrence, waiting);
    assert_eq!(retried.handoff, handoff);
    assert!(bridge.refused_onsets.is_empty());
    let (teardown, raw, _ended) = bridge.tear_down(control, audio);
    assert!(teardown.refused_onsets.is_empty());
    let issued = teardown.raw_issued.clone();
    assert_eq!(
        settle_raw_by_cancellation(raw, &issued).bridge.len(),
        issued.len()
    );
}

#[test]
fn refused_onset_split_accounts_for_raw_link_and_both_teardown_states() {
    #[derive(Clone, Copy)]
    enum Tail {
        Absent,
        Staged,
        Settled,
    }

    for tail in [Tail::Absent, Tail::Staged, Tail::Settled] {
        let (prepared, candidate) = history_tests::one_shot_with_live(
            true,
            crate::quantities::HeldNoteCount::measured(1),
            EventCount::measured(1),
        );
        let (mut control, mut audio) = prepared
            .arm_one_shot(
                candidate,
                &history_tests::profile_with_ingress_depth(EventCount::measured(3)),
            )
            .unwrap();
        let scope = super::TEST_SCOPE;
        let pedal = control
            .submit_pedal(MixedIngressOriginId(1), scope, SampleTime::new(100), true)
            .unwrap();
        assert!(matches!(pedal, super::MixedPedalSubmitted::Command(..)));
        audio.service_test_ingress_queue();
        assert!(matches!(
            control.collect_ingress_result().unwrap().outcome,
            MixedIngressOutcome::PedalDown(Ok(_))
        ));

        let mut bridge = RawStageBridge::new(4);
        bridge.frontier(Source::Second, 200);
        let first = bridge
            .offer(Source::First, 110, input(0x90, 60, 100))
            .unwrap();
        let admitted = bridge
            .service_onset_or_refuse_full(&mut control, &mut audio, scope)
            .unwrap()
            .unwrap();
        assert!(matches!(
            &admitted,
            OnsetDecision::Admitted(CombinedResult {
                occurrence,
                disposition: MixedDisposition::Result(MixedIngressOutcome::Onset(Ok(_))),
                ..
            }) if *occurrence == first
        ));
        let OnsetDecision::Admitted(admitted) = admitted else {
            panic!("first onset must be admitted");
        };
        let blocked = bridge
            .offer(Source::First, 111, input(0x90, 60, 100))
            .unwrap();
        assert_eq!(control.ingress_room(), 1);
        let command = control.last_ingress_id;
        let handles = control.occurrence_ledger.held();
        let split_onset = bridge
            .service_onset_or_refuse_full(&mut control, &mut audio, scope)
            .unwrap()
            .unwrap();
        let OnsetDecision::Refused(split_onset) = split_onset else {
            panic!("the selected onset has no ordinary command place");
        };
        assert_eq!(split_onset.occurrence, blocked);
        assert_eq!(
            split_onset.disposition,
            SplitDisposition::Onset {
                reason: MixedOnsetSubmitError::Full,
            }
        );
        assert_eq!(split_onset.handoff.original, input(0x90, 60, 100));
        assert_eq!(control.last_ingress_id, command);
        assert_eq!(control.occurrence_ledger.held(), handles);
        assert!(bridge.handoffs.is_empty());
        assert_eq!(bridge.refused_onsets.len(), 1);
        assert_eq!(bridge.refused_onsets[0].onset_raw, split_onset.handoff.raw);
        assert_eq!(bridge.stage.pressure(Source::First), (0, 2));

        if !matches!(tail, Tail::Absent) {
            let first_release = bridge
                .offer(Source::First, 112, input(0x80, 60, 0))
                .unwrap();
            let refused_release = bridge
                .offer(Source::First, 113, input(0x80, 60, 0))
                .unwrap();
            assert_eq!(first_release, first);
            assert_eq!(refused_release, blocked);
            assert_eq!(bridge.handoffs[0].2.link, Some(admitted.handoff.raw));
            assert_eq!(bridge.handoffs[1].2.link, Some(split_onset.handoff.raw));
            assert_eq!(bridge.stage.pressure(Source::First), (2, 0));
            if matches!(tail, Tail::Settled) {
                let first_result = bridge.service(&mut control, &mut audio).unwrap();
                assert_eq!(first_result.occurrence, first);
                assert_eq!(first_result.handoff.link, Some(admitted.handoff.raw));
                let split = bridge.service_refused_key_up().unwrap();
                assert_eq!(split.occurrence, blocked);
                assert_eq!(split.handoff.link, Some(split_onset.handoff.raw));
                assert_eq!(
                    split.disposition,
                    SplitDisposition::KeyUpAfterOnset {
                        onset_raw: split_onset.handoff.raw,
                        reason: MixedOnsetSubmitError::Full,
                    }
                );
                assert!(bridge.refused_onsets.is_empty());
                assert_eq!(bridge.stage.pressure(Source::First), (0, 0));
            }
        }

        let (teardown, raw, ended) = bridge.tear_down(control, audio);
        assert_eq!(ended.end, MixedCollectionEnd::Pending);
        assert_eq!(
            teardown.onsets.len(),
            usize::from(!matches!(tail, Tail::Settled))
        );
        assert_eq!(
            teardown.reservations.len(),
            2 * usize::from(matches!(tail, Tail::Absent))
        );
        match tail {
            Tail::Absent => {
                assert_eq!(
                    teardown.refused_onsets,
                    vec![(
                        RefusedOnsetTombstone {
                            source: Source::First,
                            occurrence: blocked,
                            onset_raw: split_onset.handoff.raw,
                            reason: MixedOnsetSubmitError::Full,
                        },
                        RefusedOnsetPending::ReservedKeyUp,
                    )]
                );
            }
            Tail::Staged => {
                assert_eq!(teardown.unoffered.len(), 2);
                assert_eq!(teardown.unoffered[0].link, Some(admitted.handoff.raw));
                assert_eq!(teardown.unoffered[1].link, Some(split_onset.handoff.raw));
                assert_eq!(
                    teardown.refused_onsets,
                    vec![(
                        RefusedOnsetTombstone {
                            source: Source::First,
                            occurrence: blocked,
                            onset_raw: split_onset.handoff.raw,
                            reason: MixedOnsetSubmitError::Full,
                        },
                        RefusedOnsetPending::StagedKeyUp(teardown.unoffered[1].raw),
                    )]
                );
            }
            Tail::Settled => {
                assert!(teardown.unoffered.is_empty());
                assert!(teardown.refused_onsets.is_empty());
            }
        }
        let issued = teardown.raw_issued.clone();
        let settled = settle_raw_by_cancellation(raw, &issued);
        assert_eq!(settled.bridge.len(), issued.len());
    }
}

#[path = "stage_bend_tests.rs"]
mod bend;
