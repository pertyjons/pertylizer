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

use crate::plan::{BufferRegion, InputBinding, NodeStep, SampleSlot};
use crate::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, DelayFeedback, DelayTime, Frequency, GainFactor,
    KeyIdentity, NormalizedLevel, NoteVelocity, ParameterValue, Resonance, Seconds, SegmentFrames,
};
use crate::sample::{
    KeyRange, LoopRegion, PlayMode, PlaybackRegion, PreparedSample, SUSTAIN_FADE_FRAMES,
    VelocityRange,
};
use crate::time::{PlanPosition, QuantumOffset};

/// How many inputs one kernel may be handed.
///
/// A bound rather than a guess: the binding below hands out one mutable and up to this
/// many shared borrows of one arena, and doing that without aliasing needs a fixed
/// number of them. Raising it is a deliberate change to the kernel signature.
pub const MAX_INPUTS: usize = synth_core::script::MAX_SOURCES;

/// Fill the first two input bindings for native nodes.
pub(crate) const fn pair_inputs(
    first: Option<crate::plan::BufferSlot>,
    second: Option<crate::plan::BufferSlot>,
) -> [Option<crate::plan::BufferSlot>; MAX_INPUTS] {
    let mut inputs = [None; MAX_INPUTS];
    inputs[0] = first;
    inputs[1] = second;
    inputs
}

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
/// [`velocity_scaler`].
pub const VELOCITY_SCALER: Kernel = Kernel(velocity_scaler);
/// The sampler's kernel (ADR-0026).
pub const SAMPLER: Kernel = Kernel(sampler);
/// The monitor's kernel: its input, unchanged.
pub const MONITOR: Kernel = Kernel(monitor);
/// The mix channel's kernel (`SOUND-INV-031`).
pub const CHANNEL: Kernel = Kernel(channel);
/// The explicit sum's kernel: the summed region, unchanged (`SOUND-INV-031`).
pub const MIX: Kernel = Kernel(mix);
/// The balance stage's kernel (`SOUND-INV-032`).
pub const BALANCE: Kernel = Kernel(balance);
/// The trim's kernel (`SOUND-INV-032`).
pub const TRIM: Kernel = Kernel(trim);
/// The send's kernel (`SOUND-INV-034`).
pub const SEND: Kernel = Kernel(send);
/// V1's post-fader channel send's kernel (`SOUND-INV-034`).
pub const POST_FADER_SEND: Kernel = Kernel(post_fader_send);
/// V1's channel-stage soft clipper's kernel (`SOUND-INV-032`).
pub const SOFT_CLIP: Kernel = Kernel(soft_clip);
/// V1's output clamp's kernel (`SOUND-INV-032`).
pub const HARD_CLAMP: Kernel = Kernel(hard_clamp);
/// V1's distortion insert, soft-clip mode (`SOUND-INV-033`).
pub const DISTORTION: Kernel = Kernel(distortion);
/// V1's delay insert, mono mode (`SOUND-INV-033`).
pub const DELAY: Kernel = Kernel(delay);
/// The low-frequency oscillator's kernel (`SOUND-INV-027`).
pub const LFO: Kernel = Kernel(lfo);
/// The bounded, prepared YAMS VM.
pub const SCRIPT: Kernel = Kernel(script);
/// The per-sample YAMS domain.
pub const AUDIO_SCRIPT: Kernel = Kernel(audio_script);
pub const NOTE_SCRIPT: Kernel = Kernel(note_script);

pub fn note_script(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    crate::script::hot::capture(prepared, state, io);
}

pub fn audio_script(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    crate::script::hot::audio(prepared, state, io);
}

pub fn script(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    crate::script::hot::run(prepared, state, io);
}

/// A node's immutable prepared data.
///
/// Prepared once, off the audio thread, and shared by every state that runs it. What is
/// *derived* from the stream — a sine's per-frame phase step, a filter's coefficients —
/// is computed here rather than per quantum, which is the whole point of the split.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PreparedNode {
    /// Immutable VM code, identity seed and evaluation rate.
    Script {
        program: crate::script::ScriptSlot,
        seed: crate::script::ScriptSeed,
        rate: f32,
    },
    /// A declared controller source.
    Controller {
        kind: crate::controller::ControllerKind,
    },
    /// A declared per-occurrence source.
    NoteSource,
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
        /// The authored attack, the base its slot starts from (`P07-S002`).
        ///
        /// Seconds rather than frames since the times became controls: the kernel reads a
        /// duration per frame from its slot and converts it where a segment starts, by
        /// the arithmetic preparation used to refuse an authored duration no counter holds.
        attack: Seconds,
        /// The authored decay.
        decay: Seconds,
        /// The authored release.
        release: Seconds,
        /// The level a held gate settles at, the base its slot starts from.
        ///
        /// Still the validated type: a raw `f32` here would let a prepared record carry a
        /// sustain the IR would have refused.
        sustain: NormalizedLevel,
        /// The authored velocity sensitivity, the base its slot starts from (ADR-0059).
        velocity_sensitivity: NormalizedLevel,
        /// The stream's rate, as the frame conversion multiplies by it.
        rate: f64,
    },
    /// A one-zone sampler's zone, resolved against the plan (ADR-0026).
    ///
    /// The sample is a **slot** into the plan's table, not the frames: the record stays
    /// `Copy` and the frames are held once per plan. The root's frequency is resolved at
    /// admission through the scope's tuning and written here, so the kernel divides by a
    /// number and never converts a key.
    Sampler {
        /// Which prepared sample of the plan the zone plays.
        sample: SampleSlot,
        /// The keys that select the zone; a note outside it plays nothing.
        keys: KeyRange,
        /// The velocities that select the zone.
        velocities: VelocityRange,
        /// The key the sample plays at its recorded rate.
        root: KeyIdentity,
        /// That key's frequency under the scope's tuning.
        root_frequency: Frequency,
        /// `2^(fine_cents / 1200)`, V1's fine-tune factor.
        fine_factor: f64,
        /// The frames the zone plays.
        region: PlaybackRegion,
        /// The frames it repeats in `Loop` mode, if any.
        loop_region: Option<LoopRegion>,
        /// The zone's own level.
        gain: GainFactor,
        /// The authored level, the base its quantum-rate slot starts from.
        level: Amplitude,
        /// The authored velocity sensitivity, the base its slot starts from.
        velocity_sensitivity: NormalizedLevel,
        /// Where in the region a note starts, as a fraction of it.
        start_offset: NormalizedLevel,
        /// How it plays once triggered.
        play_mode: PlayMode,
    },
    /// A velocity scaler's authored sensitivity (ADR-0059).
    VelocityScaler {
        /// The base its sensitivity slot starts from.
        sensitivity: NormalizedLevel,
    },
    /// A mix channel's authored fader, pan and mute (`SOUND-INV-031`): the bases its three
    /// slots start from.
    Channel {
        /// The base the fader slot starts from.
        fader: Amplitude,
        /// The base the pan slot starts from.
        pan: crate::controller::BipolarLevel,
        /// Whether the mute starts held.
        muted: bool,
    },
    /// An explicit sum: nothing is prepared, the summed region is its input.
    Mix,
    /// A balance stage's authored level, pan and mute (`SOUND-INV-032`): the bases its three
    /// slots start from.
    Balance {
        /// The base the level slot starts from.
        level: Amplitude,
        /// The base the pan slot starts from.
        pan: crate::controller::BipolarLevel,
        /// Whether the mute starts held.
        muted: bool,
    },
    /// A trim's authored level (`SOUND-INV-032`): the base its one slot starts from.
    Trim {
        /// The base the level slot starts from.
        level: Amplitude,
    },
    /// A send's authored level and mute (`SOUND-INV-034`).
    Send {
        /// The base the level slot starts from.
        level: Amplitude,
        /// Whether the mute starts held.
        muted: bool,
    },
    /// A post-fader send's authored bases (`SOUND-INV-034`): the channel's fader, pan and
    /// mute, and the send's level.
    PostFaderSend {
        /// The base the fader slot starts from.
        fader: Amplitude,
        /// The base the pan slot starts from.
        pan: crate::controller::BipolarLevel,
        /// Whether the mute starts held.
        muted: bool,
        /// The base the level slot starts from.
        level: Amplitude,
    },
    /// V1's soft clipper: nothing is prepared, the law has no parameter.
    SoftClip,
    /// V1's output clamp: nothing is prepared, the bounds are full scale.
    HardClamp,
    /// V1's distortion in its soft-clip mode: the three authored levels and the rate its
    /// tone corner is derived at (`SOUND-INV-033`).
    Distortion {
        /// The authored drive, the base its slot starts from.
        drive: NormalizedLevel,
        /// The authored tone.
        tone: NormalizedLevel,
        /// The authored mix.
        mix: NormalizedLevel,
        /// The stream's rate, for the tone filter's coefficient.
        rate: f32,
    },
    /// V1's delay in its mono mode: the five authored values, the rate, and the frames V1
    /// keeps per side (`SOUND-INV-033`).
    Delay {
        /// The authored left time.
        time_left: DelayTime,
        /// The authored right time.
        time_right: DelayTime,
        /// The authored feedback.
        feedback: DelayFeedback,
        /// The authored mix.
        mix: NormalizedLevel,
        /// The authored tone, the feedback high cut.
        tone: NormalizedLevel,
        /// The stream's rate, for the times and the high cut's coefficient.
        rate: f32,
        /// The frames per side, V1's `2 s` at the rate truncated as V1 truncates it — the
        /// history the renderer hands the kernel holds two of these.
        line: usize,
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
        /// The three derived integrator coefficients for the authored corner and quality,
        /// what the state starts with and what an unmodulated filter reads throughout.
        ///
        /// The damping the quality factor implies is *inside* them; a low-pass output
        /// never reads it separately.
        integrator: [f32; 3],
        /// The authored corner frequency, the base its slot starts from (`P07-S002`).
        cutoff: CutoffFrequency,
        /// The authored quality factor, the base its slot starts from.
        resonance: Resonance,
        /// The stream's rate, for the coefficients a moved corner or quality needs.
        rate: f64,
    },
    /// One audio input scaled by one control input. It carries nothing of its own.
    Amplifier,
    /// One buffer copied into another.
    ///
    /// The compiler's own operation rather than an authored node: ADR-0002 clause 7
    /// makes a mono-to-stereo widening a scheduled operation with an identity, and this
    /// is the kernel that performs it.
    Copy,
    /// A low-frequency oscillator's shape, polarity and offset, with its phase step and the
    /// bases its two slots start from (`SOUND-INV-027`).
    Lfo {
        /// One frame as a fraction of a second, as a sine's is.
        seconds_per_frame: f64,
        /// The shape traced over one period.
        waveform: crate::ir::LfoWaveform,
        /// Whether the shape is folded into `[0, 1]`.
        polarity: crate::ir::LfoPolarity,
        /// Where in the period the cycle starts, added where the shape is read.
        phase_offset: f64,
        /// The rate the slot starts from.
        rate: Frequency,
        /// The depth the slot starts from.
        depth: NormalizedLevel,
    },
}

