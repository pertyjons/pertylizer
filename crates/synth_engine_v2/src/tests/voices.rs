//! `P06-S001`: `N` voice instances of one prepared voice plan.
//!
//! Inside the crate to read the renderer's state table and the plan's prepared records
//! directly: the falsifiers are that two overlapping notes render as two voices, that a
//! release ends its own voice and no other, that the prepared data is shared and the state
//! is not, and that the report charges exactly what preparation holds per instance.

use crate::compile::{RenderConfig, compile};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain,
};
use crate::plan::{CompiledPlan, PlanOp};
use crate::profile::HostProfile;
use crate::publish::PublicationArbiter;
use crate::quantities::{
    Amplitude, ChannelLayout, EventCount, Frequency, HeldNoteCount, KeyIdentity, NormalizedLevel,
    NoteVelocity, SampleRate, Seconds,
};
use crate::render::{AudioBlockMut, PreparedRenderer};
use crate::report::{ResourceAmount, ResourceField};
use crate::schedule::{AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent};
use crate::stream::StreamControl;
use crate::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};

const OSCILLATOR: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);
const Q: u64 = QUANTUM_FRAMES as u64;
const BLOCK: usize = 256;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);

#[test]
fn two_overlapping_notes_render_as_the_sum_of_two_single_note_renders() {
    // The core claim, sample for sample. Each note alone sounds on one instance while the
    // others hold exact zeros; together they sound on two. The kernels are deterministic and
    // every instance starts from the same prepared record, so the second note renders the
    // same bytes whichever instance it lands on, and the voice sum adds them in one order.
    let plan = admit(&voice(4));
    let a = note(&plan, 60, 0, 12 * Q);
    let b = note(&plan, 67, 3 * Q + 5, 12 * Q);
    let alone_a = render(&plan, std::slice::from_ref(&a), 16);
    let alone_b = render(&plan, std::slice::from_ref(&b), 16);
    let together = render(&plan, &[a, b], 16);
    assert!(alone_a.iter().any(|s| *s != 0.0) && alone_b.iter().any(|s| *s != 0.0));
    let summed: Vec<f32> = alone_a.iter().zip(&alone_b).map(|(x, y)| x + y).collect();
    assert_eq!(together, summed, "two notes did not render as two voices");
    // And they are two: the overlap is louder than either alone at some sample.
    let peak = |s: &[f32]| s.iter().fold(0.0_f32, |m, x| m.max(x.abs()));
    assert!(peak(&together) > peak(&alone_a).max(peak(&alone_b)) * 1.05);
}

#[test]
fn a_release_ends_its_own_voice_and_no_other() {
    // Note A holds throughout; note B starts later and ends before A does. After B's release
    // the render equals A alone, bit for bit — B's instance is silent and A's untouched — and
    // during B, it does not.
    let plan = admit(&voice(4));
    let a = note(&plan, 60, 0, 30 * Q);
    let b = note(&plan, 72, 4 * Q, 8 * Q);
    let alone_a = render(&plan, std::slice::from_ref(&a), 24);
    let both = render(&plan, &[a, b], 24);
    // Output lags engine time by the primed quantum: B is released at engine 12Q, output 13Q.
    let after = (13 * Q) as usize;
    assert_eq!(
        &both[after..],
        &alone_a[after..],
        "after B's release the render is not A alone"
    );
    let during = (5 * Q) as usize..(12 * Q) as usize;
    assert_ne!(&both[during.clone()], &alone_a[during], "B did not sound");
}

#[test]
fn a_release_of_a_key_ends_the_newest_open_note_on_that_key() {
    // Two notes on one key, the second struck softer. The one release names the key, and
    // it ends the **newest** — `CompiledPayload::NoteOff`'s rule — so what remains is the
    // first note, bit for bit. Ending the oldest instead would leave the softer note, which
    // the comparison sees because velocity scales the envelope.
    let plan = admit(&voice(4));
    let loud = note(&plan, 60, 0, 30 * Q);
    let soft = struck(&plan, 60, NoteVelocity::saturating(0.5), 4 * Q, 8 * Q);
    let alone = render(&plan, std::slice::from_ref(&loud), 24);
    let both = render(&plan, &[loud, soft], 24);
    let after = (13 * Q) as usize;
    assert_eq!(
        &both[after..],
        &alone[after..],
        "the release did not end the newest note on the key"
    );
    let during = (5 * Q) as usize..(12 * Q) as usize;
    assert_ne!(
        &both[during.clone()],
        &alone[during],
        "the soft note did not sound"
    );
}

