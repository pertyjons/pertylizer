# TODO - Pertylizer

> Completed entries are removed rather than kept as `[x]` — the commit and
> `docs/history.md` carry that record. Section numbers are **not** renumbered when
> entries go, so gaps are expected and existing references (commit messages,
> notes) keep pointing at the right thing.

## 0. Project Safety & Core Workflow

These are high-priority correctness and trust items. A creative application must
not lose work, make destructive edits impossible to reverse, or let view-specific
input handling interfere with standard application shortcuts.

**All five subsections are closed as of 2026-08-06.** The section is kept rather
than deleted: it is the design record for how dirty state, atomic saves,
recovery, undo coverage and input routing fit together, and each one names the
failure it exists to prevent. What was closed by decision instead of by
observation is called out in §0.5.

### 0.1 Reliable dirty-state propagation

**Done** (`c417e404`). Dirty state is *derived*, not reported: `SharedSong` and
`SampleLibrary` carry edit revisions, the engine graph version is reused, and the
patch canvas contributes a fingerprint of the layout the save path writes.
`SharedSong`'s counter keys off the write guard's `DerefMut`, so a guard that only
reads is not mistaken for an edit. Loading and a successful save establish the
clean baseline; undo back to the save point reads clean again via history
position, and that shortcut disables itself for the session if any mutation
bypasses the undo manager, so it can never report a false *clean*.
`tests/dirty_state_coverage.rs` asserts a mutation from each major view (piano
roll/tracker, arrangement, transport, Note Grid, Mod Grid, mixer, return buses,
rack, samples) reaches the signal, plus that reading never marks dirty.

**Global state escaped the counters — fixed 2026-08-06.** The in-app pass (§0.5)
found that persisted state outside the four counters could be changed with the
project still reporting itself clean: the master fader (0.80 → 0.38), the
keyboard octave, and adding a master effect all left the title reading `Untitled`
with no `*`. The last was the sharpest — the undo manager *had* recorded it, but
`is_dirty` returned at its opening counter check before the undo position was
ever read. `dirty.rs`'s own table was part of the cause: it claimed
`SharedGraph::version` covered "return/master effect chains", which it never did
(those are `RwLock`s on `EngineState`, a different struct).

The consequence was not a missing asterisk. Autosave (`autosave_flow.rs`) and the
close prompt are gated on the same predicate, so that work was neither
snapshotted for crash recovery nor asked about on close.

Two changes:

- **A `global` fingerprint** joins `layout` as a derived term, covering master
  volume, keyboard octave, glide, the transport loop region and the master /
  return-bus effect chains — everything `GlobalProjectState` and the loop mirror
  persist. Derived rather than reported for the same reason `layout` is: these
  are reached from the GUI, MCP, project load and undo, and none of them should
  have to remember. `dirty::tests::global` covers each term one mutation at a
  time, including chain reorder and knob edits.
- **The undo stack is consulted before the counters.** They are independent
  observers and either seeing a change is enough; making the counters a gate is
  what let a recorded edit report clean.

Taking a baseline now waits (bounded, 100 ms) for queued engine commands to be
applied first. Without that the fingerprint exposed a latent race: a project
reset sends the master volume to the audio thread, which applies it a block
later, so the baseline described a state the engine had not reached — a freshly
opened project read dirty for a frame, and `observe_untracked_mutation` would
latch `untracked_mutation_since_save` for the session, permanently disabling
undo-back-to-clean. Verified live afterwards: fader, octave and master-effect
edits each raise the `*`; New Project reads clean immediately; and undoing a
master-effect add returns the title to clean.

**An instrument's effect-chain order escaped the counters too — fixed
2026-08-06.** Found while closing the §0.3 reorder gap, and the same hole the
master fader had: `effect_chain_order` lives on
`EngineState::instrument_snapshots`, not on the `SharedGraphState` whose version
the `graph` counter reads, and the `global` fingerprint covers only the master
and return chains — yet the order *is* persisted
(`patch.settings.effect_chain_order`). Moving an effect up or down a chain
therefore changed the file that would be written while the project read clean,
so the work was neither autosaved nor asked about on close.