/// A node's mutable state.
///
/// One record per node instance, owned by the renderer and never by the plan. A plan can
/// therefore be rendered by two streams at once, and Phase 6's voice pool gets many
/// states over one prepared node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeState {
    /// Each instance owns its VM registers and deterministic reset seed.
    Script {
        registers: synth_core::script::RegisterFile,
        seed: crate::script::ScriptSeed,
        reset_pending: bool,
        voice: crate::script::ScriptVoiceId,
        first_sample: bool,
        captured: [f32; synth_core::script::MAX_SOURCES],
        captured_once: bool,
    },
    /// A node that keeps nothing between quanta.
    Stateless,
    /// A velocity scaler's note velocity, held between quanta (ADR-0059).
    Scaled {
        /// The velocity last written, applied to every sample.
        velocity: NoteVelocity,
    },
    /// A send's mute, held between quanta (`SOUND-INV-034`); its level is a quantum-rate
    /// slot read from the ramp.
    Send {
        /// Whether the mute was held at the end of the last quantum.
        muted: bool,
    },
    /// A post-fader send's mute, held between quanta (`SOUND-INV-034`); its fader, pan and
    /// level are quantum-rate slots read from the ramps.
    PostFaderSend {
        /// Whether the mute was held at the end of the last quantum.
        muted: bool,
    },
    /// A mix channel's mute, held between quanta (`SOUND-INV-031`). Its fader and pan are
    /// quantum-rate slots read from the ramps, so nothing else is kept.
    Channel {
        /// Whether the mute was held at the end of the last quantum.
        muted: bool,
    },
    /// A balance stage's mute, held between quanta (`SOUND-INV-032`), as the channel's is.
    Balance {
        /// Whether the mute was held at the end of the last quantum.
        muted: bool,
    },
    /// A distortion's tone filter, one state per side, and the tone and coefficient last
    /// read, so the coefficient is re-derived only where the tone moves (`SOUND-INV-033`).
    Distortion {
        /// The one-pole state per side.
        filters: [f32; 2],
        /// The tone last read from its slot.
        tone: f32,
        /// The coefficient in force, that tone's.
        coef: f32,
    },
    /// A delay's write index into its history, its feedback filter per side, and the tone
    /// and coefficient last read (`SOUND-INV-033`). The lines themselves are the history the
    /// renderer keeps and hands the kernel per quantum.
    Delay {
        /// The frame the next input is written at, in both lines.
        write: usize,
        /// The feedback high cut's one-pole state per side.
        filters: [f32; 2],
        /// The tone last read from its slot.
        tone: f32,
        /// The coefficient in force, that tone's.
        coef: f32,
    },
    /// An LFO's place in its period, in `[0, 1)` (`SOUND-INV-027`). Its rate and depth are
    /// quantum-rate slots and are read from the ramps, so nothing else is kept.
    Lfo {
        /// The accumulator, before the authored offset.
        phase: f64,
    },
    /// A sampler's playback, held between quanta (ADR-0026 clause 9).
    ///
    /// One record per voice instance, sized at preparation and reset by the on edge: what
    /// V1 allocated as a player per note is a position and a rate here.
    Sampler {
        /// Where in the sample the next frame is read, in frames; fractional.
        position: f64,
        /// Frames advanced per output frame: the resolved frequency over the root's, times
        /// the fine-tune factor.
        rate: f64,
        /// The velocity last written, applied to every sample.
        velocity: NoteVelocity,
        /// Whether it is idle, playing, or fading after the off edge.
        playback: Playback,
        /// Frames left in the fade, when fading.
        fade_remaining: u32,
        /// Whether the trigger is currently held, so an on edge is a transition rather
        /// than any positive value — as the envelope's gate is.
        held: bool,
    },
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
        /// The corner frequency last read from its slot (`P07-S002`): a pair equal to the
        /// last one read re-derives nothing.
        cutoff: f32,
        /// The quality factor last read from its slot.
        resonance: f32,
        /// The coefficients in force: those of the last **usable** pair read, which is the
        /// pair above except where that pair had no usable filter and these were held.
        integrator: [f32; 3],
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

/// Where a sampler is in a note (ADR-0026 clause 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Playback {
    /// Nothing sounds.
    Idle,
    /// The region, or its loop, is being read.
    Playing,
    /// The off edge arrived in `Sustain` or `Loop` mode: the read continues under a
    /// linear fade of [`SUSTAIN_FADE_FRAMES`] frames.
    Fading,
}

