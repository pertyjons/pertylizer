# TODO - Pertylizer (v0.211.0)

## Priority 0 — DSP Helper Abstractions

> Full plan: [dsp-helpers-plan.md](dsp-helpers-plan.md)

### 0.1 InputReader — zero-cost input port reader
- [ ] Add `InputReader<'a>` struct with `Index<usize>` to `synth_core/src/module_traits.rs`
- [ ] Add `InputPorts::reader()` method
- [ ] Tests for connected/unconnected paths
- [ ] Migrate modules: `amplifier.rs`, `filter.rs`, `oscillator.rs`, `lfo.rs`, `output.rs`, `envelope.rs`

### 0.2 Stereo Frame Helpers
- [ ] Add `StereoSample::read_frame()` and `write_frame()` to `synth_core/src/types/audio.rs`
- [ ] Add `StereoFrameIter` iterator type
- [ ] Tests for even/odd/empty slices
- [ ] Migrate all 18 effects to use frame helpers

### 0.3 DenormalGuard (FTZ/DAZ)
- [ ] Add `DenormalGuard` RAII type in new `synth_core/src/types/denormal.rs`
- [ ] Integrate in cpal audio callback (`cpal_backend.rs`)
- [ ] Remove 23 manual `flush_denormals()` calls from modules and DSP code

### 0.4 Composable DSP Primitives
- [ ] Add `StereoSvf` and `StereoBiquad` to `synth_dsp/src/filters.rs`
- [ ] Add `OnePoleSmooth` parameter smoother to `synth_core/src/types/audio.rs`
- [ ] Tests for all new types

---

## Priority 0.A — Sequencer & Music Creation (completed)

### 0.1 Arrangement view fixes
- [x] Pattern miniature preview — draw note positions as tiny rectangles in placements
- [x] Fix track header/track-line vertical alignment (header padding vs timeline row height)
- [x] Snap pattern placements to beat grid when dragging
- [x] Pattern hover tooltip — show name, length, note count, instrument
- [x] Timeline zoom — horizontal zoom in/out for arrangement view (Ctrl+scroll)
- [x] Double-click on empty track area — create new empty pattern at that position
- [x] Fix right-click "Add pattern" uses mouse position at menu open, not current mouse position
- [x] Highlight/focus the target track row on right-click so it's clear which track gets the pattern
- [x] Drag-to-move placements — reposition pattern placements by dragging
- [x] Playhead click-to-seek in ruler — clicking the ruler should seek playback position
- [x] Auto-scroll follow playhead — arrangement view scrolls horizontally to keep the playhead visible during playback; disable auto-follow while user is scrolling, re-enable when scroll interaction ends
- [x] Piano roll mini-transport controls — play/pause/stop pattern from piano roll toolbar

### 0.2 Recording (MIDI & keyboard)
- [x] Record button in transport bar — arm recording, capture notes on play
- [x] MIDI recording to pattern — incoming MIDI note on/off → pattern notes
- [x] Computer keyboard recording — capture keyboard piano input to pattern
- [x] Metronome/click track — audible click during recording and playback
- [x] Count-in (pre-roll) — 1 bar metronome before recording starts
- [x] Recording target = opened piano roll pattern (button disabled if none open)
- [x] Visual recording indicator — red blinking REC/ARM/COUNT-IN in transport
- [x] Recording quantization — snap recorded notes to grid on input
- [x] Overdub mode — layer new notes on top of existing pattern

### 0.3 Copy/paste/duplicate in piano roll
- [x] Ctrl+C — copy selected notes
- [x] Ctrl+V — paste at cursor position
- [x] Ctrl+X — cut selected notes
- [x] Ctrl+D — duplicate selected notes (offset right)
- [x] Duplicate pattern — expose in arrangement right-click menu (backend exists)

### 0.4 Keyboard shortcuts for piano roll
- [x] Ctrl+A — select all notes
- [x] Escape — clear selection (already works)
- [x] Delete/Backspace — delete selected notes (already works)
- [x] Arrow up/down — transpose selection by semitone
- [x] Shift+arrow up/down — transpose selection by octave
- [x] Space — toggle play/pause

### 0.5 Quantization UI
- [x] Quantize menu/button — apply quantization to selected notes
- [x] Quantize strength slider (0–100%) — blend between original and quantized timing
- [x] Grid resolution selector in piano roll toolbar (1/4, 1/8, 1/16, 1/32)
- [x] Swing/shuffle control — offset even subdivisions

### 0.6 Velocity editing
- [x] Velocity lane in piano roll — bar graph below notes showing per-note velocity
- [x] Drag velocity bars to edit
- [x] Scale velocities for selection (percentage or curve)
- [x] Adjustable default velocity for drawing/recording (currently fixed 0.8)

### 0.7 Piano roll improvements
- [x] Note preview on click — play the note sound when clicking/drawing
- [x] Note length presets in toolbar (1/4, 1/8, 1/16) for draw mode
- [x] Humanize — slight random timing/velocity offset for selected notes (backend exists)
- [x] Step entry mode — advance cursor by grid step after each note input

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
- [ ] Create `crates/synth_osc/` crate skeleton (config, address constants, sender)
- [ ] Add second event ring buffer to `SynthEngine` for OSC event stream
- [ ] Implement OSC sender thread (poll shared state + drain events → rosc → UDP)
- [ ] Expose master `VisualizationBuffer` (spectrum FFT data) for OSC access
- [ ] Wire into `pertylizer` app with `--osc` CLI flag
- [ ] Test with external OSC monitor tool

### Phase 2: Bevy visualizer (separate project `pertylizer-visualizer`)
- [ ] Scaffold Bevy 0.16 project with camera and ground plane
- [ ] Implement `SynthTelemetry` resource and non-blocking OSC receiver system
- [ ] Build FFT bar visualization (128 cubes driven by frequency bands)
- [ ] Add RMS-driven point light and note-flash emissive sphere
- [ ] Add bloom post-processing, orbital camera, beat-synced pulse

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
