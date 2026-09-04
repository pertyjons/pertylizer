//! EVD-0009 harness: what the prepared function table costs, against direct calls.
//!
//! [ADR-0004](../../../plans/v2/decisions/ADR-0004-native-node-representation.md)
//! acceptance rule B: the dispatch overhead is measured on the minimal voice path at
//! `Q` = 64, against a hand-written direct-call variant of **the same plan**, and Option C
//! is accepted if the overhead is below 3% of the plan's per-quantum cost.
//!
//! Build and run exactly as the evidence record states:
//!
//! ```text
//! cargo build --release --example evd_0009_dispatch_cost -p synth_engine_v2
//! taskset -c 10 target/release/examples/evd_0009_dispatch_cost <rounds> <iterations>
//! ```
//!
//! # What each arm is
//!
//! Every arm runs **the whole plan** — the crate's own kernels, not a model of them —
//! over the same quantum of the same voice path: an envelope, a sine, a filter, an
//! amplifier, and the write into the output carry that the renderer performs. The output
//! write is in every arm because rule B's threshold is a share of *the plan's*
//! per-quantum cost, and an arm that stopped at the last node would be dividing by a
//! denominator the plan does not have. What differs is only how the work is reached.
//!
//! - `direct_ctl` / `direct` — the hand-written variant: each kernel called by name over
//!   buffers the caller holds. The two are bit-for-bit the same arm, and the spread
//!   between them is this comparison's noise floor. **The control runs first.**
//! - `enum_ctl` / `enum` — ADR-0004's Option A over the same schedule, arena and binding,
//!   dispatched by a `match`.
//! - `hybrid_ctl` / `hybrid` — **the hybrid acceptance rule C names**: a closed enum for
//!   the two hottest primitives of this path, the table for the other two.
//! - `table_ctl` / `table` — what the renderer does: a schedule of steps, each binding its
//!   slots out of one flat arena and calling through the function pointer admission
//!   resolved. It has its own null control too, so rule B's comparison has a noise floor
//!   in the candidate's instruction mix and not only in the baseline's.
//! - `fused` — the same arithmetic in one function, which the optimizer is free to fuse
//!   across node boundaries. Not an acceptance arm: it is the bound on what *any*
//!   per-node call costs, dispatch or not, and it is reported so the record can say how
//!   much of the difference is the pointer and how much is having nodes at all.
//! - `renderer` — one `render` call for exactly one quantum, through the real renderer.
//!   Reported so the overhead can also be stated as a share of a realistic per-quantum
//!   cost rather than of the node work alone.
//!
//! The arm order is **rotated in groups** each round — a control and the arm it bounds are
//! one group, so a rotation can never run an arm before its own control — and no arm keeps
//! a position; the **minimum**
//! over rounds is each arm's figure, which is the estimator the evidence rules require on
//! a machine with a permanent background load; and each control is compared to its arm
//! **paired within a round**, because two independently aggregated medians cancel drift
//! rather than measure it.

use std::hint::black_box;
use std::time::Instant;

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::node::kernels::{
    ENVELOPE_GATE, InputBuffer, MAX_INPUTS, NodeIo, NodeState, PreparedNode, TimedControl,
    amplifier, envelope, filter, sine,
};
use synth_engine_v2::plan::{BufferSlot, CompiledPlan, NodeStep, PlanOp};
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, Frequency, NormalizedLevel, ParameterValue,
    Resonance, SampleRate, Seconds,
};
use synth_engine_v2::render::{
    AudioBlockMut, EventEnvelope, EventPayload, Renderer, TimedEvent, TimedEvents,
};
use synth_engine_v2::stream::StreamControl;

use synth_engine_v2::time::{
    FrameCount, PlanPosition, QUANTUM_FRAMES, QuantumOffset, SampleTime, StreamAnchor, TimeSource,
};

/// One quantum of the sine's **authored** amplitude, for its per-frame amplitude read
/// (`SOUND-INV-024`): a constant buffer is what an unsmoothed slot fills, and it has to be
/// the fixture's own amplitude rather than unity, because the hand-built arms are compared
/// with the crate's renderer over the same plan — an independent review found a unity ramp
/// making that comparison one of two different signals. Seeded once in `main` from the
/// prepared record, and read through [`sine_ramp`]; every other kernel here has no
/// quantum-rate control and is handed nothing.
static SINE_RAMP: std::sync::OnceLock<[f32; synth_engine_v2::time::QUANTUM_FRAMES as usize]> =
    std::sync::OnceLock::new();

