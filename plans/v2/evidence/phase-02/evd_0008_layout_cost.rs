//! EVD-0008 harness: planar versus interleaved cost at one render quantum.
//!
//! Standalone on purpose — no workspace dependency, no criterion, no crate
//! boundary to widen. Build and run exactly as the evidence record states:
//!
//! ```text
//! rustc -O -C target-cpu=native -o /tmp/evd_0008 evd_0008_layout_cost.rs
//! taskset -c 10,11 /tmp/evd_0008 <rounds> <iterations>
//! ```
//!
//! Arms, rotated in order once per round, minimum over rounds:
//!
//! - `planar` / `planar_ctl` — two mono `Q`-frame buffers, one contiguous biquad
//!   pass each. The second is the null control for the *kernel* comparison: the
//!   spread between the two is that comparison's noise floor.
//! - `strided` — one interleaved `2Q` buffer, one stride-2 biquad pass per channel.
//! - `gain_planar` / `gain_strided` — a non-recursive pass in each layout, which is
//!   where a layout can matter at all: a recursive filter is latency-bound and
//!   indifferent to it.
//! - `transpose` — the boundary a **planar** arena pays: two planar buffers written
//!   into one interleaved device buffer.
//! - `boundary_memcpy` — the boundary an **interleaved** arena pays instead.
//! - `chain5` — five sequential biquad passes over one mono buffer.
//! - `chain_min` — the minimal five-node mono chain, one call per node.
//! - `stereo_planar` / `stereo_planar_ctl` / `stereo_inter` — the criterion-D arms:
//!   the same stereo five-node chain in each layout, **one call per node per
//!   channel**, plus that layout's own boundary operation. `stereo_planar_ctl` is
//!   criterion D's own null control, because the kernel comparison's control has a
//!   different instruction mix and cannot bound this one.
//!
//! Every node is a separate `#[inline(never)]` call, because that is the execution
//! shape the decision is about: a planar stereo chain schedules and dispatches twice
//! as many operations as an interleaved one, and fusing them into a single function
//! would let the optimizer erase exactly the cost being measured.
//!
//! Nothing here models V1 or V2 code. It models the two memory layouts under the
//! same arithmetic.

use std::hint::black_box;
use std::time::Instant;

/// Frames in one render quantum. ADR-0037's provisional value.
const Q: usize = 64;
/// Channels in the stereo arms.
const CHANNELS: usize = 2;

