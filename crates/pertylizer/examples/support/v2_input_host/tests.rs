use super::*;
use synth_engine_v2::{
    compile::{RenderConfig, compile},
    host::{
        EndpointId,
        input::{
            InputCapacity, InputLimits, InputRate, InputTick, InputTickSpan, SimulatedInputClock,
            SimulatedNoteInput,
        },
        session::{
            LoopRecordingSession, SessionCommandCapacity, SessionLimits, SessionOutcome,
            SessionSourceCapacity, SessionSourceLimits,
        },
    },
    ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain},
    looping::{CompiledLoopStream, LoopSettings},
    profile::{CaptureLimits, CaptureLimitsInput, HostProfile, RecordingLimits},
    quantities::{
        CapturePassCount, CaptureResultCount, CaptureSourceCount, ChannelLayout, EventCount,
        HeldNoteCount, PreparedBytes, ProjectionTickCount, SampleRate, TrackedInputNoteCount,
    },
    recording::{
        CaptureOutcome,
        notes::{
            CaptureMode, CaptureQuantization, ControllerSnapshot, FixtureRevision, FixtureTargetId,
            Midi1Input, MusicalInterval,
            loop_capture::{LoopCaptureSession, LoopNoteArmInput},
        },
    },
    schedule::AdmittedCompiledStream,
    tempo::{Bpm, MusicalTick, TempoMap},
    time::{FrameCount, PlanPosition},
    transport::LoopInterval,
};

fn fixture() -> (LiveControl, LiveAudio, [ConnectionGeneration; 2]) {
    let rate = SampleRate::new(48000.0).unwrap();
    let profile = HostProfile::harness(rate, FrameCount::new(8192), ChannelLayout::Mono).unwrap();
    let graph = GraphIr::builder()
        .node(
            NodeId::new(1),
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
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
    let events = AdmittedCompiledStream::admit(&plan, &[]).unwrap();
    let stream = CompiledLoopStream::prepare(
        plan,
        events,
        profile,
        LoopSettings::new(
            LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(200)).unwrap(),
            PlanPosition::ZERO,
            PreparedBytes::measured(1_000_000),
        )
        .unwrap(),
    )
    .unwrap();
    let limits = RecordingLimits::new(
        HeldNoteCount::limit(4).unwrap(),
        EventCount::limit(64).unwrap(),
    )
    .unwrap()
    .with_capture(
        CaptureLimits::new(CaptureLimitsInput {
            max_tracked_input_notes: TrackedInputNoteCount::limit(8).unwrap(),
            max_capture_sources: CaptureSourceCount::limit(2).unwrap(),
            max_capture_passes: CapturePassCount::limit(32).unwrap(),
            max_pending_capture_results: CaptureResultCount::limit(1).unwrap(),
            max_capture_bytes: PreparedBytes::measured(1_048_576),
            max_audio_capture_frames: FrameCount::new(1),
            max_projection_ticks: ProjectionTickCount::limit(100).unwrap(),
            capture_lateness_allowance: FrameCount::ZERO,
        })
        .unwrap(),
    )
    .unwrap();
    let mut capture =
        LoopCaptureSession::prepare(stream, limits, PreparedBytes::measured(8192)).unwrap();
    let mut inputs = Vec::new();
    let mut sources = Vec::new();
    let mut generations = Vec::new();
    for port in 0..2 {
        let mut input = SimulatedNoteInput::new(
            EndpointId::new(format!("simulated-{port}")).unwrap(),
            InputLimits {
                cells: InputCapacity::new(32).unwrap(),
                bytes: PreparedBytes::measured(65536),
            },
        )
        .unwrap();
        let generation = input.begin().unwrap();
        input
            .prepare(
                generation,
                prepare::simulated_clock(capture.initial().epoch, port).unwrap(),
            )
            .unwrap();
        sources.push(
            input
                .bind_capture(generation, &mut capture, ControllerSnapshot::neutral())
                .unwrap(),
        );
        generations.push(generation);
        inputs.push(input);
    }
    let _ticket = capture
        .arm_at(
            LoopNoteArmInput {
                target: FixtureTargetId::new(1).unwrap(),
                expected_revision: FixtureRevision::new(1),
                interval: MusicalInterval::new(MusicalTick::ZERO, MusicalTick::new(8)).unwrap(),
                mode: CaptureMode::Overdub,
                quantization: CaptureQuantization::Off,
                tempo: TempoMap::new(Bpm::new(120.0).unwrap(), &[], rate).unwrap(),
            },
            &sources,
            SampleTime::new(256),
        )
        .unwrap();
    let session = LoopRecordingSession::prepare(
        capture,
        SessionLimits {
            commands: SessionCommandCapacity::new(4).unwrap(),
            command_bytes: PreparedBytes::measured(16384),
        },
        SessionSourceLimits {
            actions: SessionSourceCapacity::new(32).unwrap(),
            bytes: PreparedBytes::measured(32768),
        },
    )
    .unwrap();
    let composition = InputCaptureSession::prepare(
        session,
        inputs.into_boxed_slice(),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let (mut control, audio, halt) = composition.split(PreparedBytes::measured(65536)).unwrap();
    for &generation in &generations {
        control.start_input(generation).unwrap();
    }
    let (mut control, audio) = LiveControl::attach(control, audio, halt);
    // Initial frame-zero fences must reach the callback before its first render.
    control.pump().unwrap();
    (control, audio, [generations[0], generations[1]])
}

fn collect(control: &mut LiveControl, commands: &mut Vec<String>) {
    while control.has_completions() {
        if let Some((_id, outcome)) = control.collect().unwrap() {
            commands.push(format!("{outcome:?}"));
        }
    }
}
fn render(audio: &mut LiveAudio, total: usize, partitions: &[usize]) -> Vec<f32> {
    let mut samples = vec![9.0; total];
    let mut offset = 0;
    let mut part = 0;
    let measured = allocation_counter::measure(|| {
        while offset < total {
            let count = partitions[part % partitions.len()].min(total - offset);
            audio
                .render(
                    AudioBlockMut::new(
                        &mut samples[offset..offset + count],
                        count,
                        ChannelLayout::Mono,
                    )
                    .unwrap(),
                )
                .unwrap();
            offset += count;
            part += 1;
        }
    });
    assert_eq!((measured.count_total, measured.count_current), (0, 0));
    samples
}
fn frontier(control: &mut LiveControl, generations: [ConnectionGeneration; 2], at: u64) {
    for (port, generation) in generations.into_iter().enumerate() {
        let _id = control
            .offer(
                generation,
                InputObservation::Frontier {
                    tick: InputTick::new(at * (port as u64 + 1)),
                },
            )
            .unwrap();
    }
}
fn message(control: &mut LiveControl, generation: ConnectionGeneration, at: u64, bytes: [u8; 3]) {
    let _id = control
        .offer(
            generation,
            InputObservation::Message {
                tick: InputTick::new(at),
                arrival: SampleTime::new(at),
                input: Midi1Input::from_bytes(bytes).unwrap(),
            },
        )
        .unwrap();
}
fn reunited(control: LiveControl, audio: LiveAudio) -> InputCaptureSession {
    match control.reunite(audio, |_, _| {}) {
        Ok(owner) => owner,
        Err(ReuniteError::Pending(_)) => panic!("outstanding host custody"),
        Err(ReuniteError::Core(error, _, _)) => panic!("core reunion: {}", error.error()),
        Err(ReuniteError::Audition(_, _, _, error)) => panic!("audition reunion: {error}"),
    }
}

#[test]
fn raw_time_refusal_does_not_create_audition_custody() {
    for (tick, arrival, error) in [
        (279, 300, InputError::Order),
        (300, 299, InputError::Future),
    ] {
        let (mut control, mut audio, generations) = fixture();
        let profile = HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(8192),
            ChannelLayout::Mono,
        )
        .unwrap();
        let clocks = [
            prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap(),
            prepare::simulated_clock(audio.core.acknowledged().epoch, 1).unwrap(),
        ];
        let (audition, audition_audio, _bytes) =
            super::audition::AuditionControl::prepare(profile, generations, clocks).unwrap();
        control.audition = Some(audition);
        audio.audition = Some(audition_audio);
        frontier(&mut control, generations, 256);
        message(&mut control, generations[0], 280, [0x90, 60, 100]);
        let before = control.audition.as_ref().unwrap().custody();
        let observation = InputObservation::Message {
            tick: InputTick::new(tick),
            arrival: SampleTime::new(arrival),
            input: Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
        };
        assert_eq!(
            control.offer(generations[0], observation),
            Err(InputOfferError::Refused(observation, error))
        );
        assert_eq!(control.audition.as_ref().unwrap().custody(), before);
        assert!(control.halt_handle().is_requested());
    }
}

