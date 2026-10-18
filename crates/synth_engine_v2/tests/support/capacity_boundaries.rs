//! Actual preparation/admission boundaries, distinct from conservative frozen B.
use super::*;
use synth_engine_v2::{
    diagnostics::CompileError,
    profile::{ProducerShares, ProfileError},
    publish::ProducerClass,
    report::{ResourceAmount, ResourceField},
};

fn with_holds_and_depth(base: HostProfile, holds: EventCount, depth: EventCount) -> HostProfile {
    let old = base.limits().events();
    let shares = old.shares();
    let shares = ProducerShares::new(
        shares.compiled_event_share(),
        shares.authored_runtime_event_share(),
        shares.live_event_share(),
        shares.session_event_share(),
        shares.internal_event_share(),
        shares.release_event_share(),
        holds,
    )
    .expect("boundary shares");
    crate::frozen_capacity::replace_events(
        base,
        shares,
        depth,
        old.max_scheduled_events_in_flight(),
    )
    .expect("boundary profile")
}

fn fence(setup: &Setup) -> MixedTransferFence {
    MixedTransferFence {
        stage_cells: MixedTransferPlaces::new(setup.cells),
        held_onsets: setup
            .declared_holds
            .map_or_else(source_holds, MixedTransferPlaces::new),
        peer_wait: FrameCount::new(setup.peer_wait),
    }
}

pub(super) fn run(profile: HostProfile, make_setup: &impl Fn() -> Setup) {
    hold_partition(profile);
    executable_share_floors(profile);
    transfer_and_performance(profile, make_setup);
    stage_reservation(profile, make_setup);
}

fn hold_partition(profile: HostProfile) {
    let depth = profile
        .limits()
        .events()
        .queues()
        .performance_ingress_capacity();
    for holds in [4, 3] {
        let boundary = with_holds_and_depth(profile, EventCount::measured(holds), depth);
        let result = compile(
            &instrument_inserts::ir_with_layout(
                Some(instrument_inserts::live()),
                profile.capabilities().channel_layout(),
            ),
            &RenderConfig::new(boundary),
        )
        .into_plan();
        if holds == 4 {
            let _plan = result.expect("G4 graph at exact hold boundary");
        } else {
            assert!(matches!(result,Err(CompileError::LimitExceeded {
                field:ResourceField::ReleaseHoldCapacity,
                requested:ResourceAmount::Events(requested),available:ResourceAmount::Events(available),..
            }) if requested==EventCount::measured(4) && available==EventCount::measured(3)));
        }
    }
}

