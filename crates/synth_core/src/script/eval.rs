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
///
/// The VM is rate-agnostic: `control_rate` is whatever Hz the evaluation runs at,
/// and it drives `dt = 1/rate` for the time-based stateful ops (`lag`, `slew`,
/// `phasor`). At control rate that is `sample_rate / block_size` (one eval per
/// block); at audio rate ([`eval_block`](CompiledScript::eval_block)) it is the
/// full `sample_rate` (one eval per sample) — see [`Self::audio`].
#[derive(Debug, Clone, Copy)]
pub struct EvalContext {
    /// Evaluation rate in Hz. Control rate (`sample_rate / block_size`) for the
    /// per-block [`eval`](CompiledScript::eval); the audio sample rate for the
    /// per-sample [`eval_block`](CompiledScript::eval_block).
    pub control_rate: f32,
}

impl EvalContext {
    #[must_use]
    pub fn new(control_rate: f32) -> Self {
        Self { control_rate }
    }

    /// Construct an audio-rate context: the evaluation rate is the full sample
    /// rate (one eval per sample). Just an intent-revealing alias for
    /// [`Self::new`] — the VM math is identical, only the rate differs.
    #[must_use]
    pub fn audio(sample_rate: f32) -> Self {
        Self::new(sample_rate)
    }

    /// Seconds per evaluation step (one block at control rate, one sample at
    /// audio rate).
    fn dt(self) -> f32 {
        if self.control_rate > 0.0 {
            1.0 / self.control_rate
        } else {
            0.0
        }
    }
}

/// Which source registers an audio-rate script fills per-sample, rather than once
/// per block. Everything else (macros, context vars, slow params) is resolved
/// once and stays constant across the block; only these registers tick each
/// sample (the per-block-constant vs per-sample source split). A `None` field
/// means the script does not read that audio input.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScriptRegister(u16);