An `effect_order` fingerprint now joins `layout` and `global` as a derived term.
Its own row rather than a term inside `global`, which is defined as what
`GlobalProjectState` plus the loop mirror persist — quietly widening a row past
its documented contents is exactly how the effect chains went unnoticed the
first time. `dirty::tests::effect_order` covers the hashing (reorder, per-
instrument attribution, length, listing order), and
`dirty_state_coverage::instrument_effect_chain_reorder_marks_the_project_dirty`
covers the part a unit test cannot see: that `ReorderEffect` really does publish
into the snapshot the fingerprint reads.

### 0.2 Atomic save, autosave, and recovery

**Done** (`c417e404`).

- **Atomic saves.** `io/atomic.rs` writes to a uniquely named temp file in the
  destination's own directory, syncs, inherits the destination's permissions and
  replaces with a single rename. Used by projects (`project.rs`), sample bundles
  (`bundle.rs`, via `write_with` so ZIP encoding failures are caught before the
  replace), patches, group templates, settings, and the recovery sidecar.
  Failure-path tests cover payload errors keeping the previous file, temp-file
  cleanup, missing destination directories, and permission inheritance.
- **Crash recovery.** `recovery.rs` writes debounced snapshots to a private
  directory on a worker thread, never over the user's file, and as a ZIP bundle
  when the project holds samples so recovery does not hand back a project with
  every recorded sample gone. Snapshots are retired whenever the work stops being
  at risk, which makes one surviving to startup the crash signal itself — no lock
  file needed. Bounded by age and count; recovered documents open unsaved.
  `tests/recovery_lifecycle.rs` covers offer-after-crash, saved work not being
  re-offered, declining, an external manual save beating an older snapshot,
  ordering, round-tripping, and one-snapshot-per-project.

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

**The last two gaps closed 2026-08-06.**

- **Effect-chain reordering** now records `UndoAction::SetEffectChainOrder`,
  carrying the whole slot order on both sides rather than the direction of the
  move — replaying "one slot up" would swap whichever pair sits at that index by
  then. The surface is the ▲/▼ buttons on an effect module's header in the patch
  editor, i.e. the *instrument* chains; the master and return chains have no
  reorder control in the mixer at all (only MCP reorders them), which is why the
  original entry's "dragging an effect to a new slot" never matched anything.
- **An undone effect addition** no longer redoes as a default-parameter effect
  appended to the end. The entry is refreshed against the live chain at the
  moment it is undone, so redo restores the effect as it actually was when it
  went away. In a pure-GUI sequence the later parameter entries already replayed
  those values; what this fixes is state that changed *outside* the undo manager
  (an MCP `set_master_effect_parameter`, a chain reorder) and the half-redone
  state after redoing only the add.

### 0.4 Focus-safe shortcuts and global transport

**Done** (`c417e404`). One binding table (`gui/shortcuts.rs`, `AppShortcut`:
New, Open, Save, Save As, Undo, Redo, TogglePlayback) serves both the dispatcher
and the File menu, so a menu entry cannot drift from the key it advertises.
Shortcuts dispatch before view input and consume their keys. An `InputGate`
silences the piano *and* the shortcuts whenever a text field or a modal owns the
keyboard, which fixes the piano reading raw key state: the computer-keyboard
layout uses exactly the letters the editing chords use, so every Ctrl+S played a
C-sharp on its way to saving. Held notes release on focus loss. Transport is the
bare spacebar, dispatched at app level rather than per sequencer editor, and
yields to a focused widget. `tests/shortcut_routing.rs` covers all of it,
including the standing assertion that no application shortcut can be mistaken for
a note.

### 0.5 In-app verification of section 0

**Closed 2026-08-06.** Driven through the egui inspection MCP on v0.316.0, then
a code read over what the MCP could not judge. Section 0 is done.

Closed rather than fully verified, and the difference is worth keeping straight:
three checks below were never performed and were signed off as an owner's call,
not observed. If §0 behaviour ever looks wrong, start there — the list is the
map of what was never watched.

**Verified working:**

- **Shortcut routing.** The File menu renders Ctrl+N/O/S/Shift+S and the Edit menu
  Ctrl+Z / Ctrl+Shift+Z, greyed out correctly on an empty stack — one table, so
  menu and dispatcher cannot drift. Typing `asdfgzxcv` into the instrument search
  field played **no** notes (Voices stayed 0, no keys lit): the headline §0.4 bug
  is gone.
