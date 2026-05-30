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
//! - LFSR: deterministic shift-register noise with retro digital texture
//! - Chip: SID-inspired 23-bit shift-register noise

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, PortName, ProcessContext, WidgetHint,
};
use synth_core::{FilterState, Gain, MidiNote, SampleRate, Velocity};
use synth_core::{ModuleType, NoiseParam, NoiseType, Param};

const LFSR_SEED: u16 = 0x7fff;
const CHIP_LFSR_SEED: u32 = 0x7ffff8;
const CHIP_LFSR_MASK: u32 = 0x7fffff;

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

    // State for clocked digital noise.
    lfsr_state: u16,
    chip_lfsr_state: u32,

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
            lfsr_state: LFSR_SEED,
            chip_lfsr_state: CHIP_LFSR_SEED,
            sample_rate: SampleRate::DVD_QUALITY,
            output_buffer: AudioBuffer::new(1024),
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

        // Second difference: y[n] = x[n] - 2*x[n-1] + x[n-2]
        let output = white - 2.0 * self.violet_prev[0].as_f32() + self.violet_prev[1].as_f32();

        self.violet_prev[1] = self.violet_prev[0];
        self.violet_prev[0] = FilterState::new(white);

        // Normalize (double differentiation significantly increases amplitude)
        output * 0.35
    }

    /// Generate deterministic 15-bit LFSR noise.
    ///
    /// The byte-shaped output keeps the tone gritty without collapsing to a
    /// pure one-bit square stream.
    #[inline]
    fn lfsr_noise(&mut self) -> f32 {
        let bit = ((self.lfsr_state >> 14) ^ (self.lfsr_state >> 13)) & 1;
        self.lfsr_state = ((self.lfsr_state << 1) | bit) & 0x7fff;
        if self.lfsr_state == 0 {
            self.lfsr_state = LFSR_SEED;
        }

        let byte = (self.lfsr_state & 0x00ff) as f32;
        byte / 127.5 - 1.0
    }

    /// Generate SID-inspired chip noise from a 23-bit shift register.
    ///
    /// This is intentionally a lightweight musical model, not exact SID
    /// emulation. It uses the commonly referenced SID noise output taps to get
    /// the sparse metallic character that works well for drums and FX.
    #[inline]
    fn chip_noise(&mut self) -> f32 {
        let bit = ((self.chip_lfsr_state >> 22) ^ (self.chip_lfsr_state >> 17)) & 1;
        self.chip_lfsr_state = ((self.chip_lfsr_state << 1) | bit) & CHIP_LFSR_MASK;
        if self.chip_lfsr_state == 0 {
            self.chip_lfsr_state = CHIP_LFSR_SEED;
        }

        let taps = [22, 20, 16, 13, 11, 7, 4, 2];
        let mut byte = 0u8;
        for (i, tap) in taps.iter().enumerate() {
            byte |= (((self.chip_lfsr_state >> tap) & 1) as u8) << (7 - i);
        }

        (f32::from(byte) / 127.5 - 1.0) * 1.05
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
            NoiseType::Lfsr => self.lfsr_noise(),
            NoiseType::Chip => self.chip_noise(),
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
                    "type",
                    Param::Noise(NoiseParam::Type(NoiseType::White)),
                    "Type",
                    NoiseType::to_choices(),
                )
                .description("Noise color/spectrum")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::Noise(NoiseParam::Level(Gain::new(0.8))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.8)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Noise output. Connect to: Filter In, Amplifier In, Mixer"),
            )
    }
}

impl PolyModule for NoiseGenerator {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        for i in 0..context.samples.as_usize() {
            self.output_buffer[i] = self.generate_sample();
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
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
        self.lfsr_state = LFSR_SEED;
        self.chip_lfsr_state = CHIP_LFSR_SEED;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
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
        assert_eq!(NoiseType::ALL.len(), 7);
        assert_eq!(NoiseType::Lfsr.id(), "lfsr");
        assert_eq!(NoiseType::Chip.id(), "chip");
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
                    (-1.0..=1.0).contains(&sample),
                    "{:?} produced out-of-range sample: {}",
                    noise_type,
                    sample
                );
            }
        }
    }

    #[test]
    fn test_lfsr_noise_resets_deterministically() {
        let mut noise = NoiseGenerator::new();
        noise.noise_type = NoiseType::Lfsr;
        noise.level = Gain::new(1.0);

        let first: Vec<f32> = (0..32).map(|_| noise.generate_sample()).collect();
        noise.reset();
        let second: Vec<f32> = (0..32).map(|_| noise.generate_sample()).collect();

        assert_eq!(first, second);
    }

    #[test]
    fn test_chip_noise_resets_deterministically() {
        let mut noise = NoiseGenerator::new();
        noise.noise_type = NoiseType::Chip;
        noise.level = Gain::new(1.0);

        let first: Vec<f32> = (0..32).map(|_| noise.generate_sample()).collect();
        noise.reset();
        let second: Vec<f32> = (0..32).map(|_| noise.generate_sample()).collect();

        assert_eq!(first, second);
    }
}
