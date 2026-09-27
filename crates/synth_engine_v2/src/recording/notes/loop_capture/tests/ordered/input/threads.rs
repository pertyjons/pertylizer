mod continuous;
use super::*;
use crate::host::{
    input::{InputCaptureAudio, InputCaptureControl, InputCaptureHalt},
    session::loop_transfer::{
        LoopTransferCompletion, LoopTransferError, LoopTransferOutcome, LoopTransferPacket,
    },
};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, Ordering},
};

const BUDGET: PreparedBytes = PreparedBytes::measured(65_536);

fn split(
    cells: u32,
    sources: u32,
) -> (
    InputCaptureControl,
    InputCaptureAudio,
    InputCaptureHalt,
    [ConnectionGeneration; 2],
) {
    let (session, inputs, generations) = prepared(cells, sources);
    let owner = InputCaptureSession::prepare(session, inputs, BUDGET).unwrap();
    let (mut control, audio, halt) = owner.split(BUDGET).unwrap();
    for generation in generations {
        control.start_input(generation).unwrap();
    }
    (control, audio, halt, generations)
}
fn observation(port: usize, nominal: u64, arrival: u64, bytes: [u8; 3]) -> InputObservation {
    InputObservation::Message {
        tick: tick(port, nominal),
        arrival: SampleTime::new(arrival),
        input: Midi1Input::from_bytes(bytes).unwrap(),
    }
}
fn observations(port: usize) -> [InputObservation; 3] {
    [
        observation(
            port,
            if port == 0 { 10 } else { 20 },
            if port == 0 { 40 } else { 30 },
            [0x90, 60, 100],
        ),
        observation(port, if port == 0 { 50 } else { 60 }, 70, [0x80, 60, 0]),
        InputObservation::Frontier {
            tick: tick(port, 320),
        },
    ]
}
fn offer(control: &mut InputCaptureControl, generations: [ConnectionGeneration; 2]) {
    for (port, generation) in generations.into_iter().enumerate() {
        for observation in observations(port) {
            let _id = control.offer_observation(generation, observation).unwrap();
        }
    }
}

struct Callback {
    audio: InputCaptureAudio,
    input: HeapCons<LoopTransferPacket>,
    output: HeapProd<LoopTransferCompletion>,
    refused: Option<LoopTransferPacket>,
    failed: Option<LoopTransferCompletion>,
    returned: usize,
}
impl Callback {
    fn admit(&mut self) {
        let prefix = self.input.occupied_len();
        for _ in 0..prefix {
            if self.refused.is_some() {
                break;
            }
            let packet = self.input.try_pop().unwrap();
            if let Err((packet, _)) = self.audio.enqueue(packet) {
                self.refused = Some(packet);
            }
        }
    }
    fn flush(&mut self) {
        if let Some(completion) = self.failed.take() {
            match self.output.try_push(completion) {
                Ok(()) => self.returned += 1,
                Err(completion) => {
                    self.failed = Some(completion);
                    return;
                }
            }
        }
        for _ in 0..36 {
            if self.output.is_full() {
                break;
            }
            let Some(completion) = self.audio.take_completed() else {
                break;
            };
            match self.output.try_push(completion) {
                Ok(()) => self.returned += 1,
                Err(completion) => {
                    self.failed = Some(completion);
                    break;
                }
            }
        }
    }
    fn render(&mut self, samples: &mut [f32]) -> Result<(), LoopSessionError> {
        self.admit();
        let count = samples.len();
        let result = self
            .audio
            .render(AudioBlockMut::new(samples, count, ChannelLayout::Mono).unwrap());
        self.flush();
        result
    }
}
fn queues(
    audio: InputCaptureAudio,
    slots: usize,
) -> (
    HeapProd<LoopTransferPacket>,
    HeapCons<LoopTransferCompletion>,
    Callback,
) {
    // Charges ring cells, both ring headers and padded Arc allocations, callback
    // wrapper including failed-send cells, and two producer/merger input queues.
    let bytes = slots * size_of::<LoopTransferPacket>()
        + size_of::<LoopTransferCompletion>()
        + size_of::<HeapRb<LoopTransferPacket>>()
        + size_of::<HeapRb<LoopTransferCompletion>>()
        + 4 * 256
        + size_of::<Callback>()
        + 2 * (size_of::<InputObservation>() + size_of::<HeapRb<InputObservation>>());
    assert!(bytes <= 65_536);
    let (writer, input) = HeapRb::new(slots).split();
    let (output, reader) = HeapRb::new(1).split();
    (
        writer,
        reader,
        Callback {
            audio,
            input,
            output,
            refused: None,
            failed: None,
            returned: 0,
        },
    )
}
fn receipt_text(receipt: crate::host::input::InputReceipt) -> String {
    format!(
        "{} {:?} {:?}",
        receipt.id.serial(),
        receipt.observation,
        match receipt.outcome {
            InputOutcome::Delivered(SessionSourceOutcome::Published(p)) => format!(
                "published {:?} {:?}",
                p.capture,
                p.occurrence.map(|id| id.serial())
            ),
            other => format!("{other:?}"),
        }
    )
}
fn receipts(
    owner: &mut InputCaptureSession,
    generations: [ConnectionGeneration; 2],
) -> Vec<String> {
    let mut receipts = Vec::new();
    for generation in generations {
        while let Some(receipt) = owner.collect_input(generation).unwrap() {
            receipts.push(receipt_text(receipt));
        }
    }
    receipts
}
fn summary(owner: &InputCaptureSession, generations: [ConnectionGeneration; 2]) -> String {
    let sources = generations.map(|g| owner.input(g).unwrap().source().unwrap());
    let result = owner.result().unwrap();
    let raw: Vec<_> = result
        .records()
        .map(|r| {
            (
                sources.iter().position(|s| *s == r.source()).unwrap(),
                r.sequence().as_u64(),
                r.stamp().nominal(),
                r.stamp().published_at(),
                r.input(),
                r.occurrence().map(|id| id.serial()),
            )
        })
        .collect();
    let passes: Vec<_> = result
        .loop_passes()
        .map(|p| {
            (
                p.id().as_u64(),
                p.rendered().as_u64(),
                p.window().start(),
                p.window().end(),
                p.position(),
            )
        })
        .collect();
    let carry: Vec<_> = result
        .loop_carry()
        .map(|c| {
            (
                sources
                    .iter()
                    .position(|s| *s == c.occurrence().source())
                    .unwrap(),
                c.occurrence().serial(),
                c.pass().as_u64(),
                c.peer().as_u64(),
                c.at(),
                c.direction(),
            )
        })
        .collect();
    format!(
        "{:?} {:?} {:?} {raw:?} {passes:?} {carry:?}",
        result.window().start(),
        result.window().end(),
        result.sealed_outcome()
    )
}

