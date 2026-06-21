# Plan: YAMS language gaps — arrays + missing primitives

Status: **design sketch** (no code yet). Companion to `plans/yams-realtime-plan.md`
(which generalizes *where* scripts run) — this document is about *what the language
can express*. Source of truth for the current language: `docs/yams.md`. Toolchain:
`crates/synth_script` (lexer/parser/ast/compile/fmt); real-time VM:
`crates/synth_core/src/script` (`bytecode.rs`, `eval.rs`, `bound.rs`).

The throughline: today YAMS can only do **continuous math over scalar sources**. It
cannot look a value up in a hand-authored table, snap pitch to a scale, phase-align a
ramp to an event, or read another module's parameter. The items below close those
gaps. Each is scoped to preserve YAMS's two load-bearing invariants:

- **Everything is `f32`** — one value type, no booleans, no aggregates-as-values.
- **RT-safe by construction** — flat bytecode, pre-allocated registers, no allocation,
  no recursion, NaN-free / clamp / disable-and-keep.

Caps to respect (from `docs/yams.md`): 256 instructions, 64 registers (≤32 sources,
≤16 state, ≤16 scratch), 32 source bindings, 32 nesting depth, 4 KiB source text.

---

## Priority summary

Ranked by musical payoff per unit of build cost.

| # | Item | Kind | Payoff | Cost | Notes |
|---|------|------|--------|------|-------|
| 1 | **Arrays (const tables)** | capability | high | medium | step sequencer + custom shapes; the headline |
| 2 | **Scale-aware pitch quantize** | capability | high | low–med | makes pitch modulation musical; pairs with #1 |
| 3 | **Phasor / ramp sync (reset input)** | capability | high | low | phase-align ramps to note-on / triggers |
| 4 | **Smooth random LFO** | capability | medium | low | the one common modulation source still missing |
| 5 | **Parameter sources** (`flt-1.cutoff`) | capability | medium | high | already tracked (TODO §3.5); cross-crate resolver |
| 6 | **Ergonomic helpers** | ergonomics | low–med | trivial | `unipolar`/`bipolar`, `pulse(div)` clock |

Deliberately **out of scope** (see §7): mutable arrays, user-defined functions,
multi-out (owned by `yams-realtime-plan.md`), `prev`/`z1` (subsumed by that plan's
Phase 4 `state` declaration).

---

## 1. Arrays — const lookup tables (the headline)

**The gap.** YAMS can compute a continuous function of its inputs but cannot index a
hand-authored table. Faking it with nested `?:` (`i==0 ? a : i==1 ? b : …`) blows the
256-instruction cap fast and is unreadable. The two use cases that justify a real
construct:

```yams
# Per-parameter step sequencer (pairs with the transport `beat` context).
arr seq = [0, 0.5, 1, 0.25, 0, 0.75, 0.5, 0]
out = seq[floor(beat) % 8]
```

```yams
# Custom LFO / transfer shape indexed by a phasor.
arr shape = [0, 0.3, 0.8, 1, 0.8, 0.3, 0, -0.3]
out = shape[floor(phasor(2) * len(shape))]
```

The step-sequencer case is genuinely new: it turns any modulation slot into a
rhythmic, per-parameter step generator on top of the `beat`/`bar_phase` transport
vars that already landed.

### Design — the invariant-preserving shape

The key decision that keeps this **cheap**: arrays are **compile-time-constant
literals, read-only**. With that, the type system does *not* change:

- An array name is **not an expression on its own**. Only `name[expr]` is — and it
  yields an `f32`. An array is a *syntactic indexing construct over a constant pool*,
  not a new value type you can pass around or return. "Everything is `f32`" holds.
- Values live in a read-only constant pool baked at compile time (a `[f32]` arena in
  `BoundScript`). Indexing is **one new opcode**, e.g.
  `IndexConst { base: u16, len: u16 }` that pops an index, computes
  `pool[base + clamp(round(i), 0, len-1)]`, pushes the result. RT-safe: read-only
  indexed load, no allocation, bounds-clamp ⇒ no OOB panic. Fits the existing
  NaN-free / clamp / disable-and-keep philosophy exactly.
- `len(arr)` is a compile-time-constant builtin (folds to a literal), so
  `i % len(seq)` needs no runtime length lookup.

