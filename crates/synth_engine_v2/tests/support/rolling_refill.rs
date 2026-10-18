//! Single-arm rolling publication control, with explicit barriers BETWEEN bursts.
//! Within each burst the persistent worker renders autonomously. This is neither
//! independent callback cadence nor a reset, deadline or capacity qualification.
//! Falsifiers: changed serial audio/receipts, late commands, stale coverage accepted,
//! heap activity in callback/handoff, revival after a coverage fault, or lost custody.
use super::*;
use std::time::{Duration, Instant};

const BURSTS: usize = 4;
const OUTPUT_FRAMES: usize = BURSTS * 1_024;
const DEADLINE: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
struct BurstIndex(usize);
impl BurstIndex {
    fn new(value: usize) -> Self {
        assert!(value < BURSTS, "bounded rolling script");
        Self(value)
    }
    fn start(self) -> SampleTime {
        at(u64::try_from(self.0).expect("four bursts") * 1_024)
    }
    fn end(self) -> SampleTime {
        self.start()
            .checked_add(FrameCount::new(1_024))
            .expect("bounded script")
    }
}

/// The queue is created for exactly one arm, never reused across reset. The
/// sequence binds a request to a burst; the cursor binds its coverage to output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BurstRequest {
    index: BurstIndex,
    at: SampleTime,
    /// Sound boundary, checked against raw's acknowledged clock. The shared-carry
    /// control in the parent module establishes their equality for this fixed arm.
    from: SampleTime,
    coverage: Option<PrefixCoverage>,
}
impl BurstRequest {
    fn captured(rig: &mut Rig, profile: HostProfile, index: BurstIndex) -> Self {
        assert!(!rig.host_halt.is_requested());
        let waits = rig.waits.len();
        service_prefix_control(rig);
        assert!(rig.errors.is_empty() && rig.raw_errors.is_empty());
        assert_eq!(rig.waits.len(), waits + 1, "fresh successful pump wait");
        Self {
            index,
            at: index.start(),
            from: rig.control.audio_boundary(),
            coverage: Some(PrefixCoverage::capture_at_boundary(
                rig,
                profile,
                index.end(),
            )),
        }
    }

    fn apply(self, audio: &mut PrefixCallback, next: BurstIndex) -> bool {
        if self.index != next
            || self.at != audio.output_at
            || self.from != audio.callback.raw.acknowledged().clock
        {
            return false;
        }
        if let Some(coverage) = self.coverage {
            let old = audio.coverage;
            if coverage.sound < old.sound
                || coverage.raw < old.raw
                || coverage
                    .sources
                    .iter()
                    .zip(old.sources)
                    .any(|(new, old)| new.generation != old.generation || new.through < old.through)
            {
                return false;
            }
            // Replace the complete snapshot. Never merge per-component maxima.
            audio.coverage = coverage;
        }
        true
    }
}

