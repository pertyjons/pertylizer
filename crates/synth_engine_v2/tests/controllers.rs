//! P07-S004: observable source values, layer order, cadence and occurrence ownership.
mod common;

use synth_engine_v2::controller::{
    BipolarLevel, ControllerKind, MidiController, NoteExpression, NoteSource,
};
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::node::AMPLIFIER_CONTROL;
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, KeyIdentity, NormalizedLevel, NoteVelocity, ParameterValue, Seconds,
};
use synth_engine_v2::schedule::CompiledPayload;
use synth_engine_v2::time::{FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime};

const SOURCE: NodeId = NodeId::new(10);
const CONSTANT: NodeId = NodeId::new(11);
const AMP: NodeId = NodeId::new(12);
const GATE: NodeId = NodeId::new(13);
const OUT: NodeId = NodeId::new(14);
/// An amplifier the gate drives, read by nothing: what puts the gate in the source's
/// island (`P08-S002`), since a note's destinations are the played node's island.
const GATE_SINK: NodeId = NodeId::new(15);
const Q: usize = QUANTUM_FRAMES as usize;
const FRAMES: usize = 12 * Q;

/// Reading a source through a constant times its control output exposes its exact value.
fn source_plan(kind: IrNodeKind, voices: u32) -> CompiledPlan {
    source_declaring(kind, common::compiled_notes(voices))
}
fn source_declaring(
    kind: IrNodeKind,
    declarations: synth_engine_v2::ir::PlanDeclarations,
) -> CompiledPlan {
    let scope = if matches!(kind, IrNodeKind::NoteSource { .. }) {
        ExecutionScope::Voice
    } else {
        ExecutionScope::Global
    };
    let mut builder = GraphIr::builder()
        .node(NodeId::new(1), IrNodeKind::Silence, ExecutionScope::Voice)
        .node(SOURCE, kind, scope)
        .node(
            CONSTANT,
            IrNodeKind::Constant {
                level: Amplitude::UNITY,
            },
            scope,
        )
        .node(AMP, IrNodeKind::Amplifier, scope)
        .node(OUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (CONSTANT, PortId::FIRST),
            (AMP, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SOURCE, PortId::FIRST),
            (AMP, AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMP, PortId::FIRST),
            (OUT, PortId::FIRST),
            SignalDomain::Audio,
        );
    if scope == ExecutionScope::Voice {
        builder = builder
            .node(
                GATE,
                IrNodeKind::Envelope {
                    attack: Seconds::ZERO,
                    decay: Seconds::ZERO,
                    sustain: NormalizedLevel::FULL,
                    release: Seconds::ZERO,
                    velocity_sensitivity: NormalizedLevel::FULL,
                },
                scope,
            )
            .node(GATE_SINK, IrNodeKind::Amplifier, scope)
            .connect(
                (CONSTANT, PortId::FIRST),
                (GATE_SINK, PortId::FIRST),
                SignalDomain::Audio,
            )
            .connect(
                (GATE, PortId::FIRST),
                (GATE_SINK, AMPLIFIER_CONTROL),
                SignalDomain::Control,
            );
    }
    common::admit(
        &builder.declaring(declarations).build().expect("IR"),
        common::profile(FRAMES as u64, ChannelLayout::Mono),
    )
}
fn event(frame: usize, payload: CompiledPayload) -> OfflineEvent {
    OfflineEvent::new(SampleTime::new(frame as u64), payload)
}
fn render(plan: &CompiledPlan, events: &[OfflineEvent]) -> Vec<f32> {
    render_offline(
        plan.clone(),
        FrameCount::new(FRAMES as u64),
        PlanPosition::ZERO,
        events,
    )
    .expect("render")
}
fn control(plan: &CompiledPlan, value: Option<f32>) -> CompiledPayload {
    CompiledPayload::Controller(
        plan.resolve_controller(SOURCE)
            .expect("declared")
            .change(value.map(|v| BipolarLevel::new(v).expect("bipolar")))
            .expect("valid for controller"),
    )
}
fn parameter(plan: &CompiledPlan, value: f32) -> CompiledPayload {
    CompiledPayload::SetParameter {
        slot: plan
            .resolve_parameter(SOURCE, parameters::SOURCE_VALUE)
            .expect("source value"),
        value: ParameterValue::new(value).expect("finite"),
    }
}
fn note(plan: &CompiledPlan, key: u8, velocity: f32) -> CompiledPayload {
    CompiledPayload::NoteOn {
        slot: plan.resolve_note(GATE).expect("playable"),
        key: KeyIdentity::new(key).expect("key"),
        velocity: NoteVelocity::new(velocity).expect("velocity"),
    }
}
fn expression(plan: &CompiledPlan, key: u8, expression: NoteExpression) -> CompiledPayload {
    CompiledPayload::Expression {
        slot: plan.resolve_note(GATE).expect("playable"),
        key: KeyIdentity::new(key).expect("key"),
        expression,
    }
}
fn assert_frames(samples: &[f32], first: usize, end: usize, value: f32) {
    assert!(
        samples[first..end]
            .iter()
            .all(|v| v.to_bits() == value.to_bits()),
        "frames {first}..{end} should be {value}, first values {:?}",
        &samples[first..(first + 8).min(end)]
    );
}