#[test]
fn producers_merger_and_audio_match_serial_with_full_queues_and_stalled_receipt_worker() {
    let (mut serial, generations) = linked(16, 32);
    transport(&mut serial, 320);
    for (port, generation) in generations.into_iter().enumerate() {
        for observation in observations(port) {
            match observation {
                InputObservation::Message {
                    tick,
                    arrival,
                    input,
                } => {
                    let _id = serial
                        .offer_message(generation, tick, arrival, input)
                        .unwrap();
                }
                InputObservation::Frontier { tick } => {
                    let _id = serial.advance_frontier(generation, tick).unwrap();
                }
            }
        }
    }
    serial.pump().unwrap();
    let reference_audio = render_input(&mut serial, 512, &[512]);
    serial.finalize().unwrap();
    let reference = summary(&serial, generations);
    let reference_receipts = receipts(&mut serial, generations);
    for partitions in [vec![512], vec![64], vec![256], vec![1, 37, 128, 3, 256]] {
        let (mut control, audio, _halt, generations) = split(16, 32);
        let (mut writer, mut reader, mut callback) = queues(audio, 1);
        let barrier = Arc::new(Barrier::new(3));
        let mut consumers = Vec::new();
        let mut producers = Vec::new();
        for port in 0..2 {
            let (mut producer, consumer) = HeapRb::new(1).split();
            consumers.push(consumer);
            let barrier = Arc::clone(&barrier);
            producers.push(std::thread::spawn(move || {
                let fixture = observations(port);
                producer.try_push(fixture[0]).unwrap();
                // Deterministic pressure: merger waits until both failed sends retain ownership.
                let mut pending = producer.try_push(fixture[1]).unwrap_err();
                barrier.wait();
                loop {
                    match producer.try_push(pending) {
                        Ok(()) => break,
                        Err(returned) => {
                            pending = returned;
                            std::thread::yield_now();
                        }
                    }
                }
                pending = fixture[2];
                loop {
                    match producer.try_push(pending) {
                        Ok(()) => break,
                        Err(returned) => {
                            pending = returned;
                            std::thread::yield_now();
                        }
                    }
                }
                producer // Queue backing remains alive through audio join.
            }));
        }
        let prepared = Arc::new(AtomicBool::new(false));
        let worker_prepared = Arc::clone(&prepared);
        let stalled = Arc::new(Barrier::new(2));
        let audio_stalled = Arc::clone(&stalled);
        let worker = std::thread::spawn(move || {
            barrier.wait();
            let mut received = [0; 2];
            while received != [3; 2] {
                for (port, consumer) in consumers.iter_mut().enumerate() {
                    if let Some(observation) = consumer.try_pop() {
                        let _id = control
                            .offer_observation(generations[port], observation)
                            .unwrap();
                        received[port] += 1;
                    }
                }
                std::thread::yield_now();
            }
            let mut send = |mut packet| loop {
                match writer.try_push(packet) {
                    Ok(()) => break,
                    Err(returned) => {
                        packet = returned;
                        std::thread::yield_now();
                    }
                }
            };
            send(
                control
                    .prepare_command(SampleTime::ZERO, SessionCommand::Play)
                    .unwrap(),
            );
            send(
                control
                    .prepare_command(SampleTime::new(320), SessionCommand::Stop)
                    .unwrap(),
            );
            let mut issued = 2;
            while let Some(packet) = control.next_packet().unwrap() {
                send(packet);
                issued += 1;
            }
            assert_eq!(issued, 10);
            worker_prepared.store(true, Ordering::Release);
            // No completion collection during any rendering: one return slot only.
            stalled.wait();
            let mut received = 0;
            let mut commands = Vec::new();
            while received < issued {
                if let Some(completion) = reader.try_pop() {
                    if let Some((_, outcome)) = control.collect(completion).unwrap() {
                        commands.push(outcome);
                    }
                    received += 1;
                } else {
                    std::thread::yield_now();
                }
            }
            (control, writer, reader, consumers, commands)
        });
        let audio = std::thread::spawn(move || {
            while !prepared.load(Ordering::Acquire) {
                assert_eq!(
                    crate::render_allocation::count_allocs(|| callback.admit()),
                    0
                );
                std::thread::yield_now();
            }
            let mut samples = vec![9.0; 512];
            let mut offset = 0;
            let mut part = 0;
            while offset < samples.len() {
                let count = partitions[part % partitions.len()].min(samples.len() - offset);
                assert_eq!(
                    crate::render_allocation::count_allocs(|| callback
                        .render(&mut samples[offset..offset + count])
                        .unwrap()),
                    0
                );
                offset += count;
                part += 1;
            }
            assert_eq!(callback.returned, 1);
            audio_stalled.wait();
            while callback.returned < 10 {
                assert_eq!(
                    crate::render_allocation::count_allocs(|| callback.flush()),
                    0
                );
                std::thread::yield_now();
            }
            (callback, samples)
        });
        let (callback, samples) = audio.join().unwrap();
        let producer_owners: Vec<_> = producers.into_iter().map(|p| p.join().unwrap()).collect();
        let (control, _writer, _reader, _consumers, commands) = worker.join().unwrap();
        assert_eq!(producer_owners.len(), 2);
        assert!(callback.refused.is_none());
        assert!(commands.iter().all(|outcome| matches!(outcome, LoopTransferOutcome::Command(r) if matches!(r.outcome,SessionOutcome::Applied {..}))));
        assert_eq!(commands.len(), 2);
        let mut owner = callback.audio.reunite(control).unwrap();
        owner.finalize().unwrap();
        assert_eq!(samples, reference_audio);
        assert_eq!(summary(&owner, generations), reference);
        assert_eq!(receipts(&mut owner, generations), reference_receipts);
    }
}

