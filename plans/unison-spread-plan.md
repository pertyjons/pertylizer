# Unison Spread — implementation plan

Per-voice **stereo width** for the voice-allocator's `AllocationMode::Unison`:
a `unison_spread` (0..1) control that pans the unison voices across the stereo
field (voice 0 → left … voice n-1 → right), widening a unison stack. This is the
second half of `plans/TODO.md` §2.4; the **detune** half shipped to `main` as
`268441f9`.

> **Distinct from the per-module unison** on `Oscillator`/`VoiceSynth`/`Fof`,
> which already spreads voices *inside one module*. This spreads whole
> **allocator voices** in the global `Unison` allocation mode.

**Why this one is "the real feature":** detune was pure plumbing (re-add a
field). Spread needs **new per-voice DSP** — voices today carry only a pitch
detune (`set_oscillator_detune`), no stereo position. The research below found
the hook points already exist, so it is bounded, not open-ended.

> **External DSP/Rust review incorporated.** An expert pass validated the math
> (incl. a headroom proof) and added: a hot-path **bypass** when gains are unity
> (Step 0.4), the **oversampled-loop AWE asymmetry** (Step 0.4), and confirmation
> that `#[serde(default)]` alone suffices (Step 4). One suggestion — a dedicated
> `UnisonSpread` newtype — was **declined** (decision 6: it would break parity with
> the per-module `UnisonSpread(NormalizedValue)` param and the detune sibling).

---

## Design decisions (LOCKED — the recommended defaults)

1. **Default spread = 0 (off).** Unlike detune's 10 ct default, spread starts at
   zero, so every existing `Unison` patch is **byte- and sound-identical** until
   the user dials it up. New projects too.
2. **Blend from unity.** At `spread = 0` the per-voice gains are exactly `(1, 1)`
   (no −3 dB drop, no widening); they glide toward the equal-power pan as
   `spread → 1`. Avoids a discontinuous −3 dB jump the instant spread leaves 0.
3. **Computed at allocation** (like detune): a held unison note does not
   re-spread live when the slider moves — it takes effect on the next retrigger.
   Consistent with `set_oscillator_detune`.
4. **Composes with the AWE spatial pan** (the two per-voice pans multiply when
   both are active). Acceptable; perfect constant-power composition is a possible
   later refinement, not a blocker.
5. **Pan law = the canonical `Gain::from_pan` / `math::equal_power_pan`**
   (constant-power sin/cos), **not** the AWE `sqrt((1±pan)/2)` curve.
6. **Type = `NormalizedValue` directly** (0..1) — the `InstrumentParam` variant is
   named `UnisonSpread` but wraps `NormalizedValue`, exactly like the per-module
   `OscillatorParam::UnisonSpread(NormalizedValue)` and the
   `InstrumentParam::VelocityAmpSensitivity(NormalizedValue)` siblings, and
   matching how the detune sibling used `Cents` directly. **Do NOT add a dedicated
   `UnisonSpread` newtype struct** — `NormalizedValue` is already a newtype (the
   no-raw-primitive rule is satisfied), and a wrapper would make this param
   inconsistent with all three of its direct precedents. No `StereoWidth` newtype
   either. (`NormalizedValue::default()` is `MIN` = 0.0, so persistence needs only a
   plain `#[serde(default)]` — see plumbing step 4.)

---

## Architecture findings (verified — file:line)

- **Voices are already stereo.** `Voice::process_audio` writes a stereo pair
  (`crates/synth_engine/src/voice.rs:917`); the instrument sum keeps L/R separate
  (`crates/synth_engine/src/instrument.rs:1503-1507`). So spread is a **gain
  applied at the per-voice mix-down**, not a voice-graph change.
- **A per-voice pan multiply already exists at that exact spot.** The AWE
  spatial-panning block (`instrument.rs:1477-1501`) multiplies
  `temp_left`/`temp_right` by per-voice `(gain_l, gain_r)` *before* summing — but
  it is **gated on `spatial_ctx` (AWE room sim only)** and pans by *note*. Unison
  spread slots in at the same location, keyed on the unison **index**.