- **Global transport.** Bare spacebar toggles play/pause from all eight main views
  (Home, Rack, Notes, Mod, Pattern, Seq, Mixer, Sample), checked per view rather
  than by parity so a single dead view could not hide.
- **Unsaved-changes prompt.** New Project over a dirty project raises the
  Save / Don't Save / Cancel modal; Don't Save establishes a clean baseline.
- **Crash recovery, end to end.** Edited, waited out the 30 s debounce, confirmed
  `~/.local/share/pertylizer/recovery/untitled.{ptz,json}` appeared, `kill -9`,
  relaunched: the offer appeared ("closed with unsaved changes to 'Untitled'"),
  Recover restored the instrument, master fader, master Reverb and octave exactly,
  and the document opened as `Untitled *` with "save to keep it".

**Never observed — closed by decision, not by test:**

- **Undo/redo per editor, by ear.** Only the mixer path was driven. Samples,
  instruments/rack and the sequencer were never taken through
  change/undo/redo with the *sound* confirmed following, which no automated
  pass can judge. Their record/apply paths were read instead, and the pattern
  gaps that read found are fixed below.
- **Save-path checks.** Overwriting an existing project through the file
  dialog and confirming the file is whole was never exercised in the app.
  `io/atomic.rs` covers the mechanism — overwrite, permission inheritance, a
  failing payload leaving the previous save intact, no leaked temp files — but
  not the GUI path to it.
- **Modal input priority** was never exercised in the app. The two defects the
  code read found here are fixed below, and `dialogs::tests` now pins the
  predicate.

**Code read over those three, 2026-08-06 — four defects found, all fixed.** A
read is no substitute for the ear test, but each of these would have been hit by
a click-through.

- **A cancelled file dialog closed the input gate for the rest of the session.**
  The sharpest of the four, and it was found while fixing the modal predicate
  rather than looked for. `update_file_dialog` only cleared `file_dialog_mode`
  when it had a path to hand back, but backing out never yields one — so
  `is_file_dialog_open()` stayed true forever, and with it `modal_is_open()`.
  One cancelled Open Project and every application shortcut plus the whole
  computer-keyboard piano were dead until restart. `FileDialogResult` gained a
  `Cancelled` variant so the mode is cleared either way, which the deferred-save
  fix below then reuses as its own cancellation signal.
- **The instrument-delete confirmation was not in `modal_is_open()`.** While
  "Delete instrument?" was up a bare space toggled playback, the letter keys
  played the piano, and Ctrl+N/O/Z reached the document behind it — on a window
  whose own text says the action cannot be undone. The predicate was a
  hand-maintained chain of nine flags, the "remember to report" shape §0.1
  exists to remove, so the fix is structural: a `ModalDialogs` struct whose
  `any_open` destructures it **exhaustively**, making a new dialog that earns a
  field a compile error until it is handled.
- **"Save" in the unsaved-changes prompt dropped the pending action for an
  untitled project.** `save_current_project` returned a bare `bool`, which
  cannot tell "the save failed" from "the save is waiting on a filename", and
  the prompt cleared `pending_action` on both. So on a never-saved project:
  close the window, Save, pick a name — the project saved and the app did not
  quit. `SaveOutcome` now distinguishes the two; the file dialog resumes the
  action once the bytes are on disk, and drops it on failure or cancel.
- **Four pattern actions in the arrangement view recorded no undo.**
  Double-click on empty timeline, "New Pattern Here", "Duplicate Pattern" and
  "Place Existing Pattern" — while the same operations in the pattern view did
  record, and "Delete Pattern" in the same context menu did too. Ctrl+Z after
  creating a pattern therefore undid the *previous* edit and left the new
  pattern standing, and the untracked mutation latched
  `untracked_mutation_since_save`, disabling undo-back-to-clean for the session.
  A create-and-place now collapses into one `AddPattern` entry, and placing an
  existing pattern records an `InsertPlacement`.

**Found during the pass:**

