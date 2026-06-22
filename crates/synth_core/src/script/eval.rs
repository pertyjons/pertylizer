//! The real-time YAMS evaluator: a stack-machine `for` loop over the bytecode.
//!
//! RT contract: no heap allocation, no locks, no panics, no logging. The value
//! stack and local slots are fixed-size arrays on the call frame; per-voice
//! persistent state lives in [`RegisterFile`]. NaN/Inf is sanitized at every
//! state-cell write and at the final output (decision: NaN poisoning is fatal
//! to state, so it must not survive into a persistent cell).

use crate::hash::{splitmix64, splitmix64_unit};
use crate::script::bytecode::{
    Builtin, CompiledScript, MAX_LOCALS, MAX_STACK, MAX_STATE, Op, finite_or_zero, safe_div,
};

/// Per-evaluation context supplied by the engine.
#[derive(Debug, Clone, Copy)]
pub struct EvalContext {
    /// Control rate in Hz (evaluations per second = sample_rate / block_size).
    /// Drives time-based stateful ops (`lag`, `slew`, `phasor`).
    pub control_rate: f32,
}

impl EvalContext {
    #[must_use]
    pub fn new(control_rate: f32) -> Self {
        Self { control_rate }
    }

    /// Seconds per control-rate block.
    fn dt(self) -> f32 {
        if self.control_rate > 0.0 {
            1.0 / self.control_rate
        } else {
            0.0
        }
    }
}

/// Per-voice mutable state for a running script: persistent state cells plus the
/// PRNG stream. Cloned/owned per voice; never shared (the `CompiledScript` is
/// the shared, immutable half).
#[derive(Debug, Clone)]
pub struct RegisterFile {
    state: [f32; MAX_STATE],
    prng_seed: u64,
    prng_counter: u64,
}

impl RegisterFile {
    /// Create a register file for `voice_index`, seeded from `global_seed`.
    #[must_use]
    pub fn new(voice_index: u32, global_seed: u64) -> Self {
        let mut rf = Self {
            state: [0.0; MAX_STATE],
            prng_seed: 0,
            prng_counter: 0,
        };
        rf.reset(voice_index, global_seed);
        rf
    }

    /// Reset on note-on / voice-steal: zero the state cells, and **re-seed**
    /// (not zero) the PRNG so simultaneous voices stay decorrelated while
    /// retriggers remain deterministic (decision #4).
    pub fn reset(&mut self, voice_index: u32, global_seed: u64) {
        self.state = [0.0; MAX_STATE];
        self.prng_seed = splitmix64(global_seed ^ u64::from(voice_index));
        self.prng_counter = 0;
    }

    fn state_get(&self, i: u16) -> f32 {
        self.state.get(i as usize).copied().unwrap_or(0.0)
    }

    /// Store into a state cell, sanitizing NaN/Inf to 0 so a poisoned value can
    /// never persist (decision: two-layer NaN sanitize, this is layer 2).
    fn state_set(&mut self, i: u16, v: f32) {
        if let Some(cell) = self.state.get_mut(i as usize) {
            *cell = finite_or_zero(v);
        }
    }

    /// Next uniform random in `[0, 1)` from the per-voice stream.
    fn next_unit(&mut self) -> f32 {
        let v = splitmix64_unit(self.prng_seed.wrapping_add(self.prng_counter));
        self.prng_counter = self.prng_counter.wrapping_add(1);
        v
    }
}

