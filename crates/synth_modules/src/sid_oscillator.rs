//! SID oscillator module — the MOS 6581/8580 waveform generator.
//!
//! Chip-faithful core: a 24-bit integer phase accumulator stepped at a
//! fractional number of SID clocks per host sample (Q24.32 fixed point, so the
//! clock-domain conversion is jitter-free), waveforms derived from the raw
//! accumulator bits (combinable, like the chip's control register), a 12-bit
//! pulse-width register, and MSB-based hard sync. Band-limiting: PolyBLEP /
//! PolyBLAMP residuals at pure-waveform and sync discontinuities, plus an
//! optional 4x oversample + half-band decimation path (`Quality`).
//!
//! See `docs/sid-oscillator.md` for the full design rationale (the `§N`
//! references throughout this module point at its numbered sections).

use std::collections::HashMap;

use synth_core::VoicePitch;
use synth_core::{
    AudioBuffer, Describable, InputPorts, ModuleCategory, ModuleDescriptor, ModuleParam,
    ParamModOffsets, ParameterDescriptor, ParameterUnit, PolyModule, PortDescriptor,
    ProcessContext, Seconds, WidgetHint,
};

use crate::osc_glide::OscGlide;
use synth_core::{
    Gain, Hertz, MidiNote, ModuleType, Param, PortName, SID_FREQ_REG_MAX, SID_PW_REG_MAX,
    SID_SEQ_STEPS, SampleRate, SidClock, SidModel, SidNoiseSeed, SidOscillatorParam, SidQuality,
    Velocity,
};
use synth_dsp::oscillators::{poly_blamp, poly_blep};
use synth_dsp::oversampling::{Downsampler, OversamplingFactor};

/// Accumulator width: 24 integer bits, 32 fractional bits (Q24.32).
const ACC_FRAC_BITS: u32 = 32;
/// Mask keeping the accumulator within its 24.32 range (wrap at 2^24 cycles).
const ACC_MASK: u64 = (1 << (24 + ACC_FRAC_BITS)) - 1;
/// One full accumulator cycle (2^24) in Q24.32.
const ACC_CYCLE: f64 = (1u64 << (24 + ACC_FRAC_BITS)) as f64;
/// Scale from fractional accumulator units to Q24.32 (2^32).
const ACC_FRAC_SCALE: f64 = (1u64 << ACC_FRAC_BITS) as f64;
/// Accumulator MSB (bit 23 of the integer part) — the sync/ring signal.
const ACC_MSB: u32 = 0x80_0000;
/// Max host samples per `Downsampler` call at 4x: its intermediate stage holds
/// 8192 samples (= input/2), so one call accepts at most 16384 oversampled
/// input samples → 4096 output samples. Larger blocks are decimated in chunks.
const MAX_DECIMATE_OUT: usize = 4096;

// --- Noise LFSR (plan §2) ----------------------------------------------------
/// 23-bit register mask.
const LFSR_MASK: u32 = 0x7F_FFFF;
/// Power-on / TEST-reload value: all 23 bits set (= the full mask). An
/// all-zero LFSR is stuck forever (no feedback can ever set a bit), so the
/// reload is what makes noise-based drums identical on every playback.
const LFSR_INIT: u32 = LFSR_MASK;
/// The eight output-tap bits (22, 20, 16, 13, 11, 7, 4, 2) — also the bits the
/// NMOS bus conflict pulls low when noise is combined with another waveform.
const LFSR_TAP_MASK: u32 =
    (1 << 22) | (1 << 20) | (1 << 16) | (1 << 13) | (1 << 11) | (1 << 7) | (1 << 4) | (1 << 2);
/// The LFSR clocks on accumulator bit 19's 0→1 edge. In Q24.32 that bit has
/// period 2^52 with the rising boundary at odd multiples of 2^51.
const BIT19_EDGE_SHIFT: u32 = 19 + 1 + ACC_FRAC_BITS;
const BIT19_EDGE_OFFSET: u64 = 1 << (19 + ACC_FRAC_BITS);

// --- Waveform mask bits (SeqStep nibble; the chip's control-register order) --
const MASK_TRIANGLE: u8 = 1 << 0;
const MASK_SAWTOOTH: u8 = 1 << 1;
const MASK_PULSE: u8 = 1 << 2;
const MASK_NOISE: u8 = 1 << 3;

// --- 6581 combined-waveform pulldown fit (plan §2 option C, §11 targets) -----
/// Saw+tri run length: a bus bit needs this many set bits below it (plus one
/// above) to survive — fitted so the digital AC RMS lands ~26 dB below the
/// saw's with the fundamental killed, as measured on the chip (§11 gap 1).
const ST_PULLDOWN_RUN: u32 = 3;
/// MSB of the 12-bit waveform bus (bit 11 — distinct from the accumulator's
/// [`ACC_MSB`]).
const BUS_MSB: u32 = 0x800;

/// DC-blocker corner frequency — the C64's output coupling (§11: combined
/// 6581 waveforms carry real DC, e.g. ≈ −0.22 for tri+pulse, which eats mix
/// headroom; reSID's reference path is AC-coupled downstream too).
const DC_BLOCK_HZ: f32 = 16.0;

// --- Waveform DAC (plan §2 "DAC non-linearity", phase 3) ----------------------
/// 6581 waveform-DAC 2R/R resistor ratio. The ideal ladder uses exactly 2R;
/// the 6581's poly-silicon resistors measure ≈ 2.2R, which (together with its
/// missing termination resistor) produces the characteristic non-linear,
/// kinked DAC curve. The 8580 DAC is near-ideal and modelled as linear.
const DAC_6581_RATIO: f64 = 2.20;

/// Per-bit output weights of a 12-bit R-2R ladder with 2R = `k`·R, derived by
/// superposition with a Thevenin walk from the LSB node to the MSB (output)
/// node. `terminated` selects whether the LSB end has the classic 2R
/// termination to ground (the 6581 famously lacks it). Pure circuit analysis —
/// no copied emulator tables (plan §7 licensing).
fn r2r_dac_weights(k: f64, terminated: bool) -> [f64; 12] {
    let mut weights = [0.0f64; 12];
    for (j, weight) in weights.iter_mut().enumerate() {
        // Superposition: bit j's source at 1, every other bit grounded
        // through its own 2R leg. Node 0 is the LSB end.
        let b0 = if j == 0 { 1.0 } else { 0.0 };
        let (mut v, mut r) = if terminated {
            // Bit leg (k) in parallel with the k termination to ground.
            (b0 * 0.5, k * 0.5)
        } else {
            (b0, k)
        };
        for i in 1..12 {
            let bi = if j == i { 1.0 } else { 0.0 };
            // Series R to node i, then the node's bit leg (k) joins in.
            let rs = r + 1.0;
            v = (v * k + bi * rs) / (rs + k);
            r = rs * k / (rs + k);
        }
        *weight = v;
    }
    weights
}

/// 6581 waveform-DAC lookup: 12-bit digital value → normalized 0..1 output.
/// Built once (forced off the audio thread in [`SidOscillator::new`]) and
/// shared read-only by every instance.
static DAC_6581: std::sync::LazyLock<[f32; 4096]> = std::sync::LazyLock::new(|| {
    let weights = r2r_dac_weights(DAC_6581_RATIO, false);
    let full: f64 = weights.iter().sum();
    let mut table = [0.0f32; 4096];
    for (value, out) in table.iter_mut().enumerate() {
        let mut sum = 0.0f64;
        for (bit, weight) in weights.iter().enumerate() {
            if value & (1 << bit) != 0 {
                sum += weight;
            }
        }
        #[allow(clippy::cast_possible_truncation)]
        {
            *out = (sum / full) as f32;
        }
    }
    table
});

/// Which single waveform is active, when exactly one is — the cases the
/// PolyBLEP/PolyBLAMP residuals cover. Combined selections (and TEST) are
/// `None`: their step structure is left to the oversample path.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PureWave {
    None,
    Triangle,
    Sawtooth,
    Pulse,
}

/// The MOS 6581/8580 SID waveform generator as a per-voice module.
#[derive(Clone)]
pub struct SidOscillator {
    // Register-level parameters
    triangle: bool,
    sawtooth: bool,
    pulse: bool,
    noise: bool,
    noise_seed: SidNoiseSeed,
    freq_reg: u32,
    track_voice_pitch: bool,
    pulse_width_reg: u32,
    test: bool,
    ring_mod: bool,
    hard_sync: bool,
    model: SidModel,
    clock: SidClock,
    quality: SidQuality,
    dc_block: bool,
    level: Gain,
    // Waveform-mask sequence program (per-frame, driver-style)
    seq_length: u8,
    seq_rate: u8,
    seq_loop: bool,
    seq_steps: [u8; SID_SEQ_STEPS],
    seq_freq_mask: u16,
    seq_step_freqs: [u32; SID_SEQ_STEPS],

    // Chip state (free-running; note_on must NOT reseed — plan §5)
    /// Phase accumulator, Q24.32 (top 24 bits are the SID accumulator).
    acc: u64,
    /// Live played-note register while `TrackVoicePitch` is on. Kept apart from
    /// the authored `freq_reg` param so playback never mutates the value the
    /// save path serializes.
    tracked_freq_reg: Option<u32>,
    /// Per-oscillator glide (portamento); `0` glide time = follow the voice glide.
    /// Only affects tuning while `track_voice_pitch` is on.
    glide: OscGlide,
    /// 23-bit noise LFSR, clocked on accumulator bit 19's rising edge.
    lfsr: u32,
    /// Previous sample's sync-input value, for MSB 0→1 edge detection.
    prev_sync: f32,
    /// Half the height of a just-applied sync-reset discontinuity, consumed by
    /// the next generated sample's PolyBLEP residual (0 = no pending step).
    pending_sync_step: f32,
    /// Evaluation distance of that residual, in generated samples ∈ [0, 1]:
    /// how long before the next generated sample the output step occurred.
    pending_sync_d: f32,
    /// Oversampling factor of the previous block — a change invalidates the
    /// half-band filter state.
    last_factor: usize,
    /// Previous sample's combined TEST state (param OR gate input), for the
    /// hard-restart rising edge that reloads the LFSR.
    prev_test_active: bool,
    /// Previous sample's ring-input MSB state, for fold-flip edge detection.
    prev_ring_high: bool,
    /// DC-blocker state: previous input / output sample (host rate).
    dc_x1: f32,
    dc_y1: f32,
    /// Half the height of a just-applied ring fold-flip discontinuity, consumed
    /// by the next generated sample's PolyBLEP residual (0 = no pending step) —
    /// the triangle inverts (`tri → -tri`) at the ring source's MSB edge, a step
    /// the same one-sided BLEP that band-limits the sync reset cleans without
    /// low-passing the real HF sidebands (§11 shoulder-texture).
    pending_ring_step: f32,
    /// Evaluation distance of that residual, in generated samples ∈ [0, 1].
    pending_ring_d: f32,

    // Waveform-sequence playback state (restarted by note_on — program data,
    // not chip state)
    /// Samples into the current driver frame.
    frame_pos: f32,
    /// Driver frames elapsed since the sequence (re)started.
    frame_count: u32,

    // Mod matrix offsets (generic store; `freq_reg`/`pw_reg`/`level`)
    mod_offsets: ParamModOffsets,

    // Transient sequencer-automation overrides (never mutate the base params
    // — cleared on transport stop). PW automation is a core SID idiom, so the
    // register lanes must be live (plan §11 addendum ask 2).
    override_freq_reg: Option<u32>,
    override_pw_reg: Option<u32>,
    override_level: Option<Gain>,

    // Host-rate state
    sample_rate: SampleRate,
    downsampler: Downsampler,

    // Custom port names (interned once in the constructor — never in process)
    msb_port: PortName,
    ring_port: PortName,
    test_port: PortName,

    // Output scratch
    output_buffer: AudioBuffer,
    msb_buffer: AudioBuffer,
    oversample_buffer: AudioBuffer,
}

impl SidOscillator {
    pub fn new() -> Self {
        // Build the shared 6581 DAC table now, off the audio thread — the
        // first process() call must never pay the LazyLock initialization.
        std::sync::LazyLock::force(&DAC_6581);
        Self {
            triangle: false,
            sawtooth: true,
            pulse: false,
            noise: false,
            noise_seed: SidNoiseSeed::DEFAULT,
            freq_reg: 0,
            track_voice_pitch: true,
            pulse_width_reg: 2048,
            test: false,
            ring_mod: false,
            hard_sync: false,
            model: SidModel::default(),
            clock: SidClock::default(),
            quality: SidQuality::default(),
            dc_block: true,
            level: Gain::UNITY,
            seq_length: 0,
            seq_rate: 1,
            seq_loop: false,
            seq_steps: [0; SID_SEQ_STEPS],
            seq_freq_mask: 0,
            seq_step_freqs: [0; SID_SEQ_STEPS],
            acc: 0,
            tracked_freq_reg: None,
            glide: OscGlide::new(),
            lfsr: LFSR_INIT,
            prev_sync: 0.0,
            pending_sync_step: 0.0,
            pending_sync_d: 0.0,
            last_factor: 1,
            prev_test_active: false,
            prev_ring_high: false,
            pending_ring_step: 0.0,
            pending_ring_d: 0.0,
            dc_x1: 0.0,
            dc_y1: 0.0,
            frame_pos: 0.0,
            frame_count: 0,
            mod_offsets: ParamModOffsets::new(),
            override_freq_reg: None,
            override_pw_reg: None,
            override_level: None,
            sample_rate: SampleRate::DVD_QUALITY,
            downsampler: Downsampler::new(),
            msb_port: PortName::intern("msb"),
            ring_port: PortName::intern("ring"),
            test_port: PortName::intern("test"),
            output_buffer: AudioBuffer::new(1024),
            msb_buffer: AudioBuffer::new(1024),
            oversample_buffer: AudioBuffer::new(4096),
        }
    }

