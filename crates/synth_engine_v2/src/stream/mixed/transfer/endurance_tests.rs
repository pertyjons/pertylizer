//! Deterministic credit/identity churn and bounded overload, not production sizing.
use super::*;
use sha2::{Digest, Sha256};

const CYCLES: usize = 65_540;
const CYCLE_FRAMES: usize = 128;

#[derive(Debug, PartialEq, Eq)]
struct Trace {
    audio: [u8; 32],
    generations: Vec<u32>,
    audio_refusals: usize,
    control_overload_refusals: usize,
    receipts: usize,
    stale_handle_refusals: usize,
}

fn churn(partition: usize) -> Trace {
    let mut armed = arm();
    drain_carry(&mut armed);
    let initial_storage = armed.audio.resource_usage().expect("prepared storage");
    let initial_credits = armed.control.inner().bend_credits_free;
    let mut digest = Sha256::new();
    let mut generations = Vec::with_capacity(CYCLES);
    let mut previous_handle = None;
    let mut previous_index = None;
    let mut audio_refusals = 0;
    let mut control_overload_refusals = 0;
    let mut receipts = 0;
    let mut stale_handle_refusals = 0;
    for cycle in 0..CYCLES {
        let clock = armed.audio.inner.audio.renderer.clock().as_u64();
        let (on, handle) = armed
            .control
            .submit_note_on(&mut armed.control_port, note_on(clock + 16, 64))
            .expect("onset");
        if let Some(stale) = previous_handle {
            let frontier = armed.control.frontier();
            let stale_up = MixedSourceKeyUp {
                at: SampleTime::new(clock + 20),
                handle: stale,
            };
            assert_eq!(
                refused(
                    armed
                        .control
                        .submit_key_up(&mut armed.control_port, stale_up)
                ),
                (MixedTransferRefusal::NotOpen, stale_up)
            );
            assert_eq!(armed.control.frontier(), frontier);
            stale_handle_refusals += 1;
        }
        let mut accepted_bends = 0;
        let burst = cycle % 64 == 0;
        for bend in 0..if burst { depth() + 1 } else { 1 } {
            let observation = bend_on(
                clock + 24,
                MidiChannel::CH1,
                if bend % 2 == 0 { 25.0 } else { -25.0 },
            );
            let frontier = armed.control.frontier();
            match armed
                .control
                .submit_bend(&mut armed.control_port, observation)
            {
                Ok(_) => accepted_bends += 1,
                Err(MixedTransferSubmitError::Refused { reason, original }) => {
                    assert!(burst);
                    assert!(matches!(
                        reason,
                        MixedTransferRefusal::CommandCapacity | MixedTransferRefusal::BendCredit
                    ));
                    assert_eq!(original, observation);
                    assert_eq!(armed.control.frontier(), frontier);
                    assert_eq!(armed.control.fault(), None);
                    control_overload_refusals += 1;
                    break;
                }
                Err(error) => panic!("unexpected host fault: {error:?}"),
            }
        }
        let up = armed
            .control
            .submit_key_up(
                &mut armed.control_port,
                MixedSourceKeyUp {
                    at: SampleTime::new(clock + 80),
                    handle,
                },
            )
            .expect("reserved key-up survives overload");
        let mut samples = [0.0; CYCLE_FRAMES];
        assert_eq!(
            count_allocs(|| {
                for part in samples.chunks_mut(partition) {
                    callback(&mut armed.audio, &mut armed.audio_port, part).expect("render");
                }
            }),
            0,
            "no callback allocator activity during churn"
        );
        for sample in samples {
            digest.update(sample.to_bits().to_le_bytes());
        }
        let mut identity = None;
        let mut count = 0;
        while let Some(report) = armed
            .control
            .collect(&mut armed.control_port)
            .expect("collect")
        {
            assert_eq!(
                report.at(),
                report.executed_at(),
                "no late software command"
            );
            match report.outcome() {
                MixedTransferOutcome::NoteOn {
                    handle: reported,
                    result: Ok(note),
                } => {
                    assert_eq!(report.id(), on);
                    assert_eq!(reported, handle);
                    assert_eq!(note.generation(), u32::try_from(cycle).expect("generation"));
                    if let Some(index) = previous_index {
                        assert_eq!(note.index(), index);
                    }
                    previous_index = Some(note.index());
                    generations.push(note.generation());
                    identity = Some(note);
                }
                MixedTransferOutcome::Bend {
                    result: Ok(applied),
                    ..
                } => assert_eq!(applied.members().get(), 1),
                MixedTransferOutcome::Bend {
                    result: Err(MixedTransferBendRefused::Ingress(_)),
                    ..
                } => audio_refusals += 1,
                MixedTransferOutcome::KeyUp {
                    handle: reported,
                    result: Ok(MixedKeyUpRetired::Released(note)),
                } => {
                    assert_eq!(report.id(), up);
                    assert_eq!(reported, handle);
                    assert_eq!(Some(note), identity);
                }
                other => panic!("lost or unexpected command result: {other:?}"),
            }
            count += 1;
        }
        assert_eq!(count, accepted_bends + 2);
        receipts += count;
        assert_eq!(
            armed.control.inner().release_reserved,
            MixedIngressQueueCount::NONE
        );
        assert_eq!(
            armed.control.inner().ingress_outstanding,
            MixedIngressQueueCount::NONE
        );
        let inner = armed.control.inner();
        assert_eq!(
            inner.bend_credits_free.as_usize() + inner.bend_credit_return.pending().as_usize(),
            initial_credits.as_usize(),
            "free and lock-free returned bend credits are conserved"
        );
        assert_eq!(armed.audio.live_holds_outstanding(), EventCount::NONE);
        assert!(armed.audio.inner.test_ingress.is_empty());
        assert_eq!(armed.audio.inner.audio.live_minter.live(), 0);
        assert_eq!(
            armed.audio.inner.audio.compiled_minter.live(),
            1,
            "compiled note remains held until the final Stop"
        );
        let usage = armed.audio.resource_usage().expect("storage");
        assert_eq!(usage.renderer_storage, initial_storage.renderer_storage);
        assert_eq!(usage.ingress_storage, initial_storage.ingress_storage);
        assert_eq!(
            usage.publication_storage,
            initial_storage.publication_storage
        );
        assert_eq!(usage.renderer.rejected_control_writes, EventCount::NONE);
        previous_handle = Some(handle);
    }
    assert_eq!(generations.len(), CYCLES);
    assert_eq!(control_overload_refusals, CYCLES.div_ceil(64));
    let stop = armed
        .audio
        .inner
        .audio
        .renderer
        .clock()
        .checked_add(FrameCount::QUANTUM)
        .expect("stop boundary");
    let _scheduled = armed
        .audio
        .inner
        .schedule_compiled_stop(stop)
        .expect("compiled Stop");
    let mut samples = [0.0; CYCLE_FRAMES];
    assert_eq!(
        count_allocs(|| {
            for part in samples.chunks_mut(partition) {
                callback(&mut armed.audio, &mut armed.audio_port, part).expect("drain");
            }
        }),
        0
    );
    assert_eq!(armed.audio.inner.audio.compiled_minter.live(), 0);
    assert_eq!(armed.audio.inner.audio.live_minter.live(), 0);
    assert!(
        !armed
            .audio
            .inner
            .audio
            .renderer
            .has_replacement_obligations()
    );
    let usage = armed.audio.publication_usage();
    for class in [
        ProducerClass::Compiled,
        ProducerClass::Live,
        ProducerClass::Session,
        ProducerClass::Release,
    ] {
        assert!(usage.high_water(class).expect("available producer").get() > 0);
    }
    for class in [ProducerClass::AuthoredRuntime, ProducerClass::Internal] {
        assert_eq!(
            usage.high_water(class),
            None,
            "unavailable is never a measured zero"
        );
    }
    let teardown = armed
        .control
        .join(
            armed.audio,
            Some(armed.control_port),
            Some(armed.audio_port),
        )
        .expect("joined custody");
    assert!(teardown.outstanding().is_empty());
    assert!(teardown.unserved().is_empty());
    assert!(teardown.uncollected().is_empty());
    assert!(teardown.lost().is_empty());
    assert!(teardown.sounding_live().expect("snapshot").is_empty());
    assert_eq!(teardown.holds_outstanding(), EventCount::NONE);
    Trace {
        audio: digest.finalize().into(),
        generations,
        audio_refusals,
        control_overload_refusals,
        receipts,
        stale_handle_refusals,
    }
}

#[test]
fn repeated_live_identity_and_credit_reuse_recovers_from_overload_with_partition_invariant_audio() {
    assert_eq!(
        profile().capabilities().sample_rate(),
        crate::quantities::SampleRate::new(48_000.0).expect("rate")
    );
    let oracle = churn(128);
    assert_eq!(oracle.receipts, 208_921);
    assert_eq!(oracle.control_overload_refusals, 1_025);
    assert_eq!(oracle.audio_refusals, 0);
    assert_eq!(oracle.stale_handle_refusals, CYCLES - 1);
    eprintln!(
        "software_only=true production_qualified=false cycles={CYCLES} frames={} receipts={} control_overload_refusals={} audio_refusals={} stale_handle_refusals={} last_generation={} available_producers=4/6 profile=48000Hz/Mono",
        CYCLES * CYCLE_FRAMES,
        oracle.receipts,
        oracle.control_overload_refusals,
        oracle.audio_refusals,
        oracle.stale_handle_refusals,
        oracle.generations.last().expect("generation")
    );
    assert!(oracle.generations.last().copied().expect("generation") > u32::from(u16::MAX));
    for partition in [1, 37, 64] {
        assert_eq!(churn(partition), oracle, "partition {partition}");
    }
}
