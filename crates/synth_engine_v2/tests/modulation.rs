//! `SOUND-INV-027` — the modulation edge and the first native modulator, `P07-S001`.
//!
//! An LFO feeds a declared parameter through a modulation edge whose depth is in the
//! target law's units; the renderer evaluates the source in a pre-pass, composes the sum
//! into the parameter's slot before the quantum's positioned writes are placed, and the
//! kernel reads one resolved value. Nothing here reads a kernel's state: every claim is
//! held on rendered samples, by bits.
//!
//! # The oracle
//!
//! The slot's arithmetic is already held by `parameter_slot`; what this file holds is that
//! the **edge** delivers exactly that arithmetic at exactly the quantum's first frame. The
//! oracle is therefore two renders of plans without an edge: the LFO's own per-frame values,
//! read by patching it into an amplifier's control port over a constant of one; and the
//! target sine driven by a positioned `SetParameter` at every quantum boundary carrying the
//! value the slot composes from that frame — `base × 2^(depth × v / 12)` in the same `f32`
//! steps. A modulated render that is not those bits has moved the composition, the frame,
//! or the value.

mod common;

use common::{OUTPUT, SOURCE, profile};
use synth_engine_v2::diagnostics::CompileError;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrError, IrNodeKind, LfoPolarity, LfoWaveform, ModulationDepth,
    ModulationUnit, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::node::AMPLIFIER_CONTROL;
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::{CompiledPlan, PlanOp};
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, Frequency, NormalizedLevel, ParameterValue, PhaseOffset, Seconds,
};
use synth_engine_v2::render::{AudioBlockMut, Renderer, TimedEvents};
use synth_engine_v2::report::{ResourceAmount, ResourceField};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::stream::StreamControl;
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const LFO: NodeId = NodeId::new(10);
const SECOND_LFO: NodeId = NodeId::new(11);
const CONSTANT: NodeId = NodeId::new(20);
const AMPLIFIER: NodeId = NodeId::new(21);
const ENVELOPE: NodeId = NodeId::new(30);
const Q: u64 = QUANTUM_FRAMES as u64;
const QUANTA: u64 = 64;
const FRAMES: u64 = QUANTA * Q;
const BASE: f32 = 440.0;
const PEAK: f32 = 0.5;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);

fn hz(raw: f32) -> Frequency {
    Frequency::new(raw).expect("finite")
}

fn depth(unit: ModulationUnit, amount: f32) -> ModulationDepth {
    ModulationDepth::new(unit, amount).expect("finite")
}

fn semitones(amount: f32) -> ModulationDepth {
    depth(ModulationUnit::Semitones, amount)
}

fn sine() -> IrNodeKind {
    IrNodeKind::Sine {
        frequency: hz(BASE),
        amplitude: Amplitude::new(PEAK).expect("finite"),
    }
}

fn lfo(waveform: LfoWaveform, rate: f32) -> IrNodeKind {
    IrNodeKind::Lfo {
        waveform,
        rate: hz(rate),
        depth: NormalizedLevel::FULL,
        phase_offset: PhaseOffset::ZERO,
        polarity: LfoPolarity::Bipolar,
    }
}

fn admit(ir: &GraphIr) -> CompiledPlan {
    common::admit(ir, profile(FRAMES, ChannelLayout::Mono))
}

fn refuse(ir: &GraphIr) -> CompileError {
    common::refuse(ir, profile(FRAMES, ChannelLayout::Mono))
}

/// A sine into the output, with the LFOs given and the edges given, all in one scope.
fn modulated(
    scope: ExecutionScope,
    lfos: &[(NodeId, IrNodeKind)],
    edges: &[(NodeId, synth_engine_v2::ir::ParameterId, ModulationDepth)],
) -> GraphIr {
    let mut builder = GraphIr::builder()
        .node(SOURCE, sine(), scope)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        );
    for (id, kind) in lfos {
        builder = builder.node(*id, *kind, scope);
    }
    for (source, parameter, depth) in edges {
        builder = builder.modulate((*source, PortId::FIRST), (SOURCE, *parameter), *depth);
    }
    builder.build().expect("a readable plan")
}

/// The LFO's own per-frame values: patched into an amplifier's control port over a constant
/// of one, so the render **is** the modulator's signal, frame for frame.
fn read_lfo(kind: IrNodeKind) -> Vec<f32> {
    let ir = GraphIr::builder()
        .node(
            CONSTANT,
            IrNodeKind::Constant {
                level: Amplitude::UNITY,
            },
            ExecutionScope::Global,
        )
        .node(LFO, kind, ExecutionScope::Global)
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (CONSTANT, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (LFO, PortId::FIRST),
            (AMPLIFIER, AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan");
    render(&admit(&ir), &[])
}

fn render(plan: &CompiledPlan, events: &[OfflineEvent]) -> Vec<f32> {
    render_offline(
        plan.clone(),
        FrameCount::new(FRAMES),
        PlanPosition::ZERO,
        events,
    )
    .expect("renders")
}

fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|sample| sample.to_bits()).collect()
}

/// The sine alone, driven by one positioned frequency write per quantum boundary carrying
/// `value_at(q)` — the oracle's second half.
fn driven_sine(value_at: impl Fn(u64) -> f32) -> Vec<f32> {
    let plan = admit(&common::source_plan(sine()));
    let frequency = plan
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("the sine declares a frequency");
    let events: Vec<OfflineEvent> = (0..QUANTA)
        .map(|q| {
            OfflineEvent::new(
                SampleTime::new(q * Q),
                CompiledPayload::SetParameter {
                    slot: frequency,
                    value: ParameterValue::new(value_at(q)).expect("finite"),
                },
            )
        })
        .collect();
    render(&plan, &events)
}