#[test]
fn full_audition_settlement_retains_raw_receipt_until_retry() {
    use synth_engine_v2::recording::notes::AuditionTrace;

    let (mut control, mut audio, generations) = fixture();
    let profile = HostProfile::harness(
        SampleRate::new(48000.0).unwrap(),
        FrameCount::new(8192),
        ChannelLayout::Mono,
    )
    .unwrap();
    let clocks = [
        prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap(),
        prepare::simulated_clock(audio.core.acknowledged().epoch, 1).unwrap(),
    ];
    let (audition, audition_audio, _bytes) =
        super::audition::AuditionControl::prepare(profile, generations, clocks).unwrap();
    control.audition = Some(audition);
    audio.audition = Some(audition_audio);
    let _play = control
        .command(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    frontier(&mut control, generations, 256);
    let observation = InputObservation::Message {
        tick: InputTick::new(560),
        arrival: SampleTime::new(280),
        input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
    };
    let id = control.offer(generations[1], observation).unwrap();
    frontier(&mut control, generations, 512);
    control.pump().unwrap();
    render(&mut audio, 768, &[768]);
    while control.has_completions() {
        let _ = control.collect().unwrap();
    }
    for _ in 0..2 {
        let prior = control.collect_input(generations[1]).unwrap().unwrap();
        assert!(matches!(prior.audition, AuditionTrace::NotOffered));
    }

    control
        .audition
        .as_mut()
        .unwrap()
        .fill_settlements(generations[1]);
    assert_eq!(
        control.collect_input(generations[1]).unwrap_err(),
        InputError::Full
    );
    assert_eq!(control.pending_input.as_ref().unwrap().id, id);
    assert!(control.collect_input(generations[0]).unwrap().is_none());
    assert_eq!(control.pending_input.as_ref().unwrap().id, id);

    control.recover(&mut audio, |_, _| {}).unwrap();
    assert!(audio.audition.as_ref().unwrap().is_finished());
    assert!(control.pending.is_none());
    assert!(control.failed_collection.is_none());
    assert!(audio.pending.is_none());
    assert!(audio.refused.is_none());
    assert!(audio.packets.is_empty());
    assert!(control.completions.is_empty());
    let Err(ReuniteError::Pending(owners)) = control.reunite(audio, |_, _| {}) else {
        panic!("reunion must retain the unsettled raw receipt");
    };
    let (mut control, mut audio) = *owners;
    assert!(
        audio
            .audition
            .as_mut()
            .unwrap()
            .free_settlement_slot()
            .is_some()
    );
    let receipt = control.collect_input(generations[1]).unwrap().unwrap();
    assert_eq!(receipt.id, id);
    assert_eq!(receipt.observation, observation);
    assert!(matches!(receipt.audition, AuditionTrace::Pending(_)));
    assert!(control.pending_input.is_none());
    // The fixture's source-zero frontiers were withheld while source one owned
    // the pending receipt; they must still be available after its retry.
    let other_source = control.collect_input(generations[0]).unwrap().unwrap();
    assert!(matches!(other_source.audition, AuditionTrace::NotOffered));
    let next = control.collect_input(generations[1]).unwrap().unwrap();
    assert!(next.id.serial() > id.serial());
    assert!(control.collect_input(generations[1]).unwrap().is_none());
}

#[test]
fn post_receipt_audition_fault_returns_accepted_id_without_retryable_input() {
    let (mut control, mut audio, generations) = fixture();
    let profile = HostProfile::harness(
        SampleRate::new(48000.0).unwrap(),
        FrameCount::new(8192),
        ChannelLayout::Mono,
    )
    .unwrap();
    let epoch = audio.core.acknowledged().epoch;
    let clocks = [
        SimulatedInputClock::new(
            epoch,
            SampleTime::ZERO,
            InputTick::new(999),
            InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
            InputTickSpan::new(0),
        ),
        prepare::simulated_clock(epoch, 1).unwrap(),
    ];
    let (audition, audition_audio, _bytes) =
        super::audition::AuditionControl::prepare(profile, generations, clocks).unwrap();
    control.audition = Some(audition);
    audio.audition = Some(audition_audio);
    frontier(&mut control, generations, 256);
    let observation = InputObservation::Message {
        tick: InputTick::new(280),
        arrival: SampleTime::new(280),
        input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
    };
    let InputOfferError::Accepted {
        id,
        error,
        settlement_error,
    } = control.offer(generations[0], observation).unwrap_err()
    else {
        panic!("core must retain the accepted input");
    };
    assert_eq!(id.generation(), generations[0]);
    assert_eq!(error, InputError::ClockRange);
    assert_eq!(settlement_error, None);
    assert_eq!(control.audition.as_ref().unwrap().custody(), ([0; 2], 0, 0));
    assert!(control.halt_handle().is_requested());
}

#[test]
fn joined_recovery_resolves_renderer_refusal_and_queued_suffix_without_callback() {
    use synth_engine_v2::host::live::{AuditionOutcome, LiveInputError};

    let (mut control, mut audio, generations) = fixture();
    let profile = HostProfile::harness(
        SampleRate::new(48000.0).unwrap(),
        FrameCount::new(8192),
        ChannelLayout::Mono,
    )
    .unwrap();
    let clocks = [
        prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap(),
        prepare::simulated_clock(audio.core.acknowledged().epoch, 1).unwrap(),
    ];
    let (audition, audition_audio, _bytes) =
        super::audition::AuditionControl::prepare(profile, generations, clocks).unwrap();
    control.audition = Some(audition);
    audio.audition = Some(audition_audio);
    frontier(&mut control, generations, 256);
    for (offset, key) in (60..66).enumerate() {
        message(
            &mut control,
            generations[0],
            280 + offset as u64,
            [0x90, key, 100],
        );
    }
    frontier(&mut control, generations, 512);
    control.pump().unwrap();
    audio
        .audition
        .as_mut()
        .unwrap()
        .hold_refused_after(4)
        .unwrap();
    let mut block = [0.0; 512];
    let result = audio.render(AudioBlockMut::new(&mut block, 512, ChannelLayout::Mono).unwrap());
    assert!(
        matches!(result, Err(HostError::Audition(LiveInputError::Closed))),
        "{result:?}"
    );
    assert!(control.halt_handle().is_requested());
    control.recover(&mut audio, |_, _| {}).unwrap();
    let audition = audio.audition.as_mut().unwrap();
    assert_eq!(audition.outcomes().count(), 6);
    audition.finish().unwrap();
    assert_eq!(audition.outcomes().count(), 6);
    for generation in generations {
        while control.collect_input(generation).unwrap().is_some() {}
    }
    let mut outcomes = Vec::new();
    let mut owner = match control.reunite(audio, |id, outcome| outcomes.push((id, outcome))) {
        Ok(owner) => owner,
        Err(_) => panic!("joined reunion retained an audition packet"),
    };
    outcomes.sort_by_key(|(id, _)| id.serial());
    assert_eq!(outcomes.len(), 6);
    for (index, (id, outcome)) in outcomes.into_iter().enumerate() {
        assert_eq!(id.source(), generations[0]);
        assert_eq!(id.serial(), index as u64 + 1);
        assert_eq!(outcome, AuditionOutcome::Cancelled);
    }
    for generation in generations {
        owner.acknowledge_input_quiescence(generation).unwrap();
    }
    owner.finalize().unwrap();
}

#[test]
fn joined_recovery_keeps_a_renderer_identity_fault_in_custody() {
    use synth_engine_v2::host::{
        live::LiveInputError, session::loop_transfer::LoopTransferOutcome,
    };

    let (mut control, mut audio, generations) = fixture();
    let profile = HostProfile::harness(
        SampleRate::new(48000.0).unwrap(),
        FrameCount::new(8192),
        ChannelLayout::Mono,
    )
    .unwrap();
    let clocks = [
        prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap(),
        prepare::simulated_clock(audio.core.acknowledged().epoch, 1).unwrap(),
    ];
    let (audition, audition_audio, _bytes) =
        super::audition::AuditionControl::prepare(profile, generations, clocks).unwrap();
    control.audition = Some(audition);
    audio.audition = Some(audition_audio);
    frontier(&mut control, generations, 256);
    message(&mut control, generations[0], 280, [0x90, 60, 100]);
    message(&mut control, generations[0], 281, [0x90, 62, 100]);
    let in_core = control
        .command(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    audio.admit().unwrap();
    let queued = control
        .command(SampleTime::new(512), SessionCommand::Stop)
        .unwrap();
    {
        let audition = audio.audition.as_mut().unwrap();
        audition.hold_refused_after(1).unwrap();
        audition.duplicate_refused_id(1).unwrap();
        assert!(matches!(audition.finish(), Err(LiveInputError::Identity)));
        assert!(!audition.is_finished());
    }
    let mut command_outcomes = Vec::new();
    assert!(matches!(
        control.recover(&mut audio, |id, outcome| command_outcomes
            .push((id, outcome))),
        Err(HostError::Audition(LiveInputError::Identity))
    ));
    assert_eq!(command_outcomes.len(), 2);
    assert!(
        command_outcomes
            .iter()
            .any(|(id, outcome)| *id == queued && matches!(outcome, HostOutcome::Cancelled))
    );
    assert!(command_outcomes.iter().any(|(id, outcome)| *id == in_core
        && matches!(outcome,
            HostOutcome::Delivered(LoopTransferOutcome::Command(receipt))
                if matches!(receipt.outcome, SessionOutcome::Cancelled))));
    assert!(matches!(
        control.recover(&mut audio, |id, outcome| command_outcomes
            .push((id, outcome))),
        Err(HostError::Audition(LiveInputError::Identity))
    ));
    assert_eq!(command_outcomes.len(), 2);
    let (mut foreign, _foreign_audio, _) = fixture();
    audio.refused = Some(
        foreign
            .core
            .prepare_command(SampleTime::new(256), SessionCommand::Play)
            .unwrap(),
    );
    let error = control
        .recover(&mut audio, |id, outcome| {
            command_outcomes.push((id, outcome))
        })
        .unwrap_err();
    let HostError::Recovery { earlier, later } = error else {
        panic!("both independent recovery faults must be reported");
    };
    assert!(matches!(
        *earlier,
        HostError::Audition(LiveInputError::Identity)
    ));
    assert!(matches!(*later, HostError::Transfer(_)));
    assert!(audio.refused.is_some());
    assert_eq!(command_outcomes.len(), 2);
    assert!(!audio.audition.as_ref().unwrap().is_finished());
    assert!(matches!(
        control.reunite(audio, |_, _| {}),
        Err(ReuniteError::Pending(_))
    ));
}

#[test]
fn recovery_keeps_later_command_when_older_refused_packet_cannot_cancel() {
    let (mut control, mut audio, _) = fixture();
    let (mut foreign, _foreign_audio, _) = fixture();
    audio.refused = Some(
        foreign
            .core
            .prepare_command(SampleTime::new(256), SessionCommand::Play)
            .unwrap(),
    );
    // Reproduce a packet retained after a full-ring push without filling the
    // ring with unrelated source observations.
    let later = control
        .core
        .prepare_command(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    let later_id = later.id();
    control.pending = Some(later);
    let queued_before = audio.packets.occupied_len();
    assert!(queued_before > 0);
    let mut outcomes = Vec::new();
    assert!(matches!(
        control.recover(&mut audio, |id, outcome| outcomes.push((id, outcome))),
        Err(HostError::Transfer(_))
    ));
    assert!(outcomes.is_empty());
    assert!(audio.refused.is_some());
    assert_eq!(audio.packets.occupied_len(), queued_before);
    assert!(
        control
            .pending
            .as_ref()
            .is_some_and(|packet| packet.id() == later_id)
    );
    assert!(matches!(
        control.reunite(audio, |_, _| {}),
        Err(ReuniteError::Pending(_))
    ));
}

#[test]
fn reusable_host_accepts_transport_after_rendering_and_retains_input_outcomes() {
    let mut reference = None;
    for partitions in [&[512][..], &[64], &[256], &[1, 37, 128, 3]] {
        let (mut control, mut audio, generations) = fixture();
        let mut pcm = render(&mut audio, 192, partitions);
        assert_eq!(audio.clock(), SampleTime::new(128));
        let _play = control
            .command(SampleTime::new(256), SessionCommand::Play)
            .unwrap();
        frontier(&mut control, generations, 256);
        message(&mut control, generations[0], 280, [0x90, 60, 100]);
        message(&mut control, generations[0], 360, [0x80, 60, 0]);
        frontier(&mut control, generations, 768);
        control.pump().unwrap();
        pcm.extend(render(&mut audio, 320, partitions));
        let _stop = control
            .command(SampleTime::new(768), SessionCommand::Stop)
            .unwrap();
        pcm.extend(render(&mut audio, 384, partitions));
        let mut outcomes = Vec::new();
        collect(&mut control, &mut outcomes);
        assert_eq!(outcomes.len(), 2);
        control
            .finish_after_join(&mut audio, |_, _| panic!("commands already collected"))
            .unwrap();
        let mut received = 0;
        for generation in generations {
            while let Some(receipt) = control.collect_input(generation).unwrap() {
                assert!(
                    matches!(
                        receipt.outcome,
                        synth_engine_v2::host::input::InputOutcome::Delivered(_)
                    ),
                    "{receipt:?}"
                );
                received += 1;
            }
        }
        // Two initial source-frontier receipts plus the six explicit observations.
        assert_eq!(received, 8);
        let mut owner = reunited(control, audio);
        owner.finalize().unwrap();
        assert_eq!(
            owner.result().unwrap().sealed_outcome(),
            CaptureOutcome::Complete
        );
        assert_eq!(owner.result().unwrap().records().count(), 2);
        if let Some(expected) = &reference {
            assert_eq!(&pcm, expected);
        } else {
            reference = Some(pcm);
        }
    }
}

#[test]
fn fresh_attempt_cannot_be_changed_by_old_handles_and_retention_is_bounded() {
    use super::archive::{ArchiveError, RetainedRuns};
    let mut retained = RetainedRuns::prepare(
        CaptureResultCount::limit(1).unwrap(),
        PreparedBytes::measured(4_000_000),
        PreparedBytes::measured(8_100_000),
    )
    .unwrap();
    assert!(retained.bytes().get() <= 8_100_000);
    let (mut old, mut audio, generations) = fixture();
    let old_epoch = audio.core.acknowledged().epoch;
    retained.reserve_for_test(old_epoch).unwrap();
    let old_halt = old.halt_handle();
    old.recover(&mut audio, |_, _| {}).unwrap();
    for generation in generations {
        while old.collect_input(generation).unwrap().is_some() {}
    }
    let mut old = reunited(old, audio);
    for generation in generations {
        old.acknowledge_input_quiescence(generation).unwrap();
    }
    old.finalize().unwrap();
    assert!(retained.retain(old).is_ok());
    let (mut fresh, mut audio, fresh_generations) = fixture();
    let fresh_epoch = audio.core.acknowledged().epoch;
    assert_ne!(old_epoch, fresh_epoch);
    assert!(matches!(
        retained.reserve_for_test(fresh_epoch),
        Err(ArchiveError::Full)
    ));
    let old = retained.take(old_epoch).unwrap();
    assert!(retained.take(old_epoch).is_none());
    retained.reserve_for_test(fresh_epoch).unwrap();
    assert!(matches!(
        retained.reserve_for_test(fresh_epoch),
        Err(ArchiveError::Duplicate)
    ));
    old_halt.request_device_lost();
    assert!(
        fresh
            .offer(
                generations[0],
                InputObservation::Frontier {
                    tick: InputTick::new(0)
                }
            )
            .is_err()
    );
    let _play = fresh
        .command(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    frontier(&mut fresh, fresh_generations, 256);
    fresh.pump().unwrap();
    let pcm = render(&mut audio, 512, &[37]);
    assert_eq!(pcm[320], 1.0);
    assert_eq!(
        old.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
}

#[test]
fn producer_queue_full_and_joined_shutdown_return_every_original_observation() {
    use super::source::{SourceInbox, SourceSendError};
    let (mut control, mut audio, generations) = fixture();
    assert!(SourceInbox::storage_bytes().get() < 65536);
    let clock = prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], control.halt_handle(), clock);
    let producer = std::thread::spawn(move || {
        let mut refused = None;
        for at in 1..=17 {
            let observation = InputObservation::Frontier {
                tick: InputTick::new(at),
            };
            if let Err(error) = producer.send(observation) {
                refused = Some(error);
            }
        }
        (producer, refused)
    });
    let (mut joined, refused) = producer.join().unwrap();
    assert_eq!(
        refused,
        Some(SourceSendError::Retry(InputObservation::Frontier {
            tick: InputTick::new(17)
        }))
    );
    control.halt_handle().request_device_lost();
    let halted = InputObservation::Frontier {
        tick: InputTick::new(18),
    };
    assert_eq!(joined.send(halted), Err(SourceSendError::Halted(halted)));
    let mut refusals = Vec::new();
    inbox.service_identified(&mut control, |id, outcome| {
        let InputOfferError::Refused(original, _) = outcome.unwrap_err() else {
            panic!("expected an unaccepted observation");
        };
        refusals.push((id.unwrap(), original))
    });
    assert_eq!(refusals.len(), 16);
    assert!(inbox.is_empty());
    for (index, (id, observation)) in refusals.into_iter().enumerate() {
        assert_eq!(id.generation(), generations[0]);
        assert_eq!(id.serial(), index as u64 + 1);
        assert_eq!(
            observation,
            InputObservation::Frontier {
                tick: InputTick::new(index as u64 + 1)
            }
        );
    }
    control.recover(&mut audio, |_, _| {}).unwrap();
}

#[test]
fn producer_retry_keeps_time_state_until_the_ring_accepts_the_original() {
    use super::source::{SourceInbox, SourceSendError};
    let (mut control, audio, generations) = fixture();
    let clock = prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], control.halt_handle(), clock);
    for at in 1..=16 {
        producer
            .send(InputObservation::Frontier {
                tick: InputTick::new(at),
            })
            .unwrap();
    }
    let next = InputObservation::Frontier {
        tick: InputTick::new(17),
    };
    assert_eq!(producer.send(next), Err(SourceSendError::Retry(next)));
    assert!(!control.halt_handle().is_requested());
    let mut accepted = 0;
    inbox.service(&mut control, |result| {
        accepted += 1;
        assert_eq!(result.unwrap().serial(), accepted + 1);
    });
    assert_eq!(accepted, 16);
    assert_eq!(producer.send(next), Ok(()));
    inbox.service(&mut control, |result| {
        accepted += 1;
        assert_eq!(result.unwrap().serial(), accepted + 1);
    });
    assert_eq!(accepted, 17);
}

#[test]
fn source_queue_ids_distinguish_equal_observations_and_skip_full_retries() {
    use super::source::{SourceInbox, SourceSendError};
    let (mut control, audio, generations) = fixture();
    let clock = prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], control.halt_handle(), clock);
    let onset = InputObservation::Message {
        tick: InputTick::new(10),
        arrival: SampleTime::new(10),
        input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
    };
    let first = producer.send_identified(onset).unwrap();
    let second = producer.send_identified(onset).unwrap();
    assert_eq!(first.generation(), generations[0]);
    assert_eq!(first.serial(), 1);
    assert_eq!(second.serial(), 2);
    assert_ne!(first, second);
    for at in 11..=24 {
        producer
            .send(InputObservation::Frontier {
                tick: InputTick::new(at),
            })
            .unwrap();
    }
    let pending = InputObservation::Frontier {
        tick: InputTick::new(25),
    };
    assert_eq!(
        producer.send_identified(pending),
        Err(SourceSendError::Retry(pending))
    );
    let mut delivered = Vec::new();
    inbox.service_identified(&mut control, |id, result| {
        delivered.push((id.unwrap(), result));
    });
    assert_eq!(delivered.len(), 16);
    assert_eq!(delivered[0].0, first);
    assert_eq!(delivered[1].0, second);
    for (index, (id, result)) in delivered.iter().enumerate() {
        assert_eq!(id.serial(), index as u64 + 1);
        assert!(result.is_ok());
    }
    let retried = producer.send_identified(pending).unwrap();
    assert_eq!(retried.serial(), 17);
    let mut retried_results = 0;
    inbox.service_identified(&mut control, |id, result| {
        assert_eq!(id, Some(retried));
        assert!(result.is_ok());
        retried_results += 1;
    });
    assert_eq!(retried_results, 1);
}

