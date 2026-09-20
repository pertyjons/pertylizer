//! Thread orchestration is outside callback work; data custody uses fixed SPSC rings.
use super::*;

pub(super) enum ProducerStep {
    Generate(Wave),
    Finish,
}
pub(super) struct ProducerReport {
    pub high: usize,
    pub failed_pushes: usize,
}
pub(super) struct ProducerActor {
    sender: SyncSender<ProducerStep>,
    pub reports: Receiver<ProducerReport>,
    thread: std::thread::JoinHandle<(HeapProd<Envelope>, [Option<Envelope>; 12], usize)>,
}
impl ProducerActor {
    pub fn start(
        port: usize,
        generation: ConnectionGeneration,
        mut writer: HeapProd<Envelope>,
        scenario: Scenario,
    ) -> Self {
        let (sender, receiver) = sync_channel(1);
        let (report, reports) = sync_channel(1);
        let thread = std::thread::spawn(move || {
            let mut pending = [None; 12];
            let mut generated = 0;
            let mut high = 0;
            while let ProducerStep::Generate(id) = receive(&receiver) {
                for envelope in wave(port, generation, id) {
                    let free = pending.iter_mut().find(|cell| cell.is_none()).unwrap();
                    *free = Some(envelope);
                    generated += 1;
                }
                let mut failed_pushes = 0;
                for slot in &mut pending {
                    let Some(envelope) = slot.take() else {
                        continue;
                    };
                    if port == 0
                        && scenario.stall_frontier_at == Some(envelope.wave)
                        && matches!(envelope.observation, InputObservation::Frontier { .. })
                    {
                        *slot = Some(envelope);
                        break;
                    }
                    if let Err(envelope) = writer.try_push(envelope) {
                        *slot = Some(envelope);
                        failed_pushes += 1;
                        break;
                    }
                    high = high.max(writer.occupied_len());
                }
                // Preserve FIFO when failed sends survive into the next release.
                let mut write = 0;
                for read in 0..pending.len() {
                    if let Some(value) = pending[read].take() {
                        pending[write] = Some(value);
                        write += 1;
                    }
                }
                report
                    .send(ProducerReport {
                        high,
                        failed_pushes,
                    })
                    .unwrap();
            }
            (writer, pending, generated)
        });
        Self {
            sender,
            reports,
            thread,
        }
    }
    pub fn send(&self, step: ProducerStep) {
        self.sender.send(step).unwrap();
    }
    pub fn finish(self) -> (HeapProd<Envelope>, [Option<Envelope>; 12], usize) {
        self.send(ProducerStep::Finish);
        self.thread.join().unwrap()
    }
}

pub(super) enum AudioStep {
    Render,
    Finish,
}
pub(super) struct AudioReport {
    pub samples: [f32; PERIOD],
    pub returns: usize,
    pub calls: usize,
    pub closed: bool,
}
pub(super) struct AudioActor {
    sender: SyncSender<AudioStep>,
    pub reports: Receiver<AudioReport>,
    pub cuts: Receiver<()>,
    thread: std::thread::JoinHandle<Callback>,
}
impl AudioActor {
    pub fn start(mut callback: Callback, partitions: Vec<usize>) -> Self {
        let (sender, receiver) = sync_channel(1);
        let (report, reports) = sync_channel(1);
        let (cut, cuts) = sync_channel(1);
        let thread = std::thread::spawn(move || {
            let mut part = 0;
            while let AudioStep::Render = receive(&receiver) {
                // Capture/admit one bounded prefix. New worker pushes after this cut
                // stay queued until the next period, including across short partitions.
                assert_eq!(
                    crate::render_allocation::count_allocs(|| callback.admit()),
                    0
                );
                cut.send(()).unwrap(); // Fixture coordination, outside callback work.
                let mut samples = [9.0; PERIOD];
                let mut offset = 0;
                let mut calls = 0;
                let mut closed = false;
                assert_eq!(
                    crate::render_allocation::count_allocs(|| {
                        while offset < PERIOD {
                            let count = partitions[part % partitions.len()].min(PERIOD - offset);
                            closed |= callback
                                .audio
                                .render(
                                    AudioBlockMut::new(
                                        &mut samples[offset..offset + count],
                                        count,
                                        ChannelLayout::Mono,
                                    )
                                    .unwrap(),
                                )
                                .is_err();
                            callback.flush();
                            part += 1;
                            calls += 1;
                            offset += count;
                        }
                    }),
                    0
                );
                report
                    .send(AudioReport {
                        samples,
                        returns: callback.output.occupied_len(),
                        calls,
                        closed,
                    })
                    .unwrap();
            }
            callback
        });
        Self {
            sender,
            reports,
            cuts,
            thread,
        }
    }
    pub fn send(&self, step: AudioStep) {
        self.sender.send(step).unwrap();
    }
    pub fn finish(self) -> Callback {
        self.send(AudioStep::Finish);
        self.thread.join().unwrap()
    }
}

