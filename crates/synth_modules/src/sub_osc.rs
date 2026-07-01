//! Sub-Oscillator module for adding bass weight.
//!
//! A dedicated sub-oscillator that tracks the main pitch at -1 or -2 octaves.
//! Essential for adding "fatness" to bass sounds without sacrificing a main oscillator.
//!
//! Features:
//! - -1 or -2 octave transposition
//! - Sine, Triangle, Saw, Square, 25% Pulse, or DSF Saw waveforms
//! - Level control
//! - Follows note input

use std::collections::HashMap;
use std::f32::consts::TAU;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, PortName, ProcessContext,
    WidgetHint,
};
use synth_core::{
    BipolarValue, Gain, Hertz, MidiNote, NormalizedValue, Phase, SampleRate, Velocity,
};
use synth_core::{ModuleType, Param, SubOscOctave, SubOscParam, SubOscWaveform};

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
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

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
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Generate a single sample.
    #[inline]
    fn generate_sample(&mut self, freq: Hertz) -> f32 {
        let phase = self.phase.as_f32();

        let sample = match self.waveform {
            SubOscWaveform::Sine => (phase * TAU).sin(),
            SubOscWaveform::Triangle => self.phase.triangle(),
            SubOscWaveform::Sawtooth => self.phase.sawtooth(),
            SubOscWaveform::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            SubOscWaveform::Pulse25 => {
                if phase < 0.25 {
                    1.0
                } else {
                    -1.0
                }
            }
            SubOscWaveform::DsfSaw => {
                // Band-limited sawtooth (harmonic sum) — alias-free even though
                // the other shapes are naive; cheap here because sub frequencies
                // are low so the harmonic count stays small.
                #[allow(clippy::cast_possible_truncation)]
                let num_harmonics =
                    (self.sample_rate.as_f32() / (2.0 * freq.as_f32())).max(1.0) as u32;
                synth_dsp::oscillators::dsf_sawtooth(
                    self.phase,
                    num_harmonics,
                    NormalizedValue::new(0.95),
                )
            }
        };

        // Advance phase
        let dt = freq.phase_increment(self.sample_rate);
        self.phase = self.phase.advance(dt);

        // Level (and its mod offset) is applied once per block in `process`.
        sample
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
            .width(synth_core::ModuleWidth::ExtraLarge)
            .description("Sub-oscillator for bass reinforcement at -1 or -2 octaves")
            .category(ModuleCategory::Oscillator)
            .tag("oscillator")
            .tag("sub")
            .tag("bass")
            .parameter(
                ParameterDescriptor::choice(
                    "waveform",
                    Param::SubOsc(SubOscParam::Waveform(SubOscWaveform::Square)),
                    "Waveform",
                    SubOscWaveform::to_choices(),
                )
                .description("Sub-oscillator waveform")
                .widget(WidgetHint::WaveformSelector),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "octave",
                    Param::SubOsc(SubOscParam::Octave(SubOscOctave::MinusOne)),
                    "Octave",
                    SubOscOctave::to_choices(),
                )
                .description("Octave transposition")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::SubOsc(SubOscParam::Level(Gain::new(0.5))),
                    "Level",
                )
                .description("Sub-oscillator output level")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("pitch_cv", "Pitch CV")
                    .description("1V/oct pitch offset (octaves). Connect: LFO, Envelope, Pitch"),
            )
            .port(
                PortDescriptor::control_input("level_cv", "Level CV").description(
                    "Modulate output level (added to the Level knob). Connect: LFO, Envelope",
                ),
            )
            .port(
                PortDescriptor::audio_output("out", "Out").description(
                    "Sub-oscillator output. Connect to: Amplifier In, Filter In, Mixer",
                ),
            )
    }
}

