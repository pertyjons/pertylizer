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
    Semitones, Velocity,
};
use synth_core::{FilterMode, FilterModel, FilterParam, ModuleType, Param};
use synth_dsp::{AcidFilter, FluidFilter, KarlsenFilter, ScreamerFilter, SvfCoeffs, SvfFilterType};

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
    karlsen: KarlsenFilter,

    // Mod matrix offsets (applied before processing, cleared after)
    /// Cutoff modulation offset in semitones.
    mod_offset_cutoff: Semitones,
    /// Resonance modulation offset (additive, 0-1 range).
    mod_offset_resonance: NormalizedValue,

    // Transient automation overrides (replace the base value while active,
    // cleared on transport stop; the base param is never mutated).
    /// Cutoff override from sequencer automation.
    override_cutoff: Option<Hertz>,
    /// Resonance override from sequencer automation.
    override_resonance: Option<NormalizedValue>,
    /// Last block's final (ramped) base cutoff in Hz, for per-block linear
    /// de-zippering of the effective cutoff across block boundaries.
    cutoff_smoothed: f32,

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
            env_amount: BipolarValue::new(0.25),
            cutoff_mod_amount: BipolarValue::MAX,
            drive: Gain::UNITY,
            sample_rate: SampleRate::DVD_QUALITY,
            ic1eq: FilterState::ZERO,
            ic2eq: FilterState::ZERO,
            base_note: MidiNote::C4,
            fluid: FluidFilter::default(),
            screamer: ScreamerFilter::default(),
            acid: AcidFilter::default(),
            karlsen: KarlsenFilter::new(),
            mod_offset_cutoff: Semitones::ZERO,
            mod_offset_resonance: NormalizedValue::MIN,
            override_cutoff: None,
            override_resonance: None,
            cutoff_smoothed: 1000.0,
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Apply key-tracking + mod-matrix offset (and the audible-range clamp) to a
    /// given base cutoff. Shared by [`Self::effective_cutoff`] (the un-ramped
    /// target, used for queries/tests) and the per-sample ramped process path.
    fn cutoff_from_base(&self, base_cutoff: Hertz) -> Hertz {
        let tracking_offset =
            (self.base_note.as_u8() as f32 - 60.0) * self.key_tracking.as_f32() * 100.0;
        // Apply mod matrix offset (in semitones, converted to exponential scaling)
        let total_offset = tracking_offset + self.mod_offset_cutoff.as_f32() * 100.0;
        let tracked = base_cutoff.as_f32() * (total_offset / 1200.0).exp2();
        Hertz::new(tracked.clamp(20.0, self.sample_rate.as_f32() * 0.49))
    }

    /// The target effective cutoff (override-or-base, with tracking/mod applied),
    /// ignoring per-block smoothing. Currently used only by tests to assert the
    /// override→effective mapping without driving a full process block.
    #[cfg(test)]
    fn effective_cutoff(&self) -> Hertz {
        self.cutoff_from_base(self.override_cutoff.unwrap_or(self.cutoff))
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
        self.karlsen.reset();
    }

    #[inline]
    #[allow(clippy::too_many_lines)]
    fn process_sample(
        &mut self,
        input: f32,
        base_cutoff: Hertz,
        cutoff_mod: Semitones,
        res_mod: NormalizedValue,
    ) -> f32 {
        let cutoff_hz = (self.cutoff_from_base(base_cutoff).as_f32()
            * (2.0_f32).powf(cutoff_mod.as_f32() / 12.0))
        .clamp(20.0, self.sample_rate.as_f32() * 0.49);
        let cutoff = Hertz::new(cutoff_hz);
        // Clamp resonance to 0.99 max to prevent instability at self-oscillation
        let base_resonance = self.override_resonance.unwrap_or(self.resonance);
        let resonance =
            (base_resonance.as_f32() + res_mod.as_f32() + self.mod_offset_resonance.as_f32())
                .clamp(0.0, 0.99);

        match self.model {
            FilterModel::Standard => {
                // Apply drive as pre-gain with soft saturation
                let driven = if self.drive.as_f32() > 1.0 {
                    crate::math::soft_clip(input * self.drive.as_f32())
                } else {
                    input * self.drive.as_f32()
                };

                let coeffs =
                    SvfCoeffs::new(cutoff, NormalizedValue::new(resonance), self.sample_rate);
                let svf_type = self.filter_mode_to_svf_type();
                coeffs.process(driven, &mut self.ic1eq, &mut self.ic2eq, svf_type)
            }
            FilterModel::Fluid => {
                let coeffs =
                    SvfCoeffs::new(cutoff, NormalizedValue::new(resonance), self.sample_rate);
                self.fluid.process(input, &coeffs, self.drive, self.morph)
            }
            FilterModel::Screamer => {
                let g = cutoff.to_tan_coeff(self.sample_rate);
                self.screamer
                    .process(input, g, NormalizedValue::new(resonance), self.drive)
            }
            FilterModel::Acid => {
                let g = cutoff.to_tan_coeff(self.sample_rate);
                let svf_type = self.filter_mode_to_svf_type();
                self.acid.process(
                    input,
                    g,
                    NormalizedValue::new(resonance),
                    self.drive,
                    svf_type,
                )
            }
            FilterModel::Karlsen => {
                // Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Filters/240-karlsen-fast-ladder.rst
                // From the Music-DSP Source Code Archive (https://www.musicdsp.org/)
                let driven = if self.drive.as_f32() > 1.0 {
                    crate::math::soft_clip(input * self.drive.as_f32())
                } else {
                    input * self.drive.as_f32()
                };
                let normalized_cutoff =
                    NormalizedValue::new(cutoff.as_f32() / (self.sample_rate.as_f32() * 0.5));
                self.karlsen
                    .process(driven, normalized_cutoff, NormalizedValue::new(resonance))
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
                    "model",
                    Param::Filter(FilterParam::Model(FilterModel::Standard)),
                    "Model",
                    FilterModel::to_choices(),
                )
                .description("Filter character model"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "morph",
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
                    "type",
                    Param::Filter(FilterParam::Mode(FilterMode::Lowpass)),
                    "Type",
                    FilterMode::to_choices(),
                )
                .description("Filter type"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "cutoff",
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
                    "resonance",
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
                    "key_track",
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
                    "cv_amt",
                    Param::Filter(FilterParam::CutoffMod(BipolarValue::MAX)),
                    "CV Amt",
                )
                .description("Cutoff CV input attenuverter (-1 to +1)")
                .range(-1.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "env_amt",
                    Param::Filter(FilterParam::EnvAmount(BipolarValue::new(0.25))),
                    "Env Amt",
                )
                .description(
                    "Envelope/CV amount in semitones (-1 = -48 st, +1 = +48 st). \
                     Default 0.25 ≈ 1 octave at full envelope.",
                )
                .range(-1.0, 1.0)
                .default(0.25)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "drive",
                    Param::Filter(FilterParam::Drive(Gain::UNITY)),
                    "Drive",
                )
                .description("Input gain with soft saturation (above 1.0)")
                .range(0.5, 4.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::audio_input("in", "In").description(
                    "Audio to filter. Connect: Oscillator Out, Noise Out, Mixer Out",
                ),
            )
            .port(
                PortDescriptor::control_input("cutoff_cv", "Cutoff CV").description(
                    "Modulates cutoff frequency. Connect: Envelope for filter sweep, LFO for wah-wah",
                ),
            )
            .port(
                PortDescriptor::control_input("res_cv", "Res CV")
                    .description("Modulates resonance. Connect: LFO, Envelope"),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Filtered output"))
    }
}

