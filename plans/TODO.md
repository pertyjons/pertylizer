# TODO - Pertylizer

## 1. Sequencer & Arrangement

### 1.1 Tempo automation

> **Not to be confused with the deleted `Global(Tempo)` automation lane.** Two different
> things shared the "tempo automation" name. (1) The generic `AutomationTarget::Global(Tempo)`
> automation *lane* was a no-op and was **removed for good** on 2026-06-01 — tempo changes the
> playback time grid itself, so it can't be a per-block lane value, and a lane would be a second
> source of truth competing with the tempo map. That is the dead code; it is **not** coming back.
> (2) The **tempo map** below is a separate, live mechanism and *is* the real feature to finish.

- [ ] **Expose + edit the tempo map** (`Song::set_tempo_at` / `tempo_at` / `tempo_changes`,
  `song.rs:686`). The tempo map already exists and is partly wired: the engine reads it, and the
  arrangement view already draws tempo changes (`gui/sequencer/arrangement.rs:1299`) and can set
  them (`arrangement.rs:1010` → `set_tempo_at`). What remains:
    1. **MCP tools** for the map — today MCP only exposes `set_song_tempo` (the global default);
       there is no way to add/move/remove tempo points in the map via MCP.
    2. **A dedicated GUI tempo-track/curve editor** — the current editing is rudimentary, not a
       proper tempo lane.
    3. **Interpolation between adjacent points** (accelerando/ritardando ramps). `tempo_at`
       (`song.rs:697`) is **step-only** — it returns the previous point's bpm with no ramping.
       This is the real "tempo automation" feature — built on the tempo map, not a generic
       automation lane.

### 1.2 Section markers

- [ ] Verse, chorus, bridge labels in the arrangement

### 1.3 Macro controllers

- [ ] Map multiple parameters to a single macro knob for live performance

### 1.4 Track reorder via drag-handle

- [ ] Replace (or complement) the current ↑/↓ arrow buttons in the track header with a drag-handle
  (e.g. `ri::DRAG_MOVE_2_LINE`) on the left edge of each row. Drag should snap the row vertically to the
  nearest neighbour while dragging, then commit on release via `Song::reorder_track`. The arrow buttons
  shipped because they are simpler and robust, but drag-handle reorder is the DAW convention.

### 1.5 Pattern looping within placement length (future)

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
  `gui/sequencer/arrangement.rs` (mini-note loop, near the `inst_color_cache` use) should repeat the miniature
  across the placement's `effective_length / pattern.length` iterations, so the user sees what they hear.
- [ ] **Add a toggle on `PatternPlacement`** (`loop_mode: PlacementLoopMode { Clip, Repeat }`, default
  `Repeat` to match DAW expectations). Surface in the placement context menu and in the right-edge
  resize-grab tooltip so the user can choose per placement. Migration of older songs: default existing
  placements to `Clip` so behaviour is preserved, or `Repeat` if we accept a one-time semantic change.

### 1.6 Persist the transport loop region across save/load

- [ ] **`set_transport_loop` is not saved with the project.** The loop region (start/end/enabled) set via
  the `set_transport_loop` MCP tool (and the arrangement-ruler loop) is live transport state that
  `save_project` drops: a saved `.json` has no `transport_loop` under `song` or `global`, so on reload the
  arrangement plays through once and stops, and the loop must be re-enabled by hand. Surfaced while building
  the `YAMS Script Lab` example (the scr-1 "Rhythm Brain" demo relies on a loop to keep `playing` true).
  Fix: serialize the loop region with the song (e.g. `Song.transport_loop: Option<LoopRegion { start, end,
  enabled }>`) and restore it on load. Decide whether `enabled` persists or always loads disabled. If the
  loop is *intentionally* ephemeral, document that in the `set_transport_loop` / `save_project` tool
  descriptions instead, so it is not a silent surprise.

---

## 2. Sound Design — Expanded Capabilities

### 2.1 Sample & wavetable import

- [ ] Sample import — load .wav files as oscillator source or in granular synth
- [ ] Wavetable import — load custom wavetables (Serum format, single-cycle .wav)

### 2.2 Alternative tunings