#[test]
fn controller_replaces_automation_until_cleared_and_changes_at_the_next_boundary() {
    for kind in [
        ControllerKind::ModWheel,
        ControllerKind::Aftertouch,
        ControllerKind::PitchBend,
        ControllerKind::MidiCc(MidiController::new(74).expect("CC")),
    ] {
        let plan = source_plan(IrNodeKind::Controller { kind }, 1);
        let samples = render(
            &plan,
            &[
                event(0, parameter(&plan, 0.25)),
                event(1, control(&plan, Some(0.75))),
                event(Q + 1, parameter(&plan, 0.5)),
                event(2 * Q + 1, control(&plan, None)),
            ],
        );
        assert_frames(&samples, 0, Q, 0.25);
        assert_frames(&samples, Q, 3 * Q, 0.75);
        assert_frames(&samples, 3 * Q, FRAMES, 0.5);
        assert!(plan.resolve_controller(CONSTANT).is_none());
        assert_eq!(
            plan.resolve_controller(SOURCE).expect("controller").kind(),
            kind
        );
    }
}

#[test]
fn controller_values_reject_invalid_input_and_pitch_bend_keeps_its_sign() {
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.01, 1.01] {
        assert!(BipolarLevel::new(invalid).is_err());
    }
    assert!(MidiController::new(128).is_err());
    assert_eq!(MidiController::new(127).expect("CC").as_u8(), 127);
    let plan = source_plan(
        IrNodeKind::Controller {
            kind: ControllerKind::PitchBend,
        },
        1,
    );
    assert_frames(
        &render(&plan, &[event(0, control(&plan, Some(-0.5)))]),
        0,
        FRAMES,
        -0.5,
    );
    let wheel = source_plan(
        IrNodeKind::Controller {
            kind: ControllerKind::ModWheel,
        },
        1,
    );
    assert!(
        wheel
            .resolve_controller(SOURCE)
            .expect("controller")
            .change(Some(BipolarLevel::new(-0.5).expect("bipolar")))
            .is_err()
    );
}

