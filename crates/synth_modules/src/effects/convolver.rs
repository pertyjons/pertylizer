//! Convolution reverb effect.
//!
//! Features:
//! - Partitioned convolution via FFT for efficient long-IR processing
//! - Four mathematically generated impulse responses (Plate/Room/Spring/Hall)
//! - Pre-delay, decay trim, brightness controls
//! - Stereo processing (dual convolver)

use crate::math::{buffer_rms, dynamic_convolution_weights};
use synth_core::module_traits::ChoiceOption;
use synth_core::{
    AudioEffect, ConvolverParam, DecayTrim, Describable, Gain, ImpulseResponse, Milliseconds,
    ModuleCategory, ModuleDescriptor, ModuleType, NormalizedValue, Param, ParameterDescriptor,
    ParameterUnit, ProcessContext, SampleCount, SampleRate, StereoSample, WidgetHint,
};
use synth_dsp::PartitionedConvolver;

/// Partition size for convolution FFT.
const PARTITION_SIZE: usize = 512;

/// Maximum IR length in samples (~2s at 48kHz).
const MAX_IR_SAMPLES: usize = 96_000;

/// Convolution reverb with generated impulse responses.
///
/// Supports dynamic convolution: when `dynamic_mode` > 0, three IR variants
/// (soft/medium/loud) are convolved in parallel and crossfaded based on input
/// RMS using `buffer_rms` and `dynamic_convolution_weights`.
pub struct Convolver {
    // Parameters
    ir_type: ImpulseResponse,
    mix: NormalizedValue,
    pre_delay_ms: Milliseconds,
    decay_trim: DecayTrim,
    brightness: NormalizedValue,
    dynamic_mode: NormalizedValue,

    // Convolution engines (stereo) — medium IR
    conv_left: PartitionedConvolver,
    conv_right: PartitionedConvolver,

    // Dynamic convolution: soft IR (lower amplitude)
    conv_soft_l: PartitionedConvolver,
    conv_soft_r: PartitionedConvolver,
    // Dynamic convolution: loud IR (higher amplitude)
    conv_loud_l: PartitionedConvolver,
    conv_loud_r: PartitionedConvolver,

    // Pre-delay buffer
    delay_buf_l: Vec<f32>,
    delay_buf_r: Vec<f32>,
    delay_write: usize,
    delay_samples: usize,

    // One-pole LP for brightness
    lp_state_l: f32,
    lp_state_r: f32,

    // Accumulator for non-partition-aligned blocks
    input_accum_l: Vec<f32>,
    input_accum_r: Vec<f32>,
    output_accum_l: Vec<f32>,
    output_accum_r: Vec<f32>,
    accum_pos: usize,

    // Accumulators for dynamic mode
    output_soft_l: Vec<f32>,
    output_soft_r: Vec<f32>,
    output_loud_l: Vec<f32>,
    output_loud_r: Vec<f32>,

    // Cached dynamic weights (computed per block, not per sample)
    dyn_lo: usize,
    dyn_hi: usize,
    dyn_cf: f32,

    // Pre-allocated IR scratch buffers, reused by rebuild_ir to avoid
    // per-rebuild heap allocation on the audio thread. Each holds up to
    // MAX_IR_SAMPLES samples; rebuild_ir truncates/extends in place.
    ir_scratch: Vec<f32>,
    ir_soft_scratch: Vec<f32>,
    ir_loud_scratch: Vec<f32>,

    // State
    sample_rate: SampleRate,
}

