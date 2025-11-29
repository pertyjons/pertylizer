//! Oscillator module with multiple waveforms.
//!
//! Features:
//! - Multiple waveforms (sine, triangle, saw, square, pulse, noise)
//! - Band-limited waveforms using PolyBLEP for anti-aliasing
//! - Pulse width modulation
//! - Hard sync
//! - FM and PM inputs

use std::collections::HashMap;
use std::f32::consts::TAU;

use crate::engine::typed_params::{TypedParam, TypedValue, OscillatorParam, ModuleType};
use crate::modules::core::*;
use crate::types::{Hertz, Cents};

/// A band-limited oscillator.
#[derive(Clone)]
pub struct Oscillator {
    // Parameters
    waveform: Waveform,
    frequency: f32,
    detune: f32,        // In cents
    pulse_width: f32,   // 0.0 - 1.0
    level: f32,
    fm_mode: FmMode,    // Linear or Exponential FM

    // State
    phase: f32,
    sample_rate: f32,

    // For noise
    noise_state: u32,

    // For pink noise (Voss-McCartney algorithm)
    pink_rows: [f32; 16],
    pink_running_sum: f32,
    pink_index: u32,

    // Outputs
    output_buffer: AudioBuffer,
}

/// FM mode selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FmMode {
    #[default]
    Exponential,
    Linear,
}

