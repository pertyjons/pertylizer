use super::*;
use crate::compile::{RenderConfig, compile};
use crate::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use crate::quantities::{Amplitude, ChannelLayout, SampleRate};
use crate::render::AudioBlockMut;
use crate::time::FrameCount;

pub(super) fn profile() -> HostProfile {
    HostProfile::harness(
        SampleRate::new(48_000.0).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .unwrap()
}
pub(super) fn graph() -> GraphIr {
    GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Constant {
                level: Amplitude::new(0.25).unwrap(),
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
        .unwrap()
}
pub(super) fn limits() -> SessionTransferLimits {
    SessionTransferLimits {
        session: SessionLimits {
            commands: SessionCommandCapacity::new(2).unwrap(),
            command_bytes: PreparedBytes::limit(16384).unwrap(),
        },
        control_bytes: PreparedBytes::limit(16_384).unwrap(),
    }
}
pub(super) fn setup() -> (SessionControl, SessionAudio) {
    let plan = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    SessionControl::prepare(plan, stream, profile(), limits()).unwrap()
}
pub(super) fn notes_plan() -> (CompiledPlan, AdmittedCompiledStream) {
    use crate::ir::{NoteProducerDeclaration, PlanDeclarations};
    use crate::quantities::{HeldNoteCount, KeyIdentity, NormalizedLevel, NoteVelocity, Seconds};
    use crate::schedule::{CompiledPayload, PlanEvent};
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).unwrap(),
            },
            ExecutionScope::Voice,
        )
        .node(
            NodeId::new(2),
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).unwrap(),
                decay: Seconds::new(0.0).unwrap(),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.0).unwrap(),
                velocity_sensitivity: NormalizedLevel::FULL,
            },
            ExecutionScope::Voice,
        )
        .node(NodeId::new(3), IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(NodeId::new(4), IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (NodeId::new(1), PortId::FIRST),
            (NodeId::new(3), PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (NodeId::new(2), PortId::FIRST),
            (NodeId::new(3), crate::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (NodeId::new(3), PortId::FIRST),
            (NodeId::new(4), PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(PlanDeclarations {
            note_producers: vec![NoteProducerDeclaration {
                compiled: true,
                simultaneous_notes: HeldNoteCount::measured(2),
                simultaneous_holds: EventCount::NONE,
            }],
            ..PlanDeclarations::default()
        })
        .build()
        .unwrap();
    let plan = compile(&graph, &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let slot = plan.resolve_note(NodeId::new(2)).unwrap();
    let key = KeyIdentity::new(60).unwrap();
    let events = [(0, true), (128, false), (192, true), (256, false)].map(|(position, on)| {
        PlanEvent::new(
            PlanPosition::new(position),
            if on {
                CompiledPayload::NoteOn {
                    slot,
                    key,
                    velocity: NoteVelocity::FULL,
                }
            } else {
                CompiledPayload::NoteOff { slot, key }
            },
        )
    });
    let stream = AdmittedCompiledStream::admit(&plan, &events).unwrap();
    (plan, stream)
}

pub(super) fn setup_notes() -> (SessionControl, SessionAudio) {
    let (plan, stream) = notes_plan();
    SessionControl::prepare(plan, stream, profile(), limits()).unwrap()
}

fn render(audio: &mut SessionAudio, samples: &mut [f32], partition: usize) {
    for chunk in samples.chunks_mut(partition) {
        let frames = chunk.len();
        audio
            .render(AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).unwrap())
            .unwrap();
    }
}

#[test]
fn delayed_collection_preserves_stop_and_resume_under_every_partition() {
    for partition in [1, 37, 64, 256, 512] {
        let (mut control, mut audio) = setup();
        let play = control.prepare_play(SampleTime::ZERO).unwrap();
        let stop = control.prepare_stop(SampleTime::new(64)).unwrap();
        assert!(matches!(
            control.prepare_play(SampleTime::new(128)),
            Err(SessionError::Full)
        ));
        let mut output = [9.0; 320];
        let mut completed = [None, None];
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                audio.enqueue(play).unwrap();
                audio.enqueue(stop).unwrap();
                render(&mut audio, &mut output, partition);
                completed[0] = audio.take_completed();
                completed[1] = audio.take_completed();
            }),
            0
        );
        assert_eq!(&output[..64], &[0.0; 64]);
        assert_eq!(&output[64..128], &[0.25; 64]);
        assert!(output[128..].iter().all(|sample| *sample == 0.0));
        assert_eq!(audio.state(), PlaybackState::Stopped(PlanPosition::new(64)));
        // Moving receipts out of audio has not returned a controller credit.
        assert!(control.prepare_stop(SampleTime::new(256)).is_err());
        for packet in completed.into_iter().flatten() {
            let _receipt = control.collect(packet).unwrap();
        }
        assert!(!control.has_outstanding());
        let resume = control.prepare_play(SampleTime::new(256)).unwrap();
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                audio.enqueue(resume).unwrap();
                render(&mut audio, &mut output[..64], partition);
            }),
            0
        );
        assert_eq!(&output[..64], &[0.25; 64]);
        let _receipt = control.collect(audio.take_completed().unwrap()).unwrap();
    }
}