impl CompiledScript {
    /// Evaluate the script for one control block. `sources` holds the voice's
    /// resolved source values (indexed by source register); `regs` is this
    /// voice's persistent state. Returns the sanitized output offset.
    #[must_use]
    pub fn eval(&self, sources: &[f32], regs: &mut RegisterFile, ctx: &EvalContext) -> f32 {
        let dt = ctx.dt();
        let mut stack = Stack::new();
        let mut locals = [0.0f32; MAX_LOCALS];

        for op in &self.code {
            match *op {
                Op::PushConst(i) => {
                    stack.push(self.constants.get(i as usize).copied().unwrap_or(0.0))
                }
                Op::IndexConst { base, len } => {
                    let raw = stack.pop().floor();
                    // Clamp into 0..len-1 without f32::clamp (which panics on a
                    // NaN bound) — NaN/negative → 0, too-large → len-1.
                    let max = len.saturating_sub(1);
                    let idx = if raw >= f32::from(max) {
                        max
                    } else if raw >= 0.0 {
                        raw as u16
                    } else {
                        0
                    };
                    let at = base.saturating_add(idx) as usize;
                    stack.push(self.constants.get(at).copied().unwrap_or(0.0));
                }
                Op::PushSource(i) => stack.push(sources.get(i as usize).copied().unwrap_or(0.0)),
                Op::LoadLocal(i) => stack.push(locals.get(i as usize).copied().unwrap_or(0.0)),
                Op::StoreLocal(i) => {
                    let v = stack.pop();
                    if let Some(slot) = locals.get_mut(i as usize) {
                        *slot = v;
                    }
                }

                Op::Add => binop(&mut stack, |a, b| a + b),
                Op::Sub => binop(&mut stack, |a, b| a - b),
                Op::Mul => binop(&mut stack, |a, b| a * b),
                Op::Div => binop(&mut stack, safe_div),
                Op::Rem => binop(&mut stack, |a, b| if b == 0.0 { 0.0 } else { a % b }),
                Op::Pow => binop(&mut stack, f32::powf),

                Op::Neg => {
                    let a = stack.pop();
                    stack.push(-a);
                }
                Op::Not => {
                    let a = stack.pop();
                    stack.push(b2f(a == 0.0));
                }

                Op::Eq => binop(&mut stack, |a, b| b2f(a == b)),
                Op::Ne => binop(&mut stack, |a, b| b2f(a != b)),
                Op::Lt => binop(&mut stack, |a, b| b2f(a < b)),
                Op::Gt => binop(&mut stack, |a, b| b2f(a > b)),
                Op::Le => binop(&mut stack, |a, b| b2f(a <= b)),
                Op::Ge => binop(&mut stack, |a, b| b2f(a >= b)),
                Op::And => binop(&mut stack, |a, b| b2f(a != 0.0 && b != 0.0)),
                Op::Or => binop(&mut stack, |a, b| b2f(a != 0.0 || b != 0.0)),

                Op::Select => {
                    let els = stack.pop();
                    let then = stack.pop();
                    let cond = stack.pop();
                    stack.push(if cond != 0.0 { then } else { els });
                }

                Op::Call(b) => apply_builtin(b, &mut stack),

                Op::Lag(i) => {
                    let alpha = stack.pop();
                    let x = stack.pop();
                    let y = regs.state_get(i);
                    regs.state_set(i, y + alpha * (x - y));
                    stack.push(regs.state_get(i));
                }
                Op::Slew(i) => {
                    let down = stack.pop();
                    let up = stack.pop();
                    let x = stack.pop();
                    let y = regs.state_get(i);
                    let ny = if x > y {
                        (y + up * dt).min(x)
                    } else if x < y {
                        (y - down * dt).max(x)
                    } else {
                        y
                    };
                    regs.state_set(i, ny);
                    stack.push(regs.state_get(i));
                }
                Op::Sah(i) => {
                    let trig = stack.pop();
                    let x = stack.pop();
                    let prev = i.saturating_add(1);
                    let prev_trig = regs.state_get(prev);
                    if trig > 0.0 && prev_trig <= 0.0 {
                        regs.state_set(i, x);
                    }
                    regs.state_set(prev, trig);
                    stack.push(regs.state_get(i));
                }
                Op::Accum(i) => {
                    let x = stack.pop();
                    regs.state_set(i, regs.state_get(i) + x);
                    stack.push(regs.state_get(i));
                }
                Op::AccumReset(i) => {
                    let reset = stack.pop();
                    let x = stack.pop();
                    let prev = i.saturating_add(1);
                    let prev_reset = regs.state_get(prev);
                    let sum = if reset > 0.0 && prev_reset <= 0.0 {
                        0.0 // rising edge → clear the integrator (skip x this block)
                    } else {
                        regs.state_get(i) + x
                    };
                    regs.state_set(i, sum);
                    regs.state_set(prev, reset);
                    stack.push(regs.state_get(i));
                }
                Op::Delta(i) => {
                    let x = stack.pop();
                    let prev = regs.state_get(i);
                    regs.state_set(i, x);
                    stack.push(x - prev);
                }
                Op::Phasor(i) => {
                    let rate = stack.pop();
                    let ph = regs.state_get(i) + rate * dt;
                    regs.state_set(i, ph - ph.floor());
                    stack.push(regs.state_get(i));
                }
                Op::PhasorSync(i) => {
                    let sync = stack.pop();
                    let rate = stack.pop();
                    let prev = i.saturating_add(1);
                    let prev_sync = regs.state_get(prev);
                    let ph = if sync > 0.0 && prev_sync <= 0.0 {
                        0.0 // rising edge → reset phase
                    } else {
                        let p = regs.state_get(i) + rate * dt;
                        p - p.floor()
                    };
                    regs.state_set(i, ph);
                    regs.state_set(prev, sync);
                    stack.push(regs.state_get(i));
                }
                Op::Edge(i) => {
                    let x = stack.pop();
                    let prev = regs.state_get(i);
                    regs.state_set(i, x);
                    stack.push(b2f(x > 0.0 && prev <= 0.0));
                }
                Op::Counter(i) => {
                    let trig = stack.pop();
                    let prev = i.saturating_add(1);
                    let prev_trig = regs.state_get(prev);
                    if trig > 0.0 && prev_trig <= 0.0 {
                        regs.state_set(i, regs.state_get(i) + 1.0);
                    }
                    regs.state_set(prev, trig);
                    stack.push(regs.state_get(i));
                }
                Op::Rand => {
                    let hi = stack.pop();
                    let lo = stack.pop();
                    let u = regs.next_unit();
                    stack.push(lo + (hi - lo) * u);
                }
                Op::RandSmooth(i) => {
                    // Clamp to a non-negative Hz rate: a negative rate would
                    // drive the phase below 0 (the wrap only catches `>= 1.0`),
                    // letting the smoothstep output escape [0, 1) unbounded. A
                    // non-positive rate simply freezes the current segment.
                    let rate = stack.pop().max(0.0);
                    let c1 = i.saturating_add(1);
                    let c2 = i.saturating_add(2);
                    let mut ph = regs.state_get(i);
                    let mut start = regs.state_get(c1);
                    let mut end = regs.state_get(c2);
                    // Cold start: reset() zeroes all cells, so seed the first
                    // segment instead of gliding 0 → 0 for the whole first cycle.
                    if ph == 0.0 && start == 0.0 && end == 0.0 {
                        start = regs.next_unit();
                        end = regs.next_unit();
                    }
                    ph += rate * dt;
                    if ph >= 1.0 {
                        ph -= ph.floor();
                        start = end;
                        end = regs.next_unit(); // new per-voice target
                    }
                    regs.state_set(i, ph);
                    regs.state_set(c1, start);
                    regs.state_set(c2, end);
                    let t = ph * ph * (3.0 - 2.0 * ph); // smoothstep
                    stack.push(start + t * (end - start));
                }
            }
        }

        finite_or_zero(stack.top())
    }
}

