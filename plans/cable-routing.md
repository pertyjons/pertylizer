# Cable Rendering: Foreground, Gradients, and Focus Mode

Status: PROPOSED (scoped down from an earlier draft)

## 0. Scope

This is a deliberately small, low-risk pass over the patch-editor cable
rendering. It covers three changes that all reuse infrastructure that already
exists:

- **B. Foreground rendering with configurable opacity** — draw cables (and their
  flow particles) in front of module faceplates at reduced opacity so signal
  paths stay visible without hiding the controls underneath.
- **C. Source→destination color gradient** — paint each cable as a gradient from
  its source port color to its destination port color, so cross-domain
  connections read at a glance.
- **D. Focus mode** — when a module is selected, keep cables touching it at full
  opacity and dim all others.

### Explicitly out of scope

Two ideas from the earlier draft are **not** part of this plan:

- **Cubic Bézier "hanging wire" routing** — pure aesthetics; the orthogonal
  routing is fine and works with every downstream consumer. If wanted later it
  can go behind a `CableStyle` toggle without touching anything here (all four
  public cable functions already route through the single private
  `calculate_route` in `cable.rs`, so a routing swap is localized).
- **Real-time telemetry driving particle speed/density from live signal
  levels** — this needs an entire audio-thread producer path that does not
  exist today (`SharedGraphState::update_output_level` is defined but never
  called), and doing it correctly runs into real-time-safety rules (no locks /
  no allocation on the audio thread — the `HashMap<PortName, Amplitude>` behind
  a `RwLock` is not usable from `process()`) and an undecided polyphony question
  (the editor shows one instrument's graph but it is played by up to N voices —
  "the" level per port is undefined). Left for a separate, properly-designed
  effort.

## 1. Motivation

The current patch editor uses **orthogonal (Manhattan) routing** and renders
cables **behind modules** on the background layer (`cable.rs:1-6`,
`wiring.rs:205`). That has three concrete problems:

- **Occlusion**: the vertical trunk of each cable is hidden behind module
  faceplates; only the short horizontal stubs at the ports are visible, so
  tracing a signal path requires hovering to highlight it.
- **Hidden flow particles**: the animated activity particles are drawn on the
  background painter (`wiring.rs:341`), so they disappear behind exactly the
  modules whose I/O they describe.
- **Invisible domain crossings**: Pertylizer allows compatible cross-type
  connections — `Audio ↔ Control` and `Gate → Control` (`PortType::can_drive`,
  `module_traits.rs:1034`). But the cable is painted using the **source port
  color only** (`cable_color(info.port_type, 180)` where
  `info.port_type = from_pos.port_type`, `wiring.rs:285,327`); the destination
  port type is resolved and then discarded. So a Control→Audio cable renders
  solid orange even though it lands on a blue port — the crossover is
  invisible.
- **Clutter**: in large patches every cable is drawn at the same weight with no
  way to focus on one module's connections.

## 2. Current state (verified, for the implementer)

- `draw_connections` (`wiring.rs:205`) already builds **two painters**: a
  `bg_painter` behind modules and an `fg_painter` on a sublayer *above* the
  scene that carries the same pan/zoom transform (`wiring.rs:216-223`). Today
  only the hovered / context-menu-target cable uses `fg_painter`; every other
  cable and **all** particles use `bg_painter`. The foreground machinery this
  plan needs is therefore already present and proven correct.
- Painter shapes are **non-interactive** — egui only routes pointer input to
  widgets, not to painted shapes. Drawing cables in the foreground does **not**
  block clicks on the knobs/buttons underneath, so the "controls stay
  interactive" requirement holds for free.
- `draw_cable` currently hardcodes the main-cable alpha to `160` and adds a drop
  shadow (`cable.rs:231`, `CABLE_SHADOW`). Opacity is baked in, so it must be
  parameterized.
- Both endpoint port types are already available per cable: `from_pos.port_type`
  and `to_pos.port_type` in the `cable_infos` build loop (`wiring.rs:266-287`).
  `CableInfo` currently stores only the source type.
