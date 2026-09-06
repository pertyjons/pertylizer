//! `P06-S001`: `N` voice instances of one prepared voice plan.
//!
//! Inside the crate to read the renderer's state table and the plan's prepared records
//! directly: the falsifiers are that two overlapping notes render as two voices, that a
//! release ends its own voice and no other, that the prepared data is shared and the state
//! is not, and that the report charges exactly what preparation holds per instance.

use crate::compile::{RenderConfig, compile};
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain, StealingPolicy,
};
use crate::plan::{CompiledPlan, PlanOp};
use crate::profile::HostProfile;
use crate::publish::PublicationArbiter;
use crate::quantities::{
    Amplitude, Cents, ChannelLayout, EventCount, Frequency, HeldNoteCount, KeyIdentity,
    NormalizedLevel, NoteVelocity, SampleRate, Seconds,
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
    // Parameter rows: frequency and amplitude on the sine, gate, velocity and velocity
    // sensitivity on the envelope — five controls, four rows each, five addresses.
    assert_eq!(plan.parameter_targets().len(), 5 * 4);
    assert_eq!(plan.parameter_addresses().len(), 5);
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
    voice_with(notes, StealingPolicy::None)
}

/// V1's fade, in frames.
const FADE: u64 = 128;

/// The same voice under a stealing policy (ADR-0058).
fn voice_with(notes: u32, stealing: StealingPolicy) -> GraphIr {
    voice_shaped(notes, stealing, Seconds::ZERO)
}

/// The same, with an envelope attack, so a fresh voice is audibly fresh.
fn voice_shaped(notes: u32, stealing: StealingPolicy, attack: Seconds) -> GraphIr {
    voice_composed(notes, stealing, attack, NormalizedLevel::FULL, None)
}

/// V1's voice-output velocity stage, when a plan carries one.
const SCALER: NodeId = NodeId::new(5);

/// The voice with its velocity composition chosen (ADR-0059): the envelope's sensitivity, and
/// a velocity scaler between the amplifier and the output where `scaler` names its
/// sensitivity.
fn voice_composed(
    notes: u32,
    stealing: StealingPolicy,
    attack: Seconds,
    envelope_sensitivity: NormalizedLevel,
    scaler: Option<NormalizedLevel>,
) -> GraphIr {
    let mut builder = GraphIr::builder()
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
                attack,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release: Seconds::ZERO,
                velocity_sensitivity: envelope_sensitivity,
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
        );
    builder = match scaler {
        Some(sensitivity) => builder
            .node(
                SCALER,
                IrNodeKind::VelocityScaler { sensitivity },
                ExecutionScope::Voice,
            )
            .connect(
                (AMPLIFIER, PortId::FIRST),
                (SCALER, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (SCALER, PortId::FIRST),
                (OUTPUT, PortId::FIRST),
                SignalDomain::Audio,
            ),
        None => builder.connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        ),
    };
    builder
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
            stealing,
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
    render_counted(plan, notes, quanta).0
}

/// [`render`], and how many releases preparation dropped after a steal (ADR-0058).
fn render_counted(
    plan: &CompiledPlan,
    notes: &[Vec<PlanEvent>],
    quanta: u64,
) -> (Vec<f32>, usize, usize) {
    let mut events: Vec<PlanEvent> = notes.iter().flatten().copied().collect();
    events.sort_by_key(|event| event.position());
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(plan, &events).expect("the stream fits");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");
    let mut arbiter = PublicationArbiter::prepare(&profile()).expect("the store is preparable");
    let out = drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        (quanta * Q) as usize,
    );
    (
        out,
        scheduler.released_after_steal(),
        scheduler.expressions_after_steal(),
    )
}

/// The gain the fade applies `i` frames in, as the sum kernel computes it.
fn fade_gain(i: u64) -> f32 {
    (FADE - i) as f32 / FADE as f32
}

