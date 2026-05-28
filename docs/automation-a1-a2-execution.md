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
- [ ] **S2 — Implement override for A1-target modules.** Filter (Cutoff, Resonance),
  Envelope (Attack/Decay/Sustain/Release), Amplifier (Level/Pan), Oscillator (the
  pitch/PWM-relevant params). `process()` reads base+override. Unit tests per module.
- [ ] **S3 — Graph + Voice plumbing.** `Graph::apply_param_override(module_id,
  param, value)` / `clear_param_overrides()`, fanned out to every live voice; clear
  hook on transport stop. RT-safe. Tests.
- [ ] **S4 — A1 dispatch.** Replace the `_ => {}` in the instrument `Parameter`
  dispatch (`synth_engine.rs:~2660`) for FilterCutoff/FilterResonance/Attack/Decay/
  Sustain/Release: resolve to the instrument's filter/envelope module (convention:
  **first module of that type in the graph**, documented next to the dispatch),
  denormalize `NormalizedValue 0..1` via descriptor ranges, apply via the override
  path. Clear on stop. Tests covering a real draw → sound → stop → revert cycle.

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
