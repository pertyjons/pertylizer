//! Granular synthesis oscillator.
//!
//! Features:
//! - 32 concurrent grains (fixed pool, no heap allocation in process)
//! - Pre-allocated grain buffer (2s audio, filled with selectable waveform)
//! - Density-based grain triggering
//! - Position, pitch, and pan spread per grain
//! - Freeze mode for looping at current position
//! - Selectable grain window envelope (Hann/Gaussian/Trapezoid)
//! - RT-safe xorshift32 RNG

use std::collections::HashMap;
use std::f32::consts::TAU;

use synth_core::module_traits::ChoiceOption;
use synth_core::{
    AudioBuffer, BipolarValue, Describable, Gain, GrainSource, GrainWindow, GranularParam, Hertz,
    InputPorts, Milliseconds, ModuleCategory, ModuleDescriptor, ModuleType, NormalizedValue, Param,
    ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, PortName, ProcessContext,
    SampleRate, WidgetHint,
};
use synth_core::{MidiNote, Velocity};

/// Maximum number of simultaneous grains.
const MAX_GRAINS: usize = 32;

/// Source buffer duration in seconds.
const SOURCE_BUFFER_SECONDS: f32 = 2.0;

/// Maximum source buffer size at 48kHz.
const MAX_SOURCE_SAMPLES: usize = 96_000;

/// A single grain instance.
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Grain {
    active: bool,
    /// Start position in source buffer (in samples).
    start_pos: usize,
    /// Current position within the grain (in samples).
    pos: f32,
    /// Grain length in samples.
    length: usize,
    /// Playback rate (1.0 = normal pitch).
    rate: f32,
    /// Stereo pan (-1 left, +1 right).
    pan: BipolarValue,
}

impl Default for Grain {
    fn default() -> Self {
        Self {
            active: false,
            start_pos: 0,
            pos: 0.0,
            length: 0,
            rate: 1.0,
            pan: BipolarValue::CENTER,
        }
    }
}

/// RT-safe xorshift32 pseudo-random number generator.
#[derive(Clone, Copy)]
struct Xorshift32 {
    state: u32,
}

impl Xorshift32 {
    fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    /// Returns a value in [0, 1).
    #[inline]
    fn next_f32(&mut self) -> f32 {
        crate::math::xorshift32(&mut self.state)
    }

