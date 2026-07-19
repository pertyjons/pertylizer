# TODO - Pertylizer

## 1. Sequencer & Arrangement

### 1.1 Section markers

- [ ] Verse, chorus, bridge labels in the arrangement

### 1.2 Pattern-loop presentation and controls

**Playback looping shipped:** placement positions now wrap through type-safe
`PatternTick::looping_at`; crossing notes keep their absolute NoteOff and retrigger on the
next pass, while automation restarts in pattern space each pass.

- [ ] **Mini-note visualization should mirror the loop.** `NoteMiniature.start_frac` is currently
  fraction-of-pattern-length. For loop-within semantics the rendering in
  `gui/sequencer/arrangement.rs` (mini-note loop, near the `inst_color_cache` use) should repeat the miniature
  across the placement's `effective_length / pattern.length` iterations, so the user sees what they hear.
- [ ] **Add a toggle on `PatternPlacement`** (`loop_mode: PlacementLoopMode { Clip, Repeat }`, default
  `Repeat` to match DAW expectations). Surface in the placement context menu and in the right-edge
  resize-grab tooltip so the user can choose per placement. Migration of older songs: default existing
  placements to `Clip` so behaviour is preserved, or `Repeat` if we accept a one-time semantic change.

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

### 2.4 YAMS scripting follow-ups

- [ ] **Per-sample pitch binding for `note_hz` in AudioScript.** The `note_hz`
  context var is currently *block-constant* — resolved once per block by the
  voice (`ScriptCtx`), same as the oscillator's own `set_voice_pitch`. So an
  audio-rate `phasor(note_hz)` does not follow intra-block pitch bend / glide at
  per-sample resolution; fast portamento steps once per block. A future
  per-sample pitch binding (analogous to how the audio-in registers are injected
  each sample in `eval_block`, via `AudioBindings`) would make scripted
  oscillators track fast portamento faithfully. Small-to-medium; only matters for
  audible fast glides.
  **Investigated 2026-07-03 (deferred until someone actually needs it):** the
  `AudioBindings`/`eval_block` half is trivial (add
  `note_hz: Option<ScriptRegister>`, a
  `bindings_for` arm, and a per-sample `set_source`). The real work is that
  **there is no per-sample pitch signal in the engine at all** — the whole voice
  pitch pipeline is block-rate: `glide.update` runs once per block
  (`instrument.rs:~1277`) and `process_audio` delivers a single scalar
  `set_voice_pitch(freq)` (`voice.rs:~973`). Pragmatic fix: have the voice expose
  this block's start→end frequency (remember the previous block's `note_hz`) via a
  small new module method, and **lerp per sample inside `eval_block`** — glide/bend
  are piecewise-linear, so a per-block linear ramp reconstructs the trajectory
  exactly. Bundle the per-sample inputs into a struct rather than growing
  `eval_block`'s already-`too_many_arguments` signature.

### 2.5 Script-exposed params follow-ups

The shipped `Script` module is a one-program **4 CV-in (`in1..in4`) / 4 CV-out
(`out1..out4`)** node. `Script` and `AudioScript` expose user-declared `param`
knobs through the GUI, Mod Matrix, automation, persistence, and cross-script reads.

- [ ] **In-app GUI eyeball (pending verification, not a bug).** The 4-in/4-out faceplate
  ports, the ƒx editor's live control-ports status, and the declared-knob rendering are
  wired + unit/integration-tested but never clicked through in the running app. Confirm: a
  `param drive = 0.5` shows a **Drive** knob that changes the sound; the ƒx popup lists the
  declared params; rewiring a cable into `in1` changes the read without editing the script;
  editing a live script to add a `param` makes the knob appear with no audio glitch.
- [ ] **Cross-script reads don't see automation overrides.** `resolve_param_source`
  (`voice.rs`) reads another script's knob (`scr-1.drive`) as its *stored base* value via
  `get_param` — the transient sequencer-automation override (and the per-block mod-offset,
  deliberately) are excluded. So one script reading another's *automated* knob sees the
  un-automated value. Minor v1 limitation; if it matters, route the read through the knob
  store's effective-minus-offset value. **S**.
- [ ] **Optional per-CV-port display labels (`in1 "rate"` / `out1 "pitch"`).** Deferred from
  plan §2.A — the `param` string label/tooltip shipped, but the cosmetic per-port faceplate
  labels did not; ports show bare `in1..in4` / `out1..out4`. Needs a small header-declaration
  grammar addition (a bare `in1 "label"` statement) + carrying the label onto the port
  descriptor (it must NOT change the port id, so no cable churn). **S–M**, purely cosmetic.
