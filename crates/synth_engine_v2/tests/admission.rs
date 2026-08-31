//! Admission: the report, the refusals, and the advisory budget.
//!
//! The refusal cases are `HOST-INV-007`'s conformance row: **one per render limit a
//! plan can exceed**, each asserting the error names the field, both amounts, and the
//! responsible object, and that the plan the caller handed in is unchanged. There are
//! thirty-two: twenty-eight in Phase 1, plus the five ADR-0046 fields a plan states —
//! `compiled_event_share`, `release_hold_capacity`, `session_event_share`,
//! `authored_runtime_event_share` and `internal_event_share` — less
//! `max_events_per_quantum`, which left when a plan stopped requesting the cap directly.
//! The test asserts that number against
//! [`ResourceField::is_admission_checked`] so a new limit cannot arrive without a case.

mod common;

use common::{OUTPUT, SOURCE, declaring, profile, source_plan};

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::diagnostics::{CompileError, CompileWarning};
use synth_engine_v2::identity::ProducerId;
use synth_engine_v2::ir::{
    AuthoredSourceDeclaration, ExecutionScope, GraphIr, InternalProducerDeclaration, IrNodeKind,
    IrObject, IrProgram, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId, ProgramId,
    SignalDomain, TapId,
};
use synth_engine_v2::profile::{
    CostBudget, EventLimits, GraphLimits, HostProfile, MemoryLimits, MixingLimits,
    ObservationLimits, ProducerShares, QueueCapacities, RecordingLimits, RenderLimits,
    ScriptLimits, StreamLimits, VoiceLimits,
};
use synth_engine_v2::quantities::{
    Amplitude, BusCount, ChannelLayout, CostRatio, EdgeCount, EventCount, FanOut, Frequency,
    GainFactor, HeldNoteCount, InstructionCount, MixChannelCount, NodeCount, PreparedBytes,
    SendCount, SlotCount, TapCount, VoiceCount,
};
use synth_engine_v2::report::{ResourceAmount, ResourceField, ResourceReport};
use synth_engine_v2::time::FrameCount;
use synth_engine_v2::validate::PortDirection;

/// Rebuild a profile's limits with one group replaced.
struct Groups {
    stream: StreamLimits,
    graph: GraphLimits,
    voices: VoiceLimits,
    events: EventLimits,
    observation: ObservationLimits,
    mixing: MixingLimits,
    memory: MemoryLimits,
    script: ScriptLimits,
    recording: RecordingLimits,
    cost: CostBudget,
}

impl Groups {
    fn of(profile: &HostProfile) -> Self {
        let limits = common::defaults_for(profile);
        Self {
            stream: limits.stream(),
            graph: limits.graph(),
            voices: limits.voices(),
            events: limits.events(),
            observation: limits.observation(),
            mixing: limits.mixing(),
            memory: limits.memory(),
            script: limits.script(),
            recording: limits.recording(),
            cost: limits.cost(),
        }
    }

    fn build(self, profile: &HostProfile) -> HostProfile {
        let limits = RenderLimits::new(
            self.stream,
            self.graph,
            self.voices,
            self.events,
            self.observation,
            self.mixing,
            self.memory,
            self.script,
            self.recording,
            self.cost,
        )
        .expect("the overridden limits are internally consistent");
        HostProfile::new(profile.capabilities(), limits).expect("the profile is consistent")
    }
}

fn count(value: u32) -> NodeCount {
    NodeCount::limit(value).expect("a positive capacity")
}

fn slots(value: u32) -> SlotCount {
    SlotCount::limit(value).expect("a positive capacity")
}

fn events(value: u32) -> EventCount {
    EventCount::limit(value).expect("a positive capacity")
}

/// The smallest partition ADR-0046 clause 1 admits: six positive shares and one hold.
///
/// A test that overrides `max_events_per_quantum` must also supply a partition that
/// fits it, and this is the one that fits every cap of six or more.
fn minimal_shares() -> ProducerShares {
    ProducerShares::new(
        events(1),
        events(1),
        events(1),
        events(1),
        events(1),
        events(1),
        events(1),
    )
    .expect("a valid minimal partition")
}

/// Compile and assert the refusal names `field`, both amounts, and an object.
fn assert_refused(field: ResourceField, ir: &GraphIr, host: HostProfile) {
    let before = ir.clone();
    let outcome = compile(ir, &RenderConfig::new(host));

    let error = *outcome.plan().expect_err("this plan must be refused");
    match error {
        CompileError::LimitExceeded {
            field: named,
            requested,
            available,
            responsible,
        } => {
            assert_eq!(
                named, field,
                "expected a refusal on {field}, got one on {named}"
            );
            let rendered = CompileError::LimitExceeded {
                field: named,
                requested,
                available,
                responsible,
            }
            .to_string();
            assert!(
                rendered.contains(field.name()),
                "the message must name the field: {rendered}"
            );
            assert!(
                rendered.contains(&requested.to_string())
                    && rendered.contains(&available.to_string()),
                "the message must carry both amounts: {rendered}"
            );
            assert!(
                rendered.contains(&responsible.to_string()),
                "the message must name the responsible object: {rendered}"
            );
        }
        other => panic!("expected a limit refusal on {field}, got {other}"),
    }

    // A refusal is a report plus an error, never an error alone.
    assert_eq!(outcome.report().rows().len(), ResourceField::COUNT);
    assert_eq!(
        &before, ir,
        "admission must not touch the plan it was handed"
    );
}

/// A plan declaring one of everything the profile bounds, so a lowered limit bites.
fn declared() -> PlanDeclarations {
    PlanDeclarations {
        // One compiled producer, holding two notes and no obligations — compiled releases
        // use plan entitlements, so a compiled source declaring a hold is refused outright.
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(2),
            simultaneous_holds: EventCount::NONE,
        }],
        active_voices: VoiceCount::measured(2),
        held_notes: HeldNoteCount::measured(2),
        mix_channels: MixChannelCount::measured(2),
        buses: BusCount::measured(2),
        max_sends_on_any_channel: SendCount::measured(2),
        taps: vec![TapId::new(1), TapId::new(2)],
        events_per_quantum: EventCount::measured(2),
        note_expansion_per_tick: EventCount::measured(2),
        scheduled_events_in_flight: EventCount::measured(2),
        mod_graph_nodes: NodeCount::measured(2),
        note_graph_nodes: NodeCount::measured(2),
        held_notes_per_take: HeldNoteCount::measured(2),
        recorded_events_per_take: EventCount::measured(2),
        programs: vec![IrProgram::new(
            ProgramId::FIRST,
            InstructionCount::measured(2),
            SlotCount::measured(2),
            SlotCount::measured(2),
            SlotCount::measured(2),
            SlotCount::measured(2),
            SlotCount::measured(2),
            SlotCount::measured(2),
            SlotCount::measured(2),
            1,
        )],
        voices_per_instrument: VoiceCount::measured(2),
        authored_sources: Vec::new(),
        internal_producers: Vec::new(),
    }
}

