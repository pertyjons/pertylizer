#![cfg(feature = "simulated-ingress")]

mod common;

use synth_engine_v2::host::session::{
    PlaybackState, SessionCommandCapacity, SessionError, SessionLimits, SessionOutcome,
};
use synth_engine_v2::host::*;
use synth_engine_v2::ir::IrNodeKind;
use synth_engine_v2::quantities::{Amplitude, ChannelLayout, PreparedBytes};
use synth_engine_v2::render::AudioBlockMut;
use synth_engine_v2::schedule::AdmittedCompiledStream;
use synth_engine_v2::time::{FrameCount, PlanPosition, SampleTime};

fn setup_graph(graph: &synth_engine_v2::ir::GraphIr) -> (SimulatedHost, ConnectionGeneration) {
    let mut host = SimulatedHost::new();
    let endpoint = EndpointId::new("session-output".to_owned()).unwrap();
    let format = OutputFormat {
        rate: common::rate(48_000.0),
        layout: ChannelLayout::Mono,
    };
    let generation = host
        .begin(OutputRequest::new(
            EndpointSelection::Exact(endpoint.clone()),
            format,
        ))
        .unwrap();
    let backend = SimulatedBackend {
        endpoints: vec![SimulatedEndpoint {
            id: endpoint,
            display_name: "Session output".to_owned(),
            format,
            callback_bound: CallbackBound::Guaranteed(FrameCount::new(512)),
            open_succeeds: true,
        }],
        default_output: None,
    };
    host.prepare(generation, &backend, graph).unwrap();
    host.activate(generation).unwrap();
    (host, generation)
}

fn enable(
    host: &mut SimulatedHost,
    generation: ConnectionGeneration,
    stream: AdmittedCompiledStream,
) {
    host.enable_ordered_transport(
        generation,
        stream,
        SessionLimits {
            commands: SessionCommandCapacity::new(2).unwrap(),
            command_bytes: PreparedBytes::limit(8192).unwrap(),
        },
    )
    .unwrap();
    host.start(generation).unwrap();
}

fn setup() -> (SimulatedHost, ConnectionGeneration) {
    let graph = common::source_plan(IrNodeKind::Constant {
        level: Amplitude::new(0.25).unwrap(),
    });
    let (mut host, generation) = setup_graph(&graph);
    let stream = AdmittedCompiledStream::admit(host.last_valid_plan().unwrap(), &[]).unwrap();
    enable(&mut host, generation, stream);
    (host, generation)
}

fn render(
    host: &mut SimulatedHost,
    generation: ConnectionGeneration,
    frames: usize,
    partition: usize,
) -> Vec<f32> {
    let mut output = vec![9.0; frames];
    for chunk in output.chunks_mut(partition) {
        host.callback(
            generation,
            AudioBlockMut::new(chunk, chunk.len(), ChannelLayout::Mono).unwrap(),
        )
        .unwrap();
    }
    output
}

#[test]
fn play_stop_and_resume_follow_engine_boundaries_under_every_partition() {
    for partition in [512, 256, 64, 37, 1] {
        let (mut host, generation) = setup();
        let play = host
            .offer_session_play(generation, SampleTime::ZERO)
            .unwrap();
        let stop = host
            .offer_session_stop(generation, SampleTime::new(64))
            .unwrap();
        let output = render(&mut host, generation, 320, partition);
        assert_eq!(&output[..64], &[0.0; 64]);
        assert_eq!(&output[64..128], &[0.25; 64]);
        assert!(output[128..].iter().all(|sample| *sample == 0.0));
        assert_eq!(
            host.session_state(),
            Some(PlaybackState::Stopped(PlanPosition::new(64)))
        );
        assert_eq!(host.active().unwrap().clock, SampleTime::new(256));
        // Both commands took effect even though neither receipt was collected.
        let receipt = host.collect_session_receipt(generation).unwrap().unwrap();
        assert_eq!(receipt.boundary.id, play);
        assert_eq!(
            receipt.outcome,
            SessionOutcome::Applied {
                position: PlanPosition::ZERO
            }
        );
        assert_eq!(
            host.collect_session_receipt(generation)
                .unwrap()
                .unwrap()
                .boundary
                .id,
            stop
        );
        assert!(host.collect_session_receipt(generation).unwrap().is_none());

        let _play = host
            .offer_session_play(generation, SampleTime::new(256))
            .unwrap();
        let _stop = host
            .offer_session_stop(generation, SampleTime::new(320))
            .unwrap();
        let resumed = render(&mut host, generation, 256, partition);
        assert_eq!(&resumed[..64], &[0.25; 64]);
        assert!(resumed[64..].iter().all(|sample| *sample == 0.0));
        assert_eq!(
            host.session_state(),
            Some(PlaybackState::Stopped(PlanPosition::new(128)))
        );
        assert_eq!(host.active().unwrap().clock, SampleTime::new(512));
    }
}

