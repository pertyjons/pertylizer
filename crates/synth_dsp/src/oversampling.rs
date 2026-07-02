//! Oversampling infrastructure for anti-aliased synthesis.
//!
//! Provides a [`Downsampler`] with half-band FIR filtering for 2x and 4x
//! decimation, and an [`OversamplingFactor`] enum for per-instrument configuration.
//!
//! ## Usage
//!
//! 1. Process voices at `sample_rate * factor` with `sample_count * factor` samples
//! 2. Downsample the result back to the original rate with anti-aliasing
//!
//! The downsampler uses an 11-tap half-band FIR filter (~60dB stopband rejection).
//! For 4x decimation, two stages are cascaded (4x → 2x → 1x).

/// Oversampling factor for per-instrument audio quality.
///
/// Higher factors reduce aliasing artifacts from oscillators and waveshapers
/// at the cost of increased CPU usage (roughly proportional to the factor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OversamplingFactor {
    /// No oversampling (1x) — default, zero overhead.
    #[default]
    X1,
    /// 2x oversampling — reduces aliasing, moderate CPU cost.
    X2,
    /// 4x oversampling — highest quality, highest CPU cost.
    X4,
}

impl OversamplingFactor {
    /// All available oversampling factors.
    pub const ALL: [Self; 3] = [Self::X1, Self::X2, Self::X4];

    /// Get the numeric multiplication factor.
    #[must_use]
    pub const fn factor(self) -> usize {
        match self {
            Self::X1 => 1,
            Self::X2 => 2,
            Self::X4 => 4,
        }
    }

    /// Human-readable name for UI display.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::X1 => "Off",
            Self::X2 => "2x",
            Self::X4 => "4x",
        }
    }

    /// Get factor from array index (0=X1, 1=X2, 2=X4).
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::X1)
    }

    /// Get the array index for this factor.
    #[must_use]
    pub const fn to_index(self) -> usize {
        match self {
            Self::X1 => 0,
            Self::X2 => 1,
            Self::X4 => 2,
        }
    }
}

// ============================================================================
// HALF-BAND FIR FILTER
// ============================================================================

/// 11-tap half-band FIR filter coefficients for 2x decimation.
///
/// Parks-McClellan equiripple design with ~60dB stopband rejection.
/// Symmetric: h[k] = h[10-k]. Even-indexed coefficients (except center) are zero.
const HALFBAND_COEFFS: [f32; 11] = [
    0.013_18, 0.0, -0.063_49, 0.0, 0.300_31, 0.5, 0.300_31, 0.0, -0.063_49, 0.0, 0.013_18,
];

const HALFBAND_LEN: usize = HALFBAND_COEFFS.len();

/// Single-stage half-band FIR filter for 2x decimation.
///
/// Maintains internal state (delay line) between calls for continuous processing.
#[derive(Clone)]
struct HalfBandFilter {
    delay: [f32; HALFBAND_LEN],
}

impl HalfBandFilter {
    fn new() -> Self {
        Self {
            delay: [0.0; HALFBAND_LEN],
        }
    }

    /// Push a sample into the delay line and compute the filtered output.
    #[inline]
    fn push(&mut self, sample: f32) -> f32 {
        // Shift delay line
        for i in (1..HALFBAND_LEN).rev() {
            self.delay[i] = self.delay[i - 1];
        }
        self.delay[0] = sample;

        // FIR convolution — exploit symmetry and zero coefficients:
        // Non-zero taps: 0, 2, 4, 5 (center), 6, 8, 10
        self.delay[5] * HALFBAND_COEFFS[5]
            + (self.delay[0] + self.delay[10]) * HALFBAND_COEFFS[0]
            + (self.delay[2] + self.delay[8]) * HALFBAND_COEFFS[2]
            + (self.delay[4] + self.delay[6]) * HALFBAND_COEFFS[4]
    }

    /// Decimate a block by factor 2: for each pair of input samples, output one sample.
    ///
    /// `input.len()` must be `output.len() * 2`.
    #[inline]
    fn decimate(&mut self, input: &[f32], output: &mut [f32]) {
        let out_len = (input.len() / 2).min(output.len());
        for i in 0..out_len {
            self.push(input[i * 2]);
            output[i] = self.push(input[i * 2 + 1]);
        }
    }

    fn reset(&mut self) {
        self.delay = [0.0; HALFBAND_LEN];
    }
}

// ============================================================================
// DOWNSAMPLER
// ============================================================================