- [ ] **Support tunings other than 12-TET.** Today the pitch path hardcodes 12-tone equal
  temperament when converting `MidiNote` → `Hertz`. Route that conversion through a pluggable
  tuning table so the synth can play just intonation (pure integer ratios like `3/2`, `5/4`),
  microtonal systems (19/22/31-EDO, quarter-tones), and arbitrary historical/non-Western scales.
- [ ] **Load Scala `.scl` files** as the import format — the de facto standard for sharing
  tunings (scale steps given in cents or as frequency ratios). Parsing a `.scl` file fills the
  tuning table from the previous item.

### 2.3 Expression & articulation

**Remaining open work from the retired note-expression roadmap:**

- [ ] **Phase D residual — automate master/return effect params.** A `Filter` can be
  placed on the master or a return bus and set today (`set_master_effect_parameter` /
  `set_return_effect_parameter`), but its cutoff **cannot be swept by an automation
  lane**: `AutomationTarget::Module` resolves only against instrument-owned modules
  (`synth_engine.rs:~3293`, `instruments.iter_mut().find(...)`). Add a target variant
  (e.g. `AutomationTarget::{MasterEffect, ReturnEffect}` keyed by slot + `param_id`)
  dispatched through the same override layer as A2. Delivers exact *shared* SID-style
  filter sweeps. **S** task — build only when a tune genuinely needs a shared (not
  per-instrument) automated sweep; per-instrument sweeps are already covered by A2.

**Iceboxed — the rest of Phase E (build on demand only).** The expensive,
narrow-audience remainder of the old north-star phase. No plan doc; pick up only when a
concrete need appears:

- [ ] Per-note hand-drawn expression curves + the **piano-roll per-note curve editor**
  (the real cost center that gated the whole phase).
- [ ] Per-note **spatial via AWE** — primitive 1 with an AWE room param as the target
  (per-note position in the simulated room). Genuine differentiator; no equivalent in
  other synths — worth keeping on the list even though it is niche.

### 2.4 Polyphony settings

- [ ] **Unison detune + spread controls — NOT IMPLEMENTED (config removed, only a fixed constant remains).**
  The global `AllocationMode::Unison` *mode* works (selectable in the Mode dropdown; `allocate_unison`
  plays the held note on every voice with an evenly-spread pitch detune), but the **detune amount is a
  fixed 10-cent constant inlined in `allocate_unison`** (`voice_allocator.rs`). The old
  `AllocatorConfig.unison_detune: Cents` field was a first-commit (v0.12) design stub that never got a
  setter, `InstrumentParam`, snapshot field, persistence, or GUI — so it was a misleading "config" knob
  that could never actually change. **It was removed** and replaced by the inline constant, so nothing in
  the codebase now pretends unison detune is configurable. **`spread` never existed on any layer** — it is
  a wishlist word from the original §5.4 roadmap stub (commit `181d6c8`), not a half-built feature.

  **Distinct from the per-module unison** (`Oscillator` / `VoiceSynth` / `Fof`), which *is* fully
  implemented with `UnisonDetune` params + MCP + GUI. This item is only about the **voice-allocator's
  global Unison allocation mode**.

  To actually implement it (full vertical slice, in dependency order):
    1. **Allocator/DSP** (`voice_allocator.rs`): re-add `unison_detune: Cents` to `AllocatorConfig` +
       `set_unison_detune()` setter (mirror `set_stealing`); add a **new** `unison_spread` field. Decide
       what "spread" means — almost certainly **stereo width** (pan voices L↔R). That is *new DSP*, not
       plumbing: voices currently only carry a pitch detune (`set_oscillator_detune`); there is no
       per-voice pan, so `Voice`/the voice mixer must gain a per-voice stereo position first.
    2. **Command** (`commands.rs`): `InstrumentParam::UnisonDetune(Cents)` + `UnisonSpread(NormalizedValue)`.
    3. **Engine dispatch** (`synth_engine.rs` ~1693): match arms calling the new setters.
    4. **Snapshot** (`shared_state.rs`): add fields to the instrument snapshot; populate them where
       `allocator_cfg` is read (`synth_engine.rs` ~2670, next to `allocation_mode`/`stealing_strategy`).
    5. **Persistence** (`InstrumentState`, `project_apply.rs`, `project.rs`): serde fields + build into
       `AllocatorConfig` on load + push initial values via `InstrumentParam` (like `AllocationMode` at
       `project_apply.rs` ~687) + defaults at the ~4 `InstrumentState` construction sites.
    6. **GUI** (`gui/egui_backend.rs` + `gui/instrument_rack.rs`): UI-struct fields + two sliders + send
       flags + apply block + dirty (same pattern as the stealing selector), greyed out unless the
       instrument is in `Unison` mode.
    7. **MCP + tests**: expose via `set_parameter` / surface on `get_instrument_info`; round-trip tests
       mirroring the `allocation_mode` ones in `project_load_snapshot.rs`.

  Detune alone (steps 1–7 minus spread) is mostly plumbing and could ship as a small first vertical;
  **spread is the real feature** because it needs new per-voice stereo DSP. Design separately.

