use super::*;
use crate::profile::{CaptureLimits, CaptureLimitsInput, RecordingLimits};
use crate::quantities::{
    CapturePassCount, CaptureResultCount, CaptureSourceCount, EventCount, HeldNoteCount,
    SampleRate, TrackedInputNoteCount,
};
use crate::recording::notes::*;
use crate::tempo::{Bpm, TempoChange, TempoMap};
use crate::time::{FrameCount, StreamAnchor, StreamEpoch, issue_epoch};

fn limits(bytes: u64, ticks: u64) -> RecordingLimits {
    RecordingLimits::new(
        HeldNoteCount::limit(8).unwrap(),
        EventCount::limit(64).unwrap(),
    )
    .unwrap()
    .with_capture(
        CaptureLimits::new(CaptureLimitsInput {
            max_tracked_input_notes: TrackedInputNoteCount::limit(16).unwrap(),
            max_capture_sources: CaptureSourceCount::limit(2).unwrap(),
            max_capture_passes: CapturePassCount::limit(1).unwrap(),
            max_pending_capture_results: CaptureResultCount::limit(2).unwrap(),
            max_capture_bytes: PreparedBytes::limit(bytes).unwrap(),
            max_audio_capture_frames: FrameCount::new(1),
            max_projection_ticks: ProjectionTickCount::limit(ticks).unwrap(),
            capture_lateness_allowance: FrameCount::ZERO,
        })
        .unwrap(),
    )
    .unwrap()
}
fn input(epoch: StreamEpoch, start: u64, end: u64) -> NoteArmInput {
    NoteArmInput {
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
    }
}
fn setup_with(
    input: NoteArmInput,
    limits: RecordingLimits,
    controls: ControllerSnapshot,
) -> (SimulatedNoteRecorder, ConnectionGeneration, TakeReservation) {
    let mut recorder = SimulatedNoteRecorder::prepare_fixture(input.epoch, limits).unwrap();
    let source = recorder.bind_fixture_source(Some(controls)).unwrap();
    let context = NoteArmContext::prepare(input).unwrap();
    let begin = context.window().start().as_u64();
    let ticket = recorder.arm(context, &[source]).unwrap();
    fence(&mut recorder, source, begin);
    recorder.start(ticket).unwrap();
    (recorder, source, ticket)
}
fn setup() -> (SimulatedNoteRecorder, ConnectionGeneration, TakeReservation) {
    setup_with(
        input(issue_epoch().unwrap(), 1, 20),
        limits(1_048_576, 4096),
        ControllerSnapshot::neutral(),
    )
}
fn fence(recorder: &mut SimulatedNoteRecorder, source: ConnectionGeneration, at: u64) {
    recorder
        .fence(
            source,
            recorder.epoch,
            SampleTime::new(at),
            recorder.source_sequence(source).unwrap(),
        )
        .unwrap();
}
fn publish(
    recorder: &mut SimulatedNoteRecorder,
    source: ConnectionGeneration,
    nominal: u64,
    published: u64,
    bytes: [u8; 3],
    audition: AuditionTrace,
) -> PublicationReceipt {
    recorder
        .publish(
            source,
            CaptureStamp::exact_fixture(
                recorder.epoch,
                SampleTime::new(nominal),
                SampleTime::new(published),
            )
            .unwrap(),
            Midi1Input::from_bytes(bytes).unwrap(),
            audition,
        )
        .unwrap()
}
fn event(
    recorder: &mut SimulatedNoteRecorder,
    source: ConnectionGeneration,
    at: u64,
    bytes: [u8; 3],
) -> PublicationReceipt {
    publish(recorder, source, at, at, bytes, AuditionTrace::NotOffered)
}