/// Maximum intermediate buffer size for 4x → 2x first stage.
/// Supports buffers up to 4096 * 2 = 8192 samples per channel.
const MAX_INTERMEDIATE: usize = 8192;

/// Multi-stage downsampler supporting 1x (passthrough), 2x, and 4x decimation.
///
/// For 4x decimation, two half-band stages are cascaded:
/// 4x → 2x (stage 1) → 1x (stage 2).
///
/// Internal buffers are pre-allocated to avoid real-time allocations.
#[derive(Clone)]
pub struct Downsampler {
    stage1: HalfBandFilter,
    stage2: HalfBandFilter,
    /// Pre-allocated intermediate buffer for 4x cascaded decimation.
    intermediate: [f32; MAX_INTERMEDIATE],
}

impl Downsampler {
    /// Create a new downsampler with pre-allocated buffers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stage1: HalfBandFilter::new(),
            stage2: HalfBandFilter::new(),
            intermediate: [0.0; MAX_INTERMEDIATE],
        }
    }

    /// Downsample `input` into `output` by the given factor.
    ///
    /// Requirements:
    /// - `input.len() == output.len() * factor`
    /// - For 4x: `input.len() / 2 <= MAX_INTERMEDIATE` (8192)
    #[inline]
    pub fn process(&mut self, input: &[f32], output: &mut [f32], factor: OversamplingFactor) {
        match factor {
            OversamplingFactor::X1 => {
                output[..input.len()].copy_from_slice(input);
            }
            OversamplingFactor::X2 => {
                self.stage1.decimate(input, output);
            }
            OversamplingFactor::X4 => {
                // Two-stage cascade: 4x → 2x → 1x
                let intermediate_len = (input.len() / 2).min(MAX_INTERMEDIATE);
                let usable_input_len = intermediate_len * 2;
                self.stage1.decimate(
                    &input[..usable_input_len],
                    &mut self.intermediate[..intermediate_len],
                );
                self.stage2
                    .decimate(&self.intermediate[..intermediate_len], output);
            }
        }
    }

    /// Reset filter state (call when oversampling factor changes).
    pub fn reset(&mut self) {
        self.stage1.reset();
        self.stage2.reset();
    }
}

impl Default for Downsampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_oversampling_factor_basics() {
        assert_eq!(OversamplingFactor::X1.factor(), 1);
        assert_eq!(OversamplingFactor::X2.factor(), 2);
        assert_eq!(OversamplingFactor::X4.factor(), 4);

        assert_eq!(OversamplingFactor::X1.name(), "Off");
        assert_eq!(OversamplingFactor::X2.name(), "2x");
        assert_eq!(OversamplingFactor::X4.name(), "4x");

        assert_eq!(OversamplingFactor::from_index(0), OversamplingFactor::X1);
        assert_eq!(OversamplingFactor::from_index(1), OversamplingFactor::X2);
        assert_eq!(OversamplingFactor::from_index(2), OversamplingFactor::X4);
        assert_eq!(OversamplingFactor::from_index(99), OversamplingFactor::X1);
    }

    #[test]
    fn test_downsampler_passthrough() {
        let mut ds = Downsampler::new();
        let input = [1.0, 2.0, 3.0, 4.0];
        let mut output = [0.0; 4];
        ds.process(&input, &mut output, OversamplingFactor::X1);
        assert_eq!(output, input);
    }

    #[test]
    fn test_downsampler_2x_dc() {
        // A constant DC signal should pass through (with filter settling)
        let mut ds = Downsampler::new();
        let input = vec![1.0; 256];
        let mut output = vec![0.0; 128];
        ds.process(&input, &mut output, OversamplingFactor::X2);

        // After filter settling, output should approach 1.0
        // (half-band filter has unity DC gain)
        let last = output[output.len() - 1];
        assert!(
            (last - 1.0).abs() < 0.01,
            "DC signal should pass through, got {last}"
        );
    }

    #[test]
    fn test_downsampler_4x_dc() {
        let mut ds = Downsampler::new();
        let input = vec![1.0; 512];
        let mut output = vec![0.0; 128];
        ds.process(&input, &mut output, OversamplingFactor::X4);

        let last = output[output.len() - 1];
        assert!(
            (last - 1.0).abs() < 0.02,
            "DC signal should pass through 4x, got {last}"
        );
    }

    #[test]
    fn test_halfband_coefficients_sum_to_unity() {
        let sum: f32 = HALFBAND_COEFFS.iter().sum();
        assert!(
            (sum - 1.0).abs() < 0.001,
            "Half-band coefficients should sum to 1.0, got {sum}"
        );
    }
}
