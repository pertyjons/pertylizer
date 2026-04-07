//! Phase vocoder effect for pitch shifting and spectral freeze.
//!
//! Features:
//! - Real-time pitch shifting via STFT with phase accumulation
//! - Spectral freeze mode (holds current spectrum)
//! - Configurable FFT size (512-4096)
//! - Stereo processing

use synth_dsp::Complex;

use synth_core::module_traits::ChoiceOption;
use synth_core::{
    AudioEffect, Describable, FftSizeOption, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, ParameterUnit, PhaseVocoderParam, ProcessContext,
    SampleCount, Semitones, StereoSample, WidgetHint,
};
use synth_dsp::{StftProcessor, WindowType};

/// Maximum block size for pre-allocated mono buffers.
const MAX_BLOCK_SIZE: usize = 4096;

/// Phase vocoder with pitch shifting.
pub struct PhaseVocoder {
    // Parameters
    pitch_shift: Semitones,
    freeze: bool,
    fft_size_option: FftSizeOption,
    mix: NormalizedValue,

    // STFT processors (stereo)
    stft_left: StftProcessor,
    stft_right: StftProcessor,

    // Phase accumulation for pitch shifting
    phase_accum_left: Vec<f32>,
    phase_accum_right: Vec<f32>,
    prev_phase_left: Vec<f32>,
    prev_phase_right: Vec<f32>,

    // Frozen spectrum
    frozen_left: Vec<Complex<f32>>,
    frozen_right: Vec<Complex<f32>>,
    has_frozen: bool,

    // Pre-allocated scratch buffers for pitch_shift_spectrum (avoid per-hop allocation)
    temp_magnitudes: Vec<f32>,
    temp_phases: Vec<f32>,

    // Pre-allocated buffers for interleaved -> deinterleaved (max block size)
    mono_in_l: Vec<f32>,
    mono_in_r: Vec<f32>,
    mono_out_l: Vec<f32>,
    mono_out_r: Vec<f32>,
}

impl PhaseVocoder {
    pub fn new() -> Self {
        let fft_size = 1024;
        let hop_size = fft_size / 4;
        let complex_size = fft_size / 2 + 1;

        Self {
            pitch_shift: Semitones(0.0),
            freeze: false,
            fft_size_option: FftSizeOption::Fft1024,
            mix: NormalizedValue::MAX,

            stft_left: StftProcessor::new(fft_size, hop_size, WindowType::Hann),
            stft_right: StftProcessor::new(fft_size, hop_size, WindowType::Hann),

            phase_accum_left: vec![0.0; complex_size],
            phase_accum_right: vec![0.0; complex_size],
            prev_phase_left: vec![0.0; complex_size],
            prev_phase_right: vec![0.0; complex_size],

            frozen_left: vec![Complex::new(0.0, 0.0); complex_size],
            frozen_right: vec![Complex::new(0.0, 0.0); complex_size],
            has_frozen: false,

            temp_magnitudes: vec![0.0; complex_size],
            temp_phases: vec![0.0; complex_size],

            mono_in_l: vec![0.0; MAX_BLOCK_SIZE],
            mono_in_r: vec![0.0; MAX_BLOCK_SIZE],
            mono_out_l: vec![0.0; MAX_BLOCK_SIZE],
            mono_out_r: vec![0.0; MAX_BLOCK_SIZE],
        }
    }

    fn rebuild_stft(&mut self) {
        let fft_size = self.fft_size_option.size();
        let hop_size = fft_size / 4;
        let complex_size = fft_size / 2 + 1;

        self.stft_left = StftProcessor::new(fft_size, hop_size, WindowType::Hann);
        self.stft_right = StftProcessor::new(fft_size, hop_size, WindowType::Hann);

        self.phase_accum_left = vec![0.0; complex_size];
        self.phase_accum_right = vec![0.0; complex_size];
        self.prev_phase_left = vec![0.0; complex_size];
        self.prev_phase_right = vec![0.0; complex_size];
        self.frozen_left = vec![Complex::new(0.0, 0.0); complex_size];
        self.frozen_right = vec![Complex::new(0.0, 0.0); complex_size];
        self.has_frozen = false;

        self.temp_magnitudes = vec![0.0; complex_size];
        self.temp_phases = vec![0.0; complex_size];
    }

