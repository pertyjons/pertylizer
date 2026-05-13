# TODO - Pertylizer

## 0. Known Bugs

### 0.2 Project settings — unsaved instrument strip parameters

The following `InstrumentParam` variants exist in the engine but have no UI controls and are not persisted in
project/patch files:

- [x] `AllocationMode` — voice allocation mode (Polyphonic/Mono/Legato/Unison). Persisted on `InstrumentState`,
  applied at instrument construction; editable in the instrument edit window.
- [x] `MaxVoices` — maximum polyphony per instrument. Persisted on `InstrumentState` as `VoiceCount`,
  applied via `Session::add_instrument_with_id_and_config` on project load (engine can't resize the voice
  pool at runtime, so changes take effect on the next reload). Editable in the instrument edit window.
- [x] `VelocityAmpSensitivity` — velocity → amplitude mapping sensitivity. Persisted; live-editable.
- [x] `VelocityFilterSensitivity` — velocity → filter cutoff mapping sensitivity. Persisted; live-editable.

---

## ★ Description fields on user-facing entities (for AI context)

Add an optional free-text `description: String` field on user-facing entities so the user (or AI) can record
*intent* — what a thing is for, not just what it is. The structural data (graph, notes, params) already tells you
*what*; descriptions capture the *why*, which is otherwise unrecoverable.

**Every instance-level description must be readable AND writable via MCP**, so AI can both read existing intent
and populate descriptions after analysis. Read access is typically free (include the field in the existing
resource/getter response — `get_instrument_info`, `get_song_info`, `list_patterns`, `list_tracks`,
`list_samples`, `get_awe_state`); write access needs a dedicated setter tool routed through
`bridge.rs` → `mcp_bridge.rs` → `server.rs`.

**Every instance-level description must also be editable from the GUI** — never MCP-only. The user must be
able to write the same intent they'd ask AI to write. Each entity gets an inline-editable text field (header
band, properties dialog, or context-menu "Edit description…" action) wired through the same session/bridge
path so MCP and GUI writes share validation and undo. Type-level descriptors (`ModuleDescriptor`,
`ParameterDescriptor`, `PortDescriptor`) are already read-exposed via `get_module_type_info` /
`list_module_types` / `search_modules` and are hardcoded in source — no GUI edit and no MCP write tool there.

### Current status of description fields

| Entity                     | Field exists?                         | MCP read | MCP write |
|----------------------------|---------------------------------------|----------|-----------|
| `InstrumentState`          | ✅ `patch.rs:697`                     | ❌       | ❌        |
| `Patch`                    | ✅ `patch.rs:130, 307` (Option)       | ❌       | ❌        |
| `AwePresetFile`            | ✅ `patch.rs:792`                     | ❌       | ❌        |
| `Song`                     | ❌ — add to `synth_sequencer/song.rs` | ❌       | ❌        |
| `Pattern`                  | ❌ — add to `synth_sequencer`         | ❌       | ❌        |
| `SequencerTrack`           | ❌ — add to `synth_sequencer`         | ❌       | ❌        |
| `Sample` entry             | ❌ — add to sample registry           | ❌       | ❌        |
| Module *instance* (in patch) | ❌ — separate from `ModuleDescriptor` | ❌       | ❌        |
| `ModuleDescriptor` (type)  | ✅ `module_traits.rs:869`             | ✅       | n/a (hardcoded) |
| `ParameterDescriptor` (type) | ✅ `module_traits.rs:586`            | ✅       | n/a       |
| `PortDescriptor` (type)    | ✅                                    | ✅       | n/a       |
| `ChoiceOption` (type)      | ✅ `module_traits.rs:557` (Option)    | partial  | n/a       |

### Phase 1 — entities whose fields already exist; only MCP plumbing needed

- [x] Surface `InstrumentState.description` in `get_instrument_info` / `list_instruments` (MCP read) —
  done by mirroring description through engine: `Instrument.description` + getter/setter,
  `InstrumentSnapshot.description`, populated in `snapshot_to_info`.
