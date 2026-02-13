//! State Variable Filter module.
//!
//! Features:
//! - Multiple filter types (LP, HP, BP, Notch, Peak, Shelving)
//! - Resonance up to self-oscillation
//! - Cutoff and resonance modulation inputs
//! - Key tracking

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, ResponseCurve, WidgetHint,
};
use synth_core::{
    BipolarValue, FilterState, Gain, Hertz, MidiNote, NormalizedValue, PortName, SampleRate,
    Velocity,
};
use synth_core::{FilterMode, FilterModel, FilterParam, ModuleType, Param};
use synth_dsp::{AcidFilter, FluidFilter, ScreamerFilter, SvfCoeffs, SvfFilterType};

/// State Variable Filter with multiple modes.
#[derive(Clone)]
pub struct Filter {
    // Parameters
    filter_type: FilterMode,
    model: FilterModel,
    morph: NormalizedValue,
    cutoff: Hertz,
    resonance: NormalizedValue,
    key_tracking: NormalizedValue,
    env_amount: BipolarValue,
    cutoff_mod_amount: BipolarValue,
    drive: Gain,

    // State
    sample_rate: SampleRate,
    /// Integrator 1 state (type-safe DSP state).
    ic1eq: FilterState,
    /// Integrator 2 state (type-safe DSP state).
    ic2eq: FilterState,
    base_note: MidiNote,

    // Character filter states
    fluid: FluidFilter,
    screamer: ScreamerFilter,
    acid: AcidFilter,

