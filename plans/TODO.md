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

### 2.4 Polyphony settings

- [ ] **Unison detune + spread controls.** **Detune SHIPPED** (`268441f9`,
  configurable end-to-end — see the progress block below). **Spread is the
  remaining work** — full plan in
  [`plans/unison-spread-plan.md`](unison-spread-plan.md). _History (why the
  original config was removed before detune re-added it properly):_ the global
  `AllocationMode::Unison` *mode* always worked, but the detune amount used to be a
  **fixed 10-cent constant inlined in `allocate_unison`** (`voice_allocator.rs`). The old
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

  **Progress — detune vertical (branch `feat/unison-detune`):**
    - [x] **Step 1 — Allocator/DSP.** Re-added `unison_detune: Cents` to
      `AllocatorConfig` (default `10.0` = the old inline constant, so behaviour is
      preserved) + `set_unison_detune()` setter (mirrors `set_stealing`);
      `allocate_unison` now reads `self.config.unison_detune` instead of the
      hardcoded `Cents::new(10.0)`. Green (synth_engine tests pass). **Spread is
      NOT part of this vertical** — it needs new per-voice stereo DSP; deferred.
    - [x] **Step 2 — Command + dispatch.** Added `InstrumentParam::UnisonDetune(Cents)`
      + the engine dispatch arm (`synth_engine.rs` ~1777) calling
      `allocator_mut().set_unison_detune(d)` (mirrors `StealingStrategy`). The
      `InstrumentParam` dispatch match is exhaustive, so the arm is required. Green.
    - [x] **Step 3 — Snapshot.** Added `unison_detune: synth_core::Cents` to
      `InstrumentSnapshot` (shared_state.rs) + populated it from
      `allocator_cfg.unison_detune` in the snapshot build (synth_engine.rs ~2822,
      beside `allocation_mode`/`stealing_strategy`). Updated the
      `tests/instrument_profile.rs` constructor. Green (`--all-targets`).
    - [x] **Step 4 — Persistence.** `InstrumentState` (patch.rs) gains
      `unison_detune: Cents` with `#[serde(default = "default_unison_detune")]`
      (→ `10.0`, so projects saved before the field keep the historical sound) —
      added to BOTH the real struct and the manual-`Deserialize` `Raw` mirror +
      the `raw.unison_detune` mapping + a `default_unison_detune()` fn. Load wires
      it into the `AllocatorConfig` in `install_instrument` (explicitly, *before*
      `..Default::default()`, so the loaded value isn't dropped) AND pushes
      `InstrumentParam::UnisonDetune` (mirrors `allocation_mode`); also set in
      `snapshot_to_instrument_state` + `default_instrument_state`. Regenerated
      `schemas/project.schema.json`. Agent review `[]` (backward-compat default +
      drop-trap both verified). Green.
    - [x] **Step 5 — GUI.** `InstrumentUiState` gains `unison_detune: Cents`
      (struct + Default + `new()`, default 10.0). Added a `DragValue`
      (0..=100 ct, suffix " ct") in the patch bar after the stealing combo,
      `add_enabled(is_unison, …)` so it greys out unless the instrument is in
      Unison mode; on change sends `InstrumentParam::UnisonDetune` via a
      `send_unison_detune` dirty flag + apply block; synced from `inst_state` on
      load (beside allocation_mode/stealing). Agent review `[]` (wiring/greying/
      sync all mirror the allocation_mode pattern; note: allocator-config fields
      are intentionally not reconciled from the live snapshot — shared by all
      siblings, and no MCP path mutates them **yet** → step 6). Green.
    - [x] **Step 6 — Tests (+ MCP triage).** Added two focused serde tests in
      `patch.rs`: a round-trip (set 25 ct → save → load → 25 ct) and a
      backward-compat one (strip the field from the JSON → loads as 10 ct, not 0).
      Added `unison_detune_cents` to the `project_load_snapshot` golden summary +
      regenerated all 10 example-project fixtures (each correctly shows `10.0`,
      proving pre-field projects still load the historical value). **MCP NOT
      added (deliberate triage):** `synth_mcp` exposes *no* allocator-config param
      — `allocation_mode`/`stealing_strategy`/`max_voices` have neither a getter on
      `get_instrument_info` nor a setter. Adding a lone `unison_detune` MCP path
      would be inconsistent piecemeal scope-creep; MCP exposure belongs to a
      separate "surface the whole allocator config (mode/stealing/max_voices/
      detune) via MCP" task. Green.

  **DETUNE VERTICAL COMPLETE** (steps 1–6, merged to `main` as
  `268441f9`): unison detune is configurable end-to-end (allocator → command →
  snapshot → persistence w/ 10 ct backward-compat → GUI slider greyed outside
  Unison mode → tests). **Still open:** (a) **spread** — full implementation plan
  in [`plans/unison-spread-plan.md`](unison-spread-plan.md) (per-voice stereo
  width; recommended defaults locked, ready to build); (b) **MCP** allocator-config
  surface (see step 6 triage).

### 2.5 Hardening Newtype Invariants