    /// Convert a note frequency to the 16-bit SID frequency register for the
    /// configured clock: `freq_reg = round(f_hz * 2^24 / f_clock)` (plan §5).
    fn freq_to_reg(&self, freq: Hertz) -> u32 {
        let reg =
            (f64::from(freq.as_f32()) * f64::from(1u32 << 24) / self.clock.clock_hz()).round();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            (reg.max(0.0) as u32).min(SID_FREQ_REG_MAX)
        }
    }

    /// The frequency register the DSP runs from: the live played-note register
    /// while `TrackVoicePitch` is on, the authored `FreqReg` param (or its
    /// transient automation override) otherwise.
    fn effective_freq_reg(&self) -> u32 {
        let authored = self.override_freq_reg.unwrap_or(self.freq_reg);
        if self.track_voice_pitch {
            self.tracked_freq_reg.unwrap_or(authored)
        } else {
            authored
        }
    }

    /// Q24.32 accumulator units per frequency-register unit per generated
    /// sample — the block-constant factor of the increment. Each SID clock adds
    /// `freq_reg` to the 24-bit accumulator, so per generated sample the
    /// advance is `freq_reg * (f_clock / rate)` accumulator units.
    fn reg_to_inc(&self, rate: f64) -> f64 {
        self.clock.clock_hz() / rate * ACC_FRAC_SCALE
    }

    /// Accumulator increment (Q24.32) for a register value, given the
    /// block-constant [`reg_to_inc`](Self::reg_to_inc) factor.
    fn acc_increment(freq_reg_eff: f64, reg_to_inc: f64) -> u64 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            ((freq_reg_eff * reg_to_inc).round() as u64) & ACC_MASK
        }
    }

    /// Phase increment in cycles per generated sample, for the BLEP windows.
    fn inc_to_dt(inc: u64) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        {
            (inc as f64 / ACC_CYCLE) as f32
        }
    }

    /// The 4-bit waveform mask from the static waveform-bit params.
    fn static_mask(&self) -> u8 {
        (u8::from(self.triangle) * MASK_TRIANGLE)
            | (u8::from(self.sawtooth) * MASK_SAWTOOTH)
            | (u8::from(self.pulse) * MASK_PULSE)
            | (u8::from(self.noise) * MASK_NOISE)
    }

    /// The waveform mask the current driver frame selects: the sequence step
    /// while a sequence is programmed (looping when `SeqLoop` is set, holding
    /// its last step otherwise), the static bits otherwise.
    fn live_mask(&self) -> u8 {
        if self.seq_length == 0 {
            return self.static_mask();
        }
        let rate = u32::from(self.seq_rate.max(1));
        let pos = self.frame_count / rate;
        let idx = if self.seq_loop {
            pos % u32::from(self.seq_length)
        } else {
            pos.min(u32::from(self.seq_length - 1))
        };
        self.seq_steps.get(idx as usize).copied().unwrap_or(0) & 0xF
    }

    fn live_step_index(&self) -> Option<usize> {
        if self.seq_length == 0 {
            return None;
        }
        let position = self.frame_count / u32::from(self.seq_rate.max(1));
        let index = if self.seq_loop {
            position % u32::from(self.seq_length)
        } else {
            position.min(u32::from(self.seq_length - 1))
        };
        Some(index as usize)
    }

    fn live_freq_reg(&self, inherited: f64) -> f64 {
        let Some(index) = self.live_step_index() else {
            return inherited;
        };
        if self.seq_freq_mask & (1u16 << index) == 0 {
            return inherited;
        }
        self.seq_step_freqs
            .get(index)
            .copied()
            .map_or(inherited, f64::from)
    }

    /// Classify a mask for the band-limiting residual (recomputed only when
    /// the mask changes, not per sub-sample).
    fn pure_from_mask(mask: u8) -> PureWave {
        match mask {
            MASK_TRIANGLE => PureWave::Triangle,
            MASK_SAWTOOTH => PureWave::Sawtooth,
            MASK_PULSE => PureWave::Pulse,
            _ => PureWave::None,
        }
    }

    /// The live mask with its frame-constant derived state: the pure-waveform
    /// classification and whether NoiseLock (noise + another waveform) is
    /// active. One derivation shared by the block prologue and the
    /// frame-boundary update, so the two can't diverge.
    fn mask_state(&self) -> (u8, PureWave, bool) {
        let mask = self.live_mask();
        let locked = mask & MASK_NOISE != 0 && mask & !MASK_NOISE != 0;
        (mask, Self::pure_from_mask(mask), locked)
    }

    /// Whether any waveform this block can select needs the oversampled path:
    /// combined masks and noise, whose many small steps PolyBLEP can't clean
    /// (plan §2 hybrid strategy), and the ring-modulated triangle, whose
    /// fold flips are un-BLEPped discontinuities (§11 ring-mod brightness) —
    /// only while a ring source is actually connected (`ring_connected`,
    /// block-constant), since without one no flip can ever occur. With a
    /// sequence programmed, every reachable step is checked so the factor
    /// stays block-constant.
    fn needs_oversample(&self, ring_connected: bool) -> bool {
        let ring_live = self.ring_mod && ring_connected;
        let needs = |m: u8| {
            m & MASK_NOISE != 0 || m.count_ones() > 1 || (ring_live && m & MASK_TRIANGLE != 0)
        };
        if self.seq_length > 0 {
            let len = usize::from(self.seq_length).min(SID_SEQ_STEPS);
            self.seq_steps[..len].iter().any(|&m| needs(m & 0xF))
        } else {
            needs(self.static_mask())
        }
    }

    /// Combined-waveform bus model — the plan §2 **option C** parametric
    /// pulldown, per (mask, model), on top of the plain AND the caller already
    /// produced. 6581: the analog bus conflict pulls a line low unless enough
    /// neighbours drive it high, with a per-combination strength fitted to the
    /// §11 golden A/B measurements (independently derived — no emulator
    /// tables, plan §7):
    ///
    /// - **saw+tri (0x31):** a bit survives only inside a run of set bits
    ///   (three below + one above) — kills the fundamental and leaves narrow
    ///   spikes whose energy sits at high accumulator-bit products
    ///   (~8/16/32·f0), matching the measured chip (ref RMS ≈ −26 dB vs saw).
    /// - **pulse+saw (0x61) / all three (0x71):** the conflict collapses the
    ///   whole bus — only a full run of twelve set bits survives, i.e. the
    ///   output blips once at the very top of the ramp and otherwise reads as
    ///   near-silence, like the chip.
    /// - **pulse+tri (0x51) and noise combinations:** the mild
    ///   neighbour-support model (measured at the A/B floor — keep).
    ///
    /// 8580: modelled as the plain AND (measured at/near the floor for every
    /// combination; its small DC-offset effect is a documented deferral).
    fn combine_bus(&self, mask: u8, v: u32) -> u32 {
        const ST: u8 = MASK_TRIANGLE | MASK_SAWTOOTH;
        const PS: u8 = MASK_PULSE | MASK_SAWTOOTH;
        const PST: u8 = PS | MASK_TRIANGLE;
        match self.model {
            SidModel::Mos6581 => {
                if mask & MASK_NOISE != 0 {
                    return Self::neighbour_support(v);
                }
                match mask & PST {
                    ST => Self::pulldown_run(v, ST_PULLDOWN_RUN),
                    PS | PST => Self::bus_collapse(v),
                    _ => Self::neighbour_support(v),
                }
            }
            SidModel::Mos8580 => v,
        }
    }

    /// Mild 6581 bus conflict: a cleared bit drags a lone neighbour down — a
    /// bit survives only with at least one set neighbour (the former option-B
    /// baseline; still the fitted model for pulse+tri and noise combinations).
    /// `v` is a 12-bit bus value, so the shifts cannot leak past the AND.
    fn neighbour_support(v: u32) -> u32 {
        v & ((v << 1) | (v >> 1))
    }

    /// Strong 6581 bus pulldown: bit `i` survives only when bits
    /// `i-run_below..=i+1` are all set (the bus MSB counts as externally
    /// supported). The fitted option-C form: the run length is the pulldown
    /// strength.
    fn pulldown_run(v: u32, run_below: u32) -> u32 {
        let mut r = v & ((v >> 1) | BUS_MSB);
        for d in 1..=run_below {
            r &= v << d;
        }
        r
    }

    /// Total 6581 bus collapse (pulse+saw combinations): only a full run of
    /// twelve set bits keeps the bus up — the MSB line blips once at the very
    /// top of the ramp, otherwise the output reads as near-silence, like the
    /// chip (§11 gap 2). Closed form of `pulldown_run(v, 11)` without the
    /// upper-neighbour term.
    fn bus_collapse(v: u32) -> u32 {
        if v == 0xFFF { BUS_MSB } else { 0 }
    }

    /// One LFSR shift: `new_bit0 = bit22 XOR bit17`, register shifted left,
    /// masked to 23 bits. While noise is combined with another waveform the
    /// NMOS bus conflict pulls the output-tap bits low ("NoiseLock"): the
    /// register corrupts and stays degenerate until TEST/reset (plan §2).
    fn clock_lfsr(&mut self, noise_locked: bool) {
        let bit = ((self.lfsr >> 22) ^ (self.lfsr >> 17)) & 1;
        self.lfsr = ((self.lfsr << 1) | bit) & LFSR_MASK;
        if noise_locked {
            self.lfsr &= !LFSR_TAP_MASK;
        }
    }

    /// The 8-bit noise value assembled from LFSR taps 22,20,16,13,11,7,4,2.
    fn noise_output8(sr: u32) -> u32 {
        ((sr >> 22) & 1) << 7
            | ((sr >> 20) & 1) << 6
            | ((sr >> 16) & 1) << 5
            | ((sr >> 13) & 1) << 4
            | ((sr >> 11) & 1) << 3
            | ((sr >> 7) & 1) << 2
            | ((sr >> 4) & 1) << 1
            | ((sr >> 2) & 1)
    }

    /// Advance the accumulator by `inc` and clock the LFSR once per
    /// accumulator-bit-19 rising edge crossed (the chip's noise clock — the
    /// LFSR free-runs regardless of waveform selection). Edge counting is done
    /// on the unwrapped Q24.32 positions, so no 1 MHz loop is needed. The
    /// count is capped: bit 19 rises 16× per full accumulator cycle, so more
    /// than a couple of cycles per generated sample (only reachable at
    /// pathologically low sample rates) carries no audible information and
    /// must not become an unbounded loop.
    fn advance_acc(&mut self, inc: u64, noise_locked: bool) {
        let old = self.acc;
        let new_unwrapped = old + inc;
        self.acc = new_unwrapped & ACC_MASK;
        let edges = (((new_unwrapped + BIT19_EDGE_OFFSET) >> BIT19_EDGE_SHIFT)
            - ((old + BIT19_EDGE_OFFSET) >> BIT19_EDGE_SHIFT))
            .min(32);
        for _ in 0..edges {
            self.clock_lfsr(noise_locked);
        }
    }

    /// The bipolar DAC sample for the current state — the one conversion used
    /// by the sync pre/post pair and the main generation loop.
    fn dac_sample(&self, mask: u8, acc24: u32, pw: u32, ring_high: bool, test_active: bool) -> f32 {
        self.waveform_12bit(mask, acc24, pw, ring_high, test_active)
            .map_or(0.0, |v| self.dac_to_bipolar(v))
    }

    /// The 12-bit digital waveform value for the current accumulator + LFSR
    /// state under `mask`. Selected waveforms meet on the shared bus: modelled
    /// as a bitwise AND, with the 6581's analog bus conflict additionally
    /// pulling bits low per combination (the parametric option-C model in
    /// [`combine_bus`](Self::combine_bus) — plan §2). Returns `None` when no
    /// waveform bit is set (the DAC input floats — treated as 0).
    fn waveform_12bit(
        &self,
        mask: u8,
        acc24: u32,
        pw: u32,
        ring_high: bool,
        test_active: bool,
    ) -> Option<u32> {
        if mask == 0 {
            return None;
        }
        let mut v = 0xFFFu32;
        if mask & MASK_TRIANGLE != 0 {
            // Saw→triangle fold: MSB selects direct or inverted accumulator;
            // the top 11 bits drive DAC bits 11..1 (LSB is 0) — half the
            // amplitude resolution of the saw. Ring mod XORs the fold
            // direction with the ring source's MSB — triangle only (plan §3).
            let fold = (acc24 & ACC_MSB != 0) ^ ring_high;
            let folded = if fold { !acc24 } else { acc24 } & 0x7F_FFFF;
            v &= (folded >> 12) << 1;
        }
        if mask & MASK_SAWTOOTH != 0 {
            v &= acc24 >> 12;
        }
        if mask & MASK_PULSE != 0 {
            // High while the top 12 accumulator bits are >= the PW register.
            // TEST forces the pulse output high (chip behaviour).
            let high = test_active || (acc24 >> 12) >= pw;
            v &= if high { 0xFFF } else { 0 };
        }
        if mask & MASK_NOISE != 0 {
            v &= Self::noise_output8(self.lfsr) << 4;
        }
        if mask.count_ones() > 1 {
            v = self.combine_bus(mask, v);
        }
        Some(v)
    }

    /// The parameter of the same kind as `p`, carrying this module's live
    /// value — the single field → `SidOscillatorParam` mapping shared by
    /// `get_param` and `get_params` (so the value encoding can't diverge).
    fn current(&self, p: &SidOscillatorParam) -> SidOscillatorParam {
        match p {
            SidOscillatorParam::Triangle(_) => SidOscillatorParam::Triangle(self.triangle),
            SidOscillatorParam::Sawtooth(_) => SidOscillatorParam::Sawtooth(self.sawtooth),
            SidOscillatorParam::Pulse(_) => SidOscillatorParam::Pulse(self.pulse),
            SidOscillatorParam::Noise(_) => SidOscillatorParam::Noise(self.noise),
            SidOscillatorParam::NoiseSeed(_) => SidOscillatorParam::NoiseSeed(self.noise_seed),
            SidOscillatorParam::FreqReg(_) => SidOscillatorParam::FreqReg(self.freq_reg),
            SidOscillatorParam::TrackVoicePitch(_) => {
                SidOscillatorParam::TrackVoicePitch(self.track_voice_pitch)
            }
            SidOscillatorParam::PulseWidthReg(_) => {
                SidOscillatorParam::PulseWidthReg(self.pulse_width_reg)
            }
            SidOscillatorParam::Test(_) => SidOscillatorParam::Test(self.test),
            SidOscillatorParam::RingMod(_) => SidOscillatorParam::RingMod(self.ring_mod),
            SidOscillatorParam::HardSync(_) => SidOscillatorParam::HardSync(self.hard_sync),
            SidOscillatorParam::Model(_) => SidOscillatorParam::Model(self.model),
            SidOscillatorParam::Clock(_) => SidOscillatorParam::Clock(self.clock),
            SidOscillatorParam::Quality(_) => SidOscillatorParam::Quality(self.quality),
            SidOscillatorParam::DcBlock(_) => SidOscillatorParam::DcBlock(self.dc_block),
            SidOscillatorParam::Level(_) => SidOscillatorParam::Level(self.level),
            SidOscillatorParam::GlideTime(_) => SidOscillatorParam::GlideTime(self.glide.time()),
            SidOscillatorParam::SeqLength(_) => SidOscillatorParam::SeqLength(self.seq_length),
            SidOscillatorParam::SeqRate(_) => SidOscillatorParam::SeqRate(self.seq_rate),
            SidOscillatorParam::SeqLoop(_) => SidOscillatorParam::SeqLoop(self.seq_loop),
            SidOscillatorParam::SeqStep(i, _) => SidOscillatorParam::SeqStep(
                *i,
                self.seq_steps.get(usize::from(*i)).copied().unwrap_or(0),
            ),
            SidOscillatorParam::SeqFreqMask(_) => {
                SidOscillatorParam::SeqFreqMask(u32::from(self.seq_freq_mask))
            }
            SidOscillatorParam::SeqStepFreq(i, _) => SidOscillatorParam::SeqStepFreq(
                *i,
                self.seq_step_freqs
                    .get(usize::from(*i))
                    .copied()
                    .unwrap_or(0),
            ),
        }
    }

    /// Map a 12-bit digital value through the model's waveform DAC to a
    /// bipolar sample: the 6581's kinked non-linear ladder via the shared
    /// lookup, the near-ideal 8580 as a straight line (plan §2 phase 3).
    fn dac_to_bipolar(&self, v: u32) -> f32 {
        match self.model {
            SidModel::Mos6581 => DAC_6581[(v & 0xFFF) as usize] * 2.0 - 1.0,
            #[allow(clippy::cast_precision_loss)]
            SidModel::Mos8580 => (v as f32) * (2.0 / 4095.0) - 1.0,
        }
    }

    /// Band-limiting residual for the *pure* (single-waveform) shapes at the
    /// current phase: PolyBLEP at saw wrap / pulse edges, PolyBLAMP at the
    /// triangle corners. Combined waveforms are left to the oversample path.
    /// `ring_high` inverts the triangle fold (digital `tri_ring = -tri`), so
    /// its corners swap peak/trough and the residual flips sign with it.
    fn pure_waveform_residual(pure: PureWave, p: f32, dt: f32, pw: u32, ring_high: bool) -> f32 {
        match pure {
            PureWave::None => 0.0,
            // Rising ramp with a -2 step at the wrap.
            PureWave::Sawtooth => -poly_blep(p, dt),
            PureWave::Pulse => {
                #[allow(clippy::cast_precision_loss)]
                let edge = (pw as f32) / 4096.0;
                if edge <= 0.0 {
                    return 0.0; // pw = 0: always high, no edges
                }
                // Low before `edge`, high after, falling at the wrap.
                let rising = poly_blep((p - edge).rem_euclid(1.0), dt);
                let falling = poly_blep(p, dt);
                rising - falling
            }
            PureWave::Triangle => {
                // Trough at p = 0 (slope -4 → +4), peak at p = 0.5 (+4 → -4);
                // exactly mirrored while the ring input holds the fold inverted.
                let d_trough = if p > 0.5 { p - 1.0 } else { p };
                let r = poly_blamp(p - 0.5, dt) * 4.0 - poly_blamp(d_trough, dt) * 4.0;
                if ring_high { -r } else { r }
            }
        }
    }
}

