# Plan: Note Processors GUI (NP6)

The one remaining slice of the Note Processors feature (`plans/note-processors-plan.md`).
Everything else — the engine, every processor, the MCP surface, and save/load
persistence — shipped in v0.311.0. NP6 is the human-facing UI layer, deferred
because it is interactive egui work that benefits from visual iteration and is
**not headless-testable** (no GUI test harness in this project).

NP6 surfaces two already-built data structures into the GUI:

1. **The per-pattern processor rack** — `Pattern.processors: Vec<NoteProcessor>`
   (`crates/synth_sequencer/src/pattern.rs:122`, `pub(crate)`), 4 processor kinds
   in a locked chain order (`ScaleQuantize → Chord → Arpeggiator → Humanize`).
2. **The per-note ornament** — `Note.ornament: Option<Ornament>`
   (`crates/synth_sequencer/src/note.rs:220`), the timed-repeat figure
   (flam / drag / ruff / roll / grace).

Nothing in the engine or data model needs to change — NP6 is read/write plumbing
onto existing types via their public accessors, plus one visual aid (ghost-note
preview in the piano roll).

## The governing decision: tracker edits, piano roll shows

The sequencer has **two pattern editors** sharing the same central area: the
**tracker** (grid/column, the more specialized editor) and the **piano roll**
(freeform). Note processors are step/grid concepts (arps, rolls, scale-quantize),
so the **tracker is the primary editing surface** and the **piano roll is mostly
a visualization** — with one light shortcut so a piano-roll user is never stranded.

This plays to each view's grain *and* is less to build: the tracker already ships
read-only NoteProcessor output columns (T3, `64847fc`/`e736ba6`), and its per-note
sub-column machinery (`ExprField`, `edit_expression_cell`) is exactly what an
ornament editor needs — so we reuse it instead of building a bespoke piano-roll
right-click editor.

| | Tracker (primary editor) | Piano roll (view + shortcut) |
|---|---|---|
| **Rack** | the shared Note FX panel + its existing read-only stage columns | the **same** shared panel + ghost-note preview |
| **Ornament** | native ornament **sub-column** (reuses the expression-cell flow) | read-only glyph + ghost notes + an "Edit…" shortcut that opens the **shared** ornament popup |

The **rack inspector panel** and the **ornament editor popup** are each built
**once** and shared by both views — there is only ever one editing code path per
feature, just different affordances to reach it.

## Status at a glance

- [x] **NP6.1** — The shared **Note FX rack inspector**: a right-docked panel to add / remove / configure processors (+ freeze), available beside *either* inner editor. Closes the deferred tracker "rack management" item. **Shipped** — panel + toggle (`1da8093`), Pattern-view wiring (`975a69f`), per-type param widgets + coalesced undo (`856048b`), undoable freeze via full-pattern snapshot. Freeze gate relaxed to `rack OR any count≥2 ornament`.
- [x] **NP6.2** — **Ornament editing in the tracker**: an always-present "Orn" sub-column per voice lane (`47089d7`, prep `0cb2585`) — compact tag, Enter opens the shared popup, Delete clears. `TrackerColumn::Ornament`, `cols_per_voice = 2 + optional EXPR_FIELDS`. Earlier deferral note (kept for context) — the shared ornament-editor popup *shipped* (in NP6.3) and ornament editing works via the piano-roll inspector; the tracker also already *shows* ornament effects through its read-only NP-stage columns (the expansion includes ornament hits). The remaining piece — a **selectable** ornament sub-column wired into the tracker — requires extending the tracker's `TrackerColumn`/`cols_per_voice`/`resolved()` cursor model and click/render paths, the codebase's most fragile, test-free UI. That is exactly the "interactive egui work that benefits from visual iteration" NP6 was deferred for, so it is left for an in-app session with the user rather than an autonomous pass. Reuse `draw_ornament_editor` (move `draw_ornament_popup` from `piano_roll.rs` to `ornament.rs` to share it).
- [x] **NP6.3** — **Piano-roll surface**: ghost-note preview of the expansion (`0e4d169`), a read-only ornament glyph on notes (`ef64655`), and the shared ornament-editor popup + an "Edit…" shortcut in the selection inspector (`d49ec7a`).

