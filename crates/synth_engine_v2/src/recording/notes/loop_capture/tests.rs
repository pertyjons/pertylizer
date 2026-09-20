use super::*;
mod ordered;
use crate::{
    compile::{RenderConfig, compile},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    looping::LoopSettings,
    profile::{CaptureLimits, CaptureLimitsInput, HostProfile},
    quantities::{
        CaptureResultCount, CaptureSourceCount, ChannelLayout, EventCount, HeldNoteCount,
        ProjectionTickCount, TrackedInputNoteCount,
    },
    render::AudioBlockMut,
    schedule::AdmittedCompiledStream,
    tempo::Bpm,
};

fn limits(held: u32, passes: u32, bytes: u64) -> RecordingLimits {
    RecordingLimits::new(
        HeldNoteCount::limit(held).unwrap(),
        EventCount::limit(32).unwrap(),
    )
    .unwrap()
    .with_capture(
        CaptureLimits::new(CaptureLimitsInput {
            max_tracked_input_notes: TrackedInputNoteCount::limit(held + 2).unwrap(),
            max_capture_sources: CaptureSourceCount::limit(2).unwrap(),
            max_capture_passes: CapturePassCount::limit(passes).unwrap(),
            max_pending_capture_results: CaptureResultCount::limit(1).unwrap(),
            max_capture_bytes: PreparedBytes::limit(bytes).unwrap(),
            max_audio_capture_frames: FrameCount::new(1),
            max_projection_ticks: ProjectionTickCount::limit(100).unwrap(),
            capture_lateness_allowance: FrameCount::ZERO,
        })
        .unwrap(),
    )
    .unwrap()
}
fn stream(entry: u64, maximum: u64) -> CompiledLoopStream {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(maximum),
        ChannelLayout::Mono,
    )
    .unwrap();
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
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
    let plan = compile(&graph, &RenderConfig::new(profile))
        .into_plan()
        .unwrap();
    let events = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    CompiledLoopStream::prepare(
        plan,
        events,
        profile,
        LoopSettings::new(
            LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(50)).unwrap(),
            PlanPosition::new(entry),
            PreparedBytes::measured(1_000_000),
        )
        .unwrap(),
    )
    .unwrap()
}
fn input() -> LoopNoteArmInput {
    LoopNoteArmInput {
        target: FixtureTargetId::new(1).unwrap(),
        expected_revision: FixtureRevision::new(4),
        interval: MusicalInterval::new(MusicalTick::ZERO, MusicalTick::new(2)).unwrap(),
        mode: CaptureMode::Overdub,
        quantization: CaptureQuantization::Off,
        tempo: TempoMap::new(
            Bpm::new(120.0).unwrap(),
            &[],
            SampleRate::new(48_000.0).unwrap(),
        )
        .unwrap(),
    }
}
fn prepared(held: u32, passes: u32) -> (LoopCaptureSession, ConnectionGeneration) {
    let mut owner = LoopCaptureSession::prepare(
        stream(0, 512),
        limits(held, passes, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let source = owner.bind_source(ControllerSnapshot::neutral()).unwrap();
    let _ticket = owner.arm(input(), &[source]).unwrap();
    fence(&mut owner, source, 0);
    owner.start().unwrap();
    (owner, source)
}
fn fence(owner: &mut LoopCaptureSession, source: ConnectionGeneration, at: u64) {
    owner
        .fence(
            source,
            SampleTime::new(at),
            owner.source_sequence(source).unwrap(),
        )
        .unwrap();
}
fn publish(
    owner: &mut LoopCaptureSession,
    source: ConnectionGeneration,
    nominal: u64,
    published: u64,
    bytes: [u8; 3],
) -> PublicationReceipt {
    owner
        .publish(
            source,
            CaptureStamp::exact_fixture(
                owner.initial().epoch,
                SampleTime::new(nominal),
                SampleTime::new(published),
            )
            .unwrap(),
            Midi1Input::from_bytes(bytes).unwrap(),
            AuditionTrace::NotOffered,
        )
        .unwrap()
}
fn render(owner: &mut LoopCaptureSession, total: usize, partition: usize) -> Vec<f32> {
    let mut samples = vec![9.0; total];
    for chunk in samples.chunks_mut(partition) {
        let count = chunk.len();
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                owner
                    .render(AudioBlockMut::new(chunk, count, ChannelLayout::Mono).unwrap())
                    .unwrap();
            }),
            0
        );
    }
    samples
}