pub(super) enum MergerStep {
    Service(usize),
    Finish,
}
pub(super) struct MergerReport {
    pub frontiers: [SampleTime; 2],
    pub closed: bool,
    pub input: [usize; 2],
    pub sources: usize,
    pub packets: usize,
    pub failed_pushes: usize,
}
pub(super) struct MergerActor {
    sender: SyncSender<MergerStep>,
    pub reports: Receiver<MergerReport>,
    thread: std::thread::JoinHandle<Worker>,
}
impl MergerActor {
    pub fn start(
        control: InputCaptureControl,
        readers: Vec<HeapCons<Envelope>>,
        writer: HeapProd<LoopTransferPacket>,
        returns: HeapCons<LoopTransferCompletion>,
        generations: [ConnectionGeneration; 2],
        initial: usize,
        scenario: Scenario,
    ) -> Self {
        let (sender, receiver) = sync_channel(1);
        let (report, reports) = sync_channel(1);
        let thread = std::thread::spawn(move || {
            let mut worker = Worker {
                control,
                readers,
                writer,
                returns,
                generations,
                pending: None,
                mapping: Vec::with_capacity(32),
                receipts: std::array::from_fn(|_| Vec::new()),
                commands: Vec::new(),
                resolved: Vec::new(),
                accepted: 0,
                refused: 0,
                input_held: [1; 2],
                sources: initial,
                frontiers: [SampleTime::ZERO; 2],
                high: HighWater::default(),
                scenario,
            };
            while let MergerStep::Service(period) = receive(&receiver) {
                worker.service(period);
                report
                    .send(MergerReport {
                        frontiers: worker.frontiers,
                        closed: worker.generations.iter().any(|g| {
                            worker.control.input(*g).unwrap().state() == ConnectionState::Quiescing
                        }),
                        input: worker.high.input,
                        sources: worker.high.sources,
                        packets: worker.high.packets,
                        failed_pushes: worker.high.failed_pushes,
                    })
                    .unwrap();
                worker.high.failed_pushes = 0;
            }
            worker
        });
        Self {
            sender,
            reports,
            thread,
        }
    }
    pub fn send(&self, step: MergerStep) {
        self.sender.send(step).unwrap();
    }
    pub fn finish(self) -> Worker {
        self.send(MergerStep::Finish);
        self.thread.join().unwrap()
    }
}

