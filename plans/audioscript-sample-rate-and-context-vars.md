# AudioScript sample-rate correctness + YAMS `sr`/`note_hz` context vars

> **Status: proposed (2026-07-01).** Two related issues, found while using an
> `asc` AudioScript to reproduce a SID combined waveform for spectral A/B matching
> from an external project (`sid-analyzer`). One is a **real pitch bug** in the
> offline render; the other is a **missing static variable** that would let scripts
> be sample-rate-portable instead of hardcoding a ratio. Both are small, RT-safe.

## TL;DR

1. **BUG:** an audio-rate `phasor(freq)` in an `asc` module renders **~8.16 % flat**
   in the offline render path (`render_to_wav` / `analyze_*` / `compare_spectra`).
   Measured: `phasor(110)` → **101.08 Hz** = `110 · 44100/48000`. Root cause:
   `AudioScript::process` builds the eval context from a **stale cached sample rate**
   (defaults to 48 kHz) instead of the per-call `ProcessContext.sample_rate`
   (44.1 kHz in the offline render). Inaudible live *only because* the device rate
   happens to equal the 48 kHz default. **Fix = one line.**
2. **MISSING VAR:** YAMS exposes `cr` (control rate) but has **no audio sample-rate
   variable**, so an audio-rate script literally cannot know what Hz it runs at and
   cannot self-correct or compute SR-independent per-sample increments. Add an `sr`
   context var (the audio sample rate). Optionally add a per-voice `note_hz` so an
   `asc` oscillator doesn't need `mtof(note*127)` + an external phase source.

## Evidence (all measured via the MCP against a running engine)

- Device rate is 48 kHz (`get_engine_status.sample_rate = 48000`); the offline
  render runs at 44.1 kHz (`arrangement_render.rs:337`, `scope.render_sample_rate`).
- `asc` script `out = phasor(110) * 2 - 1`, one held note @ MIDI 45, analysed mid-note:
  `analyze_spectrum.f0_hz = 101.08`. `101.08 / 110 = 0.9189 = 44100 / 48000`.
- Workaround that already works (SR-correct, note-tracked): drive phase from a real
  `osc`'s `phase` output (audio, 0..1 ramp) into `asc.in_l` and read `in` — measured
  `f0 = 109.99 Hz`, no compensation. This is the interim pattern; it should not be
  *required* just to get correct pitch.

## Part 1 — fix the audio-rate sample-rate bug

### Root cause (source-grounded)

- `crates/synth_modules/src/audio_script.rs:163`
  ```rust
  let ctx = EvalContext::audio(self.sample_rate.as_f32());
  ```
  uses the module's cached `self.sample_rate` field, which:
    - defaults to `SampleRate::DVD_QUALITY` = **48000** (`audio_script.rs:79`;
      `crates/synth_core/src/audio/types.rs:19`, and it is the `Default`, `types.rs:50`);
    - is only ever updated by the `set_sample_rate` trait method (`audio_script.rs:231`),
      which the **offline render loader never calls** (no `set_sample_rate` in
      `crates/pertylizer/src/audio/arrangement_render.rs`).
- Peer modules refresh from the live context instead — e.g. `lfo.rs:249`
  (`self.sample_rate = context.sample_rate`), `am_formant.rs:186`, `audio_input.rs:80`.
  `AudioScript` is the outlier that trusts its stale field.
- The VM itself is already correct and rate-agnostic: `EvalContext::audio(sr)` sets
  the eval rate and `dt = 1/control_rate` drives `Op::Phasor` (`eval.rs:39-47, 282`).
  So the fix belongs entirely in the module, not the VM.
- `ProcessContext` already carries the right value: `ProcessContext.sample_rate`
  (`crates/synth_core/src/module_traits.rs:340`), and `process` already holds
  `context` (`audio_script.rs:143`, reads `context.samples` at 145).

### Fix

`audio_script.rs:163` — build the context from the live per-call rate:

```rust
let ctx = EvalContext::audio(context.sample_rate.as_f32());
```

The cached `sample_rate` field + `set_sample_rate` become dead for eval; either
remove the field (keep `set_sample_rate` as a no-op if the trait requires it) or
keep it only for parity. Prefer removing to avoid a second, divergent source of truth.

### Audit the control Script module (`scr`) for the same class of bug

