//! The node kernels: the per-quantum work, and everything the render loop calls out to.
//!
//! [ADR-0004](../../../plans/v2/decisions/ADR-0004-native-node-representation.md) is what
//! this file implements. A node kind contributes one free function with a single
//! signature — prepared data, mutable state, and the arena slots it was assigned — and
//! the compiler resolves it to a function pointer once, at admission. The render loop
//! walks a schedule and calls through that pointer, so **adding a node adds no control
//! flow**: it adds a kernel here and a registry entry in [`super`].
//!
//! # Why this file is part of the checked region
//!
//! `tests/render_loop_purity.rs` scans `src/render/hot.rs` *and this file*, under the
//! same rules. ADR-0004 clause 4 requires that every callee reachable from the loop be
//! enumerable from source, and a dispatch that reached code the scan could not see would
//! cost the phase its real-time guarantee rather than merely some inlining. The callee
//! set is the registry, the registry names functions defined here, and a test asserts
//! both halves.
//!
//! Nothing here may allocate, lock, perform I/O, log, or panic. A kernel handed prepared
//! data or state of the wrong variant returns without writing rather than asserting:
//! admission pairs the two, so a mismatch is a compiler defect, and the audio thread is
//! the one place that cannot report it.

use crate::plan::{InputBinding, NodeStep};
use crate::quantities::{
    Amplitude, Frequency, GainFactor, NormalizedLevel, ParameterValue, SegmentFrames,
};
use crate::time::PlanPosition;

/// How many inputs one kernel may be handed.
///
/// A bound rather than a guess: the binding below hands out one mutable and up to this
/// many shared borrows of one arena, and doing that without aliasing needs a fixed
/// number of them. Raising it is a deliberate change to the kernel signature.
pub const MAX_INPUTS: usize = 2;

/// The one kernel signature.
///
/// ADR-0004 clause 5: prepared data, mutable state, and the slots the arena assigned —
/// **never `&self`**, so rendering a node cannot mutate its configuration and one
/// prepared node can serve several states without copying.
pub type Kernel = fn(&PreparedNode, &mut NodeState, &mut NodeIo<'_>);

/// A node's immutable prepared data.
///
/// Prepared once, off the audio thread, and shared by every state that runs it. What is
/// *derived* from the stream — a sine's per-frame phase step, a filter's coefficients —
/// is computed here rather than per quantum, which is the whole point of the split.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreparedNode {
    /// Zeros.
    Silence,
    /// One level on every sample.
    Constant {
        /// The level.
        level: Amplitude,
    },
    /// One sample of `1.0` at a plan position.
    Impulse {
        /// Where in the plan the click is.
        position: PlanPosition,
    },
    /// A sine from a phase accumulator.
    Sine {
        /// One frame as a fraction of a second, so a frequency becomes a phase step
        /// by one multiply instead of a divide per quantum.
        seconds_per_frame: f64,
        /// The frequency the state starts at.
        frequency: Frequency,
        /// The peak amplitude the state starts at.
        amplitude: Amplitude,
    },
    /// A constant factor applied to one input.
    Gain {
        /// The factor.
        factor: GainFactor,
    },
    /// A four-segment envelope, as the frames each of its segments lasts.
    ///
    /// The authored durations are gone: what a quantum needs is a **frame count**, which
    /// is a function of the duration and the rate. Frames rather than a per-sample
    /// increment, because an increment is not enough to end a segment on the frame it was
    /// asked to end on — accumulating a rounded `f32` step reaches its threshold tens of
    /// samples early or late over a one-second attack, and the error grows with the
    /// duration. A counter ends it exactly, and the level is derived from the counter
    /// rather than accumulated, so it lands on its target rather than near it.
    ///
    /// It is also what makes a *starting level* free: a note retriggered while it is
    /// still releasing ramps from where it is, over its authored attack, with no click
    /// and no shortened segment.
    Envelope {
        /// How many frames an attack lasts.
        ///
        /// Its own type rather than [`crate::time::FrameCount`]: that one is a position
        /// or a span on the stream's timeline, in `u64`, and a segment length is
        /// neither.
        attack_frames: SegmentFrames,
        /// How many frames a decay lasts.
        decay_frames: SegmentFrames,
        /// How many frames a release lasts.
        release_frames: SegmentFrames,
        /// The level a held gate settles at.
        ///
        /// Still the validated type: it is the one authored value that survives
        /// preparation unchanged, and a raw `f32` here would let a prepared record carry
        /// a sustain the IR would have refused.
        sustain: NormalizedLevel,
    },
    /// A two-pole low-pass, as the four coefficients its integrators read.
    ///
    /// The corner frequency and the quality factor are **gone** by this point: they were
    /// the authored values, and what a quantum needs is the arithmetic they imply. That
    /// is what "prepared" means, and computing it here rather than per quantum is the
    /// whole reason the split exists.
    ///
    /// The form is the topology-preserving state-variable filter, not a direct-form
    /// biquad, and that is a numerical decision rather than a taste one: a direct form
    /// stores a coefficient that approaches `1` as the corner frequency falls, so in
    /// `f32` a low-pass below roughly a thousandth of the sample rate quantizes into
    /// something that is no longer the filter that was asked for — measured, not
    /// assumed. This form's coefficients stay well scaled there, and it is also the form
    /// `synth_dsp` already has, which is where P02-T006 puts the shared kernel.
    Filter {
        /// The three derived integrator coefficients.
        ///
        /// The damping the quality factor implies is *inside* them; a low-pass output
        /// never reads it separately, and carrying it as well would be prepared data no
        /// kernel touches.
        integrator: [f32; 3],
    },
    /// One audio input scaled by one control input. It carries nothing of its own.
    Amplifier,
    /// One buffer copied into another.
    ///
    /// The compiler's own operation rather than an authored node: ADR-0002 clause 7
    /// makes a mono-to-stereo widening a scheduled operation with an identity, and this
    /// is the kernel that performs it.
    Copy,
}

