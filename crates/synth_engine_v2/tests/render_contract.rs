//! The Phase 1 exit gate's render bullets, and the ADR-0001 clauses behind them.

mod common;

use common::{OUTPUT, SOURCE, admit, profile, rate, source_plan};

const ENVELOPE: NodeId = NodeId::new(11);
const AMPLIFIER: NodeId = NodeId::new(12);

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::diagnostics::RenderError;
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::{CompiledPlan, ParameterSlot};
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, Frequency, NormalizedLevel, ParameterValue, Seconds,
};
use synth_engine_v2::render::{
    AudioBlockMut, EventEnvelope, EventPayload, NoteEdge, PreparedRenderer, Renderer, TimedEvent,
    TimedEvents,
};
use synth_engine_v2::time::{
    FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor, StreamEpoch, TimeSource,
};

/// Render `frames` frames in blocks of `block`, live (so the priming head is kept).
fn render_live(plan: CompiledPlan, frames: usize, block: usize) -> Vec<f32> {
    let layout = plan.channel_layout();
    let channels = layout.channels();
    let mut renderer = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");

    let mut out = Vec::with_capacity(frames * channels);
    let mut scratch = vec![0.0_f32; block * channels];
    let mut produced = 0;
    while produced < frames {
        let this = block.min(frames - produced);
        let region = &mut scratch[..this * channels];
        region.fill(0.0);
        let output = AudioBlockMut::new(region, this, layout).expect("shaped block");
        renderer
            .render(output, TimedEvents::EMPTY)
            .expect("a block within the maximum renders");
        out.extend_from_slice(&scratch[..this * channels]);
        produced += this;
    }
    out
}

#[test]
fn an_empty_plan_renders_silence_deterministically() {
    let host = profile(512, ChannelLayout::Stereo);
    let first = render_live(admit(&GraphIr::empty(), host), 1_024, 512);
    let second = render_live(admit(&GraphIr::empty(), host), 1_024, 512);

    assert_eq!(first.len(), 1_024 * 2);
    assert!(
        first.iter().all(|sample| *sample == 0.0),
        "an empty plan renders silence"
    );
    assert_eq!(first, second, "two renders of one plan must be identical");
}

#[test]
fn a_constant_source_renders_deterministically() {
    let host = profile(512, ChannelLayout::Stereo);
    let ir = source_plan(IrNodeKind::Constant {
        level: Amplitude::new(0.25).expect("finite"),
    });
    let first = render_live(admit(&ir, host), 512, 512);
    let second = render_live(admit(&ir, host), 512, 512);
    assert_eq!(first, second);

    // The first `Q` frames are the priming silence of ADR-0001 clause 6; after them the
    // constant is present. This is the live path, which carries the latency clause 7
    // charges.
    let quantum = QUANTUM_FRAMES as usize;
    assert!(first[..quantum * 2].iter().all(|sample| *sample == 0.0));
    assert!(
        first[quantum * 2..]
            .iter()
            .all(|sample| (*sample - 0.25).abs() < f32::EPSILON),
        "after the priming head the constant must be on every channel"
    );
}

#[test]
fn a_sine_source_renders_deterministically_and_audibly() {
    let host = profile(512, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(440.0).expect("finite"),
        amplitude: Amplitude::new(0.5).expect("finite"),
    });
    let first = render_live(admit(&ir, host), 2_048, 512);
    let second = render_live(admit(&ir, host), 2_048, 512);
    assert_eq!(first, second, "the sine must be reproducible");

    let peak = first.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
    assert!(
        peak > 0.4,
        "a 0.5-amplitude sine must be audible, not silence dressed as a render; peak was {peak}"
    );
    assert!(
        peak <= 0.5 + f32::EPSILON,
        "and must not exceed its amplitude"
    );
}

#[test]
fn varying_caller_block_sizes_produce_the_same_audio() {
    // The exit gate's second bullet, and the property ADR-0001 exists for: the
    // sequence of quanta is identical however the caller chops its requests, because
    // the renderer never shortens a quantum to fit a block.
    let host = profile(4_096, ChannelLayout::Stereo);
    let ir = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(997.0).expect("finite"),
        amplitude: Amplitude::new(0.8).expect("finite"),
    });
    let frames = 4_096;

    let reference = render_live(admit(&ir, host), frames, 4_096);
    for block in [1_usize, 7, 63, 64, 65, 128, 256, 1_024, 4_096] {
        let partitioned = render_live(admit(&ir, host), frames, block);
        assert_eq!(
            partitioned, reference,
            "a {block}-frame partition rendered different audio than one {frames}-frame block"
        );
    }
}

