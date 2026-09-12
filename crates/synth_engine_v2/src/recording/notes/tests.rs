use super::*;
use crate::profile::{CaptureLimits, CaptureLimitsInput};
use crate::quantities::{
    CapturePassCount, CaptureResultCount, CaptureSourceCount, EventCount, HeldNoteCount,
    ProjectionTickCount, SampleRate, TrackedInputNoteCount,
};
use crate::tempo::Bpm;
use crate::time::{FrameCount, PlanPosition, issue_epoch};

pub(super) fn limits(bytes: u64) -> RecordingLimits {
    RecordingLimits::new(
        HeldNoteCount::limit(2).unwrap(),
        EventCount::limit(4).unwrap(),
    )
    .unwrap()
    .with_capture(
        CaptureLimits::new(CaptureLimitsInput {
            max_tracked_input_notes: TrackedInputNoteCount::limit(4).unwrap(),
            max_capture_sources: CaptureSourceCount::limit(1).unwrap(),
            max_capture_passes: CapturePassCount::limit(1).unwrap(),
            max_pending_capture_results: CaptureResultCount::limit(2).unwrap(),
            max_capture_bytes: PreparedBytes::limit(bytes).unwrap(),
            max_audio_capture_frames: FrameCount::new(1),
            max_projection_ticks: ProjectionTickCount::limit(961).unwrap(),
            capture_lateness_allowance: FrameCount::ZERO,
        })
        .unwrap(),
    )
    .unwrap()
}

pub(super) fn context(epoch: StreamEpoch) -> NoteArmContext {
    NoteArmContext::prepare(context_input(epoch)).unwrap()
}

