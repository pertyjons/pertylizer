//! Independent software callback cadence, not physical timing or capacity evidence.
//! One startup barrier; thereafter neither sources nor control grant render permits.
//! Falsifiers: audio/receipt differences, late healthy commands, callback heap work,
//! stale-arm acceptance, revival after control loss, or incomplete joined custody.
use super::*;
use std::time::{Duration, Instant};
use synth_engine_v2::host::mixed_input::MixedRawPublication;
use synth_engine_v2::stream::{
    MixedPublication, MixedPublicationFault, MixedPublicationGate, MixedPublicationReceiver,
    MixedPublicationRefusal,
};

const CHUNKS: usize = 32;
const CHUNK: u64 = 512;
const TOTAL: usize = CHUNKS * 512;
const RESET: SampleTime = SampleTime::new(4_096);
const STOP: SampleTime = SampleTime::new(3_584);
const SEEK: SampleTime = SampleTime::new(5_760);
const DEADLINE: Duration = Duration::from_secs(15);
// Deliberately independent of device rate: this clock tests causality, not deadlines.
const MICROS_PER_FRAME: u64 = 30;
type ResetOwner =
    MixedBridgeResetOwner<HostConsumer<MixedTransferCommand>, HostProducer<MixedTransferResult>>;
type ResetRetired = synth_engine_v2::stream::MixedBridgeResetRetired<
    HostConsumer<MixedTransferCommand>,
    HostProducer<MixedTransferResult>,
>;

#[derive(Default)]
struct Progress {
    start: AtomicBool,
    stop: AtomicBool,
    source_stop: AtomicBool,
    output: AtomicUsize,
    done: AtomicBool,
    joined: AtomicBool,
    sources_joined: AtomicUsize,
    blocked_returns: AtomicUsize,
}
enum Operation {
    Reset(ResetOwner),
    Activation(MixedActivationOffer),
}
enum Retirement {
    Reset(ResetRetired),
    Activation(MixedActivationRetired),
    RefusedActivation(MixedActivationRefusal, MixedActivationOffer),
    RefusedReset(MixedBridgeResetRefusal, ResetOwner),
}
struct SourceReturn {
    producer: SourceProducer,
    refused: Option<SourceOwnedSendError>,
}
struct AudioReturn {
    publication_reader: MixedPublicationReceiver,
    callback: HostCallback,
    gate: MixedPublicationGate,
    samples: Vec<f32>,
    outcomes: Vec<Result<Rendered, MixedPublicationFault>>,
    pending: Option<Retirement>,
    rejected: usize,
}
struct Workers {
    progress: Arc<Progress>,
    sources: Vec<thread::JoinHandle<SourceReturn>>,
    audio: Option<thread::JoinHandle<AudioReturn>>,
}
impl Drop for Workers {
    fn drop(&mut self) {
        self.progress.source_stop.store(true, Ordering::Release);
        for source in self.sources.drain(..) {
            match source.join() {
                Ok(returned) => drop(returned),
                Err(reason) => eprintln!("continuous source cleanup: {reason:?}"),
            }
            self.progress.sources_joined.fetch_add(1, Ordering::Release);
        }
        self.progress.stop.store(true, Ordering::Release);
        if let Some(audio) = self.audio.take() {
            match audio.join() {
                Ok(returned) => drop(returned),
                Err(reason) => eprintln!("continuous audio cleanup: {reason:?}"),
            }
            self.progress.joined.store(true, Ordering::Release);
        }
    }
}
impl Workers {
    fn new(
        rig: &mut Rig,
        pattern: &[usize],
        ready: MixedPublicationReceiver,
        operations: HeapCons<Operation>,
        returns: HeapProd<Retirement>,
    ) -> Self {
        let progress = Arc::new(Progress::default());
        let mut workers = Self {
            progress,
            sources: Vec::new(),
            audio: None,
        };
        for source in [FIRST, SECOND] {
            let producer = rig.producers[source]
                .take()
                .expect("one producer per worker");
            let shared = Arc::clone(&workers.progress);
            workers.sources.push(thread::spawn(move || {
                source_worker(producer, source, shared)
            }));
        }
        let callback = rig.callback.take().expect("sole callback handoff");
        let gate = rig.control.publication_gate();
        let shared = Arc::clone(&workers.progress);
        let parts = callback_parts(pattern);
        workers.audio = Some(thread::spawn(move || {
            audio_worker(callback, gate, parts, ready, operations, returns, shared)
        }));
        workers
    }
    fn join(mut self, rig: &mut Rig) -> (AudioReturn, Vec<SourceOwnedSendError>) {
        let mut refusals = Vec::new();
        self.progress.source_stop.store(true, Ordering::Release);
        for source in [FIRST, SECOND] {
            let returned = self.sources.remove(0).join().expect("source quiescent");
            if let Some(refused) = returned.refused {
                refusals.push(refused);
            }
            rig.producers[source] = Some(returned.producer);
            self.progress.sources_joined.fetch_add(1, Ordering::Release);
        }
        let returned = self
            .audio
            .take()
            .expect("audio handle")
            .join()
            .expect("audio quiescent");
        self.progress.joined.store(true, Ordering::Release);
        (returned, refusals)
    }
}

