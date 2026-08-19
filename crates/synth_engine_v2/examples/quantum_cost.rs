//! What the render quantum costs on the real V2 path — EVD-0012's instrument.
//!
//! ADR-0037 accepted `Q` = 64 **provisionally**, on a V1 proxy whose direction of error
//! it could not establish, and made a Phase 2 re-measurement against real V2 nodes
//! binding. This is that measurement's arm. One build renders one `Q`; the record's
//! ratios come from five builds that differ in that constant and in nothing else.
//!
//! ```text
//! taskset -c 10,11 <binary> <rounds> <iterations>
//! ```
//!
//! # What this file must get right, and why
//!
//! **Every arm has to render the same audio.** Otherwise a ratio between two arms is a
//! ratio between two programs. The fixtures present at most one note, at plan sample 0,
//! and no control-rate event at all, so the rendered signal does not depend on `Q` — and
//! the digest each arm prints is what checks that rather than assuming it.
//!
//! **Every arm has to enter the timed loop at the same plan sample.** The clock stands at
//! `Q` after the note call, and a 512-frame call advances it by 512, so the reachable set
//! would be `Q + 512k` — which contains 49 152 at no candidate quantum. The settle
//! therefore uses `Q`-frame calls, and 49 152 is a whole multiple of every candidate.
//!
//! **The clock is read twice per round, not twice per call.** At a few hundred nanoseconds
//! per quantum a per-call `Instant::now()` is a measurable fraction of the figure.

use std::hint::black_box;
use std::time::Instant;

use sha2::{Digest, Sha256};
use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::ir::{ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain};
use synth_engine_v2::offline::{OfflineEvent, render_offline};
use synth_engine_v2::plan::CompiledPlan;
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, CutoffFrequency, Frequency, GainFactor, NormalizedLevel, Resonance,
    SampleRate, Seconds,
};
use synth_engine_v2::render::{
    AudioBlockMut, EventEnvelope, EventPayload, NoteEdge, PreparedRenderer, Renderer, TimedEvent,
    TimedEvents,
};
use synth_engine_v2::time::{
    FrameCount, PlanPosition, QUANTUM_FRAMES, SampleTime, StreamAnchor, TimeSource,
};

/// This build's quantum, in frames.
const Q: usize = QUANTUM_FRAMES as usize;

/// The caller block every timed call uses, identical in every arm.
const BLOCK: usize = 512;

/// The plan sample every arm enters the timed loop at, and the length every digest is over.
///
/// A whole multiple of 32, 64, 128 and 256 alike, which is what makes it reachable by
/// `Q`-frame calls in every arm.
const SETTLE: u64 = 49_152;

/// How many gains the dispatch-heavy shape chains.
const GAINS: u32 = 32;

const ENVELOPE: NodeId = NodeId::new(1);
const SINE: NodeId = NodeId::new(2);
const FILTER: NodeId = NodeId::new(3);
const AMPLIFIER: NodeId = NodeId::new(4);
const OUTPUT: NodeId = NodeId::new(9);
/// The first gain of the chain; the rest follow consecutively.
const GAIN_BASE: u32 = 100;

/// Which fixture an arm is rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    VoiceMono,
    VoiceStereo,
    GainChain,
}

impl Shape {
    const ALL: [Self; 3] = [Self::VoiceMono, Self::VoiceStereo, Self::GainChain];

    const fn name(self) -> &'static str {
        match self {
            Self::VoiceMono => "voice-mono",
            Self::VoiceStereo => "voice-stereo",
            Self::GainChain => "gain-chain",
        }
    }

    const fn layout(self) -> ChannelLayout {
        match self {
            Self::VoiceStereo => ChannelLayout::Stereo,
            Self::VoiceMono | Self::GainChain => ChannelLayout::Mono,
        }
    }

    /// Whether the fixture has an envelope, and therefore a note to present.
    const fn gated(self) -> bool {
        !matches!(self, Self::GainChain)
    }

    fn graph(self) -> GraphIr {
        match self {
            Self::VoiceMono | Self::VoiceStereo => voice(),
            Self::GainChain => gain_chain(),
        }
    }
}

/// The minimal voice path: envelope, sine, filter, amplifier, output.
///
/// ADR-0041 clause 16's first baseline fixture, and the path Phase 2 exists to render.
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
        .expect("the voice path is a readable plan")
}

