//! EVD-0017 harness: what one publication pass costs.
//!
//! ADR-0046 clause 2 makes one arbiter the sole normal writer of renderer input, which
//! serialises work a per-producer design would have spread, and the record names the cost
//! as owed: "Phase 3 must measure its bounded cost."
//!
//! Build and run exactly as the evidence record states:
//!
//! ```text
//! cargo build --release --example evd_0017_publication_cost -p synth_engine_v2
//! taskset -c 10,11 target/release/examples/evd_0017_publication_cost <rounds> <iterations>
//! ```
//!
//! # The two arms and why the empty one is the control
//!
//! - `empty` — `open` then `seal`, charging nothing. `open` clears the **whole** prepared
//!   ledger, so this arm scales with the store rather than with the work, and it is the
//!   fixed cost every pass pays.
//! - `full` — the same pair with every quantum of the callback filled to the compiled
//!   share, which is the admitted maximum for this producer.
//!
//! The per-event cost is `(full - empty) / events`. Taking `full` alone would charge the
//! ledger clear to the events and report a per-event figure that shrinks as the batch
//! grows, which is an artefact rather than a property.
//!
//! # Estimator
//!
//! Minimum over rounds. Every source of noise on this host adds time, so the minimum is the
//! closest estimate of the true cost and the mean would report the background load. Arms are
//! interleaved within each round so drift affects both equally rather than whichever ran
//! second.

use std::time::{Duration, Instant};

use synth_engine_v2::compile::{RenderConfig, compile};
use synth_engine_v2::ir::{
    ExecutionScope, GraphIr, IrNodeKind, NodeId, PortId, SignalDomain, parameters,
};
use synth_engine_v2::plan::ParameterSlot;
use synth_engine_v2::profile::HostProfile;
use synth_engine_v2::publish::{ProducerClass, PublicationArbiter, WindowRow};
use synth_engine_v2::quantities::{
    Amplitude, ChannelLayout, Frequency, ParameterValue, SampleRate,
};
use synth_engine_v2::render::{EventEnvelope, EventPayload, TimedEvent};
use synth_engine_v2::time::{FrameCount, QUANTUM_FRAMES, SampleTime, StreamEpoch, TimeSource};

const RATE: f32 = 48_000.0;

fn profile(block: u64) -> HostProfile {
    HostProfile::harness(
        SampleRate::new(RATE).expect("a valid rate"),
        FrameCount::new(block),
        ChannelLayout::Mono,
    )
    .expect("the harness profile is valid")
}

/// A real parameter slot, since the fill identities are crate-private and an example is an
/// ordinary external consumer of this crate — which is the right constraint: a harness that
/// could reach inside would not be measuring the API anyone else uses.
fn parameter_slot(host: HostProfile) -> ParameterSlot {
    const SOURCE: NodeId = NodeId::new(1);
    const OUTPUT: NodeId = NodeId::new(2);
    let ir = GraphIr::builder()
        .node(
            SOURCE,
            IrNodeKind::Sine {
                frequency: Frequency::new(440.0).expect("finite"),
                amplitude: Amplitude::new(0.5).expect("finite"),
            },
            ExecutionScope::Voice,
        )
        .node(OUTPUT, IrNodeKind::Output, ExecutionScope::Global)
        .connect(
            (SOURCE, PortId::FIRST),
            (OUTPUT, PortId::FIRST),
            SignalDomain::Audio,
        )
        .build()
        .expect("a sine into an output is a readable plan");
    compile(&ir, &RenderConfig::new(host))
        .into_plan()
        .expect("the plan fits")
        .resolve_parameter(SOURCE, parameters::SINE_FREQUENCY)
        .expect("the sine declares a frequency parameter")
}

/// An event landing in `quantum`, one frame past its boundary.
fn event(slot: ParameterSlot, quantum: u64) -> TimedEvent {
    TimedEvent::new(
        EventEnvelope::new(
            StreamEpoch::from_raw(1),
            SampleTime::new(quantum * u64::from(QUANTUM_FRAMES) + 1),
            TimeSource::Compiled,
        ),
        EventPayload::SetParameter {
            slot,
            value: ParameterValue::ZERO,
        },
    )
}

fn empty_pass(arbiter: &mut PublicationArbiter, quanta: usize, iterations: u32) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        let publication = arbiter
            .open(SampleTime::ZERO, quanta)
            .expect("the window fits the prepared store");
        let batch = publication.seal();
        // A **ledger row**, not the length. `len` does not depend on the clear, so black-
        // boxing it alone would leave the clear elidable in principle. This is belt and
        // braces rather than a repair: the empty arm's figure did not move when the read was
        // added, and about nine nanoseconds for the 1.8 kB this clears is ordinary L1 store
        // bandwidth. An earlier note here claimed the figure was below memset speed and had
        // exposed an elision; that used DRAM bandwidth for an L1-resident buffer and was
        // wrong.
        let _ = std::hint::black_box(batch.spent(WindowRow::FIRST, ProducerClass::Compiled));
    }
    start.elapsed()
}

