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
- [x] **`ParamId(Arc<str>)` off-thread drop (F1 residual). — RESOLVED.** The full
  fix shipped: `SequencerEngine::clear_automation_dedup` (`sequencer_engine.rs:656`)
  drains cleared/replaced automation targets into the dedicated `automation_trash`
  ring (`set_automation_trash`), and the main thread frees them via
  `automation_trash_consumer` (`synth_engine.rs:202`). The `ParamId(Arc<str>)` final
  drop never lands on the audio thread (documented full-channel fallback aside).

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

### 3.5 YAMS Script editor follow-ups

- [x] **Detect/warn on recursive or self-referential script source graphs.** A script's
  sources are *address-based* (`src x = lfo-1.out`, `src y = scr-2.out1`) and are resolved
  with a **1-block latency** into a scratch buffer — they do **not** flow through the graph's
  cable connections, so the existing cable cycle-detection (`drag_cycle_blocked`, the topo
  sort in `graph.rs`) does **not** cover them. Note up front: because YAMS bytecode is
  straight-line (no loops/recursion — `bytecode.rs:5`) and the 1-block latency breaks the
  dependency, a self-reference (`scr-1` reading `scr-1.out1`) or a script↔script cycle
  **cannot** infinite-loop or stack-overflow at runtime; it just reads last block's value.
  So this is a **UX/correctness warning**, not a safety fix: surface "this script feeds back
  on itself / forms a cycle" so the user isn't surprised by a 1-block-delayed feedback path.
  Build a dependency graph over scripted slots (extract each script's `Module{..}` source
  addresses, same extraction as §3.4) and flag self-edges + cycles in the ƒx editor status
  line. Decide whether to hard-block or just warn (lean warn — delayed feedback is sometimes
  intentional, e.g. a leaky integrator).

- [x] **"Select input" picker in the ƒx expression editor.** Add a button (e.g. labelled
  "Select input") *before* the existing Format button in the script popup
  (`draw_slot_expression_editor`, `patch_editor.rs`). It opens a **tree picker**: top level =
  every module in the patch, expandable to that module's selectable members (output ports and
  modulatable parameters), plus the macro/context sources (`velocity`, `beat`, …). Selecting a
  leaf inserts a suggested binding into the editor at the cursor — a `src <name> = <address>`
  line with an auto-derived variable name (e.g. `src lfo1_out = lfo-1.out`) and the assignment.
  Source the module/parameter list the same way the Mod Matrix address pickers do (S1.5c —
  the patch descriptor catalog), so it stays in sync with what `resolve_source` can actually
  bind.
  — *Done: output ports only (params as YAMS sources are still deferred — `ScriptInput::ModuleParam`
  needs the cross-crate resolver); macro/context leaves insert bare identifiers.*

---

## 4. Template Library & Presets

### 4.1 Template library

- [ ] Add patch template directory and `Save Patch as Template` action
- [ ] Add Patch Template browser to load patch templates
- [ ] Support optional `license` and `min_app_version` metadata in group templates

### 4.2 Preset sharing

- [ ] Community format for sharing patches online

---

## 5. AI & Automation

### 5.1 MCP & AI Interaction

- [ ] Tier 3: `compare_to_reference`, `compare_patterns`, `compare_patches`,
  `humanize_notes`, `generate_variation`, `analyze_track`, `get_mix_meters`.
- [ ] Enable AI to "play freely" via MCP to autonomously generate complete songs and arrangements
- [ ] Implement real-time parameter interpolation (gliding) to allow smoother AI-driven sound design

### 5.2 Technical follow-ups from the MCP music tools plan

- [x] **`HarmonyQuery` enum replaces `analyze_song_harmony` argument sprawl. — DONE** (commit `fdcb506`;
  correctness warning earlier in `6b59a28`). `analyze_song_harmony` now takes a `HarmonyQuery
  { Pattern{pattern_id} | Arrangement{start_tick, end_tick, exclude_drums, exclude_track_ids} }` +
  `grouping_ticks` (4 args; the `#[allow(clippy::too_many_arguments)]` is gone). The exclusion options
  live only on the `Arrangement` variant, so "ignored in pattern scope" is a compile-time impossibility
  for internal callers. A `harmony_query_from_flat()` mapper centralizes the flat MCP-param→enum
  conversion and the two pattern-scope "ignored" warnings; the public `analyze_harmony` bridge and
  `analyze_harmonic_function` both surface them (the latent warning-drop seam in the wrapper callers is
  resolved). Named `HarmonyQuery`, not `HarmonyScope`, because the latter is the existing serialized
  *output* enum. Behavior-preserving across all scope/option combos.
