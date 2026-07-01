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

The actual feature — **unison detune + spread controls** for the voice-allocator's
global `AllocationMode::Unison` — is **shipped**: detune end-to-end (`268441f9`,
allocator → command → snapshot → persistence w/ 10 ct backward-compat → GUI slider
greyed outside Unison mode → tests) and per-voice stereo spread (`eac9b020`). Only
the MCP surface remains:

- [ ] **Surface the whole allocator config via MCP.** `synth_mcp` exposes **no**
  allocator-config param at all — `allocation_mode`, `stealing_strategy`,
  `max_voices`, and `unison_detune`/`unison_spread` have neither a getter on
  `get_instrument_info` nor a setter. Add them as a set (not a lone piecemeal
  `unison_detune` path, which would be inconsistent scope-creep): read them on
  `get_instrument_info` and set them via `set_parameter`, with round-trip tests
  mirroring the `allocation_mode` ones in `project_load_snapshot.rs`.

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

Residual after the shared-widget-helpers work landed — these are the remaining areas to polish the GUI helpers layer:

- [ ] **Global FileDialog memory across kinds.** Refactor `ensure_dialog` in [dialogs.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/dialogs.rs) to reuse a single global `FileDialog` instance across all kinds (Open/Save Patch, templates, etc.) rather than rebuilding it when `file_dialog_kind` changes. Update its `config_mut().file_filters` dynamically on every open. This enables directory memory and highlighting (`retain_selected_entry`) to survive switching between Open and Save actions.
- [ ] **Unify inline name/description editors.** Create a helper `inline_editable_text(ui, &mut String, &mut bool, multiline)` in [controls.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/widgets/controls.rs) to wrap the focus-grabbing and lost-focus/Enter-key commit logic currently duplicated in [piano_roll.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/piano_roll.rs#L2510) and [arrangement.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/arrangement.rs#L1580).
- [ ] **Address inline toggle button variations.** Several inline toggle styles (e.g. M/S muting/soloing badges, custom-colored selections) still bypass `toggle_button_colored`. Create a flexible `toggle_badge` or `selectable_toggle` helper to cover these and keep sizes consistent (preventing drift).
- [ ] **Perform a visual eyeball check on normalized captions.** Verify that the normalized size shift (~9px to 10px `size_small`) for the 24 migrated `.small()` labels does not cause visual clipping or alignment issues in tight spaces (especially grid cells in [tracker.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/tracker.rs) and Vol/Pan knob rows in [arrangement.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/arrangement.rs)).

### 3.9 Drop the vendored egui-0.35 forks once upstream ships 0.35

- [ ] **Replace the vendored `third_party/egui-remixicon` crate with the crates.io version once they publish an egui-0.35-compatible release.** The egui 0.34→0.35 upgrade was blocked because neither `egui-remixicon` nor `egui-file-dialog` had a 0.35 release at the time (egui 0.35 landed 2026-06-25).
  * Note: `egui-file-dialog` was already successfully upgraded to its official 0.35-native version `0.14.1` on crates.io and its fork was dropped.
  * For `egui-remixicon`: when upstream releases a 0.35 version, bump `egui-remixicon` in `Cargo.toml`, remove the `[patch.crates-io]` block and the `third_party/egui-remixicon` directory, and verify the build. Watch: https://github.com/get200/egui-remixicon

### 3.10 Review the mixer view layout

- [ ] **Give the mixer view (`gui/mixer_view.rs`) a proper layout pass.** The module-header
  consolidation (2026-07-01) shared `draw_module_header`'s right-alignment across the mixer, switched
  its strips to the shared `icon_button`, and sized channel strips / return columns off the
  `ModuleWidth` buckets (`Small` 192 / `Medium` 256) instead of hardcoded 108/200 — which fixed the
  header title/icon overlap but was a spot fix, not a considered layout. Still worth reviewing: overall
  strip proportions and spacing at the new widths, sends/pan/meter/fader arrangement inside a strip, the
  master strip, and how it all reads next to the patch editor. Vertical scrolling was just added
  (`ScrollArea::both`); confirm it behaves with tall strips and many channels.

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

- [ ] **`list_module_types` response is too large to consume.** The full catalog is ~275 KB
  / 10 311 lines (72 module types with every port + parameter), which blows past the tool-result
  token cap — it can't be read inline at all, only dumped to a file and `jq`'d. There's no
  lightweight variant. Fix: add a `brief`/`keys_only` mode (or make `search_modules` with no
  filter return a compact `[{type_key, name, category}]` list) so callers can get the catalog
  without the full port/parameter dump. Surfaced 2026-07-01 building the all-modules reference patch.

- [ ] **The module catalog doesn't expose the `ModuleWidth` bucket / panel width.**
  `list_module_types` entries have `type_key, name, description, category, input_ports,
  output_ports, parameters, signal_flow_hint` — but not the module's `ModuleWidth` (ExtraSmall…
  ExtraLarge) or its px width. That's GUI-layout metadata an agent can't see, which made a
  desired-vs-actual width audit impossible from MCP alone (had to grep `synth_modules` source).
  Fix: add `width_bucket` + `width_px` to each catalog entry. Surfaced 2026-07-01.