fn chunk(source: usize, index: usize) -> Vec<InputObservation> {
    let base = u64::try_from(index).expect("bounded source script") * CHUNK;
    let (key, delta) = if source == FIRST { (60, 0) } else { (64, 10) };
    let mut observations = Vec::new();
    // Two long occurrences per source stay inside this fixture's four live slots.
    // Repeated bends exercise rolling publication without assuming release-tail reuse.
    if index == 0 || index == 9 {
        observations.push(message(base + 80 + delta, note_on(key)));
    }
    if index != 8 {
        observations.push(message(
            base + 128 + delta,
            bend(8_200 + u16::try_from(index).expect("bounded bend")),
        ));
    }
    if index == 8 || index == CHUNKS - 1 {
        observations.push(message(base + 384 + delta, note_off(key)));
    }
    observations.push(frontier(base + CHUNK));
    observations
}
fn source_worker(
    mut producer: SourceProducer,
    source: usize,
    progress: Arc<Progress>,
) -> SourceReturn {
    let deadline = Instant::now() + DEADLINE;
    for index in 0..CHUNKS {
        while u64::try_from(progress.output.load(Ordering::Acquire)).expect("bounded output")
            + 1_024
            <= u64::try_from(index).expect("chunk") * CHUNK
        {
            if progress.source_stop.load(Ordering::Acquire) {
                return SourceReturn {
                    producer,
                    refused: None,
                };
            }
            assert!(Instant::now() < deadline, "source worker timeout");
            thread::sleep(Duration::from_micros(100));
        }
        for observation in chunk(source, index) {
            if progress.source_stop.load(Ordering::Acquire) {
                return SourceReturn {
                    producer,
                    refused: None,
                };
            }
            if let Err(refused) = producer.submit_owned(observation) {
                return SourceReturn {
                    producer,
                    refused: Some(refused),
                };
            }
        }
    }
    SourceReturn {
        producer,
        refused: None,
    }
}
fn callback_parts(pattern: &[usize]) -> Vec<usize> {
    assert!(!pattern.is_empty() && pattern.iter().all(|frames| *frames > 0 && *frames <= 1_024));
    let mut parts = Vec::new();
    let mut done = 0;
    while done < TOTAL {
        let frames = pattern[parts.len() % pattern.len()].min(TOTAL - done);
        parts.push(frames);
        done += frames;
    }
    parts
}
fn wrapper(
    callback: &mut HostCallback,
) -> &mut MixedBridgeResetAudio<HostConsumer<MixedTransferCommand>, HostProducer<MixedTransferResult>>
{
    match callback.mixed.as_mut().expect("mixed callback") {
        MixedHostAudio::Reset(wrapper) => wrapper,
        MixedHostAudio::Direct(..) => panic!("reset wrapper required"),
    }
}
fn take_retirement(callback: &mut HostCallback) -> Option<Retirement> {
    if let Some(MixedHostAudio::Reset(wrapper)) = callback.mixed.as_mut() {
        wrapper
            .take_retired_activation()
            .map(Retirement::Activation)
            .or_else(|| wrapper.take_retired().map(Retirement::Reset))
    } else {
        None
    }
}
fn check(
    callback: &HostCallback,
    gate: &mut MixedPublicationGate,
    frames: FrameCount,
) -> Result<(), MixedPublicationFault> {
    match callback.mixed.as_ref().expect("mixed owner") {
        MixedHostAudio::Direct(audio, _) => audio.check_publication(gate, frames),
        MixedHostAudio::Reset(wrapper) => wrapper.check_publication(gate, frames),
    }
}
fn update(
    callback: &HostCallback,
    gate: &mut MixedPublicationGate,
    publication: MixedPublication,
    raw: MixedRawPublication,
) -> Result<(), MixedPublicationRefusal> {
    match callback.mixed.as_ref().expect("mixed owner") {
        MixedHostAudio::Direct(audio, _) => audio.offer_publication(gate, publication, raw),
        MixedHostAudio::Reset(wrapper) => wrapper.offer_publication(gate, publication, raw),
    }
}
/// Validate caller-owned output before consuming raw packets or touching the
/// sticky source guard. Shape refusals remain retryable on the same owners.
fn render_guarded(
    callback: &mut HostCallback,
    gate: &mut MixedPublicationGate,
    samples: &mut [f32],
) -> Result<Rendered, MixedPublicationFault> {
    let channels = callback.mixed_layout.channels();
    let frames = u64::try_from(samples.len() / channels)
        .ok()
        .map(FrameCount::new);
    let Some(frames) = frames.filter(|frames| {
        samples.len().is_multiple_of(channels) && *frames <= callback.maximum_block
    }) else {
        samples.fill(0.0);
        return Ok(Rendered {
            mixed: Err(MixedHostRenderError::Host(
                MixedTransferRenderError::OutputShape,
            )),
            raw_ok: false,
        });
    };
    match check(callback, gate, frames) {
        Ok(()) => Ok(callback.render(samples)),
        Err(fault) => {
            callback.host_halt.request();
            callback.raw_halt.request_invalid();
            samples.fill(0.0);
            Err(fault)
        }
    }
}
fn audio_worker(
    mut callback: HostCallback,
    mut gate: MixedPublicationGate,
    parts: Vec<usize>,
    mut ready: MixedPublicationReceiver,
    mut operations: HeapCons<Operation>,
    mut returns: HeapProd<Retirement>,
    progress: Arc<Progress>,
) -> AudioReturn {
    let channels = callback.mixed_layout.channels();
    let mut samples = vec![0.25; TOTAL * channels];
    let mut outcomes = Vec::with_capacity(parts.len());
    let mut pending = None;
    let mut rejected = 0;
    let deadline = Instant::now() + DEADLINE;
    while !progress.start.load(Ordering::Acquire) {
        if progress.stop.load(Ordering::Acquire) {
            samples.clear();
            return AudioReturn {
                publication_reader: ready,
                callback,
                gate,
                samples,
                outcomes,
                pending,
                rejected,
            };
        }
        assert!(Instant::now() < deadline, "startup timeout");
        thread::sleep(Duration::from_micros(100));
    }
    let mut output = 0;
    for frames in parts {
        if progress.stop.load(Ordering::Acquire) {
            break;
        }
        // Functional cadence, not device deadlines: a delayed wake must not cause
        // a burst of catch-up callbacks. This waits on no source/control progress;
        // missing publication still reaches the unchanged strict guard below.
        let next = Instant::now()
            + Duration::from_micros(
                u64::try_from(frames).expect("bounded cadence") * MICROS_PER_FRAME,
            );
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
        if progress.stop.load(Ordering::Acquire) {
            break;
        }
        let slice = &mut samples[output * channels..(output + frames) * channels];
        let outcome = callback_probe::without_heap(|| {
            if pending.is_none()
                && let Some(operation) = operations.try_pop()
            {
                pending = match operation {
                    Operation::Reset(owner) => wrapper(&mut callback)
                        .offer_reset(owner, RESET)
                        .err()
                        .map(|(reason, owner)| Retirement::RefusedReset(reason, owner)),
                    Operation::Activation(offer) => wrapper(&mut callback)
                        .offer_activation(offer)
                        .err()
                        .map(|(reason, offer)| Retirement::RefusedActivation(reason, offer)),
                };
            }
            pending = pending
                .take()
                .or_else(|| take_retirement(&mut callback))
                .and_then(|owner| returns.try_push(owner).err());
            if pending.is_some() {
                progress.blocked_returns.fetch_add(1, Ordering::Release);
            }
            // Acquire the proof FIRST. HostCallback then drains raw and sound queues.
            if let Some((publication, raw)) = ready.try_read()
                && update(&callback, &mut gate, publication, raw).is_err()
            {
                rejected += 1;
            }
            let outcome = render_guarded(&mut callback, &mut gate, slice);
            // A retirement produced by this render must reach control during the
            // inter-callback interval, not after the next callback has already begun.
            pending = pending
                .take()
                .or_else(|| take_retirement(&mut callback))
                .and_then(|owner| returns.try_push(owner).err());
            outcome
        });
        outcomes.push(outcome);
        output += frames;
        progress.output.store(output, Ordering::Release);
    }
    progress.done.store(true, Ordering::Release);
    samples.truncate(output * channels);
    AudioReturn {
        publication_reader: ready,
        callback,
        gate,
        samples,
        outcomes,
        pending,
        rejected,
    }
}

