# TODO - Pertylizer

> Completed entries are removed rather than kept as `[x]` — the commit and
> `docs/history.md` carry that record. Section numbers are **not** renumbered when
> entries go, so gaps are expected and existing references (commit messages,
> notes) keep pointing at the right thing.

## 0. Project Safety & Core Workflow

These are high-priority correctness and trust items. A creative application must
not lose work, make destructive edits impossible to reverse, or let view-specific
input handling interfere with standard application shortcuts.

### 0.1 Reliable dirty-state propagation

- [ ] **Mark the project dirty after every successful project mutation.** Dirty
  tracking is currently driven by scattered `SynthApp::mark_dirty()` calls, while
  several editors mutate shared song/sample state without reporting the change to
  the application shell. Establish one reliable mutation signal or revision-based
  mechanism covering at least notes, patterns, placements, tempo/time signatures,
  automation, Note/Mod Grid graphs, mixer controls and routing, return/master FX,
  sample edits, rack/module edits, and instrument/project metadata.

  Loading a project and completing a successful save must establish the clean
  baseline; undo/redo must update dirty state relative to that baseline rather
  than blindly clearing it. Closing, opening, or creating a project after any
  mutation must consistently show the unsaved-changes prompt. Add focused tests
  for mutations originating in each major view so future editors cannot silently
  bypass the mechanism. **P0, M, correctness/data safety.**

### 0.2 Atomic save, autosave, and recovery

- [ ] **Make manual saves atomic for both plain projects and sample bundles.** Write
  the complete project to a uniquely named temporary file beside the destination,
  flush/sync it, and only then replace the destination. Preserve or restore the
  previous valid file if serialization, sample encoding, disk I/O, or the final
  replacement fails; never truncate the user's last good save before the new one
  is complete. Cover new files, overwrite saves, `.ptz` projects, bundled projects,
  and platform-specific replacement behaviour with failure-path tests.

- [ ] **Add debounced autosave and startup recovery without overwriting the manual
  project file.** Store recovery snapshots in a separate per-project location,
  write them atomically, and retain enough identity/timestamp information to offer
  recovery only when the snapshot is newer than the last manual save or follows an
  unclean shutdown. A recovered document must open as unsaved, successful manual
  saves should retire obsolete recovery data, and failed autosaves must be reported
  non-disruptively without clearing dirty state. Define retention/cleanup for
  abandoned and untitled projects so recovery storage remains bounded. **P0, L,
  data safety.**

### 0.3 Complete and consistent undo/redo

**Done.** Undo/redo now covers every editor:

- **Mixer** — track volume/pan/mute/solo, track sends (level, pre/post tap,
  enable), bus-to-bus sends, return-bus controls, return-bus create/delete
  (including its engine-side insert chain), and return/master effect add,
  remove, bypass and parameter edits.
- **Samples** — import, recording, delete, rename, description, root note, loop
  and crop regions, and the destructive normalize/reverse edits.
- **Instruments and rack** — instrument properties and performance settings
  (captured as a whole snapshot per editor), module parameters, and module
  add/remove with the cables attached to them.
- **Sequencer** — pattern create/duplicate, default time signature, and
  committing a recording take, which were the gaps the audit found.

Continuous gestures collapse into one entry two ways: `DragCoalescer` for
controls that expose a `Response` (faders, sliders, sample handles), and
time-windowed merging for module and effect parameters, which arrive as a plain
list with no gesture signal. Sample history shares `Arc` buffers and is bounded
by a 256 MiB ceiling on retained audio, evicting oldest-first but never the
newest entry. Undoing back to the save point reads clean again, via undo-stack
depth — a shortcut that disables itself for the session if any mutation is seen
that did not pass through the undo manager, so it can never produce a false
*clean*.

Remaining:

- [ ] **Effect-chain reordering is not undoable.** Add/remove/bypass/parameters
  are, and a restored effect returns to its original slot, but dragging an
  effect to a new slot records nothing. The engine command it needs now exists
  (`SetReturnEffectChainOrder`, added alongside the master one), so this is just
  capturing the order around the drag. **S.**
- [ ] **An undone effect addition loses parameter edits made after it.** The
  entry records a freshly-created effect with default parameters, so redoing an
  add restores defaults rather than the state the effect had when it was
  removed. Only affects add-then-edit-then-undo-then-redo. **S.**
