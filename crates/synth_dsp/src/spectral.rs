//! FFT-based spectral processing infrastructure.
//!
//! Provides:
//! - [`WindowType`] — Standard window functions (Hann, Hamming, Blackman-Harris)
//! - [`FftProcessor`] — Pre-allocated real FFT wrapper
//! - [`StftProcessor`] — Overlap-add STFT with spectral processing callback
//! - [`PartitionedConvolver`] — Uniform partitioned convolution for long IRs

use realfft::{RealFftPlanner, num_complex::Complex};
use std::f32::consts::PI;

// ============================================================================
// WINDOW FUNCTIONS
// ============================================================================

/// Standard window function types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowType {
    Hann,
    Hamming,
    BlackmanHarris,
}

/// Fill a buffer with the specified window function.
pub fn fill_window(buf: &mut [f32], window: WindowType) {
    let n = buf.len();
    if n == 0 {
        return;
    }
    if n == 1 {
        buf[0] = 1.0;
        return;
    }
    let nm1 = (n - 1) as f32;
    for (i, sample) in buf.iter_mut().enumerate() {
        let x = i as f32 / nm1;
        *sample = match window {
            WindowType::Hann => 0.5 * (1.0 - (2.0 * PI * x).cos()),
            WindowType::Hamming => 0.54 - 0.46 * (2.0 * PI * x).cos(),
            WindowType::BlackmanHarris => {
                0.355_68 - 0.487_396 * (2.0 * PI * x).cos() + 0.144_232 * (4.0 * PI * x).cos()
                    - 0.012_604 * (6.0 * PI * x).cos()
            }
        };
    }
}

// ============================================================================
// FFT PROCESSOR
// ============================================================================

/// Pre-allocated real FFT wrapper.
///
/// Manages the forward/inverse FFT plans and scratch buffers so callers
/// never need to allocate during processing.
pub struct FftProcessor {
    fft_size: usize,
    forward: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    inverse: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
    scratch_forward: Vec<Complex<f32>>,
    scratch_inverse: Vec<Complex<f32>>,
}

impl FftProcessor {
    /// Create a new FFT processor for the given size (must be power of 2).
    ///
    /// # Panics
    /// Panics if `fft_size` is not a power of 2 or is zero.
    pub fn new(fft_size: usize) -> Self {
        assert!(
            fft_size > 0 && fft_size.is_power_of_two(),
            "FFT size must be a power of 2, got {fft_size}"
        );
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(fft_size);
        let inverse = planner.plan_fft_inverse(fft_size);
        let scratch_forward = forward.make_scratch_vec();
        let scratch_inverse = inverse.make_scratch_vec();
        Self {
            fft_size,
            forward,
            inverse,
            scratch_forward,
            scratch_inverse,
        }
    }

    /// FFT size.
    #[must_use]
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    /// Number of complex bins (fft_size/2 + 1).
    #[must_use]
    pub fn complex_size(&self) -> usize {
        self.fft_size / 2 + 1
    }

    /// Forward FFT: time-domain `input` -> frequency-domain `output`.
    ///
    /// `input` must have length `fft_size`, `output` must have length `fft_size/2 + 1`.
    pub fn forward(&mut self, input: &mut [f32], output: &mut [Complex<f32>]) {
        if let Err(_e) = self
            .forward
            .process_with_scratch(input, output, &mut self.scratch_forward)
        {
            // Silently fail in real-time context
            for bin in output.iter_mut() {
                *bin = Complex::new(0.0, 0.0);
            }
        }
    }

    /// Inverse FFT: frequency-domain `input` -> time-domain `output`.
    ///
    /// `input` must have length `fft_size/2 + 1`, `output` must have length `fft_size`.
    /// Output is NOT normalized — divide by `fft_size` after calling.
    pub fn inverse(&mut self, input: &mut [Complex<f32>], output: &mut [f32]) {
        if let Err(_e) = self
            .inverse
            .process_with_scratch(input, output, &mut self.scratch_inverse)
        {
            for s in output.iter_mut() {
                *s = 0.0;
            }
        }
    }
}

