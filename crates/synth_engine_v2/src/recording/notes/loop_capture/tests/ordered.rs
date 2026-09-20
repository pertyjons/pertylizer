mod input;
mod transfer;
use super::*;
use crate::host::session::{
    LoopRecordingSession, LoopSessionError, SessionCaptureOutcome, SessionCommand,
    SessionCommandCapacity, SessionError, SessionLimits, SessionOutcome, SessionSourceAction,
    SessionSourceCapacity, SessionSourceLimits, SessionSourceOutcome,
};

fn command_limits(slots: u32, bytes: u64) -> SessionLimits {
    SessionLimits {
        commands: SessionCommandCapacity::new(slots).unwrap(),
        command_bytes: PreparedBytes::measured(bytes),
    }
}

fn source_limits(slots: u32, bytes: u64) -> SessionSourceLimits {
    SessionSourceLimits {
        actions: SessionSourceCapacity::new(slots).unwrap(),
        bytes: PreparedBytes::measured(bytes),
    }
}

fn armed() -> (LoopCaptureSession, [ConnectionGeneration; 2]) {
    let mut owner = LoopCaptureSession::prepare(
        stream(0, 2048),
        limits(2, 64, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let sources = [
        owner.bind_source(ControllerSnapshot::neutral()).unwrap(),
        owner.bind_source(ControllerSnapshot::neutral()).unwrap(),
    ];
    let _ticket = owner.arm(input(), &sources).unwrap();
    (owner, sources)
}

pub(in crate::recording::notes::loop_capture) fn setup()
-> (LoopRecordingSession, [ConnectionGeneration; 2]) {
    let (capture, sources) = armed();
    (
        LoopRecordingSession::prepare(capture, command_limits(4, 16384), source_limits(32, 32768))
            .unwrap(),
        sources,
    )
}

fn fence_action(
    owner: &LoopRecordingSession,
    source: ConnectionGeneration,
    at: u64,
) -> SessionSourceAction {
    SessionSourceAction::Fence {
        source,
        epoch: owner.acknowledged().epoch,
        frontier: SampleTime::new(at),
    }
}

fn queue_fence(owner: &mut LoopRecordingSession, source: ConnectionGeneration, at: u64) {
    let _id = owner.offer_source(fence_action(owner, source, at)).unwrap();
}

fn queue_input(
    owner: &mut LoopRecordingSession,
    source: ConnectionGeneration,
    at: u64,
    bytes: [u8; 3],
) {
    let _id = owner
        .offer_source(SessionSourceAction::Publish {
            source,
            stamp: CaptureStamp::exact_fixture(
                owner.acknowledged().epoch,
                SampleTime::new(at),
                SampleTime::new(at),
            )
            .unwrap(),
            input: Midi1Input::from_bytes(bytes).unwrap(),
            audition: AuditionTrace::NotOffered,
        })
        .unwrap();
}

fn queue_transport(owner: &mut LoopRecordingSession, stop: u64) {
    let _play = owner.offer(SampleTime::ZERO, SessionCommand::Play).unwrap();
    let _stop = owner
        .offer(SampleTime::new(stop), SessionCommand::Stop)
        .unwrap();
}

fn audio(owner: &mut LoopRecordingSession, total: usize, partitions: &[usize]) -> Vec<f32> {
    let mut output = vec![9.0; total];
    let mut offset = 0;
    let mut partition = 0;
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            while offset < total {
                let count = partitions[partition % partitions.len()].min(total - offset);
                owner
                    .render(
                        AudioBlockMut::new(
                            &mut output[offset..offset + count],
                            count,
                            ChannelLayout::Mono,
                        )
                        .unwrap(),
                    )
                    .unwrap();
                offset += count;
                partition += 1;
            }
        }),
        0
    );
    output
}

