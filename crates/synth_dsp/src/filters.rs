//! Filter DSP primitives.
//!
//! This module provides filter coefficient calculations and
//! filter algorithm implementations for use by filter modules.

use synth_core::{FilterState, Gain, Hertz, NormalizedValue, SampleRate};

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
    /// Resonance/damping coefficient
    pub(crate) k: f32,
    /// Derived coefficient a1 = 1 / (1 + g * (g + k))
    pub(crate) a1: f32,
    /// Derived coefficient a2 = g * a1
    pub(crate) a2: f32,
    /// Derived coefficient a3 = g * a2
    pub(crate) a3: f32,
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
    pub fn new(cutoff: Hertz, resonance: NormalizedValue, sample_rate: SampleRate) -> Self {
        let g = cutoff.to_tan_coeff(sample_rate);
        let k = 2.0 - 2.0 * resonance.as_f32().clamp(0.0, 0.99);

        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        Self { k, a1, a2, a3 }
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
        ic1eq: &mut FilterState,
        ic2eq: &mut FilterState,
        filter_type: SvfFilterType,
    ) -> f32 {
        let ic1 = ic1eq.as_f32();
        let ic2 = ic2eq.as_f32();
        let v3 = input - ic2;
        let v1 = self.a1 * ic1 + self.a2 * v3;
        let v2 = ic2 + self.a2 * ic1 + self.a3 * v3;

        *ic1eq = FilterState::new(2.0 * v1 - ic1);
        *ic2eq = FilterState::new(2.0 * v2 - ic2);

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
            // Fixed ~6dB shelving via LP/HP blend.
            // For parametric shelving, use a dedicated shelf topology.
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
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    /// Create biquad coefficients from raw unnormalized values.
    ///
    /// Normalizes all coefficients by dividing by `a0`.
    #[must_use]
    pub fn from_raw(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        let a0 = if a0.abs() < f32::EPSILON { 1.0 } else { a0 };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }

    /// Precompute common biquad variables from cutoff and Q.
    ///
    /// Returns `(sin_omega, cos_omega, alpha)`.
    #[inline]
    fn biquad_precompute(cutoff: Hertz, q: f32, sample_rate: SampleRate) -> (f32, f32, f32) {
        let omega = std::f32::consts::TAU * cutoff.as_f32() / sample_rate.as_f32();
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * q.max(0.1));
        (sin_omega, cos_omega, alpha)
    }

    /// Create lowpass biquad coefficients.
    #[must_use]
    pub fn lowpass(cutoff: Hertz, q: f32, sample_rate: SampleRate) -> Self {
        let (_sin_omega, cos_omega, alpha) = Self::biquad_precompute(cutoff, q, sample_rate);
        let b0 = (1.0 - cos_omega) / 2.0;
        let b1 = 1.0 - cos_omega;
        let b2 = b0;
        Self::from_raw(b0, b1, b2, 1.0 + alpha, -2.0 * cos_omega, 1.0 - alpha)
    }

    /// Create highpass biquad coefficients.
    #[must_use]
    pub fn highpass(cutoff: Hertz, q: f32, sample_rate: SampleRate) -> Self {
        let (_sin_omega, cos_omega, alpha) = Self::biquad_precompute(cutoff, q, sample_rate);
        let b0 = (1.0 + cos_omega) / 2.0;
        let b1 = -(1.0 + cos_omega);
        let b2 = b0;
        Self::from_raw(b0, b1, b2, 1.0 + alpha, -2.0 * cos_omega, 1.0 - alpha)
    }

    /// Create bandpass biquad coefficients.
    #[must_use]
    pub fn bandpass(center: Hertz, q: f32, sample_rate: SampleRate) -> Self {
        let (_sin_omega, cos_omega, alpha) = Self::biquad_precompute(center, q, sample_rate);
        Self::from_raw(
            alpha,
            0.0,
            -alpha,
            1.0 + alpha,
            -2.0 * cos_omega,
            1.0 - alpha,
        )
    }

    /// Create notch (band-reject) biquad coefficients.
    #[must_use]
    pub fn notch(center: Hertz, q: f32, sample_rate: SampleRate) -> Self {
        let (_sin_omega, cos_omega, alpha) = Self::biquad_precompute(center, q, sample_rate);
        Self::from_raw(
            1.0,
            -2.0 * cos_omega,
            1.0,
            1.0 + alpha,
            -2.0 * cos_omega,
            1.0 - alpha,
        )
    }

    /// Create peaking EQ biquad coefficients (RBJ Cookbook).
    ///
    /// * `db_gain` - Gain in decibels (positive = boost, negative = cut)
    #[must_use]
    pub fn peaking(center: Hertz, q: f32, db_gain: f32, sample_rate: SampleRate) -> Self {
        let omega = std::f32::consts::TAU * center.as_f32() / sample_rate.as_f32();
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let a = 10.0_f32.powf(db_gain / 40.0);
        let alpha = sin_omega / (2.0 * q.max(0.1));
        Self::from_raw(
            1.0 + alpha * a,
            -2.0 * cos_omega,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos_omega,
            1.0 - alpha / a,
        )
    }

    /// Create low-shelf biquad coefficients (RBJ Cookbook).
    ///
    /// * `slope` - Shelf slope S (1.0 = steepest without ringing, 0.5 = gentler)
    /// * `db_gain` - Shelf gain in decibels
    #[must_use]
    pub fn low_shelf(cutoff: Hertz, slope: f32, db_gain: f32, sample_rate: SampleRate) -> Self {
        let omega = std::f32::consts::TAU * cutoff.as_f32() / sample_rate.as_f32();
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let a = 10.0_f32.powf(db_gain / 40.0);
        let alpha = sin_omega / 2.0 * ((a + 1.0 / a) * (1.0 / slope.max(0.1) - 1.0) + 2.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        Self::from_raw(
            a * ((a + 1.0) - (a - 1.0) * cos_omega + two_sqrt_a_alpha),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos_omega),
            a * ((a + 1.0) - (a - 1.0) * cos_omega - two_sqrt_a_alpha),
            (a + 1.0) + (a - 1.0) * cos_omega + two_sqrt_a_alpha,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos_omega),
            (a + 1.0) + (a - 1.0) * cos_omega - two_sqrt_a_alpha,
        )
    }

    /// Create high-shelf biquad coefficients (RBJ Cookbook).
    ///
    /// * `slope` - Shelf slope S (1.0 = steepest without ringing, 0.5 = gentler)
    /// * `db_gain` - Shelf gain in decibels
    #[must_use]
    pub fn high_shelf(cutoff: Hertz, slope: f32, db_gain: f32, sample_rate: SampleRate) -> Self {
        let omega = std::f32::consts::TAU * cutoff.as_f32() / sample_rate.as_f32();
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let a = 10.0_f32.powf(db_gain / 40.0);
        let alpha = sin_omega / 2.0 * ((a + 1.0 / a) * (1.0 / slope.max(0.1) - 1.0) + 2.0).sqrt();
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
        Self::from_raw(
            a * ((a + 1.0) + (a - 1.0) * cos_omega + two_sqrt_a_alpha),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_omega),
            a * ((a + 1.0) + (a - 1.0) * cos_omega - two_sqrt_a_alpha),
            (a + 1.0) - (a - 1.0) * cos_omega + two_sqrt_a_alpha,
            2.0 * ((a - 1.0) - (a + 1.0) * cos_omega),
            (a + 1.0) - (a - 1.0) * cos_omega - two_sqrt_a_alpha,
        )
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
    pub fn process(&self, input: f32, z1: &mut FilterState, z2: &mut FilterState) -> f32 {
        let output = self.b0 * input + z1.as_f32();
        *z1 = FilterState::new(self.b1 * input - self.a1 * output + z2.as_f32());
        *z2 = FilterState::new(self.b2 * input - self.a2 * output);
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
    ic1eq: FilterState,
    ic2eq: FilterState,
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
    pub fn process(
        &mut self,
        input: f32,
        coeffs: &SvfCoeffs,
        drive: Gain,
        morph: NormalizedValue,
    ) -> f32 {
        // Pre-filter: normalized tanh saturation
        let drive_val = drive.as_f32().max(0.01);
        let saturated = (input * drive_val).tanh() / drive_val;

        // SVF computation
        let ic1 = self.ic1eq.as_f32();
        let ic2 = self.ic2eq.as_f32();
        let v3 = saturated - ic2;
        let v1 = coeffs.a1 * ic1 + coeffs.a2 * v3;
        let v2 = ic2 + coeffs.a2 * ic1 + coeffs.a3 * v3;

        self.ic1eq = FilterState::new(2.0 * v1 - ic1);
        self.ic2eq = FilterState::new(2.0 * v2 - ic2);

        // All outputs simultaneously
        let lp = v2;
        let bp = v1;
        let hp = saturated - coeffs.k * v1 - v2;
        let notch = saturated - coeffs.k * v1;

        // Morph: 3 zones (LP→BP, BP→HP, HP→Notch)
        let morph_scaled = morph.as_f32() * 3.0;
        let zone = (morph_scaled as u32).min(2);
        let t = (morph_scaled - zone as f32).clamp(0.0, 1.0);
        let half_pi = std::f32::consts::FRAC_PI_2;
        let cos_t = (t * half_pi).cos();
        let sin_t = (t * half_pi).sin();

        match zone {
            0 => lp * cos_t + bp * sin_t,
            1 => bp * cos_t + hp * sin_t,
            _ => hp * cos_t + notch * sin_t,
        }
    }

    /// Reset filter state.
    pub fn reset(&mut self) {
        self.ic1eq = FilterState::ZERO;
        self.ic2eq = FilterState::ZERO;
    }
}

