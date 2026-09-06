//! `SOUND-INV-023` in the renderer: a parameter's layers reach a kernel composed, once.
//!
//! Inside the crate because the modulation layer's only writer before Phase 7 is the
//! crate-private seam `PreparedRenderer::modulate`, and the observable that settles a pitch
//! exactly — the oscillator's state — is crate-private too. Each render test is one
//! falsifier: a write path that hands a kernel the caller's value instead of the slot's
//! resolved one fails it, and each path — a quantum-rate `apply`, a sample-positioned
//! `SetParameter`, a note's magnitude, an activation's catch-up — has its own.

use crate::compile::{RenderConfig, compile};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain, parameters,
};
use crate::node::kernels::{ENVELOPE_GATE, SINE_FREQUENCY};
use crate::node::{ModulationLaw, ModulationSum, ParameterUnit};
use crate::plan::{CompiledPlan, ParameterSlot};
use crate::profile::HostProfile;
use crate::publish::PublicationArbiter;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, Frequency, HeldNoteCount, KeyIdentity, NormalizedLevel,
    NoteVelocity, ParameterValue, SampleRate, Seconds,
};
use crate::render::slot::SlotState;
use crate::render::{AudioBlockMut, PreparedRenderer};
use crate::schedule::{AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent};
use crate::stream::{ActivationRequest, StreamControl};
use crate::time::{PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const OSCILLATOR: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);
const Q: u64 = QUANTUM_FRAMES as u64;
const BLOCK: usize = 256;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);

fn value(raw: f32) -> ParameterValue {
    ParameterValue::new(raw).expect("finite")
}

fn sum(raw: f32) -> ModulationSum {
    ModulationSum::new(raw).expect("finite")
}

// --- the laws, against the record's arithmetic -------------------------------------------

#[test]
fn each_law_resolves_as_adr_0007_states_it() {
    // One figure per law, chosen so a neighbouring law gives a different answer: the
    // semitone law at +12 doubles where the decibel law at +12 gives ×3.98, and the
    // normalized clamp at 0.7 + 0.5 stops at 1 where the physical law gives 1.2.
    let cases: [(ModulationLaw, f32, f32, f32); 10] = [
        (ModulationLaw::NormalizedAdditive, 0.7, 0.5, 1.0),
        (ModulationLaw::NormalizedAdditive, 0.2, -0.5, 0.0),
        (ModulationLaw::BipolarAdditive, 0.5, 0.75, 1.0),
        (ModulationLaw::BipolarAdditive, -0.5, -0.75, -1.0),
        (ModulationLaw::SemitoneAdditive, 440.0, 12.0, 880.0),
        (ModulationLaw::DecibelAdditive, 1.0, -20.0, 0.1),
        (ModulationLaw::PhysicalLinearAdditive, 0.7, 0.5, 1.2),
        (ModulationLaw::MultiplicativeGain, 0.5, 0.5, 0.25),
        (ModulationLaw::ThresholdedBoolean, 0.3, 0.2, 1.0),
        (ModulationLaw::NotModulatable, 0.3, 5.0, 0.3),
    ];
    for (law, base, m, expected) in cases {
        let resolved = law.resolve(value(base), sum(m));
        assert!(
            (resolved - expected).abs() < 1e-5,
            "{law:?}: resolve({base}, {m}) = {resolved}, the record says {expected}"
        );
    }
    // The threshold is at exactly one half, and the resolved value is exactly a boolean.
    assert_eq!(
        ModulationLaw::ThresholdedBoolean.resolve(value(0.49), sum(0.0)),
        0.0
    );
    assert_eq!(
        ModulationLaw::ThresholdedBoolean.resolve(value(0.5), sum(0.0)),
        1.0
    );
}