fn fill(control: &mut InputCaptureControl, writer: &mut HeapProd<LoopTransferPacket>, stop: u64) {
    writer
        .try_push(
            control
                .prepare_command(SampleTime::ZERO, SessionCommand::Play)
                .unwrap(),
        )
        .unwrap();
    writer
        .try_push(
            control
                .prepare_command(SampleTime::new(stop), SessionCommand::Stop)
                .unwrap(),
        )
        .unwrap();
    while let Some(packet) = control.next_packet().unwrap() {
        writer.try_push(packet).unwrap();
    }
}
fn settle(
    control: &mut InputCaptureControl,
    callback: &mut Callback,
    reader: &mut HeapCons<LoopTransferCompletion>,
) -> Vec<LoopTransferOutcome> {
    let mut commands = Vec::new();
    while let Some(completion) = reader
        .try_pop()
        .or_else(|| callback.failed.take())
        .or_else(|| callback.audio.take_completed())
    {
        if let Some((_, outcome)) = control.collect(completion).unwrap() {
            commands.push(outcome);
        }
    }
    if let Some(packet) = callback.refused.take() {
        let _id = control.cancel(packet).unwrap();
    }
    while let Some(packet) = callback.input.try_pop() {
        let _id = control.cancel(packet).unwrap();
    }
    commands
}

