//! The node registry: what a node kind declares, and which kernel runs it.
//!
//! [ADR-0004](../../plans/v2/decisions/ADR-0004-native-node-representation.md) selects a
//! **closed kernel registry dispatched through a prepared function table**, and this is
//! the table. Everything a node kind says about itself lives here — its ports, its
//! controls, the prepared data admission builds for it, and the kernel that renders it —
//! so that adding a node kind is a descriptor and a kernel rather than an edit to the
//! compiler, the arena, the validator, or the render loop.
//!
//! # The split, and which side of it this file is on
//!
//! This file runs **at admission only**. It allocates, it reads [`IrNodeKind`], and it
//! knows about the profile. [`kernels`] is the other side: the per-quantum work, scanned
//! by `tests/render_loop_purity.rs` together with the render loop, and holding nothing a
//! compiler would need. The registry below is the seam, and it is resolved once — the
//! renderer is handed function pointers and slots.

pub mod kernels;

use crate::diagnostics::{CompileError, PreparationFault};
use crate::ir::{IrNodeKind, NodeId, ParameterId, SignalDomain, parameters};
use crate::plan::ControlRate;
use crate::quantities::{
    ChannelLayout, CutoffFrequency, NormalizedLevel, Resonance, SampleRate, Seconds, SegmentFrames,
};
use crate::validate::{PortDirection, PortSpec};
use kernels::{ControlIndex, Kernel, PreparedNode};

/// Which magnitude of a note payload a control is the destination of.
///
/// `SOUND-INV-021`: a note-on carries exactly two magnitudes beside its gate, and a node
/// **kind** declares which of its controls each one lands on. A producer names a node and
/// never a destination, so this is what lets a key reach an oscillator that is not the node
/// the note was sent to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteMagnitude {
    /// The key identity, resolved to a frequency through the node's prepared tuning.
    Pitch,
    /// The velocity, as the normalized magnitude it was validated as.
    Velocity,
}

impl std::fmt::Display for NoteMagnitude {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pitch => f.write_str("pitch"),
            Self::Velocity => f.write_str("velocity"),
        }
    }
}

/// One control a node kind exposes, and the parameter identity that addresses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(crate) struct ControlSpec {
    /// What a caller names.
    pub(crate) parameter: ParameterId,
    /// What the node's state calls it.
    pub(crate) control: ControlIndex,
    /// When moving it takes effect, per ADR-0001 clause 14.
    ///
    /// Declared by the kind rather than chosen by the caller: the clause splits on the
    /// *effect*, so a gate is sample-positioned whichever payload addressed it.
    pub(crate) rate: ControlRate,
    /// Which note magnitude this control is the destination of, where it is one.
    ///
    /// `SOUND-INV-021`'s declaration, and it lives beside the rate rather than in a second
    /// list because the two are one statement: a magnitude has to be in force at the sample
    /// its note's gate rises, so a destination that is not [`ControlRate::Sample`] cannot
    /// carry one. `descriptor_destinations_are_sample_positioned` is what holds the pair
    /// together.
    pub(crate) magnitude: Option<NoteMagnitude>,
}

/// Everything the compiler needs to know about a node kind.
///
/// Read at admission and nowhere else: the plan carries the kernel pointer and the
/// prepared data, and the render loop carries neither this nor the kind it came from.
#[derive(Debug, Clone)]
#[must_use]
pub(crate) struct NodeDescriptor {
    /// The kernel that renders it.
    pub(crate) kernel: Kernel,
    /// The ports it declares, given the stream the plan renders into.
    pub(crate) ports: Vec<PortSpec>,
    /// The controls a parameter event can move.
    pub(crate) controls: Vec<ControlSpec>,
    /// Whether writing its output over its first input changes its result.
    ///
    /// ADR-0005 clause 5's *first* condition. The arena's second condition — that the
    /// input is not read again — is not a property of the node and stays there.
    pub(crate) in_place_safe: bool,
    /// The control a note edge moves, where the kind can be played at all.
    ///
    /// `None` for every kind that is not playable, which is what makes
    /// [`crate::plan::CompiledPlan::resolve_note`] able to refuse a node a caller cannot
    /// send a note to instead of accepting an event that would do nothing.
    pub(crate) note_control: Option<ControlIndex>,
}

/// An amplifier's control port, which is its second input.
pub const AMPLIFIER_CONTROL: crate::ir::PortId = crate::ir::PortId::new(1);

/// The audio output every source declares.
const AUDIO_OUT: PortSpec = PortSpec::new(
    crate::ir::PortId::FIRST,
    PortDirection::Output,
    SignalDomain::Audio,
    ChannelLayout::Mono,
);

/// The audio output every source declares, for the arms not yet moved to a declaration.
fn audio_out() -> PortSpec {
    AUDIO_OUT
}

/// A declared kind's preparation: its prepared record from its IR fields and the rate.
pub(crate) type PrepareFn =
    fn(NodeId, IrNodeKind, SampleRate) -> Result<PreparedNode, CompileError>;

/// What a declaration answers when handed a kind that is not its own.
fn declared_for_another_kind(node: NodeId) -> CompileError {
    CompileError::DeclaredForAnotherKind { node }
}

/// A frame as a fraction of a second, so a frequency becomes a phase step by one
/// multiply instead of a divide per quantum.
fn seconds_per_frame(rate: SampleRate) -> f64 {
    1.0 / f64::from(rate.as_f32())
}