impl NodeState {
    /// The state a prepared node starts in.
    #[must_use]
    pub fn initial(prepared: &PreparedNode) -> Self {
        match prepared {
            PreparedNode::Script { seed, .. } => Self::Script {
                registers: synth_core::script::RegisterFile::new(
                    0,
                    seed.for_voice(crate::script::ScriptVoiceId::ZERO).as_u64(),
                ),
                seed: seed.for_voice(crate::script::ScriptVoiceId::ZERO),
                reset_pending: false,
                voice: crate::script::ScriptVoiceId::ZERO,
                first_sample: true,
                captured: [0.0; synth_core::script::MAX_SOURCES],
                captured_once: false,
            },
            PreparedNode::Sine { frequency, .. } => Self::Sine {
                phase: 0.0,
                frequency: *frequency,
            },
            PreparedNode::Saw { frequency, .. } => Self::Saw {
                phase: 0.0,
                frequency: *frequency,
            },
            PreparedNode::Filter {
                integrator,
                cutoff,
                resonance,
                ..
            } => Self::Filter {
                band: 0.0,
                low: 0.0,
                cutoff: cutoff.as_f32(),
                resonance: resonance.as_f32(),
                integrator: *integrator,
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
            PreparedNode::VelocityScaler { .. } => Self::Scaled {
                velocity: NoteVelocity::FULL,
            },
            PreparedNode::Channel { muted, .. } => Self::Channel { muted: *muted },
            PreparedNode::Mix => Self::Stateless,
            PreparedNode::Balance { muted, .. } => Self::Balance { muted: *muted },
            PreparedNode::Distortion { tone, rate, .. } => Self::Distortion {
                filters: [0.0; 2],
                tone: tone.as_f32(),
                coef: distortion_tone_coefficient(tone.as_f32(), *rate),
            },
            PreparedNode::Delay { tone, rate, .. } => Self::Delay {
                write: 0,
                filters: [0.0; 2],
                tone: tone.as_f32(),
                coef: delay_high_cut_coefficient(tone.as_f32(), *rate),
            },
            PreparedNode::Trim { .. } | PreparedNode::SoftClip | PreparedNode::HardClamp => {
                Self::Stateless
            }
            PreparedNode::Send { muted, .. } => Self::Send { muted: *muted },
            PreparedNode::PostFaderSend { muted, .. } => Self::PostFaderSend { muted: *muted },
            PreparedNode::Controller { .. } | PreparedNode::NoteSource => Self::Stateless,
            PreparedNode::Lfo { .. } => Self::Lfo { phase: 0.0 },
            PreparedNode::Sampler { .. } => Self::Sampler {
                position: 0.0,
                rate: 0.0,
                velocity: NoteVelocity::FULL,
                playback: Playback::Idle,
                fade_remaining: 0,
                held: false,
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
            Self::Scaled { velocity } => match control {
                VELOCITY_SCALER_VELOCITY => ParameterValue::new(velocity.as_f32()).ok(),
                _ => None,
            },
            Self::Channel { muted } => match control {
                CHANNEL_MUTE => Some(if *muted {
                    ParameterValue::ONE
                } else {
                    ParameterValue::ZERO
                }),
                _ => None,
            },
            Self::Send { muted } => match control {
                SEND_MUTE => Some(if *muted {
                    ParameterValue::ONE
                } else {
                    ParameterValue::ZERO
                }),
                _ => None,
            },
            Self::PostFaderSend { muted } => match control {
                POST_FADER_SEND_MUTE => Some(if *muted {
                    ParameterValue::ONE
                } else {
                    ParameterValue::ZERO
                }),
                _ => None,
            },
            Self::Balance { muted } => match control {
                BALANCE_MUTE => Some(if *muted {
                    ParameterValue::ONE
                } else {
                    ParameterValue::ZERO
                }),
                _ => None,
            },
            Self::Sampler { velocity, held, .. } => match control {
                SAMPLER_TRIGGER => Some(if *held {
                    ParameterValue::ONE
                } else {
                    ParameterValue::ZERO
                }),
                SAMPLER_VELOCITY => ParameterValue::new(velocity.as_f32()).ok(),
                _ => None,
            },
            Self::Filter { .. }
            | Self::Sum { .. }
            | Self::Lfo { .. }
            | Self::Script { .. }
            | Self::Distortion { .. }
            | Self::Delay { .. }
            | Self::Stateless => None,
        }
    }
}

/// The frames of history a prepared record's kernel keeps between quanta, at its output
/// width (`SOUND-INV-033`): the delay's line, and nothing for every other record. Read
/// once, at preparation, to size the renderer's history slab; the declaration's timing
/// states the same figure through the same line length, and
/// `a_delays_history_is_charged_as_the_renderer_allocates_it` holds the two equal.
#[must_use]
pub fn history_frames(prepared: &PreparedNode) -> usize {
    match prepared {
        PreparedNode::Delay { line, .. } => *line,
        _ => 0,
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
        PreparedNode::Envelope {
            attack,
            decay,
            release,
            sustain,
            velocity_sensitivity,
            ..
        } => match control {
            ENVELOPE_VELOCITY_SENSITIVITY => {
                Some(ParameterValue::from_level(*velocity_sensitivity))
            }
            ENVELOPE_ATTACK => Some(ParameterValue::saturating(attack.as_f32())),
            ENVELOPE_DECAY => Some(ParameterValue::saturating(decay.as_f32())),
            ENVELOPE_SUSTAIN => Some(ParameterValue::from_level(*sustain)),
            ENVELOPE_RELEASE => Some(ParameterValue::saturating(release.as_f32())),
            _ => None,
        },
        PreparedNode::Filter {
            cutoff, resonance, ..
        } => match control {
            FILTER_CUTOFF => Some(ParameterValue::saturating(cutoff.as_f32())),
            FILTER_RESONANCE => Some(ParameterValue::saturating(resonance.as_f32())),
            _ => None,
        },
        PreparedNode::VelocityScaler { sensitivity } => match control {
            VELOCITY_SCALER_SENSITIVITY => Some(ParameterValue::from_level(*sensitivity)),
            _ => None,
        },
        PreparedNode::Channel { fader, pan, muted } => match control {
            CHANNEL_FADER => Some(ParameterValue::from_amplitude(*fader)),
            CHANNEL_PAN => Some(ParameterValue::from_bipolar(*pan)),
            CHANNEL_MUTE => Some(if *muted {
                ParameterValue::ONE
            } else {
                ParameterValue::ZERO
            }),
            _ => None,
        },
        PreparedNode::Mix => None,
        PreparedNode::Balance { level, pan, muted } => match control {
            BALANCE_LEVEL => Some(ParameterValue::from_amplitude(*level)),
            BALANCE_PAN => Some(ParameterValue::from_bipolar(*pan)),
            BALANCE_MUTE => Some(if *muted {
                ParameterValue::ONE
            } else {
                ParameterValue::ZERO
            }),
            _ => None,
        },
        PreparedNode::Trim { level } => match control {
            TRIM_LEVEL => Some(ParameterValue::from_amplitude(*level)),
            _ => None,
        },
        PreparedNode::Send { level, muted } => match control {
            SEND_LEVEL => Some(ParameterValue::from_amplitude(*level)),
            SEND_MUTE => Some(if *muted {
                ParameterValue::ONE
            } else {
                ParameterValue::ZERO
            }),
            _ => None,
        },
        PreparedNode::PostFaderSend {
            fader,
            pan,
            muted,
            level,
        } => match control {
            POST_FADER_SEND_FADER => Some(ParameterValue::from_amplitude(*fader)),
            POST_FADER_SEND_PAN => Some(ParameterValue::from_bipolar(*pan)),
            POST_FADER_SEND_MUTE => Some(if *muted {
                ParameterValue::ONE
            } else {
                ParameterValue::ZERO
            }),
            POST_FADER_SEND_LEVEL => Some(ParameterValue::from_amplitude(*level)),
            _ => None,
        },
        PreparedNode::Distortion {
            drive, tone, mix, ..
        } => match control {
            DISTORTION_DRIVE => Some(ParameterValue::from_level(*drive)),
            DISTORTION_TONE => Some(ParameterValue::from_level(*tone)),
            DISTORTION_MIX => Some(ParameterValue::from_level(*mix)),
            _ => None,
        },
        PreparedNode::Delay {
            time_left,
            time_right,
            feedback,
            mix,
            tone,
            ..
        } => match control {
            DELAY_TIME_LEFT => Some(ParameterValue::saturating(time_left.as_f32())),
            DELAY_TIME_RIGHT => Some(ParameterValue::saturating(time_right.as_f32())),
            DELAY_FEEDBACK => Some(ParameterValue::saturating(feedback.as_f32())),
            DELAY_MIX => Some(ParameterValue::from_level(*mix)),
            DELAY_TONE => Some(ParameterValue::from_level(*tone)),
            _ => None,
        },
        PreparedNode::SoftClip | PreparedNode::HardClamp => None,
        PreparedNode::Controller { .. } | PreparedNode::NoteSource => Some(ParameterValue::ZERO),
        PreparedNode::Lfo { rate, depth, .. } => match control {
            LFO_RATE => Some(ParameterValue::from_frequency(*rate)),
            LFO_DEPTH => Some(ParameterValue::from_level(*depth)),
            _ => None,
        },
        PreparedNode::Sampler {
            level,
            velocity_sensitivity,
            ..
        } => match control {
            SAMPLER_LEVEL => Some(ParameterValue::from_amplitude(*level)),
            SAMPLER_VELOCITY_SENSITIVITY => Some(ParameterValue::from_level(*velocity_sensitivity)),
            _ => None,
        },
        PreparedNode::Silence
        | PreparedNode::Constant { .. }
        | PreparedNode::Impulse { .. }
        | PreparedNode::Gain { .. }
        | PreparedNode::Amplifier
        | PreparedNode::Copy
        | PreparedNode::Script { .. } => None,
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

/// The envelope's velocity sensitivity, `s` in V1's `1 − s × (1 − v)` (ADR-0059): a
/// quantum-rate control, read per frame from its slot's ramp.
pub const ENVELOPE_VELOCITY_SENSITIVITY: ControlIndex = ControlIndex::new(2);
/// The envelope's attack, in seconds; quantum-rate (`P07-S002`).
pub const ENVELOPE_ATTACK: ControlIndex = ControlIndex::new(3);
/// The envelope's decay, in seconds.
pub const ENVELOPE_DECAY: ControlIndex = ControlIndex::new(4);
/// The envelope's sustain level.
pub const ENVELOPE_SUSTAIN: ControlIndex = ControlIndex::new(5);
/// The envelope's release, in seconds.
pub const ENVELOPE_RELEASE: ControlIndex = ControlIndex::new(6);
/// The filter's corner frequency; quantum-rate (`P07-S002`).
pub const FILTER_CUTOFF: ControlIndex = ControlIndex::new(0);
/// The filter's quality factor.
pub const FILTER_RESONANCE: ControlIndex = ControlIndex::new(1);

/// The velocity scaler's velocity destination (ADR-0059): `SOUND-INV-021`'s second velocity
/// write, held as a level like the envelope's.
pub const VELOCITY_SCALER_VELOCITY: ControlIndex = ControlIndex::new(0);

/// The velocity scaler's sensitivity, `s` in V1's `(1 − s) + s × v` (ADR-0059).
pub const VELOCITY_SCALER_SENSITIVITY: ControlIndex = ControlIndex::new(1);
/// The sampler's trigger destination: the note's on and off edges (ADR-0026).
pub const SAMPLER_TRIGGER: ControlIndex = ControlIndex::new(0);
/// The sampler's pitch destination: the frequency the scope's tuning resolves.
pub const SAMPLER_PITCH: ControlIndex = ControlIndex::new(1);
/// The sampler's velocity destination.
pub const SAMPLER_VELOCITY: ControlIndex = ControlIndex::new(2);
/// The sampler's level, V1's `level`: a quantum-rate control, read per frame.
pub const SAMPLER_LEVEL: ControlIndex = ControlIndex::new(3);
/// The sampler's velocity sensitivity, `s` in V1's `(1 − s) + s × v`.
pub const SAMPLER_VELOCITY_SENSITIVITY: ControlIndex = ControlIndex::new(4);
/// A sine's frequency control.
pub const SINE_FREQUENCY: ControlIndex = ControlIndex::new(0);
/// A sine's amplitude control.
pub const SINE_AMPLITUDE: ControlIndex = ControlIndex::new(1);

/// A sawtooth's frequency control.
pub const SAW_FREQUENCY: ControlIndex = ControlIndex::new(0);

/// A sawtooth's amplitude control.
pub const SAW_AMPLITUDE: ControlIndex = ControlIndex::new(1);
/// An LFO's rate, quantum-rate (`SOUND-INV-027`).
pub const LFO_RATE: ControlIndex = ControlIndex::new(0);
/// An LFO's depth, quantum-rate.
pub const LFO_DEPTH: ControlIndex = ControlIndex::new(1);
/// A mix channel's fader, quantum-rate, read per frame from its ramp (`SOUND-INV-031`).
pub const CHANNEL_FADER: ControlIndex = ControlIndex::new(0);
/// A mix channel's pan, quantum-rate, read per frame from its ramp.
pub const CHANNEL_PAN: ControlIndex = ControlIndex::new(1);
/// A mix channel's mute, sample-positioned: held from the frame it lands on.
pub const CHANNEL_MUTE: ControlIndex = ControlIndex::new(2);
/// A balance stage's level, quantum-rate, read per frame from its ramp (`SOUND-INV-032`).
pub const BALANCE_LEVEL: ControlIndex = ControlIndex::new(0);
/// A balance stage's pan, quantum-rate, read per frame from its ramp.
pub const BALANCE_PAN: ControlIndex = ControlIndex::new(1);
/// A balance stage's mute, sample-positioned: held from the frame it lands on.
pub const BALANCE_MUTE: ControlIndex = ControlIndex::new(2);
/// A trim's level, quantum-rate, read per frame from its ramp (`SOUND-INV-032`).
pub const TRIM_LEVEL: ControlIndex = ControlIndex::new(0);
/// A send's level, quantum-rate, read per frame from its ramp (`SOUND-INV-034`).
pub const SEND_LEVEL: ControlIndex = ControlIndex::new(0);
/// A send's mute, sample-positioned: held from the frame it lands on.
pub const SEND_MUTE: ControlIndex = ControlIndex::new(1);
/// A post-fader send's fader, the channel's, quantum-rate (`SOUND-INV-034`).
pub const POST_FADER_SEND_FADER: ControlIndex = ControlIndex::new(0);
/// A post-fader send's pan, the channel's, quantum-rate.
pub const POST_FADER_SEND_PAN: ControlIndex = ControlIndex::new(1);
/// A post-fader send's mute, the channel's, sample-positioned.
pub const POST_FADER_SEND_MUTE: ControlIndex = ControlIndex::new(2);
/// A post-fader send's level, quantum-rate; the third ramp, after the fader's and the pan's.
pub const POST_FADER_SEND_LEVEL: ControlIndex = ControlIndex::new(3);
/// A distortion's drive (`SOUND-INV-033`).
pub const DISTORTION_DRIVE: ControlIndex = ControlIndex::new(0);
/// A distortion's tone.
pub const DISTORTION_TONE: ControlIndex = ControlIndex::new(1);
/// A distortion's mix.
pub const DISTORTION_MIX: ControlIndex = ControlIndex::new(2);
/// A delay's left time (`SOUND-INV-033`).
pub const DELAY_TIME_LEFT: ControlIndex = ControlIndex::new(0);
/// A delay's right time.
pub const DELAY_TIME_RIGHT: ControlIndex = ControlIndex::new(1);
/// A delay's feedback.
pub const DELAY_FEEDBACK: ControlIndex = ControlIndex::new(2);
/// A delay's mix.
pub const DELAY_MIX: ControlIndex = ControlIndex::new(3);
/// A delay's feedback high cut.
pub const DELAY_TONE: ControlIndex = ControlIndex::new(4);

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
    /// The signal this node keeps between quanta, at its output width — the history its
    /// kind declared (`SOUND-INV-033`), allocated once per scheduled step by the renderer
    /// and handed back to the same step every quantum. Empty for every kind that declares
    /// none, which is every kind but the delay today.
    pub history: &'a mut [f32],
    /// The plan's prepared samples, indexed by a prepared record's [`SampleSlot`]
    /// (ADR-0026). Read through one index; the frames sit behind an `Arc` the plan holds.
    pub samples: &'a [PreparedSample],
    /// Prepared numeric script bindings and parameter layers.
    pub scripts: crate::script::ScriptResources<'a>,
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
    resources: NodeResources<'a>,
) -> Option<NodeIo<'a>> {
    let mut out: Option<&'a mut [f32]> = None;
    let mut inputs = [InputBuffer::Unpatched; MAX_INPUTS];
    let mut rest = buffers;
    let mut consumed = 0_usize;

    // The roles in ascending offset order, worked out at admission. Walking them forwards
    // and splitting each region off in turn is what lets one mutable and up to 32 shared
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
        history: resources.history,
        samples: resources.samples,
        scripts: resources.scripts,
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

/// A low-frequency oscillator: one control-rate value per frame from a phase accumulator,
/// `SOUND-INV-027`'s first modulation source.
///
/// Per frame, `depth × shape(phase + offset)`, the shape folded to `[0, 1]` under the
/// unipolar polarity; then the phase advances by the frame's rate. Both the rate and the
/// depth are quantum-rate slots read per frame from the ramps, so the kernel composes
/// nothing. The shapes are V1's own: `sin(2πp)`, a triangle from `-1` at the period's start
/// through `1` at its middle, `2p − 1`, and a square high through the first half. The two
/// random shapes reach no kernel — validation refuses them by name until ADR-0008 gives a
/// node a seed — and the arm writes silence rather than inventing a stream.
///
/// The only sample-positioned control it answers is the loop's reset (ADR-0058): the
/// accumulator returns to the start of the authored cycle before the frame named is written.
pub fn lfo(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Lfo {
        seconds_per_frame,
        waveform,
        polarity,
        phase_offset,
        ..
    } = prepared
    else {
        return;
    };
    let NodeState::Lfo { phase } = state else {
        return;
    };
    let rate = ramp_of(io.ramps, 0);
    let depth = ramp_of(io.ramps, 1);
    let mut running = *phase;
    let mut due = 0_usize;
    for (frame, sample) in io.out.iter_mut().enumerate() {
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, ControlIndex::RESET) {
                running = 0.0;
            }
        }
        let mut position = running + phase_offset;
        if !(0.0..1.0).contains(&position) {
            position -= position.floor();
        }
        let raw = match waveform {
            crate::ir::LfoWaveform::Sine => (std::f64::consts::TAU * position).sin(),
            crate::ir::LfoWaveform::Triangle => {
                if position < 0.5 {
                    4.0 * position - 1.0
                } else {
                    3.0 - 4.0 * position
                }
            }
            crate::ir::LfoWaveform::Sawtooth => 2.0 * position - 1.0,
            crate::ir::LfoWaveform::Square => {
                if position < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            crate::ir::LfoWaveform::SampleAndHold | crate::ir::LfoWaveform::SmoothRandom => 0.0,
        };
        let shaped = match polarity {
            crate::ir::LfoPolarity::Bipolar => raw,
            crate::ir::LfoPolarity::Unipolar => (raw + 1.0) * 0.5,
        };
        let peak = f64::from(depth.get(frame).or(depth.last()).copied().unwrap_or(0.0));
        *sample = (peak * shaped) as f32;
        let hertz = f64::from(rate.get(frame).or(rate.last()).copied().unwrap_or(0.0));
        running += hertz * seconds_per_frame;
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

/// How many frames `seconds` last at `rate`: rounded, and held to the counter's range.
///
/// The kernel's twin of preparation's `frames_in`, which **refuses** an authored duration
/// no counter holds; here the value came through a slot and nothing can refuse, so the
/// duration saturates to the longest segment the counter names. The product is the same
/// `f64` arithmetic, so an unmodulated duration converts to exactly the frames preparation
/// would have counted, and a decimal duration is not truncated one frame short.
fn frames_of(seconds: f32, rate: f64) -> SegmentFrames {
    let frames = (f64::from(seconds) * rate).round();
    if frames.is_finite() && frames >= 0.0 && frames <= f64::from(u32::MAX) {
        // Proven to fit: finite, non-negative and at most `u32::MAX` by the test above.
        SegmentFrames::new(frames as u32)
    } else {
        SegmentFrames::new(u32::MAX)
    }
}

/// The value a quantum-rate control holds at `frame`, or the authored base where the node
/// was handed no buffer for it — a harness's shape, never the renderer's.
fn control_at(ramp: &[f32], frame: usize, authored: f32) -> f32 {
    ramp.get(frame).or(ramp.last()).copied().unwrap_or(authored)
}

/// The three integrator coefficients of a two-pole low-pass in the topology-preserving
/// state-variable form: `g = tan(pi f / fs)`, a damping of `1/Q`, and the three that follow.
///
/// In `f64`, as preparation always derived them; the kernel calls this where a corner or a
/// quality moves, and preparation where a plan is admitted, so an unmodulated filter and a
/// modulated one standing at its authored values read the same coefficients.
#[must_use]
pub fn low_pass_coefficients(cutoff: f64, resonance: f64, rate: f64) -> [f32; 3] {
    let g = (std::f64::consts::PI * cutoff / rate).tan();
    let damping = 1.0 / resonance;
    let first = 1.0 / (1.0 + g * (g + damping));
    [first as f32, (g * first) as f32, (g * g * first) as f32]
}

/// Whether every coefficient is representable: zero or normal, and the middle one normal.
///
/// A subnormal coefficient has lost most of its significand and would stall the audio
/// thread on processors without flush-to-zero; preparation refuses such a plan, and the
/// kernel holds its previous coefficients where a moved value produces one.
#[must_use]
pub fn coefficients_representable(integrator: [f32; 3]) -> bool {
    let representable = |value: f32| value == 0.0 || value.is_normal();
    integrator.iter().copied().all(representable) && integrator[1].is_normal()
}

/// Whether the recurrence with these coefficients is stable, by Jury's criterion over the
/// rounded values the kernel will multiply.
#[must_use]
pub fn coefficients_stable(integrator: [f32; 3]) -> bool {
    let (first, second, third) = (
        f64::from(integrator[0]),
        f64::from(integrator[1]),
        f64::from(integrator[2]),
    );
    let trace = 2.0 * first - 2.0 * third;
    let determinant = (2.0 * first - 1.0) * (1.0 - 2.0 * third) + 4.0 * second * second;
    determinant < 1.0 && trace.abs() < 1.0 + determinant
}

/// The coefficients for a corner and quality the slot handed the kernel, or `None` where
/// the pair has no usable filter: a corner not above zero or at or above Nyquist, a quality
/// not above zero, or coefficients preparation would have refused. The kernel then holds
/// the coefficients in force rather than rendering a filter nobody asked for.
fn usable_low_pass(cutoff: f64, resonance: f64, rate: f64) -> Option<[f32; 3]> {
    if !(cutoff > 0.0 && cutoff < rate * 0.5 && resonance > 0.0) {
        return None;
    }
    let integrator = low_pass_coefficients(cutoff, resonance, rate);
    (coefficients_representable(integrator) && coefficients_stable(integrator))
        .then_some(integrator)
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
        attack,
        decay,
        release,
        sustain,
        rate,
        ..
    } = prepared
    else {
        return;
    };
    // ADR-0059: the velocity sensitivity, quantum-rate, one value per frame from its slot.
    let sensitivity = ramp_of(io.ramps, 0);
    // `P07-S002`: the times and the sustain level, quantum-rate slots read per frame in the
    // declaration's order. A time is converted where its segment starts; the sustain is
    // read where it is held.
    let (attacks, decays, sustains, releases) = (
        ramp_of(io.ramps, 1),
        ramp_of(io.ramps, 2),
        ramp_of(io.ramps, 3),
        ramp_of(io.ramps, 4),
    );
    let (attack, decay, release, rate) = (attack.as_f32(), decay.as_f32(), release.as_f32(), *rate);
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

    let authored_sustain = sustain.as_f32();
    let mut sustain = authored_sustain;
    let mut decay_frames = frames_of(decay, rate);
    // The envelope's own port table admits one channel, so a frame is a sample and an
    // offset indexes `out` directly. Deriving the frame from the channel count anyway
    // would be arithmetic defending against a layout this kind cannot be given.
    let mut due = 0_usize;
    for (frame, sample) in io.out.iter_mut().enumerate() {
        sustain = control_at(sustains, frame, authored_sustain);
        decay_frames = frames_of(control_at(decays, frame, decay), rate);
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
                ENVELOPE_GATE => run.gate(
                    control.value,
                    sustain,
                    frames_of(control_at(attacks, frame, attack), rate),
                    frames_of(control_at(releases, frame, release), rate),
                ),
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
        run.hand_over(sustain, decay_frames);
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
        // ADR-0059, V1's own arithmetic: `1 − s × (1 − v)`.
        let s = sensitivity
            .get(frame)
            .or(sensitivity.last())
            .copied()
            .unwrap_or(1.0);
        *sample = level * (1.0 - s * (1.0 - run.velocity.as_f32()));
    }
    // Settled before it is stored, so a quantum that ends exactly on a segment boundary
    // leaves the state on the segment that follows rather than on the exhausted one.
    run.hand_over(sustain, decay_frames);
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
    let PreparedNode::Filter {
        cutoff: authored_cutoff,
        resonance: authored_resonance,
        rate,
        ..
    } = prepared
    else {
        return;
    };
    let NodeState::Filter {
        band,
        low,
        cutoff,
        resonance,
        integrator,
    } = state
    else {
        return;
    };
    // `P07-S002`: the corner and the quality are quantum-rate slots read per frame. The
    // coefficients are re-derived only where the pair moves, so an unmodulated filter reads
    // the prepared ones throughout; a pair with no usable filter — a corner at or above
    // Nyquist, a quality not above zero, coefficients preparation would refuse — holds the
    // coefficients in force rather than rendering a filter nobody asked for.
    let (cutoffs, resonances) = (ramp_of(io.ramps, 0), ramp_of(io.ramps, 1));
    let (mut last_cutoff, mut last_resonance) = (*cutoff, *resonance);
    let mut coefficients = *integrator;
    let source = io.inputs[0];
    let (mut first, mut second) = (*band, *low);
    let mut due = 0_usize;
    for (index, sample) in io.out.iter_mut().enumerate() {
        // The filter declares no sample-positioned control of its own; the one control it
        // takes is the loop's reset (ADR-0058), which clears both integrators at the frame
        // and returns the pair and coefficients to the prepared ones as the reference the
        // slot's values are then compared against — a modulated pair re-derives at the same
        // frame, an unmodulated one reads the prepared coefficients on.
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != index {
                break;
            }
            due += 1;
            if matches!(control.control, ControlIndex::RESET)
                && let NodeState::Filter {
                    cutoff: prepared_cutoff,
                    resonance: prepared_resonance,
                    integrator: prepared_integrator,
                    ..
                } = NodeState::initial(prepared)
            {
                (first, second) = (0.0, 0.0);
                (last_cutoff, last_resonance) = (prepared_cutoff, prepared_resonance);
                coefficients = prepared_integrator;
            }
        }
        let wanted_cutoff = control_at(cutoffs, index, authored_cutoff.as_f32());
        let wanted_resonance = control_at(resonances, index, authored_resonance.as_f32());
        if wanted_cutoff != last_cutoff || wanted_resonance != last_resonance {
            (last_cutoff, last_resonance) = (wanted_cutoff, wanted_resonance);
            if let Some(next) =
                usable_low_pass(f64::from(wanted_cutoff), f64::from(wanted_resonance), *rate)
            {
                coefficients = next;
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
        let band_pass = coefficients[0] * first + coefficients[1] * drive;
        let low_pass = second + coefficients[1] * first + coefficients[2] * drive;
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
    (*cutoff, *resonance, *integrator) = (last_cutoff, last_resonance, coefficients);
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
    let [audio, control, ..] = io.inputs;
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

/// A mix channel (`SOUND-INV-031`): every frame scaled per side by the fader and V1's
/// constant-power pan, and silenced while the mute is held.
///
/// The pan law is V1's `Gain::from_pan`, computed here rather than called: the angle is
/// `(pan + 1) × π/4`, the left gain its cosine and the right its sine, so centre is
/// `cos(π/4)` per side and never unity. The per-side gain is formed as V1's channel stage
/// forms it — the pan coefficient times the fader, then the sample times that — so a test
/// oracle built from V1's own function matches bit for bit. The fader and the pan are read
/// per frame from their ramps; the mute is applied at the frame its control lands on, as
/// every sample-positioned control is. Channel `0` takes the left gain and every further
/// channel the right, which is correct for the one layout the port table admits.
pub fn channel(_prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let NodeState::Channel { muted } = state else {
        return;
    };
    let fader = ramp_of(io.ramps, 0);
    let pan = ramp_of(io.ramps, 1);
    let channels = io.channels.channels().max(1);
    let frames = io.out.len() / channels;
    let source = io.inputs[0];
    let mut held = *muted;
    let mut due = 0_usize;
    for frame in 0..frames {
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, CHANNEL_MUTE) {
                held = control.value.as_f32() > 0.0;
            }
        }
        let level = if held {
            0.0
        } else {
            fader.get(frame).or(fader.last()).copied().unwrap_or(1.0)
        };
        let position = pan.get(frame).or(pan.last()).copied().unwrap_or(0.0);
        let angle = (position + 1.0) * core::f32::consts::FRAC_PI_4;
        let left = angle.cos() * level;
        let right = angle.sin() * level;
        for channel in 0..channels {
            let index = frame * channels + channel;
            let input = match source {
                InputBuffer::Patched(source) => source.get(index).copied().unwrap_or(0.0),
                InputBuffer::InPlace => io.out.get(index).copied().unwrap_or(0.0),
                InputBuffer::Unpatched => 0.0,
            };
            let gain = if channel == 0 { left } else { right };
            if let Some(sample) = io.out.get_mut(index) {
                *sample = input * gain;
            }
        }
    }
    *muted = held;
}

/// An explicit sum's step (`SOUND-INV-031`): the region the compiler summed its cables
/// into, passed through unchanged — as the monitor passes its input — so the arena may give
/// the node that very region and the step writes nothing.
pub fn mix(_prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
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

/// A balance stage (`SOUND-INV-032`): every frame scaled per side by the level and V1's
/// **balance** law, and zeroed while the mute is held.
///
/// The law is V1's `apply_track_control`, term for term: the left gain is
/// `sqrt(1 − pan) × level` and the right `sqrt(1 + pan) × level`, so centre is unity and not
/// the constant-power `cos(π/4)` the mix channel forms — the two stages are two laws in V1
/// and two kinds here (`SOUND-INV-013`). The level and the pan are read per frame from their
/// ramps; the mute is applied at the frame its control lands on, and a muted frame is
/// written as zero, as V1 fills a muted voice. Channel `0` takes the left gain and every
/// further channel the right, which is correct for the one layout the port table admits.
pub fn balance(_prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let NodeState::Balance { muted } = state else {
        return;
    };
    let level = ramp_of(io.ramps, 0);
    let pan = ramp_of(io.ramps, 1);
    let channels = io.channels.channels().max(1);
    let frames = io.out.len() / channels;
    let source = io.inputs[0];
    let mut held = *muted;
    let mut due = 0_usize;
    for frame in 0..frames {
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, BALANCE_MUTE) {
                held = control.value.as_f32() > 0.0;
            }
        }
        let volume = level.get(frame).or(level.last()).copied().unwrap_or(1.0);
        let position = pan.get(frame).or(pan.last()).copied().unwrap_or(0.0);
        let left = (1.0 - position).sqrt() * volume;
        let right = (1.0 + position).sqrt() * volume;
        for channel in 0..channels {
            let index = frame * channels + channel;
            let input = match source {
                InputBuffer::Patched(source) => source.get(index).copied().unwrap_or(0.0),
                InputBuffer::InPlace => io.out.get(index).copied().unwrap_or(0.0),
                InputBuffer::Unpatched => 0.0,
            };
            let gain = if channel == 0 { left } else { right };
            if let Some(sample) = io.out.get_mut(index) {
                *sample = if held { 0.0 } else { input * gain };
            }
        }
    }
    *muted = held;
}

/// A trim (`SOUND-INV-032`): every sample times the level, read per frame from its ramp.
///
/// V1's master stage multiplies each side by the one master volume it read for the
/// callback; this is the same multiplication, at quantum grain.
pub fn trim(_prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
    scale_by_level(io);
}

/// A send (`SOUND-INV-034`): every sample times the level, read per frame from its ramp, and
/// zero while the mute is held.
///
/// V1's `apply_send_tap` multiplies the tapped signal by the send level it read for the
/// block, and taps nothing from a channel that is not audible; this is the same
/// multiplication at quantum grain, with the mute applied at the frame its control lands
/// on, as every sample-positioned control is.
pub fn send(_prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let NodeState::Send { muted } = state else {
        return;
    };
    let level = ramp_of(io.ramps, 0);
    let channels = io.channels.channels().max(1);
    let frames = io.out.len() / channels;
    let source = io.inputs[0];
    let mut held = *muted;
    let mut due = 0_usize;
    for frame in 0..frames {
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, SEND_MUTE) {
                held = control.value.as_f32() > 0.0;
            }
        }
        let gain = if held {
            0.0
        } else {
            level.get(frame).or(level.last()).copied().unwrap_or(1.0)
        };
        for channel in 0..channels {
            let index = frame * channels + channel;
            let input = match source {
                InputBuffer::Patched(source) => source.get(index).copied().unwrap_or(0.0),
                InputBuffer::InPlace => io.out.get(index).copied().unwrap_or(0.0),
                InputBuffer::Unpatched => 0.0,
            };
            if let Some(sample) = io.out.get_mut(index) {
                *sample = input * gain;
            }
        }
    }
    *muted = held;
}

/// V1's post-fader channel send (`SOUND-INV-034`): every frame scaled per side by the
/// channel's gain times the send level, formed in V1's order, and zero while the mute is
/// held.
///
/// The law is V1's `mix_channel_busses` and `apply_send_tap`, term for term: the pan
/// coefficient times the fader is the side's gain, that gain times the level is what the
/// sample is multiplied by. The three products round in that order, which is why this is a
/// kind of its own rather than a channel feeding a send. The fader, the pan and the level
/// are read per frame from their ramps, the level from the third; the mute is applied at
/// the frame its control lands on. Channel `0` takes the left gain and every further
/// channel the right, which is correct for the one layout the port table admits.
pub fn post_fader_send(_prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let NodeState::PostFaderSend { muted } = state else {
        return;
    };
    let fader = ramp_of(io.ramps, 0);
    let pan = ramp_of(io.ramps, 1);
    let level = ramp_of(io.ramps, 2);
    let channels = io.channels.channels().max(1);
    let frames = io.out.len() / channels;
    let source = io.inputs[0];
    let mut held = *muted;
    let mut due = 0_usize;
    for frame in 0..frames {
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, POST_FADER_SEND_MUTE) {
                held = control.value.as_f32() > 0.0;
            }
        }
        let volume = fader.get(frame).or(fader.last()).copied().unwrap_or(1.0);
        let position = pan.get(frame).or(pan.last()).copied().unwrap_or(0.0);
        let amount = level.get(frame).or(level.last()).copied().unwrap_or(1.0);
        let angle = (position + 1.0) * core::f32::consts::FRAC_PI_4;
        let left_gain = angle.cos() * volume;
        let right_gain = angle.sin() * volume;
        let (left, right) = if held {
            (0.0, 0.0)
        } else {
            (left_gain * amount, right_gain * amount)
        };
        for channel in 0..channels {
            let index = frame * channels + channel;
            let input = match source {
                InputBuffer::Patched(source) => source.get(index).copied().unwrap_or(0.0),
                InputBuffer::InPlace => io.out.get(index).copied().unwrap_or(0.0),
                InputBuffer::Unpatched => 0.0,
            };
            let gain = if channel == 0 { left } else { right };
            if let Some(sample) = io.out.get_mut(index) {
                *sample = input * gain;
            }
        }
    }
    *muted = held;
}

