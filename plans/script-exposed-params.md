# Script-exposed parameters (user knobs on `script` / `audio_script`)

Status: PROPOSED

Origin: external architecture review (§3), evaluated against the code
2026-07-12. This is the strongest of the review's points — a genuine gap, not
already covered elsewhere.

## 1. Current state (verified)

Both scripting modules deliberately expose **zero** numeric parameters:

- `synth_modules/src/script_module.rs:120` — `ScriptModule` (block-rate YAMS,
  `out1`..`out8` output ports): *"No numeric parameters — the scripts are
  installed via `set_script`."* `get_params()` returns `Vec::new()`.
- `synth_modules/src/audio_script.rs:203` — `AudioScriptModule` (audio-rate,
  stereo `in`/`out`): same, *"No numeric parameters — the program is installed
  via `set_script`."*

A YAMS program reads **bound sources** — any module output/param or macro,
resolved address-based like a mod-matrix source (`synth_core/src/script/bound.rs`)
— but it cannot **declare** a value of its own. So a user who writes a
waveshaper in `audio_script` and wants a "Drive" knob has:

- no knob in the module UI (the module has no params to render),
- no `DestAddr` to point a mod-matrix routing at (`scr-1.drive` doesn't exist),
- no way to automate it from the sequencer.

The only way to vary a script's behaviour today is to route an **existing**
module output/macro into it as a source. There is a related known-deferred item
in the YAMS audio-script work: `ScriptInput::ModuleParam` param-sources resolve
to `0.0`.

`DestAddr` (`synth_core/src/params/mod_matrix.rs:504`) is already
`(module_type, instance, param)` and can target *any* modulatable param on any
module — so the moment a script module actually **has** a param named `drive`,
the mod matrix can already address `scr-1.drive`. The missing piece is letting a
script declare that param and having the module surface it.

## 2. Goal

Let a YAMS script declare named parameters that:

1. **Render as knobs** in the module's node UI (with default / range / unit).
2. **Persist** in the patch (save/load) like any other module parameter.
3. Are **readable inside the script** by name (`out = in * drive`).
4. Become **mod-matrix destinations** (`scr-1.drive`) and **automation lanes**,
   so an LFO/MSEG/sequencer lane can drive the user's knob.

Target both `ScriptModule` and `AudioScriptModule` (shared mechanism).

## 3. Design

### A. Declaration syntax (YAMS / `synth_script`)

Add a top-of-program declaration, e.g.:

```
param drive = 0.5        # default 0.5, unit-less 0..1
param cutoff = 1000 [20, 20000] "Hz"   # default, [min,max], unit label
out = tanh(in * (1 + drive * 8))
```

The compiler (`synth_script` crate) collects these into a
`Vec<ScriptParamDecl { name, default, min, max, unit }>` carried on the compiled
program / `BoundScript`. Inside the body, a declared `param` name resolves to a
**register** filled per block from the module's stored param value (same slot
mechanism the block-constant sources already use — see
`synth_core/src/script/eval.rs`). No audio-thread allocation: the register
mapping is fixed at compile time.

### B. Per-instance parameter store on the module

`ScriptModule` / `AudioScriptModule` gain a small param store:

```rust
struct ScriptParams {
    decls: Vec<ScriptParamDecl>,   // from the installed BoundScript
    values: Vec<f32>,              // current knob values, index-aligned
}
```

- `descriptor()` appends one `ParamDescriptor` per declared param (name, label,
  default, range, unit) **derived from the installed script** — so the params are
  *per-instance*, unlike the static 8 output ports.
- `set_param` / `get_param` / `get_params` read and write `values` by the
  param's interned id. Standard save/load already serialises a module's params
  into its `parameters` map, so persistence comes for free once these are real
  params.
- `set_script` refreshes `decls`, preserving any existing value whose name
  survives (so editing a script doesn't reset unrelated knobs).

### C. Mod-matrix destination + automation

- The mod-matrix resolver walks a module's modulatable params to apply an offset
  (`PolyModule::set_mod_offset`, cf. `oscillator.rs:807` and the generic
  `ParamModOffsets` store). Give the script modules a generic
  `set_mod_offset(name, value)` that adds the offset into the effective value of
  the matching declared param (base value + accumulated offset, cleared each
  block like every other module).
- `DestAddr::parse("scr-1.drive")` already works structurally; add the script
  params to whatever enumerates a module's modulatable targets (the descriptor
  param list feeds this) so the picker and `get_instrument_automation_targets`
  show them.

### D. GUI

The node view renders module params generically from the descriptor, so
declared params should appear automatically once (B) is done. Verify the
ƒx/script editor popup (`7dbbd75`, YAMS expression editor) shows a live list of
the params the current script declares, and that the knobs sit on the node
faceplate.

## 4. Real-time safety

Param values are **block-constant**: resolved into the script's source/param
registers once per block (off the hot inner loop), exactly like macros and slow
params today. Mod-offsets accumulate into a pre-allocated store and are cleared
per block. No allocation, no locking in `process()`.

## 5. Files to touch

- `crates/synth_script/` — grammar + compiler: parse `param` decls, carry
  `ScriptParamDecl`s on the compiled program, map declared names to registers.
- `crates/synth_core/src/script/{bound.rs,eval.rs}` — plumb declared params into
  the eval context / register file.
- `crates/synth_modules/src/script_module.rs` and `audio_script.rs` — param
  store, dynamic `descriptor()` params, `set/get_param`, `set_mod_offset`.
- `crates/synth_engine/src/voice.rs` — fill script param registers per block
  (beside `resolve_script_sources`); resolve `scr-N.<param>` mod destinations.
- MCP: ensure `get_module_info` / automation-target listing surface the dynamic
  params; `set_parameter` already dispatches by name.
- GUI: confirm node faceplate + script editor render the declared knobs.

## 6. Open questions

- **Per-instance descriptors.** Most modules have params fixed by *type*; here
  they vary by the installed *script*. Confirm the MCP/GUI param enumeration and
  `rebuild_instrument_preserve_automation` cope with a module whose param set
  changes when its script changes (automation bound to a param that a script
  edit removed must degrade gracefully).
- **Bipolar vs unipolar / unit metadata.** How much of `[min,max] "unit"` to
  support in v1 — start with `default` + optional `[min,max]`, unit-less.
- **Audio-script `first_sample`.** Declared params are block-constant; document
  that they update per block, not per sample (fine for knobs, not for audio-rate
  cross-mod — use a real input port for that).

## 7. Exit gate

- A YAMS script with `param drive = 0.5` shows a **Drive** knob on the node.
- Turning the knob changes the sound; the value saves and reloads with the patch.
- A mod-matrix routing `lfo-1.out → scr-1.drive` modulates it; an automation
  lane can target it.
- Both `script` and `audio_script` support it via the same mechanism.
- Editing the script to rename/remove a param preserves surviving knob values
  and drops orphaned automation without panicking.
- Workspace green (`build` / `clippy --all-targets` / `test` / `fmt --check`).
