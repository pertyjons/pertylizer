# Per-oscillator glide (portamento)

Status: PROPOSED

Origin: external architecture review (§5, "Per-Oscillator Portamento"),
evaluated against the code 2026-07-12. Correct observation: glide is currently
voice-global. A legitimate expressive enhancement, medium priority.

## 1. Current state (verified)

Glide is a single global patch setting applied to the **whole voice**:

- `crates/pertylizer/src/patch.rs:634` — `PatchSettings.glide_time: Seconds`
  (one value, `0.0` = off).
- `crates/synth_engine/src/voice.rs:186` — `GlideState` is **per-voice**; the
  voice glides its playing frequency and hands the single glided `note_hz` to
  the sound sources for the block (`ScriptCtx.note_hz`, `voice.rs:79`).
- Every pitched module in the voice receives that one glided pitch (oscillators
  via `set_note` / base frequency; `oscillator.rs:788`). Per-note glide (seeded
  endpoints, `GlideState::start_from`) exists for tracker import, but it is still
  one glide shared by all oscillators.

So you cannot have a sub-bass oscillator glide between notes while an FM bell or
formant oscillator in the same voice jumps distinctly — they all follow the same
portamento.

## 2. Goal

Let each pitched source glide (or not) independently: a per-oscillator glide
time that, when set, overrides the voice-global glide for that oscillator only.

## 3. Design

### Chosen approach: per-oscillator `glide_time` param + one-pole smoothing

Give pitched modules an optional `glide_time` parameter. When non-zero, the
module smooths the **raw (un-glided) target pitch** toward its destination with
its own one-pole/linear ramp, ignoring the voice-level glide. When zero, it
follows the voice `note_hz` as today (so existing patches are unchanged).

This mirrors how `mod_offset_pitch` already lives *inside* the oscillator
(`oscillator.rs:807`, the `"pitch"`/`"detune"`/`"frequency"` offset accumulation)
— pitch shaping per module is an established pattern here.

Mechanism:

1. The voice exposes **two** pitch values to modules that opt in: the current
   glided `note_hz` (default) and the **raw target** note frequency (the glide
   destination). Modules with `glide_time == 0` read `note_hz`; modules with
   `glide_time > 0` read the raw target and run their own smoother.
2. Add a tiny shared helper (a one-pole toward-target ramp) so all pitched
   modules smooth identically — do **not** re-implement per module. Candidate
   home: `synth_dsp` (a `Glide`/`OnePole` primitive) reused by each oscillator.
3. Apply per-module glide in the frequency computation (`actual_frequency` /
   `set_note` path) so it composes with detune, pitch-bend, vibrato and
   mod-offsets already folded there.

### Rejected alternative: a dedicated Glide/Portamento CV module

A "smooth a pitch CV and wire it into an oscillator's pitch input" module is the
modular-purist option and reuses the graph, but oscillators are **note-driven**
(`set_note`), not base-pitch-CV-driven — the only pitch *input* today is the
additive `mod_offset` "pitch" (semitone offset), not a base-pitch source.
Making oscillators accept a base-pitch CV that *replaces* the note pitch is a
much larger change to the pitch model. Defer; the param approach delivers the
feature with far less surface.

## 4. Interaction with voice-level glide & scripts (the key decision)

`note_hz` (voice glide) also feeds scripted oscillators (`phasor(note_hz)`) and
is the value shown to `ScriptCtx`. Keep the voice-level `GlideState` as the
**default/master** so:

- Modules that don't opt in behave exactly as today (no regression, `note_hz`
  unchanged).
- A module with its own `glide_time` **overrides** by reading the raw target and
  smoothing itself — it does **not** double-glide (must not smooth an already
  smoothed value).

Decide and document: does per-oscillator glide fully *replace* the voice glide
for that module, or *add* to it? Recommend **replace** (opt-in module owns its
portamento; voice glide is the fallback for everyone else).

## 5. Scope — which modules

Pitched sources that should get the param (9): `oscillator`, `sub_osc`,
`wavetable_osc`, `math_oscillator`, `additive_osc`, `granular_osc`,
`fractal_osc`, `chaotic_osc`, `sid_oscillator`. Land the shared smoother helper
first, then opt each module in one at a time (one small commit per module, like
the `set_mod_offset` roll-out). `PatchSettings.glide_time` stays as the
voice-level default.

## 6. Real-time safety

The smoother is a scalar one-pole updated per block (or per sample if needed),
using pre-allocated state on the module — no allocation, no lock. `glide_time`
is a normal param (block-constant).

## 7. Files to touch

- `crates/synth_dsp/` — shared one-pole/linear pitch-glide primitive.
- `crates/synth_engine/src/voice.rs` — expose the raw pitch target alongside
  `note_hz`; hand both to modules.
- `crates/synth_modules/src/{oscillator,sub_osc,wavetable_osc,math_oscillator,
  additive_osc,granular_osc,fractal_osc,chaotic_osc,sid_oscillator}.rs` —
  `glide_time` param + smoothing in the frequency path.
- MCP: `glide_time` becomes a normal per-module param (no new tool needed).
- GUI: renders automatically as a module param.

## 8. Open questions

- **Replace vs add** semantics vs the voice glide (recommend replace).
- **Sample- vs block-rate** smoothing — block-rate is likely inaudible-enough for
  portamento and cheaper; start there.
- Should the per-note glide (tracker import, `GlideState::start_from`) also gain
  per-oscillator behaviour, or stay voice-level? Likely voice-level is fine.

## 9. Exit gate

- Two oscillators in one voice: one with `glide_time > 0` audibly portamentos
  between notes while the other jumps instantly.
- Patches with no per-oscillator glide sound identical to before (voice-global
  glide unchanged).
- `glide_time` saves/reloads per module and is settable via MCP.
- Workspace green (`build` / `clippy --all-targets` / `test` / `fmt --check`).
