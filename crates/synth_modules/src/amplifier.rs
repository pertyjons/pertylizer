//! Amplifier (VCA) module.

use std::collections::HashMap;

use synth_core::{AmplifierParam, MixerParam, ModuleType, Param};
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    PolyModule, PortDescriptor, ProcessContext, WidgetHint,
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
            ClipMode::Soft => x.tanh(),
            ClipMode::Hard => x.clamp(-1.0, 1.0),
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
                    Param::Amplifier(AmplifierParam::Level(Gain::UNITY)),
                    "Level",
                )
                .range(0.0, 2.0)
                .default(1.0)
                .widget(WidgetHint::Slider),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Amplifier(AmplifierParam::Pan(BipolarValue::CENTER)),
                    "Pan",
                )
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::PanKnob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Amplifier(AmplifierParam::CvBipolar(false)),
                    "CV Bipolar",
                )
                .description("Allow negative CV for ring modulation")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Toggle),
            )
            .port(
                PortDescriptor::audio_input("in", "In")
                    .description("Mono-ingång. Koppla: Oscillator Out, Filter Out, annan modul"),
            )
            .port(
                PortDescriptor::audio_input("in_l", "In L")
                    .description("Vänster kanal. Koppla: Oscillator Out L"),
            )
            .port(
                PortDescriptor::audio_input("in_r", "In R")
                    .description("Höger kanal. Koppla: Oscillator Out R"),
            )
            .port(
                PortDescriptor::control_input("cv", "CV")
                    .description("Styr volymen. Koppla: Envelope för dynamik, LFO för tremolo"),
            )
            .port(
                PortDescriptor::control_input("pan_cv", "Pan CV")
                    .description("Styr panorering. Koppla: LFO för auto-pan, Envelope"),
            )
            .port(PortDescriptor::audio_output("left", "L").description("Vänster output"))
            .port(PortDescriptor::audio_output("right", "R").description("Höger output"))
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

        let audio_in = inputs.get(PortName::IN);
        let audio_in_l = inputs.get(PortName::IN_L);
        let audio_in_r = inputs.get(PortName::IN_R);
        let cv_in = inputs.get(PortName::CV);
        let pan_cv = inputs.get(PortName::PAN_CV);

        // Determine if we have stereo input
        let has_stereo_input = audio_in_l.is_some() || audio_in_r.is_some();

        for i in 0..context.samples.as_usize() {
            // Get input: use stereo inputs if connected, otherwise mono
            let (input_l, input_r) = if has_stereo_input {
                (
                    audio_in_l.map(|b| b[i]).unwrap_or(0.0),
                    audio_in_r.map(|b| b[i]).unwrap_or(0.0),
                )
            } else {
                let mono = audio_in.map(|b| b[i]).unwrap_or(0.0);
                (mono, mono)
            };

            let cv = cv_in.map(|b| b[i]).unwrap_or(1.0);
            // In bipolar mode, CV can be negative (ring modulation)
            // In unipolar mode, CV is clamped to positive (standard VCA)
            let cv_scaled = if self.cv_bipolar { cv } else { cv.max(0.0) };
            // Apply mod matrix level offset (additive to base level)
            let base_level = (self.level.as_f32() + self.mod_offset_level.as_f32()).clamp(0.0, 2.0);
            let effective_level = base_level * cv_scaled;

            let effective_pan = if let Some(pan_mod) = pan_cv {
                BipolarValue::new(self.pan.as_f32() + pan_mod[i] + self.mod_offset_pan.as_f32())
            } else {
                BipolarValue::new(
                    (self.pan.as_f32() + self.mod_offset_pan.as_f32()).clamp(-1.0, 1.0),
                )
            };

            let (pan_left, pan_right) = Gain::from_pan(effective_pan);

            let left = input_l * effective_level * pan_left.as_f32();
            let right = input_r * effective_level * pan_right.as_f32();

            self.output_left[i] = Self::apply_clip(left, self.clip_mode);
            self.output_right[i] = Self::apply_clip(right, self.clip_mode);
        }

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
                AmplifierParam::Level(l) => self.level = Gain::new(l.as_f32().clamp(0.0, 2.0)),
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

    fn set_mod_offset(&mut self, dest_index: u8, value: f32) {
        match dest_index {
            0 => self.mod_offset_level = BipolarValue::new(self.mod_offset_level.as_f32() + value),
            1 => self.mod_offset_pan = BipolarValue::new(self.mod_offset_pan.as_f32() + value),
            _ => {}
        }
    }

    fn clear_mod_offsets(&mut self) {
        self.mod_offset_level = BipolarValue::CENTER;
        self.mod_offset_pan = BipolarValue::CENTER;
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
    output_buffer: AudioBuffer,
}

impl Mixer {
    pub fn new() -> Self {
        Self {
            levels: [Gain::UNITY; 8],
            master_level: Gain::UNITY,
            mute_state: MuteState::Unmuted,
            limit_mode: LimitMode::Disabled,
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
            ParameterDescriptor::float(Param::Mixer(MixerParam::Master(Gain::UNITY)), "Master")
                .range(0.0, 2.0)
                .default(1.0)
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
                        "Audio-ingång. Koppla: Oscillator, Filter, eller annan ljudkälla",
                    ),
                )
                .parameter(
                    ParameterDescriptor::float(Param::Mixer(param), format!("Input {n}"))
                        .range(0.0, 2.0)
                        .default(1.0)
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
                    let level = self.levels[idx].as_f32();
                    for j in 0..context.samples.as_usize() {
                        self.output_buffer[j] += input[j] * level;
                    }
                }
            }
            self.output_buffer.scale(self.master_level.as_f32());

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
                MixerParam::Master(l) => self.master_level = Gain::new(l.as_f32().clamp(0.0, 2.0)),
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

    #[test]
    fn test_constant_power_pan() {
        let mut amp = Amplifier::new();
        amp.pan = BipolarValue::CENTER;
        let (l, r) = amp.pan_coefficients();
        assert!((l.as_f32() - r.as_f32()).abs() < 0.01);
    }
}