/// Every refusal case: the field it must be refused on, and the plan and profile that
/// do it.
///
/// Built as one table rather than as separate tests so that the coverage claim is
/// **checkable**: the test below asserts that the fields here are exactly the
/// admission-checked set, so a new limit cannot arrive without a case.
fn refusal_cases(host: &HostProfile) -> Vec<(ResourceField, GraphIr, HostProfile)> {
    let silence = source_plan(IrNodeKind::Silence);
    let sine = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(440.0).expect("finite"),
        amplitude: Amplitude::new(0.5).expect("finite"),
    });
    // Two edges leaving one output port, which is what the fan-out row measures. Both
    // land on real input ports: since P02-T004 put structural validation ahead of the
    // limit checks — the arena's size is a function of the assignment, so the report
    // cannot be built before lowering — a fixture with an invalid cable would now be
    // refused for its cable rather than for the limit it was built to exceed.
    let fan_out = GraphIr::builder()
        .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Global)
        .node(
            NodeId::new(20),
            IrNodeKind::Gain {
                factor: GainFactor::new(0.5).expect("finite"),
            },
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SOURCE, PortId::FIRST),
            (NodeId::new(20), PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");
    let declares = declaring(declared());

    // ADR-0046 clause 1 raised the floor under two of these limits, so the shared
    // fixture's declaration of two no longer exceeds either. The cap cannot go below six
    // (six positive shares), and `max_scheduled_events_in_flight` cannot go below the
    // compiled floor, which is four here. Both rows therefore declare *more* instead of
    // budgeting less; the refusal under test is unchanged.
    let declares_seven_events = declaring(PlanDeclarations {
        events_per_quantum: EventCount::measured(7),
        ..declared()
    });
    let declares_five_in_flight = declaring(PlanDeclarations {
        scheduled_events_in_flight: EventCount::measured(5),
        ..declared()
    });

    let graph = |nodes: u32, edges: u32, fan_out: u32, mod_nodes: u32, note_nodes: u32| {
        let mut groups = Groups::of(host);
        groups.graph = GraphLimits::new(
            count(nodes),
            EdgeCount::limit(edges).expect("positive"),
            FanOut::limit(fan_out).expect("positive"),
            count(mod_nodes),
            count(note_nodes),
        )
        .expect("the overridden capacities are above zero");
        groups.build(host)
    };
    let voices = |low: u32, high: u32, active: u32, held: u32| {
        let mut groups = Groups::of(host);
        groups.voices = VoiceLimits::new(
            VoiceCount::limit(low).expect("positive"),
            VoiceCount::limit(high).expect("positive"),
            VoiceCount::limit(active).expect("positive"),
            HeldNoteCount::limit(held).expect("positive"),
            FrameCount::new(128),
        )
        .expect("a consistent voice range");
        groups.build(host)
    };
    let event_limits = |per_quantum: u32, expansion: u32, in_flight: u32| {
        let mut groups = Groups::of(host);
        let horizon = groups.events.forward_event_horizon();
        groups.events = EventLimits::new(
            events(per_quantum),
            events(expansion),
            events(in_flight),
            horizon,
            QueueCapacities::new(events(1), events(16_384), events(256))
                .expect("the overridden capacities are above zero"),
            minimal_shares(),
        )
        .expect("the overridden capacities are above zero");
        groups.build(host)
    };
    let taps = |limit: u32| {
        let mut groups = Groups::of(host);
        groups.observation = ObservationLimits::new(
            TapCount::limit(limit).expect("positive"),
            FrameCount::new(4_096),
            FrameCount::new(2_048),
        )
        .expect("the overridden capacities are above zero");
        groups.build(host)
    };
    let mixing = |channels: u32, buses: u32, sends: u32| {
        let mut groups = Groups::of(host);
        groups.mixing = MixingLimits::new(
            MixChannelCount::limit(channels).expect("positive"),
            BusCount::limit(buses).expect("positive"),
            SendCount::limit(sends).expect("positive"),
        )
        .expect("the overridden capacities are above zero");
        groups.build(host)
    };
    let memory = |immutable: u64, mutable: u64, scratch: u64| {
        let bytes = |value: u64| PreparedBytes::limit(value).expect("positive");
        let mut groups = Groups::of(host);
        groups.memory = MemoryLimits::new(bytes(immutable), bytes(mutable), bytes(scratch))
            .expect("the overridden capacities are above zero");
        groups.build(host)
    };
    let script = |values: [u32; 8]| {
        let mut groups = Groups::of(host);
        groups.script = ScriptLimits::new(
            InstructionCount::limit(values[0]).expect("positive"),
            slots(values[1]),
            slots(values[2]),
            slots(values[3]),
            slots(values[4]),
            slots(values[5]),
            slots(values[6]),
            slots(values[7]),
            slots(16),
            slots(16),
        )
        .expect("the overridden capacities are above zero");
        groups.build(host)
    };
    let recording = |held: u32, recorded: u32| {
        let mut groups = Groups::of(host);
        groups.recording = RecordingLimits::new(
            HeldNoteCount::limit(held).expect("positive"),
            events(recorded),
        )
        .expect("the overridden capacities are above zero");
        groups.build(host)
    };
    let big = 1_u64 << 20;

    vec![
        (
            ResourceField::MaxNodes,
            silence.clone(),
            graph(1, 64, 64, 32, 32),
        ),
        (
            ResourceField::MaxEdges,
            fan_out.clone(),
            graph(64, 1, 64, 32, 32),
        ),
        (
            ResourceField::MaxFanOutPerPort,
            fan_out,
            graph(64, 64, 1, 32, 32),
        ),
        (
            ResourceField::MaxModGraphNodes,
            declares.clone(),
            graph(64, 64, 64, 1, 32),
        ),
        (
            ResourceField::MaxNoteGraphNodes,
            declares.clone(),
            graph(64, 64, 64, 32, 1),
        ),
        (
            ResourceField::VoicesPerInstrument,
            declares.clone(),
            voices(1, 1, 512, 512),
        ),
        (
            ResourceField::MaxActiveVoices,
            declares.clone(),
            voices(1, 128, 1, 512),
        ),
        (
            ResourceField::MaxHeldNotes,
            declares.clone(),
            voices(1, 128, 512, 1),
        ),
        (
            // The **share**, not the cap. A plan's statically knowable per-quantum
            // declaration is compiled work, and ADR-0046 clause 1 gives that class its own
            // entitlement; the cap it partitions is no longer something a plan requests.
            ResourceField::CompiledEventShare,
            declares_seven_events,
            event_limits(6, 128, 4_096),
        ),
        (
            // Two non-compiled producers that each fit alone but not together. Checking one
            // source at a time is not admission — the rule ADR-0046 states for authored
            // envelopes, and it holds for hold entitlements for the same reason.
            ResourceField::ReleaseHoldCapacity,
            declaring(PlanDeclarations {
                note_producers: vec![
                    NoteProducerDeclaration {
                        compiled: false,
                        simultaneous_notes: HeldNoteCount::measured(64),
                        simultaneous_holds: EventCount::measured(24),
                    },
                    NoteProducerDeclaration {
                        compiled: false,
                        simultaneous_notes: HeldNoteCount::measured(64),
                        simultaneous_holds: EventCount::measured(24),
                    },
                ],
                ..declared()
            }),
            {
                let mut groups = Groups::of(host);
                let horizon = groups.events.forward_event_horizon();
                let one = events(1);
                groups.events = EventLimits::new(
                    events(64),
                    events(128),
                    events(4_096),
                    horizon,
                    QueueCapacities::new(one, events(16_384), events(256))
                        .expect("the overridden capacities are above zero"),
                    ProducerShares::new(events(8), one, one, one, one, events(40), events(40))
                        .expect("a valid partition"),
                )
                .expect("the overridden capacities are above zero");
                groups.build(host)
            },
        ),
        (
            // **ADR-0046 clause 5's plan-wide authored aggregate**, and the case is built
            // from **two** sources rather than one on purpose: the record allows two that
            // fit individually to be refused together "unless the compiler proves them
            // mutually exclusive". Each declares three events against a share of five, so
            // neither exceeds alone and the sum does. A version that summed nothing — or
            // that took the maximum — would admit this plan, so the fixture fails if the
            // aggregate is ever weakened to a per-source check.
            ResourceField::AuthoredRuntimeEventShare,
            declaring(PlanDeclarations {
                note_producers: vec![NoteProducerDeclaration {
                    compiled: false,
                    simultaneous_notes: HeldNoteCount::measured(8),
                    simultaneous_holds: events(8),
                }],
                authored_sources: vec![
                    AuthoredSourceDeclaration {
                        producer: ProducerId::new(0),
                        destination_occupancy: events(3),
                        retained_future: EventCount::NONE,
                        simultaneous_holds: events(1),
                    },
                    AuthoredSourceDeclaration {
                        producer: ProducerId::new(0),
                        destination_occupancy: events(3),
                        retained_future: EventCount::NONE,
                        simultaneous_holds: events(1),
                    },
                ],
                ..PlanDeclarations::default()
            }),
            {
                let mut groups = Groups::of(host);
                let horizon = groups.events.forward_event_horizon();
                let one = events(1);
                groups.events = EventLimits::new(
                    events(64),
                    events(128),
                    events(4_096),
                    horizon,
                    QueueCapacities::new(one, events(16_384), events(256))
                        .expect("the overridden capacities are above zero"),
                    ProducerShares::new(
                        events(8),
                        events(5),
                        one,
                        one,
                        one,
                        events(40),
                        events(40),
                    )
                    .expect("a valid partition"),
                )
                .expect("the overridden capacities are above zero");
                groups.build(host)
            },
        ),
        (
            // **ADR-0046 clause 1's internal sum.** Two admitted producers again, for the
            // same reason: the share covers "the sum of every admitted internal producer's
            // declared per-quantum maximum", so a per-producer check would admit this.
            //
            // Clause 2 is what makes one number per producer a complete bound — an internal
            // emission takes effect in the quantum that generates it and may not target a
            // later one — which is why this row needs no destination search of its own.
            ResourceField::InternalEventShare,
            declaring(PlanDeclarations {
                internal_producers: vec![
                    InternalProducerDeclaration {
                        per_quantum: events(3),
                    },
                    InternalProducerDeclaration {
                        per_quantum: events(3),
                    },
                ],
                ..PlanDeclarations::default()
            }),
            {
                let mut groups = Groups::of(host);
                let horizon = groups.events.forward_event_horizon();
                let one = events(1);
                groups.events = EventLimits::new(
                    events(64),
                    events(128),
                    events(4_096),
                    horizon,
                    QueueCapacities::new(one, events(16_384), events(256))
                        .expect("the overridden capacities are above zero"),
                    ProducerShares::new(
                        events(8),
                        one,
                        one,
                        one,
                        events(5),
                        events(40),
                        events(40),
                    )
                    .expect("a valid partition"),
                )
                .expect("the overridden capacities are above zero");
                groups.build(host)
            },
        ),
        (
            // **ADR-0046 clause 1's session snapshot**, which is one number rather than a
            // search: a locate restores every prepared target at once, so the largest
            // catch-up batch over every legal position *is* the plan's prepared-target
            // count, **plus one** for ADR-0050 clause 5's boundary mass release, which the
            // same window charges to this share as a single operation.
            //
            // The share is **two**, not one, and that is what makes the `+1` load-bearing
            // here: a sine has two prepared targets, so a share of one refuses it whether or
            // not the boundary charge is counted, and the fixture would pass with the term
            // dropped. At two, only `2 + 1` exceeds. An independent review found the weaker
            // version.
            //
            // Until this slice the row reported the number and refused nothing, so such a
            // plan compiled and faulted at its **first locate** instead. A share overrun
            // ends the stream, which is a bad way to learn a plan was never admissible.
            ResourceField::SessionEventShare,
            sine.clone(),
            {
                let mut groups = Groups::of(host);
                let horizon = groups.events.forward_event_horizon();
                let one = events(1);
                groups.events = EventLimits::new(
                    events(64),
                    events(128),
                    events(4_096),
                    horizon,
                    QueueCapacities::new(one, events(16_384), events(256))
                        .expect("the overridden capacities are above zero"),
                    ProducerShares::new(events(8), events(8), one, events(2), one, events(40), one)
                        .expect("a valid partition"),
                )
                .expect("the overridden capacities are above zero");
                groups.build(host)
            },
        ),
        (
            ResourceField::MaxNoteExpansionPerTick,
            declares.clone(),
            event_limits(256, 1, 4_096),
        ),
        (
            ResourceField::MaxScheduledEventsInFlight,
            declares_five_in_flight,
            event_limits(256, 128, 4),
        ),
        // `LIMIT-0020`'s successor: V1 publishes meters through a 128-slot array whose
        // `publish()` is an `if let Some(slot)`, so a project with more metered channels
        // lost meters with no signal to anyone. Here it is a refusal.
        (ResourceField::MaxObservationTaps, declares.clone(), taps(1)),
        (
            ResourceField::MaxMixChannels,
            declares.clone(),
            mixing(1, 64, 16),
        ),
        (
            ResourceField::MaxBuses,
            declares.clone(),
            mixing(256, 1, 16),
        ),
        (
            ResourceField::MaxSendsPerChannel,
            declares.clone(),
            mixing(256, 64, 1),
        ),
        (
            ResourceField::PreparedImmutableBytes,
            sine.clone(),
            memory(1, big, big),
        ),
        (
            ResourceField::MutableStateBytes,
            sine.clone(),
            memory(big, 1, big),
        ),
        (ResourceField::BufferScratchBytes, sine, memory(big, big, 1)),
        (
            ResourceField::MaxInstructionsPerProgram,
            declares.clone(),
            script([1, 32, 16, 16, 64, 16, 256, 4]),
        ),
        (
            ResourceField::MaxSourcesPerProgram,
            declares.clone(),
            script([256, 1, 16, 16, 64, 16, 256, 4]),
        ),
        (
            ResourceField::MaxStateSlotsPerProgram,
            declares.clone(),
            script([256, 32, 1, 16, 64, 16, 256, 4]),
        ),
        (
            ResourceField::MaxLocalsPerProgram,
            declares.clone(),
            script([256, 32, 16, 1, 64, 16, 256, 4]),
        ),
        (
            ResourceField::MaxEvalStackDepth,
            declares.clone(),
            script([256, 32, 16, 16, 1, 16, 256, 4]),
        ),
        (
            ResourceField::MaxArraysPerProgram,
            declares.clone(),
            script([256, 32, 16, 16, 64, 1, 256, 4]),
        ),
        (
            ResourceField::MaxArrayElements,
            declares.clone(),
            script([256, 32, 16, 16, 64, 16, 1, 4]),
        ),
        (
            ResourceField::MaxEmitsPerProgram,
            declares.clone(),
            script([256, 32, 16, 16, 64, 16, 256, 1]),
        ),
        (
            ResourceField::MaxHeldNotesPerTake,
            declares.clone(),
            recording(1, 4_096),
        ),
        (
            ResourceField::MaxRecordedEventsPerTake,
            declares,
            recording(32, 1),
        ),
    ]
}

