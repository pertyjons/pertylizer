//! P02-T007's conformance check: a note edge at its declared sample, and the complete
//! voice rendered from it.
//!
//! [ADR-0001](../../plans/v2/decisions/ADR-0001-internal-render-quantum.md) clause 14 is
//! what this file proves. A sample-positioned effect — note-on, note-off, gate,
//! retrigger — occurs at *the offset its render position names within the quantum that
//! renders it*, while a control-rate response begins at the first boundary at or after
//! that position. ADR-0043 restated the clause over that quantity; here the two coincide,
//! because this file renders offline over a sorted list with a monotone clock and so
//! cannot present a late event. `render_contract` covers the clamped case. Before this task the envelope's gate was
//! an ordinary control and landed on the boundary that followed it, so a note-on could be
//! up to `Q - 1` frames late and the lateness depended on nothing a caller could see.
//!
//! Every check below states the value it expects at a named frame rather than a property
//! of the signal, because the defect this task closes moves an edge by fewer than 64
//! frames and any weaker assertion passes with the edge in the wrong place.

mod common;

use common::{OUTPUT, SOURCE, profile};
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::offline::{OfflineEvent, render_offline, render_offline_reporting};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, Frequency, NormalizedLevel, ParameterValue,
    Resonance, Seconds,
};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime};

const ENVELOPE: NodeId = NodeId::new(11);
const AMPLIFIER: NodeId = NodeId::new(12);
const FILTER: NodeId = NodeId::new(13);

/// A quantum, as a frame index, so an offset can be written as `Q + k`.
const Q: u64 = QUANTUM_FRAMES as u64;

/// A gated constant: the sharpest instrument this phase has for *where* an edge landed.
///
/// Every segment is instantaneous and the sustain level is exactly one, so the rendered
/// signal is `0.0` before the note and `1.0` from the note's own sample onward — with no
/// ramp to hide a one-frame error inside and no rounding to force a tolerance. A gate
/// applied at a quantum boundary instead of at its sample changes up to 63 exact values.
fn gated_constant() -> GraphIr {
    GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).expect("not negative"),
                decay: Seconds::new(0.0).expect("not negative"),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.0).expect("not negative"),
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
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(common::compiled_notes(16))
        .build()
        .expect("a readable plan")
}

/// The complete Phase 2 voice: a sine through a low-pass into an amplifier the envelope
/// drives, into the output.
///
/// This is the path the phase exists to render — the master plan's "note events, an
/// envelope, an oscillator, a filter, an amplifier, an output" with every node the crate
/// has, and with the note as the only thing that starts it.
fn voice() -> GraphIr {
    GraphIr::builder()
        .node(
            SOURCE,
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
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.002).expect("not negative"),
                decay: Seconds::new(0.010).expect("not negative"),
                sustain: NormalizedLevel::new(0.6).expect("within range"),
                release: Seconds::new(0.020).expect("not negative"),
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
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
        .declaring(common::compiled_notes(16))
        .build()
        .expect("a readable plan")
}

/// A note edge on the plan's one playable node.
fn note(plan: &CompiledPlan, at: u64, on: bool) -> OfflineEvent {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("an envelope is a node a note can be sent to");
    // A compiled list names the node on **both** edges — that is how stamping pairs them —
    // while the stamped event names it on the on edge alone, per `SOUND-INV-017`.
    let payload = if on {
        CompiledPayload::NoteOn { slot }
    } else {
        CompiledPayload::NoteOff { slot }
    };
    OfflineEvent::new(SampleTime::new(at), payload)
}

/// The first frame whose value is not exactly zero, if any.
fn first_sounding(rendered: &[f32]) -> Option<usize> {
    rendered.iter().position(|sample| *sample != 0.0)
}

#[test]
fn a_note_on_takes_effect_at_its_declared_sample() {
    // Deliberately not a multiple of `Q`: the whole defect is that a mid-quantum edge used
    // to be rounded up to the boundary that follows it, and an edge at offset 0 cannot
    // tell the two behaviours apart. 36 frames into quantum 2.
    const AT: u64 = 2 * Q + 36;

    let plan = common::admit(&gated_constant(), profile(256, ChannelLayout::Mono));
    let events = [note(&plan, AT, true)];
    let rendered =
        render_offline(plan, FrameCount::new(512), PlanPosition::ZERO, &events).expect("renders");

    for (frame, sample) in rendered.iter().enumerate() {
        let expected = if (frame as u64) < AT { 0.0 } else { 1.0 };
        assert_eq!(
            *sample, expected,
            "frame {frame} of a note at sample {AT} is {sample} rather than {expected}"
        );
    }
    assert_eq!(
        first_sounding(&rendered),
        Some(AT as usize),
        "the note has to start on the sample it named, not on a quantum boundary"
    );
}

