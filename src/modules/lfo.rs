//! LFO (Low Frequency Oscillator) module.
//!
//! Features:
//! - Multiple waveforms
//! - Tempo sync option
//! - Phase offset
//! - Unipolar/bipolar output
//! - Sample and hold mode

use std::collections::HashMap;
use std::f32::consts::TAU;

use crate::engine::typed_params::{TypedParam, TypedValue, LfoParam, ModuleType};
use crate::modules::core::*;
use crate::types::{Hertz, Phase, NormalizedValue, SampleRate};

/// LFO output mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoMode {
    /// Output ranges from -1 to +1
    Bipolar,
    /// Output ranges from 0 to +1
    Unipolar,
}

/// Low Frequency Oscillator.
#[derive(Clone)]
pub struct Lfo {
    // Parameters
    waveform: LfoWaveform,
    rate: Hertz,
    depth: NormalizedValue,
    phase_offset: Phase,
    mode: LfoMode,

    // State
    phase: Phase,
    sample_rate: SampleRate,

    // For S&H
    sh_value: f32,
    sh_trigger_prev: f32,
    noise_state: u32,

    // Tempo sync
    tempo_sync: bool,
    sync_division: f32,  // In beats (0.25 = 16th note, 1.0 = quarter, etc)

    // Output
    output_buffer: AudioBuffer,
}