#[test]
fn source_queue_identity_exhaustion_is_a_terminal_pre_ring_fault() {
    use super::source::{SourceInbox, SourceSendError};
    let (mut control, audio, generations) = fixture();
    let clock = prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], control.halt_handle(), clock);
    producer.set_serial_for_test(u64::MAX);
    let original = InputObservation::Frontier {
        tick: InputTick::new(1),
    };
    assert_eq!(
        producer.send_identified(original),
        Err(SourceSendError::Invalid(
            original,
            InputError::IdentityExhausted
        ))
    );
    assert!(inbox.is_empty());
    let mut callbacks = 0;
    inbox.service_identified(&mut control, |_, _| callbacks += 1);
    assert_eq!(callbacks, 0);
    let fault = control
        .core
        .input(generations[0])
        .unwrap()
        .pre_ring_failure()
        .unwrap();
    assert_eq!(fault.reason, InputError::IdentityExhausted);
    assert_eq!(fault.observation, Some(original));
}

#[test]
fn producer_cannot_overtake_an_unresolved_retry_after_the_ring_drains() {
    use super::source::{SourceInbox, SourceSendError};
    let (mut control, audio, generations) = fixture();
    let clock = prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], control.halt_handle(), clock);
    for at in 1..=16 {
        producer
            .send(InputObservation::Frontier {
                tick: InputTick::new(at),
            })
            .unwrap();
    }
    let pending = InputObservation::Frontier {
        tick: InputTick::new(17),
    };
    assert_eq!(producer.send(pending), Err(SourceSendError::Retry(pending)));
    let mut delivered = 0;
    inbox.service(&mut control, |result| {
        let _id = result.unwrap();
        delivered += 1;
    });
    assert_eq!(delivered, 16);
    assert!(inbox.is_empty());

    let overtaking = InputObservation::Frontier {
        tick: InputTick::new(18),
    };
    assert_eq!(
        producer.send(overtaking),
        Err(SourceSendError::Invalid(overtaking, InputError::Order))
    );
    assert!(control.halt_handle().is_requested());
    inbox.service(&mut control, |_| panic!("no later packet entered the ring"));
    let fault = control
        .core
        .input(generations[0])
        .unwrap()
        .pre_ring_failure()
        .unwrap();
    assert_eq!(fault.reason, InputError::Order);
    assert_eq!(fault.observation, Some(overtaking));
    assert_eq!(
        producer.send(pending),
        Err(SourceSendError::Halted(pending))
    );
    assert!(inbox.close(producer).is_ok());
}