impl ScriptRegister {
    pub const fn new(index: u16) -> Self {
        Self(index)
    }

    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AudioBindings {
    /// Source register fed the per-sample left / mono audio input (`in` / `in-l`).
    pub in_left: Option<ScriptRegister>,
    /// Source register fed the per-sample right audio input (`in-r`).
    pub in_right: Option<ScriptRegister>,
    /// Source register fed `first_sample` — `1.0` only at sample 0 of the note's
    /// very first block, `0.0` everywhere after (audio-rate one-shot init).
    pub first_sample: Option<ScriptRegister>,
}

/// Number of addressable output slots captured during one evaluation. The
/// dialect decides what each slot means: the audio dialect uses `0 = left`,
/// `1 = right`; the `note_event` dialect uses `0 = pitch`, `1 = vel`,
/// `2 = dur`, `3 = gate`.
const OUT_SLOTS: usize = 4;

/// Output slots captured while running one evaluation. Each slot is `Some(v)`
/// once an `out.<member>` statement has written it (`Op::StoreOut`), and
/// `None` otherwise — a bare `out = expr` writes nothing here and leaves its
/// value on the value stack, so `eval_block` falls back to duplicating the stack
/// top to both unwritten audio channels, and `eval_note` reports unwritten note
/// fields as `None` (pass-through).
#[derive(Debug, Clone, Copy, Default)]
struct OutCapture {
    slots: [Option<f32>; OUT_SLOTS],
}

/// The four note-event outputs a `note_event`-dialect script may write, produced
/// by [`CompiledScript::eval_note`]. Each field is `Some(v)` iff the script wrote
/// that `out.<field>` statement, and `None` otherwise — an unwritten field means
/// the note-graph consumer should pass the source note's value through unchanged.
///
/// No clamping or sentinel interpretation is applied here (the store only
/// sanitizes NaN/Inf to 0); the consumer interprets the raw values — e.g. a
/// negative `vel` to drop a note, or a negative `dur` to restore "plays until
/// cut" — before clamping the survivors into legal ranges.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NoteOutputs {
    pub pitch: Option<f32>,
    pub vel: Option<f32>,
    pub dur: Option<f32>,
    pub gate: Option<f32>,
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
        let mut stack = Stack::new();
        let mut locals = [0.0f32; MAX_LOCALS];
        let mut out = OutCapture::default();
        self.run(sources, regs, ctx.dt(), &mut stack, &mut locals, &mut out);
        finite_or_zero(stack.top())
    }

    /// Evaluate a `note_event`-dialect script once for one note event. `sources`
    /// holds the note-field / `in1..in4` values indexed by source register (the
    /// caller builds it from the compiled input list); `regs` is a caller-owned
    /// [`RegisterFile`] (a `note_event` transform is stateless per event, so the
    /// caller resets or discards it between events). Returns the four
    /// [`NoteOutputs`]: `Some(v)` for each `out.<field>` the script wrote, `None`
    /// for an unwritten (pass-through) field. Real-time safe.
    #[must_use]
    pub fn eval_note(&self, sources: &[f32], regs: &mut RegisterFile) -> NoteOutputs {
        let mut stack = Stack::new();
        let mut locals = [0.0f32; MAX_LOCALS];
        let mut out = OutCapture::default();
        // A note_event script runs once per event with no per-block/per-sample
        // time integration, so `dt` is 0 (time-based stateful ops simply hold).
        // The transport position is available to the script via the `tick` field.
        self.run(sources, regs, 0.0, &mut stack, &mut locals, &mut out);
        NoteOutputs {
            pitch: out.slots[0],
            vel: out.slots[1],
            dur: out.slots[2],
            gate: out.slots[3],
        }
    }

    /// Evaluate a **control-ports**-dialect script (the `Script` module) for one
    /// control block, capturing up to four outputs `out1..out4`
    /// (`Op::StoreOut(0..3)`). The control-rate twin of [`eval_note`](Self::eval_note):
    /// `sources` holds the voice-resolved block constants plus the `in1..in4` port
    /// values indexed by source register, `regs` is this voice's persistent state.
    ///
    /// Slot `i` is `Some(v)` when the program wrote `out{i+1}`; an unwritten higher
    /// slot stays `None` (the module emits `0` / a silent port). Slot 0 additionally
    /// falls back to the value-stack top, so a bare `out = expr` (which leaves its
    /// value on the stack rather than storing) still yields `out1`. Real-time safe.
    #[must_use]
    pub fn eval_multi(
        &self,
        sources: &[f32],
        regs: &mut RegisterFile,
        ctx: &EvalContext,
    ) -> [Option<f32>; OUT_SLOTS] {
        let mut stack = Stack::new();
        let mut locals = [0.0f32; MAX_LOCALS];
        let mut out = OutCapture::default();
        self.run(sources, regs, ctx.dt(), &mut stack, &mut locals, &mut out);
        let mut slots = out.slots;
        if slots[0].is_none() {
            slots[0] = Some(finite_or_zero(stack.top()));
        }
        slots
    }

    /// Run the straight-line program once over `sources`, leaving the result on
    /// the value stack (a bare `out = expr`) and/or in `out` (the `out.left` /
    /// `out.right` multi-out grammar). The single shared op-dispatch loop behind
    /// both [`eval`](Self::eval) (per block) and [`eval_block`](Self::eval_block)
    /// (per sample) — kept in one place so the two rates can never diverge.
    /// `dt` is the seconds-per-step the time-based ops integrate over.
    fn run(
        &self,
        sources: &[f32],
        regs: &mut RegisterFile,
        dt: f32,
        stack: &mut Stack,
        locals: &mut [f32; MAX_LOCALS],
        out: &mut OutCapture,
    ) {
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

                Op::Add => binop(stack, |a, b| a + b),
                Op::Sub => binop(stack, |a, b| a - b),
                Op::Mul => binop(stack, |a, b| a * b),
                Op::Div => binop(stack, safe_div),
                Op::Rem => binop(stack, |a, b| if b == 0.0 { 0.0 } else { a % b }),
                Op::Pow => binop(stack, f32::powf),

                Op::Neg => {
                    let a = stack.pop();
                    stack.push(-a);
                }
                Op::Not => {
                    let a = stack.pop();
                    stack.push(b2f(a == 0.0));
                }

                Op::Eq => binop(stack, |a, b| b2f(a == b)),
                Op::Ne => binop(stack, |a, b| b2f(a != b)),
                Op::Lt => binop(stack, |a, b| b2f(a < b)),
                Op::Gt => binop(stack, |a, b| b2f(a > b)),
                Op::Le => binop(stack, |a, b| b2f(a <= b)),
                Op::Ge => binop(stack, |a, b| b2f(a >= b)),
                Op::And => binop(stack, |a, b| b2f(a != 0.0 && b != 0.0)),
                Op::Or => binop(stack, |a, b| b2f(a != 0.0 || b != 0.0)),

                Op::Select => {
                    let els = stack.pop();
                    let then = stack.pop();
                    let cond = stack.pop();
                    stack.push(if cond != 0.0 { then } else { els });
                }

                Op::Call(b) => apply_builtin(b, stack),

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

                Op::TableLin { base, len } => {
                    let pos = stack.pop();
                    let last = len.saturating_sub(1);
                    // Clamp pos into 0..=last; floor → lower index, fract → blend.
                    let p = if pos.is_nan() {
                        0.0
                    } else {
                        pos.clamp(0.0, f32::from(last))
                    };
                    let frac = p - p.floor();
                    let i0 = p.floor() as u16; // in 0..=last (p is clamped)
                    let i1 = (i0 + 1).min(last); // next neighbour, clamped
                    let a = self
                        .constants
                        .get(base.saturating_add(i0) as usize)
                        .copied()
                        .unwrap_or(0.0);
                    let b = self
                        .constants
                        .get(base.saturating_add(i1) as usize)
                        .copied()
                        .unwrap_or(0.0);
                    stack.push(a + (b - a) * frac);
                }
                Op::ScaleSnap { base, len } => {
                    let p = stack.pop();
                    // Split into octave + pitch class, then snap the pitch class
                    // to the nearest table member testing S, S-12, S+12 so a class
                    // just under the octave can snap up to the next root.
                    let octave = (p / 12.0).floor();
                    let pc = p - 12.0 * octave;
                    let mut best = 0.0;
                    let mut best_dist = f32::INFINITY;
                    for k in 0..len {
                        let s = self
                            .constants
                            .get(base.saturating_add(k) as usize)
                            .copied()
                            .unwrap_or(0.0);
                        for cand in [s - 12.0, s, s + 12.0] {
                            let d = (pc - cand).abs();
                            if d < best_dist {
                                best_dist = d;
                                best = cand;
                            }
                        }
                    }
                    stack.push(12.0 * octave + best);
                }

                // ---- Phase 4: author-addressable state + multi-out ----------
                Op::LoadState(i) => stack.push(regs.state_get(i)),
                Op::StoreState(i) => {
                    let v = stack.pop();
                    regs.state_set(i, v); // layer-2 sanitize: NaN/Inf → 0
                }
                Op::StoreOut(slot) => {
                    let v = finite_or_zero(stack.pop());
                    if let Some(cell) = out.slots.get_mut(slot as usize) {
                        *cell = Some(v);
                    }
                }
            }
        }
    }

    /// Evaluate the script once per sample across one audio block — the
    /// audio-rate counterpart to [`eval`](Self::eval).
    ///
    /// The value stack and `locals` are allocated once and kept warm across the
    /// whole block (only `sp` is reset per sample via [`Stack::clear`], never the
    /// 64-float buffer), so per-sample overhead is just the op loop. `sources` is
    /// pre-filled by the caller with the **per-block-constant** values (macros,
    /// context vars, slow params); the registers named in `bindings` are the only
    /// ones overwritten each sample — the audio inputs and the `first_sample`
    /// one-shot. `ctx` carries the audio sample rate (see [`EvalContext::audio`]).
    ///
    /// Output: each sample's left/right is the `out.left` / `out.right` value
    /// (`Op::StoreOut`) when the program writes them, else the bare
    /// `out = expr` value (the value-stack top) duplicated to both channels.
    /// `first_block` is `true` only for the note's very first block, so
    /// `first_sample` fires exactly once at global sample 0. Real-time safe.
    #[allow(clippy::too_many_arguments)]
    pub fn eval_block(
        &self,
        sources: &mut [f32],
        bindings: &AudioBindings,
        in_left: &[f32],
        in_right: &[f32],
        out_left: &mut [f32],
        out_right: &mut [f32],
        regs: &mut RegisterFile,
        ctx: &EvalContext,
        first_block: bool,
    ) {
        let dt = ctx.dt();
        let n = out_left.len().min(out_right.len());
        let mut stack = Stack::new();
        let mut locals = [0.0f32; MAX_LOCALS];

        for i in 0..n {
            // Per-sample source injection: only the audio-in and `first_sample`
            // registers tick; every other source stays at its block-constant value.
            if let Some(r) = bindings.in_left {
                set_source(sources, r.as_u16(), in_left.get(i).copied().unwrap_or(0.0));
            }
            if let Some(r) = bindings.in_right {
                set_source(sources, r.as_u16(), in_right.get(i).copied().unwrap_or(0.0));
            }
            if let Some(r) = bindings.first_sample {
                set_source(sources, r.as_u16(), f32::from(first_block && i == 0));
            }

            stack.clear(); // reset sp only — keep the warm buffer
            let mut out = OutCapture::default();
            self.run(sources, regs, dt, &mut stack, &mut locals, &mut out);

            // Mono fallback: a bare `out = expr` leaves its value on the stack;
            // an unwritten channel (no `out.left`/`out.right`) duplicates it.
            let mono = finite_or_zero(stack.top());
            out_left[i] = out.slots[0].unwrap_or(mono);
            out_right[i] = out.slots[1].unwrap_or(mono);
        }
    }
}

