//! P09-S002: storage admission and retained-result custody, before a capture publisher.

use synth_engine_v2::host::{
    ConnectionGeneration, EndpointSelection, OutputFormat, OutputRequest, SimulatedHost,
};
use synth_engine_v2::profile::{CaptureLimits, CaptureLimitsInput, ProfileError, RecordingLimits};
use synth_engine_v2::quantities::{
    CapturePassCount, CaptureResultCount, CaptureSourceCount, ChannelLayout, EventCount,
    HeldNoteCount, PreparedBytes, ProjectionTickCount, SampleRate, TrackedInputNoteCount,
};
use synth_engine_v2::recording::*;
use synth_engine_v2::time::{FrameCount, SampleTime, StreamEpoch, issue_epoch};

type Cell = [u8; 16];

fn config() -> CaptureLimitsInput {
    CaptureLimitsInput {
        max_tracked_input_notes: TrackedInputNoteCount::limit(3).unwrap(),
        max_capture_sources: CaptureSourceCount::limit(2).unwrap(),
        max_capture_passes: CapturePassCount::limit(3).unwrap(),
        max_pending_capture_results: CaptureResultCount::limit(2).unwrap(),
        max_capture_bytes: PreparedBytes::limit(1_048_576).unwrap(),
        max_audio_capture_frames: FrameCount::new(4_800),
        max_projection_ticks: ProjectionTickCount::limit(961).unwrap(),
        capture_lateness_allowance: FrameCount::new(5),
    }
}

fn limits(input: CaptureLimitsInput) -> RecordingLimits {
    RecordingLimits::new(
        HeldNoteCount::limit(2).unwrap(),
        EventCount::limit(3).unwrap(),
    )
    .unwrap()
    .with_capture(CaptureLimits::new(input).unwrap())
    .unwrap()
}

fn source() -> ConnectionGeneration {
    SimulatedHost::new()
        .begin(OutputRequest::new(
            EndpointSelection::SystemDefault,
            OutputFormat {
                rate: SampleRate::new(48_000.0).unwrap(),
                layout: ChannelLayout::Mono,
            },
        ))
        .unwrap()
}

fn window(epoch: StreamEpoch) -> CaptureWindow {
    CaptureWindow::new(epoch, SampleTime::new(10), SampleTime::new(20)).unwrap()
}

fn fixture() -> (
    SimulatedTakeStore<Cell>,
    TakeReservation,
    ConnectionGeneration,
    StreamEpoch,
) {
    let mut store = SimulatedTakeStore::prepare_fixture(limits(config())).unwrap();
    let source = source();
    let epoch = issue_epoch().unwrap();
    let ticket = store.reserve(window(epoch), &[source]).unwrap();
    (store, ticket, source, epoch)
}

fn fence(
    store: &mut SimulatedTakeStore<Cell>,
    ticket: TakeReservation,
    source: ConnectionGeneration,
    epoch: StreamEpoch,
) {
    // With allowance 5, a consumed fence at render frontier 25 establishes watermark 20.
    store
        .acknowledge_fixture_fence(ticket, source, epoch, SampleTime::new(25))
        .unwrap();
}

#[test]
fn all_configuration_is_required_and_every_positive_field_rejects_zero() {
    let absent = RecordingLimits::new(
        HeldNoteCount::limit(2).unwrap(),
        EventCount::limit(3).unwrap(),
    )
    .unwrap();
    assert!(absent.capture().is_none());
    assert!(matches!(
        SimulatedTakeStore::<Cell>::prepare_fixture(absent),
        Err(CaptureError::MissingConfiguration)
    ));
    for index in 0..7 {
        let mut input = config();
        match index {
            0 => input.max_tracked_input_notes = TrackedInputNoteCount::NONE,
            1 => input.max_capture_sources = CaptureSourceCount::NONE,
            2 => input.max_capture_passes = CapturePassCount::NONE,
            3 => input.max_pending_capture_results = CaptureResultCount::NONE,
            4 => input.max_capture_bytes = PreparedBytes::NONE,
            5 => input.max_audio_capture_frames = FrameCount::ZERO,
            _ => input.max_projection_ticks = ProjectionTickCount::NONE,
        }
        assert!(CaptureLimits::new(input).is_err(), "field {index}");
    }
    let mut input = config();
    input.capture_lateness_allowance = FrameCount::ZERO;
    assert_eq!(
        CaptureLimits::new(input)
            .unwrap()
            .capture_lateness_allowance(),
        FrameCount::ZERO
    );
    input.max_tracked_input_notes = TrackedInputNoteCount::limit(1).unwrap();
    assert!(matches!(
        absent.with_capture(CaptureLimits::new(input).unwrap()),
        Err(ProfileError::CaptureTrackerBelowHeld { .. })
    ));
}