#[test]
fn independent_halt_stops_audio_with_worker_stalled_and_full_completion_queue() {
    let (mut control, audio, halt, generations) = split(16, 32);
    offer(&mut control, generations);
    let (mut writer, mut reader, mut callback) = queues(audio, 36);
    fill(&mut control, &mut writer, 512);
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = Arc::clone(&barrier);
    let worker = std::thread::spawn(move || {
        worker_barrier.wait();
        control
    });
    let (ready, waiting) = std::sync::mpsc::sync_channel(1);
    let resume = Arc::new(Barrier::new(2));
    let audio_resume = Arc::clone(&resume);
    let audio = std::thread::spawn(move || {
        let mut samples = [9.0; 192];
        assert_eq!(
            crate::render_allocation::count_allocs(|| callback.render(&mut samples).unwrap()),
            0
        );
        let before = callback.audio.acknowledged();
        assert_eq!(callback.returned, 1);
        ready.send(()).unwrap();
        audio_resume.wait();
        assert_eq!(
            crate::render_allocation::count_allocs(|| assert!(
                callback.render(&mut samples).is_err()
            )),
            0
        );
        assert_eq!(samples, [0.0; 192]);
        assert_eq!(callback.audio.acknowledged(), before);
        callback
    });
    waiting.recv().unwrap();
    assert_eq!(
        crate::render_allocation::count_allocs(|| halt.request_stop()),
        0
    );
    resume.wait();
    let mut callback = audio.join().unwrap();
    barrier.wait();
    let mut control = worker.join().unwrap();
    let commands = settle(&mut control, &mut callback, &mut reader);
    assert_eq!(commands.len(), 2);
    let mut owner = callback.audio.reunite(control).unwrap();
    assert!(owner.finalize().is_err()); // Audio join does not fence either source.
    for generation in generations {
        owner.acknowledge_input_quiescence(generation).unwrap();
    }
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
    assert_eq!(receipts(&mut owner, generations).len(), 8);
}

