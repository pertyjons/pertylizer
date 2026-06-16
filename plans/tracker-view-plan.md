# Plan: Tracker view (toggleable, per-pattern)

A second, **vertical tracker-style editor** for a single pattern, toggleable against
the existing horizontal piano roll **inside the Pattern view**. Rows = time steps,
columns = voice/automation/(future) processor lanes. Goal: a dense numeric overview
of notes *and* parameter values per step, with the **automation curve drawn behind
the grid** so you see discrete values and the continuous shape at once.

Scope locked with the user 2026-06-03:
- **Per-pattern only.** Not an arrangement-wide / sequencer tracker ("blir för stort,
  lättare att överklicka").
- **Columns are polyphony/voice lanes feeding the pattern's ONE instrument** — *not*
  multi-instrument. A pattern is instrument-agnostic content today; the placement's
  track sets the instrument and per-note instrument is not consulted at playback
  ("Phase 4"). No data-model change to multi-instrument.
- Column types: **note/voice lanes** + **automation lanes** + (later, when they land)
  **NoteProcessor** lanes and **NoteExpression** sub-columns.
- Must be able to **add new columns** and **remove empty columns** (both note lanes
  and automation/other lanes).

---

## Verified engine/GUI state (confirmed against code 2026-06-03)

- **Clean toggle point.** `draw_pattern_view` (`crates/pertylizer/src/gui/pattern_view.rs`)
  is a pattern browser that, when a pattern is open, calls the shared
  `draw_piano_roll(...)` (`pattern_view.rs:167`) with a snapshot from
  `collect_piano_roll_data(song, pattern_id)` (`:143`). The toggle is a single branch
  there: `match editor_mode { PianoRoll => draw_piano_roll(...), Tracker => draw_tracker(...) }`.
- **Shared snapshot is reusable as-is.** `PianoRollData` (`gui/sequencer/mod.rs:2857`)
  already carries `ticks_per_row` (`:2862`), `notes`, `automation_lanes`, `time_sig`,
  `pitch_min/max`. The tracker eats the same snapshot — no new collection path.
- **Row/step model already exists in the data.** `RowResolution` on `Pattern` with
  `row_to_tick` (`pattern.rs:67`), `tick_to_row` (`:72`), `quantize` (`:77`),
  `RowIndex` newtype. ticks↔rows mapping is built-in.
- **Automation is trivial to render.** `AutomationLane` (`automation.rs:144`) with
  `AutomationPoint { tick, value, curve }` (`:66`), `CurveType` =
  Linear/Step/Exponential/SCurve (`:92`), and `value_at(tick)` already interpolates
  (`:186`). The piano roll already draws the curve pixel-by-pixel via `value_at`
  (`gui/sequencer/mod.rs` automation zone ~6164–6314).
- **Step-entry stub exists.** `step_entry_mode` / `step_cursor_tick`
  (`gui/sequencer/mod.rs:165,167`) + keyboard advance/commit (~5516–5560) — the
  natural input model for T2.
- **Rendering is immediate-mode egui `Painter`** (rects/lines/circles/text); all
  primitives the tracker needs already in use.
- **View mode storage:** add an `editor_mode: PatternEditorMode { PianoRoll, Tracker }`
  to `PatternViewState` (`pattern_view.rs:40`) — a per-pattern-view sub-mode, NOT a new
  top-level `AppView`.

---

## The one open design decision — RESOLVED 2026-06-12 (commit `85008b9`)

**How do free-tick, possibly-overlapping notes map to stable voice columns?**
Notes are a `Vec<Note>` with `start` + `duration` and had no stored voice/lane index.

- **T1 (read-only) — SHIPPED:** lanes computed on the fly with greedy interval coloring
  (`assign_voice_lanes` in `tracker.rs`, assigns each note the lowest column free at its
  start tick). No model change; validates the view.
- **T2 (editing + add/remove columns) — RESOLVED as recommended:** the lane index is now
  **stored on `Note`** — a `NoteLane` newtype (`synth_sequencer/src/ids.rs`, next to
  `TrackIndex`) + an additive `lane: NoteLane` field on `Note` (`#[serde(default)]` → 0,
  `with_lane` builder; piano roll ignores it; `project.schema.json` regenerated). This
  makes add/remove-column a real, persisted operation and stops notes reshuffling between
  columns. **The greedy fallback still drives the read-only render; T2 editing must switch
  the lane assignment over to the stored `Note.lane`.**

---

## Rendering approach — LOCKED 2026-06-03: `egui_extras::TableBuilder`

Chosen with the user over raw `Painter`: lean on the table's automatic features
(row **virtualization** via `body.rows`, column **resize**, sticky **header**,
**scroll**) and keep cells as **standard widgets** (`ui.label`/small widgets). The
automation **curve-behind-grid** is done as a **painted overlay** — experiment with
alignment (see risk below). egui/egui_extras are at **0.34.3**; `egui_extras` is
already a dependency (currently only for image loaders), so this is the first real
`TableBuilder` use in the codebase.

**Build notes (egui_extras 0.34 API):**
- `TableBuilder::new(ui).striped(true).resizable(true).cell_layout(...)` then one
  `Column` per lane: `Column::auto()` for the row/time gutter,
  `Column::initial(w).resizable(true)` (or `Column::remainder()`) per
  voice/automation column. Columns are declared **per frame**, so **add/remove
  column = change the column list** — fits T2 cheaply.
- `.header(row_h, |mut header| header.col(|ui| { ui.label(lane_name) }))` for labels.
- `.body(|mut body| body.rows(row_h, n_rows, |mut row| { row.col(|ui| { … }) }))` —
  **uniform fixed row height matches a tracker exactly** and only visible rows render.
- Cells: note name / value via `ui.label`; off-grid + expression markers as small
  glyphs/`RichText`; tooltip carries the true tick.

**Risk to watch (the experimental bit):** `TableBuilder` clips per cell and
virtualizes rows, so a **continuous curve spanning rows** has no single canvas. Plan:
read the table's used rect (wrap the table and read the parent `ui.min_rect`, or use
the returned response), then paint the curve into an aligned layer over each
automation column's x-range, mapping `tick → y` from `(first_visible_row, row_h)`.
If aligning the overlay to the virtualized geometry proves too fiddly, fallbacks in
order: (1) draw the curve as **per-cell segments** inside each `row.col` (no overlay
alignment needed), (2) revisit raw `Painter` for the automation columns only. Log
which path T1 actually used.

