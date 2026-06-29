# Plan: Shared GUI widget helpers (style consolidation)

Pertylizer's GUI already has a strong **central theme** (`gui/theme.rs`,
`setup_custom_style`) and a partial helper layer (`gui/widgets/controls.rs`,
`gui/widgets/param_grid.rs`). But the *call sites* still repeat the same
theme-coloring and layout configuration by hand — most visibly **173** copies of
`RichText::new(x).color(theme().colors.text_dim/secondary)` and **50+**
hand-rolled `ui.horizontal(label + widget)` rows.

Goal: factor the highest-repetition patterns into a small set of **thin,
`Response`-returning** helpers so a future theme/spacing change lands in one
place and the UI stays visually consistent — **without** building a heavyweight
component framework that fights egui's immediate-mode grain.

This is a quality/consolidation refactor. No behavior change, no new features.

---

## Implementation progress

- [x] **Step 0 — Core helpers** (`controls.rs`): `dim_label` / `secondary_label`
  / `accent_label`, `labeled_row` / `labeled_row_aligned`, `time_drag_value` /
  `suffix_slider` / `int_slider`; `section_header` now returns `Response` and
  takes an optional description; `toggle_button*` take `WidgetText`. Re-exported
  from `mod.rs`. Built green (`section_header` title stays `&str` — `WidgetText`
  has no `.size()`, and the heading needs the explicit theme size; it is not a
  hot path).
- [x] **Step 1 — `egui_backend.rs`** dim-labels. Migrated the 6 *pure*
  `ui.label(RichText::new(x).color(text_dim))` sites to `dim_label`. NOTE: the
  raw 42 `text_dim` / 10 `text_secondary` counts overstate the addressable set —
  most are `.prefix(..)` on `DragValue`, tuple atoms, or carry extra
  `.size()/.strong()` styling the colour-only helper can't express, and the
  `text_secondary` ones are `RichText` args to non-`label` widgets. Those stay.
- [~] **Step 2 — `mixer_view.rs`** — SKIPPED (nothing migratable with the
  current colour-only helpers). All its `text_dim`/`text_secondary` text is
  either `.size(size_small).color(..)` captions, `Button::new(RichText..)`,
  tuple atoms, or ternary colour values — none is a plain
  `ui.label(RichText::new(x).color(..))`, and it has no simple `label + widget`
  rows. See the **Premise correction** finding below.
- [x] **Step 3 — `awe_view.rs`** sliders → `suffix_slider` (18 migrated;
  verified faithful). DEVIATIONS, by design: (a) the 3 `.logarithmic(true)`
  sliders (high_cut, low_cut, LFO rate) were **left inline** — `suffix_slider`
  can't express a log scale; migrating would silently drop it (helper gap to
  note). (b) The **`Grid` + `dim_label` restyle was NOT done** — it changes
  appearance (column alignment + label colour from default→dim) and the plan
  itself requires an in-app eyeball for AWE, which an autonomous run can't do.
  Deferred as a flagged visual change. So Step 3 here = the provably
  no-visual-change `suffix_slider` rename only. `export_dialog.rs` →
  `time_drag_value` handled in Step 3b below if any sites exist.
- [x] **Step 4 — `sequencer/` + `export_dialog.rs`.** Migrated 3 pure
  `dim_label` sites (`sequencer/mod.rs` "Pattern not found" / "Song locked…",
  `piano_roll.rs` "Auto:") and the `export_dialog.rs` *duration* DragValue →
  `time_drag_value`. SKIPPED, by design: the export *tail* DragValue uses
  `speed(0.1)` but `time_drag_value` hardcodes `speed(0.5)` — migrating would
  change drag sensitivity (helper-rigidity gap: it should take a speed/decimals
  arg or expose a builder). `note_fx.rs` has nothing migratable — its
  `DragValue`s aren't seconds and its `Slider`s carry `.text(..)` or non-plain
  int ranges that `suffix_slider`/`int_slider` don't cover.
- [x] **Step 5 —** swept the remaining *pure* `ui.label(RichText::new(x)
  .color(t.colors.text_dim))` sites (the `t.colors` local-binding variant the
  Step-1 grep missed): 14 in `egui_backend.rs` (song/author/metadata dialogs),
  + `transport.rs` "STOPPED", + `note_fx.rs` "Pattern unavailable…". All
  `dim_label`. No pure `text_secondary` `ui.label` sites remain. The styled
  captions (`.size(size_small).color(..)`) are intentionally left for the
  caption-helper decision (see Premise correction).
- [x] **Step 6 — caption helper + helper flexibilization** (per user request).
  Added `caption(ui, text, CaptionTone::{Dim,Secondary})` and migrated **12**
  `.size(size_small).color(text_dim|secondary)` sites (`mixer_view` ×5, `popups`
  ×4, `patch_editor` ×1, `param_grid` ×1, and `section_header` now uses it
  internally). Flexibilized: `time_drag_value` gained a `speed: f64` arg (the
  export *tail*, `speed 0.1`, is now migrated too — both export sites done); and
  a new `log_suffix_slider` covers logarithmic sliders, so the **3** AWE log
  sliders (high_cut, low_cut, LFO rate) are now migrated. Verified faithful by
  an independent review (no text/tone swaps, ranges/suffixes/log-scale/speeds
  preserved). All green.

### Loop outcome & open decisions (autonomous run, branch `feat/widget-style-helpers`)

**Done (safe, no-visual-change, all green + reviewed):** the helper module
(Step 0) + ~45 mechanical call-site migrations — `dim_label` ×25
(`egui_backend`, `sequencer`), `suffix_slider` ×18 (`awe_view`),
`time_drag_value` ×1 (`export_dialog`).

