//! Loop mode in the private mixed owner (draft rules 2 and 3.1, work order stage 5).
//!
//! A loop is entered and left through the single offer slot like a seek: the boundary
//! release, the program and anchor swap and scoped restoration, compiled-scoped. A wrap
//! never uses the slot. It runs inside the quantum's compiled event order at its own
//! sample: a Session-charged `Off` for every open compiled note, one Session operation,
//! then the next pass's restoration and events. No loop step visits a live occurrence,
//! row, ramp or minter, so a held live note cannot tell that a wrap happened.
//!
//! The fixture's loop is `[200, 237)`, 37 frames, entered at engine 128 at plan 230. Its
//! wraps fall at 135 + 37k: twice inside the quantum at 192 (209 and 246) and once
//! exactly on the boundary at 320. The exit at 448 is a plain seek to the plan position
//! the loop plays there. With a constant source and no release, compiled output is the
//! number of sounding compiled notes, so a gate model of the script states every
//! entry, wrap and exit edge to the sample.

use super::history_tests::{key, mixed_profile, profile_with_session_share};
use super::hot::loop_position;
use super::*;
use crate::compile::{RenderConfig, compile};
use crate::identity::Resolution;
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain,
};
use crate::plan::CompiledPlan;
use crate::quantities::{Amplitude, ChannelLayout, NormalizedLevel, NoteVelocity, Seconds};
use crate::render::{AudioBlockMut, NoteEdge, Renderer, TimedEvents};
use crate::schedule::PlanEvent;
use crate::time::{QUANTUM_FRAMES, TimeSource as TestOrigin};

const SOURCE: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);

/// The armed transition adopts plan 0 here.
const ARMED: u64 = 64;
const ENTRY: u64 = 128;
const ENTRY_AT: u64 = 230;
const START: u64 = 200;
const END: u64 = 237;
const LENGTH: u64 = END - START;
const EXIT: u64 = 448;
/// The plan position the loop plays at `EXIT`: 200 + (30 + 320) mod 37.
const EXIT_AT: u64 = 217;
const FRAMES: usize = 704;
/// Each compiled or live voice adds this much while its gate is up.
const LEVEL: f32 = 0.25;

const PARTITIONS: [&[usize]; 5] = [&[1], &[37], &[64], &[256], &[7, 63, 90, 1, 256, 95, 128]];

/// Output index `i` holds engine sample `i - 64`: the first output quantum is the carry.
const fn output(at: u64) -> usize {
    at as usize + QUANTUM_FRAMES as usize
}

fn interval() -> LoopInterval {
    LoopInterval::new(PlanPosition::new(START), PlanPosition::new(END)).expect("loop")
}

/// The wraps the script plays before its exit.
fn wraps() -> Vec<u64> {
    (0..)
        .map(|pass| ENTRY + (END - ENTRY_AT) + pass * LENGTH)
        .take_while(|wrap| *wrap < EXIT)
        .collect()
}

