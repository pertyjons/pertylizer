//! Band-limited oscillator primitives.
//!
//! This module provides PolyBLEP (Polynomial Band-Limited Step) functions
//! for generating alias-free waveforms.
//!
//! ## PolyBLEP
//!
//! PolyBLEP is a technique for reducing aliasing in digital waveforms by
//! applying a polynomial correction near discontinuities. It's more efficient
//! than traditional band-limiting methods while providing good results.
//!
//! ## Example
//!
//! ```ignore
//! use pertylizer::dsp::oscillators::poly_blep;
//!
//! let phase = 0.01; // Near the discontinuity
//! let dt = 0.01;    // Phase increment per sample
//! let correction = poly_blep(phase, dt);
//! let saw = 2.0 * phase - 1.0 - correction;
//! ```

use synth_core::{NormalizedValue, Phase};

/// PolyBLEP correction for band-limited waveforms.
///
/// This function calculates the polynomial correction to apply near
/// discontinuities in waveforms (like the reset of a sawtooth or
/// the edges of a square wave).
///
/// # Arguments
///
/// * `t` - Current phase position (0.0 to 1.0)
/// * `dt` - Phase increment per sample (frequency / sample_rate)
///
/// # Returns
///
/// A correction value to subtract from/add to the naive waveform.
#[inline]
#[must_use]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    let dt = dt.max(f32::EPSILON);
    if t < dt {
        // Rising edge at start of period
        let t = t / dt;
        2.0 * t - t * t - 1.0
    } else if t > 1.0 - dt {
        // Falling edge at end of period
        let t = (t - 1.0) / dt;
        t * t + 2.0 * t + 1.0
    } else {
        0.0
    }
}

/// Integrated PolyBLEP for smoother waveforms.
///
/// This variant provides additional smoothing for waveforms
/// that need extra alias reduction.
///
/// # Arguments
///
/// * `t` - Current phase position (0.0 to 1.0)
/// * `dt` - Phase increment per sample
///
/// # Returns
///
/// An integrated correction value for smoother waveforms.
#[inline]
#[must_use]
pub fn poly_blep_integrated(t: f32, dt: f32) -> f32 {
    let dt = dt.max(f32::EPSILON);
    if t < dt {
        let t = t / dt;
        let t2 = t * t;
        t2 * t - t2 - t + 1.0 / 3.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        let t2 = t * t;
        t2 * t + t2 + t + 1.0 / 3.0
    } else {
        0.0
    }
}

/// PolyBLAMP (Polynomial Band-Limited rAMP) for triangle waves.
///
/// Used to smooth the corners of triangle waves where the derivative
/// has a discontinuity. This is the integrated form of PolyBLEP.
///
/// # Arguments
///
/// * `t` - Distance from corner point (can be negative)
/// * `dt` - Phase increment per sample
///
/// # Returns
///
/// A correction value to add to the triangle wave.
#[inline]
#[must_use]
pub fn poly_blamp(t: f32, dt: f32) -> f32 {
    let dt = dt.max(f32::EPSILON);
    if t < dt && t > -dt {
        // Within the transition region
        let t_norm = t / dt;
        if t_norm < 0.0 {
            // Before the corner
            let x = t_norm + 1.0;
            -dt * x * x * x / 6.0
        } else {
            // After the corner
            let x = t_norm - 1.0;
            dt * x * x * x / 6.0
        }
    } else {
        0.0
    }
}

/// Discrete Summation Formula (DSF) for band-limited sawtooth generation.
///
/// Sums harmonics with geometric rolloff using direct evaluation.
/// `phase` is 0.0-1.0, `num_harmonics` limits harmonics below Nyquist.
/// `rolloff` (0.0-0.999) controls per-harmonic attenuation.
///
/// Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/140-dsf-super-set-of-blit.rst>
/// From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)
#[inline]
#[must_use]
pub fn dsf_sawtooth(phase: Phase, num_harmonics: u32, rolloff: NormalizedValue) -> f32 {
    if num_harmonics == 0 {
        return 0.0;
    }
    let a = rolloff.as_f32().clamp(0.001, 0.999);
    let omega = phase.as_f32() * std::f32::consts::TAU;

    // Direct summation for small harmonic counts (RT-safe, bounded loop)
    let mut sum = 0.0_f32;
    let mut amp = 1.0_f32;
    let n = num_harmonics.min(64); // cap at 64 for RT safety
    for k in 1..=n {
        sum += amp * (k as f32 * omega).sin() / k as f32;
        amp *= a;
    }

    // Normalize to approximately [-1, 1]
    sum * (2.0 / std::f32::consts::PI)
}

