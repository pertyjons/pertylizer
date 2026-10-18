//! Functional ownership rehearsal, not continuous scheduling or capacity evidence.
//! Logical render credits deliberately wait for source/control delivery. The callback
//! stays on its worker throughout; no callback or active owner returns for maintenance.
//! Falsifiers: changed audio/receipts, late/refused sound, lost owners, callback or
//! handoff heap activity, blocked rendering behind retirement, or a final extra render.
//! This tests a short, cross-thread lockstep schedule, not the sustained serial script.
use super::*;
use crate::frozen_capacity::FrozenHostConfig;
use std::{
    sync::{atomic::AtomicUsize, mpsc},
    thread,
    time::{Duration, Instant},
};
use synth_engine_v2::stream::MixedBridgeResetRetired;

const CALLBACKS: usize = 16;
const DEADLINE: Duration = Duration::from_secs(15);
type ResetOwner =
    MixedBridgeResetOwner<HostConsumer<MixedTransferCommand>, HostProducer<MixedTransferResult>>;
type ResetRetired =
    MixedBridgeResetRetired<HostConsumer<MixedTransferCommand>, HostProducer<MixedTransferResult>>;

enum Operation {
    Activation(MixedActivationOffer),
    Reset(ResetOwner),
}
enum Retirement {
    Activation(MixedActivationRetired),
    Reset(ResetRetired),
    RefusedActivation(MixedActivationRefusal, MixedActivationOffer),
    RefusedReset(MixedBridgeResetRefusal, ResetOwner),
}

/// Test-only logical clock and bounded failure escape, all outside render scopes.
#[derive(Default)]
struct Progress {
    permits: AtomicUsize,
    callbacks: AtomicUsize,
    operations: AtomicUsize,
    blocked_returns: AtomicUsize,
    stop: AtomicBool,
    audio_joined: AtomicBool,
    sources_joined: AtomicUsize,
}
/// Declared after Rig so unwind quiesces and joins every worker before the
/// control/raw halves drop. Returned audio allocations are destroyed on control.
struct Workers {
    progress: Arc<Progress>,
    sources: Vec<SourceWorker>,
    audio: Option<thread::JoinHandle<AudioReturn>>,
}
impl Workers {
    fn new(
        rig: &mut Rig,
        operations: HeapCons<Operation>,
        returns: HeapProd<Retirement>,
        progress: Arc<Progress>,
    ) -> Self {
        let callback = rig.callback.take().expect("sole handoff to audio worker");
        let audio_progress = Arc::clone(&progress);
        let mut workers = Self {
            progress,
            sources: Vec::new(),
            audio: None,
        };
        workers.audio = Some(thread::spawn(move || {
            audio_worker(callback, operations, returns, audio_progress)
        }));
        for source in [FIRST, SECOND] {
            workers.sources.push(SourceWorker::new(
                rig.producers[source].take().expect("source producer"),
                source,
            ));
        }
        workers
    }

    fn join(&mut self, rig: &mut Rig) -> AudioReturn {
        for source in [FIRST, SECOND] {
            let worker = self.sources.remove(0);
            drop(worker.request);
            rig.producers[source] = Some(worker.worker.join().expect("source joined"));
            self.progress.sources_joined.fetch_add(1, Ordering::Release);
        }
        self.progress.stop.store(true, Ordering::Release);
        let returned = self
            .audio
            .take()
            .expect("audio handle")
            .join()
            .expect("audio joined before control/raw teardown");
        self.progress.audio_joined.store(true, Ordering::Release);
        returned
    }
}
impl Drop for Workers {
    fn drop(&mut self) {
        for worker in self.sources.drain(..) {
            drop(worker.request);
            match worker.worker.join() {
                Ok(producer) => drop(producer),
                Err(reason) => eprintln!("source worker panicked during cleanup: {reason:?}"),
            }
            self.progress.sources_joined.fetch_add(1, Ordering::Release);
        }
        self.progress.stop.store(true, Ordering::Release);
        if let Some(audio) = self.audio.take() {
            match audio.join() {
                Ok(returned) => drop(returned),
                Err(reason) => eprintln!("audio worker panicked during cleanup: {reason:?}"),
            }
            self.progress.audio_joined.store(true, Ordering::Release);
        }
    }
}