### Open design decisions

1. **Out-of-bounds index.** Clamp (good for curves) vs wrap/modulo (good for
   sequences). **Lean: clamp as the safety net**, and let the user write `% len(arr)`
   for wrap — consistent with how `/0 → 0` already works. Document both idioms.
2. **Index rounding — `floor`, and it is load-bearing (DSP analysis).** `round` is
   *wrong* for a sequencer/table, not just a style choice. Indexing a length-4 array
   with a phasor `p ∈ [0,1)` via `seq[p * 4]`:
   - With `floor`, the interval splits into **4 equal steps**: `[0,1) [1,2) [2,3)
     [3,4)` — each step has equal duration.
   - With `round`, the first and last steps are **half-width**: `[0,0.5) [0.5,1.5)
     [1.5,2.5) [2.5,3.5) [3.5,4)` (the edges clamp to half a step). The sequence
     speeds up at the boundaries — a classic digital-sequencer bug.
   So `floor` is the only correct rule; the user can `round()` the index expression
   themselves when they genuinely want nearest-neighbour.
3. **Interpolated lookup (later).** Stepped `arr[i]` gives zipper noise on smooth-LFO
   use. A `table_lin(arr, pos)` builtin (lerp between neighbours, `pos` in
   `0..len-1`) solves the wavetable case. Ship stepped indexing first; add
   `table_lin` in a second pass. **Note:** like all array-taking functions it is a
   *dedicated opcode*, not a normal builtin — see below.

### Array-taking functions must be dedicated opcodes (not `Op::Call`)

A consequence of "arrays are not first-class values": an array cannot be pushed onto
the operand stack, so a function that *takes* an array (`table_lin(arr, pos)`,
`scale_snap(val, arr)`) **cannot** be compiled as a normal `Op::Call(Builtin)` that
reads its args off the stack — there is no array value to read. Each such function
compiles to its **own opcode** carrying the array's pool location:

- `Op::TableLin { base: u16, len: u16 }` — pops `pos`, reads from `pool[base..base+len]`.
- `Op::ScaleSnap { base: u16, len: u16 }` — pops `val`, reads the scale table.

The opcode pops only the *numeric* argument(s) from the stack and uses the baked
`base`/`len` to index the constant pool directly. The parser must recognise the array
argument at the call site and the codegen must emit the specialised opcode (resolving
the array name → `base`/`len` at compile time). General rule: **any builtin with an
array parameter is a specialised opcode**, never a stack-argument call.

### Symbol table must track array vs scalar kind

Because an array name is not a valid expression on its own, the `synth_script` symbol
table must carry the **kind** of each binding, not just a register index — e.g.
`Symbol::Scalar(reg_idx)` vs `Symbol::Array { base, len }`. The type checker then
rejects misuse at compile time with clear diagnostics (all errors in one pass, per the
existing contract):

- `out = my_array + 1` → *"arrays cannot be used directly in arithmetic; index it
  with `my_array[i]`"*.
- `table_lin(my_scalar, pos)` → *"expected an array name, found scalar variable
  `my_scalar`"*.
- `my_array[i]` where `my_array` is scalar → *"`my_array` is not an array"*.

This is standard symbol-kind tracking and folds cleanly into the Step 1 parser/checker
work — it is the same mechanism that already reserves predefined identifiers.

### Cap model

Add a constant-pool budget (e.g. total array storage ≤ 256 `f32`, max N arrays) next
to the existing caps. A script exceeding it is a **compile error** (never truncated),
same contract as the instruction cap.

### Touch points

`crates/synth_script`: `lexer.rs` (`[ ] ,` already tokenized for calls — reuse),
`ast.rs` (an `ArrayDecl` header node + an `Index` expr node), `parser.rs`
(`arr name = [ … ]` in the header grammar; `name[expr]` postfix), `compile.rs`
(emit the constant pool + `IndexConst`; `len()` constant-fold; pool-size cap),
`fmt.rs` (canonical array formatting — see below), `symbols.rs` (reserve `arr`,
`len`, `table_lin`). `crates/synth_core/src/script`: `bytecode.rs` (the opcode +
pool field on `BoundScript`), `eval.rs` (execute it). Plus `docs/yams.md`.

