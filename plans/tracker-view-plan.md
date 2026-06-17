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

The T3 follow-ups and the T4 polish below are **all shipped** (one commit each;
each step gate-checked + code-reviewed).

---

## Done

### T3 follow-ups

- [x] **Cell-level tooltips** — voice cells show pitch / velocity / true start tick
  (off-grid flagged) / duration / legato / glide; ornament cells show the full
  config (`ornament_detail`).
- [x] **Group the flat tracker fields into a `TrackerViewState` sub-struct** — the
  five `tracker_*` fields now live on a single `tracker` field.

> **Dropped:** in-grid NoteProcessor *rack management* (add/remove/configure
> processors as tracker columns). Superseded — the **Note FX rack panel (NP6.1)**
> is now available alongside the tracker in the Pattern view, so managing the rack
> in-grid is moot. The tracker keeps the read-only NP-stage output columns.

### T4 — UX & visual polish

- [x] **Vertical zoom-Y controls in the toolbar** — `+` / `1x` / `-` driving
  `pr_zoom_y` (×/÷1.2, clamped [0.5, 3.0]).
- [x] **Dynamic font scaling** — monospace cell text scales with `pr_zoom_y`,
  clamped [9, 20].
- [x] **Context menus for cells and column headers** — voice (delete / legato /
  glide), ornament (edit / clear), expression (clear field), automation (clear
  point), automation header (delete lane). _Voice-header bulk-clear was
  intentionally skipped — awkward multi-note undo; the toolbar Clean + per-cell
  delete cover it._
- [x] **Row hover highlight** — faint tint behind the row under the pointer.
- [x] **Bar separator lines** — a divider at the top edge of each bar-start row.
- [x] **Focus-state cursor indicator** — dimmed cursor outline when another widget
  holds keyboard focus.
- [x] **Info/help popup** — a `?` toolbar button with a keyboard/mouse cheat-sheet.

---

## Cross-cutting principles (still apply)

- **No second source of truth.** The tracker reads and writes the *same* `Pattern`
  notes + automation + ornaments as the piano roll — a different lens, not a
  different store; toggling back and forth must show identical content.
- **Off-grid honesty.** Never silently hide or mangle off-grid notes — always mark
  them (`~` glyph + hover with the true tick) and preserve the real tick.
