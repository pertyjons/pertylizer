use super::*;
use crate::host::*;
use crate::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use crate::profile::{CaptureLimits, CaptureLimitsInput};
use crate::quantities::{
    Amplitude, CapturePassCount, CaptureResultCount, CaptureSourceCount, ChannelLayout, EventCount,
    HeldNoteCount, PreparedBytes, ProjectionTickCount, SampleRate, TrackedInputNoteCount,
};
use crate::recording::notes::{
    CaptureMode, CaptureQuantization, FixtureRevision, FixtureTargetId, MusicalInterval,
    NoteArmInput,
};
use crate::recording::{CaptureError, CaptureOutcome};
use crate::render::AudioBlockMut;
use crate::tempo::{Bpm, MusicalTick, TempoMap};
use crate::time::{FrameCount, PlanPosition, StreamAnchor, issue_epoch};

fn limits(events: u32, results: u32) -> RecordingLimits {
    RecordingLimits::new(
        HeldNoteCount::limit(2).unwrap(),
        EventCount::limit(events).unwrap(),
    )
    .unwrap()
    .with_capture(
        CaptureLimits::new(CaptureLimitsInput {
            max_tracked_input_notes: TrackedInputNoteCount::limit(8).unwrap(),
            max_capture_sources: CaptureSourceCount::limit(2).unwrap(),
            max_capture_passes: CapturePassCount::limit(1).unwrap(),
            max_pending_capture_results: CaptureResultCount::limit(results).unwrap(),
            max_capture_bytes: PreparedBytes::limit(1_048_576).unwrap(),
            max_audio_capture_frames: FrameCount::new(1024),
            max_projection_ticks: ProjectionTickCount::limit(961).unwrap(),
            capture_lateness_allowance: FrameCount::ZERO,
        })
        .unwrap(),
    )
    .unwrap()
}
fn prepare(host: &mut SimulatedHost) -> ConnectionGeneration {
    let format = OutputFormat {
        rate: SampleRate::new(48_000.0).unwrap(),
        layout: ChannelLayout::Mono,
    };
    let endpoint = EndpointId::new("output".to_owned()).unwrap();
    let generation = host
        .begin(OutputRequest::new(
            EndpointSelection::Exact(endpoint.clone()),
            format,
        ))
        .unwrap();
    let backend = SimulatedBackend {
        endpoints: vec![SimulatedEndpoint {
            id: endpoint,
            display_name: "Output".to_owned(),
            format,
            callback_bound: CallbackBound::Guaranteed(FrameCount::new(256)),
            open_succeeds: true,
        }],
        default_output: None,
    };
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Constant {
                level: Amplitude::new(0.25).unwrap(),
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
    host.prepare(generation, &backend, &graph).unwrap();
    generation
}
fn setup(
    events: u32,
    results: u32,
) -> (
    SimulatedHost,
    ConnectionGeneration,
    [ConnectionGeneration; 2],
    StreamEpoch,
) {
    let mut host = SimulatedHost::new();
    let generation = prepare(&mut host);
    host.activate(generation).unwrap();
    host.prepare_note_capture(generation, limits(events, results))
        .unwrap();
    let epoch = host.active().unwrap().identity.unwrap().epoch;
    let mut control = host.note_capture_control(generation).unwrap();
    let sources = [
        control.bind_source(ControllerSnapshot::neutral()).unwrap(),
        control.bind_source(ControllerSnapshot::neutral()).unwrap(),
    ];
    (host, generation, sources, epoch)
}
fn context(epoch: StreamEpoch, start: u64, end: u64) -> NoteArmContext {
    NoteArmContext::prepare(NoteArmInput {
        target: FixtureTargetId::new(11).unwrap(),
        expected_revision: FixtureRevision::new(7),
        interval: MusicalInterval::new(MusicalTick::new(start), MusicalTick::new(end)).unwrap(),
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
    .unwrap()
}
fn fence(
    host: &mut SimulatedHost,
    generation: ConnectionGeneration,
    source: ConnectionGeneration,
    epoch: StreamEpoch,
    at: u64,
) {
    let sequence = host
        .note_capture()
        .unwrap()
        .source_sequence(source)
        .unwrap();
    host.note_capture_control(generation)
        .unwrap()
        .fence(source, epoch, SampleTime::new(at), sequence)
        .unwrap();
}
fn publish(
    host: &mut SimulatedHost,
    generation: ConnectionGeneration,
    source: ConnectionGeneration,
    epoch: StreamEpoch,
    at: u64,
    bytes: [u8; 3],
) {
    let _receipt = host
        .note_capture_control(generation)
        .unwrap()
        .publish(
            source,
            CaptureStamp::exact_fixture(epoch, SampleTime::new(at), SampleTime::new(at)).unwrap(),
            Midi1Input::from_bytes(bytes).unwrap(),
            AuditionTrace::NotOffered,
        )
        .unwrap();
}
fn arm_start(
    host: &mut SimulatedHost,
    generation: ConnectionGeneration,
    sources: &[ConnectionGeneration],
    epoch: StreamEpoch,
    start: u64,
    end: u64,
) -> TakeReservation {
    let context = context(epoch, start, end);
    let at = context.window().start().as_u64();
    let ticket = host
        .note_capture_control(generation)
        .unwrap()
        .arm(context, sources)
        .unwrap();
    for source in sources {
        fence(host, generation, *source, epoch, at);
    }
    host.note_capture_control(generation)
        .unwrap()
        .start(ticket)
        .unwrap();
    ticket
}
fn acknowledge_all(
    host: &mut SimulatedHost,
    generation: ConnectionGeneration,
    sources: &[ConnectionGeneration],
) {
    for source in sources {
        host.acknowledge_note_source_quiescence(generation, *source)
            .unwrap();
    }
    host.acknowledge_quiescence(generation).unwrap();
}

#[test]
fn loss_uses_minimum_capture_frontier_with_or_without_a_final_callback() {
    for final_callback in [false, true] {
        for reverse_ack in [false, true] {
            let (mut host, generation, sources, epoch) = setup(16, 2);
            let ticket = arm_start(&mut host, generation, &sources, epoch, 1, 5);
            host.start(generation).unwrap();
            publish(
                &mut host,
                generation,
                sources[0],
                epoch,
                30,
                [0x90, 60, 100],
            );
            publish(
                &mut host,
                generation,
                sources[1],
                epoch,
                40,
                [0x90, 64, 100],
            );
            fence(&mut host, generation, sources[0], epoch, 60);
            fence(&mut host, generation, sources[1], epoch, 80);
            // This accepted record remains raw, even though the frozen selection ends at 60.
            publish(&mut host, generation, sources[1], epoch, 90, [0x80, 64, 0]);
            let bytes = host.note_capture().unwrap().bytes_reserved();
            host.device_lost(generation).unwrap();
            host.device_lost(generation).unwrap();
            assert!(matches!(
                host.note_capture_control(generation),
                Err(HostError::WrongState)
            ));
            assert!(matches!(
                host.acknowledge_quiescence(generation),
                Err(HostError::AwaitingCaptureQuiescence)
            ));
            if final_callback {
                let mut samples = [9.0; 37];
                host.callback(
                    generation,
                    AudioBlockMut::new(&mut samples, 37, ChannelLayout::Mono).unwrap(),
                )
                .unwrap();
                assert_eq!(samples, [0.0; 37]);
            }
            let order = if reverse_ack {
                [sources[1], sources[0]]
            } else {
                sources
            };
            host.acknowledge_note_source_quiescence(generation, order[0])
                .unwrap();
            assert!(host.note_capture().unwrap().result(ticket).is_err());
            assert!(host.active().unwrap().identity.is_some());
            host.acknowledge_note_source_quiescence(generation, order[0])
                .unwrap();
            host.acknowledge_note_source_quiescence(generation, order[1])
                .unwrap();
            let result = host.note_capture().unwrap().result(ticket).unwrap();
            assert_eq!(result.window().start(), SampleTime::new(25));
            assert_eq!(result.window().end(), SampleTime::new(60));
            assert_eq!(result.effective_outcome(), CaptureOutcome::Interrupted);
            assert_eq!(result.records().count(), 3);
            assert_eq!(result.selected_records().count(), 2);
            assert_eq!(result.closures().count(), 2);
            assert!(
                result
                    .closures()
                    .all(|closure| closure.time == SampleTime::new(60)
                        && closure.reason == CaptureStopReason::DeviceLost)
            );
            assert_eq!(host.note_capture().unwrap().bytes_reserved(), bytes);
            host.acknowledge_quiescence(generation).unwrap();
            assert!(host.active().unwrap().identity.is_none());
            assert!(host.last_valid_plan().is_some());
            assert_eq!(
                host.note_capture()
                    .unwrap()
                    .result(ticket)
                    .unwrap()
                    .records()
                    .count(),
                3
            );
        }
    }
}

#[test]
fn reconnection_cannot_replace_retained_results_or_resume_capture() {
    let (mut host, old, sources, epoch) = setup(16, 1);
    let ticket = arm_start(&mut host, old, &sources, epoch, 1, 5);
    publish(&mut host, old, sources[0], epoch, 30, [0x90, 60, 100]);
    for source in sources {
        fence(&mut host, old, source, epoch, 60);
    }
    host.device_lost(old).unwrap();
    assert!(matches!(
        host.release_note_capture(old),
        Err(HostError::CaptureRetained)
    ));
    acknowledge_all(&mut host, old, &sources);
    let new = prepare(&mut host);
    host.activate(new).unwrap();
    assert_eq!(host.active().unwrap().state, ConnectionState::Ready);
    assert_ne!(host.active().unwrap().identity.unwrap().epoch, epoch);
    assert!(matches!(
        host.note_capture_control(old),
        Err(HostError::StaleGeneration)
    ));
    assert!(matches!(
        host.note_capture_control(new),
        Err(HostError::NoNoteCapture)
    ));
    assert!(matches!(
        host.prepare_note_capture(new, limits(16, 1)),
        Err(HostError::CaptureRetained)
    ));
    assert!(matches!(
        host.acknowledge_note_source_quiescence(old, sources[0]),
        Err(HostError::StaleGeneration)
    ));
    assert!(matches!(
        host.device_lost(old),
        Err(HostError::StaleGeneration)
    ));
    let mut samples = [1.0; 32];
    host.callback(
        new,
        AudioBlockMut::new(&mut samples, 32, ChannelLayout::Mono).unwrap(),
    )
    .unwrap();
    assert_eq!(samples, [0.0; 32]);
    let quality = host
        .note_capture()
        .unwrap()
        .result(ticket)
        .unwrap()
        .quality();
    host.discard_note_capture(old, ticket, quality).unwrap();
    host.release_note_capture(old).unwrap();
    host.prepare_note_capture(new, limits(16, 1)).unwrap();
    assert!(!host.note_capture().unwrap().is_active());
    let fresh = host
        .note_capture_control(new)
        .unwrap()
        .bind_source(ControllerSnapshot::neutral())
        .unwrap();
    assert!(!sources.contains(&fresh));
    assert!(matches!(
        host.note_capture_control(new)
            .unwrap()
            .arm(context(epoch, 1, 5), &[fresh]),
        Err(NoteCaptureError::ForeignEpoch)
    ));
}

#[test]
fn source_loss_names_the_failure_and_unselected_sources_also_require_fences() {
    let (mut host, generation, sources, epoch) = setup(16, 1);
    let ticket = arm_start(&mut host, generation, &sources[..1], epoch, 1, 5);
    publish(
        &mut host,
        generation,
        sources[0],
        epoch,
        30,
        [0x90, 60, 100],
    );
    fence(&mut host, generation, sources[0], epoch, 75);
    host.note_source_lost(generation, sources[1]).unwrap();
    assert_eq!(
        host.active().unwrap().failure,
        Some(HostFailure::CaptureSourceLost(sources[1]))
    );
    host.acknowledge_note_source_quiescence(generation, sources[0])
        .unwrap();
    assert_eq!(
        host.note_capture()
            .unwrap()
            .result(ticket)
            .unwrap()
            .window()
            .end(),
        SampleTime::new(75)
    );
    assert!(matches!(
        host.acknowledge_quiescence(generation),
        Err(HostError::AwaitingCaptureQuiescence)
    ));
    host.acknowledge_note_source_quiescence(generation, sources[1])
        .unwrap();
    host.acknowledge_quiescence(generation).unwrap();
}

#[test]
fn count_in_loss_without_a_source_frontier_retains_an_empty_interrupted_take() {
    let (mut host, generation, sources, epoch) = setup(16, 1);
    let ticket = host
        .note_capture_control(generation)
        .unwrap()
        .arm(context(epoch, 4, 8), &sources)
        .unwrap();
    publish(
        &mut host,
        generation,
        sources[0],
        epoch,
        10,
        [0x90, 60, 100],
    );
    host.device_lost(generation).unwrap();
    acknowledge_all(&mut host, generation, &sources);
    let result = host.note_capture().unwrap().result(ticket).unwrap();
    assert_eq!(result.window().start(), SampleTime::ZERO);
    assert_eq!(result.window().end(), SampleTime::ZERO);
    assert_eq!(result.context().window().start(), SampleTime::new(100));
    assert_eq!(result.effective_outcome(), CaptureOutcome::Interrupted);
    assert_eq!(result.records().count(), 0);
}

#[test]
fn candidate_loss_and_foreign_source_refusals_do_not_interrupt_active_capture() {
    let (mut host, generation, sources, epoch) = setup(16, 1);
    let ticket = arm_start(&mut host, generation, &sources, epoch, 1, 5);
    let candidate = prepare(&mut host);
    host.device_lost(candidate).unwrap();
    host.acknowledge_quiescence(candidate).unwrap();
    assert!(host.note_capture().unwrap().is_active());
    let (_, _, foreign, _) = setup(16, 1);
    assert!(matches!(
        host.note_source_lost(generation, foreign[0]),
        Err(HostError::NoteCapture(NoteCaptureError::ForeignSource))
    ));
    assert!(host.active().unwrap().failure.is_none());
    assert!(matches!(
        host.stop(generation),
        Err(HostError::CaptureActive)
    ));
    for source in sources {
        fence(&mut host, generation, source, epoch, 125);
    }
    assert_eq!(
        host.note_capture()
            .unwrap()
            .result(ticket)
            .unwrap()
            .effective_outcome(),
        CaptureOutcome::Complete
    );
    host.stop(generation).unwrap();
}

#[test]
fn terminal_render_fault_closes_admission_before_off_thread_capture_finalization() {
    let (mut host, generation, sources, epoch) = setup(16, 1);
    let ticket = arm_start(&mut host, generation, &sources, epoch, 1, 5);
    publish(
        &mut host,
        generation,
        sources[0],
        epoch,
        30,
        [0x90, 60, 100],
    );
    for source in sources {
        fence(&mut host, generation, source, epoch, 75);
    }
    let mut samples = [9.0; 257];
    assert!(matches!(
        host.callback(
            generation,
            AudioBlockMut::new(&mut samples, 257, ChannelLayout::Mono).unwrap()
        ),
        Err(CallbackError::Render(_))
    ));
    assert_eq!(samples, [0.0; 257]);
    assert!(host.active().unwrap().needs_reprepare);
    assert!(matches!(
        host.note_capture_control(generation),
        Err(HostError::WrongState)
    ));
    acknowledge_all(&mut host, generation, &sources);
    let result = host.note_capture().unwrap().result(ticket).unwrap();
    assert_eq!(result.effective_outcome(), CaptureOutcome::Interrupted);
    assert!(
        result
            .closures()
            .all(|closure| closure.reason == CaptureStopReason::DeviceReprepare)
    );
}

#[test]
fn notification_stall_and_full_ordinary_storage_preserve_every_retained_result() {
    let (mut host, generation, sources, epoch) = setup(1, 2);
    let first = arm_start(&mut host, generation, &sources, epoch, 1, 5);
    for source in sources {
        fence(&mut host, generation, source, epoch, 125);
    }
    let second = arm_start(&mut host, generation, &sources, epoch, 6, 10);
    publish(
        &mut host,
        generation,
        sources[0],
        epoch,
        160,
        [0x90, 60, 100],
    );
    publish(&mut host, generation, sources[0], epoch, 170, [0x80, 60, 0]);
    for source in sources {
        fence(&mut host, generation, source, epoch, 200);
    }
    host.device_lost(generation).unwrap();
    acknowledge_all(&mut host, generation, &sources);
    let capture = host.note_capture().unwrap();
    assert_eq!(capture.retained_results().count(), 2);
    assert_eq!(
        capture.result(first).unwrap().effective_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(capture.result(second).unwrap().records().count(), 1);
    assert_ne!(
        capture.result(second).unwrap().effective_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(capture.notification_misses().as_u64(), 1);
    assert!(matches!(
        host.release_note_capture(generation),
        Err(HostError::CaptureRetained)
    ));
}

#[test]
fn empty_owner_and_voluntary_shutdown_wait_for_both_fences_before_release() {
    let (mut host, generation, sources, _) = setup(16, 1);
    host.shutdown(generation).unwrap();
    for source in sources {
        host.acknowledge_note_source_quiescence(generation, source)
            .unwrap();
    }
    assert!(matches!(
        host.release_note_capture(generation),
        Err(HostError::AwaitingQuiescence)
    ));
    host.acknowledge_quiescence(generation).unwrap();
    host.release_note_capture(generation).unwrap();
    let new = prepare(&mut host);
    host.activate(new).unwrap();
    host.prepare_note_capture(new, limits(16, 1)).unwrap();
    host.device_lost(new).unwrap();
    host.acknowledge_quiescence(new).unwrap(); // No source exists to acknowledge separately.
    host.release_note_capture(new).unwrap();
}

#[test]
fn publication_loss_silence_and_source_finalization_do_not_allocate_or_drop_storage() {
    let (mut host, generation, sources, epoch) = setup(16, 1);
    let ticket = arm_start(&mut host, generation, &sources, epoch, 1, 5);
    let mut samples = [9.0; 64];
    let events = crate::render_allocation::count_allocs(|| {
        publish(
            &mut host,
            generation,
            sources[0],
            epoch,
            30,
            [0x90, 60, 100],
        );
        for source in sources {
            fence(&mut host, generation, source, epoch, 75);
        }
        host.device_lost(generation).unwrap();
        host.callback(
            generation,
            AudioBlockMut::new(&mut samples, 64, ChannelLayout::Mono).unwrap(),
        )
        .unwrap();
        for source in sources {
            host.acknowledge_note_source_quiescence(generation, source)
                .unwrap();
        }
    });
    assert_eq!(events, 0);
    assert_eq!(samples, [0.0; 64]);
    assert!(host.active().unwrap().identity.is_some());
    assert_eq!(
        host.note_capture()
            .unwrap()
            .result(ticket)
            .unwrap()
            .records()
            .count(),
        1
    );
}

#[test]
fn capture_preparation_is_transactional_and_rejects_foreign_context() {
    let mut host = SimulatedHost::new();
    let generation = prepare(&mut host);
    assert!(matches!(
        host.prepare_note_capture(generation, limits(16, 1)),
        Err(HostError::StaleGeneration)
    ));
    host.activate(generation).unwrap();
    host.start(generation).unwrap();
    assert!(matches!(
        host.prepare_note_capture(generation, limits(16, 1)),
        Err(HostError::WrongState)
    ));
    host.stop(generation).unwrap();
    let missing = RecordingLimits::new(
        HeldNoteCount::limit(2).unwrap(),
        EventCount::limit(16).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        host.prepare_note_capture(generation, missing),
        Err(HostError::NoteCapture(NoteCaptureError::Storage(
            CaptureError::MissingConfiguration
        )))
    ));
    assert!(host.note_capture().is_none());
    host.prepare_note_capture(generation, limits(16, 1))
        .unwrap();
    let source = host
        .note_capture_control(generation)
        .unwrap()
        .bind_source(ControllerSnapshot::neutral())
        .unwrap();
    assert!(matches!(
        host.note_capture_control(generation)
            .unwrap()
            .arm(context(issue_epoch().unwrap(), 1, 5), &[source]),
        Err(NoteCaptureError::ForeignEpoch)
    ));
    assert!(!host.note_capture().unwrap().is_active());
}

#[test]
fn host_arm_rejects_foreign_rate_and_anchor_without_consuming_a_result_slot() {
    let (mut host, generation, sources, epoch) = setup(16, 1);
    for (rate, anchor, expected) in [
        (
            44_100.0,
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
            NoteCaptureError::HostSampleRate,
        ),
        (
            48_000.0,
            StreamAnchor::new(SampleTime::new(1), PlanPosition::ZERO),
            NoteCaptureError::HostAnchor,
        ),
    ] {
        let context = NoteArmContext::prepare(NoteArmInput {
            target: FixtureTargetId::new(11).unwrap(),
            expected_revision: FixtureRevision::new(7),
            interval: MusicalInterval::new(MusicalTick::new(1), MusicalTick::new(5)).unwrap(),
            mode: CaptureMode::Overdub,
            quantization: CaptureQuantization::Off,
            epoch,
            anchor,
            tempo: TempoMap::new(
                Bpm::new(120.0).unwrap(),
                &[],
                SampleRate::new(rate).unwrap(),
            )
            .unwrap(),
        })
        .unwrap();
        assert_eq!(
            host.note_capture_control(generation)
                .unwrap()
                .arm(context, &sources),
            Err(expected)
        );
        assert!(!host.note_capture().unwrap().is_active());
    }
    let _ticket = arm_start(&mut host, generation, &sources, epoch, 1, 5);
}

#[test]
fn natural_end_is_not_sealed_until_every_source_covers_it_before_loss() {
    for caught_up_before_loss in [false, true] {
        for voluntary_shutdown in [false, true] {
            for reverse_ack in [false, true] {
                let (mut host, generation, sources, epoch) = setup(16, 1);
                let ticket = arm_start(&mut host, generation, &sources, epoch, 1, 5);
                publish(
                    &mut host,
                    generation,
                    sources[0],
                    epoch,
                    30,
                    [0x90, 60, 100],
                );
                fence(&mut host, generation, sources[0], epoch, 125);
                // One producer requests the natural end, but cannot seal the other's input.
                assert!(host.note_capture().unwrap().is_active());
                assert!(host.note_capture().unwrap().result(ticket).is_err());
                if caught_up_before_loss {
                    fence(&mut host, generation, sources[1], epoch, 125);
                    assert!(!host.note_capture().unwrap().is_active());
                }
                if voluntary_shutdown {
                    host.shutdown(generation).unwrap();
                } else {
                    host.device_lost(generation).unwrap();
                }
                let order = if reverse_ack {
                    [sources[1], sources[0]]
                } else {
                    sources
                };
                acknowledge_all(&mut host, generation, &order);
                let result = host.note_capture().unwrap().result(ticket).unwrap();
                assert_eq!(result.window().start(), SampleTime::new(25));
                assert_eq!(
                    result.window().end(),
                    SampleTime::new(if caught_up_before_loss { 125 } else { 25 })
                );
                assert_eq!(
                    result.effective_outcome(),
                    if caught_up_before_loss {
                        CaptureOutcome::Complete
                    } else {
                        CaptureOutcome::Interrupted
                    }
                );
                let records: Vec<_> = result.records().collect();
                assert_eq!(records.len(), 1);
                assert_eq!(records[0].stamp().nominal(), SampleTime::new(30));
                assert_eq!(records[0].source(), sources[0]);
                assert_eq!(
                    result.selected_records().count(),
                    usize::from(caught_up_before_loss)
                );
            }
        }
    }
}
