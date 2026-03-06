# TODO - Pertylizer (v0.224.0)

## Priority 0 — OSC & Visualizer

### 0.6 Settings & control
- [ ] OSC enable/disable toggle in Pertylizer settings GUI
- [ ] `/viz/` OSC control endpoints (effect select, param set, scene load)
- [ ] Support connecting multiple OSC clients simultaneously (e.g., via `send_to` and active client tracking)

### 0.6 Shared dB conversion utility
- [ ] Extract `magnitude_to_normalized_db()` into `synth_core` or `synth_dsp` — inline `20.0 * x.log10()` + normalization repeated in 4+ locations

---

## Priority 1 — Foundation & Core Functionality

### 1.5 Settings expansion
- [ ] Add Browse button in Settings dialog to change patches directory

### 1.6 Template library
- [ ] Add patch template directory and `Save Patch as Template` action
- [ ] Add Patch Template browser to load patch templates
- [ ] Support optional `license` and `min_app_version` metadata in group templates

---

## Priority 2 — UI Structure & Layout

### 2.1 Effects section — visual separation from voice modules
- [ ] Add a visual divider between voice modules and effects in the grid
- [ ] Use a distinct background color/tint for the effects area
- [ ] Label the section clearly: "Master Effects" or "Effect Chain"

### 2.2 Module Groups — Phase 2–3
- [ ] Phase 2: Template variants (parameter presets with remap)
- [ ] Phase 3: Probes data pipeline (ringbuffers, audio-thread safe collection)
- [ ] Phase 3: Probe rendering (waveform/spectrum/meter) with PortType-based signal type
- [ ] Phase 3: Polyphony probes = sum of voices (mixdown)

---

## Priority 3 — Visual Polish

### 3.1 Improve module knobs
- [ ] Better visual design — gradient fill, shadow, tick marks, value tooltip
- [ ] Consistent sizing across module types
- [ ] Arc-style knobs with colored fill showing current value

### 3.2 Improve module ports
- [ ] Clearer port type distinction (audio vs control vs gate vs MIDI)
- [ ] Better hover feedback
- [ ] Colored rings matching cable colors, port labels on hover

---

## Priority 4 — AWE Improvements

Findings and concrete ideas: `docs/AWE-Improvement-Findings.md`.

### 4.1 Rework room visualization
- [ ] Redesign the 3D isometric room rendering
- [ ] Improve animations (sound rings, reflection paths)
- [ ] Better visual clarity for room shape and dimensions

### 4.2 Differentiate effects more clearly
- [ ] Each material/effect should have more distinct visual representation
- [ ] Color-coded zones, animated textures per material, spectral visualization

---

## Priority 5 — Future / Later

### 5.1 Redesign instrument list
- [ ] Tabbed interface, mixer-style vertical strips, or collapsible panels

### 5.2 MIDI learn
- [ ] Map MIDI CC to any module parameter via right-click → "MIDI Learn"
- [ ] Visual indicator on mapped parameters
- [ ] Save/load MIDI mappings with patch or settings

### 5.3 Module presets
- [ ] Save/load parameter presets per module type (not the whole patch)
- [ ] Preset browser in module context menu or header
- [ ] Ship default presets for common module types

### 5.4 Polyphony settings
- [ ] Voice count configurable per instrument (GUI control)
- [ ] Voice stealing mode selection (oldest, quietest, none)
- [ ] Unison detune/spread controls

### 5.5 Visualizer themes
- [ ] OSC `/viz/theme/select` control endpoint

### 5.6 Post-processing & shader effects
- [ ] Chromatic aberration — intensity scales with RMS level
- [ ] Glitch/distortion effect — triggered by CPU spikes or spectral flux
- [ ] Kaleidoscope mode — radial scene mirroring (configurable segment count)
- [ ] CRT/VHS filter — scanlines, color bleed, static noise
- [ ] Motion blur — strength synced to tempo

### 5.7 Camera modes
- [ ] Audio-reactive camera — shake on transients, dolly-zoom on bass drops
- [ ] Multiple camera presets (first-person fly-through, top-down, fixed angles, free orbit)
- [ ] Beat-synced camera cuts — VJ-style automatic camera switching on downbeats
- [ ] Camera mode switching via keyboard shortcut (C) or OSC `/viz/camera/select`

### 5.8 Multi-effect layering
- [ ] Show 2–3 effects simultaneously instead of one at a time
- [ ] Per-instrument visual layers — each instrument gets its own color/effect layer
- [ ] Blending modes between layers (additive, multiply, screen)
- [ ] Layer opacity control via OSC `/viz/layer/opacity`

### 5.9 Reactive environment
- [ ] Skybox that reacts to music — stars pulse with RMS, clouds move with tempo
- [ ] Reactive ground — ripples on note-on, cracks on bass hits
- [ ] Fog/mist density driven by reverb level or sustain
- [ ] Day/night cycle driven by song position
- [ ] Weather effects — rain on high spectral flux, lightning on transients

### 5.10 Generative geometry
- [ ] L-system trees — branches grow with incoming notes, wither during silence
- [ ] Voronoi patterns — cells split/shatter with spectral flux
- [ ] Reaction-diffusion patterns — driven by spectral centroid (warm ↔ cool)
- [ ] Fractal terrain — landscape deforms in real-time with FFT bands

### 5.11 Interaction & export
- [ ] Video recording — render to MP4 or image sequence
- [ ] Screenshot button (P key)
- [ ] OSC parameter tweaking — live control of intensity, speed, scale per effect
- [ ] Fullscreen toggle (F key)
- [ ] Debug HUD overlay — FPS, telemetry values, active effect, draw calls

### 5.12 Advanced simulations
- [ ] Swarm/flock simulation — particles flock or scatter based on dynamics (loud = scatter, quiet = flock)
- [ ] Cloth simulation — fabric that billows and ripples with FFT energy
- [ ] Text/typography — display song title, BPM, key in stylized 3D text
- [ ] AWE spatialization — if room data is available via OSC, visualize sound source position in 3D space