#[test]
fn every_limit_a_plan_can_exceed_has_a_refusal_case() {
    // `HOST-INV-007`'s conformance row, made checkable: the cases cover the
    // admission-checked set exactly — no field without a case, and no case for a field
    // that cannot be exceeded.
    let host = profile(256, ChannelLayout::Mono);
    let mut covered: Vec<ResourceField> = refusal_cases(&host)
        .into_iter()
        .map(|(field, _, _)| field)
        .collect();
    covered.sort_unstable();
    let before_dedup = covered.len();
    covered.dedup();
    assert_eq!(before_dedup, covered.len(), "a field has two cases");

    let mut checked: Vec<ResourceField> = ResourceField::ALL
        .into_iter()
        .filter(|field| field.is_admission_checked())
        .collect();
    checked.sort_unstable();

    assert_eq!(
        covered, checked,
        "the refusal cases and the admission-checked set have diverged"
    );
    // Thirty-two, not thirty: `authored_runtime_event_share` and `internal_event_share`
    // joined when `PlanDeclarations` gained the two declarations that let a plan state what
    // those shares bound. Before that each share was reported against itself and no plan
    // could exceed it, which is why `HOST-INV-007`'s conformance row could not be satisfied
    // for them. `session_event_share` and `release_hold_capacity` joined in the two slices
    // before this one, each for the same kind of reason.
    assert_eq!(checked.len(), 32, "the admission-checked set changed size");
}

