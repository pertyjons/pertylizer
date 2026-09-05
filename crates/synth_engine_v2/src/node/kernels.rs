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
    Amplitude, ChannelLayout, Frequency, GainFactor, NormalizedLevel, NoteVelocity, ParameterValue,
    SegmentFrames,
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

/// The band-limited sawtooth.
pub const SAW: Kernel = Kernel(saw);
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
/// The voice sum's kernel: one instance's output added into the shared mix.
pub const ACCUMULATE: Kernel = Kernel(accumulate);
/// The monitor's kernel: its input, unchanged.
pub const MONITOR: Kernel = Kernel(monitor);

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
    /// A sawtooth from a phase accumulator, band-limited at its discontinuity.
    Saw {
        /// One frame as a fraction of a second, as a sine's is and for the same reason.
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
        /// The scale [`ENVELOPE_VELOCITY`] last set, applied to every emitted sample.
        ///
        /// Starts at [`NoteVelocity::FULL`], so a plan rendered before its first note emits
        /// the envelope it was authored with. That is not a fallback for a missing
        /// magnitude: `SOUND-INV-021` refuses a plan whose note scope declares no velocity
        /// destination, so the only way to observe the initial value is to render before
        /// the first note-on.
        ///
        /// **Typed, not a raw `f32`**, because the domain is `[0, 1]` and the parameter path
        /// can present any finite value. [`NoteVelocity::saturating`] is the documented
        /// policy that owns it; an independent review found this field holding a bare float
        /// with the clamp written at the assignment instead.
        velocity: NoteVelocity,
    },
    /// A voice sum step's fade (ADR-0058): frames of the fade remaining and its whole
    /// length, both zero when no fade is in force.
    Sum {
        /// Frames left before the step's contribution reaches zero.
        fade_remaining: u32,
        /// The fade's length; zero is no fade, and unity gain.
        fade_total: u32,
    },
    /// The two integrator states of a state-variable filter.
    Filter {
        /// The band-pass integrator.
        band: f32,
        /// The low-pass integrator.
        low: f32,
    },
    /// A phase accumulator and the sample-positioned frequency.
    ///
    /// No amplitude: it is a quantum-rate control, and since `SOUND-INV-024` a kernel reads
    /// such a control per frame from its slot's buffer rather than from a state field the
    /// renderer wrote once per quantum — the segment lives in the slot, not here.
    Sine {
        /// Normalized phase in `[0, 1)`.
        phase: f64,
        /// The current frequency.
        frequency: Frequency,
    },
    /// The same two values a sine keeps, for the sawtooth.
    ///
    /// A separate variant rather than a shared one, so that no control write can reach the
    /// wrong kernel's state through a shape the type system would have accepted.
    Saw {
        /// Normalized phase in `[0, 1)`.
        phase: f64,
        /// The current frequency.
        frequency: Frequency,
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
            PreparedNode::Sine { frequency, .. } => Self::Sine {
                phase: 0.0,
                frequency: *frequency,
            },
            PreparedNode::Saw { frequency, .. } => Self::Saw {
                phase: 0.0,
                frequency: *frequency,
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
                velocity: NoteVelocity::FULL,
            },
            PreparedNode::Copy => Self::Sum {
                fade_remaining: 0,
                fade_total: 0,
            },
            PreparedNode::Silence
            | PreparedNode::Amplifier
            | PreparedNode::Constant { .. }
            | PreparedNode::Impulse { .. }
            | PreparedNode::Gain { .. } => Self::Stateless,
        }
    }

    /// What this state holds for one of its sample-positioned controls, for a test that
    /// reads the kernel's own record of the last write it applied.
    ///
    /// `None` where the kind keeps no such control. Test-only since `P05-S007b`: the
    /// stored base a slot starts from is [`authored_value`]'s, and no production path reads
    /// state back.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn control_value(&self, control: ControlIndex) -> Option<ParameterValue> {
        match self {
            Self::Sine { frequency, .. } => match control {
                SINE_FREQUENCY => ParameterValue::new(frequency.as_f32()).ok(),
                _ => None,
            },
            Self::Saw { frequency, .. } => match control {
                SAW_FREQUENCY => ParameterValue::new(frequency.as_f32()).ok(),
                _ => None,
            },
            Self::Envelope { held, velocity, .. } => match control {
                ENVELOPE_GATE => Some(if *held {
                    ParameterValue::ONE
                } else {
                    ParameterValue::ZERO
                }),
                ENVELOPE_VELOCITY => ParameterValue::new(velocity.as_f32()).ok(),
                _ => None,
            },
            Self::Filter { .. } | Self::Sum { .. } | Self::Stateless => None,
        }
    }
}

