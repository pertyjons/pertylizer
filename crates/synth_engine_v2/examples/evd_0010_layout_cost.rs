//! EVD-0010 harness: what the planar arena costs against an interleaved one, on the
//! **real** voice path.
//!
//! [ADR-0002](../../../plans/v2/decisions/ADR-0002-internal-channel-layout.md) was
//! accepted against its own measurement, on a shared-kernel premise
//! [ADR-0040](../../../plans/v2/decisions/ADR-0040-v2-owns-its-dsp.md) proposes to
//! remove. [EVD-0008](../../../plans/v2/evidence/phase-02/EVD-0008-internal-channel-layout-cost.md)
//! modelled two memory layouts under invented arithmetic because no path existed;
//! P02-T005 built one, so this harness measures **the crate's own kernels over the
//! crate's own arena** instead.
//!
//! Build and run exactly as the evidence record states:
//!
//! ```text
//! cargo build --release --example evd_0010_layout_cost -p synth_engine_v2
//! taskset -c 10 target/release/examples/evd_0010_layout_cost <rounds> <iterations>
//! ```
//!
//! # The three shapes, and why there are three
//!
//! "The real voice path" has three honest readings, and a record that measured only one
//! of them would answer a different question than the one it asked.
//!
//! - **A — as compiled today.** A stereo profile compiles the minimal voice patch to a
//!   *mono* chain plus one widening copy and two strided output writes. This is what the
//!   phase renders now: the planar arm is that schedule, asserted operation by operation
//!   against the compiler's own output, and required to reproduce the real renderer's
//!   carry bit for bit.
//! - **B — a stereo chain.** The widening moved upstream of the filter and the amplifier,
//!   which is what a path carrying a stereo signal looks like once a node produces one.
//!   EVD-0008's criterion-D shape, rebuilt with real kernels.
//! - **C — independent per-channel control.** Shape B with two envelopes and two filter
//!   settings: the case EVD-0008 named as unmeasured and expected to narrow the margin,
//!   because its interleaved amplifier read one control value per frame where this one
//!   reads two.
//!
//! # What makes the comparison a comparison
//!
//! Four asymmetries were found by review **before** any data was collected, three of them
//! favouring the interleaved arm. Each is closed here rather than noted:
//!
//! - **One call discipline on both sides.** Every node call in every arm is a direct call
//!   to an `#[inline(never)]` shim. An earlier revision reached the planar kernels through
//!   a function-pointer parameter and the interleaved ones directly, which charged planar
//!   an indirect dispatch per node — ABI cost, not layout cost, and the planar arms have
//!   more nodes. A block-based graph does dispatch per node, so the boundary stays; what
//!   it may not do is differ between the arms.
//! - **One kernel contract on both sides.** The interleaved kernels take a prepared record
//!   and a state as their own enums, destructure them with the same return-without-writing
//!   prologue the crate's kernels use, and read their inputs through the crate's own
//!   `NodeIo` and `InputBuffer`. An earlier revision handed them raw coefficients and raw
//!   slices, which compared a generic planar ABI against a bespoke interleaved one.
//! - **Each layout gets the arena its own liveness needs.** An earlier revision gave every
//!   interleaved arm one `Q` slot more than a linear scan would assign, which favoured
//!   planar. The maps below are each layout's actual peak, and the schedules are each
//!   layout's own: planar orders shape C as envelope, amplifier, envelope, amplifier so
//!   its second control reuses the first one's slot, which interleaved cannot do because
//!   its single amplifier call needs both controls live at once.
//! - **Every arm ends by filling an interleaved carry**, because that is the product the
//!   plan owes the host. Planar pays two strided writes; interleaved pays one memcpy.
//!
//! # What has to be true before anything is timed
//!
//! - **Bit-identical carries, every quantum of the settle, not only the last.** A
//!   comparison of one settled block cannot tell an arm that skipped work from one that
//!   did it, because a sustained note looks the same either way; the check therefore runs
//!   over the attack and the decay as well, compares raw bit patterns rather than
//!   differences — `f32::max` ignores a `NaN`, so a difference-based check can pass on a
//!   carry that is partly `NaN` — and refuses a silent arm.
//! - **Shape A must be the plan the crate really renders.** Its schedule is asserted
//!   against the compiler's operations, and its carry against the real renderer's, which
//!   aborts the run rather than printing a note.
//! - **Controls, rotation and pairing** are EVD-0009's, which arrived at them through nine
//!   retained corrections: one null control per arm in the arm's own instruction mix,
//!   groups rotated once per round with the control first, the minimum over rounds as each
//!   arm's figure, and every ratio and every control spread taken **within** the round
//!   both arms were measured in.

// This harness is what EVD-0010's recorded figures were measured on, so the shape of
// its interleaved and planar reads is the quantity under measurement rather than an
// incidental style choice. Rewriting `chunks_exact` into `as_chunks` changes the
// bounds checks the timed loops emit, which would silently make the record describe
// code that no longer exists. The lint is therefore refused for this file alone.
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::hint::black_box;
use std::time::Instant;

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::node::kernels::{
    ENVELOPE_GATE, InputBuffer, MAX_INPUTS, NodeIo, NodeState, PreparedNode, TimedControl,
    amplifier, copy, envelope, filter, sine,
};
use synth_engine_v2::plan::{CompiledPlan, NodeStep, PlanOp};
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
///
/// A gate is sample-positioned since P02-T007, so it reaches the envelope with a buffer
/// rather than being set on the state beforehand. Offset 0 is where the boundary-applied
/// gate this harness used to set landed.
fn held_gate() -> TimedControl {
    TimedControl {
        offset: QuantumOffset::ZERO,
        control: ENVELOPE_GATE,
        value: ParameterValue::ONE,
    }
}

/// Frames in one render quantum, from the crate rather than restated here.
const Q: usize = QUANTUM_FRAMES as usize;

/// Channels in the stereo arms. Not a general channel count: the interleaved kernels
/// below step frames of exactly this width, which is what an interleaved kernel does.
const CHANNELS: usize = 2;

const ENVELOPE: NodeId = NodeId::new(1);
const OSCILLATOR: NodeId = NodeId::new(2);
const FILTER: NodeId = NodeId::new(3);
const AMPLIFIER: NodeId = NodeId::new(4);
const OUTPUT: NodeId = NodeId::new(5);

/// The minimal voice path: an envelope, an oscillator, a filter, an amplifier, an output.
///
/// The same graph EVD-0009 measured, so the two records describe one path.
fn voice_path(cutoff: f32, attack: f32, decay: f32, sustain: f32, release: f32) -> GraphIr {
    GraphIr::builder()
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(attack).expect("not negative"),
                decay: Seconds::new(decay).expect("not negative"),
                sustain: NormalizedLevel::new(sustain).expect("within range"),
                release: Seconds::new(release).expect("not negative"),
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
                cutoff: CutoffFrequency::new(cutoff).expect("positive"),
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

fn profile(layout: ChannelLayout) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("a valid rate"),
        FrameCount::new(Q as u64),
        layout,
    )
    .expect("a valid harness profile")
}