fn transfer_and_performance(profile: HostProfile, make_setup: &impl Fn() -> Setup) {
    let setup = make_setup();
    let mut raw = raw_owner_with_budgets(
        setup.raw_cells,
        profile.capabilities().sample_rate(),
        profile.capabilities().maximum_block_size(),
        setup.raw_commands,
        setup.raw_actions,
        setup.raw_action_bytes,
    );
    let sources = MixedTransferSources::new(raw.generations[FIRST], raw.generations[SECOND])
        .expect("two real generations");
    let c = usize::try_from(
        profile
            .limits()
            .events()
            .queues()
            .performance_ingress_capacity()
            .get(),
    )
    .expect("depth");
    let n = c.checked_add(1).expect("protected Panic slot");
    let (owner, candidate) = saved_prepared_with_profile(at(0), profile);
    let required = MixedTransferPlaces::new(n);
    let reported = MixedTransferPlaces::new(c);
    let refusal = owner
        .arm_bridge_rehearsal(
            candidate,
            &profile,
            sources,
            fence(&setup),
            HostQueue::<MixedTransferCommand>::new(c),
            HostQueue::<MixedTransferResult>::new(n),
        )
        .expect_err("C lacks command Panic place");
    assert_eq!(
        refusal.reason(),
        MixedTransferArmError::CommandCapacity { required, reported }
    );
    let (owner, candidate, commands, results) = refusal.into_parts();
    assert_eq!(commands.ring.occupied_len(), 0);
    assert_eq!(results.ring.occupied_len(), 0);
    let refusal = owner
        .arm_bridge_rehearsal(
            candidate,
            &profile,
            sources,
            fence(&setup),
            HostQueue::<MixedTransferCommand>::new(n),
            HostQueue::<MixedTransferResult>::new(c),
        )
        .expect_err("C lacks result Panic place");
    assert_eq!(
        refusal.reason(),
        MixedTransferArmError::ResultCapacity { required, reported }
    );
    let (owner, candidate, commands, results) = refusal.into_parts();
    assert_eq!(commands.ring.occupied_len(), 0);
    assert_eq!(results.ring.occupied_len(), 0);
    let armed = owner
        .arm_bridge_rehearsal(
            candidate,
            &profile,
            sources,
            fence(&setup),
            commands,
            HostQueue::<MixedTransferResult>::new(n),
        )
        .expect("returned owner/candidate arm at C+1");
    let joined = armed
        .control
        .join(
            armed.audio,
            Some(armed.control_port),
            Some(armed.audio_port),
        )
        .expect("boundary arm joins");
    assert_eq!(joined.transfer().custody(), MixedTransferCustody::Complete);

    // Arm's concrete minimum is2G+1=9, independently of the selected C.
    let eight = with_holds_and_depth(profile, EventCount::measured(4), EventCount::measured(8));
    let nine = with_holds_and_depth(profile, EventCount::measured(4), EventCount::measured(9));
    let (owner, candidate) = saved_prepared_with_profile(at(0), eight);
    let refusal = owner
        .arm_bridge_rehearsal(
            candidate,
            &eight,
            sources,
            fence(&setup),
            HostQueue::<MixedTransferCommand>::new(10),
            HostQueue::<MixedTransferResult>::new(10),
        )
        .expect_err("C8 cannot serve G4 with bend credit");
    assert!(matches!(refusal.reason(), MixedTransferArmError::Owner(_)));
    assert_eq!(
        refusal.reason().to_string(),
        "mixed ingress depth 8 cannot serve 4 live holds; need 9"
    );
    let (owner, candidate, commands, results) = refusal.into_parts();
    let armed = owner
        .arm_bridge_rehearsal(candidate, &nine, sources, fence(&setup), commands, results)
        .expect("same returned parts arm at9");
    let joined = armed
        .control
        .join(
            armed.audio,
            Some(armed.control_port),
            Some(armed.audio_port),
        )
        .expect("minimum ingress arm joins");
    assert_eq!(joined.transfer().custody(), MixedTransferCustody::Complete);

    let (owner, candidate) = saved_prepared_with_profile(at(0), profile);
    let mut one = fence(&setup);
    one.stage_cells = MixedTransferPlaces::new(1);
    let refusal = owner
        .arm_bridge_rehearsal(
            candidate,
            &profile,
            sources,
            one,
            HostQueue::<MixedTransferCommand>::new(n),
            HostQueue::<MixedTransferResult>::new(n),
        )
        .expect_err("stage needs onset plus protected key-up");
    assert_eq!(
        refusal.reason(),
        MixedTransferArmError::StageCells {
            cells: MixedTransferPlaces::new(1),
            maximum: MixedTransferPlaces::new(256)
        }
    );
    let (owner, candidate, commands, results) = refusal.into_parts();
    one.stage_cells = MixedTransferPlaces::new(2);
    let armed = owner
        .arm_bridge_rehearsal(candidate, &profile, sources, one, commands, results)
        .expect("same returned parts arm at stage2");
    let joined = armed
        .control
        .join(
            armed.audio,
            Some(armed.control_port),
            Some(armed.audio_port),
        )
        .expect("stage boundary arm joins");
    assert_eq!(joined.transfer().custody(), MixedTransferCustody::Complete);
    release_share_floor(profile, sources, &setup, n);
    raw.halt.request_stop();
    let mut audio = raw.audio.take().expect("raw audio");
    audio.synchronize_halt().expect("raw stop");
    let (_, error) = reunite_raw(audio, raw.control, raw.generations);
    assert!(error.is_none());
}

