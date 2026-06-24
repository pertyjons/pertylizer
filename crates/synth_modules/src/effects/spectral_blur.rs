//! Spectral blur/smear effect.
//!
//! Features:
//! - STFT-based spectral processing with Hann window
//! - Temporal IIR smoothing per frequency bin
//! - Spectral FIR smoothing across frequency bins
//! - Freeze mode (holds current spectrum)

use synth_core::MAX_BLOCK_SIZE;
use synth_core::{
    AudioEffect, Describable, FftSizeOption, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, ProcessContext, SampleRate, SpectralBlurParam,
    StereoSample, WidgetHint,
};
use synth_dsp::{Complex, StftProcessor, WindowType};

/// Maximum FFT size supported.
const MAX_FFT: usize = 4096;
/// Number of complex bins for max FFT.
const MAX_BINS: usize = MAX_FFT / 2 + 1;

/// STFT-based spectral blur/smear effect.
pub struct SpectralBlur {
    // Parameters
    fft_size: FftSizeOption,
    blur_time: NormalizedValue,
    blur_freq: NormalizedValue,
    freeze: bool,
    mix: NormalizedValue,

    // STFT processors, one per channel × one per FftSizeOption. Pre-built in
    // `new()` (off the audio thread, where FFT planning may allocate); switching
    // FFT size is then an O(1) index change with no allocation or re-planning.
    stft_l_pool: Vec<StftProcessor>,
    stft_r_pool: Vec<StftProcessor>,

    // Smoothed magnitude buffers per channel
    mag_smooth_l: Vec<f32>,
    mag_smooth_r: Vec<f32>,
    // Temp buffer for spectral smoothing
    mag_temp: Vec<f32>,

    // Pre-allocated mono I/O buffers (max block size)
    mono_l: Vec<f32>,
    mono_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,

    sample_rate: SampleRate,
}

impl SpectralBlur {
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
            fft_size: FftSizeOption::Fft1024,
            blur_time: NormalizedValue::new(0.7),
            blur_freq: NormalizedValue::new(0.3),
            freeze: false,
            mix: NormalizedValue::MAX,

            stft_l_pool: make_pool(),
            stft_r_pool: make_pool(),

            mag_smooth_l: vec![0.0; MAX_BINS],
            mag_smooth_r: vec![0.0; MAX_BINS],
            mag_temp: vec![0.0; MAX_BINS],

            mono_l: vec![0.0; MAX_BLOCK_SIZE],
            mono_r: vec![0.0; MAX_BLOCK_SIZE],
            out_l: vec![0.0; MAX_BLOCK_SIZE],
            out_r: vec![0.0; MAX_BLOCK_SIZE],

            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    /// Reset the currently-active STFT processors and clear the smoothing state.
    /// Called on FFT-size change (the destination processor must start clean),
    /// sample-rate change, and `reset()` — all allocation-free (the processors are
    /// pre-built in `new()`; this only zeroes ring/accumulator state).
    fn reset_active(&mut self) {
        let idx = self.fft_size.index();
        self.stft_l_pool[idx].reset();
        self.stft_r_pool[idx].reset();
        self.mag_smooth_l.fill(0.0);
        self.mag_smooth_r.fill(0.0);
    }

    /// Apply spectral FIR smoothing across frequency bins.
    fn smooth_freq(mag: &[f32], out: &mut [f32], blur: f32, num_bins: usize) {
        // Kernel half-width: 0 (no blur) to ~64 bins
        #[allow(clippy::cast_possible_truncation)]
        let m = (blur * 64.0) as usize;
        if m == 0 {
            out[..num_bins].copy_from_slice(&mag[..num_bins]);
            return;
        }

        for k in 0..num_bins {
            let lo = k.saturating_sub(m);
            let hi = (k + m + 1).min(num_bins);
            let count = (hi - lo) as f32;
            let mut sum = 0.0f32;
            for j in lo..hi {
                sum += mag[j];
            }
            out[k] = sum / count;
        }
    }
}

