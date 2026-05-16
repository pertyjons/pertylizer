# Auto-layout Improvement Plan

This document is an implementation plan for an AI coding agent that should improve the
module auto-layout feature in Pertylizer.

All ten problems below have been cross-checked against the current source tree. Each
problem section includes a **Verified** note with the file:line landmarks the implementing
agent needs.

## Goal

Make auto-layout deterministic, easier to reason about, and robust across:

- normal signal-chain modules,
- modulation modules,
- global/effect/visualizer modules,
- disconnected modules,
- collapsed groups,
- MCP-triggered layout requests.

The main code paths are:

- `crates/pertylizer/src/gui/auto_layout.rs` (1438 lines)
- `crates/pertylizer/src/gui/patch_editor.rs` (5520 lines)
- `crates/pertylizer/src/gui/egui_backend.rs` (5840 lines)
- `crates/pertylizer/src/mcp_bridge.rs` (6044 lines)
- `crates/pertylizer/src/gui/patch_bridge.rs`
- `crates/pertylizer/src/patch.rs`
- `crates/pertylizer/src/session.rs`
- `crates/synth_engine/src/effect_chain.rs`
- `crates/synth_engine/src/shared_state.rs`
- `crates/synth_mcp/src/server.rs`

## Current Problems

### 1. Modulator ordering is calculated but not respected

`place_modulation()` returns `HashMap<ModuleId, (usize, usize)>` where the tuple is
`(column, modulation_row)`. The final placement loop iterates that `HashMap` and binds
the row to `_mod_row`, discarding it. The y-coordinate then comes from a running
`col_bottom` counter, so envelope/LFO order depends on `HashMap` iteration order.

**Verified.** Key landmarks:

- `auto_layout.rs:496-507` — `place_modulation()` signature and row computation (`530-535`).
- `auto_layout.rs:729` — `for (&mod_id, &(col, _mod_row)) in &mod_positions` discards the row.
- `auto_layout.rs:731` — y comes from `col_bottom`, not from the row.

### 2. Effect chain modules are laid out twice

`PatchEditor::apply_auto_layout()` calls `calculate_layout_with_chain_order()` and then
calls `align_effect_chain()` as an unconditional post-pass. The post-pass derives its
x-coordinate from the *current* panel positions (`avg_x = mean(panel.position.x + panel.size.x * 0.5)`),
so it overwrites the auto-layout result with a function of where the effects used to be.

Effects without graph cables are classified as `Disconnected` by
`classify_module_group()` (because the classifier only treats `Effect` as
signal-chain when it has connections), then moved again by the post-pass — which can
overlap with the disconnected zone.

**Verified.** Key landmarks:

- `patch_editor.rs:4172-4177` — `apply_auto_layout()` calls `calculate_layout_with_chain_order()`.
- `patch_editor.rs:4188-4189` — unconditional `self.align_effect_chain(effect_chain_order)`
  immediately after applying positions.
- `patch_editor.rs:3967-4008` — `align_effect_chain()` body; computes `avg_x` from current
  panel positions, stacks vertically with `GRID_SIZE` gaps.
- `auto_layout.rs:91-92, 107` — `Effect` modules without connections classify as
  `Disconnected`; with connections classify as `SignalChain`.

### 3. `available_rect` is passed but ignored

Both public layout functions accept `available_rect: Rect` but bind it as
`_available_rect` and never read it. Layout always starts at `(GRID, GRID) = (48, 48)`
and expands left-to-right indefinitely.

**Verified.** Key landmarks:

- `auto_layout.rs:562-568` — `calculate_layout()` parameter is `_available_rect: Rect`.
- `auto_layout.rs:575-580` — `calculate_layout_with_chain_order()` same.
- `auto_layout.rs:687-688` — `let start_x = GRID; let start_y = GRID;`.

### 4. Collapsed groups are not treated as visible layout objects

`compute_group_layout()` extends `hidden_modules` with all `group.members` when
`group.collapsed` is true, and the render loop skips those modules. But
`apply_auto_layout()` collects `ModuleInfo` from `self.panels` without consulting group
state, so it moves the hidden member panels and never repositions the collapsed group box.

**Verified.** Key landmarks:

- `patch_editor.rs:882, 902-904` — `compute_group_layout()` builds `hidden_modules` from
  collapsed group members.
- `patch_editor.rs:1358-1360, 1441-1442` — render path skips `hidden_modules`.
- `patch_editor.rs:4137-4190` — `apply_auto_layout()` never inspects `groups` or
  `collapsed`.