#[test]
fn actual_loop_audio_and_raw_segments_match_across_callback_partitions() {
    for partition in [1, 37, 64, 256, 512] {
        let (mut owner, source) = prepared(2, 4);
        let first = publish(&mut owner, source, 1, 1, [0x90, 60, 100])
            .occurrence
            .unwrap();
        let second = publish(&mut owner, source, 49, 49, [0x90, 60, 90])
            .occurrence
            .unwrap();
        assert_eq!(
            publish(&mut owner, source, 50, 50, [0x80, 60, 0]).occurrence,
            Some(first)
        );
        assert_eq!(
            publish(&mut owner, source, 100, 100, [0x80, 60, 0]).occurrence,
            Some(second)
        );
        fence(&mut owner, source, 200);
        assert!(matches!(
            owner.finalize(),
            Err(LoopCaptureError::AwaitingAudio)
        ));
        assert!(owner.result().is_err());
        let samples = render(&mut owner, 320, partition);
        for (frame, sample) in samples.into_iter().enumerate() {
            assert_eq!(sample, f32::from(frame >= 64 && (frame - 64) % 50 == 0));
        }
        owner.finalize().unwrap();
        owner.finalize().unwrap();
        let raw = owner.result().unwrap();
        assert_eq!(raw.sealed_outcome(), CaptureOutcome::Complete);
        assert!(raw.context().anchor().is_none());
        assert_eq!(raw.records().count(), 4);
        assert_eq!(
            raw.loop_passes()
                .map(|pass| (pass.window().start().as_u64(), pass.window().end().as_u64()))
                .collect::<Vec<_>>(),
            [(0, 50), (50, 100), (100, 150), (150, 200)]
        );
        let passes: Vec<_> = raw.loop_passes().copied().collect();
        assert_eq!(raw.pass(), Some(passes[0].id()));
        assert_eq!(raw.loop_carry().count(), 6); // both keys at50, remaining key at100
        assert!(
            raw.loop_carry()
                .all(|carry| carry.occurrence() == first || carry.occurrence() == second)
        );
        assert_eq!(
            raw.records()
                .filter(|r| r.stamp().nominal() >= passes[1].window().start()
                    && r.stamp().nominal() < passes[1].window().end())
                .count(),
            1
        );
        assert!(matches!(
            owner.recorder.project_notes(owner.ticket.unwrap()),
            Err(projection::ProjectionError::LoopMapping)
        ));
    }
}

#[test]
fn nominal_carry_overflow_keeps_all_raw_and_closes_before_unadmitted_pass() {
    for held in [1, 2, 3] {
        let (mut owner, source) = prepared(held, 4);
        for (nominal, published, bytes) in [
            (0, 0, [0x90, 60, 1]),
            (100, 100, [0x80, 60, 0]),
            (1, 101, [0x90, 60, 1]),
            (101, 102, [0x80, 60, 0]),
            (2, 103, [0x90, 60, 1]),
            (102, 104, [0x80, 60, 0]),
        ] {
            assert_eq!(
                publish(&mut owner, source, nominal, published, bytes).capture,
                CaptureDisposition::Recorded
            );
            assert!(owner.recorder.tracker.iter().flatten().count() <= 1);
        }
        let _samples = render(&mut owner, 320, 37);
        assert!(matches!(
            owner.finalize(),
            Err(LoopCaptureError::AwaitingSources)
        ));
        fence(&mut owner, source, 200);
        owner.finalize().unwrap();
        let raw = owner.result().unwrap();
        assert_eq!(raw.records().count(), 6);
        if held < 3 {
            assert_eq!(owner.carry_capacity(), Some(SampleTime::new(50)));
            assert_eq!(raw.window().end(), SampleTime::new(50));
            assert_eq!(raw.sealed_outcome(), CaptureOutcome::Partial);
            assert_eq!(raw.loop_passes().count(), 1);
            assert_eq!(raw.loop_carry().count(), 0);
            assert_eq!(raw.closures().count(), 3);
            assert!(
                raw.closures()
                    .all(|c| c.reason == CaptureStopReason::LoopCarryCapacity)
            );
        } else {
            assert_eq!(owner.carry_capacity(), None);
            assert_eq!(raw.sealed_outcome(), CaptureOutcome::Complete);
            assert_eq!(raw.loop_carry().count(), 12);
        }
    }
}

