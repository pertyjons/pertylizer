//! P09-S003: executable note lifetime and custody checks for the exact serial publisher.
#![cfg(feature = "simulated-ingress")]

use synth_engine_v2::host::ConnectionGeneration;
use synth_engine_v2::profile::{CaptureLimits, CaptureLimitsInput, RecordingLimits};
use synth_engine_v2::quantities::{
    CapturePassCount, CaptureResultCount, CaptureSourceCount, EventCount, HeldNoteCount,
    PreparedBytes, ProjectionTickCount, SampleRate, TrackedInputNoteCount,
};
use synth_engine_v2::recording::notes::*;
use synth_engine_v2::recording::{CaptureError, CaptureOutcome, TakeReservation};
use synth_engine_v2::tempo::{Bpm, MusicalTick, TempoMap};
use synth_engine_v2::time::{
    FrameCount, PlanPosition, SampleTime, StreamAnchor, StreamEpoch, TimeSource, issue_epoch,
};

fn limits(held: u32, events: u32, tracked: u32, results: u32) -> RecordingLimits {
    RecordingLimits::new(
        HeldNoteCount::limit(held).unwrap(),
        EventCount::limit(events).unwrap(),
    )
    .unwrap()
    .with_capture(
        CaptureLimits::new(CaptureLimitsInput {
            max_tracked_input_notes: TrackedInputNoteCount::limit(tracked).unwrap(),
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
fn context(epoch: StreamEpoch, start: u64, end: u64) -> NoteArmContext {
    NoteArmContext::prepare(NoteArmInput {
        target: FixtureTargetId::new(11).unwrap(),
        expected_revision: FixtureRevision::new(7),
        interval: MusicalInterval::new(MusicalTick::new(start), MusicalTick::new(end)).unwrap(),
        mode: CaptureMode::default(),
        quantization: CaptureQuantization::default(),
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
fn recorder(limits: RecordingLimits) -> (SimulatedNoteRecorder, ConnectionGeneration, StreamEpoch) {
    let epoch = issue_epoch().unwrap();
    let mut recorder = SimulatedNoteRecorder::prepare_fixture(epoch, limits).unwrap();
    let source = recorder
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    (recorder, source, epoch)
}
fn publish(
    recorder: &mut SimulatedNoteRecorder,
    source: ConnectionGeneration,
    epoch: StreamEpoch,
    time: u64,
    bytes: [u8; 3],
) -> PublicationReceipt {
    recorder
        .publish(
            source,
            CaptureStamp::exact_fixture(epoch, SampleTime::new(time), SampleTime::new(time))
                .unwrap(),
            Midi1Input::from_bytes(bytes).unwrap(),
            AuditionTrace::NotOffered,
        )
        .unwrap()
}
fn fence(
    recorder: &mut SimulatedNoteRecorder,
    source: ConnectionGeneration,
    epoch: StreamEpoch,
    time: u64,
) {
    recorder
        .fence(
            source,
            epoch,
            SampleTime::new(time),
            recorder.source_sequence(source).unwrap(),
        )
        .unwrap();
}
fn start(
    recorder: &mut SimulatedNoteRecorder,
    sources: &[ConnectionGeneration],
    epoch: StreamEpoch,
) -> TakeReservation {
    let prepared = context(epoch, 1, 5);
    assert_eq!(prepared.window().start(), SampleTime::new(25));
    assert_eq!(prepared.window().end(), SampleTime::new(125));
    let ticket = recorder.arm(prepared, sources).unwrap();
    for source in sources {
        fence(recorder, *source, epoch, 25);
    }
    recorder.start(ticket).unwrap();
    ticket
}

#[test]
fn midi_validation_refuses_invalid_and_unsupported_input_and_normalizes_zero_velocity() {
    for bytes in [[0x90, 128, 1], [0x80, 1, 128], [0xe0, 0, 128]] {
        assert!(matches!(
            Midi1Input::from_bytes(bytes),
            Err(NoteCaptureError::InvalidMidi)
        ));
    }
    for bytes in [[0xb0, 1, 20], [0xc0, 0, 0], [0xf0, 0, 0], [0x00, 0, 0]] {
        assert!(matches!(
            Midi1Input::from_bytes(bytes),
            Err(NoteCaptureError::UnsupportedMidi)
        ));
    }
    let release = Midi1Input::from_bytes([0x9f, 60, 0]).unwrap();
    assert!(matches!(release.event(), Midi1Event::KeyRelease { .. }));
    assert_eq!(release.channel().as_u8(), 16);
    for (bytes, expected) in [
        ([0xe0, 0, 0], -1.0),
        ([0xe0, 0, 64], 0.0),
        ([0xe0, 127, 127], 1.0),
    ] {
        let Midi1Event::PitchBend { value } = Midi1Input::from_bytes(bytes).unwrap().event() else {
            panic!("bend");
        };
        assert_eq!(value.as_f32(), expected);
    }
}

#[test]
fn arm_requires_known_state_and_retains_target_revision_mapping_and_defaults() {
    let epoch = issue_epoch().unwrap();
    let mut recorder = SimulatedNoteRecorder::prepare_fixture(epoch, limits(2, 16, 6, 2)).unwrap();
    let source = recorder.bind_fixture_source(None).unwrap();
    assert!(matches!(
        recorder.arm(context(epoch, 1, 5), &[source]),
        Err(NoteCaptureError::Unsynchronized)
    ));
    assert!(!recorder.is_active());
    recorder
        .synchronize_fixture_source(source, ControllerSnapshot::neutral())
        .unwrap();
    let ticket = start(&mut recorder, &[source], epoch);
    assert!(matches!(
        recorder.arm(context(epoch, 6, 10), &[source]),
        Err(NoteCaptureError::AlreadyArmed)
    ));
    let _receipt = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100]);
    fence(&mut recorder, source, epoch, 125); // End seals even without another input event.
    assert!(!recorder.is_active());
    let result = recorder.result(ticket).unwrap();
    assert_eq!(result.context().target().as_u64(), 11);
    assert_eq!(result.context().expected_revision().as_u64(), 7);
    assert_eq!(result.context().mode(), CaptureMode::Overdub);
    assert_eq!(result.context().quantization(), CaptureQuantization::Off);
    assert_eq!(
        result
            .context()
            .tempo()
            .position_of(MusicalTick::new(5))
            .unwrap(),
        PlanPosition::new(125)
    );
    assert_eq!(result.effective_outcome(), CaptureOutcome::Complete);
    assert_eq!(result.closures().next().unwrap().time, SampleTime::new(125));
}

#[test]
fn prearm_and_countin_keys_remain_uncaptured_and_releases_match_fifo() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 6, 2));
    let a = publish(&mut recorder, source, epoch, 10, [0x90, 60, 100])
        .occurrence
        .unwrap();
    let ticket = recorder.arm(context(epoch, 1, 5), &[source]).unwrap();
    let b = publish(&mut recorder, source, epoch, 20, [0x90, 60, 100])
        .occurrence
        .unwrap();
    fence(&mut recorder, source, epoch, 25);
    recorder.start(ticket).unwrap();
    let c = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100])
        .occurrence
        .unwrap();
    for (time, expected) in [(40, a), (50, b), (60, c)] {
        assert_eq!(
            publish(&mut recorder, source, epoch, time, [0x80, 60, 0]).occurrence,
            Some(expected)
        );
    }
    fence(&mut recorder, source, epoch, 125);
    let result = recorder.result(ticket).unwrap();
    assert_eq!(
        result
            .initial_held()
            .map(|held| held.occurrence)
            .collect::<Vec<_>>(),
        [a, b]
    );
    assert_eq!(
        result
            .records()
            .filter(|record| matches!(record.input().event(), Midi1Event::NoteOn { .. }))
            .count(),
        1
    );
    assert_eq!(result.closures().count(), 0);
    assert_eq!(result.records().count(), 4);
}