/// The value a prepared record carries for one of its controls, where it carries one.
///
/// `SOUND-INV-023`'s **stored base**: what the node was prepared with, which is the
/// authored value. Read once, at admission, to seed the parameter slot; the compiler takes
/// the declaration's resting value where the record carries none, which is the envelope's
/// gate and velocity. Off the audio thread.
#[must_use]
pub(crate) fn authored_value(
    prepared: &PreparedNode,
    control: ControlIndex,
) -> Option<ParameterValue> {
    match prepared {
        PreparedNode::Sine {
            frequency,
            amplitude,
            ..
        } => match control {
            SINE_FREQUENCY => Some(ParameterValue::from_frequency(*frequency)),
            SINE_AMPLITUDE => Some(ParameterValue::from_amplitude(*amplitude)),
            _ => None,
        },
        PreparedNode::Saw {
            frequency,
            amplitude,
            ..
        } => match control {
            SAW_FREQUENCY => Some(ParameterValue::from_frequency(*frequency)),
            SAW_AMPLITUDE => Some(ParameterValue::from_amplitude(*amplitude)),
            _ => None,
        },
        PreparedNode::Silence
        | PreparedNode::Constant { .. }
        | PreparedNode::Impulse { .. }
        | PreparedNode::Gain { .. }
        | PreparedNode::Envelope { .. }
        | PreparedNode::Filter { .. }
        | PreparedNode::Amplifier
        | PreparedNode::Copy => None,
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
    /// Restore the node's state to its prepared record at the frame named (ADR-0058).
    ///
    /// **Reserved to the render loop**: no declaration may use it, and a test holds every
    /// declared control below [`Self::RESERVED_FLOOR`]. It reaches a kernel as a timed
    /// control like any other, so the reset lands at a sample rather than a boundary.
    pub const RESET: Self = Self(u8::MAX);
    /// Fade the step's output linearly from one to zero over the frames the value carries,
    /// then hold silence until [`Self::RESET`] (ADR-0058). Handled by the voice-sum kernels.
    pub const FADE_OUT: Self = Self(u8::MAX - 1);
    /// Every declared control index is below this.
    pub const RESERVED_FLOOR: Self = Self(u8::MAX - 1);

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

/// An envelope's velocity control: the scale its emitted level is multiplied by.
///
/// `SOUND-INV-021`'s velocity destination. It is a **level**, not an edge: a note-on writes
/// it beside the gate and it stays until the next one writes it, which is why it lives in
/// state rather than being consumed by the frame it arrives on.
///
/// It scales the *emitted* level rather than the attack's target, which is V1's law read at
/// `crates/synth_modules/src/envelope.rs` and recorded in ADR-0025: V1 attacks to `1.0`,
/// keeps the authored sustain as its internal target, and multiplies the completed level.
/// Aiming the attack at the velocity instead would hard-code full sensitivity and would break
/// on this kernel's own handoff, which assigns `level = 1.0` unconditionally.
pub const ENVELOPE_VELOCITY: ControlIndex = ControlIndex::new(1);
/// A sine's frequency control.
pub const SINE_FREQUENCY: ControlIndex = ControlIndex::new(0);
/// A sine's amplitude control.
pub const SINE_AMPLITUDE: ControlIndex = ControlIndex::new(1);

/// A sawtooth's frequency control.
pub const SAW_FREQUENCY: ControlIndex = ControlIndex::new(0);

/// A sawtooth's amplitude control.
pub const SAW_AMPLITUDE: ControlIndex = ControlIndex::new(1);

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
    /// This node's quantum-rate controls, one value per frame each, in the declaration's
    /// control order — `SOUND-INV-024`'s segment, already advanced by the slot.
    ///
    /// A kernel reads one value per sample from here and never advances anything: the
    /// renderer advanced every slot before the schedule walk, so what a frame reads is the
    /// segment's value at that frame, and its last frame reads exactly the target. Indexed
    /// through [`ramp_of`], which is how a kernel with one such control names it.
    pub ramps: &'a [f32],
}

/// The per-frame values of a node's `index`-th quantum-rate control, from its ramps.
///
/// A kernel reads `buffer.get(frame).or(buffer.last())`: exactly the frame's value inside a
/// quantum, which is the only length the renderer ever asks for, and the segment's last
/// value held for a run longer than one — a test harness's shape, never the loop's.
///
/// A free function over the slice rather than a method on [`NodeIo`], so a kernel can hold
/// it across the loop that writes `io.out`: the two are disjoint fields, and a method would
/// borrow the whole. Empty rather than a panic for an index the node has no buffer for,
/// which a kernel then reads as silence: a declaration naming a control the renderer did
/// not prepare a buffer for is a registry inconsistency, and the one thing the audio thread
/// can do about it is not trap.
#[must_use]
pub fn ramp_of(ramps: &[f32], index: usize) -> &[f32] {
    let quantum = crate::time::QUANTUM_FRAMES as usize;
    let start = index.saturating_mul(quantum);
    ramps
        .get(start..start.saturating_add(quantum))
        .unwrap_or(&[])
}

/// One control change at a resolved offset inside the quantum.
///
/// The sample-positioned twin of a slot's quantum-rate buffer: the same node-local control
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
    ramps: &'a [f32],
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
        ramps,
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
    let NodeState::Sine { phase, frequency } = state else {
        return;
    };
    // The amplitude is quantum-rate: a parameter event inside a quantum takes effect at
    // the next boundary — ADR-0001 clause 13's causality made concrete — and from there
    // the slot's segment (`SOUND-INV-024`) supplies one value per frame, read below. The
    // frequency is sample-positioned, and the loop says why.
    let peak = ramp_of(io.ramps, 0);
    let mut increment = f64::from(frequency.as_f32()) * seconds_per_frame;
    let mut running = *phase;
    let mut due = 0_usize;
    for (frame, sample) in io.out.iter_mut().enumerate() {
        // `SOUND-INV-021`'s pitch destination, applied before the frame it names is
        // written. A note's key describes the note its gate starts, so the frequency has
        // to be in force at that sample rather than at the boundary after it — otherwise
        // every note not landing on a boundary sounds its predecessor's pitch for up to a
        // quantum. `while` rather than `if` because two writes may share a quantum.
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, SINE_FREQUENCY) {
                *frequency = control.value.into_frequency();
                increment = f64::from(frequency.as_f32()) * seconds_per_frame;
            } else if matches!(control.control, ControlIndex::RESET)
                && let NodeState::Sine {
                    phase: prepared_phase,
                    frequency: prepared_frequency,
                } = NodeState::initial(prepared)
            {
                // ADR-0058: the instance restarts as prepared before the frame is written.
                running = prepared_phase;
                *frequency = prepared_frequency;
                increment = f64::from(frequency.as_f32()) * seconds_per_frame;
            }
        }
        let amplitude = f64::from(peak.get(frame).or(peak.last()).copied().unwrap_or(0.0));
        *sample = (amplitude * (std::f64::consts::TAU * running).sin()) as f32;
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

