mod threads;
use super::*;
use crate::host::{
    ConnectionState, EndpointId,
    input::{
        InputCapacity, InputCaptureError, InputCaptureSession, InputDiscontinuity, InputError,
        InputLimits, InputObservation, InputOutcome, InputRate, InputTick, InputTickSpan,
        SimulatedInputClock, SimulatedNoteInput,
    },
};

fn clock(
    epoch: StreamEpoch,
    origin: u64,
    frames: u64,
    ticks: u64,
    uncertainty: u64,
) -> SimulatedInputClock {
    SimulatedInputClock::new(
        epoch,
        SampleTime::ZERO,
        InputTick::new(origin),
        InputRate::new(FrameCount::new(frames), InputTickSpan::new(ticks)).unwrap(),
        InputTickSpan::new(uncertainty),
    )
}
fn port(epoch: StreamEpoch, index: usize, cells: u32, budget: u64) -> SimulatedNoteInput {
    let mut input = SimulatedNoteInput::new(
        EndpointId::new(format!("input-{index}")).unwrap(),
        InputLimits {
            cells: InputCapacity::new(cells).unwrap(),
            bytes: PreparedBytes::measured(budget),
        },
    )
    .unwrap();
    let generation = input.begin().unwrap();
    let clock = if index == 0 {
        clock(epoch, 1000, 2, 1, 0)
    } else {
        clock(epoch, 9000, 1, 2, 0)
    };
    input.prepare(generation, clock).unwrap();
    input
}
fn prepared(
    cells: u32,
    source_cells: u32,
) -> (
    LoopRecordingSession,
    Box<[SimulatedNoteInput]>,
    [ConnectionGeneration; 2],
) {
    let mut capture = LoopCaptureSession::prepare(
        stream(0, 2048),
        limits(2, 64, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let epoch = capture.initial().epoch;
    let mut inputs: Box<[_]> = (0..2)
        .map(|index| port(epoch, index, cells, 65536))
        .collect();
    let generations = [
        inputs[0].generation().unwrap(),
        inputs[1].generation().unwrap(),
    ];
    let sources = [
        inputs[0]
            .bind_capture(generations[0], &mut capture, ControllerSnapshot::neutral())
            .unwrap(),
        inputs[1]
            .bind_capture(generations[1], &mut capture, ControllerSnapshot::neutral())
            .unwrap(),
    ];
    assert_ne!(generations[0], sources[0]);
    let _ticket = capture.arm(super::input(), &sources).unwrap();
    let session = LoopRecordingSession::prepare(
        capture,
        command_limits(4, 16384),
        source_limits(source_cells, 32768),
    )
    .unwrap();
    (session, inputs, generations)
}
fn linked(cells: u32, source_cells: u32) -> (InputCaptureSession, [ConnectionGeneration; 2]) {
    let (session, inputs, generations) = prepared(cells, source_cells);
    let mut linked =
        InputCaptureSession::prepare(session, inputs, PreparedBytes::measured(8192)).unwrap();
    for generation in generations {
        linked.start_input(generation).unwrap();
    }
    (linked, generations)
}
fn tick(port: usize, frame: u64) -> InputTick {
    InputTick::new(if port == 0 {
        1000 + frame / 2
    } else {
        9000 + frame * 2
    })
}
fn message(
    owner: &mut InputCaptureSession,
    generation: ConnectionGeneration,
    port: usize,
    nominal: u64,
    arrival: u64,
    bytes: [u8; 3],
) {
    let _id = owner
        .offer_message(
            generation,
            tick(port, nominal),
            SampleTime::new(arrival),
            Midi1Input::from_bytes(bytes).unwrap(),
        )
        .unwrap();
}
fn frontier(owner: &mut InputCaptureSession, generations: [ConnectionGeneration; 2], frame: u64) {
    for (port, generation) in generations.into_iter().enumerate() {
        let _id = owner
            .advance_frontier(generation, tick(port, frame))
            .unwrap();
    }
}
fn transport(owner: &mut InputCaptureSession, stop: u64) {
    let _play = owner.offer(SampleTime::ZERO, SessionCommand::Play).unwrap();
    let _stop = owner
        .offer(SampleTime::new(stop), SessionCommand::Stop)
        .unwrap();
}
fn render_input(owner: &mut InputCaptureSession, total: usize, partitions: &[usize]) -> Vec<f32> {
    let mut samples = vec![9.0; total];
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            let mut offset = 0;
            let mut index = 0;
            while offset < total {
                let count = partitions[index % partitions.len()].min(total - offset);
                owner
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
                index += 1;
            }
        }),
        0
    );
    samples
}

