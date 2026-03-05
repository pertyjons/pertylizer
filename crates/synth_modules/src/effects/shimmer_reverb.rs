//! Shimmer reverb effect.
//!
//! Features:
//! - FDN reverb core with pitch-shifted feedback
//! - Granular ring-buffer pitch shifter (RT-safe, no STFT)
//! - Configurable pitch interval in semitones
//! - Pre-delay up to 500ms

use synth_core::{
    AudioEffect, Describable, Gain, ModuleCategory, ModuleDescriptor, ModuleType, NormalizedValue,
    Param, ParameterDescriptor, ParameterUnit, ProcessContext, SampleCount, SampleRate, Seconds,
    Semitones, ShimmerReverbParam, StereoSample, WidgetHint,
};
use synth_dsp::FdnCore;

const MAX_PRE_DELAY_SECS: f32 = 0.5;
/// Pitch shifter grain size in samples (at 48 kHz ~ 21ms).
const SHIFT_GRAIN_SIZE: usize = 1024;
/// Pitch shifter buffer (must be >= 2 * SHIFT_GRAIN_SIZE).
const SHIFT_BUF_SIZE: usize = 4096;

/// Simple granular pitch shifter for the shimmer feedback path.
/// Uses two overlapping grains with Hann crossfade for glitch-free shifting.
struct GrainShifter {
    buffer: Vec<f32>,
    write_pos: usize,
    read_pos: f32,
    ratio: f32,
}

impl GrainShifter {
    fn new() -> Self {
        Self {
            buffer: vec![0.0; SHIFT_BUF_SIZE],
            write_pos: 0,
            read_pos: 0.0,
            ratio: 2.0,
        }
    }

    fn set_ratio(&mut self, ratio: f32) {
        self.ratio = ratio.clamp(0.25, 4.0);
    }

    #[inline]
    fn process(&mut self, input: f32) -> f32 {
        let len = self.buffer.len();
        self.buffer[self.write_pos] = input;
        self.write_pos = (self.write_pos + 1) % len;

        // Two overlapping read heads
        let grain = SHIFT_GRAIN_SIZE as f32;
        let pos1 = self.read_pos;
        let pos2 = (self.read_pos + grain * 0.5).rem_euclid(len as f32);

        // Hann crossfade based on position within grain
        let phase1 = (pos1.rem_euclid(grain)) / grain;
        let phase2 = (pos2.rem_euclid(grain)) / grain;
        let w1 = 0.5 * (1.0 - (phase1 * std::f32::consts::TAU).cos());
        let w2 = 0.5 * (1.0 - (phase2 * std::f32::consts::TAU).cos());

        let s1 = Self::read_lerp(&self.buffer, pos1);
        let s2 = Self::read_lerp(&self.buffer, pos2);

        // Advance read position
        self.read_pos = (self.read_pos + self.ratio).rem_euclid(len as f32);

        let total_w = w1 + w2;
        if total_w > 0.001 {
            (s1 * w1 + s2 * w2) / total_w
        } else {
            0.0
        }
    }

    #[inline]
    fn read_lerp(buffer: &[f32], pos: f32) -> f32 {
        let len = buffer.len();
        let wrapped = pos.rem_euclid(len as f32);
        let idx0 = (wrapped as usize) % len;
        let idx1 = (idx0 + 1) % len;
        let frac = wrapped - wrapped.floor();
        buffer[idx0] * (1.0 - frac) + buffer[idx1] * frac
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
        self.read_pos = 0.0;
    }
}

/// Shimmer reverb with pitch-shifted feedback.
pub struct ShimmerReverb {
    // Parameters
    room_size: NormalizedValue,
    decay: NormalizedValue,
    damping: NormalizedValue,
    pre_delay: Seconds,
    pitch_semitones: Semitones,
    shimmer_mix: NormalizedValue,
    mix: NormalizedValue,

    // FDN core
    core: FdnCore,

    // Pre-delay
    pre_delay_buffer: Vec<f32>,
    pre_delay_index: usize,

    // Pitch shifter (mono, applied in feedback path)
    shifter: GrainShifter,