#[test]
fn ordered_loop_audio_raw_passes_carry_and_receipts_match_callback_partitions() {
    let mut reference = None;
    for partitions in [&[512][..], &[64], &[256], &[1, 37, 128, 3, 256], &[1]] {
        let (mut owner, sources) = setup();
        queue_transport(&mut owner, 320);
        for source in sources {
            queue_fence(&mut owner, source, 0);
        }
        queue_input(&mut owner, sources[0], 1, [0x90, 60, 100]);
        queue_input(&mut owner, sources[1], 49, [0x90, 60, 90]);
        queue_input(&mut owner, sources[0], 50, [0x80, 60, 0]);
        queue_input(&mut owner, sources[1], 100, [0x80, 60, 0]);
        queue_input(&mut owner, sources[0], 299, [0x90, 62, 100]);
        for source in sources {
            queue_fence(&mut owner, source, 320);
        }
        queue_input(&mut owner, sources[0], 320, [0x80, 62, 0]);
        let output = audio(&mut owner, 512, partitions);
        for (frame, sample) in output.iter().enumerate() {
            assert_eq!(
                *sample,
                f32::from((64..384).contains(&frame) && (frame - 64) % 50 == 0)
            );
        }
        owner.finalize().unwrap();
        let commands: Vec<_> = std::iter::from_fn(|| owner.collect())
            .map(|r| {
                assert_eq!(r.capture, Some(SessionCaptureOutcome::Applied));
                (
                    r.boundary.id.serial(),
                    r.boundary.at,
                    r.boundary.command,
                    r.outcome,
                )
            })
            .collect();
        assert_eq!(commands.len(), 2);
        let source_receipts: Vec<_> = std::iter::from_fn(|| owner.collect_source())
            .map(|r| {
                assert!(!matches!(
                    r.outcome,
                    SessionSourceOutcome::Cancelled | SessionSourceOutcome::Refused(_)
                ));
                r.id.serial()
            })
            .collect();
        let result = owner.result().unwrap();
        assert_eq!(result.window().end(), SampleTime::new(320));
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Complete);
        let records: Vec<_> = result
            .records()
            .map(|r| {
                (
                    sources
                        .iter()
                        .position(|source| *source == r.source())
                        .unwrap(),
                    r.sequence().as_u64(),
                    r.stamp().nominal(),
                    r.input(),
                    r.occurrence().map(|id| id.serial()),
                )
            })
            .collect();
        let passes: Vec<_> = result
            .loop_passes()
            .map(|p| {
                (
                    p.id().as_u64(),
                    p.rendered().as_u64(),
                    p.window().start(),
                    p.window().end(),
                    p.position(),
                )
            })
            .collect();
        assert_eq!(passes.len(), 7);
        let carry: Vec<_> = result
            .loop_carry()
            .map(|c| {
                (
                    sources
                        .iter()
                        .position(|source| *source == c.occurrence().source())
                        .unwrap(),
                    c.occurrence().serial(),
                    c.pass().as_u64(),
                    c.peer().as_u64(),
                    c.at(),
                    c.direction(),
                )
            })
            .collect();
        let observed = (output, records, passes, carry, commands, source_receipts);
        if let Some(reference) = &reference {
            assert_eq!(&observed, reference);
        } else {
            reference = Some(observed);
        }
    }
}

#[test]
fn stop_at_exact_loop_boundary_prevents_wrap_and_empty_final_pass() {
    for partition in [&[2048][..], &[64], &[256], &[17, 1, 99, 7]] {
        let (mut owner, sources) = setup();
        queue_transport(&mut owner, 1600);
        for source in sources {
            queue_fence(&mut owner, source, 0);
        }
        queue_input(&mut owner, sources[0], 1599, [0x90, 60, 1]);
        for source in sources {
            queue_fence(&mut owner, source, 1600);
        }
        let output = audio(&mut owner, 2048, partition);
        assert!(output[1664..].iter().all(|sample| *sample == 0.0));
        owner.finalize().unwrap();
        let end = owner.observation_end().unwrap();
        assert_eq!(end.at, SampleTime::new(1600));
        assert_eq!(end.pass.as_u64(), 32);
        assert_eq!(
            end.reason,
            crate::looping::journal::LoopJournalEndReason::Finished
        );
        let _play = owner.collect().unwrap();
        let stop = owner.collect().unwrap();
        assert_eq!(
            stop.outcome,
            SessionOutcome::Applied {
                position: PlanPosition::new(50)
            }
        );
        let result = owner.result().unwrap();
        assert_eq!(result.loop_passes().count(), 32);
        assert!(
            result
                .loop_passes()
                .all(|p| p.window().start() < p.window().end())
        );
        assert_eq!(result.loop_carry().count(), 0);
    }
}