- [ ] **In-app verification.** None of this has been clicked through in the
  running app — only tested headlessly. Worth a pass over each editor: change,
  undo, redo, and confirm both the display and the sound follow. This is the
  biggest remaining risk: the undo paths that write both a GUI mirror and the
  engine are exactly where a wrong ordering shows up only live.

### 0.4 Focus-safe shortcuts and global transport

- [ ] **Centralize shortcut routing and prevent the computer-keyboard piano from
  consuming text or command input.** Do not trigger piano notes, octave changes,
  editor actions, undo, or transport from ordinary typing in a focused text field;
  modifier chords such as Ctrl/Cmd+C, V, X, Z, S, O, and N must never also play
  notes. Handle focus loss and view changes without leaving stuck notes, and let
  modal dialogs take input priority.

  Provide consistent application-wide shortcuts for Save, Save As, New, Open,
  Undo/Redo, and play/stop, using the platform command modifier. Spacebar transport
  must work from every main view when no text field or modal owns it, rather than
  only from individual sequencer editors. Route commands through one dispatcher so
  menus can show the same bindings and views do not implement conflicting copies;
  add input tests for text focus, modifiers, modal focus, view switching, and global
  transport. **P0, M, correctness/UX.**

## 1. Sequencer & Arrangement

### 1.3 Automation targets for send/return routing

- [ ] **Expose track send levels and return-bus volume/mute as automatable
  targets.** Today the automation DSL covers instrument macros, module params,
  track mixer params (Volume/Pan/Mute/Pitch), and `global:MasterVolume`, but not a
  track's send level to a return bus, nor a return bus's own volume/mute. So
  transition-only effects (reverse-reverb swells, granular/spectral throws for a
  few bars) can't be automated — `set_track_send` only sets a static value, forcing
  a permanently-low constant send instead. Requested target DSL:
  `track:Send:<return_id>` (and the relative `track:Send:<return_id>` on the host
  track) plus `return:<return_id>:Volume` / `return:<return_id>:Mute`.

  This is a **real-time audio feature, not a quick fix** — deliberately deferred
  from the 2026-07-20 MCP feedback batch (items 1–7 shipped). Scope, in order:
  1. **`synth_sequencer`** — add `AutomationTarget` variants (e.g.
     `TrackSend { track: Option<TrackId>, return_bus: ReturnBusId }` and
     `Return { bus: ReturnBusId, param: ReturnParam { Volume, Mute } }`). They must
     stay `Eq`/`Hash` (lane-map keys) with serde + `display_name` + the
     `to_target_string` DSL round-trip.
  2. **`synth_engine` mixdown** — apply the lane values per block to the send
     matrix and return-bus gain/mute, RT-safe (no alloc/lock on the audio thread;
     mirror the pre-keyed slot pattern used by `mod_grid.track_offsets`).
  3. **Offline render parity** — `arrangement_render` must apply the same so
     analysis/export match live playback.
  4. **MCP** — `build_(live_)automation_target` DSL parse/format, structured
     target variant, boundary validation (return id exists), and **discovery** in
     `list_mod_targets`, `get_instrument_automation_targets`, and
     `list_automation_lanes`.
  5. **GUI (optional, separate)** — draw/edit these lanes in the arrangement view.

  A partial version (DSL + discovery without the engine application) is worse than
  nothing: it creates lanes that silently do nothing. Land 1–4 together. **L,
  feature.**

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
- [ ] **Per-graph CPU attribution (7.6 refinement).** The header CPU is total across
  all running instances (reuses the per-stage timing). Per-graph cost needs separate
  instrumentation (time each `ModGridInstance` and map to its `ModGraphId`, exposed to
  the GUI). **M.**
- [ ] **Soft cap on Mod Grid assignments.** Once per-graph CPU attribution is available,
  use the measured instance cost to decide whether track-scoped graphs need a soft
  assignment limit. Warn in the GUI when the estimated aggregate cost exceeds the
  budget; do not impose a hard limit unless real projects demonstrate a need. **S–M.**
