//! String Resonator module using Karplus-Strong algorithm.
//!
//! Physical modeling of a vibrating string with:
//! - Variable feedback (decay time)
//! - Damping (brightness control)
//! - Inharmonicity (string stiffness)
//! - Key tracking for realistic pitch

use std::collections::HashMap;

use crate::engine::typed_params::{ModuleType, Param, StringResonatorParam};
use crate::modules::core::*;
use crate::types::{Gain, MidiNote, NormalizedValue, SampleRate};

/// Maximum delay line size (enough for lowest MIDI notes at 48kHz).
const MAX_DELAY_SIZE: usize = 4096;

/// Karplus-Strong string resonator with inharmonicity.
#[derive(Clone)]
pub struct StringResonator {
    // Parameters
    feedback: NormalizedValue,
    damping: NormalizedValue,
    inharmonicity: NormalizedValue,
    brightness: NormalizedValue,
    level: Gain,

    // Delay line for Karplus-Strong
    delay_line: [f32; MAX_DELAY_SIZE],
    delay_write_pos: usize,
    delay_length: usize,
    fractional_delay: f32,

    // Lowpass filter state for damping
    filter_state: f32,
    prev_output: f32,

    // Allpass filter for inharmonicity
    allpass_state: f32,

    // Sample rate
    sample_rate: SampleRate,

    // Current note frequency
    current_freq: f32,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl StringResonator {
    pub fn new() -> Self {
        Self {
            feedback: NormalizedValue::new(0.995),
            damping: NormalizedValue::new(0.3),
            inharmonicity: NormalizedValue::new(0.0),
            brightness: NormalizedValue::new(0.7),
            level: Gain::new(0.8),

            delay_line: [0.0; MAX_DELAY_SIZE],
            delay_write_pos: 0,
            delay_length: 100,
            fractional_delay: 0.0,

            filter_state: 0.0,
            prev_output: 0.0,
            allpass_state: 0.0,

            sample_rate: SampleRate::DVD_QUALITY,
            current_freq: 440.0,
            output_buffer: AudioBuffer::new(256),
        }
    }

    /// Set the fundamental frequency of the string.
    fn set_frequency(&mut self, freq: f32) {
        self.current_freq = freq.max(20.0).min(20000.0);
        let period = self.sample_rate.as_f32() / self.current_freq;
        self.delay_length = (period as usize).min(MAX_DELAY_SIZE - 1).max(1);
        self.fractional_delay = period - self.delay_length as f32;
    }

    /// Excite the string with an impulse (noise burst).
    fn excite(&mut self, brightness: f32) {
        // Fill delay line with filtered noise for excitation
        let bright = brightness.clamp(0.0, 1.0);

        for i in 0..self.delay_length {
            // White noise
            let noise = fastrand::f32() * 2.0 - 1.0;

            // Simple lowpass for brightness control
            self.filter_state = self.filter_state * (1.0 - bright) + noise * bright;
            self.delay_line[i] = self.filter_state;
        }

        // Reset filter state after excitation
        self.filter_state = 0.0;
    }

    /// Process one sample through the string model.
    #[inline]
    fn process_sample(&mut self, input: f32) -> f32 {
        // Read from delay line with linear interpolation for fractional delay
        let read_pos = (self.delay_write_pos + MAX_DELAY_SIZE - self.delay_length) % MAX_DELAY_SIZE;
        let next_pos = (read_pos + 1) % MAX_DELAY_SIZE;

        let delayed = self.delay_line[read_pos] * (1.0 - self.fractional_delay)
            + self.delay_line[next_pos] * self.fractional_delay;

        // Damping filter (simple one-pole lowpass)
        let damp = self.damping.as_f32();
        let filtered = self.prev_output * damp + delayed * (1.0 - damp);
        self.prev_output = filtered;

        // Allpass filter for inharmonicity (dispersion)
        let inharm = self.inharmonicity.as_f32() * 0.5;
        let allpass_out = if inharm > 0.001 {
            let a = (1.0 - inharm) / (1.0 + inharm);
            let y = a * (filtered - self.allpass_state) + self.filter_state;
            self.allpass_state = y;
            self.filter_state = filtered;
            y
        } else {
            filtered
        };

        // Apply feedback
        let feedback_val = self.feedback.as_f32();
        let feedback_sample = (allpass_out + input) * feedback_val;

        // Write to delay line
        self.delay_line[self.delay_write_pos] = feedback_sample.clamp(-1.0, 1.0);
        self.delay_write_pos = (self.delay_write_pos + 1) % MAX_DELAY_SIZE;

        allpass_out
    }
}

impl Default for StringResonator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for StringResonator {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("string_resonator", "String Resonator")
            .description("Karplus-Strong string synthesis with inharmonicity")
            .category(ModuleCategory::PhysicalModeling)
            .tag("string")
            .tag("physical")
            .tag("karplus")
            .tag("piano")
            .parameter(
                ParameterDescriptor::float(
                    Param::StringResonator(StringResonatorParam::Feedback(NormalizedValue::new(
                        0.995,
                    ))),
                    "Feedback",
                )
                .description("String decay time (higher = longer sustain)")
                .range(0.9, 0.9999)
                .default(0.995)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::StringResonator(StringResonatorParam::Damping(NormalizedValue::new(
                        0.3,
                    ))),
                    "Damping",
                )
                .description("High frequency damping (higher = darker)")
                .range(0.0, 0.9)
                .default(0.3)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::StringResonator(StringResonatorParam::Inharmonicity(
                        NormalizedValue::new(0.0),
                    )),
                    "Inharm",
                )
                .description("String stiffness / inharmonicity")
                .range(0.0, 1.0)
                .default(0.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::StringResonator(StringResonatorParam::Brightness(NormalizedValue::new(
                        0.7,
                    ))),
                    "Bright",
                )
                .description("Excitation brightness")
                .range(0.0, 1.0)
                .default(0.7)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::StringResonator(StringResonatorParam::Level(Gain::new(0.8))),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(0.8)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in", "In").description("Excitation input"))
            .port(PortDescriptor::audio_output("out", "Out").description("String output"))
    }
}