    // Feedback accumulator
    feedback_acc: f32,

    sample_rate: SampleRate,
    params_dirty: bool,
}

impl ShimmerReverb {
    pub fn new() -> Self {
        Self {
            room_size: NormalizedValue::new(0.6),
            decay: NormalizedValue::new(0.7),
            damping: NormalizedValue::new(0.4),
            pre_delay: Seconds::ZERO,
            pitch_semitones: Semitones::new(12.0),
            shimmer_mix: NormalizedValue::new(0.35),
            mix: NormalizedValue::new(0.4),

            core: FdnCore::new(),

            pre_delay_buffer: vec![0.0; 48000],
            pre_delay_index: 0,

            shifter: GrainShifter::new(),
            feedback_acc: 0.0,

            sample_rate: SampleRate::DVD_QUALITY,
            params_dirty: true,
        }
    }

    fn update_params(&mut self) {
        if !self.params_dirty {
            return;
        }
        self.params_dirty = false;

        let scale = self.sample_rate.as_f32() / 44100.0;
        let room_scale = 0.5 + self.room_size.as_f32() * 1.5;
        self.core.set_delay_times(scale, room_scale);

        self.shifter.set_ratio(self.pitch_semitones.to_ratio());
    }

    fn resize_for_sample_rate(&mut self) {
        let scale = self.sample_rate.as_f32() / 44100.0;
        let room_scale = 0.5 + self.room_size.as_f32() * 1.5;
        self.core.set_delay_times(scale, room_scale);

        #[allow(clippy::cast_possible_truncation)]
        let max_pre = (MAX_PRE_DELAY_SECS * self.sample_rate.as_f32()) as usize;
        let max_pre = max_pre.max(1);
        if self.pre_delay_buffer.len() != max_pre {
            self.pre_delay_buffer.resize(max_pre, 0.0);
            self.pre_delay_index = 0;
        }
    }

    #[inline]
    fn feedback_gain(&self) -> Gain {
        Gain::new(0.3 + self.decay.as_f32() * 0.67)
    }

    #[inline]
    fn lowpass_coeff(&self) -> NormalizedValue {
        NormalizedValue::new(self.damping.as_f32() * 0.9)
    }
}