fn wrapper(
    callback: &mut HostCallback,
) -> &mut MixedBridgeResetAudio<HostConsumer<MixedTransferCommand>, HostProducer<MixedTransferResult>>
{
    match callback.mixed.as_mut().expect("prepared mixed owner") {
        MixedHostAudio::Reset(wrapper) => wrapper,
        MixedHostAudio::Direct(..) => panic!("wrapper prepared before worker handoff"),
    }
}

struct AudioReturn {
    callback: HostCallback,
    pending: Option<Retirement>,
    samples: Vec<f32>,
    callbacks: usize,
}

fn audio_worker(
    mut callback: HostCallback,
    mut operations: HeapCons<Operation>,
    mut retirements: HeapProd<Retirement>,
    progress: Arc<Progress>,
) -> AudioReturn {
    // Test storage and the deadline clock are outside every callback/handoff scope.
    let channels = callback.mixed_layout.channels();
    let mut samples = vec![0.0; CALLBACKS * Q * channels];
    let deadline = Instant::now() + DEADLINE;
    let mut pending = None;
    let mut callbacks = 0;
    while !progress.stop.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "control failed to stop audio worker"
        );
        if pending.is_none()
            && let Some(operation) = callback_probe::without_heap(|| operations.try_pop())
        {
            pending = callback_probe::without_heap(|| match operation {
                Operation::Activation(offer) => wrapper(&mut callback)
                    .offer_activation(offer)
                    .err()
                    .map(|(reason, offer)| Retirement::RefusedActivation(reason, offer)),
                Operation::Reset(owner) => wrapper(&mut callback)
                    .offer_reset(owner, at(384))
                    .err()
                    .map(|(reason, owner)| Retirement::RefusedReset(reason, owner)),
            });
            progress.operations.fetch_add(1, Ordering::Release);
        }
        pending = callback_probe::without_heap(|| {
            let next = pending.take().or_else(|| {
                let audio = wrapper(&mut callback);
                audio
                    .take_retired_activation()
                    .map(Retirement::Activation)
                    .or_else(|| audio.take_retired().map(Retirement::Reset))
            });
            next.and_then(|retired| retirements.try_push(retired).err())
        });
        if pending.is_some() {
            progress.blocked_returns.fetch_add(1, Ordering::Release);
        }
        if callbacks < progress.permits.load(Ordering::Acquire) {
            assert!(callbacks < CALLBACKS, "bounded preallocated output");
            let start = callbacks * Q * channels;
            let rendered = callback_probe::without_heap(|| {
                callback.render(&mut samples[start..start + Q * channels])
            });
            assert_eq!(
                rendered,
                Rendered {
                    mixed: Ok(()),
                    raw_ok: true
                }
            );
            callbacks += 1;
            progress.callbacks.store(callbacks, Ordering::Release);
        } else {
            // The rehearsal clock is paced. No wait/yield belongs to render or handoff.
            thread::yield_now();
        }
    }
    // Even a full return mailbox never drops its retained owner on this thread.
    AudioReturn {
        callback,
        pending,
        samples,
        callbacks,
    }
}

fn phase(source: usize, index: usize) -> Vec<InputObservation> {
    match (source, index) {
        (FIRST, 0) => vec![message(100, note_on(60)), frontier(370)],
        (SECOND, 0) => vec![
            message(110, sustain(true)),
            message(120, note_on(64)),
            message(130, note_off(64)),
            frontier(370),
        ],
        (FIRST, 1) => vec![message(500, note_off(60)), frontier(610)],
        (SECOND, 1) => vec![message(510, sustain(false)), frontier(610)],
        (FIRST, 2) => vec![
            message(700, note_on(62)),
            message(800, note_off(62)),
            frontier(1_024),
        ],
        (SECOND, 2) => vec![frontier(1_024)],
        _ => panic!("three phases and two sources"),
    }
}