#[derive(Default)]
struct Progress {
    completed: AtomicUsize,
    rejected: AtomicUsize,
    stop: AtomicBool,
    joined: AtomicBool,
}
struct Returned {
    audio: PrefixCallback,
    samples: Vec<f32>,
    outcomes: Vec<CheckedOutcome>,
    bursts: usize,
}
/// Declare after Rig, so unwinding joins before control/raw custody is dropped.
struct Worker {
    requests: HeapProd<BurstRequest>,
    progress: Arc<Progress>,
    worker: Option<thread::JoinHandle<Returned>>,
}
impl Worker {
    fn spawn(audio: PrefixCallback, pattern: &[usize]) -> Self {
        let (requests, incoming) = HeapRb::new(1).split();
        Self::with_queue(audio, partitions(pattern), requests, incoming)
    }
    fn with_queue(
        mut audio: PrefixCallback,
        parts: Vec<usize>,
        requests: HeapProd<BurstRequest>,
        mut incoming: HeapCons<BurstRequest>,
    ) -> Self {
        let progress = Arc::new(Progress::default());
        let shared = Arc::clone(&progress);
        let worker = thread::spawn(move || {
            let channels = audio.callback.mixed_layout.channels();
            let mut samples = vec![0.25; OUTPUT_FRAMES * channels];
            let mut outcomes = Vec::with_capacity(BURSTS * parts.len());
            let mut bursts = 0;
            let mut offset = 0;
            let deadline = Instant::now() + DEADLINE;
            while !shared.stop.load(Ordering::Acquire) {
                assert!(
                    Instant::now() < deadline,
                    "control did not join rolling worker"
                );
                // Explicit test barrier outside callbacks; no hardware cadence claim.
                let Some(request) = callback_probe::without_heap(|| incoming.try_pop()) else {
                    thread::yield_now();
                    continue;
                };
                if bursts == BURSTS
                    || !callback_probe::without_heap(|| {
                        request.apply(&mut audio, BurstIndex::new(bursts))
                    })
                {
                    shared.rejected.fetch_add(1, Ordering::Release);
                    continue;
                }
                for frames in &parts {
                    let end = offset + frames * channels;
                    let outcome =
                        callback_probe::without_heap(|| audio.render(&mut samples[offset..end]));
                    outcomes.push(outcome);
                    offset = end;
                }
                bursts += 1;
                shared.completed.store(bursts, Ordering::Release);
            }
            samples.truncate(offset);
            Returned {
                audio,
                samples,
                outcomes,
                bursts,
            }
        });
        Self {
            requests,
            progress,
            worker: Some(worker),
        }
    }
    fn send(&mut self, request: BurstRequest) {
        assert!(
            self.requests.try_push(request).is_ok(),
            "one free mailbox place"
        );
    }
    fn wait_completed(&self, count: usize) {
        wait_until(|| self.progress.completed.load(Ordering::Acquire) == count);
    }
    fn wait_rejected(&self, count: usize) {
        wait_until(|| self.progress.rejected.load(Ordering::Acquire) == count);
    }
    fn join(mut self) -> Returned {
        self.progress.stop.store(true, Ordering::Release);
        let returned = self
            .worker
            .take()
            .expect("audio handle")
            .join()
            .expect("rolling worker");
        self.progress.joined.store(true, Ordering::Release);
        returned
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.progress.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            match worker.join() {
                Ok(returned) => drop(returned),
                Err(reason) => eprintln!("rolling worker failed during cleanup: {reason:?}"),
            }
            self.progress.joined.store(true, Ordering::Release);
        }
    }
}
fn wait_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !condition() {
        assert!(Instant::now() < deadline, "rolling control timed out");
        thread::yield_now();
    }
}

fn batch(source: usize, index: BurstIndex) -> Vec<InputObservation> {
    let offset = index.start();
    script(source)
        .into_iter()
        .map(|observation| match observation {
            InputObservation::Message {
                tick,
                arrival,
                input,
            } => InputObservation::Message {
                // These fixture clocks are explicitly one frame per synthetic tick.
                tick: InputTick::new(tick.as_u64() + offset.as_u64()),
                arrival: arrival
                    .checked_add(FrameCount::new(offset.as_u64()))
                    .expect("script time"),
                input,
            },
            InputObservation::Frontier { tick } => InputObservation::Frontier {
                tick: InputTick::new(tick.as_u64() + offset.as_u64()),
            },
        })
        .collect()
}
fn stage_batch(rig: &mut Rig, index: BurstIndex) {
    for source in [FIRST, SECOND] {
        let observations = batch(source, index);
        for observation in &observations {
            if let InputObservation::Message { tick, arrival, .. } = observation {
                assert_eq!(rig.raw.clocks[source].map(*tick), Ok(*arrival));
            }
        }
        rig.send_all(source, &observations);
    }
    service_prefix_control(rig);
    assert!(rig.errors.is_empty() && rig.raw_errors.is_empty());
}