#[test]
fn every_law_resolves_to_its_base_at_its_identity() {
    // The property the bit-identical renders rest on: an unmodulated slot hands the kernel
    // exactly what was written, under every law — including the two exponential ones,
    // whose `2^0` and `10^0` are exactly one.
    let laws = [
        ModulationLaw::NormalizedAdditive,
        ModulationLaw::BipolarAdditive,
        ModulationLaw::SemitoneAdditive,
        ModulationLaw::DecibelAdditive,
        ModulationLaw::PhysicalLinearAdditive,
        ModulationLaw::MultiplicativeGain,
        ModulationLaw::NotModulatable,
    ];
    // Not a negative zero: an additive law's `b + 0` turns it positive, which is the one
    // bit a base can lose at the identity. No render writes one — a gate is thresholded and
    // a velocity is a note's — and the multiplicative laws keep it, as the mutation check
    // against `SemitoneAdditive` below shows.
    for law in laws {
        for base in [0.0_f32, 0.5, -1.0, 0.999_999_9, 12_345.678, -0.25] {
            // Inside the law's own domain: a level's law holds `[0, 1]` and a bipolar
            // value's `[−1, 1]`, and a base outside them is clamped rather than kept —
            // which is the type's clamp doing its job, not the identity moving a value.
            let in_domain = match law {
                ModulationLaw::NormalizedAdditive => (0.0..=1.0).contains(&base),
                ModulationLaw::BipolarAdditive => (-1.0..=1.0).contains(&base),
                _ => true,
            };
            if !in_domain {
                continue;
            }
            assert_eq!(
                law.resolve(value(base), law.identity()).to_bits(),
                base.to_bits(),
                "{law:?} moves {base} at its identity"
            );
        }
    }
    assert_eq!(
        ModulationLaw::SemitoneAdditive
            .resolve(value(-0.0), sum(0.0))
            .to_bits(),
        (-0.0_f32).to_bits()
    );
    // The boolean law's identity holds the base only as a boolean, which is what its
    // domain is: exactly zero and exactly one are fixed points.
    for base in [0.0_f32, 1.0] {
        assert_eq!(
            ModulationLaw::ThresholdedBoolean
                .resolve(value(base), ModulationLaw::ThresholdedBoolean.identity()),
            base
        );
    }
}

#[test]
fn an_override_replaces_the_base_and_leaves_the_modulation_in_force() {
    // Clause 2's last sentence, on the slot alone: a replacement layer replaces the base
    // only, so an automated pitch still bends.
    let mut slot = SlotState::prepared(
        ModulationLaw::SemitoneAdditive,
        ParameterUnit::Hertz,
        crate::node::Smoothing::None,
        value(440.0),
    );
    assert_eq!(slot.resolved(), value(440.0));
    assert_eq!(slot.modulate(sum(12.0)), value(880.0));
    assert_eq!(slot.write_override(value(220.0)), value(440.0));
    // And the type's clamp follows the law: a level pushed past one by an override is
    // held at one, however the law left it.
    let mut level = SlotState::prepared(
        ModulationLaw::NormalizedAdditive,
        ParameterUnit::NormalizedLevel,
        crate::node::Smoothing::None,
        value(0.5),
    );
    assert_eq!(level.modulate(sum(0.3)), value(0.8));
    assert_eq!(level.write_override(value(0.9)), value(1.0));
    // A prepared slot starts at its law's identity, not at zero: the one law whose identity
    // is one would otherwise silence its base until the first modulator wrote it.
    let gain = SlotState::prepared(
        ModulationLaw::MultiplicativeGain,
        ParameterUnit::LinearAmplitude,
        crate::node::Smoothing::None,
        value(0.5),
    );
    assert_eq!(gain.resolved(), value(0.5));
}

#[test]
fn a_law_that_leaves_the_finite_domain_saturates_rather_than_poisoning_state() {
    // The one way composition can produce a non-finite value: an exponential law fed a sum
    // nothing bounds yet. The slot hands the kernel the widest finite value, and a zero base
    // — `0 × ∞` — stays zero, which every law agrees on.
    let mut slot = SlotState::prepared(
        ModulationLaw::SemitoneAdditive,
        ParameterUnit::Hertz,
        crate::node::Smoothing::None,
        value(440.0),
    );
    assert_eq!(slot.modulate(sum(1.0e9)), value(f32::MAX));
    assert_eq!(slot.write_override(value(-1.0)), value(f32::MIN));
    assert_eq!(slot.write_override(value(0.0)), value(0.0));
}

