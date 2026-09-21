use super::*;

fn scheduled(start: u64) -> (LoopRecordingSession, [ConnectionGeneration; 2]) {
    let mut capture = LoopCaptureSession::prepare(
        stream(0, 2048),
        limits(2, 64, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let sources = [
        capture.bind_source(ControllerSnapshot::neutral()).unwrap(),
        capture.bind_source(ControllerSnapshot::neutral()).unwrap(),
    ];
    let _ticket = capture
        .arm_at(input(), &sources, SampleTime::new(start))
        .unwrap();
    assert!(
        capture.start().is_err(),
        "direct start cannot bypass reserved time"
    );
    (
        LoopRecordingSession::prepare(capture, command_limits(4, 16384), source_limits(32, 32768))
            .unwrap(),
        sources,
    )
}

#[test]
fn future_play_and_stop_are_published_while_the_device_clock_runs() {
    let mut reference = None;
    for partitions in [&[512][..], &[64], &[256], &[1, 37, 128, 3, 256]] {
        let (mut owner, sources) = scheduled(256);
        let mut pcm = audio(&mut owner, 192, partitions);
        assert!(pcm.iter().all(|sample| *sample == 0.0));
        assert_eq!(owner.acknowledged().position, PlanPosition::ZERO);
        let _play = owner
            .offer(SampleTime::new(256), SessionCommand::Play)
            .unwrap();
        for source in sources {
            queue_fence(&mut owner, source, 256);
        }
        queue_input(&mut owner, sources[0], 260, [0x90, 60, 100]);
        queue_input(&mut owner, sources[0], 340, [0x80, 60, 0]);
        for source in sources {
            queue_fence(&mut owner, source, 512);
        }
        pcm.extend(audio(&mut owner, 192, partitions));
        assert!(matches!(
            owner.collect().unwrap().outcome,
            SessionOutcome::Applied { .. }
        ));
        let _stop = owner
            .offer(SampleTime::new(512), SessionCommand::Stop)
            .unwrap();
        pcm.extend(audio(&mut owner, 256, partitions));
        owner.finalize().unwrap();
        let result = owner.result().unwrap();
        assert_eq!(result.window().start(), SampleTime::new(256));
        assert_eq!(result.window().end(), SampleTime::new(512));
        assert_eq!(
            result.loop_passes().next().unwrap().window().start(),
            SampleTime::new(256)
        );
        assert_eq!(result.records().count(), 2);
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Complete);
        assert_eq!(pcm[320], 1.0, "the prepared 64-frame carry stays explicit");
        if let Some(expected) = &reference {
            assert_eq!(&pcm, expected);
        } else {
            reference = Some(pcm);
        }
    }
}

#[test]
fn split_preparation_retains_the_reserved_start_and_rejects_late_delivery() {
    use crate::host::session::loop_transfer::LoopTransferOutcome;
    let (owner, _) = scheduled(256);
    let (mut control, mut audio) = owner.split(PreparedBytes::measured(65536)).unwrap();
    assert!(
        control
            .prepare_command(SampleTime::ZERO, SessionCommand::Play)
            .is_err()
    );
    let packet = control
        .prepare_command(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    audio
        .render(AudioBlockMut::new(&mut [0.0; 512], 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    audio.enqueue(packet).unwrap();
    let completion = audio.take_completed().unwrap();
    let (_, outcome) = control.collect(completion).unwrap();
    assert!(matches!(
        outcome,
        LoopTransferOutcome::Refused(LoopSessionError::Session(SessionError::Boundary))
    ));
}

#[test]
fn stopped_before_reserved_start_retains_an_empty_take_and_cancels_play() {
    let (mut owner, sources) = scheduled(256);
    let _stop = owner
        .offer(SampleTime::new(128), SessionCommand::Stop)
        .unwrap();
    let _play = owner
        .offer(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    for source in sources {
        queue_fence(&mut owner, source, 128);
    }
    let output = audio(&mut owner, 512, &[37]);
    assert!(output.iter().all(|sample| *sample == 0.0));
    owner.finalize().unwrap();
    let result = owner.result().unwrap();
    assert_eq!(result.window().start(), SampleTime::new(128));
    assert_eq!(result.window().end(), SampleTime::new(128));
    assert!(
        result
            .loop_passes()
            .all(|pass| pass.window() == result.window())
    );
    assert_eq!(
        std::iter::from_fn(|| owner.collect())
            .filter(|receipt| receipt.outcome == SessionOutcome::Cancelled)
            .count(),
        1
    );
}

#[test]
fn failed_reserved_start_is_not_retried_and_stop_closes_an_empty_take() {
    let (mut owner, sources) = scheduled(256);
    let _play = owner
        .offer(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    let _stop = owner
        .offer(SampleTime::new(512), SessionCommand::Stop)
        .unwrap();
    // A later fence cannot retroactively promise the exact start boundary.
    for source in sources {
        queue_fence(&mut owner, source, 512);
    }
    let pcm = audio(&mut owner, 640, &[37]);
    assert!(pcm.iter().all(|sample| *sample == 0.0));
    assert_eq!(
        owner.collect().unwrap().outcome,
        SessionOutcome::CaptureRefused
    );
    assert!(matches!(
        owner.collect().unwrap().outcome,
        SessionOutcome::Applied { .. }
    ));
    owner.finalize().unwrap();
    let result = owner.result().unwrap();
    assert_eq!(result.window().start(), SampleTime::new(512));
    assert_eq!(result.window().end(), SampleTime::new(512));
    assert_eq!(result.records().count(), 0);
}

#[test]
fn unaligned_arm_refuses_without_consuming_the_fresh_capture() {
    let mut capture = LoopCaptureSession::prepare(
        stream(0, 2048),
        limits(2, 64, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let source = capture.bind_source(ControllerSnapshot::neutral()).unwrap();
    assert!(matches!(
        capture.arm_at(input(), &[source], SampleTime::new(63)),
        Err(LoopCaptureError::Mapping)
    ));
    let _ticket = capture
        .arm_at(input(), &[source], SampleTime::new(256))
        .unwrap();
    assert_eq!(capture.start, SampleTime::new(256));
}

#[test]
fn pending_audition_blocks_sealing_until_joined_resolution_and_panic_preserves_quality() {
    use crate::host::live::{AuditionId, AuditionOutcome};
    let (mut owner, sources) = scheduled(256);
    let _play = owner
        .offer(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    for source in sources {
        queue_fence(&mut owner, source, 256);
    }
    let id = AuditionId::new(sources[0], 1).unwrap();
    let _input = owner
        .offer_source(SessionSourceAction::Publish {
            source: sources[0],
            stamp: CaptureStamp::exact_fixture(
                owner.acknowledged().epoch,
                SampleTime::new(280),
                SampleTime::new(280),
            )
            .unwrap(),
            input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
            audition: AuditionTrace::Pending(id),
        })
        .unwrap();
    for source in sources {
        queue_fence(&mut owner, source, 512);
    }
    let _panic = owner
        .offer(SampleTime::new(512), SessionCommand::Panic)
        .unwrap();
    let _pcm = audio(&mut owner, 640, &[37]);
    assert!(
        owner.finalize().is_err(),
        "a pending trace cannot escape in a sealed take"
    );
    assert!(owner.result().is_err());
    assert_eq!(
        owner
            .resolve_audition(id, AuditionOutcome::Cancelled)
            .unwrap(),
        EventCount::measured(1)
    );
    owner.finalize().unwrap();
    let result = owner.result().unwrap();
    assert_eq!(result.sealed_outcome(), CaptureOutcome::Complete);
    assert_eq!(
        result.records().next().unwrap().audition(),
        AuditionTrace::Resolved(AuditionOutcome::Cancelled)
    );
    assert!(
        owner
            .resolve_audition(id, AuditionOutcome::Unsupported)
            .is_err(),
        "sealed annotation is immutable"
    );
}
