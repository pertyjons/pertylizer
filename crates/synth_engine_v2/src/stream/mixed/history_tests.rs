//! Bound, private mixed history: scoped batches carry compiled-only spans.

use super::*;
use crate::compile::{RenderConfig, compile};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain, parameters,
};
use crate::plan::{CompiledPlan, ControlRate, ParameterSlot};
use crate::profile::HostProfile;
use crate::quantities::{
    Amplitude, Cents, ChannelLayout, EventCount, Frequency, KeyIdentity, NormalizedLevel,
    NoteVelocity, SampleRate, Seconds,
};
use crate::render::{AudioBlockMut, NoteEdge, Renderer, TimedEvents};
use crate::sample::{
    PlayDirection, PlayMode, PlaybackRegion, PreparedSample, SampleFrame, SampleMap, SampleMapRef,
    SampleRef, SampleZone,
};
use crate::schedule::PlanEvent;
use crate::time::{FrameCount, TimeSource as TestOrigin};

const SOURCE: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);
const CONTROLLER: NodeId = NodeId::new(5);
const GLOBAL: NodeId = NodeId::new(6);
const NOTE_SOURCE: NodeId = NodeId::new(7);

impl MixedOneShotAudio {
    /// One test-only Live event through the owned arbiter, prepared before rendering.
    fn arm_test_live_on(
        &mut self,
        at: SampleTime,
        key: KeyIdentity,
        velocity: NoteVelocity,
    ) -> Result<NoteIdentity, MixedOneShotTestLiveError> {
        use MixedOneShotTestLiveError as Refused;
        #[cfg(feature = "simulated-ingress")]
        if self.test_ingress.adopted_by().is_some() {
            return Err(Refused::Timing);
        }
        if self.render_started
            || self.test_live.is_some()
            || at < self.audio.renderer.clock()
            || at >= self.timing.effective()
        {
            return Err(Refused::Timing);
        }
        if self.test_live_share < EventCount::measured(1) {
            return Err(Refused::Share);
        }
        let identity = self.audio.minter.mint_keyed(self.audio.note, key)?;
        self.test_live = Some(TimedEvent::new(
            crate::ingress::PerformanceIngress::envelope_for(self.audio.renderer.epoch(), at),
            EventPayload::Note {
                identity,
                edge: NoteEdge::On {
                    slot: self.audio.note,
                    key,
                    velocity,
                },
            },
        ));
        Ok(identity)
    }
}

fn key(raw: u8) -> KeyIdentity {
    KeyIdentity::new(raw).expect("valid key")
}

fn bound(compiled_first: bool) -> MixedJoinedPrepared {
    bound_with_events(compiled_first, |note| {
        vec![
            PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(10),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(72),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(20),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(72),
                },
            ),
            PlanEvent::new(
                PlanPosition::new(30),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(60),
                },
            ),
        ]
    })
}

fn bound_with_compiled_note_held_at_boundary(compiled_first: bool) -> MixedJoinedPrepared {
    bound_with_events(compiled_first, |note| {
        vec![
            PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(100),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(72),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(150),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(72),
                },
            ),
            PlanEvent::new(
                PlanPosition::new(200),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(60),
                },
            ),
        ]
    })
}

fn bound_with_events(
    compiled_first: bool,
    events: impl FnOnce(NoteSlot) -> Vec<PlanEvent>,
) -> MixedJoinedPrepared {
    let plan = mixed_plan(compiled_first);
    let note = plan.resolve_note(ENVELOPE).expect("playable envelope");
    let events = events(note);
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("admitted stream");
    let binding = MixedTargetAdmission::admit(plan, stream, note).expect("mixed target binding");
    MixedJoinedStream::open(
        binding,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("joined owner")
    .prepare_initial()
    .expect("stamped initial schedule")
}

pub(super) fn mixed_profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("rate"),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .expect("profile")
}

fn profile_with_session_share(session: EventCount) -> HostProfile {
    use crate::profile::{EventLimits, ProducerShares, RenderLimits};

    let original = mixed_profile();
    let limits = original.limits();
    let events = limits.events();
    let shares = events.shares();
    let shares = ProducerShares::new(
        shares.compiled_event_share(),
        shares.authored_runtime_event_share(),
        shares.live_event_share(),
        session,
        shares.internal_event_share(),
        shares.release_event_share(),
        shares.release_hold_capacity(),
    )
    .expect("valid shares");
    let events = EventLimits::new(
        events.max_events_per_quantum(),
        events.max_note_expansion_per_tick(),
        events.max_scheduled_events_in_flight(),
        events.forward_event_horizon(),
        events.queues(),
        shares,
    )
    .expect("valid event limits");
    let limits = RenderLimits::new(
        limits.stream(),
        limits.graph(),
        limits.voices(),
        events,
        limits.observation(),
        limits.mixing(),
        limits.memory(),
        limits.script(),
        limits.recording(),
        limits.cost(),
    )
    .expect("valid limits");
    HostProfile::new(original.capabilities(), limits).expect("valid profile")
}

#[cfg(feature = "simulated-ingress")]
fn profile_with_release_limits(
    release_share: EventCount,
    hold_capacity: EventCount,
) -> HostProfile {
    use crate::profile::{EventLimits, ProducerShares, RenderLimits};

    let original = mixed_profile();
    let limits = original.limits();
    let events = limits.events();
    let shares = events.shares();
    let shares = ProducerShares::new(
        shares.compiled_event_share(),
        shares.authored_runtime_event_share(),
        shares.live_event_share(),
        shares.session_event_share(),
        shares.internal_event_share(),
        release_share,
        hold_capacity,
    )
    .expect("valid shares");
    let events = EventLimits::new(
        events.max_events_per_quantum(),
        events.max_note_expansion_per_tick(),
        events.max_scheduled_events_in_flight(),
        events.forward_event_horizon(),
        events.queues(),
        shares,
    )
    .expect("valid event limits");
    let limits = RenderLimits::new(
        limits.stream(),
        limits.graph(),
        limits.voices(),
        events,
        limits.observation(),
        limits.mixing(),
        limits.memory(),
        limits.script(),
        limits.recording(),
        limits.cost(),
    )
    .expect("valid limits");
    HostProfile::new(original.capabilities(), limits).expect("valid profile")
}

pub(super) fn one_shot_with_boundary_on(
    compiled_first: bool,
) -> (MixedJoinedPrepared, MixedStampedCandidate) {
    one_shot_with_boundary_on_at(compiled_first, SampleTime::new(64))
}