#[test]
fn note_sources_read_their_own_key_velocity_pressure_and_release_velocity() {
    for (kind, value) in [
        (NoteSource::Velocity, 0.25),
        (NoteSource::NoteNumber, 60.0 / 127.0),
        (NoteSource::Pressure, 0.0),
        (NoteSource::ReleaseVelocity, 0.0),
    ] {
        let plan = source_plan(IrNodeKind::NoteSource { kind }, 1);
        let samples = render(&plan, &[event(1, note(&plan, 60, 0.25))]);
        assert_frames(&samples, 0, Q, 0.0);
        assert_frames(&samples, Q, FRAMES, value);
        assert!(plan.resolve_controller(SOURCE).is_none());
    }
    for kind in [NoteSource::Pressure, NoteSource::ReleaseVelocity] {
        let plan = source_plan(IrNodeKind::NoteSource { kind }, 2);
        let value = NormalizedLevel::new(0.25).expect("normalized");
        let expr = if kind == NoteSource::Pressure {
            NoteExpression::Pressure(value)
        } else {
            NoteExpression::ReleaseVelocity(value)
        };
        let samples = render(
            &plan,
            &[
                event(0, note(&plan, 60, 1.0)),
                event(0, note(&plan, 64, 1.0)),
                event(Q + 1, expression(&plan, 60, expr)),
                event(3 * Q, expression(&plan, 64, expr)),
            ],
        );
        assert_frames(&samples, 0, 2 * Q, 0.0);
        assert_frames(&samples, 2 * Q, 3 * Q, 0.25);
        assert_frames(&samples, 3 * Q, FRAMES, 0.5);
    }
}

#[test]
fn a_new_occurrence_resets_expression_and_a_release_keeps_its_magnitude() {
    let plan = source_plan(
        IrNodeKind::NoteSource {
            kind: NoteSource::ReleaseVelocity,
        },
        1,
    );
    let slot = plan.resolve_note(GATE).expect("playable");
    let key = KeyIdentity::new(60).expect("key");
    let samples = render(
        &plan,
        &[
            event(0, note(&plan, 60, 1.0)),
            event(
                Q + 1,
                expression(
                    &plan,
                    60,
                    NoteExpression::ReleaseVelocity(NormalizedLevel::FULL),
                ),
            ),
            event(Q + 1, CompiledPayload::NoteOff { slot, key }),
            event(3 * Q + 1, note(&plan, 64, 1.0)),
        ],
    );
    assert_frames(&samples, 0, 2 * Q, 0.0);
    assert_frames(&samples, 2 * Q, 4 * Q, 1.0);
    assert_frames(&samples, 4 * Q, FRAMES, 0.0);
    assert!(
        render_offline(
            plan.clone(),
            FrameCount::new(FRAMES as u64),
            PlanPosition::ZERO,
            &[event(
                0,
                expression(&plan, 60, NoteExpression::Pressure(NormalizedLevel::FULL))
            )]
        )
        .is_err()
    );
}

fn scheduled(
    plan: &CompiledPlan,
    events: &[synth_engine_v2::schedule::PlanEvent],
    block: usize,
    seek: Option<usize>,
) -> Vec<f32> {
    use synth_engine_v2::schedule::{AdmittedCompiledStream, CompiledEventScheduler};
    use synth_engine_v2::stream::{ActivationRequest, StreamControl};
    use synth_engine_v2::time::StreamAnchor;
    let origin = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);
    let (mut control, mut renderer) = StreamControl::open(plan.clone(), origin).expect("open");
    let events = AdmittedCompiledStream::admit(plan, events).expect("admitted");
    let empty = AdmittedCompiledStream::admit(plan, &[]).expect("empty");
    let mut scheduler = CompiledEventScheduler::prepare(
        &mut control,
        if seek.is_some() { &empty } else { &events },
    )
    .expect("schedule");
    if let Some(position) = seek {
        let activation = control
            .plan_activation(
                &events,
                ActivationRequest {
                    at: SampleTime::ZERO,
                    position: PlanPosition::new(position as u64),
                    loop_interval: None,
                },
            )
            .expect("activation");
        scheduler.offer(&mut renderer, activation).expect("offer");
    }
    let mut arbiter = synth_engine_v2::publish::PublicationArbiter::prepare(&common::profile(
        FRAMES as u64,
        ChannelLayout::Mono,
    ))
    .expect("arbiter");
    let mut output = vec![0.0; FRAMES];
    for samples in output.chunks_mut(block) {
        let frames = samples.len();
        scheduler
            .render(
                &mut renderer,
                &mut arbiter,
                synth_engine_v2::render::AudioBlockMut::new(samples, frames, ChannelLayout::Mono)
                    .expect("block"),
            )
            .expect("render");
    }
    output
}

