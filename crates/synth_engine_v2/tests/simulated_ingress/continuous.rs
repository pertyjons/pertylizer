use super::*;
use synth_engine_v2::{
    host::{
        ConnectionGeneration, EndpointId,
        input::{InputCapacity, InputLimits, SimulatedNoteInput},
        live::{
            AuditionId, AuditionOutcome, LiveInputStream, SwappingLiveStream, UpdateStatus,
            UpdateVersion,
        },
    },
    ingress::ReleaseCause,
    quantities::{ParameterValue, PreparedBytes},
    recording::notes::Midi1Input,
};
fn source() -> ConnectionGeneration {
    SimulatedNoteInput::new(
        EndpointId::new("continuous".into()).unwrap(),
        InputLimits {
            cells: InputCapacity::new(4).unwrap(),
            bytes: PreparedBytes::measured(65536),
        },
    )
    .unwrap()
    .begin()
    .unwrap()
}
fn live(plan: CompiledPlan, source: ConnectionGeneration, capacity: u32) -> LiveInputStream {
    let note = plan.resolve_note(ENVELOPE).unwrap();
    LiveInputStream::prepare(
        plan,
        common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono),
        note,
        &[source],
        EventCount::measured(capacity),
        PreparedBytes::measured(2_000_000),
    )
    .unwrap()
}
fn offer(
    live: &mut LiveInputStream,
    source: ConnectionGeneration,
    serial: u64,
    at: u64,
    bytes: [u8; 3],
) -> AuditionId {
    let id = AuditionId::new(source, serial).unwrap();
    live.queue(
        id,
        SampleTime::new(at),
        Midi1Input::from_bytes(bytes).unwrap(),
    )
    .unwrap();
    id
}
fn render(live: &mut LiveInputStream, frames: usize) -> Vec<f32> {
    let mut pcm = vec![0.0; frames];
    live.render(AudioBlockMut::new(&mut pcm, frames, ChannelLayout::Mono).unwrap())
        .unwrap();
    pcm
}
#[test]
fn ten_thousand_cycles_reuse_storage_without_reusing_audition_ids_or_fifo_position() {
    let source = source();
    let mut live = live(plan(), source, 4);
    let bytes = live.bytes();
    render(&mut live, 64);
    for cycle in 0..10_000 {
        let at = live.clock().as_u64();
        let on = offer(&mut live, source, cycle * 2 + 1, at, [0x90, 60, 100]);
        let off = offer(&mut live, source, cycle * 2 + 2, at + 17, [0x80, 60, 0]);
        let pcm = render(&mut live, 64);
        assert!(pcm[0] > 0.0);
        assert!(pcm[17..].iter().all(|v| *v == 0.0));
        assert!(matches!(
            live.take_outcome(on),
            Some(AuditionOutcome::Executed { .. })
        ));
        assert!(matches!(
            live.take_outcome(off),
            Some(AuditionOutcome::Executed { .. })
        ));
        assert_eq!(live.holds(), EventCount::NONE);
        assert_eq!(live.bytes(), bytes);
    }
    assert!(
        live.queue(
            AuditionId::new(source, 1).unwrap(),
            live.clock(),
            Midi1Input::from_bytes([0x90, 60, 100]).unwrap()
        )
        .is_err()
    );
}
#[test]
fn full_uncollected_outcomes_do_not_block_protected_stop() {
    let source = source();
    let mut live = live(plan(), source, 2);
    render(&mut live, 64);
    let _id = offer(&mut live, source, 1, 0, [0x90, 60, 100]);
    let _id = offer(&mut live, source, 2, 1, [0xb0, 64, 127]);
    render(&mut live, 64);
    assert!(
        live.queue(
            AuditionId::new(source, 3).unwrap(),
            live.clock(),
            Midi1Input::from_bytes([0x80, 60, 0]).unwrap()
        )
        .is_err()
    );
    live.end_at(live.clock(), ReleaseCause::Panic).unwrap();
    assert!(render(&mut live, 64).iter().all(|v| *v == 0.0));
    assert_eq!(live.holds(), EventCount::NONE);
    assert!(!live.has_sounding_obligations());
}
#[test]
fn bend_tracks_channel_sustain_and_is_inherited_by_new_notes() {
    fn run(channel: u8, bend_before: bool, partition: usize, sustained_bend: bool) -> Vec<f32> {
        let source = source();
        let mut live = live(pitched(live_only(4, 4)), source, 16);
        render(&mut live, 64);
        let events = if bend_before {
            vec![(0, [0xe0 | channel, 127, 127]), (0, [0x90, 60, 127])]
        } else {
            vec![(0, [0x90, 60, 127]), (0, [0xe0 | channel, 127, 127])]
        };
        for (index, (at, bytes)) in events.into_iter().enumerate() {
            let _id = offer(&mut live, source, index as u64 + 1, at, bytes);
        }
        let _id = offer(&mut live, source, 3, 64, [0xb0, 64, 127]);
        let _id = offer(&mut live, source, 4, 80, [0x80, 60, 0]);
        if sustained_bend {
            let _id = offer(&mut live, source, 5, 128, [0xe0 | channel, 0, 0]);
        }
        let _id = offer(&mut live, source, 6, 192, [0xb0, 64, 0]);
        let mut pcm = Vec::new();
        while pcm.len() < 256 {
            pcm.extend(render(&mut live, partition.min(256 - pcm.len())));
        }
        assert!(pcm[80..192].iter().any(|v| *v != 0.0));
        assert!(pcm[192..].iter().all(|v| *v == 0.0));
        pcm
    }
    let bent = run(0, true, 64, true);
    let unbent_sustain = run(0, true, 64, false);
    assert_eq!(&bent[..128], &unbent_sustain[..128]);
    assert_ne!(
        &bent[128..192],
        &unbent_sustain[128..192],
        "bend must move a sustained key-up voice"
    );
    assert_eq!(bent, run(0, false, 37, true));
    assert_ne!(
        bent,
        run(1, true, 64, true),
        "another MIDI channel cannot move this voice"
    );
}
#[test]
fn parameter_flood_coalesces_without_spending_release_custody_and_cancels_at_stop() {
    let source = source();
    let plan = plan();
    let slot = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_SUSTAIN)
        .unwrap();
    let gate = plan.note_targets()[0].parameter;
    let mut live = live(plan, source, 4);
    assert!(
        live.prepare_parameters(&[gate], PreparedBytes::measured(2_000_000))
            .is_err()
    );
    live.prepare_parameters(&[slot], PreparedBytes::measured(2_000_000))
        .unwrap();
    render(&mut live, 64);
    let _id = offer(&mut live, source, 1, 0, [0x90, 60, 127]);
    for version in 1..=10_000 {
        let replaced = live
            .update_parameter(
                slot,
                UpdateVersion::new(version).unwrap(),
                ParameterValue::new(0.5).unwrap(),
            )
            .unwrap();
        assert_eq!(
            replaced.map(UpdateVersion::as_u64),
            (version > 1).then_some(version - 1)
        );
    }
    let pcm = render(&mut live, 64);
    assert_eq!(pcm[0], 0.5);
    assert_eq!(
        live.parameter_status(slot),
        Some(UpdateStatus::Applied(UpdateVersion::new(10_000).unwrap()))
    );
    assert!(
        live.update_parameter(slot, UpdateVersion::new(1).unwrap(), ParameterValue::ZERO)
            .is_err()
    );
    live.update_parameter(
        slot,
        UpdateVersion::new(10_001).unwrap(),
        ParameterValue::ONE,
    )
    .unwrap();
    live.end_at(live.clock(), ReleaseCause::Stop).unwrap();
    assert!(render(&mut live, 64).iter().all(|v| *v == 0.0));
    assert_eq!(
        live.parameter_status(slot),
        Some(UpdateStatus::Cancelled(UpdateVersion::new(10_001).unwrap()))
    );
}
#[test]
fn running_swap_fades_releases_retires_and_preserves_old_key_tombstones() {
    let source = source();
    let graph = gated_constant(live_only(4, 4));
    let mut live = SwappingLiveStream::prepare(
        &graph,
        common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono),
        ENVELOPE,
        &[source],
        &[],
        EventCount::measured(16),
        PreparedBytes::measured(8_000_000),
    )
    .unwrap();
    let mut pcm = [0.0; 64];
    live.render(AudioBlockMut::new(&mut pcm, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    live.queue(
        AuditionId::new(source, 1).unwrap(),
        SampleTime::ZERO,
        Midi1Input::from_bytes([0x90, 60, 127]).unwrap(),
    )
    .unwrap();
    live.render(AudioBlockMut::new(&mut pcm, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(pcm, [1.0; 64]);
    let epoch = live.active().epoch();
    let old_plan = live.active().plan_id();
    let invalid = GraphIr::builder()
        .node(NodeId::new(99), IrNodeKind::Silence, ExecutionScope::Global)
        .build()
        .unwrap();
    assert!(live.prepare_candidate(&invalid, ENVELOPE, &[]).is_err());
    assert_eq!(live.active().plan_id(), old_plan);
    let new_plan = live.prepare_candidate(&graph, ENVELOPE, &[]).unwrap();
    assert!(live.prepare_candidate(&graph, ENVELOPE, &[]).is_err());
    live.render(AudioBlockMut::new(&mut pcm, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(live.active().plan_id(), new_plan);
    assert_ne!(live.active().epoch(), epoch);
    assert_eq!(pcm[0], 63.0 / 64.0);
    assert_eq!(pcm[63], 0.0);
    assert!(pcm.windows(2).all(|p| p[0] >= p[1]));
    assert!(
        live.prepare_candidate(&graph, ENVELOPE, &[]).is_err(),
        "retirement holds its candidate credit"
    );
    assert!(
        matches!(live.take_outcome(AuditionId::new(source, 1).unwrap()),
        Some(AuditionOutcome::Executed { epoch: executed_epoch, .. }) if executed_epoch == epoch)
    );
    assert!(live.collect_retired(|old| {
        assert_eq!(old.holds(), EventCount::NONE);
        assert!(!old.has_sounding_obligations());
        assert_eq!(old.outcomes().count(), 0);
    }));
    let at = live.active().clock();
    for (serial, bytes) in [(2, [0x90, 60, 127]), (3, [0x80, 60, 0])] {
        live.queue(
            AuditionId::new(source, serial).unwrap(),
            at,
            Midi1Input::from_bytes(bytes).unwrap(),
        )
        .unwrap();
    }
    live.render(AudioBlockMut::new(&mut pcm, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(
        pcm, [1.0; 64],
        "old physical release consumes the inherited tombstone"
    );
    live.end_at(live.active().clock(), ReleaseCause::Panic)
        .unwrap();
    live.render(AudioBlockMut::new(&mut pcm, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(pcm, [0.0; 64]);
}

#[test]
fn bend_overload_is_terminal_silent_and_never_reports_partial_execution() {
    let source = source();
    let mut live = live(pitched(live_only(8, 8)), source, 128);
    render(&mut live, 64);
    for serial in 1..=8 {
        let _id = offer(&mut live, source, serial, 0, [0x90, 60, 127]);
    }
    render(&mut live, 64);
    for serial in 9..=72 {
        let _id = offer(&mut live, source, serial, 64, [0xe0, 127, 127]);
    }
    let mut pcm = [1.0; 64];
    assert!(
        live.render(AudioBlockMut::new(&mut pcm, 64, ChannelLayout::Mono).unwrap())
            .is_err()
    );
    assert_eq!(pcm, [0.0; 64]);
    assert!(
        live.outcomes()
            .filter(|(id, _)| id.serial() >= 9)
            .all(|(_, outcome)| outcome == AuditionOutcome::Cancelled)
    );
    assert!(
        live.render(AudioBlockMut::new(&mut pcm, 64, ChannelLayout::Mono).unwrap())
            .is_err()
    );
}
#[test]
fn swap_output_and_release_are_equal_under_small_and_irregular_callbacks() {
    fn run(maximum: u64, partitions: &[usize]) -> Vec<f32> {
        let source = source();
        let graph = gated_constant(live_only(4, 4));
        let mut live = SwappingLiveStream::prepare(
            &graph,
            common::profile(maximum, ChannelLayout::Mono),
            ENVELOPE,
            &[source],
            &[],
            EventCount::measured(16),
            PreparedBytes::measured(8_000_000),
        )
        .unwrap();
        let mut step = 0;
        let mut render = |live: &mut SwappingLiveStream, count: usize| {
            let mut pcm = vec![0.0; count];
            let mut offset = 0;
            while offset < count {
                let frames = partitions[step % partitions.len()].min(count - offset);
                step += 1;
                live.render(
                    AudioBlockMut::new(
                        &mut pcm[offset..offset + frames],
                        frames,
                        ChannelLayout::Mono,
                    )
                    .unwrap(),
                )
                .unwrap();
                offset += frames;
            }
            pcm
        };
        render(&mut live, 64);
        live.queue(
            AuditionId::new(source, 1).unwrap(),
            SampleTime::ZERO,
            Midi1Input::from_bytes([0x90, 60, 127]).unwrap(),
        )
        .unwrap();
        render(&mut live, 64);
        let _plan = live.prepare_candidate(&graph, ENVELOPE, &[]).unwrap();
        let pcm = render(&mut live, 128);
        assert!(live.collect_retired(|old| assert!(!old.has_sounding_obligations())));
        pcm
    }
    assert_eq!(run(256, &[128]), run(16, &[3, 16, 1, 7]));
}

#[test]
fn outer_callback_failure_cancels_earlier_quantum_outcomes_and_parameter_versions() {
    let source = source();
    let graph = gated_constant(live_only(8, 8));
    let mut live = SwappingLiveStream::prepare(
        &graph,
        common::profile(256, ChannelLayout::Mono),
        ENVELOPE,
        &[source],
        &[(ENVELOPE, parameters::ENVELOPE_SUSTAIN)],
        EventCount::measured(128),
        PreparedBytes::measured(8_000_000),
    )
    .unwrap();
    let mut pcm = [0.0; 64];
    live.render(AudioBlockMut::new(&mut pcm, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    for serial in 1..=7 {
        live.queue(
            AuditionId::new(source, serial).unwrap(),
            SampleTime::ZERO,
            Midi1Input::from_bytes([0x90, 60, 127]).unwrap(),
        )
        .unwrap();
    }
    live.render(AudioBlockMut::new(&mut pcm, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    let on = AuditionId::new(source, 8).unwrap();
    live.queue(
        on,
        SampleTime::new(64),
        Midi1Input::from_bytes([0x90, 60, 127]).unwrap(),
    )
    .unwrap();
    for serial in 9..=72 {
        live.queue(
            AuditionId::new(source, serial).unwrap(),
            SampleTime::new(128),
            Midi1Input::from_bytes([0xe0, 127, 127]).unwrap(),
        )
        .unwrap();
    }
    let slot = live
        .active()
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_SUSTAIN)
        .unwrap();
    let version = UpdateVersion::new(1).unwrap();
    live.update_parameter(slot, version, ParameterValue::new(0.5).unwrap())
        .unwrap();
    let mut pcm = [1.0; 128];
    assert!(
        live.render(AudioBlockMut::new(&mut pcm, 128, ChannelLayout::Mono).unwrap())
            .is_err()
    );
    assert_eq!(pcm, [0.0; 128]);
    assert_eq!(live.take_outcome(on), Some(AuditionOutcome::Cancelled));
    assert_eq!(
        live.active().parameter_status(slot),
        Some(UpdateStatus::Cancelled(version))
    );
    assert!(
        matches!(
            live.take_outcome(AuditionId::new(source, 1).unwrap()),
            Some(AuditionOutcome::Executed { .. })
        ),
        "a previous successful callback keeps its outcome"
    );
}
#[test]
fn normal_stop_keeps_parameter_execution_from_an_earlier_quantum_of_the_same_callback() {
    let source = source();
    let plan = plan();
    let slot = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_SUSTAIN)
        .unwrap();
    let mut live = live(plan, source, 4);
    live.prepare_parameters(&[slot], PreparedBytes::measured(2_000_000))
        .unwrap();
    render(&mut live, 64);
    let version = UpdateVersion::new(1).unwrap();
    live.update_parameter(slot, version, ParameterValue::new(0.5).unwrap())
        .unwrap();
    live.end_at(SampleTime::new(64), ReleaseCause::Stop)
        .unwrap();
    render(&mut live, 128);
    assert_eq!(
        live.parameter_status(slot),
        Some(UpdateStatus::Applied(version))
    );
}
#[test]
fn external_quality_factor_updates_require_the_positive_resonance_domain() {
    use synth_engine_v2::quantities::{CutoffFrequency, Resonance};
    let graph = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(1000.0).unwrap(),
                resonance: Resonance::BUTTERWORTH,
            },
            ExecutionScope::Global,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(live_only(4, 4))
        .build()
        .unwrap();
    let plan = common::admit(
        &graph,
        common::profile(TOTAL_FRAMES as u64, ChannelLayout::Mono),
    );
    let slot = plan
        .resolve_parameter(SOURCE, parameters::FILTER_RESONANCE)
        .unwrap();
    let mut live = live(plan, source(), 4);
    live.prepare_parameters(&[slot], PreparedBytes::measured(2_000_000))
        .unwrap();
    let version = UpdateVersion::new(1).unwrap();
    for invalid in [0.0, -0.5] {
        assert!(
            live.update_parameter(slot, version, ParameterValue::new(invalid).unwrap())
                .is_err()
        );
    }
    assert!(
        live.update_parameter(slot, version, ParameterValue::new(0.01).unwrap())
            .is_ok()
    );
}

#[test]
fn one_million_real_note_identity_generations_reject_stale_handles() {
    use synth_engine_v2::identity::{IdentityTable, ProducerId, Resolution};
    let plan = plan();
    let note = plan.resolve_note(ENVELOPE).unwrap();
    let mut identities =
        IdentityTable::new(HeldNoteCount::measured(4), &[HeldNoteCount::measured(4)]).unwrap();
    let first = identities.mint(ProducerId::new(0), note).unwrap();
    assert_eq!(identities.release(first), Resolution::Live);
    let mut previous = first;
    for _ in 0..1_000_000 {
        let current = identities.mint(ProducerId::new(0), note).unwrap();
        assert_ne!(current, previous);
        assert_ne!(identities.resolve(first), Resolution::Live);
        assert_ne!(identities.resolve(previous), Resolution::Live);
        assert_eq!(identities.release(current), Resolution::Live);
        previous = current;
    }
    assert_eq!(identities.live(), 0);
    // The finite-space control must fail closed when the same mechanism exhausts.
    let mut finite = IdentityTable::with_generation_ceiling(
        HeldNoteCount::measured(1),
        &[HeldNoteCount::measured(1)],
        3,
    )
    .unwrap();
    for _ in 0..4 {
        let identity = finite.mint(ProducerId::new(0), note).unwrap();
        assert_eq!(finite.release(identity), Resolution::Live);
    }
    assert!(finite.mint(ProducerId::new(0), note).is_err());
    println!(
        "real_note_generations=1000000 stale_matches=0 finite_space_control=refused production_width_qualification=pending_representative_workload"
    );
}
