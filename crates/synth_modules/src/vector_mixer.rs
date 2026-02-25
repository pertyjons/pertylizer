//! Vector Mixer — 4-corner XY vector synthesis crossfader.
//!
//! Takes 4 audio inputs (A, B, C, D) arranged in a square:
//!   A --- B
//!   |     |
//!   C --- D
//!
//! An XY joystick position blends between the four corners using
//! equal-power bilinear interpolation.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{BipolarValue, MidiNote, PortName, SampleRate, Velocity};
use synth_core::{ModuleType, Param, VectorMixerParam};

/// Vector mixer with 4 audio inputs and XY crossfade.
#[derive(Clone)]
pub struct VectorMixer {
    // Parameters
    x: BipolarValue,
    y: BipolarValue,

    // State
    sample_rate: SampleRate,

    // Buffers
    output_buffer: AudioBuffer,
}

impl VectorMixer {
    pub fn new() -> Self {
        Self {
            x: BipolarValue::CENTER,
            y: BipolarValue::CENTER,
            sample_rate: SampleRate::DVD_QUALITY,
            output_buffer: AudioBuffer::new(1024),
        }
    }

    /// Compute equal-power gains for the 4 corners from XY position.
    ///
    /// X: -1=left, +1=right
    /// Y: -1=bottom, +1=top
    ///
    /// Returns (gain_a, gain_b, gain_c, gain_d)
    #[cfg(test)]
    fn compute_gains(&self) -> (f32, f32, f32, f32) {
        // Map from [-1,1] to [0,1]
        let xn = (self.x.as_f32() + 1.0) * 0.5;
        let yn = (self.y.as_f32() + 1.0) * 0.5;

        // Bilinear interpolation weights
        let w_a = (1.0 - xn) * yn; // top-left
        let w_b = xn * yn; // top-right
        let w_c = (1.0 - xn) * (1.0 - yn); // bottom-left
        let w_d = xn * (1.0 - yn); // bottom-right

        // Equal-power: sqrt of linear weights
        (w_a.sqrt(), w_b.sqrt(), w_c.sqrt(), w_d.sqrt())
    }
}

impl Default for VectorMixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for VectorMixer {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("vector_mixer", "Vector Mixer")
            .description("4-corner XY vector crossfader for vector synthesis")
            .category(ModuleCategory::Utility)
            .tag("vector")
            .tag("mixer")
            .tag("crossfade")
            .port(
                PortDescriptor::audio_input("in_a", "In A")
                    .description("Top-left corner input. Koppla: Oscillator Out"),
            )
            .port(
                PortDescriptor::audio_input("in_b", "In B")
                    .description("Top-right corner input. Koppla: Oscillator Out"),
            )
            .port(
                PortDescriptor::audio_input("in_c", "In C")
                    .description("Bottom-left corner input. Koppla: Oscillator Out"),
            )
            .port(
                PortDescriptor::audio_input("in_d", "In D")
                    .description("Bottom-right corner input. Koppla: Oscillator Out"),
            )
            .port(
                PortDescriptor::control_input("x_cv", "X CV")
                    .description("Modulerar X-position. Koppla: LFO, Envelope"),
            )
            .port(
                PortDescriptor::control_input("y_cv", "Y CV")
                    .description("Modulerar Y-position. Koppla: LFO, Envelope"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Mixed output. Koppla till: Filter In, Amplifier In"),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::VectorMixer(VectorMixerParam::X(BipolarValue::CENTER)),
                    "X",
                )
                .description("X position (-1=left, +1=right)")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::VectorMixer(VectorMixerParam::Y(BipolarValue::CENTER)),
                    "Y",
                )
                .description("Y position (-1=bottom, +1=top)")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl PolyModule for VectorMixer {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        let in_a = inputs.get(PortName::intern("in_a"));
        let in_b = inputs.get(PortName::intern("in_b"));
        let in_c = inputs.get(PortName::intern("in_c"));
        let in_d = inputs.get(PortName::intern("in_d"));
        let x_cv = inputs.get(PortName::intern("x_cv"));
        let y_cv = inputs.get(PortName::intern("y_cv"));

        for i in 0..num_samples {
            // Apply CV modulation to XY
            let mut x_val = self.x.as_f32();
            let mut y_val = self.y.as_f32();

            if let Some(cv) = x_cv {
                x_val = (x_val + cv[i]).clamp(-1.0, 1.0);
            }
            if let Some(cv) = y_cv {
                y_val = (y_val + cv[i]).clamp(-1.0, 1.0);
            }

            // Map to [0,1]
            let xn = (x_val + 1.0) * 0.5;
            let yn = (y_val + 1.0) * 0.5;

            // Bilinear weights with equal-power
            let w_a = ((1.0 - xn) * yn).sqrt();
            let w_b = (xn * yn).sqrt();
            let w_c = ((1.0 - xn) * (1.0 - yn)).sqrt();
            let w_d = (xn * (1.0 - yn)).sqrt();

            let sa = in_a.map_or(0.0, |buf| buf[i]);
            let sb = in_b.map_or(0.0, |buf| buf[i]);
            let sc = in_c.map_or(0.0, |buf| buf[i]);
            let sd = in_d.map_or(0.0, |buf| buf[i]);

            self.output_buffer[i] = sa * w_a + sb * w_b + sc * w_c + sd * w_d;
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::VectorMixer(p) = param {
            match p {
                VectorMixerParam::X(v) => self.x = v,
                VectorMixerParam::Y(v) => self.y = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::VectorMixer(p) = param {
            Some(match p {
                VectorMixerParam::X(_) => self.x.as_f32(),
                VectorMixerParam::Y(_) => self.y.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::VectorMixer(VectorMixerParam::X(self.x)),
            Param::VectorMixer(VectorMixerParam::Y(self.y)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::VectorMixer
    }

    fn reset(&mut self) {
        // No internal state to reset beyond parameters
    }

    fn note_on(&mut self, _note: MidiNote, _velocity: Velocity) {
        // Stateless mixer — nothing to do on note
    }

    fn note_off(&mut self) {
        // Nothing to do
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;

    #[test]
    fn test_vector_mixer_creation() {
        let vm = VectorMixer::new();
        assert_eq!(vm.x.as_f32(), 0.0);
        assert_eq!(vm.y.as_f32(), 0.0);
    }

    #[test]
    fn test_equal_mix_at_center() {
        let vm = VectorMixer::new();
        let (ga, gb, gc, gd) = vm.compute_gains();
        // At center, all gains should be equal
        assert!((ga - gb).abs() < 0.001);
        assert!((ga - gc).abs() < 0.001);
        assert!((ga - gd).abs() < 0.001);
    }

    #[test]
    fn test_corner_isolation() {
        let mut vm = VectorMixer::new();

        // Top-left corner: A should dominate
        vm.x = BipolarValue::new(-1.0);
        vm.y = BipolarValue::new(1.0);
        let (ga, gb, gc, gd) = vm.compute_gains();
        assert!(ga > 0.9);
        assert!(gb < 0.01);
        assert!(gc < 0.01);
        assert!(gd < 0.01);
    }

    #[test]
    fn test_vector_mixer_params() {
        let mut vm = VectorMixer::new();
        vm.set_param(Param::VectorMixer(VectorMixerParam::X(BipolarValue::new(
            0.5,
        ))));
        assert!((vm.x.as_f32() - 0.5).abs() < 0.001);

        let params = vm.get_params();
        assert_eq!(params.len(), 2);
    }
}
