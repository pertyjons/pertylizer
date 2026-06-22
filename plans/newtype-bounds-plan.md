# Plan: single-source parameter bounds via semantic `ValueRange` presets

> From TODO §5.2 (last bullet). Architecture cleanup, not a bug.
>
> **REVISED 2026-06-22 after a senior-DSP review.** The original design (a single
> `const RANGE` per newtype + a `BoundedNewtype` trait + clamping in `new()`) is
> **rejected** — one newtype serves many parameter contexts with different
> ranges, so there is no single correct range for `Hertz`/`Gain`/`Cents`, and
> clamping in `new()` would corrupt intermediate DSP math (FM, filter sweeps).
> The codebase already proves this: `Hertz` carries `clamp_audible` / `clamp_lfo`
> / `clamp_filter` and `Cents` carries `clamp_detune` — per-*context* clamps, not
> a per-*type* one. The revised plan uses **semantic range presets** (named
> `const ValueRange` per context) as the single source of truth, with no
> constructor or DSP-path changes.
>
> **Endorsed 2026-06-22** by a second senior-DSP review ("proceed immediately"),
> with three refinements folded in: align each preset's `default` with the param
> variant's default (§3.3), name presets specifically enough to disambiguate
> multiple controls of the same unit (§1), and add an optional global drift-lint
> (§3.5).

## 0. Why

The same numeric bound is currently restated in up to **three** places that can
drift. For the oscillator `detune` (±100 ¢):

1. the descriptor — `ParameterDescriptor` `.range(-100.0, 100.0)`
   (`crates/synth_modules/src/oscillator.rs` ~`:354`),
2. the `with_f32` apply clamp — `Cents::new(value.clamp(-100.0, 100.0))`
   (`crates/synth_core/src/params/oscillators.rs:687`),
3. the type's own clamp method — `Cents::clamp_detune()`
   (used at `oscillator.rs:630` / `:771`).

Three literals for one bound. The same pattern exists for `Hertz` (descriptor
ranges vs. `clamp_audible`/`clamp_lfo`/`clamp_filter`), `Gain`, etc. Goal: **one
named constant per (type, context)** that the descriptor, the `with_f32` clamp,
and the type's clamp method all reference — so GUI, MCP validation, and internal
conversion can never disagree.

## 1. The shape — semantic `ValueRange` presets (Option C)

`ValueRange` (`crates/synth_core/src/types/range.rs`) already has what we need:
`const fn new(min, max, default)`, public `min`/`max`/`default`, and `clamp(v)`.
Define presets as associated `const`s on each newtype, **named by context**:

```rust
// crates/synth_core/src/types/frequency.rs
impl Hertz {
    pub const OSC_RANGE:    ValueRange = ValueRange::new(1.0,  20_000.0, 440.0);
    pub const FILTER_RANGE: ValueRange = ValueRange::new(20.0, 20_000.0, 1_000.0);
    pub const LFO_RANGE:    ValueRange = ValueRange::new(0.01, 50.0,     1.0);
}

// crates/synth_core/src/types/pitch.rs
impl Cents {
    pub const DETUNE_RANGE:        ValueRange = ValueRange::new(-100.0, 100.0, 0.0);
    pub const UNISON_DETUNE_RANGE: ValueRange = ValueRange::new(0.0,    100.0, 10.0);
}
```

(Exact ranges/defaults must be **read off the existing descriptors/clamps**, not
invented — the values above are illustrative; the migration copies the current
numbers verbatim so behavior is unchanged.)

**Naming:** name a preset specifically enough that it can't be mistaken for a
different control of the same unit. If only one detune control exists,
`Cents::DETUNE_RANGE` is fine; if a coarse (±1200 ¢) and a fine (±100 ¢) detune
both exist, name them by their limit (`DETUNE_FINE_RANGE` / `DETUNE_COARSE_RANGE`)
so a future call site can't grab the wrong one.

**No `BoundedNewtype` trait, no `const RANGE`, no clamping in `new()`.** Generic
validation that needs "the bound" takes a `&ValueRange` argument (or reads
`descriptor.range`) — it never assumes one-range-per-type. `new()` stays an
unclamped `const fn`, so FM/filter intermediate math is untouched and the
real-time audio path gains zero branches.