#[test]
fn a_full_producer_takes_the_oldest_voice_fades_it_and_starts_the_new_note_when_the_fade_ends() {
    // ADR-0058 clauses 3 and 4 on a two-voice plan: A and B hold both voices when C arrives
    // at `p`. A is the oldest, so A's voice fades linearly over 128 frames from `p` while B
    // plays on, and at `p + 128` the voice is reset and C starts on it as a fresh note. The
    // oracle is three single-note renders combined sample for sample — A alone scaled by
    // the fade, then C alone started at `p + 128`, B alone added throughout — in the order
    // the sum adds them, so the comparison is exact. A's own release, later, names a note
    // the steal ended: dropped at preparation and counted once.
    let plan = admit(&voice_with(
        2,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
    ));
    let p = 4 * Q + 5;
    let a = note(&plan, 60, 0, 30 * Q);
    let b = note(&plan, 67, 2 * Q, 30 * Q);
    let c = note(&plan, 72, p, 30 * Q);
    let alone_a = render(&plan, std::slice::from_ref(&a), 24);
    let alone_b = render(&plan, std::slice::from_ref(&b), 24);
    let alone_c = render(&plan, &[note(&plan, 72, 0, 30 * Q)], 24);
    let (together, dropped, _) = render_counted(&plan, &[a, b, c], 24);
    assert_eq!(dropped, 1, "A's release names a note the steal ended");
    // Output lags engine time by the primed quantum. The reset voice is a **fresh** one —
    // its oscillator restarts at phase zero — so C's oracle is C rendered from time zero on a
    // fresh stream, shifted to where the fade completes; C rendered alone at `p + 128` would
    // carry the phase its oscillator had accumulated by then.
    let fade_from = (p + Q) as usize;
    let fade_to = fade_from + FADE as usize;
    let shift = (p + FADE) as usize;
    for (k, actual) in together.iter().copied().enumerate() {
        let expected = if k < fade_from {
            alone_a[k] + alone_b[k]
        } else if k < fade_to {
            alone_a[k] * fade_gain((k - fade_from) as u64) + alone_b[k]
        } else {
            alone_c[k - shift] + alone_b[k]
        };
        assert_eq!(
            actual, expected,
            "frame {k}: the steal did not fade A over 128 frames and start C on its voice"
        );
    }
    // The new note is audible: C's voice is not silent after the fade.
    assert!(
        together[fade_to..]
            .iter()
            .zip(&alone_b[fade_to..])
            .any(|(t, b)| t != b)
    );
}

#[test]
fn a_same_note_policy_retriggers_the_held_key_at_once_and_without_a_fade() {
    // ADR-0058 clause 4's retrigger: A holds key 60, B holds 67, and a second note on key 60
    // arrives with the producer full. Under `SameNote` it takes A's voice at its own
    // position with no fade and no reset — the gate falls and rises before one frame is
    // written, so the envelope re-attacks from the level it stood at, here full — so the only
    // audible change is the new note's velocity: instance 0 emits A's signal at half level
    // from `p`, exactly, and B is untouched. **A compiled release names a key**, so A's
    // release at `20 Q` pairs with the newest open note of its key, which is the new one,
    // and ends it: from there instance 0 is silent. The new note's own release then names a
    // note the steal took, and is dropped and counted. An independent read asked whether
    // the taken note's release "ends another note" here against the record's falsifier; it
    // does, by the key-pairing rule the compiled stream has always had, which the record's
    // identity clause cannot reach where a release carries no identity.
    let plan = admit(&voice_with(
        2,
        StealingPolicy::SameNote {
            fade: FrameCount::new(FADE),
        },
    ));
    let p = 4 * Q + 5;
    let a = note(&plan, 60, 0, 20 * Q);
    let b = note(&plan, 67, 2 * Q, 30 * Q);
    let again = struck(&plan, 60, NoteVelocity::saturating(0.5), p, 18 * Q);
    let alone_a = render(&plan, std::slice::from_ref(&a), 24);
    let alone_b = render(&plan, std::slice::from_ref(&b), 24);
    let (together, dropped, _) = render_counted(&plan, &[a, b, again], 24);
    assert_eq!(dropped, 1, "the new note's own release, after A's ended it");
    let from = (p + Q) as usize;
    let ended = (20 * Q + Q) as usize;
    for (k, actual) in together.iter().copied().enumerate() {
        let expected = if k < from {
            alone_a[k] + alone_b[k]
        } else if k < ended {
            alone_a[k] * 0.5 + alone_b[k]
        } else {
            alone_b[k]
        };
        assert_eq!(
            actual, expected,
            "frame {k}: the retrigger did not take the held key's voice at once"
        );
    }
}

#[test]
fn the_taken_voice_is_reset_so_the_new_note_attacks_from_silence() {
    // ADR-0058 clause 4's reset, made audible by an attack: on the taken voice the new note
    // must climb from zero as a fresh voice does. Without the reset the envelope would still
    // be held at full level and a gate re-asserted on it is not an edge, so the new note
    // would start at full level instead. The oracle is the same fresh-voice shift as above.
    let plan = admit(&voice_shaped(
        2,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
        Seconds::new(0.01).expect("not negative"),
    ));
    let p = 4 * Q + 5;
    let a = note(&plan, 60, 0, 30 * Q);
    let b = note(&plan, 67, 2 * Q, 30 * Q);
    let c = note(&plan, 72, p, 30 * Q);
    let alone_a = render(&plan, std::slice::from_ref(&a), 24);
    let alone_b = render(&plan, std::slice::from_ref(&b), 24);
    let alone_c = render(&plan, &[note(&plan, 72, 0, 30 * Q)], 24);
    let together = render(&plan, &[a, b, c], 24);
    let fade_from = (p + Q) as usize;
    let fade_to = fade_from + FADE as usize;
    let shift = (p + FADE) as usize;
    for (k, actual) in together.iter().copied().enumerate() {
        let expected = if k < fade_from {
            alone_a[k] + alone_b[k]
        } else if k < fade_to {
            alone_a[k] * fade_gain((k - fade_from) as u64) + alone_b[k]
        } else {
            alone_c[k - shift] + alone_b[k]
        };
        assert_eq!(
            actual, expected,
            "frame {k}: the new note did not attack from silence"
        );
    }
}