---

## Status at a glance

- [x] **T1** — Read-only tracker render (MVP): toggle + note lanes + automation lanes
  with per-row values **and curve-behind-grid** + off-grid markers. No editing.
  IMPLEMENTED 2026-06-03.
- [x] **T2** — Editing + column management. SHIPPED 2026-06-13 over scaffold `85008b9`:
  stored-lane display (`f180be7`), step-entry note input + delete (`aa4d6b3`), numeric
  automation entry + delete point (`e7bcb5e`), voice column add/clean (`e3d53de`),
  automation column add/clean via the shared target picker (`9c9c81c`), and auto-follow
  scroll-away. All writes go through the pattern mutators with `UndoAction`, quantized to
  the row grid.
- [x] **T3** — Future column types. SHIPPED 2026-06-13 on branch `feat/tracker-t3`
  (merged to main): NoteExpression sub-columns (`3f0a858`, `76e0092`) + read-only
  NoteProcessor output columns, **one per processor stage** (`64847fc`, `e736ba6`),
  plus Expr-on-by-default + per-column header tooltips (`a8c6434`). Only the optional
  NP **rack-management** (add/remove/configure from the tracker) is left, deferred —
  see T3 section.

Build order is value-first: **T1 ships the overview you asked for with zero model
change.** Stop after T1 if that's enough; T2/T3 as appetite allows.

---

## T1 — Read-only tracker render (MVP) — IMPLEMENTED 2026-06-03 (full gate green)

The headline deliverable: see all voice lanes, all automation values per step, and the
automation curves behind the grid — without touching any editing logic.

- [x] `PatternEditorMode` enum + Piano roll / Tracker toggle (`selectable_value`) in the
  pattern-view CentralPanel. Persisted on `PatternViewState.editor_mode`
  (`pattern_view.rs`).