impl Default for ShimmerReverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for ShimmerReverb {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("shimmer_reverb", "Shimmer Reverb")
            .description("FDN reverb with pitch-shifted feedback")
            .category(ModuleCategory::Effect)
            .tag("reverb")
            .tag("effect")
            .tag("shimmer")
            .parameter(
                ParameterDescriptor::float(
                    Param::ShimmerReverb(ShimmerReverbParam::RoomSize(NormalizedValue::new(0.6))),
                    "Room Size",
                )
                .range(0.0, 1.0)
                .default(0.6)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ShimmerReverb(ShimmerReverbParam::Decay(NormalizedValue::new(0.7))),
                    "Decay",
                )
                .range(0.0, 1.0)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ShimmerReverb(ShimmerReverbParam::Damping(NormalizedValue::new(0.4))),
                    "Damping",
                )
                .range(0.0, 1.0)
                .default(0.4)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ShimmerReverb(ShimmerReverbParam::PreDelay(Seconds::ZERO)),
                    "Pre-Delay",
                )
                .range(0.0, 0.5)
                .default(0.0)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::TimeSlider),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ShimmerReverb(ShimmerReverbParam::PitchSemitones(Semitones::new(12.0))),
                    "Pitch",
                )
                .description("Pitch shift in semitones")
                .range(-12.0, 24.0)
                .default(12.0)
                .unit(ParameterUnit::Semitones)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ShimmerReverb(ShimmerReverbParam::ShimmerMix(NormalizedValue::new(
                        0.35,
                    ))),
                    "Shimmer",
                )
                .description("Amount of pitch-shifted feedback")
                .range(0.0, 1.0)
                .default(0.35)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ShimmerReverb(ShimmerReverbParam::Mix(NormalizedValue::new(0.4))),
                    "Mix",
                )
                .range(0.0, 1.0)
                .default(0.4)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for ShimmerReverb {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.update_params();

        let feedback_gain = self.feedback_gain();
        let lp_coeff = self.lowpass_coeff();
        let mix = self.mix.as_f32();
        let shimmer = self.shimmer_mix.as_f32();
        let sr_recip = 1.0 / self.sample_rate.as_f32();

        #[allow(clippy::cast_possible_truncation)]
        let pre_delay_samples = ((self.pre_delay.as_f32() * self.sample_rate.as_f32()) as usize)
            .min(self.pre_delay_buffer.len() - 1);

        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);

            // Pre-delay
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

            // Mix in pitch-shifted feedback
            let input_with_shimmer = pre_delayed + self.feedback_acc * feedback_gain.as_f32();

            // FDN processing
            let wet = self.core.process_sample(
                input_with_shimmer,
                feedback_gain,
                lp_coeff,
                NormalizedValue::new(0.0), // no highpass
                NormalizedValue::new(0.3), // moderate diffusion
                NormalizedValue::new(1.0), // full width
                sr_recip,
            );

            // Generate shimmer feedback: blend reverb output with pitch-shifted version
            let reverb_mono = (wet.left + wet.right) * 0.5;
            let shifted = self.shifter.process(reverb_mono);
            self.feedback_acc = reverb_mono * (1.0 - shimmer) + shifted * shimmer;

            // Mix
            let result = dry.blend(wet.into(), mix);

            StereoSample::write_frame(output, frame, result);
        }
    }

    fn reset(&mut self) {
        self.core.clear();
        self.pre_delay_buffer.fill(0.0);
        self.pre_delay_index = 0;
        self.shifter.clear();
        self.feedback_acc = 0.0;
    }

    fn tail_samples(&self) -> SampleCount {
        let decay_time = 1.0 + self.decay.as_f32() * 10.0;
        #[allow(clippy::cast_possible_truncation)]
        let samples = (decay_time * self.sample_rate.as_f32()) as usize;
        SampleCount::new(samples)
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }
    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::ShimmerReverb(p) = param {
            match p {
                ShimmerReverbParam::RoomSize(v) => {
                    self.room_size = v;
                    self.params_dirty = true;
                }
                ShimmerReverbParam::Decay(v) => self.decay = v,
                ShimmerReverbParam::Damping(v) => self.damping = v,
                ShimmerReverbParam::PreDelay(v) => {
                    self.pre_delay = Seconds::new(v.as_f32().clamp(0.0, MAX_PRE_DELAY_SECS));
                }
                ShimmerReverbParam::PitchSemitones(v) => {
                    self.pitch_semitones = Semitones::new(v.as_f32().clamp(-12.0, 24.0));
                    self.params_dirty = true;
                }
                ShimmerReverbParam::ShimmerMix(v) => self.shimmer_mix = v,
                ShimmerReverbParam::Mix(v) => self.mix = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::ShimmerReverb(p) = param {
            Some(match p {
                ShimmerReverbParam::RoomSize(_) => self.room_size.as_f32(),
                ShimmerReverbParam::Decay(_) => self.decay.as_f32(),
                ShimmerReverbParam::Damping(_) => self.damping.as_f32(),
                ShimmerReverbParam::PreDelay(_) => self.pre_delay.as_f32(),
                ShimmerReverbParam::PitchSemitones(_) => self.pitch_semitones.as_f32(),
                ShimmerReverbParam::ShimmerMix(_) => self.shimmer_mix.as_f32(),
                ShimmerReverbParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::ShimmerReverb(ShimmerReverbParam::RoomSize(self.room_size)),
            Param::ShimmerReverb(ShimmerReverbParam::Decay(self.decay)),
            Param::ShimmerReverb(ShimmerReverbParam::Damping(self.damping)),
            Param::ShimmerReverb(ShimmerReverbParam::PreDelay(self.pre_delay)),
            Param::ShimmerReverb(ShimmerReverbParam::PitchSemitones(self.pitch_semitones)),
            Param::ShimmerReverb(ShimmerReverbParam::ShimmerMix(self.shimmer_mix)),
            Param::ShimmerReverb(ShimmerReverbParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::ShimmerReverb
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        self.resize_for_sample_rate();
    }
}
