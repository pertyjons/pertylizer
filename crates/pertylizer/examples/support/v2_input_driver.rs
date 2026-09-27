//! ALSA callback custody for the concrete simulated-input fixture.
use crate::input_host::{
    self, HostOutcome, InputOfferError, LiveAudio, archive::RetainedRuns, managed::ManagedRun,
    prepare::PreparedAttempt, source::SourceProducer,
};
use cpal::{
    Device, SampleFormat,
    traits::{DeviceTrait, StreamTrait},
};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Producer, Split},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use synth_engine_v2::{
    host::{
        input::{InputObservation, InputTick},
        session::{SessionCommand, SessionOutcome, loop_transfer::LoopTransferOutcome},
    },
    profile::HostProfile,
    quantities::{CaptureResultCount, ChannelLayout, PreparedBytes, SampleRate},
    recording::notes::Midi1Input,
    render::AudioBlockMut,
    time::{FrameCount, SampleTime},
};
use thiserror::Error;

#[derive(Error)]
enum DriverError {
    #[error("{0}")]
    Message(String),
    #[error("joined capture retains unresolved ownership: {0:?}")]
    Finish(Box<input_host::managed::FinishFailure>),
    #[error("callback custody failed after backend join: {message}")]
    Custody {
        message: String,
        device: Box<DeviceRun>,
        managed: Box<ManagedRun>,
    },
    #[error("source queue unresolved after producer join")]
    Source {
        producers: Vec<SourceProducer>,
        owners: Box<(ManagedRun, LiveAudio)>,
    },
}
impl std::fmt::Debug for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Custody {
                device, managed, ..
            } => {
                let _retained_control = managed;
                f.debug_struct("Custody")
                    .field("callbacks", &device.callbacks.load(Ordering::Acquire))
                    .field("error", &self.to_string())
                    .finish()
            }
            Self::Source { producers, owners } => {
                let _retained_endpoints = producers;
                f.debug_struct("Source")
                    .field("clock", &owners.1.clock())
                    .finish()
            }
            _ => f
                .debug_struct("DriverError")
                .field("error", &self.to_string())
                .finish(),
        }
    }
}
fn error(message: impl ToString) -> Box<dyn std::error::Error> {
    Box::new(DriverError::Message(message.to_string()))
}

struct Callback {
    audio: LiveAudio,
    scratch: Box<[f32]>,
    layout: ChannelLayout,
}
impl Callback {
    fn render<T: cpal::SizedSample + cpal::FromSample<f32>>(&mut self, output: &mut [T]) -> bool {
        let channels = self.layout.channels();
        if output.is_empty() || !output.len().is_multiple_of(channels) {
            return false;
        }
        let Some(scratch) = self.scratch.get_mut(..output.len()) else {
            return false;
        };
        let Ok(block) = AudioBlockMut::new(scratch, output.len() / channels, self.layout) else {
            return false;
        };
        if self.audio.render(block).is_err() {
            return false;
        }
        for (destination, sample) in output.iter_mut().zip(scratch) {
            *destination = T::from_sample(*sample);
        }
        true
    }
}