/// One direct-form-1 biquad state.
#[derive(Clone, Copy, Default)]
struct Biquad {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

/// Coefficients, prepared once — the split ADR-0005 and the master plan both ask for.
#[derive(Clone, Copy)]
struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Coeffs {
    /// A low-pass at roughly 1 kHz / 44.1 kHz, Q = 0.707. Values, not a design.
    const LOWPASS: Self = Self {
        b0: 0.004_824_343,
        b1: 0.009_648_686,
        b2: 0.004_824_343,
        a1: -1.809_793_5,
        a2: 0.829_090_9,
    };
}

/// Contiguous: the block is one channel, back to back.
#[inline(never)]
fn process_contiguous(block: &mut [f32], state: &mut Biquad, c: Coeffs) {
    for sample in block.iter_mut() {
        let x = *sample;
        let y = c.b0 * x + c.b1 * state.x1 + c.b2 * state.x2 - c.a1 * state.y1 - c.a2 * state.y2;
        state.x2 = state.x1;
        state.x1 = x;
        state.y2 = state.y1;
        state.y1 = y;
        *sample = y;
    }
}

/// Strided: the block holds `stride` interleaved channels and this is one of them.
#[inline(never)]
fn process_strided(block: &mut [f32], offset: usize, stride: usize, state: &mut Biquad, c: Coeffs) {
    let mut index = offset;
    while index < block.len() {
        let x = block[index];
        let y = c.b0 * x + c.b1 * state.x1 + c.b2 * state.x2 - c.a1 * state.y1 - c.a2 * state.y2;
        state.x2 = state.x1;
        state.x1 = x;
        state.y2 = state.y1;
        state.y1 = y;
        block[index] = y;
        index += stride;
    }
}

/// A gain pass: no recursion, so the contiguous form is free to vectorize.
#[inline(never)]
fn gain_contiguous(block: &mut [f32], level: f32) {
    for sample in block.iter_mut() {
        *sample *= level;
    }
}

/// The same gain over one channel of an interleaved block.
#[inline(never)]
fn gain_strided(block: &mut [f32], offset: usize, stride: usize, level: f32) {
    let mut index = offset;
    while index < block.len() {
        block[index] *= level;
        index += stride;
    }
}

/// State for the voice-chain arms.
#[derive(Clone, Copy)]
struct ChainState {
    gate: bool,
    level: f32,
    env_coeff: f32,
    phase: f32,
    increment: f32,
    lp: f32,
    lp_coeff: f32,
    gain: f32,
}

impl Default for ChainState {
    fn default() -> Self {
        Self {
            gate: true,
            level: 0.0,
            env_coeff: 0.002,
            phase: 0.0,
            increment: 0.01,
            lp: 0.0,
            lp_coeff: 0.15,
            gain: 0.8,
        }
    }
}

// The five nodes of the minimal chain, one function each. Deliberately the
// cheapest defensible implementations: a one-pole envelope, a naive saw with no
// band limiting, a one-pole filter rather than a biquad, a gain, and a copy. Every
// real node this phase renders is more expensive than its counterpart here, so a
// boundary share measured against this chain overstates the real one.

/// Node 1: envelope. Recursive, mono, identical in both layouts.
#[inline(never)]
fn env_pass(out: &mut [f32], state: &mut ChainState) {
    let target = if state.gate { 1.0 } else { 0.0 };
    for sample in out.iter_mut() {
        state.level += (target - state.level) * state.env_coeff;
        *sample = state.level;
    }
}

/// Node 2: oscillator. Recursive, mono, identical in both layouts.
#[inline(never)]
fn osc_pass(out: &mut [f32], state: &mut ChainState) {
    for sample in out.iter_mut() {
        state.phase += state.increment;
        if state.phase >= 1.0 {
            state.phase -= 1.0;
        }
        *sample = state.phase.mul_add(2.0, -1.0);
    }
}

/// Node 3: filter, one channel of a planar arena.
#[inline(never)]
fn filter_pass_planar(block: &mut [f32], state: &mut ChainState) {
    for sample in block.iter_mut() {
        state.lp += (*sample - state.lp) * state.lp_coeff;
        *sample = state.lp;
    }
}

/// Node 3, interleaved: two channel states, one pass, a frame at a time.
#[inline(never)]
fn filter_pass_interleaved(inter: &mut [f32], left: &mut ChainState, right: &mut ChainState) {
    for frame in inter.chunks_exact_mut(CHANNELS) {
        left.lp += (frame[0] - left.lp) * left.lp_coeff;
        frame[0] = left.lp;
        right.lp += (frame[1] - right.lp) * right.lp_coeff;
        frame[1] = right.lp;
    }
}

/// Node 4: amplifier, one channel of a planar arena.
#[inline(never)]
fn amp_pass_planar(block: &mut [f32], env: &[f32], gain: f32) {
    for (sample, level) in block.iter_mut().zip(env.iter()) {
        *sample *= *level * gain;
    }
}

/// Node 4, interleaved: one frame at a time, reading the envelope once per frame.
///
/// An earlier revision indexed `env[index / CHANNELS]` per sample, which put an
/// integer division and a redundant load in the inner loop — a penalty the layout
/// does not require, and one that made the interleaved arm look worse than it is.
/// Review caught it, and the corrections table records what it cost.
#[inline(never)]
fn amp_pass_interleaved(inter: &mut [f32], env: &[f32], gain: f32) {
    for (frame, level) in inter.chunks_exact_mut(CHANNELS).zip(env.iter()) {
        let scale = *level * gain;
        frame[0] *= scale;
        frame[1] *= scale;
    }
}

/// Node 5: mono to one planar channel.
#[inline(never)]
fn spread_planar(osc: &[f32], out: &mut [f32]) {
    out.copy_from_slice(osc);
}

/// Node 5, interleaved — one pass writing both channels.
#[inline(never)]
fn spread_interleaved(osc: &[f32], inter: &mut [f32]) {
    for (frame, value) in osc.iter().enumerate() {
        inter[frame * CHANNELS] = *value;
        inter[frame * CHANNELS + 1] = *value;
    }
}

/// The boundary a **planar** arena pays.
#[inline(never)]
fn interleave(left: &[f32], right: &[f32], out: &mut [f32]) {
    for frame in 0..Q {
        out[frame * CHANNELS] = left[frame];
        out[frame * CHANNELS + 1] = right[frame];
    }
}

/// The boundary an **interleaved** arena pays.
#[inline(never)]
fn boundary_copy(source: &[f32], out: &mut [f32]) {
    out.copy_from_slice(source);
}

fn timed<F: FnMut()>(iterations: u32, mut body: F) -> f64 {
    let start = Instant::now();
    for _ in 0..iterations {
        body();
    }
    start.elapsed().as_secs_f64() / f64::from(iterations)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let rounds: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(40);
    let iterations: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(20_000);

    let seed: Vec<f32> = (0..Q * CHANNELS)
        .map(|i| ((i as f32) * 0.017).sin())
        .collect();

    let mut left = [0.0_f32; Q];
    let mut right = [0.0_f32; Q];
    let mut env = [0.0_f32; Q];
    let mut mono = [0.0_f32; Q];
    let mut inter = [0.0_f32; Q * CHANNELS];
    let mut device = [0.0_f32; Q * CHANNELS];
    let c = Coeffs::LOWPASS;

    let names = [
        "planar",
        "planar_ctl",
        "strided",
        "transpose",
        "boundary_memcpy",
        "chain5",
        "gain_planar",
        "gain_planar_ctl",
        "gain_strided",
        "chain_min",
        "stereo_planar",
        "stereo_planar_ctl",
        "stereo_inter",
    ];
    let mut best = [f64::INFINITY; 13];

    // Each group keeps its control arm first, so every control is timed **before**
    // the arm it bounds — the evidence rules require the control to run first, and a
    // flat rotation would sometimes time a measurement before its own control. The
    // groups themselves rotate, so no group is always first.
    let groups: [&[usize]; 5] = [
        &[1, 0, 2],     // planar_ctl, planar, strided
        &[7, 6, 8],     // gain_planar_ctl, gain_planar, gain_strided
        &[11, 10, 12],  // stereo_planar_ctl, stereo_planar, stereo_inter
        &[3, 4],        // transpose, boundary_memcpy
        &[5, 9],        // chain5, chain_min
    ];

    for round in 0..rounds {
        for group_slot in 0..groups.len() {
            let group = groups[(group_slot + round as usize) % groups.len()];
            for &arm in group {
            left.copy_from_slice(&seed[..Q]);
            right.copy_from_slice(&seed[Q..]);
            env.copy_from_slice(&seed[..Q]);
            mono.copy_from_slice(&seed[..Q]);
            inter.copy_from_slice(&seed);
            device.copy_from_slice(&seed);
            let mut states = [Biquad::default(); 5];
            let mut chain = ChainState::default();
            let mut chain_right = ChainState::default();

            let elapsed = match arm {
                0 | 1 => timed(iterations, || {
                    process_contiguous(black_box(&mut left), &mut states[0], c);
                    process_contiguous(black_box(&mut right), &mut states[1], c);
                }),
                2 => timed(iterations, || {
                    process_strided(black_box(&mut inter), 0, CHANNELS, &mut states[0], c);
                    process_strided(black_box(&mut inter), 1, CHANNELS, &mut states[1], c);
                }),
                3 => timed(iterations, || {
                    interleave(black_box(&left), black_box(&right), black_box(&mut device));
                }),
                4 => timed(iterations, || {
                    boundary_copy(black_box(&inter), black_box(&mut device));
                }),
                5 => timed(iterations, || {
                    for state in states.iter_mut() {
                        process_contiguous(black_box(&mut mono), state, c);
                    }
                }),
                6 | 7 => timed(iterations, || {
                    gain_contiguous(black_box(&mut left), 0.5);
                    gain_contiguous(black_box(&mut right), 0.5);
                }),
                8 => timed(iterations, || {
                    gain_strided(black_box(&mut inter), 0, CHANNELS, 0.5);
                    gain_strided(black_box(&mut inter), 1, CHANNELS, 0.5);
                }),
                // The minimal chain, mono: five calls, criterion B's denominator.
                9 => timed(iterations, || {
                    env_pass(black_box(&mut env), &mut chain);
                    osc_pass(black_box(&mut mono), &mut chain);
                    filter_pass_planar(black_box(&mut mono), &mut chain);
                    amp_pass_planar(black_box(&mut mono), black_box(&env), chain.gain);
                    spread_planar(black_box(&mono), black_box(&mut left));
                }),
                // Criterion D, planar: one call per node per channel, then the
                // transpose this layout pays at the boundary.
                10 | 11 => timed(iterations, || {
                    env_pass(black_box(&mut env), &mut chain);
                    osc_pass(black_box(&mut mono), &mut chain);
                    spread_planar(black_box(&mono), black_box(&mut left));
                    spread_planar(black_box(&mono), black_box(&mut right));
                    filter_pass_planar(black_box(&mut left), &mut chain);
                    filter_pass_planar(black_box(&mut right), &mut chain_right);
                    amp_pass_planar(black_box(&mut left), black_box(&env), chain.gain);
                    amp_pass_planar(black_box(&mut right), black_box(&env), chain.gain);
                    interleave(black_box(&left), black_box(&right), black_box(&mut device));
                }),
                // Criterion D, interleaved: fewer, wider calls, then a memcpy.
                _ => timed(iterations, || {
                    env_pass(black_box(&mut env), &mut chain);
                    osc_pass(black_box(&mut mono), &mut chain);
                    spread_interleaved(black_box(&mono), black_box(&mut inter));
                    filter_pass_interleaved(black_box(&mut inter), &mut chain, &mut chain_right);
                    amp_pass_interleaved(black_box(&mut inter), black_box(&env), chain.gain);
                    boundary_copy(black_box(&inter), black_box(&mut device));
                }),
            };
            black_box(&left);
            black_box(&inter);
            black_box(&device);
            black_box(&mono);

            if elapsed < best[arm] {
                best[arm] = elapsed;
            }
            }
        }
    }

    println!("arm,seconds_per_iteration,nanoseconds_per_iteration");
    for (name, seconds) in names.iter().zip(best.iter()) {
        println!("{name},{seconds:.12},{:.3}", seconds * 1e9);
    }

    let kernel_control = (best[1] - best[0]).abs() / best[0].min(best[1]) * 100.0;
    let strided_delta = (best[2] / best[0] - 1.0) * 100.0;
    let gain_control = (best[7] - best[6]).abs() / best[6].min(best[7]) * 100.0;
    let gain_delta = (best[8] / best[6] - 1.0) * 100.0;
    let transpose_share = best[3] / best[9] * 100.0;
    // Criterion D: end-to-end totals, each including its own boundary operation,
    // against a control that repeats the planar arrangement bit for bit.
    let d_control = (best[11] - best[10]).abs() / best[10].min(best[11]) * 100.0;
    let d_delta = (best[12] / best[10] - 1.0) * 100.0;

    println!();
    println!("kernel_control_spread_percent,{kernel_control:.2}");
    println!("strided_vs_planar_percent_biquad,{strided_delta:.2}");
    println!("gain_control_spread_percent,{gain_control:.2}");
    println!("strided_vs_planar_percent_gain,{gain_delta:.2}");
    println!("transpose_share_of_chain_min_percent,{transpose_share:.2}");
    println!("criterion_d_control_spread_percent,{d_control:.2}");
    println!("criterion_d_interleaved_vs_planar_percent,{d_delta:.2}");
}
