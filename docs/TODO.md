# TODO - Pertylizer (v0.236.0)

## 0. Known Bugs

### 0.1 Project save/load
- [ ] Verify that Rack view saves each instrument's module positions and canvas size correctly when saving a project
- [ ] You should be able to load and save AWE, AWE preset should be saved in the project as well or maybe with an instrument

### 0.2 Project settings — unsaved instrument strip parameters
The following `InstrumentParam` variants exist in the engine but have no UI controls and are not persisted in project/patch files:
- [ ] `AllocationMode` — voice allocation mode (Polyphonic/Mono/Legato), currently hardcoded to Polyphonic
- [ ] `MaxVoices` — maximum polyphony per instrument, currently hardcoded in allocator
- [ ] `VelocityAmpSensitivity` — velocity → amplitude mapping sensitivity
- [ ] `VelocityFilterSensitivity` — velocity → filter cutoff mapping sensitivity
---

## 1. Core Usability & Workflow

### 1.1 Instrument management
- [ ] Rename instrument from instrument strip menu or inline edit
- [ ] Remove instruments via context menu or toolbar
- [ ] Translate all swedish descriptions in the modules here: crates/synth_modules

### 1.2 MIDI learn
- [ ] Map MIDI CC to any module parameter via right-click → "MIDI Learn"
- [ ] Visual indicator on mapped parameters
- [ ] Save/load MIDI mappings with patch or settings

### 1.3 Module presets
- [ ] Save/load parameter presets per module type (not the whole patch)
- [ ] Preset browser in module context menu or header
- [ ] Ship default presets for common module types

### 1.4 Mixer view
- [ ] Dedicated mixer view with faders, pan, sends, and inserts
- [ ] Send/return effect busses — shared effects instead of per-instrument chains only

### 1.5 Settings & utilities
- [ ] Add Browse button in Settings dialog to change patches directory
- [ ] Extract `magnitude_to_normalized_db()` into `synth_core` or `synth_dsp` — repeated in 4+ locations

### 1.6 Workflow quality of life
- [ ] Reorder effect chain — add ability to change processing order (e.g., left/right arrows on effect modules)
- [ ] A/B comparison — quick-switch between two patch versions to compare sound
- [ ] Parameter locking — lock parameters to prevent accidental changes
- [ ] Favorite modules — quick access to frequently used modules in "Add Module"

---

## 2. Sequencer & Arrangement

### 2.1 Tempo automation
- [ ] Tempo curve over time (accelerando/ritardando)

### 2.2 Section markers
- [ ] Verse, chorus, bridge labels in the arrangement

### 2.3 Macro controllers
- [ ] Map multiple parameters to a single macro knob for live performance

### 2.4 MIDI export
- [ ] Export sequences as .mid files

---

## 3. Sound Design — Expanded Capabilities

### 3.1 Sample & wavetable import
- [ ] Sample import — load .wav files as oscillator source or in granular synth
- [ ] Wavetable import — load custom wavetables (Serum format, single-cycle .wav)

### 3.2 Alternative tunings
- [ ] Scala file (.scl) support, just intonation, microtonality

### 3.3 Expression & articulation
- [ ] MPE support — MIDI Polyphonic Expression for per-note pitch bend, pressure, slide
- [ ] Polyphonic aftertouch routing to module parameters

### 3.4 Sidechain routing
- [ ] Use one instrument's audio to control another (e.g. sidechain compression)

### 3.5 Polyphony settings
- [ ] Voice count configurable per instrument (GUI control)
- [ ] Voice stealing mode selection (oldest, quietest, none)
- [ ] Unison detune/spread controls

---

## 4. UI & Visual Polish

### 4.1 Improve module knobs
- [ ] Better visual design — gradient fill, shadow, tick marks, value tooltip
- [ ] Consistent sizing across module types
- [ ] Arc-style knobs with colored fill showing current value

### 4.2 Redesign instrument list
- [ ] Tabbed interface, mixer-style vertical strips, or collapsible panels