#[test]
fn no_final_callback_loss_retains_outcomes_and_old_take_across_reconnect() {
    let (mut control, audio, old_halt, generations) = split(8, 32);
    let _id = control
        .offer_observation(generations[0], observation(0, 10, 20, [0x90, 60, 100]))
        .unwrap();
    for (port, generation) in generations.into_iter().enumerate() {
        let _id = control
            .offer_observation(
                generation,
                InputObservation::Frontier {
                    tick: tick(port, 100),
                },
            )
            .unwrap();
    }
    let (mut writer, mut reader, mut callback) = queues(audio, 36);
    fill(&mut control, &mut writer, 512);
    let mut callback = std::thread::spawn(move || {
        let mut samples = [0.0; 192];
        assert_eq!(
            crate::render_allocation::count_allocs(|| callback.render(&mut samples).unwrap()),
            0
        );
        callback
    })
    .join()
    .unwrap();
    let before = callback.audio.acknowledged();
    control.device_lost(generations[1]).unwrap();
    callback.audio.synchronize_halt().unwrap();
    let commands = settle(&mut control, &mut callback, &mut reader);
    assert_eq!(commands.len(), 2);
    let mut owner = callback.audio.reunite(control).unwrap();
    assert!(owner.finalize().is_err());
    owner.acknowledge_input_quiescence(generations[0]).unwrap();
    assert!(owner.finalize().is_err());
    owner.acknowledge_input_quiescence(generations[1]).unwrap();
    owner.finalize().unwrap();
    assert_eq!(owner.acknowledged(), before);
    assert_eq!(owner.result().unwrap().window().end(), SampleTime::new(100));
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
    assert_eq!(receipts(&mut owner, generations).len(), 5);
    let (old_take, mut inputs) = owner.into_parts().unwrap();
    for (port, input) in inputs.iter_mut().enumerate() {
        let old = generations[port];
        let clock = input.clock().unwrap();
        input.retire(old).unwrap();
        let new = input.begin().unwrap();
        input.prepare(new, clock).unwrap();
        assert_eq!(
            input.offer_message(
                old,
                tick(port, 10),
                SampleTime::new(20),
                Midi1Input::from_bytes([0x90, 60, 1]).unwrap()
            ),
            Err(InputError::Stale)
        );
        old_halt.request_device_lost();
        assert_eq!(input.state(), ConnectionState::Ready);
    }
    assert_eq!(old_take.result().unwrap().records().count(), 1);
    // A newly prepared composition has its own signal; an old handle cannot halt it.
    let (mut control, mut audio, _new_halt, new) = split(8, 32);
    let packet = control
        .prepare_command(SampleTime::ZERO, SessionCommand::Play)
        .unwrap();
    audio.enqueue(packet).unwrap();
    old_halt.request_stop();
    let mut samples = [0.0; 64];
    audio
        .render(AudioBlockMut::new(&mut samples, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_ne!(new, generations);
}

#[test]
fn split_budget_and_unresolved_reunion_preserve_original_input_and_transfer_owners() {
    let (session, inputs, generations) = prepared(8, 32);
    let owner = InputCaptureSession::prepare(session, inputs, BUDGET).unwrap();
    let (owner, _) = owner
        .split(PreparedBytes::measured(0))
        .err()
        .unwrap()
        .into_parts();
    let (mut control, audio, _halt) = owner.split(BUDGET).unwrap();
    let bytes = control.bytes();
    for generation in generations {
        control.start_input(generation).unwrap();
    }
    let packet = control.next_packet().unwrap().unwrap();
    let error = audio.reunite(control).err().unwrap();
    assert!(matches!(
        error.error(),
        InputCaptureError::Transfer(LoopTransferError::Outstanding)
    ));
    let (mut control, mut audio) = error.into_split().unwrap();
    let _id = control.cancel(packet).unwrap(); // Cancellation is a terminal input discontinuity.
    audio.synchronize_halt().unwrap();
    let mut owner = audio.reunite(control).unwrap();
    assert_eq!(receipts(&mut owner, generations).len(), 2);
    for short in [false, true] {
        let (session, inputs, _) = prepared(8, 32);
        let owner = InputCaptureSession::prepare(session, inputs, BUDGET).unwrap();
        assert_eq!(
            owner
                .split(PreparedBytes::measured(bytes.get() - u64::from(short)))
                .is_err(),
            short
        );
    }
}

#[test]
fn audio_fault_publishes_closure_and_returns_queued_unadmitted_observations() {
    let (mut control, audio, _halt, generations) = split(8, 32);
    offer(&mut control, generations);
    let (mut writer, mut reader, mut callback) = queues(audio, 36);
    fill(&mut control, &mut writer, 512);
    let mut callback = std::thread::spawn(move || {
        let mut initial = [0.0; 192];
        assert_eq!(
            crate::render_allocation::count_allocs(|| callback.render(&mut initial).unwrap()),
            0
        );
        let before = callback.audio.acknowledged();
        let mut oversized = [9.0; 2049];
        assert_eq!(
            crate::render_allocation::count_allocs(|| assert!(
                callback.render(&mut oversized).is_err()
            )),
            0
        );
        assert_eq!(oversized, [0.0; 2049]);
        assert_eq!(callback.audio.acknowledged(), before);
        callback
    })
    .join()
    .unwrap();
    let original = observation(1, 400, 400, [0x90, 64, 100]);
    assert_eq!(
        control.offer_observation(generations[1], original),
        Err((original, InputError::State))
    );
    assert!(control.next_packet().unwrap().is_none());
    assert_eq!(
        control
            .input(generations[0])
            .unwrap()
            .discontinuity()
            .unwrap()
            .reason,
        InputError::PeerInterrupted
    );
    let _commands = settle(&mut control, &mut callback, &mut reader);
    let mut owner = callback.audio.reunite(control).unwrap();
    for generation in generations {
        owner.acknowledge_input_quiescence(generation).unwrap();
    }
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
    assert_eq!(receipts(&mut owner, generations).len(), 8);
}

#[test]
fn source_stall_does_not_hold_ordered_stop_and_delayed_note_retains_its_refusal() {
    let (mut control, audio, _halt, generations) = split(8, 32);
    let _id = control
        .offer_observation(generations[0], observation(0, 10, 20, [0x90, 60, 100]))
        .unwrap();
    let _id = control
        .offer_observation(
            generations[0],
            InputObservation::Frontier { tick: tick(0, 128) },
        )
        .unwrap();
    let (mut writer, mut reader, mut callback) = queues(audio, 36);
    fill(&mut control, &mut writer, 128);
    let mut callback = std::thread::spawn(move || {
        let mut samples = [9.0; 512];
        assert_eq!(
            crate::render_allocation::count_allocs(|| callback.render(&mut samples).unwrap()),
            0
        );
        assert!(samples[192..].iter().all(|sample| *sample == 0.0));
        callback
    })
    .join()
    .unwrap();
    let commands = settle(&mut control, &mut callback, &mut reader);
    assert_eq!(commands.len(), 2);
    assert!(commands.iter().all(|outcome| matches!(outcome,LoopTransferOutcome::Command(r) if matches!(r.outcome,SessionOutcome::Applied {..}))));
    let _id = control
        .offer_observation(
            generations[1],
            InputObservation::Frontier { tick: tick(1, 128) },
        )
        .unwrap();
    while let Some(packet) = control.next_packet().unwrap() {
        callback.audio.enqueue(packet).unwrap();
    }
    callback.audio.drain_stopped_sources().unwrap();
    let _commands = settle(&mut control, &mut callback, &mut reader);
    callback.audio.synchronize_halt().unwrap();
    let mut owner = callback.audio.reunite(control).unwrap();
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
    let note = receipts
        .iter()
        .find(|r| matches!(r.observation, InputObservation::Message { .. }))
        .unwrap();
    assert!(matches!(
        note.outcome,
        InputOutcome::Delivered(SessionSourceOutcome::Refused(
            NoteCaptureError::PastBoundary
        ))
    ));
    assert_eq!(note.observation, observation(0, 10, 20, [0x90, 60, 100]));
}

#[test]
fn late_quality_from_second_input_survives_prior_faults_before_reunion() {
    for path in 0..3 {
        let (mut control, audio, _halt, generations) = split(8, 32);
        for (port, generation) in generations.into_iter().enumerate() {
            let _id = control
                .offer_observation(
                    generation,
                    InputObservation::Frontier {
                        tick: tick(port, 128),
                    },
                )
                .unwrap();
        }
        let (mut writer, mut reader, mut callback) = queues(audio, 36);
        fill(&mut control, &mut writer, 128);
        let mut samples = [0.0; 512];
        callback.render(&mut samples).unwrap();
        let original = observation(1, 20, 200, [0x90, 60, 100]);
        if path == 1 {
            let peer_future = observation(0, 200, 199, [0x90, 61, 100]);
            control
                .record_pre_ring_failure(generations[0], peer_future, InputError::Future)
                .unwrap();
            assert!(control.next_packet().unwrap().is_none());
            assert_eq!(
                control
                    .input(generations[1])
                    .unwrap()
                    .discontinuity()
                    .unwrap()
                    .reason,
                InputError::PeerInterrupted
            );
            control
                .record_pre_ring_failure(generations[1], original, InputError::Order)
                .unwrap();
        } else if path == 2 {
            let earlier_fault = InputObservation::Frontier { tick: tick(1, 0) };
            assert_eq!(
                control.offer_observation(generations[1], earlier_fault),
                Err((earlier_fault, InputError::Order))
            );
            control
                .record_pre_ring_failure(generations[1], original, InputError::Order)
                .unwrap();
        } else {
            assert_eq!(
                control.offer_observation(generations[1], original),
                Err((original, InputError::Order))
            );
        }
        callback.audio.synchronize_halt().unwrap();
        let _commands = settle(&mut control, &mut callback, &mut reader);
        let mut owner = callback.audio.reunite(control).unwrap();
        if path != 0 {
            let second = owner.input(generations[1]).unwrap();
            assert_eq!(
                second.discontinuity().unwrap().reason,
                if path == 1 {
                    InputError::PeerInterrupted
                } else {
                    InputError::Order
                }
            );
            assert_eq!(
                second.pre_ring_failure().unwrap().observation,
                Some(original)
            );
        }
        for generation in generations {
            owner.acknowledge_input_quiescence(generation).unwrap();
        }
        owner.finalize().unwrap();
        let result = owner.result().unwrap();
        assert_eq!(result.sealed_outcome(), CaptureOutcome::Interrupted);
        assert_eq!(
            result.quality().first_late().unwrap().time,
            SampleTime::new(20)
        );
        assert_eq!(result.quality().late_count().as_u64(), 1);
        assert_eq!(
            result.quality().effective_outcome(result.sealed_outcome()),
            CaptureOutcome::Interrupted
        );
    }
}

fn uncertain_split() -> (
    InputCaptureControl,
    InputCaptureAudio,
    InputCaptureHalt,
    ConnectionGeneration,
) {
    let mut capture = LoopCaptureSession::prepare(
        stream(0, 2048),
        limits(2, 64, 1_048_576),
        PreparedBytes::measured(8192),
    )
    .unwrap();
    let mut input = SimulatedNoteInput::new(
        EndpointId::new("uncertain-thread".to_owned()).unwrap(),
        InputLimits {
            cells: InputCapacity::new(8).unwrap(),
            bytes: BUDGET,
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
    let _ticket = capture.arm(super::super::input(), &[source]).unwrap();
    let serial =
        LoopRecordingSession::prepare(capture, command_limits(4, 16384), source_limits(32, 32768))
            .unwrap();
    let owner =
        InputCaptureSession::prepare(serial, vec![input].into_boxed_slice(), BUDGET).unwrap();
    let (mut control, audio, halt) = owner.split(BUDGET).unwrap();
    control.start_input(generation).unwrap();
    (control, audio, halt, generation)
}

#[test]
fn pre_ring_failure_validates_mapping_and_records_one_original() {
    let (mut control, _audio, _halt, generation) = uncertain_split();
    let uncertain = InputObservation::Message {
        tick: InputTick::new(200),
        arrival: SampleTime::new(200),
        input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
    };
    assert_eq!(
        control.record_pre_ring_failure(generation, uncertain, InputError::Order),
        Err(InputError::State)
    );
    let future = InputObservation::Message {
        tick: InputTick::new(105),
        arrival: SampleTime::new(9),
        input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
    };
    assert_eq!(
        control.record_pre_ring_failure(generation, future, InputError::Uncertain),
        Err(InputError::State)
    );
    assert_eq!(
        control.record_pre_ring_failure(
            generation,
            InputObservation::Frontier {
                tick: InputTick::new(105),
            },
            InputError::Future,
        ),
        Err(InputError::State)
    );
    assert!(
        control
            .input(generation)
            .unwrap()
            .pre_ring_failure()
            .is_none()
    );
    control
        .record_pre_ring_failure(generation, future, InputError::Future)
        .unwrap();
    control
        .record_pre_ring_failure(generation, future, InputError::Future)
        .unwrap();
    let different = InputObservation::Message {
        tick: InputTick::new(115),
        arrival: SampleTime::new(10),
        input: Midi1Input::from_bytes([0x90, 61, 100]).unwrap(),
    };
    assert_eq!(
        control.record_pre_ring_failure(generation, different, InputError::Future),
        Err(InputError::State)
    );
    let retained = control
        .input(generation)
        .unwrap()
        .pre_ring_failure()
        .unwrap();
    assert_eq!(retained.reason, InputError::Future);
    assert_eq!(retained.observation, Some(future));
}

#[test]
fn uncertainty_uses_frozen_selection_and_cannot_report_complete_after_fault() {
    for (stopped, input_tick, inside) in [
        (true, 0, true),
        (true, 200, true),
        (true, 2000, false),
        (false, 200, true),
        (false, 800, false),
    ] {
        for before_ring in [false, true] {
            let (mut control, audio, _halt, generation) = uncertain_split();
            let _id = control
                .offer_observation(
                    generation,
                    InputObservation::Frontier {
                        tick: InputTick::new(if stopped { 1285 } else { 405 }),
                    },
                )
                .unwrap();
            let (mut writer, mut reader, mut callback) = queues(audio, 36);
            fill(&mut control, &mut writer, if stopped { 128 } else { 512 });
            let mut callback = std::thread::spawn(move || {
                let mut samples = vec![0.0; if stopped { 512 } else { 192 }];
                assert_eq!(
                    crate::render_allocation::count_allocs(|| callback
                        .render(&mut samples)
                        .unwrap()),
                    0
                );
                callback
            })
            .join()
            .unwrap();
            let refused = InputObservation::Message {
                tick: InputTick::new(input_tick),
                arrival: SampleTime::new(200),
                input: Midi1Input::from_bytes([0x90, 60, 100]).unwrap(),
            };
            if before_ring {
                let reason = if input_tick == 0 {
                    InputError::ClockRange
                } else {
                    InputError::Uncertain
                };
                control
                    .record_pre_ring_failure(generation, refused, reason)
                    .unwrap();
            } else {
                assert!(
                    matches!(control.offer_observation(generation,refused),Err((same,InputError::Uncertain | InputError::ClockRange)) if same == refused)
                );
            }
            callback.audio.synchronize_halt().unwrap();
            let _commands = settle(&mut control, &mut callback, &mut reader);
            let mut owner = callback.audio.reunite(control).unwrap();
            owner.acknowledge_input_quiescence(generation).unwrap();
            owner.finalize().unwrap();
            let result = owner.result().unwrap();
            assert_eq!(result.quality().first_uncertain_source().is_some(), inside);
            assert_eq!(result.sealed_outcome(), CaptureOutcome::Interrupted);
            assert_eq!(
                result.quality().effective_outcome(result.sealed_outcome()),
                CaptureOutcome::Interrupted
            );
            assert_eq!(
                owner
                    .input(generation)
                    .unwrap()
                    .discontinuity()
                    .unwrap()
                    .observation,
                Some(refused)
            );
        }
    }
}

#[test]
fn input_cell_credit_is_held_after_core_collection_until_input_receipt_collection() {
    for collect_input in [false, true] {
        let (mut control, audio, _halt, generations) = split(3, 2);
        for (port, generation) in generations.into_iter().enumerate() {
            let _id = control
                .offer_observation(
                    generation,
                    InputObservation::Frontier {
                        tick: tick(port, 128),
                    },
                )
                .unwrap();
        }
        let (mut writer, mut reader, mut callback) = queues(audio, 36);
        fill(&mut control, &mut writer, 128);
        assert!(control.next_packet().unwrap().is_none()); // Initial fences hold both core credits.
        let mut samples = [0.0; 512];
        callback.render(&mut samples).unwrap();
        assert!(control.next_packet().unwrap().is_none()); // Receipt transit does not return credit.
        let _commands = settle(&mut control, &mut callback, &mut reader);
        let first = control.next_packet().unwrap().unwrap();
        let second = control.next_packet().unwrap().unwrap();
        callback.audio.enqueue(first).unwrap();
        callback.audio.enqueue(second).unwrap();
        callback.audio.drain_stopped_sources().unwrap();
        let _commands = settle(&mut control, &mut callback, &mut reader);
        if collect_input {
            let _receipt = control.collect_input(generations[0]).unwrap().unwrap();
        }
        let original = observation(0, 200, 200, [0x90, 60, 100]);
        let result = control.offer_observation(generations[0], original);
        if collect_input {
            assert!(result.is_ok());
        } else {
            assert_eq!(result, Err((original, InputError::Full)));
        }
    }
}

#[test]
fn foreign_completion_and_reunion_return_owners_without_spending_local_credit() {
    let (mut a, mut audio_a, halt_a, ga) = split(8, 32);
    let (mut b, mut audio_b, halt_b, gb) = split(8, 32);
    audio_a.enqueue(a.next_packet().unwrap().unwrap()).unwrap();
    audio_b.enqueue(b.next_packet().unwrap().unwrap()).unwrap();
    halt_a.request_stop();
    halt_b.request_stop();
    audio_a.synchronize_halt().unwrap();
    audio_b.synchronize_halt().unwrap();
    let completion = audio_b.take_completed().unwrap();
    let (completion, error) = *a.collect(completion).unwrap_err();
    assert_eq!(error, LoopTransferError::Unknown);
    assert!(b.collect(completion).unwrap().is_none());
    let error = audio_a.reunite(b).err().unwrap();
    assert!(matches!(
        error.error(),
        InputCaptureError::Transfer(LoopTransferError::Origin)
    ));
    let (b, mut audio_a) = error.into_split().unwrap();
    assert!(
        a.collect(audio_a.take_completed().unwrap())
            .unwrap()
            .is_none()
    );
    let mut first = audio_a.reunite(a).unwrap();
    let mut second = audio_b.reunite(b).unwrap();
    assert_eq!(receipts(&mut first, ga).len(), 2);
    assert_eq!(receipts(&mut second, gb).len(), 2);
}

#[test]
fn halt_refuses_packet_custody_without_losing_credit_or_allocating() {
    let (mut control, mut audio, halt, generations) = split(8, 32);
    let packet = control.next_packet().unwrap().unwrap();
    let id = packet.id();
    let mut returned = None;
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            halt.request_stop();
            halt.request_device_lost(); // First request wins; no reset or replacement.
            let (packet, error) = audio.enqueue(packet).unwrap_err();
            assert_eq!(error, LoopTransferError::Closed);
            returned = Some(packet);
        }),
        0
    );
    let error = audio.reunite(control).err().unwrap();
    let (mut control, audio) = error.into_split().unwrap();
    assert_eq!(control.cancel(returned.unwrap()).unwrap(), id);
    let mut owner = audio.reunite(control).unwrap();
    assert_eq!(receipts(&mut owner, generations).len(), 2);
}
