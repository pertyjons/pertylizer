//! Stop, Start and seek interacting on the private mixed owner's one-slot offer (draft
//! Item 2, rules 2.1 to 2.3, without loop or Panic).
//!
//! A Stop at `S` cancels a candidate pending at or after `S` into an owned retired
//! receipt; one before `S` adopts first. An offer at or after a pending Stop's boundary
//! is refused and returned. A Start adopts and resumes compiled publication; a seek
//! while stopped adopts and publishes only its scoped restoration, as Session. Live
//! notes must render as a live-only renderer renders them through all of it, in both
//! producer orders and four callback partitions.

use super::history_tests::{bound_with_events, key, mixed_profile};
use super::*;
use crate::quantities::{ChannelLayout, NoteVelocity};
use crate::render::{AudioBlockMut, NoteEdge, Renderer, TimedEvents};
use crate::schedule::PlanEvent;
use crate::time::{QUANTUM_FRAMES, TimeSource as TestOrigin};

/// The armed transition adopts here, at plan position 10: compiled key 60 sounds from
/// 64 until plan 300 (engine 354) unless a transport change ends it first.
const ARMED: SampleTime = SampleTime::new(64);
const STOP: SampleTime = SampleTime::new(192);
/// Where every later candidate lands: compiled key 72 starts exactly here.
const DESTINATION: u64 = 400;
/// Output index `i` holds engine sample `i - 64`: the first output quantum is the carry.
const fn output(at: SampleTime) -> usize {
    at.as_u64() as usize + QUANTUM_FRAMES as usize
}
const FRAMES: usize = 640;

const PARTITIONS: [&[usize]; 4] = [
    &[512, 128],
    &[64; 10],
    &[8; 80],
    &[7, 63, 90, 1, 256, 95, 128],
];

fn bound(compiled_first: bool) -> MixedJoinedPrepared {
    bound_with_events(compiled_first, |note| {
        let on = |at, raw| {
            PlanEvent::new(
                PlanPosition::new(at),
                CompiledPayload::NoteOn {
                    slot: note,
                    key: key(raw),
                    velocity: NoteVelocity::FULL,
                },
            )
        };
        let off = |at, raw| {
            PlanEvent::new(
                PlanPosition::new(at),
                CompiledPayload::NoteOff {
                    slot: note,
                    key: key(raw),
                },
            )
        };
        vec![on(10, 60), off(300, 60), on(400, 72), off(500, 72)]
    })
}

fn first_candidate(
    prepared: &MixedJoinedPrepared,
    requested: SampleTime,
    position: PlanPosition,
) -> MixedStampedCandidate {
    let history = prepared
        .prepare_history(requested, position)
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    prepared.stamp_suffix(suffix).expect("stamp")
}

fn armed_successor() -> ActivationSequence {
    ActivationSequence::INITIAL.next().expect("successor")
}

fn refused_offers(audio: &MixedOneShotAudio) -> u64 {
    audio.audio.renderer.diagnostics().refused_activations()
}

/// Everything audio's identity custody and program hold.
fn custody(audio: &MixedOneShotAudio) -> String {
    format!(
        "{:?} {:?} {:?} {:?} {:?} {:?} {} {} {:?} {:?}",
        audio.audio.compiled_minter,
        audio.audio.live_minter,
        audio.audio.compiled_tokens,
        audio.audio.compiled_ended,
        audio.audio.renderer.snapshot_mixed_sounding(),
        audio.timing,
        audio.events.len(),
        audio.next,
        audio.in_force,
        audio.stop_ended,
    )
}

/// One armed owner driven through a script of transport commands between callbacks.
/// Callbacks follow a repeating partition pattern; a script point splits the callback
/// that crosses it, so every command lands between two callbacks.
struct Session {
    control: MixedOneShotControl,
    audio: MixedOneShotAudio,
    samples: Vec<f32>,
    rendered: usize,
    pattern: &'static [usize],
    index: usize,
    left: usize,
    /// Restoration counts by adoption, in order: the armed one first.
    restorations: Vec<usize>,
}

