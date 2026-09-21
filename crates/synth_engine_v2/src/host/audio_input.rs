//! Exclusively scheduled synthetic audio capture and independent-clock monitoring.
//! Input, output and worker calls model separate clocks, not concurrent device callbacks.
mod hot;
use super::{
    OutputFormat,
    input::{InputTick, SimulatedInputClock},
};
use crate::{
    quantities::{EventCount, PreparedBytes, SampleRateRange},
    time::{FrameCount, SampleTime},
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[must_use]
pub struct InputFrame(u64);
impl InputFrame {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmStamp {
    pub first: InputFrame,
    pub nominal: SampleTime,
    pub frames: FrameCount,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioGapReason {
    SourceGap,
    InvalidInput,
    PoolFull,
    TakeFull,
    DeviceLost,
    ClockRange,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioGap {
    pub first: InputFrame,
    pub reason: AudioGapReason,
}
#[derive(Debug, Error)]
pub enum AudioInputError {
    #[error("invalid synthetic audio configuration or callback shape")]
    Configuration,
    #[error("synthetic audio preparation exceeds its byte grant")]
    Bytes,
    #[error("synthetic audio allocation failed")]
    Allocation,
    #[error("synthetic audio input is closed")]
    Closed,
}
#[derive(Debug, Clone, Copy)]
pub struct AudioInputConfig {
    pub input: OutputFormat,
    pub output: OutputFormat,
    pub clock: SimulatedInputClock,
    pub first: InputFrame,
    pub chunk_frames: FrameCount,
    pub chunk_cells: EventCount,
    pub take_frames: FrameCount,
    pub monitor_frames: FrameCount,
    pub target_frames: FrameCount,
    pub bytes: PreparedBytes,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorCounters {
    pub dropped: FrameCount,
    pub silent: FrameCount,
}
#[must_use]
pub struct AudioTake {
    format: OutputFormat,
    clock: SimulatedInputClock,
    samples: Box<[f32]>,
    stamps: Box<[PcmStamp]>,
    gap: Option<AudioGap>,
}
impl AudioTake {
    pub const fn clock(&self) -> SimulatedInputClock {
        self.clock
    }
    pub const fn format(&self) -> OutputFormat {
        self.format
    }
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
    pub fn stamps(&self) -> &[PcmStamp] {
        &self.stamps
    }
    pub const fn first_gap(&self) -> Option<AudioGap> {
        self.gap
    }
}
#[must_use]
pub struct SimulatedAudioInput {
    config: AudioInputConfig,
    channels: usize,
    chunk_frames: usize,
    cells: Box<[Option<PcmStamp>]>,
    pcm: Box<[f32]>,
    head: usize,
    tail: usize,
    occupied: usize,
    expected: InputFrame,
    gap: Option<AudioGap>,
    closed: bool,
    worker_samples: Vec<f32>,
    worker_stamps: Vec<PcmStamp>,
    monitor: Box<[f32]>,
    monitor_capacity: usize,
    target: usize,
    monitor_head: usize,
    monitor_len: usize,
    phase: f64,
    ratio: f64,
    step: f64,
    control_remaining: u32,
    primed: bool,
    counters: MonitorCounters,
    bytes: PreparedBytes,
}
fn extent(count: FrameCount) -> Result<usize, AudioInputError> {
    usize::try_from(count.as_u64()).map_err(|_| AudioInputError::Bytes)
}
fn reserve<T>(count: usize) -> Result<Vec<T>, AudioInputError> {
    let mut value = Vec::new();
    value
        .try_reserve_exact(count)
        .map_err(|_| AudioInputError::Allocation)?;
    if value.capacity() != count {
        return Err(AudioInputError::Bytes);
    }
    Ok(value)
}
impl SimulatedAudioInput {
    pub fn prepare(config: AudioInputConfig) -> Result<Self, AudioInputError> {
        let chunk = extent(config.chunk_frames)?;
        let cells =
            usize::try_from(config.chunk_cells.get()).map_err(|_| AudioInputError::Bytes)?;
        let take = extent(config.take_frames)?;
        let monitor_capacity = extent(config.monitor_frames)?;
        let target = extent(config.target_frames)?;
        let channels = config.input.layout.channels();
        let ratio = f64::from(config.input.rate.as_f32()) / f64::from(config.output.rate.as_f32());
        if chunk == 0
            || cells == 0
            || take == 0
            || target < 16
            || u32::try_from(monitor_capacity).is_err()
            || monitor_capacity < target.saturating_add(chunk).saturating_add(16)
            || config.input.layout != config.output.layout
            || !SampleRateRange::engine_supported().contains(config.input.rate)
            || !SampleRateRange::engine_supported().contains(config.output.rate)
            || !(0.125..=8.0).contains(&ratio)
            || config.clock.map(InputTick::new(config.first.0)).is_err()
        {
            return Err(AudioInputError::Configuration);
        }
        let pool_samples = cells
            .checked_mul(chunk)
            .and_then(|n| n.checked_mul(channels))
            .ok_or(AudioInputError::Bytes)?;
        let take_samples = take.checked_mul(channels).ok_or(AudioInputError::Bytes)?;
        let monitor_samples = monitor_capacity
            .checked_mul(channels)
            .ok_or(AudioInputError::Bytes)?;
        // One-frame callbacks are legal, hence at most one stamp per retained frame.
        let bytes = pool_samples
            .checked_add(take_samples)
            .and_then(|n| n.checked_add(monitor_samples))
            .and_then(|n| n.checked_mul(size_of::<f32>()))
            .and_then(|n| n.checked_add(cells.checked_mul(size_of::<Option<PcmStamp>>())?))
            .and_then(|n| n.checked_add(take.checked_mul(size_of::<PcmStamp>())?))
            .and_then(|n| n.checked_add(size_of::<Self>()))
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(AudioInputError::Bytes)?;
        if bytes > config.bytes.get() {
            return Err(AudioInputError::Bytes);
        }
        let mut metadata = reserve(cells)?;
        metadata.resize(cells, None);
        let mut pcm = reserve(pool_samples)?;
        pcm.resize(pool_samples, 0.0);
        let mut monitor = reserve(monitor_samples)?;
        monitor.resize(monitor_samples, 0.0);
        Ok(Self {
            config,
            channels,
            chunk_frames: chunk,
            cells: metadata.into_boxed_slice(),
            pcm: pcm.into_boxed_slice(),
            head: 0,
            tail: 0,
            occupied: 0,
            expected: config.first,
            gap: None,
            closed: false,
            worker_samples: reserve(take_samples)?,
            worker_stamps: reserve(take)?,
            monitor: monitor.into_boxed_slice(),
            monitor_capacity,
            target,
            monitor_head: 0,
            monitor_len: 0,
            phase: 0.0,
            ratio,
            step: ratio,
            control_remaining: 0,
            primed: false,
            counters: MonitorCounters {
                dropped: FrameCount::ZERO,
                silent: FrameCount::ZERO,
            },
            bytes: PreparedBytes::measured(bytes),
        })
    }
    pub const fn bytes(&self) -> PreparedBytes {
        self.bytes
    }
    pub const fn first_gap(&self) -> Option<AudioGap> {
        self.gap
    }
    pub const fn monitor_counters(&self) -> MonitorCounters {
        self.counters
    }
    pub fn buffered_frames(&self) -> FrameCount {
        FrameCount::new(self.monitor_len as u64)
    }
    /// Worker-side copy; never called by either simulated callback. A full take
    /// retains the exact prefix and consumes later queued chunks without appending.
    pub fn drain_worker(&mut self) {
        for _ in 0..self.cells.len() {
            if self.occupied == 0 {
                break;
            }
            let Some(stamp) = self.cells[self.tail].take() else {
                break;
            };
            let frames = usize::try_from(stamp.frames.as_u64()).unwrap_or(0);
            let count = frames * self.channels;
            if self.gap.is_none_or(|gap| stamp.first < gap.first) {
                if self.worker_samples.len() + count > self.worker_samples.capacity() {
                    self.mark_gap(stamp.first, AudioGapReason::TakeFull);
                } else {
                    let begin = self.tail * self.chunk_frames * self.channels;
                    self.worker_samples
                        .extend_from_slice(&self.pcm[begin..begin + count]);
                    self.worker_stamps.push(stamp);
                }
            }
            self.tail = (self.tail + 1) % self.cells.len();
            self.occupied -= 1;
        }
    }
    /// Exclusive stopped/joined worker ownership; no final callback is necessary.
    pub fn finish(mut self) -> AudioTake {
        self.closed = true;
        self.drain_worker();
        AudioTake {
            format: self.config.input,
            clock: self.config.clock,
            samples: self.worker_samples.into_boxed_slice(),
            stamps: self.worker_stamps.into_boxed_slice(),
            gap: self.gap,
        }
    }
}
