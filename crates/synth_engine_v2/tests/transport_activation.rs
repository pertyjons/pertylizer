//! ADR-0050's activation, driven through actual host calls.
//!
//! `SOUND-INV-018` is what these check. The unit tests beside `transport.rs` cover the value
//! types; this file covers the properties that need a rendering stream — where the boundary
//! falls, what the carry keeps, what a crossing note does, and what an offer refuses.

mod common;

use synth_engine_v2::admit::AdmissionError;
use synth_engine_v2::identity::ProducerId;
use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::publish::{ProducerClass, PublicationArbiter};
use synth_engine_v2::quantities::{Amplitude, ChannelLayout, EventCount, NormalizedLevel, Seconds};
use synth_engine_v2::render::{AudioBlockMut, PreparedRenderer};
use synth_engine_v2::schedule::{
    AdmittedCompiledStream, CompiledEventScheduler, CompiledPayload, PlanEvent,
};
use synth_engine_v2::stream::{ActivationBuildError, ActivationRequest, StreamControl};
use synth_engine_v2::time::{PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor};
use synth_engine_v2::transport::{ActivationRefused, ActivationSequence, LoopInterval};

const SOURCE: NodeId = NodeId::new(1);
const OUTPUT: NodeId = NodeId::new(2);
const ENVELOPE: NodeId = NodeId::new(11);
const AMPLIFIER: NodeId = NodeId::new(12);
const SECOND_ENVELOPE: NodeId = NodeId::new(13);
const SECOND_AMPLIFIER: NodeId = NodeId::new(14);

const Q: u64 = QUANTUM_FRAMES as u64;
const ORIGIN: StreamAnchor = StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO);
const TOTAL: usize = 2_048;

/// A gated constant: one note edge changes the output between silence and full scale, so a
/// sample's value says exactly which side of an edge it is on.
fn gated_constant(simultaneous: u32) -> GraphIr {
    GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).expect("not negative"),
                decay: Seconds::new(0.0).expect("not negative"),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.0).expect("not negative"),
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
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(common::compiled_notes(simultaneous))
        .build()
        .expect("a readable plan")
}

fn plan() -> CompiledPlan {
    common::admit(
        &gated_constant(8),
        common::profile(TOTAL as u64, ChannelLayout::Mono),
    )
}

fn arbiter() -> PublicationArbiter {
    PublicationArbiter::prepare(&common::profile(TOTAL as u64, ChannelLayout::Mono))
        .expect("the publication store is preparable")
}

/// One catch-up row per prepared target, plus the boundary mass release's single operation.
///
/// ADR-0051 clause 1 and clause 4: the batch covers every prepared target and the rule decides a
/// row's value, never whether it exists, so this is what the session share carries at a locate.
fn session_load(plan: &CompiledPlan) -> EventCount {
    EventCount::measured(1 + plan.parameter_targets().len() as u32)
}

fn note(plan: &CompiledPlan, position: u64, on: bool) -> PlanEvent {
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let payload = if on {
        CompiledPayload::NoteOn { slot }
    } else {
        CompiledPayload::NoteOff { slot }
    };
    PlanEvent::new(PlanPosition::new(position), payload)
}

fn admitted(plan: &CompiledPlan, events: &[PlanEvent]) -> AdmittedCompiledStream {
    AdmittedCompiledStream::admit(plan, events).expect("the stream fits its share")
}

/// Render `frames` frames in `block`-sized calls, collecting the output.
fn drive(
    scheduler: &mut CompiledEventScheduler,
    renderer: &mut PreparedRenderer,
    arbiter: &mut PublicationArbiter,
    frames: usize,
    block: usize,
    out: &mut Vec<f32>,
) {
    let mut done = 0;
    while done < frames {
        let this = block.min(frames - done);
        let mut samples = vec![0.0_f32; this];
        let output =
            AudioBlockMut::new(&mut samples, this, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render(renderer, arbiter, output)
            .expect("the stream renders");
        out.extend_from_slice(&samples);
        done += this;
    }
}

fn request(at: u64, position: u64) -> ActivationRequest {
    ActivationRequest {
        at: SampleTime::new(at),
        position: PlanPosition::new(position),
        loop_interval: None,
    }
}

#[test]
fn the_effective_point_is_the_same_under_every_host_partition() {
    // ADR-0050 clause 1's central property: the boundary is a function of the requested time,
    // `Q` and the clock alone, so it does not depend on how the host chops its callbacks.
    // The falsifier is direct — the same activation landing on a different output frame under
    // a different partition — and it is what rules out "the start of the next render call",
    // which would have been the simplest rule and is exactly the one ADR-0001 forbids.
    //
    // The activation seeks to a plan position where a note is already sounding, so the frame
    // the output changes on **is** the effective point, observed rather than reported.
    let partitions: [usize; 4] = [TOTAL, 256, 64, 37];
    let renders: Vec<Vec<f32>> = partitions
        .iter()
        .map(|block| seek_and_render(*block))
        .collect();
    let reference = renders.first().expect("four renders");
    for (index, rendered) in renders.iter().enumerate().skip(1) {
        assert_eq!(
            rendered, reference,
            "host partition {index} moved the activation"
        );
    }

    // And it landed where clause 1 says: the requested time is not on a boundary, so it
    // snapped forward to one, and the output shows it `Q` frames later than that — the
    // stream's declared latency, which clause 2 leaves alone.
    let sounding = reference
        .iter()
        .position(|sample| *sample > 0.5)
        .expect("the seek reaches a sounding note");
    assert_eq!(
        sounding as u64,
        4 * Q + Q,
        "the boundary at 4Q, heard Q frames later"
    );
}

/// Play silence, then seek at a non-boundary time into a position where a note sounds.
fn seek_and_render(block: usize) -> Vec<f32> {
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();

    // The stream in force: nothing at all, so the output is silent until the seek.
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    // The stream after the seek: a note that is already on at the destination.
    let sounding = admitted(&plan, &[note(&plan, 8 * Q, true)]);
    let activation = control
        .plan_activation(&sounding, request(3 * Q + 17, 8 * Q))
        .expect("the candidate builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    let mut out = Vec::with_capacity(TOTAL);
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        TOTAL,
        block,
        &mut out,
    );
    out
}

#[test]
fn an_offer_is_refused_when_it_supersedes_a_state_no_longer_in_force() {
    // ADR-0050 clause 6. Two candidates built against one in-force value are ordered rather
    // than raced: the first adopted moves the value and the second is refused, so a
    // superseded intent can never activate after the intent that replaced it.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    // **Both** built against the one value in force, which is what clause 6 deliberately
    // allows: building competing candidates is fine, and it is acceptance that orders them.
    let first = control
        .plan_activation(&quiet, request(Q, 0))
        .expect("the first candidate builds");
    let second = control
        .plan_activation(&quiet, request(8 * Q, 0))
        .expect("and so does a competitor, against the same in-force value");
    assert_eq!(
        first.supersedes(),
        second.supersedes(),
        "two candidates built before either was accepted name one predecessor"
    );

    let adopted = first.sequence();
    scheduler
        .offer(&mut renderer, first)
        .expect("the first offer is accepted");

    // Adopt it by rendering past its boundary, then collect so the control is free again.
    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        4 * Q as usize,
        64,
        &mut out,
    );
    let retired = scheduler
        .collect()
        .expect("adoption produced a retired value");
    assert_eq!(retired.sequence(), adopted, "the value in force moved");
    control
        .adopted(retired)
        .expect("the control promotes its own retirement");

    // The competitor named the value that was in force before the first activation, and that
    // value has moved. A superseded intent can therefore never activate after the intent that
    // replaced it, however long the caller held on to it.
    let (returned, refusal) = scheduler
        .offer(&mut renderer, second)
        .expect_err("its predecessor is no longer in force");
    assert_eq!(
        refusal,
        ActivationRefused::Superseded {
            supersedes: ActivationSequence::INITIAL,
            in_force: adopted,
        }
    );
    assert_eq!(
        renderer.diagnostics().refused_activations(),
        1,
        "a refusal is counted: a stream that declines every seek must not look like one that \
         adopts them"
    );

    // The candidate comes back so the control can withdraw it, which restores nothing
    // because nothing was taken — the working copy it carried is dropped with it.
    control
        .withdraw(returned)
        .expect("the control withdraws its own candidate");
}

#[test]
fn an_offer_is_refused_while_a_retired_activation_is_uncollected() {
    // `HOST-INV-022`: the exchange is one slot in both directions, and an offer while it is
    // occupied is backpressure rather than a fault. This is the occupancy that means the
    // off-thread half has fallen behind, and it is reported as itself rather than as a
    // pending candidate.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let first = control
        .plan_activation(&quiet, request(Q, 0))
        .expect("the first candidate builds");
    let adopted = first.sequence();
    scheduler
        .offer(&mut renderer, first)
        .expect("the first offer is accepted");
    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        4 * Q as usize,
        64,
        &mut out,
    );

    // Deliberately **not** collected. Building a second candidate is still allowed —
    // ADR-0050 clause 6 says "outstanding" means *accepted at the offer and not yet
    // collected*, and it deliberately permits two candidates to be built against one
    // in-force value. An earlier revision refused the build instead, which removed the very
    // race the sequence exists to resolve; an independent review found it.
    let second = control
        .plan_activation(&quiet, request(8 * Q, 0))
        .expect("a candidate still builds while a retired value is uncollected");

    // It is the **offer** that refuses, and as itself. The candidate is also superseded —
    // adoption moved the value and the control has not collected the proof — but reporting
    // that would name the consequence and hide the cause, which is that this half has not
    // kept up.
    let (returned, refusal) = scheduler
        .offer(&mut renderer, second)
        .expect_err("the slot still holds a retired value");
    assert_eq!(refusal, ActivationRefused::RetiredUncollected);
    control
        .withdraw(returned)
        .expect("the control withdraws its own candidate");

    let retired = scheduler.collect().expect("the retired value is there");
    assert_eq!(retired.sequence(), adopted, "the value in force moved");
    control
        .adopted(retired)
        .expect("the control promotes its own retirement");
    let third = control
        .plan_activation(&quiet, request(8 * Q, 0))
        .expect("and the control builds against the value now in force");
    let fourth = control
        .plan_activation(&quiet, request(16 * Q, 0))
        .expect("as does its competitor");
    scheduler
        .offer(&mut renderer, third)
        .expect("the first of them is accepted");

    // The other occupancy, and the reason the two are separate variants: a pending candidate
    // means the audio thread has not reached the boundary yet, which is a wait rather than a
    // half that has fallen behind.
    let (returned, refusal) = scheduler
        .offer(&mut renderer, fourth)
        .expect_err("a candidate is already waiting to be adopted");
    assert_eq!(refusal, ActivationRefused::AlreadyPending);
    control
        .withdraw(returned)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_note_the_replaced_schedule_started_is_cut_at_the_boundary() {
    // ADR-0050 clause 5. The outgoing schedule plays a note that never ends inside its own
    // list; the activation replaces that schedule, and the note is ended at the boundary
    // rather than left sounding forever.
    //
    // The falsifier is the sample after the boundary: without the mass release the gate
    // stays raised and the output stays at full scale, because nothing else would ever lower
    // it — the new schedule has no release for an occurrence it did not mint.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();

    let held = admitted(&plan, &[note(&plan, 0, true)]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &held).expect("the held note prepares");

    let quiet = admitted(&plan, &[]);
    let activation = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    let mut out = Vec::with_capacity(TOTAL);
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );

    // Sounding from the note's own edge plus the declared carry, silent from the boundary
    // plus the same carry.
    let on = Q as usize;
    let off = (4 * Q + Q) as usize;
    for (frame, sample) in out.iter().copied().enumerate() {
        let expected = if (on..off).contains(&frame) { 1.0 } else { 0.0 };
        assert_eq!(
            sample, expected,
            "frame {frame} is on the wrong side of the boundary release"
        );
    }
}

