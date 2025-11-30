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

use crate::engine::typed_params::{TypedParam, TypedValue, NoiseParam, ModuleType};
use crate::modules::core::*;
use crate::types::{Gain, SampleRate};

/// Noise color types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoiseType {
    /// White noise - flat spectrum, equal energy per frequency
    #[default]
    White,
    /// Pink noise - -3dB/octave, equal energy per octave
    Pink,
    /// Brown/Red noise - -6dB/octave, darker, rumbling
    Brown,
    /// Blue noise - +3dB/octave, brighter, hissing
    Blue,
    /// Violet noise - +6dB/octave, very bright, sharp
    Violet,
}

impl NoiseType {
    pub const ALL: [Self; 5] = [Self::White, Self::Pink, Self::Brown, Self::Blue, Self::Violet];

    pub fn id(&self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Pink => "pink",
            Self::Brown => "brown",
            Self::Blue => "blue",
            Self::Violet => "violet",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::White => "White",
            Self::Pink => "Pink",
            Self::Brown => "Brown",
            Self::Blue => "Blue",
            Self::Violet => "Violet",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::White => "Flat spectrum - crisp hi-hats, snares",
            Self::Pink => "-3dB/oct - natural, cymbals, atmosphere",
            Self::Brown => "-6dB/oct - dark rumble, thunder",
            Self::Blue => "+3dB/oct - bright, hissing",
            Self::Violet => "+6dB/oct - very bright, sharp",
        }
    }

    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|n| ChoiceOption::new(n.id(), n.name()).with_description(n.description()))
            .collect()
    }
}

/// Advanced noise generator with spectral coloring.
#[derive(Clone)]
pub struct NoiseGenerator {
    // Parameters
    noise_type: NoiseType,
    level: Gain,

    // State for pink noise (Voss-McCartney algorithm)
    pink_rows: [f32; 16],
    pink_running_sum: f32,
    pink_index: u32,

    // State for brown noise (integrator)
    brown_state: f32,

    // State for blue/violet noise (differentiator)
    blue_prev: f32,
    violet_prev: [f32; 2],

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
            pink_rows: [0.0; 16],
            pink_running_sum: 0.0,
            pink_index: 0,
            brown_state: 0.0,
            blue_prev: 0.0,
            violet_prev: [0.0; 2],
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
                self.pink_running_sum -= self.pink_rows[i];
                self.pink_rows[i] = (fastrand::f32() * 2.0 - 1.0) * 0.5;
                self.pink_running_sum += self.pink_rows[i];
                break;
            }
        }

        // Combine and normalize
        (self.pink_running_sum + white) / 5.0
    }

    /// Generate brown noise by integrating white noise.
    /// Brown noise has -6dB/octave slope (darker than pink).
    #[inline]
    fn brown_noise(&mut self) -> f32 {
        let white = Self::white_noise();

        // Leaky integrator to prevent DC drift
        // Coefficient tuned for stable output
        self.brown_state = self.brown_state * 0.99 + white * 0.1;

        // Normalize output (brown noise tends to have lower amplitude)
        self.brown_state * 3.5
    }

    /// Generate blue noise by differentiating white noise.
    /// Blue noise has +3dB/octave slope (brighter than white).
    #[inline]
    fn blue_noise(&mut self) -> f32 {
        let white = Self::white_noise();

        // Simple differentiator (high-pass character)
        let output = white - self.blue_prev;
        self.blue_prev = white;

        // Normalize (differentiation increases amplitude)
        output * 0.5
    }

    /// Generate violet noise by double-differentiating white noise.
    /// Violet noise has +6dB/octave slope (very bright).
    #[inline]
    fn violet_noise(&mut self) -> f32 {
        let white = Self::white_noise();

        // Double differentiator
        let diff1 = white - self.violet_prev[0];
        let output = diff1 - self.violet_prev[1];

        self.violet_prev[1] = self.violet_prev[0];
        self.violet_prev[0] = white;

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
                    TypedParam::Noise(NoiseParam::Type),
                    "Type",
                    NoiseType::to_choices(),
                )
                .description("Noise color/spectrum")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Noise(NoiseParam::Level), "Level")
                    .description("Output level")
                    .range(0.0, 1.0)
                    .default(0.8)
                    .unit(ParameterUnit::Percent)
                    .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Noise output"))
    }
}

impl VoiceModule for NoiseGenerator {
    fn process(
        &mut self,
        _inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = SampleRate::new(context.sample_rate);
        self.output_buffer.resize(context.samples);

        for i in 0..context.samples {
            self.output_buffer[i] = self.generate_sample();
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Noise(noise_param) = param {
            match noise_param {
                NoiseParam::Type => {
                    if let Some(i) = value.as_int() {
                        self.noise_type = match i {
                            0 => NoiseType::White,
                            1 => NoiseType::Pink,
                            2 => NoiseType::Brown,
                            3 => NoiseType::Blue,
                            4 => NoiseType::Violet,
                            _ => NoiseType::White,
                        };
                    }
                }
                NoiseParam::Level => {
                    if let Some(l) = value.as_float() {
                        self.level = Gain::new(l.clamp(0.0, 1.0));
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Noise(noise_param) = param {
            match noise_param {
                NoiseParam::Type => Some(TypedValue::Int(self.noise_type as i32)),
                NoiseParam::Level => Some(TypedValue::Float(self.level.as_f32())),
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Noise
    }

    fn reset(&mut self) {
        // Reset all filter states to avoid clicks
        self.pink_rows = [0.0; 16];
        self.pink_running_sum = 0.0;
        self.pink_index = 0;
        self.brown_state = 0.0;
        self.blue_prev = 0.0;
        self.violet_prev = [0.0; 2];
    }

    fn note_on(&mut self, _note: u8, _velocity: f32) {
        // Reset state on note for consistent attack
        self.reset();
    }

    fn note_off(&mut self) {
        // Noise doesn't need to do anything on note off
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = SampleRate::new(sample_rate);
    }

    fn box_clone(&self) -> Box<dyn VoiceModule> {
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