- [ ] **Sostenuto (CC66) is unmodelled — re-scoped from "S" to "M".** The pedal
  path only handles CC64 (defer NoteOffs while held). **CC67 (soft) is NOT a code
  gap**: all CC state already feeds Mod Grid `MidiCc` sources, so a soft pedal is a
  routing answer today (CC67 → a volume/gain destination). **CC66 (sostenuto)** is a
  real feature but bigger than a CC64 mirror: it must snapshot the keys *down at the
  moment the pedal is pressed* and defer only those NoteOffs (notes struck after are
  not held), which needs new per-channel keys-down tracking + a captured set +
  selective deferral. Do as its own small-M task if a use case appears. Low priority.
- [ ] **CPU metering uses `Instant::now()` every audio callback (review finding 5).**
  The mod-grid stage timing follows the *pre-existing* per-stage pattern (voices /
  module-graph / master-fx already do 4 clock reads per callback). `Instant::now()`
  isn't guaranteed to be a syscall-free vDSO read on every platform, which the RT rules
  frown on, and the reads happen even when the CPU tooltip is hidden. Consider making the
  whole metering opt-in/off-by-default or sampling it sparsely — a cross-cutting change to
  the existing metering subsystem, not mod-grid-specific. **S–M.**

### 2.8 Port value domains — open follow-ups

`PortValueDomain` annotates every port with the value contract it accepts/produces
(`feat/port-value-domains`). Two ports were deliberately left alone during the
review of that branch because fixing them changes audible behaviour or needs a new
domain.

- [ ] **Turing Machine's pitch range is capped at 1 octave by its own storage type.**
  `turing_machine.rs` quantizes to semitones/12 and stores the result in a
  `NormalizedValue`, which clamps to `0..1` — so the `range` parameter's advertised
  "up to 2 octaves" can never exceed one. The port now *documents* the real 0–1
  octave range, but the parameter still lies. Fix by storing the pitch CV in a type
  that carries the full range (or scaling `range` into 0..1 and multiplying at the
  consumer). **Audible behaviour change** — patches relying on the current clamped
  output will transpose differently. **S.**
- [ ] **`math_oscillator`'s `fm` port has no matching domain.** It uses `apply_fm`
  (2 octaves per unit), which neither `Octaves` (1 octave/unit) nor `Control`
  describes, so it was left on the generic `control` domain while every other 1V/oct
  FM port got annotated. Either add a domain variant for the 2-octave scaling or
  change `math_oscillator` to the standard 1V/oct `apply_cv`. **S.**

---

## 3. UI & Visual Polish

### 3.5 Drop the vendored egui-0.35 forks once upstream ships 0.35