#[test]
fn same_key_sources_channels_and_zero_velocity_releases_do_not_alias() {
    let (mut recorder, a, epoch) = recorder(limits(4, 32, 8, 2));
    let b = recorder
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    let ticket = start(&mut recorder, &[a, b], epoch);
    let first = publish(&mut recorder, a, epoch, 30, [0x90, 60, 90])
        .occurrence
        .unwrap();
    let second = publish(&mut recorder, a, epoch, 31, [0x90, 60, 80])
        .occurrence
        .unwrap();
    let channel = publish(&mut recorder, a, epoch, 32, [0x91, 60, 70])
        .occurrence
        .unwrap();
    let other = publish(&mut recorder, b, epoch, 33, [0x90, 60, 60])
        .occurrence
        .unwrap();
    for (source, time, bytes, expected) in [
        (a, 40, [0x90, 60, 0], first),
        (b, 41, [0x80, 60, 0], other),
        (a, 42, [0x81, 60, 0], channel),
        (a, 43, [0x80, 60, 0], second),
    ] {
        assert_eq!(
            publish(&mut recorder, source, epoch, time, bytes).occurrence,
            Some(expected)
        );
    }
    assert_eq!(
        publish(&mut recorder, a, epoch, 44, [0x80, 60, 0]).occurrence,
        None
    );
    fence(&mut recorder, a, epoch, 125);
    assert!(recorder.is_active());
    fence(&mut recorder, b, epoch, 125);
    assert_eq!(recorder.result(ticket).unwrap().closures().count(), 0);
    assert_eq!(
        recorder
            .source_diagnostics(a)
            .unwrap()
            .unmatched_releases
            .as_u64(),
        1
    );
}

#[test]
fn sustain_does_not_delay_key_pairing_and_expression_remains_separate() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 6, 2));
    let _receipt = publish(&mut recorder, source, epoch, 10, [0xb0, 64, 127]);
    let ticket = start(&mut recorder, &[source], epoch);
    let on = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100]).occurrence;
    let _receipt = publish(&mut recorder, source, epoch, 35, [0xe0, 127, 127]);
    assert_eq!(
        publish(&mut recorder, source, epoch, 40, [0x80, 60, 0]).occurrence,
        on
    );
    fence(&mut recorder, source, epoch, 125);
    let result = recorder.result(ticket).unwrap();
    assert!(result.initial_sources().next().unwrap().controls.pedals()[0]);
    assert!(result.terminal_sources().next().unwrap().controls.pedals()[0]);
    assert_eq!(
        result.terminal_sources().next().unwrap().controls.bends()[0].as_f32(),
        1.0
    );
    assert_eq!(result.closures().count(), 0);
    assert_eq!(result.records().count(), 3);
}

#[test]
fn exclusive_end_uses_prior_held_and_pedal_state_and_preserves_outside_pairing() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 6, 2));
    let ticket = start(&mut recorder, &[source], epoch);
    let occurrence = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100])
        .occurrence
        .unwrap();
    let _receipt = publish(&mut recorder, source, epoch, 40, [0xb0, 64, 127]);
    assert_eq!(
        publish(&mut recorder, source, epoch, 125, [0x80, 60, 0]).capture,
        CaptureDisposition::OutsideInterval
    );
    let _receipt = publish(&mut recorder, source, epoch, 125, [0xb0, 64, 0]);
    let outside = publish(&mut recorder, source, epoch, 125, [0x90, 60, 100])
        .occurrence
        .unwrap();
    fence(&mut recorder, source, epoch, 125);
    let result = recorder.result(ticket).unwrap();
    let closure = result.closures().next().unwrap();
    assert_eq!(closure.occurrence, occurrence);
    assert!(closure.key_held);
    assert_eq!(closure.pedal_held, Some(true));
    assert_eq!(result.records().count(), 2);
    assert_eq!(
        publish(&mut recorder, source, epoch, 130, [0x80, 60, 0]).occurrence,
        Some(outside)
    );
}