impl Convolver {
    pub fn new() -> Self {
        let sr = SampleRate::DVD_QUALITY;
        let ir_type = ImpulseResponse::Plate;
        let mut ir = Vec::with_capacity(MAX_IR_SAMPLES);
        let mut ir_soft = Vec::with_capacity(MAX_IR_SAMPLES);
        let mut ir_loud = Vec::with_capacity(MAX_IR_SAMPLES);
        Self::fill_ir(&mut ir, ir_type, sr, 1.0);
        Self::fill_ir_soft(&mut ir_soft, ir_type, sr, 1.0);
        Self::fill_ir_loud(&mut ir_loud, ir_type, sr, 1.0);
        Self {
            ir_type,
            mix: NormalizedValue::new(0.3),
            pre_delay_ms: Milliseconds::new(0.0),
            decay_trim: DecayTrim::FULL,
            brightness: NormalizedValue::new(0.8),
            dynamic_mode: NormalizedValue::MIN,

            conv_left: PartitionedConvolver::new(PARTITION_SIZE, &ir),
            conv_right: PartitionedConvolver::new(PARTITION_SIZE, &ir),

            conv_soft_l: PartitionedConvolver::new(PARTITION_SIZE, &ir_soft),
            conv_soft_r: PartitionedConvolver::new(PARTITION_SIZE, &ir_soft),
            conv_loud_l: PartitionedConvolver::new(PARTITION_SIZE, &ir_loud),
            conv_loud_r: PartitionedConvolver::new(PARTITION_SIZE, &ir_loud),

            delay_buf_l: vec![0.0; 48_000],
            delay_buf_r: vec![0.0; 48_000],
            delay_write: 0,
            delay_samples: 0,

            lp_state_l: 0.0,
            lp_state_r: 0.0,

            input_accum_l: vec![0.0; PARTITION_SIZE],
            input_accum_r: vec![0.0; PARTITION_SIZE],
            output_accum_l: vec![0.0; PARTITION_SIZE],
            output_accum_r: vec![0.0; PARTITION_SIZE],
            accum_pos: 0,

            output_soft_l: vec![0.0; PARTITION_SIZE],
            output_soft_r: vec![0.0; PARTITION_SIZE],
            output_loud_l: vec![0.0; PARTITION_SIZE],
            output_loud_r: vec![0.0; PARTITION_SIZE],

            dyn_lo: 1,
            dyn_hi: 1,
            dyn_cf: 0.0,

            // Reuse the freshly generated IR buffers as the rebuild scratch,
            // each already grown to its starting length / capacity.
            ir_scratch: ir,
            ir_soft_scratch: ir_soft,
            ir_loud_scratch: ir_loud,

            sample_rate: sr,
        }
    }

    /// Fill `buf` with a synthetic medium-variant impulse response, reusing the
    /// buffer's allocation (cleared and resized in place; only grows the backing
    /// store if the trimmed length exceeds the current capacity).
    fn fill_ir(
        buf: &mut Vec<f32>,
        ir_type: ImpulseResponse,
        sample_rate: SampleRate,
        decay_trim: f32,
    ) {
        let sr = sample_rate.as_f32();
        let (duration, decay_rate, character) = match ir_type {
            ImpulseResponse::Plate => (1.5, 3.0, 0.8),
            ImpulseResponse::Room => (0.8, 5.0, 0.5),
            ImpulseResponse::Spring => (1.0, 4.0, 0.95),
            ImpulseResponse::Hall => (2.5, 1.5, 0.3),
        };

        let trimmed_duration = duration * decay_trim;
        let len = ((trimmed_duration * sr) as usize).min(MAX_IR_SAMPLES);
        buf.clear();
        buf.resize(len, 0.0);

        // Simple deterministic noise with exponential decay
        let mut rng_state: u32 = 0x1234_5678;
        for (i, sample) in buf.iter_mut().enumerate() {
            // xorshift32 PRNG
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 17;
            rng_state ^= rng_state << 5;
            let noise = (rng_state as f32 / u32::MAX as f32) * 2.0 - 1.0;

            let t = i as f32 / sr;
            let env = (-decay_rate * t).exp();

            // Character: high density reflections vs sparse
            let early = if t < 0.05 {
                let idx = (t * sr) as u32;
                if idx.is_multiple_of(3 + (character * 10.0) as u32) {
                    1.5
                } else {
                    0.3
                }
            } else {
                1.0
            };

            *sample = noise * env * early;
        }
    }

    /// Fill `buf` with the soft IR variant (shorter decay, lower amplitude),
    /// reusing the buffer's allocation.
    fn fill_ir_soft(
        buf: &mut Vec<f32>,
        ir_type: ImpulseResponse,
        sample_rate: SampleRate,
        decay_trim: f32,
    ) {
        Self::fill_ir(buf, ir_type, sample_rate, decay_trim);
        let sr = sample_rate.as_f32();
        for (i, sample) in buf.iter_mut().enumerate() {
            let extra_decay = (-2.0 * i as f32 / sr).exp();
            *sample *= 0.7 * extra_decay;
        }
    }