fn prepare_silence(
    node: NodeId,
    kind: IrNodeKind,
    _: SampleRate,
) -> Result<PreparedNode, CompileError> {
    let IrNodeKind::Silence = kind else {
        return Err(declared_for_another_kind(node));
    };
    Ok(PreparedNode::Silence)
}

fn prepare_constant(
    node: NodeId,
    kind: IrNodeKind,
    _: SampleRate,
) -> Result<PreparedNode, CompileError> {
    let IrNodeKind::Constant { level } = kind else {
        return Err(declared_for_another_kind(node));
    };
    Ok(PreparedNode::Constant { level })
}

fn prepare_impulse(
    node: NodeId,
    kind: IrNodeKind,
    _: SampleRate,
) -> Result<PreparedNode, CompileError> {
    let IrNodeKind::Impulse { position } = kind else {
        return Err(declared_for_another_kind(node));
    };
    Ok(PreparedNode::Impulse { position })
}

fn prepare_sine(
    node: NodeId,
    kind: IrNodeKind,
    rate: SampleRate,
) -> Result<PreparedNode, CompileError> {
    let IrNodeKind::Sine {
        frequency,
        amplitude,
    } = kind
    else {
        return Err(declared_for_another_kind(node));
    };
    Ok(PreparedNode::Sine {
        seconds_per_frame: seconds_per_frame(rate),
        frequency,
        amplitude,
    })
}

fn prepare_saw(
    node: NodeId,
    kind: IrNodeKind,
    rate: SampleRate,
) -> Result<PreparedNode, CompileError> {
    let IrNodeKind::Saw {
        frequency,
        amplitude,
    } = kind
    else {
        return Err(declared_for_another_kind(node));
    };
    Ok(PreparedNode::Saw {
        seconds_per_frame: seconds_per_frame(rate),
        frequency,
        amplitude,
    })
}

fn prepare_gain(
    node: NodeId,
    kind: IrNodeKind,
    _: SampleRate,
) -> Result<PreparedNode, CompileError> {
    let IrNodeKind::Gain { factor } = kind else {
        return Err(declared_for_another_kind(node));
    };
    Ok(PreparedNode::Gain { factor })
}

fn prepare_amplifier(
    node: NodeId,
    kind: IrNodeKind,
    _: SampleRate,
) -> Result<PreparedNode, CompileError> {
    let IrNodeKind::Amplifier = kind else {
        return Err(declared_for_another_kind(node));
    };
    Ok(PreparedNode::Amplifier)
}

fn prepare_filter(
    node: NodeId,
    kind: IrNodeKind,
    rate: SampleRate,
) -> Result<PreparedNode, CompileError> {
    let IrNodeKind::Filter { cutoff, resonance } = kind else {
        return Err(declared_for_another_kind(node));
    };
    low_pass(node, cutoff, resonance, rate)
}

fn prepare_envelope(
    node: NodeId,
    kind: IrNodeKind,
    rate: SampleRate,
) -> Result<PreparedNode, CompileError> {
    let IrNodeKind::Envelope {
        attack,
        decay,
        sustain,
        release,
    } = kind
    else {
        return Err(declared_for_another_kind(node));
    };
    // Each segment as the frames it lasts. The level a segment moves *through* is not
    // prepared, because it is not known until the segment starts: a note let go during
    // its attack releases from wherever it had reached.
    let mut frames = [SegmentFrames::NONE; 3];
    for (slot, duration) in frames.iter_mut().zip([attack, decay, release]) {
        match frames_in(duration, rate) {
            Some(count) => *slot = count,
            None => {
                return Err(CompileError::NodeNotPreparable {
                    node,
                    fault: PreparationFault::SegmentTooLong {
                        duration,
                        limit: u32::MAX,
                    },
                });
            }
        }
    }
    Ok(PreparedNode::Envelope {
        attack_frames: frames[0],
        decay_frames: frames[1],
        release_frames: frames[2],
        sustain,
    })
}

/// What one node kind declares about itself, in one place.
///
/// Phase 5's first slice, `P05-S001`, and the shape every later kind moves to. Before it,
/// what a kind said about itself was spread over one `match` arm per registry function —
/// its descriptor, its prepared and mutable byte attribution — so a kind's facts could
/// disagree with each other and nothing asked. A declaration is one value the registry
/// functions **derive** from through [`declaration`], so the facts cannot disagree. The
/// descriptor and the two byte attributions still match exhaustively over every kind, so
/// a declared kind keeps an arm in each — but that arm **defers** to the declaration and
/// states nothing, and `tests/node_representation.rs` holds every arm of a declared kind
/// to that form; [`prepare`] has no arm at all and forwards to the declaration's entry.
///
/// Preparation is here too, as `P05-S005` moved it: each declaration names the function
/// that builds its prepared record from its own IR fields, so [`prepare`] has no arm per
/// kind and the four registry functions all derive from one value. The IR still carries
/// a kind's stored base as typed variant fields; when parameters become slots the
/// preparation reads them there, and the declaration's entry stays where it is.
#[derive(Debug)]
#[must_use]
pub(crate) struct NodeDeclaration {
    /// The kernel that renders it.
    pub(crate) kernel: Kernel,
    /// The ports it declares. Stream-independent for every kind but the output node,
    /// which has no declaration because it has no kernel.
    pub(crate) ports: &'static [PortSpec],
    /// The controls a parameter event can move, with the note magnitude each carries.
    pub(crate) controls: &'static [ControlSpec],
    /// ADR-0005 clause 5's first condition, as [`NodeDescriptor::in_place_safe`].
    pub(crate) in_place_safe: bool,
    /// The control a note edge moves, where the kind can be played at all.
    pub(crate) note_control: Option<ControlIndex>,
    /// How this kind's prepared data is built from its IR fields, against the stream's
    /// rate — the master plan's *off-thread preparation*, as the kind's own entry.
    ///
    /// Takes the kind by value and reads its own variant's fields — a fieldless kind still
    /// matches its variant — and a kind that is not this declaration's is refused as
    /// [`CompileError::DeclaredForAnotherKind`], which [`declaration`]'s pairing makes
    /// unreachable and a test holds it to.
    pub(crate) prepare: PrepareFn,
    /// The immutable payload this kind is charged for, per node, in the resource report.
    pub(crate) prepared_bytes: u64,
    /// The mutable payload this kind is charged for, per node, in the resource report.
    pub(crate) state_bytes: u64,
}