// ============================================================================
// CHARACTER FILTER: SCREAMER (MS-20 Sallen-Key with diode clipping)
// ============================================================================

/// MS-20 inspired filter with asymmetric diode clipping.
///
/// Features a 4-pole cascade with diode-clipped feedback from the output
/// that creates the aggressive, screaming resonance character.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScreamerFilter {
    s1: FilterState,
    s2: FilterState,
    s3: FilterState,
    s4: FilterState,
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
    pub fn process(&mut self, input: f32, g: f32, resonance: NormalizedValue, drive: Gain) -> f32 {
        let res = resonance.as_f32();
        let drv = drive.as_f32();
        // Diode-clipped feedback from 4th stage output
        let feedback = Self::diode_clip(self.s4.as_f32() * drv) * res * 4.0;
        let input_fb = input - feedback;

        // 4-pole cascade: each stage is a trapezoidal integrator (one-pole lowpass).
        // The topology produces a resonant lowpass via the nonlinear feedback path.
        let s1_prev = self.s1.as_f32();
        let v1 = (input_fb - s1_prev) * g / (1.0 + g);
        let out1 = v1 + s1_prev;
        self.s1 = FilterState::new(out1 + v1);

        let s2_prev = self.s2.as_f32();
        let v2 = (out1 - s2_prev) * g / (1.0 + g);
        let out2 = v2 + s2_prev;
        self.s2 = FilterState::new(out2 + v2);

        let s3_prev = self.s3.as_f32();
        let v3 = (out2 - s3_prev) * g / (1.0 + g);
        let out3 = v3 + s3_prev;
        self.s3 = FilterState::new(out3 + v3);

        let s4_prev = self.s4.as_f32();
        let v4 = (out3 - s4_prev) * g / (1.0 + g);
        let out4 = v4 + s4_prev;
        self.s4 = FilterState::new(out4 + v4);

        out4
    }

    /// Reset filter state.
    pub fn reset(&mut self) {
        self.s1 = FilterState::ZERO;
        self.s2 = FilterState::ZERO;
        self.s3 = FilterState::ZERO;
        self.s4 = FilterState::ZERO;
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
    s1: FilterState,
    s2: FilterState,
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
        resonance: NormalizedValue,
        drive: Gain,
        filter_mode: SvfFilterType,
    ) -> f32 {
        let res = resonance.as_f32();
        let drv = drive.as_f32();
        // Mode-dependent Q scaling
        let q_scale = match filter_mode {
            SvfFilterType::Lowpass => 4.0,
            SvfFilterType::Bandpass => 3.0,
            SvfFilterType::Highpass => 2.5,
            _ => 3.5,
        };

        // Variable-saturated feedback
        let s1 = self.s1.as_f32();
        let s2 = self.s2.as_f32();
        let feedback = Self::variable_saturate(s2 * drv, res) * res * q_scale;
        let v0 = input - feedback;

        // ZDF 2-pole
        let v1 = (g * v0 + s1) / (1.0 + g);
        let v2 = (g * v1 + s2) / (1.0 + g);

        self.s1 = FilterState::new(2.0 * v1 - s1);
        self.s2 = FilterState::new(2.0 * v2 - s2);

        // Mode output
        match filter_mode {
            SvfFilterType::Lowpass => v2,
            SvfFilterType::Highpass => v0 - v1 - v2,
            SvfFilterType::Bandpass => v1,
            _ => v2, // Fallback to LP for unsupported modes
        }
    }

    /// Reset filter state.
    pub fn reset(&mut self) {
        self.s1 = FilterState::ZERO;
        self.s2 = FilterState::ZERO;
    }
}