struct SourceWorker {
    request: mpsc::Sender<usize>,
    finished: mpsc::Receiver<usize>,
    worker: thread::JoinHandle<SourceProducer>,
}
impl SourceWorker {
    fn new(mut producer: SourceProducer, source: usize) -> Self {
        let (request, requests) = mpsc::channel();
        let (finished, completions) = mpsc::channel();
        let worker = thread::spawn(move || {
            while let Ok(index) = requests.recv_timeout(DEADLINE) {
                for observation in phase(source, index) {
                    // Each batch fits the ordinary ring; a refusal is a test failure,
                    // never a replacement attempt or a silent retry.
                    assert!(producer.submit_owned(observation).is_ok());
                }
                finished
                    .send(index)
                    .expect("control retains completion receiver");
            }
            producer
        });
        Self {
            request,
            finished: completions,
            worker,
        }
    }
}

fn require_healthy(rig: &Rig) {
    assert!(rig.errors.is_empty(), "{:?}", rig.errors);
    assert!(rig.raw_errors.is_empty(), "{:?}", rig.raw_errors);
    assert!(!rig.host_halt.is_requested());
    assert!(!rig.raw.halt.is_requested());
}

fn wait(rig: &mut Rig, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !ready() {
        assert!(Instant::now() < deadline, "bounded control wait expired");
        rig.service();
        require_healthy(rig);
        thread::yield_now();
    }
    rig.service();
    require_healthy(rig);
}

fn send_phase(rig: &mut Rig, workers: &[SourceWorker], index: usize) {
    let before = rig.admitted.len();
    let expected: usize = [FIRST, SECOND]
        .into_iter()
        .map(|source| phase(source, index).len())
        .sum();
    for worker in workers {
        worker.request.send(index).expect("source request");
    }
    let mut finished = [false; 2];
    wait(rig, || {
        for (source, worker) in workers.iter().enumerate() {
            if !finished[source]
                && let Ok(done) = worker.finished.try_recv()
            {
                assert_eq!(done, index);
                finished[source] = true;
            }
        }
        finished.iter().all(|done| *done)
    });
    // Control has custody of every batch before granting the logical render credit.
    // Each batch is smaller than the prepared stage/raw/command queues.
    rig.service();
    assert_eq!(rig.admitted.len() - before, expected);
    assert!(rig.raw_unsent.is_none());
}

fn drive(rig: &mut Rig, progress: &Progress, boundary: SampleTime) {
    let callbacks =
        usize::try_from(boundary.as_u64() / u64::from(QUANTUM_FRAMES)).expect("bounded clock") + 1;
    // Sound publication has a one-callback window. Raw readiness alone cannot
    // authorize several callbacks: pump and collect before each logical credit.
    while progress.callbacks.load(Ordering::Acquire) < callbacks {
        rig.service();
        let next = progress.callbacks.load(Ordering::Acquire) + 1;
        progress.permits.store(next, Ordering::Release);
        wait(rig, || progress.callbacks.load(Ordering::Acquire) == next);
    }
}

fn offer(
    rig: &mut Rig,
    operations: &mut HeapProd<Operation>,
    progress: &Progress,
    operation: Operation,
    count: usize,
) {
    assert!(
        operations.try_push(operation).is_ok(),
        "single operation mailbox is empty"
    );
    wait(rig, || progress.operations.load(Ordering::Acquire) == count);
}

fn prepare_reset(rig: &mut Rig, config: &FrozenHostConfig) -> (ResetOwner, ControlPort) {
    let (prepared, candidate) = saved_prepared_with_profile(at(384), config.profile);
    let MixedBridgeArmed {
        control,
        audio,
        control_port,
        audio_port,
    } = arm_prepared(
        &rig.raw,
        config.setup(),
        prepared,
        candidate,
        config.profile,
    );
    rig.control
        .seal_reset(control, at(384))
        .expect("sealed stopped owner");
    (
        MixedBridgeResetOwner::new(audio, audio_port).expect("successor port"),
        control_port,
    )
}

