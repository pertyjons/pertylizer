//! Actual producer controls, including the suppressed-record and cap+1 falsifiers.
use super::*;
use crate::{
    ir::{
        InternalProducerDeclaration, RuntimeParameterEntry, RuntimeParameterProgram,
        RuntimeProgramId, parameters,
    },
    publish::ProducerClass,
    quantities::ParameterValue,
    render::AudioBlockMut,
};

fn programs() -> Vec<RuntimeParameterProgram> {
    let program = |id, targets: &[NodeId]| {
        RuntimeParameterProgram::new(
            RuntimeProgramId::new(id),
            FrameCount::new(128),
            targets
                .iter()
                .flat_map(|node| {
                    [
                        RuntimeParameterEntry::new(
                            *node,
                            parameters::TRIM_LEVEL,
                            FrameCount::new(0),
                            ParameterValue::ZERO,
                        ),
                        RuntimeParameterEntry::new(
                            *node,
                            parameters::TRIM_LEVEL,
                            FrameCount::new(65),
                            ParameterValue::ONE,
                        ),
                    ]
                })
                .collect(),
        )
        .unwrap()
    };
    // Stable target sorting would interleave A, B, A. Producer order is A, A, B.
    vec![
        program(1, &[TRIM, NodeId::new(52)]),
        program(2, &[NodeId::new(51)]),
    ]
}
fn internal() -> Vec<InternalProducerDeclaration> {
    vec![
        InternalProducerDeclaration::periodic_delay_clear(
            DELAY,
            FrameCount::new(97),
            FrameCount::new(2),
        )
        .unwrap(),
    ]
}
fn arm_programs(profile: HostProfile, authored: bool, clears: bool) -> Armed {
    arm_configuration(
        profile,
        if authored { programs() } else { vec![] },
        if clears { internal() } else { vec![] },
        SampleTime::ZERO,
    )
}
fn arm_configuration(
    profile: HostProfile,
    authored: Vec<RuntimeParameterProgram>,
    clears: Vec<InternalProducerDeclaration>,
    start: SampleTime,
) -> Armed {
    let targets = targets_with_programs(profile, 0.8, authored, clears);
    let prepared = MixedJoinedStream::open(targets, StreamAnchor::new(start, PlanPosition::ZERO))
        .unwrap()
        .prepare_initial()
        .unwrap();
    let history = prepared
        .prepare_history(
            start.checked_add(FrameCount::QUANTUM).unwrap(),
            PlanPosition::ZERO,
        )
        .unwrap();
    let suffix = prepared.prepare_suffix(history).unwrap();
    let candidate = prepared.stamp_suffix(suffix).unwrap();
    let capacity = MixedJoinedPrepared::transfer_capacity(&profile).get();
    prepared
        .arm_transfer_rehearsal(
            candidate,
            &profile,
            Ring::new(capacity),
            Ring::new(capacity),
        )
        .unwrap()
}
fn render(armed: &mut Armed, frames: usize) -> Vec<f32> {
    let mut block = vec![0.0; frames];
    let allocations =
        count_allocs(|| callback(&mut armed.audio, &mut armed.audio_port, &mut block).unwrap());
    assert_eq!(allocations, 0);
    while armed
        .control
        .collect(&mut armed.control_port)
        .unwrap()
        .is_some()
    {}
    block
}
#[test]
fn six_real_producers_share_a_quantum_and_measure_actual_combined_occupancy() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let mut armed = arm_programs(profile(hz), true, true);
        assert_eq!(
            armed
                .audio
                .publication_usage()
                .high_water(ProducerClass::AuthoredRuntime),
            None
        );
        let (_, handle) = armed
            .control
            .submit_note_on(&mut armed.control_port, note_on(65, 67))
            .unwrap();
        let _up = armed
            .control
            .submit_key_up(
                &mut armed.control_port,
                MixedSourceKeyUp {
                    at: SampleTime::new(66),
                    handle,
                },
            )
            .unwrap();
        render(&mut armed, 64);
        render(&mut armed, 64);
        render(&mut armed, 64);
        let same_sample: Vec<_> = armed
            .audio
            .inner
            .arbiter
            .published_for_test()
            .iter()
            .filter(|event| event.envelope().time() == SampleTime::new(65))
            .map(|event| event.payload())
            .collect();
        let plan = armed.audio.inner.audio.renderer.plan();
        assert!(matches!(
            same_sample.first(),
            Some(crate::render::EventPayload::Note { .. })
        ));
        let targets = [TRIM, NodeId::new(52), NodeId::new(51)];
        for (event, node) in same_sample.iter().skip(1).zip(targets) {
            assert!(
                matches!(event, crate::render::EventPayload::SetParameter { slot, .. } if *slot == plan.resolve_parameter(node, parameters::TRIM_LEVEL).unwrap())
            );
        }
        assert_eq!(same_sample.len(), 5);
        assert!(matches!(
            same_sample.last(),
            Some(crate::render::EventPayload::Note { .. })
        ));
        let usage = armed.audio.publication_usage();
        let (time, counts) = usage.last_quantum().unwrap();
        assert_eq!(time, SampleTime::new(64));
        for class in ProducerClass::ALL {
            assert!(
                counts[class.index()].unwrap().get() > 0,
                "{class:?}: {counts:?}"
            );
        }
        let total: u32 = counts.iter().map(|count| count.unwrap().get()).sum();
        assert!(usage.high_water_combined_total().unwrap().get() >= total);
        assert!(usage.high_water_combined_total().unwrap() > usage.high_water_external_total());
        for _ in 0..32 {
            render(&mut armed, 97);
        }
        let usage = armed.audio.publication_usage();
        let teardown = armed
            .control
            .join(
                armed.audio,
                Some(armed.control_port),
                Some(armed.audio_port),
            )
            .unwrap();
        assert_eq!(teardown.publication_usage(), usage);
    }
}
#[test]
fn runtime_parameter_arena_is_the_only_cause_of_its_audible_override() {
    let mut actual = arm_programs(profile(48_000.0), true, false);
    let mut suppressed = arm_programs(profile(48_000.0), true, false);
    suppressed
        .audio
        .inner
        .audio
        .renderer
        .producer_runtime_for_test()
        .suppress_authored = true;
    let mut changed = false;
    for _ in 0..16 {
        changed |= render(&mut actual, 64) != render(&mut suppressed, 64);
    }
    assert!(
        changed,
        "removing actual publication must change the gain result"
    );
    assert_eq!(
        suppressed
            .audio
            .publication_usage()
            .high_water(ProducerClass::AuthoredRuntime),
        Some(EventCount::NONE)
    );
    assert!(
        actual
            .audio
            .publication_usage()
            .high_water(ProducerClass::AuthoredRuntime)
            .unwrap()
            .get()
            >= 2
    );
}
#[test]
fn internal_arena_is_the_only_cause_of_a_delay_history_clear() {
    let mut actual = arm_programs(profile(48_000.0), false, true);
    let mut suppressed = arm_programs(profile(48_000.0), false, true);
    suppressed
        .audio
        .inner
        .audio
        .renderer
        .producer_runtime_for_test()
        .suppress_internal = true;
    let mut changed = false;
    for _ in 0..32 {
        changed |= render(&mut actual, 64) != render(&mut suppressed, 64);
    }
    assert!(
        changed,
        "suppressing the clear record must preserve a different warm history"
    );
    assert_eq!(
        suppressed
            .audio
            .publication_usage()
            .high_water(ProducerClass::Internal),
        Some(EventCount::NONE)
    );
}
#[test]
fn actual_internal_cap_plus_one_ends_the_epoch_and_silences_the_entire_callback() {
    let mut armed = arm_programs(profile(48_000.0), true, true);
    armed
        .audio
        .inner
        .audio
        .renderer
        .producer_runtime_for_test()
        .internal_limit = Some(EventCount::NONE);
    let mut block = [0.5; 256];
    let allocations = count_allocs(|| {
        assert!(callback(&mut armed.audio, &mut armed.audio_port, &mut block).is_err())
    });
    assert_eq!(allocations, 0);
    assert!(block.iter().all(|sample| *sample == 0.0));
    let renderer = &armed.audio.inner.audio.renderer;
    assert!(renderer.diagnostics().needs_reprepare());
    assert_eq!(renderer.diagnostics().publication_faults(), 0);
    assert_eq!(renderer.diagnostics().internal_producer_faults(), 1);
    assert_eq!(
        armed
            .audio
            .publication_usage()
            .high_water(ProducerClass::Internal),
        Some(EventCount::NONE)
    );
    assert_eq!(renderer.carry_frames(), 0);
    assert!(callback(&mut armed.audio, &mut armed.audio_port, &mut block).is_err());
    assert!(block.iter().all(|sample| *sample == 0.0));
}
#[test]
fn programmed_trim_base_change_is_a_cold_plan_refusal() {
    let mut armed = arm_programs(profile(48_000.0), true, true);
    render(&mut armed, 256);
    let at = SampleTime::new(armed.audio.inner.audio.renderer.clock().as_u64() + 128);
    let error = armed
        .control
        .prepare_continuation(
            targets_with_programs(profile(48_000.0), 0.7, programs(), internal()),
            || Some(at),
            BYTES,
            RECORDS,
        )
        .unwrap_err();
    assert!(
        matches!(
            error.0,
            MixedContinuationPrepareError::Proof(crate::plan::ContinuationError::Plan)
        ),
        "{:?}",
        error.0
    );
    assert!(armed.audio.fault().is_none());
}
#[test]
fn identical_producer_continuation_preserves_phase_and_initialized_value() {
    let host = profile(48_000.0);
    let mut armed = arm_programs(host, true, true);
    let mut reference = arm_programs(host, true, true);
    for _ in 0..12 {
        advance(&mut armed, &mut reference, 64);
    }
    let at = SampleTime::new(armed.audio.inner.audio.renderer.clock().as_u64() + 128);
    // No program has an ordinary event at B. Reinitialization would therefore
    // add a visible extra burst even though the values and audio stay unchanged.
    assert!(programs().iter().all(|program| {
        program
            .entries()
            .iter()
            .all(|entry| entry.phase().as_u64() != at.as_u64() % program.period().as_u64())
    }));
    let packet = armed
        .control
        .prepare_continuation(
            targets_with_programs(host, 0.8, programs(), internal()),
            || Some(at),
            BYTES,
            RECORDS,
        )
        .unwrap();
    armed.audio.offer_continuation(packet).unwrap();
    for frames in [1, 63, 7, 121, 64, 128, 3, 253] {
        advance(&mut armed, &mut reference, frames);
        let parameters = |owner: &Armed| {
            owner
                .audio
                .inner
                .arbiter
                .published_for_test()
                .iter()
                .filter_map(|event| match event.payload() {
                    crate::render::EventPayload::SetParameter { slot, value } => {
                        Some((event.envelope().time(), slot.index(), value))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            parameters(&armed),
            parameters(&reference),
            "continuation must not add an initialization burst at {at:?}"
        );
        let authored = |owner: &Armed| {
            owner
                .audio
                .publication_usage()
                .last_quantum()
                .map(|(_, counts)| counts[ProducerClass::AuthoredRuntime.index()])
        };
        assert_eq!(authored(&armed), authored(&reference));
    }
    let packet = armed.audio.take_retired_continuation().unwrap();
    let returned = armed.control.collect_continuation(packet).unwrap();
    assert!(matches!(
        returned.outcome(),
        Some(MixedContinuationOutcome::InstalledRendered { .. })
    ));
    assert!(
        armed
            .audio
            .inner
            .audio
            .renderer
            .producer_runtime_for_test()
            .initialized
    );
}

#[test]
fn absent_and_installed_idle_producers_are_distinct_even_after_carry_callbacks() {
    let mut absent = arm_programs(profile(48_000.0), false, false);
    let mut present = arm_programs(profile(48_000.0), true, true);
    present
        .audio
        .inner
        .audio
        .renderer
        .producer_runtime_for_test()
        .suppress_authored = true;
    present
        .audio
        .inner
        .audio
        .renderer
        .producer_runtime_for_test()
        .suppress_internal = true;
    render(&mut absent, 1);
    render(&mut present, 1);
    assert_eq!(
        present
            .audio
            .publication_usage()
            .high_water_combined_total(),
        None
    );
    for _ in 0..4 {
        render(&mut absent, 64);
        render(&mut present, 64);
    }
    for class in [ProducerClass::AuthoredRuntime, ProducerClass::Internal] {
        assert_eq!(absent.audio.publication_usage().high_water(class), None);
        assert_eq!(
            present.audio.publication_usage().high_water(class),
            Some(EventCount::NONE)
        );
    }
    let before = present.audio.publication_usage();
    // Each one-frame step delivers carry until a new quantum opens. None may
    // manufacture a new zero observation for an unopened quantum.
    let clock = present.audio.inner.audio.renderer.clock();
    render(&mut present, 1);
    assert_eq!(present.audio.inner.audio.renderer.clock(), clock);
    let after = present.audio.publication_usage();
    assert_eq!(
        after.high_water_combined_total(),
        before.high_water_combined_total()
    );
    assert_eq!(
        after.high_water_external_total(),
        before.high_water_external_total()
    );
    for class in ProducerClass::ALL {
        assert_eq!(after.high_water(class), before.high_water(class));
    }
    assert_eq!(after.last_quantum(), None);
}

#[test]
fn ordinary_owner_refuses_real_programs_instead_of_silently_omitting_them() {
    for (authored, clears) in [(true, false), (false, true)] {
        let host = profile(48_000.0);
        let targets = targets_with_programs(
            host,
            0.8,
            if authored { programs() } else { vec![] },
            if clears { internal() } else { vec![] },
        );
        assert!(
            crate::stream::StreamControl::open(
                targets.plan().clone(),
                StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO)
            )
            .is_err()
        );
    }
}

#[test]
#[ignore = "run explicitly in a representative environment; measurements do not select production limits"]
fn calibrate_real_six_producer_software_load() {
    use sha2::{Digest, Sha256};
    use std::time::Instant;
    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .unwrap();
    assert!(rustc.status.success());
    println!(
        "environment os={} arch={} parallelism={} compiler={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().unwrap(),
        String::from_utf8(rustc.stdout).unwrap().trim()
    );
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let host = profile(hz);
        let mut armed = arm_programs(host, true, true);
        let fixture = format!(
            "six-producer-software-v1;profile={host:?};authored={:?};internal={:?};prepared={:?};callback=64;iterations=4096;fixture_source={}\n{}",
            programs(),
            internal(),
            armed.audio.inner.audio.renderer.plan().prepared_nodes(),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/stream/mixed/transfer/continuation/tests.rs"
            )),
            include_str!("producer_tests.rs")
        );
        let digest: String = Sha256::digest(fixture.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let (_, handle) = armed
            .control
            .submit_note_on(&mut armed.control_port, note_on(64, 67))
            .unwrap();
        let _up = armed
            .control
            .submit_key_up(
                &mut armed.control_port,
                MixedSourceKeyUp {
                    at: SampleTime::new(65),
                    handle,
                },
            )
            .unwrap();
        let mut durations = Vec::with_capacity(4096);
        let mut block = [0.0; 64];
        for _ in 0..4096 {
            let start = Instant::now();
            callback(&mut armed.audio, &mut armed.audio_port, &mut block).unwrap();
            durations.push(start.elapsed());
            while armed
                .control
                .collect(&mut armed.control_port)
                .unwrap()
                .is_some()
            {}
        }
        durations.sort_unstable();
        let usage = armed.audio.publication_usage();
        for class in ProducerClass::ALL {
            assert!(usage.high_water(class).unwrap().get() > 0);
        }
        println!(
            "fixture_sha256={digest} rate={hz} callback_frames=64 callbacks=4096 callback_ns_p50={} callback_ns_p99={} callback_ns_max={} producer_peaks={:?} external_peak={} combined_peak={} internal_clear_sample_writes_q={} cpu_cost_model=excludes-clears qualification=software-only",
            durations[2047].as_nanos(),
            durations[4055].as_nanos(),
            durations[4095].as_nanos(),
            ProducerClass::ALL.map(|class| usage.high_water(class)),
            usage.high_water_external_total().get(),
            usage.high_water_combined_total().unwrap().get(),
            armed
                .audio
                .inner
                .audio
                .renderer
                .plan()
                .internal_clear_sample_writes_per_quantum()
                .as_usize()
        );
        let teardown = armed
            .control
            .join(
                armed.audio,
                Some(armed.control_port),
                Some(armed.audio_port),
            )
            .unwrap();
        assert_eq!(teardown.publication_usage(), usage);
    }
}