impl Session {
    /// Arm at 64, returning a live-only reference renderer of the same owner, epoch and
    /// table, so it hears this owner's live identities.
    fn new(compiled_first: bool, pattern: &'static [usize]) -> (Self, LiveReference) {
        let prepared = bound(compiled_first);
        let candidate = first_candidate(&prepared, ARMED, PlanPosition::new(10));
        let reference = LiveReference::of(&prepared);
        let (control, audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("private arm");
        let restoration = audio
            .capsule
            .restoration_count
            .as_usize()
            .expect("restoration count");
        let session = Self {
            control,
            audio,
            samples: vec![0.0; FRAMES],
            rendered: 0,
            pattern,
            index: 0,
            left: 0,
            restorations: vec![restoration],
        };
        (session, reference)
    }

    /// Render up to output frame `frame`, asserting no callback allocates or frees.
    fn render_to(&mut self, frame: usize) {
        while self.rendered < frame {
            if self.left == 0 {
                self.left = self.pattern[self.index % self.pattern.len()];
                self.index += 1;
            }
            let frames = self.left.min(frame - self.rendered);
            self.left -= frames;
            let start = self.rendered;
            let block = AudioBlockMut::new(
                &mut self.samples[start..start + frames],
                frames,
                ChannelLayout::Mono,
            )
            .expect("mono block");
            let audio = &mut self.audio;
            let allocations = crate::render_allocation::count_allocs(|| {
                audio.render_private(block).expect("private render");
            });
            assert_eq!(allocations, 0, "callback allocated or deallocated");
            self.rendered += frames;
        }
    }

    fn stop(&mut self, at: SampleTime) -> Result<MixedStopAccepted, MixedStopRefused> {
        let mut result = None;
        let audio = &mut self.audio;
        let allocations = crate::render_allocation::count_allocs(|| {
            result = Some(audio.schedule_compiled_stop(at));
        });
        assert_eq!(allocations, 0, "scheduling a Stop allocated or deallocated");
        result.expect("schedule ran")
    }

    fn seal(&self, at: SampleTime, kind: MixedOfferKind) -> MixedCandidateOffer {
        let candidate = self
            .control
            .prepare_subsequent(at, PlanPosition::new(DESTINATION), self.control.in_force)
            .expect("later candidate");
        self.control
            .seal_offer(candidate, at, kind)
            .expect("sealed offer")
    }

    fn offer(
        &mut self,
        at: SampleTime,
        kind: MixedOfferKind,
    ) -> Result<(), (MixedOfferRefused, MixedCandidateOffer)> {
        let offer = self.seal(at, kind);
        let restoration = offer
            .0
            .candidate
            .restoration_count
            .as_usize()
            .expect("restoration count");
        let mut result = None;
        let audio = &mut self.audio;
        let allocations = crate::render_allocation::count_allocs(|| {
            result = Some(audio.offer_candidate(offer));
        });
        assert_eq!(allocations, 0, "the offer allocated or deallocated");
        let result = result.expect("offer ran");
        if result.is_ok() {
            self.restorations.push(restoration);
        }
        result
    }

    /// Take the retired offer on the audio side and collect it on control's side.
    fn collect(&mut self) -> MixedOfferReceipt {
        let mut retired = None;
        let audio = &mut self.audio;
        let allocations = crate::render_allocation::count_allocs(|| {
            retired = Some(audio.take_retired_offer());
        });
        assert_eq!(
            allocations, 0,
            "taking the retired offer allocated or freed"
        );
        let retired = retired.flatten().expect("retired offer");
        self.control.collect_offer(retired).expect("collected")
    }

    fn finish(mut self) -> Self {
        self.render_to(FRAMES);
        self
    }
}

/// A renderer bound to the same plan and partition that hears only live events.
struct LiveReference {
    renderer: PreparedRenderer,
    epoch: StreamEpoch,
}

impl LiveReference {
    fn of(prepared: &MixedJoinedPrepared) -> Self {
        let mut renderer = PreparedRenderer::prepare(
            Arc::clone(&prepared.owner.control.plan),
            prepared.owner.control.anchor,
            prepared.epoch(),
            prepared.table_id(),
        )
        .expect("reference renderer");
        assert!(renderer.bind_mixed_partition(Arc::clone(&prepared.owner.control.partition)));
        Self {
            renderer,
            epoch: prepared.epoch(),
        }
    }
}

fn render_live_only(reference: LiveReference, events: &[TimedEvent]) -> Vec<f32> {
    let mut reference = reference.renderer;
    let mut samples = vec![0.0_f32; FRAMES];
    for chunk in samples.chunks_mut(QUANTUM_FRAMES as usize) {
        let start = reference.clock().as_u64();
        let quanta = reference.quanta_needed_for(chunk.len()) as u64;
        let end = start + quanta * u64::from(QUANTUM_FRAMES);
        let due = events
            .iter()
            .copied()
            .filter(|event| (start..end).contains(&event.envelope().time().as_u64()))
            .collect::<Vec<_>>();
        let frames = chunk.len();
        reference
            .render(
                AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).expect("reference block"),
                TimedEvents::new(&due),
            )
            .expect("reference quantum");
    }
    samples
}

/// The same destination reached by a seek while running at `at`, compiled only: what a
/// Start there must sound like from its boundary on.
fn running_seek(compiled_first: bool, at: SampleTime) -> Vec<f32> {
    let (mut session, _) = Session::new(compiled_first, PARTITIONS[0]);
    session
        .offer(at, MixedOfferKind::Seek)
        .map_err(|(reason, _)| reason)
        .expect("running seek");
    session.finish().samples
}

fn live_edge(epoch: StreamEpoch, at: u64, identity: NoteIdentity, edge: NoteEdge) -> TimedEvent {
    TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::new(at), TestOrigin::Simulated),
        EventPayload::Note { identity, edge },
    )
}

fn live_on(
    epoch: StreamEpoch,
    at: u64,
    identity: NoteIdentity,
    slot: NoteSlot,
    raw: u8,
) -> TimedEvent {
    live_edge(
        epoch,
        at,
        identity,
        NoteEdge::On {
            slot,
            key: key(raw),
            velocity: NoteVelocity::FULL,
        },
    )
}

fn assert_sum(mixed: &[f32], compiled: &[f32], live: &[f32]) {
    assert_eq!(mixed.len(), compiled.len());
    for (index, ((mixed, compiled), live)) in mixed.iter().zip(compiled).zip(live).enumerate() {
        assert!(
            (mixed - (compiled + live)).abs() < 0.00001,
            "frame {index}: mixed {mixed}, compiled {compiled}, live {live}"
        );
    }
}

