//! `SOUND-INV-027` inside the crate: the LFO kernel's shapes, the pre-pass's reset, and
//! the one-advance rule, `P07-S001`.
//!
//! In-crate for the reasons `kernel_tests` and `voice_tests` are: a kernel's shape is only
//! observable by calling it, a modulator's phase after a steal is a state record the public
//! API does not expose, and the slot's smoothing seam is test-only. `tests/modulation.rs`
//! holds everything the public API can hold, on rendered bits.

use crate::compile::{RenderConfig, compile};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, LfoPolarity, LfoWaveform, ModulationDepth, ModulationUnit,
    NodeId, PortId, SignalDomain, StealingPolicy, parameters,
};
use crate::node::AMPLIFIER_CONTROL;
use crate::node::kernels::{
    ControlIndex, InputBuffer, MAX_INPUTS, NodeIo, NodeState, PreparedNode, TimedControl, lfo,
};
use crate::offline::render_offline;
use crate::plan::CompiledPlan;
use crate::profile::HostProfile;
use crate::publish::PublicationArbiter;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, Frequency, HeldNoteCount, KeyIdentity, NormalizedLevel,
    NoteVelocity, ParameterValue, PhaseOffset, SampleRate, Seconds,
};
use crate::render::{AudioBlockMut, PreparedRenderer};
use crate::schedule::{AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent};
use crate::stream::StreamControl;
use crate::time::{
    FrameCount, PlanPosition, QUANTUM_FRAMES, QuantumOffset, SampleTime, StreamAnchor,
};

const Q: usize = QUANTUM_FRAMES as usize;
const RATE: f32 = 48_000.0;
const BLOCK: usize = 256;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);

const SOURCE: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);
const LFO: NodeId = NodeId::new(10);
const SECOND_LFO: NodeId = NodeId::new(11);
const CONSTANT: NodeId = NodeId::new(20);

fn prepared(waveform: LfoWaveform, polarity: LfoPolarity, offset: f32) -> PreparedNode {
    PreparedNode::Lfo {
        seconds_per_frame: 1.0 / f64::from(RATE),
        waveform,
        polarity,
        phase_offset: f64::from(offset),
        rate: Frequency::ONE,
        depth: NormalizedLevel::FULL,
    }
}

/// One quantum of the kernel, at `rate` hertz and `depth`, with the controls given.
fn run(
    prepared: &PreparedNode,
    state: &mut NodeState,
    rate: f32,
    depth: f32,
    controls: &[TimedControl],
) -> Vec<f32> {
    let mut out = vec![0.0; Q];
    let mut ramps = vec![rate; Q];
    ramps.extend(std::iter::repeat_n(depth, Q));
    let mut io = NodeIo {
        out: &mut out,
        channels: ChannelLayout::Mono,
        inputs: [InputBuffer::Unpatched; MAX_INPUTS],
        position: None,
        controls,
        ramps: &ramps,
        samples: &[],
        scripts: crate::script::ScriptResources::default(),
    };
    lfo(prepared, state, &mut io);
    out
}

/// One period per quantum: frame `k` reads position `k / Q`.
const PERIOD_PER_QUANTUM: f32 = RATE / Q as f32;

#[test]
fn each_deterministic_shape_traces_v1s_arithmetic_over_one_period() {
    // At one period per quantum frame `k` is position `k / 64`, so the quarter, half and
    // three-quarter points are frames 16, 32 and 48 — where the four shapes differ most.
    let cases: [(LfoWaveform, [f32; 4]); 4] = [
        (LfoWaveform::Sine, [0.0, 1.0, 0.0, -1.0]),
        (LfoWaveform::Triangle, [-1.0, 0.0, 1.0, 0.0]),
        (LfoWaveform::Sawtooth, [-1.0, -0.5, 0.0, 0.5]),
        (LfoWaveform::Square, [1.0, 1.0, -1.0, -1.0]),
    ];
    for (waveform, expected) in cases {
        let prepared = prepared(waveform, LfoPolarity::Bipolar, 0.0);
        let mut state = NodeState::initial(&prepared);
        let out = run(&prepared, &mut state, PERIOD_PER_QUANTUM, 1.0, &[]);
        for (frame, want) in [0, 16, 32, 48].into_iter().zip(expected) {
            assert!(
                (out[frame] - want).abs() < 1e-6,
                "{waveform:?} frame {frame}: {} against {want}",
                out[frame]
            );
        }
        // And the phase came back to where it started, one period on.
        let NodeState::Lfo { phase } = state else {
            panic!("an LFO keeps its phase")
        };
        assert!(phase.abs() < 1e-9 || (1.0 - phase) < 1e-9, "phase {phase}");
    }
}