---

## 3. UI & Visual Polish

### 3.1 Improve module knobs

- [ ] Better visual design — gradient fill, shadow, tick marks, value tooltip
- [ ] Consistent sizing across module types
- [ ] Arc-style knobs with colored fill showing current value

### 3.2 Redesign instrument list

- [ ] Tabbed interface, mixer-style vertical strips, or collapsible panels

### 3.3 Module Groups — Phase 2–3

- [ ] Phase 2: Template variants (parameter presets with remap)
- [ ] Phase 3: Probes data pipeline (ringbuffers, audio-thread safe collection)
- [ ] Phase 3: Probe rendering (waveform/spectrum/meter) with PortType-based signal type
- [ ] Phase 3: Polyphony probes = sum of voices (mixdown)

### 3.4 Mod Matrix routing visibility

Header badges and MCP surfacing shipped in v0.289.0 (`get_mod_matrix_routings`, virtual `"matrix"`
port on `list_modules`, header arrow badge with tooltip). Remaining work:

- [ ] **Reflect YAMS-script sources in the per-knob source markers + macro rail (S2.4 follow-up).**
  After the expression editor shipped, a scripted Mod Matrix slot has no slot *source* address —
  its sources live inside the script (`src lfo = lfo-1.out`, plus macros like `velocity`/`mod_wheel`).
  `PatchAnalysis::from_panels` (`gui/patch_editor.rs`) only reads `slot_addrs` source/dest, so the
  modules/macros a script reads are **not** marked (the S1.5a/b source glyphs + macro-source rail stay
  dark for script-only sources). Fix: for each entry in `panel.slot_scripts`, extract its referenced
  sources and fold them into `mod_matrix_sources` / `mod_matrix_macros` alongside the scalar slot
  sources. Cleanest extraction is `synth_script::compile(text)` → `program.into_bound(text).inputs`
  (a `Vec<ScriptInput>`; `ScriptInput::Source(SrcAddr::Module{..})` → resolve to `ModuleId` like the
  existing slot-source path, `SrcAddr::Macro(m)` → `mod_matrix_macros`). **Caveat:** `from_panels`
  runs per frame, so compiling every scripted slot each frame is wasteful — cache the extracted
  source set per (slot, script-text) and invalidate when `slot_scripts` changes, rather than
  recompiling unconditionally.

---

## 4. AI & Automation

### 4.1 MCP & AI Interaction

- [ ] Tier 3: `compare_to_reference`, `compare_patterns`, `compare_patches`,
  `humanize_notes`, `generate_variation`, `analyze_track`, `get_mix_meters`.

### 4.2 Technical follow-ups from the MCP music tools plan

- [ ] **Nice-to-have: global drift-lint over preset-backed descriptors (was §3.5 of the deleted
  newtype-bounds plan).** The per-preset drift-guard tests that shipped with `f90125f` only cover the
  params they were hand-written for, so a future dev could still hardcode `.range(-100.0, 100.0)` on a
  *new* `Cents`/`Hertz`/`Gain` param instead of reusing the preset, and nothing would catch the
  re-introduced drift. Add one test that walks **every** registered module's descriptors and, for each
  param whose unit is a preset-backed newtype, asserts its `.range` is one of the approved presets
  (`Cents::DETUNE_RANGE`, `Hertz::OSC_RANGE`, …) rather than a raw literal. Requires a curated
  **allow-list of legitimate one-offs** (not every `Hertz`/`Cents` param maps to a shared preset — some
  genuinely have a unique range), or it produces false positives. Belt-and-suspenders on top of the
  per-param asserts; low priority.