struct Driver {
    rig: Rig,
    successor: Option<(MixedBridgeControl, ResetOwner, ControlPort)>,
    reset_offer: Option<ResetOwner>,
    next_port: Option<ControlPort>,
    sealed: bool,
    reset_sent: bool,
    committed: bool,
    seek_sent: bool,
    stale_sent: bool,
    reset_enabled: bool,
    next_source: usize,
}
impl Driver {
    fn new(profile: HostProfile, reset: bool) -> Self {
        let mut rig = fresh(profile);
        let successor = if reset {
            rig.wrap_for_reset(STOP);
            let (prepared, candidate) = saved_prepared_with_profile(RESET, profile);
            let MixedBridgeArmed {
                control,
                audio,
                control_port,
                audio_port,
            } = arm_prepared(&rig.raw, Setup::default(), prepared, candidate, profile);
            Some((
                control,
                MixedBridgeResetOwner::new(audio, audio_port).expect("successor"),
                control_port,
            ))
        } else {
            None
        };
        Self {
            rig,
            successor,
            reset_offer: None,
            next_port: None,
            sealed: false,
            reset_sent: false,
            committed: false,
            seek_sent: false,
            stale_sent: false,
            reset_enabled: reset,
            next_source: 0,
        }
    }
    fn feed_serial(&mut self, output: usize) {
        while self.next_source < CHUNKS
            && u64::try_from(output).expect("output") + 1_024
                > u64::try_from(self.next_source).expect("chunk") * CHUNK
        {
            for source in [FIRST, SECOND] {
                self.rig.send_all(source, &chunk(source, self.next_source));
            }
            self.next_source += 1;
        }
    }
    fn tick(&mut self, output: usize) -> Option<Operation> {
        service_prefix_control(&mut self.rig);
        assert!(self.rig.errors.is_empty() && self.rig.raw_errors.is_empty());
        if self.reset_enabled
            && !self.sealed
            && self
                .rig
                .control
                .publication()
                .is_some_and(|proof| proof.through() >= RESET)
        {
            let (control, owner, port) = self.successor.take().expect("prebuilt reset");
            self.rig
                .control
                .seal_reset(control, RESET)
                .expect("sealed before future releases forward");
            self.reset_offer = Some(owner);
            self.next_port = Some(port);
            self.sealed = true;
        }
        if self.sealed
            && !self.reset_sent
            && self.rig.control.audio_boundary() > STOP
            && self.rig.control.reset_ready() == Some(RESET)
            && self.rig.control.last_stamp().is_none_or(|last| {
                last.quantum_start()
                    .checked_add(FrameCount::QUANTUM)
                    .is_ok_and(|end| self.rig.control.audio_boundary() >= end)
            })
        {
            self.reset_sent = true;
            return Some(Operation::Reset(
                self.reset_offer.take().expect("sealed reset audio"),
            ));
        }
        if self.committed && !self.seek_sent {
            self.seek_sent = true;
            return Some(Operation::Activation(
                self.rig
                    .control
                    .prepare_activation(SEEK, PlanPosition::ZERO, SEEK, MixedActivationKind::Seek)
                    .expect("seek before live release"),
            ));
        }
        if self.seek_sent && !self.stale_sent && output >= 6_144 {
            self.stale_sent = true;
            let at = at(7_168);
            return Some(Operation::Activation(
                self.rig
                    .control
                    .prepare_activation(at, PlanPosition::ZERO, at, MixedActivationKind::Seek)
                    .expect("uncollected-sequence candidate"),
            ));
        }
        None
    }
    fn retire(&mut self, retired: Retirement) {
        match retired {
            Retirement::Reset(retired) => {
                assert_eq!(retired.receipt().at(), RESET);
                assert_eq!(
                    retired.receipt().ended().count(),
                    2,
                    "held live notes reset, raw continues"
                );
                let old = self
                    .rig
                    .control
                    .commit_reset(retired.receipt())
                    .expect("exact reset receipt");
                self.rig.reset_old.push((
                    old,
                    std::mem::replace(
                        &mut self.rig.control_port,
                        self.next_port.take().expect("new control port"),
                    ),
                ));
                self.rig.collect();
                join_retired(
                    &mut self.rig.reset_old,
                    &mut self.rig.retired_resets,
                    retired,
                );
                self.committed = true;
            }
            Retirement::Activation(retired) => {
                let receipt = self
                    .rig
                    .control
                    .collect_activation(retired)
                    .expect("seek retirement");
                assert!(
                    matches!(receipt.outcome(), MixedActivationOutcome::Adopted(adopted) if adopted.effective() == SEEK)
                );
                assert!(!receipt.late());
            }
            Retirement::RefusedActivation(reason, offer) => {
                assert!(matches!(reason, MixedActivationRefusal::Superseded { .. }));
                assert_eq!(offer.effective(), at(7_168));
                drop(offer);
            }
            Retirement::RefusedReset(reason, owner) => {
                drop(owner);
                panic!("unexpected reset refusal: {reason:?}");
            }
        }
    }
    fn finish(mut self) -> (Vec<u32>, Vec<String>, Vec<String>) {
        self.rig.collect();
        assert_eq!(self.rig.messages().count(), 2 * (CHUNKS + 3));
        assert_eq!(self.rig.settled.len(), self.rig.messages().count());
        let mut reset_releases = 0;
        for (_, settled) in &self.rig.settled {
            match settled {
                Settled::Report(report) => sustained_host::assert_applied_sound_outcome(*report),
                Settled::Pump(MixedSplit::AfterRefusedOnset {
                    reason: MixedOnsetRefusal::Reset,
                    ..
                }) if self.reset_enabled => reset_releases += 1,
                other => panic!("unexpected source settlement: {other:?}"),
            }
        }
        assert_eq!(reset_releases, if self.reset_enabled { 2 } else { 0 });
        if self.reset_enabled {
            assert!(self.committed && self.seek_sent && self.stale_sent);
        }
        assert!(self.rig.rendered.iter().any(|sample| *sample != 0.0));
        finish(self.rig)
    }
}
fn raw_ready(rig: &Rig) -> MixedRawPublication {
    let queued = rig.raw_queued_inputs.as_ref().expect("raw send tracing");
    let traced = [FIRST, SECOND]
        .map(|source| {
            rig.admitted
                .iter()
                .filter_map(|admitted| {
                    if admitted.source != source
                        || !admitted.raw_id().is_some_and(|id| queued.contains(&id))
                    {
                        return None;
                    }
                    let InputObservation::Frontier { tick } = admitted.observation else {
                        return None;
                    };
                    Some(
                        rig.raw.clocks[source]
                            .map(tick)
                            .expect("bound source clock"),
                    )
                })
                .max()
                .unwrap_or(SampleTime::ZERO)
        })
        .into_iter()
        .min()
        .expect("two sources");
    let native = rig.raw_publisher.publication();
    assert_eq!(native.through(), traced);
    native
}
fn serial(profile: HostProfile, reset: bool) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let mut driver = Driver::new(profile, reset);
    let mut delayed = None;
    let mut refused = None;
    for output in (0..TOTAL).step_by(64) {
        driver.feed_serial(output);
        if let Some(operation) = driver.tick(output) {
            match operation {
                Operation::Reset(owner) => {
                    wrapper(driver.rig.callback.as_mut().expect("serial callback"))
                        .offer_reset(owner, RESET)
                        .expect("serial reset")
                }
                Operation::Activation(offer) => {
                    if let Err((reason, offer)) =
                        wrapper(driver.rig.callback.as_mut().expect("callback"))
                            .offer_activation(offer)
                    {
                        refused = Some(Retirement::RefusedActivation(reason, offer));
                    }
                }
            }
        }
        driver.rig.render(64).expect("serial callback");
        if let Some(retired) =
            take_retirement(driver.rig.callback.as_mut().expect("serial callback"))
        {
            if matches!(retired, Retirement::Reset(_)) {
                driver.retire(retired);
            } else {
                assert!(delayed.replace(retired).is_none());
            }
        }
    }
    if let Some(retired) = delayed {
        driver.retire(retired);
    }
    if let Some(retired) = refused {
        driver.retire(retired);
    }
    driver.finish()
}
fn continuous(
    profile: HostProfile,
    reset: bool,
    pattern: &[usize],
) -> (Vec<u32>, Vec<String>, Vec<String>) {
    let mut driver = Driver::new(profile, reset);
    let (mut ready, incoming) = driver.rig.control.publication_mailbox();
    let (mut operations, operation_inbox) = HeapRb::new(1).split();
    let (returns, mut retired) = HeapRb::new(1).split();
    // Workers is declared after the control owner and joins before its fields drop.
    let workers = Workers::new(&mut driver.rig, pattern, incoming, operation_inbox, returns);
    let progress = Arc::clone(&workers.progress);
    let startup = Instant::now() + DEADLINE;
    loop {
        assert!(Instant::now() < startup, "source startup deadline");
        assert!(driver.tick(0).is_none());
        if raw_ready(&driver.rig).through() >= at(1_024)
            && driver
                .rig
                .control
                .publication()
                .is_some_and(|proof| proof.through() >= at(1_024))
        {
            break;
        }
        thread::sleep(Duration::from_micros(100));
    }
    let mut last = (
        driver.rig.control.publication().expect("startup proof"),
        raw_ready(&driver.rig),
    );
    ready.publish(last.0, last.1).expect("startup publication");
    progress.start.store(true, Ordering::Release);
    let deadline = Instant::now() + DEADLINE;
    while !progress.done.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "continuous host deadline");
        let output = progress.output.load(Ordering::Acquire);
        if let Some(operation) = driver.tick(output) {
            assert!(
                operations.try_push(operation).is_ok(),
                "one operation pending"
            );
        }
        // Reset must commit immediately; the later seek's return deliberately fills
        // the mailbox while its stale successor is retained on audio.
        if !driver.committed
            && let Some(retired) = retired.try_pop()
        {
            driver.retire(retired);
        }
        if let Some(proof) = driver.rig.control.publication() {
            let next = (proof, raw_ready(&driver.rig));
            if next != last {
                ready
                    .publish(next.0, next.1)
                    .expect("same bridge latest publication");
                last = next;
            }
        }
        thread::sleep(Duration::from_micros(50));
    }
    let (returned, source_refusals) = workers.join(&mut driver.rig);
    let _publication_reader = returned.publication_reader;
    assert!(
        returned.gate.fault().is_none(),
        "{:?} at {:?}; {:?} {:?} reset={reset} {pattern:?}; sources={source_refusals:?}",
        returned.gate.fault(),
        returned.outcomes.iter().position(Result::is_err),
        profile.capabilities().sample_rate(),
        driver.rig.mixed_layout
    );
    assert!(source_refusals.is_empty(), "{source_refusals:?}");
    assert_eq!(returned.outcomes.len(), callback_parts(pattern).len());
    for outcome in returned.outcomes {
        assert_eq!(
            outcome,
            Ok(Rendered {
                mixed: Ok(()),
                raw_ok: true
            })
        );
    }
    assert_eq!(progress.output.load(Ordering::Acquire), TOTAL);
    assert!(operations.is_empty());
    if reset {
        assert!(progress.blocked_returns.load(Ordering::Acquire) > 0);
    }
    while let Some(retired) = retired.try_pop() {
        driver.retire(retired);
    }
    if let Some(retired) = returned.pending {
        driver.retire(retired);
    }
    let _rejected_stale_updates = returned.rejected;
    driver.rig.callback = Some(returned.callback);
    driver.rig.rendered = returned.samples;
    driver.finish()
}