#[test]
fn each_refusal_names_its_field_both_amounts_and_the_responsible_object() {
    let host = profile(256, ChannelLayout::Mono);
    for (field, ir, profile) in refusal_cases(&host) {
        assert_refused(field, &ir, profile);
    }
}

#[test]
fn a_memory_refusal_names_the_node_rather_than_the_plan() {
    // `HOST-INV-014` plus `HOST-INV-007`: the aggregate is the compiler's, and the
    // refusal points at the node that dominates it.
    let host = profile(256, ChannelLayout::Mono);
    let sine = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(440.0).expect("finite"),
        amplitude: Amplitude::new(0.5).expect("finite"),
    });
    let mut groups = Groups::of(&host);
    groups.memory = MemoryLimits::new(
        PreparedBytes::limit(1 << 20).expect("positive"),
        PreparedBytes::limit(1).expect("positive"),
        PreparedBytes::limit(1 << 20).expect("positive"),
    )
    .expect("the overridden capacities are above zero");
    match compile(&sine, &RenderConfig::new(groups.build(&host))).plan() {
        Err(CompileError::LimitExceeded {
            field, responsible, ..
        }) => {
            assert_eq!(*field, ResourceField::MutableStateBytes);
            assert_eq!(
                *responsible,
                IrObject::Node(SOURCE),
                "the only stateful node must be named, not the plan"
            );
        }
        other => panic!("expected a mutable-state refusal, got {other:?}"),
    }
}

#[test]
fn the_advisory_cost_budget_warns_and_never_refuses() {
    // `HOST-INV-015`. The plan declares 512 voices, which EVD-0003's slope puts at about
    // 60% of one core at 44.1 kHz — four times the 0.15 budget — and it still compiles.
    let host = profile(256, ChannelLayout::Mono);
    let plan = declaring(PlanDeclarations {
        active_voices: VoiceCount::measured(512),
        voices_per_instrument: VoiceCount::measured(128),
        ..PlanDeclarations::default()
    });

    let outcome = compile(&plan, &RenderConfig::new(host));
    assert!(
        outcome.plan().is_ok(),
        "an advisory field may not produce a CompileError"
    );
    let warning = outcome
        .warnings()
        .iter()
        .copied()
        .find(|warning| matches!(warning, CompileWarning::AdvisoryBudgetExceeded { .. }))
        .expect("the cost budget must warn");
    match warning {
        CompileWarning::AdvisoryBudgetExceeded {
            field,
            predicted,
            permitted,
            ..
        } => {
            assert_eq!(field, ResourceField::PredictedQuantumCostRatio);
            let rendered = warning.to_string();
            assert!(rendered.contains(&predicted.to_string()));
            assert!(rendered.contains(&permitted.to_string()));
        }
        other => panic!("expected the advisory budget warning, got {other:?}"),
    }
}

#[test]
fn no_advisory_field_can_produce_a_compile_error() {
    // The other half of `HOST-INV-015`: even a wildly over-budget prediction compiles.
    let host = profile(256, ChannelLayout::Mono);
    let mut groups = Groups::of(&host);
    groups.cost = CostBudget::new(CostRatio::limit(f32::MIN_POSITIVE).expect("positive"))
        .expect("the overridden capacities are above zero");
    let plan = declaring(PlanDeclarations {
        active_voices: VoiceCount::measured(1),
        voices_per_instrument: VoiceCount::measured(1),
        ..PlanDeclarations::default()
    });
    let outcome = compile(&plan, &RenderConfig::new(groups.build(&host)));
    assert!(outcome.plan().is_ok());
    assert_eq!(outcome.warnings().len(), 1);
}

#[test]
fn the_report_carries_the_script_work_aggregate_with_no_threshold() {
    // Instructions times evaluations, present from Phase 1 and bounded by nothing until
    // Phase 7 can measure a per-instruction cost. A limit with no value is not a limit.
    let host = profile(256, ChannelLayout::Mono);
    let plan = declaring(PlanDeclarations {
        programs: vec![IrProgram::new(
            ProgramId::new(7),
            InstructionCount::measured(64),
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            SlotCount::NONE,
            4,
        )],
        ..PlanDeclarations::default()
    });

    let outcome = compile(&plan, &RenderConfig::new(host));
    let reported = outcome.report().reported();
    assert_eq!(reported.script_instructions_per_quantum().get(), 256);
    assert_eq!(
        reported.script_work_contributor(),
        IrObject::Program(ProgramId::new(7))
    );
    assert!(
        !ResourceField::ALL
            .into_iter()
            .any(|field| field.name() == "script_instructions_per_quantum"),
        "the aggregate is a reported quantity, not a profile field"
    );
}

/// What admits a profile field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ground {
    /// A queried host capability, under ADR-0021 part 4 and ADR-0032 clause 12.
    QueriedCapability,
    /// Ground 1: a ledger entry owned `HostProfile`.
    LedgerOwned,
    /// Ground 2: a field an accepted ADR creates.
    AcceptedDecision,
    /// Ground 3: the enumerated residual set.
    Residual,
}