#[test]
fn aggregate_byte_limit_accounts_for_every_result_before_allocating() {
    let report = CaptureLayout::for_payload::<Cell>(limits(config())).unwrap();
    assert_eq!(report.cells_in(CaptureBuffer::Ordinary), 3);
    assert_eq!(report.cells_in(CaptureBuffer::TerminalOccurrence), 2);
    assert_eq!(report.cells_in(CaptureBuffer::InitialSourceState), 2);
    assert_eq!(report.cells_in(CaptureBuffer::TerminalSourceState), 2);
    assert_eq!(report.cells_in(CaptureBuffer::Carry), 8); // 2 * H * (P - 1).
    assert_eq!(report.cells_in(CaptureBuffer::Pass), 3);
    let mut input = config();
    input.max_capture_bytes = PreparedBytes::limit(report.bytes().get() - 1).unwrap();
    assert!(matches!(
        SimulatedTakeStore::<Cell>::prepare_fixture(limits(input)),
        Err(CaptureError::ByteBudget { .. })
    ));
    input.max_capture_bytes = report.bytes();
    let exact = SimulatedTakeStore::<Cell>::prepare_fixture(limits(input)).unwrap();
    assert_eq!(exact.layout().bytes(), report.bytes());
    input.max_pending_capture_results = CaptureResultCount::limit(3).unwrap();
    assert!(matches!(
        SimulatedTakeStore::<Cell>::prepare_fixture(limits(input)),
        Err(CaptureError::ByteBudget { .. })
    ));
    assert!(matches!(
        SimulatedTakeStore::<[u8; 32]>::prepare_fixture(limits(input)),
        Err(CaptureError::ByteBudget { .. })
    ));
}

#[test]
fn finalization_arithmetic_refuses_overflow_before_any_allocation() {
    let mut input = config();
    input.max_tracked_input_notes = TrackedInputNoteCount::limit(u32::MAX).unwrap();
    input.max_capture_passes = CapturePassCount::limit(u32::MAX).unwrap();
    input.max_capture_bytes = PreparedBytes::limit(u64::MAX).unwrap();
    let limits = RecordingLimits::new(
        HeldNoteCount::limit(u32::MAX).unwrap(),
        EventCount::limit(1).unwrap(),
    )
    .unwrap()
    .with_capture(CaptureLimits::new(input).unwrap())
    .unwrap();
    assert!(matches!(
        SimulatedTakeStore::<Cell>::prepare_fixture(limits),
        Err(CaptureError::LayoutOverflow)
    ));
}