#[test]
fn same_time_cancellation_leaves_activation_unoffered() {
    let (mut control, mut audio) = setup();
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    audio
        .enqueue(control.prepare_stop(SampleTime::ZERO).unwrap())
        .unwrap();
    let mut output = [1.0; 192];
    render(&mut audio, &mut output, 37);
    assert_eq!(output, [0.0; 192]);
    let cancelled = audio.take_completed().unwrap();
    assert_eq!(cancelled.outcome(), Some(SessionOutcome::Cancelled));
    assert!(
        cancelled
            .entry
            .activation
            .as_ref()
            .unwrap()
            .effective()
            .is_none()
    );
    let _receipt = control.collect(cancelled).unwrap();
    let _receipt = control.collect(audio.take_completed().unwrap()).unwrap();
}

#[test]
fn a_late_command_is_retained_without_turning_other_audio_into_a_fault() {
    let (mut control, mut audio) = setup();
    let play = control.prepare_play(SampleTime::ZERO).unwrap();
    let late_stop = control.prepare_stop(SampleTime::new(64)).unwrap();
    audio.enqueue(play).unwrap();
    let mut output = [1.0; 256];
    render(&mut audio, &mut output, 256);
    assert_eq!(audio.clock(), SampleTime::new(192));
    audio.enqueue(late_stop).unwrap();
    assert_eq!(
        crate::render_allocation::count_allocs(|| render(&mut audio, &mut output, 37)),
        0
    );
    assert_eq!(output, [0.25; 256]);
    assert!(audio.fault.is_none());
    let _receipt = control.collect(audio.take_completed().unwrap()).unwrap();
    let receipt = control.collect(audio.take_completed().unwrap()).unwrap();
    assert_eq!(
        receipt.outcome,
        SessionOutcome::DeliveryRefused(SessionDeliveryError::Late {
            observed: SampleTime::new(192)
        })
    );
    let stop = control.prepare_stop(audio.clock()).unwrap();
    audio.enqueue(stop).unwrap();
    render(&mut audio, &mut output, 256);
    assert_eq!(output, [0.0; 256]);
    let _receipt = control.collect(audio.take_completed().unwrap()).unwrap();
}

#[test]
fn a_late_play_returns_its_unoffered_activation_before_another_play_is_prepared() {
    let (mut control, mut audio) = setup_notes();
    let late_play = control.prepare_play(SampleTime::ZERO).unwrap();
    render(&mut audio, &mut [0.0; 256], 37);
    let observed = audio.clock();
    let mut completed = None;
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            audio.enqueue(late_play).unwrap();
            render(&mut audio, &mut [0.0; 128], 37);
            completed = audio.take_completed();
        }),
        0
    );
    let completed = completed.unwrap();
    assert_eq!(
        completed.outcome(),
        Some(SessionOutcome::DeliveryRefused(
            SessionDeliveryError::Late { observed }
        ))
    );
    assert!(
        completed
            .entry
            .activation
            .as_ref()
            .unwrap()
            .effective()
            .is_none()
    );
    assert!(control.has_outstanding());
    let _receipt = control.collect(completed).unwrap();
    assert!(!control.has_outstanding());
    let play = control.prepare_play(audio.clock()).unwrap();
    audio.enqueue(play).unwrap();
    render(&mut audio, &mut [0.0; 128], 37);
    let receipt = control.collect(audio.take_completed().unwrap()).unwrap();
    assert!(matches!(receipt.outcome, SessionOutcome::Applied { .. }));
}