#[test]
fn independent_cadence_matches_serial_at_four_rates_both_layouts_with_reset_and_backpressure() {
    for (rate, layout) in candidates() {
        let profile = profile(rate, layout, FrameCount::new(1_024));
        for reset in [false, true] {
            let oracle = serial(profile, reset);
            for pattern in [&[64][..], &[7, 13, 1, 43][..], &[256][..]] {
                assert_eq!(
                    continuous(profile, reset, pattern),
                    oracle,
                    "{rate:?} {layout:?} reset={reset} {pattern:?}"
                );
            }
        }
    }
}

#[test]
fn bridge_certificate_caps_unforwarded_heads_and_foreign_raw_proofs_are_rejected() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = fresh(profile);
    rig.send_all(FIRST, &[message(80, note_on(60)), frontier(1_024)]);
    rig.send_all(SECOND, &[message(90, note_on(64)), frontier(1_024)]);
    rig.stage_all();
    let publication = rig.control.publication().expect("actual stage proof");
    assert_eq!(
        publication.through(),
        at(80),
        "neither head has been forwarded"
    );
    let mut gate = rig.control.publication_gate();
    let callback = rig.callback.as_ref().expect("callback");
    update(
        callback,
        &mut gate,
        publication,
        rig.raw_publisher.publication(),
    )
    .expect("same owners");
    let other = fresh(profile);
    assert_eq!(
        update(
            callback,
            &mut gate,
            publication,
            other.raw_publisher.publication()
        ),
        Err(MixedPublicationRefusal::Foreign)
    );
    assert_eq!(
        update(
            callback,
            &mut gate,
            other.control.publication().expect("other empty arm"),
            rig.raw_publisher.publication()
        ),
        Err(MixedPublicationRefusal::Foreign)
    );
    let fault = MixedPublicationFault::Unprepared {
        ready: at(80),
        required: at(128),
    };
    assert_eq!(
        callback_probe::without_heap(|| check(callback, &mut gate, FrameCount::new(192))),
        Err(fault)
    );
    rig.service();
    let publication = rig.control.publication().expect("now forwarded");
    assert_eq!(publication.through(), at(1_024));
    let callback = rig.callback.as_ref().expect("callback");
    update(
        callback,
        &mut gate,
        publication,
        rig.raw_publisher.publication(),
    )
    .expect("late full proof");
    assert_eq!(
        callback_probe::without_heap(|| check(callback, &mut gate, FrameCount::new(192))),
        Err(fault),
        "refill cannot revive latched preflight"
    );
    rig.join().assert_each_packet_once();
    other.join().assert_each_packet_once();
}