- Selection state exists: `selected_module: Option<ModuleId>` and
  `selected_modules: HashSet<ModuleId>` (`patch_editor.rs:1106-1107`).

## 3. Technical Implementation Plan

### Part 1 — Settings (`theme.rs`)

Add fields next to the existing `cable_thickness` in the theme `sizes` struct
(consistent with where cable styling already lives; a later refactor could move
these to `synth_config` if they should persist per-user):

```rust
pub cable_opacity: f32,   // default ~0.45 — base opacity for normal cables
pub cable_focus_mode: bool, // default true
pub cable_focus_dim: f32, // default ~0.10 — opacity for cables not touching the selection
```

No `CableStyle` enum — routing is unchanged.

### Part 2 — Opacity + gradient in the cable primitives (`cable.rs`)

1. **Parameterize opacity.** Change `draw_cable` and `draw_flow_particles` to
   honor the alpha of the color they are handed instead of overriding it to a
   constant. The caller (`draw_connections`) becomes the single owner of a
   cable's resolved alpha.
2. **Gradient.** Give `draw_cable` (and, for consistency,
   `draw_cable_highlighted`) a `from_color` and a `to_color` instead of one
   `color`. Add a gradient variant of `draw_segments` that walks the waypoints,
   accumulates path length (reuse `path_length`), and strokes each segment with
   an interpolated color at its midpoint `t`:

   ```rust
   fn interpolate_color(c1: Color32, c2: Color32, t: f32) -> Color32 {
       let lerp = |a: u8, b: u8| (f32::from(a) * (1.0 - t) + f32::from(b) * t) as u8;
       Color32::from_rgba_unmultiplied(
           lerp(c1.r(), c2.r()),
           lerp(c1.g(), c2.g()),
           lerp(c1.b(), c2.b()),
           lerp(c1.a(), c2.a()),
       )
   }
   ```

   When source and destination port types match, `from_color == to_color` and
   the gradient degenerates to a solid stroke — **no special-casing needed**.
3. **Particle color.** `draw_flow_particles` already computes each particle's
   distance along the path; color it with the same source→dest interpolation
   (`t = dist / total_len`) so particles fade across the crossover too.
4. **Shadow.** The drop shadow reads as mud when a translucent cable sits over a
   module faceplate — drop it (or scale its alpha with `cable_opacity`) in the
   foreground path.

### Part 3 — Wiring it together (`wiring.rs`)

1. Extend `CableInfo` with the destination port type (`to_port_type`) captured
   from `to_pos.port_type` in the build loop.
2. Resolve a single **alpha per cable** in the draw loop:
   - hovered / menu-target → full opacity via the existing
     `draw_cable_highlighted` glow path (unchanged);
   - otherwise, if `cable_focus_mode` **and** there is a selection
     (`selected_module.is_some() || !selected_modules.is_empty()`): full
     `cable_opacity` when `from_module` **or** `to_module` is in the selection,
     else `cable_focus_dim`;
   - otherwise `cable_opacity`.
3. Route **all** non-hovered cables and their particles through the existing
   `fg_painter` (instead of `bg_painter`) at the resolved alpha, so trunks and
   particles are no longer occluded. Compute `from_color`/`to_color` with
   `cable_color(from_type, alpha)` / `cable_color(to_type, alpha)`.

Routing, hit detection, and drag-to-connect are untouched — `calculate_route`
and all its callers stay as-is.

## 4. Exit Gate

- **Foreground clarity**: normal cables and their flow particles render in front
  of modules at `cable_opacity`; knobs and buttons underneath remain visible and
  clickable.
- **Signal crossovers**: a cross-type cable (`Audio ↔ Control`, `Gate →
  Control`) shows a smooth source→destination color gradient; a same-type cable
  stays a solid color.
- **Focus mode**: with focus mode on and a module selected, cables touching that
  module stay at full opacity and all others dim to `cable_focus_dim`; toggling
  focus mode off (or clearing the selection) restores uniform opacity.
- **Hover**: the nearest cable still highlights with its glow at full opacity in
  the foreground layer.
- **Hit detection**: clicking near a cable still highlights/selects it (routing
  unchanged).