**Helper-usage audit (after Step 6 + review).** USED: `dim_label` ×25,
`caption` ×12, `suffix_slider` ×18, `log_suffix_slider` ×3, `time_drag_value` ×2.
The 5 zero-call-site helpers (`secondary_label`, `accent_label`, `labeled_row`,
`labeled_row_aligned`, `int_slider`) were **REMOVED in Step 7** (review consensus
+ CLAUDE.md "minimize public API surface"). Re-add when a real call site appears.

### Step 7 — high-effort `/code-review --fix` (8-angle)

- [x] **Removed the 5 dead helpers** (above) + their `mod.rs` re-exports and the
  now-unused `InnerResponse` import.
- [x] **Fixed a real latent bug:** `analyze.rs` built its `wav_dialog` WITHOUT
  `.as_modal(false)`, unlike `dialogs.rs::new_file_dialog`. egui-file-dialog
  defaults `as_modal: true`, and `new_file_dialog`'s comment documents that under
  egui 0.35 the modal backdrop can render on top and freeze the dialog. Added
  `.as_modal(false)` to both `analyze.rs` `wav_dialog` sites for consistency.
  (Flagged by review angles A + B + C.)
- [x] **Step 8 — made `retain_selected_entry` actually work** (was a no-op).
  Root cause (review angle C): every `open_*_dialog()` / the analyze save path
  rebuilt the `FileDialog` instance, discarding the per-instance retained state
  (highlighted entry + `storage.last_picked_dir`, which the default
  `OpeningMode::LastPickedDir` reads). Fix: a **persistent instance** + a
  `file_dialog_kind` field; `ensure_dialog(kind, …)` rebuilds the filters ONLY
  when the kind changes, so reopening the same dialog reuses the instance and its
  retained directory + entry survive. Save dialogs set `default_file_name` via
  `config_mut()` each open (works on reuse). `analyze.rs` reuses its persistent
  `wav_dialog`. Verified correct against the egui-file-dialog 0.14.1 source by an
  independent review (`[]`). KNOWN LIMITATION: *switching* dialog kind (e.g. Open
  Patch → Save Patch) rebuilds and resets retention — it persists within a kind,
  not across kinds. Global cross-kind memory would need filter-swapping on one
  shared instance; the P2 high-effort review (altitude angle) confirmed this is
  feasible *without* fragile manual `FileFilter` construction — `config.file_filters`
  is a public `Vec<FileFilter>` (clear + re-add per open via `config_mut()`),
  which would also delete the `file_dialog_kind` field, the rebuild branch, and
  this limitation. Deferred (the current design is correct + reviewed; this is a
  behaviour-improving refactor best done with an in-app eyeball), but it is the
  recommended cleaner shape if cross-kind retention is wanted.
- Review also confirmed every caption/slider/dim_label/`time_drag_value`
  migration is behavior-faithful (no text/tone/range/suffix/speed/log swaps), and
  found no CLAUDE.md violations, no perf regressions, no caller breakage.

**Still deferred (need a user decision / in-app eyeball):**
1. **AWE `Grid` + `dim_label` restyle** — a visual change (alignment + label
   colour); needs an in-app eyeball.
2. **`section_header` at the `awe_view` "Mix" panel.** The description sub-line
   IS exactly `caption(.., Dim)` (theme `size_small == 10.0`), but the heading
   uses `ui.heading(..)` vs `section_header`'s `ui.label(..).strong()` — not
   provably identical (heading text-style vs bold), so it needs an eyeball before
   migrating. Left raw for now.

**Git note:** this branch's first commit is the unrelated egui-file-dialog 0.14.1
upgrade — split it out before squash-merging if you want it landed separately.

### ⚠ Premise correction (found during migration — needs a decision)

The intro's "~173 colour-only repetitions" overcounts the set the colour-only
helpers can actually replace. Measured across `gui/`:

- **Pure `ui.label(RichText::new(x).color(text_dim|secondary))` (single-line):
  ~4 total.** The colour-only `dim_label`/`secondary_label` have almost no
  uptake beyond Step 1's handful.
- **`.size(size_small).color(text_dim|secondary)` ("caption" style): ~28.** This
  is the *actually* dominant repeated pattern — small + dimmed/secondary text
  (channel values, sends labels, "✖", `{:.2}` readouts, …).
- The remaining 235 `text_dim`/`text_secondary` references are `Button::new`
  args, `DragValue::prefix`, tuple atoms, or ternary colour values — not labels.

**Proposed amendment (for the user to approve before coding):** add a caption
helper rather than forcing the colour-only ones. Either a dedicated
`caption(ui, text, Variant)` / `small_dim_label` / `small_secondary_label`, or
give the existing label helpers an optional size. ~28 sites would migrate vs ~4.
Not done here — adding a new public helper API is a design call (naming / shape)
left to the user; Steps 2 and 5 stay thin until then.

---

## 0. Design principles (govern every helper below)

1. **Helpers return `Response`** (or `InnerResponse<R>`), never `()`. Call sites
   must still be able to chain `.changed()`, `.on_hover_text()`,
   `.context_menu()`, `.clicked()`. A helper that swallows the response is a
   regression, not a convenience.
2. **Wrap the common case only.** A helper removes repetition; it is never
   mandatory. An odd one-off keeps calling `egui::Button`/`Slider` directly.
   Resist growing a helper past ~4–5 args — when a call needs a sixth knob, it
   wants the raw widget, not another parameter.
3. **Two generalization styles, used for different things:**
   - **A — thin functional wrappers** for free layout (labels, rows, presets).
     Idiomatic egui composition. This plan is mostly A.
   - **B — descriptor-driven** for inputs that already carry a schema
     (range/unit/choices). `param_grid::labeled_param` already does this for
     module params; extend that idea later, view-by-view — *not* in this plan.