/// One voice of `source` through a gated amplifier: three compiled and four live notes.
fn loop_plan(compiled_first: bool, source: IrNodeKind, release: Seconds) -> CompiledPlan {
    let compiled = NoteProducerDeclaration {
        compiled: true,
        simultaneous_notes: HeldNoteCount::measured(3),
        simultaneous_holds: EventCount::NONE,
    };
    let live = NoteProducerDeclaration {
        compiled: false,
        simultaneous_notes: HeldNoteCount::measured(4),
        simultaneous_holds: EventCount::measured(4),
    };
    let ir = GraphIr::builder()
        .node(SOURCE, source, ExecutionScope::Voice)
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::ZERO,
                decay: Seconds::ZERO,
                sustain: NormalizedLevel::FULL,
                release,
                velocity_sensitivity: NormalizedLevel::ZERO,
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
        .tuning(
            ExecutionScope::Voice,
            crate::tuning::PreparedTuning::equal_temperament().expect("tuning"),
        )
        .declaring(PlanDeclarations {
            note_producers: if compiled_first {
                vec![compiled, live]
            } else {
                vec![live, compiled]
            },
            held_notes: HeldNoteCount::measured(7),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("voice graph");
    compile(&ir, &RenderConfig::new(mixed_profile()))
        .into_plan()
        .expect("plan")
}

fn constant() -> IrNodeKind {
    IrNodeKind::Constant {
        level: Amplitude::new(LEVEL).expect("finite level"),
    }
}

/// Key 60 sounds from plan 0 past the loop. Inside the loop, key 62 sounds from its
/// first frame for ten frames, reusing the voice key 64 leaves at the wrap. Keys 64 and
/// 66 sound from plans 220 and 230 past the loop's end, so every wrap ends both, and
/// key 66's voice falls silent at the wrap: a wrap that forgot an `Off` is audible.
fn bound(plan: CompiledPlan) -> MixedJoinedPrepared {
    let note = plan.resolve_note(ENVELOPE).expect("playable envelope");
    let edge = |at: u64, raw: u8, on: bool| {
        let payload = if on {
            CompiledPayload::NoteOn {
                slot: note,
                key: key(raw),
                velocity: NoteVelocity::FULL,
            }
        } else {
            CompiledPayload::NoteOff {
                slot: note,
                key: key(raw),
            }
        };
        PlanEvent::new(PlanPosition::new(at), payload)
    };
    let events = vec![
        edge(0, 60, true),
        edge(200, 62, true),
        edge(210, 62, false),
        edge(220, 64, true),
        edge(230, 66, true),
        edge(400, 64, false),
        edge(400, 66, false),
        edge(1000, 60, false),
    ];
    let stream = AdmittedCompiledStream::admit(&plan, &events).expect("admitted stream");
    let binding = MixedTargetAdmission::admit(plan, stream, note).expect("mixed target binding");
    MixedJoinedStream::open(
        binding,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("joined owner")
    .prepare_initial()
    .expect("stamped initial schedule")
}

fn arm(
    prepared: MixedJoinedPrepared,
    profile: &HostProfile,
) -> (MixedOneShotControl, MixedOneShotAudio) {
    let history = prepared
        .prepare_history(SampleTime::new(ARMED), PlanPosition::ZERO)
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    prepared
        .arm_one_shot(candidate, profile)
        .expect("private arm")
}

/// Compiled gates the script plays, as engine `[on, off)` spans: key 60 from the
/// joined stream's own start, again from the armed transition's, until the entry's
/// boundary release; key 66 from the entry, whose position is its onset, to the first
/// wrap; then every pass's keys 62, 64 and 66; then keys 64 and 66 again from the exit
/// position's linear plan.
fn compiled_spans() -> Vec<(u64, u64)> {
    let first_wrap = ENTRY + END - ENTRY_AT;
    let mut spans = vec![(0, ARMED), (ARMED, ENTRY), (ENTRY, first_wrap)];
    for wrap in wraps() {
        for (on, off) in [(0, 10), (20, LENGTH), (30, LENGTH)] {
            if wrap + on < EXIT {
                spans.push((wrap + on, (wrap + off).min(EXIT)));
            }
        }
    }
    for on in [220, 230] {
        spans.push((EXIT + on - EXIT_AT, EXIT + 400 - EXIT_AT));
    }
    spans
}

fn sounding(spans: &[(u64, u64)], at: u64) -> usize {
    spans
        .iter()
        .filter(|(on, off)| *on <= at && at < *off)
        .count()
}

/// One armed owner driven over a repeating callback partition; a script point splits
/// the callback that crosses it, and no callback may allocate or free.
struct Driver {
    control: MixedOneShotControl,
    audio: MixedOneShotAudio,
    samples: Vec<f32>,
    rendered: usize,
    pattern: &'static [usize],
    index: usize,
    left: usize,
}

impl Driver {
    fn new(
        prepared: MixedJoinedPrepared,
        profile: &HostProfile,
        pattern: &'static [usize],
    ) -> Self {
        let (control, audio) = arm(prepared, profile);
        Self {
            control,
            audio,
            samples: vec![0.0; FRAMES],
            rendered: 0,
            pattern,
            index: 0,
            left: 0,
        }
    }

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

    fn offer_loop(&mut self, at: u64, kind: MixedOfferKind) {
        let candidate = self
            .control
            .prepare_loop(
                SampleTime::new(at),
                PlanPosition::new(ENTRY_AT),
                interval(),
                self.control.in_force,
            )
            .expect("loop candidate");
        let offer = self
            .control
            .seal_loop_offer(candidate, SampleTime::new(at), kind)
            .map_err(|refusal| refusal.0)
            .expect("sealed loop offer");
        self.audio
            .offer_candidate(offer)
            .map_err(|(reason, _)| reason)
            .expect("loop offer");
    }

    fn offer_seek(&mut self, at: u64, position: PlanPosition) {
        let candidate = self
            .control
            .prepare_subsequent(SampleTime::new(at), position, self.control.in_force)
            .expect("seek candidate");
        let offer = self
            .control
            .seal_offer(candidate, SampleTime::new(at), MixedOfferKind::Seek)
            .map_err(|refusal| refusal.0)
            .expect("sealed seek");
        self.audio
            .offer_candidate(offer)
            .map_err(|(reason, _)| reason)
            .expect("seek offer");
    }

    /// Take the retired offer to control, which drops it off-thread. Its cell must hold
    /// the loop program the adoption retired, and nothing when none was in force.
    fn collect(&mut self, retires_loop: bool) -> MixedOfferAdopted {
        let retired = self.audio.take_retired_offer().expect("retired offer");
        assert_eq!(retired.cell.loop_program.is_some(), retires_loop);
        let receipt = self.control.collect_offer(retired).expect("collected");
        let MixedOfferOutcome::Adopted(adopted) = receipt.outcome else {
            panic!("the offer must adopt: {receipt:?}");
        };
        adopted
    }

    /// Enter the loop at `ENTRY`, play it to `EXIT`, then leave it with a plain seek to
    /// the position the loop plays there, and render to the end.
    fn run(mut self) -> (Self, MixedOfferAdopted, MixedOfferAdopted) {
        self.offer_loop(ENTRY, MixedOfferKind::Seek);
        self.render_to(output(EXIT));
        let entry = self.collect(false);
        let anchor = StreamAnchor::new(SampleTime::new(ENTRY), PlanPosition::new(ENTRY_AT));
        let exit_at = loop_position(anchor, interval(), SampleTime::new(EXIT)).expect("mapped");
        assert_eq!(exit_at, PlanPosition::new(EXIT_AT));
        self.offer_seek(EXIT, exit_at);
        self.render_to(FRAMES);
        let exit = self.collect(true);
        (self, entry, exit)
    }
}

fn restoration_groups(audio: &MixedOneShotAudio) -> usize {
    audio.audio.partition.restoration_groups().len()
}

#[test]
fn entry_wraps_and_exit_land_on_their_samples_in_every_partition_and_producer_order() {
    let spans = compiled_spans();
    for compiled_first in [true, false] {
        let mut first = None;
        for pattern in PARTITIONS {
            let prepared = bound(loop_plan(compiled_first, constant(), Seconds::ZERO));
            let driver = Driver::new(prepared, &mixed_profile(), pattern);
            let (driver, entry, exit) = driver.run();
            let context = format!("compiled_first={compiled_first} {pattern:?}");
            for at in 0..(FRAMES - output(0)) as u64 {
                let expected = sounding(&spans, at) as f32 * LEVEL;
                let found = driver.samples[output(at)];
                assert!(
                    (found - expected).abs() < 0.00001,
                    "{context} engine {at}: {found}, expected {expected}"
                );
            }
            let groups = restoration_groups(&driver.audio);
            assert_eq!(entry.loop_interval, Some(interval()), "{context}");
            assert_eq!(entry.retired_pass, None, "{context}");
            // Key 60 was the only compiled note the entry's boundary release found.
            assert_eq!(entry.released, HeldNoteCount::measured(1), "{context}");
            assert_eq!(exit.loop_interval, None, "{context}");
            assert_eq!(
                exit.retired_pass.map(LoopPassId::as_u64),
                Some(1 + wraps().len() as u64),
                "{context}"
            );
            // The loop had wrapped, so its entry list (restoration and key 66's onset)
            // was spent; the rest of the pass it was in never published.
            assert_eq!(exit.retired_unconsumed_from, groups + 1, "{context}");
            assert_eq!(exit.retired_event_count, groups + 1, "{context}");
            assert_eq!(exit.released, HeldNoteCount::NONE, "{context}");
            let report = driver.audio.report();
            assert!(!report.faulted, "{context}: {:?}", report.fault);
            assert_eq!(report.loop_interval, None, "{context}");
            assert_eq!(
                report.loop_wraps.as_usize(),
                Some(wraps().len()),
                "{context}"
            );
            // The first wrap ended key 66; every later one keys 64 and 66.
            assert_eq!(
                report.loop_wrap_releases.as_usize(),
                Some(2 * wraps().len() - 1),
                "{context}"
            );
            // The linear releases at 631 left no compiled occurrence minted.
            assert_eq!(driver.audio.audio.compiled_minter.live(), 0, "{context}");
            let snapshot = (driver.samples, report);
            match &first {
                None => first = Some(snapshot),
                Some(first) => assert_eq!(&snapshot, first, "{context}: partition dependence"),
            }
        }
    }
}

#[test]
fn compiled_custody_follows_each_pass_at_every_quantum_boundary() {
    for compiled_first in [true, false] {
        let prepared = bound(loop_plan(compiled_first, constant(), Seconds::ZERO));
        let mut driver = Driver::new(prepared, &mixed_profile(), &[64]);
        driver.offer_loop(ENTRY, MixedOfferKind::Seek);
        let spans = compiled_spans();
        let mut boundary = ENTRY;
        while boundary < EXIT {
            driver.render_to(output(boundary));
            let clock = driver.audio.audio.renderer.clock().as_u64();
            // Notes whose onset published before the clock and whose end did not.
            let open = spans
                .iter()
                .filter(|(on, off)| *on < clock && clock <= *off)
                .count();
            let context = format!("compiled_first={compiled_first} clock {clock}");
            assert_eq!(
                driver.audio.audio.compiled_minter.live() as usize,
                open,
                "{context}"
            );
            let open_tokens = driver
                .audio
                .audio
                .compiled_tokens
                .iter()
                .filter(|token| matches!(token, MixedCompiledTokenState::Open(_)))
                .count();
            assert_eq!(open_tokens, open, "{context}");
            let snapshot = driver
                .audio
                .audio
                .renderer
                .snapshot_mixed_sounding()
                .expect("registry");
            assert_eq!(snapshot.compiled().len(), open, "{context}");
            boundary += u64::from(QUANTUM_FRAMES);
        }
    }
}

/// A renderer of the same plan, epoch, table and partition as `prepared`, for the
/// live-only reference.
fn reference(prepared: &MixedJoinedPrepared) -> PreparedRenderer {
    let mut renderer = PreparedRenderer::prepare(
        Arc::clone(&prepared.owner.control.plan),
        prepared.owner.control.anchor,
        prepared.epoch(),
        prepared.table_id(),
    )
    .expect("reference renderer");
    assert!(renderer.bind_mixed_partition(Arc::clone(&prepared.owner.control.partition)));
    renderer
}

/// What `renderer` plays hearing only `events`, in quantum-sized calls. Its timeline
/// runs linearly throughout.
fn live_only(mut renderer: PreparedRenderer, events: &[TimedEvent]) -> Vec<f32> {
    let mut samples = vec![0.0_f32; FRAMES];
    for chunk in samples.chunks_mut(QUANTUM_FRAMES as usize) {
        let start = renderer.clock().as_u64();
        let quanta = renderer.quanta_needed_for(chunk.len()) as u64;
        let end = start + quanta * u64::from(QUANTUM_FRAMES);
        let due = events
            .iter()
            .copied()
            .filter(|event| (start..end).contains(&event.envelope().time().as_u64()))
            .collect::<Vec<_>>();
        let frames = chunk.len();
        renderer
            .render(
                AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).expect("reference block"),
                TimedEvents::new(&due),
            )
            .expect("reference quantum");
    }
    samples
}

fn live_edge(epoch: StreamEpoch, at: u64, identity: NoteIdentity, edge: NoteEdge) -> TimedEvent {
    TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::new(at), TestOrigin::Simulated),
        EventPayload::Note { identity, edge },
    )
}