#[test]
fn actual_authored_cap_plus_one_silences_the_entire_callback() {
    let mut armed = arm_programs(profile(48_000.0), true, true);
    // Program A really emits two initialization records; admit only one in this
    // mutation control. No fake arbiter charge or extra synthetic producer is used.
    armed
        .audio
        .inner
        .audio
        .renderer
        .producer_runtime_for_test()
        .authored_limit = Some(EventCount::measured(1));
    let mut block = [0.5; 256];
    let allocations = count_allocs(|| {
        assert!(callback(&mut armed.audio, &mut armed.audio_port, &mut block).is_err())
    });
    assert_eq!(allocations, 0);
    assert!(block.iter().all(|sample| *sample == 0.0));
    assert!(
        armed
            .audio
            .inner
            .audio
            .renderer
            .diagnostics()
            .needs_reprepare()
    );
    assert_eq!(
        armed
            .audio
            .inner
            .audio
            .renderer
            .diagnostics()
            .publication_faults(),
        0
    );
    assert_eq!(
        armed
            .audio
            .inner
            .audio
            .renderer
            .diagnostics()
            .authored_source_faults(),
        1
    );
    assert_eq!(
        armed.audio.publication_usage().last_quantum().unwrap().1
            [ProducerClass::AuthoredRuntime.index()],
        Some(EventCount::measured(1))
    );
    assert_eq!(armed.audio.inner.audio.renderer.carry_frames(), 0);
    assert!(callback(&mut armed.audio, &mut armed.audio_port, &mut block).is_err());
}

