//! Amplifier (VCA) module.

use std::collections::HashMap;

use synth_core::{AmplifierParam, MixerParam, ModuleType, Param};
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{
    BipolarValue, ClipMode, Gain, LimitMode, MidiNote, MuteState, PortName, SampleRate, Velocity,
};
// PortName::MIXER_INPUTS provides static port names for zero-allocation processing

/// Voltage Controlled Amplifier.
#[derive(Clone)]
pub struct Amplifier {
    level: Gain,
    pan: BipolarValue,
    clip_mode: ClipMode,
    /// When true, CV input is bipolar (-1 to +1), allowing ring modulation
    cv_bipolar: bool,
    sample_rate: SampleRate,
    // Mod matrix offsets
    /// Level offset (multiplicative factor, from mod matrix).
    mod_offset_level: BipolarValue,
    /// Pan offset (additive, from mod matrix).
    mod_offset_pan: BipolarValue,
    // Transient automation overrides (replace the base value while active,
    // cleared on transport stop; the base param is never mutated).
    /// Level override from sequencer automation.
    override_level: Option<Gain>,
    /// Pan override from sequencer automation.
    override_pan: Option<BipolarValue>,
    /// Last block's final effective level, for per-block linear de-zippering of
    /// the (control-rate) effective level across block boundaries.
    level_prev: f32,
    output_left: AudioBuffer,
    output_right: AudioBuffer,
}

impl Amplifier {
    pub fn new() -> Self {
        Self {
            level: Gain::UNITY,
            pan: BipolarValue::CENTER,
            clip_mode: ClipMode::Off,
            cv_bipolar: false,
            sample_rate: SampleRate::DVD_QUALITY,
            mod_offset_level: BipolarValue::CENTER,
            mod_offset_pan: BipolarValue::CENTER,
            override_level: None,
            override_pan: None,
            level_prev: Gain::UNITY.as_f32(),
            output_left: AudioBuffer::new(1024),
            output_right: AudioBuffer::new(1024),
        }
    }

    #[inline]
    #[allow(dead_code)] // Useful helper for future stereo panning
    fn pan_coefficients(&self) -> (Gain, Gain) {
        Gain::from_pan(self.pan)
    }

    #[inline]
    fn apply_clip(x: f32, mode: ClipMode) -> f32 {
        match mode {
            ClipMode::Off => x,
            ClipMode::Soft => crate::math::soft_clip(x),
            ClipMode::Hard => crate::math::hard_clip(x),
        }
    }
}

impl Default for Amplifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Amplifier {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("amplifier", "VCA")
            .description("Voltage Controlled Amplifier")
            .category(ModuleCategory::Amplifier)
            .tag("amplifier")
            .tag("vca")
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::Amplifier(AmplifierParam::Level(Gain::UNITY)),
                    "Level",
                )
                .description("Output gain (0 = silent, 1 = unity, 2 = +6 dB)")
                .value_range(Gain::MIXER_RANGE)
                .widget(WidgetHint::Slider),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pan",
                    Param::Amplifier(AmplifierParam::Pan(BipolarValue::CENTER)),
                    "Pan",
                )
                .description("Stereo pan (-1 = left, 0 = center, +1 = right)")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::PanKnob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "cv_bipolar",
                    Param::Amplifier(AmplifierParam::CvBipolar(false)),
                    "CV Bipolar",
                )
                .description("Allow negative CV for ring modulation")
                .range(0.0, 1.0)
                .default(0.0)
                // Discrete mode toggle, not a continuous automation target.
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Mono input. Connect: Oscillator Out, Filter Out, another module"),
            )
            .port(
                PortDescriptor::audio_input("in_l", "In L")
                    .description("Left channel. Connect: Oscillator Out L"),
            )
            .port(
                PortDescriptor::audio_input("in_r", "In R")
                    .description("Right channel. Connect: Oscillator Out R"),
            )
            .port(
                PortDescriptor::control_input("cv", "CV").description(
                    "Controls volume. Connect: Envelope for dynamics, LFO for tremolo",
                ),
            )
            .port(
                PortDescriptor::control_input("pan_cv", "Pan CV")
                    .description("Controls panning. Connect: LFO for auto-pan, Envelope"),
            )
            .port(PortDescriptor::audio_output("left", "L").description("Left output"))
            .port(PortDescriptor::audio_output("right", "R").description("Right output"))
            .port(PortDescriptor::audio_output("out", "Out").description("Mono output (L+R)"))
    }
}

