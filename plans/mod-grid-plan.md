# Mod Grid — pooled control-rate modulator graphs

Status: IMPLEMENTED (branch `feat/mod-grid`, not yet eyeballed in-app)

Landed: data model + Song pool (§4.1), engine control-rate pre-pass with
track/global write paths (§4.3), cheap sources Macro/Transport/AudioTap (§4.2),
app builder + GUI generation-watch sync, MCP tool family (§4.5), GUI node view
with named ports + module→module cables + scope/assignment (§4.4), persistence +
offline-render wiring (§4.6), lane provenance chip + jump + quick-assign.

Follow-ups landed 2026-07-18 (see TODO §2.9): MidiCc live-CC pipeline,
module-param grid write targets (per-voice `apply_mod_offset_addr`),
cheap→module injection, and per-node `seed` application, plus the MCP
`disconnect_mod_graph` / `set_mod_graph_node` tools.

Still deferred (detailed with an implementable spec in **§7**): live in-app
eyeball of the §6 exit gate, the GUI Module-target picker, patch-editor dest
markers (corner system full + `PatchAnalysis` plumbing), instrument-level
Volume/Pan targets, DriftGenerator seeding (global `fastrand`), and full
sustain-pedal note-hold semantics.

Origin: design discussion 2026-07-18, growing out of the automation-platform
plan ("could a patch LFO drive an automation lane?"). UI mockup reference:
https://claude.ai/code/artifact/d612e28b-d0f5-4fbe-9992-e571a3f8a14f

External review 2026-07-18, verified against the code: its two technical
findings (stale-offset clearing, instance-rebuild sync) are folded into §4.3
and its recommendations resolve most of §5.

## 1. Goal

A third node view beside the patch editor and the Note Grid: a pool of
**control-rate modulator graphs** (global or track-scoped) whose outputs write
into the automation target space. The patch graph processes audio per voice,
the Note Grid processes note events — the Mod Grid processes control signals,
monophonic and always-running. Prior art: Renoise meta-devices, FL internal
controllers, Ableton M4L LFO/Envelope Follower, Reason CV.

Lanes stay pure authored **data**; the Mod Grid is live **signal**. Both write
to the same targets and compose additively.

## 2. Prerequisites — SATISFIED (verified 2026-07-18)

The automation platform landed in `main` (`c926650e`):

- **A1** — relative Track targets (`AutomationTarget::Track { track:
  Option<TrackId> }` + `resolved(host_track)`, `automation.rs:295`/`:366`) is
  the semantics track-scoped graphs reuse.
- **A2** — `TrackParam::Pitch` (`automation.rs:434`, bipolar ±48 st encoding)
  plus voice→track tagging with per-block pitch application are in.

Mod Grid can start on its own branch from `main` at any time; sequencing it
against the tracker-import DAW mapping is a prioritization call, not a
dependency.

`plans/user-macros.md` is complementary, not a dependency: per-instrument
macros travel with the patch; the Mod Grid's Macro node (§4.2) is the
song-level bank that plan's §6 explicitly deferred.

## 3. Verified current state (what gets reused)

- **Node view rendering** — ModuleNode/port/cable widgets on `egui::Scene`
  (`gui/widgets/cable.rs`, patch editor + `note_grid_view.rs`) are already
  shared by two views; the Note Grid proved the pool + scope + assignment
  pattern this copies.
- **Write path for module params** — the generic `ParamModOffsets` store
  (`synth_core/src/module_traits.rs`, landed with the dynamic mod matrix
  work) accepts external additive offsets on all voice modules, applied via
  `graph.apply_mod_offset_addr` (`graph.rs:561`).