/// Pre-computed MinBLEP (minimum-phase band-limited step) table.
///
/// Used for alias-free discontinuity insertion in waveform generation.
/// Higher quality than PolyBLEP at cost of table memory.
///
/// Algorithm source: <https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/112-waveform-generator-using-minbleps.rst>
/// From the Music-DSP Source Code Archive (<https://www.musicdsp.org/>)
pub struct MinBlepTable {
    /// Integrated MinBLEP samples.
    table: Vec<f32>,
    /// Oversampling factor used to generate the table.
    oversampling: usize,
}

impl MinBlepTable {
    /// Generate a MinBLEP table with given zero crossings and oversampling.
    ///
    /// `zero_crossings`: number of sinc lobes (4 is typical).
    /// `oversampling`: samples per zero crossing (64 typical).
    #[must_use]
    pub fn new(zero_crossings: usize, oversampling: usize) -> Self {
        let n = 2 * zero_crossings * oversampling;
        let mut sinc_windowed = vec![0.0_f32; n + 1];

        // Generate windowed sinc
        for i in 0..=n {
            let x = (i as f32 / oversampling as f32) - zero_crossings as f32;
            // Sinc function
            let sinc = if x.abs() < 1e-10 {
                1.0
            } else {
                let px = x * std::f32::consts::PI;
                px.sin() / px
            };
            // Blackman window
            let t = i as f32 / n as f32;
            let window = 0.42 - 0.5 * (std::f32::consts::TAU * t).cos()
                + 0.08 * (2.0 * std::f32::consts::TAU * t).cos();
            sinc_windowed[i] = sinc * window;
        }

        // Integrate (cumulative sum) to get the MinBLEP
        let mut table = vec![0.0_f32; n + 1];
        let mut sum = 0.0;
        for i in 0..=n {
            sum += sinc_windowed[i];
            table[i] = sum;
        }

        // Normalize so table ends at 1.0
        let last = table[n];
        if last.abs() > 1e-10 {
            for v in &mut table {
                *v /= last;
            }
        }

        Self {
            table,
            oversampling,
        }
    }

    /// Look up a MinBLEP correction value.
    /// `fractional_pos` is 0.0 to 1.0 across the entire table.
    #[inline]
    #[must_use]
    pub fn lookup(&self, fractional_pos: f32) -> f32 {
        let pos = fractional_pos * (self.table.len() - 1) as f32;
        let idx = pos as usize;
        let frac = pos - idx as f32;
        if idx + 1 < self.table.len() {
            self.table[idx] * (1.0 - frac) + self.table[idx + 1] * frac
        } else {
            self.table.last().copied().unwrap_or(1.0)
        }
    }

    /// Get the length of the table in output samples.
    #[inline]
    #[must_use]
    pub fn length_samples(&self) -> usize {
        self.table.len() / self.oversampling
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_poly_blep_at_discontinuity() {
        // Near the start of period
        let correction = poly_blep(0.005, 0.01);
        assert!(correction.abs() > 0.0);

        // In the middle - no correction needed
        let correction = poly_blep(0.5, 0.01);
        assert!((correction).abs() < 0.001);

        // Near the end of period
        let correction = poly_blep(0.995, 0.01);
        assert!(correction.abs() > 0.0);
    }

    #[test]
    fn test_poly_blep_integrated() {
        let correction = poly_blep_integrated(0.005, 0.01);
        assert!(correction.is_finite());
    }

    #[test]
    fn test_band_limited_saw() {
        // Generate a naive saw and apply PolyBLEP
        let dt = 0.01;
        let mut max_correction = 0.0f32;

        for i in 0..100 {
            let phase = i as f32 / 100.0;
            let correction = poly_blep(phase, dt);
            max_correction = max_correction.max(correction.abs());
        }

        // Correction should be bounded
        assert!(max_correction < 2.0);
    }
}