/// The slot's semitone law over one contribution, in the slot's own `f32` steps.
fn composed_frequency(base: f32, modulation: f32) -> f32 {
    base * (modulation / 12.0).exp2()
}

#[test]
fn an_lfo_on_the_frequency_is_the_sine_driven_by_the_composed_value_at_each_boundary() {
    // Vibrato at seven semitones from a five-hertz sine: the modulated render is, bit for
    // bit, the sine alone written at every quantum boundary with the value the slot
    // composes from the LFO's first frame of that quantum.
    let modulator = lfo(LfoWaveform::Sine, 5.0);
    let plan = admit(&modulated(
        ExecutionScope::Global,
        &[(LFO, modulator)],
        &[(LFO, parameters::SINE_FREQUENCY, semitones(7.0))],
    ));
    let wave = read_lfo(modulator);
    let vibrato = render(&plan, &[]);
    let driven = driven_sine(|q| composed_frequency(BASE, 7.0 * wave[(q * Q) as usize]));
    assert_eq!(
        bits(&vibrato),
        bits(&driven),
        "the edge delivered another value or frame"
    );

    // The falsifier: the same plan renders the plain sine only where the depth is zero.
    let plain = render(&admit(&common::source_plan(sine())), &[]);
    assert_ne!(
        bits(&vibrato),
        bits(&plain),
        "seven semitones changed nothing"
    );
    let still = admit(&modulated(
        ExecutionScope::Global,
        &[(LFO, modulator)],
        &[(LFO, parameters::SINE_FREQUENCY, semitones(0.0))],
    ));
    assert_eq!(
        bits(&render(&still, &[])),
        bits(&plain),
        "a zero-depth edge is not silence for the base"
    );
}

#[test]
fn a_modulated_render_is_the_same_bits_under_every_host_partition() {
    // The exit gate's cadence shape for a native modulator: a time-varying source, four
    // partitions, one set of bits. A pre-pass that ran per call rather than per quantum, or
    // a composition that read the LFO from the previous call's buffer, fails here.
    let plan = admit(&modulated(
        ExecutionScope::Global,
        &[(LFO, lfo(LfoWaveform::Triangle, 3.0))],
        &[(LFO, parameters::SINE_FREQUENCY, semitones(5.0))],
    ));
    let total = FRAMES as usize;
    let whole = [total];
    let blocks_256 = [256; 16];
    let blocks_64 = [64; 64];
    let irregular = [17, 511, 3, 64, 1024, 1, 100, 2376];
    let reference = render_partitioned(&plan, &whole);
    assert!(
        reference.iter().any(|sample| sample.abs() > 0.1),
        "the render is audible"
    );
    for partition in [&blocks_256[..], &blocks_64[..], &irregular[..]] {
        assert_eq!(
            bits(&render_partitioned(&plan, partition)),
            bits(&reference),
            "partition {partition:?} rendered other bits"
        );
    }
}