fn full_pass(
    arbiter: &mut PublicationArbiter,
    slot: ParameterSlot,
    quanta: usize,
    per_quantum: u32,
    iterations: u32,
) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        let mut publication = arbiter
            .open(SampleTime::ZERO, quanta)
            .expect("the window fits the prepared store");
        for quantum in 0..quanta as u64 {
            for _ in 0..per_quantum {
                publication
                    .charge(ProducerClass::Compiled, event(slot, quantum))
                    .expect("inside the compiled share");
            }
        }
        let batch = publication.seal();
        // A **ledger row**, not the length. `len` does not depend on the clear, so black-
        // boxing it alone would leave the clear elidable in principle. This is belt and
        // braces rather than a repair: the empty arm's figure did not move when the read was
        // added, and about nine nanoseconds for the 1.8 kB this clears is ordinary L1 store
        // bandwidth. An earlier note here claimed the figure was below memset speed and had
        // exposed an elision; that used DRAM bandwidth for an L1-resident buffer and was
        // wrong.
        let _ = std::hint::black_box(batch.spent(WindowRow::FIRST, ProducerClass::Compiled));
    }
    start.elapsed()
}

/// The estimator the evidence record names: every source of noise on this host adds time,
/// so the minimum is the closest estimate of the true cost and the mean reports the load.
fn minimum(samples: &[Duration]) -> Duration {
    samples.iter().copied().min().unwrap_or(Duration::ZERO)
}

/// Spread across rounds, so a reader can see whether the minimum is stable rather than one
/// lucky sample. Reported beside the minimum because a tight spread and a wide one support
/// very different confidence in the same figure.
fn interquartile_range(samples: &[Duration]) -> f64 {
    let mut seconds: Vec<f64> = samples.iter().map(Duration::as_secs_f64).collect();
    seconds.sort_by(f64::total_cmp);
    if seconds.len() < 4 {
        return 0.0;
    }
    let quarter = seconds.len() / 4;
    seconds[seconds.len() - 1 - quarter] - seconds[quarter]
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let rounds: u32 = arguments
        .get(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(15);
    let iterations: u32 = arguments
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(2_000);

    println!("EVD-0017 publication cost: {rounds} rounds x {iterations} iterations");
    println!(
        "{:>6}  {:>6}  {:>7}  {:>12}  {:>12}  {:>12}  {:>12}  {:>9}",
        "block", "quanta", "events", "empty/pass", "full/pass", "full IQR", "per event", "% budget"
    );

    for block in [64_u64, 256, 1_024, 4_096] {
        let host = profile(block);
        let quanta = host
            .capabilities()
            .max_quanta_per_callback()
            .expect("fits")
            .as_usize()
            .expect("fits");
        let per_quantum = host.limits().events().shares().compiled_event_share().get();
        let events = quanta as u32 * per_quantum;
        let slot = parameter_slot(host);
        let mut arbiter = PublicationArbiter::prepare(&host).expect("preparable");

        let mut empties: Vec<Duration> = Vec::with_capacity(rounds as usize);
        let mut fulls: Vec<Duration> = Vec::with_capacity(rounds as usize);
        for _ in 0..rounds {
            // Interleaved within the round, so drift lands on both arms.
            empties.push(empty_pass(&mut arbiter, quanta, iterations));
            fulls.push(full_pass(
                &mut arbiter,
                slot,
                quanta,
                per_quantum,
                iterations,
            ));
        }

        let empty_per = minimum(&empties).as_secs_f64() / f64::from(iterations);
        let full_per = minimum(&fulls).as_secs_f64() / f64::from(iterations);
        let full_iqr = interquartile_range(&fulls) / f64::from(iterations);
        let per_event = (full_per - empty_per) / f64::from(events);
        let budget = block as f64 / f64::from(RATE);
        let share_of_budget = full_per / budget * 100.0;

        println!(
            "{block:>6}  {quanta:>6}  {events:>7}  {:>10.3} us  {:>10.3} us  {:>10.3} us  \
             {:>10.2} ns  {share_of_budget:>8.3}%",
            empty_per * 1e6,
            full_per * 1e6,
            full_iqr * 1e6,
            per_event * 1e9
        );
    }
}
