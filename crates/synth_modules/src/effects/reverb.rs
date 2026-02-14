//! Reverb effect module.
//!
//! Features:
//! - 8-channel Feedback Delay Network (FDN) reverb
//! - Hadamard mixing matrix for dense reflections
//! - Per-channel damping (lowpass) and low-cut (highpass) filters
//! - Modulated delay times for diffusion
//! - Pre-delay up to 500ms
//! - Stereo output with width control

use synth_core::{
    AudioEffect, Describable, FilterState, Hertz, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, ParameterUnit, ProcessContext, ReverbParam,
    SampleCount, SampleRate, Seconds, StereoSample, WidgetHint,
};

/// Number of channels in the FDN.
const FDN_CHANNELS: usize = 8;

/// Base delay times in samples at 44100 Hz (prime numbers for minimal modal coincidence).
const BASE_DELAY_TIMES: [usize; FDN_CHANNELS] = [2039, 2311, 2543, 2719, 2917, 3109, 3301, 3511];

/// Maximum pre-delay in seconds.
const MAX_PRE_DELAY_SECS: f32 = 0.5;

/// Normalized 8x8 Hadamard matrix (each element is +/-1/sqrt(8)).
///
/// Generated recursively from the 2x2 base case:
///   H2 = [[1, 1], [1, -1]]
///   H4 = [[H2, H2], [H2, -H2]]
///   H8 = [[H4, H4], [H4, -H4]]
/// Then normalized by 1/sqrt(8).
const HADAMARD_SCALE: f32 = 0.353_553_39; // 1.0 / sqrt(8.0)