#[test]
fn a_parameter_write_to_a_voice_scope_control_reaches_every_instance() {
    // A compiled parameter write addresses the control, not an instance, so it fans out
    // over the group: opening the gate on a four-voice plan opens four envelopes, and the
    // voice sum carries four times what the same write renders on a one-voice plan. Only
    // a note's own magnitudes are per instance.
    let one = admit(&voice(1));
    let four = admit(&voice(4));
    let opened = |plan: &CompiledPlan| {
        let slot = plan
            .resolve_parameter(ENVELOPE, crate::ir::parameters::ENVELOPE_GATE)
            .expect("the gate is addressable");
        vec![PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::SetParameter {
                slot,
                value: crate::quantities::ParameterValue::ONE,
            },
        )]
    };
    let single = render(&one, &[opened(&one)], 8);
    let summed = render(&four, &[opened(&four)], 8);
    assert!(
        single.iter().any(|s| *s != 0.0),
        "the gate write did not sound"
    );
    assert_eq!(single.len(), summed.len());
    for (frame, (x, y)) in single.iter().zip(&summed).enumerate() {
        // Four identical instances added in one order: equal to `4x` up to the rounding of
        // three float additions.
        assert!(
            (4.0 * x - y).abs() <= 1e-6,
            "frame {frame}: four instances rendered {y}, one rendered {x}"
        );
    }
}

#[test]
fn a_quantum_rate_write_to_a_voice_scope_control_reaches_every_instance() {
    // The same fan-out on the quantum-rate path: the sine's amplitude is a smoothed,
    // quantum-rate control, and doubling it on a four-voice plan doubles every instance,
    // so the sum is four times the one-voice render of the same two writes. A write that
    // reached instance 0 alone would leave three instances at the prepared amplitude.
    let one = admit(&voice(1));
    let four = admit(&voice(4));
    let writes = |plan: &CompiledPlan| {
        let gate = plan
            .resolve_parameter(ENVELOPE, crate::ir::parameters::ENVELOPE_GATE)
            .expect("the gate is addressable");
        let amplitude = plan
            .resolve_parameter(OSCILLATOR, crate::ir::parameters::SINE_AMPLITUDE)
            .expect("the amplitude is addressable");
        vec![
            PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::SetParameter {
                    slot: gate,
                    value: crate::quantities::ParameterValue::ONE,
                },
            ),
            PlanEvent::new(
                PlanPosition::ZERO,
                CompiledPayload::SetParameter {
                    slot: amplitude,
                    value: crate::quantities::ParameterValue::new(0.5).expect("finite"),
                },
            ),
        ]
    };
    let single = render(&one, &[writes(&one)], 8);
    let summed = render(&four, &[writes(&four)], 8);
    let peak = |s: &[f32]| s.iter().fold(0.0_f32, |m, x| m.max(x.abs()));
    assert!(
        peak(&single) > 0.4,
        "the amplitude write did not take: {}",
        peak(&single)
    );
    for (frame, (x, y)) in single.iter().zip(&summed).enumerate() {
        assert!(
            (4.0 * x - y).abs() <= 1e-6,
            "frame {frame}: four instances rendered {y}, one rendered {x}"
        );
    }
}