/// The sine's ramp, once seeded; empty before, which the kernel reads as silence.
fn sine_ramp() -> &'static [f32] {
    SINE_RAMP.get().map_or(&[][..], |ramp| &ramp[..])
}

/// Seed the sine's ramp from the prepared record's amplitude.
fn seed_sine_ramp(prepared: &PreparedNode) {
    let amplitude = match prepared {
        PreparedNode::Sine { amplitude, .. } => amplitude.as_f32(),
        _ => 1.0,
    };
    let _ = SINE_RAMP.set([amplitude; synth_engine_v2::time::QUANTUM_FRAMES as usize]);
}

/// A raised gate at the first sample of the quantum about to be rendered.
fn held_gate() -> TimedControl {
    TimedControl {
        offset: QuantumOffset::ZERO,
        control: ENVELOPE_GATE,
        value: ParameterValue::ONE,
    }
}

/// Frames in one render quantum, from the crate rather than restated here.
const Q: usize = QUANTUM_FRAMES as usize;

const ENVELOPE: NodeId = NodeId::new(1);
const OSCILLATOR: NodeId = NodeId::new(2);
const FILTER: NodeId = NodeId::new(3);
const AMPLIFIER: NodeId = NodeId::new(4);
const OUTPUT: NodeId = NodeId::new(5);

/// The minimal voice path: an envelope, an oscillator, a filter, an amplifier, an output.
fn voice_path() -> GraphIr {
    GraphIr::builder()
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.01).expect("not negative"),
                decay: Seconds::new(0.1).expect("not negative"),
                sustain: NormalizedLevel::new(0.7).expect("within range"),
                release: Seconds::new(0.2).expect("not negative"),
            },
            ExecutionScope::Voice,
        )
        .node(
            OSCILLATOR,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::new(0.8).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(2_000.0).expect("positive"),
                resonance: Resonance::BUTTERWORTH,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
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
        // `SOUND-INV-021`: a voice scope with a pitch destination names its tuning. The
        // harness rendered before that clause and its fixture was not updated with it, so it
        // panicked at admission on every run since; the gate compiles examples and never runs
        // them, which is how that stayed unseen.
        .tuning(
            ExecutionScope::Voice,
            synth_engine_v2::tuning::PreparedTuning::equal_temperament()
                .expect("twelve-tone equal temperament prepares"),
        )
        .build()
        .expect("the minimal voice path is a readable plan")
}

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("a valid rate"),
        FrameCount::new(Q as u64),
        ChannelLayout::Mono,
    )
    .expect("a valid harness profile")
}

/// One buffer into one channel of an interleaved carry.
///
/// **One function, called by every arm.** The output write is part of the plan, so it is
/// part of what each arm costs — and if the arms wrote it differently, the comparison
/// would be measuring two output writes rather than two ways of reaching a kernel. This
/// is the shape `render/hot.rs` uses.
#[inline(never)]
fn write_channel(source: &[f32], carry: &mut [f32], channels: usize, channel: usize) {
    for frame in 0..Q {
        let value = source.get(frame).copied().unwrap_or(0.0);
        if let Some(slot) = carry.get_mut(frame * channels + channel) {
            *slot = value;
        }
    }
}

/// Time `iterations` runs of `body`, in seconds per iteration.
fn timed(iterations: u32, mut body: impl FnMut()) -> f64 {
    let start = Instant::now();
    for _ in 0..iterations {
        body();
    }
    start.elapsed().as_secs_f64() / f64::from(iterations)
}

/// The hand-written variant: every kernel called by name, over buffers held here.
struct Hand {
    envelope: PreparedNode,
    oscillator: PreparedNode,
    filter: PreparedNode,
    amplifier: PreparedNode,
    envelope_state: NodeState,
    oscillator_state: NodeState,
    filter_state: NodeState,
    amplifier_state: NodeState,
    /// The buffer the compiler assigned to the oscillator, which the filter and the
    /// amplifier then run over in place.
    audio: Vec<f32>,
    /// The buffer the compiler assigned to the envelope.
    control: Vec<f32>,
    /// The interleaved carry the output operation writes into, as the renderer has.
    carry: Vec<f32>,
}