fn healthy(mut rig: Rig) -> (Vec<u32>, Vec<String>, Vec<String>) {
    rig.collect();
    assert_eq!(rig.messages().count(), BURSTS * SCRIPT_MESSAGES);
    assert_eq!(rig.settled.len(), BURSTS * SCRIPT_MESSAGES);
    for (_, settled) in &rig.settled {
        let Settled::Report(report) = settled else {
            panic!("rolling command did not execute: {settled:?}");
        };
        sustained_host::assert_applied_sound_outcome(*report);
    }
    assert!(rig.rendered.iter().any(|sample| *sample != 0.0));
    finish(rig)
}
fn serial(profile: HostProfile) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let mut rig = fresh(profile);
    for burst in 0..BURSTS {
        stage_batch(&mut rig, BurstIndex::new(burst));
        for _ in 0..16 {
            rig.render(64).expect("unguarded serial rolling oracle");
        }
    }
    healthy(rig)
}
fn rolling(profile: HostProfile, pattern: &[usize]) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let mut rig = fresh(profile);
    stage_batch(&mut rig, BurstIndex::new(0));
    let initial = BurstRequest::captured(&mut rig, profile, BurstIndex::new(0));
    let audio = checked(&mut rig, profile);
    let mut worker = Worker::spawn(audio, pattern);
    for burst in 0..BURSTS {
        let index = BurstIndex::new(burst);
        if burst > 0 {
            stage_batch(&mut rig, index);
            // Replaying the old cursor/sequence must not start an extra burst.
            worker.send(initial);
            worker.wait_rejected(burst);
            assert_eq!(worker.progress.completed.load(Ordering::Acquire), burst);
        }
        let request = BurstRequest::captured(&mut rig, profile, index);
        let coverage = request.coverage.expect("captured");
        let boundary = rig.control.audio_boundary();
        assert_eq!(boundary, at(index.start().as_u64().saturating_sub(64)));
        assert_eq!(
            coverage.sound,
            boundary
                .checked_add(FrameCount::new(1_024))
                .expect("window")
                .min(index.end())
        );
        assert_eq!(coverage.raw, index.end());
        assert!(
            coverage
                .sources
                .iter()
                .all(|source| source.through == index.end())
        );
        worker.send(request);
        worker.wait_completed(burst + 1);
        // No service or callback permits were issued inside the whole burst.
        rig.collect();
        assert_eq!(rig.settled.len(), (burst + 1) * SCRIPT_MESSAGES);
    }
    let returned = worker.join();
    assert_eq!(returned.bursts, BURSTS);
    assert_eq!(returned.outcomes.len(), BURSTS * partitions(pattern).len());
    for outcome in returned.outcomes {
        require_rendered(outcome);
    }
    assert_eq!(returned.audio.output_at, BurstIndex::new(BURSTS - 1).end());
    assert_eq!(returned.audio.fault, None);
    rig.callback = Some(returned.audio.callback);
    rig.rendered = returned.samples;
    healthy(rig)
}

#[test]
fn four_rolling_windows_match_serial_at_all_rates_layouts_and_partitions() {
    for (rate, layout) in candidates() {
        let profile = profile(rate, layout, FrameCount::new(1_024));
        let oracle = serial(profile);
        for pattern in [&[64][..], &[7, 13, 1, 43][..], &[1_024][..]] {
            assert_eq!(
                rolling(profile, pattern),
                oracle,
                "{rate:?} {layout:?} {pattern:?}"
            );
        }
    }
}