The `scr` module runs at control rate (`control_rate = sample_rate / block_size`).
If it computes that from a cached/stale sample rate rather than the per-call
`ProcessContext.sample_rate`, then control-rate `phasor`/`age`-derived modulation
drifts by the same ratio (transport `beat` would be unaffected — it comes from
`position_beats`, not `dt`). Verify `scr`'s eval-context construction reads
`context.sample_rate`; fix identically if not. (Not yet reproduced — flagged.)

### Test

Add a render-path regression (mirror `crates/synth_modules/src/voice_pitch_harness.rs`):
install `asc` `out = phasor(220) * 2 - 1`, render offline at **44100**, assert the
detected `f0 ≈ 220 Hz` (±1 Hz), and again at 48000. Today the 44.1 kHz case yields
~202 Hz. A `preview_integration`-style test analogous to
`audio_script_program_is_replayed_in_offline_note_render` fits.

## Part 2 — add an `sr` (audio sample-rate) context var

### Why

`cr` (control rate, `sample_rate / block_size`) is exposed
(`symbols.rs:131`, `bound.rs:26 ContextVar::Cr`) but the **audio sample rate is not**.
An audio-rate script needs it to compute exact times/increments and to be portable
across device rates; a control-rate script can derive block size (`sr / cr`). Without
it, the only recourse today is hardcoding a device-specific constant
(the `* 48000/44100` hack), which is wrong at any other rate.

### Change set (small, mirrors the existing `cr` plumbing)

1. **`EvalContext` gains the real sample rate** — `crates/synth_core/src/script/eval.rs:21`.
   `control_rate` alone is insufficient: at control rate `control_rate = sr/block ≠ sr`.
   Add `pub sample_rate: f32`; `EvalContext::audio(sr)` sets both `control_rate = sr`
   and `sample_rate = sr`; `EvalContext::new(cr)` callers pass the true `sr` too
   (thread `context.sample_rate` at both call sites — the `scr` control path and
   `audio_script.rs:163`).
2. **Reserve + resolve the name** — `crates/synth_script/src/symbols.rs`: add
   `("sr", "Audio sample rate (Hz)")` to the context-var table (near `:131`) and
   `"sr" => Context::Sr` to the resolver (near `:171`). `is_reserved` picks it up.
3. **AST/bytecode enum** — `crates/synth_core/src/script/bound.rs`: add
   `ContextVar::Sr` (near `:26`).
4. **Compiler mapping** — `crates/synth_script/src/compile.rs`: map `Context::Sr`
   into the context-var register fill (same path as `Cr`).
5. **Evaluator fill** — where context vars are written each eval, set the `Sr`
   register from `EvalContext.sample_rate`.
6. **Docs** — `docs/yams.md` Context table (add `sr`; clarify `cr` = control rate,
   `sr` = audio sample rate, `sr/cr` = block size) and the `get_yams_reference` output.

Cost: one f32 register, filled once per eval — negligible.

## Part 3 (optional) — per-voice `note_hz`

`asc`-as-oscillator currently needs `mtof(note*127)` (and, until Part 1, a manual SR
fudge) or an external `osc.phase` wire. A `note_hz` context var = the voice's current
playing frequency in Hz **including pitch bend / glide / detune** would be both simpler
and more faithful than `mtof(note*127)` (which sees only the note number). Requires
threading the voice's live pitch (Hz) into the per-voice macro/context fill at eval
time — larger than Parts 1–2, so proposed separately. With it, a SID-style oscillator
is `let ph = phasor(note_hz)` and tracks bend correctly, SR-independently (once Part 1
lands). Until then, the `osc.phase → asc.in` pattern is the recommended workaround.

## Priorities

| Item                                        | Effort                         | Value                                                                          | 
|---------------------------------------------|--------------------------------|--------------------------------------------------------------------------------|
| Part 1 — SR bug fix (`audio_script.rs:163`) | 1 line + test                  | **High** — silently detunes every audio-rate script in offline render/analysis |
| Part 1 — `scr` control-path audit           | small                          | Medium — same class, unverified                                                |
| Part 2 — `sr` context var                   | small, mirrors `cr`            | High — removes the need to hardcode a rate                                     |
| Part 3 — `note_hz` var                      | medium (voice pitch threading) | Medium — ergonomics + bend fidelity                                            |

## Consumer

`sid-analyzer`'s planned SID→Pertylizer sound engine uses `asc` for the timbral core
(combined waveforms, ring-mod, `$D418` digi). See that repo's
`docs/pertylizer-sound-engine-plan.md`; the combined-waveform PoC that surfaced this
went 47 dB (native) → 16 dB (asc) spectral distance vs a reSID reference once the SR
detuning was worked around.