    /// Fill `buf` with the loud IR variant (slight saturation), reusing the
    /// buffer's allocation.
    fn fill_ir_loud(
        buf: &mut Vec<f32>,
        ir_type: ImpulseResponse,
        sample_rate: SampleRate,
        decay_trim: f32,
    ) {
        Self::fill_ir(buf, ir_type, sample_rate, decay_trim);
        for sample in buf.iter_mut() {
            let s = *sample * 1.3;
            *sample = s / (1.0 + s.abs());
        }
    }

    fn rebuild_ir(&mut self) {
        let decay = self.decay_trim.as_f32();
        // Regenerate the three IR variants into the pre-allocated scratch buffers
        // (no allocation once each buffer has reached its high-water length).
        Self::fill_ir(&mut self.ir_scratch, self.ir_type, self.sample_rate, decay);
        Self::fill_ir_soft(
            &mut self.ir_soft_scratch,
            self.ir_type,
            self.sample_rate,
            decay,
        );
        Self::fill_ir_loud(
            &mut self.ir_loud_scratch,
            self.ir_type,
            self.sample_rate,
            decay,
        );

        // Swap the IR into each convolver in place, reusing its FFT planner and
        // partition-spectra pool (allocation-free once warmed up).
        self.conv_left.update_ir(&self.ir_scratch);
        self.conv_right.update_ir(&self.ir_scratch);
        self.conv_soft_l.update_ir(&self.ir_soft_scratch);
        self.conv_soft_r.update_ir(&self.ir_soft_scratch);
        self.conv_loud_l.update_ir(&self.ir_loud_scratch);
        self.conv_loud_r.update_ir(&self.ir_loud_scratch);

        self.accum_pos = 0;
        self.input_accum_l.fill(0.0);
        self.input_accum_r.fill(0.0);
        self.output_accum_l.fill(0.0);
        self.output_accum_r.fill(0.0);
        self.output_soft_l.fill(0.0);
        self.output_soft_r.fill(0.0);
        self.output_loud_l.fill(0.0);
        self.output_loud_r.fill(0.0);
    }

    fn update_delay(&mut self) {
        let sr = self.sample_rate.as_f32();
        self.delay_samples =
            ((self.pre_delay_ms.as_f32() * 0.001 * sr) as usize).min(self.delay_buf_l.len() - 1);
    }
}

