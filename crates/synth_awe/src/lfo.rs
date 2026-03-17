//! AWE-internal control-rate LFOs.
//!
//! These LFOs run once per audio block (not per sample) and modulate
//! AWE parameters such as source position, room dimensions, and dry/wet mix.

use synth_core::{BipolarValue, Hertz, NormalizedValue, Phase, SampleCount, SampleRate};

use crate::params::AweLfoTarget;

/// Minimum LFO rate in Hz.
const MIN_RATE: Hertz = Hertz::new(0.01);

/// Maximum LFO rate in Hz.
const MAX_RATE: Hertz = Hertz::new(20.0);

/// An AWE-internal control-rate LFO.
///
/// Produces a bipolar sine wave (-1.0 to 1.0) scaled by an amount parameter.
/// Designed to run at control rate (once per audio block) for efficient
/// modulation of AWE parameters.
///
/// # Example
///
/// ```ignore
/// let mut lfo = AweLfo::new();
/// lfo.set_rate(2.0);
/// lfo.set_amount(0.5);
/// lfo.set_target(AweLfoTarget::SourceX);
///
/// // Call once per block
/// let modulation = lfo.advance(128, 48000.0);
/// // modulation is in range [-0.5, 0.5]
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct AweLfo {
    phase: Phase,
    /// Rate in Hz (clamped to 0.01 -- 20.0).
    rate: Hertz,
    /// Modulation amount (clamped to 0.0 -- 1.0).
    amount: NormalizedValue,
    /// Which parameter this LFO modulates.
    target: AweLfoTarget,
    /// Cached output value (-amount..+amount).
    current_value: BipolarValue,
}

impl AweLfo {
    /// Create a new AWE LFO with default settings.
    ///
    /// Defaults: rate = 0.5 Hz, amount = 0.0, target = `SourceX`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the LFO by one block of samples (control-rate update).
    ///
    /// Returns the current output value in the range (-amount..+amount).
    #[allow(clippy::cast_precision_loss)]
    pub fn advance(&mut self, block_size: SampleCount, sample_rate: SampleRate) -> BipolarValue {
        let block_duration = block_size / sample_rate;
        let phase_increment = self.rate.as_f32() * block_duration.as_f32();
        self.phase = self.phase.advance(phase_increment);
        let sine = self.phase.as_radians().sin();
        self.current_value = BipolarValue::new(sine * self.amount.as_f32());
        self.current_value
    }

    /// Get the current output value without advancing.
    #[must_use]
    pub fn value(&self) -> BipolarValue {
        self.current_value
    }

    /// Get the target parameter this LFO modulates.
    #[must_use]
    pub fn target(&self) -> AweLfoTarget {
        self.target
    }

    /// Set rate in Hz, clamped to [0.01, 20.0].
    pub fn set_rate(&mut self, rate_hz: Hertz) {
        let clamped = rate_hz.as_f32().clamp(MIN_RATE.as_f32(), MAX_RATE.as_f32());
        self.rate = Hertz::new(clamped);
    }

    /// Set modulation amount, clamped to [0.0, 1.0].
    pub fn set_amount(&mut self, amount: NormalizedValue) {
        self.amount = amount;
    }

    /// Set the modulation target parameter.
    pub fn set_target(&mut self, target: AweLfoTarget) {
        self.target = target;
    }

    /// Reset the LFO phase to zero.
    pub fn reset(&mut self) {
        self.phase.reset();
        self.current_value = BipolarValue::CENTER;
    }
}

