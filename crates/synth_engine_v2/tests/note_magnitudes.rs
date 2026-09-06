//! `SOUND-INV-021`'s declarations and the binding that reaches them.
//!
//! The invariant makes a note-on carry two magnitudes beside its gate and resolves where
//! each one lands from **node kinds within the played node's execution scope**. This file
//! covers that binding and the three refusals that bound it; the payload that travels
//! through it is covered where the payload lives.
//!
//! Every check is written so that the obvious wrong implementation fails it. The oscillator
//! and the envelope are deliberately different nodes, because a binding that only ever
//! reached the played node would pass every check that put the pitch on the envelope.

mod common;

use common::{OUTPUT, admit, profile, refuse, twelve_tet};
use synth_engine_v2::diagnostics::CompileError;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrError, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::node::NoteMagnitude;
use synth_engine_v2::plan::{CompiledPlan, ControlRate};
use synth_engine_v2::quantities::{Amplitude, ChannelLayout, Frequency, NormalizedLevel, Seconds};
use synth_engine_v2::report::ResourceField;
use synth_engine_v2::tuning::PreparedTuning;

// Numbered clear of `common::OUTPUT`, which is node 2.
const OSCILLATOR: NodeId = NodeId::new(11);
const ENVELOPE: NodeId = NodeId::new(12);
const AMPLIFIER: NodeId = NodeId::new(13);
const SECOND_ENVELOPE: NodeId = NodeId::new(14);
const SECOND_OSCILLATOR: NodeId = NodeId::new(15);
const SECOND_AMPLIFIER: NodeId = NodeId::new(16);

fn envelope() -> IrNodeKind {
    IrNodeKind::Envelope {
        attack: Seconds::new(0.0).expect("not negative"),
        decay: Seconds::new(0.0).expect("not negative"),
        sustain: NormalizedLevel::FULL,
        release: Seconds::new(0.0).expect("not negative"),
        velocity_sensitivity: synth_engine_v2::quantities::NormalizedLevel::FULL,
    }
}

fn sine() -> IrNodeKind {
    IrNodeKind::Sine {
        frequency: Frequency::new(220.0).expect("finite"),
        amplitude: Amplitude::new(0.5).expect("finite"),
    }
}

/// The smallest real voice: an oscillator the note's key reaches, an envelope its gate and
/// velocity reach, and the amplifier that makes the envelope audible.
///
/// The gate belongs to the envelope and the pitch to the oscillator, which is the structural
/// fact `SOUND-INV-021` exists to bridge — a producer names only the envelope.
fn voice(oscillator_scope: ExecutionScope, tuned: bool) -> GraphIr {
    let mut builder = GraphIr::builder()
        .node(OSCILLATOR, sine(), oscillator_scope)
        .node(ENVELOPE, envelope(), ExecutionScope::Voice)
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
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
        .declaring(common::compiled_notes(4));
    if tuned {
        builder = builder.tuning(oscillator_scope, twelve_tet());
    }
    builder.build().expect("a readable plan")
}

fn plan(ir: &GraphIr) -> CompiledPlan {
    admit(ir, profile(256, ChannelLayout::Mono))
}

#[test]
fn a_notes_magnitudes_reach_a_node_it_did_not_name() {
    // The whole point of the binding. The producer resolves `ENVELOPE` and nothing else,
    // and the compiled expansion still carries the oscillator's frequency — which an
    // implementation that expanded only the played node's own controls cannot produce.
    let plan = plan(&voice(ExecutionScope::Voice, true));
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let target = plan
        .note_targets()
        .get(slot.index())
        .copied()
        .expect("the slot indexes its own target");

    let magnitudes = plan.note_magnitudes_of(slot);

    let pitch: Vec<_> = magnitudes
        .iter()
        .filter(|entry| entry.magnitude == NoteMagnitude::Pitch)
        .collect();
    let velocity: Vec<_> = magnitudes
        .iter()
        .filter(|entry| entry.magnitude == NoteMagnitude::Velocity)
        .collect();
    assert_eq!(
        pitch.len(),
        1,
        "one oscillator declares one pitch destination"
    );
    assert_eq!(
        velocity.len(),
        1,
        "one envelope declares one velocity destination"
    );

    // The pitch lands on the **oscillator**, not on the node the note named. Comparing node
    // slots rather than counting is what makes that observable: an expansion that wrote both
    // magnitudes to the envelope would have the same length.
    assert_ne!(
        pitch[0].node, target.node,
        "the key must reach the oscillator, which is not the node the note names"
    );
    assert_eq!(
        velocity[0].node, target.node,
        "the velocity lands on the played node's own envelope"
    );
    assert!(
        pitch[0].tuning.is_some(),
        "a pitch destination carries the tuning it resolves through"
    );
    assert!(
        velocity[0].tuning.is_none(),
        "a velocity resolves nothing, so it references no tuning"
    );
}