4. **Custom-painter widgets are out of scope.** knob, meter, port, cable, scope,
   spectrum, waveform, envelope are correctly bespoke; leave them.
5. **All colors/sizes/spacing come from `theme()`** — helpers never introduce a
   new literal. They are the enforcement point for "no inline `Color32` outside
   theme.rs".

---

## 0a. File organization — one file per type? (decided)

**No file-per-widget-type for thin helpers.** The existing `gui/widgets/`
already follows the right principle, and it is *not* "one file per type":

- **One file per widget** is reserved for *heavyweight custom-painter widgets*
  with their own state and paint code: `knob.rs` (310), `meter.rs` (332),
  `cable.rs` (387), `envelope.rs` (518), `port.rs`, `scope.rs`, `spectrum.rs`,
  `waveform.rs`. These earn their own file.
- **Thin compositions are grouped by role**: `controls.rs` (126) already holds
  toggle-button + icon-button + section-header + dialog-row + modal in *one*
  file — not a file per button type.

The dividing line is **amount of code + cohesion, not "type of widget."** A
`label.rs` / `dropdown.rs` / `slider.rs` would each be 10–30 lines, where the
`mod` decl + re-export + `use` lines outweigh the content — fragmentation, not
structure. Rule going forward:

| Pattern | Home |
|---|---|
| Thin style wrappers (label, row, numeric preset) | **fold into `controls.rs`** |
| Heavyweight bespoke widgets (knob, meter, cable…) | one file per widget (unchanged) |
| Descriptor-driven inputs (style B) | `param_grid.rs` / later per-view work |

Give a helper its own file only once it grows its own state, its own painting,
or ~150+ lines — never just because it is "a different type."

**Decision: fold into `controls.rs`, do NOT add a new `style_helpers.rs`.**
`controls.rs` is only 126 lines and its own doc-comment already says it exists
"so the styling lives in one place" — labels, rows and numeric presets are
exactly that. A separate `style_helpers.rs` would split base-styling primitives
across two files and add `mod` + re-export + `use` boilerplate in `mod.rs` for
no real cohesion gain (~126 + ~80 lines stays comfortably one-file-sized). Add
the new helpers to `controls.rs` and broaden its module doc-comment to cover
"labels and labeled rows" alongside the existing composite controls.

**While here, fix `section_header` — it violates principle 1.** It currently
returns `()` (`controls.rs:71`), so callers can't chain `.on_hover_text(..)`.
Change it to return the label's `Response` (the trailing `add_space` stays).
This is the canonical heading helper (see §1).

---

## 1. The helpers (Phase A core)

Add these to `gui/widgets/controls.rs` (already re-exported from
`gui/widgets/mod.rs`); broaden its module doc-comment to mention labels and
labeled rows. Start with exactly these; add more only when a real call site
demands it.

### Labels (kills ~173 repetitions)

```rust
pub fn dim_label(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response;
pub fn secondary_label(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response;
pub fn accent_label(ui: &mut Ui, text: impl Into<egui::WidgetText>, color: Color32) -> Response;
// heading: reuse the existing `section_header` (see below) — do not add a 4th label fn.
```

`dim_label` replaces `ui.label(RichText::new(t).color(theme().colors.text_dim))`
(110×); `secondary_label` the `text_secondary` variant (63×). Body is e.g.
`ui.label(text.into().color(theme().colors.text_dim))` — `WidgetText::color()`
exists in egui 0.35 (`widget_text.rs:618`), so this compiles for both `&str` and
`String` inputs.

**Take `impl Into<egui::WidgetText>`, not `impl Into<String>` (decided).** egui
redraws at frame rate; `impl Into<String>` would heap-allocate a fresh `String`
every frame for each of the 170+ `&str` literal labels. `WidgetText` accepts a
borrowed `&str` with **zero allocation** and is the idiomatic egui label type.
Refactor the existing `toggle_button` / `toggle_button_colored` /
`section_header` to the same signature in the same pass — they have the same
needless-allocation today.

**Heading: one `section_header` with an optional description (decided).** Don't
add `heading_label`, and don't add a separate `section_header_described` either —
both would be near-duplicates, exactly what this refactor removes. Instead extend
the existing `section_header` (`controls.rs:71`) with an optional sub-line:

```rust
pub fn section_header(
    ui: &mut Ui,
    title: impl Into<egui::WidgetText>,
    description: Option<&str>, // None = bare heading; Some = small dim sub-line
    color: Color32,
) -> Response;
```

- Title: strong, `color`, `size_heading` (unchanged from today).
- `Some(desc)`: a `size_small` / `text_dim` line directly below the title —
  folds in the repeated "heading + dim description + space" pattern (e.g. the
  `Mix` / "Dry/wet signal balance" panels).
- Always ends with the trailing `add_space` it already emits.
- Returns the **title** label's `Response` (the §0a fix) so callers can still
  `.on_hover_text(..)` the heading.

This is `controls.rs:71` made to return `Response` *and* grow the one extra
optional arg — no second function. Existing 3-arg callers just pass `None` (no
back-compat concern in this project). Optional ergonomic touch: type the param
as `impl Into<Option<&str>>` so callers write `"desc"` or `None` without
wrapping in `Some`.

### Labeled row (kills 50+ hand-rolled horizontals)

```rust
pub fn labeled_row<R>(
    ui: &mut Ui,
    label: impl Into<egui::WidgetText>,
    widget: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R>;

// Fixed label column so widgets line up across rows of unequal label length.
pub fn labeled_row_aligned<R>(
    ui: &mut Ui,
    label: impl Into<egui::WidgetText>,
    label_width: f32,
    widget: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R>;
```