impl NodeDeclaration {
    /// The descriptor admission reads, derived rather than restated.
    fn descriptor(&self) -> NodeDescriptor {
        NodeDescriptor {
            kernel: self.kernel,
            ports: self.ports.to_vec(),
            controls: self.controls.to_vec(),
            in_place_safe: self.in_place_safe,
            note_control: self.note_control,
        }
    }
}

/// The sawtooth, declared once.
///
/// A pitch destination on its frequency, as the sine's is and for the same reason: a
/// note's key has to be in force at the sample its gate rises, so the destination is
/// sample-positioned. The byte attributions name the kernel's prepared and state layouts
/// — a phase accumulator beside its frequency and amplitude — as the report charges them.
pub(crate) const SAW: NodeDeclaration = NodeDeclaration {
    kernel: kernels::SAW,
    ports: &[AUDIO_OUT],
    controls: &[
        ControlSpec {
            parameter: parameters::SAW_FREQUENCY,
            control: kernels::SAW_FREQUENCY,
            rate: ControlRate::Sample,
            magnitude: Some(NoteMagnitude::Pitch),
        },
        ControlSpec {
            parameter: parameters::SAW_AMPLITUDE,
            control: kernels::SAW_AMPLITUDE,
            rate: ControlRate::Quantum,
            magnitude: None,
        },
    ],
    in_place_safe: false,
    note_control: None,
    prepare: prepare_saw,
    prepared_bytes: size_of::<(
        f64,
        crate::quantities::Frequency,
        crate::quantities::Amplitude,
    )>() as u64,
    state_bytes: size_of::<(
        f64,
        crate::quantities::Frequency,
        crate::quantities::Amplitude,
    )>() as u64,
};

/// The control-rate output an envelope declares.
const CONTROL_OUT: PortSpec = PortSpec::new(
    crate::ir::PortId::FIRST,
    PortDirection::Output,
    SignalDomain::Control,
    ChannelLayout::Mono,
);

/// The envelope, declared once — `P05-S002`, the first **playable** declared kind.
///
/// Its gate is the control a note edge moves, so `note_control` names it and the gate is
/// sample-positioned (ADR-0001 clause 14): addressing it as a parameter and playing the node
/// as a note reach the same control under the same timing law. Its velocity is
/// `SOUND-INV-021`'s velocity destination, and the reason it is the envelope's rather than
/// an oscillator's: the invariant requires the played node's scope to declare a destination
/// that **scales** the rendered amplitude, and an envelope's output is what an amplifier
/// multiplies its audio by. The byte attributions name the kernel's layouts: three segment
/// lengths and a sustain level prepared; a segment, three levels, a remaining count, the
/// gate and the velocity kept between quanta.
pub(crate) const ENVELOPE: NodeDeclaration = NodeDeclaration {
    kernel: kernels::ENVELOPE,
    ports: &[CONTROL_OUT],
    controls: &[
        ControlSpec {
            parameter: parameters::ENVELOPE_GATE,
            control: kernels::ENVELOPE_GATE,
            rate: ControlRate::Sample,
            magnitude: None,
        },
        ControlSpec {
            parameter: parameters::ENVELOPE_VELOCITY,
            control: kernels::ENVELOPE_VELOCITY,
            rate: ControlRate::Sample,
            magnitude: Some(NoteMagnitude::Velocity),
        },
    ],
    in_place_safe: false,
    note_control: Some(kernels::ENVELOPE_GATE),
    prepare: prepare_envelope,
    prepared_bytes: size_of::<(SegmentFrames, SegmentFrames, SegmentFrames, NormalizedLevel)>()
        as u64,
    state_bytes: size_of::<(
        kernels::Segment,
        f32,
        f32,
        f32,
        u32,
        bool,
        crate::quantities::NoteVelocity,
    )>() as u64,
};

/// The sine, declared once — `P05-S003`. The sawtooth's shape with the sine's kernel.
pub(crate) const SINE: NodeDeclaration = NodeDeclaration {
    kernel: kernels::SINE,
    ports: &[AUDIO_OUT],
    controls: &[
        // `SOUND-INV-021`'s pitch destination. Sample-positioned because it is one: a
        // note's key describes the note its gate starts, so a frequency that waited for
        // the next boundary would sound the previous note's pitch for up to a quantum.
        ControlSpec {
            parameter: parameters::SINE_FREQUENCY,
            control: kernels::SINE_FREQUENCY,
            rate: ControlRate::Sample,
            magnitude: Some(NoteMagnitude::Pitch),
        },
        ControlSpec {
            parameter: parameters::SINE_AMPLITUDE,
            control: kernels::SINE_AMPLITUDE,
            rate: ControlRate::Quantum,
            magnitude: None,
        },
    ],
    in_place_safe: false,
    note_control: None,
    prepare: prepare_sine,
    prepared_bytes: size_of::<(
        f64,
        crate::quantities::Frequency,
        crate::quantities::Amplitude,
    )>() as u64,
    state_bytes: size_of::<(
        f64,
        crate::quantities::Frequency,
        crate::quantities::Amplitude,
    )>() as u64,
};