#[test]
fn input_loss_and_audio_failure_need_independent_quiescence_without_another_render() {
    let (mut owner, source) = prepared(1, 4);
    let _receipt = publish(&mut owner, source, 1, 1, [0x90, 60, 100]);
    let _samples = render(&mut owner, 128, 128);
    owner.quiesce(source, SampleTime::new(40)).unwrap();
    assert!(owner.result().is_err());
    let end = owner.finish_observation();
    assert_eq!(end.at, SampleTime::new(64));
    owner.finalize().unwrap();
    let raw = owner.result().unwrap();
    assert_eq!(raw.window().end(), SampleTime::new(40));
    assert_eq!(raw.sealed_outcome(), CaptureOutcome::Interrupted);
    assert_eq!(raw.loop_passes().count(), 1);
    assert_eq!(raw.closures().count(), 1);

    let (mut owner, source) = prepared(1, 4);
    let _samples = render(&mut owner, 128, 128);
    assert!(
        owner
            .render(AudioBlockMut::new(&mut [0.0; 513], 513, ChannelLayout::Mono).unwrap())
            .is_err()
    );
    let first = owner.observation_end();
    fence(&mut owner, source, 64);
    assert!(matches!(
        owner.finalize(),
        Err(LoopCaptureError::AwaitingSources)
    ));
    owner.quiesce(source, SampleTime::new(64)).unwrap();
    owner.finalize().unwrap();
    assert_eq!(owner.observation_end(), first);
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
}

#[test]
fn pass_limit_one_never_allocates_carry_or_invents_an_empty_next_pass() {
    let (mut owner, source) = prepared(1, 1);
    let _samples = render(&mut owner, 320, 37);
    fence(&mut owner, source, 50);
    owner.finalize().unwrap();
    let raw = owner.result().unwrap();
    assert_eq!(raw.loop_passes().count(), 1);
    assert_eq!(raw.loop_carry().count(), 0);
    assert_eq!(raw.window().end(), SampleTime::new(50));
    assert_eq!(owner.journal_bytes(), PreparedBytes::NONE);
}