Build order: NP6.1 (the headline) → NP6.3 (piano-roll editing + visuals) → NP6.2
(tracker editing column, deferred for an interactive session). Each step was its
own commit + `/code-review --fix`.

---

## Decisions locked

- **No per-processor on/off (bypass) flag.** Considered (a `⦿` toggle like the
  effect chain), **rejected for v1.** It would need a new `enabled: bool` on each
  config struct *and* a two-sited skip in the **locked RT expand path**: both the
  buf-side processor loop (`note_processor.rs:1379`, `process_at_tick`) **and** the
  held-view chain (`expand_pitch`/`expand_held_pitch:998`, which the arp and strum
  read upstream through) — skipping only the loop would still feed a "disabled"
  chord's tones to a downstream arp. Plus a `Default`-true gotcha (`ScaleQuantize`
  derives `Default`, so a bare `bool` would default `false`), a schema regen, and
  a new engine test. **To bypass a processor, remove it and re-add it.**
  `add_processor` re-inserts at the canonical stage position, so re-adding is one
  click and order is automatic.
- **Placement: a right-docked `egui::SidePanel::right`** inside the sequencer
  central area, toggled by a `Note FX (n)` button (badge `n =
  pattern.processors().len()`). It sits beside *whichever* inner editor is active
  (tracker or piano roll), so the rack is editable from both contexts without a
  second widget. Chosen over a floating `egui::Window` (covers the notes) and an
  inline fold under the pattern header (steals vertical grid height).
- **No drag-reorder.** The rack self-orders by `NoteProcessor::chain_stage()`
  (`note_processor.rs:793`); the order is *locked*. "+ Add" only offers stages not
  already present (at most one of each kind). This deliberately differs from the
  effect chain (which is drag-reorderable).
- **Hand-rolled param rows, not `param_grid.rs`.** `draw_parameter_grid` is
  descriptor-driven (reads a `ModuleDescriptor`); note processors are serde enums,
  not modules, so there is no descriptor. With 4 fixed types the param rows are
  written by hand, **reusing the visual primitives** (`ModuleFrame`,
  `draw_module_header`, the knob widget, `egui::ComboBox`) but not the descriptor
  machinery. Do *not* synthesize fake `ModuleDescriptor`s.

---

## Context — verified API & GUI conventions

Confirmed against live code during the NP6 design pass.

**Pattern rack API (all public, used from the GUI crate — the field is `pub(crate)`):**
- `processors() -> &[NoteProcessor]` (`note_processor.rs:1287`) — read for rendering.
- `add_processor(p) -> usize` (`:1293`) — inserts at the canonical stage position.
- `remove_processor(index) -> Option<NoteProcessor>` (`:1303`).
- `set_processor(index, p) -> bool` (`:1311`) — **replace in place**, position
  preserved. Param edits replace a processor with the same *kind* (same stage), so
  this is the edit path; it does not reorder. Replacing with a different stage is
  caller-forbidden (use remove + add) — the GUI never does that.
- `freeze_processors() -> usize` (`:1391`) — UI-thread-only one-shot bake to plain
  notes (drops ornaments + clears the rack). Already exists.
- `expand_at_tick` / `expand_at_tick_through` (`:1339`/`:1353`) — the expansion the
  ghost preview and the tracker columns read (see below).
- `ExpandedNote` (`:1026`) carries its own `duration: Option<Duration>`, and
  generators emit a note **only at its onset tick** (the arp returns early on
  non-onset ticks). So the ghost preview draws one block per emitted note at its
  own length — **no per-tick smear, no de-dupe needed** (resolved during review).

**Effect-chain rack** — `gui/mixer_view.rs` (~1040–1120): each effect is a
`ModuleFrame`-wrapped box; `draw_module_header(accent, name, None, |ui| { … })`;
a `+ Add FX` menu button at the bottom. Visual template for the panel (minus
bypass, minus param-grid).