#[test]
fn a_same_note_policy_takes_the_newest_held_note_of_the_key() {
    // Two held notes share the key on a three-voice plan, with a third key held beside them;
    // a fourth note on the shared key takes the **newest** of the two — the rule a release
    // uses — so the older one plays on at full level and the newer one's voice carries the
    // new velocity. Taking the oldest would leave the two voices the other way round, and
    // the sum tells them apart because their oscillators changed pitch at different times.
    let plan = admit(&voice_with(
        3,
        StealingPolicy::SameNote {
            fade: FrameCount::new(FADE),
        },
    ));
    let p = 6 * Q + 9;
    let first = note(&plan, 60, 0, 30 * Q);
    let second = note(&plan, 60, 2 * Q, 30 * Q);
    let other = note(&plan, 67, 3 * Q, 30 * Q);
    let fourth = struck(&plan, 60, NoteVelocity::saturating(0.5), p, 30 * Q);
    let alone_first = render(&plan, std::slice::from_ref(&first), 16);
    let alone_second = render(&plan, std::slice::from_ref(&second), 16);
    let alone_other = render(&plan, std::slice::from_ref(&other), 16);
    let together = render(&plan, &[first, second, other, fourth], 16);
    let from = (p + Q) as usize;
    for (k, actual) in together.iter().copied().enumerate() {
        let expected = if k < from {
            alone_first[k] + alone_second[k] + alone_other[k]
        } else {
            alone_first[k] + alone_second[k] * 0.5 + alone_other[k]
        };
        assert_eq!(
            actual, expected,
            "frame {k}: the retrigger took the wrong voice"
        );
    }
}

#[test]
fn a_note_shorter_than_the_fade_still_starts_and_ends_on_the_voice_it_took() {
    // The note that takes a voice starts `fade` frames late, and its release is displaced
    // with it, so a note shorter than the fade is not released before it starts — which
    // would leave it sounding with no release to come — and every taken-in note keeps its
    // authored length. An independent read found the release left at its authored position.
    let plan = admit(&voice_with(
        2,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
    ));
    let p = 4 * Q + 5;
    let a = note(&plan, 60, 0, 30 * Q);
    let b = note(&plan, 67, 2 * Q, 30 * Q);
    let short = note(&plan, 72, p, 50);
    let alone_a = render(&plan, std::slice::from_ref(&a), 16);
    let alone_b = render(&plan, std::slice::from_ref(&b), 16);
    let alone_short = render(&plan, &[note(&plan, 72, 0, 50)], 16);
    let (together, dropped, _) = render_counted(&plan, &[a, b, short], 16);
    assert_eq!(dropped, 1, "A's release");
    let fade_from = (p + Q) as usize;
    let fade_to = fade_from + FADE as usize;
    let shift = (p + FADE) as usize;
    for (k, actual) in together.iter().copied().enumerate() {
        let expected = if k < fade_from {
            alone_a[k] + alone_b[k]
        } else if k < fade_to {
            alone_a[k] * fade_gain((k - fade_from) as u64) + alone_b[k]
        } else {
            alone_short[k - shift] + alone_b[k]
        };
        assert_eq!(
            actual, expected,
            "frame {k}: the short note did not keep its length"
        );
    }
    // And it did end: past its displaced release the voice is silent, so B alone remains.
    let silent_from = shift + 50 + Q as usize;
    assert_eq!(&together[silent_from..], &alone_b[silent_from..]);
    assert!(
        together[fade_to..silent_from]
            .iter()
            .zip(&alone_b[fade_to..])
            .any(|(t, b)| t != b)
    );
}