/// A node's mutable state.
///
/// One record per node instance, owned by the renderer and never by the plan. A plan can
/// therefore be rendered by two streams at once, and Phase 6's voice pool gets many
/// states over one prepared node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeState {
    /// A node that keeps nothing between quanta.
    Stateless,
    /// An envelope's segment, the ramp it is on, and its gate.
    Envelope {
        /// Which segment it is in.
        segment: Segment,
        /// The level it is at, in `[0, 1]`.
        level: f32,
        /// The level the current segment is heading for.
        target: f32,
        /// How far the level moves per remaining frame, signed.
        ///
        /// Derived where the segment **starts**, from the level that is actually there:
        /// a note let go during its attack releases from where it had reached, over its
        /// authored release time. A step prepared from the sustain level would make that
        /// duration wrong for every short note, and with a sustain of zero it would be a
        /// step of zero, which never arrives.
        step: f32,
        /// Frames left in the current segment.
        remaining: SegmentFrames,
        /// Whether the gate is currently held.
        ///
        /// Kept so that Attack begins on a low-to-high **transition** rather than on any
        /// positive value: automation that emits the same held gate every quantum would
        /// otherwise restart the note continuously and never let it settle.
        held: bool,
    },
    /// The two integrator states of a state-variable filter.
    Filter {
        /// The band-pass integrator.
        band: f32,
        /// The low-pass integrator.
        low: f32,
    },
    /// A phase accumulator and the control values read once per quantum.
    Sine {
        /// Normalized phase in `[0, 1)`.
        phase: f64,
        /// The current frequency.
        frequency: Frequency,
        /// The current peak amplitude.
        amplitude: Amplitude,
    },
}

/// Which segment of an envelope is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    /// The gate is low and the level is zero.
    Idle,
    /// Rising towards one.
    Attack,
    /// Falling towards the sustain level.
    Decay,
    /// Held at the sustain level.
    Sustain,
    /// Falling towards zero.
    Release,
}