#[test]
fn stalled_source_cannot_delay_stop_and_can_finish_without_another_callback() {
    let (mut owner, sources) = setup();
    queue_transport(&mut owner, 128);
    for source in sources {
        queue_fence(&mut owner, source, 0);
    }
    queue_input(&mut owner, sources[1], 1, [0x90, 60, 100]);
    queue_fence(&mut owner, sources[0], 128);
    let output = audio(&mut owner, 512, &[37]);
    assert!(output[192..].iter().all(|sample| *sample == 0.0));
    assert!(matches!(
        owner.finalize(),
        Err(LoopSessionError::Capture(LoopCaptureError::AwaitingSources))
    ));
    let _play = owner.collect().unwrap();
    assert!(matches!(
        owner.collect().unwrap().outcome,
        SessionOutcome::Applied { .. }
    ));
    let clock = owner.acknowledged();
    queue_fence(&mut owner, sources[1], 512);
    assert_eq!(
        crate::render_allocation::count_allocs(|| owner.drain_stopped_sources().unwrap()),
        0
    );
    owner.finalize().unwrap();
    assert_eq!(owner.acknowledged(), clock);
    assert_eq!(owner.result().unwrap().window().end(), SampleTime::new(128));
}

#[test]
fn missing_start_fence_refuses_play_and_same_time_stop_cancels_it() {
    for cancel in [false, true] {
        let (mut owner, sources) = setup();
        queue_transport(&mut owner, if cancel { 0 } else { 128 });
        queue_fence(&mut owner, sources[0], 0);
        let output = audio(&mut owner, 512, &[512]);
        assert!(output.iter().all(|sample| *sample == 0.0));
        let play = owner.collect().unwrap();
        if cancel {
            assert_eq!(play.outcome, SessionOutcome::Cancelled);
            assert_eq!(play.capture, Some(SessionCaptureOutcome::Cancelled));
        } else {
            assert_eq!(play.outcome, SessionOutcome::CaptureRefused);
            assert_eq!(
                play.capture,
                Some(SessionCaptureOutcome::Refused(NoteCaptureError::StartFence))
            );
        }
        assert!(matches!(
            owner.collect().unwrap().outcome,
            SessionOutcome::Applied { .. }
        ));
        for source in sources {
            queue_fence(&mut owner, source, 128);
        }
        owner.drain_stopped_sources().unwrap();
        owner.finalize().unwrap();
        let result = owner.result().unwrap();
        assert_eq!(result.window().start(), result.window().end());
        assert_eq!(result.records().count(), 0);
        assert_eq!(result.loop_passes().count(), 1);
        for pass in result.loop_passes() {
            assert_eq!(pass.window().start(), result.window().start());
            assert_eq!(pass.window().end(), result.window().end());
        }
    }
}

#[test]
fn loss_keeps_identified_outcomes_and_freezes_capture_without_another_callback() {
    for via_source in [false, true] {
        let (mut owner, sources) = setup();
        assert!(matches!(
            owner.acknowledge_source_quiescence(sources[0]),
            Err(LoopSessionError::NotClosed)
        ));
        let (_, foreign_sources) = setup();
        assert!(matches!(
            owner.source_lost(foreign_sources[0]),
            Err(LoopSessionError::Capture(LoopCaptureError::Capture(
                NoteCaptureError::ForeignSource
            )))
        ));
        queue_transport(&mut owner, 512);
        for source in sources {
            queue_fence(&mut owner, source, 0);
        }
        queue_input(&mut owner, sources[0], 1, [0x90, 60, 100]);
        for source in sources {
            queue_fence(&mut owner, source, 100);
        }
        queue_input(&mut owner, sources[1], 500, [0x90, 60, 100]);
        let _output = audio(&mut owner, 192, &[192]);
        let acknowledged = owner.acknowledged();
        assert_eq!(acknowledged.clock, SampleTime::new(128));
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                if via_source {
                    owner.source_lost(sources[0]).unwrap();
                } else {
                    owner.device_lost().unwrap();
                }
            }),
            0
        );
        assert!(matches!(
            owner.offer(SampleTime::new(512), SessionCommand::Stop),
            Err(LoopSessionError::Session(SessionError::Closed))
        ));
        assert!(matches!(
            owner.offer_source(fence_action(&owner, sources[0], 512)),
            Err(LoopSessionError::Session(SessionError::Closed))
        ));
        assert!(matches!(
            owner.finalize(),
            Err(LoopSessionError::Capture(LoopCaptureError::AwaitingSources))
        ));
        owner.acknowledge_source_quiescence(sources[0]).unwrap();
        assert!(owner.finalize().is_err());
        owner.acknowledge_source_quiescence(sources[1]).unwrap();
        owner.finalize().unwrap();
        owner.device_lost().unwrap();
        assert_eq!(owner.acknowledged(), acknowledged);
        assert!(matches!(
            owner.collect().unwrap().outcome,
            SessionOutcome::Applied { .. }
        ));
        assert_eq!(owner.collect().unwrap().outcome, SessionOutcome::Cancelled);
        let source_receipts: Vec<_> = std::iter::from_fn(|| owner.collect_source()).collect();
        assert_eq!(source_receipts.len(), 6);
        assert_eq!(
            source_receipts.last().unwrap().outcome,
            SessionSourceOutcome::Cancelled
        );
        let result = owner.result().unwrap();
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Interrupted);
        assert_eq!(result.window().end(), SampleTime::new(100));
        assert_eq!(result.records().count(), 1);
        assert_eq!(result.loop_passes().count(), 2);
        assert!(
            result
                .closures()
                .all(|closure| closure.reason == CaptureStopReason::DeviceLost)
        );
    }
}

