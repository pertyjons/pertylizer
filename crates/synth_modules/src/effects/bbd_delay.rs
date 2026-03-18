//! BBD (Bucket Brigade Device) analog delay emulation.
//!
//! Features:
//! - Analog-style delay with bandwidth limitation
//! - Compander (tanh compression/expansion) for analog character
//! - Wow & flutter modulation via internal LFO
//! - Clock noise proportional to delay time
//! - Per-repeat darkening via lowpass in feedback path
//! - Maximum 1.0 second delay time

use synth_core::{
    AudioEffect, BbdDelayParam, BufferIndex, Describable, FilterState, Hertz, ModuleCategory,
    ModuleDescriptor, ModuleType, NormalizedValue, Param, ParameterDescriptor, ParameterUnit,
    ProcessContext, ResponseCurve, SampleCount, SampleRate, Seconds, StereoSample, WidgetHint,
};

/// Maximum delay time in seconds for the BBD delay.
const MAX_DELAY_SECONDS: f32 = 1.0;

/// BBD (Bucket Brigade Device) analog delay effect.
///
/// Emulates the character of analog bucket-brigade delay chips with
/// bandwidth limitation, companding, wow/flutter modulation, and clock noise.
pub struct BbdDelay {
    // Parameters
    time: Seconds,
    feedback: NormalizedValue,
    tone: NormalizedValue,
    wow_flutter: NormalizedValue,
    clock_noise: NormalizedValue,
    mix: NormalizedValue,

    // Delay buffers (pre-allocated)
    buffer_left: Vec<f32>,
    buffer_right: Vec<f32>,
    write_pos: usize,

    // Filter states for feedback path darkening
    feedback_filter_left: FilterState,
    feedback_filter_right: FilterState,

    // Input bandwidth limitation filters
    input_filter_left: FilterState,
    input_filter_right: FilterState,

    // Wow & flutter LFO phase (0.0 to 1.0)
    lfo_phase: f32,

    // Noise generator state (simple LCG)
    noise_state: u32,

    // State
    sample_rate: SampleRate,
}

impl BbdDelay {
    /// Create a new BBD delay effect with default parameters.
    #[must_use]
    pub fn new() -> Self {
        let sample_rate = SampleRate::DVD_QUALITY;
        let buffer_size = (MAX_DELAY_SECONDS * sample_rate.as_f32()) as usize + 1;

        Self {
            time: Seconds::new(0.3),
            feedback: NormalizedValue::new(0.5),
            tone: NormalizedValue::CENTER,
            wow_flutter: NormalizedValue::new(0.3),
            clock_noise: NormalizedValue::new(0.2),
            mix: NormalizedValue::CENTER,

            buffer_left: vec![0.0; buffer_size],
            buffer_right: vec![0.0; buffer_size],
            write_pos: 0,

            feedback_filter_left: FilterState::ZERO,
            feedback_filter_right: FilterState::ZERO,
            input_filter_left: FilterState::ZERO,
            input_filter_right: FilterState::ZERO,

            lfo_phase: 0.0,

            noise_state: 0x5EED_CAFE,

            sample_rate,
        }
    }

    /// Resize delay buffers for the current sample rate.
    fn resize_buffers(&mut self) {
        let size = (MAX_DELAY_SECONDS * self.sample_rate.as_f32()) as usize + 1;
        if self.buffer_left.len() != size {
            self.buffer_left.resize(size, 0.0);
            self.buffer_right.resize(size, 0.0);
            self.write_pos = 0;
        }
    }

    /// Compander: compress signal using tanh before entering delay.
    #[inline]
    fn compress(sample: f32) -> f32 {
        // Gentle tanh compression to emulate analog compander input stage
        (sample * 1.5).tanh()
    }

    /// Compander: expand signal after reading from delay.
    #[inline]
    fn expand(sample: f32) -> f32 {
        // Inverse-ish expansion to restore dynamics
        // atanh is undefined outside (-1,1), so we use a safe approximation
        let clamped = sample.clamp(-0.99, 0.99);
        clamped / (1.0 - clamped * clamped * 0.33)
    }