    // Mod matrix offsets (applied before processing, cleared after)
    /// Cutoff modulation offset in semitones.
    mod_offset_cutoff: f32,
    /// Resonance modulation offset (additive, 0-1 range).
    mod_offset_resonance: f32,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl Filter {
    pub fn new() -> Self {
        Self {
            filter_type: FilterMode::Lowpass,
            model: FilterModel::Standard,
            morph: NormalizedValue::MIN,
            cutoff: Hertz::new(1000.0),
            resonance: NormalizedValue::MIN,
            key_tracking: NormalizedValue::MIN,
            env_amount: BipolarValue::MAX,
            cutoff_mod_amount: BipolarValue::MAX,
            drive: Gain::UNITY,
            sample_rate: SampleRate::DVD_QUALITY,
            ic1eq: FilterState::ZERO,
            ic2eq: FilterState::ZERO,
            base_note: MidiNote::C4,
            fluid: FluidFilter::default(),
            screamer: ScreamerFilter::default(),
            acid: AcidFilter::default(),
            mod_offset_cutoff: 0.0,
            mod_offset_resonance: 0.0,
            output_buffer: AudioBuffer::new(256),
        }
    }

    fn effective_cutoff(&self) -> Hertz {
        let tracking_offset =
            (self.base_note.as_u8() as f32 - 60.0) * self.key_tracking.as_f32() * 100.0;
        // Apply mod matrix offset (in semitones, converted to exponential scaling)
        let total_offset = tracking_offset + self.mod_offset_cutoff * 100.0;
        let tracked = self.cutoff.as_f32() * (total_offset / 1200.0).exp2();
        Hertz::new(tracked.clamp(20.0, self.sample_rate.as_f32() * 0.49))
    }

    /// Map FilterMode to SvfFilterType for character filters.
    fn filter_mode_to_svf_type(&self) -> SvfFilterType {
        match self.filter_type {
            FilterMode::Lowpass => SvfFilterType::Lowpass,
            FilterMode::Highpass => SvfFilterType::Highpass,
            FilterMode::Bandpass => SvfFilterType::Bandpass,
            FilterMode::Notch => SvfFilterType::Notch,
            FilterMode::Peak => SvfFilterType::Peak,
            FilterMode::LowShelf => SvfFilterType::LowShelf,
            FilterMode::HighShelf => SvfFilterType::HighShelf,
        }
    }

    /// Reset all filter states (standard SVF + character filters).
    fn reset_filter_states(&mut self) {
        self.ic1eq = FilterState::ZERO;
        self.ic2eq = FilterState::ZERO;
        self.fluid.reset();
        self.screamer.reset();
        self.acid.reset();
    }

    #[inline]
    #[allow(clippy::too_many_lines)]
    fn process_sample(&mut self, input: f32, cutoff_mod: f32, res_mod: f32) -> f32 {
        let cutoff_hz = (self.effective_cutoff().as_f32() * (1.0 + cutoff_mod))
            .clamp(20.0, self.sample_rate.as_f32() * 0.49);
        let cutoff = Hertz::new(cutoff_hz);
        // Clamp resonance to 0.99 max to prevent instability at self-oscillation
        let resonance =
            (self.resonance.as_f32() + res_mod + self.mod_offset_resonance).clamp(0.0, 0.99);

        match self.model {
            FilterModel::Standard => {
                // Apply drive as pre-gain with soft saturation
                let driven = if self.drive.as_f32() > 1.0 {
                    (input * self.drive.as_f32()).tanh()
                } else {
                    input * self.drive.as_f32()
                };

                let g = cutoff.to_tan_coeff(self.sample_rate);
                let k = 2.0 - 2.0 * resonance;

                let a1 = 1.0 / (1.0 + g * (g + k));
                let a2 = g * a1;
                let a3 = g * a2;

                let ic1 = self.ic1eq.as_f32();
                let ic2 = self.ic2eq.as_f32();

                let v3 = driven - ic2;
                let v1 = a1 * ic1 + a2 * v3;
                let v2 = ic2 + a2 * ic1 + a3 * v3;

                self.ic1eq = FilterState::new(2.0 * v1 - ic1);
                self.ic2eq = FilterState::new(2.0 * v2 - ic2);

                // Prevent denormals for consistent performance
                self.ic1eq.flush_denormals();
                self.ic2eq.flush_denormals();

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
            FilterModel::Fluid => {
                let coeffs = SvfCoeffs::new(cutoff, resonance, self.sample_rate);
                let out =
                    self.fluid
                        .process(input, &coeffs, self.drive.as_f32(), self.morph.as_f32());
                self.fluid.flush_denormals();
                out
            }
            FilterModel::Screamer => {
                let g = cutoff.to_tan_coeff(self.sample_rate);
                let out = self
                    .screamer
                    .process(input, g, resonance, self.drive.as_f32());
                self.screamer.flush_denormals();
                out
            }
            FilterModel::Acid => {
                let g = cutoff.to_tan_coeff(self.sample_rate);
                let svf_type = self.filter_mode_to_svf_type();
                let out = self
                    .acid
                    .process(input, g, resonance, self.drive.as_f32(), svf_type);
                self.acid.flush_denormals();
                out
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
                    Param::Filter(FilterParam::Model(FilterModel::Standard)),
                    "Model",
                    FilterModel::to_choices(),
                )
                .description("Filter character model"),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Filter(FilterParam::Morph(NormalizedValue::MIN)),
                    "Morph",
                )
                .range(0.0, 1.0)
                .default(0.0)
                .description("Fluid: LP→BP→HP→Notch crossfade")
                .widget(WidgetHint::Knob),
            )
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
                    Param::Filter(FilterParam::CutoffMod(BipolarValue::MAX)),
                    "CV Amt",
                )
                .description("Cutoff CV input attenuverter (-1 to +1)")
                .range(-1.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(Param::Filter(FilterParam::Drive(Gain::UNITY)), "Drive")
                    .description("Input gain with soft saturation (above 1.0)")
                    .range(0.5, 4.0)
                    .default(1.0)
                    .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in", "In").description("Audio input"))
            .port(
                PortDescriptor::control_input("cutoff_cv", "Cutoff CV")
                    .description("Cutoff modulation"),
            )
            .port(
                PortDescriptor::control_input("res_cv", "Res CV")
                    .description("Resonance modulation"),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Filtered output"))
    }
}

impl PolyModule for Filter {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let audio_in = inputs.get(PortName::IN);
        let cutoff_cv = inputs.get(PortName::CUTOFF_CV);
        let res_cv = inputs.get(PortName::RESONANCE_CV);

        for i in 0..context.samples.as_usize() {
            let input = audio_in.map(|b| b[i]).unwrap_or(0.0);
            let cutoff_mod = cutoff_cv
                .map(|b| b[i] * self.cutoff_mod_amount.as_f32())
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
                FilterParam::CutoffMod(c) => self.cutoff_mod_amount = c,
                FilterParam::Model(m) => {
                    self.model = m;
                    self.reset_filter_states();
                }
                FilterParam::Morph(v) => self.morph = v,
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
                FilterParam::CutoffMod(_) => self.cutoff_mod_amount.as_f32(),
                FilterParam::Model(_) => self.model.index() as f32,
                FilterParam::Morph(_) => self.morph.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Filter(FilterParam::Model(self.model)),
            Param::Filter(FilterParam::Morph(self.morph)),
            Param::Filter(FilterParam::Mode(self.filter_type)),
            Param::Filter(FilterParam::Cutoff(self.cutoff)),
            Param::Filter(FilterParam::Resonance(self.resonance)),
            Param::Filter(FilterParam::KeyTracking(self.key_tracking)),
            Param::Filter(FilterParam::Drive(self.drive)),
            Param::Filter(FilterParam::CutoffMod(self.cutoff_mod_amount)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Filter
    }

    fn reset(&mut self) {
        self.reset_filter_states();
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.base_note = note;
    }

    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    fn set_mod_offset(&mut self, dest_index: u8, value: f32) {
        match dest_index {
            0 => self.mod_offset_cutoff += value,
            1 => self.mod_offset_resonance += value,
            _ => {}
        }
    }

    fn clear_mod_offsets(&mut self) {
        self.mod_offset_cutoff = 0.0;
        self.mod_offset_resonance = 0.0;
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
        let cutoff = Hertz::new(
            effective_cutoff
                .as_f32()
                .clamp(20.0, self.sample_rate.as_f32() * 0.49),
        );
        let g = cutoff.to_tan_coeff(self.sample_rate);
        // Clamp resonance to 0.99 max for stability, then scale to feedback gain
        let k = self.resonance.as_f32().min(0.99) * 4.0;

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

        // Prevent denormals for consistent performance
        for state in &mut self.stage {
            state.flush_denormals();
        }
        for state in &mut self.delay {
            state.flush_denormals();
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
                ParameterDescriptor::float(Param::Filter(FilterParam::Drive(Gain::UNITY)), "Drive")
                    .description("Saturation amount (soft clipping above 1.0)")
                    .range(0.5, 4.0)
                    .default(1.0)
                    .unit(ParameterUnit::None)
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
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let audio_in = inputs.get(PortName::IN);
        let cutoff_cv = inputs.get(PortName::CUTOFF_CV);

        for i in 0..context.samples.as_usize() {
            let input = audio_in.map(|b| b[i]).unwrap_or(0.0);

            let effective_cutoff = if let Some(cv) = cutoff_cv {
                let mod_amount = cv[i];
                Hertz::new((self.cutoff.as_f32() * (mod_amount * 4.0).exp2()).clamp(20.0, 20000.0))
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

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}
    fn note_off(&mut self) {}

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
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