/// The renderer driven directly in the caller's blocks, with no events.
fn render_partitioned(plan: &CompiledPlan, partition: &[usize]) -> Vec<f32> {
    let (_control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut out = Vec::with_capacity(FRAMES as usize);
    for block in partition.iter().copied() {
        let mut samples = vec![0.0_f32; block];
        let output =
            AudioBlockMut::new(&mut samples, block, ChannelLayout::Mono).expect("a shaped block");
        renderer
            .render(output, TimedEvents::EMPTY)
            .expect("the block renders");
        out.extend_from_slice(&samples);
    }
    assert_eq!(
        out.len(),
        FRAMES as usize,
        "the partition covers the render"
    );
    out
}

#[test]
fn two_edges_into_one_parameter_sum_in_the_laws_units() {
    // Two LFOs at different rates on one frequency: the sum is gathered edge by edge in
    // identity order, and the oracle takes the same two steps.
    let first = lfo(LfoWaveform::Sine, 5.0);
    let second = lfo(LfoWaveform::Sawtooth, 2.0);
    let plan = admit(&modulated(
        ExecutionScope::Global,
        &[(LFO, first), (SECOND_LFO, second)],
        &[
            (LFO, parameters::SINE_FREQUENCY, semitones(7.0)),
            (SECOND_LFO, parameters::SINE_FREQUENCY, semitones(-3.0)),
        ],
    ));
    let (wave_a, wave_b) = (read_lfo(first), read_lfo(second));
    let both = render(&plan, &[]);
    let driven = driven_sine(|q| {
        let frame = (q * Q) as usize;
        let gathered = (0.0_f32 + 7.0 * wave_a[frame]) + (-3.0 * wave_b[frame]);
        composed_frequency(BASE, gathered)
    });
    assert_eq!(bits(&both), bits(&driven));
}

#[test]
fn an_override_written_under_an_edge_keeps_the_modulation_in_force() {
    // A positioned frequency write to 880 Hz at quantum twenty: from there the vibrato is
    // around 880, not around 440 and not 880 flat. At the write's own quantum both the
    // pre-pass's value and the write land at the first frame, in that order, so the write
    // — composed with the same sum — is what the kernel keeps.
    let modulator = lfo(LfoWaveform::Sine, 5.0);
    let plan = admit(&modulated(
        ExecutionScope::Global,
        &[(LFO, modulator)],
        &[(LFO, parameters::SINE_FREQUENCY, semitones(7.0))],
    ));
    let frequency = plan
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("the sine declares a frequency");
    let wave = read_lfo(modulator);
    let written = render(
        &plan,
        &[OfflineEvent::new(
            SampleTime::new(20 * Q),
            CompiledPayload::SetParameter {
                slot: frequency,
                value: ParameterValue::new(880.0).expect("finite"),
            },
        )],
    );
    let driven = driven_sine(|q| {
        let base = if q < 20 { BASE } else { 880.0 };
        composed_frequency(base, 7.0 * wave[(q * Q) as usize])
    });
    assert_eq!(bits(&written), bits(&driven));
}

#[test]
fn an_edge_into_a_quantum_rate_parameter_retargets_its_segment_at_the_boundary() {
    // The amplitude is quantum-rate and decibel-additive. Its oracle is the sine alone with
    // a quantum-rate write at every boundary carrying `peak × 10^(6 × v / 20)`, applied at
    // the boundary as `apply` applies it — so the segment the kernel reads per frame is the
    // same buffer under both.
    let modulator = lfo(LfoWaveform::Sine, 4.0);
    let plan = admit(&modulated(
        ExecutionScope::Global,
        &[(LFO, modulator)],
        &[(
            LFO,
            parameters::SINE_AMPLITUDE,
            depth(ModulationUnit::Decibels, 6.0),
        )],
    ));
    let wave = read_lfo(modulator);
    let tremolo = render(&plan, &[]);

    let alone = admit(&common::source_plan(sine()));
    let amplitude = alone
        .resolve_parameter(SOURCE, parameters::SINE_AMPLITUDE)
        .expect("the sine declares an amplitude");
    let events: Vec<OfflineEvent> = (0..QUANTA)
        .map(|q| {
            let modulation = 6.0 * wave[(q * Q) as usize];
            OfflineEvent::new(
                SampleTime::new(q * Q),
                CompiledPayload::SetParameter {
                    slot: amplitude,
                    value: ParameterValue::new(PEAK * 10.0_f32.powf(modulation / 20.0))
                        .expect("finite"),
                },
            )
        })
        .collect();
    assert_eq!(bits(&tremolo), bits(&render(&alone, &events)));
    assert_ne!(
        bits(&tremolo),
        bits(&render(&alone, &[])),
        "six decibels changed nothing"
    );
}

#[test]
fn a_plan_without_an_edge_has_no_pre_pass() {
    let plan = admit(&common::source_plan(sine()));
    assert_eq!(plan.prepass_ops(), 0);
    assert!(
        plan.ops()
            .iter()
            .all(|op| !matches!(op, PlanOp::Modulate(_)))
    );
}

#[test]
fn the_pre_pass_holds_the_sources_and_every_composition_ahead_of_the_main_walk() {
    // One source, one edge: the pre-pass is the LFO's step and the one composition; the
    // sine and the output follow. A source scheduled in the main walk, or a composition
    // after its target's step, reads the wrong quantum.
    let plan = admit(&modulated(
        ExecutionScope::Global,
        &[(LFO, lfo(LfoWaveform::Sine, 5.0))],
        &[(LFO, parameters::SINE_FREQUENCY, semitones(7.0))],
    ));
    assert_eq!(plan.prepass_ops(), 2);
    assert!(matches!(plan.ops()[0], PlanOp::Node(_)));
    assert!(matches!(plan.ops()[1], PlanOp::Modulate(step) if step.last()));
    assert!(
        plan.ops()[2..]
            .iter()
            .all(|op| !matches!(op, PlanOp::Modulate(_)))
    );
}

#[test]
fn a_voice_scope_source_is_instantiated_per_voice_and_an_outer_one_is_shared() {
    // Two voices. A voice-scope LFO gives two source steps and two compositions reading two
    // different buffers; an instrument-scope LFO gives one step and two compositions reading
    // one buffer — the master plan's broadcast. Either way the render of two identical,
    // silent-until-played voices is exactly twice the render of one, which is the sum
    // kernel adding equal samples.
    let per_voice = voiced(ExecutionScope::Voice, 2);
    let steps = |plan: &CompiledPlan| -> (usize, Vec<usize>) {
        let pre = &plan.ops()[..plan.prepass_ops()];
        (
            pre.iter()
                .filter(|op| matches!(op, PlanOp::Node(_)))
                .count(),
            pre.iter()
                .filter_map(|op| match op {
                    PlanOp::Modulate(step) => Some(step.source().index()),
                    _ => None,
                })
                .collect(),
        )
    };
    let (sources, reads) = steps(&per_voice);
    assert_eq!(sources, 2, "one LFO step per voice");
    assert_eq!(reads.len(), 2);
    assert_ne!(reads[0], reads[1], "each instance reads its own LFO");

    let shared = voiced(ExecutionScope::InstrumentInstance, 2);
    let (sources, reads) = steps(&shared);
    assert_eq!(sources, 1, "one LFO for the instrument");
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[0], reads[1], "both instances read the one LFO");

    let one = render(&voiced(ExecutionScope::Voice, 1), &[]);
    let two = render(&per_voice, &[]);
    let doubled: Vec<f32> = one.iter().map(|sample| 2.0 * sample).collect();
    assert_eq!(bits(&two), bits(&doubled));
    assert!(one.iter().any(|sample| sample.abs() > 0.1));
}

/// A voice-scope sine modulated by an LFO in `scope`, with `voices` compiled voices.
fn voiced(scope: ExecutionScope, voices: u32) -> CompiledPlan {
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Voice)
        .node(LFO, lfo(LfoWaveform::Sine, 5.0), scope)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(7.0),
        )
        .declaring(common::compiled_notes(voices))
        .build()
        .expect("a readable plan");
    admit(&ir)
}