- [x] `draw_tracker(ui, data, playhead_tick, view_state)` in
  `crates/pertylizer/src/gui/sequencer/tracker.rs` (child module of `sequencer`, so it
  reads the private `PianoRollData` snapshot directly). Built with
  `egui_extras::TableBuilder`.
- [x] **Columns (declared per frame):** `Column::auto()` row/time gutter; one
  `Column::initial(72)` per **voice lane** (greedy interval-coloring assignment,
  read-only); one `Column::initial(80)` per **automation lane**. Header row carries
  `V1..Vn` + `target.display_name()`.
- [x] **Rows:** `body.rows(row_h, n_rows, …)` — virtualized, uniform height
  (`TRACKER_ROW_HEIGHT * pr_zoom_y`).
- [x] **Note cells:** note name (`Pitch` Display), 2-digit velocity, legato/glide/
  expression markers, empty step = `·`.
- [x] **Automation columns:** per-row numeric value via a local `sample_at` that mirrors
  `AutomationLane::value_at` and reuses `CurveType::interpolate` (exact curve shape).
  **Curve rendered as PER-CELL SEGMENTS** (the plan's fallback 1), value→x within the
  cell, top-tick→bottom-tick; adjacent cells share edges so the curve is continuous.
  The single-overlay path was *not* used in T1 — per-cell is robust under
  virtualization and needs no geometry alignment. Revisit a true overlay only if a
  smoother/anti-aliased curve is wanted.
- [x] **Off-grid markers:** `~` glyph + hover with the true tick when
  `start_tick % ticks_per_row != 0`.
- [x] **Playhead** marker (`▶` + accent) on the matching gutter row.
- [x] **Shared toolbar row** (user feedback 2026-06-03): extracted the piano-roll's
  instrument selector + "track plays" badge + mini-transport (play/pause/stop/record/
  solo) into `draw_pattern_instrument_transport` in `sequencer/mod.rs`; both the piano
  roll and the tracker call it (no duplication of the recording-arm logic). Tracker
  header row = pattern name + that shared control.
- [x] **Auto-follow playhead** (user feedback): `TableBuilder::scroll_to_row(playhead,
  Center)` when playing + `auto_follow_playhead` (guarded `prow < n_rows`); repaint
  requested each frame while playing.
  - **Known divergence (deferred to T2 polish):** unlike the piano roll, the tracker
    does not yet detect manual scroll-away to break follow — it re-centers every frame
    while playing (lock-follow). Acceptable for T1; add scroll-offset detection +
    `auto_follow_playhead` disable mirroring the piano roll when editing lands.
- [x] **Visual confirmation in the running app** — re-verified BOTH the piano roll
  (toolbar unchanged after extraction) AND the tracker (toolbar + auto-follow). T1 is
  fully shipped; the read-only render then got the view-adapter refactor + cursor
  scaffold in `85008b9` (see T2).

**Bonus:** the Rust 1.96 `clippy::manual_is_multiple_of` lint flagged the `% == 0`
beat/off-grid checks → switched to `.is_multiple_of()`.

**Acceptance:** open a pattern with a chord + at least one automation lane, toggle to
tracker, and see every voice in its own column, per-row automation values, and the
curve behind the automation column. No edits possible yet; piano roll unchanged.

---

## T2 — Editing + column management

Lane-storage decision RESOLVED: `NoteLane` is stored on `Note` (see above).

**Scaffold already landed (`85008b9`, read-only):**
- [x] View-adapter refactor — `TrackerColors` palette snapshotted once per frame +
  pure `Cow` text helpers; cells no longer format inline or take a per-cell theme lock.
- [x] Cursor model — `TrackerCursor` (row + flat column index) + `TrackerColumn`
  resolution on `SequencerViewState`; arrow/Page/Home/End nav, click-to-place,
  row/cell/column-header highlight. `handle_tracker_keys` in `tracker.rs`.
- [x] `NoteLane` newtype (`ids.rs`) + additive `lane: NoteLane` on `Note`
  (`#[serde(default)]`, `with_lane`); `project.schema.json` regenerated.

**Editing — SHIPPED 2026-06-13:**
- [x] Display by stored `Note.lane` (`f180be7`): lane-organized patterns render by the
  stored lane; legacy all-zero patterns keep the greedy fallback; the first lane-assigning
  edit migrates greedy → stored in the same undo step. Multiple notes in one (row, lane)
  show a `+N` overflow marker rather than hiding any.
- [x] **Add note column** / **remove empty (Clean)** (`e3d53de`): `tracker_voice_columns`
  minimum (capped at 32); Clean compacts stored lanes densely via one `SetLaneBatch`.
- [x] **Add automation column** (shared target picker → `get_or_create_automation`) /
  **remove empty automation lanes** via the same **Clean** (`9c9c81c`); new
  `AddAutomationLane`/`RemoveAutomationLane` undo actions; Clean bundles every removal +
  the note compaction into one undo step.
- [x] **Step-entry note input + delete** (`aa4d6b3`): a computer-keyboard piano key
  inserts a note at the cursor row on its voice lane (quantized to `row * tpr`),
  Delete/Backspace removes it; no-op on an occupied cell (re-type = delete-then-enter).
- [x] **Numeric automation entry + delete point** (`e7bcb5e`): type digits/`.` in an
  automation cell (caret shown), Enter writes a point at `row_to_tick(row)` (clamped 0..1),
  Delete removes it; replacing a point keeps its curve via `MoveAutomationPoint` undo.
- [x] All writes go through the pattern mutators with `UndoAction`, so **undo/redo works**
  and the audio engine (live `Arc<RwLock<Song>>` reader) picks edits up next tick. Tracker
  writes are row-quantized while the piano roll keeps free placement.
- [x] Auto-follow scroll-away: manual wheel scroll during playback breaks lock-follow
  (keyed off scroll input, since `TableBuilder` hides its offset); re-enabled on Play.

**Acceptance:** build a small pattern entirely in the tracker (notes + an automation
lane), add and remove a column, prune empties, undo/redo each step, and confirm it
plays identically and round-trips `save_project`/`load_project`.

---

## T3 — Future column types

Branch `feat/tracker-t3`. Scope confirmed with the user 2026-06-13: expression
sub-columns first (per-field, behind a toggle), NP lanes after / as appetite allows.

- [x] **NoteExpression sub-columns** — SHIPPED 2026-06-13 (`3f0a858` display, `76e0092`
  editing). An **"Expr" toggle** (on by default since `a8c6434`) interleaves four
  narrow per-note columns (**Acc/Gat/Gho/Prb**) after each voice column. New `ExprField`
  + `TrackerColumn::Expr(lane, field)`; the flat selectable-column layout became
  contiguous per-voice groups of `cols_per_voice` (1 or 5) followed by automation
  lanes (one `voice_group_base` helper shared by decode + encode). Accent/Gate/
  Probability edit via the shared digit buffer (×100 display, `EXPR_DISPLAY_SCALE`);
  Ghost is an Enter/Space flag; Delete clears; writes go through
  `set_note_expression` (empty → `None`) with `SetExpressionBatch` undo. **Vibrato is
  excluded** from the sub-columns (too rich for one cell) — the note cell keeps its
  `•` marker for it.

- [x] **NoteProcessor lanes (read-only contribution)** — SHIPPED 2026-06-13
  (`64847fc`, refined `e736ba6`). **One non-selectable column per processor** in the
  rack (shown only when the rack is non-empty), headed by the processor kind
  (`scale_quantize`/`chord`/`arp`/`humanize`) and showing the rack's **cumulative**
  expansion *after* that stage — so comparing adjacent columns reveals each
  processor's contribution. Computed offline via the new `pub
  Pattern::expand_at_tick_through(through)` (`expand_at_tick` delegates with
  `through = rack length`, so the audio thread is unchanged); the only other needed
  API (`processors()`/`ExpansionBuffer`) was already public. Row-resolution: samples
  at each row tick, so sub-row generated events aren't shown (documented). User chose
  "read-only contribution first".
- [ ] **NoteProcessor rack management (optional, not done)** — the heavier half of the
  original bullet: add/remove/configure processors from the tracker ("add/remove
  mirrors T2"). Needs a rack-management UX for *configured* processors (each kind has
  its own config). Deferred — pursue only if the read-only view proves worth editing
  in-place rather than via the existing NP rack UI.

**Known follow-ups (not blocking; flagged by code review):**
- Header tooltips cover every column (`a8c6434`); cell-level tooltips are not added.
- The four tracker fields (`tracker_cursor` / `tracker_value_buffer` /
  `tracker_voice_columns` / `tracker_show_expression`) live flat on
  `SequencerViewState`; reviews suggest grouping them into a `TrackerViewState`
  sub-struct (pure cleanup, deferred).

---

## T4 — UX & Visual Polish (Proposed Improvements)

- [ ] **Vertical Zoom Y Controls in Toolbar** — Add `+`, `1x`, `-` small buttons right-aligned on the second toolbar row of the tracker to modify `view_state.pr_zoom_y` directly.
- [ ] **Dynamic Font Scaling** — Scale cell text `font_size` proportionally to `view_state.pr_zoom_y` (e.g., `(11.0 * pr_zoom_y).clamp(9.0, 20.0)`) to prevent text clipping or disappearing on zoom.
- [ ] **Context Menus for Cells and Column Headers** — Add egui `.context_menu(|ui| { ... })` on responses:
  - Voice cells: Delete, toggle legato/glide.
  - Expression cells: Set/clear values.
  - Automation cells: Set/clear values.
  - Column headers: Delete column, clear column data.
- [ ] **Row Hover Highlight** — Draw a very subtle background tint behind the row currently under the mouse pointer to ease horizontal coordinate scanning across many columns.
- [ ] **Takt/Bar Separator Lines** — Draw a solid horizontal divider line under the row (e.g. using `colors.row_dim`) whenever a row starts a new bar (e.g., `row_tick.is_multiple_of(ticks_per_beat * 4)`).
- [ ] **Focus State Cursor Indicator** — Check if the tracker has keyboard focus using `ui.memory(|m| m.has_focus(table_id))` (and requesting focus on click/navigation). Draw the cursor outline as dashed or a dimmer color when unfocused.
- [ ] **Info/Help Icon Popup** — Add a `❓` small button (using `ri::INFORMATION_LINE` or similar) in the toolbar displaying a clean cheat-sheet popup of keyboard navigation, note entry, expression editing, and automation.

---

## Cross-cutting

- **No second source of truth.** The tracker reads and writes the *same* `Pattern`
  notes + automation as the piano roll. It is a different lens, not a different store;
  toggling back and forth must show identical content.
- **Off-grid honesty.** Never silently hide or mangle off-grid notes — always mark
  them and preserve the true tick.
- **Maintenance cost.** Two view modes. T1 is pure rendering (no editing duplication);
  the editing/hit-test cost only arrives in T2.

## Critical files

| Concern | File |
|---|---|
| Toggle point + pattern-view host | `crates/pertylizer/src/gui/pattern_view.rs:143,167` (`draw_pattern_view`, `PatternViewState:40`) |
| Shared snapshot (reused as-is) | `crates/pertylizer/src/gui/sequencer/mod.rs:2857` (`PianoRollData`, `ticks_per_row:2862`) |
| New tracker renderer | `crates/pertylizer/src/gui/sequencer/tracker.rs` (new) — `egui_extras::TableBuilder` |
| Table component (egui 0.34.3, already a dep) | `egui_extras::TableBuilder` / `Column` / `body.rows` |
| Row/step mapping | `crates/synth_sequencer/src/pattern.rs:67` (`row_to_tick`/`tick_to_row`/`quantize`, `RowResolution`) |
| Automation value + curve | `crates/synth_sequencer/src/automation.rs:144,186` (`AutomationLane`, `value_at`); piano-roll automation zone `gui/sequencer/mod.rs` ~6164 |
| Step-entry stub to reuse | `crates/pertylizer/src/gui/sequencer/mod.rs:165,167` + ~5516 |
| Lane index (T2, if stored) | `crates/synth_sequencer/src/note.rs:158` (`Note`) |
