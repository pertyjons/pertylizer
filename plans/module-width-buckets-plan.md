# Module Width Buckets — Fixed S/M/L/XL widths for all modules

## Goal

Replace content-driven auto-sizing of patch-editor modules (and the mixer's
return-bus inserts) with **five fixed width buckets** declared per module type:
`ExtraSmall`, `Small`, `Medium`, `Large`, `ExtraLarge`. A module's width becomes a
deliberate, known-up-front property instead of an emergent side effect of its
widest body row.

### Why

1. **Unblocks the module-header right-align (§5B).** The open blocker recorded in
   memory is that patch modules are auto-sized `egui::Area`s, so anything that
   fills available width in the header feeds an unbounded grow loop. With the
   width fixed *before* the body renders, the header can `Atom::grow()`-truncate
   the title and right-pack the action icons safely. This refactor and that
   long-shelved task are the same lever.
2. **Calmer layout.** Uniform widths make auto-layout columns uniform and the
   canvas read as a tidy grid instead of ragged boxes.
3. **One meaning of "module width" everywhere.** The mixer already uses fixed
   widths (`STRIP_WIDTH=108`, `INSERT_WIDTH=200`); the patch editor is the
   outlier. Unifying inserts onto the bucket system makes the concept singular.
4. **Foundation for widget inheritance (later).** Once a module owns a width
   bucket, knobs/sliders/visualizers can derive their size from it. Deferred to a
   follow-up (see "Out of scope").

## Decisions (confirmed with user, 2026-06-25)

- **Width source:** an explicit field on `ModuleDescriptor` (synth_core), not a
  GUI-side classifier or per-category map. Per-module intent.
- **Scope:** *widths first*. Fix module/strip widths + the header right-align.
  Widget-size inheritance (knobs/scopes scaling to the bucket) is a later effort.
- **Mixer:** fold return-bus insert modules into the bucket system. Channel
  strips keep their own fixed fader width.

## Bucket sizes (grid-aligned)

Bucket value = **total module Area width** (== `panel_state.size.x`, what
auto-layout and grid-snapping consume).

**Grid mechanics (verified):** `GRID_SIZE = 32` (`patch_editor.rs:36`). Positions
snap to nearest 32 (`snap_to_grid`); sizes **ceil up to whole grid cells**
(`snap_size_to_grid = ceil(px/32)*32`, `auto_layout.rs:989`). Auto-layout column
width = `max(snapped width in column) + GAP`, where `GAP = GRID = 32`
(`auto_layout.rs:843–859`); column x-positions are cumulative from `start_x = 32`.

**Therefore every bucket must be an exact multiple of 32.** Then rendered width ==
snapped cell (no sub-grid gap), and since the gap is also 32, all column edges
land on the grid.

| Bucket       | Cells | Total px | Content px* | Intended use                                                                      |
|--------------|-------|----------|-------------|-----------------------------------------------------------------------------------|
| `ExtraSmall` | 4     | 128      | ~40         | signal monitor, tiny single-widget utilities                                      |
| `Small`      | 5     | 160      | ~72         | 1 knob / toggles only / one narrow slider                                         |
| `Medium`     | 7     | 224      | ~136        | the common case — oscillators, filters, LFOs, most FX (2 knobs/row)               |
| `Large`      | 10    | 320      | ~232        | a visualizer/editor body — envelope, scope, mod-matrix, sampler                   |
| `ExtraLarge` | 14    | 448      | ~360        | the widest content — full oscilloscope/spectrum, wave editor, MSEG, script editor |

\* Content px ≈ total − 2×port_col(28) − 2×item_spacing(8) − 2×inner_margin(8) =
total − 88. XL's ~360 covers the scope/spectrum clamp ceiling (340); Large's ~232
covers the envelope (≤250 today, now bounded). Mixer inserts have **no** port
columns, so their content px = bucket − 2×margin.

**ExtraSmall note:** the *inline* signal monitor (`inline_signal_monitor`, 100×50)
uses a bespoke compact path with 8 px ports, bypassing the normal frame — it stays
special-cased. ExtraSmall is the smallest *normal-frame* bucket (e.g. the full
signal-monitor module, currently clamped 120–300).

## Architecture

```
synth_core::module_traits
  enum ModuleWidth { ExtraSmall, Small, Medium, Large, ExtraLarge }  // Default = Medium
    // plain enum — NO serde derives (width is code-declared, never persisted)
    fn module_px(self) -> f32      // total Area width (multiple of 32)
    fn content_px(self) -> f32     // module_px minus chrome (ports/margins helper)
  struct ModuleDescriptor { ..., width: ModuleWidth }      // #[serde(skip)]
    fn width(self, ModuleWidth) -> Self                     // builder

gui/patch_editor.rs   — set Area/body width from descriptor.width (was content-driven)
gui/widgets/frame.rs  — draw_module_header right-aligns actions (now width is known)
gui/mixer_view.rs     — return-bus inserts use the bucket instead of INSERT_WIDTH
gui/auto_layout.rs    — unchanged logic; widths it reads are now uniform per type
```

## Phases