/// Every sample times the node's one quantum-rate level, read per frame from its ramp.
fn scale_by_level(io: &mut NodeIo<'_>) {
    let level = ramp_of(io.ramps, 0);
    let channels = io.channels.channels().max(1);
    let frames = io.out.len() / channels;
    let source = io.inputs[0];
    for frame in 0..frames {
        let gain = level.get(frame).or(level.last()).copied().unwrap_or(1.0);
        for channel in 0..channels {
            let index = frame * channels + channel;
            let input = match source {
                InputBuffer::Patched(source) => source.get(index).copied().unwrap_or(0.0),
                InputBuffer::InPlace => io.out.get(index).copied().unwrap_or(0.0),
                InputBuffer::Unpatched => 0.0,
            };
            if let Some(sample) = io.out.get_mut(index) {
                *sample = input * gain;
            }
        }
    }
}

/// The knee of V1's soft clipper: a sample within it passes unchanged.
const SOFT_CLIP_THRESHOLD: f32 = 0.8;

/// V1's `soft_clip`, term for term (`SOUND-INV-032`): identity to the knee, and above it
/// `knee + headroom × (1 − e^(−excess / headroom))` with the sample's sign, where the
/// headroom is what remains to full scale. Computed here rather than called
/// (`SOUND-INV-013`) and held to V1's own function by a bit-for-bit test.
fn soft_clip_law(sample: f32) -> f32 {
    if sample.abs() <= SOFT_CLIP_THRESHOLD {
        return sample;
    }
    let abs_sample = sample.abs();
    let excess = abs_sample - SOFT_CLIP_THRESHOLD;
    let headroom = 1.0 - SOFT_CLIP_THRESHOLD;
    let compressed = SOFT_CLIP_THRESHOLD + headroom * (1.0 - (-excess / headroom).exp());
    if sample < 0.0 {
        -compressed
    } else {
        compressed
    }
}