#[test]
fn original_decreasing_timestamps_and_audition_trace_survive_without_repair() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 6, 2));
    let ticket = start(&mut recorder, &[source], epoch);
    let on = publish(&mut recorder, source, epoch, 50, [0x90, 60, 100]).occurrence;
    let release = recorder
        .publish(
            source,
            CaptureStamp::exact_fixture(epoch, SampleTime::new(45), SampleTime::new(60)).unwrap(),
            Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
            AuditionTrace::Executed(SampleTime::new(80)),
        )
        .unwrap();
    assert_eq!(release.occurrence, on);
    fence(&mut recorder, source, epoch, 125);
    let result = recorder.result(ticket).unwrap();
    let record = result.records().nth(1).unwrap();
    assert_eq!(record.stamp().nominal(), SampleTime::new(45));
    assert_eq!(record.stamp().provenance(), TimeSource::Simulated);
    assert!(record.timing_anomaly());
    assert_eq!(
        record.audition(),
        AuditionTrace::Executed(SampleTime::new(80))
    );
    assert_eq!(
        recorder
            .source_diagnostics(source)
            .unwrap()
            .timing_anomalies
            .as_u64(),
        1
    );
}

#[test]
fn pending_stop_accepts_delayed_in_interval_release_until_its_source_fence() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 6, 2));
    let ticket = start(&mut recorder, &[source], epoch);
    let on = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100]).occurrence;
    recorder
        .stop(ticket, SampleTime::new(80), CaptureStopReason::Disarm)
        .unwrap();
    let late_execution = recorder
        .publish(
            source,
            CaptureStamp::exact_fixture(epoch, SampleTime::new(70), SampleTime::new(90)).unwrap(),
            Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
            AuditionTrace::NotOffered,
        )
        .unwrap();
    assert_eq!(late_execution.capture, CaptureDisposition::Recorded);
    assert_eq!(late_execution.occurrence, on);
    fence(&mut recorder, source, epoch, 90);
    let result = recorder.result(ticket).unwrap();
    assert_eq!(result.window().end(), SampleTime::new(80));
    assert_eq!(result.closures().count(), 0);
    assert_eq!(result.records().count(), 2);
}

#[test]
fn held_and_ordinary_exhaustion_preserve_prefix_and_emergency_closures() {
    for (held, events, expected) in [(1, 16, 1), (2, 1, 1)] {
        let (mut recorder, source, epoch) = recorder(limits(held, events, 6, 2));
        let ticket = start(&mut recorder, &[source], epoch);
        let _receipt = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100]);
        assert_eq!(
            publish(&mut recorder, source, epoch, 40, [0x90, 61, 100]).capture,
            CaptureDisposition::Stopped(CaptureStopReason::Capacity)
        );
        fence(&mut recorder, source, epoch, 40);
        let result = recorder.result(ticket).unwrap();
        assert_eq!(result.effective_outcome(), CaptureOutcome::Partial);
        assert_eq!(result.records().count(), expected);
        assert_eq!(result.closures().count(), 1);
        assert_eq!(result.closures().next().unwrap().time, SampleTime::new(40));
    }
}

#[test]
fn countin_cannot_consume_the_admitted_tracker_reserve() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 4, 2));
    let _receipt = publish(&mut recorder, source, epoch, 5, [0x90, 50, 100]);
    let _receipt = publish(&mut recorder, source, epoch, 10, [0x90, 51, 100]);
    let ticket = recorder.arm(context(epoch, 1, 5), &[source]).unwrap();
    let stamp =
        CaptureStamp::exact_fixture(epoch, SampleTime::new(20), SampleTime::new(20)).unwrap();
    let input = Midi1Input::from_bytes([0x90, 52, 100]).unwrap();
    assert_eq!(
        recorder.publish(source, stamp, input, AuditionTrace::NotOffered),
        Err(NoteCaptureError::TrackerReserved)
    );
    assert_eq!(
        recorder.publish(source, stamp, input, AuditionTrace::NotOffered),
        Err(NoteCaptureError::Unsynchronized)
    );
    recorder
        .quiesce(source, epoch, SampleTime::new(20))
        .unwrap();
    let result = recorder.result(ticket).unwrap();
    assert_eq!(result.effective_outcome(), CaptureOutcome::Interrupted);
    assert_eq!(result.window().start(), result.window().end());
    assert_eq!(result.records().count(), 0);
    let fresh = recorder
        .rebind_fixture_source(source, Some(ControllerSnapshot::neutral()))
        .unwrap()
        .generation;
    assert_ne!(fresh, source);
}

#[test]
fn late_refusals_reach_old_results_and_never_move_to_a_new_take() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 8, 2));
    let first = start(&mut recorder, &[source], epoch);
    let _receipt = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100]);
    let _receipt = publish(&mut recorder, source, epoch, 40, [0x80, 60, 0]);
    fence(&mut recorder, source, epoch, 125);
    let second = recorder.arm(context(epoch, 6, 10), &[source]).unwrap();
    let late = recorder
        .publish(
            source,
            CaptureStamp::exact_fixture(epoch, SampleTime::new(35), SampleTime::new(140)).unwrap(),
            Midi1Input::from_bytes([0xb0, 64, 127]).unwrap(),
            AuditionTrace::NotOffered,
        )
        .unwrap();
    assert_eq!(late.capture, CaptureDisposition::Late);
    assert_eq!(
        recorder.result(first).unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(
        recorder.result(first).unwrap().effective_outcome(),
        CaptureOutcome::Partial
    );
    fence(&mut recorder, source, epoch, 150);
    recorder.start(second).unwrap();
    fence(&mut recorder, source, epoch, 250);
    assert_eq!(recorder.result(second).unwrap().records().count(), 0);
    assert_eq!(
        recorder.result(second).unwrap().effective_outcome(),
        CaptureOutcome::Complete
    );
    assert!(matches!(
        recorder.arm(context(epoch, 11, 15), &[source]),
        Err(NoteCaptureError::Storage(CaptureError::ResultsFull))
    ));
    let _notification = recorder.take_notification();
    assert_eq!(recorder.retained_results().count(), 2);
    recorder
        .quiesce(source, epoch, SampleTime::new(250))
        .unwrap();
    for ticket in [first, second] {
        let quality = recorder.result(ticket).unwrap().quality();
        recorder.discard(ticket, quality).unwrap();
    }
    assert_eq!(recorder.retained_results().count(), 0);
}