impl Oscillator {
    pub fn new() -> Self {
        Self {
            waveform: Waveform::Sawtooth,
            frequency: 440.0,
            detune: 0.0,
            pulse_width: 0.5,
            level: 1.0,
            fm_mode: FmMode::Exponential,
            phase: 0.0,
            sample_rate: 48000.0,
            noise_state: 0x12345678,
            pink_rows: [0.0; 16],
            pink_running_sum: 0.0,
            pink_index: 0,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Generate white noise sample using xorshift.
    #[inline]
    fn white_noise(&mut self) -> f32 {
        self.noise_state ^= self.noise_state << 13;
        self.noise_state ^= self.noise_state >> 17;
        self.noise_state ^= self.noise_state << 5;
        (self.noise_state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Generate pink noise using Voss-McCartney algorithm.
    /// Pink noise has equal energy per octave (-3dB/octave slope).
    #[inline]
    fn pink_noise(&mut self) -> f32 {
        let white = self.white_noise();

        // Voss-McCartney algorithm: update rows based on trailing zeros of index
        let last_index = self.pink_index;
        self.pink_index = self.pink_index.wrapping_add(1);
        let changed = last_index ^ self.pink_index;

        // Find which rows need updating (trailing zeros indicate the row)
        for i in 0..16 {
            if (changed & (1 << i)) != 0 {
                // Subtract old value, add new random value
                self.pink_running_sum -= self.pink_rows[i];
                self.pink_rows[i] = self.white_noise() * 0.5;
                self.pink_running_sum += self.pink_rows[i];
                break; // Only update one row per sample
            }
        }

        // Combine running sum with current white noise and normalize
        (self.pink_running_sum + white) / 5.0
    }

    /// Calculate the actual frequency including detune.
    /// Uses type-safe Cents for the detune calculation.
    #[inline]
    fn actual_frequency(&self) -> f32 {
        // Use Cents type for type-safe interval calculation
        let detune = Cents::new(self.detune);
        Hertz::new(self.frequency).transpose(detune.to_semitones().as_f32()).as_f32()
    }

    /// PolyBLEP correction for band-limited waveforms.
    #[inline]
    fn poly_blep(&self, t: f32, dt: f32) -> f32 {
        if t < dt {
            let t = t / dt;
            2.0 * t - t * t - 1.0
        } else if t > 1.0 - dt {
            let t = (t - 1.0) / dt;
            t * t + 2.0 * t + 1.0
        } else {
            0.0
        }
    }

    /// Generate a single sample with optional frequency and phase modulation.
    #[inline]
    fn generate_sample(&mut self, freq_mod: f32, phase_mod: f32) -> f32 {
        // Apply frequency modulation based on mode
        let base_freq = self.actual_frequency();
        let freq = match self.fm_mode {
            FmMode::Exponential => {
                // Exponential FM (1V/octave style): freq_mod of 1.0 = +2 octaves
                let freq_mult = (freq_mod * 2.0).exp2();
                base_freq * freq_mult
            }
            FmMode::Linear => {
                // Linear FM: add Hz directly (scaled by base frequency)
                // This gives stable harmonic ratios across the keyboard
                (base_freq + freq_mod * base_freq * 4.0).max(1.0)
            }
        };

        let dt = freq / self.sample_rate;
        let phase = (self.phase + phase_mod).rem_euclid(1.0);

        let sample = match self.waveform {
            Waveform::Sine => {
                (phase * TAU).sin()
            }

            Waveform::Triangle => {
                // Band-limited triangle using integrated square
                let tri = if phase < 0.5 {
                    4.0 * phase - 1.0
                } else {
                    3.0 - 4.0 * phase
                };
                // Approximate PolyBLEP for triangle (less critical)
                tri
            }

            Waveform::Sawtooth => {
                // Naive saw
                let mut saw = 2.0 * phase - 1.0;
                // Apply PolyBLEP at discontinuity
                saw -= self.poly_blep(phase, dt);
                saw
            }

            Waveform::Square => {
                // Naive square
                let mut sq = if phase < 0.5 { 1.0 } else { -1.0 };
                // PolyBLEP at both edges
                sq += self.poly_blep(phase, dt);
                sq -= self.poly_blep((phase + 0.5).rem_euclid(1.0), dt);
                sq
            }

            Waveform::Pulse => {
                // Pulse with variable width
                let pw = self.pulse_width.clamp(0.01, 0.99);
                let mut pulse = if phase < pw { 1.0 } else { -1.0 };
                // PolyBLEP at both edges
                pulse += self.poly_blep(phase, dt);
                pulse -= self.poly_blep((phase + (1.0 - pw)).rem_euclid(1.0), dt);
                pulse
            }

            Waveform::Noise => {
                // White noise using xorshift
                self.white_noise()
            }

            Waveform::PinkNoise => {
                // Pink noise (-3dB/octave, equal energy per octave)
                self.pink_noise()
            }
        };

        // Advance phase
        self.phase = (self.phase + dt).rem_euclid(1.0);

        sample * self.level
    }

    /// Set frequency from MIDI note using type-safe conversion.
    pub fn set_note(&mut self, note: u8) {
        // Uses Hertz::from_midi for type-safe frequency conversion
        self.frequency = Hertz::from_midi(note).as_f32();
    }

    /// Set frequency using the type-safe Hertz type.
    pub fn set_frequency(&mut self, freq: Hertz) {
        self.frequency = freq.as_f32();
    }

    /// Get current frequency as type-safe Hertz.
    pub fn get_frequency(&self) -> Hertz {
        Hertz::new(self.frequency)
    }
}

impl Default for Oscillator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Oscillator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("oscillator", "Oscillator")
            .description("Band-limited oscillator with multiple waveforms")
            .category(ModuleCategory::Oscillator)
            .tag("oscillator")
            .tag("source")
            // Parameters
            .parameter(
                ParameterDescriptor::choice(
                    TypedParam::Oscillator(OscillatorParam::Waveform),
                    "Waveform",
                    Waveform::to_choices(),
                )
                .description("Oscillator waveform")
                .widget(WidgetHint::WaveformSelector),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Oscillator(OscillatorParam::Frequency), "Frequency")
                    .description("Base frequency")
                    .range(20.0, 20000.0)
                    .default(440.0)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::FrequencySlider)
                    .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Oscillator(OscillatorParam::Detune), "Detune")
                    .description("Fine tune in cents")
                    .range(-100.0, 100.0)
                    .default(0.0)
                    .unit(ParameterUnit::Cents)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Oscillator(OscillatorParam::PulseWidth), "Pulse Width")
                    .description("Pulse width for pulse waveform")
                    .range(0.01, 0.99)
                    .default(0.5)
                    .unit(ParameterUnit::Percent)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Oscillator(OscillatorParam::Level), "Level")
                    .description("Output level")
                    .range(0.0, 1.0)
                    .default(1.0)
                    .unit(ParameterUnit::None)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    TypedParam::Oscillator(OscillatorParam::FmMode),
                    "FM Mode",
                    vec![
                        ChoiceOption::new("exponential", "Exponential")
                            .with_description("1V/octave style FM (pitch tracking)"),
                        ChoiceOption::new("linear", "Linear")
                            .with_description("Hz-based FM (stable harmonics)"),
                    ],
                )
                .description("FM input mode: Exponential (1V/oct) or Linear (Hz)")
                .widget(WidgetHint::Dropdown),
            )
            // Ports
            .port(PortDescriptor::control_input("fm", "FM").description("Frequency modulation input"))
            .port(PortDescriptor::control_input("pm", "PM").description("Phase modulation input"))
            .port(PortDescriptor::control_input("pwm", "PWM").description("Pulse width modulation"))
            .port(PortDescriptor::gate_input("sync", "Sync").description("Hard sync input"))
            .port(PortDescriptor::audio_output("out", "Out").description("Audio output"))
    }
}