impl Lfo {
    pub fn new() -> Self {
        Self {
            waveform: LfoWaveform::Sine,
            rate: Hertz::new(1.0),
            depth: NormalizedValue::MAX,
            phase_offset: Phase::ZERO,
            mode: LfoMode::Bipolar,
            phase: Phase::ZERO,
            sample_rate: SampleRate::DVD_QUALITY,
            sh_value: 0.0,
            sh_trigger_prev: 0.0,
            noise_state: 0x12345678,
            tempo_sync: false,
            sync_division: 1.0,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Generate random value for S&H.
    #[inline]
    fn random(&mut self) -> f32 {
        self.noise_state ^= self.noise_state << 13;
        self.noise_state ^= self.noise_state >> 17;
        self.noise_state ^= self.noise_state << 5;
        (self.noise_state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Generate a single sample.
    #[inline]
    fn generate_sample(&mut self, tempo: f32) -> f32 {
        let effective_rate = if self.tempo_sync {
            // Convert beat division to Hz based on tempo
            Hertz::new(tempo / 60.0 / self.sync_division)
        } else {
            self.rate
        };

        let phase_inc = effective_rate.phase_increment(self.sample_rate);
        let phase = self.phase.advance(self.phase_offset.as_f32()).as_f32();

        let raw = match self.waveform {
            LfoWaveform::Sine => {
                (phase * TAU).sin()
            }
            LfoWaveform::Triangle => {
                if phase < 0.5 {
                    4.0 * phase - 1.0
                } else {
                    3.0 - 4.0 * phase
                }
            }
            LfoWaveform::Sawtooth => {
                2.0 * phase - 1.0
            }
            LfoWaveform::Square => {
                if phase < 0.5 { 1.0 } else { -1.0 }
            }
            LfoWaveform::SampleAndHold => {
                // Trigger new random value at phase wrap
                let trigger = if phase < phase_inc { 1.0 } else { 0.0 };
                if trigger > 0.5 && self.sh_trigger_prev <= 0.5 {
                    self.sh_value = self.random();
                }
                self.sh_trigger_prev = trigger;
                self.sh_value
            }
        };

        // Advance phase
        self.phase = self.phase.advance(phase_inc);

        // Apply mode
        let output = match self.mode {
            LfoMode::Bipolar => raw,
            LfoMode::Unipolar => (raw + 1.0) * 0.5,
        };

        output * self.depth.as_f32()
    }

    /// Reset the LFO phase.
    pub fn retrigger(&mut self) {
        self.phase = Phase::ZERO;
    }
}

impl Default for Lfo {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Lfo {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("lfo", "LFO")
            .description("Low Frequency Oscillator for modulation")
            .category(ModuleCategory::LFO)
            .tag("lfo")
            .tag("modulation")
            .parameter(
                ParameterDescriptor::choice(
                    TypedParam::Lfo(LfoParam::Waveform),
                    "Waveform",
                    LfoWaveform::to_choices(),
                )
                .description("LFO waveform"),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Lfo(LfoParam::Rate), "Rate")
                    .description("LFO rate")
                    .range(0.01, 50.0)
                    .default(1.0)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::FrequencySlider)
                    .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Lfo(LfoParam::Depth), "Depth")
                    .description("Modulation depth")
                    .range(0.0, 1.0)
                    .default(1.0)
                    .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(TypedParam::Lfo(LfoParam::Phase), "Phase")
                    .description("Phase offset")
                    .range(0.0, 1.0)
                    .default(0.0)
                    .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::gate_input("retrigger", "Retrig").description("Reset phase on trigger"))
            .port(PortDescriptor::control_input("rate_cv", "Rate CV").description("Rate modulation"))
            .port(PortDescriptor::audio_output("out", "Out").description("LFO output"))
    }
}

impl VoiceModule for Lfo {
    fn process(
        &mut self,
        inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = SampleRate::new(context.sample_rate);
        self.output_buffer.resize(context.samples);

        let retrigger_input = inputs.get("retrigger");
        let rate_cv = inputs.get("rate_cv");

        let mut prev_retrigger = 0.0f32;

        for i in 0..context.samples {
            // Check retrigger
            if let Some(retrig) = retrigger_input {
                let val = retrig[i];
                if val > 0.5 && prev_retrigger <= 0.5 {
                    self.retrigger();
                }
                prev_retrigger = val;
            }

            // Rate modulation
            if let Some(cv) = rate_cv {
                let mod_amount = cv[i];
                // Exponential rate modulation (exp2 is faster than powf)
                let rate_mult = (mod_amount * 2.0).exp2();
                self.rate = Hertz::new((self.rate.as_f32() * rate_mult).clamp(0.01, 50.0));
            }

            self.output_buffer[i] = self.generate_sample(context.tempo);
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: TypedParam, value: TypedValue) {
        if let TypedParam::Lfo(lfo_param) = param {
            match lfo_param {
                LfoParam::Waveform => {
                    if let TypedValue::LfoWaveform(w) = value {
                        // Direct assignment - same type now!
                        self.waveform = w;
                    }
                }
                LfoParam::Rate => {
                    if let Some(r) = value.as_float() {
                        self.rate = Hertz::new(r.clamp(0.01, 50.0));
                    }
                }
                LfoParam::Depth => {
                    if let Some(d) = value.as_float() {
                        self.depth = NormalizedValue::new(d);
                    }
                }
                LfoParam::Phase => {
                    if let Some(p) = value.as_float() {
                        self.phase_offset = Phase::new(p);
                    }
                }
                LfoParam::TempoSync => {
                    if let TypedValue::Bool(sync) = value {
                        self.tempo_sync = sync;
                    }
                }
                LfoParam::Retrigger => {
                    // Not yet stored as parameter
                }
            }
        }
    }

    fn get_param(&self, param: TypedParam) -> Option<TypedValue> {
        if let TypedParam::Lfo(lfo_param) = param {
            match lfo_param {
                LfoParam::Waveform => {
                    // Direct use - same type now!
                    Some(TypedValue::LfoWaveform(self.waveform))
                }
                LfoParam::Rate => Some(TypedValue::Float(self.rate.as_f32())),
                LfoParam::Depth => Some(TypedValue::Float(self.depth.as_f32())),
                LfoParam::Phase => Some(TypedValue::Float(self.phase_offset.as_f32())),
                LfoParam::TempoSync => Some(TypedValue::Bool(self.tempo_sync)),
                LfoParam::Retrigger => None,
            }
        } else {
            None
        }
    }
    
    fn module_type(&self) -> ModuleType {
        ModuleType::Lfo
    }

    fn reset(&mut self) {
        self.phase = Phase::ZERO;
        self.sh_value = 0.0;
    }

    fn note_on(&mut self, _note: u8, _velocity: f32) {
        // Optionally retrigger on note on
        // self.retrigger();
    }

    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn VoiceModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lfo_creation() {
        let lfo = Lfo::new();
        assert_eq!(lfo.waveform, LfoWaveform::Sine);
        assert!((lfo.rate.as_f32() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_lfo_output_range_bipolar() {
        let mut lfo = Lfo::new();
        lfo.sample_rate = SampleRate::DVD_QUALITY;
        lfo.rate = Hertz::new(10.0);
        lfo.mode = LfoMode::Bipolar;
        lfo.depth = NormalizedValue::MAX;

        let mut min = f32::MAX;
        let mut max = f32::MIN;

        for _ in 0..10000 {
            let sample = lfo.generate_sample(120.0);
            min = min.min(sample);
            max = max.max(sample);
        }

        assert!(min >= -1.0 - 0.01);
        assert!(max <= 1.0 + 0.01);
    }

    #[test]
    fn test_lfo_output_range_unipolar() {
        let mut lfo = Lfo::new();
        lfo.sample_rate = SampleRate::DVD_QUALITY;
        lfo.rate = Hertz::new(10.0);
        lfo.mode = LfoMode::Unipolar;
        lfo.depth = NormalizedValue::MAX;

        let mut min = f32::MAX;
        let mut max = f32::MIN;

        for _ in 0..10000 {
            let sample = lfo.generate_sample(120.0);
            min = min.min(sample);
            max = max.max(sample);
        }

        assert!(min >= -0.01);
        assert!(max <= 1.01);
    }
}
