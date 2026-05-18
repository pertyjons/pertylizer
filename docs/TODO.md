# TODO - Pertylizer

## 0. Known Bugs

### 0.1 Misc findings

- There is no way to look at pattern which is not in a track.. Need some sort of view/list of patterns.
- When saving a project with samples, the save should always be in zip-format and file extention .zip, and all other
  should be saved in json with file extention .json
- In rack view, when a module is below/under the instrument list to the left the module is getting the mouse input not
  the instrument list.
- [x] **When a song ends (last track) the song should stop, the timer and the vertical bar should also
  stop.** Fixed by caching `Song::calculate_length()` in `SequencerEngine` and adding a non-looping
  auto-stop branch to the per-tick loop next to the existing loop-wrap branch: at the song end the
  sequencer releases active notes, resets to tick 0, and transitions to `PlayState::Stopped`. The
  audio thread observes the `Playing → !Playing` transition after `sequencer.process()` and mirrors
  `EngineCommand::Stop` on the transport side (clear `is_playing`, `all_notes_off` on every
  instrument), so the GUI playhead and time counter stop together.
- [x] **MCP `save_project` / `load_project` / `new_project` time out when the Pertylizer window is minimized,
  hidden, or unfocused.** Fixed by routing MCP project I/O off the GUI thread — the bridge now calls
  `project_apply::apply_project` directly with `block_in_place`, and notifies the GUI via
  `McpSharedState.pending_project_refresh` + `project_revision`. `submit_project_action`,
  `pending_project_action`, and `project_action_result` are removed.
- **Follow-up: same fix template applies to the other `pending_*` queues.** `pending_patch`,
  `pending_awe_state`, and `pending_auto_layout` are still GUI-drained and will hang the same way if MCP-side
  writes ever block on confirmation. They don't today (none use a condvar), but the architectural shape is
  worth migrating: MCP writes shared state directly, GUI consumes a refresh marker. Filed as separate task.

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

| Entity                       | Field exists?                        | MCP read | MCP write       |
|------------------------------|--------------------------------------|----------|-----------------|
| `InstrumentState`            | ✅ `patch.rs:697`                     | ❌        | ❌               |
| `Patch`                      | ✅ `patch.rs:130, 307` (Option)       | ❌        | ❌               |
| `AwePresetFile`              | ✅ `patch.rs:792`                     | ❌        | ❌               |
| `Song`                       | ❌ — add to `synth_sequencer/song.rs` | ❌        | ❌               |
| `Pattern`                    | ❌ — add to `synth_sequencer`         | ❌        | ❌               |
| `SequencerTrack`             | ❌ — add to `synth_sequencer`         | ❌        | ❌               |
| `Sample` entry               | ❌ — add to sample registry           | ❌        | ❌               |
| Module *instance* (in patch) | ❌ — separate from `ModuleDescriptor` | ❌        | ❌               |
| `ModuleDescriptor` (type)    | ✅ `module_traits.rs:869`             | ✅        | n/a (hardcoded) |
| `ParameterDescriptor` (type) | ✅ `module_traits.rs:586`             | ✅        | n/a             |
| `PortDescriptor` (type)      | ✅                                    | ✅        | n/a             |
| `ChoiceOption` (type)        | ✅ `module_traits.rs:557` (Option)    | partial  | n/a             |

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
- [ ] Surface in `get_module_info` (MCP read); add `set_module_description(instrument_id, module_id, description)`
  MCP tool (MCP write). Accept `""` to clear.
- [ ] Editable from GUI (module header context menu)

#### Persistence (must round-trip on save/load)

Module-instance descriptions **must** survive serialization, both at the project and standalone-patch
level — otherwise AI-applied notes silently vanish on save/reload. Same pattern as
`Patch.description` and the planned color persistence above.

- [ ] **Project save** — every module instance's description persisted inside its containing patch
  in the project JSON. Round-trip test: MCP-set description on `lfo-1` → `save_project` →
  `new_project` → `load_project` → `get_module_info` returns the same text.
