# Cable Routing, Layering, and Aesthetics

Status: PROPOSED

## 1. Motivation

Pertylizer's current patch editor (Rack view) utilizes an **orthogonal (Manhattan) routing** algorithm to draw cables. These cables are rendered **behind modules** (on the background layer). 

While this creates a structured, schematic-like appearance, it suffers from several visual and interactive limitations:
- **Visual Blockage (Occlusion)**: The vertical trunk lines of cables are completely hidden behind module faceplates. Users only see small horizontal "stubs" sticking out of ports, making it difficult to trace signal paths without hovering to highlight them.
- **Rigid Aesthetics**: Physical modular synthesizers (e.g., Eurorack, VCV Rack) are iconic for their loose, hanging patch cables. The sharp right-angled routing feels synthetic and lacks the premium tactile feel of a hardware rack.
- **Hidden Flow Particles**: The animated flow particles showing signal activity travel behind modules, disappearing where they are most needed.
- **Visual Clutter**: In complex patches, overlapping orthogonal lines create "spaghetti clutter" that is difficult to visually parse.

## 2. Proposed Design

To resolve these issues, we propose a comprehensive enhancement to the cable drawing and telemetry subsystem.

### A. Cubic Bézier Curved Routing (Hanging Wires)
Instead of orthogonal lines, we will compute cable paths using a **Cubic Bézier curve** with a gravity-induced downward dip (sag):
- **Start Tangent**: Exits the output port (right-facing) and curves downwards.
- **End Tangent**: Enters the input port (left-facing) from underneath or horizontally.
- **Gravity/Sag**: The depth of the sag is proportional to the distance between ports, creating a natural drape.
- **Backward/Feedback Loops**: If the destination is to the left of the source, the curve arches downwards in a wider loop to avoid passing straight through the modules.

Because the rest of the cable subsystem (flow particles, collision/hover detection, dragging) is built on arbitrary waypoints (`Vec<Pos2>`), changing `calculate_route` in `cable.rs` to return Bézier points automatically upgrades the entire system without breaking existing logic.

### B. Foreground Rendering with Transparency
Instead of drawing cables entirely behind modules, we will draw them **in front of modules** with a configurable opacity/transparency:
- **Default Opacity**: Draw normal cables at ~40–50% opacity (alpha of `100` to `120`). This makes the paths and flow particles fully visible while keeping the underlying text, knobs, and buttons readable and click-active.
- **Hover/Highlight**: When hovered, a cable transitions to 100% opacity with its existing outer glow effect.
- **Configuration**: Add `cable_style` (Orthogonal vs. Curved) and `cable_opacity` settings to the UI Theme/Settings.

### C. Color Gradients for Signal Crossover
Pertylizer allows connections between different but compatible port types (e.g., `Gate` (Green) ➔ `Control` (Orange)). Currently, the cable takes only the source color. 

We will implement **linear color gradients** along the cable length:
- A Gate-to-Control cable will transition smoothly from Green (source) to Orange (destination).
- This visually signals domain conversion (trigger to continuous parameter) at a glance, aids in debugging incorrect connections, and enhances visual appeal.
- Implementation: Interpolate colors segment-by-segment along the 30 computed Bézier waypoints.

### D. Focus Mode (Selection-Based Dimming)
To combat spaghetti clutter in large patches:
- When **no module is selected**: All cables render at default opacity (e.g., 40%).
- When **a module is selected**: Cables connected to the selected module render at 100% opacity; all other cables are dimmed to 10% opacity.

### E. Real-Time Telemetry and Dynamic Particle Behavior
Instead of rendering cosmetic particles at a constant speed, we will feed **real-time signal data** from the audio engine into the particle rendering loop to represent actual signal flow:

1. **Control-Rate (CV) Cables**:
   - Particle speed and direction are mapped directly to the instantaneous voltage of the control signal (e.g., from an LFO or Envelope).
   - If the LFO output is `+1.0`, particles travel forward at maximum speed.
   - If the LFO output is `0.0`, particles pause.
   - If the LFO output is `-1.0`, particles **flow backward** (reversing direction).
2. **Audio-Rate Cables**:
   - Because audio signals oscillate at high frequencies (e.g., 440 Hz), mapping instantaneous values would cause particles to jitter back and forth erratically.
   - Instead, audio-rate cables will keep a constant forward direction speed, but the **particle density, glow intensity, and sizing** will modulate in real-time based on the signal's **RMS/Peak amplitude**. Quiet wires will have thin, dim, sparse particles; loud wires will have bright, thick, dense particles.
3. **Gate/Trigger Wires**:
   - Gate wires will remain completely dark (no particles) when inactive.
   - When a Gate transitions from `0` to `1` (key press), a **burst of dense particles** is "fired" down the cable from the source to the target port, visually showing the trigger traveling in real-time.

