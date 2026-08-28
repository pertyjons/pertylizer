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

use crate::plan::{BufferRegion, InputBinding, NodeStep};
use crate::quantities::{
    Amplitude, ChannelLayout, Frequency, GainFactor, NormalizedLevel, ParameterValue, SegmentFrames,
};
use crate::time::{PlanPosition, QuantumOffset};

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
type KernelFn = fn(&PreparedNode, &mut NodeState, &mut NodeIo<'_>);

/// A node's render entry, and the only thing a descriptor can hold.
///
/// # Why this is a newtype and not the function pointer
///
/// `SOUND-INV-013` says every kernel reachable from the render loop lives in this crate,
/// and audio is not routed through a function whose behaviour V1's corpus digests pin.
/// While this was a bare `fn` alias, that invariant had a hole its own conformance row
/// admitted: `render_loop_purity` could check that every *registered* kernel is defined
/// in the checked region, but not that a descriptor's pointer resolves inside it. A
/// descriptor written against any other path was simply invisible to a scan keyed on the
/// path it expected.
///
/// The wrapper closes the **cross-module** half of that by construction, and only that
/// half. Its field is private to this module, so a `Kernel` can only be built **here**:
/// **a descriptor elsewhere naming any function is not caught by a test; it does not
/// compile.**
///
/// The type system says nothing about what is built here. An in-module
/// `Kernel(foreign)` is well typed, and what rejects it is `render_loop_purity`'s scan of
/// this file's construction sites — which recognises source forms and is bounded as
/// such. The specification's *Unresolved questions* records that boundary.
///
/// The kernel functions stay public beside the constants, because EVD-0009's and
/// EVD-0010's harnesses call them directly and a comparison that reimplemented them would
/// be measuring a model. The function is the arithmetic; the constant is the registrable
/// form.
/// No derived `PartialEq`: comparing two kernels is `is_same`'s job — it is crate-internal
/// and records what function-pointer equality can promise — and having two ways to do it
/// invites the wrong one.
#[derive(Debug, Clone, Copy)]
#[must_use]
pub struct Kernel(KernelFn);

impl Kernel {
    /// Render one node's quantum.
    ///
    /// Real-time: this is a call through a function pointer and nothing else. The
    /// wrapper is a newtype over that pointer, so it costs what the bare call cost.
    ///
    /// **Public, and that does not weaken the provenance guarantee.** The guarantee is
    /// about *construction*: nothing outside this module can make a `Kernel`. Invoking
    /// one obtained from a compiled plan is what EVD-0009's and EVD-0010's harnesses do,
    /// and it is the surface P02-T005's deviation 5 already records — a harness that
    /// reimplemented the dispatch would be measuring a model of it.
    #[inline]
    pub fn run(self, prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
        (self.0)(prepared, state, io);
    }

    /// Whether two kernels are the same function.
    ///
    /// Function-pointer equality is what it is: two identical functions may be merged to
    /// one address, and one function may have two addresses across codegen units. It is
    /// used to compare *schedules*, where both directions are acceptable — a schedule
    /// that differs in its slots is what a test is really asking about.
    ///
    /// It lives here rather than at the call site because the pointer is private to this
    /// module, which is the point of the wrapper.
    pub(crate) fn is_same(self, other: Self) -> bool {
        std::ptr::fn_addr_eq(self.0, other.0)
    }
}

/// The registrable form of each kernel.
///
/// One per function with the kernel signature. `render_loop_purity` checks that the two
/// sets agree, so a kernel with no constant or a constant with no kernel fails — a scan
/// over this file's source, with the reach that implies.
pub const SILENCE: Kernel = Kernel(silence);
/// See [`SILENCE`].
pub const CONSTANT: Kernel = Kernel(constant);
/// See [`SILENCE`].
pub const IMPULSE: Kernel = Kernel(impulse);
/// See [`SILENCE`].
pub const SINE: Kernel = Kernel(sine);
/// See [`SILENCE`].
pub const GAIN: Kernel = Kernel(gain);
/// See [`SILENCE`].
pub const ENVELOPE: Kernel = Kernel(envelope);
/// See [`SILENCE`].
pub const FILTER: Kernel = Kernel(filter);
/// See [`SILENCE`].
pub const AMPLIFIER: Kernel = Kernel(amplifier);
/// See [`SILENCE`].
pub const COPY: Kernel = Kernel(copy);

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
    /// assumed. This form's coefficients stay well scaled there.
    ///
    /// It is also the form `synth_dsp` already has, which is a **coincidence of two
    /// independent choices** rather than sharing: ADR-0040 gives V2 its own DSP, and
    /// P02-T006's extraction is closed as not happening. The two engines run the same
    /// recurrence because it is the right one, and EVD-0013 measured their magnitude
    /// responses agreeing to 0.068 dB across six octave bands. Nothing is shared, and a
    /// fix to one does not reach the other.
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

    /// What a control holds, for a caller that must restore it.
    ///
    /// The symmetric reader of [`Self::set_control`], and it exists for ADR-0051's catch-up
    /// batch: a locate restores **every** prepared target, and a target with no write before
    /// the destination is restored to the value it was prepared with. A batch that skipped
    /// those would leave whatever the pre-seek position set, which is exactly the value the
    /// seek was supposed to leave behind.
    ///
    /// **An envelope's gate answers here even though [`Self::set_control`] ignores it**, and
    /// the asymmetry is the point rather than an oversight. A gate is sample-positioned, so
    /// the edge law belongs to the kernel and nothing quantum-rate may move it — but a gate
    /// can be *raised* by automation rather than by a note, and such a gate has no live-note
    /// entry for the boundary mass release to find. If the batch skipped it, seeking back
    /// past the automation that raised it would leave it high with nothing able to lower it.
    ///
    /// Off the audio thread. `None` only where the kind has no such control.
    ///
    /// Crate-private: the one caller is `StreamControl::catch_up`, and a restoration hook this
    /// specific is not a commitment worth making to a downstream reader.
    #[must_use]
    pub(crate) fn control_value(&self, control: ControlIndex) -> Option<ParameterValue> {
        match self {
            Self::Sine {
                frequency,
                amplitude,
                ..
            } => match control {
                SINE_FREQUENCY => ParameterValue::new(frequency.as_f32()).ok(),
                SINE_AMPLITUDE => ParameterValue::new(amplitude.as_f32()).ok(),
                _ => None,
            },
            Self::Envelope { held, .. } => match control {
                ENVELOPE_GATE => Some(if *held {
                    ParameterValue::ONE
                } else {
                    ParameterValue::ZERO
                }),
                _ => None,
            },
            Self::Filter { .. } | Self::Stateless => None,
        }
    }

    /// Move one of this node's **quantum-rate** controls.
    ///
    /// The index is the node kind's own, resolved at admission from the parameter
    /// identity a caller addressed. An index this state does not have does nothing: the
    /// pairing is the compiler's, and the audio thread cannot report a defect in it.
    ///
    /// Sample-positioned controls do not come through here. ADR-0001 clause 14 puts them
    /// at the offset their render position names, which is inside a kernel's own loop, so
    /// the renderer hands them to the kernel as [`NodeIo::controls`] instead.
    pub fn set_control(&mut self, control: ControlIndex, value: ParameterValue) {
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
            // An envelope has no quantum-rate control. Its gate is sample-positioned
            // (ADR-0001 clause 14), so the edge law lives in the kernel — the one place
            // that knows which sample it is — and reaches it through [`NodeIo::controls`]
            // rather than through here. Two authorities on one edge would be one too many.
            Self::Envelope { .. } | Self::Filter { .. } | Self::Stateless => {}
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
    /// The buffer this node writes: `Q` frames of [`Self::channels`], interleaved.
    pub out: &'a mut [f32],
    /// How many channels the output holds, frame-major.
    ///
    /// ADR-0041 clause 4. A kernel is told its channel count and must be correct for
    /// every count its own ports admit; a mono-only kernel is told `Mono` and its port
    /// table says why that is the only value it can see.
    pub channels: ChannelLayout,
    /// The buffers it reads, in port order.
    pub inputs: [InputBuffer<'a>; MAX_INPUTS],
    /// The plan position of this quantum's first frame, where the anchor reaches it.
    pub position: Option<PlanPosition>,
    /// This node's sample-positioned control changes, due inside this quantum.
    ///
    /// ADR-0001 clause 14, as ADR-0043 restated it: a note-on, note-off, gate or
    /// retrigger occurs at the offset its render position names rather than at the
    /// boundary that follows it, and a kernel is the only place that knows where its
    /// samples are. The renderer resolves that position before it builds this slice, so a
    /// kernel never sees the distinction between a stamp and a clamped position. Ascending by offset, and empty for every quantum
    /// and every node kind that has none — which is all of them but the envelope today.
    ///
    /// The renderer resolves these once per quantum, so this is not a control-rate
    /// evaluation happening more than once (ADR-0001 clause 4) and it is not the
    /// event-boundary quantum split clause 15 reserves for Phase 3: the schedule is still
    /// walked exactly once, and only the node the edge names sees it.
    pub controls: &'a [TimedControl],
}

/// One control change at a resolved offset inside the quantum.
///
/// The sample-positioned twin of [`NodeState::set_control`]: the same node-local control
/// index and the same value, plus the offset the change happens at. The renderer builds
/// these from the events a quantum is due; a kernel applies them as it reaches each frame.
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct TimedControl {
    /// Where inside the quantum it happens.
    pub offset: QuantumOffset,
    /// Which of the node's controls it moves.
    pub control: ControlIndex,
    /// The value it moves it to.
    pub value: ParameterValue,
}

impl TimedControl {
    /// The value the scratch is filled with at preparation.
    ///
    /// Never read: a node's slice is bounded by the index table, and the fill exists so
    /// the buffer can be allocated to its full length once. That is what makes growth
    /// *impossible* in the loop rather than merely unlikely.
    pub(crate) const FILL: Self = Self {
        offset: QuantumOffset::ZERO,
        control: ControlIndex::new(u8::MAX),
        value: ParameterValue::ZERO,
    };
}

/// Borrow the arena regions one step names.
///
/// The one place a kernel's slots become slices. It hands out **one** mutable region and
/// up to [`MAX_INPUTS`] shared ones, each a different chunk of one flat allocation, by
/// walking the chunks in ascending offset order and splitting each off in turn — so no
/// two borrows can overlap and none of it needs `unsafe`. An input naming the output's
/// chunk is reported as `None` rather than borrowed twice.
///
/// `regions` is the plan's table: since
/// [ADR-0041](../../../plans/v2/decisions/ADR-0041-interleaved-internal-channel-layout.md)
/// clause 2 a slot's place in the arena is an offset and a length the plan **records**,
/// because a signal occupies `c * Q` samples and multiplying a slot index by the quantum
/// no longer describes anything.
pub fn bind<'a>(
    buffers: &'a mut [f32],
    regions: &[BufferRegion],
    step: &NodeStep,
    position: Option<PlanPosition>,
    controls: &'a [TimedControl],
) -> Option<NodeIo<'a>> {
    let mut out: Option<&'a mut [f32]> = None;
    let mut inputs = [InputBuffer::Unpatched; MAX_INPUTS];
    let mut rest = buffers;
    let mut consumed = 0_usize;

    // The roles in ascending offset order, worked out at admission. Walking them forwards
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
        let region = regions.get(slot.index()).copied()?;
        let skip = region.offset().checked_sub(consumed)?;
        // `rest` is moved out and put back, which is what lets each piece keep the
        // arena's own lifetime instead of a reborrow that ends with the loop body.
        let taken = rest;
        let (_, tail) = taken.split_at_mut_checked(skip)?;
        let (piece, remainder) = tail.split_at_mut_checked(region.length())?;
        rest = remainder;
        consumed = consumed.checked_add(skip)?.checked_add(region.length())?;
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
        channels: step.out_layout(),
        inputs,
        position,
        controls,
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
///
/// Its gate is the phase's one sample-positioned control (ADR-0001 clause 14), so this is
/// where a note edge takes effect: an edge at offset `k` is applied before frame `k` is
/// written and after frame `k - 1` was, which is what makes the note start on the sample
/// it named rather than on the quantum boundary that follows it.
pub fn envelope(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Envelope {
        attack_frames,
        decay_frames,
        release_frames,
        sustain,
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
        held,
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
        held: *held,
    };

    let sustain = sustain.as_f32();
    // The envelope's own port table admits one channel, so a frame is a sample and an
    // offset indexes `out` directly. Deriving the frame from the channel count anyway
    // would be arithmetic defending against a layout this kind cannot be given.
    let mut due = 0_usize;
    for (frame, sample) in io.out.iter_mut().enumerate() {
        // Before `hand_over` and before the write, which is exactly where the boundary
        // path put a gate when it was an ordinary control: the edge is applied to the
        // level the frame was going to start from. `while` rather than `if` because two
        // edges may share a quantum — a note released and retriggered inside 1.33 ms —
        // and each of them is a separate edge at its own sample.
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, ENVELOPE_GATE) {
                run.gate(control.value, sustain, *attack_frames, *release_frames);
            }
        }
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

    (*segment, *level, *target, *step, *remaining, *held) = (
        run.stage,
        run.level,
        run.target,
        run.step,
        run.remaining,
        run.held,
    );
}

