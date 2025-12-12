//! Reverb effect module.
//!
//! Features:
//! - Schroeder reverb algorithm
//! - Adjustable room size and damping
//! - Pre-delay
//! - Stereo spread

use synth_core::{
    AudioEffect, Describable, FilterState, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, ParameterUnit, PortDescriptor, ProcessContext,
    ReverbParam, SampleCount, SampleRate, Seconds, StereoSample, WidgetHint,
};

/// Comb filter for reverb.
struct CombFilter {
    buffer: Vec<f32>,
    index: usize,
    feedback: f32,
    damp: f32,
    filter_state: FilterState,
}

impl CombFilter {
    fn new(size: usize, feedback: f32, damp: f32) -> Self {
        Self {
            buffer: vec![0.0; size],
            index: 0,
            feedback,
            damp,
            filter_state: FilterState::ZERO,
        }
    }

    fn set_feedback(&mut self, feedback: f32) {
        self.feedback = feedback;
    }

    fn set_damp(&mut self, damp: f32) {
        self.damp = damp;
    }

    fn resize(&mut self, new_size: usize) {
        if self.buffer.len() != new_size {
            self.buffer.resize(new_size, 0.0);
            self.index = 0;
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.index];

        // Lowpass filter in feedback path for damping
        let new_state = output * (1.0 - self.damp) + self.filter_state.as_f32() * self.damp;
        self.filter_state = FilterState::new(new_state);

        self.buffer[self.index] = input + self.filter_state.as_f32() * self.feedback;

        self.index = (self.index + 1) % self.buffer.len();

        output
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.filter_state = FilterState::ZERO;
    }
}

/// Allpass filter for reverb.
struct AllpassFilter {
    buffer: Vec<f32>,
    index: usize,
    feedback: f32,
}

impl AllpassFilter {
    fn new(size: usize, feedback: f32) -> Self {
        Self {
            buffer: vec![0.0; size],
            index: 0,
            feedback,
        }
    }

    fn resize(&mut self, new_size: usize) {
        if self.buffer.len() != new_size {
            self.buffer.resize(new_size, 0.0);
            self.index = 0;
        }
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let buffered = self.buffer[self.index];
        let output = -input + buffered;

        self.buffer[self.index] = input + buffered * self.feedback;

        self.index = (self.index + 1) % self.buffer.len();

        output
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
    }
}

/// Schroeder reverb.
pub struct Reverb {
    // Parameters
    room_size: NormalizedValue,
    damping: NormalizedValue,
    mix: NormalizedValue,
    pre_delay: Seconds,
    stereo_spread: NormalizedValue,

    // Comb filters (8 for stereo)
    combs_l: [CombFilter; 4],
    combs_r: [CombFilter; 4],

    // Allpass filters (4 for stereo)
    allpasses_l: [AllpassFilter; 2],
    allpasses_r: [AllpassFilter; 2],

    // Pre-delay line
    pre_delay_buffer: Vec<f32>,
    pre_delay_index: usize,

    // State
    sample_rate: SampleRate,
}

impl Reverb {
    // Comb filter delay times (in samples at 44100 Hz)
    const COMB_TUNING: [usize; 4] = [1116, 1188, 1277, 1356];
    // Allpass filter delay times
    const ALLPASS_TUNING: [usize; 2] = [556, 441];
    // Stereo spread (in samples)
    const STEREO_SPREAD: usize = 23;

