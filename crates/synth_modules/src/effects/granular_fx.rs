//! Input-driven granular FX processor.
//!
//! Features:
//! - Stereo ring buffer capture
//! - Pre-allocated grain pool (max 64 grains)
//! - Hann-windowed grains with position/pitch/pan spread
//! - Freeze mode (stops writing to buffer)

use synth_core::NoiseState;
use synth_core::{
    AudioEffect, BipolarValue, Describable, GranularFxParam, Milliseconds, ModuleCategory,
    ModuleDescriptor, ModuleType, NormalizedValue, Param, ParameterDescriptor, ParameterUnit,
    ProcessContext, SampleRate, Seconds, StereoSample, WidgetHint,
};

const MAX_GRAINS: usize = 64;
const MAX_BUFFER_SECS: f32 = 4.0;
const MIN_GRAIN_SAMPLES: usize = 16;

#[derive(Clone, Copy)]
struct Grain {
    active: bool,
    start_idx: usize,
    length: usize,
    pos: f32,
    rate: f32,
    pan: BipolarValue,
    age: usize,
}

impl Default for Grain {
    fn default() -> Self {
        Self {
            active: false,
            start_idx: 0,
            length: MIN_GRAIN_SAMPLES,
            pos: 0.0,
            rate: 1.0,
            pan: BipolarValue::CENTER,
            age: 0,
        }
    }
}

/// Input-driven granular FX.
pub struct GranularFx {
    // Parameters
    buffer_time: Seconds,
    grain_size: Milliseconds,
    density: NormalizedValue,
    position: NormalizedValue,
    position_spread: NormalizedValue,
    pitch_spread: NormalizedValue,
    pan_spread: NormalizedValue,
    freeze: bool,
    mix: NormalizedValue,

    // Stereo ring buffer
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    write_pos: usize,
    buffer_len: usize,

    // Grain pool
    grains: [Grain; MAX_GRAINS],

    // Spawn timing
    spawn_counter: f32,
    spawn_interval: f32,

    noise: NoiseState,
    sample_rate: SampleRate,
}

impl GranularFx {
    pub fn new() -> Self {
        let buf_size = (MAX_BUFFER_SECS * 48000.0) as usize;
        Self {
            buffer_time: Seconds::new(2.0),
            grain_size: Milliseconds::new(40.0),
            density: NormalizedValue::new(0.6),
            position: NormalizedValue::CENTER,
            position_spread: NormalizedValue::new(0.3),
            pitch_spread: NormalizedValue::new(0.2),
            pan_spread: NormalizedValue::CENTER,
            freeze: false,
            mix: NormalizedValue::MAX,

            buffer_l: vec![0.0; buf_size],
            buffer_r: vec![0.0; buf_size],
            write_pos: 0,
            buffer_len: buf_size,

            grains: [Grain::default(); MAX_GRAINS],
            spawn_counter: 0.0,
            spawn_interval: 1000.0,

            noise: NoiseState::DEFAULT,
            sample_rate: SampleRate::DVD_QUALITY,
        }
    }

    fn update_buffer_len(&mut self) {
        #[allow(clippy::cast_possible_truncation)]
        let len = (self.buffer_time.as_f32() * self.sample_rate.as_f32()) as usize;
        self.buffer_len = len.clamp(MIN_GRAIN_SAMPLES * 2, self.buffer_l.len());
    }

    fn spawn_grain(&mut self) {
        // Find inactive slot
        let slot = self.grains.iter_mut().find(|g| !g.active);
        let Some(grain) = slot else { return };

        #[allow(clippy::cast_possible_truncation)]
        let grain_len = (self.grain_size.as_f32() / 1000.0 * self.sample_rate.as_f32()) as usize;
        let grain_len = grain_len.max(MIN_GRAIN_SAMPLES).min(self.buffer_len);

        // Position with spread
        let pos_base = self.position.as_f32();
        let spread = self.position_spread.as_f32();
        let pos = (pos_base + self.noise.next() * spread * 0.5).clamp(0.0, 1.0);

        #[allow(clippy::cast_possible_truncation)]
        let start = (pos * (self.buffer_len - grain_len) as f32) as usize;

        // Pitch with spread
        let pitch_rng = self.noise.next() * self.pitch_spread.as_f32();
        let rate = 2.0f32.powf(pitch_rng);

        // Pan
        let pan = BipolarValue::new(self.noise.next() * self.pan_spread.as_f32());

        *grain = Grain {
            active: true,
            start_idx: start,
            length: grain_len,
            pos: 0.0,
            rate,
            pan,
            age: 0,
        };
    }

