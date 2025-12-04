//! State Variable Filter module.
//!
//! Features:
//! - Multiple filter types (LP, HP, BP, Notch, Peak, Shelving)
//! - Resonance up to self-oscillation
//! - Cutoff and resonance modulation inputs
//! - Key tracking

use std::collections::HashMap;
use std::f32::consts::PI;

use crate::engine::typed_params::{Param, FilterParam, FilterMode, ModuleType};
use crate::modules::core::*;
use crate::types::{BipolarValue, FilterState, Gain, Hertz, MidiNote, NormalizedValue, SampleRate};

/// State Variable Filter with multiple modes.
#[derive(Clone)]
pub struct Filter {
    // Parameters
    filter_type: FilterMode,
    cutoff: Hertz,
    resonance: NormalizedValue,
    key_tracking: NormalizedValue,
    env_amount: BipolarValue,
    drive: Gain,

    // State
    sample_rate: SampleRate,
    ic1eq: f32,
    ic2eq: f32,
    base_note: MidiNote,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl Filter {
    pub fn new() -> Self {
        Self {
            filter_type: FilterMode::Lowpass,
            cutoff: Hertz::new(1000.0),
            resonance: NormalizedValue::MIN,
            key_tracking: NormalizedValue::MIN,
            env_amount: BipolarValue::MAX,
            drive: Gain::UNITY,
            sample_rate: SampleRate::DVD_QUALITY,
            ic1eq: 0.0,
            ic2eq: 0.0,
            base_note: MidiNote::C4,
            output_buffer: AudioBuffer::new(256),
        }
    }

    fn effective_cutoff(&self) -> Hertz {
        let tracking_offset =
            (self.base_note.as_u8() as f32 - 60.0) * self.key_tracking.as_f32() * 100.0;
        let tracked = self.cutoff.as_f32() * (tracking_offset / 1200.0).exp2();
        Hertz::new(tracked.clamp(20.0, self.sample_rate.as_f32() * 0.49))
    }

    #[inline]
    fn process_sample(&mut self, input: f32, cutoff_mod: f32, res_mod: f32) -> f32 {
        let cutoff = (self.effective_cutoff().as_f32() * (1.0 + cutoff_mod))
            .clamp(20.0, self.sample_rate.as_f32() * 0.49);
        let resonance = (self.resonance.as_f32() + res_mod).clamp(0.0, 1.0);

        let g = (PI * cutoff / self.sample_rate.as_f32()).tan();
        let k = 2.0 - 2.0 * resonance;

        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = input - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;

        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        match self.filter_type {
            FilterMode::Lowpass => v2,
            FilterMode::Highpass => input - k * v1 - v2,
            FilterMode::Bandpass => v1,
            FilterMode::Notch => input - k * v1,
            FilterMode::Peak => {
                let lp = v2;
                let hp = input - k * v1 - v2;
                lp - hp
            }
            FilterMode::LowShelf => {
                let lp = v2;
                input * 0.5 + lp * 0.5
            }
            FilterMode::HighShelf => {
                let hp = input - k * v1 - v2;
                input * 0.5 + hp * 0.5
            }
        }
    }
}

impl Default for Filter {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Filter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("filter", "Filter")
            .description("State Variable Filter with multiple modes")
            .category(ModuleCategory::Filter)
            .tag("filter")
            .tag("svf")
            .parameter(
                ParameterDescriptor::choice(
                    Param::Filter(FilterParam::Mode(FilterMode::Lowpass)),
                    "Type",
                    FilterMode::to_choices(),
                )
                .description("Filter type"),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Filter(FilterParam::Cutoff(Hertz::new(1000.0))),
                    "Cutoff",
                )
                .description("Cutoff frequency")
                .range(20.0, 20000.0)
                .default(1000.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::FrequencySlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Filter(FilterParam::Resonance(NormalizedValue::MIN)),
                    "Resonance",
                )
                .description("Filter resonance (Q)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Filter(FilterParam::KeyTracking(NormalizedValue::MIN)),
                    "Key Track",
                )
                .description("Keyboard tracking amount")
                .range(0.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Filter(FilterParam::EnvAmount(BipolarValue::MAX)),
                    "Env Amt",
                )
                .description("Envelope modulation amount (-1 to +1)")
                .range(-1.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in", "In").description("Audio input"))
            .port(
                PortDescriptor::control_input("cutoff_cv", "Cutoff CV")
                    .description("Cutoff modulation"),
            )
            .port(
                PortDescriptor::control_input("res_cv", "Res CV").description("Resonance modulation"),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Filtered output"))
    }
}

impl PolyModule for Filter {
    fn process(
        &mut self,
        inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        let audio_in = inputs.get("in");
        let cutoff_cv = inputs.get("cutoff_cv");
        let res_cv = inputs.get("res_cv");

        for i in 0..context.samples {
            let input = audio_in.map(|b| b[i]).unwrap_or(0.0);
            let cutoff_mod = cutoff_cv
                .map(|b| b[i] * self.env_amount.as_f32())
                .unwrap_or(0.0);
            let res_mod = res_cv.map(|b| b[i]).unwrap_or(0.0);

            self.output_buffer[i] = self.process_sample(input, cutoff_mod, res_mod);
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Filter(filter_param) = param {
            match filter_param {
                FilterParam::Mode(m) => self.filter_type = m,
                FilterParam::Cutoff(c) => self.cutoff = c.clamp_audible(),
                FilterParam::Resonance(r) => self.resonance = r,
                FilterParam::KeyTracking(k) => self.key_tracking = k,
                FilterParam::Drive(d) => self.drive = d,
                FilterParam::EnvAmount(e) => self.env_amount = e,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Filter(filter_param) = param {
            Some(match filter_param {
                FilterParam::Mode(_) => self.filter_type.index() as f32,
                FilterParam::Cutoff(_) => self.cutoff.as_f32(),
                FilterParam::Resonance(_) => self.resonance.as_f32(),
                FilterParam::KeyTracking(_) => self.key_tracking.as_f32(),
                FilterParam::Drive(_) => self.drive.as_f32(),
                FilterParam::EnvAmount(_) => self.env_amount.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Filter(FilterParam::Mode(self.filter_type)),
            Param::Filter(FilterParam::Cutoff(self.cutoff)),
            Param::Filter(FilterParam::Resonance(self.resonance)),
            Param::Filter(FilterParam::KeyTracking(self.key_tracking)),
            Param::Filter(FilterParam::Drive(self.drive)),
            Param::Filter(FilterParam::EnvAmount(self.env_amount)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Filter
    }

    fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: f32) {
        self.base_note = note;
    }

    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = SampleRate::new(sample_rate);
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

/// Moog-style 24dB/oct ladder filter.
#[derive(Clone)]
pub struct LadderFilter {
    cutoff: Hertz,
    resonance: NormalizedValue,
    drive: Gain,
    sample_rate: SampleRate,
    /// 4-stage filter state (type-safe DSP state)
    stage: [FilterState; 4],
    /// 4-stage delay state (type-safe DSP state)
    delay: [FilterState; 4],
    output_buffer: AudioBuffer,
}

impl LadderFilter {
    pub fn new() -> Self {
        Self {
            cutoff: Hertz::new(1000.0),
            resonance: NormalizedValue::MIN,
            drive: Gain::UNITY,
            sample_rate: SampleRate::DVD_QUALITY,
            stage: [FilterState::ZERO; 4],
            delay: [FilterState::ZERO; 4],
            output_buffer: AudioBuffer::new(256),
        }
    }

    #[inline]
    fn saturate(x: f32) -> f32 {
        x.tanh()
    }

    #[inline]
    fn process_sample(&mut self, input: f32, effective_cutoff: Hertz) -> f32 {
        let cutoff = effective_cutoff
            .as_f32()
            .clamp(20.0, self.sample_rate.as_f32() * 0.49);
        let g = (PI * cutoff / self.sample_rate.as_f32()).tan();
        let k = self.resonance.as_f32() * 4.0;

        let driven = if self.drive.as_f32() > 1.0 {
            Self::saturate(input * self.drive.as_f32())
        } else {
            input
        };

        let feedback = k * self.delay[3].as_f32();
        let input_with_fb = driven - feedback;

        for i in 0..4 {
            let prev = if i == 0 {
                input_with_fb
            } else {
                self.stage[i - 1].as_f32()
            };
            let delay_val = self.delay[i].as_f32();
            let new_stage = (prev - delay_val) * g / (1.0 + g) + delay_val;

            self.stage[i] = FilterState::new(if self.drive.as_f32() > 1.0 {
                Self::saturate(new_stage)
            } else {
                new_stage
            });
            self.delay[i] = self.stage[i];
        }

        self.stage[3].as_f32()
    }
}

impl Default for LadderFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for LadderFilter {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("ladder_filter", "Ladder Filter")
            .description("24dB/oct Moog-style ladder filter")
            .category(ModuleCategory::Filter)
            .tag("filter")
            .tag("moog")
            .tag("ladder")
            .parameter(
                ParameterDescriptor::float(
                    Param::Filter(FilterParam::Cutoff(Hertz::new(1000.0))),
                    "Cutoff",
                )
                .description("Cutoff frequency")
                .range(20.0, 20000.0)
                .default(1000.0)
                .unit(ParameterUnit::Hertz)
                .widget(WidgetHint::FrequencySlider)
                .curve(ResponseCurve::Logarithmic),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Filter(FilterParam::Resonance(NormalizedValue::MIN)),
                    "Resonance",
                )
                .description("Filter resonance")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Filter(FilterParam::Drive(Gain::UNITY)),
                    "Drive",
                )
                .description("Saturation amount")
                .range(0.0, 4.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in", "In"))
            .port(PortDescriptor::control_input("cutoff_cv", "Cutoff CV"))
            .port(PortDescriptor::audio_output("out", "Out"))
    }
}

impl PolyModule for LadderFilter {
    fn process(
        &mut self,
        inputs: &HashMap<String, &AudioBuffer>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        let audio_in = inputs.get("in");
        let cutoff_cv = inputs.get("cutoff_cv");

        for i in 0..context.samples {
            let input = audio_in.map(|b| b[i]).unwrap_or(0.0);

            let effective_cutoff = if let Some(cv) = cutoff_cv {
                let mod_amount = cv[i];
                Hertz::new(
                    (self.cutoff.as_f32() * (mod_amount * 4.0).exp2()).clamp(20.0, 20000.0),
                )
            } else {
                self.cutoff
            };

            self.output_buffer[i] = self.process_sample(input, effective_cutoff);
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Filter(filter_param) = param {
            match filter_param {
                FilterParam::Cutoff(c) => self.cutoff = c.clamp_audible(),
                FilterParam::Resonance(r) => self.resonance = r,
                FilterParam::Drive(d) => self.drive = d,
                _ => {}
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Filter(filter_param) = param {
            Some(match filter_param {
                FilterParam::Cutoff(_) => self.cutoff.as_f32(),
                FilterParam::Resonance(_) => self.resonance.as_f32(),
                FilterParam::Drive(_) => self.drive.as_f32(),
                _ => return None,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Filter(FilterParam::Cutoff(self.cutoff)),
            Param::Filter(FilterParam::Resonance(self.resonance)),
            Param::Filter(FilterParam::Drive(self.drive)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Filter
    }

    fn reset(&mut self) {
        self.stage.fill(FilterState::ZERO);
        self.delay.fill(FilterState::ZERO);
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: f32) {}
    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = SampleRate::new(sample_rate);
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_creation() {
        let filter = Filter::new();
        assert_eq!(filter.filter_type, FilterMode::Lowpass);
        assert!((filter.cutoff.as_f32() - 1000.0).abs() < 0.001);
    }

    #[test]
    fn test_filter_stability() {
        let mut filter = Filter::new();
        filter.sample_rate = SampleRate::DVD_QUALITY;
        filter.cutoff = Hertz::new(100.0);
        filter.resonance = NormalizedValue::new(0.99);

        for _ in 0..1000 {
            let out = filter.process_sample(0.5, 0.0, 0.0);
            assert!(out.is_finite(), "Filter output is not finite");
            assert!(out.abs() < 100.0, "Filter output exploded");
        }
    }
}