fn one_shot_with_boundary_on_at(
    compiled_first: bool,
    requested: SampleTime,
) -> (MixedJoinedPrepared, MixedStampedCandidate) {
    let prepared = bound_with_events(compiled_first, |note| {
        vec![PlanEvent::new(
            PlanPosition::new(10),
            CompiledPayload::NoteOn {
                slot: note,
                key: key(60),
                velocity: NoteVelocity::FULL,
            },
        )]
    });
    let history = prepared
        .prepare_history(requested, PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    (prepared, candidate)
}

fn one_shot_at_compiled_share(
    compiled_first: bool,
) -> (MixedJoinedPrepared, MixedStampedCandidate) {
    let share = mixed_profile()
        .limits()
        .events()
        .shares()
        .compiled_event_share();
    assert_eq!(share.get() % 2, 0);
    let prepared = bound_with_events(compiled_first, |note| {
        let mut events = Vec::new();
        for frame in 0..share.get() / 2 {
            let at = PlanPosition::new(u64::from(frame));
            events.push(PlanEvent::new(
                at,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            ));
            events.push(PlanEvent::new(
                at,
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(60),
                },
            ));
        }
        events
    });
    let history = prepared
        .prepare_history(SampleTime::new(32), PlanPosition::ZERO)
        .expect("empty prefix");
    let suffix = prepared.prepare_suffix(history).expect("full suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("valid stamp");
    (prepared, candidate)
}

fn mixed_plan(compiled_first: bool) -> CompiledPlan {
    let compiled = NoteProducerDeclaration {
        compiled: true,
        simultaneous_notes: HeldNoteCount::measured(2),
        simultaneous_holds: EventCount::NONE,
    };
    let live = NoteProducerDeclaration {
        compiled: false,
        simultaneous_notes: HeldNoteCount::measured(2),
        simultaneous_holds: EventCount::measured(2),
    };
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite frequency"),
                amplitude: Amplitude::new(0.8).expect("finite amplitude"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::ZERO,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(
            CONTROLLER,
            IrNodeKind::Controller {
                kind: crate::controller::ControllerKind::ModWheel,
            },
            ExecutionScope::Voice,
        )
        .node(
            NOTE_SOURCE,
            IrNodeKind::NoteSource {
                kind: crate::controller::NoteSource::Velocity,
            },
            ExecutionScope::Voice,
        )
        .node(
            GLOBAL,
            IrNodeKind::Sine {
                frequency: Frequency::new(110.0).expect("finite frequency"),
                amplitude: Amplitude::new(0.1).expect("finite amplitude"),
            },
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(
            ExecutionScope::Voice,
            crate::tuning::PreparedTuning::equal_temperament().expect("tuning"),
        )
        .declaring(PlanDeclarations {
            note_producers: if compiled_first {
                vec![compiled, live]
            } else {
                vec![live, compiled]
            },
            held_notes: HeldNoteCount::measured(4),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("mixed graph");
    compile(&ir, &RenderConfig::new(mixed_profile()))
        .into_plan()
        .expect("plan")
}

fn restore(candidate: &MixedHistoryCandidate, slot: ParameterSlot) -> ScopedParameterRestore {
    candidate
        .restoration
        .iter()
        .find_map(|event| match event.payload() {
            EventPayload::ScopedRestore(restore) if restore.slot() == slot => Some(restore),
            _ => None,
        })
        .expect("one scoped event for the group")
}

fn render_quantum(renderer: &mut PreparedRenderer, events: &[TimedEvent]) -> Vec<f32> {
    let mut samples = vec![0.0_f32; crate::time::QUANTUM_FRAMES as usize];
    let frames = samples.len();
    let output = AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("mono block");
    renderer
        .render(output, TimedEvents::new(events))
        .expect("admitted quantum");
    samples
}

fn one_shot_rehearsal(
    partition: &[usize],
    compiled_first: bool,
) -> (Vec<f32>, MixedOneShotRenderReport) {
    let prepared = bound_with_compiled_note_held_at_boundary(compiled_first);
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(100))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let live = audio
        .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("one admitted test live onset");
    assert_eq!(audio.audio.minter.resolve(live), Resolution::Live);
    let mut samples = vec![0.0_f32; partition.iter().sum()];
    let mut offset = 0;
    for &frames in partition {
        let end = offset + frames;
        let output = AudioBlockMut::new(&mut samples[offset..end], frames, ChannelLayout::Mono)
            .expect("mono block");
        let allocator_events = crate::render_allocation::count_allocs(|| {
            audio.render_private(output).expect("private render");
        });
        assert_eq!(allocator_events, 0, "callback allocated or deallocated");
        offset = end;
    }
    assert_eq!(audio.audio.minter.resolve(live), Resolution::Live);
    (samples, audio.report())
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_channel_reports_conflict_with_direct_test_live_ownership() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (mut control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let _identity = audio
        .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("direct test live owner");
    let request = MixedIngressRequest::Onset {
        origin: MixedIngressOriginId(1),
        at: SampleTime::ZERO,
        key: key(49),
        velocity: NoteVelocity::FULL,
    };
    let id = control.submit_ingress(request).expect("queued request");
    audio.service_test_ingress_queue();
    assert_eq!(
        control.collect_ingress_result(),
        Some(MixedIngressResult {
            id,
            request,
            outcome: MixedIngressOutcome::Onset(Err(IngressRefused::MixedTestConflict)),
        })
    );
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_ingress_reuses_only_after_ordered_release() {
    use crate::ingress::IngressRefused;

    for compiled_first in [true, false] {
        let prepared = bound_with_compiled_note_held_at_boundary(compiled_first);
        let EventPayload::Note {
            identity: compiled,
            edge: NoteEdge::On { .. },
        } = prepared.events[0].payload()
        else {
            panic!("compiled onset");
        };
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(256))
            .expect("history");
        let suffix = prepared.prepare_suffix(history).expect("suffix");
        let candidate = prepared.stamp_suffix(suffix).expect("stamp");
        let (_, mut audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("private arm");
        let first = audio
            .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("first onset");
        assert_eq!(
            audio.test_ingress.holds_outstanding(),
            EventCount::measured(1)
        );
        assert_eq!(
            audio.offer_test_note_off(SampleTime::new(8), compiled),
            Err(IngressRefused::OrphanRelease { identity: compiled })
        );
        audio
            .offer_test_note_off(SampleTime::new(16), first)
            .expect("reserved release");
        assert_eq!(audio.test_ingress.holds_outstanding(), EventCount::NONE);
        assert_eq!(
            audio.offer_test_note_off(SampleTime::new(16), first),
            Err(IngressRefused::OrphanRelease { identity: first })
        );
        assert_eq!(
            audio.offer_test_note_on(SampleTime::new(15), key(49), NoteVelocity::FULL),
            Err(IngressRefused::NonMonotoneStamp {
                time: SampleTime::new(15),
                last: SampleTime::new(16),
            })
        );
        let second = audio
            .offer_test_note_on(SampleTime::new(16), key(49), NoteVelocity::FULL)
            .expect("same-time reused onset");
        assert_eq!(first.index(), second.index());
        assert_ne!(first, second);
        let mut samples = [0.0_f32; 256];
        audio
            .render_private(
                AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("block"),
            )
            .expect("ordered mixed render");
        assert!(audio.report().adopted);
        assert_eq!(audio.test_ingress.len(), 0);
        let sounding = audio
            .audio
            .renderer
            .snapshot_mixed_sounding()
            .expect("registry");
        assert_eq!(
            sounding
                .live()
                .iter()
                .map(|note| note.identity)
                .collect::<Vec<_>>(),
            vec![second]
        );
    }
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_ingress_arm_refuses_unbounded_release_backlog() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let epoch = prepared.epoch();
    let count = prepared.event_count();
    let refused = prepared
        .arm_one_shot(
            candidate,
            &profile_with_release_limits(EventCount::measured(16), EventCount::measured(16)),
        )
        .expect_err("release share below registered queue depth");
    assert!(matches!(
        refused.reason,
        MixedOneShotArmError::Ingress(
            crate::ingress::IngressPrepareError::ReleaseBacklogShare {
                share,
                capacity,
            }
        ) if share == EventCount::measured(16) && capacity == EventCount::measured(32)
    ));
    assert_eq!(refused.owner.epoch(), epoch);
    assert_eq!(refused.owner.event_count(), count);
    assert_eq!(refused.candidate.suffix.history.epoch(), epoch);
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_ingress_arm_refuses_profile_below_plan_holds() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let epoch = prepared.epoch();
    let refused = prepared
        .arm_one_shot(
            candidate,
            &profile_with_release_limits(EventCount::measured(40), EventCount::measured(1)),
        )
        .expect_err("bound plan has two live holds");
    assert_eq!(
        refused.reason,
        MixedOneShotArmError::Admission(MixedOneShotAdmissionError::ProfileMismatch)
    );
    assert_eq!(refused.owner.epoch(), epoch);
    assert_eq!(refused.candidate.suffix.history.epoch(), epoch);
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_ingress_counts_each_resource_and_preserves_release_slot() {
    use crate::ingress::{ExhaustedResource, IngressRefused};

    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let first = audio
        .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("first hold");
    let second = audio
        .offer_test_note_on(SampleTime::ZERO, key(49), NoteVelocity::FULL)
        .expect("second hold");
    assert_eq!(
        audio.test_ingress.adopted_by(),
        Some(audio.audio.renderer.epoch())
    );
    assert_eq!(
        audio.offer_test_note_on(SampleTime::ZERO, key(50), NoteVelocity::FULL),
        Err(IngressRefused::Dropped {
            resource: ExhaustedResource::Hold,
        })
    );
    assert_eq!(audio.test_ingress.counters().dropped_hold(), 1);
    audio
        .offer_test_note_off(SampleTime::ZERO, first)
        .expect("first protected release");
    audio
        .offer_test_note_off(SampleTime::ZERO, second)
        .expect("second protected release");

    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    for _ in 0..15 {
        let identity = audio
            .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("queued onset");
        audio
            .offer_test_note_off(SampleTime::ZERO, identity)
            .expect("queued release");
    }
    let protected = audio
        .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("last onset reserves release slot");
    assert_eq!(
        audio.offer_test_note_on(SampleTime::ZERO, key(49), NoteVelocity::FULL),
        Err(IngressRefused::Dropped {
            resource: ExhaustedResource::Slot,
        })
    );
    audio
        .offer_test_note_off(SampleTime::ZERO, protected)
        .expect("release fills last slot despite queue pressure");
    assert_eq!(audio.test_ingress.len(), 32);
    assert_eq!(audio.test_ingress.counters().dropped_slot(), 1);

    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    audio.audio.minter.retire_first_generation_for_test();
    for raw in [48, 49] {
        let identity = audio
            .offer_test_note_on(SampleTime::ZERO, key(raw), NoteVelocity::FULL)
            .expect("first generation in range");
        audio
            .offer_test_note_off(SampleTime::ZERO, identity)
            .expect("release retires index");
    }
    assert_eq!(
        audio.offer_test_note_on(SampleTime::ZERO, key(50), NoteVelocity::FULL),
        Err(IngressRefused::Dropped {
            resource: ExhaustedResource::Identity,
        })
    );
    assert_eq!(audio.test_ingress.holds_outstanding(), EventCount::NONE);
    assert_eq!(audio.test_ingress.counters().dropped_identity(), 1);
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_ingress_late_full_queue_fits_adoption_partition() {
    use crate::publish::ProducerClass;

    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let mut old = [0.0_f32; 128];
    audio
        .render_private(AudioBlockMut::new(&mut old, 128, ChannelLayout::Mono).expect("old"))
        .expect("old quantum");
    assert!(!audio.report().adopted);
    for _ in 0..16 {
        let identity = audio
            .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("late onset");
        audio
            .offer_test_note_off(SampleTime::ZERO, identity)
            .expect("late protected release");
    }
    assert_eq!(audio.test_ingress.len(), 32);
    let mut boundary = [0.0_f32; 1];
    audio
        .render_private(
            AudioBlockMut::new(&mut boundary, 1, ChannelLayout::Mono).expect("boundary"),
        )
        .expect("adoption with full late backlog");
    assert!(audio.report().adopted);
    assert_eq!(audio.test_ingress.len(), 0);
    assert_eq!(
        audio.arbiter.high_water(ProducerClass::Live),
        EventCount::measured(16)
    );
    assert_eq!(
        audio.arbiter.high_water(ProducerClass::Release),
        EventCount::measured(16)
    );
    let limits = mixed_profile().limits().events();
    assert!(audio.arbiter.high_water(ProducerClass::Live) <= limits.shares().live_event_share());
    assert!(
        audio.arbiter.high_water(ProducerClass::Release) <= limits.shares().release_event_share()
    );
    assert!(audio.arbiter.high_water_external_total() <= limits.max_events_per_quantum());
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_ingress_teardown_classifies_pending_and_faulted_release() {
    for fault_after_charge in [false, true] {
        let (prepared, candidate) = one_shot_with_boundary_on(true);
        let (control, mut audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("private arm");
        let live = audio
            .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("onset");
        audio
            .offer_test_note_off(SampleTime::new(8), live)
            .expect("release spends hold");
        assert_eq!(audio.test_ingress.holds_outstanding(), EventCount::NONE);
        if fault_after_charge {
            audio.test_fail_after_ingress_at = Some(SampleTime::ZERO);
            let mut samples = [1.0_f32; 128];
            assert_eq!(
                audio.render_private(
                    AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("block")
                ),
                Err(MixedOneShotRenderError::InjectedAfterIngress)
            );
            assert!(samples.iter().all(|sample| *sample == 0.0));
            let queued = audio.test_ingress.len();
            let counters = audio.test_ingress.counters();
            assert_eq!(
                audio.offer_test_note_on(SampleTime::new(16), key(49), NoteVelocity::FULL),
                Err(crate::ingress::IngressRefused::TerminalOwner)
            );
            assert_eq!(
                audio.offer_test_note_off(SampleTime::new(16), live),
                Err(crate::ingress::IngressRefused::TerminalOwner)
            );
            assert_eq!(audio.test_ingress.len(), queued);
            assert_eq!(audio.test_ingress.counters(), counters);
        }
        let MixedCollection::Ended(ended) = control.collect(audio).expect("teardown") else {
            panic!("owner must end");
        };
        assert_eq!(
            ended.end,
            if fault_after_charge {
                MixedCollectionEnd::Faulted
            } else {
                MixedCollectionEnd::Pending
            }
        );
        let pending = if fault_after_charge {
            assert!(ended.ingress.queued.is_empty());
            &ended.ingress.charged_in_faulted_callback
        } else {
            assert!(ended.ingress.charged_in_faulted_callback.is_empty());
            &ended.ingress.queued
        };
        assert_eq!(pending.len(), 2);
        assert_eq!(pending.iter().filter(|entry| entry.1).count(), 1);
        assert_eq!(ended.ingress.holds_outstanding, EventCount::NONE);
        assert!(ended.ingress.minted_live.is_empty());
        assert_eq!(ended.ingress.counters.dropped(), 0);
    }
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_ingress_fault_retains_earlier_quantum_charges() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let first = audio
        .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("first onset");
    audio
        .offer_test_note_off(SampleTime::ZERO, first)
        .expect("first release");
    let second = audio
        .offer_test_note_on(SampleTime::new(64), key(49), NoteVelocity::FULL)
        .expect("second onset");
    audio
        .offer_test_note_off(SampleTime::new(64), second)
        .expect("second release");
    audio.test_fail_after_ingress_at = Some(SampleTime::new(64));
    let mut samples = [1.0_f32; 192];
    assert_eq!(
        audio.render_private(
            AudioBlockMut::new(&mut samples, 192, ChannelLayout::Mono).expect("block")
        ),
        Err(MixedOneShotRenderError::InjectedAfterIngress)
    );
    assert!(samples.iter().all(|sample| *sample == 0.0));
    assert_eq!(audio.report().completed_quanta, QuantumCount::measured(1));
    let MixedCollection::Ended(ended) = control.collect(audio).expect("terminal teardown") else {
        panic!("faulted owner must end");
    };
    assert_eq!(ended.end, MixedCollectionEnd::Faulted);
    assert!(ended.ingress.queued.is_empty());
    assert_eq!(ended.ingress.charged_in_faulted_callback.len(), 4);
    assert_eq!(
        ended
            .ingress
            .charged_in_faulted_callback
            .iter()
            .map(|(event, redeems)| {
                let EventPayload::Note { identity, edge } = event.payload() else {
                    panic!("journal contains only note edges");
                };
                (
                    event.envelope().time(),
                    identity,
                    matches!(edge, NoteEdge::Off),
                    *redeems,
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (SampleTime::ZERO, first, false, false),
            (SampleTime::ZERO, first, true, true),
            (SampleTime::new(64), second, false, false),
            (SampleTime::new(64), second, true, true),
        ]
    );
    assert!(ended.sounding.live().is_empty());
    assert_eq!(ended.boundary_ended.len(), 1);
    assert!(ended.ingress.minted_live.is_empty());
    assert_eq!(ended.ingress.holds_outstanding, EventCount::NONE);
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_ingress_resumed_teardown_keeps_pending_release() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let live = audio
        .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("onset");
    let mut samples = [0.0_f32; 129];
    audio
        .render_private(AudioBlockMut::new(&mut samples, 129, ChannelLayout::Mono).expect("block"))
        .expect("adopted render");
    let MixedCollection::Resumed {
        control, mut audio, ..
    } = control.collect(audio).expect("resumed owner")
    else {
        panic!("adopted owner must resume");
    };
    audio
        .offer_test_note_off(SampleTime::new(129), live)
        .expect("pending release");
    let ended = control.teardown(*audio).expect("resumed teardown");
    assert_eq!(ended.end, MixedCollectionEnd::ResumedTeardown);
    assert_eq!(ended.ingress.queued.len(), 1);
    assert!(ended.ingress.queued[0].1);
    assert!(ended.ingress.charged_in_faulted_callback.is_empty());
    assert!(ended.ingress.minted_live.is_empty());
    assert_eq!(ended.ingress.holds_outstanding, EventCount::NONE);
    assert_eq!(ended.ingress.counters.orphan_releases(), 0);
}

#[cfg(feature = "simulated-ingress")]
fn mixed_ingress_audio(partition: &[usize]) -> (Vec<f32>, Vec<f32>) {
    let prepared = bound_with_compiled_note_held_at_boundary(true);
    let plan = std::sync::Arc::clone(&prepared.owner.control.plan);
    let instance_partition = std::sync::Arc::clone(&prepared.owner.control.partition);
    let epoch = prepared.epoch();
    let table = prepared.table_id();
    let anchor = prepared.owner.control.anchor;
    let history = prepared
        .prepare_history(SampleTime::new(128), PlanPosition::new(256))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let first = audio
        .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("before-boundary onset");
    audio
        .offer_test_note_off(SampleTime::new(80), first)
        .expect("before-boundary release");
    let second = audio
        .offer_test_note_on(SampleTime::new(144), key(49), NoteVelocity::FULL)
        .expect("after-boundary onset");
    audio
        .offer_test_note_off(SampleTime::new(208), second)
        .expect("after-boundary release");
    let events = [
        TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::ZERO, TestOrigin::Simulated),
            EventPayload::Note {
                identity: first,
                edge: NoteEdge::On {
                    slot: audio.audio.note,
                    key: key(48),
                    velocity: NoteVelocity::FULL,
                },
            },
        ),
        TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::new(80), TestOrigin::Simulated),
            EventPayload::Note {
                identity: first,
                edge: NoteEdge::Off,
            },
        ),
        TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::new(144), TestOrigin::Simulated),
            EventPayload::Note {
                identity: second,
                edge: NoteEdge::On {
                    slot: audio.audio.note,
                    key: key(49),
                    velocity: NoteVelocity::FULL,
                },
            },
        ),
        TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::new(208), TestOrigin::Simulated),
            EventPayload::Note {
                identity: second,
                edge: NoteEdge::Off,
            },
        ),
    ];
    let mut reference = PreparedRenderer::prepare(plan, anchor, epoch, table).expect("reference");
    assert!(reference.bind_mixed_partition(instance_partition));
    let mut mixed = vec![0.0_f32; partition.iter().sum()];
    let mut offset = 0;
    for &frames in partition {
        audio
            .render_private(
                AudioBlockMut::new(
                    &mut mixed[offset..offset + frames],
                    frames,
                    ChannelLayout::Mono,
                )
                .expect("mixed block"),
            )
            .expect("mixed callback");
        offset += frames;
    }
    let mut live_only = vec![0.0_f32; mixed.len()];
    reference
        .render(
            AudioBlockMut::new(&mut live_only, mixed.len(), ChannelLayout::Mono)
                .expect("reference block"),
            TimedEvents::new(&events),
        )
        .expect("reference callback");
    assert!(audio.report().adopted);
    assert!(
        audio
            .audio
            .renderer
            .snapshot_mixed_sounding()
            .expect("mixed rows")
            .live()
            .is_empty()
    );
    assert!(
        reference
            .snapshot_mixed_sounding()
            .expect("reference rows")
            .live()
            .is_empty()
    );
    (mixed, live_only)
}

#[cfg(feature = "simulated-ingress")]
fn mixed_ingress_compiled_baseline(partition: &[usize]) -> Vec<f32> {
    let prepared = bound_with_compiled_note_held_at_boundary(true);
    let history = prepared
        .prepare_history(SampleTime::new(128), PlanPosition::new(256))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let mut compiled = vec![0.0_f32; partition.iter().sum()];
    let mut offset = 0;
    for &frames in partition {
        audio
            .render_private(
                AudioBlockMut::new(
                    &mut compiled[offset..offset + frames],
                    frames,
                    ChannelLayout::Mono,
                )
                .expect("compiled block"),
            )
            .expect("compiled callback");
        offset += frames;
    }
    compiled
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_ingress_matches_live_reference_across_adoption() {
    let (mixed, live_only) = mixed_ingress_audio(&[320]);
    let compiled = mixed_ingress_compiled_baseline(&[320]);
    assert!(live_only[64..128].iter().any(|sample| *sample != 0.0));
    assert!(live_only[208..272].iter().any(|sample| *sample != 0.0));
    for (index, ((mixed, live), compiled)) in
        mixed.iter().zip(&live_only).zip(&compiled).enumerate()
    {
        assert!(
            (mixed - (live + compiled)).abs() < 0.00001,
            "frame {index}: mixed {mixed}, live {live}, compiled {compiled}"
        );
    }
    let (partitioned, partitioned_reference) = mixed_ingress_audio(&[64; 5]);
    assert_eq!(partitioned, mixed);
    assert_eq!(partitioned_reference, live_only);
    assert_eq!(mixed_ingress_compiled_baseline(&[64; 5]), compiled);
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn private_mixed_ingress_live_rows_match_reference_before_and_after_adoption() {
    let prepared = bound_with_compiled_note_held_at_boundary(true);
    let plan = std::sync::Arc::clone(&prepared.owner.control.plan);
    let partition = std::sync::Arc::clone(&prepared.owner.control.partition);
    let epoch = prepared.epoch();
    let table = prepared.table_id();
    let anchor = prepared.owner.control.anchor;
    let history = prepared
        .prepare_history(SampleTime::new(128), PlanPosition::new(256))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let first = audio
        .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("first onset");
    audio
        .offer_test_note_off(SampleTime::new(80), first)
        .expect("first release");
    let second = audio
        .offer_test_note_on(SampleTime::new(144), key(49), NoteVelocity::FULL)
        .expect("second onset");
    audio
        .offer_test_note_off(SampleTime::new(208), second)
        .expect("second release");
    let event = |at, identity, edge| {
        TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::new(at), TestOrigin::Simulated),
            EventPayload::Note { identity, edge },
        )
    };
    let events = [
        event(
            0,
            first,
            NoteEdge::On {
                slot: audio.audio.note,
                key: key(48),
                velocity: NoteVelocity::FULL,
            },
        ),
        event(80, first, NoteEdge::Off),
        event(
            144,
            second,
            NoteEdge::On {
                slot: audio.audio.note,
                key: key(49),
                velocity: NoteVelocity::FULL,
            },
        ),
        event(208, second, NoteEdge::Off),
    ];
    let mut reference = PreparedRenderer::prepare(plan, anchor, epoch, table).expect("reference");
    assert!(reference.bind_mixed_partition(partition));
    for (stage, frames) in [128_usize, 64, 64, 64].into_iter().enumerate() {
        let mut mixed_output = vec![0.0_f32; frames];
        let mut reference_output = vec![0.0_f32; frames];
        audio
            .render_private(
                AudioBlockMut::new(&mut mixed_output, frames, ChannelLayout::Mono)
                    .expect("mixed block"),
            )
            .expect("mixed callback");
        reference
            .render(
                AudioBlockMut::new(&mut reference_output, frames, ChannelLayout::Mono)
                    .expect("reference block"),
                TimedEvents::new(&events[stage..stage + 1]),
            )
            .expect("reference callback");
        let expected_clock = SampleTime::new([64_u64, 128, 192, 256][stage]);
        assert_eq!(audio.audio.renderer.clock(), expected_clock);
        assert_eq!(reference.clock(), expected_clock);
        assert_eq!(audio.report().adopted, stage >= 2);
        let mixed_rows = audio
            .audio
            .renderer
            .snapshot_mixed_sounding()
            .expect("mixed rows");
        let reference_rows = reference.snapshot_mixed_sounding().expect("reference rows");
        assert_eq!(mixed_rows.live(), reference_rows.live(), "stage {stage}");
        let expected = match stage {
            0 => Some(first),
            2 => Some(second),
            _ => None,
        };
        assert_eq!(
            mixed_rows.live().first().map(|note| note.identity),
            expected
        );
    }
}

#[test]
fn private_one_shot_audio_is_partition_invariant_with_live_held_across_boundary() {
    let one = [512];
    let sixty_four = [64; 8];
    let tiny = [8; 64];
    let irregular = [7, 63, 90, 1, 256, 95];
    for compiled_first in [true, false] {
        let (reference, report) = one_shot_rehearsal(&one, compiled_first);
        assert!(report.adopted);
        assert!(!report.faulted);
        assert!(report.release_charged);
        assert_eq!(report.released_compiled, HeldNoteCount::measured(1));
        assert!(report.restoration_charged.get() > 0);
        assert!(report.suffix_charged.get() > 0);
        assert_eq!(report.completed_quanta, QuantumCount::measured(7));
        assert!(reference.iter().any(|sample| *sample != 0.0));
        for partition in [&sixty_four[..], &tiny[..], &irregular[..]] {
            let (samples, candidate_report) = one_shot_rehearsal(partition, compiled_first);
            assert_eq!(samples, reference);
            assert_eq!(candidate_report, report);
        }
    }
}

#[test]
fn private_one_shot_audio_waits_for_a_new_quantum_before_adoption() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let mut samples = [0.0_f32; 128];
    let first = AudioBlockMut::new(&mut samples[..64], 64, ChannelLayout::Mono).expect("first");
    audio.render_private(first).expect("carry only");
    assert_eq!(audio.audio.renderer.clock(), SampleTime::ZERO);
    assert!(!audio.report().adopted);
    let second = AudioBlockMut::new(&mut samples[64..], 64, ChannelLayout::Mono).expect("second");
    audio.render_private(second).expect("old quantum");
    assert_eq!(audio.audio.renderer.clock(), SampleTime::new(64));
    assert!(
        !audio.report().adopted,
        "ending exactly at boundary stays pending"
    );
    let mut next = [0.0_f32; 1];
    let output = AudioBlockMut::new(&mut next, 1, ChannelLayout::Mono).expect("new quantum");
    audio.render_private(output).expect("boundary quantum");
    assert!(audio.report().adopted);
    assert_eq!(audio.report().completed_quanta, QuantumCount::measured(2));
}

#[test]
fn private_one_shot_audio_refuses_wrong_shape_without_changing_the_fixed_boundary() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let mut wrong = [0.25_f32; 2];
    let block = AudioBlockMut::new(&mut wrong, 1, ChannelLayout::Stereo).expect("stereo block");
    assert_eq!(
        audio.render_private(block),
        Err(MixedOneShotRenderError::OutputShape)
    );
    assert_eq!(wrong, [0.25; 2]);
    assert_eq!(audio.audio.renderer.clock(), SampleTime::ZERO);
    assert!(!audio.report().faulted);
    let mut correct = [0.0_f32; 129];
    let block = AudioBlockMut::new(&mut correct, 129, ChannelLayout::Mono).expect("mono block");
    audio.render_private(block).expect("same fixed boundary");
    assert!(audio.report().adopted);
    assert_eq!(audio.report().effective_anchor.time(), SampleTime::new(64));
}

#[test]
fn private_one_shot_terminal_capsule_preflight_still_allows_original_pair_teardown() {
    for corrupt in ["plan", "epoch", "table"] {
        let (prepared, candidate) = one_shot_with_boundary_on(true);
        let foreign = bound(false);
        let (control, mut audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("private arm");
        match corrupt {
            "plan" => audio.capsule.plan = foreign.owner.control.plan.id(),
            "epoch" => audio.capsule.epoch = foreign.epoch(),
            "table" => audio.capsule.table = foreign.table_id(),
            _ => panic!("known field"),
        }
        let mut samples = [1.0_f32; 1];
        let error = audio
            .render_private(
                AudioBlockMut::new(&mut samples, 1, ChannelLayout::Mono).expect("block"),
            )
            .expect_err("capsule mismatch");
        assert_eq!(error, MixedOneShotRenderError::Pairing, "{corrupt}");
        assert_eq!(samples, [0.0]);
        assert!(!audio.report().adopted);
        let MixedCollection::Ended(ended) = control.collect(audio).expect("original pair teardown")
        else {
            panic!("terminal owner must end");
        };
        assert_eq!(ended.end, MixedCollectionEnd::Faulted);
        assert_eq!(ended.report.fault, Some(error));
        assert!(ended.sounding.compiled().is_empty());
    }
}

#[test]
fn private_one_shot_already_faulted_renderer_refuses_before_publication() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let mut scratch = [1.0_f32; 1];
    let mut scratch_block =
        AudioBlockMut::new(&mut scratch, 1, ChannelLayout::Mono).expect("scratch");
    audio
        .audio
        .renderer
        .terminal_mixed_fault(&mut scratch_block);
    let mut samples = [1.0_f32; 1];
    let error = audio
        .render_private(AudioBlockMut::new(&mut samples, 1, ChannelLayout::Mono).expect("block"))
        .expect_err("renderer already faulted");
    assert_eq!(error, MixedOneShotRenderError::RendererFaulted);
    assert_eq!(samples, [0.0]);
    assert_eq!(audio.report().completed_quanta, QuantumCount::NONE);
    let MixedCollection::Ended(ended) = control.collect(audio).expect("terminal teardown") else {
        panic!("terminal owner must end");
    };
    assert_eq!(ended.end, MixedCollectionEnd::Faulted);
    assert_eq!(ended.report.fault, Some(error));
}

#[test]
fn private_one_shot_head_fault_silences_call_and_keeps_candidate_unadopted() {
    let (prepared, candidate) = one_shot_with_boundary_on_at(true, SampleTime::new(65));
    let (control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let mut before = [0.0_f32; 128];
    audio
        .render_private(AudioBlockMut::new(&mut before, 128, ChannelLayout::Mono).expect("old"))
        .expect("first old quantum");
    assert_eq!(audio.audio.renderer.clock(), SampleTime::new(64));
    assert_eq!(audio.timing.effective(), SampleTime::new(128));
    let stale = audio.events[0];
    audio.events.push(stale);
    let mut samples = [1.0_f32; 65];
    let error = audio
        .render_private(
            AudioBlockMut::new(&mut samples, 65, ChannelLayout::Mono).expect("crossing call"),
        )
        .expect_err("old event misses head clock");
    assert_eq!(
        error,
        MixedOneShotRenderError::MissedEvent {
            event: stale.envelope().time(),
            clock: SampleTime::new(64),
        }
    );
    assert!(samples.iter().all(|sample| *sample == 0.0));
    assert!(!audio.report().adopted);
    assert_eq!(audio.adoption_after_quanta, None);
    assert_eq!(audio.report().completed_quanta, QuantumCount::measured(1));
    let MixedCollection::Ended(ended) = control.collect(audio).expect("terminal teardown") else {
        panic!("head fault must end");
    };
    assert_eq!(ended.end, MixedCollectionEnd::Faulted);
    assert!(ended.boundary_ended.is_empty());
    assert_eq!(ended.sounding.compiled().len(), 1);
}

#[test]
fn private_one_shot_audio_boundary_fault_silences_complete_callback_and_stays_terminal() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    audio.audio.compiled_ended.clear();
    let mut samples = [1.0_f32; 256];
    let block = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("block");
    assert_eq!(
        audio.render_private(block),
        Err(MixedOneShotRenderError::Boundary(
            crate::render::MixedBoundaryReleaseError::EndedStorage
        ))
    );
    assert!(samples.iter().all(|sample| *sample == 0.0));
    assert_eq!(audio.report().completed_quanta, QuantumCount::measured(1));
    assert!(!audio.report().adopted);
    assert!(audio.report().faulted);
    assert_eq!(
        audio.report().fault,
        Some(MixedOneShotRenderError::Boundary(
            crate::render::MixedBoundaryReleaseError::EndedStorage
        ))
    );
    assert!(audio.capsule.retired_anchor.is_none());
    let mut later = [1.0_f32; 64];
    let block = AudioBlockMut::new(&mut later, 64, ChannelLayout::Mono).expect("later block");
    assert_eq!(
        audio.render_private(block),
        Err(MixedOneShotRenderError::Faulted)
    );
    assert!(later.iter().all(|sample| *sample == 0.0));
}

#[test]
fn private_one_shot_audio_leaves_old_boundary_event_in_retired_list() {
    let prepared = bound_with_events(true, |note| {
        vec![
            PlanEvent::new(
                PlanPosition::new(63),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(64),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(60),
                },
            ),
        ]
    });
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(100))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let mut samples = [0.0_f32; 192];
    let block = AudioBlockMut::new(&mut samples, 192, ChannelLayout::Mono).expect("block");
    audio.render_private(block).expect("cross boundary");
    assert_eq!(audio.capsule.retired_next, Some(1));
    assert_eq!(audio.capsule.events.len(), 2);
    assert_eq!(
        audio.capsule.events[1].envelope().time(),
        SampleTime::new(64)
    );
    assert_eq!(audio.audio.renderer.diagnostics().orphan_note_events(), 0);
}