/// A sine into a chain of gains: the dispatch-heavy shape.
///
/// No superlative is claimed for it — EVD-0012 says why. It is here to show whether the
/// outcome moves when the dispatch count rises against the per-sample arithmetic, and
/// 33 dispatches of a one-multiply kernel do that.
fn gain_chain() -> GraphIr {
    let mut builder = GraphIr::builder()
        .node(
            SINE,
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global);

    let mut upstream = SINE;
    for index in 0..GAINS {
        let gain = NodeId::new(GAIN_BASE + index);
        builder = builder
            .node(
                gain,
                IrNodeKind::Gain {
                    // Unity would be a factor the arithmetic could not distinguish from
                    // no gain at all; this one is a multiply whose result is used.
                    factor: GainFactor::new(0.999).expect("finite"),
                },
                ExecutionScope::Voice,
            )
            .connect(
                (upstream, PortId::FIRST),
                (gain, PortId::FIRST),
                SignalDomain::Audio,
            );
        upstream = gain;
    }

    builder
        .connect(
            (upstream, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a sine into a chain of gains is a readable plan")
}

fn plan(shape: Shape) -> CompiledPlan {
    let profile = HostProfile::harness(
        SampleRate::new(48_000.0).expect("a valid rate"),
        FrameCount::new(BLOCK as u64),
        shape.layout(),
    )
    .expect("the harness profile is valid");
    compile(&shape.graph(), &RenderConfig::new(profile))
        .into_plan()
        .expect("the fixture is admissible")
}

/// Clause 5's input carry, as a cost rather than as state.
///
/// `PreparedRenderer::input_carry` is private and a harness cannot reach it, so this is
/// its own buffer of exactly clause 5's size. It appends `N` frames per call and consumes
/// `Q` per quantum the call renders, compacting the remainder to the front once per call —
/// which is the procedure the record measures the renderer as not yet paying for.
struct InputCarry {
    buffer: Vec<f32>,
    source: Vec<f32>,
    channels: usize,
    /// Frames currently held.
    held: usize,
}

impl InputCarry {
    fn new(channels: usize) -> Self {
        let frames = BLOCK + Q;
        Self {
            buffer: vec![0.0; frames * channels],
            source: vec![0.5; BLOCK * channels],
            channels,
            held: 0,
        }
    }

    /// One call's worth of clause 5's bookkeeping, for `frames` served and `quanta`
    /// rendered.
    fn call(&mut self, frames: usize, quanta: usize) {
        let channels = self.channels;
        let start = self.held * channels;
        let end = start + frames * channels;
        if let (Some(into), Some(from)) = (
            self.buffer.get_mut(start..end),
            self.source.get(..frames * channels),
        ) {
            into.copy_from_slice(from);
        }
        self.held += frames;

        let consumed = quanta * Q;
        if consumed <= self.held {
            self.held -= consumed;
            let live = (consumed + self.held) * channels;
            self.buffer.copy_within(consumed * channels..live, 0);
        }
        // The release build is free to delete copies into a buffer nothing reads.
        black_box(&self.buffer);
    }

    /// One sample, folded into a printed value so the copies above cannot be elided.
    fn witness(&self) -> f32 {
        self.buffer.first().copied().unwrap_or(0.0)
    }
}

/// A renderer settled to plan sample [`SETTLE`], with its note played.
///
/// Every arm leaves this function at the same plan sample, with an empty output carry, and
/// with the envelope long past its 10 ms attack and 100 ms decay.
fn settled(shape: Shape, carry: Option<&mut InputCarry>) -> (PreparedRenderer, Vec<f32>) {
    let plan = plan(shape);
    let layout = shape.layout();
    let channels = layout.channels();
    let note = shape.gated().then(|| {
        plan.resolve_note(ENVELOPE)
            .expect("the envelope is playable")
    });

    let mut renderer = PreparedRenderer::prepare(
        plan,
        StreamAnchor::new(SampleTime::ZERO, PlanPosition::ZERO),
    )
    .expect("an admitted plan prepares");
    let mut block = vec![0.0_f32; BLOCK * channels];
    let mut carry = carry;

    let mut call = |renderer: &mut PreparedRenderer,
                    block: &mut Vec<f32>,
                    frames: usize,
                    events: &[TimedEvent]| {
        let quanta = renderer.quanta_needed_for(frames);
        let region = block
            .get_mut(..frames * channels)
            .expect("the block holds a call");
        let output = AudioBlockMut::new(region, frames, layout).expect("a well-shaped block");
        renderer
            .render(output, TimedEvents::new(events))
            .expect("the fixture renders");
        if let Some(carry) = carry.as_deref_mut() {
            carry.call(frames, quanta);
        }
    };

    // The output carry is primed with `Q` frames of silence, so this call serves the
    // priming and renders no quantum — which is why no event may be presented with it.
    call(&mut renderer, &mut block, Q, &[]);

    // The first call that renders. The note lands at plan sample 0 at every `Q`.
    let played: Vec<TimedEvent> = note
        .map(|slot| {
            vec![TimedEvent::new(
                EventEnvelope::new(renderer.epoch(), SampleTime::ZERO, TimeSource::Compiled),
                EventPayload::Note {
                    slot,
                    edge: NoteEdge::On,
                },
            )]
        })
        .unwrap_or_default();
    call(&mut renderer, &mut block, Q, &played);

    // `Q`-frame calls, not `BLOCK`-frame ones: the clock stands at `Q` and 512-frame calls
    // would only ever reach `Q + 512k`, which is never 49 152.
    while renderer.clock().as_u64() < SETTLE {
        call(&mut renderer, &mut block, Q, &[]);
    }

    (renderer, block)
}

/// One arm: `iterations` calls of [`BLOCK`] frames as one timed batch, in seconds per
/// rendered frame.
fn arm(shape: Shape, iterations: u32, clause_five: bool) -> (f64, f32) {
    let channels = shape.layout().channels();
    let mut carry = clause_five.then(|| InputCarry::new(channels));
    let (mut renderer, mut block) = settled(shape, carry.as_mut());
    let layout = shape.layout();

    let start = Instant::now();
    for _ in 0..iterations {
        let quanta = renderer.quanta_needed_for(BLOCK);
        let region = block
            .get_mut(..BLOCK * channels)
            .expect("the block holds a call");
        let output = AudioBlockMut::new(region, BLOCK, layout).expect("a well-shaped block");
        renderer
            .render(output, TimedEvents::EMPTY)
            .expect("renders a block");
        if let Some(carry) = carry.as_mut() {
            carry.call(BLOCK, quanta);
        }
        black_box(&block);
    }
    let elapsed = start.elapsed().as_secs_f64();
    let frames = f64::from(iterations) * BLOCK as f64;
    (
        elapsed / frames,
        carry.as_ref().map_or(0.0, InputCarry::witness),
    )
}

/// SHA-256 over `SETTLE` frames rendered offline from plan sample 0.
///
/// The gate every timing figure is read behind. `render_offline` is latency-compensated,
/// so its first output sample is plan sample 0 at every `Q` — which is what makes the
/// five arms' digests comparable at all.
fn digest(shape: Shape) -> String {
    let plan = plan(shape);
    let events: Vec<OfflineEvent> = if shape.gated() {
        let slot = plan
            .resolve_note(ENVELOPE)
            .expect("the envelope is playable");
        vec![OfflineEvent::new(
            SampleTime::ZERO,
            EventPayload::Note {
                slot,
                edge: NoteEdge::On,
            },
        )]
    } else {
        // `gain-chain` has no envelope, so it renders with no events at all — which is
        // what makes its output independent of `Q` by a shorter argument than the voices'.
        Vec::new()
    };

    let rendered = render_offline(plan, FrameCount::new(SETTLE), PlanPosition::ZERO, &events)
        .expect("the fixture renders offline");

    let mut hasher = Sha256::new();
    for sample in &rendered {
        hasher.update(sample.to_bits().to_le_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
    let rounds = count("rounds", 25);
    let iterations = count("iterations", 2_000);

    println!("quantum,{Q}");
    println!("block,{BLOCK}");
    println!("rounds,{rounds}");
    println!("iterations,{iterations}");
    for shape in Shape::ALL {
        println!("digest,{},{}", shape.name(), digest(shape));
    }

    // Each shape and variant twice per round, the control first, and compared within the
    // round both were measured in. The minimum over rounds is this arm's figure: a slower
    // round was slower for a reason outside the code under comparison.
    for shape in Shape::ALL {
        for clause_five in [false, true] {
            let variant = if clause_five { "clause-5" } else { "as-built" };
            let mut measured = f64::MAX;
            let mut control = f64::MAX;
            let mut spreads = Vec::with_capacity(rounds as usize);
            let mut witness = 0.0_f32;
            for _ in 0..rounds {
                let (control_round, _) = arm(shape, iterations, clause_five);
                let (measured_round, seen) = arm(shape, iterations, clause_five);
                witness += seen;
                spreads.push(
                    (measured_round - control_round).abs() / measured_round.min(control_round)
                        * 100.0,
                );
                control = control.min(control_round);
                measured = measured.min(measured_round);
            }
            spreads.sort_by(f64::total_cmp);
            let spread = spreads.get(spreads.len() / 2).copied().unwrap_or(f64::NAN);
            // Milliseconds of elapsed render time per second of rendered audio, which is
            // seconds per frame times 48 000 frames times 1 000 milliseconds.
            let cost = |seconds_per_frame: f64| seconds_per_frame * 48_000.0 * 1_000.0;
            println!(
                "cost,{},{variant},{:.6},{:.6},{:.3}",
                shape.name(),
                cost(measured),
                cost(control),
                spread
            );
            // Printed so the clause-5 copies cannot be optimized away as unobserved.
            println!("witness,{},{variant},{witness:.3}", shape.name());
        }
    }
}
