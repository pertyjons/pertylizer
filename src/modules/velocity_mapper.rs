//! Velocity Mapper module for velocity curve shaping.
//!
//! Transforms MIDI velocity values through various curves to
//! customize the feel and response of the instrument.

use std::collections::HashMap;

use crate::engine::typed_params::{ModuleType, Param, VelocityCurve, VelocityMapperParam};
use crate::modules::core::*;
use crate::types::{MidiNote, NormalizedValue, SampleRate};

/// Velocity curve shaping module.
#[derive(Clone)]
pub struct VelocityMapper {
    // Parameters
    curve: VelocityCurve,
    min_out: NormalizedValue,
    max_out: NormalizedValue,
    curve_amount: f32,

    // Current mapped velocity
    current_velocity: f32,

    // Sample rate
    sample_rate: SampleRate,

    // Output buffer (outputs mapped velocity as control signal)
    output_buffer: AudioBuffer,
}

impl VelocityMapper {
    pub fn new() -> Self {
        Self {
            curve: VelocityCurve::Linear,
            min_out: NormalizedValue::new(0.0),
            max_out: NormalizedValue::new(1.0),
            curve_amount: 0.0,

            current_velocity: 0.0,

            sample_rate: SampleRate::DVD_QUALITY,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Map input velocity through the selected curve.
    fn map_velocity(&self, input: f32) -> f32 {
        let v = input.clamp(0.0, 1.0);

        // Apply curve type
        let curved = match self.curve {
            VelocityCurve::Linear => v,
            VelocityCurve::Soft => {
                // Square root curve - more response at low velocities
                v.sqrt()
            }
            VelocityCurve::Hard => {
                // Square curve - more response at high velocities
                v * v
            }
            VelocityCurve::SCurve => {
                // Smooth S-curve (smoothstep)
                v * v * (3.0 - 2.0 * v)
            }
            VelocityCurve::Fixed => 1.0, // Always output max
        };

        // Apply additional curve amount adjustment
        let adjusted = if self.curve_amount.abs() > 0.01 {
            if self.curve_amount > 0.0 {
                // Compress dynamic range (toward soft)
                curved.powf(1.0 / (1.0 + self.curve_amount))
            } else {
                // Expand dynamic range (toward hard)
                curved.powf(1.0 - self.curve_amount)
            }
        } else {
            curved
        };

        // Scale to output range
        let min = self.min_out.as_f32();
        let max = self.max_out.as_f32();
        min + adjusted * (max - min)
    }
}

impl Default for VelocityMapper {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for VelocityMapper {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("velocity_mapper", "Velocity Mapper")
            .description("Velocity curve shaping for customizing touch response")
            .category(ModuleCategory::Utility)
            .tag("velocity")
            .tag("curve")
            .tag("utility")
            .tag("dynamics")
            .parameter(
                ParameterDescriptor::choice(
                    Param::VelocityMapper(VelocityMapperParam::Curve(VelocityCurve::Linear)),
                    "Curve",
                    VelocityCurve::to_choices(),
                )
                .description("Velocity curve type")
                .widget(WidgetHint::Dropdown),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::VelocityMapper(VelocityMapperParam::MinOut(NormalizedValue::new(0.0))),
                    "Min",
                )
                .description("Minimum output velocity")
                .range(0.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::VelocityMapper(VelocityMapperParam::MaxOut(NormalizedValue::new(1.0))),
                    "Max",
                )
                .description("Maximum output velocity")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::VelocityMapper(VelocityMapperParam::CurveAmount(0.0)),
                    "Amount",
                )
                .description("Curve adjustment (-1 = harder, +1 = softer)")
                .range(-1.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_output("out", "Out").description("Mapped velocity output"))
    }
}

impl PolyModule for VelocityMapper {
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        // Output the mapped velocity as a constant control signal
        for i in 0..context.samples {
            self.output_buffer[i] = self.current_velocity;
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::VelocityMapper(p) = param {
            match p {
                VelocityMapperParam::Curve(c) => self.curve = c,
                VelocityMapperParam::MinOut(v) => self.min_out = v,
                VelocityMapperParam::MaxOut(v) => self.max_out = v,
                VelocityMapperParam::CurveAmount(a) => self.curve_amount = a.clamp(-1.0, 1.0),
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::VelocityMapper(p) = param {
            Some(match p {
                VelocityMapperParam::Curve(_) => self.curve.index() as f32,
                VelocityMapperParam::MinOut(_) => self.min_out.as_f32(),
                VelocityMapperParam::MaxOut(_) => self.max_out.as_f32(),
                VelocityMapperParam::CurveAmount(_) => self.curve_amount,
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::VelocityMapper(VelocityMapperParam::Curve(self.curve)),
            Param::VelocityMapper(VelocityMapperParam::MinOut(self.min_out)),
            Param::VelocityMapper(VelocityMapperParam::MaxOut(self.max_out)),
            Param::VelocityMapper(VelocityMapperParam::CurveAmount(self.curve_amount)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::VelocityMapper
    }

    fn reset(&mut self) {
        self.current_velocity = 0.0;
    }

    fn note_on(&mut self, _note: MidiNote, velocity: f32) {
        // Map the incoming velocity and store it
        self.current_velocity = self.map_velocity(velocity);
    }

    fn note_off(&mut self) {
        self.current_velocity = 0.0;
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = SampleRate::new(sample_rate);
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}