/// One prepared record of the wanted kind, out of a compiled plan.
///
/// **By variant, never by position.** A kernel handed a record of the wrong kind returns
/// without writing — the right thing on the audio thread — so an arm built by position
/// becomes a measurement of early returns that nothing about it looks wrong. EVD-0009's
/// first correction was exactly this.
fn prepared_of(plan: &CompiledPlan, wanted: fn(&PreparedNode) -> bool) -> PreparedNode {
    plan.prepared_nodes()
        .iter()
        .copied()
        .find(wanted)
        .expect("the voice path has this node")
}

// ---------------------------------------------------------------------------
// The planar calls: the crate's kernels, one non-inlined direct call per node
// ---------------------------------------------------------------------------
//
// One shim per kernel rather than one shim taking a function pointer. The pointer form
// compiles to an indirect call, and since a planar chain has more nodes than an
// interleaved one, charging it that per node would put dispatch cost inside a layout
// measurement. The boundary itself stays: without it the optimizer is free to fuse a
// whole chain into one pass and report the layout question as answered.

macro_rules! shim {
    ($name:ident, $kernel:path) => {
        /// The crate's kernel, one non-inlined call, under the signature every kernel in
        /// this harness is called with.
        #[inline(never)]
        fn $name(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
            $kernel(prepared, state, io);
        }
    };
}

shim!(call_sine, sine);
shim!(call_filter, filter);
shim!(call_envelope, envelope);
shim!(call_amplifier, amplifier);
shim!(call_copy, copy);

// ---------------------------------------------------------------------------
// The interleaved kernels, under the same contract the crate's kernels have
// ---------------------------------------------------------------------------

/// What an interleaved node would have prepared.
///
/// The crate's [`PreparedNode`] cannot express it — a filter over an interleaved buffer
/// needs the coefficients of the channels it steps — so this is its counterpart, with the
/// same property that matters: a kernel destructures it and **returns without writing**
/// if it is the wrong variant, rather than asserting on the audio thread.
#[derive(Debug, Clone, Copy)]
enum InterPrepared {
    /// A widening, which carries nothing.
    Widen,
    /// One filter setting, applied to every channel of the frame.
    Filter {
        /// The three integrator coefficients.
        integrator: [f32; 3],
    },
    /// A filter setting per channel.
    FilterPerChannel {
        /// One set of integrator coefficients per channel.
        integrator: [[f32; 3]; CHANNELS],
    },
    /// An amplifier, which carries nothing.
    Amplifier,
}

/// What an interleaved node would keep between quanta.
///
/// A filter's history is per channel, which is the one thing interleaving changes about a
/// node's state: one record, `n` channels wide, rather than `n` records.
#[derive(Debug, Clone, Copy)]
enum InterState {
    /// A node that keeps nothing.
    Stateless,
    /// The two integrator states, per channel.
    Filter {
        /// The band-pass integrator of each channel.
        band: [f32; CHANNELS],
        /// The low-pass integrator of each channel.
        low: [f32; CHANNELS],
    },
}

/// A mono signal widened into an interleaved stereo buffer.
///
/// The interleaved layout's counterpart of the `copy` kernel the compiler schedules for
/// the planar one: ADR-0002 clause 7 makes the widening a scheduled operation with its own
/// buffer in either layout.
///
/// Neither the prepared record nor the state is read, because the crate's `copy` reads
/// neither: a kernel that checked a discriminant its counterpart does not would put a
/// branch on one side of the comparison only.
#[inline(never)]
fn widen_interleaved(_prepared: &InterPrepared, _state: &mut InterState, io: &mut NodeIo<'_>) {
    let InputBuffer::Patched(source) = io.inputs[0] else {
        io.out.fill(0.0);
        return;
    };
    for (frame, value) in io.out.chunks_exact_mut(CHANNELS).zip(source.iter()) {
        for slot in frame.iter_mut() {
            *slot = *value;
        }
    }
}

/// The state-variable filter over an interleaved buffer, one setting for every channel.
///
/// The crate's [`filter`] arithmetic, in the crate's order, stepping frames instead of
/// samples. Each channel's own sequence of operations over its own state is identical to
/// the planar arm's, which is why the two arms are required to agree **bit for bit**.
#[inline(never)]
fn filter_interleaved(prepared: &InterPrepared, state: &mut InterState, io: &mut NodeIo<'_>) {
    let InterPrepared::Filter { integrator } = prepared else {
        return;
    };
    let InterState::Filter { band, low } = state else {
        return;
    };
    let source = io.inputs[0];
    for frame in io.out.chunks_exact_mut(CHANNELS) {
        for (channel, sample) in frame.iter_mut().enumerate() {
            let (Some(first), Some(second)) = (band.get_mut(channel), low.get_mut(channel)) else {
                continue;
            };
            let input = match source {
                InputBuffer::InPlace => *sample,
                InputBuffer::Patched(_) | InputBuffer::Unpatched => 0.0,
            };
            let drive = input - *second;
            let band_pass = integrator[0] * *first + integrator[1] * drive;
            let low_pass = *second + integrator[1] * *first + integrator[2] * drive;
            *first = 2.0 * band_pass - *first;
            *second = 2.0 * low_pass - *second;
            *sample = low_pass;
        }
    }
    // Once per quantum per channel, as the crate's kernel does once per quantum.
    for (first, second) in band.iter_mut().zip(low.iter_mut()) {
        *first = flush(*first);
        *second = flush(*second);
    }
}

/// The same filter, with a setting of its own per channel — shape C.
#[inline(never)]
fn filter_interleaved_split(prepared: &InterPrepared, state: &mut InterState, io: &mut NodeIo<'_>) {
    let InterPrepared::FilterPerChannel { integrator } = prepared else {
        return;
    };
    let InterState::Filter { band, low } = state else {
        return;
    };
    let source = io.inputs[0];
    for frame in io.out.chunks_exact_mut(CHANNELS) {
        for (channel, sample) in frame.iter_mut().enumerate() {
            let (Some(first), Some(second), Some(coefficients)) = (
                band.get_mut(channel),
                low.get_mut(channel),
                integrator.get(channel),
            ) else {
                continue;
            };
            let input = match source {
                InputBuffer::InPlace => *sample,
                InputBuffer::Patched(_) | InputBuffer::Unpatched => 0.0,
            };
            let drive = input - *second;
            let band_pass = coefficients[0] * *first + coefficients[1] * drive;
            let low_pass = *second + coefficients[1] * *first + coefficients[2] * drive;
            *first = 2.0 * band_pass - *first;
            *second = 2.0 * low_pass - *second;
            *sample = low_pass;
        }
    }
    for (first, second) in band.iter_mut().zip(low.iter_mut()) {
        *first = flush(*first);
        *second = flush(*second);
    }
}

/// Zero, where a value is too small to be signal — the crate's rule, at its threshold.
fn flush(value: f32) -> f32 {
    if value.abs() < 1e-30 { 0.0 } else { value }
}