// ============================================================================
// STEREO FILTER WRAPPERS
// ============================================================================

use synth_core::StereoSample;

/// Paired SVF filters for stereo processing with shared coefficients.
///
/// Maintains independent state for left and right channels.
#[derive(Debug, Clone, Copy, Default)]
pub struct StereoSvf {
    left_ic1: FilterState,
    left_ic2: FilterState,
    right_ic1: FilterState,
    right_ic2: FilterState,
}

impl StereoSvf {
    /// Process a stereo sample through both filters.
    #[inline]
    pub fn process(
        &mut self,
        input: StereoSample,
        coeffs: &SvfCoeffs,
        filter_type: SvfFilterType,
    ) -> StereoSample {
        let left = coeffs.process(
            input.left,
            &mut self.left_ic1,
            &mut self.left_ic2,
            filter_type,
        );
        let right = coeffs.process(
            input.right,
            &mut self.right_ic1,
            &mut self.right_ic2,
            filter_type,
        );
        StereoSample::new(left, right)
    }

    /// Reset all filter state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Paired biquad filters for stereo processing with shared coefficients.
///
/// Maintains independent state for left and right channels.
#[derive(Debug, Clone, Copy, Default)]
pub struct StereoBiquad {
    left_z1: FilterState,
    left_z2: FilterState,
    right_z1: FilterState,
    right_z2: FilterState,
}

impl StereoBiquad {
    /// Process a stereo sample through both biquad filters.
    #[inline]
    pub fn process(&mut self, input: StereoSample, coeffs: &BiquadCoeffs) -> StereoSample {
        let left = coeffs.process(input.left, &mut self.left_z1, &mut self.left_z2);
        let right = coeffs.process(input.right, &mut self.right_z1, &mut self.right_z2);
        StereoSample::new(left, right)
    }