#[test]
fn the_unipolar_polarity_folds_the_shape_and_the_depth_scales_it() {
    let prepared = prepared(LfoWaveform::Sine, LfoPolarity::Unipolar, 0.0);
    let mut state = NodeState::initial(&prepared);
    let out = run(&prepared, &mut state, PERIOD_PER_QUANTUM, 0.5, &[]);
    // Bipolar 0, 1, 0, −1 folds to 0.5, 1, 0.5, 0; at half depth, 0.25, 0.5, 0.25, 0.
    for (frame, want) in [0, 16, 32, 48].into_iter().zip([0.25, 0.5, 0.25, 0.0]) {
        assert!(
            (out[frame] - want).abs() < 1e-6,
            "frame {frame}: {} against {want}",
            out[frame]
        );
    }
}

#[test]
fn the_phase_offset_is_applied_where_the_shape_is_read_and_a_reset_restarts_the_cycle() {
    // A quarter turn puts the sine's peak at the first frame; a reset at frame 40 puts it
    // there again, which is the offset surviving the reset rather than being consumed by it.
    let prepared = prepared(LfoWaveform::Sine, LfoPolarity::Bipolar, 0.25);
    let mut state = NodeState::initial(&prepared);
    let reset = [TimedControl {
        offset: QuantumOffset::new(40).expect("inside the quantum"),
        control: ControlIndex::RESET,
        value: ParameterValue::ZERO,
    }];
    let out = run(&prepared, &mut state, PERIOD_PER_QUANTUM, 1.0, &reset);
    assert!(
        (out[0] - 1.0).abs() < 1e-6,
        "the offset puts the peak first"
    );
    assert!((out[16]).abs() < 1e-6, "a quarter on, the zero crossing");
    assert!(
        (out[40] - 1.0).abs() < 1e-6,
        "the reset restarts the cycle: {}",
        out[40]
    );
    assert!((out[56]).abs() < 1e-6, "and the cycle continues from there");
}

#[test]
fn the_rate_is_read_per_frame_from_the_ramps() {
    // Zero for the first half, then one period per quantum: the phase stays put, then
    // moves. A rate read once per quantum would move from the first frame.
    let prepared = prepared(LfoWaveform::Sawtooth, LfoPolarity::Bipolar, 0.0);
    let mut state = NodeState::initial(&prepared);
    let mut out = vec![0.0; Q];
    let mut ramps = vec![0.0_f32; Q / 2];
    ramps.extend(std::iter::repeat_n(PERIOD_PER_QUANTUM, Q / 2));
    ramps.extend(std::iter::repeat_n(1.0, Q));
    let mut io = NodeIo {
        out: &mut out,
        channels: ChannelLayout::Mono,
        inputs: [InputBuffer::Unpatched; MAX_INPUTS],
        position: None,
        controls: &[],
        ramps: &ramps,
        samples: &[],
        scripts: crate::script::ScriptResources::default(),
    };
    lfo(&prepared, &mut state, &mut io);
    assert!(
        out[..Q / 2].iter().all(|s| (*s + 1.0).abs() < 1e-6),
        "held at the start"
    );
    assert!(
        (out[Q / 2 + 16] + 0.5).abs() < 1e-6,
        "then a quarter period on: {}",
        out[Q / 2 + 16]
    );
}

