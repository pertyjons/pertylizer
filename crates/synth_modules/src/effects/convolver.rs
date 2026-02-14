//! Convolution reverb effect.
//!
//! Features:
//! - Partitioned convolution via FFT for efficient long-IR processing
//! - Four mathematically generated impulse responses (Plate/Room/Spring/Hall)
//! - Pre-delay, decay trim, brightness controls
//! - Stereo processing (dual convolver)

use synth_core::module_traits::ChoiceOption;
use synth_core::{
    AudioEffect, ConvolverParam, Describable, ImpulseResponse, Milliseconds, ModuleCategory,
    ModuleDescriptor, ModuleType, NormalizedValue, Param, ParameterDescriptor, ParameterUnit,
    ProcessContext, SampleCount, SampleRate, WidgetHint,
};
use synth_dsp::PartitionedConvolver;

/// Partition size for convolution FFT.
const PARTITION_SIZE: usize = 512;

/// Maximum IR length in samples (~2s at 48kHz).
const MAX_IR_SAMPLES: usize = 96_000;

/// Convolution reverb with generated impulse responses.
pub struct Convolver {
    // Parameters
    ir_type: ImpulseResponse,
    mix: NormalizedValue,
    pre_delay_ms: Milliseconds,
    decay_trim: NormalizedValue,
    brightness: NormalizedValue,

    // Convolution engines (stereo)
    conv_left: PartitionedConvolver,
    conv_right: PartitionedConvolver,

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

    // State
    sample_rate: SampleRate,
    ir_dirty: bool,
}

impl Convolver {
    pub fn new() -> Self {
        let ir = Self::generate_ir(ImpulseResponse::Plate, SampleRate::DVD_QUALITY, 1.0);
        Self {
            ir_type: ImpulseResponse::Plate,
            mix: NormalizedValue::new(0.3),
            pre_delay_ms: Milliseconds::new(0.0),
            decay_trim: NormalizedValue::MAX,
            brightness: NormalizedValue::new(0.8),

            conv_left: PartitionedConvolver::new(PARTITION_SIZE, &ir),
            conv_right: PartitionedConvolver::new(PARTITION_SIZE, &ir),

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

            sample_rate: SampleRate::DVD_QUALITY,
            ir_dirty: false,
        }
    }

    /// Generate a synthetic impulse response.
    fn generate_ir(ir_type: ImpulseResponse, sample_rate: SampleRate, decay_trim: f32) -> Vec<f32> {
        let sr = sample_rate.as_f32();
        let (duration, decay_rate, character) = match ir_type {
            ImpulseResponse::Plate => (1.5, 3.0, 0.8),
            ImpulseResponse::Room => (0.8, 5.0, 0.5),
            ImpulseResponse::Spring => (1.0, 4.0, 0.95),
            ImpulseResponse::Hall => (2.5, 1.5, 0.3),
        };

        let trimmed_duration = duration * decay_trim;
        let len = ((trimmed_duration * sr) as usize).min(MAX_IR_SAMPLES);
        let mut ir = vec![0.0f32; len];

        // Simple deterministic noise with exponential decay
        let mut rng_state: u32 = 0x1234_5678;
        for i in 0..len {
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

            ir[i] = noise * env * early;
        }

        ir
    }

    fn rebuild_ir(&mut self) {
        self.ir_dirty = false;
        let ir = Self::generate_ir(self.ir_type, self.sample_rate, self.decay_trim.as_f32());
        self.conv_left = PartitionedConvolver::new(PARTITION_SIZE, &ir);
        self.conv_right = PartitionedConvolver::new(PARTITION_SIZE, &ir);
        self.accum_pos = 0;
        self.input_accum_l.fill(0.0);
        self.input_accum_r.fill(0.0);
        self.output_accum_l.fill(0.0);
        self.output_accum_r.fill(0.0);
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
            .parameter(ParameterDescriptor::choice(
                Param::Convolver(ConvolverParam::Ir(ImpulseResponse::Plate)),
                "IR Type",
                ImpulseResponse::ALL
                    .iter()
                    .map(|i| ChoiceOption::new(i.id(), i.name()))
                    .collect(),
            ))
            .parameter(
                ParameterDescriptor::float(
                    Param::Convolver(ConvolverParam::Mix(NormalizedValue::new(0.3))),
                    "Mix",
                )
                .range(0.0, 1.0)
                .default(0.3)
                .unit(ParameterUnit::Percent)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Convolver(ConvolverParam::PreDelay(Milliseconds::new(0.0))),
                    "Pre-Delay",
                )
                .range(0.0, 200.0)
                .default(0.0)
                .unit(ParameterUnit::Milliseconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Convolver(ConvolverParam::DecayTrim(NormalizedValue::MAX)),
                    "Decay",
                )
                .range(0.1, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    Param::Convolver(ConvolverParam::Brightness(NormalizedValue::new(0.8))),
                    "Brightness",
                )
                .range(0.0, 1.0)
                .default(0.8)
                .widget(WidgetHint::Knob),
            )
    }
}

impl AudioEffect for Convolver {
    #[allow(clippy::too_many_lines)]
    fn process(&mut self, input: &[f32], output: &mut [f32], _context: &ProcessContext) {
        if self.ir_dirty {
            self.rebuild_ir();
        }

        let num_frames = input.len() / 2;
        let mix = self.mix.as_f32();
        let dry = 1.0 - mix;

        // One-pole LP coefficient for brightness
        let brightness = self.brightness.as_f32();
        let lp_coeff = brightness * brightness; // 0=dark, 1=bright (bypass)

        for frame in 0..num_frames {
            let in_l = input[frame * 2];
            let in_r = input[frame * 2 + 1];

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
            if self.accum_pos >= PARTITION_SIZE {
                self.accum_pos = 0;
                self.conv_left
                    .process_block(&self.input_accum_l, &mut self.output_accum_l);
                self.conv_right
                    .process_block(&self.input_accum_r, &mut self.output_accum_r);
            }

            // Read convolved output (from most recent full block)
            let wet_l = if self.accum_pos > 0 && self.accum_pos <= PARTITION_SIZE {
                self.output_accum_l
                    .get(self.accum_pos.wrapping_sub(1))
                    .copied()
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            let wet_r = if self.accum_pos > 0 && self.accum_pos <= PARTITION_SIZE {
                self.output_accum_r
                    .get(self.accum_pos.wrapping_sub(1))
                    .copied()
                    .unwrap_or(0.0)
            } else {
                0.0
            };

            // Apply brightness (one-pole LP)
            self.lp_state_l += lp_coeff * (wet_l - self.lp_state_l);
            self.lp_state_r += lp_coeff * (wet_r - self.lp_state_r);
            let filtered_l = self.lp_state_l;
            let filtered_r = self.lp_state_r;

            // Mix dry/wet
            output[frame * 2] = in_l * dry + filtered_l * mix;
            output[frame * 2 + 1] = in_r * dry + filtered_r * mix;
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::Convolver(p) = param {
            match p {
                ConvolverParam::Ir(ir) => {
                    if ir != self.ir_type {
                        self.ir_type = ir;
                        self.ir_dirty = true;
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
                        self.ir_dirty = true;
                    }
                }
                ConvolverParam::Brightness(v) => self.brightness = v,
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
        ]
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::Convolver
    }

    fn reset(&mut self) {
        self.conv_left.reset();
        self.conv_right.reset();
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
            self.ir_dirty = true;
            self.update_delay();
        }
    }
}