impl PolyModule for Amplifier {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_left.resize(context.samples.as_usize());
        self.output_right.resize(context.samples.as_usize());

        let audio_in = inputs.reader(PortName::IN, 0.0);
        let audio_in_l = inputs.reader(PortName::IN_L, 0.0);
        let audio_in_r = inputs.reader(PortName::IN_R, 0.0);
        let cv_in = inputs.reader(PortName::CV, 1.0);
        let pan_cv = inputs.reader(PortName::PAN_CV, 0.0);

        // Determine if we have stereo input
        let has_stereo_input = audio_in_l.is_connected() || audio_in_r.is_connected();

        // Effective base values: an automation override replaces the base while
        // active; the base param is left untouched.
        let level_target = self.override_level.unwrap_or(self.level).as_f32();
        let pan = self.override_pan.unwrap_or(self.pan);

        // Per-block linear ramp of the effective level from the previous block's
        // final value to this block's target, so control-rate level/volume
        // automation doesn't step (zipper) at block boundaries.
        let n = context.samples.as_usize();
        let level_start = self.level_prev;
        #[allow(clippy::cast_precision_loss)]
        let inv_n = if n > 0 { 1.0 / n as f32 } else { 0.0 };

        for i in 0..n {
            // Get input: use stereo inputs if connected, otherwise mono
            let (input_l, input_r) = if has_stereo_input {
                (audio_in_l[i], audio_in_r[i])
            } else {
                let mono = audio_in[i];
                (mono, mono)
            };

            let cv = cv_in.get(i);
            // In bipolar mode, CV can be negative (ring modulation)
            // In unipolar mode, CV is clamped to positive (standard VCA)
            let cv_scaled = if self.cv_bipolar { cv } else { cv.max(0.0) };
            // Linear ramp: sample (n-1) lands exactly on the target.
            #[allow(clippy::cast_precision_loss)]
            let ramp_t = (i + 1) as f32 * inv_n;
            let level_now = level_start + (level_target - level_start) * ramp_t;
            // Apply mod matrix level offset (additive to base level)
            let base_level = Gain::MIXER_RANGE.clamp(level_now + self.mod_offset_level.as_f32());
            let effective_level = base_level * cv_scaled;

            // BipolarValue::new() clamps internally to [-1, 1]
            let effective_pan = if pan_cv.is_connected() {
                BipolarValue::new(pan.as_f32() + pan_cv.get(i) + self.mod_offset_pan.as_f32())
            } else {
                BipolarValue::new((pan.as_f32() + self.mod_offset_pan.as_f32()).clamp(-1.0, 1.0))
            };

            let (pan_left, pan_right) = Gain::from_pan(effective_pan);

            let left = input_l * effective_level * pan_left.as_f32();
            let right = input_r * effective_level * pan_right.as_f32();

            self.output_left[i] = Self::apply_clip(left, self.clip_mode);
            self.output_right[i] = Self::apply_clip(right, self.clip_mode);
        }
        // Block ended exactly on the target (ramp_t == 1 at the last sample);
        // carry it as the start of the next block for boundary continuity.
        self.level_prev = level_target;