/// The amplifier over an interleaved buffer, in place, driven by **one mono control**.
///
/// The control is read **once per frame**, not once per sample with an index division to
/// find it. The per-sample form is the one EVD-0008 measured by mistake, and correcting it
/// moved that record's result by more than twenty points.
///
/// Reads neither the prepared record nor the state, because the crate's `amplifier` reads
/// neither.
#[inline(never)]
fn amplifier_interleaved(_prepared: &InterPrepared, _state: &mut InterState, io: &mut NodeIo<'_>) {
    let InputBuffer::Patched(control) = io.inputs[1] else {
        io.out.fill(0.0);
        return;
    };
    for (frame, level) in io.out.chunks_exact_mut(CHANNELS).zip(control.iter()) {
        for sample in frame.iter_mut() {
            *sample *= *level;
        }
    }
}

/// The amplifier over an interleaved buffer, driven by **one mono control per channel**.
///
/// Shape C's case: two control signals, so the frame reads two values instead of one.
#[inline(never)]
fn amplifier_interleaved_split(
    _prepared: &InterPrepared,
    _state: &mut InterState,
    io: &mut NodeIo<'_>,
) {
    let [InputBuffer::Patched(left), InputBuffer::Patched(right)] = io.inputs else {
        io.out.fill(0.0);
        return;
    };
    for ((frame, first), second) in io
        .out
        .chunks_exact_mut(CHANNELS)
        .zip(left.iter())
        .zip(right.iter())
    {
        if let Some(sample) = frame.first_mut() {
            *sample *= *first;
        }
        if let Some(sample) = frame.get_mut(1) {
            *sample *= *second;
        }
    }
}

// ---------------------------------------------------------------------------
// The two boundary writes
// ---------------------------------------------------------------------------

/// One mono buffer into one channel of an interleaved carry — **the planar boundary**.
///
/// Written the way a competent planar implementation would: the frame stride is the
/// iterator's, so neither the source nor the destination is index-checked per sample. This
/// is the form the layout comparison uses, because the comparison is between layouts and
/// not between two standards of care.
#[inline(never)]
fn write_channel(source: &[f32], carry: &mut [f32], channel: usize) {
    for (frame, value) in carry.chunks_exact_mut(CHANNELS).zip(source.iter()) {
        if let Some(slot) = frame.get_mut(channel) {
            *slot = *value;
        }
    }
}

/// The same write, as `render/hot.rs` performs it today: index arithmetic per sample.
///
/// Not the comparison's planar form. It is timed as its own arm so the record can say what
/// the renderer's current loop costs against the one above — an implementation debt, which
/// is a different thing from a layout cost and must not be charged to one.
#[inline(never)]
fn write_channel_indexed(source: &[f32], carry: &mut [f32], channel: usize) {
    for frame in 0..Q {
        let value = source.get(frame).copied().unwrap_or(0.0);
        if let Some(slot) = carry.get_mut(frame * CHANNELS + channel) {
            *slot = value;
        }
    }
}

/// One interleaved buffer into the carry — the interleaved boundary.
#[inline(never)]
fn write_interleaved(source: &[f32], carry: &mut [f32]) {
    let samples = source.len().min(carry.len());
    if let (Some(from), Some(into)) = (source.get(..samples), carry.get_mut(..samples)) {
        into.copy_from_slice(from);
    }
}

// ---------------------------------------------------------------------------
// Arena borrowing
// ---------------------------------------------------------------------------

/// One region of an arm's flat arena.
#[derive(Debug, Clone, Copy)]
struct Region {
    start: usize,
    len: usize,
}

impl Region {
    const fn new(start: usize, len: usize) -> Self {
        Self { start, len }
    }

    const fn end(self) -> usize {
        self.start + self.len
    }
}

/// One mutable region.
#[inline(always)]
fn one(arena: &mut [f32], region: Region) -> &mut [f32] {
    arena
        .get_mut(region.start..region.end())
        .expect("the arm's arena holds its own regions")
}

/// One mutable region and one shared region below or above it, without `unsafe`.
///
/// Inlined deliberately. A real engine resolves an operation's regions in `bind`, whose
/// cost `walk_only` and `bind_only` attribute separately; leaving this as a call of its
/// own would charge each arm harness plumbing in proportion to how many multi-region
/// operations it has — which is one of the things the layouts differ in.
#[inline(always)]
fn two(arena: &mut [f32], out: Region, input: Region) -> (&mut [f32], &[f32]) {
    assert!(
        out.end() <= input.start || input.end() <= out.start,
        "an arm's output and its input must be disjoint regions of its arena"
    );
    if out.end() <= input.start {
        let (head, tail) = arena.split_at_mut(input.start);
        let written = head
            .get_mut(out.start..out.end())
            .expect("the output lies below the input");
        let read = tail.get(..input.len).expect("the input starts the tail");
        (written, read)
    } else {
        let (head, tail) = arena.split_at_mut(out.start);
        let written = tail.get_mut(..out.len).expect("the output starts the tail");
        let read = head
            .get(input.start..input.end())
            .expect("the input lies below the output");
        (written, read)
    }
}

/// One mutable region at the front of the arena, and two shared regions above it.
#[inline(always)]
fn three(
    arena: &mut [f32],
    out: Region,
    first: Region,
    second: Region,
) -> (&mut [f32], &[f32], &[f32]) {
    assert!(
        out.start == 0 && out.end() <= first.start && out.end() <= second.start,
        "the three-region borrow needs the written region first in the arena"
    );
    let (head, tail) = arena.split_at_mut(out.len);
    let written = head.get_mut(..out.len).expect("the output is the head");
    let read_first = tail
        .get(first.start - out.len..first.end() - out.len)
        .expect("the first input lies above the output");
    let read_second = tail
        .get(second.start - out.len..second.end() - out.len)
        .expect("the second input lies above the output");
    (written, read_first, read_second)
}

// ---------------------------------------------------------------------------
// The planar arms
// ---------------------------------------------------------------------------

/// A planar arm: one buffer is one channel, and a stereo signal occupies two of them.
///
/// ADR-0002 clauses 1 and 2, as the crate implements them. The region maps mirror what a
/// linear-scan arena assigns to each shape's schedule, including its reuse: shape A's
/// widening writes into the buffer the envelope has finished with — which is the
/// assignment `compile` produced, asserted against it — and shape C's second envelope
/// writes into the buffer its first one has finished with.
struct Planar {
    sine: PreparedNode,
    sine_state: NodeState,
    filter_left: PreparedNode,
    filter_left_state: NodeState,
    filter_right: PreparedNode,
    filter_right_state: NodeState,
    envelope_left: PreparedNode,
    envelope_left_state: NodeState,
    envelope_right: PreparedNode,
    envelope_right_state: NodeState,
    amplifier: PreparedNode,
    amplifier_state: NodeState,
    copy: PreparedNode,
    copy_state: NodeState,
    arena: Vec<f32>,
    carry: Vec<f32>,
    left: Region,
    right: Region,
    control_left: Region,
    control_right: Region,
    /// A gate edge queued by `open_gate`, due at offset 0 of the next quantum this arm
    /// renders and cleared there, so only that quantum carries it.
    pending_gate: Vec<TimedControl>,
}

