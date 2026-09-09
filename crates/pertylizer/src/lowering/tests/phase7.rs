//! P07-S008: real authoring routes share the same composition law.
use super::*;
use synth_engine_v2::{
    compile::{RenderConfig, compile},
    ir::{
        ExecutionScope, GraphIr, IrNodeKind, ModulationDepth, ModulationUnit, PortId, parameters,
    },
    offline::{OfflineEvent, render_offline},
    plan::CompiledPlan,
    quantities::{
        ChannelLayout, EventCount, KeyIdentity, NoteVelocity, ParameterValue, SampleRate,
    },
    schedule::CompiledPayload,
    script::{ProjectSeed, ScriptIdentity, ScriptStateId},
    time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime},
};
const SCRIPT: NodeId = NodeId::new(1_000_000);
const QUANTA: usize = 32;
const Q: usize = QUANTUM_FRAMES as usize;

fn authoring() -> GraphIr {
    use synth_sequencer::{
        AutomationTarget, ModConnection, ModNodeConfig, ModNodeId, ModTarget, ModulationAmount,
        ModuleNode,
    };
    let (mut modules, connections) = corpus_patch("sawtooth");
    let mut source = lfo("lfo-1", &[("rate", 1.0), ("depth", 1.0), ("phase", 0.0)]);
    choice(&mut source, "waveform", "square");
    modules.push(source);
    modules.push(mod_matrix("mmx-1", &[("lfo-1.out", "flt-1.cutoff", 0.7)]));
    let mut song = four_note_song();
    let id = song.create_mod_graph("combined");
    let graph = song.mod_graph_mut(id).expect("graph");
    graph
        .try_insert_node(
            ModNodeId::new(1),
            ModNodeConfig::Module(ModuleNode {
                module_type: ModuleType::Lfo,
                params: BTreeMap::from([
                    ("rate".into(), 2.0),
                    ("depth".into(), 1.0),
                    ("waveform".into(), 3.0),
                    ("phase".into(), 0.0),
                ]),
                seed: Some(7),
            }),
        )
        .expect("source");
    graph
        .try_insert_node(
            ModNodeId::new(2),
            ModNodeConfig::Target(ModTarget {
                target: AutomationTarget::Module {
                    instrument: instrument(),
                    module_type: ModuleType::Filter,
                    instance: 1,
                    param_id: "cutoff".into(),
                },
                amount: ModulationAmount::new(-0.5),
                combine: Default::default(),
            }),
        )
        .expect("target");
    graph
        .try_connect(ModConnection::new(
            ModNodeId::new(1),
            "out",
            ModNodeId::new(2),
            "in",
        ))
        .expect("route");
    let sources = super::super::modulation::lower_mod_grid(&song, instrument(), &modules);
    assert!(!sources.refused, "{:?}", sources.diagnostics);
    let lowered = super::super::graph::lower_voice_patch_with(
        instrument(),
        &modules,
        &connections,
        EventCount::measured(2),
        None,
        None,
        &sources,
    );
    lowered
        .ir
        .unwrap_or_else(|| panic!("{:?}", lowered.diagnostics))
}

fn rebuilt(ir: &GraphIr, enabled: [bool; 3]) -> GraphIr {
    let mut builder = GraphIr::builder().declaring(ir.declarations().clone());
    for node in ir.nodes() {
        builder = builder.node(node.id(), node.kind(), node.scope());
    }
    for edge in ir.edges() {
        builder = builder.connect(edge.from(), edge.to(), edge.domain());
    }
    for tuning in ir.tunings() {
        builder = builder.tuning(tuning.scope(), tuning.tuning().clone());
    }
    assert!(ir.samples().is_empty() && ir.maps().is_empty());
    for (index, edge) in ir.modulations().iter().enumerate() {
        if enabled[index] {
            builder = builder.modulate(edge.source(), edge.target(), edge.depth());
        }
    }
    if enabled[2] {
        let program = ScriptIdentity::new(SCRIPT, ScriptStateId::new(2), ProjectSeed::new(3))
            .compile_control(
                "out = accum(0.12345)",
                SampleRate::new(48_000.0).expect("rate"),
                &[],
            )
            .expect("script");
        builder = builder.script(program, ExecutionScope::Global).modulate(
            (SCRIPT, PortId::FIRST),
            ir.modulations()[0].target(),
            ModulationDepth::new(ModulationUnit::Semitones, 1.0).expect("depth"),
        );
    }
    builder.build().expect("rebuilt IR")
}