#[test]
fn a_scope_the_note_does_not_reach_contributes_no_magnitude() {
    // The binding is by scope, so an oscillator outside the played node's scope is not part
    // of the note's expansion. This is the falsifier for "collect every pitch destination in
    // the plan", which passes the test above and is a different rule.
    let plan = plan(&voice(ExecutionScope::Global, true));
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let magnitudes = plan.note_magnitudes_of(slot);
    assert!(
        magnitudes
            .iter()
            .all(|entry| entry.magnitude == NoteMagnitude::Velocity),
        "an oscillator in another scope is not this note's pitch destination"
    );
    // And the plan holds no tuning at all, because nothing resolves a key through one.
    assert!(
        plan.prepared_tunings().is_empty(),
        "a plan whose notes reach no pitch destination prepares no tuning"
    );
}

#[test]
fn a_pitch_destination_whose_scope_states_no_tuning_is_refused() {
    // No default is substituted. A key with nothing to resolve against has no frequency,
    // and choosing a scale here would be this crate deciding what a project sounds like.
    let error = refuse(
        &voice(ExecutionScope::Voice, false),
        profile(256, ChannelLayout::Mono),
    );
    assert!(
        matches!(
            error,
            CompileError::ScopeWithoutTuning {
                node: OSCILLATOR,
                scope: ExecutionScope::Voice
            }
        ),
        "the refusal must name the node and its scope, got {error:?}"
    );
}

