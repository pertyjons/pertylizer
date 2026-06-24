//! Convolution reverb effect.
//!
//! Features:
//! - Partitioned convolution via FFT for efficient long-IR processing
//! - Four mathematically generated impulse responses (Plate/Room/Spring/Hall)
//! - Pre-delay, decay trim, brightness controls
//! - Stereo processing (dual convolver)

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Producer, Split};

use crate::math::{buffer_rms, dynamic_convolution_weights};
use synth_core::module_traits::ChoiceOption;
use synth_core::{
    AudioEffect, ConvolverParam, DecayTrim, Describable, Gain, ImpulseResponse, Milliseconds,
    ModuleCategory, ModuleDescriptor, ModuleType, NormalizedValue, Param, ParameterDescriptor,
    ParameterUnit, ProcessContext, SampleCount, SampleRate, StereoSample, WidgetHint,
};
use synth_dsp::{IrSpectra, IrSpectraBuilder, PartitionedConvolver};

/// Partition size for convolution FFT.
const PARTITION_SIZE: usize = 512;

/// Maximum IR length in samples (~2s at 48kHz).
const MAX_IR_SAMPLES: usize = 96_000;

/// Worst-case partition count for an IR up to `MAX_IR_SAMPLES` long. Each
/// convolver pre-reserves this many partition-spectra buffers so an off-thread IR
/// swap never allocates on the audio thread.
const MAX_PARTITIONS: usize = MAX_IR_SAMPLES.div_ceil(PARTITION_SIZE);

/// Request sent to the IR-build worker thread: rebuild all six IR-variant spectra
/// for these parameters.
struct IrRequest {
    ir_type: ImpulseResponse,
    sample_rate: SampleRate,
    decay_trim: f32,
}

/// Freshly built IR partition spectra for all six convolvers (medium/soft/loud ×
/// L/R), handed back from the worker for a lock-free pointer swap on the audio
/// thread. After the swap each field holds the *old* (displaced) spectra and is
/// returned to the worker to be dropped off-thread.
struct IrResult {
    left: IrSpectra,
    right: IrSpectra,
    soft_l: IrSpectra,
    soft_r: IrSpectra,
    loud_l: IrSpectra,
    loud_r: IrSpectra,
}

/// Build all six IR-variant spectra for `req` using the worker's scratch. Runs off
/// the audio thread (worker thread, or the calling thread in the no-worker
/// fallback). L and R share an IR, so each variant is built once and cloned.
fn build_ir_result(
    builder: &mut IrSpectraBuilder,
    ir: &mut Vec<f32>,
    ir_soft: &mut Vec<f32>,
    ir_loud: &mut Vec<f32>,
    req: &IrRequest,
) -> IrResult {
    Convolver::fill_ir(ir, req.ir_type, req.sample_rate, req.decay_trim);
    Convolver::fill_ir_soft(ir_soft, req.ir_type, req.sample_rate, req.decay_trim);
    Convolver::fill_ir_loud(ir_loud, req.ir_type, req.sample_rate, req.decay_trim);
    let left = builder.build(ir);
    let soft_l = builder.build(ir_soft);
    let loud_l = builder.build(ir_loud);
    IrResult {
        right: left.clone(),
        soft_r: soft_l.clone(),
        loud_r: loud_l.clone(),
        left,
        soft_l,
        loud_l,
    }
}

