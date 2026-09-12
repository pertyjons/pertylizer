use super::*;
use crate::host::capture::tests::{context, limits, prepare_with_bound};
use crate::host::session::{
    SessionCaptureOutcome, SessionCommandCapacity, SessionLimits, SessionOutcome,
    SessionSourceCapacity, SessionSourceOutcome,
};
use crate::quantities::ChannelLayout;
use crate::recording::notes::{AuditionTrace, CaptureStamp, Midi1Input, NoteCaptureError};
use crate::render::AudioBlockMut;
use crate::schedule::AdmittedCompiledStream;
use crate::time::StreamEpoch;

fn setup() -> (
    SimulatedHost,
    ConnectionGeneration,
    [ConnectionGeneration; 2],
    StreamEpoch,
) {
    let mut host = SimulatedHost::new();
    let generation = prepare_with_bound(&mut host, crate::time::FrameCount::new(512));
    host.activate(generation).unwrap();
    let stream = AdmittedCompiledStream::admit(host.last_valid_plan().unwrap(), &[]).unwrap();
    host.enable_ordered_transport(
        generation,
        stream,
        SessionLimits {
            commands: SessionCommandCapacity::new(4).unwrap(),
            command_bytes: PreparedBytes::limit(16384).unwrap(),
        },
    )
    .unwrap();
    host.prepare_ordered_capture(
        generation,
        limits(32, 2),
        SessionSourceLimits {
            actions: SessionSourceCapacity::new(16).unwrap(),
            bytes: PreparedBytes::limit(32768).unwrap(),
        },
    )
    .unwrap();
    let epoch = host.active().unwrap().identity.unwrap().epoch;
    let mut capture = host.session_capture_control(generation).unwrap();
    let sources = [
        capture.bind_source(ControllerSnapshot::neutral()).unwrap(),
        capture.bind_source(ControllerSnapshot::neutral()).unwrap(),
    ];
    (host, generation, sources, epoch)
}
fn fence(
    host: &mut SimulatedHost,
    generation: ConnectionGeneration,
    source: ConnectionGeneration,
    epoch: StreamEpoch,
    at: u64,
) {
    let _id = host
        .offer_session_source(
            generation,
            SessionSourceAction::Fence {
                source,
                epoch,
                frontier: SampleTime::new(at),
            },
        )
        .unwrap();
}
fn input(
    host: &mut SimulatedHost,
    generation: ConnectionGeneration,
    source: ConnectionGeneration,
    epoch: StreamEpoch,
    at: u64,
    bytes: [u8; 3],
) {
    let _id = host
        .offer_session_source(
            generation,
            SessionSourceAction::Publish {
                source,
                stamp: CaptureStamp::exact_fixture(epoch, SampleTime::new(at), SampleTime::new(at))
                    .unwrap(),
                input: Midi1Input::from_bytes(bytes).unwrap(),
                audition: AuditionTrace::NotOffered,
            },
        )
        .unwrap();
}
fn render(
    host: &mut SimulatedHost,
    generation: ConnectionGeneration,
    output: &mut [f32],
    partition: usize,
) {
    for chunk in output.chunks_mut(partition) {
        let frames = chunk.len();
        host.callback(
            generation,
            AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn audio_raw_take_and_receipts_are_partition_invariant_without_callback_allocations() {
    let mut reference = None;
    for partition in [512, 256, 64, 37, 1] {
        let (mut host, generation, sources, epoch) = setup();
        let ticket = host
            .session_capture_control(generation)
            .unwrap()
            .arm(context(epoch, 0, 960), &sources)
            .unwrap();
        let _play = host
            .offer_session_recording_play(generation, SampleTime::ZERO, ticket)
            .unwrap();
        let _stop = host
            .offer_session_stop(generation, SampleTime::new(128))
            .unwrap();
        for source in sources {
            fence(&mut host, generation, source, epoch, 0);
        }
        input(&mut host, generation, sources[0], epoch, 0, [0x90, 60, 100]);
        input(&mut host, generation, sources[1], epoch, 32, [0x90, 60, 80]);
        input(&mut host, generation, sources[0], epoch, 96, [0x80, 60, 0]);
        for source in sources {
            fence(&mut host, generation, source, epoch, 128);
        }
        input(&mut host, generation, sources[1], epoch, 128, [0x80, 60, 0]);
        host.start(generation).unwrap();
        let mut output = [9.0; 320];
        assert_eq!(
            crate::render_allocation::count_allocs(|| render(
                &mut host,
                generation,
                &mut output,
                partition
            )),
            0
        );
        assert_eq!(&output[..64], &[0.0; 64]);
        assert_eq!(&output[64..192], &[0.25; 128]);
        assert!(output[192..].iter().all(|sample| *sample == 0.0));
        let play = host.collect_session_receipt(generation).unwrap().unwrap();
        assert_eq!(play.capture, Some(SessionCaptureOutcome::Applied));
        let stop = host.collect_session_receipt(generation).unwrap().unwrap();
        assert_eq!(stop.capture, Some(SessionCaptureOutcome::Applied));
        let mut receipts = 0;
        while let Some(receipt) = host.collect_session_source_receipt(generation).unwrap() {
            assert!(!matches!(
                receipt.outcome,
                SessionSourceOutcome::Refused(_) | SessionSourceOutcome::Cancelled
            ));
            receipts += 1;
        }
        assert_eq!(receipts, 8);
        let result = host.note_capture().unwrap().result(ticket).unwrap();
        assert_eq!(result.window().end(), SampleTime::new(128));
        let records: Vec<_> = result
            .selected_records()
            .map(|record| {
                (
                    sources
                        .iter()
                        .position(|source| *source == record.source())
                        .unwrap(),
                    record.sequence().as_u64(),
                    record.stamp().nominal(),
                    record.stamp().published_at(),
                    record.input(),
                    record.occurrence().map(|id| {
                        (
                            sources
                                .iter()
                                .position(|source| *source == id.source())
                                .unwrap(),
                            id.serial(),
                        )
                    }),
                    record.audition(),
                )
            })
            .collect();
        assert_eq!(records.len(), 3);
        if let Some(expected) = &reference {
            assert_eq!(&records, expected);
        } else {
            reference = Some(records);
        }
    }
}

#[test]
fn absent_start_fence_refuses_coupled_play_without_faulting_output() {
    let (mut host, generation, sources, epoch) = setup();
    let ticket = host
        .session_capture_control(generation)
        .unwrap()
        .arm(context(epoch, 0, 960), &sources)
        .unwrap();
    let _play = host
        .offer_session_recording_play(generation, SampleTime::ZERO, ticket)
        .unwrap();
    fence(&mut host, generation, sources[0], epoch, 0);
    host.start(generation).unwrap();
    let mut output = [9.0; 128];
    render(&mut host, generation, &mut output, 37);
    assert_eq!(output, [0.0; 128]);
    assert_eq!(host.active().unwrap().state, ConnectionState::Running);
    let receipt = host.collect_session_receipt(generation).unwrap().unwrap();
    assert_eq!(receipt.outcome, SessionOutcome::CaptureRefused);
    assert_eq!(
        receipt.capture,
        Some(SessionCaptureOutcome::Refused(NoteCaptureError::StartFence))
    );
}

#[test]
fn same_time_stop_cancels_start_before_fences_and_retains_both_outcomes() {
    let (mut host, generation, sources, epoch) = setup();
    let ticket = host
        .session_capture_control(generation)
        .unwrap()
        .arm(context(epoch, 0, 960), &sources)
        .unwrap();
    let _play = host
        .offer_session_recording_play(generation, SampleTime::ZERO, ticket)
        .unwrap();
    let _stop = host
        .offer_session_stop(generation, SampleTime::ZERO)
        .unwrap();
    for source in sources {
        fence(&mut host, generation, source, epoch, 0);
    }
    input(&mut host, generation, sources[0], epoch, 0, [0x90, 60, 100]);
    host.start(generation).unwrap();
    let mut output = [9.0; 128];
    render(&mut host, generation, &mut output, 37);
    assert_eq!(output, [0.0; 128]);
    assert_eq!(
        host.collect_session_receipt(generation)
            .unwrap()
            .unwrap()
            .capture,
        Some(SessionCaptureOutcome::Cancelled)
    );
    assert_eq!(
        host.collect_session_receipt(generation)
            .unwrap()
            .unwrap()
            .capture,
        Some(SessionCaptureOutcome::Applied)
    );
    let result = host.note_capture().unwrap().result(ticket).unwrap();
    assert_eq!(result.selected_records().count(), 0);
    assert_eq!(result.window().start(), result.window().end());
}

#[test]
fn a_fence_after_same_time_input_is_refused_before_spending_its_identity() {
    let (mut host, generation, sources, epoch) = setup();
    input(&mut host, generation, sources[0], epoch, 0, [0x90, 60, 100]);
    assert!(matches!(
        host.offer_session_source(
            generation,
            SessionSourceAction::Fence {
                source: sources[0],
                epoch,
                frontier: SampleTime::ZERO
            }
        ),
        Err(HostError::Session(SessionError::SourceOrder))
    ));
    let id = host
        .offer_session_source(
            generation,
            SessionSourceAction::Fence {
                source: sources[0],
                epoch,
                frontier: SampleTime::new(64),
            },
        )
        .unwrap();
    assert_eq!(id.serial(), 2);
    assert!(matches!(
        host.note_capture_control(generation),
        Err(HostError::Session(SessionError::OrderedTransport))
    ));
}

#[test]
fn audible_stop_applies_while_capture_waits_for_its_second_source() {
    let (mut host, generation, sources, epoch) = setup();
    let ticket = host
        .session_capture_control(generation)
        .unwrap()
        .arm(context(epoch, 0, 960), &sources)
        .unwrap();
    let _play = host
        .offer_session_recording_play(generation, SampleTime::ZERO, ticket)
        .unwrap();
    let _stop = host
        .offer_session_stop(generation, SampleTime::new(128))
        .unwrap();
    for source in sources {
        fence(&mut host, generation, source, epoch, 0);
    }
    input(&mut host, generation, sources[0], epoch, 0, [0x90, 60, 100]);
    fence(&mut host, generation, sources[0], epoch, 128);
    fence(&mut host, generation, sources[1], epoch, 192);
    host.start(generation).unwrap();
    let mut output = [9.0; 193];
    render(&mut host, generation, &mut output, 37);
    assert_eq!(output[192], 0.0);
    assert_eq!(
        host.session_state(),
        Some(PlaybackState::Stopped(PlanPosition::new(128)))
    );
    assert!(matches!(
        host.note_capture().unwrap().result(ticket),
        Err(NoteCaptureError::Storage(
            crate::recording::CaptureError::NotSealed
        ))
    ));
    let _play = host.collect_session_receipt(generation).unwrap().unwrap();
    assert_eq!(
        host.collect_session_receipt(generation)
            .unwrap()
            .unwrap()
            .capture,
        Some(SessionCaptureOutcome::Applied)
    );
    render(&mut host, generation, &mut [9.0; 64], 37);
    assert_eq!(
        host.note_capture()
            .unwrap()
            .result(ticket)
            .unwrap()
            .window()
            .end(),
        SampleTime::new(128)
    );
}

#[test]
fn source_queue_and_receipt_saturation_cannot_spend_the_stop_slot() {
    let (mut host, generation, sources, epoch) = setup();
    for _ in 0..16 {
        input(&mut host, generation, sources[0], epoch, 0, [0xe0, 0, 64]);
    }
    assert!(matches!(
        host.offer_session_source(
            generation,
            SessionSourceAction::Publish {
                source: sources[0],
                stamp: CaptureStamp::exact_fixture(epoch, SampleTime::ZERO, SampleTime::ZERO)
                    .unwrap(),
                input: Midi1Input::from_bytes([0xe0, 0, 64]).unwrap(),
                audition: AuditionTrace::NotOffered
            }
        ),
        Err(HostError::Session(SessionError::Full))
    ));
    let _play = host
        .offer_session_play(generation, SampleTime::ZERO)
        .unwrap();
    let _stop = host
        .offer_session_stop(generation, SampleTime::new(64))
        .unwrap();
    host.start(generation).unwrap();
    let mut output = [9.0; 256];
    render(&mut host, generation, &mut output, 37);
    assert_eq!(&output[64..128], &[0.25; 64]);
    assert!(output[128..].iter().all(|sample| *sample == 0.0));
    assert!(matches!(
        host.session_capture_control(generation),
        Err(HostError::Session(SessionError::RetainedOutcomes))
    ));
    let mut count = 0;
    while let Some(receipt) = host.collect_session_source_receipt(generation).unwrap() {
        assert!(matches!(
            receipt.outcome,
            SessionSourceOutcome::Published(_)
        ));
        count += 1;
    }
    assert_eq!(count, 16);
}

#[test]
fn loss_without_callback_retains_source_and_command_cancellations_before_reclamation() {
    let (mut host, generation, sources, epoch) = setup();
    let ticket = host
        .session_capture_control(generation)
        .unwrap()
        .arm(context(epoch, 0, 960), &sources)
        .unwrap();
    let _play = host
        .offer_session_recording_play(generation, SampleTime::ZERO, ticket)
        .unwrap();
    let _stop = host
        .offer_session_stop(generation, SampleTime::new(128))
        .unwrap();
    for source in sources {
        fence(&mut host, generation, source, epoch, 0);
    }
    input(&mut host, generation, sources[0], epoch, 0, [0x90, 60, 100]);
    host.device_lost(generation).unwrap();
    assert!(host.acknowledge_quiescence(generation).is_err());
    for _ in 0..2 {
        let receipt = host.collect_session_receipt(generation).unwrap().unwrap();
        assert_eq!(receipt.outcome, SessionOutcome::Cancelled);
        assert_eq!(receipt.capture, Some(SessionCaptureOutcome::Cancelled));
    }
    for _ in 0..3 {
        assert_eq!(
            host.collect_session_source_receipt(generation)
                .unwrap()
                .unwrap()
                .outcome,
            SessionSourceOutcome::Cancelled
        );
    }
    assert!(host.acknowledge_quiescence(generation).is_err());
    for source in sources {
        host.acknowledge_note_source_quiescence(generation, source)
            .unwrap();
    }
    host.acknowledge_quiescence(generation).unwrap();
    assert_eq!(host.active().unwrap().state, ConnectionState::Unavailable);
    let result = host.note_capture().unwrap().result(ticket).unwrap();
    assert_eq!(result.selected_records().count(), 0);
}

#[test]
fn capture_stop_refusal_does_not_erase_audio_already_rendered_in_the_callback() {
    let (mut host, generation, sources, epoch) = setup();
    // This context naturally ends at sample 25, before the ordered Stop at 64.
    let ticket = host
        .session_capture_control(generation)
        .unwrap()
        .arm(context(epoch, 0, 1), &sources)
        .unwrap();
    let _play = host
        .offer_session_recording_play(generation, SampleTime::ZERO, ticket)
        .unwrap();
    let _stop = host
        .offer_session_stop(generation, SampleTime::new(64))
        .unwrap();
    for source in sources {
        fence(&mut host, generation, source, epoch, 0);
    }
    input(&mut host, generation, sources[0], epoch, 0, [0x90, 60, 100]);
    for source in sources {
        fence(&mut host, generation, source, epoch, 25);
    }
    host.start(generation).unwrap();
    let mut output = [9.0; 256];
    render(&mut host, generation, &mut output, 256);
    assert_eq!(&output[64..128], &[0.25; 64]);
    assert_eq!(host.active().unwrap().state, ConnectionState::Running);
    let _play = host.collect_session_receipt(generation).unwrap().unwrap();
    let stop = host.collect_session_receipt(generation).unwrap().unwrap();
    assert!(matches!(stop.outcome, SessionOutcome::Applied { .. }));
    assert_eq!(
        stop.capture,
        Some(SessionCaptureOutcome::Refused(NoteCaptureError::NotActive))
    );
}

#[test]
fn ordinary_play_stop_after_recording_carries_no_stale_capture_association() {
    for natural_end in [false, true] {
        let (mut host, generation, sources, epoch) = setup();
        let ticket = host
            .session_capture_control(generation)
            .unwrap()
            .arm(
                context(epoch, 0, if natural_end { 1 } else { 960 }),
                &sources,
            )
            .unwrap();
        let _play = host
            .offer_session_recording_play(generation, SampleTime::ZERO, ticket)
            .unwrap();
        let _stop = host
            .offer_session_stop(generation, SampleTime::new(64))
            .unwrap();
        for source in sources {
            fence(&mut host, generation, source, epoch, 0);
        }
        input(&mut host, generation, sources[0], epoch, 0, [0x90, 60, 100]);
        for source in sources {
            fence(
                &mut host,
                generation,
                source,
                epoch,
                if natural_end { 25 } else { 64 },
            );
        }
        host.start(generation).unwrap();
        render(&mut host, generation, &mut [9.0; 256], 37);
        for _ in 0..2 {
            let _receipt = host.collect_session_receipt(generation).unwrap().unwrap();
        }
        while let Some(_receipt) = host.collect_session_source_receipt(generation).unwrap() {}
        assert!(host.note_capture().unwrap().result(ticket).is_ok());
        let _play = host
            .offer_session_play(generation, SampleTime::new(192))
            .unwrap();
        let _stop = host
            .offer_session_stop(generation, SampleTime::new(256))
            .unwrap();
        let mut output = [9.0; 256];
        render(&mut host, generation, &mut output, 37);
        assert_eq!(&output[..64], &[0.25; 64]);
        assert!(output[64..].iter().all(|sample| *sample == 0.0));
        for _ in 0..2 {
            let receipt = host.collect_session_receipt(generation).unwrap().unwrap();
            assert_eq!(receipt.boundary.capture, None);
            assert_eq!(receipt.capture, None);
        }
    }
}
