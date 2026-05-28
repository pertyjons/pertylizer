# Execution ledger — Phases A1 + A2 (autonomous run)

Driver doc for the overnight autonomous run of `docs/note-expression-roadmap.md`
Phases A1 + A2. Each loop iteration: read this ledger, do the **next unchecked
step**, gate, code-review, commit, check the box.

## Locked decisions (do not re-litigate)

1. **Override model: generic override layer.** Add a transient per-param override
   to the module API (`set_param_override(param, value)` + `clear_param_overrides()`),
   default no-op on `PolyModule`. `process()` applies `base + override` (or override
   *replaces* base — see combine rule per step); the base param (set via `set_param`)
   is **never** overwritten by automation. Overrides cleared on transport stop →
   param reverts to base. First cut implements the override for the modules A1/A2
   actually target (Filter, Envelope/ADSR, Amplifier, Oscillator); default no-op
   elsewhere, documented.
2. **Scope: A1 + A2 first cut.** Deferred to documented TODOs (NOT this run):
   offline-render parity for `analyze_*`, mod-matrix-vs-automation combine ordering,
   stable (non-positional) ModuleId identity.
3. **On a red step: keep trying** alternative approaches until green — but *different*
   approaches, not the same one. No broken commits, ever.
4. **Branch `feat/automation-a1-a2`, no push.** Commit each green step.

## Per-step gate (every step, no exceptions)

```
cargo fmt --check
cargo build                  # RUSTFLAGS="-D warnings"
cargo clippy --all-targets
cargo test
```

Then `/code-review --fix` on the step diff → re-run the full gate → only then
`git commit`. One commit per green step, message `A1/A2 Sx: <what>`.

Newtype rules and RT-safety rules from `CLAUDE.md` apply throughout. The override
fan-out and clear run on the audio thread → pre-allocated, lock-free, no panics.

## Step ledger

### Phase A1 + the override layer it needs
- [x] **S1 — Override API on `PolyModule`.** Added `set_param_override(&mut self,
  param: Param)` and `clear_param_overrides(&mut self)`, default no-op (mirrors
  `set_mod_offset`/`clear_mod_offsets`). `Param` carries the value, so the
  `(&Param, f32)` shape was dropped for the cleaner `Param`-by-value. Override =
  absolute *replace* (vs additive `set_mod_offset`). Per-module storage is S2.
- [x] **S2 — Implement override for A1-target modules.** Filter (Cutoff, Resonance),
  Envelope (Attack/Decay/Sustain/Release), Amplifier (Level/Pan), Oscillator (Detune,
  PulseWidth — the pitch/PWM-relevant continuous params; base Frequency is note-driven).
  Per-module `Option<T>` override fields; `process()` reads `override.unwrap_or(base)`,
  mod-matrix offsets still apply additively on top. Override setters clamp like
  `set_param`; `clear_param_overrides` reverts to base. One behavioral unit test per
  module (effective value under override + base-untouched + revert-on-clear).
- [x] **S3 — Graph + Voice plumbing.** `ModuleGraph::apply_param_override(module_id,
  param)` (value carried in `Param`, per S1) routes to one module; `clear_param_overrides()`
  fans out to all nodes (mirrors `apply_mod_offset`/`clear_mod_offsets`). `Voice` delegates
  to its graph; `Instrument` fans out to template `voice_graph` + every pooled voice.
  Clear hook on transport stop = `SynthEngine::handle_all_notes_off` (instruments +
  modular `module_graph`). RT-safe (map/slice iteration, no alloc/lock/panic). Tests:
  graph routing→silence→revert + unknown-module no-op, voice delegation.
- [x] **S4 — A1 dispatch.** Replaced the `_ => {}` in `route_sequencer_events` for
  FilterCutoff/FilterResonance/Attack/Decay/Sustain/Release. New
  `Instrument::apply_normalized_override(module_type, is_target, build, normalized)`
  resolves the **first module of that type** (BTreeMap order = lowest instance),
  reads the **cached** descriptor via new `ModuleGraph::module_descriptor` (zero-alloc;
  `PolyModule::descriptor()` allocates a Vec, forbidden on the audio thread),
  denormalizes 0..1 through the matched param's descriptor range/curve, then applies
  via the override path. Clear-on-stop is the S3 `handle_all_notes_off` hook. Tests:
  instrument-level draw→sound→override→revert (settled-energy) + dispatch-level
  `route_sequencer_events` FilterCutoff → attenuation.

