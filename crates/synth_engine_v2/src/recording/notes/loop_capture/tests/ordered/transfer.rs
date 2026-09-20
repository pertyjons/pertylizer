use super::*;
use crate::host::session::loop_transfer::*;
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};

const TRANSFER_BYTES: PreparedBytes = PreparedBytes::measured(65_536);
const QUEUE_SLOTS: usize = 36;

struct Callback {
    audio: LoopRecordingAudio,
    input: HeapCons<LoopTransferPacket>,
    output: HeapProd<LoopTransferCompletion>,
    failed: Option<LoopTransferCompletion>,
    refused: Option<LoopTransferPacket>,
}
impl Callback {
    fn admit(&mut self, prefix: usize) {
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
        if let Some(packet) = self.failed.take()
            && let Err(packet) = self.output.try_push(packet)
        {
            self.failed = Some(packet);
            return;
        }
        // The queue may be smaller than the credit population. Runtime cells retain the rest.
        for _ in 0..QUEUE_SLOTS {
            if self.output.is_full() {
                break;
            }
            let Some(packet) = self.audio.take_completed() else {
                break;
            };
            if let Err(packet) = self.output.try_push(packet) {
                self.failed = Some(packet);
                break;
            }
        }
    }
    fn callback(&mut self, samples: &mut [f32]) {
        let prefix = self.input.occupied_len();
        self.admit(prefix);
        self.audio
            .render(AudioBlockMut::new(samples, samples.len(), ChannelLayout::Mono).unwrap())
            .unwrap();
        self.flush();
    }
}

fn queues(
    audio: LoopRecordingAudio,
    returns: usize,
) -> (
    HeapProd<LoopTransferPacket>,
    HeapCons<LoopTransferCompletion>,
    Callback,
) {
    // Concrete bounded backing is prepared off-thread and both endpoints survive audio join.
    // 64 KiB covers packet cells, ring objects, padded Arc headers and failed-transfer slots.
    let charge = QUEUE_SLOTS * size_of::<LoopTransferPacket>()
        + returns * size_of::<LoopTransferCompletion>()
        + size_of::<HeapRb<LoopTransferPacket>>()
        + size_of::<HeapRb<LoopTransferCompletion>>()
        + 4 * 256
        + size_of::<Callback>();
    assert!(charge <= 65_536);
    let (input, reader) = HeapRb::new(QUEUE_SLOTS).split();
    let (writer, output) = HeapRb::new(returns).split();
    (
        input,
        output,
        Callback {
            audio,
            input: reader,
            output: writer,
            failed: None,
            refused: None,
        },
    )
}

fn source_actions(
    owner: &LoopRecordingSession,
    sources: [ConnectionGeneration; 2],
    end: u64,
) -> Vec<SessionSourceAction> {
    let mut actions: Vec<_> = sources
        .into_iter()
        .map(|s| fence_action(owner, s, 0))
        .collect();
    for (source, at, bytes) in [
        (sources[0], 1, [0x90, 60, 100]),
        (sources[1], 49, [0x90, 60, 90]),
        (sources[0], 50, [0x80, 60, 0]),
        (sources[1], 100, [0x80, 60, 0]),
        (sources[0], 299, [0x90, 62, 100]),
    ] {
        actions.push(SessionSourceAction::Publish {
            source,
            stamp: CaptureStamp::exact_fixture(
                owner.acknowledged().epoch,
                SampleTime::new(at),
                SampleTime::new(at),
            )
            .unwrap(),
            input: Midi1Input::from_bytes(bytes).unwrap(),
            audition: AuditionTrace::NotOffered,
        });
    }
    actions.extend(sources.into_iter().map(|s| fence_action(owner, s, end)));
    actions
}

fn normalized(owner: &LoopRecordingSession, sources: [ConnectionGeneration; 2]) -> String {
    let result = owner.result().unwrap();
    let records: Vec<_> = result
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
        "{:?} {:?} {:?} {records:?} {passes:?} {carry:?}",
        result.window().start(),
        result.window().end(),
        result.sealed_outcome()
    )
}

