//! Spatial Panner — per-voice binaural positioning module.
//!
//! Positions a mono voice in a fixed virtual room by summing two parallel
//! DSP paths on the same dry signal (the standard pro spatial-reverb split):
//!
//! - a **binaural direct path** ([`Spatializer`]): ITD / ILD / head-shadow,
//! - **early reflections** ([`EarlyReflections`]): ISM taps from six walls.
//!
//! Both take mono in → stereo out and their outputs are summed (never chained
//! in series — that would collapse the direct path's stereo back to mono).
//! This is the salvaged, per-voice form of the retired AWE monolith's spatial
//! stage, now a first-class `PolyModule` so it lives in the patch graph and its
//! position (`x`/`y`/`z`) is Mod-Matrix / YAMS modulatable per voice.
//!
//! The room dimensions are fixed; `x`/`y`/`z` are bipolar offsets of the source
//! from the room centre (the listener position).

mod early_reflections;
mod spatializer;

use std::collections::HashMap;

use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, OnePoleSmooth,
    ParamModOffsets, ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor,
    ProcessContext, WidgetHint,
};
use synth_core::{
    BipolarValue, Meters, MetersPerSecond, ModuleType, NormalizedValue, Param, PortName, Position3,
    SampleCount, SampleRate, Seconds, SpatialPannerParam,
};

use early_reflections::EarlyReflections;
use spatializer::Spatializer;

/// Fixed virtual room dimensions (meters): length (X), width (Y), height (Z).
const ROOM_LENGTH_M: f32 = 12.0;
const ROOM_WIDTH_M: f32 = 9.0;
const ROOM_HEIGHT_M: f32 = 4.0;

/// Room centre (listener position).
const CENTER_X_M: f32 = ROOM_LENGTH_M * 0.5;
const CENTER_Y_M: f32 = ROOM_WIDTH_M * 0.5;
const CENTER_Z_M: f32 = ROOM_HEIGHT_M * 0.5;

/// Half-extent the bipolar `x`/`y`/`z` params map onto (leaves a 0.5 m margin
/// so a full-scale offset never lands the source exactly on a wall).
const X_SPAN_M: f32 = CENTER_X_M - 0.5;
const Y_SPAN_M: f32 = CENTER_Y_M - 0.5;
const Z_SPAN_M: f32 = CENTER_Z_M - 0.5;

/// Speed of sound at ~20 °C.
const SPEED_OF_SOUND: MetersPerSecond = MetersPerSecond::new(343.0);

/// Per-voice early-reflection delay line size (~170 ms at 96 kHz).
const PER_VOICE_MAX_DELAY: SampleCount = SampleCount::new(16_384);

/// Time constant for smoothing the ER / direct mix levels (~5 ms).
const LEVEL_SMOOTH_SECONDS: Seconds = Seconds::new(0.005);

/// Per-voice binaural spatial panner.
#[derive(Clone)]
pub struct SpatialPanner {
    // Parameters (bipolar position offsets + normalized amounts).
    x: BipolarValue,
    y: BipolarValue,
    z: BipolarValue,
    diffusion: NormalizedValue,
    er_level: NormalizedValue,
    direct_level: NormalizedValue,
    absorption: NormalizedValue,
    air_absorption: NormalizedValue,

    // DSP (kept as two independent halves per the AWE-decomposition design).
    early: EarlyReflections,
    spatializer: Spatializer,

    // Smoothed mix levels (guard against zipper noise when levels are modulated).
    er_level_smooth: OnePoleSmooth,
    direct_level_smooth: OnePoleSmooth,

    sample_rate: SampleRate,
    /// Generic mod-matrix offsets (descriptor-driven). See [`ParamModOffsets`].
    mod_offsets: ParamModOffsets,

    output_left: AudioBuffer,
    output_right: AudioBuffer,
}