### Phase A2 — generic `AutomationTarget::Module`
- [ ] **S5 — Data model.** Add `AutomationTarget::Module { instrument:
  SeqInstrumentId, module_id: ModuleId, param: Param }` in
  `synth_sequencer/src/automation.rs` (additive). Serde round-trip test.
- [ ] **S6 — Dispatch.** Route the `Module` target through the same override apply
  path as A1 in the sequencer playback path. Tests.
- [ ] **S7 — Automatable allowlist.** Descriptor-level flag: a param is automatable
  iff continuous *and* RT-safe. Exclude `choice`/enum params (FilterMode, Waveform…)
  and structural/sizing params (mod-matrix `grid_size`, etc.). Tests asserting the
  exclusions.
- [ ] **S8 — GUI picker.** Extend the lane target picker (`sequencer/mod.rs:~3375`)
  to browse modules + params for the selected instrument, filtered to the allowlist.
- [ ] **S9 — MCP.** Accept module targets in `build_automation_target`
  (`mcp_bridge.rs:~5302`) and surface them in `automation_target_info`. Validate
  against the allowlist.
- [ ] **S10 — Smoothing.** Per-param ramp (or per-block interpolation) in the
  override application so control-rate automation of cutoff/volume doesn't zipper.
  Test/measure no discontinuity at block boundaries.

### Cross-cutting (first cut)
- [ ] **S11 — Reference index + Rack badge + delete guard.** One
  `module_id → [lanes]` index (sequencer-side); Rack view (`instrument_rack.rs`,
  `module_panel.rs`) shows an *automated* badge on referenced modules; module delete
  (`patch_editor.rs:624 remove_module`) is guarded (block-with-warning or
  orphan-flag) for referenced modules.
- [ ] **S12 — Wrap-up.** Update `docs/note-expression-roadmap.md` checkboxes +
  Status lines; record deferred items (offline parity, combine order, stable
  ModuleId) as explicit roadmap TODOs; update `docs/history.md` (one/two-sentence
  style). Final full gate.

## Status log (append one line per iteration)

- (run starts here)
- S1 done — override API on `PolyModule` (default no-op). Gate green (fmt/build/clippy/test exit 0), code-review (none). Found: `ParameterDescriptor.modulatable` already encodes the S7 allowlist (choice→false, float→true).
- S2 done — per-module override storage for Filter/Envelope/Amplifier/Oscillator (`Option<T>` fields, `override.unwrap_or(base)` in process, mod-offset still additive on top). 4 behavioral unit tests. Gate green (fmt/build/clippy/test exit 0); independent code-review found no correctness bugs. Decisions: Oscillator targets Detune+PulseWidth (base Frequency is note-driven, left out); other modules keep the default no-op.
- S3 done — Graph/Voice/Instrument override fan-out + transport-stop clear in `handle_all_notes_off`. 3 tests (osc→amp graph: override silences, clear restores; voice delegation; unknown-module no-op). Gate green (fmt/build/clippy/test exit 0); independent review found no correctness bugs. Notes: clear hook is `handle_all_notes_off` (sequencer sends AllNotesOff on stop); the allocator reuses pooled voices, so a future note inherits a template override only on an explicit voice rebuild (clone_structure copies override state) — doc'd on `Instrument::apply_param_override`.
- S4 done — A1 dispatch: `route_sequencer_events` now handles all 6 module-targeted params via `Instrument::apply_normalized_override` (first-of-type module + cached-descriptor denormalize + override apply). Added zero-alloc `ModuleGraph::module_descriptor` (so the audio thread never calls the allocating `descriptor()`). 2 tests (instrument settled-energy revert cycle + dispatch path). Gate green (fmt/build/clippy/test exit 0); independent review confirmed all 6 arms correctly wired + RT-safe, no bugs. Test-debug lesson: comparing filter energy needs warm-up blocks — a cold-start/retune transient initially made a 20 Hz lowpass read *louder* than 1 kHz; 16 warm-up blocks fixed the measurement (not the feature).
