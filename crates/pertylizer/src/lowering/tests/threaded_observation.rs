//! Owning application observers on a real second thread, compared with unobserved sound.
use super::*;
use crate::lowering::{live::prepare_unqualified, observation::ObservationLaneError};
use synth_engine_v2::{
    compile::{RenderConfig, compile},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    quantities::{Amplitude, ChannelLayout, SampleRate},
};

fn profile(rate: f32) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(rate).unwrap(),
        FrameCount::new(256),
        ChannelLayout::Stereo,
    )
    .unwrap()
}

fn lowered(profile: HostProfile, extra: bool) -> LoweredProject {
    let mut graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Sine {
                frequency: synth_engine_v2::quantities::Frequency::new(317.0).unwrap(),
                amplitude: Amplitude::new(0.25).unwrap(),
            },
            ExecutionScope::Global,
        )
        .node(
            NodeId::new(2),
            IrNodeKind::StereoMonitor,
            ExecutionScope::Global,
        )
        .node(NodeId::new(3), IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(2), PortId::FIRST),
            SignalDomain::Audio,
        );
    if extra {
        graph = graph
            .node(
                NodeId::new(4),
                IrNodeKind::HardClamp,
                ExecutionScope::Global,
            )
            .connect(
                (NodeId::new(2), PortId::FIRST),
                (NodeId::new(4), PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (NodeId::new(4), PortId::FIRST),
                (NodeId::new(3), PortId::FIRST),
                SignalDomain::Audio,
            );
    } else {
        graph = graph.connect(
            (NodeId::new(2), PortId::FIRST),
            (NodeId::new(3), PortId::FIRST),
            SignalDomain::Audio,
        );
    }
    LoweredProject {
        plan: Some(
            compile(&graph.build().unwrap(), &RenderConfig::new(profile))
                .into_plan()
                .unwrap(),
        ),
        events: Vec::new(),
        diagnostics: Vec::new(),
        lowered_events: EventCount::NONE,
        lowered_frames: FrameCount::new(1_000_000),
        report: None,
    }
}

fn callback(audio: &mut SongAudio, frames: usize) -> Vec<f32> {
    let mut samples = vec![0.0; frames * 2];
    let mut rendered = false;
    let measured = allocation_counter::measure(|| {
        rendered =
            audio.render(AudioBlockMut::new(&mut samples, frames, ChannelLayout::Stereo).unwrap())
    });
    assert!(rendered);
    assert_eq!((measured.count_total, measured.count_current), (0, 0));
    samples
}

#[test]
fn threaded_observer_install_read_remove_backpressure_and_recreation_are_passive() {
    for rate in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let profile = profile(rate);
        let original = lowered(profile, false);
        let tap = original
            .plan
            .as_ref()
            .unwrap()
            .resolve_tap(NodeId::new(2), PortId::FIRST)
            .unwrap();
        let (mut control, mut audio) = prepare_unqualified(original, profile).unwrap();
        let (mut reference_control, mut reference) =
            prepare_unqualified(lowered(profile, false), profile).unwrap();
        let at = control.play().unwrap();
        assert_eq!(at, reference_control.play().unwrap());
        let (step, incoming) = std::sync::mpsc::sync_channel::<Option<usize>>(1);
        let (outgoing, result) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            while let Some(frames) = incoming.recv().unwrap() {
                outgoing.send(callback(&mut audio, frames)).unwrap();
            }
            audio
        });
        let advance = |frames, reference: &mut SongAudio| {
            step.send(Some(frames)).unwrap();
            assert_eq!(result.recv().unwrap(), callback(reference, frames));
        };
        let (_, handles) = control.subscribe_observations(&profile, &[tap]).unwrap();
        advance(256, &mut reference);
        assert!(control.take_observation_reply().unwrap().result.is_ok());
        for _ in 0..160 {
            advance(256, &mut reference);
        }
        let buffer =
            ObservationBuffer::prepare(ChannelLayout::Stereo, FrameCount::new(512), OWNER_BYTES)
                .unwrap();
        let _read = control.read_observations(handles[0], buffer).unwrap();
        advance(256, &mut reference);
        let reply = control.take_observation_reply().unwrap();
        let read = reply.result.unwrap().unwrap();
        assert_eq!(
            read.frames,
            FrameCount::new(256),
            "read work is capped to one admitted callback"
        );
        assert!(
            read.dropped.as_u64() > 0,
            "stalled subscriber overflow is explicit"
        );
        assert!(!read.ended);
        if let ObservationRequest::Read { buffer, .. } = reply.request {
            assert!(buffer.samples()[..512].iter().any(|value| *value != 0.0));
        } else {
            panic!("read owner returned");
        }
        // Fill both credit positions and their completed returns, without polling control.
        for _ in 0..2 {
            let buffer =
                ObservationBuffer::prepare(ChannelLayout::Stereo, FrameCount::new(64), OWNER_BYTES)
                    .unwrap();
            let _queued = control
                .observers
                .lane
                .submit(ObservationRequest::Read {
                    subscription: handles[0],
                    buffer,
                })
                .unwrap();
        }
        advance(256, &mut reference);
        advance(256, &mut reference);
        let buffer =
            ObservationBuffer::prepare(ChannelLayout::Stereo, FrameCount::new(64), OWNER_BYTES)
                .unwrap();
        assert!(matches!(
            control.observers.lane.submit(ObservationRequest::Read {
                subscription: handles[0],
                buffer
            }),
            Err((_, ObservationLaneError::Full))
        ));
        for _ in 0..8 {
            advance(256, &mut reference);
        }
        assert!(control.take_observation_reply().unwrap().result.is_ok());
        assert!(control.take_observation_reply().unwrap().result.is_ok());
        let _remove = control.remove_observations().unwrap();
        advance(256, &mut reference);
        let mut old = control.take_observation_reply().unwrap().retired.unwrap();
        let mut tail = [0.0; 128];
        assert!(old.read(handles[0], &mut tail).unwrap().ended);
        assert!(matches!(
            control.observers.lane.submit(ObservationRequest::Install {
                expected: None,
                incoming: Some(old)
            }),
            Err((_, ObservationLaneError::UsedStore))
        ));
        let (_, new_handles) = control.subscribe_observations(&profile, &[tap]).unwrap();
        assert_ne!(handles[0].store(), new_handles[0].store());
        advance(256, &mut reference);
        assert!(control.take_observation_reply().unwrap().result.is_ok());
        let buffer =
            ObservationBuffer::prepare(ChannelLayout::Stereo, FrameCount::new(64), OWNER_BYTES)
                .unwrap();
        let _read = control.read_observations(handles[0], buffer).unwrap();
        advance(256, &mut reference);
        assert_eq!(
            control.take_observation_reply().unwrap().result,
            Err(ObservationLaneError::ForeignStore)
        );
        // Shutdown with a queued owning request: no extra callback is required.
        let buffer =
            ObservationBuffer::prepare(ChannelLayout::Stereo, FrameCount::new(64), OWNER_BYTES)
                .unwrap();
        let cancelled = control.read_observations(new_handles[0], buffer).unwrap();
        step.send(None).unwrap();
        let audio = worker.join().unwrap();
        let retired = control
            .close_observations_after_quiescence(audio)
            .unwrap_or_else(|_| panic!("matching halves close"));
        assert_eq!(retired.cancelled.len(), 1);
        assert_eq!(retired.cancelled[0].0, cancelled);
        assert!(retired.active.is_some());
    }
}

