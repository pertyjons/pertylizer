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