#[test]
fn stale_stopped_position_refuses_before_offer_and_preserves_the_packet() {
    let (mut control, mut audio) = setup();
    let play = control.prepare_play(SampleTime::ZERO).unwrap();
    // A boundary-side change after the acknowledged control snapshot.
    audio.runtime.state = PlaybackState::Stopped(PlanPosition::new(99));
    audio.enqueue(play).unwrap();
    render(&mut audio, &mut [0.0; 128], 128);
    let packet = audio.take_completed().unwrap();
    assert_eq!(
        packet.outcome(),
        Some(SessionOutcome::DeliveryRefused(
            SessionDeliveryError::PositionChanged {
                actual: PlanPosition::new(99)
            }
        ))
    );
    assert!(
        packet
            .entry
            .activation
            .as_ref()
            .unwrap()
            .effective()
            .is_none()
    );
    let _receipt = control.collect(packet).unwrap();
    assert_eq!(
        control.snapshot().playback,
        PlaybackState::Stopped(PlanPosition::new(99))
    );
}

#[test]
fn loss_without_a_final_callback_cancels_transit_and_runtime_ownership() {
    let (mut control, mut audio) = setup();
    let play = control.prepare_play(SampleTime::ZERO).unwrap();
    let unpublished_stop = control.prepare_stop(SampleTime::new(64)).unwrap();
    audio.enqueue(play).unwrap();
    control.close_admission();
    audio.close_after_quiescence().unwrap();
    audio.close_after_quiescence().unwrap();
    let receipt = control.collect(audio.take_completed().unwrap()).unwrap();
    assert_eq!(receipt.outcome, SessionOutcome::Cancelled);
    assert_eq!(
        control.cancel(unpublished_stop).unwrap().outcome,
        SessionOutcome::Cancelled
    );
    assert!(!control.has_outstanding());
    assert!(matches!(
        control.prepare_stop(SampleTime::new(64)),
        Err(SessionError::Closed)
    ));
}

#[test]
fn foreign_packet_returns_to_its_owner_and_out_of_order_collection_does_not_rewind() {
    let (mut control, mut audio) = setup();
    let (mut foreign, _) = setup();
    let packet = foreign.prepare_play(SampleTime::ZERO).unwrap();
    let (packet, error) = audio.enqueue(packet).unwrap_err();
    assert_eq!(error, SessionTransferError::Origin);
    let _receipt = foreign.cancel(packet).unwrap();
    audio
        .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
        .unwrap();
    audio
        .enqueue(control.prepare_stop(SampleTime::new(128)).unwrap())
        .unwrap();
    render(&mut audio, &mut [0.0; 128], 128);
    let earlier = audio.take_completed().unwrap();
    render(&mut audio, &mut [0.0; 128], 128);
    let later = audio.take_completed().unwrap();
    let _receipt = control.collect(later).unwrap();
    let latest = control.snapshot();
    let _receipt = control.collect(earlier).unwrap();
    assert_eq!(control.snapshot(), latest);
    assert_eq!(
        latest.playback,
        PlaybackState::Stopped(PlanPosition::new(128))
    );
}

#[test]
fn actual_threads_prepare_and_render_with_disjoint_mutable_owners() {
    let (mut control, mut audio) = setup_notes();
    let (send, receive) = std::sync::mpsc::sync_channel(2);
    let mut output = [1.0; 1024];
    let mut completed = [None, None];
    std::thread::scope(|scope| {
        let audio = &mut audio;
        let output = &mut output;
        let completed = &mut completed;
        let worker = scope.spawn(move || {
            // Control prepares while this distinct thread owns renderer/registry.
            assert_eq!(
                crate::render_allocation::count_allocs(|| render(audio, &mut output[..512], 37)),
                0
            );
            // This test's synchronization is outside the guarded callback. The host
            // uses bounded SPSC rings, not these blocking test rendezvous calls.
            for _ in 0..2 {
                let packet = receive.recv().unwrap();
                assert_eq!(
                    crate::render_allocation::count_allocs(|| audio.enqueue(packet).unwrap()),
                    0
                );
            }
            assert_eq!(
                crate::render_allocation::count_allocs(|| {
                    render(audio, &mut output[512..], 37);
                    completed[0] = audio.take_completed();
                    completed[1] = audio.take_completed();
                }),
                0
            );
        });
        send.send(control.prepare_play(SampleTime::new(512)).unwrap())
            .unwrap();
        send.send(control.prepare_stop(SampleTime::new(640)).unwrap())
            .unwrap();
        worker.join().unwrap();
    });
    assert_eq!(&output[..576], &[0.0; 576]);
    assert_eq!(&output[576..704], &[1.0; 128]);
    assert!(output[704..].iter().all(|sample| *sample == 0.0));
    for packet in completed.into_iter().flatten() {
        let _receipt = control.collect(packet).unwrap();
    }
    assert!(!control.has_outstanding());
}

