# Plan: uniform machine-readable bounds on `synth_core` newtypes

> From TODO §5.2 (last bullet). The TODO itself rates this **"larger,
> cross-cutting refactor — plan separately"** — this is that separate plan.
> Architecture cleanup, not a bug.

## 0. Why

Newtype clamping/bounds are inconsistent today:

- `NormalizedValue` / `BipolarValue` / `Velocity` clamp in `new()`; `Phase` wraps
  (`rem_euclid`).
- `Hertz` / `Gain` / `Cents` / `Semitones` are `const fn new` with **no clamp**.
- There is **no uniform `const RANGE: ValueRange`** on any newtype — only ad-hoc
  `MIN`/`MAX` on a few.

Consequences:
1. A shared "spec → (schema | validation)" abstraction can only read bounds from
   the per-module `ParameterDescriptor`, never from the type itself.
2. `Param::with_f32` (`crates/synth_core/src/params/mod.rs:1070`, dispatching to
   per-module impls) re-clamps ad hoc and sometimes **hardcodes a range that
   duplicates the descriptor** — e.g. `Detune::with_f32`'s `-100..100` clamp
   (`crates/synth_core/src/params/oscillators.rs:530`) hand-mirrors the descriptor
   range at `crates/synth_modules/src/oscillator.rs:354`. Two sources of truth that
   can drift.

Goal: bounds live **on the type**, and descriptors / `with_f32` derive from them —
one source of truth.

## 1. The shape — `BoundedNewtype` (or `const RANGE`)

Two candidate designs; decide before coding:

### Option A — a `BoundedNewtype` trait
```rust
pub trait BoundedNewtype: Copy {
    const RANGE: ValueRange;        // min / max / default (+ maybe curve)
    fn clamped(raw: f32) -> Self;   // new(raw.clamp(RANGE.min, RANGE.max))
}
```
- Each newtype implements it once; `new()` (or a `clamped` ctor) routes through
  `RANGE`. Descriptors can read `T::RANGE` instead of restating literals.

### Option B — a plain `const RANGE` per newtype (no trait)
- Simpler, but no generic code can be written over "any bounded newtype" — every
  consumer names the concrete type. Loses the descriptor-derivation win.

**Lean Option A** — the trait is what unlocks "descriptor derives from the type"
and a generic validate path. But confirm `ValueRange` (synth_core) already carries
min/max/default/curve in the shape needed, or extend it.

## 2. Scope decisions (resolve before coding — this is the risky part)

1. **Which newtypes get bounds?** The naturally-bounded ones: `Hertz` (audio
   range? or unbounded — a frequency can legitimately be >20 kHz for LFO-as-audio
   edge cases), `Gain`, `Cents`, `Semitones`, plus the already-clamping ones
   retrofitted onto the trait. **`Hertz` is the contentious one** — clamping it
   globally could break legitimate out-of-audio uses; it may need to stay
   unbounded or carry a very wide range. Enumerate each newtype and decide
   bounded-vs-not explicitly.
2. **Does `new()` start clamping?** Changing `const fn new` on `Hertz`/`Gain`/etc.
   to clamp is a **behavior change** — every existing construction now clamps.
   Audit call sites for any that rely on unclamped values (e.g. intermediate math
   that briefly exceeds range). Possibly introduce `new` (unclamped, const) +
   `clamped` (bounded) rather than changing `new`.
3. **`const fn` constraint.** `clamp` on f32 is `const`-incompatible in older Rust;
   verify against the toolchain whether `RANGE`-based clamping can stay `const fn`
   or must drop `const`.

## 3. Migration steps (once the shape is decided)

1. Add `ValueRange` `const`s + the trait impl to each chosen newtype in
   `synth_core` (one newtype per commit — small, reviewable).
2. Route the per-module `ParameterDescriptor` ranges to read from `T::RANGE` where
   a descriptor's range duplicates a newtype's (start with the documented
   `Detune` duplication — descriptor at `oscillator.rs:354` ↔ `with_f32` clamp at
   `oscillators.rs:530`).
3. Collapse the ad-hoc `with_f32` re-clamps to the trait's `clamped`.
4. Tests: round-trip each newtype's clamp at min/max/over/under; assert the
   descriptor range equals `T::RANGE` for the migrated params so they can't drift.

## 4. Risks

- **Cross-cutting** — touches `synth_core` newtypes used everywhere; a wrong
  clamp on `new()` is a silent value change across the whole engine. Hence the
  "audit `new()` call sites" gate and the one-newtype-per-commit cadence.
- **`Hertz` clamping** is the highest-risk single decision.
- Net win is maintainability (one source of truth for bounds), not runtime
  behavior — so the bar for "don't change observable behavior" is high; lean on
  the per-newtype round-trip tests and the descriptor==RANGE assertions.

## 5. Recommendation

Do this **only when** the schema/validation unification it enables is actually
being built (the descriptor-derives-from-type payoff). Standalone, it's a large
refactor with mostly latent benefit. Start with the single concrete win — the
`Detune` descriptor/`with_f32` duplication — as a proof of concept (one newtype,
one commit) before committing to the full sweep.