#[test]
fn threaded_observers_follow_running_edits_and_refuse_subscription_changes_until_acknowledged() {
    let profile = profile(48_000.0);
    let original = lowered(profile, false);
    let tap = original
        .plan
        .as_ref()
        .unwrap()
        .resolve_tap(NodeId::new(2), PortId::FIRST)
        .unwrap();
    let (mut control, mut audio) = prepare_unqualified(original, profile).unwrap();
    let (mut reference_control, mut reference) =
        prepare_unqualified(lowered(profile, false), profile).unwrap();
    let at = control.play().unwrap();
    assert_eq!(at, reference_control.play().unwrap());
    let (step, incoming) = std::sync::mpsc::sync_channel::<Option<usize>>(1);
    let (outgoing, result) = std::sync::mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        while let Some(frames) = incoming.recv().unwrap() {
            outgoing.send(callback(&mut audio, frames)).unwrap();
        }
        audio
    });
    let advance = |reference: &mut SongAudio| {
        step.send(Some(256)).unwrap();
        assert_eq!(result.recv().unwrap(), callback(reference, 256));
    };
    let (_, handles) = control.subscribe_observations(&profile, &[tap]).unwrap();
    for _ in 0..8 {
        advance(&mut reference);
    }
    assert!(control.take_observation_reply().unwrap().result.is_ok());
    control.collect();
    let install = crate::lowering::app::InstallSequence::NONE.next().unwrap();
    let (new_plan, _) = control
        .replace_running_guarded(lowered(profile, true), profile, install, true, || true)
        .unwrap();
    assert!(control.remove_observations().is_err());
    assert!(control.subscribe_observations(&profile, &[tap]).is_err());
    for _ in 0..32 {
        advance(&mut reference);
    }
    control.collect();
    assert_eq!(control.plan_id(), new_plan);
    let buffer =
        ObservationBuffer::prepare(ChannelLayout::Stereo, FrameCount::new(64), OWNER_BYTES)
            .unwrap();
    let _read = control.read_observations(handles[0], buffer).unwrap();
    advance(&mut reference);
    let read = control
        .take_observation_reply()
        .unwrap()
        .result
        .unwrap()
        .unwrap();
    assert_eq!(read.plan, new_plan);
    assert_eq!(read.generation.as_u64(), 1);
    assert!(!read.ended);
    let _remove = control.remove_observations().unwrap();
    advance(&mut reference);
    assert!(control.take_observation_reply().unwrap().retired.is_some());
    step.send(None).unwrap();
    let audio = worker.join().unwrap();
    let retired = control
        .close_observations_after_quiescence(audio)
        .unwrap_or_else(|_| panic!("matching halves close"));
    assert!(retired.active.is_none());
    assert!(retired.cancelled.is_empty());
}

