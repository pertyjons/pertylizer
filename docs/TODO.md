# TODO - Pertylizer

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

### 0.3 Miscellaneous
- [x] Fix padding on the label Voices in Topbar so it always shows 3 digits wide, as the CPU label does.
---

## ★ MCP: Enrich tool metadata for AI discovery

`list_module_types` returns module types but lacks the context an AI needs to build patches autonomously:

- [x] **Parameter ranges** — return min/max/default for every parameter (e.g. `cutoff: 20.0–20000.0 Hz, default 1000.0`)
- [x] **Enum/discrete values** — list named options for discrete params (e.g. `waveform: 0=Sine, 1=Saw, 2=Square, 3=Triangle`)
- [x] **Port types** — mark ports as audio/cv/gate and input/output so AI knows what to connect
- [x] **Signal flow hints** — describe typical patch topology (e.g. `osc → filter → amp → out`, `env → amp.cv_gain`)
- [x] **Parameter units** — annotate with Hz, dB, ms, 0.0–1.0 normalized, etc.

This enables AI to self-discover the full synth architecture without hardcoded knowledge.

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
- [x] Reorder effect chain — up/down arrows on effect modules, visual chain cables, auto-aligned vertical column *(v0.237.0)*
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
- [ ] Add "Discovery" tools for the AI to better understand available port types and valid parameter ranges (see ★ MCP section above)

---

## 7. AWE Improvements

Findings and concrete ideas: `docs/AWE-Improvement-Findings.md`.

### 7.0 AWE acoustic engine — prioritized plan

#### Phase 1 — Quick wins (high impact, low complexity)
- [x] **1. Pre-delay** — new `PreDelay(Milliseconds)` param (0–200ms), delay before first reflection
- [x] **2. Air absorption** — new `AirAbsorption(NormalizedValue)` param, distance-proportional LP filtering per reflection
- [x] **3. Stereo width** — new `Width(NormalizedValue)` param (currently hardcoded to 1.0 in FDN)
- [x] **4. Wet signal EQ** — new `HighCut(Hertz)` and `LowCut(Hertz)` params, biquad filters on wet output
- [x] **5. FDN internal modulation** — new `ModulationDepth(NormalizedValue)` and `ModulationRate(Hertz)` params, chorus inside FDN to break metallic character
- [x] **6. Temperature → speed of sound** — new `Temperature(Celsius)` param, formula `v = 331.3 + 0.606 * T`, affects all delay calculations

#### Phase 2 — Medium complexity
- [ ] **7. Per-surface materials** — `MaterialConfig { floor, walls, ceiling }` instead of single global `Material`, ISM uses correct material per reflection
- [ ] **8. Second-order reflections** — extend ISM from 6 to ~30 taps (configurable `ReflectionOrder(u8)` 1–3)
- [x] **9. Extended room modes** — add tangential modes `f = c/2 * sqrt((n/L)² + (m/W)²)` and axial overtones, ~12 total combs instead of 3
- [ ] **10. Resonant objects** — sympathetic resonance from objects in the room (strings, membranes, plates, Helmholtz cavities, loose panels, chimes), implemented as bandpass + feedback at object frequency
- [x] **11. Eyring RT60** — replace Sabine `RT60 = 0.161V/(Sα)` with Eyring `RT60 = -0.161V/(S*ln(1-α))` for better accuracy at high absorption
- [ ] **12. Doppler effect** — track radial velocity between source/listener, shift pitch via variable delay read speed: `ratio = v_sound / (v_sound + v_radial)`

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
