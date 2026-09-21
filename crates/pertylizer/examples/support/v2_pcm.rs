//! Independent input, monitor and recording-worker ownership. No callback owns a take.
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use std::sync::Arc;
use synth_engine_v2::{
    host::{
        OutputFormat,
        audio_input::{AudioInputConfig, InputFrame, MonitorCounters, SimulatedAudioInput},
    },
    quantities::PreparedBytes,
    render::AudioBlockMut,
    time::FrameCount,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PcmError {
    #[error("invalid PCM configuration or callback")]
    Configuration,
    #[error("PCM storage exceeds the prepared byte grant")]
    Bytes,
    #[error("PCM source is closed")]
    Closed,
}
#[derive(Clone, Copy)]
struct Frame {
    first: InputFrame,
    samples: [f32; 2],
    timing: Option<cpal::InputStreamTimestamp>,
}
#[derive(Debug, Clone, Copy)]
pub struct InputReport {
    pub next: InputFrame,
    pub gap: Option<InputFrame>,
    pub monitor_dropped: FrameCount,
    pub lost: bool,
}
pub struct PcmInput {
    record: HeapProd<Frame>,
    monitor: HeapProd<Frame>,
    channels: usize,
    maximum: usize,
    report: InputReport,
    closed: bool,
}
pub struct PcmOutput {
    queue: HeapCons<Frame>,
    monitor: SimulatedAudioInput,
    scratch: Box<[f32]>,
    channels: usize,
    maximum: usize,
    shed: FrameCount,
}
pub struct PcmWorker {
    queue: HeapCons<Frame>,
    // Backing ownership outlives both callbacks, including stream destruction.
    _record: Arc<HeapRb<Frame>>,
    _monitor: Arc<HeapRb<Frame>>,
    frames: Vec<Frame>,
    limit: usize,
    gap: Option<InputFrame>,
    input: OutputFormat,
}
#[must_use]
pub struct PcmTake {
    frames: Box<[Frame]>,
    pub input: OutputFormat,
    pub report: InputReport,
}
impl PcmTake {
    pub fn frames(&self) -> FrameCount {
        FrameCount::new(self.frames.len() as u64)
    }
    pub fn peak(&self) -> f32 {
        self.frames
            .iter()
            .flat_map(|frame| &frame.samples[..self.input.layout.channels()])
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
    }
    /// Backend-reported capture age, never a calibrated ADC or round-trip measurement.
    pub fn input_latency(&self) -> Option<(std::time::Duration, std::time::Duration)> {
        let mut range: Option<(std::time::Duration, std::time::Duration)> = None;
        for frame in &self.frames {
            if let Some(timing) = frame.timing
                && let Some(age) = timing.callback.checked_duration_since(timing.capture)
            {
                range = Some(range.map_or((age, age), |(low, high)| (low.min(age), high.max(age))));
            }
        }
        range
    }
}

pub fn prepare(
    config: AudioInputConfig,
    queue_frames: FrameCount,
) -> Result<(PcmInput, PcmOutput, PcmWorker, PreparedBytes), PcmError> {
    let channels = config.input.layout.channels();
    let maximum = usize::try_from(config.chunk_frames.as_u64()).map_err(|_| PcmError::Bytes)?;
    let cells = usize::try_from(queue_frames.as_u64()).map_err(|_| PcmError::Bytes)?;
    let take = usize::try_from(config.take_frames.as_u64()).map_err(|_| PcmError::Bytes)?;
    if !(1..=2).contains(&channels) || maximum == 0 || cells < maximum || take == 0 {
        return Err(PcmError::Configuration);
    }
    // The existing monitor is only a nominal-clock resampler here. Its synthetic
    // one-frame recorder is never used to construct or timestamp the physical take.
    let monitor = SimulatedAudioInput::prepare(AudioInputConfig {
        take_frames: FrameCount::new(1),
        chunk_cells: synth_engine_v2::quantities::EventCount::measured(1),
        ..config
    })
    .map_err(|_| PcmError::Configuration)?;
    let bytes = cells
        .checked_mul(2)
        .and_then(|n| n.checked_add(take))
        .and_then(|n| n.checked_mul(size_of::<Frame>()))
        .and_then(|n| {
            n.checked_add(
                maximum
                    .checked_mul(channels)?
                    .checked_mul(size_of::<f32>())?,
            )
        })
        .and_then(|n| {
            n.checked_add(
                2 * size_of::<HeapRb<Frame>>()
                    + size_of::<PcmInput>()
                    + size_of::<PcmOutput>()
                    + size_of::<PcmWorker>()
                    + 1024,
            )
        })
        .and_then(|n| u64::try_from(n).ok())
        .and_then(|n| n.checked_add(monitor.bytes().get()))
        .ok_or(PcmError::Bytes)?;
    if bytes > config.bytes.get() {
        return Err(PcmError::Bytes);
    }
    let record = Arc::new(HeapRb::new(cells));
    let monitoring = Arc::new(HeapRb::new(cells));
    let (record_writer, record_reader) = Arc::clone(&record).split();
    let (monitor_writer, monitor_reader) = Arc::clone(&monitoring).split();
    Ok((
        PcmInput {
            record: record_writer,
            monitor: monitor_writer,
            channels,
            maximum,
            closed: false,
            report: InputReport {
                next: config.first,
                gap: None,
                monitor_dropped: FrameCount::ZERO,
                lost: false,
            },
        },
        PcmOutput {
            queue: monitor_reader,
            monitor,
            channels,
            maximum,
            scratch: vec![0.0; maximum * channels].into_boxed_slice(),
            shed: FrameCount::ZERO,
        },
        PcmWorker {
            queue: record_reader,
            _record: record,
            _monitor: monitoring,
            frames: Vec::with_capacity(take),
            limit: take,
            gap: None,
            input: config.input,
        },
        PreparedBytes::measured(bytes),
    ))
}
impl PcmInput {
    pub fn process(
        &mut self,
        samples: &[f32],
        timing: Option<cpal::InputStreamTimestamp>,
    ) -> Result<(), PcmError> {
        if self.closed {
            return Err(PcmError::Closed);
        }
        let frames = samples.len() / self.channels;
        if frames == 0
            || frames > self.maximum
            || !samples.len().is_multiple_of(self.channels)
            || samples.iter().any(|value| !value.is_finite())
        {
            self.report.gap.get_or_insert(self.report.next);
            return Err(PcmError::Configuration);
        }
        let first = self.report.next.as_u64();
        let next = first.checked_add(frames as u64).ok_or_else(|| {
            self.report.gap.get_or_insert(self.report.next);
            PcmError::Configuration
        })?;
        if self.record.vacant_len() < frames {
            self.report.gap.get_or_insert(self.report.next);
        }
        let record = self.report.gap.is_none();
        let monitor = self.monitor.vacant_len() >= frames;
        if !monitor {
            self.report.monitor_dropped = FrameCount::new(
                self.report
                    .monitor_dropped
                    .as_u64()
                    .saturating_add(frames as u64),
            );
        }
        for frame in 0..frames {
            let mut values = [0.0; 2];
            for channel in 0..self.channels {
                values[channel] = samples[frame * self.channels + channel];
            }
            let packet = Frame {
                first: InputFrame::new(first + frame as u64),
                samples: values,
                timing: if frame == 0 { timing } else { None },
            };
            // Single producer: the consumer can only increase the reserved vacancy.
            if record && self.record.try_push(packet).is_err() {
                self.report.gap.get_or_insert(packet.first);
                return Err(PcmError::Configuration);
            }
            if monitor && self.monitor.try_push(packet).is_err() {
                return Err(PcmError::Configuration);
            }
        }
        self.report.next = InputFrame::new(next);
        Ok(())
    }
    pub fn stop(&mut self, lost: bool) -> InputReport {
        if lost {
            self.report.gap.get_or_insert(self.report.next);
        }
        self.closed = true;
        self.report.lost |= lost;
        self.report
    }
}
impl PcmOutput {
    pub fn render(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), PcmError> {
        if output.frames() > self.maximum {
            output.silence();
            return Err(PcmError::Configuration);
        }
        // Consumer-owned shedding leaves at most one maximum input block in transit.
        // Source frame stamps make every shed/drop an explicit interpolation reset.
        let prefix = self.queue.occupied_len();
        for _ in 0..prefix.saturating_sub(self.maximum) {
            if self.queue.try_pop().is_some() {
                self.shed = FrameCount::new(self.shed.as_u64().saturating_add(1));
            }
        }
        let mut copied = 0;
        let mut first = None;
        let available = self.queue.occupied_len().min(self.maximum);
        for _ in 0..available {
            let Some(packet) = self.queue.first() else {
                break;
            };
            if first.is_some_and(|start: InputFrame| {
                start.as_u64() + copied as u64 != packet.first.as_u64()
            }) {
                break;
            }
            let Some(packet) = self.queue.try_pop() else {
                break;
            };
            first.get_or_insert(packet.first);
            for channel in 0..self.channels {
                self.scratch[copied * self.channels + channel] = packet.samples[channel];
            }
            copied += 1;
        }
        if let Some(first) = first {
            let _gap = self
                .monitor
                .input(first, &self.scratch[..copied * self.channels])
                .map_err(|_| PcmError::Configuration)?;
        }
        self.monitor
            .monitor(output)
            .map_err(|_| PcmError::Configuration)
    }
    pub fn counters(&self) -> MonitorCounters {
        let mut counters = self.monitor.monitor_counters();
        counters.dropped =
            FrameCount::new(counters.dropped.as_u64().saturating_add(self.shed.as_u64()));
        counters
    }
    pub fn buffered(&self) -> FrameCount {
        FrameCount::new(self.monitor.buffered_frames().as_u64() + self.queue.occupied_len() as u64)
    }
}
impl PcmWorker {
    pub fn drain(&mut self) {
        let prefix = self.queue.occupied_len();
        for _ in 0..prefix {
            let Some(frame) = self.queue.try_pop() else {
                break;
            };
            if self.frames.len() == self.limit {
                self.gap.get_or_insert(frame.first);
            }
            if self.gap.is_none() {
                self.frames.push(frame);
            }
        }
    }
    /// Invoke only after the unique input owner has stopped and its callback joined.
    pub fn finish(mut self, mut report: InputReport) -> PcmTake {
        self.drain();
        if let Some(gap) = self.gap
            && report.gap.is_none_or(|prior| gap < prior)
        {
            report.gap = Some(gap);
        }
        PcmTake {
            frames: self.frames.into_boxed_slice(),
            input: self.input,
            report,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_engine_v2::{
        host::input::{InputRate, InputTick, InputTickSpan, SimulatedInputClock},
        quantities::{ChannelLayout, EventCount, SampleRate},
        time::{SampleTime, issue_epoch},
    };
    fn config(take: u64) -> AudioInputConfig {
        let format = OutputFormat {
            rate: SampleRate::new(48000.0).unwrap(),
            layout: ChannelLayout::Mono,
        };
        AudioInputConfig {
            input: format,
            output: format,
            clock: SimulatedInputClock::new(
                issue_epoch().unwrap(),
                SampleTime::ZERO,
                InputTick::new(0),
                InputRate::new(FrameCount::new(1), InputTickSpan::new(1)).unwrap(),
                InputTickSpan::new(0),
            ),
            first: InputFrame::new(0),
            chunk_frames: FrameCount::new(64),
            chunk_cells: EventCount::measured(1),
            take_frames: FrameCount::new(take),
            monitor_frames: FrameCount::new(1024),
            target_frames: FrameCount::new(256),
            bytes: PreparedBytes::measured(128_000_000),
        }
    }
    fn output(owner: &mut PcmOutput) -> [f32; 64] {
        let mut output = [0.0; 64];
        let measured = allocation_counter::measure(|| {
            owner
                .render(
                    AudioBlockMut::new(
                        &mut output,
                        64,
                        synth_engine_v2::quantities::ChannelLayout::Mono,
                    )
                    .unwrap(),
                )
                .unwrap()
        });
        assert_eq!(measured.count_total, 0);
        assert_eq!(measured.count_current, 0);
        output
    }
    #[test]
    fn worker_stall_and_monitor_loss_preserve_the_exact_original_prefix() {
        let (mut input, mut monitor, worker, _) =
            prepare(config(1024), FrameCount::new(128)).unwrap();
        for _ in 0..4 {
            let measured =
                allocation_counter::measure(|| input.process(&[0.125; 64], None).unwrap());
            assert_eq!(measured.count_total, 0);
            assert_eq!(measured.count_current, 0);
        }
        let _pcm = output(&mut monitor);
        let report = input.stop(true);
        let take = worker.finish(report);
        assert_eq!(take.frames().as_u64(), 128);
        assert_eq!(take.report.gap, Some(InputFrame::new(128)));
        assert!(take.report.lost);
        assert_eq!(take.report.monitor_dropped.as_u64(), 128);
        assert!(take.frames.iter().all(|frame| frame.samples[0] == 0.125));
    }
    #[test]
    fn worker_limit_and_no_final_output_callback_retain_take_and_source_timestamps() {
        let (mut input, _output, mut worker, _) =
            prepare(config(96), FrameCount::new(256)).unwrap();
        let timing = cpal::InputStreamTimestamp {
            callback: cpal::StreamInstant::from_millis(10),
            capture: cpal::StreamInstant::from_millis(7),
        };
        for _ in 0..3 {
            input.process(&[0.25; 64], Some(timing)).unwrap();
            worker.drain();
        }
        let take = worker.finish(input.stop(false));
        assert_eq!(take.frames().as_u64(), 96);
        assert_eq!(take.report.gap, Some(InputFrame::new(96)));
        assert_eq!(
            take.input_latency(),
            Some((
                std::time::Duration::from_millis(3),
                std::time::Duration::from_millis(3)
            ))
        );
        assert!(
            take.frames
                .iter()
                .enumerate()
                .all(|(index, frame)| frame.first.as_u64() == index as u64)
        );
    }
    #[test]
    fn independent_threads_retain_every_frame_and_bound_monitor_backlog() {
        let cycles = 4096;
        let (mut input, mut monitor, mut worker, _) =
            prepare(config(cycles * 64), FrameCount::new(512)).unwrap();
        let barrier = std::sync::Barrier::new(3);
        std::thread::scope(|scope| {
            let source = scope.spawn(|| {
                for cycle in 0..cycles {
                    barrier.wait();
                    let value = f32::from(u16::try_from(cycle % 1024).unwrap()) / 1024.0;
                    let measured =
                        allocation_counter::measure(|| input.process(&[value; 64], None).unwrap());
                    assert_eq!(measured.count_total, 0);
                    assert_eq!(measured.count_current, 0);
                    barrier.wait();
                }
            });
            let audio = scope.spawn(|| {
                for _ in 0..cycles {
                    barrier.wait();
                    assert!(output(&mut monitor).iter().all(|value| value.is_finite()));
                    assert!(monitor.buffered().as_u64() <= 1536);
                    barrier.wait();
                }
            });
            for _ in 0..cycles {
                barrier.wait();
                worker.drain();
                barrier.wait();
            }
            source.join().unwrap();
            audio.join().unwrap();
        });
        let take = worker.finish(input.stop(false));
        assert_eq!(take.frames().as_u64(), cycles * 64);
        assert!(take.report.gap.is_none());
        for (index, frame) in take.frames.iter().enumerate() {
            assert_eq!(
                frame.samples[0],
                f32::from(u16::try_from(index / 64 % 1024).unwrap()) / 1024.0
            );
        }
    }
    #[test]
    fn software_loopback_measures_monitor_delay_without_changing_recording_alignment() {
        let (mut input, mut monitor, mut worker, _) =
            prepare(config(1024), FrameCount::new(512)).unwrap();
        let mut observed = Vec::new();
        for cycle in 0..16 {
            let mut block = [0.0; 64];
            if cycle == 1 {
                block[16] = 1.0;
            }
            input.process(&block, None).unwrap();
            worker.drain();
            observed.extend(output(&mut monitor));
        }
        let peak = observed
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .unwrap()
            .0;
        let take = worker.finish(input.stop(false));
        assert_eq!(take.frames[80].samples[0], 1.0);
        assert!((128..=384).contains(&(peak - 80)));
        println!(
            "software_monitor_latency_frames={} input_alignment=original-source-frame physical_round_trip=unmeasured",
            peak - 80
        );
    }
    #[test]
    fn byte_grant_covers_real_allocations_and_reconfiguration_uses_new_owners() {
        let mut owners = None;
        let measured = allocation_counter::measure(|| {
            owners = Some(prepare(config(1024), FrameCount::new(512)).unwrap())
        });
        let (mut input, _monitor, worker, bytes) = owners.unwrap();
        assert!(
            measured.bytes_total <= bytes.get(),
            "actual={} charged={}",
            measured.bytes_total,
            bytes.get()
        );
        input.process(&[0.5; 64], None).unwrap();
        let first = worker.finish(input.stop(true));
        let mut next = config(1024);
        next.input.rate = SampleRate::new(44100.0).unwrap();
        next.output.rate = next.input.rate;
        let (mut input, _monitor, worker, _) = prepare(next, FrameCount::new(512)).unwrap();
        input.process(&[0.25; 64], None).unwrap();
        let second = worker.finish(input.stop(false));
        assert_eq!(first.input.rate.as_f32(), 48000.0);
        assert_eq!(second.input.rate.as_f32(), 44100.0);
        assert_eq!(first.peak(), 0.5);
        assert_eq!(second.peak(), 0.25);
    }
}