impl SpatialPanner {
    pub fn new() -> Self {
        let er_default = NormalizedValue::new(0.4);
        let direct_default = NormalizedValue::new(1.0);
        let mut er_level_smooth = OnePoleSmooth::new(LEVEL_SMOOTH_SECONDS, SampleRate::DVD_QUALITY);
        er_level_smooth.set(er_default.as_f32());
        let mut direct_level_smooth =
            OnePoleSmooth::new(LEVEL_SMOOTH_SECONDS, SampleRate::DVD_QUALITY);
        direct_level_smooth.set(direct_default.as_f32());

        Self {
            x: BipolarValue::CENTER,
            y: BipolarValue::new(0.4),
            z: BipolarValue::CENTER,
            diffusion: NormalizedValue::new(0.3),
            er_level: er_default,
            direct_level: direct_default,
            absorption: NormalizedValue::new(0.4),
            air_absorption: NormalizedValue::new(0.1),

            early: EarlyReflections::with_max_delay(PER_VOICE_MAX_DELAY),
            spatializer: Spatializer::new(),

            er_level_smooth,
            direct_level_smooth,

            sample_rate: SampleRate::DVD_QUALITY,
            mod_offsets: ParamModOffsets::new(),
            output_left: AudioBuffer::new(1024),
            output_right: AudioBuffer::new(1024),
        }
    }

    /// Recompute the source position and update both DSP halves from the
    /// (possibly modulated) parameters. Runs once per block (control rate).
    fn update_geometry(&mut self) {
        let ex = self.mod_offsets.effective("x", self.x.as_f32());
        let ey = self.mod_offsets.effective("y", self.y.as_f32());
        let ez = self.mod_offsets.effective("z", self.z.as_f32());
        let diffusion = self
            .mod_offsets
            .effective("diffusion", self.diffusion.as_f32());
        let absorption = self
            .mod_offsets
            .effective("absorption", self.absorption.as_f32());
        let air = self
            .mod_offsets
            .effective("air_absorption", self.air_absorption.as_f32());

        // Source = room centre + bipolar offset, kept just inside the walls.
        let sx = (CENTER_X_M + ex * X_SPAN_M).clamp(0.1, ROOM_LENGTH_M - 0.1);
        let sy = (CENTER_Y_M + ey * Y_SPAN_M).clamp(0.1, ROOM_WIDTH_M - 0.1);
        let sz = (CENTER_Z_M + ez * Z_SPAN_M).clamp(0.1, ROOM_HEIGHT_M - 0.1);
        let source = Position3::from([sx, sy, sz]);
        let listener = Position3::from([CENTER_X_M, CENTER_Y_M, CENTER_Z_M]);

        let abs = NormalizedValue::new(absorption);
        self.early.update_geometry(
            Meters::new(ROOM_LENGTH_M),
            Meters::new(ROOM_WIDTH_M),
            Meters::new(ROOM_HEIGHT_M),
            source,
            listener,
            abs,
            abs,
            abs,
            NormalizedValue::new(diffusion),
            NormalizedValue::new(air),
            SPEED_OF_SOUND,
            self.sample_rate,
        );
        self.spatializer
            .update(source, listener, SPEED_OF_SOUND, self.sample_rate);
    }
}

impl Default for SpatialPanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for SpatialPanner {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("spatial_panner", "Spatial Panner")
            .description(
                "Per-voice binaural positioning: ISM early reflections + ITD/ILD direct path",
            )
            .category(ModuleCategory::PhysicalModeling)
            .tag("panner")
            .tag("spatial")
            .tag("binaural")
            .tag("reflections")
            .parameter(
                ParameterDescriptor::float(
                    "x",
                    Param::SpatialPanner(SpatialPannerParam::X(BipolarValue::CENTER)),
                    "X",
                )
                .description("Left/right source offset (-1 = left, +1 = right)")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "y",
                    Param::SpatialPanner(SpatialPannerParam::Y(BipolarValue::new(0.4))),
                    "Y",
                )
                .description("Front/back source offset (-1 = behind, +1 = in front)")
                .range(-1.0, 1.0)
                .default(0.4)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "z",
                    Param::SpatialPanner(SpatialPannerParam::Z(BipolarValue::CENTER)),
                    "Z",
                )
                .description("Down/up source offset (-1 = below, +1 = above)")
                .range(-1.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "diffusion",
                    Param::SpatialPanner(SpatialPannerParam::Diffusion(NormalizedValue::new(0.3))),
                    "Diffusion",
                )
                .description("Scattering of the reflections (0 = specular, 1 = diffuse)")
                .range(0.0, 1.0)
                .default(0.3)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "er_level",
                    Param::SpatialPanner(SpatialPannerParam::ErLevel(NormalizedValue::new(0.4))),
                    "ER Level",
                )
                .description("Early-reflection (room) level")
                .range(0.0, 1.0)
                .default(0.4)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "direct_level",
                    Param::SpatialPanner(SpatialPannerParam::DirectLevel(NormalizedValue::new(
                        1.0,
                    ))),
                    "Direct Level",
                )
                .description("Direct (binaural) path level")
                .range(0.0, 1.0)
                .default(1.0)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "absorption",
                    Param::SpatialPanner(SpatialPannerParam::Absorption(NormalizedValue::new(0.4))),
                    "Absorption",
                )
                .description("Wall absorption (0 = reflective, 1 = dead)")
                .range(0.0, 1.0)
                .default(0.4)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "air_absorption",
                    Param::SpatialPanner(SpatialPannerParam::AirAbsorption(NormalizedValue::new(
                        0.1,
                    ))),
                    "Air Absorption",
                )
                .description("High-frequency damping over distance")
                .range(0.0, 1.0)
                .default(0.1)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .port(PortDescriptor::audio_input("in", "In").description("Mono input"))
            .port(PortDescriptor::audio_output("out_l", "Out L").description("Left output"))
            .port(PortDescriptor::audio_output("out_r", "Out R").description("Right output"))
    }
}