- [ ] **Replace the vendored `third_party/egui-remixicon` crate with the crates.io version once they publish an
  egui-0.35-compatible release.** The egui 0.34→0.35 upgrade was blocked because neither `egui-remixicon` nor
  `egui-file-dialog` had a 0.35 release at the time (egui 0.35 landed 2026-06-25).
    * Note: `egui-file-dialog` was already successfully upgraded to its official 0.35-native version `0.14.1` on
      crates.io and its fork was dropped.
    * For `egui-remixicon`: when upstream releases a 0.35 version, bump `egui-remixicon` in `Cargo.toml`, remove the
      `[patch.crates-io]` block and the `third_party/egui-remixicon` directory, and verify the build.
      Watch: https://github.com/get200/egui-remixicon
    * **egui 0.36 (released 2026-08-05) repeats the pattern**, with the roles swapped: this time
      `egui-file-dialog` is the blocker (0.14.1 is pinned to egui 0.35) while the remixicon fork
      only needs its egui dep bumped. After that bump, eyeball the piano-roll and arrangement
      pinned gutter/ruler strips (upstream #8367 now counts the panel separator line in the outer
      width) and knob/note dragging (#8365 treats a press that leaves a widget as a drag).

### 3.6 Review the mixer view layout

- [ ] **Give the mixer view (`gui/mixer_view.rs`) a proper layout pass.** The module-header
  consolidation (2026-07-01) shared `draw_module_header`'s right-alignment across the mixer, switched
  its strips to the shared `icon_button`, and sized channel strips / return columns off the
  `ModuleWidth` buckets (`Small` 192 / `Medium` 256) instead of hardcoded 108/200 — which fixed the
  header title/icon overlap but was a spot fix, not a considered layout. Still worth reviewing: overall
  strip proportions and spacing at the new widths, sends/pan/meter/fader arrangement inside a strip, the
  master strip, and how it all reads next to the patch editor. Vertical scrolling was just added
  (`ScrollArea::both`); confirm it behaves with tall strips and many channels.
- [ ] **Track-properties popup is clipped by the pattern panel** (`gui/sequencer/arrangement.rs`,
  seen 2026-07-26). The kebab popup on a track header opens downward and the lower half (Colour
  row) disappears behind the docked pattern editor. Flip or clamp it to the visible area.

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

## 6. MCP protocol capabilities

> Added 2026-08-05 after the `rmcp` 2.2 → 3.1 upgrade (MCP spec `2026-07-28`).
> Tool annotations from that audit have shipped; the rest is open. None of these
> are bug fixes — they are capabilities the protocol gained that we do not use.

### 6.7 Structured tool output (`outputSchema` + `structuredContent`)

- [ ] **Return typed results instead of a JSON string.** All 219 tools return
  `String` built by `to_json(...)`, so a client receives an opaque text blob it
  must parse blind, with no schema to validate against. rmcp's `Json<T>` wrapper
  (`IntoCallToolResult for Json<T> where T: Serialize + JsonSchema`) emits
  `structuredContent` and `#[tool]` can declare a matching `outputSchema`.
  Two obstacles, neither fatal:
    * The ~139 result structs in `synth_mcp/src/types.rs` derive `Serialize` only,
      not `JsonSchema`. Adding the derive is mechanical but touches every type.
    * Our in-band error convention (`format!("Error: {e}")`) has no place in a
      typed result — those would have to become real `is_error` results, which is
      exactly what `result_is_failure` and the batch rollback gate key off. Change
      that convention and the batch verdict logic must move with it.

  **Do this incrementally on the most-used tools, not as one sweep.** **L.**

### 6.8 Argument completions (`ServerHandler::complete`)

- [ ] **Implement `complete` for our string-keyed argument space.** Module type
  keys (`osc`, `flt`), parameter addresses (`flt-1.cutoff`), instrument names,
  pattern ids and automation target DSL strings are all free-text today; a typo
  costs a failed call plus a `search_modules`/`list_*` round trip to recover. The
  catalogs the completions would draw from already exist behind the discovery
  tools. **M**, and probably the biggest day-to-day ergonomics win left.

### 6.9 Long-running calls: tasks extension + progress

- [ ] **Return a task handle for offline renders instead of blocking.** The
  `analyze_*` family and `render_to_wav` render offline inside the call, so a long
  render is indistinguishable from a hang and can hit a client timeout. MCP
  `2026-07-28` moved tasks into the `io.modelcontextprotocol/tasks` extension:
  the server returns a task handle, the client polls `tasks/get`, and
  `notifications/progress` reports progress meanwhile. rmcp exposes the
  `get_task`/`update_task`/`cancel_task` handler hooks and our `call_tool`
  already passes the `Task` response variant through untouched. **M–L.**

### 6.10 Confirmation via MRTR / elicitation

- [ ] **Ask before irreversible calls.** Multi-round-trip requests let a tool
  return `InputRequiredResult` mid-call to ask the client for input, replacing the
  old server-initiated elicitation. The 50 tools now marked `destructiveHint`
  (`delete_*`, `clear_*`, `new_project`, `load_project`, `set_song`, …) are the
  natural candidates: confirm before discarding unsaved work. `call_tool` already
  passes `InputRequired` through. Weigh against the annotation hints, which may
  already give clients enough to prompt on their own. **M.**

### 6.11 Cacheable list results (`ttlMs` / `cacheScope`)

- [ ] **Advertise cache lifetimes on the static catalogs.** `2026-07-28` adds
  `ttlMs` + `cacheScope` to `tools/list`, `resources/list` and friends. Our module
  and port-type catalogs are immutable for the life of a build, so they can carry
  a long TTL and stop being re-fetched. Small win, small task. **S.**

### 6.12 Not applicable — deprecated features

  For the record, so nobody adds them later: the spec deprecates **Roots**,
  **Sampling** and **Logging** (12-month window). We use none of them, and our
  `tracing`-to-stderr logging is already the migration the spec recommends. The
  deterministic `tools/list` ordering it now asks for is also already satisfied
  (verified: identical order hash across runs).

---

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