`labeled_row` centralizes
`ui.horizontal(|ui| { dim_label(label); add_space(xs); widget })` and returns the
widget's inner result so `.changed()` still works. It is for **free-layout**
rows — one-off rows where cross-row alignment doesn't matter.

`labeled_row_aligned` is a **fallback** for a small handful of rows that *do*
need to line up but aren't worth a Grid (see Alignment below). It allocates a
fixed-width cell for the label (`ui.allocate_ui([label_width, h], …)`). Pull
`label_width` from `theme().spacing`, never a per-call magic number — and reach
for it only when Grid is genuinely overkill.

### Alignment: prefer `egui::Grid`, not fixed label widths

For a **rack of rows that must line up** (the AWE shape: 32 label+slider rows,
unequal labels), the right tool is `egui::Grid`, not a fixed `label_width`. Grid
auto-sizes the label column to the widest label and aligns every row — no magic
number, pixel-perfect, and it's already used in five GUI files (`dialogs.rs`,
`export_dialog.rs`, `sample_view.rs`, `piano_roll.rs`, `egui_backend.rs`). A
hand-tuned `label_width` reinvents a worse Grid: 80 px clips `"Modulation Rate"`,
and a per-view width is just an untracked magic constant.

Before (today, `awe_view.rs` — ragged, one `horizontal` per row):

```rust
ui.horizontal(|ui| {
    ui.label("Diffusion:");
    ui.add(egui::Slider::new(&mut state.material_diffusion, 0.0..=1.0))
        .on_hover_text("…");
});
// …repeated 32× with labels of different widths
```

After (one Grid, two cells per row, auto-aligned):

```rust
egui::Grid::new("awe_room").num_columns(2).show(ui, |ui| {
    dim_label(ui, "Diffusion");
    suffix_slider(ui, &mut state.material_diffusion, 0.0..=1.0, "")
        .on_hover_text("…");
    ui.end_row();
    // …one dim_label + suffix_slider pair per row
});
```

**Do not** use `labeled_row*` *inside* a Grid — they wrap the row in their own
`horizontal`, which fights Grid's per-cell layout. Inside a Grid emit
`dim_label(ui, …)` then the widget as two separate cells, then `ui.end_row()`.

### Numeric presets

```rust
// Time DragValue — for export_dialog.rs etc. where the control really IS a
// seconds DragValue. NOT for AWE (see below).
pub fn time_drag_value(ui: &mut Ui, secs: &mut f32, range: RangeInclusive<f32>) -> Response;
// preset: .speed(0.5).suffix(" s") with sensible decimals

// Slider with a unit suffix — the AWE shape. Generic so it also covers i32/usize.
pub fn suffix_slider<T: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut T,
    range: RangeInclusive<T>,
    suffix: &str,
) -> Response;
// preset: egui::Slider::new(value, range).suffix(suffix)

// Discrete/integer slider, generic over any numeric type.
pub fn int_slider<T: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut T,
    range: RangeInclusive<T>,
) -> Response;
// preset: .step_by(1.0).min_decimals(0).max_decimals(0)
```

**AWE wants `suffix_slider`, not `time_drag_value` (corrected).** `awe_view.rs`
is **32 `egui::Slider`s, zero `DragValue`s**, and the suffixes are `m`, ` Hz`,
` ms`, ` °C`, `x` — not seconds. A hardcoded time/DragValue helper does not fit;
the generic `suffix_slider` (Slider + arbitrary suffix) does. `time_drag_value`
stays, but only for the call sites that are genuinely seconds DragValues
(`export_dialog.rs`).

**`int_slider` / `suffix_slider` are generic over `egui::emath::Numeric`
(corrected).** That lets egui drive `i64`/`i32`/`usize`/`f32` directly with no
caller-side casts. Note many "discrete" params in the patch editor /
`param_grid.rs` are stored as `f32` with a step of `1.0` — the generic form
handles both those and true integer state without a second helper.

ComboBox is **deliberately excluded** — widths (90–180 px) and id-salts vary too
much for a worthwhile wrapper.

---

## 2. Migration (one reviewable commit per cluster)

Land Phase 1 (the helpers in `controls.rs`, plus the `section_header`/
`toggle_button` signature fixes) first, on its own, green. Then migrate call
sites in small, mechanical, separately-reviewable commits — do **not** rewrite
the whole GUI in one diff. Suggested order by ROI / blast-radius:

1. `egui_backend.rs` dim-labels (patch bar, song editor: e.g. lines ~2162,
   3784–3813, 4099–4110). Highest count, self-contained.
2. `mixer_view.rs` labels + the labeled-row rows.
3. `awe_view.rs` — wrap each rack of rows in an `egui::Grid` and migrate to
   `dim_label` + `suffix_slider` cells (32 sliders w/ suffix); `export_dialog.rs`
   time inputs → `time_drag_value`.
4. `sequencer/` labels + `note_fx.rs` drag values.
5. Sweep remaining `text_dim`/`text_secondary` literals view by view.

After each commit: `cargo fmt --check && cargo build && cargo clippy
--all-targets && cargo test` must be green (zero warnings — `-D warnings` is on).

---

## 3. Done / verification

- A grep for `RichText::new(` immediately followed by
  `.color(theme().colors.text_dim` / `text_secondary` returns ~0 hits outside
  `controls.rs` and the genuinely-special cases.
- No new `Color32::` literals introduced outside `theme.rs`.
- In-app eyeball: every migrated view looks identical to before (this is a
  no-visual-change refactor) and a theme switch still recolors everything.
- Net: ~200+ lines of repeated theme config removed; one edit point for label
  styling and labeled-row spacing going forward.

---

