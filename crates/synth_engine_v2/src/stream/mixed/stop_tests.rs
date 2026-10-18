//! A compiled-scoped Stop on the private mixed audio owner (draft S2, D1, rule 2.3).
//!
//! Stop ends only compiled custody at its quantum boundary. A live note held across it,
//! or played while stopped, must render sample-for-sample as a live-only renderer of the
//! same plan renders it, and no compiled edge may publish after the boundary.

use super::history_tests::{bound_with_events, key, mixed_profile};
use super::*;
use crate::quantities::{ChannelLayout, NoteVelocity};
use crate::render::{AudioBlockMut, NoteEdge, Renderer, TimedEvents};
use crate::schedule::PlanEvent;
use crate::time::{QUANTUM_FRAMES, TimeSource as TestOrigin};

/// The engine sample at which every Stop below applies. Output index `i` holds
/// engine sample `i - 64`, because the first output quantum is the primed carry.
const STOP: SampleTime = SampleTime::new(192);
const STOP_OUTPUT: usize = 256;
const FRAMES: usize = 640;

/// Compiled key 60 sounds from the adoption at 64 across the Stop; its release
/// (engine 354) and the later key-72 pair (454, 554) must never publish once stopped.
fn stop_fixture(compiled_first: bool) -> (MixedJoinedPrepared, MixedStampedCandidate) {
    let prepared = bound_with_events(compiled_first, |note| {
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
    });
    let history = prepared
        .prepare_history(SampleTime::new(64), PlanPosition::new(10))
        .expect("history");
    let suffix = prepared.prepare_suffix(history).expect("suffix");
    let candidate = prepared.stamp_suffix(suffix).expect("stamp");
    (prepared, candidate)
}

fn live_on(
    epoch: StreamEpoch,
    at: u64,
    identity: NoteIdentity,
    slot: NoteSlot,
    raw: u8,
) -> TimedEvent {
    TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::new(at), TestOrigin::Simulated),
        EventPayload::Note {
            identity,
            edge: NoteEdge::On {
                slot,
                key: key(raw),
                velocity: NoteVelocity::FULL,
            },
        },
    )
}

/// A renderer bound to the same plan and partition that hears only `events`.
fn live_only_reference(prepared: &MixedJoinedPrepared) -> PreparedRenderer {
    let mut reference = PreparedRenderer::prepare(
        Arc::clone(&prepared.owner.control.plan),
        prepared.owner.control.anchor,
        prepared.epoch(),
        prepared.table_id(),
    )
    .expect("reference renderer");
    assert!(reference.bind_mixed_partition(Arc::clone(&prepared.owner.control.partition)));
    reference
}