// --- the segment (SOUND-INV-024) ------------------------------------------------------------

#[test]
fn a_segment_reads_past_its_start_on_its_first_frame_and_exactly_its_target_on_its_last() {
    // ADR-0006 as SOUND-INV-024 states it: frame `k` of `N` reads `start + (target − start)
    // × (k + 1) / N`. So the first frame is already past the start, the last is exactly the
    // target — not the sum's rounding of it — and every later frame holds it.
    let mut slot = SlotState::prepared(
        ModulationLaw::DecibelAdditive,
        ParameterUnit::LinearAmplitude,
        crate::node::Smoothing::None,
        value(0.0),
    );
    slot.smooth_over(10);
    assert_eq!(
        slot.write_override(value(1.0)),
        value(0.0),
        "a retarget reads the current value"
    );
    let mut frames = [0.0_f32; 16];
    slot.advance(&mut frames);
    assert!(
        frames[0] > 0.0,
        "the first frame reads past the start: {}",
        frames[0]
    );
    assert!(
        (frames[0] - 0.1).abs() < 1e-6,
        "the first frame reads one tenth of the way"
    );
    for pair in frames[..10].windows(2) {
        assert!(pair[1] > pair[0], "the segment is monotone: {frames:?}");
    }
    assert_eq!(frames[9], 1.0, "the last frame reads exactly the target");
    assert!(
        frames[10..].iter().all(|f| *f == 1.0),
        "every later frame holds it"
    );
    assert_eq!(slot.current(), value(1.0));

    // A `None` policy is a step read on its first frame.
    let mut step = SlotState::prepared(
        ModulationLaw::DecibelAdditive,
        ParameterUnit::LinearAmplitude,
        crate::node::Smoothing::None,
        value(0.0),
    );
    assert_eq!(step.write_override(value(1.0)), value(1.0));
    let mut frames = [0.0_f32; 4];
    step.advance(&mut frames);
    assert_eq!(frames, [1.0; 4]);
}

#[test]
fn a_retarget_mid_segment_continues_from_the_current_value_not_the_previous_target() {
    // ADR-0006 clause 3. Halfway toward 1.0 the slot is at 0.5; a write of 0.0 then starts
    // from 0.5 and reaches 0.0 over a whole new segment, never from the 1.0 it was heading
    // for — which would have made the next frame jump to 0.9.
    let mut slot = SlotState::prepared(
        ModulationLaw::DecibelAdditive,
        ParameterUnit::LinearAmplitude,
        crate::node::Smoothing::None,
        value(0.0),
    );
    slot.smooth_over(10);
    let _ = slot.write_override(value(1.0));
    let mut first = [0.0_f32; 5];
    slot.advance(&mut first);
    assert!(
        (slot.current().as_f32() - 0.5).abs() < 1e-6,
        "halfway: {:?}",
        slot.current()
    );
    assert_eq!(slot.write_override(value(0.0)), slot.current());
    let mut second = [0.0_f32; 10];
    slot.advance(&mut second);
    assert!(
        (second[0] - 0.45).abs() < 1e-6,
        "continues from 0.5 downward: {second:?}"
    );
    assert_eq!(second[9], 0.0, "and reaches the new target exactly");
}

#[test]
fn a_seeded_slot_takes_its_next_write_as_a_step_whatever_its_policy() {
    // SOUND-INV-024's last clause on the slot alone: an activation seeds every slot, so the
    // catch-up that follows lands with current equal to target and nothing remaining. The
    // seed is spent by that one write; the write after it ramps again.
    let mut slot = SlotState::prepared(
        ModulationLaw::DecibelAdditive,
        ParameterUnit::LinearAmplitude,
        crate::node::Smoothing::None,
        value(0.0),
    );
    slot.smooth_over(10);
    slot.seed();
    assert_eq!(
        slot.write_override(value(1.0)),
        value(1.0),
        "seeded: a step"
    );
    assert_eq!(
        slot.write_override(value(0.0)),
        value(1.0),
        "spent: a segment again"
    );
}

