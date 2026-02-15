//! Feedback Delay Network (FDN) core for shared use in reverb and AWE.
//!
//! Contains:
//! - 8-channel FDN with Hadamard mixing matrix
//! - Per-channel damping (lowpass) and low-cut (highpass) filters
//! - Modulated delay times for diffusion
//! - Stereo output with width control

use synth_core::FilterState;

/// Number of channels in the FDN.
pub const FDN_CHANNELS: usize = 8;

/// Base delay times in samples at 44100 Hz (prime numbers for minimal modal coincidence).
pub const BASE_DELAY_TIMES: [usize; FDN_CHANNELS] =
    [2039, 2311, 2543, 2719, 2917, 3109, 3301, 3511];

/// Normalized 8x8 Hadamard matrix (each element is +/-1/sqrt(8)).
///
/// Generated recursively from the 2x2 base case:
///   H2 = [[1, 1], [1, -1]]
///   H4 = [[H2, H2], [H2, -H2]]
///   H8 = [[H4, H4], [H4, -H4]]
/// Then normalized by 1/sqrt(8).
const HADAMARD_SCALE: f32 = 0.353_553_39; // 1.0 / sqrt(8.0)

const HADAMARD_8: [[f32; FDN_CHANNELS]; FDN_CHANNELS] = {
    let s = HADAMARD_SCALE;
    let n = -HADAMARD_SCALE;
    [
        [s, s, s, s, s, s, s, s],
        [s, n, s, n, s, n, s, n],
        [s, s, n, n, s, s, n, n],
        [s, n, n, s, s, n, n, s],
        [s, s, s, s, n, n, n, n],
        [s, n, s, n, n, s, n, s],
        [s, s, n, n, n, n, s, s],
        [s, n, n, s, n, s, s, n],
    ]
};

/// Per-channel LFO rates in Hz for delay modulation (~0.3 Hz, slightly different per channel).
pub const LFO_RATES: [f32; FDN_CHANNELS] = [0.27, 0.31, 0.29, 0.33, 0.26, 0.34, 0.28, 0.32];

/// Maximum delay modulation depth in samples.
const MAX_MOD_DEPTH_SAMPLES: f32 = 3.0;

/// A single FDN delay channel with delay line, damping filter, and low-cut filter.
pub(crate) struct FdnChannel {
    /// Circular delay buffer (pre-allocated).
    buffer: Vec<f32>,
    /// Write index into the delay buffer.
    write_index: usize,
    /// Current delay length in samples (base, before modulation).
    delay_samples: usize,
    /// One-pole lowpass filter state for damping in feedback path.
    lowpass_state: FilterState,
    /// One-pole highpass filter state for low-cut in feedback path.
    highpass_state: FilterState,
    /// LFO phase for delay modulation (0.0 to 1.0).
    lfo_phase: f32,
    /// LFO rate in Hz for this channel.
    lfo_rate: f32,
}

impl FdnChannel {
    fn new(base_delay: usize, lfo_rate: f32) -> Self {
        // Pre-allocate worst-case buffer to prevent heap allocation in resize()
        // on the audio thread. Factor 48 covers: sample_rate_scale ≈ 2.18 (96 kHz)
        // × effective room_scale up to ~22 (large rooms like 150m Tube with tail_stretch 3.5).
        // If exceeded, delay is clamped rather than allocating.
        let max_size = base_delay * 48 + 16;
        Self {
            buffer: vec![0.0; max_size],
            write_index: 0,
            delay_samples: base_delay,
            lowpass_state: FilterState::ZERO,
            highpass_state: FilterState::ZERO,
            lfo_phase: 0.0,
            lfo_rate,
        }
    }

    /// Set the delay length, clamping to pre-allocated buffer capacity (RT-safe: no allocation).
    pub(crate) fn resize(&mut self, new_delay: usize) {
        // Clamp delay to fit within pre-allocated buffer.
        // Buffer needs delay_samples + MAX_MOD_DEPTH_SAMPLES + 2 for interpolated reads.
        let max_delay = self.buffer.len().saturating_sub(6);
        self.delay_samples = new_delay.min(max_delay).max(1);
        // Ensure write_index is valid
        if self.write_index >= self.buffer.len() {
            self.write_index = 0;
        }
    }

    /// Read from the delay line with linear interpolation (for modulated delay).
    #[inline]
    fn read_interpolated(&self, delay_frac: f32) -> f32 {
        let buf_len = self.buffer.len();
        let delay_int = delay_frac as usize;
        let frac = delay_frac - delay_int as f32;

        let read_a = (self.write_index + buf_len - delay_int) % buf_len;
        let read_b = (self.write_index + buf_len - delay_int - 1) % buf_len;

        self.buffer[read_a] * (1.0 - frac) + self.buffer[read_b] * frac
    }