fn stage_reservation(profile: HostProfile, make_setup: &impl Fn() -> Setup) {
    let mut setup = make_setup();
    setup.cells = 2;
    let (prepared, candidate) = saved_prepared_with_profile(at(0), profile);
    let mut rig = Rig::with_prepared(setup, prepared, candidate, profile);
    rig.prime();
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(101, note_on(62)),
            message(200, note_off(60)),
            message(201, note_off(62)),
            frontier(512),
        ],
    );
    rig.send(SECOND, frontier(512));
    rig.stage_all();
    let refused = rig.packet_of(FIRST, 101);
    let refused_up = rig.packet_of(FIRST, 201);
    assert_eq!(
        rig.settled_of(refused),
        [Settled::Stage(Ok(MixedStaged::Split(
            MixedSplit::StageFull
        )))]
    );
    assert_eq!(
        rig.settled_of(refused_up),
        [Settled::Stage(Ok(MixedStaged::Split(
            MixedSplit::AfterRefusedOnset {
                onset: refused,
                reason: MixedOnsetRefusal::StageFull
            }
        )))]
    );
    for _ in 0..8 {
        rig.step();
    }
    let onset = rig.packet_of(FIRST, 100);
    let up = rig.packet_of(FIRST, 200);
    let identity = identity_of(rig.report_of(onset).expect("accepted onset"));
    assert!(
        matches!(rig.report_of(up).expect("reserved exact key-up").outcome(),MixedBridgeOutcome::KeyUp {onset:found,result:Ok(MixedKeyUpRetired::Released(ended))} if found==onset && ended==identity)
    );
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.teardown.open().is_empty());
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
}

fn with_share(
    base: HostProfile,
    class: ProducerClass,
    value: EventCount,
) -> Result<HostProfile, crate::frozen_capacity::FrozenError> {
    let old = base.limits().events();
    let shares = old.shares();
    let mut values = [
        shares.compiled_event_share(),
        shares.authored_runtime_event_share(),
        shares.live_event_share(),
        shares.session_event_share(),
        shares.internal_event_share(),
        shares.release_event_share(),
    ];
    values[class.index()] = value;
    let shares = ProducerShares::new(
        values[0],
        values[1],
        values[2],
        values[3],
        values[4],
        values[5],
        shares.release_hold_capacity(),
    )?;
    crate::frozen_capacity::replace_events(
        base,
        shares,
        old.queues().performance_ingress_capacity(),
        old.max_scheduled_events_in_flight(),
    )
}

fn executable_share_floors(profile: HostProfile) {
    // AuthoredRuntime/Internal have no executable producer in this host. Their
    // frozen None observations and retained entitlements are checked separately;
    // inventing an N/N-1 emission control would misrepresent their availability.
    for (class, n, field) in [
        (ProducerClass::Session, 29, ResourceField::SessionEventShare),
        (
            ProducerClass::Compiled,
            2,
            ResourceField::CompiledEventShare,
        ),
    ] {
        for value in [n, n - 1] {
            let boundary = with_share(profile, class, EventCount::measured(value))
                .expect("constructible compile boundary");
            let result = compile(
                &instrument_inserts::ir_with_layout(
                    Some(instrument_inserts::live()),
                    profile.capabilities().channel_layout(),
                ),
                &RenderConfig::new(boundary),
            )
            .into_plan();
            if value == n {
                let _plan = result.expect("exact executable share request");
            } else {
                assert!(
                    matches!(result,Err(CompileError::LimitExceeded {field:found,requested:ResourceAmount::Events(requested),available:ResourceAmount::Events(available),..})
                    if found==field && requested==EventCount::measured(n) && available==EventCount::measured(n-1)),
                    "exact {class} admission request"
                );
            }
        }
    }
    // The unchanged saved graph declares two compiled events/quantum, so its
    // share1 case stops at compile admission before a stream can be constructed.
    let c = profile
        .limits()
        .events()
        .queues()
        .performance_ingress_capacity();
    let _at = with_share(profile, ProducerClass::Live, c).expect("Live share exactly covers C");
    let below = EventCount::measured(c.get() - 1);
    assert!(
        matches!(with_share(profile,ProducerClass::Live,below),Err(crate::frozen_capacity::FrozenError::Profile(ProfileError::LiveShareBelowIngressCapacity {share,capacity})) if share==below && capacity==c)
    );
}