#[test]
fn queue_saturation_preserves_stop_credit_and_every_source_outcome() {
    let (capture, sources) = armed();
    let mut owner =
        LoopRecordingSession::prepare(capture, command_limits(2, 16384), source_limits(2, 32768))
            .unwrap();
    queue_transport(&mut owner, 128);
    assert!(matches!(
        owner.offer(SampleTime::new(192), SessionCommand::Stop),
        Err(LoopSessionError::Session(SessionError::Full))
    ));
    for source in sources {
        queue_fence(&mut owner, source, 0);
    }
    assert!(matches!(
        owner.offer_source(fence_action(&owner, sources[0], 128)),
        Err(LoopSessionError::Session(SessionError::Full))
    ));
    let _audio = audio(&mut owner, 512, &[512]);
    assert!(matches!(
        owner.finalize(),
        Err(LoopSessionError::Capture(LoopCaptureError::AwaitingSources))
    ));
    for _ in 0..2 {
        assert_eq!(
            owner.collect_source().unwrap().outcome,
            SessionSourceOutcome::Fenced
        );
    }
    for source in sources {
        queue_fence(&mut owner, source, 128);
    }
    owner.drain_stopped_sources().unwrap();
    owner.finalize().unwrap();
    assert!(matches!(
        owner.discard(CaptureQuality::default()),
        Err(LoopSessionError::Session(SessionError::RetainedOutcomes))
    ));
    assert_eq!(std::iter::from_fn(|| owner.collect()).count(), 2);
    assert_eq!(std::iter::from_fn(|| owner.collect_source()).count(), 2);
    owner.close_completed().unwrap();
    for source in sources {
        owner.acknowledge_source_quiescence(source).unwrap();
    }
    let quality = owner.result().unwrap().quality();
    owner.discard(quality).unwrap();
    assert!(owner.result().is_err());
}

#[test]
fn command_and_source_budgets_admit_exactly_the_charged_storage() {
    let (owner, _) = setup();
    let command_bytes = owner.command_bytes();
    let source_bytes = owner.source_bytes();
    for short in [false, true] {
        let (capture, _) = armed();
        let result = LoopRecordingSession::prepare(
            capture,
            command_limits(4, command_bytes.get() - u64::from(short)),
            source_limits(32, source_bytes.get()),
        );
        assert_eq!(result.is_err(), short);
        let (capture, _) = armed();
        let result = LoopRecordingSession::prepare(
            capture,
            command_limits(4, command_bytes.get()),
            source_limits(32, source_bytes.get() - u64::from(short)),
        );
        assert_eq!(result.is_err(), short);
    }
}