#[test]
fn a_random_shape_reaches_the_kernel_as_silence() {
    // Validation refuses it; the arm exists because the match is exhaustive, and it must
    // not invent a stream.
    for waveform in [LfoWaveform::SampleAndHold, LfoWaveform::SmoothRandom] {
        let prepared = prepared(waveform, LfoPolarity::Bipolar, 0.0);
        let mut state = NodeState::initial(&prepared);
        let out = run(&prepared, &mut state, PERIOD_PER_QUANTUM, 1.0, &[]);
        assert!(out.iter().all(|s| *s == 0.0), "{waveform:?}");
    }
}

// --- the pre-pass ---------------------------------------------------------------------------

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(RATE).expect("valid rate"),
        FrameCount::new(BLOCK as u64),
        ChannelLayout::Mono,
    )
    .expect("valid harness profile")
}

fn admit(ir: &GraphIr) -> CompiledPlan {
    compile(ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect("the plan fits this profile")
}

fn constant_source(offset: f32) -> IrNodeKind {
    IrNodeKind::Lfo {
        waveform: LfoWaveform::Sine,
        rate: Frequency::ZERO,
        depth: NormalizedLevel::FULL,
        phase_offset: PhaseOffset::new(offset).expect("in range"),
        polarity: LfoPolarity::Bipolar,
    }
}

#[test]
fn a_modulated_row_advances_once_per_quantum_after_its_composition() {
    // LFO A at rate zero and a quarter turn is a constant one; its edge of half a unit into
    // LFO B's depth retargets that depth from a half to one. B at rate zero and a quarter
    // turn is a constant one scaled by its depth, patched into an amplifier over a constant
    // — so the render **is** B's depth segment, frame for frame. Under a two-quantum
    // segment the first rendered quantum climbs from a half to three quarters and the
    // second to one; a row advanced both before and after its composition would reach one
    // by the end of the first.
    let ir = GraphIr::builder()
        .node(LFO, constant_source(0.25), ExecutionScope::Global)
        .node(
            SECOND_LFO,
            IrNodeKind::Lfo {
                waveform: LfoWaveform::Sine,
                rate: Frequency::ZERO,
                depth: NormalizedLevel::new(0.5).expect("in range"),
                phase_offset: PhaseOffset::new(0.25).expect("in range"),
                polarity: LfoPolarity::Bipolar,
            },
            ExecutionScope::Global,
        )
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
            (SECOND_LFO, PortId::FIRST),
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
            (SECOND_LFO, parameters::LFO_DEPTH),
            ModulationDepth::new(ModulationUnit::Normalized, 0.5).expect("finite"),
        )
        .build()
        .expect("a readable plan");
    let plan = admit(&ir);
    let depth = plan
        .resolve_parameter(SECOND_LFO, parameters::LFO_DEPTH)
        .expect("the LFO declares a depth");
    let (_control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    renderer.smooth_over(depth, 2 * QUANTUM_FRAMES);
    let mut out = vec![0.0_f32; 4 * Q];
    let block = AudioBlockMut::new(&mut out, 4 * Q, ChannelLayout::Mono).expect("a mono block");
    crate::render::Renderer::render(&mut renderer, block, crate::render::TimedEvents::EMPTY)
        .expect("renders");
    // Output quantum `n + 1` is engine quantum `n`: the primed quantum is silence.
    assert!(out[..Q].iter().all(|s| *s == 0.0));
    let first = &out[Q..2 * Q];
    let second = &out[2 * Q..3 * Q];
    let third = &out[3 * Q..];
    for k in 0..Q {
        let want = 0.5 + 0.5 * (k + 1) as f32 / (2 * Q) as f32;
        assert!(
            (first[k] - want).abs() < 1e-5,
            "first quantum, frame {k}: {} against {want}",
            first[k]
        );
        let want = 0.75 + 0.5 * (k + 1) as f32 / (2 * Q) as f32;
        assert!(
            (second[k] - want).abs() < 1e-5,
            "second quantum, frame {k}: {} against {want}",
            second[k]
        );
    }
    assert!(
        third.iter().all(|s| (*s - 1.0).abs() < 1e-6),
        "the segment ended on its target"
    );
    assert!(
        first[Q - 1] < 0.8,
        "the first quantum ends short of the target: {}",
        first[Q - 1]
    );
}

/// A pitched voice with a voice-scope LFO on its frequency, two compiled voices, stealing
/// the oldest with V1's fade.
fn stealing_voice_with_lfo() -> CompiledPlan {
    stealing_voice_from(GraphIr::builder().node(
        LFO,
        IrNodeKind::Lfo {
            waveform: LfoWaveform::Sine,
            rate: Frequency::new(5.0).expect("finite"),
            depth: NormalizedLevel::FULL,
            phase_offset: PhaseOffset::ZERO,
            polarity: LfoPolarity::Bipolar,
        },
        ExecutionScope::Voice,
    ))
}

fn stealing_voice_from(builder: crate::ir::GraphIrBuilder) -> CompiledPlan {
    let ir = builder
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::new(0.25).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.002).expect("finite"),
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.004).expect("finite"),
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
        .modulate(
            (LFO, PortId::FIRST),
            (SOURCE, parameters::SINE_FREQUENCY),
            ModulationDepth::new(ModulationUnit::Semitones, 2.0).expect("finite"),
        )
        .tuning(
            ExecutionScope::Voice,
            crate::tuning::PreparedTuning::equal_temperament().expect("prepares"),
        )
        .declaring(crate::ir::PlanDeclarations {
            note_producers: vec![crate::ir::NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(2),
                simultaneous_holds: EventCount::NONE,
            }],
            held_notes: HeldNoteCount::measured(2),
            stealing: StealingPolicy::Oldest {
                fade: FrameCount::new(128),
            },
            ..crate::ir::PlanDeclarations::default()
        })
        .build()
        .expect("a readable plan");
    let outcome = compile(&ir, &RenderConfig::new(profile()));
    let reported = match outcome
        .report()
        .row(crate::report::ResourceField::MutableStateBytes)
        .expect("row")
        .requested()
    {
        crate::report::ResourceAmount::Bytes(bytes) => bytes.get(),
        other => panic!("mutable bytes: {other:?}"),
    };
    let plan = admit(&ir);
    assert_eq!(
        plan.script_bytes_held() as u64,
        ir.script_bytes(),
        "immutable script allocation matches its charge"
    );
    let (_, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("stream");
    assert_eq!(
        renderer.slot_bytes_held() as u64
            + renderer.ramp_table_bytes_held() as u64
            + u64::from(renderer.prepared_record_count().get())
                * crate::node::state_bytes_per_node(),
        reported,
        "snapshots and per-voice slots are charged exactly"
    );
    plan
}