/// Pool backing stays on control until the ALSA stream destructor has joined.
struct DeviceRun {
    stream: Option<cpal::Stream>,
    pool: Arc<HeapRb<Callback>>,
    ready: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    backend_kinds: Arc<AtomicU64>,
    callbacks: Arc<AtomicU64>,
}
impl Drop for DeviceRun {
    fn drop(&mut self) {
        self.ready.store(false, Ordering::Release);
        drop(self.stream.take());
    }
}
impl DeviceRun {
    fn open(
        device: &Device,
    ) -> Result<(Self, HostProfile, ringbuf::HeapProd<Callback>), Box<dyn std::error::Error>> {
        let supported = device.default_output_config()?;
        let layout = match supported.channels() {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            _ => return Err(error("capture fixture supports mono/stereo output")),
        };
        if !matches!(
            supported.sample_format(),
            SampleFormat::F32 | SampleFormat::I32
        ) {
            return Err(error("capture fixture supports F32/I32 output"));
        }
        let raw_rate = supported.sample_rate();
        if !(8000..=synth_core::audio::DeviceSampleRate::MAX_SUPPORTED.as_u32()).contains(&raw_rate)
        {
            return Err(error("device sample rate outside fixture range"));
        }
        let rate = synth_core::audio::DeviceSampleRate::new(raw_rate);
        let rate = SampleRate::new(rate.as_f32())?;
        let pool = Arc::new(HeapRb::new(1));
        let (writer, mut reader) = Arc::clone(&pool).split();
        let ready = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let callbacks = Arc::new(AtomicU64::new(0));
        let backend_kinds = Arc::new(AtomicU64::new(0));
        let error_kinds = Arc::clone(&backend_kinds);
        let callback_ready = Arc::clone(&ready);
        let callback_failed = Arc::clone(&failed);
        let backend_failed = Arc::clone(&failed);
        let callback_count = Arc::clone(&callbacks);
        let stream = device.build_output_stream_raw(
            supported.config(),
            supported.sample_format(),
            move |data, _| {
                match data.sample_format() {
                    SampleFormat::F32 => {
                        if let Some(samples) = data.as_slice_mut::<f32>() {
                            samples.fill(0.0);
                        }
                    }
                    SampleFormat::I32 => {
                        if let Some(samples) = data.as_slice_mut::<i32>() {
                            samples.fill(0);
                        }
                    }
                    _ => {
                        callback_failed.store(true, Ordering::Release);
                        return;
                    }
                }
                if !callback_ready.load(Ordering::Acquire)
                    || callback_failed.load(Ordering::Acquire)
                {
                    return;
                }
                let Some(callback): Option<&mut Callback> = reader.first_mut() else {
                    callback_failed.store(true, Ordering::Release);
                    return;
                };
                let success = match data.sample_format() {
                    SampleFormat::F32 => data
                        .as_slice_mut::<f32>()
                        .is_some_and(|samples| callback.render(samples)),
                    SampleFormat::I32 => data
                        .as_slice_mut::<i32>()
                        .is_some_and(|samples| callback.render(samples)),
                    _ => false,
                };
                if !success
                    || callback_count
                        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                            count.checked_add(1)
                        })
                        .is_err()
                {
                    callback_failed.store(true, Ordering::Release);
                }
            },
            move |error| {
                let bit = crate::linux::BACKEND_KINDS
                    .iter()
                    .position(|kind| *kind == error.kind())
                    .unwrap_or(63);
                error_kinds.fetch_or(1_u64 << bit, Ordering::Relaxed);
                backend_failed.store(true, Ordering::Release);
            },
            None,
        )?;
        let bound = FrameCount::new(u64::from(stream.buffer_size()?));
        if bound.as_u64() == 0 || bound.as_u64() > 8192 {
            return Err(error("callback bound outside fixture ceiling"));
        }
        let profile = HostProfile::harness(rate, bound, layout)?;
        Ok((
            Self {
                stream: Some(stream),
                pool,
                ready,
                failed,
                backend_kinds,
                callbacks,
            },
            profile,
            writer,
        ))
    }
    fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.ready.store(true, Ordering::Release);
        self.stream
            .as_ref()
            .ok_or_else(|| error("stream already joined"))?
            .play()?;
        Ok(())
    }
    fn join(&mut self) -> Result<Callback, Box<dyn std::error::Error>> {
        self.ready.store(false, Ordering::Release);
        drop(self.stream.take());
        Arc::get_mut(&mut self.pool)
            .and_then(Consumer::try_pop)
            .ok_or_else(|| error("callback custody did not join"))
    }
}

fn source_wave(
    scale: u64,
    notes: bool,
    start: SampleTime,
    stop: SampleTime,
) -> Result<Vec<InputObservation>, Box<dyn std::error::Error>> {
    let mut observations = vec![InputObservation::Frontier {
        tick: InputTick::new(start.as_u64() * scale),
    }];
    if notes {
        for (at, bytes) in [
            (start.as_u64() + 64, [0x90, 60, 100]),
            (stop.as_u64() - 128, [0x80, 60, 0]),
        ] {
            observations.push(InputObservation::Message {
                tick: InputTick::new(at * scale),
                arrival: SampleTime::new(at),
                input: Midi1Input::from_bytes(bytes)?,
            });
        }
    }
    observations.push(InputObservation::Frontier {
        tick: InputTick::new(stop.as_u64() * scale),
    });
    Ok(observations)
}