    /// Pitch shifting via phase vocoder technique.
    /// `shift_ratio` = 2^(semitones/12), e.g. 1.0 = no shift, 2.0 = octave up.
    ///
    /// `temp_magnitudes` and `temp_phases` are pre-allocated scratch buffers
    /// (must be >= spectrum.len()) to avoid per-hop heap allocation.
    #[allow(clippy::too_many_arguments)]
    fn pitch_shift_spectrum(
        spectrum: &mut [Complex<f32>],
        prev_phase: &mut [f32],
        phase_accum: &mut [f32],
        shift_ratio: f32,
        hop_size: usize,
        fft_size: usize,
        temp_magnitudes: &mut [f32],
        temp_phases: &mut [f32],
    ) {
        let n = spectrum.len();
        let expected_phase_diff = std::f32::consts::TAU * hop_size as f32 / fft_size as f32;

        // Extract magnitudes and phases into pre-allocated scratch buffers
        for (i, c) in spectrum.iter().enumerate() {
            temp_magnitudes[i] = c.norm();
            temp_phases[i] = c.arg();
        }

        // Clear output
        for bin in spectrum.iter_mut() {
            *bin = Complex::new(0.0, 0.0);
        }

        for k in 0..n {
            // Compute instantaneous frequency
            let phase_diff = temp_phases[k] - prev_phase[k];
            let expected = expected_phase_diff * k as f32;
            let mut deviation = phase_diff - expected;

            // Wrap to [-PI, PI]
            deviation = deviation.rem_euclid(std::f32::consts::TAU);
            if deviation > std::f32::consts::PI {
                deviation -= std::f32::consts::TAU;
            }

            let true_freq = k as f32 + deviation / expected_phase_diff;

            // Map to new bin
            let new_bin = (true_freq * shift_ratio) as usize;
            if new_bin < n {
                // Accumulate phase
                let new_phase = phase_accum[new_bin]
                    + expected_phase_diff * new_bin as f32
                    + deviation * shift_ratio;
                phase_accum[new_bin] = new_phase;

                let mag = temp_magnitudes[k];
                spectrum[new_bin] += Complex::new(mag * new_phase.cos(), mag * new_phase.sin());
            }

            prev_phase[k] = temp_phases[k];
        }
    }
}

