//! Continuous finite-input experiment under ADR-0070, not OS timing qualification.
//!
//! Fixed profile: 8 waves, two messages and one explicit frontier per source/wave,
//! 256 device frames per service period, 512 device frames synthetic lookahead.
//! Wave 0 is generated only AFTER the first 256 frames render. Stop is 2816 and
//! output is 3072 frames. Both source clocks and every original observation stay
//! identical across schedules. A wave released at r must be queued before r+2's
//! first callback. Worker service at r or r+1 publishes after that period's cut,
//! for audio admission at r+1 or r+2. Service at r+2 misses the cut. Missing that
//! logical cut halts; no waiting is hidden in audio.
//!
//! Falsifiers: whole-trace preload; input/raw/audio disagreement; lost custody;
//! deadline miss rescued by waiting; frontier inferred from queue occupancy;
//! callback allocation/free; old generation resumed. Negative controls run before
//! the supported schedule matrix. This test host does not change capture lateness,
//! promise a physical 512-frame latency, or qualify OS scheduling/production sizing.
mod actors;
use super::*;
use crate::host::input::InputEventId;
use actors::*;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Duration;

const WAVES: usize = 8;
const PERIOD: usize = 256;
const STOP: usize = (WAVES + 3) * PERIOD;
const PERIODS: usize = WAVES + 4;
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
struct Wave(usize);
impl Wave {
    fn release(self) -> usize {
        self.0 + 1
    }
    fn base(self) -> u64 {
        u64::try_from((self.release() + 2) * PERIOD).unwrap()
    }
    fn frontier(self) -> SampleTime {
        SampleTime::new(self.base() + u64::try_from(PERIOD).unwrap())
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
struct Envelope {
    generation: ConnectionGeneration,
    wave: Wave,
    observation: InputObservation,
}
fn wave(port: usize, generation: ConnectionGeneration, id: Wave) -> [Envelope; 3] {
    let base = id.base();
    [
        observation(
            port,
            base + if port == 0 { 16 } else { 20 },
            base + if port == 0 { 40 } else { 30 },
            [0x90, 60, 100],
        ),
        observation(
            port,
            base + if port == 0 { 80 } else { 90 },
            base + if port == 0 { 100 } else { 110 },
            [0x80, 60, 0],
        ),
        InputObservation::Frontier {
            tick: tick(port, id.frontier().as_u64()),
        },
    ]
    .map(|observation| Envelope {
        generation,
        wave: id,
        observation,
    })
}

#[derive(Clone, Copy)]
struct Scenario {
    worker_delay: FrameCount,
    stall_worker_at: Option<Wave>,
    stall_frontier_at: Option<Wave>,
    ingress_cells: EventCount,
    packet_cells: EventCount,
    return_cells: EventCount,
    device_loss_at: Option<SampleTime>,
}
impl Scenario {
    const NORMAL: Self = Self {
        worker_delay: FrameCount::ZERO,
        stall_worker_at: None,
        stall_frontier_at: None,
        ingress_cells: EventCount::measured(9),
        packet_cells: EventCount::measured(32),
        return_cells: EventCount::measured(8),
        device_loss_at: None,
    };
}
#[derive(Default, Debug)]
struct HighWater {
    input: [usize; 2],
    sources: usize,
    ingress: [usize; 2],
    packets: usize,
    returns: usize,
    failed_pushes: usize,
}
struct Run {
    owner: InputCaptureSession,
    generations: [ConnectionGeneration; 2],
    samples: Vec<f32>,
    receipts: [Vec<String>; 2],
    commands: Vec<String>,
    high: HighWater,
    generated: usize,
    refused: usize,
    accepted: usize,
    halted_at: Option<usize>,
    audio_calls: usize,
}
fn receive<T>(receiver: &Receiver<T>) -> T {
    receiver.recv_timeout(TIMEOUT).unwrap()
}

fn execute(scenario: Scenario, partitions: &[usize]) -> Run {
    assert!(
        scenario
            .worker_delay
            .as_u64()
            .is_multiple_of(u64::try_from(PERIOD).unwrap())
    );
    let ingress_cells = scenario.ingress_cells.as_usize().unwrap();
    let packet_cells = scenario.packet_cells.as_usize().unwrap();
    let return_cells = scenario.return_cells.as_usize().unwrap();
    let (mut control, audio, halt, generations) = split(16, 32);
    let (mut packet_writer, packet_reader) = HeapRb::new(packet_cells).split();
    let (return_writer, return_reader) = HeapRb::new(return_cells).split();
    // Finite backing, including both ring headers/counters, producer retry cells,
    // callback failed sends and the complete envelope-to-input association table.
    let transport_bytes = packet_cells * size_of::<LoopTransferPacket>()
        + return_cells * size_of::<LoopTransferCompletion>()
        + 2 * ingress_cells * size_of::<Envelope>()
        + 2 * 12 * size_of::<Option<Envelope>>()
        + 32 * size_of::<(InputEventId, Envelope)>()
        + size_of::<Callback>()
        + size_of::<HeapRb<LoopTransferPacket>>()
        + size_of::<HeapRb<LoopTransferCompletion>>()
        + 2 * size_of::<HeapRb<Envelope>>()
        + 4 * 256;
    assert!(transport_bytes <= 65_536);
    // Commands and only the initial explicit fences precede the first callback.
    for (at, command) in [(0, SessionCommand::Play), (STOP, SessionCommand::Stop)] {
        packet_writer
            .try_push(
                control
                    .prepare_command(SampleTime::new(u64::try_from(at).unwrap()), command)
                    .unwrap(),
            )
            .unwrap();
    }
    let mut initial = 0;
    while let Some(packet) = control.next_packet().unwrap() {
        assert_eq!(control.packet_input_id(&packet).unwrap().serial(), 1);
        packet_writer.try_push(packet).unwrap();
        initial += 1;
    }
    assert_eq!(initial, 2);
    let mut input_readers = Vec::new();
    let mut producers = Vec::new();
    for (port, generation) in generations.into_iter().enumerate() {
        let (writer, reader) = HeapRb::new(ingress_cells).split();
        input_readers.push(reader);
        producers.push(ProducerActor::start(port, generation, writer, scenario));
    }
    let merger = MergerActor::start(
        control,
        input_readers,
        packet_writer,
        return_reader,
        generations,
        initial,
        scenario,
    );
    let callback = Callback {
        audio,
        input: packet_reader,
        output: return_writer,
        refused: None,
        failed: None,
        returned: 0,
    };
    let renderer = AudioActor::start(callback, partitions.to_vec());
    let mut samples = Vec::new();
    let mut halted_at = None;
    let mut high = HighWater::default();
    let mut audio_calls = 0;
    let mut published = [SampleTime::ZERO; 2];
    for period in 0..PERIODS {
        let due = period.checked_sub(3).filter(|wave| *wave < WAVES).map(Wave);
        if due.is_some_and(|id| published.iter().any(|f| *f < id.frontier())) {
            halt.request_stop();
            halted_at = Some(period);
        }
        if scenario.device_loss_at == Some(SampleTime::new(u64::try_from(period * PERIOD).unwrap()))
        {
            halt.request_device_lost();
            halted_at = Some(period);
            break; // No final callback, including no further packet admission.
        }
        renderer.send(AudioStep::Render);
        receive(&renderer.cuts);
        // Audio proceeds without waiting. Producers and the merger now run after
        // its captured cut and can overlap rendering; no data wait is inside audio.
        if halted_at.is_none() && period > 0 && period <= WAVES {
            for producer in &producers {
                producer.send(ProducerStep::Generate(Wave(period - 1)));
            }
            for (port, producer) in producers.iter().enumerate() {
                let report = receive(&producer.reports);
                high.ingress[port] = high.ingress[port].max(report.high);
                high.failed_pushes += report.failed_pushes;
            }
        }
        merger.send(MergerStep::Service(period));
        let report = receive(&merger.reports);
        published = report.frontiers;
        if report.closed {
            halted_at = Some(period);
        }
        high.input = std::array::from_fn(|p| high.input[p].max(report.input[p]));
        high.sources = high.sources.max(report.sources);
        high.packets = high.packets.max(report.packets);
        high.failed_pushes += report.failed_pushes;
        let report = receive(&renderer.reports);
        samples.extend_from_slice(&report.samples);
        high.returns = high.returns.max(report.returns);
        audio_calls += report.calls;
        if report.closed {
            halted_at = Some(period);
        }
        if halted_at.is_some() {
            break;
        }
    }
    let mut remaining = Vec::new();
    let mut producer_owners = Vec::new();
    let mut generated = 0;
    for producer in producers.drain(..) {
        let (writer, pending, count) = producer.finish();
        producer_owners.push(writer);
        remaining.extend(pending.into_iter().flatten());
        generated += count;
    }
    let mut callback = renderer.finish(); // All callback access and producers joined.
    callback.audio.synchronize_halt().unwrap();
    let mut worker = merger.finish();
    worker.recover(&mut callback, remaining);
    let mut owner = callback.audio.reunite(worker.control).unwrap();
    if halted_at.is_some() {
        assert!(owner.finalize().is_err());
        for generation in generations {
            owner.acknowledge_input_quiescence(generation).unwrap();
        }
    }
    owner.finalize().unwrap();
    assert_eq!(generated, worker.accepted + worker.refused);
    let generated_waves = generated / 6;
    for (port, generation) in generations.into_iter().enumerate() {
        for id in 0..generated_waves {
            for original in wave(port, generation, Wave(id)) {
                assert_eq!(
                    worker
                        .resolved
                        .iter()
                        .filter(|value| **value == original)
                        .count(),
                    1
                );
            }
        }
    }
    assert_eq!(
        worker.receipts.iter().map(Vec::len).sum::<usize>(),
        worker.accepted + 2
    );
    assert!(producer_owners.iter().all(Observer::is_empty));
    Run {
        owner,
        generations,
        samples,
        receipts: worker.receipts,
        commands: worker.commands,
        high,
        generated,
        refused: worker.refused,
        accepted: worker.accepted,
        halted_at,
        audio_calls,
    }
}

fn reference() -> Run {
    let (mut owner, generations) = linked(16, 32);
    transport(&mut owner, u64::try_from(STOP).unwrap());
    let mut samples = Vec::new();
    let mut receipts: [Vec<String>; 2] = std::array::from_fn(|_| Vec::new());
    let mut commands = Vec::new();
    for period in 0..PERIODS {
        if period > 0 && period <= WAVES {
            for (port, generation) in generations.into_iter().enumerate() {
                for envelope in wave(port, generation, Wave(period - 1)) {
                    match envelope.observation {
                        InputObservation::Message {
                            tick,
                            arrival,
                            input,
                        } => {
                            let _id = owner
                                .offer_message(generation, tick, arrival, input)
                                .unwrap();
                        }
                        InputObservation::Frontier { tick } => {
                            let _id = owner.advance_frontier(generation, tick).unwrap();
                        }
                    }
                }
            }
        }
        owner.pump().unwrap();
        samples.extend(render_input(&mut owner, PERIOD, &[PERIOD]));
        owner.pump().unwrap();
        for (port, generation) in generations.into_iter().enumerate() {
            while let Some(receipt) = owner.collect_input(generation).unwrap() {
                receipts[port].push(receipt_text(receipt));
            }
        }
        while let Some(r) = owner.collect() {
            commands.push(format!(
                "{:?} {:?} {:?}",
                r.boundary.command, r.outcome, r.capture
            ));
        }
    }
    owner.finalize().unwrap();
    Run {
        owner,
        generations,
        samples,
        receipts,
        commands,
        high: HighWater::default(),
        generated: WAVES * 6,
        refused: 0,
        accepted: WAVES * 6,
        halted_at: None,
        audio_calls: PERIODS,
    }
}

#[test]
fn continuous_delivery_controls_precede_supported_deadline_and_capacity_matrix() {
    // Delay counterfactual: same fixture, one period beyond the service contract.
    let late = execute(
        Scenario {
            worker_delay: FrameCount::new(512),
            ..Scenario::NORMAL
        },
        &[256],
    );
    assert_eq!(late.halted_at, Some(3));
    assert_eq!(
        late.owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
    assert!(late.refused > 0);
    // Queue counterfactuals: retain failed pushes; never rescue a missed cut by waiting.
    for scenario in [
        Scenario {
            ingress_cells: EventCount::measured(1),
            ..Scenario::NORMAL
        },
        Scenario {
            packet_cells: EventCount::measured(4),
            ..Scenario::NORMAL
        },
    ] {
        let short = execute(scenario, &[256]);
        assert!(short.high.failed_pushes > 0);
        assert!(short.halted_at.is_some());
        assert_eq!(
            short.owner.result().unwrap().sealed_outcome(),
            CaptureOutcome::Interrupted
        );
    }
    let reference = reference();
    assert_eq!(reference.owner.result().unwrap().records().count(), 32);
    assert_eq!(reference.commands.len(), 2);
    let expected = summary(&reference.owner, reference.generations);
    for delay in 0..=1 {
        for partitions in [&[256][..], &[64], &[1, 37, 128, 3, 87]] {
            let run = execute(
                Scenario {
                    worker_delay: FrameCount::new(delay * 256),
                    ..Scenario::NORMAL
                },
                partitions,
            );
            assert_eq!(run.halted_at, None);
            assert_eq!(run.generated, 48);
            assert_eq!(run.accepted, 48);
            assert_eq!(run.refused, 0);
            assert_eq!(run.samples, reference.samples);
            assert_eq!(summary(&run.owner, run.generations), expected);
            assert_eq!(run.receipts, reference.receipts);
            assert_eq!(run.commands, reference.commands);
            assert_eq!(
                run.owner.result().unwrap().sealed_outcome(),
                CaptureOutcome::Complete
            );
            assert!(run.audio_calls >= PERIODS);
            assert!(run.high.input.iter().all(|count| *count <= 16));
            assert!(run.high.sources <= 32);
            assert!(run.high.ingress.iter().all(|count| *count <= 9));
            assert!(run.high.packets <= 32);
            assert!(run.high.returns <= 8);
            println!(
                "continuous delay={delay} partitions={partitions:?} high={:?}",
                run.high
            );
        }
    }
}

#[test]
fn continuous_source_and_worker_stalls_halt_at_the_declared_cut() {
    for scenario in [
        Scenario {
            stall_worker_at: Some(Wave(3)),
            ..Scenario::NORMAL
        },
        Scenario {
            stall_frontier_at: Some(Wave(3)),
            ..Scenario::NORMAL
        },
    ] {
        let run = execute(scenario, &[64]);
        assert_eq!(run.halted_at, Some(6));
        assert_eq!(
            run.owner.result().unwrap().sealed_outcome(),
            CaptureOutcome::Interrupted
        );
        assert!(run.accepted > 0);
        assert!(run.refused > 0);
        assert!(
            run.samples[run.samples.len() - PERIOD..]
                .iter()
                .all(|sample| *sample == 0.0)
        );
        assert!(run.owner.result().unwrap().window().end().as_u64() <= 6 * 256);
    }
}

#[test]
fn continuous_no_final_callback_loss_retains_old_take_and_reconnects_explicitly() {
    let run = execute(
        Scenario {
            device_loss_at: Some(SampleTime::new(1536)),
            ..Scenario::NORMAL
        },
        &[256],
    );
    assert_eq!(run.samples.len(), 6 * PERIOD);
    assert_eq!(run.halted_at, Some(6));
    assert_eq!(
        run.owner.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
    let before: Vec<_> = run.owner.result().unwrap().records().copied().collect();
    // Waves 0, 1 and 2 render before loss; wave 3 starts at nominal frame 1552.
    assert_eq!(before.len(), 12);
    let (take, mut inputs) = run.owner.into_parts().unwrap();
    for (port, input) in inputs.iter_mut().enumerate() {
        let old = run.generations[port];
        let mapping = input.clock().unwrap();
        input.retire(old).unwrap();
        let new = input.begin().unwrap();
        input.prepare(new, mapping).unwrap();
        let InputObservation::Message {
            tick,
            arrival,
            input: note,
        } = wave(port, old, Wave(0))[0].observation
        else {
            panic!("message fixture");
        };
        assert_eq!(
            input.offer_message(old, tick, arrival, note),
            Err(InputError::Stale)
        );
        assert_eq!(input.state(), ConnectionState::Ready);
        assert!(input.source().is_none());
    }
    assert_eq!(
        take.result()
            .unwrap()
            .records()
            .copied()
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(
        take.result().unwrap().sealed_outcome(),
        CaptureOutcome::Interrupted
    );
}

#[test]
fn packet_correlation_identifies_only_its_retaining_input_owner() {
    let (mut control, _audio, _halt, generations) = split(8, 32);
    let (mut foreign, _foreign_audio, _foreign_halt, _) = split(8, 32);
    let command = control
        .prepare_command(SampleTime::ZERO, SessionCommand::Play)
        .unwrap();
    assert!(control.packet_input_id(&command).is_none());
    let foreign_command = foreign
        .prepare_command(SampleTime::ZERO, SessionCommand::Play)
        .unwrap();
    assert!(foreign.packet_input_id(&foreign_command).is_none());
    let source = control.next_packet().unwrap().unwrap();
    let id = control.packet_input_id(&source).unwrap();
    assert_eq!(id.generation(), generations[0]);
    assert_eq!(id.serial(), 1);
    assert!(foreign.packet_input_id(&source).is_none());
    let other = foreign.next_packet().unwrap().unwrap();
    assert_eq!(source.id().serial(), other.id().serial());
    assert!(control.packet_input_id(&other).is_none());
    assert_eq!(control.packet_input_id(&source), Some(id));
}