#[test]
fn private_one_shot_audio_publication_fault_after_adoption_reports_reached_charges() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let restoration = candidate.restoration_count;
    let session = restoration
        .checked_add(EventCount::measured(1))
        .expect("one release");
    let profile = profile_with_session_share(session);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &profile)
        .expect("exact Session share");
    // Fault injection after admission: treat the first suffix event as another
    // Session restoration. No production path can mutate the sealed capsule.
    audio.capsule.restoration_count = session;
    let mut samples = [1.0_f32; 256];
    let block = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("block");
    let error = audio.render_private(block).expect_err("Session overrun");
    assert!(matches!(
        error,
        MixedOneShotRenderError::Publication(crate::publish::PublicationFault::ShareOverrun {
            class: crate::publish::ProducerClass::Session,
            ..
        })
    ));
    assert!(samples.iter().all(|sample| *sample == 0.0));
    let report = audio.report();
    assert!(report.adopted && report.faulted && report.release_charged);
    assert_eq!(report.fault, Some(error));
    assert_eq!(report.restoration_charged, restoration);
    assert_eq!(report.suffix_charged, EventCount::NONE);
    assert_eq!(report.completed_quanta, QuantumCount::measured(1));
    assert_eq!(
        report.retired_anchor,
        Some(StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO))
    );
}