/// V1's soft clipper as a node (`SOUND-INV-032`): every sample through V1's law.
pub fn soft_clip(_prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
    let source = io.inputs[0];
    for index in 0..io.out.len() {
        let input = match source {
            InputBuffer::Patched(source) => source.get(index).copied().unwrap_or(0.0),
            InputBuffer::InPlace => io.out.get(index).copied().unwrap_or(0.0),
            InputBuffer::Unpatched => 0.0,
        };
        if let Some(sample) = io.out.get_mut(index) {
            *sample = soft_clip_law(input);
        }
    }
}

/// V1's output clamp as a node (`SOUND-INV-032`): every sample held to `[−1, 1]`, as V1's
/// output stage holds each side after its master volume.
pub fn hard_clamp(_prepared: &PreparedNode, _state: &mut NodeState, io: &mut NodeIo<'_>) {
    let source = io.inputs[0];
    for index in 0..io.out.len() {
        let input = match source {
            InputBuffer::Patched(source) => source.get(index).copied().unwrap_or(0.0),
            InputBuffer::InPlace => io.out.get(index).copied().unwrap_or(0.0),
            InputBuffer::Unpatched => 0.0,
        };
        if let Some(sample) = io.out.get_mut(index) {
            *sample = input.clamp(-1.0, 1.0);
        }
    }
}

