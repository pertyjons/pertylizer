//! Actual session quanta, including transport and idle carry, stay passive.
use super::*;
use crate::{
    compile::{RenderConfig, compile},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    observe::ObservationSubscriptions,
    quantities::{Amplitude, ChannelLayout},
    render::AudioBlockMut,
    render_allocation::count_allocs,
    time::FrameCount,
};

const MONITOR: NodeId = NodeId::new(2);
const FRAMES: usize = 8192;
const STOP: usize = 7168;
const Q: usize = crate::time::QUANTUM_FRAMES as usize;

#[derive(Clone, Copy)]
enum Reader {
    None,
    Draining,
    Stalled,
    Foreign,
}

fn monitored_plan() -> CompiledPlan {
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Constant {
                level: Amplitude::new(0.25).expect("level"),
            },
            ExecutionScope::Global,
        )
        .node(MONITOR, IrNodeKind::Monitor, ExecutionScope::Global)
        .node(NodeId::new(3), IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (MONITOR, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (MONITOR, PortId::FIRST),
            (NodeId::new(3), PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("graph");
    compile(&graph, &RenderConfig::new(super::tests::profile()))
        .into_plan()
        .expect("plan")
}

struct Observed {
    sound: Vec<f32>,
    signal: Vec<f32>,
    dropped: FrameCount,
}

fn run(reader: Reader, partition: usize) -> Observed {
    let profile = super::tests::profile();
    let plan = monitored_plan();
    let observed_plan = if matches!(reader, Reader::Foreign) {
        monitored_plan()
    } else {
        plan.clone()
    };
    let mut store = ObservationSubscriptions::prepare(&profile, &observed_plan).expect("store");
    let tap = store
        .subscribe(
            &observed_plan,
            observed_plan
                .resolve_tap(MONITOR, PortId::FIRST)
                .expect("tap"),
        )
        .expect("subscription");
    let stream = AdmittedCompiledStream::admit(&plan, &[]).expect("program");
    let (mut control, mut audio) =
        SessionControl::prepare(plan, stream, profile, super::tests::limits()).expect("session");
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).expect("play"))
        .expect("enqueue play");
    audio
        .enqueue(
            control
                .prepare_stop(SampleTime::new(STOP as u64))
                .expect("stop"),
        )
        .expect("enqueue stop");
    // The whole store moves with the actual audio owner; control never borrows it.
    let (sound, mut signal, length, dropped, mut audio) = std::thread::spawn(move || {
        let mut sound = vec![0.0; FRAMES];
        let mut signal = vec![0.0; FRAMES];
        let mut length = 0;
        let mut dropped = 0;
        for chunk in sound.chunks_mut(partition) {
            let frames = chunk.len();
            assert_eq!(
                count_allocs(|| {
                    let output =
                        AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).expect("block");
                    if matches!(reader, Reader::None) {
                        audio.render(output).expect("render");
                    } else {
                        audio
                            .render_observed(output, Some(&mut store))
                            .expect("observed render");
                    }
                    if matches!(reader, Reader::Draining) {
                        let read = store.read(tap, &mut signal[length..]).expect("read");
                        length += read.frames.as_usize().expect("frames");
                        dropped += read.dropped.as_u64();
                        assert_eq!(read.plan, observed_plan.id());
                        assert_eq!(read.generation.as_u64(), 0);
                        assert!(!read.ended);
                    }
                }),
                0,
                "session and observation must not touch the allocator"
            );
        }
        if !matches!(reader, Reader::Draining) {
            let read = store.read(tap, &mut signal).expect("final read");
            length = read.frames.as_usize().expect("frames");
            dropped = read.dropped.as_u64();
            assert_eq!(read.plan, observed_plan.id());
            if matches!(reader, Reader::Foreign) {
                assert!(read.ended);
            }
        }
        (sound, signal, length, FrameCount::new(dropped), audio)
    })
    .join()
    .expect("joined audio owner");
    signal.truncate(length);
    let mut receipts = 0;
    while let Some(completed) = audio.take_completed() {
        let _receipt = control.collect(completed).expect("collect exact command");
        receipts += 1;
    }
    assert_eq!(receipts, 2);
    assert!(!control.has_outstanding());
    Observed {
        sound,
        signal,
        dropped,
    }
}

#[test]
fn actual_session_taps_are_passive_across_partitions_transport_and_subscriber_stalls() {
    let reference = run(Reader::None, 512);
    assert!(reference.sound[..Q].iter().all(|sample| *sample == 0.0));
    assert!(
        reference.sound[Q..STOP + Q]
            .iter()
            .all(|sample| *sample == 0.25)
    );
    assert!(
        reference.sound[STOP + Q..]
            .iter()
            .all(|sample| *sample == 0.0)
    );
    for partition in [1, 37, 64, 256, 512] {
        let draining = run(Reader::Draining, partition);
        let stalled = run(Reader::Stalled, partition);
        let foreign = run(Reader::Foreign, partition);
        assert_eq!(draining.sound, reference.sound);
        assert_eq!(stalled.sound, reference.sound);
        assert_eq!(foreign.sound, reference.sound);
        assert_eq!(draining.signal, vec![0.25; STOP]);
        assert_eq!(draining.dropped, FrameCount::ZERO);
        assert_eq!(
            stalled.signal.len() as u64 + stalled.dropped.as_u64(),
            STOP as u64
        );
        assert!(stalled.dropped > FrameCount::ZERO);
        assert!(stalled.signal.iter().all(|sample| *sample == 0.25));
        assert!(foreign.signal.is_empty());
    }
}

#[test]
fn invalid_output_has_identical_terminal_faults_with_or_without_a_foreign_store() {
    let mut expected = None;
    for observed in [false, true] {
        let profile = super::tests::profile();
        let plan = monitored_plan();
        let foreign = monitored_plan();
        let mut store = ObservationSubscriptions::prepare(&profile, &foreign).expect("store");
        let handle = store
            .subscribe(
                &foreign,
                foreign.resolve_tap(MONITOR, PortId::FIRST).expect("tap"),
            )
            .expect("subscription");
        let stream = AdmittedCompiledStream::admit(&plan, &[]).expect("program");
        let (_control, mut audio) =
            SessionControl::prepare(plan, stream, profile, super::tests::limits())
                .expect("session");
        let mut output = [9.0; 2];
        let error = audio
            .render_observed(
                AudioBlockMut::new(&mut output, 1, ChannelLayout::Stereo).expect("stereo shape"),
                observed.then_some(&mut store),
            )
            .expect_err("wrong layout is terminal in a session");
        assert_eq!(output, [0.0; 2]);
        if let Some(expected) = expected {
            assert_eq!(error, expected);
        } else {
            expected = Some(error);
        }
        assert!(!store.read(handle, &mut []).expect("read").ended);
        let mut later = [9.0];
        assert_eq!(
            audio.render_observed(
                AudioBlockMut::new(&mut later, 1, ChannelLayout::Mono).expect("mono shape"),
                observed.then_some(&mut store),
            ),
            Err(error)
        );
        assert_eq!(later, [0.0]);
        assert!(!store.read(handle, &mut []).expect("still unchanged").ended);
    }
}