- [ ] **Standalone patch save** — the per-instance description travels with the .json patch file
  when the user invokes "Save Patch…". A patch saved by AI must carry its module notes into other
  projects that load it.
- [ ] **Project / patch load → engine mirror** — on load, copy each saved module description into
  the engine's runtime mirror so subsequent MCP reads see what was loaded, not stale defaults.
  Mirror the `Patch.description` → `Instrument.description` plumbing used in Phase 1.
- [ ] **No partial states** — do not ship `set_module_description` without the full save/load path;
  document as known-broken until both halves land.

### Cross-cutting

- [ ] Include all instance-level descriptions in `get_graph_diagnostics` / `analyze_note` output so AI sees
  intent alongside structure
- [ ] Surface descriptions as tooltips on the corresponding GUI elements
- [ ] Decide on max length (suggest 500 chars soft, 2000 hard) — long enough for a paragraph, short enough to
  stay readable in tooltips
- [ ] Persistence format: inline in the existing JSON containers (no sidecar files)

---

## ★ Color fields via MCP

Color fields already exist on several entities (Patch, Instrument, Group, SequencerTrack), but
**no MCP setter** exposes them — AI can build a song but can't paint the strips/tracks to make the
arrangement visually scannable. Parallel structure to the description roadmap above: read on the
existing getter response, write via a dedicated setter routed through
`bridge.rs` → `mcp_bridge.rs` → `server.rs`. Color is also already GUI-editable in most cases.

### Current status of color fields

| Entity            | Field exists?                        | MCP read | MCP write |
|-------------------|--------------------------------------|----------|-----------|
| `InstrumentState` | ✅ `patch.rs:700` `Option<HexColor>`  | ❌        | ❌         |
| `Patch`           | ✅ `patch.rs:255` `Option<HexColor>`  | ❌        | ❌         |
| `Group`           | ✅ `patch.rs:316` `Option<HexColor>`  | ❌        | ❌         |
| `SequencerTrack`  | ✅ in song JSON `{r, g, b}` per track | ❌        | ❌         |

### Work to do

- [ ] Surface color on `get_instrument_info` / `list_instruments` (MCP read); add
  `set_instrument_color(instrument_id, color)` MCP tool. Accept `"#RRGGBB"` / `"#RRGGBBAA"` and
  `""`/`null` to clear back to "auto" / default.
- [ ] Surface patch color on the same getters as a separate `patch_color` field, mirroring how
  `patch_description` is exposed alongside `description`. Add `set_patch_color` MCP tool.
- [ ] Surface track color on `list_tracks`; add `set_track_color(track_id, color)` MCP tool.
- [ ] Surface group color on `get_instrument_info` (or wherever groups are listed); add
  `set_group_color` MCP tool.
- [ ] Decide whether AI-friendly named palettes are useful (`"warm-orange"`, `"cool-blue"`) on top of
  raw hex — same pattern as `set_awe_preset` vs `set_awe_parameter`. Out of scope for v1; raw hex
  is enough.

### Persistence (must round-trip on save/load)

Color writes via MCP **must** survive serialization, both at the project and standalone-patch level
— otherwise AI-applied colors silently vanish on save/reload. This is the same architectural
pattern used for `Patch.description` (runtime mirror in the engine, project load copies in, project
save reads back).

- [ ] **Project save** — all four entities' colors persisted in the project JSON. Verify by
  round-tripping: MCP-set a color → `save_project` → `new_project` → `load_project` → color is the
  same. `InstrumentState.color`, `SequencerTrack` color and `Group.color` already serialize via
  serde; mainly need to confirm the engine-side runtime mirror is read back into the snapshot at
  save time (mirror the `description` plumbing).
- [ ] **Standalone patch save** — `Patch.color` travels with the .json patch file when the user
  invokes "Save Patch…" from the instrument-edit window. Important because a patch saved by AI
  should carry its color into other projects that load it.
- [ ] **Project load → engine mirror** — on `load_project`, push each saved color into the engine's
  runtime mirror (analogous to how `Patch.description` is copied into `Instrument.description` at
  load time) so subsequent MCP reads see what was loaded, not stale defaults.