#[test]
fn the_kernel_reads_the_segment_per_frame_and_a_step_policy_renders_as_before() {
    // Through the renderer: a sine at amplitude 0 written to 1 at the boundary. Under the
    // declared `None` policy quantum 1 is at full amplitude from its first frame; under a
    // one-quantum segment (the test seam) its envelope rises across quantum 1 and is full
    // from quantum 2 — and the sine's own phase is untouched by either, which is what the
    // per-frame read from the slot's buffer buys over a state field the kernel read once.
    let plan = admit(&sine_alone(0.0));
    let amplitude = plan
        .resolve_parameter(OSCILLATOR, parameters::SINE_AMPLITUDE)
        .expect("the sine declares an amplitude");
    let write = [PlanEvent::new(
        PlanPosition::new(Q),
        CompiledPayload::SetParameter {
            slot: amplitude,
            value: ParameterValue::ONE,
        },
    )];
    let stepped = render(&plan, &[], &write, 4);
    let ramped = render_smoothed(&plan, amplitude, QUANTUM_FRAMES, &write, 4);
    // The renderer primes one quantum of silence, so plan quantum `n` is output quantum
    // `n + 1`: the write at `Q` takes effect at plan quantum 1, output quantum 2.
    let q = Q as usize;
    let (from, to) = (2 * q, 3 * q);
    assert!(stepped[..from].iter().all(|s| *s == 0.0) && ramped[..from].iter().all(|s| *s == 0.0));
    // Quantum 1: the step is a plain sine; the ramp is that sine scaled by (k + 1) / 64.
    for k in 0..q {
        let scale = (k + 1) as f32 / q as f32;
        let expected = stepped[from + k] * scale;
        assert!(
            (ramped[from + k] - expected).abs() < 1e-6,
            "frame {k} of the segment: {} against {expected}",
            ramped[from + k]
        );
    }
    // From quantum 2 on the two are identical: the segment ended exactly on its target.
    assert_eq!(&stepped[to..], &ramped[to..]);
    assert!(peak(&stepped[from..to]) > 0.5, "the step sounds at once");
}

#[test]
fn an_activation_never_ramps_even_under_a_smoothing_policy() {
    // The catch-up restores the amplitude a write before the destination set, and the
    // first quantum the new mapping governs reads it in force — not the first frame of a
    // segment toward it — even though the slot's policy would ramp any other write.
    let plan = admit(&sine_alone(0.0));
    let amplitude = plan
        .resolve_parameter(OSCILLATOR, parameters::SINE_AMPLITUDE)
        .expect("the sine declares an amplitude");
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    renderer.smooth_over(amplitude, QUANTUM_FRAMES);
    let mut arbiter = arbiter();
    let quiet = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");
    let events = vec![PlanEvent::new(
        PlanPosition::ZERO,
        CompiledPayload::SetParameter {
            slot: amplitude,
            value: ParameterValue::ONE,
        },
    )];
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
    let activation = control
        .plan_activation(
            &stream,
            ActivationRequest {
                at: SampleTime::new(4 * Q),
                position: PlanPosition::new(8 * Q),
                loop_interval: None,
            },
        )
        .expect("the seek builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");
    let out = drive(&mut scheduler, &mut renderer, &mut arbiter, 16 * Q as usize);
    // The renderer primes one quantum, so engine time `4Q` is output frame `5Q`.
    let q = Q as usize;
    let first = &out[5 * q..6 * q];
    assert!(
        out[..5 * q].iter().all(|s| *s == 0.0),
        "silent before the destination"
    );
    assert!(
        peak(first) > 0.9,
        "the first quantum after the activation peaks at {}, so the restored amplitude ramped \
         rather than landing in force",
        peak(first)
    );
}

// --- the renderer: every write path reaches the kernel through the slot -------------------