- [ ] **Built-in knob smoothing (`smooth()` / slew) for audio-rate params.** Plan §6: a
  declared `param` is block-constant, so under fast automation/mod it *steps* at each block
  boundary — audible on a steep `audio_script` knob (filter cutoff, gain). v1 leaves click-free
  knobs to user-side per-sample smoothing in the script (`s = s + (drive - s) * 0.005` via a
  `state` cell); a built-in `smooth(x, coeff)` helper (or a `param … smooth` modifier) would
  remove the boilerplate. Defer unless the manual one-pole proves too fiddly.
- [ ] **Unit keyword for `param` metadata.** v1 carries default + `[min,max]` + `"label"` +
  `"tooltip"`; the `ParameterUnit` (Hz/dB/…), bipolar-vs-unipolar, and response curve are
  deferred (default linear/unipolar). A later optional `param … unit hz` keyword maps a
  recognized token → the `ParameterUnit` enum, else `None`. **S**.

### 2.6 Per-oscillator glide (portamento)

The eight pitched oscillators have shipped with an opt-in `glide_time` parameter.

- [ ] **In-app eyeball (pending — not a bug).** Two oscillators in one voice: set
  `glide_time > 0` on one, confirm it audibly portamentos between notes while the
  other jumps; confirm the **Glide** knob renders and changes the sound; confirm
  no-glide patches sound unchanged and that pitch-bend/vibrato still track on a
  gliding oscillator.
- [ ] **Extend the opt-in param to the other pitch-tracking sources.** `voice_synth`,
  `vocal_tract`, `fof`, `ring_mod`, `padsynth`, `sampler` took the `VoicePitch`
  signature change but not the `glide_time` param (plan §5 "candidates"). Each is a
  small `OscGlide` adoption if wanted. **S each.**
- [ ] **(Optional) Make `glide_time` modulatable/automatable.** It's deliberately
  `.modulatable(false)` on every module because `OscGlide` doesn't read
  `ParamModOffsets`/automation overrides for it — marking it modulatable without
  that would be a silent-drop bug (`is_automatable()` also gates on `modulatable`).
  To let an LFO/automation sweep the glide time, have `OscGlide` consume the mod
  offset for `glide_time`. **S–M.**
- [ ] **(Optional) Per-note glide + stepped glissando, per-oscillator.** The
  per-note glide (tracker import, `GlideState::start_from`) and stepped/glissando
  voice glides stay voice-level; a per-osc glide is always continuous. Mirror them
  per-oscillator only if a use case appears.

### 2.7 Mod Grid follow-ups

Mod Grid has shipped across the data model, engine, GUI, MCP, persistence, and
offline rendering. Remaining refinements follow.

- [ ] **Optional: fuller live exit-gate walk-through.** Not blocking. For an
  exhaustive pass, verify `LFO → Track 2 volume` with
  no lane; a Track graph on two tracks; `LFO → Pitch` host-only; an Audio-tap duck;
  routing-removal returns to base; a seeded render matches live; `Macro → LFO.rate_cv`;
  a live `MidiCc`; sustain hold+release.
- [ ] **Cache the Module-target snapshot (7.1 refinement).** `egui_backend` rebuilds
  the per-instrument `module_target_groups` for **every** instrument every frame the
  Mod Grid view is open (a registry lock + descriptor clone each). Fine for a handful
  of instruments (egui repaints on demand), but cache it on a structure-generation
  bump — or build only for the target's selected instrument — for large projects. **S.**
- [ ] **Per-graph CPU attribution (7.6 refinement).** The header CPU is total across
  all running instances (reuses the per-stage timing). Per-graph cost needs separate
  instrumentation (time each `ModGridInstance` and map to its `ModGraphId`, exposed to
  the GUI). **M.**
- [ ] **Soft cap on Mod Grid assignments.** Once per-graph CPU attribution is available,
  use the measured instance cost to decide whether track-scoped graphs need a soft
  assignment limit. Warn in the GUI when the estimated aggregate cost exceeds the
  budget; do not impose a hard limit unless real projects demonstrate a need. **S–M.**
- [ ] **Per-voice decorrelation for RandomGates / TuringMachine (optional).**
  DriftGenerator now decorrelates per voice via `set_voice_index`; RandomGates and
  TuringMachine still run in lockstep across a patch's voices (they don't override it).
  Add `set_voice_index` to them too if per-voice variation is wanted — but it may be
  intentional for rhythmic modules (a consistent gate pattern), so decide per module. **S.**
