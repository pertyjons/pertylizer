//! Keyboard Panner module for note-based stereo positioning.
//!
//! Simulates the natural stereo positioning of piano strings,
//! where low notes are panned left and high notes are panned right.

use std::collections::HashMap;

use crate::engine::typed_params::{KeyboardPannerParam, ModuleType, Param};
use crate::modules::core::*;
use crate::types::{MidiNote, NormalizedValue, SampleRate};

/// Keyboard panner with configurable spread and center.
#[derive(Clone)]
pub struct KeyboardPanner {
    // Parameters
    spread: NormalizedValue,
    center_note: u8,
    curve: f32,
    invert: bool,

    // Current pan position (-1.0 to 1.0)
    current_pan: f32,

    // Sample rate
    sample_rate: SampleRate,

    // Output buffers
    output_left: AudioBuffer,
    output_right: AudioBuffer,
}

impl KeyboardPanner {
    pub fn new() -> Self {
        Self {
            spread: NormalizedValue::new(0.5),
            center_note: 60, // Middle C
            curve: 0.0,
            invert: false,

            current_pan: 0.0,

            sample_rate: SampleRate::DVD_QUALITY,
            output_left: AudioBuffer::new(256),
            output_right: AudioBuffer::new(256),
        }
    }

    /// Calculate pan position from MIDI note.
    fn calculate_pan(&mut self, note: MidiNote) {
        let note_val = note.as_u8() as f32;
        let center = self.center_note as f32;

        // Normalize to -1.0 to 1.0 range based on keyboard position
        // Typical piano range is 21 (A0) to 108 (C8)
        let normalized = (note_val - center) / 44.0; // 44 semitones = ~4 octaves each side

        // Apply curve
        let curved = if self.curve.abs() > 0.01 {
            if self.curve > 0.0 {
                // Exponential curve (more center, less extremes)
                normalized.signum() * normalized.abs().powf(1.0 + self.curve)
            } else {
                // Logarithmic curve (less center, more extremes)
                normalized.signum() * normalized.abs().powf(1.0 / (1.0 - self.curve))
            }
        } else {
            normalized
        };

        // Apply spread
        let spread = self.spread.as_f32();
        let mut pan = curved * spread;

        // Invert if needed
        if self.invert {
            pan = -pan;
        }

        self.current_pan = pan.clamp(-1.0, 1.0);
    }

    /// Get left and right gains from pan position using constant power panning.
    #[inline]
    fn pan_gains(&self) -> (f32, f32) {
        // Constant power panning using sin/cos
        let angle = (self.current_pan + 1.0) * 0.25 * std::f32::consts::PI;
        let left = angle.cos();
        let right = angle.sin();
        (left, right)
    }
}

impl Default for KeyboardPanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for KeyboardPanner {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("keyboard_panner", "Keyboard Panner")
            .description("Note-based stereo positioning like a piano")
            .category(ModuleCategory::PhysicalModeling)
            .tag("panner")
            .tag("stereo")
            .tag("piano")
            .tag("spatial")
            .parameter(
                ParameterDescriptor::float(
                    Param::KeyboardPanner(KeyboardPannerParam::Spread(NormalizedValue::new(0.5))),
                    "Spread",
                )
                .description("Stereo spread amount (0 = mono)")
                .range(0.0, 1.0)
                .default(0.5)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::KeyboardPanner(KeyboardPannerParam::CenterNote(60)),
                    "Center",
                )
                .description("Center note for panning (MIDI note)")
                .range(0.0, 127.0)
                .default(60.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::KeyboardPanner(KeyboardPannerParam::Curve(0.0)),
                    "Curve",
                )
                .description("Pan curve shape (-1 to 1)")
                .range(-1.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::KeyboardPanner(KeyboardPannerParam::Invert(false)),
                    "Invert",
                )
                .description("Invert panning direction")
                .range(0.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::None)
                .widget(WidgetHint::Toggle),
            )
            .port(PortDescriptor::audio_input("in", "In").description("Mono input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
    }
}

impl PolyModule for KeyboardPanner {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_left.resize(context.samples);
        self.output_right.resize(context.samples);

        let input = inputs.get("in");
        let (left_gain, right_gain) = self.pan_gains();

        for i in 0..context.samples {
            let input_sample = input.map_or(0.0, |buf| buf[i]);
            self.output_left[i] = input_sample * left_gain;
            self.output_right[i] = input_sample * right_gain;
        }

        if let Some(out_l) = outputs.get_mut("out_l") {
            out_l.copy_from(&self.output_left);
        }
        if let Some(out_r) = outputs.get_mut("out_r") {
            out_r.copy_from(&self.output_right);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::KeyboardPanner(p) = param {
            match p {
                KeyboardPannerParam::Spread(v) => self.spread = v,
                KeyboardPannerParam::CenterNote(n) => self.center_note = n.clamp(0, 127),
                KeyboardPannerParam::Curve(c) => self.curve = c.clamp(-1.0, 1.0),
                KeyboardPannerParam::Invert(b) => self.invert = b,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::KeyboardPanner(p) = param {
            Some(match p {
                KeyboardPannerParam::Spread(_) => self.spread.as_f32(),
                KeyboardPannerParam::CenterNote(_) => f32::from(self.center_note),
                KeyboardPannerParam::Curve(_) => self.curve,
                KeyboardPannerParam::Invert(_) => {
                    if self.invert {
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
            Param::KeyboardPanner(KeyboardPannerParam::Spread(self.spread)),
            Param::KeyboardPanner(KeyboardPannerParam::CenterNote(self.center_note)),
            Param::KeyboardPanner(KeyboardPannerParam::Curve(self.curve)),
            Param::KeyboardPanner(KeyboardPannerParam::Invert(self.invert)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::KeyboardPanner
    }

    fn reset(&mut self) {
        self.current_pan = 0.0;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: f32) {
        self.calculate_pan(note);
    }

    fn note_off(&mut self) {
        // Keep current pan position
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = SampleRate::new(sample_rate);
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}