### 5. MCP auto-layout requests can be lost outside Rack view

`pending_auto_layout` is read with `swap(false, Relaxed)` at the top of every frame.
The application of auto-layout only happens inside the `AppView::Rack` match arm.
A request that arrives while the user is in `AcousticWorld` or `Sequencer` is consumed
and dropped without `apply_auto_layout()` running.

**Verified.** Key landmarks:

- `mcp_shared.rs:39` — `pub pending_auto_layout: AtomicBool`.
- `egui_backend.rs:676-680` — `swap(false, Relaxed)` at frame top, unconditionally.
- `egui_backend.rs:2257-2262` — application only inside `AppView::Rack` branch.
- `egui_backend.rs:2270, 2312` — `AcousticWorld` and `Sequencer` branches do nothing
  with the flag.
- `synth_mcp/src/server.rs:2370-2375` — tool returns `"queued for next frame"`
  unconditionally; no acknowledgement contract with the GUI.

### 6. Effect-chain order is not persisted explicitly

Patch/project files persist module positions, group positions, and canvas size, but
they do not persist an explicit effect-chain order. `PatchSettings` has no such field.
On load, processing order is reconstructed from the order of `patch.modules`. That
order is produced by `PatchEditor::module_ids()` which collects from a `HashMap`, and
`EffectChain::add_effect()` unconditionally pushes to the end.

**Verified.** Key landmarks:

- `patch.rs:538-576` — `PatchSettings` has master_volume, bpm, octave_offset,
  glide_time, awe, canvas_size — no `effect_chain_order`.
- `patch_editor.rs:280` — `panels: HashMap<ModuleId, ModulePanelState>`.
- `patch_editor.rs:4017-4018` — `module_ids()` returns `self.panels.keys().copied().collect()`.
- `patch_bridge.rs:450` — `create_patch_from_editor()` iterates `module_ids()`.
- `patch_bridge.rs:75-77` — `load_patch()` adds modules in `patch.modules` order.
- `synth_engine/src/effect_chain.rs:212` — `add_effect()` pushes to end.

### 7. Saved effect positions are overwritten on first render after load

`PatchEditor::show()` compares `effect_chain_order` to `prev_effect_chain_order` and
calls `align_effect_chain()` when they differ. After `load_patch_data()` or
`load_project_data()`, `prev_effect_chain_order` is still `Vec::new()` from
construction. Any patch with at least one effect realigns on the first Rack render.
Because `align_effect_chain()` recomputes positions from `avg_x` of current panels
(see Problem 2), saved positions are lost.

**Verified.** Key landmarks:

- `patch_editor.rs:329` — `prev_effect_chain_order: Vec<ModuleId>` field.
- `patch_editor.rs:366` — initialized to `Vec::new()`.
- `patch_editor.rs:1304-1306` — comparison at top of `show()` triggers
  `align_effect_chain()`.
- `egui_backend.rs:4789-4839` — `load_patch_data()` restores canvas size but does not
  seed `prev_effect_chain_order`.
- `egui_backend.rs:5329-5405` — `load_project_data()` same.

### 8. Module save order is nondeterministic

`create_patch_from_editor()` iterates `module_ids()` which iterates `HashMap` keys.
Parameter values use `BTreeMap` (stable) but the modules array itself is unstable,
producing noisy diffs and feeding the effect-chain reconstruction problem above.

**Verified.** Key landmarks:

- `patch_editor.rs:4017-4018` — unsorted `HashMap` keys.
- `patch_bridge.rs:450` — used directly without sorting.

### 9. Collapsed group movement does not move member modules

When the user drags a collapsed group, `draw_collapsed_groups()` updates
`group.position` only. Member panel positions are not touched. The expand toggle just
flips `group.collapsed = false`, after which `group_bounds_world()` recomputes from
unchanged member panels, so the group visually snaps back to the old location.

This also affects persistence and templates:

- Project save stores both `ModuleState.position` for members and
  `ModuleGroupState.position` for the collapsed box.
- After a drag those two systems disagree.
- `build_group_template()` uses member positions, so a template captured from a moved
  collapsed group represents the old internal layout.

**Verified.** Key landmarks:

- `patch_editor.rs:2990` — `group_mut.position = snap_to_grid(logical_pos);` — only
  the group, no delta applied to members.
- `patch_editor.rs:2993-2995, 3007-3010` — expand toggle sets `collapsed = false`
  and nothing else.
