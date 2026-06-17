# Plan: Tracker view (toggleable, per-pattern)

A vertical tracker-style editor for a single pattern, toggled against the piano
roll inside the Pattern view (rows = time steps; columns = voice / ornament /
expression / automation / read-only NoteProcessor-output lanes, with automation
curves painted behind the grid).

**T1–T3 are shipped and merged to main** — read-only render (T1), editing +
column management (T2), and the future column types (T3): NoteExpression
sub-columns, read-only NoteProcessor output columns, and the per-note ornament
column. All tracker code lives in
`crates/pertylizer/src/gui/sequencer/tracker.rs` (+ the shared snapshot in
`sequencer/mod.rs` and the ornament popup in `sequencer/ornament.rs`).

What remains is optional follow-ups and polish.

---

## Remaining

### T3 follow-ups (deferred, not blocking)

- [ ] **Cell-level tooltips.** Header tooltips already cover every column; per-cell
  tooltips (e.g. the true tick of an off-grid note, the full ornament config) are
  not added.
- [ ] **Group the flat tracker fields into a `TrackerViewState` sub-struct.**
  `tracker_cursor` / `tracker_value_buffer` / `tracker_voice_columns` /
  `tracker_show_expression` (and the ornament-edit state) live flat on
  `SequencerViewState`; a review suggested grouping them. Pure cleanup.

> **Dropped:** in-grid NoteProcessor *rack management* (add/remove/configure
> processors as tracker columns). Superseded — the **Note FX rack panel (NP6.1)**
> is now available alongside the tracker in the Pattern view, so managing the rack
> in-grid is moot. The tracker keeps the read-only NP-stage output columns.

### T4 — UX & visual polish

- [ ] **Vertical zoom-Y controls in the toolbar** — `+`, `1x`, `-` small buttons
  (right-aligned, second toolbar row) driving `view_state.pr_zoom_y` directly.
- [ ] **Dynamic font scaling** — scale cell text size with `pr_zoom_y`
  (e.g. `(11.0 * pr_zoom_y).clamp(9.0, 20.0)`) so text doesn't clip/vanish on zoom.
- [ ] **Context menus for cells and column headers** (`.context_menu`):
  - Voice cells: delete, toggle legato/glide.
  - Expression / automation cells: set / clear values.
  - Column headers: delete column, clear column data.
- [ ] **Row hover highlight** — subtle background tint behind the row under the
  pointer, to ease horizontal scanning across many columns.
- [ ] **Bar separator lines** — a solid divider under a row that starts a new bar
  (`row_tick.is_multiple_of(ticks_per_beat * 4)`).
- [ ] **Focus-state cursor indicator** — dim/dash the cursor outline when the
  tracker lacks keyboard focus (`ui.memory(|m| m.has_focus(table_id))`).
- [ ] **Info/help popup** — a `❓` toolbar button with a cheat-sheet of keyboard
  navigation, note entry, expression/ornament editing, and automation.

---

## Cross-cutting principles (still apply)

- **No second source of truth.** The tracker reads and writes the *same* `Pattern`
  notes + automation + ornaments as the piano roll — a different lens, not a
  different store; toggling back and forth must show identical content.
- **Off-grid honesty.** Never silently hide or mangle off-grid notes — always mark
  them (`~` glyph + hover with the true tick) and preserve the real tick.