// ============================================================================
// STFT PROCESSOR
// ============================================================================

/// Overlap-add STFT processor with spectral processing callback.
///
/// Call [`process`] each audio block. The callback receives the complex spectrum
/// for in-place modification; the processor handles windowing, overlap, and
/// output accumulation.
pub struct StftProcessor {
    fft_size: usize,
    hop_size: usize,
    fft: FftProcessor,

    // Pre-allocated buffers
    window: Vec<f32>,
    input_ring: Vec<f32>,
    output_accum: Vec<f32>,
    fft_in: Vec<f32>,
    fft_out: Vec<Complex<f32>>,
    ifft_out: Vec<f32>,

    ring_pos: usize,
    samples_since_last_fft: usize,
}

impl StftProcessor {
    /// Create a new STFT processor.
    ///
    /// `fft_size` must be a power of 2. `hop_size` is typically `fft_size / 4`.
    #[must_use]
    pub fn new(fft_size: usize, hop_size: usize, window: WindowType) -> Self {
        let complex_size = fft_size / 2 + 1;
        let mut win = vec![0.0f32; fft_size];
        fill_window(&mut win, window);

        Self {
            fft_size,
            hop_size,
            fft: FftProcessor::new(fft_size),
            window: win,
            input_ring: vec![0.0; fft_size],
            output_accum: vec![0.0; fft_size * 2],
            fft_in: vec![0.0; fft_size],
            fft_out: vec![Complex::new(0.0, 0.0); complex_size],
            ifft_out: vec![0.0; fft_size],
            ring_pos: 0,
            samples_since_last_fft: 0,
        }
    }

    /// FFT size.
    #[must_use]
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    /// Reset internal state (call on note-on or when switching presets).
    pub fn reset(&mut self) {
        self.input_ring.fill(0.0);
        self.output_accum.fill(0.0);
        self.ring_pos = 0;
        self.samples_since_last_fft = 0;
    }

    /// Process a block of audio with a spectral callback.
    ///
    /// `spectral_fn` receives `&mut [Complex<f32>]` (the spectrum) for in-place modification.
    pub fn process<F>(&mut self, input: &[f32], output: &mut [f32], mut spectral_fn: F)
    where
        F: FnMut(&mut [Complex<f32>]),
    {
        let fft_size = self.fft_size;
        let hop_size = self.hop_size;
        let norm = 1.0 / fft_size as f32;

        for (i, &sample) in input.iter().enumerate() {
            // Write into input ring buffer
            self.input_ring[self.ring_pos] = sample;
            self.ring_pos = (self.ring_pos + 1) % fft_size;
            self.samples_since_last_fft += 1;

            // When we've accumulated enough samples, run FFT
            if self.samples_since_last_fft >= hop_size {
                self.samples_since_last_fft = 0;

                // Copy ring buffer into fft_in with correct ordering and apply window
                for j in 0..fft_size {
                    let ring_idx = (self.ring_pos + j) % fft_size;
                    self.fft_in[j] = self.input_ring[ring_idx] * self.window[j];
                }

                // Forward FFT
                self.fft.forward(&mut self.fft_in, &mut self.fft_out);

                // Apply spectral processing
                spectral_fn(&mut self.fft_out);

                // Inverse FFT
                self.fft.inverse(&mut self.fft_out, &mut self.ifft_out);

                // Overlap-add into output accumulator
                let out_pos = self.ring_pos; // aligns with current position
                for j in 0..fft_size {
                    let idx = (out_pos + j) % (fft_size * 2);
                    self.output_accum[idx] += self.ifft_out[j] * self.window[j] * norm;
                }
            }

            // Read from output accumulator
            let read_idx = self.ring_pos % (fft_size * 2);
            output[i] = self.output_accum[read_idx];
            self.output_accum[read_idx] = 0.0;
        }
    }
}