/// Live key 48 is held across entry, every wrap and exit. Keys 50 to 52 start one
/// frame before, at and after the wrap at 209 and end one frame after, at and before
/// the wrap at 246.
const LIVE: [(u8, u64, u64); 4] = [(48, 0, 600), (50, 208, 247), (51, 209, 246), (52, 210, 245)];

#[test]
fn live_notes_and_tails_sound_as_a_live_only_renderer_hears_them_across_the_loop() {
    // A release longer than a pass, so compiled and live tails cross wraps.
    let release = Seconds::new(48.0 / 48_000.0).expect("finite release");
    for compiled_first in [true, false] {
        let mut first = None;
        for pattern in PARTITIONS {
            let compiled_only = {
                let prepared = bound(loop_plan(compiled_first, constant(), release));
                Driver::new(prepared, &mixed_profile(), pattern)
                    .run()
                    .0
                    .samples
            };
            let prepared = bound(loop_plan(compiled_first, constant(), release));
            let epoch = prepared.epoch();
            let renderer = reference(&prepared);
            let mut driver = Driver::new(prepared, &mixed_profile(), pattern);
            let slot = driver.audio.audio.note;
            // The store takes edges in stamp order, so offer them that way.
            let mut edges = LIVE
                .iter()
                .flat_map(|(raw, on, off)| [(*on, *raw, true), (*off, *raw, false)])
                .collect::<Vec<_>>();
            edges.sort_by_key(|(at, _, _)| *at);
            let mut reference = Vec::new();
            let mut identities = std::collections::HashMap::new();
            for (at, raw, on) in edges {
                if on {
                    let identity = driver
                        .audio
                        .offer_test_note_on(SampleTime::new(at), key(raw), NoteVelocity::FULL)
                        .expect("live onset");
                    identities.insert(raw, identity);
                    reference.push(live_edge(
                        epoch,
                        at,
                        identity,
                        NoteEdge::On {
                            slot,
                            key: key(raw),
                            velocity: NoteVelocity::FULL,
                        },
                    ));
                } else {
                    let identity = identities[&raw];
                    driver
                        .audio
                        .offer_live_note_off(SampleTime::new(at), identity)
                        .expect("live release");
                    reference.push(live_edge(epoch, at, identity, NoteEdge::Off));
                }
            }
            driver.offer_loop(ENTRY, MixedOfferKind::Seek);
            driver.render_to(output(EXIT));
            driver.collect(false);
            let anchor = StreamAnchor::new(SampleTime::new(ENTRY), PlanPosition::new(ENTRY_AT));
            let exit_at = loop_position(anchor, interval(), SampleTime::new(EXIT)).expect("mapped");
            driver.offer_seek(EXIT, exit_at);
            driver.render_to(FRAMES);
            driver.collect(true);
            let report = driver.audio.report();
            assert!(!report.faulted, "{:?}", report.fault);

            let live = live_only(renderer, &reference);
            let context = format!("compiled_first={compiled_first} {pattern:?}");
            for (frame, ((mixed, compiled), live)) in driver
                .samples
                .iter()
                .zip(&compiled_only)
                .zip(&live)
                .enumerate()
            {
                assert!(
                    (mixed - (compiled + live)).abs() < 0.00001,
                    "{context} frame {frame}: mixed {mixed}, compiled {compiled}, live {live}"
                );
            }
            match &first {
                None => first = Some(driver.samples),
                Some(first) => assert_eq!(&driver.samples, first, "{context}"),
            }
        }
    }
}