- `patch_editor.rs:852-866` — `group_bounds_world()` recomputes from members.
- `patch_editor.rs:640-718` — `build_group_template()` normalizes from member
  positions.

### 10. Position readback suppression does not cover collapsed group boxes

`PatchEditor::clear()` sets `suppress_position_readback = true`. The module panel
readback path checks this flag before overwriting `panel.position` from
`area_rect`. The collapsed group readback does not check the flag and overwrites
`group.position` from `area_rect` unconditionally.

The flag is reset to `false` at the end of `show()`, so suppression lasts a single
frame. That is the same window the module path uses; the group path just needs to opt
into it.

**Verified.** Key landmarks:

- `patch_editor.rs:335` — `suppress_position_readback: bool`.
- `patch_editor.rs:440` — `clear()` sets it to true.
- `patch_editor.rs:2159` — reset to false at end of `show()`.
- `patch_editor.rs:2036-2043` — module readback gated by the flag.
- `patch_editor.rs:2986-2991` — group readback NOT gated.

## Implementation Plan

### Phase 1 - Add focused regression tests first

Twelve tests already exist in `auto_layout.rs` (`#[cfg(test)] mod tests` at line 808):

- `test_empty_layout`
- `test_single_disconnected_module`
- `test_linear_chain_within_bounds`
- `test_modulation_below_and_within_bounds`
- `test_disconnected_modules_in_corner`
- `test_multi_source_to_mixer`
- `test_complex_patch`
- `test_no_overlap`
- `test_output_rightmost`
- `test_effect_in_signal_chain`
- `test_utility_in_signal_chain`
- `test_no_overlap_mixed_sizes`

Extend this module. Add or update tests for:

- Multiple modulation modules targeting different rows in the same column.
  - Expected: final y-order follows target row order, with deterministic tie-breaks.
- Multiple modulation modules targeting the same row.
  - Expected: deterministic tie-break by `module_sort_key`
    (auto_layout.rs:66-68, sorts by `(module_type as u32, instance)`).
- Effect-chain modules plus disconnected voice modules.
  - Expected: no overlap and effect modules remain in chain order.
- Effect-chain modules with no graph cables.
  - Expected: classified/placed as global effect-chain modules, not as disconnected
    modules.
- Non-zero `available_rect.min`.
  - Expected: generated positions start from the rect origin plus grid padding.
- Collapsed group.
  - Expected: collapsed group box is treated as the visible thing to move, or
    auto-layout skips hidden members without corrupting the group state.
- MCP request while not in Rack view.
  - Expected: request remains pending until Rack can apply it, or the bridge returns
    an explicit "cannot apply now" result.

Run at minimum:

```bash
cargo test -p pertylizer gui::auto_layout
```

If `PatchEditor` tests are added and no narrow filter exists, run:

```bash
cargo test -p pertylizer
```

### Phase 2 - Make modulation placement deterministic

Change modulation placement so ordering survives into pixel placement.

Recommended approach:

1. Replace or supplement `place_modulation()` so it returns a sorted vector, for example:

   ```rust
   Vec<ModPlacement>
   ```

   where `ModPlacement` contains:

   - `module_id`
   - `column`
   - `row`
   - optional `target_row`

2. Sort by:

   - column,
   - target row / modulation row,
   - `module_sort_key(module_id)` (existing helper at `auto_layout.rs:66-68`).

3. In the final placement loop (currently `auto_layout.rs:729`), iterate that sorted
   vector instead of the `HashMap`. The y coordinate should advance per row inside a
   column rather than relying on an opaque `col_bottom` counter.

4. Remove the `_mod_row` binding (`auto_layout.rs:729`) once placement consumes the row.

Acceptance criteria:

- Repeated runs produce identical positions for the same graph.
- Modulator vertical order matches target row order.
- Unit tests fail on the old unordered implementation and pass after the change.

### Phase 3 - Move effect/global layout into the core layout result

Remove the need for `PatchEditor::apply_auto_layout()` to call `align_effect_chain()`
after `calculate_layout_with_chain_order()`. The current post-pass derives x from the
old panel layout (avg_x), which is what causes auto-layout output to be partly
overwritten.

Recommended approach:

1. Clarify classification (`auto_layout.rs:classify_module_group`):

   - Effects should be treated as effect-chain/global modules even when they have no
     graph cables (today they leak into `Disconnected` at `auto_layout.rs:91-92`).
   - Global visualizers and Mod Matrix should keep global placement behavior.
   - Voice-level inline visualizers such as Signal Monitor should remain in the signal
     chain when connected inline.

