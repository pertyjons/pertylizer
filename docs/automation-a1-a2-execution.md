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
- [x] **S5 — Data model.** Added `AutomationTarget::Module { instrument:
  SeqInstrumentId, module_type: ModuleType, instance: u16, param_id: String }` in
  `synth_sequencer/src/automation.rs` (additive). **Design deviation from the ledger's
  `module_id: ModuleId, param: Param`** forced by two hard constraints: (1)
  `AutomationTarget` is a `HashMap` key (`last_automation_values`) so must stay
  `Eq+Hash`, but `Param` holds `f32` (PartialEq only); (2) `ModuleId` lives in
  `synth_engine`, which `synth_sequencer` can't depend on. So module identity is
  positional (`module_type`+`instance`, mirrors `ModuleId`; stable-id deferred per
  decision 2) and the param is its descriptor `type_id` string (stable/unique). S6
  rebuilds the `Param` via `descriptor.id.with_f32(denormalize(value))`. Serde
  round-trip + map-key test; regenerated `schemas/project.schema.json`.
- [x] **S6 — Dispatch.** Added `else if let AutomationTarget::Module { .. }` branch in
  `route_sequencer_events` calling new `Instrument::apply_module_param_override(module_type,
  instance, param_id, normalized)`: rebuilds `ModuleId::new(module_type, instance)`, reads
  the cached descriptor, finds the param by `type_id`, denormalizes via its range, rebuilds
  the concrete `Param` with `descriptor.id.with_f32(value)`, applies via the override path
  (reverted on stop). Fully generic — no per-param arms. RT-safe (cached descriptor, no
  alloc/lock/panic). Dispatch test: Module(Filter,1,"cutoff")→0.0→20 Hz attenuation.
- [x] **S7 — Automatable allowlist.** Added `ParameterDescriptor::is_automatable()` =
  `modulatable && choices.is_none()` (reuses the existing continuous/RT-safe `modulatable`
  flag; excludes choice/enum). Marked structural/sizing float params `modulatable(false)`:
  oscillator `unison` (voice count), euclidean `steps`/`pulses`/`rotation`, turing_machine
  `length` (review found these were `modulatable=true` by default). `modulatable` is read
  nowhere else, so the flips are side-effect-free. Tests: predicate unit test + per-module
  allowlist tests (filter, oscillator, euclidean). **Caveat (doc'd on `is_automatable`):**
  it's descriptor-level *eligibility*, not override coverage — automating an eligible param
  on a non-A1/A2 module is a documented no-op (per locked decision 1) until its override
  lands; S8/S9 should keep this in mind.
- [x] **S8 — GUI picker.** Added a third section to the automation-lane target ComboBox
  (`sequencer/mod.rs`): browses the selected instrument's modules (via
  `PatchEditor::module_ids` + `module_descriptor`), and for each param where
  `is_automatable()`, offers a `AutomationTarget::Module { instrument, module_type,
  instance, param_id }` lane. Deterministic (type,instance) order; labels include the
  instance to disambiguate; lazy separator (no stray header when empty); skips params
  already shown as non-empty lanes. Also extended the existing-lane "foreign instrument"
  badge to treat `Module` lanes like `Instrument` (both carry `instrument`). UI not
  exercised headlessly — relied on type-check/clippy/test gate + review.
- [x] **S9 — MCP.** `build_automation_target` now accepts `"module:<prefix>:<instance>:<param_id>"`
  (e.g. `module:flt:1:cutoff`) → `AutomationTarget::Module`, validated against the allowlist
  (param must exist on the module type's descriptor via `module_factory::get_descriptor` and
  be `is_automatable()`). `automation_target_info` emits the same canonical form, so info↔build
  round-trips. The `"module:"` prefix can't collide with `AutoInstrumentParam` names. Tests:
  parse+validate, instrument-target still works, rejects non-automatable/unknown/bad-arity,
  info→build round-trip. Independent review confirmed prefix↔from_prefix are perfect inverses
  and descriptor `type_id` is consistent across GUI(S8)/MCP/engine(S6) — no bugs.
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
- S5 done — `AutomationTarget::Module` variant added. **Ledger's `module_id: ModuleId, param: Param` was not implementable**: `AutomationTarget` is a HashMap key (needs Eq+Hash) but `Param` holds f32 (PartialEq only), and `ModuleId` is in `synth_engine` (unreachable from `synth_sequencer`). Redesigned to `{ instrument, module_type, instance, param_id: String }` — all Eq/Hash/JsonSchema/Serde-clean; positional module identity + descriptor `type_id` for the param; S6 rebuilds `Param` via `with_f32`+`denormalize`. serde round-trip + map-key test; regenerated `project.schema.json` (only schema touched). Gate green; independent review confirmed `type_id` is documented stable/unique and the design is sound, no bugs. Added `serde_json` dev-dep to synth_sequencer. Note: Module automation events are silently dropped until S6 wires the dispatch.
- S7 done — `ParameterDescriptor::is_automatable()` = `modulatable && choices.is_none()`. Marked structural floats `modulatable(false)`: osc `unison`, euclidean `steps`/`pulses`/`rotation`, turing `length`. 4 tests (predicate + filter/osc/euclidean allowlists). Gate green; independent review found (1) structural over-inclusion in euclidean/turing — FIXED here; (2) `modulatable` is read nowhere else so flips are safe; (3) design risk: `is_automatable` is broader than actual override coverage (osc level/fm_amt, filter drive/morph, modmatrix slot amounts are eligible but no-op) — accepted per locked decision 1 ("default no-op elsewhere, documented") and now doc'd on `is_automatable`; S8/S9 should be aware. NOT gating the allowlist on override-capability (no descriptor-level signal; would duplicate/drift).
- S9 done — MCP: `build_automation_target` parses `module:<prefix>:<instance>:<param_id>` → `AutomationTarget::Module`, validated against the allowlist (`get_descriptor` + `is_automatable`); `automation_target_info` emits the same canonical form (info↔build round-trips). 4 tests. Gate green (fmt/build/clippy/test exit 0); independent review confirmed prefix↔from_prefix perfect inverses (67/67), all error paths sensible, no unvalidated construction path, descriptor `type_id` consistent across GUI/MCP/engine — no bugs.
- S8 done — GUI lane-target picker section 3: browses selected instrument's modules (`PatchEditor::module_ids`/`module_descriptor`) and offers `AutomationTarget::Module` lanes for each `is_automatable()` param; disambiguating labels, deterministic order, lazy separator, foreign-badge extended to Module lanes. Gate green (build/clippy/test exit 0); independent review found no bugs (borrow safety, dedup filter, casts, or-pattern, ordering all verified). UI not exercised headlessly — type-check + review only.
- S6 done — Module dispatch: `route_sequencer_events` gained an `else if let Module` branch → `Instrument::apply_module_param_override` (rebuild ModuleId from module_type+instance, resolve param by descriptor `type_id`, denormalize, `Param::with_f32`, apply via override path). Fully generic, no per-param arms; RT-safe. Dispatch test (Module Filter/1/"cutoff" → attenuation). Gate green (fmt/build/clippy/test exit 0); independent review confirmed RT-safety, with_f32/denormalize round-trip, mutually-exclusive branch flow, 1-based instance match, discriminating test — no bugs. Note: A1 uses first-of-type; S6 uses exact (type,instance) — agrees on fresh graphs, positional by design.