#[test]
fn missing_second_refill_latches_silence_and_late_publication_cannot_revive() {
    for (rate, layout) in candidates() {
        let profile = profile(rate, layout, FrameCount::new(1_024));
        let mut rig = fresh(profile);
        stage_batch(&mut rig, BurstIndex::new(0));
        let first = BurstRequest::captured(&mut rig, profile, BurstIndex::new(0));
        let audio = checked(&mut rig, profile);
        let mut worker = Worker::spawn(audio, &[64]);
        worker.send(first);
        worker.wait_completed(1);
        // The second scheduled burst proceeds without any source/control refill.
        worker.send(BurstRequest {
            index: BurstIndex::new(1),
            at: at(1_024),
            from: rig.control.audio_boundary(),
            coverage: None,
        });
        worker.wait_completed(2);
        let boundary = rig.control.audio_boundary();
        assert_eq!(boundary, at(1_024));
        let admitted = rig.admitted.len();
        let forwarded = rig.forwarded.len();
        let late = batch(FIRST, BurstIndex::new(2));
        rig.send_all(FIRST, &late);
        service_prefix_control(&mut rig);
        assert_eq!(rig.admitted.len(), admitted);
        assert_eq!(rig.forwarded.len(), forwarded);
        assert_eq!(rig.halted_sends.len(), late.len());
        let mut returned = worker.join();
        let fault = returned.audio.fault.expect("first missing prefix");
        assert_eq!(fault.at, at(1_088));
        assert!(
            matches!(fault.missing, MissingPrefix::Source { through, required, .. } if through == at(1_024) && required == at(1_088))
        );
        for outcome in &returned.outcomes[..17] {
            require_rendered(*outcome);
        }
        assert!(
            returned.outcomes[17..]
                .iter()
                .all(|outcome| *outcome == CheckedOutcome::Refused(fault))
        );
        assert!(silent(&returned.samples[1_088 * layout.channels()..]));
        // Even a later complete-looking snapshot cannot clear the latched guard.
        returned.audio.coverage = PrefixCoverage {
            sound: BurstIndex::new(BURSTS - 1).end(),
            raw: BurstIndex::new(BURSTS - 1).end(),
            sources: returned.audio.coverage.sources.map(|source| SourcePrefix {
                through: BurstIndex::new(BURSTS - 1).end(),
                ..source
            }),
        };
        let mut retry = vec![0.25; Q * layout.channels()];
        assert_eq!(
            callback_probe::without_heap(|| returned.audio.render(&mut retry)),
            CheckedOutcome::Refused(fault)
        );
        assert!(silent(&retry));
        assert_eq!(rig.control.audio_boundary(), boundary);
        rig.callback = Some(returned.audio.callback);
        rig.rendered = returned.samples;
        let joined = rig.join();
        joined.assert_each_packet_once();
        assert_eq!(
            joined.rendered.len() / layout.channels(),
            2 * 1_024,
            "no final render"
        );
        assert!(joined.host_halted && joined.raw_halted);
        assert_eq!(joined.teardown.fault(), None);
        assert_eq!(
            joined.teardown.transfer().custody(),
            MixedTransferCustody::Complete
        );
        assert!(joined.teardown.held().is_empty() && joined.teardown.open().is_empty());
        assert_eq!(joined.settled.len(), SCRIPT_MESSAGES);
        assert!(joined.unexamined.is_empty() && joined.retired.is_empty());
    }
}

