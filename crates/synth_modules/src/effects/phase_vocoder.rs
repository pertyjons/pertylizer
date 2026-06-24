//! Phase vocoder effect for pitch shifting and spectral freeze.
//!
//! Features:
//! - Real-time pitch shifting via STFT with phase accumulation
//! - Spectral freeze mode (holds current spectrum)
//! - Configurable FFT size (512-4096)
//! - Stereo processing

use synth_dsp::Complex;

use synth_core::MAX_BLOCK_SIZE;
use synth_core::module_traits::ChoiceOption;
use synth_core::{
    AudioEffect, Describable, FftSizeOption, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, ParameterUnit, PhaseVocoderParam, ProcessContext,
    SampleCount, Semitones, StereoSample, WidgetHint,
};
use synth_dsp::{StftProcessor, WindowType};

/// Maximum FFT size supported (the largest `FftSizeOption`).
const MAX_FFT: usize = 4096;
/// Number of complex bins for the max FFT — the worst-case size for every
/// per-bin scratch buffer, so they never reallocate on an FFT-size change.
const MAX_BINS: usize = MAX_FFT / 2 + 1;

/// Phase vocoder with pitch shifting.
pub struct PhaseVocoder {
    // Parameters
    pitch_shift: Semitones,
    freeze: bool,
    fft_size_option: FftSizeOption,
    mix: NormalizedValue,

    // STFT processors, stereo × one per FftSizeOption. Pre-built in `new()` (off
    // the audio thread, where FFT planning allocates); switching FFT size is then
    // an O(1) index change — no allocation or re-planning on the audio thread.
    stft_left_pool: Vec<StftProcessor>,
    stft_right_pool: Vec<StftProcessor>,

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
        // One STFT per FftSizeOption, per channel — built once here, off the audio
        // thread. hop = size/4 (75% overlap), matching the inline convention.
        let make_pool = || {
            FftSizeOption::ALL
                .iter()
                .map(|o| StftProcessor::new(o.size(), o.size() / 4, WindowType::Hann))
                .collect()
        };