- [x] `set_instrument_description` MCP tool — `EngineCommand::SetInstrumentDescription` +
  `Session::set_instrument_description` + `SynthBridge::set_instrument_description` +
  `server.rs` tool. Accepts `""` to clear. Already editable in the instrument edit window;
  the GUI now dispatches the engine command on every changed frame.
- [x] Surface `Patch.description` per instrument in `InstrumentInfo.patch_description` (MCP read) —
  the engine's `Instrument` gained a runtime mirror; project load copies the saved
  `Patch.description` into it, project save writes it back from the snapshot.
- [x] `set_patch_description` MCP tool (MCP write) — `EngineCommand::SetPatchDescription` +
  `Session::set_patch_description` + bridge method + `server.rs` tool. Accepts `""` to clear
  (treated as `None`). Distinct from `set_instrument_description` — see the tool description.
- [x] Surface `AwePresetFile.description` in `get_awe_state` (MCP read) — `AweStateInfo.description`
  populated from a new `McpSharedState.awe_description: Mutex<String>` slot (lives outside
  `AweState` to avoid touching the 36+ preset literals). `list_awe_presets` already exposed
  preset descriptions via `AwePresetInfo`.
- [x] `set_awe_description` MCP tool (MCP write) — bridge method updates
  `McpSharedState.awe_description` directly (no engine command since description never
  affects audio). Accepts `""` to clear.
- [x] Editable from GUI for the remaining two fields:
  - **Patch description** — `InstrumentUiState.patch_description: String` mirror, multiline
    `TextEdit` in the instrument-edit window directly below the existing Description field.
    Hover tooltips on both labels distinguish per-instance song-role intent vs sound-design
    intent. Dispatches `EngineCommand::SetPatchDescription` on change; loads from
    `inst_state.patch.description`.
  - **AWE description** — `AweUiState.description: String` mirror with an
    `description_edit_in_progress` flag, multiline `TextEdit` at the top of the AWE controls
    panel. Two-way sync with `McpSharedState.awe_description`: GUI → shared while the user is
    typing (edit-in-progress guard), shared → GUI otherwise so MCP writes propagate back.
    Persists separately as `GlobalProjectState.awe_description: Option<String>` so the text
    survives project save / load round-trips.

### Phase 2 — add new description fields + MCP read/write tools

- [ ] Add `description: String` to `Song` (`synth_sequencer/src/song.rs:79`); surface in `get_song_info`;
  add `set_song_description` MCP tool
- [ ] Add `description: String` to `Pattern`; surface in `list_patterns` / pattern resource;
  add `set_pattern_description` MCP tool (e.g. `"chorus drop, half-time feel"`)
- [ ] Add `description: String` to `SequencerTrack`; surface in `list_tracks`;
  add `set_track_description` MCP tool
- [ ] Add `description: String` to sample registry entries; surface in `list_samples` / `get_sample_info`;
  add `set_sample_description` MCP tool
- [ ] Editable from GUI (song properties, pattern properties dialog, track header context menu, sample library)

### Phase 3 — per-module-instance notes (different concept from type docs)