fn assert_close(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (actual - expected).abs() < 0.00001,
            "frame {index}: {actual} != {expected}"
        );
    }
}

fn silent(samples: &[f32]) -> bool {
    samples.iter().all(|sample| *sample == 0.0)
}

fn identities(notes: &[crate::identity::EndedNote]) -> Vec<NoteIdentity> {
    notes.iter().map(|note| note.identity).collect()
}

/// The two Start scripts of the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartScript {
    /// Stop at 192, then Start at 320 to plan 400.
    StopStart,
    /// Stop at 192, seek while stopped at 320 to plan 400, then Start at 448 there.
    StoppedSeekStart,
}

impl StartScript {
    const fn start(self) -> SampleTime {
        match self {
            Self::StopStart => SampleTime::new(320),
            Self::StoppedSeekStart => SampleTime::new(448),
        }
    }
}

const SEEK_WHILE_STOPPED: SampleTime = SampleTime::new(320);

/// Live keys for the matrix, all through the private ingress store: 48 held from 0
/// across every transport change until 560; 49 starting exactly on the Stop boundary
/// and released exactly on the first later boundary (320); 50 starting at 448, which
/// is the stopped-seek script's Start boundary, and released at 512.
#[cfg(feature = "simulated-ingress")]
struct Live {
    held: NoteIdentity,
    on_stop: NoteIdentity,
    after: Option<NoteIdentity>,
}

#[cfg(feature = "simulated-ingress")]
impl Live {
    fn before(audio: &mut MixedOneShotAudio) -> Self {
        let held = audio
            .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("held onset");
        let on_stop = audio
            .offer_test_note_on(STOP, key(49), NoteVelocity::FULL)
            .expect("onset on the Stop boundary");
        audio
            .offer_live_note_off(SEEK_WHILE_STOPPED, on_stop)
            .expect("release on the later boundary");
        Self {
            held,
            on_stop,
            after: None,
        }
    }

    /// Key 50 needs key 49's place, so it is offered once 49's release has applied.
    fn after(&mut self, audio: &mut MixedOneShotAudio) {
        let after = audio
            .offer_test_note_on(SampleTime::new(448), key(50), NoteVelocity::FULL)
            .expect("onset on the stopped-seek Start boundary");
        audio
            .offer_live_note_off(SampleTime::new(512), after)
            .expect("release after Start");
        audio
            .offer_live_note_off(SampleTime::new(560), self.held)
            .expect("held release");
        self.after = Some(after);
    }

    fn events(&self, epoch: StreamEpoch, slot: NoteSlot) -> Vec<TimedEvent> {
        let after = self.after.expect("key 50 offered");
        vec![
            live_on(epoch, 0, self.held, slot, 48),
            live_on(epoch, STOP.as_u64(), self.on_stop, slot, 49),
            live_edge(
                epoch,
                SEEK_WHILE_STOPPED.as_u64(),
                self.on_stop,
                NoteEdge::Off,
            ),
            live_on(epoch, 448, after, slot, 50),
            live_edge(epoch, 512, after, NoteEdge::Off),
            live_edge(epoch, 560, self.held, NoteEdge::Off),
        ]
    }
}

#[cfg(feature = "simulated-ingress")]
struct MatrixRun {
    session: Session,
    live_only: Option<Vec<f32>>,
    seek_receipt: Option<MixedOfferReceipt>,
    start_receipt: MixedOfferReceipt,
}