pub(super) fn context_input(epoch: StreamEpoch) -> NoteArmInput {
    NoteArmInput {
        target: FixtureTargetId::new(1).unwrap(),
        expected_revision: FixtureRevision::new(0),
        interval: MusicalInterval::new(MusicalTick::new(1), MusicalTick::new(5)).unwrap(),
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

fn setup() -> (SimulatedNoteRecorder, ConnectionGeneration, TakeReservation) {
    let epoch = issue_epoch().unwrap();
    let mut recorder = SimulatedNoteRecorder::prepare_fixture(epoch, limits(1_048_576)).unwrap();
    let source = recorder
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    let ticket = recorder.arm(context(epoch), &[source]).unwrap();
    (recorder, source, ticket)
}

pub(super) fn fence(recorder: &mut SimulatedNoteRecorder, source: ConnectionGeneration, time: u64) {
    recorder
        .fence(
            source,
            recorder.epoch,
            SampleTime::new(time),
            recorder.source_sequence(source).unwrap(),
        )
        .unwrap();
}

pub(super) fn publish(
    recorder: &mut SimulatedNoteRecorder,
    source: ConnectionGeneration,
    time: u64,
    bytes: [u8; 3],
) -> Result<PublicationReceipt, NoteCaptureError> {
    recorder.publish(
        source,
        CaptureStamp::exact_fixture(recorder.epoch, SampleTime::new(time), SampleTime::new(time))
            .unwrap(),
        Midi1Input::from_bytes(bytes).unwrap(),
        AuditionTrace::NotOffered,
    )
}

#[test]
fn ordered_start_pairing_overflow_seal_and_late_fault_have_no_allocator_activity() {
    let (mut recorder, source, ticket) = setup();
    let count = crate::render_allocation::count_allocs(|| {
        fence(&mut recorder, source, 25);
        recorder.start(ticket).unwrap();
        assert!(publish(&mut recorder, source, 26, [0x90, 60, 100]).is_ok());
        assert!(publish(&mut recorder, source, 27, [0xb0, 64, 127]).is_ok());
        assert!(publish(&mut recorder, source, 28, [0x90, 61, 100]).is_ok());
        assert!(publish(&mut recorder, source, 29, [0xe0, 0, 65]).is_ok());
        assert_eq!(
            publish(&mut recorder, source, 30, [0x80, 60, 0])
                .unwrap()
                .capture,
            CaptureDisposition::Stopped(CaptureStopReason::Capacity)
        );
        let refused = recorder
            .publish(
                source,
                CaptureStamp::exact_fixture(
                    recorder.epoch,
                    SampleTime::new(29),
                    SampleTime::new(31),
                )
                .unwrap(),
                Midi1Input::from_bytes([0xb0, 64, 0]).unwrap(),
                AuditionTrace::NotOffered,
            )
            .unwrap();
        assert_eq!(
            refused.capture,
            CaptureDisposition::Stopped(CaptureStopReason::Capacity)
        );
        fence(&mut recorder, source, 31);
        assert_eq!(recorder.result(ticket).unwrap().closures().count(), 2);
        assert!(
            recorder
                .result(ticket)
                .unwrap()
                .closures()
                .all(|closure| closure.pedal_held == Some(false))
        );
        let receipt = recorder
            .publish(
                source,
                CaptureStamp::exact_fixture(
                    recorder.epoch,
                    SampleTime::new(27),
                    SampleTime::new(31),
                )
                .unwrap(),
                Midi1Input::from_bytes([0x80, 61, 0]).unwrap(),
                AuditionTrace::NotOffered,
            )
            .unwrap();
        assert_eq!(receipt.capture, CaptureDisposition::Late);
        recorder
            .quiesce(source, recorder.epoch, SampleTime::new(31))
            .unwrap();
    });
    assert_eq!(count, 0);
    assert_eq!(
        recorder
            .result(ticket)
            .unwrap()
            .quality()
            .late_count()
            .as_u64(),
        1
    );
}

#[test]
fn aggregate_budget_covers_owned_layout_and_retained_maps_at_exact_boundary() {
    let epoch = issue_epoch().unwrap();
    let mut recorder = SimulatedNoteRecorder::prepare_fixture(epoch, limits(1_048_576)).unwrap();
    let mut actual = size_of_val(&recorder)
        + size_of_val(&*recorder.sources)
        + size_of_val(&*recorder.tracker)
        + size_of_val(&*recorder.contexts)
        + size_of_val(&*recorder.store.slots);
    for slot in &recorder.store.slots {
        actual += size_of_val(&*slot.data) + size_of_val(&*slot.sources);
    }
    let base = recorder.bytes_reserved().get();
    assert_eq!(base, actual as u64);
    assert!(matches!(
        SimulatedNoteRecorder::prepare_fixture(epoch, limits(base - 1)),
        Err(NoteCaptureError::Storage(CaptureError::ByteBudget { .. }))
    ));
    let mut exact = SimulatedNoteRecorder::prepare_fixture(epoch, limits(base)).unwrap();
    let source = exact
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    assert!(matches!(
        exact.arm(context(epoch), &[source]),
        Err(NoteCaptureError::Storage(CaptureError::ByteBudget { .. }))
    ));
    assert!(!exact.is_active());
    assert!(exact.store.slots.iter().all(|slot| slot.state.is_none()));
    let map_bytes = context(epoch).tempo().bytes_held() as u64;
    recorder = SimulatedNoteRecorder::prepare_fixture(epoch, limits(base + map_bytes)).unwrap();
    let source = recorder
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    let ticket = recorder.arm(context(epoch), &[source]).unwrap();
    assert_eq!(recorder.bytes_reserved().get(), base + map_bytes);
    recorder.quiesce(source, epoch, SampleTime::ZERO).unwrap();
    let quality = recorder.result(ticket).unwrap().quality();
    recorder.discard(ticket, quality).unwrap();
    assert_eq!(recorder.bytes_reserved().get(), base);
}

#[test]
fn exhausted_session_pass_sequence_and_occurrence_ids_do_not_wrap() {
    let counter = AtomicU64::new(u64::MAX - 1);
    assert_eq!(issue_session(&counter).unwrap().as_u64(), u64::MAX);
    for _ in 0..2 {
        assert_eq!(
            issue_session(&counter),
            Err(NoteCaptureError::IdentityExhausted)
        );
    }
    for exhaust_sequence in [false, true] {
        let (mut recorder, source, ticket) = setup();
        fence(&mut recorder, source, 25);
        recorder.start(ticket).unwrap();
        let state = recorder.sources[0].as_mut().unwrap();
        if exhaust_sequence {
            state.sequence = PublicationSequence(u64::MAX);
        } else {
            state.last_occurrence = Some(PerformedOccurrenceId {
                source,
                serial: u64::MAX,
            });
        }
        assert_eq!(
            publish(&mut recorder, source, 26, [0x90, 60, 1]),
            Err(NoteCaptureError::IdentityExhausted)
        );
        assert_eq!(
            publish(&mut recorder, source, 27, [0x80, 60, 0]),
            Err(NoteCaptureError::Unsynchronized)
        );
        assert!(recorder.tracker.iter().all(Option::is_none));
        recorder
            .quiesce(source, recorder.epoch, SampleTime::new(26))
            .unwrap();
        assert_eq!(
            recorder.result(ticket).unwrap().sealed_outcome(),
            CaptureOutcome::Interrupted
        );
    }
    let epoch = issue_epoch().unwrap();
    let mut recorder = SimulatedNoteRecorder::prepare_fixture(epoch, limits(1_048_576)).unwrap();
    let source = recorder
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    recorder.last_pass = CapturePassId(u64::MAX);
    assert_eq!(
        recorder.arm(context(epoch), &[source]),
        Err(NoteCaptureError::IdentityExhausted)
    );
    assert!(recorder.store.slots.iter().all(|slot| slot.state.is_none()));
}

#[test]
fn late_quality_survives_sequence_and_occurrence_identity_exhaustion() {
    for sequence in [false, true] {
        let (mut recorder, source, ticket) = setup();
        fence(&mut recorder, source, 25);
        recorder.start(ticket).unwrap();
        assert!(publish(&mut recorder, source, 30, [0x90, 60, 1]).is_ok());
        assert!(publish(&mut recorder, source, 40, [0x80, 60, 0]).is_ok());
        fence(&mut recorder, source, 125);
        let state = recorder.sources[0].as_mut().unwrap();
        if sequence {
            state.sequence = PublicationSequence(u64::MAX);
        } else {
            state.last_occurrence = Some(PerformedOccurrenceId {
                source,
                serial: u64::MAX,
            });
        }
        let error = recorder.publish(
            source,
            CaptureStamp::exact_fixture(recorder.epoch, SampleTime::new(35), SampleTime::new(140))
                .unwrap(),
            Midi1Input::from_bytes([0x90, 60, 1]).unwrap(),
            AuditionTrace::NotOffered,
        );
        assert_eq!(error, Err(NoteCaptureError::IdentityExhausted));
        assert_eq!(
            recorder.result(ticket).unwrap().effective_outcome(),
            CaptureOutcome::Partial
        );
        assert_eq!(
            recorder
                .result(ticket)
                .unwrap()
                .quality()
                .late_count()
                .as_u64(),
            1
        );
    }
}