#[test]
fn exhausted_ordinary_storage_preserves_data_and_all_finalization_reserves() {
    let (mut store, ticket, source, epoch) = fixture();
    for value in [1, 2, 3] {
        store.push_fixture(ticket, [value; 16]).unwrap();
    }
    assert_eq!(
        store.push_fixture(ticket, [4; 16]),
        Err(CaptureError::OrdinaryFull)
    );
    assert_eq!(
        store.push_fixture(ticket, [5; 16]),
        Err(CaptureError::CaptureStopped)
    );
    for buffer in [
        CaptureBuffer::TrackedInput,
        CaptureBuffer::TerminalOccurrence,
        CaptureBuffer::InitialSourceState,
        CaptureBuffer::TerminalSourceState,
        CaptureBuffer::Pass,
        CaptureBuffer::Carry,
    ] {
        for index in 0..store.layout().cells_in(buffer) {
            store
                .write_fixture_metadata(ticket, buffer, index, [9; 16])
                .unwrap();
        }
        assert_eq!(
            store.write_fixture_metadata(ticket, buffer, store.layout().cells_in(buffer), [8; 16]),
            Err(CaptureError::MetadataBounds)
        );
    }
    fence(&mut store, ticket, source, epoch);
    store
        .seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(20))
        .unwrap();
    let result = store.result(ticket).unwrap();
    assert_eq!(result.sealed_outcome(), CaptureOutcome::Partial);
    assert_eq!(
        result.cells(CaptureBuffer::Ordinary),
        &[Some([1; 16]), Some([2; 16]), Some([3; 16])]
    );
    assert_eq!(
        result.cells(CaptureBuffer::TerminalOccurrence),
        &[Some([9; 16]); 2]
    );
    assert_eq!(result.cells(CaptureBuffer::Carry), &[Some([9; 16]); 8]);
    assert_eq!(
        store.write_fixture_metadata(ticket, CaptureBuffer::TerminalOccurrence, 0, [8; 16]),
        Err(CaptureError::Sealed)
    );
    assert_eq!(
        store.push_fixture(ticket, [6; 16]),
        Err(CaptureError::Sealed)
    );
}

#[test]
fn stalled_notifications_and_result_readers_do_not_release_entitlement() {
    let (mut store, first, source, epoch) = fixture();
    let second = store.reserve(window(epoch), &[source]).unwrap();
    for (ticket, value) in [(first, 1), (second, 2)] {
        store.push_fixture(ticket, [value; 16]).unwrap();
        fence(&mut store, ticket, source, epoch);
        store
            .seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(20))
            .unwrap();
    }
    assert_eq!(store.notification_misses().as_u64(), 1);
    assert_eq!(
        store.retained_results().collect::<Vec<_>>(),
        vec![first, second]
    );
    assert_eq!(store.take_notification(), Some(first));
    assert_eq!(store.take_notification(), None);
    assert_eq!(
        store.result(second).unwrap().cells(CaptureBuffer::Ordinary),
        &[Some([2; 16])]
    );
    assert_eq!(
        store.reserve(
            CaptureWindow::new(epoch, SampleTime::new(25), SampleTime::new(30)).unwrap(),
            &[source]
        ),
        Err(CaptureError::ResultsFull)
    );
    let quality = store.result(first).unwrap().quality();
    assert_eq!(
        store.discard(first, quality),
        Err(CaptureError::SourcesLive)
    );
    store
        .acknowledge_fixture_quiescence(first, source, epoch, SampleTime::new(20))
        .unwrap();
    store.discard(first, quality).unwrap();
    let replacement = store.reserve(window(epoch), &[self::source()]).unwrap();
    assert_ne!(replacement.id(), first.id());
    assert_eq!(
        store.push_fixture(first, [7; 16]),
        Err(CaptureError::StaleReservation)
    );
    assert_eq!(
        store.result(second).unwrap().cells(CaptureBuffer::Ordinary),
        &[Some([2; 16])]
    );
}

#[test]
fn minimum_source_watermark_and_explicit_allowance_control_sealing() {
    let (mut store, _, first, epoch) = fixture();
    let second = source();
    let ticket = store.reserve(window(epoch), &[first, second]).unwrap();
    fence(&mut store, ticket, first, epoch);
    assert_eq!(
        store.seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(20)),
        Err(CaptureError::AwaitingFences)
    );
    store
        .acknowledge_fixture_fence(ticket, second, epoch, SampleTime::new(20))
        .unwrap();
    assert_eq!(
        store.seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(20)),
        Err(CaptureError::AwaitingFences)
    );
    assert_eq!(
        store.acknowledge_fixture_fence(ticket, first, epoch, SampleTime::new(24)),
        Err(CaptureError::WatermarkRetreat)
    );
    fence(&mut store, ticket, second, epoch);
    store
        .seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(20))
        .unwrap();
    assert_eq!(
        store.result(ticket).unwrap().effective_outcome(),
        CaptureOutcome::Complete
    );
}

