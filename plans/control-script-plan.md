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
- [ ] **S1.1** — Widen the write channel: `set_mod_offset(dest_index: u8)` → additive
  offset **by modulatable param-id**, on every module. Re-express the existing 19
  destinations on top of it. *Internally* behavior-preserving for current routings —
  verify bit-for-bit against the acid-bass demo.
- [ ] **S1.1b** — **Dynamic routing list** (replaces the fixed 16 slots + Grid Size): the
  module holds a `Vec<Routing>` (`source, destination, amount, enabled`), managed by
  dedicated add/remove/set/reorder commands — **not** the fixed descriptor-param model.
  This is the container S1.2/S1.3 addresses fill. See "Data model" section.
- [ ] **S1.2** — Dynamic **destination** addressing: a routing's destination is
  `(ModuleId, ParamId)` over the patch's actual modulatable params, not the 19-role enum.
- [ ] **S1.3** — Dynamic **source** addressing + the shared **macro-source registry**:
  sources are `(ModuleId, port|param)` over the patch **plus** the named per-voice macros.
  Removes the 2-LFO / 2-env ceiling. **Taxonomy (decided):** the *true macros* are exactly
  `velocity, mod_wheel, aftertouch, pitch_bend, note, poly_at` — they have no `ModuleId`.
  Everything else in today's 16-source enum is a **module** and becomes a normal
  module-addressed source: `lfo`, `env`, **`kinetic_pos/vel/acc`** (KineticModulator),
  **`efl1/efl2`** (EnvFollower). The macro rail shows only the six true macros.
  Includes **S1.7 voice-snapshot wiring** (below).
- [ ] **S1.7** — Voice snapshot wiring: each block the Voice collects the **union of
  referenced source ports/params** into the per-voice cache (today it populates only the 16
  fixed sources). A real mechanism extension — the resolved routing list drives which ports
  to snapshot. Pairs with S1.3.
- [ ] **S1.4** — Persistence + MCP: address-based routings round-trip; dangling-reference
  policy on module delete (disable-and-keep); `get_mod_matrix_routings` already reports
  dotted IDs — extend authoring tools to take addresses.
- [ ] **S1.5a** — GUI: per-parameter **destination** knob marker — **lands with S1.2**.
  Trivial once routings carry the exact `ParamId` (the analysis already computes the target
  param; the wrinkle is the enum's `"pitch"` tag, gone after S1.2). See GUI section.
- [ ] **S1.5b** — GUI: per-parameter **source** marker + the **macro-source rail** —
  **lands with S1.3**. Per-param sources only mean something once a knob's *value* is a
  readable source; the macros need a visible home (no `ModuleId` to mark).
- [ ] **S1.5c** — GUI: **pick-by-addressing** dropdowns (module → param/port, or macro)
  replacing the fixed enum dropdowns. The per-module badge already exists
  (`patch_editor.rs:1834`) and becomes a roll-up of the per-param data.

(All S1.5 work is DEFERRED for interactive egui work — not headless-testable.)

**S1.1 is shippable alone** — it lifts the destination ceiling (any modulatable param)
with no UI change. S1.2/S1.3 deliver the full dynamic addressing; the matching GUI markers
(S1.5a/S1.5b) ride along with them rather than as a separate phase.

### Step 2 — Control Script (the compute layer)

- [ ] **S2.1** — `CompiledScript` type + control-rate evaluator: expression tree per
  destination first (`dest = sigmoid(a*s1 + b*s2)`), grow to a small stack/register
  **bytecode VM** when state/conditionals/multi-output demand it. Offline compile,
  immutable `Arc`, allocation-free eval, fixed register file, hard instruction cap.
- [ ] **S2.2** — Make the amount cell **scalar-or-expression**: a routing whose amount is
  an expression evaluates the script (reading its bound sources) instead of a single
  multiply. Same addressing, same offset write.
- [ ] **S2.3** — MCP authoring (source text or structured route list) + inspection.
- [ ] **S2.4** — GUI: expand an amount cell into an expression editor (DEFERRED).

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

### Per-parameter marker difficulty — LOW, and do it *with* S1.2

Assessed against `PatchAnalysis` (`patch_editor.rs:210`) on 2026-06-14. The data and the
render hook already exist:

- **The target param is already computed and thrown away.** The analysis build loop
  decodes each destination via `ModDestination::module_target_position()`, which returns
  `(ModuleType, pos, param_tag)` — `param_tag` ∈ `"cutoff" | "resonance" | "level" | "pan"
  | "rate" | "depth" | "pitch"` (`mod_matrix.rs:442`). Today only `(mt, pos)` is bound to a
  `ModuleId` and the tag is discarded (`patch_editor.rs:273`, the `_`). Keeping it is a
  one-char change plus a wider key.
- **The shared knob renderer already takes per-param closures.** `draw_parameter_grid`
  (`widgets/param_grid.rs:70`) already accepts `get` and `choice_visible` per-param
  closures; adding a third `mod_role: Fn(&ParameterDescriptor) -> Option<ModRole>` is
  idiomatic. The other caller (mixer return-bus inserts) passes `|_| None`.
- **The module badge becomes a roll-up**, not a removal: `is_mod_matrix_source/
  destination(module_id)` becomes "any param entry for this module".

Change surface: `PatchAnalysis` `HashSet<ModuleId>` → `HashMap<ModuleId,
HashSet<ParamKey>>` keeping the roll-up accessors (~30 LOC); stop discarding the tag
(~5 LOC); a new optional closure + per-knob badge in `draw_parameter_grid` (~40–60 LOC);
thread `analysis` + `module_id` into the call site (`patch_editor.rs:5182`).

**The one wrinkle — and why it vanishes with S1.2.** The `ModDestination` tag maps cleanly
to a descriptor `type_id` for `cutoff/resonance/level/pan/rate/depth`, but `"pitch"` has
**no knob** (pitch is set by the note; pitch-mod is an additive semitone offset). So a
retrofit on today's enum is **MEDIUM** (pitch/level edge cases, lossy tag→knob mapping).
Once S1.2 stores the **exact `ParamId`** over real modulatable params, the match is true
by construction — **TRIVIAL**. Therefore: build the per-knob marker **as part of S1.2**,
not as a standalone retrofit. Per-param *source* markers wait for S1.3 (a source is a whole
module's output today; per-param sources only mean something once a knob's *value* is a
readable source).

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