#[test]
fn producer_rejects_frontier_before_prior_arrival_before_ring_custody() {
    use super::source::{SourceInbox, SourceSendError};
    let (mut control, audio, generations) = fixture();
    let clock = prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], control.halt_handle(), clock);
    let onset = InputObservation::Message {
        tick: InputTick::new(10),
        arrival: SampleTime::new(12),
        input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
    };
    producer.send(onset).unwrap();
    let regressed_frontier = InputObservation::Frontier {
        tick: InputTick::new(11),
    };
    assert_eq!(
        producer.send(regressed_frontier),
        Err(SourceSendError::Invalid(
            regressed_frontier,
            InputError::Order
        ))
    );
    assert!(control.halt_handle().is_requested());
    let mut delivered = Vec::new();
    inbox.service(&mut control, |result| delivered.push(result));
    assert_eq!(delivered.len(), 1);
    assert!(matches!(
        delivered.pop(),
        Some(Err(InputOfferError::Refused(original, _))) if original == onset
    ));
    let fault = control
        .core
        .input(generations[0])
        .unwrap()
        .discontinuity()
        .unwrap();
    assert_eq!(fault.reason, InputError::Order);
    assert_eq!(fault.observation, Some(regressed_frontier));
}

#[test]
fn producer_rejects_later_serial_with_earlier_nominal_time() {
    use super::source::{SourceInbox, SourceSendError};
    let (mut control, audio, generations) = fixture();
    let clock = prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], control.halt_handle(), clock);
    let onset = InputObservation::Message {
        tick: InputTick::new(10),
        arrival: SampleTime::new(10),
        input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
    };
    producer.send(onset).unwrap();
    let release = InputObservation::Message {
        tick: InputTick::new(9),
        arrival: SampleTime::new(11),
        input: Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
    };
    assert_eq!(
        producer.send(release),
        Err(SourceSendError::Invalid(release, InputError::Order))
    );
    assert!(control.halt_handle().is_requested());
    assert!(!inbox.is_empty());
    let mut queued_outcomes = 0;
    inbox.service(&mut control, |result| {
        assert!(matches!(result, Err(InputOfferError::Refused(original, _)) if original == onset));
        queued_outcomes += 1;
    });
    assert_eq!(queued_outcomes, 1);
    let fault = control
        .core
        .input(generations[0])
        .unwrap()
        .discontinuity()
        .unwrap();
    assert_eq!(fault.reason, InputError::Order);
    assert_eq!(fault.observation, Some(release));
}