#[test]
fn private_one_shot_audio_later_quantum_fault_silences_earlier_output() {
    let (prepared, candidate) = one_shot_with_boundary_on(false);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let share = mixed_profile()
        .limits()
        .events()
        .shares()
        .compiled_event_share();
    let original = *audio.capsule.events.last().expect("compiled suffix");
    let late = TimedEvent::new(
        EventEnvelope::new(
            original.envelope().epoch(),
            SampleTime::new(128),
            original.envelope().source(),
        ),
        original.payload(),
    );
    // Fault injection after admission, without a producer API that could offer it.
    audio
        .capsule
        .events
        .extend(std::iter::repeat_n(late, share.get() as usize + 1));
    let mut samples = [1.0_f32; 320];
    let block = AudioBlockMut::new(&mut samples, 320, ChannelLayout::Mono).expect("block");
    let error = audio
        .render_private(block)
        .expect_err("later Compiled overrun");
    assert!(matches!(
        error,
        MixedOneShotRenderError::Publication(crate::publish::PublicationFault::ShareOverrun {
            class: crate::publish::ProducerClass::Compiled,
            ..
        })
    ));
    assert!(samples.iter().all(|sample| *sample == 0.0));
    let report = audio.report();
    assert!(report.adopted && report.faulted);
    assert_eq!(report.fault, Some(error));
    assert_eq!(report.completed_quanta, QuantumCount::measured(2));
    assert_eq!(
        report.suffix_charged,
        share
            .checked_add(EventCount::measured(1))
            .expect("charge count")
    );
}

#[test]
fn private_one_shot_audio_displacement_fault_keeps_cause_and_counter() {
    let (prepared, candidate) = one_shot_with_boundary_on_at(true, SampleTime::new(32));
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let index = audio.capsule.events.len() - 1;
    let event = audio.capsule.events[index];
    // Fault injection after off-thread admission; the real candidate cannot overflow.
    audio.capsule.events[index] = TimedEvent::new(
        EventEnvelope::new(
            event.envelope().epoch(),
            SampleTime::new(u64::MAX),
            event.envelope().source(),
        ),
        event.payload(),
    );
    let mut samples = [1.0_f32; 256];
    let block = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("block");
    let error = audio.render_private(block).expect_err("shift overflow");
    assert_eq!(
        error,
        MixedOneShotRenderError::Displacement(MixedEffectiveTimeError::EventTimeUnrepresentable {
            event_index: index,
            time: SampleTime::new(u64::MAX),
            shift: FrameCount::new(32),
        })
    );
    assert_eq!(audio.report().fault, Some(error));
    assert_eq!(audio.audio.renderer.diagnostics().displacement_faults(), 1);
    assert!(samples.iter().all(|sample| *sample == 0.0));
}

#[test]
fn private_one_shot_audio_preserves_held_live_output_after_compiled_release() {
    for compiled_first in [true, false] {
        let prepared = bound_with_compiled_note_held_at_boundary(compiled_first);
        let plan = std::sync::Arc::clone(&prepared.owner.control.plan);
        let partition = std::sync::Arc::clone(&prepared.owner.control.partition);
        let epoch = prepared.epoch();
        let table = prepared.table_id();
        let anchor = prepared.owner.control.anchor;
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(256))
            .expect("closed compiled history");
        let suffix = prepared.prepare_suffix(history).expect("empty suffix");
        let candidate = prepared.stamp_suffix(suffix).expect("stamp");
        let (_, mut audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("private arm");
        let live = audio
            .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("test live onset");
        let mut reference =
            PreparedRenderer::prepare(plan, anchor, epoch, table).expect("reference renderer");
        assert!(reference.bind_mixed_partition(partition));
        let live_event = TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::ZERO, TestOrigin::Simulated),
            EventPayload::Note {
                identity: live,
                edge: NoteEdge::On {
                    slot: audio.audio.note,
                    key: key(48),
                    velocity: NoteVelocity::FULL,
                },
            },
        );
        let mut mixed = [0.0_f32; 256];
        let mixed_output = AudioBlockMut::new(&mut mixed, 256, ChannelLayout::Mono).expect("mixed");
        audio.render_private(mixed_output).expect("mixed render");
        let mut live_only = [0.0_f32; 256];
        let reference_output =
            AudioBlockMut::new(&mut live_only, 256, ChannelLayout::Mono).expect("live reference");
        reference
            .render(reference_output, TimedEvents::new(&[live_event]))
            .expect("live render");
        assert!(audio.report().adopted);
        assert_eq!(audio.report().released_compiled, HeldNoteCount::measured(1));
        // The first output quantum is the initial carry. Frames 64..128 hold
        // the old compiled note; frames 128..192 are the adoption quantum.
        assert_ne!(&mixed[64..128], &live_only[64..128]);
        assert_eq!(&mixed[128..], &live_only[128..]);
        assert!(live_only[128..].iter().any(|sample| *sample != 0.0));
    }
}

#[test]
fn private_mixed_sounding_snapshot_separates_unpublished_and_boundary_released_identities() {
    for compiled_first in [true, false] {
        let prepared = bound_with_compiled_note_held_at_boundary(compiled_first);
        let EventPayload::Note {
            identity: compiled,
            edge: NoteEdge::On { .. },
        } = prepared.events[0].payload()
        else {
            panic!("initial compiled onset");
        };
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(256))
            .expect("history");
        let suffix = prepared.prepare_suffix(history).expect("suffix");
        let candidate = prepared.stamp_suffix(suffix).expect("stamp");
        let (_, mut audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("private arm");
        let live = audio
            .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("mint test-live identity");
        let before = audio
            .audio
            .renderer
            .snapshot_mixed_sounding()
            .expect("bound registry");
        assert!(before.compiled().is_empty());
        assert!(before.live().is_empty(), "minting is not sounding");

        let mut old_output = [0.0_f32; 128];
        let block = AudioBlockMut::new(&mut old_output, 128, ChannelLayout::Mono).expect("old");
        audio.render_private(block).expect("old quantum");
        let sounding = audio
            .audio
            .renderer
            .snapshot_mixed_sounding()
            .expect("stopped old quantum");
        assert_eq!(
            sounding
                .compiled()
                .iter()
                .map(|note| note.identity)
                .collect::<Vec<_>>(),
            vec![compiled]
        );
        assert_eq!(
            sounding
                .live()
                .iter()
                .map(|note| note.identity)
                .collect::<Vec<_>>(),
            vec![live]
        );

        let mut boundary_output = [0.0_f32; 1];
        let block =
            AudioBlockMut::new(&mut boundary_output, 1, ChannelLayout::Mono).expect("boundary");
        audio.render_private(block).expect("adopted quantum");
        let after = audio
            .audio
            .renderer
            .snapshot_mixed_sounding()
            .expect("stopped adopted quantum");
        assert!(after.compiled().is_empty());
        assert_eq!(
            after
                .live()
                .iter()
                .map(|note| note.identity)
                .collect::<Vec<_>>(),
            vec![live]
        );
        assert_eq!(audio.report().released_compiled, HeldNoteCount::measured(1));
        assert_eq!(
            audio.audio.compiled_ended[0].map(|note| note.identity),
            Some(compiled),
            "boundary-ended identity remains separate from current sounding notes"
        );
    }
}

#[test]
fn private_pending_collection_discards_unpublished_reservations() {
    for compiled_first in [true, false] {
        let (prepared, candidate) = one_shot_with_boundary_on(compiled_first);
        let (control, mut audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("private arm");
        let unpublished = audio
            .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("test-live reservation");
        let MixedCollection::Ended(ended) = control.collect(audio).expect("pending teardown")
        else {
            panic!("pending owner must not resume");
        };
        assert_eq!(ended.end, MixedCollectionEnd::Pending);
        assert!(ended.sounding.compiled().is_empty());
        assert!(ended.sounding.live().is_empty());
        assert!(ended.boundary_ended.is_empty());
        assert_eq!(ended.unpublished_live, Some(unpublished));
    }
}

#[test]
fn private_teardown_keeps_charged_but_unregistered_live_distinct() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let live = audio
        .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("test-live reservation");
    // Model a renderer refusal after the arbiter accepted the Live charge but
    // before the registry admitted the note.
    audio.test_live_spent = true;
    audio.fault = Some(MixedOneShotRenderError::RendererFaulted);
    let MixedCollection::Ended(ended) = control.collect(audio).expect("terminal teardown") else {
        panic!("faulted owner must end");
    };
    assert_eq!(ended.end, MixedCollectionEnd::Faulted);
    assert_eq!(ended.unpublished_live, None);
    assert_eq!(ended.charged_unregistered_live, Some(live));
}

#[test]
fn private_pending_collection_classifies_sounding_compiled_and_live_separately() {
    for compiled_first in [true, false] {
        let prepared = bound_with_compiled_note_held_at_boundary(compiled_first);
        let EventPayload::Note {
            identity: compiled, ..
        } = prepared.events[0].payload()
        else {
            panic!("compiled onset");
        };
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(256))
            .expect("history");
        let suffix = prepared.prepare_suffix(history).expect("suffix");
        let candidate = prepared.stamp_suffix(suffix).expect("stamp");
        let (control, mut audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("arm");
        let live = audio
            .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("live onset");
        let mut samples = [0.0_f32; 128];
        audio
            .render_private(
                AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("old"),
            )
            .expect("before boundary");
        let MixedCollection::Ended(ended) = control.collect(audio).expect("pending teardown")
        else {
            panic!("pending owner must end");
        };
        assert_eq!(ended.end, MixedCollectionEnd::Pending);
        assert_eq!(
            ended
                .sounding
                .compiled()
                .iter()
                .map(|note| note.identity)
                .collect::<Vec<_>>(),
            vec![compiled]
        );
        assert_eq!(
            ended
                .sounding
                .live()
                .iter()
                .map(|note| note.identity)
                .collect::<Vec<_>>(),
            vec![live]
        );
        assert!(ended.boundary_ended.is_empty());
        assert_eq!(ended.unpublished_live, None);
    }
}

#[test]
fn private_collection_returns_crossed_pending_pairs_unchanged() {
    let (first, candidate_a) = one_shot_with_boundary_on(true);
    let (second, candidate_b) = one_shot_with_boundary_on(false);
    let (control_a, audio_a) = first
        .arm_one_shot(candidate_a, &mixed_profile())
        .expect("first");
    let (control_b, audio_b) = second
        .arm_one_shot(candidate_b, &mixed_profile())
        .expect("second");
    let refused = control_a.collect(audio_b).expect_err("crossed pair");
    assert_eq!(refused.reason, MixedCollectionError::CrossedPair);
    let MixedCollectionRefusal {
        control: control_a,
        audio: audio_b,
        ..
    } = *refused;
    assert!(matches!(
        control_a.collect(audio_a),
        Ok(MixedCollection::Ended(_))
    ));
    assert!(matches!(
        control_b.collect(audio_b),
        Ok(MixedCollection::Ended(_))
    ));
}

#[test]
fn private_collection_uses_birth_pairing_even_when_capsule_names_other_owner() {
    let (first, candidate_a) = one_shot_with_boundary_on(true);
    let (second, candidate_b) = one_shot_with_boundary_on(false);
    let (control_a, audio_a) = first
        .arm_one_shot(candidate_a, &mixed_profile())
        .expect("first");
    let (control_b, mut audio_b) = second
        .arm_one_shot(candidate_b, &mixed_profile())
        .expect("second");
    audio_b.capsule.plan = control_a.control.plan.id();
    audio_b.capsule.epoch = control_a.control.epoch;
    audio_b.capsule.table = control_a.control.minter.id();
    let refused = control_a.collect(audio_b).expect_err("crossed birth pair");
    assert_eq!(refused.reason, MixedCollectionError::CrossedPair);
    let MixedCollectionRefusal {
        control: control_a,
        audio: audio_b,
        ..
    } = *refused;
    assert!(matches!(
        control_a.collect(audio_a),
        Ok(MixedCollection::Ended(_))
    ));
    assert!(matches!(
        control_b.collect(audio_b),
        Ok(MixedCollection::Ended(_))
    ));
}

#[test]
fn private_collection_returns_crossed_adopted_pairs_unchanged() {
    let (first, candidate_a) = one_shot_with_boundary_on(true);
    let (second, candidate_b) = one_shot_with_boundary_on(false);
    let (control_a, mut audio_a) = first
        .arm_one_shot(candidate_a, &mixed_profile())
        .expect("first");
    let (control_b, mut audio_b) = second
        .arm_one_shot(candidate_b, &mixed_profile())
        .expect("second");
    for audio in [&mut audio_a, &mut audio_b] {
        let mut samples = [0.0_f32; 129];
        audio
            .render_private(
                AudioBlockMut::new(&mut samples, 129, ChannelLayout::Mono).expect("block"),
            )
            .expect("adopt");
        assert!(audio.report().adopted);
    }
    let refused = control_a
        .collect(audio_b)
        .expect_err("crossed adopted pair");
    assert_eq!(refused.reason, MixedCollectionError::CrossedPair);
    let MixedCollectionRefusal {
        control: control_a,
        audio: audio_b,
        ..
    } = *refused;
    for result in [control_a.collect(audio_a), control_b.collect(audio_b)] {
        let MixedCollection::Resumed { control, audio, .. } = result.expect("correct rejoin")
        else {
            panic!("healthy adopted pair must resume");
        };
        let ended = control.teardown(*audio).expect("final teardown");
        assert_eq!(ended.end, MixedCollectionEnd::ResumedTeardown);
    }
}