impl PolyModule for SubOscillator {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        // Base sub frequency (note pitch / octave divisor), constant per block;
        // the Pitch CV input shifts it 1V/oct per sample. Base level = knob +
        // mod-matrix offset, with the Level CV added per sample on top.
        let base_freq = Hertz::new(self.base_frequency.as_f32() / self.octave.divisor());
        let base_level = self.mod_offsets.effective("level", self.level.as_f32());
        let pitch_cv = inputs.reader(PortName::PITCH_CV, 0.0);
        let level_cv = inputs.reader(PortName::LEVEL_CV, 0.0);
        for i in 0..context.samples.as_usize() {
            let freq = if pitch_cv.is_connected() {
                base_freq.apply_cv(BipolarValue::new(pitch_cv.get(i)))
            } else {
                base_freq
            };
            let level = (base_level + level_cv.get(i)).clamp(0.0, 1.0);
            self.output_buffer[i] = self.generate_sample(freq) * level;
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::SubOsc(sub_param) = param {
            match sub_param {
                SubOscParam::Waveform(w) => self.waveform = w,
                SubOscParam::Octave(o) => self.octave = o,
                SubOscParam::Level(l) => self.level = Gain::new(l.as_f32().clamp(0.0, 1.0)),
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::SubOsc(sub_param) = param {
            Some(match sub_param {
                SubOscParam::Waveform(_) => self.waveform.index() as f32,
                SubOscParam::Octave(_) => self.octave.index() as f32,
                SubOscParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::SubOsc(SubOscParam::Waveform(self.waveform)),
            Param::SubOsc(SubOscParam::Octave(self.octave)),
            Param::SubOsc(SubOscParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::SubOscillator
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.phase = Phase::ZERO;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.base_frequency = note.to_frequency();
        // Reset phase on note for consistent attack
        self.phase = Phase::ZERO;
    }

    fn set_voice_pitch(&mut self, freq: Hertz) {
        // Track the voice's modulated note pitch (glide / vibrato / bend); the
        // octave divisor still applies in `generate_sample`. Phase accumulates
        // continuously, so only the step size changes — no click.
        self.base_frequency = Hertz::new(Hertz::OSC_RANGE.clamp(freq.as_f32()));
    }

    fn note_off(&mut self) {
        // Sub-oscillator doesn't need to do anything on note off
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice_pitch_harness::{amdf_fundamental, render_mono};
    use synth_core::{MidiNote, SampleRate, Velocity};

    /// `set_voice_pitch` continuously retunes the sub-osc: 2× voice pitch doubles
    /// the rendered fundamental; a static note stays put. Default octave is −1,
    /// so the output sits an octave below the note (`base / 2`).
    #[test]
    fn sub_osc_tracks_voice_pitch() {
        let sr = SampleRate::DVD_QUALITY;
        let srf = sr.as_f32();
        let f = MidiNote::A4.to_frequency().as_f32();
        let cents = |est: f32, target: f32| 1200.0 * (est / target).log2();

        let mut s = SubOscillator::new();
        s.note_on(MidiNote::A4, Velocity::MAX);
        let stat = render_mono(&mut s, sr, 4, 1024, |_| {});
        let est_stat = amdf_fundamental(&stat[2048..], srf, f / 2.0);
        assert!(cents(est_stat, f / 2.0).abs() < 50.0, "static {est_stat}");

        let mut s2 = SubOscillator::new();
        s2.note_on(MidiNote::A4, Velocity::MAX);
        let up = render_mono(&mut s2, sr, 4, 1024, |m| {
            m.set_voice_pitch(Hertz::new(f * 2.0));
        });
        let est_up = amdf_fundamental(&up[2048..], srf, f); // base 880 / 2 = 440
        assert!(cents(est_up, f).abs() < 50.0, "2x {est_up}");
    }

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

    /// `level` is now a working mod destination via the generic store: a
    /// normalized offset raises the output level, and clearing reverts. (The
    /// graph normally calls `populate`; the test does it directly.)
    #[test]
    fn level_mod_offset_scales_output() {
        let mut sub = SubOscillator::new();
        let desc = sub.descriptor();
        sub.mod_offsets_mut().unwrap().populate(&desc);
        sub.note_on(MidiNote::A4, Velocity::MAX);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            ..ProcessContext::default()
        };
        fn peak(sub: &mut SubOscillator, ctx: &ProcessContext) -> f32 {
            let mut outs = std::collections::HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(256));
            sub.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max)
        }

        // Base level 0.5 → square output peaks ~0.5.
        let base = peak(&mut sub, &ctx);
        assert!(base > 0.1 && base < 0.99, "base ~0.5, got {base}");

        // +0.5 normalized on level (0..1) → effective level 1.0 → louder.
        sub.set_mod_offset("level", 0.5);
        let modded = peak(&mut sub, &ctx);
        assert!(
            modded > base + 0.1,
            "mod should raise level: {modded} vs {base}"
        );

        // Clearing reverts to base.
        sub.clear_mod_offsets();
        assert!((peak(&mut sub, &ctx) - base).abs() < 0.05);
    }

    /// Every waveform in `SubOscWaveform::ALL` (now 6) must render audible,
    /// non-silent output — guards the new Triangle/Sawtooth/DsfSaw shapes.
    #[test]
    fn all_waveforms_produce_signal() {
        for wf in SubOscWaveform::ALL {
            let mut sub = SubOscillator::new();
            sub.set_param(Param::SubOsc(SubOscParam::Waveform(wf)));
            sub.note_on(MidiNote::A4, Velocity::MAX);

            let ctx = ProcessContext {
                samples: synth_core::SampleCount::new(256),
                ..ProcessContext::default()
            };
            let mut outs = std::collections::HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(256));
            sub.process(InputPorts::empty(), &mut outs, &ctx);

            let b = &outs[&PortName::OUT];
            let peak = (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max);
            assert!(
                peak > 0.01,
                "{} should produce signal, peak {peak}",
                wf.name()
            );
        }
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