impl NodeState {
    /// The state a prepared node starts in.
    #[must_use]
    pub const fn initial(prepared: &PreparedNode) -> Self {
        match prepared {
            PreparedNode::Sine {
                frequency,
                amplitude,
                ..
            } => Self::Sine {
                phase: 0.0,
                frequency: *frequency,
                amplitude: *amplitude,
            },
            PreparedNode::Filter { .. } => Self::Filter {
                band: 0.0,
                low: 0.0,
            },
            PreparedNode::Envelope { .. } => Self::Envelope {
                segment: Segment::Idle,
                level: 0.0,
                target: 0.0,
                step: 0.0,
                remaining: SegmentFrames::NONE,
                held: false,
            },
            PreparedNode::Silence
            | PreparedNode::Amplifier
            | PreparedNode::Constant { .. }
            | PreparedNode::Impulse { .. }
            | PreparedNode::Gain { .. }
            | PreparedNode::Copy => Self::Stateless,
        }
    }

    /// Move one of this node's controls.
    ///
    /// The index is the node kind's own, resolved at admission from the parameter
    /// identity a caller addressed. An index this state does not have does nothing: the
    /// pairing is the compiler's, and the audio thread cannot report a defect in it.
    pub fn set_control(
        &mut self,
        prepared: &PreparedNode,
        control: ControlIndex,
        value: ParameterValue,
    ) {
        match self {
            Self::Sine {
                frequency,
                amplitude,
                ..
            } => match control {
                SINE_FREQUENCY => *frequency = value.into_frequency(),
                SINE_AMPLITUDE => *amplitude = value.into_amplitude(),
                _ => {}
            },
            // A gate is a control like any other in this phase: it is observed once per
            // quantum. P02-T007 is where a note edge lands at its declared sample.
            Self::Envelope {
                segment,
                level,
                target,
                step,
                remaining,
                held,
            } => {
                let PreparedNode::Envelope {
                    attack_frames,
                    release_frames,
                    ..
                } = prepared
                else {
                    return;
                };
                if !matches!(control, ENVELOPE_GATE) {
                    return;
                }
                let raised = value.as_f32() > 0.0;
                if raised == *held {
                    // Not an edge. A held gate re-asserted is the same note, and
                    // restarting its attack would be a retrigger nobody asked for.
                    return;
                }
                *held = raised;
                let (destination, frames) = if raised {
                    (1.0, *attack_frames)
                } else {
                    (0.0, *release_frames)
                };
                *segment = match (raised, *level > 0.0) {
                    (true, _) => Segment::Attack,
                    (false, true) => Segment::Release,
                    (false, false) => Segment::Idle,
                };
                *target = destination;
                (*step, *remaining) = ramp(*level, destination, frames);
            }
            Self::Filter { .. } | Self::Stateless => {}
        }
    }
}

/// Which control of a node kind a parameter event moves.
///
/// A node-local index, not an identity: the compiler resolves `(node, parameter)` to a
/// slot and this index once, and the render loop carries neither name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct ControlIndex(u8);

impl ControlIndex {
    /// A control index.
    pub const fn new(index: u8) -> Self {
        Self(index)
    }