    /// Returns a value in [-1, 1).
    #[inline]
    fn next_bipolar(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

/// Granular synthesis oscillator.
#[derive(Clone)]
pub struct GranularOsc {
    // Parameters
    grain_size: Milliseconds,
    density: NormalizedValue,
    position: NormalizedValue,
    position_spread: NormalizedValue,
    pitch_spread: NormalizedValue,
    pan_spread: NormalizedValue,
    freeze: bool,
    window: GrainWindow,
    source: GrainSource,
    level: Gain,

    // Grain pool (fixed-size, no allocation)
    grains: [Grain; MAX_GRAINS],

    // Pre-allocated source buffer
    source_buffer: Vec<f32>,
    source_len: usize,

    // State
    sample_rate: SampleRate,
    note_freq: Hertz,
    samples_until_next_grain: f32,
    rng: Xorshift32,

    // Output buffer
    output_buffer: AudioBuffer,
}

impl GranularOsc {
    pub fn new() -> Self {
        let mut osc = Self {
            grain_size: Milliseconds::new(50.0),
            density: NormalizedValue::new(0.5),
            position: NormalizedValue::MIN,
            position_spread: NormalizedValue::new(0.1),
            pitch_spread: NormalizedValue::MIN,
            pan_spread: NormalizedValue::MIN,
            freeze: false,
            window: GrainWindow::Hann,
            source: GrainSource::Saw,
            level: Gain::UNITY,

            grains: [Grain::default(); MAX_GRAINS],

            source_buffer: vec![0.0; MAX_SOURCE_SAMPLES],
            source_len: MAX_SOURCE_SAMPLES,

            sample_rate: SampleRate::DVD_QUALITY,
            note_freq: Hertz::A4,
            samples_until_next_grain: 0.0,
            rng: Xorshift32::new(42),

            output_buffer: AudioBuffer::new(1024),
        };
        // Fill the source buffer up-front so the first render produces audio
        // even if no one calls `set_sample_rate` before `process` and the
        // patch's stored `source` matches the constructor default (in which
        // case `set_param`'s equality skip would have left the buffer as
        // zeros, producing silent grains).
        osc.fill_source_buffer();
        osc
    }

    /// Fill the source buffer with the selected waveform at a fixed base pitch.
    fn fill_source_buffer(&mut self) {
        let sr = self.sample_rate.as_f32();
        self.source_len = (SOURCE_BUFFER_SECONDS * sr) as usize;
        self.source_len = self.source_len.min(MAX_SOURCE_SAMPLES);

        let base_freq = 110.0; // Low A for rich content
        let phase_inc = base_freq / sr;

        let mut rng = Xorshift32::new(12345);
        let mut phase = 0.0f32;

        for i in 0..self.source_len {
            self.source_buffer[i] = match self.source {
                GrainSource::Saw => 2.0 * phase - 1.0,
                GrainSource::Sine => (TAU * phase).sin(),
                GrainSource::Square => {
                    if phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                GrainSource::Triangle => {
                    if phase < 0.5 {
                        4.0 * phase - 1.0
                    } else {
                        3.0 - 4.0 * phase
                    }
                }
                GrainSource::Noise => rng.next_bipolar(),
            };
            phase += phase_inc;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }
    }

    /// Compute grain window envelope value.
    #[inline]
    fn grain_envelope(window: GrainWindow, t: f32) -> f32 {
        match window {
            GrainWindow::Hann => crate::math::hann_window(t),
            GrainWindow::Gaussian => crate::math::gaussian_window(t, 0.4),
            GrainWindow::Trapezoid => crate::math::trapezoid_window(t, 0.1),
        }
    }

    /// Spawn a new grain.
    fn spawn_grain(&mut self) {
        // Find a free grain slot
        let slot = self.grains.iter().position(|g| !g.active);
        let Some(idx) = slot else { return };

        let sr = self.sample_rate.as_f32();
        let grain_samples = (self.grain_size.as_f32() * 0.001 * sr) as usize;
        let grain_samples = grain_samples.clamp(16, self.source_len / 2);

        // Position with spread
        let base_pos = self.position.as_f32();
        let spread = self.position_spread.as_f32();
        let pos = (base_pos + self.rng.next_bipolar() * spread * 0.5).clamp(0.0, 1.0);
        let max_start = self.source_len.saturating_sub(grain_samples);
        let start = (pos * max_start as f32) as usize;

        // Pitch variation (semitones -> rate)
        let pitch_spread_semitones = self.pitch_spread.as_f32() * 24.0;
        let pitch_offset = self.rng.next_bipolar() * pitch_spread_semitones;
        let rate =
            (self.note_freq.as_f32() / 440.0) * crate::math::semitones_to_ratio(pitch_offset);

        // Pan
        let pan = BipolarValue::new(self.rng.next_bipolar() * self.pan_spread.as_f32());

        self.grains[idx] = Grain {
            active: true,
            start_pos: start,
            pos: 0.0,
            length: grain_samples,
            rate,
            pan,
        };
    }
}

impl Default for GranularOsc {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for GranularOsc {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("granular_osc", "Granular")
            .description("Granular synthesis with 32 concurrent grains")
            .category(ModuleCategory::Oscillator)
            .tag("granular")
            .tag("oscillator")
            .tag("texture")
            .parameter(
                ParameterDescriptor::float(
                    "grain_size",
                    Param::GranularOsc(GranularParam::GrainSize(Milliseconds::new(50.0))),
                    "Grain Size",
                )
                .description("Length of each grain in milliseconds")
                .range(5.0, 500.0)
                .default(50.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "density",
                    Param::GranularOsc(GranularParam::Density(NormalizedValue::new(0.5))),
                    "Density",
                )
                .description("Grain trigger rate (0 = sparse, 1 = dense)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "position",
                    Param::GranularOsc(GranularParam::Position(NormalizedValue::MIN)),
                    "Position",
                )
                .description("Read position within the source buffer (0 = start, 1 = end)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pos_spread",
                    Param::GranularOsc(GranularParam::PositionSpread(NormalizedValue::new(0.1))),
                    "Pos Spread",
                )
                .description("Random offset around the read position per grain")
                .range(0.0, 1.0)
                .default(0.1)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pitch_spread",
                    Param::GranularOsc(GranularParam::PitchSpread(NormalizedValue::MIN)),
                    "Pitch Spread",
                )
                .description("Random pitch variation per grain")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pan_spread",
                    Param::GranularOsc(GranularParam::PanSpread(NormalizedValue::MIN)),
                    "Pan Spread",
                )
                .description("Random stereo pan per grain (0 = mono, 1 = full)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "freeze",
                    Param::GranularOsc(GranularParam::Freeze(false)),
                    "Freeze",
                )
                .description("Freeze source buffer playback (0 = off, 1 = on)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "window",
                    Param::GranularOsc(GranularParam::Window(GrainWindow::Hann)),
                    "Window",
                    GrainWindow::ALL
                        .iter()
                        .map(|w| {
                            ChoiceOption::new(w.id(), w.name()).with_description(w.description())
                        })
                        .collect(),
                )
                .description("Grain envelope shape"),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "source",
                    Param::GranularOsc(GranularParam::Source(GrainSource::Saw)),
                    "Source",
                    GrainSource::ALL
                        .iter()
                        .map(|s| {
                            ChoiceOption::new(s.id(), s.name()).with_description(s.description())
                        })
                        .collect(),
                )
                .description("Source waveform used to fill the grain buffer"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::GranularOsc(GranularParam::Level(Gain::UNITY)),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(
                PortDescriptor::audio_output("out", "Out")
                    .description("Granular output. Connect to: Amplifier In, Filter In"),
            )
    }
}

impl PolyModule for GranularOsc {
    #[allow(clippy::too_many_lines)]
    fn process(
        &mut self,
        _inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        let samples = context.samples.as_usize();
        self.output_buffer.resize(samples);
        self.output_buffer.clear();

        let sr = self.sample_rate.as_f32();
        let level = self.level.as_f32();

        // Density: grains per second (1-100 range mapped from 0-1)
        let grains_per_second = 1.0 + self.density.as_f32() * 99.0;
        let samples_between_grains = sr / grains_per_second;

        for i in 0..samples {
            // Trigger new grains
            self.samples_until_next_grain -= 1.0;
            if self.samples_until_next_grain <= 0.0 {
                self.spawn_grain();
                self.samples_until_next_grain += samples_between_grains;
                if self.samples_until_next_grain < 1.0 {
                    self.samples_until_next_grain = 1.0;
                }
            }

            // Mix active grains
            let mut mix = 0.0f32;
            for grain in &mut self.grains {
                if !grain.active {
                    continue;
                }

                if grain.length == 0 {
                    grain.active = false;
                    continue;
                }

                let t = grain.pos / grain.length as f32;
                if t >= 1.0 {
                    grain.active = false;
                    continue;
                }

                let env = Self::grain_envelope(self.window, t);

                // Read from source buffer with linear interpolation
                let read_pos = grain.start_pos as f32 + grain.pos * grain.rate;
                let idx = read_pos as usize;
                let frac = read_pos - idx as f32;
                let s0 = if idx < self.source_len {
                    self.source_buffer[idx]
                } else {
                    0.0
                };
                let s1 = if idx + 1 < self.source_len {
                    self.source_buffer[idx + 1]
                } else {
                    0.0
                };
                let sample = crate::math::lerp(s0, s1, frac);

                mix += sample * env;
                grain.pos += 1.0;
            }

            self.output_buffer[i] = mix * level;
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::GranularOsc(p) = param {
            match p {
                GranularParam::GrainSize(ms) => self.grain_size = ms,
                GranularParam::Density(v) => self.density = v,
                GranularParam::Position(v) => self.position = v,
                GranularParam::PositionSpread(v) => self.position_spread = v,
                GranularParam::PitchSpread(v) => self.pitch_spread = v,
                GranularParam::PanSpread(v) => self.pan_spread = v,
                GranularParam::Freeze(b) => self.freeze = b,
                GranularParam::Window(w) => self.window = w,
                GranularParam::Source(s) => {
                    if s != self.source {
                        self.source = s;
                        self.fill_source_buffer();
                    }
                }
                GranularParam::Level(g) => self.level = g,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::GranularOsc(p) = param {
            #[allow(clippy::cast_precision_loss)]
            Some(match p {
                GranularParam::GrainSize(_) => self.grain_size.as_f32(),
                GranularParam::Density(_) => self.density.as_f32(),
                GranularParam::Position(_) => self.position.as_f32(),
                GranularParam::PositionSpread(_) => self.position_spread.as_f32(),
                GranularParam::PitchSpread(_) => self.pitch_spread.as_f32(),
                GranularParam::PanSpread(_) => self.pan_spread.as_f32(),
                GranularParam::Freeze(_) => {
                    if self.freeze {
                        1.0
                    } else {
                        0.0
                    }
                }
                GranularParam::Window(_) => self.window.index() as f32,
                GranularParam::Source(_) => self.source.index() as f32,
                GranularParam::Level(_) => self.level.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::GranularOsc(GranularParam::GrainSize(self.grain_size)),
            Param::GranularOsc(GranularParam::Density(self.density)),
            Param::GranularOsc(GranularParam::Position(self.position)),
            Param::GranularOsc(GranularParam::PositionSpread(self.position_spread)),
            Param::GranularOsc(GranularParam::PitchSpread(self.pitch_spread)),
            Param::GranularOsc(GranularParam::PanSpread(self.pan_spread)),
            Param::GranularOsc(GranularParam::Freeze(self.freeze)),
            Param::GranularOsc(GranularParam::Window(self.window)),
            Param::GranularOsc(GranularParam::Source(self.source)),
            Param::GranularOsc(GranularParam::Level(self.level)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::GranularOsc
    }

    fn reset(&mut self) {
        for grain in &mut self.grains {
            grain.active = false;
        }
        self.samples_until_next_grain = 0.0;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.samples_until_next_grain = 0.0;
    }

    fn note_off(&mut self) {
        // Grains continue until they finish naturally
    }

    fn set_sample_rate(&mut self, rate: SampleRate) {
        self.sample_rate = rate;
        self.fill_source_buffer();
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A freshly-constructed GranularOsc must already have a populated source
    /// buffer. Otherwise the offline `analyze_note` preview path renders
    /// silence whenever the patch's stored source matches the constructor
    /// default — `set_param`'s equality skip suppresses the only fill site
    /// that runs before the first `process` call.
    #[test]
    fn source_buffer_is_populated_at_construction() {
        let osc = GranularOsc::new();
        let nonzero = osc
            .source_buffer
            .iter()
            .take(osc.source_len)
            .any(|s| s.abs() > 1e-6);
        assert!(
            nonzero,
            "GranularOsc::new() left source_buffer as all zeros — \
             grains will be silent until something forces a refill"
        );
        assert!(
            osc.source_len > 0,
            "GranularOsc::new() left source_len at 0"
        );
    }

    /// Setting Source(Saw) on a freshly constructed module (whose default
    /// source is already Saw) must not regress to silence even though the
    /// equality check in `set_param` skips the explicit refill — the buffer
    /// from `new()` is the safety net.
    #[test]
    fn setting_source_to_default_leaves_buffer_valid() {
        let mut osc = GranularOsc::new();
        osc.set_param(Param::GranularOsc(GranularParam::Source(GrainSource::Saw)));
        let nonzero = osc
            .source_buffer
            .iter()
            .take(osc.source_len)
            .any(|s| s.abs() > 1e-6);
        assert!(
            nonzero,
            "Setting Source(Saw) on a fresh module produced an empty buffer"
        );
    }

    /// All five GrainSource variants must produce non-empty buffers when
    /// applied via set_param after construction. Previously only sources
    /// that *differed* from the default Saw triggered a fill, leaving Saw
    /// silently broken.
    #[test]
    fn every_grain_source_produces_nonzero_buffer() {
        for source in GrainSource::ALL {
            let mut osc = GranularOsc::new();
            osc.set_param(Param::GranularOsc(GranularParam::Source(source)));
            let nonzero = osc
                .source_buffer
                .iter()
                .take(osc.source_len)
                .any(|s| s.abs() > 1e-6);
            assert!(
                nonzero,
                "GrainSource::{:?} produced an all-zero buffer",
                source
            );
        }
    }
}