#[test]
fn a_fresh_reset_clock_publishes_the_value_in_force_at_its_absolute_sample_phase() {
    let program = RuntimeParameterProgram::new(
        RuntimeProgramId::new(1),
        FrameCount::new(191),
        vec![
            RuntimeParameterEntry::new(
                TRIM,
                parameters::TRIM_LEVEL,
                FrameCount::new(0),
                ParameterValue::ZERO,
            ),
            RuntimeParameterEntry::new(
                TRIM,
                parameters::TRIM_LEVEL,
                FrameCount::new(64),
                ParameterValue::ONE,
            ),
        ],
    )
    .unwrap();
    let mut armed = arm_configuration(
        profile(48_000.0),
        vec![program],
        internal(),
        SampleTime::new(128),
    );
    // Use the actual fresh-clock operation used by whole-owner reset. A plan
    // anchor alone maps plan positions and never changes the render clock.
    armed
        .audio
        .inner
        .audio
        .renderer
        .install_stopped_clock(StreamAnchor::new(SampleTime::new(128), PlanPosition::ZERO));
    render(&mut armed, 64);
    let events = armed.audio.inner.arbiter.published_for_test();
    assert!(
        events
            .iter()
            .any(|event| event.envelope().time() == SampleTime::new(128)
                && matches!(
                    event.payload(),
                    crate::render::EventPayload::SetParameter {
                        value: ParameterValue::ONE,
                        ..
                    }
                ))
    );
}