    /// Generate next pseudo-random noise sample using LCG.
    #[inline]
    fn next_noise(&mut self) -> f32 {
        // Linear congruential generator
        self.noise_state = self
            .noise_state
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        #[allow(clippy::cast_precision_loss)]
        let normalized = (self.noise_state >> 1) as f32 / (u32::MAX >> 1) as f32;
        normalized * 2.0 - 1.0
    }

    /// One-pole lowpass filter for feedback/tone path.
    #[inline]
    fn lowpass(input: f32, state: &mut FilterState, cutoff_hz: f32, sample_rate: f32) -> f32 {
        let coeff = Hertz::new(cutoff_hz).to_exp_coeff(SampleRate::new(sample_rate));
        state.one_pole(input, coeff)
    }

    /// Calculate the BBD bandwidth cutoff frequency.
    ///
    /// Real BBD chips have bandwidth that decreases with delay time.
    /// Short delays: ~10kHz, long delays: ~3kHz.
    #[inline]
    fn bbd_bandwidth_hz(&self) -> f32 {
        let time_norm = (self.time.as_f32() - 0.01) / (MAX_DELAY_SECONDS - 0.01);
        let time_norm = time_norm.clamp(0.0, 1.0);
        // Interpolate between 10kHz (short delay) and 3kHz (long delay)
        10_000.0 * (1.0 - time_norm) + 3_000.0 * time_norm
    }

    /// Calculate the feedback tone cutoff frequency from the tone parameter.
    #[inline]
    fn feedback_cutoff_hz(&self) -> f32 {
        crate::math::exponential_frequency_map(self.tone.as_f32(), 800.0, 12_000.0)
    }
}