/// The PolyBLEP residual at a normalized phase, for a step of `dt` per frame.
///
/// A sawtooth's discontinuity is a unit step once per period. Sampling it directly puts a
/// perfect edge into a band-limited signal, and every harmonic above Nyquist folds back — so
/// the edge is *corrected* rather than filtered: this returns the difference between the ideal
/// band-limited step and the sampled one, over the two frames that straddle the wrap.
///
/// Away from the discontinuity it is exactly zero, which is what keeps the correction local
/// and the rest of the ramp untouched.
///
/// # Its domain, and what happens outside it
///
/// The two windows are `[0, dt)` and `(1 - dt, 1)`, so they are disjoint only while
/// `dt < 0.5` — a frequency below half the sample rate, which is Nyquist. At or above it they
/// overlap, the first branch wins for phases the second describes, and the residual stops
/// being small: at `dt = 2.1` and phase `0.9` it returns `-0.327`, which added to a naive
/// `0.8` gives `1.127` and breaks the kernel's own statement that the ramp runs between the
/// negated amplitude and it. An independent review found that, with those numbers.
///
/// So the correction is **zero outside its domain**, and [`saw`] does not use it there: what
/// the kernel emits past Nyquist is decided at the kernel, not here.
///
/// Real-time legal by inspection: four comparisons and four multiplies, no branch that
/// allocates, no call that can panic. `dt` is guarded against zero because the residual
/// divides by it, and a zero-frequency sawtooth is an ordinary thing to ask for.
#[inline]
fn poly_blep(phase: f64, dt: f64) -> f64 {
    // At or above Nyquist the two windows overlap and the residual is no longer a correction.
    // A non-finite step takes the same branch: it cannot be compared into the domain, so it is
    // not in it. Spelled as two positive tests rather than one negation, which reads as a
    // partial-order trap whether or not it is one.
    if !dt.is_finite() || dt >= 0.5 {
        return 0.0;
    }
    let dt = if dt > 0.0 { dt } else { f64::EPSILON };
    if phase < dt {
        // The frame after the wrap.
        let t = phase / dt;
        2.0 * t - t * t - 1.0
    } else if phase > 1.0 - dt {
        // The frame before it.
        let t = (phase - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

/// A band-limited sawtooth, from a phase accumulator.
///
/// The naive ramp is `2·phase − 1`, rising from `−1` to `+1` across a period and dropping
/// discontinuously at the wrap. The PolyBLEP residual above subtracts the aliasing that
/// discontinuity would otherwise fold into the band.
///
/// **Above Nyquist it is silent**, because a sawtooth whose fundamental is at or above
/// Nyquist has no partial below it.
///
/// `SOUND-INV-013` requires this kernel to justify itself by its own checks rather than by
/// likeness to V1, and the checks are: a rising ramp between the wraps, no DC over whole
/// periods, amplitude scaling, a guarded divisor at zero frequency, bounded phase at a
/// negative one, silence past Nyquist, and aliasing measured at the bins the folded partials
/// actually land in. None of those mentions V1.
pub fn saw(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Saw {
        seconds_per_frame, ..
    } = prepared
    else {
        return;
    };
    let NodeState::Saw { phase, frequency } = state else {
        return;
    };
    // The amplitude is read per frame from the slot's segment, exactly as the sine's is;
    // the frequency is a pitch destination and is applied at the sample it names, for the
    // reason the sine gives.
    let peak = ramp_of(io.ramps, 0);
    let mut increment = f64::from(frequency.as_f32()) * seconds_per_frame;
    // The residual is a function of the step's magnitude. A negative frequency runs the phase
    // backwards, and the discontinuity is the same size either way.
    let mut step = increment.abs();
    // A sawtooth's partials are its fundamental and every multiple of it. Once the
    // fundamental reaches Nyquist there is no partial below Nyquist at all, so the
    // band-limited signal is **exactly zero** — silence is the answer rather than a fallback.
    //
    // An earlier revision emitted the naive ramp here, reasoning that it was at least bounded.
    // An independent review showed what that actually produces: at a 48 kHz frequency the
    // phase advances by one whole period per frame and never moves, so every sample is `-1` —
    // constant DC from a node whose contract says DC-free. At 24 kHz it alternates `-1, 0`,
    // which is DC-biased by half the amplitude. Neither is a sawtooth.
    let mut representable = step.is_finite() && step < 0.5;
    let mut running = *phase;
    let mut due = 0_usize;
    for (frame, sample) in io.out.iter_mut().enumerate() {
        // `SOUND-INV-021`'s pitch destination, as the sine applies it. The three values
        // derived from the frequency are re-derived with it, because a stale `step` would
        // aim the band-limiting correction at the wrong discontinuity width.
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, SAW_FREQUENCY) {
                *frequency = control.value.into_frequency();
                increment = f64::from(frequency.as_f32()) * seconds_per_frame;
                step = increment.abs();
                representable = step.is_finite() && step < 0.5;
            } else if matches!(control.control, ControlIndex::RESET)
                && let NodeState::Saw {
                    phase: prepared_phase,
                    frequency: prepared_frequency,
                } = NodeState::initial(prepared)
            {
                // ADR-0058: the instance restarts as prepared before the frame is written.
                running = prepared_phase;
                *frequency = prepared_frequency;
                increment = f64::from(frequency.as_f32()) * seconds_per_frame;
                step = increment.abs();
                representable = step.is_finite() && step < 0.5;
            }
        }
        *sample = if representable {
            let naive = 2.0 * running - 1.0;
            let amplitude = f64::from(peak.get(frame).or(peak.last()).copied().unwrap_or(0.0));
            (amplitude * (naive - poly_blep(running, step))) as f32
        } else {
            0.0
        };
        running += increment;
        // Both directions, for the reason the sine gives: a negative frequency would otherwise
        // walk the phase below zero without bound.
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
        velocity,
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
        velocity: *velocity,
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
            match control.control {
                ENVELOPE_GATE => run.gate(control.value, sustain, *attack_frames, *release_frames),
                // `SOUND-INV-021`'s velocity destination. A level rather than an edge, so
                // it is stored and not consumed: it stands until the next note-on writes
                // it. Written **before** the gate of the note it arrives with, which is
                // what the renderer's expansion order guarantees and what makes this a
                // plain assignment rather than a rule about which came first.
                //
                // Through the **saturating constructor**, which is where the domain lives:
                // `NoteVelocity::new` refuses out-of-range input and is what the note
                // payload is built through, while this path has no way to refuse and so
                // takes the documented policy the type owns. See that constructor for why
                // the two differ.
                ENVELOPE_VELOCITY => {
                    run.velocity = NoteVelocity::saturating(control.value.as_f32());
                }
                // ADR-0058: the taken voice's envelope is idle again, at zero, with nothing
                // held, so the note that follows attacks from silence as a fresh voice does.
                ControlIndex::RESET => {
                    run = Run {
                        stage: Segment::Idle,
                        level: 0.0,
                        target: 0.0,
                        step: 0.0,
                        remaining: SegmentFrames::NONE,
                        held: false,
                        velocity: NoteVelocity::FULL,
                    };
                }
                _ => {}
            }
        }
        run.hand_over(sustain, *decay_frames);
        let level = match run.stage {
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
        // The **unscaled** level is what the run carries forward: every segment's
        // arithmetic, and the level a gate edge releases from, are in the authored space.
        // Velocity scales what leaves the node and nothing else, which is V1's law — it
        // multiplies the completed envelope rather than aiming a segment at the velocity.
        run.level = level;
        *sample = level * run.velocity.as_f32();
    }
    // Settled before it is stored, so a quantum that ends exactly on a segment boundary
    // leaves the state on the segment that follows rather than on the exhausted one.
    run.hand_over(sustain, *decay_frames);
    // And the level stored is the one the **next** sample will have, not the last one
    // written: the counter has already moved past it. A gate edge arriving at a quantum
    // boundary starts its ramp from this value, and starting from the previous sample
    // instead would put a step in the signal that nothing in the plan asked for.
    run.level = run.boundary_level(sustain);

    (
        *segment, *level, *target, *step, *remaining, *held, *velocity,
    ) = (
        run.stage,
        run.level,
        run.target,
        run.step,
        run.remaining,
        run.held,
        run.velocity,
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
    velocity: NoteVelocity,
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
    let mut due = 0_usize;
    for (index, sample) in io.out.iter_mut().enumerate() {
        // The filter declares no sample-positioned control of its own; the one control it
        // takes is the loop's reset (ADR-0058), which clears both integrators at the frame.
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != index {
                break;
            }
            due += 1;
            if matches!(control.control, ControlIndex::RESET) {
                (first, second) = (0.0, 0.0);
            }
        }
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

/// A monitor: the input passed through unchanged, so the tap on its output names the
/// signal that entered it (`SOUND-INV-022`, passive).
///
/// In-place safe, and nothing to do in place: the output already holds the input. The
/// layouts are equal by the declaration, so this is a plain per-sample copy where the
/// arena gave the two distinct regions.
pub fn monitor(_prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
    match io.inputs[0] {
        InputBuffer::Patched(source) => {
            for (index, sample) in io.out.iter_mut().enumerate() {
                *sample = source.get(index).copied().unwrap_or(0.0);
            }
        }
        InputBuffer::InPlace => {}
        InputBuffer::Unpatched => io.out.fill(0.0),
    }
}

/// One voice instance's output added into the voice sum (`P06-S001`).
///
/// The compiler inserts one of these per instance after the first, whose output is copied
/// into the sum region instead; the region is this step's **in-place** second input, so the
/// sum accumulates where the downstream node reads it. An unpatched first input adds nothing,
/// which is what an instance that produced no buffer contributes.
pub fn accumulate(_prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let InputBuffer::Patched(source) = io.inputs[0] else {
        return;
    };
    let NodeState::Sum {
        fade_remaining,
        fade_total,
    } = state
    else {
        return;
    };
    if *fade_total == 0 && io.controls.is_empty() {
        // No fade in force and none due: the sum as it always was, bit for bit.
        for (index, sample) in io.out.iter_mut().enumerate() {
            *sample += source.get(index).copied().unwrap_or(0.0);
        }
        return;
    }
    let mut fade = Fade {
        remaining: *fade_remaining,
        total: *fade_total,
    };
    let mut due = 0_usize;
    for (index, sample) in io.out.iter_mut().enumerate() {
        fade.take(io.controls, &mut due, index);
        *sample += fade.gain() * source.get(index).copied().unwrap_or(0.0);
    }
    (*fade_remaining, *fade_total) = (fade.remaining, fade.total);
}

/// A voice sum step's fade (ADR-0058), while a quantum is being written.
///
/// `gain` is `remaining / total` and falls by one frame per frame; at zero it holds until
/// a reset, so a taken voice whose fade completed contributes nothing until the new note
/// starts on it. With no fade in force the gain is exactly one.
struct Fade {
    remaining: u32,
    total: u32,
}

impl Fade {
    /// Apply the controls due at `frame`: a fade-out starts one, a reset ends it.
    fn take(&mut self, controls: &[TimedControl], due: &mut usize, frame: usize) {
        while let Some(control) = controls.get(*due) {
            if control.offset.as_usize() != frame {
                break;
            }
            *due += 1;
            match control.control {
                ControlIndex::FADE_OUT => {
                    let frames = control.value.as_frames();
                    (self.remaining, self.total) = (frames, frames);
                }
                ControlIndex::RESET => (self.remaining, self.total) = (0, 0),
                _ => {}
            }
        }
    }

    /// This frame's gain, and one frame of the fade spent.
    fn gain(&mut self) -> f32 {
        if self.total == 0 {
            return 1.0;
        }
        let gain = self.remaining as f32 / self.total as f32;
        self.remaining = self.remaining.saturating_sub(1);
        gain
    }
}

/// One buffer copied into another.
pub fn copy(_prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
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
    let fading = matches!(state, NodeState::Sum { fade_total, .. } if *fade_total != 0)
        || !io.controls.is_empty();
    if !fading {
        for (frame, input) in source.iter().enumerate() {
            for channel in 0..channels {
                if let Some(sample) = io.out.get_mut(frame * channels + channel) {
                    *sample = *input;
                }
            }
        }
        return;
    }
    // ADR-0058: the copy is instance 0's voice-sum step, so it carries that instance's
    // fade exactly as the accumulates carry the others'.
    let NodeState::Sum {
        fade_remaining,
        fade_total,
    } = state
    else {
        return;
    };
    let mut fade = Fade {
        remaining: *fade_remaining,
        total: *fade_total,
    };
    let mut due = 0_usize;
    for (frame, input) in source.iter().enumerate() {
        fade.take(io.controls, &mut due, frame);
        let scaled = fade.gain() * *input;
        for channel in 0..channels {
            if let Some(sample) = io.out.get_mut(frame * channels + channel) {
                *sample = scaled;
            }
        }
    }
    (*fade_remaining, *fade_total) = (fade.remaining, fade.total);
}