/// Zeros, declared once — `P05-S003`. No control, nothing prepared, nothing kept.
pub(crate) const SILENCE: NodeDeclaration = NodeDeclaration {
    kernel: kernels::SILENCE,
    ports: &[AUDIO_OUT],
    controls: &[],
    in_place_safe: false,
    note_control: None,
    prepare: prepare_silence,
    prepared_bytes: 0,
    state_bytes: 0,
};

/// A constant level, declared once — `P05-S003`. The level is prepared; nothing is kept.
pub(crate) const CONSTANT: NodeDeclaration = NodeDeclaration {
    kernel: kernels::CONSTANT,
    ports: &[AUDIO_OUT],
    controls: &[],
    in_place_safe: false,
    note_control: None,
    prepare: prepare_constant,
    prepared_bytes: size_of::<crate::quantities::Amplitude>() as u64,
    state_bytes: 0,
};

/// One click at a plan position, declared once — `P05-S003`. The position is prepared.
pub(crate) const IMPULSE: NodeDeclaration = NodeDeclaration {
    kernel: kernels::IMPULSE,
    ports: &[AUDIO_OUT],
    controls: &[],
    in_place_safe: false,
    note_control: None,
    prepare: prepare_impulse,
    prepared_bytes: size_of::<crate::time::PlanPosition>() as u64,
    state_bytes: 0,
};

/// The mono audio input on the first port, for the kinds that take one.
const AUDIO_IN: PortSpec = PortSpec::new(
    crate::ir::PortId::FIRST,
    PortDirection::Input,
    SignalDomain::Audio,
    ChannelLayout::Mono,
);

/// The amplifier's control input, on [`AMPLIFIER_CONTROL`].
const AMPLIFIER_CONTROL_IN: PortSpec = PortSpec::new(
    AMPLIFIER_CONTROL,
    PortDirection::Input,
    SignalDomain::Control,
    ChannelLayout::Mono,
);

/// The amplifier, declared once — `P05-S004`. Audio in, control in, audio out, and nothing
/// prepared or kept: each output sample is its input sample times its control sample, so
/// writing the result over the audio input changes nothing about it (ADR-0005 clause 5).
pub(crate) const AMPLIFIER: NodeDeclaration = NodeDeclaration {
    kernel: kernels::AMPLIFIER,
    ports: &[AUDIO_IN, AMPLIFIER_CONTROL_IN, AUDIO_OUT],
    controls: &[],
    in_place_safe: true,
    note_control: None,
    prepare: prepare_amplifier,
    prepared_bytes: 0,
    state_bytes: 0,
};

/// A fixed gain, declared once — `P05-S004`. The factor is prepared; nothing is kept; and a
/// gain scales each sample independently, so one buffer serves as input and output.
pub(crate) const GAIN: NodeDeclaration = NodeDeclaration {
    kernel: kernels::GAIN,
    ports: &[AUDIO_IN, AUDIO_OUT],
    controls: &[],
    in_place_safe: true,
    note_control: None,
    prepare: prepare_gain,
    prepared_bytes: size_of::<crate::quantities::GainFactor>() as u64,
    state_bytes: 0,
};

/// The low-pass filter, declared once — `P05-S004`. Three coefficients prepared, two
/// history samples kept. A biquad reads each input sample before it writes that sample's
/// output and its history is in its state rather than in the buffer, so writing over its
/// input changes nothing about its result.
pub(crate) const FILTER: NodeDeclaration = NodeDeclaration {
    kernel: kernels::FILTER,
    ports: &[AUDIO_IN, AUDIO_OUT],
    controls: &[],
    in_place_safe: true,
    note_control: None,
    prepare: prepare_filter,
    prepared_bytes: size_of::<[f32; 3]>() as u64,
    state_bytes: size_of::<(f32, f32)>() as u64,
};

/// The declaration a kind has, where it has moved to one.
///
/// The **only** per-kind `match` that carries a fact about a declared kind — its own
/// `prepare_*` function destructures its variant, and every other arm forwards here. Every
/// registry function asks here first and falls back to its own arms for the kinds that
/// have not moved yet, so migration is one kind at a time and a kind cannot be half
/// declared: its descriptor and both byte attributions come from the same value or none of
/// them do.
pub(crate) fn declaration(kind: IrNodeKind) -> Option<&'static NodeDeclaration> {
    match kind {
        IrNodeKind::Saw { .. } => Some(&SAW),
        IrNodeKind::Envelope { .. } => Some(&ENVELOPE),
        IrNodeKind::Sine { .. } => Some(&SINE),
        IrNodeKind::Silence => Some(&SILENCE),
        IrNodeKind::Constant { .. } => Some(&CONSTANT),
        IrNodeKind::Impulse { .. } => Some(&IMPULSE),
        IrNodeKind::Amplifier => Some(&AMPLIFIER),
        IrNodeKind::Gain { .. } => Some(&GAIN),
        IrNodeKind::Filter { .. } => Some(&FILTER),
        // The output node has no kernel and no declaration: writing the stream's channels
        // is the renderer's boundary rather than a node's work.
        IrNodeKind::Output => None,
    }
}

