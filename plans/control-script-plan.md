# Plan: Dynamic Mod Matrix → Control Script

A two-step evolution of patch-level modulation:

- **Step 1 — Dynamic Mod Matrix.** Make the Mod Matrix address **any module's
  modulatable parameter** (destination) and **any module output / macro source**
  (source) — driven by what is actually in the patch, not a hardcoded enum. Fixes the
  artificial ceilings (only 19 destination roles, only 2 LFOs / 2 envelopes as sources).
  Plus the GUI affordances that make a dynamic system legible: per-module
  **source/destination markers** and a visible **macro-source rail**.
- **Step 2 — Control Script.** Add the *compute* layer on top: the per-routing "amount"
  scalar becomes an optional **expression** (cross-source math, conditionals, curves,
  state). Same addressing, same engine seam — only the cell that turns sources into an
  offset gets richer.

The split is deliberate: Step 1 ships a complete, better Mod Matrix on its own; Step 2
is purely additive. After both, the Mod Matrix and the Control Script are the **same
addressing model**, differing only by *scalar vs expression* in the amount cell — which
is why the likely end-state is one unified panel, not two tools.

## Why this shape (the core insight)

Today's `ModMatrix` hides two separable things:

1. **The engine mechanism** (the valuable part): per-voice source cache → scaling →
   **additive offset applied to a module, control-rate, resolved by identity** (not by a
   cable). This is what drives `flt-1.cutoff` with no wire into `cutoff_cv`. We **keep
   and share** it.
2. **The address book** (the limiting part): hardcoded `ModSource` (16) / `ModDestination`
   (19) enums, keyed by module *role-index* (`flt1`, `lfo2`). The menu is **static** —
   identical regardless of patch contents. Only the *binding* is dynamic (`flt1` →
   whichever filter currently fills role 1). A third LFO, `osc1.detune`, `env1.decay`:
   unreachable, because they are not in the enum.

Step 1 replaces the static address book with **dynamic addressing** while keeping the
mechanism. The scalar-multiply cell is untouched in Step 1; Step 2 generalizes it.

| | Mod Matrix today | After Step 1 | After Step 2 |
|---|---|---|---|
| Destinations | 19 hardcoded roles | any modulatable param on any module | same |
| Sources | 16 hardcoded (2 LFOs, 2 envs…) | any module output/param **+ macro registry** | same |
| Amount cell | `amount × source` | `amount × source` (unchanged) | **scalar OR expression** |
| Write channel | additive offset (`set_mod_offset`) | additive offset, widened to all modulatable params | same |
| Execution | per-voice, control-rate, 1-block latency | identical | identical |

## Scope — what this is NOT

Two separable "scripting" domains exist; this plan is the second:

- **Domain A — signal-rate DSP** (`a*x+b*y` on audio buffers): a normal `PolyModule`
  graph node with input/output ports. Port-sandboxed — cannot read other modules'
  params. A separate, independent future feature. **Out of scope here.**
- **Domain B — control-rate parameter modulation** (read params/outputs across modules,
  write params across modules). Needs a privileged cross-module view. **This plan.**

The two coexist already: a patch mixes a cabled signal path (`env → amp.cv`) with the
Mod Matrix's parameter-offset path (`mmx → flt1.cutoff`, no wire).

---

## Status at a glance

### Step 1 — Dynamic Mod Matrix

- [ ] **S1.0** — Architecture lock: per-voice placement, additive-offset write channel,
  1-block read→write latency, **ID-based vs role-based addressing** decision (see Open
  Questions) — LOCK before coding.
- [x] **S1.1** — Widen the write channel: `set_mod_offset(dest_index: u8)` → keyed by a
  stable param identifier (the descriptor `type_id`). graph.rs resolves via
  `module_target_position()`. Behavior-preserving. **SHIPPED `76b84c8`.**
- [x] **S1.1b** — **Dynamic routing list (revised).** Grid dropped from *processing* (Voice
  + `calculate_modulations` iterate all 16 slots; `grid_size` vestigial); GUI panel rewritten
  as a stateless derived list (configured routings + trailing add-row + per-row clear).
  **Data model / descriptor / schema / save format kept UNCHANGED** (the true-`Vec` attempt
  was reverted — see the S1.1b implementation notes). **SHIPPED `506f1eb`.**
- [x] **S1.2** — Dynamic **destination** addressing (engine + persistence + schema).
  **SHIPPED** via load-time migration in sub-steps:
  - `9161cd3` **S1.2a** — `DestAddr` type (`{module_type, instance, param}`, copyable via
    interned `PortName`) + dual-format parse (new `"flt-1.cutoff"` / legacy `"flt1_cutoff"`).
  - `de82668` **S1.2b** — voice reads routings via `PolyModule::mod_routings()` (drops the
    `f32`-index read bottleneck).
  - `19d255f` **S1.2c** — `ModRouting.destination: Option<DestAddr>`, applied via the
    ID-based `graph.apply_mod_offset_addr` (exact `ModuleId`, no fuzzy fallback).
  - `f38cf91` **S1.2d-1** — `SlotDestination` param carries `Option<DestAddr>`
    (behavior-preserving; 19 roles round-trip via the legacy choice path).
  - `33cfea8` **S1.2d-2** — persist dest as a free-form **address string** (dual-format
    `from_param`/`to_param`); schema `slot_dest` enum → string; old projects auto-upgrade,
    **#6 stays parked.**
  A routing can now target **any modulatable param on any module** and round-trip.
  **Remaining to make it user-*creatable*:** the MCP address path (S1.4) and/or the
  deferred GUI picker (S1.5c) — the GUI combo + MCP `set_parameter` still go through the
  19-choice path. Same address approach applies to S1.3 sources.
