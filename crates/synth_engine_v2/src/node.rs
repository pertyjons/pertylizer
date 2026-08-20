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
fn audio_out() -> PortSpec {
    PortSpec::new(
        crate::ir::PortId::FIRST,
        PortDirection::Output,
        SignalDomain::Audio,
        ChannelLayout::Mono,
    )
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
    let descriptor = match kind {
        IrNodeKind::Output => return None,
        IrNodeKind::Silence => NodeDescriptor {
            kernel: kernels::SILENCE,
            ports: vec![audio_out()],
            controls: Vec::new(),
            in_place_safe: false,
            note_control: None,
        },
        IrNodeKind::Constant { .. } => NodeDescriptor {
            kernel: kernels::CONSTANT,
            ports: vec![audio_out()],
            controls: Vec::new(),
            in_place_safe: false,
            note_control: None,
        },
        IrNodeKind::Impulse { .. } => NodeDescriptor {
            kernel: kernels::IMPULSE,
            ports: vec![audio_out()],
            controls: Vec::new(),
            in_place_safe: false,
            note_control: None,
        },
        IrNodeKind::Sine { .. } => NodeDescriptor {
            kernel: kernels::SINE,
            ports: vec![audio_out()],
            controls: vec![
                ControlSpec {
                    parameter: parameters::SINE_FREQUENCY,
                    control: kernels::SINE_FREQUENCY,
                    rate: ControlRate::Quantum,
                },
                ControlSpec {
                    parameter: parameters::SINE_AMPLITUDE,
                    control: kernels::SINE_AMPLITUDE,
                    rate: ControlRate::Quantum,
                },
            ],
            in_place_safe: false,
            note_control: None,
        },
        IrNodeKind::Amplifier => NodeDescriptor {
            kernel: kernels::AMPLIFIER,
            ports: vec![
                audio_in(crate::ir::PortId::FIRST),
                PortSpec::new(
                    AMPLIFIER_CONTROL,
                    PortDirection::Input,
                    SignalDomain::Control,
                    ChannelLayout::Mono,
                ),
                audio_out(),
            ],
            controls: Vec::new(),
            // Each output sample depends on its own input sample and nothing else, so
            // writing the result over the audio input changes nothing about it.
            in_place_safe: true,
            note_control: None,
        },
        IrNodeKind::Envelope { .. } => NodeDescriptor {
            kernel: kernels::ENVELOPE,
            ports: vec![PortSpec::new(
                crate::ir::PortId::FIRST,
                PortDirection::Output,
                SignalDomain::Control,
                ChannelLayout::Mono,
            )],
            // The gate is **sample-positioned** (ADR-0001 clause 14), and it says so here
            // rather than at the payload, so addressing it as a parameter and playing the
            // node as a note reach the same control under the same timing law. An earlier
            // revision left this at quantum rate, which is the defect P02-T007 closes.
            controls: vec![ControlSpec {
                parameter: parameters::ENVELOPE_GATE,
                control: kernels::ENVELOPE_GATE,
                rate: ControlRate::Sample,
            }],
            in_place_safe: false,
            note_control: Some(kernels::ENVELOPE_GATE),
        },
        IrNodeKind::Filter { .. } => NodeDescriptor {
            kernel: kernels::FILTER,
            ports: vec![audio_in(crate::ir::PortId::FIRST), audio_out()],
            controls: Vec::new(),
            // A biquad reads each input sample before it writes that sample's output, and
            // the history it needs is in its state rather than in the buffer, so writing
            // over its input changes nothing about its result.
            in_place_safe: true,
            note_control: None,
        },
        IrNodeKind::Gain { .. } => NodeDescriptor {
            kernel: kernels::GAIN,
            ports: vec![audio_in(crate::ir::PortId::FIRST), audio_out()],
            controls: Vec::new(),
            // A gain scales each sample independently, so reading and writing one buffer
            // changes nothing about its result.
            in_place_safe: true,
            note_control: None,
        },
    };
    Some(descriptor)
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
    Ok(match kind {
        // The output node has no kernel and no prepared data of its own; it is given a
        // record so that the prepared and state tables stay indexed by the same slot.
        IrNodeKind::Output | IrNodeKind::Silence => PreparedNode::Silence,
        IrNodeKind::Constant { level } => PreparedNode::Constant { level },
        IrNodeKind::Impulse { position } => PreparedNode::Impulse { position },
        IrNodeKind::Sine {
            frequency,
            amplitude,
        } => PreparedNode::Sine {
            seconds_per_frame: 1.0 / f64::from(rate.as_f32()),
            frequency,
            amplitude,
        },
        IrNodeKind::Gain { factor } => PreparedNode::Gain { factor },
        IrNodeKind::Amplifier => PreparedNode::Amplifier,
        IrNodeKind::Filter { cutoff, resonance } => return low_pass(node, cutoff, resonance, rate),
        IrNodeKind::Envelope {
            attack,
            decay,
            sustain,
            release,
        } => {
            // Each segment as the frames it lasts. The level a segment moves *through* is
            // not prepared, because it is not known until the segment starts: a note let
            // go during its attack releases from wherever it had reached.
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
            PreparedNode::Envelope {
                attack_frames: frames[0],
                decay_frames: frames[1],
                release_frames: frames[2],
                sustain,
            }
        }
    })
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
    (match kind {
        // The output node has no kernel, so it carries no prepared data of its own.
        IrNodeKind::Output | IrNodeKind::Silence | IrNodeKind::Amplifier => 0,
        IrNodeKind::Constant { .. } => size_of::<crate::quantities::Amplitude>(),
        IrNodeKind::Impulse { .. } => size_of::<crate::time::PlanPosition>(),
        IrNodeKind::Sine { .. } => size_of::<(
            f64,
            crate::quantities::Frequency,
            crate::quantities::Amplitude,
        )>(),
        IrNodeKind::Gain { .. } => size_of::<crate::quantities::GainFactor>(),
        IrNodeKind::Filter { .. } => size_of::<[f32; 3]>(),
        IrNodeKind::Envelope { .. } => {
            size_of::<(SegmentFrames, SegmentFrames, SegmentFrames, NormalizedLevel)>()
        }
    }) as u64
}

/// The mutable payload one kind carries, for the same attribution and by the same rule.
#[must_use]
pub fn state_payload_bytes(kind: IrNodeKind) -> u64 {
    (match kind {
        // Only these keep anything between quanta.
        IrNodeKind::Sine { .. } => size_of::<(
            f64,
            crate::quantities::Frequency,
            crate::quantities::Amplitude,
        )>(),
        IrNodeKind::Filter { .. } => size_of::<(f32, f32)>(),
        IrNodeKind::Envelope { .. } => size_of::<(kernels::Segment, f32, f32, f32, u32, bool)>(),
        IrNodeKind::Output
        | IrNodeKind::Silence
        | IrNodeKind::Amplifier
        | IrNodeKind::Constant { .. }
        | IrNodeKind::Impulse { .. }
        | IrNodeKind::Gain { .. } => 0,
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
}