#[test]
fn private_collection_never_promotes_corrupted_adopted_capsule() {
    for corrupt in ["plan", "epoch", "table"] {
        let (prepared, candidate) = one_shot_with_boundary_on(true);
        let foreign = bound(false);
        let (control, mut audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("private arm");
        let mut samples = [0.0_f32; 129];
        audio
            .render_private(
                AudioBlockMut::new(&mut samples, 129, ChannelLayout::Mono).expect("block"),
            )
            .expect("adopted render");
        assert!(audio.report().adopted);
        match corrupt {
            "plan" => audio.capsule.plan = foreign.owner.control.plan.id(),
            "epoch" => audio.capsule.epoch = foreign.epoch(),
            "table" => audio.capsule.table = foreign.table_id(),
            _ => panic!("known field"),
        }
        let MixedCollection::Ended(ended) = control.collect(audio).expect("correct pair") else {
            panic!("corrupted capsule must not promote: {corrupt}");
        };
        assert_eq!(ended.end, MixedCollectionEnd::PromotionRefused, "{corrupt}");
        assert!(!ended.report.faulted);
        assert_eq!(ended.boundary_ended.len(), 1);
        assert_eq!(ended.sounding.compiled().len(), 1);
    }
}

#[test]
fn private_resumed_terminal_pairing_fault_can_teardown_with_boundary_history() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let foreign = bound(false);
    let (control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let mut samples = [0.0_f32; 129];
    audio
        .render_private(AudioBlockMut::new(&mut samples, 129, ChannelLayout::Mono).expect("block"))
        .expect("adopted render");
    let MixedCollection::Resumed {
        control, mut audio, ..
    } = control.collect(audio).expect("healthy rejoin")
    else {
        panic!("healthy adopted pair must resume");
    };
    audio.capsule.table = foreign.table_id();
    let mut later = [1.0_f32; 1];
    let error = audio
        .render_private(AudioBlockMut::new(&mut later, 1, ChannelLayout::Mono).expect("later"))
        .expect_err("capsule table mismatch");
    assert_eq!(error, MixedOneShotRenderError::Pairing);
    assert_eq!(later, [0.0]);
    let ended = control
        .teardown(*audio)
        .expect("original resumed pair teardown");
    assert_eq!(ended.end, MixedCollectionEnd::ResumedTeardown);
    assert_eq!(ended.report.fault, Some(error));
    assert_eq!(ended.boundary_ended.len(), 1);
}

#[test]
fn private_adopted_collection_promotes_one_authority_and_reclaims_retired_list() {
    for compiled_first in [true, false] {
        let (prepared, candidate) =
            one_shot_with_boundary_on_at(compiled_first, SampleTime::new(65));
        let EventPayload::Note {
            identity: original, ..
        } = prepared.events[0].payload()
        else {
            panic!("initial compiled onset");
        };
        let successor = candidate.outstanding[0];
        let (control, mut audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("private arm");
        let mut before = [0.0_f32; 192];
        audio
            .render_private(AudioBlockMut::new(&mut before, 192, ChannelLayout::Mono).expect("old"))
            .expect("old quanta");
        assert!(!audio.report().adopted);
        let mut boundary = [0.0_f32; 1];
        audio
            .render_private(
                AudioBlockMut::new(&mut boundary, 1, ChannelLayout::Mono).expect("boundary"),
            )
            .expect("adopted quantum");
        assert!(audio.report().boundary_quantum_completed);
        let MixedCollection::Resumed {
            control,
            mut audio,
            retired,
        } = control.collect(audio).expect("adopted rejoin")
        else {
            panic!("healthy adopted owner must resume");
        };
        assert_eq!(
            control.control.anchor,
            StreamAnchor::new(SampleTime::new(128), PlanPosition::new(10))
        );
        assert_eq!(
            control.sequence,
            ActivationSequence::INITIAL.next().expect("successor")
        );
        assert_eq!(control.outstanding, vec![successor]);
        assert_eq!(control.control.minter.resolve(successor), Resolution::Live);
        assert!(audio.capsule.minter.is_none());
        assert!(audio.capsule.events.is_empty());
        assert_eq!(retired.event_count, 1);
        assert_eq!(retired.unconsumed_from, 1);
        assert_eq!(
            retired.anchor,
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO)
        );
        let mut later = [0.0_f32; 64];
        audio
            .render_private(AudioBlockMut::new(&mut later, 64, ChannelLayout::Mono).expect("later"))
            .expect("resumed rendering");
        let ended = control.teardown(*audio).expect("resumed teardown");
        assert_eq!(ended.end, MixedCollectionEnd::ResumedTeardown);
        assert_eq!(
            ended
                .sounding
                .compiled()
                .iter()
                .map(|note| note.identity)
                .collect::<Vec<_>>(),
            vec![successor]
        );
        assert_eq!(
            ended
                .boundary_ended
                .iter()
                .map(|note| note.identity)
                .collect::<Vec<_>>(),
            vec![original]
        );
    }
}

#[test]
fn private_promotion_refusal_and_terminal_fault_end_the_correct_pair() {
    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let (control, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("arm");
    let mut samples = [0.0_f32; 129];
    audio
        .render_private(AudioBlockMut::new(&mut samples, 129, ChannelLayout::Mono).expect("block"))
        .expect("adopt");
    audio.capsule.outstanding_count = HeldNoteCount::NONE;
    let MixedCollection::Ended(ended) = control.collect(audio).expect("defensive collection")
    else {
        panic!("corrupt promotion must terminate");
    };
    assert_eq!(ended.end, MixedCollectionEnd::PromotionRefused);

    let (prepared, candidate) = one_shot_with_boundary_on(true);
    let EventPayload::Note { identity: old, .. } = prepared.events[0].payload() else {
        panic!("compiled onset");
    };
    let restoration = candidate.restoration_count;
    let session = restoration
        .checked_add(EventCount::measured(1))
        .expect("release credit");
    let (control, mut audio) = prepared
        .arm_one_shot(candidate, &profile_with_session_share(session))
        .expect("arm");
    audio.capsule.restoration_count = session;
    let mut samples = [0.0_f32; 129];
    assert!(
        audio
            .render_private(
                AudioBlockMut::new(&mut samples, 129, ChannelLayout::Mono).expect("block")
            )
            .is_err()
    );
    let MixedCollection::Ended(ended) = control.collect(audio).expect("terminal teardown") else {
        panic!("faulted owner must terminate");
    };
    assert_eq!(ended.end, MixedCollectionEnd::Faulted);
    assert!(!ended.report.boundary_quantum_completed);
    assert!(ended.sounding.compiled().is_empty());
    assert_eq!(
        ended
            .boundary_ended
            .iter()
            .map(|note| note.identity)
            .collect::<Vec<_>>(),
        vec![old]
    );
}

#[test]
fn prefix_restores_last_magnitude_and_zero_gate_in_both_producer_orders() {
    for compiled_first in [true, false] {
        let prepared = bound(compiled_first);
        let plan = &prepared.owner.control.plan;
        let partition = &prepared.owner.control.partition;
        let note = plan.resolve_note(ENVELOPE).expect("note");
        let gate = plan.note_targets()[note.index()].parameter;
        let frequency = plan
            .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
            .expect("frequency");
        let controller = plan
            .resolve_parameter(CONTROLLER, parameters::SOURCE_VALUE)
            .expect("controller");
        let note_source = plan
            .resolve_parameter(NOTE_SOURCE, parameters::SOURCE_VALUE)
            .expect("note source");
        let table_before = prepared.table_id();
        let outstanding_before = prepared.outstanding_count();
        let candidate = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(30))
            .expect("bounded prefix");
        assert_eq!(candidate.plan_id(), plan.id());
        assert_eq!(candidate.epoch(), prepared.epoch());
        assert_eq!(candidate.table_id(), table_before);
        assert_eq!(candidate.requested(), SampleTime::new(64));
        assert_eq!(candidate.position(), PlanPosition::new(30));
        assert_eq!(candidate.prefix_end(), 3);
        assert_eq!(candidate.open_at_destination_count().get(), 1);
        assert_eq!(candidate.book.entries().len(), 1);
        assert_eq!(candidate.open_at_destination.len(), 1);
        assert_eq!(prepared.table_id(), table_before);
        assert_eq!(prepared.outstanding_count(), outstanding_before);
        assert_eq!(
            candidate.restoration_count().get() as usize,
            partition.restoration_groups().len()
        );
        assert_eq!(restore(&candidate, gate).value(), ParameterValue::ZERO);
        assert_eq!(restore(&candidate, gate).controller(), None);
        let expected_pitch = ParameterValue::from_frequency(
            crate::tuning::PreparedTuning::equal_temperament()
                .expect("tuning")
                .frequency_of(key(72)),
        );
        assert_eq!(restore(&candidate, frequency).value(), expected_pitch);
        assert_eq!(restore(&candidate, frequency).controller(), None);
        assert_eq!(
            restore(&candidate, controller).value(),
            plan.parameter_targets()[controller.index()].base
        );
        assert_eq!(restore(&candidate, controller).controller(), Some(None));
        assert_eq!(
            restore(&candidate, note_source).value(),
            plan.parameter_targets()[note_source.index()].base
        );
        for event in &candidate.restoration {
            let EventPayload::ScopedRestore(scoped) = event.payload() else {
                panic!("private batch must be scoped");
            };
            assert_eq!(event.envelope().time(), SampleTime::new(64));
            assert!(partition.restoration_groups().iter().any(|group| {
                group.parameter() == scoped.slot() && group.instances() == scoped.instances()
            }));
        }
    }
}

#[test]
fn destination_events_stay_in_suffix_and_unrepresentable_time_refuses_without_mutation() {
    let prepared = bound(false);
    let at_off = prepared
        .prepare_history(SampleTime::ZERO, PlanPosition::new(20))
        .expect("off at destination belongs to suffix");
    assert_eq!(at_off.prefix_end(), 2);
    assert_eq!(at_off.open_at_destination_count().get(), 2);
    let table_before = prepared.table_id();
    let outstanding_before = prepared.outstanding_count();
    assert_eq!(
        prepared
            .prepare_history(SampleTime::new(u64::MAX), PlanPosition::new(20))
            .expect_err("no next boundary"),
        MixedHistoryPrepareError::BoundaryUnrepresentable {
            at: SampleTime::new(u64::MAX)
        }
    );
    assert_eq!(prepared.table_id(), table_before);
    assert_eq!(prepared.outstanding_count(), outstanding_before);
}

#[test]
fn equal_position_order_and_repeated_key_pairing_are_retained() {
    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![
                PlanEvent::new(
                    PlanPosition::ZERO,
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(60),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(10),
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(60),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(10),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(10),
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(72),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(20),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(72),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(30),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
            ]
        });
        let before = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("destination events are suffix events");
        assert_eq!(before.prefix_end(), 1);
        assert_eq!(before.open_at_destination.len(), 1);
        assert_eq!(before.open_at_destination[0].key, key(60));

        let after = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(20))
            .expect("same-position prefix is admitted in order");
        assert_eq!(after.prefix_end(), 4);
        assert_eq!(after.open_at_destination.len(), 2);
        assert_eq!(after.open_at_destination[0].key, key(60));
        assert_eq!(after.open_at_destination[0].opened, 0);
        assert_eq!(after.open_at_destination[1].key, key(72));
        let frequency = prepared
            .owner
            .control
            .plan
            .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
            .expect("frequency");
        let expected_pitch = ParameterValue::from_frequency(
            crate::tuning::PreparedTuning::equal_temperament()
                .expect("tuning")
                .frequency_of(key(72)),
        );
        assert_eq!(restore(&after, frequency).value(), expected_pitch);
    }
}

#[test]
fn sample_positioned_controller_is_refused_at_target_binding() {
    for compiled_first in [true, false] {
        let mut plan = mixed_plan(compiled_first);
        let controller = plan
            .resolve_parameter(CONTROLLER, parameters::SOURCE_VALUE)
            .expect("controller source value");
        assert!(plan.set_parameter_rate_for_test(controller, ControlRate::Sample));
        let note = plan.resolve_note(ENVELOPE).expect("playable envelope");
        let stream = AdmittedCompiledStream::admit(&plan, &[]).expect("empty stream");
        let failure = MixedTargetAdmission::admit(plan, stream, note)
            .expect_err("sample-positioned controller cannot bind");
        assert_eq!(
            failure.reason(),
            crate::host::mixed_targets::MixedTargetError::SampleController { slot: controller }
        );
        let (plan, stream, returned_note) = failure.into_inputs();
        assert_eq!(returned_note, note);
        assert_eq!(stream.plan(), plan.id());
    }
}

#[test]
fn destination_open_sampler_trigger_is_part_of_zeroed_history() {
    for compiled_first in [true, false] {
        let rate = SampleRate::new(48_000.0).expect("rate");
        let sample = PreparedSample::prepare(vec![0.25; 4096], ChannelLayout::Mono, rate)
            .expect("finite sample");
        let region = PlaybackRegion::new(SampleFrame::new(0), SampleFrame::new(4096))
            .expect("nonempty region");
        let compiled = NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(1),
            simultaneous_holds: EventCount::NONE,
        };
        let live = NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(1),
            simultaneous_holds: EventCount::measured(1),
        };
        let ir = GraphIr::builder()
            .sample(sample)
            .sample_map(SampleMap::new(vec![SampleZone::new(
                SampleRef::new(0),
                KeyIdentity::LOWEST,
                region,
            )]))
            .node(
                SOURCE,
                IrNodeKind::Sampler {
                    map: SampleMapRef::new(0),
                    level: Amplitude::UNITY,
                    velocity_sensitivity: NormalizedLevel::FULL,
                    start_offset: NormalizedLevel::ZERO,
                    play_mode: PlayMode::Sustain,
                    direction: PlayDirection::Forward,
                },
                ExecutionScope::Voice,
            )
            .node(
                ENVELOPE,
                IrNodeKind::Envelope {
                    attack: Seconds::ZERO,
                    decay: Seconds::ZERO,
                    sustain: NormalizedLevel::FULL,
                    release: Seconds::ZERO,
                    velocity_sensitivity: NormalizedLevel::ZERO,
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
                (AMPLIFIER, crate::node::AMPLIFIER_CONTROL),
                SignalDomain::Control,
            )
            .connect(
                (AMPLIFIER, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .tuning(
                ExecutionScope::Voice,
                crate::tuning::PreparedTuning::equal_temperament().expect("tuning"),
            )
            .declaring(PlanDeclarations {
                note_producers: if compiled_first {
                    vec![compiled, live]
                } else {
                    vec![live, compiled]
                },
                held_notes: HeldNoteCount::measured(2),
                ..PlanDeclarations::default()
            })
            .build()
            .expect("sampler graph");
        let profile =
            HostProfile::harness(rate, FrameCount::new(512), ChannelLayout::Mono).expect("profile");
        let plan = compile(&ir, &RenderConfig::new(profile))
            .into_plan()
            .expect("sampler plan");
        let note = plan.resolve_note(ENVELOPE).expect("playable envelope");
        let trigger = plan
            .resolve_parameter(SOURCE, parameters::SAMPLER_TRIGGER)
            .expect("sampler trigger");
        assert!(super::super::gate_rows(&plan)[note.index()].contains(&trigger.index()));
        let events = [
            PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: KeyIdentity::LOWEST,
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(30),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: KeyIdentity::LOWEST,
                },
            ),
        ];
        let stream = AdmittedCompiledStream::admit(&plan, &events).expect("sampler stream");
        let binding = MixedTargetAdmission::admit(plan, stream, note).expect("sampler binding");
        let prepared = MixedJoinedStream::open(
            binding,
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        )
        .expect("sampler owner")
        .prepare_initial()
        .expect("initial sampler schedule");
        let candidate = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(20))
            .expect("sampler prefix");
        assert_eq!(candidate.open_at_destination_count().get(), 1);
        assert_eq!(restore(&candidate, trigger).value(), ParameterValue::ZERO);
    }
}

