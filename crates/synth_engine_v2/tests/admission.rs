//! Admission: the report, the refusals, and the advisory budget.
//!
//! The refusal cases are `HOST-INV-007`'s conformance row: **one per render limit a
//! plan can exceed**, each asserting the error names the field, both amounts, and the
//! responsible object, and that the plan the caller handed in is unchanged. There are
//! twenty-eight, and the test asserts that number against
//! [`ResourceField::is_admission_checked`] so a new limit cannot arrive without a case.

mod common;

use common::{OUTPUT, SOURCE, declaring, profile, source_plan};

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::diagnostics::{CompileError, CompileWarning};
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, IrObject, IrProgram, NodeId, PlanDeclarations, PortId,
    ProgramId, SignalDomain, TapId,
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
use synth_engine_v2::report::{ResourceField, ResourceReport};
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
            ResourceField::MaxEventsPerQuantum,
            declares_seven_events,
            event_limits(6, 128, 4_096),
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
    assert_eq!(checked.len(), 28, "the admission-checked set changed size");
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
