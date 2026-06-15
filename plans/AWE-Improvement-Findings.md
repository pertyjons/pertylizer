# AWE Improvements - Findings and Concrete Ideas

This document summarizes concrete ideas for improving the AWE experience, based on the current
implementation in:

- `crates/pertylizer/src/gui/awe_view.rs`
- `crates/synth_awe/src/awe_engine.rs`
- `crates/synth_awe/src/early_reflections.rs`
- `crates/synth_awe/src/room_modes.rs`
- `crates/synth_dsp/src/fdn.rs`
- `crates/synth_awe/src/spatializer.rs`

The ideas are grouped by the TODO Priority 4 sections: room visualization and effect
differentiation (visual and audio).

---

# Reality check (2026-06-15)

A code-level deep-dive into the whole AWE module — DSP engine, visualization, and the
editing UI. This section reframes the idea catalog below: it records **what the module
actually is today** (not what its vocabulary implies) so the improvement work targets the
real gaps. Three findings, then a strategic recommendation.

## A. The DSP is a hybrid emulation, NOT ray-tracing

The original vision was "3D ray-tracing for sound." The implementation is a small, genuine
geometric front-end bolted onto a classic statistical reverb. Signal chain:
pre-delay → early reflections (ISM) → late reverb (FDN) → room modes → mix → air absorption →
spatializer → EQ → portal → dry/wet (`awe_engine.rs:199-406`).

- **Early reflections = real but minimal physics.** Genuine image-source method: mirror
  sources from actual source/listener positions, distance→delay via speed of sound,
  per-reflection frequency-dependent absorption. Moving the source really changes delays/gains.
  **But only 6 taps, first-order, hardcoded to a rectangular box** — the mirror equations in
  `early_reflections.rs` always assume 6 orthogonal walls regardless of room shape. This is a
  *slice* of geometric acoustics, not a simulation (true ray-tracing traces many higher-order
  bounces off real surfaces to build an impulse response).
- **Late reverb (FDN) = pure emulation.** 8-channel Schroeder FDN with prime-number delay
  times chosen for perceptual smoothness (`fdn.rs:14`: `[2039, 2311, 2543, 2719, 2917, 3109,
  3301, 3511]`), not derived from geometry. The room affects it only by uniformly scaling all
  8 delays by the average dimension and setting one feedback gain from RT60 (Eyring). This is
  the bulk of what you hear on high wet — and it traces no geometry at all.
- **Room modes = physically correct, but rectangular.** Real wave-equation formula
  `f = c/2·√((nx/L)²+(ny/W)²+(nz/H)²)` (`room_modes.rs:175-189`), 12 axial/tangential/oblique
  modes. Correct — but assumes a box.
- **Speed of sound / temperature = correct.** ISO formula `331.3 + 0.606·°C`
  (`types.rs:198-202`), propagates into delays and mode frequencies.

### The biggest lie: room shapes are acoustically near-cosmetic

This is the most severe "visual ≠ sound" gap in the module. Cylinder, Sphere, Dome, L-Shape
and Tube are rendered as full 3D spaces, but **in the audio path every shape is flattened to
its bounding box.** Shape affects *only* RT60, mode frequencies, and portal delay (all via
volume / average dimension). The early reflections always use 6 rectangular mirrors and the
FDN topology is shape-independent. A user selecting "Sphere" hears a box of the same volume —
no curved-surface focusing, no degenerate modes, no different modal density. The shape picker
is ~90% cosmetic to the ear.

### Other engine-vs-vocabulary mismatches