#[test]
fn the_charges_derive_alike_from_the_ir_and_the_plan_and_cover_what_is_held() {
    // A sample-positioned target in the voice scope receives one control per instance per
    // quantum, and the scratch is charged for it; a quantum-rate target receives none.
    for voices in [1_u32, 2, 8] {
        let ir = GraphIr::builder()
            .node(SOURCE, sine(), ExecutionScope::Voice)
            .node(LFO, lfo(LfoWaveform::Sine, 5.0), ExecutionScope::Voice)
            .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
            .connect(
                (SOURCE, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            )
            .modulate(
                (LFO, PortId::FIRST),
                (SOURCE, parameters::SINE_FREQUENCY),
                semitones(7.0),
            )
            .modulate(
                (LFO, PortId::FIRST),
                (SOURCE, parameters::SINE_AMPLITUDE),
                depth(ModulationUnit::Decibels, 3.0),
            )
            .declaring(common::compiled_notes(voices))
            .build()
            .expect("a readable plan");
        let host = profile(FRAMES, ChannelLayout::Mono);
        let plan = common::admit(&ir, host);
        assert_eq!(ir.modulated_sample_positioned_rows(), voices);
        assert_eq!(
            plan.modulated_sample_positioned_rows(),
            ir.modulated_sample_positioned_rows()
        );
        let (_control, renderer) = StreamControl::open(plan, ORIGIN).expect("the stream opens");
        let charged = synth_engine_v2::render::timed_control_scratch_bytes(
            host.limits().events().max_events_per_quantum(),
            renderer.prepared_record_count(),
            synth_engine_v2::quantities::HeldNoteCount::measured(voices),
            ir.max_writes_per_note()
                .fanned_out(ir.sample_positioned_fan_out())
                .widest(ir.steal_expansion()),
            ir.modulated_sample_positioned_rows(),
            ir.voice_instances(),
        );
        let held = renderer.control_scratch_bytes();
        assert!(
            charged >= held as u64,
            "{voices} voices: charged {charged} bytes of control scratch, holds {held}"
        );
    }
}

#[test]
fn edges_into_a_voice_parameter_are_admitted_as_slots_per_voice() {
    // Two edges into voice-scope targets request two slots; an edge into a global target
    // requests none of this row.
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Voice)
        .node(LFO, lfo(LfoWaveform::Sine, 5.0), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(7.0),
        )
        .modulate(
            (LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_AMPLITUDE),
            depth(ModulationUnit::Decibels, 3.0),
        )
        .build()
        .expect("a readable plan");
    let outcome = synth_engine_v2::compile::compile(
        &ir,
        &synth_engine_v2::compile::RenderConfig::new(profile(FRAMES, ChannelLayout::Mono)),
    );
    let row = outcome
        .report()
        .row(ResourceField::ModMatrixSlotsPerVoice)
        .expect("the row is reported");
    assert_eq!(
        row.requested(),
        ResourceAmount::Slots(synth_engine_v2::quantities::SlotCount::measured(2))
    );
    assert_eq!(
        row.contributor(),
        synth_engine_v2::ir::IrObject::Node(SOURCE),
        "attributed to the target node"
    );
    let outcome = synth_engine_v2::compile::compile(
        &modulated(
            ExecutionScope::Global,
            &[(LFO, lfo(LfoWaveform::Sine, 5.0))],
            &[(LFO, parameters::SINE_FREQUENCY, semitones(7.0))],
        ),
        &synth_engine_v2::compile::RenderConfig::new(profile(FRAMES, ChannelLayout::Mono)),
    );
    let row = outcome
        .report()
        .row(ResourceField::ModMatrixSlotsPerVoice)
        .expect("the row is reported");
    assert_eq!(
        row.requested(),
        ResourceAmount::Slots(synth_engine_v2::quantities::SlotCount::measured(0))
    );
}

// --- refusals, each by name ---------------------------------------------------------------

#[test]
fn a_depth_in_another_unit_than_the_targets_law_is_refused() {
    let error = refuse(&modulated(
        ExecutionScope::Global,
        &[(LFO, lfo(LfoWaveform::Sine, 5.0))],
        &[(
            LFO,
            parameters::SINE_FREQUENCY,
            depth(ModulationUnit::Decibels, 6.0),
        )],
    ));
    assert!(
        matches!(
            error,
            CompileError::ModulationUnitMismatch {
                node,
                declared: ModulationUnit::Decibels,
                expected: ModulationUnit::Semitones,
                ..
            } if node == SOURCE
        ),
        "{error:?}"
    );
}

#[test]
fn a_parameter_the_target_does_not_declare_is_refused() {
    let error = refuse(&modulated(
        ExecutionScope::Global,
        &[(LFO, lfo(LfoWaveform::Sine, 5.0))],
        &[(
            LFO,
            synth_engine_v2::ir::ParameterId::new(9),
            semitones(1.0),
        )],
    ));
    assert!(
        matches!(error, CompileError::ModulationUnknownParameter { node, .. } if node == SOURCE),
        "{error:?}"
    );
}

#[test]
fn a_source_port_that_is_not_a_control_signal_is_refused() {
    // An audio output as a modulation source: reading its first frame as a control value
    // would be a downsampling law nobody declared.
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Global)
        .node(SECOND_LFO, sine(), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (SECOND_LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(1.0),
        )
        .build()
        .expect("a readable plan");
    let error = refuse(&ir);
    assert!(
        matches!(
            error,
            CompileError::ModulationSourceNotControl {
                node,
                domain: SignalDomain::Audio,
                ..
            } if node == SECOND_LFO
        ),
        "{error:?}"
    );
}