fn collect_reset(rig: &mut Rig, retired: ResetRetired, next: ControlPort) {
    let receipt = retired.receipt();
    assert_eq!(receipt.at(), at(384));
    assert_eq!(receipt.ended().count(), 2, "held and sustained live notes");
    let old = rig
        .control
        .commit_reset(receipt)
        .expect("matching reset receipt");
    rig.reset_old
        .push((old, std::mem::replace(&mut rig.control_port, next)));
    rig.collect();
    join_retired(&mut rig.reset_old, &mut rig.retired_resets, retired);
    let old = rig
        .retired_resets
        .last()
        .expect("old owner joined off audio");
    assert_eq!(
        old.bridge().transfer().custody(),
        MixedTransferCustody::Complete
    );
    assert_eq!(
        old.bridge().transfer().holds_outstanding(),
        EventCount::NONE
    );
    assert!(old.unclassified().is_empty() && old.unmatched().is_empty());
}

fn activation(rig: &Rig, boundary: SampleTime, kind: MixedActivationKind) -> MixedActivationOffer {
    rig.control
        .prepare_activation(boundary, PlanPosition::ZERO, boundary, kind)
        .expect("later activation prepares")
}

fn collect_activation_return(
    rig: &mut Rig,
    retired: MixedActivationRetired,
    boundary: SampleTime,
) -> MixedActivationReceipt {
    let receipt = rig
        .control
        .collect_activation(retired)
        .expect("activation collected off audio");
    assert!(
        matches!(receipt.outcome(), MixedActivationOutcome::Adopted(adopted)
        if adopted.effective() == boundary && !adopted.stopped())
    );
    assert!(!receipt.late());
    receipt
}

fn fresh(config: &FrozenHostConfig) -> Rig {
    let profile = config.profile;
    let (prepared, candidate) = saved_prepared_with_profile(at(64), profile);
    let mut rig = Rig::with_prepared(config.setup(), prepared, candidate, profile);
    rig.wrap_for_reset(at(192));
    rig
}

/// Serial oracle for the same stamped requests, Stop, reset, Start and Seek.
/// It does not use the thread protocol, credit clock or handoff mailboxes.
fn serial(config: &FrozenHostConfig) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let mut rig = fresh(config);
    rig.prime();
    let mut next = 0;
    for source in [FIRST, SECOND] {
        rig.send_all(source, &phase(source, 0));
    }
    serve_to(&mut rig, at(320), &[64], &mut next);
    let (owner, port) = prepare_reset(&mut rig, config);
    rig.reset_wrapper()
        .offer_reset(owner, at(384))
        .expect("serial reset");
    for source in [FIRST, SECOND] {
        rig.send_all(source, &phase(source, 1));
    }
    serve_to(&mut rig, at(576), &[64], &mut next);
    let retired = rig
        .reset_wrapper()
        .take_retired()
        .expect("serial fade complete");
    collect_reset(&mut rig, retired, port);
    let start = activation(&rig, at(640), MixedActivationKind::Start);
    rig.reset_wrapper()
        .offer_activation(start)
        .expect("serial Start");
    for source in [FIRST, SECOND] {
        rig.send_all(source, &phase(source, 2));
    }
    serve_to(&mut rig, at(704), &[64], &mut next);
    let retired = rig
        .reset_wrapper()
        .take_retired_activation()
        .expect("serial Start retired");
    let seek = activation(&rig, at(768), MixedActivationKind::Seek);
    let (reason, refused) = rig
        .reset_wrapper()
        .offer_activation(seek)
        .expect_err("stale control sequence");
    assert!(matches!(reason, MixedActivationRefusal::Superseded { .. }));
    assert_eq!(refused.effective(), at(768));
    drop(refused);
    let _start_receipt = collect_activation_return(&mut rig, retired, at(640));
    serve_to(&mut rig, at(960), &[64], &mut next);
    finish(rig)
}