- [ ] **Full sustain-pedal is CC64-specific.** The pedal path only handles CC64; other
  hold-type pedals (sostenuto CC66, soft CC67) are not modelled. Low priority. **S.**
- [ ] **CPU metering uses `Instant::now()` every audio callback (review finding 5).**
  The mod-grid stage timing follows the *pre-existing* per-stage pattern (voices /
  module-graph / master-fx already do 4 clock reads per callback). `Instant::now()`
  isn't guaranteed to be a syscall-free vDSO read on every platform, which the RT rules
  frown on, and the reads happen even when the CPU tooltip is hidden. Consider making the
  whole metering opt-in/off-by-default or sampling it sparsely — a cross-cutting change to
  the existing metering subsystem, not mod-grid-specific. **S–M.**

---

## 3. UI & Visual Polish

### 3.1 MSEG UI overhaul (problematic — needs review)

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

### 3.2 `ModuleParam` single-definition cleanup (MAYBE — aesthetics only, future)

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

### 3.3 Unified list-panel follow-ups (deferred from code review)

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

### 3.4 Shared widget helpers follow-ups (evaluating Phase 2 residual)

Residual after the shared-widget-helpers work landed — these are the remaining areas to polish the GUI helpers layer:

- [ ] **Global FileDialog memory across kinds.** Refactor `ensure_dialog`
  in `gui/dialogs.rs` to reuse a single global
  `FileDialog` instance across all kinds (Open/Save Patch, templates, etc.) rather than rebuilding it when
  `file_dialog_kind` changes. Update its `config_mut().file_filters` dynamically on every open. This enables directory
  memory and highlighting (`retain_selected_entry`) to survive switching between Open and Save actions.
- [ ] **Address inline toggle button variations.** Several inline toggle styles (e.g. M/S muting/soloing badges,
  custom-colored selections) still bypass `toggle_button_colored`. Create a flexible `toggle_badge` or
  `selectable_toggle` helper to cover these and keep sizes consistent (preventing drift).
- [ ] **Perform a visual eyeball check on normalized captions.** Verify that the normalized size shift (~9px to 10px
  `size_small`) for the 24 migrated `.small()` labels does not cause visual clipping or alignment issues in tight
  spaces (especially grid cells
  in `gui/sequencer/tracker.rs` and Vol/Pan knob rows in
  `gui/sequencer/arrangement.rs`).

### 3.5 Drop the vendored egui-0.35 forks once upstream ships 0.35

- [ ] **Replace the vendored `third_party/egui-remixicon` crate with the crates.io version once they publish an
  egui-0.35-compatible release.** The egui 0.34→0.35 upgrade was blocked because neither `egui-remixicon` nor
  `egui-file-dialog` had a 0.35 release at the time (egui 0.35 landed 2026-06-25).
    * Note: `egui-file-dialog` was already successfully upgraded to its official 0.35-native version `0.14.1` on
      crates.io and its fork was dropped.
    * For `egui-remixicon`: when upstream releases a 0.35 version, bump `egui-remixicon` in `Cargo.toml`, remove the
      `[patch.crates-io]` block and the `third_party/egui-remixicon` directory, and verify the build.
      Watch: https://github.com/get200/egui-remixicon

### 3.6 Review the mixer view layout

- [ ] **Give the mixer view (`gui/mixer_view.rs`) a proper layout pass.** The module-header
  consolidation (2026-07-01) shared `draw_module_header`'s right-alignment across the mixer, switched
  its strips to the shared `icon_button`, and sized channel strips / return columns off the
  `ModuleWidth` buckets (`Small` 192 / `Medium` 256) instead of hardcoded 108/200 — which fixed the
  header title/icon overlap but was a spot fix, not a considered layout. Still worth reviewing: overall
  strip proportions and spacing at the new widths, sends/pan/meter/fader arrangement inside a strip, the
  master strip, and how it all reads next to the patch editor. Vertical scrolling was just added
  (`ScrollArea::both`); confirm it behaves with tall strips and many channels.

---

## 4. Architectural & Performance Hardening

