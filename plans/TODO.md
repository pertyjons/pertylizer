# TODO - Pertylizer

## 1. Sequencer & Arrangement

### 1.1 Tempo automation

**Done.** The **tempo map** (position-specific tempo + accelerando/ritardando ramps)
shipped in full: MCP tools (`set_tempo_at` / `remove_tempo_at` / `get_tempo_map`, each
with a `ramp` flag) + the map in `get_song_info`; ramp interpolation in `tempo_at` and
ramp-aware `tick_to_seconds` / `seconds_to_tick`; ramp-aware undo (`SetTempo` +
`MoveTempo`); and a draggable GUI tempo lane in the arrangement — curve + handles with
drag/add/remove, hover glow, a dynamic BPM axis (frozen during a drag), and the global
default drawn/labelled distinct from map points. (Not to be confused with the generic
`AutomationTarget::Global(Tempo)` lane, removed for good 2026-06-01 — a tempo-map point
can't be a per-block lane value; that dead code is not coming back.)

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

- [ ] **Reflect *script*-read sources in the patch-editor source markers, and distinguish
  Mod Matrix / Script / AudioScript sources by icon+colour.** Three modules read modulation
  sources *from inside a script* rather than through a cable or a scalar slot address, so those
  sources are currently **invisible** in the patch editor:
    1. **Mod Matrix** slots whose amount is overridden by a YAMS expression (`src lfo = lfo-1.out`,
       macros like `velocity`/`mod_wheel`) — no scalar slot *source* address exists.
    2. **`ScriptModule`** (`ModuleType::Script`, key `"script"`) — has **no input ports at all**;
       100 % of its inputs are in-script `SrcAddr` references, so nothing shows in the cable graph
       *or* the markers.
    3. **`AudioScript`** (`ModuleType::AudioScript`, key `"asc"`) — its per-sample audio comes via
       the `in_l`/`in_r` ports (cabled, already visible), but its **block-constant** modulation
       sources (macros, `lfo-1.out`, module params) are in-script references — invisible.

  Today `PatchAnalysis::from_panels` (`gui/patch_editor.rs`) only inspects `ModuleType::ModMatrix`
  panels and only their scalar `slot_addrs`; it never reads `slot_scripts`. So the S1.5a/b source
  glyphs, header badge, and macro rail stay dark for every script-read source.

  **What a script source can be (not just knobs).** `Voice::resolve_source` (`synth_engine/voice.rs`)
  resolves a `SrcAddr::Module{ module_type, instance, name }` to **any** of: an **output port**
  (`get_module_output`), **any parameter** by `type_id` (normalised through its descriptor
  range+curve — so sliders/dropdowns/toggles, not only knobs), the KineticModulator `vel`/`acc`
  pseudo-outputs, or a **macro** (`SrcAddr::Macro`). The markers must therefore cover every
  parameter widget type **and** output ports **and** the macro rail — not just knobs.

  **Extraction.** `slot_scripts` (`gui/module_panel.rs`) is already populated for *every* module
  snapshot (`sync_module_scripts`), so the data is present for ModMatrix, Script and AudioScript
  panels alike. For each script text, compile once and walk its bound inputs:
  `synth_script::compile(text, opts)` → `program.into_bound(text).inputs` (a `Vec<ScriptInput>`);
  `ScriptInput::Source(SrcAddr::Module{..})` → resolve to the source `ModuleId`+member (as the
  existing scalar slot-source path does), `ScriptInput::Source(SrcAddr::Macro(m))` → macro rail.
  The extraction machinery already half-exists as `script_refs_from_inputs` (`patch_editor.rs`), but
  it currently filters to `ModuleType::Script` *outputs* for the §3.5 cycle detector and discards
  macros + other module sources — generalise it to keep all module sources + macros.

  **Icon + colour scheme** (three distinct source kinds, two icons):

  | Source kind             | Icon                         | Colour                   |
  |-------------------------|------------------------------|--------------------------|
  | Mod Matrix              | `↗` `ARROW_RIGHT_UP_LINE`    | `accent_purple` (existing)|
  | Script (control-rate)   | `ƒx` `FUNCTION_LINE`         | `accent_cyan` (teal)     |
  | AudioScript (audio-rate)| `ƒx` `FUNCTION_LINE`         | `accent_yellow`          |

  Destination markers stay Mod-Matrix-only (`↙`, purple) — scripts write out via ports/cables,
  which are already visible. **No "what feeds what" tooltip** in this pass (deferred to a future
  item); the marker just says *which kinds* read this element.

  **Four rendering layers, all zero layout-stretch:**
    - **Footer status badge** (module level) — in `draw_module_footer` (`widgets/frame.rs`), show the
      glyph(s) for whichever kinds this module feeds (own row, no layout pressure).
    - **Param — slider/dropdown/waveform** — the glyph folds into the param *name label* via the
      existing `labeled_param` (`widgets/param_grid.rs`) `AtomLayout` trailing atom; already works
      for all non-knob widgets, so this is free once the source set is populated.
    - **Param — knob** — corner glyph on the knob via the existing `mod_marker` path.
    - **Output port** — a **corner glyph painted inside the port's fixed 20×20 box** (`widgets/port.rs`
      allocates `Vec2::splat(20.0)` but draws only an ~8 px-radius shape, so there is corner room —
      paint the glyph via `painter.text`, e.g. top-right, in the source-kind colour). **Zero width
      change**, consistent with the knob corner glyph, and it also disambiguates a script-fed output
      (which has no cable, so today renders as an empty/"dangling" `is_connected == false` dot).

  **Data-model change.** The current per-element role is a single `ModRole` (`Source`/`Destination`/
  `Both`) — it can express only one role. An element can now be several source kinds at once
  (Matrix **and** Script **and** AudioScript), so replace `Option<ModRole>` with a small **set** of
  markers rendered as up to a few adjacent mini-glyphs.

  **Caveats / open sub-decisions:**
    - **Don't compile per frame.** `from_panels` runs every frame; compiling every script each frame
      is wasteful. Cache the extracted source set per `(module_id, slot, script-text)` and invalidate
      when `slot_scripts` changes. This matters more now that many modules (each `ScriptModule` has up
      to 8 slots) can carry scripts, not just the Mod Matrix.
    - **Multiple glyphs in one corner.** When a knob or a port feeds more than one kind, decide how the
      mini-glyphs cluster in the limited corner space (small horizontal cluster vs. concentric on
      ports). Minor; settle during implementation.

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