fn concurrent(config: &FrozenHostConfig) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let mut rig = fresh(config);
    let progress = Arc::new(Progress::default());
    let (mut operations, operation_inbox) = HeapRb::new(1).split();
    let (returns, mut retirements) = HeapRb::new(1).split();
    let mut workers = Workers::new(&mut rig, operation_inbox, returns, Arc::clone(&progress));
    drive(&mut rig, &progress, at(0));
    send_phase(&mut rig, &workers.sources, 0);
    drive(&mut rig, &progress, at(320));
    let (owner, port) = prepare_reset(&mut rig, config);
    offer(
        &mut rig,
        &mut operations,
        &progress,
        Operation::Reset(owner),
        1,
    );
    send_phase(&mut rig, &workers.sources, 1);
    drive(&mut rig, &progress, at(576));
    let mut reset = None;
    wait(&mut rig, || {
        if reset.is_none() {
            reset = retirements.try_pop();
        }
        reset.is_some()
    });
    let Retirement::Reset(retired) = reset.expect("reset retirement") else {
        panic!("reset is the first retired owner");
    };
    collect_reset(&mut rig, retired, port);
    let start = activation(&rig, at(640), MixedActivationKind::Start);
    offer(
        &mut rig,
        &mut operations,
        &progress,
        Operation::Activation(start),
        2,
    );
    send_phase(&mut rig, &workers.sources, 2);
    drive(&mut rig, &progress, at(704));
    // Keep the first retired candidate in the one-place return mailbox. The second
    // refused offer must remain owned by audio while callbacks continue through 960.
    wait(&mut rig, || retirements.occupied_len() == 1);
    let seek = activation(&rig, at(768), MixedActivationKind::Seek);
    offer(
        &mut rig,
        &mut operations,
        &progress,
        Operation::Activation(seek),
        3,
    );
    wait(&mut rig, || {
        progress.blocked_returns.load(Ordering::Acquire) > 0
    });
    let before_blocked_render = progress.callbacks.load(Ordering::Acquire);
    drive(&mut rig, &progress, at(960));
    assert_eq!(
        progress.callbacks.load(Ordering::Acquire) - before_blocked_render,
        4,
        "four callbacks continued with the refused owner retained and return mailbox full"
    );
    // Producers quiesce first; audio stops at exactly the permitted callback count.
    let returned = workers.join(&mut rig);
    assert!(progress.audio_joined.load(Ordering::Acquire));
    assert_eq!(progress.sources_joined.load(Ordering::Acquire), 2);
    assert_eq!(returned.callbacks, CALLBACKS, "no extra final callback");
    assert!(operations.is_empty());
    assert_eq!(progress.operations.load(Ordering::Acquire), 3);
    let Retirement::Activation(first) = retirements.try_pop().expect("mailbox retained Start")
    else {
        panic!("expected Start retirement");
    };
    let start_receipt = collect_activation_return(&mut rig, first, at(640));
    match returned.pending.expect("audio retained refused Seek") {
        Retirement::RefusedActivation(reason, offer) => {
            assert_eq!(
                reason,
                MixedActivationRefusal::Superseded {
                    supersedes: offer.supersedes(),
                    in_force: start_receipt.sequence(),
                }
            );
            assert_eq!(offer.effective(), at(768));
            assert_eq!(offer.kind(), MixedActivationKind::Seek);
            drop(offer); // The returned owned candidate is destroyed on control.
        }
        Retirement::RefusedReset(reason, owner) => {
            // Preserve both fields until after audio join even on a failed test.
            drop(owner);
            panic!("unexpected reset refusal: {reason:?}");
        }
        _ => panic!("expected the exact refused Seek"),
    }
    assert!(retirements.is_empty());
    rig.callback = Some(returned.callback);
    rig.rendered = returned.samples;
    for range in [192..256, 384..448, 768..832] {
        assert!(
            rig.frame_samples(range).iter().any(|sample| *sample != 0.0),
            "live or compiled saved-project output unexpectedly silent"
        );
    }
    rig.collect();
    require_healthy(&rig);
    for (_, settled) in &rig.settled {
        if let Settled::Report(report) = settled {
            assert_eq!(report.nominal_at(), report.stamp(), "no peer deferral");
            assert_eq!(report.stamp(), report.executed_at(), "no late sound");
            match report.outcome() {
                MixedBridgeOutcome::NoteOn(result) => assert!(result.is_ok()),
                MixedBridgeOutcome::KeyUp { result, .. } => assert!(result.is_ok()),
                MixedBridgeOutcome::PedalDown { result, .. } => assert!(result.is_ok()),
                MixedBridgeOutcome::Lift { result, .. } => assert!(result.is_ok()),
                other => panic!("unexpected source outcome: {other:?}"),
            }
        }
    }
    assert_eq!(rig.messages().count(), 8);
    assert_eq!(rig.settled.len(), 8);
    finish(rig)
}