/// Overwrite one source register if it is in range (per-sample audio injection).
#[inline]
fn set_source(sources: &mut [f32], reg: u16, value: f32) {
    if let Some(slot) = sources.get_mut(reg as usize) {
        *slot = value;
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

    /// Reset the stack to empty by zeroing the stack pointer only. The warm-frame
    /// audio loop calls this per sample instead of `Stack::new()`, which would
    /// `memset` the whole 64-float buffer every sample — stale slots above `sp`
    /// are never read, so leaving them is safe.
    fn clear(&mut self) {
        self.sp = 0;
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

    const CR: f32 = 750.0; // 48 kHz / 64-sample blocks
    const SEED: u64 = 0x5EED;

    fn run(code: &[Op], constants: &[f32], sources: &[f32]) -> f32 {
        let script =
            CompiledScript::new(code.to_vec(), constants.to_vec(), sources.len() as u16, 4);
        let mut regs = RegisterFile::new(0, SEED);
        script.eval(sources, &mut regs, &EvalContext::new(CR))
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
    fn mtof_takes_raw_midi() {
        use crate::script::bytecode::Builtin;
        // Raw MIDI note in (industry standard): A4 = 69 → 440 Hz exactly,
        // middle C = 60 → ~261.63 Hz. (Not a normalized 0..1 note.)
        let code = [Op::PushConst(0), Op::Call(Builtin::Mtof)];
        assert!(approx(run(&code, &[69.0], &[]), 440.0));
        assert!((run(&code, &[60.0], &[]) - 261.6256).abs() < 0.01);
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
        let out = script.eval(&[], &mut regs, &EvalContext::new(CR));
        assert_eq!(out, 0.0);
        assert_eq!(regs.state_get(0), 0.0);
    }

    #[test]
    fn lag_smooths_toward_target() {
        // alpha 0.5, x 1.0, state starts 0 → 0.5, then 0.75.
        let code = [Op::PushConst(0), Op::PushConst(1), Op::Lag(0)];
        let script = CompiledScript::new(code.to_vec(), vec![1.0, 0.5], 0, 1);
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(CR);
        assert!(approx(script.eval(&[], &mut regs, &ctx), 0.5));
        assert!(approx(script.eval(&[], &mut regs, &ctx), 0.75));
    }

    #[test]
    fn phasor_advances_by_rate_times_dt() {
        // rate = CR * 0.25 → +0.25 per block.
        let code = [Op::PushConst(0), Op::Phasor(0)];
        let script = CompiledScript::new(code.to_vec(), vec![CR * 0.25], 0, 1);
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(CR);
        assert!(approx(script.eval(&[], &mut regs, &ctx), 0.25));
        assert!(approx(script.eval(&[], &mut regs, &ctx), 0.5));
    }

    #[test]
    fn sah_holds_until_rising_edge() {
        // Hold source[0]; trig = source[1]. Sample only on a rising edge.
        let code = [Op::PushSource(0), Op::PushSource(1), Op::Sah(0)];
        let script = CompiledScript::new(code.to_vec(), vec![], 2, 2);
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(CR);
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
            script.eval(&[i], &mut regs, &EvalContext::new(CR))
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
            script.eval(&[], &mut regs, &EvalContext::new(CR)),
            7.0
        ));
    }

    #[test]
    fn phasor_sync_resets_phase_on_rising_edge() {
        // rate = CR * 0.25 (+0.25/block); sync from source[0]. Cells 0,1.
        let code = [Op::PushConst(0), Op::PushSource(0), Op::PhasorSync(0)];
        let script = CompiledScript::new(code.to_vec(), vec![CR * 0.25], 1, 2);
        let mut regs = RegisterFile::new(0, SEED);
        let ctx = EvalContext::new(CR);
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
        let ctx = EvalContext::new(CR);
        assert!(approx(script.eval(&[0.0], &mut regs, &ctx), 1.0));
        assert!(approx(script.eval(&[0.0], &mut regs, &ctx), 2.0));
        // Rising edge → clears (skips x this block).
        assert!(approx(script.eval(&[1.0], &mut regs, &ctx), 0.0));
        // Edge held high (no new edge) → resumes accumulating.
        assert!(approx(script.eval(&[1.0], &mut regs, &ctx), 1.0));
    }

    #[test]
    fn rand_smooth_is_seeded_continuous_and_decorrelated() {
        // rate = CR * 0.25 → quarter-cycle per block. Cells 0,1,2.
        let code = [Op::PushConst(0), Op::RandSmooth(0)];
        let script = CompiledScript::new(code.to_vec(), vec![CR * 0.25], 0, 3);
        let ctx = EvalContext::new(CR);

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
        let ctx = EvalContext::new(CR);
        for _ in 0..32 {
            let v = script.eval(&[], &mut regs, &ctx);
            assert!(
                (0.0..1.0).contains(&v),
                "escaped [0,1) on negative rate: {v}"
            );
        }
    }

    #[test]
    fn table_lin_interpolates_and_clamps() {
        // Table [0, 10, 20] at base 0; pos from source[0].
        let consts = vec![0.0, 10.0, 20.0];
        let code = [Op::PushSource(0), Op::TableLin { base: 0, len: 3 }];
        let at = |pos: f32| {
            let script = CompiledScript::new(code.to_vec(), consts.clone(), 1, 0);
            let mut regs = RegisterFile::new(0, SEED);
            script.eval(&[pos], &mut regs, &EvalContext::new(CR))
        };
        assert!(approx(at(0.0), 0.0));
        assert!(approx(at(0.5), 5.0)); // halfway between 0 and 10
        assert!(approx(at(1.25), 12.5)); // 10 + 0.25*(20-10)
        assert!(approx(at(2.0), 20.0));
        assert!(approx(at(5.0), 20.0)); // clamp high
        assert!(approx(at(-1.0), 0.0)); // clamp low
        assert!(approx(at(f32::NAN), 0.0)); // NaN → 0
    }

    #[test]
    fn scale_snap_is_octave_aware() {
        // Major scale pitch classes.
        let consts = vec![0.0, 2.0, 4.0, 5.0, 7.0, 9.0, 11.0];
        let code = [Op::PushSource(0), Op::ScaleSnap { base: 0, len: 7 }];
        let snap = |p: f32| {
            let script = CompiledScript::new(code.to_vec(), consts.clone(), 1, 0);
            let mut regs = RegisterFile::new(0, SEED);
            script.eval(&[p], &mut regs, &EvalContext::new(CR))
        };
        assert!(approx(snap(0.2), 0.0)); // near C → C
        assert!(approx(snap(3.4), 4.0)); // between D# region → E (4)
        assert!(approx(snap(6.6), 7.0)); // → G
        // Octave-aware: 11.8 is numerically closest to 11 (B) but musically
        // closest to 12 (C of the next octave) via the +12 candidate.
        assert!(approx(snap(11.8), 12.0));
        // Works in a higher octave: 24.2 → 24 (C two octaves up).
        assert!(approx(snap(24.2), 24.0));
        // And below zero: -0.2 → 0 (root), -1.0 → -1 (B below, i.e. 11-12).
        assert!(approx(snap(-0.2), 0.0));
    }

    #[test]
    fn prng_is_per_voice_decorrelated_and_deterministic() {
        let code = [Op::PushConst(0), Op::PushConst(1), Op::Rand];
        let script = CompiledScript::new(code.to_vec(), vec![0.0, 1.0], 0, 0);
        let ctx = EvalContext::new(CR);

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

    // ---- Phase 4: audio-rate `eval_block` ---------------------------------

    const SR: f32 = 48_000.0;

    /// Run an audio-rate program over one block. `sources` is the pre-resolved
    /// block-constant slice (audio-in slots are placeholders, overwritten by the
    /// bindings); returns the (left, right) output buffers.
    #[allow(clippy::too_many_arguments)]
    fn run_block(
        code: &[Op],
        constants: &[f32],
        source_count: u16,
        state_count: u16,
        bindings: &AudioBindings,
        sources: &mut [f32],
        input: &[f32],
        first_block: bool,
    ) -> (Vec<f32>, Vec<f32>) {
        let script =
            CompiledScript::new(code.to_vec(), constants.to_vec(), source_count, state_count);
        let mut regs = RegisterFile::new(0, SEED);
        let n = input.len();
        let mut l = vec![0.0; n];
        let mut r = vec![0.0; n];
        script.eval_block(
            sources,
            bindings,
            input,
            input,
            &mut l,
            &mut r,
            &mut regs,
            &EvalContext::audio(SR),
            first_block,
        );
        (l, r)
    }

    #[test]
    fn eval_block_passthrough_is_bit_exact() {
        // `out = in` — left/right must equal the input sample-for-sample.
        let bindings = AudioBindings {
            in_left: Some(ScriptRegister::new(0)),
            ..Default::default()
        };
        let input: Vec<f32> = (0..8).map(|i| (i as f32 * 0.131).sin()).collect();
        let mut sources = vec![0.0; 1];
        let (l, r) = run_block(
            &[Op::PushSource(0)],
            &[],
            1,
            0,
            &bindings,
            &mut sources,
            &input,
            true,
        );
        for i in 0..input.len() {
            assert_eq!(l[i], input[i], "left passthrough bit-exact");
            assert_eq!(r[i], input[i], "mono out duplicates to right");
        }
    }

    #[test]
    fn eval_block_applies_per_sample_gain() {
        // `out = in * 0.5`.
        let bindings = AudioBindings {
            in_left: Some(ScriptRegister::new(0)),
            ..Default::default()
        };
        let input: Vec<f32> = vec![1.0, -0.4, 0.8, -1.0];
        let mut sources = vec![0.0; 1];
        let (l, _r) = run_block(
            &[Op::PushSource(0), Op::PushConst(0), Op::Mul],
            &[0.5],
            1,
            0,
            &bindings,
            &mut sources,
            &input,
            true,
        );
        for i in 0..input.len() {
            assert!(approx(l[i], input[i] * 0.5));
        }
    }

    #[test]
    fn eval_block_scripted_one_pole_matches_reference() {
        // A custom IIR via LoadState/StoreState: y = y + a*(x - y); out = y.
        //   LoadState(0) -> y
        //   PushSource(0) -> y, x
        //   LoadState(0) -> y, x, y
        //   Sub          -> y, (x - y)
        //   PushConst(0) -> y, (x-y), a
        //   Mul          -> y, a*(x-y)
        //   Add          -> y + a*(x-y)
        //   StoreState(0)-> []  state[0] = new y
        //   LoadState(0) -> new y (mono out)
        let a = 0.2_f32;
        let code = [
            Op::LoadState(0),
            Op::PushSource(0),
            Op::LoadState(0),
            Op::Sub,
            Op::PushConst(0),
            Op::Mul,
            Op::Add,
            Op::StoreState(0),
            Op::LoadState(0),
        ];
        let bindings = AudioBindings {
            in_left: Some(ScriptRegister::new(0)),
            ..Default::default()
        };
        let input: Vec<f32> = (0..32).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect(); // impulse
        let mut sources = vec![0.0; 1];
        let (l, _r) = run_block(&code, &[a], 1, 1, &bindings, &mut sources, &input, true);

        // Reference one-pole.
        let mut y = 0.0_f32;
        for (i, &x) in input.iter().enumerate() {
            y += a * (x - y);
            assert!(approx(l[i], y), "sample {i}: {} != {y}", l[i]);
        }
    }

    #[test]
    fn eval_block_multi_out_writes_independent_channels() {
        // out.left = in;  out.right = -in  (via StoreOut).
        //   PushSource(0); StoreOut(0)   -> left = in
        //   PushSource(0); Neg; StoreOut(1) -> right = -in
        let code = [
            Op::PushSource(0),
            Op::StoreOut(0),
            Op::PushSource(0),
            Op::Neg,
            Op::StoreOut(1),
        ];
        let bindings = AudioBindings {
            in_left: Some(ScriptRegister::new(0)),
            ..Default::default()
        };
        let input: Vec<f32> = vec![0.25, -0.5, 1.0];
        let mut sources = vec![0.0; 1];
        let (l, r) = run_block(&code, &[], 1, 0, &bindings, &mut sources, &input, true);
        for i in 0..input.len() {
            assert!(approx(l[i], input[i]));
            assert!(approx(r[i], -input[i]));
        }
    }

    #[test]
    fn eval_block_first_sample_fires_once_per_note() {
        // `out = first_sample` — 1 only at global sample 0, then 0 forever.
        let bindings = AudioBindings {
            first_sample: Some(ScriptRegister::new(0)),
            ..Default::default()
        };
        let zeros = vec![0.0; 4];

        // First block: sample 0 reads 1, the rest 0.
        let mut sources = vec![0.0; 1];
        let (l0, _r) = run_block(
            &[Op::PushSource(0)],
            &[],
            1,
            0,
            &bindings,
            &mut sources,
            &zeros,
            true,
        );
        assert!(approx(l0[0], 1.0));
        assert!(l0[1..].iter().all(|&v| approx(v, 0.0)));

        // A non-first block never fires.
        let mut sources = vec![0.0; 1];
        let (l1, _r) = run_block(
            &[Op::PushSource(0)],
            &[],
            1,
            0,
            &bindings,
            &mut sources,
            &zeros,
            false,
        );
        assert!(l1.iter().all(|&v| approx(v, 0.0)));
    }

    // ---- note_event dialect: `eval_note` ----------------------------------

    #[test]
    fn eval_note_captures_written_fields_only() {
        // out.pitch (slot 0) = source[0] + 12; no other slot written → None.
        let code = [
            Op::PushSource(0),
            Op::PushConst(0),
            Op::Add,
            Op::StoreOut(0),
        ];
        let script = CompiledScript::new(code.to_vec(), vec![12.0], 1, 0);
        let mut regs = RegisterFile::new(0, SEED);
        let outs = script.eval_note(&[60.0], &mut regs);
        assert_eq!(outs.pitch, Some(72.0));
        assert_eq!(outs.vel, None);
        assert_eq!(outs.dur, None);
        assert_eq!(outs.gate, None);
    }

    #[test]
    fn eval_note_writes_every_slot_independently() {
        // Write all four slots from four distinct constants.
        let code = [
            Op::PushConst(0),
            Op::StoreOut(0), // pitch
            Op::PushConst(1),
            Op::StoreOut(1), // vel
            Op::PushConst(2),
            Op::StoreOut(2), // dur
            Op::PushConst(3),
            Op::StoreOut(3), // gate
        ];
        let script = CompiledScript::new(code.to_vec(), vec![64.0, 0.5, 120.0, 0.9], 0, 0);
        let mut regs = RegisterFile::new(0, SEED);
        let outs = script.eval_note(&[], &mut regs);
        assert_eq!(outs.pitch, Some(64.0));
        assert_eq!(outs.vel, Some(0.5));
        assert_eq!(outs.dur, Some(120.0));
        assert_eq!(outs.gate, Some(0.9));
    }

    #[test]
    fn eval_note_sanitizes_nan_to_zero_on_store() {
        // A NaN written to a field is stored as 0 (never leaks NaN to the consumer).
        let code = [Op::PushConst(0), Op::StoreOut(1)];
        let script = CompiledScript::new(code.to_vec(), vec![f32::NAN], 0, 0);
        let mut regs = RegisterFile::new(0, SEED);
        let outs = script.eval_note(&[], &mut regs);
        assert_eq!(outs.vel, Some(0.0));
        assert_eq!(outs.pitch, None);
    }

    #[test]
    fn eval_block_nan_in_feedback_is_clamped() {
        // A feedback loop fed a NaN must clamp the state to 0, never blow up:
        //   y = y + in;  out = y   (accumulate the input into state)
        // Feeding a NaN once must not poison subsequent samples.
        let code = [
            Op::LoadState(0),
            Op::PushSource(0),
            Op::Add,
            Op::StoreState(0),
            Op::LoadState(0),
        ];
        let bindings = AudioBindings {
            in_left: Some(ScriptRegister::new(0)),
            ..Default::default()
        };
        let input: Vec<f32> = vec![0.1, f32::NAN, 0.2, 0.3];
        let mut sources = vec![0.0; 1];
        let (l, _r) = run_block(&code, &[], 1, 1, &bindings, &mut sources, &input, true);
        // Every output is finite — the NaN never propagates past the state write.
        assert!(l.iter().all(|v| v.is_finite()), "outputs: {l:?}");
        // The NaN sample writes `finite_or_zero(NaN) = 0` into the accumulator, so
        // the loop resets to 0 there and resumes cleanly: 0.1, 0, 0.2, 0.5.
        assert!(approx(l[0], 0.1));
        assert!(approx(l[1], 0.0));
        assert!(approx(l[2], 0.2));
        assert!(approx(l[3], 0.5));
    }

    // ---- control-ports dialect: `eval_multi` ------------------------------

    #[test]
    fn eval_multi_captures_written_ports_only() {
        // out1 = 0.5 (slot 0); out3 = -0.25 (slot 2); out2/out4 unwritten → None.
        let code = [
            Op::PushConst(0),
            Op::StoreOut(0),
            Op::PushConst(1),
            Op::StoreOut(2),
        ];
        let script = CompiledScript::new(code.to_vec(), vec![0.5, -0.25], 0, 0);
        let mut regs = RegisterFile::new(0, SEED);
        let outs = script.eval_multi(&[], &mut regs, &EvalContext::new(CR));
        assert_eq!(outs[0], Some(0.5));
        assert_eq!(outs[1], None);
        assert_eq!(outs[2], Some(-0.25));
        assert_eq!(outs[3], None);
    }

    #[test]
    fn eval_multi_bare_out_falls_back_to_stack_top() {
        // A bare `out = expr` leaves its value on the stack (no store); slot 0
        // must fall back to it so `out` still means `out1`.
        let bare = CompiledScript::new(vec![Op::PushConst(0)], vec![0.7], 0, 0);
        let mut regs = RegisterFile::new(0, SEED);
        let outs = bare.eval_multi(&[], &mut regs, &EvalContext::new(CR));
        assert_eq!(outs[0], Some(0.7));
        assert_eq!(outs[1], None);
        assert_eq!(outs[2], None);
        assert_eq!(outs[3], None);
    }
}