#[test]
fn control_byte_budget_profile_mismatch_and_identity_exhaustion_refuse() {
    let plan = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let mut small = limits();
    small.control_bytes = PreparedBytes::limit(1).unwrap();
    assert!(matches!(
        SessionControl::prepare(plan, stream, profile(), small),
        Err(HostError::Session(SessionError::ByteBudget { .. }))
    ));
    let plan = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let other = HostProfile::harness(
        SampleRate::new(44_100.0).unwrap(),
        FrameCount::new(512),
        ChannelLayout::Mono,
    )
    .unwrap();
    assert!(matches!(
        SessionControl::prepare(plan, stream, other, limits()),
        Err(HostError::Session(SessionError::TransferProfile))
    ));
    let (mut control, _) = setup();
    control.serial = u64::MAX;
    assert!(matches!(
        control.prepare_stop(SampleTime::ZERO),
        Err(SessionError::IdentityExhausted)
    ));
    assert!(!control.has_outstanding());
}

#[test]
fn note_cut_and_resume_match_the_serial_owner_bit_for_bit() {
    for partition in [1, 37, 64, 256, 512] {
        let (mut control, mut audio) = setup_notes();
        let (plan, stream) = notes_plan();
        let (mut serial_control, mut serial_renderer) = StreamControl::open(
            plan,
            StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
        )
        .unwrap();
        let mut serial = SessionRuntime::prepare(
            ConnectionGeneration(1),
            &mut serial_control,
            stream,
            &profile(),
            limits().session,
        )
        .unwrap();
        audio
            .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
            .unwrap();
        audio
            .enqueue(control.prepare_stop(SampleTime::new(64)).unwrap())
            .unwrap();
        let _play = serial
            .offer_play(
                &mut serial_control,
                SampleTime::ZERO,
                SampleTime::ZERO,
                None,
            )
            .unwrap();
        let serial_id = serial
            .validate_offer(SampleTime::ZERO, SampleTime::new(64), SessionCommand::Stop)
            .unwrap();
        let _stop = serial.insert(
            serial_id,
            SampleTime::new(64),
            SessionCommand::Stop,
            None,
            None,
        );
        let mut actual = [0.0; 640];
        let mut expected = [0.0; 640];
        for pass in 0..2 {
            if pass == 1 {
                audio
                    .enqueue(control.prepare_play(SampleTime::new(256)).unwrap())
                    .unwrap();
                let _resume = serial
                    .offer_play(
                        &mut serial_control,
                        serial_renderer.clock(),
                        SampleTime::new(256),
                        None,
                    )
                    .unwrap();
            }
            let range = pass * 320..(pass + 1) * 320;
            assert_eq!(
                crate::render_allocation::count_allocs(|| render(
                    &mut audio,
                    &mut actual[range.clone()],
                    partition
                )),
                0
            );
            for chunk in expected[range].chunks_mut(partition) {
                let frames = chunk.len();
                serial
                    .render(
                        &mut serial_renderer,
                        AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).unwrap(),
                        None,
                    )
                    .unwrap();
            }
            while let Some(packet) = audio.take_completed() {
                let receipt = control.collect(packet).unwrap();
                let serial_receipt = serial.collect(&mut serial_control).unwrap().unwrap();
                assert_eq!(receipt.outcome, serial_receipt.outcome);
            }
            assert!(serial.collect(&mut serial_control).unwrap().is_none());
        }
        assert_eq!(actual, expected);
        assert_eq!(&actual[64..128], &[1.0; 64]);
        assert_eq!(&actual[320..448], &[0.0; 128]);
        assert_eq!(&actual[448..512], &[1.0; 64]);
        assert_eq!(&actual[512..], &[0.0; 128]);
    }
}

#[test]
fn malformed_callback_is_terminal_silent_and_keeps_unadopted_commands_collectable() {
    for layout in [ChannelLayout::Mono, ChannelLayout::Stereo] {
        let (mut control, mut audio) = setup();
        audio
            .enqueue(control.prepare_play(SampleTime::ZERO).unwrap())
            .unwrap();
        let frames = if layout == ChannelLayout::Mono {
            513
        } else {
            128
        };
        let mut output = vec![9.0; frames * layout.channels()];
        assert!(
            audio
                .render(AudioBlockMut::new(&mut output, frames, layout).unwrap())
                .is_err()
        );
        assert!(output.iter().all(|sample| *sample == 0.0));
        assert_eq!(audio.clock(), SampleTime::ZERO);
        let packet = control.prepare_stop(SampleTime::ZERO).unwrap();
        let (packet, error) = audio.enqueue(packet).unwrap_err();
        assert_eq!(error, SessionTransferError::Closed);
        let _receipt = control.cancel(packet).unwrap();
        control.close_admission();
        audio.close_after_quiescence().unwrap();
        let receipt = control.collect(audio.take_completed().unwrap()).unwrap();
        assert_eq!(receipt.outcome, SessionOutcome::Cancelled);
        assert!(!control.has_outstanding());
    }
}