**Pattern header / toolbar** — `gui/sequencer/piano_roll.rs` (~2410–2541):
editable name / description / length, plus the instrument-selector + transport
row. The `Note FX (n)` toggle slots into this row (shown for both inner editors).

**Tracker — already shipped, reuse heavily:**
- **Read-only NP stage columns** — `compute_np_stages` (`gui/sequencer/tracker.rs:147`)
  runs the rack offline via `expand_at_tick_through(tick, |_|true, p+1, buf)`, one
  column per stage showing the **cumulative** output after that processor. This is
  the tracker's "preview" — already done. *Limitation:* it samples at row-tick
  resolution, so sub-row events (a 1/32 arp, a fast roll) aren't shown (documented,
  `tracker-view-plan.md:251`). The piano-roll ghost preview (NP6.3) is the
  full-resolution complement.
- **Per-note sub-column edit flow** — `edit_expression_cell` (`tracker.rs:826`):
  a `tracker_value_buffer` digit buffer, **Enter** commits, **Delete** clears,
  written via `pattern.set_note_expression(...)` with a `SetExpressionBatch` undo.
  The ornament sub-column (NP6.2) mirrors this verbatim.
- The four tracker view-state fields live flat on `SequencerViewState`
  (`gui/sequencer/mod.rs`); a review suggested grouping them into a
  `TrackerViewState` sub-struct — optional, fold in if convenient when touching.

**Sequencer mutation pattern** — sequencer edits take a short `song.write()`
lock, mutate the `Pattern` directly, then push an `UndoAction`
(`piano_roll.rs` ~362–378). The rack edits follow the same path — **not**
`handle.send`: the rack lives on the shared `Song`, and the audio thread re-reads
it each tick (`expand_at_tick` runs against `try_read()`), so no `EngineCommand`
round-trip is needed.

**Undo** — `crates/pertylizer/src/undo.rs` has module parallels
(`AddModule`/`RemoveModule`/`SetParameter`) but **nothing for processors or
ornaments** — all NP6 undo variants are new (see each step).

---

## NP6.1 — The shared Note FX rack inspector (headline)

A right-docked panel showing the active pattern's rack as a vertical card stack,
available beside either inner editor. This *is* the rack-management UX for both
views (closes `tracker-view-plan.md:253`).

### Layout

```
┌─ NOTE FX ───────────── [freeze ▾] [×] ┐
│ ┌ ① Scale Quantize ──────────── ⋮ ┐    │   ⋮ = remove
│ │ Root [ C ▾]  Scale [ Major ▾]  │    │   12 pills = custom ScaleMask
│ │ ◌C ●D ◌C# ●E ●F ◌F# ●G …       │    │
│ └────────────────────────────────┘    │
│ ┌ ② Chord ───────────────────── ⋮ ┐   │
│ │ Type [Major ▾] [0][+4][+7][+]  │    │   interval chips, click = remove
│ │ Strum ▓▓░░ 0   Dir (Up|Down)   │    │   Dir greyed when strum = 0
│ └────────────────────────────────┘    │
│ ┌ ③ Arpeggiator ─────────────── ⋮ ┐   │
│ │ Mode [Up ▾]  Rate [1/16 ▾] Oct◀1▶│  │
│ │ (gate) (swing)  Vel [AsPlayed ▾]│   │
│ │ Latch ☐                         │   │
│ └────────────────────────────────┘    │
│ ┌ ④ Humanize ────────────────── ⋮ ┐   │
│ │ (vel ±) (gate ±)  Seed 0  🎲    │    │   🎲 = reroll seed
│ └────────────────────────────────┘    │
│ [ + Add ▾ ] → Scale / Chord / Arp / …  │   only missing stages
└────────────────────────────────────────┘
```

### Per-type param mapping (straight off the data model)