impl Planar {
    /// `buffers` is how many `Q`-frame slots this shape's schedule needs; the indices are
    /// which slot each further signal is assigned.
    fn new(records: &Records, buffers: usize, right: usize, controls: (usize, usize)) -> Self {
        Self {
            sine: records.sine,
            sine_state: NodeState::initial(&records.sine),
            filter_left: records.filter_left,
            filter_left_state: NodeState::initial(&records.filter_left),
            filter_right: records.filter_right,
            filter_right_state: NodeState::initial(&records.filter_right),
            envelope_left: records.envelope_left,
            envelope_left_state: NodeState::initial(&records.envelope_left),
            envelope_right: records.envelope_right,
            envelope_right_state: NodeState::initial(&records.envelope_right),
            amplifier: records.amplifier,
            amplifier_state: NodeState::initial(&records.amplifier),
            copy: records.copy,
            copy_state: NodeState::initial(&records.copy),
            arena: vec![0.0; buffers * Q],
            carry: vec![0.0; Q * CHANNELS],
            left: Region::new(0, Q),
            right: Region::new(right * Q, Q),
            control_left: Region::new(controls.0 * Q, Q),
            control_right: Region::new(controls.1 * Q, Q),
            pending_gate: Vec::new(),
        }
    }

    /// Hold the gate on both envelopes.
    fn open_gate(&mut self) {
        // The edge is **queued**, not rendered. A gate is sample-positioned since
        // P02-T007, so it reaches the envelope with the buffer of the quantum it falls
        // in — and rendering one here to apply it would advance this arm's envelope by a
        // quantum the renderer arm has not rendered, which the per-quantum comparison
        // below catches as a disagreement at quantum 0. The queued edge is consumed by
        // the first quantum this arm renders and cleared there, which is where the
        // renderer arm's gate event lands too.
        self.pending_gate = vec![held_gate()];
    }

    /// **Shape A**: the schedule the compiler produced for a stereo profile.
    ///
    /// Sine, filter, envelope, amplifier, the widening copy, and one output write per
    /// channel — in that order, over the two buffers the arena assigned.
    fn voice(&mut self) {
        self.voice_nodes();
        self.write_carry();
    }

    /// The same shape, ending in the carry write `render/hot.rs` performs today.
    fn voice_indexed(&mut self) {
        self.voice_nodes();
        self.write_carry_indexed();
    }

    /// Shape A's five node operations, without the plan's output operations.
    #[inline(always)]
    fn voice_nodes(&mut self) {
        call_sine(
            &self.sine,
            &mut self.sine_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &[],
                ramps: sine_ramp(),
            },
        );
        call_filter(
            &self.filter_left,
            &mut self.filter_left_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_envelope(
            &self.envelope_left,
            &mut self.envelope_left_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.control_left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &self.pending_gate,
                ramps: &[],
            },
        );
        let (out, control) = two(&mut self.arena, self.left, self.control_left);
        call_amplifier(
            &self.amplifier,
            &mut self.amplifier_state,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Patched(control)],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        // The widening, into the slot the envelope has finished with.
        let (out, source) = two(&mut self.arena, self.right, self.left);
        call_copy(
            &self.copy,
            &mut self.copy_state,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Patched(source), InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        // The queued gate edge is consumed by this quantum and by no later one, which is
        // what makes it one edge rather than a gate re-asserted every quantum.
        self.pending_gate.clear();
    }

    /// **Shape B**: the widening upstream, so the filter and the amplifier run per channel.
    fn bus(&mut self) {
        call_sine(
            &self.sine,
            &mut self.sine_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &[],
                ramps: sine_ramp(),
            },
        );
        let (out, source) = two(&mut self.arena, self.right, self.left);
        call_copy(
            &self.copy,
            &mut self.copy_state,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Patched(source), InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_filter(
            &self.filter_left,
            &mut self.filter_left_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_filter(
            &self.filter_left,
            &mut self.filter_right_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.right),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_envelope(
            &self.envelope_left,
            &mut self.envelope_left_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.control_left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &self.pending_gate,
                ramps: &[],
            },
        );
        let (out, control) = two(&mut self.arena, self.left, self.control_left);
        call_amplifier(
            &self.amplifier,
            &mut self.amplifier_state,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Patched(control)],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        let (out, control) = two(&mut self.arena, self.right, self.control_left);
        call_amplifier(
            &self.amplifier,
            &mut self.amplifier_state,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Patched(control)],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        self.write_carry();
        // The queued gate edge is consumed by this quantum and by no later one, which is
        // what makes it one edge rather than a gate re-asserted every quantum.
        self.pending_gate.clear();
    }

    /// **Shape C**: shape B with a filter and an envelope of its own per channel.
    ///
    /// Ordered envelope, amplifier, envelope, amplifier rather than both envelopes first,
    /// because that is what lets the second control reuse the first one's slot — an economy
    /// this layout has and the interleaved one does not, since a single interleaved
    /// amplifier call needs both controls live at once.
    fn split(&mut self) {
        call_sine(
            &self.sine,
            &mut self.sine_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &[],
                ramps: sine_ramp(),
            },
        );
        let (out, source) = two(&mut self.arena, self.right, self.left);
        call_copy(
            &self.copy,
            &mut self.copy_state,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Patched(source), InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_filter(
            &self.filter_left,
            &mut self.filter_left_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_filter(
            &self.filter_right,
            &mut self.filter_right_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.right),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_envelope(
            &self.envelope_left,
            &mut self.envelope_left_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.control_left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &self.pending_gate,
                ramps: &[],
            },
        );
        let (out, control) = two(&mut self.arena, self.left, self.control_left);
        call_amplifier(
            &self.amplifier,
            &mut self.amplifier_state,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Patched(control)],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_envelope(
            &self.envelope_right,
            &mut self.envelope_right_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.control_right),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &self.pending_gate,
                ramps: &[],
            },
        );
        let (out, control) = two(&mut self.arena, self.right, self.control_right);
        call_amplifier(
            &self.amplifier,
            &mut self.amplifier_state,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Patched(control)],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        self.write_carry();
        // The queued gate edge is consumed by this quantum and by no later one, which is
        // what makes it one edge rather than a gate re-asserted every quantum.
        self.pending_gate.clear();
    }

    /// The plan's output operations: one strided write per channel.
    ///
    /// Two methods rather than one taking the write to perform. Selecting it through a
    /// function pointer compiles to an indirect call, and only the planar arms would have
    /// had one — the same class of asymmetry the shims above exist to avoid.
    fn write_carry(&mut self) {
        let (audio, carry) = (&self.arena, &mut self.carry);
        if let Some(source) = audio.get(self.left.start..self.left.end()) {
            write_channel(source, carry, 0);
        }
        if let Some(source) = audio.get(self.right.start..self.right.end()) {
            write_channel(source, carry, 1);
        }
    }

    /// The same, as `render/hot.rs` writes it today.
    fn write_carry_indexed(&mut self) {
        let (audio, carry) = (&self.arena, &mut self.carry);
        if let Some(source) = audio.get(self.left.start..self.left.end()) {
            write_channel_indexed(source, carry, 0);
        }
        if let Some(source) = audio.get(self.right.start..self.right.end()) {
            write_channel_indexed(source, carry, 1);
        }
    }
}