## 4. Explicitly out of scope (and why)

- **A component framework that owns layout.** egui is immediate-mode; an opaque
  wrapper that hides `Response` or forces all layout through it fights the grain
  and accumulates argument bloat. Keep helpers thin and optional.
- **Compound `egui::Widget` structs (`LabeledSlider`, `LabeledDragValue`) —
  deferred, not rejected.** A `ui.add(LabeledSlider::new(..).suffix(..))` builder
  reads nicely, but it doesn't clearly beat the two cheaper options we already
  have: for free-layout rows, `labeled_row` + `suffix_slider` is enough; for
  aligned racks, `egui::Grid` is *better* than a struct that hardcodes a
  `label_width` (which reintroduces the magic-width problem and the very
  "Widget that owns layout" smell above — a 6-field builder also bumps the
  ~4–5-arg ceiling of principle 0.2). **Condition to promote it later:** after
  the Grid + thin-helper migration, if real call sites show the builder sugar is
  worth it, add it then — with width from `theme()` and alignment delegated to an
  enclosing Grid, never a self-owned fixed width.
- **`LabeledComboBox` — stays out (see ComboBox below).** A label-derived
  `make_persistent_id` collides when two combos share a label and share their
  open/closed state; the id-salt must be caller-supplied, not derived. Combined
  with varying widths, the ROI isn't there.
- **ComboBox wrapper** — varying width/id, low repetition, poor ROI.
- **Custom-painter restyling** — knobs/meters/cables already read `theme()`.
- **Descriptor-driven inputs beyond `param_grid`** (style B for AWE/export/etc.)
  — a real generalization, but a separate later effort done per-view where an
  actual field schema exists. This plan is the thin-wrapper layer only.