- [x] **`shared_song(Song) -> Arc<RwLock<Song>>` constructor. — DONE** (commit `ecb339f`).
  Added as `synth_engine::shared_song` (not `synth_sequencer` as originally suggested — that
  pure-data crate has no `parking_lot` dep, and adding one purely for this convenience would invert
  layering; `synth_engine` already owns the shared-song threading model and depends on `parking_lot`).
  All **7** production call sites converted (the 7 in `synth_engine.rs` / others originally counted
  turned out to be inside `#[cfg(test)]`); test fixtures keep the inline form intentionally.
  Unit test `shared_song_tests::shared_song_wraps_a_readable_song`.
- [ ] **`OfflineNoteSession` — engine reuse across patch-sweep steps.** `analyze_instrument_range_impl`
  (`crates/pertylizer/src/mcp_bridge.rs:7366`) and `analyze_velocity_response_impl` (`:7426`) call
  `analyze_rendered_note` (`:7303`) once per swept value via the `sweep_range` loop; each call goes
  through `audio::preview::render_note_to_buffer` (`crates/pertylizer/src/audio/preview.rs:203`),
  which spins up a fresh `SynthEngine::new()` and reloads the instrument's module graph + sample
  data. For a 60-note semitone-step sweep that's 60 fresh engines; for the default 8-step velocity
  sweep that's 8. Mirror the existing `OfflineEngineSession`
  (`crates/pertylizer/src/audio/arrangement_render.rs:219`, ctor `:262`/`new_with_scope` `:274`,
  `render_range` reuses one engine across calls) — a wrapper that takes
  `SynthSession` + `SharedSampleLibrary` + `InstrumentId` at construction, builds the engine +
  loads the patch + samples once, then exposes `render(note, velocity, duration_ms, tail_ms)
  -> RenderedNote` per call. Reproduce the voice-bleed drain between renders (same problem
  `OfflineEngineSession` already solves). Determinism tests would mirror
  `tests/arrangement_render_determinism.rs::session_render_range_is_bit_exact_across_three_calls`
  (`:191`). After session-reuse lands, parallelize the sweep target vector with `par_iter` for a
  2-4× speedup on top. **→ Plan: `plans/offline-note-session-plan.md`.**
- [x] **Static `#[schemars(range(...))]` on fixed-range numeric MCP fields. — DONE** (commit `650a588`).
  All fixed-range numeric tool fields — MIDI note/pitch (0–127), velocity (0/1–127), MIDI channel
  (1–16), LFO index (1–4) — now carry `#[schemars(range(min, max))]` so their `JsonSchema` exposes
  machine-readable `minimum`/`maximum`, not just prose. Covers single-line and multi-line attrs and
  `Option<u8>` variants; verified against `schemars` 1.2.1. Schema test
  `server::schema_range_tests::fixed_range_fields_expose_min_max_in_schema` asserts the emitted bounds.
- [ ] **Uniform machine-readable bounds on `synth_core` newtypes.** Newtype clamping is inconsistent:
  `NormalizedValue`/`BipolarValue`/`Velocity` clamp in `new()` and `Phase` wraps (`rem_euclid`), but
  `Hertz`/`Gain`/`Cents`/`Semitones` are `const fn new` with no clamp, and there is no uniform
  `const RANGE: ValueRange` on any newtype (only ad-hoc `MIN`/`MAX` on a few) — so a shared
  "spec → (schema | validation)" abstraction can only read bounds from the per-module
  `ParameterDescriptor`, never from the type. `Param::with_f32` (`crates/synth_core/src/params/mod.rs:1070`
  dispatching to per-module impls) then re-clamps ad hoc and sometimes hardcodes a range that
  duplicates the descriptor (e.g. the 2026-05-27 `Detune::with_f32` `-100..100` clamp at
  `crates/synth_core/src/params/oscillators.rs:530` mirrors the descriptor range at
  `crates/synth_modules/src/oscillator.rs:354` by hand). No `BoundedNewtype` trait exists yet.
  Consider one (or a `const RANGE`) so bounds live on the type and descriptors/`with_f32` derive
  from it. Larger, cross-cutting refactor — plan separately. **→ Plan: `plans/newtype-bounds-plan.md`.**