/// The plan position the script plays at engine `at`: the joined stream's own anchor,
/// the armed transition's from 64, the loop from its entry, and the exit's from 448.
fn plan_at(at: u64) -> u64 {
    let first_wrap = ENTRY + END - ENTRY_AT;
    if at < ARMED {
        at
    } else if at < ENTRY {
        at - ARMED
    } else if at < first_wrap {
        ENTRY_AT + (at - ENTRY)
    } else if at < EXIT {
        START + (at - first_wrap) % LENGTH
    } else {
        EXIT_AT + (at - EXIT)
    }
}

#[test]
fn the_plan_timeline_wraps_with_the_loop_under_a_held_live_note() {
    // A voice-scope Impulse clicks once in every sounding voice at each frame that
    // visits its position. Live key 48 sounds throughout, so every visit is audible:
    // plan 215 at the fifteenth frame of every repeating pass, while no compiled note
    // sounds; plan 230 at the entry frame beside key 66's onset, in every repeating
    // pass beside keys 64 and 66, and once more after the exit, beside both again.
    let spans = compiled_spans();
    for target in [215, 230] {
        for compiled_first in [true, false] {
            let mut first = None;
            for pattern in PARTITIONS {
                let prepared = bound(loop_plan(
                    compiled_first,
                    IrNodeKind::Impulse {
                        position: PlanPosition::new(target),
                    },
                    Seconds::ZERO,
                ));
                let mut driver = Driver::new(prepared, &mixed_profile(), pattern);
                let held = driver
                    .audio
                    .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
                    .expect("held live onset");
                let (driver, _, _) = driver.run();
                assert_eq!(
                    driver.audio.audio.live_minter.resolve(held),
                    Resolution::Live
                );
                let context =
                    format!("target {target} compiled_first={compiled_first} {pattern:?}");
                let mut visits = 0;
                for at in 0..(FRAMES - output(0)) as u64 {
                    let expected = if plan_at(at) == target {
                        visits += 1;
                        1.0 + sounding(&spans, at) as f32
                    } else {
                        0.0
                    };
                    let found = driver.samples[output(at)];
                    assert!(
                        (found - expected).abs() < 0.00001,
                        "{context} engine {at}: {found}, expected {expected}"
                    );
                }
                assert!(visits >= wraps().len(), "{context}: {visits} visits");
                match &first {
                    None => first = Some(driver.samples),
                    Some(first) => assert_eq!(&driver.samples, first, "{context}"),
                }
            }
        }
    }
}

