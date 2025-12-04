//! Delay effect module.
//!
//! Features:
//! - Stereo delay with independent L/R times
//! - Tempo sync option
//! - Ping-pong mode
//! - Feedback with soft limiting
//! - Modulation input for chorus-like effects

use crate::engine::typed_params::{DelayMode, DelayParam, ModuleType, Param};
use crate::modules::core::*;
use crate::types::{
    BeatDivision, Bpm, BufferIndex, FilterState, Hertz, NormalizedValue, SampleRate, Seconds,
};

/// Maximum delay time in seconds.
const MAX_DELAY_SECONDS: f32 = 2.0;

/// Stereo delay effect.
pub struct Delay {
    // Parameters
    mode: DelayMode,
    time_left: Seconds,
    time_right: Seconds,
    feedback: NormalizedValue,
    mix: NormalizedValue,
    high_cut: Hertz,

    // Tempo sync
    tempo_sync: bool,
    sync_division: BeatDivision,

    // Delay buffers
    buffer_left: Vec<f32>,
    buffer_right: Vec<f32>,
    write_pos: BufferIndex,

    // Filter state for feedback
    filter_left: FilterState,
    filter_right: FilterState,

    // State
    sample_rate: SampleRate,
}

impl Delay {
    pub fn new() -> Self {
        let buffer_size = (MAX_DELAY_SECONDS * 48000.0) as usize;
        Self {
            mode: DelayMode::Mono,
            time_left: Seconds::new(0.375),
            time_right: Seconds::new(0.5),
            feedback: NormalizedValue::new(0.4),
            mix: NormalizedValue::CENTER,
            high_cut: Hertz::new(8000.0),
            tempo_sync: false,
            sync_division: BeatDivision::QUARTER,
            buffer_left: vec![0.0; buffer_size],
            buffer_right: vec![0.0; buffer_size],
            write_pos: BufferIndex::ZERO,
            filter_left: FilterState::ZERO,
            filter_right: FilterState::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Calculate delay time in seconds from BPM and sync division.
    #[inline]
    fn synced_delay_time(&self, bpm: f32) -> Seconds {
        if bpm <= 0.0 {
            return self.time_left;
        }
        let tempo = Bpm::new(bpm);
        let duration = self.sync_division.to_duration(tempo);
        Seconds::new(duration.as_f32().min(MAX_DELAY_SECONDS))
    }

    /// Resize buffers for new sample rate.
    fn resize_buffers(&mut self) {
        let size = (MAX_DELAY_SECONDS * self.sample_rate.as_f32()) as usize;
        if self.buffer_left.len() != size {
            self.buffer_left.resize(size, 0.0);
            self.buffer_right.resize(size, 0.0);
            self.write_pos = BufferIndex::ZERO;
        }
    }

    /// Read from delay buffer with linear interpolation.
    #[inline]
    fn read_interpolated(buffer: &[f32], write_pos: BufferIndex, delay_samples: f32) -> f32 {
        let len = buffer.len();
        let read_pos = (write_pos.as_usize() as f32 - delay_samples).rem_euclid(len as f32);
        let idx0 = (read_pos as usize) % len;
        let idx1 = (idx0 + 1) % len;
        let frac = read_pos - read_pos.floor();

        buffer[idx0] * (1.0 - frac) + buffer[idx1] * frac
    }

    /// One-pole lowpass for feedback damping.
    #[inline]
    fn lowpass(
        input: f32,
        state: &mut FilterState,
        cutoff: Hertz,
        sample_rate: SampleRate,
    ) -> f32 {
        let coef = (-std::f32::consts::TAU * cutoff.as_f32() / sample_rate.as_f32()).exp();
        state.0 = input * (1.0 - coef) + state.0 * coef;
        state.0
    }
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Delay {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("delay", "Delay")
            .description("Stereo delay with ping-pong mode")
            .category(ModuleCategory::Effect)
            .tag("delay")
            .tag("effect")
            .tag("time")
            .port(PortDescriptor::audio_input("in_l", "In L").description("Left input"))
            .port(PortDescriptor::audio_input("in_r", "In R").description("Right input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
            .port(
                PortDescriptor::control_input("time_cv", "Time CV")
                    .description("Delay time modulation"),
            )
            .parameter(
                ParameterDescriptor::choice(
                    Param::Delay(DelayParam::Mode(DelayMode::Mono)),
                    "Mode",
                    DelayMode::to_choices(),
                )
                .description("Delay mode"),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Delay(DelayParam::Time(Seconds::new(0.375))),
                    "Time",
                )
                .description("Delay time")
                .range(0.001, MAX_DELAY_SECONDS)
                .default(0.375)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::TimeSlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Delay(DelayParam::Feedback(NormalizedValue::new(0.4))),
                    "Feedback",
                )
                .description("Feedback amount")
                .range(0.0, 0.95)
                .default(0.4)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Delay(DelayParam::Mix(NormalizedValue::CENTER)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Delay(DelayParam::Damping(NormalizedValue::new(0.4))),
                    "Tone",
                )
                .description("Feedback high-cut filter")
                .range(0.0, 1.0)
                .default(0.4)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Delay {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;
        self.resize_buffers();

        // Calculate effective delay time (synced or manual)
        let effective_time = if self.tempo_sync {
            self.synced_delay_time(context.tempo.as_f32())
        } else {
            self.time_left
        };

        let delay_samples_left = (effective_time.as_f32() * self.sample_rate.as_f32())
            .min((self.buffer_left.len() - 1) as f32);
        let delay_samples_right = if self.tempo_sync {
            delay_samples_left
        } else {
            (self.time_right.as_f32() * self.sample_rate.as_f32())
                .min((self.buffer_right.len() - 1) as f32)
        };

        let len = self.buffer_left.len();
        let feedback = self.feedback.as_f32();
        let mix = self.mix.as_f32();

        // Process assuming interleaved stereo
        let channels = 2;
        for frame in 0..context.samples {
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

            // Read from delay buffers
            let delayed_l =
                Self::read_interpolated(&self.buffer_left, self.write_pos, delay_samples_left);
            let delayed_r =
                Self::read_interpolated(&self.buffer_right, self.write_pos, delay_samples_right);

            // Apply feedback filtering
            let fb_l =
                Self::lowpass(delayed_l, &mut self.filter_left, self.high_cut, self.sample_rate);
            let fb_r =
                Self::lowpass(delayed_r, &mut self.filter_right, self.high_cut, self.sample_rate);

            // Calculate feedback signal based on mode
            let (write_l, write_r) = match self.mode {
                DelayMode::Mono => {
                    let mono_in = (in_l + in_r) * 0.5;
                    let mono_fb = (fb_l + fb_r) * 0.5;
                    let write = mono_in + mono_fb * feedback;
                    (write, write)
                }
                DelayMode::Stereo => (in_l + fb_l * feedback, in_r + fb_r * feedback),
                DelayMode::PingPong => (in_l + fb_r * feedback, in_r + fb_l * feedback),
            };

            // Soft limit feedback to prevent runaway
            let write_l = write_l.tanh();
            let write_r = write_r.tanh();

            // Write to delay buffers
            self.buffer_left[self.write_pos.as_usize()] = write_l;
            self.buffer_right[self.write_pos.as_usize()] = write_r;

            // Advance write position
            self.write_pos = self.write_pos.advance(len);

            // Mix dry/wet
            if idx_l < output.len() {
                output[idx_l] = in_l * (1.0 - mix) + delayed_l * mix;
            }
            if idx_r < output.len() {
                output[idx_r] = in_r * (1.0 - mix) + delayed_r * mix;
            }
        }
    }

    fn reset(&mut self) {
        self.buffer_left.fill(0.0);
        self.buffer_right.fill(0.0);
        self.filter_left = FilterState::ZERO;
        self.filter_right = FilterState::ZERO;
        self.write_pos = BufferIndex::ZERO;
    }

    fn set_mix(&mut self, mix: f32) {
        self.mix = NormalizedValue::new(mix);
    }

    fn get_mix(&self) -> f32 {
        self.mix.as_f32()
    }

    fn tail_samples(&self) -> usize {
        let decay_time =
            self.time_left.as_f32() * (1.0 / (1.0 - self.feedback.as_f32())).ln() / 3.0;
        (decay_time * self.sample_rate.as_f32()) as usize
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Delay(delay_param) = param {
            match delay_param {
                DelayParam::Mode(m) => self.mode = m,
                DelayParam::Time(t) => {
                    let clamped = t.as_f32().clamp(0.001, MAX_DELAY_SECONDS);
                    self.time_left = Seconds::new(clamped);
                    self.time_right = Seconds::new(clamped);
                }
                DelayParam::TimeLeft(t) => {
                    self.time_left = Seconds::new(t.as_f32().clamp(0.001, MAX_DELAY_SECONDS))
                }
                DelayParam::TimeRight(t) => {
                    self.time_right = Seconds::new(t.as_f32().clamp(0.001, MAX_DELAY_SECONDS))
                }
                DelayParam::Feedback(f) => {
                    self.feedback = NormalizedValue::new(f.as_f32().clamp(0.0, 0.95))
                }
                DelayParam::Mix(m) => self.mix = m,
                DelayParam::Damping(d) => {
                    // Map normalized 0-1 to frequency 200-20000 Hz
                    let freq = 200.0 + d.as_f32() * (20000.0 - 200.0);
                    self.high_cut = Hertz::new(freq);
                }
                DelayParam::TempoSync(s) => self.tempo_sync = s,
                DelayParam::SyncDivision(d) => self.sync_division = d,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Delay(delay_param) = param {
            Some(match delay_param {
                DelayParam::Mode(_) => self.mode.index() as f32,
                DelayParam::Time(_) => self.time_left.as_f32(),
                DelayParam::TimeLeft(_) => self.time_left.as_f32(),
                DelayParam::TimeRight(_) => self.time_right.as_f32(),
                DelayParam::Feedback(_) => self.feedback.as_f32(),
                DelayParam::Mix(_) => self.mix.as_f32(),
                DelayParam::Damping(_) => {
                    // Convert frequency back to normalized
                    (self.high_cut.as_f32() - 200.0) / (20000.0 - 200.0)
                }
                DelayParam::TempoSync(_) => {
                    if self.tempo_sync {
                        1.0
                    } else {
                        0.0
                    }
                }
                DelayParam::SyncDivision(_) => self.sync_division.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Delay(DelayParam::Mode(self.mode)),
            Param::Delay(DelayParam::Time(self.time_left)),
            Param::Delay(DelayParam::TimeLeft(self.time_left)),
            Param::Delay(DelayParam::TimeRight(self.time_right)),
            Param::Delay(DelayParam::Feedback(self.feedback)),
            Param::Delay(DelayParam::Mix(self.mix)),
            Param::Delay(DelayParam::Damping(NormalizedValue::new(
                (self.high_cut.as_f32() - 200.0) / (20000.0 - 200.0),
            ))),
            Param::Delay(DelayParam::TempoSync(self.tempo_sync)),
            Param::Delay(DelayParam::SyncDivision(self.sync_division)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delay_creation() {
        let delay = Delay::new();
        assert_eq!(delay.mode, DelayMode::Mono);
        assert!((delay.time_left.as_f32() - 0.375).abs() < 0.001);
    }

    #[test]
    fn test_delay_no_explosion() {
        let mut delay = Delay::new();
        delay.sample_rate = SampleRate::DVD_QUALITY;
        delay.feedback = NormalizedValue::new(0.9);
        delay.resize_buffers();

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: 256,
            ..Default::default()
        };

        let input = vec![1.0; 512];
        let mut output = vec![0.0; 512];

        for _ in 0..100 {
            delay.process(&input, &mut output, &context);
        }

        for sample in &output {
            assert!(sample.abs() < 10.0, "Delay output exploded");
        }
    }
}
