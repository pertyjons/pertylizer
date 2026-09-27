//! Reduced bounded mixed lane after independent raw source service.

use std::collections::VecDeque;

use super::{MixedIngressOriginId, MixedIngressOutcome, MixedIngressRequest, history_tests};
use crate::{
    recording::notes::{Midi1Event, Midi1Input},
    time::SampleTime,
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