impl PolyModule for StringResonator {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<String, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.output_buffer.resize(context.samples);

        let input = inputs.get("in");
        let level = self.level.as_f32();

        for i in 0..context.samples {
            let input_sample = input.map_or(0.0, |buf| buf[i]);
            let output = self.process_sample(input_sample);
            self.output_buffer[i] = output * level;
        }

        if let Some(out) = outputs.get_mut("out") {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::StringResonator(p) = param {
            match p {
                StringResonatorParam::Feedback(v) => self.feedback = v,
                StringResonatorParam::Damping(v) => self.damping = v,
                StringResonatorParam::Inharmonicity(v) => self.inharmonicity = v,
                StringResonatorParam::Brightness(v) => self.brightness = v,
                StringResonatorParam::Level(g) => self.level = g,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::StringResonator(p) = param {
            Some(match p {
                StringResonatorParam::Feedback(_) => self.feedback.as_f32(),
                StringResonatorParam::Damping(_) => self.damping.as_f32(),
                StringResonatorParam::Inharmonicity(_) => self.inharmonicity.as_f32(),
                StringResonatorParam::Brightness(_) => self.brightness.as_f32(),
                StringResonatorParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::StringResonator(StringResonatorParam::Feedback(self.feedback)),
            Param::StringResonator(StringResonatorParam::Damping(self.damping)),
            Param::StringResonator(StringResonatorParam::Inharmonicity(self.inharmonicity)),
            Param::StringResonator(StringResonatorParam::Brightness(self.brightness)),
            Param::StringResonator(StringResonatorParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::StringResonator
    }

    fn reset(&mut self) {
        self.delay_line.fill(0.0);
        self.filter_state = 0.0;
        self.prev_output = 0.0;
        self.allpass_state = 0.0;
        self.delay_write_pos = 0;
    }

    fn note_on(&mut self, note: MidiNote, velocity: f32) {
        // Set frequency from MIDI note
        let freq = 440.0 * 2.0_f32.powf((f32::from(note.as_u8()) - 69.0) / 12.0);
        self.set_frequency(freq);

        // Excite string with velocity-scaled brightness
        let bright = self.brightness.as_f32() * (0.5 + velocity * 0.5);
        self.excite(bright);
    }

    fn note_off(&mut self) {
        // Increase damping on note off for faster decay
        // (don't stop immediately - let string ring out)
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = SampleRate::new(sample_rate);
        // Recalculate delay length for current frequency
        self.set_frequency(self.current_freq);
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}