#[test]
fn a_quantum_full_of_fanned_out_writes_has_room_in_the_control_scratch() {
    // The scratch a quantum's sample-positioned controls land in is sized on the widest
    // write one event can make, and since `P06-S001` that is a parameter write fanned out
    // over the voice instances — sixteen here, wider than a note-on's three. Fill one
    // quantum with the compiled share's worth of gate writes: every instance's run has to
    // fit, or the runs past the end are dropped without a word and those instances stay
    // shut. Sixteen instances then render sixteen times what one does.
    let one = admit(&voice(1));
    let sixteen = admit(&voice(16));
    let share = profile()
        .limits()
        .events()
        .shares()
        .compiled_event_share()
        .get() as u64;
    assert!(
        share * 16 > 256 * 3 + 16,
        "the fixture must need more scratch than a note-on's expansion alone would size"
    );
    let writes = |plan: &CompiledPlan| {
        let gate = plan
            .resolve_parameter(ENVELOPE, crate::ir::parameters::ENVELOPE_GATE)
            .expect("the gate is addressable");
        (0..share)
            .map(|n| {
                PlanEvent::new(
                    PlanPosition::new(n % Q),
                    CompiledPayload::SetParameter {
                        slot: gate,
                        value: crate::quantities::ParameterValue::ONE,
                    },
                )
            })
            .collect::<Vec<_>>()
    };
    let single = render(&one, &[writes(&one)], 4);
    let summed = render(&sixteen, &[writes(&sixteen)], 4);
    assert!(
        single.iter().any(|s| *s != 0.0),
        "the gate writes did not sound"
    );
    for (frame, (x, y)) in single.iter().zip(&summed).enumerate() {
        assert!(
            (16.0 * x - y).abs() <= 1e-5,
            "frame {frame}: sixteen instances rendered {y}, one rendered {x}"
        );
    }
}

#[test]
fn the_preflight_arena_bound_covers_a_voiced_plans_exact_arena() {
    // A plan refused before lowering carries the report built on the arena's **upper
    // bound**, and a bound is only a bound if it is not below the exact figure lowering
    // finds. Four voices schedule every voice-scope node four times and each instance writes
    // its own region, so a bound that counted each authored node once — as an independent
    // read found it doing — understated a four-voice plan's arena. The refusal here is on
    // the voice count, which the field order decides before the arena row, so the refused
    // outcome's scratch row is the preflight's.
    let ir = voice(4);
    let scratch = |outcome: &crate::compile::CompileOutcome| match outcome
        .report()
        .row(ResourceField::BufferScratchBytes)
        .map(|row| row.requested())
    {
        Some(ResourceAmount::Bytes(bytes)) => bytes.get(),
        other => panic!("the scratch row carries bytes, not {other:?}"),
    };
    let exact = scratch(&compile(&ir, &RenderConfig::new(profile())));
    let refused = compile(&ir, &RenderConfig::new(profile_with_voices(2)));
    assert!(matches!(
        refused.plan(),
        Err(crate::diagnostics::CompileError::LimitExceeded {
            field: ResourceField::MaxActiveVoices,
            ..
        })
    ));
    let bound = scratch(&refused);
    assert!(
        bound >= exact,
        "the preflight bound is {bound} bytes where lowering the same plan takes {exact}"
    );
}

#[test]
fn a_plan_with_one_simultaneous_note_has_one_instance_and_no_voice_sum() {
    // Polyphony one is today's plan exactly: one state per node, no inserted sum, and the
    // same schedule shape — which is what keeps every existing render bit-identical.
    let plan = admit(&voice(1));
    let records = plan
        .ops()
        .iter()
        .filter(|op| matches!(op, PlanOp::Node(_)))
        .count();
    assert_eq!(
        records, 3,
        "oscillator, envelope, amplifier — nothing inserted"
    );
    assert_eq!(plan.prepared_nodes().len(), 3);
    assert_eq!(plan.voice_instances().get(), 1);
}