impl Hand {
    fn new(plan: &CompiledPlan) -> Self {
        // The **plan's** prepared records, so the arms differ in dispatch and in nothing
        // else: a hand-written variant that prepared its own coefficients would be
        // measuring a different filter.
        //
        // Found by variant rather than by position, and it matters: a kernel handed a
        // record of the wrong kind returns without writing, which is the right thing on
        // the audio thread and which would silently turn this arm into a measurement of
        // four early returns. The first version of this harness did exactly that, and
        // reported the direct arm as a hundred times faster than the fused one.
        let find = |wanted: fn(&PreparedNode) -> bool| -> PreparedNode {
            plan.prepared_nodes()
                .iter()
                .copied()
                .find(wanted)
                .expect("the voice path has this node")
        };
        let envelope = find(|node| matches!(node, PreparedNode::Envelope { .. }));
        let oscillator = find(|node| matches!(node, PreparedNode::Sine { .. }));
        let filter = find(|node| matches!(node, PreparedNode::Filter { .. }));
        let amplifier = find(|node| matches!(node, PreparedNode::Amplifier));
        Self {
            envelope,
            oscillator,
            filter,
            amplifier,
            envelope_state: NodeState::initial(&envelope),
            oscillator_state: NodeState::initial(&oscillator),
            filter_state: NodeState::initial(&filter),
            amplifier_state: NodeState::initial(&amplifier),
            audio: vec![0.0; Q],
            control: vec![0.0; Q],
            carry: vec![0.0; Q],
        }
    }

    /// Hold the gate.
    ///
    /// A gate is sample-positioned since P02-T007, so it reaches the envelope with a
    /// buffer rather than being set on the state beforehand: one quantum rendered with
    /// the edge at offset 0, which is where the boundary-applied gate used to land. The
    /// control buffer it writes is overwritten by the first timed quantum.
    fn open_gate(&mut self) {
        envelope(
            &self.envelope,
            &mut self.envelope_state,
            &mut NodeIo {
                out: &mut self.control,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &[held_gate()],
                ramps: &[],
            },
        );
    }