### 4.3 Module Groups — Phase 2–3
- [ ] Phase 2: Template variants (parameter presets with remap)
- [ ] Phase 3: Probes data pipeline (ringbuffers, audio-thread safe collection)
- [ ] Phase 3: Probe rendering (waveform/spectrum/meter) with PortType-based signal type
- [ ] Phase 3: Polyphony probes = sum of voices (mixdown)

---

## 5. Template Library & Presets

### 5.1 Template library
- [ ] Add patch template directory and `Save Patch as Template` action
- [ ] Add Patch Template browser to load patch templates
- [ ] Support optional `license` and `min_app_version` metadata in group templates

### 5.2 Preset sharing
- [ ] Community format for sharing patches online

---

## 6. AI & Automation

### 6.1 MCP & AI Interaction
- [ ] Enable AI to "play freely" via MCP to autonomously generate complete songs and arrangements
- [ ] Implement real-time parameter interpolation (gliding) to allow smoother AI-driven sound design
- [ ] Support batching of MCP commands to reduce latency and overhead during complex generations
- [ ] Add "Discovery" tools for the AI to better understand available port types and valid parameter ranges

---

## 7. AWE Improvements

Findings and concrete ideas: `docs/AWE-Improvement-Findings.md`.

### 7.1 Rework room visualization
- [ ] Redesign the 3D isometric room rendering
- [ ] Improve animations (sound rings, reflection paths)
- [ ] Better visual clarity for room shape and dimensions

### 7.2 Differentiate effects more clearly
- [ ] Each material/effect should have more distinct visual representation
- [ ] Color-coded zones, animated textures per material, spectral visualization

---

## 8. Visualizer & OSC

### 8.1 OSC control & connectivity
- [ ] OSC enable/disable toggle in Pertylizer settings GUI
- [ ] `/viz/` OSC control endpoints (effect select, param set, scene load)
- [ ] OSC `/viz/theme/select` control endpoint
- [ ] OSC parameter tweaking — live control of intensity, speed, scale per effect
- [ ] Support connecting multiple OSC clients simultaneously

### 8.2 Post-processing & shaders
- [ ] Chromatic aberration — intensity scales with RMS level
- [ ] Glitch/distortion effect — triggered by CPU spikes or spectral flux
- [ ] Kaleidoscope mode — radial scene mirroring (configurable segment count)
- [ ] CRT/VHS filter — scanlines, color bleed, static noise
- [ ] Motion blur — strength synced to tempo

### 8.3 Multi-effect layering
- [ ] Show 2–3 effects simultaneously instead of one at a time
- [ ] Per-instrument visual layers — each instrument gets its own color/effect layer
- [ ] Blending modes between layers (additive, multiply, screen)
- [ ] Layer opacity control via OSC

### 8.4 Reactive environment
- [ ] Skybox that reacts to music — stars pulse with RMS, clouds move with tempo
- [ ] Reactive ground — ripples on note-on, cracks on bass hits
- [ ] Fog/mist density driven by reverb level or sustain
- [ ] Day/night cycle driven by song position
- [ ] Weather effects — rain on high spectral flux, lightning on transients

### 8.5 Advanced simulations
- [ ] Swarm/flock simulation — particles flock or scatter based on dynamics
- [ ] Cloth simulation — fabric that billows and ripples with FFT energy
- [ ] Text/typography — display song title, BPM, key in stylized 3D text
- [ ] AWE spatialization — visualize sound source position in 3D space

### 8.6 Video export
- [ ] Video recording — render to MP4 or image sequence

---

## 9. Advanced / Long-term

### 9.1 Audio tracks
- [ ] Import and arrange audio files, not just synth tracks

### 9.2 Audio recording
- [ ] Record external audio via cpal input

### 9.3 Clip launching
- [ ] Ableton-style live mode with follow actions

### 9.4 Plugin export
- [ ] Export instruments as VST3/CLAP plugins
