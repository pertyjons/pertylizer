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
6. **Newtype = `UnisonSpread(NormalizedValue)`** (0..1), matching the per-module
   `OscillatorParam::UnisonSpread`. No new `StereoWidth` newtype.

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
3. **Reset on non-Unison allocation.** Poly/Mono/Legato reuse the same `Voice`
   objects, so reset `unison_pan_gains` to `(1.0, 1.0)` on those paths (cleanest:
   in `note_on_expr`, so any non-unison trigger clears a stale spread after a mode
   switch).
4. **Apply at the mix-down** — `temp_left[i] *= gl; temp_right[i] *= gr;` beside
   the AWE block, in **both** the normal (`instrument.rs:~1503`) and oversampled
   (`~1360`) loops. Read the gains once per block from `voice.unison_pan_gains()`.
   When AWE spatial is active the two pans compose (multiply) — decision 4.
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
4. **Persistence** (`patch.rs`): serde field with `#[serde(default)]` (→ 0.0, so
   pre-field projects get no spread) on the real struct **and the manual-Deserialize
   `Raw` mirror** + the mapping; load into `AllocatorConfig` in `install_instrument`
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
   - centre voice of an odd count → `gl == gr`.
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