#[test]
fn same_time_stop_cancels_play_before_offer_and_releases_it_on_collection() {
    let (mut host, generation) = setup();
    let play = host
        .offer_session_play(generation, SampleTime::ZERO)
        .unwrap();
    let _stop = host
        .offer_session_stop(generation, SampleTime::ZERO)
        .unwrap();
    assert_eq!(render(&mut host, generation, 128, 37), vec![0.0; 128]);
    let receipt = host.collect_session_receipt(generation).unwrap().unwrap();
    assert_eq!(receipt.boundary.id, play);
    assert_eq!(receipt.outcome, SessionOutcome::Cancelled);
    let _stop = host.collect_session_receipt(generation).unwrap().unwrap();
    let _play = host
        .offer_session_play(generation, SampleTime::new(64))
        .unwrap();
    assert_eq!(render(&mut host, generation, 64, 37), vec![0.25; 64]);
}

#[test]
fn pending_play_cannot_spend_the_stop_or_receipt_entitlement() {
    let (mut host, generation) = setup();
    let _play = host
        .offer_session_play(generation, SampleTime::ZERO)
        .unwrap();
    assert!(
        host.offer_session_play(generation, SampleTime::new(64))
            .is_err()
    );
    let _stop = host
        .offer_session_stop(generation, SampleTime::new(64))
        .unwrap();
    assert!(
        host.offer_session_stop(generation, SampleTime::new(128))
            .is_err()
    );
    let output = render(&mut host, generation, 512, 512);
    assert!(output[128..].iter().all(|sample| *sample == 0.0));
    assert!(
        host.offer_session_play(generation, SampleTime::new(448))
            .is_err()
    );
    let _play = host.collect_session_receipt(generation).unwrap().unwrap();
    let _stop = host.collect_session_receipt(generation).unwrap().unwrap();
    let _play = host
        .offer_session_play(generation, SampleTime::new(448))
        .unwrap();
}

#[test]
fn carry_only_and_empty_calls_do_not_apply_a_pending_play() {
    let (mut host, generation) = setup();
    let _play = host
        .offer_session_play(generation, SampleTime::ZERO)
        .unwrap();
    host.callback(
        generation,
        AudioBlockMut::new(&mut [], 0, ChannelLayout::Mono).unwrap(),
    )
    .unwrap();
    assert_eq!(render(&mut host, generation, 64, 64), vec![0.0; 64]);
    assert!(host.collect_session_receipt(generation).unwrap().is_none());
    assert_eq!(
        host.session_state(),
        Some(PlaybackState::Stopped(PlanPosition::ZERO))
    );
    assert_eq!(render(&mut host, generation, 1, 1), vec![0.25]);
    assert!(host.collect_session_receipt(generation).unwrap().is_some());
}

#[test]
fn nonaligned_and_past_requests_refuse_without_spending_command_identity() {
    let (mut host, generation) = setup();
    assert!(
        host.offer_session_play(generation, SampleTime::new(1))
            .is_err()
    );
    let _output = render(&mut host, generation, 128, 128);
    assert!(
        host.offer_session_play(generation, SampleTime::ZERO)
            .is_err()
    );
    let play = host
        .offer_session_play(generation, SampleTime::new(64))
        .unwrap();
    assert_eq!(play.serial(), 1);
    assert_eq!(play.generation(), generation);
}

#[test]
fn device_loss_without_a_callback_retains_cancellations_until_collection() {
    let (mut host, generation) = setup();
    let play = host
        .offer_session_play(generation, SampleTime::new(128))
        .unwrap();
    let stop = host
        .offer_session_stop(generation, SampleTime::new(192))
        .unwrap();
    host.device_lost(generation).unwrap();
    assert!(matches!(
        host.acknowledge_quiescence(generation),
        Err(HostError::Session(SessionError::RetainedOutcomes))
    ));
    assert_eq!(render(&mut host, generation, 128, 37), vec![0.0; 128]);
    for id in [play, stop] {
        let receipt = host.collect_session_receipt(generation).unwrap().unwrap();
        assert_eq!(receipt.boundary.id, id);
        assert_eq!(receipt.outcome, SessionOutcome::Cancelled);
    }
    host.acknowledge_quiescence(generation).unwrap();
    assert_eq!(host.active().unwrap().state, ConnectionState::Unavailable);
    assert!(host.last_valid_plan().is_some());
}