// ---------------------------------------------------------------------------
// The interleaved arms
// ---------------------------------------------------------------------------

/// An interleaved arm: a stereo signal is one buffer of `Q` frames of two channels.
///
/// Written the way a competent implementer would, which is the correction EVD-0008's third
/// revision needed: **a mono signal stays contiguous** — the crate's own kernels run over
/// it unchanged — and only a signal that genuinely has channels is stepped frame by frame.
///
/// The region map is this layout's own liveness. The stereo buffer is first because a
/// two-input call borrows it as the written region; the mono buffer dies at the widening,
/// so a control can have its slot; and shape A's control dies before the stereo buffer is
/// born, so it can live inside it.
struct Interleaved {
    sine: PreparedNode,
    sine_state: NodeState,
    /// Shape A's filter is mono, so it is the crate's kernel over the crate's records: an
    /// interleaved arena stores a mono signal exactly as a planar one does.
    filter_mono: PreparedNode,
    filter_mono_state: NodeState,
    amplifier: PreparedNode,
    amplifier_state: NodeState,
    /// Shapes B and C, where the signal genuinely has channels.
    filter_stereo: InterPrepared,
    filter_stereo_state: InterState,
    widen: InterPrepared,
    amplifier_stereo: InterPrepared,
    stateless: InterState,
    envelope_left: PreparedNode,
    envelope_left_state: NodeState,
    envelope_right: PreparedNode,
    envelope_right_state: NodeState,
    arena: Vec<f32>,
    carry: Vec<f32>,
    stereo: Region,
    mono: Region,
    control_left: Region,
    control_right: Region,
    /// A gate edge queued by `open_gate`, due at offset 0 of the next quantum this arm
    /// renders and cleared there, so only that quantum carries it.
    pending_gate: Vec<TimedControl>,
}

impl Interleaved {
    /// `per_channel` is shape C: a filter setting of its own on the second channel, and a
    /// second control that has to be live at the same time as the first.
    fn new(records: &Records, per_channel: bool) -> Self {
        let integrator = |prepared: &PreparedNode| match prepared {
            PreparedNode::Filter { integrator } => *integrator,
            _ => [0.0; 3],
        };
        // Peak liveness, not a slot per signal. The stereo buffer is `2Q`; the mono buffer
        // is dead once the widening has read it, so a control takes its slot; and only a
        // second, simultaneously live control needs another.
        let slots = if per_channel {
            CHANNELS + 2
        } else {
            CHANNELS + 1
        };
        let filter_stereo = if per_channel {
            InterPrepared::FilterPerChannel {
                integrator: [
                    integrator(&records.filter_left),
                    integrator(&records.filter_right),
                ],
            }
        } else {
            InterPrepared::Filter {
                integrator: integrator(&records.filter_left),
            }
        };
        Self {
            sine: records.sine,
            sine_state: NodeState::initial(&records.sine),
            filter_mono: records.filter_left,
            filter_mono_state: NodeState::initial(&records.filter_left),
            amplifier: records.amplifier,
            amplifier_state: NodeState::initial(&records.amplifier),
            filter_stereo,
            filter_stereo_state: InterState::Filter {
                band: [0.0; CHANNELS],
                low: [0.0; CHANNELS],
            },
            widen: InterPrepared::Widen,
            amplifier_stereo: InterPrepared::Amplifier,
            stateless: InterState::Stateless,
            envelope_left: records.envelope_left,
            envelope_left_state: NodeState::initial(&records.envelope_left),
            envelope_right: records.envelope_right,
            envelope_right_state: NodeState::initial(&records.envelope_right),
            arena: vec![0.0; slots * Q],
            carry: vec![0.0; Q * CHANNELS],
            stereo: Region::new(0, CHANNELS * Q),
            mono: Region::new(CHANNELS * Q, Q),
            // Shape A's control lives inside the stereo buffer, which is not yet written
            // when the amplifier reads it; shapes B and C's live in the mono buffer's slot,
            // which the widening has finished with.
            control_left: Region::new(0, Q),
            control_right: Region::new((CHANNELS + 1) * Q, Q),
            pending_gate: Vec::new(),
        }
    }

    /// Shapes B and C put their first control where the mono buffer was.
    fn with_stereo_controls(mut self) -> Self {
        self.control_left = Region::new(CHANNELS * Q, Q);
        self
    }

    fn open_gate(&mut self) {
        // The edge is **queued**, not rendered. A gate is sample-positioned since
        // P02-T007, so it reaches the envelope with the buffer of the quantum it falls
        // in — and rendering one here to apply it would advance this arm's envelope by a
        // quantum the renderer arm has not rendered, which the per-quantum comparison
        // below catches as a disagreement at quantum 0. The queued edge is consumed by
        // the first quantum this arm renders and cleared there, which is where the
        // renderer arm's gate event lands too.
        self.pending_gate = vec![held_gate()];
    }