- **Master volume recorded no undo entry** — fixed. The fader sent
  `SetMasterVolume` straight past the `MixerUndo` already in scope, while every
  other mixer control captured, so it was the one control in the mixer with no
  history. It now records through `record_drag` like the rest, via a
  `SetMasterVolume` undo action that re-sends the command (master volume is an
  engine atomic, not a `Song` field, so it cannot reuse the mixer-value
  appliers). Verified live: fader 1.00 → 0.32, Ctrl+Z restores 1.00 and the
  title reads clean again.
- **"Menu popups render see-through" was not a bug — withdrawn.** Recorded here
  so it does not get re-filed. Menus looked translucent in every screenshot with
  panel content legible straight through them; that is egui's `Area` fade-in
  animation, caught mid-transition because each screenshot was taken in the same
  MCP round trip as the click that opened the menu. Waiting a beat first shows a
  fully opaque menu. **Lesson for future GUI checks through the inspection MCP:
  let animations settle (`wait_for`, or a second call) before judging what a
  screenshot shows.**

## 1. Sequencer & Arrangement

### 1.1 Automation carriers and overlapping placements

- [ ] **Review and improve how automation-only pattern carriers coexist with
  musical placements on the same track. High priority.** A real exported project
  (`Nemesis_the_Warlock.ptz` from `sid-analyzer --format synth-native`) contains
  one song-length automation-only pattern per instrument track, overlapping the
  sequential musical pattern placements. Playback is currently correct: the
  arrangement engine evaluates every active placement, automation still runs on
  a muted host track, and explicit `Module` targets can address several
  instruments from one carrier. The arrangement UI nevertheless makes these
  carriers look like empty or conflicting musical clips, and overlapping lanes
  that address the same target have no clearly surfaced conflict policy.

  Investigate and decide, in order:

  1. **Recommended persistence model** — keep lanes pattern-owned, add true
     track/song-owned automation, or formally recommend one consolidated or
     dedicated automation carrier. Confirm that one carrier targeting multiple
     instruments survives save/load and produces live/offline-render parity.
  2. **Arrangement presentation** — distinguish automation-only placements from
     note clips, prevent a song-length carrier from visually obscuring the
     musical arrangement, and keep its lanes discoverable and editable.
  3. **Overlap semantics** — define and surface what happens when two active
     placements automate the same target at the same tick (deterministic
     precedence, merge rule, validation error, or load/render warning).
  4. **Producer guidance** — document the preferred project shape for external
     exporters and expose enough schema/MCP diagnostics to detect redundant or
     conflicting carriers.
  5. **Regression coverage** — pin automation-only plus musical overlap,
     automation on muted hosts, cross-instrument module targets from one
     carrier, conflict handling, save/load, and headless rendering.

  The goal is not to forbid placement overlap: overlapping note clips are a
  useful layering feature. The goal is to make automation layering intentional,
  legible, and deterministic without forcing exporters to manufacture dozens of
  apparently empty clips. **High priority; M, sequencer UX/data-model review.**

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

### 5.6 Headless render CLI — open follow-ups

*(`pertylizer render` shipped to main 2026-08-06; contract and rationale kept in
`plans/headless-render-cli.md`. Core in `crates/pertylizer/src/render/`.)*

- [ ] **Point `sid-abtest` at the new command.** It still emits
  `--tap final-mix` / `--tap voice-N`, which never existed; it moves to
  `--solo-track <id>` with the track ids its own exporter wrote. The change
  lives in the **`sid-analyzer` repository**, not this one — this was the
  feature's whole reason to exist, so it is the one open exit-gate item.
- [ ] **Per-sender dropped-command counters.** `CommandSync::dropped` closed the
  "a lost command is invisible" hole, but its delta is global: a concurrent MCP
  or GUI send that drops during a load is attributed to that load. The alarm is
  real, the attribution approximate. Only worth splitting if it ever misleads.
- [ ] **Test a valid bundle whose samples are missing.** The plan asked for it;
  a *truncated* bundle is covered instead, because it was never established
  whether `load_bundle` errors or merely warns when a referenced sample entry is
  absent. Settle the behaviour first, then test it.
- [ ] **`AnalysisScope` and `McpBridgeError` still leak into the render core.**
  The scope type is plain configuration data living in `synth_mcp`, and
  `arrangement_render` still fails with `McpBridgeError`, which
  `RenderError::Render` flattens to a string. Relocating the scope to
  `synth_core` and re-typing `arrangement_render` are each their own cleanup.