/// Run one Start script over `pattern`, with or without the live keys.
#[cfg(feature = "simulated-ingress")]
fn matrix_run(
    compiled_first: bool,
    script: StartScript,
    pattern: &'static [usize],
    with_live: bool,
) -> MatrixRun {
    let (mut session, reference) = Session::new(compiled_first, pattern);
    let epoch = reference.epoch;
    let mut live = with_live.then(|| Live::before(&mut session.audio));
    assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
    session.render_to(output(SEEK_WHILE_STOPPED));
    assert_eq!(
        session.audio.report().transport,
        MixedCompiledTransport::Stopped {
            at: STOP,
            released: HeldNoteCount::measured(1),
            position: Some(PlanPosition::new(138)),
        }
    );
    let mut seek_receipt = None;
    match script {
        StartScript::StopStart => {
            session
                .offer(script.start(), MixedOfferKind::Start)
                .map_err(|(reason, _)| reason)
                .expect("Start while stopped");
            session.render_to(output(SampleTime::new(448)));
        }
        StartScript::StoppedSeekStart => {
            session
                .offer(SEEK_WHILE_STOPPED, MixedOfferKind::Seek)
                .map_err(|(reason, _)| reason)
                .expect("seek while stopped");
            session.render_to(output(SampleTime::new(448)));
            // Adopted while stopped: only the restoration published, no note edge.
            let audio = &session.audio;
            let restoration = session.restorations[1];
            assert!(restoration > 0);
            assert_eq!(audio.next, restoration);
            assert!(audio.events.len() > restoration);
            assert!(audio.audio.compiled_minter.live_identities().is_empty());
            assert!(
                audio
                    .audio
                    .compiled_tokens
                    .iter()
                    .all(|token| *token == super::MixedCompiledTokenState::Free)
            );
            assert_eq!(audio.audio.renderer.seeded_row_count(), 0);
            assert!(matches!(
                audio.report().transport,
                MixedCompiledTransport::Stopped { at: STOP, .. }
            ));
            seek_receipt = Some(session.collect());
        }
    }
    if let Some(live) = &mut live {
        live.after(&mut session.audio);
    }
    if script == StartScript::StoppedSeekStart {
        session
            .offer(script.start(), MixedOfferKind::Start)
            .map_err(|(reason, _)| reason)
            .expect("Start after the stopped seek");
    }
    let mut session = session.finish();
    let start_receipt = session.collect();
    let live_only =
        live.map(|live| render_live_only(reference, &live.events(epoch, session.audio.audio.note)));
    MatrixRun {
        session,
        live_only,
        seek_receipt,
        start_receipt,
    }
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn start_and_stopped_seek_resume_or_hold_compiled_publication_and_keep_live_notes() {
    for compiled_first in [true, false] {
        for script in [StartScript::StopStart, StartScript::StoppedSeekStart] {
            let base = matrix_run(compiled_first, script, PARTITIONS[0], true);
            let compiled = matrix_run(compiled_first, script, PARTITIONS[0], false);
            let live_only = base.live_only.as_deref().expect("live reference");
            let start = script.start();
            let start_out = output(start);
            let stop_out = output(STOP);

            // Live continuity: the mixed output is the compiled-only run plus a live-only
            // renderer's output, before, across and after every transport change.
            assert_sum(&base.session.samples, &compiled.session.samples, live_only);
            // Compiled key 60 sounded before the Stop; nothing compiled sounds while
            // stopped, including across the stopped seek and its restoration.
            assert!(!silent(&compiled.session.samples[output(ARMED)..stop_out]));
            assert!(silent(&compiled.session.samples[stop_out..start_out]));
            assert_eq!(
                &base.session.samples[stop_out..start_out],
                &live_only[stop_out..start_out]
            );
            // Key 49 is heard from the Stop boundary; 48 sounds throughout.
            assert!(!silent(&live_only[stop_out..stop_out + 64]));
            // From Start the compiled program publishes as a running seek there does.
            assert!(!silent(
                &compiled.session.samples[start_out..start_out + 64]
            ));
            assert_close(
                &compiled.session.samples[start_out..],
                &running_seek(compiled_first, start)[start_out..],
            );

            let report = base.session.audio.report();
            assert!(report.adopted && !report.faulted, "{report:?}");
            assert_eq!(report.transport, MixedCompiledTransport::Running);
            assert_eq!(report.stop_pending, None);
            assert_eq!(report.stops_charged, EventCount::measured(1));
            let restorations = &base.session.restorations;
            let adoptions = restorations.len();
            assert_eq!(report.releases_charged.as_usize(), Some(adoptions));
            // Every adoption's restoration is Session, including the stopped seek's.
            assert_eq!(
                report.restoration_charged.as_usize(),
                Some(restorations.iter().sum())
            );
            // Key 60's onset, then key 72's onset and release after Start.
            assert_eq!(report.suffix_charged, EventCount::measured(3));
            assert_eq!(report.offer, MixedOfferSlotState::Empty);
            assert_eq!(refused_offers(&base.session.audio), 0);

            let retired_anchor = match script {
                StartScript::StopStart => {
                    assert_eq!(adoptions, 2);
                    assert!(base.seek_receipt.is_none());
                    StreamAnchor::new(ARMED, PlanPosition::new(10))
                }
                StartScript::StoppedSeekStart => {
                    assert_eq!(adoptions, 3);
                    let seek = base.seek_receipt.expect("seek receipt");
                    assert_eq!(seek.kind, MixedOfferKind::Seek);
                    let MixedOfferOutcome::Adopted(adopted) = seek.outcome else {
                        panic!("the stopped seek adopts: {seek:?}");
                    };
                    assert_eq!(adopted.effective, SEEK_WHILE_STOPPED);
                    assert!(adopted.stopped);
                    // The Stop had already ended every compiled occurrence.
                    assert_eq!(adopted.released, HeldNoteCount::NONE);
                    assert_eq!(adopted.retired_unconsumed_from, restorations[0] + 1);
                    StreamAnchor::new(SEEK_WHILE_STOPPED, PlanPosition::new(DESTINATION))
                }
            };
            let receipt = base.start_receipt;
            assert_eq!(receipt.kind, MixedOfferKind::Start);
            assert!(!receipt.late);
            let MixedOfferOutcome::Adopted(adopted) = receipt.outcome else {
                panic!("the Start adopts: {receipt:?}");
            };
            assert_eq!(adopted.effective, start);
            assert_eq!(
                adopted.anchor,
                StreamAnchor::new(start, PlanPosition::new(DESTINATION))
            );
            assert_eq!(adopted.retired_anchor, retired_anchor);
            assert_eq!(adopted.released, HeldNoteCount::NONE);
            assert!(!adopted.stopped);
            assert_eq!(base.session.control.in_force, receipt.sequence);
            assert_eq!(base.session.control.in_force_effective, start);

            // Every live and compiled obligation ended; every hold was redeemed.
            let audio = &base.session.audio;
            let sounding = audio
                .audio
                .renderer
                .snapshot_mixed_sounding()
                .expect("bound registry");
            assert!(sounding.compiled().is_empty());
            assert!(sounding.live().is_empty());
            assert!(audio.audio.compiled_minter.live_identities().is_empty());
            assert!(
                audio
                    .audio
                    .live_minter
                    .live_identities_for_test()
                    .is_empty()
            );
            assert_eq!(audio.test_ingress.holds_outstanding(), EventCount::NONE);

            for pattern in &PARTITIONS[1..] {
                let run = matrix_run(compiled_first, script, pattern, true);
                assert_eq!(
                    run.session.samples, base.session.samples,
                    "{script:?} {pattern:?}"
                );
                assert_eq!(run.session.audio.report(), report, "{script:?} {pattern:?}");
                assert_eq!(run.start_receipt, receipt, "{script:?} {pattern:?}");
                assert_eq!(
                    run.seek_receipt, base.seek_receipt,
                    "{script:?} {pattern:?}"
                );
            }
        }
    }
}

/// What every cancellation script must sound like: Stop at 192 with nothing else.
fn stop_only(compiled_first: bool, pattern: &'static [usize]) -> (Session, NoteIdentity) {
    let (mut session, _) = Session::new(compiled_first, pattern);
    let live = session
        .audio
        .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("test live onset");
    assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
    (session.finish(), live)
}

/// The scripts in which a candidate meets a Stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancelScript {
    /// A seek at 256 is pending when a Stop at 192 is scheduled.
    AfterStop,
    /// A seek at 192 is pending when a Stop at 192 is scheduled.
    SameBoundaryOfferFirst,
    /// A Stop at 192 is pending when a seek at 192 is offered.
    SameBoundaryStopFirst,
    /// A Start at 384 is pending while stopped when a Stop at 384 is scheduled.
    StartAtStop,
}

