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

## 4.2 Differentiate Effects More Clearly (Audio)

### Early reflections
- Add second-order reflections or a short diffusion stage after early reflections
  to increase density and provide clearer material signatures.
- Apply stronger material-dependent damping per reflection path (distance + air absorption).

### Late reverb (FDN)
- Increase material contrast by widening the mapping of absorption to LP/HP coefficients.
- Introduce shape-dependent modulation (e.g. Tube = longer delay, lower diffusion;
  Dome/Sphere = higher diffusion).

### Room modes
- Add tangential/oblique modes (more combs) for clearer room identity.
- Scale mode intensity by geometry (e.g. L-shape and Tube feel more resonant).

### Spatializer
- Add a gentle EQ tilt or multi-band head shadow filter to make spatial cues more obvious.
- Optionally add early/late spatial width differences (wider in late field).

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

