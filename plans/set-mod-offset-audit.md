# Audit: `set_mod_offset` coverage across voice modules (2026-06-15)

**Trigger:** while fixing oscillator pitch destinations (control-script #12-B) it
emerged that the oscillator *silently dropped* `detune`/`frequency` mod offsets.
This audit checks whether the same gap exists elsewhere.

**Verdict: it is pervasive.** S1.1's headline promise — "the Mod Matrix can target
**any** modulatable param on any module" — is **not actually delivered**. The
address picker (GUI + MCP) offers any descriptor param with `modulatable == true`,
but a routing only does anything if the target module's `set_mod_offset` has a
`match` arm for that param's `type_id`. Otherwise it hits `_ => {}` and **silently
vanishes** — the routing validates, the marker lights, it persists, and it does
nothing.

## The numbers

- **40** voice (`PolyModule`) modules.
- **4** override `set_mod_offset` at all (`amplifier`, `filter`, `lfo`,
  `oscillator`) — the other **36 inherit the no-op default**, so *every* modulatable
  param on them is dropped.
- Even the 4 that override handle only **2–4** targets each and drop the rest.

## Gap table (modulatable float params dropped)

### Partial coverage — override `set_mod_offset` but drop some targets

| Module | Handled | **Dropped (silent)** |
|---|---|---|
| oscillator | pitch, detune, frequency, level | **pulse_width (PWM!)**, fm_amt, x_mod (cross-mod) · *structural: unison, uni_detune, uni_spread, uni_phase* |
| filter | cutoff, resonance | **drive**, env_amt, key_track, cv_amt, morph |
| amplifier | level, pan | master, cv_bipolar |
| lfo | rate, depth | phase |

### Zero coverage — no `set_mod_offset` (ALL params dropped) — sound shapers (high priority)

| Module | Dropped (all modulatable) |
|---|---|
| sub_osc | level |
| noise | level |
| ring_mod | carrier_freq, mix, freq_ratio, key_track |
| wavetable_osc | position, detune, level · *octave* |
| additive_osc | tilt, odd_even, brightness, stretch, randomize, level |
| formant_filter | vowel, cutoff, resonance, mix |
| vocal_tract | tongue, constriction, lips, length, nasality, breathiness, level |
| voice_synth | vowel, formant_shift, breathiness, open_quotient, tilt, vibrato_rate/depth, unison_*, level |
| fof | vowel, formant_shift, skirt, bandwidth, breathiness, vibrato_rate/depth, unison_*, level |
| math_oscillator, chaotic_osc, fractal_osc, granular_osc, padsynth, la_synth, am_formant, fooglers, body_resonance, mechanical_noise, sampler | (all float params — 3–11 each) |

### Zero coverage — modulator / utility modules (judgment needed)

- **Modulator sources** whose params *are* meaningful destinations (modulate an
  envelope's decay, an MSEG rate, etc.): `envelope` (8 floats), `mseg`, `lfo`
  (partial), `envelope_follower`, `drift_generator`, `kinetic_modulator`,
  `turing_machine`, `random_gates`, `euclidean`.
- **Genuinely N/A** (mod-destination meaningless — should likely be
  `.modulatable(false)` so the picker stops offering them): `output`,
  `signal_monitor`, `audio_input`, `mod_matrix`, `pitch_tracker`, `beat_detector`,
  `vector_mixer` / `keyboard_panner` (maybe), and the *structural* params noted
  above (`unison` voice count, `octave` step, `randomize` seed, …).

## Two honest fixes per dropped param

1. **Implement** — add a `set_mod_offset` arm that applies the normalized offset
   to the param's effective value (musical scale per the param's unit).
2. **Mark `.modulatable(false)`** — for params where mod is meaningless/structural,
   so the picker stops offering a dead target.

Every modulatable param must end up in (1) or (2). Right now ~150 params are in
neither — silently dropped.

## Strategic options for the fix (NEEDS A DECISION)

This is **not** a mechanical per-module loop — it's an architecture choice:

- **A — Generic default mechanism (recommended for scale).** Give `PolyModule` a
  default `set_mod_offset` that stores normalized offsets in a per-`type_id` map,
  plus a helper `effective(param) = denormalize_through_curve(base_norm + offset)`
  that modules call when reading a param. Realizes the *already-locked* scaling
  contract (option A: normalized-through-range) generically — **one** implementation
  covers every param. Cost: each module's param reads must route through the helper
  (mechanical but touches every module). Per-param musical exceptions (osc pitch =
  semitones, filter cutoff = ±48) stay as overrides.
- **B — Per-module hand-wiring.** Add `set_mod_offset` arms module-by-module for the
  params that matter; mark the rest non-modulatable. ~40 modules, judgment per param,
  many commits. Pragmatic but slow and inconsistent (every module invents its own
  scale, the exact thing the scaling contract wanted to avoid).
- **C — Triage + shrink.** First mark all non-musical floats `.modulatable(false)`
  (kills the dead picker entries cheaply), then hand-wire only the high-value
  shapers (osc PWM, filter drive, sub/wavetable/ring/noise level, formant/vowel
  params). Smaller than B, leaves a curated set.

**Recommendation: A** (it's the scaling contract finally realized, and the only
option that scales to 40 modules without 40 inconsistent hand-rolled scales). If A
is too big a refactor to take on now, **C** is the pragmatic interim.

---

# Concrete design for Option A — generic `set_mod_offset`

**Goal:** one mechanism so *any* `modulatable` param gets a normalized-through-curve
offset (the locked scaling contract #2 option A), instead of 40 hand-rolled
`set_mod_offset` impls. Musical exceptions (osc pitch = semitones, filter cutoff =
±48) stay as per-module overrides.

## 1. A small RT-safe offset store (`synth_core`)

```rust
/// Per-module accumulator of normalized mod-matrix offsets, one entry per
/// modulatable param. Populated once (off the audio thread) from the module's
/// descriptor — caches each param's range+curve so the apply path never rebuilds
/// the descriptor. All hot-path ops are alloc-free and lock-free (linear scan of
/// a tiny fixed list + f32 math) — RT-safe.
pub struct ParamModOffsets {
    entries: Vec<ParamOffset>, // sized once at populate(); ≤ ~12 per module
}
struct ParamOffset { type_id: String, range: ValueRange, curve: ResponseCurve, offset: f32 }

impl ParamModOffsets {
    /// Off-thread, called by the graph when the module is added (it has the
    /// cached descriptor). Registers every `modulatable` param.
    pub fn populate(&mut self, desc: &ModuleDescriptor) { /* push modulatable params */ }

    pub fn add(&mut self, type_id: &str, value: f32) {        // audio thread, RT-safe
        if let Some(e) = self.entries.iter_mut().find(|e| e.type_id == type_id) { e.offset += value; }
    }
    pub fn clear(&mut self) { for e in &mut self.entries { e.offset = 0.0; } } // each block

    /// Effective native value = denormalize(clamp(normalize(base) + offset)).
    pub fn effective(&self, type_id: &str, base: f32) -> f32 {     // at read sites, RT-safe
        match self.entries.iter().find(|e| e.type_id == type_id) {
            Some(e) if e.offset != 0.0 =>
                e.curve.denormalize((e.curve.normalize(base, e.range) + e.offset).clamp(0.0, 1.0), e.range),
            _ => base,
        }
    }
}
```

## 2. `PolyModule` default impls delegate to the store

```rust
trait PolyModule {
    /// Modules with mod-destinations expose their store (one line). Default None.
    fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> { None }

    fn set_mod_offset(&mut self, target: &str, value: f32) {            // default
        if let Some(o) = self.mod_offsets_mut() { o.add(target, value); }
    }
    fn clear_mod_offsets(&mut self) {                                   // default
        if let Some(o) = self.mod_offsets_mut() { o.clear(); }
    }
}
```

## 3. The graph populates the store (off audio thread, descriptor already cached)

```rust
// graph.rs add_module / add_module_with_id, after building `descriptor`:
if let Some(o) = module.mod_offsets_mut() { o.populate(&descriptor); }
```

## 4. Per-module change (the only real per-module work)

Each module:
1. add field `mod_offsets: ParamModOffsets` (+ `Default` in `new()`),
2. `fn mod_offsets_mut(&mut self) -> Option<&mut ParamModOffsets> { Some(&mut self.mod_offsets) }`,
3. **wrap each modulatable param read** in `process()`:
   `let cutoff = self.mod_offsets.effective("cutoff", base_cutoff);`

Steps 1–2 are mechanical (2 lines). Step 3 is the genuine cost — it touches each
read site, so it requires reading each module's `process()`. But the pattern is
uniform (no per-param scale to invent).

## 5. Musical exceptions coexist

A module that wants exact semitones overrides `set_mod_offset` for *those* targets
and delegates the rest:

```rust
fn set_mod_offset(&mut self, target: &str, value: f32) {
    match target {
        "pitch" | "detune" | "frequency" => { /* semitone math → mod_offset_pitch */ }
        "level" => { /* existing */ }
        other => self.mod_offsets.add(other, value), // generic for pwm, fm_amt, x_mod, …
    }
}
```
So the osc keeps its B1/B2 semitone pitch handling and gains PWM/FM/cross-mod for free.

## 6. RT-safety

- `populate()` (alloc) runs in `add_module` — **off** the audio thread.
- `add` / `clear` / `effective` are linear scans of a ≤12-entry `Vec` + f32 math —
  no alloc, no lock, no panic. Same RT cost class as today's `match target {}`.
- `clamp` keeps it in range (the contract's clamp); the curve handles musicality.

## 7. Migration (incremental, low-risk)

1. Land `ParamModOffsets` + the `PolyModule` defaults + the graph populate hook
   (no behaviour change — no module exposes a store yet). One commit, reviewable.
2. Convert modules **one per commit** (or small groups), each: add the field +
   accessor, wrap reads, drop now-redundant hand-rolled offset fields. The 4
   existing overriders keep their musical arms and delegate the rest.
3. Separately, mark genuinely non-musical floats `.modulatable(false)` (structural:
   `unison`, `octave` step, `randomize` seed, IO modules) so the picker stops
   offering dead targets.

## 8. Honest cost & what it does NOT solve

- ~40 modules still need step-3 read-site wrapping — mechanical but real, several
  commits. (A is *less* code and *consistent* vs B's per-param arms, not zero.)
- It does not auto-decide which params *should* be modulatable — the
  `.modulatable(false)` triage (step 3 of migration) is still a judgment pass.
- A few modules read params in tight inner loops; for those, call `effective()`
  once per block (cache the value), not per sample.