#[test]
fn a_source_with_a_sample_positioned_control_cannot_run_ahead_and_is_refused() {
    // The envelope's output is a control signal, but its gate is sample-positioned, and a
    // source is evaluated before the quantum's positioned writes exist. Refused by name
    // rather than evaluated a quantum late; the slice that lowers V1's envelope source owns
    // the split collection that would let it run.
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Global)
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: NormalizedLevel::FULL,
            },
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (ENVELOPE, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(1.0),
        )
        .build()
        .expect("a readable plan");
    let error = refuse(&ir);
    assert!(
        matches!(error, CompileError::ModulationSourceNotAhead { node, .. } if node == ENVELOPE),
        "{error:?}"
    );
}

#[test]
fn a_port_the_source_does_not_declare_is_refused() {
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Global)
        .node(LFO, lfo(LfoWaveform::Sine, 5.0), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (LFO, PortId::new(3)),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(1.0),
        )
        .build()
        .expect("a readable plan");
    let error = refuse(&ir);
    assert!(
        matches!(error, CompileError::ModulationUnknownPort { node, port, .. } if node == LFO && port == PortId::new(3)),
        "{error:?}"
    );
}

#[test]
fn a_source_inside_its_targets_scope_is_refused_for_want_of_a_reduction() {
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Global)
        .node(LFO, lfo(LfoWaveform::Sine, 5.0), ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(1.0),
        )
        .build()
        .expect("a readable plan");
    let error = refuse(&ir);
    assert!(
        matches!(
            error,
            CompileError::ModulationScopeCrossing {
                source_scope: ExecutionScope::Voice,
                target_scope: ExecutionScope::Global,
                ..
            }
        ),
        "{error:?}"
    );
    // The other way is the broadcast the master plan allows.
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Voice)
        .node(LFO, lfo(LfoWaveform::Sine, 5.0), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(1.0),
        )
        .build()
        .expect("a readable plan");
    let _ = admit(&ir);
}

#[test]
fn a_voice_scope_source_also_cabled_outside_its_scope_is_refused() {
    // Summed across voices for the outer reader, the source's sum steps would sit in the
    // pre-pass beyond a steal's fade. Refused by name; the same source cabled inside the
    // scope is fine.
    let cabled_out = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Voice)
        .node(LFO, lfo(LfoWaveform::Sine, 5.0), ExecutionScope::Voice)
        .node(
            CONSTANT,
            IrNodeKind::Constant {
                level: Amplitude::UNITY,
            },
            ExecutionScope::Global,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (CONSTANT, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (LFO, PortId::FIRST),
            (AMPLIFIER, AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(1.0),
        )
        .declaring(common::compiled_notes(2))
        .build()
        .expect("a readable plan");
    let error = refuse(&cabled_out);
    assert!(
        matches!(error, CompileError::ModulationSourceReadOutsideScope { node, .. } if node == LFO),
        "{error:?}"
    );

    let cabled_in = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Voice)
        .node(LFO, lfo(LfoWaveform::Sine, 5.0), ExecutionScope::Voice)
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (LFO, PortId::FIRST),
            (AMPLIFIER, AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(1.0),
        )
        .declaring(common::compiled_notes(2))
        .build()
        .expect("a readable plan");
    let plan = admit(&cabled_in);
    // A source cabled inside its scope is a pre-pass step whose buffer the main walk reads:
    // the render is tremolo and vibrato at once, and audible.
    assert!(render(&plan, &[]).iter().any(|sample| sample.abs() > 0.1));
}

#[test]
fn a_random_shape_is_refused_until_a_seed_exists() {
    // `P06-R001`: no node may consume randomness before ADR-0008 decides what a seed is.
    // Refused whether or not an edge reads the node.
    for waveform in [LfoWaveform::SampleAndHold, LfoWaveform::SmoothRandom] {
        let error = refuse(&modulated(
            ExecutionScope::Global,
            &[(LFO, lfo(waveform, 5.0))],
            &[],
        ));
        assert!(
            matches!(error, CompileError::SeedlessRandomWaveform { node, waveform: named } if node == LFO && named == waveform),
            "{error:?}"
        );
    }
}

#[test]
fn two_sources_modulating_each_others_rate_are_a_cycle() {
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Global)
        .node(LFO, lfo(LfoWaveform::Sine, 5.0), ExecutionScope::Global)
        .node(
            SECOND_LFO,
            lfo(LfoWaveform::Sine, 3.0),
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (LFO, PortId::FIRST),
            (SECOND_LFO, parameters::LFO_RATE),
            semitones(1.0),
        )
        .modulate(
            (SECOND_LFO, PortId::FIRST),
            (LFO, parameters::LFO_RATE),
            semitones(1.0),
        )
        .build()
        .expect("a readable plan");
    let error = refuse(&ir);
    assert!(
        matches!(error, CompileError::ModulationCycle { .. }),
        "{error:?}"
    );
}