struct CancelRun {
    session: Session,
    live: NoteIdentity,
    /// The candidate refused at the offer, if the script's offer was refused.
    returned: Option<(MixedOfferRefused, MixedCandidateOffer)>,
}

fn cancel_run(compiled_first: bool, script: CancelScript, pattern: &'static [usize]) -> CancelRun {
    let (mut session, _) = Session::new(compiled_first, pattern);
    let live = session
        .audio
        .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("test live onset");
    let mut returned = None;
    match script {
        CancelScript::AfterStop => {
            session
                .offer(SampleTime::new(256), MixedOfferKind::Seek)
                .map_err(|(reason, _)| reason)
                .expect("seek after the Stop");
            assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
        }
        CancelScript::SameBoundaryOfferFirst => {
            session
                .offer(STOP, MixedOfferKind::Seek)
                .map_err(|(reason, _)| reason)
                .expect("seek on the Stop boundary");
            assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
        }
        CancelScript::SameBoundaryStopFirst => {
            assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
            let before = (custody(&session.audio), session.audio.report());
            returned = Some(
                session
                    .offer(STOP, MixedOfferKind::Seek)
                    .expect_err("one transport change per boundary"),
            );
            assert_eq!((custody(&session.audio), session.audio.report()), before);
        }
        CancelScript::StartAtStop => {
            assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
            session.render_to(output(SampleTime::new(320)));
            session
                .offer(SampleTime::new(384), MixedOfferKind::Start)
                .map_err(|(reason, _)| reason)
                .expect("Start while stopped");
            // Stop while stopped still cancels the Start it would otherwise precede.
            assert_eq!(
                session.stop(SampleTime::new(384)),
                Ok(MixedStopAccepted::Scheduled)
            );
        }
    }
    CancelRun {
        session: session.finish(),
        live,
        returned,
    }
}

#[test]
fn a_stop_cancels_a_candidate_at_or_after_it_and_refuses_one_offered_there() {
    for compiled_first in [true, false] {
        let (reference, _) = stop_only(compiled_first, PARTITIONS[0]);
        let stop_report = reference.audio.report();
        for script in [
            CancelScript::AfterStop,
            CancelScript::SameBoundaryOfferFirst,
            CancelScript::SameBoundaryStopFirst,
            CancelScript::StartAtStop,
        ] {
            let mut base = None;
            for pattern in PARTITIONS {
                let mut run = cancel_run(compiled_first, script, pattern);
                // The program, the charges and the live note are exactly a lone Stop's.
                assert_eq!(
                    run.session.samples, reference.samples,
                    "{script:?} {pattern:?}"
                );
                let report = run.session.audio.report();
                assert_eq!(
                    MixedOneShotRenderReport {
                        offer: MixedOfferSlotState::Empty,
                        ..report
                    },
                    stop_report,
                    "{script:?} {pattern:?}"
                );
                assert_eq!(run.session.audio.in_force, armed_successor());
                assert_eq!(
                    run.session.audio.audio.live_minter.resolve(run.live),
                    Resolution::Live
                );
                match script {
                    CancelScript::SameBoundaryStopFirst => {
                        let (reason, offer) = run.returned.take().expect("refused offer");
                        assert_eq!(
                            reason,
                            MixedOfferRefused::StopPending {
                                stop: STOP,
                                effective: STOP,
                            }
                        );
                        // The candidate came back whole, ready to drop off-thread.
                        assert_eq!(offer.0.timing.effective(), STOP);
                        assert_eq!(offer.0.candidate.supersedes, armed_successor());
                        assert_eq!(offer.0.released, HeldNoteCount::NONE);
                        assert_eq!(report.offer, MixedOfferSlotState::Empty);
                        assert_eq!(refused_offers(&run.session.audio), 1);
                    }
                    _ => {
                        let (stop, kind) = match script {
                            CancelScript::StartAtStop => {
                                (SampleTime::new(384), MixedOfferKind::Start)
                            }
                            _ => (STOP, MixedOfferKind::Seek),
                        };
                        let receipt = MixedOfferReceipt {
                            supersedes: armed_successor(),
                            sequence: armed_successor().next().expect("later"),
                            late: false,
                            kind,
                            outcome: MixedOfferOutcome::Cancelled { stop },
                        };
                        assert_eq!(report.offer, MixedOfferSlotState::Retired(receipt));
                        // A cancellation is not a refusal.
                        assert_eq!(refused_offers(&run.session.audio), 0);
                        // Control collects the cancelled receipt; its sequence stays.
                        assert_eq!(run.session.collect(), receipt);
                        assert_eq!(run.session.control.in_force, armed_successor());
                        assert_eq!(run.session.control.in_force_effective, ARMED);
                    }
                }
                let samples = run.session.samples.clone();
                match &base {
                    None => base = Some(samples),
                    Some(base) => assert_eq!(&samples, base, "{script:?} {pattern:?}"),
                }
            }
        }
    }
}