- **Materials are uniform across all surfaces** — no per-wall assignment (see §7 "per-surface
  materials" below). All 6 walls get identical absorption coefficients.
- **Several controls are pure effects, not physics:** Portal (synthetic delayed-feedback with
  fixed constants), Freq Warp (tooltip itself says "not physically realistic"), Diffusion
  (= jitter on tap delays, not real scattering).

**Verdict (honest):** ~60% physically-grounded front-end, ~40% perceptual emulation. A
*reasonable* engineering trade-off (real-time ray-traced reverb is an offline-grade cost), but
not a unified geometric simulation — and the UI/visualization promise more realism than the
DSP delivers.

| Aspect | Mechanism | Physically grounded? |
|--------|-----------|----------------------|
| Early reflections | ISM, 6 first-order, box | Genuine but minimal |
| Late reverb (FDN) | Schroeder statistical | Emulation (only RT60 scaling is physics) |
| Room modes | comb filters, wave equation | Correct, assumes box |
| **Room shapes** | **flattened to bounding box in audio** | **~cosmetic to the ear** |
| Temperature / speed of sound | ISO formula | Correct |
| Materials | uniform absorption | Partial (no per-surface) |
| Portal / Freq Warp / Diffusion | effect knobs | Not physics |

## B. The visualization is cosmetic, not data-driven

`awe_view.rs` (~2777 lines). The room render + sound rings + reflection "marching ants" are
driven by **only two inputs**: output peak meters (volume) and a wall-clock timer. The
visualization reads **zero** acoustic parameters from the AWE engine.

```rust
// awe_view.rs:1072 — the ONLY signal read from the audio:
let (peak_l, peak_r) = handle.peak_meters();
let audio_level = peak_l.as_f32().max(peak_r.as_f32()).clamp(0.0, 1.0);
```

- Sound rings spawn faster at higher volume and grow on a fixed 2.5 s timer — unrelated to
  room size, decay, or reflections.
- Reflection lines recompute mirror geometry *from scratch in the GUI*, ignoring the engine's
  real taps (delay/gain/damping); animation speed is just `30 + 60·volume`.
- Material has **no** visual representation (only a static RT60 number in an info box).

So changing absorption or tail does not change the picture. This directly violates the doc's
own acceptance criterion #4 ("reflection visuals should encode both path timing and
attenuation, not only input level").

### Feasibility of an honest data channel — EASY-to-MODERATE (~200 LOC)

The fix is well-scoped because the data already exists and there's a proven pattern to copy:

- The reflection taps are persistent struct fields, already computed: `EarlyTap { delay_samples,
  gain_left, gain_right, lp_coeff, hp_coeff, … }` × 6 (`early_reflections.rs:37`,
  `taps: [EarlyTap; 6]`). Recomputed only on geometry change, not per-sample. (Pan is baked
  into gain_left/right — L/R balance *is* the pan, no extra field needed.)
- Mirror the existing `peak_meters()` channel: `Arc<EngineState>` with `AtomicF32` fields the
  audio thread writes (`MeterState::update_peak` in `state.rs`) and the UI reads
  (`synth_engine.rs:271`). Real-time-safe: audio thread just stores already-computed values
  (no alloc, no lock).
- Changes: getter on `EarlyReflections`/`AweEngine` → new `EarlyReflectionTaps` atomics in
  `EngineState` (6 taps × 5 f32 = 30 atomics) → snapshot call per block in `synth_engine.rs`
  → `handle.early_reflection_taps()` → render in `awe_view.rs`.
- The only real design work is the *visualization* (delay→position, gain→brightness/thickness,
  absorption→colour), not the plumbing. = doc §4.1 "Animation improvements" + §4.2 "Reflection
  visualization", finally made honest.

## C. The editing UI is organized by DSP wiring, not by how people think about a room

`draw_controls()` (`awe_view.rs:2114`) exposes ~30 controls in the order Room → Material → Mix
→ Effects → Spatial → LFO 1-4. Pedagogically weak because:

1. **Raw DSP knobs with no real-world anchor** sit next to physical quantities — Freq Warp
   (−1..1), Modes (0..1), Mod Depth/Rate, Portal, Resonance, Tail (×) vs. metres, °C, Hz.
2. **"Effects beyond physics" is a junk drawer** — mixes truly non-physical (Freq Warp, Portal),
   fully physical (Air Absorb, Temperature, Pre-delay, EQ) and DSP-internal (FDN modulation).
3. **Three overlapping "how much room do I hear" controls** (Dry/Wet, Early/Late, Modes) with no
   interaction model.
4. **Material + Diffusion override** creates an ambiguous state: the panel still says "Concrete"
   after the user changes diffusion to something Concrete never has.

### Chosen redesign: progressive disclosure (simple-first)

Principle: follow how a person imagines a space — *How big? → Made of what (hard/soft)? → How
close am I? → (optional) special character* — with **live result feedback** (RT60, volume, a
verbal descriptor) as the headline, reusing the existing RT60 calc.

- **Phase 0 — restructure the panel (no behaviour change, low risk; all in `draw_controls()`):**
  - *Basics (always visible):* Shape, Size, Material, Dry/Wet.
  - *▸ Advanced (collapsed):* break up the junk drawer into perceptual groups — Tone & brightness
    (Diffusion, Air Absorb, High/Low Cut, Temperature); Distance & space (Early/Late, Pre-delay,
    Width); Creative/non-physical (Freq Warp, Portal, Resonance, Modes, Tail); Movement
    (Mod Depth/Rate).
  - *▸ Spatial (collapsed):* Per-voice, Mapping.
  - *▸ Modulation (collapsed):* LFO 1-4.
- **Phase 1 — live result feedback (low risk, pure GUI):** promote RT60/volume
  (`awe_view.rs:~2316`) to a live line under Size + a verbal descriptor ("large, bright, long
  tail"); per-shape icons + consistent dimension labels (today "Length" means different things
  per shape).
- **Phase 2 — anchors & ambiguity (low-med):** units/anchors + better tooltips on raw knobs;
  resolve the Material+Diffusion ambiguity (show "Concrete (modified)" + reset).
- **Open decision — a "Liveness" (hard↔soft) macro** has no backing param today (absorption
  comes entirely from material). Either compose from existing params (Tail + Early/Late +
  Diffusion) or add an engine-side absorption-scale. **Recommendation: skip in v1**; Basics
  stays on real existing params (Size, Material, Dry/Wet) + live feedback.

## Strategic fork (which direction to take the module)

1. **Make the engine more honest (more physics)** — shape-aware early reflections (2nd-order +
   non-box walls), per-surface materials (§7), shape-dependent FDN. Largest effort; this is what
   makes the "3D for sound" vision actually true. Full geometric simulation is a research-grade
   cost and probably not worth it wholesale.
2. **Make the controls honest (relabel/regroup)** — keep the DSP, stop promising physics it
   lacks: separate "real room" (dimensions, material, temperature) from "creative effects"
   (portal, freq warp), and be explicit that shape mostly affects size/modes. = the progressive
   redesign above. Cheap, immediate pedagogical win.
3. **Targeted physics where the ear notices most** — make room shape audible (the single biggest
   bild-vs-ljud lie) and per-surface materials, leaving the FDN as the honestly statistical tail
   it is.

**Recommendation:** not (1) wholesale. The honest winner is **(2) as the baseline** (stop
misleading — cheap, lifts pedagogy now) **plus a targeted (3) effort on room shape** (the
largest visual-vs-audio gap). That makes both the UI and the most visible choice — the shape
picker — honest, without building a ray-tracer. If pursuing (3), the next scoping step is to
estimate the cost of shape-aware early reflections (same kind of feasibility pass done for the
tap data channel in §B).

---

## 4.1 Rework Room Visualization (Visual)

### Depth and shape clarity

- Add a subtle floor grid (1m and 5m steps) to give scale and reinforce dimensions.
- Emphasize cutaway edges with a heavier front outline and lighter/dashed back edges.
- Add a faint inner shadow on the cutaway edge to show thickness.
- Add vertical "pins" from source/listener markers up to mid-height for stronger 3D cues.

### Shading and material cues

- Introduce per-surface shading (floor, back wall, side wall) with stronger contrast.
- Add a material-dependent tint overlay to walls/floor (not just the markers).
- Use per-material textures/patterns (simple procedural line/dot patterns are enough).

### Animation improvements

- Reflection path animation should scale with path length (distance / speed of sound),
  not only audio level.
- Reflection path alpha should scale by per-path attenuation (distance + absorption).
- Sound rings should include light depth cues: 2-3 concentric ellipses at slightly
  different heights or thicknesses to hint at 3D propagation.

## 4.2 Differentiate Effects More Clearly (Visual)

### Material identity

- Map each material to a distinctive visual style (color + pattern + animation speed).
- Add a small "absorption bar" visual in the info box: L/M/H absorption + diffusion.

### Reflection visualization

- For non-Box shapes, show simplified reflection hints (e.g. cylinder arc reflection,
  dome/sphere mirror arcs) to make geometry-driven behavior visible.
- Color-code reflection paths by frequency band (low/mid/high) to hint absorption.

## 4.3 Differentiate Effects More Clearly (Audio)

### Early reflections

- Add second-order reflections or a short diffusion stage after early reflections
  to increase density and provide clearer material signatures.
- Apply stronger material-dependent damping per reflection path (distance + air absorption).

### Late reverb (FDN)

- Increase material contrast by widening the mapping of absorption to LP/HP coefficients.
- Introduce shape-dependent modulation (e.g. Tube = longer delay, lower diffusion;
  Dome/Sphere = higher diffusion).

### Room modes

- Tangential/oblique modes are already implemented; extend with higher-order mode sets
  and shape-dependent weighting for clearer room identity.
- Scale mode intensity by geometry (e.g. L-shape and Tube feel more resonant).

### Spatializer

- Add a gentle EQ tilt or multi-band head shadow filter to make spatial cues more obvious.
- Optionally add early/late spatial width differences (wider in late field).

## Acceptance Criteria and RT Constraints

To keep this document actionable and safe for the audio engine, evaluate each change against:

1. Real-time safety:

- No heap allocations or blocking locks on audio callback / engine audio thread.
- Bounded processing cost per sample/block (no unbounded loops from room complexity).

2. CPU budget:

- Target < 10% extra DSP cost vs current AWE baseline at 48 kHz / 256 samples
  with a representative preset.
- If a feature exceeds budget, provide a quality toggle or reduced-complexity mode.

3. Measurable audio differentiation:

- Material/shape changes should produce measurable deltas (e.g. RT60, spectral centroid,
  early reflection density) in offline analysis.
- Keep output level-compensated in A/B tests to avoid loudness bias.

4. Visual acceptance:

- Reflection visuals should encode both path timing and attenuation (not only input level).
- Geometry/material cues should remain readable at typical UI sizes without clutter.

## Suggested Quick Wins

1. Visual: floor grid + material tint + absorption bars.
2. Audio: increase material contrast in FDN and early reflections damping.
3. Visual: per-shape reflection hints for non-Box rooms.

---

## Long-term / Ambitious Ideas (Phase 3+)

### 13. Surface coupling / wall vibration

Thin walls (drywall, wood panels) don't just absorb — they vibrate and re-radiate sound.
Model as an extra LF feedback loop per surface, controlled by wall thickness, mass (kg/m²),
and stiffness. Thin walls add "chest tone" and body to the reverb; the room "breathes" at
low frequencies.

### 14. Wall openings & room coupling

Define openings (doors, windows) on specific walls. An opening reduces reflections from that
wall and adds HF loss from edge diffraction. Openings can connect to an exterior (absorption
sink) or to a second room shape (extended portal concept). Struct: wall ID, position on wall,
size in m², and target (Outside or coupled Room).

### 15. Weather effects (outdoor scenes)

Wind, rain, and fog for outdoor or semi-outdoor spaces:

- **Wind**: asymmetric delay modulation (downwind = shorter, upwind = longer), direction param
- **Rain**: stochastic noise modulation of reflection times + increased diffuse absorption
- **Fog**: extreme HF roll-off proportional to density

### 16. Acoustic focusing & caustics (curved surfaces)

Concave surfaces (dome, sphere) focus sound toward focal points — dramatic gain boost at the
center. Convex surfaces scatter. Per-reflection gain adjustment based on surface curvature at
the reflection point. Sphere/dome rooms would exhibit strong convergent focusing.