- [ ] Add per-instance `description: String` on placed modules in a patch (e.g. annotate "this LFO is the
  wobble modulator" on a specific `lfo-1` instance). Distinct from `ModuleDescriptor.description` which
  documents the module *type* and is shared across all instances.
- [ ] Surface in `get_module_info` (MCP read); add `set_module_description` MCP tool (MCP write)
- [ ] Editable from GUI (module header context menu)

### Cross-cutting

- [ ] Include all instance-level descriptions in `get_graph_diagnostics` / `analyze_note` output so AI sees
  intent alongside structure
- [ ] Surface descriptions as tooltips on the corresponding GUI elements
- [ ] Decide on max length (suggest 500 chars soft, 2000 hard) — long enough for a paragraph, short enough to
  stay readable in tooltips
- [ ] Persistence format: inline in the existing JSON containers (no sidecar files)

---

## 1. Core Usability & Workflow

### 1.1 Instrument management

- [x] Rename instrument from instrument strip menu or inline edit — each row in the instrument-strip
  dropdown has a "⋯" submenu with "Rename / edit…" that opens the instrument-edit window with the
  name field, plus all the other instrument properties.
- [x] Remove instruments via context menu or toolbar — same "⋯" submenu has a red "Delete…" action.
  Opens a confirmation modal (instrument-level undo isn't wired yet, so a confirm step prevents
  accidental unrecoverable deletes).
- [x] Translate all swedish descriptions in the modules here: crates/synth_modules

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
- [ ] Add ergonomic constructor `SamplerParam::sample_select(u64) -> Self` (or `Param::sample_select`) in
  `synth_core/src/params/sampler.rs` — 4 call sites currently spell out
  `Param::Sampler(SamplerParam::SampleSelect(SampleId(id)))` verbatim (`session.rs`, `mcp_bridge.rs`,
  `gui/patch_editor.rs`, `gui/egui_backend.rs`)
- [ ] Add `param_sample_id(name, id)` to `ModuleStateBuilder` in `pertylizer/src/patch.rs` for symmetry with
  `param_f` / `param_i` / `param_b` / `param_choice` (no current callers — for API completeness)
- [x] Add `SampleId(u64)` variant to `PatchParamValue` in `synth_mcp/src/types.rs` — fixed by emitting
  `PatchParamValue::SampleId(*sample_id)` in `mcp_bridge.rs` (was `Int(sample_id as i32)` which silently
  truncated sample ids ≥ 2³¹).
- [ ] **Bundle piano-roll coordinate plumbing into a `PianoRollCoords` struct.**
  `handle_piano_roll_interaction` (`gui/sequencer/mod.rs`) currently takes 17 parameters and
  `draw_arrangement` takes 9. Four of those (`x_to_tick`, `y_to_pitch`, `tick_to_x`, `pitch_to_y`) plus
  `view_pitch_min`/`view_pitch_max`/`note_row_height` are one coherent concept — a piano-roll coordinate
  transform. Extracting a struct collapses 7 args → 1 and removes the need to thread `note_row_height`
  through `note_at_pos` separately.
- [ ] **Deduplicate "Set Length" write+undo in the arrangement context menu.**
  `gui/sequencer/mod.rs` "Set Length…" submenu has the same ~22-line "read old length → write new → push
  `SetPatternLength` undo" block in two places: the free-input `DragValue` + Apply branch and the preset
  buttons loop. Extract `fn apply_pattern_length(song, undo_manager, pat_id, new_len)`.
- [ ] **Unify `SeqInstrumentId` ↔ `InstrumentId` raw conversions.**
  ~9 sites in `gui/sequencer/mod.rs` do `inst.id.0 == seq_id.0 as u64` and a few do the reverse
  `SeqInstrumentId::new(inst.id.0 as u16)` (lossy `u64 → u16` cast is silent). Pick one of: add
  `impl From<SeqInstrumentId> for InstrumentId` + `TryFrom<InstrumentId> for SeqInstrumentId`, or add a
  single `find_instrument_by_seq_id(&[InstrumentUiState], SeqInstrumentId) -> Option<&InstrumentUiState>`
  helper. (A `build_instrument_colour_cache` helper already exists for the hot-path lookups.)
- [ ] **Convert `Song::insert_pattern` / `Song::insert_track` double-lookup to `Entry` API.**
  Both currently do `if self.patterns.contains_key(...) { return false; } self.patterns.insert(...)` —
  two map lookups per call. Switch to `self.patterns.entry(id).or_insert_with(...)` (or `Entry::Vacant`
  for the explicit "already exists" branch). Low priority — only runs on undo/restore.

### 1.6 Workflow quality of life

- [ ] A/B comparison — quick-switch between two patch versions to compare sound
- [ ] Parameter locking — lock parameters to prevent accidental changes
- [ ] Favorite modules — quick access to frequently used modules in "Add Module"

---

## 2. Sequencer & Arrangement

### 2.1 Tempo automation

- [x] Tempo curve over time (accelerando/ritardando) — right-click the arrangement ruler →
  "Set tempo here…" opens a DragValue (20–300 BPM) + Apply. Existing changes can be removed via the
  same menu. The engine already polls `song.tempo_at(current_tick)` per tick, so changes take effect
  during playback without an extra command. Markers render as small orange flags on the ruler with
  the BPM number.
- [ ] Tempo curve interpolation (currently step changes only — accelerando ramps would smooth between
  two adjacent points)
- [ ] Undo for tempo set / remove (not yet wired into `UndoManager`)

### 2.2 Section markers

- [ ] Verse, chorus, bridge labels in the arrangement

### 2.3 Macro controllers

- [ ] Map multiple parameters to a single macro knob for live performance

### 2.4 MIDI export

- [ ] Export sequences as .mid files

### 2.5 Track reorder via drag-handle

- [ ] Replace (or complement) the current ↑/↓ arrow buttons in the track header with a drag-handle
  (e.g. `ri::DRAG_MOVE_2_LINE`) on the left edge of each row. Drag should snap the row vertically to the
  nearest neighbour while dragging, then commit on release via `Song::reorder_track`. The arrow buttons
  shipped because they are simpler and robust, but drag-handle reorder is the DAW convention.

### 2.6 Pattern looping within placement length (future)

- [ ] **Switch placement-resize from clip to loop-within semantics.** Today
  `PatternPlacement.length_override` (added in v0.281 with placement-resize) uses *clip* semantics: when the
  placement is longer than `pattern.length`, the pattern plays once and the remainder is silent. Most DAWs
  (Ableton, FL Studio, Renoise, Bitwig) loop the pattern internally instead, so a 1-bar drum pattern
  stretched to 4 bars plays four times. Implementing it touches three places in
  `crates/synth_engine/src/sequencer_engine.rs`:
  1. **Modulo on `pattern_tick`** — `collect_events_at_tick` currently computes
     `pattern_tick = (current_tick - placement.start) as u32`. With looping it becomes
     `pattern_tick = raw % pattern.length.0`. Trivial.
  2. **NoteOff timing across loop boundaries.** A note starting at `pattern_tick=800` with duration
     `200` in a 960-tick pattern would NoteOff at 1000 — past the loop. The active-notes buffer must
     hold the *absolute* end-tick (not modulo), and the next loop iteration's identical NoteOn must
     either retrigger or be coalesced with the still-ringing note. Pick a policy and document it.
  3. **Automation re-trigger.** Automation points need a re-trigger or "carry-over last value"
     decision per loop iteration. Today there is only one playback of each automation lane per
     placement — see `pattern.automation` collection at line ~360.
- [ ] **Mini-note visualization should mirror the loop.** `NoteMiniature.start_frac` is currently
  fraction-of-pattern-length. For loop-within semantics the rendering in
  `gui/sequencer/mod.rs` (mini-note loop, near the `inst_color_cache` use) should repeat the miniature
  across the placement's `effective_length / pattern.length` iterations, so the user sees what they hear.
- [ ] **Add a toggle on `PatternPlacement`** (`loop_mode: PlacementLoopMode { Clip, Repeat }`, default
  `Repeat` to match DAW expectations). Surface in the placement context menu and in the right-edge
  resize-grab tooltip so the user can choose per placement. Migration of older songs: default existing
  placements to `Clip` so behaviour is preserved, or `Repeat` if we accept a one-time semantic change.

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

- [x] Use one instrument's audio to control another (e.g. sidechain compression). Full path
  shipped: data model + persistence + MCP + GUI + engine audio routing.
  - [x] `Instrument.sidechain_source_id: Option<InstrumentId>` engine field + getter/setter.
  - [x] `EngineCommand::SetSidechainSource` + handler (rejects self-routing). `transactions::try_clone`
        and `hub.rs` permission gate updated.
  - [x] `InstrumentSnapshot.sidechain_source_id` exposed to MCP via `InstrumentInfo`.
  - [x] `Session::set_sidechain_source` + `SynthBridge::set_sidechain_source` + new MCP tool
        `set_sidechain_source(instrument_id, source: Option<u64>)`.
  - [x] `InstrumentState.sidechain_source_id` persistence (`#[serde(default)]`) + load path sends
        the engine command after instrument construction.
  - [x] GUI combobox in the instrument-edit window listing all other instruments + `— None —`.
  - [x] **Engine audio routing**:
    - `SynthEngine::prev_instrument_outputs: HashMap<InstrumentId, AudioBuffer>` — pre-allocated
      on instrument add/remove (no audio-thread allocs).
    - `Instrument::last_output_interleaved()` exposes the post-effect-chain interleaved-stereo
      output for the engine to capture after each `process()`.
    - `Instrument::feed_sidechain_inputs(buffer)` walks the effect chain and calls
      `AudioEffect::set_sidechain_input` on every effect slot. Default trait method is a
      no-op; `Compressor` overrides to forward into its existing inherent
      `set_sidechain_input`.
    - `SynthEngine::process_voices` uses previous-callback semantics (read source's
      previous output, then process, then capture this callback's output for next time).
      Introduces ~1 audio-buffer of sidechain detection latency. Order of `self.instruments`
      no longer matters — A and B can sidechain each other safely.
    - Removing an instrument clears its prev cache *and* clears any other instrument's
      `sidechain_source_id` that pointed at it.
  - [ ] Cycle detection deeper than 1 (currently only self-routing is rejected at the engine
        boundary; longer cycles are harmless thanks to previous-buffer semantics but produce
        unintuitive ducking patterns).

### 3.5 Polyphony settings

- [x] Voice count configurable per instrument (GUI control) — DragValue 1–128 in the instrument edit window;
  applied at project-reload time (engine voice pool is fixed-size at construction).
- [x] Allocation mode (Poly / Mono / Legato / Unison) — combobox in the instrument edit window, persisted.
- [ ] Voice stealing mode selection (oldest, quietest, none) — engine + persistence done, GUI selector
  not yet added (defaults still applied).
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

Tier-0 music-analysis tools shipped in v0.276.0 (`analyze_harmony`,
`analyze_mix_bus`, `analyze_section`); Tier-1 follow-ups for drum-track
filtering and per-track contribution breakdown shipped in v0.277.0.
Roadmap for the remaining tiers lives in `docs/mcp-music-tools-plan.md`.

- [x] Per-track stem breakdown for `analyze_section` — v0.277.0
      (`include_per_track` parameter).
- [x] Drum-track filtering for `analyze_harmony` — v0.277.0
      (`exclude_drums` defaults to true; `exclude_track_ids` for explicit drops).
- [ ] Tier 1: `analyze_pattern` (symbolic pattern stats — density, polyphony,
      rhythmic and velocity variance), `analyze_instrument_range` (sweep an
      instrument across MIDI range; flag aliasing, energy loss, pitch
      tracking), `render_section_to_wav`.
- [ ] Tier 2: `generate_chord`, `transpose_notes`, `quantize_notes_to_scale`,
      `quantize_notes_to_grid`, `analyze_groove`, `analyze_velocity_response`.
- [ ] Tier 3: `analyze_arrangement`, `compare_to_reference`,
      `compare_patterns`, `compare_patches`, `humanize_notes`,
      `generate_variation`, `analyze_track`, `get_mix_meters`.
- [ ] Enable AI to "play freely" via MCP to autonomously generate complete songs and arrangements
- [ ] Implement real-time parameter interpolation (gliding) to allow smoother AI-driven sound design

---

## 7. AWE Improvements

Findings and concrete ideas: `docs/AWE-Improvement-Findings.md`.

### 7.0 AWE acoustic engine — prioritized plan

#### Phase 2 — Medium complexity

- [ ] **7. Per-surface materials** — `MaterialConfig { floor, walls, ceiling }` instead of single global `Material`, ISM
  uses correct material per reflection
- [ ] **8. Second-order reflections** — extend ISM from 6 to ~30 taps (configurable `ReflectionOrder(u8)` 1–3)
- [ ] **10. Resonant objects** — sympathetic resonance from objects in the room (strings, membranes, plates, Helmholtz
  cavities, loose panels, chimes), implemented as bandpass + feedback at object frequency
- [ ] **12. Doppler effect** — track radial velocity between source/listener, shift pitch via variable delay read speed:
  `ratio = v_sound / (v_sound + v_radial)`

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
