//! Reverb effect module.
//!
//! Features:
//! - 8-channel Feedback Delay Network (FDN) reverb via FdnCore
//! - Hadamard mixing matrix for dense reflections
//! - Per-channel damping (lowpass) and low-cut (highpass) filters
//! - Modulated delay times for diffusion
//! - Pre-delay up to 500ms
//! - Stereo output with width control

use synth_core::{
    AudioEffect, Describable, Gain, Hertz, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, ParameterUnit, ProcessContext, ReverbParam,
    SampleCount, SampleRate, Seconds, StereoSample, WidgetHint,
};
use synth_dsp::FdnCore;

/// Maximum pre-delay in seconds.
const MAX_PRE_DELAY_SECS: f32 = 0.5;

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

    // FDN core
    core: FdnCore,

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

            core: FdnCore::new(),

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

        self.core.set_delay_times(scale, room_scale);
    }

    /// Scale all buffers for a new sample rate.
    fn resize_for_sample_rate(&mut self) {
        let scale = self.sample_rate.as_f32() / 44100.0;
        let room_scale = 0.5 + self.room_size.as_f32() * 1.5;

        self.core.set_delay_times(scale, room_scale);

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
    fn feedback_gain(&self) -> Gain {
        Gain::new(0.3 + self.decay.as_f32() * 0.67)
    }

    /// Compute the lowpass coefficient from the damping parameter.
    ///
    /// Higher damping = more high-frequency absorption.
    /// coeff near 1.0 = heavy filtering, coeff near 0.0 = no filtering.
    #[inline]
    fn lowpass_coeff(&self) -> NormalizedValue {
        NormalizedValue::new(self.damping.as_f32() * 0.9)
    }

    /// Compute the highpass coefficient from the low-cut frequency.
    ///
    /// Simple one-pole highpass: coeff = 1 - (2*pi*fc / sr)
    #[inline]
    fn highpass_coeff(&self) -> NormalizedValue {
        let fc = self.low_cut.as_f32();
        let sr = self.sample_rate.as_f32();
        // One-pole highpass coefficient
        // coeff close to 1.0 = low cutoff, coeff close to 0.0 = high cutoff
        NormalizedValue::new((1.0 - (std::f32::consts::TAU * fc / sr)).clamp(0.0, 0.9999))
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
                    "room_size",
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
                    "decay",
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
                    "damping",
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
                    "diffusion",
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
                    "pre_delay",
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
                    "low_cut",
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
                    "width",
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
                    "mix",
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
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        debug_assert!(
            (self.sample_rate.as_f32() - context.sample_rate.as_f32()).abs() <= 1.0,
            "Reverb sample rate mismatch - call set_sample_rate() before processing"
        );

        self.update_delay_sizes();

        let feedback_gain = self.feedback_gain();
        let lp_coeff = self.lowpass_coeff();
        let hp_coeff = self.highpass_coeff();
        let diffusion = self.diffusion;
        let mix = self.mix.as_f32();
        let width = self.width;
        let sample_rate_recip = 1.0 / self.sample_rate.as_f32();

        #[allow(clippy::cast_possible_truncation)]
        let pre_delay_samples = ((self.pre_delay.as_f32() * self.sample_rate.as_f32()) as usize)
            .min(self.pre_delay_buffer.len() - 1);

        for frame in 0..context.samples.as_usize() {
            // Get input as stereo sample
            let dry = StereoSample::read_frame(input, frame);

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

            // --- FDN Processing (delegated to FdnCore) ---
            let wet = self.core.process_sample(
                pre_delayed,
                feedback_gain,
                lp_coeff,
                hp_coeff,
                diffusion,
                width,
                sample_rate_recip,
            );

            // Mix dry/wet
            let result = dry.blend(wet.into(), mix);

            StereoSample::write_frame(output, frame, result);
        }
    }

    fn reset(&mut self) {
        self.core.clear();
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
