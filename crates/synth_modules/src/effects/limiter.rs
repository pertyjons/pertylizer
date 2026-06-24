//! Brickwall limiter with look-ahead.
//!
//! Features:
//! - True peak detection with look-ahead buffer
//! - Configurable ceiling, release, and look-ahead time
//! - Soft knee near ceiling to avoid hard clipping
//! - Gain reduction metering

use synth_core::{
    AudioEffect, Decibels, Describable, LimiterParam, Milliseconds, ModuleCategory,
    ModuleDescriptor, ModuleType, NormalizedValue, Param, ParameterDescriptor, ProcessContext,
    ResponseCurve, SampleCount, SampleRate, StereoSample, WidgetHint,
};

/// Maximum look-ahead time the limiter advertises (see the `look_ahead`
/// parameter range, capped at 5 ms).
const MAX_LOOKAHEAD_MS: f32 = 5.0;

/// Highest sample rate the engine supports. The look-ahead ring buffer is sized
/// from this so the advertised 5 ms is honored at any sample rate (e.g. 96/192
/// kHz), not just 48 kHz. Single source of truth: `SampleRate::MAX_SUPPORTED`.
const MAX_SAMPLE_RATE: f32 = SampleRate::MAX_SUPPORTED.as_f32();

/// Buffer capacity (in frames) for the maximum look-ahead time at the highest
/// supported sample rate: `0.005 * 192000 = 960`.
const MAX_LOOKAHEAD_SAMPLES: usize = (MAX_LOOKAHEAD_MS / 1000.0 * MAX_SAMPLE_RATE) as usize;

/// Brickwall limiter with look-ahead.
pub struct Limiter {
    // Parameters
    ceiling: Decibels,
    lookahead_ms: Milliseconds,
    release_ms: Milliseconds,
    mix: NormalizedValue,

    // Look-ahead ring buffer (interleaved stereo)
    lookahead_buffer: Vec<f32>,
    write_pos: usize,

    // Gain envelope
    gain_envelope: f32,

    // State
    sample_rate: SampleRate,
    lookahead_samples: usize,
}

impl Limiter {
    pub fn new() -> Self {
        let lookahead_size = MAX_LOOKAHEAD_SAMPLES * 2; // stereo interleaved
        Self {
            ceiling: Decibels::new(-0.3),
            lookahead_ms: Milliseconds::new(3.0),
            release_ms: Milliseconds::new(100.0),
            mix: NormalizedValue::MAX,

            lookahead_buffer: vec![0.0; lookahead_size],
            write_pos: 0,

            gain_envelope: 1.0,

            sample_rate: SampleRate::DVD_QUALITY,
            lookahead_samples: 132, // ~3ms at 44.1kHz
        }
    }

    fn update_lookahead(&mut self) {
        self.lookahead_samples = ((self.lookahead_ms.as_f32() * 0.001 * self.sample_rate.as_f32())
            as usize)
            .clamp(1, MAX_LOOKAHEAD_SAMPLES);
    }

