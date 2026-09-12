//! Test-only finite loop-journal transfer. No physical callback or source mapping.

use ringbuf::{
    HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};
use std::sync::Arc;
use synth_engine_v2::{
    compile::{RenderConfig, compile},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    looping::{
        CompiledLoopStream, LoopBoundary, LoopSettings,
        journal::{JournaledLoopStream, LoopJournalEnd, LoopJournalEndReason},
    },
    profile::HostProfile,
    quantities::{ChannelLayout, LoopPassCount, PreparedBytes, SampleRate},
    render::AudioBlockMut,
    schedule::AdmittedCompiledStream,
    time::{FrameCount, PlanPosition, SampleTime},
    transport::LoopInterval,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Record {
    Boundary(LoopBoundary),
    End(LoopJournalEnd),
}

#[derive(Default)]
struct Publisher {
    sent: usize,
    end_sent: bool,
    failed: Option<Record>,
}

impl Publisher {
    // This bounded test traverses the retained prefix to find its unsent suffix.
    // It proves custody, not admission of a physical callback's transfer cost.
    fn publish(&mut self, journal: &JournaledLoopStream, lane: &mut HeapProd<Record>) {
        if self.failed.is_some() {
            return;
        }
        for boundary in journal.boundaries().skip(self.sent) {
            if let Err(record) = lane.try_push(Record::Boundary(*boundary)) {
                self.failed = Some(record);
                return;
            }
            self.sent += 1;
        }
        if !self.end_sent
            && let Some(end) = journal.end()
        {
            match lane.try_push(Record::End(end)) {
                Ok(()) => self.end_sent = true,
                Err(record) => self.failed = Some(record),
            }
        }
    }
}

fn prepare_stream(length: u64) -> CompiledLoopStream {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .unwrap();
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
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
    let plan = compile(&graph, &RenderConfig::new(profile))
        .into_plan()
        .unwrap();
    let events = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    CompiledLoopStream::prepare(
        plan,
        events,
        profile,
        LoopSettings::new(
            LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(length)).unwrap(),
            PlanPosition::ZERO,
            PreparedBytes::measured(1_000_000),
        )
        .unwrap(),
    )
    .unwrap()
}

fn prepare(length: u64, passes: LoopPassCount) -> JournaledLoopStream {
    let stream = prepare_stream(length);
    JournaledLoopStream::prepare(stream, passes, PreparedBytes::measured(1_000_000)).unwrap()
}

#[test]
fn reported_journal_heap_charge_matches_actual_retained_allocations() {
    for passes in [1_u32, 2, 17] {
        let stream = prepare_stream(65);
        let mut owner = None;
        let measured = allocation_counter::measure(|| {
            owner = Some(
                JournaledLoopStream::prepare(
                    stream,
                    LoopPassCount::limit(passes).unwrap(),
                    PreparedBytes::measured(1_000_000),
                )
                .unwrap(),
            );
        });
        let owner = owner.unwrap();
        assert_eq!(
            u64::try_from(measured.bytes_current).unwrap(),
            owner.storage_bytes().get()
        );
        assert_eq!(measured.count_current, i64::from(passes != 1));
    }
}

fn no_heap_activity(f: impl FnOnce()) {
    let measured = allocation_counter::measure(f);
    assert_eq!(measured.count_total, 0);
    assert_eq!(measured.count_current, 0);
}

fn records(owner: &JournaledLoopStream) -> Vec<Record> {
    owner
        .boundaries()
        .copied()
        .map(Record::Boundary)
        .chain(owner.end().map(Record::End))
        .collect()
}