#[test]
fn loss_before_start_and_panic_both_retain_a_readable_result() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 6, 2));
    let before = recorder.arm(context(epoch, 1, 5), &[source]).unwrap();
    recorder
        .quiesce(source, epoch, SampleTime::new(10))
        .unwrap();
    assert_eq!(
        recorder.result(before).unwrap().window().start(),
        SampleTime::new(10)
    );
    assert_eq!(recorder.result(before).unwrap().records().count(), 0);
    let source = recorder
        .rebind_fixture_source(source, Some(ControllerSnapshot::neutral()))
        .unwrap()
        .generation;
    let panic = start(&mut recorder, &[source], epoch);
    let _receipt = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100]);
    recorder
        .stop(panic, SampleTime::new(60), CaptureStopReason::Panic)
        .unwrap();
    fence(&mut recorder, source, epoch, 60);
    let result = recorder.result(panic).unwrap();
    assert_eq!(result.records().count(), 1);
    assert_eq!(
        result.closures().next().unwrap().reason,
        CaptureStopReason::Panic
    );
}

#[test]
fn stale_fences_and_unordered_start_refuse_without_consuming_input() {
    let (mut recorder, source, epoch) = recorder(limits(2, 12, 4, 2));
    let ticket = recorder.arm(context(epoch, 1, 5), &[source]).unwrap();
    let initial_sequence = recorder.source_sequence(source).unwrap();
    assert_eq!(recorder.start(ticket), Err(NoteCaptureError::StartFence));
    let stamp =
        CaptureStamp::exact_fixture(epoch, SampleTime::new(25), SampleTime::new(25)).unwrap();
    let note = Midi1Input::from_bytes([0x90, 60, 100]).unwrap();
    assert_eq!(
        recorder.publish(source, stamp, note, AuditionTrace::NotOffered),
        Err(NoteCaptureError::StartRequired)
    );
    assert_eq!(recorder.source_sequence(source).unwrap(), initial_sequence);
    fence(&mut recorder, source, epoch, 25);
    recorder.start(ticket).unwrap();
    let first = publish(&mut recorder, source, epoch, 26, [0x90, 60, 100]);
    assert_eq!(
        recorder.fence(source, epoch, SampleTime::new(30), initial_sequence),
        Err(NoteCaptureError::InvalidFence)
    );
    fence(&mut recorder, source, epoch, 30);
    let past =
        CaptureStamp::exact_fixture(epoch, SampleTime::new(29), SampleTime::new(29)).unwrap();
    assert_eq!(
        recorder.publish(source, past, note, AuditionTrace::NotOffered),
        Err(NoteCaptureError::PastBoundary)
    );
    assert_eq!(recorder.source_sequence(source).unwrap(), first.sequence);
    let foreign = issue_epoch().unwrap();
    assert_eq!(
        recorder.fence(source, foreign, SampleTime::new(31), first.sequence),
        Err(NoteCaptureError::ForeignEpoch)
    );
    fence(&mut recorder, source, epoch, 125);
    assert_eq!(recorder.result(ticket).unwrap().records().count(), 1);
}

#[test]
fn mapping_change_waits_for_both_sources_and_retains_each_context() {
    let (mut recorder, source, epoch) = recorder(limits(2, 12, 4, 2));
    let other = recorder
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    let first = start(&mut recorder, &[source, other], epoch);
    let _receipt = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100]);
    recorder
        .stop(first, SampleTime::new(50), CaptureStopReason::TempoChange)
        .unwrap();
    fence(&mut recorder, source, epoch, 50);
    assert!(recorder.is_active());
    assert_eq!(
        recorder.arm(context(epoch, 3, 8), &[source]),
        Err(NoteCaptureError::AlreadyArmed)
    );
    fence(&mut recorder, other, epoch, 50);
    assert!(!recorder.is_active());
    let new_context = NoteArmContext::prepare(NoteArmInput {
        target: FixtureTargetId::new(22).unwrap(),
        expected_revision: FixtureRevision::new(8),
        interval: MusicalInterval::new(MusicalTick::new(3), MusicalTick::new(8)).unwrap(),
        mode: CaptureMode::Replace,
        quantization: CaptureQuantization::Grid(
            QuantizationGrid::new(MusicalTick::new(2)).unwrap(),
        ),
        epoch,
        anchor: StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        tempo: TempoMap::new(
            Bpm::new(60.0).unwrap(),
            &[],
            SampleRate::new(48_000.0).unwrap(),
        )
        .unwrap(),
    })
    .unwrap();
    assert_eq!(new_context.window().start(), SampleTime::new(150));
    let second = recorder.arm(new_context, &[source]).unwrap();
    fence(&mut recorder, source, epoch, 150);
    recorder.start(second).unwrap();
    fence(&mut recorder, source, epoch, 400);
    let old = recorder.result(first).unwrap();
    let new = recorder.result(second).unwrap();
    assert_eq!(old.context().window().end(), SampleTime::new(125));
    assert_eq!(old.window().end(), SampleTime::new(50));
    assert_eq!(
        old.closures().next().unwrap().reason,
        CaptureStopReason::TempoChange
    );
    assert_eq!(old.context().mode(), CaptureMode::Overdub);
    assert_eq!(new.context().mode(), CaptureMode::Replace);
    assert!(matches!(
        new.context().quantization(),
        CaptureQuantization::Grid(_)
    ));
    assert_ne!(old.pass(), new.pass());
}