#[test]
fn control_stall_exhausts_publication_without_another_audio_permit_or_final_render() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Mono,
        FrameCount::new(1_024),
    );
    let mut rig = fresh(profile);
    for source in [FIRST, SECOND] {
        for index in 0..4 {
            rig.send_all(source, &chunk(source, index));
        }
    }
    rig.service();
    let (mut ready, incoming) = rig.control.publication_mailbox();
    assert!(
        ready
            .publish(
                rig.control.publication().expect("initial proof"),
                rig.raw_publisher.publication()
            )
            .is_ok()
    );
    let (operations, op_inbox) = HeapRb::new(1).split();
    let (returns, retired) = HeapRb::new(1).split();
    let progress = Arc::new(Progress::default());
    let callback = rig.callback.take().expect("one handoff");
    let gate = rig.control.publication_gate();
    let mut workers = Workers {
        progress: Arc::clone(&progress),
        sources: Vec::new(),
        audio: None,
    };
    let shared = Arc::clone(&progress);
    workers.audio = Some(thread::spawn(move || {
        audio_worker(
            callback,
            gate,
            callback_parts(&[64]),
            incoming,
            op_inbox,
            returns,
            shared,
        )
    }));
    progress.start.store(true, Ordering::Release);
    // Control does no pumping, collection or publication during the whole run.
    let returned = workers
        .audio
        .take()
        .expect("handle")
        .join()
        .expect("bounded autonomous run");
    workers.progress.joined.store(true, Ordering::Release);
    let _publication_reader = returned.publication_reader;
    let fault = MixedPublicationFault::Unprepared {
        ready: at(1_024),
        required: at(1_088),
    };
    assert_eq!(returned.gate.fault(), Some(fault));
    assert_eq!(returned.outcomes.len(), TOTAL / 64);
    assert!(returned.outcomes[..17].iter().all(|result| *result
        == Ok(Rendered {
            mixed: Ok(()),
            raw_ok: true
        })));
    assert!(
        returned.outcomes[17..]
            .iter()
            .all(|result| *result == Err(fault))
    );
    assert!(silent(&returned.samples[1_088..]));
    assert_eq!(rig.control.audio_boundary(), at(1_024));
    rig.callback = Some(returned.callback);
    rig.rendered = returned.samples;
    assert!(operations.is_empty() && retired.is_empty() && returned.pending.is_none());
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.host_halted && joined.raw_halted);
    assert_eq!(
        joined.teardown.fault(),
        None,
        "strict host reason remains separate"
    );
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
    assert_eq!(joined.rendered.len(), TOTAL, "no extra shutdown callback");
    assert_eq!(
        joined.teardown.open().len(),
        2,
        "both live releases stay classified at join"
    );
}