- [ ] **`add_module` can't add visualizer modules; no way to know in advance.** Adding
  `scp`/`mtr`/`spa` (Oscilloscope/Meter/Spectrum) fails with *"visualizer modules require GUI
  (VisualizationBuffer)"*, so "add every module type" is impossible from MCP. Nothing in the
  catalog flags which types are GUI-only, so the failure is only discoverable by trying. Fix:
  either let MCP create them with a headless/no-op buffer, or add a `gui_only: true` flag to the
  catalog entries so callers can filter them out up front. Surfaced 2026-07-01.

- [ ] **No MCP tool to save a single instrument as a patch file.** Only `save_project` exists
  (whole project → JSON); `load_project` *reads* single patch files, but there's no `save_patch`
  / `export_instrument` to write one. So an attempt to save one instrument to
  `assets/examples/patches/` produced a **project** file (had to be moved to
  `assets/examples/projects/` afterwards) rather than the single-instrument patch format the
  examples use. Fix: add a `save_patch(instrument_id, path)` that writes patch format.
  Surfaced 2026-07-01.

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

## 5. Architectural & Performance Hardening

> **Ground rule: nothing here gets optimized before it is a *measured* problem.**
> Readable, well-structured code wins over speculative micro-optimization. The
> long catalogue of speculative cycle-shaving items (SIMD/alignment, `rem_euclid`
> tricks, mmap, dashmap, PGO, reciprocal-mult, etc.) was **removed on 2026-06-30**
> — it traded clarity for unmeasured gains. What remains is split into two tiers:
>
> - **Tier A = cheap wins** that raise safety / readability / diagnostics or fix a
>   *known* bug. Do them whenever; no trigger needed. Ordered cheapest-first.
> - **Tier B = real problems that need a trigger.** Each is a genuine
>   correctness/RT-safety issue, but architectural enough that it should be driven
>   by an *actually observed symptom*, not done pre-emptively. Ordered by impact.

### Tier A — cheap quality/safety wins (do whenever, cheapest first)

These are not performance bets; they make the code safer, clearer, or more
debuggable at low cost.

#### A1. Thread diagnostics: named background threads
- [ ] **Use named threads via `std::thread::Builder` for background tasks.**
  Background threads spawned in `null_backend.rs`, `gui/analyze.rs`,
  `synth_osc/src/lib.rs`, and `main.rs` are currently unnamed. Named threads make
  debugging/profiling (`htop`, `perf`, `gdb`) much more readable. Trivial.

#### A2. Code quality: standardise on to_radians / to_degrees
- [ ] **Replace custom degree↔radian multipliers.** Conversions multiply by
  `PI / 180.0` literals; standardise on `f32::to_radians` / `to_degrees` — cleaner
  and uses the stdlib intrinsics. Pure readability cleanup. Trivial.

#### A3. Invariant checking: debug_assert in new_unchecked constructors
- [ ] **Add `debug_assert!` inside `new_unchecked` newtype constructors.**
  Constructors like `NormalizedValue::new_unchecked(value)` bypass bounds checks.
  A `debug_assert!` on the invariant (e.g. `[0.0, 1.0]`) catches invalid states in
  test/debug and compiles to a zero-cost no-op in release. This is the right tool
  for the "unchecked" constructors — note we deliberately do **not** mark them
  `unsafe` (that keyword is reserved for memory safety, not logical invariants).

#### A4. DSP: prevent CPU denormal spikes via FTZ/DAZ
- [ ] **Prevent CPU denormal exceptions in DSP filters.** Decaying signals in
  recursive filters (biquads, comb filters) reach subnormal ranges, triggering CPU
  microcode exceptions that spike load when instruments fade to silence. This is a
  **known, real** audio-engine bug, not speculation. Set Flush-to-Zero (FTZ) and
  Denormals-Are-Zero (DAZ) flags once on the audio thread (or add anti-denormal
  bias e.g. `1e-24` to feedback states). Cheap, high impact.