#[test]
fn a_seek_before_a_pending_stop_adopts_first_and_the_stop_ends_its_program() {
    for compiled_first in [true, false] {
        let seek = SampleTime::new(128);
        // Falsifier control: the same seek with no Stop keeps key 72 until 228.
        let (mut running, _) = Session::new(compiled_first, PARTITIONS[0]);
        running
            .offer(seek, MixedOfferKind::Seek)
            .map_err(|(reason, _)| reason)
            .expect("seek");
        let running = running.finish();
        let mut base = None;
        for pattern in PARTITIONS {
            let (mut session, _) = Session::new(compiled_first, pattern);
            let live = session
                .audio
                .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
                .expect("test live onset");
            session
                .offer(seek, MixedOfferKind::Seek)
                .map_err(|(reason, _)| reason)
                .expect("seek before the Stop");
            assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
            let mut session = session.finish();
            let report = session.audio.report();
            // The seek ended key 60 at 128; the Stop ended the seek's key 72 at 192.
            assert_eq!(
                report.transport,
                MixedCompiledTransport::Stopped {
                    at: STOP,
                    released: HeldNoteCount::measured(1),
                    position: Some(PlanPosition::new(DESTINATION + 64)),
                }
            );
            assert_eq!(report.stops_charged, EventCount::measured(1));
            assert_eq!(report.releases_charged, EventCount::measured(2));
            let receipt = session.collect();
            let MixedOfferOutcome::Adopted(adopted) = receipt.outcome else {
                panic!("the earlier seek adopts: {receipt:?}");
            };
            assert_eq!(adopted.effective, seek);
            assert_eq!(adopted.released, HeldNoteCount::measured(1));
            assert!(!adopted.stopped);
            assert_eq!(
                session.audio.audio.live_minter.resolve(live),
                Resolution::Live
            );
            assert!(
                session
                    .audio
                    .audio
                    .compiled_minter
                    .live_identities()
                    .is_empty()
            );
            let stopped = &session.samples;
            match &base {
                None => {
                    // Live 48 alone is heard after the Stop; key 72 still sounded before.
                    let (lone, _) = stop_only(compiled_first, PARTITIONS[0]);
                    assert_eq!(&stopped[output(STOP)..], &lone.samples[output(STOP)..]);
                    let (unlived, _) = Session::new(compiled_first, PARTITIONS[0]);
                    let mut unlived = unlived;
                    unlived
                        .offer(seek, MixedOfferKind::Seek)
                        .map_err(|(reason, _)| reason)
                        .expect("seek");
                    assert_eq!(unlived.stop(STOP), Ok(MixedStopAccepted::Scheduled));
                    let unlived = unlived.finish();
                    assert!(!silent(&unlived.samples[output(seek)..output(STOP)]));
                    assert!(silent(&unlived.samples[output(STOP)..]));
                    assert!(!silent(
                        &running.samples[output(STOP)..output(SampleTime::new(228))]
                    ));
                    base = Some(stopped.clone());
                }
                Some(base) => assert_eq!(stopped, base, "{pattern:?}"),
            }
        }
    }
}

#[test]
fn a_start_before_a_later_stop_resumes_until_that_stop() {
    for compiled_first in [true, false] {
        let mut base = None;
        for pattern in PARTITIONS {
            let (mut session, _) = Session::new(compiled_first, pattern);
            assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
            session.render_to(output(SampleTime::new(320)));
            session
                .offer(SampleTime::new(320), MixedOfferKind::Start)
                .map_err(|(reason, _)| reason)
                .expect("Start");
            // While stopped, a Stop after a pending Start follows it.
            assert_eq!(
                session.stop(SampleTime::new(384)),
                Ok(MixedStopAccepted::Scheduled)
            );
            let mut session = session.finish();
            let report = session.audio.report();
            assert_eq!(
                report.transport,
                MixedCompiledTransport::Stopped {
                    at: SampleTime::new(384),
                    released: HeldNoteCount::measured(1),
                    position: Some(PlanPosition::new(DESTINATION + 64)),
                }
            );
            assert_eq!(report.stops_charged, EventCount::measured(2));
            let receipt = session.collect();
            assert!(matches!(
                receipt.outcome,
                MixedOfferOutcome::Adopted(MixedOfferAdopted { stopped: false, .. })
            ));
            let samples = &session.samples;
            assert!(silent(&samples[output(STOP)..output(SampleTime::new(320))]));
            assert!(!silent(
                &samples[output(SampleTime::new(320))..output(SampleTime::new(384))]
            ));
            assert!(silent(&samples[output(SampleTime::new(384))..]));
            match &base {
                None => base = Some(samples.clone()),
                Some(base) => assert_eq!(samples, base, "{pattern:?}"),
            }
        }
    }
}