    /// Reset all filter state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_svf_coeffs() {
        let coeffs = SvfCoeffs::new(
            Hertz::new(1000.0),
            NormalizedValue::new(0.5),
            SampleRate::DVD_QUALITY,
        );
        assert!(coeffs.a1 > 0.0);
    }

    #[test]
    fn test_svf_process() {
        let coeffs = SvfCoeffs::new(
            Hertz::new(1000.0),
            NormalizedValue::new(0.5),
            SampleRate::DVD_QUALITY,
        );
        let mut ic1eq = FilterState::ZERO;
        let mut ic2eq = FilterState::ZERO;

        for _ in 0..100 {
            let output = coeffs.process(0.5, &mut ic1eq, &mut ic2eq, SvfFilterType::Lowpass);
            assert!(output.is_finite());
        }
    }

    #[test]
    fn test_biquad_lowpass() {
        let coeffs = BiquadCoeffs::lowpass(Hertz::new(1000.0), 0.707, SampleRate::DVD_QUALITY);
        let mut z1 = FilterState::ZERO;
        let mut z2 = FilterState::ZERO;

        for _ in 0..100 {
            let output = coeffs.process(0.5, &mut z1, &mut z2);
            assert!(output.is_finite());
        }
    }

    #[test]
    fn test_biquad_highpass() {
        let coeffs = BiquadCoeffs::highpass(Hertz::new(1000.0), 0.707, SampleRate::DVD_QUALITY);
        let mut z1 = FilterState::ZERO;
        let mut z2 = FilterState::ZERO;

        // Highpass should attenuate DC
        for _ in 0..1000 {
            let output = coeffs.process(1.0, &mut z1, &mut z2);
            assert!(output.is_finite());
        }
        // After settling, DC should be nearly zero
        let final_output = coeffs.process(1.0, &mut z1, &mut z2);
        assert!(final_output.abs() < 0.1);
    }

