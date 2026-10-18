//! The private mixed owner's plan timeline while its compiled program is stopped
//! (draft rule 2.3, ADR-0061's frozen stopped position).
//!
//! A Stop holds every later quantum at the plan position it found, and a seek while
//! stopped moves the held position to its destination; engine time, live notes and
//! release tails keep advancing in the same audio owner. A voice-scope Impulse under a
//! held live note makes the plan position audible: it clicks once, at the frame that
//! visits its position. A timeline that advanced while stopped would click inside the
//! stopped span, and one that resumed elsewhere would click at the wrong frame.

use super::history_tests::{key, mixed_profile};
use super::*;
use crate::compile::{RenderConfig, compile};
use crate::identity::Resolution;
use crate::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, NoteProducerDeclaration, PlanDeclarations, PortId,
    SignalDomain,
};
use crate::plan::CompiledPlan;
use crate::quantities::{
    Amplitude, ChannelLayout, Frequency, NormalizedLevel, NoteVelocity, Seconds,
};
use crate::render::{AudioBlockMut, NoteEdge, Renderer, TimedEvents};
use crate::schedule::PlanEvent;
use crate::time::{QUANTUM_FRAMES, TimeSource as TestOrigin};

const SOURCE: NodeId = NodeId::new(1);
const ENVELOPE: NodeId = NodeId::new(2);
const AMPLIFIER: NodeId = NodeId::new(3);
const OUTPUT: NodeId = NodeId::new(4);

/// The armed transition adopts here at plan 10, so engine `t` plays plan `t - 54`
/// until a transport change.
const ARMED: SampleTime = SampleTime::new(64);
const STOP: SampleTime = SampleTime::new(192);
/// The plan position the running anchor gives the Stop boundary.
const HELD: u64 = 138;
const SEEK: SampleTime = SampleTime::new(320);
const SEEK_TO: u64 = 1000;
const FRAMES: usize = 640;

/// Output index `i` holds engine sample `i - 64`: the first output quantum is the carry.
const fn output(at: SampleTime) -> usize {
    at.as_u64() as usize + QUANTUM_FRAMES as usize
}

const PARTITIONS: [&[usize]; 4] = [
    &[512, 128],
    &[64; 10],
    &[8; 80],
    &[7, 63, 90, 1, 256, 95, 128],
];

/// One voice: `source` through a gated amplifier, with the given release.
fn voice_plan(compiled_first: bool, source: IrNodeKind, release: Seconds) -> CompiledPlan {
    let compiled = NoteProducerDeclaration {
        compiled: true,
        simultaneous_notes: HeldNoteCount::measured(2),
        simultaneous_holds: EventCount::NONE,
    };
    let live = NoteProducerDeclaration {
        compiled: false,
        simultaneous_notes: HeldNoteCount::measured(2),
        simultaneous_holds: EventCount::measured(2),
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
            held_notes: HeldNoteCount::measured(4),
            ..PlanDeclarations::default()
        })
        .build()
        .expect("voice graph");
    compile(&ir, &RenderConfig::new(mixed_profile()))
        .into_plan()
        .expect("plan")
}

