use super::*;
use crate::{
    compile::{RenderConfig, compile},
    host::session::{
        SessionCommandCapacity, SessionLimits,
        transfer::{
            SessionAudio, SessionControl, SessionTransferLimits,
            replacement::{PlanReplacementOutcome, PlanReplacementRefusal},
        },
    },
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    profile::HostProfile,
    quantities::{
        Amplitude, ChannelLayout, CutoffFrequency, DelayFeedback, DelayTime, Frequency, GainFactor,
        NormalizedLevel, Resonance, SampleRate,
    },
    render::AudioBlockMut,
    schedule::AdmittedCompiledStream,
    stream::StreamControl,
    time::{FrameCount, PlanPosition, SampleTime, StreamAnchor},
};

const SOURCE: NodeId = NodeId::new(10);
const FILTER: NodeId = NodeId::new(20);
const DELAY: NodeId = NodeId::new(30);
const OUTPUT: NodeId = NodeId::new(40);
const EXTRA: NodeId = NodeId::new(15);
fn graph(extra: bool, cutoff: f32) -> GraphIr {
    let mut builder = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(317.0).unwrap(),
                amplitude: Amplitude::new(0.25).unwrap(),
            },
            ExecutionScope::Global,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(cutoff).unwrap(),
                resonance: Resonance::BUTTERWORTH,
            },
            ExecutionScope::Global,
        )
        .node(
            DELAY,
            IrNodeKind::Delay {
                time_left: DelayTime::new(0.001).unwrap(),
                time_right: DelayTime::new(0.002).unwrap(),
                feedback: DelayFeedback::new(0.4).unwrap(),
                mix: NormalizedLevel::new(0.4).unwrap(),
                tone: NormalizedLevel::FULL,
            },
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global);
    let source = if extra {
        builder = builder
            .node(
                EXTRA,
                IrNodeKind::Gain {
                    factor: GainFactor::new(1.0).unwrap(),
                },
                ExecutionScope::Global,
            )
            .connect(
                (SOURCE, PortId::FIRST),
                (EXTRA, PortId::FIRST),
                SignalDomain::Audio,
            );
        EXTRA
    } else {
        SOURCE
    };
    builder
        .connect(
            (source, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (DELAY, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (DELAY, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .unwrap()
}
fn profile(hz: f32) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(hz).unwrap(),
        FrameCount::new(256),
        ChannelLayout::Stereo,
    )
    .unwrap()
}
fn plan(extra: bool, cutoff: f32, profile: HostProfile) -> CompiledPlan {
    compile(&graph(extra, cutoff), &RenderConfig::new(profile))
        .into_plan()
        .unwrap()
}
fn limits() -> SessionTransferLimits {
    SessionTransferLimits {
        session: SessionLimits {
            commands: SessionCommandCapacity::new(4).unwrap(),
            command_bytes: PreparedBytes::measured(65_536),
        },
        control_bytes: PreparedBytes::measured(65_536),
    }
}
fn collect(control: &mut SessionControl, audio: &mut SessionAudio) {
    while let Some(packet) = audio.take_completed() {
        let receipt = control.collect(packet).unwrap();
        assert!(matches!(
            receipt.outcome,
            crate::host::session::SessionOutcome::Applied { .. }
        ));
    }
}
fn render(audio: &mut SessionAudio, frames: usize, partition: &[usize]) -> Vec<u32> {
    let mut samples = vec![0.0; frames * 2];
    let mut offset = 0;
    let mut index = 0;
    while offset < frames {
        let n = partition[index % partition.len()].min(frames - offset);
        let block = AudioBlockMut::new(
            &mut samples[offset * 2..(offset + n) * 2],
            n,
            ChannelLayout::Stereo,
        )
        .unwrap();
        assert_eq!(
            crate::render_allocation::count_allocs(|| audio.render(block).unwrap()),
            0
        );
        index += 1;
        offset += n;
    }
    samples.into_iter().map(f32::to_bits).collect()
}
fn run(hz: f32, policy: Option<bool>, partition: &[usize]) -> Vec<u32> {
    let profile = profile(hz);
    let old = plan(false, 1_000.0, profile);
    let old_filter = old
        .non_voice_state_nodes()
        .iter()
        .find(|(id, _, _)| *id == FILTER)
        .unwrap()
        .2;
    let stream = AdmittedCompiledStream::admit(&old, &[]).unwrap();
    let (mut control, mut audio) = SessionControl::prepare(old, stream, profile, limits()).unwrap();
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    render(&mut audio, 384, partition);
    collect(&mut control, &mut audio);
    audio
        .enqueue(control.prepare_stop(audio.clock()).unwrap())
        .unwrap();
    render(&mut audio, 64, partition);
    collect(&mut control, &mut audio);
    let candidate = policy.map(|compatible| {
        let new = plan(true, 1_000.0, profile);
        let new_filter = new
            .non_voice_state_nodes()
            .iter()
            .find(|(id, _, _)| *id == FILTER)
            .unwrap()
            .2;
        assert_ne!(
            old_filter, new_filter,
            "fixture must move a stateful node's schedule slot"
        );
        let stream = AdmittedCompiledStream::admit(&new, &[]).unwrap();
        if compatible {
            let packet = control
                .prepare_compatible_replacement(new, stream, profile, limits())
                .unwrap();
            assert_eq!(packet.compatible_records(), RecordCount::measured(3));
            assert!(packet.migration_bytes().get() > 0);
            packet
        } else {
            control
                .prepare_replacement(new, stream, profile, limits())
                .unwrap()
        }
    });
    render(&mut audio, 192, partition); // The host clock advances while DSP is paused.
    if let Some(candidate) = candidate {
        let mut returned = None;
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                returned = Some(audio.install_replacement(candidate).unwrap());
            }),
            0
        );
        let returned = returned.unwrap();
        assert!(matches!(
            returned.outcome(),
            Some(PlanReplacementOutcome::Installed(_))
        ));
        let receipt = control.collect_replacement(returned).unwrap();
        assert!(matches!(
            receipt.outcome,
            PlanReplacementOutcome::Installed(_)
        ));
    }
    audio
        .enqueue(control.prepare_play(audio.clock()).unwrap())
        .unwrap();
    let output = render(&mut audio, 512, partition);
    collect(&mut control, &mut audio);
    output
}