| Card | Field → widget |
|---|---|
| **ScaleQuantize** (`note_processor.rs:151`) | `root: PitchClass` → ComboBox C…B · `mask: ScaleMask` → preset ComboBox (Chromatic / Major / Nat-minor / Harm-minor / Penta±, the consts at `:94`) **+** a 12-pill toggle row for a custom mask (`contains_interval` / set bit) |
| **Chord** (`:226`) | preset ComboBox (Major / Minor / Dom7 / Custom, from `Chord::major/minor/dominant7`) → editable interval chips bound to `intervals()` (`:293`, max `MAX_CHORD_INTERVALS = 8`) · `strum: Duration` → slider (0 = block chord) · `direction: StrumDirection` → Up/Down toggle, greyed while `strum == 0` |
| **Arpeggiator** (`:417`) | `mode` ComboBox (7 `ArpMode`) · `rate` ComboBox (12 `ArpRate`) · `octaves: u8` stepper 1–4 · `gate` / `swing` `NormalizedValue` knobs · `velocity` ComboBox (3 `ArpVelocity`) · `latch: bool` toggle |
| **Humanize** (`:709`) | `velocity` / `gate` `NormalizedValue` knobs (±) · `seed: u64` `DragValue` + a dice button that rerolls (note: the seed lives on the pattern, so all placements humanize identically — documented limitation, not a bug) |

### Wiring

- [ ] `SequencerViewState`: add `note_fx_panel_open: bool`.
- [ ] `Note FX (n)` toolbar button in the pattern header row (~2528) toggling the
  flag; badge `n = pattern.processors().len()`. Visible for both inner editors.
- [ ] `egui::SidePanel::right("note_fx")` in the sequencer central area, shown when
  the flag is set **and** a pattern is open. Clone the rack snapshot
  (`Vec<NoteProcessor>`) under a short lock before drawing, like the existing
  piano-roll snapshots — never hold `song.write()` across rendering.
- [ ] Each card: `ModuleFrame` + `draw_module_header(accent, name, None, |ui| {
  remove ✖ })`, then the hand-rolled param body.
- [ ] `+ Add ▾` lists only kinds whose `chain_stage()` is absent; on pick, call
  `add_processor` (auto-inserts at the right slot).
- [ ] Mutations via `song.write()` → `pattern_mut(pid)` → `add_processor` /
  `remove_processor` / `set_processor` → push undo. **New undo variants**
  (`undo.rs`): `AddNoteProcessor` + `RemoveNoteProcessor` (carry the processor +
  its index for restore) and `SetNoteProcessorConfig` (before/after the whole
  config enum — cheap, the configs are small/`Copy`). Param drags coalesce per
  gesture like `SetVelocitiesBatch`.
- [ ] **freeze ▾** → confirm dialog (freeze is destructive: it bakes the rack into
  plain notes and clears it) → `song.write()` → `freeze_processors()`. **Undoable**
  (decided): snapshot the whole pre-freeze pattern (its notes + rack) into a new
  `FreezePattern` undo variant carrying it; undo restores both. The snapshot is two
  cloned `Vec`s — cheap — and `DeletePattern`/`AddPattern` already set the
  whole-pattern-snapshot precedent. The confirm dialog still warns it's a bake.

---

## NP6.2 — Ornament editing in the tracker (primary)

The ornament lives on the note; the tracker edits notes per-cell, and its
sub-column flow already exists — so this is the natural, cheap home.

### The shared ornament-editor popup (built here, reused by NP6.3)

A small popup (`egui::Window` or `popup_below_widget`) editing the full
`Ornament` (`note.rs:220`): `count` stepper · `spacing: Duration` ·
`spacing_curve` (Even / Accel / Decel) · `dynamics` (Flat / Cresc / Decresc) ·
`placement` (LeadIn / OnBeat) · `pitch_offset: Semitones` · `grace_gate:
NormalizedValue` knob. Preset buttons set `count` + the canonical curve by the
drum-rudiment convention (flam = 2, drag = 3, ruff = 4, roll = N, grace = 2 with
a `pitch_offset`; `Ornament::default` is a flam). Factor it as a standalone fn
that takes `&mut Option<Ornament>` so both the tracker cell and the piano-roll
shortcut call the same code.