- **Write path for track params** — `update_track_controls`
  (`synth_engine.rs:2953`) composes `track_auto` per block; the grid becomes a
  second contributor into that composition (and A2's pitch path).
- **Target addressing** — `AutomationTarget`
  (`synth_sequencer/src/automation.rs`) is the shared address space; the GUI
  lane picker and MCP `AutomationTargetInput` already enumerate it.
- **Source modules** — the control-rate family exists in `synth_modules`:
  `lfo`, `mseg`, `envelope_follower`, `drift_generator`, `kinetic_modulator`,
  `turing_machine`, `random_gates`, `euclidean`, `beat_detector`,
  `script_module` (YAMS, 4 CV out + knobs), `math`.

## 4. Design

### 4.1 Data model

- `ModGraphId` newtype; a pooled `ModGraph { id, name, scope, nodes, cables,
  targets }` stored in the `Song` (sibling of the note-graph pool).
- `ModGraphScope::Global` — exactly one running instance, always on.
- `ModGraphScope::Track` — pooled; the graph is **assigned** to one or more
  tracks, one running instance per assignment, and "this track" targets
  resolve to the host track (A1 semantics). One "duck vs kick" graph assigned
  to five tracks = five instances.
- **No instrument scope.** Per-instrument modulation stays in the patch (mod
  matrix, per-voice, retrig) and in user macros. A grid graph may still
  *target* instrument/module params from global/track scope.
- **Target node** = `{ target: AutomationTarget, amount (in the target's
  units), combine: Add }` with an input CV port. Additive, clamped to the
  target's range. `TrackParam::Mute` (bool) is excluded from the picker
  initially.

### 4.2 Nodes

Existing modules hosted **mono** by a control-rate harness: one instance, no
voice, processed once per block with transport/tempo context; per-voice-only
behaviors (note retrig, key tracking) are simply absent in this hosting. New
cheap nodes: **Macro** (named knob — the song-level macro bank), **Transport**
(beat/bar phase ramps, song position, tempo), **MIDI CC in**, **Audio tap**
(reads a track/bus level from the mixer; source only, one-block latency).

### 4.3 Engine

- Graph instances live in the engine and are processed **once per block on
  the audio thread, before instruments and before `update_track_controls`**.
  Control-rate, pre-allocated, lock-free — normal RT discipline.
- Outputs land as block-constant offsets: module params via the
  `ParamModOffsets` path (`apply_mod_offset_addr` per active voice — uniform
  across voices), track params into the `track_auto` composition, globals
  into their existing per-block reads. Composition rule everywhere: `lane
  value (or base value) + grid offset`, clamped. A lane need not exist for a
  routing to apply.
- **Fix first — unconditional offset clearing** (review finding):
  `voice.rs:1081` runs `graph.clear_mod_offsets()` only when
  `mod_matrix_id.is_some()`, so a grid routing into a patch without a Mod
  Matrix module would leave stale offsets forever. Make the clear
  unconditional before wiring any grid writes.
- **Instance-rebuild sync** (review finding): add `mod_grid_generation: u64`
  to `Song`, mirroring `structure_generation` (`song.rs:246`) — bumped on any
  graph/assignment mutation; the audio thread compares it per block and
  rebuilds running instances off the hot path.
- **Audio tap**: pre-fader, via `Instrument::last_output_interleaved()`
  (`instrument.rs:724`). Pre-fader so a grid-ducked fader cannot re-duck its
  own detector (feedback); lock-free by construction.
- No added latency for pure-control chains; only audio-derived sources (tap →
  env follower) read the previous block.
- **Determinism**: random-family nodes get explicit seeds (persisted), so
  offline render — which runs the same engine path — reproduces the live
  result exactly. No new offline-snapshot surface (cf. the analyze_* offline
  snapshot bug class).

### 4.4 GUI

- New **Mod Grid** view tab: left pool panel (graphs with GLOBAL/TRACK scope
  chips, assignments editor), node canvas reusing the shared widgets, canvas
  header with name + scope + a small CPU readout.
- **Target nodes** render with the owner's color as an edge stripe
  (track/instrument/global), the amount in the target's units, and the
  combine rule.
- **Lane-side quick assign** — the floor of the feature: on any automation
  lane zone (and parameter context menus), a "⊕ Mod Grid" menu that picks a
  graph + output and creates the target node without opening the view.
  "LFO on track volume" must be ~3 clicks.
- **Provenance is first-class, not polish**: a chip on lane headers naming
  the writing graph (click = jump to it), dest markers on modulated params in
  the patch editor (same marker language as the mod matrix), and a "what
  writes to this target" listing. With mod matrix + YAMS + lanes + note
  processors + macros + grid all able to move a knob, "why is this parameter
  moving?" must always be answerable in one glance.

### 4.5 MCP

House style: array-capable, descriptor-validated.

- Pool: `create_mod_graph`, `delete_mod_graph`, `list_mod_graphs`,
  `set_mod_graph_scope`, `assign_mod_graph` (tracks).
- Contents: add/remove node, connect/disconnect, set node parameter —
  mirroring the note-graph tool family.
- Routing: `add_mod_target`, `remove_mod_target`, `set_mod_target_amount`;
  targets addressed with the same `AutomationTargetInput` (incl. A4's
  track/global variants).
- Provenance: `list_mod_targets` (per graph and per target — "what writes
  here"), and grid routings reported by `get_automation_summary` /
  `list_automation_lanes` alongside lanes.

### 4.6 Persistence

Graph pool + assignments + seeds in the project save; upgrade-free
(additive). The schema propagates by regenerating (`cargo run -p pertylizer
--bin gen_schemas`). Save/load round-trip is part of the exit gate.

## 5. Open questions

Resolved by the 2026-07-18 review:

- **Zipper control** — reuse the modules' existing per-block smoothing;
  block-constant offsets match mod-matrix behavior. No per-target one-poles.
- **Macro node vs user-macros GUI** — shared knob-rail widget, visually
  distinct groups ("Song Macros" vs "Instrument Macros").
- **Audio tap points** — pre-fader (`last_output_interleaved`), fixed tap
  list to start; feedback rationale in §4.3.

Still open:

- **Instance CPU budget** — show per-graph cost in the canvas header; decide
  a soft cap on assignments if needed.

## 6. Exit gate

- A global graph (LFO → Track 2 volume) wobbles the fader with no lane
  present; adding a volume lane shows the composed result (lane + offset) and
  the lane header chip names the graph.
- A track-scoped graph assigned to two tracks runs two instances, each
  resolving "this track" to its host (moving an assignment follows the
  track).
- LFO → `TrackParam::Pitch` bends all voices of the host track only (rides
  A2's `set_voice_pitch` path); a second track sharing the instrument is
  unaffected.
- Env-follower graph tapping the drum track ducks another track's volume —
  sidechain without audio re-routing.
- Seeded random node renders bit-identically live vs `render_to_wav`.
- Removing a grid routing (or the whole graph) returns every target to its
  base value — no stale offsets, including on patches without a Mod Matrix
  module (the §4.3 clearing fix).
- Full round-trip: save/load, MCP create→route→enumerate, quick-assign from
  the lane side, provenance listing answers "what writes to this target".
- Workspace green (`build` / `clippy --all-targets` / `test` / `fmt
  --check`).

## 7. Remaining work (deferred follow-ups)

**Status 2026-07-18: 7.1–7.6 all SHIPPED** (branch `feat/mod-grid`, one commit
each, gate green + per-step review): DriftGenerator RNG (7.4), instrument-level
targets (7.3), sustain-pedal note-hold (7.5), patch-editor grid dest markers via
Alt A (7.2), GUI Module-target picker (7.1), and Mod Grid CPU readout (7.6). 7.1
was later upgraded from a positional/free-form picker to a **validated** one, then
to the **same nested `tree_picker_button` menu the pattern-view "Auto:" selector
uses**, driven by a shared `module_targets::module_target_groups` helper (single
source of truth, also now used by MCP `get_instrument_automation_targets`) — it
lists each instrument's real automatable modules + params. Scope note: 7.6 is
total-across-instances, not per-graph. **The §7 GUI was eyeballed live 2026-07-18
("good enough")**, so 7.0 is no longer a blocker.

### 7.0 In-app eyeball — DONE ("good enough", 2026-07-18)

The §7 GUI (Module-target menu, grid-dest markers, CPU readout) was clicked through
in-app and looks fine. An exhaustive walk of every §6 exit-gate scenario is still
worthwhile but no longer blocking. For reference, the paths to spot-check are: a
`Macro → LFO.rate_cv` injection changes
the wobble; a `MidiCc` node moved by a live controller drives its target; a grid
`Module` target opens a filter cutoff; a Track graph on two tracks decorrelates;
the grid-dest marker stacks cleanly above a matrix-dest one (7.2's 12px offset);
the Module-target picker builds a working target (7.1); the header CPU % moves
(7.6); and the sustain pedal holds + releases notes (7.5). The four engine items
(7.3/7.4/7.5 + the 7.2 data layer) are unit-tested; the GUI is not.
when the nodes carry a `seed`. Capture anything that looks wrong as a fresh TODO.

### 7.1 GUI Module-target picker (grid Target nodes) — **M**

Today `edit_target_body` (`gui/mod_grid_view.rs`) only offers the This-track and
Master presets; a `Module` target renders its address but is **created** only via
MCP or the lane-side quick-assign. Add an instrument → module-type → instance →
`param_id` picker so a Target can address a module param from the canvas.

- Needs per-instrument module introspection in `ModGridViewState` (it currently
  carries only `descriptors: HashMap<ModuleType, Option<ModuleDescriptor>>` and
  `tracks`). Plumb in a `Vec<(SeqInstrumentId, name, Vec<(ModuleType, instance,
  Vec<param>)>)>` snapshot collected where the view is built, mirroring how the
  automation-lane picker already enumerates `AutomationTarget::Module`.
- Reuse the lane picker's target-enumeration logic rather than re-deriving it;
  emit the same `AutomationTarget::Module { instrument, module_type, instance,
  param_id }` the engine already processes.
- Only continuous, modulatable params should be offered (filter by the descriptor
  `modulatable` flag / `ParamModOffsets` registration).

### 7.2 Patch-editor dest markers for grid-written module params — **S–M once unblocked**

Module targets are engine-processed now, so a "this param is written by a Mod
Grid" marker in the patch editor is no longer aspirational (plan §4.4). Two real
blockers surfaced while implementing:

- **The `ModMarker` corner system is full.** Four markers already own the four
  fixed corners (`param_grid.rs`, `corner()` + `paint_marker_corners`), with the
  invariant "two markers never collide". A 5th grid-dest marker needs a corner
  decision: either stack the two *destination* markers (matrix `↙` + a new grid
  glyph) at the bottom-left with a vertical offset, or fold both into one
  "destination" marker carrying a kind. Prefer the offset-stack — it keeps the
  matrix marker untouched.
- **`PatchAnalysis::from_panels` can't see the grid.** It only takes the patch
  panels. It needs the song mod-graph pool + the edited `SeqInstrumentId` to find
  `Module` targets whose `instrument` matches, then mark `(module_type, instance,
  param_id)`. Mirror the existing `automated_modules: &HashSet<ModuleId>` param
  that the caller already computes for the lane "automated" badge — add a sibling
  `grid_dest_params: &HashMap<ModuleId, HashSet<String>>` (or a module-level badge
  if the per-param corner work is deferred).
- This is un-eyeballable pixel-layout work; provenance is *already* answerable via
  MCP `list_mod_targets` and the grid view, so it is polish, not a gap.

### 7.3 Instrument-level grid targets (Volume / Pan) — **S–M**

`AutomationTarget::Instrument { param: Volume | Pan }` is still a `_ => {}` no-op in
`process_mod_grid` (only the module-backed path landed). These are *channel-level*
(`inst.set_volume` / `set_pan`), not a per-voice mod offset, so they need an
instrument-offset accumulator analogous to `grid_track_offsets` —
`grid_instrument_offsets: HashMap<InstrumentId, {volume, pan}>` reset each block,
folded into the instrument's channel gain after its own automation. The
module-backed `AutoInstrumentParam` variants (FilterCutoff/Resonance/ADSR) could
instead map to a `DestAddr` at build time (first module of the type, the existing
convention) and reuse the §7-shipped per-voice path — cheaper than a new store.

### 7.4 DriftGenerator seeding — **S–M**

`ModuleNode.seed` decorrelation works for RandomGates + TuringMachine but
`DriftGenerator::set_seed` is the default no-op: it draws from **global**
`fastrand`, so it can't be seeded per instance — and is therefore also
**non-deterministic in offline render today** (a pre-existing determinism gap,
independent of the grid). Give it a per-instance xorshift RNG (a `rng_state: u32`
field seeded in `set_seed`, replacing the `fastrand::f32()` calls in `process`),
matching RandomGates/TuringMachine. This closes both the decorrelation gap and the
offline-determinism gap in one change. Add a determinism test (two renders match).

### 7.5 Full sustain-pedal (CC64) note-hold semantics — **M, not grid-specific**

CC64 now reaches the grid CC state (a `MidiCc` node can read the pedal), but the
per-voice pedal behavior is still unimplemented: hold NoteOff for notes sounding
while the pedal is down, release them when it lifts. Needs voice-level
held-note tracking in the engine and an `EngineCommand` path from the CC64 value.
Tracked here only because the MidiCc work touched the same MIDI-parse site.

### 7.6 Smaller leftovers

- **Instance CPU budget** (from §5, still open) — per-graph cost in the canvas
  header, and a soft cap on assignments if needed.
- **Cheap→module for `MidiCc` injections** already works (the injection path
  handles every `ModSource`), but its determinism in offline render is nil (no
  live CC) — document at the call site if it ever surprises.
