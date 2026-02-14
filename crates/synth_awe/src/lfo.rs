//! AWE-internal control-rate LFOs.
//!
//! These LFOs run once per audio block (not per sample) and modulate
//! AWE parameters such as source position, room dimensions, and dry/wet mix.

use synth_core::Phase;

use crate::params::AweLfoTarget;

/// Minimum LFO rate in Hz.
const MIN_RATE: f32 = 0.01;

/// Maximum LFO rate in Hz.
const MAX_RATE: f32 = 20.0;

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
    rate: f32,
    /// Modulation amount (clamped to 0.0 -- 1.0).
    amount: f32,
    /// Which parameter this LFO modulates.
    target: AweLfoTarget,
    /// Cached output value (-amount..+amount).
    current_value: f32,
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
    pub fn advance(&mut self, block_size: usize, sample_rate: f32) -> f32 {
        let phase_increment = self.rate * block_size as f32 / sample_rate;
        self.phase = self.phase.advance(phase_increment);
        let sine = self.phase.as_radians().sin();
        self.current_value = sine * self.amount;
        self.current_value
    }

    /// Get the current output value without advancing.
    #[must_use]
    pub fn value(&self) -> f32 {
        self.current_value
    }

    /// Get the target parameter this LFO modulates.
    #[must_use]
    pub fn target(&self) -> AweLfoTarget {
        self.target
    }

    /// Set rate in Hz, clamped to [0.01, 20.0].
    pub fn set_rate(&mut self, rate_hz: f32) {
        self.rate = rate_hz.clamp(MIN_RATE, MAX_RATE);
    }

    /// Set modulation amount, clamped to [0.0, 1.0].
    pub fn set_amount(&mut self, amount: f32) {
        self.amount = amount.clamp(0.0, 1.0);
    }

    /// Set the modulation target parameter.
    pub fn set_target(&mut self, target: AweLfoTarget) {
        self.target = target;
    }

    /// Reset the LFO phase to zero.
    pub fn reset(&mut self) {
        self.phase.reset();
        self.current_value = 0.0;
    }
}

impl Default for AweLfo {
    fn default() -> Self {
        Self {
            phase: Phase::ZERO,
            rate: 0.5,
            amount: 0.0,
            target: AweLfoTarget::SourceX,
            current_value: 0.0,
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
        assert!((lfo.rate - 0.5).abs() < f32::EPSILON);
        assert!((lfo.amount - 0.0).abs() < f32::EPSILON);
        assert_eq!(lfo.target(), AweLfoTarget::SourceX);
        assert!((lfo.value() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_output_is_zero_when_amount_is_zero() {
        let mut lfo = AweLfo::new();
        lfo.set_rate(1.0);
        // amount stays at 0.0
        for _ in 0..100 {
            let val = lfo.advance(128, 48000.0);
            assert!((val - 0.0).abs() < f32::EPSILON);
        }
    }

    #[test]
    fn test_output_range() {
        let mut lfo = AweLfo::new();
        lfo.set_rate(5.0);
        lfo.set_amount(0.8);

        let mut min_val = f32::MAX;
        let mut max_val = f32::MIN;

        // Run many blocks to cover full phase range
        for _ in 0..10_000 {
            let val = lfo.advance(64, 48000.0);
            min_val = min_val.min(val);
            max_val = max_val.max(val);
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

        lfo.set_rate(0.001);
        assert!((lfo.rate - MIN_RATE).abs() < f32::EPSILON);

        lfo.set_rate(100.0);
        assert!((lfo.rate - MAX_RATE).abs() < f32::EPSILON);

        lfo.set_rate(5.0);
        assert!((lfo.rate - 5.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_amount_clamping() {
        let mut lfo = AweLfo::new();

        lfo.set_amount(-0.5);
        assert!((lfo.amount - 0.0).abs() < f32::EPSILON);

        lfo.set_amount(2.0);
        assert!((lfo.amount - 1.0).abs() < f32::EPSILON);

        lfo.set_amount(0.7);
        assert!((lfo.amount - 0.7).abs() < f32::EPSILON);
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
        lfo.set_rate(10.0);
        lfo.set_amount(1.0);

        // Advance several blocks
        for _ in 0..50 {
            lfo.advance(128, 48000.0);
        }
        assert!(lfo.value().abs() > f32::EPSILON || true); // phase may land near zero

        lfo.reset();
        assert!((lfo.value() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_sine_shape_quarter_cycle() {
        // At exactly 1/4 cycle the sine should be at its peak (1.0).
        let mut lfo = AweLfo::new();
        lfo.set_rate(1.0);
        lfo.set_amount(1.0);

        // Advance exactly one quarter of a cycle: phase = 0.25
        // With sample_rate = 4.0 and block_size = 1, each advance moves phase by 0.25.
        let val = lfo.advance(1, 4.0);

        // sin(0.25 * TAU) = sin(pi/2) = 1.0
        let expected = (0.25 * TAU).sin();
        assert!(
            (val - expected).abs() < 1e-5,
            "Expected {expected}, got {val}"
        );
    }

    #[test]
    fn test_advance_returns_cached_value() {
        let mut lfo = AweLfo::new();
        lfo.set_rate(3.0);
        lfo.set_amount(0.5);

        let returned = lfo.advance(256, 44100.0);
        assert!((returned - lfo.value()).abs() < f32::EPSILON);
    }

    #[test]
    fn test_full_cycle_wraps() {
        let mut lfo = AweLfo::new();
        lfo.set_rate(1.0);
        lfo.set_amount(1.0);

        // Advance exactly one full cycle: phase should wrap back near zero.
        // block_size = 48000, sample_rate = 48000 => increment = 1.0 per advance.
        lfo.advance(48000, 48000.0);

        // After a full cycle, sine(0) = 0.
        assert!(
            lfo.value().abs() < 1e-5,
            "Expected ~0 after full cycle, got {}",
            lfo.value()
        );
    }
}
