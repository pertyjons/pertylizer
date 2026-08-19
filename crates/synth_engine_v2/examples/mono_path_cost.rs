//! What the mono path costs, before and after the arena became interleaved.
//!
//! ADR-0041 clause 3 says what this exists to answer. The record can promise that a mono
//! signal's **storage** is unchanged — `Q` contiguous samples, bit for bit the arrangement
//! the planar crate had — and it deliberately refuses to promise that a mono path *costs*
//! the same afterwards, because EVD-0010 measured the storage question with the kernels of
//! the time and the conversion changes the ABI around them. That gap is a named risk with
//! a re-measurement as its control, and this is the re-measurement.
//!
//! It is deliberately not part of EVD-0010's harness: that one compares two layouts by
//! hand within one build, and this compares one build against another. What it needs from
//! the crate is only the public path a host takes — compile, prepare, render — so the same
//! file runs against the planar commit and this one.
//!
//! ```text
//! taskset -c 10,11 cargo run --release -p synth_engine_v2 --example mono_path_cost -- 9 2000
//! ```
//!
//! Rounds first, then iterations per round. The estimator is EVD-0010's, and matching it
//! matters more than it looks: that record times a **whole batch** of iterations and
//! divides, so the clock is read twice per round rather than twice per quantum. Timing
//! each quantum separately would measure `Instant::now()` alongside the render — tens of
//! nanoseconds against a figure of a few hundred — and selecting the fastest single sample
//! would pick the luckiest quantum rather than the arm's cost.
//!
//! So: each round times one batch per arm and divides by the iteration count; **within a
//! round the control runs first**; the **minimum** over rounds is one run's figure; and
//! the median over runs is the reported one. Every control is compared to its arm within
//! the round both were measured in.

use std::hint::black_box;
use std::time::Instant;

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, Frequency, NormalizedLevel, ParameterValue,
    Resonance, SampleRate, Seconds,
};
use synth_engine_v2::render::{
    AudioBlockMut, EventEnvelope, EventPayload, PreparedRenderer, Renderer, TimedEvent, TimedEvents,
};
use synth_engine_v2::time::{
    FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor, TimeSource,
};

const Q: usize = QUANTUM_FRAMES as usize;

const ENVELOPE: NodeId = NodeId::new(1);
const SINE: NodeId = NodeId::new(2);
const FILTER: NodeId = NodeId::new(3);
const AMPLIFIER: NodeId = NodeId::new(4);
const OUTPUT: NodeId = NodeId::new(9);