fn serial_receipts(owner: &mut LoopRecordingSession) -> Vec<String> {
    let mut receipts = Vec::new();
    while let Some(r) = owner.collect() {
        receipts.push(format!(
            "command {:?} {:?} {:?}",
            r.boundary.command, r.outcome, r.capture
        ));
    }
    while let Some(r) = owner.collect_source() {
        receipts.push(source_outcome(&r));
    }
    receipts.sort();
    receipts
}
fn source_outcome(receipt: &crate::host::session::SessionSourceReceipt) -> String {
    match &receipt.outcome {
        SessionSourceOutcome::Published(r) => format!(
            "source {} published {:?}",
            receipt.id.serial(),
            r.occurrence.map(|id| id.serial())
        ),
        other => format!("source {} {other:?}", receipt.id.serial()),
    }
}
fn collect(
    control: &mut LoopRecordingControl,
    completion: LoopTransferCompletion,
) -> (LoopTransferId, String) {
    let (id, outcome) = control.collect(completion).unwrap();
    let text = match outcome {
        LoopTransferOutcome::Command(r) => format!(
            "command {:?} {:?} {:?}",
            r.boundary.command, r.outcome, r.capture
        ),
        LoopTransferOutcome::Source(r) => source_outcome(&r),
        other => panic!("unexpected transfer outcome: {other:?}"),
    };
    (id, text)
}

#[test]
fn real_queues_and_three_threads_match_serial_audio_raw_passes_carry_and_outcomes() {
    for stop in [320, 1600] {
        let total = if stop == 320 { 512 } else { 2048 };
        let (mut serial, sources) = setup();
        queue_transport(&mut serial, stop);
        for action in source_actions(&serial, sources, stop) {
            let _id = serial.offer_source(action).unwrap();
        }
        let reference_audio = audio(&mut serial, total, &[total]);
        serial.finalize().unwrap();
        let reference_raw = normalized(&serial, sources);
        let reference_receipts = serial_receipts(&mut serial);
        assert!(reference_audio.iter().any(|s| *s != 0.0));
        for partitions in [&[total][..], &[64], &[256], &[1, 37, 128, 3, 256]] {
            let (session, sources) = setup();
            let actions = source_actions(&session, sources, stop);
            let (mut control, audio) = session.split(TRANSFER_BYTES).unwrap();
            let (mut writer, mut reader, mut callback) = queues(audio, 1);
            let mut issued = Vec::new();
            for (at, command) in [(0, SessionCommand::Play), (stop, SessionCommand::Stop)] {
                let packet = control
                    .prepare_command(SampleTime::new(at), command)
                    .unwrap();
                issued.push(packet.id());
                writer.try_push(packet).unwrap();
            }
            for action in actions {
                let packet = control.prepare_source(action).unwrap();
                issued.push(packet.id());
                writer.try_push(packet).unwrap();
            }
            // The reader deliberately stalls through every callback, with only one return slot.
            let partitions = partitions.to_vec();
            let (callback, samples) = std::thread::spawn(move || {
                let mut samples = vec![9.0; total];
                let mut offset = 0;
                let mut part = 0;
                while offset < total {
                    let count = partitions[part % partitions.len()].min(total - offset);
                    assert_eq!(
                        crate::render_allocation::count_allocs(|| {
                            callback.callback(&mut samples[offset..offset + count]);
                        }),
                        0
                    );
                    offset += count;
                    part += 1;
                }
                (callback, samples)
            })
            .join()
            .unwrap();
            assert_eq!(samples, reference_audio);
            assert!(control.has_outstanding());
            assert_eq!(reader.occupied_len(), 1);
            let mut callback = callback;
            let mut receipts = Vec::new();
            let mut ids = Vec::new();
            while let Some(completion) = reader.try_pop() {
                let (id, text) = collect(&mut control, completion);
                ids.push(id);
                receipts.push(text);
            }
            while let Some(completion) = callback.audio.take_completed() {
                let (id, text) = collect(&mut control, completion);
                ids.push(id);
                receipts.push(text);
            }
            assert!(callback.failed.is_none() && callback.refused.is_none());
            assert!(!control.has_outstanding());
            ids.sort_by_key(|id| id.serial());
            assert_eq!(ids, issued);
            receipts.sort();
            assert_eq!(receipts, reference_receipts);
            // Join precedes ownership transfer to a distinct finalization worker.
            let raw = std::thread::spawn(move || {
                let mut owner = callback.audio.reunite(control).unwrap();
                owner.finalize().unwrap();
                let raw = normalized(&owner, sources);
                owner.close_completed().unwrap();
                for source in sources {
                    owner.acknowledge_source_quiescence(source).unwrap();
                }
                let quality = owner.result().unwrap().quality();
                owner.discard(quality).unwrap();
                raw
            })
            .join()
            .unwrap();
            assert_eq!(raw, reference_raw);
            drop(writer); // endpoints and their backing survive callback join
        }
    }
}