/// IR-build worker loop. Parks when idle (0% CPU); woken by `unpark` when a request
/// is queued, when the audio thread frees a result slot, or on shutdown. Coalesces:
/// only the most recent pending request is built. Drops returned (old) spectra here,
/// off the audio thread.
///
/// A freshly built result is **never dropped**: if the result slot is full it is
/// held in `pending` and re-delivered once the audio thread drains the slot (which
/// unparks us). Combined with request coalescing this guarantees the convolver
/// converges to the *latest* requested IR — dropping a built result would otherwise
/// strand it on a stale IR.
fn ir_worker(
    mut req_rx: ringbuf::HeapCons<IrRequest>,
    mut res_tx: ringbuf::HeapProd<IrResult>,
    mut trash_rx: ringbuf::HeapCons<IrResult>,
    shutdown: Arc<AtomicBool>,
) {
    let mut builder = IrSpectraBuilder::new(PARTITION_SIZE);
    let mut ir = Vec::with_capacity(MAX_IR_SAMPLES);
    let mut ir_soft = Vec::with_capacity(MAX_IR_SAMPLES);
    let mut ir_loud = Vec::with_capacity(MAX_IR_SAMPLES);
    // A built-but-undelivered result (the result slot was full). Held and retried,
    // never dropped, so the latest IR is never lost.
    let mut pending: Option<IrResult> = None;

    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        // Drop displaced (old) spectra off the audio thread.
        while trash_rx.try_pop().is_some() {}

        // Re-deliver a previously undeliverable result first.
        if let Some(result) = pending.take()
            && let Err(result) = res_tx.try_push(result)
        {
            pending = Some(result);
        }

        // Build the latest queued request — but only while not holding an
        // undelivered result, otherwise the newer build would have to be dropped.
        if pending.is_none() {
            let mut latest = None;
            while let Some(r) = req_rx.try_pop() {
                latest = Some(r);
            }
            if let Some(req) = latest {
                let result =
                    build_ir_result(&mut builder, &mut ir, &mut ir_soft, &mut ir_loud, &req);
                if let Err(result) = res_tx.try_push(result) {
                    pending = Some(result);
                }
                // Loop again to drain trash / retry delivery promptly.
                continue;
            }
        }

        // Idle, or holding an undelivered result waiting for the audio thread to
        // free a result slot (it unparks us when it drains one). Park until woken.
        std::thread::park();
    }
}

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

    // IR-build worker (off-thread FFT). The heavy spectra build runs on this
    // thread; the audio thread only swaps the finished spectra in (lock-free).
    // `Option` so `Drop` can take the handle for the join handshake, and so the
    // module degrades gracefully (synchronous rebuild) if the thread can't spawn.
    rebuild_tx: Option<ringbuf::HeapProd<IrRequest>>,
    result_rx: Option<ringbuf::HeapCons<IrResult>>,
    trash_tx: Option<ringbuf::HeapProd<IrResult>>,
    worker: Option<std::thread::JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    /// A param changed while the worker was mid-build; re-request after the next
    /// result is drained so we always converge on the latest IR.
    rebuild_dirty: bool,

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

        // Build the six convolvers and pre-reserve the worst-case partition pools
        // so a later off-thread IR swap never allocates on the audio thread.
        let make = |ir: &[f32]| {
            let mut c = PartitionedConvolver::new(PARTITION_SIZE, ir);
            c.reserve_partitions(MAX_PARTITIONS);
            c
        };
        let conv_left = make(&ir);
        let conv_right = make(&ir);
        let conv_soft_l = make(&ir_soft);
        let conv_soft_r = make(&ir_soft);
        let conv_loud_l = make(&ir_loud);
        let conv_loud_r = make(&ir_loud);

        // Spawn the IR-build worker. On the (very unlikely) spawn failure the
        // module falls back to building synchronously in `request_rebuild`.
        let shutdown = Arc::new(AtomicBool::new(false));
        let (rebuild_tx, rebuild_rx) = HeapRb::<IrRequest>::new(1).split();
        let (res_tx, result_rx) = HeapRb::<IrResult>::new(2).split();
        let (trash_tx, trash_rx) = HeapRb::<IrResult>::new(4).split();
        let worker_shutdown = Arc::clone(&shutdown);
        let worker = std::thread::Builder::new()
            .name("convolver-ir".to_string())
            .spawn(move || ir_worker(rebuild_rx, res_tx, trash_rx, worker_shutdown))
            .ok();
        let (rebuild_tx, result_rx, trash_tx) = if worker.is_some() {
            (Some(rebuild_tx), Some(result_rx), Some(trash_tx))
        } else {
            (None, None, None)
        };

        Self {
            ir_type,
            mix: NormalizedValue::new(0.3),
            pre_delay_ms: Milliseconds::new(0.0),
            decay_trim: DecayTrim::FULL,
            brightness: NormalizedValue::new(0.8),
            dynamic_mode: NormalizedValue::MIN,

            conv_left,
            conv_right,
            conv_soft_l,
            conv_soft_r,
            conv_loud_l,
            conv_loud_r,

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

            rebuild_tx,
            result_rx,
            trash_tx,
            worker,
            shutdown,
            rebuild_dirty: false,

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

    /// Flush the block accumulators after an IR swap (mirrors the old inline
    /// rebuild's tail reset).
    fn reset_accumulators(&mut self) {
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

    /// Swap freshly built spectra into the six convolvers — **no FFT, no alloc** on
    /// the audio thread (the FFTs ran on the worker). `result` is left holding the
    /// displaced (old) spectra for off-thread disposal.
    fn apply_result(&mut self, result: &mut IrResult) {
        self.conv_left.swap_ir_spectra(&mut result.left);
        self.conv_right.swap_ir_spectra(&mut result.right);
        self.conv_soft_l.swap_ir_spectra(&mut result.soft_l);
        self.conv_soft_r.swap_ir_spectra(&mut result.soft_r);
        self.conv_loud_l.swap_ir_spectra(&mut result.loud_l);
        self.conv_loud_r.swap_ir_spectra(&mut result.loud_r);
        self.reset_accumulators();
    }

    /// Request an IR rebuild for the current params: hand the (heavy, FFT-bound)
    /// build to the worker thread so the audio thread never runs it. If the
    /// worker's single request slot is full it sets `rebuild_dirty`, so
    /// `drain_rebuild` re-requests once the pending build lands — coalescing rapid
    /// automation to the latest value. Falls back to a synchronous build only if
    /// the worker thread couldn't be spawned.
    fn request_rebuild(&mut self) {
        let req = IrRequest {
            ir_type: self.ir_type,
            sample_rate: self.sample_rate,
            decay_trim: self.decay_trim.as_f32(),
        };
        if self.rebuild_tx.is_some() && self.worker.is_some() {
            let pushed = self
                .rebuild_tx
                .as_mut()
                .is_some_and(|tx| tx.try_push(req).is_ok());
            if pushed {
                if let Some(worker) = self.worker.as_ref() {
                    worker.thread().unpark();
                }
                self.rebuild_dirty = false;
            } else {
                // Worker busy; converge after it finishes (see `drain_rebuild`).
                self.rebuild_dirty = true;
            }
        } else {
            self.rebuild_now_inline(&req);
        }
    }

    /// Drain a finished IR build (if any) and swap it in. Called at the top of
    /// `process`. Returns the displaced spectra to the worker for off-thread drop,
    /// and re-requests if a param changed while the worker was busy.
    fn drain_rebuild(&mut self) {
        let popped = self
            .result_rx
            .as_mut()
            .and_then(ringbuf::traits::Consumer::try_pop);
        if let Some(mut result) = popped {
            self.apply_result(&mut result);
            // Hand the old spectra back to the worker to drop off the audio thread;
            // if the trash slot is full it drops here (rare, one-off).
            if let Some(trash) = self.trash_tx.as_mut() {
                let _ = trash.try_push(result);
            }
            if let Some(worker) = self.worker.as_ref() {
                worker.thread().unpark();
            }
            if self.rebuild_dirty {
                self.request_rebuild();
            }
        }
    }

    /// Synchronous IR rebuild on the calling thread — the no-worker fallback only.
    fn rebuild_now_inline(&mut self, req: &IrRequest) {
        let mut builder = IrSpectraBuilder::new(PARTITION_SIZE);
        let mut ir = Vec::with_capacity(MAX_IR_SAMPLES);
        let mut ir_soft = Vec::with_capacity(MAX_IR_SAMPLES);
        let mut ir_loud = Vec::with_capacity(MAX_IR_SAMPLES);
        let mut result = build_ir_result(&mut builder, &mut ir, &mut ir_soft, &mut ir_loud, req);
        self.apply_result(&mut result);
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
        // Swap in any IR the worker finished building since the last block (no FFT
        // or allocation on the audio thread; the worker did the heavy work).
        self.drain_rebuild();

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
                        self.request_rebuild();
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
                        self.request_rebuild();
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
            self.request_rebuild();
            self.update_delay();
        }
    }
}