fn release_share_floor(
    profile: HostProfile,
    sources: MixedTransferSources,
    setup: &Setup,
    queue_depth: usize,
) {
    let events = profile.limits().events();
    let c = events.queues().performance_ingress_capacity();
    let global = events.shares().release_hold_capacity();
    let cut = EventCount::measured(c.get().checked_add(1).expect("S2 compiled Panic group"));
    let n = global.max(cut);
    let below = EventCount::measured(n.get() - 1);
    let exact =
        with_share(profile, ProducerClass::Release, n).expect("exact release floor profile");
    let (owner, candidate) = saved_prepared_with_profile(at(0), exact);
    let armed = owner
        .arm_bridge_rehearsal(
            candidate,
            &exact,
            sources,
            fence(setup),
            HostQueue::<MixedTransferCommand>::new(queue_depth),
            HostQueue::<MixedTransferResult>::new(queue_depth),
        )
        .expect("release floor arms");
    let joined = armed
        .control
        .join(
            armed.audio,
            Some(armed.control_port),
            Some(armed.audio_port),
        )
        .expect("exact release floor joins");
    assert_eq!(joined.transfer().custody(), MixedTransferCustody::Complete);
    if global >= cut {
        assert!(
            matches!(with_share(profile,ProducerClass::Release,below),Err(crate::frozen_capacity::FrozenError::Profile(ProfileError::ReleaseShareBelowHoldCapacity {share,capacity})) if share==below && capacity==global)
        );
    } else {
        let under =
            with_share(profile, ProducerClass::Release, below).expect("hold floor still fits");
        let (owner, candidate) = saved_prepared_with_profile(at(0), under);
        let refusal = owner
            .arm_bridge_rehearsal(
                candidate,
                &under,
                sources,
                fence(setup),
                HostQueue::<MixedTransferCommand>::new(queue_depth),
                HostQueue::<MixedTransferResult>::new(queue_depth),
            )
            .expect_err("full live backlog plus compiled cut needs C+1");
        assert!(matches!(refusal.reason(), MixedTransferArmError::Owner(_)));
        assert_eq!(
            refusal.reason().to_string(),
            format!(
                "release share {} cannot cover ingress depth {} plus a compiled Panic cut of 2 occurrences; need {}",
                below.get(),
                c.get(),
                n.get()
            )
        );
        let reason = refusal.reason();
        let (owner, candidate, commands, results) = refusal.into_parts();
        // Producer shares belong to the compiled partition. A new release share
        // needs the independently recompiled positive plan above; these returned
        // parts must instead retain the same exact refusal under their own profile.
        let again = owner
            .arm_bridge_rehearsal(candidate, &under, sources, fence(setup), commands, results)
            .expect_err("same returned parts preserve the original underfloor reason");
        assert_eq!(again.reason(), reason);
        let (_owner, _candidate, commands, results) = again.into_parts();
        assert_eq!(commands.ring.occupied_len(), 0);
        assert_eq!(results.ring.occupied_len(), 0);
    }
}

#[test]
fn release_backlog_plus_compiled_cut_has_its_own_named_arm_floor() {
    let base = crate::capacity_selection_controls::capacity_selection::candidate_profile(
        synth_engine_v2::quantities::SampleRate::new(48_000.0).expect("rate"),
        FrameCount::new(64),
        ChannelLayout::Stereo,
    )
    .expect("candidate");
    // G4's exact ingress minimum9 makes the compiled-cut floor10 dominate the
    // global hold floor4. This exercises the arm refusal even when P_ref's larger
    // global hold capacity makes its constructor reject first.
    let profile = with_holds_and_depth(base, EventCount::measured(4), EventCount::measured(9));
    transfer_and_performance(profile, &Setup::default);
}