fn send_wave(producer: &mut SourceProducer, wave: &[InputObservation]) -> Vec<InputObservation> {
    let mut refused = Vec::with_capacity(wave.len());
    for observation in wave {
        if let Err(observation) = producer.send(*observation) {
            refused.push(observation);
        }
    }
    refused
}

fn service(managed: &mut ManagedRun, stopped: &mut bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut input_fault = false;
    managed.service(
        |result| {
            if let Err(fault) = result {
                match fault {
                    InputOfferError::Refused(observation, reason) => {
                        eprintln!("source_refusal={observation:?} reason={reason:?}");
                    }
                    InputOfferError::Accepted {
                        id,
                        error,
                        settlement_error,
                    } => {
                        eprintln!("accepted_input_fault={id:?} error={error:?} settlement={settlement_error:?}");
                    }
                }
                input_fault = true;
            }
        },
        |id, outcome| {
            if let HostOutcome::Delivered(LoopTransferOutcome::Command(receipt)) = &outcome {
                *stopped |= matches!(
                    receipt.boundary.command,
                    SessionCommand::Stop | SessionCommand::Panic
                ) && matches!(receipt.outcome, SessionOutcome::Applied { .. });
            }
            println!("command={id:?} outcome={outcome:?}");
        },
        |receipt| println!("input={receipt:?}"),
    )?;
    let _collected_plans = managed.collect_live_plans();
    while let Some((id, outcome)) = managed.collect_audition() {
        println!("audition={id:?} outcome={outcome:?}");
    }
    if input_fault {
        return Err(error("input delivery failed"));
    }
    Ok(())
}

struct TransportProgram {
    start: SampleTime,
    stop: SampleTime,
    end: SessionCommand,
}