#[test]
fn a_note_off_takes_effect_at_its_declared_sample() {
    const ON: u64 = Q + 5;
    const OFF: u64 = 4 * Q + 51;

    let plan = common::admit(&gated_constant(), profile(256, ChannelLayout::Mono));
    let events = [note(&plan, ON, true), note(&plan, OFF, false)];
    let rendered =
        render_offline(plan, FrameCount::new(512), PlanPosition::ZERO, &events).expect("renders");

    for (frame, sample) in rendered.iter().enumerate() {
        let held = (ON..OFF).contains(&(frame as u64));
        let expected = if held { 1.0 } else { 0.0 };
        assert_eq!(
            *sample, expected,
            "frame {frame} between a note at {ON} and its release at {OFF} is {sample} \
             rather than {expected}"
        );
    }
}

#[test]
fn two_note_edges_in_one_quantum_both_take_effect() {
    // A note let go and played again inside 1.33 ms at 48 kHz. Both are edges at their own
    // samples: an implementation that keeps one pending edge per node loses the first, and
    // one that collapses a quantum's edges onto its boundary loses both positions.
    const ON: u64 = Q;
    const OFF: u64 = 3 * Q + 20;
    const AGAIN: u64 = 3 * Q + 27;

    let plan = common::admit(&gated_constant(), profile(256, ChannelLayout::Mono));
    let events = [
        note(&plan, ON, true),
        note(&plan, OFF, false),
        note(&plan, AGAIN, true),
    ];
    let rendered =
        render_offline(plan, FrameCount::new(512), PlanPosition::ZERO, &events).expect("renders");

    for (frame, sample) in rendered.iter().enumerate() {
        let frame = frame as u64;
        let silent = frame < ON || (OFF..AGAIN).contains(&frame);
        let expected = if silent { 0.0 } else { 1.0 };
        assert_eq!(
            *sample, expected,
            "frame {frame} of a retrigger between {OFF} and {AGAIN} is {sample} rather \
             than {expected}"
        );
    }
}

#[test]
fn an_edge_mid_ramp_starts_from_the_level_that_frame_would_have_had() {
    // The other half of "at its declared sample": the edge has to be applied to the level
    // the signal is **at**, not to the one it was at a frame ago. During a ramp those
    // differ by one step, because the level a quantum stores is the one its next sample
    // will have — the counter has already moved past the sample last written.
    //
    // Every other check in this file uses instantaneous segments, where the level is
    // exactly 0 or exactly the sustain level and the two readings agree. So does every
    // fixture in `layout_baseline`, whose edges are all on quantum boundaries where they
    // agree by construction. This is the case that separates them.
    //
    // A 256-frame attack from silence writes `f / 256` at frame `f`, so the release
    // beginning at frame `AT` has to start from exactly `AT / 256` — a value the release's
    // own first sample carries, since a segment's first frame is the level it inherited.
    const ATTACK: u64 = 256;
    const AT: u64 = 2 * Q + 36;

    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(ATTACK as f32 / 48_000.0).expect("not negative"),
                decay: Seconds::new(0.0).expect("not negative"),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(ATTACK as f32 / 48_000.0).expect("not negative"),
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
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(common::compiled_notes(16))
        .build()
        .expect("a readable plan");

    let plan = common::admit(&ir, profile(256, ChannelLayout::Mono));
    let events = [note(&plan, 0, true), note(&plan, AT, false)];
    let rendered =
        render_offline(plan, FrameCount::new(512), PlanPosition::ZERO, &events).expect("renders");

    // The attack itself, so the expected level at the edge is read off a checked ramp
    // rather than asserted twice.
    for frame in 0..AT {
        let expected = frame as f32 / ATTACK as f32;
        let sample = rendered.get(frame as usize).copied().unwrap_or(-1.0);
        assert!(
            (sample - expected).abs() < 1e-6,
            "frame {frame} of the attack is {sample} rather than {expected}"
        );
    }

    let at_edge = rendered.get(AT as usize).copied().unwrap_or(-1.0);
    let expected = AT as f32 / ATTACK as f32;
    assert!(
        (at_edge - expected).abs() < 1e-6,
        "the release starts at {at_edge} rather than at the level frame {AT} would have          had, {expected}; one step back is {}",
        (AT - 1) as f32 / ATTACK as f32
    );
    assert!(
        rendered
            .get(AT as usize..)
            .expect("the render reaches the release")
            .windows(2)
            .all(|pair| pair[1] <= pair[0]),
        "and it falls from there without a step back up"
    );
}