#[test]
fn output_refusal_preserves_commands_and_terminal_fault_retains_results() {
    let (mut owner, sources) = setup();
    queue_transport(&mut owner, 512);
    for source in sources {
        queue_fence(&mut owner, source, 0);
    }
    let before = owner.acknowledged();
    assert!(
        owner
            .render(AudioBlockMut::new(&mut [9.0; 128], 64, ChannelLayout::Stereo).unwrap())
            .is_err()
    );
    assert_eq!(owner.acknowledged(), before);
    assert!(owner.collect().is_none());
    for source in sources {
        queue_fence(&mut owner, source, 64);
    }
    let _audio = audio(&mut owner, 192, &[192]);
    let before_fault = owner.acknowledged();
    let mut oversized = [9.0; 2049];
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            assert!(
                owner
                    .render(AudioBlockMut::new(&mut oversized, 2049, ChannelLayout::Mono).unwrap())
                    .is_err()
            );
        }),
        0
    );
    assert_eq!(oversized, [0.0; 2049]);
    assert_eq!(owner.acknowledged(), before_fault);
    assert!(owner.finalize().is_err());
    for source in sources {
        owner.acknowledge_source_quiescence(source).unwrap();
    }
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
    assert_eq!(owner.result().unwrap().window().end(), SampleTime::new(64));
    assert!(matches!(
        owner.collect().unwrap().outcome,
        SessionOutcome::Applied { .. }
    ));
    assert_eq!(owner.collect().unwrap().outcome, SessionOutcome::Cancelled);
}

#[test]
fn command_boundaries_source_order_and_finite_play_refuse_without_spending_identity() {
    let (mut owner, sources) = setup();
    assert!(matches!(
        owner.offer(SampleTime::new(1), SessionCommand::Stop),
        Err(LoopSessionError::Session(SessionError::Boundary))
    ));
    assert!(matches!(
        owner.offer(SampleTime::new(64), SessionCommand::Play),
        Err(LoopSessionError::FinitePlay)
    ));
    assert_eq!(
        owner
            .offer(SampleTime::ZERO, SessionCommand::Play)
            .unwrap()
            .serial(),
        1
    );
    assert!(matches!(
        owner.offer(SampleTime::ZERO, SessionCommand::Play),
        Err(LoopSessionError::FinitePlay)
    ));
    assert_eq!(
        owner
            .offer(SampleTime::new(64), SessionCommand::Stop)
            .unwrap()
            .serial(),
        2
    );
    for source in sources {
        queue_fence(&mut owner, source, 0);
    }
    queue_input(&mut owner, sources[0], 0, [0x90, 60, 1]);
    assert!(matches!(
        owner.offer_source(fence_action(&owner, sources[1], 0)),
        Err(LoopSessionError::Session(SessionError::SourceOrder))
    ));
    assert!(matches!(
        owner.drain_stopped_sources(),
        Err(LoopSessionError::NotStopped)
    ));
    let _audio = audio(&mut owner, 256, &[256]);
    assert!(matches!(
        owner.offer(SampleTime::new(192), SessionCommand::Play),
        Err(LoopSessionError::FinitePlay)
    ));
    assert_eq!(
        owner
            .offer_source(fence_action(&owner, sources[0], 64))
            .unwrap()
            .serial(),
        4
    );
}

#[test]
fn conversion_refuses_started_or_rendered_capture_and_session_share_overrun() {
    let (mut capture, sources) = armed();
    for source in sources {
        super::fence(&mut capture, source, 0);
    }
    capture.start().unwrap();
    let _receipt = super::publish(&mut capture, sources[0], 1, 1, [0x90, 60, 100]);
    let _audio = super::render(&mut capture, 128, 128);
    let error = match LoopRecordingSession::prepare(
        capture,
        command_limits(4, 16384),
        source_limits(32, 32768),
    ) {
        Err(error) => error,
        Ok(_) => panic!("started capture converted"),
    };
    assert!(matches!(error.error(), LoopSessionError::FreshCapture));
    let (mut capture, _) = error.into_parts();
    let _end = capture.finish_observation();
    for source in sources {
        super::fence(&mut capture, source, 64);
    }
    capture.finalize().unwrap();
    assert_eq!(capture.result().unwrap().records().count(), 1);
    let (capture, _) = armed();
    let mut owner = LoopRecordingSession::prepare(
        capture,
        command_limits(256, 1_000_000),
        source_limits(32, 32768),
    )
    .unwrap();
    let mut accepted = 0;
    loop {
        match owner.offer(SampleTime::ZERO, SessionCommand::Stop) {
            Ok(id) => {
                accepted += 1;
                assert_eq!(id.serial(), accepted);
            }
            Err(LoopSessionError::Session(SessionError::SessionShare)) => break,
            other => panic!("unexpected admission: {other:?}"),
        }
    }
    assert!(accepted > 0 && accepted < 256);
    let _audio = audio(&mut owner, 128, &[128]);
    assert_eq!(
        std::iter::from_fn(|| owner.collect()).count() as u64,
        accepted
    );
}