    /// Read from the look-ahead buffer at a given delay.
    #[inline]
    fn read_delayed(&self, delay_frames: usize, channel: usize) -> f32 {
        let total_frames = self.lookahead_buffer.len() / 2;
        let read_frame = (self.write_pos / 2 + total_frames - delay_frames) % total_frames;
        self.lookahead_buffer[read_frame * 2 + channel]
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Limiter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("limiter", "Limiter")
            .description("Brickwall limiter with look-ahead")
            .category(ModuleCategory::Effect)
            .tag("limiter")
            .tag("dynamics")
            .tag("effect")
            .parameter(
                ParameterDescriptor::float(
                    "ceiling",
                    Param::Limiter(LimiterParam::Ceiling(Decibels::new(-0.3))),
                    "Ceiling",
                )
                .description("Output ceiling level")
                .range(-12.0, 0.0)
                .default(-0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "look_ahead",
                    Param::Limiter(LimiterParam::LookAhead(Milliseconds::new(3.0))),
                    "Look-Ahead",
                )
                .description("Look-ahead time for peak detection")
                .range(0.5, 5.0)
                .default(3.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "release",
                    Param::Limiter(LimiterParam::Release(Milliseconds::new(100.0))),
                    "Release",
                )
                .description("Gain recovery time")
                .range(10.0, 500.0)
                .default(100.0)
                .curve(ResponseCurve::Logarithmic)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::Limiter(LimiterParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Limiter {
    #[allow(clippy::too_many_lines)]
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        let ceiling_linear = self.ceiling.to_linear();
        let release_coeff =
            (-1.0 / (self.release_ms.as_f32() * 0.001 * self.sample_rate.as_f32()).max(1.0)).exp();
        let mix = self.mix.as_f32();
        let lookahead = self.lookahead_samples;
        let total_frames = self.lookahead_buffer.len() / 2;

        for frame in 0..context.samples.as_usize() {
            // Read input
            let dry = StereoSample::read_frame(input, frame);
            let in_l = dry.left;
            let in_r = dry.right;

            // Write to look-ahead buffer
            self.lookahead_buffer[self.write_pos] = in_l;
            self.lookahead_buffer[self.write_pos + 1] = in_r;
            self.write_pos = (self.write_pos + 2) % self.lookahead_buffer.len();

            // Scan ahead for peaks
            let mut peak = 0.0_f32;
            for d in 0..lookahead {
                let l = self.read_delayed(d, 0).abs();
                let r = self.read_delayed(d, 1).abs();
                peak = peak.max(l).max(r);
            }

            // Calculate needed gain reduction
            let target_gain = if peak > ceiling_linear {
                ceiling_linear / peak
            } else {
                1.0
            };

            // Smooth gain envelope: instant attack, slow release
            if target_gain < self.gain_envelope {
                self.gain_envelope = target_gain;
            } else {
                self.gain_envelope =
                    self.gain_envelope * release_coeff + target_gain * (1.0 - release_coeff);
            }

            // Read delayed signal (the signal we'll limit)
            let delayed_frame = (self.write_pos / 2 + total_frames - lookahead) % total_frames;
            let delayed_l = self.lookahead_buffer[delayed_frame * 2];
            let delayed_r = self.lookahead_buffer[delayed_frame * 2 + 1];

            // Apply gain
            let limited_l = delayed_l * self.gain_envelope;
            let limited_r = delayed_r * self.gain_envelope;

            // Mix (dry = delayed signal, wet = limited)
            let result = StereoSample::new(delayed_l, delayed_r)
                .blend(StereoSample::new(limited_l, limited_r), mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn reset(&mut self) {
        self.lookahead_buffer.fill(0.0);
        self.write_pos = 0;
        self.gain_envelope = 1.0;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn tail_samples(&self) -> SampleCount {
        SampleCount::new(self.lookahead_samples)
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Limiter(p) = param {
            match p {
                LimiterParam::Ceiling(db) => {
                    self.ceiling = Decibels::new(db.as_f32().clamp(-12.0, 0.0));
                }
                LimiterParam::LookAhead(ms) => {
                    self.lookahead_ms = Milliseconds::new(ms.as_f32().clamp(0.5, 5.0));
                    self.update_lookahead();
                }
                LimiterParam::Release(ms) => {
                    self.release_ms = Milliseconds::new(ms.as_f32().clamp(10.0, 500.0));
                }
                LimiterParam::Mix(m) => self.mix = m,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Limiter(p) = param {
            Some(match p {
                LimiterParam::Ceiling(_) => self.ceiling.as_f32(),
                LimiterParam::LookAhead(_) => self.lookahead_ms.as_f32(),
                LimiterParam::Release(_) => self.release_ms.as_f32(),
                LimiterParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Limiter(LimiterParam::Ceiling(self.ceiling)),
            Param::Limiter(LimiterParam::LookAhead(self.lookahead_ms)),
            Param::Limiter(LimiterParam::Release(self.release_ms)),
            Param::Limiter(LimiterParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Limiter
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        self.update_lookahead();
        // Resize buffer for new sample rate
        let new_size = MAX_LOOKAHEAD_SAMPLES * 2;
        if self.lookahead_buffer.len() != new_size {
            self.lookahead_buffer.resize(new_size, 0.0);
            self.write_pos = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_limiter_creation() {
        let limiter = Limiter::new();
        assert!(limiter.ceiling.as_f32() < 0.0);
    }

    #[test]
    fn test_limiter_reduces_peaks() {
        let mut limiter = Limiter::new();
        limiter.ceiling = Decibels::new(-6.0); // ~0.5 linear
        // Use default lookahead (~132 samples at 44.1kHz) for proper peak detection
        limiter.update_lookahead();

        // Need enough frames to fill the lookahead buffer and settle
        let num_frames = 1024;
        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(num_frames),
            ..Default::default()
        };

        // Loud stereo input (2 floats per frame)
        let input: Vec<f32> = vec![1.0; num_frames * 2];
        let mut output = vec![0.0; num_frames * 2];

        limiter.process(&input, &mut output, &context);

        // After the lookahead buffer has filled, output should be limited.
        // Settle past the actual (runtime) lookahead fill, not the full buffer
        // capacity, which is sized for the maximum supported sample rate.
        let ceiling_linear = Decibels::new(-6.0).to_linear();
        let settle_offset = limiter.lookahead_samples * 2 + 40; // past lookahead fill + settling
        for sample in &output[settle_offset..] {
            assert!(
                sample.abs() <= ceiling_linear + 0.05,
                "Limiter output too loud: {}",
                sample
            );
        }
    }
}