2. Extend `ModuleGroup` (currently at `auto_layout.rs:59-64` with variants
   `SignalChain`, `Modulation`, `Global`, `Disconnected`) by adding an explicit
   variant for effect-chain modules:

   ```rust
   enum ModuleGroup {
       SignalChain,
       Modulation,
       Global,
       EffectChain,
       Disconnected,
   }
   ```

3. In `calculate_layout_with_chain_order()`:

   - place signal-chain columns first,
   - place modulation modules,
   - place effect-chain modules in one vertical column ordered by `effect_chain_order`,
   - place other global modules in a neighboring global column,
   - place disconnected modules after those columns.

4. Delete the `align_effect_chain()` call from `apply_auto_layout()`
   (`patch_editor.rs:4188-4189`).

   - Keep the function itself for explicit user-triggered chain reordering only.
   - Do not invoke it as a post-pass after auto-layout.

Acceptance criteria:

- Auto-layout result is self-contained: every final module position comes from the
  layout result.
- Effect-chain order is top-to-bottom stable.
- Disconnected modules cannot overlap with post-moved effects.
- Existing effect-chain visual behavior still works when the chain order changes
  manually (see Phase 10 for the related "first render after load" trap).

### Phase 4 - Give `available_rect` real semantics

Make `available_rect` mean what the API says.

Recommended approach:

1. In both `calculate_layout()` and `calculate_layout_with_chain_order()`:

   ```rust
   let start_x = snap_to_grid_up(available_rect.min.x + GRID);
   let start_y = snap_to_grid_up(available_rect.min.y + GRID);
   ```

   replacing the hardcoded constants at `auto_layout.rs:687-688`.

2. Decide and document width behavior:

   - Conservative first step: respect origin only, keep the existing left-to-right
     expansion.
   - Better step: wrap extra columns into a new band when x would exceed
     `available_rect.max.x`.

3. Rename `_available_rect` to `available_rect` in both signatures (`auto_layout.rs:562-568`,
   `575-580`) after it is used.

4. Add helper functions for grid snapping if needed:

   - snap position up,
   - snap size up,
   - maybe snap rect origin.

Acceptance criteria:

- Tests with non-zero rect origin pass.
- No compiler warnings about unused rect arguments.
- Layout still works for the current scroll rect behavior.

### Phase 5 - Handle collapsed groups deliberately

Choose one explicit behavior and implement it consistently.

Recommended behavior:

- If a group is collapsed, auto-layout treats the collapsed group box as a single
  visible layout node.
- The internal member panels keep relative positions inside the group and are not
  individually spread across the canvas while hidden.

Implementation options:

1. Add a `LayoutNode` layer in `PatchEditor::apply_auto_layout()` (`patch_editor.rs:4137`):

   ```rust
   enum LayoutNode {
       Module(ModuleId),
       CollapsedGroup(GroupId),
   }
   ```

2. Build `ModuleInfo` from visible modules plus collapsed group nodes.

3. For a collapsed group:

   - category can be derived from contained modules, or use a neutral
     signal-chain/global classification based on incoming/outgoing external
     connections.
   - size should be the existing `collapsed_group_size()` helper
     (`patch_editor.rs:63-83`).
   - connections should collapse member endpoints to the group node for external
     edges.

4. After layout:

   - move collapsed group `group.position`,
   - do not move hidden member modules independently (which is what happens today —
     `apply_auto_layout()` only iterates `self.panels`).

Alternative simpler behavior:

- Auto-layout ignores collapsed groups and hidden member modules completely.
- This is easier but less useful; only choose it if implementing group nodes becomes
  too large.

Acceptance criteria:

- Running auto-layout with a collapsed group does not move hidden modules out from
  under the group.
- The visible collapsed group box moves to a clean layout position.
- Expanding the group after auto-layout keeps member positions coherent.

### Phase 6 - Fix MCP request lifecycle

Avoid clearing MCP auto-layout requests unless the request is actually handled.

Recommended approach:

1. At `egui_backend.rs:676-680`, replace the unconditional
   `swap(false, Relaxed)` with a non-destructive load:

   - `load(Relaxed)` to check whether a request exists,
   - only `store(false, Relaxed)` after `patch_editor.apply_auto_layout(...)` runs.

2. The application site (`egui_backend.rs:2257-2262`) is the only place that should
   clear the flag.