#[test]
fn rational_clock_oracle_checks_hand_authored_frames_and_uncertainty() {
    let epoch = stream(0, 2048).snapshot().epoch;
    let drift = clock(epoch, 73, 1001, 1000, 0);
    for (input, expected) in [(73, 0), (74, 1), (1072, 999), (1073, 1001), (2073, 2002)] {
        assert_eq!(
            drift.map(InputTick::new(input)),
            Ok(SampleTime::new(expected))
        );
    }
    let slow = clock(epoch, 100, 1, 10, 1);
    assert_eq!(slow.map(InputTick::new(105)), Ok(SampleTime::ZERO));
    assert_eq!(slow.map(InputTick::new(110)), Err(InputError::Uncertain));
    assert_eq!(slow.map(InputTick::new(100)), Err(InputError::ClockRange));
    assert_eq!(
        slow.map(InputTick::new(u64::MAX)),
        Err(InputError::ClockRange)
    );
    assert_eq!(
        clock(epoch, 0, u64::MAX, 1, 0).map(InputTick::new(2)),
        Err(InputError::ClockRange)
    );
    assert!(InputRate::new(FrameCount::ZERO, InputTickSpan::new(1)).is_err());
    assert!(InputRate::new(FrameCount::new(1), InputTickSpan::new(0)).is_err());
}

#[test]
fn exact_start_tick_matches_exhaustive_rational_clock_oracle() {
    let epoch = stream(0, 2048).snapshot().epoch;
    for (frames, ticks) in [(1, 1), (2, 1), (3, 2), (1, 10)] {
        for uncertainty in [0, 1, 3] {
            let clock = clock(epoch, 73, frames, ticks, uncertainty);
            for frame in 0..20 {
                let time = SampleTime::new(frame);
                let expected = (73..400)
                    .map(InputTick::new)
                    .find(|tick| clock.map(*tick) == Ok(time));
                assert_eq!(clock.exact_tick(time).ok(), expected, "{clock:?} {time:?}");
            }
        }
    }
    assert!(
        clock(epoch, 0, 1, 1, 1)
            .exact_tick(SampleTime::ZERO)
            .is_err()
    );
}

#[test]
fn independent_clocks_arrival_order_and_callback_partitions_match() {
    let mut reference = None;
    for partitions in [&[512][..], &[64], &[256], &[1, 37, 128, 3, 256], &[1]] {
        for reverse in [false, true] {
            let (mut owner, generations) = linked(16, 32);
            transport(&mut owner, 320);
            let source_order = if reverse { [1, 0] } else { [0, 1] };
            for port in source_order {
                // Nominals are arrival-inverted between sources. Merger order must
                // be (20,30), (10,40), not a sort on the nominal frame.
                if port == 0 {
                    message(&mut owner, generations[0], 0, 10, 40, [0x90, 60, 100]);
                    message(&mut owner, generations[0], 0, 50, 70, [0x80, 60, 0]);
                } else {
                    message(&mut owner, generations[1], 1, 20, 30, [0x90, 62, 90]);
                    message(&mut owner, generations[1], 1, 60, 70, [0x80, 62, 0]);
                }
                let _id = owner
                    .advance_frontier(generations[port], tick(port, 100))
                    .unwrap();
                owner.pump().unwrap();
            }
            // Interior fences permit later delayed input only above their promised
            // nominal floor, and are strictly beyond all earlier source arrivals.
            message(&mut owner, generations[0], 0, 120, 140, [0x90, 64, 80]);
            message(&mut owner, generations[0], 0, 160, 180, [0x80, 64, 0]);
            frontier(&mut owner, generations, 320);
            owner.pump().unwrap();
            let audio = render_input(&mut owner, 512, partitions);
            owner.finalize().unwrap();
            assert_eq!(
                owner.result().unwrap().sealed_outcome(),
                CaptureOutcome::Complete
            );
            let raw: Vec<_> = owner
                .result()
                .unwrap()
                .records()
                .map(|r| {
                    (
                        r.stamp().nominal().as_u64(),
                        r.stamp().published_at().as_u64(),
                        r.input(),
                    )
                })
                .collect();
            assert_eq!(
                raw.iter().map(|r| (r.0, r.1)).collect::<Vec<_>>(),
                [
                    (20, 30),
                    (10, 40),
                    (50, 70),
                    (60, 70),
                    (120, 140),
                    (160, 180)
                ]
            );
            let mut receipts = Vec::new();
            for generation in generations {
                while let Some(receipt) = owner.collect_input(generation).unwrap() {
                    assert!(matches!(
                        receipt.outcome,
                        InputOutcome::Delivered(
                            SessionSourceOutcome::Fenced | SessionSourceOutcome::Published(_)
                        )
                    ));
                    receipts.push((receipt.id.serial(), receipt.observation));
                }
            }
            let observed = (audio, raw, receipts);
            if let Some(reference) = &reference {
                assert_eq!(&observed, reference);
            } else {
                reference = Some(observed);
            }
        }
    }
}