### Tracker ornament sub-column

- [ ] Add an **ornament sub-column** alongside the expression sub-columns
  (extend the `ExprField`-style set in `tracker.rs:299`, or a parallel column).
  Cell shows a marker + `count` when set, `·` when unset.
- [ ] **Enter** on the cell opens the shared popup; **Delete** clears the ornament
  (`note.ornament = None`); presets reachable from the popup. Reuse the
  `tracker_value_buffer` / commit pattern from `edit_expression_cell` (`:826`).
- [ ] Write via the note-mut path the MCP `set_note_ornament` already uses
  (NP7b, `670edf2`); push a `SetNoteOrnament` undo (before/after `Option<Ornament>`,
  batched across the selection to mirror `SetExpressionBatch`).
- [ ] Header tooltip for the column (the tracker convention, `a8c6434`).

---

## NP6.3 — Piano-roll surface (view + shortcut)

The piano roll *shows* the result and offers one light editing shortcut.

- [ ] **Ghost-note preview.** `SequencerViewState`: `show_note_fx_ghosts: bool` +
  a 👁 toggle. When on and the pattern has any processors or ornamented notes,
  sweep `Pattern::expand_at_tick` across the pattern length on the **UI thread**
  (allocation fine here) and paint the expanded notes behind the source notes at
  reduced alpha, non-interactive. Cache the sweep keyed on `(pattern_id,
  rack-hash, notes-hash)` — the expansion is deterministic. Each emitted
  `ExpandedNote` carries its own `duration` and is emitted only at its onset tick,
  so draw one ghost block per emission at its own length (no de-dupe needed — see
  the API note above).
- [ ] **Ornament glyph.** Notes with `ornament.is_some()` get a small glyph at the
  note head (e.g. a `ri::` flag icon), read-only — so an ornamented note is
  visible at a glance.
- [ ] **"Edit…" shortcut.** When a single note is selected, show a compact
  ornament summary + an "Edit…" button in the selection-inspector row
  (`piano_roll.rs` ~259–489, alongside velocity / tie / glide) that opens the
  **shared popup from NP6.2**. No bespoke right-click menu, no grid popup — the
  inspector button is the whole piano-roll editing affordance.
- [ ] The **rack panel** (NP6.1) is already visible here (it is docked beside
  whichever editor is active), so adding/removing/configuring processors works in
  the piano roll too — this is the rest of "a piano-roll user is never stranded."

---

## Cross-cutting

- [ ] **Undo coverage** — every NP6 mutation has an `UndoAction`: add / remove /
  config (NP6.1), freeze (NP6.1, undoable via a full pre-freeze pattern snapshot),
  set / clear ornament (NP6.2/6.3). All four+ variants are new in `undo.rs`.
- [ ] **One editor per feature** — the rack panel (NP6.1) and the ornament popup
  (NP6.2) are each a single widget reached from both views; the tracker column and
  the piano-roll "Edit…" button only *launch* the shared popup. No duplicated
  editing logic.
- [ ] **Snapshot discipline** — clone the rack / ornament / expansion data under a
  short `song.write()`/`try_read()` window before drawing, matching
  `collect_piano_roll_data`; never hold the lock across rendering.
- [ ] **No headless test** — NP6 is GUI-only (the underlying engine / MCP /
  persistence paths are already tested, and with the bypass flag dropped there is
  no new data-model change to round-trip). Verify with the `/verify` skill: build
  a rack, edit an ornament in the tracker, confirm the stage columns + piano-roll
  ghosts match what plays, and that freeze bakes correctly.
- [ ] **On ship** — flip the NP6 checkbox in `plans/note-processors-plan.md:44`,
  mark NP6 done in `plans/TODO.md §3.3`, mark the tracker rack-management item
  (`tracker-view-plan.md:253`) closed-by-the-shared-panel, and update the
  `project_note_processors` memory.
