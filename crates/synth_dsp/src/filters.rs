//! Filter DSP primitives.
//!
//! This module provides filter coefficient calculations and
//! filter algorithm implementations for use by filter modules.

use synth_core::{Hertz, SampleRate};

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

// ============================================================================
// CHARACTER FILTER: FLUID (Oberheim-inspired SVF with morph)
// ============================================================================

/// Oberheim-inspired SVF with continuous LP→BP→HP→Notch morph.
///
/// Features normalized tanh pre-saturation and constant-power crossfade
/// between filter outputs controlled by the morph parameter.
#[derive(Debug, Clone, Copy, Default)]
pub struct FluidFilter {
    pub ic1eq: f32,
    pub ic2eq: f32,
}

impl FluidFilter {
    /// Process a single sample through the Fluid filter.
    ///
    /// # Arguments
    /// * `input` - Input sample
    /// * `coeffs` - Pre-computed SVF coefficients
    /// * `drive` - Drive amount (1.0 = unity)
    /// * `morph` - Morph position (0.0 = LP, 0.33 = BP, 0.67 = HP, 1.0 = Notch)
    #[inline]
    pub fn process(&mut self, input: f32, coeffs: &SvfCoeffs, drive: f32, morph: f32) -> f32 {
        // Pre-filter: normalized tanh saturation
        let drive_clamped = drive.max(0.01);
        let saturated = (input * drive_clamped).tanh() / drive_clamped;

        // SVF computation
        let v3 = saturated - self.ic2eq;
        let v1 = coeffs.a1 * self.ic1eq + coeffs.a2 * v3;
        let v2 = self.ic2eq + coeffs.a2 * self.ic1eq + coeffs.a3 * v3;

        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        // All outputs simultaneously
        let lp = v2;
        let bp = v1;
        let hp = saturated - coeffs.k * v1 - v2;
        let notch = saturated - coeffs.k * v1;

        // Morph: 3 zones (LP→BP, BP→HP, HP→Notch)
        let morph_scaled = morph * 3.0;
        let zone = morph_scaled as u32;
        let t = morph_scaled - zone as f32;
        let half_pi = std::f32::consts::FRAC_PI_2;
        let cos_t = (t * half_pi).cos();
        let sin_t = (t * half_pi).sin();

        match zone {
            0 => lp * cos_t + bp * sin_t,
            1 => bp * cos_t + hp * sin_t,
            _ => hp * cos_t + notch * sin_t,
        }
    }

    /// Flush denormals to zero for consistent performance.
    #[inline]
    pub fn flush_denormals(&mut self) {
        if self.ic1eq.abs() < 1e-15 {
            self.ic1eq = 0.0;
        }
        if self.ic2eq.abs() < 1e-15 {
            self.ic2eq = 0.0;
        }
    }

    /// Reset filter state.
    pub fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }
}

// ============================================================================
// CHARACTER FILTER: SCREAMER (MS-20 Sallen-Key with diode clipping)
// ============================================================================

/// MS-20 inspired Sallen-Key filter with asymmetric diode clipping.
///
/// Features a 2-pole HP→LP cascade with diode-clipped feedback
/// that creates the aggressive, screaming resonance character.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScreamerFilter {
    pub hp_s1: f32,
    pub hp_s2: f32,
    pub lp_s1: f32,
    pub lp_s2: f32,
}

impl ScreamerFilter {
    /// Asymmetric diode clipping function.
    ///
    /// Positive half clips softer (0.8x), negative half clips harder (1.2x),
    /// creating the characteristic MS-20 harmonic asymmetry.
    #[inline]
    fn diode_clip(x: f32) -> f32 {
        if x >= 0.0 {
            1.0 - (-x * 0.8).exp()
        } else {
            -(1.0 - (x * 1.2).exp())
        }
    }

