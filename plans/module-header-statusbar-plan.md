# Patch module — bottom status bar + header zoning

> Goal: declutter the patch-editor module by moving the non-interactive **status
> badges** out of the header into a dedicated **bottom status bar**, fix their
> hover cursor (arrow, not hand), and *investigate* — behind the same branch —
> whether the header should be split into left/right (or center/right) zones so
> the title sits left and the interactive buttons sit right.
>
> **STATUS (branch `feat/module-status-bar`):** Phase 1 **implemented** — code
> complete, build/clippy/fmt/test green, high-effort code-review done + fixes
> applied. Remaining: in-app eyeball (see §1.4) + the colour pass is deferred.
> Phase 2 (header zoning) **not started**. See the per-section ✅ notes below.

## Current state (branch `main`)

Each module is drawn at a **fixed width** (`module_w` = 160/192/256/352/448 px,
`patch_editor.rs:2313`) in three layers:

- **Header** — `draw_module_header` (`widgets/frame.rs:114`). A single flat
  `ui.horizontal` row (`frame.rs:148`):
  `[accent bar 4px] [title] [add_space(xs)] [actions…]`. A gradient mesh washes
  the header background upward to the frame edge (`frame.rs:125-145`).
- **Header actions** — `draw_module_header_actions` (`header.rs:30`). Everything
  **left** of the `ui.separator()` at `header.rs:150` is a non-interactive
  **status badge**; everything **right** is interactive.
  - Status badges (`header.rs:53-145`): source (`UPLOAD_2_FILL`), sink
    (`DOWNLOAD_2_FILL`), automated (`PULSE_FILL`), connectivity
    (`LINK`/`ERROR_WARNING_LINE`/`LINK_UNLINK`/`FLASHLIGHT_FILL`), mod-matrix role
    (`badge_role.glyph()`).
  - Interactive (`header.rs:152-261`): bypass/power, effect-chain reorder
    up/down, info `ⓘ`, overflow `⋯` menu, close `×`.
- **Body** — `draw_module_body` (`node.rs:31`). Already a three-column layout
  (IN ports 28px | stretched content | OUT ports 28px, `node.rs:144-197`), keyed
  off `header_width` so OUT anchors to the right edge.

The status flags are bundled in `ModuleHeaderCtx` (`patch_editor.rs:1060`):
`is_source, is_sink, is_automated, is_bypassed, is_global_module, connectivity`,
plus mod-matrix role derived from `analysis` in `header.rs:124-126`.

### Cursor problem

Status badges are rendered with `icon_button` / `icon_button_sized`
(`widgets/controls.rs:46,61`), which build an egui `Button`. A `Button` senses
clicks, so egui shows the **pointing-hand** cursor on hover — even though these
badges are purely informational (their `.clicked()` is never read). They should
show the **default arrow** cursor.

## Goal

1. A **bottom status bar** on each module that holds all status badges, visually
   mirroring the header but at the bottom of the frame.
2. Status badges hover with the **arrow** cursor, not the hand.
3. The header keeps only the title + interactive controls, and we **evaluate**
   splitting it into zones (left title / right buttons, or center/right) so the
   layout reads as a clean two-zone bar.

## Widget ownership (non-negotiable)

To avoid divergent widget styling, every reusable widget / composite this plan
introduces lives in **`widgets/controls.rs`**; call sites must not hand-roll
`egui::Button`/`Label`/layout primitives:

- **`widgets/controls.rs`** — owns *widgets* and *composite widgets*:
  `status_badge_sized` (the arrow-cursor badge), `right_aligned_row` (the
  right-pin layout idiom), alongside the existing `icon_button` etc.
- **`widgets/frame.rs`** — owns the **`ModuleFrame` chrome only**: the gradient
  wash, margins, separators, and the `draw_module_header` / `draw_module_footer`
  *painters*. These compose `controls.rs` widgets for their contents; they do not
  define new widgets.
- **`header.rs` / `patch_editor.rs`** — own only the *logical contents* (which
  badges apply, click handling), built exclusively from the helpers above.

So the footer painter sits beside `draw_module_header` in `frame.rs` (symmetry),
but every actual widget it draws comes from `controls.rs`.

---

## Phase 1 — Bottom status bar — ✅ DONE (not yet eyeballed)

The substance of the change. Independent of Phase 2.

**What landed (and where it deviated from this plan):**
- ✅ §1.1 `draw_module_status_badges` extracted; header keeps only interactive controls.
- ✅ §1.2 Arrow cursor — but solved **globally in the helper**, not per badge. The
  cursor came from the module card's `Grab`, not the badge widget; forcing
  `on_hover_cursor(Default)` in `icon_button` fixes every icon (header + bar).
- ✅ §1.3 `draw_module_footer` chrome in `frame.rs`. **The gradient wash was added
  then removed** at the user's request — the footer is now just `separator` + row.
  The `fill_gradient_quad` helper extracted for the header wash stays.
