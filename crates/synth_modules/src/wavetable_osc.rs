//! Wavetable Oscillator module.
//!
//! A scannable wavetable synthesizer with multiple built-in wavetable banks.
//! Scanning through the table position morphs between different waveform shapes.
//!
//! Features:
//! - 6 built-in wavetable banks (Basic, Harmonics, PWM, Formant, Digital, Warm)
//! - Position scanning with CV modulation
//! - Detune in cents
//! - Octave offset
//! - FM input for frequency modulation

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{
    Cents, Gain, Hertz, MidiNote, NormalizedValue, Phase, PortName, SampleRate, Velocity,
};
use synth_core::{ModuleType, Param, WavetableParam, WavetableSelect};

use crate::wavetable_data::get_wavetable;

/// Wavetable oscillator voice module.
#[derive(Clone)]
pub struct WavetableOsc {
    // Parameters
    table: WavetableSelect,
    position: NormalizedValue,
    detune: Cents,
    octave: i8,
    level: Gain,

    // State
    phase: Phase,
    note_freq: Hertz,
    sample_rate: SampleRate,

    // Buffers
    output_buffer: AudioBuffer,
}

impl WavetableOsc {
    pub fn new() -> Self {
        Self {
            table: WavetableSelect::default(),
            position: NormalizedValue::MIN,
            detune: Cents::ZERO,
            octave: 0,
            level: Gain::new(0.8),
            phase: Phase::ZERO,
            note_freq: Hertz::A4,
            sample_rate: SampleRate::DVD_QUALITY,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Compute effective frequency from note + detune + octave.
    #[inline]
    fn effective_freq(&self) -> Hertz {
        let mut freq = self.note_freq.as_f32();

        // Apply octave offset
        if self.octave != 0 {
            freq *= (2.0_f32).powi(i32::from(self.octave));
        }

        // Apply detune in cents
        let detune_cents = self.detune.as_f32();
        if detune_cents.abs() > 0.001 {
            freq *= (2.0_f32).powf(detune_cents / 1200.0);
        }

        Hertz::new(freq)
    }
}

impl Default for WavetableOsc {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for WavetableOsc {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("wavetable_osc", "Wavetable")
            .description("Wavetable oscillator with scannable waveform banks")
            .category(ModuleCategory::Oscillator)
            .tag("wavetable")
            .tag("oscillator")
            .tag("source")
            .parameter(
                ParameterDescriptor::choice(
                    Param::WavetableOsc(WavetableParam::Table(WavetableSelect::Basic)),
                    "Table",
                    WavetableSelect::to_choices(),
                )
                .description("Wavetable bank selection")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::WavetableOsc(WavetableParam::Position(NormalizedValue::MIN)),
                    "Position",
                )
                .description("Scan position within the wavetable")
                .range(0.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::WavetableOsc(WavetableParam::Detune(Cents::ZERO)),
                    "Detune",
                )
                .description("Detune in cents")
                .range(-100.0, 100.0)
                .default(0.0)
                .unit(ParameterUnit::Cents)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::WavetableOsc(WavetableParam::Octave(0)),
                    "Octave",
                )
                .description("Octave offset (-2 to +2)")
                .range(-2.0, 2.0)
                .default(0.0)
                .unit(ParameterUnit::Semitones) // Using semitones as closest unit
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::WavetableOsc(WavetableParam::Level(Gain::new(0.8))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.8)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("fm", "FM").description("Frequency modulation input"),
            )
            .port(
                PortDescriptor::control_input("pos_cv", "Pos CV")
                    .description("Position modulation CV"),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Wavetable output"))
    }
}

impl PolyModule for WavetableOsc {
    #[allow(clippy::cast_possible_truncation)]
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        let fm_input = inputs.get(PortName::FM);
        let pos_cv = inputs.get(PortName::intern("pos_cv"));

        let bank = get_wavetable(self.table);
        let base_freq = self.effective_freq();
        let base_position = self.position.as_f32();
        let level = self.level.as_f32();