- **There are TWO copies of the voice loop** — the normal 1× path
  (`instrument.rs:1386-1508`) and the oversampled path (`~1247-1365`). The
  per-voice gain must be applied in **both**.
- **Canonical pan law:** `Gain::from_pan(BipolarValue) -> (Gain, Gain)`
  (constant-power sin/cos, `crates/synth_core/src/types/amplitude.rs:80`); the
  identical `math::equal_power_pan(f32)`
  (`crates/synth_modules/src/math.rs:122`) is what the per-module unison uses.
- **Template to copy:** `VoiceSpread::derive`
  (`crates/synth_modules/src/voice_common.rs:70-83`):
  `pos = voice/(active-1)*2 - 1; (l, r) = equal_power_pan(pos*spread)`, with
  `VoiceSpread::solo()` centring the `n ≤ 1` case. Oscillator
  (`oscillator.rs:176-196`) shares **one `t`** for both detune and pan — the same
  index→[-1,1] map `unison_voice_detune` already uses
  (`voice_allocator.rs:586-592`).
- **Newtype:** `NormalizedValue` (`crates/synth_core/src/types/normalized.rs`)
  is the standard 0..1 amount/width type (consts `MIN`/`CENTER`/`MAX`); no
  `StereoWidth` newtype exists.

---

## Step 0 — DSP core (the novel part; do first, gated behind the config)

1. **`Voice` gains precomputed spread gains.** Add
   `unison_pan_gains: (f32, f32)` (default `(1.0, 1.0)`) to `Voice`
   (`voice.rs:415`), init in `Voice::new`, plus an RT-safe
   `set_unison_pan_gains(l, r)` setter (no alloc — mirror `set_oscillator_detune`).
2. **`allocate_unison` sets them** (`voice_allocator.rs:486`). Extract a pure free
   fn next to `unison_voice_detune`:
   ```rust
   /// Per-voice (left, right) gains for unison voice `index` of `num_voices`,
   /// given a 0..1 `spread`. Blends from unity (spread 0 → (1,1)) toward the
   /// equal-power pan at full spread; a single voice stays centred (unity).
   fn unison_spread_gains(index: usize, num_voices: usize, spread: f32) -> (f32, f32) {
       if num_voices <= 1 || spread <= 0.0 {
           return (1.0, 1.0);
       }
       let t = index as f32 / (num_voices as f32 - 1.0) * 2.0 - 1.0; // [-1, 1]
       let (gl, gr) = equal_power_pan(t * spread);                   // canonical law
       (1.0 + spread * (gl - 1.0), 1.0 + spread * (gr - 1.0))        // blend-from-unity
   }
   ```
   Call it in the `allocate_unison` loop alongside `unison_voice_detune` and push
   via `set_unison_pan_gains`.
   - *Headroom property (validated):* since `equal_power_pan` returns gains in
     `[0, 1]`, the term `(g_pan − 1)` is ≤ 0, so the blended gain
     `1 + spread·(g_pan − 1)` stays in `[0, 1]` — it can never exceed unity, so a
     spread sweep cannot introduce clipping.
3. **Reset on non-Unison allocation.** Poly/Mono/Legato reuse the same `Voice`
   objects, so reset `unison_pan_gains` to `(1.0, 1.0)` on those paths (cleanest:
   in `note_on_expr`, so any non-unison trigger clears a stale spread after a mode
   switch).