#[test]
fn equal_frontier_message_waits_for_every_sources_explicit_prefix() {
    for first_callback in [64, 128] {
        let (mut owner, generations) = linked(8, 32);
        transport(&mut owner, 128);
        frontier(&mut owner, generations, 20);
        message(&mut owner, generations[0], 0, 20, 20, [0x90, 60, 100]);
        owner.pump().unwrap();
        let _first = render_input(&mut owner, first_callback, &[first_callback]);
        // The first 64 frames drain prepared initial silence. The next quantum
        // commits engine clock 64 and makes the still-withheld arrival 20 late.
        assert_eq!(
            owner.acknowledged().clock.as_u64(),
            (first_callback - 64) as u64
        );
        owner.finalize().unwrap_err();
        let mut observed = Vec::new();
        for generation in generations {
            while let Some(receipt) = owner.collect_input(generation).unwrap() {
                observed.push(receipt);
            }
        }
        assert_eq!(observed.len(), if first_callback == 64 { 0 } else { 4 });
        assert!(
            observed
                .iter()
                .all(|r| matches!(r.observation, InputObservation::Frontier { .. }))
        );
        frontier(&mut owner, generations, 128);
        owner.pump().unwrap();
        if first_callback == 64 {
            let _rest = render_input(&mut owner, 256, &[64]);
            owner.finalize().unwrap();
            assert_eq!(owner.result().unwrap().records().count(), 1);
        } else {
            for generation in generations {
                owner.acknowledge_input_quiescence(generation).unwrap();
            }
            owner.finalize().unwrap();
            assert_eq!(
                owner.result().unwrap().sealed_outcome(),
                CaptureOutcome::Interrupted
            );
            let refused = owner.collect_input(generations[0]).unwrap().unwrap();
            assert!(matches!(
                refused.outcome,
                InputOutcome::Refused(LoopSessionError::Session(SessionError::SourceOrder))
            ));
            assert_eq!(owner.result().unwrap().records().count(), 0);
        }
    }
}

#[test]
fn slow_source_holds_merge_but_stop_and_delayed_worker_finalization_survive() {
    let (mut owner, generations) = linked(8, 32);
    transport(&mut owner, 128);
    message(&mut owner, generations[0], 0, 10, 20, [0x90, 60, 100]);
    frontier(&mut owner, generations, 64);
    owner.pump().unwrap();
    let _frontier = owner
        .advance_frontier(generations[0], tick(0, 128))
        .unwrap();
    owner.pump().unwrap();
    let audio = render_input(&mut owner, 512, &[37]);
    assert!(audio[192..].iter().all(|sample| *sample == 0.0));
    assert!(owner.finalize().is_err());
    let _play = owner.collect().unwrap();
    assert!(matches!(
        owner.collect().unwrap().outcome,
        SessionOutcome::Applied { .. }
    ));
    let before = owner.acknowledged();
    let _frontier = owner
        .advance_frontier(generations[1], tick(1, 128))
        .unwrap();
    owner.drain_stopped_sources().unwrap();
    owner.finalize().unwrap();
    assert_eq!(owner.acknowledged(), before);
    assert_eq!(owner.result().unwrap().records().count(), 1);
    assert_eq!(owner.result().unwrap().window().end(), SampleTime::new(128));
}