impl Default for SidOscillator {
    fn default() -> Self {
        Self::new()
    }
}

impl Describable for SidOscillator {
    #[allow(clippy::too_many_lines)]
    fn descriptor(&self) -> ModuleDescriptor {
        let mut desc = ModuleDescriptor::new("sid_oscillator", "SID Oscillator")
            .description(
                "MOS 6581/8580 (SID) waveform generator: combinable waveforms from a 24-bit \
                 accumulator, raw frequency/pulse-width registers, MSB-based ring/hard-sync, \
                 and a per-frame waveform-mask sequence",
            )
            .category(ModuleCategory::Oscillator)
            .width(synth_core::ModuleWidth::ExtraLarge)
            .tag("oscillator")
            .tag("source")
            .tag("sid")
            .tag("chiptune")
            .parameter(
                ParameterDescriptor::float(
                    "noise_seed",
                    Param::SidOscillator(SidOscillatorParam::NoiseSeed(SidNoiseSeed::DEFAULT)),
                    "Noise Seed",
                )
                .description("Non-zero 23-bit LFSR state loaded by reset and TEST")
                .range(1.0, SidNoiseSeed::MAX as f32)
                .default(SidNoiseSeed::DEFAULT.as_u32() as f32)
                .modulatable(false)
                .widget(WidgetHint::Hidden),
            )
            .parameter(
                ParameterDescriptor::float(
                    "triangle",
                    Param::SidOscillator(SidOscillatorParam::Triangle(false)),
                    "Triangle",
                )
                .description("Triangle waveform bit (combinable)")
                .range(0.0, 1.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::WaveformToggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "sawtooth",
                    Param::SidOscillator(SidOscillatorParam::Sawtooth(true)),
                    "Sawtooth",
                )
                .description("Sawtooth waveform bit (combinable)")
                .range(0.0, 1.0)
                .default(1.0)
                .modulatable(false)
                .widget(WidgetHint::WaveformToggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pulse",
                    Param::SidOscillator(SidOscillatorParam::Pulse(false)),
                    "Pulse",
                )
                .description("Pulse waveform bit (combinable, width from PW Reg)")
                .range(0.0, 1.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::WaveformToggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "noise",
                    Param::SidOscillator(SidOscillatorParam::Noise(false)),
                    "Noise",
                )
                .description("Noise waveform bit (23-bit LFSR clocked by the accumulator)")
                .range(0.0, 1.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::WaveformToggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "freq_reg",
                    Param::SidOscillator(SidOscillatorParam::FreqReg(0)),
                    "Freq Reg",
                )
                .description(
                    "Raw 16-bit SID frequency register — used when Track Pitch is off \
                     (e.g. a ring/sync source held at a neighbour voice's pitch). \
                     Mod-matrix modulatable",
                )
                .range(0.0, SID_FREQ_REG_MAX as f32)
                .default(0.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "track_pitch",
                    Param::SidOscillator(SidOscillatorParam::TrackVoicePitch(true)),
                    "Track Pitch",
                )
                .description(
                    "Derive Freq Reg from the played note (on) or hold the authored \
                     Freq Reg (off — ring/sync-source tuning)",
                )
                .range(0.0, 1.0)
                .default(1.0)
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "pw_reg",
                    Param::SidOscillator(SidOscillatorParam::PulseWidthReg(2048)),
                    "PW Reg",
                )
                .description(
                    "Raw 12-bit pulse-width register (2048 = square). Mod-matrix \
                     modulatable — PWM is a core SID idiom",
                )
                .range(0.0, SID_PW_REG_MAX as f32)
                .default(2048.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "test",
                    Param::SidOscillator(SidOscillatorParam::Test(false)),
                    "Test",
                )
                .description(
                    "TEST bit: zeroes and holds the accumulator (hard restart) and \
                     reloads the noise LFSR",
                )
                .range(0.0, 1.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "ring_mod",
                    Param::SidOscillator(SidOscillatorParam::RingMod(false)),
                    "Ring Mod",
                )
                .description(
                    "RING bit: the triangle folding direction XORs with the `ring` \
                     input's MSB (triangle only, like the chip)",
                )
                .range(0.0, 1.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "hard_sync",
                    Param::SidOscillator(SidOscillatorParam::HardSync(false)),
                    "Hard Sync",
                )
                .description(
                    "SYNC bit: reset the accumulator on the `sync` input's MSB \
                     0→1 rising edge",
                )
                .range(0.0, 1.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "model",
                    Param::SidOscillator(SidOscillatorParam::Model(SidModel::Mos6581)),
                    "Model",
                    SidModel::to_choices(),
                )
                .description("Chip model: combined-waveform behaviour + DAC curve"),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "clock",
                    Param::SidOscillator(SidOscillatorParam::Clock(SidClock::Pal)),
                    "Clock",
                    SidClock::to_choices(),
                )
                .description("Master clock standard for the pitch mapping"),
            )
            .parameter(
                ParameterDescriptor::choice(
                    "quality",
                    Param::SidOscillator(SidOscillatorParam::Quality(SidQuality::Fast)),
                    "Quality",
                    SidQuality::to_choices(),
                )
                .description("Clock-domain conversion strategy (CPU vs fidelity)"),
            )
            .parameter(
                ParameterDescriptor::float(
                    "dc_block",
                    Param::SidOscillator(SidOscillatorParam::DcBlock(true)),
                    "DC Block",
                )
                .description(
                    "One-pole DC blocker (~16 Hz, the C64 output coupling). \
                     Combined 6581 waveforms carry real DC that eats mix headroom",
                )
                .range(0.0, 1.0)
                .default(1.0)
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            )
            .parameter(
                ParameterDescriptor::float(
                    "level",
                    Param::SidOscillator(SidOscillatorParam::Level(Gain::UNITY)),
                    "Level",
                )
                .description("Output level")
                .range(0.0, 1.0)
                .default(1.0)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "glide_time",
                    Param::SidOscillator(SidOscillatorParam::GlideTime(Seconds::ZERO)),
                    "Glide",
                )
                .description("Per-oscillator portamento time (0 = follow the voice glide)")
                .range(0.0, 2.0)
                .default(0.0)
                .modulatable(false)
                .unit(ParameterUnit::Seconds)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "seq_len",
                    Param::SidOscillator(SidOscillatorParam::SeqLength(0)),
                    "Seq Length",
                )
                .description(
                    "Active waveform-sequence steps (0 = off, the static waveform \
                     bits apply). Past the last step the sequence holds it, or \
                     repeats when Seq Loop is on",
                )
                .range(0.0, SID_SEQ_STEPS as f32)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "seq_rate",
                    Param::SidOscillator(SidOscillatorParam::SeqRate(1)),
                    "Seq Rate",
                )
                .description("Driver frames (50/60 Hz) per waveform-sequence step")
                .range(1.0, 16.0)
                .default(1.0)
                .modulatable(false)
                .widget(WidgetHint::Knob),
            )
            .parameter(
                ParameterDescriptor::float(
                    "seq_loop",
                    Param::SidOscillator(SidOscillatorParam::SeqLoop(false)),
                    "Seq Loop",
                )
                .description(
                    "Loop the waveform sequence for the whole note (off = hold \
                     the last step — drum-attack programs)",
                )
                .range(0.0, 1.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Toggle),
            );

        // Per-step waveform-mask program. Hidden from the auto-renderer but in
        // the descriptor so the sequence round-trips through descriptor-driven
        // save/load and is settable via MCP (mirrors the MSEG segment params).
        for i in 0..SID_SEQ_STEPS as u8 {
            desc = desc.parameter(
                ParameterDescriptor::float(
                    format!("seq_step_{i}"),
                    Param::SidOscillator(SidOscillatorParam::SeqStep(i, 0)),
                    format!("Seq Step {i}"),
                )
                .description(
                    "Waveform mask for this step (bit 0 = triangle, 1 = sawtooth, \
                     2 = pulse, 3 = noise)",
                )
                .range(0.0, 15.0)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Hidden),
            );
        }
        desc = desc.parameter(
            ParameterDescriptor::float(
                "seq_freq_mask",
                Param::SidOscillator(SidOscillatorParam::SeqFreqMask(0)),
                "Seq Freq Mask",
            )
            .description("Bit mask selecting sequence steps with an explicit frequency")
            .range(0.0, 65535.0)
            .default(0.0)
            .modulatable(false)
            .widget(WidgetHint::Hidden),
        );
        for i in 0..SID_SEQ_STEPS as u8 {
            desc = desc.parameter(
                ParameterDescriptor::float(
                    format!("seq_step_freq_{i}"),
                    Param::SidOscillator(SidOscillatorParam::SeqStepFreq(i, 0)),
                    format!("Seq Step Freq {i}"),
                )
                .description("Raw SID frequency register for this sequence step")
                .range(0.0, SID_FREQ_REG_MAX as f32)
                .default(0.0)
                .modulatable(false)
                .widget(WidgetHint::Hidden),
            );
        }

        desc.port(PortDescriptor::control_input("fm", "FM").description(
            "Additive pitch modulation in Freq Reg units (offset-from-base; \
                 the exporter/scripts emit target − base)",
        ))
        .port(
            PortDescriptor::control_input("pwm", "PWM")
                .description("Additive pulse-width modulation in PW Reg units (offset-from-base)"),
        )
        .port(PortDescriptor::audio_input("sync", "Sync").description(
            "Hard-sync master: resets this accumulator on the source's MSB 0→1 \
             rising edge. Connect: another SID Oscillator's MSB output",
        ))
        .port(PortDescriptor::audio_input("ring", "Ring").description(
            "Ring source: the neighbour voice's MSB gate. With the Ring Mod bit \
             set it XORs the triangle folding direction (triangle only, like \
             the chip). Connect: another SID Oscillator's MSB output",
        ))
        // A control input, not a gate input: gate inputs only accept gate-typed
        // sources (PortType::can_drive) and no module emits one — the intended
        // drivers (Script outputs, gates from generative modules) are all
        // audio/control-typed. Gate semantics (>0.5 = on) live in the module.
        .port(PortDescriptor::control_input("test", "Test").description(
            "TEST / hard-restart gate (>0.5 = on): while high the accumulator \
             is zeroed and held and the noise LFSR is reloaded. Drive per-frame \
             from a Script module for hard-restart effects",
        ))
        .port(
            PortDescriptor::audio_output("out", "Out").description("Audio output (DAC'd waveform)"),
        )
        .port(PortDescriptor::audio_output("msb", "MSB").description(
            "Accumulator MSB as a 0/1 gate — the exact signal SID ring/sync read. \
             Connect: → another SID Oscillator's Sync or Ring input",
        ))
    }
}