- [ ] **Harden type-safety invariants of domain newtypes.** Convert newtypes in `synth_core`
  (like `NormalizedValue`, `BipolarValue`, `Phase`, `MidiNote`, etc.) from using public tuple
  fields (e.g. `pub struct NormalizedValue(pub f32)`) to private fields (`pub struct NormalizedValue(f32)`).
  This prevents external code from bypassing validation constraints and guarantees that values
  remain valid once instantiated. Ensure that:
    1. Validation-guaranteeing constructors (`new()`, etc.) are the only public creation vectors.
    2. Explicit `new_unchecked()` constructors are exposed and used only in performance-critical
       hot paths where the calling context has already proven/ensured correctness.
    3. The helper macros in `macros.rs` and other modules are updated or verified to compile
       properly under module-level visibility rules.

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

### 3.5 MSEG UI overhaul (problematic — needs review)

- [ ] **The MSEG module UI is very problematic and must be reworked.** MSEG is a multi-segment
  envelope (up to 16 segments, each with time/level/curve, plus loop start/end), but it currently has
  **no graphical editor** — the only UI is the generic descriptor-driven knob grid. Consequences:
    1. **The actual envelope shape is not editable in the GUI.** The 48 per-segment params
       (`seg{0..15}_{time,level,curve}`) are deliberately `WidgetHint::Hidden` (added so the shape
       round-trips through save/load and is MCP-settable — see the State Sync work), so the only way
       to draw/shape the envelope today is per-id via MCP `set_parameter`. There is no way to do it by
       hand in the app.
    2. **The visible knobs are awkward.** `Segments`/`Sustain Seg`/`Loop Start`/`Loop End` are integer
       knobs (now `.step(1.0)`-snapped) and `Time Scale` is a multiplier — a grid of knobs is a poor fit
       for what is fundamentally a *curve*.
       Fix direction: build a proper **graphical multi-segment envelope editor** (drag segment
       nodes for time/level, drag handles for per-segment curve, visible sustain + loop-region markers),
       rendered via a custom widget (`WidgetHint::EnvelopeEditor` already exists as a hint). The Hidden
       segment params can stay as the persistence/MCP backing; the editor just reads/writes them. Also
       consider an array-style MCP tool (`set_mseg_segments`) so the shape can be set in one call instead of
       ~50 individual `set_parameter`s. Review the whole MSEG UX as part of this.

### 3.6 `ModuleParam` single-definition cleanup (MAYBE — aesthetics only, future)

- [ ] **Collapse the inherent-vs-trait duplication for the param method set — purely for
  "one definition" tidiness, low priority.** Phase 7 of the param-type-system work
  (`plans/param-type-system-plan.md` §10, shipped) added the `ModuleParam` trait via a
  delegation macro: each of the 67 `*Param` enums + `Param` `impl ModuleParam` by
  *forwarding* to the existing inherent methods (`fn as_f32(&self) { Self::as_f32(self) }`).
  So the bodies live in the inherent impls and the trait is a thin forwarding layer — there
  is a small amount of duplication (the ~470 macro-generated one-liners). The "pure" form
  would make `ModuleParam` the **single** definition and delete the inherent methods.
    - **Why it's only a maybe:** the literal version means the trait must be in scope at the
      **~2489 call sites** of `.as_f32()`/`.with_f32()`/`.same_kind()` across the workspace
      (via a `synth_core::prelude` glob in dozens of files). That is a large, sprawling,
      purely-cosmetic diff with **zero functional/correctness gain** — the aggregate `Param`
      match + the macro already force the full contract on every enum (a missing method is a
      compile error today). YAGNI: nothing currently needs it.
    - **If we ever do it:** **own branch + own session** (it touches most files in the
      workspace). Mechanism: move each method body into `impl ModuleParam for X`, delete the
      inherent method, and add `use synth_core::prelude::*` where the compiler flags missing
      trait scope. Let the compiler drive the call-site fixes; gate per crate.

### 3.7 Unified list-panel follow-ups (deferred from code review)

Surfaced during the shared left-list-panel work (`feat/uniform-list-panels`,
2026-06-24, `gui/list_panel.rs` + Instruments/Patterns/Samples panels). None are
correctness bugs (those were fixed in that branch); these are the cleanup/
efficiency/altitude items deliberately left out of that change.

- [ ] **Cache sample-usage instead of recomputing every frame.** The Sample view
  rebuilds `used_sample_ids` on every repaint by calling
  `self.session.state().shared_graph.get_all_modules()` (which clones *every*
  module snapshot incl. its full `parameters` vec) and scanning for
  `Param::Sampler(SamplerParam::SampleSelect(..))` — see the sample-view call site
  in `gui/egui_backend.rs` (the `used_sample_ids` block just before
  `draw_sample_view`). Only runs while the Sample tab is open, but it allocates +
  walks the whole graph ~60×/sec. Fix: cache the id set and invalidate on a
  graph-version change (`shared_graph.version()`), or expose a lighter query that
  yields just the referenced `SampleId`s without cloning snapshots.
