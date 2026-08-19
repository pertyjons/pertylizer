//! What a kernel does, called directly.
//!
//! These are here rather than in `tests/` because a kernel's contract is stated in terms
//! the public API cannot express: a control signal has no path to the output node — the
//! output takes audio — so an envelope's shape is only observable through the node that
//! consumes it. Checking the kernel itself is the alternative to waiting for the rest of
//! the path, and it is a sharper instrument in any case: a rendered chain would hide a
//! wrong segment behind the amplifier that reads it.
//!
//! `tests/render_loop_purity.rs` scans `src/node/kernels.rs` and would fail on the
//! assertions themselves, which is the other reason these live in their own file.

use crate::node::kernels::{
    InputBuffer, MAX_INPUTS, NodeIo, NodeState, PreparedNode, Segment, TimedControl, envelope,
};
use crate::quantities::{NormalizedLevel, ParameterValue, SegmentFrames};
use crate::time::QuantumOffset;

/// Run a kernel over one quantum-sized buffer and return what it wrote.
fn run(
    kernel: fn(&PreparedNode, &mut NodeState, &mut NodeIo<'_>),
    prepared: &PreparedNode,
    state: &mut NodeState,
    frames: usize,
) -> Vec<f32> {
    run_with(kernel, prepared, state, frames, &[])
}

/// The same, with sample-positioned control changes due inside the buffer.
fn run_with(
    kernel: fn(&PreparedNode, &mut NodeState, &mut NodeIo<'_>),
    prepared: &PreparedNode,
    state: &mut NodeState,
    frames: usize,
    controls: &[TimedControl],
) -> Vec<f32> {
    let mut out = vec![0.0; frames];
    let mut io = NodeIo {
        out: &mut out,
        channels: crate::quantities::ChannelLayout::Mono,
        inputs: [InputBuffer::Unpatched; MAX_INPUTS],
        position: None,
        controls,
    };
    kernel(prepared, state, &mut io);
    out
}

/// An envelope whose segments each last `frames` frames at the given sustain level.
fn adsr(frames: u32, sustain: f32) -> PreparedNode {
    PreparedNode::Envelope {
        attack_frames: SegmentFrames::new(frames),
        decay_frames: SegmentFrames::new(frames),
        release_frames: SegmentFrames::new(frames),
        sustain: NormalizedLevel::new(sustain).expect("a level within the range"),
    }
}

/// A gate edge at an offset inside the quantum about to be rendered.
///
/// Since P02-T007 a gate is sample-positioned (ADR-0001 clause 14), so it reaches the
/// kernel with the buffer rather than being set on the state beforehand. An edge at
/// offset 0 is what the old boundary-applied gate was, and every test below that only
/// cares about *whether* the gate is held uses that offset.
fn gate_at(offset: u16, held: bool) -> TimedControl {
    TimedControl {
        offset: QuantumOffset::new(offset).expect("an offset inside the quantum"),
        control: crate::node::kernels::ENVELOPE_GATE,
        value: ParameterValue::new(if held { 1.0 } else { 0.0 }).expect("finite"),
    }
}

#[test]
fn an_ungated_envelope_is_silent() {
    let prepared = adsr(64, 0.5);
    let mut state = NodeState::initial(&prepared);
    let rendered = run(envelope, &prepared, &mut state, 64);
    assert!(
        rendered.iter().all(|value| *value == 0.0),
        "an envelope nobody gated has produced something"
    );
}

#[test]
fn a_held_gate_rises_then_settles_at_the_sustain_level() {
    let prepared = adsr(64, 0.5);
    let mut state = NodeState::initial(&prepared);
    // A segment of `n` frames writes `n` values, starting at the level it inherited and
    // ending one step short of its target: the target itself is the first value of the
    // segment that follows. That is what makes a chain continuous, and it is why the peak
    // lands on frame 65 rather than 64.
    let attack = run_with(envelope, &prepared, &mut state, 65, &[gate_at(0, true)]);
    let peak = attack.last().copied().unwrap_or(0.0);
    assert!(
        (peak - 1.0).abs() < 1e-6,
        "a 64-frame attack arrives at full level on the frame after it, not {peak}"
    );
    assert!(
        attack.first().copied().unwrap_or(1.0) == 0.0,
        "and it starts from the level it had, which was silence"
    );
    assert!(
        attack.windows(2).all(|pair| pair[1] >= pair[0]),
        "the attack segment rises monotonically"
    );

    let decay = run(envelope, &prepared, &mut state, 64);
    let settled = decay.last().copied().unwrap_or(0.0);
    assert!(
        (settled - 0.5).abs() < 1e-6,
        "a 64-frame decay arrives at the sustain level, not {settled}"
    );

    // Held, so it stays there for as long as the caller renders.
    let sustain = run(envelope, &prepared, &mut state, 256);
    assert!(
        sustain.iter().all(|value| (*value - 0.5).abs() < 1e-6),
        "a held gate holds the sustain level"
    );
    assert!(
        matches!(
            state,
            NodeState::Envelope {
                segment: Segment::Sustain,
                level,
                ..
            } if (level - 0.5).abs() < 1e-6
        ),
        "a held gate ends in Sustain at the sustain level, not {state:?}"
    );
}

#[test]
fn a_released_gate_falls_to_silence_and_stops() {
    let prepared = adsr(64, 0.5);
    let mut state = NodeState::initial(&prepared);
    run_with(envelope, &prepared, &mut state, 128, &[gate_at(0, true)]);

    let release = run_with(envelope, &prepared, &mut state, 65, &[gate_at(0, false)]);
    let last = release.last().copied().unwrap_or(1.0);
    assert!(
        last == 0.0,
        "a 64-frame release from the sustain level reaches exactly zero, not {last}"
    );
    assert!(
        matches!(
            state,
            NodeState::Envelope {
                segment: Segment::Idle,
                level: 0.0,
                ..
            }
        ),
        "and it stops there rather than falling through zero, not {state:?}"
    );
}

#[test]
fn a_note_let_go_early_still_reaches_silence() {
    // The release increment cannot be prepared from the sustain level, and a sustain of
    // zero is where that shows: an envelope released during its attack would sit at
    // whatever level it had reached, falling by zero per sample, for as long as the
    // stream ran. It is a held note nobody played, and it would be inaudible in a mix
    // and obvious in isolation.
    let prepared = adsr(64, 0.0);
    let mut state = NodeState::initial(&prepared);
    // Let go a quarter of the way up the attack.
    run_with(envelope, &prepared, &mut state, 16, &[gate_at(0, true)]);

    let release = run_with(envelope, &prepared, &mut state, 65, &[gate_at(0, false)]);
    assert_eq!(
        release.last().copied(),
        Some(0.0),
        "a release that started at a quarter level has to arrive at silence"
    );
    assert!(
        matches!(
            state,
            NodeState::Envelope {
                segment: Segment::Idle,
                ..
            }
        ),
        "and the envelope has to be idle afterwards, not {state:?}"
    );
}

#[test]
fn a_gate_edge_on_a_quantum_boundary_continues_from_where_the_ramp_is() {
    // The state a quantum leaves behind is the level the **next** sample would have, not
    // the last one it wrote — the counter has already moved past that. A release
    // beginning exactly on a boundary therefore starts where the attack had got to, and
    // storing the previous sample instead would put a one-step jump in the signal at
    // every boundary a note happens to end on.
    let prepared = adsr(64, 0.5);
    let mut state = NodeState::initial(&prepared);
    let attack = run_with(envelope, &prepared, &mut state, 32, &[gate_at(0, true)]);
    let after_boundary = match state {
        NodeState::Envelope { level, .. } => level,
        other => panic!("an envelope's state is an envelope: {other:?}"),
    };

    let release = run_with(envelope, &prepared, &mut state, 32, &[gate_at(0, false)]);
    let first = release.first().copied().unwrap_or(0.0);
    assert!(
        (first - after_boundary).abs() < 1e-6,
        "the release starts at the boundary level {after_boundary}, not at {first}"
    );
    let last_attack = attack.last().copied().unwrap_or(0.0);
    assert!(
        first > last_attack,
        "and the boundary level is one step further along the ramp than the last sample \
         written ({first} against {last_attack})"
    );
}

#[test]
fn a_gate_re_asserted_does_not_restart_the_note() {
    // Automation that emits the same held gate every quantum is ordinary, and treating
    // each of them as a note-on would restart the attack forever: the envelope would
    // never reach its sustain level, and a long note would sound like a stuttering one.
    // Attack begins on the **edge**.
    let prepared = adsr(64, 0.5);
    let mut state = NodeState::initial(&prepared);
    run_with(envelope, &prepared, &mut state, 128, &[gate_at(0, true)]);
    let settled = state;

    // Two runs from the same settled state: one told the gate is high again, one told
    // nothing. A retrigger would make them differ, and comparing against a control is
    // what turns "it stayed at sustain" into a check a wrong envelope can fail — an
    // envelope that restarted its attack from full level would also sit near 0.5.
    let mut reasserted = settled;
    let held = run_with(
        envelope,
        &prepared,
        &mut reasserted,
        64,
        &[gate_at(0, true)],
    );
    let mut untouched = settled;
    let control = run(envelope, &prepared, &mut untouched, 64);

    assert_eq!(
        held, control,
        "re-asserting a gate that is already held changed what the envelope wrote"
    );
    assert_eq!(
        reasserted, untouched,
        "re-asserting a gate that is already held changed the envelope's state"
    );
    assert!(
        held.iter().all(|value| (*value - 0.5).abs() < 1e-6),
        "and it stays at the sustain level rather than climbing again"
    );
}

#[test]
fn an_authored_duration_is_the_duration_it_takes() {
    // The reason the level is derived from a frame counter rather than accumulated: a
    // rounded per-sample increment added up over a one-second attack arrives tens of
    // samples early or late, and the error grows with the duration. Here the segment ends
    // on the frame it was given, at any length.
    for frames in [1_u32, 7, 64, 1_000, 48_000] {
        let prepared = adsr(frames, 0.5);
        let mut state = NodeState::initial(&prepared);
        let rendered = run_with(
            envelope,
            &prepared,
            &mut state,
            frames as usize + 1,
            &[gate_at(0, true)],
        );
        let arrival = rendered.last().copied().unwrap_or(0.0);
        assert!(
            (arrival - 1.0).abs() < 1e-6,
            "a {frames}-frame attack arrives at {arrival} rather than at full level"
        );
        let before = rendered
            .get(rendered.len().saturating_sub(2))
            .copied()
            .unwrap_or(1.0);
        assert!(
            before < 1.0 || frames == 1,
            "and it has not arrived early: the frame before is already at {before}"
        );
    }
}

#[test]
fn a_zero_length_attack_is_instantaneous_rather_than_infinite() {
    // The division-by-zero case, which is also the ordinary way to ask for a click: a
    // segment shorter than a frame moves the whole distance in one sample.
    let prepared = PreparedNode::Envelope {
        attack_frames: SegmentFrames::NONE,
        decay_frames: SegmentFrames::new(2),
        release_frames: SegmentFrames::new(2),
        sustain: NormalizedLevel::new(0.5).expect("a level within the range"),
    };
    let mut state = NodeState::initial(&prepared);
    let rendered = run_with(envelope, &prepared, &mut state, 4, &[gate_at(0, true)]);
    assert!(
        (rendered.first().copied().unwrap_or(0.0) - 1.0).abs() < f32::EPSILON,
        "the first sample of an instant attack is already at full level"
    );
}

#[test]
fn a_control_index_the_state_does_not_have_changes_nothing() {
    // The constants in `set_control` are matched as **patterns**, and a name that is not
    // a constant in scope would silently become a binding that matches everything — which
    // would make every parameter of a node move every one of its controls. This is what
    // that failure would look like from outside.
    //
    // Over the sine, which is the state with quantum-rate controls to confuse: an
    // envelope has none since P02-T007 moved its gate to the sample-positioned path, so
    // the pattern-binding failure would be invisible there.
    let prepared = PreparedNode::Sine {
        seconds_per_frame: 1.0 / 48_000.0,
        frequency: crate::quantities::Frequency::new(440.0).expect("finite"),
        amplitude: crate::quantities::Amplitude::UNITY,
    };
    let mut state = NodeState::initial(&prepared);
    state.set_control(
        crate::node::kernels::ControlIndex::new(7),
        ParameterValue::new(1.0).expect("finite"),
    );
    assert_eq!(
        state,
        NodeState::initial(&prepared),
        "an index this state does not have moved something"
    );
}

#[test]
fn a_gate_edge_inside_a_quantum_takes_effect_at_its_own_sample() {
    // ADR-0001 clause 14 at the kernel: an edge at offset `k` is applied before frame `k`
    // is written and after frame `k - 1` was. With instantaneous segments the envelope is
    // at the sustain level from frame `k` onward and at zero before it, so the boundary is
    // one sample wide and a gate applied to the whole quantum — the behaviour before this
    // task — puts it at frame 0 instead.
    const OFFSET: u16 = 37;
    let prepared = PreparedNode::Envelope {
        attack_frames: SegmentFrames::NONE,
        decay_frames: SegmentFrames::NONE,
        release_frames: SegmentFrames::NONE,
        sustain: NormalizedLevel::new(0.5).expect("a level within the range"),
    };
    let mut state = NodeState::initial(&prepared);
    let rendered = run_with(
        envelope,
        &prepared,
        &mut state,
        64,
        &[gate_at(OFFSET, true)],
    );

    for (frame, value) in rendered.iter().enumerate() {
        let expected = if frame < OFFSET as usize { 0.0 } else { 0.5 };
        assert!(
            (*value - expected).abs() < 1e-6,
            "frame {frame} of a gate at offset {OFFSET} is {value} rather than {expected}"
        );
    }
}

#[test]
fn two_edges_in_one_quantum_each_take_effect_at_their_own_sample() {
    // A note let go and retriggered inside 1.33 ms. Both edges are separate edges at
    // separate samples, so an implementation that keeps one pending edge per node loses
    // the first and one that collapses them onto a boundary loses both positions.
    const OFF: u16 = 20;
    const ON: u16 = 23;
    let prepared = PreparedNode::Envelope {
        attack_frames: SegmentFrames::NONE,
        decay_frames: SegmentFrames::NONE,
        release_frames: SegmentFrames::NONE,
        sustain: NormalizedLevel::new(0.5).expect("a level within the range"),
    };
    let mut state = NodeState::initial(&prepared);
    run_with(envelope, &prepared, &mut state, 64, &[gate_at(0, true)]);

    let rendered = run_with(
        envelope,
        &prepared,
        &mut state,
        64,
        &[gate_at(OFF, false), gate_at(ON, true)],
    );
    for (frame, value) in rendered.iter().enumerate() {
        let silent = (OFF as usize..ON as usize).contains(&frame);
        let expected = if silent { 0.0 } else { 0.5 };
        assert!(
            (*value - expected).abs() < 1e-6,
            "frame {frame} between edges at {OFF} and {ON} is {value} rather than {expected}"
        );
    }
}

/// The widening kernel, at **every** channel count its port admits.
///
/// ADR-0041 clause 12: an audio kernel is tested at every count its own ports admit, and
/// the compiler's copy is the one whose port follows the stream — so it is the one kernel
/// in this phase that has two counts to be right at. At one channel it is a plain copy;
/// at two it must write each source sample into both channels of a frame, which is
/// clause 8's duplication.
#[test]
fn the_widening_writes_every_channel_of_every_frame() {
    use crate::node::kernels::{InputBuffer, MAX_INPUTS, NodeIo, PreparedNode, copy};
    use crate::quantities::ChannelLayout;

    let frames = 4;
    let source: Vec<f32> = (0..frames).map(|frame| frame as f32 + 1.0).collect();

    for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
        let channels = layout.channels();
        // Seeded with a value no copy produces, so a sample the kernel fails to write is
        // visible rather than indistinguishable from silence — which is the defect a
        // widening that treats a `c * Q` region as mono produces.
        let mut out = vec![-1.0_f32; frames * channels];
        let mut inputs = [InputBuffer::Unpatched; MAX_INPUTS];
        inputs[0] = InputBuffer::Patched(&source);
        let mut io = NodeIo {
            out: &mut out,
            channels: layout,
            inputs,
            position: None,
            controls: &[],
        };
        copy(
            &PreparedNode::Copy,
            &mut crate::node::kernels::NodeState::Stateless,
            &mut io,
        );

        for (frame, expected) in source.iter().enumerate() {
            for channel in 0..channels {
                assert_eq!(
                    out.get(frame * channels + channel).copied(),
                    Some(*expected),
                    "at {layout:?}, frame {frame} channel {channel} must carry the source \
                     sample"
                );
            }
        }
    }
}