#[test]
fn serial_log_partition_and_audition_trace_do_not_change_capture_lifetimes() {
    let events = [
        (25, [0x90, 60, 100]),
        (64, [0x90, 60, 90]),
        (65, [0xb0, 64, 127]),
        (128, [0x80, 60, 40]),
        (257, [0xe0, 0, 65]),
        (700, [0x90, 60, 0]),
        (999, [0xb0, 64, 0]),
    ];
    let mut reference = None;
    let mut interior_fence_hits_event = false;
    for partition in [
        &[1000][..],
        &[64][..],
        &[256][..],
        &[39][..],
        &[17, 65, 3, 201][..],
    ] {
        for trace in [
            AuditionTrace::NotOffered,
            AuditionTrace::Refused(synth_engine_v2::ingress::IngressRefused::NonMonotoneStamp {
                time: SampleTime::new(1),
                last: SampleTime::new(2),
            }),
            AuditionTrace::Executed(SampleTime::new(4096)),
        ] {
            let (mut recorder, source, epoch) = recorder(limits(4, 16, 8, 1));
            let ticket = recorder.arm(context(epoch, 1, 40), &[source]).unwrap();
            fence(&mut recorder, source, epoch, 25);
            recorder.start(ticket).unwrap();
            let mut cursor = 0;
            let mut boundary = 25;
            let mut step = 0;
            while boundary < 1000 {
                boundary = (boundary + partition[step % partition.len()]).min(1000);
                while let Some(&(time, bytes)) = events.get(cursor) {
                    if time >= boundary {
                        break;
                    }
                    let receipt = recorder
                        .publish(
                            source,
                            CaptureStamp::exact_fixture(
                                epoch,
                                SampleTime::new(time),
                                SampleTime::new(time),
                            )
                            .unwrap(),
                            Midi1Input::from_bytes(bytes).unwrap(),
                            trace,
                        )
                        .unwrap();
                    assert_eq!(receipt.capture, CaptureDisposition::Recorded);
                    cursor += 1;
                }
                interior_fence_hits_event |= events.iter().any(|(time, _)| *time == boundary);
                fence(&mut recorder, source, epoch, boundary);
                step += 1;
            }
            let result = recorder.result(ticket).unwrap();
            assert_eq!(result.sealed_outcome(), CaptureOutcome::Complete);
            assert_eq!(result.closures().count(), 0);
            assert!(result.records().all(|record| record.audition() == trace));
            // Each run owns fresh global source IDs; compare source-local identity and timing.
            let normalized: Vec<_> = result
                .records()
                .map(|record| {
                    (
                        record.stamp().nominal(),
                        record.input(),
                        record.sequence().as_u64(),
                        record.occurrence().map(PerformedOccurrenceId::serial),
                    )
                })
                .collect();
            if let Some(reference) = &reference {
                assert_eq!(&normalized, reference);
            } else {
                reference = Some(normalized);
            }
        }
    }
    assert!(interior_fence_hits_event);
}

#[test]
fn finalized_session_boundary_also_constrains_new_sources() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 4, 2));
    let ticket = start(&mut recorder, &[source], epoch);
    fence(&mut recorder, source, epoch, 125);
    assert!(recorder.result(ticket).is_ok());
    let fresh = recorder
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    assert_eq!(
        recorder.arm(context(epoch, 2, 4), &[fresh]),
        Err(NoteCaptureError::PastBoundary)
    );
    let next = recorder.arm(context(epoch, 6, 8), &[fresh]).unwrap();
    fence(&mut recorder, fresh, epoch, 150);
    recorder.start(next).unwrap();
    fence(&mut recorder, fresh, epoch, 200);
    assert_eq!(
        recorder.result(next).unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
}

#[test]
fn stopping_or_losing_countin_never_bypasses_start_or_creates_a_nonempty_take() {
    for at in [10, 80, 150] {
        for interrupted in [false, true] {
            let (mut recorder, source, epoch) = recorder(limits(2, 16, 4, 2));
            let ticket = recorder.arm(context(epoch, 1, 5), &[source]).unwrap();
            if interrupted {
                recorder
                    .quiesce(source, epoch, SampleTime::new(at))
                    .unwrap();
            } else {
                recorder
                    .stop(ticket, SampleTime::new(at), CaptureStopReason::Disarm)
                    .unwrap();
                let receipt = recorder
                    .publish(
                        source,
                        CaptureStamp::exact_fixture(epoch, SampleTime::new(5), SampleTime::new(at))
                            .unwrap(),
                        Midi1Input::from_bytes([0x90, 60, 1]).unwrap(),
                        AuditionTrace::NotOffered,
                    )
                    .unwrap();
                assert_ne!(receipt.capture, CaptureDisposition::Recorded);
                fence(&mut recorder, source, epoch, at);
            }
            let result = recorder.result(ticket).unwrap();
            assert_eq!(result.window().start(), result.window().end());
            assert_eq!(result.records().count(), 0);
            assert_eq!(
                result.sealed_outcome(),
                if interrupted {
                    CaptureOutcome::Interrupted
                } else {
                    CaptureOutcome::Complete
                }
            );
        }
    }
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 4, 2));
    let ticket = recorder.arm(context(epoch, 1, 5), &[source]).unwrap();
    fence(&mut recorder, source, epoch, 40);
    assert_eq!(recorder.start(ticket), Err(NoteCaptureError::StartFence));
    recorder
        .stop(ticket, SampleTime::new(40), CaptureStopReason::Disarm)
        .unwrap();
    assert_eq!(
        recorder.result(ticket).unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
}