#[test]
fn slot_pressure_is_a_retained_sound_refusal_and_never_a_raw_or_publication_fault() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = fresh(profile);
    for source in [FIRST, SECOND] {
        let delta = if source == FIRST { 0 } else { 10 };
        let key = if source == FIRST { 60 } else { 64 };
        let mut observations = Vec::new();
        for on in [80, 160, 240] {
            observations.push(message(on + delta, note_on(key)));
            observations.push(message(on + 40 + delta, note_off(key)));
        }
        observations.push(frontier(1_024));
        rig.send_all(source, &observations);
    }
    rig.service();
    let mut gate = rig.control.publication_gate();
    update(
        rig.callback.as_ref().expect("callback"),
        &mut gate,
        rig.control.publication().expect("prefix"),
        rig.raw_publisher.publication(),
    )
    .expect("proof");
    for _ in 0..16 {
        check(
            rig.callback.as_ref().expect("callback"),
            &mut gate,
            FrameCount::QUANTUM,
        )
        .expect("prepared");
        rig.render(64).expect("slot pressure remains nonterminal");
    }
    rig.collect();
    let mut dropped = 0;
    for (_, settled) in &rig.settled {
        if let Settled::Report(report) = settled {
            assert_eq!(report.nominal_at(), report.executed_at());
            if matches!(
                report.outcome(),
                MixedBridgeOutcome::NoteOn(Err(
                    synth_engine_v2::ingress::IngressRefused::Dropped {
                        resource: synth_engine_v2::ingress::ExhaustedResource::Slot
                    }
                ))
            ) {
                dropped += 1;
            }
        }
    }
    assert_eq!(
        dropped, 2,
        "six overlapping release tails need more than four slots"
    );
    assert_eq!(gate.fault(), None);
    finish(rig);
}

fn reset_fixture(profile: HostProfile) -> Rig {
    let mut rig = fresh(profile);
    rig.wrap_for_reset(at(192));
    for source in [FIRST, SECOND] {
        rig.send(source, frontier(2_048));
    }
    rig.service();
    for _ in 0..6 {
        rig.render(64).expect("stopped prelude");
        rig.collect();
    }
    assert_eq!(rig.control.audio_boundary(), at(320));
    rig
}
fn successor(
    rig: &Rig,
    profile: HostProfile,
    at: SampleTime,
) -> (MixedBridgeControl, ResetOwner, ControlPort) {
    let (prepared, candidate) = saved_prepared_with_profile(at, profile);
    let MixedBridgeArmed {
        control,
        audio,
        control_port,
        audio_port,
    } = arm_prepared(&rig.raw, Setup::default(), prepared, candidate, profile);
    (
        control,
        MixedBridgeResetOwner::new(audio, audio_port).expect("bound successor"),
        control_port,
    )
}
fn commit(rig: &mut Rig, retired: ResetRetired, port: ControlPort) {
    let old = rig
        .control
        .commit_reset(retired.receipt())
        .expect("exact receipt");
    rig.reset_old
        .push((old, std::mem::replace(&mut rig.control_port, port)));
    rig.collect();
    join_retired(&mut rig.reset_old, &mut rig.retired_resets, retired);
}

#[test]
fn a_missed_reset_boundary_returns_the_owner_and_rearms_without_halting_either_path() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = reset_fixture(profile);
    let ordinary_old = rig.control.publication().expect("old arm proof");
    let (control, owner, port) = successor(&rig, profile, at(384));
    rig.control
        .seal_reset(control, at(384))
        .expect("first seal");
    for _ in 0..5 {
        rig.render(64)
            .expect("callback clock does not wait for reset offer");
        rig.collect();
    }
    let (reason, owner) = rig
        .reset_wrapper()
        .offer_reset(owner, at(384))
        .expect_err("missed fixed boundary");
    assert!(matches!(
        reason,
        MixedBridgeResetRefusal::BoundaryPassed { .. }
    ));
    let unused = rig
        .control
        .abandon_reset(&owner)
        .expect("return exact fresh owner");
    let (audio, audio_port) = owner.into_parts();
    let joined = unused
        .join(audio, Some(port), Some(audio_port))
        .expect("unused successor custody");
    assert_eq!(joined.transfer().custody(), MixedTransferCustody::Complete);
    let next = rig
        .control
        .audio_boundary()
        .checked_add(FrameCount::new(128))
        .expect("later boundary");
    let (control, owner, port) = successor(&rig, profile, next);
    rig.control.seal_reset(control, next).expect("later seal");
    let mut gate = rig.control.publication_gate();
    update(
        rig.callback.as_ref().expect("callback"),
        &mut gate,
        rig.control.publication().expect("transition"),
        rig.raw_publisher.publication(),
    )
    .expect("exact transition proof");
    rig.reset_wrapper()
        .offer_reset(owner, next)
        .expect("later owner accepted");
    let end = next
        .checked_add(FrameCount::new(128))
        .expect("fade complete");
    while rig.control.audio_boundary() < next {
        check(
            rig.callback.as_ref().expect("callback"),
            &mut gate,
            FrameCount::QUANTUM,
        )
        .expect("transition preflight");
        rig.render(64).expect("next callback");
        rig.collect();
    }
    // The old bridge's boundary stops at B; read raw's continuing clock here.
    while rig
        .callback
        .as_ref()
        .expect("callback")
        .raw
        .acknowledged()
        .clock
        < end
    {
        check(
            rig.callback.as_ref().expect("callback"),
            &mut gate,
            FrameCount::QUANTUM,
        )
        .expect("successor preflight");
        rig.render(64).expect("fade callback");
        rig.collect();
    }
    let retired = rig.reset_wrapper().take_retired().expect("fade retirement");
    commit(&mut rig, retired, port);
    assert_eq!(
        update(
            rig.callback.as_ref().expect("callback"),
            &mut gate,
            ordinary_old,
            rig.raw_publisher.publication()
        ),
        Err(MixedPublicationRefusal::Foreign)
    );
    update(
        rig.callback.as_ref().expect("callback"),
        &mut gate,
        rig.control.publication().expect("new arm"),
        rig.raw_publisher.publication(),
    )
    .expect("successor accepted");
    check(
        rig.callback.as_ref().expect("callback"),
        &mut gate,
        FrameCount::QUANTUM,
    )
    .expect("stale rejection retained valid proof");
    assert!(!rig.host_halt.is_requested() && !rig.raw.halt.is_requested());
    finish(rig);
}