        Self {
            pitch_shift: Semitones(0.0),
            freeze: false,
            fft_size_option: FftSizeOption::Fft1024,
            mix: NormalizedValue::MAX,

            stft_left_pool: make_pool(),
            stft_right_pool: make_pool(),

            // Worst-case (MAX_BINS) so an FFT-size switch never reallocates; the
            // active region [0..complex_size) is reset on each switch.
            phase_accum_left: vec![0.0; MAX_BINS],
            phase_accum_right: vec![0.0; MAX_BINS],
            prev_phase_left: vec![0.0; MAX_BINS],
            prev_phase_right: vec![0.0; MAX_BINS],

            frozen_left: vec![Complex::new(0.0, 0.0); MAX_BINS],
            frozen_right: vec![Complex::new(0.0, 0.0); MAX_BINS],
            has_frozen: false,

            temp_magnitudes: vec![0.0; MAX_BINS],
            temp_phases: vec![0.0; MAX_BINS],

            mono_in_l: vec![0.0; MAX_BLOCK_SIZE],
            mono_in_r: vec![0.0; MAX_BLOCK_SIZE],
            mono_out_l: vec![0.0; MAX_BLOCK_SIZE],
            mono_out_r: vec![0.0; MAX_BLOCK_SIZE],
        }
    }

    /// Reset the currently-active STFT processors and clear all per-bin phase /
    /// freeze state. Called on FFT-size change (the destination processor must
    /// start clean), and on `reset()` — all allocation-free (processors pre-built,
    /// buffers pre-sized to `MAX_BINS`; this only zeroes them).
    fn reset_active(&mut self) {
        let idx = self.fft_size_option.index();
        self.stft_left_pool[idx].reset();
        self.stft_right_pool[idx].reset();
        self.phase_accum_left.fill(0.0);
        self.phase_accum_right.fill(0.0);
        self.prev_phase_left.fill(0.0);
        self.prev_phase_right.fill(0.0);
        self.frozen_left.fill(Complex::new(0.0, 0.0));
        self.frozen_right.fill(Complex::new(0.0, 0.0));
        self.has_frozen = false;
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
                .description("Pitch shift in semitones")
                .range(-24.0, 24.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "freeze",
                    Param::PhaseVocoder(PhaseVocoderParam::Freeze(false)),
                    "Freeze",
                )
                .description("Freeze current spectrum (0 = off, 1 = on)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "fft_size",
                    Param::PhaseVocoder(PhaseVocoderParam::FftSize(FftSizeOption::Fft1024)),
                    "FFT Size",
                    FftSizeOption::ALL
                        .iter()
                        .map(|f| {
                            ChoiceOption::new(f.id(), f.name()).with_description(f.description())
                        })
                        .collect(),
                )
                .description("FFT window size (larger = better frequency resolution)"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::PhaseVocoder(PhaseVocoderParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
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
        let idx = self.fft_size_option.index();

        // Deinterleave — use pre-allocated buffers, clamp to max size
        let num_frames = num_frames.min(MAX_BLOCK_SIZE);

        for i in 0..num_frames {
            let frame = StereoSample::read_frame(input, i);
            self.mono_in_l[i] = frame.left;
            self.mono_in_r[i] = frame.right;
        }

        // Track whether a frozen frame was captured during this block. The STFT
        // processor may emit zero, one, or several frames per block; the first
        // frozen frame captures the live spectrum, later ones replay it.
        let mut captured_left = false;
        let mut captured_right = false;

        // Process left channel
        let prev_phase_l = &mut self.prev_phase_left;
        let phase_accum_l = &mut self.phase_accum_left;
        let frozen_l = &mut self.frozen_left;
        let has_frozen = self.has_frozen;
        let temp_mags = &mut self.temp_magnitudes;
        let temp_phs = &mut self.temp_phases;

        self.stft_left_pool[idx].process(
            &self.mono_in_l[..num_frames],
            &mut self.mono_out_l[..num_frames],
            |spectrum| {
                // `frozen_l` is pre-sized to MAX_BINS; slice to the active spectrum.
                let n = spectrum.len();
                if freeze {
                    if has_frozen || captured_left {
                        // Spectrum already captured (on an earlier block, or an
                        // earlier hop of this same block): replay it. Checking the
                        // per-block `captured_left` flag ensures that when a block
                        // spans multiple STFT hops we hold the FIRST frozen hop
                        // rather than letting later hops overwrite it.
                        spectrum.copy_from_slice(&frozen_l[..n]);
                    } else {
                        // First frozen frame: capture the live spectrum, then play
                        // it back so this frame already outputs the held content.
                        frozen_l[..n].copy_from_slice(spectrum);
                        captured_left = true;
                    }
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
        let frozen_r = &mut self.frozen_right;
        let temp_mags = &mut self.temp_magnitudes;
        let temp_phs = &mut self.temp_phases;

        self.stft_right_pool[idx].process(
            &self.mono_in_r[..num_frames],
            &mut self.mono_out_r[..num_frames],
            |spectrum| {
                let n = spectrum.len();
                if freeze {
                    if has_frozen || captured_right {
                        // Replay the already-captured spectrum (earlier block or
                        // earlier hop of this block); see the left-channel note.
                        spectrum.copy_from_slice(&frozen_r[..n]);
                    } else {
                        frozen_r[..n].copy_from_slice(spectrum);
                        captured_right = true;
                    }
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

        // Latch the freeze once both channels have captured a frame, so the next
        // block replays the held spectrum instead of capturing a fresh one.
        if freeze && captured_left && captured_right {
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
                        // O(1) index switch — just reset the now-active processor + state.
                        self.reset_active();
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
        self.reset_active();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(frames: usize) -> ProcessContext<'static> {
        ProcessContext {
            samples: synth_core::SampleCount::new(frames),
            ..ProcessContext::default()
        }
    }

    /// Switching FFT size now swaps a pre-built pool index (was an audio-thread
    /// STFT rebuild + 7 allocations). Cycle through every size while pitch-shifting,
    /// then freeze and switch size *while frozen* — the freeze path's
    /// `copy_from_slice` against the `MAX_BINS`-sized frozen buffer must slice to the
    /// active spectrum or it panics on a length mismatch. Assert finite + no panic.
    #[test]
    fn fft_size_switch_and_freeze_stays_finite() {
        let mut pv = PhaseVocoder::new();
        pv.set_param(Param::PhaseVocoder(PhaseVocoderParam::PitchShift(
            Semitones(7.0),
        )));
        let frames = 400usize;
        let input: Vec<f32> = (0..frames * 2)
            .map(|i| (i as f32 * 0.021).sin() * 0.4)
            .collect();
        let mut output = vec![0.0f32; frames * 2];

        for opt in FftSizeOption::ALL {
            pv.set_param(Param::PhaseVocoder(PhaseVocoderParam::FftSize(opt)));
            for _ in 0..8 {
                pv.process(&input, &mut output, &ctx(frames));
                assert!(
                    output.iter().all(|s| s.is_finite()),
                    "non-finite output at fft size {}",
                    opt.size()
                );
            }
        }

        // Freeze (exercises the frozen-buffer copy_from_slice slicing), then switch
        // size while frozen — must not panic.
        pv.set_param(Param::PhaseVocoder(PhaseVocoderParam::Freeze(true)));
        for _ in 0..4 {
            pv.process(&input, &mut output, &ctx(frames));
            assert!(output.iter().all(|s| s.is_finite()));
        }
        pv.set_param(Param::PhaseVocoder(PhaseVocoderParam::FftSize(
            FftSizeOption::Fft4096,
        )));
        pv.process(&input, &mut output, &ctx(frames));
        assert!(output.iter().all(|s| s.is_finite()));
    }
}
