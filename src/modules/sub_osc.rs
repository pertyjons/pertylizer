//! Sub-Oscillator module for adding bass weight.
//!
//! A dedicated sub-oscillator that tracks the main pitch at -1 or -2 octaves.
//! Essential for adding "fatness" to bass sounds without sacrificing a main oscillator.
//!
//! Features:
//! - -1 or -2 octave transposition
//! - Square, Sine, or 25% Pulse waveforms
//! - Level control
//! - Follows note input

use std::collections::HashMap;
use std::f32::consts::TAU;

use crate::engine::typed_params::{TypedParam, TypedValue, SubOscParam, ModuleType};
use crate::modules::core::*;
use crate::types::{Hertz, Phase, Gain, SampleRate};

/// Sub-oscillator waveform types.
/// Limited set optimized for bass reinforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubOscWaveform {
    /// Pure sine - smooth, fundamental bass
    #[default]
    Sine,
    /// Square wave - punchy, full harmonics
    Square,
    /// 25% pulse - hollow, distinctive character
    Pulse25,
}

impl SubOscWaveform {
    pub const ALL: [Self; 3] = [Self::Sine, Self::Square, Self::Pulse25];

    pub fn id(&self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::Square => "square",
            Self::Pulse25 => "pulse25",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Sine => "Sine",
            Self::Square => "Square",
            Self::Pulse25 => "Pulse 25%",
        }
    }

    pub fn to_choices() -> Vec<ChoiceOption> {
        Self::ALL
            .iter()
            .map(|w| ChoiceOption::new(w.id(), w.name()))
            .collect()
    }
}

/// Sub-oscillator octave selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubOscOctave {
    /// One octave down (-1)
    #[default]
    MinusOne,
    /// Two octaves down (-2)
    MinusTwo,
}

impl SubOscOctave {
    /// Get the frequency divisor for this octave.
    #[inline]
    pub fn divisor(self) -> f32 {
        match self {
            Self::MinusOne => 2.0,
            Self::MinusTwo => 4.0,
        }
    }
}

/// Sub-oscillator for bass reinforcement.
#[derive(Clone)]
pub struct SubOscillator {
    // Parameters
    waveform: SubOscWaveform,
    octave: SubOscOctave,
    level: Gain,

    // State
    phase: Phase,
    base_frequency: Hertz,
    sample_rate: SampleRate,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl SubOscillator {
    pub fn new() -> Self {
        Self {
            waveform: SubOscWaveform::Square,
            octave: SubOscOctave::MinusOne,
            level: Gain::new(0.5),
            phase: Phase::ZERO,
            base_frequency: Hertz::A4,
            sample_rate: SampleRate::DVD_QUALITY,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Generate a single sample.
    #[inline]
    fn generate_sample(&mut self) -> f32 {
        let freq = Hertz::new(self.base_frequency.as_f32() / self.octave.divisor());
        let phase = self.phase.as_f32();

        let sample = match self.waveform {
            SubOscWaveform::Sine => {
                (phase * TAU).sin()
            }
            SubOscWaveform::Square => {
                if phase < 0.5 { 1.0 } else { -1.0 }
            }
            SubOscWaveform::Pulse25 => {
                if phase < 0.25 { 1.0 } else { -1.0 }
            }
        };

        // Advance phase
        let dt = freq.phase_increment(self.sample_rate);
        self.phase = self.phase.advance(dt);

        sample * self.level.as_f32()
    }
}

impl Default for SubOscillator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for SubOscillator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("sub_osc", "Sub Osc")
            .description("Sub-oscillator for bass reinforcement at -1 or -2 octaves")
            .category(ModuleCategory::Oscillator)
            .tag("oscillator")
            .tag("sub")
            .tag("bass")
            .parameter(
                ParameterDescriptor::choice(
                    TypedParam::SubOsc(SubOscParam::Waveform),
                    "Waveform",
                    SubOscWaveform::to_choices(),
                )
                .description("Sub-oscillator waveform")
                .widget(WidgetHint::WaveformSelector),
            )
            .parameter(
                ParameterDescriptor::choice(
                    TypedParam::SubOsc(SubOscParam::Octave),
                    "Octave",
                    vec![
                        ChoiceOption::new("minus1", "-1 Oct")
                            .with_description("One octave below"),
                        ChoiceOption::new("minus2", "-2 Oct")
                            .with_description("Two octaves below"),
                    ],
                )
                .description("Octave transposition")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::SubOsc(SubOscParam::Level), "Level")
                    .description("Sub-oscillator output level")
                    .range(0.0, 1.0)
                    .default(0.5)
                    .unit(ParameterUnit::Percent)
                    .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Audio output"))
    }
}

impl VoiceModule for SubOscillator {
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
        if let TypedParam::SubOsc(sub_param) = param {
            match sub_param {
                SubOscParam::Waveform => {
                    if let Some(i) = value.as_int() {
                        self.waveform = match i {
                            0 => SubOscWaveform::Sine,
                            1 => SubOscWaveform::Square,
                            2 => SubOscWaveform::Pulse25,
                            _ => SubOscWaveform::Square,
                        };
                    }
                }
                SubOscParam::Octave => {
                    if let Some(i) = value.as_int() {
                        self.octave = match i {
                            0 => SubOscOctave::MinusOne,
                            1 => SubOscOctave::MinusTwo,
                            _ => SubOscOctave::MinusOne,
                        };
                    }
                }
                SubOscParam::Level => {
                    if let Some(l) = value.as_float() {
                        self.level = Gain::new(l.clamp(0.0, 1.0));
                    }
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::SubOsc(sub_param) = param {
            match sub_param {
                SubOscParam::Waveform => Some(TypedValue::Int(self.waveform as i32)),
                SubOscParam::Octave => Some(TypedValue::Int(self.octave as i32)),
                SubOscParam::Level => Some(TypedValue::Float(self.level.as_f32())),
            }
        } else {
            None
        }
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::SubOscillator
    }

    fn reset(&mut self) {
        self.phase = Phase::ZERO;
    }

    fn note_on(&mut self, note: u8, _velocity: f32) {
        self.base_frequency = Hertz::from_midi(note);
        // Reset phase on note for consistent attack
        self.phase = Phase::ZERO;
    }

    fn note_off(&mut self) {
        // Sub-oscillator doesn't need to do anything on note off
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
    fn test_sub_osc_creation() {
        let sub = SubOscillator::new();
        assert_eq!(sub.waveform, SubOscWaveform::Square);
        assert_eq!(sub.octave, SubOscOctave::MinusOne);
    }

    #[test]
    fn test_octave_divisor() {
        assert_eq!(SubOscOctave::MinusOne.divisor(), 2.0);
        assert_eq!(SubOscOctave::MinusTwo.divisor(), 4.0);
    }

    #[test]
    fn test_sub_osc_frequency() {
        let mut sub = SubOscillator::new();
        sub.base_frequency = Hertz::new(440.0);
        sub.octave = SubOscOctave::MinusOne;

        // At -1 octave, 440 Hz should become 220 Hz
        let expected_freq = 220.0;
        let actual_freq = sub.base_frequency.as_f32() / sub.octave.divisor();
        assert!((actual_freq - expected_freq).abs() < 0.001);
    }
}