impl PolyModule for SpatialPanner {
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        self.sample_rate = context.sample_rate;
        self.er_level_smooth
            .set_time(LEVEL_SMOOTH_SECONDS, context.sample_rate);
        self.direct_level_smooth
            .set_time(LEVEL_SMOOTH_SECONDS, context.sample_rate);
        let frames = context.samples.as_usize();
        self.output_left.resize(frames);
        self.output_right.resize(frames);

        // Control-rate: recompute source geometry and per-ear targets once.
        self.update_geometry();
        let target_er = self
            .mod_offsets
            .effective("er_level", self.er_level.as_f32());
        let target_direct = self
            .mod_offsets
            .effective("direct_level", self.direct_level.as_f32());

        let input = inputs.get(PortName::IN);

        for i in 0..frames {
            let sample = input.map_or(0.0, |buf| buf[i]);

            // Smooth the mix levels to avoid zipper noise on fast modulation.
            let er_level = self.er_level_smooth.process(target_er);
            let direct_level = self.direct_level_smooth.process(target_direct);

            // Parallel: direct binaural path + early reflections, summed.
            let (spat_l, spat_r) = self.spatializer.process(sample);
            let (er_l, er_r) = self.early.process(sample);

            self.output_left[i] = direct_level * spat_l + er_level * er_l;
            self.output_right[i] = direct_level * spat_r + er_level * er_r;
        }