impl Drop for Convolver {
    fn drop(&mut self) {
        // Signal the worker to exit, then unpark it so a parked worker actually
        // wakes to observe the flag, and join — otherwise each dropped Convolver
        // (project reload, track delete) would leak a parked thread.
        self.shutdown.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::traits::Observer;

    #[test]
    fn convolver_spawns_and_drop_joins_worker() {
        let c = Convolver::new();
        assert!(c.worker.is_some(), "IR-build worker thread should spawn");
        // Dropping must signal + join the worker without hanging or panicking.
        drop(c);
    }

    #[test]
    fn off_thread_rebuild_delivers_and_stays_finite() {
        let mut c = Convolver::new();
        let ctx = ProcessContext::default();
        // Interleaved stereo, a couple of partitions' worth.
        let input = vec![0.1f32; PARTITION_SIZE * 2 * 2];
        let mut output = vec![0.0f32; input.len()];

        // Baseline render is finite.
        c.process(&input, &mut output, &ctx);
        assert!(output.iter().all(|s| s.is_finite()));

        // Trigger an off-thread IR rebuild (Plate -> Hall).
        c.set_param(Param::Convolver(ConvolverParam::Ir(ImpulseResponse::Hall)));

        // The worker must deliver a finished build within a bounded wait.
        let mut delivered = false;
        for _ in 0..2000 {
            if c.result_rx.as_ref().is_some_and(|rx| !rx.is_empty()) {
                delivered = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(delivered, "worker did not deliver the rebuilt IR in time");

        // Draining + swapping the new IR in `process` stays finite and doesn't panic.
        c.process(&input, &mut output, &ctx);
        assert!(output.iter().all(|s| s.is_finite()));
        // The result was consumed and no re-request is pending.
        assert!(!c.rebuild_dirty);
        assert!(
            c.result_rx
                .as_ref()
                .is_some_and(ringbuf::traits::Observer::is_empty)
        );
    }

    #[test]
    fn rapid_ir_changes_settle_without_livelock() {
        // Rapidly change the IR several times, then process until quiescent. The
        // worker never drops a built result and strands a stale IR (it holds and
        // re-delivers), so the system converges: no pending result, no dirty flag,
        // finite output — and crucially it terminates (no livelock).
        let mut c = Convolver::new();
        let ctx = ProcessContext::default();
        let input = vec![0.05f32; PARTITION_SIZE * 2 * 2];
        let mut output = vec![0.0f32; input.len()];

        for ir in [
            ImpulseResponse::Hall,
            ImpulseResponse::Room,
            ImpulseResponse::Spring,
            ImpulseResponse::Plate,
        ] {
            c.set_param(Param::Convolver(ConvolverParam::Ir(ir)));
        }

        let mut settled = false;
        for _ in 0..5000 {
            c.process(&input, &mut output, &ctx);
            assert!(output.iter().all(|s| s.is_finite()));
            let quiescent = !c.rebuild_dirty
                && c.result_rx
                    .as_ref()
                    .is_some_and(ringbuf::traits::Observer::is_empty);
            if quiescent {
                // Let the worker settle, then confirm it stays quiescent (converged).
                std::thread::sleep(std::time::Duration::from_millis(2));
                if !c.rebuild_dirty
                    && c.result_rx
                        .as_ref()
                        .is_some_and(ringbuf::traits::Observer::is_empty)
                {
                    settled = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(settled, "convolver did not settle after rapid IR changes");
    }
}