#[test]
fn an_oversized_whole_callback_cannot_adopt_before_its_refusal() {
    let (mut host, generation) = setup();
    let play = host
        .offer_session_play(generation, SampleTime::ZERO)
        .unwrap();
    let mut output = [1.0; 513];
    assert!(
        host.callback(
            generation,
            AudioBlockMut::new(&mut output, 513, ChannelLayout::Mono).unwrap()
        )
        .is_err()
    );
    assert_eq!(output, [0.0; 513]);
    assert_eq!(host.active().unwrap().clock, SampleTime::ZERO);
    let receipt = host.collect_session_receipt(generation).unwrap().unwrap();
    assert_eq!(receipt.boundary.id, play);
    assert_eq!(receipt.outcome, SessionOutcome::Cancelled);
    host.acknowledge_quiescence(generation).unwrap();
}

#[test]
fn an_applied_play_is_promoted_before_shutdown_without_erasing_its_receipt() {
    let (mut host, generation) = setup();
    let play = host
        .offer_session_play(generation, SampleTime::ZERO)
        .unwrap();
    let _output = render(&mut host, generation, 128, 37);
    host.shutdown(generation).unwrap();
    let receipt = host.collect_session_receipt(generation).unwrap().unwrap();
    assert_eq!(receipt.boundary.id, play);
    assert!(matches!(receipt.outcome, SessionOutcome::Applied { .. }));
    host.acknowledge_quiescence(generation).unwrap();
    assert_eq!(host.active().unwrap().state, ConnectionState::Stopped);
}

#[test]
fn legacy_stop_cannot_bypass_the_ordered_lane() {
    let (mut host, generation) = setup();
    assert!(matches!(
        host.stop(generation),
        Err(HostError::Session(SessionError::OrderedTransport))
    ));
    let _play = host
        .offer_session_play(generation, SampleTime::ZERO)
        .unwrap();
    assert_eq!(render(&mut host, generation, 128, 128)[64..], [0.25; 64]);
}

#[test]
fn resume_cuts_crossing_notes_and_keeps_later_note_edges_partition_invariant() {
    use synth_engine_v2::ir::{ExecutionScope, GraphIr, NodeId, PortId, SignalDomain};
    use synth_engine_v2::quantities::{NormalizedLevel, Seconds};
    use synth_engine_v2::schedule::{CompiledPayload, PlanEvent};
    const SOURCE: NodeId = NodeId::new(1);
    const OUTPUT: NodeId = NodeId::new(2);
    const ENVELOPE: NodeId = NodeId::new(11);
    const AMPLIFIER: NodeId = NodeId::new(12);
    let graph = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).expect("not negative"),
                decay: Seconds::new(0.0).expect("not negative"),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.0).expect("not negative"),
                velocity_sensitivity: synth_engine_v2::quantities::NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(common::compiled_notes(2))
        .build()
        .expect("a readable plan");
    let mut reference = None;
    for partition in [512, 256, 64, 37, 1] {
        let (mut host, generation) = setup_graph(&graph);
        let plan = host.last_valid_plan().unwrap();
        let slot = plan.resolve_note(ENVELOPE).unwrap();
        let events = [(0, true), (128, false), (192, true), (256, false)].map(|(position, on)| {
            PlanEvent::new(
                PlanPosition::new(position),
                if on {
                    common::note_on(slot)
                } else {
                    CompiledPayload::NoteOff {
                        slot,
                        key: common::any_key(),
                    }
                },
            )
        });
        let stream = AdmittedCompiledStream::admit(plan, &events).unwrap();
        enable(&mut host, generation, stream);
        let _play = host
            .offer_session_play(generation, SampleTime::ZERO)
            .unwrap();
        let _stop = host
            .offer_session_stop(generation, SampleTime::new(64))
            .unwrap();
        let first = render(&mut host, generation, 320, partition);
        assert!(first[64..128].iter().any(|sample| *sample > 0.0));
        assert!(first[128..].iter().all(|sample| *sample == 0.0));
        let _play_receipt = host.collect_session_receipt(generation).unwrap().unwrap();
        let _stop_receipt = host.collect_session_receipt(generation).unwrap().unwrap();
        let _resume = host
            .offer_session_play(generation, SampleTime::new(256))
            .unwrap();
        let resumed = render(&mut host, generation, 320, partition);
        // At frozen position 64 the first note crosses the new anchor: it is cut,
        // never retriggered. Position 192 starts the next note 128 frames later.
        assert!(resumed[..128].iter().all(|sample| *sample == 0.0));
        assert!(resumed[128..192].iter().any(|sample| *sample > 0.0));
        assert!(resumed[192..].iter().all(|sample| *sample == 0.0));
        if let Some(expected) = &reference {
            assert_eq!(&resumed, expected);
        } else {
            reference = Some(resumed);
        }
    }
}
