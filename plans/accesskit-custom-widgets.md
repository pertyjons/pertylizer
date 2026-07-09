# Plan: AccessKit exposure for custom-painted widgets

Make every custom-painted widget in Pertylizer visible to **AccessKit**, and
therefore to the **egui-inspection MCP** (`query_tree` / `content_contains` /
`click` / `drag`). Today **zero** widgets call `widget_info`, so custom painter
widgets (knobs, ports, cables, node cards, canvases, the keyboard, meters) show
up as `Unknown` with no label or value in the accessibility tree — the exact gap
the `egui_mcp` spike hit (see memory `project_egui_mcp_inspection`).

This is a **standalone branch off `main`** (`feat/accesskit-custom-widgets`) so
it lands independently and is then merged into `feat/note-grid`: the Note Grid
Scene canvas reuses the patch-editor's node/port/cable rendering, so fixing those
here gives the Note Grid GUI (plan phase 5) MCP visibility **for free**.

---

## 0. Background — how egui exposes a widget (verified against egui 0.35)

The MCP inspector reads the **AccessKit tree**. A widget enters that tree only
when something fills its AccessKit node. Standard egui widgets do it for you;
custom painter widgets do not — hence `Unknown`. Two mechanisms exist:

1. **`Response::widget_info(|| WidgetInfo)`** (`egui/src/response.rs:849`) — the
   ergonomic path. Internally calls `accesskit_node_builder` **and**
   `register_widget_info`, filling the node with the `WidgetInfo`'s
   `typ`/`label`/`value`/`selected`/… Crucially, it fills the node **every
   frame** in its `else` branch (when no click/change event fired), so a widget
   that calls it each frame is always present in the tree — not only on the frame
   it changes.
2. **`Context::accesskit_node_builder(id, |node: &mut accesskit::Node| …)`**
   (`egui/src/context.rs:3582`) — the low-level escape hatch: write arbitrary
   AccessKit node fields directly. Returns `None` when AccessKit is off (so it is
   free when inspection is disabled). Needed only for pure-paint elements that
   have **no `Response`** (cables) or that need roles/relations `WidgetInfo`
   cannot model.

`WidgetInfo` fields are all `pub` (`typ`, `enabled`, `label`,
`current_text_value`, `selected`, `value: Option<f64>`, `hint_text`, …). The MCP
matches `content_contains` against **label OR value**, and many read-only
widgets carry their text in `value`, so exposing a numeric `value` makes meters /
scopes queryable. `WidgetType` variants (`egui/src/lib.rs`): `Label`, `Link`,
`TextEdit`, `Button`, `Checkbox`, `RadioButton`, `RadioGroup`, `SelectableLabel`,
`ComboBox`, `Slider`, `DragValue`, `ColorButton`, `Image`, `CollapsingHeader`,
`Panel`, `ProgressIndicator`, `Window`, `ResizeHandle`, `ScrollBar`, `Other`.

---

## 1. The shared helpers (the generalization)

Per CLAUDE.md, small reusable widget idioms live in
`crates/pertylizer/src/gui/widgets/controls.rs`. Add there:

```rust
/// Expose a custom-painted, `Response`-backed widget to AccessKit — and thus to
/// the egui-inspection MCP (`query_tree`, `content_contains`, `click`, `drag`).
/// Call once per frame, right before returning the `Response`. `value` lands in
/// the node's `value` field (matched by the MCP's `value_contains`); pass `None`
/// for non-numeric widgets. Because egui's `widget_info` fills the node every
/// frame (not just on change), the widget is always present in the tree.
pub fn expose(
    response: &egui::Response,
    typ: egui::WidgetType,
    label: impl Into<String>,
    value: Option<f64>,
) {
    let label = label.into();
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(typ, response.enabled(), label.clone());
        info.value = value;
        info
    });
}

/// Variant for on/off / active-tab widgets: reports `selected` state so the MCP
/// can read which tab/toggle is active.
pub fn expose_selected(
    response: &egui::Response,
    typ: egui::WidgetType,
    label: impl Into<String>,
    selected: bool,
) {
    let label = label.into();
    response.widget_info(|| {
        egui::WidgetInfo::selected(typ, response.enabled(), selected, label.clone())
    });
}

/// Escape hatch for pure-paint elements that have NO `Response` (cables) or need
/// AccessKit roles/relations `WidgetInfo` cannot model. No-op when AccessKit is
/// off. `id` must be stable across frames for the node to persist.
pub fn expose_painted(ui: &egui::Ui, id: egui::Id, bounds: egui::Rect, label: &str) {
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_bounds(accesskit::Rect {
            x0: bounds.min.x.into(), y0: bounds.min.y.into(),
            x1: bounds.max.x.into(), y1: bounds.max.y.into(),
        });
        node.set_label(label);
    });
}
```