#[test]
fn late_faults_before_and_after_seal_preserve_payload_and_require_current_quality_ack() {
    for before_seal in [false, true] {
        let (mut store, ticket, source, epoch) = fixture();
        store.push_fixture(ticket, [17; 16]).unwrap();
        fence(&mut store, ticket, source, epoch);
        assert_eq!(
            store.attribute_fixture_late(ticket, source, epoch, SampleTime::new(9)),
            Ok(LateAttribution::OutsideSelectedInterval)
        );
        if !before_seal {
            store
                .seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(20))
                .unwrap();
        }
        let clean = CaptureQuality::default();
        for time in [15, 16, 17] {
            assert_eq!(
                store.attribute_fixture_late(ticket, source, epoch, SampleTime::new(time)),
                Ok(LateAttribution::TakeFaulted)
            );
        }
        if before_seal {
            assert_eq!(
                store.push_fixture(ticket, [18; 16]),
                Err(CaptureError::CaptureStopped)
            );
            store
                .seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(20))
                .unwrap();
        }
        let result = store.result(ticket).unwrap();
        assert_eq!(result.cells(CaptureBuffer::Ordinary), &[Some([17; 16])]);
        assert_eq!(result.effective_outcome(), CaptureOutcome::Partial);
        assert_eq!(
            result.sealed_outcome(),
            if before_seal {
                CaptureOutcome::Partial
            } else {
                CaptureOutcome::Complete
            }
        );
        let quality = result.quality();
        assert_eq!(
            quality.first_late(),
            Some(LateCaptureInput {
                source,
                time: SampleTime::new(15)
            })
        );
        assert_eq!(quality.late_count().as_u64(), 3);
        store
            .acknowledge_fixture_quiescence(ticket, source, epoch, SampleTime::new(20))
            .unwrap();
        assert_eq!(
            store.attribute_fixture_late(ticket, source, epoch, SampleTime::new(15)),
            Err(CaptureError::SourceQuiescent)
        );
        assert_eq!(
            store.discard(ticket, clean),
            Err(CaptureError::QualityChanged)
        );
        store.discard(ticket, quality).unwrap();
    }
}

#[test]
fn source_loss_finalizes_without_a_callback_at_the_last_valid_boundary() {
    let (mut store, ticket, source, epoch) = fixture();
    store.push_fixture(ticket, [31; 16]).unwrap();
    store
        .acknowledge_fixture_fence(ticket, source, epoch, SampleTime::new(17))
        .unwrap();
    assert_eq!(
        store.seal_fixture(ticket, CaptureOutcome::Interrupted, SampleTime::new(12)),
        Err(CaptureError::SourcesLive)
    );
    store
        .acknowledge_fixture_quiescence(ticket, source, epoch, SampleTime::new(12))
        .unwrap();
    store
        .seal_fixture(ticket, CaptureOutcome::Interrupted, SampleTime::new(12))
        .unwrap();
    let result = store.result(ticket).unwrap();
    assert_eq!(result.window().end(), SampleTime::new(12));
    assert_eq!(result.sealed_outcome(), CaptureOutcome::Interrupted);
    assert_eq!(result.cells(CaptureBuffer::Ordinary), &[Some([31; 16])]);
}

#[test]
fn invalid_sources_epochs_and_foreign_tickets_cannot_mutate_a_take() {
    let (mut store, ticket, source, epoch) = fixture();
    assert_eq!(
        store.reserve(window(epoch), &[]),
        Err(CaptureError::InvalidSources)
    );
    assert_eq!(
        store.reserve(window(epoch), &[source, source]),
        Err(CaptureError::InvalidSources)
    );
    let foreign = self::source();
    assert_eq!(
        store.acknowledge_fixture_fence(ticket, foreign, epoch, SampleTime::new(25)),
        Err(CaptureError::ForeignSource)
    );
    assert_eq!(
        store.acknowledge_fixture_fence(
            ticket,
            source,
            issue_epoch().unwrap(),
            SampleTime::new(25)
        ),
        Err(CaptureError::ForeignEpoch)
    );
    let (mut other, _, _, _) = fixture();
    assert_eq!(
        other.push_fixture(ticket, [1; 16]),
        Err(CaptureError::StaleReservation)
    );
    assert_eq!(
        store.seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(21)),
        Err(CaptureError::InvalidStop)
    );
    assert_eq!(
        store.seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(20)),
        Err(CaptureError::AwaitingFences)
    );
}

