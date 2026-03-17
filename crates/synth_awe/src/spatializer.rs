//! Simple stereo spatializer using ITD (interaural time difference)
//! and ILD (interaural level difference).
//!
//! Applied to the wet signal in AWE to create spatial positioning.
//! Uses two interpolated delay lines (one per ear) for ITD,
//! gain-per-ear for ILD, and a one-pole low-pass head shadow filter
//! on the ear farther from the source.

use synth_core::{
    BipolarValue, FilterState, Gain, NormalizedValue, SampleCount, SampleRate, Seconds,
};
use synth_dsp::InterpolatedDelayLine;

use crate::room::SPEED_OF_SOUND;
use crate::types::{Meters, Position3, SampleOffset};

/// Head width in meters (average distance between human ears).
const HEAD_WIDTH: Meters = Meters::new(0.17);

/// Maximum delay line size in samples.
/// At 96 kHz: 0.17 / 343 * 96000 ~ 47 samples. Pre-allocate 64 for headroom.
const MAX_ITD_SAMPLES: SampleCount = SampleCount::new(64);

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
    itd_left: SampleOffset,
    /// Current right-ear ITD in samples.
    itd_right: SampleOffset,
    /// Gain applied to left-ear signal (ILD).
    gain_left: Gain,
    /// Gain applied to right-ear signal (ILD).
    gain_right: Gain,
    /// One-pole filter state for left-ear head shadow.
    shadow_state_left: FilterState,
    /// One-pole filter state for right-ear head shadow.
    shadow_state_right: FilterState,
    /// Head shadow LP coefficient for the left ear.
    shadow_coeff_left: NormalizedValue,
    /// Head shadow LP coefficient for the right ear.
    shadow_coeff_right: NormalizedValue,
}

impl Spatializer {
    /// Create a new spatializer with default (center) positioning.
    pub fn new() -> Self {
        Self {
            delay_left: InterpolatedDelayLine::new(MAX_ITD_SAMPLES.as_usize()),
            delay_right: InterpolatedDelayLine::new(MAX_ITD_SAMPLES.as_usize()),
            itd_left: SampleOffset::new(0.0),
            itd_right: SampleOffset::new(0.0),
            gain_left: Gain::UNITY,
            gain_right: Gain::UNITY,
            shadow_state_left: FilterState::ZERO,
            shadow_state_right: FilterState::ZERO,
            shadow_coeff_left: NormalizedValue::MIN,
            shadow_coeff_right: NormalizedValue::MIN,
        }
    }