#[test]
fn pending_sound_heads_need_a_fresh_nonzero_publication_window() {
    for (rate, layout) in candidates() {
        let profile = profile(rate, layout, FrameCount::new(1_024));
        let prepared = || {
            let mut rig = fresh(profile);
            rig.send_all(
                FIRST,
                &[
                    message(1_983, note_on(60)),
                    message(1_984, note_off(60)),
                    frontier(3_072),
                ],
            );
            rig.send(SECOND, frontier(3_072));
            rig.service();
            assert_eq!(
                rig.waits.last(),
                Some(&MixedBridgeWait::Window {
                    stamp: at(1_983),
                    end: at(1_024)
                })
            );
            rig
        };
        let mut reference = prepared();
        for _ in 0..16 {
            reference.render(64).expect("first serial window");
        }
        reference.service();
        for _ in 0..16 {
            reference.render(64).expect("second serial window");
        }
        // Onset is at the second window's end - 1; key-up is exactly at its end.
        assert_eq!(
            reference.waits.last(),
            Some(&MixedBridgeWait::Window {
                stamp: at(1_984),
                end: at(1_984)
            })
        );
        reference.service();
        for _ in 0..16 {
            reference.render(64).expect("third serial window");
        }
        reference.collect();
        let oracle = finish(reference);
        for refill in [true, false] {
            let mut rig = prepared();
            let first = BurstRequest::captured(&mut rig, profile, BurstIndex::new(0));
            let audio = checked(&mut rig, profile);
            let mut worker = Worker::spawn(audio, &[64]);
            worker.send(first);
            worker.wait_completed(1);
            assert_eq!(rig.control.audio_boundary(), at(960));
            let request = if refill {
                service_prefix_control(&mut rig);
                let captured = BurstRequest::captured(&mut rig, profile, BurstIndex::new(1));
                let coverage = captured.coverage.expect("refill");
                assert_eq!(coverage.sound, at(1_984));
                assert_eq!(coverage.raw, at(3_072));
                captured
            } else {
                BurstRequest {
                    index: BurstIndex::new(1),
                    at: at(1_024),
                    from: rig.control.audio_boundary(),
                    coverage: None,
                }
            };
            worker.send(request);
            worker.wait_completed(2);
            if refill {
                assert_eq!(
                    rig.waits.last(),
                    Some(&MixedBridgeWait::Window {
                        stamp: at(1_984),
                        end: at(1_984)
                    })
                );
                let third = BurstRequest::captured(&mut rig, profile, BurstIndex::new(2));
                assert_eq!(third.from, at(1_984));
                worker.send(third);
                worker.wait_completed(3);
            }
            let returned = worker.join();
            let fault = returned.audio.fault;
            if refill {
                assert_eq!(fault, None);
                for outcome in returned.outcomes {
                    require_rendered(outcome);
                }
                rig.callback = Some(returned.audio.callback);
                rig.rendered = returned.samples;
                rig.collect();
                assert_eq!(rig.settled.len(), 2);
                for (_, settled) in &rig.settled {
                    let Settled::Report(report) = settled else {
                        panic!("unexecuted: {settled:?}");
                    };
                    sustained_host::assert_applied_sound_outcome(*report);
                }
                assert_eq!(finish(rig), oracle);
            } else {
                let expected = PrefixFault {
                    at: at(1_088),
                    missing: MissingPrefix::Sound {
                        through: at(1_024),
                        required: at(1_088),
                    },
                };
                assert_eq!(fault, Some(expected));
                for outcome in &returned.outcomes[..17] {
                    require_rendered(*outcome);
                }
                assert!(
                    returned.outcomes[17..]
                        .iter()
                        .all(|outcome| *outcome == CheckedOutcome::Refused(expected))
                );
                assert!(silent(&returned.samples[1_088 * layout.channels()..]));
                rig.callback = Some(returned.audio.callback);
                rig.rendered = returned.samples;
                service_prefix_control(&mut rig);
                assert!(rig.forwarded.is_empty(), "halt blocks staged refill");
                let joined = rig.join();
                joined.assert_each_packet_once();
                assert_eq!(joined.teardown.fault(), None);
                assert_eq!(
                    joined.teardown.transfer().custody(),
                    MixedTransferCustody::Complete
                );
                assert_eq!(joined.teardown.held().len(), 2);
                assert!(
                    joined
                        .teardown
                        .held()
                        .iter()
                        .all(|(_, place)| *place == MixedSourceCustody::Staged)
                );
                assert!(joined.unexamined.is_empty() && joined.retired.is_empty());
                assert_eq!(joined.rendered.len() / layout.channels(), 2 * 1_024);
            }
        }
    }
}

#[test]
fn one_place_request_mailbox_does_not_overwrite_and_stale_retry_does_not_render() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = fresh(profile);
    stage_batch(&mut rig, BurstIndex::new(0));
    let first = BurstRequest::captured(&mut rig, profile, BurstIndex::new(0));
    let (mut requests, incoming) = HeapRb::new(1).split();
    assert!(requests.try_push(first).is_ok());
    let second = BurstRequest {
        index: BurstIndex::new(1),
        ..first
    };
    let retained = requests
        .try_push(second)
        .expect_err("mailbox full before worker starts");
    assert_eq!(retained, second);
    let audio = checked(&mut rig, profile);
    let mut worker = Worker::with_queue(audio, partitions(&[64]), requests, incoming);
    worker.wait_completed(1);
    worker.send(retained);
    worker.wait_rejected(1);
    assert_eq!(worker.progress.completed.load(Ordering::Acquire), 1);
    let returned = worker.join();
    assert_eq!(returned.audio.output_at, PREFIX_END);
    rig.callback = Some(returned.audio.callback);
    rig.rendered = returned.samples;
    healthy_result(rig);
}