    /// The raw index.
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

/// An envelope's gate control.
pub const ENVELOPE_GATE: ControlIndex = ControlIndex::new(0);
/// A sine's frequency control.
pub const SINE_FREQUENCY: ControlIndex = ControlIndex::new(0);
/// A sine's amplitude control.
pub const SINE_AMPLITUDE: ControlIndex = ControlIndex::new(1);

/// What one of a kernel's inputs turned out to be.
///
/// Three states, and they mean three different things — collapsing any two of them is a
/// silent behaviour change. An unpatched input is silence the node must *produce*; an
/// in-place input is a buffer the node must read **from its own output**, which is
/// ADR-0005 clause 5's merge; a patched input is an ordinary distinct region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputBuffer<'a> {
    /// Nothing is patched here.
    Unpatched,
    /// The arena gave this input the output's own slot.
    InPlace,
    /// A distinct region of the arena.
    Patched(&'a [f32]),
}

/// The buffers one kernel call may touch, and where the quantum sits in the plan.
///
/// A borrow of the arena, resolved by [`bind`] from the slots the compiler assigned: one
/// mutable output and up to [`MAX_INPUTS`] inputs, each of them a distinct region or one
/// of the two states above.
///
/// The fields are public because this **is** the kernel interface: a kernel is a free
/// function over these three things, and a caller that writes one — or measures one, as
/// the ADR-0004 evidence harness does — needs to build the same borrow bundle the
/// renderer builds.
#[derive(Debug)]
pub struct NodeIo<'a> {
    /// The buffer this node writes, `Q` frames.
    pub out: &'a mut [f32],
    /// The buffers it reads, in port order.
    pub inputs: [InputBuffer<'a>; MAX_INPUTS],
    /// The plan position of this quantum's first frame, where the anchor reaches it.
    pub position: Option<PlanPosition>,
}

/// Borrow the arena regions one step names.
///
/// The one place a kernel's slots become slices. It hands out **one** mutable region and
/// up to [`MAX_INPUTS`] shared ones, each a different `Q`-frame chunk of one flat
/// allocation, by walking the chunks in ascending order and splitting each off in turn —
/// so no two borrows can overlap and none of it needs `unsafe`. An input naming the
/// output's chunk is reported as `None` rather than borrowed twice.
pub fn bind<'a>(
    buffers: &'a mut [f32],
    quantum: usize,
    step: &NodeStep,
    position: Option<PlanPosition>,
) -> Option<NodeIo<'a>> {
    let mut out: Option<&'a mut [f32]> = None;
    let mut inputs = [InputBuffer::Unpatched; MAX_INPUTS];
    let mut rest = buffers;
    let mut consumed = 0_usize;

    // The roles in ascending slot order, worked out at admission. Walking them forwards
    // and splitting each region off in turn is what lets one mutable and up to two shared
    // borrows of one allocation coexist without `unsafe` — and there is nothing to decide
    // here, because the compiler already decided it.
    for role in *step.order() {
        let slot = match role {
            0 => step.out(),
            _ => match step.inputs().get(role as usize - 1).copied().flatten() {
                Some(slot) => slot,
                None => break,
            },
        };
        let skip = slot.index().checked_mul(quantum)?.checked_sub(consumed)?;
        // `rest` is moved out and put back, which is what lets each piece keep the
        // arena's own lifetime instead of a reborrow that ends with the loop body.
        let taken = rest;
        let (_, tail) = taken.split_at_mut_checked(skip)?;
        let (piece, remainder) = tail.split_at_mut_checked(quantum)?;
        rest = remainder;
        consumed = consumed.checked_add(skip)?.checked_add(quantum)?;
        match role {
            0 => out = Some(piece),
            _ => {
                if let Some(entry) = inputs.get_mut(role as usize - 1) {
                    *entry = InputBuffer::Patched(piece);
                }
            }
        }
    }

    // The inputs that borrow nothing of their own: unpatched, the output's own slot, or
    // a region an earlier input already holds.
    for (index, binding) in step.bindings().iter().enumerate() {
        let resolved = match binding {
            InputBinding::Distinct => continue,
            InputBinding::Unpatched => InputBuffer::Unpatched,
            InputBinding::InPlace => InputBuffer::InPlace,
            InputBinding::Mirrors(earlier) => match inputs.get(*earlier as usize).copied() {
                Some(source) => source,
                None => InputBuffer::Unpatched,
            },
        };
        if let Some(entry) = inputs.get_mut(index) {
            *entry = resolved;
        }
    }

    Some(NodeIo {
        out: out?,
        inputs,
        position,
    })
}

/// Zeros.
pub fn silence(_prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
    io.out.fill(0.0);
}

/// One level on every sample.
pub fn constant(prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Constant { level } = prepared else {
        return;
    };
    io.out.fill(level.as_f32());
}

/// One sample of `1.0` where the plan position falls inside this quantum.
pub fn impulse(prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Impulse { position } = prepared else {
        return;
    };
    io.out.fill(0.0);
    let Some(start) = io.position else {
        return;
    };
    let Some(offset) = position.as_u64().checked_sub(start.as_u64()) else {
        return;
    };
    let Ok(offset) = usize::try_from(offset) else {
        return;
    };
    if let Some(sample) = io.out.get_mut(offset) {
        *sample = 1.0;
    }
}