#[test]
fn input_loss_without_final_callback_retains_take_and_reconnect_stays_ready() {
    let (mut owner, generations) = linked(8, 32);
    transport(&mut owner, 512);
    message(&mut owner, generations[0], 0, 10, 20, [0x90, 60, 100]);
    frontier(&mut owner, generations, 100);
    owner.pump().unwrap();
    let _audio = render_input(&mut owner, 192, &[192]);
    let before = owner.acknowledged();
    assert_eq!(
        crate::render_allocation::count_allocs(|| owner.device_lost(generations[0]).unwrap()),
        0
    );
    assert!(owner.finalize().is_err());
    owner.acknowledge_input_quiescence(generations[0]).unwrap();
    assert!(owner.finalize().is_err());
    owner.acknowledge_input_quiescence(generations[1]).unwrap();
    owner.finalize().unwrap();
    assert_eq!(owner.acknowledged(), before);
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
    assert_eq!(owner.result().unwrap().window().end(), SampleTime::new(100));
    assert_eq!(owner.result().unwrap().records().count(), 1);
    assert_eq!(
        owner
            .input(generations[0])
            .unwrap()
            .discontinuity()
            .unwrap()
            .reason,
        InputError::DeviceLost
    );
    for generation in generations {
        while owner.collect_input(generation).unwrap().is_some() {}
    }
    while owner.collect().is_some() {}
    let (old_take, mut inputs) = owner.into_parts().unwrap();
    for (port, input) in inputs.iter_mut().enumerate() {
        let old = generations[port];
        let mapping = input.clock().unwrap();
        input.retire(old).unwrap();
        let new = input.begin().unwrap();
        assert_ne!(old, new);
        assert_eq!(input.prepare(old, mapping), Err(InputError::Stale));
        assert_eq!(input.fail_preparation(old), Err(InputError::Stale));
        input.prepare(new, mapping).unwrap();
        assert_eq!(input.start(old), Err(InputError::Stale));
        assert_eq!(input.device_lost(old), Err(InputError::Stale));
        assert_eq!(
            input.offer_message(
                old,
                tick(port, 10),
                SampleTime::new(20),
                Midi1Input::from_bytes([0x90, 60, 1]).unwrap()
            ),
            Err(InputError::Stale)
        );
        assert_eq!(input.state(), ConnectionState::Ready);
        assert_eq!(
            input.offer_message(
                new,
                tick(port, 10),
                SampleTime::new(20),
                Midi1Input::from_bytes([0x90, 60, 1]).unwrap()
            ),
            Err(InputError::State)
        );
        assert_eq!(input.source(), None);
    }
    assert_eq!(old_take.result().unwrap().records().count(), 1);
}