/// The minimal voice path, mono: the fixture clause 16 renders and the phase's own shape.
fn voice() -> GraphIr {
    GraphIr::builder()
        .node(
            ENVELOPE,
            IrNodeKind::Envelope {
                attack: Seconds::new(0.010).expect("finite"),
                decay: Seconds::new(0.100).expect("finite"),
                sustain: NormalizedLevel::new(0.700).expect("in range"),
                release: Seconds::new(0.200).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            SINE,
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(
            FILTER,
            IrNodeKind::Filter {
                cutoff: CutoffFrequency::new(1_000.0).expect("positive"),
                resonance: Resonance::BUTTERWORTH,
            },
            ExecutionScope::Voice,
        )
        .node(AMPLIFIER, IrNodeKind::Amplifier, ExecutionScope::Voice)
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SINE, PortId::FIRST),
            (FILTER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (FILTER, PortId::FIRST),
            (AMPLIFIER, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (AMPLIFIER, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .connect(
            (ENVELOPE, PortId::FIRST),
            (AMPLIFIER, PortId::new(1)),
            SignalDomain::Control,
        )
        .build()
        .expect("the minimal voice path is a readable plan")
}

fn plan() -> CompiledPlan {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).expect("valid rate"),
        FrameCount::new(Q as u64),
        ChannelLayout::Mono,
    )
    .expect("the harness profile is valid");
    compile(&voice(), &RenderConfig::new(profile))
        .into_plan()
        .expect("the minimal voice path is admissible")
}

/// One arm: render `iterations` quanta as one timed batch, in seconds per quantum.
///
/// Prepared once outside the loop, and the gate opened before timing, so no iteration
/// times an allocation or a silent voice. The clock is read **twice**, not twice per
/// quantum: at this scale a per-call `Instant::now()` is a measurable fraction of what is
/// being measured, and it is the same fraction in both arms only if nothing else varies.
fn arm(iterations: u32) -> f64 {
    let plan = plan();
    let gate = plan
        .resolve_parameter(ENVELOPE, parameters::ENVELOPE_GATE)
        .expect("the envelope declares a gate");
    let mut renderer = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("an admitted plan prepares");
    let mut block = vec![0.0_f32; Q];

    // The first call returns the primed carry and renders no quantum, so it cannot carry
    // the gate: an event presented with it is refused. The gate goes on the second call,
    // which is the first that renders.
    let output = AudioBlockMut::new(&mut block, Q, ChannelLayout::Mono).expect("one quantum");
    renderer
        .render(output, TimedEvents::EMPTY)
        .expect("the primed call renders");
    let opened = [TimedEvent::new(
        EventEnvelope::new(renderer.epoch(), SampleTime::ZERO, TimeSource::Compiled),
        EventPayload::SetParameter {
            slot: gate,
            value: ParameterValue::new(1.0).expect("finite"),
        },
    )];
    let output = AudioBlockMut::new(&mut block, Q, ChannelLayout::Mono).expect("one quantum");
    renderer
        .render(output, TimedEvents::new(&opened))
        .expect("the gate opens");

    let start = Instant::now();
    for _ in 0..iterations {
        let output = AudioBlockMut::new(&mut block, Q, ChannelLayout::Mono).expect("one quantum");
        renderer
            .render(output, TimedEvents::EMPTY)
            .expect("renders a quantum");
        black_box(&block);
    }
    let elapsed = start.elapsed().as_secs_f64();
    elapsed / f64::from(iterations)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    match values.len() {
        0 => f64::NAN,
        length => values.get(length / 2).copied().unwrap_or(f64::NAN),
    }
}

fn main() {
    let mut arguments = std::env::args().skip(1);
    let mut count = |name: &str, fallback: u32| -> u32 {
        match arguments.next() {
            None => fallback,
            Some(value) => match value.parse::<u32>() {
                Ok(parsed) if parsed > 0 => parsed,
                _ => {
                    eprintln!("{name} must be a positive whole number, and is {value:?}");
                    std::process::exit(2);
                }
            },
        }
    };
    let rounds = count("rounds", 9);
    let iterations = count("iterations", 2_000);

    // The arm and its control are the *same* measurement. Within a round the control runs
    // first, and the two are compared within the round both were measured in: whatever
    // separates them there is what this machine does to two identical measurements, and a
    // margin smaller than that separation is not a result.
    let mut measured = Vec::with_capacity(rounds as usize);
    let mut control = Vec::with_capacity(rounds as usize);
    let mut spreads = Vec::with_capacity(rounds as usize);
    for _ in 0..rounds {
        let control_round = arm(iterations);
        let measured_round = arm(iterations);
        // Divided by the **smaller** of the two, as EVD-0010 does: the separation is
        // reported against the faster measurement, so a slow round cannot flatter it.
        spreads.push(
            (measured_round - control_round).abs() / measured_round.min(control_round) * 100.0,
        );
        control.push(control_round);
        measured.push(measured_round);
    }

    // The minimum over rounds is this run's figure. A round slower than the fastest was
    // slower for a reason outside the code under comparison.
    let fastest = |values: &[f64]| values.iter().copied().fold(f64::MAX, f64::min);
    let measured = fastest(&measured);
    let control = fastest(&control);
    println!("rounds,{rounds}");
    println!("iterations,{iterations}");
    println!("mono_render_seconds_per_quantum,{measured:.12}");
    println!("mono_render_nanoseconds_per_quantum,{:.2}", measured * 1e9);
    println!("control_seconds_per_quantum,{control:.12}");
    println!("control_nanoseconds_per_quantum,{:.2}", control * 1e9);
    // The **median** of the per-round separations, not the worst: one bad round should
    // not set the threshold every later comparison is judged against, and a threshold
    // taken from the worst round is one chosen after seeing the data.
    println!("control_spread_percent,{:.2}", median(&mut spreads));
}