#[test]
fn a_seek_restores_controller_and_automation_as_separate_layers() {
    use synth_engine_v2::schedule::PlanEvent;
    let plan = source_plan(
        IrNodeKind::Controller {
            kind: ControllerKind::ModWheel,
        },
        2,
    );
    let events = [
        PlanEvent::new(PlanPosition::ZERO, parameter(&plan, 0.25)),
        PlanEvent::new(PlanPosition::new(1), control(&plan, Some(0.75))),
        PlanEvent::new(PlanPosition::new(2), parameter(&plan, 0.5)),
        PlanEvent::new(PlanPosition::new((4 * Q) as u64), control(&plan, None)),
    ];
    for block in [FRAMES, 256, Q, 17] {
        let samples = scheduled(&plan, &events, block, Some(2 * Q));
        assert_frames(&samples, 0, Q, 0.0);
        assert_frames(&samples, Q, 3 * Q, 0.75);
        assert_frames(&samples, 3 * Q, FRAMES, 0.5);
    }
}

#[test]
fn controller_and_expression_schedules_are_bit_identical_under_host_partitions() {
    use synth_engine_v2::schedule::PlanEvent;
    let plan = source_plan(
        IrNodeKind::NoteSource {
            kind: NoteSource::Pressure,
        },
        2,
    );
    let events = [
        PlanEvent::new(PlanPosition::new(1), note(&plan, 60, 0.5)),
        PlanEvent::new(PlanPosition::new(2), note(&plan, 64, 0.75)),
        PlanEvent::new(
            PlanPosition::new((Q + 1) as u64),
            expression(&plan, 60, NoteExpression::Pressure(NormalizedLevel::FULL)),
        ),
        PlanEvent::new(
            PlanPosition::new((2 * Q) as u64),
            expression(
                &plan,
                64,
                NoteExpression::Pressure(NormalizedLevel::new(0.25).expect("level")),
            ),
        ),
    ];
    let reference = scheduled(&plan, &events, FRAMES, None);
    assert_frames(&reference, 3 * Q, FRAMES, 1.25);
    for block in [256, Q, 17] {
        assert_eq!(scheduled(&plan, &events, block, None), reference);
    }
}