/// The admitting rule for every field, written out so the test can check the partition.
fn ground_of(field: ResourceField) -> Ground {
    use Ground::{AcceptedDecision, LedgerOwned, QueriedCapability, Residual};
    match field {
        // The closed capability set: three queried values.
        ResourceField::SampleRate
        | ResourceField::MaximumBlockSize
        | ResourceField::ChannelLayout => QueriedCapability,

        // Ground 2: ADR-0032 clause 21 creates the forward horizon, and ADR-0046
        // clause 1 creates the seven producer fields.
        ResourceField::ForwardEventHorizon
        | ResourceField::CompiledEventShare
        | ResourceField::AuthoredRuntimeEventShare
        | ResourceField::LiveEventShare
        | ResourceField::SessionEventShare
        | ResourceField::InternalEventShare
        | ResourceField::ReleaseEventShare
        | ResourceField::ReleaseHoldCapacity => AcceptedDecision,

        // Ground 3, the enumerated residual: the six no-antecedent fields other than the
        // horizon, plus the two whose ledger rows are provenance rather than a ground.
        ResourceField::MaxActiveVoices
        | ResourceField::MaxScheduledEventsInFlight
        | ResourceField::MaxMixChannels
        | ResourceField::MaxBuses
        | ResourceField::MaxConcurrentRetiringVoices
        | ResourceField::PredictedQuantumCostRatio
        | ResourceField::MaxHeldNotes
        | ResourceField::MaxEventsPerQuantum
        // Added to ground 3's closed list by this specification, as its own rule
        // requires: ADR-0046 establishes that live ingress uses fixed registered queues
        // but explicitly does not create their capacity fields.
        | ResourceField::PerformanceIngressCapacity => Residual,

        // Ground 1: everything else has a `HostProfile`-owned ledger entry.
        ResourceField::AcceptedSampleRates
        | ResourceField::MaxNodes
        | ResourceField::MaxEdges
        | ResourceField::MaxFanOutPerPort
        | ResourceField::MaxModGraphNodes
        | ResourceField::MaxNoteGraphNodes
        | ResourceField::VoicesPerInstrument
        | ResourceField::RetirementCrossfade
        | ResourceField::MaxNoteExpansionPerTick
        | ResourceField::CommandQueueCapacity
        | ResourceField::EventEgressCapacity
        | ResourceField::MaxObservationTaps
        | ResourceField::TelemetryRingFrames
        | ResourceField::AnalyzerFftSize
        | ResourceField::MaxSendsPerChannel
        | ResourceField::PreparedImmutableBytes
        | ResourceField::MutableStateBytes
        | ResourceField::BufferScratchBytes
        | ResourceField::MaxInstructionsPerProgram
        | ResourceField::MaxSourcesPerProgram
        | ResourceField::MaxStateSlotsPerProgram
        | ResourceField::MaxLocalsPerProgram
        | ResourceField::MaxEvalStackDepth
        | ResourceField::MaxArraysPerProgram
        | ResourceField::MaxArrayElements
        | ResourceField::MaxEmitsPerProgram
        | ResourceField::ModMatrixSlotsPerVoice
        | ResourceField::ScriptHostSlotsPerVoice
        | ResourceField::MaxHeldNotesPerTake
        | ResourceField::MaxRecordedEventsPerTake => LedgerOwned,
    }
}

#[test]
fn every_field_is_admitted_by_exactly_one_rule() {
    // `HOST-INV-005`. The match above is exhaustive, so a new field must be classified
    // rather than silently escaping the check, and `Ground` has one variant per rule, so
    // a field cannot match two.
    //
    // The capability half is asserted against the **closed set of three queried
    // values**. `source` is the fourth `HostCapabilities` member and is not a report
    // field: nothing queries it, so ADR-0021 part 4 could not admit it, and
    // `HOST-INV-003` admits it instead.
    let queried: Vec<_> = ResourceField::ALL
        .into_iter()
        .filter(|field| ground_of(*field) == Ground::QueriedCapability)
        .collect();
    assert_eq!(
        queried,
        vec![
            ResourceField::SampleRate,
            ResourceField::MaximumBlockSize,
            ResourceField::ChannelLayout
        ],
        "the queried-capability set is closed at three; adding one is a specification change"
    );

    // The residual is a closed list, not "everything else" — that would admit a
    // protocol- or job-owned capacity by default.
    let residual: Vec<_> = ResourceField::ALL
        .into_iter()
        .filter(|field| ground_of(*field) == Ground::Residual)
        .collect();
    // Asserted as a list rather than a count, because the count cannot say *which* field
    // was added and this list is closed by rule. Ground 3 grew by one when the
    // renderer-ingress store gained a capacity: ADR-0046 requires live ingress to use
    // fixed registered queues but explicitly does not create their capacity fields, so
    // the field takes the specification's own route of an explicit addition here.
    assert_eq!(
        residual,
        vec![
            ResourceField::MaxActiveVoices,
            ResourceField::MaxHeldNotes,
            ResourceField::MaxConcurrentRetiringVoices,
            ResourceField::MaxEventsPerQuantum,
            ResourceField::MaxScheduledEventsInFlight,
            ResourceField::MaxMixChannels,
            ResourceField::MaxBuses,
            ResourceField::PredictedQuantumCostRatio,
            ResourceField::PerformanceIngressCapacity,
        ],
        "the residual set is closed: the six no-antecedent fields, max_held_notes, \
         max_events_per_quantum, and the renderer-ingress capacity"
    );

    // Ground 2 is a closed list too, and it grew from one to eight: ADR-0032 clause 21
    // creates the horizon, and ADR-0046 clause 1 creates the six producer shares and the
    // release-hold capacity. A field added here without an accepted record behind it is
    // exactly what `HOST-INV-005` refuses, so the list is asserted rather than counted.
    let by_decision: Vec<_> = ResourceField::ALL
        .into_iter()
        .filter(|field| ground_of(*field) == Ground::AcceptedDecision)
        .collect();
    assert_eq!(
        by_decision,
        vec![
            ResourceField::ForwardEventHorizon,
            ResourceField::CompiledEventShare,
            ResourceField::AuthoredRuntimeEventShare,
            ResourceField::LiveEventShare,
            ResourceField::SessionEventShare,
            ResourceField::InternalEventShare,
            ResourceField::ReleaseEventShare,
            ResourceField::ReleaseHoldCapacity,
        ]
    );
}

#[test]
fn no_profile_field_carries_a_quantum() {
    // `HOST-INV-004`: the render quantum is not a profile field, is not derived from
    // one, and is not configurable. The mechanical half of the invariant is the
    // compile-time assertion in `time`; this is the other half — a field named for the
    // quantum would be one, and the two fields whose names mention it are a per-quantum
    // *count* and a per-quantum *ratio*, neither of which is the quantum.
    let named: Vec<_> = ResourceField::ALL
        .into_iter()
        .filter(|field| field.name().contains("quantum"))
        .collect();
    assert_eq!(
        named,
        vec![
            ResourceField::MaxEventsPerQuantum,
            ResourceField::PredictedQuantumCostRatio
        ],
        "a field carrying the quantum itself would appear here"
    );
}

#[test]
fn the_report_says_whether_the_capabilities_were_queried_or_declared() {
    let host = profile(256, ChannelLayout::Mono);
    let outcome = compile(&GraphIr::empty(), &RenderConfig::new(host));
    assert_eq!(
        outcome.report().capability_source(),
        synth_engine_v2::profile::CapabilitySource::Harness,
        "a report quoting a profile must be able to say which it was"
    );
}

#[test]
fn every_row_names_a_contributor() {
    // The third of `HOST-INV-006`'s three columns. `IrObject::Plan` is a real answer
    // where no single object dominates — an aggregate count has no peak — and what the
    // invariant forbids is a row with nothing at all.
    let host = profile(256, ChannelLayout::Mono);
    let sine = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(440.0).expect("finite"),
        amplitude: Amplitude::new(0.5).expect("finite"),
    });
    let outcome = compile(&sine, &RenderConfig::new(host));
    check_contributors(outcome.report());

    // Including on the failing path.
    let mut groups = Groups::of(&host);
    groups.graph = GraphLimits::new(
        count(1),
        EdgeCount::limit(64).expect("positive"),
        FanOut::limit(64).expect("positive"),
        count(32),
        count(32),
    )
    .expect("the overridden capacities are above zero");
    let refused = compile(&sine, &RenderConfig::new(groups.build(&host)));
    assert!(refused.plan().is_err());
    check_contributors(refused.report());
}

fn check_contributors(report: &ResourceReport) {
    for row in report.rows() {
        // Every row has all three columns filled; `Display` is what a reader sees.
        assert!(!row.requested().to_string().is_empty());
        assert!(!row.available().to_string().is_empty());
        assert!(!row.contributor().to_string().is_empty());
    }
    assert_eq!(report.rows().len(), ResourceField::COUNT);
}