#[test]
fn producer_rejects_future_arrival_without_queue_custody() {
    use super::source::{SourceInbox, SourceSendError};
    let (mut control, audio, generations) = fixture();
    let clock = prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap();
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], control.halt_handle(), clock);
    let future = InputObservation::Message {
        tick: InputTick::new(10),
        arrival: SampleTime::new(9),
        input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
    };
    assert_eq!(
        producer.send(future),
        Err(SourceSendError::Invalid(future, InputError::Future))
    );
    assert!(control.halt_handle().is_requested());
    assert!(inbox.is_empty());
    let producer = inbox.close(producer).err().unwrap();
    inbox.service(&mut control, |_| {
        panic!("the refused observation was never queued")
    });
    let fault = control
        .core
        .input(generations[0])
        .unwrap()
        .discontinuity()
        .unwrap();
    assert_eq!(fault.reason, InputError::Future);
    assert_eq!(fault.observation, Some(future));
    assert!(inbox.close(producer).is_ok());
}

#[test]
fn producer_rejects_repeated_frontier_before_ring_custody() {
    use super::source::{SourceInbox, SourceSendError};
    let (control, audio, generations) = fixture();
    let clock = prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap();
    let (mut producer, inbox) = SourceInbox::prepare(generations[0], control.halt_handle(), clock);
    let frontier = InputObservation::Frontier {
        tick: InputTick::new(1),
    };
    producer.send(frontier).unwrap();
    assert_eq!(
        producer.send(frontier),
        Err(SourceSendError::Invalid(frontier, InputError::Order))
    );
    assert!(control.halt_handle().is_requested());
    assert!(!inbox.is_empty());
}

#[test]
fn producer_rejects_unmappable_clock_before_ring_custody() {
    use super::source::{SourceInbox, SourceSendError};
    let (mut control, audio, generations) = fixture();
    // This deliberately differs from raw input's clock to test failed recording.
    let clock = SimulatedInputClock::new(
        audio.core.acknowledged().epoch,
        SampleTime::ZERO,
        InputTick::new(0),
        InputRate::new(FrameCount::new(1), InputTickSpan::new(2)).unwrap(),
        InputTickSpan::new(1),
    );
    let (mut producer, mut inbox) =
        SourceInbox::prepare(generations[0], control.halt_handle(), clock);
    let ambiguous = InputObservation::Frontier {
        tick: InputTick::new(1),
    };
    assert_eq!(
        producer.send(ambiguous),
        Err(SourceSendError::Invalid(ambiguous, InputError::Uncertain))
    );
    assert!(control.halt_handle().is_requested());
    assert!(inbox.is_empty());
    let mut results = Vec::new();
    inbox.service_identified(&mut control, |id, result| {
        assert!(id.is_none());
        results.push(result);
    });
    inbox.service_identified(&mut control, |id, result| {
        assert!(id.is_none());
        results.push(result);
    });
    assert!(matches!(
        results.as_slice(),
        [Err(InputOfferError::Refused(original, InputError::State))] if *original == ambiguous
    ));
    assert!(inbox.close(producer).is_ok());
}

#[test]
fn managed_service_records_second_source_failure_before_first_queue_halt() {
    use super::{archive::RetainedRuns, managed::ManagedRun, prepare::PreparedAttempt};
    let profile = HostProfile::harness(
        SampleRate::new(48000.0).unwrap(),
        FrameCount::new(8192),
        ChannelLayout::Mono,
    )
    .unwrap();
    let prepared =
        PreparedAttempt::new(profile, SampleTime::new(256), IrNodeKind::Silence).unwrap();
    let bytes = prepared.bytes();
    let mut archive = RetainedRuns::prepare(
        CaptureResultCount::limit(1).unwrap(),
        bytes,
        PreparedBytes::measured(bytes.get() * 2 + 65536),
    )
    .unwrap();
    let (mut managed, _audio, [mut first, mut second]) =
        ManagedRun::start(&mut archive, prepared).unwrap();
    let queued = InputObservation::Frontier {
        tick: InputTick::new(1),
    };
    first.send(queued).unwrap();
    let invalid = InputObservation::Frontier {
        tick: InputTick::new(0),
    };
    assert_eq!(
        second.send(invalid),
        Err(super::source::SourceSendError::Invalid(
            invalid,
            InputError::Order
        ))
    );
    let mut results = Vec::new();
    managed
        .service(|result| results.push(result), |_, _| {}, |_| {})
        .unwrap();
    assert!(matches!(
        results.as_slice(),
        [Err(InputOfferError::Refused(original, _))] if *original == queued
    ));
    assert_eq!(
        managed.source_discontinuity(0).unwrap().reason,
        InputError::PeerInterrupted
    );
    let second_fault = managed.source_discontinuity(1).unwrap();
    assert_eq!(second_fault.reason, InputError::Order);
    assert_eq!(second_fault.observation, Some(invalid));
    assert!(managed.close_source(first).is_ok());
    assert!(managed.close_source(second).is_ok());
}

#[test]
fn managed_attempt_enforces_source_join_archive_capacity_and_measured_byte_admission() {
    use super::{archive::RetainedRuns, managed::ManagedRun, prepare::PreparedAttempt};
    let prepare = || {
        PreparedAttempt::new(
            HostProfile::harness(
                SampleRate::new(48000.0).unwrap(),
                FrameCount::new(8192),
                ChannelLayout::Mono,
            )
            .unwrap(),
            SampleTime::new(256),
            IrNodeKind::Impulse {
                position: PlanPosition::ZERO,
            },
        )
        .unwrap()
    };
    let mut prepared = None;
    let allocation = allocation_counter::measure(|| prepared = Some(prepare()));
    let prepared = prepared.unwrap();
    // Granted live heap, including the compiled renderer and all retained capture,
    // must fit the charge computed by the recipe, not a number asserted by its caller.
    assert!(u64::try_from(allocation.bytes_current).unwrap() <= prepared.bytes().get());
    let epoch = prepared.epoch();
    let per_attempt = prepared.bytes();
    let ceiling = PreparedBytes::measured(per_attempt.get() * 2 + 65536);
    let mut archive =
        RetainedRuns::prepare(CaptureResultCount::limit(1).unwrap(), per_attempt, ceiling).unwrap();
    let (mut managed, mut audio, producers) = ManagedRun::start(&mut archive, prepared).unwrap();
    // The first callback must work without merger service after start.
    let _silence = render(&mut audio, 192, &[64]);
    let _play = managed
        .command(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    let [mut first, mut second] = producers;
    for (producer, scale) in [(&mut first, 1), (&mut second, 2)] {
        producer
            .send(InputObservation::Frontier {
                tick: InputTick::new(256 * scale),
            })
            .unwrap();
    }
    first
        .send(InputObservation::Message {
            tick: InputTick::new(280),
            arrival: SampleTime::new(280),
            input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
        })
        .unwrap();
    first
        .send(InputObservation::Message {
            tick: InputTick::new(360),
            arrival: SampleTime::new(360),
            input: Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
        })
        .unwrap();
    for (producer, scale) in [(&mut first, 1), (&mut second, 2)] {
        producer
            .send(InputObservation::Frontier {
                tick: InputTick::new(768 * scale),
            })
            .unwrap();
    }
    first
        .send(InputObservation::Message {
            tick: InputTick::new(800),
            arrival: SampleTime::new(800),
            input: Midi1Input::from_bytes([0x90, 64, 100]).unwrap(),
        })
        .unwrap();
    // Returning a live endpoint cannot acknowledge a queue which has not been resolved.
    first = managed
        .close_source(first)
        .expect_err("queued source cannot close");
    managed
        .service(
            |result| {
                assert!(result.is_ok());
            },
            |_, _| {},
            |_| {},
        )
        .unwrap();
    let _pcm = render(&mut audio, 320, &[64]);
    let _stop = managed
        .command(SampleTime::new(768), SessionCommand::Stop)
        .unwrap();
    let _pcm = render(&mut audio, 384, &[37]);
    managed
        .service(
            |result| {
                assert!(result.is_ok());
            },
            |_, _| {},
            |_| {},
        )
        .unwrap();
    assert!(managed.close_source(first).is_ok());
    // Source 2 is still live, even though its ring is empty and audio has joined.
    let error = managed
        .finish(audio, false, &mut archive, |_, _| {}, |_| {}, |_, _| {})
        .unwrap_err();
    let super::managed::FinishFailure::Pending(owners, HostError::SourcesOpen) = *error else {
        panic!("missing source endpoint must retain owners");
    };
    let (mut managed, audio) = *owners;
    assert!(managed.close_source(second).is_ok());
    let mut final_receipts = Vec::new();
    managed
        .finish(
            audio,
            false,
            &mut archive,
            |_, _| {},
            |receipt| final_receipts.push(receipt),
            |_, _| {},
        )
        .unwrap();
    assert!(final_receipts.iter().any(|receipt| matches!(
        receipt.outcome,
        synth_engine_v2::host::input::InputOutcome::Cancelled
    )));
    assert!(ManagedRun::start(&mut archive, prepare()).is_err());
    let owner = archive.take(epoch).unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(owner.result().unwrap().records().count(), 2);
    let (mut next, audio, [first, second]) = ManagedRun::start(&mut archive, prepare()).unwrap();
    next.halt_handle().request_device_lost();
    next.service(
        |result| {
            assert!(result.is_err());
        },
        |_, _| {},
        |_| {},
    )
    .unwrap();
    assert!(next.close_source(first).is_ok());
    assert!(next.close_source(second).is_ok());
    next.finish(audio, true, &mut archive, |_, _| {}, |_| {}, |_, _| {})
        .unwrap();
    let tiny = PreparedAttempt::new(
        HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(8192),
            ChannelLayout::Mono,
        )
        .unwrap(),
        SampleTime::new(256),
        IrNodeKind::Silence,
    )
    .unwrap();
    let mut small = RetainedRuns::prepare(
        CaptureResultCount::limit(1).unwrap(),
        PreparedBytes::measured(1),
        PreparedBytes::measured(100000),
    )
    .unwrap();
    assert!(ManagedRun::start(&mut small, tiny).is_err());
    let candidate = prepare();
    let cancelled = candidate.epoch();
    let mut cancelled_archive =
        RetainedRuns::prepare(CaptureResultCount::limit(1).unwrap(), per_attempt, ceiling).unwrap();
    cancelled_archive.admit(&candidate).unwrap();
    cancelled_archive.cancel_preparation(cancelled).unwrap();
    assert!(cancelled_archive.admit(&candidate).is_ok());
}