#[test]
fn empty_segments_still_own_result_and_quality_entitlement() {
    let (mut store, _, source, epoch) = fixture();
    let empty = CaptureWindow::new(epoch, SampleTime::new(10), SampleTime::new(10)).unwrap();
    let ticket = store.reserve(empty, &[source]).unwrap();
    store
        .acknowledge_fixture_quiescence(ticket, source, epoch, SampleTime::new(10))
        .unwrap();
    store
        .seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(10))
        .unwrap();
    assert!(
        store
            .result(ticket)
            .unwrap()
            .cells(CaptureBuffer::Ordinary)
            .is_empty()
    );
    assert_eq!(
        store.reserve(window(epoch), &[self::source()]),
        Err(CaptureError::ResultsFull)
    );
}

#[test]
fn empty_interval_at_epoch_origin_requires_each_explicit_source_fence() {
    let mut input = config();
    input.capture_lateness_allowance = FrameCount::ZERO;
    let mut store = SimulatedTakeStore::<Cell>::prepare_fixture(limits(input)).unwrap();
    let epoch = issue_epoch().unwrap();
    let sources = [source(), source()];
    let empty = CaptureWindow::new(epoch, SampleTime::ZERO, SampleTime::ZERO).unwrap();
    let ticket = store.reserve(empty, &sources).unwrap();
    for source in sources {
        assert_eq!(
            store.seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::ZERO),
            Err(CaptureError::AwaitingFences)
        );
        store
            .acknowledge_fixture_fence(ticket, source, epoch, SampleTime::ZERO)
            .unwrap();
    }
    store
        .seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::ZERO)
        .unwrap();
    assert_eq!(
        store.result(ticket).unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
}

#[test]
fn loss_before_requested_start_seals_an_empty_result_and_releases_entitlement() {
    let (mut store, first, a, epoch) = fixture();
    let b = source();
    let second = store.reserve(window(epoch), &[a, b]).unwrap();
    for ticket in [first, second] {
        if ticket == first {
            store
                .acknowledge_fixture_quiescence(ticket, a, epoch, SampleTime::new(5))
                .unwrap();
        }
        assert_eq!(
            store.seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(5)),
            Err(CaptureError::InvalidStop)
        );
        if ticket == second {
            assert!(
                store
                    .seal_fixture(ticket, CaptureOutcome::Interrupted, SampleTime::new(5))
                    .is_err()
            );
            store
                .acknowledge_fixture_quiescence(ticket, b, epoch, SampleTime::new(7))
                .unwrap();
        }
        store
            .seal_fixture(ticket, CaptureOutcome::Interrupted, SampleTime::new(5))
            .unwrap();
        let result = store.result(ticket).unwrap();
        assert_eq!(result.requested_window(), window(epoch));
        assert_eq!(result.window().start(), SampleTime::new(5));
        assert_eq!(result.window().end(), SampleTime::new(5));
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Interrupted);
        assert!(result.cells(CaptureBuffer::Ordinary).is_empty());
        let quality = result.quality();
        store.discard(ticket, quality).unwrap();
    }
    assert_eq!(store.retained_results().count(), 0);
    let fresh = source();
    for _ in 0..2 {
        let _ticket = store.reserve(window(epoch), &[fresh]).unwrap();
    }
}

#[test]
fn narrowing_the_selected_interval_preserves_an_already_attributed_fault() {
    let (mut store, ticket, source, epoch) = fixture();
    fence(&mut store, ticket, source, epoch);
    store
        .attribute_fixture_late(ticket, source, epoch, SampleTime::new(18))
        .unwrap();
    store
        .seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(12))
        .unwrap();
    let result = store.result(ticket).unwrap();
    assert_eq!(result.requested_window(), window(epoch));
    assert_eq!(result.window().end(), SampleTime::new(12));
    assert_eq!(
        result.quality().first_late().unwrap().time,
        SampleTime::new(18)
    );
    assert_eq!(result.effective_outcome(), CaptureOutcome::Partial);
    assert_eq!(
        store.attribute_fixture_late(ticket, source, epoch, SampleTime::new(18)),
        Ok(LateAttribution::OutsideSelectedInterval)
    );
    assert_eq!(
        store
            .result(ticket)
            .unwrap()
            .quality()
            .late_count()
            .as_u64(),
        1
    );
}

