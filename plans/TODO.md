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

- [x] **`set_transport_loop` is now saved with the project.** DONE (`aa02ea44`): `Song`
  gained a serialized `transport_loop: Option<LoopRegion { start, end, enabled }>` carrier;
  `build_project_from_engine` captures the engine loop off the `TransportState` mirror (RT-safe)
  and `apply_project` restores it via a `SetLoop` command (clearing any stale loop when a loaded
  project has none). Covers the GUI + MCP save/load paths; `enabled` persists as saved. Headless
  round-trip tests added; `project.schema.json` regenerated.

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

### 2.6 YAMS scripting follow-ups

- [ ] **Per-sample pitch binding for `note_hz` in AudioScript.** The `note_hz`
  context var is currently *block-constant* — resolved once per block by the
  voice (`ScriptCtx`), same as the oscillator's own `set_voice_pitch`. So an
  audio-rate `phasor(note_hz)` does not follow intra-block pitch bend / glide at
  per-sample resolution; fast portamento steps once per block. A future
  per-sample pitch binding (analogous to how the audio-in registers are injected
  each sample in `eval_block`, via `AudioBindings`) would make scripted
  oscillators track fast portamento faithfully. Small-to-medium; only matters for
  audible fast glides.
- [ ] **Generate the context-var lists from `CONTEXT_CATALOG` instead of hand
  maintenance.** The YAMS context vars now live in *seven* parallel places
  (`Context` enum, `context_from_name`, `CONTEXT_CATALOG`, `context_to_runtime`,
  `ScriptContext` + `resolve_script_input`, the patch-editor help popup in
  `gui/patch_editor/popups.rs`, and the table in `docs/yams.md`). The
  `every_context_var_declares_catalog_membership` test guards the
  catalog↔resolver↔enum triangle, but the GUI popup and `yams.md` are still
  hand-maintained prose and already drifted once (a stale `sr`). Generate the
  popup line (and ideally the docs table) from `CONTEXT_CATALOG` so there is one
  source of truth. Cleanup/aesthetics — no behaviour change.

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

**Done.** Header badges and MCP surfacing shipped in v0.289.0
(`get_mod_matrix_routings`, virtual `"matrix"` port on `list_modules`). The
script-source markers then shipped in full: `ModRole` was replaced by a
multi-kind `ModMarkers` set, and `PatchAnalysis` now extracts sources read *from
inside a script* — Mod Matrix slot expressions, `ScriptModule` (`"scr"`), and
`AudioScript` (`"asc"`) — not just scalar `slot_addrs`, each tagged with its
consumer kind (per-slot compile cache; disabled Mod Matrix slots emit nothing).
Three source kinds are distinguished by icon+colour: Mod Matrix `↗` purple,
Script `ƒx` teal, AudioScript `ƒx` yellow, plus the Mod Matrix destination `↙`
purple. Markers render on param labels/knobs, output-port corners (glyph inside
the fixed 20×20 box), the module footer badge, and the macro rail — each kind in
its **own fixed corner** (knobs push the glyph just outside the circle, grown
vertically inward so it clears the label), each glyph with its own hover tooltip.
Shipped alongside: GUI patch load/save now install/capture per-slot control
scripts (`patch_bridge::load_module` + `create_patch_from_editor`), which the GUI
paths had been silently dropping. No "what feeds what" tooltip yet — a possible
future refinement, but not tracked as open work.

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
- [x] **Unify inline name/description editors.** DONE (`5b42949b`): added
  `inline_editable_text` + `InlineEdit` in
  [controls.rs](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/widgets/controls.rs)
  (folds the focus-grab + lost-focus/Enter end-of-edit detection); the four inline
  pattern/track name and description editors in `piano_roll.rs` and `arrangement.rs`
  now call it, each keeping its own editing-state + commit policy.
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

- [x] **`compare_spectra`: guard the empty-bin / silence floor.** DONE on
  `feat/sid-fidelity` (2026-07-02): (1) log bins are clamped up to −80 dB
  (peak-normalised) before diffing and bins floored on BOTH sides are excluded
  from the RMS — the sparse-harmonic empty-bin inflation is gone; (2) both
  sources under an absolute −80 dBFS broadband RMS (`EnergyBands::total_rms`)
  compare as distance 0 with `floor_limited: true` (silence agrees with
  silence). The response now carries `floor_coverage` (fraction of informative
  bins) + `floor_limited` (no information at all).

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

- [x] **Use named threads via `std::thread::Builder` for background tasks.**
  DONE (`b775d290`): named the four long-lived background threads — `mcp-server`
  (main.rs), `osc-telemetry` (synth_osc), `null-audio` (null backend),
  `analyze-render` (GUI note analysis). `Builder::spawn`'s `io::Result` is handled
  without expect/unwrap (log-or-degrade / map to `StreamCreationFailed` / skip).

#### A2. Code quality: standardise on to_radians / to_degrees

- [x] **N/A — no such conversions exist.** Audited the whole workspace: there are
  **zero** `PI / 180.0`-style degree↔radian multipliers (and no `to_radians` /
  `to_degrees` calls). The M/S "rotation" maps a normalized value ×π (not
  degrees×π/180), and AWE angle math uses `atan2`/geometry in radians directly.
  Every remaining "degrees" hit is a comment, a graph in-degree, or a musical
  scale-degree. Nothing to change.

#### A3. Invariant checking: debug_assert in new_unchecked constructors

- [x] **Add `debug_assert!` inside `new_unchecked` newtype constructors.** DONE
  (`6194ecf5`): asserts on the documented bound for `NormalizedValue` [0,1],
  `BipolarValue` [-1,1], `Phase` [0,1), `VoiceCount` [1,128], `Velocity` [0,1]
  (bare-condition + static-str so they stay const-fn-safe; not `unsafe`). Surfaced
  and fixed a real invariant violation: `compare_spectra` stuffed signed
  candidate−target deltas into `NormalizedValue` — retyped those `SpectrumDistance`
  fields to `f32` (the MCP type was already `f32`).

#### A4. DSP: prevent CPU denormal spikes via FTZ/DAZ

- [x] **Prevent CPU denormal exceptions in DSP filters.** DONE. The real-time
  path was already covered by `DenormalGuard` (FTZ+DAZ via MXCSR on x86_64, FZ via
  FPCR on aarch64, RAII restore) installed at the top of the cpal output callback.
  Extended (`5b02e7f8`) to the offline `engine.process` loops —
  `arrangement_render::render_range`, `export::render_to_wav`, and the shared
  `OfflineNoteSession::render` (covers `preview_note`) — so offline renders match
  live playback at the denormal level and avoid the same slowdown.

#### A5. UX: custom panic hook for desktop crash diagnostics

- [ ] **Implement a custom panic hook.** Today a panic prints a stack trace to
  stderr and terminates. A custom hook (`std::panic::set_hook`) can show a
  user-friendly crash dialog or dump diagnostics to a log file, improving desktop
  supportability. Not perf — pure usability.

#### A6. Compile-time safety: static assertions for lock-free structs

- [x] **Use `static_assertions` to verify bounds of thread-transferred data.**
  DONE (`91921dfa`): `assert_impl_all!(EngineCommand: Send)` /
  `assert_impl_all!(EngineEvent: Send)` plus a const `'static` check, pinned at the
  enum definitions in `commands.rs` (added the `static_assertions` workspace dep).
  Ringbuf only checks `Send` deep in its generics, so this gives a clear failure
  site if a variant ever captures a non-`Send`/borrowed payload. Struct-**size**
  asserts were deliberately skipped — they churn on every field edit for no
  invariant gain.

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