#[test]
fn stalled_reader_retains_every_boundary_and_terminal_across_os_thread_join() {
    for length in [1_u64, 65] {
        for passes in [1_u32, 3, 17] {
            for partition in [1_usize, 37, 64, 256, 512] {
                let mut owner = prepare(length, LoopPassCount::limit(passes).unwrap());
                let backing = Arc::new(HeapRb::new(passes as usize));
                let (mut producer, mut consumer) = Arc::clone(&backing).split();
                let mut output = Box::new([9.0; 2112]);
                let worker = std::thread::spawn(move || {
                    let mut publisher = Publisher::default();
                    for chunk in output.chunks_mut(partition) {
                        let frames = chunk.len();
                        no_heap_activity(|| {
                            owner
                                .render(
                                    AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).unwrap(),
                                )
                                .unwrap();
                            publisher.publish(&owner, &mut producer);
                        });
                    }
                    // Return every owning value; destruction is after join on the reader.
                    (owner, producer, publisher, output)
                });
                // Do not drain a single cell until the entire writer has finished.
                let (owner, producer, publisher, output) = worker.join().unwrap();
                assert_eq!(publisher.failed, None);
                assert!(publisher.end_sent);
                let mut received = Vec::new();
                while let Some(record) = consumer.try_pop() {
                    received.push(record);
                }
                assert_eq!(received, records(&owner));
                assert_eq!(received.len(), passes as usize);
                assert_eq!(owner.end().unwrap().reason, LoopJournalEndReason::PassLimit);
                assert_eq!(owner.end().unwrap().at.as_u64(), u64::from(passes) * length);
                for (frame, sample) in output.iter().enumerate() {
                    let expected = if frame < 64 {
                        0.0
                    } else {
                        f32::from(((frame - 64) as u64).is_multiple_of(length))
                    };
                    assert_eq!(*sample, expected);
                }
                drop((owner, producer, consumer, backing, output));
            }
        }
    }
}

#[test]
fn draining_the_lane_cannot_renew_the_total_pass_budget() {
    let mut owner = prepare(65, LoopPassCount::limit(3).unwrap());
    let backing = Arc::new(HeapRb::new(3));
    let (mut producer, mut consumer) = Arc::clone(&backing).split();
    let mut publisher = Publisher::default();
    let mut received = Vec::new();
    for _ in 0..100 {
        no_heap_activity(|| {
            owner
                .render(AudioBlockMut::new(&mut [0.0; 64], 64, ChannelLayout::Mono).unwrap())
                .unwrap();
            publisher.publish(&owner, &mut producer);
        });
        while let Some(record) = consumer.try_pop() {
            received.push(record);
        }
    }
    assert_eq!(received.len(), 3);
    assert_eq!(received, records(&owner));
    assert_eq!(publisher.failed, None);
    assert_eq!(owner.end().unwrap().at, SampleTime::new(195));
    assert_eq!(owner.acknowledged().clock, SampleTime::new(6336));
}

#[test]
fn broken_lane_retains_failed_record_and_authoritative_journal_for_joined_recovery() {
    let mut owner = prepare(65, LoopPassCount::limit(3).unwrap());
    // Fault injection: this deliberately violates the required P-cell reservation.
    let backing = Arc::new(HeapRb::new(1));
    let (mut producer, mut consumer) = Arc::clone(&backing).split();
    let worker = std::thread::spawn(move || {
        let mut publisher = Publisher::default();
        no_heap_activity(|| {
            owner
                .render(AudioBlockMut::new(&mut [0.0; 512], 512, ChannelLayout::Mono).unwrap())
                .unwrap();
            publisher.publish(&owner, &mut producer);
        });
        (owner, producer, publisher)
    });
    let (owner, mut producer, mut publisher) = worker.join().unwrap();
    let expected = records(&owner);
    assert_eq!(consumer.try_pop(), Some(expected[0]));
    assert_eq!(publisher.failed, Some(expected[1]));
    publisher.publish(&owner, &mut producer);
    assert_eq!(
        consumer.try_pop(),
        None,
        "the first protocol fault remains latched"
    );
    assert_eq!(
        expected.len(),
        3,
        "joined recovery retains the unsent terminal too"
    );
}

#[test]
fn joined_owner_can_finish_observation_without_a_final_render_callback() {
    let mut owner = prepare(65, LoopPassCount::limit(17).unwrap());
    let worker = std::thread::spawn(move || {
        owner
            .render(AudioBlockMut::new(&mut [0.0; 256], 256, ChannelLayout::Mono).unwrap())
            .unwrap();
        owner
    });
    let mut owner = worker.join().unwrap();
    let before = owner.acknowledged();
    let end = owner.finish();
    assert_eq!(end.reason, LoopJournalEndReason::Finished);
    assert_eq!(end.at, before.clock);
    assert_eq!(records(&owner).len(), 3);
}