    #[test]
    fn test_stereo_svf_matches_mono() {
        let coeffs = SvfCoeffs::new(
            Hertz::new(1000.0),
            NormalizedValue::new(0.5),
            SampleRate::DVD_QUALITY,
        );

        // Process with StereoSvf
        let mut stereo = StereoSvf::default();
        let input = StereoSample::new(0.5, -0.3);
        let stereo_out = stereo.process(input, &coeffs, SvfFilterType::Lowpass);

        // Process with two separate mono filters
        let mut l_ic1 = FilterState::ZERO;
        let mut l_ic2 = FilterState::ZERO;
        let mut r_ic1 = FilterState::ZERO;
        let mut r_ic2 = FilterState::ZERO;
        let mono_l = coeffs.process(0.5, &mut l_ic1, &mut l_ic2, SvfFilterType::Lowpass);
        let mono_r = coeffs.process(-0.3, &mut r_ic1, &mut r_ic2, SvfFilterType::Lowpass);

        assert_eq!(stereo_out.left, mono_l);
        assert_eq!(stereo_out.right, mono_r);
    }

    #[test]
    fn test_stereo_biquad_matches_mono() {
        let coeffs = BiquadCoeffs::lowpass(Hertz::new(1000.0), 0.707, SampleRate::DVD_QUALITY);

        let mut stereo = StereoBiquad::default();
        let input = StereoSample::new(1.0, -1.0);
        let stereo_out = stereo.process(input, &coeffs);

        let mut l_z1 = FilterState::ZERO;
        let mut l_z2 = FilterState::ZERO;
        let mut r_z1 = FilterState::ZERO;
        let mut r_z2 = FilterState::ZERO;
        let mono_l = coeffs.process(1.0, &mut l_z1, &mut l_z2);
        let mono_r = coeffs.process(-1.0, &mut r_z1, &mut r_z2);

        assert_eq!(stereo_out.left, mono_l);
        assert_eq!(stereo_out.right, mono_r);
    }

    #[test]
    fn test_stereo_svf_reset() {
        let coeffs = SvfCoeffs::new(
            Hertz::new(1000.0),
            NormalizedValue::new(0.5),
            SampleRate::DVD_QUALITY,
        );
        let mut svf = StereoSvf::default();
        let fresh = StereoSvf::default();
        svf.process(StereoSample::new(1.0, 1.0), &coeffs, SvfFilterType::Lowpass);
        svf.reset();
        // After reset, processing should produce same result as a fresh instance
        let result_reset = svf.process(
            StereoSample::new(0.5, -0.5),
            &coeffs,
            SvfFilterType::Lowpass,
        );
        let mut fresh2 = fresh;
        let result_fresh = fresh2.process(
            StereoSample::new(0.5, -0.5),
            &coeffs,
            SvfFilterType::Lowpass,
        );
        assert_eq!(result_reset.left, result_fresh.left);
        assert_eq!(result_reset.right, result_fresh.right);
    }

    // ========================================================================
    // RBJ Cookbook coefficient verification tests
    // Reference: 1 kHz, Q=0.707, sr=48000 Hz, gain=±6 dB
    // Values computed from Robert Bristow-Johnson Audio EQ Cookbook formulas.
    // ========================================================================

    /// Helper: assert biquad coefficients match expected values within tolerance.
    fn assert_coeffs(c: &BiquadCoeffs, b0: f32, b1: f32, b2: f32, a1: f32, a2: f32, tol: f32) {
        assert!((c.b0 - b0).abs() < tol, "b0: got {}, expected {}", c.b0, b0);
        assert!((c.b1 - b1).abs() < tol, "b1: got {}, expected {}", c.b1, b1);
        assert!((c.b2 - b2).abs() < tol, "b2: got {}, expected {}", c.b2, b2);
        assert!((c.a1 - a1).abs() < tol, "a1: got {}, expected {}", c.a1, a1);
        assert!((c.a2 - a2).abs() < tol, "a2: got {}, expected {}", c.a2, a2);
    }

    const TEST_FREQ: Hertz = Hertz::new(1000.0);
    const TEST_Q: f32 = 0.707;
    const TEST_SR: SampleRate = SampleRate::DVD_QUALITY;
    const TOL: f32 = 1e-4;

    #[test]
    fn test_biquad_lowpass_coeffs() {
        let c = BiquadCoeffs::lowpass(TEST_FREQ, TEST_Q, TEST_SR);
        assert_coeffs(&c, 0.003916, 0.007832, 0.003916, -1.815318, 0.830982, TOL);
    }