const HADAMARD_8: [[f32; FDN_CHANNELS]; FDN_CHANNELS] = {
    // Signs for the normalized Hadamard matrix H8
    // Row 0: + + + + + + + +
    // Row 1: + - + - + - + -
    // Row 2: + + - - + + - -
    // Row 3: + - - + + - - +
    // Row 4: + + + + - - - -
    // Row 5: + - + - - + - +
    // Row 6: + + - - - - + +
    // Row 7: + - - + - + + -
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
const LFO_RATES: [f32; FDN_CHANNELS] = [0.27, 0.31, 0.29, 0.33, 0.26, 0.34, 0.28, 0.32];

/// Maximum delay modulation depth in samples.
const MAX_MOD_DEPTH_SAMPLES: f32 = 3.0;

/// A single FDN delay channel with delay line, damping filter, and low-cut filter.
struct FdnChannel {
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
        // Allocate enough for the maximum possible delay (room_size=1.0 at high sample rates)
        // We use 2x base delay as a safe maximum, plus modulation headroom
        let max_size = base_delay * 3 + 16;
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

    /// Resize the delay buffer for a new sample rate.
    fn resize(&mut self, new_delay: usize) {
        self.delay_samples = new_delay;
        let required = new_delay * 3 + 16;
        if self.buffer.len() < required {
            self.buffer.resize(required, 0.0);
        }
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

/// 8-channel Feedback Delay Network reverb.
pub struct Reverb {
    // Parameters
    room_size: NormalizedValue,
    damping: NormalizedValue,
    mix: NormalizedValue,
    pre_delay: Seconds,
    width: NormalizedValue,
    decay: NormalizedValue,
    diffusion: NormalizedValue,
    low_cut: Hertz,

    // FDN channels
    channels: [FdnChannel; FDN_CHANNELS],

    // Pre-delay line
    pre_delay_buffer: Vec<f32>,
    pre_delay_index: usize,

    // State
    sample_rate: SampleRate,
    /// Whether delay line sizes need recalculation.
    params_dirty: bool,
}

impl Reverb {
    #[must_use]
    pub fn new() -> Self {
        Self {
            room_size: NormalizedValue::CENTER,
            damping: NormalizedValue::CENTER,
            mix: NormalizedValue::new(0.3),
            pre_delay: Seconds::ZERO,
            width: NormalizedValue::MAX,
            decay: NormalizedValue::CENTER,
            diffusion: NormalizedValue::CENTER,
            low_cut: Hertz::new(80.0),

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

            pre_delay_buffer: vec![0.0; 48000], // ~1 second at 48kHz
            pre_delay_index: 0,

            sample_rate: SampleRate::DVD_QUALITY,
            params_dirty: true,
        }
    }

    /// Update delay line sizes based on room size and sample rate.
    fn update_delay_sizes(&mut self) {
        if !self.params_dirty {
            return;
        }
        self.params_dirty = false;

        let scale = self.sample_rate.as_f32() / 44100.0;
        // Room size scales delay times: 0.5x to 2.0x
        let room_scale = 0.5 + self.room_size.as_f32() * 1.5;

        for (i, ch) in self.channels.iter_mut().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let new_delay = (BASE_DELAY_TIMES[i] as f32 * scale * room_scale) as usize;
            ch.resize(new_delay.max(1));
        }
    }

    /// Scale all buffers for a new sample rate.
    fn resize_for_sample_rate(&mut self) {
        let scale = self.sample_rate.as_f32() / 44100.0;
        let room_scale = 0.5 + self.room_size.as_f32() * 1.5;

        for (i, ch) in self.channels.iter_mut().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let new_delay = (BASE_DELAY_TIMES[i] as f32 * scale * room_scale) as usize;
            ch.resize(new_delay.max(1));
        }

        // Pre-delay buffer: max 500ms
        #[allow(clippy::cast_possible_truncation)]
        let max_pre_delay = (MAX_PRE_DELAY_SECS * self.sample_rate.as_f32()) as usize;
        let max_pre_delay = max_pre_delay.max(1);
        if self.pre_delay_buffer.len() != max_pre_delay {
            self.pre_delay_buffer.resize(max_pre_delay, 0.0);
            self.pre_delay_index = 0;
        }
    }

    /// Compute the feedback gain from the decay parameter.
    ///
    /// Decay 0.0 -> short reverb (low feedback), Decay 1.0 -> long reverb (high feedback).
    /// Maps to approximately 0.3 .. 0.97 feedback range.
    #[inline]
    fn feedback_gain(&self) -> f32 {
        0.3 + self.decay.as_f32() * 0.67
    }

    /// Compute the lowpass coefficient from the damping parameter.
    ///
    /// Higher damping = more high-frequency absorption.
    /// coeff near 1.0 = heavy filtering, coeff near 0.0 = no filtering.
    #[inline]
    fn lowpass_coeff(&self) -> f32 {
        self.damping.as_f32() * 0.9
    }

    /// Compute the highpass coefficient from the low-cut frequency.
    ///
    /// Simple one-pole highpass: coeff = 1 - (2*pi*fc / sr)
    #[inline]
    fn highpass_coeff(&self) -> f32 {
        let fc = self.low_cut.as_f32();
        let sr = self.sample_rate.as_f32();
        // One-pole highpass coefficient
        // coeff close to 1.0 = low cutoff, coeff close to 0.0 = high cutoff
        (1.0 - (std::f32::consts::TAU * fc / sr)).clamp(0.0, 0.9999)
    }
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Reverb {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("reverb", "Reverb")
            .description("FDN reverb with 8-channel feedback delay network")
            .category(ModuleCategory::Effect)
            .tag("reverb")
            .tag("effect")
            .tag("space")
            // No ports - effect chain modules are processed automatically
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::RoomSize(NormalizedValue::CENTER)),
                    "Room Size",
                )
                .description("Size of the virtual room (scales delay times)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::Decay(NormalizedValue::CENTER)),
                    "Decay",
                )
                .description("Reverb tail length (feedback gain)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::Damping(NormalizedValue::CENTER)),
                    "Damping",
                )
                .description("High frequency absorption in feedback path")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::Diffusion(NormalizedValue::CENTER)),
                    "Diffusion",
                )
                .description("Delay modulation for density (0=sparse, 1=dense)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::PreDelay(Seconds::ZERO)),
                    "Pre-Delay",
                )
                .description("Initial delay before reverb onset")
                .range(0.0, 0.5)
                .default(0.0)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::TimeSlider),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::LowCut(Hertz::new(80.0))),
                    "Low Cut",
                )
                .description("Highpass filter frequency for reverb tail")
                .range(20.0, 500.0)
                .default(80.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::Width(NormalizedValue::MAX)),
                    "Width",
                )
                .description("Stereo width (0=mono, 1=full stereo)")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::Mix(NormalizedValue::new(0.3))),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Reverb {
    #[allow(clippy::too_many_lines)]
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        debug_assert!(
            (self.sample_rate.as_f32() - context.sample_rate.as_f32()).abs() <= 1.0,
            "Reverb sample rate mismatch - call set_sample_rate() before processing"
        );

        self.update_delay_sizes();

        let feedback_gain = self.feedback_gain();
        let lp_coeff = self.lowpass_coeff();
        let hp_coeff = self.highpass_coeff();
        let diffusion = self.diffusion.as_f32();
        let mix = self.mix.as_f32();
        let width = self.width.as_f32();
        let sample_rate_recip = 1.0 / self.sample_rate.as_f32();

        #[allow(clippy::cast_possible_truncation)]
        let pre_delay_samples = ((self.pre_delay.as_f32() * self.sample_rate.as_f32()) as usize)
            .min(self.pre_delay_buffer.len() - 1);

        let channels = 2;
        for frame in 0..context.samples.as_usize() {
            let idx_l = frame * channels;
            let idx_r = frame * channels + 1;

            // Get input as stereo sample
            let dry = if idx_r < input.len() {
                StereoSample::new(input[idx_l], input[idx_r])
            } else if idx_l < input.len() {
                StereoSample::from_mono(input[idx_l])
            } else {
                StereoSample::ZERO
            };

            // Pre-delay (works on mono sum of input)
            let pre_delayed = if pre_delay_samples > 0 {
                let mono_in = dry.to_mono();
                let read_idx = (self.pre_delay_index + self.pre_delay_buffer.len()
                    - pre_delay_samples)
                    % self.pre_delay_buffer.len();
                let delayed = self.pre_delay_buffer[read_idx];
                self.pre_delay_buffer[self.pre_delay_index] = mono_in;
                self.pre_delay_index = (self.pre_delay_index + 1) % self.pre_delay_buffer.len();
                delayed
            } else {
                dry.to_mono()
            };

            // --- FDN Processing ---

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
                let write_val = hp_out + pre_delayed;
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

            // Mix dry/wet
            let result = StereoSample::new(
                dry.left * (1.0 - mix) + wet_left * mix,
                dry.right * (1.0 - mix) + wet_right * mix,
            );

            if idx_l < output.len() {
                output[idx_l] = result.left;
            }
            if idx_r < output.len() {
                output[idx_r] = result.right;
            }
        }
    }

    fn reset(&mut self) {
        for ch in &mut self.channels {
            ch.clear();
        }
        self.pre_delay_buffer.fill(0.0);
        self.pre_delay_index = 0;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn tail_samples(&self) -> SampleCount {
        // Tail length depends on decay: longer decay = longer tail
        let decay_time = 1.0 + self.decay.as_f32() * 8.0;
        #[allow(clippy::cast_possible_truncation)]
        let samples = (decay_time * self.sample_rate.as_f32()) as usize;
        SampleCount::new(samples)
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Reverb(reverb_param) = param {
            match reverb_param {
                ReverbParam::RoomSize(s) => {
                    self.room_size = s;
                    self.params_dirty = true;
                }
                ReverbParam::Damping(d) => {
                    self.damping = d;
                }
                ReverbParam::PreDelay(p) => {
                    self.pre_delay = Seconds::new(p.as_f32().clamp(0.0, MAX_PRE_DELAY_SECS));
                }
                ReverbParam::Width(w) => self.width = w,
                ReverbParam::Mix(m) => self.mix = m,
                ReverbParam::Decay(d) => self.decay = d,
                ReverbParam::Diffusion(d) => self.diffusion = d,
                ReverbParam::LowCut(hz) => {
                    self.low_cut = Hertz::new(hz.as_f32().clamp(20.0, 500.0));
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Reverb(reverb_param) = param {
            Some(match reverb_param {
                ReverbParam::RoomSize(_) => self.room_size.as_f32(),
                ReverbParam::Damping(_) => self.damping.as_f32(),
                ReverbParam::PreDelay(_) => self.pre_delay.as_f32(),
                ReverbParam::Width(_) => self.width.as_f32(),
                ReverbParam::Mix(_) => self.mix.as_f32(),
                ReverbParam::Decay(_) => self.decay.as_f32(),
                ReverbParam::Diffusion(_) => self.diffusion.as_f32(),
                ReverbParam::LowCut(_) => self.low_cut.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Reverb(ReverbParam::RoomSize(self.room_size)),
            Param::Reverb(ReverbParam::Decay(self.decay)),
            Param::Reverb(ReverbParam::PreDelay(self.pre_delay)),
            Param::Reverb(ReverbParam::Damping(self.damping)),
            Param::Reverb(ReverbParam::Diffusion(self.diffusion)),
            Param::Reverb(ReverbParam::LowCut(self.low_cut)),
            Param::Reverb(ReverbParam::Width(self.width)),
            Param::Reverb(ReverbParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Reverb
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        // Resize buffers when sample rate changes (called from main thread, not audio thread)
        self.resize_for_sample_rate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverb_creation() {
        let reverb = Reverb::new();
        assert!((reverb.room_size.as_f32() - 0.5).abs() < 0.001);
        assert!((reverb.decay.as_f32() - 0.5).abs() < 0.001);
        assert!((reverb.diffusion.as_f32() - 0.5).abs() < 0.001);
        assert!((reverb.low_cut.as_f32() - 80.0).abs() < 0.1);
    }

    #[test]
    fn test_reverb_stability() {
        let mut reverb = Reverb::new();
        reverb.sample_rate = SampleRate::DVD_QUALITY;
        reverb.room_size = NormalizedValue::new(0.9);
        reverb.decay = NormalizedValue::new(0.9);
        reverb.resize_for_sample_rate();

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(256),
            ..Default::default()
        };

        let input = vec![1.0; 512];
        let mut output = vec![0.0; 512];

        for _ in 0..100 {
            reverb.process(&input, &mut output, &context);
        }

        for sample in &output {
            assert!(sample.is_finite(), "Reverb output is not finite");
            assert!(sample.abs() < 10.0, "Reverb output exploded");
        }
    }

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
    fn test_reverb_params() {
        let mut reverb = Reverb::new();

        // Test all param setters/getters
        reverb.set_param(Param::Reverb(ReverbParam::RoomSize(NormalizedValue::new(
            0.8,
        ))));
        assert!(
            (reverb
                .get_param(&Param::Reverb(ReverbParam::RoomSize(NormalizedValue::MIN)))
                .unwrap()
                - 0.8)
                .abs()
                < 0.001
        );

        reverb.set_param(Param::Reverb(ReverbParam::Decay(NormalizedValue::new(0.7))));
        assert!(
            (reverb
                .get_param(&Param::Reverb(ReverbParam::Decay(NormalizedValue::MIN)))
                .unwrap()
                - 0.7)
                .abs()
                < 0.001
        );

        reverb.set_param(Param::Reverb(ReverbParam::Diffusion(NormalizedValue::new(
            0.6,
        ))));
        assert!(
            (reverb
                .get_param(&Param::Reverb(ReverbParam::Diffusion(NormalizedValue::MIN)))
                .unwrap()
                - 0.6)
                .abs()
                < 0.001
        );

        reverb.set_param(Param::Reverb(ReverbParam::LowCut(Hertz::new(120.0))));
        assert!(
            (reverb
                .get_param(&Param::Reverb(ReverbParam::LowCut(Hertz::new(0.0))))
                .unwrap()
                - 120.0)
                .abs()
                < 0.1
        );
    }

    #[test]
    fn test_reverb_get_params_all_variants() {
        let reverb = Reverb::new();
        let params = reverb.get_params();
        // Should return all 8 params
        assert_eq!(params.len(), 8);
    }

    #[test]
    fn test_reverb_produces_output() {
        let mut reverb = Reverb::new();
        reverb.set_sample_rate(SampleRate::DVD_QUALITY);
        reverb.set_param(Param::Reverb(ReverbParam::Mix(NormalizedValue::MAX)));
        reverb.set_param(Param::Reverb(ReverbParam::Decay(NormalizedValue::new(0.8))));

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(256),
            ..Default::default()
        };

        // Feed an impulse
        let mut input = vec![0.0; 512];
        input[0] = 1.0;
        input[1] = 1.0;
        let mut output = vec![0.0; 512];

        reverb.process(&input, &mut output, &context);

        // Process enough silent blocks for signal to propagate through FDN delay lines.
        // Base delays are 2039-3511 samples (at 44.1kHz, room_size=0.5 -> scale 1.25,
        // so effective delays ~2549-4389). With 256 samples/block, the shortest delay
        // appears around block 10. Check across all blocks for any energy.
        let silent_input = vec![0.0; 512];
        let mut tail_output = vec![0.0; 512];
        let mut max_energy: f32 = 0.0;
        for _ in 0..30 {
            reverb.process(&silent_input, &mut tail_output, &context);
            let energy: f32 = tail_output.iter().map(|s| s * s).sum();
            if energy > max_energy {
                max_energy = energy;
            }
        }

        assert!(
            max_energy > 0.0001,
            "Reverb should have a tail after impulse, max energy: {max_energy}"
        );
    }
}