#[test]
fn source_fences_and_retirement_apply_to_every_take_and_cannot_be_resurrected() {
    let (mut store, first, source, epoch) = fixture();
    fence(&mut store, first, source, epoch);
    assert_eq!(
        store.reserve(window(epoch), &[source]),
        Err(CaptureError::WindowBeforeFrontier)
    );
    let next = CaptureWindow::new(epoch, SampleTime::new(25), SampleTime::new(30)).unwrap();
    let second = store.reserve(next, &[source]).unwrap();
    // Equality at the consumed frontier is valid, but the new interval still needs
    // a subsequent fence before it can seal; a refused past window spent no slot.
    assert_eq!(
        store.seal_fixture(second, CaptureOutcome::Complete, SampleTime::new(30)),
        Err(CaptureError::AwaitingFences)
    );
    store
        .acknowledge_fixture_fence(first, source, epoch, SampleTime::new(35))
        .unwrap();
    store
        .seal_fixture(first, CaptureOutcome::Complete, SampleTime::new(20))
        .unwrap();
    store
        .seal_fixture(second, CaptureOutcome::Complete, SampleTime::new(30))
        .unwrap();
    store
        .acknowledge_fixture_quiescence(first, source, epoch, SampleTime::new(30))
        .unwrap();
    for ticket in [first, second] {
        assert_eq!(
            store.attribute_fixture_late(ticket, source, epoch, SampleTime::new(15)),
            Err(CaptureError::SourceQuiescent)
        );
        assert_eq!(
            store.acknowledge_fixture_fence(ticket, source, epoch, SampleTime::new(25)),
            Err(CaptureError::SourceQuiescent)
        );
        let quality = store.result(ticket).unwrap().quality();
        store.discard(ticket, quality).unwrap();
    }
    assert_eq!(
        store.reserve(window(epoch), &[source]),
        Err(CaptureError::SourceNotFresh)
    );
    let fresh = self::source();
    let new = store.reserve(window(epoch), &[fresh]).unwrap();
    store.push_fixture(new, [1; 16]).unwrap();
}

#[test]
fn a_source_has_one_epoch_and_watermark_across_all_live_reservations() {
    let (mut store, first, source, epoch) = fixture();
    assert_eq!(
        store.reserve(window(issue_epoch().unwrap()), &[source]),
        Err(CaptureError::ForeignEpoch)
    );
    let second = store.reserve(window(epoch), &[source]).unwrap();
    fence(&mut store, first, source, epoch);
    assert_eq!(
        store.acknowledge_fixture_fence(second, source, epoch, SampleTime::new(24)),
        Err(CaptureError::WatermarkRetreat)
    );
    store
        .seal_fixture(second, CaptureOutcome::Complete, SampleTime::new(20))
        .unwrap();
}

#[test]
fn consumed_frontier_blocks_past_windows_even_when_watermark_saturates_at_zero() {
    let (mut store, first, source, epoch) = fixture();
    store
        .acknowledge_fixture_fence(first, source, epoch, SampleTime::new(3))
        .unwrap();
    for start in [0, 2] {
        let past = CaptureWindow::new(epoch, SampleTime::new(start), SampleTime::new(10)).unwrap();
        assert_eq!(
            store.reserve(past, &[source]),
            Err(CaptureError::WindowBeforeFrontier)
        );
    }
    assert_eq!(
        store.acknowledge_fixture_fence(first, source, epoch, SampleTime::new(2)),
        Err(CaptureError::WatermarkRetreat)
    );
    let current = CaptureWindow::new(epoch, SampleTime::new(3), SampleTime::new(10)).unwrap();
    let second = store.reserve(current, &[source]).unwrap();
    assert_eq!(
        store.seal_fixture(second, CaptureOutcome::Complete, SampleTime::new(10)),
        Err(CaptureError::AwaitingFences)
    );
}