/// One mono audio input on the given port.
fn audio_in(port: crate::ir::PortId) -> PortSpec {
    PortSpec::new(
        port,
        PortDirection::Input,
        SignalDomain::Audio,
        ChannelLayout::Mono,
    )
}

/// What `kind` declares, for a plan rendering into `stream`.
///
/// The output node is the one kind with no kernel: it does not produce a signal, and
/// what it does — writing the stream's channels — is the renderer's boundary rather than
/// a node's work. It is the only kind whose ports depend on the stream, because it is
/// the one place a plan meets the layout the host asked for.
pub(crate) fn descriptor(kind: IrNodeKind) -> Option<NodeDescriptor> {
    let declared = declaration(kind);
    // Every kind but the output node is declared, so every arm below defers to its
    // declaration and states nothing; `tests/node_representation.rs` holds each to that
    // form. The match stays exhaustive rather than collapsing to `declared.map(..)` so that
    // a kind added to the IR is a compile error here until `declaration` knows it.
    match kind {
        IrNodeKind::Output => None,
        IrNodeKind::Silence => declared.map(NodeDeclaration::descriptor),
        IrNodeKind::Constant { .. } => declared.map(NodeDeclaration::descriptor),
        IrNodeKind::Impulse { .. } => declared.map(NodeDeclaration::descriptor),
        IrNodeKind::Sine { .. } => declared.map(NodeDeclaration::descriptor),
        IrNodeKind::Saw { .. } => declared.map(NodeDeclaration::descriptor),
        IrNodeKind::Envelope { .. } => declared.map(NodeDeclaration::descriptor),
        IrNodeKind::Amplifier => declared.map(NodeDeclaration::descriptor),
        IrNodeKind::Filter { .. } => declared.map(NodeDeclaration::descriptor),
        IrNodeKind::Gain { .. } => declared.map(NodeDeclaration::descriptor),
    }
}

/// The ports `kind` declares. The output node's depend on the stream; nothing else's do.
///
/// Public because it is the plan's *interface*: a caller building a graph, and the test
/// that checks every declared port is reachable, both need to read it. Everything else in
/// the registry is admission's.
#[must_use]
pub fn ports(kind: IrNodeKind, stream: ChannelLayout) -> Vec<PortSpec> {
    match descriptor(kind) {
        Some(descriptor) => descriptor.ports,
        None => vec![PortSpec::new(
            crate::ir::PortId::FIRST,
            PortDirection::Input,
            SignalDomain::Audio,
            stream,
        )],
    }
}

/// The compiler's own copy operation, which no authored node declares.
///
/// ADR-0002 clause 7 makes a mono-to-stereo widening a scheduled operation with an
/// identity; this is the descriptor the compiler schedules it under.
pub(crate) fn copy_descriptor() -> NodeDescriptor {
    NodeDescriptor {
        kernel: kernels::COPY,
        ports: vec![audio_in(crate::ir::PortId::FIRST), audio_out()],
        controls: Vec::new(),
        // A copy exists to produce a second buffer. Writing it over its own input would
        // leave one buffer where the plan needs two.
        in_place_safe: false,
        note_control: None,
    }
}

/// Build the prepared data for one node, against the stream it will render into.
///
/// Everything derived from the stream is derived **here**, once — a sine's per-frame
/// phase step rather than a divide per quantum.
pub(crate) fn prepare(
    node: NodeId,
    kind: IrNodeKind,
    rate: SampleRate,
) -> Result<PreparedNode, CompileError> {
    match declaration(kind) {
        Some(declared) => (declared.prepare)(node, kind, rate),
        // The output node has no kernel and no prepared data of its own; it is given a
        // record so that the prepared and state tables stay indexed by the same slot.
        None => Ok(PreparedNode::Silence),
    }
}

/// How many frames a duration lasts at a rate, or `None` where that is not a frame count.
///
/// `Seconds` admits any finite non-negative value, and the largest of them is more frames
/// than a counter can hold — `f32::MAX` seconds is longer than the universe has run. The
/// conversion is where that stops being a quantity and becomes a number this node cannot
/// use, so it is where the plan is refused rather than silently given a segment that
/// never advances. `u32::MAX` frames is a little over a day at 48 kHz.
fn frames_in(duration: Seconds, rate: SampleRate) -> Option<SegmentFrames> {
    // Rounded, not truncated. A decimal duration is not exactly representable, so ten
    // milliseconds at 48 kHz is 479.999989 frames — and truncating it would make every
    // ordinary duration one frame short of what was written.
    let frames = (f64::from(duration.as_f32()) * f64::from(rate.as_f32())).round();
    if frames.is_finite() && frames <= f64::from(u32::MAX) {
        Some(SegmentFrames::new(frames as u32))
    } else {
        None
    }
}