#[test]
fn suffix_pairs_repeated_keys_first_and_counts_only_crossing_obligations() {
    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![
                PlanEvent::new(
                    PlanPosition::ZERO,
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(60),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(10),
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(60),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(15),
                    CompiledPayload::Bend {
                        slot: note,
                        key: key(60),
                        cents: Cents::new(50.0).expect("finite bend"),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(20),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(25),
                    CompiledPayload::Bend {
                        slot: note,
                        key: key(60),
                        cents: Cents::new(25.0).expect("finite bend"),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(26),
                    CompiledPayload::Expression {
                        slot: note,
                        key: key(60),
                        expression: crate::controller::NoteExpression::Pressure(
                            NormalizedLevel::FULL,
                        ),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(30),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
            ]
        });
        let table_before = prepared.table_id();
        let outstanding_before = prepared.outstanding_count();
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("prefix note is open");
        let destination_open = history.open_at_destination.clone();
        let suffix = prepared.prepare_suffix(history).expect("bound suffix");
        assert_eq!(suffix.included, vec![1, 2, 3]);
        assert_eq!(suffix.included_event_count().get(), 3);
        assert_eq!(suffix.omitted_expression_count().get(), 2);
        assert_eq!(suffix.omitted_release_count().get(), 1);
        assert_eq!(suffix.open_at_destination_count().get(), 1);
        assert_eq!(suffix.history.open_at_destination, destination_open);
        assert!(suffix.history.book.entries().is_empty());
        assert_eq!(prepared.table_id(), table_before);
        assert_eq!(prepared.outstanding_count(), outstanding_before);
    }
}

#[test]
fn a_suffix_candidate_cannot_be_used_by_another_prepared_owner() {
    let prepared = bound(true);
    let other = bound(true);
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(20))
        .expect("prefix");
    assert_eq!(
        other
            .prepare_suffix(history)
            .expect_err("foreign table and epoch"),
        MixedSuffixPrepareError::ForeignCandidate
    );
    assert_eq!(other.outstanding_count(), 0);
}

#[test]
fn crossing_release_is_absent_from_private_suffix_selection() {
    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![
                PlanEvent::new(
                    PlanPosition::ZERO,
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(60),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(10),
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(72),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(20),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(30),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(72),
                    },
                ),
            ]
        });
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("one prefix note");
        let suffix = prepared.prepare_suffix(history).expect("bound suffix");
        assert_eq!(suffix.included, vec![1, 3]);
        assert_eq!(suffix.omitted_release_count().get(), 1);
        assert_eq!(suffix.omitted_expression_count().get(), 0);
        assert_eq!(suffix.open_at_destination_count().get(), 1);
    }
}

#[test]
fn private_stamp_releases_only_its_copy_and_orders_restoration_before_destination_on() {
    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![
                PlanEvent::new(
                    PlanPosition::ZERO,
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(60),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(10),
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(72),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(20),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
            ]
        });
        let original = prepared.outstanding[0];
        assert_eq!(
            prepared.owner.control.minter.resolve(original),
            Resolution::Live
        );
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("prefix");
        let suffix = prepared.prepare_suffix(history).expect("suffix selection");
        assert_eq!(suffix.included, vec![1]);
        let stamped = prepared.stamp_suffix(suffix).expect("private stamp");
        let restoration_count = prepared.owner.control.partition.restoration_groups().len();
        assert_eq!(
            stamped.anchor(),
            StreamAnchor::new(SampleTime::new(64), PlanPosition::new(10))
        );
        assert_eq!(stamped.supersedes(), ActivationSequence::INITIAL);
        assert_eq!(
            stamped.restoration_count().as_usize(),
            Some(restoration_count)
        );
        assert!(stamped.suffix.history.restoration.is_empty());
        assert_eq!(stamped.suffix.history.restoration_count(), EventCount::NONE);
        assert_eq!(
            stamped.event_count().as_usize(),
            Some(restoration_count + 1)
        );
        assert_eq!(stamped.outstanding_count().get(), 1);
        assert_eq!(stamped.minter.live(), 1);
        assert_eq!(stamped.outstanding.len(), 1);
        assert_eq!(
            stamped.minter.resolve(stamped.outstanding[0]),
            Resolution::Live
        );
        assert_eq!(
            prepared.owner.control.minter.resolve(original),
            Resolution::Live,
            "the authoritative compiled range cannot be released by rehearsal"
        );
        for event in &stamped.events[..restoration_count] {
            assert!(matches!(event.payload(), EventPayload::ScopedRestore(_)));
            assert_eq!(event.envelope().time(), SampleTime::new(64));
        }
        let last = stamped.events[restoration_count];
        assert!(matches!(
            last.payload(),
            EventPayload::Note {
                edge: crate::render::NoteEdge::On { key: on_key, .. },
                ..
            } if on_key == key(72)
        ));
        assert_eq!(last.envelope().time(), SampleTime::new(64));
        let timing = stamped
            .effective_timing(SampleTime::new(128))
            .expect("next boundary fits the complete private list");
        assert_eq!(timing.effective(), SampleTime::new(128));
        assert_eq!(timing.shift(), FrameCount::new(64));
        assert!(stamped.events.iter().all(|event| {
            event.envelope().time().checked_add(timing.shift()) == Ok(SampleTime::new(128))
        }));
        assert_eq!(stamped.events[0].envelope().time(), SampleTime::new(64));
        assert_eq!(
            stamped
                .effective_timing(SampleTime::new(0))
                .expect_err("earlier boundary"),
            MixedEffectiveTimeError::BeforeRequested {
                requested: SampleTime::new(64),
                effective: SampleTime::ZERO,
            }
        );
        assert_eq!(
            stamped
                .effective_timing(SampleTime::new(65))
                .expect_err("incomplete quantum"),
            MixedEffectiveTimeError::NotQuantumBoundary {
                effective: SampleTime::new(65),
            }
        );
        let last_boundary = u64::MAX - (u64::MAX % u64::from(crate::time::QUANTUM_FRAMES));
        assert_eq!(
            stamped.effective_timing(SampleTime::new(last_boundary)),
            Err(MixedEffectiveTimeError::DisplacementUnrepresentable {
                requested: SampleTime::new(64),
                effective: SampleTime::new(last_boundary),
            })
        );
        assert!(
            stamped
                .events
                .iter()
                .all(|event| !matches!(event.payload(), EventPayload::SetParameter { .. }))
        );
        assert!(stamped.outstanding.iter().all(|identity| {
            prepared
                .owner
                .control
                .minter
                .span()
                .contains(identity.index())
        }));
    }
}

#[test]
fn private_audio_capsule_is_sendable_without_moving_control_or_source_history() {
    fn require_send<T: Send>() {}
    require_send::<MixedAudioCandidate>();

    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            )]
        });
        let old = prepared.outstanding[0];
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("prefix");
        let suffix = prepared.prepare_suffix(history).expect("suffix");
        let omitted = (
            suffix.omitted_release_count(),
            suffix.omitted_expression_count(),
        );
        let stamped = prepared.stamp_suffix(suffix).expect("private stamp");
        let event_count = stamped.event_count();
        let restoration_count = stamped.restoration_count();
        let outstanding_count = stamped.outstanding_count();
        let capsule = stamped.into_audio_capsule().expect("successor sequence");
        assert_eq!(capsule.plan, prepared.owner.control.plan.id());
        assert_eq!(capsule.epoch, prepared.epoch());
        assert_eq!(capsule.table, prepared.table_id());
        assert_eq!(
            capsule.anchor,
            StreamAnchor::new(SampleTime::new(64), PlanPosition::new(10))
        );
        assert_eq!(capsule.supersedes, ActivationSequence::INITIAL);
        assert_eq!(
            capsule.sequence,
            ActivationSequence::INITIAL.next().unwrap()
        );
        assert_eq!(capsule.event_count, event_count);
        assert_eq!(capsule.events.len(), event_count.as_usize().unwrap());
        assert_eq!(capsule.restoration_count, restoration_count);
        assert_eq!(capsule.outstanding_count, outstanding_count);
        assert_eq!(
            capsule.outstanding.len(),
            outstanding_count.as_usize().unwrap()
        );
        assert_eq!(
            (capsule.omitted_releases, capsule.omitted_expressions),
            omitted
        );
        assert_eq!(
            capsule.minter.as_ref().map(CompiledRangeMinter::live),
            Some(outstanding_count.get())
        );
        assert_eq!(
            prepared.owner.control.minter.resolve(old),
            Resolution::Live,
            "boxing a copied minter must not alter authoritative custody"
        );
        drop(capsule);
    }
}

#[test]
fn private_one_shot_arm_fixes_boundary_and_splits_identity_custody() {
    fn require_send<T: Send>() {}
    require_send::<MixedOneShotAudio>();

    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            )]
        });
        let old = prepared.outstanding[0];
        let history = prepared
            .prepare_history(SampleTime::new(65), PlanPosition::new(10))
            .expect("bound history");
        let suffix = prepared.prepare_suffix(history).expect("bound suffix");
        let omitted = (
            suffix.omitted_release_count(),
            suffix.omitted_expression_count(),
        );
        let candidate = prepared.stamp_suffix(suffix).expect("private stamp");
        let restoration_count = candidate.restoration_count();
        let (control, audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("one arm");
        assert_eq!(control.control.minter.resolve(old), Resolution::Live);
        assert_eq!(control.outstanding, vec![old]);
        assert_eq!(audio.audio.renderer.clock(), SampleTime::ZERO);
        assert_eq!(audio.events[0].envelope().time(), SampleTime::ZERO);
        assert_eq!(audio.next, 0);
        assert_eq!(audio.timing.effective(), SampleTime::new(128));
        assert_eq!(audio.timing.shift(), FrameCount::new(63));
        assert_eq!(
            audio.effective_anchor,
            StreamAnchor::new(SampleTime::new(128), PlanPosition::new(10))
        );
        assert!(!audio.late_at_arm);
        assert_eq!(audio.in_force, ActivationSequence::INITIAL);
        assert_eq!(audio.capsule.supersedes, audio.in_force);
        assert_eq!(audio.capsule.sequence, audio.in_force.next().unwrap());
        assert_eq!(audio.capsule.plan, control.control.plan.id());
        assert_eq!(audio.capsule.epoch, control.control.epoch());
        assert_eq!(audio.capsule.table, control.control.table_id());
        assert_eq!(
            audio.capsule.anchor,
            StreamAnchor::new(SampleTime::new(65), PlanPosition::new(10))
        );
        assert_eq!(
            audio.capsule.events.len(),
            audio.capsule.event_count.as_usize().unwrap()
        );
        assert_eq!(
            audio.capsule.outstanding.len(),
            audio.capsule.outstanding_count.as_usize().unwrap()
        );
        assert_eq!(
            audio.capsule.minter.as_ref().map(CompiledRangeMinter::live),
            Some(audio.capsule.outstanding_count.get())
        );
        assert_eq!(audio.capsule.restoration_count, restoration_count);
        assert_eq!(
            (
                audio.capsule.omitted_releases,
                audio.capsule.omitted_expressions
            ),
            omitted
        );
    }
}

#[test]
fn private_one_shot_arm_returns_foreign_candidate_without_changing_owner() {
    for compiled_first in [true, false] {
        let prepared = bound_with_events(compiled_first, |note| {
            vec![PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            )]
        });
        let foreign = bound(!compiled_first);
        let history = foreign
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("foreign history");
        let suffix = foreign.prepare_suffix(history).expect("foreign suffix");
        let candidate = foreign.stamp_suffix(suffix).expect("foreign stamp");
        let epoch = prepared.epoch();
        let table = prepared.table_id();
        let original = prepared.outstanding[0];
        let count = prepared.event_count();
        let refused = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect_err("foreign epoch");
        assert_eq!(refused.reason, MixedOneShotArmError::ForeignCandidate);
        assert_eq!(refused.owner.epoch(), epoch);
        assert_eq!(refused.owner.table_id(), table);
        assert_eq!(refused.owner.event_count(), count);
        assert_eq!(refused.owner.owner.audio.renderer.clock(), SampleTime::ZERO);
        assert_eq!(
            refused.owner.owner.control.minter.resolve(original),
            Resolution::Live
        );
        assert_eq!(
            refused.owner.owner.audio.renderer.anchor_for_test(),
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO)
        );
        assert_eq!(refused.candidate.anchor.time(), SampleTime::new(64));
        assert_eq!(
            refused.candidate.event_count().as_usize(),
            Some(refused.candidate.events.len())
        );
        assert_eq!(
            refused.candidate.outstanding_count().as_usize(),
            Some(refused.candidate.outstanding.len())
        );
        let returned = refused.owner;
        let history = returned
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("retry history");
        let suffix = returned.prepare_suffix(history).expect("retry suffix");
        let candidate = returned.stamp_suffix(suffix).expect("retry stamp");
        let (control, audio) = returned
            .arm_one_shot(candidate, &mixed_profile())
            .expect("retry arm");
        assert_eq!(control.control.minter.resolve(original), Resolution::Live);
        assert_eq!(audio.timing.effective(), SampleTime::new(64));
    }
}