impl PolyModule for SidOscillator {
    #[allow(clippy::too_many_lines)]
    fn process(
        &mut self,
        inputs: InputPorts<'_>,
        outputs: &mut HashMap<PortName, AudioBuffer>,
        context: &ProcessContext,
    ) {
        // context.sample_rate is the source of truth; the mirror is refreshed
        // every block (plan §6).
        self.sample_rate = context.sample_rate;
        let n_samples = context.samples.as_usize();
        self.output_buffer.resize(n_samples);
        self.msb_buffer.resize(n_samples);

        // Per-oscillator glide: with glide_time > 0 run our own portamento toward
        // the note target; it only affects tuning while tracking the voice pitch.
        if let Some(glided) = self.glide.resolve(context.sample_rate, context.samples)
            && self.track_voice_pitch
        {
            self.tracked_freq_reg = Some(self.freq_to_reg(glided));
        }

        let fm_reader = inputs.reader(PortName::FM, 0.0);
        let pwm_reader = inputs.reader(PortName::PWM, 0.0);
        let sync_reader = inputs.reader(PortName::SYNC, 0.0);
        let ring_reader = inputs.reader(self.ring_port, 0.0);
        let test_reader = inputs.reader(self.test_port, 0.0);

        let os_factor = match self.quality {
            SidQuality::High => OversamplingFactor::X4,
            // Hybrid (plan §2 strategy 3): pure waveforms ride PolyBLEP at
            // host rate; combined-waveform, noise, and live ring-mod
            // selections auto-escalate to the oversampled path they need.
            SidQuality::Fast => {
                if self.needs_oversample(ring_reader.is_connected()) {
                    OversamplingFactor::X4
                } else {
                    OversamplingFactor::X1
                }
            }
        };
        let factor = os_factor.factor();
        if factor != self.last_factor {
            // Half-band state is only valid within a continuous 4x run.
            self.downsampler.reset();
            self.last_factor = factor;
        }
        #[allow(clippy::cast_precision_loss)]
        let gen_rate = f64::from(self.sample_rate.as_f32()) * factor as f64;
        if factor > 1 {
            self.oversample_buffer.resize(n_samples * factor);
        }

        // Block-constant register bases: automation override replaces the
        // base param, then the mod-matrix offset applies on top (the trait's
        // combine order); the additive `fm`/`pwm` CV rides per sample on top.
        #[allow(clippy::cast_precision_loss)]
        let inherited_freq_reg = f64::from(
            self.mod_offsets
                .effective("freq_reg", self.effective_freq_reg() as f32),
        );
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let base_pw = self
            .mod_offsets
            .effective(
                "pw_reg",
                self.override_pw_reg.unwrap_or(self.pulse_width_reg) as f32,
            )
            .round()
            .clamp(0.0, SID_PW_REG_MAX as f32) as u32;
        // Block-constant factors, hoisted out of the sample loop.
        let reg_to_inc = self.reg_to_inc(gen_rate);
        let mut live_base_freq = self.live_freq_reg(inherited_freq_reg);
        let mut inc = Self::acc_increment(live_base_freq, reg_to_inc);
        let mut dt = Self::inc_to_dt(inc);
        let level = self
            .mod_offsets
            .effective("level", self.override_level.unwrap_or(self.level).as_f32());

        // Waveform-sequence frame clock (50/60 Hz driver frames). The derived
        // state (pure classification, NoiseLock) is frame-constant.
        let seq_active = self.seq_length > 0;
        let frame_len = self.sample_rate.as_f32() / self.clock.frame_rate_hz();
        let (mut mask, mut pure, mut noise_locked) = self.mask_state();

        // With the test/ring ports unconnected their per-sample reads are
        // block-constant — resolve them (and the TEST rising edge) once.
        let test_connected = test_reader.is_connected();
        if !test_connected {
            if self.test && !self.prev_test_active {
                self.lfsr = self.noise_seed.as_u32();
            }
            self.prev_test_active = self.test;
        }
        let ring_connected = ring_reader.is_connected();

        for i in 0..n_samples {
            if seq_active {
                self.frame_pos += 1.0;
                if self.frame_pos >= frame_len {
                    self.frame_pos -= frame_len;
                    self.frame_count += 1;
                    (mask, pure, noise_locked) = self.mask_state();
                    live_base_freq = self.live_freq_reg(inherited_freq_reg);
                    inc = Self::acc_increment(live_base_freq, reg_to_inc);
                    dt = Self::inc_to_dt(inc);
                }
            }

            // TEST is the param OR the gate input; its rising edge is the
            // hard restart that reloads the LFSR to its deterministic seed.
            let test_active = if test_connected {
                let active = self.test || test_reader.get(i) > 0.5;
                if active && !self.prev_test_active {
                    self.lfsr = self.noise_seed.as_u32();
                }
                self.prev_test_active = active;
                active
            } else {
                self.test
            };

            // Ring source: the neighbour voice's MSB gate (triangle-only XOR).
            // A fold flip inverts the triangle (`tri → -tri`) — a step the
            // PolyBLEP below band-limits (computed after `pw` is resolved).
            let ring_high = self.ring_mod && ring_connected && ring_reader.get(i) > 0.5;
            let ring_flip = ring_high != self.prev_ring_high;
            let ring_prev = self.prev_ring_high;
            self.prev_ring_high = ring_high;

            // Additive CV in raw register units (offset-from-base — plan §3).
            if fm_reader.is_connected() {
                let reg = (live_base_freq + f64::from(fm_reader.get(i)))
                    .clamp(0.0, f64::from(SID_FREQ_REG_MAX));
                inc = Self::acc_increment(reg, reg_to_inc);
                dt = Self::inc_to_dt(inc);
            }
            let pw = if pwm_reader.is_connected() {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                {
                    #[allow(clippy::cast_precision_loss)]
                    let max = SID_PW_REG_MAX as f32;
                    (base_pw as f32 + pwm_reader.get(i)).round().clamp(0.0, max) as u32
                }
            } else {
                base_pw
            };

            // Ring fold flip → a one-sided PolyBLEP on the triangle-inversion
            // step (post − pre fold), the same band-limiting the sync reset
            // uses. Replaces the old linear crossfade, which low-passed the
            // real HF sidebands (§11 shoulder-texture). Computed here at the
            // pre-reset accumulator so a coincident sync reset superposes; void
            // while TEST holds the output frozen.
            if ring_flip && !test_active {
                let acc24_edge = (self.acc >> ACC_FRAC_BITS) as u32;
                let pre = self.dac_sample(mask, acc24_edge, pw, ring_prev, test_active);
                let post = self.dac_sample(mask, acc24_edge, pw, ring_high, test_active);
                // poly_blep is normalized for a ±2 step, hence the h/2 scale.
                self.pending_ring_step = (post - pre) * 0.5;
                // 1x: the flip lands mid-sample (gate carries no sub-sample
                // edge). 4x: on the first generated sub-sample (distance 0),
                // matching the sync convention.
                self.pending_ring_d = if factor == 1 { 0.5 } else { 0.0 };
            }

            // Hard sync: the master's MSB gate rising through 0.5. For a ramp-ish
            // master the crossing fraction recovers the sub-sample edge position;
            // for a clean 0/1 gate it degrades to the sample midpoint. The edge
            // history is tracked whenever the input is connected — even with the
            // SYNC bit off — so enabling it mid-note never fires a spurious reset.
            if sync_reader.is_connected() {
                let sync_val = sync_reader.get(i);
                // TEST dominates: while the accumulator is held at zero a sync
                // edge must not reset it or seed a BLEP step into the frozen
                // output (the edge history still advances below).
                if self.hard_sync
                    && !test_active
                    && crate::math::rising_edge(sync_val, self.prev_sync)
                {
                    let rise = sync_val - self.prev_sync;
                    let t = if rise > 1e-6 {
                        ((0.5 - self.prev_sync) / rise).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    // The edge happened `1 - t` of a host sample ago; the reset
                    // accumulator has since advanced that fraction.
                    #[allow(clippy::cast_precision_loss)]
                    let frac = (1.0 - t) * factor as f32;
                    let acc24_pre = (self.acc >> ACC_FRAC_BITS) as u32;
                    // Measure the pre/post pair at the post-flip fold (`ring_high`):
                    // in the canonical SID topology one master gate feeds both
                    // sync and ring, so a reset and fold flip coincide and their
                    // BLEP steps superpose (ring: pre_fold→post_fold at acc_pre;
                    // sync: acc_pre→acc_post at post_fold).
                    let pre = self.dac_sample(mask, acc24_pre, pw, ring_high, test_active);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    {
                        self.acc = ((f64::from(frac) * inc as f64) as u64) & ACC_MASK;
                    }
                    let acc24_post = (self.acc >> ACC_FRAC_BITS) as u32;
                    let post = self.dac_sample(mask, acc24_post, pw, ring_high, test_active);
                    // One-sided PolyBLEP residual for the reset discontinuity
                    // (plan §3: band-limit the hard-sync reset). poly_blep is
                    // normalized for a ±2 step, hence the h/2 scale.
                    self.pending_sync_step = (post - pre) * 0.5;
                    // Residual evaluation distance: at 1x the output steps at
                    // the true edge within this sample (frac ∈ [0, 1]); at 4x
                    // the previous host sample already ran through the edge,
                    // so the audible step lands exactly on the first
                    // generated sub-sample (distance 0).
                    self.pending_sync_d = if factor == 1 { frac.min(1.0) } else { 0.0 };
                }
                self.prev_sync = sync_val;
            }

            // TEST zeroes and holds the accumulator (plan §2) — applied before
            // the sample is generated, like the chip's immediate register
            // write. Any pending sync-reset / ring-flip step is void: the
            // output is frozen.
            if test_active {
                self.acc = 0;
                self.pending_sync_step = 0.0;
                self.pending_ring_step = 0.0;
            }

            // MSB gate output, sampled at host rate (pre-advance).
            let acc24_now = (self.acc >> ACC_FRAC_BITS) as u32;
            self.msb_buffer[i] = if acc24_now & ACC_MSB != 0 { 1.0 } else { 0.0 };

            for k in 0..factor {
                let acc24 = (self.acc >> ACC_FRAC_BITS) as u32;
                #[allow(clippy::cast_precision_loss)]
                let p = (self.acc as f64 / ACC_CYCLE) as f32;

                let mut sample = self.dac_sample(mask, acc24, pw, ring_high, test_active);
                if !test_active {
                    sample += Self::pure_waveform_residual(pure, p, dt, pw, ring_high);
                }
                if self.pending_sync_step != 0.0 {
                    // After-side PolyBLEP polynomial r(d) = 2d − d² − 1 at the
                    // stored step distance. (Evaluating poly_blep at the
                    // accumulator phase breaks at 4x: the phase exceeds the
                    // sub-sample window for most edge positions and the
                    // correction would silently drop.)
                    let d = self.pending_sync_d;
                    sample += self.pending_sync_step * (2.0 * d - d * d - 1.0);
                    self.pending_sync_step = 0.0;
                }
                if self.pending_ring_step != 0.0 {
                    // Same after-side PolyBLEP for the fold-flip step. Applied
                    // on the first generated sub-sample after the flip; a
                    // coincident sync step (above) superposes.
                    let d = self.pending_ring_d;
                    sample += self.pending_ring_step * (2.0 * d - d * d - 1.0);
                    self.pending_ring_step = 0.0;
                }
                if factor == 1 {
                    self.output_buffer[i] = sample * level;
                } else {
                    self.oversample_buffer[i * factor + k] = sample;
                }

                if !test_active {
                    self.advance_acc(inc, noise_locked);
                }
            }
        }

        if factor > 1 {
            // Decimate in chunks the Downsampler's fixed intermediate stage can
            // hold — one oversized call would silently drop the block's tail.
            let mut out_off = 0;
            while out_off < n_samples {
                let out_chunk = (n_samples - out_off).min(MAX_DECIMATE_OUT);
                let in_off = out_off * factor;
                self.downsampler.process(
                    &self.oversample_buffer.as_slice()[in_off..in_off + out_chunk * factor],
                    &mut self.output_buffer.as_mut_slice()[out_off..out_off + out_chunk],
                    os_factor,
                );
                out_off += out_chunk;
            }
            for i in 0..n_samples {
                self.output_buffer[i] *= level;
            }
        }

        if self.dc_block {
            // One-pole DC blocker, y[n] = x[n] - x[n-1] + R*y[n-1] (~16 Hz,
            // the C64 output coupling): combined 6581 waveforms sit on real
            // DC (§11), and near-silent bus-collapse combos read as -1 DC
            // through the DAC. R is derived per block from the live rate.
            // Deliberately the classic two-state form, NOT the shared
            // FilterState::dc_blocker (a leaky-integrator HP with a different
            // curve) — this response is what the §11 A/B gate will measure.
            let r = (1.0 - std::f32::consts::TAU * DC_BLOCK_HZ / self.sample_rate.as_f32())
                .clamp(0.0, 1.0);
            let (mut x1, mut y1) = (self.dc_x1, self.dc_y1);
            for i in 0..n_samples {
                let x = self.output_buffer[i];
                let y = x - x1 + r * y1;
                x1 = x;
                y1 = y;
                self.output_buffer[i] = y;
            }
            self.dc_x1 = x1;
            // Flush subnormals: on DC-flat stretches y decays geometrically
            // forever, and threads without an FTZ/DAZ guard (offline renders)
            // would pay the denormal penalty every sample (cf. waveguide.rs).
            self.dc_y1 = if y1.abs() < 1e-20 { 0.0 } else { y1 };
        }

        if let Some(out) = outputs.get_mut(&PortName::OUT) {
            out.copy_from(&self.output_buffer);
        }
        if let Some(msb) = outputs.get_mut(&self.msb_port) {
            msb.copy_from(&self.msb_buffer);
        }
    }

    fn set_param(&mut self, param: Param) {
        if let Param::SidOscillator(p) = param {
            match p {
                SidOscillatorParam::Triangle(b) => self.triangle = b,
                SidOscillatorParam::Sawtooth(b) => self.sawtooth = b,
                SidOscillatorParam::Pulse(b) => self.pulse = b,
                SidOscillatorParam::Noise(b) => self.noise = b,
                SidOscillatorParam::NoiseSeed(seed) => self.noise_seed = seed,
                SidOscillatorParam::FreqReg(v) => self.freq_reg = v.min(SID_FREQ_REG_MAX),
                SidOscillatorParam::TrackVoicePitch(b) => self.track_voice_pitch = b,
                SidOscillatorParam::PulseWidthReg(v) => {
                    self.pulse_width_reg = v.min(SID_PW_REG_MAX)
                }
                SidOscillatorParam::Test(b) => self.test = b,
                SidOscillatorParam::RingMod(b) => self.ring_mod = b,
                SidOscillatorParam::HardSync(b) => self.hard_sync = b,
                SidOscillatorParam::Model(m) => self.model = m,
                SidOscillatorParam::Clock(c) => self.clock = c,
                SidOscillatorParam::Quality(q) => {
                    // A factor change invalidates the half-band filter state
                    // (Downsampler's own contract); reset is RT-safe.
                    if q != self.quality {
                        self.downsampler.reset();
                    }
                    self.quality = q;
                }
                SidOscillatorParam::DcBlock(b) => {
                    // Fresh filter state on re-enable: stale x1/y1 from the
                    // last active stretch would fire a one-shot pop.
                    if b && !self.dc_block {
                        self.dc_x1 = 0.0;
                        self.dc_y1 = 0.0;
                    }
                    self.dc_block = b;
                }
                SidOscillatorParam::Level(g) => self.level = g,
                SidOscillatorParam::GlideTime(t) => self.glide.set_time(t),
                SidOscillatorParam::SeqLength(n) => {
                    self.seq_length = n.min(SID_SEQ_STEPS as u8);
                }
                SidOscillatorParam::SeqRate(n) => self.seq_rate = n.clamp(1, 16),
                SidOscillatorParam::SeqLoop(b) => self.seq_loop = b,
                SidOscillatorParam::SeqStep(i, mask) => {
                    if let Some(step) = self.seq_steps.get_mut(usize::from(i)) {
                        *step = mask & 0xF;
                    }
                }
                SidOscillatorParam::SeqFreqMask(mask) => {
                    self.seq_freq_mask = mask.min(u32::from(u16::MAX)) as u16;
                }
                SidOscillatorParam::SeqStepFreq(i, frequency) => {
                    if let Some(step) = self.seq_step_freqs.get_mut(usize::from(i)) {
                        *step = frequency.min(SID_FREQ_REG_MAX);
                    }
                }
            }
        }
    }

    fn get_param(&self, param: &Param) -> Option<f32> {
        if let Param::SidOscillator(p) = param {
            Some(self.current(p).as_f32())
        } else {
            None
        }
    }

    fn get_params(&self) -> Vec<Param> {
        let mut templates = vec![
            SidOscillatorParam::Triangle(false),
            SidOscillatorParam::Sawtooth(false),
            SidOscillatorParam::Pulse(false),
            SidOscillatorParam::Noise(false),
            SidOscillatorParam::NoiseSeed(SidNoiseSeed::DEFAULT),
            SidOscillatorParam::FreqReg(0),
            SidOscillatorParam::TrackVoicePitch(false),
            SidOscillatorParam::PulseWidthReg(0),
            SidOscillatorParam::Test(false),
            SidOscillatorParam::RingMod(false),
            SidOscillatorParam::HardSync(false),
            SidOscillatorParam::Model(SidModel::Mos6581),
            SidOscillatorParam::Clock(SidClock::Pal),
            SidOscillatorParam::Quality(SidQuality::Fast),
            SidOscillatorParam::DcBlock(true),
            SidOscillatorParam::Level(Gain::UNITY),
            SidOscillatorParam::GlideTime(Seconds::ZERO),
            SidOscillatorParam::SeqLength(0),
            SidOscillatorParam::SeqRate(1),
            SidOscillatorParam::SeqLoop(false),
            SidOscillatorParam::SeqFreqMask(0),
        ];
        #[allow(clippy::cast_possible_truncation)]
        templates.extend((0..SID_SEQ_STEPS as u8).map(|i| SidOscillatorParam::SeqStep(i, 0)));
        templates.extend((0..SID_SEQ_STEPS as u8).map(|i| SidOscillatorParam::SeqStepFreq(i, 0)));
        templates
            .into_iter()
            .map(|t| Param::SidOscillator(self.current(&t)))
            .collect()
    }

    fn module_type(&self) -> ModuleType {
        ModuleType::SidOscillator
    }

    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> {
        Some(&mut self.mod_offsets)
    }

    fn set_param_override(&mut self, param: Param) {
        if let Param::SidOscillator(p) = param {
            match p {
                SidOscillatorParam::FreqReg(v) => {
                    self.override_freq_reg = Some(v.min(SID_FREQ_REG_MAX));
                }
                SidOscillatorParam::PulseWidthReg(v) => {
                    self.override_pw_reg = Some(v.min(SID_PW_REG_MAX));
                }
                SidOscillatorParam::Level(g) => self.override_level = Some(g),
                // Remaining params are excluded from automation either by
                // kind (bool/enum fail is_automatable) or by modulatable(false)
                // (the u8 sequence params) — extend the arms above if one is
                // ever opted in.
                _ => {}
            }
        }
    }

    fn clear_param_overrides(&mut self) {
        self.override_freq_reg = None;
        self.override_pw_reg = None;
        self.override_level = None;
    }

    fn reset(&mut self) {
        // Power-on state. Note this is the *module* reset (graph (re)build),
        // not note_on — the chip state free-runs across notes (plan §5).
        self.acc = 0;
        self.lfsr = self.noise_seed.as_u32();
        self.prev_sync = 0.0;
        self.pending_sync_step = 0.0;
        self.pending_sync_d = 0.0;
        self.prev_test_active = false;
        self.prev_ring_high = false;
        self.pending_ring_step = 0.0;
        self.pending_ring_d = 0.0;
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
        self.frame_pos = 0.0;
        self.frame_count = 0;
        self.downsampler.reset();
        self.glide.reset();
    }

    fn note_on(&mut self, note: MidiNote, _velocity: Velocity) {
        // Latch pitch and restart the waveform-sequence program (driver-table
        // data, not chip state) — the accumulator and LFSR free-run;
        // deterministic restarts are the TEST bit's job (plan §5).
        if self.track_voice_pitch {
            self.tracked_freq_reg = Some(self.freq_to_reg(note.to_frequency()));
        }
        self.frame_pos = 0.0;
        self.frame_count = 0;
    }

    fn set_voice_pitch(&mut self, pitch: VoicePitch) {
        self.glide.store(pitch);
        if self.track_voice_pitch {
            self.tracked_freq_reg = Some(self.freq_to_reg(pitch.played));
        }
    }

    fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }

