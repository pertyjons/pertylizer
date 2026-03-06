# TODO - Pertylizer (v0.226.0)

## 1. Synth — Foundation & Core

### 1.1 Settings & utilities
- [ ] Add Browse button in Settings dialog to change patches directory
- [ ] Extract `magnitude_to_normalized_db()` into `synth_core` or `synth_dsp` — repeated in 4+ locations

### 1.2 Template library
- [ ] Add patch template directory and `Save Patch as Template` action
- [ ] Add Patch Template browser to load patch templates
- [ ] Support optional `license` and `min_app_version` metadata in group templates

### 1.3 MIDI learn
- [ ] Map MIDI CC to any module parameter via right-click → "MIDI Learn"
- [ ] Visual indicator on mapped parameters
- [ ] Save/load MIDI mappings with patch or settings

### 1.4 Module presets
- [ ] Save/load parameter presets per module type (not the whole patch)
- [ ] Preset browser in module context menu or header
- [ ] Ship default presets for common module types

### 1.5 Polyphony settings
- [ ] Voice count configurable per instrument (GUI control)
- [ ] Voice stealing mode selection (oldest, quietest, none)
- [ ] Unison detune/spread controls

---

## 2. Synth — UI & Visual Polish

### 2.1 Effects section — visual separation from voice modules
- [ ] Add a visual divider between voice modules and effects in the grid
- [ ] Use a distinct background color/tint for the effects area
- [ ] Label the section clearly: "Master Effects" or "Effect Chain"

### 2.2 Module Groups — Phase 2–3
- [ ] Phase 2: Template variants (parameter presets with remap)
- [ ] Phase 3: Probes data pipeline (ringbuffers, audio-thread safe collection)
- [ ] Phase 3: Probe rendering (waveform/spectrum/meter) with PortType-based signal type
- [ ] Phase 3: Polyphony probes = sum of voices (mixdown)

### 2.3 Improve module knobs
- [ ] Better visual design — gradient fill, shadow, tick marks, value tooltip
- [ ] Consistent sizing across module types
- [ ] Arc-style knobs with colored fill showing current value

### 2.4 Improve module ports
- [ ] Clearer port type distinction (audio vs control vs gate vs MIDI)
- [ ] Better hover feedback
- [ ] Colored rings matching cable colors, port labels on hover

### 2.5 Redesign instrument list
- [ ] Tabbed interface, mixer-style vertical strips, or collapsible panels

---

## 3. AWE Improvements

Findings and concrete ideas: `docs/AWE-Improvement-Findings.md`.

### 3.1 Rework room visualization
- [ ] Redesign the 3D isometric room rendering
- [ ] Improve animations (sound rings, reflection paths)
- [ ] Better visual clarity for room shape and dimensions

### 3.2 Differentiate effects more clearly
- [ ] Each material/effect should have more distinct visual representation
- [ ] Color-coded zones, animated textures per material, spectral visualization

---

## 4. Visualizer & OSC

### 4.1 OSC control & connectivity
- [ ] OSC enable/disable toggle in Pertylizer settings GUI
- [ ] `/viz/` OSC control endpoints (effect select, param set, scene load)
- [ ] OSC `/viz/theme/select` control endpoint
- [ ] OSC parameter tweaking — live control of intensity, speed, scale per effect
- [ ] Support connecting multiple OSC clients simultaneously

### 4.2 Post-processing & shaders
- [ ] Chromatic aberration — intensity scales with RMS level
- [ ] Glitch/distortion effect — triggered by CPU spikes or spectral flux
- [ ] Kaleidoscope mode — radial scene mirroring (configurable segment count)
- [ ] CRT/VHS filter — scanlines, color bleed, static noise
- [ ] Motion blur — strength synced to tempo

### 4.3 Multi-effect layering
- [ ] Show 2–3 effects simultaneously instead of one at a time
- [ ] Per-instrument visual layers — each instrument gets its own color/effect layer
- [ ] Blending modes between layers (additive, multiply, screen)
- [ ] Layer opacity control via OSC

### 4.4 Reactive environment
- [ ] Skybox that reacts to music — stars pulse with RMS, clouds move with tempo
- [ ] Reactive ground — ripples on note-on, cracks on bass hits
- [ ] Fog/mist density driven by reverb level or sustain
- [ ] Day/night cycle driven by song position
- [ ] Weather effects — rain on high spectral flux, lightning on transients

### 4.5 Advanced simulations
- [ ] Swarm/flock simulation — particles flock or scatter based on dynamics
- [ ] Cloth simulation — fabric that billows and ripples with FFT energy
- [ ] Text/typography — display song title, BPM, key in stylized 3D text
- [ ] AWE spatialization — visualize sound source position in 3D space

### 4.6 Video export
- [ ] Video recording — render to MP4 or image sequence

---

## 5. AI & Automation

### 5.1 MCP & AI Interaction
- [ ] Enable AI to "play freely" via MCP to autonomously generate complete songs and arrangements
- [ ] Implement real-time parameter interpolation (gliding) to allow smoother AI-driven sound design
- [ ] Support batching of MCP commands to reduce latency and overhead during complex generations
- [ ] Add "Discovery" tools for the AI to better understand available port types and valid parameter ranges