- [ ] **`--no-default-features --features gui-egui` does not build.** Pre-dates
  this work (verified on main): `gui/egui_backend.rs`'s `VersionTracker::at` is
  dead without `mcp`, and `-D warnings` rejects it. The `gui-egui,mcp` and
  default configurations are fine.

## 6. MCP protocol capabilities

> Added 2026-08-05 after the `rmcp` 2.2 → 3.1 upgrade (MCP spec `2026-07-28`).
> Tool annotations from that audit have shipped; the rest is open. None of these
> are bug fixes — they are capabilities the protocol gained that we do not use.
>
> **The audit was written from the changelog, not from the schema, and §6.8 was
> wrong as a result** — it proposed a completion mechanism that cannot target
> tool arguments in any spec version. Corrected 2026-08-10. Before building any
> entry here, confirm the feature reaches the surface it is claimed for by
> reading `schema/<version>/schema.ts`, not the prose.

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

### 6.8 Ergonomics for string-keyed tool arguments

**Rewritten 2026-08-10 — the original entry rested on a false premise.** It read
"implement `complete` for our string-keyed argument space", naming module type
keys (`osc`, `flt`), parameter addresses (`flt-1.cutoff`), instrument names,
pattern ids and automation target DSL strings, and called it the biggest
day-to-day ergonomics win left. **MCP completion cannot address tool arguments
at all.** `completion/complete` targets exactly two reference types, `ref/prompt`
and `ref/resource`; there is no `ref/tool`. Verified three ways — the pinned
`schema/2026-07-28/schema.ts` (`ref: PromptReference | ResourceTemplateReference`),
the 2026-07-28 *Reference Types* table, and the **draft** spec's identical table —
and `rmcp` 3.1's `Reference` enum (`model.rs:3365`) has the same two variants, so
the SDK is spec-correct rather than lagging. Nothing is queued upstream either:
SEP-1862 ("Tool Resolution", draft since 2026-02) is preflight *annotation*
refinement before invocation, not argument suggestion — it belongs to §6.10 if
anywhere.

Note for anyone re-reading this against the newtype convention: the domain keys
*are* typed. `ModuleType` is a 75-variant enum with `prefix()` / `from_prefix()`
(`synth_core/src/params/mod.rs:528`, `:614`) and `ModuleId` is
`{ module_type, instance }` with `FromStr` (`synth_engine/src/commands.rs:30`,
`:62`); `InstrumentId` even crosses the boundary as itself
(`SetParametersParam.instrument_id`). The `String` on `ParamSetInput.module_id`
is the JSON wire form — CLAUDE.md's documented serialization exception — so none
of the work below changes the type story.

What is actually available, smallest first:

- [x] **`complete` for the two resource templates we already expose.** Done. The
  `completions` capability is declared and `completion/complete` serves
  `synth://module-types/{type_key}` and `synth://patches/{name}`; a prompt
  reference gets an empty result, since we advertise no prompts.

  Completion turned out to need a **looser** match than the near-miss hints,
  which was not obvious: a hint guesses at what was meant and must stay quiet
  when unsure, while a completion filters a list the caller is already looking
  at. The hint ranking answers `ba` with nothing — a two-character needle is
  below its containment floor — which is right for an error message and useless
  for a dropdown over `sub-bass` and `spacey-bass`. So containment matches at
  any length, prefix first, and only when *nothing* contains the text does it
  fall back to the shared ranking, which is what still recovers `lmit` → `lmt`.
  Both spellings are matched and the canonical one answered, so typing a display
  name reaches a value keyed by something else.