#[test]
fn panic_before_at_and_after_a_wrap_ends_exactly_the_notes_sounding_at_its_sample() {
    // The wrap at 246 ends keys 64 and 66 and starts the next pass's key 62. A Panic one
    // frame earlier tombstones both, which the wrap frees without an `Off`; a Panic at
    // the wrap runs after the wrap and the pass's onset there, so key 62 gets a
    // zero-length gate; one frame later it sounds one frame. Then its own `Off` is
    // consumed.
    const WRAP: u64 = 246;
    for panic in [WRAP - 1, WRAP, WRAP + 1] {
        let spans = compiled_spans()
            .into_iter()
            .map(|(on, off)| {
                if on <= panic && panic < off {
                    (on, panic)
                } else {
                    (on, off)
                }
            })
            .collect::<Vec<_>>();
        for compiled_first in [true, false] {
            let mut first = None;
            for pattern in PARTITIONS {
                let prepared = bound(loop_plan(compiled_first, constant(), Seconds::ZERO));
                let mut driver = Driver::new(prepared, &mixed_profile(), pattern);
                let _ = driver
                    .control
                    .submit_panic(MixedIngressOriginId(7), SampleTime::new(panic))
                    .expect("the protected place is free");
                let (driver, _, _) = driver.run();
                let context = format!("panic {panic} compiled_first={compiled_first} {pattern:?}");
                for at in 0..(FRAMES - output(0)) as u64 {
                    let expected = sounding(&spans, at) as f32 * LEVEL;
                    let found = driver.samples[output(at)];
                    assert!(
                        (found - expected).abs() < 0.00001,
                        "{context} engine {at}: {found}, expected {expected}"
                    );
                }
                let report = driver.audio.report();
                assert!(!report.faulted, "{context}: {:?}", report.fault);
                let ended = if panic < WRAP { 2 } else { 1 };
                assert_eq!(report.compiled_panic_ended, HeldNoteCount::measured(ended));
                assert_eq!(
                    report.compiled_released_after_panic,
                    EventCount::measured(ended)
                );
                assert_eq!(report.compiled_panic_tombstones, HeldNoteCount::NONE);
                let wrap_releases = 2 * wraps().len() - 1 - if panic < WRAP { 2 } else { 0 };
                assert_eq!(
                    report.loop_wrap_releases.as_usize(),
                    Some(wrap_releases),
                    "{context}"
                );
                match &first {
                    None => first = Some(driver.samples),
                    Some(first) => assert_eq!(&driver.samples, first, "{context}"),
                }
            }
        }
    }
}