    /// Process a single sample through the Screamer filter.
    ///
    /// # Arguments
    /// * `input` - Input sample
    /// * `g` - Filter coefficient from `Hertz::to_tan_coeff()`
    /// * `resonance` - Resonance amount (0.0 to 1.0)
    /// * `drive` - Drive amount (1.0 = unity)
    #[inline]
    pub fn process(&mut self, input: f32, g: f32, resonance: f32, drive: f32) -> f32 {
        // Diode-clipped feedback
        let feedback = Self::diode_clip(self.lp_s2 * drive) * resonance * 4.0;
        let input_fb = input - feedback;

        // HP section: trapezoidal integrator
        let hp_v = (input_fb - self.hp_s1) * g / (1.0 + g);
        let hp_out = hp_v + self.hp_s1;
        self.hp_s1 = hp_out + hp_v;

        let hp2_v = (hp_out - self.hp_s2) * g / (1.0 + g);
        let hp2_out = hp2_v + self.hp_s2;
        self.hp_s2 = hp2_out + hp2_v;

        // LP section: trapezoidal integrator
        let lp_v = (hp2_out - self.lp_s1) * g / (1.0 + g);
        let lp_out = lp_v + self.lp_s1;
        self.lp_s1 = lp_out + lp_v;

        let lp2_v = (lp_out - self.lp_s2) * g / (1.0 + g);
        let lp2_out = lp2_v + self.lp_s2;
        self.lp_s2 = lp2_out + lp2_v;

        lp2_out
    }

    /// Flush denormals to zero for consistent performance.
    #[inline]
    pub fn flush_denormals(&mut self) {
        if self.hp_s1.abs() < 1e-15 {
            self.hp_s1 = 0.0;
        }
        if self.hp_s2.abs() < 1e-15 {
            self.hp_s2 = 0.0;
        }
        if self.lp_s1.abs() < 1e-15 {
            self.lp_s1 = 0.0;
        }
        if self.lp_s2.abs() < 1e-15 {
            self.lp_s2 = 0.0;
        }
    }

    /// Reset filter state.
    pub fn reset(&mut self) {
        self.hp_s1 = 0.0;
        self.hp_s2 = 0.0;
        self.lp_s1 = 0.0;
        self.lp_s2 = 0.0;
    }
}

// ============================================================================
// CHARACTER FILTER: ACID (Steiner-Parker with variable saturation)
// ============================================================================

/// Steiner-Parker inspired filter with variable saturation.
///
/// Features a resonance-dependent saturation that morphs from tanh
/// to sine-fold as resonance increases, creating the squelchy acid character.
/// Supports LP, BP, and HP modes.
#[derive(Debug, Clone, Copy, Default)]
pub struct AcidFilter {
    pub s1: f32,
    pub s2: f32,
}

impl AcidFilter {
    /// Variable saturation that blends tanh→sine-fold based on resonance.
    #[inline]
    fn variable_saturate(x: f32, resonance: f32) -> f32 {
        let blend = resonance * resonance;
        let tanh_out = x.tanh();
        let fold_out = (x * std::f32::consts::FRAC_PI_2).sin();
        tanh_out * (1.0 - blend) + fold_out * blend
    }

    /// Process a single sample through the Acid filter.
    ///
    /// # Arguments
    /// * `input` - Input sample
    /// * `g` - Filter coefficient from `Hertz::to_tan_coeff()`
    /// * `resonance` - Resonance amount (0.0 to 1.0)
    /// * `drive` - Drive amount (1.0 = unity)
    /// * `filter_mode` - Filter output type (LP, BP, HP supported; others fallback to LP)
    #[inline]
    pub fn process(
        &mut self,
        input: f32,
        g: f32,
        resonance: f32,
        drive: f32,
        filter_mode: SvfFilterType,
    ) -> f32 {
        // Mode-dependent Q scaling
        let q_scale = match filter_mode {
            SvfFilterType::Lowpass => 4.0,
            SvfFilterType::Bandpass => 3.0,
            SvfFilterType::Highpass => 2.5,
            _ => 3.5,
        };

        // Variable-saturated feedback
        let feedback = Self::variable_saturate(self.s2 * drive, resonance) * resonance * q_scale;
        let v0 = input - feedback;

        // ZDF 2-pole
        let v1 = (g * v0 + self.s1) / (1.0 + g);
        let v2 = (g * v1 + self.s2) / (1.0 + g);

        self.s1 = 2.0 * v1 - self.s1;
        self.s2 = 2.0 * v2 - self.s2;

        // Mode output
        match filter_mode {
            SvfFilterType::Lowpass => v2,
            SvfFilterType::Highpass => v0 - v1 - v2,
            SvfFilterType::Bandpass => v1,
            _ => v2, // Fallback to LP for unsupported modes
        }
    }

    /// Flush denormals to zero for consistent performance.
    #[inline]
    pub fn flush_denormals(&mut self) {
        if self.s1.abs() < 1e-15 {
            self.s1 = 0.0;
        }
        if self.s2.abs() < 1e-15 {
            self.s2 = 0.0;
        }
    }

    /// Reset filter state.
    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
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