#[test]
fn protected_input_storage_keeps_accepted_cells_first_refusal_and_silences_next_callback() {
    let (mut owner, generations) = linked(6, 32);
    transport(&mut owner, 512);
    owner.pump().unwrap();
    let _audio = render_input(&mut owner, 128, &[128]);
    // Do not collect initial receipts. They continue spending input credits.
    message(&mut owner, generations[0], 0, 70, 70, [0x90, 60, 1]);
    message(&mut owner, generations[0], 0, 80, 80, [0x80, 60, 0]);
    let refused = InputObservation::Message {
        tick: tick(0, 90),
        arrival: SampleTime::new(90),
        input: Midi1Input::from_bytes([0x90, 62, 1]).unwrap(),
    };
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            assert!(matches!(
                owner.offer_message(
                    generations[0],
                    tick(0, 90),
                    SampleTime::new(90),
                    Midi1Input::from_bytes([0x90, 62, 1]).unwrap()
                ),
                Err(InputCaptureError::Input(InputError::ProtectedCapacity))
            ));
        }),
        0
    );
    let before = owner.acknowledged();
    let mut output = [9.0; 64];
    assert!(
        owner
            .render(AudioBlockMut::new(&mut output, 64, ChannelLayout::Mono).unwrap())
            .is_err()
    );
    assert_eq!(output, [0.0; 64]);
    assert_eq!(owner.acknowledged(), before);
    assert_eq!(
        owner
            .input(generations[0])
            .unwrap()
            .discontinuity()
            .unwrap()
            .observation,
        Some(refused)
    );
    for generation in generations {
        owner.acknowledge_input_quiescence(generation).unwrap();
    }
    owner.finalize().unwrap();
    let receipts: Vec<_> =
        std::iter::from_fn(|| owner.collect_input(generations[0]).unwrap()).collect();
    assert_eq!(receipts.len(), 3);
    assert_eq!(
        receipts
            .iter()
            .filter(|r| matches!(r.outcome, InputOutcome::Cancelled))
            .count(),
        2
    );
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
}

#[test]
fn ordinary_full_raw_storage_keeps_the_original_capture_discontinuity() {
    let (mut owner, generations) = linked(2, 32);
    let generation = generations[0];
    let _id = owner.advance_frontier(generation, tick(0, 128)).unwrap();
    let refused = InputObservation::Frontier { tick: tick(0, 256) };
    assert!(matches!(
        owner.advance_frontier(generation, tick(0, 256)),
        Err(InputCaptureError::Input(InputError::Full))
    ));
    assert_eq!(
        owner.input(generation).unwrap().discontinuity().unwrap(),
        InputDiscontinuity {
            reason: InputError::Full,
            observation: Some(refused),
        }
    );
}

#[test]
fn source_queue_backpressure_preserves_input_and_retries_after_stop() {
    let (mut owner, generations) = linked(8, 2);
    transport(&mut owner, 128);
    frontier(&mut owner, generations, 128);
    owner.pump().unwrap();
    let _audio = render_input(&mut owner, 512, &[512]);
    assert!(owner.finalize().is_err());
    for _ in 0..3 {
        owner.drain_stopped_sources().unwrap();
    }
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Complete
    );
    assert_eq!(owner.result().unwrap().records().count(), 0);
    for generation in generations {
        assert!(owner.input(generation).unwrap().discontinuity().is_none());
    }
}

#[test]
fn late_delivery_has_identified_refusal_and_no_silent_retimestamping() {
    let (mut owner, generations) = linked(8, 32);
    transport(&mut owner, 512);
    owner.pump().unwrap();
    let _audio = render_input(&mut owner, 192, &[192]);
    message(&mut owner, generations[0], 0, 10, 20, [0x90, 60, 100]);
    frontier(&mut owner, generations, 100);
    owner.pump().unwrap();
    for generation in generations {
        owner.acknowledge_input_quiescence(generation).unwrap();
    }
    owner.finalize().unwrap();
    let receipts: Vec<_> =
        std::iter::from_fn(|| owner.collect_input(generations[0]).unwrap()).collect();
    assert!(receipts.iter().any(|r| matches!(
        r.outcome,
        InputOutcome::Refused(LoopSessionError::Session(SessionError::SourceOrder))
    )));
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
    assert_eq!(owner.result().unwrap().records().count(), 0);
}

#[test]
fn frontier_cannot_overtake_an_accepted_delayed_message() {
    let (mut owner, generations) = linked(8, 32);
    message(&mut owner, generations[0], 0, 10, 100, [0x90, 60, 100]);
    assert!(matches!(
        owner.advance_frontier(generations[0], tick(0, 50)),
        Err(InputCaptureError::Input(InputError::Order))
    ));
    assert_eq!(
        owner.input(generations[0]).unwrap().state(),
        ConnectionState::Quiescing
    );
    assert_eq!(
        owner
            .collect_input(generations[0])
            .unwrap()
            .unwrap()
            .id
            .serial(),
        1
    );
    let note = owner.collect_input(generations[0]).unwrap().unwrap();
    assert!(
        matches!(note.observation, InputObservation::Message { arrival, .. } if arrival == SampleTime::new(100))
    );
    assert!(matches!(note.outcome, InputOutcome::Cancelled));
}

