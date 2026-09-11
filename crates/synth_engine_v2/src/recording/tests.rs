use super::*;
use crate::host::{EndpointSelection, OutputFormat, OutputRequest, SimulatedHost};
use crate::profile::{CaptureLimits, CaptureLimitsInput};
use crate::quantities::{
    CapturePassCount, CaptureResultCount, CaptureSourceCount, ChannelLayout, EventCount,
    HeldNoteCount, ProjectionTickCount, SampleRate, TrackedInputNoteCount,
};
use crate::time::issue_epoch;

fn store() -> SimulatedTakeStore<[u8; 16]> {
    let capture = CaptureLimits::new(CaptureLimitsInput {
        max_tracked_input_notes: TrackedInputNoteCount::limit(2).unwrap(),
        max_capture_sources: CaptureSourceCount::limit(1).unwrap(),
        max_capture_passes: CapturePassCount::limit(2).unwrap(),
        max_pending_capture_results: CaptureResultCount::limit(2).unwrap(),
        max_capture_bytes: PreparedBytes::limit(65_536).unwrap(),
        max_audio_capture_frames: FrameCount::new(512),
        max_projection_ticks: ProjectionTickCount::limit(961).unwrap(),
        capture_lateness_allowance: FrameCount::ZERO,
    })
    .unwrap();
    let limits = RecordingLimits::new(
        HeldNoteCount::limit(2).unwrap(),
        EventCount::limit(2).unwrap(),
    )
    .unwrap()
    .with_capture(capture)
    .unwrap();
    SimulatedTakeStore::prepare_fixture(limits).unwrap()
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

#[test]
fn writing_overflow_finalization_and_late_quality_neither_allocate_nor_deallocate() {
    let mut store = store();
    let epoch = issue_epoch().unwrap();
    let source = source();
    let window = CaptureWindow::new(epoch, SampleTime::ZERO, SampleTime::new(64)).unwrap();
    let first = store.reserve(window, &[source]).unwrap();
    let second = store.reserve(window, &[source]).unwrap();
    let count = crate::render_allocation::count_allocs(|| {
        store.push_fixture(first, [1; 16]).unwrap();
        store.push_fixture(first, [2; 16]).unwrap();
        assert_eq!(
            store.push_fixture(first, [3; 16]),
            Err(CaptureError::OrdinaryFull)
        );
        for index in 0..2 {
            store
                .write_fixture_metadata(first, CaptureBuffer::TerminalOccurrence, index, [4; 16])
                .unwrap();
        }
        for ticket in [first, second] {
            store
                .acknowledge_fixture_fence(ticket, source, epoch, SampleTime::new(64))
                .unwrap();
            store
                .seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(64))
                .unwrap();
        }
        store
            .attribute_fixture_late(second, source, epoch, SampleTime::new(32))
            .unwrap();
        store
            .acknowledge_fixture_quiescence(second, source, epoch, SampleTime::new(64))
            .unwrap();
    });
    assert_eq!(count, 0);
    assert_eq!(
        store.result(first).unwrap().cells(CaptureBuffer::Ordinary),
        &[Some([1; 16]), Some([2; 16])]
    );
    assert_eq!(
        store.result(second).unwrap().effective_outcome(),
        CaptureOutcome::Partial
    );
    assert_eq!(store.notification_misses().as_u64(), 1);
}

#[test]
fn late_count_saturation_is_sticky_and_explicit() {
    let mut store = store();
    let epoch = issue_epoch().unwrap();
    let source = source();
    let ticket = store
        .reserve(
            CaptureWindow::new(epoch, SampleTime::ZERO, SampleTime::new(64)).unwrap(),
            &[source],
        )
        .unwrap();
    store
        .acknowledge_fixture_fence(ticket, source, epoch, SampleTime::new(64))
        .unwrap();
    store
        .seal_fixture(ticket, CaptureOutcome::Complete, SampleTime::new(64))
        .unwrap();
    store.slots[ticket.slot]
        .state
        .as_mut()
        .unwrap()
        .quality
        .late_count
        .value = u64::MAX - 1;
    for _ in 0..2 {
        store
            .attribute_fixture_late(ticket, source, epoch, SampleTime::new(32))
            .unwrap();
    }
    let quality = store.result(ticket).unwrap().quality();
    assert_eq!(quality.late_count().as_u64(), u64::MAX);
    assert!(quality.late_count().saturated());
    assert_eq!(quality.first_late().unwrap().time, SampleTime::new(32));
}

#[test]
fn requested_byte_layout_covers_all_owned_allocations_and_descriptors() {
    let store = store();
    let mut bytes = size_of_val(&store) + size_of_val(&*store.slots);
    for slot in &store.slots {
        bytes += size_of_val(&*slot.data) + size_of_val(&*slot.sources);
    }
    assert_eq!(store.layout().bytes().get(), bytes as u64);
}

#[test]
fn exhausted_take_identity_space_never_wraps() {
    let counter = AtomicU64::new(u64::MAX - 1);
    assert_eq!(issue_take(&counter).unwrap().as_u64(), u64::MAX);
    assert_eq!(issue_take(&counter), Err(CaptureError::IdentityExhausted));
    assert_eq!(issue_take(&counter), Err(CaptureError::IdentityExhausted));
}
