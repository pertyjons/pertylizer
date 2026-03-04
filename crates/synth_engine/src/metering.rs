//! Audio metering system for the synthesizer engine.
//!
//! Provides peak and RMS metering with configurable update intervals.

use ringbuf::traits::Producer;
use std::sync::Arc;

use synth_core::{Amplitude, SampleRate};

use crate::commands::EngineEvent;
use crate::state::EngineState;

/// Audio metering system that tracks peak and RMS levels.
pub struct MeteringSystem {
    /// Counter for meter update interval
    counter: usize,
    /// Samples between meter updates
    interval: usize,
    /// Current peak left level
    peak_left: Amplitude,
    /// Current peak right level
    peak_right: Amplitude,
    /// Running RMS sum for left channel (intermediate accumulator)
    rms_sum_left: f32,
    /// Running RMS sum for right channel (intermediate accumulator)
    rms_sum_right: f32,
}

impl MeteringSystem {
    /// Create a new metering system with the given sample rate.
    #[allow(clippy::cast_possible_truncation)]
    pub fn new(sample_rate: SampleRate) -> Self {
        Self {
            counter: 0,
            interval: (sample_rate.as_f32() / 24.0) as usize, // ~24 updates per second
            peak_left: Amplitude::ZERO,
            peak_right: Amplitude::ZERO,
            rms_sum_left: 0.0,
            rms_sum_right: 0.0,
        }
    }

    /// Set the sample rate and update the meter interval.
    #[allow(clippy::cast_possible_truncation)]
    pub fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.interval = (sample_rate.as_f32() / 24.0) as usize;
    }

    /// Update meters with output audio data.
    ///
    /// Returns true if a meter update was sent (interval reached).
    #[allow(clippy::cast_precision_loss)]
    pub fn update(
        &mut self,
        output: &[f32],
        state: &Arc<EngineState>,
        event_producer: &mut impl Producer<Item = EngineEvent>,
    ) -> bool {
        let channels = 2;

        for frame in output.chunks(channels) {
            let left = frame.first().copied().unwrap_or(0.0);
            let right = frame.get(1).copied().unwrap_or(left);

            self.peak_left = Amplitude::new(self.peak_left.as_f32().max(left.abs()));
            self.peak_right = Amplitude::new(self.peak_right.as_f32().max(right.abs()));
            self.rms_sum_left += left * left;
            self.rms_sum_right += right * right;
        }

        self.counter += output.len() / channels;

        if self.counter >= self.interval {
            // Update shared state
            state.meters.update_peak(self.peak_left, self.peak_right);

            let rms_left = Amplitude::new((self.rms_sum_left / self.counter as f32).sqrt());
            let rms_right = Amplitude::new((self.rms_sum_right / self.counter as f32).sqrt());
            state.meters.update_rms(rms_left, rms_right);

            // Send event to UI
            let _ = event_producer.try_push(EngineEvent::PeakMeter {
                left: self.peak_left,
                right: self.peak_right,
            });

            // Reset accumulators
            self.peak_left = Amplitude::ZERO;
            self.peak_right = Amplitude::ZERO;
            self.rms_sum_left = 0.0;
            self.rms_sum_right = 0.0;
            self.counter = 0;

            true
        } else {
            false
        }
    }

    /// Reset all meter values to zero.
    pub fn reset(&mut self) {
        self.peak_left = Amplitude::ZERO;
        self.peak_right = Amplitude::ZERO;
        self.rms_sum_left = 0.0;
        self.rms_sum_right = 0.0;
        self.counter = 0;
    }
}

impl Default for MeteringSystem {
    fn default() -> Self {
        Self::new(SampleRate::DVD_QUALITY)
    }
}