#[test]
fn prepared_data_is_shared_and_state_is_per_instance() {
    // Four voices: four times the states and the parameter rows, one set of prepared
    // records — plus the voice sum's inserted copy and three accumulates, each with its own
    // small prepared record. The report says the same, and preparation allocates it.
    let ir = voice(4);
    let outcome = compile(&ir, &RenderConfig::new(profile()));
    let plan = outcome.plan().expect("admissible").clone();
    let node_records = plan
        .ops()
        .iter()
        .filter(|op| matches!(op, PlanOp::Node(_)))
        .count();
    assert_eq!(
        node_records,
        3 * 4 + 4,
        "three nodes × four voices, plus the sum"
    );
    assert_eq!(
        plan.prepared_nodes().len(),
        3 + 4,
        "one prepared record per node and per inserted step, none per voice"
    );
    // Every instance's step names the same prepared record as instance 0's.
    let mut by_prepared = std::collections::HashMap::new();
    for op in plan.ops() {
        if let PlanOp::Node(step) = op {
            *by_prepared.entry(step.prepared().index()).or_insert(0) += 1;
        }
    }
    assert_eq!(
        by_prepared.values().filter(|n| **n == 4).count(),
        3,
        "each node's record is read by four steps"
    );
    // Parameter rows: frequency and amplitude on the sine, gate and velocity on the envelope
    // — four controls, four rows each, four addresses.
    assert_eq!(plan.parameter_targets().len(), 4 * 4);
    assert_eq!(plan.parameter_addresses().len(), 4);
    assert!(
        plan.parameter_targets()
            .iter()
            .all(|t| t.instances.get() == 4)
    );
    // The prepared row does not grow with voices; the mutable row does, and preparation
    // holds exactly what it charges.
    let one = compile(&voice(1), &RenderConfig::new(profile()));
    let requested = |outcome: &crate::compile::CompileOutcome, field| match outcome
        .report()
        .row(field)
        .map(|row| row.requested())
    {
        Some(ResourceAmount::Bytes(bytes)) => bytes.get(),
        other => panic!("{field:?} carries bytes, not {other:?}"),
    };
    let prepared_one = requested(&one, ResourceField::PreparedImmutableBytes);
    let prepared_four = requested(&outcome, ResourceField::PreparedImmutableBytes);
    let node_records_one = 3_u64;
    // Four inserted sum steps add their own prepared records and nothing else.
    assert_eq!(
        prepared_four - prepared_one,
        4 * crate::node::prepared_bytes_per_node(),
        "prepared memory grew by more than the sum's own records"
    );
    let _ = node_records_one;
    let mutable_one = requested(&one, ResourceField::MutableStateBytes);
    let mutable_four = requested(&outcome, ResourceField::MutableStateBytes);
    assert!(
        mutable_four > 3 * mutable_one,
        "mutable memory does not scale with voices"
    );
    let (_, renderer) = StreamControl::open(plan.clone(), ORIGIN).expect("opens");
    assert_eq!(
        renderer.prepared_record_count().get() as usize,
        node_records
    );
    assert_eq!(
        renderer.slot_bytes_held() as u64
            + renderer.ramp_table_bytes_held() as u64
            + node_records as u64 * crate::node::state_bytes_per_node(),
        mutable_four,
        "preparation holds what the mutable row charges"
    );
}

#[test]
fn the_voice_count_is_admitted_as_derived_from_the_producers() {
    // `max_active_voices` refuses the derived count, naming both amounts; the same plan with
    // the count under the limit is admitted. A plan without a voice-scope node requests none.
    let host = profile_with_voices(2);
    let refused = compile(&voice(4), &RenderConfig::new(host));
    match refused.plan() {
        Err(crate::diagnostics::CompileError::LimitExceeded {
            field,
            requested,
            available,
            ..
        }) => {
            assert_eq!(*field, ResourceField::MaxActiveVoices);
            assert_eq!(
                *requested,
                ResourceAmount::Voices(crate::quantities::VoiceCount::measured(4))
            );
            assert_eq!(
                *available,
                ResourceAmount::Voices(crate::quantities::VoiceCount::limit(2).expect("positive"))
            );
        }
        other => panic!("expected a voice refusal, got {other:?}"),
    }
    assert!(compile(&voice(2), &RenderConfig::new(host)).plan().is_ok());
}

// --- fixtures -----------------------------------------------------------------------------

fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        FrameCount::new(BLOCK as u64),
        ChannelLayout::Mono,
    )
    .expect("valid harness profile")
}

/// The harness profile with `max_active_voices` overridden.
fn profile_with_voices(voices: u32) -> HostProfile {
    let base = profile();
    let limits = base.limits();
    let v = limits.voices();
    let voices_limits = crate::profile::VoiceLimits::new(
        v.minimum_voices_per_instrument(),
        v.maximum_voices_per_instrument(),
        crate::quantities::VoiceCount::limit(voices).expect("positive"),
        v.max_held_notes(),
        v.retirement_crossfade(),
    )
    .expect("the overridden capacities are above zero");
    let limits = crate::profile::RenderLimits::new(
        limits.stream(),
        limits.graph(),
        voices_limits,
        limits.events(),
        limits.observation(),
        limits.mixing(),
        limits.memory(),
        limits.script(),
        limits.recording(),
        limits.cost(),
    )
    .expect("the overridden limits are internally consistent");
    HostProfile::new(base.capabilities(), limits).expect("the profile is consistent")
}