#[test]
fn a_quantum_rate_write_reaches_the_kernel_composed_with_the_modulation_in_force() {
    // The amplitude is the one quantum-rate control, under the decibel law. Modulated by
    // −20 dB the peak is a tenth; an override of 1.0 written on top is then a tenth of
    // one, not one — which is the falsifier for `apply` handing state the caller's value.
    let plan = admit(&sine_alone(0.5));
    let amplitude = plan
        .resolve_parameter(OSCILLATOR, parameters::SINE_AMPLITUDE)
        .expect("the sine declares an amplitude");

    let unmodulated = render(&plan, &[], &[], 8);
    let modulated = render(&plan, &[(amplitude, -20.0)], &[], 8);
    let overridden = render(
        &plan,
        &[(amplitude, -20.0)],
        &[PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::SetParameter {
                slot: amplitude,
                value: ParameterValue::ONE,
            },
        )],
        8,
    );
    let (loud, soft, written) = (peak(&unmodulated), peak(&modulated), peak(&overridden));
    assert!(
        (loud - 0.5).abs() < 1e-3,
        "the unmodulated sine peaks at {loud}"
    );
    assert!(
        (soft / loud - 0.1).abs() < 1e-3,
        "−20 dB on a peak of {loud} rendered {soft}"
    );
    assert!(
        (written / loud - 0.2).abs() < 1e-3,
        "an override of 1.0 under −20 dB rendered a peak of {written} against {loud}; the \
         override replaced the modulation instead of the base"
    );
}

#[test]
fn a_sample_positioned_write_and_a_notes_pitch_both_reach_the_kernel_composed() {
    // The frequency is sample-positioned and a pitch destination, so two paths write it:
    // `SetParameter` through the timed-control collection, and a note-on's key through the
    // magnitude expansion. Under +12 semitones each must land doubled — exactly, because
    // `2^(12/12)` is exactly two — and the oscillator's own state is where the kernel keeps
    // what it was handed.
    let plan = admit(&voice());
    let frequency = plan
        .resolve_parameter(OSCILLATOR, parameters::SINE_FREQUENCY)
        .expect("the sine declares a frequency");
    let note = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");

    let written = render_then_state(
        &plan,
        &[(frequency, 12.0)],
        &[PlanEvent::new(
            PlanPosition::new(Q + 5),
            CompiledPayload::SetParameter {
                slot: frequency,
                value: value(300.0),
            },
        )],
        OSCILLATOR,
        SINE_FREQUENCY,
    );
    assert_eq!(written, value(600.0), "a 300 Hz write under +12 st");

    let played = render_then_state(
        &plan,
        &[(frequency, 12.0)],
        &[PlanEvent::new(
            PlanPosition::new(Q + 5),
            CompiledPayload::NoteOn {
                slot: note,
                key: KeyIdentity::new(69).expect("A4"),
                velocity: NoteVelocity::FULL,
            },
        )],
        OSCILLATOR,
        SINE_FREQUENCY,
    );
    assert_eq!(
        played,
        ParameterValue::from_frequency(Frequency::new(880.0).expect("finite")),
        "A4 under +12 st"
    );
}

#[test]
fn a_gate_is_resolved_by_the_threshold_law_to_exactly_one_or_zero() {
    // The gate's declared law. A caller's 0.3 is below the threshold and releases; a 0.6 is
    // above it and holds — and what the envelope holds afterwards is exactly the boolean,
    // which is how the kernel's own above-zero test and the law's one-half agree.
    let plan = admit(&voice());
    let gate = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_GATE)
        .expect("the envelope declares a gate");
    for (raw, expected) in [(0.6_f32, ParameterValue::ONE), (0.3, ParameterValue::ZERO)] {
        let held = render_then_state(
            &plan,
            &[],
            &[PlanEvent::new(
                PlanPosition::new(Q + 1),
                CompiledPayload::SetParameter {
                    slot: gate,
                    value: value(raw),
                },
            )],
            ENVELOPE,
            ENVELOPE_GATE,
        );
        assert_eq!(held, expected, "a gate written as {raw}");
    }
}