impl Default for SpectralBlur {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for SpectralBlur {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("spectral_blur", "Spectral Blur")
            .description("STFT-based spectral smearing effect")
            .category(ModuleCategory::Effect)
            .tag("spectral")
            .tag("effect")
            .tag("blur")
            .parameter(
                ParameterDescriptor::choice(
                    "fft_size",
                    Param::SpectralBlur(SpectralBlurParam::FftSize(FftSizeOption::Fft1024)),
                    "FFT Size",
                    FftSizeOption::to_choices(),
                )
                .description("FFT window size"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "blur_time",
                    Param::SpectralBlur(SpectralBlurParam::BlurTime(NormalizedValue::new(0.7))),
                    "Blur Time",
                )
                .description("Temporal smoothing amount")
                .range(0.0, 1.0)
                .default(0.7)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "blur_freq",
                    Param::SpectralBlur(SpectralBlurParam::BlurFreq(NormalizedValue::new(0.3))),
                    "Blur Freq",
                )
                .description("Spectral smoothing amount")
                .range(0.0, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "freeze",
                    Param::SpectralBlur(SpectralBlurParam::Freeze(false)),
                    "Freeze",
                )
                .description("Hold current spectrum")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::SpectralBlur(SpectralBlurParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for SpectralBlur {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        let idx = self.fft_size.index();
        let fft_size = self.fft_size.size();
        let num_bins = fft_size / 2 + 1;
        let blur_time = self.blur_time.as_f32();
        let blur_freq = self.blur_freq.as_f32();
        let freeze = self.freeze;
        let mix = self.mix.as_f32();

        // Deinterleave input — use pre-allocated buffers, clamp to max size
        let num_frames = context.samples.as_usize();
        let frames = num_frames.min(MAX_BLOCK_SIZE);
        for i in 0..frames {
            let frame = StereoSample::read_frame(input, i);
            self.mono_l[i] = frame.left;
            self.mono_r[i] = frame.right;
        }

        // Capture references to smooth buffers for the closure
        let mag_smooth_l = &mut self.mag_smooth_l;
        let mag_temp = &mut self.mag_temp;

        // Process left channel
        self.stft_l_pool[idx].process(
            &self.mono_l[..frames],
            &mut self.out_l[..frames],
            |spectrum| {
                for (k, bin) in spectrum.iter_mut().enumerate().take(num_bins) {
                    let mag = bin.norm();

                    // Temporal IIR smoothing
                    let smoothed = if freeze {
                        mag_smooth_l[k]
                    } else {
                        let s = (1.0 - blur_time) * mag + blur_time * mag_smooth_l[k];
                        mag_smooth_l[k] = s;
                        s
                    };
                    mag_temp[k] = smoothed;
                }

                // Spectral FIR smoothing (in-place via temp)
                // Apply back to spectrum
                let mut freq_out = [0.0f32; MAX_BINS];
                Self::smooth_freq(mag_temp, &mut freq_out, blur_freq, num_bins);

                for (k, bin) in spectrum.iter_mut().enumerate().take(num_bins) {
                    let phase = bin.arg();
                    *bin = Complex::new(freq_out[k] * phase.cos(), freq_out[k] * phase.sin());
                }
            },
        );

        let mag_smooth_r = &mut self.mag_smooth_r;
        let mag_temp = &mut self.mag_temp;

        // Process right channel
        self.stft_r_pool[idx].process(
            &self.mono_r[..frames],
            &mut self.out_r[..frames],
            |spectrum| {
                for (k, bin) in spectrum.iter_mut().enumerate().take(num_bins) {
                    let mag = bin.norm();

                    let smoothed = if freeze {
                        mag_smooth_r[k]
                    } else {
                        let s = (1.0 - blur_time) * mag + blur_time * mag_smooth_r[k];
                        mag_smooth_r[k] = s;
                        s
                    };
                    mag_temp[k] = smoothed;
                }

                let mut freq_out = [0.0f32; MAX_BINS];
                Self::smooth_freq(mag_temp, &mut freq_out, blur_freq, num_bins);

                for (k, bin) in spectrum.iter_mut().enumerate().take(num_bins) {
                    let phase = bin.arg();
                    *bin = Complex::new(freq_out[k] * phase.cos(), freq_out[k] * phase.sin());
                }
            },
        );

        // Re-interleave with dry/wet mix
        for i in 0..frames {
            let result = StereoSample::new(self.mono_l[i], self.mono_r[i])
                .blend(StereoSample::new(self.out_l[i], self.out_r[i]), mix);
            StereoSample::write_frame(output, i, result);
        }
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

    fn set_param(&mut self, param: Param) {
        if let Param::SpectralBlur(p) = param {
            match p {
                SpectralBlurParam::FftSize(v) => {
                    if v != self.fft_size {
                        self.fft_size = v;
                        // O(1) index switch — just reset the now-active processor.
                        self.reset_active();
                    }
                }
                SpectralBlurParam::BlurTime(v) => self.blur_time = v,
                SpectralBlurParam::BlurFreq(v) => self.blur_freq = v,
                SpectralBlurParam::Freeze(v) => self.freeze = v,
                SpectralBlurParam::Mix(v) => self.mix = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::SpectralBlur(p) = param {
            Some(match p {
                SpectralBlurParam::FftSize(_) => self.fft_size.index() as f32,
                SpectralBlurParam::BlurTime(_) => self.blur_time.as_f32(),
                SpectralBlurParam::BlurFreq(_) => self.blur_freq.as_f32(),
                SpectralBlurParam::Freeze(_) => {
                    if self.freeze {
                        1.0
                    } else {
                        0.0
                    }
                }
                SpectralBlurParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::SpectralBlur(SpectralBlurParam::FftSize(self.fft_size)),
            Param::SpectralBlur(SpectralBlurParam::BlurTime(self.blur_time)),
            Param::SpectralBlur(SpectralBlurParam::BlurFreq(self.blur_freq)),
            Param::SpectralBlur(SpectralBlurParam::Freeze(self.freeze)),
            Param::SpectralBlur(SpectralBlurParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::SpectralBlur
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        // The STFT is sample-rate-agnostic (it works in samples); just reset state.
        self.sample_rate = sample_rate;
        self.reset_active();
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

    /// Switching FFT size (which used to rebuild the STFT + allocate on the audio
    /// thread) now just swaps a pre-built pool index. Cycle through every size
    /// while processing, plus freeze, and assert the output stays finite and
    /// nothing panics (the pool index + pre-sized buffers must all stay in range).
    #[test]
    fn fft_size_switch_and_freeze_stays_finite() {
        let mut sb = SpectralBlur::new();
        let frames = 400usize; // not partition-aligned, to exercise accumulation
        let input: Vec<f32> = (0..frames * 2)
            .map(|i| (i as f32 * 0.013).sin() * 0.5)
            .collect();
        let mut output = vec![0.0f32; frames * 2];

        for opt in FftSizeOption::ALL {
            sb.set_param(Param::SpectralBlur(SpectralBlurParam::FftSize(opt)));
            for _ in 0..8 {
                sb.process(&input, &mut output, &ctx(frames));
                assert!(
                    output.iter().all(|s| s.is_finite()),
                    "non-finite output at fft size {}",
                    opt.size()
                );
            }
        }

        sb.set_param(Param::SpectralBlur(SpectralBlurParam::Freeze(true)));
        for _ in 0..4 {
            sb.process(&input, &mut output, &ctx(frames));
            assert!(output.iter().all(|s| s.is_finite()));
        }
    }
}