impl Default for Convolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for Convolver {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor::new("convolver", "Convolver")
            .description("Convolution reverb with generated impulse responses")
            .category(ModuleCategory::Effect)
            .tag("reverb")
            .tag("convolution")
            .tag("ir")
            .parameter(
                ParameterDescriptor::choice(
                    "ir_type",
                    Param::Convolver(ConvolverParam::Ir(ImpulseResponse::Plate)),
                    "IR Type",
                    ImpulseResponse::ALL
                        .iter()
                        .map(|i| {
                            ChoiceOption::new(i.id(), i.name()).with_description(i.description())
                        })
                        .collect(),
                )
                .description("Impulse response type (Plate, Room, Spring, Hall)"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "mix",
                    Param::Convolver(ConvolverParam::Mix(NormalizedValue::new(0.3))),
                    "Mix",
                )
                .description("Dry/wet mix")
                .range(0.0, 1.0)
                .default(0.3)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pre_delay",
                    Param::Convolver(ConvolverParam::PreDelay(Milliseconds::new(0.0))),
                    "Pre-Delay",
                )
                .description("Time before reverb onset")
                .range(0.0, 200.0)
                .default(0.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "decay",
                    Param::Convolver(ConvolverParam::DecayTrim(DecayTrim::FULL)),
                    "Decay",
                )
                .description("IR tail length trim")
                .range(0.1, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "brightness",
                    Param::Convolver(ConvolverParam::Brightness(NormalizedValue::new(0.8))),
                    "Brightness",
                )
                .description("High frequency content of reverb tail")
                .range(0.0, 1.0)
                .default(0.8)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "dynamic",
                    Param::Convolver(ConvolverParam::DynamicMode(NormalizedValue::MIN)),
                    "Dynamic",
                )
                .description("Dynamic convolution amount (amplitude-dependent IR)")
                .range(0.0, 1.0)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Convolver {
    #[allow(clippy::too_many_lines)]
    fn process(&mut self, input: &[f32], output: &mut [f32], _context: &ProcessContext<'_>) {
        let num_frames = input.len() / 2;
        let mix = self.mix.as_f32();

        // One-pole LP coefficient for brightness
        let brightness = self.brightness.as_f32();
        let lp_coeff = brightness * brightness; // 0=dark, 1=bright (bypass)

        for frame in 0..num_frames {
            let dry = StereoSample::read_frame(input, frame);
            let in_l = dry.left;
            let in_r = dry.right;

            // Write to pre-delay buffer
            let buf_len = self.delay_buf_l.len();
            self.delay_buf_l[self.delay_write % buf_len] = in_l;
            self.delay_buf_r[self.delay_write % buf_len] = in_r;

            // Read from pre-delay buffer
            let read_pos = (self.delay_write + buf_len - self.delay_samples) % buf_len;
            let delayed_l = self.delay_buf_l[read_pos];
            let delayed_r = self.delay_buf_r[read_pos];
            self.delay_write = (self.delay_write + 1) % buf_len;

            // Feed into convolution accumulator
            self.input_accum_l[self.accum_pos] = delayed_l;
            self.input_accum_r[self.accum_pos] = delayed_r;
            self.accum_pos += 1;

            // When we've accumulated a full partition, run convolution
            let dynamic = self.dynamic_mode.as_f32();
            if self.accum_pos >= PARTITION_SIZE {
                self.accum_pos = 0;
                self.conv_left
                    .process_block(&self.input_accum_l, &mut self.output_accum_l);
                self.conv_right
                    .process_block(&self.input_accum_r, &mut self.output_accum_r);

                if dynamic > 0.01 {
                    self.conv_soft_l
                        .process_block(&self.input_accum_l, &mut self.output_soft_l);
                    self.conv_soft_r
                        .process_block(&self.input_accum_r, &mut self.output_soft_r);
                    self.conv_loud_l
                        .process_block(&self.input_accum_l, &mut self.output_loud_l);
                    self.conv_loud_r
                        .process_block(&self.input_accum_r, &mut self.output_loud_r);

                    let rms = buffer_rms(&self.input_accum_l);
                    let levels = [Gain::new(0.0), Gain::new(0.33), Gain::new(0.66)];
                    let (lo, hi, crossfade) = dynamic_convolution_weights(rms, &levels);
                    self.dyn_lo = lo;
                    self.dyn_hi = hi;
                    self.dyn_cf = crossfade.as_f32();
                }
            }

            // Read convolved output (from most recent full block)
            let pos = self.accum_pos;
            let mut wet_l = if pos < PARTITION_SIZE {
                self.output_accum_l.get(pos).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            let mut wet_r = if pos < PARTITION_SIZE {
                self.output_accum_r.get(pos).copied().unwrap_or(0.0)
            } else {
                0.0
            };

            if dynamic > 0.01 && pos < PARTITION_SIZE {
                let cf = self.dyn_cf;
                let get = |idx: usize, buf_s: &[f32], buf_m: &[f32], buf_l: &[f32]| -> f32 {
                    match idx {
                        0 => buf_s.get(pos).copied().unwrap_or(0.0),
                        2 => buf_l.get(pos).copied().unwrap_or(0.0),
                        _ => buf_m.get(pos).copied().unwrap_or(0.0),
                    }
                };

                let lo_l = get(
                    self.dyn_lo,
                    &self.output_soft_l,
                    &self.output_accum_l,
                    &self.output_loud_l,
                );
                let hi_l = get(
                    self.dyn_hi,
                    &self.output_soft_l,
                    &self.output_accum_l,
                    &self.output_loud_l,
                );
                let lo_r = get(
                    self.dyn_lo,
                    &self.output_soft_r,
                    &self.output_accum_r,
                    &self.output_loud_r,
                );
                let hi_r = get(
                    self.dyn_hi,
                    &self.output_soft_r,
                    &self.output_accum_r,
                    &self.output_loud_r,
                );

                let dyn_l = lo_l * (1.0 - cf) + hi_l * cf;
                let dyn_r = lo_r * (1.0 - cf) + hi_r * cf;

                wet_l = wet_l * (1.0 - dynamic) + dyn_l * dynamic;
                wet_r = wet_r * (1.0 - dynamic) + dyn_r * dynamic;
            }

            // Apply brightness (one-pole LP)
            self.lp_state_l += lp_coeff * (wet_l - self.lp_state_l);
            self.lp_state_r += lp_coeff * (wet_r - self.lp_state_r);
            let filtered_l = self.lp_state_l;
            let filtered_r = self.lp_state_r;

            // Mix dry/wet
            let result =
                StereoSample::new(in_l, in_r).blend(StereoSample::new(filtered_l, filtered_r), mix);
            StereoSample::write_frame(output, frame, result);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Convolver(p) = param {
            match p {
                ConvolverParam::Ir(ir) => {
                    if ir != self.ir_type {
                        self.ir_type = ir;
                        self.rebuild_ir();
                    }
                }
                ConvolverParam::Mix(v) => self.mix = v,
                ConvolverParam::PreDelay(ms) => {
                    self.pre_delay_ms = ms;
                    self.update_delay();
                }
                ConvolverParam::DecayTrim(v) => {
                    if (v.as_f32() - self.decay_trim.as_f32()).abs() > 0.01 {
                        self.decay_trim = v;
                        self.rebuild_ir();
                    }
                }
                ConvolverParam::Brightness(v) => self.brightness = v,
                ConvolverParam::DynamicMode(v) => self.dynamic_mode = v,
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::Convolver(p) = param {
            #[allow(clippy::cast_precision_loss)]
            Some(match p {
                ConvolverParam::Ir(_) => self.ir_type.index() as f32,
                ConvolverParam::Mix(_) => self.mix.as_f32(),
                ConvolverParam::PreDelay(_) => self.pre_delay_ms.as_f32(),
                ConvolverParam::DecayTrim(_) => self.decay_trim.as_f32(),
                ConvolverParam::Brightness(_) => self.brightness.as_f32(),
                ConvolverParam::DynamicMode(_) => self.dynamic_mode.as_f32(),
            })
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        vec![
            Param::Convolver(ConvolverParam::Ir(self.ir_type)),
            Param::Convolver(ConvolverParam::Mix(self.mix)),
            Param::Convolver(ConvolverParam::PreDelay(self.pre_delay_ms)),
            Param::Convolver(ConvolverParam::DecayTrim(self.decay_trim)),
            Param::Convolver(ConvolverParam::Brightness(self.brightness)),
            Param::Convolver(ConvolverParam::DynamicMode(self.dynamic_mode)),
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Convolver
    }

    fn reset(&mut self) {
        self.conv_left.reset();
        self.conv_right.reset();
        self.conv_soft_l.reset();
        self.conv_soft_r.reset();
        self.conv_loud_l.reset();
        self.conv_loud_r.reset();
        self.delay_buf_l.fill(0.0);
        self.delay_buf_r.fill(0.0);
        self.delay_write = 0;
        self.lp_state_l = 0.0;
        self.lp_state_r = 0.0;
        self.accum_pos = 0;
        self.input_accum_l.fill(0.0);
        self.input_accum_r.fill(0.0);
        self.output_accum_l.fill(0.0);
        self.output_accum_r.fill(0.0);
        self.output_soft_l.fill(0.0);
        self.output_soft_r.fill(0.0);
        self.output_loud_l.fill(0.0);
        self.output_loud_r.fill(0.0);
    }

    fn set_mix(&mut self, mix: NormalizedValue) {
        self.mix = mix;
    }

    fn get_mix(&self) -> NormalizedValue {
        self.mix
    }

    fn tail_samples(&self) -> SampleCount {
        // Approximate: 2 seconds of tail
        SampleCount::new((self.sample_rate.as_f32() * 2.0) as usize)
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            self.rebuild_ir();
            self.update_delay();
        }
    }
}