/// A sine, from a phase accumulator.
pub fn sine(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Sine {
        seconds_per_frame, ..
    } = prepared
    else {
        return;
    };
    let NodeState::Sine {
        phase,
        frequency,
        amplitude,
    } = state
    else {
        return;
    };
    // Control values are read **once**, here: a parameter event inside a quantum takes
    // effect at the next boundary, which is ADR-0001 clause 13's causality made concrete.
    let increment = f64::from(frequency.as_f32()) * seconds_per_frame;
    let peak = f64::from(amplitude.as_f32());
    let mut running = *phase;
    for sample in io.out.iter_mut() {
        *sample = (peak * (std::f64::consts::TAU * running).sin()) as f32;
        running += increment;
        // Both directions. A negative frequency is legal and means the phase runs
        // backwards, so wrapping only at 1.0 would let it fall below zero and grow
        // without bound — feeding `sin` ever-larger arguments, which loses precision to
        // range reduction instead of staying periodic.
        if !(0.0..1.0).contains(&running) {
            running -= running.floor();
        }
    }
    *phase = running;
}

/// One input scaled by a constant factor.
pub fn gain(prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Gain { factor } = prepared else {
        return;
    };
    let factor = factor.as_f32();
    let source = io.inputs[0];
    match source {
        InputBuffer::Patched(source) => {
            for (sample, input) in io.out.iter_mut().zip(source.iter()) {
                *sample = *input * factor;
            }
        }
        // In place, which is ADR-0005 clause 5: the arena gave this node its input's
        // slot because nothing reads that value again.
        InputBuffer::InPlace => {
            for sample in io.out.iter_mut() {
                *sample *= factor;
            }
        }
        // An unpatched input is legal and quiet — validation warns about an unreached
        // *output*, not an unpatched input. Quiet has to be **made** true rather than
        // assumed: the buffer holds whatever an earlier value left in this arena slot.
        InputBuffer::Unpatched => io.out.fill(0.0),
    }
}

/// The signed per-frame step and the frame count of one segment.
///
/// `level = target + remaining * step` holds at every frame, so the level starts exactly
/// where the previous segment left it and arrives exactly on its target — neither of
/// which an accumulated increment can promise.
const fn ramp(from: f32, to: f32, frames: SegmentFrames) -> (f32, SegmentFrames) {
    if frames.is_finished() {
        (0.0, SegmentFrames::NONE)
    } else {
        ((from - to) / frames.get() as f32, frames)
    }
}

/// A four-segment envelope, one value per sample.
pub fn envelope(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Envelope {
        decay_frames,
        sustain,
        ..
    } = prepared
    else {
        return;
    };
    let NodeState::Envelope {
        segment,
        level,
        target,
        step,
        remaining,
        ..
    } = state
    else {
        return;
    };
    let mut run = Run {
        stage: *segment,
        level: *level,
        target: *target,
        step: *step,
        remaining: *remaining,
    };

    let sustain = sustain.as_f32();
    for sample in io.out.iter_mut() {
        run.hand_over(sustain, *decay_frames);
        *sample = match run.stage {
            Segment::Idle => 0.0,
            Segment::Sustain => sustain,
            Segment::Attack | Segment::Decay | Segment::Release => {
                // The start of the segment on its first frame and one step short of its
                // target on its last: the target itself belongs to the segment that
                // follows, which is what makes a chain of segments continuous and each
                // of them exactly as long as it was authored.
                let value = run.target + run.remaining.get() as f32 * run.step;
                run.remaining = run.remaining.spent();
                value
            }
        };
        run.level = *sample;
    }
    // Settled before it is stored, so a quantum that ends exactly on a segment boundary
    // leaves the state on the segment that follows rather than on the exhausted one.
    run.hand_over(sustain, *decay_frames);
    // And the level stored is the one the **next** sample will have, not the last one
    // written: the counter has already moved past it. A gate edge arriving at a quantum
    // boundary starts its ramp from this value, and starting from the previous sample
    // instead would put a step in the signal that nothing in the plan asked for.
    run.level = run.boundary_level(sustain);

    (*segment, *level, *target, *step, *remaining) =
        (run.stage, run.level, run.target, run.step, run.remaining);
}

/// An envelope's ramp, while a quantum is being written.
struct Run {
    stage: Segment,
    level: f32,
    target: f32,
    step: f32,
    remaining: SegmentFrames,
}

