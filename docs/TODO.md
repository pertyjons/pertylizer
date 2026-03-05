# TODO - Pertylizer (v0.212.0)

## Priority 0.A.8 — Newtype arithmetic refinements (completed)
- [x] `view_pitch_min`/`view_pitch_max` should be `Pitch` in `handle_piano_roll_interaction` (eliminates ~6 `Pitch::new().unwrap_or()` calls)
- [x] Add `impl Sub<PatternTick> for PatternTick` → `SeqDuration` (eliminates ~10 manual `.0.saturating_sub(.0)` patterns)
- [x] Add `SeqDuration::as_pattern_tick()` helper for `PatternTick(data.length_ticks.0)` conversion

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

## Priority 1.A — OSC Telemetry & Bevy Visualizer

> Full plan: [osc-telemetry-plan.md](osc-telemetry-plan.md)

### Phase 1: Synth OSC sender (`synth_osc` crate)
- [x] Create `crates/synth_osc/` crate skeleton (config, address constants, sender)
- [x] Add second event ring buffer to `SynthEngine` for OSC event stream (note on/off)
- [x] Implement OSC sender thread (poll shared state → rosc → UDP at ~30 Hz)
- [x] Expose master `VisualizationBuffer` (spectrum FFT data) for OSC access
- [x] Wire into `pertylizer` app with `--osc` CLI flag
- [x] Test with external OSC monitor tool

### Phase 2: Bevy visualizer (separate project `pertylizer-visualizer`)
- [ ] Scaffold Bevy 0.16 project with camera and ground plane
- [ ] Implement `SynthTelemetry` resource and non-blocking OSC receiver system
- [ ] Build FFT bar visualization (128 cubes driven by frequency bands)
- [ ] Add RMS-driven point light and note-flash emissive sphere
- [ ] Add bloom post-processing, orbital camera, beat-synced pulse

### 1.7 Shared dB conversion utility
- [ ] Extract `magnitude_to_normalized_db()` into `synth_core` or `synth_dsp` — inline `20.0 * x.log10()` + normalization is repeated in 4+ locations (spectrum_analyzer, OSC sender, meter widget, visual_state)

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