#[test]
fn collection_and_next_preparation_can_overlap_later_audio_callbacks() {
    let (mut control, mut audio) = setup_notes();
    let (send, receive) = std::sync::mpsc::sync_channel(2);
    let (return_send, return_receive) = std::sync::mpsc::sync_channel(2);
    let mut output = [0.0; 2176];
    std::thread::scope(|scope| {
        let audio = &mut audio;
        let output = &mut output;
        let worker = scope.spawn(move || {
            assert_eq!(
                crate::render_allocation::count_allocs(|| render(audio, &mut output[..512], 37)),
                0
            );
            for _ in 0..2 {
                let command = receive.recv().unwrap();
                assert_eq!(
                    crate::render_allocation::count_allocs(|| audio.enqueue(command).unwrap()),
                    0
                );
            }
            for start in [512, 1024] {
                let mut completed = None;
                assert_eq!(
                    crate::render_allocation::count_allocs(|| {
                        render(audio, &mut output[start..start + 512], 37);
                        completed = audio.take_completed();
                    }),
                    0
                );
                // Ownership transfer in the test rendezvous is outside the core guard.
                return_send.send(completed.unwrap()).unwrap();
            }
            assert_eq!(
                crate::render_allocation::count_allocs(|| render(
                    audio,
                    &mut output[1536..1664],
                    37
                )),
                0
            );
            let resume = receive.recv().unwrap();
            let mut completed = None;
            assert_eq!(
                crate::render_allocation::count_allocs(|| {
                    audio.enqueue(resume).unwrap();
                    render(audio, &mut output[1664..], 37);
                    completed = audio.take_completed();
                }),
                0
            );
            return_send.send(completed.unwrap()).unwrap();
        });
        send.send(control.prepare_play(SampleTime::new(512)).unwrap())
            .unwrap();
        send.send(control.prepare_stop(SampleTime::new(1280)).unwrap())
            .unwrap();
        let play = control.collect(return_receive.recv().unwrap()).unwrap();
        assert!(matches!(play.outcome, SessionOutcome::Applied { .. }));
        let stop = control.collect(return_receive.recv().unwrap()).unwrap();
        assert_eq!(
            stop.outcome,
            SessionOutcome::Applied {
                position: PlanPosition::new(768)
            }
        );
        send.send(control.prepare_play(SampleTime::new(1664)).unwrap())
            .unwrap();
        let resume = control.collect(return_receive.recv().unwrap()).unwrap();
        assert_eq!(
            resume.outcome,
            SessionOutcome::Applied {
                position: PlanPosition::new(768)
            }
        );
        worker.join().unwrap();
    });
    assert!(!control.has_outstanding());
    assert!(audio.fault.is_none());
    assert_eq!(audio.clock(), SampleTime::new(2112));
}

#[test]
fn table_identity_rejects_a_packet_even_when_generation_epoch_and_plan_match() {
    let plan = compile(&graph(), &RenderConfig::new(profile()))
        .into_plan()
        .unwrap();
    let first_stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let second_stream = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let (first, mut audio) =
        SessionControl::prepare(plan.clone(), first_stream, profile(), limits()).unwrap();
    let (mut second, _other_audio) =
        SessionControl::prepare(plan, second_stream, profile(), limits()).unwrap();
    // Private seam models a repeated plan in the same connection and device epoch.
    // Its activation still belongs to second.control, which must reclaim it.
    second.origin.generation = first.origin.generation;
    second.origin.epoch = first.origin.epoch;
    assert_eq!(first.origin.plan, second.origin.plan);
    assert_ne!(first.origin.table, second.origin.table);
    let packet = second.prepare_play(SampleTime::ZERO).unwrap();
    let mut rejected = None;
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            rejected = Some(audio.enqueue(packet).unwrap_err());
        }),
        0
    );
    let (packet, error) = rejected.unwrap();
    assert_eq!(error, SessionTransferError::Origin);
    assert!(!audio.has_retained_commands());
    assert_eq!(audio.clock(), SampleTime::ZERO);
    let receipt = second.cancel(packet).unwrap();
    assert_eq!(receipt.outcome, SessionOutcome::Cancelled);
    assert!(!second.has_outstanding());
}