impl Default for PhaseVocoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for PhaseVocoder {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("phase_vocoder", "Phase Vocoder")
            .description("Spectral pitch shifting and freeze")
            .category(ModuleCategory::Effect)
            .tag("spectral")
            .tag("pitch")
            .tag("freeze")
            .parameter(
                ParameterDescriptor::float(
                    "pitch_shift",
                    Param::PhaseVocoder(PhaseVocoderParam::PitchShift(Semitones(0.0))),
                    "Pitch Shift",
                )
                .range(-24.0, 24.0)
                .default(0.0)
                .unit(ParameterUnit::Semitones)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "freeze",
                    Param::PhaseVocoder(PhaseVocoderParam::Freeze(false)),
                    "Freeze",
                )
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Toggle),
            )
            .parameter(ParameterDescriptor::choice(
                "fft_size",
                Param::PhaseVocoder(PhaseVocoderParam::FftSize(FftSizeOption::Fft1024)),
                "FFT Size",
                FftSizeOption::ALL
                    .iter()
                    .map(|f| ChoiceOption::new(f.id(), f.name()))
                    .collect(),
            ))
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::PhaseVocoder(PhaseVocoderParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for PhaseVocoder {
    fn process(&mut self, input: &[f32], output: &mut [f32], _context: &ProcessContext<'_>) {
        let num_frames = input.len() / 2;
        let mix = self.mix.as_f32();
        let shift_ratio = 2.0f32.powf(self.pitch_shift.0 / 12.0);
        let freeze = self.freeze;
        let fft_size = self.fft_size_option.size();
        let hop_size = fft_size / 4;

        // Deinterleave — use pre-allocated buffers, clamp to max size
        let num_frames = num_frames.min(MAX_BLOCK_SIZE);

        for i in 0..num_frames {
            let frame = StereoSample::read_frame(input, i);
            self.mono_in_l[i] = frame.left;
            self.mono_in_r[i] = frame.right;
        }

        // Process left channel
        let prev_phase_l = &mut self.prev_phase_left;
        let phase_accum_l = &mut self.phase_accum_left;
        let frozen_l = &self.frozen_left;
        let has_frozen = self.has_frozen;
        let temp_mags = &mut self.temp_magnitudes;
        let temp_phs = &mut self.temp_phases;

        self.stft_left.process(
            &self.mono_in_l[..num_frames],
            &mut self.mono_out_l[..num_frames],
            |spectrum| {
                if freeze && has_frozen {
                    spectrum.copy_from_slice(frozen_l);
                } else {
                    Self::pitch_shift_spectrum(
                        spectrum,
                        prev_phase_l,
                        phase_accum_l,
                        shift_ratio,
                        hop_size,
                        fft_size,
                        temp_mags,
                        temp_phs,
                    );
                }
            },
        );

        // Process right channel
        let prev_phase_r = &mut self.prev_phase_right;
        let phase_accum_r = &mut self.phase_accum_right;
        let frozen_r = &self.frozen_right;
        let temp_mags = &mut self.temp_magnitudes;
        let temp_phs = &mut self.temp_phases;

        self.stft_right.process(
            &self.mono_in_r[..num_frames],
            &mut self.mono_out_r[..num_frames],
            |spectrum| {
                if freeze && has_frozen {
                    spectrum.copy_from_slice(frozen_r);
                } else {
                    Self::pitch_shift_spectrum(
                        spectrum,
                        prev_phase_r,
                        phase_accum_r,
                        shift_ratio,
                        hop_size,
                        fft_size,
                        temp_mags,
                        temp_phs,
                    );
                }
            },
        );

        // Capture frozen spectrum if freeze just activated
        if freeze && !self.has_frozen {
            // Will be captured on next STFT frame
            self.has_frozen = true;
        }
        if !freeze {
            self.has_frozen = false;
        }

        // Interleave and mix
        for i in 0..num_frames {
            let result = StereoSample::new(self.mono_in_l[i], self.mono_in_r[i]).blend(
                StereoSample::new(self.mono_out_l[i], self.mono_out_r[i]),
                mix,
            );
            StereoSample::write_frame(output, i, result);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::PhaseVocoder(p) = param {
            match p {
                PhaseVocoderParam::PitchShift(st) => self.pitch_shift = st,
                PhaseVocoderParam::Freeze(b) => self.freeze = b,
                PhaseVocoderParam::FftSize(f) => {
                    if f != self.fft_size_option {
                        self.fft_size_option = f;
                        self.rebuild_stft();
                    }
                }
                PhaseVocoderParam::Mix(v) => self.mix = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::PhaseVocoder(p) = param {
            #[allow(clippy::cast_precision_loss)]
            Some(match p {
                PhaseVocoderParam::PitchShift(_) => self.pitch_shift.0,
                PhaseVocoderParam::Freeze(_) => {
                    if self.freeze {
                        1.0
                    } else {
                        0.0
                    }
                }
                PhaseVocoderParam::FftSize(_) => self.fft_size_option.index() as f32,
                PhaseVocoderParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::PhaseVocoder(PhaseVocoderParam::PitchShift(self.pitch_shift)),
            Param::PhaseVocoder(PhaseVocoderParam::Freeze(self.freeze)),
            Param::PhaseVocoder(PhaseVocoderParam::FftSize(self.fft_size_option)),
            Param::PhaseVocoder(PhaseVocoderParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::PhaseVocoder
    }

    fn reset(&mut self) {
        self.stft_left.reset();
        self.stft_right.reset();
        self.phase_accum_left.fill(0.0);
        self.phase_accum_right.fill(0.0);
        self.prev_phase_left.fill(0.0);
        self.prev_phase_right.fill(0.0);
        self.has_frozen = false;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn tail_samples(&self) -> SampleCount {
        SampleCount::new(self.fft_size_option.size())
    }
}