4. **Apply at the mix-down** in **both** voice loops. Read the gains once per block
   and **bypass the per-sample multiply when they're unity** (the default
   `spread = 0` case — most patches), so the hot path is untouched unless spread is
   actually on:
   ```rust
   let (uni_l, uni_r) = voice.unison_pan_gains();
   if uni_l != 1.0 || uni_r != 1.0 {
       for i in 0..sample_count {
           temp_left[i] *= uni_l;
           temp_right[i] *= uni_r;
       }
   }
   ```
   (Exact `!= 1.0` compares are safe — the unity path sets *exactly* `1.0`.)
   - **Normal 1× loop** (`instrument.rs:~1497-1507`): apply just before the
     `self.voice_left[i] += temp_left[i]` sum, right after the AWE spatial-pan block.
     When AWE spatial is also active the two pans compose (multiply) — decision 4.
   - **Oversampled loop** (`instrument.rs:~1247-1365`): **note** — verified that
     this loop does the spatial `write_voice` *capture* but does **not** apply the
     AWE dry-pan multiply (only the 1× loop does). So spread is **not** "mirrored
     beside an AWE block" here; apply it independently, right before the
     `self.os_voice_left[i] += temp_left[i]` sum. (Pre-existing AWE asymmetry between
     the two loops is out of scope — just don't replicate the *absence* of spread.)
5. **Gate behind the config** (`AllocatorConfig.unison_spread`, plumbing step 1) so
   this commit is inert (default 0 → `(1,1)`) until plumbed.

---

## Plumbing vertical (mirrors the detune vertical exactly — one commit each)

1. **Allocator** (`voice_allocator.rs`): `unison_spread: NormalizedValue`
   (default `NormalizedValue::MIN` = 0) on `AllocatorConfig` + `set_unison_spread()`.
2. **Command** (`commands.rs`): `InstrumentParam::UnisonSpread(NormalizedValue)`
   + the exhaustive-match dispatch arm (`synth_engine.rs:~1778`) →
   `set_unison_spread`.
3. **Snapshot** (`shared_state.rs`): `unison_spread` on `InstrumentSnapshot`,
   populated from `allocator_cfg` (`synth_engine.rs:~2822`); update the
   `tests/instrument_profile.rs` constructor.
4. **Persistence** (`patch.rs`): serde field with a **plain `#[serde(default)]`**
   (→ 0.0, so pre-field projects get no spread) on the real struct **and the
   manual-Deserialize `Raw` mirror** + the mapping. Unlike detune (which needed an
   explicit `default_unison_detune` → 10.0), `NormalizedValue::default()` is already
   `MIN` = 0, so no default fn is required. Load into `AllocatorConfig` in
   `install_instrument`
   (explicitly, before `..Default`) + push `InstrumentParam::UnisonSpread`; set in
   `snapshot_to_instrument_state` + `default_instrument_state`; regenerate
   `schemas/project.schema.json` + the golden fixtures (all show 0.0).
5. **GUI** (`gui/egui_backend.rs` + `gui/instrument_rack.rs`): a **second**
   `DragValue` in the patch bar next to the detune one — `0..=100 %` mapping a
   `NormalizedValue` 0..1 (suffix `" %"`), `add_enabled(is_unison, …)`, with its own
   `send_unison_spread` dirty flag + apply arm + `inst_state` sync. (TODO §2.4
   already calls for "two sliders".)
6. **Tests:** mirror the detune tests in `patch.rs` (round-trip + backward-compat
   `missing field → 0.0`), plus a **pure `unison_spread_gains` test** in
   `voice_allocator.rs`:
   - `spread = 0 → (1, 1)` for every `index`;
   - symmetric: voice 0 left-biased (`gl > gr`), voice `n-1` right-biased, and the
     pair mirrors (`gains(0) == swap(gains(n-1))`);
   - `n ≤ 1 → (1, 1)`;
   - centre voice of an odd count → `gl == gr`, reaching `≈ 0.7071` (−3 dB,
     constant-power centre) at `spread = 1.0`;
   - `spread = 1.0` edges (`index 0` / `n-1`) → approach `equal_power_pan(∓1)`
     ≈ `(1, 0)` / `(0, 1)`.
   Add `unison_spread_cents`/`unison_spread` to the `project_load_snapshot` golden
   summary if useful (optional, mirrors the detune addition).

---

## RT-safety

The voice loop is the audio hot path (no alloc, no lock). `unison_pan_gains` is
precomputed at allocation and stored on the `Voice`; the mix-down reads it and
does one extra multiply per sample — no new allocation, no per-sample `sin`/`cos`
(the gains were computed once at note-on). `set_unison_pan_gains` mirrors
`set_oscillator_detune`'s no-alloc contract. Mirror the multiply into the
oversampled loop.

---

## Optional altitude refinement

Detune and spread share the same index→[-1,1] `t` map. If it reads cleaner, fold
them into one `unison_voice_placement(index, num_voices, detune, spread) ->
(Cents, (f32, f32))` rather than two free fns — but only if it genuinely
simplifies `allocate_unison`; two small, separately-tested fns are also fine.

---

## Size estimate

Step 0 (DSP) ≈ one focused commit; the 6 plumbing layers ≈ one commit each — same
cadence and cost as the shipped detune vertical. Because spread defaults to 0
(inert), every step is safe to land incrementally on `main` behind that default.

---

## Progress (branch `feat/unison-spread`)

- [x] **S1 — DSP core.** `AllocatorConfig.unison_spread: NormalizedValue`
  (default `MIN` = 0) + `set_unison_spread()`; pure `unison_spread_gains(index,
  num_voices, spread)` (blend-from-unity, `Gain::from_pan`, guards `n≤1`/`spread≤0`)
  + edge-case unit test; `Voice.unison_pan_gains: (f32,f32)` (all 3 constructors,
  reset to `(1,1)` in `note_on_expr`); `allocate_unison` sets per-voice gains;
  applied (bypassed-when-unity) in **both** the normal and oversampled mix loops.
  Inert until plumbed (default 0). Agent review `[]` (buffer lengths, stale-spread
  chokepoint, RT-safety, math, AWE composition all verified). Green.
- [x] **S2 — Command + dispatch.** `InstrumentParam::UnisonSpread(NormalizedValue)`
  + the engine dispatch arm (`synth_engine.rs:~1781`) → `set_unison_spread`
  (mirrors `UnisonDetune`; exhaustive match requires the arm). Green.
- [x] **S3 — Snapshot.** `unison_spread: NormalizedValue` on `InstrumentSnapshot`
  (shared_state.rs), populated from `allocator_cfg.unison_spread`
  (synth_engine.rs:~2826) + the `instrument_profile.rs` test constructor. Green.
- [x] **S4 — Persistence.** `InstrumentState` (patch.rs) gains `unison_spread:
  NormalizedValue` with a plain `#[serde(default)]` (→ 0.0; `NormalizedValue::
  default()` = MIN, so no default fn) on BOTH the real struct and the
  manual-`Deserialize` `Raw` mirror + the `raw.unison_spread` mapping. Loaded into
  `AllocatorConfig` in `install_instrument` (explicitly, before `..Default`) +
  pushed via `InstrumentParam::UnisonSpread`; set in `snapshot_to_instrument_state`
  + `default_instrument_state`. Added `unison_spread` to the
  `project_load_snapshot` golden summary; regenerated `schemas/project.schema.json`
  + the 14 fixtures (all 0.0). Agent review `[]` (backward-compat default on the
  `Raw` struct + drop-trap both verified). Green.
- [x] **S5 — GUI.** `InstrumentUiState.unison_spread: NormalizedValue` (struct +
  Default + `new()`, default 0). A **second** `DragValue` in the patch bar beside
  detune — `0..=100 %` mapping the 0..1 `NormalizedValue` (`*100` display, `/100`
  write-back), reusing the detune control's `is_unison` for `add_enabled` greying;
  sends `InstrumentParam::UnisonSpread` via a `send_unison_spread` dirty flag +
  apply arm; synced from `inst_state`. Agent review `[]` (percent mapping drift-free,
  wiring/greying/sync verified). Green.
- [x] **S6 — Tests.** Two `patch.rs` serde tests mirroring detune: round-trip
  (set 0.6 → save → load → 0.6) and backward-compat (strip the field → loads as 0).
  (The pure `unison_spread_gains` edge-case test landed in S1; the golden-summary
  round-trip coverage in S4.) Green.

**SPREAD VERTICAL COMPLETE** (S1–S6, branch `feat/unison-spread`): unison stereo
spread is configurable end-to-end — DSP (per-voice pan at the mix-down, both
loops, blend-from-unity, RT-safe) → command → snapshot → persistence (0-default
backward-compat) → GUI slider greyed outside Unison → tests. Defaults to 0, so it
is inert for every existing project until dialled up. Each step agent- or
inline-reviewed. **§2.4 Polyphony settings is now fully done** (detune + spread);
only the separate "expose the whole allocator config via MCP" task remains (TODO
§2.4 step-6 triage).