    /// **Shape A**: the mono chain contiguous, widened once, copied to the carry.
    ///
    /// The single-channel half runs the crate's kernels over a contiguous buffer, because
    /// an interleaved arena stores a mono signal exactly as a planar one does. Charging it
    /// a stride here is the straw man EVD-0008's third correction removed.
    fn voice(&mut self) {
        call_sine(
            &self.sine,
            &mut self.sine_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.mono),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &[],
                ramps: sine_ramp(),
            },
        );
        call_filter(
            &self.filter_mono,
            &mut self.filter_mono_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.mono),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_envelope(
            &self.envelope_left,
            &mut self.envelope_left_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.control_left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &self.pending_gate,
                ramps: &[],
            },
        );
        let (out, control) = two(&mut self.arena, self.mono, self.control_left);
        call_amplifier(
            &self.amplifier,
            &mut self.amplifier_state,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::InPlace, InputBuffer::Patched(control)],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        let (out, source) = two(&mut self.arena, self.stereo, self.mono);
        widen_interleaved(
            &self.widen,
            &mut self.stateless,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Stereo,
                inputs: [InputBuffer::Patched(source), InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        self.write_carry();
        // The queued gate edge is consumed by this quantum and by no later one, which is
        // what makes it one edge rather than a gate re-asserted every quantum.
        self.pending_gate.clear();
    }

    /// **Shape B**: widened upstream, then a filter and an amplifier over frames.
    fn bus(&mut self) {
        call_sine(
            &self.sine,
            &mut self.sine_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.mono),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &[],
                ramps: sine_ramp(),
            },
        );
        let (out, source) = two(&mut self.arena, self.stereo, self.mono);
        widen_interleaved(
            &self.widen,
            &mut self.stateless,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Stereo,
                inputs: [InputBuffer::Patched(source), InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        filter_interleaved(
            &self.filter_stereo,
            &mut self.filter_stereo_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.stereo),
                channels: synth_engine_v2::quantities::ChannelLayout::Stereo,
                inputs: [InputBuffer::InPlace, InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_envelope(
            &self.envelope_left,
            &mut self.envelope_left_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.control_left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &self.pending_gate,
                ramps: &[],
            },
        );
        let (out, control) = two(&mut self.arena, self.stereo, self.control_left);
        amplifier_interleaved(
            &self.amplifier_stereo,
            &mut self.stateless,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Stereo,
                inputs: [InputBuffer::InPlace, InputBuffer::Patched(control)],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        self.write_carry();
        // The queued gate edge is consumed by this quantum and by no later one, which is
        // what makes it one edge rather than a gate re-asserted every quantum.
        self.pending_gate.clear();
    }

    /// **Shape C**: the same, with a control signal per channel — two reads per frame.
    fn split(&mut self) {
        call_sine(
            &self.sine,
            &mut self.sine_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.mono),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &[],
                ramps: sine_ramp(),
            },
        );
        let (out, source) = two(&mut self.arena, self.stereo, self.mono);
        widen_interleaved(
            &self.widen,
            &mut self.stateless,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Stereo,
                inputs: [InputBuffer::Patched(source), InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        filter_interleaved_split(
            &self.filter_stereo,
            &mut self.filter_stereo_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.stereo),
                channels: synth_engine_v2::quantities::ChannelLayout::Stereo,
                inputs: [InputBuffer::InPlace, InputBuffer::Unpatched],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        call_envelope(
            &self.envelope_left,
            &mut self.envelope_left_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.control_left),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &self.pending_gate,
                ramps: &[],
            },
        );
        call_envelope(
            &self.envelope_right,
            &mut self.envelope_right_state,
            &mut NodeIo {
                out: one(&mut self.arena, self.control_right),
                channels: synth_engine_v2::quantities::ChannelLayout::Mono,
                inputs: [InputBuffer::Unpatched; MAX_INPUTS],
                position: None,
                controls: &self.pending_gate,
                ramps: &[],
            },
        );
        let (out, left, right) = three(
            &mut self.arena,
            self.stereo,
            self.control_left,
            self.control_right,
        );
        amplifier_interleaved_split(
            &self.amplifier_stereo,
            &mut self.stateless,
            &mut NodeIo {
                out,
                channels: synth_engine_v2::quantities::ChannelLayout::Stereo,
                inputs: [InputBuffer::Patched(left), InputBuffer::Patched(right)],
                position: None,
                controls: &[],
                ramps: &[],
            },
        );
        self.write_carry();
        // The queued gate edge is consumed by this quantum and by no later one, which is
        // what makes it one edge rather than a gate re-asserted every quantum.
        self.pending_gate.clear();
    }

    /// The plan's output operation: one contiguous copy.
    fn write_carry(&mut self) {
        if let Some(source) = self.arena.get(self.stereo.start..self.stereo.end()) {
            write_interleaved(source, &mut self.carry);
        }
    }
}

/// The prepared records every arm is built from — the **crate's**, never the harness's.
struct Records {
    sine: PreparedNode,
    filter_left: PreparedNode,
    filter_right: PreparedNode,
    envelope_left: PreparedNode,
    envelope_right: PreparedNode,
    amplifier: PreparedNode,
    copy: PreparedNode,
}

/// The schedule and arena of the real stereo plan, for the binding attribution.
struct Steps {
    steps: Vec<NodeStep>,
    arena: Vec<f32>,
    /// Where each slot's samples live, which `bind` resolves rather than computing.
    regions: Vec<synth_engine_v2::plan::BufferRegion>,
}

impl Steps {
    fn new(plan: &CompiledPlan) -> Self {
        Self {
            steps: plan
                .ops()
                .iter()
                .filter_map(|op| match op {
                    PlanOp::Node(step) => Some(*step),
                    PlanOp::Output { .. } => None,
                })
                .collect(),
            // ADR-0041 clause 2: `bind` reads the regions the plan records, and since
            // the widening writes `c * Q` they are no longer uniform. Fabricating them
            // as `slot * Q` would bind this arm to a layout the plan does not have.
            regions: plan.regions().to_vec(),
            arena: vec![0.0; plan.arena_samples()],
        }
    }

    /// The schedule walked, and nothing else — the binding's control.
    fn walk_only(&mut self) {
        for step in &self.steps {
            black_box(step);
        }
    }

    /// The schedule walked and every step's slots bound — and nothing called.
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
}

/// Time `iterations` runs of `body`, in seconds per iteration.
fn timed(iterations: u32, mut body: impl FnMut()) -> f64 {
    let start = Instant::now();
    for _ in 0..iterations {
        body();
    }
    start.elapsed().as_secs_f64() / f64::from(iterations)
}

/// Whether two carries are the same bits, and every sample of them a number.
///
/// Bits rather than a difference: `f32::max` ignores a `NaN`, so a difference-based check
/// passes on a carry that is partly `NaN` — which is what a kernel reading arena memory
/// nothing wrote would produce.
fn identical(first: &[f32], second: &[f32]) -> bool {
    first.len() == second.len()
        && first
            .iter()
            .zip(second.iter())
            .all(|(a, b)| a.to_bits() == b.to_bits() && a.is_finite())
}

/// The loudest sample of a carry.
fn peak(carry: &[f32]) -> f32 {
    carry
        .iter()
        .copied()
        .fold(0.0_f32, |loudest, value| loudest.max(value.abs()))
}

/// Shape A's planar arm mirrors the compiler's assignment operation by operation.
///
/// A count of buffers and operations is not enough: a compiler that assigned different
/// slots, or dropped an in-place binding, would keep every count and leave this arm
/// measuring a memory layout the plan does not have.
fn assert_shape_a_is_the_compiled_plan(plan: &CompiledPlan) {
    let buffers = plan.buffer_count();
    assert!(
        buffers == 3,
        "shape A mirrors a three-region assignment since the widening takes `c * Q` of \
         its own, and the compiler produced {buffers}"
    );
    // Kind, written slot, and the binding of each input — in schedule order.
    let expected: [(&str, usize, [Option<&str>; MAX_INPUTS]); 5] = [
        ("Sine", 0, [None, None]),
        ("Filter", 0, [Some("InPlace"), None]),
        ("Envelope", 1, [None, None]),
        ("Amplifier", 0, [Some("InPlace"), Some("Distinct")]),
        // The widening cannot reuse the envelope's freed region: that one is `Q` and
        // this one needs `2Q`, which is ADR-0041 clause 14's in-place rule seen from the
        // allocator's side.
        ("Copy", 2, [Some("Distinct"), None]),
    ];
    let mut seen = 0;
    let mut channels = Vec::new();
    for op in plan.ops() {
        match op {
            PlanOp::Node(step) => {
                let kind = match plan.prepared_nodes().get(step.node().index()) {
                    Some(PreparedNode::Sine { .. }) => "Sine",
                    Some(PreparedNode::Filter { .. }) => "Filter",
                    Some(PreparedNode::Envelope { .. }) => "Envelope",
                    Some(PreparedNode::Amplifier) => "Amplifier",
                    Some(PreparedNode::Copy) => "Copy",
                    other => panic!("shape A has no node of kind {other:?}"),
                };
                let (wanted_kind, wanted_slot, wanted_bindings) = expected
                    .get(seen)
                    .copied()
                    .expect("the compiler scheduled more nodes than shape A has");
                let bindings = format!("{:?}", step.bindings());
                assert!(
                    kind == wanted_kind && step.out().index() == wanted_slot,
                    "shape A's operation {seen} is {kind} into slot {} and the arm mirrors \
                     {wanted_kind} into slot {wanted_slot}",
                    step.out().index()
                );
                for (index, wanted) in wanted_bindings.iter().enumerate() {
                    if let Some(wanted) = wanted {
                        assert!(
                            bindings.contains(wanted),
                            "shape A's {kind} input {index} should bind {wanted} and the \
                             compiler produced {bindings}"
                        );
                    }
                }
                seen += 1;
            }
            PlanOp::Output { source } => {
                // Since ADR-0041 the boundary is one operation over one interleaved
                // region, so what there is to check is which region it reads — not a
                // write per channel, which is the planar shape this harness's *planar*
                // arm models by hand.
                channels.push((source.index(), 0));
            }
        }
    }
    assert!(
        seen == expected.len(),
        "shape A has {} node operations and the compiler produced {seen}",
        expected.len()
    );
    assert!(
        channels.len() == 1,
        "shape A ends in one output operation over one interleaved region; the compiler \
         produced {channels:?}"
    );
}