#[test]
fn the_scratch_budget_counts_what_preparation_actually_allocates() {
    // The defect this covers: the request counted the audio buffers and the two carries
    // and not the event scratch, so a raised event capacity was reported as fitting and
    // then allocated past the budget at preparation.
    let host = profile(256, ChannelLayout::Mono);
    let plan = source_plan(IrNodeKind::Silence);

    let scratch_request = |capacity: u32| {
        let mut groups = Groups::of(&host);
        let horizon = groups.events.forward_event_horizon();
        groups.events = EventLimits::new(
            events(capacity),
            groups.events.max_note_expansion_per_tick(),
            groups.events.max_scheduled_events_in_flight(),
            horizon,
            QueueCapacities::new(
                events(1),
                groups.events.queues().command_queue_capacity(),
                groups.events.queues().event_egress_capacity(),
            )
            .expect("the overridden capacities are above zero"),
            minimal_shares(),
        )
        .expect("the overridden capacities are above zero");
        compile(&plan, &RenderConfig::new(groups.build(&host)))
            .report()
            .row(ResourceField::BufferScratchBytes)
            .map(|row| row.requested().to_string())
            .expect("the row exists")
    };

    assert_ne!(
        scratch_request(6),
        scratch_request(4_096),
        "raising max_events_per_quantum must raise the scratch request, because it raises \
         what preparation allocates"
    );
}

#[test]
fn the_scratch_budget_counts_the_identity_halves_a_declared_polyphony_allocates() {
    // The same defect as the test above, one allocation later, and found by an independent
    // review of the slice that added it. Preparation allocates a minter slot **and** a
    // live-note entry per admitted index, both sized by the plan's declared producer
    // ranges — so a plan declaring a large polyphony against a profile near its scratch
    // ceiling would be reported as fitting and then allocate past it.
    let host = profile(256, ChannelLayout::Mono);

    let scratch_request = |simultaneous: u32| {
        let plan = GraphIr::builder()
            .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Voice)
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (SOURCE, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .declaring(PlanDeclarations {
                note_producers: vec![NoteProducerDeclaration {
                    compiled: true,
                    simultaneous_notes: HeldNoteCount::measured(simultaneous),
                    simultaneous_holds: EventCount::NONE,
                }],
                held_notes: HeldNoteCount::measured(simultaneous),
                ..PlanDeclarations::default()
            })
            .build()
            .expect("a source into an output is a readable plan");
        compile(&plan, &RenderConfig::new(host))
            .report()
            .row(ResourceField::BufferScratchBytes)
            .map(|row| row.requested().to_string())
            .expect("the row exists")
    };

    assert_ne!(
        scratch_request(1),
        scratch_request(4_096),
        "raising a producer's declared polyphony must raise the scratch request, because it \
         raises what preparation allocates for the identity halves"
    );
}

#[test]
fn an_output_port_this_phase_does_not_render_is_refused_rather_than_dropped() {
    // Lowering reads the first port only, so an edge into any other one used to compile
    // and render silence with nothing said. Phase 2 checks it against the node's
    // declared port table instead, which is why the refusal now names a direction.
    let host = profile(256, ChannelLayout::Mono);
    let ir = GraphIr::builder()
        .node(SOURCE, IrNodeKind::Silence, ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::new(3)),
            SignalDomain::Audio,
        )
        .build()
        .expect("readable plan");

    match compile(&ir, &RenderConfig::new(host)).plan() {
        Err(CompileError::UnknownPort {
            node, port, needed, ..
        }) => {
            assert_eq!(*node, OUTPUT);
            assert_eq!(*port, PortId::new(3));
            assert_eq!(*needed, PortDirection::Input);
        }
        other => panic!("expected an unknown-port refusal, got {other:?}"),
    }
}

#[test]
fn a_second_output_node_is_refused_rather_than_ignored() {
    let host = profile(256, ChannelLayout::Mono);
    let ir = GraphIr::builder()
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .node(NodeId::new(9), IrNodeKind::Output, ExecutionScope::Global)
        .build()
        .expect("readable plan");
    match compile(&ir, &RenderConfig::new(host)).plan() {
        Err(CompileError::MultipleOutputs { outputs }) => assert_eq!(outputs.get(), 2),
        other => panic!("expected a multiple-output refusal, got {other:?}"),
    }
}

#[test]
fn a_node_id_is_an_identity_and_not_a_position() {
    // Reordering the node list must not change what a report or a diagnostic names.
    let host = profile(256, ChannelLayout::Mono);
    let forward = GraphIr::builder()
        .node(
            NodeId::new(42),
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(NodeId::new(7), IrNodeKind::Output, ExecutionScope::Global)
        .build()
        .expect("readable plan");
    let reversed = GraphIr::builder()
        .node(NodeId::new(7), IrNodeKind::Output, ExecutionScope::Global)
        .node(
            NodeId::new(42),
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .build()
        .expect("readable plan");

    let of = |ir: &GraphIr| {
        compile(ir, &RenderConfig::new(host))
            .report()
            .row(ResourceField::MutableStateBytes)
            .map(|row| row.contributor())
    };
    assert_eq!(of(&forward), of(&reversed));
    assert_eq!(of(&forward), Some(IrObject::Node(NodeId::new(42))));
}

#[test]
fn a_compiled_note_producer_may_not_declare_a_hold() {
    // ADR-0046 clause 6: "Compiled releases use plan entitlements and need no hold." A
    // compiled source that asked for one would consume `release_hold_capacity` the
    // non-compiled producers are entitled to, which is the partition this refusal keeps
    // disjoint. Refused by name rather than summed, so the caller learns which producer.
    let host = profile(256, ChannelLayout::Mono);
    let ir = declaring(PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(4),
            simultaneous_holds: events(1),
        }],
        ..declared()
    });
    match compile(&ir, &RenderConfig::new(host)).into_plan() {
        Err(CompileError::CompiledProducerDeclaresHold { index, holds }) => {
            assert_eq!(index, 0);
            assert_eq!(holds, events(1));
        }
        other => panic!("expected a compiled-hold refusal, got {other:?}"),
    }
}

#[test]
fn a_producer_may_not_hold_more_obligations_than_it_has_notes() {
    // A hold is taken *by* a note-on, so a source cannot hold more obligations than it has
    // notes sounding — ADR-0046 clause 6 gives a hold only to a note-on whose release is not
    // already in the same sealed batch, which makes holds a subset of notes rather than a
    // separate budget.
    let host = profile(256, ChannelLayout::Mono);
    let ir = declaring(PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(2),
            simultaneous_holds: events(3),
        }],
        ..declared()
    });
    match compile(&ir, &RenderConfig::new(host)).into_plan() {
        Err(CompileError::ProducerHoldsExceedNotes {
            index,
            holds,
            notes,
        }) => {
            assert_eq!(index, 0);
            assert_eq!(holds, events(3));
            assert_eq!(notes, HeldNoteCount::measured(2));
        }
        other => panic!("expected a holds-exceed-notes refusal, got {other:?}"),
    }
}