#[test]
fn a_maximum_block_below_one_quantum_is_admitted_and_renders_the_same_audio() {
    // `HOST-INV-012`: a host whose largest block is smaller than one quantum is
    // supported unchanged. A `maximum_block_size >= Q` clause would have refused a host
    // the render model was built for.
    let small = profile(16, ChannelLayout::Mono);
    let large = profile(4_096, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(220.0).expect("finite"),
        amplitude: Amplitude::new(0.3).expect("finite"),
    });

    let tiny_blocks = render_live(admit(&ir, small), 1_024, 16);
    let one_block = render_live(admit(&ir, large), 1_024, 1_024);
    assert_eq!(
        tiny_blocks, one_block,
        "a 16-frame host must hear what a 1024-frame host hears"
    );
}

#[test]
fn an_impulse_at_plan_sample_zero_lands_at_output_sample_zero() {
    // ADR-0001's named risk control. A render path that skips the offline trim emits
    // audio shifted by `Q` frames — still valid audio, which no listening test catches.
    let host = profile(256, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Impulse {
        position: PlanPosition::ZERO,
    });
    let rendered = render_offline(
        admit(&ir, host),
        FrameCount::new(512),
        PlanPosition::ZERO,
        &[],
    )
    .expect("the offline path renders");

    assert_eq!(
        rendered.len(),
        512,
        "an offline request returns exactly N frames"
    );
    assert_eq!(
        rendered[0], 1.0,
        "plan sample 0 must be output sample 0; a shifted render would put it at {}",
        QUANTUM_FRAMES
    );
    assert!(
        rendered[1..].iter().all(|sample| *sample == 0.0),
        "and nowhere else"
    );
}

#[test]
fn an_impulse_inside_the_range_lands_on_its_own_frame() {
    let host = profile(256, ChannelLayout::Mono);
    for at in [1_u64, 63, 64, 65, 200, 511] {
        let ir = source_plan(IrNodeKind::Impulse {
            position: PlanPosition::new(at),
        });
        let rendered = render_offline(
            admit(&ir, host),
            FrameCount::new(512),
            PlanPosition::ZERO,
            &[],
        )
        .expect("the offline path renders");
        let hit = rendered.iter().position(|sample| *sample == 1.0);
        assert_eq!(
            hit,
            Some(at as usize),
            "an impulse declared at plan sample {at} landed at {hit:?}"
        );
    }
}

#[test]
fn an_offline_range_starting_late_is_anchored_to_its_start() {
    // ADR-0032 clause 27: the offline range's start is an anchor, and the engine clock
    // still begins at zero. A harness that treated one as the other would reintroduce
    // the shift the test above exists to catch.
    let host = profile(256, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Impulse {
        position: PlanPosition::new(1_000),
    });
    let rendered = render_offline(
        admit(&ir, host),
        FrameCount::new(256),
        PlanPosition::new(900),
        &[],
    )
    .expect("the offline path renders");
    assert_eq!(
        rendered.iter().position(|sample| *sample == 1.0),
        Some(100),
        "plan sample 1000 is 100 frames into a range starting at 900"
    );
}

#[test]
fn offline_and_live_render_the_same_content_and_differ_only_by_the_carry_delay() {
    // ADR-0001 clause 10. V1 cannot claim this: its offline path evaluates control at a
    // different rate than its live path, so the two are not the same signal at all.
    let host = profile(512, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(330.0).expect("finite"),
        amplitude: Amplitude::new(0.4).expect("finite"),
    });
    let quantum = QUANTUM_FRAMES as usize;

    let live = render_live(admit(&ir, host), 1_024 + quantum, 512);
    let offline = render_offline(
        admit(&ir, host),
        FrameCount::new(1024),
        PlanPosition::ZERO,
        &[],
    )
    .expect("offline renders");

    assert_eq!(
        &live[quantum..quantum + 1_024],
        &offline[..],
        "live output is the offline output delayed by exactly one quantum"
    );
}