#[test]
fn real_step_and_period_ramp_tables_match_exhaustive_oracle_at_every_frame() {
    for rate in [8_000.0, 48_000.0] {
        for bpm in [120.0, 1_000_000.0] {
            let mut input = input(issue_epoch().unwrap(), 3, 100);
            input.tempo = TempoMap::new(
                Bpm::new(bpm).unwrap(),
                &[
                    TempoChange::step(MusicalTick::new(17), Bpm::new(bpm * 0.7).unwrap()),
                    TempoChange::ramp(MusicalTick::new(29), Bpm::new(bpm * 0.7).unwrap()),
                    TempoChange::step(MusicalTick::new(73), Bpm::new(bpm * 1.6).unwrap()),
                ],
                SampleRate::new(rate).unwrap(),
            )
            .unwrap();
            let context = NoteArmContext::prepare(input).unwrap();
            let count =
                ProjectionTable::count(context.interval(), ProjectionTickCount::limit(98).unwrap())
                    .unwrap();
            let table = ProjectionTable::prepare(
                &context,
                count,
                AllocationBudget {
                    occupied: PreparedBytes::NONE,
                    limit: PreparedBytes::measured(1_048_576),
                },
            )
            .unwrap();
            let oracle: Vec<_> = (3..=100)
                .map(|tick| {
                    let tick = MusicalTick::new(tick);
                    (tick, context.tempo().position_of(tick).unwrap())
                })
                .collect();
            for frame in context.window().start().as_u64()..=context.window().end().as_u64() {
                let expected = oracle
                    .iter()
                    .min_by_key(|(tick, at)| (at.as_u64().abs_diff(frame), tick.as_u64()))
                    .unwrap();
                let actual = table.lookup_time(&context, SampleTime::new(frame)).unwrap();
                assert_eq!(actual.tick(), expected.0);
                assert_eq!(
                    actual.error(),
                    expected.1.difference(PlanPosition::new(frame)).unwrap()
                );
            }
        }
    }
}

#[test]
fn projection_uses_fifo_key_lifetimes_and_original_time_across_delivery_partitions() {
    let mut control = None;
    for partition in [1, 39, 64, 256, 1000] {
        for audition in [
            AuditionTrace::NotOffered,
            AuditionTrace::Executed(SampleTime::new(999)),
        ] {
            let (mut recorder, source, ticket) = setup();
            let mut boundary = 25 + partition;
            for (time, bytes) in [
                (31, [0x90, 60, 100]),
                (64, [0x90, 60, 90]),
                (103, [0x80, 60, 0]),
                (176, [0x90, 60, 0]),
            ] {
                while boundary <= time {
                    fence(&mut recorder, source, boundary);
                    boundary += partition;
                }
                let _receipt = publish(&mut recorder, source, time, time, bytes, audition);
            }
            fence(&mut recorder, source, 500);
            let projection = recorder.project_notes(ticket).unwrap();
            let actual: Vec<_> = projection
                .notes()
                .iter()
                .map(|n| {
                    (
                        n.occurrence().serial(),
                        n.start(),
                        n.end(),
                        n.onset().error(),
                        n.release().error(),
                        n.velocity(),
                    )
                })
                .collect();
            assert_eq!(actual[0].1, MusicalTick::new(1));
            assert_eq!(actual[0].2, MusicalTick::new(4));
            assert_eq!(actual[1].1, MusicalTick::new(3));
            assert_eq!(actual[1].2, MusicalTick::new(7));
            if let Some(ref expected) = control {
                assert_eq!(&actual, expected);
            } else {
                control = Some(actual);
            }
            assert_eq!(projection.raw().records().count(), 4);
        }
    }
}

#[test]
fn ordered_stop_reports_synthetic_ending_and_excludes_later_onsets() {
    let (mut recorder, source, ticket) = setup();
    let kept = event(&mut recorder, source, 50, [0x90, 60, 100])
        .occurrence
        .unwrap();
    recorder
        .stop(ticket, SampleTime::new(150), CaptureStopReason::Stop)
        .unwrap();
    let _off = event(&mut recorder, source, 200, [0x80, 60, 0]);
    let _later = event(&mut recorder, source, 225, [0x90, 61, 100]);
    fence(&mut recorder, source, 250);
    let projection = recorder.project_notes(ticket).unwrap();
    assert_eq!(projection.raw().records().count(), 1);
    assert_eq!(projection.notes().len(), 1);
    let note = projection.notes()[0];
    assert_eq!(note.occurrence(), kept);
    assert_eq!(note.end(), MusicalTick::new(6));
    assert_eq!(
        note.ending(),
        ProjectedEnding::Synthetic {
            reason: CaptureStopReason::Stop,
            key_held: true,
            pedal_held: Some(false)
        }
    );
}

#[test]
fn nonpositive_raw_or_projected_lifetime_refuses_whole_take_and_names_occurrence() {
    for (on, off) in [(100, 20), (100, 90), (100, 100), (101, 102)] {
        let (mut recorder, source, ticket) = setup();
        let occurrence = event(&mut recorder, source, on, [0x90, 60, 100])
            .occurrence
            .unwrap();
        let _off = publish(
            &mut recorder,
            source,
            off,
            on.max(off),
            [0x80, 60, 0],
            AuditionTrace::NotOffered,
        );
        fence(&mut recorder, source, 500);
        assert!(
            matches!(recorder.project_notes(ticket), Err(ProjectionError::InvalidLifetime { occurrence: found }) if found == occurrence)
        );
        assert_eq!(
            recorder.result(ticket).unwrap().records().count(),
            if off < 25 { 1 } else { 2 }
        );
    }
}