fn note_on(plan: &CompiledPlan, key: u8, at: u64) -> PlanEvent {
    PlanEvent::new(
        PlanPosition::new(at),
        CompiledPayload::NoteOn {
            slot: plan.resolve_note(ENVELOPE).expect("playable"),
            key: KeyIdentity::new(key).expect("a keyboard position"),
            velocity: NoteVelocity::FULL,
        },
    )
}

fn note_off(plan: &CompiledPlan, key: u8, at: u64) -> PlanEvent {
    PlanEvent::new(
        PlanPosition::new(at),
        CompiledPayload::NoteOff {
            slot: plan.resolve_note(ENVELOPE).expect("playable"),
            key: KeyIdentity::new(key).expect("a keyboard position"),
        },
    )
}

/// Render `quanta` quanta of the plan through the compiled scheduler and hand back the
/// renderer, whose state records are then readable.
fn render_through(
    plan: &CompiledPlan,
    events: &[PlanEvent],
    quanta: usize,
) -> (PreparedRenderer, usize) {
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(plan, events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let mut arbiter = PublicationArbiter::prepare(&profile()).expect("the store is preparable");
    let mut done = 0;
    let frames = quanta * Q;
    while done < frames {
        let this = BLOCK.min(frames - done);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render(&mut renderer, &mut arbiter, output)
            .expect("the stream renders");
        done += this;
    }
    (renderer, scheduler.released_after_steal())
}

fn lfo_phases(renderer: &PreparedRenderer) -> Vec<f64> {
    renderer
        .node_states()
        .iter()
        .filter_map(|state| match state {
            NodeState::Lfo { phase } => Some(*phase),
            _ => None,
        })
        .collect()
}

#[test]
fn a_control_script_defers_a_steal_reset_to_the_next_quantum_for_only_that_voice() {
    use crate::script::{ProjectSeed, ScriptIdentity, ScriptStateId};
    let program = ScriptIdentity::new(LFO, ScriptStateId::new(1), ProjectSeed::new(2))
        .compile_control(
            "param a = 0\nparam b = 0\nparam c = 0\nparam d = 0\nparam fifth = 0\nout = rand(0, 1) + accum(0.125) + a + b + c + d + fifth",
            SampleRate::new(RATE).expect("rate"),
            &[],
        )
        .expect("program");
    let plan = stealing_voice_from(GraphIr::builder().script(program, ExecutionScope::Voice));
    let q = Q as u64;
    let held = [note_on(&plan, 60, 0), note_on(&plan, 67, q)];
    let stolen = [held[0], held[1], note_on(&plan, 72, 4 * q + 5)];
    let states = |events: &[PlanEvent], quanta| {
        render_through(&plan, events, quanta)
            .0
            .node_states()
            .iter()
            .filter_map(|state| {
                if let NodeState::Script {
                    registers,
                    seed,
                    reset_pending,
                    voice,
                    ..
                } = state
                {
                    Some((*registers, *seed, *reset_pending, *voice))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    let pending = states(&stolen, 8);
    let control = states(&held, 8);
    assert_eq!(pending.len(), 2);
    assert_ne!(pending[0].1, pending[1].1, "voice seeds are distinct");
    assert!(pending[0].2, "reset at 6Q+5 waits for the next boundary");
    assert_eq!(pending[0].0, control[0].0, "no partial-quantum evaluation");
    assert_eq!(pending[1], control[1], "the other voice is untouched");
    let reset = states(&stolen, 9);
    let fresh = states(&held, 2);
    assert_eq!(reset[0], fresh[0], "one evaluation from the original seed");
    assert_eq!(reset[1], states(&held, 9)[1], "the other voice continues");
}

#[test]
fn a_steal_resets_the_taken_voices_modulator_and_leaves_the_others() {
    // ADR-0058 clause 4 reaches a modulator through the pre-pass's reset marks: after the
    // third note takes the first voice, that voice's LFO restarts its cycle while the second
    // voice's runs on, so the two phases differ. With two notes and no steal they agree,
    // which is the control that says the difference is the reset and not the instancing.
    let plan = stealing_voice_with_lfo();
    let q = Q as u64;
    let stolen = [
        note_on(&plan, 60, 0),
        note_on(&plan, 67, q),
        note_on(&plan, 72, 4 * q + 5),
        note_off(&plan, 60, 12 * q),
        note_off(&plan, 67, 12 * q),
        note_off(&plan, 72, 12 * q),
    ];
    let (renderer, releases) = render_through(&plan, &stolen, 16);
    assert_eq!(releases, 1, "the third note took a voice");
    let phases = lfo_phases(&renderer);
    assert_eq!(phases.len(), 2, "one LFO per voice");
    assert!(
        (phases[0] - phases[1]).abs() > 1e-6,
        "the taken voice's LFO restarted: {phases:?}"
    );

    let held = [
        note_on(&plan, 60, 0),
        note_on(&plan, 67, q),
        note_off(&plan, 60, 12 * q),
        note_off(&plan, 67, 12 * q),
    ];
    let (renderer, releases) = render_through(&plan, &held, 16);
    assert_eq!(releases, 0);
    let phases = lfo_phases(&renderer);
    assert!(
        (phases[0] - phases[1]).abs() < 1e-12,
        "without a steal both run on together: {phases:?}"
    );
}

#[test]
fn a_modulated_voice_plan_renders_the_same_bits_offline_and_through_the_scheduler() {
    // The two paths differ in who drives the blocks and where events enter; the pre-pass
    // is in both, and the offline render is the compiled stream's one priming quantum
    // apart — as `P06-S007` holds for an unmodulated voice.
    let plan = stealing_voice_with_lfo();
    let q = Q as u64;
    let events = [note_on(&plan, 60, 0), note_on(&plan, 67, q)];
    let quanta = 12;
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let mut arbiter = PublicationArbiter::prepare(&profile()).expect("the store is preparable");
    let mut through = Vec::new();
    let mut done = 0;
    while done < quanta * Q {
        let this = BLOCK.min(quanta * Q - done);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render(&mut renderer, &mut arbiter, output)
            .expect("the stream renders");
        through.extend_from_slice(&samples);
        done += this;
    }
    let offline = render_offline(
        plan,
        FrameCount::new((quanta * Q) as u64),
        PlanPosition::ZERO,
        &events
            .iter()
            .map(|event| {
                crate::offline::OfflineEvent::new(
                    SampleTime::new(event.position().as_u64()),
                    event.payload(),
                )
            })
            .collect::<Vec<_>>(),
    )
    .expect("renders");
    let through_bits: Vec<u32> = through[Q..].iter().map(|s| s.to_bits()).collect();
    let offline_bits: Vec<u32> = offline[..(quanta - 1) * Q]
        .iter()
        .map(|s| s.to_bits())
        .collect();
    assert_eq!(through_bits, offline_bits);
    assert!(through.iter().any(|s| s.abs() > 0.1), "audible");
}

#[test]
fn an_audio_script_resets_at_the_taken_voices_exact_sample() {
    use crate::script::{ProjectSeed, ScriptIdentity, ScriptStateId};
    let program = ScriptIdentity::new(SOURCE, ScriptStateId::new(1), ProjectSeed::new(2))
        .compile_audio(
            "out = accum(0.125)",
            SampleRate::new(RATE).expect("rate"),
            ChannelLayout::Mono,
            &[],
        )
        .expect("program");
    let ir = GraphIr::builder()
        .script(program, ExecutionScope::Voice)
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
            (AMPLIFIER, AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(crate::ir::PlanDeclarations {
            note_producers: vec![crate::ir::NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(2),
                simultaneous_holds: EventCount::NONE,
            }],
            held_notes: HeldNoteCount::measured(2),
            stealing: StealingPolicy::Oldest {
                fade: FrameCount::new(128),
            },
            ..crate::ir::PlanDeclarations::default()
        })
        .build()
        .expect("IR");
    let plan = admit(&ir);
    let q = Q as u64;
    let held = [note_on(&plan, 60, 0), note_on(&plan, 67, q)];
    let taken = [held[0], held[1], note_on(&plan, 72, 4 * q + 5)];
    let states = |events: &[PlanEvent]| {
        render_through(&plan, events, 8)
            .0
            .node_states()
            .iter()
            .filter_map(|state| {
                if let NodeState::Script {
                    registers, seed, ..
                } = state
                {
                    Some((*registers, *seed))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    let code = synth_script::compile::compile(
        "out = accum(0.125)",
        &synth_script::compile::CompileOptions::default(),
    )
    .0
    .expect("oracle program")
    .script;
    for (events, evaluations) in [(&held[..], [448, 448]), (&taken[..], [59, 448])] {
        let states = states(events);
        assert_eq!(states.len(), 2);
        for ((actual, seed), count) in states.into_iter().zip(evaluations) {
            let mut expected = synth_core::script::RegisterFile::new(0, seed.as_u64());
            for _ in 0..count {
                let _ = code.eval(
                    &[],
                    &mut expected,
                    &synth_core::script::EvalContext::audio(RATE),
                );
            }
            assert_eq!(
                actual, expected,
                "the reset at 6Q+5 leaves 59 evaluations; the other voice keeps all 448"
            );
        }
    }
}
