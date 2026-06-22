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
    ParamModOffsets, ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor, PortName,
    ProcessContext, SampleRate, WidgetHint,
};
use synth_core::{MidiNote, Velocity};

/// Maximum number of simultaneous grains.
const MAX_GRAINS: usize = 32;

/// Source buffer duration in seconds.
const SOURCE_BUFFER_SECONDS: f32 = 2.0;

/// Maximum source buffer size at 48kHz.
const MAX_SOURCE_SAMPLES: usize = 96_000;

/// Base frequency the source buffer is filled at (low A, for harmonic richness).
/// Grain playback rate is `note_freq / SOURCE_BASE_FREQ`, so a played note comes
/// out at concert pitch (rate 1.0 reproduces the buffer's native 110 Hz).
const SOURCE_BASE_FREQ: f32 = 110.0;

/// A single grain instance.
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Grain {
    active: bool,
    /// Start position in source buffer (in samples, fractional). Captured from the
    /// shared free-running playhead at spawn time so overlapping grains stay
    /// phase-coherent.
    start_pos: f32,
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
            start_pos: 0.0,
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
    /// `(source, sample_rate)` the `source_buffer` was last filled with. The
    /// rebuild guard compares against this — not the live `source`/`sample_rate`
    /// fields — so the heavy fill runs at most once per *actual* change and is
    /// never repeated per block. Set to `None` to force a (re)fill.
    built_with: Option<(GrainSource, SampleRate)>,

    // State
    sample_rate: SampleRate,
    note_freq: Hertz,
    /// Free-running read position into the source buffer (in samples). Advances
    /// every output sample at the note's base rate and wraps at the buffer end.
    /// New grains capture this value as their start position, which keeps all
    /// overlapping grains phase-coherent (see `spawn_grain`).
    playhead: f32,
    samples_until_next_grain: f32,
    rng: Xorshift32,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

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
            built_with: None,

            sample_rate: SampleRate::DVD_QUALITY,
            note_freq: Hertz::A4,
            playhead: 0.0,
            samples_until_next_grain: 0.0,
            rng: Xorshift32::new(42),
            mod_offsets: ParamModOffsets::new(),

            output_buffer: AudioBuffer::new(1024),
        };
        // Fill the source buffer up-front so the first render produces audio
        // even if no one calls `set_sample_rate` before `process` and the
        // patch's stored `source` matches the constructor default (in which
        // case the change-guarded rebuild would skip the fill, leaving the
        // buffer as zeros and producing silent grains). This also seeds
        // `built_with` so later guards know what's already in the buffer.
        osc.fill_source_buffer();
        osc
    }

    /// Rebuild the source buffer only if the live `(source, sample_rate)` no
    /// longer matches what the buffer was last filled with. This is the single
    /// guarded entry point used from the audio thread (`process`): it skips the
    /// ~96k-iteration trig fill entirely when nothing changed, so the common
    /// per-block case is a cheap tuple comparison.
    fn rebuild_source_buffer_if_changed(&mut self) {
        if self.built_with == Some((self.source, self.sample_rate)) {
            return;
        }
        self.fill_source_buffer();
    }

    /// Fill the source buffer with the selected waveform at a fixed base pitch.
    /// Records `(source, sample_rate)` in `built_with` so the guard above can
    /// short-circuit subsequent calls. Reuses the pre-allocated buffer in place.
    fn fill_source_buffer(&mut self) {
        let sr = self.sample_rate.as_f32();
        self.source_len = (SOURCE_BUFFER_SECONDS * sr) as usize;
        self.source_len = self.source_len.min(MAX_SOURCE_SAMPLES);

        let phase_inc = SOURCE_BASE_FREQ / sr;

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

        self.built_with = Some((self.source, self.sample_rate));
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

        // Start position = the shared free-running playhead at spawn time, offset by
        // the Position knob (+ per-grain spread). Capturing the same playhead for every
        // grain is what keeps overlapping grains phase-coherent: with spread = 0 they
        // all read the identical buffer sample at a given output time, so the windowed
        // overlap-add reconstructs the source tone instead of comb-filtering it into
        // pitch-dependent dropouts. The read wraps at the buffer end, so no clamp here.
        let base_pos = self.position.as_f32();
        let spread = self.position_spread.as_f32();
        let pos_offset =
            (base_pos + self.rng.next_bipolar() * spread * 0.5) * self.source_len as f32;
        let start = self.playhead + pos_offset;

        // Pitch variation (semitones -> rate)
        let pitch_spread_semitones = self.pitch_spread.as_f32() * 24.0;
        let pitch_offset = self.rng.next_bipolar() * pitch_spread_semitones;
        let rate = (self.note_freq.as_f32() / SOURCE_BASE_FREQ)
            * crate::math::semitones_to_ratio(pitch_offset);

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
                // Discrete on/off toggle, not a continuous automation target.
                .modulatable(false)
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

        // Follow the render sample rate. The engine propagates the rate to
        // voice-graph modules only through ProcessContext — it never calls
        // set_sample_rate on them — and this module's pitch and grain timing are
        // tied to the rate its source buffer was filled at. Without this sync an
        // offline render at 44.1 kHz of a buffer filled at the 48 kHz default
        // plays back ~147 cents flat.
        self.sample_rate = context.sample_rate;
        // Refill the source buffer only when the source waveform or sample rate
        // actually changed since the last fill — `rebuild_source_buffer_if_changed`
        // short-circuits to a tuple comparison otherwise. Done once per block
        // (not per sample), so the per-sample hot path below stays allocation-free.
        // Residual cost: the rare block where a change *did* land pays the
        // ~96k-iteration trig fill on the audio thread. That happens at most once
        // per genuine waveform/rate change (e.g. switching the Source choice),
        // not every block, so it can't repeat into a sustained dropout.
        self.rebuild_source_buffer_if_changed();

        let sr = self.sample_rate.as_f32();
        let level = self.mod_offsets.effective("level", self.level.as_f32());

        // Density: grains per second (1-100 range mapped from 0-1)
        let grains_per_second =
            1.0 + self.mod_offsets.effective("density", self.density.as_f32()) * 99.0;
        let samples_between_grains = sr / grains_per_second;

        // The remaining mod destinations (grain_size, position, pos_spread,
        // pitch_spread, pan_spread) are read inside spawn_grain, called from the
        // loop below. Apply their effective values to the fields for the block
        // and restore after (single loop, no early return).
        let saved = (
            self.grain_size,
            self.position,
            self.position_spread,
            self.pitch_spread,
            self.pan_spread,
        );
        self.grain_size = Milliseconds::new(
            self.mod_offsets
                .effective("grain_size", self.grain_size.as_f32()),
        );
        self.position = NormalizedValue::new(
            self.mod_offsets
                .effective("position", self.position.as_f32()),
        );
        self.position_spread = NormalizedValue::new(
            self.mod_offsets
                .effective("pos_spread", self.position_spread.as_f32()),
        );
        self.pitch_spread = NormalizedValue::new(
            self.mod_offsets
                .effective("pitch_spread", self.pitch_spread.as_f32()),
        );
        self.pan_spread = NormalizedValue::new(
            self.mod_offsets
                .effective("pan_spread", self.pan_spread.as_f32()),
        );

        // Base playback rate (note pitch relative to the buffer's native pitch). The
        // shared playhead advances at this rate; per-grain Pitch Spread detunes around it.
        let base_rate = self.note_freq.as_f32() / SOURCE_BASE_FREQ;
        let source_len_f = self.source_len as f32;

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

                // Read from source buffer with linear interpolation, wrapping at the
                // buffer end so a grain that runs past it loops back instead of reading
                // silence (the buffer holds a whole number of source cycles).
                let read_pos = (grain.start_pos + grain.pos * grain.rate).rem_euclid(source_len_f);
                let idx = (read_pos as usize).min(self.source_len - 1);
                let frac = read_pos - idx as f32;
                let s0 = self.source_buffer[idx];
                let next = if idx + 1 < self.source_len {
                    idx + 1
                } else {
                    0
                };
                let s1 = self.source_buffer[next];
                let sample = crate::math::lerp(s0, s1, frac);

                mix += sample * env;
                grain.pos += 1.0;
            }

            self.output_buffer[i] = mix * level;

            // Advance the shared playhead, wrapping at the buffer end.
            self.playhead += base_rate;
            while self.playhead >= source_len_f {
                self.playhead -= source_len_f;
            }
        }

        // Restore the base params so next block re-applies offsets from scratch.
        (
            self.grain_size,
            self.position,
            self.position_spread,
            self.pitch_spread,
            self.pan_spread,
        ) = saved;

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
                // Just record the new waveform. The heavy ~96k-iteration refill
                // is deferred to the next `process` block, which rebuilds via
                // the change-guarded path — so a Source change drained on the
                // audio thread costs only a field write here, not a fill.
                GranularParam::Source(s) => self.source = s,
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

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        for grain in &mut self.grains {
            grain.active = false;
        }
        self.playhead = 0.0;
        self.samples_until_next_grain = 0.0;
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        self.note_freq = note.to_frequency();
        self.playhead = 0.0;
        self.samples_until_next_grain = 0.0;
    }

    fn note_off(&mut self) {
        // Grains continue until they finish naturally
    }

    fn set_sample_rate(&mut self, rate: SampleRate) {
        self.sample_rate = rate;
        // Off the audio thread (engine setup). Guarded so it's a no-op when the
        // rate is unchanged; `process` would otherwise catch the change anyway.
        self.rebuild_source_buffer_if_changed();
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::Velocity;

    /// `level` is a working mod destination via the generic store, and the
    /// transiently-applied spawn-time params are restored after each block.
    #[test]
    fn level_mod_offset_scales_and_params_restore() {
        let mut osc = GranularOsc::new();
        let desc = osc.descriptor();
        osc.mod_offsets_mut().unwrap().populate(&desc);
        osc.note_on(MidiNote::A4, Velocity::MAX);

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(512),
            ..ProcessContext::default()
        };
        fn rms(osc: &mut GranularOsc, ctx: &ProcessContext) -> f32 {
            let mut outs = HashMap::new();
            outs.insert(PortName::OUT, AudioBuffer::new(512));
            osc.process(InputPorts::empty(), &mut outs, ctx);
            let b = &outs[&PortName::OUT];
            (0..b.len()).map(|i| b[i] * b[i]).sum::<f32>().sqrt()
        }

        let base = rms(&mut osc, &ctx);
        assert!(base > 1e-4, "base output present, got {base}");

        // Drive level to ~0 (Gain 0..2, base 1.0 → norm 0.5; -1.0 offset clamps
        // to 0 → silence). Unambiguous despite the grain stream's variation.
        let pos_before = osc.position.as_f32();
        osc.set_mod_offset("level", -1.0);
        osc.set_mod_offset("position", 0.3); // exercise a transient spawn param
        let silent = rms(&mut osc, &ctx);
        assert!(
            (osc.position.as_f32() - pos_before).abs() < 1e-6,
            "position field must be restored after process"
        );
        assert!(
            silent < base * 0.05,
            "level→0 should silence output: {silent}"
        );

        osc.clear_mod_offsets();
        let reverted = rms(&mut osc, &ctx);
        assert!(reverted > silent, "clearing restores level");
    }

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

    /// Render `blocks` × 512 samples of a held note and return the full output.
    fn render_held_note(note: u8) -> Vec<f32> {
        const BLOCK: usize = 512;
        const BLOCKS: usize = 70; // ~0.75 s at 48 kHz
        let mut osc = GranularOsc::new();
        // The "tuning check" config that exposed the bug: zero spreads, full
        // density, long grains, periodic Sine source, read from position 0.
        for p in [
            Param::GranularOsc(GranularParam::Source(GrainSource::Sine)),
            Param::GranularOsc(GranularParam::Density(NormalizedValue::new(1.0))),
            Param::GranularOsc(GranularParam::GrainSize(Milliseconds::new(150.0))),
            Param::GranularOsc(GranularParam::Position(NormalizedValue::MIN)),
            Param::GranularOsc(GranularParam::PositionSpread(NormalizedValue::MIN)),
            Param::GranularOsc(GranularParam::PitchSpread(NormalizedValue::MIN)),
        ] {
            osc.set_param(p);
        }
        osc.note_on(MidiNote::new(note), Velocity::new(1.0));

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(BLOCK),
            sample_rate: SampleRate::DVD_QUALITY,
            ..ProcessContext::default()
        };
        let mut out = HashMap::new();
        out.insert(PortName::OUT, AudioBuffer::new(BLOCK));

        let mut samples = Vec::with_capacity(BLOCK * BLOCKS);
        for _ in 0..BLOCKS {
            osc.process(InputPorts::empty(), &mut out, &ctx);
            let buf = &out[&PortName::OUT];
            for i in 0..BLOCK {
                samples.push(buf[i]);
            }
        }
        samples
    }

    fn rms(slice: &[f32]) -> f32 {
        if slice.is_empty() {
            return 0.0;
        }
        (slice.iter().map(|s| s * s).sum::<f32>() / slice.len() as f32).sqrt()
    }

    /// A held note must keep sounding for its whole duration at *every* pitch.
    ///
    /// Regression guard: when every grain read from the same fixed buffer
    /// position (the old behaviour), overlapping phase-locked grains comb-filtered
    /// each other into near-silence at most pitches — e.g. C4 collapsed to ~1 % of
    /// its onset level a few grains in while D4 sustained. The shared free-running
    /// playhead keeps overlapping grains phase-coherent so the late-note level
    /// stays close to the early-note level across the chromatic scale.
    #[test]
    fn held_note_does_not_collapse_across_pitches() {
        // One octave, C4..B4 — the old bug killed roughly half of these.
        for note in 60u8..=71 {
            let buf = render_held_note(note);
            let len = buf.len();
            // Skip the attack; compare an early sustain window to a late one.
            let early = rms(&buf[len / 5..len * 2 / 5]);
            let late = rms(&buf[len * 3 / 5..len * 4 / 5]);
            assert!(
                late > 0.05,
                "note {note}: late-note RMS collapsed to {late:.4} (early {early:.4})"
            );
            assert!(
                late > early * 0.5,
                "note {note}: late RMS {late:.4} fell below half of early RMS {early:.4} \
                 — grains are comb-filtering instead of sustaining"
            );
        }
    }

    /// Render a held note at an explicit render sample rate, driven purely
    /// through ProcessContext (never via set_sample_rate) — mirroring how the
    /// engine feeds voice-graph modules.
    fn render_note_at(note: u8, rate: SampleRate, blocks: usize) -> Vec<f32> {
        const BLOCK: usize = 512;
        let mut osc = GranularOsc::new();
        for p in [
            Param::GranularOsc(GranularParam::Source(GrainSource::Sine)),
            Param::GranularOsc(GranularParam::Density(NormalizedValue::new(1.0))),
            Param::GranularOsc(GranularParam::GrainSize(Milliseconds::new(150.0))),
            Param::GranularOsc(GranularParam::Position(NormalizedValue::MIN)),
            Param::GranularOsc(GranularParam::PositionSpread(NormalizedValue::MIN)),
            Param::GranularOsc(GranularParam::PitchSpread(NormalizedValue::MIN)),
        ] {
            osc.set_param(p);
        }
        osc.note_on(MidiNote::new(note), Velocity::new(1.0));

        let ctx = ProcessContext {
            samples: synth_core::SampleCount::new(BLOCK),
            sample_rate: rate,
            ..ProcessContext::default()
        };
        let mut out = HashMap::new();
        out.insert(PortName::OUT, AudioBuffer::new(BLOCK));

        let mut samples = Vec::with_capacity(BLOCK * blocks);
        for _ in 0..blocks {
            osc.process(InputPorts::empty(), &mut out, &ctx);
            let buf = &out[&PortName::OUT];
            for i in 0..BLOCK {
                samples.push(buf[i]);
            }
        }
        samples
    }

    /// Upward-zero-crossing pitch estimate over the steady-state middle half.
    fn dominant_freq_hz(buf: &[f32], rate: f32) -> f32 {
        let seg = &buf[buf.len() / 4..buf.len() * 3 / 4];
        let crossings = seg.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        let seconds = seg.len() as f32 / rate;
        crossings as f32 / seconds
    }

    /// Pitch must not depend on the render sample rate. The source buffer is
    /// filled at one rate, but the engine only hands voice modules the render
    /// rate through ProcessContext (it never calls set_sample_rate on them).
    /// Regression: an offline render at 44.1 kHz of a buffer left at the 48 kHz
    /// default played C4 ~147 cents flat.
    #[test]
    fn pitch_is_sample_rate_independent() {
        let expected = 261.626_f32; // C4
        for rate in [SampleRate::DVD_QUALITY, SampleRate::new(44_100.0)] {
            let buf = render_note_at(60, rate, 60);
            let f = dominant_freq_hz(&buf, rate.as_f32());
            let cents = 1200.0 * (f / expected).log2();
            assert!(
                cents.abs() < 50.0,
                "C4 rendered at {} Hz read as {f:.1} Hz ({cents:+.0} cents)",
                rate.as_f32()
            );
        }
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
