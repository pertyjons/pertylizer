//! Spectrum Analyzer visualization module.
//!
//! Displays the frequency spectrum via FFT. Pass-through module that
//! captures samples for spectral display without modifying the signal.

use synth_core::*;
use synth_core::{Gain, ModuleType, Param, SpectrumAnalyzerParam};

use synth_dsp::{Complex, FftProcessor, WindowType, fill_window};

use super::VisualizationBuffer;

/// Default FFT size for spectrum analysis (good frequency resolution).
const FFT_SIZE: usize = 2048;

/// Spectrum analyzer module for frequency spectrum visualization.
pub struct SpectrumAnalyzer {
    /// Shared buffer for GUI display (magnitude data written as L-channel snapshot).
    buffer: VisualizationBuffer,
    /// Vertical gain scaling.
    gain: Gain,
    /// Wet/dry mix (always 1.0 for pass-through).
    mix: NormalizedValue,
    /// Sample rate.
    sample_rate: SampleRate,
    /// Pre-allocated FFT processor.
    fft: FftProcessor,
    /// Ring buffer for collecting mono input samples.
    fft_input: Vec<f32>,
    /// Pre-allocated Hann window.
    window: Vec<f32>,
    /// Pre-allocated FFT output (complex bins).
    fft_output: Vec<Complex<f32>>,
    /// Pre-allocated magnitude buffer (dB values per bin).
    magnitude_buf: Vec<f32>,
    /// Current write position in the ring buffer.
    write_pos: usize,
}

impl SpectrumAnalyzer {
    pub fn new() -> Self {
        let fft = FftProcessor::new(FFT_SIZE);
        let complex_size = fft.complex_size();

        let mut window = vec![0.0; FFT_SIZE];
        fill_window(&mut window, WindowType::Hann);

        Self {
            buffer: VisualizationBuffer::new(complex_size),
            gain: Gain::UNITY,
            mix: NormalizedValue::MAX,
            sample_rate: SampleRate::DVD_QUALITY,
            fft,
            fft_input: vec![0.0; FFT_SIZE],
            window,
            fft_output: vec![Complex { re: 0.0, im: 0.0 }; complex_size],
            magnitude_buf: vec![0.0; complex_size],
            write_pos: 0,
        }
    }
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SpectrumAnalyzer {
    fn clone(&self) -> Self {
        // Rebuild from scratch since FftProcessor contains Arc plans
        Self::new()
    }
}

impl Describable for SpectrumAnalyzer {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("spectrum_analyzer", "Spectrum")
            .description("FFT spectrum analyzer display")
            .category(ModuleCategory::Visualizer)
            .tag("visualizer")
            .tag("spectrum")
            .tag("fft")
            .parameter(
                ParameterDescriptor::float(
                    "gain",
                    Param::SpectrumAnalyzer(SpectrumAnalyzerParam::Gain(Gain::UNITY)),
                    "Gain",
                )
                .description("Vertical gain scaling")
                .range(0.1, 10.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
        // No ports - visualizers receive the final output signal automatically
    }
}

impl AudioEffect for SpectrumAnalyzer {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext) {
        self.sample_rate = context.sample_rate;

        // Pass-through: copy input to output
        output.copy_from_slice(input);

        // Collect mono samples (L+R)/2 into ring buffer
        let num_frames = input.len() / 2;
        for i in 0..num_frames {
            let mono = (input[i * 2] + input[i * 2 + 1]) * 0.5;
            self.fft_input[self.write_pos] = mono;
            self.write_pos += 1;

            // When we have a full FFT block, process it
            if self.write_pos >= FFT_SIZE {
                self.write_pos = 0;
                self.run_fft();
            }
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::SpectrumAnalyzer(sa_param) = param {
            match sa_param {
                SpectrumAnalyzerParam::Gain(g) => self.gain = g,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::SpectrumAnalyzer(sa_param) = param {
            Some(match sa_param {
                SpectrumAnalyzerParam::Gain(_) => self.gain.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![Param::SpectrumAnalyzer(SpectrumAnalyzerParam::Gain(
            self.gain,
        ))]
    }

    fn reset(&mut self) {
        self.fft_input.fill(0.0);
        self.write_pos = 0;
        self.magnitude_buf.fill(0.0);
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn tail_samples(&self) -> SampleCount {
        SampleCount::ZERO // No tail - pass-through
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::SpectrumAnalyzer
    }
}

impl SpectrumAnalyzer {
    /// Run FFT on the collected input and update the visualization buffer.
    fn run_fft(&mut self) {
        // Apply window function to a copy (fft_input is mutated by FFT)
        let mut windowed = self.fft_input.clone();
        for i in 0..FFT_SIZE {
            windowed[i] *= self.window[i];
        }

        // Run forward FFT
        self.fft.forward(&mut windowed, &mut self.fft_output);

        // Compute magnitude in dB
        let gain = self.gain.as_f32();
        let norm = 1.0 / FFT_SIZE as f32;
        for (i, bin) in self.fft_output.iter().enumerate() {
            let magnitude = (bin.re * bin.re + bin.im * bin.im).sqrt() * norm * 2.0;
            // Convert to dB, floor at -100dB
            let db = if magnitude > 1e-10 {
                20.0 * magnitude.log10()
            } else {
                -100.0
            };
            // Normalize: map -100dB..0dB to 0.0..1.0, apply gain
            self.magnitude_buf[i] = ((db + 100.0) / 100.0 * gain).clamp(0.0, 1.0);
        }

        // Write magnitude data as L-channel snapshot for GUI to read
        self.buffer
            .write_samples(&self.magnitude_buf, &self.magnitude_buf);
    }
}