**Formatter rules** (one canonical form, per the `yamsfmt` contract): `arr` decls
go in the header block with `src`; `[a, b, c]` with one space after each comma, no
space inside brackets; element numbers canonicalize like any literal (`1.0` → `1`);
long arrays stay on one line (no auto-wrap) unless we add a width rule later.

---

## 2. Scale-aware pitch quantization

**The gap.** `quantize(x, step)` is *linear* step quantization. There is no way to
snap a modulation value to a **musical scale** (major/minor/etc.) — which is what
makes pitch modulation usable instead of a continuous glissando. Today a random or
LFO source driving pitch slides between notes; you want it to land on scale degrees.

Two ways, not mutually exclusive:

- **Array-based (falls out of §1).** `arr maj = [0, 2, 4, 5, 7, 9, 11]` plus an
  octave-wrap idiom: pick the degree with `maj[i % len(maj)]` and add
  `12 * floor(i / len(maj))`. Composable the moment arrays exist — document it as a
  recipe.
- **Builtin (nicer).** `scale_snap(semitones, arr)` → nearest member of the scale
  table (in semitone space, octave-aware), so `out = semis(scale_snap(x * 24, maj))`
  is a clean two-step. A builtin avoids the wrap boilerplate and reads better.
  Compiles to the dedicated `Op::ScaleSnap { base, len }` opcode (§1) — the array is
  not a stack value.

**The snap must be octave-aware (algorithm).** Naive nearest-member-in-table is
musically wrong at octave boundaries: for scale `[0,2,4,5,7,9,11]` an input of `11.8`
semitones is numerically closest to `11` (B) but musically closest to `12` (C, next
octave). The opcode must:

1. Split the input `P` into octave `O = floor(P / 12)` and pitch class
   `PC = P - 12*O`.
2. Over every scale member `S`, minimise the distance to `PC` testing **`S`, `S-12`,
   and `S+12`** — so a `PC` just below the octave can snap up to the next octave's
   root and a `PC` just above `0` can snap down. Track the best `(S, octave-shift)`.
3. Result = `12*O + S_best` (carrying the chosen ±12 shift).

This is a small fixed loop over `len` table entries — bounded, branch-light, RT-safe.

**Lean:** ship the array recipe first (zero extra code once §1 lands), add
`scale_snap` as a builtin if the recipe proves clumsy in practice. Pairs naturally
with the existing `semis`/`mtof` musical functions.

---

## 3. Phasor / ramp sync (reset input)

**The gap.** `phasor(rate)` is free-running — it **cannot be phase-aligned to
note-on or to a trigger**. So you can't start a custom LFO in phase with the note,
retrigger a ramp on an event, or sync a built-in LFO to the bar. This is a real
modulation limitation, not sugar.

**Design.** Add an optional reset/sync argument that resets phase to 0 on a rising
edge (reusing the `edge` detector semantics already in the VM):

- `phasor(rate, sync)` — reset the ramp on a rising edge of `sync`.
  `phasor(0.5, gate_on)` aligns to note-on; `phasor(r, edge(bar_phase < prev))`
  aligns to the bar.
- `accum(x, reset)` — same idea for the integrator (today it runs forever with no
  way to zero it). Symmetric.

**Cost correction — the synced forms need TWO state cells, not one.** Edge detection
in a block-based VM requires storing the *previous block's* `sync` value to detect a
rising edge, so a synced `phasor` allocates **two** cells (phase + `prev_sync`), and
likewise `accum(x, reset)` (sum + `prev_reset`). The compiler must account for this in
`state_count` allocation so it never overwrites a neighbouring variable's state — the
non-synced `phasor(rate)` stays at one cell, the synced overload bumps to two. Sketch:

```rust
let sync = stack.pop();
let rate = stack.pop();
let prev_sync = regs.state_get(i + 1);
let mut ph    = regs.state_get(i);

if sync > 0.0 && prev_sync <= 0.0 {
    ph = 0.0;                 // rising edge → reset phase
} else {
    ph += rate * dt;
    ph -= ph.floor();         // wrap 0..1
}

regs.state_set(i, ph);
regs.state_set(i + 1, sync);  // remember trig for next block
stack.push(ph);
```

So this *is* a per-arity state-footprint change the allocator must know about — not
"no new state" as an earlier draft implied. No new opcode family is needed (it is the
same `phasor`/`accum` op with a wider state stride), but the compile-time state
accounting is load-bearing.

