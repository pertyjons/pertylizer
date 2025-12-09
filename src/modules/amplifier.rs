//! Amplifier (VCA) module.

use std::collections::HashMap;

use crate::engine::typed_params::{AmplifierParam, MixerParam, ModuleType, Param};
use crate::modules::core::*;
use crate::types::{BipolarValue, ClipMode, Gain, MidiNote, SampleRate};

/// Voltage Controlled Amplifier.
#[derive(Clone)]
pub struct Amplifier {
    level: Gain,
    pan: BipolarValue,
    clip_mode: ClipMode,
    sample_rate: SampleRate,
    output_left: AudioBuffer,
    output_right: AudioBuffer,
}

impl Amplifier {
    pub fn new() -> Self {
        Self {
            level: Gain::UNITY,
            pan: BipolarValue::CENTER,
            clip_mode: ClipMode::Off,
            sample_rate: SampleRate::DVD_QUALITY,
            output_left: AudioBuffer::new(256),
            output_right: AudioBuffer::new(256),
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
            .port(PortDescriptor::audio_input("in", "In"))
            .port(PortDescriptor::control_input("cv", "CV"))
            .port(PortDescriptor::control_input("pan_cv", "Pan CV"))
            .port(PortDescriptor::audio_output("left", "L"))
            .port(PortDescriptor::audio_output("right", "R"))
            .port(PortDescriptor::audio_output("out", "Out"))
    }
}

impl PolyModule for Amplifier {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_left.resize(context.samples);
        self.output_right.resize(context.samples);

        let audio_in = inputs.get("in");
        let cv_in = inputs.get("cv");
        let pan_cv = inputs.get("pan_cv");

        for i in 0..context.samples {
            let input = audio_in.map(|b| b[i]).unwrap_or(0.0);
            let cv = cv_in.map(|b| b[i]).unwrap_or(1.0);
            let effective_level = self.level * cv.max(0.0);

            let effective_pan = if let Some(pan_mod) = pan_cv {
                BipolarValue::new(self.pan.as_f32() + pan_mod[i])
            } else {
                self.pan
            };

            let (pan_left, pan_right) = Gain::from_pan(effective_pan);

            let left = input * effective_level.as_f32() * pan_left.as_f32();
            let right = input * effective_level.as_f32() * pan_right.as_f32();

            self.output_left[i] = Self::apply_clip(left, self.clip_mode);
            self.output_right[i] = Self::apply_clip(right, self.clip_mode);
        }

        if let Some(left) = outputs.get_mut("left") {
            left.copy_from(&self.output_left);
        }
        if let Some(right) = outputs.get_mut("right") {
            right.copy_from(&self.output_right);
        }
        if let Some(out) = outputs.get_mut("out") {
            for i in 0..context.samples {
                out[i] = (self.output_left[i] + self.output_right[i]) * 0.5;
            }
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Amplifier(amp_param) = param {
            match amp_param {
                AmplifierParam::Level(l) => self.level = Gain::new(l.as_f32().clamp(0.0, 2.0)),
                AmplifierParam::Pan(p) => self.pan = p,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Amplifier(amp_param) = param {
            Some(match amp_param {
                AmplifierParam::Level(_) => self.level.as_f32(),
                AmplifierParam::Pan(_) => self.pan.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Amplifier(AmplifierParam::Level(self.level)),
            Param::Amplifier(AmplifierParam::Pan(self.pan)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Amplifier
    }

    fn reset(&mut self) {
        self.output_left.clear();
        self.output_right.clear();
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: f32) {}
    fn note_off(&mut self) {}

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

/// Simple mixer for combining multiple audio sources.
#[derive(Clone)]
pub struct Mixer {
    levels: [Gain; 8],
    master_level: Gain,
    mute: bool,
    limit: bool,
    output_buffer: AudioBuffer,
}

impl Mixer {
    pub fn new() -> Self {
        Self {
            levels: [Gain::UNITY; 8],
            master_level: Gain::UNITY,
            mute: false,
            limit: false,
            output_buffer: AudioBuffer::new(256),
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

        for i in 1..=8 {
            desc = desc.port(PortDescriptor::audio_input(
                format!("in{i}"),
                format!("In {i}"),
            ));
        }

        desc = desc
            .parameter(
                ParameterDescriptor::float(Param::Mixer(MixerParam::Master(Gain::UNITY)), "Master")
                    .range(0.0, 2.0)
                    .default(1.0)
                    .widget(WidgetHint::Slider),
            )
            .port(PortDescriptor::audio_output("out", "Out"));

        desc
    }
}

impl PolyModule for Mixer {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.output_buffer.resize(context.samples);
        self.output_buffer.clear();

        if !self.mute {
            for i in 1..=8 {
                let key = format!("in{i}");
                if let Some(input) = inputs.get(&key) {
                    let level = self.levels[i - 1].as_f32();
                    for j in 0..context.samples {
                        self.output_buffer[j] += input[j] * level;
                    }
                }
            }
            self.output_buffer.scale(self.master_level.as_f32());
        }

        if let Some(out) = outputs.get_mut("out") {
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
                MixerParam::Mute(m) => self.mute = m,
                MixerParam::Limit(l) => self.limit = l,
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
                MixerParam::Mute(_) => {
                    if self.mute {
                        1.0
                    } else {
                        0.0
                    }
                }
                MixerParam::Limit(_) => {
                    if self.limit {
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
            Param::Mixer(MixerParam::Mute(self.mute)),
            Param::Mixer(MixerParam::Limit(self.limit)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Mixer
    }

    fn reset(&mut self) {
        self.output_buffer.clear();
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: f32) {}
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