#[test]
fn quantization_preserves_duration_and_native_errors_and_refuses_outside_target() {
    for (start, end, on, off, expected) in [
        (0, 20, 75, 125, Some((2, 4))),
        (1, 20, 25, 75, None),
        (0, 5, 100, 124, Some((4, 5))),
        (0, 5, 88, 124, Some((4, 5))),
    ] {
        let mut arm = input(issue_epoch().unwrap(), start, end);
        arm.quantization =
            CaptureQuantization::Grid(QuantizationGrid::new(MusicalTick::new(2)).unwrap());
        let (mut recorder, source, ticket) =
            setup_with(arm, limits(1_048_576, 4096), ControllerSnapshot::neutral());
        let occurrence = event(&mut recorder, source, on, [0x90, 60, 100])
            .occurrence
            .unwrap();
        let _off = event(&mut recorder, source, off, [0x80, 60, 0]);
        fence(&mut recorder, source, end * 25);
        if let Some((a, b)) = expected {
            let projection = recorder.project_notes(ticket).unwrap();
            let note = projection.notes()[0];
            assert_eq!((note.start().as_u64(), note.end().as_u64()), (a, b));
            assert_eq!(
                b - a,
                note.release().tick().as_u64() - note.onset().tick().as_u64()
            );
            assert_eq!(
                note.onset().error(),
                PlanPosition::new(note.onset().tick().as_u64() * 25)
                    .difference(PlanPosition::new(on))
                    .unwrap()
            );
            assert_eq!(
                projection
                    .raw()
                    .records()
                    .next()
                    .unwrap()
                    .stamp()
                    .nominal()
                    .as_u64(),
                on
            );
        } else {
            assert!(
                matches!(recorder.project_notes(ticket), Err(ProjectionError::OutsideTarget { occurrence: found }) if found == occurrence)
            );
        }
    }
    assert_eq!(
        snap(
            MusicalTick::new(u64::MAX),
            CaptureQuantization::Grid(QuantizationGrid::new(MusicalTick::new(2)).unwrap())
        ),
        Some(MusicalTick::new(u64::MAX - 1))
    );
}