#[test]
fn a_rendered_reset_retirement_is_sent_without_waiting_for_the_next_callback() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = reset_fixture(profile);
    // Use the worker's declared reset B, retaining an empty promised interval.
    // Advance the old stopped owner close to B before its single autonomous call.
    for source in [FIRST, SECOND] {
        rig.send(source, frontier(8_192));
    }
    rig.service();
    while rig.control.audio_boundary() < at(4_032) {
        rig.render(64).expect("stopped run");
        rig.collect();
    }
    let (control, owner, port) = successor(&rig, profile, RESET);
    rig.control.seal_reset(control, RESET).expect("seal");
    let (mut publisher, reader) = rig.control.publication_mailbox();
    publisher
        .publish(
            rig.control.publication().expect("transition"),
            rig.raw_publisher.publication(),
        )
        .expect("publish");
    let (mut operations, incoming) = HeapRb::new(1).split();
    assert!(operations.try_push(Operation::Reset(owner)).is_ok());
    let (returns, mut retired) = HeapRb::new(1).split();
    let progress = Arc::new(Progress::default());
    let callback = rig.callback.take().expect("one handoff");
    let gate = rig.control.publication_gate();
    let mut workers = Workers {
        progress: Arc::clone(&progress),
        sources: Vec::new(),
        audio: None,
    };
    let shared = Arc::clone(&progress);
    workers.audio = Some(thread::spawn(move || {
        audio_worker(callback, gate, vec![256], reader, incoming, returns, shared)
    }));
    progress.start.store(true, Ordering::Release);
    let returned = workers
        .audio
        .take()
        .expect("handle")
        .join()
        .expect("one callback");
    workers.progress.joined.store(true, Ordering::Release);
    let _reader = returned.publication_reader;
    assert_eq!(
        returned.outcomes,
        [Ok(Rendered {
            mixed: Ok(()),
            raw_ok: true
        })]
    );
    let Retirement::Reset(reset) = retired
        .try_pop()
        .expect("post-render return before another callback")
    else {
        panic!("expected reset");
    };
    assert!(retired.is_empty() && operations.is_empty() && returned.pending.is_none());
    rig.callback = Some(returned.callback);
    rig.rendered.extend(returned.samples);
    commit(&mut rig, reset, port);
    finish(rig);
}

#[test]
fn a_transition_certificate_for_an_abandoned_pair_cannot_cover_another_successor() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = reset_fixture(profile);
    let (first, owner, port) = successor(&rig, profile, at(384));
    rig.control.seal_reset(first, at(384)).expect("first seal");
    let stale = rig.control.publication().expect("first transition proof");
    let first = rig
        .control
        .abandon_reset(&owner)
        .expect("unused pair abandoned");
    let (audio, audio_port) = owner.into_parts();
    assert_eq!(
        first
            .join(audio, Some(port), Some(audio_port))
            .expect("first pair custody")
            .transfer()
            .custody(),
        MixedTransferCustody::Complete
    );
    let (second, owner, port) = successor(&rig, profile, at(384));
    rig.control.seal_reset(second, at(384)).expect("new pair");
    rig.reset_wrapper()
        .offer_reset(owner, at(384))
        .expect("second successor pending");
    let mut gate = rig.control.publication_gate();
    update(
        rig.callback.as_ref().expect("callback"),
        &mut gate,
        stale,
        rig.raw_publisher.publication(),
    )
    .expect("still the same old arm before B");
    assert_eq!(
        callback_probe::without_heap(|| check(
            rig.callback.as_ref().expect("callback"),
            &mut gate,
            FrameCount::new(192)
        )),
        Err(MixedPublicationFault::MissingArm),
        "crossing requires the exact pending pair"
    );
    let returned = rig
        .reset_wrapper()
        .withdraw_pending()
        .expect("unrendered successor retained");
    let second = rig
        .control
        .abandon_reset(&returned)
        .expect("return second pair");
    let (audio, audio_port) = returned.into_parts();
    assert_eq!(
        second
            .join(audio, Some(port), Some(audio_port))
            .expect("second custody")
            .transfer()
            .custody(),
        MixedTransferCustody::Complete
    );
    rig.join().assert_each_packet_once();
}

#[test]
fn a_foreign_raw_policy_cannot_authorize_a_raw_only_halt_in_another_host() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = fresh(profile);
    let other = fresh(profile);
    let mut policy = synth_engine_v2::host::mixed_input::MixedRawPolicy::new(
        &other.raw.control,
        other.control.sources(),
    )
    .expect("other binding");
    let late = InputOutcome::Delivered(SessionSourceOutcome::Refused(
        NoteCaptureError::PastBoundary,
    ));
    policy
        .observe(other.generation(FIRST), &late)
        .expect("other late class");
    let observation = message(80, note_on(60));
    rig.send(FIRST, observation);
    let packet = MixedSourcePacketId::new(rig.generation(FIRST), 1).expect("first source packet");
    rig.raw.halt.request_invalid();
    let refused = rig
        .control
        .route_raw_first(&mut rig.raw.control, &policy, packet, observation)
        .expect_err("foreign policy cannot excuse the halt");
    assert!(matches!(
        *refused,
        synth_engine_v2::host::mixed_input::MixedInputRefusal::Halted {
            fault: MixedBridgeError::Fault(MixedBridgeFault::RawSelfHalt),
            ..
        }
    ));
    rig.halt_host();
    assert_eq!(
        policy.observe(rig.generation(FIRST), &late),
        Err(InputError::Stale)
    );
    assert_eq!(policy.decision(), Some(RawHaltClass::Terminal));
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert!(joined.unexamined.contains(&(FIRST, observation)));
    assert_eq!(joined.teardown.fault(), Some(MixedBridgeFault::RawSelfHalt));
    other.join().assert_each_packet_once();
}