3. If the app is not in `AppView::Rack` when the frame ticks, leave the flag set so
   the next Rack frame consumes it.

4. Consider exposing a clearer MCP response later
   (`synth_mcp/src/server.rs:2370-2375`):

   - current bridge response says "queued for next frame",
   - if requests can remain pending until Rack is active, document that.

Acceptance criteria:

- MCP auto-layout request made from another view is applied when Rack becomes active.
- No request is silently dropped.
- GUI menu behavior is unchanged.

### Phase 7 - Clean up naming and comments

After behavior is fixed:

- update comments in `auto_layout.rs` to match the final phases,
- remove stale "fixed estimated sizes" wording if actual rendered sizes are used,
- document how effect/global/disconnected zones are ordered,
- keep public API names consistent with actual behavior.

### Phase 8 - Persist and restore effect-chain order explicitly

Add an explicit saved effect-chain order so effect processing and visual ordering do
not depend on `HashMap` or JSON module order.

Recommended approach:

1. Add a field to `PatchSettings` (`patch.rs:538-576`):

   ```rust
   #[serde(default, skip_serializing_if = "Vec::is_empty")]
   pub effect_chain_order: Vec<String>
   ```

   Use strings for file stability and compatibility with the existing
   `ModuleState.id` format. Follow the same pattern as the existing
   `canvas_size: Option<CanvasSize>`.

2. During patch/project save, populate it from the engine's current effect-chain
   order. `InstrumentSnapshot.effect_chain_order: Vec<ModuleId>` already exists at
   `synth_engine/src/shared_state.rs:366`, so the save path can read it directly.

   - For project save, use the available instrument snapshots.
   - For single patch save, use the active instrument's engine snapshot.
   - Filter to modules that actually exist in the patch.

3. During load, the engine has no public "set chain order" command — only
   `EffectChain::add_effect()` which appends. Two viable strategies:

   - **(a) Add chain rebuild on load**: after all modules are added in `patch_bridge.rs:75-77`,
     re-read `effect_chain_order` and rebuild the engine chain to match. This is a
     one-time setup so the work can happen on the audio side via a new
     `EngineCommand::SetEffectChainOrder(Vec<ModuleId>)` or equivalent.
   - **(b) Add modules in saved order first**: iterate the saved
     `effect_chain_order` to add effects, then add the remaining (non-effect)
     modules. Simpler but depends on `add_effect()` keeping its "push to end"
     contract (`synth_engine/src/effect_chain.rs:212`).

   Strategy (a) is more robust against future engine changes. Use (b) only if (a) is
   too large for this pass.

4. Fall back to legacy behavior only when the field is missing (`#[serde(default)]`
   handles this).

5. Keep visualizers separate from effect processing order unless the engine
   intentionally includes them in `effect_chain_order`.

Acceptance criteria:

- Saving and loading a patch preserves effect processing order exactly.
- Re-saving a loaded patch does not randomly reorder effects in JSON.
- Legacy patches without the new field still load.

### Phase 9 - Make patch/module serialization order stable

Stabilize save output so JSON order is deterministic and cannot accidentally define
effect-chain semantics.

Recommended approach:

1. Change `create_patch_from_editor()` (`patch_bridge.rs:450`) to build a sorted
   module list.

2. Sorting rules:

   - modules in saved `effect_chain_order` should be emitted in that order or grouped
     consistently,
   - non-effect modules should sort by `module_sort_key` (`auto_layout.rs:66-68`) or
     by stable `ModuleId` string,
   - keep connections sorted by `(from_module, from_port, to_module, to_port)`,
   - keep groups already sorted by `GroupId`.

3. Avoid changing `PatchEditor::module_ids()` (`patch_editor.rs:4017-4018`) globally
   unless all callers want sorted order. Sorting locally in save code is lower risk.

Acceptance criteria:

- Repeated save without changes produces stable module/connection order.
- Effect-chain order no longer depends on `patch.modules` order.

### Phase 10 - Stop first-render effect realignment from overwriting loaded positions

Saved effect module positions should survive load unless the user explicitly runs
auto-layout or reorders the effect chain.

Recommended approach (primary):

1. After loading a patch/project (`egui_backend.rs:4789-4839` for `load_patch_data()`,
   `5329-5405` for `load_project_data()`), seed
   `PatchEditor::prev_effect_chain_order` (`patch_editor.rs:329`) with the loaded
   engine chain order before the first render. The first-frame comparison at
   `patch_editor.rs:1304-1306` will then see equal vectors and skip the realignment.