#[test]
fn complete_result_closes_inputs_and_requires_outcomes_before_release() {
    let (mut owner, generations) = linked(8, 32);
    transport(&mut owner, 128);
    frontier(&mut owner, generations, 128);
    owner.pump().unwrap();
    let _audio = render_input(&mut owner, 512, &[256]);
    owner.finalize().unwrap();
    owner.close_completed().unwrap();
    for generation in generations {
        owner.acknowledge_input_quiescence(generation).unwrap();
    }
    assert!(matches!(
        owner.into_parts().err().unwrap().error(),
        InputCaptureError::Input(InputError::Retained)
    ));
}

#[test]
fn complete_result_with_held_note_releases_raw_claims_before_reconnect() {
    let (mut owner, generations) = linked(8, 32);
    transport(&mut owner, 128);
    message(&mut owner, generations[0], 0, 10, 10, [0x90, 60, 100]);
    frontier(&mut owner, generations, 128);
    owner.pump().unwrap();
    let _audio = render_input(&mut owner, 512, &[256]);
    owner.finalize().unwrap();
    owner.close_completed().unwrap();
    for generation in generations {
        owner.acknowledge_input_quiescence(generation).unwrap();
        while owner.collect_input(generation).unwrap().is_some() {}
    }
    while owner.collect().is_some() {}
    let (_take, mut inputs) = owner.into_parts().unwrap();
    for (input, generation) in inputs.iter_mut().zip(generations) {
        input.retire(generation).unwrap();
        assert_ne!(input.begin().unwrap(), generation);
    }
}

#[test]
fn exact_budgets_and_attachment_refusals_preserve_owners() {
    let (session, inputs, _) = prepared(8, 32);
    let bytes = inputs[0].bytes().get();
    let endpoint = EndpointId::new("measured".to_owned()).unwrap();
    assert!(
        SimulatedNoteInput::new(
            endpoint.clone(),
            InputLimits {
                cells: InputCapacity::new(8).unwrap(),
                bytes: PreparedBytes::measured(bytes)
            }
        )
        .is_ok()
    );
    assert!(matches!(
        SimulatedNoteInput::new(
            endpoint,
            InputLimits {
                cells: InputCapacity::new(8).unwrap(),
                bytes: PreparedBytes::measured(bytes - 1)
            }
        ),
        Err(InputError::ByteBudget { .. })
    ));
    let owner =
        InputCaptureSession::prepare(session, inputs, PreparedBytes::measured(8192)).unwrap();
    let bytes = owner.bytes();
    for short in [false, true] {
        let (session, inputs, _) = prepared(8, 32);
        assert_eq!(
            InputCaptureSession::prepare(
                session,
                inputs,
                PreparedBytes::measured(bytes.get() - u64::from(short))
            )
            .is_err(),
            short
        );
    }
    let (session, inputs, _) = prepared(8, 32);
    let mut subset = inputs.into_vec();
    let _omitted = subset.pop().unwrap();
    let error = InputCaptureSession::prepare(
        session,
        subset.into_boxed_slice(),
        PreparedBytes::measured(8192),
    )
    .err()
    .unwrap();
    let (session, returned, error) = error.into_parts();
    assert!(matches!(
        error,
        InputCaptureError::Input(InputError::Attachment)
    ));
    assert_eq!(returned.len(), 1);
    assert_eq!(session.acknowledged().clock, SampleTime::ZERO);
}