- ➕ **Beyond plan (user-driven consolidation):** `status_badge`/`status_badge_sized`
  and `icon_button_sized` were all collapsed into a single
  `icon_button(ui, icon, color, tooltip)` — one hit target (`ICON_BUTTON_SIZE`
  18×20), one glyph size (`theme().fonts.size_normal`), arrow cursor, and the
  tooltip all baked in. `size_module_header` was tried then dropped in favour of
  `size_normal`. No magic size literals left in the patch-editor icon code.
- ➕ **Beyond plan:** the three grey header icons (info / overflow / close) recoloured
  to `accent_primary`. A broader colour pass (group-header icons, close-as-red?) is
  **deferred** — "kolla vidare på färger senare".

### 1.1 Extract a status-badge renderer

Carve the badge block (`header.rs:53-145`) out of `draw_module_header_actions`
into its own function, e.g. `draw_module_status_badges(&self, ui, ctx, analysis)`,
returning nothing (it only draws + tooltips). `draw_module_header_actions` keeps
**only** the interactive controls (`header.rs:152-261`) and drops the now-orphan
`ui.separator()` at `header.rs:150`.

The badge data it needs — `is_source/is_sink/is_automated/is_global_module/`
`connectivity` + mod-matrix role — is already in `ModuleHeaderCtx` + `analysis`.
Decide whether the bottom bar gets its own small ctx struct or reuses
`ModuleHeaderCtx`; reuse is fine since the same struct is already built at the
call site.

### 1.2 Fix the cursor (arrow, not hand)

The badges must not look clickable. Options, simplest first:

- **A (preferred):** render each badge as an `egui::Label`
  (`RichText::new(icon).color(..).size(..)`) with `.selectable(false)` instead of
  a frameless `Button`, then `.on_hover_text(..)`. A `Label` doesn't sense clicks,
  so the cursor stays the default arrow and the tooltip still works.
- **B:** keep `icon_button_sized` but append
  `.on_hover_cursor(egui::CursorIcon::Default)` to each badge response.

Prefer **A** — it's semantically correct (non-interactive = not a button) and
removes the hand intent at the source.

Add a shared helper in `widgets/controls.rs` next to `icon_button` and **use
`ui.add_sized` with the same `min_size` the badges use today** (e.g. 14×20) so the
new `Label` keeps the exact hitbox/geometry of the old `icon_button_sized` badges —
otherwise the row spacing shifts (expert review §2.2):

```rust
pub fn status_badge_sized(
    ui: &mut Ui,
    icon: &str,
    color: Color32,
    icon_size: f32,
    min_size: Vec2,
) -> Response {
    let text = RichText::new(icon).color(color).size(icon_size);
    ui.add_sized(min_size, egui::Label::new(text).selectable(false))
}
```

Returns a `Response` so callers chain `.on_hover_text(..)` (Labels report hover
fine). The connectivity/mod-matrix badges use `icon_button` (default 20×20 square)
today, not `icon_button_sized` — pass `Vec2::splat(ICON_BUTTON_MIN_SIZE)` for those
to preserve their geometry. Shared by header (during transition) and the bottom bar.

### 1.3 Render the bottom bar — own it in `frame.rs` (`draw_module_footer`)

Keep the structural chrome (separator, gradient wash, margins) in
`widgets/frame.rs` to mirror `draw_module_header`, instead of inlining it in
`patch_editor.rs` (expert review §3.1). `patch_editor.rs`/`header.rs` should only
own the *logical contents* (which badges, their tooltips). Add a sibling to
`draw_module_header`:

```rust
pub fn draw_module_footer<F>(ui: &mut Ui, accent_color: Color32, content: F)
where F: FnOnce(&mut Ui) { … }
```

Call it **after** `draw_module_body` returns, inside the `frame.show` closure
(`patch_editor.rs:2377-2392`) — symmetric with the header which is the first thing
in that closure. The footer body is a `ui.horizontal` calling
`draw_module_status_badges`.

- Separator + `add_space(xxs)` up front, mirroring the header's trailing
  separator (`frame.rs:178`).
- Alignment: badges left-aligned is simplest; right-aligned (mirroring where they
  used to sit) needs a `right_to_left` layout (see Phase 2 gotcha).

**⚠ The `max_rect` height trap (expert review §2.1 — critical).** The child UI is
seeded with height `600.0` (`patch_editor.rs:2327`), so `ui.max_rect().bottom()`
is the bottom of the *max allowable space*, **not** the rendered module's bottom.
Use `ui.max_rect()` only for the left/right edges (its width is the correct
`module_w` after `set_width` at `patch_editor.rs:2347`) and derive the vertical
extent from the cursor — exactly how `draw_module_header` already does it
(`frame.rs:127` uses `ui.cursor().min.y`, not `max_rect.top()`):

```rust
let margin = ui.spacing().window_margin;
let module_rect = ui.max_rect();             // correct WIDTH, wrong height — use only L/R
let footer_top = ui.cursor().min.y;          // vertical from cursor, not max_rect
let footer_bottom = footer_top + footer_height + f32::from(margin.bottom);
let footer_rect = egui::Rect::from_min_max(
    egui::pos2(module_rect.left()  - f32::from(margin.left),  footer_top),
    egui::pos2(module_rect.right() + f32::from(margin.right), footer_bottom),
);
```

