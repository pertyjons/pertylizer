use super::*;
use crate::host::ConnectionGeneration;
use crate::recording::notes::tests::{context, fence, limits, publish};
use crate::recording::notes::{CaptureDisposition, ControllerSnapshot};
use crate::recording::notes::{MusicalInterval, NoteArmContext};
use crate::recording::{CaptureError, CaptureOutcome};
use crate::tempo::MusicalTick;
use crate::time::issue_epoch;

fn setup(capacity: u32) -> (SimulatedNoteRecorder, ConnectionGeneration, TakeReservation) {
    let epoch = issue_epoch().unwrap();
    let mut recorder = SimulatedNoteRecorder::prepare_fixture(epoch, limits(1_048_576)).unwrap();
    recorder
        .enable_ordered_session(CaptureCommandCapacity::new(capacity).unwrap())
        .unwrap();
    let source = recorder
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    let ticket = recorder.arm(context(epoch), &[source]).unwrap();
    (recorder, source, ticket)
}

fn offer(
    recorder: &mut SimulatedNoteRecorder,
    at: u64,
    command: CaptureCommand,
) -> CaptureCommandId {
    recorder
        .offer_boundary(recorder.epoch, SampleTime::new(at), command)
        .unwrap()
}

fn dispatch(recorder: &mut SimulatedNoteRecorder, id: CaptureCommandId) {
    let receipt = recorder.dispatch_boundary().unwrap();
    assert_eq!(receipt.boundary.id, id);
    assert_eq!(receipt.outcome, BoundaryOutcome::Applied);
}

#[test]
fn count_in_and_end_exclude_equal_sample_input_until_the_boundary_runs() {
    for (end, reason) in [
        (CaptureEnd::Stop, CaptureStopReason::Stop),
        (CaptureEnd::Disarm, CaptureStopReason::Disarm),
        (CaptureEnd::Panic, CaptureStopReason::Panic),
    ] {
        let (mut recorder, source, ticket) = setup(2);
        let start = offer(&mut recorder, 25, CaptureCommand::Start(ticket));
        let stop = offer(&mut recorder, 75, CaptureCommand::End(ticket, end));
        assert_ne!(start, stop);
        // A pre-start key remains physical state; it is not a captured onset.
        assert_eq!(
            publish(&mut recorder, source, 10, [0x90, 60, 100])
                .unwrap()
                .capture,
            CaptureDisposition::OutsideInterval
        );
        assert_eq!(
            recorder.start(ticket),
            Err(NoteCaptureError::OrderedSession)
        );
        assert_eq!(
            recorder.stop(ticket, SampleTime::new(75), reason),
            Err(NoteCaptureError::OrderedSession)
        );
        for _ in 0..2 {
            let receipt = recorder.dispatch_boundary().unwrap();
            assert_eq!(receipt.boundary.id, start);
            assert_eq!(receipt.outcome, BoundaryOutcome::WaitingForSources);
        }
        let before = recorder.source_sequence(source).unwrap();
        assert_eq!(
            publish(&mut recorder, source, 25, [0x90, 61, 100]),
            Err(NoteCaptureError::PendingBoundary)
        );
        assert_eq!(recorder.source_sequence(source).unwrap(), before);
        assert_eq!(
            recorder.fence(source, recorder.epoch, SampleTime::new(26), before),
            Err(NoteCaptureError::PendingBoundary)
        );
        fence(&mut recorder, source, 25);
        dispatch(&mut recorder, start);
        assert_eq!(
            publish(&mut recorder, source, 25, [0x90, 61, 100])
                .unwrap()
                .capture,
            CaptureDisposition::Recorded
        );
        let _receipt = publish(&mut recorder, source, 30, [0x80, 60, 0]).unwrap();
        assert_eq!(
            publish(&mut recorder, source, 75, [0x80, 61, 0]),
            Err(NoteCaptureError::PendingBoundary)
        );
        let sequence = recorder.source_sequence(source).unwrap();
        assert_eq!(
            recorder.fence(source, recorder.epoch, SampleTime::new(75), sequence),
            Err(NoteCaptureError::PendingBoundary)
        );
        dispatch(&mut recorder, stop);
        // Applying the stop did not fake the remaining source fence.
        assert!(recorder.result(ticket).is_err());
        fence(&mut recorder, source, 75);
        let _receipt = publish(&mut recorder, source, 75, [0x80, 61, 0]).unwrap();
        let result = recorder.result(ticket).unwrap();
        assert_eq!(result.effective_outcome(), CaptureOutcome::Complete);
        assert_eq!(result.window().end(), SampleTime::new(75));
        assert_eq!(result.initial_held().count(), 1);
        let closures: Vec<_> = result.closures().collect();
        assert_eq!(closures.len(), 1);
        assert_eq!(closures[0].time, SampleTime::new(75));
        assert_eq!(closures[0].reason, reason);
        assert!(closures[0].key_held);
    }
}