#[test]
fn an_edge_naming_a_node_the_plan_lacks_or_leaving_the_output_cannot_be_built() {
    let dangling = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .modulate(
            (LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(1.0),
        )
        .build();
    assert!(
        matches!(dangling, Err(IrError::UnknownModulationNode { node, .. }) if node == LFO),
        "{dangling:?}"
    );
    let from_output = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .modulate(
            (OUTPUT, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(1.0),
        )
        .build();
    assert!(
        matches!(from_output, Err(IrError::ModulationNotFromASource { node, .. }) if node == OUTPUT),
        "{from_output:?}"
    );
}

#[test]
fn a_source_modulating_another_sources_rate_is_composed_before_that_source_runs() {
    // A→B.rate, B→sine.frequency: the pre-pass is A's step, the composition into B's rate,
    // B's step, the composition into the frequency — and B's cycle speeds up under A, which
    // the render shows against B alone.
    let ir = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Global)
        .node(LFO, lfo(LfoWaveform::Sine, 0.0), ExecutionScope::Global)
        .node(
            SECOND_LFO,
            lfo(LfoWaveform::Sine, 3.0),
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (SECOND_LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(7.0),
        )
        .modulate(
            (LFO, PortId::FIRST),
            (SECOND_LFO, parameters::LFO_RATE),
            semitones(12.0),
        )
        .build()
        .expect("a readable plan");
    let plan = admit(&ir);
    assert_eq!(plan.prepass_ops(), 4);
    let shape: Vec<&str> = plan.ops()[..4]
        .iter()
        .map(|op| match op {
            PlanOp::Node(_) => "node",
            PlanOp::Modulate(_) => "modulate",
            PlanOp::Output { .. } => "output",
        })
        .collect();
    assert_eq!(shape, ["node", "modulate", "node", "modulate"]);

    // A at rate zero and phase zero is a constant zero, so B runs at its authored rate and
    // the render is B alone on the frequency; a phase offset of a quarter makes A a constant
    // one, +12 semitones on B's rate, and B runs at twice its rate.
    let with_quarter = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Global)
        .node(
            LFO,
            IrNodeKind::Lfo {
                waveform: LfoWaveform::Sine,
                rate: hz(0.0),
                depth: NormalizedLevel::FULL,
                phase_offset: PhaseOffset::new(0.25).expect("in range"),
                polarity: LfoPolarity::Bipolar,
            },
            ExecutionScope::Global,
        )
        .node(
            SECOND_LFO,
            lfo(LfoWaveform::Sine, 3.0),
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (SECOND_LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            semitones(7.0),
        )
        .modulate(
            (LFO, PortId::FIRST),
            (SECOND_LFO, parameters::LFO_RATE),
            semitones(12.0),
        )
        .build()
        .expect("a readable plan");
    let doubled_rate = modulated(
        ExecutionScope::Global,
        &[(SECOND_LFO, lfo(LfoWaveform::Sine, 6.0))],
        &[(SECOND_LFO, parameters::SINE_FREQUENCY, semitones(7.0))],
    );
    let plain_rate = modulated(
        ExecutionScope::Global,
        &[(SECOND_LFO, lfo(LfoWaveform::Sine, 3.0))],
        &[(SECOND_LFO, parameters::SINE_FREQUENCY, semitones(7.0))],
    );
    assert_eq!(
        bits(&render(&plan, &[])),
        bits(&render(&admit(&plain_rate), &[])),
        "a zero source leaves B at its authored rate"
    );
    assert_eq!(
        bits(&render(&admit(&with_quarter), &[])),
        bits(&render(&admit(&doubled_rate), &[])),
        "+12 semitones on B's rate is B at twice its rate"
    );
}

#[test]
fn the_lfo_is_discoverable_with_its_two_quantum_rate_controls() {
    use synth_engine_v2::node::{NodeKindId, catalog};
    let entry = catalog()
        .into_iter()
        .find(|entry| entry.id == NodeKindId::Lfo)
        .expect("the LFO is discoverable");
    assert_eq!(entry.name, "lfo");
    assert_eq!(entry.parameters.len(), 2);
    assert!(
        entry
            .parameters
            .iter()
            .all(|parameter| parameter.rate == synth_engine_v2::plan::ControlRate::Quantum)
    );
    assert!(!entry.playable);
    assert_eq!(entry.ports.len(), 1);
    assert_eq!(entry.ports[0].domain, SignalDomain::Control);
}

// --- `P07-S002a`: the filter's and the envelope's controls ---------------------------------

const FILTER: NodeId = NodeId::new(40);
const CUTOFF: f32 = 1_000.0;

/// A sawtooth into a low-pass into the output, all in one scope, with an LFO and the edges
/// given: a shape with harmonics for the corner to act on.
fn filtered(
    lfos: &[(NodeId, IrNodeKind)],
    edges: &[(NodeId, synth_engine_v2::ir::ParameterId, ModulationDepth)],
) -> GraphIr {
    let mut builder = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Saw {
                frequency: hz(110.0),
                amplitude: Amplitude::new(PEAK).expect("finite"),
            },
            ExecutionScope::Global,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: synth_engine_v2::quantities::CutoffFrequency::new(CUTOFF)
                    .expect("positive"),
                resonance: synth_engine_v2::quantities::Resonance::BUTTERWORTH,
            },
            ExecutionScope::Global,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        );
    for (id, kind) in lfos {
        builder = builder.node(*id, *kind, ExecutionScope::Global);
    }
    for (source, parameter, depth) in edges {
        builder = builder.modulate((*source, PortId::FIRST), (FILTER, *parameter), *depth);
    }
    builder.build().expect("a readable plan")
}

/// The filtered sawtooth alone, driven by one quantum-rate write per boundary on `parameter`
/// carrying `value_at(q)`.
fn driven_filter(
    parameter: synth_engine_v2::ir::ParameterId,
    value_at: impl Fn(u64) -> f32,
) -> Vec<f32> {
    let plan = admit(&filtered(&[], &[]));
    let slot = plan
        .resolve_parameter(FILTER, parameter)
        .expect("the filter declares it");
    let events: Vec<OfflineEvent> = (0..QUANTA)
        .map(|q| {
            OfflineEvent::new(
                SampleTime::new(q * Q),
                CompiledPayload::SetParameter {
                    slot,
                    value: ParameterValue::new(value_at(q)).expect("finite"),
                },
            )
        })
        .collect();
    render(&plan, &events)
}

