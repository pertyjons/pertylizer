//! Filter DSP primitives.
//!
//! This module provides filter coefficient calculations and
//! filter algorithm implementations for use by filter modules.

use crate::types::{Hertz, SampleRate};

/// SVF filter type enumeration for coefficient calculation.
///
/// Used with `SvfCoeffs::process()` to select output type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvfFilterType {
    #[default]
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
    Peak,
    LowShelf,
    HighShelf,
}

/// State Variable Filter (SVF) coefficients.
///
/// These coefficients implement a 2nd-order SVF that provides
/// simultaneous lowpass, highpass, and bandpass outputs.
#[derive(Debug, Clone, Copy, Default)]
pub struct SvfCoeffs {
    /// Filter gain coefficient
    pub g: f32,
    /// Resonance/damping coefficient
    pub k: f32,
    /// Derived coefficient a1 = 1 / (1 + g * (g + k))
    pub a1: f32,
    /// Derived coefficient a2 = g * a1
    pub a2: f32,
    /// Derived coefficient a3 = g * a2
    pub a3: f32,
}

impl SvfCoeffs {
    /// Calculate SVF coefficients for given cutoff and resonance.
    ///
    /// # Arguments
    ///
    /// * `cutoff` - Cutoff frequency in Hz
    /// * `resonance` - Resonance amount (0.0 to 1.0)
    /// * `sample_rate` - Sample rate in Hz
    #[must_use]
    pub fn new(cutoff: Hertz, resonance: f32, sample_rate: SampleRate) -> Self {
        let g = cutoff.to_tan_coeff(sample_rate);
        let k = 2.0 - 2.0 * resonance.clamp(0.0, 0.99);

        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        Self { g, k, a1, a2, a3 }
    }

    /// Process a single sample through the SVF.
    ///
    /// # Arguments
    ///
    /// * `input` - Input sample
    /// * `ic1eq` - Internal state 1 (mutable)
    /// * `ic2eq` - Internal state 2 (mutable)
    /// * `filter_type` - Desired output type
    ///
    /// # Returns
    ///
    /// The filtered output sample.
    #[inline]
    #[must_use]
    pub fn process(
        &self,
        input: f32,
        ic1eq: &mut f32,
        ic2eq: &mut f32,
        filter_type: SvfFilterType,
    ) -> f32 {
        let v3 = input - *ic2eq;
        let v1 = self.a1 * *ic1eq + self.a2 * v3;
        let v2 = *ic2eq + self.a2 * *ic1eq + self.a3 * v3;

        *ic1eq = 2.0 * v1 - *ic1eq;
        *ic2eq = 2.0 * v2 - *ic2eq;

        match filter_type {
            SvfFilterType::Lowpass => v2,
            SvfFilterType::Highpass => input - self.k * v1 - v2,
            SvfFilterType::Bandpass => v1,
            SvfFilterType::Notch => input - self.k * v1,
            SvfFilterType::Peak => {
                let lp = v2;
                let hp = input - self.k * v1 - v2;
                lp - hp
            }
            SvfFilterType::LowShelf => {
                let lp = v2;
                input * 0.5 + lp * 0.5
            }
            SvfFilterType::HighShelf => {
                let hp = input - self.k * v1 - v2;
                input * 0.5 + hp * 0.5
            }
        }
    }
}

/// Biquad filter coefficients.
///
/// Standard biquad coefficients for various filter types.
/// Uses Direct Form II transposed implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct BiquadCoeffs {
    /// Numerator coefficients
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    /// Denominator coefficients (normalized, a0 = 1)
    pub a1: f32,
    pub a2: f32,
}

impl BiquadCoeffs {
    /// Create lowpass biquad coefficients.
    #[must_use]
    pub fn lowpass(cutoff: Hertz, q: f32, sample_rate: SampleRate) -> Self {
        let omega = std::f32::consts::TAU * cutoff.as_f32() / sample_rate.as_f32();
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * q.max(0.1));

        let b0 = (1.0 - cos_omega) / 2.0;
        let b1 = 1.0 - cos_omega;
        let b2 = (1.0 - cos_omega) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_omega;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Create highpass biquad coefficients.
    #[must_use]
    pub fn highpass(cutoff: Hertz, q: f32, sample_rate: SampleRate) -> Self {
        let omega = std::f32::consts::TAU * cutoff.as_f32() / sample_rate.as_f32();
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * q.max(0.1));

        let b0 = (1.0 + cos_omega) / 2.0;
        let b1 = -(1.0 + cos_omega);
        let b2 = (1.0 + cos_omega) / 2.0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_omega;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Create bandpass biquad coefficients.
    #[must_use]
    pub fn bandpass(center: Hertz, q: f32, sample_rate: SampleRate) -> Self {
        let omega = std::f32::consts::TAU * center.as_f32() / sample_rate.as_f32();
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * q.max(0.1));

        let b0 = alpha;
        let b1 = 0.0;
        let b2 = -alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_omega;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Process a single sample through the biquad filter.
    ///
    /// Uses Direct Form II transposed.
    ///
    /// # Arguments
    ///
    /// * `input` - Input sample
    /// * `z1` - Delay state 1 (mutable)
    /// * `z2` - Delay state 2 (mutable)
    #[inline]
    #[must_use]
    pub fn process(&self, input: f32, z1: &mut f32, z2: &mut f32) -> f32 {
        let output = self.b0 * input + *z1;
        *z1 = self.b1 * input - self.a1 * output + *z2;
        *z2 = self.b2 * input - self.a2 * output;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_svf_coeffs() {
        let coeffs = SvfCoeffs::new(Hertz::new(1000.0), 0.5, SampleRate::DVD_QUALITY);
        assert!(coeffs.g > 0.0);
        assert!(coeffs.a1 > 0.0);
    }

    #[test]
    fn test_svf_process() {
        let coeffs = SvfCoeffs::new(Hertz::new(1000.0), 0.5, SampleRate::DVD_QUALITY);
        let mut ic1eq = 0.0;
        let mut ic2eq = 0.0;

        for _ in 0..100 {
            let output = coeffs.process(0.5, &mut ic1eq, &mut ic2eq, SvfFilterType::Lowpass);
            assert!(output.is_finite());
        }
    }

    #[test]
    fn test_biquad_lowpass() {
        let coeffs = BiquadCoeffs::lowpass(Hertz::new(1000.0), 0.707, SampleRate::DVD_QUALITY);
        let mut z1 = 0.0;
        let mut z2 = 0.0;

        for _ in 0..100 {
            let output = coeffs.process(0.5, &mut z1, &mut z2);
            assert!(output.is_finite());
        }
    }

    #[test]
    fn test_biquad_highpass() {
        let coeffs = BiquadCoeffs::highpass(Hertz::new(1000.0), 0.707, SampleRate::DVD_QUALITY);
        let mut z1 = 0.0;
        let mut z2 = 0.0;

        // Highpass should attenuate DC
        for _ in 0..1000 {
            let output = coeffs.process(1.0, &mut z1, &mut z2);
            assert!(output.is_finite());
        }
        // After settling, DC should be nearly zero
        let final_output = coeffs.process(1.0, &mut z1, &mut z2);
        assert!(final_output.abs() < 0.1);
    }
}