// ============================================================================
// PARTITIONED CONVOLVER
// ============================================================================

/// Uniform partitioned convolution for efficient long-IR convolution.
///
/// Splits the impulse response into `fft_size/2`-length partitions,
/// pre-computes their spectra, and convolves in the frequency domain
/// using overlap-save.
pub struct PartitionedConvolver {
    fft_size: usize,
    partition_size: usize,
    fft: FftProcessor,

    /// Pre-computed IR partition spectra.
    ir_spectra: Vec<Vec<Complex<f32>>>,

    /// Input history ring buffer (time domain).
    input_buffer: Vec<f32>,
    input_pos: usize,

    /// FFT of recent input blocks (frequency domain ring).
    input_spectra: Vec<Vec<Complex<f32>>>,
    input_spectra_pos: usize,

    /// Temporary buffers.
    fft_in: Vec<f32>,
    fft_out: Vec<Complex<f32>>,
    accum: Vec<Complex<f32>>,
    ifft_out: Vec<f32>,
}

impl PartitionedConvolver {
    /// Create a new partitioned convolver.
    ///
    /// `partition_size` is the hop size (typically 256-2048).
    /// `ir` is the full impulse response.
    #[must_use]
    pub fn new(partition_size: usize, ir: &[f32]) -> Self {
        let fft_size = partition_size * 2;
        let complex_size = fft_size / 2 + 1;
        let mut fft = FftProcessor::new(fft_size);

        // Partition IR and pre-compute spectra
        let num_partitions = ir.len().div_ceil(partition_size);
        let num_partitions = num_partitions.max(1);
        let mut ir_spectra = Vec::with_capacity(num_partitions);

        let mut fft_buf = vec![0.0f32; fft_size];
        let mut spec_buf = vec![Complex::new(0.0, 0.0); complex_size];

        for p in 0..num_partitions {
            fft_buf.fill(0.0);
            let start = p * partition_size;
            let end = (start + partition_size).min(ir.len());
            for (i, &s) in ir[start..end].iter().enumerate() {
                fft_buf[i] = s;
            }
            fft.forward(&mut fft_buf, &mut spec_buf);
            ir_spectra.push(spec_buf.clone());
        }

        let input_spectra = (0..num_partitions)
            .map(|_| vec![Complex::new(0.0, 0.0); complex_size])
            .collect();

        Self {
            fft_size,
            partition_size,
            fft,
            ir_spectra,
            input_buffer: vec![0.0; fft_size],
            input_pos: 0,
            input_spectra,
            input_spectra_pos: 0,
            fft_in: vec![0.0; fft_size],
            fft_out: vec![Complex::new(0.0, 0.0); complex_size],
            accum: vec![Complex::new(0.0, 0.0); complex_size],
            ifft_out: vec![0.0; fft_size],
        }
    }

    /// Reset internal state.
    pub fn reset(&mut self) {
        self.input_buffer.fill(0.0);
        self.input_pos = 0;
        for spec in &mut self.input_spectra {
            for bin in spec.iter_mut() {
                *bin = Complex::new(0.0, 0.0);
            }
        }
        self.input_spectra_pos = 0;
    }