    #[inline]
    fn read_buffer(buffer: &[f32], buf_len: usize, idx: f32) -> f32 {
        let wrapped = idx.rem_euclid(buf_len as f32);
        let i0 = (wrapped as usize) % buf_len;
        let i1 = (i0 + 1) % buf_len;
        let frac = wrapped - wrapped.floor();
        buffer[i0] * (1.0 - frac) + buffer[i1] * frac
    }
}

impl Default for GranularFx {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for GranularFx {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("granular_fx", "Granular FX")
            .description("Input-driven granular cloud processor")
            .category(ModuleCategory::Effect)
            .tag("granular")
            .tag("effect")
            .tag("cloud")
            .parameter(
                ParameterDescriptor::float(
                    "buffer",
                    Param::GranularFx(GranularFxParam::BufferTime(Seconds::new(2.0))),
                    "Buffer",
                )
                .description("Capture buffer length")
                .range(0.5, 4.0)
                .default(2.0)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "grain_size",
                    Param::GranularFx(GranularFxParam::GrainSize(Milliseconds::new(40.0))),
                    "Grain Size",
                )
                .description("Length of each grain in milliseconds")
                .range(10.0, 120.0)
                .default(40.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "density",
                    Param::GranularFx(GranularFxParam::Density(NormalizedValue::new(0.6))),
                    "Density",
                )
                .description("Grain trigger rate (0 = sparse, 1 = dense)")
                .range(0.0, 1.0)
                .default(0.6)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "position",
                    Param::GranularFx(GranularFxParam::Position(NormalizedValue::CENTER)),
                    "Position",
                )
                .description("Read offset into the capture buffer (0 = newest, 1 = oldest)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pos_spread",
                    Param::GranularFx(GranularFxParam::PositionSpread(NormalizedValue::new(0.3))),
                    "Pos Spread",
                )
                .description("Random offset around the read position per grain")
                .range(0.0, 1.0)
                .default(0.3)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pitch_spread",
                    Param::GranularFx(GranularFxParam::PitchSpread(NormalizedValue::new(0.2))),
                    "Pitch Spread",
                )
                .description("Random pitch variation per grain")
                .range(0.0, 1.0)
                .default(0.2)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pan_spread",
                    Param::GranularFx(GranularFxParam::PanSpread(NormalizedValue::CENTER)),
                    "Pan Spread",
                )
                .description("Random stereo pan per grain (0 = mono, 1 = full)")
                .range(0.0, 1.0)
                .default(0.5)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "freeze",
                    Param::GranularFx(GranularFxParam::Freeze(false)),
                    "Freeze",
                )
                .description("Stop writing to buffer")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::GranularFx(GranularFxParam::Mix(NormalizedValue::MAX)),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for GranularFx {
    #[allow(clippy::too_many_lines)]
    fn process(&mut self, input: &[f32], output: &mut [f32], context: &ProcessContext<'_>) {
        let sr = self.sample_rate.as_f32();
        let grains_per_sec = 1.0 + self.density.as_f32() * 59.0;
        self.spawn_interval = sr / grains_per_sec;
        let mix = self.mix.as_f32();

        for frame in 0..context.samples.as_usize() {
            let dry = StereoSample::read_frame(input, frame);
            let dry_l = dry.left;
            let dry_r = dry.right;

            // Write to ring buffer (unless frozen)
            if !self.freeze {
                self.buffer_l[self.write_pos % self.buffer_len] = dry_l;
                self.buffer_r[self.write_pos % self.buffer_len] = dry_r;
                self.write_pos = (self.write_pos + 1) % self.buffer_len;
            }

            // Spawn grains
            self.spawn_counter += 1.0;
            if self.spawn_counter >= self.spawn_interval {
                self.spawn_counter -= self.spawn_interval;
                self.spawn_grain();
            }

            // Sum active grains
            let mut wet_l = 0.0f32;
            let mut wet_r = 0.0f32;

            for grain in &mut self.grains {
                if !grain.active {
                    continue;
                }

                // Hann window envelope
                let t = grain.pos / grain.length as f32;
                if t >= 1.0 {
                    grain.active = false;
                    continue;
                }
                let env = 0.5 * (1.0 - (t * std::f32::consts::TAU).cos());

                // Read from buffer with interpolation
                let read_idx = grain.start_idx as f32 + grain.pos * grain.rate;
                let sample_l = Self::read_buffer(&self.buffer_l, self.buffer_len, read_idx);
                let sample_r = Self::read_buffer(&self.buffer_r, self.buffer_len, read_idx);

                // Apply envelope and pan
                let gain_l = (0.5_f32 - grain.pan.as_f32() * 0.5).sqrt();
                let gain_r = (0.5_f32 + grain.pan.as_f32() * 0.5).sqrt();

                wet_l += sample_l * env * gain_l;
                wet_r += sample_r * env * gain_r;

                grain.pos += 1.0;
                grain.age += 1;
            }

            // Mix
            let result =
                StereoSample::new(dry_l, dry_r).blend(StereoSample::new(wet_l, wet_r), mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn reset(&mut self) {
        self.buffer_l.fill(0.0);
        self.buffer_r.fill(0.0);
        self.write_pos = 0;
        self.grains = [Grain::default(); MAX_GRAINS];
        self.spawn_counter = 0.0;
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }
    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn set_param(&mut self, param: Param) {
        if let Param::GranularFx(p) = param {
            match p {
                GranularFxParam::BufferTime(v) => {
                    self.buffer_time = Seconds::new(v.as_f32().clamp(0.5, MAX_BUFFER_SECS));
                    self.update_buffer_len();
                }
                GranularFxParam::GrainSize(v) => {
                    self.grain_size = Milliseconds::new(v.as_f32().clamp(10.0, 120.0));
                }
                GranularFxParam::Density(v) => self.density = v,
                GranularFxParam::Position(v) => self.position = v,
                GranularFxParam::PositionSpread(v) => self.position_spread = v,
                GranularFxParam::PitchSpread(v) => self.pitch_spread = v,
                GranularFxParam::PanSpread(v) => self.pan_spread = v,
                GranularFxParam::Freeze(v) => self.freeze = v,
                GranularFxParam::Mix(v) => self.mix = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::GranularFx(p) = param {
            Some(match p {
                GranularFxParam::BufferTime(_) => self.buffer_time.as_f32(),
                GranularFxParam::GrainSize(_) => self.grain_size.as_f32(),
                GranularFxParam::Density(_) => self.density.as_f32(),
                GranularFxParam::Position(_) => self.position.as_f32(),
                GranularFxParam::PositionSpread(_) => self.position_spread.as_f32(),
                GranularFxParam::PitchSpread(_) => self.pitch_spread.as_f32(),
                GranularFxParam::PanSpread(_) => self.pan_spread.as_f32(),
                GranularFxParam::Freeze(_) => {
                    if self.freeze {
                        1.0
                    } else {
                        0.0
                    }
                }
                GranularFxParam::Mix(_) => self.mix.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::GranularFx(GranularFxParam::BufferTime(self.buffer_time)),
            Param::GranularFx(GranularFxParam::GrainSize(self.grain_size)),
            Param::GranularFx(GranularFxParam::Density(self.density)),
            Param::GranularFx(GranularFxParam::Position(self.position)),
            Param::GranularFx(GranularFxParam::PositionSpread(self.position_spread)),
            Param::GranularFx(GranularFxParam::PitchSpread(self.pitch_spread)),
            Param::GranularFx(GranularFxParam::PanSpread(self.pan_spread)),
            Param::GranularFx(GranularFxParam::Freeze(self.freeze)),
            Param::GranularFx(GranularFxParam::Mix(self.mix)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::GranularFx
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        let buf_size = (MAX_BUFFER_SECS * sample_rate.as_f32()) as usize;
        if self.buffer_l.len() != buf_size {
            self.buffer_l.resize(buf_size, 0.0);
            self.buffer_r.resize(buf_size, 0.0);
            self.write_pos = 0;
        }
        self.update_buffer_len();
    }
}