- [x] **S1.3** — Dynamic **source** addressing + macro registry. **SHIPPED** (mirrors S1.2):
  - `bd9659b` **S1.3a** — `SrcAddr = Macro(MacroSource) | Module{type, instance, name}` +
    `MacroSource` (the six true macros) + dual-format parse (macro id / `"lfo-1.out"` /
    legacy `"lfo1"`, incl. kinetic pos=`out` port, vel/acc=params).
  - `578e2be` **S1.3b** — `ModRouting.source: Option<SrcAddr>`; the Voice resolves each
    source (macro / `get_module_output` / kinetic `get_param`) — a **3rd LFO** is now a
    usable source. Includes **S1.7** (voice snapshot of arbitrary module outputs, RT-safe).
    Removed the dead `source_values`/`gather_mod_source_values` machinery.
  - `7d1f1ce` **S1.3c** — `SlotSource` carries `Option<SrcAddr>`; persist as an address
    string (dual-format `from_param`/`to_param`); schema `slot_source` enum → string.
  - `c35e033` **S1.3d** — MCP `set_parameter` accepts a source address string (unified with
    the dest path). A 3rd LFO is creatable via MCP.
- [x] **S1.7** — Voice snapshot wiring. **SHIPPED in S1.3b** (`578e2be`): the Voice reads
  each referenced source's output port / param directly via the graph, per block, alloc-free.
- [x] **S1.4** — Persistence + MCP (destination side). **SHIPPED `53bc171`.** MCP
  `set_parameter` accepts a free-form address string for a slot dest ('flt-1.cutoff' /
  legacy id / 'none'), parsed via `to_param` — so MCP can create an arbitrary destination.
  Persistence round-trip landed in S1.2d-2. Dangling references are **disable-and-keep**
  (apply_mod_offset_addr no-ops when the addressed module is absent). Source-side MCP rides
  along with S1.3.
**Re-planned 2026-06-14** (the GUI section below was written before S1.2/S1.3 landed
address-based addressing; the GUI never moved to it). The whole patch-editor mod-matrix
seam is still on the **legacy f32-enum-index** representation — both read (`PatchAnalysis`
`patch_editor.rs:256`) and write (`draw_mod_matrix_grid` `patch_editor.rs:5212`,
`param_values: HashMap<String, f32>` can only carry a legacy index, not an address). So
**S1.5c is foundational, not last**: the markers (a/b) read whatever the address picker
writes. Corrected ordering — **S1.5c → S1.5a → S1.5b**.