#[test]
fn the_retired_schedule_comes_back_rather_than_being_dropped() {
    // ADR-0050 clause 3: adoption **exchanges**. Every piece that owns an allocation moves
    // into the slot for the off-thread half to free, so nothing is deallocated in a callback.
    // That the outgoing schedule's events are still there when it is collected is what makes
    // that observable from outside.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();

    let two = admitted(&plan, &[note(&plan, 0, true), note(&plan, 16 * Q, false)]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &two).expect("the outgoing stream prepares");

    let quiet = admitted(&plan, &[]);
    let activation = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );

    let retired = scheduler
        .collect()
        .expect("adoption produced a retired value");
    assert_eq!(
        retired.events(),
        2,
        "the outgoing schedule's storage came back for the control to free"
    );
    assert_eq!(
        retired.producers(),
        1,
        "and so did the producer list its boundary release named"
    );
    assert!(
        retired.release_capacity() >= 8,
        "the release's report storage is sized to the identity partition, which is what makes \
         it all-or-nothing rather than partial"
    );
}

#[test]
fn runtime_loop_playback_fails_closed_until_sample_exact_wraps_exist() {
    // The off-thread checks can admit the interval's density and polyphony, but the runtime
    // has no sample-exact wrap mechanism. The offer must refuse instead of recording a loop
    // and then playing silently past its end.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let interval = LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(4 * Q))
        .expect("a positive interval");
    let scheduler_before = scheduler.in_force();
    let control_before = control.in_force();
    let activation = control
        .plan_activation(
            &quiet,
            ActivationRequest {
                at: SampleTime::new(2 * Q),
                position: PlanPosition::ZERO,
                loop_interval: Some(interval),
            },
        )
        .expect("the candidate passes the off-thread loop bounds");
    let (activation, refusal) = scheduler
        .offer(&mut renderer, activation)
        .expect_err("runtime loop playback is not implemented");
    assert_eq!(
        refusal,
        ActivationRefused::LoopPlaybackUnsupported {
            start: interval.start(),
            end: interval.end(),
        }
    );
    assert_eq!(
        renderer.diagnostics().refused_activations(),
        1,
        "the unsupported offer is visible"
    );
    assert_eq!(
        scheduler.in_force(),
        scheduler_before,
        "refusal leaves the scheduler's active sequence unchanged"
    );
    assert_eq!(
        control.in_force(),
        control_before,
        "refusal leaves the control's active sequence unchanged"
    );
    assert!(
        scheduler.collect().is_none(),
        "refusal leaves neither a pending candidate nor a retired value"
    );
    control
        .withdraw(activation)
        .expect("a refused candidate still costs the control nothing");
}

#[test]
fn a_late_activation_takes_effect_at_the_clock_and_is_counted() {
    // ADR-0050 clause 1's preserving clamp for an activation. Building is off-thread work of
    // unbounded duration, so a candidate can be finished after its requested time has passed;
    // it then activates at the clock, and the counter is what makes that attributable rather
    // than leaving a seek that felt sluggish unexplained.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    // Render first, so the clock is already past the time the candidate will name.
    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );
    let clock = renderer.clock();
    assert!(clock.as_u64() > 2 * Q, "the clock is past the request");

    let activation = control
        .plan_activation(&quiet, request(2 * Q, 0))
        .expect("the candidate builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        Q as usize,
        64,
        &mut out,
    );

    let retired = scheduler.collect().expect("it was adopted");
    assert_eq!(
        retired.effective(),
        Some(clock),
        "a late activation takes effect at the clock, not at the time it named"
    );
    assert_eq!(
        renderer.diagnostics().late_activations(),
        1,
        "and the displacement is counted rather than silent"
    );
}

/// Which prepared node the envelope's gate lives on.

