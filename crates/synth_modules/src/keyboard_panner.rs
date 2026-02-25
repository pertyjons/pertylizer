//! Keyboard Panner module for note-based stereo positioning.
//!
//! Simulates the natural stereo positioning of piano strings,
//! where low notes are panned left and high notes are panned right.

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ParameterDescriptor,
    ParameterUnit, PolyModule, PortDescriptor, ProcessContext, WidgetHint,
};
use synth_core::{
    BipolarValue, MidiNote, NormalizedValue, Polarity, PortName, SampleRate, StereoBalance,
    StereoSample, Velocity,
};
use synth_core::{KeyboardPannerParam, ModuleType, Param};

/// Keyboard panner with configurable spread and center.
#[derive(Clone)]
pub struct KeyboardPanner {
    // Parameters
    spread: NormalizedValue,
    center_note: MidiNote,
    curve: BipolarValue,
    polarity: Polarity,

    // Current pan position
    current_pan: StereoBalance,

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
            center_note: MidiNote::C4,
            curve: BipolarValue::CENTER,
            polarity: Polarity::Normal,

            current_pan: StereoBalance::CENTER,

            sample_rate: SampleRate::DVD_QUALITY,
            output_left: AudioBuffer::new(1024),
            output_right: AudioBuffer::new(1024),
        }
    }

    /// Calculate pan position from MIDI note.
    fn calculate_pan(&mut self, note: MidiNote) {
        let note_val = note.as_u8() as f32;
        let center = self.center_note.as_u8() as f32;

        // Normalize to -1.0 to 1.0 range based on keyboard position
        // Typical piano range is 21 (A0) to 108 (C8)
        let normalized = (note_val - center) / 44.0; // 44 semitones = ~4 octaves each side

        // Apply curve
        let curve = self.curve.as_f32();
        let curved = if curve.abs() > 0.01 {
            if curve > 0.0 {
                // Exponential curve (more center, less extremes)
                normalized.signum() * normalized.abs().powf(1.0 + curve)
            } else {
                // Logarithmic curve (less center, more extremes)
                normalized.signum() * normalized.abs().powf(1.0 / (1.0 - curve))
            }
        } else {
            normalized
        };

        // Apply spread
        let spread = self.spread.as_f32();
        let pan = curved * spread * self.polarity.multiplier();

        self.current_pan = StereoBalance::new(pan);
    }

    /// Apply current pan to a mono sample, returning stereo.
    #[inline]
    fn apply_pan(&self, mono: f32) -> StereoSample {
        StereoSample::from_mono(mono).apply_pan(self.current_pan)
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
                    Param::KeyboardPanner(KeyboardPannerParam::CenterNote(MidiNote::C4)),
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
                    Param::KeyboardPanner(KeyboardPannerParam::Curve(BipolarValue::CENTER)),
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
                    Param::KeyboardPanner(KeyboardPannerParam::Invert(Polarity::Normal)),
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
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_left.resize(context.samples.as_usize());
        self.output_right.resize(context.samples.as_usize());

        let input = inputs.get(PortName::IN);

        for i in 0..context.samples.as_usize() {
            let input_sample = input.map_or(0.0, |buf| buf[i]);
            let stereo = self.apply_pan(input_sample);
            self.output_left[i] = stereo.left;
            self.output_right[i] = stereo.right;
        }

        if let Some(out_l) = outputs.get_mut(&PortName::OUT_L) {
            out_l.copy_from(&self.output_left);
        }
        if let Some(out_r) = outputs.get_mut(&PortName::OUT_R) {
            out_r.copy_from(&self.output_right);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::KeyboardPanner(p) = param {
            match p {
                KeyboardPannerParam::Spread(v) => self.spread = v,
                KeyboardPannerParam::CenterNote(n) => self.center_note = n,
                KeyboardPannerParam::Curve(c) => self.curve = c,
                KeyboardPannerParam::Invert(p) => self.polarity = p,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::KeyboardPanner(p) = param {
            Some(match p {
                KeyboardPannerParam::Spread(_) => self.spread.as_f32(),
                KeyboardPannerParam::CenterNote(_) => f32::from(self.center_note.as_u8()),
                KeyboardPannerParam::Curve(_) => self.curve.as_f32(),
                KeyboardPannerParam::Invert(_) => {
                    if self.polarity.is_inverted() {
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
            Param::KeyboardPanner(KeyboardPannerParam::Invert(self.polarity)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::KeyboardPanner
    }

    fn reset(&mut self) {
        self.current_pan = StereoBalance::CENTER;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.calculate_pan(note);
    }

    fn note_off(&mut self) {
        // Keep current pan position
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}