---

## 6. AWE Improvements

Findings and concrete ideas: `docs/AWE-Improvement-Findings.md`.

### 6.0 AWE acoustic engine — prioritized plan

#### Phase 2 — Medium complexity

- [ ] **7. Per-surface materials** — `MaterialConfig { floor, walls, ceiling }` instead of single global `Material`, ISM
  uses correct material per reflection
- [ ] **8. Second-order reflections** — extend ISM from 6 to ~30 taps (configurable `ReflectionOrder(u8)` 1–3)
- [ ] **10. Resonant objects** — sympathetic resonance from objects in the room (strings, membranes, plates, Helmholtz
  cavities, loose panels, chimes), implemented as bandpass + feedback at object frequency
- [ ] **12. Doppler effect** — track radial velocity between source/listener, shift pitch via variable delay read speed:
  `ratio = v_sound / (v_sound + v_radial)`

### 6.1 Rework room visualization

- [ ] Redesign the 3D isometric room rendering
- [ ] Improve animations (sound rings, reflection paths)
- [ ] Better visual clarity for room shape and dimensions

### 6.2 Differentiate effects more clearly

- [ ] Each material/effect should have more distinct visual representation
- [ ] Color-coded zones, animated textures per material, spectral visualization

---

## 7. Visualizer & OSC

### 7.1 OSC control & connectivity

- [ ] OSC enable/disable toggle in Pertylizer settings GUI
- [ ] `/viz/` OSC control endpoints (effect select, param set, scene load)
- [ ] OSC `/viz/theme/select` control endpoint
- [ ] OSC parameter tweaking — live control of intensity, speed, scale per effect
- [ ] Support connecting multiple OSC clients simultaneously

### 7.2 Post-processing & shaders

- [ ] Chromatic aberration — intensity scales with RMS level
- [ ] Glitch/distortion effect — triggered by CPU spikes or spectral flux
- [ ] Kaleidoscope mode — radial scene mirroring (configurable segment count)
- [ ] CRT/VHS filter — scanlines, color bleed, static noise
- [ ] Motion blur — strength synced to tempo

### 7.3 Multi-effect layering

- [ ] Show 2–3 effects simultaneously instead of one at a time
- [ ] Per-instrument visual layers — each instrument gets its own color/effect layer
- [ ] Blending modes between layers (additive, multiply, screen)
- [ ] Layer opacity control via OSC

### 7.4 Reactive environment

- [ ] Skybox that reacts to music — stars pulse with RMS, clouds move with tempo
- [ ] Reactive ground — ripples on note-on, cracks on bass hits
- [ ] Fog/mist density driven by reverb level or sustain
- [ ] Day/night cycle driven by song position
- [ ] Weather effects — rain on high spectral flux, lightning on transients

### 7.5 Advanced simulations

- [ ] Swarm/flock simulation — particles flock or scatter based on dynamics
- [ ] Cloth simulation — fabric that billows and ripples with FFT energy
- [ ] Text/typography — display song title, BPM, key in stylized 3D text
- [ ] AWE spatialization — visualize sound source position in 3D space

### 7.6 Video export

- [ ] Video recording — render to MP4 or image sequence

---

## 8. Advanced / Long-term

### 8.1 Audio tracks

- [ ] Import and arrange audio files, not just synth tracks

### 8.2 Audio recording

- [ ] Record external audio via cpal input

### 8.3 Clip launching

- [ ] Ableton-style live mode with follow actions

### 8.4 Plugin export

- [ ] Export instruments as VST3/CLAP plugins