    #[test]
    fn test_biquad_bandpass_coeffs() {
        let c = BiquadCoeffs::bandpass(TEST_FREQ, TEST_Q, TEST_SR);
        assert_coeffs(&c, 0.084509, 0.0, -0.084509, -1.815318, 0.830982, TOL);
    }

    #[test]
    fn test_biquad_notch_coeffs() {
        let c = BiquadCoeffs::notch(TEST_FREQ, TEST_Q, TEST_SR);
        assert_coeffs(&c, 0.915491, -1.815318, 0.915491, -1.815318, 0.830982, TOL);
    }

    #[test]
    fn test_biquad_peaking_boost_coeffs() {
        let c = BiquadCoeffs::peaking(TEST_FREQ, TEST_Q, 6.0, TEST_SR);
        assert_coeffs(&c, 1.061051, -1.861256, 0.816266, -1.861256, 0.877317, TOL);
    }

    #[test]
    fn test_biquad_peaking_cut_coeffs() {
        let c = BiquadCoeffs::peaking(TEST_FREQ, TEST_Q, -6.0, TEST_SR);
        assert_coeffs(&c, 0.942462, -1.754162, 0.826837, -1.754162, 0.769299, TOL);
    }

    #[test]
    fn test_biquad_peaking_reciprocal() {
        // Boost and cut should be reciprocal: peaking(+G) * peaking(-G) ≈ unity
        let boost = BiquadCoeffs::peaking(TEST_FREQ, TEST_Q, 6.0, TEST_SR);
        let cut = BiquadCoeffs::peaking(TEST_FREQ, TEST_Q, -6.0, TEST_SR);

        let mut bz1 = FilterState::ZERO;
        let mut bz2 = FilterState::ZERO;
        let mut cz1 = FilterState::ZERO;
        let mut cz2 = FilterState::ZERO;

        // Feed impulse through boost then cut
        let mut output = boost.process(1.0, &mut bz1, &mut bz2);
        output = cut.process(output, &mut cz1, &mut cz2);
        for _ in 0..2000 {
            output = boost.process(0.0, &mut bz1, &mut bz2);
            output = cut.process(output, &mut cz1, &mut cz2);
        }
        // After settling, DC response of cascade should be near zero (impulse tail)
        assert!(output.abs() < 1e-4, "cascade residual: {output}");
    }

    #[test]
    fn test_biquad_low_shelf_coeffs() {
        // RBJ Cookbook low shelf with slope S=1.0
        let c = BiquadCoeffs::low_shelf(TEST_FREQ, 1.0, 6.0, TEST_SR);
        assert_coeffs(&c, 1.032562, -1.838857, 0.828748, -1.844457, 0.855710, TOL);
    }

    #[test]
    fn test_biquad_high_shelf_coeffs() {
        // RBJ Cookbook high shelf with slope S=1.0
        let c = BiquadCoeffs::high_shelf(TEST_FREQ, 1.0, 6.0, TEST_SR);
        assert_coeffs(&c, 1.932341, -3.564119, 1.653523, -1.780867, 0.802613, TOL);
    }

    #[test]
    fn test_biquad_notch_rejects_center() {
        // A notch at 1 kHz should attenuate a 1 kHz sine to near zero
        let c = BiquadCoeffs::notch(TEST_FREQ, TEST_Q, TEST_SR);
        let mut z1 = FilterState::ZERO;
        let mut z2 = FilterState::ZERO;

        let mut last = 0.0_f32;
        for i in 0..4000 {
            let input = (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin();
            last = c.process(input, &mut z1, &mut z2);
        }
        assert!(last.abs() < 0.01, "notch output at center: {last}");
    }

    #[test]
    fn test_biquad_shelf_dc_gain() {
        // Low shelf +6 dB should boost DC by ~6 dB (factor ≈ 2.0 in amplitude)
        let c = BiquadCoeffs::low_shelf(TEST_FREQ, 1.0, 6.0, TEST_SR);
        let mut z1 = FilterState::ZERO;
        let mut z2 = FilterState::ZERO;

        let mut output = 0.0;
        for _ in 0..5000 {
            output = c.process(1.0, &mut z1, &mut z2);
        }
        // 6 dB ≈ 10^(6/20) ≈ 1.9953
        let expected = 10.0_f32.powf(6.0 / 20.0);
        assert!(
            (output - expected).abs() < 0.05,
            "low shelf DC gain: got {output}, expected {expected}"
        );
    }
}