- [ ] **Generalize the per-panel scaffolding (altitude).** `list_panel::row`/
  `header`/`search_box` centralize the row visuals, but the three call sites
  (`render_instruments_panel` in `gui/egui_backend.rs`, `draw_browser_row` in
  `gui/pattern_view.rs`, and the sample loop in `gui/sample_view.rs`) still repeat
  the same surrounding boilerplate: build the used/unused tooltip string, dispatch
  `clicked()`/`double_clicked()`, apply the search-needle filter, and render the
  empty-state placeholder. A higher-altitude helper taking
  `(selected, used, name, tip, kebab) -> RowOutcome { clicked, double_clicked }`
  would remove the repetition the first pass left behind.
- [ ] **Drop the redundant `select` flag in the sample row loop**
  (`gui/sample_view.rs`). It is only ever read in `if select || rename` and
  `rename` already implies selection; the selection assignment can test the row
  response (and `rename`) directly. Pure cleanup, no behavior change.
- [ ] **Detach deleted samples from referencing sampler modules.** Deleting a
  sample from the list kebab (and the old toolbar Delete before it) calls
  `SampleLibrary::remove(id)` but leaves any `Sampler` module still holding that
  `SampleSelect(id)` pointing at a now-missing sample. Pre-existing (not a
  regression from the list-panel work), but the kebab makes deletion easier to
  reach. Confirm the sampler/engine tolerates a missing referenced sample
  gracefully (silent → no sound vs. panic), and consider warning on / blocking
  deletion of an in-use sample, or resetting referencing modules to "no sample".
### 3.8 Shared widget helpers follow-ups (evaluating Phase 2 residual)