#[test]
fn stopped_native_state_and_delay_history_survive_reordered_plans_at_four_rates_and_partitions() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let reference = run(hz, None, &[64]);
        assert!(reference.iter().any(|bits| bits & 0x7fff_ffff != 0));
        assert_ne!(
            run(hz, Some(false), &[64]),
            reference,
            "reset control must expose missing state"
        );
        for partition in [&[1][..], &[64], &[256], &[7, 111, 19, 63]] {
            assert_eq!(
                run(hz, Some(true), partition),
                reference,
                "{hz} {partition:?}"
            );
        }
    }
}

fn renderer(plan: CompiledPlan) -> PreparedRenderer {
    StreamControl::open(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .unwrap()
    .1
}
fn drained(plan: CompiledPlan) -> PreparedRenderer {
    let mut renderer = renderer(plan);
    let mut samples = [0.0; 128];
    renderer
        .render_idle(AudioBlockMut::new(&mut samples, 64, ChannelLayout::Stereo).unwrap())
        .unwrap();
    renderer
}
fn budget() -> PreparedBytes {
    PreparedBytes::measured(65_536)
}

#[test]
fn changed_parameters_and_new_nodes_stay_fresh_while_unchanged_state_moves_by_id() {
    let profile = profile(48_000.0);
    let mut old = drained(plan(false, 1_000.0, profile));
    let mut new = renderer(plan(true, 2_000.0, profile));
    let migration = PreparedGlobalMigration::prepare(old.plan(), new.plan(), budget()).unwrap();
    assert_eq!(migration.records(), RecordCount::measured(2)); // oscillator and delay
    let source = old
        .plan
        .non_voice_state_nodes()
        .iter()
        .find(|(id, _, _)| *id == SOURCE)
        .unwrap()
        .2
        .index();
    // Change audio-owned state only AFTER preparing the map. A captured DSP
    // snapshot would restore the old phase and fail the comparison below.
    if let NodeState::Sine { phase, .. } = &mut old.node_states[source] {
        assert_eq!(*phase, 0.0);
        *phase = 0.37;
    }
    let initial = new.node_states.clone();
    migration.transfer_into(&mut old, &mut new).unwrap();
    for (id, _, slot) in new.plan.non_voice_state_nodes() {
        if *id == FILTER || *id == EXTRA {
            assert_eq!(new.node_states[slot.index()], initial[slot.index()]);
        }
        if *id == SOURCE {
            assert_eq!(new.node_states[slot.index()], old.node_states[source]);
        }
    }
}

#[test]
fn preflight_of_a_late_damaged_history_span_changes_no_state_or_parameter() {
    let profile = profile(48_000.0);
    let mut old = drained(plan(false, 1_000.0, profile));
    for state in &mut old.node_states {
        if let NodeState::Sine { phase, .. } = state {
            *phase = 0.37;
        }
    }
    for buffer in &mut old.history {
        buffer.fill(0.125);
    }
    let value = old
        .parameter_slots
        .first_mut()
        .unwrap()
        .write_override(crate::quantities::ParameterValue::new(0.75).unwrap());
    assert_eq!(value.as_f32(), 0.75);
    let mut new = renderer(plan(true, 1_000.0, profile));
    let migration = PreparedGlobalMigration::prepare(old.plan(), new.plan(), budget()).unwrap();
    let delay = new
        .plan
        .non_voice_state_nodes()
        .iter()
        .find(|(id, _, _)| *id == DELAY)
        .unwrap()
        .2
        .index();
    new.history[delay] = Box::default();
    let states = new.node_states.clone();
    let slots = new.parameter_slots.clone();
    let history = new.history.clone();
    assert_eq!(
        migration.transfer_into(&mut old, &mut new),
        Err(GlobalMigrationError::Storage)
    );
    assert_eq!(new.node_states, states);
    assert_eq!(new.parameter_slots, slots);
    assert_eq!(new.history, history);
}

#[test]
fn foreign_maps_and_too_small_budgets_refuse_before_any_copy() {
    let profile = profile(48_000.0);
    let mut old = drained(plan(false, 1_000.0, profile));
    let mut new = renderer(plan(true, 1_000.0, profile));
    assert!(matches!(
        PreparedGlobalMigration::prepare(old.plan(), new.plan(), PreparedBytes::NONE),
        Err(GlobalMigrationError::Bytes)
    ));
    let foreign = plan(false, 1_000.0, profile);
    let migration = PreparedGlobalMigration::prepare(&foreign, new.plan(), budget()).unwrap();
    let before = new.node_states.clone();
    assert_eq!(
        migration.transfer_into(&mut old, &mut new),
        Err(GlobalMigrationError::Origin)
    );
    assert_eq!(new.node_states, before);
}

#[test]
fn an_intervening_same_plan_publication_invalidates_a_migration_by_table_identity() {
    let profile = profile(48_000.0);
    let old = plan(false, 1_000.0, profile);
    let stream = AdmittedCompiledStream::admit(&old, &[]).unwrap();
    let (mut control, mut audio) =
        SessionControl::prepare(old.clone(), stream, profile, limits()).unwrap();
    render(&mut audio, 64, &[64]);
    let reset = control
        .prepare_replacement(
            old.clone(),
            AdmittedCompiledStream::admit(&old, &[]).unwrap(),
            profile,
            limits(),
        )
        .unwrap();
    let compatible = control
        .prepare_compatible_replacement(
            old.clone(),
            AdmittedCompiledStream::admit(&old, &[]).unwrap(),
            profile,
            limits(),
        )
        .unwrap();
    let reset = audio.install_replacement(reset).unwrap();
    let refused = audio.install_replacement(compatible).unwrap();
    assert_eq!(
        refused.outcome(),
        Some(PlanReplacementOutcome::Refused(
            PlanReplacementRefusal::MigrationOrigin
        ))
    );
    let installed = control.collect_replacement(reset).unwrap();
    assert!(matches!(
        installed.outcome,
        PlanReplacementOutcome::Installed(_)
    ));
    let rejected = control.collect_replacement(refused).unwrap();
    assert_eq!(
        rejected.outcome,
        PlanReplacementOutcome::Refused(PlanReplacementRefusal::MigrationOrigin)
    );
    assert!(!control.has_replacements());
}

#[test]
fn modulation_cannot_leave_old_layers_in_an_unmodulated_successor() {
    use crate::ir::{LfoPolarity, LfoWaveform, ModulationDepth, ModulationUnit, parameters};
    let profile = profile(48_000.0);
    let plain = graph(false, 1_000.0);
    let mut builder = GraphIr::builder();
    for node in plain.nodes() {
        builder = builder.node(node.id(), node.kind(), node.scope());
    }
    for edge in plain.edges() {
        builder = builder.connect(edge.from(), edge.to(), edge.domain());
    }
    let modulated = builder
        .node(
            NodeId::new(50),
            IrNodeKind::Lfo {
                waveform: LfoWaveform::Sine,
                rate: Frequency::ONE,
                depth: NormalizedLevel::FULL,
                phase_offset: crate::quantities::PhaseOffset::new(0.0).unwrap(),
                polarity: LfoPolarity::Bipolar,
            },
            ExecutionScope::Global,
        )
        .modulate(
            (NodeId::new(50), PortId::FIRST),
            (FILTER, parameters::FILTER_CUTOFF),
            ModulationDepth::new(ModulationUnit::Semitones, 1.0).unwrap(),
        )
        .build()
        .unwrap();
    let modulated = compile(&modulated, &RenderConfig::new(profile))
        .into_plan()
        .unwrap();
    let plain = plan(false, 1_000.0, profile);
    for (old, new) in [(&modulated, &plain), (&plain, &modulated)] {
        assert!(matches!(
            PreparedGlobalMigration::prepare(old, new, budget()),
            Err(GlobalMigrationError::Routing)
        ));
    }
}

#[test]
fn inserted_stateful_compensation_refuses_instead_of_losing_its_history() {
    use crate::{ir::PlanDeclarations, latency::CompensationPolicy};
    let profile = profile(48_000.0);
    let ir = GraphIr::builder()
        .declaring(PlanDeclarations {
            compensation: CompensationPolicy::Compensate,
            ..PlanDeclarations::default()
        })
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::UNITY,
            },
            ExecutionScope::Global,
        )
        .node(
            FILTER,
            IrNodeKind::Latency {
                frames: FrameCount::new(17),
            },
            ExecutionScope::Global,
        )
        .node(DELAY, IrNodeKind::Mix, ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SOURCE, PortId::FIRST),
            (DELAY, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (DELAY, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (DELAY, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .unwrap();
    let plan = compile(&ir, &RenderConfig::new(profile))
        .into_plan()
        .unwrap();
    assert!(matches!(
        PreparedGlobalMigration::prepare(&plan, &plan, budget()),
        Err(GlobalMigrationError::UnidentifiedState { .. })
    ));
}

#[test]
fn migration_budget_failure_spends_no_replacement_credit() {
    let profile = profile(48_000.0);
    let plan = plan(false, 1_000.0, profile);
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let (mut control, _audio) =
        SessionControl::prepare(plan.clone(), stream, profile, limits()).unwrap();
    let mut tight = limits();
    tight.control_bytes = control.control_bytes();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    assert!(matches!(
        control.prepare_compatible_replacement(plan, stream, profile, tight),
        Err(
            crate::host::session::transfer::replacement::PlanReplacementError::Migration(
                GlobalMigrationError::Bytes
            )
        )
    ));
    assert!(!control.has_replacements());
    let play = control.prepare_play(SampleTime::ZERO).unwrap();
    let cancelled = control.cancel(play).unwrap();
    assert_eq!(
        cancelled.outcome,
        crate::host::session::SessionOutcome::Cancelled
    );
}

#[test]
fn an_override_layer_follows_its_stable_parameter_after_rows_move() {
    let profile = profile(48_000.0);
    let mut old = drained(plan(false, 1_000.0, profile));
    // Gain moves node slots but has no parameter rows. This fixture inserts
    // another filter to independently prove parameter-row relocation.
    let base = graph(false, 1_000.0);
    let mut builder = GraphIr::builder();
    for node in base.nodes() {
        builder = builder.node(node.id(), node.kind(), node.scope());
    }
    for edge in base.edges().iter().filter(|edge| edge.to().0 != FILTER) {
        builder = builder.connect(edge.from(), edge.to(), edge.domain());
    }
    let edited = builder
        .node(
            EXTRA,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(1_500.0).unwrap(),
                resonance: Resonance::BUTTERWORTH,
            },
            ExecutionScope::Global,
        )
        .connect(
            (SOURCE, PortId::FIRST),
            (EXTRA, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (EXTRA, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .unwrap();
    let mut new = renderer(
        compile(&edited, &RenderConfig::new(profile))
            .into_plan()
            .unwrap(),
    );
    let source = old
        .plan
        .resolve_parameter(FILTER, crate::ir::parameters::FILTER_CUTOFF)
        .unwrap();
    let destination = new
        .plan
        .resolve_parameter(FILTER, crate::ir::parameters::FILTER_CUTOFF)
        .unwrap();
    assert_ne!(source.index(), destination.index());
    let resolved = old.parameter_slots[source.index()]
        .write_override(crate::quantities::ParameterValue::new(650.0).unwrap());
    assert_eq!(resolved.as_f32(), 650.0);
    assert_ne!(
        new.parameter_slots[destination.index()],
        old.parameter_slots[source.index()]
    );
    let migration = PreparedGlobalMigration::prepare(old.plan(), new.plan(), budget()).unwrap();
    assert_eq!(
        crate::render_allocation::count_allocs(|| migration
            .transfer_into(&mut old, &mut new)
            .unwrap()),
        0
    );
    assert_eq!(
        new.parameter_slots[destination.index()],
        old.parameter_slots[source.index()]
    );
}

fn scoped_plan(scope: ExecutionScope, extra: bool) -> CompiledPlan {
    let entry = NodeId::new(5);
    let strip = NodeId::new(35);
    let fold = NodeId::new(6);
    let mut graph = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(317.0).unwrap(),
                amplitude: Amplitude::new(0.25).unwrap(),
            },
            ExecutionScope::Global,
        )
        .node(entry, IrNodeKind::Mix, scope)
        .node(fold, IrNodeKind::StereoToMono, scope)
        .connect(
            (entry, PortId::FIRST),
            (fold, PortId::FIRST),
            SignalDomain::Audio,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(1_000.0).unwrap(),
                resonance: Resonance::BUTTERWORTH,
            },
            scope,
        )
        .node(
            DELAY,
            IrNodeKind::Delay {
                time_left: DelayTime::new(0.001).unwrap(),
                time_right: DelayTime::new(0.002).unwrap(),
                feedback: DelayFeedback::new(0.4).unwrap(),
                mix: NormalizedLevel::new(0.4).unwrap(),
                tone: NormalizedLevel::FULL,
            },
            scope,
        )
        .node(
            strip,
            if matches!(scope, ExecutionScope::Channel(_) | ExecutionScope::Bus(_)) {
                IrNodeKind::Channel {
                    fader: Amplitude::new(1.0).unwrap(),
                    pan: crate::controller::BipolarLevel::new(0.0).unwrap(),
                    muted: false,
                }
            } else {
                IrNodeKind::Balance {
                    level: Amplitude::new(1.0).unwrap(),
                    pan: crate::controller::BipolarLevel::new(0.0).unwrap(),
                    muted: false,
                }
            },
            scope,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (entry, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (DELAY, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (DELAY, PortId::FIRST),
            (strip, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (strip, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        );
    let predecessor = if extra {
        graph = graph
            .node(
                EXTRA,
                IrNodeKind::Gain {
                    factor: GainFactor::new(1.0).unwrap(),
                },
                scope,
            )
            .connect(
                (fold, PortId::FIRST),
                (EXTRA, PortId::FIRST),
                SignalDomain::Audio,
            );
        EXTRA
    } else {
        fold
    };
    let graph = graph
        .connect(
            (predecessor, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .unwrap();
    compile(&graph, &RenderConfig::new(profile(48_000.0)))
        .into_plan()
        .unwrap()
}

fn warm(renderer: &mut PreparedRenderer) {
    let mut output = [0.0; 512];
    for _ in 0..5 {
        crate::render::Renderer::render(
            renderer,
            AudioBlockMut::new(&mut output, 256, ChannelLayout::Stereo).unwrap(),
            crate::render::TimedEvents::EMPTY,
        )
        .unwrap();
    }
}

#[test]
fn authored_non_voice_scopes_preserve_state_and_history_after_slots_move() {
    for scope in [
        ExecutionScope::Global,
        ExecutionScope::InstrumentInstance,
        ExecutionScope::Channel(crate::ir::ChannelTag::FIRST),
        ExecutionScope::Bus(crate::ir::BusTag::FIRST),
    ] {
        let mut old = renderer(scoped_plan(scope, false));
        let mut new = renderer(scoped_plan(scope, true));
        warm(&mut old);
        let mapping = PreparedGlobalMigration::prepare(&old.plan, &new.plan, budget()).unwrap();
        let filter_slot = |plan: &CompiledPlan| {
            plan.non_voice_state_nodes()
                .iter()
                .find(|(id, _, _)| *id == FILTER)
                .unwrap()
                .2
        };
        assert_ne!(filter_slot(&old.plan), filter_slot(&new.plan));
        assert!(
            old.history
                .iter()
                .any(|buffer| buffer.iter().any(|sample| *sample != 0.0))
        );
        let expected = old.history.clone();
        let pointers: Vec<_> = old.history.iter().map(|buffer| buffer.as_ptr()).collect();
        assert_eq!(
            crate::render_allocation::count_allocs(|| mapping
                .transfer_into(&mut old, &mut new)
                .unwrap()),
            0
        );
        for (id, _, slot) in new.plan.non_voice_state_nodes() {
            if let Some((_, _, source)) = old
                .plan
                .non_voice_state_nodes()
                .iter()
                .find(|(candidate, _, _)| candidate == id)
            {
                assert_eq!(
                    new.node_states[slot.index()],
                    old.node_states[source.index()],
                    "{scope:?}: {id:?}"
                );
                assert_eq!(new.history[slot.index()], expected[source.index()]);
                if !new.history[slot.index()].is_empty() {
                    assert_eq!(new.history[slot.index()].as_ptr(), pointers[source.index()]);
                }
            }
        }
    }
}

#[test]
fn identical_node_ids_in_changed_scopes_or_scope_tags_stay_fresh() {
    use crate::ir::{BusTag, ChannelTag};
    for (from, to) in [
        (ExecutionScope::Global, ExecutionScope::InstrumentInstance),
        (ExecutionScope::InstrumentInstance, ExecutionScope::Global),
        (
            ExecutionScope::Channel(ChannelTag::FIRST),
            ExecutionScope::Channel(ChannelTag::new(1)),
        ),
        (
            ExecutionScope::Bus(BusTag::FIRST),
            ExecutionScope::Bus(BusTag::new(1)),
        ),
    ] {
        let mut old = renderer(scoped_plan(from, false));
        let mut new = renderer(scoped_plan(to, true));
        warm(&mut old);
        let initial = new.node_states.clone();
        let mapping = PreparedGlobalMigration::prepare(&old.plan, &new.plan, budget()).unwrap();
        mapping.transfer_into(&mut old, &mut new).unwrap();
        for (id, scope, slot) in new.plan.non_voice_state_nodes() {
            if *scope == to && *id != SOURCE {
                assert_eq!(
                    new.node_states[slot.index()],
                    initial[slot.index()],
                    "{from:?} to {to:?}: {id:?}"
                );
                assert!(new.history[slot.index()].iter().all(|value| *value == 0.0));
            }
        }
    }
}

#[test]
fn duplicate_source_or_destination_refuses_before_any_history_owner_moves() {
    for source_alias in [false, true] {
        let mut old = renderer(scoped_plan(ExecutionScope::InstrumentInstance, false));
        let mut new = renderer(scoped_plan(ExecutionScope::InstrumentInstance, true));
        warm(&mut old);
        let mut map = PreparedGlobalMigration::prepare(&old.plan, &new.plan, budget()).unwrap();
        assert!(map.nodes.len() >= 2);
        if source_alias {
            map.nodes[1].old = map.nodes[0].old;
        } else {
            map.nodes[1].new = map.nodes[0].new;
        }
        let old_contents = old.history.clone();
        let old_pointers: Vec<_> = old.history.iter().map(|buffer| buffer.as_ptr()).collect();
        let new_contents = new.history.clone();
        let states = new.node_states.clone();
        let parameters = new.parameter_slots.clone();
        let mut result = Ok(());
        assert_eq!(
            crate::render_allocation::count_allocs(
                || result = map.transfer_into(&mut old, &mut new)
            ),
            0
        );
        assert_eq!(result, Err(GlobalMigrationError::Storage));
        assert_eq!(old.history, old_contents);
        assert_eq!(
            old.history
                .iter()
                .map(|buffer| buffer.as_ptr())
                .collect::<Vec<_>>(),
            old_pointers
        );
        assert_eq!(new.history, new_contents);
        assert_eq!(new.node_states, states);
        assert_eq!(new.parameter_slots, parameters);
    }
}