        for i in 0..num_samples {
            // Apply FM
            let freq = if let Some(fm) = fm_input {
                let fm_val = fm[i];
                Hertz::new(base_freq.as_f32() * (2.0_f32).powf(fm_val))
            } else {
                base_freq
            };

            // Apply position CV
            let position = if let Some(cv) = pos_cv {
                (base_position + cv[i]).clamp(0.0, 1.0)
            } else {
                base_position
            };

            // Sample from wavetable
            let sample = bank.sample(position, self.phase.as_f32());

            // Advance phase
            let dt = freq.phase_increment(self.sample_rate);
            self.phase = self.phase.advance(dt);

            self.output_buffer[i] = sample * level;
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::WavetableOsc(p) = param {
            match p {
                WavetableParam::Table(t) => self.table = t,
                WavetableParam::Position(v) => self.position = v,
                WavetableParam::Detune(c) => {
                    self.detune = Cents::new(c.as_f32().clamp(-100.0, 100.0));
                }
                WavetableParam::Octave(o) => self.octave = o.clamp(-2, 2),
                WavetableParam::Level(g) => {
                    self.level = Gain::new(g.as_f32().clamp(0.0, 1.0));
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::WavetableOsc(p) = param {
            Some(match p {
                WavetableParam::Table(_) => self.table.index() as f32,
                WavetableParam::Position(_) => self.position.as_f32(),
                WavetableParam::Detune(_) => self.detune.as_f32(),
                WavetableParam::Octave(_) => f32::from(self.octave),
                WavetableParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::WavetableOsc(WavetableParam::Table(self.table)),
            Param::WavetableOsc(WavetableParam::Position(self.position)),
            Param::WavetableOsc(WavetableParam::Detune(self.detune)),
            Param::WavetableOsc(WavetableParam::Octave(self.octave)),
            Param::WavetableOsc(WavetableParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::WavetableOsc
    }

    fn reset(&mut self) {
        self.phase = Phase::ZERO;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.phase = Phase::ZERO;
    }

    fn note_off(&mut self) {
        // Nothing to do
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wavetable_osc_creation() {
        let wt = WavetableOsc::new();
        assert_eq!(wt.table, WavetableSelect::Basic);
        assert_eq!(wt.octave, 0);
    }

    #[test]
    fn test_wavetable_osc_produces_output() {
        let mut wt = WavetableOsc::new();
        wt.note_on(MidiNote::new(69), Velocity::new(100.0)); // A4

        let mut outputs = HashMap::new();
        outputs.insert("out".to_string(), AudioBuffer::new(256));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        wt.process(InputPorts::empty(), &mut outputs, &context);

        // Should produce non-zero output
        let out = &outputs["out"];
        let max_abs = (0..256).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(
            max_abs > 0.01,
            "Expected non-zero output, max abs was {}",
            max_abs
        );
    }

    #[test]
    fn test_wavetable_osc_params() {
        let mut wt = WavetableOsc::new();
        wt.set_param(Param::WavetableOsc(WavetableParam::Position(
            NormalizedValue::new(0.5),
        )));
        assert!((wt.position.as_f32() - 0.5).abs() < 0.001);

        wt.set_param(Param::WavetableOsc(WavetableParam::Octave(-1)));
        assert_eq!(wt.octave, -1);

        let params = wt.get_params();
        assert_eq!(params.len(), 5);
    }

    #[test]
    fn test_wavetable_osc_all_tables() {
        for table_select in WavetableSelect::ALL {
            let mut wt = WavetableOsc::new();
            wt.table = table_select;
            wt.note_on(MidiNote::new(69), Velocity::new(100.0));

            let mut outputs = HashMap::new();
            outputs.insert("out".to_string(), AudioBuffer::new(128));

            let context = ProcessContext {
                samples: synth_core::SampleCount::new(128),
                sample_rate: SampleRate::DVD_QUALITY,
                ..ProcessContext::default()
            };

            wt.process(InputPorts::empty(), &mut outputs, &context);

            let out = &outputs["out"];
            for i in 0..128 {
                assert!(
                    out[i].abs() <= 1.5, // Allow some headroom
                    "{:?} produced extreme sample: {} at index {}",
                    table_select,
                    out[i],
                    i
                );
            }
        }
    }
}