#[test]
fn source_stall_can_finish_on_worker_after_audio_join_without_callback() {
    let (session, sources) = setup();
    let actions = source_actions(&session, sources, 512);
    let (mut control, mut audio) = session.split(TRANSFER_BYTES).unwrap();
    for (at, command) in [(0, SessionCommand::Play), (320, SessionCommand::Stop)] {
        audio
            .enqueue(
                control
                    .prepare_command(SampleTime::new(at), command)
                    .unwrap(),
            )
            .unwrap();
    }
    for action in actions.iter().take(7) {
        audio
            .enqueue(control.prepare_source(*action).unwrap())
            .unwrap();
    }
    let mut audio = std::thread::spawn(move || {
        let mut samples = [0.0; 512];
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                audio
                    .render(AudioBlockMut::new(&mut samples, 512, ChannelLayout::Mono).unwrap())
                    .unwrap();
            }),
            0
        );
        audio
    })
    .join()
    .unwrap();
    while let Some(completion) = audio.take_completed() {
        let _receipt = collect(&mut control, completion);
    }
    let snapshot = audio.acknowledged();
    let source_worker = std::thread::spawn(move || actions.into_iter().skip(7).collect::<Vec<_>>());
    let delayed = source_worker.join().unwrap();
    // The source producer has joined; its explicit final fences still carry their own authority.
    std::thread::spawn(move || {
        let mut session = audio.reunite(control).unwrap();
        assert!(session.finalize().is_err());
        for action in delayed {
            let _id = session.offer_source(action).unwrap();
        }
        session.drain_stopped_sources().unwrap();
        assert_eq!(session.acknowledged(), snapshot);
        while session.collect_source().is_some() {}
        session.finalize().unwrap();
        assert_eq!(
            session.result().unwrap().window().end(),
            SampleTime::new(320)
        );
        assert_eq!(
            session.result().unwrap().sealed_outcome(),
            CaptureOutcome::Complete
        );
    })
    .join()
    .unwrap();
}

