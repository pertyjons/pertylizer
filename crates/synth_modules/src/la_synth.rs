//! LA (Linear Arithmetic) Synthesis module.
//!
//! Roland's LA synthesis technique: a short synthesized attack transient
//! (click, noise burst, pluck, or hammer) is crossfaded into a sustained
//! tone from an audio input. This gives natural-sounding attack characteristics
//! to any sustained sound.
//!
//! The module generates the attack transient internally and crossfades to
//! the input signal over a configurable time.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{
    LaSynthParam, MidiNote, Milliseconds, NormalizedValue, PortName, SampleRate, Velocity,
};
use synth_core::{ModuleType, Param};

/// LA Synthesis voice module.
#[derive(Clone)]
pub struct LaSynth {
    // Parameters
    attack_type: NormalizedValue,
    attack_time: Milliseconds,
    attack_level: NormalizedValue,
    crossfade_time: Milliseconds,
    brightness: NormalizedValue,

    // State
    sample_rate: SampleRate,
    phase: f32,          // attack transient phase (samples since note-on)
    attack_samples: u32, // attack duration in samples
    xfade_samples: u32,  // crossfade duration in samples
    total_samples: u32,  // attack + crossfade duration
    active: bool,        // whether in attack/crossfade phase
    noise_state: u32,    // simple noise PRNG state

    // One-pole lowpass for brightness
    filter_state: f32,

    // Buffers
    output_buffer: AudioBuffer,
}

impl LaSynth {
    pub fn new() -> Self {
        let mut s = Self {
            attack_type: NormalizedValue::MIN,
            attack_time: Milliseconds::new(30.0),
            attack_level: NormalizedValue::new(0.7),
            crossfade_time: Milliseconds::new(50.0),
            brightness: NormalizedValue::new(0.5),
            sample_rate: SampleRate::DVD_QUALITY,
            phase: 0.0,
            attack_samples: 0,
            xfade_samples: 0,
            total_samples: 0,
            active: false,
            noise_state: 0x1234_5678,
            filter_state: 0.0,
            output_buffer: AudioBuffer::new(1024),
        };
        s.update_times();
        s
    }

    fn update_times(&mut self) {
        let sr = self.sample_rate.as_f32();
        self.attack_samples = (self.attack_time.as_f32() * 0.001 * sr) as u32;
        self.xfade_samples = (self.crossfade_time.as_f32() * 0.001 * sr) as u32;
        self.total_samples = self.attack_samples + self.xfade_samples;
        if self.total_samples == 0 {
            self.total_samples = 1;
        }
    }

    /// Generate attack transient sample based on attack_type.
    #[inline]
    fn generate_attack(&mut self) -> f32 {
        let t = self.attack_type.as_f32();
        let level = self.attack_level.as_f32();

        let raw = if t < 0.25 {
            // Click: sharp impulse decaying exponentially
            let phase_norm = self.phase / self.attack_samples.max(1) as f32;
            (-phase_norm * 10.0).exp()
        } else if t < 0.5 {
            // Noise burst: filtered white noise with decay
            let phase_norm = self.phase / self.attack_samples.max(1) as f32;
            let noise = self.next_noise();
            noise * (-phase_norm * 5.0).exp()
        } else if t < 0.75 {
            // Pluck: decaying sine with slight detuning
            let phase_norm = self.phase / self.attack_samples.max(1) as f32;
            let freq = 800.0 + 400.0 * (1.0 - phase_norm); // descending pitch
            let t_sec = self.phase / self.sample_rate.as_f32();
            (t_sec * freq * std::f32::consts::TAU).sin() * (-phase_norm * 6.0).exp()
        } else {
            // Hammer: short broadband noise followed by resonant decay
            let phase_norm = self.phase / self.attack_samples.max(1) as f32;
            if phase_norm < 0.2 {
                // Initial impact
                self.next_noise() * (1.0 - phase_norm * 5.0)
            } else {
                // Resonant tail
                let t_sec = self.phase / self.sample_rate.as_f32();
                (t_sec * 300.0 * std::f32::consts::TAU).sin()
                    * (-(phase_norm - 0.2) * 8.0).exp()
                    * 0.5
            }
        };

        // Apply brightness filter (one-pole lowpass)
        let cutoff = 0.1 + self.brightness.as_f32() * 0.89; // 0.1 to 0.99
        self.filter_state += cutoff * (raw - self.filter_state);

        self.filter_state * level
    }

    /// Simple noise generator (no allocation).
    #[inline]
    fn next_noise(&mut self) -> f32 {
        // Xorshift32
        self.noise_state ^= self.noise_state << 13;
        self.noise_state ^= self.noise_state >> 17;
        self.noise_state ^= self.noise_state << 5;
        // Map to [-1, 1]
        (self.noise_state as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

impl Default for LaSynth {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for LaSynth {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("la_synth", "LA Synth")
            .description(
                "Linear Arithmetic synthesis — attack transient crossfaded to sustain input",
            )
            .category(ModuleCategory::Oscillator)
            .tag("la_synth")
            .tag("synthesis")
            .tag("transient")
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Sustain signal input. Koppla: Oscillator Out, Wavetable Out"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out").description(
                    "Output with attack transient. Koppla till: Filter In, Amplifier In",
                ),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::LaSynth(LaSynthParam::AttackType(NormalizedValue::MIN)),
                    "Attack Type",
                )
                .description("0=Click, 0.33=Noise burst, 0.66=Pluck, 1.0=Hammer")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::LaSynth(LaSynthParam::AttackTime(Milliseconds::new(30.0))),
                    "Attack Time",
                )
                .description("Duration of attack transient")
                .range(1.0, 200.0)
                .default(30.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::LaSynth(LaSynthParam::AttackLevel(NormalizedValue::new(0.7))),
                    "Attack Level",
                )
                .description("Level of attack transient")
                .range(0.0, 1.0)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::LaSynth(LaSynthParam::CrossfadeTime(Milliseconds::new(50.0))),
                    "X-Fade Time",
                )
                .description("Crossfade time from attack to sustain")
                .range(1.0, 500.0)
                .default(50.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::LaSynth(LaSynthParam::Brightness(NormalizedValue::new(0.5))),
                    "Brightness",
                )
                .description("Brightness filter on attack transient")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
    }
}