    /// Process a block of exactly `partition_size` samples.
    ///
    /// For blocks of different sizes, buffer externally and call this
    /// in chunks of `partition_size`.
    pub fn process_block(&mut self, input: &[f32], output: &mut [f32]) {
        let ps = self.partition_size;
        let num_partitions = self.ir_spectra.len();
        let norm = 1.0 / self.fft_size as f32;

        // Prepare zero-padded input: [previous_partition | current_partition]
        // Copy previous partition_size samples (second half of input_buffer)
        // and the new input block
        for i in 0..ps {
            self.fft_in[i] = self.input_buffer[self.input_pos.wrapping_add(i) % self.fft_size];
        }
        let len = ps.min(input.len());
        self.fft_in[ps..ps + len].copy_from_slice(&input[..len]);
        for i in len..ps {
            self.fft_in[ps + i] = 0.0;
        }

        // Update input ring
        for i in 0..len {
            self.input_buffer[(self.input_pos + i) % self.fft_size] = input[i];
        }
        self.input_pos = (self.input_pos + ps) % self.fft_size;

        // Forward FFT of input block
        self.fft.forward(&mut self.fft_in, &mut self.fft_out);
        self.input_spectra[self.input_spectra_pos].copy_from_slice(&self.fft_out);

        // Accumulate frequency-domain products
        for bin in self.accum.iter_mut() {
            *bin = Complex::new(0.0, 0.0);
        }

        for p in 0..num_partitions {
            let spec_idx = (self.input_spectra_pos + num_partitions - p) % num_partitions;
            let ir_spec = &self.ir_spectra[p];
            let in_spec = &self.input_spectra[spec_idx];

            for (i, acc) in self.accum.iter_mut().enumerate() {
                *acc += in_spec[i] * ir_spec[i];
            }
        }

        // Inverse FFT
        self.fft.inverse(&mut self.accum, &mut self.ifft_out);

        // Overlap-save: take second half of output (the valid part)
        let out_len = ps.min(output.len());
        for i in 0..out_len {
            output[i] = self.ifft_out[ps + i] * norm;
        }

        // Advance input spectra ring
        self.input_spectra_pos = (self.input_spectra_pos + 1) % num_partitions;
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fill_window_hann() {
        let mut buf = vec![0.0; 256];
        fill_window(&mut buf, WindowType::Hann);
        // Hann window should be 0 at endpoints, 1 at center
        assert!(buf[0].abs() < 1e-6);
        assert!((buf[128] - 1.0).abs() < 0.01);
        assert!(buf[255].abs() < 1e-6);
    }

    #[test]
    fn test_fft_roundtrip() {
        let fft_size = 256;
        let mut fft = FftProcessor::new(fft_size);
        let mut input = vec![0.0f32; fft_size];
        // Simple sine wave
        for i in 0..fft_size {
            input[i] = (2.0 * PI * i as f32 / fft_size as f32).sin();
        }
        let original = input.clone();

        let mut spectrum = vec![Complex::new(0.0, 0.0); fft_size / 2 + 1];
        fft.forward(&mut input, &mut spectrum);

        let mut output = vec![0.0f32; fft_size];
        fft.inverse(&mut spectrum, &mut output);

        // Normalize
        let norm = 1.0 / fft_size as f32;
        for s in output.iter_mut() {
            *s *= norm;
        }

        // Check roundtrip
        for i in 0..fft_size {
            assert!(
                (output[i] - original[i]).abs() < 1e-4,
                "Mismatch at {}: {} vs {}",
                i,
                output[i],
                original[i]
            );
        }
    }

    #[test]
    fn test_stft_passthrough() {
        let fft_size = 512;
        let hop_size = fft_size / 4;
        let mut stft = StftProcessor::new(fft_size, hop_size, WindowType::Hann);

        // Process silence — should output near-silence
        let input = vec![0.0f32; 2048];
        let mut output = vec![0.0f32; 2048];
        stft.process(&input, &mut output, |_spectrum| {
            // Identity: no modification
        });

        for &s in &output {
            assert!(s.abs() < 1e-6, "Expected silence, got {}", s);
        }
    }

    #[test]
    fn test_partitioned_convolver_impulse() {
        // Convolve with a unit impulse — should pass through
        let partition_size = 256;
        let mut ir = vec![0.0f32; partition_size];
        ir[0] = 1.0;
        let mut conv = PartitionedConvolver::new(partition_size, &ir);

        let mut input = vec![0.0f32; partition_size];
        input[0] = 1.0;
        let mut output = vec![0.0f32; partition_size];
        conv.process_block(&input, &mut output);

        // The first output should contain the convolved impulse
        assert!(
            output[0].abs() > 0.5,
            "Expected impulse at 0, got {}",
            output[0]
        );
    }
}