#[test]
fn all_credit_locations_survive_loss_and_queue_pressure_without_final_callback() {
    let (session, sources) = setup();
    let fences: Vec<_> = sources
        .into_iter()
        .map(|s| fence_action(&session, s, 0))
        .collect();
    let (mut control, audio) = session.split(TRANSFER_BYTES).unwrap();
    let (mut writer, mut reader, mut callback) = queues(audio, 1);
    let play = control
        .prepare_command(SampleTime::ZERO, SessionCommand::Play)
        .unwrap();
    writer.try_push(play).unwrap();
    for action in fences {
        writer
            .try_push(control.prepare_source(action).unwrap())
            .unwrap();
    }
    let stop = control
        .prepare_command(SampleTime::new(320), SessionCommand::Stop)
        .unwrap();
    writer.try_push(stop).unwrap();
    let unpublished = control
        .prepare_command(SampleTime::new(384), SessionCommand::Stop)
        .unwrap();
    let queued = control
        .prepare_command(SampleTime::new(448), SessionCommand::Stop)
        .unwrap();
    // Four command credits are now occupied; runtime/completion progress cannot renew one.
    assert!(matches!(
        control.prepare_command(SampleTime::new(512), SessionCommand::Stop),
        Err(LoopSessionError::Session(SessionError::Full))
    ));
    let mut callback = std::thread::spawn(move || {
        let mut samples = [0.0; 128];
        assert_eq!(
            crate::render_allocation::count_allocs(|| callback.callback(&mut samples)),
            0
        );
        // Deliberately exercise the owning failed-return slot against the already full ring.
        let completion = callback.audio.take_completed().unwrap();
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                callback.failed = Some(callback.output.try_push(completion).unwrap_err());
            }),
            0
        );
        callback
    })
    .join()
    .unwrap();
    writer.try_push(queued).unwrap(); // still in ingress at loss
    control.close_admission();
    callback.audio.device_lost_after_callback_join().unwrap();
    let mut resolved = Vec::new();
    resolved.push(control.cancel(unpublished).unwrap());
    while let Some(packet) = callback.input.try_pop() {
        resolved.push(control.cancel(packet).unwrap());
    }
    while let Some(completion) = reader.try_pop() {
        resolved.push(control.collect(completion).unwrap().0);
    }
    resolved.push(control.collect(callback.failed.take().unwrap()).unwrap().0);
    while let Some(completion) = callback.audio.take_completed() {
        resolved.push(control.collect(completion).unwrap().0);
    }
    resolved.sort_by_key(|id| id.serial());
    assert_eq!(
        resolved.iter().map(|id| id.serial()).collect::<Vec<_>>(),
        (1..=6).collect::<Vec<_>>()
    );
    assert!(!control.has_outstanding());
    let mut owner = callback.audio.reunite(control).unwrap();
    assert!(owner.finalize().is_err());
    for source in sources {
        owner.acknowledge_source_quiescence(source).unwrap();
    }
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
    assert_eq!(owner.result().unwrap().window().end(), SampleTime::ZERO);
}

#[test]
fn publication_after_captured_prefix_waits_and_late_play_is_identified_refusal() {
    let (session, sources) = setup();
    let fence = fence_action(&session, sources[0], 0);
    let (mut control, audio) = session.split(TRANSFER_BYTES).unwrap();
    let (mut writer, mut reader, mut callback) = queues(audio, QUEUE_SLOTS);
    let (cut_sent, cut_received) = std::sync::mpsc::sync_channel(0);
    let (sent, received) = std::sync::mpsc::sync_channel(0);
    let thread = std::thread::spawn(move || {
        let prefix = callback.input.occupied_len();
        assert_eq!(prefix, 0);
        cut_sent.send(()).unwrap();
        received.recv().unwrap(); // outside callback guard
        let mut samples = [7.0; 128];
        assert_eq!(
            crate::render_allocation::count_allocs(|| {
                callback.admit(prefix);
                callback
                    .audio
                    .render(AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).unwrap())
                    .unwrap();
                callback.flush();
            }),
            0
        );
        assert!(samples.iter().all(|s| *s == 0.0));
        assert_eq!(callback.input.occupied_len(), 2);
        assert_eq!(
            crate::render_allocation::count_allocs(|| callback.callback(&mut samples)),
            0
        );
        assert!(samples.iter().all(|s| *s == 0.0));
        callback
    });
    cut_received.recv().unwrap();
    let packet = control
        .prepare_command(SampleTime::ZERO, SessionCommand::Play)
        .unwrap();
    let id = packet.id();
    writer.try_push(packet).unwrap();
    writer
        .try_push(control.prepare_source(fence).unwrap())
        .unwrap();
    sent.send(()).unwrap();
    let mut callback = thread.join().unwrap();
    let completion = reader.try_pop().unwrap();
    assert_eq!(completion.id(), id);
    assert!(matches!(
        control.collect(completion).unwrap().1,
        LoopTransferOutcome::Refused(LoopSessionError::Session(SessionError::Boundary))
    ));
    let completion = reader.try_pop().unwrap();
    assert!(matches!(
        control.collect(completion).unwrap().1,
        LoopTransferOutcome::Refused(LoopSessionError::Session(SessionError::SourceOrder))
    ));
    assert!(!control.has_outstanding());
    callback.audio.device_lost_after_callback_join().unwrap();
    let _session = callback.audio.reunite(control).unwrap();
}