/// Render `FRAMES` of the reference in quantum-sized calls, handing each call only
/// the events inside the quanta it renders.
fn render_reference(reference: &mut PreparedRenderer, events: &[TimedEvent]) -> Vec<f32> {
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

/// Render the owner over `partition`, asserting every callback allocates nothing.
fn render_owner(audio: &mut MixedOneShotAudio, partition: &[usize]) -> Vec<f32> {
    assert_eq!(partition.iter().sum::<usize>(), FRAMES);
    let mut samples = vec![0.0_f32; FRAMES];
    let mut offset = 0;
    for &frames in partition {
        let output = AudioBlockMut::new(
            &mut samples[offset..offset + frames],
            frames,
            ChannelLayout::Mono,
        )
        .expect("mono block");
        let allocations = crate::render_allocation::count_allocs(|| {
            audio.render_private(output).expect("private render");
        });
        assert_eq!(allocations, 0, "callback allocated or deallocated");
        offset += frames;
    }
    samples
}

struct StopRun {
    samples: Vec<f32>,
    live_only: Vec<f32>,
    report: MixedOneShotRenderReport,
    audio: MixedOneShotAudio,
    live: NoteIdentity,
}

fn stop_run(compiled_first: bool, partition: &[usize], stop: bool) -> StopRun {
    let (prepared, candidate) = stop_fixture(compiled_first);
    let mut reference = live_only_reference(&prepared);
    let epoch = prepared.epoch();
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let live = audio
        .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("test live onset");
    if stop {
        assert_eq!(
            audio.schedule_compiled_stop(STOP),
            Ok(MixedStopAccepted::Scheduled)
        );
    }
    let samples = render_owner(&mut audio, partition);
    let live_only = render_reference(
        &mut reference,
        &[live_on(epoch, 0, live, audio.audio.note, 48)],
    );
    StopRun {
        samples,
        live_only,
        report: audio.report(),
        audio,
        live,
    }
}

const PARTITIONS: [&[usize]; 4] = [
    &[512, 128],
    &[64; 10],
    &[8; 80],
    &[7, 63, 90, 1, 256, 95, 128],
];

#[test]
fn compiled_stop_keeps_a_held_live_note_and_publishes_no_later_compiled_edge() {
    for compiled_first in [true, false] {
        let reference = stop_run(compiled_first, PARTITIONS[0], true);
        let report = reference.report;
        assert!(report.adopted);
        assert!(!report.faulted);
        assert_eq!(
            report.transport,
            MixedCompiledTransport::Stopped {
                at: STOP,
                released: HeldNoteCount::measured(1),
                position: Some(PlanPosition::new(138)),
            }
        );
        assert_eq!(report.stops_charged, EventCount::measured(1));
        assert_eq!(report.stop_pending, None);
        // Compiled key 60 sounded before the Stop; afterwards the output is exactly
        // the live-only rendering, so the live gate, rows, ramps and DSP are untouched.
        assert_ne!(
            &reference.samples[128..STOP_OUTPUT],
            &reference.live_only[128..STOP_OUTPUT]
        );
        assert_eq!(
            &reference.samples[STOP_OUTPUT..],
            &reference.live_only[STOP_OUTPUT..]
        );
        assert!(reference.live_only[STOP_OUTPUT..].iter().any(|s| *s != 0.0));

        let audio = &reference.audio;
        // Only the adoption's restoration and key 60's onset were consumed.
        let restoration = audio.capsule.restoration_count.as_usize().expect("count");
        assert_eq!(audio.next, restoration + 1);
        assert!(audio.audio.compiled_minter.live_identities().is_empty());
        assert!(
            audio
                .audio
                .compiled_tokens
                .iter()
                .all(|token| *token == super::MixedCompiledTokenState::Free)
        );
        let sounding = audio
            .audio
            .renderer
            .snapshot_mixed_sounding()
            .expect("bound registry");
        assert!(sounding.compiled().is_empty());
        assert_eq!(
            sounding
                .live()
                .iter()
                .map(|n| n.identity)
                .collect::<Vec<_>>(),
            vec![reference.live]
        );
        assert_eq!(
            audio.audio.live_minter.resolve(reference.live),
            Resolution::Live
        );
        assert_eq!(audio.audio.renderer.seeded_row_count(), 0);
        // The adoption ended the initial list's key 60; the Stop ended the suffix's.
        // Each keeps its own record, so teardown still reads the adoption's.
        assert_eq!(report.released_compiled, HeldNoteCount::measured(1));
        let adopted = audio.audio.compiled_ended[0].expect("adoption record");
        let stopped = audio.stop_ended[0].expect("Stop record");
        assert_ne!(adopted.identity, stopped.identity);

        for partition in &PARTITIONS[1..] {
            let run = stop_run(compiled_first, partition, true);
            assert_eq!(run.samples, reference.samples, "{partition:?}");
            assert_eq!(run.report, report, "{partition:?}");
        }

        // Falsifier control: without the Stop, compiled key 60 keeps sounding.
        let running = stop_run(compiled_first, PARTITIONS[0], false);
        assert_eq!(running.report.transport, MixedCompiledTransport::Running);
        assert_eq!(running.report.stops_charged, EventCount::NONE);
        assert_ne!(
            &running.samples[STOP_OUTPUT..],
            &running.live_only[STOP_OUTPUT..]
        );
        assert_eq!(running.audio.next, running.audio.events.len());
    }
}

#[test]
fn compiled_stop_seeds_only_the_rows_its_boundary_writes_spend() {
    let (prepared, candidate) = stop_fixture(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    assert_eq!(
        audio.schedule_compiled_stop(STOP),
        Ok(MixedStopAccepted::Scheduled)
    );
    let mut samples = [0.0_f32; QUANTUM_FRAMES as usize];
    // Carry, then quanta 0 to 2: the clock reaches the Stop boundary.
    for _ in 0..4 {
        let output = AudioBlockMut::new(&mut samples, QUANTUM_FRAMES as usize, ChannelLayout::Mono)
            .expect("block");
        audio.render_private(output).expect("before Stop");
    }
    assert_eq!(audio.audio.renderer.clock(), STOP);
    assert_eq!(audio.audio.renderer.seeded_row_count(), 0);
    let anchor = audio.audio.renderer.mixed_anchor();
    let output = AudioBlockMut::new(&mut samples, QUANTUM_FRAMES as usize, ChannelLayout::Mono)
        .expect("block");
    audio.render_private(output).expect("Stop quantum");
    assert!(matches!(
        audio.report().transport,
        MixedCompiledTransport::Stopped { .. }
    ));
    // An adoption-scoped seed would leave every unwritten compiled row marked.
    assert_eq!(audio.audio.renderer.seeded_row_count(), 0);
    assert_eq!(audio.audio.renderer.mixed_anchor(), anchor);
}

#[test]
fn renderer_stop_marks_only_its_queued_rows_and_spends_them_at_the_boundary() {
    let (prepared, candidate) = stop_fixture(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let mut samples = [0.0_f32; QUANTUM_FRAMES as usize];
    for _ in 0..4 {
        let output = AudioBlockMut::new(&mut samples, QUANTUM_FRAMES as usize, ChannelLayout::Mono)
            .expect("block");
        audio.render_private(output).expect("before Stop");
    }
    let producer = audio.audio.partition.compiled_producer();
    let renderer = &mut audio.audio.renderer;
    assert_eq!(renderer.seeded_row_count(), 0);
    assert_eq!(
        renderer.stop_mixed_compiled_boundary(
            producer,
            SampleTime::new(256),
            &mut audio.stop_ended
        ),
        Err(crate::render::MixedBoundaryReleaseError::ClockMismatch {
            clock: STOP,
            offered: SampleTime::new(256),
        })
    );
    assert_eq!(renderer.seeded_row_count(), 0);
    assert_eq!(
        renderer.stop_mixed_compiled_boundary(producer, STOP, &mut audio.stop_ended),
        Ok(HeldNoteCount::measured(1))
    );
    // The released note's gate (and any trigger) rows, not the compiled partition.
    let marked = renderer.seeded_row_count();
    assert!(marked > 0);
    assert!(marked < audio.audio.partition.compiled_rows().len());
    let output = AudioBlockMut::new(&mut samples, QUANTUM_FRAMES as usize, ChannelLayout::Mono)
        .expect("block");
    renderer
        .render(output, TimedEvents::new(&[]))
        .expect("boundary quantum");
    assert_eq!(renderer.seeded_row_count(), 0);
}

#[test]
fn repeated_compiled_stop_is_acknowledged_and_changes_nothing() {
    for compiled_first in [true, false] {
        let single = stop_run(compiled_first, PARTITIONS[1], true);
        let (prepared, candidate) = stop_fixture(compiled_first);
        let (_, mut audio) = prepared
            .arm_one_shot(candidate, &mixed_profile())
            .expect("private arm");
        let live = audio
            .arm_test_live_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
            .expect("test live onset");
        assert_eq!(
            audio.schedule_compiled_stop(STOP),
            Ok(MixedStopAccepted::Scheduled)
        );
        assert_eq!(
            audio.schedule_compiled_stop(SampleTime::new(128)),
            Err(MixedStopRefused::Pending { pending: STOP })
        );
        assert_eq!(audio.report().transport, MixedCompiledTransport::Running);
        assert_eq!(audio.report().stop_pending, Some(STOP));
        let mut samples = vec![0.0_f32; FRAMES];
        for (index, chunk) in samples.chunks_mut(64).enumerate() {
            if index == 6 {
                let before = audio.report();
                assert!(matches!(
                    before.transport,
                    MixedCompiledTransport::Stopped { .. }
                ));
                let clock = audio.audio.renderer.clock();
                assert_eq!(
                    audio.schedule_compiled_stop(clock),
                    Ok(MixedStopAccepted::AlreadyStopped)
                );
                assert_eq!(audio.report(), before);
            }
            let frames = chunk.len();
            audio
                .render_private(
                    AudioBlockMut::new(chunk, frames, ChannelLayout::Mono).expect("block"),
                )
                .expect("callback");
        }
        assert_eq!(samples, single.samples);
        assert_eq!(audio.report(), single.report);
        assert_eq!(audio.audio.live_minter.resolve(live), Resolution::Live);
    }
}

#[test]
fn compiled_stop_refusals_change_no_state() {
    let (prepared, candidate) = stop_fixture(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let effective = audio.timing.effective();
    for (at, refused) in [
        (
            SampleTime::new(200),
            MixedStopRefused::NotBoundary {
                at: SampleTime::new(200),
            },
        ),
        (
            SampleTime::ZERO,
            MixedStopRefused::ActivationPending {
                at: SampleTime::ZERO,
                effective,
            },
        ),
        (
            effective,
            MixedStopRefused::ActivationPending {
                at: effective,
                effective,
            },
        ),
    ] {
        assert_eq!(audio.schedule_compiled_stop(at), Err(refused));
        assert_eq!(audio.report().transport, MixedCompiledTransport::Running);
        assert_eq!(audio.report().stop_pending, None);
    }
    let mut samples = [0.0_f32; 512];
    let output = AudioBlockMut::new(&mut samples, 512, ChannelLayout::Mono).expect("block");
    audio.render_private(output).expect("running");
    let clock = audio.audio.renderer.clock();
    assert_eq!(
        audio.schedule_compiled_stop(STOP),
        Err(MixedStopRefused::Past { at: STOP, clock })
    );
    assert_eq!(audio.report().transport, MixedCompiledTransport::Running);

    audio.fault = Some(MixedOneShotRenderError::RendererFaulted);
    assert_eq!(
        audio.schedule_compiled_stop(clock),
        Err(MixedStopRefused::Faulted)
    );
    assert_eq!(audio.report().transport, MixedCompiledTransport::Running);
}

/// Live onset 48 is held across the Stop and released while stopped; onset 49 lands
/// on the Stop boundary and ends while stopped. Both go through the private ingress store.
#[cfg(feature = "simulated-ingress")]
fn ingress_stop_run(compiled_first: bool, partition: &[usize]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let (prepared, candidate) = stop_fixture(compiled_first);
    let mut reference = live_only_reference(&prepared);
    let mut held_only = live_only_reference(&prepared);
    let epoch = prepared.epoch();
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    let held = audio
        .offer_test_note_on(SampleTime::ZERO, key(48), NoteVelocity::FULL)
        .expect("held onset");
    let stopped = audio
        .offer_test_note_on(STOP, key(49), NoteVelocity::FULL)
        .expect("onset while stopped");
    audio
        .offer_live_note_off(SampleTime::new(400), stopped)
        .expect("release while stopped");
    audio
        .offer_live_note_off(SampleTime::new(480), held)
        .expect("held release while stopped");
    assert_eq!(
        audio.schedule_compiled_stop(STOP),
        Ok(MixedStopAccepted::Scheduled)
    );
    let slot = audio.audio.note;
    let off = |at, identity| {
        TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::new(at), TestOrigin::Simulated),
            EventPayload::Note {
                identity,
                edge: NoteEdge::Off,
            },
        )
    };
    let samples = render_owner(&mut audio, partition);
    let live_only = render_reference(
        &mut reference,
        &[
            live_on(epoch, 0, held, slot, 48),
            live_on(epoch, STOP.as_u64(), stopped, slot, 49),
            off(400, stopped),
            off(480, held),
        ],
    );
    let held_only = render_reference(
        &mut held_only,
        &[live_on(epoch, 0, held, slot, 48), off(480, held)],
    );
    let report = audio.report();
    assert_eq!(
        report.transport,
        MixedCompiledTransport::Stopped {
            at: STOP,
            released: HeldNoteCount::measured(1),
            position: Some(PlanPosition::new(138)),
        }
    );
    assert!(audio.audio.compiled_minter.live_identities().is_empty());
    let sounding = audio
        .audio
        .renderer
        .snapshot_mixed_sounding()
        .expect("bound registry");
    assert!(sounding.compiled().is_empty());
    assert!(sounding.live().is_empty());
    (samples, live_only, held_only)
}

#[cfg(feature = "simulated-ingress")]
#[test]
fn live_notes_start_and_end_while_compiled_is_stopped() {
    for compiled_first in [true, false] {
        let (reference, live_only, held_only) = ingress_stop_run(compiled_first, PARTITIONS[0]);
        assert_ne!(&reference[128..STOP_OUTPUT], &live_only[128..STOP_OUTPUT]);
        assert_eq!(&reference[STOP_OUTPUT..], &live_only[STOP_OUTPUT..]);
        // Key 49 is heard from the Stop boundary (engine 192..400), and nowhere before.
        assert_ne!(&live_only[STOP_OUTPUT..464], &held_only[STOP_OUTPUT..464]);
        assert_eq!(&live_only[..STOP_OUTPUT], &held_only[..STOP_OUTPUT]);
        for partition in &PARTITIONS[1..] {
            let (samples, ..) = ingress_stop_run(compiled_first, partition);
            assert_eq!(samples, reference, "{partition:?}");
        }
    }
}

#[test]
fn a_stop_due_before_adoption_faults_and_silences_the_callback() {
    let (prepared, candidate) = stop_fixture(true);
    let (_, mut audio) = prepared
        .arm_one_shot(candidate, &mixed_profile())
        .expect("private arm");
    // Scheduling refuses this; force the state a defect would leave.
    audio.stop_pending = Some(SampleTime::ZERO);
    let mut samples = [1.0_f32; 128];
    let error = audio
        .render_private(AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("block"))
        .expect_err("Stop before adoption");
    assert_eq!(error, MixedOneShotRenderError::StopBeforeAdoption);
    assert!(samples.iter().all(|sample| *sample == 0.0));
    assert_eq!(audio.report().fault, Some(error));
    assert!(!audio.report().adopted);
}