#[test]
fn an_lfo_on_the_filters_cutoff_is_the_filter_driven_by_the_composed_corner_at_each_boundary() {
    // The corner is quantum-rate under the semitone law: the kernel re-derives its
    // coefficients at the first frame of every quantum the corner moves, and the oracle is
    // the filter alone written the composed corner at every boundary.
    let modulator = lfo(LfoWaveform::Sine, 3.0);
    let plan = admit(&filtered(
        &[(LFO, modulator)],
        &[(LFO, parameters::FILTER_CUTOFF, semitones(24.0))],
    ));
    let wave = read_lfo(modulator);
    let swept = render(&plan, &[]);
    let driven = driven_filter(parameters::FILTER_CUTOFF, |q| {
        composed_frequency(CUTOFF, 24.0 * wave[(q * Q) as usize])
    });
    assert_eq!(bits(&swept), bits(&driven));
    let still = render(&admit(&filtered(&[], &[])), &[]);
    assert_ne!(
        bits(&swept),
        bits(&still),
        "two octaves of sweep changed nothing"
    );
    assert!(still.iter().any(|sample| sample.abs() > 0.05));
}

#[test]
fn an_lfo_on_the_filters_resonance_is_the_filter_driven_by_the_composed_quality() {
    // A triangle started a quarter turn in, so it rises from zero through the render and the
    // quality moves from its authored value upward rather than below zero, where it is held.
    let modulator = IrNodeKind::Lfo {
        waveform: LfoWaveform::Triangle,
        rate: hz(2.0),
        depth: NormalizedLevel::FULL,
        phase_offset: PhaseOffset::new(0.25).expect("in range"),
        polarity: LfoPolarity::Bipolar,
    };
    let plan = admit(&filtered(
        &[(LFO, modulator)],
        &[(
            LFO,
            parameters::FILTER_RESONANCE,
            depth(ModulationUnit::Physical, 4.0),
        )],
    ));
    let wave = read_lfo(modulator);
    let swept = render(&plan, &[]);
    let base = synth_engine_v2::quantities::Resonance::BUTTERWORTH.as_f32();
    let driven = driven_filter(parameters::FILTER_RESONANCE, |q| {
        base + 4.0 * wave[(q * Q) as usize]
    });
    assert_eq!(bits(&swept), bits(&driven));
    assert_ne!(
        bits(&swept),
        bits(&render(&admit(&filtered(&[], &[])), &[])),
        "the quality's sweep changed nothing"
    );
}

#[test]
fn a_corner_with_no_usable_filter_holds_the_coefficients_in_force() {
    // Two hundred semitones on a kilohertz is far past Nyquist, and a quality moved below
    // zero has no filter: the kernel keeps the coefficients it had, so the render is the
    // unmodulated filter's, bit for bit, rather than a filter nobody asked for or a `NaN`.
    let still = render(&admit(&filtered(&[], &[])), &[]);
    let constant = IrNodeKind::Lfo {
        waveform: LfoWaveform::Sine,
        rate: hz(0.0),
        depth: NormalizedLevel::FULL,
        phase_offset: PhaseOffset::new(0.25).expect("in range"),
        polarity: LfoPolarity::Bipolar,
    };
    let past_nyquist = admit(&filtered(
        &[(LFO, constant)],
        &[(LFO, parameters::FILTER_CUTOFF, semitones(200.0))],
    ));
    assert_eq!(bits(&render(&past_nyquist, &[])), bits(&still));
    let below_zero = admit(&filtered(
        &[(LFO, constant)],
        &[(
            LFO,
            parameters::FILTER_RESONANCE,
            depth(ModulationUnit::Physical, -5.0),
        )],
    ));
    assert_eq!(bits(&render(&below_zero, &[])), bits(&still));
    // A corner near Nyquist with a quality in the tens of millions passes the range tests
    // and fails Jury's criterion on the rounded coefficients, which preparation refuses as
    // unstable; the kernel holds. Both edges move from the first quantum, so the pair is
    // never usable and the unmodulated coefficients stand throughout.
    let unstable = admit(&filtered(
        &[(LFO, constant)],
        &[
            (LFO, parameters::FILTER_CUTOFF, semitones(52.0)),
            (
                LFO,
                parameters::FILTER_RESONANCE,
                depth(ModulationUnit::Physical, 5.4e7),
            ),
        ],
    ));
    assert_eq!(bits(&render(&unstable, &[])), bits(&still));
    // The same corner at the authored quality is usable, so the hold was the quality's.
    let near_nyquist = admit(&filtered(
        &[(LFO, constant)],
        &[(LFO, parameters::FILTER_CUTOFF, semitones(52.0))],
    ));
    assert_ne!(bits(&render(&near_nyquist, &[])), bits(&still));
    assert!(
        render(&past_nyquist, &[])
            .iter()
            .all(|sample| sample.is_finite())
    );
}