    /// Write a sample into the delay line and advance the write pointer.
    #[inline]
    fn write_and_advance(&mut self, sample: f32) {
        self.buffer[self.write_index] = sample;
        self.write_index = (self.write_index + 1) % self.buffer.len();
    }

    /// Advance the internal LFO phase.
    #[inline]
    fn advance_lfo(&mut self, sample_rate_recip: f32) {
        self.lfo_phase += self.lfo_rate * sample_rate_recip;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= 1.0;
        }
    }

    /// Get the current modulated delay in fractional samples.
    #[inline]
    fn modulated_delay(&self, diffusion: f32) -> f32 {
        // Sine LFO for smooth modulation
        let lfo_value = (self.lfo_phase * std::f32::consts::TAU).sin();
        let mod_depth = diffusion * MAX_MOD_DEPTH_SAMPLES;
        let delay = self.delay_samples as f32 + lfo_value * mod_depth;
        // Clamp to valid range
        delay.max(1.0).min((self.buffer.len() - 2) as f32)
    }

    /// Clear all state.
    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_index = 0;
        self.lowpass_state = FilterState::ZERO;
        self.highpass_state = FilterState::ZERO;
        self.lfo_phase = 0.0;
    }
}

/// Stereo output from the FDN.
#[derive(Debug, Clone, Copy)]
pub struct FdnStereoOutput {
    /// Left channel sample.
    pub left: f32,
    /// Right channel sample.
    pub right: f32,
}

/// 8-channel Feedback Delay Network core.
///
/// Processes a mono input sample through an 8-channel FDN with Hadamard mixing,
/// per-channel damping/lowcut filters, and modulated delay lines.
/// Returns a stereo output pair.
pub struct FdnCore {
    channels: [FdnChannel; FDN_CHANNELS],
}

impl FdnCore {
    /// Create a new FDN core with default delay times.
    #[must_use]
    pub fn new() -> Self {
        Self {
            channels: [
                FdnChannel::new(BASE_DELAY_TIMES[0], LFO_RATES[0]),
                FdnChannel::new(BASE_DELAY_TIMES[1], LFO_RATES[1]),
                FdnChannel::new(BASE_DELAY_TIMES[2], LFO_RATES[2]),
                FdnChannel::new(BASE_DELAY_TIMES[3], LFO_RATES[3]),
                FdnChannel::new(BASE_DELAY_TIMES[4], LFO_RATES[4]),
                FdnChannel::new(BASE_DELAY_TIMES[5], LFO_RATES[5]),
                FdnChannel::new(BASE_DELAY_TIMES[6], LFO_RATES[6]),
                FdnChannel::new(BASE_DELAY_TIMES[7], LFO_RATES[7]),
            ],
        }
    }

    /// Process a single mono input sample through the FDN.
    ///
    /// Returns a stereo output pair with the specified width.
    ///
    /// # Arguments
    /// * `input` - Mono input sample (typically pre-delayed)
    /// * `feedback_gain` - Feedback amount (0.0 to ~0.97)
    /// * `lp_coeff` - Lowpass damping coefficient (0.0=no filtering, 0.9=heavy)
    /// * `hp_coeff` - Highpass low-cut coefficient
    /// * `diffusion` - Delay modulation depth (0.0 to 1.0)
    /// * `width` - Stereo width (0.0=mono, 1.0=full stereo)
    /// * `sample_rate_recip` - 1.0 / sample_rate
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn process_sample(
        &mut self,
        input: f32,
        feedback_gain: f32,
        lp_coeff: f32,
        hp_coeff: f32,
        diffusion: f32,
        width: f32,
        sample_rate_recip: f32,
    ) -> FdnStereoOutput {
        // Step 1: Read from all delay lines (before writing new values)
        let mut delay_outputs = [0.0_f32; FDN_CHANNELS];
        for i in 0..FDN_CHANNELS {
            let mod_delay = self.channels[i].modulated_delay(diffusion);
            delay_outputs[i] = self.channels[i].read_interpolated(mod_delay);
        }

        // Step 2: Apply Hadamard mixing matrix
        let mut mixed = [0.0_f32; FDN_CHANNELS];
        for i in 0..FDN_CHANNELS {
            let mut sum = 0.0_f32;
            for j in 0..FDN_CHANNELS {
                sum += HADAMARD_8[i][j] * delay_outputs[j];
            }
            mixed[i] = sum;
        }

        // Step 3: Apply feedback gain, damping (lowpass), low-cut (highpass),
        //         add input, and write back into delay lines
        for i in 0..FDN_CHANNELS {
            let fb = mixed[i] * feedback_gain;

            // One-pole lowpass for damping (high frequency absorption)
            let lp_out = self.channels[i].lowpass_state.one_pole(fb, lp_coeff);

            // One-pole highpass for low-cut (remove low frequency buildup)
            let hp_out = self.channels[i]
                .highpass_state
                .one_pole_hp(lp_out, hp_coeff);

            // Flush denormals periodically (cheap check)
            self.channels[i].lowpass_state.flush_denormals();
            self.channels[i].highpass_state.flush_denormals();

            // Add input signal and write to delay line
            let write_val = hp_out + input;
            self.channels[i].write_and_advance(write_val);

            // Advance LFO
            self.channels[i].advance_lfo(sample_rate_recip);
        }

        // Step 4: Sum to stereo output
        // Even channels (0,2,4,6) -> left, odd channels (1,3,5,7) -> right
        let mut left_sum = 0.0_f32;
        let mut right_sum = 0.0_f32;
        for i in 0..FDN_CHANNELS {
            if i % 2 == 0 {
                left_sum += delay_outputs[i];
            } else {
                right_sum += delay_outputs[i];
            }
        }

        // Normalize by number of channels per side (4)
        left_sum *= 0.25;
        right_sum *= 0.25;

        // Apply width (stereo crossfeed)
        // width=0 -> mono (L=R=(L+R)/2), width=1 -> full stereo
        let mono = (left_sum + right_sum) * 0.5;
        let wet_left = mono + (left_sum - mono) * width;
        let wet_right = mono + (right_sum - mono) * width;

        FdnStereoOutput {
            left: wet_left,
            right: wet_right,
        }
    }