#[test]
fn loss_without_a_final_callback_recovers_packets_and_take() {
    let (mut control, mut audio, generations) = fixture();
    let _play = control
        .command(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    frontier(&mut control, generations, 256);
    control.pump().unwrap();
    let _pcm = render(&mut audio, 384, &[64]);
    let _stop = control
        .command(SampleTime::new(1024), SessionCommand::Stop)
        .unwrap();
    control.halt_handle().request_device_lost();
    let mut outcomes = Vec::new();
    control
        .recover(&mut audio, |id, outcome| outcomes.push((id, outcome)))
        .unwrap();
    assert!(outcomes.iter().any(|(_, outcome)| matches!(outcome,
        HostOutcome::Delivered(LoopTransferOutcome::Command(receipt)) if matches!(receipt.outcome, SessionOutcome::Applied { .. }))));
    for generation in generations {
        while control.collect_input(generation).unwrap().is_some() {}
    }
    let mut owner = reunited(control, audio);
    for generation in generations {
        owner.acknowledge_input_quiescence(generation).unwrap();
    }
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
}

#[test]
fn queue_charge_covers_actual_preparation_and_pending_reunion_retains_owners() {
    let (control, audio, _) = fixture();
    assert!(LiveControl::storage_bytes().get() < 65536);
    let measured = allocation_counter::measure(|| {
        let _packets = Arc::new(HeapRb::<LoopTransferPacket>::new(CELLS));
        let _completions = Arc::new(HeapRb::<LoopTransferCompletion>::new(CELLS));
    });
    assert!(measured.bytes_total <= LiveControl::storage_bytes().get());
    let mut control = control;
    let _play = control
        .command(SampleTime::new(256), SessionCommand::Play)
        .unwrap();
    let Err(ReuniteError::Pending(owners)) = control.reunite(audio, |_, _| {}) else {
        panic!("queued command lost");
    };
    let (mut control, mut audio) = *owners;
    control.recover(&mut audio, |_, _| {}).unwrap();
    let _owner = reunited(control, audio);
}

#[test]
fn audible_capture_uses_real_voices_sustain_panic_and_resolved_raw_receipts() {
    use synth_engine_v2::{host::live::AuditionOutcome, recording::notes::AuditionTrace};
    let mut reference = None;
    for partitions in [&[8192][..], &[64], &[256], &[1, 37, 128, 3]] {
        let (mut control, mut audio, generations) = fixture();
        let profile = HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(8192),
            ChannelLayout::Mono,
        )
        .unwrap();
        let (live_control, live_audio, _bytes) = super::audition::AuditionControl::prepare(
            profile,
            generations,
            [
                prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap(),
                prepare::simulated_clock(audio.core.acknowledged().epoch, 1).unwrap(),
            ],
        )
        .unwrap();
        control.audition = Some(live_control);
        audio.audition = Some(live_audio);
        let mut pcm = render(&mut audio, 192, partitions);
        let _play = control
            .command(SampleTime::new(256), SessionCommand::Play)
            .unwrap();
        frontier(&mut control, generations, 256);
        for (at, bytes) in [
            (280, [0x90, 60, 100]),
            (300, [0xb0, 64, 127]),
            (360, [0x80, 60, 0]),
            (500, [0xb0, 64, 0]),
            (600, [0x90, 64, 100]),
        ] {
            message(&mut control, generations[0], at, bytes);
        }
        frontier(&mut control, generations, 768);
        control.pump().unwrap();
        pcm.extend(render(&mut audio, 320, partitions));
        let _panic = control
            .command(SampleTime::new(768), SessionCommand::Panic)
            .unwrap();
        pcm.extend(render(&mut audio, 512, partitions));
        assert!(
            pcm[430..500].iter().any(|sample| sample.abs() > 0.001),
            "pedal keeps a real voice sounding after key-up"
        );
        assert!(pcm[564..664].iter().all(|sample| *sample == 0.0));
        assert!(
            pcm[832..].iter().all(|sample| *sample == 0.0),
            "panic ends held notes after Q carry"
        );
        if let Some(expected) = &reference {
            assert_eq!(&pcm, expected);
        } else {
            reference = Some(pcm);
        }
        control.finish_after_join(&mut audio, |_, _| {}).unwrap();
        for generation in generations {
            while control.collect_input(generation).unwrap().is_some() {}
        }
        let mut heard = Vec::new();
        let mut owner = match control.reunite(audio, |id, outcome| heard.push((id, outcome))) {
            Ok(owner) => owner,
            Err(_) => panic!("joined reunion failed"),
        };
        owner.finalize().unwrap();
        owner.close_completed().unwrap();
        for generation in generations {
            owner.acknowledge_input_quiescence(generation).unwrap();
        }
        owner.finalize().unwrap();
        let result = owner.result().unwrap();
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Complete);
        assert_eq!(heard.len(), 5);
        assert!(
            heard
                .iter()
                .all(|(_, outcome)| matches!(outcome, AuditionOutcome::Executed { .. }))
        );
        assert!(result.records().all(|record| matches!(
            record.audition(),
            AuditionTrace::Resolved(AuditionOutcome::Executed { .. })
        )));
    }
}