Design rules for every call site:
- **Call unconditionally, every frame**, right before the widget's `Response` is
  returned — never gated behind `if response.clicked()`.
- **Label must be meaningful and stable** — the human/MCP name (module name, port
  name+type, tab name), not a generic "knob".
- **`value` carries the live scalar** where one exists (knob value, meter dB),
  so `value_contains` and value assertions work.
- Prefer the specific `WidgetType` (`Slider`, `SelectableLabel`, `Button`) over
  `Other`; use `Other` only when nothing fits.

### 1.1. Retrofit the shared `icon_button` helper (one change, wide cascade)

`controls::icon_button` (`controls.rs:79`) renders a Remix Icon **glyph as the
button text** (e.g. `ri::CLOSE_LINE` = `\u{EE29}`). It inherits standard `Button`
widget info, so egui fills the AccessKit **label with that raw unicode
codepoint** — the MCP cannot match, query, or `click` these by name (a "Mute"
button reads as `\u{EE29}`). Because `mute_toggle`, `solo_toggle`,
`bypass_toggle`, delete/close buttons and toolbar toggles all delegate to
`icon_button`, overriding the label **once here** makes every one of them
queryable by readable text.

Use the first line of the existing `tooltip` (already human-readable, e.g.
`"Muted\nOutput is silenced.\n…"` → `"Muted"`) as the accessible label:

```rust
let response = ui.add(/* existing Button */);
let clean_label = tooltip.lines().next().unwrap_or(tooltip);
controls::expose(&response, egui::WidgetType::Button, clean_label, None);
response.on_hover_text(tooltip).on_hover_cursor(egui::CursorIcon::Default)
```

Note the tooltip's first line is state-dependent for the toggles (`"Muted"` vs
`"Audible"`), which is exactly the label a driver wants. This lands in Phase 1
alongside the helper definitions since it *is* a shared-control change.

---

## 2. Inventory & call sites

All line numbers are anchors at plan-writing time — re-locate before editing.
Every "interactive" site below already produces a `Response`; the change is one
`controls::expose(...)` line before the existing return/use.

### 2.1. Reusable widgets (`widgets/`) — **do these first**

These are shared and are what the Note Grid Scene canvas will reuse.

| Widget | File:line | `WidgetType` | label | value |
|--------|-----------|--------------|-------|-------|
| **Knob** | `widgets/knob.rs:156` → before `response` (~:299) | `Slider` | `self.label` (fallback: accent/param name) | `Some(*self.value as f64)` |
| **Port** | `widgets/port.rs:103` → before `(response, center)` (~:158) | `Other` | `"{name} · {port_type:?}"` | — (`expose`, or set `selected = connected`) |
| **Waveform button** | `widgets/waveform.rs:120` | `Button` | waveform name | — |
| **Envelope editor** (interactive ADSR) | `widgets/envelope.rs:184` | `Other` | `"ADSR editor"` | — (optionally per-handle later) |

### 2.2. Patch editor (`patch_editor/`, `patch_editor.rs`)

Reused by the Note Grid Scene canvas — high priority.