- [ ] **No partial states** — if any color setter is added without the corresponding save+load path
  wired, document it as known-broken until both halves land; do not ship a setter that only
  updates the live engine but not the project file.

### Use case (motivation)

When AI builds a multi-instrument song via MCP, every track defaults to the same color (e.g. all
tracks in the just-built sidechain demo render as `{r:100, g:100, b:255}`). With color setters AI
can make the arrangement self-documenting at a glance — e.g. red kick, blue pad, green bass.

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
- [x] Undo for tempo set / remove — `UndoAction::SetTempo { tick, old_bpm,
  new_bpm }` captures both apply and remove paths via `Option<Bpm>`; pushed
  on every Apply/Remove menu click and applied through `apply_undo_action`.

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

### 2.7 Transport loop region — visibility + MCP control

The transport loop region (`SharedState.loop_enabled` / `loop_start` / `loop_end`, set via right-click
on the arrangement ruler) silently clips playback wrap. Today the loop is invisible in the timeline
ruler — the only way to discover it exists is via the right-click context menu. If a user (or AI)
extends the arrangement past a previously-set `loop_end`, playback keeps wrapping at the stale
position with no visual hint, and there is no MCP tool to inspect or clear the region.

- [x] **Persistent visual markers on the timeline ruler** — `TransportState`
  now mirrors `SequencerEngine` loop state into `(loop_enabled,
  loop_start_ticks, loop_end_ticks)` atomics; the arrangement ruler draws a
  cyan ribbon, faint timeline band, vertical edges and inward-pointing flag
  triangles whenever the loop is enabled.
- [x] **Status indicator in the transport bar** — small "LOOP s–e" badge
  (bar-numbered) appears in `draw_transport_bar` when the loop is active;
  clicking it sends `EngineCommand::SetLoop { enabled: false }` to clear.
- [x] **MCP exposure of the transport loop** — `set_transport_loop(start_beats,
  end_beats, enabled)` and `clear_transport_loop()` tools route through
  `bridge.rs` → `mcp_bridge.rs` → `server.rs` onto the same
  `EngineCommand::SetLoop` path used by the GUI. `SongInfo` now carries
  `transport_loop_enabled`, `transport_loop_start_beats`,
  `transport_loop_end_beats` so AI can detect a stale region.
- [x] **Auto-extend on arrangement growth** — `place_pattern` /
  `place_patterns` compute the new placement's end tick and call
  `AppSynthBridge::auto_extend_transport_loop`, which only extends when the
  loop is already enabled and the placement reaches past `loop_end`. The
  Neuro F#m 174 (2026-05-13) pitfall no longer silently clips playback.

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
    - [x] Cycle detection deeper than 1 — both the engine
      (`SynthEngine::sidechain_chain_contains`) and the MCP bridge
      (`set_sidechain_source` pre-check) walk the proposed chain up to
      `instruments.len()` hops and reject anything that loops back through
      the target instrument. Bounded iteration so a corrupted chain can't
      spin forever; MCP returns a clear "would form a cycle" error.

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
filtering and per-track contribution breakdown shipped in v0.277.0. The
roadmap doc (`docs/mcp-music-tools-plan.md`) was closed and deleted on
2026-05-17 — remaining work for that roadmap lives below.

- [x] Per-track stem breakdown for `analyze_section` — v0.277.0
  (`include_per_track` parameter).
- [x] Drum-track filtering for `analyze_harmony` — v0.277.0
  (`exclude_drums` defaults to true; `exclude_track_ids` for explicit drops).
- [x] Tier 1: `analyze_pattern` (2026-05-16), `analyze_instrument_range`
  (v0.284.0). `render_section_to_wav` still pending — pick up when
  `compare_to_reference` becomes relevant.
