# Plan: Mod Matrix routing visibility (§4.4 of docs/TODO.md)

## Context

### The problem
When a patch routes a modulation via Mod Matrix (e.g., `lfo-1 → flt-1.cutoff`), the source module looks dead in the patch editor — no visible cable, and `list_modules` reports `output_ports: []`. The user can't tell from the cable view alone whether `lfo-1` is unused or routed via matrix.

The current patch editor already gives the **effect chain** its own framed zone (`draw_effect_zone`, `patch_editor.rs:2438–2503`) — a tinted rounded rect with a `ri::FLASHLIGHT_FILL` icon and "Effect Chain" label. The **Mod Matrix module floats free on the canvas without any visual signalling that it's a special routing artifact** (confirmed by user screenshot).

### Why Mod Matrix is *not* on the cable graph (historical answer)
The cable graph (`crates/synth_engine/src/graph.rs`) enforces strict acyclic topology via `would_create_cycle()` (line 663). A direct cable from an `EnvelopeFollower` (which reads a filter's audio output) into that same filter's cutoff would form `Filter → Amp → EnvFollower → Filter` — rejected by the topo sort.

Mod Matrix bypasses this by:
- Storing routings as **parameter slots** on a dedicated module (16 slots, `crates/synth_modules/src/mod_matrix.rs:51`), not as `Connection` objects
- Applying modulations from **cached source values** in `Voice::process()` (`crates/synth_engine/src/voice.rs:670–730`) *before* the graph processes — so the cycle constraint never engages
- Secondary benefits: mass modulation without cable clutter (1 source → N dests with individual attenuators), per-voice efficiency

Drawing ghost cables for matrix routings would fight that design intent — visually re-introducing exactly what was structurally avoided.

## Current state (what's already done)

User-confirmed via screenshot — these existing pieces work and do not need rework:

- **Framed effect-chain zone** exists: `draw_effect_zone` (`patch_editor.rs:2438–2503`) draws tinted rect + border + label for Effect/Visualizer modules. The user's screenshot shows it working correctly.
- **`align_effect_chain`** (`patch_editor.rs:4030–4071`) stacks effect modules vertically by chain order.
- **Mod Matrix renders with semantic dropdowns**: `draw_mod_matrix_grid` (`patch_editor.rs:4776`) already shows `[LFO 1 ▾]`, `[Filter 1 Cutoff ▾]`, `[Amount knob]` — no enum-index UI.
- **MCP slot reading** works via the module's parameters (`get_module_info` on `mmx-1` shows `slot_1_source`, `slot_1_dest`, etc.).

What's missing (visible in user screenshot):
1. **No frame around the Mod Matrix module** — it floats next to the Effect Chain zone without its own framed treatment.
2. **No badge on `lfo-1`** showing it's a matrix source — the user can't tell that LFO 1 is actually wired up via matrix.
3. **MCP `output_ports: []`** on matrix-only source modules is still misleading (AI users see "no outputs" and conclude "dead").
4. **No clickable cross-reference** between slot dropdowns and the source/dest module in the canvas (clicking "LFO 1" in the slot should optionally focus the lfo-1 module).

## Approach

Mirror the existing `draw_effect_zone` pattern for ModMatrix, add module-header badges keyed off a per-frame cross-reference index, and add the MCP surface. No ghost cables.

### Why this beats ghost cables
- **Respects the design intent**: three paradigms visually distinct (voice graph + cables, framed effect chain, framed mod matrix), not mixed
- **Scales**: 16 slots in a list don't clutter the canvas; 16 ghost cables would
- **Mass-modulation case** (one source → many dests) is readable in the existing matrix grid, illegible as fan-out cables

## Phased plan

### Phase 1 — MCP surface (no GUI)
Lowest risk, gives AI tools matrix-routing visibility before any GUI work. ~60 lines.

- In `crates/pertylizer/src/mcp_bridge.rs:155–235` (the `list_modules` / `get_module_info` JSON builder), when a module's `type_id == "mod_matrix"` emit `output_ports: ["matrix"]` instead of `[]`, and add a `mod_matrix_routings` field on `get_module_info` containing the live slot table: `[{slot: 1, source: "lfo-1", dest: "flt-1.cutoff", amount: 0.9, enabled: true}, ...]`.
- New MCP tool `get_mod_matrix_routings(instrument_id) -> Vec<Routing>` walks all ModMatrix instances in the patch and returns their non-empty slots; register in `crates/synth_mcp/src/server.rs` next to `get_connections`.
- Source/dest enum → semantic-ID conversion already lives in `graph.rs:454–485`; reuse it (this is what `draw_mod_matrix_grid` uses for its dropdown labels too).
- Regression test: load `Acid Bass`, call new tool, assert `[{slot:1, source:"env-2", dest:"flt-1.cutoff", amount:0.9, enabled:true}]`.

### Phase 2 — Framed Mod Matrix zone
Clone `draw_effect_zone` for `ModuleCategory::Utility` modules of `type_id == "mod_matrix"`. ~30 lines.

- New `draw_mod_matrix_zone(ui, scroll_rect)` in `patch_editor.rs`, structurally identical to `draw_effect_zone` (lines 2438–2503): bounding box of matrix modules → tinted rect (use a distinct accent — e.g. a "modulation" colour, separate from `category_color(Effect)`) → 1px border at 15% alpha → header text with icon and "Mod Matrix" label.
- Icon: `ri::SWAP_LINE` or `ri::PULSE_LINE` (something signalling routing/modulation).
- Call site: wherever `draw_effect_zone` is invoked (look up the single caller in `patch_editor.rs`).
- No `align_mod_matrix_column` needed — patches typically have one matrix module, so vertical stacking is moot. If multiple matrices ever exist, they get framed together in one zone the same way effects do.

### Phase 3 — Badges on referenced voice-graph modules
Surface "this module is wired through the matrix" on the source/dest module headers.

- New helper `compute_mod_matrix_references(panels, descriptors) -> HashMap<ModuleId, ModMatrixRole>` rebuilt each frame, walking matrix-module slot params and resolving `ModSource`/`ModDestination` → semantic `ModuleId` via `graph.rs:454–485`. Role enum: `{Source(Vec<SlotIdx>), Destination(Vec<SlotIdx>), Both(Vec<SlotIdx>)}`.
- In `module_panel.rs:115–126` (the header strip), when the module ID is in the reference map, append an `egui::Label` with `ri::ARROW_RIGHT_UP_LINE` (source) and/or `ri::ARROW_LEFT_DOWN_LINE` (destination). Different colours for the two roles.
- Tooltip lists routings: `"Source for Slot 1 → flt-1.cutoff (+0.9)"`.
- Cheap: 16 slots × N modules per frame is negligible.

### Phase 4 — Click-to-highlight in slot dropdowns (optional polish)
Make the existing semantic dropdowns in `draw_mod_matrix_grid` (`patch_editor.rs:4776`) clickable as targets.

- When the slot's source/dest label is clicked (or right-clicked → "Locate"), scroll the canvas to the corresponding module and add a temporary highlight ring.
- Requires a "focus this module" mechanism — check if one already exists (e.g., for "Go to module" actions); reuse it. If not, add a transient `highlighted_module: Option<(ModuleId, Instant)>` state that the module-panel reads and fades over ~1s.
- Pure polish — Phases 1–3 already solve the discoverability problem. This makes the matrix → graph navigation feel two-way.

## Critical files

| File                                                         | What changes                                                          |
|--------------------------------------------------------------|-----------------------------------------------------------------------|
| `crates/pertylizer/src/mcp_bridge.rs:155–235`                | Phase 1: `output_ports: ["matrix"]` + `mod_matrix_routings` field     |
| `crates/synth_mcp/src/server.rs`                             | Phase 1: register `get_mod_matrix_routings` tool                      |
| `crates/synth_engine/src/graph.rs:454–485`                   | Reuse: `ModDestination` → semantic module ID                          |
| `crates/synth_modules/src/mod_matrix.rs:140–187`             | Reuse: slot iteration                                                 |
| `crates/pertylizer/src/gui/patch_editor.rs` (new fn)         | Phase 2: `draw_mod_matrix_zone`, call site                            |
| `crates/pertylizer/src/gui/patch_editor.rs:2438–2503`        | Reuse template: `draw_effect_zone` as the model                       |
| `crates/pertylizer/src/gui/patch_editor.rs` (new helper)     | Phase 3: `compute_mod_matrix_references`                              |
| `crates/pertylizer/src/gui/module_panel.rs:115–126`          | Phase 3: badge icon(s) in header                                      |
| `crates/pertylizer/src/gui/patch_editor.rs:4776`             | Phase 4 (optional): click-to-highlight from slot dropdowns            |

## Verification

After Phase 1:
- `apply_example_patch "Acid Bass"` via MCP
- `get_mod_matrix_routings 1` returns `[{slot:1, source:"env-2", dest:"flt-1.cutoff", amount:0.9, enabled:true}]`
- `get_module_info` on `mmx-1`: `output_ports: ["matrix"]`; `mod_matrix_routings` field populated
- New unit test in `mod_matrix.rs` asserting routing extraction

After Phase 2:
- Load any patch with a Mod Matrix module: the matrix module sits inside a framed tinted zone with a "Mod Matrix" header, visually parallel to the Effect Chain zone in the user's screenshot
- Empty patches (no matrix): no zone drawn (mirror `has_effects` early-return)

After Phase 3:
- Open `Acid Bass`: `env-2` shows a source-arrow badge in its header
- `flt-1` shows a destination-arrow badge
- Tooltip on `env-2`'s badge reads "Source for Slot 1 → flt-1.cutoff (+0.9)"
- Open a patch with no matrix routing: no badges anywhere

After Phase 4 (if pursued):
- In the Mod Matrix slot UI, clicking the "env-2" label scrolls the canvas to env-2 and pulses its border for ~1s
- Clicking the "Filter 1 Cutoff" dest label does the same for flt-1