#[test]
fn controllers_and_incomplete_quality_cannot_silently_become_plain_notes() {
    for bytes in [[0xb0, 64, 127], [0xe0, 0, 65]] {
        let (mut recorder, source, ticket) = setup();
        let _controller = event(&mut recorder, source, 50, bytes);
        fence(&mut recorder, source, 500);
        assert!(matches!(
            recorder.project_notes(ticket),
            Err(ProjectionError::UnsupportedControllers { .. })
        ));
        let controls = ControllerSnapshot::neutral()
            .with_input(Midi1Input::from_bytes(bytes).unwrap())
            .unwrap();
        let (mut recorder, source, ticket) = setup_with(
            input(issue_epoch().unwrap(), 1, 20),
            limits(1_048_576, 4096),
            controls,
        );
        fence(&mut recorder, source, 500);
        assert!(matches!(
            recorder.project_notes(ticket),
            Err(ProjectionError::UnsupportedControllers { .. })
        ));
    }
    let (mut recorder, source, ticket) = setup();
    let _on = event(&mut recorder, source, 50, [0x90, 60, 100]);
    fence(&mut recorder, source, 500);
    assert!(recorder.project_notes(ticket).is_ok());
    let _late = publish(
        &mut recorder,
        source,
        60,
        501,
        [0x80, 60, 0],
        AuditionTrace::NotOffered,
    );
    assert!(matches!(
        recorder.project_notes(ticket),
        Err(ProjectionError::Incomplete {
            outcome: CaptureOutcome::Partial
        })
    ));
    assert_eq!(
        recorder.result(ticket).unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
}

#[test]
fn lookup_refuses_foreign_session_epoch_and_outside_mapping_and_keeps_retained_context() {
    let (mut recorder, source, ticket) = setup();
    let session = recorder.session();
    let epoch = recorder.epoch;
    fence(&mut recorder, source, 500);
    let other = SimulatedNoteRecorder::prepare_fixture(epoch, limits(1_048_576, 4096)).unwrap();
    let projection = recorder.project_notes(ticket).unwrap();
    let stamp =
        |ep, at| CaptureStamp::exact_fixture(ep, SampleTime::new(at), SampleTime::new(at)).unwrap();
    assert_eq!(
        projection.project_stamp(other.session(), stamp(epoch, 100)),
        Err(ProjectionError::ForeignContext)
    );
    assert_eq!(
        projection.project_stamp(session, stamp(issue_epoch().unwrap(), 100)),
        Err(ProjectionError::ForeignContext)
    );
    for at in [0, 24, 501, u64::MAX] {
        assert!(projection.project_stamp(session, stamp(epoch, at)).is_err());
    }
    assert_eq!(
        projection
            .project_stamp(session, stamp(epoch, 100))
            .unwrap()
            .tick(),
        MusicalTick::new(4)
    );
    drop(projection);
    let mut next = input(epoch, 30, 40);
    next.expected_revision = FixtureRevision::new(99);
    next.tempo = TempoMap::new(
        Bpm::new(60.0).unwrap(),
        &[],
        SampleRate::new(96_000.0).unwrap(),
    )
    .unwrap();
    let next = recorder
        .arm(NoteArmContext::prepare(next).unwrap(), &[source])
        .unwrap();
    fence(&mut recorder, source, 3000);
    recorder.start(next).unwrap();
    fence(&mut recorder, source, 4000);
    assert_eq!(
        recorder
            .project_notes(ticket)
            .unwrap()
            .project_stamp(session, stamp(epoch, 100))
            .unwrap()
            .tick(),
        MusicalTick::new(4)
    );
    let projection = recorder.project_notes(next).unwrap();
    assert_eq!(
        projection.raw().context().expected_revision(),
        FixtureRevision::new(99)
    );
    assert_eq!(
        projection
            .project_stamp(session, stamp(epoch, 3100))
            .unwrap()
            .tick(),
        MusicalTick::new(31)
    );
}

#[test]
fn checked_tick_and_aggregate_byte_admission_preserve_capture_on_failure() {
    let interval = MusicalInterval::new(MusicalTick::ZERO, MusicalTick::new(u64::MAX)).unwrap();
    assert!(matches!(
        ProjectionTable::count(interval, ProjectionTickCount::limit(u64::MAX).unwrap()),
        Err(ProjectionError::IntervalOverflow)
    ));
    let (mut recorder, source, ticket) = setup();
    let _on = event(&mut recorder, source, 50, [0x90, 60, 100]);
    fence(&mut recorder, source, 500);
    let before = recorder.bytes_reserved();
    let projection = recorder.project_notes(ticket).unwrap();
    let actual = before.get()
        + size_of_val(&projection) as u64
        + super::super::array_bytes::<PlanPosition>(20).unwrap()
        + size_of_val(projection.notes()) as u64;
    assert_eq!(projection.bytes_reserved().get(), actual);
    drop(projection);
    recorder.limits = limits(actual - 1, 20);
    assert!(matches!(
        recorder.project_notes(ticket),
        Err(ProjectionError::Capture(NoteCaptureError::Storage(
            CaptureError::ByteBudget { .. }
        )))
    ));
    recorder.limits = limits(actual, 19);
    assert!(matches!(
        recorder.project_notes(ticket),
        Err(ProjectionError::TickBudget { .. })
    ));
    recorder.limits = limits(actual, 20);
    assert_eq!(recorder.project_notes(ticket).unwrap().notes().len(), 1);
    assert_eq!(recorder.bytes_reserved(), before);
    assert_eq!(recorder.result(ticket).unwrap().records().count(), 1);
}

#[test]
fn anchored_projection_maps_engine_time_back_to_retained_plan_position() {
    let epoch = issue_epoch().unwrap();
    let mut arm = input(epoch, 10, 20);
    arm.anchor = StreamAnchor::new(SampleTime::new(10_000), PlanPosition::new(250));
    let (mut recorder, source, ticket) =
        setup_with(arm, limits(1_048_576, 4096), ControllerSnapshot::neutral());
    let _on = event(&mut recorder, source, 10_026, [0x90, 60, 100]);
    let _off = event(&mut recorder, source, 10_126, [0x80, 60, 0]);
    fence(&mut recorder, source, 10_250);
    let session = recorder.session();
    let projection = recorder.project_notes(ticket).unwrap();
    assert_eq!(
        (projection.notes()[0].start(), projection.notes()[0].end()),
        (MusicalTick::new(11), MusicalTick::new(15))
    );
    assert_eq!(projection.notes()[0].onset().error(), FrameDelta::new(-1));
    let stamp =
        CaptureStamp::exact_fixture(epoch, SampleTime::new(9999), SampleTime::new(9999)).unwrap();
    assert!(matches!(
        projection.project_stamp(session, stamp),
        Err(ProjectionError::OutsideMapping { .. })
    ));
}

#[test]
fn upper_quantization_overrun_refuses_instead_of_truncating() {
    let mut arm = input(issue_epoch().unwrap(), 0, 6);
    arm.quantization =
        CaptureQuantization::Grid(QuantizationGrid::new(MusicalTick::new(4)).unwrap());
    let (mut recorder, source, ticket) =
        setup_with(arm, limits(1_048_576, 4096), ControllerSnapshot::neutral());
    let occurrence = event(&mut recorder, source, 75, [0x90, 60, 100])
        .occurrence
        .unwrap();
    fence(&mut recorder, source, 150);
    assert!(
        matches!(recorder.project_notes(ticket), Err(ProjectionError::OutsideTarget { occurrence: found }) if found == occurrence)
    );
    assert_eq!(recorder.result(ticket).unwrap().records().count(), 1);
}

#[test]
fn interruption_and_unsealed_takes_refuse_and_keep_raw_input() {
    let (mut recorder, source, ticket) = setup();
    assert!(matches!(
        recorder.project_notes(ticket),
        Err(ProjectionError::Capture(NoteCaptureError::Storage(
            CaptureError::NotSealed
        )))
    ));
    let _on = event(&mut recorder, source, 50, [0x90, 60, 100]);
    let _off = event(&mut recorder, source, 200, [0x80, 60, 0]);
    let _later = event(&mut recorder, source, 225, [0x90, 61, 100]);
    recorder
        .quiesce(source, recorder.epoch, SampleTime::new(150))
        .unwrap();
    assert!(matches!(
        recorder.project_notes(ticket),
        Err(ProjectionError::Incomplete {
            outcome: CaptureOutcome::Interrupted
        })
    ));
    assert_eq!(recorder.result(ticket).unwrap().records().count(), 3);
    assert_eq!(
        recorder.result(ticket).unwrap().selected_records().count(),
        1
    );
}

#[test]
fn empty_cancelled_count_in_ignores_unused_initial_controllers() {
    let epoch = issue_epoch().unwrap();
    let mut recorder =
        SimulatedNoteRecorder::prepare_fixture(epoch, limits(1_048_576, 4096)).unwrap();
    let controls = ControllerSnapshot::neutral()
        .with_input(Midi1Input::from_bytes([0xb0, 64, 127]).unwrap())
        .unwrap();
    let source = recorder.bind_fixture_source(Some(controls)).unwrap();
    let ticket = recorder
        .arm(
            NoteArmContext::prepare(input(epoch, 10, 20)).unwrap(),
            &[source],
        )
        .unwrap();
    recorder
        .stop(ticket, SampleTime::new(100), CaptureStopReason::Disarm)
        .unwrap();
    fence(&mut recorder, source, 100);
    let projection = recorder.project_notes(ticket).unwrap();
    assert!(projection.notes().is_empty());
    assert_eq!(
        projection.raw().window().start(),
        projection.raw().window().end()
    );
    assert_eq!(
        projection.raw().initial_sources().next().unwrap().controls,
        controls
    );
}

#[test]
fn table_and_note_capacity_checks_refuse_overgrant_and_layout_overflow() {
    fn check<T>() {
        let budget = AllocationBudget {
            occupied: PreparedBytes::measured(1234),
            limit: PreparedBytes::measured(1234 + 2 * size_of::<T>() as u64),
        };
        assert!(budget.check::<T>(2).is_ok());
        assert!(matches!(
            budget.check::<T>(3),
            Err(ProjectionError::Storage(CaptureError::ByteBudget { .. }))
        ));
        assert!(budget.reserve::<T>(3).is_err());
        assert!(budget.check::<T>(usize::MAX).is_err());
    }
    check::<PlanPosition>();
    check::<ProjectedNote>();
}

#[test]
fn increasing_raw_lifetime_that_rounds_to_one_tick_refuses_after_a_valid_note() {
    let (mut recorder, source, ticket) = setup();
    let _first = event(&mut recorder, source, 30, [0x90, 60, 100]);
    let _release = event(&mut recorder, source, 70, [0x80, 60, 0]);
    let occurrence = event(&mut recorder, source, 101, [0x90, 61, 100])
        .occurrence
        .unwrap();
    let _release = event(&mut recorder, source, 102, [0x80, 61, 0]);
    fence(&mut recorder, source, 500);
    // 101 < 102, so both raw-lifetime guards pass; both endpoints choose tick 4 (frame 100).
    assert!(
        matches!(recorder.project_notes(ticket), Err(ProjectionError::InvalidLifetime { occurrence: found }) if found == occurrence)
    );
    assert_eq!(recorder.result(ticket).unwrap().records().count(), 4);
}
