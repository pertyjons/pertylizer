//! Modal resonator bank effect.
//!
//! Features:
//! - Bank of up to 16 biquad bandpass resonators
//! - Harmonic frequencies derived from base MIDI note
//! - Inharmonicity spread for metallic/bell tones
//! - Brightness controls high mode rolloff and Q

use synth_core::FilterState;
use synth_core::{
    AudioEffect, Describable, Hertz, MidiNote, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, ProcessContext, SampleRate, StereoSample,
    WidgetHint,
};
use synth_core::{ModalResonatorParam, NoiseState};
use synth_dsp::BiquadCoeffs;

const MAX_MODES: usize = 16;

/// Per-mode biquad state.
struct ModeState {
    coeffs: BiquadCoeffs,
    z1: FilterState,
    z2: FilterState,
}

impl Default for ModeState {
    fn default() -> Self {
        Self {
            coeffs: BiquadCoeffs::default(),
            z1: FilterState::ZERO,
            z2: FilterState::ZERO,
        }
    }
}

/// Modal resonator bank effect.
pub struct ModalResonator {
    // Parameters
    base_note: MidiNote,
    spread: NormalizedValue,
    modes: u8,
    decay: NormalizedValue,
    brightness: NormalizedValue,
    mix: NormalizedValue,

    // Per-mode filter state
    mode_states: Vec<ModeState>,
    // Randomized inharmonicity offsets (fixed per reset)
    inharmonicity: [f32; MAX_MODES],

    noise: NoiseState,
    sample_rate: SampleRate,
    params_dirty: bool,
}

impl ModalResonator {
    pub fn new() -> Self {
        let mut effect = Self {
            base_note: MidiNote::new(60),
            spread: NormalizedValue::new(0.3),
            modes: 8,
            decay: NormalizedValue::new(0.6),
            brightness: NormalizedValue::CENTER,
            mix: NormalizedValue::MAX,

            mode_states: (0..MAX_MODES).map(|_| ModeState::default()).collect(),
            inharmonicity: [0.0; MAX_MODES],

            noise: NoiseState::DEFAULT,
            sample_rate: SampleRate::DVD_QUALITY,
            params_dirty: true,
        };
        effect.randomize_inharmonicity();
        effect
    }

    fn randomize_inharmonicity(&mut self) {
        for i in 0..MAX_MODES {
            self.inharmonicity[i] = self.noise.next();
        }
    }

    fn update_coeffs(&mut self) {
        if !self.params_dirty {
            return;
        }
        self.params_dirty = false;

        let f0 = self.base_note.to_frequency();
        let q = 0.5 + self.decay.as_f32() * 20.0;
        let nyquist = self.sample_rate.as_f32() * 0.5;
        let mode_count = (self.modes as usize).clamp(1, MAX_MODES);

        for i in 0..mode_count {
            // Harmonic frequency with inharmonicity spread
            let harmonic = (i + 1) as f32;
            let detune = 1.0 + self.spread.as_f32() * self.inharmonicity[i] * 0.02;
            let freq = f0.as_f32() * harmonic * detune;

            // Skip modes above Nyquist
            if freq >= nyquist * 0.95 {
                self.mode_states[i].coeffs = BiquadCoeffs::default();
                continue;
            }

            // Higher modes get lower Q based on brightness
            let mode_q =
                q * (1.0 - (i as f32 / MAX_MODES as f32) * (1.0 - self.brightness.as_f32()));

            self.mode_states[i].coeffs =
                BiquadCoeffs::bandpass(Hertz::new(freq), mode_q.max(0.5), self.sample_rate);
        }

        // Deactivate unused modes
        for i in mode_count..MAX_MODES {
            self.mode_states[i].coeffs = BiquadCoeffs::default();
        }
    }
}