fn admitted(ir: &GraphIr) -> CompiledPlan {
    let profile = synth_engine_v2::profile::HostProfile::harness(
        SampleRate::new(48_000.0).expect("rate"),
        FrameCount::new((Q * QUANTA) as u64),
        ChannelLayout::Mono,
    )
    .expect("profile");
    compile(ir, &RenderConfig::new(profile))
        .into_plan()
        .expect("plan")
}
fn events(plan: &CompiledPlan, ir: &GraphIr, oracle: bool, automation: bool) -> Vec<OfflineEvent> {
    let envelope = ir
        .nodes()
        .iter()
        .find(|node| matches!(node.kind(), IrNodeKind::Envelope { .. }))
        .expect("envelope")
        .id();
    let mut events = vec![OfflineEvent::new(
        SampleTime::ZERO,
        CompiledPayload::NoteOn {
            slot: plan.resolve_note(envelope).expect("note target"),
            key: KeyIdentity::new(60).expect("key"),
            velocity: NoteVelocity::FULL,
        },
    )];
    let (node, parameter) = ir.modulations()[0].target();
    let slot = plan.resolve_parameter(node, parameter).expect("cutoff");
    let mut script = 0.0_f32;
    for quantum in 0..QUANTA {
        script += 0.12345;
        if oracle || (automation && quantum == 16) {
            let base = if automation && quantum >= 16 {
                2400.0_f32
            } else {
                1200.0_f32
            };
            // Square LFOs remain +1 for this entire 32Q window at 1 and 2 Hz.
            // Sum in edge order, then apply the semitone law once. Hertz has no
            // narrower domain clamp; these values remain finite and below Nyquist.
            let sum = (-0.5_f32 * 48.0 + 0.7_f32 * 48.0) + script;
            let value = if oracle {
                base * (sum / 12.0).exp2()
            } else {
                base
            };
            events.push(OfflineEvent::new(
                SampleTime::new((quantum * Q) as u64),
                CompiledPayload::SetParameter {
                    slot,
                    value: ParameterValue::new(value).expect("finite"),
                },
            ));
        }
    }
    events
}
fn rendered(plan: &CompiledPlan, events: &[OfflineEvent]) -> Vec<u32> {
    render_offline(
        plan.clone(),
        FrameCount::new((Q * QUANTA) as u64),
        PlanPosition::ZERO,
        events,
    )
    .expect("render")
    .into_iter()
    .map(f32::to_bits)
    .collect()
}

#[test]
fn matrix_grid_and_yams_share_one_sum_law_and_override_order() {
    let ir = authoring();
    assert_eq!(
        ir.modulations().len(),
        2,
        "one saved Matrix and one saved Grid edge"
    );
    assert_eq!(ir.modulations()[0].target(), ir.modulations()[1].target());
    assert_eq!(ir.modulations()[0].target().1, parameters::FILTER_CUTOFF);
    let scopes: Vec<_> = ir
        .modulations()
        .iter()
        .map(|edge| ir.node(edge.source().0).expect("source").scope())
        .collect();
    assert!(scopes.contains(&ExecutionScope::Voice) && scopes.contains(&ExecutionScope::Global));
    assert_eq!(
        ir.modulations()
            .iter()
            .map(|e| e.depth().amount())
            .collect::<Vec<_>>(),
        vec![-0.5_f32 * 48.0, 0.7_f32 * 48.0]
    );
    let modulated = admitted(&rebuilt(&ir, [true; 3]));
    let plain = admitted(&rebuilt(&ir, [false; 3]));
    for automation in [false, true] {
        let actual = rendered(&modulated, &events(&modulated, &ir, false, automation));
        let expected = rendered(&plain, &events(&plain, &ir, true, automation));
        assert!(
            actual
                .iter()
                .filter(|bits| f32::from_bits(**bits) != 0.0)
                .count()
                > actual.len() / 2,
            "audible oracle"
        );
        assert_eq!(
            actual, expected,
            "sum and override order, automation={automation}"
        );
        for missing in 0..3 {
            let mut enabled = [true; 3];
            enabled[missing] = false;
            let plan = admitted(&rebuilt(&ir, enabled));
            assert_ne!(
                actual,
                rendered(&plan, &events(&plan, &ir, false, automation)),
                "contributor {missing} reaches the sound"
            );
        }
    }
}
