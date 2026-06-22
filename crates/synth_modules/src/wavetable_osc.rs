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
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{
    BipolarValue, Cents, Gain, Hertz, MidiNote, NormalizedValue, Octaves, Phase, PortName,
    SampleRate, Velocity,
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
    octave: Octaves,
    level: Gain,

    // State
    phase: Phase,
    note_freq: Hertz,
    sample_rate: SampleRate,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

    // Buffers
    output_buffer: AudioBuffer,
}

impl WavetableOsc {
    pub fn new() -> Self {
        Self {
            table: WavetableSelect::default(),
            position: NormalizedValue::MIN,
            detune: Cents::ZERO,
            octave: Octaves::ZERO,
            level: Gain::new(0.8),
            phase: Phase::ZERO,
            note_freq: Hertz::A4,
            sample_rate: SampleRate::DVD_QUALITY,
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Compute effective frequency from note + detune + octave.
    #[inline]
    fn effective_freq(&self) -> Hertz {
        let mut freq = self.note_freq.as_f32();

        // Apply octave offset
        if self.octave != Octaves::ZERO {
            freq *= (2.0_f32).powi(self.octave.as_i32());
        }

        // Apply detune in cents (modulatable via the generic store — its ±cents
        // range makes a normalized offset land in musical cents).
        let detune_cents = self.mod_offsets.effective("detune", self.detune.as_f32());
        if detune_cents.abs() > 0.001 {
            freq *= crate::math::cents_to_ratio(detune_cents);
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
                    "table",
                    Param::WavetableOsc(WavetableParam::Table(WavetableSelect::Basic)),
                    "Table",
                    WavetableSelect::to_choices(),
                )
                .description("Wavetable bank selection")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    "position",
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
                    "detune",
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
                    "octave",
                    Param::WavetableOsc(WavetableParam::Octave(Octaves::ZERO)),
                    "Octave",
                )
                .description("Octave offset (-2 to +2)")
                .range(-2.0, 2.0)
                .default(0.0)
                .unit(ParameterUnit::Semitones) // Using semitones as closest unit
                // Quantized to integer octaves in `effective_freq`, so a smooth
                // mod offset is meaningless — not a mod destination.
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
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
                PortDescriptor::control_input("fm", "FM").description(
                    "Modulates pitch. Connect: LFO for vibrato, Envelope for pitch sweep",
                ),
            )
            .port(
                PortDescriptor::control_input("pos_cv", "Pos CV").description(
                    "Modulates wavetable position. Connect: LFO, Envelope, Kinetic Modulator",
                ),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Wavetable output. Connect to: Amplifier In, Filter In"),
            )
    }
}

impl PolyModule for WavetableOsc {
    #[allow(clippy::cast_possible_truncation)]
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        let fm_input = inputs.get(PortName::FM);
        let pos_cv = inputs.get(PortName::POS_CV);

        let bank = get_wavetable(self.table);
        let base_freq = self.effective_freq();
        // Effective (modulated) values, once per block.
        let base_position = self
            .mod_offsets
            .effective("position", self.position.as_f32());
        let level = self.mod_offsets.effective("level", self.level.as_f32());

        for i in 0..num_samples {
            // Apply FM
            let freq = if let Some(fm) = fm_input {
                let fm_val = crate::math::sanitize_cv(fm[i]);
                base_freq.apply_cv(BipolarValue::new(fm_val))
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

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
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
                WavetableParam::Octave(o) => self.octave = Octaves::new(o.as_i32().clamp(-2, 2)),
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
                WavetableParam::Octave(_) => self.octave.as_i32() as f32,
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

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
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

    /// `detune` is now a working pitch mod destination (it used to be dropped):
    /// a normalized offset shifts the effective frequency, clearing reverts.
    #[test]
    fn detune_mod_offset_shifts_pitch() {
        let mut o = WavetableOsc::new();
        let desc = o.descriptor();
        o.mod_offsets_mut().unwrap().populate(&desc);
        o.note_on(MidiNote::A4, Velocity::MAX);

        let base = o.effective_freq().as_f32();
        o.set_mod_offset("detune", 0.5); // +half the cents range → pitch up
        assert!(
            o.effective_freq().as_f32() > base * 1.001,
            "detune mod should raise pitch"
        );
        o.clear_mod_offsets();
        assert!((o.effective_freq().as_f32() - base).abs() < 0.5);
    }

    #[test]
    fn test_wavetable_osc_creation() {
        let wt = WavetableOsc::new();
        assert_eq!(wt.table, WavetableSelect::Basic);
        assert_eq!(wt.octave, Octaves::ZERO);
    }

    #[test]
    fn test_wavetable_osc_produces_output() {
        let mut wt = WavetableOsc::new();
        wt.note_on(MidiNote::new(69), Velocity::new(100.0)); // A4

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(1024));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        wt.process(InputPorts::empty(), &mut outputs, &context);

        // Should produce non-zero output
        let out = &outputs[&PortName::OUT];
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

        wt.set_param(Param::WavetableOsc(WavetableParam::Octave(Octaves::new(
            -1,
        ))));
        assert_eq!(wt.octave, Octaves::new(-1));

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
            outputs.insert(PortName::OUT, AudioBuffer::new(128));

            let context = ProcessContext {
                samples: synth_core::SampleCount::new(128),
                sample_rate: SampleRate::DVD_QUALITY,
                ..ProcessContext::default()
            };

            wt.process(InputPorts::empty(), &mut outputs, &context);

            let out = &outputs[&PortName::OUT];
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