## 2. Scope decisions (resolve before coding)

1. **Which (type, context) presets exist?** Enumerate from the *current* code, not
   from first principles — every place a `ValueRange`/`.range()`/`.clamp(min,max)`
   literal or a `clamp_*` method appears. Candidates: `Hertz` osc/filter/LFO (the
   three `clamp_*` already imply these), `Cents` detune/unison, `Gain`
   channel-level/boost, `Semitones` transpose. Only create a preset where a real
   duplication exists today.
2. **Where each preset lives.** On the newtype it bounds (`Hertz` in
   `frequency.rs`, `Cents` in `pitch.rs`, …) so it sits beside the `clamp_*`
   method it will back.
3. **`clamp_*` methods reference the preset.** Re-express existing
   `Cents::clamp_detune` / `Hertz::clamp_lfo` etc. as
   `Self::new(SELF::X_RANGE.clamp(self.as_f32()))` so the method and the constant
   can't diverge. (Keeps the ergonomic method; removes the second literal.)

Explicitly **out of scope** (the rejected Option A): clamping in `new()`, a
per-type single range, the `BoundedNewtype` trait.

## 3. Migration steps (one (type,context) per commit)

1. Add the `const ValueRange` preset(s) for one newtype, copying the *exact*
   current min/max/default.
2. Point the three sites at it:
   - descriptor → `.value_range(Cents::DETUNE_RANGE)` (the builder already exists,
     `module_traits.rs:681` — no new extension method needed),
   - `with_f32` → `Cents::new(Cents::DETUNE_RANGE.clamp(value))`,
   - the `clamp_*` method → `Self::new(Self::DETUNE_RANGE.clamp(self.as_f32()))`.
3. **Drift-guard test:** assert the live descriptor's `.range` equals the preset,
   so descriptor and clamp can never disagree:
   ```rust
   assert_eq!(osc.find_parameter("detune").unwrap().range, Cents::DETUNE_RANGE);
   ```
   plus a clamp round-trip (under/at/over min & max).
   - **Also assert the two defaults agree.** A descriptor carries a default in
     *two* places: the `Param` id variant (`Detune(Cents::ZERO)` → 0.0) and
     `range.default` (what `default_value()` actually returns —
     `module_traits.rs:799`). `.value_range(preset)` overwrites `range.default`
     with the preset's, so the preset's `default` must match the variant's value
     or the system has two conflicting "default" states. Assert
     `preset.default == <variant>.as_f32()` in the same test (verified: e.g.
     `Cents::ZERO` for `Detune`; `10.0` for `UnisonDetune`).

**Proof of concept = `Detune`** (the triplicated case in §0): one newtype, one
commit, three call sites collapsed to one constant. If it reads cleanly, repeat
for the other presets; if not, stop — nothing else depends on it.

### 3.5 Optional: a global drift-lint (stretch)
A single test that walks **every** registered module's descriptors and, for each
param whose unit is a preset-backed newtype, asserts its `.range` is one of the
approved presets (not a raw literal) — catching a future dev who hand-writes
`.range(-100.0, 100.0)` instead of reusing `Cents::DETUNE_RANGE`. Valuable, but
defer until several presets exist, and accept it needs an **allow-list of
legitimate one-offs** (not every `Hertz`/`Cents` param maps to a shared preset —
some genuinely have a unique range), or it will flag false positives. Per-param
assertions (§3.3) come first; this is the belt-and-suspenders layer.

## 4. Risks

Much lower than the original Option A:
- **No behavior change** — presets copy the existing literals verbatim; `new()`
  and the DSP path are untouched (the big Option-A risk — clamping intermediate
  FM/filter math — is gone by construction).
- The only real risk is fat-fingering a copied min/max/default; the
  descriptor==preset assertion + clamp round-trip catch that.
- Cross-cutting only in breadth (many descriptors), not depth — and it's
  commit-per-preset, so each step is small and independently verifiable.

## 5. Recommendation

Still best done **when the schema/validation unification that consumes these
presets is being built** — standalone it's maintainability-only. But it is now
low-risk and incremental: ship the `Detune` proof-of-concept first (it removes a
genuine three-way drift hazard), then extend preset-by-preset as adjacent code is
touched, rather than as one big sweep.
