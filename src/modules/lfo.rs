//! LFO (Low Frequency Oscillator) module.

use std::collections::HashMap;

use crate::engine::typed_params::{LfoParam, LfoWaveform, ModuleType, Param};
use crate::modules::core::*;
use crate::types::{
    BeatDivision, Hertz, MidiNote, NormalizedValue, Phase, RetriggerMode, SampleRate, SyncMode,
};

/// LFO output mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoMode {
    Bipolar,
    Unipolar,
}

/// Low Frequency Oscillator.
#[derive(Clone)]
pub struct Lfo {
    waveform: LfoWaveform,
    rate: Hertz,
    depth: NormalizedValue,
    phase_offset: Phase,
    mode: LfoMode,
    phase: Phase,
    sample_rate: SampleRate,
    sh_value: f32,
    sh_trigger_prev: f32,
    sync_mode: SyncMode,
    sync_division: BeatDivision,
    retrigger_mode: RetriggerMode,
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
            sync_mode: SyncMode::Free,
            sync_division: BeatDivision::QUARTER,
            retrigger_mode: RetriggerMode::Continue,
            output_buffer: AudioBuffer::new(256),
        }
    }

    #[inline]
    fn random(&self) -> f32 {
        fastrand::f32() * 2.0 - 1.0
    }

    #[inline]
    fn generate_sample(&mut self, effective_rate: Hertz) -> f32 {
        let phase_inc = effective_rate.phase_increment(self.sample_rate);
        let phase = self.phase.advance(self.phase_offset.as_f32()).as_f32();
        let phase_wrapped = Phase::new_unchecked(phase);

        let raw = match self.waveform {
            LfoWaveform::Sine => phase_wrapped.sin(),
            LfoWaveform::Triangle => phase_wrapped.triangle(),
            LfoWaveform::Sawtooth => phase_wrapped.sawtooth(),
            LfoWaveform::Square => phase_wrapped.pulse(NormalizedValue::CENTER),
            LfoWaveform::SampleAndHold => {
                let trigger = if phase < phase_inc { 1.0 } else { 0.0 };
                if trigger > 0.5 && self.sh_trigger_prev <= 0.5 {
                    self.sh_value = self.random();
                }
                self.sh_trigger_prev = trigger;
                self.sh_value
            }
        };

        self.phase = self.phase.advance(phase_inc);

        let output = match self.mode {
            LfoMode::Bipolar => raw,
            LfoMode::Unipolar => (raw + 1.0) * 0.5,
        };

        output * self.depth.as_f32()
    }

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
            .description("Low Frequency Oscillator")
            .category(ModuleCategory::LFO)
            .tag("lfo")
            .tag("modulation")
            .parameter(ParameterDescriptor::choice(
                Param::Lfo(LfoParam::Waveform(LfoWaveform::Sine)),
                "Waveform",
                LfoWaveform::to_choices(),
            ))
            .parameter(
                ParameterDescriptor::float(Param::Lfo(LfoParam::Rate(Hertz::new(1.0))), "Rate")
                    .range(0.01, 50.0)
                    .default(1.0)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::FrequencySlider)
                    .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Lfo(LfoParam::Depth(NormalizedValue::MAX)),
                    "Depth",
                )
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(Param::Lfo(LfoParam::Phase(Phase::ZERO)), "Phase")
                    .range(0.0, 1.0)
                    .default(0.0)
                    .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::gate_input("retrigger", "Retrig"))
            .port(PortDescriptor::control_input("rate_cv", "Rate CV"))
            .port(PortDescriptor::audio_output("out", "Out"))
    }
}

impl PolyModule for Lfo {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        let retrigger_input = inputs.get("retrigger");
        let rate_cv = inputs.get("rate_cv");
        let mut prev_retrigger = 0.0f32;

        for i in 0..context.samples {
            if let Some(retrig) = retrigger_input {
                let val = retrig[i];
                if val > 0.5 && prev_retrigger <= 0.5 {
                    self.retrigger();
                }
                prev_retrigger = val;
            }

            let base_rate = if self.sync_mode.is_tempo_sync() {
                Hertz::new(context.tempo.as_f32() / 60.0 / self.sync_division.as_f32())
            } else {
                self.rate
            };

            let effective_rate = if let Some(cv) = rate_cv {
                let mod_amount = cv[i];
                let rate_mult = (mod_amount * 2.0).exp2();
                Hertz::new((base_rate.as_f32() * rate_mult).clamp(0.01, 50.0))
            } else {
                base_rate
            };

            self.output_buffer[i] = self.generate_sample(effective_rate);
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Lfo(lfo_param) = param {
            match lfo_param {
                LfoParam::Waveform(w) => self.waveform = w,
                LfoParam::Rate(r) => self.rate = Hertz::new(r.as_f32().clamp(0.01, 50.0)),
                LfoParam::Depth(d) => self.depth = d,
                LfoParam::Phase(p) => self.phase_offset = p,
                LfoParam::TempoSync(s) => self.sync_mode = SyncMode::from(s),
                LfoParam::SyncDivision(d) => self.sync_division = d,
                LfoParam::Retrigger(r) => self.retrigger_mode = RetriggerMode::from(r),
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Lfo(lfo_param) = param {
            Some(match lfo_param {
                LfoParam::Waveform(_) => self.waveform.index() as f32,
                LfoParam::Rate(_) => self.rate.as_f32(),
                LfoParam::Depth(_) => self.depth.as_f32(),
                LfoParam::Phase(_) => self.phase_offset.as_f32(),
                LfoParam::TempoSync(_) => {
                    if self.sync_mode.is_tempo_sync() {
                        1.0
                    } else {
                        0.0
                    }
                }
                LfoParam::SyncDivision(_) => self.sync_division.as_f32(),
                LfoParam::Retrigger(_) => {
                    if self.retrigger_mode.should_retrigger() {
                        1.0
                    } else {
                        0.0
                    }
                }
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Lfo(LfoParam::Waveform(self.waveform)),
            Param::Lfo(LfoParam::Rate(self.rate)),
            Param::Lfo(LfoParam::Depth(self.depth)),
            Param::Lfo(LfoParam::Phase(self.phase_offset)),
            Param::Lfo(LfoParam::TempoSync(self.sync_mode.is_tempo_sync())),
            Param::Lfo(LfoParam::SyncDivision(self.sync_division)),
            Param::Lfo(LfoParam::Retrigger(self.retrigger_mode.should_retrigger())),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Lfo
    }

    fn reset(&mut self) {
        self.phase = Phase::ZERO;
        self.sh_value = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: f32) {}
    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn PolyModule> {
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
    }
}