/// V1's line length per delay side, in seconds (`SOUND-INV-033`).
pub const MAX_DELAY_SECONDS: f32 = 2.0;

/// V1's drive law's scale: the drive squared times this, plus one, is the gain.
const DRIVE_SCALE: f32 = 50.0;

/// V1's rational `tanh` (`fast_tanh`), term for term: clamped past `±3`, and the Padé form
/// `x (27 + x²) / (27 + 9 x²)` within. The distortion's law, and not the delay's, which
/// limits its feedback write with `f32::tanh` — the two are kept distinct so each kernel's
/// bits are V1's (`SOUND-INV-013`).
fn rational_tanh(x: f32) -> f32 {
    if x < -3.0 {
        return -1.0;
    }
    if x > 3.0 {
        return 1.0;
    }
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// V1's one-pole coefficient for a corner at `hertz`: `e^(−2π f / rate)`, in `f32` as V1's
/// `Hertz::to_exp_coeff` forms it.
fn exp_coefficient(hertz: f32, rate: f32) -> f32 {
    (-core::f32::consts::TAU * hertz / rate).exp()
}

/// The distortion's tone coefficient at `tone`: V1's corner `200 + tone² × 15000` Hz through
/// [`exp_coefficient`]. Public to the crate so the declaration's timing states the decay
/// the kernel renders.
pub(crate) fn distortion_tone_coefficient(tone: f32, rate: f32) -> f32 {
    exp_coefficient(200.0 + tone * tone * 15_000.0, rate)
}

/// The delay's feedback high-cut coefficient at `tone`: V1's corner `200 + tone × 19800` Hz,
/// spelled as V1 spells it, through [`exp_coefficient`].
pub(crate) fn delay_high_cut_coefficient(tone: f32, rate: f32) -> f32 {
    exp_coefficient(200.0 + tone * (20_000.0 - 200.0), rate)
}

/// One input sample of a stereo stage, from whichever of the three input states the arena
/// bound: a distinct region, the output's own slot, or nothing.
fn stage_input(source: InputBuffer<'_>, out: &[f32], index: usize) -> f32 {
    match source {
        InputBuffer::Patched(source) => source.get(index).copied().unwrap_or(0.0),
        InputBuffer::InPlace => out.get(index).copied().unwrap_or(0.0),
        InputBuffer::Unpatched => 0.0,
    }
}

/// V1's distortion insert in its soft-clip mode as a node (`SOUND-INV-033`), term for term:
/// per sample, the input times `1 + drive² × 50`, through V1's rational `tanh`, through the
/// side's one-pole tone filter as V1 writes it — `y × (1 − coef) + state × coef` — and
/// blended with the dry sample as `dry × (1 − mix) + wet × mix`. The coefficient is
/// re-derived only where the tone moves; a sample-positioned reset clears the filters.
pub fn distortion(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Distortion {
        drive: authored_drive,
        tone: authored_tone,
        mix: authored_mix,
        rate,
    } = prepared
    else {
        return;
    };
    let NodeState::Distortion {
        filters,
        tone,
        coef,
    } = state
    else {
        return;
    };
    let (drives, tones, mixes) = (
        ramp_of(io.ramps, 0),
        ramp_of(io.ramps, 1),
        ramp_of(io.ramps, 2),
    );
    let channels = io.channels.channels().max(1);
    let frames = io.out.len() / channels;
    let source = io.inputs[0];
    let mut states = *filters;
    let (mut last_tone, mut coefficient) = (*tone, *coef);
    let mut due = 0_usize;
    for frame in 0..frames {
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, ControlIndex::RESET) {
                states = [0.0; 2];
            }
        }
        let drive = control_at(drives, frame, authored_drive.as_f32());
        let wanted_tone = control_at(tones, frame, authored_tone.as_f32());
        let mix = control_at(mixes, frame, authored_mix.as_f32());
        if wanted_tone != last_tone {
            last_tone = wanted_tone;
            coefficient = distortion_tone_coefficient(wanted_tone, *rate);
        }
        let gain = 1.0 + drive * drive * DRIVE_SCALE;
        for channel in 0..channels {
            let index = frame * channels + channel;
            let dry = stage_input(source, io.out, index);
            let shaped = rational_tanh(dry * gain);
            let filtered = match states.get_mut(channel.min(1)) {
                Some(filter) => {
                    *filter = shaped * (1.0 - coefficient) + *filter * coefficient;
                    *filter
                }
                None => shaped,
            };
            if let Some(sample) = io.out.get_mut(index) {
                *sample = dry * (1.0 - mix) + filtered * mix;
            }
        }
    }
    *filters = states;
    *tone = last_tone;
    *coef = coefficient;
}