#[test]
fn an_unused_bound_source_can_enter_after_a_newer_generation() {
    let (mut recorder, older, epoch) = recorder(limits(2, 16, 4, 2));
    let newer = recorder
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    let first = start(&mut recorder, &[newer], epoch);
    fence(&mut recorder, newer, epoch, 125);
    let second = recorder.arm(context(epoch, 6, 8), &[older]).unwrap();
    fence(&mut recorder, older, epoch, 150);
    recorder.start(second).unwrap();
    fence(&mut recorder, older, epoch, 200);
    assert_eq!(
        recorder.result(first).unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(
        recorder.result(second).unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    recorder
        .quiesce(older, epoch, SampleTime::new(200))
        .unwrap();
    assert!(matches!(
        recorder.arm(context(epoch, 9, 10), &[older]),
        Err(NoteCaptureError::Unsynchronized)
    ));
}

#[test]
fn unselected_sources_cannot_spend_a_captures_remaining_tracker_slots() {
    let (mut recorder, selected, epoch) = recorder(limits(2, 16, 4, 2));
    let other = recorder
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    let ticket = start(&mut recorder, &[selected], epoch);
    for time in [26, 27] {
        let _receipt = publish(&mut recorder, other, epoch, time, [0x90, 60, 1]);
    }
    let stamp =
        CaptureStamp::exact_fixture(epoch, SampleTime::new(28), SampleTime::new(28)).unwrap();
    assert_eq!(
        recorder.publish(
            other,
            stamp,
            Midi1Input::from_bytes([0x90, 60, 1]).unwrap(),
            AuditionTrace::NotOffered
        ),
        Err(NoteCaptureError::TrackerReserved)
    );
    for time in [29, 30] {
        assert_eq!(
            publish(&mut recorder, selected, epoch, time, [0x90, 60, 1]).capture,
            CaptureDisposition::Recorded
        );
    }
    fence(&mut recorder, selected, epoch, 125);
    assert_eq!(
        recorder.result(ticket).unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(recorder.result(ticket).unwrap().closures().count(), 2);
}

#[test]
fn a_shortened_interval_closes_selected_onsets_only_and_rebuilds_its_pedal_state() {
    let (mut recorder, source, epoch) = recorder(limits(1, 16, 4, 2));
    let ticket = start(&mut recorder, &[source], epoch);
    for (nominal, published, bytes) in [
        (30, 30, [0x90, 60, 100]),
        (90, 90, [0x80, 60, 0]),
        (50, 100, [0x90, 61, 100]),
        (95, 110, [0x80, 61, 0]),
        (60, 115, [0x90, 62, 100]),
        (85, 116, [0xb0, 64, 127]),
    ] {
        let receipt = recorder
            .publish(
                source,
                CaptureStamp::exact_fixture(
                    epoch,
                    SampleTime::new(nominal),
                    SampleTime::new(published),
                )
                .unwrap(),
                Midi1Input::from_bytes(bytes).unwrap(),
                AuditionTrace::NotOffered,
            )
            .unwrap();
        assert_eq!(receipt.capture, CaptureDisposition::Recorded);
    }
    recorder
        .quiesce(source, epoch, SampleTime::new(75))
        .unwrap();
    let result = recorder.result(ticket).unwrap();
    assert_eq!(result.records().count(), 6);
    assert_eq!(result.selected_records().count(), 3);
    // A backwards cut can expose more historical open notes than H's reusable live slots.
    assert_eq!(result.closures().count(), 3);
    assert!(
        result
            .closures()
            .all(|closure| closure.time == SampleTime::new(75) && closure.pedal_held == Some(false))
    );
    assert!(!result.terminal_sources().next().unwrap().controls.pedals()[0]);
}

#[test]
fn quiescence_from_either_source_never_closes_an_outside_onset() {
    for two_sources in [false, true] {
        let (mut recorder, a, epoch) = recorder(limits(2, 16, 4, 2));
        let b = recorder
            .bind_fixture_source(Some(ControllerSnapshot::neutral()))
            .unwrap();
        let selected = if two_sources { &[a, b][..] } else { &[a][..] };
        let ticket = start(&mut recorder, selected, epoch);
        let note_source = if two_sources { b } else { a };
        let _receipt = publish(&mut recorder, note_source, epoch, 60, [0x90, 60, 100]);
        recorder.quiesce(a, epoch, SampleTime::new(30)).unwrap();
        if two_sources {
            recorder.quiesce(b, epoch, SampleTime::new(60)).unwrap();
        }
        let result = recorder.result(ticket).unwrap();
        assert_eq!(result.records().count(), 1);
        assert_eq!(result.selected_records().count(), 0);
        assert_eq!(result.closures().count(), 0);
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Interrupted);
    }
}

#[test]
fn physical_tracker_exhaustion_requires_rebind_and_transfers_retired_diagnostics() {
    let (mut recorder, source, epoch) = recorder(limits(1, 16, 2, 2));
    for time in [10, 20] {
        let _receipt = publish(&mut recorder, source, epoch, time, [0x90, 60, 1]);
    }
    let stamp =
        CaptureStamp::exact_fixture(epoch, SampleTime::new(30), SampleTime::new(30)).unwrap();
    assert_eq!(
        recorder.publish(
            source,
            stamp,
            Midi1Input::from_bytes([0x90, 61, 1]).unwrap(),
            AuditionTrace::NotOffered
        ),
        Err(NoteCaptureError::TrackerFull)
    );
    assert_eq!(
        recorder.arm(context(epoch, 2, 4), &[source]),
        Err(NoteCaptureError::Unsynchronized)
    );
    recorder
        .quiesce(source, epoch, SampleTime::new(30))
        .unwrap();
    let rebound = recorder
        .rebind_fixture_source(source, Some(ControllerSnapshot::neutral()))
        .unwrap();
    assert_eq!(rebound.retired_generation, source);
    assert_eq!(rebound.diagnostics.pairing_failures.as_u64(), 1);
    assert_ne!(rebound.generation, source);
    assert_eq!(
        recorder
            .source_sequence(rebound.generation)
            .unwrap()
            .as_u64(),
        0
    );
}

#[test]
fn preseal_late_input_faults_the_take_and_retains_source_time_through_rebind() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 4, 2));
    let ticket = start(&mut recorder, &[source], epoch);
    let _receipt = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100]);
    fence(&mut recorder, source, epoch, 50);
    let receipt = recorder
        .publish(
            source,
            CaptureStamp::exact_fixture(epoch, SampleTime::new(40), SampleTime::new(60)).unwrap(),
            Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
            AuditionTrace::NotOffered,
        )
        .unwrap();
    assert_eq!(receipt.capture, CaptureDisposition::Late);
    fence(&mut recorder, source, epoch, 60);
    let result = recorder.result(ticket).unwrap();
    assert_eq!(result.sealed_outcome(), CaptureOutcome::Partial);
    assert_eq!(result.records().count(), 1);
    assert_eq!(result.closures().count(), 1);
    assert!(!result.closures().next().unwrap().key_held);
    recorder
        .quiesce(source, epoch, SampleTime::new(60))
        .unwrap();
    let rebound = recorder
        .rebind_fixture_source(source, Some(ControllerSnapshot::neutral()))
        .unwrap();
    let late = rebound.diagnostics.first_late.unwrap();
    assert_eq!(late.source, source);
    assert_eq!(late.time, SampleTime::new(40));
}