/// The coefficients of a two-pole low-pass, or why this stream cannot have one.
///
/// The topology-preserving state-variable form: `g = tan(pi f / fs)` and a damping of
/// `1/Q`, from which the three integrator coefficients follow. Evaluated once, in `f64`,
/// off the audio thread.
///
/// A corner frequency at or above the stream's Nyquist frequency is **refused rather
/// than clamped**: clamping would render a filter the caller did not ask for, and a plan
/// whose corner frequency is above the rate it is being admitted against is a mistake
/// worth a diagnostic.
fn low_pass(
    node: NodeId,
    cutoff: CutoffFrequency,
    resonance: Resonance,
    rate: SampleRate,
) -> Result<PreparedNode, CompileError> {
    let nyquist = rate.nyquist();
    if cutoff.as_f32() >= nyquist.as_f32() {
        return Err(CompileError::NodeNotPreparable {
            node,
            fault: PreparationFault::CutoffAboveNyquist { cutoff, nyquist },
        });
    }

    let g = (std::f64::consts::PI * f64::from(cutoff.as_f32()) / f64::from(rate.as_f32())).tan();
    let damping = 1.0 / f64::from(resonance.as_f32());
    let first = 1.0 / (1.0 + g * (g + damping));
    let integrator = [first as f32, (g * first) as f32, (g * g * first) as f32];

    // The **derived** values are what the kernel reads, so they are what is checked, and
    // the check is what the form makes possible: this filter's gain at DC is one by
    // construction, so the only way it can be wrong is for a coefficient to leave the
    // representable range. A quality factor small enough to be subnormal drives the
    // damping past `1e45` and takes the coefficients down with it, producing a plan that
    // admits, renders, and is silent forever with nothing saying so.
    //
    // **Subnormal counts as unusable**, not merely zero. A subnormal coefficient has
    // lost most of its significand, so it is already the wrong filter — and it would go
    // on to be multiplied on the audio thread, where subnormal arithmetic is a stall on
    // several of the processors this runs on. Exact zero stays legal for the last
    // coefficient, which is `g²` scaled and is genuinely zero for a corner frequency
    // near the bottom of the range.
    let representable = |value: f32| value == 0.0 || value.is_normal();
    let usable = integrator.iter().copied().all(representable) && integrator[1].is_normal();
    if !usable {
        return Err(CompileError::NodeNotPreparable {
            node,
            fault: PreparationFault::CoefficientsUnusable { cutoff, resonance },
        });
    }

    // Representable is not the same as stable, and the difference is a filter that grows
    // without bound. The check is over the **rounded** coefficients — the ones the kernel
    // will actually multiply — because rounding is what moves a pole: this recurrence's
    // state matrix is `[[2a1-1, -2a2], [2a2, 1-2a3]]`, and Jury's criterion for a
    // second-order system puts both of its eigenvalues inside the unit circle exactly
    // when the determinant is below one and the trace is inside `1 + determinant`. A
    // quality factor in the tens of millions rounds `a1` up far enough to fail it, after
    // which an impulse response climbs for as long as the stream runs.
    let (first, second, third) = (
        f64::from(integrator[0]),
        f64::from(integrator[1]),
        f64::from(integrator[2]),
    );
    let trace = 2.0 * first - 2.0 * third;
    let determinant = (2.0 * first - 1.0) * (1.0 - 2.0 * third) + 4.0 * second * second;
    if determinant >= 1.0 || trace.abs() >= 1.0 + determinant {
        return Err(CompileError::NodeNotPreparable {
            node,
            fault: PreparationFault::CoefficientsUnstable { cutoff, resonance },
        });
    }

    Ok(PreparedNode::Filter { integrator })
}

/// The prepared data the compiler's copy operation carries.
pub(crate) const fn prepare_copy() -> PreparedNode {
    PreparedNode::Copy
}

/// The bytes one node's prepared data occupies.
///
/// **Measured from the representation, not declared per kind.** ADR-0004's registry gives
/// every node one [`PreparedNode`] record whatever its kind, so this is what a plan
/// actually allocates — which is what `HOST-INV-014` asks the report to state. The kind
/// with the widest payload therefore sets the cost of every node, and
/// [`prepared_payload_bytes`] is how the report says which kind that was.
#[must_use]
pub const fn prepared_bytes_per_node() -> u64 {
    size_of::<PreparedNode>() as u64
}

/// The bytes one node's mutable state occupies.
#[must_use]
pub const fn state_bytes_per_node() -> u64 {
    size_of::<kernels::NodeState>() as u64
}

/// The prepared payload one kind carries, before the representation rounds it up.
///
/// Not what a node costs — [`prepared_bytes_per_node`] is that. This is what the report
/// attributes *with*: every node costs the same, so the object responsible for that cost
/// is the one whose payload is widest, and a reader who wants the row smaller needs to
/// know which kind that is.
///
/// Each figure is `size_of` a tuple of the variant's own field types rather than a sum of
/// their sizes. The difference is alignment: a payload of one `f64` and two `f32`s is
/// wider than one of four `f32`s even though the sums are equal, and it is the wider one
/// that sets the record. Summing by hand named the wrong node for exactly that reason.
#[must_use]
pub fn prepared_payload_bytes(kind: IrNodeKind) -> u64 {
    let declared = declaration(kind);
    (match kind {
        // Declared: the arm defers and states nothing.
        IrNodeKind::Saw { .. } => return declared.map_or(0, |d| d.prepared_bytes),
        IrNodeKind::Silence => return declared.map_or(0, |d| d.prepared_bytes),
        IrNodeKind::Constant { .. } => return declared.map_or(0, |d| d.prepared_bytes),
        IrNodeKind::Impulse { .. } => return declared.map_or(0, |d| d.prepared_bytes),
        IrNodeKind::Sine { .. } => return declared.map_or(0, |d| d.prepared_bytes),
        IrNodeKind::Amplifier => return declared.map_or(0, |d| d.prepared_bytes),
        IrNodeKind::Gain { .. } => return declared.map_or(0, |d| d.prepared_bytes),
        IrNodeKind::Filter { .. } => return declared.map_or(0, |d| d.prepared_bytes),
        // The output node has no kernel, so it carries no prepared data of its own.
        IrNodeKind::Output => 0,
        IrNodeKind::Envelope { .. } => return declared.map_or(0, |d| d.prepared_bytes),
    }) as u64
}