impl PolyModule for LaSynth {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        let input = inputs.get(PortName::IN);

        for i in 0..num_samples {
            let sustain = input.map_or(0.0, |buf| buf[i]);

            if self.active && (self.phase as u32) < self.total_samples {
                let sample_pos = self.phase as u32;

                if sample_pos < self.attack_samples {
                    // Pure attack phase
                    self.output_buffer[i] = self.generate_attack();
                } else {
                    // Crossfade phase
                    let xfade_pos = sample_pos - self.attack_samples;
                    let xfade_amt = xfade_pos as f32 / self.xfade_samples.max(1) as f32;
                    let xfade_amt = xfade_amt.clamp(0.0, 1.0);

                    let attack = self.generate_attack();
                    // Equal-power crossfade
                    let attack_gain = ((1.0 - xfade_amt) * std::f32::consts::FRAC_PI_2).sin();
                    let sustain_gain = (xfade_amt * std::f32::consts::FRAC_PI_2).sin();
                    self.output_buffer[i] = attack * attack_gain + sustain * sustain_gain;
                }

                self.phase += 1.0;

                if self.phase as u32 >= self.total_samples {
                    self.active = false;
                }
            } else {
                // Past attack/crossfade: pure sustain
                self.output_buffer[i] = sustain;
            }
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::LaSynth(p) = param {
            match p {
                LaSynthParam::AttackType(v) => self.attack_type = v,
                LaSynthParam::AttackTime(v) => {
                    self.attack_time = Milliseconds::new(v.as_f32().clamp(1.0, 200.0));
                    self.update_times();
                }
                LaSynthParam::AttackLevel(v) => self.attack_level = v,
                LaSynthParam::CrossfadeTime(v) => {
                    self.crossfade_time = Milliseconds::new(v.as_f32().clamp(1.0, 500.0));
                    self.update_times();
                }
                LaSynthParam::Brightness(v) => self.brightness = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::LaSynth(p) = param {
            Some(match p {
                LaSynthParam::AttackType(_) => self.attack_type.as_f32(),
                LaSynthParam::AttackTime(_) => self.attack_time.as_f32(),
                LaSynthParam::AttackLevel(_) => self.attack_level.as_f32(),
                LaSynthParam::CrossfadeTime(_) => self.crossfade_time.as_f32(),
                LaSynthParam::Brightness(_) => self.brightness.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::LaSynth(LaSynthParam::AttackType(self.attack_type)),
            Param::LaSynth(LaSynthParam::AttackTime(self.attack_time)),
            Param::LaSynth(LaSynthParam::AttackLevel(self.attack_level)),
            Param::LaSynth(LaSynthParam::CrossfadeTime(self.crossfade_time)),
            Param::LaSynth(LaSynthParam::Brightness(self.brightness)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::LaSynth
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.active = false;
        self.filter_state = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        self.phase = 0.0;
        self.active = true;
        self.filter_state = 0.0;
        self.update_times();
    }

    fn note_off(&mut self) {
        // Attack/crossfade continues until finished; sustain handled by amp envelope
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;

    #[test]
    fn test_la_synth_creation() {
        let la = LaSynth::new();
        assert_eq!(la.attack_type.as_f32(), 0.0);
        assert_eq!(la.attack_time.as_f32(), 30.0);
    }

    #[test]
    fn test_passthrough_without_note() {
        let mut la = LaSynth::new();
        let num = 64;
        let mut in_buf = AudioBuffer::new(num);
        for i in 0..num {
            in_buf[i] = 0.5;
        }

        let mut outputs = HashMap::new();
        outputs.insert("out".to_string(), AudioBuffer::new(num));

        let context = ProcessContext {
            samples: SampleCount::new(num),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        let inputs = InputPorts::from_single("in", &in_buf);
        la.process(inputs, &mut outputs, &context);

        // Without note_on, should pass through sustain
        let out = &outputs["out"];
        for i in 0..num {
            assert!(
                (out[i] - 0.5).abs() < 0.001,
                "Expected passthrough, got {} at {}",
                out[i],
                i
            );
        }
    }

    #[test]
    fn test_la_synth_params() {
        let mut la = LaSynth::new();
        la.set_param(Param::LaSynth(LaSynthParam::AttackLevel(
            NormalizedValue::new(0.9),
        )));
        assert!((la.attack_level.as_f32() - 0.9).abs() < 0.001);

        let params = la.get_params();
        assert_eq!(params.len(), 5);
    }
}