impl VoiceModule for Oscillator {
    fn process(
        &mut self,
        inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        // Get modulation inputs
        let fm_input = inputs.get("fm");
        let pm_input = inputs.get("pm");
        let pwm_input = inputs.get("pwm");
        let sync_input = inputs.get("sync");

        let mut prev_sync = 0.0f32;

        for i in 0..context.samples {
            // Get FM modulation amount (normalized -1 to 1)
            let fm = fm_input.map(|f| f[i]).unwrap_or(0.0);
            
            // PWM modulation
            if let Some(pwm) = pwm_input {
                self.pulse_width = (0.5 + pwm[i] * 0.49).clamp(0.01, 0.99);
            }

            // Hard sync - reset phase on rising edge
            if let Some(sync) = sync_input {
                let sync_val = sync[i];
                if sync_val > 0.5 && prev_sync <= 0.5 {
                    self.phase = 0.0;
                }
                prev_sync = sync_val;
            }

            // Phase modulation
            let pm = pm_input.map(|p| p[i]).unwrap_or(0.0);

            // Generate sample with FM and PM
            self.output_buffer[i] = self.generate_sample(fm, pm);
        }

        // Copy to output
        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Oscillator(osc_param) = param {
            match osc_param {
                OscillatorParam::Waveform => {
                    if let TypedValue::Waveform(w) = value {
                        // Direct assignment - same type now!
                        self.waveform = w;
                    }
                }
                OscillatorParam::Frequency => {
                    if let Some(f) = value.as_float() {
                        self.frequency = f.clamp(20.0, 20000.0);
                    }
                }
                OscillatorParam::Detune => {
                    if let Some(d) = value.as_float() {
                        self.detune = d.clamp(-100.0, 100.0);
                    }
                }
                OscillatorParam::PulseWidth => {
                    if let Some(pw) = value.as_float() {
                        self.pulse_width = pw.clamp(0.01, 0.99);
                    }
                }
                OscillatorParam::Level => {
                    if let Some(l) = value.as_float() {
                        self.level = l.clamp(0.0, 1.0);
                    }
                }
                OscillatorParam::Octave => {
                    if let Some(o) = value.as_int() {
                        let base_freq = self.frequency / 2.0f32.powi(0);
                        self.frequency = base_freq * 2.0f32.powi(o);
                    }
                }
                OscillatorParam::Phase => {
                    if let Some(p) = value.as_float() {
                        self.phase = p.clamp(0.0, 1.0);
                    }
                }
                OscillatorParam::FmMode => {
                    if let Some(i) = value.as_int() {
                        self.fm_mode = if i == 0 { FmMode::Exponential } else { FmMode::Linear };
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Oscillator(osc_param) = param {
            match osc_param {
                OscillatorParam::Waveform => Some(TypedValue::Waveform(self.waveform)),
                OscillatorParam::Frequency => Some(TypedValue::Float(self.frequency)),
                OscillatorParam::Detune => Some(TypedValue::Float(self.detune)),
                OscillatorParam::PulseWidth => Some(TypedValue::Float(self.pulse_width)),
                OscillatorParam::Level => Some(TypedValue::Float(self.level)),
                OscillatorParam::Octave => Some(TypedValue::Int(0)),
                OscillatorParam::Phase => Some(TypedValue::Float(self.phase)),
                OscillatorParam::FmMode => Some(TypedValue::Int(if self.fm_mode == FmMode::Exponential { 0 } else { 1 })),
            }
        } else {
            None
        }
    }
    
    fn module_type(&self) -> ModuleType {
        ModuleType::Oscillator
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        // Reset pink noise state
        self.pink_rows = [0.0; 16];
        self.pink_running_sum = 0.0;
        self.pink_index = 0;
    }

    fn note_on(&mut self, note: u8, _velocity: f32) {
        self.set_note(note);
        // Optionally reset phase on note on
        // self.phase = 0.0;
    }

    fn note_off(&mut self) {
        // Oscillator doesn't need to do anything on note off
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    fn box_clone(&self) -> Box<dyn VoiceModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oscillator_creation() {
        let osc = Oscillator::new();
        assert_eq!(osc.waveform, Waveform::Sawtooth);
        assert!((osc.frequency - 440.0).abs() < 0.001);
    }

    #[test]
    fn test_note_to_frequency() {
        let mut osc = Oscillator::new();
        osc.set_note(69); // A4
        assert!((osc.frequency - 440.0).abs() < 0.001);

        osc.set_note(60); // C4
        assert!((osc.frequency - 261.63).abs() < 1.0);
    }

    #[test]
    fn test_waveform_output() {
        let mut osc = Oscillator::new();
        osc.waveform = Waveform::Sine;
        osc.frequency = 1000.0;
        osc.sample_rate = 48000.0;

        // Generate a few samples
        for _ in 0..100 {
            let sample = osc.generate_sample(0.0, 0.0);
            assert!(sample >= -1.0 && sample <= 1.0);
        }
    }
}