#[test]
fn stop_and_seek_leave_absolute_producer_phase_unchanged() {
    let host = profile(48_000.0);
    let mut actual = arm_programs(host, true, true);
    let mut reference = arm_programs(host, true, true);
    for _ in 0..4 {
        render(&mut actual, 64);
        render(&mut reference, 64);
    }
    let now = actual.audio.inner.audio.renderer.clock();
    let at = now.checked_add(FrameCount::new(128)).unwrap();
    let _stop = actual.audio.schedule_stop(at).unwrap();
    for step in 0..24 {
        if step == 8 {
            let at = actual
                .audio
                .inner
                .audio
                .renderer
                .clock()
                .checked_add(FrameCount::new(128))
                .unwrap();
            let offer = actual
                .control
                .prepare_activation(
                    at,
                    PlanPosition::new(8192),
                    at,
                    crate::stream::MixedActivationKind::Seek,
                )
                .unwrap();
            actual.audio.offer_activation(offer).unwrap();
        }
        render(&mut actual, 64);
        render(&mut reference, 64);
        let parameters = |armed: &Armed| {
            armed
                .audio
                .inner
                .arbiter
                .published_for_test()
                .iter()
                .filter_map(|event| match event.payload() {
                    crate::render::EventPayload::SetParameter { slot, value } => {
                        Some((event.envelope().time(), slot.index(), value))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(parameters(&actual), parameters(&reference));
        let internal_count = |armed: &Armed| {
            armed.audio.publication_usage().last_quantum().unwrap().1
                [ProducerClass::Internal.index()]
        };
        assert_eq!(internal_count(&actual), internal_count(&reference));
        if let Some(retired) = actual.audio.take_retired_activation() {
            let _receipt = actual.control.collect_activation(retired).unwrap();
        }
    }
}

#[test]
fn actual_authored_and_internal_emitters_reach_their_shares_without_borrowing_release_credit() {
    let host = profile(48_000.0);
    let authored_share = host
        .limits()
        .events()
        .shares()
        .authored_runtime_event_share();
    let internal_share = host.limits().events().shares().internal_event_share();
    assert!(authored_share.get() > 1 && authored_share.get() <= crate::time::QUANTUM_FRAMES);
    assert_eq!(crate::time::QUANTUM_FRAMES % internal_share.get(), 0);
    let program = RuntimeParameterProgram::new(
        RuntimeProgramId::new(1),
        FrameCount::QUANTUM,
        (1..authored_share.get())
            .map(|phase| {
                RuntimeParameterEntry::new(
                    TRIM,
                    parameters::TRIM_LEVEL,
                    FrameCount::new(u64::from(phase)),
                    if phase % 2 == 0 {
                        ParameterValue::ONE
                    } else {
                        ParameterValue::ZERO
                    },
                )
            })
            .collect(),
    )
    .unwrap();
    assert_eq!(program.destination_occupancy(), authored_share);
    let clear = InternalProducerDeclaration::periodic_delay_clear(
        DELAY,
        FrameCount::new(u64::from(
            crate::time::QUANTUM_FRAMES / internal_share.get(),
        )),
        FrameCount::new(0),
    )
    .unwrap();
    assert_eq!(clear.per_quantum(), internal_share);
    let mut armed = arm_configuration(host, vec![program], vec![clear], SampleTime::ZERO);
    let (_, handle) = armed
        .control
        .submit_note_on(&mut armed.control_port, note_on(65, 67))
        .unwrap();
    let _up = armed
        .control
        .submit_key_up(
            &mut armed.control_port,
            MixedSourceKeyUp {
                at: SampleTime::new(66),
                handle,
            },
        )
        .unwrap();
    for _ in 0..8 {
        render(&mut armed, 64);
    }
    let usage = armed.audio.publication_usage();
    assert_eq!(
        usage.high_water(ProducerClass::AuthoredRuntime),
        Some(authored_share)
    );
    assert_eq!(
        usage.high_water(ProducerClass::Internal),
        Some(internal_share)
    );
    assert!(usage.high_water(ProducerClass::Release).unwrap().get() > 0);
    assert!(armed.audio.fault().is_none());
    let teardown = armed
        .control
        .join(
            armed.audio,
            Some(armed.control_port),
            Some(armed.audio_port),
        )
        .unwrap();
    assert_eq!(teardown.publication_usage(), usage);
}

#[test]
fn running_whole_owner_reset_starts_both_real_producers_at_the_absolute_boundary_phase() {
    use crate::stream::{MixedBridgeResetAudio, MixedBridgeResetOwner};
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let host = profile(hz);
        let mut old = arm_programs(host, true, true);
        for _ in 0..4 {
            render(&mut old, 64);
        }
        let old_plan = old.audio.inner.audio.renderer.plan().id();
        let program = RuntimeParameterProgram::new(
            RuntimeProgramId::new(1),
            FrameCount::new(191),
            vec![
                RuntimeParameterEntry::new(
                    TRIM,
                    parameters::TRIM_LEVEL,
                    FrameCount::new(0),
                    ParameterValue::ZERO,
                ),
                RuntimeParameterEntry::new(
                    TRIM,
                    parameters::TRIM_LEVEL,
                    FrameCount::new(64),
                    ParameterValue::ONE,
                ),
            ],
        )
        .unwrap();
        let at = SampleTime::new(320);
        let next = arm_configuration(host, vec![program], internal(), SampleTime::new(256));
        let new_plan = next.audio.inner.audio.renderer.plan().id();
        let mut wrapper = MixedBridgeResetAudio::new(
            MixedBridgeResetOwner::new(old.audio, old.audio_port).unwrap(),
        );
        let successor = MixedBridgeResetOwner::new(next.audio, next.audio_port).unwrap();
        assert_eq!(
            count_allocs(|| wrapper.offer_running_reset(successor, at).unwrap()),
            0
        );
        let mut block = [0.0; 64];
        for _ in 0..3 {
            assert_eq!(
                count_allocs(|| wrapper
                    .render(AudioBlockMut::new(&mut block, 64, ChannelLayout::Mono).unwrap())
                    .unwrap()),
                0
            );
        }
        assert_ne!(old_plan, new_plan);
        assert_eq!(wrapper.plan_id(), new_plan);
        let retired = wrapper.take_retired().unwrap();
        assert_eq!(retired.receipt().at(), at);
        let (audio, port) = retired.into_owner().into_parts();
        let old_teardown = old
            .control
            .join(audio, Some(old.control_port), Some(port))
            .unwrap();
        assert!(old_teardown.audio_fault().is_none());
        let (active, secondary) = wrapper.into_owners();
        assert!(secondary.is_none());
        let (audio, port) = active.into_parts();
        let first = audio.inner.arbiter.published_for_test();
        assert!(first.iter().any(|event| event.envelope().time() == at && matches!(event.payload(), crate::render::EventPayload::SetParameter { value, .. } if value == ParameterValue::ONE)));
        assert_eq!(
            audio
                .publication_usage()
                .high_water(ProducerClass::Internal),
            Some(EventCount::NONE)
        );
        let next_teardown = next
            .control
            .join(audio, Some(next.control_port), Some(port))
            .unwrap();
        assert!(next_teardown.audio_fault().is_none());
    }
}

#[test]
fn an_internal_partial_overrun_reports_only_accepted_records_and_the_same_quantum_total() {
    let clear = InternalProducerDeclaration::periodic_delay_clear(
        DELAY,
        FrameCount::new(4),
        FrameCount::new(0),
    )
    .unwrap();
    let mut armed = arm_configuration(profile(48_000.0), programs(), vec![clear], SampleTime::ZERO);
    armed
        .audio
        .inner
        .audio
        .renderer
        .producer_runtime_for_test()
        .internal_limit = Some(EventCount::measured(1));
    let mut block = [0.5; 256];
    assert!(callback(&mut armed.audio, &mut armed.audio_port, &mut block).is_err());
    assert!(block.iter().all(|sample| *sample == 0.0));
    let usage = armed.audio.publication_usage();
    let (time, counts) = usage.last_quantum().unwrap();
    assert_eq!(time, SampleTime::ZERO);
    assert_eq!(
        counts[ProducerClass::Internal.index()],
        Some(EventCount::measured(1))
    );
    assert_eq!(
        usage.high_water(ProducerClass::Internal),
        Some(EventCount::measured(1))
    );
    assert_eq!(
        usage.high_water_combined_total().unwrap().get(),
        usage.high_water_external_total().get() + 1
    );
    assert_eq!(
        armed
            .audio
            .inner
            .audio
            .renderer
            .diagnostics()
            .internal_producer_faults(),
        1
    );
    assert_eq!(
        armed
            .audio
            .inner
            .audio
            .renderer
            .diagnostics()
            .publication_faults(),
        0
    );
}

#[test]
fn periodic_clear_work_is_reported_as_excluded_from_the_cpu_prediction() {
    let host = profile(192_000.0);
    let targets = targets_with_programs(host, 0.8, programs(), internal());
    // The prepared native delay owns two two-second lines at this rate.
    assert_eq!(
        targets
            .plan()
            .internal_clear_sample_writes_per_quantum()
            .as_usize(),
        768_000
    );
    let missing = PlanDeclarations {
        internal_producers: vec![
            InternalProducerDeclaration::periodic_delay_clear(
                NodeId::new(999),
                FrameCount::new(97),
                FrameCount::new(2),
            )
            .unwrap(),
        ],
        ..PlanDeclarations::default()
    };
    assert!(matches!(
        crate::producer_programs::ProducerPrograms::prepare(&missing, targets.plan()),
        Err(crate::diagnostics::CompileError::ProducerProgram {
            reason: "internal target node does not exist"
        })
    ));
}

#[test]
fn a_combined_total_refusal_does_not_record_the_denied_internal_clear() {
    let mut armed = arm_programs(profile(48_000.0), true, true);
    // The outgoing prefix emits two compiled notes before the adoption at 64;
    // three authored initialization records make five accepted external charges.
    // The next real clear must be denied before it occupies the arena.
    armed
        .audio
        .inner
        .audio
        .renderer
        .producer_runtime_for_test()
        .combined_limit = Some(EventCount::measured(5));
    let mut block = [0.5; 256];
    let allocations = count_allocs(|| {
        assert!(callback(&mut armed.audio, &mut armed.audio_port, &mut block).is_err())
    });
    assert_eq!(allocations, 0);
    assert!(block.iter().all(|sample| *sample == 0.0));
    let usage = armed.audio.publication_usage();
    let (time, counts) = usage.last_quantum().unwrap();
    assert_eq!(time, SampleTime::ZERO);
    assert_eq!(
        counts[ProducerClass::Compiled.index()],
        Some(EventCount::measured(2))
    );
    assert_eq!(
        counts[ProducerClass::AuthoredRuntime.index()],
        Some(EventCount::measured(3))
    );
    assert_eq!(usage.high_water_external_total(), EventCount::measured(5));
    assert_eq!(
        usage.high_water(ProducerClass::Internal),
        Some(EventCount::NONE)
    );
    assert_eq!(
        usage.last_quantum().unwrap().1[ProducerClass::Internal.index()],
        Some(EventCount::NONE)
    );
    assert_eq!(
        usage.high_water_combined_total(),
        Some(EventCount::measured(5))
    );
    assert_eq!(
        armed
            .audio
            .inner
            .audio
            .renderer
            .producer_runtime_for_test()
            .length,
        0
    );
    assert_eq!(
        armed
            .audio
            .inner
            .audio
            .renderer
            .diagnostics()
            .internal_producer_faults(),
        1
    );
    assert_eq!(
        armed
            .audio
            .inner
            .audio
            .renderer
            .diagnostics()
            .publication_faults(),
        0
    );
}