fn admit(ir: &GraphIr) -> CompiledPlan {
    compile(ir, &RenderConfig::new(profile()))
        .into_plan()
        .expect("the plan fits this profile")
}

/// The smallest real voice, with a compiled producer of `notes` simultaneous notes.
fn voice(notes: u32) -> GraphIr {
    GraphIr::builder()
        .node(
            OSCILLATOR,
            IrNodeKind::Sine {
                frequency: Frequency::new(220.0).expect("finite"),
                amplitude: Amplitude::new(0.25).expect("finite"),
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
                simultaneous_notes: HeldNoteCount::measured(notes),
                simultaneous_holds: EventCount::NONE,
            }],
            held_notes: HeldNoteCount::measured(notes),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("a readable plan")
}

/// A full-velocity note-on at `key` from `at`, released `length` frames later.
fn note(plan: &CompiledPlan, key: u8, at: u64, length: u64) -> Vec<PlanEvent> {
    struck(plan, key, NoteVelocity::FULL, at, length)
}

/// A note-on at `key` struck at `velocity` from `at`, released `length` frames later.
fn struck(
    plan: &CompiledPlan,
    key: u8,
    velocity: NoteVelocity,
    at: u64,
    length: u64,
) -> Vec<PlanEvent> {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    vec![
        PlanEvent::new(
            PlanPosition::new(at),
            CompiledPayload::NoteOn {
                slot,
                key: KeyIdentity::new(key).expect("a keyboard position"),
                velocity,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(at + length),
            CompiledPayload::NoteOff {
                slot,
                key: KeyIdentity::new(key).expect("a keyboard position"),
            },
        ),
    ]
}

/// Render `quanta` quanta of the plan with the given notes, in host blocks.
fn render(plan: &CompiledPlan, notes: &[Vec<PlanEvent>], quanta: u64) -> Vec<f32> {
    let mut events: Vec<PlanEvent> = notes.iter().flatten().copied().collect();
    events.sort_by_key(|event| event.position());
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(plan, &events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let mut arbiter = PublicationArbiter::prepare(&profile()).expect("the store is preparable");
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        (quanta * Q) as usize,
    )
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

#[test]
fn each_note_lands_on_its_own_instance_and_the_rest_stay_at_rest() {
    // The routing, read from the kernels' own state: after two notes start, two oscillator
    // instances carry the two keys' frequencies and two envelopes are held, while the other
    // two instances of each still hold the prepared frequency and a released gate.
    use crate::node::kernels::{ENVELOPE_GATE, SINE_FREQUENCY};
    let plan = admit(&voice(4));
    let a = note(&plan, 60, 0, 12 * Q);
    let b = note(&plan, 67, 3 * Q + 5, 12 * Q);
    let mut events: Vec<PlanEvent> = [a, b].iter().flatten().copied().collect();
    events.sort_by_key(|event| event.position());
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let mut arbiter = PublicationArbiter::prepare(&profile()).expect("the store is preparable");
    let _ = drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        (8 * Q) as usize,
    );

    let value = |v: f32| crate::quantities::ParameterValue::new(v).expect("finite");
    // Control indices are per kind, so the states are picked by variant first.
    let frequencies: Vec<_> = renderer
        .node_states()
        .iter()
        .filter(|state| matches!(state, crate::node::kernels::NodeState::Sine { .. }))
        .filter_map(|state| state.control_value(SINE_FREQUENCY))
        .collect();
    let gates: Vec<_> = renderer
        .node_states()
        .iter()
        .filter(|state| matches!(state, crate::node::kernels::NodeState::Envelope { .. }))
        .filter_map(|state| state.control_value(ENVELOPE_GATE))
        .collect();
    let tuning = crate::tuning::PreparedTuning::equal_temperament().expect("prepares");
    let hz = |key: u8| {
        crate::quantities::ParameterValue::from_frequency(
            tuning.frequency_of(KeyIdentity::new(key).expect("a keyboard position")),
        )
    };
    assert_eq!(
        frequencies,
        vec![hz(60), hz(67), value(220.0), value(220.0)],
        "the keys did not land on the two instances the identities name"
    );
    assert_eq!(
        gates,
        vec![value(1.0), value(1.0), value(0.0), value(0.0)],
        "the gates did not land on the two instances the identities name"
    );
}
