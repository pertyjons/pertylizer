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

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{
    BipolarValue, Cents, Gain, Hertz, MidiNote, NormalizedValue, Phase, PortName, SampleRate,
    Semitones, Velocity, Waveform,
};
use synth_core::{FmMode, ModuleType, OscillatorParam, Param};
use synth_dsp::oscillators::{poly_blamp, poly_blep};

/// A band-limited oscillator.
#[derive(Clone)]
pub struct Oscillator {
    // Parameters
    waveform: Waveform,
    frequency: Hertz,
    detune: Cents,
    octave: Semitones,
    pulse_width: NormalizedValue,
    level: Gain,
    phase_offset: Phase,
    fm_mode: FmMode,
    fm_amount: BipolarValue,

    // State
    phase: Phase,
    sample_rate: SampleRate,
    /// Previous sync signal value for edge detection (persists across buffers).
    prev_sync: f32,

    // Mod matrix offsets
    /// Pitch offset in semitones (from mod matrix).
    mod_offset_pitch: f32,
    /// Level offset (additive, from mod matrix).
    mod_offset_level: f32,

    // Outputs
    output_buffer: AudioBuffer,
}

impl Oscillator {
    pub fn new() -> Self {
        Self {
            waveform: Waveform::Sawtooth,
            frequency: Hertz::A4,
            detune: Cents::ZERO,
            octave: Semitones::ZERO,
            pulse_width: NormalizedValue::CENTER,
            level: Gain::UNITY,
            phase_offset: Phase::ZERO,
            fm_mode: FmMode::Exponential,
            fm_amount: BipolarValue::MAX,
            phase: Phase::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
            prev_sync: 0.0,
            mod_offset_pitch: 0.0,
            mod_offset_level: 0.0,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Calculate the actual frequency including detune, octave, and mod matrix pitch offset.
    #[inline]
    fn actual_frequency(&self) -> Hertz {
        let octave_mult = 2.0f32.powf(self.octave.as_f32() / 12.0);
        // Apply mod matrix pitch offset (in semitones)
        let mod_mult = (self.mod_offset_pitch / 12.0).exp2();
        Hertz::new(self.detune.apply(self.frequency).as_f32() * octave_mult * mod_mult)
    }

    /// Generate a single sample with optional frequency and phase modulation.
    #[inline]
    fn generate_sample(
        &mut self,
        freq_mod: f32,
        phase_mod: f32,
        effective_pulse_width: NormalizedValue,
    ) -> f32 {
        let base_freq = self.actual_frequency();
        let freq = match self.fm_mode {
            FmMode::Exponential => {
                let freq_mult = (freq_mod * 2.0).exp2();
                Hertz::new(base_freq.as_f32() * freq_mult)
            }
            FmMode::Linear => {
                Hertz::new((base_freq.as_f32() + freq_mod * base_freq.as_f32() * 4.0).max(1.0))
            }
        };

        let dt = freq.phase_increment(self.sample_rate);
        // Apply phase modulation and static phase offset
        let phase = (self.phase.advance(phase_mod).as_f32() + self.phase_offset.as_f32()) % 1.0;

        let sample = match self.waveform {
            Waveform::Sine => (phase * TAU).sin(),
            Waveform::Triangle => {
                // Apply PolyBLAMP at triangle corners (0.25 and 0.75)
                let mut tri = Phase::new_unchecked(phase).triangle();
                // Corner at phase 0.25 (peak)
                let dist_to_peak = phase - 0.25;
                tri += poly_blamp(dist_to_peak, dt) * 4.0; // Scale by slope change magnitude
                // Corner at phase 0.75 (trough)
                let dist_to_trough = phase - 0.75;
                tri -= poly_blamp(dist_to_trough, dt) * 4.0;
                tri
            }
            Waveform::Sawtooth => {
                let mut saw = Phase::new_unchecked(phase).sawtooth();
                saw -= poly_blep(phase, dt);
                saw
            }
            Waveform::Square => {
                let mut sq = Phase::new_unchecked(phase).pulse(NormalizedValue::CENTER);
                sq += poly_blep(phase, dt);
                sq -= poly_blep((phase + 0.5).rem_euclid(1.0), dt);
                sq
            }
            Waveform::Pulse => {
                let mut pulse = Phase::new_unchecked(phase).pulse(effective_pulse_width);
                let pw = effective_pulse_width.as_f32().clamp(0.01, 0.99);
                pulse += poly_blep(phase, dt);
                pulse -= poly_blep((phase + (1.0 - pw)).rem_euclid(1.0), dt);
                pulse
            }
        };

        self.phase = self.phase.advance(dt);
        let effective_level = (self.level.as_f32() + self.mod_offset_level).clamp(0.0, 2.0);
        sample * effective_level
    }

    /// Set frequency from MIDI note.
    pub fn set_note(&mut self, note: MidiNote) {
        self.frequency = note.to_frequency();
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
            .parameter(
                ParameterDescriptor::choice(
                    Param::Oscillator(OscillatorParam::Waveform(Waveform::Sawtooth)),
                    "Waveform",
                    Waveform::to_choices(),
                )
                .description("Oscillator waveform")
                .widget(WidgetHint::WaveformSelector),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Oscillator(OscillatorParam::Frequency(Hertz::A4)),
                    "Frequency",
                )
                .description("Base frequency")
                .range(20.0, 20000.0)
                .default(440.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::FrequencySlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Oscillator(OscillatorParam::Detune(Cents::ZERO)),
                    "Detune",
                )
                .description("Fine tune in cents")
                .range(-100.0, 100.0)
                .default(0.0)
                .unit(ParameterUnit::Cents)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Oscillator(OscillatorParam::PulseWidth(NormalizedValue::CENTER)),
                    "Pulse Width",
                )
                .description("Pulse width for pulse waveform")
                .range(0.01, 0.99)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Oscillator(OscillatorParam::Level(Gain::UNITY)),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::choice(
                    Param::Oscillator(OscillatorParam::FmMode(FmMode::Exponential)),
                    "FM Mode",
                    FmMode::to_choices(),
                )
                .description("FM input mode: Exponential (1V/oct) or Linear (Hz)")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Oscillator(OscillatorParam::FmAmount(BipolarValue::MAX)),
                    "FM Amt",
                )
                .description("FM input attenuverter (-1 to +1)")
                .range(-1.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("fm", "FM").description("Frequency modulation input"),
            )
            .port(PortDescriptor::control_input("pm", "PM").description("Phase modulation input"))
            .port(PortDescriptor::control_input("pwm", "PWM").description("Pulse width modulation"))
            .port(PortDescriptor::gate_input("sync", "Sync").description("Hard sync input"))
            .port(PortDescriptor::audio_output("out", "Out").description("Audio output"))
    }
}