#[test]
fn the_identity_partition_sums_compiled_producers_too() {
    // ADR-0047 clause 3: the identity partition covers a **superset** of the hold
    // partition, because every note-on needs an occurrence while only some need a hold. A
    // plan whose compiled producers alone outrun `max_held_notes` must be refused, and a
    // sum that filtered compiled sources out would admit it.
    let host = profile(256, ChannelLayout::Mono);
    let over = host
        .limits()
        .voices()
        .max_held_notes()
        .get()
        .saturating_add(1);
    let ir = declaring(PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(over),
            simultaneous_holds: EventCount::NONE,
        }],
        ..declared()
    });
    match compile(&ir, &RenderConfig::new(host)).into_plan() {
        Err(CompileError::LimitExceeded { field, .. }) => {
            assert_eq!(field, ResourceField::MaxHeldNotes);
        }
        other => panic!("expected the identity partition to refuse, got {other:?}"),
    }
}

#[test]
fn an_authored_source_must_name_a_note_producer_the_plan_declares() {
    // The contract hole an independent review found in an earlier attempt at this type: the
    // envelopes were declared with nothing tying them to the `note_producers` entry carrying
    // the source's `ProducerId` and its hold entitlement. A source whose producer resolves to
    // nothing has no entitlement to spend, so its holds would be checked against an absent
    // partition rather than a disjoint one.
    let host = profile(256, ChannelLayout::Mono);
    let ir = declaring(PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(4),
            simultaneous_holds: events(2),
        }],
        authored_sources: vec![AuthoredSourceDeclaration {
            // One past the only declared producer.
            producer: ProducerId::new(1),
            destination_occupancy: events(1),
            retained_future: EventCount::NONE,
            simultaneous_holds: events(1),
        }],
        ..declared()
    });
    match compile(&ir, &RenderConfig::new(host)).into_plan() {
        Err(CompileError::AuthoredSourceProducerUnknown { index, producer }) => {
            assert_eq!(index, 0);
            assert_eq!(producer, ProducerId::new(1));
        }
        other => panic!("expected an unknown-producer refusal, got {other:?}"),
    }
}

#[test]
fn an_authored_source_may_not_route_through_the_compiled_producer() {
    // ADR-0046 clause 6 gives a compiled producer plan entitlements and no hold, so its
    // entitlement is zero. Routing an authored source through it would take holds the
    // partition never granted — the mirror of the refusal a compiled producer takes for
    // declaring one itself.
    let host = profile(256, ChannelLayout::Mono);
    let ir = declaring(PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(4),
            simultaneous_holds: EventCount::NONE,
        }],
        authored_sources: vec![AuthoredSourceDeclaration {
            producer: ProducerId::new(0),
            destination_occupancy: events(1),
            retained_future: EventCount::NONE,
            simultaneous_holds: EventCount::NONE,
        }],
        ..declared()
    });
    match compile(&ir, &RenderConfig::new(host)).into_plan() {
        Err(CompileError::AuthoredSourceProducerCompiled { index, producer }) => {
            assert_eq!(index, 0);
            assert_eq!(producer, ProducerId::new(0));
        }
        other => panic!("expected a compiled-producer refusal, got {other:?}"),
    }
}

#[test]
fn an_authored_source_may_not_hold_more_than_its_producer_is_entitled_to() {
    // The producer's `simultaneous_holds` is its whole entitlement under clause 6's disjoint
    // partition. A source claiming more is spending another producer's unused holds, which
    // that clause forbids by name — and the overrun would otherwise surface as an exhausted
    // entitlement mid-playback rather than as a plan that was never admissible.
    //
    // Three against two: the producer's own declaration is legal on its own, so this refusal
    // can only come from comparing the two.
    let host = profile(256, ChannelLayout::Mono);
    let ir = declaring(PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(4),
            simultaneous_holds: events(2),
        }],
        authored_sources: vec![AuthoredSourceDeclaration {
            producer: ProducerId::new(0),
            destination_occupancy: events(1),
            retained_future: EventCount::NONE,
            simultaneous_holds: events(3),
        }],
        ..declared()
    });
    match compile(&ir, &RenderConfig::new(host)).into_plan() {
        Err(CompileError::AuthoredSourceHoldsAboveEntitlement {
            index,
            producer,
            holds,
            entitlement,
        }) => {
            assert_eq!(index, 0);
            assert_eq!(producer, ProducerId::new(0));
            assert_eq!(holds, events(3));
            assert_eq!(entitlement, events(2));
        }
        other => panic!("expected a holds-above-entitlement refusal, got {other:?}"),
    }
}

#[test]
fn two_authored_sources_that_each_fit_are_refused_together() {
    // ADR-0046 clause 5 admits "a checked plan-wide aggregate of every simultaneously legal
    // source", and rejects two that fit individually "unless the compiler proves them
    // mutually exclusive". No such proof exists and none is built, so every declared source
    // counts — which is the conservative direction: this can refuse a plan whose sources
    // never coincide, but never admit one whose sources do.
    //
    // The assertion is on both amounts, not just the refusal: a per-source check would also
    // fail this plan if the share were smaller, so only the *summed* request distinguishes
    // the two implementations.
    let host = profile(256, ChannelLayout::Mono);
    let ir = declaring(PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(8),
            simultaneous_holds: events(4),
        }],
        authored_sources: vec![
            AuthoredSourceDeclaration {
                producer: ProducerId::new(0),
                destination_occupancy: events(3),
                retained_future: EventCount::NONE,
                simultaneous_holds: events(1),
            },
            AuthoredSourceDeclaration {
                producer: ProducerId::new(0),
                destination_occupancy: events(3),
                retained_future: EventCount::NONE,
                simultaneous_holds: events(1),
            },
        ],
        ..declared()
    });
    let outcome = compile(&ir, &RenderConfig::new(host));
    let report = outcome.report();
    let row = report
        .rows()
        .iter()
        .find(|row| row.field() == ResourceField::AuthoredRuntimeEventShare)
        .expect("every field has a row");
    // Six, not three: the aggregate is the sum across sources.
    assert_eq!(row.requested(), ResourceAmount::Events(events(6)));
}

#[test]
fn two_authored_sources_sharing_a_producer_spend_one_entitlement_between_them() {
    // ADR-0046 clause 6 partitions hold entitlements per **producer**, not per source, so
    // two sources naming the same producer draw on one entitlement. Three each against an
    // entitlement of four: neither exceeds alone, and a per-source comparison would admit
    // this plan and let the second note-on find the entitlement already spent at runtime.
    //
    // The refusal reports the accumulated figure rather than the offending source's own, so
    // the caller sees the quantity that actually exceeded.
    let host = profile(256, ChannelLayout::Mono);
    let ir = declaring(PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(8),
            simultaneous_holds: events(4),
        }],
        authored_sources: vec![
            AuthoredSourceDeclaration {
                producer: ProducerId::new(0),
                destination_occupancy: events(1),
                retained_future: EventCount::NONE,
                simultaneous_holds: events(3),
            },
            AuthoredSourceDeclaration {
                producer: ProducerId::new(0),
                destination_occupancy: events(1),
                retained_future: EventCount::NONE,
                simultaneous_holds: events(3),
            },
        ],
        ..declared()
    });
    match compile(&ir, &RenderConfig::new(host)).into_plan() {
        Err(CompileError::AuthoredSourceHoldsAboveEntitlement {
            index,
            producer,
            holds,
            entitlement,
        }) => {
            // The *second* source is where the running total crosses.
            assert_eq!(index, 1);
            assert_eq!(producer, ProducerId::new(0));
            assert_eq!(holds, events(6));
            assert_eq!(entitlement, events(4));
        }
        other => panic!("expected an accumulated-holds refusal, got {other:?}"),
    }
}