- [ ] **Global FileDialog memory across kinds.** Refactor `ensure_dialog`
  in [dialogs.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/dialogs.rs) to reuse a single global
  `FileDialog` instance across all kinds (Open/Save Patch, templates, etc.) rather than rebuilding it when
  `file_dialog_kind` changes. Update its `config_mut().file_filters` dynamically on every open. This enables directory
  memory and highlighting (`retain_selected_entry`) to survive switching between Open and Save actions.
- [ ] **Unify inline name/description editors.** Create a helper
  `inline_editable_text(ui, &mut String, &mut bool, multiline)`
  in [controls.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/widgets/controls.rs) to wrap the
  focus-grabbing and lost-focus/Enter-key commit logic currently duplicated
  in [piano_roll.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/piano_roll.rs#L2510)
  and [arrangement.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/arrangement.rs#L1580).
- [ ] **Address inline toggle button variations.** Several inline toggle styles (e.g. M/S muting/soloing badges,
  custom-colored selections) still bypass `toggle_button_colored`. Create a flexible `toggle_badge` or
  `selectable_toggle` helper to cover these and keep sizes consistent (preventing drift).
- [ ] **Perform a visual eyeball check on normalized captions.** Verify that the normalized size shift (~9px to 10px
  `size_small`) for the 24 migrated `.small()` labels does not cause visual clipping or alignment issues in tight
  spaces (especially grid cells
  in [tracker.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/tracker.rs) and Vol/Pan knob
  rows in [arrangement.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/arrangement.rs)).

### 3.9 Drop the vendored egui-0.35 forks once upstream ships 0.35

- [ ] **Replace the vendored `third_party/egui-remixicon` crate with the crates.io version once they publish an
  egui-0.35-compatible release.** The egui 0.34→0.35 upgrade was blocked because neither `egui-remixicon` nor
  `egui-file-dialog` had a 0.35 release at the time (egui 0.35 landed 2026-06-25).
    * Note: `egui-file-dialog` was already successfully upgraded to its official 0.35-native version `0.14.1` on
      crates.io and its fork was dropped.
    * For `egui-remixicon`: when upstream releases a 0.35 version, bump `egui-remixicon` in `Cargo.toml`, remove the
      `[patch.crates-io]` block and the `third_party/egui-remixicon` directory, and verify the build.
      Watch: https://github.com/get200/egui-remixicon

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

_No open items._

---

## 5. Architectural & Performance Hardening

> **Ground rule: nothing here gets optimized before it is a *measured* problem.**
> Readable, well-structured code wins over speculative micro-optimization. The
> long catalogue of speculative cycle-shaving items (SIMD/alignment, `rem_euclid`
> tricks, mmap, dashmap, PGO, reciprocal-mult, etc.) was **removed on 2026-06-30**
> — it traded clarity for unmeasured gains. What remains is split into two tiers:
>
> - **Tier A = cheap wins** that raise safety / readability / diagnostics or fix a
    > *known* bug. Do them whenever; no trigger needed. Ordered cheapest-first.
> - **Tier B = real problems that need a trigger.** Each is a genuine
    > correctness/RT-safety issue, but architectural enough that it should be driven
    > by an *actually observed symptom*, not done pre-emptively. Ordered by impact.

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