    fn box_clone(&self) -> Box<dyn PolyModule> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use synth_core::SampleCount;

    fn sid_with(mask: (bool, bool, bool), freq_reg: u32) -> SidOscillator {
        let mut sid = SidOscillator::new();
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Triangle(mask.0)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Sawtooth(mask.1)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Pulse(mask.2)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::TrackVoicePitch(
            false,
        )));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::FreqReg(freq_reg)));
        sid
    }

    fn render(sid: &mut SidOscillator, n: usize) -> (Vec<f32>, Vec<f32>) {
        render_with_inputs(sid, n, InputPorts::empty())
    }

    /// Pulse duty cycle: skip one block of BLEP transients, then the share of
    /// positive samples over a 2048-sample window (the shared measurement all
    /// duty assertions use, +/- 0.03 tolerance at the call sites).
    fn measured_duty(sid: &mut SidOscillator) -> f32 {
        let _ = render(sid, 512);
        let (out, _) = render(sid, 2048);
        #[allow(clippy::cast_precision_loss)]
        {
            out.iter().filter(|&&v| v > 0.0).count() as f32 / out.len() as f32
        }
    }

    fn render_with_inputs(
        sid: &mut SidOscillator,
        n: usize,
        inputs: InputPorts<'_>,
    ) -> (Vec<f32>, Vec<f32>) {
        let ctx = ProcessContext {
            samples: SampleCount::new(n),
            ..ProcessContext::default()
        };
        let mut outs = HashMap::new();
        outs.insert(PortName::OUT, AudioBuffer::new(n));
        outs.insert(PortName::intern("msb"), AudioBuffer::new(n));
        sid.process(inputs, &mut outs, &ctx);
        let collect = |name: PortName| {
            let b = &outs[&name];
            (0..b.len()).map(|i| b[i]).collect::<Vec<f32>>()
        };
        (collect(PortName::OUT), collect(PortName::intern("msb")))
    }

    /// `freq_reg = round(f * 2^24 / f_clock)` — the chip pitch mapping (plan §5),
    /// only applied while `TrackVoicePitch` is on.
    #[test]
    fn voice_pitch_maps_to_freq_reg() {
        let mut sid = SidOscillator::new();
        sid.set_voice_pitch(VoicePitch::tracking(Hertz::new(440.0)));
        // PAL: 440 * 16777216 / 985248 = 7493.06… → 7493
        assert_eq!(sid.effective_freq_reg(), 7493);

        sid.set_param(Param::SidOscillator(SidOscillatorParam::Clock(
            SidClock::Ntsc,
        )));
        sid.set_voice_pitch(VoicePitch::tracking(Hertz::new(440.0)));
        // NTSC: 440 * 16777216 / 1022727 = 7218.4… → 7218
        assert_eq!(sid.effective_freq_reg(), 7218);

        // Playback never mutates the authored FreqReg param (the save path
        // serializes it) — only the tracked register moves.
        assert_eq!(sid.freq_reg, 0);
        assert_eq!(
            sid.get_param(&Param::SidOscillator(SidOscillatorParam::FreqReg(0))),
            Some(0.0)
        );

        // Ring/sync-source mode: the played pitch is ignored (plan §4).
        sid.set_param(Param::SidOscillator(SidOscillatorParam::TrackVoicePitch(
            false,
        )));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::FreqReg(1234)));
        sid.set_voice_pitch(VoicePitch::tracking(Hertz::new(880.0)));
        assert_eq!(sid.effective_freq_reg(), 1234);
        sid.note_on(MidiNote::A4, Velocity::MAX);
        assert_eq!(sid.effective_freq_reg(), 1234);
    }

    /// The sawtooth is a rising ramp from the top 12 accumulator bits that
    /// wraps once per accumulator cycle.
    #[test]
    fn sawtooth_is_rising_ramp_with_wraps() {
        // freq_reg 7493 (A4, PAL) → cycle ≈ 100 host samples @ 44.1 kHz.
        // 8580: its linear DAC keeps the digital ramp strictly monotonic
        // (the 6581's kinked DAC dips at bit carries by design).
        let mut sid = sid_with((false, true, false), 7493);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Model(
            SidModel::Mos8580,
        )));
        let (out, msb) = render(&mut sid, 400);

        let mut wraps = 0;
        let mut rising = 0;
        for w in out.windows(2) {
            if w[1] < w[0] - 1.0 {
                wraps += 1;
            } else if w[1] >= w[0] {
                rising += 1;
            }
        }
        assert!((3..=4).contains(&wraps), "expected ~4 wraps, got {wraps}");
        assert!(
            rising >= 390,
            "saw should rise monotonically between wraps, rising = {rising}"
        );
        // MSB gate: high in the second half of each cycle.
        assert!(msb.contains(&0.0) && msb.contains(&1.0));
    }

    /// Pulse threshold: high while `(acc >> 12) >= pw_reg`, so the duty cycle
    /// is `1 - pw/4096`.
    #[test]
    fn pulse_duty_follows_pw_register() {
        let duty = |pw: u32| {
            let mut sid = sid_with((false, false, true), 7493);
            sid.set_param(Param::SidOscillator(SidOscillatorParam::PulseWidthReg(pw)));
            measured_duty(&mut sid)
        };
        assert!((duty(1024) - 0.75).abs() < 0.03, "pw 1024 → 75% high");
        assert!((duty(2048) - 0.5).abs() < 0.03, "pw 2048 → 50% high");
        assert!((duty(3072) - 0.25).abs() < 0.03, "pw 3072 → 25% high");
    }

    /// Triangle folds at the accumulator MSB: value rises to +1 mid-cycle and
    /// falls back — twice the ramp slope, peak aligned with the MSB edge.
    #[test]
    fn triangle_folds_at_msb() {
        let mut sid = sid_with((true, false, false), 7493);
        let (out, msb) = render(&mut sid, 200);

        let max = out.iter().copied().fold(f32::MIN, f32::max);
        let min = out.iter().copied().fold(f32::MAX, f32::min);
        assert!(max > 0.95, "triangle should reach near +1, max = {max}");
        assert!(min < -0.95, "triangle should reach near -1, min = {min}");
        // The fold points sit on the MSB edges: near +1 at every rise (fold
        // down begins), near -1 at every fall (the wrap).
        for (i, w) in msb.windows(2).enumerate() {
            if w[0] == 0.0 && w[1] == 1.0 {
                assert!(
                    out[i] > 0.9,
                    "triangle should peak at the MSB rise (sample {i}): {}",
                    out[i]
                );
            }
            if w[0] == 1.0 && w[1] == 0.0 {
                assert!(
                    out[i + 1] < -0.9,
                    "triangle should trough at the wrap (sample {}): {}",
                    i + 1,
                    out[i + 1]
                );
            }
        }
    }

    /// TEST zeroes and holds the accumulator: the MSB stays low and the saw
    /// output freezes at its bottom value.
    #[test]
    fn test_bit_zeroes_and_holds_accumulator() {
        let mut sid = sid_with((false, true, false), 7493);
        // Chip-state test: bypass the output DC blocker (its decay would
        // read as movement on the frozen value).
        sid.set_param(Param::SidOscillator(SidOscillatorParam::DcBlock(false)));
        let _ = render(&mut sid, 300);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Test(true)));
        let (out, msb) = render(&mut sid, 200);
        assert!(
            msb.iter().all(|&v| v == 0.0),
            "MSB must stay low under TEST"
        );
        assert!(
            out.iter().all(|&v| (v - out[0]).abs() < 1e-6),
            "output must freeze under TEST"
        );
        assert_eq!(sid.acc, 0, "accumulator must be held at zero");
    }

    /// Hard sync resets the slave accumulator on the master MSB's 0→1 edge —
    /// the slave's own cycle never completes.
    #[test]
    fn hard_sync_resets_on_msb_rising_edge() {
        // Master gate: one MSB rise every 64 samples.
        let n = 512;
        let mut master = AudioBuffer::new(n);
        for i in 0..n {
            master[i] = if (i / 32) % 2 == 1 { 1.0 } else { 0.0 };
        }

        // Slave much slower than the master: free-running it would climb the
        // full saw ramp; synced it restarts every 64 samples.
        let mut sid = sid_with((false, true, false), 1000);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::HardSync(true)));

        let (free, _) = render(&mut sid, n);
        let free_max = free.iter().copied().fold(f32::MIN, f32::max);

        sid.reset();
        let inputs = [(PortName::SYNC, &master)];
        let (synced, _) = render_with_inputs(&mut sid, n, InputPorts::new(&inputs));
        let synced_max = synced.iter().copied().fold(f32::MIN, f32::max);

        assert!(
            free_max > 0.0,
            "free-running slave should pass mid-ramp, max = {free_max}"
        );
        assert!(
            synced_max < free_max - 0.2,
            "synced slave must be clamped below the free run: {synced_max} vs {free_max}"
        );
        // Without the SYNC bit the input is ignored.
        sid.set_param(Param::SidOscillator(SidOscillatorParam::HardSync(false)));
        sid.reset();
        let (unsynced, _) = render_with_inputs(&mut sid, n, InputPorts::new(&inputs));
        let unsynced_max = unsynced.iter().copied().fold(f32::MIN, f32::max);
        assert!(
            (unsynced_max - free_max).abs() < 1e-4,
            "SYNC off must ignore the input"
        );
    }

    /// Additive CV convention (plan §3): `fm`/`pwm` offset the raw registers,
    /// so a CV of `target - base` reaches an absolute register value.
    #[test]
    fn fm_and_pwm_cv_offset_registers() {
        // fm: base 1000 + CV 6493 ≡ freq_reg 7493 — compare against a base of
        // 7493 with no CV: identical output.
        let n = 400;
        let mut fm_buf = AudioBuffer::new(n);
        for i in 0..n {
            fm_buf[i] = 6493.0;
        }
        let mut modulated = sid_with((false, true, false), 1000);
        let inputs = [(PortName::FM, &fm_buf)];
        let (with_cv, _) = render_with_inputs(&mut modulated, n, InputPorts::new(&inputs));

        let mut reference = sid_with((false, true, false), 7493);
        let (direct, _) = render(&mut reference, n);
        for (a, b) in with_cv.iter().zip(&direct) {
            assert!((a - b).abs() < 1e-6, "fm offset must equal direct register");
        }

        // pwm: base 1024 + CV 1024 ≡ pw 2048 — identical output to the direct
        // register (same offset-from-base check as the fm half).
        let mut pwm_buf = AudioBuffer::new(n);
        for i in 0..n {
            pwm_buf[i] = 1024.0;
        }
        let mut modulated = sid_with((false, false, true), 7493);
        modulated.set_param(Param::SidOscillator(SidOscillatorParam::PulseWidthReg(
            1024,
        )));
        let inputs = [(PortName::PWM, &pwm_buf)];
        let (with_cv, _) = render_with_inputs(&mut modulated, n, InputPorts::new(&inputs));

        let mut reference = sid_with((false, false, true), 7493);
        reference.set_param(Param::SidOscillator(SidOscillatorParam::PulseWidthReg(
            2048,
        )));
        let (direct, _) = render(&mut reference, n);
        for (a, b) in with_cv.iter().zip(&direct) {
            assert!(
                (a - b).abs() < 1e-6,
                "pwm offset must equal direct register"
            );
        }
    }

    /// The LFSR shifts `bit22 XOR bit17` into bit 0. From the all-ones seed the
    /// feedback is 0 until bit 17 has drained, then 1s return — a fixed,
    /// testable prefix (and never the stuck all-zero state).
    #[test]
    fn lfsr_sequence_from_fixed_seed() {
        let mut sid = SidOscillator::new();
        assert_eq!(sid.lfsr, 0x7F_FFFF);
        sid.clock_lfsr(false);
        assert_eq!(sid.lfsr, 0x7F_FFFE, "1 XOR 1 = 0 shifted in");
        sid.clock_lfsr(false);
        assert_eq!(sid.lfsr, 0x7F_FFFC);
        // At clock 19 the zeros have drained past bit 17 while bit 22 is
        // still 1 → the feedback shifts a 1 back in.
        for _ in 0..17 {
            sid.clock_lfsr(false);
        }
        assert_eq!(sid.lfsr & 1, 1, "bit22=1 XOR bit17=0 shifts in a 1");
        // Long-run sanity: never reaches the stuck all-zero state.
        for _ in 0..100_000 {
            sid.clock_lfsr(false);
            assert_ne!(sid.lfsr, 0, "LFSR must never go all-zero");
        }
    }

    #[test]
    fn configurable_noise_seed_has_golden_prefix_and_test_reload() {
        let mut sid = SidOscillator::new();
        sid.set_param(Param::SidOscillator(SidOscillatorParam::NoiseSeed(
            SidNoiseSeed::new(0x12_3456),
        )));
        sid.reset();
        assert_eq!(sid.lfsr, 0x12_3456);
        sid.clock_lfsr(false);
        assert_eq!(sid.lfsr, 0x24_68AD);
        sid.clock_lfsr(false);
        assert_eq!(sid.lfsr, 0x48_D15A);

        sid.set_param(Param::SidOscillator(SidOscillatorParam::Test(true)));
        let _ = render(&mut sid, 1);
        assert_eq!(sid.lfsr, 0x12_3456, "TEST reloads the configured seed");

        let mut second = SidOscillator::new();
        second.set_param(Param::SidOscillator(SidOscillatorParam::NoiseSeed(
            SidNoiseSeed::new(1),
        )));
        second.reset();
        second.clock_lfsr(false);
        assert_eq!(second.lfsr, 2);
        second.clock_lfsr(false);
        assert_eq!(second.lfsr, 4);
    }

    #[test]
    fn noise_seed_rejects_the_zero_lock_state() {
        assert_eq!(SidNoiseSeed::new(0).as_u32(), 1);
        assert_eq!(SidNoiseSeed::new(u32::MAX).as_u32(), SidNoiseSeed::MAX);
    }

    /// Noise is deterministic from the fixed seed: two fresh modules render
    /// identical noise, its rate tracks the frequency register, and a TEST
    /// pulse restores the exact power-on sequence (plan §2/§6).
    #[test]
    fn noise_is_deterministic_and_test_reloads() {
        let noise_sid = || {
            let mut sid = sid_with((false, false, false), 4000);
            sid.set_param(Param::SidOscillator(SidOscillatorParam::Noise(true)));
            // Bit-exact sequence comparisons: bypass the DC blocker, whose
            // state depends on each render's history.
            sid.set_param(Param::SidOscillator(SidOscillatorParam::DcBlock(false)));
            sid
        };
        let n = 2048;
        let (r1, _) = render(&mut noise_sid(), n);
        let (r2, _) = render(&mut noise_sid(), n);
        assert_eq!(r1, r2, "fresh modules must render identical noise");
        assert!(
            r1.windows(2).any(|w| (w[0] - w[1]).abs() > 1e-3),
            "noise must actually vary"
        );

        // Pitch tracking: a higher freq_reg clocks the LFSR faster.
        let changes = |reg: u32| {
            let mut sid = sid_with((false, false, false), reg);
            sid.set_param(Param::SidOscillator(SidOscillatorParam::Noise(true)));
            sid.set_param(Param::SidOscillator(SidOscillatorParam::DcBlock(false)));
            let (out, _) = render(&mut sid, n);
            out.windows(2).filter(|w| w[0] != w[1]).count()
        };
        assert!(
            changes(60000) > changes(2000),
            "noise rate must track the frequency register"
        );

        // TEST reload: run a while, pulse TEST, release — the sequence
        // restarts exactly from the power-on state.
        let mut sid = noise_sid();
        let (from_poweron, _) = render(&mut sid, n);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Test(true)));
        let _ = render(&mut sid, 64);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Test(false)));
        let (after_test, _) = render(&mut sid, n);
        // Noise rides the auto-oversampled path; skip the decimator's finite
        // FIR memory (~8 host samples of tail from the pre-TEST audio), after
        // which the restored chip state must reproduce power-on exactly.
        assert_eq!(
            from_poweron[32..],
            after_test[32..],
            "TEST must restore the deterministic power-on noise"
        );
    }

    /// Noise combined with another waveform corrupts the LFSR (NoiseLock) —
    /// the state stays degenerate after the combination ends, until TEST.
    #[test]
    fn noise_lock_corrupts_until_test() {
        let n = 2048;
        let mut sid = sid_with((false, true, false), 4000);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Noise(true)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::DcBlock(false)));
        // saw+noise → bus conflict pulls the tap bits low.
        let _ = render(&mut sid, n);
        assert_eq!(sid.lfsr & LFSR_TAP_MASK, 0, "taps must be pulled low");

        // Back to noise-only: the corrupted register does NOT match the
        // deterministic power-on sequence...
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Sawtooth(false)));
        let (corrupted, _) = render(&mut sid, n);
        let mut fresh = sid_with((false, false, false), 4000);
        fresh.set_param(Param::SidOscillator(SidOscillatorParam::Noise(true)));
        fresh.set_param(Param::SidOscillator(SidOscillatorParam::DcBlock(false)));
        let (clean, _) = render(&mut fresh, n);
        assert_ne!(corrupted, clean, "lock must leave the LFSR corrupted");

        // ...until a TEST pulse reloads it.
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Test(true)));
        let _ = render(&mut sid, 64);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Test(false)));
        let (restored, _) = render(&mut sid, n);
        // Skip the decimator's finite FIR memory (see the TEST-reload test).
        assert_eq!(restored[32..], clean[32..], "TEST must clear the lock");
    }

    /// Ring mod XORs the triangle folding direction with the ring input's MSB
    /// — and affects ONLY the triangle (plan §3).
    #[test]
    fn ring_mod_affects_only_triangle() {
        let n = 512;
        // Ring source gate: toggling at an unrelated rate.
        let mut ring = AudioBuffer::new(n);
        for i in 0..n {
            ring[i] = if (i / 37) % 2 == 1 { 1.0 } else { 0.0 };
        }

        let render_ring = |mask: (bool, bool, bool), ring_on: bool| {
            let mut sid = sid_with(mask, 7493);
            sid.set_param(Param::SidOscillator(SidOscillatorParam::RingMod(ring_on)));
            let inputs = [(PortName::intern("ring"), &ring)];
            let (out, _) = render_with_inputs(&mut sid, n, InputPorts::new(&inputs));
            out
        };

        // Triangle: ring changes the output.
        let tri_dry = render_ring((true, false, false), false);
        let tri_ring = render_ring((true, false, false), true);
        assert_ne!(tri_dry, tri_ring, "ring must modulate the triangle");

        // Sawtooth: ring has no effect.
        let saw_dry = render_ring((false, true, false), false);
        let saw_ring = render_ring((false, true, false), true);
        assert_eq!(saw_dry, saw_ring, "ring must not touch the sawtooth");
    }

    /// A constantly-high ring input holds the triangle fold inverted — with
    /// the linear 8580 DAC the digital fold complement makes `tri_ring ≈ -tri`
    /// (within one bus LSB), and the BLAMP corner residual must flip sign with
    /// it: a wrong-sign residual doubles the corner error well past the LSB
    /// bound. Same 4x path (Quality High) for both renders, DC blocker off.
    #[test]
    fn ring_high_triangle_is_negation_within_lsb() {
        let n = 512;
        let mut high = AudioBuffer::new(n);
        for i in 0..n {
            high[i] = 1.0;
        }
        let render_tri = |ring_on: bool| {
            let mut sid = sid_with((true, false, false), 7493);
            sid.set_param(Param::SidOscillator(SidOscillatorParam::Model(
                SidModel::Mos8580,
            )));
            sid.set_param(Param::SidOscillator(SidOscillatorParam::Quality(
                SidQuality::High,
            )));
            sid.set_param(Param::SidOscillator(SidOscillatorParam::DcBlock(false)));
            sid.set_param(Param::SidOscillator(SidOscillatorParam::RingMod(ring_on)));
            let inputs = [(PortName::intern("ring"), &high)];
            let (out, _) = render_with_inputs(&mut sid, n, InputPorts::new(&inputs));
            out
        };
        let dry = render_tri(false);
        let rung = render_tri(true);
        // Skip the initial fold-flip fade (ring goes 0->high at sample 0).
        // Bound: one 12-bit bus LSB (2/4095 ~ 4.9e-4) + decimation ripple.
        // With the residual sign correct the deviation sits exactly on the
        // one-LSB fold-complement bound (2/4095 ~ 4.9e-4, measured 4.88e-4);
        // a wrong-sign corner residual doubles it (measured 8.9e-4).
        let max_dev = dry
            .iter()
            .zip(&rung)
            .skip(16)
            .map(|(a, b)| (a + b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_dev < 6.5e-4,
            "ring-high triangle must negate within one bus LSB: {max_dev}"
        );
    }

    /// §11 ring-mod brightness: a ring-modulated triangle escalates to the
    /// oversampled path (its fold flips are un-BLEPped discontinuities), and
    /// the flip itself is slewed — the output crosses the fold step over
    /// several samples instead of jumping in one.
    #[test]
    fn ring_mod_fold_flip_is_oversampled_and_slewed() {
        // Escalation: triangle + RingMod + a connected ring source → 4x;
        // saw + RingMod stays 1x, and so does an unconnected ring (no source
        // → no flip can ever occur → don't pay the oversample CPU).
        let gate = AudioBuffer::new(64);
        let escalated = |mask: (bool, bool, bool), connect: bool| {
            let mut sid = sid_with(mask, 7493);
            sid.set_param(Param::SidOscillator(SidOscillatorParam::RingMod(true)));
            if connect {
                let inputs = [(PortName::intern("ring"), &gate)];
                let _ = render_with_inputs(&mut sid, 64, InputPorts::new(&inputs));
            } else {
                let _ = render(&mut sid, 64);
            }
            sid.last_factor
        };
        assert_eq!(escalated((true, false, false), true), 4, "ring tri → 4x");
        assert_eq!(
            escalated((false, true, false), true),
            1,
            "ring saw stays 1x"
        );
        assert_eq!(
            escalated((true, false, false), false),
            1,
            "unconnected ring must not pay 4x"
        );

        // Slew: a near-static triangle (tiny freq_reg) with one ring edge.
        // The fold flip inverts the output; the transition must be spread
        // over multiple samples, never one full-height jump.
        let n = 256;
        let edge = 128;
        let mut ring = AudioBuffer::new(n);
        for i in edge..n {
            ring[i] = 1.0;
        }
        let mut sid = sid_with((true, false, false), 3);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::RingMod(true)));
        // Advance away from the trough so the flip has real height.
        let _ = render(&mut sid, 20_000);
        let inputs = [(PortName::intern("ring"), &ring)];
        let (out, _) = render_with_inputs(&mut sid, n, InputPorts::new(&inputs));

        let step = (out[edge + 16] - out[edge - 16]).abs();
        assert!(step > 0.05, "the fold flip must actually move: {step}");
        // PolyBLEP + half-band decimation band-limit the flip: the largest
        // single-sample move stays well below the full step height (the old
        // 1x path jumped the whole step in one sample — the §11 brightness).
        let max_jump = out[edge - 8..edge + 16]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_jump < step * 0.85,
            "fold flip must be slewed over several samples: \
             max jump {max_jump} vs step {step}"
        );
    }

    /// The combined-waveform model differs per chip model: the 6581's bus
    /// leakage pulls bits below the 8580's plain AND.
    #[test]
    fn combined_waveforms_differ_per_model() {
        let n = 512;
        let render_model = |model: SidModel| {
            let mut sid = sid_with((true, true, false), 7493);
            sid.set_param(Param::SidOscillator(SidOscillatorParam::Model(model)));
            let (out, _) = render(&mut sid, n);
            out
        };
        let m6581 = render_model(SidModel::Mos6581);
        let m8580 = render_model(SidModel::Mos8580);
        assert_ne!(m6581, m8580, "6581 and 8580 combined tables must differ");
        // The 6581 leakage only ever clears bits — checked in the digital
        // domain (the analog DAC curve need not stay monotonic).
        let digital = |model: SidModel| {
            let mut sid = sid_with((true, true, false), 7493);
            sid.set_param(Param::SidOscillator(SidOscillatorParam::Model(model)));
            sid
        };
        let (d6581, d8580) = (digital(SidModel::Mos6581), digital(SidModel::Mos8580));
        let mask = MASK_TRIANGLE | MASK_SAWTOOTH;
        for acc24 in (0..0xFF_FFFF).step_by(0x1235) {
            let a = d6581.waveform_12bit(mask, acc24, 2048, false, false);
            let b = d8580.waveform_12bit(mask, acc24, 2048, false, false);
            let (a, b) = (a.unwrap_or(0), b.unwrap_or(0));
            assert_eq!(
                a & !b,
                0,
                "6581 leakage may only clear bits: {a:03x} vs {b:03x}"
            );
        }
        // Single waveforms produce identical digital values across models
        // (no combining involved) — only the DAC curve differs.
        let d_saw_6581 = digital(SidModel::Mos6581);
        let d_saw_8580 = digital(SidModel::Mos8580);
        for acc24 in (0..0xFF_FFFF).step_by(0x1235) {
            assert_eq!(
                d_saw_6581.waveform_12bit(MASK_SAWTOOTH, acc24, 2048, false, false),
                d_saw_8580.waveform_12bit(MASK_SAWTOOTH, acc24, 2048, false, false),
            );
        }
    }

    /// Digital-domain stats for a full accumulator cycle of a combined
    /// waveform: (AC RMS relative to the saw's in dB, fundamental magnitude
    /// relative to the saw's in dB). The waveform depends only on the top 12
    /// accumulator bits, so 4096 codes cover one exact cycle.
    fn combined_stats(model: SidModel, mask: u8, pw: u32) -> (f32, f32) {
        let mut sid = SidOscillator::new();
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Model(model)));
        let n = 4096u32;
        #[allow(clippy::cast_precision_loss)]
        let series = |mask: u8| -> Vec<f32> {
            (0..n)
                .map(|code| {
                    sid.waveform_12bit(mask, code << 12, pw, false, false)
                        .unwrap_or(0) as f32
                })
                .collect()
        };
        // AC RMS + fundamental (DFT bin 1 over the exact full cycle, where the
        // mean only lands in bin 0 — so no AC-coupling needed for Goertzel).
        let stats = |x: &[f32]| -> (f32, f32) {
            #[allow(clippy::cast_precision_loss)]
            let len = x.len() as f32;
            let mean = x.iter().sum::<f32>() / len;
            let rms = (x.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / len).sqrt();
            let f0 = crate::math::goertzel_magnitude(x, Hertz::new(1.0), SampleRate::new(len));
            (rms, f0)
        };
        let (saw_rms, saw_f0) = stats(&series(MASK_SAWTOOTH));
        let (rms, f0) = stats(&series(mask));
        let db = |x: f32, reference: f32| {
            if x > 0.0 {
                20.0 * (x / reference).log10()
            } else {
                -120.0
            }
        };
        (db(rms, saw_rms), db(f0, saw_f0))
    }

    /// 6581 saw+tri (0x31) — §11 gap 1: the real chip kills the fundamental
    /// (ref RMS ≈ −26 dB vs saw) and leaves energy at high accumulator-bit
    /// products. The option-C pulldown must reproduce that character, where
    /// the old option-B baseline kept a full-level spectrum with a strong f0.
    #[test]
    fn st_6581_kills_fundamental() {
        let (rms_db, f0_db) = combined_stats(SidModel::Mos6581, MASK_TRIANGLE | MASK_SAWTOOTH, 0);
        assert!(
            (-30.0..=-20.0).contains(&rms_db),
            "6581 ST RMS must sit ~26 dB below saw, got {rms_db:.1} dB"
        );
        assert!(
            f0_db < -35.0,
            "6581 ST must kill the fundamental, got {f0_db:.1} dB"
        );
        // The 8580 keeps a full-level AND — the models must stay distinct.
        let (rms_8580, _) = combined_stats(SidModel::Mos8580, MASK_TRIANGLE | MASK_SAWTOOTH, 0);
        assert!(
            rms_8580 > -15.0,
            "8580 ST stays near full level, got {rms_8580:.1} dB"
        );
    }

    /// 6581 pulse+saw (0x61) — §11 gap 2: the real chip pulls the bus to
    /// near-silence (reads unvoiced). The bus-collapse model leaves at most a
    /// blip at the very top of the ramp. Same for all-three (0x71).
    #[test]
    fn ps_6581_collapses_to_near_silence() {
        for mask in [
            MASK_PULSE | MASK_SAWTOOTH,
            MASK_PULSE | MASK_SAWTOOTH | MASK_TRIANGLE,
        ] {
            let (rms_db, _) = combined_stats(SidModel::Mos6581, mask, 2048);
            assert!(
                rms_db < -25.0,
                "6581 mask {mask:#x} must collapse to near-silence, got {rms_db:.1} dB"
            );
            // 8580: plain AND keeps a loud voiced waveform.
            let (rms_8580, _) = combined_stats(SidModel::Mos8580, mask, 2048);
            assert!(rms_8580 > rms_db + 10.0, "8580 must stay voiced");
        }
    }

    /// §11: 6581 combined tri+pulse carries real DC (~ −0.22 measured) that
    /// eats mix headroom — the default-on DC blocker (the C64's AC-coupled
    /// output) must remove it; bypassing the blocker must expose it.
    #[test]
    fn dc_blocker_removes_combined_waveform_dc() {
        let render_mean = |dc_block: bool| {
            let mut sid = sid_with((true, false, true), 7493);
            sid.set_param(Param::SidOscillator(SidOscillatorParam::DcBlock(dc_block)));
            // Settle past the blocker's initial transient (~16 Hz corner).
            let _ = render(&mut sid, 24_000);
            let (out, _) = render(&mut sid, 8192);
            #[allow(clippy::cast_precision_loss)]
            {
                out.iter().sum::<f32>() / out.len() as f32
            }
        };
        let raw = render_mean(false);
        assert!(
            raw < -0.05,
            "6581 tri+pulse must carry negative DC without the blocker: {raw}"
        );
        let blocked = render_mean(true);
        assert!(
            blocked.abs() < 0.01,
            "the DC blocker must remove it: {blocked}"
        );
    }

    /// 6581 pulse+tri (0x51) — measured at the A/B floor with the mild
    /// neighbour-support model: the option-C split must keep it byte-exact.
    #[test]
    fn pt_6581_keeps_neighbour_support_model() {
        let sid = {
            let mut s = SidOscillator::new();
            s.set_param(Param::SidOscillator(SidOscillatorParam::Model(
                SidModel::Mos6581,
            )));
            s
        };
        let mask = MASK_PULSE | MASK_TRIANGLE;
        for code in 0..4096u32 {
            let acc = code << 12;
            let got = sid.waveform_12bit(mask, acc, 1024, false, false);
            // Oracle composed from the already-tested single-waveform paths:
            // PT combining is neighbour_support of the plain AND.
            let single = |m: u8| sid.waveform_12bit(m, acc, 1024, false, false).unwrap_or(0);
            let expected =
                SidOscillator::neighbour_support(single(MASK_TRIANGLE) & single(MASK_PULSE));
            assert_eq!(got, Some(expected), "PT mismatch at code {code:#x}");
        }
    }

    /// Render-path pitch regression (mirrors `voice_pitch_harness`, plan §8):
    /// a plain SID saw at MIDI 45 renders f0 ≈ 110 Hz at both 44.1 kHz and
    /// 48 kHz — guards the sample-rate-correctness rule (plan §6) — and the
    /// per-block voice pitch tracks continuously (2× doubles the fundamental).
    #[test]
    fn render_path_pitch_is_sample_rate_correct() {
        use crate::voice_pitch_harness::{amdf_fundamental, render_mono};
        let a2 = MidiNote::new(45);
        let f = a2.to_frequency().as_f32(); // 110 Hz
        let cents = |est: f32, target: f32| 1200.0 * (est / target).log2();

        for sr in [SampleRate::CD_QUALITY, SampleRate::DVD_QUALITY] {
            let mut sid = SidOscillator::new();
            sid.note_on(a2, Velocity::MAX);
            let out = render_mono(&mut sid, sr, 4, 1024, |_| {});
            let est = amdf_fundamental(&out[2048..], sr.as_f32(), f);
            assert!(
                cents(est, f).abs() < 50.0,
                "sr {}: estimated {est} Hz vs {f} Hz",
                sr.as_f32()
            );
        }

        // Continuous voice pitch: 2× per block doubles the fundamental.
        let sr = SampleRate::DVD_QUALITY;
        let mut sid = SidOscillator::new();
        sid.note_on(a2, Velocity::MAX);
        let up = render_mono(&mut sid, sr, 4, 1024, |m| {
            m.set_voice_pitch(VoicePitch::tracking(Hertz::new(f * 2.0)));
        });
        let est_up = amdf_fundamental(&up[2048..], sr.as_f32(), f * 2.0);
        assert!(
            cents(est_up, f * 2.0).abs() < 50.0,
            "2x voice pitch: estimated {est_up} Hz vs {} Hz",
            f * 2.0
        );
    }

    /// Circuit-analysis sanity: an ideal terminated R-2R ladder (k = 2) must
    /// produce exact binary weights — validates the Thevenin walk itself.
    #[test]
    fn ideal_r2r_ladder_gives_binary_weights() {
        let w = r2r_dac_weights(2.0, true);
        for i in 0..11 {
            let ratio = w[i + 1] / w[i];
            assert!(
                (ratio - 2.0).abs() < 1e-9,
                "ideal ladder weight ratio must be exactly 2, got {ratio} at bit {i}"
            );
        }
        // MSB of an ideal ladder contributes exactly half the reference.
        assert!(
            (w[11] - 0.5).abs() < 1e-9,
            "MSB weight must be 1/2: {}",
            w[11]
        );
    }

    /// The 6581 waveform DAC is visibly non-linear (kinked ladder, no
    /// termination); the 8580 is a straight line. Both stay within ±1.
    #[test]
    fn dac_curves_differ_per_model() {
        let sid_6581 = {
            let mut s = SidOscillator::new();
            s.set_param(Param::SidOscillator(SidOscillatorParam::Model(
                SidModel::Mos6581,
            )));
            s
        };
        let sid_8580 = SidOscillator::new(); // constructor default model is 6581
        let mut sid_8580 = sid_8580;
        sid_8580.set_param(Param::SidOscillator(SidOscillatorParam::Model(
            SidModel::Mos8580,
        )));

        let mut max_dev = 0.0f32;
        for v in 0..4096u32 {
            let a = sid_6581.dac_to_bipolar(v);
            let b = sid_8580.dac_to_bipolar(v);
            assert!((-1.0..=1.0).contains(&a), "6581 DAC out of range: {a}");
            assert!((-1.0..=1.0).contains(&b), "8580 DAC out of range: {b}");
            max_dev = max_dev.max((a - b).abs());
        }
        // Endpoints agree (both normalized), the middle bends.
        assert!((sid_6581.dac_to_bipolar(0) - -1.0).abs() < 1e-6);
        assert!((sid_6581.dac_to_bipolar(4095) - 1.0).abs() < 1e-6);
        assert!(
            max_dev > 0.01,
            "6581 DAC must deviate from linear, max dev = {max_dev}"
        );
        // The signature 6581 kink: the biggest single-code step sits at the
        // MSB carry (0x7FF → 0x800), unlike the uniform linear ladder.
        let kink = (sid_6581.dac_to_bipolar(0x800) - sid_6581.dac_to_bipolar(0x7FF)).abs();
        let linear_step = 2.0 / 4095.0;
        assert!(
            kink > linear_step * 4.0,
            "expected a pronounced MSB kink, got {kink} vs linear step {linear_step}"
        );
    }

    /// The waveform sequence steps the mask once per driver frame and holds
    /// its last step; note_on restarts the program.
    #[test]
    fn waveform_sequence_steps_per_frame() {
        let mut sid = sid_with((true, false, false), 7493);
        // Exact-silence assertions: bypass the DC blocker's decay tail.
        sid.set_param(Param::SidOscillator(SidOscillatorParam::DcBlock(false)));
        // Program: frame 0 = sawtooth, frame 1+ = silence (mask 0).
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqLength(2)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStep(0, 0b0010)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStep(1, 0)));

        // PAL frame at the default 48 kHz test rate = 960 samples.
        let n = 2600;
        let (out, _) = render(&mut sid, n);
        let energy =
            |range: std::ops::Range<usize>| -> f32 { out[range].iter().map(|v| v.abs()).sum() };
        assert!(
            energy(0..958) > 1.0,
            "frame 0 must play the programmed sawtooth"
        );
        assert!(
            energy(980..2600) == 0.0,
            "frames 1+ must hold the silent last step"
        );

        // note_on restarts the program: the saw frame plays again.
        sid.note_on(MidiNote::A4, Velocity::MAX);
        let (again, _) = render(&mut sid, 880);
        assert!(
            again.iter().map(|v| v.abs()).sum::<f32>() > 1.0,
            "note_on must restart the sequence"
        );
    }

    /// With `SeqLoop` on the program repeats (`idx = pos % len`) for the whole
    /// note — the canonical SID waveform alternation — instead of holding the
    /// last step (§11 addendum ask 1).
    #[test]
    fn waveform_sequence_loops_when_seq_loop_set() {
        let mut sid = sid_with((false, false, false), 7493);
        // Near-silence assertions: bypass the DC blocker's decay tail.
        sid.set_param(Param::SidOscillator(SidOscillatorParam::DcBlock(false)));
        // Program: saw, silence — looped it must alternate every frame.
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqLength(2)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStep(0, 0b0010)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStep(1, 0)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqLoop(true)));

        // PAL frame at the default 48 kHz test rate = 960 samples.
        let n = 4800; // 5 frames: saw, off, saw, off, saw
        let (out, _) = render(&mut sid, n);
        let energy =
            |range: std::ops::Range<usize>| -> f32 { out[range].iter().map(|v| v.abs()).sum() };
        assert!(energy(0..940) > 1.0, "frame 0 plays the saw");
        assert!(energy(1000..1900) < 0.5, "frame 1 is the silent step");
        assert!(energy(1940..2860) > 1.0, "frame 2 loops back to the saw");
        assert!(energy(2920..3820) < 0.5, "frame 3 is silent again");
        assert!(energy(3860..4780) > 1.0, "frame 4 loops back to the saw");
    }

    #[test]
    fn sequence_frequency_switches_on_the_waveform_step_boundary() {
        let mut sid = sid_with((false, true, false), 7493);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqLength(2)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStep(0, 0b0010)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStep(1, 0b0001)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqFreqMask(0b11)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStepFreq(
            0, 1000,
        )));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStepFreq(
            1, 2000,
        )));

        let _ = render(&mut sid, 959);
        assert_eq!(sid.live_mask(), MASK_SAWTOOTH);
        let before = sid.acc;
        let _ = render(&mut sid, 1);
        let actual_increment = sid.acc.wrapping_sub(before) & ACC_MASK;
        let expected_increment = SidOscillator::acc_increment(2000.0, sid.reg_to_inc(48_000.0));

        assert_eq!(sid.live_mask(), MASK_TRIANGLE);
        assert_eq!(actual_increment, expected_increment);
    }

    #[test]
    fn sequence_frequency_clear_mask_bits_inherit_the_live_base() {
        let mut sid = sid_with((false, true, false), 1000);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqLength(2)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqFreqMask(0b01)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStepFreq(
            0, 2000,
        )));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStepFreq(
            1, 3000,
        )));

        assert_eq!(sid.live_freq_reg(1234.0), 2000.0);
        sid.frame_count = u32::from(sid.seq_rate);
        assert_eq!(sid.live_freq_reg(1234.0), 1234.0);
    }

    /// TEST dominates hard sync: a master edge during a TEST hold must not
    /// inject a BLEP step into the frozen output.
    #[test]
    fn test_hold_suppresses_sync_steps() {
        let n = 256;
        let mut master = AudioBuffer::new(n);
        for i in 0..n {
            master[i] = if (i / 32) % 2 == 1 { 1.0 } else { 0.0 };
        }
        let mut sid = sid_with((false, true, false), 7493);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::DcBlock(false)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::HardSync(true)));
        let _ = render(&mut sid, 300);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Test(true)));
        let inputs = [(PortName::SYNC, &master)];
        let (out, _) = render_with_inputs(&mut sid, n, InputPorts::new(&inputs));
        assert!(
            out.iter().all(|&v| (v - out[0]).abs() < 1e-6),
            "output must stay frozen under TEST despite sync edges"
        );
    }

    /// Enabling Hard Sync while the master gate is already high must not fire
    /// a spurious reset — the edge history is tracked even with SYNC off.
    #[test]
    fn enabling_hard_sync_mid_high_does_not_reset() {
        let n = 64;
        let mut high = AudioBuffer::new(n);
        for i in 0..n {
            high[i] = 1.0;
        }
        let inputs = [(PortName::SYNC, &high)];

        let mut sid = sid_with((false, true, false), 1000);
        // Block 1: sync input high, SYNC bit off — history catches up.
        let _ = render_with_inputs(&mut sid, n, InputPorts::new(&inputs));
        let acc_before = sid.acc;
        // Block 2: SYNC bit enabled; the input never has a 0→1 edge, so the
        // accumulator must keep free-running from where it was.
        sid.set_param(Param::SidOscillator(SidOscillatorParam::HardSync(true)));
        let _ = render_with_inputs(&mut sid, n, InputPorts::new(&inputs));
        assert!(
            sid.acc > acc_before,
            "no edge → no reset: acc must keep advancing ({} → {})",
            acc_before,
            sid.acc
        );
    }

    /// High quality decimates oversized blocks in chunks — the tail of a block
    /// larger than the Downsampler's one-call capacity must still carry audio.
    #[test]
    fn high_quality_fills_large_blocks_completely() {
        let mut sid = sid_with((false, true, false), 7493);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::Quality(
            SidQuality::High,
        )));
        let n = 8192;
        let (out, _) = render(&mut sid, n);
        let tail_energy: f32 = out[n - 2048..].iter().map(|v| v.abs()).sum();
        assert!(
            tail_energy > 1.0,
            "tail of an oversized High-quality block must contain the saw, energy = {tail_energy}"
        );
    }

    /// Params round-trip through set/get and every `get_params()` entry has a
    /// descriptor entry (save-path invariant).
    #[test]
    fn params_round_trip_and_match_descriptor() {
        let mut sid = SidOscillator::new();
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqLength(3)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::NoiseSeed(
            SidNoiseSeed::new(0x12_3456),
        )));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStep(0, 0x1)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStep(1, 0x4)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStep(2, 0x8)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqFreqMask(0x5)));
        sid.set_param(Param::SidOscillator(SidOscillatorParam::SeqStepFreq(
            0, 1234,
        )));

        let desc = sid.descriptor();
        for p in sid.get_params() {
            assert!(
                desc.parameters.iter().any(|d| p.same_kind(&d.id)),
                "get_params() entry without descriptor: {p:?}"
            );
            assert!(sid.get_param(&p).is_some());
        }
        assert_eq!(
            sid.get_param(&Param::SidOscillator(SidOscillatorParam::SeqStep(1, 0))),
            Some(4.0)
        );
        assert_eq!(
            sid.get_param(&Param::SidOscillator(SidOscillatorParam::SeqFreqMask(0))),
            Some(5.0)
        );
        assert_eq!(
            sid.get_param(&Param::SidOscillator(SidOscillatorParam::NoiseSeed(
                SidNoiseSeed::DEFAULT
            ))),
            Some(0x12_3456 as f32)
        );
        assert_eq!(
            sid.get_param(&Param::SidOscillator(SidOscillatorParam::SeqStepFreq(0, 0))),
            Some(1234.0)
        );
        // The raw registers are automatable as stepped/sample-hold lanes
        // (integer automation — plan §3's "general alternative", now the
        // engine rule) and mod-matrix modulatable; `level` is the continuous
        // target.
        let automatable: Vec<&str> = desc
            .parameters
            .iter()
            .filter(|p| p.is_automatable())
            .map(|p| p.type_id.as_str())
            .collect();
        assert_eq!(automatable, ["freq_reg", "pw_reg", "level"]);
        let modulatable: Vec<&str> = desc
            .parameters
            .iter()
            .filter(|p| p.modulatable)
            .map(|p| p.type_id.as_str())
            .collect();
        assert_eq!(modulatable, ["freq_reg", "pw_reg", "level"]);
    }

    /// The transient-override path (§11 addendum ask 2) is honored by the
    /// module: the override replaces the base without mutating it, and
    /// `clear_param_overrides` restores it. With integer automation in the
    /// engine rule, `pw_reg`/`freq_reg` lanes reach these overrides at render
    /// time (stepped/sample-hold).
    #[test]
    fn param_overrides_reach_registers_without_mutating_base() {
        // PW: base 3072 (25% duty) overridden to 1024 (75% duty).
        let duty = measured_duty;
        let mut sid = sid_with((false, false, true), 7493);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::PulseWidthReg(
            3072,
        )));
        assert!((duty(&mut sid) - 0.25).abs() < 0.03, "base pw 3072");
        sid.set_param_override(Param::SidOscillator(SidOscillatorParam::PulseWidthReg(
            1024,
        )));
        assert!((duty(&mut sid) - 0.75).abs() < 0.03, "override pw 1024");
        // The base param is never mutated by automation (save-path rule).
        assert_eq!(
            sid.get_param(&Param::SidOscillator(SidOscillatorParam::PulseWidthReg(0))),
            Some(3072.0)
        );
        sid.clear_param_overrides();
        assert!(
            (duty(&mut sid) - 0.25).abs() < 0.03,
            "cleared -> base again"
        );

        // FreqReg: override honored while Track Pitch is off (ring/sync-source
        // pitch lanes), outranked by the played note while it is on.
        let mut sid = sid_with((false, true, false), 1000);
        sid.set_param_override(Param::SidOscillator(SidOscillatorParam::FreqReg(7493)));
        assert_eq!(sid.effective_freq_reg(), 7493);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::TrackVoicePitch(
            true,
        )));
        sid.set_voice_pitch(VoicePitch::tracking(Hertz::new(880.0)));
        // PAL: 880 * 16777216 / 985248 = 14985.01… → 14985
        assert_eq!(sid.effective_freq_reg(), 14985, "played note outranks");
    }

    /// Mod-matrix offsets land on the registers through the generic store
    /// (normalized through the descriptor range, added on top of the base).
    #[test]
    fn mod_offsets_reach_registers() {
        let mut sid = sid_with((false, false, true), 7493);
        sid.set_param(Param::SidOscillator(SidOscillatorParam::PulseWidthReg(
            1024,
        )));
        let desc = sid.descriptor();
        if let Some(offsets) = sid.mod_offsets_mut() {
            offsets.populate(&desc);
        }
        // +0.25 normalized over the 0..4095 range ≈ +1024 register units:
        // duty drops from 75% (pw 1024) to ~50% (pw 2048).
        sid.set_mod_offset("pw_reg", 0.25);
        let high = measured_duty(&mut sid);
        assert!((high - 0.5).abs() < 0.03, "pw mod offset must land: {high}");
    }
}