    /// One quantum of the voice path, by direct call.
    ///
    /// **Over the buffers the compiler assigned, with the aliasing it chose.** The plan
    /// gives this path two buffers: the oscillator writes one, the filter and the
    /// amplifier both run in place over it, and the envelope writes the other. A
    /// hand-written variant that used a buffer per node would be a different memory
    /// layout and a different branch inside the amplifier — a comparison of two programs
    /// rather than of two ways to reach the same one.
    fn quantum(&mut self) {
        sine(
            &self.oscillator,
            &mut self.oscillator_state,
            &mut NodeIo {
                out: &mut self.audio,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &[],
                ramps: sine_ramp(),
            },
        );
        filter(
            &self.filter,
            &mut self.filter_state,
            &mut NodeIo {
                out: &mut self.audio,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        envelope(
            &self.envelope,
            &mut self.envelope_state,
            &mut NodeIo {
                out: &mut self.control,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        amplifier(
            &self.amplifier,
            &mut self.amplifier_state,
            &mut NodeIo {
                out: &mut self.audio,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Patched(&self.control)],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        // The plan's last operation, which the renderer performs and which a hand-written
        // variant would perform too: one channel of the quantum into the output carry.
        write_channel(&self.audio, &mut self.carry, 1, 0);
    }
}

/// The renderer's variant: a schedule of steps over one flat arena, dispatched through
/// the pointers admission resolved.
struct Table {
    prepared: Vec<PreparedNode>,
    states: Vec<NodeState>,
    steps: Vec<NodeStep>,
    /// The same schedule as a closed enum, for the Option A arm.
    kinds: Vec<Kind>,
    /// The plan's output operations: which buffer goes to which channel.
    outputs: Vec<(BufferSlot, usize)>,
    arena: Vec<f32>,
    /// Where each slot's samples live, which `bind` resolves rather than computing.
    regions: Vec<synth_engine_v2::plan::BufferRegion>,
    /// The interleaved carry the output operations write into.
    carry: Vec<f32>,
}

/// The node kinds this path has, as a closed enum — ADR-0004's Option A.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Envelope,
    Sine,
    Filter,
    Amplifier,
}

impl Table {
    fn new(plan: &CompiledPlan) -> Self {
        let steps: Vec<NodeStep> = plan
            .ops()
            .iter()
            .filter_map(|op| match op {
                PlanOp::Node(step) => Some(*step),
                PlanOp::Output { .. } => None,
            })
            .collect();
        // The plan's output operations, executed by every arm: the acceptance rule's
        // denominator is the plan's per-quantum cost, not its node work.
        let outputs: Vec<(BufferSlot, usize)> = plan
            .ops()
            .iter()
            .filter_map(|op| match op {
                PlanOp::Output { source } => Some((*source, 0)),
                PlanOp::Node(_) => None,
            })
            .collect();
        let kinds: Vec<Kind> = steps
            .iter()
            .filter_map(
                |step| match plan.prepared_nodes().get(step.node().index()) {
                    Some(PreparedNode::Envelope { .. }) => Some(Kind::Envelope),
                    Some(PreparedNode::Sine { .. }) => Some(Kind::Sine),
                    Some(PreparedNode::Filter { .. }) => Some(Kind::Filter),
                    Some(PreparedNode::Amplifier) => Some(Kind::Amplifier),
                    _ => None,
                },
            )
            .collect();
        assert!(
            kinds.len() == steps.len(),
            "the enum arm must cover every step of the schedule, or it is measuring less work"
        );
        Self {
            kinds,
            outputs,
            carry: vec![0.0; Q * plan.channel_layout().channels()],
            prepared: plan.prepared_nodes().to_vec(),
            states: plan
                .prepared_nodes()
                .iter()
                .map(NodeState::initial)
                .collect(),
            steps,
            // ADR-0041 clause 2: `bind` reads the regions the plan records. This harness
            // allocates the same uniform slots it always did, so its table is the planar
            // arithmetic written down rather than derived — the arms it times are
            // unchanged.
            regions: (0..plan.buffer_count())
                .map(|slot| {
                    synth_engine_v2::plan::BufferRegion::new(slot * Q, Q)
                        .expect("a quantum-wide slot is a region")
                })
                .collect(),
            arena: vec![0.0; plan.buffer_count() * Q],
        }
    }

    /// Hold the gate on whichever node is the envelope.
    ///
    /// One quantum through the envelope kernel with the edge at offset 0, for the reason
    /// given on the hand-written arm's `open_gate`.
    fn open_gate(&mut self) {
        let mut scratch = vec![0.0_f32; Q];
        for (index, prepared) in self.prepared.clone().iter().enumerate() {
            if let (PreparedNode::Envelope { .. }, Some(state)) =
                (prepared, self.states.get_mut(index))
            {
                envelope(
                    prepared,
                    state,
                    &mut NodeIo {
                        out: &mut scratch,
                        channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                        inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                        position: None,
                        controls: &[held_gate()],
                        ramps: &[],
                    },
                );
            }
        }
    }

    /// The schedule walked, and nothing else.
    ///
    /// The control for [`Self::bind_only`], and the reason the binding figure means what
    /// it says: an arm that walks the steps *and* binds them cannot tell a reader how
    /// much of its cost is the binding. The difference between the two arms can.
    fn walk_only(&mut self) {
        for step in &self.steps {
            black_box(step);
        }
    }

    /// The schedule walked and every step's slots bound — and nothing called.
    ///
    /// The table's cost has two halves, and they have different remedies: resolving each
    /// step's slots into slices, and the indirect call itself. A record that reported
    /// only the total would send a reader to the wrong one.
    fn bind_only(&mut self) {
        for step in &self.steps {
            let io = synth_engine_v2::node::kernels::bind(
                &mut self.arena,
                &self.regions,
                step,
                None,
                &[],
                sine_ramp(),
            );
            black_box(&io);
        }
    }

    /// One quantum of the voice path, through a **closed enum** instead of the table.
    ///
    /// ADR-0004's Option A, over the same schedule, the same arena and the same binding,
    /// so the two arms differ in dispatch and in nothing else. This is the comparison the
    /// record is actually choosing between; the direct arm is the comparison rule B
    /// names, and it also carries the cost of having a compiled schedule at all.
    fn quantum_by_enum(&mut self) {
        for (step, kind) in self.steps.iter().zip(self.kinds.iter()) {
            let Some(prepared) = self.prepared.get(step.node().index()) else {
                continue;
            };
            let Some(state) = self.states.get_mut(step.node().index()) else {
                continue;
            };
            let Some(mut io) = synth_engine_v2::node::kernels::bind(
                &mut self.arena,
                &self.regions,
                step,
                None,
                &[],
                sine_ramp(),
            ) else {
                continue;
            };
            match kind {
                Kind::Envelope => envelope(prepared, state, &mut io),
                Kind::Sine => sine(prepared, state, &mut io),
                Kind::Filter => filter(prepared, state, &mut io),
                Kind::Amplifier => amplifier(prepared, state, &mut io),
            }
        }
        self.write_output();
    }

    /// The plan's output operations, as the renderer performs them.
    fn write_output(&mut self) {
        let channels = self.carry.len() / Q.max(1);
        for (source, channel) in &self.outputs {
            let base = source.index() * Q;
            let Some(region) = self.arena.get(base..base + Q) else {
                continue;
            };
            write_channel(region, &mut self.carry, channels, *channel);
        }
    }

    /// One quantum of the voice path, through **the hybrid rule C names**.
    ///
    /// A closed enum for the few hottest primitives and the table for everything else.
    /// "Hottest" is decided by the arithmetic rather than by preference: the oscillator
    /// evaluates a transcendental per sample and the filter runs a two-integrator
    /// recurrence, while the envelope and the amplifier are a multiply and an add. Those
    /// two are dispatched by `match`; the other two go through the pointer.
    fn quantum_hybrid(&mut self) {
        for (step, kind) in self.steps.iter().zip(self.kinds.iter()) {
            let Some(prepared) = self.prepared.get(step.node().index()) else {
                continue;
            };
            let Some(state) = self.states.get_mut(step.node().index()) else {
                continue;
            };
            let Some(mut io) = synth_engine_v2::node::kernels::bind(
                &mut self.arena,
                &self.regions,
                step,
                None,
                &[],
                sine_ramp(),
            ) else {
                continue;
            };
            match kind {
                Kind::Sine => sine(prepared, state, &mut io),
                Kind::Filter => filter(prepared, state, &mut io),
                Kind::Envelope | Kind::Amplifier => step.kernel().run(prepared, state, &mut io),
            }
        }
        self.write_output();
    }

    /// One quantum of the voice path, through the table.
    fn quantum(&mut self) {
        for step in &self.steps {
            let Some(prepared) = self.prepared.get(step.node().index()) else {
                continue;
            };
            let Some(state) = self.states.get_mut(step.node().index()) else {
                continue;
            };
            let Some(mut io) = synth_engine_v2::node::kernels::bind(
                &mut self.arena,
                &self.regions,
                step,
                None,
                &[],
                sine_ramp(),
            ) else {
                continue;
            };
            step.kernel().run(prepared, state, &mut io);
        }
        self.write_output();
    }
}

/// The same arithmetic in one function, with no node boundary for the optimizer to keep.
///
/// Deliberately *not* built from the kernels: it is the fused bound, and fusing is what
/// it is measuring. The coefficients come from the plan so the arithmetic is the same.
fn fused(state: &mut Fused, out: &mut [f32]) {
    for sample in out.iter_mut() {
        // Envelope: held at the sustain level, which is the steady state the other arms
        // also spend their quanta in.
        let level = state.sustain;
        // Oscillator.
        let value = state.amplitude * (std::f64::consts::TAU * state.phase).sin() as f32;
        state.phase += state.increment;
        if !(0.0..1.0).contains(&state.phase) {
            state.phase -= state.phase.floor();
        }
        // Filter.
        let drive = value - state.low;
        let band_pass = state.integrator[0] * state.band + state.integrator[1] * drive;
        let low_pass = state.low + state.integrator[1] * state.band + state.integrator[2] * drive;
        state.band = 2.0 * band_pass - state.band;
        state.low = 2.0 * low_pass - state.low;
        // Amplifier, and the output write this arm folds into the same pass.
        *sample = low_pass * level;
    }
}

/// The fused arm's state, gathered from the same prepared records.
struct Fused {
    phase: f64,
    increment: f64,
    amplitude: f32,
    sustain: f32,
    band: f32,
    low: f32,
    integrator: [f32; 3],
}

impl Fused {
    fn new(plan: &CompiledPlan) -> Self {
        let mut fused = Self {
            phase: 0.0,
            increment: 220.0 / 48_000.0,
            amplitude: 0.8,
            sustain: 0.7,
            band: 0.0,
            low: 0.0,
            integrator: [0.0; 3],
        };
        for prepared in plan.prepared_nodes() {
            if let PreparedNode::Filter { integrator } = prepared {
                fused.integrator = *integrator;
            }
        }
        fused
    }
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let rounds: u32 = arguments
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(9);
    let iterations: u32 = arguments
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200_000);

    let plan = compile(&voice_path(), &RenderConfig::new(profile()))
        .into_plan()
        .expect("the minimal voice path is admitted");
    if let Some(sine) = plan
        .prepared_nodes()
        .iter()
        .find(|node| matches!(node, PreparedNode::Sine { .. }))
    {
        seed_sine_ramp(sine);
    }

    // The hand-written arm mirrors the compiler's buffer assignment, so a plan that no
    // longer matches it must fail loudly rather than be compared against a different
    // memory layout.
    let compiled_buffers = plan.buffer_count();
    assert!(
        compiled_buffers == 2,
        "the hand-written arm mirrors a two-buffer assignment and the compiler produced \
         {compiled_buffers}; the arms would no longer differ only in dispatch"
    );

    let names = [
        "direct_ctl",
        "direct",
        "enum_ctl",
        "enum",
        "hybrid_ctl",
        "hybrid",
        "table_ctl",
        "table",
        "walk_only",
        "bind_only",
        "fused",
        "renderer",
    ];
    let mut best = [f64::MAX; 12];

    let mut hand = Hand::new(&plan);
    let mut table = Table::new(&plan);
    // Separate instances, so each arm has its own arena, states and carry: sharing them
    // would let one arm's quanta advance another's phase, and would make the agreement
    // check below compare an arm with itself.
    let mut enum_table = Table::new(&plan);
    let mut hybrid_table = Table::new(&plan);
    let mut fused_state = Fused::new(&plan);
    let mut fused_out = vec![0.0; Q];

    // The renderer arm, gated on and prepared once: its own carry and event scratch are
    // part of what it costs, and re-preparing it per round would time an allocation.
    let (_control, mut renderer) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("an admitted plan prepares");
    let gate = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_GATE)
        .expect("the envelope declares a gate");
    let opened = [TimedEvent::new(
        EventEnvelope::new(renderer.epoch(), SampleTime::ZERO, TimeSource::Compiled),
        EventPayload::SetParameter {
            slot: gate,
            value: ParameterValue::new(1.0).expect("finite"),
        },
    )];
    let mut block = vec![0.0; Q];
    if let Ok(audio) = AudioBlockMut::new(&mut block, Q, ChannelLayout::Mono) {
        let _ = renderer.render(audio, TimedEvents::new(&opened));
    }

    // Open the gate on the other arms too, so every one of them is in the same steady
    // state: a held note rather than an idle voice.
    hand.open_gate();
    table.open_gate();
    enum_table.open_gate();
    hybrid_table.open_gate();

    // **The arms have to agree before either of them is timed.** A kernel handed the
    // wrong record renders nothing and costs nothing, and a harness that did not check
    // would report that as a speed. Both arms run past the envelope's attack and decay
    // into the sustain the timed quanta measure, and then their outputs are compared
    // sample for sample.
    let settle = 200;
    for _ in 0..settle {
        hand.quantum();
        table.quantum();
        enum_table.quantum_by_enum();
        hybrid_table.quantum_hybrid();
    }
    // Compared on the **carry**, which is what the plan produces, rather than on the
    // arena buffer the last node happens to write. Reading the intermediate is how an arm
    // that skipped the plan's output operation passed this check once: its arena agreed
    // and its output did not exist.
    // Every arm, not only the two the first version compared: the table-against-enum
    // comparison is the one the decision turns on, so an enum arm that mapped a kind
    // wrongly would show up as a speed rather than as a defect.
    let mismatch = [&table.carry, &enum_table.carry, &hybrid_table.carry]
        .into_iter()
        .flat_map(|carry| {
            hand.carry
                .iter()
                .zip(carry.iter())
                .map(|(a, b)| (a - b).abs())
        })
        .fold(0.0_f32, f32::max);
    let loudest = hand
        .carry
        .iter()
        .copied()
        .fold(0.0_f32, |peak, value| peak.max(value.abs()));
    let quietest = [&table.carry, &enum_table.carry, &hybrid_table.carry]
        .into_iter()
        .map(|carry| {
            carry
                .iter()
                .copied()
                .fold(0.0_f32, |peak, value| peak.max(value.abs()))
        })
        .fold(f32::INFINITY, f32::min);
    if quietest <= 0.0 {
        println!("an_arm_carry_is_silent,{quietest:e}");
        println!("one arm produced no output; no timing below means anything");
        return;
    }
    if mismatch > 0.0 || loudest <= 0.0 {
        println!("arms_disagree,{mismatch:e}");
        println!("peak,{loudest:e}");
        println!("the two arms do not compute the same signal; no timing below means anything");
        return;
    }
    println!("arms_agree_peak,{loudest:.6}");

    // Every round's timing for every arm, kept so the controls can be compared **paired**
    // — the control against its arm inside the round both were measured in. Comparing two
    // independently aggregated medians cancels drift instead of measuring it, and reports
    // a noise floor an order of magnitude below the real one.
    let mut rounds_seen: Vec<Vec<f64>> = vec![Vec::new(); names.len()];

    // The arms as **groups**, because a control and the arm it bounds are one unit: a flat
    // rotation would sometimes start at an arm and run it before its own control, which is
    // the ordering the evidence rules exist to forbid. Groups rotate; inside a group the
    // control is always first.
    let groups: [&[usize]; 7] = [
        &[0, 1],
        &[2, 3],
        &[4, 5],
        &[6, 7],
        // The walk is the binding's control, so the two are one group and the walk runs
        // first — a rotation that started at the binding would charge it whatever drift
        // the round had accumulated.
        &[8, 9],
        &[10],
        &[11],
    ];

    for round in 0..rounds {
        // The starting group advances each round, so no arm keeps a position and a cache
        // or thermal bias that follows position is spread across all of them rather than
        // charged to whichever arm sits second.
        for offset in 0..groups.len() {
            let Some(group) = groups.get((round as usize + offset) % groups.len()) else {
                continue;
            };
            for arm in group.iter().copied() {
                let elapsed = match arm {
                    // The control runs first, inside the same rotation, so it bounds the arm
                    // it is a control for rather than a differently loaded moment.
                    0 | 1 => timed(iterations, || {
                        hand.quantum();
                        black_box(&hand.carry);
                    }),
                    // The enum arm's own control, because it is a different instruction mix
                    // from the direct one and cannot share its noise floor.
                    2 | 3 => timed(iterations, || {
                        enum_table.quantum_by_enum();
                        black_box(&enum_table.carry);
                    }),
                    // The hybrid's own control, for the same reason the enum has one.
                    4 | 5 => timed(iterations, || {
                        hybrid_table.quantum_hybrid();
                        black_box(&hybrid_table.carry);
                    }),
                    // The table's own control: the same arm again, so rule B's
                    // comparison has a null in the candidate's instruction mix and not
                    // only in the baseline's.
                    6 | 7 => timed(iterations, || {
                        table.quantum();
                        black_box(&table.carry);
                    }),
                    8 => timed(iterations, || {
                        table.walk_only();
                    }),
                    9 => timed(iterations, || {
                        table.bind_only();
                        black_box(&table.arena);
                    }),
                    10 => timed(iterations, || {
                        fused(black_box(&mut fused_state), black_box(&mut fused_out));
                    }),
                    _ => timed(iterations, || {
                        if let Ok(audio) = AudioBlockMut::new(&mut block, Q, ChannelLayout::Mono) {
                            let _ = renderer.render(audio, TimedEvents::EMPTY);
                        }
                        black_box(&block);
                    }),
                };
                if elapsed < best[arm] {
                    best[arm] = elapsed;
                }
                if let Some(seen) = rounds_seen.get_mut(arm) {
                    seen.push(elapsed);
                }
            }
        }
    }

    println!("arm,seconds_per_quantum,nanoseconds_per_quantum");
    for (name, seconds) in names.iter().zip(best.iter()) {
        println!("{name},{seconds:.12},{:.3}", seconds * 1e9);
    }

    // Paired: within each round, the control against the arm it bounds.
    let paired_spread = |control: usize, arm: usize| -> (f64, f64) {
        let (Some(controls), Some(arms)) = (rounds_seen.get(control), rounds_seen.get(arm)) else {
            return (f64::NAN, f64::NAN);
        };
        let mut spreads: Vec<f64> = controls
            .iter()
            .zip(arms.iter())
            .map(|(c, a)| (c - a).abs() / c.min(*a) * 100.0)
            .collect();
        spreads.sort_by(f64::total_cmp);
        let median = spreads.get(spreads.len() / 2).copied().unwrap_or(f64::NAN);
        let worst = spreads.iter().copied().fold(0.0_f64, f64::max);
        (median, worst)
    };
    let (direct_control, direct_control_worst) = paired_spread(0, 1);
    let (enum_control, enum_control_worst) = paired_spread(2, 3);
    let (hybrid_control, hybrid_control_worst) = paired_spread(4, 5);
    let (table_control, table_control_worst) = paired_spread(6, 7);

    // The decision-driving ratios, **paired within a round** as well as taken from the
    // per-arm minima: a ratio of two independently selected minima can pick each arm's
    // best moment out of a different round, and the paired median cannot.
    let paired_ratio = |over: usize, under: usize| -> f64 {
        let (Some(overs), Some(unders)) = (rounds_seen.get(over), rounds_seen.get(under)) else {
            return f64::NAN;
        };
        let mut ratios: Vec<f64> = overs
            .iter()
            .zip(unders.iter())
            .map(|(a, b)| (a / b - 1.0) * 100.0)
            .collect();
        ratios.sort_by(f64::total_cmp);
        ratios.get(ratios.len() / 2).copied().unwrap_or(f64::NAN)
    };

    let table_vs_direct = (best[7] / best[1] - 1.0) * 100.0;
    let hybrid_vs_direct = (best[5] / best[1] - 1.0) * 100.0;
    let enum_vs_direct = (best[3] / best[1] - 1.0) * 100.0;
    let table_vs_enum = (best[7] / best[3] - 1.0) * 100.0;
    let table_vs_direct_paired = paired_ratio(7, 1);
    let hybrid_vs_direct_paired = paired_ratio(5, 1);
    let enum_vs_direct_paired = paired_ratio(3, 1);
    // Both of these shapes bind the arena, so their difference is the dispatch shape with
    // the arena's cost on both sides — the comparison the redraft turns on.
    // The binding, paired: the difference between the two arms inside each round.
    let paired_difference = |over: usize, under: usize| -> f64 {
        let (Some(overs), Some(unders)) = (rounds_seen.get(over), rounds_seen.get(under)) else {
            return f64::NAN;
        };
        let mut deltas: Vec<f64> = overs
            .iter()
            .zip(unders.iter())
            .map(|(a, b)| (a - b) * 1e9)
            .collect();
        deltas.sort_by(f64::total_cmp);
        deltas.get(deltas.len() / 2).copied().unwrap_or(f64::NAN)
    };
    let table_vs_enum_paired = paired_ratio(7, 3);
    let table_vs_hybrid_paired = paired_ratio(7, 5);
    let binding = paired_difference(9, 8);
    let walk = best[8] / best[1] * 100.0;
    let node_boundaries = (best[1] / best[10] - 1.0) * 100.0;

    println!();
    println!("direct_control_spread_percent,{direct_control:.2}");
    println!("direct_control_worst_percent,{direct_control_worst:.2}");
    println!("enum_control_spread_percent,{enum_control:.2}");
    println!("enum_control_worst_percent,{enum_control_worst:.2}");
    println!("hybrid_control_spread_percent,{hybrid_control:.2}");
    println!("hybrid_control_worst_percent,{hybrid_control_worst:.2}");
    println!("table_control_spread_percent,{table_control:.2}");
    println!("table_control_worst_percent,{table_control_worst:.2}");
    println!("table_vs_direct_percent,{table_vs_direct:.2}");
    println!("table_vs_direct_paired_percent,{table_vs_direct_paired:.2}");
    println!("hybrid_vs_direct_paired_percent,{hybrid_vs_direct_paired:.2}");
    println!("enum_vs_direct_paired_percent,{enum_vs_direct_paired:.2}");
    println!("table_vs_enum_paired_percent,{table_vs_enum_paired:.2}");
    println!("table_vs_hybrid_paired_percent,{table_vs_hybrid_paired:.2}");
    println!("hybrid_vs_direct_percent,{hybrid_vs_direct:.2}");
    println!("enum_vs_direct_percent,{enum_vs_direct:.2}");
    println!("table_vs_enum_percent,{table_vs_enum:.2}");
    println!("binding_paired_nanoseconds,{binding:.2}");
    println!("walk_share_of_direct_percent,{walk:.2}");
    println!("direct_vs_fused_percent,{node_boundaries:.2}");
}
