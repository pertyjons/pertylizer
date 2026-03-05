# TODO - Pertylizer (v0.213.0)

## Priority 0 — OSC & Visualizer: Phase 3 (Polish & Extend)

> Full plan: [osc-telemetry-plan.md](osc-telemetry-plan.md)

### 0.1 OSC idle mode (`synth_osc`) — DONE
- [x] Skip FFT computation and UDP sends when no visualizer is connected
- [x] `/viz/ping`↔`/viz/pong` handshake: sender includes `/viz/ping` in meta bundles, visualizer replies with `/viz/pong` every 2s
- [x] Idle timeout (5s default): when no pong received, skip FFT + full telemetry, send meta-only beacon
- [x] Automatic resume when client connects, with console log for state changes

### 0.1b OSC status indicator in top bar — DONE
- [x] Show OSC status icon + text in the top bar (like MCP indicator)
- [x] Three states: **Off** (red), **Idle** (dim, sending beacon but no client), **Connected** (green, client responding with `/viz/pong`)
- [x] Hover tooltip with status description

### 0.2 Additional OSC telemetry streams — DONE
- [x] `/synth/audio/centroid` — spectral centroid (brightness) for effect selection
- [x] `/synth/audio/flux` — spectral flux for onset/section detection
- [x] `/synth/transport/phase` — beat phase 0..1 within current beat
- [x] `/synth/event/cc` — MIDI CC events (cc, value, channel) including pitch bend (128) and aftertouch (129)
- [x] `/synth/engine/event_drops` — event ring buffer drop count (only sent when > 0)
- [x] `/synth/meta/fft_freqs` — 128 center frequencies for log-spaced bar positioning (sent with meta)

### 0.3 Visualizer improvements (`visualizer/`) — DONE
- [x] Particle systems for note events (golden-angle burst on note-on, gravity + fade, 512 particle cap)
- [x] Camera auto-movement synced to tempo (orbit speed scales with BPM, 120 BPM = baseline)
- [x] "Waiting for signal" indicator when no OSC data received (fades in after ~2s stale)
- [x] Protocol version check — warn once on mismatch with `/synth/meta`
- [x] Visualizer handles all Phase 3 telemetry streams (centroid, flux, phase, cc, drops, fft_freqs)
- [x] Per-particle material mutations (up to 512/frame) — consider shared material approach or GPU instancing to reduce asset churn
- [ ] Configurable FFT bin count (64/128/256)
- [ ] Extract shared OSC address constants — visualizer hardcodes `"/viz/pong"` instead of using `synth_osc::addresses::VIZ_PONG` (separate workspace can't depend on `synth_osc`, consider a shared `synth_osc_protocol` crate or constants file)
- [ ] Replace `last_note_on: Option<(u8, u8, u8)>` with a named `NoteOnEvent` struct (fields: `note`, `velocity`, `channel`) — raw tuple used in telemetry + 3 visual consumers

### 0.4 Visualizer effect system — DONE
- [x] Effect rack with switchable visual layers (Left/Right arrows for prev/next, R for random)
- [x] 2 additional effect modes: Waveform Ring (circular FFT) and Spectral Waterfall (scrolling 3D spectrogram)
- [x] Fade-through-black crossfade between effects on switch
- [x] Spectral waterfall: replace 2048-entity grid with texture-based approach (single quad + custom shader) for fewer draw calls (Implemented via shared materials and Y-scale scaling instead)
- [ ] Replace per-effect `if active != MyEffect { return }` guards with Bevy run conditions — cleaner system signatures, skips dispatch entirely (matters when effect count grows)

### 0.5 Settings & control
- [ ] OSC enable/disable toggle in Pertylizer settings GUI
- [ ] `/viz/` OSC control endpoints (effect select, param set, scene load)
- [ ] Support connecting multiple OSC clients simultaneously (e.g., via `send_to` and active client tracking)

### 0.6 Shared dB conversion utility
- [ ] Extract `magnitude_to_normalized_db()` into `synth_core` or `synth_dsp` — inline `20.0 * x.log10()` + normalization repeated in 4+ locations

---

## Priority 1 — Foundation & Core Functionality

### 1.1 Undo/Redo
- [ ] Implement undo/redo for sequencer operations (note add/delete/move, pattern edits)
- [ ] Implement undo/redo for module operations (add, delete, move, parameter changes)
- [ ] Implement undo/redo for connection operations (add, remove)
- [ ] Keyboard shortcuts: Ctrl+Z / Ctrl+Shift+Z

### 1.2 Audio export
- [ ] Render arrangement to WAV file (offline, faster-than-realtime)
- [ ] Export dialog: file path, sample rate, bit depth, duration/range
- [ ] Progress bar during render

### 1.3 Song save/load
- [ ] Recent projects — remember last opened projects in settings, show in menu
- [ ] Dirty state tracking — warn on unsaved changes before loading or quitting

### 1.4 Copy/paste modules
- [ ] Copy a module with its current parameters
- [ ] Paste as a new instance with the same settings
- [ ] Consider: copy a selection of modules + their internal connections

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

### 5.5 Future Visualizer Effects
- [x] Spectral Cathedral (FFT bands form arches that breathe, driven by FFT, RMS)
- [x] Harmonic Ribbons (Ribbons track pitch and glide, driven by Note-on, pitch)
- [x] Chord Bloom (Chords trigger radial bursts, driven by Note clusters)
- [x] Pulse Terrain (Landscape breathes with bass, driven by Low FFT, RMS)
- [x] Spectral Origami (Folded planes open with harmonics, driven by FFT, centroid)
- [x] Ferrofluid Tendrils (Magnetic tendrils from bass, driven by Low FFT)
- [x] Neon Calligraphy (Notes draw glyph strokes, driven by Note on/off, pitch)
- [x] Fractal Pulse (Recursive shapes synced to beat, driven by Tempo, RMS)
- [x] CPU Overdrive Core (Glowing core that spins and fractures under load, driven by CPU Usage, Voice Count)
- [x] Flux Supernova (Star that explodes on sudden spectral changes, driven by Spectral Flux, RMS)
- [x] Phase Rings (Concentric rings expanding with the beat phase, driven by Beat Phase, Tempo)
- [x] Centroid Nebula (Particle cloud shifting color/shape based on brightness, driven by Spectral Centroid, RMS)
- [x] Velocity Meteors (Meteors falling with size based on impact, driven by Note-on, Velocity)