| Element | File:line | `WidgetType` | label |
|---------|-----------|--------------|-------|
| **Node card** | `patch_editor.rs:2409` (`node_response`) | `Button` | module display name |
| **Canvas background** | `patch_editor.rs:2228` (`canvas_bg`) | `Panel` | `"patch canvas"` |
| **Node input/output ports** | `patch_editor/ports.rs:61, 138` (`port_resp`) | `Other` | `"{node} · {port} in/out"` |
| **Node close button** | `patch_editor/ports.rs:186` (`close_resp`) | `Button` | `"close {node}"` |
| **Group box** | `patch_editor/groups.rs:411` | `Other` / `Panel` | group name |
| **Effect-chain / node bars** | `patch_editor/node.rs:475` (hover rect) | `ProgressIndicator` | bar label | 

**Cables** (`patch_editor/wiring.rs` `draw_cable`, `draw_cable_highlighted`) are
**pure paint with no `Response`**. Decision: **do not** make each cable a tree
node in v1. Instead encode the connection on the two port nodes (put
`"→ {dest}"` / `"← {src}"` in the port label, or set AccessKit relations via
`expose_painted`). The MCP rarely needs to click a cable; it needs to *read* the
topology, which the port labels give. A cable-as-node pass is optional follow-up.

### 2.3. Top-bar view selector (`egui_backend.rs`)

| Element | File:line | `WidgetType` | note |
|---------|-----------|--------------|------|
| **View tabs** (Home/Rack/AWE/Pattern/Seq/Mixer/Sample) | `egui_backend.rs:3694` (`resp`, in loop) | `SelectableLabel` | `expose_selected(&resp, …, label, is_active)` |

This one is high-value for MCP navigation — it lets a driver switch views by name
instead of by pixel.

### 2.4. Big interactive canvases (single-response widgets)

These allocate **one** large `Response` and resolve sub-elements by pointer
position. v1 scope: **expose the container** with a descriptive label so it is
present and locatable in the tree. Per-element MCP drivability (clicking an
individual note / clip / key) needs per-element `ui.interact(sub_rect, …)` and is
**explicitly out of scope** here — a larger, view-specific effort noted per view.

| Canvas | File:line | container label |
|--------|-----------|-----------------|
| **Piano roll** | `sequencer/piano_roll.rs:1455` | `"piano roll canvas"` |
| **Tracker** | `sequencer/tracker.rs:1224` (`draw_tracker`, the `TableBuilder` body/container) | `"tracker canvas"` |
| **Arrangement** | `sequencer/arrangement.rs:574` | `"arrangement canvas"` |
| **Arrangement tempo lane bg** | `arrangement.rs:1283` (`lane_bg`) | `Panel`, `"tempo lane"` |
| **Arrangement tempo point handles** | `arrangement.rs:1365` (per-point `resp`) | `Slider`, `"tempo point {i}"`, `value = Some(bpm)` |
| **Track color swatches** | `arrangement.rs:2284` (per-preset `resp` in the header popup) | `ColorButton`, `"track color {preset:?}"` |
| **AWE room** | `awe_view.rs:1058` | `"acoustic world canvas"` |
| **Sample waveform** | `sample_view.rs:697` | `"sample waveform"` |

The tracker cells (notes, ornaments, automation inputs, row headers) are
raw-painted inside `TableBuilder` rows and are `Unknown` today. v1 scope, like the
other big canvases, is the **container** label only (`"tracker canvas"`);
per-cell drivability is the same deferred per-element effort noted above. The
track color swatches and tempo points, by contrast, are already **discrete
per-element responses**, so they get full `expose` calls (clickable/queryable by
name — the color swatches make MCP-driven track recoloring possible).

**Keyboard** (`keyboard.rs:240`, one `inner_response`, keys resolved by
`hover_pos`): expose the container as `"keyboard"` and put the currently
hovered/pressed note in `value`. **Per-key clicking via MCP is out of scope** —
it would require a `ui.interact(key_rect, …)` per key; add only if a test needs
it.

### 2.5. Read-only visualizers — expose value only