#### A5. UX: custom panic hook for desktop crash diagnostics
- [ ] **Implement a custom panic hook.** Today a panic prints a stack trace to
  stderr and terminates. A custom hook (`std::panic::set_hook`) can show a
  user-friendly crash dialog or dump diagnostics to a log file, improving desktop
  supportability. Not perf — pure usability.

#### A6. Compile-time safety: static assertions for lock-free structs
- [ ] **Use `static_assertions` to verify layouts/bounds of thread-transferred
  data.** Command, event, and telemetry structs sent over lock-free ring buffers
  (`EngineCommand`, `EngineEvent`) should stay layout-stable, `Send`, and
  lock-free. Compile-time static asserts (verifying `Send` bounds / struct sizes)
  document and lock those invariants at zero runtime cost.

#### A7. Real-time safety: automated allocation testing with assert-no-alloc
- [ ] **Integrate `assert-no-alloc` (or a custom allocator guard) in tests.** To
  stop heap alloc/dealloc regressions slipping into the real-time path
  (`SynthEngine::process()`), wrap the audio-thread processing tests in an
  allocator tracker that fails the suite immediately on any allocation in a
  designated RT path. Automates the RT rule the project already enforces by hand.

### Tier B — real problems, trigger-based (do when the symptom appears)

Principled correctness/RT-safety issues, not guesses — but each is architectural
enough to be driven by an actual observed symptom. Ordered by likely impact.

#### B1. Architectural: RCU / arc-swap to remove RwLock<Song> read locks on the audio thread
- [ ] **Replace `RwLock<Song>` with an RCU/double-buffering pattern.** The audio
  thread uses `try_read()` on `Arc<RwLock<Song>>`. When the UI takes a write lock
  (e.g. a large project mutation), `try_read()` fails and the audio thread skips
  blocks / plays silence — an **audible dropout during heavy editing**, not a perf
  nicety. Evaluate RCU pointers (`arc-swap`) or double-buffered pointer swaps for
  lock-free, contention-free reads. **Trigger: do it if you hear dropouts while
  editing big projects.**

#### B2. Reproducibility & RT safety: replace fastrand on the audio thread
- [ ] **Replace `fastrand` usage on the audio thread.** `synth_core/src/hash.rs`
  already states the audio path should never call an RNG — to keep renders
  *deterministic/reproducible* and avoid TLS lookups. Yet `noise.rs`,
  `drift_generator.rs`, and `oscillator.rs` call `fastrand::f32()`. Refactor them
  onto the deterministic SplitMix64 helpers in `synth_core::hash`. A
  correctness/reproducibility fix, not a speed bet.

#### B3. Real-time safety: metadata deallocation on the audio thread
- [ ] **Stop deallocating metadata on the audio thread.** Commands like
  `RenameInstrument` / `SetInstrumentDescription` move `String`s by value to the
  audio thread; dropping them heap-deallocates there — a direct violation of the
  project's own RT rules. Evaluate separating metadata from the audio engine's
  structs (keep it in GUI state / shared graph; the audio thread holds only numeric
  IDs), or return old metadata to the UI thread via a queue for disposal. Best
  folded into the next change to the instrument-state model.

#### B4. Real-time safety: replace HashMap usage on the audio thread
- [ ] **Remove `HashMap` lookups/updates from the audio thread.**
  `last_automation_values`, `track_auto`, `prev_instrument_outputs`, and
  `track_controls` use `std::collections::HashMap` (SipHash 1-3, worst-case O(n) on
  collisions) in `synth_engine.rs` / `sequencer_engine.rs` — a latency-jitter risk.
  Evaluate flat arrays (`[Option<T>; MAX]`) or linear search over small stack
  arrays, which are cache-friendly with deterministic WCET, and can read more
  cleanly. **Trigger: when jitter is actually measured.**

#### B5. DSP: parameter smoothing for CV/cutoff changes
- [ ] **Add parameter smoothing to hot paths.** Sudden block-by-block parameter
  jumps cause audible clicks / "zipper noise". A lightweight smoother (1-pole
  lowpass or sample-rate linear ramp) in oscillators, amplifiers, and filters
  guarantees smooth transitions. An **audible-quality** fix (effectively a small
  feature). **Trigger: when you hear clicks on cutoff/CV moves.**