#[test]
fn an_oldest_policy_fades_a_taken_voice_even_when_the_keys_match() {
    // `Oldest` names the oldest voice and says nothing about keys: a third note on the first
    // note's key still takes that voice by fade-then-start, not by `SameNote`'s retrigger.
    let plan = admit(&voice_with(
        2,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
    ));
    let p = 4 * Q + 5;
    let a = note(&plan, 60, 0, 30 * Q);
    let b = note(&plan, 67, 2 * Q, 30 * Q);
    let c = struck(&plan, 60, NoteVelocity::saturating(0.5), p, 30 * Q);
    let alone_a = render(&plan, std::slice::from_ref(&a), 16);
    let alone_b = render(&plan, std::slice::from_ref(&b), 16);
    let alone_c = render(
        &plan,
        &[struck(&plan, 60, NoteVelocity::saturating(0.5), 0, 30 * Q)],
        16,
    );
    let together = render(&plan, &[a, b, c], 16);
    let fade_from = (p + Q) as usize;
    let fade_to = fade_from + FADE as usize;
    let shift = (p + FADE) as usize;
    for (k, actual) in together.iter().copied().enumerate() {
        let expected = if k < fade_from {
            alone_a[k] + alone_b[k]
        } else if k < fade_to {
            alone_a[k] * fade_gain((k - fade_from) as u64) + alone_b[k]
        } else {
            alone_c[k - shift] + alone_b[k]
        };
        assert_eq!(
            actual, expected,
            "frame {k}: the same key was retriggered under Oldest"
        );
    }
}