fn main() {
    // An unparsed or zero count is refused rather than defaulted: zero rounds leaves every
    // arm at `f64::MAX` and every ratio a `NaN`, which prints as a result.
    let mut arguments = std::env::args().skip(1);
    let mut count = |name: &str, fallback: u32| -> u32 {
        match arguments.next() {
            None => fallback,
            Some(value) => match value.parse::<u32>() {
                Ok(parsed) if parsed > 0 => parsed,
                _ => {
                    eprintln!("{name} must be a positive whole number, and is {value:?}");
                    std::process::exit(2);
                }
            },
        }
    };
    let rounds = count("rounds", 25);
    let iterations = count("iterations", 50_000);

    // The real stereo plan: shape A's planar arm is the schedule this produced, and the
    // `renderer` arm runs it through the real renderer.
    let stereo = compile(
        &voice_path(2_000.0, 0.01, 0.1, 0.7, 0.2),
        &RenderConfig::new(profile(ChannelLayout::Stereo)),
    )
    .into_plan()
    .expect("the minimal voice path is admitted");
    // Shape C's second channel: a different filter and a different envelope, prepared by
    // the crate's own compiler rather than by arithmetic written here.
    let alternate = compile(
        &voice_path(800.0, 0.02, 0.05, 0.5, 0.3),
        &RenderConfig::new(profile(ChannelLayout::Mono)),
    )
    .into_plan()
    .expect("the alternate voice path is admitted");

    assert_shape_a_is_the_compiled_plan(&stereo);

    let records = Records {
        sine: prepared_of(&stereo, |node| matches!(node, PreparedNode::Sine { .. })),
        filter_left: prepared_of(&stereo, |node| matches!(node, PreparedNode::Filter { .. })),
        filter_right: prepared_of(&alternate, |node| {
            matches!(node, PreparedNode::Filter { .. })
        }),
        envelope_left: prepared_of(&stereo, |node| {
            matches!(node, PreparedNode::Envelope { .. })
        }),
        envelope_right: prepared_of(&alternate, |node| {
            matches!(node, PreparedNode::Envelope { .. })
        }),
        amplifier: prepared_of(&stereo, |node| matches!(node, PreparedNode::Amplifier)),
        copy: prepared_of(&stereo, |node| matches!(node, PreparedNode::Copy)),
    };
    seed_sine_ramp(&records.sine);
    assert!(
        records.filter_left != records.filter_right
            && records.envelope_left != records.envelope_right,
        "shape C's two channels must differ, or it is shape B with more buffers"
    );

    // One instance per arm. Separate instances, because two arms sharing a state would let
    // one arm's quanta advance the other's phase and would make the agreement check below
    // compare an arm with itself.
    let mut voice_planar = Planar::new(&records, 2, 1, (1, 1));
    let mut voice_asis = Planar::new(&records, 2, 1, (1, 1));
    let mut bus_planar = Planar::new(&records, 3, 1, (2, 2));
    let mut split_planar = Planar::new(&records, 3, 1, (2, 2));
    let mut voice_inter = Interleaved::new(&records, false);
    let mut bus_inter = Interleaved::new(&records, false).with_stereo_controls();
    let mut split_inter = Interleaved::new(&records, true).with_stereo_controls();
    let mut steps = Steps::new(&stereo);

    for arm in [
        &mut voice_planar,
        &mut voice_asis,
        &mut bus_planar,
        &mut split_planar,
    ] {
        arm.open_gate();
    }
    for arm in [&mut voice_inter, &mut bus_inter, &mut split_inter] {
        arm.open_gate();
    }

    // The renderer arm: the real plan through the real renderer, gated on once and prepared
    // once, so no round times an allocation.
    let (_control, mut renderer) = StreamControl::open(
        stereo.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("an admitted plan prepares");
    let gate = stereo
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_GATE)
        .expect("the envelope declares a gate");
    let opened = [TimedEvent::new(
        EventEnvelope::new(renderer.epoch(), SampleTime::ZERO, TimeSource::Compiled),
        EventPayload::SetParameter {
            slot: gate,
            value: ParameterValue::new(1.0).expect("finite"),
        },
    )];
    let mut block = vec![0.0; Q * CHANNELS];
    // **One call before the gate, and it is not a formality.** `prepare` primes the carry
    // with `Q` frames of silence, so the first call of exactly `Q` frames serves that
    // priming and renders **no quantum at all** — and an event presented on it is outside
    // the call's span, which fails the whole call and takes the gate with it. The gate
    // therefore goes on the *second* call, and after this one the renderer renders exactly
    // one quantum per call, which is what the timed arm needs it to do.
    if let Ok(audio) = AudioBlockMut::new(&mut block, Q, ChannelLayout::Stereo) {
        let _ = renderer.render(audio, TimedEvents::EMPTY);
    }

    // **Every arm past its attack and decay into the sustain the timed quanta measure, from
    // its own state — and compared at every quantum on the way.** One settled block cannot
    // separate an arm that did the work from one that skipped it, because a held note looks
    // the same either way; a transient can.
    let settle = 200;
    for quantum in 0..settle {
        voice_planar.voice();
        voice_asis.voice_indexed();
        bus_planar.bus();
        split_planar.split();
        voice_inter.voice();
        bus_inter.bus();
        split_inter.split();
        // The gate opens on the renderer's **first rendering** call, because the hand arms
        // hold it before their first quantum. A renderer one quantum ahead would be compared
        // at a different moment of the same signal rather than as a different implementation
        // of it.
        let events = if quantum == 0 {
            TimedEvents::new(&opened)
        } else {
            TimedEvents::EMPTY
        };
        // A rejected call renders nothing and returns an error this harness must not
        // swallow: the gate arrived on a call that rendered no quantum once, and the
        // renderer went on producing silence while every timing still looked ordinary.
        match AudioBlockMut::new(&mut block, Q, ChannelLayout::Stereo) {
            Ok(audio) => {
                if let Err(error) = renderer.render(audio, events) {
                    println!("renderer_call_rejected,{error:?}");
                    return;
                }
            }
            Err(error) => {
                println!("renderer_block_refused,{error:?}");
                return;
            }
        }

        for (shape, planar, other) in [
            ("voice", &voice_planar.carry, &voice_inter.carry),
            ("bus", &bus_planar.carry, &bus_inter.carry),
            ("split", &split_planar.carry, &split_inter.carry),
            ("carry_form", &voice_planar.carry, &voice_asis.carry),
            ("renderer", &voice_planar.carry, &block),
        ] {
            if !identical(planar, other) {
                println!("arms_disagree_{shape},quantum,{quantum}");
                println!("the arms do not compute the same signal; no timing means anything");
                if shape == "renderer" {
                    println!("renderer_diagnostics,{:?}", renderer.diagnostics());
                }
                return;
            }
        }
    }
    let loudest = peak(&voice_planar.carry)
        .min(peak(&bus_planar.carry))
        .min(peak(&split_planar.carry))
        .min(peak(&voice_inter.carry))
        .min(peak(&bus_inter.carry))
        .min(peak(&split_inter.carry));
    if loudest <= 0.0 {
        println!("an_arm_carry_is_silent,{loudest:e}");
        return;
    }
    println!("arms_agree_over_quanta,{settle}");
    println!("arms_agree_quietest_peak,{loudest:.6}");

    let names = [
        "voice_planar_ctl",
        "voice_planar",
        "voice_asis_ctl",
        "voice_asis",
        "voice_inter_ctl",
        "voice_inter",
        "bus_planar_ctl",
        "bus_planar",
        "bus_inter_ctl",
        "bus_inter",
        "split_planar_ctl",
        "split_planar",
        "split_inter_ctl",
        "split_inter",
        "walk_only",
        "bind_only",
        "renderer",
    ];
    let mut best = [f64::MAX; 17];
    let mut rounds_seen: Vec<Vec<f64>> = vec![Vec::new(); names.len()];

    // A control and the arm it bounds are one group; groups rotate; the control is always
    // first inside its group.
    let groups: [&[usize]; 9] = [
        &[0, 1],
        &[2, 3],
        &[4, 5],
        &[6, 7],
        &[8, 9],
        &[10, 11],
        &[12, 13],
        &[14, 15],
        &[16],
    ];

    for round in 0..rounds {
        for offset in 0..groups.len() {
            let Some(group) = groups.get((round as usize + offset) % groups.len()) else {
                continue;
            };
            for arm in group.iter().copied() {
                let elapsed = match arm {
                    0 | 1 => timed(iterations, || {
                        voice_planar.voice();
                        black_box(&voice_planar.carry);
                    }),
                    2 | 3 => timed(iterations, || {
                        voice_asis.voice_indexed();
                        black_box(&voice_asis.carry);
                    }),
                    4 | 5 => timed(iterations, || {
                        voice_inter.voice();
                        black_box(&voice_inter.carry);
                    }),
                    6 | 7 => timed(iterations, || {
                        bus_planar.bus();
                        black_box(&bus_planar.carry);
                    }),
                    8 | 9 => timed(iterations, || {
                        bus_inter.bus();
                        black_box(&bus_inter.carry);
                    }),
                    10 | 11 => timed(iterations, || {
                        split_planar.split();
                        black_box(&split_planar.carry);
                    }),
                    12 | 13 => timed(iterations, || {
                        split_inter.split();
                        black_box(&split_inter.carry);
                    }),
                    14 => timed(iterations, || {
                        steps.walk_only();
                    }),
                    15 => timed(iterations, || {
                        steps.bind_only();
                        black_box(&steps.arena);
                    }),
                    _ => timed(iterations, || {
                        if let Ok(audio) = AudioBlockMut::new(&mut block, Q, ChannelLayout::Stereo)
                        {
                            let _ = renderer.render(audio, TimedEvents::EMPTY);
                        }
                        black_box(&block);
                    }),
                };
                if let Some(seen) = best.get_mut(arm)
                    && elapsed < *seen
                {
                    *seen = elapsed;
                }
                if let Some(seen) = rounds_seen.get_mut(arm) {
                    seen.push(elapsed);
                }
            }
        }
    }

    println!();
    println!("arm,seconds_per_quantum,nanoseconds_per_quantum");
    for (name, seconds) in names.iter().zip(best.iter()) {
        println!("{name},{seconds:.12},{:.3}", seconds * 1e9);
    }

    // The control against its arm inside the round both were measured in. Dividing two
    // independently aggregated medians cancels drift rather than measuring it.
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

    // A ratio of two independently selected minima can take each arm's best round out of a
    // different round; the paired median cannot.
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

    println!();
    for (shape, control, arm) in [
        ("voice_planar", 0, 1),
        ("voice_asis", 2, 3),
        ("voice_inter", 4, 5),
        ("bus_planar", 6, 7),
        ("bus_inter", 8, 9),
        ("split_planar", 10, 11),
        ("split_inter", 12, 13),
    ] {
        let (median, worst) = paired_spread(control, arm);
        println!("{shape}_control_spread_percent,{median:.2}");
        println!("{shape}_control_worst_percent,{worst:.2}");
    }

    println!();
    // Negative means the **interleaved** arrangement is cheaper, which is EVD-0008's sign
    // convention for the same comparison.
    for (shape, planar, interleaved) in [("voice", 1, 5), ("bus", 7, 9), ("split", 11, 13)] {
        println!(
            "{shape}_interleaved_vs_planar_paired_percent,{:.2}",
            paired_ratio(interleaved, planar)
        );
        println!(
            "{shape}_interleaved_vs_planar_unpaired_percent,{:.2}",
            (best[interleaved] / best[planar] - 1.0) * 100.0
        );
        println!(
            "{shape}_interleaved_vs_planar_paired_nanoseconds,{:.2}",
            paired_difference(interleaved, planar)
        );
    }

    println!();
    println!("schedule_steps,{}", steps.steps.len());
    println!(
        "binding_paired_nanoseconds,{:.2}",
        paired_difference(15, 14)
    );
    println!(
        "renderer_vs_voice_planar_paired_percent,{:.2}",
        paired_ratio(16, 1)
    );
    // The renderer's current output loop against the one the comparison uses. An
    // implementation debt in `render/hot.rs`, reported separately so it is not read as
    // something the planar layout costs.
    println!(
        "carry_indexed_vs_chunked_paired_nanoseconds,{:.2}",
        paired_difference(3, 1)
    );
    println!(
        "carry_indexed_vs_chunked_paired_percent,{:.2}",
        paired_ratio(3, 1)
    );
}