#[test]
fn transport_refusals_and_acknowledgements_return_the_candidate_and_change_nothing() {
    for compiled_first in [true, false] {
        // A Start needs a stopped program at its boundary.
        let (mut session, _) = Session::new(compiled_first, PARTITIONS[1]);
        let before = (custody(&session.audio), session.audio.report());
        let (reason, offer) = session
            .offer(SampleTime::new(256), MixedOfferKind::Start)
            .expect_err("Start while running");
        assert_eq!(reason, MixedOfferRefused::StartWhileRunning);
        assert_eq!(offer.0.timing.effective(), SampleTime::new(256));
        assert_eq!((custody(&session.audio), session.audio.report()), before);
        // A pending Stop does not make the program stopped before its boundary, and
        // refuses any candidate at or after it.
        assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
        let before = (custody(&session.audio), session.audio.report());
        let (reason, _) = session
            .offer(SampleTime::new(128), MixedOfferKind::Start)
            .expect_err("Start before the Stop");
        assert_eq!(reason, MixedOfferRefused::StartWhileRunning);
        let (reason, _) = session
            .offer(SampleTime::new(256), MixedOfferKind::Start)
            .expect_err("Start after the pending Stop");
        assert_eq!(
            reason,
            MixedOfferRefused::StopPending {
                stop: STOP,
                effective: SampleTime::new(256),
            }
        );
        assert_eq!((custody(&session.audio), session.audio.report()), before);
        assert_eq!(refused_offers(&session.audio), 3);
        assert_eq!(
            session.stop(SampleTime::new(256)),
            Err(MixedStopRefused::Pending { pending: STOP })
        );

        // Stopped: a seek before a later Stop leaves the program stopped, so that Stop
        // is acknowledged; one at the seek's boundary would cancel it.
        session.render_to(output(SampleTime::new(320)));
        let stopped = session.audio.report();
        assert_eq!(stopped.stop_pending, None);
        session
            .offer(SampleTime::new(384), MixedOfferKind::Seek)
            .map_err(|(reason, _)| reason)
            .expect("seek while stopped");
        let pending = session.audio.report();
        assert_eq!(
            session.stop(SampleTime::new(448)),
            Ok(MixedStopAccepted::AlreadyStopped)
        );
        assert_eq!(session.audio.report(), pending);
        assert_eq!(
            session.stop(SampleTime::new(384)),
            Ok(MixedStopAccepted::Scheduled)
        );
        assert_eq!(
            session.audio.report().stop_pending,
            Some(SampleTime::new(384))
        );
        let session = session.finish();
        let report = session.audio.report();
        let MixedOfferSlotState::Retired(receipt) = report.offer else {
            panic!("the Stop at the seek's boundary cancels it");
        };
        assert_eq!(receipt.kind, MixedOfferKind::Seek);
        assert_eq!(
            receipt.outcome,
            MixedOfferOutcome::Cancelled {
                stop: SampleTime::new(384)
            }
        );
        // A Stop applied while stopped only cancels: no release, no charge.
        assert_eq!(report.transport, stopped.transport);
        assert_eq!(report.stops_charged, EventCount::measured(1));
        assert_eq!(report.stop_pending, None);
    }
}

/// Fault the owner through a real callback: the capsule names another table.
fn fault_by_pairing(session: &mut Session, compiled_first: bool) {
    let foreign = bound(compiled_first).table_id();
    assert_ne!(foreign, session.audio.capsule.table);
    session.audio.capsule.table = foreign;
    let mut samples = [1.0_f32; 1];
    let error = session
        .audio
        .render_private(AudioBlockMut::new(&mut samples, 1, ChannelLayout::Mono).expect("block"))
        .expect_err("capsule table mismatch");
    assert_eq!(error, MixedOneShotRenderError::Pairing);
    assert_eq!(samples, [0.0]);
}

/// Run to just before the Stop quantum with a seek at 256 pending and live 48 held,
/// returning key 60's compiled occurrence, which the Stop is about to end.
fn before_stop_with_pending_seek(compiled_first: bool) -> (Session, NoteIdentity, NoteIdentity) {
    let (mut session, _) = Session::new(compiled_first, PARTITIONS[1]);
    let live = session
        .audio
        .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("test live onset");
    session
        .offer(SampleTime::new(256), MixedOfferKind::Seek)
        .map_err(|(reason, _)| reason)
        .expect("seek");
    assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
    session.render_to(output(STOP));
    let old = session.audio.audio.compiled_minter.live_identities();
    assert_eq!(old.len(), 1, "key 60 sounds up to the Stop");
    (session, old[0], live)
}