/// V1's two-tap read of a delay line, term for term (`BufferIndex::read_interpolated`): the
/// read position `write − delay` wrapped into the line in `f32`, its floor and the frame
/// after as the two taps, and their weighted sum by the fraction.
fn read_delayed(line: &[f32], write: usize, delay: f32) -> f32 {
    if line.is_empty() {
        return 0.0;
    }
    let len = line.len();
    let read_pos = (write as f32 - delay).rem_euclid(len as f32);
    let idx0 = (read_pos as usize) % len;
    let idx1 = (idx0 + 1) % len;
    let frac = read_pos - read_pos.floor();
    line.get(idx0).copied().unwrap_or(0.0) * (1.0 - frac)
        + line.get(idx1).copied().unwrap_or(0.0) * frac
}

/// V1's one-pole as `FilterState::one_pole` writes it: `input + (state − input) × coef`.
/// Not the distortion's form, which V1 spells differently; the two round apart.
fn feedback_low_pass(state: &mut f32, input: f32, coef: f32) -> f32 {
    *state = input + (*state - input) * coef;
    *state
}

/// V1's delay insert in its mono mode as a node (`SOUND-INV-033`), term for term.
///
/// Per frame: each side is read from its own line at its own time — the time in frames
/// held to the line's last frame, as V1 holds it — and that read is the side's wet sample;
/// each read is filtered by the feedback high cut; the two dry samples and the two filtered
/// reads are each averaged to mono, the feedback applied, the sum limited by `f32::tanh`
/// and written into **both** lines at the write index; the index advances; and each side's
/// output is its dry sample blended with its own **unfiltered** read by the mix. The two
/// lines are the halves of the history the renderer keeps for this step. A
/// sample-positioned reset — a stolen voice's, where the node runs in the voice scope —
/// zeroes both lines, both filters and the index; an instrument-scope delay is not in any
/// instance group and never receives one.
pub fn delay(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Delay {
        time_left: authored_left,
        time_right: authored_right,
        feedback: authored_feedback,
        mix: authored_mix,
        tone: authored_tone,
        rate,
        line,
    } = prepared
    else {
        return;
    };
    let NodeState::Delay {
        write,
        filters,
        tone,
        coef,
    } = state
    else {
        return;
    };
    let len = *line;
    let channels = io.channels.channels().max(1);
    let frames = io.out.len() / channels;
    let source = io.inputs[0];
    let Some((left_line, rest)) = io.history.split_at_mut_checked(len) else {
        return;
    };
    let Some(right_line) = rest.get_mut(..len) else {
        return;
    };
    if len == 0 {
        return;
    }
    let (lefts, rights, feedbacks, mixes, tones) = (
        ramp_of(io.ramps, 0),
        ramp_of(io.ramps, 1),
        ramp_of(io.ramps, 2),
        ramp_of(io.ramps, 3),
        ramp_of(io.ramps, 4),
    );
    let last = (len - 1) as f32;
    let mut position = *write;
    let mut states = *filters;
    let (mut last_tone, mut coefficient) = (*tone, *coef);
    let mut due = 0_usize;
    for frame in 0..frames {
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, ControlIndex::RESET) {
                left_line.fill(0.0);
                right_line.fill(0.0);
                states = [0.0; 2];
                position = 0;
            }
        }
        let time_left = control_at(lefts, frame, authored_left.as_f32());
        let time_right = control_at(rights, frame, authored_right.as_f32());
        let feedback = control_at(feedbacks, frame, authored_feedback.as_f32());
        let mix = control_at(mixes, frame, authored_mix.as_f32());
        let wanted_tone = control_at(tones, frame, authored_tone.as_f32());
        if wanted_tone != last_tone {
            last_tone = wanted_tone;
            coefficient = delay_high_cut_coefficient(wanted_tone, *rate);
        }
        let delay_left = (time_left * *rate).min(last);
        let delay_right = (time_right * *rate).min(last);
        let index = frame * channels;
        let dry_left = stage_input(source, io.out, index);
        let dry_right = if channels > 1 {
            stage_input(source, io.out, index + 1)
        } else {
            dry_left
        };
        let delayed_left = read_delayed(left_line, position, delay_left);
        let delayed_right = read_delayed(right_line, position, delay_right);
        let (fed_left, fed_right) = (
            feedback_low_pass(&mut states[0], delayed_left, coefficient),
            feedback_low_pass(&mut states[1], delayed_right, coefficient),
        );
        let mono_in = (dry_left + dry_right) * 0.5;
        let mono_fed = (fed_left + fed_right) * 0.5;
        let written = (mono_in + mono_fed * feedback).tanh();
        if let Some(slot) = left_line.get_mut(position) {
            *slot = written;
        }
        if let Some(slot) = right_line.get_mut(position) {
            *slot = written;
        }
        position = (position + 1) % len;
        let dry_amount = 1.0 - mix;
        if let Some(sample) = io.out.get_mut(index) {
            *sample = dry_left * dry_amount + delayed_left * mix;
        }
        if channels > 1
            && let Some(sample) = io.out.get_mut(index + 1)
        {
            *sample = dry_right * dry_amount + delayed_right * mix;
        }
    }
    *write = position;
    *filters = states;
    *tone = last_tone;
    *coef = coefficient;
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