#[test]
fn a_voice_waiting_to_start_is_not_taken_and_a_released_one_stays_committed_to_its_tail() {
    // Two holes an independent read found, on the stamped events. C takes A's voice at `p` and
    // starts at `p + 128`; D arrives at `p + 60`, while C waits. The only voice that may be
    // taken is B's — C's is committed to a start that has not happened — so D's fade names
    // B's occurrence, not C's. And C's own release at `p + 50`, displaced to `p + 178`, keeps
    // C's index in the minter until then: a release freed at its authored position let a
    // later note mint onto a voice whose deferred start then clobbered it.
    let plan = admit(&voice_with(
        2,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
    ));
    let p = 4 * Q + 5;
    let mut events: Vec<PlanEvent> = [
        note(&plan, 60, 0, 30 * Q),
        note(&plan, 67, 2 * Q, 30 * Q),
        note(&plan, 72, p, 50),
        note(&plan, 76, p + 60, 30 * Q),
    ]
    .iter()
    .flatten()
    .copied()
    .collect();
    events.sort_by_key(|event| event.position());
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
    let placed: Vec<crate::schedule::CompiledEvent> = stream
        .events()
        .iter()
        .map(|event| {
            crate::schedule::CompiledEvent::new(
                SampleTime::new(event.position().as_u64()),
                event.payload(),
            )
        })
        .collect();
    let mut minter = control.minter_mut().working_copy();
    let (stamped, _) = crate::schedule::stamp_into(&mut minter, &plan, control.epoch(), &placed)
        .expect("the list stamps");
    let fades: Vec<(u64, u16)> = stamped
        .iter()
        .filter_map(|event| match event.payload() {
            crate::render::EventPayload::Fade { identity, .. } => {
                Some((event.envelope().time().as_u64(), identity.index()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        fades,
        vec![(p, 0), (p + 60, 1)],
        "C took A's voice (index 0); D, arriving while C waits, took B's (index 1)"
    );
    // C's displaced release is the last edge of its voice, and D never lands on index 0.
    let starts: Vec<(u64, u16)> = stamped
        .iter()
        .filter_map(|event| match event.payload() {
            crate::render::EventPayload::Note {
                identity,
                edge: crate::render::NoteEdge::On { .. },
            } => Some((event.envelope().time().as_u64(), identity.index())),
            _ => None,
        })
        .collect();
    assert_eq!(
        starts,
        vec![(0, 0), (2 * Q, 1), (p + FADE, 0), (p + 60 + FADE, 1)]
    );
}

#[test]
fn a_note_on_finding_every_voice_waiting_to_start_is_an_over_emission() {
    // On one voice, C takes A at `p` and waits to start; D arrives at `p + 60` while it waits.
    // The only voice is committed, so nothing may be taken and the note-on is what it would
    // be without stealing: an over-emission, refused at preparation with the minter's cause.
    // Once the fade has passed, the voice has started and D takes it.
    let plan = admit(&voice_with(
        1,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
    ));
    let p = 4 * Q + 5;
    let prepare = |third_at: u64| {
        let mut events: Vec<PlanEvent> = [
            note(&plan, 60, 0, 30 * Q),
            note(&plan, 72, p, 30 * Q),
            note(&plan, 76, third_at, 30 * Q),
        ]
        .iter()
        .flatten()
        .copied()
        .collect();
        events.sort_by_key(|event| event.position());
        let (mut control, _renderer) =
            StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
        let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
        CompiledEventScheduler::prepare(&mut control, &stream).map(|_| ())
    };
    assert!(matches!(
        prepare(p + 60),
        Err(crate::schedule::SchedulePrepareError::Identity {
            event_index: 2,
            source: crate::identity::IdentityError::ProducerOverEmitted { .. },
        })
    ));
    assert!(prepare(p + FADE).is_ok());
}

/// A bend of the newest open note on the envelope with `key`, at `at`.
fn bend(plan: &CompiledPlan, key: u8, at: u64, cents: f32) -> PlanEvent {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    PlanEvent::new(
        PlanPosition::new(at),
        CompiledPayload::Bend {
            slot,
            key: KeyIdentity::new(key).expect("a keyboard position"),
            cents: Cents::new(cents).expect("a bend"),
        },
    )
}

#[test]
fn a_bend_moves_one_occurrence_by_its_cents_under_the_semitone_law() {
    // `SOUND-INV-021`'s bend, held to `SOUND-INV-023`'s law: on one voice, a bend of +100
    // cents at `p` renders exactly what a sample-positioned write of the frequency the law
    // resolves for one semitone above the key renders — the same arithmetic, in the slot,
    // reached by two paths. The oracle takes the law's own figure, so nothing here restates
    // the arithmetic.
    let plan = admit(&voice(1));
    let p = 4 * Q + 5;
    let a = note(&plan, 60, 0, 30 * Q);
    let bent = render(&plan, &[a.clone(), vec![bend(&plan, 60, p, 100.0)]], 16);
    let tuning = crate::tuning::PreparedTuning::equal_temperament().expect("prepares");
    let base = crate::quantities::ParameterValue::from_frequency(
        tuning.frequency_of(KeyIdentity::new(60).expect("a keyboard position")),
    );
    let resolved = crate::quantities::ParameterValue::saturating(
        crate::node::ModulationLaw::SemitoneAdditive.resolve(
            base,
            crate::node::ModulationSum::from_bend(Cents::new(100.0).expect("a bend")),
        ),
    );
    let frequency = plan
        .resolve_parameter(OSCILLATOR, crate::ir::parameters::SINE_FREQUENCY)
        .expect("the frequency is addressable");
    let written = render(
        &plan,
        &[
            a,
            vec![PlanEvent::new(
                PlanPosition::new(p),
                CompiledPayload::SetParameter {
                    slot: frequency,
                    value: resolved,
                },
            )],
        ],
        16,
    );
    assert_eq!(
        bent, written,
        "the bend did not resolve under the semitone law"
    );
    let unbent = render(&plan, &[note(&plan, 60, 0, 30 * Q)], 16);
    assert_ne!(
        bent[(p + Q) as usize..],
        unbent[(p + Q) as usize..],
        "the bend moved nothing"
    );
}

#[test]
fn a_bend_followed_by_its_notes_release_in_one_call_still_moves_the_note() {
    // The independent read's case: the release lands forty frames after the bend, in the
    // same render call, and the walk that resolves targets applies every release before the
    // passes run. A bend that looked the note up at pass time found it gone and moved nothing;
    // resolved once with its target, it moves the note for the frames it has.
    let plan = admit(&voice(1));
    let p = 4 * Q + 5;
    let bent = render(
        &plan,
        &[note(&plan, 60, 0, p + 40), vec![bend(&plan, 60, p, 100.0)]],
        16,
    );
    let tuning = crate::tuning::PreparedTuning::equal_temperament().expect("prepares");
    let base = crate::quantities::ParameterValue::from_frequency(
        tuning.frequency_of(KeyIdentity::new(60).expect("a keyboard position")),
    );
    let resolved = crate::quantities::ParameterValue::saturating(
        crate::node::ModulationLaw::SemitoneAdditive.resolve(
            base,
            crate::node::ModulationSum::from_bend(Cents::new(100.0).expect("a bend")),
        ),
    );
    let frequency = plan
        .resolve_parameter(OSCILLATOR, crate::ir::parameters::SINE_FREQUENCY)
        .expect("the frequency is addressable");
    let written = render(
        &plan,
        &[
            note(&plan, 60, 0, p + 40),
            vec![PlanEvent::new(
                PlanPosition::new(p),
                CompiledPayload::SetParameter {
                    slot: frequency,
                    value: resolved,
                },
            )],
        ],
        16,
    );
    assert_eq!(
        bent, written,
        "the bend before the release in one call moved nothing"
    );
    let unbent = render(&plan, &[note(&plan, 60, 0, p + 40)], 16);
    assert_ne!(bent, unbent);
}

#[test]
fn a_bend_reaches_only_the_occurrence_it_names() {
    // Two voices held; a bend names the first key's note. Its instance moves and the other is
    // untouched, so the render is the bent note alone plus the other alone — and the velocity
    // destination of the bent note is not touched either, which a half velocity shows: a bend
    // that landed on every row of the note would move it through the normalized law.
    let plan = admit(&voice(2));
    let p = 4 * Q + 5;
    let a = struck(&plan, 60, NoteVelocity::saturating(0.5), 0, 30 * Q);
    let b = note(&plan, 67, 2 * Q, 30 * Q);
    let alone_a_bent = render(&plan, &[a.clone(), vec![bend(&plan, 60, p, -350.0)]], 16);
    let alone_a = render(&plan, std::slice::from_ref(&a), 16);
    let alone_b = render(&plan, std::slice::from_ref(&b), 16);
    let together = render(&plan, &[a, b, vec![bend(&plan, 60, p, -350.0)]], 16);
    for (k, actual) in together.iter().copied().enumerate() {
        assert_eq!(actual, alone_a_bent[k] + alone_b[k], "frame {k}");
    }
    assert_ne!(
        alone_a_bent[(p + Q) as usize..],
        alone_a[(p + Q) as usize..]
    );
}

#[test]
fn a_new_occurrence_on_a_voice_starts_unbent() {
    // Per-note expression is the occurrence's: A is bent and released, and the next note on
    // the same voice starts at exactly the frequency its own key resolves to, read from the
    // oscillator's state. A bend that outlived its note would carry into C.
    use crate::node::kernels::SINE_FREQUENCY;
    let plan = admit(&voice(1));
    let p = 4 * Q + 5;
    let mut events: Vec<PlanEvent> = [
        note(&plan, 60, 0, 8 * Q),
        vec![bend(&plan, 60, p, 700.0)],
        note(&plan, 72, 10 * Q, 30 * Q),
    ]
    .iter()
    .flatten()
    .copied()
    .collect();
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
        (12 * Q) as usize,
    );
    let tuning = crate::tuning::PreparedTuning::equal_temperament().expect("prepares");
    let expected = crate::quantities::ParameterValue::from_frequency(
        tuning.frequency_of(KeyIdentity::new(72).expect("a keyboard position")),
    );
    let frequencies: Vec<_> = renderer
        .node_states()
        .iter()
        .filter(|state| matches!(state, crate::node::kernels::NodeState::Sine { .. }))
        .filter_map(|state| state.control_value(SINE_FREQUENCY))
        .collect();
    assert_eq!(frequencies, vec![expected], "C carried A's bend");
}

#[test]
fn a_bend_for_a_note_a_steal_ended_is_dropped_and_counted_with_its_release() {
    // ADR-0058 clause 5: expression addressed to a taken note is an orphan. On the compiled
    // path preparation drops it, and counts it beside the taken note's release.
    let plan = admit(&voice_with(
        2,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
    ));
    let p = 4 * Q + 5;
    let (_, releases, bends) = render_counted(
        &plan,
        &[
            note(&plan, 60, 0, 30 * Q),
            note(&plan, 67, 2 * Q, 30 * Q),
            note(&plan, 72, p, 30 * Q),
            vec![bend(&plan, 60, p + 3 * Q, 50.0)],
        ],
        16,
    );
    assert_eq!(
        (releases, bends),
        (1, 1),
        "A's release, and A's bend, each under its own name"
    );
}

#[test]
fn a_bend_naming_no_open_note_is_refused_at_preparation() {
    let plan = admit(&voice(2));
    let events = vec![bend(&plan, 60, Q, 10.0)];
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
    assert!(matches!(
        CompiledEventScheduler::prepare(&mut control, &stream),
        Err(crate::schedule::SchedulePrepareError::UnmatchedExpression { event_index: 0 })
    ));
}

#[test]
fn a_bend_of_a_note_that_took_a_voice_is_displaced_with_its_start() {
    // C takes A's voice at `p` and starts at `p + 128`; a bend of C at `p + 50` cannot move a
    // note that has not started, so it lands at `p + 178`, displaced as C's edges are.
    let plan = admit(&voice_with(
        2,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
    ));
    let p = 4 * Q + 5;
    let mut events: Vec<PlanEvent> = [
        note(&plan, 60, 0, 30 * Q),
        note(&plan, 67, 2 * Q, 30 * Q),
        note(&plan, 72, p, 30 * Q),
        vec![bend(&plan, 72, p + 50, 25.0)],
    ]
    .iter()
    .flatten()
    .copied()
    .collect();
    events.sort_by_key(|event| event.position());
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the stream fits");
    let placed: Vec<crate::schedule::CompiledEvent> = stream
        .events()
        .iter()
        .map(|event| {
            crate::schedule::CompiledEvent::new(
                SampleTime::new(event.position().as_u64()),
                event.payload(),
            )
        })
        .collect();
    let mut minter = control.minter_mut().working_copy();
    let (stamped, _) = crate::schedule::stamp_into(&mut minter, &plan, control.epoch(), &placed)
        .expect("the list stamps");
    let bends: Vec<(u64, u16)> = stamped
        .iter()
        .filter_map(|event| match event.payload() {
            crate::render::EventPayload::Bend { identity, .. } => {
                Some((event.envelope().time().as_u64(), identity.index()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(bends, vec![(p + 50 + FADE, 0)]);
}

#[test]
fn the_envelope_scales_its_level_by_v1s_sensitivity_law() {
    // ADR-0059 clause 2: the envelope's scale is V1's `1 − s × (1 − v)`, computed in the
    // kernel from the note's velocity and the authored sensitivity. With attack zero and
    // sustain full the envelope emits exactly that scale, and the amplifier multiplies the
    // full-velocity render by it, so the oracle is the same `f32` expression. Sensitivity
    // one is the bare velocity, zero ignores it.
    let reference = admit(&voice_composed(
        1,
        StealingPolicy::None,
        Seconds::ZERO,
        NormalizedLevel::FULL,
        None,
    ));
    let full = render(&reference, &[note(&reference, 60, 0, 30 * Q)], 8);
    for (s, v) in [
        (1.0_f32, 0.25_f32),
        (0.0, 0.25),
        (0.5, 0.25),
        (0.75, 0.7559055),
    ] {
        let plan = admit(&voice_composed(
            1,
            StealingPolicy::None,
            Seconds::ZERO,
            NormalizedLevel::new(s).expect("in range"),
            None,
        ));
        let rendered = render(
            &plan,
            &[struck(&plan, 60, NoteVelocity::saturating(v), 0, 30 * Q)],
            8,
        );
        let scale = 1.0 - s * (1.0 - v);
        for (k, actual) in rendered.iter().copied().enumerate() {
            assert_eq!(actual, full[k] * scale, "s {s} v {v} frame {k}");
        }
    }
}

#[test]
fn a_velocity_scaler_applies_v1s_output_law_after_the_envelope() {
    // ADR-0059 clauses 3 and 4: with a scaler after the amplifier the note is scaled twice —
    // the envelope's `1 − s_env × (1 − v)` and then the scaler's `(1 − s_out) + s_out × v` —
    // and at both defaults that is V1's `velocity²`. The oracle applies the two `f32` factors
    // to the full-velocity render in the order the graph applies them. A scaler declares a
    // second velocity destination, so the note's expansion is three writes.
    let reference = admit(&voice_composed(
        1,
        StealingPolicy::None,
        Seconds::ZERO,
        NormalizedLevel::FULL,
        None,
    ));
    let full = render(&reference, &[note(&reference, 60, 0, 30 * Q)], 8);
    let plan = admit(&voice_composed(
        1,
        StealingPolicy::None,
        Seconds::ZERO,
        NormalizedLevel::FULL,
        Some(NormalizedLevel::FULL),
    ));
    let slot = plan.resolve_note(ENVELOPE).expect("playable");
    assert_eq!(
        plan.note_magnitudes_of(slot).len(),
        3,
        "a pitch and two velocity destinations"
    );
    for v in [0.25_f32, 0.5, 0.7559055] {
        let rendered = render(
            &plan,
            &[struck(&plan, 60, NoteVelocity::saturating(v), 0, 30 * Q)],
            8,
        );
        let envelope_scale = 1.0 - 1.0 * (1.0 - v);
        let output_scale = (1.0 - 1.0) + 1.0 * v;
        for (k, actual) in rendered.iter().copied().enumerate() {
            assert_eq!(
                actual,
                (full[k] * envelope_scale) * output_scale,
                "v {v} frame {k}"
            );
        }
    }
    // And a scaler at sensitivity zero is the identity: one factor only.
    let plain = admit(&voice_composed(
        1,
        StealingPolicy::None,
        Seconds::ZERO,
        NormalizedLevel::FULL,
        Some(NormalizedLevel::new(0.0).expect("in range")),
    ));
    let once = render(
        &plain,
        &[struck(&plain, 60, NoteVelocity::saturating(0.5), 0, 30 * Q)],
        8,
    );
    let expected: Vec<f32> = full.iter().map(|x| x * 0.5).collect();
    assert_eq!(once, expected);
}

#[test]
fn a_one_voice_stealing_plan_sums_its_single_voice_so_the_fade_has_a_step() {
    // With one voice there is no voice sum to fade on unless the compiler inserts one, and
    // ADR-0058 makes it insert one for a stealing plan. The second note steals the first,
    // through the same fade-then-start as on a wider plan.
    let plan = admit(&voice_with(
        1,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
    ));
    assert_eq!(
        plan.sum_groups().len(),
        1,
        "one sum group, of one copy step"
    );
    assert_eq!(
        plan.instance_groups().len(),
        4,
        "three voice nodes and the sum"
    );
    let p = 3 * Q + 17;
    let a = note(&plan, 60, 0, 30 * Q);
    let c = note(&plan, 72, p, 30 * Q);
    let alone_a = render(&plan, std::slice::from_ref(&a), 16);
    let alone_c = render(&plan, &[note(&plan, 72, 0, 30 * Q)], 16);
    let together = render(&plan, &[a, c], 16);
    let fade_from = (p + Q) as usize;
    let fade_to = fade_from + FADE as usize;
    let shift = (p + FADE) as usize;
    for (k, actual) in together.iter().copied().enumerate() {
        let expected = if k < fade_from {
            alone_a[k]
        } else if k < fade_to {
            alone_a[k] * fade_gain((k - fade_from) as u64)
        } else {
            alone_c[k - shift]
        };
        assert_eq!(actual, expected, "frame {k}");
    }
}

#[test]
fn a_plan_declaring_no_stealing_refuses_a_full_producer_as_before() {
    // `None` is today's behaviour: the third note on a two-voice plan is an over-emission,
    // refused at preparation with the minter's own cause, and nothing renders.
    let plan = admit(&voice(2));
    let events: Vec<PlanEvent> = [
        note(&plan, 60, 0, 30 * Q),
        note(&plan, 67, Q, 30 * Q),
        note(&plan, 72, 2 * Q, 30 * Q),
    ]
    .iter()
    .flatten()
    .copied()
    .collect();
    let mut sorted = events;
    sorted.sort_by_key(|event| event.position());
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(&plan, &sorted).expect("the stream fits");
    match CompiledEventScheduler::prepare(&mut control, &stream) {
        Err(crate::schedule::SchedulePrepareError::Identity {
            event_index: 2,
            source: crate::identity::IdentityError::ProducerOverEmitted { .. },
        }) => {}
        other => panic!("expected the over-emission refusal, got {other:?}"),
    }
}

#[test]
fn a_steal_whose_expansion_overruns_the_compiled_share_is_refused_at_preparation() {
    // ADR-0058 clause 7: the reset and the new note land `fade` frames after the note-on,
    // and they are charged against the compiled share there. Fill that quantum with the
    // share's worth of writes and the source stream still admits — each quantum holds no
    // more than the share — but the expansion puts two more into it, and preparation
    // refuses by name.
    let plan = admit(&voice_with(
        2,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
    ));
    let share = plan.compiled_event_share().get() as u64;
    let p = 4 * Q;
    let gate = plan
        .resolve_parameter(ENVELOPE, crate::ir::parameters::ENVELOPE_GATE)
        .expect("the gate is addressable");
    let mut events: Vec<PlanEvent> = [
        note(&plan, 60, 0, 30 * Q),
        note(&plan, 67, Q, 30 * Q),
        note(&plan, 72, p, 30 * Q),
    ]
    .iter()
    .flatten()
    .copied()
    .collect();
    events.extend((0..share).map(|_| {
        PlanEvent::new(
            PlanPosition::new(p + FADE),
            CompiledPayload::SetParameter {
                slot: gate,
                value: crate::quantities::ParameterValue::ONE,
            },
        )
    }));
    events.sort_by_key(|event| event.position());
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("the source stream fits");
    assert!(matches!(
        CompiledEventScheduler::prepare(&mut control, &stream),
        Err(crate::schedule::SchedulePrepareError::StealsOverrunShare { .. })
    ));
}

#[test]
fn the_steal_expansion_is_derived_alike_from_the_ir_and_the_plan() {
    // Admission charges from the IR and preparation from the plan, so the two must be one
    // figure: three voice-scope nodes and one voice sum reset per steal on this voice, and
    // one write — nothing to charge — where the plan does not steal.
    let stealing = voice_with(
        4,
        StealingPolicy::Oldest {
            fade: FrameCount::new(FADE),
        },
    );
    let plan = admit(&stealing);
    assert_eq!(stealing.steal_expansion().get(), 4);
    assert_eq!(plan.steal_expansion(), stealing.steal_expansion());
    let plain = voice(4);
    assert_eq!(
        plain.steal_expansion(),
        crate::quantities::WritesPerNote::GATE_ONLY
    );
    assert_eq!(admit(&plain).steal_expansion(), plain.steal_expansion());
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