#[test]
fn loop_admission_is_refused_one_session_credit_short_of_the_entry_quantum() {
    for compiled_first in [true, false] {
        let plan = || loop_plan(compiled_first, constant(), Seconds::ZERO);
        let groups = restoration_groups(&arm(bound(plan()), &mixed_profile()).1);
        // The entry quantum holds the boundary release and restoration at 128, the
        // first wrap at 135 ending key 66, and the second at 172 ending keys 64 and 66.
        let needed = u32::try_from(3 * (groups + 1) + 3).expect("small");

        let short = profile_with_session_share(EventCount::measured(needed - 1));
        let (control, audio) = arm(bound(plan()), &short);
        let before = audio.report();
        let candidate = control
            .prepare_loop(
                SampleTime::new(ENTRY),
                PlanPosition::new(ENTRY_AT),
                interval(),
                control.in_force,
            )
            .expect("loop candidate");
        let refusal = control
            .seal_loop_offer(candidate, SampleTime::new(ENTRY), MixedOfferKind::Seek)
            .expect_err("one credit short");
        assert!(
            matches!(
                refusal.0,
                MixedOfferPrepareError::LoopAdmission {
                    class: ProducerClass::Session,
                    ..
                }
            ),
            "{:?}",
            refusal.0
        );
        // Nothing changed: the same control still seals and offers a plain seek.
        assert_eq!(audio.report(), before);
        let mut driver = Driver {
            control,
            audio,
            samples: vec![0.0; FRAMES],
            rendered: 0,
            pattern: &[64],
            index: 0,
            left: 0,
        };
        driver.offer_seek(ENTRY, PlanPosition::new(ENTRY_AT));

        // The exact share is admitted, and the entry quantum spends all of it: admission
        // is tight against what the wraps charge.
        let exact = profile_with_session_share(EventCount::measured(needed));
        let (driver, _, _) = Driver::new(bound(plan()), &exact, &[64]).run();
        let report = driver.audio.report();
        assert!(!report.faulted, "{:?}", report.fault);
        assert_eq!(report.loop_wraps.as_usize(), Some(wraps().len()));
        assert_eq!(
            driver.audio.arbiter.high_water(ProducerClass::Session),
            EventCount::measured(needed)
        );
    }
}