Each phase: edit → `/code-review` → full gate (`cargo fmt --check && cargo build
&& cargo clippy --all-targets && cargo test`) → commit. Branch
`feat/module-width-buckets`. Eyeball in-app after Phase 2 and Phase 3.
Squash-merge to `main` at the end per CLAUDE.md.

### Phase 0 — Foundation type (no behaviour change)

- Add `ModuleWidth` enum to `synth_core::module_traits` with `module_px()` /
  `content_px()`, `#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]` (Medium
  is `#[default]`), `#[must_use]`. **No serde derives** — width is a code-declared
  constant per module type, never serialized.
- Add `width: ModuleWidth` field to `ModuleDescriptor` (default Medium in `new()`),
  marked `#[serde(skip)]` so the existing `ModuleDescriptor` serde (MCP
  `get_module_type_info`) still compiles and round-trips, plus a `.width()` builder.
- Field is unused this phase. Gate green. Commit.

### Phase 1 — Classify module types (data only)

- Walk all ~66 `descriptor()` builders; add `.width(...)` only where ≠ Medium.
    - **Large/XL:** envelope (ADSR editor), oscilloscope, spectrum, level meter,
      signal monitor (full), mod matrix, sampler, MSEG, wave editor, script/YAMS.
    - **Small:** amplifier, output, simple utilities, single-control mixers.
    - **ExtraSmall:** genuinely tiny single-widget modules (e.g. the full signal
      monitor if it reads better compact; pure pass-through utils).
    - Everything else stays default Medium (no edit).
- Add a test: every registered module type resolves to a width, and every module
  with a bespoke wide body (envelope/scope/spectrum/mod-matrix/sampler/mseg/
  script) is `>= Large` — a guard so a wide UI can't regress into a narrow box.
- Still unused at render time. Commit (logical batches OK).

### Phase 2 — Apply fixed width in the patch editor

- In the module render (`patch_editor.rs` ~2357), drive the Area/body width from
  `descriptor.width.module_px()` instead of letting content size it.
- Replace `header_width = ui.min_rect().width()` (post-hoc) with the bucket width
  known up front.
- The three-column body: `content_min_w` → a **fixed** content width
  (`set_width`, not `set_min_width`) so IN | content | OUT exactly span the
  bucket. Knobs already wrap to available width; sliders fill; the bespoke
  visualizers already `clamp(min, available)` so they now fill the fixed content
  width cleanly.
- `panel_state.size.x` becomes deterministic (= bucket); height stays
  content-driven.
- Verify no overflow: long combo/dropdown text (truncate/ellipsis), long titles
  (handled in Phase 3), wide waveform selectors.
- Eyeball: every module category at its bucket.

### Phase 3 — Module-header right-align (the §5B payoff)

- Rework `draw_module_header` (`frame.rs`): now that the frame width is fixed,
  right-pack the trailing action icons (source/sink/automation/connectivity/
  close/menu) against the known right edge and let the title take the remaining
  space with `Atom::grow()` + ellipsis truncation.
- This is the top-bar tidiness the request is really after. Eyeball with the
  widest action set (a source+automated+connected module) and a very long name.

### Phase 4 — Mixer unification

- Replace `INSERT_WIDTH = 200` with the Medium bucket (no-port content width) for
  return-bus inserts; the embedded `draw_parameter_grid` renders at that content
  width. Channel strips keep `STRIP_WIDTH`.
- Confirm collapsed vs. with-inserts column width still reads correctly.

### Phase 5 — Groups

- Give module groups a deterministic width consistent with the scheme:
  `collapsed_group_size` and the expanded-group body (~line 4083) derive width
  from a bucket (Large default, or max of member buckets) instead of ad-hoc
  title/port math.

### Phase 6 — Auto-layout + cleanup

- `auto_layout.rs` already reads `.size`; with uniform per-type widths the
  columns become even. Eyeball a busy patch; tune column gap if needed.
- Update `ModulePanelState::new` default size to a bucket value (was 250×200).
- Remove now-dead content-width-derivation code paths.

### Phase 7 — (dropped)

Width is not serialized (`#[serde(skip)]`), so it is intentionally absent from the
MCP `get_module_type_info` JSON — it is a pure GUI layout concern with no external
consumer. No MCP work needed.

## Out of scope (explicit follow-ups)

- **Widget-size inheritance** — knobs/sliders/visualizers scaling to the bucket.
  Seam: thread `ModuleWidth` into `draw_parameter_grid` / `draw_knobs` later.
- Channel-strip width bucketing (strips are a fader UI, not a content module).
- Theme `Sizes` consolidation (scope/adsr fixed sizes) — revisit once widgets
  inherit the bucket.

## Risks & mitigations

- **Content wider than bucket → overflow.** Mitigated: knobs wrap, sliders/combos
  fill, visualizers clamp to available; Phase 1 guards wide UIs to ≥Large; long
  titles truncate in Phase 3. Eyeball is the backstop.
- **Saved projects.** No save-format change: projects persist module *instances*
  (type_id + params), never descriptors; sizes are recomputed each frame from the
  code-built descriptor. Width is `#[serde(skip)]` — nothing to migrate.
- **Auto-layout regressions.** Logic unchanged; inputs become uniform. Eyeball a
  dense patch in Phase 6.

```