impl Default for ModalResonator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for ModalResonator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("modal_resonator", "Modal Resonator")
            .description("Bank of tuned bandpass resonators")
            .category(ModuleCategory::Effect)
            .tag("resonator")
            .tag("effect")
            .tag("modal")
            .parameter(
                ParameterDescriptor::float(
                    Param::ModalResonator(ModalResonatorParam::BaseNote(MidiNote::new(60))),
                    "Base Note",
                )
                .description("Base MIDI note for fundamental")
                .range(24.0, 96.0)
                .default(60.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ModalResonator(ModalResonatorParam::Spread(NormalizedValue::new(0.3))),
                    "Spread",
                )
                .description("Inharmonicity/detune spread")
                .range(0.0, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ModalResonator(ModalResonatorParam::Modes(8)),
                    "Modes",
                )
                .description("Number of resonant modes")
                .range(4.0, 16.0)
                .default(8.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ModalResonator(ModalResonatorParam::Decay(NormalizedValue::new(0.6))),
                    "Decay",
                )
                .description("Resonance decay (Q)")
                .range(0.0, 1.0)
                .default(0.6)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ModalResonator(ModalResonatorParam::Brightness(NormalizedValue::CENTER)),
                    "Brightness",
                )
                .description("High mode level and Q")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::ModalResonator(ModalResonatorParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for ModalResonator {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.update_coeffs();

        let mode_count = (self.modes as usize).clamp(1, MAX_MODES);
        let mix = self.mix.as_f32();
        let brightness = self.brightness.as_f32();

        let channels = 2;
        for frame in 0..context.samples.as_usize() {
            let idx_l = frame * channels;
            let idx_r = frame * channels + 1;

            let dry = if idx_r < input.len() {
                StereoSample::new(input[idx_l], input[idx_r])
            } else if idx_l < input.len() {
                StereoSample::from_mono(input[idx_l])
            } else {
                StereoSample::ZERO
            };

            let mono_in = dry.to_mono();
            let mut resonated = 0.0f32;

            for i in 0..mode_count {
                let state = &mut self.mode_states[i];
                let out = state.coeffs.process(mono_in, &mut state.z1, &mut state.z2);

                // Higher modes reduced by brightness
                let mode_gain = 1.0 - (i as f32 / mode_count as f32) * (1.0 - brightness);
                resonated += out * mode_gain;
            }

            // Normalize by mode count to prevent clipping
            resonated /= (mode_count as f32).sqrt();

            let result = StereoSample::new(
                dry.left * (1.0 - mix) + resonated * mix,
                dry.right * (1.0 - mix) + resonated * mix,
            );

            if idx_l < output.len() {
                output[idx_l] = result.left;
            }
            if idx_r < output.len() {
                output[idx_r] = result.right;
            }
        }
    }

    fn reset(&mut self) {
        for state in &mut self.mode_states {
            state.z1 = FilterState::ZERO;
            state.z2 = FilterState::ZERO;
        }
        self.randomize_inharmonicity();
        self.params_dirty = true;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }
    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::ModalResonator(p) = param {
            match p {
                ModalResonatorParam::BaseNote(v) => {
                    self.base_note = MidiNote::new(v.as_u8().clamp(24, 96));
                    self.params_dirty = true;
                }
                ModalResonatorParam::Spread(v) => {
                    self.spread = v;
                    self.params_dirty = true;
                }
                ModalResonatorParam::Modes(v) => {
                    self.modes = v.clamp(4, 16);
                    self.params_dirty = true;
                }
                ModalResonatorParam::Decay(v) => {
                    self.decay = v;
                    self.params_dirty = true;
                }
                ModalResonatorParam::Brightness(v) => {
                    self.brightness = v;
                    self.params_dirty = true;
                }
                ModalResonatorParam::Mix(v) => self.mix = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::ModalResonator(p) = param {
            Some(match p {
                ModalResonatorParam::BaseNote(_) => self.base_note.as_u8() as f32,
                ModalResonatorParam::Spread(_) => self.spread.as_f32(),
                ModalResonatorParam::Modes(_) => self.modes as f32,
                ModalResonatorParam::Decay(_) => self.decay.as_f32(),
                ModalResonatorParam::Brightness(_) => self.brightness.as_f32(),
                ModalResonatorParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::ModalResonator(ModalResonatorParam::BaseNote(self.base_note)),
            Param::ModalResonator(ModalResonatorParam::Spread(self.spread)),
            Param::ModalResonator(ModalResonatorParam::Modes(self.modes)),
            Param::ModalResonator(ModalResonatorParam::Decay(self.decay)),
            Param::ModalResonator(ModalResonatorParam::Brightness(self.brightness)),
            Param::ModalResonator(ModalResonatorParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::ModalResonator
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        self.params_dirty = true;
    }
}