    /// Set delay times for all channels based on scale factors.
    ///
    /// # Arguments
    /// * `sample_rate_scale` - sample_rate / 44100.0
    /// * `room_scale` - room size scaling factor
    pub fn set_delay_times(&mut self, sample_rate_scale: f32, room_scale: f32) {
        for (i, ch) in self.channels.iter_mut().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let new_delay = (BASE_DELAY_TIMES[i] as f32 * sample_rate_scale * room_scale) as usize;
            ch.resize(new_delay.max(1));
        }
    }

    /// Clear all channel state (buffers, filters, LFOs).
    pub fn clear(&mut self) {
        for ch in &mut self.channels {
            ch.clear();
        }
    }
}

impl Default for FdnCore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hadamard_orthogonality() {
        // Verify that H * H^T = I (the Hadamard matrix is orthogonal when normalized)
        for i in 0..FDN_CHANNELS {
            for j in 0..FDN_CHANNELS {
                let mut dot: f32 = 0.0;
                for k in 0..FDN_CHANNELS {
                    dot += HADAMARD_8[i][k] * HADAMARD_8[j][k];
                }
                if i == j {
                    assert!(
                        (dot - 1.0).abs() < 0.001,
                        "Diagonal element [{i}][{j}] should be 1.0, got {dot}"
                    );
                } else {
                    assert!(
                        dot.abs() < 0.001,
                        "Off-diagonal element [{i}][{j}] should be 0.0, got {dot}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_fdn_core_process_sample() {
        let mut fdn = FdnCore::new();
        let output = fdn.process_sample(1.0, 0.5, 0.3, 0.9, 0.5, 1.0, 1.0 / 44100.0);
        assert!(output.left.is_finite());
        assert!(output.right.is_finite());
    }

    #[test]
    fn test_fdn_core_stability() {
        let mut fdn = FdnCore::new();
        // Feed an impulse and process many samples
        fdn.process_sample(1.0, 0.9, 0.5, 0.9, 0.5, 1.0, 1.0 / 44100.0);
        for _ in 0..10000 {
            let out = fdn.process_sample(0.0, 0.9, 0.5, 0.9, 0.5, 1.0, 1.0 / 44100.0);
            assert!(out.left.is_finite(), "FDN output is not finite");
            assert!(out.right.is_finite(), "FDN output is not finite");
            assert!(out.left.abs() < 10.0, "FDN output exploded");
            assert!(out.right.abs() < 10.0, "FDN output exploded");
        }
    }

    #[test]
    fn test_fdn_core_set_delay_times() {
        let mut fdn = FdnCore::new();
        fdn.set_delay_times(1.0, 1.5);
        // Should not panic
        let out = fdn.process_sample(1.0, 0.5, 0.3, 0.9, 0.5, 1.0, 1.0 / 44100.0);
        assert!(out.left.is_finite());
    }

    #[test]
    fn test_fdn_core_clear() {
        let mut fdn = FdnCore::new();
        // Feed some signal
        for _ in 0..100 {
            fdn.process_sample(1.0, 0.5, 0.3, 0.9, 0.5, 1.0, 1.0 / 44100.0);
        }
        fdn.clear();
        // After clearing, output should be zero
        let out = fdn.process_sample(0.0, 0.5, 0.3, 0.9, 0.5, 1.0, 1.0 / 44100.0);
        assert!((out.left).abs() < 0.0001);
        assert!((out.right).abs() < 0.0001);
    }
}