> **Ground rule: nothing here gets optimized before it is a *measured* problem.**
> Readable, well-structured code wins over speculative micro-optimization. The
> long catalogue of speculative cycle-shaving items (SIMD/alignment, `rem_euclid`
> tricks, mmap, dashmap, PGO, reciprocal-mult, etc.) was **removed on 2026-06-30**
> — it traded clarity for unmeasured gains. The remaining entries are genuine
> correctness/RT-safety issues, but architectural enough to be driven by an
> *actually observed symptom*, not done pre-emptively.

### Trigger-based hardening (do when the symptom appears)

#### Real-time safety: replace HashMap usage on the audio thread

- [ ] **Remove `HashMap` lookups/updates from the audio thread.**
  `last_automation_values`, `track_auto`, `prev_instrument_outputs`, and
  `track_controls` use `std::collections::HashMap` (SipHash 1-3, worst-case O(n) on
  collisions) in `synth_engine.rs` / `sequencer_engine.rs` — a latency-jitter risk.
  Evaluate flat arrays (`[Option<T>; MAX]`) or linear search over small stack
  arrays, which are cache-friendly with deterministic WCET, and can read more
  cleanly. **Trigger: when jitter is actually measured.**

#### DSP: parameter smoothing for CV/cutoff changes

- [ ] **Add parameter smoothing to hot paths.** Sudden block-by-block parameter
  jumps cause audible clicks / "zipper noise". A lightweight smoother (1-pole
  lowpass or sample-rate linear ramp) in oscillators, amplifiers, and filters
  guarantees smooth transitions. An **audible-quality** fix (effectively a small
  feature). **Trigger: when you hear clicks on cutoff/CV moves.**

#### Per-track pre-FX sends and metering for shared instruments

- [ ] **Add per-track accumulators for pre-FX sends and metering.** Track
  volume/pan/mute now resolves through a generation-marked `TrackId` table and is
  applied per voice before the shared instrument effect chain, so shared-instrument
  dry playback is track-correct. Pre-FX sends and meters still aggregate at the
  instrument channel; split those taps per track if a project needs independent
  routing or meters for tracks sharing one instrument. **Trigger: when such a
  project needs separate pre-FX sends or meters.**

---

## 5. Future features (harvested from retired plan docs)

> **Consolidated 2026-07-13.** The standalone design/status docs that used to live
> under `plans/` were folded in here. Their full text is recoverable from git
> history; only their remaining open work is captured below.

### 5.1 Deferred design work

- [ ] **Cable layering, gradients, and focus mode.** Render cables and flow
  particles transparently in front of module faceplates, colour cross-domain
  cables with a source→destination gradient, and dim cables unrelated to the
  current selection. Keep the existing orthogonal routing and telemetry model;
  this is a scoped visual pass touching `theme.rs`, `cable.rs`, and `wiring.rs`.

### 5.2 Note Grid — deferred earned-escalation

*(from `plans/note-grid.md`; Note Grid shipped + squash-merged to main 2026-07-13 @`65f12900`. Full plan in git history.)*

- [ ] **DAG (branch/merge) escalation** — relax the linear-stream validation; add
  `KeyZoneSplitter`/`VelocitySplitter`/`RoundRobin` + merge semantics
  (connection-sorted concat under the buffer cap). Data model is already
  graph-shaped (no serde change); the hard part is **held-pitch resolution through a
  *branched* upstream** (`expand_pitch` in `note_processor.rs` today assumes one
  linear upstream chain). Value is low (single terminal output, no cross-instrument
  routing) — **only if real usage shows branching is needed**, not scheduled.
- [ ] **Track scope** (`SequencerTrack::note_graph`) — graph over the merged stream
  of all placements on a track: tails across pattern boundaries + the future
  live-input path (Ableton/Bitwig model). Costs: cross-placement look-back source
  material + a freeze-semantics answer.
- [ ] **`NoteScriptGenerator`** (YAMS `note_event` + `emit`) — statement-only 1-to-N
  generation (`emit(pitch,vel,dur[,delay])`, `MAX_SCRIPT_EMITS=16`), purity via the
  same bounded look-back idiom as Delay/Ratchet. Real language-surface work
  (parser/compiler/VM/`yamsfmt`/docs); may need a `StreamOnset`-style anti-stall cap.
- [ ] **`MicrotonalTuner` note-graph module** — per-note detune field + event
  plumbing. Related to §2.2 (alternative tunings) but delivered as a Note Grid node.
- [ ] **Misc later**: per-reference overrides, per-scope `Vec` of graphs,
  cross-track routing, cable telemetry, per-node tracker taps.

### 5.3 SID oscillator — open fidelity follow-ups