#[test]
fn cancelling_countin_releases_the_unused_tracker_reserve_while_fences_are_pending() {
    let (mut recorder, source, epoch) = recorder(limits(2, 16, 4, 2));
    for time in [1, 2] {
        let _receipt = publish(&mut recorder, source, epoch, time, [0x90, 60, 1]);
    }
    let ticket = recorder.arm(context(epoch, 1, 5), &[source]).unwrap();
    recorder
        .stop(ticket, SampleTime::new(10), CaptureStopReason::Disarm)
        .unwrap();
    assert_eq!(
        publish(&mut recorder, source, epoch, 10, [0x90, 61, 1]).capture,
        CaptureDisposition::OutsideInterval
    );
    fence(&mut recorder, source, epoch, 10);
    assert_eq!(
        recorder.result(ticket).unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(
        recorder
            .source_diagnostics(source)
            .unwrap()
            .pairing_failures
            .as_u64(),
        0
    );
}

#[test]
fn late_quality_survives_full_tracker_and_protected_reserve_refusals() {
    for protected in [false, true] {
        let (mut recorder, source, epoch) = recorder(limits(1, 16, 2, 2));
        let first = start(&mut recorder, &[source], epoch);
        let _on = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100]);
        let _off = publish(&mut recorder, source, epoch, 40, [0x80, 60, 0]);
        fence(&mut recorder, source, epoch, 125);
        if protected {
            let _armed = recorder.arm(context(epoch, 8, 10), &[source]).unwrap();
        }
        let _on = publish(&mut recorder, source, epoch, 130, [0x90, 61, 1]);
        if !protected {
            let _on = publish(&mut recorder, source, epoch, 131, [0x90, 62, 1]);
        }
        let error = recorder.publish(
            source,
            CaptureStamp::exact_fixture(epoch, SampleTime::new(35), SampleTime::new(140)).unwrap(),
            Midi1Input::from_bytes([0x90, 63, 1]).unwrap(),
            AuditionTrace::NotOffered,
        );
        assert_eq!(
            error,
            Err(if protected {
                NoteCaptureError::TrackerReserved
            } else {
                NoteCaptureError::TrackerFull
            })
        );
        assert_eq!(
            recorder.result(first).unwrap().sealed_outcome(),
            CaptureOutcome::Complete
        );
        assert_eq!(
            recorder.result(first).unwrap().effective_outcome(),
            CaptureOutcome::Partial
        );
        assert_eq!(
            recorder
                .result(first)
                .unwrap()
                .quality()
                .late_count()
                .as_u64(),
            1
        );
        let diagnostics = recorder.source_diagnostics(source).unwrap();
        assert_eq!(diagnostics.late_refusals.as_u64(), 1);
        assert_eq!(diagnostics.first_late.unwrap().time, SampleTime::new(35));
    }
}

#[test]
fn refused_in_interval_release_and_pedal_update_closure_observations() {
    let (mut recorder, source, epoch) = recorder(limits(2, 2, 4, 2));
    let ticket = start(&mut recorder, &[source], epoch);
    let first = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100])
        .occurrence
        .unwrap();
    let second = publish(&mut recorder, source, epoch, 31, [0x90, 61, 100])
        .occurrence
        .unwrap();
    for (nominal, published, bytes) in [
        (32, 40, [0x80, 60, 0]),
        (35, 41, [0xb0, 64, 127]),
        (40, 42, [0xb0, 64, 0]),
    ] {
        let receipt = recorder
            .publish(
                source,
                CaptureStamp::exact_fixture(
                    epoch,
                    SampleTime::new(nominal),
                    SampleTime::new(published),
                )
                .unwrap(),
                Midi1Input::from_bytes(bytes).unwrap(),
                AuditionTrace::NotOffered,
            )
            .unwrap();
        assert_eq!(
            receipt.capture,
            CaptureDisposition::Stopped(CaptureStopReason::Capacity)
        );
    }
    fence(&mut recorder, source, epoch, 42);
    let result = recorder.result(ticket).unwrap();
    assert_eq!(result.records().count(), 2);
    assert!(
        !result
            .closures()
            .find(|closure| closure.occurrence == first)
            .unwrap()
            .key_held
    );
    assert!(
        result
            .closures()
            .find(|closure| closure.occurrence == second)
            .unwrap()
            .key_held
    );
    assert!(
        result
            .closures()
            .all(|closure| closure.pedal_held == Some(true))
    );
    let terminal = result.terminal_sources().next().unwrap();
    assert!(!terminal.controls.pedals()[0]); // Accepted capture log is still unchanged.
    assert_eq!(
        terminal.observed_pedal(Midi1Input::from_bytes([0xb0, 64, 0]).unwrap().channel()),
        Some(true)
    );
}