#[test]
fn a_later_host_panic_is_not_relabelled_as_an_earlier_source_promise() {
    let profile = profile(
        SampleRate::new(48_000.0).expect("rate"),
        ChannelLayout::Stereo,
        FrameCount::new(1_024),
    );
    let mut rig = fresh(profile);
    for source in [FIRST, SECOND] {
        for index in 0..2 {
            rig.send_all(source, &chunk(source, index));
        }
    }
    rig.service();
    let proof = rig.control.publication().expect("source prefix");
    assert_eq!(proof.through(), at(1_024));
    for _ in 0..5 {
        rig.render(64).expect("already sounding source prefix");
        rig.collect();
    }
    rig.control
        .panic(at(100))
        .expect("asynchronous host request retains nominal time");
    rig.service();
    for _ in 0..11 {
        rig.render(64).expect("late Panic is not a host fault");
        rig.collect();
    }
    assert_eq!(rig.panics.len(), 1);
    let report = rig.panics[0];
    assert_eq!(report.nominal_at(), at(100));
    assert!(report.executed_at() > report.nominal_at());
    assert_eq!(report.stamp(), report.executed_at());
    assert!(!rig.host_halt.is_requested() && !rig.raw.halt.is_requested());
    let joined = rig.join();
    joined.assert_each_packet_once();
    assert_eq!(
        joined.teardown.transfer().custody(),
        MixedTransferCustody::Complete
    );
}

struct ControlDropWitness {
    driver: Driver,
    progress: Option<Arc<Progress>>,
    observed: Arc<AtomicBool>,
}
impl Drop for ControlDropWitness {
    fn drop(&mut self) {
        self.observed.store(
            self.progress.as_ref().is_some_and(|progress| {
                progress.joined.load(Ordering::Acquire)
                    && progress.sources_joined.load(Ordering::Acquire) == 2
            }),
            Ordering::Release,
        );
    }
}
#[test]
fn control_unwind_joins_both_sources_and_continuous_audio_before_control_drop() {
    let observed = Arc::new(AtomicBool::new(false));
    let mut shared = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let profile = profile(
            SampleRate::new(48_000.0).expect("rate"),
            ChannelLayout::Mono,
            FrameCount::new(1_024),
        );
        let mut control = ControlDropWitness {
            driver: Driver::new(profile, false),
            progress: None,
            observed: Arc::clone(&observed),
        };
        let (mut publisher, reader) = control.driver.rig.control.publication_mailbox();
        let (_operations, inbox) = HeapRb::new(1).split();
        let (returns, _retired) = HeapRb::new(1).split();
        let workers = Workers::new(&mut control.driver.rig, &[64], reader, inbox, returns);
        let progress = Arc::clone(&workers.progress);
        control.progress = Some(Arc::clone(&progress));
        shared = Some(Arc::clone(&progress));
        let deadline = Instant::now() + DEADLINE;
        while raw_ready(&control.driver.rig).through() < at(1_024) {
            assert!(Instant::now() < deadline);
            assert!(control.driver.tick(0).is_none());
            thread::sleep(Duration::from_micros(100));
        }
        publisher
            .publish(
                control.driver.rig.control.publication().expect("prefix"),
                raw_ready(&control.driver.rig),
            )
            .expect("publish");
        progress.start.store(true, Ordering::Release);
        while progress.output.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_micros(100));
        }
        panic!("control unwind with autonomous callbacks active");
    }));
    assert!(result.is_err());
    assert!(observed.load(Ordering::Acquire));
    let progress = shared.expect("workers started");
    assert!(progress.joined.load(Ordering::Acquire));
    assert_eq!(progress.sources_joined.load(Ordering::Acquire), 2);
}

#[test]
fn invalid_output_is_retryable_before_strict_preflight_or_raw_admission() {
    for layout in [ChannelLayout::Stereo, ChannelLayout::Mono] {
        let profile = profile(
            SampleRate::new(48_000.0).expect("rate"),
            layout,
            FrameCount::new(64),
        );
        let mut rig = fresh(profile);
        let mut gate = rig.control.publication_gate();
        let callback = rig.callback.as_mut().expect("callback");
        let sizes = if layout == ChannelLayout::Stereo {
            vec![127, 130]
        } else {
            vec![65]
        };
        for size in sizes {
            let mut output = vec![0.25; size];
            assert_eq!(
                callback_probe::without_heap(|| render_guarded(callback, &mut gate, &mut output)),
                Ok(Rendered {
                    mixed: Err(MixedHostRenderError::Host(
                        MixedTransferRenderError::OutputShape
                    )),
                    raw_ok: false
                })
            );
            assert!(silent(&output));
            assert_eq!(gate.fault(), None);
            assert!(!callback.host_halt.is_requested() && !callback.raw_halt.is_requested());
        }
        // An empty, valid proof covers the initial carry. Invalid shapes consumed
        // neither that carry nor raw, and have not latched MissingArm/Unprepared.
        update(
            callback,
            &mut gate,
            rig.control.publication().expect("empty proof"),
            rig.raw_publisher.publication(),
        )
        .expect("matching owners");
        let mut output = vec![0.25; 64 * layout.channels()];
        assert_eq!(
            callback_probe::without_heap(|| render_guarded(callback, &mut gate, &mut output)),
            Ok(Rendered {
                mixed: Ok(()),
                raw_ok: true
            })
        );
        assert_eq!(gate.fault(), None);
        rig.collect();
        rig.join().assert_each_packet_once();
    }
}
