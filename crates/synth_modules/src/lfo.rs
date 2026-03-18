//! LFO (Low Frequency Oscillator) module.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{
    BeatDivision, BipolarValue, Hertz, MidiNote, NormalizedValue, Phase, PortName, RetriggerMode,
    SampleRate, SyncMode, Velocity,
};
use synth_core::{LfoParam, LfoWaveform, ModuleType, Param};

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
    sh_trigger_prev: NormalizedValue,
    sync_mode: SyncMode,
    sync_division: BeatDivision,
    retrigger_mode: RetriggerMode,
    /// Previous retrigger signal value for edge detection (persists across buffers).
    prev_retrigger: NormalizedValue,
    // Mod matrix offsets
    /// Rate offset (additive Hz, from mod matrix).
    mod_offset_rate: BipolarValue,
    /// Depth offset (additive, from mod matrix).
    mod_offset_depth: BipolarValue,
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
            sh_trigger_prev: NormalizedValue::MIN,
            sync_mode: SyncMode::Free,
            sync_division: BeatDivision::QUARTER,
            retrigger_mode: RetriggerMode::Continue,
            prev_retrigger: NormalizedValue::MIN,
            mod_offset_rate: BipolarValue::CENTER,
            mod_offset_depth: BipolarValue::CENTER,
            output_buffer: AudioBuffer::new(1024),
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
                if trigger > 0.5 && self.sh_trigger_prev.as_f32() <= 0.5 {
                    self.sh_value = self.random();
                }
                self.sh_trigger_prev = NormalizedValue::new(trigger);
                self.sh_value
            }
        };

        self.phase = self.phase.advance(phase_inc);

        let output = match self.mode {
            LfoMode::Bipolar => raw,
            LfoMode::Unipolar => crate::math::bipolar_to_unipolar(raw),
        };

        let effective_depth =
            (self.depth.as_f32() + self.mod_offset_depth.as_f32()).clamp(0.0, 1.0);
        output * effective_depth
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
                "waveform",
                Param::Lfo(LfoParam::Waveform(LfoWaveform::Sine)),
                "Waveform",
                LfoWaveform::to_choices(),
            ))
            .parameter(
                ParameterDescriptor::float("rate", Param::Lfo(LfoParam::Rate(Hertz::new(1.0))), "Rate")
                    .range(0.01, 50.0)
                    .default(1.0)
                    .unit(ParameterUnit::Hertz)
                    .widget(WidgetHint::FrequencySlider)
                    .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "depth",
                    Param::Lfo(LfoParam::Depth(NormalizedValue::MAX)),
                    "Depth",
                )
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float("phase", Param::Lfo(LfoParam::Phase(Phase::ZERO)), "Phase")
                    .range(0.0, 1.0)
                    .default(0.0)
                    .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::gate_input("retrigger", "Retrig").description("Återstartar LFO-cykeln. Koppla: Envelope Gate, annan LFO, Euclidean Gate"))
            .port(PortDescriptor::control_input("rate_cv", "Rate CV").description("Modulerar LFO-hastighet. Koppla: annan LFO, Envelope, Kinetic Modulator"))
            .port(PortDescriptor::audio_output("out", "Out").description("LFO-signal (±1). Koppla till: Oscillator FM/PM/PWM, Filter Cutoff CV, Amplifier CV"))
    }
}

impl PolyModule for Lfo {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let retrigger_reader = inputs.reader(PortName::RETRIGGER, 0.0);
        let rate_cv_reader = inputs.reader(PortName::RATE_CV, 0.0);

        // In tempo sync mode, derive phase directly from beat position for lock
        let use_beat_sync = self.sync_mode.is_tempo_sync() && context.is_playing;

        for i in 0..context.samples.as_usize() {
            // Always track retrigger input for edge detection, but only act on it
            // when retrigger_mode is enabled. This prevents stale prev_retrigger
            // state when the mode is toggled.
            if retrigger_reader.is_connected() {
                let val = retrigger_reader[i];
                if self.retrigger_mode.should_retrigger()
                    && crate::math::rising_edge(val, self.prev_retrigger.as_f32())
                {
                    self.retrigger();
                }
                self.prev_retrigger = NormalizedValue::new(val);
            }

            if use_beat_sync {
                // Calculate phase from beat position for perfect sync
                let sample_offset = i as f32 / self.sample_rate.as_f32();
                let beat_offset = sample_offset * context.tempo.as_f32() / 60.0;
                let current_beat = context.position_beats.as_f32() + beat_offset;
                // Phase cycles through one LFO cycle per sync_division beats
                let lfo_phase = (current_beat / self.sync_division.as_f32()).rem_euclid(1.0);
                self.phase = Phase::new(lfo_phase);
                self.output_buffer[i] = self.generate_sample(Hertz::new(1.0)); // Rate doesn't matter in sync
            } else {
                let base_rate = if self.sync_mode.is_tempo_sync() {
                    Hertz::new(context.tempo.as_f32() / 60.0 / self.sync_division.as_f32())
                } else {
                    // Apply mod matrix rate offset
                    Hertz::new(
                        (self.rate.as_f32() + self.mod_offset_rate.as_f32()).clamp(0.01, 50.0),
                    )
                };

                let effective_rate = if rate_cv_reader.is_connected() {
                    let mod_amount = rate_cv_reader[i];
                    Hertz::new(base_rate.apply_fm(mod_amount).as_f32().clamp(0.01, 50.0))
                } else {
                    base_rate
                };

                self.output_buffer[i] = self.generate_sample(effective_rate);
            }
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
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
        self.prev_retrigger = NormalizedValue::MIN;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}
    fn note_off(&mut self) {}

    fn set_mod_offset(&mut self, dest_index: u8, value: f32) {
        match dest_index {
            0 => self.mod_offset_rate = BipolarValue::new(self.mod_offset_rate.as_f32() + value),
            1 => self.mod_offset_depth = BipolarValue::new(self.mod_offset_depth.as_f32() + value),
            _ => {}
        }
    }

    fn clear_mod_offsets(&mut self) {
        self.mod_offset_rate = BipolarValue::CENTER;
        self.mod_offset_depth = BipolarValue::CENTER;
    }

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