#[test]
fn backwards_cut_through_refused_pedal_history_is_explicitly_unknown() {
    for (cut, expected) in [(35, Some(false)), (45, None), (55, Some(false))] {
        let (mut recorder, source, epoch) = recorder(limits(1, 1, 2, 2));
        let ticket = start(&mut recorder, &[source], epoch);
        let _on = publish(&mut recorder, source, epoch, 30, [0x90, 60, 100]);
        for (nominal, published, value) in [(40, 60, 127), (50, 61, 0)] {
            let _receipt = recorder
                .publish(
                    source,
                    CaptureStamp::exact_fixture(
                        epoch,
                        SampleTime::new(nominal),
                        SampleTime::new(published),
                    )
                    .unwrap(),
                    Midi1Input::from_bytes([0xb0, 64, value]).unwrap(),
                    AuditionTrace::NotOffered,
                )
                .unwrap();
        }
        recorder
            .quiesce(source, epoch, SampleTime::new(cut))
            .unwrap();
        let result = recorder.result(ticket).unwrap();
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Interrupted);
        assert_eq!(result.closures().next().unwrap().pedal_held, expected);
        assert_eq!(result.records().count(), 1);
    }
}

#[test]
fn invalid_pairing_keeps_late_quality_custody_until_source_quiescence() {
    let (mut recorder, source, epoch) = recorder(limits(1, 16, 2, 2));
    let first = start(&mut recorder, &[source], epoch);
    fence(&mut recorder, source, epoch, 125);
    let second = recorder.arm(context(epoch, 6, 10), &[source]).unwrap();
    fence(&mut recorder, source, epoch, 150);
    recorder.start(second).unwrap();
    fence(&mut recorder, source, epoch, 250);
    for time in [260, 261] {
        let _on = publish(&mut recorder, source, epoch, time, [0x90, 60, 1]);
    }
    let input = Midi1Input::from_bytes([0x90, 61, 1]).unwrap();
    assert_eq!(
        recorder.publish(
            source,
            CaptureStamp::exact_fixture(epoch, SampleTime::new(262), SampleTime::new(262)).unwrap(),
            input,
            AuditionTrace::NotOffered
        ),
        Err(NoteCaptureError::TrackerFull)
    );
    for (ticket, nominal, published) in [(first, 35, 270), (second, 175, 271)] {
        assert_eq!(
            recorder.publish(
                source,
                CaptureStamp::exact_fixture(
                    epoch,
                    SampleTime::new(nominal),
                    SampleTime::new(published)
                )
                .unwrap(),
                input,
                AuditionTrace::NotOffered
            ),
            Err(NoteCaptureError::Unsynchronized)
        );
        assert_eq!(
            recorder.result(ticket).unwrap().effective_outcome(),
            CaptureOutcome::Partial
        );
    }
    assert_eq!(
        recorder
            .source_diagnostics(source)
            .unwrap()
            .late_refusals
            .as_u64(),
        2
    );
}

#[test]
fn retry_after_start_required_attributes_a_late_input_exactly_once() {
    for same_source in [false, true] {
        let (mut recorder, old_source, epoch) = recorder(limits(2, 16, 4, 2));
        let old = start(&mut recorder, &[old_source], epoch);
        let _on = publish(&mut recorder, old_source, epoch, 30, [0x90, 60, 1]);
        fence(&mut recorder, old_source, epoch, 125);
        let source = if same_source {
            old_source
        } else {
            recorder
                .bind_fixture_source(Some(ControllerSnapshot::neutral()))
                .unwrap()
        };
        let next = recorder.arm(context(epoch, 6, 10), &[source]).unwrap();
        let sequence = recorder.source_sequence(old_source).unwrap();
        let stamp =
            CaptureStamp::exact_fixture(epoch, SampleTime::new(35), SampleTime::new(150)).unwrap();
        let input = Midi1Input::from_bytes([0x90, 61, 1]).unwrap();
        for _ in 0..2 {
            assert_eq!(
                recorder.publish(old_source, stamp, input, AuditionTrace::NotOffered),
                Err(NoteCaptureError::StartRequired)
            );
            assert_eq!(recorder.source_sequence(old_source).unwrap(), sequence);
            assert_eq!(
                recorder
                    .result(old)
                    .unwrap()
                    .quality()
                    .late_count()
                    .as_u64(),
                0
            );
            assert_eq!(
                recorder
                    .source_diagnostics(old_source)
                    .unwrap()
                    .late_refusals
                    .as_u64(),
                0
            );
        }
        fence(&mut recorder, source, epoch, 150);
        recorder.start(next).unwrap();
        assert_eq!(
            recorder
                .publish(old_source, stamp, input, AuditionTrace::NotOffered)
                .unwrap()
                .capture,
            CaptureDisposition::Late
        );
        assert_eq!(
            recorder
                .result(old)
                .unwrap()
                .quality()
                .late_count()
                .as_u64(),
            1
        );
        assert_eq!(
            recorder
                .source_diagnostics(old_source)
                .unwrap()
                .late_refusals
                .as_u64(),
            1
        );
    }
}