        if let Some(out_l) = outputs.get_mut(&PortName::OUT_L) {
            out_l.copy_from(&self.output_left);
        }
        if let Some(out_r) = outputs.get_mut(&PortName::OUT_R) {
            out_r.copy_from(&self.output_right);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::SpatialPanner(p) = param {
            match p {
                SpatialPannerParam::X(v) => self.x = v,
                SpatialPannerParam::Y(v) => self.y = v,
                SpatialPannerParam::Z(v) => self.z = v,
                SpatialPannerParam::Diffusion(v) => self.diffusion = v,
                SpatialPannerParam::ErLevel(v) => self.er_level = v,
                SpatialPannerParam::DirectLevel(v) => self.direct_level = v,
                SpatialPannerParam::Absorption(v) => self.absorption = v,
                SpatialPannerParam::AirAbsorption(v) => self.air_absorption = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::SpatialPanner(p) = param {
            Some(match p {
                SpatialPannerParam::X(_) => self.x.as_f32(),
                SpatialPannerParam::Y(_) => self.y.as_f32(),
                SpatialPannerParam::Z(_) => self.z.as_f32(),
                SpatialPannerParam::Diffusion(_) => self.diffusion.as_f32(),
                SpatialPannerParam::ErLevel(_) => self.er_level.as_f32(),
                SpatialPannerParam::DirectLevel(_) => self.direct_level.as_f32(),
                SpatialPannerParam::Absorption(_) => self.absorption.as_f32(),
                SpatialPannerParam::AirAbsorption(_) => self.air_absorption.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::SpatialPanner(SpatialPannerParam::X(self.x)),
            Param::SpatialPanner(SpatialPannerParam::Y(self.y)),
            Param::SpatialPanner(SpatialPannerParam::Z(self.z)),
            Param::SpatialPanner(SpatialPannerParam::Diffusion(self.diffusion)),
            Param::SpatialPanner(SpatialPannerParam::ErLevel(self.er_level)),
            Param::SpatialPanner(SpatialPannerParam::DirectLevel(self.direct_level)),
            Param::SpatialPanner(SpatialPannerParam::Absorption(self.absorption)),
            Param::SpatialPanner(SpatialPannerParam::AirAbsorption(self.air_absorption)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::SpatialPanner
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn reset(&mut self) {
        self.early.clear();
        self.spatializer.clear();
        self.er_level_smooth.set(self.er_level.as_f32());
        self.direct_level_smooth.set(self.direct_level.as_f32());
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
        self.er_level_smooth
            .set_time(LEVEL_SMOOTH_SECONDS, sample_rate);
        self.direct_level_smooth
            .set_time(LEVEL_SMOOTH_SECONDS, sample_rate);
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_and_run(sp: &mut SpatialPanner, input: &[f32]) -> (AudioBuffer, AudioBuffer) {
        let frames = input.len();
        let mut in_buf = AudioBuffer::new(frames);
        in_buf.as_mut_slice().copy_from_slice(input);
        let in_ports = [(PortName::IN, &in_buf)];
        let inputs = InputPorts::new(&in_ports);

        let mut outputs: HashMap<PortName, AudioBuffer> = HashMap::new();
        outputs.insert(PortName::OUT_L, AudioBuffer::new(frames));
        outputs.insert(PortName::OUT_R, AudioBuffer::new(frames));

        let context = ProcessContext {
            sample_rate: SampleRate::DVD_QUALITY,
            samples: SampleCount::new(frames),
            ..ProcessContext::default()
        };
        sp.process(inputs, &mut outputs, &context);
        (
            outputs.remove(&PortName::OUT_L).expect("left out"),
            outputs.remove(&PortName::OUT_R).expect("right out"),
        )
    }

    #[test]
    fn produces_stereo_output_from_impulse() {
        let mut sp = SpatialPanner::new();
        let desc = sp.descriptor();
        sp.mod_offsets_mut().expect("offsets").populate(&desc);

        let mut input = vec![0.0_f32; 512];
        input[0] = 1.0;
        let (l, r) = ctx_and_run(&mut sp, &input);

        let max_l = (0..l.len()).map(|i| l[i].abs()).fold(0.0_f32, f32::max);
        let max_r = (0..r.len()).map(|i| r[i].abs()).fold(0.0_f32, f32::max);
        assert!(max_l > 0.0, "left channel should have output");
        assert!(max_r > 0.0, "right channel should have output");
    }

    #[test]
    fn x_modulation_shifts_stereo_balance() {
        // Hard-left vs hard-right x should move energy toward that channel.
        let energy = |x_off: f32| -> (f32, f32) {
            let mut sp = SpatialPanner::new();
            let desc = sp.descriptor();
            sp.mod_offsets_mut().expect("offsets").populate(&desc);
            sp.set_param(Param::SpatialPanner(SpatialPannerParam::X(
                BipolarValue::new(x_off),
            )));
            // Kill the reflection path so we measure the binaural direct path only.
            sp.set_param(Param::SpatialPanner(SpatialPannerParam::ErLevel(
                NormalizedValue::MIN,
            )));

            let mut input = vec![0.0_f32; 4096];
            for (i, s) in input.iter_mut().enumerate() {
                *s = if i % 2 == 0 { 0.5 } else { -0.5 };
            }
            let (l, r) = ctx_and_run(&mut sp, &input);
            let el: f32 = (0..l.len()).map(|i| l[i] * l[i]).sum();
            let er: f32 = (0..r.len()).map(|i| r[i] * r[i]).sum();
            (el, er)
        };

        let (left_l, left_r) = energy(-1.0);
        let (right_l, right_r) = energy(1.0);
        assert!(left_l > left_r, "x=-1 should favor the left channel");
        assert!(right_r > right_l, "x=+1 should favor the right channel");
    }

    #[test]
    fn silence_in_silence_out() {
        let mut sp = SpatialPanner::new();
        let (l, r) = ctx_and_run(&mut sp, &vec![0.0_f32; 256]);
        for i in 0..l.len() {
            assert!(l[i].abs() < 1e-6 && r[i].abs() < 1e-6);
        }
    }
}