#[test]
fn foreign_reordered_and_closed_packets_return_ownership_and_reunion_waits_for_credits() {
    let (session, _) = setup();
    let (other, _) = setup();
    let (mut control, mut audio) = session.split(TRANSFER_BYTES).unwrap();
    let (mut foreign, foreign_audio) = other.split(TRANSFER_BYTES).unwrap();
    let packet = foreign
        .prepare_command(SampleTime::ZERO, SessionCommand::Play)
        .unwrap();
    let (packet, error) = audio.enqueue(packet).unwrap_err();
    assert_eq!(error, LoopTransferError::Origin);
    let _id = foreign.cancel(packet).unwrap();
    let first = control
        .prepare_command(SampleTime::new(128), SessionCommand::Stop)
        .unwrap();
    let second = control
        .prepare_command(SampleTime::new(256), SessionCommand::Stop)
        .unwrap();
    audio.enqueue(second).unwrap();
    let (first, error) = audio.enqueue(first).unwrap_err();
    assert_eq!(error, LoopTransferError::Order);
    let error = match audio.reunite(control) {
        Ok(_) => panic!("outstanding credits"),
        Err(error) => error,
    };
    assert_eq!(error.error(), LoopTransferError::Outstanding);
    let (mut control, mut audio, _) = error.into_parts();
    let _id = control.cancel(first).unwrap();
    audio.device_lost_after_callback_join().unwrap();
    let packet = control
        .prepare_command(SampleTime::new(320), SessionCommand::Stop)
        .unwrap();
    let (packet, error) = audio.enqueue(packet).unwrap_err();
    assert_eq!(error, LoopTransferError::Closed);
    let _id = control.cancel(packet).unwrap();
    let completion = audio.take_completed().unwrap();
    let (completion, error) = *foreign.collect(completion).unwrap_err();
    assert_eq!(error, LoopTransferError::Origin);
    assert!(matches!(
        control.collect(completion).unwrap().1,
        LoopTransferOutcome::Command(crate::host::session::SessionReceipt {
            outcome: SessionOutcome::Cancelled,
            ..
        })
    ));
    let error = match audio.reunite(foreign) {
        Ok(_) => panic!("wrong owner"),
        Err(error) => error,
    };
    assert_eq!(error.error(), LoopTransferError::Origin);
    let (foreign, audio, _) = error.into_parts();
    let _session = foreign_audio.reunite(foreign).unwrap();
    let _session = audio.reunite(control).unwrap();
}

#[test]
fn separate_source_credits_cannot_spend_stop_reserve_and_failed_sends_recover() {
    let (capture, sources) = armed();
    let session =
        LoopRecordingSession::prepare(capture, command_limits(2, 16384), source_limits(1, 32768))
            .unwrap();
    let fence = fence_action(&session, sources[0], 0);
    let (mut control, mut audio) = session.split(TRANSFER_BYTES).unwrap();
    let (mut writer, mut reader) = HeapRb::new(1).split();
    let play = control
        .prepare_command(SampleTime::ZERO, SessionCommand::Play)
        .unwrap();
    writer.try_push(play).unwrap();
    let source = control.prepare_source(fence).unwrap();
    assert!(matches!(
        control.prepare_source(fence),
        Err(LoopSessionError::Session(SessionError::Full))
    ));
    let source = writer.try_push(source).unwrap_err(); // caller retains failed send
    let stop = control
        .prepare_command(SampleTime::ZERO, SessionCommand::Stop)
        .unwrap();
    audio.enqueue(reader.try_pop().unwrap()).unwrap();
    audio.enqueue(stop).unwrap();
    let mut samples = [7.0; 128];
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            audio
                .render(AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).unwrap())
                .unwrap();
        }),
        0
    );
    assert!(samples.iter().all(|s| *s == 0.0));
    let mut held = Vec::new();
    while let Some(c) = audio.take_completed() {
        held.push(c);
    }
    assert_eq!(held.len(), 2);
    assert!(matches!(
        control.prepare_command(SampleTime::new(128), SessionCommand::Stop),
        Err(LoopSessionError::Session(SessionError::Full))
    ));
    // Return older completions last; the acknowledged clock cannot rewind.
    for completion in held.into_iter().rev() {
        let _receipt = control.collect(completion).unwrap();
    }
    let _id = control.cancel(source).unwrap();
    assert!(!control.has_outstanding());
    let mut session = audio.reunite(control).unwrap();
    session.device_lost().unwrap();
    for source in sources {
        session.acknowledge_source_quiescence(source).unwrap();
    }
    session.finalize().unwrap();
    assert_eq!(
        session.result().unwrap().window().start(),
        session.result().unwrap().window().end()
    );
}