- **Sequencer/keyboard hardcoded colors** — those are a *theme-coverage* gap
  (colors not in `theme.colors`, so they don't switch with presets), tracked
  separately from this call-site-consolidation work.

---

## 5. GUI-wide helper audit (Phase 2 backlog)

A multi-agent audit (11 parallel readers over the whole `gui/` tree, ~45k LOC →
181 raw findings → **38 distinct un-helpered idioms**) inventoried every
widget/button/input/label call-site **not** going through a baseline helper, so
we can decide which to fold in. The user's intent: even low-count idioms are
worth a helper so they can later be refactored away or extended.

Baseline already covered (do not re-list): `toggle_button(_colored)`,
`icon_button(_sized)`, `dialog_button_row`, `modal_window`, `dim_label`,
`caption(CaptionTone::{Dim,Secondary})`, `section_header`, `time_drag_value`,
`suffix_slider`, `log_suffix_slider`. Out of scope: bespoke painter widgets
(knob/meter/cable/port/scope/spectrum/waveform/envelope/frame/tooltip),
descriptor-driven `param_grid`.

### Reconciliation with earlier decisions

The audit's *data* revisits two earlier calls in this doc:
- **ComboBox** was "deliberately excluded" (§4, §1 numeric). The audit finds it
  is the **widest** idiom — 34 sites across 12 files — and `note_fx.rs` already
  has a local `enum_combo()`. Recommend **reversing** the exclusion: an
  `enum_combo<T>(ui, id_salt, &mut T, &[(T,&str)])` taking a *caller-supplied*
  id-salt sidesteps the original id-collision objection. (Grouped/headered
  preset combos stay bespoke.)
- **`labeled_row`** was removed in Step 7 as a zero-call-site helper. The audit
  finds **49** hand-rolled `ui.horizontal(label + control)` rows (5 files) — a
  real call-site demand now exists, so re-add it (this satisfies principle 0.2's
  "add when a real call site demands it").

### Tier A — extend EXISTING helpers (highest leverage, ~0 behaviour change)

The surprising theme: many sites re-implement helpers that already exist (some
even imported in the same file). Pure drift — cheapest wins.

| Idiom | Sites / files | Example | Action |
|---|---|---|---|
| Colored/sized inline caption (free `Color32` + explicit size) | **57 / 8** | `patch_editor.rs:3443`, `node.rs:67` | `caption` → add `CaptionTone::Color(Color32)` (+ `Primary`) and a `caption_sized(ui, text, size, color)` variant; optional monospace flag |
| Raw dim caption matching `caption`/`dim_label` byte-for-byte | **39 / 9** | `piano_roll.rs:297`, `dialogs.rs:535` | Route through existing `caption(.., Dim)` / `dim_label`; let `caption` take an explicit size to absorb the `size(10.0)` callers |
| Slider without unit suffix (normalized / bipolar / `.text()`) | 22 / 5 | `awe_view.rs:2349`, `mixer_view.rs:630` | `suffix_slider` → `labeled_slider(ui, v, range, Option<suffix>, Option<text>)` |
| Inline section header (heading + opt sub-line / leading icon) | 16 / 6 | `awe_view.rs:2334`, `dialogs.rs:415` | `section_header` → optional leading `icon: Option<&str>` |
| Inline colored TOGGLE-state text button (re-rolls `toggle_button_colored`) | 10 / 6 | `awe_view.rs:610`, `pattern_view.rs:207` | Route through helper; parameterize the inactive tone (some use `text_secondary` vs helper's `text_dim`) |
| Dialog Cancel/Confirm footer re-hand-rolled | 10 / 4 | `egui_backend.rs:6240`, `export_dialog.rs:208` | `dialog_button_row` → add `confirm_enabled: bool` |
| Frameless icon button re-implemented inline (re-rolls `icon_button`) | 9 / 3 | `mixer_view.rs:476`, `note_fx.rs:72` | `icon_button_sized` → add `color` / `size` args |
| Inline strong/bold section-title label | 15 / 4 | `piano_roll.rs:290`, `welcome_view.rs:108` | `strong_label(ui, text, Option<Color32>)` or a non-heading-size `section_header` variant |

### Tier B — NEW high-impact helpers (not covered today)

| Idiom | Sites / files | Example | Proposed signature |
|---|---|---|---|
| ComboBox enum dropdown | **34 / 12** | `egui_backend.rs:2135`, `awe_view.rs:2148` | `enum_combo<T: PartialEq+Copy>(ui, id_salt: impl Hash, &mut T, &[(T,&str)]) -> Response` |
| Unit-suffixed/prefixed DragValue (non-seconds) | **43 / 8** | `piano_roll.rs:342`, `egui_backend.rs:2198` | `unit_drag_value<T: Numeric>(ui, &mut v, range, speed, prefix, suffix) -> Response` |
| Framed icon+text action button | **43 / 8** | `egui_backend.rs:1665`, `canvas.rs:351` | `action_button(ui, icon, label: impl Into<RichText>, color: Option<Color32>) -> Response` |
| Labeled field row (label + one control) | **49 / 5** | `awe_view.rs:2172`, `dialogs.rs:449` | `labeled_row<R>(ui, label, FnOnce(&mut Ui)->R) -> R` (re-add; standardize label-column width) |
| `add_enabled` gated button (+ disabled hover / shortcut) | 28 / 6 | `egui_backend.rs:1826`, `wiring.rs:409` | `gated_button(ui, enabled, content, disabled_hint: Option<&str>, shortcut: Option<&str>) -> Response` |
| Framed single-colored-icon button (transport/toolbar glyph + tooltip) | 17 / 4 | `transport.rs:92`, `piano_roll.rs:1030` | `framed_icon_button(ui, icon, color, tooltip) -> Response` (default frame; sibling to frameless `icon_button`) |
| Colored status pill / badge (read-only + clickable) | 17 / 3 | `analyze.rs:602`, `transport.rs:453` | `status_pill(ui, text, StatusTone{Good,Warn,Bad,Accent(Color32)}) -> Response` (`analyze.rs` has a local `chip()`) |
| Momentary small toolbar / zoom button | 12 / 3 | `arrangement.rs:142`, `tracker.rs:1301` | `small_button(ui, label, hover, Option<Color32>) -> Response` |
| Destructive (accent_red) text button | 8 / 4 | `arrangement.rs:1637`, `list_panel.rs:42` | `danger_button(ui, label)` + generic `colored_button(ui, label, color)` |

### Tier C — smaller new helpers (the "even if few" bucket)

- **`inline_editable_text(ui, &mut String, &mut bool editing, multiline)`** —
  8 / 4 (`piano_roll.rs:2557`, `arrangement.rs:1574`). **Correctness case**: the
  request_focus-first-frame / commit-on-`lost_focus`|Enter handshake is subtle
  and duplicated ("Mirrors the instrument edit window"); a helper stops the
  editors drifting. Pairs with `clickable_label`.
- `stepper<T>(ui, &mut v, range, step, fmt)` — −/value/+ (6 / 3).
- `segmented_selector<T>(ui, &mut T, &[(T,&str)])` — inline tab/mode switch (5 / 3).
- `bypass_checkbox(ui, &mut bool, tooltip)` — empty-label enable/bypass (6 / 4).
- `empty_state(ui, text)` — centered dim placeholder (3 / 3; `list_panel` has local `empty()`).
- `clickable_label(ui, text) -> Response` — `Label::sense(click)` title (3 / 2).
- `property_row(ui, key, value)` + `property_grid` scaffold — key/value rows (14 / 1, `sample_view`).
- `checked_menu_item(ui, selected, name)` — selection-bullet menu row (7 / 1, `egui_backend` menus).
- `status_label(icon, text, color) -> RichText` — top-bar indicator builder (5 / 1).
- `menu_section_header(ui, title)` — secondary 11px caption + separator (4 / 3, patch-editor menus).
- `fx_button(ui, active)` — ƒx lit/unlit (2) · `color_swatch(ui, color, selected)` (2) · `palette_menu_button` (2).
- Containers: `dialog_window` (extend `modal_window` with resizable/anchored/open-bool, 5 / 2) · `floating_window` (anchored popups, 4 / 1) · `position_menu_popup` (2 / 2, identical 9-line builder) · `list_scroll` (browser ScrollArea config, 6 / 5) · `toast` (bottom-anchored status, 1).

### Leave-bespoke (justified — no shared styling to factor)

| Idiom | Sites / files | Why bespoke |
|---|---|---|
| Plain default-color label (dynamic readouts/tooltips/prompts) | 88 / 8 | Default color/size, per-call `format!` content — nothing to centralize |
| Plain text menu / context-menu / dialog button | 58 / 10 | egui already styles these uniformly; each carries unique click logic |
| Monospace value / grid cell | 16 / 4 | Mostly tracker grid cells coupled to `TrackerColors` + table render; painter-adjacent |
| Nested `menu_button` submenu tree | 15 / 4 | Per-level tree construction, unique bodies, local wrappers already shell fixed parts |
| Labeled boolean checkbox (Tie/Glide/…) | 9 / 3 | Bare idiomatic egui checkbox; the real repetition is batch-undo *logic*, not the widget |
| Hand-styled `Frame` / panel container | 6 / 3 | Structurally different per use (panel chrome vs accent group vs track-row geometry) |

### Recommended sequencing

1. **Tier A first** (~160 sites): drift fixes — biggest consistency win, near-zero
   risk, and they shrink the helper-vs-inline divergence the audit exposed.
2. **Tier B's four structural** (`enum_combo`, `unit_drag_value`, `action_button`,
   `labeled_row` — ~170 sites) next; they cover gaps the four baseline helpers
   never addressed.
3. **`inline_editable_text` early** — it's a correctness case, not just style.
4. **Tier C** opportunistically, per view.

Same workflow as Steps 0–8: one helper (or one Tier-A drift cluster) per
reviewable commit, `fmt`/`build`/`clippy`/`test` green each step, behaviour-faithful
migrations only (visual restyles — like the colored-caption tone work touching
accent colors — get an in-app eyeball).

### Phase 2 implementation progress

Autonomous rule: only **provably no-visual-change** migrations are applied; any
site whose look would shift (e.g. `.small()` ≈ 9px vs `caption` 10px, accent-tone
consolidation, heading-vs-`.strong()`) is SKIPPED and flagged for an in-app
eyeball. Helpers are added only alongside real migratable sites (no speculative
dead API — see Step 7).

- [x] **P2-4 — `danger_button` (Tier B).** Added
  `danger_button(ui, label) = ui.button(label.color(accent_red))` and migrated
  the **9** provably-identical `ui.button(RichText::new(x).color(accent_red))`
  destructive buttons (`arrangement` ×4 Delete Track/Pattern/Clear loop/Remove
  tempo, `egui_backend` ×2 Delete…, `piano_roll`/`pattern_view`/`sample_view`
  Delete/✖). The `accent_primary` "+" add buttons are a separate `colored_button`
  case, left for now.
- [x] **P2-3 — `enum_combo` (Tier B).** Promoted `note_fx`'s local `enum_combo`
  to `controls.rs` (now returns `Response` per principle 1, caller-supplied
  `id_salt`), removed the local dup, and routed `note_fx`'s 2 sites through it.
  FLAGGED: the broader ~32 combo sites are **bespoke** — the two most common
  (awe `from_label`+idx+deferred-diff; the patch-bar oversampling combo with
  `.width()` + `selectable_label().clicked()` side-effects) don't fit the
  simple `(variant,&str)` + direct-write shape and need per-site rework + an
  eyeball; `enum_combo` is now the canonical home for new/simple ones.
- [x] **P2-2 — `CaptionTone::{Primary,Color(Color32)}` + colored captions.**
  Extended the enum, then migrated the **8** provably-identical
  `ui.label(RichText::new(x).size(size_small).color(<color>))` sites to
  `caption(.., CaptionTone::Color(c))` (`patch_editor` ×5, `mixer_view` ×2,
  `param_grid` ×1 — accent/runtime tints). SKIPPED: the **22** `.small()`-based
  colored labels (same egui-default-Small ≠ `size_small` size shift as P2-1) and
  any explicitly-non-`size_small` sized labels — visual change, eyeball needed.
- [x] **P2-1 — dim-caption routing.** Migrated the **17** provably-identical
  `ui.label(RichText::new(x).size(10.0|size_small).color(text_dim))` sites to
  `caption(.., CaptionTone::Dim)` (`piano_roll` ×13, `arrangement` ×2, `awe_view`
  ×2 — incl. the "Mix"/"beyond physics" sub-lines). SKIPPED: the `.small()
  .color(text_dim)` variant (`dialogs.rs:535` etc.) — `.small()` uses egui's
  default Small (~9px), NOT theme `size_small` (10px), so migrating would shrink
  text; the theme does not override `style.text_styles`.

### Phase 2 — deferred remainder (triage; needs the user)

After P2-1…P2-4, the **autonomously-safe (provably no-visual-change) subset is
exhausted**. Every remaining idiom from the audit falls into one of three
buckets below. The recurring lesson across P2-1…P2-4: the audit's headline
counts (57/43/49/…) are dominated by sites that are NOT byte-for-byte identical
to a helper call — the clean subset of each idiom is small (8, 17, 9, 2). The
rest genuinely needs a human: either an **in-app eyeball** (the change shifts
pixels) or **per-site judgement** (the sites differ in args/options/side-effects
that no single thin helper captures).

**Bucket V — visual change, needs an in-app eyeball** (do these while running
the app, one helper at a time, comparing before/after):
- `.small()`-based dim/colored captions (~22) — `.small()` ≈ 9px vs `size_small`
  10px. Normalizing to `caption` is a deliberate ~1px restyle.
- Inline colored TOGGLE buttons → `toggle_button_colored` (10) — sites use
  `text_secondary` as the inactive tone vs the helper's `text_dim`; routing them
  changes the inactive colour.
- Inline section headers → `section_header` + optional icon (16) — the AWE "Mix"
  headings use `ui.heading(..)` vs the helper's `ui.label(..).strong()`
  (heading text-style vs bold); not provably identical.
- `labeled_row` (re-add, 49) — the rows use a **default-colour** `ui.label("X:")`;
  the helper's body dims the label, so routing them is a colour change. Decide
  whether labeled_row keeps default or dims, then eyeball.
- Inline strong/bold title labels → `strong_label` (15) — varied sizes/colours.
- Slider-without-suffix → `labeled_slider` (22) — requires redesigning
  `suffix_slider` (optional suffix) which touches its 18 existing call sites;
  bipolar/normalized/`.text()` variants need visual confirmation.

**Bucket B — bespoke per-site, needs per-site judgement** (a single thin helper
can't absorb them without growing past the ~4–5-arg ceiling; migrate
incrementally where one genuinely fits):
- `enum_combo` broad (~32) — `.width()`, `selectable_label().clicked()`
  side-effects, `from_label`, deferred index-diff. `enum_combo` is the home for
  *new/simple* combos.
- `unit_drag_value` (43) — every site has its own range/speed/prefix/suffix and
  often a `%↔0..1` conversion + drag-undo coalescing; a helper would need a
  formatter/callback.
- `action_button` (43), `gated_button` (28), `framed_icon_button` (17),
  `small_button` (12), `status_pill` (17, promote `analyze.rs`'s local `chip()`),
  `colored_button` (the accent `+`), `stepper` (6), `segmented_selector` (5),
  `property_row` (14), `checked_menu_item` (7), `status_label` (5),
  `menu_section_header` (4), `fx_button` (2), `color_swatch` (2),
  `palette_menu_button` (2) — each a real idiom but with per-site content/args.
- Containers: `dialog_window` (extend `modal_window`, 5), `floating_window` (4),
  `position_menu_popup` (2), `list_scroll` (6), `toast` (1).

**Bucket C — correctness-sensitive, needs an eyeball**:
- `inline_editable_text` (8) — the request_focus/commit-on-`lost_focus`|Enter
  handshake. A helper is *worth it* (prevents drift across 4 editors) but it
  changes interaction behaviour, so it must be verified live.

**Recommendation:** the highest-value next move is an **interactive session**
(app running) to do Bucket V's `section_header`-icon + the AWE "Mix" heading,
`labeled_row`, and `inline_editable_text` (Bucket C) with live before/after
comparison — then Bucket B incrementally as those views are touched. The
already-added helpers (`caption` tones, `enum_combo`, `danger_button`) are the
canonical homes new code should use going forward.

### Remaining-work checklist (pick up any item later)

Each item = add/extend the helper, migrate its sites, then the per-step gate
(`fmt`/`build`/`clippy`/`test`) + the noted verification. Counts are the audit's
upper bound; expect the clean subset to be smaller (see the P2 lesson). `[ ]` =
not started.

**Bucket V — needs the app running (in-app before/after eyeball):**
- [ ] `section_header` + optional leading `icon`; migrate the ~16 inline section
      headers **incl. the AWE "Mix" heading** (`ui.heading` vs `.strong()` — the
      one deferred since Step 8). Verify heading weight/size unchanged.
- [ ] Re-add `labeled_row<R>(ui, label, FnOnce)` for the ~49 `horizontal(label +
      control)` rows. **Decide first:** keep the label default-colour or dim it
      (the rows use default `ui.label("X:")`; dimming is a visual change).
- [ ] `.small()`-based dim/colored captions (~22) → `caption` — only if you
      accept the ~1px size normalization (9px→10px). Eyeball a couple.
- [ ] Inline colored TOGGLE buttons → `toggle_button_colored` (~10) — parameterize
      the inactive tone (`text_secondary` vs the helper's `text_dim`); confirm the
      inactive colour looks right.
- [ ] `strong_label(ui, text, Option<Color32>)` for ~15 bold inline titles.
- [ ] `suffix_slider` → `labeled_slider` (optional suffix + `.text()`); touches
      the 18 existing `suffix_slider` call sites — migrate + eyeball the
      normalized/bipolar sliders (~22).

**Bucket C — correctness-sensitive (eyeball the interaction):**
- [ ] `inline_editable_text(ui, &mut String, &mut editing, multiline)` — fold the
      request_focus / commit-on-`lost_focus`|Enter handshake duplicated across the
      4 name/description editors (`piano_roll`, `arrangement`, `pattern_view`,
      `sample_view`). Worth it (prevents drift) but test focus/commit live.

**Bucket B — bespoke per-site (add helper, migrate only sites that genuinely fit):**
- [ ] `unit_drag_value<T>(ui, &mut v, range, speed, prefix, suffix)` — non-seconds
      DragValues (~43); needs an optional `%↔0..1` formatter for the percent ones.
- [ ] `action_button(ui, icon, label, Option<Color32>)` — framed icon+text menu/
      toolbar buttons (~43).
- [ ] `gated_button(ui, enabled, content, disabled_hint, shortcut)` — `add_enabled`
      + reason-hover (~28, incl. Undo/Redo shortcut).
- [ ] `framed_icon_button(ui, icon, color, tooltip)` — default-framed colored glyph
      (~17, transport/toolbar).
- [ ] `status_pill(ui, text, StatusTone)` — promote `analyze.rs`'s local `chip()`;
      route transport/dialog badges (~17).
- [ ] The accent `+` add buttons — give them a **role-named** sibling to
      `danger_button` (e.g. `add_button`), NOT a generic `colored_button(label,
      color)`: the high-effort review judged a colour-parameterized catch-all the
      wrong altitude (it carries no meaning and invites scattering raw colours
      back through call sites — the very thing this audit consolidates).
- [ ] Broad `enum_combo` migration (~32) — only the combos without
      `.width()`/click-side-effects/`from_label`/deferred-diff. **Next clean
      candidates** (review-flagged): `ornament.rs`'s 3 combos (drop their `_name()`
      helpers, use static `(variant,&str)` tables). `note_fx` is now fully
      converted (its StrumDirection combo was done in the review follow-up).
- [ ] Smaller: `small_button` (12), `stepper` (6), `segmented_selector` (5),
      `bypass_checkbox` (6), `property_row` (14), `checked_menu_item` (7),
      `status_label` (5), `menu_section_header` (4), `clickable_label` (3),
      `empty_state` (3), `fx_button` (2), `color_swatch` (2),
      `palette_menu_button` (2).
- [ ] Containers: `dialog_window` (extend `modal_window`, 5), `floating_window`
      (4), `position_menu_popup` (2), `list_scroll` (6), `toast` (1).

**Leave-bespoke (no action — recorded for completeness):** plain default-colour
labels (88), plain text menu/context buttons (58), monospace tracker-grid cells
(16), nested `menu_button` trees (15), labeled checkboxes (9, the repetition is
batch-undo logic not the widget), hand-styled `Frame` containers (6).
