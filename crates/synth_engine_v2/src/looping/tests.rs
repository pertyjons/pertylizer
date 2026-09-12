//! Independent sample and boundary oracles for the exclusive loop owner.
#[path = "journal/tests.rs"]
mod journal_tests;
use super::*;
use crate::{
    compile::{RenderConfig, compile},
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    profile::HostProfile,
    quantities::{ChannelLayout, SampleRate},
    render::AudioBlockMut,
    schedule::AdmittedCompiledStream,
};

fn impulse(length: u64, entry: u64, target: u64, maximum: u64) -> CompiledLoopStream {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(maximum),
        ChannelLayout::Mono,
    )
    .unwrap();
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Impulse {
                position: PlanPosition::new(target),
            },
            ExecutionScope::Global,
        )
        .node(NodeId::new(2), IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(2), PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .unwrap();
    let plan = compile(&graph, &RenderConfig::new(profile))
        .into_plan()
        .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    CompiledLoopStream::prepare(
        plan,
        stream,
        profile,
        LoopSettings::new(
            LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(length)).unwrap(),
            PlanPosition::new(entry),
            PreparedBytes::limit(1_000_000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn samples_and_pass_boundaries_match_the_oracle_across_host_partitions() {
    const TOTAL: usize = 2112;
    for length in [1_u64, 2, 63, 64, 65, 127, 513] {
        for entry in [0, length - 1, length] {
            for target in [0, length - 1] {
                let entry_position = if entry == length { 0 } else { entry };
                for partition in [1_usize, 37, 64, 256, 512] {
                    let mut owner = impulse(length, entry, target, 512);
                    let mut output = [9.0; TOTAL];
                    let mut boundaries = Vec::new();
                    for chunk in output.chunks_mut(partition) {
                        let frames = chunk.len();
                        let mut result = Ok(());
                        assert_eq!(
                            crate::render_allocation::count_allocs(|| {
                                result = owner.render(
                                    AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).unwrap(),
                                );
                            }),
                            0
                        );
                        result.unwrap();
                        boundaries.extend(owner.boundaries().copied());
                    }
                    // The renderer's fixed one-quantum latency is independent of
                    // the host partition; the first 64 delivered samples are priming.
                    for (frame, sample) in output.iter().enumerate() {
                        let expected = if frame < 64 {
                            0.0
                        } else {
                            f32::from((entry_position + (frame - 64) as u64) % length == target)
                        };
                        assert_eq!(
                            *sample, expected,
                            "L={length}, entry={entry}, target={target}, partition={partition}, frame={frame}"
                        );
                    }
                    let expected: Vec<_> = (length - entry_position..2048)
                        .step_by(length as usize)
                        .collect();
                    assert_eq!(
                        boundaries.iter().map(|b| b.at.as_u64()).collect::<Vec<_>>(),
                        expected
                    );
                    for (index, boundary) in boundaries.iter().enumerate() {
                        assert_eq!(boundary.previous.as_u64(), index as u64 + 1);
                        assert_eq!(boundary.next.as_u64(), index as u64 + 2);
                        assert_eq!(boundary.epoch, owner.snapshot().epoch);
                    }
                    assert_eq!(owner.snapshot().clock.as_u64(), 2048);
                    assert_eq!(owner.snapshot().pass.as_u64(), boundaries.len() as u64 + 1);
                    assert_eq!(owner.fault(), None);
                }
            }
        }
    }
}

#[test]
fn host_maximum_below_a_quantum_still_has_room_for_every_internal_wrap() {
    let mut owner = impulse(1, 0, 0, 1);
    let mut output = [9.0];
    for call in 0..65 {
        owner
            .render(AudioBlockMut::new(&mut output, 1, ChannelLayout::Mono).unwrap())
            .unwrap();
        assert_eq!(output[0], if call < 64 { 0.0 } else { 1.0 });
    }
    assert_eq!(owner.boundaries().count(), 63);
    assert_eq!(owner.snapshot().clock.as_u64(), 64);
}

#[test]
fn terminal_validation_and_pass_exhaustion_latch_and_silence_the_whole_callback() {
    let mut oversized = impulse(65, 0, 0, 64);
    let mut output = [9.0; 65];
    let error = oversized
        .render(AudioBlockMut::new(&mut output, 65, ChannelLayout::Mono).unwrap())
        .unwrap_err();
    assert!(matches!(
        error,
        LoopFault::Render(RenderError::OversizedCallback { .. })
    ));
    assert_eq!(oversized.fault(), Some(error));
    assert_eq!(output, [0.0; 65]);

    let mut owner = impulse(127, 0, 0, 512);
    owner.source.pass = LoopPassId(u64::MAX);
    let mut output = [9.0; 512];
    let mut result = Ok(());
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            result =
                owner.render(AudioBlockMut::new(&mut output, 512, ChannelLayout::Mono).unwrap());
        }),
        0
    );
    assert_eq!(result, Err(LoopFault::PassExhausted));
    assert_eq!(output, [0.0; 512]);
    assert_eq!(owner.boundaries().count(), 0);
    output.fill(9.0);
    assert_eq!(
        owner.render(AudioBlockMut::new(&mut output, 512, ChannelLayout::Mono).unwrap()),
        result
    );
    assert_eq!(output, [0.0; 512]);
}