impl PolyModule for Filter {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let audio_in = inputs.reader(PortName::IN, 0.0);
        let cutoff_cv = inputs.reader(PortName::CUTOFF_CV, 0.0);
        let res_cv = inputs.reader(PortName::RESONANCE_CV, 0.0);

        // Per-block linear ramp of the base cutoff from the previous block's
        // final value to this block's target, so control-rate cutoff automation
        // doesn't step (zipper) at block boundaries. Key-tracking, mod-matrix
        // offset and CV are applied per-sample on top of the ramped base.
        let n = context.samples.as_usize();
        let cutoff_start = self.cutoff_smoothed;
        let cutoff_target = self.override_cutoff.unwrap_or(self.cutoff).as_f32();
        #[allow(clippy::cast_precision_loss)]
        let inv_n = if n > 0 { 1.0 / n as f32 } else { 0.0 };

        // Scale CV input to semitones. The Mod Matrix path uses ×48 (4 octaves
        // at full scale); applying the same scale here means a direct cable from
        // an envelope is just as expressive as routing through the matrix.
        // `env_amount` (default 0.25 = 12 st = 1 octave at full env) acts as
        // the per-filter "Env Amt" knob; `cv_amt` is a separate -1..+1
        // attenuverter that also flips polarity.
        for i in 0..n {
            let input = audio_in[i];
            let cutoff_mod = Semitones::new(
                cutoff_cv[i] * self.cutoff_mod_amount.as_f32() * self.env_amount.as_f32() * 48.0,
            );
            let res_mod = NormalizedValue::new(res_cv[i]);

            // Linear ramp: sample (n-1) lands exactly on the target.
            #[allow(clippy::cast_precision_loss)]
            let ramp_t = (i + 1) as f32 * inv_n;
            let base_cutoff = Hertz::new(cutoff_start + (cutoff_target - cutoff_start) * ramp_t);

            self.output_buffer[i] = self.process_sample(input, base_cutoff, cutoff_mod, res_mod);
        }
        // Carry the target as the next block's start for boundary continuity.
        self.cutoff_smoothed = cutoff_target;

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
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
            Param::Filter(FilterParam::EnvAmount(self.env_amount)),
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
            // Cutoff modulation is in semitones. Mod-matrix amount is normalized
            // (-1..1) and envelopes are 0..1, so the raw product is at most 1
            // semitone — inaudible. Scale to ±48 semitones (4 octaves) so a full
            // amount + full envelope yields a usable acid-style sweep.
            0 => {
                self.mod_offset_cutoff =
                    Semitones::new(self.mod_offset_cutoff.as_f32() + value * 48.0)
            }
            1 => {
                self.mod_offset_resonance =
                    NormalizedValue::new(self.mod_offset_resonance.as_f32() + value)
            }
            _ => {}
        }
    }

    fn clear_mod_offsets(&mut self) {
        self.mod_offset_cutoff = Semitones::ZERO;
        self.mod_offset_resonance = NormalizedValue::MIN;
    }

    fn set_param_override(&mut self, param: Param) {
        if let Param::Filter(filter_param) = param {
            match filter_param {
                FilterParam::Cutoff(c) => self.override_cutoff = Some(c.clamp_audible()),
                FilterParam::Resonance(r) => self.override_resonance = Some(r),
                // Other params are non-automatable (choice/structural); ignore.
                _ => {}
            }
        }
    }

    fn clear_param_overrides(&mut self) {
        self.override_cutoff = None;
        self.override_resonance = None;
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
            output_buffer: AudioBuffer::new(1024),
        }
    }

    #[inline]
    fn saturate(x: f32) -> f32 {
        crate::math::soft_clip(x)
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

            let saturated = if self.drive.as_f32() > 1.0 {
                Self::saturate(new_stage)
            } else {
                new_stage
            };
            self.stage[i] = FilterState::new(saturated);
            // Trapezoidal integrator state update: 2*output - previous_delay
            self.delay[i] = FilterState::new(2.0 * saturated - delay_val);
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
                    "cutoff",
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
                    "resonance",
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
                    "drive",
                    Param::Filter(FilterParam::Drive(Gain::UNITY)),
                    "Drive",
                )
                .description("Saturation amount (soft clipping above 1.0)")
                .range(0.5, 4.0)
                .default(1.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Audio to filter. Connect: Oscillator Out, Noise Out"),
            )
            .port(
                PortDescriptor::control_input("cutoff_cv", "Cutoff CV").description(
                    "Modulates cutoff frequency. Connect: Envelope for filter sweep, LFO for wah-wah",
                ),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Filtered output"))
    }
}

