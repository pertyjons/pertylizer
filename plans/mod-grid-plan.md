# Mod Grid — pooled control-rate modulator graphs

Status: PROPOSED

Origin: design discussion 2026-07-18, growing out of the automation-platform
plan ("could a patch LFO drive an automation lane?"). UI mockup reference:
https://claude.ai/code/artifact/d612e28b-d0f5-4fbe-9992-e571a3f8a14f

## 1. Goal

A third node view beside the patch editor and the Note Grid: a pool of
**control-rate modulator graphs** (global or track-scoped) whose outputs write
into the automation target space. The patch graph processes audio per voice,
the Note Grid processes note events — the Mod Grid processes control signals,
monophonic and always-running. Prior art: Renoise meta-devices, FL internal
controllers, Ableton M4L LFO/Envelope Follower, Reason CV.

Lanes stay pure authored **data**; the Mod Grid is live **signal**. Both write
to the same targets and compose additively.

## 2. Prerequisites (hard ordering)

Builds on `plans/automation-platform-plan.md`:

- **A1** — relative Track targets (`Option<TrackId>`, "this track") is the
  semantics track-scoped graphs reuse.
- **A2** — `TrackParam::Pitch` + voice→track tagging + the per-block
  `set_voice_pitch` application path is what makes pitch a grid target.

Land the automation platform (and the tracker-import DAW mapping that waits on
it) first; Mod Grid is its own branch from `main` afterwards.

`plans/user-macros.md` is complementary, not a dependency: per-instrument
macros travel with the patch; the Mod Grid's Macro node (§4.2) is the
song-level bank that plan's §6 explicitly deferred.

## 3. Verified current state (what gets reused)

- **Node view rendering** — ModuleNode/port/cable widgets on `egui::Scene`
  (`gui/widgets/cable.rs`, patch editor + `note_grid_view.rs`) are already
  shared by two views; the Note Grid proved the pool + scope + assignment
  pattern this copies.
- **Write path for module params** — the generic `ParamModOffsets` store
  (landed with the dynamic mod matrix work) accepts external additive offsets
  on all voice modules.
- **Write path for track params** — `update_track_controls`
  (`synth_engine.rs:2945`) composes `track_auto` per block; the grid becomes a
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
  `ParamModOffsets` path (uniform across voices), track params into the
  `track_auto` composition, globals into their existing per-block reads.
  Composition rule everywhere: `lane value (or base value) + grid offset`,
  clamped. A lane need not exist for a routing to apply.
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

Graph pool + assignments + seeds in the project save; schema + upgrade-free
(additive). Save/load round-trip is part of the exit gate.

## 5. Open questions

- **Zipper control** — offsets are block-constant; decide whether target
  application reuses existing param smoothing or needs a one-pole per target.
- **Macro node vs user-macros GUI** — one shared knob-rail widget? Decide
  when both exist.
- **Audio tap points** — track post-fader vs pre-fader; start with one
  (post-fader) and a fixed tap list.
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
- Full round-trip: save/load, MCP create→route→enumerate, quick-assign from
  the lane side, provenance listing answers "what writes to this target".
- Workspace green (`build` / `clippy --all-targets` / `test` / `fmt
  --check`).