- **Gradient wash (symmetry):** mirror the header mesh (`frame.rs:125-145`)
  downward — the header fades tint→transparent top-to-bottom; the footer fades the
  other way (`left_top = tint*0.35`, `left_bottom = tint`) so the wash hugs the
  bottom edge. Lives inside `draw_module_footer`. Skip the wash in the first cut if
  fiddly; the rect math above is what matters.

Edge cases:
- **Global modules** (Effect/Visualizer, `node.rs:62-132`) take the no-port body
  path and currently still show connectivity/global badges. The bottom bar must
  render for them too — don't gate it on `is_global`.
- **No active badges:** when a module has zero badges to show (rare — connectivity
  always renders one), skip the separator + row so we don't leave an empty bar.
- The module's measured `panel.size` (read back at `patch_editor.rs:2398`) will
  grow by the bar's height; confirm auto-layout still packs sanely.

### 1.4 Verify Phase 1

- `cargo fmt --check && cargo build && cargo clippy --all-targets && cargo test`.
- Run the app (`/run` or `/verify`), open the patch editor, and check via the
  `egui` MCP + a screenshot:
  - Status badges now sit in a bottom bar, header holds only title + interactive
    buttons.
  - Hovering a badge shows the **arrow** cursor and still shows its tooltip.
  - Hovering an interactive header button still shows the **hand** cursor.
  - A source module, a sink, an orphaned module, an automated module, a
    mod-matrix source/dest, and a global Effect module all render their badges in
    the bar correctly.
  - Narrow (`ExtraSmall` 160px) and wide (`ExtraLarge` 448px) modules both look
    right; no clipping.
  - **Mod-matrix Macro Source Rail** (and per-knob mod-source markers) stays
    aligned after the frame grows by the bottom-bar height — confirm it doesn't
    shift or detach (expert review §4).

---

## Phase 2 — Header zoning (investigation + likely implementation) — ⛔ NOT STARTED

Once the badges leave the header (Phase 1), the header is just `title + interactive
buttons`. Evaluate splitting it so the title is left and the buttons are right.

### 2.1 Options to try

- **Left/right (recommended first try):** keep one row but draw `actions(ui)` in a
  `ui.with_layout(egui::Layout::right_to_left(Align::Center), …)` so buttons pin to
  the right edge and the title gets the slack. Replaces the fixed `add_space(xs)`
  at `frame.rs:172`.
- **Center/right or left/center/right:** only worth it if we want header sub-zones
  vertically aligned over the body's IN/center/OUT columns (28px sides). Likely
  **too tight on 160px** modules (144px usable − 2×28 ≈ 84px center can't hold the
  title). Treat as a spike; probably reject in favor of left/right.

### 2.2 Known gotcha — `right_to_left` reverses widget order

A `right_to_left` layout lays widgets out from the right, so the **first**
`ui.add` lands rightmost. `draw_module_header_actions` currently draws in reading
order (bypass → reorder → info → ⋯ → ×); under naive `right_to_left` that mirrors
to `× ⋯ ⓘ … bypass`. Handle by nesting: an outer `right_to_left` that allocates
the right slack, with an inner `ui.horizontal` for the actual buttons so their
reading order is preserved. Don't reverse the source order (expert review §3.2).

Per **Widget ownership**, this nested idiom is itself reusable, so it goes in
`controls.rs` as a composite — `draw_module_header` calls it, no inline layout:

```rust
// widgets/controls.rs
pub fn right_aligned_row<R>(ui: &mut Ui, content: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.horizontal(|ui| content(ui)).inner   // preserves L→R reading order
    })
    .inner
}
```

`draw_module_header` then replaces the fixed `add_space(xs)` at `frame.rs:172`
with `right_aligned_row(ui, |ui| actions(ui))`.

### 2.3 Degradation

When title + buttons already fill the row (long title on a narrow module), the
slack is zero and it degrades to today's behavior — nothing clips worse than now.
Confirm this with a deliberately long-named module.

### 2.4 Verify Phase 2

- Same gate (`fmt/build/clippy/test`).
- Screenshot check: title hard-left, buttons hard-right, sensible gap; button
  order unchanged (× still rightmost); long-title narrow module doesn't clip more
  than before.

---

## Out of scope

- The body three-column layout (`node.rs:144-197`) is unchanged.
- No change to `ModuleWidth` buckets / fixed widths.
- Tooltips' text content is unchanged — only their host widget (Phase 1.2) and
  position (Phase 1.3) change.

## Suggested commits

1. `feat(widgets): add status_badge_sized (arrow cursor) + right_aligned_row to controls.rs`
2. `refactor(patch): extract draw_module_status_badges from header actions`
3. `feat(widgets): add draw_module_footer chrome to frame.rs (mirrors header)`
4. `feat(patch): render module status badges in a bottom status bar`
5. `feat(patch): right-align interactive header controls (left title / right buttons)`

(Spike for left/center/right, if attempted, folds into commit 3 or is dropped.)