// Every fallible live operation returns to the caller's common join/recovery path.
fn exercise(
    device: &DeviceRun,
    managed: &mut ManagedRun,
    producers: &mut [SourceProducer; 2],
    waves: &[Vec<InputObservation>; 2],
    target: u64,
    program: TransportProgram,
) -> Result<(), Box<dyn std::error::Error>> {
    let TransportProgram { start, stop, end } = program;
    let mut stopped = false;
    service(managed, &mut stopped)?; // Publish initial source promises before audio runs.
    device.start()?;
    let mut sent = false;
    let mut swapped = false;
    let started = Instant::now();
    loop {
        if device.failed.load(Ordering::Acquire) {
            return Err(error("backend or callback fault"));
        }
        if !sent && device.callbacks.load(Ordering::Acquire) > 0 {
            let _play = managed.command(start, SessionCommand::Play)?;
            let _stop = managed.command(stop, end)?;
            // Workers borrow endpoints. Even a worker panic returns endpoint custody
            // to this owner after scoped joins; it never loses the unique close proof.
            let [first, second] = producers;
            let results = std::thread::scope(|scope| {
                let first = scope.spawn(|| send_wave(first, &waves[0]));
                let second = scope.spawn(|| send_wave(second, &waves[1]));
                [first.join(), second.join()]
            });
            let mut failure = false;
            for result in results {
                match result {
                    Ok(refused) => {
                        for observation in refused {
                            eprintln!("source_shutdown_refusal={observation:?}");
                            failure = true;
                        }
                    }
                    Err(_) => {
                        eprintln!("source worker panicked; endpoint retained after join");
                        failure = true;
                    }
                }
            }
            if failure {
                return Err(error("source worker failed"));
            }
            sent = true;
        }
        service(managed, &mut stopped)?;
        if sent && !swapped && device.callbacks.load(Ordering::Acquire) > 2 {
            let graph = input_host::audition::live_graph()?;
            let plan = managed.publish_live_plan(&graph)?;
            println!("live_plan_published={plan:?}");
            swapped = true;
        }
        if device.callbacks.load(Ordering::Acquire) >= target {
            if !stopped {
                return Err(error("callback target ended before ordered Stop"));
            }
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err(error("callback timeout"));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub fn run(device: &Device, target: u64) -> Result<(), Box<dyn std::error::Error>> {
    let mut archive = None;
    let mut epochs = Vec::new();
    let mut run_error = None;
    for attempt in 0..2 {
        let (mut device_run, profile, mut writer) = DeviceRun::open(device)?;
        let prepared =
            PreparedAttempt::counted(profile, synth_engine_v2::tempo::MusicalTick::new(1920))?
                .with_audition()?;
        let start = prepared.start();
        let stop = start.checked_add(FrameCount::new(start.as_u64()))?;
        let waves = [
            source_wave(1, true, start, stop)?,
            source_wave(2, false, start, stop)?,
        ];
        let end = if attempt == 0 {
            SessionCommand::Stop
        } else {
            SessionCommand::Panic
        };
        let epoch = prepared.epoch();
        if archive.is_none() {
            let ceiling = prepared
                .bytes()
                .get()
                .checked_mul(3)
                .and_then(|n| n.checked_add(65536))
                .ok_or_else(|| error("archive layout overflow"))?;
            archive = Some(RetainedRuns::prepare(
                CaptureResultCount::limit(2)?,
                prepared.bytes(),
                PreparedBytes::measured(ceiling),
            )?);
        }
        let retained = archive
            .as_mut()
            .ok_or_else(|| error("archive not prepared"))?;
        let layout = profile.capabilities().channel_layout();
        let frames = usize::try_from(profile.capabilities().maximum_block_size().as_u64())?;
        let scratch = vec![0.0; frames * layout.channels()].into_boxed_slice();
        let (mut managed, audio, mut producers) = ManagedRun::start(retained, prepared)?;
        let callback = Callback {
            audio,
            scratch,
            layout,
        };
        if let Err(callback) = writer.try_push(callback) {
            return Err(Box::new(DriverError::Finish(Box::new(
                input_host::managed::FinishFailure::Pending(
                    Box::new((managed, callback.audio)),
                    input_host::HostError::Pending,
                ),
            ))));
        }
        drop(writer);
        if let Err(failure) = exercise(
            &device_run,
            &mut managed,
            &mut producers,
            &waves,
            target,
            TransportProgram { start, stop, end },
        ) {
            managed.halt_handle().request_device_lost();
            run_error = Some(failure);
        }
        let callback = match device_run.join() {
            Ok(callback) => callback,
            Err(failure) => {
                return Err(Box::new(DriverError::Custody {
                    message: failure.to_string(),
                    device: Box::new(device_run),
                    managed: Box::new(managed),
                }));
            }
        };
        println!(
            "attempt={attempt} backend_joined=true backend_kind_bits={}",
            device_run.backend_kinds.load(Ordering::Acquire)
        );
        // Draining producer observations is mandatory even after a terminal fault.
        // service reports each original refusal before it returns a pump error.
        if let Err(failure) = service(&mut managed, &mut false) {
            eprintln!("joined_service={failure}");
            if run_error.is_none() {
                run_error = Some(failure);
            }
        }
        let mut unresolved = Vec::new();
        for producer in producers {
            if let Err(producer) = managed.close_source(producer) {
                unresolved.push(producer);
            }
        }
        if !unresolved.is_empty() {
            return Err(Box::new(DriverError::Source {
                producers: unresolved,
                owners: Box::new((managed, callback.audio)),
            }));
        }
        managed
            .finish(
                callback.audio,
                run_error.is_some(),
                retained,
                |id, outcome| println!("final_command={id:?} outcome={outcome:?}"),
                |receipt| println!("final_input={receipt:?}"),
                |id, outcome| println!("audition={id:?} outcome={outcome:?}"),
            )
            .map_err(|failure| Box::new(DriverError::Finish(failure)))?;
        epochs.push(epoch);
        println!(
            "attempt={attempt} epoch={epoch:?} result_retained=true archive_bytes={}",
            retained.bytes().get()
        );
        if run_error.is_some() {
            break;
        }
    }
    let retained = archive.as_mut().ok_or_else(|| error("no archive"))?;
    for epoch in epochs {
        let mut owner = retained
            .take(epoch)
            .ok_or_else(|| error("retained take missing"))?;
        let result = owner.result()?;
        println!(
            "collected_epoch={epoch:?} outcome={:?} records={} passes={}",
            result.effective_outcome(),
            result.records().count(),
            result.loop_passes().count()
        );
        match owner.project_notes() {
            Ok(projection) => println!(
                "projected_epoch={epoch:?} segments={} mode={:?}",
                projection.notes().len(),
                projection.raw().context().mode()
            ),
            Err(refusal) => println!("projection_refused_epoch={epoch:?} reason={refusal}"),
        }
    }
    match run_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