#[test]
fn mapping_and_identity_admission_refuse_without_consuming_a_take() {
    let mut owner = LoopCaptureSession::prepare(
        stream(0, 512),
        limits(1, 4, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let source = owner.bind_source(ControllerSnapshot::neutral()).unwrap();
    let mut wrong = input();
    wrong.interval = MusicalInterval::new(MusicalTick::ZERO, MusicalTick::new(3)).unwrap();
    assert!(matches!(
        owner.arm(wrong, &[source]),
        Err(LoopCaptureError::Mapping)
    ));
    assert!(owner.ticket.is_none());
    let mut wrong_rate = input();
    // Both endpoints still map to 0..50; geometry equality must not conceal a rate mismatch.
    wrong_rate.tempo = TempoMap::new(
        Bpm::new(240.0).unwrap(),
        &[],
        SampleRate::new(96_000.0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        wrong_rate.tempo.position_of(MusicalTick::new(2)).unwrap(),
        PlanPosition::new(50)
    );
    assert!(matches!(
        owner.arm(wrong_rate, &[source]),
        Err(LoopCaptureError::Mapping)
    ));
    owner.recorder.last_pass = CapturePassId(u64::MAX - 3);
    assert!(matches!(
        owner.arm(input(), &[source]),
        Err(LoopCaptureError::Capture(
            NoteCaptureError::IdentityExhausted
        ))
    ));
    assert!(!owner.recorder.is_active());
    owner.recorder.last_pass = CapturePassId(u64::MAX - 4);
    let _ticket = owner.arm(input(), &[source]).unwrap();
    assert_eq!(owner.recorder.last_pass, CapturePassId(u64::MAX));
}

#[test]
fn combined_inline_and_recording_allocations_obey_exact_byte_budget() {
    let owner = LoopCaptureSession::prepare(
        stream(0, 512),
        limits(1, 4, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let base = owner.recording_bytes().get();
    let mut actual = size_of_val(&owner)
        + size_of_val(&*owner.recorder.sources)
        + size_of_val(&*owner.recorder.tracker)
        + size_of_val(&*owner.recorder.contexts)
        + size_of_val(&*owner.recorder.store.slots);
    for slot in &owner.recorder.store.slots {
        actual += size_of_val(&*slot.data) + size_of_val(&*slot.sources);
    }
    assert_eq!(base, actual as u64);
    assert!(
        LoopCaptureSession::prepare(
            stream(0, 512),
            limits(1, 4, base - 1),
            PreparedBytes::measured(8192)
        )
        .is_err()
    );
    let mut exact = LoopCaptureSession::prepare(
        stream(0, 512),
        limits(1, 4, base),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let source = exact.bind_source(ControllerSnapshot::neutral()).unwrap();
    assert!(exact.arm(input(), &[source]).is_err()); // map heap also belongs to recording
    let bytes = base + input().tempo.bytes_held() as u64;
    let mut exact = LoopCaptureSession::prepare(
        stream(0, 512),
        limits(1, 4, bytes),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let source = exact.bind_source(ControllerSnapshot::neutral()).unwrap();
    let _ticket = exact.arm(input(), &[source]).unwrap();
    assert_eq!(exact.recording_bytes().get(), bytes);
}

#[test]
fn all_hot_operations_keep_storage_while_two_sources_finish_independently() {
    let mut owner = LoopCaptureSession::prepare(
        stream(0, 512),
        limits(2, 4, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let a = owner.bind_source(ControllerSnapshot::neutral()).unwrap();
    let b = owner.bind_source(ControllerSnapshot::neutral()).unwrap();
    let _ticket = owner.arm(input(), &[a, b]).unwrap();
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            fence(&mut owner, a, 0);
            assert!(owner.start().is_err());
            fence(&mut owner, b, 0);
            owner.start().unwrap();
            let _receipt = publish(&mut owner, a, 1, 1, [0x90, 60, 100]);
            let _receipt = publish(&mut owner, b, 50, 50, [0x90, 61, 100]);
            owner
                .render(AudioBlockMut::new(&mut [0.0; 320], 320, ChannelLayout::Mono).unwrap())
                .unwrap();
            fence(&mut owner, a, 200);
            assert!(matches!(
                owner.finalize(),
                Err(LoopCaptureError::AwaitingSources)
            ));
            assert!(owner.result().is_err());
            fence(&mut owner, b, 200);
            owner.finalize().unwrap();
            owner.quiesce(a, SampleTime::new(200)).unwrap();
            owner.quiesce(b, SampleTime::new(200)).unwrap();
        }),
        0
    );
    let quality = owner.result().unwrap().quality();
    owner.discard(quality).unwrap();
    assert!(owner.result().is_err());
}

#[test]
fn pre_capture_held_key_keeps_fifo_and_never_becomes_a_captured_carry_attack() {
    let mut looping = stream(0, 512);
    looping
        .render(AudioBlockMut::new(&mut [0.0; 128], 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    let mut owner = LoopCaptureSession::prepare(
        looping,
        limits(1, 3, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let pedal = ControllerSnapshot::neutral()
        .with_input(Midi1Input::from_bytes([0xb0, 64, 127]).unwrap())
        .unwrap();
    let source = owner.bind_source(pedal).unwrap();
    let old = publish(&mut owner, source, 10, 10, [0x90, 60, 100])
        .occurrence
        .unwrap();
    let _ticket = owner.arm(input(), &[source]).unwrap();
    fence(&mut owner, source, 64);
    owner.start().unwrap();
    let captured = publish(&mut owner, source, 65, 65, [0x90, 60, 100])
        .occurrence
        .unwrap();
    assert_eq!(
        publish(&mut owner, source, 66, 66, [0x80, 60, 0]).occurrence,
        Some(old)
    );
    assert_eq!(
        publish(&mut owner, source, 100, 100, [0x80, 60, 0]).occurrence,
        Some(captured)
    );
    let _receipt = publish(&mut owner, source, 110, 110, [0xb0, 64, 0]);
    let _samples = render(&mut owner, 320, 37);
    fence(&mut owner, source, 200);
    owner.finalize().unwrap();
    let raw = owner.result().unwrap();
    assert_eq!(raw.initial_held().next().unwrap().occurrence, old);
    assert_eq!(raw.loop_carry().count(), 2);
    assert!(raw.loop_carry().all(|carry| carry.occurrence() == captured));
    assert!(raw.initial_sources().next().unwrap().controls.pedals()[0]);
    assert!(!raw.terminal_sources().next().unwrap().controls.pedals()[0]);
    assert_eq!(
        raw.loop_passes().next().unwrap().window().start(),
        SampleTime::new(64)
    );
}

#[test]
fn pending_initial_wrap_keeps_empty_initial_pass_and_no_empty_final_pass() {
    let mut looping = stream(0, 512);
    for _ in 0..26 {
        looping
            .render(AudioBlockMut::new(&mut [0.0; 64], 64, ChannelLayout::Mono).unwrap())
            .unwrap();
    }
    assert_eq!(looping.snapshot().position, PlanPosition::new(50));
    let mut owner = LoopCaptureSession::prepare(
        looping,
        limits(1, 2, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let source = owner.bind_source(ControllerSnapshot::neutral()).unwrap();
    let _ticket = owner.arm(input(), &[source]).unwrap();
    fence(&mut owner, source, 1600);
    owner.start().unwrap();
    let _receipt = publish(&mut owner, source, 1600, 1600, [0x90, 60, 100]);
    let _samples = render(&mut owner, 128, 37);
    fence(&mut owner, source, 1650);
    owner.finalize().unwrap();
    let raw = owner.result().unwrap();
    assert_eq!(
        raw.loop_passes()
            .map(|pass| (pass.window().start().as_u64(), pass.window().end().as_u64()))
            .collect::<Vec<_>>(),
        [(1600, 1600), (1600, 1650)]
    );
    assert_eq!(raw.loop_carry().count(), 0);
    assert_eq!(raw.records().count(), 1);
}

#[test]
fn late_quality_is_sticky_after_loop_seal_and_reading_never_renews_passes() {
    let (mut owner, source) = prepared(1, 2);
    let _samples = render(&mut owner, 320, 37);
    fence(&mut owner, source, 100);
    owner.finalize().unwrap();
    let first = owner.observation_end();
    for _ in 0..3 {
        assert_eq!(owner.result().unwrap().loop_passes().count(), 2);
        let _samples = render(&mut owner, 128, 37);
    }
    assert_eq!(owner.observation_end(), first);
    assert_eq!(
        publish(&mut owner, source, 10, 100, [0xb0, 64, 127]).capture,
        CaptureDisposition::Late
    );
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(
        owner.result().unwrap().effective_outcome(),
        CaptureOutcome::Partial
    );
    assert!(owner.discard(owner.result().unwrap().quality()).is_err());
}

#[test]
fn a_refused_release_cannot_silently_change_the_accepted_raw_carry_log() {
    let (mut owner, source) = prepared(1, 4);
    let onset = publish(&mut owner, source, 10, 10, [0x90, 60, 100])
        .occurrence
        .unwrap();
    fence(&mut owner, source, 20);
    assert_eq!(
        publish(&mut owner, source, 15, 60, [0x80, 60, 0]).capture,
        CaptureDisposition::Late
    );
    let _samples = render(&mut owner, 320, 37);
    fence(&mut owner, source, 60);
    owner.finalize().unwrap();
    let raw = owner.result().unwrap();
    assert_eq!(raw.records().count(), 1);
    assert_eq!(raw.effective_outcome(), CaptureOutcome::Partial);
    assert_eq!(raw.quality().late_count().as_u64(), 1);
    assert_eq!(raw.loop_passes().count(), 2);
    assert_eq!(raw.loop_carry().count(), 2);
    assert!(
        raw.loop_carry()
            .all(|carry| carry.occurrence() == onset && carry.at() == SampleTime::new(50))
    );
    // Terminal key diagnostics separately retain the actually paired refused release.
    assert!(!raw.closures().next().unwrap().key_held);
}

#[test]
fn a_later_carry_failure_preserves_earlier_metadata_and_stronger_interruption() {
    for lost in [false, true] {
        let (mut owner, source) = prepared(1, 4);
        for (nominal, published, bytes) in [
            (0, 0, [0x90, 60, 1]),
            (60, 60, [0x80, 60, 0]),
            (70, 70, [0x90, 60, 1]),
            (170, 170, [0x80, 60, 0]),
            (71, 171, [0x90, 60, 1]),
            (171, 172, [0x80, 60, 0]),
        ] {
            assert_eq!(
                publish(&mut owner, source, nominal, published, bytes).capture,
                CaptureDisposition::Recorded
            );
        }
        let _samples = render(&mut owner, 320, 37);
        if lost {
            owner.quiesce(source, SampleTime::new(190)).unwrap();
        } else {
            fence(&mut owner, source, 200);
        }
        owner.finalize().unwrap();
        let raw = owner.result().unwrap();
        assert_eq!(owner.carry_capacity(), Some(SampleTime::new(100)));
        assert_eq!(raw.window().end(), SampleTime::new(100));
        assert_eq!(raw.records().count(), 6);
        assert_eq!(raw.loop_passes().count(), 2);
        assert_eq!(raw.loop_carry().count(), 2);
        assert!(
            raw.loop_carry()
                .all(|carry| carry.at() == SampleTime::new(50))
        );
        assert_eq!(
            raw.sealed_outcome(),
            if lost {
                CaptureOutcome::Interrupted
            } else {
                CaptureOutcome::Partial
            }
        );
    }
}
