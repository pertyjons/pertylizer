//! Formant Filter module.
//!
//! Vowel-shaping filter using parallel bandpass filters tuned to formant frequencies.
//! Smoothly morphs between A, E, I, O, U vowel sounds.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Filters/110-formant-filter.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{FormantFilterParam, ModuleType, Param};
use synth_core::{Gain, Hertz, MidiNote, NormalizedValue, PortName, SampleRate, Velocity};

use crate::formant_tables::{FORMANT_BW, FORMANT_FREQ, FORMANT_GAIN, NUM_VOWELS};

/// Number of formant bands this module uses (F1/F2/F3 — the speech vowel
/// formants; the shared tables' singer's-formant band is voice-synth only).
const NUM_BANDS: usize = 3;

/// 2nd-order bandpass filter state.
#[derive(Clone, Copy, Default)]
struct BandpassState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BandpassState {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Bandpass filter coefficients.
#[derive(Clone, Copy, Default)]
struct BandpassCoeffs {
    b0: f32,
    a1: f32,
    a2: f32,
}

impl BandpassCoeffs {
    /// Calculate bandpass coefficients for given center frequency and bandwidth.
    fn new(center: f32, bandwidth: f32, sample_rate: f32) -> Self {
        let omega = 2.0 * std::f32::consts::PI * center / sample_rate;
        let sin_w = omega.sin();
        let cos_w = omega.cos();
        let alpha = sin_w * (bandwidth * std::f32::consts::PI / sample_rate).tanh();

        let a0_inv = 1.0 / (1.0 + alpha);
        Self {
            b0: alpha * a0_inv,
            a1: -2.0 * cos_w * a0_inv,
            a2: (1.0 - alpha) * a0_inv,
        }
    }

    /// Process one sample through this bandpass.
    #[inline]
    fn process(&self, input: f32, state: &mut BandpassState) -> f32 {
        let output = self.b0 * (input - state.x2) - self.a1 * state.y1 - self.a2 * state.y2;
        state.x2 = state.x1;
        state.x1 = input;
        state.y2 = state.y1;
        state.y1 = output;
        output
    }
}

/// Formant Filter — vowel-shaping bandpass filter bank.
#[derive(Clone)]
pub struct FormantFilter {
    vowel: NormalizedValue,
    cutoff: Hertz,
    resonance: NormalizedValue,
    mix: NormalizedValue,
    sample_rate: SampleRate,
    /// Filter states per band
    states: [BandpassState; NUM_BANDS],
    /// Current interpolated coefficients per band
    coeffs: [BandpassCoeffs; NUM_BANDS],
    /// Current interpolated gains per band
    gains: [Gain; NUM_BANDS],
    output_buffer: AudioBuffer,
}

impl FormantFilter {
    pub fn new() -> Self {
        let mut f = Self {
            vowel: NormalizedValue::MIN,
            cutoff: Hertz::new(1000.0),
            resonance: NormalizedValue::new(0.5),
            mix: NormalizedValue::MAX,
            sample_rate: SampleRate::DVD_QUALITY,
            states: [BandpassState::default(); NUM_BANDS],
            coeffs: [BandpassCoeffs::default(); NUM_BANDS],
            gains: [Gain::new(1.0), Gain::new(0.5), Gain::new(0.25)],
            output_buffer: AudioBuffer::new(1024),
        };
        f.update_coeffs();
        f
    }

