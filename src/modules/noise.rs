//! Advanced Noise Generator module.
//!
//! Provides spectrally colored noise for sound design, hi-hats, cymbals,
//! atmospheres, and textures.
//!
//! Noise colors:
//! - White: Flat spectrum (equal energy per frequency)
//! - Pink: -3dB/octave (equal energy per octave, natural sounding)
//! - Brown: -6dB/octave (darker, rumble)
//! - Blue: +3dB/octave (brighter, hissing)
//! - Violet: +6dB/octave (very bright, sharp)

use std::collections::HashMap;

use crate::engine::typed_params::{ModuleType, NoiseParam, NoiseType, Param};
use crate::modules::core::*;
use crate::types::{FilterState, Gain, MidiNote, SampleRate};

/// Advanced noise generator with spectral coloring.
#[derive(Clone)]
pub struct NoiseGenerator {
    // Parameters
    noise_type: NoiseType,
    level: Gain,

    // State for pink noise (Voss-McCartney algorithm)
    pink_rows: [FilterState; 16],
    pink_running_sum: FilterState,
    pink_index: u32,

    // State for brown noise (integrator)
    brown_state: FilterState,

    // State for blue/violet noise (differentiator)
    blue_prev: FilterState,
    violet_prev: [FilterState; 2],

    // Sample rate
    sample_rate: SampleRate,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl NoiseGenerator {
    pub fn new() -> Self {
        Self {
            noise_type: NoiseType::White,
            level: Gain::new(0.8),
            pink_rows: [FilterState::ZERO; 16],
            pink_running_sum: FilterState::ZERO,
            pink_index: 0,
            brown_state: FilterState::ZERO,
            blue_prev: FilterState::ZERO,
            violet_prev: [FilterState::ZERO; 2],
            sample_rate: SampleRate::DVD_QUALITY,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Generate white noise using fastrand (thread-local, lock-free).
    #[inline]
    fn white_noise() -> f32 {
        fastrand::f32() * 2.0 - 1.0
    }

    /// Generate pink noise using Voss-McCartney algorithm.
    /// Pink noise has equal energy per octave (-3dB/octave slope).
    #[inline]
    fn pink_noise(&mut self) -> f32 {
        let white = Self::white_noise();

        // Voss-McCartney algorithm: update rows based on trailing zeros of index
        let last_index = self.pink_index;
        self.pink_index = self.pink_index.wrapping_add(1);
        let changed = last_index ^ self.pink_index;

        // Find which rows need updating (trailing zeros indicate the row)
        for i in 0..16 {
            if (changed & (1 << i)) != 0 {
                let running_sum = self.pink_running_sum.as_f32() - self.pink_rows[i].as_f32();
                let new_row = (fastrand::f32() * 2.0 - 1.0) * 0.5;
                self.pink_rows[i] = FilterState::new(new_row);
                self.pink_running_sum = FilterState::new(running_sum + new_row);
                break;
            }
        }

        // Combine and normalize
        (self.pink_running_sum.as_f32() + white) / 5.0
    }

    /// Generate brown noise by integrating white noise.
    /// Brown noise has -6dB/octave slope (darker than pink).
    #[inline]
    fn brown_noise(&mut self) -> f32 {
        let white = Self::white_noise();

        // Leaky integrator to prevent DC drift
        // Coefficient tuned for stable output
        let new_state = self.brown_state.as_f32() * 0.99 + white * 0.1;
        self.brown_state = FilterState::new(new_state);

        // Normalize output (brown noise tends to have lower amplitude)
        new_state * 3.5
    }

    /// Generate blue noise by differentiating white noise.
    /// Blue noise has +3dB/octave slope (brighter than white).
    #[inline]
    fn blue_noise(&mut self) -> f32 {
        let white = Self::white_noise();

        // Simple differentiator (high-pass character)
        let output = white - self.blue_prev.as_f32();
        self.blue_prev = FilterState::new(white);

        // Normalize (differentiation increases amplitude)
        output * 0.5
    }

    /// Generate violet noise by double-differentiating white noise.
    /// Violet noise has +6dB/octave slope (very bright).
    #[inline]
    fn violet_noise(&mut self) -> f32 {
        let white = Self::white_noise();

        // Double differentiator
        let diff1 = white - self.violet_prev[0].as_f32();
        let output = diff1 - self.violet_prev[1].as_f32();

        self.violet_prev[1] = self.violet_prev[0];
        self.violet_prev[0] = FilterState::new(white);

        // Normalize (double differentiation significantly increases amplitude)
        output * 0.35
    }

    /// Generate a single noise sample based on current type.
    #[inline]
    fn generate_sample(&mut self) -> f32 {
        let sample = match self.noise_type {
            NoiseType::White => Self::white_noise(),
            NoiseType::Pink => self.pink_noise(),
            NoiseType::Brown => self.brown_noise(),
            NoiseType::Blue => self.blue_noise(),
            NoiseType::Violet => self.violet_noise(),
        };

        // Soft clip to prevent extreme values
        let clipped = sample.clamp(-1.0, 1.0);

        clipped * self.level.as_f32()
    }
}

impl Default for NoiseGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for NoiseGenerator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("noise", "Noise")
            .description("Spectrally colored noise generator for textures and percussion")
            .category(ModuleCategory::Oscillator)
            .tag("noise")
            .tag("source")
            .tag("texture")
            .tag("percussion")
            .parameter(
                ParameterDescriptor::choice(
                    Param::Noise(NoiseParam::Type(NoiseType::White)),
                    "Type",
                    NoiseType::to_choices(),
                )
                .description("Noise color/spectrum")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Noise(NoiseParam::Level(Gain::new(0.8))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.8)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Noise output"))
    }
}

impl PolyModule for NoiseGenerator {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        for i in 0..context.samples {
            self.output_buffer[i] = self.generate_sample();
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Noise(noise_param) = param {
            match noise_param {
                NoiseParam::Type(t) => self.noise_type = t,
                NoiseParam::Level(l) => self.level = Gain::new(l.as_f32().clamp(0.0, 1.0)),
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Noise(noise_param) = param {
            Some(match noise_param {
                NoiseParam::Type(_) => self.noise_type.index() as f32,
                NoiseParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Noise(NoiseParam::Type(self.noise_type)),
            Param::Noise(NoiseParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Noise
    }

    fn reset(&mut self) {
        // Reset all filter states to avoid clicks
        self.pink_rows.fill(FilterState::ZERO);
        self.pink_running_sum = FilterState::ZERO;
        self.pink_index = 0;
        self.brown_state = FilterState::ZERO;
        self.blue_prev = FilterState::ZERO;
        self.violet_prev.fill(FilterState::ZERO);
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: f32) {
        // Reset state on note for consistent attack
        self.reset();
    }

    fn note_off(&mut self) {
        // Noise doesn't need to do anything on note off
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noise_creation() {
        let noise = NoiseGenerator::new();
        assert_eq!(noise.noise_type, NoiseType::White);
    }

    #[test]
    fn test_noise_types_exist() {
        assert_eq!(NoiseType::ALL.len(), 5);
    }

    #[test]
    fn test_noise_output_range() {
        let mut noise = NoiseGenerator::new();
        noise.level = Gain::new(1.0);

        for noise_type in NoiseType::ALL {
            noise.noise_type = noise_type;
            noise.reset();

            for _ in 0..1000 {
                let sample = noise.generate_sample();
                assert!(
                    sample >= -1.0 && sample <= 1.0,
                    "{:?} produced out-of-range sample: {}",
                    noise_type,
                    sample
                );
            }
        }
    }
}