        if let Some(left) = outputs.get_mut(&PortName::LEFT) {
            left.copy_from(&self.output_left);
        }
        if let Some(right) = outputs.get_mut(&PortName::RIGHT) {
            right.copy_from(&self.output_right);
        }
        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            for i in 0..context.samples.as_usize() {
                out[i] = (self.output_left[i] + self.output_right[i]) * 0.5;
            }
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Amplifier(amp_param) = param {
            match amp_param {
                AmplifierParam::Level(l) => {
                    self.level = Gain::new(Gain::MIXER_RANGE.clamp(l.as_f32()));
                    // Seed the de-zipper anchor so a base change takes effect
                    // immediately rather than gliding from the previous anchor;
                    // an active automation override keeps driving the ramp.
                    if self.override_level.is_none() {
                        self.level_prev = self.level.as_f32();
                    }
                }
                AmplifierParam::Pan(p) => self.pan = p,
                AmplifierParam::CvBipolar(b) => self.cv_bipolar = b,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Amplifier(amp_param) = param {
            Some(match amp_param {
                AmplifierParam::Level(_) => self.level.as_f32(),
                AmplifierParam::Pan(_) => self.pan.as_f32(),
                AmplifierParam::CvBipolar(_) => {
                    if self.cv_bipolar {
                        1.0
                    } else {
                        0.0
                    }
                }
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Amplifier(AmplifierParam::Level(self.level)),
            Param::Amplifier(AmplifierParam::Pan(self.pan)),
            Param::Amplifier(AmplifierParam::CvBipolar(self.cv_bipolar)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Amplifier
    }

    fn reset(&mut self) {
        self.output_left.clear();
        self.output_right.clear();
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {}
    fn note_off(&mut self) {}

    fn set_mod_offset(&mut self, target: &str, value: f32) {
        match target {
            "level" => {
                self.mod_offset_level = BipolarValue::new(self.mod_offset_level.as_f32() + value)
            }
            "pan" => self.mod_offset_pan = BipolarValue::new(self.mod_offset_pan.as_f32() + value),
            _ => {}
        }
    }

    fn clear_mod_offsets(&mut self) {
        self.mod_offset_level = BipolarValue::CENTER;
        self.mod_offset_pan = BipolarValue::CENTER;
    }

    fn set_param_override(&mut self, param: Param) {
        if let Param::Amplifier(amp_param) = param {
            match amp_param {
                AmplifierParam::Level(l) => {
                    self.override_level = Some(Gain::new(Gain::MIXER_RANGE.clamp(l.as_f32())));
                }
                AmplifierParam::Pan(p) => self.override_pan = Some(p),
                // CvBipolar is a mode toggle, not an automation target.
                AmplifierParam::CvBipolar(_) => {}
            }
        }
    }

    fn clear_param_overrides(&mut self) {
        self.override_level = None;
        self.override_pan = None;
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

/// Simple mixer for combining multiple audio sources.
#[derive(Clone)]
pub struct Mixer {
    levels: [Gain; 8],
    master_level: Gain,
    mute_state: MuteState,
    limit_mode: LimitMode,
    /// Generic mod-matrix offsets (master + the 8 channel levels).
    mod_offsets: ParamModOffsets,
    output_buffer: AudioBuffer,
}

/// Descriptor `type_id`s for the 8 channel gains, as static strings so the
/// process loop can look up mod offsets without allocating (`format!`).
const MIXER_CHANNEL_IDS: [&str; 8] = [
    "input_1", "input_2", "input_3", "input_4", "input_5", "input_6", "input_7", "input_8",
];

impl Mixer {
    pub fn new() -> Self {
        Self {
            levels: [Gain::UNITY; 8],
            master_level: Gain::UNITY,
            mute_state: MuteState::Unmuted,
            limit_mode: LimitMode::Disabled,
            mod_offsets: ParamModOffsets::new(),
            output_buffer: AudioBuffer::new(1024),
        }
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Mixer {
    fn descriptor(&self) -> ModuleDescriptor {
        let mut desc = ModuleDescriptor::new("mixer", "Mixer")
            .description("8-channel mixer")
            .category(ModuleCategory::Mixer)
            .tag("mixer");

        // Master first (it's the master output level, not an input)
        desc = desc.parameter(
            ParameterDescriptor::float(
                "master",
                Param::Mixer(MixerParam::Master(Gain::UNITY)),
                "Master",
            )
            .description("Master output gain")
            .value_range(Gain::MIXER_RANGE)
            .widget(WidgetHint::Slider),
        );

        let input_params = [
            MixerParam::Input1(Gain::UNITY),
            MixerParam::Input2(Gain::UNITY),
            MixerParam::Input3(Gain::UNITY),
            MixerParam::Input4(Gain::UNITY),
            MixerParam::Input5(Gain::UNITY),
            MixerParam::Input6(Gain::UNITY),
            MixerParam::Input7(Gain::UNITY),
            MixerParam::Input8(Gain::UNITY),
        ];

        for (i, param) in input_params.into_iter().enumerate() {
            let n = i + 1;
            desc = desc
                .port(
                    PortDescriptor::audio_input(format!("in{n}"), format!("In {n}")).description(
                        "Audio input. Connect: Oscillator, Filter, or another sound source",
                    ),
                )
                .parameter(
                    ParameterDescriptor::float(
                        format!("input_{n}"),
                        Param::Mixer(param),
                        format!("Input {n}"),
                    )
                    .description(format!("Gain for input {n}"))
                    .value_range(Gain::MIXER_RANGE)
                    .widget(WidgetHint::Slider),
                );
        }

        desc = desc.port(PortDescriptor::audio_output("out", "Out").description("Mixad output"));

        desc
    }
}

impl PolyModule for Mixer {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.output_buffer.resize(context.samples.as_usize());
        self.output_buffer.clear();

        if self.mute_state.is_unmuted() {
            // Use static PortName constants - zero allocation in hot path
            for (idx, port_name) in PortName::MIXER_INPUTS.iter().enumerate() {
                if let Some(input) = inputs.get(*port_name) {
                    // Per-channel gain with its generic mod offset (per-block
                    // constant; the const id array keeps the lookup alloc-free).
                    let level = self
                        .mod_offsets
                        .effective(MIXER_CHANNEL_IDS[idx], self.levels[idx].as_f32());
                    for j in 0..context.samples.as_usize() {
                        self.output_buffer[j] += input[j] * level;
                    }
                }
            }
            let master = self
                .mod_offsets
                .effective("master", self.master_level.as_f32());
            self.output_buffer.scale(master);

            // Apply limiting if enabled
            if self.limit_mode.is_enabled() {
                for j in 0..context.samples.as_usize() {
                    // Soft limiting using tanh
                    self.output_buffer[j] = self.output_buffer[j].tanh();
                }
            }
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Mixer(mixer_param) = param {
            match mixer_param {
                MixerParam::Master(l) => {
                    self.master_level = Gain::new(Gain::MIXER_RANGE.clamp(l.as_f32()));
                }
                MixerParam::Input1(l) => self.levels[0] = l,
                MixerParam::Input2(l) => self.levels[1] = l,
                MixerParam::Input3(l) => self.levels[2] = l,
                MixerParam::Input4(l) => self.levels[3] = l,
                MixerParam::Input5(l) => self.levels[4] = l,
                MixerParam::Input6(l) => self.levels[5] = l,
                MixerParam::Input7(l) => self.levels[6] = l,
                MixerParam::Input8(l) => self.levels[7] = l,
                MixerParam::Mute(m) => self.mute_state = MuteState::from(m),
                MixerParam::Limit(l) => self.limit_mode = LimitMode::from(l),
                MixerParam::Dither(_) => {} // Dither not yet implemented in amplifier
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Mixer(mixer_param) = param {
            Some(match mixer_param {
                MixerParam::Master(_) => self.master_level.as_f32(),
                MixerParam::Input1(_) => self.levels[0].as_f32(),
                MixerParam::Input2(_) => self.levels[1].as_f32(),
                MixerParam::Input3(_) => self.levels[2].as_f32(),
                MixerParam::Input4(_) => self.levels[3].as_f32(),
                MixerParam::Input5(_) => self.levels[4].as_f32(),
                MixerParam::Input6(_) => self.levels[5].as_f32(),
                MixerParam::Input7(_) => self.levels[6].as_f32(),
                MixerParam::Input8(_) => self.levels[7].as_f32(),
                MixerParam::Mute(_) => {
                    if self.mute_state.is_muted() {
                        1.0
                    } else {
                        0.0
                    }
                }
                MixerParam::Limit(_) => {
                    if self.limit_mode.is_enabled() {
                        1.0
                    } else {
                        0.0
                    }
                }
                MixerParam::Dither(_) => 0.0, // Dither not yet implemented in amplifier
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Mixer(MixerParam::Master(self.master_level)),
            Param::Mixer(MixerParam::Input1(self.levels[0])),
            Param::Mixer(MixerParam::Input2(self.levels[1])),
            Param::Mixer(MixerParam::Input3(self.levels[2])),
            Param::Mixer(MixerParam::Input4(self.levels[3])),
            Param::Mixer(MixerParam::Input5(self.levels[4])),
            Param::Mixer(MixerParam::Input6(self.levels[5])),
            Param::Mixer(MixerParam::Input7(self.levels[6])),
            Param::Mixer(MixerParam::Input8(self.levels[7])),
            Param::Mixer(MixerParam::Mute(self.mute_state.is_muted())),
            Param::Mixer(MixerParam::Limit(self.limit_mode.is_enabled())),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Mixer
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.output_buffer.clear();
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
    fn test_amplifier_creation() {
        let amp = Amplifier::new();
        assert!((amp.level.as_f32() - 1.0).abs() < 0.001);
    }

    /// Drift guard: the amplifier `level`, mixer `master`, and mixer `input_1`
    /// descriptor ranges all share the single `Gain::MIXER_RANGE` preset.
    #[test]
    fn level_descriptors_match_mixer_range() {
        let range_of = |d: &ModuleDescriptor, id: &str| {
            d.parameters
                .iter()
                .find(|p| p.type_id == id)
                .unwrap_or_else(|| panic!("missing param {id}"))
                .range
        };
        let amp = Amplifier::new().descriptor();
        assert_eq!(range_of(&amp, "level"), Gain::MIXER_RANGE);
        let mix = Mixer::new().descriptor();
        assert_eq!(range_of(&mix, "master"), Gain::MIXER_RANGE);
        assert_eq!(range_of(&mix, "input_1"), Gain::MIXER_RANGE);
    }

    /// The mixer's `master` gain used to be dropped (Mixer never overrode
    /// set_mod_offset); it now scales the mixed output via the generic store.
    #[test]
    fn mixer_master_mod_offset_scales_output() {
        let mut mixer = Mixer::new();
        let desc = mixer.descriptor();
        mixer.mod_offsets_mut().unwrap().populate(&desc);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            ..ProcessContext::default()
        };
        fn peak(mixer: &mut Mixer, ctx: &ProcessContext) -> f32 {
            let mut buf = AudioBuffer::new(64);
            for i in 0..64 {
                buf[i] = 0.5;
            }
            let in_ports = [(PortName::MIXER_INPUTS[0], &buf)];
            let inputs = InputPorts::new(&in_ports);
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(64));
            mixer.process(inputs, &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i].abs()).fold(0.0_f32, f32::max)
        }

        let base = peak(&mut mixer, &ctx);
        assert!(
            (base - 0.5).abs() < 1e-4,
            "unity master passes input, got {base}"
        );

        // master range 0..2, base 1.0 → normalize 0.5; -0.25 offset → 0.25 norm
        // → 0.5 gain → half output.
        mixer.set_mod_offset("master", -0.25);
        let quieter = peak(&mut mixer, &ctx);
        assert!(
            quieter < base * 0.75,
            "master offset should reduce output: {quieter} vs {base}"
        );

        mixer.clear_mod_offsets();
        assert!((peak(&mut mixer, &ctx) - base).abs() < 1e-4);
    }

    #[test]
    fn test_constant_power_pan() {
        let mut amp = Amplifier::new();
        amp.pan = BipolarValue::CENTER;
        let (l, r) = amp.pan_coefficients();
        assert!((l.as_f32() - r.as_f32()).abs() < 0.01);
    }

    #[test]
    fn test_amplifier_level_override_replaces_base_and_reverts() {
        let mut amp = Amplifier::new();
        amp.set_param(Param::Amplifier(AmplifierParam::Level(Gain::new(0.5))));

        let mut input = AudioBuffer::new(1024);
        for i in 0..64 {
            input[i] = 1.0;
        }
        let context = ProcessContext {
            samples: synth_core::SampleCount::new(64),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };

        // Read the LAST sample: the effective level is now de-zippered with a
        // per-block linear ramp, so it lands on the target by the block end.
        let run = |amp: &mut Amplifier, input: &AudioBuffer| -> f32 {
            let mut outputs = HashMap::new();
            outputs.insert(PortName::OUT, AudioBuffer::new(1024));
            amp.process(
                InputPorts::new(&[(PortName::IN, input)]),
                &mut outputs,
                &context,
            );
            outputs[&PortName::OUT][63]
        };

        let base_out = run(&mut amp, &input);
        assert!(base_out > 0.0);

        // Override tripling the level (0.5 -> 1.5) roughly triples the output.
        amp.set_param_override(Param::Amplifier(AmplifierParam::Level(Gain::new(1.5))));
        let override_out = run(&mut amp, &input);
        assert!((override_out / base_out - 3.0).abs() < 1e-3);

        // Base param is never mutated by the override.
        assert!((amp.level.as_f32() - 0.5).abs() < 1e-6);

        // Clearing reverts to the base.
        amp.clear_param_overrides();
        let reverted_out = run(&mut amp, &input);
        assert!((reverted_out - base_out).abs() < 1e-4);
        assert!(amp.override_level.is_none());
        assert!(amp.override_pan.is_none());
    }

    #[test]
    fn test_amplifier_param_automatable_allowlist() {
        let d = Amplifier::new().descriptor();
        let auto = |id: &str| {
            d.parameters
                .iter()
                .find(|p| p.type_id == id)
                .unwrap_or_else(|| panic!("missing param {id}"))
                .is_automatable()
        };
        assert!(auto("level"));
        assert!(auto("pan"));
        // Discrete mode toggle — excluded from the automation allowlist.
        assert!(!auto("cv_bipolar"));
    }

    #[test]
    fn test_amplifier_setparam_seeds_dezipper_anchor() {
        // A base level set via set_param must seed the ramp anchor so the first
        // block starts at the base, not gliding from the unity init.
        let mut amp = Amplifier::new();
        amp.set_param(Param::Amplifier(AmplifierParam::Level(Gain::new(0.3))));
        assert!((amp.level_prev - 0.3).abs() < 1e-6);
    }

    #[test]
    fn test_amplifier_level_override_ramps_without_zipper() {
        let mut amp = Amplifier::new();
        // Constant DC input so the output directly tracks the effective level.
        let mut input = AudioBuffer::new(1024);
        for i in 0..256 {
            input[i] = 1.0;
        }
        let context = ProcessContext {
            samples: synth_core::SampleCount::new(256),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };
        let run = |amp: &mut Amplifier, input: &AudioBuffer| -> Vec<f32> {
            let mut outputs = HashMap::new();
            outputs.insert(PortName::OUT, AudioBuffer::new(1024));
            amp.process(
                InputPorts::new(&[(PortName::IN, input)]),
                &mut outputs,
                &context,
            );
            (0..256).map(|i| outputs[&PortName::OUT][i]).collect()
        };

        // Block A at base level (unity): settled output.
        let a = run(&mut amp, &input);
        let last_a = a[255];
        assert!(last_a > 0.1);

        // Override to silence: block B must RAMP from last_a down, not step.
        amp.set_param_override(Param::Amplifier(AmplifierParam::Level(Gain::new(0.0))));
        let b = run(&mut amp, &input);

        // Boundary continuity: B's first sample is one ramp step from A's last,
        // not a hard jump to zero (which is what zipper noise would be).
        assert!(
            (b[0] - last_a).abs() < last_a * 0.05,
            "expected smooth ramp across block boundary: last_a={last_a} first_b={}",
            b[0]
        );
        // The ramp reaches the target (silence) by the end of the block.
        assert!(
            b[255] < 1e-6,
            "should reach target by block end, got {}",
            b[255]
        );
        // Monotonic descent — no per-sample spikes.
        for w in b.windows(2) {
            assert!(w[1] <= w[0] + 1e-6, "level ramp must be monotonic down");
        }
    }
}