/// V1's voice-output velocity stage (ADR-0059): every sample scaled by `(1 − s) + s × v`,
/// `v` the velocity its own destination last received — held, as the envelope holds its —
/// and `s` its sensitivity, read per frame from its slot's ramp. In place, like a gain.
pub fn velocity_scaler(_prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let NodeState::Scaled { velocity } = state else {
        return;
    };
    let sensitivity = ramp_of(io.ramps, 0);
    let source = io.inputs[0];
    let mut held = *velocity;
    let mut due = 0_usize;
    for frame in 0..io.out.len() {
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            if matches!(control.control, VELOCITY_SCALER_VELOCITY) {
                held = NoteVelocity::saturating(control.value.as_f32());
            }
        }
        let s = sensitivity
            .get(frame)
            .or(sensitivity.last())
            .copied()
            .unwrap_or(1.0);
        let scale = (1.0 - s) + s * held.as_f32();
        let input = match source {
            InputBuffer::Patched(source) => source.get(frame).copied().unwrap_or(0.0),
            InputBuffer::InPlace => io.out.get(frame).copied().unwrap_or(0.0),
            InputBuffer::Unpatched => 0.0,
        };
        if let Some(sample) = io.out.get_mut(frame) {
            *sample = input * scale;
        }
    }
    *velocity = held;
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
    //
    // The source is either mono, widened into every channel, or already the output's own
    // layout — a stereo instance output seeding its voice sum (`P08-S002`) — and copied
    // verbatim. Decided from the lengths the arena bound rather than assumed mono: before
    // this, a stereo voice-scope output was read as twice as many mono frames and its sum
    // came out interleaved wrongly, which a probe of a two-voice stereo script showed.
    let channels = io.channels.channels().max(1);
    let source_channels = if source.len() == io.out.len() {
        channels
    } else {
        1
    };
    let frames = source.len() / source_channels;
    let fading = matches!(state, NodeState::Sum { fade_total, .. } if *fade_total != 0)
        || !io.controls.is_empty();
    if !fading {
        for frame in 0..frames {
            for channel in 0..channels {
                let read = frame * source_channels + channel.min(source_channels - 1);
                let input = source.get(read).copied().unwrap_or(0.0);
                if let Some(sample) = io.out.get_mut(frame * channels + channel) {
                    *sample = input;
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
    for frame in 0..frames {
        fade.take(io.controls, &mut due, frame);
        for channel in 0..channels {
            let read = frame * source_channels + channel.min(source_channels - 1);
            let input = source.get(read).copied().unwrap_or(0.0);
            if let Some(sample) = io.out.get_mut(frame * channels + channel) {
                *sample = fade.gain() * input;
            }
        }
    }
    (*fade_remaining, *fade_total) = (fade.remaining, fade.total);
}

/// A one-zone sampler — ADR-0026 clause 7, V1's playback law.
///
/// Per frame: every control due at the frame is applied first, as every kernel does; then
/// one frame is read from the zone's region at the fractional position, two-tap linear with
/// the second tap wrapped into the loop's start at the loop's end and clamped at the
/// region's end, a stereo sample summed to mono as `(left + right) × 0.5`; then the zone's
/// gain, the level ramp, the velocity under `(1 − s) + s × v` and the fade are applied, and
/// the position advances by the rate. The rate was set by the pitch destination as the
/// resolved frequency over the root's, times the fine-tune factor — V1's `set_voice_pitch`.
///
/// The on edge starts the read at the region's start, or at V1's `start_offset` into it
/// when the offset is above `0.001`; the off edge starts the fade in `Sustain` and `Loop`
/// mode and is ignored in `OneShot`. `Loop` repeats the loop region while the read
/// continues, fade included, as V1's player keeps looping through its release. A reset
/// (ADR-0058) silences it. A sample the slot does not resolve renders silence: the audio
/// thread has no honest substitute for the frames a plan was admitted with.
pub fn sampler(prepared: &PreparedNode, state: &mut NodeState, io: &mut NodeIo<'_>) {
    let PreparedNode::Sampler {
        sample,
        root_frequency,
        fine_factor,
        region,
        loop_region,
        gain,
        start_offset,
        play_mode,
        ..
    } = prepared
    else {
        return;
    };
    let NodeState::Sampler {
        position,
        rate,
        velocity,
        playback,
        fade_remaining,
        held,
    } = state
    else {
        return;
    };
    let level = ramp_of(io.ramps, 0);
    let sensitivity = ramp_of(io.ramps, 1);
    let Some(audio) = io.samples.get(sample.index()) else {
        io.out.fill(0.0);
        return;
    };
    let width = audio.channels.channels();
    let frames = &audio.frames;
    let start = region.start().as_index();
    let end = region.end().as_index();
    let (loop_start, loop_end) = match loop_region {
        Some(region) => (region.start().as_index(), region.end().as_index()),
        None => (start, end),
    };
    let looping = matches!(play_mode, PlayMode::Loop) && loop_region.is_some();
    let root = f64::from(root_frequency.as_f32());
    let fade_total = SUSTAIN_FADE_FRAMES as f32;
    let gain = gain.as_f32();
    let offset = start_offset.as_f32();

    let mut run = SamplerRun {
        position: *position,
        rate: *rate,
        velocity: *velocity,
        playback: *playback,
        fade_remaining: *fade_remaining,
        held: *held,
    };
    let mut due = 0_usize;
    for (frame, sample_out) in io.out.iter_mut().enumerate() {
        while let Some(control) = io.controls.get(due) {
            if control.offset.as_usize() != frame {
                break;
            }
            due += 1;
            match control.control {
                SAMPLER_TRIGGER => {
                    let raised = control.value.as_f32() > 0.0;
                    if raised && !run.held {
                        // V1 seeks into the crop only above a threshold, and so does this.
                        let span = (end - start) as f64;
                        run.position = if offset > 0.001 {
                            start as f64 + span * f64::from(offset)
                        } else {
                            start as f64
                        };
                        run.playback = Playback::Playing;
                        run.fade_remaining = 0;
                    } else if !raised && run.held && run.playback == Playback::Playing {
                        match play_mode {
                            PlayMode::OneShot => {}
                            PlayMode::Sustain | PlayMode::Loop => {
                                run.playback = Playback::Fading;
                                run.fade_remaining = SUSTAIN_FADE_FRAMES;
                            }
                        }
                    }
                    run.held = raised;
                }
                SAMPLER_PITCH => {
                    let hz = f64::from(control.value.as_f32());
                    run.rate = if root > 0.0 {
                        hz / root * fine_factor
                    } else {
                        0.0
                    };
                }
                SAMPLER_VELOCITY => {
                    run.velocity = NoteVelocity::saturating(control.value.as_f32());
                }
                ControlIndex::RESET => {
                    run = SamplerRun {
                        position: 0.0,
                        rate: 0.0,
                        velocity: NoteVelocity::FULL,
                        playback: Playback::Idle,
                        fade_remaining: 0,
                        held: false,
                    };
                }
                _ => {}
            }
        }

        let value = if run.playback == Playback::Idle {
            0.0
        } else {
            let whole = run.position.floor();
            let frac = (run.position - whole) as f32;
            let index = whole as usize;
            let mut next = index + 1;
            if looping && next >= loop_end {
                next = loop_start;
            } else if next >= end {
                next = index;
            }
            let mut read = 0.0_f32;
            for channel in 0..width {
                let s0 = frames.get(index * width + channel).copied().unwrap_or(0.0);
                let s1 = frames.get(next * width + channel).copied().unwrap_or(0.0);
                read += s0 + (s1 - s0) * frac;
            }
            if width > 1 {
                read *= 0.5;
            }
            let fade = if run.playback == Playback::Fading {
                run.fade_remaining as f32 / fade_total
            } else {
                1.0
            };
            let l = level.get(frame).or(level.last()).copied().unwrap_or(1.0);
            let s = sensitivity
                .get(frame)
                .or(sensitivity.last())
                .copied()
                .unwrap_or(1.0);
            let scale = (1.0 - s) + s * run.velocity.as_f32();
            read * gain * l * scale * fade
        };
        *sample_out = value;

        if run.playback != Playback::Idle {
            run.position += run.rate;
            if looping {
                let span = (loop_end - loop_start) as f64;
                while run.position >= loop_end as f64 {
                    run.position -= span;
                }
            } else if run.position >= end as f64 {
                run.playback = Playback::Idle;
            }
            if run.playback == Playback::Fading {
                run.fade_remaining = run.fade_remaining.saturating_sub(1);
                if run.fade_remaining == 0 {
                    run.playback = Playback::Idle;
                }
            }
        }
    }

    (
        *position,
        *rate,
        *velocity,
        *playback,
        *fade_remaining,
        *held,
    ) = (
        run.position,
        run.rate,
        run.velocity,
        run.playback,
        run.fade_remaining,
        run.held,
    );
}

/// A sampler's state for the duration of one quantum, written back at its end.
struct SamplerRun {
    position: f64,
    rate: f64,
    velocity: NoteVelocity,
    playback: Playback,
    fade_remaining: u32,
    held: bool,
}

/// Emit the centrally resolved source value. No controller or note composition in kernels.
pub fn control_source(_: &PreparedNode, _: &mut NodeState, io: &mut NodeIo<'_>) {
    let values = ramp_of(io.ramps, 0);
    for (frame, sample) in io.out.iter_mut().enumerate() {
        *sample = values.get(frame).copied().unwrap_or(0.0);
    }
}
/// The source's sole quantum-rate control.
pub const SOURCE_VALUE: ControlIndex = ControlIndex::new(0);
/// Stateless control-source kernel.
pub const CONTROL_SOURCE: Kernel = Kernel(control_source);

/// What a kernel borrows beside the arena: the plan's immutable tables, and the one mutable
/// thing that is neither arena nor state — the step's history (`SOUND-INV-033`).
#[derive(Debug, Default)]
pub struct NodeResources<'a> {
    pub samples: &'a [PreparedSample],
    pub scripts: crate::script::ScriptResources<'a>,
    /// The step's slice of the renderer's history slab, empty for a kind that keeps none.
    pub history: &'a mut [f32],
}