#[test]
fn a_candidate_from_another_stream_is_refused_as_stale() {
    // ADR-0032 clause 20's rule, applied to an activation: an epoch that is not this
    // stream's names a stream that has ended, and adopting one would move this stream's
    // anchor to a position another stream chose.
    let plan = plan();
    let (mut mine, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let (mut theirs, _their_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("a second stream opens");

    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut mine, &quiet).expect("an empty stream prepares");
    let their_quiet = admitted(&plan, &[]);
    let _their_scheduler = CompiledEventScheduler::prepare(&mut theirs, &their_quiet)
        .expect("their stream prepares too");

    let foreign = theirs
        .plan_activation(&their_quiet, request(4 * Q, 0))
        .expect("their candidate builds");
    let (returned, refusal) = scheduler
        .offer(&mut renderer, foreign)
        .expect_err("a candidate from another stream is stale");
    assert!(matches!(refusal, ActivationRefused::StaleEpoch { .. }));
    theirs
        .withdraw(returned)
        .expect("the control withdraws its own candidate");
}

#[test]
fn the_boundary_release_reaches_only_the_producers_the_activation_names() {
    // ADR-0046 clause 6's scope, which ADR-0050 clause 5 inherits: a seek moves plan time, it
    // does not lift a performer's finger. The plan here declares one producer, so the scope
    // is that one; the property under test is that the scope is *carried* rather than
    // assumed, which is what a later live producer will depend on.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let activation = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    assert_eq!(
        activation.producers(),
        plan.note_producer_ranges().len(),
        "the scope is the plan's declared note-on producers"
    );
    let _ = ProducerId::new(0);
    control
        .withdraw(activation)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_seek_between_a_notes_edges_builds_and_counts_the_release_it_omits() {
    // ADR-0050 clause 5. The commonest seek there is: land between a compiled note's on and
    // off edges. The suffix then begins with a release whose note-on the new stream never
    // plays, and stamping refuses a list like that — so without the omission this seek could
    // not produce a schedule at all. An independent review found the builder refusing it.
    //
    // The falsifier is the build itself, and it is sharp: an earlier revision returned
    // `BeforeAnchor` for the note-on and `UnmatchedRelease` for the release, so *neither*
    // way of handing the list over worked. The count is what keeps the omission a named
    // transformation rather than a silent drop, which ADR-0001 clause 16 forbids.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    // One note, open across the destination.
    let crossing = admitted(&plan, &[note(&plan, 0, true), note(&plan, 16 * Q, false)]);
    let activation = control
        .plan_activation(&crossing, request(4 * Q, 8 * Q))
        .expect("a seek through a held note builds");
    assert_eq!(
        activation.omitted_releases(),
        1,
        "the release whose note-on precedes the anchor is omitted and counted"
    );
    assert_eq!(
        activation.events(),
        1,
        "ADR-0051 clause 5: the omission drops the note contract, not the gate write the plan \
         authored — a bare gate-down takes the release's place, so automation raising that gate \
         after the destination still has something able to lower it"
    );

    // A release the suffix can pair for itself is **not** omitted: the rule is about which
    // side of the anchor a release's own note-on falls on, not about releases in general.
    let inside = admitted(
        &plan,
        &[
            note(&plan, 0, true),
            note(&plan, 16 * Q, false),
            note(&plan, 20 * Q, true),
            note(&plan, 24 * Q, false),
        ],
    );
    let activation = control
        .plan_activation(&inside, request(4 * Q, 8 * Q))
        .expect("the seek builds");
    assert_eq!(activation.omitted_releases(), 1, "only the crossing one");
    assert_eq!(
        activation.events(),
        3,
        "the pair that lies wholly after the destination is placed untouched, and the crossing \
         release contributes its gate-down beside them rather than vanishing"
    );
    control
        .withdraw(activation)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_refused_call_adopts_nothing_even_when_the_boundary_is_the_clock() {
    // ADR-0050 clause 3: a refusal leaves the stream exactly as it was. The boundary-at-the-
    // clock branch used to adopt **before** the call was validated, so a caller that crossed
    // two streams' halves — or retried after a fault — got the error it deserved *and* a
    // stream that had already seeked. An independent review found it, and this is the shape
    // it found: the epoch check now runs first.
    //
    // The falsifier is the exchange. Before the repair the candidate had been swapped out and
    // `collect` would answer with a retired value; now the slot still holds it, so the same
    // call retried against the right renderer adopts normally.
    let plan = plan();
    let (mut mine, mut my_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let (_theirs, mut their_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("a second stream opens");
    let mut arbiter = arbiter();

    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut mine, &quiet).expect("an empty stream prepares");

    // Requested at the clock itself, which is a boundary: `owned` is zero, so this is the
    // branch that adopts before rendering anything.
    let activation = mine
        .plan_activation(&quiet, request(0, 0))
        .expect("the candidate builds");
    let sequence = activation.sequence();
    scheduler
        .offer(&mut my_renderer, activation)
        .expect("the offer is accepted");

    let mut samples = vec![0.0_f32; 64];
    let output = AudioBlockMut::new(&mut samples, 64, ChannelLayout::Mono).expect("a shaped block");
    let refused = scheduler
        .render(&mut their_renderer, &mut arbiter, output)
        .expect_err("a renderer from another stream is refused");
    assert!(matches!(
        refused,
        synth_engine_v2::schedule::ScheduledRenderError::EpochMismatch { .. }
    ));
    assert_eq!(
        scheduler.in_force(),
        ActivationSequence::INITIAL,
        "the refusal moved nothing"
    );
    assert!(
        scheduler.collect().is_none(),
        "and the candidate is still waiting rather than already adopted"
    );

    // Retried against its own renderer, the same call adopts. Two quanta, because a call
    // served entirely from the carry renders none and adoption waits for one — the rule that
    // keeps an adoption's release charge payable in the call that incurs it.
    let mut samples = vec![0.0_f32; 128];
    let output =
        AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("a shaped block");
    scheduler
        .render(&mut my_renderer, &mut arbiter, output)
        .expect("the stream renders");
    assert_eq!(scheduler.in_force(), sequence, "and only then");
}

/// Admit a stream against a plan the fixture above built.

#[test]
fn the_boundary_mass_release_is_charged_to_the_session_share() {
    // ADR-0046 clause 6, which ADR-0050 clause 5 inherits: the release is **one** operation
    // charged to the session share, never one event per voice. It ran uncharged, which an
    // independent review found — and it matters beyond tidiness, because clause 7's catch-up
    // will share this row when it lands, so an uncharged operation would be exactly the
    // overrun admission was supposed to have refused.
    //
    // The falsifier is the ledger: the session mark must be exactly one, and the amount
    // admission reserves must be that same number.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let activation = control
        .plan_activation(&quiet, request(4 * Q, 8 * Q))
        .expect("the candidate builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );

    assert_eq!(
        arbiter.high_water(ProducerClass::Session),
        session_load(&plan),
        "the boundary quantum carries exactly one release **operation**, never one per voice, \
         plus ADR-0051 clause 1's catch-up row per prepared target"
    );
}

#[test]
fn a_fault_in_either_half_of_a_split_silences_the_whole_callback() {
    // The host-profile specification's terminal contract is silence over the **complete**
    // current callback. ADR-0050 clause 4 renders a crossing block as two calls, and a
    // sub-call can only silence its own span — so a fault in the tail used to leave the
    // already-rendered head audible in the same callback. An independent review found it.
    //
    // The fault is induced with a publication store prepared for a smaller callback than the
    // one the tail asks for. The falsifier is the head: a note is sounding across it, so
    // without the repair those frames come back at full scale.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");

    // Prepared for a quarter of the block this test renders, which is what the tail overruns.
    let mut arbiter = PublicationArbiter::prepare(&common::profile(256, ChannelLayout::Mono))
        .expect("the smaller store is preparable");

    let held = admitted(&plan, &[note(&plan, 0, true)]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &held).expect("the held note prepares");

    let quiet = admitted(&plan, &[]);
    let activation = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    // One callback that crosses the boundary: the head renders four quanta the store can
    // serve, the tail asks for more than it was prepared for.
    let frames = 1_024;
    let mut samples = vec![0.0_f32; frames];
    let output =
        AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("a shaped block");
    let fault = scheduler
        .render(&mut renderer, &mut arbiter, output)
        .expect_err("the tail cannot be published");
    assert!(
        matches!(
            fault,
            synth_engine_v2::schedule::ScheduledRenderError::Publication(_)
        ),
        "the tail failed to publish, which is terminal: {fault}"
    );
    assert!(
        renderer.diagnostics().needs_reprepare(),
        "a publication fault ends the epoch"
    );

    assert!(
        samples.iter().all(|sample| *sample == 0.0),
        "the head was left audible after the tail faulted: {} of {frames} frames are not \
         silent",
        samples.iter().filter(|sample| **sample != 0.0).count()
    );
}

#[test]
fn a_withdrawn_candidate_leaves_the_partition_exactly_as_it_found_it() {
    // ADR-0050 clause 3's withdrawal, and the property that makes "a failed or abandoned
    // build costs nothing" checkable rather than asserted. The candidate is stamped against a
    // **working copy** of the minter; withdrawing drops the copy, and the authoritative table
    // never saw the mints.
    //
    // The falsifier is over-emission, which needs a partition small enough to exhaust: with
    // one simultaneous note declared, a leaked reservation makes the *next* build refuse.
    // Two earlier designs of this failed here — releasing the outstanding set back is not the
    // inverse of stamping, because a note paired inside the list has already spent its
    // generation.
    let plan = single_note_plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let one = admitted(&plan, &[note(&plan, 0, true)]);
    for attempt in 0..4 {
        let candidate = control
            .plan_activation(&one, request(4 * Q, 0))
            .unwrap_or_else(|error| panic!("attempt {attempt} could not build: {error}"));
        assert_eq!(candidate.outstanding(), 1, "the note holds one index");
        control
            .withdraw(candidate)
            .expect("the control withdraws its own candidate");
    }
}

#[test]
fn the_schedule_in_force_releases_its_index_to_the_one_replacing_it() {
    // ADR-0050 clause 3 again, from the other side: the control releases the outgoing
    // schedule's outstanding occurrences into the working copy **before** stamping the
    // candidate, because clause 5's boundary release is what ends those notes. Without it a
    // producer whose declared polyphony the outgoing schedule already uses could not build
    // any replacement at all.
    //
    // The falsifier is the same over-emission, with the index held by the schedule that is
    // running rather than by an abandoned candidate.
    let plan = single_note_plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");

    // The stream in force holds the partition's only index: a note-on its own list never
    // pairs.
    let held = admitted(&plan, &[note(&plan, 0, true)]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &held).expect("the held note prepares");

    let replacement = admitted(&plan, &[note(&plan, 8 * Q, true)]);
    let candidate = control
        .plan_activation(&replacement, request(4 * Q, 8 * Q))
        .expect("the replacement can mint into the range the outgoing schedule was using");
    assert_eq!(candidate.outstanding(), 1);
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

/// The same gated constant, with a partition of exactly one index.
///
/// Small enough that a leaked reservation is the difference between a build and a refusal.
fn single_note_plan() -> CompiledPlan {
    common::admit(
        &gated_constant(1),
        common::profile(TOTAL as u64, ChannelLayout::Mono),
    )
}

#[test]
fn a_retirement_from_another_stream_is_refused_rather_than_promoted() {
    // Promotion installs an anchor, a **minter** and an outstanding set together, so a value
    // collected from another stream's scheduler would make this control mint from a table its
    // own renderer treats as foreign — every note it then placed would be filtered as someone
    // else's and the stream would go silent with nothing to point at.
    //
    // The falsifier is the identity: after the refusal this control must still mint from its
    // own table, which is what its renderer answers to. An independent review found the first
    // version checking only whether the value had been adopted at all.
    let plan = plan();
    let (mut mine, mut my_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let (mut theirs, mut their_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("a second stream opens");
    let mut their_arbiter = arbiter();

    let quiet = admitted(&plan, &[]);
    let mut my_scheduler =
        CompiledEventScheduler::prepare(&mut mine, &quiet).expect("an empty stream prepares");
    let their_quiet = admitted(&plan, &[]);
    let mut their_scheduler = CompiledEventScheduler::prepare(&mut theirs, &their_quiet)
        .expect("their stream prepares too");

    // Their stream adopts an activation and retires it.
    let theirs_candidate = theirs
        .plan_activation(&their_quiet, request(Q, 0))
        .expect("their candidate builds");
    their_scheduler
        .offer(&mut their_renderer, theirs_candidate)
        .expect("their offer is accepted");
    let mut out = Vec::new();
    drive(
        &mut their_scheduler,
        &mut their_renderer,
        &mut their_arbiter,
        4 * Q as usize,
        64,
        &mut out,
    );
    let foreign = their_scheduler
        .collect()
        .expect("their adoption produced a retired value");

    let (returned, refusal) = mine
        .adopted(foreign)
        .expect_err("a retirement from another stream is not this control's to promote");
    assert!(matches!(
        refusal,
        synth_engine_v2::stream::ActivationCollectError::ForeignStream { .. }
    ));

    // **And it comes back.** A retirement is the only proof its own control has that
    // adoption happened; consuming one handed to the wrong control would leave the right one
    // stuck on its previous sequence with every later candidate refused as superseded.
    theirs
        .adopted(returned)
        .expect("the control it belongs to can still promote it");

    // Still minting from its own table: a candidate this control builds is still one its own
    // renderer accepts, which is the property the refusal protects.
    let sounding = admitted(&plan, &[note(&plan, 0, true)]);
    let candidate = mine
        .plan_activation(&sounding, request(0, 0))
        .expect("the control still builds against its own partition");
    my_scheduler
        .offer(&mut my_renderer, candidate)
        .expect("and its own scheduler accepts it");
}

#[test]
fn an_offer_paired_with_another_streams_renderer_counts_nothing() {
    // The refusal counters belong to the stream that was offered to. Attributing this
    // stream's refusal to a renderer that is not its half would put a diagnostic on a stream
    // that was never asked anything, and `render` would later refuse the same pairing as an
    // epoch mismatch — so the offer refuses it one call earlier and touches no counter.
    let plan = plan();
    let (mut mine, my_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let (mut theirs, mut their_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("a second stream opens");

    let quiet = admitted(&plan, &[]);
    let mut my_scheduler =
        CompiledEventScheduler::prepare(&mut mine, &quiet).expect("an empty stream prepares");
    let their_quiet = admitted(&plan, &[]);
    let _their_scheduler = CompiledEventScheduler::prepare(&mut theirs, &their_quiet)
        .expect("their stream prepares too");

    let candidate = mine
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    let (returned, refusal) = my_scheduler
        .offer(&mut their_renderer, candidate)
        .expect_err("the renderer is not this schedule's half");
    assert!(matches!(refusal, ActivationRefused::ForeignRenderer { .. }));
    assert_eq!(
        their_renderer.diagnostics().refused_activations(),
        0,
        "a stream that was never offered anything must not carry the refusal"
    );
    assert_eq!(my_renderer.diagnostics().refused_activations(), 0);
    mine.withdraw(returned)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_request_with_no_quantum_boundary_after_it_is_refused_at_the_build() {
    // The effective point is the first boundary at or after the requested time, so a request
    // in the final non-boundary frames of engine time has none. Accepting one would put a
    // candidate in the exchange that no boundary can ever adopt — and a permanently occupied
    // slot is indistinguishable from a stream that has simply not reached its boundary yet,
    // so every later offer is refused with nothing to explain it.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let refusal = control
        .plan_activation(
            &quiet,
            ActivationRequest {
                at: SampleTime::new(u64::MAX),
                position: PlanPosition::ZERO,
                loop_interval: None,
            },
        )
        .expect_err("no boundary exists at or after the last frame of engine time");
    assert!(matches!(
        refusal,
        synth_engine_v2::stream::ActivationBuildError::BoundaryUnrepresentable { .. }
    ));

    // The frame one quantum earlier is the last one that does have a boundary, so the refusal
    // is about representability rather than about being near the end.
    let last = u64::MAX - (u64::MAX % u64::from(QUANTUM_FRAMES));
    let candidate = control
        .plan_activation(
            &quiet,
            ActivationRequest {
                at: SampleTime::new(last),
                position: PlanPosition::ZERO,
                loop_interval: None,
            },
        )
        .expect("the last representable boundary is still a legal request");
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_release_with_no_note_on_at_all_is_refused_rather_than_absorbed() {
    // ADR-0050 clause 5 omits a release whose note-on **precedes the anchor**. A release with
    // no note-on on either side is a malformed timeline, and `AdmittedCompiledStream` does not
    // check pairing — so the history pass has to refuse it. An independent review found a
    // saturating subtraction absorbing it instead, which turned a malformed list into a
    // successful activation the stamper never got to see.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let malformed = admitted(&plan, &[note(&plan, 0, false)]);
    let refusal = control
        .plan_activation(&malformed, request(4 * Q, 8 * Q))
        .expect_err("a release with nothing to pair it with is not a seek");
    assert!(matches!(
        refusal,
        synth_engine_v2::stream::ActivationBuildError::Stamp(
            synth_engine_v2::schedule::SchedulePrepareError::UnmatchedRelease { .. }
        )
    ));
}

#[test]
fn a_fault_in_the_head_of_a_split_silences_the_tail_it_never_rendered() {
    // The other direction of the terminal contract, and the one the first version of this
    // coverage claim did not actually test. When the head faults, the tail is never rendered
    // at all — so it still holds whatever the caller's buffer held, which is as much a
    // violation of "silence over the complete callback" as leaving rendered audio behind.
    //
    // The falsifier is the buffer's prior contents: it is filled with full scale before the
    // call, so anything left unwritten comes back audible.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");

    // Too small even for the head, which asks for the four quanta the old mapping owns.
    let mut arbiter = PublicationArbiter::prepare(&common::profile(64, ChannelLayout::Mono))
        .expect("the smaller store is preparable");

    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");
    let activation = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    let frames = 1_024;
    let mut samples = vec![1.0_f32; frames];
    let output =
        AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("a shaped block");
    let fault = scheduler
        .render(&mut renderer, &mut arbiter, output)
        .expect_err("the head cannot be published");
    assert!(
        matches!(
            fault,
            synth_engine_v2::schedule::ScheduledRenderError::Publication(_)
        ),
        "the head failed to publish, which is terminal: {fault}"
    );
    assert!(
        samples.iter().all(|sample| *sample == 0.0),
        "the tail was never rendered and never silenced: {} of {frames} frames are not silent",
        samples.iter().filter(|sample| **sample != 0.0).count()
    );
}

#[test]
fn a_block_the_renderer_cannot_serve_adopts_nothing() {
    // The block's shape is decided inside `PreparedRenderer::render`, which used to run
    // **after** the boundary-at-the-clock branch had already adopted. A layout mismatch is
    // retryable, so a caller that corrected it would find a stream that had silently seeked
    // and charged its release on a call that rendered nothing. An independent review found
    // it in exactly this branch.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    // Requested at the clock, which is a boundary: the branch that adopts before rendering.
    let activation = control
        .plan_activation(&quiet, request(0, 0))
        .expect("the candidate builds");
    let sequence = activation.sequence();
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    // Stereo against a mono plan: refused, and retryable.
    let mut samples = vec![0.0_f32; 128];
    let output =
        AudioBlockMut::new(&mut samples, 64, ChannelLayout::Stereo).expect("a shaped block");
    let refused = scheduler
        .render(&mut renderer, &mut arbiter, output)
        .expect_err("a layout the plan does not render is refused");
    assert!(
        matches!(
            refused,
            synth_engine_v2::schedule::ScheduledRenderError::Render(
                synth_engine_v2::diagnostics::RenderError::OutputBufferShape { .. }
            )
        ),
        "{refused}"
    );
    assert_eq!(
        scheduler.in_force(),
        ActivationSequence::INITIAL,
        "the refusal moved nothing"
    );
    assert!(
        scheduler.collect().is_none(),
        "and the candidate is still waiting rather than already adopted"
    );

    // Corrected, the same call adopts — and it is the call that charges the release, which
    // a call that had adopted and then failed would have lost. Two quanta, so the call
    // renders one rather than being served entirely from the carry.
    let mut samples = vec![0.0_f32; 128];
    let output =
        AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("a shaped block");
    scheduler
        .render(&mut renderer, &mut arbiter, output)
        .expect("the corrected block renders");
    assert_eq!(scheduler.in_force(), sequence);
    assert_eq!(
        arbiter.high_water(ProducerClass::Session),
        session_load(&plan),
        "the retry charged the release and its catch-up, which the refused call must not have charged"
    );
}

#[test]
fn snapping_to_a_boundary_is_not_lateness() {
    // ADR-0050 clause 1 defines a late activation as one whose requested time the clock has
    // already passed — the candidate was finished too slowly, which is the only thing the
    // counter is for. **Activating later than the requested time is not the same thing**:
    // every request that does not fall on a quantum boundary does that by construction.
    //
    // The falsifier is the counter on a perfectly prompt seek. An earlier revision tested
    // `effective > requested` and would have reported this one as delayed off-thread
    // preparation, which an independent review found.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    // Ahead of the clock, and deliberately off the grid: it snaps forward to 4Q.
    let activation = control
        .plan_activation(&quiet, request(3 * Q + 17, 0))
        .expect("the candidate builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );
    let retired = scheduler.collect().expect("it was adopted");
    assert_eq!(
        retired.effective(),
        Some(SampleTime::new(4 * Q)),
        "it did activate later than it asked, which is the snap"
    );
    assert_eq!(
        renderer.diagnostics().late_activations(),
        0,
        "but it was not late: the clock had not passed the time it named"
    );
}

#[test]
fn an_adoption_never_happens_in_a_call_that_renders_no_quantum() {
    // An adoption owes its boundary quantum one release charge, published by that call's own
    // publication pass. A call served entirely from the carry opens a window of no quanta,
    // which has no row for it — so adopting there leaves the debt owing, and a second
    // activation adopted at the same clock coalesces two release operations into one charge.
    // An independent review found it, and clause 7's catch-up will owe its quantum the same
    // way.
    //
    // **The rule is that adoption waits for a quantum, not that it waits for the debt.** A
    // first repair skipped the whole activation while any debt was outstanding, which delayed
    // an already-accepted candidate by a whole callback and so made its effective point a
    // function of the host's block size — the one property this slice exists to guarantee. A
    // second review found that. Waiting for a quantum costs nothing observable instead: a call
    // that renders none writes no audio and does not move the clock.
    //
    // The falsifier is the pairing: the call that moves the sequence must be the same call
    // that publishes the batch.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let activation = control
        .plan_activation(&quiet, request(0, 0))
        .expect("the candidate builds");
    let sequence = activation.sequence();
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    // One frame at a time. The stream opens with a whole quantum of carry, so the first
    // sixty-four calls render nothing at all.
    let owed = session_load(&plan);
    let mut adopted_at = None;
    for call in 0..(2 * QUANTUM_FRAMES) {
        let mut samples = [0.0_f32; 1];
        let output =
            AudioBlockMut::new(&mut samples, 1, ChannelLayout::Mono).expect("a shaped block");
        scheduler
            .render(&mut renderer, &mut arbiter, output)
            .expect("the stream renders");

        if scheduler.in_force() == sequence {
            adopted_at = Some(call);
            break;
        }
        assert_eq!(
            arbiter.high_water(ProducerClass::Session),
            EventCount::NONE,
            "nothing may be published before the activation that owes it is adopted"
        );
    }

    let adopted_at = adopted_at.expect("the activation is adopted once a quantum renders");
    assert!(
        adopted_at >= QUANTUM_FRAMES,
        "adoption waited for the carry to drain rather than happening on an empty call"
    );
    assert_eq!(
        arbiter.high_water(ProducerClass::Session),
        owed,
        "and the call that adopted is the call that charged its release"
    );
}

#[test]
fn a_retirement_reports_the_cursor_and_anchor_it_replaced() {
    // ADR-0050 clause 3 exchanges an atomic set. The allocations come back by being swapped;
    // the anchor and cursor are values with nowhere to go, and an earlier revision simply
    // overwrote them — so a collected retirement could not describe the state it claimed to
    // return. An independent review found the hole. ADR-0055 prevents a loop from entering
    // active state until runtime wrapping exists, so no active loop belongs in this fixture.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();

    // Advance an outgoing schedule with two edges so its retirement has a cursor to report.
    let two = admitted(&plan, &[note(&plan, 0, true), note(&plan, Q, false)]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &two).expect("the outgoing stream prepares");
    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        2 * Q as usize,
        64,
        &mut out,
    );

    // Replace it, and read back what the replacement retired.
    let quiet = admitted(&plan, &[]);
    let replacement = control
        .plan_activation(&quiet, request(8 * Q, 0))
        .expect("the replacement builds");
    scheduler
        .offer(&mut renderer, replacement)
        .expect("the offer is accepted");
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );
    let retired = scheduler.collect().expect("the replacement was adopted");
    let state = retired
        .retired()
        .expect("an adopted activation reports what it replaced");
    assert!(
        state.cursor > 0,
        "and so does how far the replaced schedule had been released"
    );
    assert_eq!(
        state.anchor.time(),
        SampleTime::ZERO,
        "and the mapping it was running under"
    );
}

#[test]
fn an_activation_offered_after_its_time_has_passed_is_late() {
    // `SOUND-INV-018`'s rule verbatim: the clock had already passed `T` when the candidate
    // arrived. This is the case a snapping-based test cannot see — the request at 65 and the
    // clock at 128 snap to the same boundary, so the activation is displaced by nothing and
    // is still late, because building it took longer than the time it asked for. An
    // independent review found the implementation reporting it as prompt.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    // Move the clock, then ask for a time inside the quantum immediately before it — so the
    // request's own snap **is** the clock and the activation is displaced by nothing.
    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        4 * Q as usize,
        64,
        &mut out,
    );
    let clock = renderer.clock();
    assert!(clock.as_u64() >= Q, "the clock has left the origin");
    assert_eq!(
        clock.as_u64() % Q,
        0,
        "the clock always stands on a quantum boundary"
    );
    let asked = clock.as_u64() - Q + 1;

    let activation = control
        .plan_activation(&quiet, request(asked, 0))
        .expect("the candidate builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        2 * Q as usize,
        64,
        &mut out,
    );

    let retired = scheduler.collect().expect("it was adopted");
    assert_eq!(
        retired.effective(),
        Some(clock),
        "it took effect at the clock, which is where its own snap would have put it anyway"
    );
    assert!(
        retired.was_late(),
        "so the displacement is zero — and it is still late, which is the distinction"
    );
    assert_eq!(renderer.diagnostics().late_activations(), 1);
}

#[test]
fn a_stamping_refusal_names_the_event_in_the_stream_the_caller_admitted() {
    // The suffix is derived here — history dropped, crossing-note releases omitted — so an
    // index into it names a position in a list the caller has never seen. An independent
    // review found a refusal reporting the fourth event of a stream as its first, which sends
    // whoever has to fix the timeline to the wrong place.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    // Three well-formed events before the destination, then a release with nothing to pair.
    let malformed = admitted(
        &plan,
        &[
            note(&plan, 0, true),
            note(&plan, Q, false),
            note(&plan, 2 * Q, true),
            note(&plan, 3 * Q, false),
            note(&plan, 12 * Q, false),
        ],
    );
    let refusal = control
        .plan_activation(&malformed, request(4 * Q, 8 * Q))
        .expect_err("the trailing release has no note-on on either side");
    match refusal {
        synth_engine_v2::stream::ActivationBuildError::Stamp(
            synth_engine_v2::schedule::SchedulePrepareError::UnmatchedRelease { event_index },
        ) => assert_eq!(
            event_index, 4,
            "the index names the caller's own list, not the suffix this builder derived"
        ),
        other => panic!("unexpected refusal: {other}"),
    }
}

#[test]
fn every_committed_stamping_keeps_its_reservations_reclaimable() {
    // Stamping commits into the one authoritative table and releases nothing, so a second
    // committed list's live set is *additional* rather than a replacement. Overwriting lost
    // the first list's reservations while the minter still held them — and an activation
    // could then not reclaim an index that no schedule admitted to holding, so a valid
    // replacement was refused as over-emission. An independent review found it.
    //
    // The falsifier needs a partition small enough to exhaust: one simultaneous note.
    let plan = single_note_plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");

    // The first committed list leaves the partition's only index live.
    let held = admitted(&plan, &[note(&plan, 0, true)]);
    let _first =
        CompiledEventScheduler::prepare(&mut control, &held).expect("the held note prepares");

    // A second committed stamping against the same control, holding nothing itself. Through
    // `stamp_compiled` rather than a second scheduler, because a stream has one schedule —
    // this is about what stamping does to the table, not about how many schedules exist.
    let _second =
        synth_engine_v2::schedule::stamp_compiled(&mut control, &[]).expect("an empty list stamps");

    // The replacement still reclaims the index the first list holds, because the control
    // remembers that it holds it.
    let replacement = admitted(&plan, &[note(&plan, 8 * Q, true)]);
    let candidate = control
        .plan_activation(&replacement, request(4 * Q, 8 * Q))
        .expect("the earlier stamping's reservation is still reclaimable");
    assert_eq!(candidate.outstanding(), 1);
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

#[test]
fn the_minter_stands_still_while_a_candidate_holds_a_snapshot_of_it() {
    // ADR-0050 clause 3 stamps a candidate against a **copy** of the authoritative table and
    // promotes that copy at collection. A stamping committed in between advances generations
    // the copy has never seen, and the promotion would rewind them — after which a later note
    // could be handed an identity that is already live, which is exactly what
    // `SOUND-INV-017`'s never-reused generation forbids. An independent review found the
    // rewind; the price of the copy is that the table stands still while one is outstanding.
    //
    // The falsifier is the refusal, and its converse: stamping is possible again the moment
    // the candidate is accounted for, whether it was withdrawn or collected.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let candidate = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    assert_eq!(control.outstanding_candidates(), 1);

    let refused = synth_engine_v2::schedule::stamp_compiled(&mut control, &[])
        .expect_err("the minter may not move while a snapshot of it is adoptable");
    assert!(matches!(
        refused,
        synth_engine_v2::schedule::SchedulePrepareError::CandidateOutstanding { outstanding: 1 }
    ));

    // Withdrawing accounts for it, and the table is free again.
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
    assert_eq!(control.outstanding_candidates(), 0);
    let _again = synth_engine_v2::schedule::stamp_compiled(&mut control, &[])
        .expect("a withdrawn candidate strands nothing");

    // So does collecting one, which is the other way a candidate is accounted for.
    let adopted = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("a second candidate builds");
    scheduler
        .offer(&mut renderer, adopted)
        .expect("the offer is accepted");
    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );
    let retired = scheduler.collect().expect("it was adopted");
    control
        .adopted(retired)
        .expect("the control promotes its own retirement");
    assert_eq!(control.outstanding_candidates(), 0);
    let _after = synth_engine_v2::schedule::stamp_compiled(&mut control, &[])
        .expect("and a collected one strands nothing either");
}

#[test]
fn a_plan_of_many_empty_note_producers_still_activates() {
    // Admission charges the identity **span**, so a plan may declare thousands of note
    // producers that admit no indices between them. The boundary release skips those, which
    // is what bounds its work by the span rather than by the declaration count — an
    // independent review found the loop scaling with the count.
    //
    // **This test does not check that bound**, and saying so is the point: it was written as
    // one and the mutation proved it vacuous, because four thousand clears of an empty scope
    // complete as fast as none. The crate has no operation counter on this path, and a timing
    // assertion would be a flake rather than a falsifier. What is checked here is the
    // correctness half — such a plan is admitted and adopts — and the cost bound is carried
    // by the code's own structure with no executable check behind it.
    let plan = many_empty_producers();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter =
        PublicationArbiter::prepare(&common::profile(TOTAL as u64, ChannelLayout::Mono))
            .expect("the publication store is preparable");
    let quiet = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream is admissible");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    assert!(
        plan.note_producer_ranges().len() >= 4_096,
        "the fixture must declare enough producers for the distinction to matter"
    );
    assert_eq!(
        plan.note_producer_ranges()
            .iter()
            .map(|range| range.get())
            .sum::<u32>(),
        0,
        "and none of them may admit an index, which is what makes the span the real bound"
    );

    let activation = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");
    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );
    assert!(
        scheduler.in_force() != ActivationSequence::INITIAL,
        "the activation was adopted"
    );
}

/// A plan declaring many note producers that admit no indices between them.
fn many_empty_producers() -> CompiledPlan {
    // Exactly one compiled producer, which is all a plan may declare; the rest are the
    // non-compiled kind, which a plan may declare freely and which still take an identity
    // range each.
    let producers = (0..4_096)
        .map(|index| synth_engine_v2::ir::NoteProducerDeclaration {
            compiled: index == 0,
            simultaneous_notes: synth_engine_v2::quantities::HeldNoteCount::NONE,
            simultaneous_holds: EventCount::NONE,
        })
        .collect();
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(synth_engine_v2::ir::PlanDeclarations {
            note_producers: producers,
            ..synth_engine_v2::ir::PlanDeclarations::default()
        })
        .build()
        .expect("a readable plan");
    common::admit(&ir, common::profile(TOTAL as u64, ChannelLayout::Mono))
}

#[test]
fn a_loop_whose_extension_does_not_fit_is_refused_at_the_build() {
    // ADR-0046 clause 4 admits a loop by checking the periodic extension of `[start, end)`
    // against a sliding `Q`-frame window, and ADR-0050 clause 3 wants the interval **already
    // admitted** when it joins the atomic set. `LoopInterval::new` proves only that the
    // interval is positive — so without this the first wrap faults at publication, which ends
    // the stream instead of refusing the state change. An independent review found the gap,
    // and this is `admit_loop`'s first caller.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");

    // A stream that fits its share linearly: four writes inside two frames is nothing
    // against a share of ninety-six.
    let gate = plan
        .resolve_parameter(ENVELOPE, synth_engine_v2::ir::parameters::ENVELOPE_GATE)
        .expect("the envelope declares a gate parameter");
    let write = |position: u64| {
        PlanEvent::new(
            PlanPosition::new(position),
            CompiledPayload::SetParameter {
                slot: gate,
                value: synth_engine_v2::quantities::ParameterValue::ZERO,
            },
        )
    };
    let stream = admitted(&plan, &[write(0), write(0), write(1), write(1)]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");

    // Wrapped every two frames, those four events appear thirty-four times in one window —
    // `ceil(Q / 2) + 2` copies — which is well past the share the linear list fitted inside.
    let dense = LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(2)).expect("positive");
    let refusal = control
        .plan_activation(
            &stream,
            ActivationRequest {
                at: SampleTime::new(4 * Q),
                position: PlanPosition::ZERO,
                loop_interval: Some(dense),
            },
        )
        .expect_err("the periodic extension does not fit the compiled share");
    assert!(
        matches!(
            refusal,
            synth_engine_v2::stream::ActivationBuildError::Loop { .. }
        ),
        "{refusal}"
    );

    // The same stream with a loop long enough to hold it is admitted.
    let roomy = LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(16 * Q)).expect("positive");
    let candidate = control
        .plan_activation(
            &stream,
            ActivationRequest {
                at: SampleTime::new(4 * Q),
                position: PlanPosition::ZERO,
                loop_interval: Some(roomy),
            },
        )
        .expect("a loop whose extension fits is admitted");
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_stream_has_one_schedule_and_a_second_is_refused() {
    // The exchange is what orders activations, and two schedulers are two exchanges: a
    // candidate superseding one sequence could be accepted by each, and since adoption is
    // infallible both would then adopt. `SOUND-INV-018`'s first-adopted-wins rule is a
    // property of **one** exchange. An independent review found a second scheduler creating a
    // second one silently — a repair for it had made a second scheduler inherit the
    // transport state, which made the duplication work rather than removing it.
    //
    // A schedule is replaced by an **activation**, which is what ADR-0050 exists to provide.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("the first one prepares");
    assert!(control.has_scheduler());

    let refused = CompiledEventScheduler::prepare(&mut control, &quiet)
        .expect_err("a stream has one schedule");
    assert!(matches!(
        refused,
        synth_engine_v2::schedule::SchedulePrepareError::SchedulerExists
    ));
}

#[test]
fn the_release_scope_names_only_producers_that_admit_an_index() {
    // ADR-0050 calls the boundary release bounded. Admission charges the identity **span**,
    // so a plan may declare thousands of producers admitting nothing between them — and the
    // scope the audio thread walks has to be bounded by the span rather than by the
    // declaration count. A first repair skipped the *clear* inside the loop and left the
    // iteration per declaration; an independent review caught the comment claiming otherwise.
    // Filtering the list off-thread is what makes the claim true.
    let empty = many_empty_producers();
    let (mut control, _renderer) =
        StreamControl::open(empty.clone(), ORIGIN).expect("the stream opens");
    let quiet = AdmittedCompiledStream::admit(&empty, &[]).expect("an empty stream is admissible");
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");
    assert!(
        empty.note_producer_ranges().len() >= 4_096,
        "the fixture must declare enough producers for the distinction to matter"
    );

    let candidate = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    assert_eq!(
        candidate.producers(),
        0,
        "not one of four thousand declarations admits an index, so the scope is empty"
    );
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");

    // And a plan whose producer does admit indices still names it, so the filter is a bound
    // rather than a silence.
    let sounding = plan();
    let (mut control, _renderer) =
        StreamControl::open(sounding.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&sounding, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");
    let candidate = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    assert_eq!(candidate.producers(), 1);
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

#[test]
fn withdrawal_refuses_what_is_not_this_controls_to_drop() {
    // Dropping a foreign candidate would free the owning control's snapshot while
    // decrementing a counter that is not its own, leaving that control unable to stamp for
    // the rest of the stream. Dropping a **retirement** would destroy the only proof its
    // owner has that adoption happened. An independent review found both, and they are the
    // same mistake `adopted` already refuses.
    let plan = plan();
    let (mut mine, _my_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let (mut theirs, mut their_renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("a second stream opens");
    let mut their_arbiter = arbiter();

    let quiet = admitted(&plan, &[]);
    let _mine = CompiledEventScheduler::prepare(&mut mine, &quiet).expect("mine prepares");
    let their_quiet = admitted(&plan, &[]);
    let mut their_scheduler = CompiledEventScheduler::prepare(&mut theirs, &their_quiet)
        .expect("their stream prepares too");

    let theirs_candidate = theirs
        .plan_activation(&their_quiet, request(4 * Q, 0))
        .expect("their candidate builds");
    let (returned, error) = mine
        .withdraw(theirs_candidate)
        .expect_err("a candidate from another stream is not this control's to drop");
    assert!(matches!(
        error,
        synth_engine_v2::stream::ActivationCollectError::ForeignStream { .. }
    ));
    assert_eq!(
        mine.outstanding_candidates(),
        0,
        "and the refusal moved no counter of this control's"
    );

    // It goes back, so its owner can still account for it.
    theirs
        .withdraw(returned)
        .expect("its own control withdraws it");
    assert_eq!(theirs.outstanding_candidates(), 0);

    // A retirement is not a candidate either.
    let adopted = theirs
        .plan_activation(&their_quiet, request(0, 0))
        .expect("a second candidate builds");
    their_scheduler
        .offer(&mut their_renderer, adopted)
        .expect("the offer is accepted");
    let mut out = Vec::new();
    drive(
        &mut their_scheduler,
        &mut their_renderer,
        &mut their_arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );
    let retirement = their_scheduler.collect().expect("it was adopted");
    let (returned, error) = theirs
        .withdraw(retirement)
        .expect_err("a retirement carries the proof its control needs to promote");
    assert!(matches!(
        error,
        synth_engine_v2::stream::ActivationCollectError::AlreadyAdopted
    ));
    theirs
        .adopted(returned)
        .expect("and it still promotes, because withdrawal gave it back");
}

#[test]
fn a_retirement_reports_the_displacement_its_events_were_read_under() {
    // A retired schedule's event list carries the times it was **stamped** at; the schedule
    // added its activation shift at every read. Adoption overwrites the live shift, so a
    // retirement that did not carry its own would describe its events against a displacement
    // that is no longer anywhere. An independent review found the gap.
    //
    // The falsifier is an off-grid first activation: it displaces its stream, and the second
    // activation's retirement has to report that displacement rather than zero.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    // Off the grid: the boundary snaps forward, so the whole placed stream shifts with it.
    let first = control
        .plan_activation(&quiet, request(2 * Q + 17, 0))
        .expect("the first candidate builds");
    scheduler
        .offer(&mut renderer, first)
        .expect("the offer is accepted");
    let mut out = Vec::new();
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );
    let retired = scheduler.collect().expect("it was adopted");
    let effective = retired.effective().expect("it took effect");
    let displacement = effective.as_u64() - (2 * Q + 17);
    assert!(displacement > 0, "an off-grid request is displaced");
    control
        .adopted(retired)
        .expect("the control promotes its own retirement");

    // The second activation retires that displaced schedule.
    let second = control
        .plan_activation(&quiet, request(16 * Q, 0))
        .expect("the second candidate builds");
    scheduler
        .offer(&mut renderer, second)
        .expect("the offer is accepted");
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        16 * Q as usize,
        64,
        &mut out,
    );
    let retired = scheduler.collect().expect("the second was adopted");
    let state = retired
        .retired()
        .expect("an adopted activation reports what it replaced");
    assert_eq!(
        state.shift,
        synth_engine_v2::time::FrameCount::new(displacement),
        "the retirement carries the displacement its own events were read under"
    );
}

#[test]
fn an_empty_producer_before_a_sounding_one_does_not_hide_its_release() {
    // The release scope is compacted off-thread, so a loop counter over it is a **position**
    // and not a `ProducerId`. A defensive range lookup indexed by that counter read the wrong
    // producer's range whenever an empty declaration came first — and skipped the release for
    // the one that was actually sounding, leaving its note held for the rest of the stream.
    // An independent review found it, and it is the repository's own rule: a compacted
    // position may not stand in for a stable identity.
    //
    // **What this test does and does not falsify.** It pins the compaction — the scope names
    // one producer where the plan declares two — and the audible cut. It does **not**
    // falsify the skipped release itself: what the skip leaves behind is a live-note registry
    // entry, and the crate exposes no reader for one. That is stated rather than papered over
    // with an assertion that would pass for the wrong reason.
    let plan = empty_producer_then_sounding();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();

    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable");
    let held = AdmittedCompiledStream::admit(
        &plan,
        &[PlanEvent::new(
            PlanPosition::ZERO,
            CompiledPayload::NoteOn { slot },
        )],
    )
    .expect("the stream fits its share");
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &held).expect("the held note prepares");

    let quiet = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream is admissible");
    let activation = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    assert_eq!(
        activation.producers(),
        1,
        "the empty declaration is compacted out, which is what made the counter lie"
    );
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    let mut out = Vec::with_capacity(TOTAL);
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        8 * Q as usize,
        64,
        &mut out,
    );

    let on = Q as usize;
    let off = (4 * Q + Q) as usize;
    for (frame, sample) in out.iter().copied().enumerate() {
        let expected = if (on..off).contains(&frame) { 1.0 } else { 0.0 };
        assert_eq!(
            sample, expected,
            "frame {frame} is on the wrong side of the boundary release"
        );
    }
}

/// A plan whose first note-producer declaration admits nothing and whose second sounds.
fn empty_producer_then_sounding() -> CompiledPlan {
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).expect("not negative"),
                decay: Seconds::new(0.0).expect("not negative"),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.0).expect("not negative"),
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
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(synth_engine_v2::ir::PlanDeclarations {
            note_producers: vec![
                // Declared first and admits nothing: the entry the counter used to read.
                synth_engine_v2::ir::NoteProducerDeclaration {
                    compiled: false,
                    simultaneous_notes: synth_engine_v2::quantities::HeldNoteCount::NONE,
                    simultaneous_holds: EventCount::NONE,
                },
                synth_engine_v2::ir::NoteProducerDeclaration {
                    compiled: true,
                    simultaneous_notes: synth_engine_v2::quantities::HeldNoteCount::measured(8),
                    simultaneous_holds: EventCount::NONE,
                },
            ],
            held_notes: synth_engine_v2::quantities::HeldNoteCount::measured(8),
            ..synth_engine_v2::ir::PlanDeclarations::default()
        })
        .build()
        .expect("a readable plan");
    common::admit(&ir, common::profile(TOTAL as u64, ChannelLayout::Mono))
}

#[test]
fn the_release_scope_is_the_compiled_producer_alone() {
    // ADR-0050 clause 5 ends the notes of "a producer whose schedule the activation
    // replaces", and an activation replaces an `AdmittedCompiledStream` — nothing else. A
    // scope naming every declared producer would, once a non-compiled one can sound a note,
    // cut a performer's note on a seek: a seek moves plan time, it does not lift a finger. An
    // independent review found the over-wide scope, and clause 8 already puts a non-compiled
    // producer out of this record's reach.
    let plan = compiled_and_live_producers();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream is admissible");
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    assert_eq!(
        plan.note_producer_ranges().len(),
        2,
        "the plan declares two producers, both admitting indices"
    );
    assert!(
        plan.note_producer_ranges()
            .iter()
            .all(|range| range.get() > 0),
        "so the filter cannot be what narrows the scope"
    );

    let candidate = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("the candidate builds");
    assert_eq!(
        candidate.producers(),
        1,
        "only the producer whose schedule this activation replaces"
    );
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

/// A plan declaring one compiled note producer and one that is not, both admitting indices.
fn compiled_and_live_producers() -> CompiledPlan {
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).expect("not negative"),
                decay: Seconds::new(0.0).expect("not negative"),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.0).expect("not negative"),
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
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(synth_engine_v2::ir::PlanDeclarations {
            note_producers: vec![
                synth_engine_v2::ir::NoteProducerDeclaration {
                    compiled: true,
                    simultaneous_notes: synth_engine_v2::quantities::HeldNoteCount::measured(4),
                    simultaneous_holds: EventCount::NONE,
                },
                synth_engine_v2::ir::NoteProducerDeclaration {
                    compiled: false,
                    simultaneous_notes: synth_engine_v2::quantities::HeldNoteCount::measured(4),
                    simultaneous_holds: EventCount::measured(4),
                },
            ],
            held_notes: synth_engine_v2::quantities::HeldNoteCount::measured(8),
            ..synth_engine_v2::ir::PlanDeclarations::default()
        })
        .build()
        .expect("a readable plan");
    common::admit(&ir, common::profile(TOTAL as u64, ChannelLayout::Mono))
}

#[test]
fn a_seek_between_a_notes_edges_does_not_reopen_the_gate() {
    // The commonest seek there is, and the one a repair broke. The suffix omits the crossing
    // release, and the pairing counter was decremented to do it — which erased the only
    // record that the note had been open at the anchor. The final pass then saw a depth of
    // zero. The defect it caused belongs to clause 7's catch-up, which is not in this slice —
    // but the counter itself is, and a seek through a held note must still end that note.
    //
    // The falsifier is audible: silence after the boundary, not full scale.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let mut arbiter = arbiter();

    // One note, open across the destination.
    let crossing = admitted(&plan, &[note(&plan, 0, true), note(&plan, 16 * Q, false)]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &crossing).expect("the stream prepares");

    let activation = control
        .plan_activation(&crossing, request(4 * Q, 8 * Q))
        .expect("a seek through a held note builds");
    assert_eq!(activation.omitted_releases(), 1);
    scheduler
        .offer(&mut renderer, activation)
        .expect("the offer is accepted");

    let mut out = Vec::with_capacity(TOTAL);
    drive(
        &mut scheduler,
        &mut renderer,
        &mut arbiter,
        16 * Q as usize,
        64,
        &mut out,
    );

    let after = (4 * Q + Q) as usize;
    let loudest = out
        .get(after..)
        .expect("the render covers the boundary")
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        loudest < 0.01,
        "the seek re-opened the crossing note's gate with nothing left to release it: {loudest}"
    );
}

#[test]
fn a_history_note_edge_needs_a_producer_to_have_come_from() {
    // The suffix's edges are checked by stamping, but a history edge never reaches it — so a
    // plan with a playable node and no compiled note producer could build an activation whose
    // suffix was derived from edges nothing was entitled to emit, while preparing the very
    // same stream refused it. An independent review found the asymmetry.
    let plan = playable_without_a_producer();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream is admissible");
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable even with no producer declared");
    let edges = AdmittedCompiledStream::admit(
        &plan,
        &[
            PlanEvent::new(PlanPosition::ZERO, CompiledPayload::NoteOn { slot }),
            PlanEvent::new(PlanPosition::new(Q), CompiledPayload::NoteOff { slot }),
        ],
    )
    .expect("admission does not check producers");

    // Wholly before the destination, so nothing of this reaches stamping.
    let refusal = control
        .plan_activation(&edges, request(8 * Q, 8 * Q))
        .expect_err("a history edge is refused by the same rule stamping applies");
    assert!(
        matches!(
            refusal,
            synth_engine_v2::stream::ActivationBuildError::Stamp(
                synth_engine_v2::schedule::SchedulePrepareError::NoCompiledNoteProducer { .. }
            )
        ),
        "{refusal}"
    );
}

#[test]
fn an_offer_into_a_faulted_stream_is_refused_rather_than_trapped() {
    // After a terminal fault no later call advances toward a boundary, so an accepted
    // candidate would sit in the exchange forever: adoption never comes, `collect` never
    // yields it, and the control cannot withdraw a value it no longer holds. Reporting an
    // impossible state change as accepted is what this refuses. An independent review found
    // it.
    let plan = plan();
    let (mut control, mut renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");

    // A store too small for the callback below, which faults the stream terminally.
    let mut arbiter = PublicationArbiter::prepare(&common::profile(64, ChannelLayout::Mono))
        .expect("the smaller store is preparable");
    let quiet = admitted(&plan, &[]);
    let mut scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let mut samples = vec![0.0_f32; 1_024];
    let output =
        AudioBlockMut::new(&mut samples, 1_024, ChannelLayout::Mono).expect("a shaped block");
    let _fault = scheduler
        .render(&mut renderer, &mut arbiter, output)
        .expect_err("the window exceeds the prepared store");
    assert!(renderer.diagnostics().needs_reprepare(), "the stream ended");

    let candidate = control
        .plan_activation(&quiet, request(16 * Q, 0))
        .expect("the candidate still builds — the control does not know the stream ended");
    let (returned, refusal) = scheduler
        .offer(&mut renderer, candidate)
        .expect_err("a faulted stream cannot adopt anything");
    assert_eq!(refusal, ActivationRefused::StreamFaulted);

    // Which is what lets the control account for it rather than losing it.
    control
        .withdraw(returned)
        .expect("the control withdraws its own candidate");
    assert_eq!(control.outstanding_candidates(), 0);
}

/// A plan whose envelope is playable but which declares no note producer at all.
fn playable_without_a_producer() -> CompiledPlan {
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).expect("not negative"),
                decay: Seconds::new(0.0).expect("not negative"),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.0).expect("not negative"),
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
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a readable plan");
    common::admit(&ir, common::profile(TOTAL as u64, ChannelLayout::Mono))
}

#[test]
fn a_loop_is_judged_on_the_pass_that_repeats_not_the_suffix_it_carries() {
    // ADR-0046 clause 4 judges what **repeats**. An activation that enters a loop late carries
    // a suffix beginning at its own position, so the suffix skips everything in
    // `[loop_start, request.position)` -- which every wrap after the first replays. Judging it
    // admits an interval whose second pass overruns. An independent review found the subject.
    //
    // The construction is exact, and it is the wrap that makes the collision: the share is
    // ninety-six, the loop is `[100, 300)` and its length is 200. Ninety-six writes sit at 110
    // and one at 290, and the activation is requested at 260 -- past the ninety-six, so the
    // suffix carries only the write at 290. The repeating pass carries both, and its second
    // copy puts the ninety-six at 310, twenty frames from the single write at 290: ninety-seven
    // in one quantum window, and the interval is refused.
    //
    // The raw stream admits: 110 and 290 are 180 frames apart, so no window of it holds more
    // than the ninety-six writes. Only the loop can bring them together, which is why this is
    // `admit_loop`'s subject and not admission's.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");

    let gate = plan
        .resolve_parameter(ENVELOPE, synth_engine_v2::ir::parameters::ENVELOPE_GATE)
        .expect("the envelope declares a gate parameter");
    let write = |position: u64| {
        PlanEvent::new(
            PlanPosition::new(position),
            CompiledPayload::SetParameter {
                slot: gate,
                value: synth_engine_v2::quantities::ParameterValue::ZERO,
            },
        )
    };
    let mut events: Vec<PlanEvent> = (0..96).map(|_| write(110)).collect();
    events.push(write(290));
    let stream = admitted(&plan, &events);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");

    let interval = LoopInterval::new(PlanPosition::new(100), PlanPosition::new(300))
        .expect("a positive interval");
    let refusal = control
        .plan_activation(
            &stream,
            ActivationRequest {
                at: SampleTime::new(8 * Q),
                position: PlanPosition::new(260),
                loop_interval: Some(interval),
            },
        )
        .expect_err("the pass that repeats overruns the compiled share");
    assert!(
        matches!(
            refusal,
            ActivationBuildError::Loop {
                source: AdmissionError::LoopWindowOverShare { requested, share, .. },
                ..
            } if requested.get() == 97 && share.get() == 96
        ),
        "the writes the suffix skips are the ones that collide at the wrap: {refusal:?}"
    );
}

#[test]
fn a_crossing_release_still_counts_in_the_repeating_pass() {
    // ADR-0051 clause 5 omits a crossing release's **identity** and keeps its gate write: a
    // bare `SetParameter` of `ZERO` at the release's own position. So the position still
    // carries an event, and the repeating pass still has to count it. Treating the omission
    // as removing the event undercounts by one per crossing release and admits an interval
    // whose wrap would overrun the compiled share -- an admitted plan reaching a runtime
    // miss, which ADR-0046 clause 3 exists to remove. An independent review of the merge
    // found exactly that.
    //
    // The construction is exact. The share is ninety-six. A crossing release sits at 105 and
    // ninety-six writes at 250; the loop is `[100, 300)`, so the release's wrapped copy lands
    // at 305 -- fifty-five frames from the writes, inside one quantum window. Counting the
    // release gives ninety-seven against a share of ninety-six and the interval is refused;
    // dropping it gives ninety-six and the interval is wrongly admitted.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");

    let gate = plan
        .resolve_parameter(ENVELOPE, synth_engine_v2::ir::parameters::ENVELOPE_GATE)
        .expect("the envelope declares a gate parameter");
    let mut events = vec![note(&plan, 0, true), note(&plan, 105, false)];
    events.extend((0..96).map(|_| {
        PlanEvent::new(
            PlanPosition::new(250),
            CompiledPayload::SetParameter {
                slot: gate,
                value: synth_engine_v2::quantities::ParameterValue::ZERO,
            },
        )
    }));
    let stream = admitted(&plan, &events);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &stream).expect("the stream prepares");

    // Anchored at 100, so the note-on at 0 is history and the release at 105 is the crossing
    // one whose identity the suffix omits and whose gate write it keeps.
    let interval = LoopInterval::new(PlanPosition::new(100), PlanPosition::new(300))
        .expect("a positive interval");
    let refusal = control
        .plan_activation(
            &stream,
            ActivationRequest {
                at: SampleTime::new(4 * Q),
                position: PlanPosition::new(100),
                loop_interval: Some(interval),
            },
        )
        .expect_err("the repeating pass overruns the compiled share");
    assert!(
        matches!(
            refusal,
            ActivationBuildError::Loop {
                source: AdmissionError::LoopWindowOverShare { requested, share, .. },
                ..
            } if requested.get() == 97 && share.get() == 96
        ),
        "the crossing release is the ninety-seventh event: {refusal:?}"
    );
}

#[test]
fn an_activation_needs_a_schedule_to_replace() {
    // A candidate needs a schedule to be offered to, and one that does not exist yet cannot
    // be created afterwards: preparing it stamps against the authoritative minter, which an
    // outstanding candidate holds still. Building here would hand back a value whose only
    // legal next step is withdrawal. An independent review found the dead end.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);

    let refusal = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect_err("there is nothing to replace yet");
    assert!(matches!(
        refusal,
        synth_engine_v2::stream::ActivationBuildError::NoSchedule
    ));
    assert_eq!(
        control.outstanding_candidates(),
        0,
        "and the refusal left nothing outstanding to block the schedule"
    );

    // Which is what keeps the stream usable: the schedule prepares, and then it builds.
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("the schedule prepares");
    let candidate = control
        .plan_activation(&quiet, request(4 * Q, 0))
        .expect("and now there is something to replace");
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_history_holding_more_notes_than_the_producer_admits_is_refused() {
    // The history never reaches stamping, so without a bound of its own a timeline the
    // producer was never entitled to emit would still decide which crossing releases the
    // suffix omits — and `plan_activation` would accept a stream that preparing refuses. An
    // independent review found the asymmetry.
    //
    // The falsifier needs a partition small enough to exceed: one simultaneous note, and two
    // note-ons open at once before the destination.
    let plan = single_note_plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    // Two notes open at once, both wholly before the destination.
    let over = admitted(
        &plan,
        &[
            note(&plan, 0, true),
            note(&plan, Q, true),
            note(&plan, 2 * Q, false),
            note(&plan, 3 * Q, false),
        ],
    );
    let refusal = control
        .plan_activation(&over, request(8 * Q, 8 * Q))
        .expect_err("the history holds more notes than the producer admits");
    assert!(
        matches!(
            refusal,
            synth_engine_v2::stream::ActivationBuildError::Stamp(
                synth_engine_v2::schedule::SchedulePrepareError::Identity { .. }
            )
        ),
        "{refusal}"
    );

    // One at a time is fine, which is what makes the refusal a bound rather than a ban.
    let within = admitted(
        &plan,
        &[
            note(&plan, 0, true),
            note(&plan, Q, false),
            note(&plan, 2 * Q, true),
            note(&plan, 3 * Q, false),
        ],
    );
    let candidate = control
        .plan_activation(&within, request(8 * Q, 8 * Q))
        .expect("a history that never exceeds the range builds");
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_loop_is_admitted_against_the_pass_that_repeats_not_the_one_that_starts() {
    // An activation may enter a loop late. The first pass then skips the events between the
    // loop's start and the seek destination — but every wrap after it re-anchors at the
    // loop's start and plays them, so a skipped prefix is part of what repeats. An
    // independent review found the check reading the first pass and letting one through.
    //
    // The construction is exact. The share is ninety-six, the loop is `[0, 300)` and the
    // destination is 200. Ninety-six writes sit at 250 and one at 10; the first pass carries
    // only the ninety-six, but the wrap puts the lone one at 310 — sixty frames from the
    // others, inside one quantum window — so every repeat holds ninety-seven.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let gate = plan
        .resolve_parameter(ENVELOPE, synth_engine_v2::ir::parameters::ENVELOPE_GATE)
        .expect("the envelope declares a gate parameter");
    let write = |position: u64| {
        PlanEvent::new(
            PlanPosition::new(position),
            CompiledPayload::SetParameter {
                slot: gate,
                value: synth_engine_v2::quantities::ParameterValue::ZERO,
            },
        )
    };
    let interval = LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(300)).expect("positive");
    let late_entry = |position: u64| ActivationRequest {
        at: SampleTime::new(4 * Q),
        position: PlanPosition::new(position),
        loop_interval: Some(interval),
    };

    let mut events = vec![write(10)];
    events.extend((0..96).map(|_| write(250)));
    let stream = admitted(&plan, &events);
    let refusal = control
        .plan_activation(&stream, late_entry(200))
        .expect_err("every wrap replays the write this first pass skips");
    assert!(
        matches!(
            refusal,
            synth_engine_v2::stream::ActivationBuildError::Loop { .. }
        ),
        "{refusal}"
    );

    // The very same stream, entered at the loop's own start, is refused for the same reason —
    // so the check is about what repeats rather than about entering late.
    assert!(
        control.plan_activation(&stream, late_entry(0)).is_err(),
        "the repeating pass is the subject either way"
    );

    // One fewer at 250 and the wrap fits, which makes the refusal a bound rather than a ban.
    let mut fits = vec![write(10)];
    fits.extend((0..95).map(|_| write(250)));
    let within = admitted(&plan, &fits);
    let candidate = control
        .plan_activation(&within, late_entry(200))
        .expect("ninety-six across the wrap is exactly the share");
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_crossing_notes_gate_is_cut_whether_or_not_automation_touched_it() {
    // ADR-0051 clause 1, and this is the decision itself rather than a consequence of it.
    //
    // A gate is edge-triggered, so automation writing a raised gate while a note already
    // holds it is **inert while playing through** — the kernel returns early on a re-asserted
    // level. The locate is where that stops being true: the boundary mass release lowers the
    // gate, and the catch-up publishes after those gate-downs, so a batch carrying the raised
    // value would make the pair a **rising edge** and re-attack an envelope with no note
    // contract behind it. That is the note chasing ADR-0050 clause 5 declines.
    //
    // Two falsifiers, and they catch different wrong implementations — which is why both are
    // asserted rather than one standing in for the other.
    //
    // The **equality** catches the abandoned design: distinguishing a gate whose last write
    // came from a note edge from one whose last write came from automation, and restoring the
    // automated value. That makes the two streams below differ, because they differ in exactly
    // that write.
    //
    // The **silence** catches dropping the substitution altogether. Both streams would then
    // restore the raised gate their note-on left in the history, so they would still be equal
    // — equal and both wrong. Mutation-verified: disabling the substitution leaves the
    // equality passing and fails here.
    let render_seek = |automated: bool| -> Vec<f32> {
        let plan = plan();
        let (mut control, mut renderer) =
            StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
        let mut arbiter = arbiter();
        let quiet = admitted(&plan, &[]);
        let mut scheduler = CompiledEventScheduler::prepare(&mut control, &quiet)
            .expect("an empty stream prepares");

        let gate = plan
            .resolve_parameter(ENVELOPE, synth_engine_v2::ir::parameters::ENVELOPE_GATE)
            .expect("the envelope declares a gate parameter");
        let mut events = vec![note(&plan, 0, true)];
        if automated {
            // Raises a gate the note already holds: no edge, so playing through cannot hear
            // it. Only a locate can turn it into one.
            events.push(PlanEvent::new(
                PlanPosition::new(4 * Q),
                CompiledPayload::SetParameter {
                    slot: gate,
                    value: synth_engine_v2::quantities::ParameterValue::ONE,
                },
            ));
        }
        events.push(note(&plan, 32 * Q, false));
        let stream = admitted(&plan, &events);

        let activation = control
            .plan_activation(&stream, request(4 * Q, 8 * Q))
            .expect("the seek builds");
        scheduler
            .offer(&mut renderer, activation)
            .expect("the offer is accepted");

        let mut out = Vec::new();
        drive(
            &mut scheduler,
            &mut renderer,
            &mut arbiter,
            16 * Q as usize,
            64,
            &mut out,
        );
        out
    };

    let plain = render_seek(false);
    let automated = render_seek(true);
    assert_eq!(
        plain, automated,
        "ADR-0051 clause 1: a gate held open by a note contract at the destination is owed \
         ZERO whatever the last write before it was, so an inert gate write cannot become \
         audible by seeking past it"
    );
    assert!(
        automated.iter().all(|sample| sample.abs() < 1e-6),
        "and the note is cut rather than resumed: ADR-0050 clause 5's audible consequence, \
         which a re-attack at the destination would contradict"
    );
}

/// Two gated stages in series, so a plan has **two** note slots.
///
/// One slot cannot exhibit ADR-0051 clause 5's crossing release at all: a release takes the
/// most recent unclosed on edge for its own slot, so on a single-slot plan the crossing
/// branch is reachable only when nothing is open — and then no count it could lower is
/// non-zero. Two slots let one note be open while the other's release crosses into the pass.
fn two_gated_stages(simultaneous: u32) -> GraphIr {
    GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Constant {
                level: Amplitude::new(1.0).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).expect("not negative"),
                decay: Seconds::new(0.0).expect("not negative"),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.0).expect("not negative"),
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(
            SECOND_ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.0).expect("not negative"),
                decay: Seconds::new(0.0).expect("not negative"),
                sustain: NormalizedLevel::FULL,
                release: Seconds::new(0.0).expect("not negative"),
            },
            ExecutionScope::Voice,
        )
        .node(
            SECOND_AMPLIFIER,
            IrNodeKind::Amplifier,
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (SECOND_AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (SECOND_ENVELOPE, PortId::FIRST),
            (SECOND_AMPLIFIER, synth_engine_v2::node::AMPLIFIER_CONTROL),
            SignalDomain::Control,
        )
        .connect(
            (SECOND_AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .declaring(common::compiled_notes(simultaneous))
        .build()
        .expect("a readable plan")
}

fn note_on_node(plan: &CompiledPlan, node: NodeId, position: u64, on: bool) -> PlanEvent {
    let slot = plan.resolve_note(node).expect("the node is playable");
    let payload = if on {
        CompiledPayload::NoteOn { slot }
    } else {
        CompiledPayload::NoteOff { slot }
    };
    PlanEvent::new(PlanPosition::new(position), payload)
}

#[test]
fn a_loops_repeating_pass_is_admitted_against_the_producers_simultaneous_notes() {
    // `SOUND-INV-017`'s producer range, applied to the one timeline that had no enforcement
    // point. ADR-0046 clause 4's window scan bounds a pass's **events per quantum** against
    // the compiled share and says nothing about how many notes it holds open at once, so a
    // loop that is sparse everywhere can still need more identity than its producer has. The
    // history walk and `stamp_into` already apply the range rule to their own timelines; the
    // pass a wrap replays is a third. Recorded without this check, such a loop over-emits at
    // its first real wrap. A design consultation for the wrap slice found the gap.
    //
    // **The subject is the pass a wrap replays, and this construction is chosen so that
    // nothing else can see it.** One note opens at `Q`, before the destination, so it is
    // history: the anchored walk counts one open note there, which is exactly what the
    // producer admits. A second opens at `5Q`, after the destination, so the suffix carries
    // one note-on and stamping mints one. Neither check sees two. The pass a wrap replays
    // starts at the loop's own start and plays both, so it holds two at `5Q` against a
    // producer admitted for one.
    let plan = single_note_plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let interval =
        LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(8 * Q)).expect("positive");
    let entering_late = ActivationRequest {
        at: SampleTime::new(16 * Q),
        position: PlanPosition::new(4 * Q),
        loop_interval: Some(interval),
    };

    let over = admitted(
        &plan,
        &[
            note(&plan, Q, true),
            note(&plan, 5 * Q, true),
            note(&plan, 6 * Q, false),
            note(&plan, 7 * Q, false),
        ],
    );
    let refusal = control
        .plan_activation(&over, entering_late)
        .expect_err("every wrap replays both note-ons, and the producer admits one");
    match refusal {
        ActivationBuildError::Loop {
            start,
            end,
            source:
                AdmissionError::LoopPolyphonyOverProducer {
                    requested,
                    admitted: admits,
                    ..
                },
        } => {
            assert_eq!(start, interval.start());
            assert_eq!(end, interval.end());
            assert_eq!(requested.get(), 2, "both note-ons are open at 5Q");
            assert_eq!(admits.get(), 1, "and the compiled producer admits one");
        }
        other => panic!("the loop's polyphony is what refuses this: {other}"),
    }

    // The same stream without a loop is **built**, which is what makes this the loop's
    // refusal rather than the stream's: with no interval there is no pass to repeat, the
    // history holds one note and the suffix mints one.
    let candidate = control
        .plan_activation(&over, request(16 * Q, 4 * Q))
        .expect("neither the history nor the suffix holds more than one");
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");

    // And moving the first note-on inside the second's pairing — so the pass never holds two
    // at once — is admitted with the loop in force. The refusal is a bound, not a ban.
    let within = admitted(
        &plan,
        &[
            note(&plan, Q, true),
            note(&plan, 2 * Q, false),
            note(&plan, 5 * Q, true),
            note(&plan, 6 * Q, false),
        ],
    );
    let candidate = control
        .plan_activation(&within, entering_late)
        .expect("one at a time repeats within the producer's range");
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_repeating_pass_starts_with_nothing_sounding() {
    // ADR-0050 clause 5's boundary mass release is what makes this true: a wrap ends the
    // notes the previous pass opened, so the pass a wrap replays begins with nothing
    // sounding. Seeding the count from the depth at `loop_start` instead would charge every
    // note twice — once where it opens and once in every later pass — and refuse loops that
    // hold nothing of their own.
    //
    // Eight notes are open where the loop starts, which is exactly what this producer admits,
    // and the loop's own pass opens one more. Under the rule the pass holds one; under the
    // inherited depth it would hold nine and be refused.
    let plan = plan();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let mut events: Vec<PlanEvent> = (0..8).map(|n| note(&plan, 2 * Q * n, true)).collect();
    events.push(note(&plan, 20 * Q, true));
    events.push(note(&plan, 22 * Q, false));
    let stream = admitted(&plan, &events);

    let interval =
        LoopInterval::new(PlanPosition::new(16 * Q), PlanPosition::new(40 * Q)).expect("positive");
    let candidate = control
        .plan_activation(
            &stream,
            ActivationRequest {
                at: SampleTime::new(64 * Q),
                position: PlanPosition::new(16 * Q),
                loop_interval: Some(interval),
            },
        )
        .expect("the pass holds the one note it opens, not the eight it inherits");
    control
        .withdraw(candidate)
        .expect("the control withdraws its own candidate");
}

#[test]
fn a_crossing_release_lowers_no_note_in_the_repeating_pass() {
    // ADR-0051 clause 5. A release whose on edge lies before the interval carries a bare
    // gate-down and **no note contract**, so it ends nothing the pass is holding. Letting it
    // decrement would hand the pass a note's worth of headroom it never had.
    //
    // Two note slots, because one cannot show it: a release takes the most recent unclosed on
    // edge for its own slot, so on a single-slot plan the crossing branch is reachable only
    // when that slot holds nothing — and then the count it would lower is already zero.
    //
    // **Two admitted notes, for a sharper reason.** With one, the construction is
    // unbuildable: the crossing note is open where the loop starts, so any pass note opening
    // before the destination is open beside it in the history, and the history's own bound
    // refuses the stream before the loop is ever judged.
    //
    // The first stage's note opens at `Q`, before the loop, and is released at `10Q` inside
    // it. The second stage opens at `6Q`, `14Q` and `16Q`. The history never holds more than
    // two, and the suffix — everything from `12Q` — mints two. The pass holds three at `16Q`
    // only because the crossing release lowered nothing; if it had, the pass would hold two
    // and this stream would be admitted.
    let plan = common::admit(
        &two_gated_stages(2),
        common::profile(TOTAL as u64, ChannelLayout::Mono),
    );
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = admitted(&plan, &[]);
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let interval =
        LoopInterval::new(PlanPosition::new(4 * Q), PlanPosition::new(24 * Q)).expect("positive");
    let entering_late = ActivationRequest {
        at: SampleTime::new(64 * Q),
        position: PlanPosition::new(12 * Q),
        loop_interval: Some(interval),
    };
    let stream = admitted(
        &plan,
        &[
            note_on_node(&plan, ENVELOPE, Q, true),
            note_on_node(&plan, SECOND_ENVELOPE, 6 * Q, true),
            note_on_node(&plan, ENVELOPE, 10 * Q, false),
            note_on_node(&plan, SECOND_ENVELOPE, 14 * Q, true),
            note_on_node(&plan, SECOND_ENVELOPE, 16 * Q, true),
            note_on_node(&plan, SECOND_ENVELOPE, 18 * Q, false),
            note_on_node(&plan, SECOND_ENVELOPE, 20 * Q, false),
            note_on_node(&plan, SECOND_ENVELOPE, 22 * Q, false),
        ],
    );

    let refusal = control
        .plan_activation(&stream, entering_late)
        .expect_err("the crossing release ends nothing the pass holds");
    match refusal {
        ActivationBuildError::Loop {
            source: AdmissionError::LoopPolyphonyOverProducer { requested, .. },
            ..
        } => assert_eq!(
            requested.get(),
            3,
            "the first stage's note is still counted where the second opens its third"
        ),
        other => panic!("the crossing release must not buy the pass headroom: {other}"),
    }
}

#[test]
fn a_loop_does_not_reclassify_a_note_that_has_no_producer_at_all() {
    // A plan declaring **no** compiled note producer admits nothing because there is nothing
    // to admit, which is a different fact from a producer whose range is empty. Comparing the
    // repeating pass against a zero admitted count would report the first as the second, and
    // — because the comparison runs before stamping — it would classify one invalid note two
    // ways depending on whether a loop interval was supplied. An independent review found it.
    //
    // The note edges sit **after** the destination, so the history's own refusal cannot be
    // what answers: this is the suffix, and `stamp_into` is what must still refuse it.
    let plan = playable_without_a_producer();
    let (mut control, _renderer) =
        StreamControl::open(plan.clone(), ORIGIN).expect("the stream opens");
    let quiet = AdmittedCompiledStream::admit(&plan, &[]).expect("an empty stream is admissible");
    let _scheduler =
        CompiledEventScheduler::prepare(&mut control, &quiet).expect("an empty stream prepares");

    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("the envelope is playable even with no producer declared");
    let edges = AdmittedCompiledStream::admit(
        &plan,
        &[
            PlanEvent::new(PlanPosition::new(2 * Q), CompiledPayload::NoteOn { slot }),
            PlanEvent::new(PlanPosition::new(3 * Q), CompiledPayload::NoteOff { slot }),
        ],
    )
    .expect("admission does not check producers");

    let refusal = control
        .plan_activation(
            &edges,
            ActivationRequest {
                at: SampleTime::new(8 * Q),
                position: PlanPosition::ZERO,
                loop_interval: Some(
                    LoopInterval::new(PlanPosition::ZERO, PlanPosition::new(8 * Q))
                        .expect("positive"),
                ),
            },
        )
        .expect_err("a note with no producer is refused whether or not a loop is in force");
    assert!(
        matches!(
            refusal,
            ActivationBuildError::Stamp(
                synth_engine_v2::schedule::SchedulePrepareError::NoCompiledNoteProducer { .. }
            )
        ),
        "the missing producer keeps its own diagnostic: {refusal}"
    );
}
