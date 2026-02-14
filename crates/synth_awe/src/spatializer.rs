//! Simple stereo spatializer using ITD (interaural time difference)
//! and ILD (interaural level difference).
//!
//! Applied to the wet signal in AWE to create spatial positioning.
//! Uses two interpolated delay lines (one per ear) for ITD,
//! gain-per-ear for ILD, and a one-pole low-pass head shadow filter
//! on the ear farther from the source.

use synth_dsp::InterpolatedDelayLine;

use crate::room::SPEED_OF_SOUND;

/// Head width in meters (average distance between human ears).
const HEAD_WIDTH: f32 = 0.17;

/// Maximum delay line size in samples.
/// At 96 kHz: 0.17 / 343 * 96000 ~ 47 samples. Pre-allocate 64 for headroom.
const MAX_ITD_SAMPLES: usize = 64;

/// Stereo spatializer using ITD and ILD.
///
/// Models interaural time difference via per-ear delay lines,
/// interaural level difference via per-ear gain, and a simple
/// one-pole low-pass head shadow filter on the far ear.
#[derive(Debug, Clone)]
#[must_use]
pub struct Spatializer {
    delay_left: InterpolatedDelayLine,
    delay_right: InterpolatedDelayLine,
    /// Current left-ear ITD in samples.
    itd_left: f32,
    /// Current right-ear ITD in samples.
    itd_right: f32,
    /// Gain applied to left-ear signal (ILD).
    gain_left: f32,
    /// Gain applied to right-ear signal (ILD).
    gain_right: f32,
    /// One-pole filter state for left-ear head shadow.
    shadow_state_left: f32,
    /// One-pole filter state for right-ear head shadow.
    shadow_state_right: f32,
    /// Head shadow LP coefficient for the left ear.
    shadow_coeff_left: f32,
    /// Head shadow LP coefficient for the right ear.
    shadow_coeff_right: f32,
}

impl Spatializer {
    /// Create a new spatializer with default (center) positioning.
    pub fn new() -> Self {
        Self {
            delay_left: InterpolatedDelayLine::new(MAX_ITD_SAMPLES),
            delay_right: InterpolatedDelayLine::new(MAX_ITD_SAMPLES),
            itd_left: 0.0,
            itd_right: 0.0,
            gain_left: 1.0,
            gain_right: 1.0,
            shadow_state_left: 0.0,
            shadow_state_right: 0.0,
            shadow_coeff_left: 1.0,
            shadow_coeff_right: 1.0,
        }
    }

    /// Update spatializer parameters from source and listener positions.
    ///
    /// Computes azimuth angle from positions, then derives ITD, ILD,
    /// and head shadow coefficients.
    ///
    /// Coordinate convention: x = left/right, y = front/back, z = up/down.
    /// Angle 0 = directly ahead, positive = source to the right.
    pub fn update(&mut self, source_pos: [f32; 3], listener_pos: [f32; 3], sample_rate: f32) {
        let dx = source_pos[0] - listener_pos[0];
        let dy = source_pos[1] - listener_pos[1];

        // Azimuth angle: 0 = directly ahead (positive y), positive = right (positive x).
        let angle = dx.atan2(dy);

        // --- ITD ---
        // Time difference based on path length difference around the head.
        let itd_seconds = HEAD_WIDTH / SPEED_OF_SOUND * angle.sin();

        // Positive angle (source right): left ear is farther, gets more delay.
        self.itd_left = itd_seconds.max(0.0) * sample_rate;
        self.itd_right = (-itd_seconds).max(0.0) * sample_rate;

        // --- ILD ---
        // Simple equal-power panning based on angle.
        let pan = angle.sin().clamp(-1.0, 1.0);
        self.gain_left = (1.0 - pan * 0.5).sqrt();
        self.gain_right = (1.0 + pan * 0.5).sqrt();

        // --- Head shadow ---
        // More HF attenuation on the far ear. Coefficient closer to 0 = more LP.
        // Near ear gets coefficient ~1.0 (no filtering), far ear gets lower.
        let shadow_amount = 0.3 + 0.5 * angle.abs() / std::f32::consts::PI;

        if angle >= 0.0 {
            // Source to the right: left ear is shadowed.
            self.shadow_coeff_left = 1.0 - shadow_amount;
            self.shadow_coeff_right = 1.0;
        } else {
            // Source to the left: right ear is shadowed.
            self.shadow_coeff_left = 1.0;
            self.shadow_coeff_right = 1.0 - shadow_amount;
        }
    }

    /// Process a mono wet signal into spatialized stereo.
    ///
    /// Returns `(left, right)`.
    #[inline]
    #[must_use]
    pub fn process(&mut self, input: f32) -> (f32, f32) {
        // Write mono input to both delay lines.
        self.delay_left.write(input);
        self.delay_right.write(input);

        // Read with ITD.
        let left_raw = self.delay_left.read_interpolated(self.itd_left);
        let right_raw = self.delay_right.read_interpolated(self.itd_right);

        // Apply head shadow (one-pole LP) per ear.
        self.shadow_state_left += self.shadow_coeff_left * (left_raw - self.shadow_state_left);
        self.shadow_state_right += self.shadow_coeff_right * (right_raw - self.shadow_state_right);

        // Apply ILD gain.
        let left = self.shadow_state_left * self.gain_left;
        let right = self.shadow_state_right * self.gain_right;

        (left, right)
    }