#[test]
fn stalled_merge_retains_rejected_publication_after_stop_and_requires_quiescence() {
    let (mut owner, generations) = linked(8, 32);
    transport(&mut owner, 128);
    owner.pump().unwrap();
    message(&mut owner, generations[0], 0, 10, 20, [0x90, 60, 100]);
    let _frontier = owner
        .advance_frontier(generations[0], tick(0, 128))
        .unwrap();
    owner.pump().unwrap();
    let _audio = render_input(&mut owner, 512, &[256]);
    let _frontier = owner
        .advance_frontier(generations[1], tick(1, 128))
        .unwrap();
    owner.drain_stopped_sources().unwrap();
    assert!(owner.finalize().is_err());
    for generation in generations {
        owner.acknowledge_input_quiescence(generation).unwrap();
    }
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
    let receipts: Vec<_> =
        std::iter::from_fn(|| owner.collect_input(generations[0]).unwrap()).collect();
    let retained = receipts
        .iter()
        .find(|receipt| matches!(receipt.observation, InputObservation::Message { .. }))
        .unwrap();
    assert!(matches!(
        retained.outcome,
        InputOutcome::Delivered(SessionSourceOutcome::Refused(
            NoteCaptureError::PastBoundary
        ))
    ));
}

#[test]
fn input_callback_first_use_mapping_and_receipt_collection_allocate_and_free_nothing() {
    let (mut owner, generations) = linked(8, 32);
    let note = Midi1Input::from_bytes([0x90, 60, 100]).unwrap();
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            let _id = owner
                .offer_message(generations[0], tick(0, 10), SampleTime::new(20), note)
                .unwrap();
            frontier(&mut owner, generations, 100);
        }),
        0
    );
    transport(&mut owner, 128);
    owner.pump().unwrap();
    let _audio = render_input(&mut owner, 192, &[192]);
    assert!(owner.finalize().is_err());
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            for generation in generations {
                while owner.collect_input(generation).unwrap().is_some() {}
            }
        }),
        0
    );
}

#[test]
fn late_input_after_sealing_keeps_quality_custody_until_source_retirement() {
    let (mut owner, generations) = linked(8, 32);
    transport(&mut owner, 128);
    frontier(&mut owner, generations, 128);
    owner.pump().unwrap();
    let _audio = render_input(&mut owner, 512, &[256]);
    owner.finalize().unwrap();
    let prior_quality = owner.result().unwrap().quality();
    let note = Midi1Input::from_bytes([0x90, 60, 100]).unwrap();
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            assert!(
                owner
                    .offer_message(generations[0], tick(0, 20), SampleTime::new(200), note)
                    .is_err()
            );
        }),
        0
    );
    let result = owner.result().unwrap();
    assert_eq!(
        result.quality().first_late().unwrap().time,
        SampleTime::new(20)
    );
    assert_eq!(
        result.quality().effective_outcome(result.sealed_outcome()),
        CaptureOutcome::Partial
    );
    let quality = result.quality();
    assert_eq!(quality.late_count().as_u64(), 1);
    for generation in generations {
        owner.acknowledge_input_quiescence(generation).unwrap();
        while owner.collect_input(generation).unwrap().is_some() {}
    }
    while owner.collect().is_some() {}
    assert!(matches!(
        owner.offer_message(generations[0], tick(0, 20), SampleTime::new(200), note),
        Err(InputCaptureError::Input(InputError::State))
    ));
    assert_eq!(owner.result().unwrap().quality(), quality);
    let (mut retained, _inputs) = owner.into_parts().unwrap();
    assert!(matches!(
        retained.discard(prior_quality),
        Err(LoopSessionError::Capture(LoopCaptureError::Capture(
            NoteCaptureError::Storage(CaptureError::QualityChanged)
        )))
    ));
    retained.discard(quality).unwrap();
}