Following the evaluation of [widget-helpers-plan.md](file:///home/per/github/pertylizer/plans/widget-helpers-plan.md), these are the remaining areas to polish the GUI helpers layer:

- [ ] **Global FileDialog memory across kinds.** Refactor `ensure_dialog` in [dialogs.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/dialogs.rs) to reuse a single global `FileDialog` instance across all kinds (Open/Save Patch, templates, etc.) rather than rebuilding it when `file_dialog_kind` changes. Update its `config_mut().file_filters` dynamically on every open. This enables directory memory and highlighting (`retain_selected_entry`) to survive switching between Open and Save actions.
- [ ] **Unify inline name/description editors.** Create a helper `inline_editable_text(ui, &mut String, &mut bool, multiline)` in [controls.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/widgets/controls.rs) to wrap the focus-grabbing and lost-focus/Enter-key commit logic currently duplicated in [piano_roll.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/piano_roll.rs#L2510) and [arrangement.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/arrangement.rs#L1580).
- [ ] **Address inline toggle button variations.** Several inline toggle styles (e.g. M/S muting/soloing badges, custom-colored selections) still bypass `toggle_button_colored`. Create a flexible `toggle_badge` or `selectable_toggle` helper to cover these and keep sizes consistent (preventing drift).
- [ ] **Perform a visual eyeball check on normalized captions.** Verify that the normalized size shift (~9px to 10px `size_small`) for the 24 migrated `.small()` labels does not cause visual clipping or alignment issues in tight spaces (especially grid cells in [tracker.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/tracker.rs) and Vol/Pan knob rows in [arrangement.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/arrangement.rs)).

### 3.9 Drop the vendored egui-0.35 forks once upstream ships 0.35

- [ ] **Replace the vendored `third_party/egui-remixicon` crate with the crates.io version once they publish an egui-0.35-compatible release.** The egui 0.34→0.35 upgrade was blocked because neither `egui-remixicon` nor `egui-file-dialog` had a 0.35 release at the time (egui 0.35 landed 2026-06-25).
  * Note: `egui-file-dialog` was already successfully upgraded to its official 0.35-native version `0.14.1` on crates.io and its fork was dropped.
  * For `egui-remixicon`: when upstream releases a 0.35 version, bump `egui-remixicon` in `Cargo.toml`, remove the `[patch.crates-io]` block and the `third_party/egui-remixicon` directory, and verify the build. Watch: https://github.com/get200/egui-remixicon

---

## 4. AI & Automation

### 4.1 MCP & AI Interaction

- [ ] **★ HIGH PRIORITY — make `set_module_description` array-consistent with the other
  mutating tools.** `set_module_description` takes **flat** params (`instrument_id`,
  `module_id`, `description`) — unlike nearly every other mutating MCP tool, which take
  arrays / `items`. This caused two deserialization errors before landing the right shape
  (tried `items: [{instrument_id, module_id, description}]` → `instrument_id` + `items` →
  finally fully flat). Worse, its sibling `set_instrument_description` takes
  `items: [{instrument_id, description}]`, so the two siblings are mutually inconsistent.
  Fix: either array-adapt `set_module_description` (e.g. `instrument_id` +
  `items: [{module_id, description}]`, or self-contained
  `items: [{instrument_id, module_id, description}]` to mirror `set_instrument_description`),
  or at minimum make the schema/description clearer so the shape is guessable on the first
  call. Surfaced 2026-06-30 while documenting the AudioScript wavefolder example patch.

- [ ] Tier 3: `compare_to_reference`, `compare_patterns`, `compare_patches`,
  `humanize_notes`, `generate_variation`, `analyze_track`, `get_mix_meters`.

### 4.2 Upgrade rmcp 1.8.0 → 2.0.0

- [ ] **Upgrade the MCP SDK to rmcp 2.0.0.** Major version bump (`Cargo.toml:105`), but the
  **JSON wire format is unchanged** — all breaks are at the Rust API level, and our usage is
  concentrated in a single file (`crates/synth_mcp/src/server.rs`). schemars stays compatible:
  rmcp 2.0 still depends on `schemars = "1.0"` (gated behind the `server` feature) and we derive
  `JsonSchema` directly with 1.2.1, so no schemars conflict. Migration guide: rust-sdk discussion
  #926. **Why bother (mostly hardening, no new features):** the **streamable-HTTP session-leak
  fix** (PR #934) directly affects our long-running server on `127.0.0.1:9850`; plus OAuth
  SSRF/spoofing fixes (#935/#937, less relevant for a local server) and `inputSchema`/`outputSchema`
  now stripped+validated (#856/#860, may trim junk fields from our 175+ tool schemas). There is no
  new macro/tool capability to leverage.

  **What breaks (all in `server.rs`), from PR #927 "align model types with MCP 2025-11-25 spec":**
    1. **Content/audio construction** (`preview_note`/`analyze_note`, ~L4619-4635):
       `Content`/`RawContent`/`Annotated<T>` collapse into a flattened **`ContentBlock`**. The imports
       `AnnotateAble, Annotated, RawAudioContent, RawContent` (L13-18) and the
       `RawContent::Audio(RawAudioContent{..}).no_annotation()` path must be rewritten.
    2. **Resource impls** (`list_resources`/`read_resource`, L4232-4373): `RawResource`,
       `RawResourceTemplate`, `ResourceContents`, `.no_annotation()` — same `Annotated` removal.
    3. **`#[non_exhaustive]`** on most wire structs: returns we *construct* with struct literals
       (`ListResourcesResult`, `ReadResourceResult`) may need a builder / `..Default::default()`;
       params we only *receive* (`PaginatedRequestParams`, `ReadResourceRequestParams`) are fine.
    4. Renames (old kept as `#[deprecated]` aliases where practical): `ResourceReference`→
       `ResourceTemplateReference`, `PrimitiveSchema`→`PrimitiveSchemaDefinition`, etc.

  **What does NOT break:** the macro trio `#[tool]`/`#[tool_router]`/`#[tool_handler]` (API unchanged
  in 2.0 — only an internal fully-qualified-syntax fix), so our **190 tool methods + `Parameters<T>`
  compile as-is**; the manual `ToolCallContext`/`ToolRouter::call` dispatch in the overridden
  `call_tool`; and the `transport::streamable_http_server::*` path. Mechanical migration, no logic
  change — best done on a branch driven by the compiler + the bump-checklist build gate.

---

## 5. Architectural & Performance Hardening (Evaluation Needed)

### 5.1 Real-time safety: metadata deallocation on the audio thread
- [ ] **Evaluate removing/disposing metadata outside the audio thread.** Some commands like `RenameInstrument` or `SetInstrumentDescription` pass `String` variables by value to the audio thread. When they are dropped, heap deallocation occurs. Evaluate whether metadata should be completely separated from the audio engine's internal structs (e.g. kept only in the GUI state / shared graph, leaving the audio thread to manage purely numeric IDs), or if old metadata must be sent back to the UI thread via a return queue for deallocation.

### 5.2 Compiler optimizations: target-cpu native flag
- [ ] **Evaluate building with `target-cpu=native`.** The current `.cargo/config.toml` only specifies `rustflags = ["-D", "warnings"]`. For locally compiled builds, evaluate enabling `-C target-cpu=native` to allow LLVM to generate SIMD instructions (like AVX2, FMA, etc.) on the target hardware. This can significantly speed up DSP loops for oscillators, filters, and mixers.

### 5.3 Zero-copy optimization: bytemuck/zerocopy integration
- [ ] **Evaluate using `bytemuck` or `zerocopy` for buffer conversions.** For high-performance transfers of audio buffers (e.g., telemetry spectrum/oscilloscope buffers, OSC packet serialization), evaluate integrating safe zero-copy casting crates to cast slices safely (e.g. `&[f32]` to `&[u8]`) without using manual `unsafe` code or copying elements.

### 5.4 Invariant checking: debug_assert in new_unchecked constructors
- [ ] **Evaluate adding `debug_assert!` inside `new_unchecked` newtype constructors.** Performance-critical constructors like `NormalizedValue::new_unchecked(value)` bypass bounds checks. Adding `debug_assert!` checks (e.g. validating `[0.0, 1.0]` bounds) will catch invalid states during testing and debug execution, compiling to zero-cost no-ops in release builds.

### 5.5 Math optimizations: explicit Fused Multiply-Add (mul_add)
- [ ] **Evaluate utilizing `f32::mul_add` for hot DSP filter/envelope paths.** In filter calculations (like those in `synth_dsp/src/filters.rs`), replacing `(a * b) + c` structures with explicit `a.mul_add(b, c)` guarantees the compiler uses Fused Multiply-Add (FMA) instructions, improving precision and performance on supported hardware (AVX2/NEON).

### 5.6 Thread diagnostics: named background threads
- [ ] **Evaluate using named threads via `std::thread::Builder` for background tasks.** Background threads spawned in `null_backend.rs`, `gui/analyze.rs`, `synth_osc/src/lib.rs`, and `main.rs` are currently unnamed. Transitioning to named threads will make debugging/profiling (using `htop`, `perf`, or `gdb`) much more readable.

### 5.7 Real-time safety: replace HashMap usage on the audio thread
- [ ] **Evaluate removing `HashMap` lookups/updates in the audio thread.** Structures like `last_automation_values`, `track_auto`, `prev_instrument_outputs`, and `track_controls` are managed via standard `std::collections::HashMap` in `synth_engine.rs` and `sequencer_engine.rs`. Since standard `HashMap` uses the relatively slow `SipHash 1-3` and has a worst-case complexity of $O(n)$ under collisions, it can introduce latency jitter. Evaluate replacing them with flat arrays (`[Option<T>; MAX]`) or linear search over small stack-allocated arrays (e.g. `arrayvec`), which are cache-friendly and have a deterministic $O(n)$ WCET bound.

### 5.8 Real-time safety: automated allocation testing with assert-no-alloc
- [ ] **Evaluate integrating `assert-no-alloc` or similar custom allocator guards in tests.** To prevent regression-based heap allocations/deallocations from slipping into the real-time audio thread (such as in `SynthEngine::process()`), evaluate wrapping the audio thread processing tests in a custom allocator tracker. This will fail the test suite immediately if a heap allocation/deallocation occurs in a designated real-time path.

### 5.9 Compile-time safety: static assertions for lock-free data structures
- [ ] **Evaluate using `static_assertions` to verify layouts/bounds of thread-transferred data.** To ensure command, event, and telemetry structs sent over lock-free ring buffers (like `EngineCommand` and `EngineEvent`) remain layout-stable, lock-free, and thread-safe, evaluate using compile-time static asserts (e.g., verifying `Send` bounds or struct sizes).

### 5.10 Timing precision: sub-sample event & MIDI scheduling
- [ ] **Evaluate implementing sub-sample event scheduling.** Currently, all commands and MIDI notes are processed via `process_commands()` at the start of each buffer block (block-rate quantization). For large buffer sizes (e.g. 512 or 1024 samples), this creates audible timing jitter and trigger latency. Evaluate adding sample-offset timestamps to `EngineCommand` and splitting buffer processing into sub-blocks at event offsets to provide microsecond-accurate note and parameter triggers.

### 5.11 Performance optimization: memory alignment for SIMD vectorization
- [ ] **Evaluate enforcing 32-byte (AVX2) or 64-byte (AVX-512) alignment for audio buffers.** The `AudioBuffer` uses a standard `Vec<f32>` which lacks strict alignment guarantees, forcing LLVM to generate unaligned memory loads (`vmovups`). Restructuring it to use custom-aligned allocators or crates like `aligned-vec` will ensure aligned vector operations (`vmovaps`), boosting auto-vectorization efficiency in filter and oscillator hot paths.

### 5.12 Jitter reduction: elevate priority of MIDI threads
- [ ] **Evaluate elevating scheduling priorities for MIDI threads.** The MIDI threads spawned by `midir` currently run with normal OS priority, unlike the high-priority (`SCHED_FIFO`) audio callback thread promoted by `cpal`. Under high system CPU load, this introduces MIDI processing jitter. Evaluate using OS-specific bindings (e.g. `pthread_setschedparam` or `libc` calls) to bump the MIDI thread priority just below the audio thread.

### 5.13 Performance: optimize fast_sin_turns by avoiding redundant floor
- [ ] **Evaluate introducing `fast_sin_turns_unchecked`.** The `fast_sin_turns` function currently calls `.floor()` to wrap values. However, in many hot paths (such as the sine oscillator), the input phase is already guaranteed to be in `[0.0, 1.0)`. Evaluate adding an unchecked variant `fast_sin_turns_unchecked` that assumes pre-wrapped input (backed by a debug assertion) to eliminate redundant floating-point `.floor()` instructions.

### 5.14 Reproducibility & Real-time safety: replace fastrand on the audio thread
- [ ] **Evaluate replacing `fastrand` usage in the audio thread.** While `synth_core/src/hash.rs` notes that the audio path should never call an RNG to preserve reproducibility and avoid thread-local storage (TLS) lookup overhead, several modules (like `noise.rs`, `drift_generator.rs`, and `oscillator.rs`) call `fastrand::f32()`. Evaluate refactoring these modules to use the deterministic SplitMix64-based helpers in `synth_core::hash`.

### 5.15 Architectural: decouple SPSC channels in VisualizationBuffer
- [ ] **Evaluate decoupling the SPSC channel ends in `VisualizationBuffer`.** Currently, both `Producer` and `Consumer` halves are stored in the shared `VisualizationBuffer` struct, requiring `parking_lot::Mutex` wrappers and `try_lock()` logic to satisfy `Sync`. Evaluate separating the channels: storing `Producer` exclusively inside the audio thread's processors and `Consumer` inside the GUI widgets. This removes the mutex and `try_lock()` overhead entirely, achieving true lock-free synchronization.

### 5.16 User experience: custom panic hook for desktop crash diagnostics
- [ ] **Evaluate implementing a custom panic hook.** When a panic occurs, the desktop app prints a stack trace to stderr and terminates. Evaluate setting a custom panic hook (`std::panic::set_hook`) to display a user-friendly crash dialog or dump diagnostics to a log file, improving the supportability of the desktop app.

### 5.17 Performance: use dashmap instead of RwLock<HashMap> for concurrent maps
- [ ] **Evaluate replacing `RwLock<HashMap>` with `dashmap`.** Shared maps in `hub.rs` and `shared_state.rs` (like `clients` and `modules`) are wrapped in a global `RwLock`. Evaluate using `dashmap` to allow concurrent, sharded reads and writes, reducing lock contention between UI and background/MCP threads.

### 5.18 Architectural: eliminate multi-writer Mutex on CommandSender via multiple SPSC queues
- [ ] **Evaluate replacing the single `Mutex`-wrapped command queue with multiple SPSC queues.** Currently, `CommandSender` wraps a single SPSC `Producer` in a `Mutex` to allow multiple threads (UI, MIDI callback, MCP) to send commands. However, locking this mutex inside the high-priority MIDI callback thread can introduce timing jitter and blocking when other threads are writing. Evaluate creating separate, dedicated SPSC queues for the UI, MIDI, and MCP threads, allowing the audio thread to drain them sequentially without locks.

### 5.19 Performance: memory-mapped sample loading via memmap2
- [ ] **Evaluate using memory-mapped files (`mmap`) for sample loading.** Currently, the `SampleLibrary` loads sample files entirely into heap-allocated `Vec<f32>` buffers during startup. For large sample libraries, this causes high memory consumption and loading delays. Evaluate integrating `memmap2` to memory-map the sample files, letting the OS load sample data on demand (page-on-demand) and share page cache memory.

### 5.20 Audio compression: support compressed sample formats (FLAC/Ogg Vorbis)
- [ ] **Evaluate adding support for compressed audio files.** Currently, only uncompressed `.wav` files are supported. For multisampled instruments, this demands large disk spaces. Evaluate adding support for FLAC or Ogg Vorbis (using lightweight, safe libraries like `claxon` or `lewton`) to compress project size and save I/O bandwidth during project load.

### 5.21 Math optimization: factor out Hadamard scale in FDN reverb
- [ ] **Evaluate factoring out the Hadamard scaling factor in FDN.** Currently, `FdnCore::process_sample` performs 64 multiplications by `HADAMARD_8[i][j]` (containing `+HADAMARD_SCALE` or `-HADAMARD_SCALE`). By factoring out `HADAMARD_SCALE` and performing the matrix mixing using only additions and subtractions of the input values, and then applying a single multiplication per channel at the end, we can reduce multiplications from 64 to 8 per sample.

### 5.22 DSP optimization: polyphase decimation in oversampling half-band FIR
- [ ] **Evaluate implementing polyphase decimation in `HalfBandFilter`.** Currently, `HalfBandFilter::decimate` calls `self.push()` twice for every pair of input samples, discarding the first output. Although the delay line state must be updated for all samples, calculating the FIR filter sum for the discarded sample is redundant. Evaluate splitting `push` into `push_state_only` (shifts delay line only) and `push` (shifts and calculates output) to reduce FIR filter arithmetic by 50% during decimation.

### 5.23 Performance optimization: power-of-two circular buffer wrapping
- [ ] **Evaluate enforcing power-of-two sizes for circular buffers.** Currently, circular buffer index wrapping in `BufferIndex` uses the modulo operator `%`, which translates to a slow division instruction on the CPU. Evaluate enforcing power-of-two buffer sizes in delay lines and using bitwise AND `index & (size - 1)` for wrapping to replace division with a single-cycle bitwise operation, speeding up delay lines and reverbs.

### 5.24 Safety: mark new_unchecked newtype constructors as unsafe
- [ ] **Evaluate marking `new_unchecked` constructors as `unsafe`.** Constructors like `NormalizedValue::new_unchecked(value)` bypass safety invariants but are marked as safe functions. In accordance with Rust safety conventions, evaluate marking them as `unsafe fn` to ensure that call sites must explicitly enclose them in `unsafe` blocks, documenting and guaranteeing that developers have verified the bounds/invariants.

### 5.25 Architectural: RCU / arc-swap to eliminate RwLock<Song> read locks on the audio thread
- [ ] **Evaluate replacing `RwLock<Song>` with an RCU/double-buffering pattern.** The audio thread uses `try_read()` on an `Arc<RwLock<Song>>` to access sequencer state. If the UI thread acquires a write lock (e.g. during a large project mutation), `try_read()` fails, causing the audio thread to skip blocks or play silence. Evaluate using RCU (Read-Copy-Update) pointers (like the `arc-swap` crate) or double-buffered pointer swaps to provide lock-free, contention-free reads on the audio thread.

### 5.26 Architectural: feature flags to prune unused modules in synth_modules
- [ ] **Evaluate adding Cargo feature flags to prune unused DSP modules.** The `synth_modules` crate contains 70 modules, which increases compilation times and binary sizes. Evaluate dividing these modules into categories (e.g. `reverb`, `spectral`, `oscillators`) gated behind Cargo features to allow lightweight builds when only a subset of DSP code is needed.

### 5.27 Performance: flatten WavetableBank Vec<Vec<f32>> to contiguous vector
- [ ] **Evaluate flattening `WavetableBank::frames` structure.** Currently, wavetable frames are stored as a nested `Vec<Vec<f32>>`, causing double indirection and poor CPU cache locality during sample scanning. Evaluate flattening this structure into a single contiguous `Vec<f32>` (of size `num_frames * FRAME_SIZE`) and indexing it mathematically (`frame_idx * FRAME_SIZE + sample_idx`) to improve cache line pre-fetching.

### 5.28 Performance: replace rem_euclid with fast wrapping in Wavetable lookup
- [ ] **Evaluate avoiding float `rem_euclid` in wavetable sampling.** In `WavetableBank::sample`, the `phase` is wrapped via `phase.rem_euclid(1.0)` which is slow on floats. Since `phase` is often generated by wrapping phase accumulators (already in `[0.0, 1.0)`), evaluate using a fast branch or assuming pre-wrapped inputs (backed by a debug assertion) to bypass `rem_euclid` in the lookup path.

### 5.29 Architectural: share TuningTable via Arc across voices
- [ ] **Evaluate wrapping `TuningTable` in an `Arc`.** Currently, each `Voice` owns a copy of the entire `TuningTable` struct (512 bytes). In 64-voice polyphony configs, this wastes memory and requires copying the entire table to every voice when the scale changes. Evaluate sharing a single `Arc<TuningTable>` across all voices to achieve zero-copy tuning updates.

### 5.30 Build optimization: Profile-Guided Optimization (PGO)
- [ ] **Evaluate documenting and supporting Profile-Guided Optimization.** Since Pertylizer is a performance-critical audio engine, evaluate setting up and documenting PGO compiler workflows (using `-C profile-generate` and `-C profile-use` flags) to allow LLVM to optimize hot branch layouts and inline placements based on real-world execution profiles.

### 5.31 Performance optimization: pre-calculate reciprocals to avoid float divisions in hot paths
- [ ] **Evaluate replacing float divisions with reciprocal multiplications.** The engine performs divisions like `delta_time / glide_time` during processing (e.g. in `GlideState::update`). In DSP hot paths, floating-point division is several times slower than multiplication. Evaluate calculating the reciprocal (`1.0 / value`) during parameter configuration and using multiplication in the real-time processing loop.

### 5.32 Performance: optimize InterpolatedDelayLine read_cubic using power-of-two mask
- [ ] **Evaluate replacing rem_euclid and modulo operations in `read_cubic`.** Currently, `read_cubic` performs a floating-point `rem_euclid`, one `floor`, and four integer `%` modulo operations per sample. By enforcing power-of-two buffer sizes, evaluate replacing the four integer modulos with bitwise AND operations (`idx & (len - 1)`) and the float `rem_euclid` with a branch or bitwise operations to speed up chorus/pitch-shifting.

### 5.33 Performance: avoid double-precision f64 math in room mode calculations
- [ ] **Evaluate replacing `f64` casting in `mode_frequency`.** In `room_modes.rs`, `mode_frequency` converts all `f32` parameters to `f64` to calculate frequencies (performing `f64::sqrt` and double-precision divisions). Since the inputs and outputs are all `f32`, evaluate using `f32` operations to avoid double-precision instruction overhead on 32-bit registers.

### 5.34 Performance: pre-multiply WOLA norm in STFT synthesis window
- [ ] **Evaluate caching normalized window values in `StftProcessor`.** In `StftProcessor::process`, the synthesis overlap-add loop performs two multiplications per sample (`ifft_out[j] * window[j] * norm`). By pre-multiplying `window[j] * norm` when the window is loaded, evaluate reducing the inner loop math to a single multiplication per sample.

### 5.35 Performance: optimize WavetableBank linear interpolation math
- [ ] **Evaluate optimizing lerp calculation in `WavetableBank::sample`.** The current linear interpolation uses `a * (1.0 - t) + b * t` which requires two float multiplications. Evaluate replacing it with `a + t * (b - a)` to reduce it to a single multiplication, saving up to 50% of the math inside the scanning hot path.

### 5.36 Performance: utilize SIMD in StereoBiquad to process channels in parallel
- [ ] **Evaluate using SIMD for stereo biquad filtering.** Currently, `StereoBiquad::process` processes left and right channels sequentially. Since both channels share filter coefficients, evaluate using SIMD vectors (`f32x2` or `f32x4` via `std::simd`) to process both channels concurrently, doubling the speed of biquad filters.

### 5.37 Code quality: standardise on to_radians and to_degrees intrinsics
- [ ] **Evaluate replacing custom degree-to-radian multipliers.** Degree-to-radian conversions are done by multiplying by `PI / 180.0` or similar literals. Standardizing on `f32::to_radians` is cleaner and utilizes optimal compile-time standard library intrinsics.

### 5.38 DSP: implement parameter smoothing for CV/cutoff changes
- [ ] **Evaluate adding parameter smoothing to hot paths.** Sudden jumps in block-by-block parameter updates can cause audible clicks and "zipper noise". Evaluate adding a lightweight parameter smoothing helper (e.g. 1-pole lowpass filter or a sample-rate linear ramp) inside oscillators, amplifiers, and filters to guarantee smooth transitions.

### 5.39 Performance: replace per-bank OnceLock with a global initialized lazy structure
- [ ] **Evaluate replacing separate `OnceLock` instances in `get_wavetable`.** Currently, lookup matches on separate static `OnceLock<WavetableBank>` variables, causing atomic synchronization checks on every call. Evaluate using a single global lazy structure (or pre-initialization step) to load all banks at startup and keep them in a contiguous static array, removing lookup overhead.

### 5.40 DSP: support fractional note interpolation in TuningTable
- [ ] **Evaluate implementing fractional MIDI note support for TuningTable.** The `TuningTable` maps integer MIDI notes (0-127) to frequencies. During pitch bend or microtonal slide execution, custom scales are bypassed or approximated using 12-TET scaling. Evaluate supporting fractional note lookup (`note_to_freq_fractional(note: f32)`) with interpolation between adjacent frequencies to preserve correct custom scale intervals during pitch sweeps.

### 5.41 DSP: prevent CPU denormal performance spikes via FTZ/DAZ or anti-denormal noise
- [ ] **Evaluate preventing CPU denormal exceptions in DSP filters.** Decaying signals in recursive filters (like biquad filters and comb filters) can reach extremely small subnormal/denormal ranges. This triggers CPU microcode exceptions that spike CPU load when instruments fade to silence. Evaluate setting Flush-to-Zero (FTZ) and Denormals-Are-Zero (DAZ) hardware flags on the audio thread or adding anti-denormal bias/noise (e.g. `1e-24`) to feedback states.

### 5.42 Performance: eliminate rem_euclid in AdditiveOsc phase accumulation
- [ ] **Evaluate replacing float `rem_euclid` in `AdditiveOsc::process`.** During phase accumulation, `rem_euclid(1.0)` is used to wrap phases. Since the phase increment is positive and small, evaluate replacing it with a simple conditional check `if new_phase >= 1.0 { new_phase -= 1.0; }` to avoid expensive float modulo divisions.

### 5.43 Performance: utilize SIMD in AdditiveOsc for parallel harmonic synthesis
- [ ] **Evaluate vectorizing additive synthesis.** The additive oscillator sums 32 harmonics per sample, representing a major CPU bottleneck. Evaluate using SIMD vectors (`f32x4` or `f32x8` via `std::simd`) to calculate phase updates and sine approximations for multiple harmonics concurrently to achieve a 3-4x speedup.

### 5.44 Performance: optimize VectorMixer by hoisting gain calculations when CV is unconnected
- [ ] **Evaluate hoisting bilinear gain calculations in `VectorMixer`.** Currently, `VectorMixer::process` calculates equal-power bilinear crossfade gains (using trigonometric/square root functions) per sample. When `x_cv` and `y_cv` ports are unconnected, XY coordinates are completely constant across the entire block. Evaluate checking for unconnected CV inputs and calculating the gains once per block outside the loop, saving significant CPU overhead.

### 5.45 Performance: control-rate (downsampled) CV evaluation for VectorMixer
- [ ] **Evaluate downsampling CV calculations in `VectorMixer`.** When CV modulations are connected, they change at control rate rather than audio rate. Evaluate computing equal-power gains at a downsampled rate (e.g., once every 16 samples) and linearly interpolating them between blocks to eliminate 90% of coordinate transform math.

### 5.46 Architectural: use AtomicCell or raw atomics instead of RwLock in shared parameter/modulation telemetry
- [ ] **Evaluate using lock-free atomics for parameter sharing.** Telemetry and state updates (like OSC, harmony analysis, and GUI updates) query parameters concurrently. Wrapping values in standard `RwLock` structures introduces locks and synchronization overhead. Evaluate using `crossbeam::atomic::AtomicCell` or direct atomic floats to allow lock-free, zero-overhead sharing of float parameter values between threads.