- [x] **Put `enum` in the input schema for the closed-set arguments.** Done for
  the sets that are genuinely closed: `search_modules`' `category` and its two
  signal-type filters are real enums now, so their values ship in the tool's
  `inputSchema` and the hand-written validation behind them is gone —
  deserialization rejects a bad value, and its message ("unknown variant
  `bogus`, expected one of `voice`, `effect`, `visualizer`") is better than the
  string we used to build.

  **Module type keys were deliberately left open, against this entry's own
  advice.** Every module-type argument is parsed by `parse_module_type`, which
  accepts the short key, the snake_case name *and* the display name, and three
  of the four field descriptions promise exactly that. A 75-key `enum` would
  advertise a constraint stricter than the server enforces, so a strict client
  would refuse calls that work today. Closing that gap means narrowing the
  parser first — a contract change, not a schema change —, and the leniency
  exists on purpose because clients pass names. `schema_enum.rs` pins the
  decision so it is not "finished" later by mistake.

  Watch the doc comments on any type reaching a tool schema: schemars publishes
  `///` as the schema's `description`, so implementation rationale written there
  ships to every client on every `tools/list`. The rationale above lives in
  plain `//` comments for that reason.
- [x] **Make the parse failures name the near miss.** Done. `ModuleId::from_str`
  answers `lim-1` with "Did you mean 'lmt' (Limiter)?" — the §7 limiter
  confusion one layer earlier — and the same hint now closes the three
  descriptor parameter lookups in the bridge, `set_parameter`'s not-found, and
  the automation DSL's unknown module type / track / global / instrument param.
  The DSL's three param lists became tables the parser and the hint both read,
  so neither can fall behind the other.

  One shared policy in `synth_core::suggest` rather than a third
  implementation: `synth_mcp` and the bridge's module search each had grown
  their own Levenshtein with different thresholds, and both moved onto it.

  Two things the work established, both counter-intuitive enough to record.
  **An edit-distance ceiling cannot do this job alone** — every module key is a
  3-char consonant skeleton, so `flt` is exactly as far from `filter` as it is
  from `osc`, and any threshold loose enough to connect the first connects the
  second. Name→key is recovered by offering the *name* as its own candidate and
  mapping the winner back (`ModuleType::suggest`), which is exact rather than
  approximate. **Containment must be anchored**, or the suggestion is
  confidently wrong: unanchored, `grain` "means" `ain` (Audio Input) and `ent`
  "means" `tsh` (Transient Shaper) while `env` sits one edit away. Both are
  pinned by tests against the real 75-type catalogue.

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

## 7. Load & apply diagnostics

**Done.** `apply_project` returns a `ProjectApplyReport` — the summary it always
returned, plus a typed `ProjectApplyDiagnostic` for everything it could not
reconstruct. The eight silent `continue`s report, `ApplyPatchResult::errors`
carries the same type instead of prose bound for stderr, and all three entry
points surface it: the render receipt takes them as warnings *before* the
render, the GUI logs one Activity-panel event each, and `load_project` appends
them to its reply. A clean load reads exactly as it did before.

Both follow-ups the work turned up are now closed; the first one left one
narrower item behind, filed below it.

- [x] **A module id that names a different type than the entry claims was still
  silent inside an instrument patch.** `apply_patch` now emits the same note the
  effect chains do, and still installs the module — but the consequence check
  this entry was waiting on came back worse than the chain case, so a note alone
  would have been the wrong answer. In a chain the id is an opaque slot key and
  such an entry loads and sounds correct. Inside a patch the id *is* the
  module's identity, and `set_parameter` picked `SetEffectParameter` vs
  `SetModuleParameter` off its prefix: a Delay filed under `flt-9` was built as
  an effect and then had every parameter sent to the voice-module map, where the
  send succeeded and the value was dropped without a word. Routing now reads the
  descriptor's `category` — the truth was already in hand two lines above —
  and `module_factory::tests::category_and_kind_agree_for_every_module_type`
  holds that invariant for every module type, since it is now load-bearing.

  What a note *is* the whole answer for is the rest: the engine's per-type scans
  (`find_module_by_type`, the per-block Script collection) and mod-matrix address
  resolution key on the id, so a Script saved as `osc-1` is built correctly and
  then never evaluated. Nothing can look that up — the id is the identity — so
  the diagnostic is the fix. Asserted through rendered audio, not the saved
  parameter: the shared-graph mirror stores the value by id and reports it
  applied either way (the §5.6 mirror-vs-engine class again).

  The review pass found the same prefix-vs-truth split one layer out:
  `audio/instrument_hydration.rs` bucketed modules effect-vs-voice from the
  snapshot's real type but *rebuilt* them from the id's prefix, so preview and
  offline render would have built a Filter where the live engine has a Delay.
  Now builds from `module.module_type` and routes from `descriptor.category`,
  the same rule as `set_parameter`. Deciding to keep mismatched modules is what
  made that reachable, so it belongs to this entry.

- [ ] **`set_mod_script` still reads the module's kind off the id prefix.**
  The compile dialect (`script_is_audio_rate` / `script_uses_control_ports`)
  and the rebuilt knob descriptor both come from `module_id.module_type`, so an
  `AudioScript` saved as `scr-1` compiles in the control-rate dialect. Left as
  it is on purpose: unlike `set_parameter` the truth is *not* at hand — the
  session registry stores a `ModuleDescriptor` whose `type_id` is a string with
  no route back to `ModuleType` — so fixing it means threading the declared type
  through a public API with eight-plus call sites, most of which only hold the
  id. The diagnostic now warns about exactly this module. Revisit if the
  registry ever starts recording the type it was built from. **S.**
- [x] **The GUI only showed diagnostics in the Activity panel.** The panel is
  worse than "a user might not open it": it lives on the Home view, and loading
  a project moves the user to the Rack — so the one place the account was
  written is the one place they are no longer looking. A status-bar badge
  (`widgets::controls::attention_badge`) now carries the count, with the first
  eight diagnostics in its tooltip; clicking it goes to Home and clears the
  badge, on the grounds that following it is the acknowledgement. Chosen over a
  banner: a recoverable load failure should be visible without being modal.
  **Not yet eyeballed in-app.**

Original entry, kept for the diagnosis:

- [x] **A load that silently drops project objects reports success.** A master
  effect saved with the schema-valid type `limiter` but the module id `lim-1`
  (the canonical prefix is `lmt`) is discarded without a word: the project
  loads, the GUI says nothing, and a `pertylizer render` receipt carries an
  empty `warnings` list over a mix that is missing an effect. Renaming the id
  to `lmt-1` makes the same project render through the limiter.

  This is the same failure class as the `dropped` command counter closed in
  §5.6 — an operation reporting success over state the engine never received —
  and it matters most where it is hardest to notice: an offline render feeding
  an external A/B harness has no human looking at it.

  **Two halves, and the second is the bigger one.**

  *`project_apply.rs` throws diagnoses away.* Eight `continue;` sites drop
  silently: an unparsable module id and an unknown effect type, each for both
  return-bus (`:237`, `:241`) and master (`:269`, `:273`) chains, plus the
  sampler binding path (`:947`–`:960`) where a missing sample id, an unparsable
  module id, or a sample absent from the library each skip a voice's audio
  without comment.

  *`ApplyPatchResult::errors` already exists and goes to stderr.* Every module,
  connection and parameter failure inside an instrument is already collected —
  `session.rs:1369` — and both callers (`project_apply.rs:738` and `:766`)
  `eprintln!` it and move on. So the per-instrument diagnoses are *already
  computed*; nothing routes them anywhere a caller can see. That is the cheap
  half of this task, and it covers exactly the bad cables, modules and
  parameters the effect-chain sites do not.

  Scope, in order:
  1. **A typed diagnostic** carrying the project-object path
     (`global.master_effects[0]`), a stable code, severity, and message.
     Recoverable omissions are warnings; reserve errors for a load that cannot
     produce a coherent project. `ApplyPatchResult::errors` is a `Vec<String>`
     today, so it needs the same treatment rather than being wrapped.
  2. **Return it from `apply_project`** alongside the existing summary, folding
     in each instrument's `ApplyPatchResult::errors` instead of printing them.
  3. **Thread it into `RenderReceipt.warnings`** before the render runs, so a
     receipt cannot look clean over a partial project.
  4. **Show it in the GUI load result and the MCP `load_project` response**, so
     all three entry points agree on what happened.
  5. **Name the expected prefix** when the effect type is known but the id's
     prefix is wrong (`limiter` → `lmt`). A prior draft of this proposed
     `ModuleType::short_key`, which **does not exist** — the available
     accessor is `prefix()` at `synth_core/src/params/mod.rs:528`; confirm what
     type it hangs off before building on it.

  Verify with one valid and one deliberately malformed `.ptz` through
  `pertylizer render`, comparing both receipts, plus a bundled example loading
  with no new diagnostics. **M.**

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