#[test]
fn a_controller_source_modulates_pitch_through_the_same_law_as_an_lfo() {
    use synth_engine_v2::ir::{ModulationDepth, ModulationUnit};
    use synth_engine_v2::quantities::Frequency;
    let sine = IrNodeKind::Sine {
        frequency: Frequency::new(220.0).expect("Hz"),
        amplitude: Amplitude::UNITY,
    };
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Controller {
                kind: ControllerKind::PitchBend,
            },
            ExecutionScope::Global,
        )
        .node(CONSTANT, sine, ExecutionScope::Global)
        .node(OUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (CONSTANT, PortId::FIRST),
            (OUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .modulate(
            (SOURCE, PortId::FIRST),
            (CONSTANT, parameters::SINE_FREQUENCY),
            ModulationDepth::new(ModulationUnit::Semitones, 12.0).expect("depth"),
        )
        .build()
        .expect("IR");
    let plan = common::admit(&ir, common::profile(FRAMES as u64, ChannelLayout::Mono));
    let observed = render(&plan, &[event(Q + 1, control(&plan, Some(-1.0)))]);
    let plain = common::admit(
        &common::source_plan(sine),
        common::profile(FRAMES as u64, ChannelLayout::Mono),
    );
    let frequency = plain
        .resolve_parameter(common::SOURCE, parameters::SINE_FREQUENCY)
        .expect("frequency");
    let expected = render(
        &plain,
        &[event(
            2 * Q,
            CompiledPayload::SetParameter {
                slot: frequency,
                value: ParameterValue::new(110.0).expect("value"),
            },
        )],
    );
    assert_eq!(observed, expected);
    assert_ne!(observed, render(&plain, &[]));
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn live_expression_updates_survive_a_deferred_start_without_coalescing_or_losing_the_release() {
    use synth_engine_v2::identity::ProducerId;
    use synth_engine_v2::ingress::PerformanceIngress;
    use synth_engine_v2::ir::{NoteProducerDeclaration, PlanDeclarations, StealingPolicy};
    use synth_engine_v2::quantities::{EventCount, HeldNoteCount};
    use synth_engine_v2::schedule::{AdmittedCompiledStream, CompiledEventScheduler, PlanEvent};
    use synth_engine_v2::stream::StreamControl;
    use synth_engine_v2::time::StreamAnchor;
    let kind = IrNodeKind::NoteSource {
        kind: NoteSource::ReleaseVelocity,
    };
    let stealing = StealingPolicy::Oldest {
        fade: FrameCount::new((2 * Q) as u64),
    };
    let compiled = source_declaring(
        kind,
        PlanDeclarations {
            stealing,
            ..common::compiled_notes(1)
        },
    );
    let release =
        |level| NoteExpression::ReleaseVelocity(NormalizedLevel::new(level).expect("level"));
    let events = [
        PlanEvent::new(PlanPosition::ZERO, note(&compiled, 60, 1.0)),
        PlanEvent::new(PlanPosition::new(1), note(&compiled, 64, 1.0)),
        PlanEvent::new(
            PlanPosition::new(2),
            expression(&compiled, 64, release(0.25)),
        ),
        PlanEvent::new(
            PlanPosition::new((Q + 2) as u64),
            expression(&compiled, 64, release(0.75)),
        ),
        PlanEvent::new(
            PlanPosition::new((4 * Q + 2) as u64),
            expression(&compiled, 64, release(0.5)),
        ),
        PlanEvent::new(
            PlanPosition::new((4 * Q + 2) as u64),
            CompiledPayload::NoteOff {
                slot: compiled.resolve_note(GATE).expect("gate"),
                key: KeyIdentity::new(64).expect("key"),
            },
        ),
        PlanEvent::new(
            PlanPosition::new((6 * Q + 2) as u64),
            parameter(&compiled, 0.9),
        ),
    ];
    let reference = scheduled(&compiled, &events, Q, None);
    assert_frames(&reference, 4 * Q, 5 * Q, 0.25);
    assert_frames(&reference, 5 * Q, 8 * Q, 0.75);
    assert_frames(&reference, 8 * Q, FRAMES, 0.9);
    for (block, prefetch) in [(FRAMES, true), (Q, true), (17, true), (Q, false)] {
        let plan = source_declaring(
            kind,
            PlanDeclarations {
                stealing,
                note_producers: vec![NoteProducerDeclaration {
                    compiled: false,
                    simultaneous_notes: HeldNoteCount::measured(1),
                    simultaneous_holds: EventCount::measured(1),
                }],
                ..PlanDeclarations::default()
            },
        );
        let host = common::profile(FRAMES as u64, ChannelLayout::Mono);
        let (mut control, mut renderer) = StreamControl::open(
            plan.clone(),
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        )
        .expect("open");
        let mut store = PerformanceIngress::prepare(&host, &plan, ProducerId::new(0), &renderer)
            .expect("store");
        let empty = AdmittedCompiledStream::admit(&plan, &[]).expect("empty");
        let mut scheduler =
            CompiledEventScheduler::prepare(&mut control, &empty).expect("scheduler");
        let slot = plan.resolve_note(GATE).expect("gate");
        let first = control
            .offer_note_on(
                &mut store,
                SampleTime::ZERO,
                slot,
                KeyIdentity::new(60).expect("key"),
                NoteVelocity::FULL,
            )
            .expect("first");
        let second = control
            .offer_note_on(
                &mut store,
                SampleTime::new(1),
                slot,
                KeyIdentity::new(64).expect("key"),
                NoteVelocity::FULL,
            )
            .expect("steal");
        assert!(
            control
                .offer_expression(&mut store, SampleTime::new(2), first, release(1.0))
                .is_err()
        );
        control
            .offer_expression(&mut store, SampleTime::new(2), second, release(0.25))
            .expect("first update");
        control
            .offer_expression(
                &mut store,
                SampleTime::new((Q + 2) as u64),
                second,
                release(0.75),
            )
            .expect("second update");
        assert!(matches!(
            control.offer_expression(
                &mut store,
                SampleTime::new((Q + 1) as u64),
                second,
                release(0.5)
            ),
            Err(synth_engine_v2::ingress::IngressRefused::NonMonotoneStamp { .. })
        ));
        let offer_sustain = |control: &mut StreamControl, store: &mut PerformanceIngress| {
            let at = SampleTime::new((4 * Q + 2) as u64);
            control
                .offer_expression(store, at, second, release(0.5))
                .expect("sustain update");
            control
                .offer_note_off(store, at, second)
                .expect("release hold");
            assert!(matches!(
                control.offer_expression(store, at, second, release(1.0)),
                Err(synth_engine_v2::ingress::IngressRefused::OrphanExpression { .. })
            ));
            // Offered after the displaced expression, at its effective timestamp: this
            // ordinary write must remain last even though it occupies the other ring.
            control
                .offer_parameter(
                    store,
                    SampleTime::new((6 * Q + 2) as u64),
                    plan.resolve_parameter(SOURCE, parameters::SOURCE_VALUE)
                        .expect("source slot"),
                    ParameterValue::new(0.9).expect("finite"),
                )
                .expect("same-position ordinary write");
        };
        if prefetch {
            offer_sustain(&mut control, &mut store);
        }
        let mut arbiter =
            synth_engine_v2::publish::PublicationArbiter::prepare(&host).expect("arbiter");
        let mut samples = vec![0.0; FRAMES];
        for (chunk, out) in samples.chunks_mut(block).enumerate() {
            if !prefetch && chunk * block == 4 * Q {
                // The pending start has already been published. Fixed note displacement
                // must not depend on whether the host prefetched this update.
                offer_sustain(&mut control, &mut store);
            }
            let frames = out.len();
            scheduler
                .render_with_ingress(
                    &mut renderer,
                    &mut arbiter,
                    Some(&mut store),
                    synth_engine_v2::render::AudioBlockMut::new(out, frames, ChannelLayout::Mono)
                        .expect("block"),
                )
                .expect("render");
        }
        assert_eq!(samples, reference);
        assert_eq!(store.counters().dropped(), 0);
        assert!(store.is_empty());
    }
}

#[test]
fn a_per_note_source_cannot_be_shared_outside_the_voice_scope() {
    for scope in [
        ExecutionScope::Global,
        ExecutionScope::Channel,
        ExecutionScope::InstrumentInstance,
    ] {
        let ir = GraphIr::builder()
            .node(
                SOURCE,
                IrNodeKind::NoteSource {
                    kind: NoteSource::Pressure,
                },
                scope,
            )
            .build()
            .expect("IR");
        assert!(matches!(
            common::refuse(&ir, common::profile(FRAMES as u64, ChannelLayout::Mono)),
            synth_engine_v2::diagnostics::CompileError::NoteSourceOutsideVoice { node: SOURCE, .. }
        ));
    }
}

#[test]
fn a_seek_ends_the_old_occurrences_source_state() {
    use synth_engine_v2::schedule::PlanEvent;
    let plan = source_plan(
        IrNodeKind::NoteSource {
            kind: NoteSource::Velocity,
        },
        2,
    );
    let samples = scheduled(
        &plan,
        &[PlanEvent::new(PlanPosition::ZERO, note(&plan, 60, 1.0))],
        Q,
        Some(2 * Q),
    );
    assert_frames(&samples, 0, FRAMES, 0.0);
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn delayed_expressions_and_controllers_cannot_borrow_the_reserved_release_slot() {
    use synth_engine_v2::identity::ProducerId;
    use synth_engine_v2::ingress::{ExhaustedResource, IngressRefused, PerformanceIngress};
    use synth_engine_v2::ir::{NoteProducerDeclaration, PlanDeclarations, StealingPolicy};
    use synth_engine_v2::quantities::{EventCount, HeldNoteCount};
    use synth_engine_v2::stream::StreamControl;
    use synth_engine_v2::time::StreamAnchor;
    let host = common::profile(FRAMES as u64, ChannelLayout::Mono);
    let capacity = host
        .limits()
        .events()
        .queues()
        .performance_ingress_capacity()
        .get() as usize;
    let live = || PlanDeclarations {
        stealing: StealingPolicy::Oldest {
            fade: FrameCount::new((2 * Q) as u64),
        },
        note_producers: vec![NoteProducerDeclaration {
            compiled: false,
            simultaneous_notes: HeldNoteCount::measured(1),
            simultaneous_holds: EventCount::measured(1),
        }],
        ..PlanDeclarations::default()
    };
    let plan = source_declaring(
        IrNodeKind::NoteSource {
            kind: NoteSource::Pressure,
        },
        live(),
    );
    let (mut control, renderer) = StreamControl::open(
        plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("open");
    let mut store =
        PerformanceIngress::prepare(&host, &plan, ProducerId::new(0), &renderer).expect("store");
    let slot = plan.resolve_note(GATE).expect("gate");
    let _ = control
        .offer_note_on(
            &mut store,
            SampleTime::ZERO,
            slot,
            KeyIdentity::new(60).expect("key"),
            NoteVelocity::FULL,
        )
        .expect("first");
    let identity = control
        .offer_note_on(
            &mut store,
            SampleTime::new(1),
            slot,
            KeyIdentity::new(64).expect("key"),
            NoteVelocity::FULL,
        )
        .expect("steal");
    // The original on, fade, reset, delayed on and held release consume five entitlements.
    for _ in 0..capacity - 5 {
        control
            .offer_expression(
                &mut store,
                SampleTime::new(2),
                identity,
                NoteExpression::Pressure(NormalizedLevel::FULL),
            )
            .expect("fits");
    }
    assert!(matches!(
        control.offer_expression(
            &mut store,
            SampleTime::new(2),
            identity,
            NoteExpression::Pressure(NormalizedLevel::FULL)
        ),
        Err(IngressRefused::Dropped {
            resource: ExhaustedResource::Slot
        })
    ));
    control
        .offer_note_off(&mut store, SampleTime::new(2), identity)
        .expect("reserved release still fits");
    assert_eq!(store.counters().dropped(), 1);
    let controller_plan = source_declaring(
        IrNodeKind::Controller {
            kind: ControllerKind::ModWheel,
        },
        live(),
    );
    let (mut control, renderer) = StreamControl::open(
        controller_plan.clone(),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("open");
    let mut store =
        PerformanceIngress::prepare(&host, &controller_plan, ProducerId::new(0), &renderer)
            .expect("store");
    let change = controller_plan
        .resolve_controller(SOURCE)
        .expect("controller")
        .change(Some(BipolarLevel::ZERO))
        .expect("valid");
    for _ in 0..capacity {
        control
            .offer_controller(&mut store, SampleTime::ZERO, change)
            .expect("fits");
    }
    assert!(matches!(
        control.offer_controller(&mut store, SampleTime::ZERO, change),
        Err(IngressRefused::Dropped {
            resource: ExhaustedResource::Slot
        })
    ));
    assert_eq!(store.len(), capacity);
}

#[test]
fn controller_discovery_uses_prepared_identity_after_polyphonic_nodes() {
    let ir = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Controller {
                kind: ControllerKind::ModWheel,
            },
            ExecutionScope::Voice,
        )
        .node(
            NodeId::new(2),
            IrNodeKind::Controller {
                kind: ControllerKind::Aftertouch,
            },
            ExecutionScope::Voice,
        )
        .node(OUT, IrNodeKind::Output, ExecutionScope::Global)
        .declaring(common::compiled_notes(4))
        .build()
        .expect("IR");
    let plan = common::admit(&ir, common::profile(FRAMES as u64, ChannelLayout::Mono));
    for (node, expected) in [
        (1, ControllerKind::ModWheel),
        (2, ControllerKind::Aftertouch),
    ] {
        assert_eq!(
            plan.resolve_controller(NodeId::new(node))
                .expect("declared")
                .kind(),
            expected
        );
    }
}