- [x] **S1.5c** — GUI: **pick-by-addressing** for slot source/destination. **SHIPPED.**
  `slot_addrs: HashMap<String, String>` on `ModulePanelState` mirrors the engine's address
  routings (`param_values: HashMap<String, f32>` can't hold `"flt-1.cutoff"`). `ModAddrCatalog`
  (built per frame from the patch's descriptors, gated on a Mod Matrix being present) drives
  `mod_source_picker`/`mod_dest_picker`: module-instance → param/port, **+ the six macros**.
  Emits the full `Param::ModMatrix(Slot{Source,Destination}(slot, Some(addr)))`; the legacy
  enum combos + `is_mod_choice_available` are gone. `PatchAnalysis::from_panels` now resolves
  `slot_addrs` addresses (absolute instance → `ModuleId`) so the module badge lights for any
  address (the legacy f32-index path missed `lfo-3`/`flt-3`). **Mirror fix:** the MCP-add path
  (`reconcile_with_session` `to_add`) mirrors via `sync_module_params`, not a plain f32 copy —
  the version-gated per-frame sync never re-ran for a freshly-added module, so a Mod Matrix
  built via MCP showed "(none)" forever. Live-verified in-app.
- [x] **S1.5a** — GUI: per-parameter **destination** knob marker. **SHIPPED.** `PatchAnalysis`
  destinations are now `HashMap<ModuleId, HashSet<String>>` (dest param `type_id`s) with
  `mod_role_for_param`; a `mod_role` closure threads through `draw_parameter_grid`/`draw_knobs`
  (mixer passes `|_| None`). Marker = purple `ModRole` glyph (shared with the module-header
  badge via `ModRole::from_flags`/`glyph`); label-group widgets draw an inline coloured icon,
  knobs paint it in the corner (`Knob::mod_marker`, off the label so the grid cell never
  widens). Source/`Both` markers are wired through the renderer but unreachable until S1.5b.
- [x] **S1.5b** *(after a)* — GUI: per-parameter **source** marker + the **macro-source
  rail**. **SHIPPED.** `PatchAnalysis.mod_matrix_sources` is now `HashMap<ModuleId,
  HashSet<String>>` (parallel to destinations), populated from each slot's `SrcAddr::Module`
  `name`; `mod_role_for_param` combines source+dest into the three-state `ModRole::from_flags`
  (`Source`/`Destination`/`Both`), so a source param (`osc-1.detune`) marks its knob while an
  output port (`lfo-3.out`) only rolls up to the module badge. `SrcAddr::Macro` sources land in
  a new `mod_matrix_macros: HashSet<MacroSource>` (added `Hash` to `MacroSource`) driving
  `draw_macro_source_rail`: a fixed foreground strip of all six `MacroSource::ALL` chips, each
  wearing the same purple `ModRole::Source` glyph when wired (gated on a Mod Matrix being
  present). **This closes Step 1's GUI scope — S1.5a/b/c all shipped.** Verify in-app.

The per-module badge already exists (`patch_editor.rs:1834`, three-state `↗`/`↙`/`↔`) and
becomes a roll-up of the per-param data — keep it.

(All S1.5 work is DEFERRED for interactive egui work — not headless-testable; verify in-app.)

- [x] **S1.8** — Address-only cleanup + faithful MCP report. **SHIPPED `406e60b`.**
  `get_mod_matrix_routings` now echoes the stored address (no lossy round-trip
  through the legacy enum — `lfo-3.out` / `osc-1.detune` no longer report as
  `"none"`); dropped the redundant `source_name`/`destination_name` fields and
  bounded the slot decode to `MAX_MOD_MATRIX_SLOTS`. Added load-time positional
  migration (`upgrade_legacy_mod_matrix`, wired into `session.apply_patch`): a
  legacy id (`"env2"`) resolves against the instrument's real instances, so a
  project on non-canonical instances (env-5/env-6) keeps modulating the right
  module instead of a different instrument's env-2. The voice now skips disabled
  routings (it previously applied them). Name kept as **Mod Matrix**.

> **MILESTONE (2026-06-14): Step 1's headless scope is COMPLETE.** Dynamic
> source **and** destination addressing ship end-to-end — engine, persistence,
> schema, MCP. A routing can read any module output / macro (incl. a **3rd LFO**)
> and target any modulatable param on any module; old projects auto-upgrade. The
> GUI now picks **by address** (S1.5c) and shows the topology: per-knob
> source/destination markers (S1.5a/b) + the macro-source rail (S1.5b).
> **Step 1 is now COMPLETE — GUI included.** Step 2 (Control Script /
> expressions) is the next build phase.

**S1.1 is shippable alone** — it lifts the destination ceiling (any modulatable param)
with no UI change. S1.2/S1.3 deliver the full dynamic addressing in engine/persistence/MCP.
The GUI markers (S1.5a/b) were originally expected to ride along with S1.2/S1.3, but the
patch editor never moved off the legacy enum — so they are now a distinct phase gated on the
address picker (**S1.5c first**, see the re-planned checklist + GUI section above).

### Step 2 — Control Script (the compute layer)

The language is **YAMS** (*Yet Another Modulation Script*). Grammar spiked in
[`yams-grammar.md`](yams-grammar.md): header `src` bindings (each → a source register) +
a body that assigns the normalized offset to `out`; rate/context-agnostic so the future
audio-rate dialect reuses the same grammar. See that doc's Open-questions section for the
locked semantics (eager eval, reserved built-ins, NaN-sanitized state, per-voice PRNG) and
the remaining sign-offs (persistence, caps, diagnostics).

- [x] **S2.1** — `CompiledScript` + control-rate evaluator. **SHIPPED** (merged `54cbcdf`,
  5 reviewed steps `9f1ff82..d852772`). Compile **source → AST → flat bytecode**; the RT
  evaluator is a `for` loop over a pre-allocated voice-local register file (O(1) stack).
  Offline compile, immutable, allocation-free eval, fixed register file, hard instruction
  cap, two-layer NaN sanitize. `synth_script` (non-RT compiler + `yamsfmt`) → `CompiledScript`
  in `synth_core` (RT). **Parser tech DECIDED 2026-06-15:** keep the hand-written
  recursive-descent parser + AST-based `yamsfmt`; the `rowan` lossless-CST option (grammar
  decision #11) is **rejected, not deferred** — it would add the first third-party dependency
  to the deliberately dep-light `synth_script` and a full parser rewrite, for formatter polish
  the current AST output doesn't need.
- [x] **S2.2** — Make the amount cell **scalar-or-expression**: a routing whose amount is
  an expression evaluates the script (reading its bound sources) instead of a single
  multiply. Same addressing, same offset write. **SHIPPED — sub-steps below all done:**
  - [x] **S2.2a** — `synth_core::script::bound`: RT `BoundScript` (`Arc`-shared script +
    resolved `ScriptInput` addresses + canonical text) + `ScriptContext` + `CompiledProgram::
    into_bound` mapping in `synth_script` (macros → `SrcAddr::Macro` so the voice resolves
    them via the existing scalar path; module refs round-trip `SrcAddr::parse`, unknown →
    `Zero`). **SHIPPED `fed674a`.** Headless, unit-tested.
  - [x] **S2.2b-1** — `ModMatrix` stores `scripts: [Option<Arc<BoundScript>>; 16]` **beside**
    `slots` (routing stays `Copy` — decision #4; script never lives on it). `PolyModule` gains
    two additive default methods: `mod_scripts()` (the slice, read by the voice) and
    `set_mod_script(slot, Option<Arc<_>>)` (the off-audio-thread install channel, since the
    `Arc` can't ride the f32 `set_param` path). **SHIPPED `d017ae8`.** Storage only, unit-tested.
  - [x] **S2.2b-2i** — Persistence data model + schema. **SHIPPED `0efdc51`.** `ModuleState`
    gains optional `scripts: BTreeMap<String,String>` (1-based slot key, matching `slot_N_*`),
    separate from the descriptor-driven `parameters`. serde `default` + `skip_serializing_if=
    empty`. **Schema wall was a non-issue**: the descriptor-driven `ModuleState` `oneOf` leaves
    each module object's `additionalProperties` *unset* (defaults true), so a `scripts` key
    validates with **no gen_schemas change** — both schema tests stay green. Round-trip
    unit-tested. No save/load wiring yet (field always empty).
  - [x] **S2.2b-2ii** — Engine **write** channel. **SHIPPED `a219688`.** `EngineCommand::
    SetModScript { instrument_id, module_id, slot, script: Option<Arc<BoundScript>> }` →
    `handle_set_mod_script` (mirrors `handle_set_module_param`: template voice graph + every
    live voice) → `ModuleGraph::set_mod_script` → `module.set_mod_script`. Arc cloned per voice
    (atomic refcount, no deep-copy). Wired through the 3 exhaustive `EngineCommand` matches
    (Debug prints source text; `try_clone` Arc-clones; hub `can_modify_params`). Dispatch
    skips `update_shared_graph` (scripts not in that snapshot — correct). Graph unit-tested.
  - [x] **S2.2b-2iii** — pertylizer **load** wiring. **SHIPPED `c675f41`.** pertylizer now deps
    `synth_script`. `SynthSession::set_mod_script` compiles off-thread (`compile` → `into_bound`
    → `Arc`) and sends `SetModScript`; compile error → `SessionError::ScriptCompile` (all diags)
    before sending. `apply_patch` installs each `ModuleState.scripts` entry after the param loop;
    1-based slot key range-checked to `1..=MAX_MOD_MATRIX_SLOTS`, bad keys + compile errors
    recorded and skipped (disable-and-keep). Unit-tested. A persisted script now reaches the
    engine module — only eval (S2.2c) is left to make it audible.
  - [x] **S2.2b-2iv** — Engine **read** channel + save wiring. **SHIPPED `7e4096d`.**
    `ModuleStateSnapshot.scripts` (1-based key) populated in `update_shared_graph_for_instrument`
    from `mod_scripts()` (`bound.source`); `SetModScript` now calls `update_shared_graph` (the
    2ii skip is obsolete now scripts are in the snapshot — mirrors `SetModuleParameter`);
    `build_patch_from_engine` copies `snapshot.scripts` → `ModuleState.scripts`. Round-trip
    tested (install → save → JSON carries it). **Known gap:** GUI-panel save paths still write
    empty scripts (panel carries none until S2.4); canonical MCP/headless save round-trips.

> **S2.2 COMPLETE (2026-06-15).** The amount cell is genuinely scalar-or-expression,
> end-to-end and round-tripping: persist → compile-on-load → per-voice eval → save.
> Next: **S2.3** (MCP authoring + inspection), then **S2.4** (GUI editor, deferred).
  - [x] **S2.2c** — Voice eval. **SHIPPED `8bc182a`.** The Voice evaluates a slot's script each
    control block: per-(voice,slot) `RegisterFile`, a stack `[f32; MAX_SOURCES]` filled from the
    script's `ScriptInput`s (Source reuses `resolve_source`; Context = gate/gate_on/age/sr; Zero
    = 0), `eval` → offset → `apply_mod_offset_addr`. A scripted slot **replaces** `source×amount`
    (decision #1); script-free matrices take the exact old scalar path (317 engine tests green).
    State resets + PRNG re-seeds per (voice,slot) on note-on. RT-safe (no alloc/lock/log/panic).
    End-to-end tested (a `out = -1` script silences the amp). `age` reads block-start (~1 block on
    block 0 — the steal-priority age bump precedes process; not worth perturbing).

> **MILESTONE (2026-06-15): YAMS scripts are AUDIBLE end-to-end.** persist
> (`ModuleState.scripts`) → compile-on-load (`SetModScript`) → per-voice eval →
> param offset. A routing's amount cell is now genuinely scalar-**or-expression**.
> Remaining S2.2: **S2.2b-2iv** (save/snapshot for round-trip persistence). Then
> **S2.3** (MCP authoring) and **S2.4** (GUI editor, deferred).
- [x] **S2.3** — MCP authoring + inspection. **SHIPPED.**
  - [x] **S2.3a** — Inspection. **SHIPPED `ac99faa`.** `get_mod_matrix_routings` reports each
    slot's `script` source (`MatrixRoutingInfo.script: Option<String>`, skip-if-none); the bridge
    threads `ModuleStateSnapshot.scripts` into `collect_mod_matrix_routings`, which iterates the
    union of param + scripted slots (a script-only slot is surfaced). Script-free output unchanged.
  - [x] **S2.3b** — Authoring. **SHIPPED `2b15ee1`.** MCP tool `set_mod_matrix_script
    { instrument_id, module_id, slot (1-based), source }` → `SynthBridge::set_mod_matrix_script`
    → `AppSynthBridge` (range-check, 1-based→0-based) → `session.set_mod_script` (compile error →
    tool error with diagnostics) / `session.clear_mod_script` (empty source clears). Wired as
    `#[tool]` + dispatch entry + `SetModMatrixScriptParam`. Integration-tested end-to-end.
    (`format_yams` tool not added — `set` already canonicalizes nothing; revisit with S2.4.)
- [ ] **S2.4** — GUI: expand an amount cell into an expression editor. **DEFERRED** — interactive
  egui, not headless-testable (verify in-app); also closes the GUI-panel-save scripts gap (S2.2b-2iv).

### Follow-ups discovered while building Step 2

- [x] **Pitch mod destinations (was parked grammar #12).** **DONE `d81697f`+`218b3f1`.** The
  parked "descriptor `mod_scale` hint on the write side" framing didn't fit — scaling is
  per-module (each `set_mod_offset` hard-codes per-target units), and the real gap was the
  oscillator **silently dropping** `detune`/`frequency` offsets (`_ => {}`). Fixed: `detune` →
  ±1 semitone, `frequency` → ±12 (one octave); legacy `osc-N.pitch` unaffected. Scale lives as
  a documented `const` per target.
- [ ] **Audit `set_mod_offset` coverage across all modules.** The oscillator fix exposed that
  S1.1's "any modulatable param is a destination" is **not actually true** — modules implement
  `set_mod_offset` for only a few hard-coded targets and drop the rest, so the address picker
  can offer a modulatable param whose offset silently vanishes. Audit every module: for each
  descriptor param with `modulatable: true`, confirm `set_mod_offset` handles it (or mark it
  non-modulatable). Headless-testable; the real remaining Step-1 debt.

> **STEP 2 COMPLETE — non-deferred scope (2026-06-15).** YAMS is a fully working modulation
> compute layer: language toolchain (S2.1) → scalar-or-expression amount cell, end-to-end and
> round-tripping (S2.2) → MCP authoring + inspection (S2.3). The **only** remaining item is the
> **S2.4 GUI editor, explicitly DEFERRED** for an interactive egui session (it needs in-app
> verification, like the S1.5 markers did). The autonomous build loop has shipped everything
> headless-verifiable in this plan.

---

## GUI design (Step 1)

A dynamic address book is only usable if you can *see* the modulation topology. Three
affordances, decided 2026-06-14:

- **Per-module markers — ALREADY EXIST, keep.** The patch editor already draws a purple
  badge on the module header (`patch_editor.rs:1834`) with a three-state icon: `↗` source,
  `↙` destination, `↔` both (`is_mod_matrix_source` / `is_mod_matrix_destination`,
  tooltips "Mod Matrix Source/Destination"). This is the module-granularity roll-up — no
  re-spec needed.
- **Per-parameter markers on the knobs — NEW, the Step 1 GUI work.** A small marker on the
  **individual control** that is the actual source/destination, carrying the same
  three-state direction as the module badge. *This is necessary because of Step 1:* today
  addressing is role-based and coarse (only ~2 params per module are reachable), so the
  module badge nearly suffices; once **any** modulatable param can be a destination, the
  module badge is ambiguous ("which of the filter's ten knobs?"). The knob marker is the
  precise indicator; the module badge becomes its roll-up. Reflects **active
  participation**, not capability. (Today only a *filter* exists at param granularity —
  the mod-matrix's own source/dest dropdowns hide unwired choices via
  `is_mod_choice_available`, `patch_editor.rs:5193` — there is no marker on the knob yet.)
- **Macro-source rail.** The hardcoded macros (`velocity, mod_wheel, aftertouch,
  pitch_bend, note, poly_at`) have no module to mark — so give them a visible home: a
  small fixed rail of macro chips in the patch editor, each carrying the **same source
  marker** as a module. Uniform visual language: everything that can be a source looks
  like a source, module or macro.
- **Pick-by-addressing.** Source/destination selection becomes "pick a module → pick a
  param/port" (or "pick a macro"), over the patch's actual contents — replacing the fixed
  enum dropdowns.

### Current GUI state — still legacy-enum, both read and write (verified 2026-06-14)

The headless address work (S1.2/S1.3) never touched the patch editor. The whole mod-matrix
seam is f32-enum-index:

- **Read (`PatchAnalysis::from_panels`, `patch_editor.rs:256`)** decodes each slot's
  source/dest as an f32 index via `ModSource::from_index` / `ModDestination::from_index` →
  `module_target_position()`, then keeps only `(mt, pos)` → `ModuleId` (the param tag at
  `:269` is discarded with `_`). So today's markers are **module-granularity** and limited to
  the ~19 legacy roles.
- **Write (`draw_mod_matrix_grid`, `patch_editor.rs:5212`)** reads slot src/dst as an f32
  index (`slot_idx_value`, `:5227`), renders combos over `descriptor.choices` (the legacy enum
  list, filtered by `is_mod_choice_available` `:5548`), and on change writes `selected as f32`
  back into `param_values` + pushes `sp.id.with_f32(...)`. Arbitrary addresses (a 3rd LFO,
  `osc-1.detune`) are unreachable from the GUI.
- **The representation gap is the crux.** `ModulePanelState.param_values` is
  `HashMap<String, f32>` — it physically cannot hold `"flt-1.cutoff"`. Address picking needs a
  parallel string channel (proposed: `slot_addrs: HashMap<String, String>` keyed by the slot
  param name) that overrides the f32 index when present and is what gets sent + persisted.
  This is the load-bearing part of **S1.5c**, and the reason markers can't precede it.

### Per-parameter marker difficulty — LOW, but **after** S1.5c

The render hook already exists; only the data source changes once addresses are in the panel:

- **The address already carries the exact param.** After S1.5c a routing holds a `DestAddr`
  (`module_type` + 1-based `instance` + `param` = the descriptor `type_id`) / `SrcAddr`. The
  marker key is an exact `(ModuleId, ParamKey)` — no `module_target_position` tag decode, no
  lossy `"cutoff"|…|"pitch"` mapping. **The old "MEDIUM, pitch has no knob" wrinkle is gone**:
  it was an artifact of retrofitting the legacy enum tag; the address path never had it.
- **The shared knob renderer already takes per-param closures.** `draw_parameter_grid`
  (`widgets/param_grid.rs:70`) accepts `get` and `choice_visible`; add a third
  `mod_role: Fn(&ParameterDescriptor) -> Option<ModRole>` (the other caller, mixer return-bus
  inserts, passes `|_| None`).
- **The module badge becomes a roll-up**, not a removal: `is_mod_matrix_source/
  destination(module_id)` becomes "any param entry for this module".

Change surface (S1.5a): `PatchAnalysis` `HashSet<ModuleId>` → `HashMap<ModuleId,
HashSet<ParamKey>>` keeping the roll-up accessors, populated from the parsed `DestAddr`/
`SrcAddr` instead of the enum tag (~30 LOC); new optional closure + per-knob badge in
`draw_parameter_grid` (~40–60 LOC); thread `analysis` + `module_id` into the call site
(`patch_editor.rs:5182`). S1.5b reuses the same map for source markers and adds the macro rail.

---

## S1.1b implementation notes (revised after a spike — 2026-06-14)

A first attempt at a true dynamic `Vec<Routing>` with a count-following descriptor
**was reverted** after it collided with two architectural realities. Recorded here so
the next attempt doesn't repeat them:

1. **The JSON schema is descriptor-driven, and example projects carry the old format.**
   `gen_schemas` builds `descriptors.json` from a *default* module instance; a
   count-following descriptor on a fresh (0-routing) matrix emits no slot params, so the
   `example_files_validate_against_schemas` test fails — the shipped example projects
   (`Synth Pop a la Codex.json`, …) still contain `grid_size` + 16 slot params. Changing
   the descriptor shape breaks schema validation unless every example is re-authored
   (parked point #6) **or** the schema stays a 16-slot+grid superset.
2. **The GUI caches each module's descriptor at add-time** (`add_module(id, descriptor)`
   → `ModulePanelState`), it is **not** refreshed per frame. So a count-following
   descriptor never shows newly-added routings — the panel can't see them.

**Revised approach (keeps schema + persistence + descriptor STABLE):**
- **Data model unchanged:** keep the fixed `[ModSlot; 16]` + `grid_size` field, the
  16-slot descriptor, `get_params`/`set_param`, and therefore the schema and all example
  projects — **no save-format break, point #6 stays parked.**
- **Drop the grid from *processing* only:** the Voice (`read_mod_matrix_slots_into_cache`)
  and `ModMatrix::calculate_modulations` iterate **all 16 slots** instead of
  `grid_size.slot_count()`. `grid_size` becomes a vestigial field (kept for compat,
  ignored). The only behavior change: a project that used a small grid to *disable*
  configured slots beyond it now processes them (accepted under "grid removed").
- **Dynamic list is a GUI presentation over the 16 fixed slots — stateless, derived:**
  `draw_mod_matrix_grid` shows every *configured* routing (source≠None ∥ dest≠None) as a
  row, plus one trailing empty "add" row (when <16 configured); a per-row ✕ clears that
  slot (source→None, dest→None). No grid selector, no new `EngineCommand`/param variants,
  no `Vec`. `PatchAnalysis` iterates all 16 likewise.
- **`set_mod_offset` re-key (S1.1) stays** as the foundation.

This delivers the user-facing goal (no grid, add/remove list, all routings live) with a
fraction of the blast radius. Relaxing the 16 cap and a true dynamic `Vec` (with a
descriptor-refresh path + example migration) is a later, separate step if wanted.

## Data model — dynamic routing list (original spec — superseded by the notes above for S1.1b)

The fixed 16 slots + `Grid Size` exist only because the addressing was a fixed enum. Once
addressing is dynamic (S1.2/S1.3), a fixed slot count is meaningless — the natural
container is a **dynamic list**. This is the same work as the addressing change, from the
container side.

- **Shape:** the module holds `Vec<Routing>`, `Routing = { source: SourceAddr, dest:
  DestAddr, amount: BipolarValue, enabled: bool }`. Add / remove / reorder dynamically.
- **Management pattern — NOT descriptor params.** The fixed `set_param(Param)` /
  descriptor model assumes a *fixed* parameter set per module (today: 64 slot params +
  Grid Size). A variable-length list belongs in the **Note-Processor-rack pattern**:
  dedicated `EngineCommand`s (`AddRouting` / `RemoveRouting` / `SetRouting` /
  `ReorderRouting`), serialized as a `Vec` (serde). Mirror `note_processor.rs`'s rack
  management.
- **Order is cosmetic — keep writes additive.** Multiple routings to one destination sum;
  addition is commutative, and cross-references read the *previous* block's snapshot, so
  **the result is order-independent** in both Step 1 (scalar) and Step 2 (expressions).
  Reorder/drag is therefore a pure readability affordance (group by destination, etc.) with
  **zero audio effect** — support it, but never let it imply a processing chain. The single
  rule that preserves this: **all writes stay additive.** A future "replace / last-wins"
  write mode would make order semantic and reintroduce the determinism problem the
  additive-snapshot model exists to avoid — out of scope, flag if ever wanted.
- **Bonus:** kills the bloated `mmx` descriptor (the 64 slot params / ~3900-line
  `get_module_type_info` response) — replaced by a small descriptor + list MCP tools
  (`get_mod_matrix_routings` already reads).
- **Touch points:** `ModMatrix` struct (`[ModSlot; 16]` + grid → `Vec<Routing>`); drop the
  slot params from the descriptor; new `EngineCommand` variants; MCP add/remove/set/list;
  serde round-trip; rewrite `draw_mod_matrix_grid` (`patch_editor.rs:5216`) as a dynamic
  list; `PatchAnalysis` iterates the `Vec` instead of slots (simpler).
- **Still a module?** Keeping `ModMatrix` a (no-op `process`) graph module is the
  low-disruption path and matches "redesign the module"; the `Vec<Routing>` lives on the
  module, voices clone it as today. This blurs into the patch-scoped-object idea but needs
  no engine restructure now.

## Scaling contract (#2) — DECIDED (option A, LOCKED 2026-06-14)

The numeric meaning of `dest_offset = amount × source`. Two halves.

### Source side (settled)

Every source presents in ~[-1, 1]:
- **Output-port source:** as-is (engine signals are ±1, unipolar `0..1` by convention).
- **Parameter-value source:** normalized through the descriptor `range` → `0..1`, or `-1..1`
  for bipolar params (`min < 0`, e.g. pan, detune). Reading `flt.cutoff` raw (20–20000) is
  meaningless; normalized it is a usable modulator.
- **Macros:** already normalized per macro.

### Destination side (the decision)

`amount` stays a bipolar attenuverter (`-1..1`). `amount × source` (~[-1,1]) is the
contribution. **Convention (decided) — normalized-range offset through the param's
existing curve, summed, then clamped:**

```
effective_norm = clamp( base_norm + Σ_i (amount_i × source_i), 0, 1 )
value          = denormalize(effective_norm)   // through the param range + ResponseCurve
```

- `amount = 1, source = 1` → push the param by 100 % of its travel. **Uniform across every
  modulatable param — no per-param table to maintain**, which is the whole point of dynamic
  addressing.
- **Musicality is free from the existing curve:** a normalized offset on a log cutoff is a
  roughly-proportional (octave-ish) Hz move; on a linear param (pan, decay) it is a plain
  linear offset.
- Summing in normalized space *before* clamp keeps it additive → order-independent.

**Rejected as default:** a native-unit per-param-type table (what the 19 current dests do —
cutoff in semitones, etc.). Musical, but needs a hand-maintained convention per param kind
and does not auto-generalize to new params. **Door left open:** an optional per-descriptor
`mod_scale` / `mod_unit` hint for the few params that want a specific feel (e.g. pitch in
exact semitones). Default stays normalized-through-curve; add hints only where the curve is
not enough.

### Concrete spec for the widened channel (S1.1)

`set_mod_offset(param_id, normalized_delta)` — the offset arg is a **normalized-space
additive delta**; the module sums deltas per param, applies through its range/curve, and
**clamps** (RT-safe, no out-of-range). `clear_mod_offsets()` unchanged.

## Engine architecture (shared by both steps)

### Where it lives

The mechanism stays where the Mod Matrix already runs: **per voice**, in the Voice
traversal, applying additive offsets to modules resolved by identity. Per-voice is the
default (it gives live per-voice source values — this voice's envelope/LFO/velocity — via
the shared cache). Per-instrument/global placement is a possible later addition; cross-
*instrument* modulation is out of scope (breaks the patch boundary; song-level logic
belongs in the sequencer/Note-Processor layer).

### Read side

- **Port snapshot** (`ModuleId` + `PortName`): previous block's output of a module —
  best for signal-derived sources (envelope position, LFO, env-follower, kinetic). Default
  to the module's canonical `out`; **the port is selectable** when a module exposes several
  (osc `out`/`out_l`/`out_r`, amp `left`/`right`/`out`).
- **Parameter value** (`ModuleId` + `Param`): a knob's current (possibly automated) value.
- **Macro registry** (S1.3): the six true per-voice macros, no `ModuleId`.

Addresses are **resolved offline** (at install time) to raw indices/pointers — exactly as
the topo sort and the current destination resolution already do. The hot loop only indexes
arrays.

### Write side (the one substantive engine change — S1.1)

Today: `set_mod_offset(dest_index: u8, value)` covers only the fixed `ModDestination` set
(offset index 0/1 per module). S1.1 widens it so a module accepts an **additive offset
against any of its `modulatable` parameters**, keyed by a stable param-id (the descriptor
already enumerates them).

- Effective value is already `(override.unwrap_or(base)) + mod_offset`. We write the
  **`mod_offset` term** — never `set_param`. This composes with the user's base value and
  with automation, and never clobbers either. **This is the load-bearing correctness rule.**
- Touch points: `PolyModule` (offset-by-param-id; `clear_mod_offsets` stays);
  `graph.rs::apply_mod_offset` (emit a general param-id offset, not the narrow index);
  `ModMatrix` resolution rewritten to target the general channel.
- After S1.1, every modulatable param (`osc1.detune`, `env1.decay`, …) is a valid
  destination — the headline win, with zero UI change.

### Latency / cycles

Reading outputs and writing params is a feedback path the topo sort does not model (true
of the Mod Matrix today). Standard resolution: **read the previous block's snapshot, write
this block's offset.** One-block latency → no cycle, deterministic, no need to place the
unit in the topo sort. Exactly how a normal modulation matrix behaves.

### RT contract (copy Note Processors)

Compile/resolve **offline** into an immutable `Arc`. On the audio thread: **no heap
alloc, no locks, no unbounded loops, no logging.** Step 2's evaluator runs against a fixed
register file with a hard instruction cap; overflow clamps and is counted, never
reallocates. Mirror `note_processor.rs` (bounded buffers, seeded PRNG, drop-and-count).

### Crate placement

- `synth_core`: `ScriptSource`/`ScriptDest` addressing, the widened modulatable-offset
  model; (Step 2) `CompiledScript` + the RT evaluator.
- `synth_engine`: Voice-traversal wiring + new `EngineCommand` variants.
- Parser/compiler (non-RT, Step 2): `synth_core` behind a feature or a small `synth_script`
  crate; UI/MCP thread only.
- `synth_mcp` + `pertylizer` (GUI): authoring/inspection.

---

## Context — verified engine state (2026-06-14)

- **`PolyModule`** (`crates/synth_core/src/module_traits.rs`): `process(inputs, outputs,
  ctx)`, `set_param`, `get_param`, `set_mod_offset(dest_index: u8, value: f32)`,
  `clear_mod_offsets`, `set_param_override`, `clear_param_overrides`. A node sees only its
  `InputPorts`.
- **Layering:** effective = `(override.unwrap_or(base)) + mod_offset` — we own `mod_offset`.
- **Mod Matrix is a no-op `process()`** (`crates/synth_modules/src/mod_matrix.rs`); routing
  runs in the **Voice** (`crates/synth_engine/src/voice.rs`) via a `source_values` cache and
  `graph.rs::apply_mod_offset(dest, value)` → module instance + offset index (0/1). **This is
  the seam Step 1 widens.**
- **Addressing primitives exist:** `ModuleId`+`Param`, `ModuleId`+`PortName`. Connections
  topo-sorted, zero-alloc, `BTreeMap` node order.
- **Descriptors mark `modulatable: bool`** per param — the exact set S1.1 exposes.
- **RT contract is practiced** by Note Processors (`crates/synth_sequencer/src/
  note_processor.rs`): offline compile, bounded hard-capped buffers, seeded PRNG, no
  alloc/log on the audio thread. Copy it.
- **`EngineCommand`** (`crates/synth_engine/src/commands.rs`) is the UI/MCP → audio channel.

---

## Decided

- **Addressing: ID-based (`ModuleId`).** LOCKED 2026-06-14. Precise, reaches arbitrary
  instances; does **not** auto-rebind and dangles on delete → requires the disable-and-keep
  policy (S1.4). (Role-based `flt1` was the alternative — convenient but fuzzy, can't reach
  a third instance cleanly.)
- **Source taxonomy:** six true macros (`velocity, mod_wheel, aftertouch, pitch_bend, note,
  poly_at`); `lfo/env/kinetic/env-follower` are module sources (S1.3).
- **Source port:** default canonical `out`, selectable for multi-output modules (S1.3 read
  side).
- **Multiple `mmx` modules:** stay allowed — simplest, no consolidation; split routings into
  several modules for organization if wanted.
- **Scaling contract (#2): option A** — normalized-range offset through the param's curve,
  summed, clamped (see Scaling contract section). Door left open for an optional per-param
  `mod_scale` hint (option C) only where the curve isn't enough.

## Open questions

_All Step 1 design locks resolved. Remaining open items are Step 2 only:_

- **Unified panel (Step 2).** Once the amount cell is scalar-or-expression, "Mod Matrix"
  and "Control Script" are the same model. *Leaning toward one unified modulation panel*
  (option B) rather than two separate tools. Data-model consequence: matrix-row and
  script-row live in one list.
- **Per-voice vs (later) per-instrument** placement — ship per-voice; revisit on demand.
- **Expression tree vs bytecode VM** (S2.1) — start with the tree; promote to a VM when
  state/conditionals/multi-output demand it.

## Risks

- **S1.1 is behavior-preserving for existing routings** — verify by ear + the mod-matrix
  tests, do not assume.
- **Feedback determinism** — honor the 1-block-latency rule everywhere a source could also
  be influenced; otherwise results depend on traversal order.
- **Dangling references** — dynamic ID addressing must define delete/rename behavior, or
  routings silently break.
- **RT discipline** — no alloc/lock/log on the audio thread; the instruction cap is
  load-bearing, exactly as the Note-Processor caps are.

## Reference — the demo this plan grew from

Instrument `osc → flt → amp → out` with a Mod Matrix (no wires into `cutoff_cv`) routing
`lfo1 → flt1.cutoff` (+0.45) and `velocity → flt1.cutoff` (+0.6). Soft strike → closed
filter; hard strike → open; LFO wobbles throughout. That is "what the Mod Matrix does
today." What it *cannot* do — a **third** LFO as a source, `osc1.detune` as a destination,
`cutoff = lfo × velocity`, `velocity > 0.8 ? open : closed` — is exactly what Step 1
(addressing) and Step 2 (compute) deliver.