/// The mutable payload one kind carries, for the same attribution and by the same rule.
#[must_use]
pub fn state_payload_bytes(kind: IrNodeKind) -> u64 {
    let declared = declaration(kind);
    (match kind {
        // Declared: the arm defers and states nothing.
        IrNodeKind::Saw { .. } => return declared.map_or(0, |d| d.state_bytes),
        IrNodeKind::Sine { .. } => return declared.map_or(0, |d| d.state_bytes),
        IrNodeKind::Silence => return declared.map_or(0, |d| d.state_bytes),
        IrNodeKind::Constant { .. } => return declared.map_or(0, |d| d.state_bytes),
        IrNodeKind::Impulse { .. } => return declared.map_or(0, |d| d.state_bytes),
        IrNodeKind::Filter { .. } => return declared.map_or(0, |d| d.state_bytes),
        IrNodeKind::Amplifier => return declared.map_or(0, |d| d.state_bytes),
        IrNodeKind::Gain { .. } => return declared.map_or(0, |d| d.state_bytes),
        IrNodeKind::Envelope { .. } => return declared.map_or(0, |d| d.state_bytes),
        IrNodeKind::Output => 0,
    }) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quantities::{Amplitude, GainFactor};
    use crate::time::PlanPosition;

    /// Every kind this phase has, so a scan over them is a scan over all of them.
    fn every_kind() -> Vec<IrNodeKind> {
        vec![
            IrNodeKind::Silence,
            IrNodeKind::Constant {
                level: Amplitude::UNITY,
            },
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
            },
            IrNodeKind::Sine {
                frequency: crate::quantities::Frequency::ZERO,
                amplitude: Amplitude::UNITY,
            },
            IrNodeKind::Saw {
                frequency: crate::quantities::Frequency::ZERO,
                amplitude: Amplitude::UNITY,
            },
            IrNodeKind::Gain {
                factor: GainFactor::UNITY,
            },
            IrNodeKind::Amplifier,
            IrNodeKind::Envelope {
                attack: crate::quantities::Seconds::ZERO,
                decay: crate::quantities::Seconds::ZERO,
                sustain: crate::quantities::NormalizedLevel::FULL,
                release: crate::quantities::Seconds::ZERO,
            },
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(1_000.0).expect("positive"),
                resonance: Resonance::BUTTERWORTH,
            },
            IrNodeKind::Output,
        ]
    }

    #[test]
    fn descriptor_destinations_are_sample_positioned() {
        // `SOUND-INV-021`: a note's magnitudes must be in force at the sample its gate
        // rises. A destination declared at quantum rate could not be — its write would
        // wait for the boundary after the gate — so the pairing is checked rather than
        // remembered. This is the assertion `ControlSpec::magnitude`'s documentation
        // names, and it fails for a kind added with the wrong rate.
        for kind in every_kind() {
            let Some(descriptor) = descriptor(kind) else {
                continue;
            };
            for spec in &descriptor.controls {
                if let Some(magnitude) = spec.magnitude {
                    assert_eq!(
                        spec.rate,
                        ControlRate::Sample,
                        "{kind:?} declares a {magnitude} destination at quantum rate, which \
                         cannot be in force at the sample its note's gate rises"
                    );
                }
            }
        }
    }

    #[test]
    fn a_playable_kind_declares_no_magnitude_twice() {
        // The renderer writes one control per declared destination, so a kind naming the
        // same control as both a pitch and a velocity destination — or naming one twice —
        // would produce two writes racing for one value with no rule about which wins.
        for kind in every_kind() {
            let Some(descriptor) = descriptor(kind) else {
                continue;
            };
            let mut seen: Vec<ControlIndex> = Vec::new();
            for spec in &descriptor.controls {
                if spec.magnitude.is_some() {
                    assert!(
                        !seen.contains(&spec.control),
                        "{kind:?} declares control {:?} as a note destination twice",
                        spec.control
                    );
                    seen.push(spec.control);
                }
            }
        }
    }

    #[test]
    fn a_declared_payload_never_exceeds_the_record_that_holds_it() {
        // The payload figures are written by hand beside the variants they describe, so
        // they drift when a variant changes — one already did, and a reviewer caught it
        // rather than a test. A payload larger than the record it lives in is that drift
        // made checkable: it cannot be true, and it is what an out-of-date figure looks
        // like when the variant shrank.
        for kind in every_kind() {
            assert!(
                prepared_payload_bytes(kind) <= prepared_bytes_per_node(),
                "{kind:?} declares a prepared payload larger than a whole prepared record"
            );
            assert!(
                state_payload_bytes(kind) <= state_bytes_per_node(),
                "{kind:?} declares a state payload larger than a whole state record"
            );
        }
    }

    #[test]
    fn the_widest_payload_accounts_for_the_record_it_sets() {
        // The other direction: the widest declared payload is what makes the record as
        // wide as it is, so the two may differ only by the discriminant and the padding
        // that follows it. A figure that is far *under* the record is the same drift seen
        // from the other side — a variant grew and its line was not updated.
        let widest = every_kind()
            .into_iter()
            .map(prepared_payload_bytes)
            .max()
            .unwrap_or(0);
        let record = prepared_bytes_per_node();
        assert!(
            record.saturating_sub(widest) <= size_of::<u64>() as u64,
            "the widest declared payload is {widest} bytes and a record is {record}; one of \
             the payload figures is out of date"
        );
    }

    /// `P05-S001`, extended by `P05-S002`: every registry fact about a declared kind is the
    /// declaration's.
    ///
    /// Read back through the registry functions rather than from the constant, so this
    /// fails if any of them stops forwarding: the descriptor's kernel, ports, controls,
    /// in-place eligibility and note control, and both byte attributions, must be the
    /// declaration's own values, for every kind `declaration` knows. Mutation-verified by
    /// restating any one of them in the function's own arm — the scan in
    /// `tests/node_representation.rs` catches the form, and this catches a restated value
    /// that happens to differ.
    #[test]
    fn a_declared_kinds_registry_facts_derive_from_its_declaration() {
        let declared: Vec<IrNodeKind> = every_kind()
            .into_iter()
            .filter(|kind| declaration(*kind).is_some())
            .collect();
        assert_eq!(
            declared.len(),
            9,
            "every kind but the output node is declared"
        );

        for kind in declared {
            let declared = declaration(kind).expect("filtered on it");
            let descriptor = descriptor(kind).expect("a declared kind has a descriptor");
            assert!(descriptor.kernel.is_same(declared.kernel), "{kind:?}");
            assert_eq!(descriptor.ports, declared.ports.to_vec(), "{kind:?}");
            assert_eq!(descriptor.controls, declared.controls.to_vec(), "{kind:?}");
            assert_eq!(descriptor.in_place_safe, declared.in_place_safe, "{kind:?}");
            assert_eq!(descriptor.note_control, declared.note_control, "{kind:?}");
            assert_eq!(
                ports(kind, ChannelLayout::Stereo),
                declared.ports.to_vec(),
                "{kind:?}: the public port set is the declaration's, for any stream"
            );
            assert_eq!(
                prepared_payload_bytes(kind),
                declared.prepared_bytes,
                "{kind:?}"
            );
            assert_eq!(state_payload_bytes(kind), declared.state_bytes, "{kind:?}");
            // The declaration's preparation builds **this** kind's record: a declaration
            // wired to another kind's `prepare_*` is refused rather than rendered.
            let rate = SampleRate::new(48_000.0).expect("a real rate");
            let prepared = prepare(NodeId::new(0), kind, rate).expect("a declared kind prepares");
            let matches_kind = matches!(
                (kind, &prepared),
                (IrNodeKind::Silence, PreparedNode::Silence)
                    | (IrNodeKind::Constant { .. }, PreparedNode::Constant { .. })
                    | (IrNodeKind::Impulse { .. }, PreparedNode::Impulse { .. })
                    | (IrNodeKind::Sine { .. }, PreparedNode::Sine { .. })
                    | (IrNodeKind::Saw { .. }, PreparedNode::Saw { .. })
                    | (IrNodeKind::Gain { .. }, PreparedNode::Gain { .. })
                    | (IrNodeKind::Amplifier, PreparedNode::Amplifier)
                    | (IrNodeKind::Filter { .. }, PreparedNode::Filter { .. })
                    | (IrNodeKind::Envelope { .. }, PreparedNode::Envelope { .. })
            );
            assert!(matches_kind, "{kind:?} prepared as {prepared:?}");
            // What each kind prepares and keeps, so a zero written where a layout belongs
            // — or a layout where nothing is kept — is caught by kind.
            let (prepares, keeps) = match kind {
                IrNodeKind::Saw { .. } | IrNodeKind::Sine { .. } | IrNodeKind::Envelope { .. } => {
                    (true, true)
                }
                IrNodeKind::Constant { .. }
                | IrNodeKind::Impulse { .. }
                | IrNodeKind::Gain { .. } => (true, false),
                IrNodeKind::Filter { .. } => (true, true),
                IrNodeKind::Silence | IrNodeKind::Amplifier => (false, false),
                other => panic!("{other:?} is declared but this test does not know its shape"),
            };
            assert_eq!(
                (declared.prepared_bytes > 0, declared.state_bytes > 0),
                (prepares, keeps),
                "{kind:?}"
            );
            // ADR-0005 clause 5's first condition, per kind: the three kinds that take an
            // input each compute an output sample from that sample and their own state, so
            // writing over the input changes nothing; a source has no input to overwrite.
            // Flipping the flag is behaviour-preserving — the arena simply allocates — so
            // nothing else notices, and this is what holds the declaration to the fact.
            assert_eq!(
                declared.in_place_safe,
                matches!(
                    kind,
                    IrNodeKind::Amplifier | IrNodeKind::Gain { .. } | IrNodeKind::Filter { .. }
                ),
                "{kind:?}"
            );
        }

        // And each declaration says what its kind is. The sawtooth is a pitched source: its
        // frequency is a sample-positioned pitch destination and no note plays it directly.
        assert!(
            SAW.controls
                .iter()
                .any(|c| c.magnitude == Some(NoteMagnitude::Pitch) && c.rate == ControlRate::Sample)
                && SAW.note_control.is_none()
        );
        // The envelope is the played node: its gate is the note control, sample-positioned,
        // and its velocity is the velocity destination, also sample-positioned.
        let gate = ENVELOPE
            .controls
            .iter()
            .find(|c| Some(c.control) == ENVELOPE.note_control)
            .expect("the note control is one of the declared controls");
        assert_eq!(gate.rate, ControlRate::Sample);
        assert!(
            ENVELOPE
                .controls
                .iter()
                .any(|c| c.magnitude == Some(NoteMagnitude::Velocity)
                    && c.rate == ControlRate::Sample)
        );
    }
}