#[test]
fn a_note_edge_survives_any_host_block_partition() {
    // ADR-0001's reason for a fixed quantum, applied to the thing this task placed: an
    // edge at its declared sample is only *at* that sample if the caller's block pattern
    // cannot move it. Three partitions of the same render — whole quanta, several quanta
    // at a time, and a size that is not a multiple of `Q` at all, so every block boundary
    // falls somewhere different.
    const ON: u64 = 2 * Q + 17;
    const OFF: u64 = 6 * Q + 3;

    let mut renders = Vec::new();
    for block in [64_u64, 256, 250] {
        let plan = common::admit(&gated_constant(), profile(block, ChannelLayout::Mono));
        let events = [note(&plan, ON, true), note(&plan, OFF, false)];
        renders.push(
            render_offline(plan, FrameCount::new(1_024), PlanPosition::ZERO, &events)
                .expect("renders"),
        );
    }

    let reference = renders.first().expect("three renders").clone();
    for (index, rendered) in renders.iter().enumerate().skip(1) {
        assert_eq!(
            *rendered, reference,
            "partition {index} rendered a different signal from the whole-quantum one"
        );
    }
    assert_eq!(
        first_sounding(&reference),
        Some(ON as usize),
        "and all three put the note on the sample it named"
    );
}

#[test]
fn a_gate_addressed_as_a_parameter_lands_on_the_same_sample_as_a_note() {
    // ADR-0001 clause 14 splits on the **effect**, not on the message. A gate is
    // sample-positioned, so addressing it as a parameter must not be a way to get the
    // boundary behaviour back — otherwise the clause is satisfiable by choosing a payload,
    // which is not a contract at all.
    const AT: u64 = 3 * Q + 41;

    let plan = common::admit(&gated_constant(), profile(256, ChannelLayout::Mono));
    let as_note = [note(&plan, AT, true)];
    let slot = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_GATE)
        .expect("the envelope still declares an addressable gate");
    let as_parameter = [OfflineEvent::new(
        SampleTime::new(AT),
        CompiledPayload::SetParameter {
            slot,
            value: ParameterValue::new(1.0).expect("finite"),
        },
    )];

    let played = render_offline(
        plan.clone(),
        FrameCount::new(512),
        PlanPosition::ZERO,
        &as_note,
    )
    .expect("renders");
    let automated = render_offline(
        plan,
        FrameCount::new(512),
        PlanPosition::ZERO,
        &as_parameter,
    )
    .expect("renders");

    assert_eq!(
        played, automated,
        "the same gate reached by two payloads rendered two different signals"
    );
    assert_eq!(
        first_sounding(&played),
        Some(AT as usize),
        "and both put the edge on the sample it named"
    );
}

#[test]
fn the_complete_voice_renders_from_a_note_edge() {
    // The phase's deliverable: an oscillator, a filter, an envelope, an amplifier and an
    // output, silent until a note arrives and started by nothing else.
    //
    // The exact-value checks are the two this path can make honestly. **Before** the note
    // the amplifier is multiplying by an envelope at exactly zero, so every frame is
    // exactly zero however the filter is ringing — that is the assertion the sample
    // position rests on. **After** it, the first sounding frame is not pinned to `ON`,
    // because the attack's first frame is the level it started from and the oscillator's
    // own sample at that instant may be near a zero crossing; what is checked instead is
    // that the voice sounds within a bounded window and that nothing sounds before it.
    const ON: u64 = 2 * Q + 29;
    const OFF: u64 = 20 * Q + 11;
    /// The attack is 2 ms — 96 frames at 48 kHz — and 220 Hz has a zero crossing every
    /// 109. One period past the attack is a window the voice cannot stay silent through
    /// unless the note never started.
    const WINDOW: u64 = 320;

    let plan = common::admit(&voice(), profile(256, ChannelLayout::Mono));
    let events = [note(&plan, ON, true), note(&plan, OFF, false)];
    let rendered = render_offline(plan, FrameCount::new(64 * Q), PlanPosition::ZERO, &events)
        .expect("renders");

    for (frame, sample) in rendered.iter().enumerate().take(ON as usize) {
        assert_eq!(
            *sample, 0.0,
            "frame {frame} sounds before the note at {ON}, at {sample}"
        );
    }
    let sounded = first_sounding(&rendered).expect("the voice has to sound at all");
    assert!(
        (ON as usize..(ON + WINDOW) as usize).contains(&sounded),
        "the voice first sounds at frame {sounded}, which is not inside the {WINDOW}-frame \
         window after the note at {ON}"
    );

    // And it stops: the release is 20 ms — 960 frames — so a full second past the note-off
    // is far beyond it. A voice that kept sounding would mean the note-off never arrived.
    let tail = rendered
        .get((OFF as usize).saturating_add(2_000)..)
        .expect("the render is longer than the release");
    assert!(
        tail.iter().all(|sample| *sample == 0.0),
        "the voice is still sounding well past its release"
    );
}

