//! Additive synthesis oscillator.
//!
//! Features:
//! - 32 harmonics with individual amplitude control via spectral profile
//! - Tilt (rolloff per octave), Odd/Even balance, Brightness
//! - Spectral stretch for inharmonic timbres
//! - Per-partial phase randomization on note-on
//! - FM input for frequency modulation

use std::collections::HashMap;
use synth_core::VoicePitch;
use synth_core::{
    AdditiveParam, AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor,
    ModuleType, Param, ParamModOffsets, ParameterDescriptor, ParameterUnit, PolyModule,
    PortDescriptor, PortValueDomain, ProcessContext, WidgetHint,
};
use synth_core::{
    BipolarValue, Hertz, MidiNote, NormalizedValue, PortName, SampleRate, Seconds, Velocity,
};

use crate::osc_glide::OscGlide;

/// Number of harmonics.
const NUM_HARMONICS: usize = 32;

/// Additive synthesis oscillator.
#[derive(Clone)]
pub struct AdditiveOsc {
    // Parameters
    tilt: NormalizedValue,
    odd_even: NormalizedValue,
    brightness: NormalizedValue,
    stretch: NormalizedValue,
    randomize: NormalizedValue,
    level: NormalizedValue,

    // State
    phases: [f32; NUM_HARMONICS],
    amplitudes: [f32; NUM_HARMONICS],
    note_freq: Hertz,
    sample_rate: SampleRate,
    inv_sample_rate: f32,
    amplitudes_dirty: bool,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,
    /// Per-oscillator glide (portamento); `0` glide time = follow the voice glide.
    glide: OscGlide,

    // Buffer
    output_buffer: AudioBuffer,
}