#[test]
fn an_activations_catch_up_is_an_override_write_and_keeps_the_modulation_in_force() {
    // `SOUND-INV-023`'s last clause over `SOUND-INV-018`: the catch-up restores the pitch the
    // last note before the destination carried, and it restores it **as an override**, so a
    // modulation in force on the frequency is composed into the restored value rather than
    // lost to it. The falsifier: a catch-up that wrote the flattened figure straight into
    // state would leave the oscillator at the key's own frequency.
    let plan = admit(&voice());
    let frequency = plan
        .resolve_parameter(OSCILLATOR, parameters::SINE_FREQUENCY)
        .expect("the sine declares a frequency");
    let note = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    renderer.modulate(frequency, sum(12.0));
    let mut arbiter = arbiter();
    let quiet = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");
    let events = vec![
        PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::NoteOn {
                slot: note,
                key: KeyIdentity::new(69).expect("A4"),
                velocity: NoteVelocity::FULL,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(2 * Q),
            CompiledPayload::NoteOff {
                slot: note,
                key: KeyIdentity::new(69).expect("A4"),
            },
        ),
    ];
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
    let activation = control
        .plan_activation(
            &stream,
            ActivationRequest {
                at: SampleTime::new(4 * Q),
                position: PlanPosition::new(8 * Q),
                loop_interval: None,
            },
        )
        .expect("the seek builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");
    drive(&mut scheduler, &mut renderer, &mut arbiter, 32 * Q as usize);

    let restored = renderer
        .node_states
        .get(node_index(&plan, OSCILLATOR))
        .and_then(|state| state.control_value(SINE_FREQUENCY))
        .expect("the oscillator carries a frequency");
    assert_eq!(
        restored,
        ParameterValue::from_frequency(Frequency::new(880.0).expect("finite")),
        "the catch-up restored A4 under +12 st"
    );
}

#[test]
fn the_slots_the_renderer_holds_are_charged_to_the_mutable_state_row() {
    // `HOST` admits parameter slots through `mutable_state_bytes` rather than as a count of
    // their own, so what preparation allocates for them has to be inside what the report
    // charges — or a profile limit between the two admits a plan that allocates past it.
    // The falsifier is the term: the charge over every node exceeds the declared state
    // bytes by exactly the slot storage the renderer then holds.
    let ir = voice();
    let plan = admit(&ir);
    let (renderer, _) = open_and_render(&plan, &[], &[], 1);
    let held = renderer.slot_bytes_held();
    // One set of slots and buffers per voice instance of a voice-scope node (`P06-S001`).
    let voices = u64::from(ir.voice_instances().get());
    let slot_term: u64 = ir
        .nodes()
        .iter()
        .map(|node| {
            let instances = if node.scope() == ExecutionScope::Voice {
                voices
            } else {
                1
            };
            crate::node::slot_payload_bytes(node.kind()) * instances
        })
        .sum();
    assert_eq!(
        slot_term, held as u64,
        "the slot storage charged against what is held"
    );
    assert!(
        held > 0,
        "the voice declares writable controls, so it holds slots"
    );
    // And the row the report publishes carries the term: it exceeds the state payload
    // alone by exactly the slots.
    let state_only: u64 = ir
        .nodes()
        .iter()
        .map(|node| crate::node::state_payload_bytes(node.kind()))
        .sum();
    // The report's own row, over the records the lowering actually scheduled — the voice
    // sum's inserted steps included — rather than a restatement with zero inserted.
    let outcome = compile(&ir, &RenderConfig::new(profile()));
    let reported = match outcome
        .report()
        .row(crate::report::ResourceField::MutableStateBytes)
        .map(|row| row.requested())
    {
        Some(crate::report::ResourceAmount::Bytes(bytes)) => bytes.get(),
        other => panic!("the mutable row carries bytes, not {other:?}"),
    };
    let records = plan
        .ops()
        .iter()
        .filter(|op| matches!(op, crate::plan::PlanOp::Node(_)))
        .count() as u64;
    let table = renderer.ramp_table_bytes_held() as u64;
    assert_eq!(
        table,
        (records + 1) * crate::node::ramp_table_bytes_per_record(),
        "the run table is one entry per record plus a terminator"
    );
    assert_eq!(
        reported,
        records * crate::node::state_bytes_per_node() + slot_term + table,
        "the mutable row is one state record per scheduled record, the slots and the table"
    );
    assert!(state_only <= records * crate::node::state_bytes_per_node());
}