#[test]
fn uncertain_input_after_sealing_cannot_leave_an_unqualified_complete_take() {
    for (tick, inside, refusal) in [
        (0, true, InputError::ClockRange),
        (200, true, InputError::Uncertain),
        (2000, false, InputError::Uncertain),
    ] {
        let mut capture = LoopCaptureSession::prepare(
            stream(0, 2048),
            limits(2, 64, 1_048_576),
            PreparedBytes::measured(8192),
        )
        .unwrap();
        let mut input = SimulatedNoteInput::new(
            EndpointId::new("uncertain".to_owned()).unwrap(),
            InputLimits {
                cells: InputCapacity::new(8).unwrap(),
                bytes: PreparedBytes::measured(65536),
            },
        )
        .unwrap();
        let generation = input.begin().unwrap();
        input
            .prepare(generation, clock(capture.initial().epoch, 0, 1, 10, 1))
            .unwrap();
        let source = input
            .bind_capture(generation, &mut capture, ControllerSnapshot::neutral())
            .unwrap();
        let _ticket = capture.arm(super::input(), &[source]).unwrap();
        let serial = LoopRecordingSession::prepare(
            capture,
            command_limits(4, 16384),
            source_limits(32, 32768),
        )
        .unwrap();
        let mut owner = InputCaptureSession::prepare(
            serial,
            vec![input].into_boxed_slice(),
            PreparedBytes::measured(8192),
        )
        .unwrap();
        owner.start_input(generation).unwrap();
        transport(&mut owner, 128);
        let _frontier = owner
            .advance_frontier(generation, InputTick::new(1285))
            .unwrap();
        owner.pump().unwrap();
        let _audio = render_input(&mut owner, 512, &[256]);
        owner.finalize().unwrap();
        let prior_quality = owner.result().unwrap().quality();
        let note = Midi1Input::from_bytes([0x90, 60, 100]).unwrap();
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                assert!(
                    matches!(owner.offer_message(generation,InputTick::new(tick),SampleTime::new(200),note),
                Err(InputCaptureError::Input(error)) if error == refusal)
                );
            }),
            0
        );
        let result = owner.result().unwrap();
        assert_eq!(
            result.quality().first_uncertain_source(),
            inside.then_some(source)
        );
        assert_eq!(result.quality().first_late(), None);
        assert_eq!(
            result.quality().effective_outcome(result.sealed_outcome()),
            if inside {
                CaptureOutcome::Partial
            } else {
                CaptureOutcome::Complete
            }
        );
        owner.acknowledge_input_quiescence(generation).unwrap();
        while owner.collect_input(generation).unwrap().is_some() {}
        while owner.collect().is_some() {}
        let (mut retained, _inputs) = owner.into_parts().unwrap();
        if inside {
            assert!(matches!(
                retained.discard(prior_quality),
                Err(LoopSessionError::Capture(LoopCaptureError::Capture(
                    NoteCaptureError::Storage(CaptureError::QualityChanged)
                )))
            ));
            let quality = retained.result().unwrap().quality();
            retained.discard(quality).unwrap();
        } else {
            retained.discard(prior_quality).unwrap();
        }
    }
}

#[test]
fn scheduled_composition_refuses_an_unreachable_start_but_plain_arm_keeps_its_admission() {
    for (start, frames, uncertainty, accepted) in
        [(64, 3, 0, false), (64, 1, 1, false), (0, 1, 1, true)]
    {
        let mut capture = LoopCaptureSession::prepare(
            stream(0, 2048),
            limits(2, 64, 1_048_576),
            PreparedBytes::measured(8192),
        )
        .unwrap();
        let mut input = SimulatedNoteInput::new(
            EndpointId::new("fence-check".to_string()).unwrap(),
            InputLimits {
                cells: InputCapacity::new(4).unwrap(),
                bytes: PreparedBytes::measured(65536),
            },
        )
        .unwrap();
        let generation = input.begin().unwrap();
        input
            .prepare(
                generation,
                clock(capture.initial().epoch, 0, frames, 1, uncertainty),
            )
            .unwrap();
        let source = input
            .bind_capture(generation, &mut capture, ControllerSnapshot::neutral())
            .unwrap();
        let _ticket = if start == 0 {
            capture.arm(super::input(), &[source])
        } else {
            capture.arm_at(super::input(), &[source], SampleTime::new(start))
        }
        .unwrap();
        let session = LoopRecordingSession::prepare(
            capture,
            command_limits(4, 16384),
            source_limits(32, 32768),
        )
        .unwrap();
        let linked = InputCaptureSession::prepare(
            session,
            vec![input].into_boxed_slice(),
            PreparedBytes::measured(8192),
        );
        assert_eq!(linked.is_ok(), accepted);
        if let Err(error) = linked {
            let (session, inputs, _) = error.into_parts();
            assert!(session.result().is_err(), "unsealed take stays owned");
            assert_eq!(inputs[0].generation(), Some(generation));
        }
    }
}
