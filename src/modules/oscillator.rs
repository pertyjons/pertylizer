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
use crate::types::{Hertz, Cents, Phase, NormalizedValue, SampleRate};

/// A band-limited oscillator.
#[derive(Clone)]
pub struct Oscillator {
    // Parameters
    waveform: Waveform,
    frequency: Hertz,
    detune: Cents,
    pulse_width: NormalizedValue,
    level: NormalizedValue,
    fm_mode: FmMode,    // Linear or Exponential FM

    // State
    phase: Phase,
    sample_rate: SampleRate,

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
            frequency: Hertz::A4,
            detune: Cents::ZERO,
            pulse_width: NormalizedValue::CENTER,
            level: NormalizedValue::MAX,
            fm_mode: FmMode::Exponential,
            phase: Phase::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
            pink_rows: [0.0; 16],
            pink_running_sum: 0.0,
            pink_index: 0,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Generate white noise sample using fastrand (thread-local, lock-free).
    #[inline]
    fn white_noise(&self) -> f32 {
        // Convert [0, 1) to [-1, 1)
        fastrand::f32() * 2.0 - 1.0
    }

    /// Generate pink noise using Voss-McCartney algorithm with fastrand source.
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
                // Subtract old value, add new random value (using fastrand)
                self.pink_running_sum -= self.pink_rows[i];
                self.pink_rows[i] = (fastrand::f32() * 2.0 - 1.0) * 0.5;
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
    fn actual_frequency(&self) -> Hertz {
        // Use Cents type for type-safe interval calculation
        self.detune.apply(self.frequency)
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
                Hertz::new(base_freq.as_f32() * freq_mult)
            }
            FmMode::Linear => {
                // Linear FM: add Hz directly (scaled by base frequency)
                // This gives stable harmonic ratios across the keyboard
                Hertz::new((base_freq.as_f32() + freq_mod * base_freq.as_f32() * 4.0).max(1.0))
            }
        };

        let dt = freq.phase_increment(self.sample_rate);
        let phase = self.phase.advance(phase_mod).as_f32();

        let sample = match self.waveform {
            Waveform::Sine => {
                (phase * TAU).sin()
            }

            Waveform::Triangle => {
                // Use Phase::triangle() for cleaner code
                Phase::new_unchecked(phase).triangle()
            }

            Waveform::Sawtooth => {
                // Use Phase::sawtooth() with PolyBLEP anti-aliasing
                let mut saw = Phase::new_unchecked(phase).sawtooth();
                saw -= self.poly_blep(phase, dt);
                saw
            }

            Waveform::Square => {
                // Use Phase::pulse() for square wave (width = 0.5)
                let mut sq = Phase::new_unchecked(phase).pulse(NormalizedValue::CENTER);
                // PolyBLEP at both edges
                sq += self.poly_blep(phase, dt);
                sq -= self.poly_blep((phase + 0.5).rem_euclid(1.0), dt);
                sq
            }

            Waveform::Pulse => {
                // Use Phase::pulse() with variable width
                let mut pulse = Phase::new_unchecked(phase).pulse(self.pulse_width);
                // PolyBLEP at both edges
                let pw = self.pulse_width.as_f32().clamp(0.01, 0.99);
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
        self.phase = self.phase.advance(dt);

        sample * self.level.as_f32()
    }

    /// Set frequency from MIDI note using type-safe conversion.
    pub fn set_note(&mut self, note: u8) {
        // Uses Hertz::from_midi for type-safe frequency conversion
        self.frequency = Hertz::from_midi(note);
    }

    /// Set frequency using the type-safe Hertz type.
    pub fn set_frequency(&mut self, freq: Hertz) {
        self.frequency = freq;
    }

    /// Get current frequency as type-safe Hertz.
    pub fn get_frequency(&self) -> Hertz {
        self.frequency
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
        self.sample_rate = SampleRate::new(context.sample_rate);
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
                self.pulse_width = NormalizedValue::new((0.5 + pwm[i] * 0.49).clamp(0.01, 0.99));
            }

            // Hard sync - reset phase on rising edge
            if let Some(sync) = sync_input {
                let sync_val = sync[i];
                if sync_val > 0.5 && prev_sync <= 0.5 {
                    self.phase = Phase::ZERO;
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
                        self.frequency = Hertz::new(f.clamp(20.0, 20000.0));
                    }
                }
                OscillatorParam::Detune => {
                    if let Some(d) = value.as_float() {
                        self.detune = Cents::new(d).clamp_detune();
                    }
                }
                OscillatorParam::PulseWidth => {
                    if let Some(pw) = value.as_float() {
                        self.pulse_width = NormalizedValue::new(pw.clamp(0.01, 0.99));
                    }
                }
                OscillatorParam::Level => {
                    if let Some(l) = value.as_float() {
                        self.level = NormalizedValue::new(l);
                    }
                }
                OscillatorParam::Octave => {
                    if let Some(o) = value.as_int() {
                        self.frequency = self.frequency.octave(o);
                    }
                }
                OscillatorParam::Phase => {
                    if let Some(p) = value.as_float() {
                        self.phase = Phase::new(p);
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
                OscillatorParam::Frequency => Some(TypedValue::Float(self.frequency.as_f32())),
                OscillatorParam::Detune => Some(TypedValue::Float(self.detune.as_f32())),
                OscillatorParam::PulseWidth => Some(TypedValue::Float(self.pulse_width.as_f32())),
                OscillatorParam::Level => Some(TypedValue::Float(self.level.as_f32())),
                OscillatorParam::Octave => Some(TypedValue::Int(0)),
                OscillatorParam::Phase => Some(TypedValue::Float(self.phase.as_f32())),
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
        self.phase = Phase::ZERO;
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
    fn test_oscillator_creation() {
        let osc = Oscillator::new();
        assert_eq!(osc.waveform, Waveform::Sawtooth);
        assert!((osc.frequency.as_f32() - 440.0).abs() < 0.001);
    }

    #[test]
    fn test_note_to_frequency() {
        let mut osc = Oscillator::new();
        osc.set_note(69); // A4
        assert!((osc.frequency.as_f32() - 440.0).abs() < 0.001);

        osc.set_note(60); // C4
        assert!((osc.frequency.as_f32() - 261.63).abs() < 1.0);
    }

    #[test]
    fn test_waveform_output() {
        let mut osc = Oscillator::new();
        osc.waveform = Waveform::Sine;
        osc.frequency = Hertz::new(1000.0);
        osc.sample_rate = SampleRate::DVD_QUALITY;

        // Generate a few samples
        for _ in 0..100 {
            let sample = osc.generate_sample(0.0, 0.0);
            assert!(sample >= -1.0 && sample <= 1.0);
        }
    }
}