#[test]
fn count_in_metronome_recording_and_panic_share_one_clock_without_callback_allocation() {
    use super::{archive::RetainedRuns, managed::ManagedRun, prepare::PreparedAttempt};
    let mut reference = None;
    for partitions in [&[8192][..], &[256], &[37, 128, 3]] {
        let profile = HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(8192),
            ChannelLayout::Mono,
        )
        .unwrap();
        let mut prepared = None;
        let measured = allocation_counter::measure(|| {
            prepared = Some(
                PreparedAttempt::counted(profile, MusicalTick::new(960))
                    .unwrap()
                    .with_audition()
                    .unwrap(),
            )
        });
        let prepared = prepared.unwrap();
        assert!(
            u64::try_from(measured.bytes_current).unwrap() <= prepared.bytes().get(),
            "all three renderers and their metadata must fit admission"
        );
        let start = prepared.start();
        assert_eq!(start, SampleTime::new(24000));
        let epoch = prepared.epoch();
        let charge = prepared.bytes();
        let mut archive = RetainedRuns::prepare(
            CaptureResultCount::limit(1).unwrap(),
            charge,
            PreparedBytes::measured(charge.get() * 2 + 65536),
        )
        .unwrap();
        let (mut run, mut audio, [mut first, mut second]) =
            ManagedRun::start(&mut archive, prepared).unwrap();
        let mut pcm = render(&mut audio, 192, partitions);
        assert!(
            pcm[65..].iter().any(|sample| *sample != 0.0),
            "count-in sounds while song transport is stopped"
        );
        let _play = run.command(start, SessionCommand::Play).unwrap();
        let end = SampleTime::new(24128);
        for (producer, scale) in [(&mut first, 1), (&mut second, 2)] {
            producer
                .send(InputObservation::Frontier {
                    tick: InputTick::new(start.as_u64() * scale),
                })
                .unwrap();
        }
        first
            .send(InputObservation::Message {
                tick: InputTick::new(24032),
                arrival: SampleTime::new(24032),
                input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
            })
            .unwrap();
        for (producer, scale) in [(&mut first, 1), (&mut second, 2)] {
            producer
                .send(InputObservation::Frontier {
                    tick: InputTick::new(end.as_u64() * scale),
                })
                .unwrap();
        }
        run.service(|value| assert!(value.is_ok()), |_, _| {}, |_| {})
            .unwrap();
        let _panic = run.command(end, SessionCommand::Panic).unwrap();
        pcm.extend(render(&mut audio, 24576 - 192, partitions));
        assert!(
            pcm[24192..].iter().all(|sample| *sample == 0.0),
            "panic cuts the sounding click and the held voice"
        );
        if let Some(expected) = &reference {
            assert_eq!(&pcm, expected);
        } else {
            reference = Some(pcm);
        }
        run.service(|value| assert!(value.is_ok()), |_, _| {}, |_| {})
            .unwrap();
        assert!(run.close_source(first).is_ok());
        assert!(run.close_source(second).is_ok());
        run.finish(audio, false, &mut archive, |_, _| {}, |_| {}, |_, _| {})
            .unwrap();
        let mut owner = archive.take(epoch).unwrap();
        let result = owner.result().unwrap();
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Complete);
        assert_eq!(
            result.records().count(),
            1,
            "count-in never creates recorded notes"
        );
        let projected = owner.project_notes().unwrap();
        assert_eq!(projected.notes().len(), 1);
    }
}

#[test]
fn source_cells_recycle_beyond_64_with_audition_and_recover_without_a_last_callback() {
    use super::{archive::RetainedRuns, managed::ManagedRun, prepare::PreparedAttempt};
    use synth_engine_v2::host::live::AuditionOutcome;
    let mut reference = None;
    for audible in [false, true] {
        let profile = HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(8192),
            ChannelLayout::Mono,
        )
        .unwrap();
        let mut prepared =
            PreparedAttempt::new(profile, SampleTime::new(256), IrNodeKind::Silence).unwrap();
        if audible {
            prepared = prepared.with_audition().unwrap();
        }
        let epoch = prepared.epoch();
        let bytes = prepared.bytes();
        let mut archive = RetainedRuns::prepare(
            CaptureResultCount::limit(1).unwrap(),
            bytes,
            PreparedBytes::measured(bytes.get() * 2 + 65536),
        )
        .unwrap();
        let (mut run, mut audio, [mut first, mut second]) =
            ManagedRun::start(&mut archive, prepared).unwrap();
        let _play = run
            .command(SampleTime::new(256), SessionCommand::Play)
            .unwrap();
        first
            .send(InputObservation::Frontier {
                tick: InputTick::new(256),
            })
            .unwrap();
        second
            .send(InputObservation::Frontier {
                tick: InputTick::new(512),
            })
            .unwrap();
        second
            .send(InputObservation::Frontier {
                tick: InputTick::new(16384),
            })
            .unwrap();
        first
            .send(InputObservation::Message {
                tick: InputTick::new(280),
                arrival: SampleTime::new(280),
                input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
            })
            .unwrap();
        run.service(|r| assert!(r.is_ok()), |_, _| {}, |_| {})
            .unwrap();
        let _pcm = render(&mut audio, 320, &[64]);
        for step in 0..61 {
            first
                .send(InputObservation::Frontier {
                    tick: InputTick::new(320 + step * 64),
                })
                .unwrap();
            run.service(|r| assert!(r.is_ok()), |_, _| {}, |_| {})
                .unwrap();
            let _pcm = render(&mut audio, 64, &[37]);
        }
        // Observation 64 is a release; uncollected audition outcomes cannot block it.
        first
            .send(InputObservation::Message {
                tick: InputTick::new(4200),
                arrival: SampleTime::new(4200),
                input: Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
            })
            .unwrap();
        run.service(|r| assert!(r.is_ok()), |_, _| {}, |_| {})
            .unwrap();
        let pcm = render(&mut audio, 192, &[64]);
        assert!(pcm[128..].iter().all(|sample| *sample == 0.0));
        let refused = InputObservation::Frontier {
            tick: InputTick::new(5000),
        };
        assert_eq!(first.send(refused), Ok(()), "no lifetime observation quota");
        run.service(|r| assert!(r.is_ok()), |_, _| {}, |_| {})
            .unwrap();
        assert!(!run.halt_handle().is_requested());
        run.halt_handle().request_device_lost();
        // No callback follows device loss. Endpoint joins and recovery suffice.
        assert!(run.close_source(first).is_ok());
        assert!(run.close_source(second).is_ok());
        let mut outcomes = Vec::new();
        run.finish(
            audio,
            true,
            &mut archive,
            |_, _| {},
            |_| {},
            |_, outcome| outcomes.push(outcome),
        )
        .unwrap();
        if audible {
            assert_eq!(outcomes.len(), 2);
            assert!(
                outcomes
                    .iter()
                    .all(|o| matches!(o, AuditionOutcome::Executed { .. }))
            );
        }
        let owner = archive.take(epoch).unwrap();
        let result = owner.result().unwrap();
        let retained = (
            result.effective_outcome(),
            result
                .records()
                .map(|r| (r.input(), r.stamp().nominal()))
                .collect::<Vec<_>>(),
        );
        assert_eq!(retained.0, CaptureOutcome::Interrupted);
        if let Some(expected) = &reference {
            assert_eq!(&retained, expected);
        } else {
            reference = Some(retained);
        }
    }
}

#[test]
fn failed_source_start_returns_all_prepared_sound_owners() {
    use super::{
        archive::RetainedRuns,
        managed::{ManagedRun, StartOwners},
        prepare::PreparedAttempt,
    };
    let profile = HostProfile::harness(
        SampleRate::new(48000.0).unwrap(),
        FrameCount::new(8192),
        ChannelLayout::Mono,
    )
    .unwrap();
    let mut prepared = PreparedAttempt::counted(profile, MusicalTick::new(960))
        .unwrap()
        .with_audition()
        .unwrap();
    // Inject a duplicated start request after normal preparation. The second start
    // fails after the first input changed state; every prepared owner must return.
    prepared.generations[1] = prepared.generations[0];
    let bytes = prepared.bytes();
    let mut archive = RetainedRuns::prepare(
        CaptureResultCount::limit(1).unwrap(),
        bytes,
        PreparedBytes::measured(bytes.get() * 2 + 65536),
    )
    .unwrap();
    let failure = match ManagedRun::start(&mut archive, prepared) {
        Err(error) => error,
        Ok(_) => panic!("duplicate start accepted"),
    };
    let StartOwners::Split(_core, sound) = failure.owners else {
        panic!("wrong failure stage")
    };
    assert!(sound.audition.is_some());
    assert!(sound.metronome.is_some());
}