#[test]
fn terminal_teardown_owns_a_cancelled_candidate_and_reports_the_stop_record() {
    for compiled_first in [true, false] {
        let (mut session, old, live) = before_stop_with_pending_seek(compiled_first);
        session.render_to(output(STOP) + 64);
        let MixedOfferSlotState::Retired(receipt) = session.audio.report().offer else {
            panic!("the Stop cancelled the seek");
        };
        assert_eq!(receipt.outcome, MixedOfferOutcome::Cancelled { stop: STOP });
        fault_by_pairing(&mut session, compiled_first);
        let Session { control, audio, .. } = session;
        let MixedCollection::Ended(ended) = control.collect(audio).expect("terminal teardown")
        else {
            panic!("a faulted owner ends");
        };
        assert_eq!(ended.end, MixedCollectionEnd::Faulted);
        assert_eq!(ended.report.offer, MixedOfferSlotState::Retired(receipt));
        assert!(matches!(
            &ended.offer,
            MixedDrainedOffer::Retired(retired)
                if retired.receipt == receipt && retired.cell.released == HeldNoteCount::NONE
        ));
        // The cancelled candidate released nothing; the Stop's record names key 60
        // once, apart from the armed adoption's record of the initial list's key 60.
        assert!(ended.offer_boundary_ended.is_empty());
        assert_eq!(identities(&ended.stop_boundary_ended), vec![old]);
        assert_eq!(ended.boundary_ended.len(), 1);
        assert_ne!(ended.boundary_ended[0].identity, old);
        assert!(ended.compiled_minted.is_empty());
        assert!(ended.sounding.compiled().is_empty());
        assert_eq!(identities(ended.sounding.live()), vec![live]);
    }
}

#[test]
fn a_custody_fault_inside_the_stop_keeps_its_release_record() {
    for compiled_first in [true, false] {
        let (mut session, old, live) = before_stop_with_pending_seek(compiled_first);
        // Fault injection: the minter loses key 60 before the Stop's release reaches it.
        assert_eq!(
            session.audio.audio.compiled_minter.release(old),
            Resolution::Live
        );
        let mut samples = [1.0_f32; 64];
        let error = session
            .audio
            .render_private(
                AudioBlockMut::new(&mut samples, 64, ChannelLayout::Mono).expect("block"),
            )
            .expect_err("custody fault at the Stop");
        assert!(
            matches!(error, MixedOneShotRenderError::CompiledRelease { .. }),
            "{error:?}"
        );
        assert!(silent(&samples));
        let report = session.audio.report();
        // The Stop never completed, but it had already cancelled the seek.
        assert_eq!(report.transport, MixedCompiledTransport::Running);
        assert_eq!(report.stop_pending, Some(STOP));
        assert_eq!(report.stops_charged, EventCount::NONE);
        let Session { control, audio, .. } = session;
        let MixedCollection::Ended(ended) = control.collect(audio).expect("terminal teardown")
        else {
            panic!("a faulted owner ends");
        };
        assert!(matches!(
            ended.report.offer,
            MixedOfferSlotState::Retired(MixedOfferReceipt {
                outcome: MixedOfferOutcome::Cancelled { stop: STOP },
                ..
            })
        ));
        // The renderer's release ended key 60 and the Stop's record says so.
        assert_eq!(identities(&ended.stop_boundary_ended), vec![old]);
        assert!(ended.sounding.compiled().is_empty());
        assert_eq!(identities(ended.sounding.live()), vec![live]);
    }
}

#[test]
fn joined_collection_hands_back_a_cancelled_or_withdrawn_start() {
    for cancel in [true, false] {
        let (mut session, _) = Session::new(true, PARTITIONS[1]);
        assert_eq!(session.stop(STOP), Ok(MixedStopAccepted::Scheduled));
        session.render_to(output(SampleTime::new(320)));
        session
            .offer(SampleTime::new(448), MixedOfferKind::Start)
            .map_err(|(reason, _)| reason)
            .expect("Start");
        if cancel {
            assert_eq!(
                session.stop(SampleTime::new(384)),
                Ok(MixedStopAccepted::Scheduled)
            );
        }
        session.render_to(output(SampleTime::new(384)) + 64);
        let Session { control, audio, .. } = session;
        let MixedCollection::Resumed {
            control,
            audio,
            offer,
            ..
        } = control.collect(audio).expect("adopted pair")
        else {
            panic!("an adopted pair resumes");
        };
        assert_eq!(audio.report().offer, MixedOfferSlotState::Empty);
        assert_eq!(control.sequence, armed_successor());
        match *offer {
            MixedDrainedOffer::Retired(retired) => {
                assert!(cancel);
                assert_eq!(retired.receipt.kind, MixedOfferKind::Start);
                assert_eq!(
                    retired.receipt.outcome,
                    MixedOfferOutcome::Cancelled {
                        stop: SampleTime::new(384)
                    }
                );
            }
            MixedDrainedOffer::Withdrawn(offer) => {
                assert!(!cancel);
                assert_eq!(offer.0.kind, MixedOfferKind::Start);
                assert_eq!(offer.0.timing.effective(), SampleTime::new(448));
            }
            MixedDrainedOffer::Empty => panic!("the slot was occupied"),
        }
    }
}