#[test]
fn a_stop_while_looping_holds_the_loop_position_and_suspends_wraps() {
    const STOP: u64 = 384;
    let spans = compiled_spans()
        .into_iter()
        .filter(|(on, _)| *on < STOP)
        .map(|(on, off)| (on, off.min(STOP)))
        .collect::<Vec<_>>();
    for compiled_first in [true, false] {
        for pattern in PARTITIONS {
            let prepared = bound(loop_plan(compiled_first, constant(), Seconds::ZERO));
            let mut driver = Driver::new(prepared, &mixed_profile(), pattern);
            driver.offer_loop(ENTRY, MixedOfferKind::Seek);
            assert_eq!(
                driver.audio.schedule_compiled_stop(SampleTime::new(STOP)),
                Ok(MixedStopAccepted::Scheduled)
            );
            driver.render_to(output(EXIT));
            let context = format!("compiled_first={compiled_first} {pattern:?}");
            let report = driver.audio.report();
            // The loop plays 200 + (30 + 256) mod 37 there; the entry anchor alone
            // would give 486.
            assert_eq!(
                report.transport,
                MixedCompiledTransport::Stopped {
                    at: SampleTime::new(STOP),
                    released: HeldNoteCount::measured(1),
                    position: Some(PlanPosition::new(227)),
                },
                "{context}"
            );
            let before = wraps().iter().filter(|wrap| **wrap < STOP).count();
            assert_eq!(report.loop_wraps.as_usize(), Some(before), "{context}");
            assert_eq!(
                report.loop_pass.map(LoopPassId::as_u64),
                Some(1 + before as u64),
                "{context}"
            );
            for at in 0..EXIT {
                let expected = sounding(&spans, at) as f32 * LEVEL;
                let found = driver.samples[output(at)];
                assert!(
                    (found - expected).abs() < 0.00001,
                    "{context} engine {at}: {found}, expected {expected}"
                );
            }
        }
    }
}

#[test]
fn a_loop_entry_outside_its_interval_is_refused_and_its_end_enters_at_the_start() {
    let (control, _audio) = arm(
        bound(loop_plan(true, constant(), Seconds::ZERO)),
        &mixed_profile(),
    );
    let prepare = |position| {
        control.prepare_loop(
            SampleTime::new(ENTRY),
            PlanPosition::new(position),
            interval(),
            control.in_force,
        )
    };
    for outside in [START - 1, END + 1] {
        assert_eq!(
            prepare(outside).map(|_| ()).expect_err("outside"),
            MixedSubsequentPrepareError::LoopEntry {
                position: PlanPosition::new(outside),
                start: PlanPosition::new(START),
                end: PlanPosition::new(END),
            }
        );
    }
    let at_end = prepare(END).expect("the end enters at the start");
    assert_eq!(at_end.entry.anchor().position(), PlanPosition::new(START));
    // The releases at 400 lie past the loop: no pass omits anything, and a pass from
    // the start leaves keys 64 and 66 open at the end.
    assert_eq!(
        at_end.pass_omissions(),
        (EventCount::NONE, EventCount::NONE)
    );
    assert_eq!(at_end.program.end_open, HeldNoteCount::measured(2));
}