#[test]
fn private_one_shot_arm_uses_on_grid_request_without_extra_quantum() {
    let prepared = bound(true);
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (_, audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("arm");
    assert_eq!(audio.timing.effective(), SampleTime::new(64));
    assert_eq!(audio.timing.shift(), FrameCount::ZERO);
    assert!(!audio.late_at_arm);
}

#[test]
fn private_one_shot_arm_uses_next_unrendered_quantum_after_request() {
    let mut prepared = bound_with_events(true, |note| {
        vec![PlanEvent::new(
            PlanPosition::new(128),
            CompiledPayload::NoteOn {
                slot: note,
                key: key(60),
                velocity: NoteVelocity::FULL,
            },
        )]
    });
    let _ = render_quantum(&mut prepared.owner.audio.renderer, &[]);
    let _ = render_quantum(&mut prepared.owner.audio.renderer, &[]);
    assert_eq!(prepared.owner.audio.renderer.clock(), SampleTime::new(64));
    let history = prepared
        .prepare_history(SampleTime::new(32), PlanPosition::ZERO)
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    let (_, audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("arm");
    assert_eq!(audio.timing.effective(), SampleTime::new(64));
    assert_eq!(audio.timing.shift(), FrameCount::new(32));
    assert!(audio.late_at_arm);
    assert_eq!(audio.capsule.anchor.time(), SampleTime::new(32));
}

#[test]
fn private_one_shot_arm_timing_refusal_preserves_both_inputs() {
    let prepared = bound_with_events(false, |note| {
        vec![PlanEvent::new(
            PlanPosition::new(64),
            CompiledPayload::NoteOn {
                slot: note,
                key: key(60),
                velocity: NoteVelocity::FULL,
            },
        )]
    });
    let boundary = u64::MAX - (u64::MAX % u64::from(crate::time::QUANTUM_FRAMES));
    let requested = SampleTime::new(boundary - 1);
    let history = prepared
        .prepare_history(requested, PlanPosition::ZERO)
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared
        .stamp_suffix(suffix)
        .expect("stamp at final sample");
    let epoch = prepared.epoch();
    let count = prepared.event_count();
    let original = prepared.outstanding[0];
    let refused = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect_err("shift overflows");
    assert!(matches!(
        refused.reason,
        MixedOneShotArmError::Timing(MixedEffectiveTimeError::EventTimeUnrepresentable { .. })
    ));
    assert_eq!(refused.owner.epoch(), epoch);
    assert_eq!(refused.owner.event_count(), count);
    assert_eq!(refused.owner.owner.audio.renderer.clock(), SampleTime::ZERO);
    assert_eq!(
        refused.owner.owner.control.minter.resolve(original),
        Resolution::Live
    );
    assert_eq!(refused.candidate.anchor.time(), requested);
    assert_eq!(
        refused
            .candidate
            .events
            .last()
            .map(|event| event.envelope().time()),
        Some(SampleTime::new(u64::MAX))
    );
}

#[test]
fn private_one_shot_arm_refuses_short_ended_storage_and_can_retry() {
    for compiled_first in [true, false] {
        let mut prepared = bound_with_events(compiled_first, |note| {
            vec![PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            )]
        });
        let old = prepared.outstanding[0];
        let needed = prepared.owner.audio.compiled_ended.len();
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("history");
        let suffix = prepared.prepare_suffix(history).expect("suffix");
        let candidate = prepared.stamp_suffix(suffix).expect("stamp");
        prepared.owner.audio.compiled_ended.clear();
        let refused = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect_err("short buffer");
        let MixedOneShotArmRefusal {
            reason,
            mut owner,
            candidate,
        } = *refused;
        assert_eq!(
            reason,
            MixedOneShotArmError::Storage(crate::render::MixedBoundaryStorageError::Ended {
                needed,
                available: 0,
            })
        );
        assert_eq!(owner.owner.control.minter.resolve(old), Resolution::Live);
        assert_eq!(owner.owner.audio.renderer.clock(), SampleTime::ZERO);
        assert_eq!(candidate.anchor.time(), SampleTime::new(64));
        owner.owner.audio.compiled_ended.resize(needed, None);
        let (control, audio) = owner
            .arm_one_shot(candidate, &mixed_profile())
            .expect("corrected storage");
        assert_eq!(control.control.minter.resolve(old), Resolution::Live);
        assert_eq!(audio.timing.effective(), SampleTime::new(64));
    }
}

#[test]
fn private_one_shot_arm_refuses_registry_partition_mismatch_without_consuming_inputs() {
    for compiled_first in [true, false] {
        let mut prepared = bound_with_events(compiled_first, |note| {
            vec![PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            )]
        });
        let old = prepared.outstanding[0];
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(10))
            .expect("history");
        let suffix = prepared.prepare_suffix(history).expect("suffix");
        let candidate = prepared.stamp_suffix(suffix).expect("stamp");
        prepared
            .owner
            .audio
            .renderer
            .shorten_mixed_registry_range_for_test();
        let refused = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect_err("registry mismatch");
        let MixedOneShotArmRefusal {
            reason,
            mut owner,
            candidate,
        } = *refused;
        assert_eq!(
            reason,
            MixedOneShotArmError::Storage(crate::render::MixedBoundaryStorageError::Partition)
        );
        assert_eq!(owner.owner.control.minter.resolve(old), Resolution::Live);
        assert_eq!(owner.owner.audio.renderer.clock(), SampleTime::ZERO);
        assert_eq!(candidate.anchor.time(), SampleTime::new(64));
        owner
            .owner
            .audio
            .renderer
            .restore_mixed_registry_range_for_test();
        let (control, audio) = owner
            .arm_one_shot(candidate, &mixed_profile())
            .expect("restored range");
        assert_eq!(control.control.minter.resolve(old), Resolution::Live);
        assert_eq!(audio.timing.effective(), SampleTime::new(64));
    }
}

#[test]
fn private_one_shot_arm_refuses_wrong_profile_before_consuming_candidate() {
    for compiled_first in [true, false] {
        let (prepared, candidate) = one_shot_with_boundary_on(compiled_first);
        let wrong = HostProfile::harness(
            SampleRate::new(48_000.0).expect("rate"),
            FrameCount::new(256),
            ChannelLayout::Mono,
        )
        .expect("alternate profile");
        let refused = prepared
            .arm_one_shot(candidate, &wrong)
            .expect_err("foreign profile");
        assert_eq!(
            refused.reason,
            MixedOneShotArmError::Admission(MixedOneShotAdmissionError::ProfileMismatch)
        );
        assert_eq!(refused.owner.owner.audio.renderer.clock(), SampleTime::ZERO);
        let (control, audio) = refused
            .owner
            .arm_one_shot(refused.candidate, &mixed_profile())
            .expect("correct profile");
        assert_eq!(control.control.epoch, audio.audio.renderer.epoch());
        assert_eq!(
            audio.arbiter.max_events_per_quantum(),
            audio.audio.renderer.plan().max_events_per_quantum()
        );
    }
}

#[test]
fn private_one_shot_arm_refuses_restoration_plus_release_above_session_share() {
    for compiled_first in [true, false] {
        let (prepared, candidate) = one_shot_with_boundary_on(compiled_first);
        let restoration = candidate.restoration_count;
        assert!(restoration.get() > 0);
        let profile = profile_with_session_share(restoration);
        let refused = prepared
            .arm_one_shot(candidate, &profile)
            .expect_err("session boundary over share");
        assert_eq!(
            refused.reason,
            MixedOneShotArmError::Admission(MixedOneShotAdmissionError::SessionShare {
                needed: restoration
                    .checked_add(EventCount::measured(1))
                    .expect("one release"),
                available: restoration,
            })
        );
        assert_eq!(refused.owner.owner.audio.renderer.clock(), SampleTime::ZERO);
        let (control, audio) = refused
            .owner
            .arm_one_shot(refused.candidate, &mixed_profile())
            .expect("original share");
        assert_eq!(control.control.epoch, audio.audio.renderer.epoch());
    }
}

#[test]
fn private_one_shot_arm_checks_actual_split_and_shifted_compiled_density() {
    for compiled_first in [true, false] {
        let (prepared, mut candidate) =
            one_shot_with_boundary_on_at(compiled_first, SampleTime::new(32));
        assert_eq!(candidate.events[0].envelope().time(), SampleTime::new(32));
        assert_eq!(
            candidate
                .effective_timing(SampleTime::new(64))
                .expect("one delayed boundary")
                .shift(),
            FrameCount::new(32)
        );
        let first = candidate.events[0];
        let suffix = candidate.events[candidate.restoration_count.get() as usize];
        candidate.events[0] = suffix;
        let refused = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect_err("non-restoration in Session prefix");
        assert_eq!(
            refused.reason,
            MixedOneShotArmError::Admission(MixedOneShotAdmissionError::CandidateShape {
                event_index: 0,
            })
        );
        let mut candidate = refused.candidate;
        candidate.events[0] = first;
        let share = mixed_profile()
            .limits()
            .events()
            .shares()
            .compiled_event_share();
        let additions = share.get() as usize;
        candidate
            .events
            .extend(std::iter::repeat_n(suffix, additions));
        candidate.event_count =
            EventCount::measured(u32::try_from(candidate.events.len()).expect("test list count"));
        let refused = refused
            .owner
            .arm_one_shot(candidate, &mixed_profile())
            .expect_err("compiled boundary over share");
        assert_eq!(
            refused.reason,
            MixedOneShotArmError::Admission(MixedOneShotAdmissionError::CompiledShare {
                at: SampleTime::new(64),
                needed: share
                    .checked_add(EventCount::measured(1))
                    .expect("one extra"),
                available: share,
            })
        );
        assert_eq!(refused.owner.owner.audio.renderer.clock(), SampleTime::ZERO);
    }
}

#[test]
fn private_one_shot_arm_accepts_a_valid_suffix_at_exact_compiled_share() {
    let share = mixed_profile()
        .limits()
        .events()
        .shares()
        .compiled_event_share();
    assert_eq!(share.get() % 2, 0);
    for compiled_first in [true, false] {
        let (prepared, candidate) = one_shot_at_compiled_share(compiled_first);
        assert_eq!(
            candidate.events.len() - candidate.restoration_count.get() as usize,
            share.get() as usize
        );
        assert_eq!(
            candidate
                .effective_timing(SampleTime::new(64))
                .expect("shifted quantum")
                .shift(),
            FrameCount::new(32)
        );
        let (control, audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("exact admitted Compiled share");
        assert_eq!(control.control.epoch, audio.audio.renderer.epoch());
    }
}

#[test]
fn private_one_shot_arm_groups_suffix_density_after_effective_shift() {
    let share = mixed_profile()
        .limits()
        .events()
        .shares()
        .compiled_event_share();
    for compiled_first in [true, false] {
        let (prepared, mut candidate) = one_shot_at_compiled_share(compiled_first);
        let last = *candidate.events.last().expect("nonempty suffix");
        assert_eq!(last.envelope().time(), SampleTime::new(79));
        candidate.events.push(last);
        candidate.event_count =
            EventCount::measured(u32::try_from(candidate.events.len()).expect("test list count"));
        let suffix = &candidate.events[candidate.restoration_count.get() as usize..];
        let before = suffix
            .iter()
            .filter(|event| event.envelope().time().quantum_index() == 0)
            .count();
        let after = suffix.len() - before;
        assert_eq!(before + after, share.get() as usize + 1);
        assert!(before < share.get() as usize);
        assert!(after < share.get() as usize);
        let refused = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect_err("shifted boundary quantum exceeds Compiled share");
        assert_eq!(
            refused.reason,
            MixedOneShotArmError::Admission(MixedOneShotAdmissionError::CompiledShare {
                at: SampleTime::new(111),
                needed: share
                    .checked_add(EventCount::measured(1))
                    .expect("one extra"),
                available: share,
            })
        );
    }
}

#[test]
fn private_one_shot_arm_rejects_reordered_effective_suffix() {
    let (prepared, mut candidate) = one_shot_with_boundary_on(true);
    let index = candidate.restoration_count.get() as usize;
    let event = candidate.events[index];
    candidate.events[index] = TimedEvent::new(
        EventEnvelope::new(
            event.envelope().epoch(),
            SampleTime::new(128),
            event.envelope().source(),
        ),
        event.payload(),
    );
    candidate.events.push(event);
    candidate.event_count =
        EventCount::measured(u32::try_from(candidate.events.len()).expect("test list count"));
    let refused = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect_err("suffix time moved backwards");
    assert_eq!(
        refused.reason,
        MixedOneShotArmError::Admission(MixedOneShotAdmissionError::CandidateShape {
            event_index: index + 1,
        })
    );
}

#[test]
fn private_one_shot_arm_refuses_restoration_spending_compiled_credit() {
    let (prepared, mut candidate) = one_shot_with_boundary_on(false);
    let suffix = candidate.restoration_count.get() as usize;
    let original = candidate.events[suffix];
    candidate.events[suffix] = candidate.events[0];
    let refused = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect_err("scoped restoration in Compiled suffix");
    assert_eq!(
        refused.reason,
        MixedOneShotArmError::Admission(MixedOneShotAdmissionError::CandidateShape {
            event_index: suffix,
        })
    );
    let mut candidate = refused.candidate;
    candidate.events[suffix] = original;
    let (control, audio) = refused
        .owner
        .arm_one_shot(candidate, &mixed_profile())
        .expect("corrected payload split");
    assert_eq!(control.control.epoch, audio.audio.renderer.epoch());
}

#[test]
fn private_effective_timing_refuses_suffix_overflow_after_restoration_fits() {
    let prepared = bound_with_events(true, |note| {
        vec![
            PlanEvent::new(
                PlanPosition::new(10),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(200),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(60),
                },
            ),
        ]
    });
    let last_boundary = u64::MAX - (u64::MAX % u64::from(crate::time::QUANTUM_FRAMES));
    let requested = SampleTime::new(last_boundary - 128);
    let history = prepared
        .prepare_history(requested, PlanPosition::new(10))
        .expect("prefix");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let stamped = prepared.stamp_suffix(suffix).expect("private stamp");
    let shift = FrameCount::new(128);
    let restoration_count = stamped
        .restoration_count()
        .as_usize()
        .expect("represented restoration count");
    assert!(restoration_count > 0);
    for event in &stamped.events[..restoration_count] {
        assert!(matches!(event.payload(), EventPayload::ScopedRestore(_)));
        assert_eq!(
            event.envelope().time().checked_add(shift),
            Ok(SampleTime::new(last_boundary))
        );
    }
    assert!(
        stamped
            .events
            .windows(2)
            .all(|pair| { pair[0].envelope().time() <= pair[1].envelope().time() })
    );
    let overflowing_index = stamped.events.len() - 1;
    assert!(overflowing_index > restoration_count);
    assert_eq!(
        stamped.effective_timing(SampleTime::new(last_boundary)),
        Err(MixedEffectiveTimeError::EventTimeUnrepresentable {
            event_index: overflowing_index,
            time: stamped.events[overflowing_index].envelope().time(),
            shift,
        })
    );
    assert_eq!(stamped.anchor().time(), requested);
}