#[test]
fn a_prepared_plan_renders_after_its_profile_is_dropped() {
    // `HOST-INV-001` and `HOST-INV-002`: the renderer reads the prepared plan and never
    // the profile, so dropping the profile cannot change what a stream does.
    let plan = {
        let host = profile(256, ChannelLayout::Mono);
        let ir = source_plan(IrNodeKind::Constant {
            level: Amplitude::new(0.5).expect("finite"),
        });
        let plan = admit(&ir, host);
        // `HostProfile` is `Copy`, so there is no destructor to run and no borrow to
        // end: what the test asserts is stronger and structural — the renderer holds no
        // reference to a profile at all, and the block below is where the only one goes
        // out of scope.
        plan
    };

    let mut renderer = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let mut samples = vec![0.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(output, TimedEvents::EMPTY)
        .expect("a plan renders without its profile");
    assert!(samples.contains(&0.5));
}

#[test]
fn an_oversized_callback_is_a_terminal_stream_fault() {
    // ADR-0021 part 3: silence, both carries invalidated, `needs_reprepare` published,
    // nothing allocated — and the epoch is over, so the next call refuses too.
    let host = profile(128, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Constant {
        level: Amplitude::new(1.0).expect("finite"),
    });
    let mut renderer = PreparedRenderer::prepare(
        admit(&ir, host),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");

    let mut samples = vec![7.0_f32; 129];
    let output = AudioBlockMut::new(&mut samples, 129, ChannelLayout::Mono).expect("shaped block");
    let error = renderer
        .render(output, TimedEvents::EMPTY)
        .expect_err("129 frames is beyond a 128-frame maximum");

    assert!(matches!(error, RenderError::OversizedCallback { .. }));
    assert!(
        samples.iter().all(|sample| *sample == 0.0),
        "the fault must silence the output rather than leaving stale samples"
    );
    assert_eq!(renderer.diagnostics().oversized_callback_faults(), 1);
    assert!(renderer.diagnostics().needs_reprepare());

    let mut again = vec![0.0_f32; 64];
    let output = AudioBlockMut::new(&mut again, 64, ChannelLayout::Mono).expect("shaped block");
    assert_eq!(
        renderer.render(output, TimedEvents::EMPTY),
        Err(RenderError::NeedsReprepare),
        "the epoch is over; recovery is re-preparation"
    );
}

#[test]
fn a_wrongly_shaped_output_block_is_refused_before_anything_is_rendered() {
    let host = profile(128, ChannelLayout::Stereo);
    let mut samples = vec![0.0_f32; 100];
    assert!(matches!(
        AudioBlockMut::new(&mut samples, 64, ChannelLayout::Stereo),
        Err(RenderError::OutputBufferShape { needed: 128, .. })
    ));
    // And the layout has to be the stream's.
    let ir = source_plan(IrNodeKind::Constant {
        level: Amplitude::new(0.5).expect("finite"),
    });
    let mut renderer = PreparedRenderer::prepare(
        admit(&ir, host),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let mut mono = vec![0.0_f32; 64];
    let output = AudioBlockMut::new(&mut mono, 64, ChannelLayout::Mono).expect("shaped block");
    assert!(matches!(
        renderer.render(output, TimedEvents::EMPTY),
        Err(RenderError::OutputBufferShape { .. })
    ));
}

/// A renderer over a sine, plus its epoch.
fn sine_renderer(block: u64) -> (PreparedRenderer, StreamEpoch) {
    let host = profile(block, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(440.0).expect("finite"),
        amplitude: Amplitude::new(0.5).expect("finite"),
    });
    let renderer = PreparedRenderer::prepare(
        admit(&ir, host),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();
    (renderer, epoch)
}

/// The compiled slot for the source sine's frequency.
///
/// Resolved from the plan rather than assumed: an address becomes a slot off the audio
/// thread, and a test that hard-coded the index would stop testing the resolution.
fn frequency_slot(renderer: &PreparedRenderer) -> ParameterSlot {
    renderer
        .plan()
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("the sine declares a frequency parameter")
}

fn set_frequency(
    slot: ParameterSlot,
    epoch: StreamEpoch,
    at: u64,
    value: f32,
    source: TimeSource,
) -> TimedEvent {
    TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::new(at), source),
        EventPayload::SetParameter {
            slot,
            value: ParameterValue::new(value).expect("a finite test value"),
        },
    )
}

#[test]
fn an_event_from_another_epoch_is_discarded_and_counted() {
    // ADR-0032 clause 20. Without this, an event stamped microseconds before a
    // re-preparation is applied against a clock that restarted at zero.
    let (mut renderer, epoch) = sine_renderer(256);
    let stale = StreamEpoch::from_raw(epoch.as_u32().wrapping_sub(1));
    let events = [set_frequency(
        frequency_slot(&renderer),
        stale,
        0,
        100.0,
        TimeSource::Compiled,
    )];

    let mut samples = vec![0.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(output, TimedEvents::new(&events))
        .expect("a stale event does not fail the call");
    assert_eq!(renderer.diagnostics().stale_epoch_events(), 1);
}

#[test]
fn an_ingress_event_beyond_the_forward_horizon_is_rejected_and_counted() {
    // ADR-0032 clause 21, and it binds ingress provenance only.
    let (mut renderer, epoch) = sine_renderer(256);
    let horizon = renderer.plan().forward_event_horizon().as_u64();
    let events = [set_frequency(
        frequency_slot(&renderer),
        epoch,
        horizon + 1,
        100.0,
        TimeSource::Hardware,
    )];

    let mut samples = vec![0.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(output, TimedEvents::new(&events))
        .expect("a rejected ingress event does not fail the call");
    assert_eq!(renderer.diagnostics().out_of_horizon_events(), 1);
}

#[test]
fn an_arrival_stamped_event_is_counted_as_such() {
    // ADR-0032 clause 19: an adapter with no hardware timestamp declares its fallback,
    // and the declaration reaches the diagnostics report.
    let (mut renderer, epoch) = sine_renderer(256);
    let events = [set_frequency(
        frequency_slot(&renderer),
        epoch,
        0,
        200.0,
        TimeSource::Arrival,
    )];

    let mut samples = vec![0.0_f32; 256];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(output, TimedEvents::new(&events))
        .expect("an arrival-stamped event is ordinary");
    assert_eq!(renderer.diagnostics().arrival_stamped_events(), 1);
    assert_eq!(
        renderer.diagnostics().late_events(),
        0,
        "provenance is not lateness"
    );
}

#[test]
fn a_late_event_is_clamped_forward_and_counted() {
    // ADR-0001 clause 16. The trigger is a *condition* — the timestamp fell in a
    // quantum that has already rendered — and not a cause.
    let (mut renderer, epoch) = sine_renderer(256);
    let mut samples = vec![0.0_f32; 256];

    // First call renders quanta 0..2, so the clock passes sample 0.
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(output, TimedEvents::EMPTY)
        .expect("the first call renders");
    assert!(renderer.clock().as_u64() > 0);

    let events = [set_frequency(
        frequency_slot(&renderer),
        epoch,
        0,
        110.0,
        TimeSource::Compiled,
    )];
    let output = AudioBlockMut::new(&mut samples, 256, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(output, TimedEvents::new(&events))
        .expect("a late event is clamped, never dropped");
    assert_eq!(renderer.diagnostics().late_events(), 1);
    assert_eq!(
        renderer.diagnostics().pre_epoch_clamps(),
        0,
        "the pre-epoch clamp is the ingress mapper's counter and Phase 3's work"
    );
}

/// A gated constant, so the frame an edge landed on is an exact value rather than a shape.
///
/// Every envelope segment is instantaneous and the sustain level is one, so the rendered
/// signal is `0.0` before the note's sample and `1.0` from that sample onward. A gate
/// applied one frame off changes an exact value, which is what makes the placement
/// assertion below falsifiable at all.
fn gated_constant() -> GraphIr {
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
        .build()
        .expect("a readable plan")
}

#[test]
fn a_late_note_edge_takes_effect_at_its_clamped_render_position() {
    // `SOUND-INV-016`, restated over the render position by ADR-0043. A late event's
    // sample-positioned effect occurs where its *render position* falls — the clamped
    // position — and an on-time one still occurs at its declared sample. The diagnostic
    // alone cannot catch either being misplaced:
    // `a_late_event_is_clamped_forward_and_counted` asserts the count and would pass with
    // the edge dropped entirely.
    //
    // Both edges are asserted in one render so the test distinguishes "the clamp targets
    // the clock" from "everything is applied at the head of the call". The warm-up block
    // is deliberately **not** a multiple of `Q`, so it leaves a partial output carry and
    // the clamped position lands at a nonzero offset in the next call's buffer. With a
    // `Q`-aligned warm-up the two coincide at frame 0 and a renderer that applied every
    // late edge at the callback head would pass.
    let warm_up = 200_usize;
    let block = 256_usize;
    let host = profile(block as u64, ChannelLayout::Mono);
    let plan = admit(&gated_constant(), host);
    let slot = plan
        .resolve_note(ENVELOPE)
        .expect("an envelope is a node a note can be sent to");
    let mut renderer = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();

    // The first call renders with no events, so the clock passes sample 0 and anything
    // stamped there is late by clause 16's condition.
    let mut samples = vec![0.0_f32; warm_up];
    let output =
        AudioBlockMut::new(&mut samples, warm_up, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(output, TimedEvents::EMPTY)
        .expect("the first call renders");
    assert!(
        samples.iter().all(|value| *value == 0.0),
        "an ungated constant is silent"
    );

    // ADR-0001 clause 11: the host's output position for input sample `S` is `S + Q`,
    // because the output carry is primed with `Q` frames of silence. A warm-up that is not
    // a whole number of quanta therefore renders past what it emitted, and the surplus
    // waits in the output carry.
    let quantum = QUANTUM_FRAMES as usize;
    let clamped_to = renderer.clock();
    // The warm-up serves `Q` primed frames first, so it renders the fewest whole quanta
    // that cover what is left of its request.
    let rendered = (warm_up - quantum).next_multiple_of(quantum);
    assert_eq!(
        clamped_to.as_u64(),
        rendered as u64,
        "the clamp target is the first not-yet-rendered position, which is a quantum \
         boundary and not the caller's frame count"
    );
    // Output frame `S + Q` carries input sample `S`, so the clamped position surfaces this
    // far into the next call — the surplus the warm-up rendered but did not emit.
    let carried = rendered + quantum - warm_up;
    assert!(
        carried > 0,
        "the warm-up must leave a carry, or the clamped position collapses onto frame 0"
    );

    // A note-on stamped at sample 0 — inside a quantum that has already rendered, so it
    // is late and clamps forward to the clock. A note-off stamped `GATE` frames after the
    // clock is on time and keeps its declared sample.
    const GATE: u64 = 100;
    let events = [
        TimedEvent::new(
            EventEnvelope::new(epoch, SampleTime::new(0), TimeSource::Compiled),
            EventPayload::Note {
                slot,
                edge: NoteEdge::On,
            },
        ),
        TimedEvent::new(
            EventEnvelope::new(
                epoch,
                SampleTime::new(clamped_to.as_u64() + GATE),
                TimeSource::Compiled,
            ),
            EventPayload::Note {
                slot,
                edge: NoteEdge::Off,
            },
        ),
    ];
    let mut samples = vec![0.0_f32; block];
    let output =
        AudioBlockMut::new(&mut samples, block, ChannelLayout::Mono).expect("shaped block");
    renderer
        .render(output, TimedEvents::new(&events))
        .expect("a late event is clamped, never dropped");
    assert_eq!(renderer.diagnostics().late_events(), 1);

    // The clamped note-on lands on the clock, which surfaces `carried` frames into this
    // call's output because the warm-up's surplus is emitted first. The on-time note-off
    // lands `GATE` frames after that.
    let opens = carried;
    let closes = carried + GATE as usize;
    assert!(
        samples[..opens].iter().all(|value| *value == 0.0),
        "the carried frames precede the clamped position and the gate is still shut"
    );
    assert!(
        samples[opens..closes].iter().all(|value| *value == 1.0),
        "the clamped note-on opens the gate at the clamped position, not at the head"
    );
    assert!(
        samples[closes..].iter().all(|value| *value == 0.0),
        "the on-time note-off still lands at its own declared sample"
    );
}

#[test]
fn a_quantum_over_its_event_capacity_is_rejected_before_anything_is_mutated() {
    // The specification's Phase 1–2 prevalidated-span rule: the call is rejected before
    // renderer state or output is mutated, and the renderer must not drop, clip,
    // partially render, or grow to absorb it.
    let host = profile(256, ChannelLayout::Mono);
    let mut limits = common::defaults_for(&host);
    // Six, not four: ADR-0046 clause 1 partitions the cap into six positive producer
    // shares, so no profile can represent a per-quantum cap below six. The test's point
    // is a cap the span exceeds, and six is the smallest one that now exists.
    let capacity = 6;
    limits = synth_engine_v2::profile::RenderLimits::new(
        limits.stream(),
        limits.graph(),
        limits.voices(),
        synth_engine_v2::profile::EventLimits::new(
            synth_engine_v2::quantities::EventCount::limit(capacity).expect("valid"),
            limits.events().max_note_expansion_per_tick(),
            limits.events().max_scheduled_events_in_flight(),
            limits.events().forward_event_horizon(),
            synth_engine_v2::profile::QueueCapacities::new(
                synth_engine_v2::quantities::EventCount::limit(1).expect("positive"),
                limits.events().queues().command_queue_capacity(),
                limits.events().queues().event_egress_capacity(),
            )
            .expect("the overridden capacities are above zero"),
            synth_engine_v2::profile::ProducerShares::new(
                synth_engine_v2::quantities::EventCount::limit(1).expect("positive"),
                synth_engine_v2::quantities::EventCount::limit(1).expect("positive"),
                synth_engine_v2::quantities::EventCount::limit(1).expect("positive"),
                synth_engine_v2::quantities::EventCount::limit(1).expect("positive"),
                synth_engine_v2::quantities::EventCount::limit(1).expect("positive"),
                synth_engine_v2::quantities::EventCount::limit(1).expect("positive"),
                synth_engine_v2::quantities::EventCount::limit(1).expect("positive"),
            )
            .expect("a valid minimal partition"),
        )
        .expect("valid event limits"),
        limits.observation(),
        limits.mixing(),
        limits.memory(),
        limits.script(),
        limits.recording(),
        limits.cost(),
    )
    .expect("only the event capacity changed");
    let host = HostProfile::new(host.capabilities(), limits).expect("valid profile");

    let ir = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(440.0).expect("finite"),
        amplitude: Amplitude::new(0.5).expect("finite"),
    });
    let plan = compile(&ir, &RenderConfig::new(host))
        .into_plan()
        .expect("the plan fits");
    let mut renderer = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();
    let slot = frequency_slot(&renderer);

    // One more event in one quantum than the capacity admits.
    let events: Vec<TimedEvent> = (0..=capacity)
        .map(|index| set_frequency(slot, epoch, u64::from(index), 300.0, TimeSource::Compiled))
        .collect();

    let mut samples = vec![9.0_f32; 128];
    let output = AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("shaped block");
    let error = renderer
        .render(output, TimedEvents::new(&events))
        .expect_err("an over-full quantum is refused");

    assert!(matches!(
        error,
        RenderError::QuantumEventOverflow {
            requested: 7,
            available: 6,
            ..
        }
    ));
    assert_eq!(
        renderer.clock(),
        SampleTime::ZERO,
        "a rejected call must not have advanced the clock"
    );
    assert!(
        samples.iter().all(|sample| *sample == 9.0),
        "a rejected call must not have written the output"
    );
    assert_eq!(
        renderer.diagnostics().late_events(),
        0,
        "and must not have moved a counter"
    );
}

#[test]
fn a_parameter_change_takes_effect_at_the_next_quantum_boundary() {
    // ADR-0001 clauses 13 and 14: control evaluation is causal, so an event inside a
    // quantum may not influence the value used from that quantum's offset 0. The
    // control-rate response begins at the next boundary.
    let host = profile(1_024, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(100.0).expect("finite"),
        amplitude: Amplitude::new(0.0).expect("finite"),
    });
    let quantum = QUANTUM_FRAMES as usize;

    let mut renderer = PreparedRenderer::prepare(
        admit(&ir, host),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();

    // Amplitude 0 until an event mid-quantum-1 raises it. Quantum 1 must still be
    // silent; quantum 2 must not be.
    let raise = TimedEvent::new(
        EventEnvelope::new(
            epoch,
            SampleTime::new(quantum as u64 + 5),
            TimeSource::Compiled,
        ),
        EventPayload::SetParameter {
            slot: renderer
                .plan()
                .resolve_parameter(SOURCE, parameters::SINE_AMPLITUDE)
                .expect("the sine declares an amplitude parameter"),
            value: ParameterValue::new(0.9).expect("finite"),
        },
    );

    let frames = quantum * 4;
    let mut samples = vec![0.0_f32; frames];
    let output = AudioBlockMut::new(&mut samples, frames, ChannelLayout::Mono).expect("shaped");
    renderer
        .render(output, TimedEvents::new(&[raise]))
        .expect("the span covers the quanta this call renders");

    // Output frame k is rendered frame k - Q because of the priming head.
    let rendered_quantum = |index: usize| -> &[f32] {
        let start = (index + 1) * quantum;
        &samples[start..start + quantum]
    };
    assert!(
        rendered_quantum(0).iter().all(|s| *s == 0.0),
        "quantum 0 precedes the event"
    );
    assert!(
        rendered_quantum(1).iter().all(|s| *s == 0.0),
        "the event falls inside quantum 1, so quantum 1 must not hear it — that would be \
         lookahead"
    );
    assert!(
        rendered_quantum(2).iter().any(|s| s.abs() > 0.1),
        "quantum 2 begins at the next boundary and must hear it"
    );
}

#[test]
fn an_event_outside_the_quanta_a_call_renders_is_refused() {
    // Phase 1's span is prevalidated and bounded. The renderer owns no future-event
    // store, and dropping an event silently is what ADR-0001 clause 16 forbids — so the
    // contract is enforced rather than bent. Phase 3 presents only sealed, admitted
    // batches for the imminent call.
    let (mut renderer, epoch) = sine_renderer(256);
    let far = renderer.plan().forward_event_horizon().as_u64() - 1;
    let events = [set_frequency(
        frequency_slot(&renderer),
        epoch,
        far,
        100.0,
        TimeSource::Compiled,
    )];

    let mut samples = vec![0.0_f32; 128];
    let output = AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("shaped block");
    assert!(matches!(
        renderer.render(output, TimedEvents::new(&events)),
        Err(RenderError::EventOutsideCallSpan { .. })
    ));
}

#[test]
fn an_event_inside_a_calls_final_quantum_takes_effect_in_the_next_call() {
    // The defect this covers: the loop applied events at each quantum boundary and
    // returned with the rest unapplied, and the next call cleared the scratch — so a
    // control change that fell inside a call's *last* quantum was lost, and automation
    // became a function of how the host partitioned its callbacks. That is exactly what
    // ADR-0001 exists to prevent, and it is invisible to a test that renders in one call.
    let host = profile(1_024, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(100.0).expect("finite"),
        amplitude: Amplitude::new(0.0).expect("finite"),
    });
    let quantum = QUANTUM_FRAMES as usize;

    let mut renderer = PreparedRenderer::prepare(
        admit(&ir, host),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let epoch = renderer.epoch();

    // A call of two quanta's worth. The primed carry already holds `Q` frames, so this
    // renders exactly quantum 0 — and the event sits inside quantum 0, at offset 5.
    let raise = TimedEvent::new(
        EventEnvelope::new(epoch, SampleTime::new(5), TimeSource::Compiled),
        EventPayload::SetParameter {
            slot: renderer
                .plan()
                .resolve_parameter(SOURCE, parameters::SINE_AMPLITUDE)
                .expect("the sine declares an amplitude parameter"),
            value: ParameterValue::new(0.9).expect("finite"),
        },
    );
    let mut samples = vec![0.0_f32; quantum * 2];
    let output =
        AudioBlockMut::new(&mut samples, quantum * 2, ChannelLayout::Mono).expect("shaped");
    assert_eq!(
        renderer.quanta_needed_for(quantum * 2),
        1,
        "the primed carry means this call renders one quantum, not two"
    );
    renderer
        .render(output, TimedEvents::new(&[raise]))
        .expect("the event is inside the quantum this call renders");

    // Two further calls, no events at all. The change must be audible: it belongs to the
    // quantum after the one the event fell in, which is this call's.
    let mut heard = Vec::new();
    for _ in 0..2 {
        let mut block = vec![0.0_f32; quantum];
        let output = AudioBlockMut::new(&mut block, quantum, ChannelLayout::Mono).expect("shaped");
        renderer
            .render(output, TimedEvents::EMPTY)
            .expect("a call with no events renders");
        heard.extend_from_slice(&block);
    }
    assert!(
        heard.iter().any(|sample| sample.abs() > 0.1),
        "the amplitude change was lost between calls"
    );
}

#[test]
fn a_span_larger_than_any_call_can_admit_is_refused_before_it_is_scanned() {
    // Bounded work on the audio thread. Every filter in event resolution may skip an
    // event — a stale epoch, a distant timestamp — so without this check the loop's work
    // would be a function of what the producer sent rather than of a declared capacity: a
    // million stale events would each be examined.
    let (mut renderer, _) = sine_renderer(256);
    let slot = frequency_slot(&renderer);
    let stale = StreamEpoch::from_raw(u32::MAX);
    let huge: Vec<TimedEvent> = (0..100_000)
        .map(|index| set_frequency(slot, stale, index, 100.0, TimeSource::Compiled))
        .collect();

    let mut samples = vec![0.0_f32; 128];
    let output = AudioBlockMut::new(&mut samples, 128, ChannelLayout::Mono).expect("shaped block");
    let error = renderer
        .render(output, TimedEvents::new(&huge))
        .expect_err("a span this large cannot be admitted by one call");
    assert!(matches!(
        error,
        RenderError::EventSpanTooLarge {
            presented: 100_000,
            ..
        }
    ));
    assert_eq!(
        renderer.diagnostics().stale_epoch_events(),
        0,
        "the span was refused before a single event was classified"
    );
}

#[test]
fn an_offline_event_is_stamped_with_the_epoch_preparation_issued() {
    // The defect this covers: `render_offline` used to take stamped events while issuing
    // the epoch itself, so a caller had no way to produce a matching stamp and every
    // offline event was discarded as stale — silently, since a discarded event is counted
    // rather than refused.
    let host = profile(256, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Sine {
        frequency: Frequency::new(1_000.0).expect("finite"),
        amplitude: Amplitude::new(0.0).expect("finite"),
    });
    let quantum = QUANTUM_FRAMES as u64;

    // The address is resolved against the plan before the render, which is where a
    // caller can still be told that a parameter does not exist.
    let plan = admit(&ir, host);
    let raise = OfflineEvent::new(
        SampleTime::ZERO,
        EventPayload::SetParameter {
            slot: plan
                .resolve_parameter(SOURCE, parameters::SINE_AMPLITUDE)
                .expect("the sine declares an amplitude parameter"),
            value: ParameterValue::new(0.8).expect("finite"),
        },
    );
    let rendered = render_offline(plan, FrameCount::new(512), PlanPosition::ZERO, &[raise])
        .expect("the offline path renders");

    // The event is at sample 0, so its control response begins at the next boundary.
    let after = usize::try_from(quantum).expect("Q fits usize");
    assert!(
        rendered[after..].iter().any(|sample| sample.abs() > 0.1),
        "an offline event must actually reach the renderer"
    );
}

#[test]
fn a_negative_frequency_stays_periodic_and_is_the_positive_render_inverted() {
    // A negative frequency is legal and means the phase runs backwards. The defect this
    // covers: the phase was wrapped only at 1.0, so a backwards phase fell below zero and
    // grew without bound, feeding `sin` ever-larger arguments and losing precision to
    // range reduction rather than staying periodic.
    //
    // `sin(-x) == -sin(x)`, so the assertion is exact rather than approximate: the
    // backwards render is the forwards one negated, sample for sample.
    let host = profile(512, ChannelLayout::Mono);
    let forwards = render_live(
        admit(
            &source_plan(IrNodeKind::Sine {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            }),
            host,
        ),
        4_096,
        512,
    );
    let backwards = render_live(
        admit(
            &source_plan(IrNodeKind::Sine {
                frequency: Frequency::new(-440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            }),
            host,
        ),
        4_096,
        512,
    );

    for (index, (ahead, behind)) in forwards.iter().zip(backwards.iter()).enumerate() {
        assert!(
            (ahead + behind).abs() < 1e-6,
            "at sample {index} the backwards render is {behind}, not the negation of {ahead}"
        );
    }
    let peak = backwards.iter().fold(0.0_f32, |peak, s| peak.max(s.abs()));
    assert!(
        peak > 0.4 && peak <= 0.5 + f32::EPSILON,
        "a backwards sine must stay inside its amplitude; peak was {peak}"
    );
}

#[test]
fn the_carry_latency_is_a_constant_quantum() {
    let host = profile(512, ChannelLayout::Mono);
    let plan = admit(&source_plan(IrNodeKind::Silence), host);
    assert_eq!(plan.added_latency().as_u64(), u64::from(QUANTUM_FRAMES));

    let renderer = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    assert_eq!(
        renderer.added_latency().as_u64(),
        u64::from(QUANTUM_FRAMES),
        "charged unconditionally, including to a host that would not need it"
    );
}

#[test]
fn two_streams_get_different_epochs() {
    let host = profile(256, ChannelLayout::Mono);
    let ir = source_plan(IrNodeKind::Silence);
    let first = PreparedRenderer::prepare(
        admit(&ir, host),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    let second = PreparedRenderer::prepare(
        admit(&ir, host),
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("preparation succeeds");
    assert!(
        first.epoch() < second.epoch(),
        "epoch identifiers strictly increase, so `A -> B -> A` cannot happen"
    );
}

#[test]
fn the_prepared_rate_is_the_one_the_profile_carries() {
    let host = profile(256, ChannelLayout::Mono);
    let plan = admit(&GraphIr::empty(), host);
    assert_eq!(plan.sample_rate().as_f32(), rate(48_000.0).as_f32());
}