impl Run {
    /// The level the next sample of this segment will have.
    fn boundary_level(&self, sustain: f32) -> f32 {
        match self.stage {
            Segment::Idle => 0.0,
            Segment::Sustain => sustain,
            Segment::Attack | Segment::Decay | Segment::Release => {
                self.target + self.remaining.get() as f32 * self.step
            }
        }
    }

    /// Move past every segment that has no frames left.
    ///
    /// Two at most — an instant attack into an instant decay — and the bound is what
    /// keeps this a loop the audio thread can afford.
    fn hand_over(&mut self, sustain: f32, decay_frames: SegmentFrames) {
        for _ in 0..2 {
            if !self.remaining.is_finished() {
                break;
            }
            match self.stage {
                Segment::Attack => {
                    self.level = 1.0;
                    self.stage = Segment::Decay;
                    self.target = sustain;
                    (self.step, self.remaining) = ramp(1.0, sustain, decay_frames);
                }
                // Neither of these sets the level: the segment they hand over to writes
                // it unconditionally, and assigning it here as well would be two
                // authorities on one value.
                Segment::Decay => self.stage = Segment::Sustain,
                Segment::Release => self.stage = Segment::Idle,
                Segment::Idle | Segment::Sustain => break,
            }
        }
    }
}

/// A two-pole low-pass, as a topology-preserving state-variable filter.
pub fn filter(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Filter { integrator } = prepared else {
        return;
    };
    let NodeState::Filter { band, low } = state else {
        return;
    };
    let source = io.inputs[0];
    let (mut first, mut second) = (*band, *low);
    for (index, sample) in io.out.iter_mut().enumerate() {
        // The three input states again, and the filter is where collapsing them would be
        // least visible: an unpatched filter must ring down from its own state rather
        // than through whatever the arena slot contained.
        let input = match source {
            InputBuffer::Patched(source) => source.get(index).copied().unwrap_or(0.0),
            InputBuffer::InPlace => *sample,
            InputBuffer::Unpatched => 0.0,
        };
        let drive = input - second;
        let band_pass = integrator[0] * first + integrator[1] * drive;
        let low_pass = second + integrator[1] * first + integrator[2] * drive;
        first = 2.0 * band_pass - first;
        second = 2.0 * low_pass - second;
        *sample = low_pass;
    }
    // Flushed once per quantum, not per sample. A filter's history decays through the
    // subnormal range after its input stops, and there it *stays*: the products that
    // would carry it further underflow to zero, so the state keeps a value around
    // `1e-45` indefinitely and every later silent sample performs subnormal arithmetic —
    // which stalls on processors without flush-to-zero, exactly the cost the preparation
    // check refuses subnormal coefficients to avoid. The threshold is roughly -600 dB,
    // far below anything a signal path carries.
    (*band, *low) = (flush(first), flush(second));
}

/// Zero, where a value is too small to be signal.
const fn flush(value: f32) -> f32 {
    if value.abs() < SUBNORMAL_GUARD {
        0.0
    } else {
        value
    }
}

/// The level below which a filter's history is treated as silence.
const SUBNORMAL_GUARD: f32 = 1e-30;

/// One audio input scaled, sample by sample, by one control input.
pub fn amplifier(_prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
    let [audio, control] = io.inputs;
    let InputBuffer::Patched(control) = control else {
        // A control input is never the in-place one — the arena merges the first input
        // only — so this is the unpatched case, and an amplifier with nothing driving it
        // is silent.
        io.out.fill(0.0);
        return;
    };
    match audio {
        InputBuffer::Patched(audio) => {
            for ((sample, input), level) in io.out.iter_mut().zip(audio.iter()).zip(control.iter())
            {
                *sample = *input * *level;
            }
        }
        InputBuffer::InPlace => {
            for (sample, level) in io.out.iter_mut().zip(control.iter()) {
                *sample *= *level;
            }
        }
        InputBuffer::Unpatched => io.out.fill(0.0),
    }
}

/// One buffer copied into another.
pub fn copy(_prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
    let source = io.inputs[0];
    let InputBuffer::Patched(source) = source else {
        // Neither other state can be produced for a copy: the compiler inserts it with a
        // source, and it is not in-place safe.
        io.out.fill(0.0);
        return;
    };
    for (sample, input) in io.out.iter_mut().zip(source.iter()) {
        *sample = *input;
    }
}