impl PolyModule for Oscillator {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let fm_input = inputs.get(PortName::FM);
        let pm_input = inputs.get(PortName::PM);
        #[allow(clippy::similar_names)]
        // pm = phase mod, pwm = pulse width mod - different concepts
        let pwm_input = inputs.get(PortName::PWM);
        let sync_input = inputs.get(PortName::SYNC);

        for i in 0..context.samples.as_usize() {
            let fm = fm_input
                .map(|f| f[i] * self.fm_amount.as_f32())
                .unwrap_or(0.0);

            let effective_pulse_width = if let Some(pwm) = pwm_input {
                NormalizedValue::new((self.pulse_width.as_f32() + pwm[i] * 0.49).clamp(0.01, 0.99))
            } else {
                self.pulse_width
            };

            if let Some(sync) = sync_input {
                let sync_val = sync[i];
                if sync_val > 0.5 && self.prev_sync <= 0.5 {
                    self.phase = Phase::ZERO;
                }
                self.prev_sync = sync_val;
            }

            let pm = pm_input.map(|p| p[i]).unwrap_or(0.0);
            self.output_buffer[i] = self.generate_sample(fm, pm, effective_pulse_width);
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Oscillator(osc_param) = param {
            match osc_param {
                OscillatorParam::Waveform(w) => self.waveform = w,
                OscillatorParam::Frequency(f) => {
                    self.frequency = Hertz::new(f.as_f32().clamp(20.0, 20000.0))
                }
                OscillatorParam::Detune(d) => self.detune = d.clamp_detune(),
                OscillatorParam::Octave(o) => self.octave = o,
                OscillatorParam::PulseWidth(pw) => {
                    self.pulse_width = NormalizedValue::new(pw.as_f32().clamp(0.01, 0.99))
                }
                OscillatorParam::Level(l) => self.level = l,
                OscillatorParam::Phase(p) => self.phase_offset = p,
                OscillatorParam::FmMode(m) => self.fm_mode = m,
                OscillatorParam::FmAmount(a) => self.fm_amount = a,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Oscillator(osc_param) = param {
            Some(match osc_param {
                OscillatorParam::Waveform(_) => self.waveform.index() as f32,
                OscillatorParam::Frequency(_) => self.frequency.as_f32(),
                OscillatorParam::Detune(_) => self.detune.as_f32(),
                OscillatorParam::Octave(_) => self.octave.as_f32(),
                OscillatorParam::PulseWidth(_) => self.pulse_width.as_f32(),
                OscillatorParam::Level(_) => self.level.as_f32(),
                OscillatorParam::Phase(_) => self.phase_offset.as_f32(),
                OscillatorParam::FmMode(_) => self.fm_mode.index() as f32,
                OscillatorParam::FmAmount(_) => self.fm_amount.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Oscillator(OscillatorParam::Waveform(self.waveform)),
            Param::Oscillator(OscillatorParam::Frequency(self.frequency)),
            Param::Oscillator(OscillatorParam::Detune(self.detune)),
            Param::Oscillator(OscillatorParam::Octave(self.octave)),
            Param::Oscillator(OscillatorParam::PulseWidth(self.pulse_width)),
            Param::Oscillator(OscillatorParam::Level(self.level)),
            Param::Oscillator(OscillatorParam::Phase(self.phase_offset)),
            Param::Oscillator(OscillatorParam::FmMode(self.fm_mode)),
            Param::Oscillator(OscillatorParam::FmAmount(self.fm_amount)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Oscillator
    }

    fn reset(&mut self) {
        self.phase = Phase::ZERO;
        self.prev_sync = 0.0;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.set_note(note);
    }

    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    fn set_mod_offset(&mut self, dest_index: u8, value: f32) {
        match dest_index {
            0 => self.mod_offset_pitch += value,
            1 => self.mod_offset_level += value,
            _ => {}
        }
    }

    fn clear_mod_offsets(&mut self) {
        self.mod_offset_pitch = 0.0;
        self.mod_offset_level = 0.0;
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
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
        osc.set_note(MidiNote::A4); // A4
        assert!((osc.frequency.as_f32() - 440.0).abs() < 0.001);

        osc.set_note(MidiNote::C4); // C4
        assert!((osc.frequency.as_f32() - 261.63).abs() < 1.0);
    }

    #[test]
    fn test_waveform_output() {
        let mut osc = Oscillator::new();
        osc.waveform = Waveform::Sine;
        osc.frequency = Hertz::new(1000.0);
        osc.sample_rate = SampleRate::DVD_QUALITY;

        let effective_pulse_width = osc.pulse_width;
        for _ in 0..100 {
            let sample = osc.generate_sample(0.0, 0.0, effective_pulse_width);
            assert!(sample >= -1.0 && sample <= 1.0);
        }
    }
}