#[test]
fn split_exact_budget_and_one_byte_short_refusal_preserve_original_owner() {
    let (session, _) = setup();
    let (control, audio) = session.split(TRANSFER_BYTES).unwrap();
    let bytes = control.transfer_bytes();
    let session = audio.reunite(control).unwrap();
    let error = match session.split(PreparedBytes::measured(bytes.get() - 1)) {
        Ok(_) => panic!("short budget"),
        Err(error) => error,
    };
    assert!(
        matches!(error.error(), LoopSessionError::Session(SessionError::ByteBudget { required, .. }) if *required == bytes)
    );
    let (session, _) = error.into_parts();
    let (control, audio) = session.split(bytes).unwrap();
    let mut session = audio.reunite(control).unwrap();
    let _id = session
        .offer(SampleTime::ZERO, SessionCommand::Play)
        .unwrap();
    let error = match session.split(bytes) {
        Ok(_) => panic!("used owner"),
        Err(error) => error,
    };
    assert!(matches!(error.error(), LoopSessionError::FreshCapture));
    let (mut session, _) = error.into_parts();
    session.device_lost().unwrap();
    assert_eq!(
        session.collect().unwrap().outcome,
        SessionOutcome::Cancelled
    );
}

#[test]
fn fence_racing_stop_has_retained_refusal_and_explicit_retry_without_extending_take() {
    let (session, sources) = setup();
    let epoch = session.acknowledged().epoch;
    let (mut control, mut audio) = session.split(TRANSFER_BYTES).unwrap();
    audio
        .enqueue(
            control
                .prepare_command(SampleTime::ZERO, SessionCommand::Play)
                .unwrap(),
        )
        .unwrap();
    audio
        .enqueue(
            control
                .prepare_command(SampleTime::new(128), SessionCommand::Stop)
                .unwrap(),
        )
        .unwrap();
    for source in sources {
        audio
            .enqueue(
                control
                    .prepare_source(SessionSourceAction::Fence {
                        source,
                        epoch,
                        frontier: SampleTime::ZERO,
                    })
                    .unwrap(),
            )
            .unwrap();
    }
    let mut block = [0.0; 192];
    audio
        .render(AudioBlockMut::new(&mut block, 192, ChannelLayout::Mono).unwrap())
        .unwrap();
    assert_eq!(audio.acknowledged().clock, SampleTime::new(128)); // Stop has not dispatched yet
    while let Some(completion) = audio.take_completed() {
        let _receipt = control.collect(completion).unwrap();
    }
    let old_fence = SessionSourceAction::Fence {
        source: sources[0],
        epoch,
        frontier: SampleTime::new(100),
    };
    let packet = control.prepare_source(old_fence).unwrap();
    let refused_id = packet.id();
    audio.enqueue(packet).unwrap();
    let refused = audio.take_completed().unwrap();
    assert_eq!(refused.id(), refused_id);
    assert!(matches!(
        control.collect(refused).unwrap().1,
        LoopTransferOutcome::Refused(LoopSessionError::Session(SessionError::SourceOrder))
    ));
    let mut block = [0.0; 64];
    audio
        .render(AudioBlockMut::new(&mut block, 64, ChannelLayout::Mono).unwrap())
        .unwrap();
    while let Some(completion) = audio.take_completed() {
        let _receipt = control.collect(completion).unwrap();
    }
    let packet = control.prepare_source(old_fence).unwrap();
    assert_ne!(packet.id(), refused_id);
    audio.enqueue(packet).unwrap();
    audio.drain_stopped_sources().unwrap();
    assert!(matches!(
        control.collect(audio.take_completed().unwrap()).unwrap().1,
        LoopTransferOutcome::Source(crate::host::session::SessionSourceReceipt {
            outcome: SessionSourceOutcome::Fenced,
            ..
        })
    ));
    // The retry supplied only 100; it must not pretend to fence the selected endpoint 128.
    let mut owner = audio.reunite(control).unwrap();
    assert!(owner.finalize().is_err());
    for source in sources {
        let _id = owner
            .offer_source(SessionSourceAction::Fence {
                source,
                epoch,
                frontier: SampleTime::new(512),
            })
            .unwrap();
    }
    owner.drain_stopped_sources().unwrap();
    while owner.collect_source().is_some() {}
    owner.finalize().unwrap();
    assert_eq!(owner.result().unwrap().window().end(), SampleTime::new(128));
}