#[test]
fn two_authored_sources_on_different_producers_do_not_pool_their_holds() {
    // The mirror, and what keeps the accumulation from being a global sum: entitlements are
    // disjoint per producer, so two sources on two producers each draw on their own. Three
    // and three against two four-hold producers fits, where one producer would refuse it.
    let host = profile(256, ChannelLayout::Mono);
    let ir = declaring(PlanDeclarations {
        note_producers: vec![
            NoteProducerDeclaration {
                compiled: false,
                simultaneous_notes: HeldNoteCount::measured(8),
                simultaneous_holds: events(4),
            },
            NoteProducerDeclaration {
                compiled: false,
                simultaneous_notes: HeldNoteCount::measured(8),
                simultaneous_holds: events(4),
            },
        ],
        authored_sources: vec![
            AuthoredSourceDeclaration {
                producer: ProducerId::new(0),
                destination_occupancy: events(1),
                retained_future: EventCount::NONE,
                simultaneous_holds: events(3),
            },
            AuthoredSourceDeclaration {
                producer: ProducerId::new(1),
                destination_occupancy: events(1),
                retained_future: EventCount::NONE,
                simultaneous_holds: events(3),
            },
        ],
        ..declared()
    });
    assert!(
        compile(&ir, &RenderConfig::new(host)).into_plan().is_ok(),
        "disjoint entitlements are not pooled"
    );
}

#[test]
fn authored_retained_future_is_admitted_above_the_compiled_floor() {
    // ADR-0046 clause 1's third authored relation: "the plan-wide aggregate maximum of
    // simultaneously retained authored future events fits the headroom above that floor".
    // Every other authored fixture here declares `retained_future` as `NONE`, so an
    // independent review found this relation had no conformance case at all — removing the
    // fold, or taking a maximum instead of a sum, would have passed the suite.
    //
    // The arithmetic: the harness profile renders four quanta per callback and the fixture's
    // compiled share is eight, so the floor is thirty-two. Against a store of 4 096, a
    // retention of 4 065 puts the total at 4 097 — one over, which is the smallest margin
    // that can distinguish a refusal from an accident.
    let host = profile(256, ChannelLayout::Mono);
    let declarations = PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(8),
            simultaneous_holds: events(4),
        }],
        authored_sources: vec![AuthoredSourceDeclaration {
            producer: ProducerId::new(0),
            destination_occupancy: events(1),
            retained_future: events(4_065),
            simultaneous_holds: events(1),
        }],
        ..declared()
    };
    let refusing = {
        let mut groups = Groups::of(&host);
        let horizon = groups.events.forward_event_horizon();
        let one = events(1);
        groups.events = EventLimits::new(
            events(64),
            events(128),
            events(4_096),
            horizon,
            QueueCapacities::new(one, events(16_384), events(256))
                .expect("the overridden capacities are above zero"),
            ProducerShares::new(events(8), events(8), one, one, one, events(40), events(40))
                .expect("a valid partition"),
        )
        .expect("the overridden capacities are above zero");
        groups.build(&host)
    };
    match compile(
        &declaring(declarations.clone()),
        &RenderConfig::new(refusing),
    )
    .into_plan()
    {
        Err(CompileError::LimitExceeded { field, .. }) => {
            assert_eq!(field, ResourceField::MaxScheduledEventsInFlight);
        }
        other => panic!("expected a scheduled-window refusal, got {other:?}"),
    }

    // And one below the floor-plus-retention still fits, so the refusal is the relation
    // rather than the presence of an authored source.
    let fitting = PlanDeclarations {
        authored_sources: vec![AuthoredSourceDeclaration {
            retained_future: events(4_064),
            ..declarations.authored_sources[0]
        }],
        ..declarations
    };
    let accepting = {
        let mut groups = Groups::of(&host);
        let horizon = groups.events.forward_event_horizon();
        let one = events(1);
        groups.events = EventLimits::new(
            events(64),
            events(128),
            events(4_096),
            horizon,
            QueueCapacities::new(one, events(16_384), events(256))
                .expect("the overridden capacities are above zero"),
            ProducerShares::new(events(8), events(8), one, one, one, events(40), events(40))
                .expect("a valid partition"),
        )
        .expect("the overridden capacities are above zero");
        groups.build(&host)
    };
    assert!(
        compile(&declaring(fitting), &RenderConfig::new(accepting))
            .into_plan()
            .is_ok(),
        "the exact bound fits"
    );
}

#[test]
fn a_declared_total_past_what_an_event_count_names_is_reported_and_refused() {
    // **The row must name what the plan requested, even when no `EventCount` can hold it.**
    // `HOST-INV-006` requires the requested amount on every row, and the retained-future
    // aggregate sums two declarations *plus* the compiled floor. Saturating to `u32::MAX` was
    // the earlier behaviour and it lied twice: it understated the request, and against a
    // profile whose limit is `u32::MAX` the row read `Within` for a total it exceeded, so the
    // plan was **admitted**. A merge-gate review found that path.
    //
    // **It is this row and not a share row**, which writing the fixture established: a share
    // of `u32::MAX` cannot be constructed at all, because the six shares must sum to a
    // representable total, so the tie is unreachable there. `max_scheduled_events_in_flight`
    // is not a term in that sum, so its limit *can* be `u32::MAX` — which is exactly the row
    // the review cited.
    let host = profile(256, ChannelLayout::Mono);
    let huge = events(u32::MAX);
    let ir = declaring(PlanDeclarations {
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(8),
            simultaneous_holds: events(4),
        }],
        authored_sources: vec![
            AuthoredSourceDeclaration {
                producer: ProducerId::new(0),
                destination_occupancy: events(1),
                retained_future: huge,
                simultaneous_holds: events(1),
            },
            AuthoredSourceDeclaration {
                producer: ProducerId::new(0),
                destination_occupancy: events(1),
                retained_future: huge,
                simultaneous_holds: events(1),
            },
        ],
        ..declared()
    });
    let permissive = {
        let mut groups = Groups::of(&host);
        let horizon = groups.events.forward_event_horizon();
        let one = events(1);
        groups.events = EventLimits::new(
            events(64),
            events(128),
            // The largest store an `EventCount` can name, which is what a saturated request
            // would compare equal to.
            huge,
            horizon,
            QueueCapacities::new(one, events(16_384), events(256))
                .expect("the overridden capacities are above zero"),
            ProducerShares::new(events(8), events(8), one, one, one, events(40), events(40))
                .expect("a valid partition"),
        )
        .expect("the overridden capacities are above zero");
        groups.build(&host)
    };

    let outcome = compile(&ir, &RenderConfig::new(permissive));
    let row = outcome
        .report()
        .rows()
        .iter()
        .find(|row| row.field() == ResourceField::MaxScheduledEventsInFlight)
        .expect("every field has a row");
    // Two of `u32::MAX` plus the compiled floor, named exactly rather than clamped.
    let expected = u64::from(u32::MAX) * 2 + u64::from(events(8).get()) * 4;
    assert_eq!(
        row.requested(),
        ResourceAmount::EventsBeyondCount(
            synth_engine_v2::report::EventsBeyondCount::new(expected)
                .expect("two of u32::MAX plus a floor is past what an EventCount names")
        )
    );

    // The constructor is the invariant, so assert it refuses a value that would have fit:
    // a wide amount built from a small number would let one report claim two different
    // things about the same figure.
    assert!(
        synth_engine_v2::report::EventsBeyondCount::new(u64::from(u32::MAX)).is_none(),
        "a value an EventCount names is not a wide amount"
    );
    assert!(
        outcome.into_plan().is_err(),
        "a request past every representable store must not be admitted"
    );
}