2. Coordinate this with Phase 3: auto-layout itself produces final effect positions,
   and the chain-change realignment in `show()` only runs when the user actually
   reorders effects.

3. Keep `align_effect_chain()` for explicit effect reorder actions if desired, but do
   not let it run merely because this is the first render after load.

Alternative considered:

- A `suppress_effect_chain_realign_once` flag on `PatchEditor`, set during load and
  consumed in `show()`. This works but adds another piece of single-frame state.
  Seeding `prev_effect_chain_order` is preferred because it uses the existing
  mechanism.

Acceptance criteria:

- A patch with manually placed effects loads with the same visual positions.
- Switching to another instrument for the first time does not move its loaded
  effects.
- Reordering effects intentionally still updates visual order according to the
  chosen UX.

### Phase 11 - Make collapsed group movement update member positions

Dragging a collapsed group should preserve the relationship between the visible group
box and its member modules.

Recommended approach:

1. In `draw_collapsed_groups()` at the position-update site
   (`patch_editor.rs:2990`), compute:

   ```rust
   let delta = new_group_position - old_group_position;
   ```

2. Apply the same delta to every member panel position in `group.members`.

3. Keep `group.position` updated to the new collapsed box position.

4. On expand, the group's bounds (`group_bounds_world()` at
   `patch_editor.rs:852-866`) will already be near the collapsed box because the
   member panels moved with it.

5. When building a group template from a collapsed group
   (`build_group_template()` at `patch_editor.rs:640-718`):

   - use the updated member positions after the drag fix, or
   - explicitly normalize from `group.position` if members are hidden.

Acceptance criteria:

- Dragging a collapsed group and expanding it preserves the moved location.
- Saving/loading a project with a moved collapsed group preserves both the collapsed
  location and the expanded member-module location.
- Saving a template from a moved collapsed group uses the moved internal layout.

### Phase 12 - Extend position readback suppression to group boxes

Make stale `egui::Area` memory unable to overwrite loaded collapsed-group positions.

Recommended approach:

1. In `draw_collapsed_groups()` at `patch_editor.rs:2986-2991`, guard group
   `area_rect` readback with the same `suppress_position_readback` flag used for
   module panels (`patch_editor.rs:2036-2043`).

2. The flag already covers the right window: `clear()` sets it true
   (`patch_editor.rs:440`), `show()` resets it at the end of the frame
   (`patch_editor.rs:2159`). Single-frame suppression matches the module path. If a
   future bug shows that one frame is not enough (e.g., because of split rendering
   passes), upgrade the bool to a small generation counter.

3. Add a regression test or debug assertion path if practical:

   - load patch with a collapsed group at a known position,
   - render one frame,
   - assert the group position remains unchanged.

Acceptance criteria:

- Collapsed group positions are not overwritten by stale Area memory after
  patch/project load.
- Module positions and group positions use the same load-safety policy.

## Suggested Final Verification

Run:

```bash
cargo fmt --check
cargo build
cargo clippy --all-targets
cargo test -p pertylizer gui::auto_layout
cargo test -p pertylizer
```

Manual GUI checks:

- simple oscillator -> filter -> amp -> output patch,
- patch with two oscillators into mixer/filter,
- patch with several envelopes/LFOs targeting different modules,
- patch with global effects reordered in the effect chain,
- patch with visualizers and Mod Matrix,
- patch with disconnected scratch modules,
- patch with a collapsed group,
- MCP `auto_layout` request while Rack is active,
- MCP `auto_layout` request while another view is active, then switch to Rack.
- save/load a patch with manually arranged effects and verify positions and order
  survive,
- save/load a project with multiple instruments containing effects, then switch
  between instruments,
- drag a collapsed group, save/load, expand it, and verify member modules remain at
  the moved location,
- save a group template from a moved collapsed group and insert it elsewhere.

## Definition of Done

- Auto-layout is deterministic for repeated runs on the same graph.
- No known overlaps between signal, modulation, effect/global, disconnected, and
  collapsed-group layout zones.
- Effect-chain order is represented directly by the layout result.
- `available_rect` has tested, documented behavior.
- MCP auto-layout requests are not silently lost.
- Patch/project save-load preserves module positions, collapsed group positions, and
  effect order.
- Group templates preserve relative module positions after collapsed-group movement.
- Regression tests cover all auto-layout and persistence findings.