#[test]
fn same_sample_commands_keep_fifo_and_a_full_queue_preserves_the_reserved_end() {
    let (mut recorder, source, ticket) = setup(2);
    let start = offer(&mut recorder, 25, CaptureCommand::Start(ticket));
    assert_eq!(
        recorder.offer_boundary(
            recorder.epoch,
            SampleTime::new(25),
            CaptureCommand::Start(ticket)
        ),
        Err(SessionError::Full)
    );
    let end = offer(
        &mut recorder,
        25,
        CaptureCommand::End(ticket, CaptureEnd::Disarm),
    );
    assert_eq!(end.serial(), start.serial() + 1);
    assert_eq!(
        recorder.offer_boundary(
            recorder.epoch,
            SampleTime::new(25),
            CaptureCommand::End(ticket, CaptureEnd::Panic)
        ),
        Err(SessionError::Full)
    );
    fence(&mut recorder, source, 25);
    dispatch(&mut recorder, start);
    assert_eq!(
        publish(&mut recorder, source, 25, [0x90, 60, 100]),
        Err(NoteCaptureError::PendingBoundary)
    );
    dispatch(&mut recorder, end);
    assert!(recorder.dispatch_boundary().is_none());
    let result = recorder.result(ticket).unwrap();
    assert_eq!(result.window().start(), result.window().end());
    assert_eq!(result.records().count(), 0);
}

#[test]
fn cancelled_count_in_and_obsolete_start_return_distinct_receipts() {
    let (mut recorder, source, ticket) = setup(3);
    let end = offer(
        &mut recorder,
        20,
        CaptureCommand::End(ticket, CaptureEnd::Disarm),
    );
    let start = offer(&mut recorder, 25, CaptureCommand::Start(ticket));
    dispatch(&mut recorder, end);
    fence(&mut recorder, source, 20);
    let result = recorder.result(ticket).unwrap();
    assert_eq!(result.window().start(), SampleTime::new(20));
    assert_eq!(result.window().end(), SampleTime::new(20));
    let receipt = recorder.dispatch_boundary().unwrap();
    assert_eq!(receipt.boundary.id, start);
    assert_eq!(
        receipt.outcome,
        BoundaryOutcome::Refused(NoteCaptureError::NotActive)
    );
    assert!(!recorder.has_pending_boundaries());
}