These allocate a hover rect and currently **discard** the response
(`let (rect, _response) = …`). Change `_response` → `response` and call
`expose(&response, WidgetType::ProgressIndicator, label, Some(current_value))`.
Ordered by value:

| Visualizer | File:line | value to expose |
|------------|-----------|-----------------|
| **Meter** (widget + strips) | `widgets/meter.rs:34, 83, 176` | peak/RMS dB |
| **Mixer channel meter** | `mixer_view.rs:436` | channel level dB |
| **Sample meter** | `sample_view.rs:447` | level dB |
| **Panel meters** | `panels/meters.rs:12, 49` | level dB |
| **Spectrum** | `widgets/spectrum.rs:23` | — (label only, or peak-bin Hz) |
| **Scope** | `widgets/scope.rs:17, 70` | — (label only) |
| **Static envelope preview** | `widgets/envelope.rs:19` | — (label only) |
| **Analyze charts** | `analyze.rs:716, 1461, 1771` | metric value where meaningful |
| **Frame accent stripe** | `widgets/frame.rs:147` | — (skip; decorative) |

Decorative-only sites (`frame.rs:147`) can be **skipped** — exposing pure chrome
adds tree noise without test value. Call that out per-site rather than blanket
exposing everything.

---

## 3. Phasing

Each phase builds clean (`cargo build`/`clippy`/`test`/`fmt`) and is committed
separately; the `widgets/` and `patch_editor/` phases are the ones Note Grid
depends on, so they come first.

1. **Helpers + shared controls + reusable widgets** (`controls.rs` helpers §1;
   the `icon_button` retrofit §1.1 — cascades to mute/solo/bypass/close/toolbar;
   §2.1 knob/port/waveform/envelope). This is the core Note-Grid-relevant surface.
2. **Patch editor** (§2.2: node cards, canvas bg, ports, close, groups, bars).
   Cables handled via port labels, not as nodes.
3. **View selector** (§2.3) — cheap, high navigation value.
4. **Read-only visualizers** (§2.5), meters first.
5. **Big canvases & timelines** (§2.4) — Piano Roll / Tracker / AWE / Sample /
   Arrangement container labels; plus the discrete Arrangement tempo lane +
   points and track color swatches (full `expose`). Per-element canvas
   drivability explicitly deferred, noted per view.

Phases 1–2 are the merge target for `feat/note-grid`; 3–5 are independent polish
that can land on `main` on their own.

---

## 4. Verification (egui-inspection MCP)

Nothing here has meaningful unit tests — it is verified by driving the running
app, the same way the original spike measured coverage:

1. Run the app with `EGUI_INSPECTION=1` (binds the inspector; the opt-in
   `egui-inspection` cargo feature is already wired, see
   `project_egui_mcp_inspection`).
2. `attach`, then `query_tree` on each view and confirm the previously-`Unknown`
   nodes now carry the expected `label`/`value`.
3. Spot-check drivability: `click` a view tab by name switches views; `click` a
   port/node card by locator resolves to exactly one node; a meter's `value`
   reads a plausible dB.
4. Regression note: confirm `widget_info` is emitted every frame (node present in
   `query_tree` without first interacting) — the `else`-branch behavior in §0.

Gotchas from the spike, still true:
- Custom painter widgets that never call `widget_info` stay `Unknown` — so a
  missed call site is silently invisible; sweep §2 exhaustively.
- egui-mcp cannot drive `Response::dragged()` synthetically for some widgets, so
  drag-only interactions may read but not fully actuate — acceptable for v1.

---

## 5. Merge into `feat/note-grid`

After this branch is green and eyeballed, squash-merge it to `main`, then merge
`main` into `feat/note-grid` (or cherry-pick phases 1–2). The Note Grid Scene
canvas, when extracted from `PatchEditor` (Note Grid plan §8 / phase 5),
instantiates the same node/port/cable rendering — so it inherits the
`controls::expose` calls added in §2.1–2.2 with no extra work, closing the
`Unknown`-node gap for both the patch editor and the Note Grid view in one pass.