    pub fn new() -> Self {
        let feedback = 0.7;
        let damp = 0.5;
        let allpass_fb = 0.5;

        Self {
            room_size: NormalizedValue::CENTER,
            damping: NormalizedValue::CENTER,
            mix: NormalizedValue::new(0.3),
            pre_delay: Seconds::ZERO,
            stereo_spread: NormalizedValue::MAX,

            combs_l: [
                CombFilter::new(Self::COMB_TUNING[0], feedback, damp),
                CombFilter::new(Self::COMB_TUNING[1], feedback, damp),
                CombFilter::new(Self::COMB_TUNING[2], feedback, damp),
                CombFilter::new(Self::COMB_TUNING[3], feedback, damp),
            ],
            combs_r: [
                CombFilter::new(Self::COMB_TUNING[0] + Self::STEREO_SPREAD, feedback, damp),
                CombFilter::new(Self::COMB_TUNING[1] + Self::STEREO_SPREAD, feedback, damp),
                CombFilter::new(Self::COMB_TUNING[2] + Self::STEREO_SPREAD, feedback, damp),
                CombFilter::new(Self::COMB_TUNING[3] + Self::STEREO_SPREAD, feedback, damp),
            ],
            allpasses_l: [
                AllpassFilter::new(Self::ALLPASS_TUNING[0], allpass_fb),
                AllpassFilter::new(Self::ALLPASS_TUNING[1], allpass_fb),
            ],
            allpasses_r: [
                AllpassFilter::new(Self::ALLPASS_TUNING[0] + Self::STEREO_SPREAD, allpass_fb),
                AllpassFilter::new(Self::ALLPASS_TUNING[1] + Self::STEREO_SPREAD, allpass_fb),
            ],

            pre_delay_buffer: vec![0.0; 48000], // 1 second max
            pre_delay_index: 0,

            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Update filter parameters based on room size and damping.
    fn update_filters(&mut self) {
        let feedback = 0.4 + self.room_size.as_f32() * 0.55;

        for comb in &mut self.combs_l {
            comb.set_feedback(feedback);
            comb.set_damp(self.damping.as_f32());
        }
        for comb in &mut self.combs_r {
            comb.set_feedback(feedback);
            comb.set_damp(self.damping.as_f32());
        }
    }

    /// Scale filter sizes for sample rate.
    fn resize_for_sample_rate(&mut self) {
        let scale = self.sample_rate.as_f32() / 44100.0;

        for (i, comb) in self.combs_l.iter_mut().enumerate() {
            comb.resize((Self::COMB_TUNING[i] as f32 * scale) as usize);
        }
        for (i, comb) in self.combs_r.iter_mut().enumerate() {
            let spread =
                (Self::STEREO_SPREAD as f32 * self.stereo_spread.as_f32() * scale) as usize;
            comb.resize((Self::COMB_TUNING[i] as f32 * scale) as usize + spread);
        }
        for (i, ap) in self.allpasses_l.iter_mut().enumerate() {
            ap.resize((Self::ALLPASS_TUNING[i] as f32 * scale) as usize);
        }
        for (i, ap) in self.allpasses_r.iter_mut().enumerate() {
            let spread =
                (Self::STEREO_SPREAD as f32 * self.stereo_spread.as_f32() * scale) as usize;
            ap.resize((Self::ALLPASS_TUNING[i] as f32 * scale) as usize + spread);
        }

        // Pre-delay buffer
        let max_pre_delay = (0.5 * self.sample_rate.as_f32()) as usize;
        if self.pre_delay_buffer.len() != max_pre_delay {
            self.pre_delay_buffer.resize(max_pre_delay, 0.0);
            self.pre_delay_index = 0;
        }
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
            .description("Schroeder reverb with stereo spread")
            .category(ModuleCategory::Effect)
            .tag("reverb")
            .tag("effect")
            .tag("space")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::RoomSize(NormalizedValue::CENTER)),
                    "Room Size",
                )
                .description("Size of the virtual room")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::Damping(NormalizedValue::CENTER)),
                    "Damping",
                )
                .description("High frequency absorption")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Reverb(ReverbParam::PreDelay(Seconds::ZERO)),
                    "Pre-Delay",
                )
                .description("Initial delay before reverb")
                .range(0.0, 0.5)
                .default(0.0)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::TimeSlider),
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
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        if (self.sample_rate.as_f32() - context.sample_rate.as_f32()).abs() > 1.0 {
            self.sample_rate = context.sample_rate;
            self.resize_for_sample_rate();
        }

        self.update_filters();

        let pre_delay_samples = ((self.pre_delay.as_f32() * self.sample_rate.as_f32()) as usize)
            .min(self.pre_delay_buffer.len() - 1);

        let mix = self.mix.as_f32();

        // Process assuming interleaved stereo
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

            // Pre-delay (works on mono sum)
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

            // Sum parallel comb filters
            let mut wet = StereoSample::ZERO;

            for comb in &mut self.combs_l {
                wet.left += comb.process(pre_delayed);
            }
            for comb in &mut self.combs_r {
                wet.right += comb.process(pre_delayed);
            }

            // Normalize
            wet *= 0.25;

            // Series allpass filters
            for ap in &mut self.allpasses_l {
                wet.left = ap.process(wet.left);
            }
            for ap in &mut self.allpasses_r {
                wet.right = ap.process(wet.right);
            }

            // Mix dry/wet
            let result = StereoSample::new(
                dry.left * (1.0 - mix) + wet.left * mix,
                dry.right * (1.0 - mix) + wet.right * mix,
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
        for comb in &mut self.combs_l {
            comb.clear();
        }
        for comb in &mut self.combs_r {
            comb.clear();
        }
        for ap in &mut self.allpasses_l {
            ap.clear();
        }
        for ap in &mut self.allpasses_r {
            ap.clear();
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
        let decay_time = 1.0 + self.room_size.as_f32() * 4.0;
        SampleCount::new((decay_time * self.sample_rate.as_f32()) as usize)
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Reverb(reverb_param) = param {
            match reverb_param {
                ReverbParam::RoomSize(s) => self.room_size = s,
                ReverbParam::Damping(d) => self.damping = d,
                ReverbParam::PreDelay(p) => {
                    self.pre_delay = Seconds::new(p.as_f32().clamp(0.0, 0.5))
                }
                ReverbParam::Width(w) => self.stereo_spread = w,
                ReverbParam::Mix(m) => self.mix = m,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Reverb(reverb_param) = param {
            Some(match reverb_param {
                ReverbParam::RoomSize(_) => self.room_size.as_f32(),
                ReverbParam::Damping(_) => self.damping.as_f32(),
                ReverbParam::PreDelay(_) => self.pre_delay.as_f32(),
                ReverbParam::Width(_) => self.stereo_spread.as_f32(),
                ReverbParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Reverb(ReverbParam::RoomSize(self.room_size)),
            Param::Reverb(ReverbParam::PreDelay(self.pre_delay)),
            Param::Reverb(ReverbParam::Damping(self.damping)),
            Param::Reverb(ReverbParam::Width(self.stereo_spread)),
            Param::Reverb(ReverbParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Reverb
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverb_creation() {
        let reverb = Reverb::new();
        assert!((reverb.room_size.as_f32() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_reverb_stability() {
        let mut reverb = Reverb::new();
        reverb.sample_rate = SampleRate::DVD_QUALITY;
        reverb.room_size = NormalizedValue::new(0.9);
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
}