- [x] Tier 2: `generate_chord`, `transpose_notes`, `quantize_notes_to_scale`,
  `quantize_notes_to_grid` (v0.285.0); `analyze_velocity_response` (v0.284.0);
  `analyze_arrangement`, `analyze_form_map`, `find_motifs`,
  `analyze_hook_strength` (v0.286.0); `analyze_tension_curve`,
  `suggest_music_fixes` (v0.287.0 — closes Tier-2 #14 and #16).
  `analyze_groove` still pending — `analyze_drum_groove` (v0.283.0)
  already covers the highest-value groove diagnostic.
- [ ] Tier 3: `compare_to_reference`, `compare_patterns`, `compare_patches`,
  `humanize_notes`, `generate_variation`, `analyze_track`, `get_mix_meters`.
- [ ] Enable AI to "play freely" via MCP to autonomously generate complete songs and arrangements
- [ ] Implement real-time parameter interpolation (gliding) to allow smoother AI-driven sound design

### 6.2 Technical follow-ups from the MCP music tools plan

Moved from §7 of the (now-deleted) MCP music tools plan on 2026-05-17 when the plan was
closed. The four shipped follow-ups (`OfflineEngineSession` for arrangement renders,
rayon-parallel per-track renders, `Song::tracks_mut` / `set_solo_only` helpers, embed
`MixBusMetrics` in `TrackContribution`) are documented in `docs/history.md` against their
ship dates.

- [ ] **`HarmonyScope` enum to fix `analyze_song_harmony` argument sprawl.** `analyze_song_harmony`
  takes 8 arguments and carries `#[allow(clippy::too_many_arguments)]`. Two of them
  (`exclude_drums`, `exclude_track_ids`) are only meaningful in arrangement scope; pattern scope
  currently emits a runtime warning if they're passed. Introduce
  `enum HarmonyScope { Pattern { pattern_id: PatternId }, Arrangement { start: Option<u64>,
  end: Option<u64>, exclude_drums: bool, exclude_track_ids: HashSet<TrackId> } }` at the bridge
  boundary; the flat `AnalyzeHarmonyParam` (JSON-schema layer requires it) maps into the enum
  inside the bridge. The runtime "ignored in pattern scope" warning becomes a compile-time
  impossibility and the `#[allow]` disappears. Touches `synth_mcp::bridge`, the bridge impl,
  and the arrangement-vs-pattern branch in `analyze_song_harmony`. Medium impact — pick up
  when next touching the harmony analyzer.
- [ ] **`synth_sequencer::shared_song(Song) -> Arc<RwLock<Song>>` constructor.** Grep finds ~9 sites
  that wrap a `Song` in `Arc::new(parking_lot::RwLock::new(...))` verbatim
  (`crates/pertylizer/src/audio/export.rs:214`, `main.rs:83`, `mcp_shared.rs:56`, sequencer
  tests, the per-track render loop, etc.). Strictly cosmetic; not on any hot path. Pick up
  as a drive-by when next touching one of those sites.
- [ ] **`OfflineNoteSession` — engine reuse across patch-sweep steps.** `analyze_instrument_range_impl`
  and `analyze_velocity_response_impl` (`crates/pertylizer/src/mcp_bridge.rs`) call
  `analyze_rendered_note` once per swept value; each call goes through
  `audio::preview::render_note_to_buffer`, spins up a fresh `SynthEngine`, and reloads the
  instrument's module graph + sample data. For a 60-note semitone-step sweep that's 60 fresh
  engines; for the default 8-step velocity sweep that's 8. Mirror §7.1's `OfflineEngineSession`
  — wrapper takes `SynthSession` + `SharedSampleLibrary` + `InstrumentId` at construction,
  builds the engine + loads the patch + samples once, then exposes
  `render(note, velocity, duration_ms, tail_ms) -> RenderedNote` per call. Reproduce the
  voice-bleed drain between renders (same problem as §7.1). Determinism tests would mirror
  `tests/arrangement_render_determinism.rs::session_render_range_is_bit_exact_across_three_calls`.
  After session-reuse lands, parallelize the sweep target vector with `par_iter` for a 2-4×
  speedup on top (same sequence as §7.1 → §7.2).

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