// ---- value stack ----------------------------------------------------------

struct Stack {
    buf: [f32; MAX_STACK],
    sp: usize,
}

impl Stack {
    fn new() -> Self {
        Self {
            buf: [0.0; MAX_STACK],
            sp: 0,
        }
    }

    fn push(&mut self, v: f32) {
        if self.sp < MAX_STACK {
            self.buf[self.sp] = v;
            self.sp += 1;
        }
    }

    fn pop(&mut self) -> f32 {
        if self.sp > 0 {
            self.sp -= 1;
            self.buf[self.sp]
        } else {
            0.0
        }
    }

    fn top(&self) -> f32 {
        if self.sp > 0 {
            self.buf[self.sp - 1]
        } else {
            0.0
        }
    }
}

/// Pop b, pop a, push `f(a, b)`.
fn binop(stack: &mut Stack, f: impl Fn(f32, f32) -> f32) {
    let b = stack.pop();
    let a = stack.pop();
    stack.push(f(a, b));
}

/// Pop the builtin's operands (last on top) and push `Builtin::eval`. The
/// scalar math lives in `bytecode.rs` so it is shared with the const folder.
fn apply_builtin(b: Builtin, s: &mut Stack) {
    let n = b.arity();
    let mut args = [0.0f32; 3];
    for slot in args.iter_mut().take(n).rev() {
        *slot = s.pop();
    }
    s.push(b.eval(&args[..n]));
}

