//! Reduced bounded mixed lane after independent raw source service.

use std::collections::VecDeque;

use super::{
    EventPayload, MixedCollection, MixedCollectionEnd, MixedIngressOriginId, MixedIngressOutcome,
    MixedIngressRequest, MixedOneShotAudio, MixedOneShotControl, MixedOneShotRenderError,
    MixedOneShotTeardown, TimedEvent, history_tests,
};
use crate::{
    host::{
        ConnectionGeneration, EndpointId,
        input::{
            InputCapacity, InputError, InputEventId, InputLimits, InputObservation, InputOutcome,
            InputRate, InputReceipt, InputTick, InputTickSpan, SimulatedInputClock,
            SimulatedNoteInput,
        },
    },
    identity::NoteIdentity,
    ingress::{ExhaustedResource, IngressRefused},
    quantities::{ChannelLayout, EventCount, PreparedBytes},
    recording::notes::{Midi1Event, Midi1Input},
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
    frontier: Option<SampleTime>,
    last_offered: Option<SampleTime>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StageRefusal {
    Order,
    NoPacketCredit,
    NoReleaseReservation,
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
        if lane.entries.len() + lane.reserved_releases.len() + 2 > self.cells_per_source {
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
        assert!(lane.entries.len() + lane.reserved_releases.len() <= self.cells_per_source);
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

    fn pop_next(&mut self) -> Option<(Source, Packet)> {
        let source = self.next()?;
        let lane = &mut self.lanes[source.index()];
        let entry = lane.entries.pop_front()?;
        if let Some(frontier) = entry.frontier_after {
            lane.frontier = Some(frontier);
        }
        Some((source, entry.packet))
    }

    fn pressure(&self, source: Source) -> (usize, usize) {
        let lane = &self.lanes[source.index()];
        (lane.entries.len(), lane.reserved_releases.len())
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
        let first_request = MixedIngressRequest::Onset {
            origin: MixedIngressOriginId(first.occurrence.0),
            at: first.at,
            key,
            velocity,
        };
        let first_command = control.submit_ingress(first_request).unwrap();
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
        let second_request = MixedIngressRequest::Release {
            origin: MixedIngressOriginId(second.occurrence.0),
            at: second.at,
            identity: first_identity,
        };
        let second_command = control.submit_ingress(second_request).unwrap();
        audio.service_test_ingress_queue();
        let second_result = control.collect_ingress_result().unwrap();
        assert_eq!(
            (second_result.id, second_result.request),
            (second_command, second_request)
        );
        assert_eq!(second_result.outcome, MixedIngressOutcome::Release(Ok(())));

        let (source, third) = stage.pop_next().unwrap();
        assert_eq!((source, third), (Source::First, onset(1, 150)));
        let Midi1Event::NoteOn { key, velocity } = third.original.event() else {
            panic!("waiting staged packet must be an onset");
        };
        let third_request = MixedIngressRequest::Onset {
            origin: MixedIngressOriginId(third.occurrence.0),
            at: third.at,
            key,
            velocity,
        };
        let third_command = control.submit_ingress(third_request).unwrap();
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
        let last_request = MixedIngressRequest::Release {
            origin: MixedIngressOriginId(last.occurrence.0),
            at: last.at,
            identity: third_identity,
        };
        let last_command = control.submit_ingress(last_request).unwrap();
        audio.service_test_ingress_queue();
        let last_result = control.collect_ingress_result().unwrap();
        assert_eq!(
            (last_result.id, last_result.request),
            (last_command, last_request)
        );
        assert_eq!(last_result.outcome, MixedIngressOutcome::Release(Ok(())));
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
    /// The actual audio-side result for this staged packet's request.
    Result(MixedIngressOutcome),
    /// The release's onset was refused by mixed ingress, so no identity exists
    /// to release. The release is not offered; it carries that onset refusal.
    OnsetRefused(IngressRefused),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CombinedResult {
    handoff: RawHandoff,
    occurrence: Occurrence,
    disposition: MixedDisposition,
}

impl CombinedResult {
    /// Both consumers accepted: raw admission produced `handoff`, and the
    /// audio-side mixed result for the same staged occurrence is positive.
    fn accepted(&self) -> bool {
        matches!(
            self.disposition,
            MixedDisposition::Result(
                MixedIngressOutcome::Onset(Ok(_)) | MixedIngressOutcome::Release(Ok(()))
            )
        )
    }
}

struct RawSource {
    input: SimulatedNoteInput,
    generation: ConnectionGeneration,
}

fn raw_source(index: usize) -> RawSource {
    let mut input = SimulatedNoteInput::new(
        EndpointId::new(format!("stage-{index}")).unwrap(),
        InputLimits {
            cells: InputCapacity::new(12).unwrap(),
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

/// Actual raw owners serve each source independently; only raw-admitted note
/// packets enter the bounded mixed stage, and releases find their staged
/// occurrence through the raw owner's own same-key FIFO link.
struct RawStageBridge {
    stage: Stage,
    sources: [RawSource; 2],
    staged_onsets: Vec<(Source, InputEventId, Occurrence)>,
    handoffs: Vec<(Occurrence, bool, RawHandoff)>,
    onset_results: Vec<(Occurrence, Result<NoteIdentity, IngressRefused>)>,
    /// Every raw ID a raw owner issued to this bridge, with its admission link.
    raw_issued: Vec<(Source, InputEventId, Option<InputEventId>)>,
    next_occurrence: u64,
}

impl RawStageBridge {
    fn new(cells_per_source: usize) -> Self {
        Self {
            stage: Stage::new(cells_per_source),
            sources: [raw_source(0), raw_source(1)],
            staged_onsets: Vec::new(),
            handoffs: Vec::new(),
            onset_results: Vec::new(),
            raw_issued: Vec::new(),
            next_occurrence: 1,
        }
    }

    fn offer(
        &mut self,
        source: Source,
        at: u64,
        original: Midi1Input,
    ) -> Result<Occurrence, BridgeRefused> {
        let at = SampleTime::new(at);
        let owner = &mut self.sources[source.index()];
        let raw = owner
            .input
            .offer_message(owner.generation, InputTick::new(at.as_u64()), at, original)
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
        let link = owner.input.matched_onset(raw);
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
            _ => Err(refuse(LaneRefusal::Ordinary)),
        }
    }

    fn frontier(&mut self, source: Source, at: u64) {
        let owner = &mut self.sources[source.index()];
        let frontier = owner
            .input
            .advance_frontier(owner.generation, InputTick::new(at))
            .unwrap();
        assert_eq!(owner.input.matched_onset(frontier), Ok(None));
        self.raw_issued.push((source, frontier, None));
        self.stage.frontier(source, SampleTime::new(at)).unwrap();
    }

    /// Offer the next time-ordered staged packet to mixed ingress. A lane
    /// selects an onset before its release, so the onset's mixed result is
    /// known here; a refused onset's release is disposed without an offer.
    fn service(
        &mut self,
        control: &mut MixedOneShotControl,
        audio: &mut MixedOneShotAudio,
    ) -> Option<CombinedResult> {
        let (source, packet) = self.stage.pop_next()?;
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
        let request = match packet.original.event() {
            Midi1Event::NoteOn { key, velocity } => MixedIngressRequest::Onset {
                origin,
                at: packet.at,
                key,
                velocity,
            },
            Midi1Event::KeyRelease { .. } => {
                let position = self
                    .onset_results
                    .iter()
                    .position(|&(id, _)| id == occurrence)
                    .expect("a lane selects each onset before its release");
                let identity = match self.onset_results.swap_remove(position).1 {
                    Ok(identity) => identity,
                    Err(refusal) => {
                        return Some(CombinedResult {
                            handoff,
                            occurrence,
                            disposition: MixedDisposition::OnsetRefused(refusal),
                        });
                    }
                };
                MixedIngressRequest::Release {
                    origin,
                    at: packet.at,
                    identity,
                }
            }
            _ => unreachable!("the stage holds only note packets"),
        };
        let command = control.submit_ingress(request).unwrap();
        audio.service_test_ingress_queue();
        let result = control.collect_ingress_result().unwrap();
        assert_eq!((result.id, result.request), (command, request));
        if let MixedIngressOutcome::Onset(onset) = result.outcome {
            self.onset_results.push((occurrence, onset));
        }
        Some(CombinedResult {
            handoff,
            occurrence,
            disposition: MixedDisposition::Result(result.outcome),
        })
    }
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
        handoff: ordinary,
        reason: LaneRefusal::Ordinary,
    }) = bridge.offer(Source::First, 163, pedal)
    else {
        panic!("an ordinary packet has no staged disposition in this model");
    };
    assert_eq!((ordinary.original, ordinary.link), (pedal, None));
    assert_eq!(bridge.stage.pressure(Source::First), (2, 0));

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
fn mixed_onset_refusal_disposes_its_raw_linked_release_without_an_offer() {
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
        assert_eq!(disposed.disposition, MixedDisposition::OnsetRefused(hold));
        assert!(!disposed.accepted());
        assert!(control.collect_ingress_result().is_none());
        assert_eq!(bridge.stage.pressure(Source::First), (0, 0));

        // B's release still waits for A's frontier beyond its stamp.
        assert!(bridge.service(&mut control, &mut audio).is_none());
        bridge.frontier(Source::First, 153);
        let peer_released = bridge.service(&mut control, &mut audio).unwrap();
        assert_eq!(peer_released.handoff.link, Some(peer.handoff.raw));
        assert!(peer_released.accepted());

        // The actual releases returned both holds; a later onset is accepted.
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

/// Every bridge credit outstanding at teardown, one entry per charge: a stage
/// packet cell, a stage release reservation, or a retained onset result.
#[derive(Debug, PartialEq)]
struct BridgeTeardown {
    raw_issued: Vec<(Source, InputEventId, Option<InputEventId>)>,
    unoffered: Vec<RawHandoff>,
    reservations: Vec<(Source, Occurrence, InputEventId)>,
    onsets: Vec<(Occurrence, OnsetAtTeardown)>,
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

impl RawStageBridge {
    /// Stop the mixed owner and account for every bridge charge exactly once.
    /// The raw owners are returned with their cells; no raw receipt is settled.
    fn tear_down(
        mut self,
        control: MixedOneShotControl,
        audio: MixedOneShotAudio,
    ) -> (BridgeTeardown, [RawSource; 2], MixedOneShotTeardown) {
        let MixedCollection::Ended(ended) = control.collect(audio).unwrap() else {
            panic!("a pending or faulted mixed owner must end at teardown");
        };
        let mut teardown = BridgeTeardown {
            raw_issued: std::mem::take(&mut self.raw_issued),
            unoffered: Vec::new(),
            reservations: Vec::new(),
            onsets: Vec::new(),
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
        }
        assert!(self.handoffs.is_empty() && self.staged_onsets.is_empty());
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
        (teardown, self.sources, ended)
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