#[test]
fn foreign_past_and_wrong_start_offers_leave_queue_and_identity_unchanged() {
    let (mut recorder, source, ticket) = setup(4);
    let foreign = issue_epoch().unwrap();
    assert_eq!(
        recorder.offer_boundary(foreign, SampleTime::new(25), CaptureCommand::Start(ticket)),
        Err(SessionError::ForeignEpoch)
    );
    assert_eq!(
        recorder.offer_boundary(
            recorder.epoch,
            SampleTime::new(24),
            CaptureCommand::Start(ticket)
        ),
        Err(SessionError::StartTime)
    );
    let start = offer(&mut recorder, 25, CaptureCommand::Start(ticket));
    assert_eq!(start.serial(), 1);
    assert_eq!(
        recorder.offer_boundary(
            recorder.epoch,
            SampleTime::new(24),
            CaptureCommand::End(ticket, CaptureEnd::Stop)
        ),
        Err(SessionError::PastBoundary)
    );
    fence(&mut recorder, source, 25);
    dispatch(&mut recorder, start);
    let _receipt = publish(&mut recorder, source, 25, [0x90, 60, 100]).unwrap();
    assert_eq!(
        recorder.offer_boundary(
            recorder.epoch,
            SampleTime::new(25),
            CaptureCommand::End(ticket, CaptureEnd::Stop)
        ),
        Err(SessionError::PastBoundary)
    );
    let end = offer(
        &mut recorder,
        50,
        CaptureCommand::End(ticket, CaptureEnd::Stop),
    );
    assert_eq!(end.serial(), 2);
    let (_, _, foreign_ticket) = setup(2);
    assert_eq!(
        recorder.offer_boundary(
            recorder.epoch,
            SampleTime::new(60),
            CaptureCommand::End(foreign_ticket, CaptureEnd::Stop)
        ),
        Err(SessionError::Capture(NoteCaptureError::NotActive))
    );
}

#[test]
fn queue_storage_is_charged_and_budget_refusal_is_transactional() {
    let epoch = issue_epoch().unwrap();
    let mut probe = SimulatedNoteRecorder::prepare_fixture(epoch, limits(1_048_576)).unwrap();
    let base = probe.bytes_reserved().get();
    let capacity = CaptureCommandCapacity::new(3).unwrap();
    probe.enable_ordered_session(capacity).unwrap();
    let required = probe.bytes_reserved().get();
    assert_eq!(
        required - base,
        size_of_val(&*probe.session_lane.as_ref().unwrap().slots) as u64
    );
    let mut short = SimulatedNoteRecorder::prepare_fixture(epoch, limits(required - 1)).unwrap();
    assert!(matches!(
        short.enable_ordered_session(capacity),
        Err(SessionError::Capture(NoteCaptureError::Storage(
            CaptureError::ByteBudget { .. }
        )))
    ));
    assert_eq!(short.bytes_reserved().get(), base);
    assert!(short.session_lane.is_none());
    let mut exact = SimulatedNoteRecorder::prepare_fixture(epoch, limits(required)).unwrap();
    exact.enable_ordered_session(capacity).unwrap();
    assert_eq!(exact.bytes_reserved().get(), required);
    assert_eq!(
        exact.enable_ordered_session(capacity),
        Err(SessionError::AlreadyEnabled)
    );
    let source = exact
        .bind_fixture_source(Some(ControllerSnapshot::neutral()))
        .unwrap();
    assert!(matches!(
        exact.arm(context(epoch), &[source]),
        Err(NoteCaptureError::Storage(CaptureError::ByteBudget { .. }))
    ));
    assert!(CaptureCommandCapacity::new(0).is_err());
    assert!(CaptureCommandCapacity::new(1).is_err());
}