    /// Interpolate formant parameters based on vowel position and update coefficients.
    fn update_coeffs(&mut self) {
        let pos = self.vowel.as_f32() * (NUM_VOWELS - 1) as f32;
        let idx = (pos as usize).min(NUM_VOWELS - 2);
        let frac = pos - idx as f32;

        let cutoff_scale = self.cutoff.as_f32() / 1000.0;
        // Resonance scales bandwidth inversely (more resonance = narrower)
        let bw_scale = 1.0 - self.resonance.as_f32() * 0.8;

        for band in 0..NUM_BANDS {
            let freq = FORMANT_FREQ[idx][band] * (1.0 - frac) + FORMANT_FREQ[idx + 1][band] * frac;
            let bw = FORMANT_BW[idx][band] * (1.0 - frac) + FORMANT_BW[idx + 1][band] * frac;
            let gain = FORMANT_GAIN[idx][band] * (1.0 - frac) + FORMANT_GAIN[idx + 1][band] * frac;

            let scaled_freq = (freq * cutoff_scale).clamp(20.0, self.sample_rate.as_f32() * 0.45);
            let scaled_bw = (bw * bw_scale).max(10.0);

            self.coeffs[band] =
                BandpassCoeffs::new(scaled_freq, scaled_bw, self.sample_rate.as_f32());
            self.gains[band] = Gain::new(gain);
        }
    }
}

impl Default for FormantFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for FormantFilter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("formant_filter", "Formant Filter")
            .description("Vowel-shaping filter (A-E-I-O-U morphing)")
            .category(ModuleCategory::Filter)
            .tag("filter")
            .tag("formant")
            .tag("vocal")
            .parameter(
                ParameterDescriptor::float(
                    "vowel",
                    Param::FormantFilter(FormantFilterParam::Vowel(NormalizedValue::MIN)),
                    "Vowel",
                )
                .description("Vowel morph (A → E → I → O → U)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "cutoff",
                    Param::FormantFilter(FormantFilterParam::Cutoff(Hertz::new(1000.0))),
                    "Cutoff",
                )
                .description("Scales formant frequencies")
                .range(200.0, 5000.0)
                .default(1000.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::FrequencySlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "resonance",
                    Param::FormantFilter(FormantFilterParam::Resonance(NormalizedValue::new(0.5))),
                    "Resonance",
                )
                .description("Formant sharpness")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::FormantFilter(FormantFilterParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Audio to filter. Connect: Oscillator Out, Noise Out"),
            )
            .port(
                PortDescriptor::control_input("vowel_cv", "Vowel CV")
                    .description("Modulate vowel position. Connect: LFO, Envelope"),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Formant-filtered output"))
    }
}

impl PolyModule for FormantFilter {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let audio_in = inputs.reader(PortName::IN, 0.0);
        let vowel_cv = inputs.reader(PortName::intern("vowel_cv"), 0.0);

        let mix = self.mix.as_f32();

        for i in 0..context.samples.as_usize() {
            // Apply vowel CV modulation
            if vowel_cv.is_connected() {
                let mod_val = vowel_cv[i];
                let new_vowel =
                    NormalizedValue::new((self.vowel.as_f32() + mod_val * 0.5).clamp(0.0, 1.0));
                // Only recalculate coefficients if vowel changed significantly
                if (new_vowel.as_f32() - self.vowel.as_f32()).abs() > 0.001 {
                    self.vowel = new_vowel;
                    self.update_coeffs();
                }
            }

            let input = audio_in[i];

            // Sum parallel bandpass filters
            let mut wet = 0.0;
            for band in 0..NUM_BANDS {
                wet += self.coeffs[band].process(input, &mut self.states[band])
                    * self.gains[band].as_f32();
            }

            // Normalize (3 bands summed)
            wet *= 0.7;

            self.output_buffer[i] = input * (1.0 - mix) + wet * mix;
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::FormantFilter(p) = param {
            match p {
                FormantFilterParam::Vowel(v) => {
                    self.vowel = v;
                    self.update_coeffs();
                }
                FormantFilterParam::Cutoff(c) => {
                    self.cutoff = Hertz::new(c.as_f32().clamp(200.0, 5000.0));
                    self.update_coeffs();
                }
                FormantFilterParam::Resonance(r) => {
                    self.resonance = r;
                    self.update_coeffs();
                }
                FormantFilterParam::Mix(m) => self.mix = m,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::FormantFilter(p) = param {
            Some(match p {
                FormantFilterParam::Vowel(_) => self.vowel.as_f32(),
                FormantFilterParam::Cutoff(_) => self.cutoff.as_f32(),
                FormantFilterParam::Resonance(_) => self.resonance.as_f32(),
                FormantFilterParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::FormantFilter(FormantFilterParam::Vowel(self.vowel)),
            Param::FormantFilter(FormantFilterParam::Cutoff(self.cutoff)),
            Param::FormantFilter(FormantFilterParam::Resonance(self.resonance)),
            Param::FormantFilter(FormantFilterParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::FormantFilter
    }

    fn reset(&mut self) {
        for state in &mut self.states {
            state.reset();
        }
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}
    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_formant_filter_vowel_range() {
        let mut ff = FormantFilter::new();
        // Test all vowel positions
        for pos in [0.0, 0.25, 0.5, 0.75, 1.0] {
            ff.vowel = NormalizedValue::new(pos);
            ff.update_coeffs();
            // Coefficients should be finite
            for band in 0..NUM_BANDS {
                assert!(ff.coeffs[band].b0.is_finite());
                assert!(ff.coeffs[band].a1.is_finite());
                assert!(ff.coeffs[band].a2.is_finite());
            }
        }
    }
}