**Touch points.** `symbols.rs` (arity), `compile.rs` (the extra arg **and the
2-cell state allocation for the synced overload**), `eval.rs` (reset-on-edge in the
`phasor`/`accum` cases), `docs/yams.md`.

---

## 4. Smooth random LFO

**The gap.** Sources of randomness today: `rand([lo,hi])` (latched, held until
re-triggered) and `white()` (per-block discontinuous noise). Missing: a **continuous,
slowly-wandering random LFO** — one of the most common modulation sources (analog
"random" / sample-and-glide). Today you approximate with `lag(white(), …)`, but the
lag eats amplitude and the rate isn't controllable cleanly.

**Design.** A `rand_smooth(rate)` stateful function: interpolated value noise — latch
a new random target at `rate` Hz and ramp toward it (smoothstep between targets),
giving a band-limited wander with controllable speed and full amplitude. It re-seeds
per voice from `hash(global_seed, voice_index)` exactly like `rand`/`white`, so
simultaneous voices stay decorrelated (stereo width) and retriggers are deterministic.

**Cost — THREE state cells per instance.** Interpolated value noise needs to hold (i)
an internal phase tracking position within the current segment `0..1`, (ii) the
segment's start value, and (iii) its target value. The compiler must allocate three
state cells (and count them against the ≤16-state cap).

**Cold-start bug — must seed the first segment.** `RegisterFile::reset` zeroes all
state cells on note-on, so the naive sketch starts with `phase = start = end = 0` and
the **first segment glides 0 → 0** — the source is silent and flat for the whole first
cycle (up to ~2 s for a 0.5 Hz LFO). Fix in `eval.rs`: detect the all-zero first block
and seed `start`/`end` immediately. (The all-three-exactly-zero sentinel can in
principle collide with a legitimately-random `0.0`, but only if `phase` is *also*
exactly `0.0` in the same block — negligible, and the worst case is one re-seed.)

```rust
let rate = stack.pop();
let mut ph    = regs.state_get(i);
let mut start = regs.state_get(i + 1);
let mut end   = regs.state_get(i + 2);

if ph == 0.0 && start == 0.0 && end == 0.0 {
    start = regs.next_unit();    // seed the first segment so it isn't 0 → 0
    end   = regs.next_unit();
}

ph += rate * dt;
if ph >= 1.0 {
    ph -= ph.floor();
    start = end;
    end   = regs.next_unit();   // new random float in [0,1), per-voice seeded
}

regs.state_set(i, ph);
regs.state_set(i + 1, start);
regs.state_set(i + 2, end);

let t = ph * ph * (3.0 - 2.0 * ph);   // smoothstep
stack.push(start + t * (end - start));
```

**Touch points.** New stateful entry: `symbols.rs` (catalog), `compile.rs` (**3-cell**
state allocation + cap accounting), `eval.rs` (the value-noise update), `docs/yams.md`.

---

## 5. Parameter sources (`flt-1.cutoff`) — already tracked

**The gap.** A `src` binding can reference a module **output port** (`lfo-1.out`) but
not a **parameter value** (`flt-1.cutoff`). Per `docs/yams.md`, such an address
parses and installs but resolves to a constant `0` — the resolver only reads output
ports. This is the clearest "should just exist": it lets a script react to what the
user/automation does to a knob.

Already tracked as **TODO §3.5 / `ScriptInput::ModuleParam`**. Listed here for
completeness and ranking. Higher cost than the others because it needs the cross-crate
parameter resolver and a **descriptor-range → normalized mapping**. Modules work in
*internal units* (e.g. 1000 Hz cutoff, −12 dB gain) but YAMS sources arrive normalized
(`0..1` or `-1..1`); so when `resolve_source` reads a parameter it must map the raw
value through the module's descriptor scale and normalize **at the resolve boundary**,
exactly as the Mod Matrix does for its other sources — otherwise a script mixing a
`cutoff` source with an LFO would be comparing 1000 against ±1. The
`yams-realtime-plan.md` Phase 1 already specifies the shape: resolve the descriptor
**once at bind time** into a fixed accessor (`graph.get_param(module_id, &param)`),
never a per-block string scan or descriptor lookup on the audio thread. Build it there
or here, but build it once.