pub(super) struct Worker {
    pub control: InputCaptureControl,
    readers: Vec<HeapCons<Envelope>>,
    writer: HeapProd<LoopTransferPacket>,
    returns: HeapCons<LoopTransferCompletion>,
    generations: [ConnectionGeneration; 2],
    pending: Option<(LoopTransferPacket, Option<Envelope>)>,
    mapping: Vec<(InputEventId, Envelope)>,
    pub receipts: [Vec<String>; 2],
    pub commands: Vec<String>,
    pub resolved: Vec<Envelope>,
    pub accepted: usize,
    pub refused: usize,
    input_held: [usize; 2],
    sources: usize,
    frontiers: [SampleTime; 2],
    high: HighWater,
    scenario: Scenario,
}
impl Worker {
    fn admit(&mut self, envelope: Envelope) {
        self.resolved.push(envelope);
        let port = self
            .generations
            .iter()
            .position(|g| *g == envelope.generation)
            .unwrap();
        match self
            .control
            .offer_observation(envelope.generation, envelope.observation)
        {
            Ok(id) => {
                self.mapping.push((id, envelope));
                self.accepted += 1;
                self.input_held[port] += 1;
                self.high.input[port] = self.high.input[port].max(self.input_held[port]);
            }
            Err((original, _)) => {
                assert_eq!(original, envelope.observation);
                self.refused += 1;
            }
        }
    }
    fn completed(&mut self, completion: LoopTransferCompletion) {
        match self.control.collect(completion).unwrap() {
            Some((_, LoopTransferOutcome::Command(r))) => self.commands.push(format!(
                "{:?} {:?} {:?}",
                r.boundary.command, r.outcome, r.capture
            )),
            Some((_, other)) => panic!("unexpected command outcome: {other:?}"),
            None => self.sources -= 1,
        }
    }
    fn collect_inputs(&mut self) {
        for (port, generation) in self.generations.into_iter().enumerate() {
            while let Some(receipt) = self.control.collect_input(generation).unwrap() {
                if receipt.id.serial() != 1 {
                    let index = self
                        .mapping
                        .iter()
                        .position(|(id, _)| *id == receipt.id)
                        .unwrap();
                    let (_id, original) = self.mapping.remove(index);
                    assert_eq!(receipt.observation, original.observation);
                }
                self.input_held[port] -= 1;
                self.receipts[port].push(receipt_text(receipt));
            }
        }
    }
    fn flush(&mut self) -> bool {
        let Some((packet, envelope)) = self.pending.take() else {
            return true;
        };
        match self.writer.try_push(packet) {
            Ok(()) => {
                self.high.packets = self.high.packets.max(self.writer.occupied_len());
                if let Some(envelope) = envelope
                    && let InputObservation::Frontier { tick } = envelope.observation
                {
                    let port = self
                        .generations
                        .iter()
                        .position(|g| *g == envelope.generation)
                        .unwrap();
                    let clock = self
                        .control
                        .input(envelope.generation)
                        .unwrap()
                        .clock()
                        .unwrap();
                    self.frontiers[port] = clock.map(tick).unwrap();
                }
                true
            }
            Err(packet) => {
                self.pending = Some((packet, envelope));
                self.high.failed_pushes += 1;
                false
            }
        }
    }
    fn service(&mut self, period: usize) {
        if self
            .scenario
            .stall_worker_at
            .is_some_and(|id| period >= id.release())
        {
            return;
        }
        while let Some(completion) = self.returns.try_pop() {
            self.completed(completion);
        }
        self.collect_inputs();
        for port in 0..self.readers.len() {
            loop {
                let ready = self.readers[port].first().is_some_and(|envelope| {
                    envelope.wave.release()
                        + usize::try_from(self.scenario.worker_delay.as_u64()).unwrap() / PERIOD
                        <= period
                });
                if !ready {
                    break;
                }
                let envelope = self.readers[port].try_pop().unwrap();
                self.admit(envelope);
            }
        }
        if !self.flush() {
            return;
        }
        while let Some(packet) = self.control.next_packet().unwrap() {
            let id = self.control.packet_input_id(&packet).unwrap();
            let envelope = self
                .mapping
                .iter()
                .find(|(input, _)| *input == id)
                .map(|(_, envelope)| *envelope);
            assert!(envelope.is_some());
            self.sources += 1;
            self.high.sources = self.high.sources.max(self.sources);
            self.pending = Some((packet, envelope));
            if !self.flush() {
                break;
            }
        }
    }
    pub fn recover(&mut self, callback: &mut Callback, remaining: Vec<Envelope>) {
        // All producers have joined; retained queue backing still lives here.
        for port in 0..self.readers.len() {
            while let Some(envelope) = self.readers[port].try_pop() {
                self.admit(envelope);
            }
        }
        for envelope in remaining {
            self.admit(envelope);
        }
        while let Some(completion) = self
            .returns
            .try_pop()
            .or_else(|| callback.failed.take())
            .or_else(|| callback.audio.take_completed())
        {
            self.completed(completion);
        }
        if let Some(packet) = callback.refused.take() {
            let _id = self.control.cancel(packet).unwrap();
            self.sources -= 1;
        }
        while let Some(packet) = callback.input.try_pop() {
            let _id = self.control.cancel(packet).unwrap();
            self.sources -= 1;
        }
        if let Some((packet, _)) = self.pending.take() {
            let _id = self.control.cancel(packet).unwrap();
            self.sources -= 1;
        }
        self.collect_inputs();
        assert_eq!(self.sources, 0);
        assert_eq!(self.input_held, [0; 2]);
        assert!(self.mapping.is_empty());
    }
}