#[test]
fn entry_outside_the_interval_refuses_and_exact_end_normalizes() {
    let interval = LoopInterval::new(PlanPosition::new(10), PlanPosition::new(20)).unwrap();
    for entry in [0, 9, 21, u64::MAX] {
        assert!(matches!(
            LoopSettings::new(
                interval,
                PlanPosition::new(entry),
                PreparedBytes::measured(0)
            ),
            Err(LoopPrepareError::Entry)
        ));
    }
    assert_eq!(
        LoopSettings::new(interval, PlanPosition::new(20), PreparedBytes::measured(0))
            .unwrap()
            .entry,
        PlanPosition::new(10)
    );
}

use crate::quantities::{Amplitude, EventCount, NormalizedLevel, Seconds};
use crate::schedule::{CompiledPayload, PlanEvent};
const SOURCE: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(2);
const ENVELOPE: NodeId = NodeId::new(11);
const AMPLIFIER: NodeId = NodeId::new(12);
fn gated_constant_declaring(declarations: crate::ir::PlanDeclarations) -> GraphIr {
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
                velocity_sensitivity: crate::quantities::NormalizedLevel::FULL,
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
            (AMPLIFIER, crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(declarations)
        .build()
        .expect("a readable plan")
}

fn notes(length: u64, entry: u64, edges: &[(u64, u8, bool)]) -> CompiledLoopStream {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .unwrap();
    let graph = gated_constant_declaring(crate::ir::PlanDeclarations {
        note_producers: vec![crate::ir::NoteProducerDeclaration {
            compiled: true,
            simultaneous_notes: HeldNoteCount::measured(8),
            simultaneous_holds: EventCount::NONE,
        }],
        ..crate::ir::PlanDeclarations::default()
    });
    let plan = compile(&graph, &RenderConfig::new(profile))
        .into_plan()
        .unwrap();
    let slot = plan.resolve_note(ENVELOPE).unwrap();
    let events: Vec<_> = edges
        .iter()
        .map(|&(position, key, on)| {
            PlanEvent::new(
                PlanPosition::new(position),
                if on {
                    CompiledPayload::NoteOn {
                        slot,
                        key: KeyIdentity::new(key).unwrap(),
                        velocity: NoteVelocity::FULL,
                    }
                } else {
                    CompiledPayload::NoteOff {
                        slot,
                        key: KeyIdentity::new(key).unwrap(),
                    }
                },
            )
        })
        .collect();
    let stream = AdmittedCompiledStream::admit(&plan, &events).unwrap();
    CompiledLoopStream::prepare(
        plan,
        stream,
        profile,
        LoopSettings::new(
            LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(length)).unwrap(),
            PlanPosition::new(entry),
            PreparedBytes::limit(1_000_000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn held_notes_end_before_each_new_pass_and_fresh_generations_never_alias() {
    for partition in [1, 37, 64, 256, 512] {
        let mut owner = notes(65, 0, &[(3, 60, true)]);
        let mut output = [9.0; 2112];
        for chunk in output.chunks_mut(partition) {
            let frames = chunk.len();
            let mut result = Ok(());
            assert_eq!(
                crate::render_allocation::count_allocs(|| {
                    result = owner
                        .render(AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).unwrap());
                }),
                0
            );
            result.unwrap();
        }
        for (frame, sample) in output.iter().enumerate() {
            let expected = f32::from(frame >= 64 && (frame - 64) % 65 >= 3);
            assert_eq!(*sample, expected, "partition={partition}, frame={frame}");
        }
        assert_eq!(owner.source.held_len, 1);
        assert_eq!(owner.renderer.diagnostics().orphan_note_events(), 0);
    }
    let mut owner = notes(65, 0, &[(3, 60, true)]);
    let mut output = [0.0; 128];
    owner
        .render(AudioBlockMut::new(&mut output, 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    let first = owner.source.tokens[0].unwrap();
    owner
        .render(AudioBlockMut::new(&mut output, 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    let later = owner.source.tokens[0].unwrap();
    assert_ne!(first, later);
    assert_ne!(
        owner.source.control.minter_mut().resolve(first),
        crate::identity::Resolution::Live
    );
}

#[test]
fn initial_entry_does_not_resurrect_crossing_notes_and_reports_omitted_releases() {
    let mut owner = notes(127, 40, &[(3, 60, true), (90, 60, false)]);
    assert_eq!(owner.omissions().0.releases.as_usize(), 1);
    assert_eq!(owner.omissions().1.releases.as_usize(), 0);
    let mut output = [9.0; 512];
    owner
        .render(AudioBlockMut::new(&mut output, 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    for (frame, sample) in output.iter().enumerate() {
        let expected = if frame < 64 + 87 {
            0.0
        } else {
            f32::from((3..90).contains(&((frame - 64 - 87) % 127)))
        };
        assert_eq!(*sample, expected, "frame={frame}");
    }
}

#[test]
fn compact_held_set_handles_middle_and_last_releases_before_wrap() {
    let mut owner = notes(
        127,
        0,
        &[
            (0, 60, true),
            (1, 61, true),
            (2, 62, true),
            (3, 61, false),
            (4, 62, false),
            (5, 60, false),
        ],
    );
    let mut output = [9.0; 512];
    owner
        .render(AudioBlockMut::new(&mut output, 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(owner.source.held_len, 0);
    assert!(owner.source.tokens.iter().all(Option::is_none));
    assert!(owner.source.held.iter().all(Option::is_none));
    assert_eq!(owner.source.control.minter_mut().live(), 0);
}

#[test]
fn generation_exhaustion_is_terminal_even_after_successful_quanta_in_this_callback() {
    let mut owner = notes(65, 0, &[(3, 60, true)]);
    owner.source.control.minter_mut().generation_ceiling = 0;
    let mut output = [9.0; 512];
    // Eight indices each mint once at generation zero, then retire. The ninth
    // pass must refuse instead of aliasing an occurrence from an earlier pass.
    owner
        .render(AudioBlockMut::new(&mut output, 512, ChannelLayout::Mono).unwrap())
        .unwrap();
    output.fill(9.0);
    let mut result = Ok(());
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            result =
                owner.render(AudioBlockMut::new(&mut output, 512, ChannelLayout::Mono).unwrap());
        }),
        0
    );
    assert!(matches!(result, Err(LoopFault::Identity(_))));
    assert_eq!(output, [0.0; 512]);
    assert_eq!(owner.source.control.minter_mut().retired(), 8);
    assert_eq!(owner.fault(), result.err());
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn clock_exhaustion_preflights_the_whole_outer_callback() {
    let mut owner = impulse(65, 0, 0, 512);
    owner
        .renderer
        .install_stopped_clock(crate::time::StreamAnchor::new(
            SampleTime::new(u64::MAX - 127),
            PlanPosition::ZERO,
        ));
    let before = owner.snapshot();
    let mut output = [9.0; 512];
    assert!(matches!(
        owner.render(AudioBlockMut::new(&mut output, 512, ChannelLayout::Mono).unwrap()),
        Err(LoopFault::Time(_))
    ));
    assert_eq!(output, [0.0; 512]);
    assert_eq!(owner.snapshot(), before);
}

#[test]
fn one_quantum_publication_capacity_uses_the_real_profile_for_every_host_maximum() {
    for maximum in [1, 37, 64, 512] {
        let owner = impulse(65, 0, 0, maximum);
        let profile = HostProfile::harness(
            SampleRate::new(48_000.0).unwrap(),
            FrameCount::new(maximum),
            ChannelLayout::Mono,
        )
        .unwrap();
        assert_eq!(
            owner.arbiter.capacity(),
            profile.limits().events().max_events_per_quantum().get() as usize
        );
    }
}

#[test]
fn mapped_renderer_span_refuses_before_carry_clock_or_identity_changes() {
    let mut owner = notes(65, 0, &[(3, 60, true)]);
    let positions = [PlanPosition::ZERO; 64];
    let mut output = [9.0; 64];
    assert!(matches!(
        owner.renderer.render_loop_quantum(
            AudioBlockMut::new(&mut output, 64, ChannelLayout::Mono).unwrap(),
            crate::render::TimedEvents::EMPTY,
            &positions
        ),
        Err(RenderError::MappedTimelineSpan { .. })
    ));
    assert_eq!(output, [9.0; 64]);
    assert_eq!(owner.renderer.carry_frames(), 64);
    assert_eq!(owner.snapshot().clock, SampleTime::ZERO);
    assert_eq!(owner.source.control.minter_mut().live(), 0);
}

#[test]
fn session_admission_checks_exact_capacity_one_over_and_initial_suffix_junction() {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .unwrap();
    let share = profile
        .limits()
        .events()
        .shares()
        .session_event_share()
        .get() as usize;
    for (entry, extra, accepts) in [(0, 0, true), (0, 1, false), (63, 0, false)] {
        let mut owner = notes(64, entry, &[(3, 60, true), (30, 60, false)]);
        let payload = owner.source.repeating.catch_up[0];
        // The real catch-up compiler emits one row per prepared parameter.
        // Vary that batch size at the admission seam to isolate its exact cost.
        owner.source.initial.catch_up = vec![payload; share - 1 + extra].into_boxed_slice();
        owner.source.repeating.catch_up = vec![payload; share - 1 + extra].into_boxed_slice();
        let result = super::prepare::admit_programs(
            &owner.source.initial,
            &owner.source.repeating,
            owner.source.interval,
            &profile,
        );
        assert_eq!(result.is_ok(), accepts, "entry={entry}, extra={extra}");
        if !accepts {
            assert!(matches!(
                result,
                Err(LoopPrepareError::Admission {
                    class: crate::publish::ProducerClass::Session,
                    ..
                })
            ));
        }
    }
}

#[test]
fn retained_storage_budget_and_stealing_policy_refuse_during_preparation() {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .unwrap();
    let interval = LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(1)).unwrap();
    let build = |stealing, budget| {
        let graph = GraphIr::builder()
            .declaring(crate::ir::PlanDeclarations {
                stealing,
                ..crate::ir::PlanDeclarations::default()
            })
            .build()
            .unwrap();
        let plan = compile(&graph, &RenderConfig::new(profile))
            .into_plan()
            .unwrap();
        let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
        CompiledLoopStream::prepare(
            plan,
            stream,
            profile,
            LoopSettings::new(
                interval,
                PlanPosition::ZERO,
                PreparedBytes::measured(budget),
            )
            .unwrap(),
        )
    };
    let required = match build(crate::ir::StealingPolicy::None, 0) {
        Err(LoopPrepareError::Storage { required, .. }) => required,
        _ => panic!("boundary storage must be charged"),
    };
    assert!(build(crate::ir::StealingPolicy::None, required.get()).is_ok());
    assert!(matches!(
        build(crate::ir::StealingPolicy::None, required.get() - 1),
        Err(LoopPrepareError::Storage { .. })
    ));
    assert!(matches!(
        build(
            crate::ir::StealingPolicy::Oldest {
                fade: FrameCount::new(128)
            },
            required.get()
        ),
        Err(LoopPrepareError::Stealing)
    ));
}

#[test]
fn output_shape_refusal_preserves_state_and_a_later_valid_call_can_proceed() {
    let mut owner = impulse(65, 0, 0, 512);
    let before = owner.snapshot();
    let mut output = [9.0; 128];
    assert!(matches!(
        owner.render(AudioBlockMut::new(&mut output, 64, ChannelLayout::Stereo).unwrap()),
        Err(LoopFault::Render(RenderError::OutputBufferShape { .. }))
    ));
    assert_eq!(owner.snapshot(), before);
    assert_eq!(owner.fault(), None);
    assert_eq!(output, [9.0; 128]);
    owner
        .render(AudioBlockMut::new(&mut output, 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(output[64], 1.0);
}

#[test]
fn render_guard_detects_heap_reclamation_even_without_an_allocation() {
    let retained = Box::new([0_u8; 128]);
    let events = crate::render_allocation::count_allocs(|| {
        drop(std::hint::black_box(retained));
    });
    assert_eq!(events, 1, "the guard must count the one deallocation");
}