The descriptor machinery to normalize against already exists in `synth_core`: a
parameter's `ParameterDescriptor` carries both its `ValueRange` and its
`ResponseCurve` (linear/exponential/etc.), which is exactly what maps a raw internal
value into the normalized `0..1`/`-1..1` source space. Resolve that descriptor once at
bind time and cache the normalization (the range + curve, or a precomputed closure)
alongside the accessor, so the per-block path is a direct read + a branchless
normalize — no descriptor-table lookup on the audio thread.

---

## 6. Ergonomic helpers (composable today, but should be primitives)

Low-cost quality-of-life. None are new capability — each removes a common hand-rolled
idiom and a class of authoring error.

- **Polarity maps** — `unipolar(x)` = `x * 0.5 + 0.5`, `bipolar(x)` = `x * 2 - 1`.
  `docs/yams.md` explicitly warns the author must track each source's polarity
  (`lfo-1.out` is ±1, `env-1.out` is 0..1); named helpers remove the by-hand
  arithmetic and its sign mistakes. Pure stateless, trivial.
- **Transport clock** — `pulse(div)`: a gate that fires once every `div` beats off the
  `beat` context (`floor(beat) % div == 0` edge). Doable by hand now, but a primitive
  is clearer now that transport exists, and pairs with the §3 sync inputs
  (`phasor(r, pulse(4))`).

Both are stateless or near-stateless additions to the function catalog — `symbols.rs`
+ `eval.rs` + `docs/yams.md`, no opcode work.

---

## 7. Out of scope (state explicitly, defer)

- **Mutable arrays / writing at runtime.** Turns state cells into arrays with
  per-voice array state + reset semantics — large complexity, thin use cases. Const
  read-only tables (§1) cover the musical needs.
- **Arrays as first-class values** (passed to functions, returned). Would break the
  single-`f32` type system. Arrays stay index-only.
- **User-defined functions.** Beyond `let`, not worth it for a modulation language.
- **Multi-out / `out.left`,`out.right`.** Owned by `plans/yams-realtime-plan.md`
  (Script module's K output ports, Phase 4 stereo grammar). Not a gap in the
  Mod-Matrix-slot binding context (one slot = one destination).
- **`prev(x)` / `z1(x)` general one-block delay.** Subsumed by the realtime plan's
  Phase 4 `state` declaration + `LoadState`/`StoreState`. Don't build separately.

---

## 8. Suggested build order

Each step keeps `cargo fmt --check && cargo build && cargo clippy --all-targets &&
cargo test` green and updates `docs/yams.md` (per CLAUDE.md). The toolchain is
unit-testable headless (lexer→parser→compile→eval), so each lands with tests before
any GUI eyeballing.

1. **Arrays (§1)** — const pool + `Op::IndexConst` + `len()` + AST/parser support for
   `arr` decls and `name[expr]` + fmt + pool-size cap + tests (round-trip parse/fmt,
   indexing incl. OOB clamp, the step-sequencer example). Foundation the rest builds
   on. **No new serde:** scripts persist as canonical YAMS *text* and recompile on
   load (`docs/yams.md` — source text is the source of truth), so the constant pool is
   rebuilt at compile time; `BoundScript` needs no new `Serialize`/`JsonSchema`. Only
   the `arr` *text* round-trips, which `yamsfmt` already covers.
2. **Stateful sync overloads + smooth random (§3 & §4)** — synced `phasor`/`accum`
   (two state cells) and `rand_smooth` (three state cells), **with the matching
   `state_count` allocation logic in the compiler** (the load-bearing part). Group
   them since they share the multi-cell allocation change.
3. **Ergonomic helpers + table/scale builtins (§6 & §2)** — `unipolar`/`bipolar`/
   `pulse` (plain stateless builtins) and `table_lin`/`scale_snap` (the dedicated
   `Op::TableLin`/`Op::ScaleSnap` opcodes from §1, with the octave-aware snap).
4. **Parameter sources (§5)** — bind-time param resolve + normalization in
   `resolve_source`; coordinate with `yams-realtime-plan.md` Phase 1 so the resolver
   is built once.

Update the bundled **"YAMS Script Demo"** project
(`assets/examples/projects/YAMS Script Demo.json`) with an array step-sequencer slot
so the headline feature ships with a runnable example.