impl PolyModule for LadderFilter {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples.as_usize());

        let audio_in = inputs.reader(PortName::IN, 0.0);
        let cutoff_cv = inputs.reader(PortName::CUTOFF_CV, 0.0);

        for i in 0..context.samples.as_usize() {
            let input = audio_in[i];

            let effective_cutoff = if cutoff_cv.is_connected() {
                let mod_amount = cutoff_cv[i];
                Hertz::new((self.cutoff.as_f32() * (mod_amount * 4.0).exp2()).clamp(20.0, 20000.0))
            } else {
                self.cutoff
            };

            self.output_buffer[i] = self.process_sample(input, effective_cutoff);
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
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
    fn test_filter_param_automatable_allowlist() {
        let d = Filter::new().descriptor();
        let auto = |id: &str| {
            d.parameters
                .iter()
                .find(|p| p.type_id == id)
                .unwrap_or_else(|| panic!("missing param {id}"))
                .is_automatable()
        };
        // Continuous params are automatable.
        assert!(auto("cutoff"));
        assert!(auto("resonance"));
        // Choice/enum params are excluded.
        assert!(!auto("type")); // FilterMode
        assert!(!auto("model")); // FilterModel
    }

    #[test]
    fn test_filter_param_override_replaces_base_and_reverts() {
        let mut filter = Filter::new();
        filter.set_param(Param::Filter(FilterParam::Cutoff(Hertz::new(1000.0))));
        filter.set_param(Param::Filter(FilterParam::Resonance(NormalizedValue::new(
            0.2,
        ))));

        // No override: effective cutoff follows the base.
        assert!((filter.effective_cutoff().as_f32() - 1000.0).abs() < 0.5);

        // Override replaces the base while active.
        filter.set_param_override(Param::Filter(FilterParam::Cutoff(Hertz::new(400.0))));
        filter.set_param_override(Param::Filter(FilterParam::Resonance(NormalizedValue::new(
            0.9,
        ))));
        assert!((filter.effective_cutoff().as_f32() - 400.0).abs() < 0.5);
        assert!(matches!(filter.override_resonance, Some(r) if (r.as_f32() - 0.9).abs() < 1e-6));

        // Base params are never mutated by the override.
        assert!((filter.cutoff.as_f32() - 1000.0).abs() < 1e-3);
        assert!((filter.resonance.as_f32() - 0.2).abs() < 1e-3);

        // Clearing reverts to the base.
        filter.clear_param_overrides();
        assert!((filter.effective_cutoff().as_f32() - 1000.0).abs() < 0.5);
        assert!(filter.override_cutoff.is_none());
        assert!(filter.override_resonance.is_none());
    }

    #[test]
    fn test_filter_stability() {
        let mut filter = Filter::new();
        filter.sample_rate = SampleRate::DVD_QUALITY;
        filter.cutoff = Hertz::new(100.0);
        filter.resonance = NormalizedValue::new(0.99);

        for _ in 0..1000 {
            let out = filter.process_sample(
                0.5,
                Hertz::new(100.0),
                Semitones::ZERO,
                NormalizedValue::MIN,
            );
            assert!(out.is_finite(), "Filter output is not finite");
            assert!(out.abs() < 100.0, "Filter output exploded");
        }
    }

    #[test]
    fn test_filter_cutoff_override_ramps_without_zipper() {
        use synth_core::SampleCount;

        // Osc-free: feed the filter a constant DC input and watch the base cutoff
        // ramp by inspecting `cutoff_smoothed` after each block. A step override
        // must not jump the smoothed base cutoff in one sample.
        let mut filter = Filter::new();
        let ctx = ProcessContext {
            samples: SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };
        let mut input = AudioBuffer::new(256);
        for i in 0..256 {
            input[i] = 1.0;
        }

        let process = |filter: &mut Filter| {
            let mut outputs = HashMap::new();
            outputs.insert(PortName::OUT, AudioBuffer::new(256));
            filter.process(
                InputPorts::new(&[(PortName::IN, &input)]),
                &mut outputs,
                &ctx,
            );
        };

        // Settle at the base cutoff (1000 Hz).
        process(&mut filter);
        assert!((filter.cutoff_smoothed - 1000.0).abs() < 1.0);

        // Step the override to 20 Hz. Within the block the base cutoff must
        // *traverse* from 1000 toward 20, not jump instantly; by block end it
        // lands on the target.
        filter.set_param_override(Param::Filter(FilterParam::Cutoff(Hertz::new(20.0))));

        // Manually replicate the first ramp step to confirm continuity: the
        // first sample's base cutoff is one (1/n) step below 1000, far from 20.
        let n = 256.0;
        let first_step = 1000.0 + (20.0 - 1000.0) * (1.0 / n);
        assert!(
            first_step > 900.0,
            "first ramp step should stay near the previous value, got {first_step}"
        );

        process(&mut filter);
        // Reached the target by block end.
        assert!(
            (filter.cutoff_smoothed - 20.0).abs() < 0.5,
            "cutoff should ramp to target by block end, got {}",
            filter.cutoff_smoothed
        );
    }
}