---

## 3. Technical Implementation Plan

### Part 1: Update Theme Configurations (`theme.rs`)
Add fields to `Theme` to allow toggling/adjusting the new features:
```rust
pub enum CableStyle {
    Orthogonal,
    Curved,
}

// In Sizes/WidgetStyle struct
pub cable_style: CableStyle,
pub cable_opacity: f32, // 0.0 to 1.0
pub cable_focus_mode: bool,
```

### Part 2: Curved Routing and Gradients (`cable.rs`)
1. Implement the Cubic Bézier generator:
```rust
fn calculate_curved_route(from: Pos2, to: Pos2, spread: f32) -> Vec<Pos2> {
    let mut points = Vec::new();
    let steps = 30;

    let dx = to.x - from.x;
    let dy = to.y - from.y;
    let dist = (to - from).length();

    let pull_x = (dx.abs() * 0.35).max(40.0);

    let sag_y = if dx > 0.0 {
        (dist * 0.20).clamp(15.0, 120.0)
    } else {
        (dist * 0.45).clamp(40.0, 200.0)
    };

    let p0 = from;
    let p1 = Pos2::new(from.x + pull_x, from.y + sag_y + spread);
    let p2 = Pos2::new(to.x - pull_x, to.y + sag_y + spread);
    let p3 = to;

    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let mt = 1.0 - t;

        let x = mt.powi(3) * p0.x
            + 3.0 * mt.powi(2) * t * p1.x
            + 3.0 * mt * t.powi(2) * p2.x
            + t.powi(3) * p3.x;

        let y = mt.powi(3) * p0.y
            + 3.0 * mt.powi(2) * t * p1.y
            + 3.0 * mt * t.powi(2) * p2.y
            + t.powi(3) * p3.y;

        points.push(Pos2::new(x, y));
    }

    points
}
```
2. Implement color interpolation:
```rust
fn interpolate_color(c1: Color32, c2: Color32, t: f32) -> Color32 {
    let r = (c1.r() as f32 * (1.0 - t) + c2.r() as f32 * t) as u8;
    let g = (c1.g() as f32 * (1.0 - t) + c2.g() as f32 * t) as u8;
    let b = (c1.b() as f32 * (1.0 - t) + c2.b() as f32 * t) as u8;
    let a = (c1.a() as f32 * (1.0 - t) + c2.a() as f32 * t) as u8;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}
```
3. Update `draw_segments` to take `from_color` and `to_color` and paint with interpolated segment strokes.

### Part 3: Layering and Focus Mode (`wiring.rs`)
1. Pass destination colors and calculate connection focus based on `selected_module`/`selected_modules`.
2. Move rendering layers to draw cables in front of module faceplates (or adjust sublayers in egui so cables are on a transparent foreground pass).

### Part 4: Real-Time Telemetry Integration
1. **Ljudmotorns skrivning (Audio Thread Writes)**:
   - The audio engine currently has stub/empty `update_output_level` in `SharedGraphState`.
   - To make it real-time safe without taking a lock, the active instrument's output ports will write their block-average CV voltage or Peak/RMS level to an array of `AtomicF32` values.
   - Alternatively, we can piggyback on the existing `ModuleSnapshot::output_levels` by writing to atomic variables associated with the active instrument's modules.
2. **GUI-trådens läsning (GUI Thread Reads)**:
   - During rendering in `draw_connections` (`wiring.rs`), query the current level/activity for the specific source module and output port:
     ```rust
     let live_level = handle.get_port_level(connection.from_module, connection.from_port);
     ```
   - Pass this `live_level` into `draw_flow_particles`:
     - For CV: Adjust `speed` parameter by multiplying with the signal value (allowing negative speed).
     - For Audio: Adjust particle radius and glow alpha by the RMS level.
     - For Gate: If gate changes from 0 to 1, trigger a new particle impulse wave.

---

## 4. Exit Gate

- **Style Selection**: User can toggle between Orthogonal and Curved cables in settings.
- **Visual Correctness**: Curved cables render with natural sag. Flow particles travel correctly along the curves.
- **Signal Crossovers**: Connecting different ports (e.g. Gate ➔ Control) displays a smooth color gradient from source to destination.
- **Foreground Clarity**: Cables render in front of modules; controls underneath remain visible and interactive.
- **Focus Mode**: Selecting a module successfully dims all unrelated cables to the configured low opacity.
- **Real-Time Animation**:
  - Particles on CV cables speed up, slow down, or reverse direction in sync with LFO/Envelope outputs.
  - Particles on Audio cables change brightness/density based on active voice volume.
  - Triggering a note fires a visual pulse down Gate cables.
- **Hit Detection**: Clicking near a curved cable highlights/selects it correctly.
