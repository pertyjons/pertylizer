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
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParamModOffsets,
    ParameterDescriptor, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
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
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

    // Buffers
    output_buffer: AudioBuffer,
}

impl VectorMixer {
    pub fn new() -> Self {
        Self {
            x: BipolarValue::CENTER,
            y: BipolarValue::CENTER,
            sample_rate: SampleRate::DVD_QUALITY,
            mod_offsets: ParamModOffsets::new(),
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
        let xn = crate::math::bipolar_to_unipolar(self.x.as_f32());
        let yn = crate::math::bipolar_to_unipolar(self.y.as_f32());
        crate::math::bilinear_equal_power_gains(xn, yn)
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
            .width(synth_core::ModuleWidth::Small)
            .description("4-corner XY vector crossfader for vector synthesis")
            .category(ModuleCategory::Utility)
            .tag("vector")
            .tag("mixer")
            .tag("crossfade")
            .port(
                PortDescriptor::audio_input("in_a", "In A")
                    .description("Top-left corner input. Connect: Oscillator Out"),
            )
            .port(
                PortDescriptor::audio_input("in_b", "In B")
                    .description("Top-right corner input. Connect: Oscillator Out"),
            )
            .port(
                PortDescriptor::audio_input("in_c", "In C")
                    .description("Bottom-left corner input. Connect: Oscillator Out"),
            )
            .port(
                PortDescriptor::audio_input("in_d", "In D")
                    .description("Bottom-right corner input. Connect: Oscillator Out"),
            )
            .port(
                PortDescriptor::control_input("x_cv", "X CV")
                    .description("Modulates X position. Connect: LFO, Envelope"),
            )
            .port(
                PortDescriptor::control_input("y_cv", "Y CV")
                    .description("Modulates Y position. Connect: LFO, Envelope"),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Mixed output. Connect to: Filter In, Amplifier In"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "x",
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
                    "y",
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
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        let num_samples = context.samples.as_usize();
        self.output_buffer.resize(num_samples);

        let in_a = inputs.get(PortName::IN_A);
        let in_b = inputs.get(PortName::IN_B);
        let in_c = inputs.get(PortName::IN_C);
        let in_d = inputs.get(PortName::IN_D);
        let x_cv = inputs.get(PortName::X_CV);
        let y_cv = inputs.get(PortName::Y_CV);

        // Generic mod offsets on the XY base (per-block); the CV ports add on top.
        let x_base = self.mod_offsets.effective("x", self.x.as_f32());
        let y_base = self.mod_offsets.effective("y", self.y.as_f32());

        for i in 0..num_samples {
            // Apply CV modulation to XY
            let mut x_val = x_base;
            let mut y_val = y_base;

            if let Some(cv) = x_cv {
                x_val = (x_val + crate::math::sanitize_cv(cv[i])).clamp(-1.0, 1.0);
            }
            if let Some(cv) = y_cv {
                y_val = (y_val + crate::math::sanitize_cv(cv[i])).clamp(-1.0, 1.0);
            }

            let xn = crate::math::bipolar_to_unipolar(x_val);
            let yn = crate::math::bipolar_to_unipolar(y_val);
            let (w_a, w_b, w_c, w_d) = crate::math::bilinear_equal_power_gains(xn, yn);

            let sa = in_a.map_or(0.0, |buf| buf[i]);
            let sb = in_b.map_or(0.0, |buf| buf[i]);
            let sc = in_c.map_or(0.0, |buf| buf[i]);
            let sd = in_d.map_or(0.0, |buf| buf[i]);

            self.output_buffer[i] = sa * w_a + sb * w_b + sc * w_c + sd * w_d;
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
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

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
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

    #[test]
    fn test_vector_mixer_creation() {
        let vm = VectorMixer::new();
        assert_eq!(vm.x.as_f32(), 0.0);
        assert_eq!(vm.y.as_f32(), 0.0);
    }

    /// `x` used to be dropped (VectorMixer never overrode set_mod_offset); it now
    /// shifts the XY blend toward the right corners via the generic store.
    #[test]
    fn x_mod_offset_shifts_blend() {
        let mut vm = VectorMixer::new();
        let desc = vm.descriptor();
        vm.mod_offsets_mut().unwrap().populate(&desc);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(8),
            ..ProcessContext::default()
        };
        // Only corner B (top-right) gets signal, so output rises as X → +1.
        fn out_b(vm: &mut VectorMixer, ctx: &ProcessContext) -> f32 {
            let mut buf = AudioBuffer::new(8);
            for i in 0..8 {
                buf[i] = 1.0;
            }
            let in_ports = [(PortName::IN_B, &buf)];
            let inputs = InputPorts::new(&in_ports);
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(8));
            vm.process(inputs, &mut outs, ctx);
            outs[&PortName::OUT][0]
        }

        let base = out_b(&mut vm, &ctx); // X=0 centered
        vm.set_mod_offset("x", 0.5); // pan toward right corners (B/D)
        let shifted = out_b(&mut vm, &ctx);
        assert!(
            shifted > base + 1e-3,
            "x offset should raise corner-B weight: {shifted} vs {base}"
        );

        vm.clear_mod_offsets();
        assert!((out_b(&mut vm, &ctx) - base).abs() < 1e-4);
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