#[test]
fn a_terminal_render_fault_ends_the_store_and_preserves_its_read_owner() {
    let profile = profile(48_000.0);
    let original = lowered(profile, false);
    let tap = original
        .plan
        .as_ref()
        .unwrap()
        .resolve_tap(NodeId::new(2), PortId::FIRST)
        .unwrap();
    let (mut control, mut audio) = prepare_unqualified(original, profile).unwrap();
    let _at = control.play().unwrap();
    let (_, handles) = control.subscribe_observations(&profile, &[tap]).unwrap();
    callback(&mut audio, 256);
    assert!(control.take_observation_reply().unwrap().result.is_ok());
    let mut invalid = [3.0; 64];
    let measured = allocation_counter::measure(|| {
        assert!(!audio.render(AudioBlockMut::new(&mut invalid, 64, ChannelLayout::Mono).unwrap()));
    });
    assert_eq!((measured.count_total, measured.count_current), (0, 0));
    assert!(invalid.iter().all(|value| *value == 0.0));
    let buffer =
        ObservationBuffer::prepare(ChannelLayout::Stereo, FrameCount::new(64), OWNER_BYTES)
            .unwrap();
    let _read = control.read_observations(handles[0], buffer).unwrap();
    let mut output = [3.0; 128];
    assert!(!audio.render(AudioBlockMut::new(&mut output, 64, ChannelLayout::Stereo).unwrap()));
    let reply = control.take_observation_reply().unwrap();
    assert!(reply.result.unwrap().unwrap().ended);
    assert!(matches!(reply.request, ObservationRequest::Read { .. }));
    assert!(control.close_observations_after_quiescence(audio).is_ok());
}

#[test]
fn stopped_observation_handoff_and_running_lineage_break_follow_owning_receipts() {
    for running in [false, true] {
        let profile = profile(48_000.0);
        let original = lowered(profile, false);
        let tap = original
            .plan
            .as_ref()
            .unwrap()
            .resolve_tap(NodeId::new(2), PortId::FIRST)
            .unwrap();
        let (mut control, mut audio) = prepare_unqualified(original, profile).unwrap();
        let (mut reference_control, mut reference) =
            prepare_unqualified(lowered(profile, false), profile).unwrap();
        if running {
            assert_eq!(control.play().unwrap(), reference_control.play().unwrap());
        }
        let (step, incoming) = std::sync::mpsc::sync_channel::<Option<usize>>(1);
        let (outgoing, result) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            while let Some(frames) = incoming.recv().unwrap() {
                outgoing.send(callback(&mut audio, frames)).unwrap();
            }
            audio
        });
        let advance = |reference: &mut SongAudio| {
            step.send(Some(256)).unwrap();
            assert_eq!(result.recv().unwrap(), callback(reference, 256));
        };
        let (_, handles) = control.subscribe_observations(&profile, &[tap]).unwrap();
        for _ in 0..8 {
            advance(&mut reference);
        }
        assert!(control.take_observation_reply().unwrap().result.is_ok());
        control.collect();
        reference_control.collect();
        let install = crate::lowering::app::InstallSequence::NONE.next().unwrap();
        let successor = lowered(profile, true);
        let successor_tap = successor
            .plan
            .as_ref()
            .unwrap()
            .resolve_tap(NodeId::new(2), PortId::FIRST)
            .unwrap();
        let plan = if running {
            let (plan, _) = control
                .replace_running_guarded(successor, profile, install, false, || true)
                .unwrap();
            let _reference = reference_control
                .replace_running_guarded(lowered(profile, true), profile, install, false, || true)
                .unwrap();
            plan
        } else {
            let plan = control
                .replace_compatible(successor, profile, install)
                .unwrap();
            let _reference = reference_control
                .replace_compatible(lowered(profile, true), profile, install)
                .unwrap();
            plan
        };
        for _ in 0..32 {
            advance(&mut reference);
        }
        control.collect();
        reference_control.collect();
        assert_eq!(control.plan_id(), plan);
        let buffer =
            ObservationBuffer::prepare(ChannelLayout::Stereo, FrameCount::new(64), OWNER_BYTES)
                .unwrap();
        let _read = control.read_observations(handles[0], buffer).unwrap();
        advance(&mut reference);
        let read = control
            .take_observation_reply()
            .unwrap()
            .result
            .unwrap()
            .unwrap();
        if running {
            assert!(read.ended);
            assert!(control.observers.mirror.is_none());
            let (_, fresh) = control
                .subscribe_observations(&profile, &[successor_tap])
                .unwrap();
            assert_ne!(fresh[0].store(), handles[0].store());
            advance(&mut reference);
            assert!(control.take_observation_reply().unwrap().result.is_ok());
        } else {
            assert!(!read.ended);
            assert_eq!(read.plan, plan);
            assert_eq!(read.generation.as_u64(), 1);
        }
        step.send(None).unwrap();
        let audio = worker.join().unwrap();
        assert!(control.close_observations_after_quiescence(audio).is_ok());
    }
}