impl AdditiveOsc {
    pub fn new() -> Self {
        Self {
            tilt: NormalizedValue::new(0.5),
            odd_even: NormalizedValue::CENTER,
            brightness: NormalizedValue::new(0.7),
            stretch: NormalizedValue::MIN,
            randomize: NormalizedValue::new(0.1),
            level: NormalizedValue::MAX,

            phases: [0.0; NUM_HARMONICS],
            amplitudes: [0.0; NUM_HARMONICS],
            note_freq: Hertz::A4,
            sample_rate: SampleRate::DVD_QUALITY,
            inv_sample_rate: 1.0 / SampleRate::DVD_QUALITY.as_f32(),
            amplitudes_dirty: true,
            mod_offsets: ParamModOffsets::new(),
            glide: OscGlide::new(),

            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Recalculate harmonic amplitudes from spectral parameters.
    fn update_amplitudes(&mut self) {
        if !self.amplitudes_dirty {
            return;
        }
        self.amplitudes_dirty = false;

        // Spectral shaping params are mod destinations via the generic store.
        // 0..1 range, so a normalized offset maps directly onto the param.
        let tilt = self.mod_offsets.effective("tilt", self.tilt.as_f32()) * 2.0; // 0-2: 0=flat, 1=normal, 2=steep
        let odd_even = self
            .mod_offsets
            .effective("odd_even", self.odd_even.as_f32()); // 0=odd, 0.5=equal, 1=even
        let brightness = self
            .mod_offsets
            .effective("brightness", self.brightness.as_f32());

        for i in 0..NUM_HARMONICS {
            let harmonic_num = (i + 1) as f32;

            // Tilt: rolloff per octave
            let tilt_atten = crate::math::spectral_rolloff(harmonic_num, tilt);

            // Odd/even balance
            let oe_factor = crate::math::odd_even_balance(i, odd_even);

            // Brightness: boost/cut upper harmonics
            let bright_factor = crate::math::brightness_boost(harmonic_num, brightness);

            self.amplitudes[i] = (tilt_atten * oe_factor * bright_factor).clamp(0.0, 1.0);
        }
    }

    /// Get the frequency for a given harmonic with stretch applied.
    #[inline]
    fn harmonic_freq(&self, harmonic_index: usize, base_freq: Hertz, stretch: f32) -> f32 {
        let n = (harmonic_index + 1) as f32;
        // Stretch factor: slightly detune higher harmonics
        // stretch=0: pure harmonic, stretch=1: significant inharmonicity
        let stretch_factor = 1.0 + stretch * 0.01 * (n - 1.0) * (n - 1.0);
        base_freq.as_f32() * n * stretch_factor
    }

    /// Randomize phases for a new note.
    fn randomize_phases(&mut self) {
        let amount = self.randomize.as_f32();
        if amount < 0.001 {
            self.phases.fill(0.0);
            return;
        }
        // Simple deterministic pseudo-random based on harmonic index
        for i in 0..NUM_HARMONICS {
            let hash = ((i as u32).wrapping_mul(2654435761)) as f32 / u32::MAX as f32;
            self.phases[i] = hash * amount;
        }
    }
}

impl Default for AdditiveOsc {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for AdditiveOsc {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("additive_osc", "Additive")
            .description("Additive synthesis with 32 harmonics")
            .category(ModuleCategory::Oscillator)
            .tag("additive")
            .tag("oscillator")
            .tag("spectral")
            .parameter(
                ParameterDescriptor::float(
                    "tilt",
                    Param::AdditiveOsc(AdditiveParam::Tilt(NormalizedValue::new(0.5))),
                    "Tilt",
                )
                .description("Spectral rolloff per octave")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "odd_even",
                    Param::AdditiveOsc(AdditiveParam::OddEven(NormalizedValue::CENTER)),
                    "Odd/Even",
                )
                .description("Balance between odd and even harmonics")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "brightness",
                    Param::AdditiveOsc(AdditiveParam::Brightness(NormalizedValue::new(0.7))),
                    "Brightness",
                )
                .description("High harmonic presence")
                .range(0.0, 1.0)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "stretch",
                    Param::AdditiveOsc(AdditiveParam::Stretch(NormalizedValue::MIN)),
                    "Stretch",
                )
                .description("Spectral inharmonicity")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "randomize",
                    Param::AdditiveOsc(AdditiveParam::Randomize(NormalizedValue::new(0.1))),
                    "Randomize",
                )
                .description("Phase randomization on note-on")
                .range(0.0, 1.0)
                .default(0.1)
                // Only consumed in `randomize_phases` at note-on to seed phases;
                // a continuous mod offset has no audible effect mid-note.
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::AdditiveOsc(AdditiveParam::Level(NormalizedValue::MAX)),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "glide_time",
                    Param::AdditiveOsc(AdditiveParam::GlideTime(Seconds::ZERO)),
                    "Glide",
                )
                .description("Per-oscillator portamento time (0 = follow the voice glide)")
                .range(0.0, 2.0)
                .default(0.0)
                .modulatable(false)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::control_input("freq_cv", "Freq CV")
                    .value_domain(PortValueDomain::Octaves)
                    .description(
                        "Modulates pitch (1V/oct), clamped to ±1 octave. \
                         Connect: LFO, Envelope, Kinetic Modulator",
                    ),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Additive output. Connect to: Amplifier In, Filter In"),
            )
    }
}

impl PolyModule for AdditiveOsc {
    #[allow(clippy::too_many_lines)]
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.inv_sample_rate = 1.0 / context.sample_rate.as_f32();
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        // Per-oscillator glide: with glide_time > 0 run our own portamento toward
        // the note target (bend/vibrato on top), overriding the voice glide.
        if let Some(glided) = self.glide.resolve(context.sample_rate, context.samples) {
            self.note_freq = Hertz::new(Hertz::OSC_RANGE.clamp(glided.as_f32()));
        }

        // Spectral params (tilt/odd_even/brightness) cache into `amplitudes`
        // behind a dirty flag; while any mod offset is live, force a recompute
        // so the modulation is reflected this block.
        if self.mod_offsets.any_active() {
            self.amplitudes_dirty = true;
        }
        self.update_amplitudes();

        let freq_cv = inputs.get(PortName::FREQ_CV);
        let level = self.mod_offsets.effective("level", self.level.as_f32());
        let stretch = self.mod_offsets.effective("stretch", self.stretch.as_f32());
        let nyquist = self.sample_rate.as_f32() * 0.5;
        let inv_sr = self.inv_sample_rate;