    /// Clear all internal state (delay buffers and filter states).
    pub fn clear(&mut self) {
        self.delay_left.clear();
        self.delay_right.clear();
        self.shadow_state_left = 0.0;
        self.shadow_state_right = 0.0;
    }
}

impl Default for Spatializer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: f32 = 48_000.0;

    #[test]
    fn test_center_source_equal_output() {
        let mut spat = Spatializer::new();
        // Source directly ahead.
        spat.update([0.0, 5.0, 0.0], [0.0, 0.0, 0.0], SAMPLE_RATE);

        assert!((spat.itd_left - 0.0).abs() < 1e-6);
        assert!((spat.itd_right - 0.0).abs() < 1e-6);
        assert!((spat.gain_left - spat.gain_right).abs() < 1e-6);
    }

    #[test]
    fn test_right_source_more_left_delay() {
        let mut spat = Spatializer::new();
        // Source to the right.
        spat.update([5.0, 0.0, 0.0], [0.0, 0.0, 0.0], SAMPLE_RATE);

        // Left ear should have more delay (farther from source).
        assert!(spat.itd_left > spat.itd_right);
        // Right ear should be louder.
        assert!(spat.gain_right > spat.gain_left);
    }

    #[test]
    fn test_left_source_more_right_delay() {
        let mut spat = Spatializer::new();
        // Source to the left.
        spat.update([-5.0, 0.0, 0.0], [0.0, 0.0, 0.0], SAMPLE_RATE);

        // Right ear should have more delay.
        assert!(spat.itd_right > spat.itd_left);
        // Left ear should be louder.
        assert!(spat.gain_left > spat.gain_right);
    }

    #[test]
    fn test_max_itd_within_bounds() {
        let mut spat = Spatializer::new();
        let high_rate = 96_000.0;
        // Source at 90 degrees right.
        spat.update([10.0, 0.0, 0.0], [0.0, 0.0, 0.0], high_rate);

        // Max ITD ~ HEAD_WIDTH / SPEED_OF_SOUND * sample_rate = 0.17/343*96000 ~ 47.5
        assert!(spat.itd_left < MAX_ITD_SAMPLES as f32);
        assert!(spat.itd_right < MAX_ITD_SAMPLES as f32);
    }

    #[test]
    fn test_process_produces_output() {
        let mut spat = Spatializer::new();
        spat.update([3.0, 4.0, 0.0], [0.0, 0.0, 0.0], SAMPLE_RATE);

        // Feed a pulse.
        let (l, r) = spat.process(1.0);

        // With ITD delay, initial output may be near zero from delay lines,
        // but after several samples we should get signal.
        let mut max_l = l.abs();
        let mut max_r = r.abs();
        for _ in 0..100 {
            let (sl, sr) = spat.process(0.0);
            max_l = max_l.max(sl.abs());
            max_r = max_r.max(sr.abs());
        }

        assert!(max_l > 0.0, "Left channel should have output");
        assert!(max_r > 0.0, "Right channel should have output");
    }

    #[test]
    fn test_clear_resets_state() {
        let mut spat = Spatializer::new();
        spat.update([5.0, 0.0, 0.0], [0.0, 0.0, 0.0], SAMPLE_RATE);

        // Feed some signal.
        for _ in 0..50 {
            let _ = spat.process(1.0);
        }

        spat.clear();

        // After clear, processing silence should yield silence.
        let (l, r) = spat.process(0.0);
        assert!((l).abs() < 1e-10);
        assert!((r).abs() < 1e-10);
    }

    #[test]
    fn test_head_shadow_far_ear_attenuated() {
        // Source far right: left ear should be more attenuated (shadow).
        let mut spat = Spatializer::new();
        spat.update([10.0, 0.0, 0.0], [0.0, 0.0, 0.0], SAMPLE_RATE);

        // Left ear shadow coeff should be less than right.
        assert!(spat.shadow_coeff_left < spat.shadow_coeff_right);
    }

    #[test]
    fn test_default_is_center() {
        let spat = Spatializer::default();
        // Default should be centered: equal gains, zero delays.
        assert!((spat.gain_left - 1.0).abs() < 1e-6);
        assert!((spat.gain_right - 1.0).abs() < 1e-6);
        assert!((spat.itd_left).abs() < 1e-6);
        assert!((spat.itd_right).abs() < 1e-6);
    }

    #[test]
    fn test_symmetric_positioning() {
        let mut spat_left = Spatializer::new();
        let mut spat_right = Spatializer::new();

        spat_left.update([-3.0, 4.0, 0.0], [0.0, 0.0, 0.0], SAMPLE_RATE);
        spat_right.update([3.0, 4.0, 0.0], [0.0, 0.0, 0.0], SAMPLE_RATE);

        // ITDs should be swapped.
        assert!((spat_left.itd_left - spat_right.itd_right).abs() < 1e-3);
        assert!((spat_left.itd_right - spat_right.itd_left).abs() < 1e-3);

        // Gains should be swapped.
        assert!((spat_left.gain_left - spat_right.gain_right).abs() < 1e-6);
        assert!((spat_left.gain_right - spat_right.gain_left).abs() < 1e-6);
    }
}