fn b2f(cond: bool) -> f32 {
    if cond { 1.0 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::bytecode::Op;

    const SR: f32 = 750.0; // 48 kHz / 64-sample blocks
    const SEED: u64 = 0x5EED;

    fn run(code: &[Op], constants: &[f32], sources: &[f32]) -> f32 {
        let script =
            CompiledScript::new(code.to_vec(), constants.to_vec(), sources.len() as u16, 4);
        let mut regs = RegisterFile::new(0, SEED);
        script.eval(sources, &mut regs, &EvalContext::new(SR))
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn constant_and_arithmetic() {
        // 2 * 3 + 1 = 7
        let code = [
            Op::PushConst(0),
            Op::PushConst(1),
            Op::Mul,
            Op::PushConst(2),
            Op::Add,
        ];
        assert!(approx(run(&code, &[2.0, 3.0, 1.0], &[]), 7.0));
    }

    #[test]
    fn reads_a_source() {
        assert!(approx(run(&[Op::PushSource(0)], &[], &[0.5]), 0.5));
    }

    #[test]
    fn select_is_eager_mux() {
        // cond ? then : els — cond true → then, cond false → els.
        let code = [
            Op::PushConst(0),
            Op::PushConst(1),
            Op::PushConst(2),
            Op::Select,
        ];
        assert!(approx(run(&code, &[1.0, 10.0, 20.0], &[]), 10.0));
        assert!(approx(run(&code, &[0.0, 10.0, 20.0], &[]), 20.0));
    }

    #[test]
    fn division_by_zero_is_safe() {
        let code = [Op::PushConst(0), Op::PushConst(1), Op::Div];
        assert!(approx(run(&code, &[1.0, 0.0], &[]), 0.0));
    }

    #[test]
    fn nan_does_not_poison_state() {
        // Feed a NaN constant into accum; the state cell and result must stay
        // finite (layer-2 sanitize).
        let script =
            CompiledScript::new(vec![Op::PushConst(0), Op::Accum(0)], vec![f32::NAN], 0, 1);
        let mut regs = RegisterFile::new(0, SEED);
        let out = script.eval(&[], &mut regs, &EvalContext::new(SR));
        assert_eq!(out, 0.0);
        assert_eq!(regs.state_get(0), 0.0);
    }

    #[test]
    fn lag_smooths_toward_target() {
        // alpha 0.5, x 1.0, state starts 0 → 0.5, then 0.75.
        let code = [Op::PushConst(0), Op::PushConst(1), Op::Lag(0)];
        let script = CompiledScript::new(code.to_vec(), vec![1.0, 0.5], 0, 1);
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(SR);
        assert!(approx(script.eval(&[], &mut regs, &ctx), 0.5));
        assert!(approx(script.eval(&[], &mut regs, &ctx), 0.75));
    }

    #[test]
    fn phasor_advances_by_rate_times_dt() {
        // rate = SR * 0.25 → +0.25 per block.
        let code = [Op::PushConst(0), Op::Phasor(0)];
        let script = CompiledScript::new(code.to_vec(), vec![SR * 0.25], 0, 1);
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(SR);
        assert!(approx(script.eval(&[], &mut regs, &ctx), 0.25));
        assert!(approx(script.eval(&[], &mut regs, &ctx), 0.5));
    }

    #[test]
    fn sah_holds_until_rising_edge() {
        // Hold source[0]; trig = source[1]. Sample only on a rising edge.
        let code = [Op::PushSource(0), Op::PushSource(1), Op::Sah(0)];
        let script = CompiledScript::new(code.to_vec(), vec![], 2, 2);
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(SR);
        // trig low → held stays at its initial 0.
        assert!(approx(script.eval(&[5.0, 0.0], &mut regs, &ctx), 0.0));
        // rising edge → sample 5.0.
        assert!(approx(script.eval(&[5.0, 1.0], &mut regs, &ctx), 5.0));
        // trig still high, source changed → held value persists (no new edge).
        assert!(approx(script.eval(&[9.0, 1.0], &mut regs, &ctx), 5.0));
    }

    #[test]
    fn index_const_reads_table_and_clamps() {
        // Table [10, 20, 30] baked at base 0; index from source[0].
        let consts = vec![10.0, 20.0, 30.0];
        let code = [Op::PushSource(0), Op::IndexConst { base: 0, len: 3 }];
        let run_idx = |i: f32| {
            let script = CompiledScript::new(code.to_vec(), consts.clone(), 1, 0);
            let mut regs = RegisterFile::new(0, SEED);
            script.eval(&[i], &mut regs, &EvalContext::new(SR))
        };
        assert!(approx(run_idx(0.0), 10.0));
        assert!(approx(run_idx(1.9), 20.0)); // floor → 1
        assert!(approx(run_idx(2.0), 30.0));
        assert!(approx(run_idx(5.0), 30.0)); // clamp high
        assert!(approx(run_idx(-1.0), 10.0)); // clamp low
        assert!(approx(run_idx(f32::NAN), 10.0)); // NaN → 0
    }

    #[test]
    fn index_const_with_base_offset() {
        // A second table living at base 2 inside the shared pool.
        let consts = vec![0.0, 0.0, 7.0, 8.0];
        let code = [
            Op::PushConst(0), /* push 0.0 as index */
            Op::IndexConst { base: 2, len: 2 },
        ];
        let script = CompiledScript::new(code.to_vec(), consts, 0, 0);
        let mut regs = RegisterFile::new(0, SEED);
        assert!(approx(
            script.eval(&[], &mut regs, &EvalContext::new(SR)),
            7.0
        ));
    }

    #[test]
    fn phasor_sync_resets_phase_on_rising_edge() {
        // rate = SR * 0.25 (+0.25/block); sync from source[0]. Cells 0,1.
        let code = [Op::PushConst(0), Op::PushSource(0), Op::PhasorSync(0)];
        let script = CompiledScript::new(code.to_vec(), vec![SR * 0.25], 1, 2);
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(SR);
        assert!(approx(script.eval(&[0.0], &mut regs, &ctx), 0.25));
        assert!(approx(script.eval(&[0.0], &mut regs, &ctx), 0.5));
        // Rising edge of sync → phase resets to 0.
        assert!(approx(script.eval(&[1.0], &mut regs, &ctx), 0.0));
        // Sync still high (no new edge) → advances again.
        assert!(approx(script.eval(&[1.0], &mut regs, &ctx), 0.25));
    }

    #[test]
    fn accum_reset_clears_on_rising_edge() {
        // x = 1.0 const; reset from source[0]. Cells 0,1.
        let code = [Op::PushConst(0), Op::PushSource(0), Op::AccumReset(0)];
        let script = CompiledScript::new(code.to_vec(), vec![1.0], 1, 2);
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(SR);
        assert!(approx(script.eval(&[0.0], &mut regs, &ctx), 1.0));
        assert!(approx(script.eval(&[0.0], &mut regs, &ctx), 2.0));
        // Rising edge → clears (skips x this block).
        assert!(approx(script.eval(&[1.0], &mut regs, &ctx), 0.0));
        // Edge held high (no new edge) → resumes accumulating.
        assert!(approx(script.eval(&[1.0], &mut regs, &ctx), 1.0));
    }

    #[test]
    fn rand_smooth_is_seeded_continuous_and_decorrelated() {
        // rate = SR * 0.25 → quarter-cycle per block. Cells 0,1,2.
        let code = [Op::PushConst(0), Op::RandSmooth(0)];
        let script = CompiledScript::new(code.to_vec(), vec![SR * 0.25], 0, 3);
        let ctx = EvalContext::new(SR);

        let mut v0 = RegisterFile::new(0, SEED);
        // Cold start must NOT glide 0 → 0: the first block already moves.
        let a0 = script.eval(&[], &mut v0, &ctx);
        let a1 = script.eval(&[], &mut v0, &ctx);
        assert!(
            a0 != a1 || a0 != 0.0,
            "first segment must be seeded, not flat 0"
        );
        // Output stays inside the seeded value range [0, 1).
        for _ in 0..16 {
            let v = script.eval(&[], &mut v0, &ctx);
            assert!((0.0..1.0).contains(&v), "rand_smooth out of [0,1): {v}");
        }

        // Different voices decorrelate.
        let mut v1 = RegisterFile::new(1, SEED);
        let mut v0b = RegisterFile::new(0, SEED);
        assert!(script.eval(&[], &mut v1, &ctx) != script.eval(&[], &mut v0b, &ctx));
    }

    #[test]
    fn rand_smooth_negative_rate_stays_bounded() {
        // A negative rate must NOT drive the phase below 0 and let the output
        // escape [0,1); it freezes instead.
        let code = [Op::PushConst(0), Op::RandSmooth(0)];
        let script = CompiledScript::new(code.to_vec(), vec![-100.0], 0, 3);
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(SR);
        for _ in 0..32 {
            let v = script.eval(&[], &mut regs, &ctx);
            assert!(
                (0.0..1.0).contains(&v),
                "escaped [0,1) on negative rate: {v}"
            );
        }
    }

    #[test]
    fn prng_is_per_voice_decorrelated_and_deterministic() {
        let code = [Op::PushConst(0), Op::PushConst(1), Op::Rand];
        let script = CompiledScript::new(code.to_vec(), vec![0.0, 1.0], 0, 0);
        let ctx = EvalContext::new(SR);

        let mut v0 = RegisterFile::new(0, SEED);
        let mut v1 = RegisterFile::new(1, SEED);
        let a0 = script.eval(&[], &mut v0, &ctx);
        let a1 = script.eval(&[], &mut v1, &ctx);
        assert!(a0 != a1, "different voices must decorrelate");
        assert!((0.0..1.0).contains(&a0));

        // Re-seed (note-on) → identical stream again (deterministic retrigger).
        v0.reset(0, SEED);
        let a0_again = script.eval(&[], &mut v0, &ctx);
        assert!(approx(a0, a0_again));
    }
}
