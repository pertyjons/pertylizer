//! Crossover splitter effect using Linkwitz-Riley filters.
//!
//! Splits audio into low and high frequency bands with phase-coherent
//! 4th-order Linkwitz-Riley crossover filters. Allows independent gain
//! control of each band.
//!
//! Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Filters/266-4th-order-linkwitz-riley-filters.rst
//! From the Music-DSP Source Code Archive (https://www.musicdsp.org/)

use synth_core::{
    AudioEffect, CrossoverParam, Describable, Hertz, ModuleCategory, ModuleDescriptor, ModuleType,
    NormalizedValue, Param, ParameterDescriptor, PortDescriptor, ProcessContext, ResponseCurve,
    SampleCount, SampleRate, StereoSample, WidgetHint,
};
use synth_dsp::{LinkwitzRiley, LinkwitzRileyCoeffs};

/// Crossover splitter — splits audio into low/high bands with independent gain.
pub struct CrossoverSplitter {
    frequency: Hertz,
    low_gain: NormalizedValue,
    high_gain: NormalizedValue,
    mix: NormalizedValue,
    lr_left: LinkwitzRiley,
    lr_right: LinkwitzRiley,
    sample_rate: SampleRate,
}

impl CrossoverSplitter {
    pub fn new() -> Self {
        Self {
            frequency: Hertz::new(1000.0),
            low_gain: NormalizedValue::MAX,
            high_gain: NormalizedValue::MAX,
            mix: NormalizedValue::MAX,
            lr_left: LinkwitzRiley::new(),
            lr_right: LinkwitzRiley::new(),
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }
}

impl Default for CrossoverSplitter {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for CrossoverSplitter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("crossover_splitter", "Crossover Splitter")
            .description("Phase-coherent frequency band splitter with independent gain")
            .category(ModuleCategory::Effect)
            .tag("filter")
            .tag("effect")
            .tag("crossover")
            .port(PortDescriptor::audio_input("in_l", "In L"))
            .port(PortDescriptor::audio_input("in_r", "In R"))
            .port(PortDescriptor::audio_output("out_l", "Out L"))
            .port(PortDescriptor::audio_output("out_r", "Out R"))
            .parameter(
                ParameterDescriptor::float(
                    "frequency",
                    Param::Crossover(CrossoverParam::Frequency(Hertz::new(1000.0))),
                    "Frequency",
                )
                .description("Crossover frequency")
                .value_range(Hertz::CROSSOVER_RANGE)
                .widget(WidgetHint::Knob)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    "low_gain",
                    Param::Crossover(CrossoverParam::LowGain(NormalizedValue::MAX)),
                    "Low Gain",
                )
                .description("Low band gain")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "high_gain",
                    Param::Crossover(CrossoverParam::HighGain(NormalizedValue::MAX)),
                    "High Gain",
                )
                .description("High band gain")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::Crossover(CrossoverParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for CrossoverSplitter {
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        self.sample_rate = context.sample_rate;

        let low_gain = self.low_gain.as_f32();
        let high_gain = self.high_gain.as_f32();
        let mix = self.mix.as_f32();
        let coeffs = LinkwitzRileyCoeffs::new(self.frequency, self.sample_rate);

        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);

            let (low_l, high_l) = self.lr_left.process_with_coeffs(dry.left, &coeffs);
            let (low_r, high_r) = self.lr_right.process_with_coeffs(dry.right, &coeffs);

            let wet = StereoSample::new(
                low_l * low_gain + high_l * high_gain,
                low_r * low_gain + high_r * high_gain,
            );

            let result = dry.blend(wet, mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Crossover(p) = param {
            match p {
                CrossoverParam::Frequency(f) => {
                    self.frequency = Hertz::new(Hertz::CROSSOVER_RANGE.clamp(f.as_f32()));
                }
                CrossoverParam::LowGain(g) => self.low_gain = g,
                CrossoverParam::HighGain(g) => self.high_gain = g,
                CrossoverParam::Mix(m) => self.mix = m,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Crossover(p) = param {
            Some(match p {
                CrossoverParam::Frequency(_) => self.frequency.as_f32(),
                CrossoverParam::LowGain(_) => self.low_gain.as_f32(),
                CrossoverParam::HighGain(_) => self.high_gain.as_f32(),
                CrossoverParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Crossover(CrossoverParam::Frequency(self.frequency)),
            Param::Crossover(CrossoverParam::LowGain(self.low_gain)),
            Param::Crossover(CrossoverParam::HighGain(self.high_gain)),
            Param::Crossover(CrossoverParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::CrossoverSplitter
    }

    fn reset(&mut self) {
        self.lr_left.reset();
        self.lr_right.reset();
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn tail_samples(&self) -> SampleCount {
        SampleCount::ZERO
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift guard: the `frequency` descriptor range and the `set_param`
    /// apply-clamp share the single `Hertz::CROSSOVER_RANGE` preset.
    #[test]
    fn frequency_descriptor_matches_crossover_range() {
        let d = CrossoverSplitter::new().descriptor();
        let freq = d
            .parameters
            .iter()
            .find(|p| p.type_id == "frequency")
            .expect("missing frequency param");
        assert_eq!(freq.range, Hertz::CROSSOVER_RANGE);
    }
}