#[test]
fn long_preroll_reuses_audition_credits_with_slow_collection_and_joined_recovery() {
    use super::{archive::RetainedRuns, managed::ManagedRun, prepare::PreparedAttempt};
    let profile = HostProfile::harness(
        SampleRate::new(48000.0).unwrap(),
        FrameCount::new(8192),
        ChannelLayout::Mono,
    )
    .unwrap();
    let prepared = PreparedAttempt::new(profile, SampleTime::new(65536), IrNodeKind::Silence)
        .unwrap()
        .with_audition()
        .unwrap();
    let epoch = prepared.epoch();
    let bytes = prepared.bytes();
    let mut archive = RetainedRuns::prepare(
        CaptureResultCount::limit(1).unwrap(),
        bytes,
        PreparedBytes::measured(bytes.get() * 2 + 65536),
    )
    .unwrap();
    let (mut run, mut audio, [mut first, mut second]) =
        ManagedRun::start(&mut archive, prepared).unwrap();
    let _pcm = render(&mut audio, 64, &[64]);
    let mut outcomes = Vec::new();
    for cycle in 0..512 {
        let at = audio.clock().as_u64();
        for (offset, bytes) in [(0, [0x90, 60, 100]), (17, [0x80, 60, 0])] {
            first
                .send(InputObservation::Message {
                    tick: InputTick::new(at + offset),
                    arrival: SampleTime::new(at + offset),
                    input: Midi1Input::from_bytes(bytes).unwrap(),
                })
                .unwrap();
        }
        first
            .send(InputObservation::Frontier {
                tick: InputTick::new(at + 64),
            })
            .unwrap();
        second
            .send(InputObservation::Frontier {
                tick: InputTick::new((at + 64) * 2),
            })
            .unwrap();
        run.service(|result| assert!(result.is_ok()), |_, _| {}, |_| {})
            .unwrap();
        let mut block = [0.0; 64];
        let measurement = allocation_counter::measure(|| {
            audio
                .render(AudioBlockMut::new(&mut block, 64, ChannelLayout::Mono).unwrap())
                .unwrap();
        });
        assert_eq!(measurement.count_total, 0);
        assert_eq!(measurement.count_current, 0);
        assert!(block[..17].iter().any(|v| *v != 0.0));
        assert!(block[17..].iter().all(|v| *v == 0.0));
        if cycle % 16 == 15 {
            while let Some(outcome) = run.collect_audition() {
                outcomes.push(outcome);
            }
        }
    }
    run.service(|result| assert!(result.is_ok()), |_, _| {}, |_| {})
        .unwrap();
    assert!(!run.halt_handle().is_requested());
    run.halt_handle().request_device_lost();
    assert!(run.close_source(first).is_ok());
    assert!(run.close_source(second).is_ok());
    run.finish(
        audio,
        true,
        &mut archive,
        |_, _| {},
        |_| {},
        |id, outcome| outcomes.push((id, outcome)),
    )
    .unwrap();
    assert_eq!(outcomes.len(), 1024);
    assert!(outcomes.iter().all(|(_, outcome)| matches!(
        outcome,
        synth_engine_v2::host::live::AuditionOutcome::Executed { .. }
    )));
    let owner = archive.take(epoch).unwrap();
    assert_eq!(
        owner.result().unwrap().records().count(),
        0,
        "pre-roll never invents a recorded onset"
    );
}

#[test]
fn audition_credit_exhaustion_retains_results_and_refused_release_without_a_final_callback() {
    use super::{archive::RetainedRuns, managed::ManagedRun, prepare::PreparedAttempt};
    let profile = HostProfile::harness(
        SampleRate::new(48000.0).unwrap(),
        FrameCount::new(8192),
        ChannelLayout::Mono,
    )
    .unwrap();
    let prepared = PreparedAttempt::new(profile, SampleTime::new(65536), IrNodeKind::Silence)
        .unwrap()
        .with_audition()
        .unwrap();
    let bytes = prepared.bytes();
    let mut archive = RetainedRuns::prepare(
        CaptureResultCount::limit(1).unwrap(),
        bytes,
        PreparedBytes::measured(bytes.get() * 2 + 65536),
    )
    .unwrap();
    let (mut run, mut audio, [mut first, mut second]) =
        ManagedRun::start(&mut archive, prepared).unwrap();
    let _pcm = render(&mut audio, 64, &[64]);
    for cycle in 0..64 {
        let at = audio.clock().as_u64();
        let tail = if cycle == 63 {
            [0xb0, 64, 127]
        } else {
            [0x80, 60, 0]
        };
        for (offset, bytes) in [(0, [0x90, 60, 100]), (17, tail)] {
            first
                .send(InputObservation::Message {
                    tick: InputTick::new(at + offset),
                    arrival: SampleTime::new(at + offset),
                    input: Midi1Input::from_bytes(bytes).unwrap(),
                })
                .unwrap();
        }
        first
            .send(InputObservation::Frontier {
                tick: InputTick::new(at + 64),
            })
            .unwrap();
        second
            .send(InputObservation::Frontier {
                tick: InputTick::new((at + 64) * 2),
            })
            .unwrap();
        run.service(|result| assert!(result.is_ok()), |_, _| {}, |_| {})
            .unwrap();
        let _pcm = render(&mut audio, 64, &[64]);
    }
    let at = audio.clock();
    let release = InputObservation::Message {
        tick: InputTick::new(at.as_u64()),
        arrival: at,
        input: Midi1Input::from_bytes([0x80, 60, 0]).unwrap(),
    };
    first.send(release).unwrap();
    let mut refused = None;
    run.service(
        |result| {
            if let Err(value) = result {
                refused = Some(value);
            }
        },
        |_, _| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(
        refused,
        Some(InputOfferError::Refused(release, InputError::Full))
    );
    assert!(run.halt_handle().is_requested());
    assert!(run.close_source(first).is_ok());
    assert!(run.close_source(second).is_ok());
    let mut outcomes = Vec::new();
    run.finish(
        audio,
        true,
        &mut archive,
        |_, _| {},
        |_| {},
        |id, outcome| outcomes.push((id, outcome)),
    )
    .unwrap();
    assert_eq!(outcomes.len(), 128);
    outcomes.sort_by_key(|(id, _)| id.serial());
    assert_eq!(outcomes.first().unwrap().0.serial(), 1);
    assert_eq!(outcomes.last().unwrap().0.serial(), 128);
}

#[test]
fn recording_across_swap_preserves_raw_repeated_keys_sustain_and_execution_epochs() {
    use synth_engine_v2::{host::live::AuditionOutcome, recording::notes::AuditionTrace};
    let mut reference = None;
    for partitions in [&[8192][..], &[64], &[256], &[1, 37, 128, 3]] {
        let (mut control, mut audio, generations) = fixture();
        let profile = HostProfile::harness(
            SampleRate::new(48000.0).unwrap(),
            FrameCount::new(8192),
            ChannelLayout::Mono,
        )
        .unwrap();
        let (live_control, live_audio, _bytes) = super::audition::AuditionControl::prepare(
            profile,
            generations,
            [
                prepare::simulated_clock(audio.core.acknowledged().epoch, 0).unwrap(),
                prepare::simulated_clock(audio.core.acknowledged().epoch, 1).unwrap(),
            ],
        )
        .unwrap();
        control.audition = Some(live_control);
        audio.audition = Some(live_audio);
        let mut pcm = render(&mut audio, 192, partitions);
        let _play = control
            .command(SampleTime::new(256), SessionCommand::Play)
            .unwrap();
        frontier(&mut control, generations, 256);
        for (at, bytes) in [
            (280, [0x90, 60, 100]),
            (300, [0xb0, 64, 127]),
            (360, [0x80, 60, 0]),
            (400, [0x90, 60, 100]),
            (600, [0x90, 60, 100]),
            (650, [0x80, 60, 0]),
            (700, [0x80, 60, 0]),
            (720, [0xb0, 64, 0]),
        ] {
            message(&mut control, generations[0], at, bytes);
        }
        frontier(&mut control, generations, 768);
        control.pump().unwrap();
        pcm.extend(render(&mut audio, 320, partitions));
        let _plan = control
            .audition
            .as_mut()
            .unwrap()
            .swaps
            .publish(&super::audition::live_graph().unwrap())
            .unwrap();
        pcm.extend(render(&mut audio, 128, partitions));
        let _panic = control
            .command(SampleTime::new(768), SessionCommand::Panic)
            .unwrap();
        pcm.extend(render(&mut audio, 384, partitions));
        assert!(pcm[832..].iter().all(|sample| *sample == 0.0));
        if let Some(expected) = &reference {
            assert_eq!(&pcm, expected);
        } else {
            reference = Some(pcm);
        }
        control.finish_after_join(&mut audio, |_, _| {}).unwrap();
        for generation in generations {
            while control.collect_input(generation).unwrap().is_some() {}
        }
        let mut heard = Vec::new();
        let mut owner = match control.reunite(audio, |id, outcome| heard.push((id, outcome))) {
            Ok(owner) => owner,
            Err(_) => panic!("joined reunion failed"),
        };
        owner.finalize().unwrap();
        owner.close_completed().unwrap();
        for generation in generations {
            owner.acknowledge_input_quiescence(generation).unwrap();
        }
        owner.finalize().unwrap();
        let result = owner.result().unwrap();
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Complete);
        assert_eq!(heard.len(), 8);
        let epochs: std::collections::BTreeSet<_> = heard
            .iter()
            .filter_map(|(_, outcome)| {
                if let AuditionOutcome::Executed { epoch, .. } = outcome {
                    Some(epoch.as_u32())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            epochs.len(),
            2,
            "recording spans the actual live reset epoch"
        );
        assert!(
            heard
                .iter()
                .any(|(_, outcome)| *outcome == AuditionOutcome::NotSounded),
            "first release consumes the pre-reset key tombstone"
        );
        assert_eq!(result.records().count(), 8);
        assert!(
            result
                .records()
                .all(|record| matches!(record.audition(), AuditionTrace::Resolved(_)))
        );
    }
}
