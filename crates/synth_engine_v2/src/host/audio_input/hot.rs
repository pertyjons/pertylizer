//! Bounded synthetic input/output callbacks. The worker owns immutable take construction.
use super::{AudioGap, AudioGapReason, AudioInputError, InputFrame, PcmStamp, SimulatedAudioInput};
use crate::{host::input::InputTick, render::AudioBlockMut, time::FrameCount};
impl SimulatedAudioInput {
    pub(super) fn mark_gap(&mut self, first: InputFrame, reason: AudioGapReason) {
        if self.gap.is_none_or(|prior| first < prior.first) {
            self.gap = Some(AudioGap { first, reason });
        }
    }
    fn drop_monitor(&mut self, frames: usize) {
        let count = frames.min(self.monitor_len);
        self.monitor_head = (self.monitor_head + count) % self.monitor_capacity;
        self.monitor_len -= count;
        self.phase = 0.0;
        self.control_remaining = 0;
        self.counters.dropped =
            FrameCount::new(self.counters.dropped.as_u64().saturating_add(count as u64));
    }
    /// Original source PCM reaches recording first. Monitor loss never changes its bytes.
    pub fn input(
        &mut self,
        first: InputFrame,
        samples: &[f32],
    ) -> Result<Option<AudioGap>, AudioInputError> {
        if self.closed {
            return Err(AudioInputError::Closed);
        }
        let frames = samples.len() / self.channels;
        if !samples.len().is_multiple_of(self.channels)
            || frames == 0
            || frames > self.chunk_frames
            || samples.iter().any(|sample| !sample.is_finite())
        {
            self.mark_gap(self.expected, AudioGapReason::InvalidInput);
            self.drop_monitor(self.monitor_len);
            self.primed = false;
            return Err(AudioInputError::Configuration);
        }
        if first != self.expected {
            self.mark_gap(self.expected, AudioGapReason::SourceGap);
            self.drop_monitor(self.monitor_len);
            self.primed = false;
        }
        let next = first.0.checked_add(frames as u64).ok_or_else(|| {
            self.mark_gap(self.expected, AudioGapReason::ClockRange);
            AudioInputError::Configuration
        })?;
        let nominal = self
            .config
            .clock
            .map(InputTick::new(first.0))
            .map_err(|_| {
                self.mark_gap(self.expected, AudioGapReason::ClockRange);
                AudioInputError::Configuration
            })?;
        if self.gap.is_none() {
            if self.occupied == self.cells.len() {
                self.mark_gap(first, AudioGapReason::PoolFull);
            } else {
                let begin = self.head * self.chunk_frames * self.channels;
                self.pcm[begin..begin + samples.len()].copy_from_slice(samples);
                self.cells[self.head] = Some(PcmStamp {
                    first,
                    nominal,
                    frames: FrameCount::new(frames as u64),
                });
                self.head = (self.head + 1) % self.cells.len();
                self.occupied += 1;
            }
        }
        self.expected = InputFrame(next);
        if self.monitor_len + frames > self.monitor_capacity {
            self.drop_monitor(self.monitor_len + frames - self.monitor_capacity);
        }
        for frame in 0..frames {
            let destination = (self.monitor_head + self.monitor_len) % self.monitor_capacity;
            for channel in 0..self.channels {
                self.monitor[destination * self.channels + channel] =
                    samples[frame * self.channels + channel];
            }
            self.monitor_len += 1;
        }
        Ok(self.gap)
    }
    /// Call after the source stops, also on device loss without another output callback.
    pub fn stop(&mut self, lost: bool) {
        if !self.closed && lost {
            self.mark_gap(self.expected, AudioGapReason::DeviceLost);
        }
        self.closed = true;
    }
    pub fn monitor(&mut self, mut output: AudioBlockMut<'_>) -> Result<(), AudioInputError> {
        if output.layout() != self.config.output.layout
            || output.frames() == 0
            || output.frames() > self.chunk_frames
        {
            output.silence();
            return Err(AudioInputError::Configuration);
        }
        output.silence();
        if self.closed {
            return Ok(());
        }
        if !self.primed && self.monitor_len >= self.target {
            self.primed = true;
        }
        for frame in 0..output.frames() {
            if !self.primed || self.monitor_len < 10 {
                self.counters.silent =
                    FrameCount::new(self.counters.silent.as_u64().saturating_add(1));
                self.phase = 0.0;
                self.primed = false;
                continue;
            }
            if self.control_remaining == 0 {
                // Fixed output-frame cadence, never callback cadence. The feedback is a
                // bounded synthetic occupancy estimator, not hardware clock calibration.
                let occupancy = u32::try_from(self.monitor_len).unwrap_or(u32::MAX);
                let target = u32::try_from(self.target).unwrap_or(u32::MAX);
                let error = (f64::from(occupancy) - f64::from(target)) / f64::from(target);
                self.step = self.ratio * (1.0 + (error * 0.001).clamp(-0.001, 0.001));
                self.control_remaining = 64;
            }
            self.control_remaining -= 1;
            let next = (self.monitor_head + 1) % self.monitor_capacity;
            for channel in 0..self.channels {
                let a = f64::from(self.monitor[self.monitor_head * self.channels + channel]);
                let b = f64::from(self.monitor[next * self.channels + channel]);
                // A convex interpolation of two validated finite f32 values fits f32.
                #[allow(clippy::cast_possible_truncation)]
                let sample = (a * (1.0 - self.phase) + b * self.phase) as f32;
                output.samples_mut()[frame * self.channels + channel] = sample;
            }
            self.phase += self.step;
            // Ratio <= 8.008 and fractional phase < 1: at most nine source frames.
            let mut consumed = 0;
            while self.phase >= 1.0 {
                self.phase -= 1.0;
                consumed += 1;
            }
            self.monitor_head = (self.monitor_head + consumed) % self.monitor_capacity;
            self.monitor_len -= consumed;
        }
        Ok(())
    }
}