*(full spec archived at `docs/sid-oscillator.md`; the `sid` module shipped to main @`d0d872f3`. Expert-review history in git.)*

- [ ] **Oversampled ring/sync bus (ring-mod HF fidelity).** Ring sideband
  *positions* are exact but broadband `compare_spectra` distance holds ~16.9 dB vs
  reSID — pinned to **host-rate ring-edge jitter (~22.7 µs)**: the neighbour's `msb`
  is read once per host sample, outside the oversample loop. Fix needs the source
  `sid` to expose its MSB at the 4× rate (or the sub-sample crossing fraction) — a
  cross-module `msb`-port contract change, out of scope for a local
  `sid_oscillator.rs` edit. The one-sided PolyBLEP fold-flip already shipped (keep
  it).
- [ ] **Golden reSID A/B acceptance re-run.** Re-run the §11 reSID matrix (the
  sid-analyzer harness) as the acceptance gate for the shipped option-C combine /
  ring / `DcBlock` changes.

### 5.4 AccessKit / egui-inspection — deferred

*(from `plans/accesskit-custom-widgets.md`; shipped to main @`c7372dae`, container-level exposure across all views. Full inventory in git history.)*

- [ ] **Per-element canvas drivability.** v1 exposed the big canvases (piano roll,
  tracker, arrangement, keyboard, sample) only at *container* level. Making
  individual notes / tracker cells / keys / clips clickable+queryable via MCP needs a
  per-element `ui.interact(sub_rect, …)` per view — a larger, view-specific effort.
- [ ] **Cables as AccessKit nodes.** Cables are pure paint (no `Response`); v1
  encodes topology on the port labels. A cable-as-node pass (via the `expose_painted`
  escape hatch + AccessKit relations) is optional follow-up.

### 5.5 Sampling & recording backlog

*(from `plans/sampling-plan.md`; feature shipped through v0.262.0. Full P1/P2/P3 backlog + RT-review notes in git history. Related: §2.1.)*

- [ ] **P2 — sample UX/DSP.** Draggable crop/loop handles + preview playback cursor;
  zero-crossing snap (match slope *sign*, not just proximity); loop crossfade
  (static/baked default, keep the original alongside; dynamic dual-read only if loop
  points are modulated); cubic-Hermite interpolation (+ oversample-in-RAM for *short*
  samples only); mini waveform in the Rack; sample-usage tracking; undo/redo for edits.
- [ ] **P3 — stretch.** Sinc resampling, mipmaps for pitch-up anti-alias, disk
  streaming for large files, multi-sample zones (pull the `SampleZone` data model
  earlier to avoid a voice/GUI rewrite), slicing, timestretch, granular
  `GrainSource::Sample`, audio track in the sequencer.

## Maybe later

### Graph-level feedback edges (mostly redundant with Script — only build for audio-rate/UX)

- [ ] **Graph-level feedback loops (allow cycles via a one-block delay).** Was
  `plans/graph-feedback-loops.md` (deleted 2026-07-13; full text in git history). The
  proposal: stop rejecting a cycle-closing cable, tag it `is_feedback`, exclude it from the
  topo sort, and read the source's *previous* block (z⁻ᵇˡᵒᶜᵏ). **Largely redundant** — the
  block-latency feedback it wants already exists for the script path: a `Script` (`scr`) or
  `AudioScript` (`asc`) reads any module output as an **address-based source**
  (`src fb = flt-1.out`), which resolves via `Voice::resolve_source` to that module's
  **previous block's** value (`voice.rs:1277`, `buf[0]`) and does **not** go through
  `validate_connection`/`would_create_cycle` (the cycle check only guards `Connection` cable
  edges). So `flt-1.out → osc-1.fm_amt` (the plan's exit-gate example) is expressible today
  as: `scr-1` reads `src fb = flt-1.out`, writes `out1 = fb * amount`, cable
  `scr-1.out1 → osc-1.fm_amt` (a forward edge, no cycle). `AudioScript` additionally gives
  **sample-accurate** in-module feedback via `state` cells — strictly better than one-block
  latency for loops that fit in one module. **Only build the graph-edge feature if we
  actually want** (a) **audio-buffer-rate** feedback wrapped around *existing* modules
  without reimplementing their DSP in script (the address source yields one control-rate
  scalar/block, not an audio buffer), or (b) the **turnkey "drag a back-edge → feedback
  cable" UX** with a visually distinct feedback arc. Otherwise
  document the script recipe as the supported way to do control/CV-rate feedback.