impl Default for BbdDelay {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for BbdDelay {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("bbd_delay", "BBD Delay")
            .description("Analog bucket-brigade delay emulation")
            .category(ModuleCategory::Effect)
            .tag("delay")
            .tag("effect")
            .tag("analog")
            .tag("time")
            .parameter(
                ParameterDescriptor::float(
                    "time",
                    Param::BbdDelay(BbdDelayParam::Time(Seconds::new(0.3))),
                    "Time",
                )
                .description("Delay time")
                .range(0.01, MAX_DELAY_SECONDS)
                .default(0.3)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::TimeSlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "feedback",
                    Param::BbdDelay(BbdDelayParam::Feedback(NormalizedValue::new(0.5))),
                    "Feedback",
                )
                .description("Feedback amount")
                .range(0.0, 0.95)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "tone",
                    Param::BbdDelay(BbdDelayParam::Tone(NormalizedValue::CENTER)),
                    "Tone",
                )
                .description("Brightness of repeats")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "wow_flutter",
                    Param::BbdDelay(BbdDelayParam::WowFlutter(NormalizedValue::new(0.3))),
                    "Wow/Flutter",
                )
                .description("Analog modulation depth")
                .range(0.0, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "clock_noise",
                    Param::BbdDelay(BbdDelayParam::ClockNoise(NormalizedValue::new(0.2))),
                    "Clock Noise",
                )
                .description("BBD clock noise amount")
                .range(0.0, 1.0)
                .default(0.2)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::BbdDelay(BbdDelayParam::Mix(NormalizedValue::CENTER)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for BbdDelay {
    #[allow(clippy::too_many_lines)]
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        debug_assert_eq!(
            self.sample_rate, context.sample_rate,
            "BbdDelay sample rate mismatch - call set_sample_rate() before processing"
        );

        let sr = self.sample_rate.as_f32();
        let len = self.buffer_left.len();

        // Base delay in samples
        let base_delay_samples = (self.time.as_f32() * sr).min((len - 1) as f32);

        let feedback = self.feedback.as_f32();
        let mix = self.mix.as_f32();
        let wow_flutter_depth = self.wow_flutter.as_f32();
        let clock_noise_amount = self.clock_noise.as_f32();

        // BBD bandwidth and feedback tone cutoff
        let bandwidth_hz = self.bbd_bandwidth_hz();
        let feedback_cutoff = self.feedback_cutoff_hz();

        // Wow/flutter LFO rate: 0.3-0.5 Hz (slow, analog-style drift)
        let lfo_rate = 0.3 + wow_flutter_depth * 0.2;
        let lfo_increment = lfo_rate / sr;

        // Wow/flutter max depth in samples (~0.5ms at full depth)
        let wow_depth_samples = 0.0005 * sr * wow_flutter_depth;

        // Clock noise scales with delay time (longer = noisier, like real BBD)
        let noise_scale = clock_noise_amount * 0.003 * (self.time.as_f32() / MAX_DELAY_SECONDS);

        // Process interleaved stereo
        for frame in 0..context.samples.as_usize() {
            // Read stereo input
            let dry = StereoSample::read_frame(input, frame);

            // Step 1: Apply compander compression to input
            let compressed = StereoSample::new(Self::compress(dry.left), Self::compress(dry.right));

            // Step 2: Apply input bandwidth limitation (BBD anti-aliasing)
            let bandwidth_limited = StereoSample::new(
                Self::lowpass(
                    compressed.left,
                    &mut self.input_filter_left,
                    bandwidth_hz,
                    sr,
                ),
                Self::lowpass(
                    compressed.right,
                    &mut self.input_filter_right,
                    bandwidth_hz,
                    sr,
                ),
            );

            // Step 3: Calculate wow/flutter modulated delay time
            // Use sine LFO for smooth analog-style modulation
            let lfo_value = (self.lfo_phase * std::f32::consts::TAU).sin();
            self.lfo_phase = (self.lfo_phase + lfo_increment).rem_euclid(1.0);

            let modulated_delay =
                (base_delay_samples + lfo_value * wow_depth_samples).clamp(1.0, (len - 1) as f32);

            // Step 4: Read from delay buffers with interpolation
            let delayed = StereoSample::new(
                BufferIndex::new(self.write_pos)
                    .read_interpolated(&self.buffer_left, modulated_delay),
                BufferIndex::new(self.write_pos)
                    .read_interpolated(&self.buffer_right, modulated_delay),
            );

            // Step 5: Apply lowpass in feedback path (tone control / per-repeat darkening)
            let filtered_fb = StereoSample::new(
                Self::lowpass(
                    delayed.left,
                    &mut self.feedback_filter_left,
                    feedback_cutoff,
                    sr,
                ),
                Self::lowpass(
                    delayed.right,
                    &mut self.feedback_filter_right,
                    feedback_cutoff,
                    sr,
                ),
            );

            // Step 6: Add subtle clock noise
            let noise_l = self.next_noise() * noise_scale;
            let noise_r = self.next_noise() * noise_scale;
            let noisy_fb =
                StereoSample::new(filtered_fb.left + noise_l, filtered_fb.right + noise_r);

            // Step 7: Write to delay buffer (input + feedback)
            let write_signal = StereoSample::new(
                bandwidth_limited.left + noisy_fb.left * feedback,
                bandwidth_limited.right + noisy_fb.right * feedback,
            );

            // Soft limit to prevent runaway feedback
            let write_signal = write_signal.soft_clip();

            self.buffer_left[self.write_pos] = write_signal.left;
            self.buffer_right[self.write_pos] = write_signal.right;

            // Advance write position
            self.write_pos = (self.write_pos + 1) % len;

            // Step 8: Apply compander expansion to delayed signal
            let expanded =
                StereoSample::new(Self::expand(delayed.left), Self::expand(delayed.right));

            // Step 9: Mix dry/wet
            let wet = dry.blend(expanded, mix);

            StereoSample::write_frame(output, frame, wet);
        }
    }

    fn reset(&mut self) {
        self.buffer_left.fill(0.0);
        self.buffer_right.fill(0.0);
        self.write_pos = 0;
        self.feedback_filter_left = FilterState::ZERO;
        self.feedback_filter_right = FilterState::ZERO;
        self.input_filter_left = FilterState::ZERO;
        self.input_filter_right = FilterState::ZERO;
        self.lfo_phase = 0.0;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn tail_samples(&self) -> SampleCount {
        // Estimate tail based on delay time and feedback
        // More feedback = longer tail
        let fb = self.feedback.as_f32();
        let decay_time = if fb < 0.95 {
            crate::math::feedback_decay_time(self.time.as_f32(), fb)
        } else {
            self.time.as_f32() * 20.0
        };
        #[allow(clippy::cast_possible_truncation)]
        let samples = (decay_time * self.sample_rate.as_f32()) as usize;
        SampleCount::new(samples)
    }

    fn set_param(&mut self, param: Param) {
        if let Param::BbdDelay(bbd_param) = param {
            match bbd_param {
                BbdDelayParam::Time(t) => {
                    self.time = Seconds::new(t.as_f32().clamp(0.01, MAX_DELAY_SECONDS));
                }
                BbdDelayParam::Feedback(f) => {
                    self.feedback = NormalizedValue::new(f.as_f32().clamp(0.0, 0.95));
                }
                BbdDelayParam::Tone(t) => {
                    self.tone = NormalizedValue::new(t.as_f32().clamp(0.0, 1.0));
                }
                BbdDelayParam::WowFlutter(w) => {
                    self.wow_flutter = NormalizedValue::new(w.as_f32().clamp(0.0, 1.0));
                }
                BbdDelayParam::ClockNoise(n) => {
                    self.clock_noise = NormalizedValue::new(n.as_f32().clamp(0.0, 1.0));
                }
                BbdDelayParam::Mix(m) => self.mix = m,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::BbdDelay(bbd_param) = param {
            Some(match bbd_param {
                BbdDelayParam::Time(_) => self.time.as_f32(),
                BbdDelayParam::Feedback(_) => self.feedback.as_f32(),
                BbdDelayParam::Tone(_) => self.tone.as_f32(),
                BbdDelayParam::WowFlutter(_) => self.wow_flutter.as_f32(),
                BbdDelayParam::ClockNoise(_) => self.clock_noise.as_f32(),
                BbdDelayParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::BbdDelay(BbdDelayParam::Time(self.time)),
            Param::BbdDelay(BbdDelayParam::Feedback(self.feedback)),
            Param::BbdDelay(BbdDelayParam::Tone(self.tone)),
            Param::BbdDelay(BbdDelayParam::WowFlutter(self.wow_flutter)),
            Param::BbdDelay(BbdDelayParam::ClockNoise(self.clock_noise)),
            Param::BbdDelay(BbdDelayParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::BbdDelay
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        // Resize buffers when sample rate changes (called from main thread, not audio thread)
        self.resize_buffers();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bbd_delay_creation() {
        let bbd = BbdDelay::new();
        assert!((bbd.time.as_f32() - 0.3).abs() < 0.001);
        assert!((bbd.feedback.as_f32() - 0.5).abs() < 0.001);
        assert!((bbd.tone.as_f32() - 0.5).abs() < 0.001);
        assert!((bbd.wow_flutter.as_f32() - 0.3).abs() < 0.001);
        assert!((bbd.clock_noise.as_f32() - 0.2).abs() < 0.001);
        assert!((bbd.mix.as_f32() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_bbd_delay_default() {
        let bbd = BbdDelay::default();
        assert!((bbd.time.as_f32() - 0.3).abs() < 0.001);
    }

    #[test]
    fn test_bbd_delay_no_explosion() {
        let mut bbd = BbdDelay::new();
        bbd.set_sample_rate(SampleRate::DVD_QUALITY);
        bbd.feedback = NormalizedValue::new(0.9);

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(256),
            ..Default::default()
        };

        let input = vec![1.0; 512];
        let mut output = vec![0.0; 512];

        for _ in 0..100 {
            bbd.process(&input, &mut output, &context);
        }

        for sample in &output {
            assert!(sample.is_finite(), "BBD delay output is not finite");
            assert!(sample.abs() < 10.0, "BBD delay output exploded");
        }
    }

    #[test]
    fn test_bbd_delay_silence_in_silence_out() {
        let mut bbd = BbdDelay::new();
        bbd.set_sample_rate(SampleRate::DVD_QUALITY);

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(256),
            ..Default::default()
        };

        let input = vec![0.0; 512];
        let mut output = vec![0.0; 512];

        bbd.process(&input, &mut output, &context);

        for sample in &output {
            assert!(
                sample.abs() < 0.001,
                "BBD delay should be near-silent with zero input"
            );
        }
    }

    #[test]
    fn test_bbd_delay_set_get_params() {
        let mut bbd = BbdDelay::new();

        bbd.set_param(Param::BbdDelay(BbdDelayParam::Time(Seconds::new(0.5))));
        assert!(
            (bbd.get_param(&Param::BbdDelay(BbdDelayParam::Time(Seconds::new(0.0))))
                .unwrap()
                - 0.5)
                .abs()
                < 0.001
        );

        bbd.set_param(Param::BbdDelay(BbdDelayParam::Feedback(
            NormalizedValue::new(0.7),
        )));
        assert!(
            (bbd.get_param(&Param::BbdDelay(BbdDelayParam::Feedback(
                NormalizedValue::MIN
            )))
            .unwrap()
                - 0.7)
                .abs()
                < 0.001
        );

        bbd.set_param(Param::BbdDelay(BbdDelayParam::Tone(NormalizedValue::new(
            0.8,
        ))));
        assert!(
            (bbd.get_param(&Param::BbdDelay(BbdDelayParam::Tone(NormalizedValue::MIN)))
                .unwrap()
                - 0.8)
                .abs()
                < 0.001
        );
    }

    #[test]
    fn test_bbd_delay_get_all_params() {
        let bbd = BbdDelay::new();
        let params = bbd.get_params();
        assert_eq!(params.len(), 6);
    }

    #[test]
    fn test_bbd_delay_module_type() {
        let bbd = BbdDelay::new();
        assert_eq!(bbd.module_type(), ModuleType::BbdDelay);
    }

    #[test]
    fn test_bbd_delay_reset() {
        let mut bbd = BbdDelay::new();
        bbd.set_sample_rate(SampleRate::DVD_QUALITY);

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(256),
            ..Default::default()
        };

        // Feed some signal
        let input = vec![0.5; 512];
        let mut output = vec![0.0; 512];
        bbd.process(&input, &mut output, &context);

        // Reset and verify buffers are cleared
        bbd.reset();
        assert_eq!(bbd.write_pos, 0);
        assert!(bbd.buffer_left.iter().all(|&s| s == 0.0));
        assert!(bbd.buffer_right.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn test_bbd_delay_feedback_clamping() {
        let mut bbd = BbdDelay::new();

        // Feedback should be clamped to 0.95
        bbd.set_param(Param::BbdDelay(BbdDelayParam::Feedback(
            NormalizedValue::new(1.0),
        )));
        assert!(
            (bbd.get_param(&Param::BbdDelay(BbdDelayParam::Feedback(
                NormalizedValue::MIN
            )))
            .unwrap()
                - 0.95)
                .abs()
                < 0.001
        );
    }

    #[test]
    fn test_bbd_delay_time_clamping() {
        let mut bbd = BbdDelay::new();

        // Time below minimum should be clamped
        bbd.set_param(Param::BbdDelay(BbdDelayParam::Time(Seconds::new(0.001))));
        assert!(bbd.time.as_f32() >= 0.01);

        // Time above maximum should be clamped
        bbd.set_param(Param::BbdDelay(BbdDelayParam::Time(Seconds::new(5.0))));
        assert!(bbd.time.as_f32() <= MAX_DELAY_SECONDS);
    }
}