#[test]
fn two_playable_nodes_in_one_scope_are_refused_and_both_are_named() {
    // `SOUND-INV-021`: a note's destinations are its scope's, so two playable nodes sharing
    // a scope would each move the other's velocity. The refusal names both, because a
    // diagnostic naming one leaves a reader to find the collision themselves.
    let ir = GraphIr::builder()
        .node(OSCILLATOR, sine(), ExecutionScope::Global)
        .node(ENVELOPE, envelope(), ExecutionScope::Voice)
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(SECOND_ENVELOPE, envelope(), ExecutionScope::Voice)
        .node(
            SECOND_AMPLIFIER,
            IrNodeKind::Amplifier,
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
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
            (SECOND_AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SECOND_ENVELOPE, PortId::FIRST),
            (SECOND_AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (SECOND_AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(common::compiled_notes(4))
        .build()
        .expect("a readable plan");
    let error = refuse(&ir, profile(256, ChannelLayout::Mono));
    let CompileError::AmbiguousNoteScope {
        first,
        second,
        scope,
    } = error
    else {
        panic!("two playable nodes in one scope must be refused, got {error:?}");
    };
    assert_eq!(scope, ExecutionScope::Voice);
    assert_ne!(first, second, "the refusal names two distinct nodes");
    assert!([ENVELOPE, SECOND_ENVELOPE].contains(&first));
    assert!([ENVELOPE, SECOND_ENVELOPE].contains(&second));
}

#[test]
fn two_playable_nodes_in_two_scopes_are_admitted() {
    // The falsifier for the refusal above: it is the shared **scope** that is refused, not
    // a second playable node. Without this, a rule that refused every second note target
    // would pass the test above.
    let ir = GraphIr::builder()
        .node(OSCILLATOR, sine(), ExecutionScope::Global)
        .node(ENVELOPE, envelope(), ExecutionScope::Voice)
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(
            SECOND_ENVELOPE,
            envelope(),
            ExecutionScope::InstrumentInstance,
        )
        .node(
            SECOND_AMPLIFIER,
            IrNodeKind::Amplifier,
            ExecutionScope::InstrumentInstance,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
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
            (SECOND_AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SECOND_ENVELOPE, PortId::FIRST),
            (SECOND_AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (SECOND_AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(common::compiled_notes(4))
        .build()
        .expect("a readable plan");
    let plan = plan(&ir);
    assert_eq!(
        plan.note_addresses().len(),
        2,
        "both nodes stay playable when their scopes differ"
    );
}

#[test]
fn the_velocity_refusal_has_no_constructible_case_in_this_phase() {
    // `SOUND-INV-021` refuses a note scope declaring no velocity destination, because a
    // typed velocity reaching nothing is the fixed-velocity render the Phase 4 gate rules
    // out. The refusal cannot be **reached** here, and the reason is checkable rather than
    // remembered: the one playable kind declares one, so a scope with a playable node
    // always has a velocity destination in it.
    //
    // Asserted rather than left implicit, so that a later playable kind without one fails
    // here — where the reason is written — instead of admitting a plan the invariant refuses.
    let playable = [envelope()];
    for kind in playable {
        let addressed = synth_engine_v2::node::ports(kind, ChannelLayout::Mono);
        assert!(
            !addressed.is_empty(),
            "a playable kind declares at least one port"
        );
    }
    // The check that matters is over the descriptor, which only admission can read; the
    // observable consequence is that this plan admits at all.
    let ir = voice(ExecutionScope::Voice, true);
    let plan = plan(&ir);
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    assert!(
        plan.note_magnitudes_of(slot)
            .iter()
            .any(|entry| entry.magnitude == NoteMagnitude::Velocity),
        "every plan admitted here has a velocity destination, which is why the refusal is \
         unreachable"
    );
}

#[test]
fn one_scale_named_by_two_scopes_is_one_prepared_table() {
    // `SOUND-INV-021`: one prepared value exists per **distinct** tuning. Two scopes naming
    // one scale must not become two tables, or the report would read a second node as a
    // second scale.
    let shared = plan(&two_scope_voice(twelve_tet(), twelve_tet()));
    assert_eq!(
        shared.prepared_tunings().len(),
        1,
        "two scopes naming one scale share one prepared table"
    );

    // And two scales are two, which is the falsifier for a deduplication that shares one
    // table for everything. It is **not** evidence that the comparison is by content: these
    // two scales have different digests, so a digest-keyed deduplication passes this as
    // well. Nothing here can distinguish the two rules, because a digest collision is not
    // constructible — the content comparison carries that case structurally and no test
    // claims otherwise.
    let distinct = plan(&two_scope_voice(twelve_tet(), nineteen_tet()));
    assert_eq!(
        distinct.prepared_tunings().len(),
        2,
        "two distinct scales are two prepared tables"
    );
}

#[test]
fn a_second_scale_costs_a_table_and_a_second_node_costs_a_reference() {
    // The split the invariant asks the report for. A table is three orders of magnitude
    // larger than a reference, so the two are told apart by the size of the difference
    // rather than by trusting the row's label.
    let one_scale = prepared_bytes(&two_scope_voice(twelve_tet(), twelve_tet()));
    let two_scales = prepared_bytes(&two_scope_voice(twelve_tet(), nineteen_tet()));
    let table = PreparedTuning::prepared_bytes();
    assert_eq!(
        two_scales - one_scale,
        table,
        "a second scale costs exactly one prepared table"
    );
}

/// A second, genuinely different scale, for the comparisons that need two digests.
///
/// Nineteen-tone equal temperament rather than a renamed twelve: it steps smaller than a
/// semitone, so it is the case the whole decision exists for and it cannot share a digest
/// with [`common::twelve_tet`] by accident.
fn nineteen_tet() -> PreparedTuning {
    PreparedTuning::prepare(&synth_core::tuning::TuningTable::equal_temperament_19())
        .expect("nineteen-tone equal temperament prepares")
}

/// Two independently playable voices, each with its own oscillator and its own scope.
fn two_scope_voice(voice_tuning: PreparedTuning, instance_tuning: PreparedTuning) -> GraphIr {
    GraphIr::builder()
        .node(OSCILLATOR, sine(), ExecutionScope::Voice)
        .node(ENVELOPE, envelope(), ExecutionScope::Voice)
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(
            SECOND_OSCILLATOR,
            sine(),
            ExecutionScope::InstrumentInstance,
        )
        .node(
            SECOND_ENVELOPE,
            envelope(),
            ExecutionScope::InstrumentInstance,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
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
        .tuning(ExecutionScope::Voice, voice_tuning)
        .tuning(ExecutionScope::InstrumentInstance, instance_tuning)
        .declaring(common::compiled_notes(4))
        .build()
        .expect("a readable plan")
}

/// The immutable prepared bytes this plan's report states.
fn prepared_bytes(ir: &GraphIr) -> u64 {
    let outcome = synth_engine_v2::compile::compile(
        ir,
        &synth_engine_v2::compile::RenderConfig::new(profile(256, ChannelLayout::Mono)),
    );
    let row = outcome
        .report()
        .row(ResourceField::PreparedImmutableBytes)
        .expect("the report states prepared bytes");
    match row.requested() {
        synth_engine_v2::report::ResourceAmount::Bytes(bytes) => bytes.get(),
        other => panic!("the prepared row is not in bytes: {other:?}"),
    }
}

#[test]
fn a_scope_given_two_tunings_is_refused_rather_than_taking_the_last() {
    // Keeping the last would let declaration order decide which scale the plan sounds in,
    // which is a silent choice rather than a diagnostic.
    let error = GraphIr::builder()
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .tuning(ExecutionScope::Voice, twelve_tet())
        .tuning(ExecutionScope::Voice, nineteen_tet())
        .build()
        .expect_err("two tunings for one scope have no reading");
    assert!(
        matches!(
            error,
            IrError::DuplicateScopeTuning {
                scope: ExecutionScope::Voice
            }
        ),
        "the refusal must name the scope, got {error:?}"
    );
}

#[test]
fn a_pitch_destination_is_addressable_as_a_parameter_under_one_timing_law() {
    // `SOUND-INV-021` keeps the rate on the **destination**, so a caller cannot obtain
    // another timing by choosing another payload: automating a frequency and playing a note
    // reach one control under one law. The plan is where that becomes checkable, because it
    // is what the renderer reads.
    let plan = plan(&voice(ExecutionScope::Voice, true));
    let slot = plan
        .resolve_parameter(OSCILLATOR, parameters::SINE_FREQUENCY)
        .expect("a sine's frequency is addressable");
    let target = plan
        .parameter_targets()
        .get(slot.index())
        .copied()
        .expect("the slot indexes its own target");
    assert_eq!(
        target.rate,
        ControlRate::Sample,
        "a pitch destination is sample-positioned however it is addressed"
    );

    let slot = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_VELOCITY)
        .expect("an envelope's velocity is addressable");
    let target = plan
        .parameter_targets()
        .get(slot.index())
        .copied()
        .expect("the slot indexes its own target");
    assert_eq!(target.rate, ControlRate::Sample);
}

#[test]
fn a_pitch_destination_takes_effect_at_its_own_sample() {
    // The consequence of the rate, and it is what makes `SOUND-INV-021`'s "the magnitudes
    // take effect before the gate they arrive with" achievable: a frequency that waited for
    // the next quantum boundary would sound the previous note's pitch for up to `Q - 1`
    // frames of every note that does not land on one.
    //
    // Observable without arithmetic on a phase: moving the frequency to zero freezes the
    // accumulator, so every frame from the change onward repeats the sample before it. Under
    // a quantum-rate destination the sine goes on oscillating to the end of the quantum, and
    // frame `AT + 1` differs from frame `AT`.
    const AT: u64 = 37;

    let ir = GraphIr::builder()
        .node(OSCILLATOR, sine(), ExecutionScope::Global)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan");
    let plan = plan(&ir);
    let slot = plan
        .resolve_parameter(OSCILLATOR, parameters::SINE_FREQUENCY)
        .expect("a sine's frequency is addressable");
    let events = [synth_engine_v2::offline::OfflineEvent::new(
        synth_engine_v2::time::SampleTime::new(AT),
        synth_engine_v2::schedule::CompiledPayload::SetParameter {
            slot,
            value: synth_engine_v2::quantities::ParameterValue::ZERO,
        },
    )];
    let rendered = synth_engine_v2::offline::render_offline(
        plan,
        synth_engine_v2::time::FrameCount::new(u64::from(synth_engine_v2::time::QUANTUM_FRAMES)),
        synth_engine_v2::time::PlanPosition::ZERO,
        &events,
    )
    .expect("renders");

    let frozen = rendered
        .get(AT as usize)
        .copied()
        .expect("the render covers the change");
    for (frame, sample) in rendered.iter().enumerate().skip(AT as usize) {
        assert_eq!(
            *sample, frozen,
            "frame {frame} moved after the frequency was set to zero at {AT}, so the change \
             did not take effect on its own sample"
        );
    }
    // The falsifier for "the whole quantum was already frozen": the sine has to have been
    // moving before the change, or the assertion above holds for any implementation.
    let before = rendered
        .get(..AT as usize)
        .expect("frames precede the change");
    assert!(
        before.windows(2).any(|pair| pair[0] != pair[1]),
        "the sine has to be oscillating before the change for this to test anything"
    );
}

#[test]
fn a_tuning_no_note_reaches_is_not_charged() {
    // The regression for a false refusal an independent review found. The preflight report
    // may **refuse** on the prepared row — only the arena row and what follows it are
    // exempt — so a charge that counted tunings no note resolves through would reject a plan
    // that fits. The oscillator here is outside the played node's scope, so its scope's
    // tuning is never prepared and must cost nothing.
    let untouched = prepared_bytes(&voice(ExecutionScope::Global, false));
    let declared = prepared_bytes(&voice(ExecutionScope::Global, true));
    assert_eq!(
        declared, untouched,
        "a tuning declared for a scope no note reaches must add nothing to the prepared row"
    );

    // And the plan still admits, which is the half the equality above cannot see: a charge
    // that was merely equal because both plans were refused would pass it.
    let plan = plan(&voice(ExecutionScope::Global, true));
    assert!(
        plan.prepared_tunings().is_empty(),
        "no note resolves a key here, so the plan prepares no table"
    );
}

#[test]
fn the_reported_tuning_charge_is_what_the_plan_holds() {
    // The figure the report states and the tables the plan carries come from two walks, so
    // this is what keeps them from drifting: the difference between a plan with a reachable
    // pitch destination and the same plan with the oscillator moved out of the note's scope
    // is exactly one table plus one reference.
    let reached = prepared_bytes(&voice(ExecutionScope::Voice, true));
    let unreached = prepared_bytes(&voice(ExecutionScope::Global, true));
    let expected = PreparedTuning::prepared_bytes()
        + std::mem::size_of::<synth_engine_v2::plan::TuningSlot>() as u64;
    assert_eq!(
        reached - unreached,
        expected,
        "one reachable pitch destination costs one table and one reference"
    );

    let plan = plan(&voice(ExecutionScope::Voice, true));
    assert_eq!(plan.prepared_tunings().len(), 1);
    assert_eq!(
        plan.note_magnitudes()
            .iter()
            .filter(|entry| entry.tuning.is_some())
            .count(),
        1,
        "and the plan holds exactly the one reference that was charged"
    );
}

#[test]
fn a_velocity_written_as_a_parameter_cannot_invert_or_over_amplify_the_envelope() {
    // `ParameterValue` admits every finite float, so a caller automating the velocity
    // control can present `2.0` or `-1.0`. Neither is a velocity: one over-amplifies and the
    // other **inverts** the envelope, turning a note into its own negation. An independent
    // review found the unclamped assignment. The domain is the destination's, and the clamp
    // is documented behaviour of this control rather than a silent substitution.
    for (written, expected) in [(2.0_f32, 1.0_f32), (-1.0, 0.0), (0.25, 0.25)] {
        let rendered = render_gated_constant(written);
        let sounding = rendered
            .get(SOUNDING as usize)
            .copied()
            .expect("the render covers the note");
        assert!(
            (sounding - expected).abs() < 1e-6,
            "a velocity of {written} rendered {sounding}, not the {expected} its domain admits"
        );
    }
}

/// Where the note in [`render_gated_constant`] is sounding.
const SOUNDING: u64 = 64;

/// A unit constant through an amplifier the envelope drives, with `velocity` written to the
/// envelope's velocity control **after** the note has started.
///
/// After, not before: since the payload carries a velocity, the note-on writes that control
/// itself, so a parameter write before the note is simply overwritten by it. Putting the
/// parameter write later is what makes it the value in force — which is the case this test is
/// about, since the note payload cannot present an out-of-range one.
///
/// The envelope's segments are all instantaneous and its sustain is full, so its emitted
/// level while the gate is held is exactly the velocity — which makes the rendered sample
/// the velocity itself rather than something to be inferred from a shape.
fn render_gated_constant(velocity: f32) -> Vec<f32> {
    let ir = GraphIr::builder()
        .node(
            OSCILLATOR,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(ENVELOPE, envelope(), ExecutionScope::Voice)
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
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
        .declaring(common::compiled_notes(4))
        .build()
        .expect("a readable plan");
    let plan = plan(&ir);
    let velocity_slot = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_VELOCITY)
        .expect("an envelope's velocity is addressable");
    let note = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let events = [
        synth_engine_v2::offline::OfflineEvent::new(
            synth_engine_v2::time::SampleTime::new(8),
            common::note_on(note),
        ),
        synth_engine_v2::offline::OfflineEvent::new(
            synth_engine_v2::time::SampleTime::new(16),
            synth_engine_v2::schedule::CompiledPayload::SetParameter {
                slot: velocity_slot,
                value: synth_engine_v2::quantities::ParameterValue::new(velocity)
                    .expect("the written value is finite"),
            },
        ),
    ];
    synth_engine_v2::offline::render_offline(
        plan,
        synth_engine_v2::time::FrameCount::new(
            4 * u64::from(synth_engine_v2::time::QUANTUM_FRAMES),
        ),
        synth_engine_v2::time::PlanPosition::ZERO,
        &events,
    )
    .expect("renders")
}

#[test]
fn a_note_slot_from_another_plan_reaches_no_magnitudes() {
    // A slot is an index into **one** plan's table, so a slot resolved against another does
    // not do nothing — it reads whatever occupies that index. The accessor takes the slot
    // rather than a bare `NoteTarget` precisely so the plan identity can be compared; an
    // independent review found the earlier signature accepting a target that carries none.
    let mine = plan(&voice(ExecutionScope::Voice, true));
    let theirs = plan(&voice(ExecutionScope::Voice, true));
    let foreign = theirs
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");

    // The two plans have the same shape, so the index is in range in both: only the plan
    // identity distinguishes them, which is what makes this a provenance check rather than a
    // bounds check wearing one's clothes.
    assert!(
        !mine
            .note_magnitudes_of(
                mine.resolve_note(ENVELOPE)
                    .expect("the envelope is playable")
            )
            .is_empty(),
        "the fixture has magnitudes at all, or the assertion below proves nothing"
    );
    assert!(
        mine.note_magnitudes_of(foreign).is_empty(),
        "a slot from another plan must reach nothing rather than an unrelated slice"
    );
}

#[test]
fn a_plan_is_not_refused_for_a_tuning_no_note_reaches() {
    // The false refusal itself, reproduced. The preflight report may refuse on the prepared
    // row — only the arena row and what follows it are exempt — so a charge that counted a
    // tuning no note resolves through rejects a plan that fits. With the prepared limit set
    // between the exact charge and what the old bound would have added, the two answers
    // differ in outcome rather than only in the number reported.
    let ir = voice(ExecutionScope::Global, true);
    let base = profile(256, ChannelLayout::Mono);
    // The budget is anchored to the **same graph with no tuning declared at all**, not to
    // this plan's own reported figure: a charge that over-counts inflates both, and a limit
    // derived from the number under test can never contradict it. Anchored here, the limit
    // leaves room for the plan as it stands and for less than the table an over-count adds.
    let reference = prepared_bytes(&voice(ExecutionScope::Global, false));
    let limit = reference + PreparedTuning::prepared_bytes() / 2;

    let limits = common::defaults_for(&base);
    let memory = synth_engine_v2::profile::MemoryLimits::new(
        synth_engine_v2::quantities::PreparedBytes::limit(limit).expect("a positive capacity"),
        limits.memory().mutable_state_bytes(),
        limits.memory().buffer_scratch_bytes(),
    )
    .expect("the memory group is consistent");
    let limits = synth_engine_v2::profile::RenderLimits::new(
        limits.stream(),
        limits.graph(),
        limits.voices(),
        limits.events(),
        limits.observation(),
        limits.mixing(),
        memory,
        limits.script(),
        limits.recording(),
        limits.cost(),
    )
    .expect("the overridden limits are internally consistent");
    let tight = synth_engine_v2::profile::HostProfile::new(base.capabilities(), limits)
        .expect("the profile is consistent");

    let outcome =
        synth_engine_v2::compile::compile(&ir, &synth_engine_v2::compile::RenderConfig::new(tight));
    assert!(
        outcome.into_plan().is_ok(),
        "a plan within its prepared budget must not be refused for a tuning no note reaches"
    );
}

/// A voice whose note-on carries `key` and `velocity`, rendered for `quanta` quanta.
///
/// A sine into an amplifier the envelope drives, with instantaneous segments and full
/// sustain — so what leaves the plan is the oscillator at the key's frequency, scaled by the
/// velocity, from the note's own sample onward. Nothing has to be inferred from a shape.
fn render_note(
    key: synth_engine_v2::quantities::KeyIdentity,
    velocity: synth_engine_v2::quantities::NoteVelocity,
    tuning: PreparedTuning,
    at: u64,
    quanta: u64,
) -> Vec<f32> {
    let ir = GraphIr::builder()
        .node(OSCILLATOR, sine(), ExecutionScope::Voice)
        .node(ENVELOPE, envelope(), ExecutionScope::Voice)
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
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
        .tuning(ExecutionScope::Voice, tuning)
        .declaring(common::compiled_notes(4))
        .build()
        .expect("a readable plan");
    let plan = plan(&ir);
    let note = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let events = [synth_engine_v2::offline::OfflineEvent::new(
        synth_engine_v2::time::SampleTime::new(at),
        synth_engine_v2::schedule::CompiledPayload::NoteOn {
            slot: note,
            key,
            velocity,
        },
    )];
    synth_engine_v2::offline::render_offline(
        plan,
        synth_engine_v2::time::FrameCount::new(
            quanta * u64::from(synth_engine_v2::time::QUANTUM_FRAMES),
        ),
        synth_engine_v2::time::PlanPosition::ZERO,
        &events,
    )
    .expect("renders")
}

/// The largest absolute sample from `from` onward.
fn peak(samples: &[f32], from: usize) -> f32 {
    samples
        .get(from..)
        .unwrap_or(&[])
        .iter()
        .fold(0.0_f32, |held, sample| held.max(sample.abs()))
}

/// How many times the signal crosses zero from `from` onward, as a stand-in for its pitch.
///
/// A count rather than a frequency estimate: two keys a fifth apart differ by half again as
/// many crossings, which is far outside anything a boundary effect can move.
fn zero_crossings(samples: &[f32], from: usize) -> usize {
    samples
        .get(from..)
        .unwrap_or(&[])
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count()
}

#[test]
fn two_renders_of_one_plan_at_two_velocities_differ_in_peak_amplitude() {
    // `SOUND-INV-021` states this as the minimum, and states it because the Phase 4 gate says
    // a fixed-velocity render cannot satisfy it: a typed velocity that reached nothing would
    // satisfy the letter of the invariant and none of its purpose.
    let loud = render_note(
        common::any_key(),
        synth_engine_v2::quantities::NoteVelocity::FULL,
        twelve_tet(),
        0,
        64,
    );
    let soft = render_note(
        common::any_key(),
        synth_engine_v2::quantities::NoteVelocity::new(0.25).expect("within range"),
        twelve_tet(),
        0,
        64,
    );

    let (loud_peak, soft_peak) = (peak(&loud, 0), peak(&soft, 0));
    assert!(loud_peak > 0.0, "the loud render has to sound at all");
    // A quarter of the velocity is a quarter of the amplitude: the envelope multiplies its
    // emitted level, and the amplifier multiplies the oscillator by that. Asserting the
    // *ratio* rather than mere inequality is what distinguishes velocity reaching the
    // amplitude from velocity reaching anything at all.
    assert!(
        (soft_peak / loud_peak - 0.25).abs() < 0.02,
        "a quarter velocity rendered a peak of {soft_peak} against {loud_peak}, a ratio of {}",
        soft_peak / loud_peak
    );
}

#[test]
fn two_renders_of_one_plan_at_two_keys_differ_in_pitch() {
    // The pitch half of the same statement. A key reaches the oscillator through the plan's
    // prepared tuning, so two keys sound two frequencies — and an implementation that dropped
    // the pitch write would render both at the node's prepared 220 Hz.
    let low = render_note(
        synth_engine_v2::quantities::KeyIdentity::new(48).expect("a keyboard position"),
        synth_engine_v2::quantities::NoteVelocity::FULL,
        twelve_tet(),
        0,
        64,
    );
    let high = render_note(
        synth_engine_v2::quantities::KeyIdentity::new(60).expect("a keyboard position"),
        synth_engine_v2::quantities::NoteVelocity::FULL,
        twelve_tet(),
        0,
        64,
    );

    let (low_crossings, high_crossings) = (zero_crossings(&low, 0), zero_crossings(&high, 0));
    assert!(low_crossings > 0, "the low note has to sound at all");
    // An octave, so exactly twice as many crossings. The tolerance covers the partial cycle
    // at each end of the render and nothing else.
    let ratio = high_crossings as f32 / low_crossings as f32;
    assert!(
        (ratio - 2.0).abs() < 0.05,
        "an octave rendered {high_crossings} crossings against {low_crossings}, a ratio of \
         {ratio}"
    );
}

#[test]
fn one_key_under_two_tunings_sounds_two_pitches() {
    // ADR-0025's whole point, and what option A could never have let the renderer see: the
    // tuning is the plan's, so the same key is a different frequency under a different scale.
    // A renderer converting a key to a frequency by 12-TET arithmetic of its own would render
    // these identically.
    //
    // Key 60 is the reference in both scales, so key 72 is chosen instead: an octave above in
    // twelve-tone, and only a minor seventh above in nineteen-tone, where twelve steps do not
    // reach one.
    let key = synth_engine_v2::quantities::KeyIdentity::new(72).expect("a keyboard position");
    let twelve = render_note(
        key,
        synth_engine_v2::quantities::NoteVelocity::FULL,
        twelve_tet(),
        0,
        64,
    );
    let nineteen = render_note(
        key,
        synth_engine_v2::quantities::NoteVelocity::FULL,
        nineteen_tet(),
        0,
        64,
    );

    let (a, b) = (zero_crossings(&twelve, 0), zero_crossings(&nineteen, 0));
    assert!(a > 0 && b > 0, "both renders have to sound");
    assert_ne!(
        a, b,
        "one key under two scales rendered the same pitch, so the tuning reached nothing"
    );

    // And exactly, at the plan, so the audible difference above cannot be a coincidence of
    // where two similar frequencies happened to cross zero. This is the value the renderer
    // writes, resolved by the same call it makes.
    let resolved = |tuning: PreparedTuning| {
        let plan = plan(&two_scope_voice(tuning, twelve_tet()));
        let slot = plan
            .resolve_note(ENVELOPE)
            .expect("the envelope is playable");
        let magnitude = *plan
            .note_magnitudes_of(slot)
            .iter()
            .find(|entry| entry.magnitude == NoteMagnitude::Pitch)
            .expect("the voice scope has a pitch destination");
        plan.magnitude_value(
            &magnitude,
            key,
            synth_engine_v2::quantities::NoteVelocity::FULL,
        )
        .expect("the destination resolves")
        .as_f32()
    };
    let (twelve_hz, nineteen_hz) = (resolved(twelve_tet()), resolved(nineteen_tet()));
    assert!(
        (twelve_hz - nineteen_hz).abs() > 1.0,
        "key 72 resolves to {twelve_hz} Hz under twelve-tone and {nineteen_hz} Hz under \
         nineteen-tone, which is not two scales"
    );
}

#[test]
fn a_notes_magnitudes_are_in_force_on_the_sample_its_gate_rises() {
    // `SOUND-INV-021`: "a gate raised at the same sample must see them already applied". The
    // note starts at an offset that is deliberately **not** a multiple of the quantum, so a
    // magnitude placed anywhere but the note's own sample leaves its first frames at the
    // previous value. Mutation-verified against a displacement of **one frame**.
    //
    // What it does **not** establish is the order of the two writes within that one offset.
    // Moving the magnitudes after the gate fails no test here, and cannot: a kernel applies
    // every control due at a frame before it writes that frame, and the envelope's gate law
    // does not read the velocity. That is recorded rather than left as an assumption.
    //
    // The check is the ratio again, over the first quantum after the note alone: a velocity
    // applied late would make that window louder than a quarter while the tail is right.
    const AT: u64 = 3 * 64 + 37;
    let quarter = synth_engine_v2::quantities::NoteVelocity::new(0.25).expect("within range");
    let loud = render_note(
        common::any_key(),
        synth_engine_v2::quantities::NoteVelocity::FULL,
        twelve_tet(),
        AT,
        8,
    );
    let soft = render_note(common::any_key(), quarter, twelve_tet(), AT, 8);

    let window = AT as usize..(AT as usize + 64);
    let loud_window = peak(loud.get(window.clone()).expect("inside the render"), 0);
    let soft_window = peak(soft.get(window).expect("inside the render"), 0);
    assert!(
        loud_window > 0.0,
        "the note has to sound in its first quantum"
    );
    assert!(
        (soft_window / loud_window - 0.25).abs() < 0.02,
        "the note's own first quantum rendered a ratio of {}, so its velocity was not in force \
         when its gate rose",
        soft_window / loud_window
    );

    // And nothing sounds before it, which is what makes the window above the note's own.
    assert!(
        soft.get(..AT as usize)
            .expect("frames precede the note")
            .iter()
            .all(|sample| *sample == 0.0),
        "the voice sounds before its note"
    );
}