    /// Update spatializer parameters from source and listener positions.
    ///
    /// Computes azimuth angle from positions, then derives ITD, ILD,
    /// and head shadow coefficients.
    ///
    /// Coordinate convention: x = left/right, y = front/back, z = up/down.
    /// Angle 0 = directly ahead, positive = source to the right.
    pub fn update(
        &mut self,
        source_pos: Position3,
        listener_pos: Position3,
        sample_rate: SampleRate,
    ) {
        let dx = source_pos.x().as_f32() - listener_pos.x().as_f32();
        let dy = source_pos.y().as_f32() - listener_pos.y().as_f32();

        // Azimuth angle: 0 = directly ahead (positive y), positive = right (positive x).
        let angle = dx.atan2(dy);

        // --- ITD ---
        // Time difference based on path length difference around the head.
        let itd_seconds = Seconds::new(HEAD_WIDTH.as_f32() / SPEED_OF_SOUND.as_f32() * angle.sin());

        // Positive angle (source right): left ear is farther, gets more delay.
        self.itd_left = SampleOffset::new(itd_seconds.as_f32().max(0.0) * sample_rate.as_f32());
        self.itd_right = SampleOffset::new((-itd_seconds.as_f32()).max(0.0) * sample_rate.as_f32());

        // --- ILD ---
        // Simple equal-power panning based on angle.
        let pan = BipolarValue::new(angle.sin());
        self.gain_left = Gain::new((1.0 - pan.as_f32() * 0.5).sqrt());
        self.gain_right = Gain::new((1.0 + pan.as_f32() * 0.5).sqrt());

        // --- Head shadow ---
        // More HF attenuation on the far ear. Coefficient closer to 1 = more LP.
        // Near ear gets coefficient ~0.0 (no filtering), far ear gets higher.
        let shadow_amount = NormalizedValue::new(0.3 + 0.5 * angle.abs() / std::f32::consts::PI);

        if angle >= 0.0 {
            // Source to the right: left ear is shadowed (higher coeff = more LP).
            self.shadow_coeff_left = shadow_amount;
            self.shadow_coeff_right = NormalizedValue::MIN;
        } else {
            // Source to the left: right ear is shadowed (higher coeff = more LP).
            self.shadow_coeff_left = NormalizedValue::MIN;
            self.shadow_coeff_right = shadow_amount;
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
        let left_raw = self.delay_left.read_interpolated(self.itd_left.as_f32());
        let right_raw = self.delay_right.read_interpolated(self.itd_right.as_f32());

        // Apply head shadow (one-pole LP) per ear.
        let left_filtered = self
            .shadow_state_left
            .one_pole(left_raw, self.shadow_coeff_left.as_f32());
        let right_filtered = self
            .shadow_state_right
            .one_pole(right_raw, self.shadow_coeff_right.as_f32());

        // Apply ILD gain.
        let left = self.gain_left.apply(left_filtered);
        let right = self.gain_right.apply(right_filtered);

        (left, right)
    }

    /// Clear all internal state (delay buffers and filter states).
    pub fn clear(&mut self) {
        self.delay_left.clear();
        self.delay_right.clear();
        self.shadow_state_left.reset();
        self.shadow_state_right.reset();
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

    const SAMPLE_RATE: SampleRate = SampleRate(48_000.0);

    fn pos(x: f32, y: f32, z: f32) -> Position3 {
        Position3::new(Meters::new(x), Meters::new(y), Meters::new(z))
    }

    #[test]
    fn test_center_source_equal_output() {
        let mut spat = Spatializer::new();
        // Source directly ahead.
        spat.update(pos(0.0, 5.0, 0.0), pos(0.0, 0.0, 0.0), SAMPLE_RATE);

        assert!((spat.itd_left.as_f32() - 0.0).abs() < 1e-6);
        assert!((spat.itd_right.as_f32() - 0.0).abs() < 1e-6);
        assert!((spat.gain_left.as_f32() - spat.gain_right.as_f32()).abs() < 1e-6);
    }

    #[test]
    fn test_right_source_more_left_delay() {
        let mut spat = Spatializer::new();
        // Source to the right.
        spat.update(pos(5.0, 0.0, 0.0), pos(0.0, 0.0, 0.0), SAMPLE_RATE);

        // Left ear should have more delay (farther from source).
        assert!(spat.itd_left.as_f32() > spat.itd_right.as_f32());
        // Right ear should be louder.
        assert!(spat.gain_right.as_f32() > spat.gain_left.as_f32());
    }

    #[test]
    fn test_left_source_more_right_delay() {
        let mut spat = Spatializer::new();
        // Source to the left.
        spat.update(pos(-5.0, 0.0, 0.0), pos(0.0, 0.0, 0.0), SAMPLE_RATE);

        // Right ear should have more delay.
        assert!(spat.itd_right.as_f32() > spat.itd_left.as_f32());
        // Left ear should be louder.
        assert!(spat.gain_left.as_f32() > spat.gain_right.as_f32());
    }

    #[test]
    fn test_max_itd_within_bounds() {
        let mut spat = Spatializer::new();
        let high_rate = SampleRate::new(96_000.0);
        // Source at 90 degrees right.
        spat.update(pos(10.0, 0.0, 0.0), pos(0.0, 0.0, 0.0), high_rate);

        // Max ITD ~ HEAD_WIDTH / SPEED_OF_SOUND * sample_rate = 0.17/343*96000 ~ 47.5
        assert!(spat.itd_left.as_f32() < MAX_ITD_SAMPLES.as_usize() as f32);
        assert!(spat.itd_right.as_f32() < MAX_ITD_SAMPLES.as_usize() as f32);
    }

    #[test]
    fn test_process_produces_output() {
        let mut spat = Spatializer::new();
        spat.update(pos(3.0, 4.0, 0.0), pos(0.0, 0.0, 0.0), SAMPLE_RATE);

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
        spat.update(pos(5.0, 0.0, 0.0), pos(0.0, 0.0, 0.0), SAMPLE_RATE);

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
        spat.update(pos(10.0, 0.0, 0.0), pos(0.0, 0.0, 0.0), SAMPLE_RATE);

        // Left ear shadow coeff should be higher (more LP filtering on far ear).
        assert!(spat.shadow_coeff_left.as_f32() > spat.shadow_coeff_right.as_f32());
    }

    #[test]
    fn test_default_is_center() {
        let spat = Spatializer::default();
        // Default should be centered: equal gains, zero delays.
        assert!((spat.gain_left.as_f32() - 1.0).abs() < 1e-6);
        assert!((spat.gain_right.as_f32() - 1.0).abs() < 1e-6);
        assert!(spat.itd_left.as_f32().abs() < 1e-6);
        assert!(spat.itd_right.as_f32().abs() < 1e-6);
    }

    #[test]
    fn test_symmetric_positioning() {
        let mut spat_left = Spatializer::new();
        let mut spat_right = Spatializer::new();

        spat_left.update(pos(-3.0, 4.0, 0.0), pos(0.0, 0.0, 0.0), SAMPLE_RATE);
        spat_right.update(pos(3.0, 4.0, 0.0), pos(0.0, 0.0, 0.0), SAMPLE_RATE);

        // ITDs should be swapped.
        assert!((spat_left.itd_left.as_f32() - spat_right.itd_right.as_f32()).abs() < 1e-3);
        assert!((spat_left.itd_right.as_f32() - spat_right.itd_left.as_f32()).abs() < 1e-3);

        // Gains should be swapped.
        assert!((spat_left.gain_left.as_f32() - spat_right.gain_right.as_f32()).abs() < 1e-6);
        assert!((spat_left.gain_right.as_f32() - spat_right.gain_left.as_f32()).abs() < 1e-6);
    }
}