#[test]
fn malformed_shape_is_retryable_but_terminal_render_failure_retains_outcomes_and_take() {
    let (session, sources) = setup();
    let fences: Vec<_> = sources
        .into_iter()
        .map(|s| fence_action(&session, s, 0))
        .collect();
    let (mut control, mut audio) = session.split(TRANSFER_BYTES).unwrap();
    audio
        .enqueue(
            control
                .prepare_command(SampleTime::ZERO, SessionCommand::Play)
                .unwrap(),
        )
        .unwrap();
    audio
        .enqueue(
            control
                .prepare_command(SampleTime::new(320), SessionCommand::Stop)
                .unwrap(),
        )
        .unwrap();
    for action in fences {
        audio
            .enqueue(control.prepare_source(action).unwrap())
            .unwrap();
    }
    let mut stereo = [8.0; 256];
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            assert!(
                audio
                    .render(AudioBlockMut::new(&mut stereo, 128, ChannelLayout::Stereo).unwrap())
                    .is_err()
            );
        }),
        0
    );
    assert_eq!(audio.acknowledged().clock, SampleTime::ZERO);
    assert!(audio.take_completed().is_none());
    let mut mono = [8.0; 128];
    audio
        .render(AudioBlockMut::new(&mut mono, 128, ChannelLayout::Mono).unwrap())
        .unwrap();
    let acknowledged = audio.acknowledged();
    let mut too_large = [8.0; 2049];
    assert_eq!(
        crate::render_allocation::count_allocs(|| {
            assert!(
                audio
                    .render(AudioBlockMut::new(&mut too_large, 2049, ChannelLayout::Mono).unwrap())
                    .is_err()
            );
        }),
        0
    );
    assert!(too_large.iter().all(|s| *s == 0.0));
    assert_eq!(audio.acknowledged(), acknowledged);
    let mut count = 0;
    while let Some(completion) = audio.take_completed() {
        let _receipt = control.collect(completion).unwrap();
        count += 1;
    }
    assert_eq!(count, 4);
    let mut owner = audio.reunite(control).unwrap();
    assert!(owner.finalize().is_err());
    for source in sources {
        owner.acknowledge_source_quiescence(source).unwrap();
    }
    owner.finalize().unwrap();
    assert_eq!(
        owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
}

#[test]
fn repeated_split_of_reunited_untouched_session_never_reuses_transfer_identity() {
    let (session, _) = setup();
    let (mut control, audio) = session.split(TRANSFER_BYTES).unwrap();
    let packet = control
        .prepare_command(SampleTime::ZERO, SessionCommand::Play)
        .unwrap();
    let previous = control.cancel(packet).unwrap();
    let session = audio.reunite(control).unwrap();
    let (mut control, audio) = session.split(TRANSFER_BYTES).unwrap();
    let packet = control
        .prepare_command(SampleTime::ZERO, SessionCommand::Play)
        .unwrap();
    let current = packet.id();
    assert_eq!(previous.serial(), current.serial());
    assert_ne!(previous.generation(), current.generation());
    assert_ne!(previous, current);
    let _id = control.cancel(packet).unwrap();
    let _session = audio.reunite(control).unwrap();
}