        for i in 0..num_samples {
            let mut sample = 0.0_f32;

            // Apply frequency CV (1V/oct)
            let base_freq = if let Some(cv) = freq_cv {
                self.note_freq
                    .apply_cv(BipolarValue::new(crate::math::sanitize_cv(cv[i])))
            } else {
                self.note_freq
            };

            let mut active_harmonics = 0_usize;
            for h in 0..NUM_HARMONICS {
                let amp = self.amplitudes[h];
                if amp < 0.001 {
                    continue;
                }

                let freq = self.harmonic_freq(h, base_freq, stretch);

                // Skip harmonics above Nyquist
                if freq >= nyquist {
                    break;
                }

                active_harmonics += 1;

                // Fast sine approximation (max error ~0.001, inaudible)
                let phase = self.phases[h];
                sample += crate::math::fast_sin_turns(phase) * amp;

                // Advance phase (multiply instead of division)
                self.phases[h] = (phase + freq * inv_sr).rem_euclid(1.0);
            }

            // Normalize by actual number of active harmonics
            let normalization = crate::math::normalization_gain(active_harmonics.max(1));
            self.output_buffer[i] = sample * normalization * level;
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::AdditiveOsc(p) = param {
            match p {
                AdditiveParam::Tilt(v) => {
                    self.tilt = v;
                    self.amplitudes_dirty = true;
                }
                AdditiveParam::OddEven(v) => {
                    self.odd_even = v;
                    self.amplitudes_dirty = true;
                }
                AdditiveParam::Brightness(v) => {
                    self.brightness = v;
                    self.amplitudes_dirty = true;
                }
                AdditiveParam::Stretch(v) => self.stretch = v,
                AdditiveParam::Randomize(v) => self.randomize = v,
                AdditiveParam::Level(v) => self.level = v,
                AdditiveParam::GlideTime(t) => self.glide.set_time(t),
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::AdditiveOsc(p) = param {
            Some(match p {
                AdditiveParam::Tilt(_) => self.tilt.as_f32(),
                AdditiveParam::OddEven(_) => self.odd_even.as_f32(),
                AdditiveParam::Brightness(_) => self.brightness.as_f32(),
                AdditiveParam::Stretch(_) => self.stretch.as_f32(),
                AdditiveParam::Randomize(_) => self.randomize.as_f32(),
                AdditiveParam::Level(_) => self.level.as_f32(),
                AdditiveParam::GlideTime(_) => self.glide.time().as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::AdditiveOsc(AdditiveParam::Tilt(self.tilt)),
            Param::AdditiveOsc(AdditiveParam::OddEven(self.odd_even)),
            Param::AdditiveOsc(AdditiveParam::Brightness(self.brightness)),
            Param::AdditiveOsc(AdditiveParam::Stretch(self.stretch)),
            Param::AdditiveOsc(AdditiveParam::Randomize(self.randomize)),
            Param::AdditiveOsc(AdditiveParam::Level(self.level)),
            Param::AdditiveOsc(AdditiveParam::GlideTime(self.glide.time())),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::AdditiveOsc
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.phases.fill(0.0);
        self.glide.reset();
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.randomize_phases();
    }

    fn set_voice_pitch(&mut self, pitch: VoicePitch) {
        // `process` rebuilds every harmonic from `note_freq` each block (skipping
        // partials above Nyquist), so tracking the modulated note pitch is just
        // updating `note_freq`. Per-harmonic phases accumulate continuously — no
        // click.
        self.glide.store(pitch);
        self.note_freq = Hertz::new(Hertz::OSC_RANGE.clamp(pitch.played.as_f32()));
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
    use crate::voice_pitch_harness::{amdf_fundamental, render_mono};

    /// `set_voice_pitch` retunes all harmonics via `note_freq` (read live by
    /// `process`): 2× voice pitch doubles the rendered fundamental; a static note
    /// holds.
    #[test]
    fn additive_osc_tracks_voice_pitch() {
        let sr = SampleRate::DVD_QUALITY;
        let srf = sr.as_f32();
        let note = MidiNote::new(45); // A2 ≈ 110 Hz
        let f = note.to_frequency().as_f32();
        let cents = |est: f32, target: f32| 1200.0 * (est / target).log2();

        let mut s = AdditiveOsc::new();
        s.note_on(note, Velocity::MAX);
        let stat = render_mono(&mut s, sr, 4, 1024, |_| {});
        let est_stat = amdf_fundamental(&stat[2048..], srf, f);
        assert!(cents(est_stat, f).abs() < 50.0, "static {est_stat}");

        let mut s2 = AdditiveOsc::new();
        s2.note_on(note, Velocity::MAX);
        let up = render_mono(&mut s2, sr, 4, 1024, |m| {
            m.set_voice_pitch(VoicePitch::tracking(Hertz::new(f * 2.0)));
        });
        let est_up = amdf_fundamental(&up[2048..], srf, f * 2.0);
        assert!(cents(est_up, f * 2.0).abs() < 50.0, "2x {est_up}");
    }

    /// Generic mod offsets reach both the per-block `level` and the cached
    /// spectral params (here `tilt`, which forces an amplitude recompute).
    #[test]
    fn mod_offsets_scale_level_and_tilt() {
        use synth_core::SampleCount;
        let mut osc = AdditiveOsc::new();
        let desc = osc.descriptor();
        osc.mod_offsets_mut().unwrap().populate(&desc);
        osc.note_on(MidiNote::A4, Velocity::MAX);

        let ctx = ProcessContext {
            samples: SampleCount::new(256),
            ..ProcessContext::default()
        };
        fn peak(osc: &mut AdditiveOsc, ctx: &ProcessContext) -> f32 {
            let mut outs = std::collections::HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(256));
            osc.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max)
        }

        let base = peak(&mut osc, &ctx);
        assert!(base > 0.01, "base output present, got {base}");

        // Lower level via a negative offset → quieter.
        osc.set_mod_offset("level", -0.8);
        let quieter = peak(&mut osc, &ctx);
        assert!(
            quieter < base,
            "level mod should reduce output: {quieter} vs {base}"
        );
        osc.clear_mod_offsets();

        // Tilt offset reshapes the dominant harmonics (recompute path). Peak is
        // normalized, so compare the full waveform instead.
        fn wave(osc: &mut AdditiveOsc, ctx: &ProcessContext) -> Vec<f32> {
            let mut outs = std::collections::HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(256));
            osc.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i]).collect()
        }
        osc.reset();
        let before = wave(&mut osc, &ctx);
        osc.set_mod_offset("tilt", 0.4);
        osc.reset();
        let after = wave(&mut osc, &ctx);
        let diff: f32 = before.iter().zip(&after).map(|(a, b)| (a - b).abs()).sum();
        assert!(
            diff > 1e-3,
            "tilt mod should alter the waveform, total diff = {diff}"
        );
    }

    #[test]
    fn test_additive_osc_creation() {
        let osc = AdditiveOsc::new();
        assert_eq!(osc.note_freq.as_f32(), 440.0);
    }

    #[test]
    fn test_additive_osc_produces_sound() {
        let mut osc = AdditiveOsc::new();
        osc.note_on(MidiNote::new(69), Velocity::new(0.8));

        let mut outputs = HashMap::new();
        outputs.insert(PortName::OUT, AudioBuffer::new(64));

        let context = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        osc.process(InputPorts::empty(), &mut outputs, &context);

        let out = &outputs[&PortName::OUT];
        let max = (0..64).map(|i| out[i].abs()).fold(0.0_f32, f32::max);
        assert!(max > 0.01, "Additive osc should produce sound, max={max}");
    }

    #[test]
    fn test_additive_osc_params() {
        let mut osc = AdditiveOsc::new();
        osc.set_param(Param::AdditiveOsc(AdditiveParam::Tilt(
            NormalizedValue::new(0.8),
        )));
        assert!((osc.tilt.as_f32() - 0.8).abs() < 0.001);
    }
}
