//! Delay line primitives.
//!
//! This module provides generic delay line implementations
//! used by delay effects, chorus, flanger, and reverb.

use synth_core::{BufferIndex, SampleRate, Seconds};

/// A simple delay line with no interpolation.
///
/// Best for fixed-delay applications like reverb comb filters
/// where the delay time doesn't change.
#[derive(Debug, Clone)]
pub struct DelayLine {
    buffer: Vec<f32>,
    write_pos: BufferIndex,
}

impl DelayLine {
    /// Create a new delay line with the given maximum size.
    #[must_use]
    pub fn new(max_samples: usize) -> Self {
        Self {
            buffer: vec![0.0; max_samples.max(1)],
            write_pos: BufferIndex::ZERO,
        }
    }

    /// Create a delay line for a given maximum time at a sample rate.
    #[must_use]
    pub fn from_max_time(max_time: Seconds, sample_rate: SampleRate) -> Self {
        let max_samples = (max_time.as_f32() * sample_rate.as_f32()) as usize;
        Self::new(max_samples)
    }

    /// Get the buffer length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if the delay line is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Resize the delay line buffer.
    pub fn resize(&mut self, new_size: usize) {
        if self.buffer.len() != new_size {
            self.buffer.resize(new_size.max(1), 0.0);
            self.write_pos = BufferIndex::ZERO;
        }
    }

    /// Clear the delay line.
    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
    }

    /// Read a sample at the given delay (in samples).
    #[inline]
    #[must_use]
    pub fn read(&self, delay_samples: usize) -> f32 {
        let len = self.buffer.len();
        let read_pos = (self.write_pos.as_usize() + len - delay_samples.min(len - 1)) % len;
        self.buffer[read_pos]
    }

    /// Write a sample and advance the write position.
    #[inline]
    pub fn write(&mut self, sample: f32) {
        self.buffer[self.write_pos.as_usize()] = sample;
        self.write_pos = self.write_pos.advance(self.buffer.len());
    }

    /// Read and write in one operation (tap-and-replace).
    #[inline]
    #[must_use]
    pub fn process(&mut self, input: f32, delay_samples: usize) -> f32 {
        let output = self.read(delay_samples);
        self.write(input);
        output
    }
}

impl Default for DelayLine {
    fn default() -> Self {
        Self::new(1024)
    }
}

/// A delay line with linear interpolation for smooth modulation.
///
/// Best for chorus, flanger, and other modulated delay effects
/// where the delay time changes continuously.
#[derive(Debug, Clone)]
pub struct InterpolatedDelayLine {
    buffer: Vec<f32>,
    write_pos: BufferIndex,
}

impl InterpolatedDelayLine {
    /// Create a new interpolated delay line.
    #[must_use]
    pub fn new(max_samples: usize) -> Self {
        Self {
            buffer: vec![0.0; max_samples.max(2)],
            write_pos: BufferIndex::ZERO,
        }
    }

    /// Create from maximum time at a sample rate.
    #[must_use]
    pub fn from_max_time(max_time: Seconds, sample_rate: SampleRate) -> Self {
        let max_samples = (max_time.as_f32() * sample_rate.as_f32()) as usize;
        Self::new(max_samples)
    }

    /// Get the buffer length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Check if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Resize the buffer.
    pub fn resize(&mut self, new_size: usize) {
        if self.buffer.len() != new_size {
            self.buffer.resize(new_size.max(2), 0.0);
            self.write_pos = BufferIndex::ZERO;
        }
    }

    /// Clear the delay line.
    pub fn clear(&mut self) {
        self.buffer.fill(0.0);
    }

    /// Read with linear interpolation at a fractional delay.
    #[inline]
    #[must_use]
    pub fn read_interpolated(&self, delay_samples: f32) -> f32 {
        let len = self.buffer.len();
        let delay_clamped = delay_samples.clamp(0.0, (len - 1) as f32);
        self.write_pos
            .read_interpolated(&self.buffer, delay_clamped)
    }

    /// Read with cubic (Hermite) interpolation for higher quality.
    ///
    /// Provides smoother results than linear interpolation, especially
    /// for pitch-shifting and high-quality modulated delays.
    #[inline]
    #[must_use]
    pub fn read_cubic(&self, delay_samples: f32) -> f32 {
        let len = self.buffer.len();
        if len < 4 {
            return self.read_interpolated(delay_samples);
        }

        let delay_clamped = delay_samples.clamp(1.0, (len - 2) as f32);
        let read_pos = (self.write_pos.as_usize() as f32 - delay_clamped).rem_euclid(len as f32);
        let idx1 = (read_pos as usize) % len;
        let idx0 = (idx1 + len - 1) % len;
        let idx2 = (idx1 + 1) % len;
        let idx3 = (idx1 + 2) % len;
        let frac = read_pos - read_pos.floor();

        let y0 = self.buffer[idx0];
        let y1 = self.buffer[idx1];
        let y2 = self.buffer[idx2];
        let y3 = self.buffer[idx3];

        // Hermite interpolation coefficients
        let c0 = y1;
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);

        ((c3 * frac + c2) * frac + c1) * frac + c0
    }

    /// Write a sample and advance.
    #[inline]
    pub fn write(&mut self, sample: f32) {
        self.buffer[self.write_pos.as_usize()] = sample;
        self.write_pos = self.write_pos.advance(self.buffer.len());
    }

    /// Read interpolated and write in one operation.
    #[inline]
    #[must_use]
    pub fn process(&mut self, input: f32, delay_samples: f32) -> f32 {
        let output = self.read_interpolated(delay_samples);
        self.write(input);
        output
    }
}

impl Default for InterpolatedDelayLine {
    fn default() -> Self {
        Self::new(1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delay_line_basic() {
        let mut delay = DelayLine::new(10);

        // Write some samples
        for i in 0..5 {
            delay.write(i as f32);
        }

        // Read at various delays
        assert!((delay.read(1) - 4.0).abs() < 0.001);
        assert!((delay.read(2) - 3.0).abs() < 0.001);
    }

    #[test]
    fn test_delay_line_process() {
        let mut delay = DelayLine::new(100);

        // Fill with zeros first
        for _ in 0..50 {
            delay.write(0.0);
        }

        // Now write a pulse and read it back delayed
        delay.write(1.0);
        for _ in 0..49 {
            delay.write(0.0);
        }

        // The pulse should be at delay of 50
        assert!((delay.read(50) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_interpolated_delay() {
        let mut delay = InterpolatedDelayLine::new(100);

        // Write a ramp
        for i in 0..50 {
            delay.write(i as f32);
        }

        // Read at fractional delay - should interpolate
        let val_at_1_5 = delay.read_interpolated(1.5);
        let val_at_1 = delay.read_interpolated(1.0);
        let val_at_2 = delay.read_interpolated(2.0);

        // Interpolated value should be between the two integer delays
        assert!(val_at_1_5 > val_at_2.min(val_at_1));
        assert!(val_at_1_5 < val_at_1.max(val_at_2));
    }

    #[test]
    fn test_delay_from_time() {
        let delay = DelayLine::from_max_time(Seconds::new(1.0), SampleRate::DVD_QUALITY);
        // 1 second at 48kHz should be 48000 samples
        assert_eq!(delay.len(), 48000);
    }

    #[test]
    fn test_delay_clear() {
        let mut delay = DelayLine::new(10);
        for i in 0..10 {
            delay.write(i as f32);
        }
        delay.clear();
        assert!((delay.read(5)).abs() < 0.001);
    }
}