#[test]
fn admitted_commands_wait_dispatch_wrap_and_cancel_without_allocator_activity() {
    let (mut recorder, source, ticket) = setup(2);
    let count = crate::render_allocation::count_allocs(|| {
        let start = offer(&mut recorder, 25, CaptureCommand::Start(ticket));
        assert_eq!(
            recorder.dispatch_boundary().unwrap().outcome,
            BoundaryOutcome::WaitingForSources
        );
        fence(&mut recorder, source, 25);
        dispatch(&mut recorder, start);
        let _receipt = publish(&mut recorder, source, 25, [0x90, 60, 100]).unwrap();
        fence(&mut recorder, source, 50);
        let end = offer(
            &mut recorder,
            75,
            CaptureCommand::End(ticket, CaptureEnd::Stop),
        );
        let cancelled = offer(
            &mut recorder,
            80,
            CaptureCommand::End(ticket, CaptureEnd::Panic),
        );
        dispatch(&mut recorder, end);
        recorder
            .interrupt_host(CaptureStopReason::DeviceLost)
            .unwrap();
        let receipt = recorder.dispatch_boundary().unwrap();
        assert_eq!(receipt.boundary.id, cancelled);
        assert_eq!(receipt.outcome, BoundaryOutcome::Cancelled);
        recorder.acknowledge_host_source(source).unwrap();
    });
    assert_eq!(count, 0);
    let result = recorder.result(ticket).unwrap();
    assert_eq!(result.sealed_outcome(), CaptureOutcome::Interrupted);
    assert_eq!(result.effective_outcome(), CaptureOutcome::Interrupted);
    assert_eq!(result.window().start(), SampleTime::new(25));
    assert_eq!(result.window().end(), SampleTime::new(50));
    assert_eq!(result.records().count(), 1);
    assert_eq!(result.selected_records().count(), 1);
    assert_eq!(result.closures().count(), 1);
    assert!(result.closures().all(|closure| {
        closure.time == SampleTime::new(50) && closure.reason == CaptureStopReason::DeviceLost
    }));
}

#[test]
fn a_refused_end_beyond_natural_completion_still_marks_the_dispatched_time_as_reached() {
    let (mut recorder, source, ticket) = setup(2);
    let start = offer(&mut recorder, 25, CaptureCommand::Start(ticket));
    let end = offer(
        &mut recorder,
        150,
        CaptureCommand::End(ticket, CaptureEnd::Stop),
    );
    fence(&mut recorder, source, 25);
    dispatch(&mut recorder, start);
    fence(&mut recorder, source, 125);
    assert_eq!(
        recorder.result(ticket).unwrap().window().end(),
        SampleTime::new(125)
    );
    let receipt = recorder.dispatch_boundary().unwrap();
    assert_eq!(receipt.boundary.id, end);
    assert_eq!(receipt.boundary.at, SampleTime::new(150));
    assert_eq!(
        receipt.outcome,
        BoundaryOutcome::Refused(NoteCaptureError::NotActive)
    );
    // Dispatch declared 150 reached even though the already-sealed take did not change.
    assert_eq!(
        publish(&mut recorder, source, 130, [0x90, 60, 100]),
        Err(NoteCaptureError::PastBoundary)
    );
    let mut past = context(recorder.epoch).input;
    past.interval = MusicalInterval::new(MusicalTick::new(5), MusicalTick::new(9)).unwrap();
    assert_eq!(
        recorder.arm(NoteArmContext::prepare(past).unwrap(), &[source]),
        Err(NoteCaptureError::PastBoundary)
    );
    let mut future = context(recorder.epoch).input;
    future.interval = MusicalInterval::new(MusicalTick::new(7), MusicalTick::new(11)).unwrap();
    let next = recorder
        .arm(NoteArmContext::prepare(future).unwrap(), &[source])
        .unwrap();
    let next_start = offer(&mut recorder, 175, CaptureCommand::Start(next));
    assert_eq!(next_start.serial(), end.serial() + 1);
    assert_eq!(
        recorder.result(ticket).unwrap().window().end(),
        SampleTime::new(125)
    );
}

#[test]
fn identity_exhaustion_refuses_without_replacing_the_last_command() {
    let (mut recorder, _, ticket) = setup(2);
    recorder.session_lane.as_mut().unwrap().serial = u64::MAX - 1;
    let last = offer(&mut recorder, 25, CaptureCommand::Start(ticket));
    assert_eq!(last.serial(), u64::MAX);
    assert_eq!(
        recorder.offer_boundary(
            recorder.epoch,
            SampleTime::new(30),
            CaptureCommand::End(ticket, CaptureEnd::Stop)
        ),
        Err(SessionError::IdentityExhausted)
    );
    assert_eq!(recorder.next_boundary().unwrap().id, last);
}