/// A played voice — sine, envelope, amplifier — with an LFO and the edges given into the
/// envelope, one compiled voice, twelve-tone tuning.
fn played_envelope(
    attack: f32,
    lfos: &[(NodeId, IrNodeKind)],
    edges: &[(NodeId, synth_engine_v2::ir::ParameterId, ModulationDepth)],
) -> CompiledPlan {
    let mut builder = GraphIr::builder()
        .node(SOURCE, sine(), ExecutionScope::Voice)
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(attack).expect("finite"),
                decay: Seconds::new(0.005).expect("finite"),
                sustain: NormalizedLevel::new(0.5).expect("in range"),
                release: Seconds::new(0.01).expect("finite"),
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
            (AMPLIFIER, AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(ExecutionScope::Voice, common::twelve_tet())
        .declaring(common::compiled_notes(1));
    for (id, kind) in lfos {
        builder = builder.node(*id, *kind, ExecutionScope::InstrumentInstance);
    }
    for (source, parameter, depth) in edges {
        builder = builder.modulate((*source, PortId::FIRST), (ENVELOPE, *parameter), *depth);
    }
    admit(&builder.build().expect("a readable plan"))
}

/// Three notes across the render, each held fourteen quanta, as offline events.
///
/// Fourteen quanta is 896 frames: past the longest attack the tests modulate (nine
/// milliseconds, 432 frames) plus the five-millisecond decay (240), so every note reaches
/// its sustain and holds it before the release — an independent read found the notes
/// released mid-decay, with the sustain never held.
fn three_notes(plan: &CompiledPlan) -> Vec<OfflineEvent> {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let mut events = Vec::new();
    for start in [2_u64, 20, 40] {
        events.push(OfflineEvent::new(
            SampleTime::new(start * Q + 7),
            CompiledPayload::NoteOn {
                slot,
                key: common::any_key(),
                velocity: synth_engine_v2::quantities::NoteVelocity::FULL,
            },
        ));
        events.push(OfflineEvent::new(
            SampleTime::new((start + 14) * Q + 7),
            CompiledPayload::NoteOff {
                slot,
                key: common::any_key(),
            },
        ));
    }
    events
}

#[test]
fn an_lfo_on_the_envelopes_attack_and_sustain_is_the_envelope_driven_at_each_boundary() {
    // The attack is read where a gate rises, the sustain where it is held; both are
    // quantum-rate under the physical and the normalized law. The oracle is the same voice
    // written the composed values at every boundary, playing the same three notes.
    let modulator = lfo(LfoWaveform::Sine, 0.7);
    let plan = played_envelope(
        0.005,
        &[(LFO, modulator)],
        &[
            (
                LFO,
                parameters::ENVELOPE_ATTACK,
                depth(ModulationUnit::Physical, 0.004),
            ),
            (
                LFO,
                parameters::ENVELOPE_SUSTAIN,
                depth(ModulationUnit::Normalized, 0.4),
            ),
        ],
    );
    let wave = read_lfo(modulator);
    let shaped = render(&plan, &three_notes(&plan));

    let alone = played_envelope(0.005, &[], &[]);
    let attack = alone
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_ATTACK)
        .expect("declared");
    let sustain = alone
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_SUSTAIN)
        .expect("declared");
    let mut events = three_notes(&alone);
    for q in 0..QUANTA {
        let v = wave[(q * Q) as usize];
        events.push(OfflineEvent::new(
            SampleTime::new(q * Q),
            CompiledPayload::SetParameter {
                slot: attack,
                value: ParameterValue::new(0.005 + 0.004 * v).expect("finite"),
            },
        ));
        events.push(OfflineEvent::new(
            SampleTime::new(q * Q),
            CompiledPayload::SetParameter {
                slot: sustain,
                value: ParameterValue::new((0.5_f32 + 0.4 * v).clamp(0.0, 1.0)).expect("finite"),
            },
        ));
    }
    events.sort_by_key(|event| event.time());
    let driven = render(&alone, &events);
    assert_eq!(bits(&shaped), bits(&driven));
    assert_ne!(
        bits(&shaped),
        bits(&render(&alone, &three_notes(&alone))),
        "the modulation changed nothing"
    );
    assert!(shaped.iter().any(|sample| sample.abs() > 0.1));
    // Each control on its own moves the render, so neither could be ignored behind the other.
    for (parameter, unit, amount) in [
        (parameters::ENVELOPE_ATTACK, ModulationUnit::Physical, 0.004),
        (
            parameters::ENVELOPE_SUSTAIN,
            ModulationUnit::Normalized,
            0.4,
        ),
    ] {
        let one = played_envelope(
            0.005,
            &[(LFO, modulator)],
            &[(LFO, parameter, depth(unit, amount))],
        );
        assert_ne!(
            bits(&render(&one, &three_notes(&one))),
            bits(&render(&alone, &three_notes(&alone))),
            "{parameter:?} changed nothing"
        );
    }
}

#[test]
fn a_duration_moved_below_zero_is_an_instant_segment() {
    // The slot holds a duration at or above zero: an attack of fifty milliseconds moved by
    // minus one second is the instant attack, bit for bit.
    let constant = IrNodeKind::Lfo {
        waveform: LfoWaveform::Sine,
        rate: hz(0.0),
        depth: NormalizedLevel::FULL,
        phase_offset: PhaseOffset::new(0.25).expect("in range"),
        polarity: LfoPolarity::Bipolar,
    };
    let negative = played_envelope(
        0.05,
        &[(LFO, constant)],
        &[(
            LFO,
            parameters::ENVELOPE_ATTACK,
            depth(ModulationUnit::Physical, -1.0),
        )],
    );
    let instant = played_envelope(0.0, &[], &[]);
    assert_eq!(
        bits(&render(&negative, &three_notes(&negative))),
        bits(&render(&instant, &three_notes(&instant)))
    );
}

#[test]
fn the_filter_and_the_envelope_are_discoverable_with_their_new_controls() {
    use synth_engine_v2::node::{NodeKindId, catalog};
    let entries = catalog();
    let filter = entries
        .iter()
        .find(|entry| entry.id == NodeKindId::Filter)
        .expect("discoverable");
    let names: Vec<&str> = filter.parameters.iter().map(|p| p.name).collect();
    assert_eq!(names, ["cutoff", "resonance"]);
    let envelope = entries
        .iter()
        .find(|entry| entry.id == NodeKindId::Envelope)
        .expect("discoverable");
    let names: Vec<&str> = envelope.parameters.iter().map(|p| p.name).collect();
    assert_eq!(
        names,
        [
            "gate",
            "velocity",
            "velocity_sensitivity",
            "attack",
            "decay",
            "sustain",
            "release"
        ]
    );
}