#[test]
fn private_event_order_check_refuses_a_reversed_combined_list() {
    let prepared = bound(true);
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let stamped = prepared.stamp_suffix(suffix).expect("ordered stamp");
    let mut events = stamped.events.clone();
    let last = events.len() - 1;
    events.swap(0, last);
    assert_eq!(
        check_mixed_event_order(&events),
        Err(MixedStampPrepareError::EventOrder { event_index: 1 })
    );
}

#[test]
fn private_effective_timing_accepts_latest_event_at_timeline_end() {
    let prepared = bound_with_events(false, |note| {
        vec![
            PlanEvent::new(
                PlanPosition::new(10),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(73),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(60),
                },
            ),
        ]
    });
    let last_boundary = u64::MAX - (u64::MAX % u64::from(crate::time::QUANTUM_FRAMES));
    let requested = SampleTime::new(last_boundary - 128);
    let history = prepared
        .prepare_history(requested, PlanPosition::new(10))
        .expect("prefix");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let stamped = prepared.stamp_suffix(suffix).expect("private stamp");
    let timing = stamped
        .effective_timing(SampleTime::new(last_boundary))
        .expect("latest event reaches the final frame exactly");
    assert_eq!(timing.shift(), FrameCount::new(128));
    let shifted = stamped
        .effective_events(timing.effective())
        .expect("checked view");
    assert_eq!(
        shifted
            .get(shifted.len() - 1)
            .expect("last read")
            .expect("last event")
            .envelope()
            .time(),
        SampleTime::new(u64::MAX)
    );
}

#[test]
fn private_effective_list_releases_old_compiled_notes_before_a_destination_on() {
    for compiled_first in [true, false] {
        let mut prepared = bound_with_events(compiled_first, |note| {
            vec![
                PlanEvent::new(
                    PlanPosition::ZERO,
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(60),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(256),
                    CompiledPayload::NoteOn {
                        slot: note,
                        key: key(72),
                        velocity: NoteVelocity::FULL,
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(384),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(60),
                    },
                ),
                PlanEvent::new(
                    PlanPosition::new(448),
                    CompiledPayload::NoteOff {
                        slot: note,
                        key: key(72),
                    },
                ),
            ]
        });
        let note = prepared
            .owner
            .control
            .plan
            .resolve_note(ENVELOPE)
            .expect("note");
        let history = prepared
            .prepare_history(SampleTime::new(64), PlanPosition::new(256))
            .expect("old note is open at destination");
        let suffix = prepared.prepare_suffix(history).expect("bound suffix");
        assert_eq!(suffix.omitted_release_count().get(), 1);
        let stamped = prepared.stamp_suffix(suffix).expect("private candidate");
        let old_on = prepared.events[0];
        let EventPayload::Note {
            identity: old_identity,
            edge: NoteEdge::On { .. },
        } = old_on.payload()
        else {
            panic!("initial compiled onset");
        };
        let epoch = prepared.epoch();
        let table = prepared.table_id();
        let plan = std::sync::Arc::clone(&prepared.owner.control.plan);
        let partition = std::sync::Arc::clone(&prepared.owner.control.partition);
        let anchor = prepared.owner.control.anchor;
        let mut live_only =
            PreparedRenderer::prepare(std::sync::Arc::clone(&plan), anchor, epoch, table)
                .expect("live renderer");
        let mut compiled_only =
            PreparedRenderer::prepare(std::sync::Arc::clone(&plan), anchor, epoch, table)
                .expect("compiled renderer");
        let mut released_only =
            PreparedRenderer::prepare(plan, anchor, epoch, table).expect("released reference");
        assert!(live_only.bind_mixed_partition(std::sync::Arc::clone(&partition)));
        assert!(compiled_only.bind_mixed_partition(std::sync::Arc::clone(&partition)));
        assert!(released_only.bind_mixed_partition(partition));
        let audio = &mut prepared.owner.audio;
        let live_identity = audio
            .minter
            .mint_keyed(note, key(48))
            .expect("live range credit");
        let live_on = TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::ZERO, TestOrigin::Simulated),
            EventPayload::Note {
                identity: live_identity,
                edge: NoteEdge::On {
                    slot: note,
                    key: key(48),
                    velocity: NoteVelocity::FULL,
                },
            },
        );
        for renderer in [
            &mut audio.renderer,
            &mut live_only,
            &mut compiled_only,
            &mut released_only,
        ] {
            let _ = render_quantum(renderer, &[]);
        }
        let _ = render_quantum(&mut audio.renderer, &[old_on, live_on]);
        let _ = render_quantum(&mut live_only, &[live_on]);
        let _ = render_quantum(&mut compiled_only, &[old_on]);
        let _ = render_quantum(&mut released_only, &[old_on]);
        for renderer in [
            &mut audio.renderer,
            &mut live_only,
            &mut compiled_only,
            &mut released_only,
        ] {
            let _ = render_quantum(renderer, &[]);
        }
        let effective = audio.renderer.clock();
        assert_eq!(effective, SampleTime::new(128));
        let shifted = stamped
            .effective_events(effective)
            .expect("all candidate events fit the effective boundary");
        assert_eq!(
            shifted.len(),
            stamped.event_count().as_usize().expect("event count")
        );
        let events = shifted
            .iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("checked events");
        assert_eq!(shifted.get(0).expect("first read"), events.first().copied());
        assert_eq!(shifted.get(events.len()).expect("end read"), None);
        assert_eq!(stamped.events[0].envelope().time(), SampleTime::new(64));
        let restoration_count = stamped
            .restoration_count()
            .as_usize()
            .expect("restoration count");
        assert!(events[..restoration_count].iter().all(|event| {
            matches!(event.payload(), EventPayload::ScopedRestore(_))
                && event.envelope().time() == effective
        }));
        let EventPayload::Note {
            identity: new_identity,
            edge: NoteEdge::On { key: new_key, .. },
        } = events[restoration_count].payload()
        else {
            panic!("destination onset follows restoration");
        };
        assert_eq!(new_key, key(72));
        assert_eq!(events[restoration_count].envelope().time(), effective);
        assert_ne!(new_identity, old_identity);
        assert_eq!(events.len(), restoration_count + 2);
        assert_eq!(
            events.last().map(|event| event.envelope().time()),
            Some(SampleTime::new(320))
        );
        assert!(matches!(
            events.last().map(TimedEvent::payload),
            Some(EventPayload::Note {
                identity,
                edge: NoteEdge::Off,
            }) if identity == new_identity
        ));
        let boundary_events: Vec<_> = events
            .iter()
            .copied()
            .filter(|event| event.envelope().time() == effective)
            .collect();
        assert_eq!(boundary_events.len(), restoration_count + 1);
        let old_anchor = audio.renderer.anchor_for_test();
        assert_ne!(
            old_anchor.time_of(stamped.suffix.history.position),
            Some(effective),
            "the seek must change the musical mapping"
        );
        assert_eq!(
            audio.adopt_compiled_boundary(SampleTime::new(64), stamped.suffix.history.position),
            Err(crate::render::MixedBoundaryReleaseError::ClockMismatch {
                clock: effective,
                offered: SampleTime::new(64),
            })
        );
        assert_eq!(audio.renderer.anchor_for_test(), old_anchor);
        let ended_storage = std::mem::take(&mut audio.compiled_ended);
        assert_eq!(
            audio.adopt_compiled_boundary(effective, stamped.suffix.history.position),
            Err(crate::render::MixedBoundaryReleaseError::EndedStorage)
        );
        assert_eq!(audio.renderer.anchor_for_test(), old_anchor);
        audio.compiled_ended = ended_storage;
        let mut adopted = None;
        let allocations = crate::render_allocation::count_allocs(|| {
            adopted = Some(
                audio
                    .adopt_compiled_boundary(effective, stamped.suffix.history.position)
                    .expect("compiled-only boundary adoption"),
            );
        });
        assert_eq!(allocations, 0);
        let adopted = adopted.expect("boundary result");
        assert_eq!(adopted.released().get(), 1);
        assert_eq!(adopted.retired_anchor(), old_anchor);
        assert_eq!(
            audio.renderer.anchor_for_test(),
            StreamAnchor::new(effective, stamped.suffix.history.position)
        );
        assert_eq!(
            audio.compiled_ended[0].map(|entry| entry.index),
            Some(old_identity.index())
        );
        let mut compiled_ended = [None; 2];
        assert_eq!(
            compiled_only
                .release_mixed_compiled_boundary(
                    audio.partition.compiled_producer(),
                    &mut compiled_ended,
                )
                .expect("reference compiled release")
                .get(),
            1
        );
        let mut released_ended = [None; 2];
        assert_eq!(
            released_only
                .release_mixed_compiled_boundary(
                    audio.partition.compiled_producer(),
                    &mut released_ended,
                )
                .expect("reference release without new onset")
                .get(),
            1
        );
        let actual = render_quantum(&mut audio.renderer, &boundary_events);
        let live = render_quantum(&mut live_only, &[]);
        let new = render_quantum(&mut compiled_only, &boundary_events);
        let released = render_quantum(&mut released_only, &boundary_events[..restoration_count]);
        assert!(live.iter().any(|sample| *sample != 0.0));
        assert!(new.iter().any(|sample| *sample != 0.0));
        assert!(released.iter().all(|sample| *sample == 0.0));
        for (index, ((actual, live), new)) in actual.iter().zip(&live).zip(&new).enumerate() {
            assert!(
                (actual - (live + new)).abs() < 0.00001,
                "boundary frame {index}: mixed {actual}, live {live}, new {new}"
            );
        }
        let actual_next = render_quantum(&mut audio.renderer, &[]);
        let live_next = render_quantum(&mut live_only, &[]);
        let new_next = render_quantum(&mut compiled_only, &[]);
        let released_next = render_quantum(&mut released_only, &[]);
        assert!(released_next.iter().all(|sample| *sample == 0.0));
        assert!(new_next.iter().any(|sample| *sample != 0.0));
        for ((actual, live), new) in actual_next.iter().zip(&live_next).zip(&new_next) {
            assert!((actual - (live + new)).abs() < 0.00001);
        }
    }
}

#[test]
fn private_stamp_refuses_unrepresentable_suffix_time_and_invalid_selection() {
    let prepared = bound_with_events(true, |note| {
        vec![
            PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(10),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(72),
                    velocity: NoteVelocity::FULL,
                },
            ),
            PlanEvent::new(
                PlanPosition::new(20),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(60),
                },
            ),
            PlanEvent::new(
                PlanPosition::new(200),
                CompiledPayload::Bend {
                    slot: note,
                    key: key(72),
                    cents: Cents::new(25.0).expect("finite bend"),
                },
            ),
        ]
    });
    let original = prepared.outstanding[0];
    assert_eq!(
        prepared.owner.control.minter.resolve(original),
        Resolution::Live
    );
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let mut suffix = prepared.prepare_suffix(history).expect("suffix selection");
    suffix.included[0] = usize::MAX;
    assert_eq!(
        prepared
            .stamp_suffix(suffix)
            .expect_err("invalid private index"),
        MixedStampPrepareError::InvalidSelection {
            event_index: usize::MAX
        }
    );
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let mut suffix = prepared.prepare_suffix(history).expect("suffix selection");
    suffix.included.swap(0, 1);
    assert_eq!(
        prepared
            .stamp_suffix(suffix)
            .expect_err("reordered indices"),
        MixedStampPrepareError::InvalidSelection { event_index: 1 }
    );
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let mut suffix = prepared.prepare_suffix(history).expect("suffix selection");
    suffix.included[0] = 0;
    assert_eq!(
        prepared
            .stamp_suffix(suffix)
            .expect_err("prefix source selected"),
        MixedStampPrepareError::InvalidSelection { event_index: 0 }
    );

    let last_boundary = u64::MAX - (u64::MAX % u64::from(crate::time::QUANTUM_FRAMES));
    let history = prepared
        .prepare_history(SampleTime::new(last_boundary), PlanPosition::new(10))
        .expect("representable boundary");
    let suffix = prepared.prepare_suffix(history).expect("suffix selection");
    assert_eq!(
        prepared
            .stamp_suffix(suffix)
            .expect_err("placed bend exceeds engine time"),
        MixedStampPrepareError::Schedule(SchedulePrepareError::TimeUnrepresentable {
            event_index: 3,
            position: PlanPosition::new(200),
        })
    );
    assert_eq!(prepared.outstanding_count(), 1);
    assert_eq!(
        prepared.owner.control.minter.resolve(original),
        Resolution::Live
    );
}

#[test]
fn private_stamp_refuses_inconsistent_old_reservation_custody() {
    let make_prepared = || {
        bound_with_events(true, |note| {
            vec![PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(60),
                    velocity: NoteVelocity::FULL,
                },
            )]
        })
    };
    let mut missing = make_prepared();
    missing.outstanding.clear();
    let history = missing
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let suffix = missing.prepare_suffix(history).expect("empty suffix");
    assert_eq!(
        missing
            .stamp_suffix(suffix)
            .expect_err("unlisted live index"),
        MixedStampPrepareError::OldReservationsRemain {
            live: HeldNoteCount::measured(1),
        }
    );

    let mut duplicate = make_prepared();
    let identity = duplicate.outstanding[0];
    duplicate.outstanding.push(identity);
    let history = duplicate
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("prefix");
    let suffix = duplicate.prepare_suffix(history).expect("empty suffix");
    assert_eq!(
        duplicate
            .stamp_suffix(suffix)
            .expect_err("duplicate reservation"),
        MixedStampPrepareError::StaleReservation {
            identity,
            resolution: Resolution::Orphan(crate::identity::OrphanCause::FreeIndex),
        }
    );
}