impl Default for AweLfo {
    fn default() -> Self {
        Self {
            phase: Phase::ZERO,
            rate: Hertz::new(0.5),
            amount: NormalizedValue::MIN,
            target: AweLfoTarget::SourceX,
            current_value: BipolarValue::CENTER,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    #[test]
    fn test_default_values() {
        let lfo = AweLfo::new();
        assert!((lfo.rate.as_f32() - 0.5).abs() < f32::EPSILON);
        assert!((lfo.amount.as_f32() - 0.0).abs() < f32::EPSILON);
        assert_eq!(lfo.target(), AweLfoTarget::SourceX);
        assert!((lfo.value().as_f32() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_output_is_zero_when_amount_is_zero() {
        let mut lfo = AweLfo::new();
        lfo.set_rate(Hertz::new(1.0));
        // amount stays at 0.0
        for _ in 0..100 {
            let val = lfo.advance(SampleCount::new(128), SampleRate::new(48000.0));
            assert!((val.as_f32() - 0.0).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn test_output_range() {
        let mut lfo = AweLfo::new();
        lfo.set_rate(Hertz::new(5.0));
        lfo.set_amount(NormalizedValue::new(0.8));

        let mut min_val = f32::MAX;
        let mut max_val = f32::MIN;

        // Run many blocks to cover full phase range
        for _ in 0..10_000 {
            let val = lfo.advance(SampleCount::new(64), SampleRate::new(48000.0));
            min_val = min_val.min(val.as_f32());
            max_val = max_val.max(val.as_f32());
        }

        // Output should be within [-amount, +amount]
        assert!(min_val >= -0.8 - f32::EPSILON);
        assert!(max_val <= 0.8 + f32::EPSILON);
        // Should reach close to the extremes
        assert!(min_val < -0.7);
        assert!(max_val > 0.7);
    }

    #[test]
    fn test_rate_clamping() {
        let mut lfo = AweLfo::new();

        lfo.set_rate(Hertz::new(0.001));
        assert!((lfo.rate.as_f32() - MIN_RATE.as_f32()).abs() < f32::EPSILON);

        lfo.set_rate(Hertz::new(100.0));
        assert!((lfo.rate.as_f32() - MAX_RATE.as_f32()).abs() < f32::EPSILON);

        lfo.set_rate(Hertz::new(5.0));
        assert!((lfo.rate.as_f32() - 5.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_amount_clamping() {
        let mut lfo = AweLfo::new();

        lfo.set_amount(NormalizedValue::new(-0.5));
        assert!((lfo.amount.as_f32() - 0.0).abs() < f32::EPSILON);

        lfo.set_amount(NormalizedValue::new(2.0));
        assert!((lfo.amount.as_f32() - 1.0).abs() < f32::EPSILON);

        lfo.set_amount(NormalizedValue::new(0.7));
        assert!((lfo.amount.as_f32() - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn test_set_target() {
        let mut lfo = AweLfo::new();
        lfo.set_target(AweLfoTarget::DryWet);
        assert_eq!(lfo.target(), AweLfoTarget::DryWet);
    }

    #[test]
    fn test_reset() {
        let mut lfo = AweLfo::new();
        lfo.set_rate(Hertz::new(10.0));
        lfo.set_amount(NormalizedValue::new(1.0));

        // Advance several blocks
        for _ in 0..50 {
            lfo.advance(SampleCount::new(128), SampleRate::new(48000.0));
        }
        assert!((0.0..=1.0).contains(&lfo.value().as_f32())); // LFO value should be in [0, 1]

        lfo.reset();
        assert!((lfo.value().as_f32() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_sine_shape_quarter_cycle() {
        // At exactly 1/4 cycle the sine should be at its peak (1.0).
        let mut lfo = AweLfo::new();
        lfo.set_rate(Hertz::new(1.0));
        lfo.set_amount(NormalizedValue::new(1.0));

        // Advance exactly one quarter of a cycle: phase = 0.25
        // With sample_rate = 4.0 and block_size = 1, each advance moves phase by 0.25.
        let val = lfo.advance(SampleCount::new(1), SampleRate::new(4.0));

        // sin(0.25 * TAU) = sin(pi/2) = 1.0
        let expected = (0.25 * TAU).sin();
        assert!(
            (val.as_f32() - expected).abs() < 1e-5,
            "Expected {expected}, got {val}"
        );
    }

    #[test]
    fn test_advance_returns_cached_value() {
        let mut lfo = AweLfo::new();
        lfo.set_rate(Hertz::new(3.0));
        lfo.set_amount(NormalizedValue::new(0.5));

        let returned = lfo.advance(SampleCount::new(256), SampleRate::new(44100.0));
        assert!((returned.as_f32() - lfo.value().as_f32()).abs() < f32::EPSILON);
    }

    #[test]
    fn test_full_cycle_wraps() {
        let mut lfo = AweLfo::new();
        lfo.set_rate(Hertz::new(1.0));
        lfo.set_amount(NormalizedValue::new(1.0));

        // Advance exactly one full cycle: phase should wrap back near zero.
        // block_size = 48000, sample_rate = 48000 => increment = 1.0 per advance.
        lfo.advance(SampleCount::new(48000), SampleRate::new(48000.0));

        // After a full cycle, sine(0) = 0.
        assert!(
            lfo.value().as_f32().abs() < 1e-5,
            "Expected ~0 after full cycle, got {}",
            lfo.value()
        );
    }
}