#[test]
fn concurrent_saved_host_matches_serial_sound_and_custody_with_blocked_retirement() {
    for hz in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        for layout in [ChannelLayout::Stereo, ChannelLayout::Mono] {
            let base = HostProfile::harness(
                synth_engine_v2::quantities::SampleRate::new(hz).expect("rate"),
                FrameCount::new(64),
                layout,
            )
            .expect("profile");
            let config =
                crate::frozen_capacity::reference_host_for_profile(base).expect("fixture profile");
            assert_eq!(concurrent(&config), serial(&config), "{hz} Hz {layout:?}");
        }
    }
}

#[test]
fn control_unwind_joins_workers_before_destroying_control_halves() {
    let config = crate::frozen_capacity::reference_host(
        synth_engine_v2::quantities::SampleRate::new(48_000.0).expect("rate"),
    )
    .expect("fixture");
    let mut rig = fresh(&config);
    let progress = Arc::new(Progress::default());
    let (_operations, operation_inbox) = HeapRb::new(1).split();
    let (returns, _retirements) = HeapRb::new(1).split();
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _workers = Workers::new(&mut rig, operation_inbox, returns, Arc::clone(&progress));
        panic!("injected control failure before render credit");
    }));
    assert!(failed.is_err());
    assert!(progress.audio_joined.load(Ordering::Acquire));
    assert_eq!(progress.sources_joined.load(Ordering::Acquire), 2);
    assert_eq!(progress.callbacks.load(Ordering::Acquire), 0);
    // Rig and its control/raw halves are still alive here, after all workers joined.
    require_healthy(&rig);
}

#[test]
fn raw_completion_backpressure_clears_without_a_final_callback() {
    let mut rig = Rig::new(Setup {
        raw_completion_ring: MixedTransferPlaces::new(1),
        ..Setup::default()
    });
    rig.prime();
    rig.send_all(
        FIRST,
        &[
            message(100, note_on(60)),
            message(120, note_off(60)),
            frontier(256),
        ],
    );
    rig.send(SECOND, frontier(256));
    rig.service();
    rig.render(256)
        .expect("one callback fills the completion ring");
    let frames = rig.output_frames();
    rig.collect();
    let captured_messages = |rig: &Rig| {
        rig.receipts
            .iter()
            .filter(|receipt| matches!(receipt.observation, InputObservation::Message { .. }))
            .count()
    };
    assert!(
        captured_messages(&rig) < 2,
        "one-place return ring must leave message completions pending on raw audio"
    );
    for _ in 0..8 {
        callback_probe::without_heap(|| {
            rig.callback
                .as_mut()
                .expect("callback")
                .service_between_callbacks()
        });
        rig.collect();
    }
    assert_eq!(captured_messages(&rig), 2);
    assert_eq!(
        rig.output_frames(),
        frames,
        "completion drain rendered no final callback"
    );
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(!joined.raw_halted && !joined.host_halted);
    assert!(joined.raw_errors.is_empty() && joined.errors.is_empty());
    for (_, state, _) in joined.ledger() {
        assert_eq!(state, RawState::Published(CaptureDisposition::Recorded));
    }
}
