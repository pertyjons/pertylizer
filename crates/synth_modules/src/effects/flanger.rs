//! Flanger effect with LFO-modulated delay.

use synth_core::{
    AudioEffect, Describable, ModuleCategory, ModuleDescriptor, ParameterDescriptor, ParameterUnit,
    PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{
    BipolarValue, BufferIndex, Hertz, Milliseconds, NormalizedValue, Phase, SampleRate,
};
use synth_core::{FlangerParam, ModuleType, Param};

/// Maximum delay time in milliseconds.
const MAX_DELAY_MS: f32 = 20.0;

/// Flanger effect with modulated delay line and feedback.
pub struct Flanger {
    // Parameters
    rate: Hertz,
    depth: NormalizedValue,
    feedback: BipolarValue,
    delay_base: Milliseconds, // Base delay time (type-safe)
    mix: NormalizedValue,

    // Delay buffers (stereo)
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: BufferIndex,

    // LFO state
    lfo_phase: Phase,

    // Feedback state
    feedback_l: f32,
    feedback_r: f32,

    // State
    sample_rate: SampleRate,
}

impl Flanger {
    pub fn new() -> Self {
        Self {
            rate: Hertz::new(0.3),
            depth: NormalizedValue::new(0.7),
            feedback: BipolarValue::new(0.7),
            delay_base: Milliseconds::new(2.0),
            mix: NormalizedValue::CENTER,
            buffer_l: vec![0.0; 4800], // Will be resized
            buffer_r: vec![0.0; 4800],
            write_pos: BufferIndex::ZERO,
            lfo_phase: Phase::ZERO,
            feedback_l: 0.0,
            feedback_r: 0.0,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    fn resize_buffers(&mut self) {
        let size = (MAX_DELAY_MS / 1000.0 * self.sample_rate.as_f32()) as usize + 1;
        if self.buffer_l.len() != size {
            self.buffer_l.resize(size, 0.0);
            self.buffer_r.resize(size, 0.0);
            self.write_pos = BufferIndex::ZERO;
        }
    }

    /// Read from delay buffer with linear interpolation.
    #[inline]
    fn read_interpolated(buffer: &[f32], write_pos: BufferIndex, delay_samples: f32) -> f32 {
        let len = buffer.len();
        let read_pos = (write_pos.as_usize() as f32 - delay_samples).rem_euclid(len as f32);
        let idx0 = read_pos as usize % len;
        let idx1 = (idx0 + 1) % len;
        let frac = read_pos - read_pos.floor();
        buffer[idx0] * (1.0 - frac) + buffer[idx1] * frac
    }
}

impl Default for Flanger {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Flanger {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("flanger", "Flanger")
            .description("Classic flanger effect with feedback")
            .category(ModuleCategory::Effect)
            .tag("flanger")
            .tag("effect")
            .tag("modulation")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            // Note: rate_cv port removed - AudioEffect trait doesn't support named CV inputs.
            // To add CV modulation, this effect would need to use PolyModule instead.
            .parameter(
                ParameterDescriptor::float(
                    Param::Flanger(FlangerParam::Rate(Hertz::new(0.3))),
                    "Rate",
                )
                .description("LFO rate")
                .range(0.05, 5.0)
                .default(0.3)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Flanger(FlangerParam::Depth(NormalizedValue::new(0.7))),
                    "Depth",
                )
                .description("Modulation depth")
                .range(0.0, 1.0)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Flanger(FlangerParam::Feedback(BipolarValue::new(0.7))),
                    "Feedback",
                )
                .description("Feedback amount")
                .range(-0.95, 0.95)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Flanger(FlangerParam::Delay(Milliseconds::new(2.0))),
                    "Delay",
                )
                .description("Base delay time")
                .range(0.1, 10.0)
                .default(2.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Flanger(FlangerParam::Mix(NormalizedValue::CENTER)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Flanger {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        // Note: sample_rate is set via set_sample_rate() which is called from main thread
        // We don't resize buffers here to avoid allocation in audio thread
        debug_assert_eq!(
            self.sample_rate, context.sample_rate,
            "Flanger sample rate mismatch - call set_sample_rate() before processing"
        );

        let phase_inc = self.rate.as_f32() / self.sample_rate.as_f32();
        let delay_ms = self.delay_base.as_f32();
        let max_mod_ms = delay_ms.min(MAX_DELAY_MS - delay_ms);

        // Process stereo interleaved
        let channels = 2;
        for frame in 0..context.samples.as_usize() {
            let idx_l = frame * channels;
            let idx_r = frame * channels + 1;

            let in_l = if idx_l < input.len() {
                input[idx_l]
            } else {
                0.0
            };
            let in_r = if idx_r < input.len() {
                input[idx_r]
            } else {
                in_l
            };

            // Calculate LFO value (triangle wave for smoother flanging)
            let phase = self.lfo_phase.as_f32();
            let lfo = if phase < 0.5 {
                phase * 4.0 - 1.0
            } else {
                3.0 - phase * 4.0
            };
            self.lfo_phase = Phase::new((phase + phase_inc).rem_euclid(1.0));

            // Calculate modulated delay time
            let depth = self.depth.as_f32();
            let mod_delay_ms = delay_ms + lfo * max_mod_ms * depth;
            let delay_samples = (mod_delay_ms / 1000.0 * self.sample_rate.as_f32()).max(1.0);

            // Write input + feedback to delay buffer
            let feedback = self.feedback.as_f32();
            let write_idx = self.write_pos.as_usize();
            self.buffer_l[write_idx] = in_l + self.feedback_l * feedback;
            self.buffer_r[write_idx] = in_r + self.feedback_r * feedback;

            // Read from delay with interpolation
            let wet_l = Self::read_interpolated(&self.buffer_l, self.write_pos, delay_samples);
            let wet_r = Self::read_interpolated(&self.buffer_r, self.write_pos, delay_samples);

            // Store for feedback
            self.feedback_l = wet_l;
            self.feedback_r = wet_r;

            // Advance write position
            self.write_pos = self.write_pos.advance(self.buffer_l.len());

            // Mix dry/wet
            let mix = self.mix.as_f32();
            if idx_l < output.len() {
                output[idx_l] = in_l * (1.0 - mix) + wet_l * mix;
            }
            if idx_r < output.len() {
                output[idx_r] = in_r * (1.0 - mix) + wet_r * mix;
            }
        }
    }

    fn reset(&mut self) {
        self.buffer_l.fill(0.0);
        self.buffer_r.fill(0.0);
        self.write_pos = BufferIndex::ZERO;
        self.feedback_l = 0.0;
        self.feedback_r = 0.0;
        self.lfo_phase = Phase::ZERO;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Flanger(flanger_param) = param {
            match flanger_param {
                FlangerParam::Rate(r) => {
                    self.rate = Hertz::new(r.as_f32().clamp(0.05, 5.0));
                }
                FlangerParam::Depth(d) => {
                    self.depth = d;
                }
                FlangerParam::Feedback(f) => {
                    self.feedback = BipolarValue::new(f.as_f32().clamp(-0.95, 0.95));
                }
                FlangerParam::Delay(d) => {
                    self.delay_base = Milliseconds::new(d.as_f32().clamp(0.1, 10.0));
                }
                FlangerParam::Mix(m) => {
                    self.mix = m;
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Flanger(flanger_param) = param {
            Some(match flanger_param {
                FlangerParam::Rate(_) => self.rate.as_f32(),
                FlangerParam::Depth(_) => self.depth.as_f32(),
                FlangerParam::Feedback(_) => self.feedback.as_f32(),
                FlangerParam::Delay(_) => self.delay_base.as_f32(),
                FlangerParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Flanger(FlangerParam::Rate(self.rate)),
            Param::Flanger(FlangerParam::Depth(self.depth)),
            Param::Flanger(FlangerParam::Feedback(self.feedback)),
            Param::Flanger(FlangerParam::Delay(self.delay_base)),
            Param::Flanger(FlangerParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Flanger
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        // Resize buffers when sample rate changes (called from main thread, not audio thread)
        self.resize_buffers();
    }
}