/// The bound owner over `plan`, with compiled key 60 from plan 10 to plan 300: it
/// sounds from the armed adoption across the Stop, which ends it.
fn bound(plan: CompiledPlan) -> MixedJoinedPrepared {
    let note = plan.resolve_note(ENVELOPE).expect("playable envelope");
    let events = vec![
        PlanEvent::new(
            PlanPosition::new(10),
            CompiledPayload::NoteOn {
                slot: note,
                key: key(60),
                velocity: NoteVelocity::FULL,
            },
        ),
        PlanEvent::new(
            PlanPosition::new(300),
            CompiledPayload::NoteOff {
                slot: note,
                key: key(60),
            },
        ),
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

fn arm(prepared: MixedJoinedPrepared) -> (MixedOneShotControl, MixedOneShotAudio) {
    let history = prepared
        .prepare_history(ARMED, PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm")
}

/// The transport scripts. Every one stops at 192.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Script {
    /// Start at 320 from the position the Stop holds.
    StopStart,
    /// Seek while stopped at 320 to plan 1000, then Start at 448 from the held seek.
    StoppedSeekStart,
}

/// One armed owner driven over a repeating callback partition; a script point splits
/// the callback that crosses it.
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
    fn new(prepared: MixedJoinedPrepared, pattern: &'static [usize]) -> Self {
        let (control, audio) = arm(prepared);
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

    fn offer(&mut self, at: SampleTime, position: PlanPosition, kind: MixedOfferKind) {
        let candidate = self
            .control
            .prepare_subsequent(at, position, self.control.in_force)
            .expect("later candidate");
        let offer = self
            .control
            .seal_offer(candidate, at, kind)
            .expect("sealed offer");
        self.audio
            .offer_candidate(offer)
            .map_err(|(reason, _)| reason)
            .expect("offer");
    }

    fn collect_adopted(&mut self) {
        let retired = self.audio.take_retired_offer().expect("retired offer");
        let receipt = self.control.collect_offer(retired).expect("collected");
        assert!(
            matches!(receipt.outcome, MixedOfferOutcome::Adopted(_)),
            "{receipt:?}"
        );
    }

    /// The plan position the stopped program holds, as its report gives it.
    fn held(&self) -> Option<PlanPosition> {
        match self.audio.report().transport {
            MixedCompiledTransport::Stopped { position, .. } => position,
            MixedCompiledTransport::Running => None,
        }
    }

    /// Run `script`, recording the held position after the Stop and, for the seek
    /// script, after the stopped seek. The Start resumes from what the report holds.
    fn run(mut self, script: Script) -> (Self, Vec<Option<PlanPosition>>) {
        assert_eq!(
            self.audio.schedule_compiled_stop(STOP),
            Ok(MixedStopAccepted::Scheduled)
        );
        let mut held = Vec::new();
        self.render_to(output(SEEK));
        held.push(self.held());
        let start = match script {
            Script::StopStart => SEEK,
            Script::StoppedSeekStart => {
                self.offer(SEEK, PlanPosition::new(SEEK_TO), MixedOfferKind::Seek);
                let start = SampleTime::new(448);
                self.render_to(output(start));
                held.push(self.held());
                self.collect_adopted();
                start
            }
        };
        let position = self.held().expect("a held position");
        self.offer(start, position, MixedOfferKind::Start);
        self.render_to(FRAMES);
        self.collect_adopted();
        assert_eq!(
            self.audio.report().transport,
            MixedCompiledTransport::Running
        );
        (self, held)
    }
}

/// The output frames that are not silent.
fn clicks(samples: &[f32]) -> Vec<usize> {
    samples
        .iter()
        .enumerate()
        .filter(|(_, sample)| **sample != 0.0)
        .map(|(frame, _)| frame)
        .collect()
}

/// An owner whose voices play an Impulse at plan `target`, with live key 48 held from
/// engine 0 for the whole run through the private ingress store.
fn impulse_run(
    compiled_first: bool,
    target: u64,
    script: Script,
    pattern: &'static [usize],
) -> (Driver, Vec<Option<PlanPosition>>) {
    let prepared = bound(voice_plan(
        compiled_first,
        IrNodeKind::Impulse {
            position: PlanPosition::new(target),
        },
        Seconds::ZERO,
    ));
    let mut driver = Driver::new(prepared, pattern);
    let held = driver
        .audio
        .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("held live onset");
    let (driver, positions) = driver.run(script);
    // The live note was never ended: the owner that held it is the owner that resumed.
    assert_eq!(
        driver.audio.audio.live_minter.resolve(held),
        Resolution::Live
    );
    (driver, positions)
}

#[test]
fn a_stopped_program_holds_the_plan_timeline_until_start_resumes_it() {
    // (script, Impulse position, the only click: its engine sample and how many voices
    // sound there). Compiled key 60 sounds from 64 to the Stop at 192, so a click at
    // plan 100 sums both producers; after the Stop only live key 48 sounds.
    // Positions the running anchor would visit while stopped click only after Start:
    // the held one exactly at Start's frame, and one 62 frames on 62 frames after it.
    // A seek while stopped is not a resume, so its destination clicks at Start too.
    let cases = [
        (Script::StopStart, 100, Some((154, 2.0))),
        (Script::StopStart, HELD, Some((320, 1.0))),
        (Script::StopStart, 200, Some((382, 1.0))),
        (Script::StoppedSeekStart, 100, Some((154, 2.0))),
        (Script::StoppedSeekStart, HELD, None),
        (Script::StoppedSeekStart, 200, None),
        (Script::StoppedSeekStart, SEEK_TO, Some((448, 1.0))),
        (Script::StoppedSeekStart, SEEK_TO + 30, Some((478, 1.0))),
    ];
    for compiled_first in [true, false] {
        for (script, target, click) in cases {
            let mut first = None;
            for pattern in PARTITIONS {
                let (driver, held) = impulse_run(compiled_first, target, script, pattern);
                let expected = match script {
                    Script::StopStart => vec![Some(PlanPosition::new(HELD))],
                    Script::StoppedSeekStart => vec![
                        Some(PlanPosition::new(HELD)),
                        Some(PlanPosition::new(SEEK_TO)),
                    ],
                };
                assert_eq!(held, expected, "{script:?} {pattern:?}");
                let samples = driver.samples;
                let found = clicks(&samples);
                let wanted = click
                    .map(|(at, _)| vec![output(SampleTime::new(at))])
                    .unwrap_or_default();
                let context = format!(
                    "compiled_first={compiled_first} {script:?} target={target} {pattern:?}"
                );
                assert_eq!(found, wanted, "{context}");
                if let Some((at, voices)) = click {
                    let level = samples[output(SampleTime::new(at))];
                    assert!((level - voices).abs() < 0.00001, "{context}: {level}");
                }
                match &first {
                    None => first = Some(samples),
                    Some(first) => assert_eq!(&samples, first, "{context}"),
                }
            }
        }
    }
}

/// A renderer of the same plan, epoch, table and partition that hears only the live
/// events it is given, in quantum-sized calls. Its timeline runs linearly throughout.
struct Reference(PreparedRenderer);

impl Reference {
    fn of(prepared: &MixedJoinedPrepared) -> Self {
        let mut renderer = PreparedRenderer::prepare(
            Arc::clone(&prepared.owner.control.plan),
            prepared.owner.control.anchor,
            prepared.epoch(),
            prepared.table_id(),
        )
        .expect("reference renderer");
        assert!(renderer.bind_mixed_partition(Arc::clone(&prepared.owner.control.partition)));
        Self(renderer)
    }

    fn render(mut self, events: &[TimedEvent]) -> Vec<f32> {
        let mut samples = vec![0.0_f32; FRAMES];
        for chunk in samples.chunks_mut(QUANTUM_FRAMES as usize) {
            let start = self.0.clock().as_u64();
            let quanta = self.0.quanta_needed_for(chunk.len()) as u64;
            let end = start + quanta * u64::from(QUANTUM_FRAMES);
            let due = events
                .iter()
                .copied()
                .filter(|event| (start..end).contains(&event.envelope().time().as_u64()))
                .collect::<Vec<_>>();
            let frames = chunk.len();
            self.0
                .render(
                    AudioBlockMut::new(chunk, frames, ChannelLayout::Mono)
                        .expect("reference block"),
                    TimedEvents::new(&due),
                )
                .expect("reference quantum");
        }
        samples
    }
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

/// The released note's tail lasts 192 frames at the harness's 48 kHz: from its
/// release at 256 to 448, past the Start at 320.
const TAIL_FROM: SampleTime = SampleTime::new(256);
const TAIL_FRAMES: usize = 192;

fn tail_prepared(compiled_first: bool) -> MixedJoinedPrepared {
    bound(voice_plan(
        compiled_first,
        IrNodeKind::Sine {
            frequency: Frequency::new(220.0).expect("finite frequency"),
            amplitude: Amplitude::new(0.8).expect("finite amplitude"),
        },
        Seconds::new(TAIL_FRAMES as f32 / 48_000.0).expect("finite release"),
    ))
}

struct TailRun {
    mixed: Vec<f32>,
    /// Both live keys, rendered by a live-only renderer.
    live_only: Vec<f32>,
    /// Key 48 alone: the control that isolates key 49's tail.
    held_only: Vec<f32>,
}

/// Live key 48 is held from 0 to 560 across Stop and Start; key 49 sounds from 128
/// and is released at 256, while stopped, so its tail starts under the held timeline.
/// With `with_live` false the run is compiled-only.
fn tail_run(compiled_first: bool, pattern: &'static [usize], with_live: bool) -> TailRun {
    let prepared = tail_prepared(compiled_first);
    let epoch = prepared.epoch();
    let live_only = Reference::of(&prepared);
    let held_only = Reference::of(&prepared);
    let mut driver = Driver::new(prepared, pattern);
    let live = with_live.then(|| {
        let audio = &mut driver.audio;
        let held = audio
            .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("held onset");
        let tail = audio
            .offer_test_note_on(SampleTime::new(128), key(49), NoteVelocity::FULL)
            .expect("tail onset");
        audio
            .offer_live_note_off(TAIL_FROM, tail)
            .expect("release while stopped");
        audio
            .offer_live_note_off(SampleTime::new(560), held)
            .expect("held release");
        (held, tail)
    });
    let pairing = |audio: &MixedOneShotAudio| {
        let renderer = &audio.audio.renderer;
        (renderer.plan().id(), renderer.epoch(), renderer.table_id())
    };
    let before = pairing(&driver.audio);
    let (driver, held) = driver.run(Script::StopStart);
    assert_eq!(held, vec![Some(PlanPosition::new(HELD))]);
    assert_eq!(
        pairing(&driver.audio),
        before,
        "the owner kept its renderer"
    );
    let slot = driver.audio.audio.note;
    let (live_only, held_only) = match live {
        Some((held, tail)) => (
            live_only.render(&[
                live_on(epoch, 0, held, slot, 48),
                live_on(epoch, 128, tail, slot, 49),
                live_edge(epoch, TAIL_FROM.as_u64(), tail, NoteEdge::Off),
                live_edge(epoch, 560, held, NoteEdge::Off),
            ]),
            held_only.render(&[
                live_on(epoch, 0, held, slot, 48),
                live_edge(epoch, 560, held, NoteEdge::Off),
            ]),
        ),
        None => (vec![0.0; FRAMES], vec![0.0; FRAMES]),
    };
    TailRun {
        mixed: driver.samples,
        live_only,
        held_only,
    }
}

#[test]
fn live_notes_and_release_tails_continue_under_the_held_timeline() {
    for compiled_first in [true, false] {
        let mut first = None;
        for pattern in PARTITIONS {
            let run = tail_run(compiled_first, pattern, true);
            let compiled = tail_run(compiled_first, pattern, false).mixed;
            let context = format!("compiled_first={compiled_first} {pattern:?}");
            // The mixed output is the compiled-only run plus a live-only renderer of
            // the same plan, before, across and after the stopped span.
            for (frame, ((mixed, compiled), live)) in run
                .mixed
                .iter()
                .zip(&compiled)
                .zip(&run.live_only)
                .enumerate()
            {
                assert!(
                    (mixed - (compiled + live)).abs() < 0.00001,
                    "{context} frame {frame}: mixed {mixed}, compiled {compiled}, live {live}"
                );
            }
            // Key 49's release tail sounds while stopped and across Start, then ends.
            let tail = |at: usize| (run.live_only[at] - run.held_only[at]).abs();
            let from = output(TAIL_FROM);
            assert!(
                (from..output(STOP) + 64 * 3).any(|at| tail(at) > 0.01),
                "{context}"
            );
            assert!(
                (output(SEEK)..output(SEEK) + 64).any(|at| tail(at) > 0.001),
                "{context}: the tail must outlast the stopped span"
            );
            assert!(
                (from + TAIL_FRAMES + 64..FRAMES).all(|at| tail(at) < 0.00001),
                "{context}"
            );
            // Compiled key 60's gate fell at the Stop; its tail keeps sounding.
            assert!(
                compiled[output(STOP)..output(STOP) + 64]
                    .iter()
                    .any(|sample| sample.abs() > 0.01),
                "{context}"
            );
            match &first {
                None => first = Some(run.mixed),
                Some(first) => assert_eq!(&run.mixed, first, "{context}"),
            }
        }
    }
}