#[test]
fn same_sequence_rejects_wrong_cursor_regression_and_foreign_source_generation() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = fresh(profile);
    stage_batch(&mut rig, BurstIndex::new(0));
    let valid = BurstRequest::captured(&mut rig, profile, BurstIndex::new(0));
    let mut audio = checked(&mut rig, profile);
    let old = audio.coverage;
    let mut wrong_cursor = valid;
    wrong_cursor.at = at(64);
    let mut wrong_boundary = valid;
    wrong_boundary.from = at(64);
    let mut future = valid;
    future.index = BurstIndex::new(1);
    let mut sound = valid;
    sound.coverage.as_mut().expect("coverage").sound = at(0);
    let mut raw = valid;
    raw.coverage.as_mut().expect("coverage").raw = at(0);
    let mut source = valid;
    source.coverage.as_mut().expect("coverage").sources[FIRST].through = at(0);
    let mut generation = valid;
    generation
        .coverage
        .as_mut()
        .expect("coverage")
        .sources
        .swap(FIRST, SECOND);
    for request in [
        wrong_cursor,
        wrong_boundary,
        future,
        sound,
        raw,
        source,
        generation,
    ] {
        assert!(!callback_probe::without_heap(
            || request.apply(&mut audio, BurstIndex::new(0))
        ));
        assert_eq!(audio.coverage, old);
        assert_eq!(audio.output_at, SampleTime::ZERO);
    }
    assert!(callback_probe::without_heap(
        || valid.apply(&mut audio, BurstIndex::new(0))
    ));
    rig.callback = Some(audio.callback);
    rig.join().assert_each_packet_once();
}

/// Observe join status immediately BEFORE this owner's Rig fields are dropped,
/// rather than observing it only after the entire unwind has already finished.
struct ControlDropProbe {
    rig: Rig,
    progress: Option<Arc<Progress>>,
    joined_before_control_drop: Arc<AtomicBool>,
}
impl Drop for ControlDropProbe {
    fn drop(&mut self) {
        self.joined_before_control_drop.store(
            self.progress
                .as_ref()
                .is_some_and(|progress| progress.joined.load(Ordering::Acquire)),
            Ordering::Release,
        );
    }
}

#[test]
fn control_unwind_joins_a_worker_waiting_for_next_refill_before_control_drop() {
    let mut observed = None;
    let joined_before_control_drop = Arc::new(AtomicBool::new(false));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let profile = profile(
            SampleRate::new(48_000.0).expect("rate"),
            ChannelLayout::Stereo,
            FrameCount::new(1_024),
        );
        let mut control = ControlDropProbe {
            rig: fresh(profile),
            progress: None,
            joined_before_control_drop: Arc::clone(&joined_before_control_drop),
        };
        stage_batch(&mut control.rig, BurstIndex::new(0));
        let request = BurstRequest::captured(&mut control.rig, profile, BurstIndex::new(0));
        let audio = checked(&mut control.rig, profile);
        let mut worker = Worker::spawn(audio, &[64]);
        control.progress = Some(Arc::clone(&worker.progress));
        observed = Some(Arc::clone(&worker.progress));
        worker.send(request);
        worker.wait_completed(1);
        panic!("control unwind between rolling bursts");
    }));
    assert!(result.is_err());
    assert!(
        observed
            .expect("worker started")
            .joined
            .load(Ordering::Acquire)
    );
    assert!(joined_before_control_drop.load(Ordering::Acquire));
}