/// An envelope's ramp, while a quantum is being written.
struct Run {
    stage: Segment,
    level: f32,
    target: f32,
    step: f32,
    remaining: SegmentFrames,
    held: bool,
}

impl Run {
    /// Apply a gate edge at the frame the run has reached.
    ///
    /// The **one** authority on what a gate does. It reads `self.level`, which is the
    /// level the frame about to be written would otherwise have started from, so a note
    /// let go during its attack releases from where it had actually reached rather than
    /// from the sustain level it never got to.
    fn gate(
        &mut self,
        value: ParameterValue,
        sustain: f32,
        attack: SegmentFrames,
        release: SegmentFrames,
    ) {
        let raised = value.as_f32() > 0.0;
        if raised == self.held {
            // Not an edge. A held gate re-asserted is the same note, and restarting its
            // attack would be a retrigger nobody asked for.
            return;
        }
        self.held = raised;
        // The level **this** frame starts from, which after frame 0 is one step further
        // along the ramp than the sample written at the frame before: the counter has
        // already moved past it. Reading `self.level` directly would compute the new
        // segment from a level the signal has left, so a note let go mid-attack would
        // repeat one sample and ramp from the wrong amplitude. At a quantum boundary the
        // two agree, because the epilogue stores exactly this value — which is why the
        // committed layout baselines, whose every edge is on a boundary, cannot see it.
        self.level = self.boundary_level(sustain);
        let (destination, frames) = if raised {
            (1.0, attack)
        } else {
            (0.0, release)
        };
        self.stage = match (raised, self.level > 0.0) {
            (true, _) => Segment::Attack,
            (false, true) => Segment::Release,
            (false, false) => Segment::Idle,
        };
        self.target = destination;
        (self.step, self.remaining) = ramp(self.level, destination, frames);
    }

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
    // ADR-0041 clause 8: the one implicit conversion this phase inserts writes each
    // sample into **both channels of one wider region**, frame-major. At one channel it
    // is the plain copy it was, which is what a mono path renders through.
    let channels = io.channels.channels();
    for (frame, input) in source.iter().enumerate() {
        for channel in 0..channels {
            if let Some(sample) = io.out.get_mut(frame * channels + channel) {
                *sample = *input;
            }
        }
    }
}