#[test]
fn a_note_addressed_to_a_node_that_cannot_be_played_does_not_resolve() {
    // The refusal happens where a caller can be told about it. A node with no note control
    // has no slot, so an event that would have done nothing cannot be built at all —
    // which is the same rule `resolve_parameter` follows for an address the plan lacks.
    let plan = common::admit(&gated_constant(), profile(256, ChannelLayout::Mono));
    assert!(
        plan.resolve_note(SOURCE).is_none(),
        "a constant is not a node a note means anything to"
    );
    assert!(
        plan.resolve_note(AMPLIFIER).is_none(),
        "neither is an amplifier"
    );
    assert_eq!(
        plan.note_addresses().len(),
        1,
        "and the one playable node in this plan is its envelope"
    );
}

#[test]
fn every_event_of_a_sorted_list_is_presented_across_an_uneven_partition() {
    // ADR-0043's named offline obligation, which `NOW.md` assigns to Phase 3's exit work:
    // *prove the stamp-window selector cannot present a late event, or window by clamped
    // render position.*
    //
    // `offline.rs`'s `events_for` selects the slice by the event's **stamp** quantum while
    // the renderer admits by **position**, and its `start` predicate skips anything whose
    // quantum precedes the call's first. Its premise is that the offline path — a sorted
    // list walked with a monotone clock — cannot produce such an event. That premise is what
    // this test discharges, and it is a premise about **tiling**: consecutive calls must
    // cover contiguous quantum ranges with no gap and no overlap, or an event falls between
    // two windows and is skipped with nothing reporting it.
    //
    // **Rendered samples alone cannot discharge it, and that was this test's own gap.**
    // They catch a *gap* — an event that falls between two windows is never applied, and the
    // audio says so. They do not catch an *overlap*: a selector re-presenting a quantum it
    // already covered would apply those events twice, and a repeated gate edge is
    // idempotent, so the samples can be identical. The instrument for an overlap is the
    // renderer's **late** counter, because a re-presented event is behind the clock by then
    // and takes ADR-0043's preserving clamp. An independent review found the omission; the
    // assertion below is what closes it, and `render_offline_reporting` exists because the
    // counters were otherwise unobservable on this path.
    //
    // The block size is what puts the premise under strain. `render_offline` renders in
    // blocks of the plan's maximum block size, and 200 is deliberately **not** a multiple of
    // `Q`, so the carry leaves a different number of quanta due on successive calls and the
    // window is a different width each time. A tiling that only works on an aligned
    // partition fails here.
    //
    // The falsifier is the output itself: `gated_constant` renders exactly 1.0 while held and
    // 0.0 otherwise, so a single skipped edge is a visible run of wrong samples rather than a
    // statistical difference.
    const FRAMES: u64 = 2048;
    let edges: [(u64, u64); 6] = [
        (0, 37),
        (Q + 1, Q + 63),
        (2 * Q + 63, 3 * Q),
        (5 * Q + 11, 9 * Q + 12),
        (17 * Q, 17 * Q + 1),
        (23 * Q + 60, 31 * Q + 3),
    ];

    let plan = common::admit(&gated_constant(), profile(200, ChannelLayout::Mono));
    let mut events = Vec::new();
    for (on, off) in edges {
        events.push(note(&plan, on, true));
        events.push(note(&plan, off, false));
    }
    let (rendered, report) =
        render_offline_reporting(plan, FrameCount::new(FRAMES), PlanPosition::ZERO, &events)
            .expect("renders");

    // The gap half: an edge lost between two windows never sounds.
    for (frame, sample) in rendered.iter().enumerate() {
        let held = edges
            .iter()
            .any(|(on, off)| (*on..*off).contains(&(frame as u64)));
        let expected = if held { 1.0 } else { 0.0 };
        assert_eq!(
            *sample, expected,
            "frame {frame} is {sample} rather than {expected}: an edge was lost between two \
             selection windows, so the offline selector does not tile the quantum range"
        );
    }

    // **The overlap half, which the samples above cannot see.** A selector re-presenting a
    // quantum it already covered applies those events a second time, behind the clock, so
    // ADR-0043's preserving clamp moves and counts them — while the audio stays identical,
    // because a repeated gate edge is idempotent. A zero here is therefore the direct
    // statement of the bullet's first branch: over a partition that strains the tiling, the
    // selector presented nothing late.
    assert_eq!(
        report.late_events(),
        0,
        "the offline selector presented {} late events, so it does not window by clamped \
         render position and its premise does not hold either",
        report.late_events()
    );
}