// --- fixtures -----------------------------------------------------------------------------

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        crate::time::FrameCount::new(BLOCK as u64),
        ChannelLayout::Mono,
    )
    .expect("valid harness profile")
}

fn admit(ir: &GraphIr) -> CompiledPlan {
    compile(ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect("the plan fits this profile")
}

fn arbiter() -> PublicationArbiter {
    PublicationArbiter::prepare(&profile()).expect("the publication store is preparable")
}

/// A sine straight into the output, at the given amplitude.
fn sine_alone(amplitude: f32) -> GraphIr {
    GraphIr::builder()
        .node(
            OSCILLATOR,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::new(amplitude).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan")
}

/// The smallest real voice, tuned, with one compiled producer of four notes.
fn voice() -> GraphIr {
    GraphIr::builder()
        .node(
            OSCILLATOR,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::UNITY,
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: crate::quantities::NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (OSCILLATOR, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .tuning(
            ExecutionScope::Voice,
            crate::tuning::PreparedTuning::equal_temperament().expect("12-TET prepares"),
        )
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(4),
                simultaneous_holds: EventCount::NONE,
            }],
            held_notes: HeldNoteCount::measured(4),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("a readable plan")
}

/// Open a stream, write the modulation seams, schedule the events, and render `quanta`.
fn open_and_render(
    plan: &CompiledPlan,
    modulation: &[(ParameterSlot, f32)],
    events: &[PlanEvent],
    quanta: u64,
) -> (PreparedRenderer, Vec<f32>) {
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    for (slot, raw) in modulation {
        renderer.modulate(*slot, sum(*raw));
    }
    let stream = AdmittedCompiledStream::admit(plan, events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let mut arbiter = arbiter();
    let out = drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        (quanta * Q) as usize,
    );
    (renderer, out)
}

fn render(
    plan: &CompiledPlan,
    modulation: &[(ParameterSlot, f32)],
    events: &[PlanEvent],
    quanta: u64,
) -> Vec<f32> {
    open_and_render(plan, modulation, events, quanta).1
}

/// Render with one slot's policy overridden to a segment of `frames`.
fn render_smoothed(
    plan: &CompiledPlan,
    slot: ParameterSlot,
    frames: u32,
    events: &[PlanEvent],
    quanta: u64,
) -> Vec<f32> {
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    renderer.smooth_over(slot, frames);
    let stream = AdmittedCompiledStream::admit(plan, events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let mut arbiter = arbiter();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        (quanta * Q) as usize,
    )
}

/// Render four quanta and read what one node's state holds for one control afterwards.
fn render_then_state(
    plan: &CompiledPlan,
    modulation: &[(ParameterSlot, f32)],
    events: &[PlanEvent],
    node: NodeId,
    control: crate::node::kernels::ControlIndex,
) -> ParameterValue {
    let (renderer, _) = open_and_render(plan, modulation, events, 4);
    renderer
        .node_states
        .get(node_index(plan, node))
        .and_then(|state| state.control_value(control))
        .expect("the node carries the control")
}

/// The scheduled index of a node, from the address table.
fn node_index(plan: &CompiledPlan, node: NodeId) -> usize {
    plan.parameter_addresses()
        .iter()
        .find(|address| address.node == node)
        .and_then(|address| plan.parameter_targets().get(address.slot.index()))
        .map(|target| target.node.index())
        .expect("the node declares a parameter and so has a slot")
}

fn drive(
    scheduler: &mut CompiledEventScheduler,
    renderer: &mut PreparedRenderer,
    arbiter: &mut PublicationArbiter,
    frames: usize,
) -> Vec<f32> {
    let mut out = Vec::new();
    let mut done = 0;
    while done < frames {
        let this = BLOCK.min(frames - done);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render(renderer, arbiter, output)
            .expect("the stream renders");
        out.extend_from_slice(&samples);
        done += this;
    }
    out
}

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .fold(0.0_f32, |held, sample| held.max(sample.abs()))
}
