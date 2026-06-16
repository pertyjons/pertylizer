# TODO - Pertylizer

## 0. Known Bugs

### 0.1 Misc findings

- [ ] **Swing / Track-Solo automation unimplemented.** Most automation targets now
  reach the engine (instrument macros, generic `AutomationTarget::Module`, Track
  Volume/Pan/Mute, Global MasterVolume — all shipped via channel-strip Phases 1–2).
  `Global(Tempo)` was removed in favour of the tempo map (see §2.1). Still no engine
  implementation for `Global(Swing)` and `Track(Solo)` — lanes drawn for those targets
  are silent no-ops. Either wire them or drop the variants.
- [x] **★ HIGH: expand Sub Oscillator waveform set from 3 to 6.** `SubOscWaveform`
  (`crates/synth_core/src/params/sub_osc.rs:13`) currently exposes only
  `Sine / Square / Pulse25`, while the main `Oscillator` exposes 6
  (`Sine / Triangle / Sawtooth / Square / Pulse / DsfSaw`). Add the three
  missing shapes: `Triangle`, `Sawtooth`, `DsfSaw`. Keep `Pulse25` distinct
  from `Pulse` (Pulse25 = fixed 25 % duty, dedicated bass shape — Pulse
  needs a PulseWidth param that the lean Sub Osc workflow deliberately
  skips). The waveform-selector widget already filters by descriptor
  choices (`gui/widgets/waveform.rs::WaveformType::from_id`) so the GUI picks
  up the new buttons automatically as long as `WaveformType::from_id` covers
  the new ids. Touch points: `sub_osc.rs:23-55` (variants + `ALL` + `name` +
  `id` + `to_choices` + rendering branch in `generate_sample`),
  `WaveformType::from_id` (mappings for `triangle`/`sawtooth`/`dsf_saw` —
  already exist). No save-format bump required; existing `"sine"` / `"square"`
  / `"pulse25"` keep loading.
- [ ] **Follow-up: remove dead modules inside patch graphs.** Modules not
  reverse-reachable from `StereoOutput` (and any sidechain source) through
  `connections` should also be pruned in `optimize_project` (which today only
  removes unused patterns/tracks/instruments/samples, not unreachable modules
  inside a patch graph). Needs graph traversal per instrument.
- [ ] **Follow-up: stale `list_instruments` readback inside one `batch_execute`.** The primary
  bridge-race (set/get validation failing with `"instrument not found"` right after
  `apply_example_patch`) was fixed by adding a synchronous `alive_instruments` mirror on
  `SynthSession`. What's left: `list_instruments`, `get_instrument_info`, and other readers that
  pull `volume`/`pan`/`mute` etc. still read `EngineState::instrument_snapshots`, which is only
  rebuilt on the audio thread. So `set_instrument_volume(5, 0.8)` followed by `list_instruments`
  in the same batch still reports the old `volume: 1.0` until the audio thread ticks (~one buffer,
  5–10 ms at 44.1 kHz / 256 samples). The audio itself is already correct; only the metadata is
  stale. Fix by layering write-through onto the `set_instrument_*` handlers in `session.rs` — patch
  `instrument_snapshots[i].volume` (etc.) under the same write lock as the queued `EngineCommand`.
  Maintenance burden: every new `set_*` tool needs to remember the write-through.

--- 

## ★ Description fields on user-facing entities (for AI context)

Add an optional free-text `description: String` field on user-facing entities so the user (or AI) can record
*intent* — what a thing is for, not just what it is. The structural data (graph, notes, params) already tells you
*what*; descriptions capture the *why*, which is otherwise unrecoverable.

**Every instance-level description must be readable AND writable via MCP**, so AI can both read existing intent
and populate descriptions after analysis. Read access is typically free (include the field in the existing
resource/getter response — `get_instrument_info`, `get_song_info`, `list_patterns`, `list_tracks`,
`list_samples`, `get_awe_state`); write access needs a dedicated setter tool routed through
`bridge.rs` → `mcp_bridge.rs` → `server.rs`.

**Every instance-level description must also be editable from the GUI** — never MCP-only. The user must be
able to write the same intent they'd ask AI to write. Each entity gets an inline-editable text field (header
band, properties dialog, or context-menu "Edit description…" action) wired through the same session/bridge
path so MCP and GUI writes share validation and undo. Type-level descriptors (`ModuleDescriptor`,
`ParameterDescriptor`, `PortDescriptor`) are already read-exposed via `get_module_type_info` /
`list_module_types` / `search_modules` and are hardcoded in source — no GUI edit and no MCP write tool there.

### Current status of description fields

Phase 1 (`InstrumentState`/`Patch`/`AwePresetFile`), Phase 2 (`Song`/`Pattern`/`SequencerTrack`/`Sample`)
and **Phase 3 (per-module-instance descriptions)** are shipped — every entity below has its `description`
field plus MCP read + write tool and GUI editing. The only remaining items are the cross-cutting polish
tasks at the end of this section (diagnostics surfacing + tooltips).

| Entity                       | Field exists?                        | MCP read | MCP write       |
|------------------------------|--------------------------------------|----------|-----------------|
| `InstrumentState`            | ✅ `patch.rs:697`                     | ✅        | ✅               |
| `Patch`                      | ✅ `patch.rs:130, 307` (Option)       | ✅        | ✅               |
| `AwePresetFile`              | ✅ `patch.rs:792`                     | ✅        | ✅               |
| `Song`                       | ✅                                    | ✅        | ✅               |
| `Pattern`                    | ✅                                    | ✅        | ✅               |
| `SequencerTrack`             | ✅                                    | ✅        | ✅               |
| `Sample` entry (`SampleMeta`)| ✅                                    | ✅        | ✅               |
| Module *instance* (in patch) | ✅ `patch.rs:144` (separate from type) | ✅        | ✅               |
| `ModuleDescriptor` (type)    | ✅ `module_traits.rs:869`             | ✅        | n/a (hardcoded) |
| `ParameterDescriptor` (type) | ✅ `module_traits.rs:586`             | ✅        | n/a             |
| `PortDescriptor` (type)      | ✅                                    | ✅        | n/a             |
| `ChoiceOption` (type)        | ✅ `module_traits.rs:557` (Option)    | partial  | n/a             |

### Phase 3 — per-module-instance notes (different concept from type docs) — SHIPPED

Shipped in `9c9dd4b` (data model + engine mirror + persistence), `91adce0` (MCP read + write),
`fc5bd4b` (GUI editing + info popup). Bridge test `set_module_description_round_trips_via_bridge`.

- [x] Add per-instance `description: String` on placed modules in a patch (e.g. annotate "this LFO is the
  wobble modulator" on a specific `lfo-1` instance). Distinct from `ModuleDescriptor.description` which
  documents the module *type* and is shared across all instances. **Done** — `ModuleState.description`
  (`patch.rs:144`).
- [x] Surface in `get_module_info` (MCP read); add `set_module_description(instrument_id, module_id, description)`
  MCP tool (MCP write). Accept `""` to clear. **Done** — `mcp_bridge.rs:910` (clears on `""`, rejects
  over-length + unknown module id).
- [x] Editable from GUI (module header context menu). **Done** — "Edit description" popup + read-only info
  popup wired through `module_description_actions` → `session.set_module_description`.

#### Persistence (must round-trip on save/load)

Module-instance descriptions **must** survive serialization, both at the project and standalone-patch
level — otherwise AI-applied notes silently vanish on save/reload. Same pattern as
`Patch.description` and the planned color persistence below.

- [x] **Project save** — every module instance's description persisted inside its containing patch
  in the project JSON. **Done** — `ModuleState.description` with `#[serde(default,
  skip_serializing_if = "String::is_empty")]`, so empty stays byte-identical/schema-valid.
- [x] **Standalone patch save** — the per-instance description travels with the .json patch file
  when the user invokes "Save Patch…". **Done** — same `ModuleState` struct serializes for standalone
  patches.
- [x] **Project / patch load → engine mirror** — on load, copy each saved module description into
  the engine's runtime mirror so subsequent MCP reads see what was loaded, not stale defaults.
  **Done** — engine `set_module_description` (`synth_engine.rs` / `instrument.rs`).
- [x] **No partial states** — **Done** — data + MCP + GUI + save/load all landed together
  (`9c9dd4b`→`91adce0`→`fc5bd4b`); no half-wired ship.

### Cross-cutting

- [ ] Include all instance-level descriptions in `get_graph_diagnostics` / `analyze_note` output so AI sees
  intent alongside structure — **still open** (module-instance descriptions not surfaced there yet)
- [ ] Surface descriptions as tooltips on the corresponding GUI elements — **still open** for the
  module-instance description (the read-only info popup shows it, but not a hover tooltip)
- [x] Decide on max length (suggest 500 chars soft, 2000 hard). **Done** — `MAX_MODULE_DESCRIPTION_LEN =
  2000` hard cap enforced at the bridge (`mcp_bridge.rs:49`); 500 soft is advisory only.
- [x] Persistence format: inline in the existing JSON containers (no sidecar files). **Done** — inline on
  `ModuleState` in the patch JSON.

---

## ★ Color fields via MCP

Color fields already exist on several entities (Patch, Instrument, Group, SequencerTrack), but
most have **no MCP setter** — AI can build a song but can't paint the strips/tracks to make the
arrangement visually scannable. Parallel structure to the description roadmap above: read on the
existing getter response, write via a dedicated setter routed through
`bridge.rs` → `mcp_bridge.rs` → `server.rs`. Color is also already GUI-editable in most cases.
Track color is the only one shipped via MCP so far (`set_track_color`); the rest are blocked on
engine-ownership refactors noted below.

### Current status of color fields

| Entity            | Field exists?                        | MCP read | MCP write |
|-------------------|--------------------------------------|----------|-----------|
| `InstrumentState` | ✅ `patch.rs` `Option<HexColor>`      | ✅        | ✅         |
| `Patch`           | ✅ `patch.rs` `Option<HexColor>`      | ✅        | ✅         |
| `Group`           | ✅ `patch.rs` `Option<HexColor>`      | ❌        | ❌ (dropped) |
| `SequencerTrack`  | ✅ in song JSON `{r, g, b}` per track | ✅        | ✅         |

### Work to do

- [x] Surface color on `get_instrument_info` / `list_instruments` (MCP read); add
  `set_instrument_color(instrument_id, color)` MCP tool. Accept `"#RRGGBB"` / `"#RRGGBBAA"` and
  `""` to clear back to "auto" / default. **Done** — made engine-owned like `description`
  (`EngineCommand::SetInstrumentColor` + `Instrument.color` runtime field + snapshot + save/load
  mirror). `8598a6d`.
- [x] Surface patch color on the same getters as a separate `patch_color` field, mirroring how
  `patch_description` is exposed alongside `description`. Add `set_patch_color` MCP tool.
  **Done** — added `Patch.color` field + engine `patch_color` mirror + round-trip (incl. standalone
  "Save Patch…"). `5ca35a5`.
- [~] ~~Surface group color~~ — **dropped**: the group concept is slated to be removed/reworked, so
  wiring a GUI-only MCP side-channel for it (groups have no engine path; only live in `PatchEditor`)
  is not worth it. Revisit only if groups survive the rework.
- [ ] Decide whether AI-friendly named palettes are useful (`"warm-orange"`, `"cool-blue"`) on top of
  raw hex — same pattern as `set_awe_preset` vs `set_awe_parameter`. Out of scope for v1; raw hex
  is enough.

### Persistence (must round-trip on save/load)

Color writes via MCP **must** survive serialization, both at the project and standalone-patch level
— otherwise AI-applied colors silently vanish on save/reload. This is the same architectural
pattern used for `Patch.description` (runtime mirror in the engine, project load copies in, project
save reads back).

- [x] **Project save** — instrument + patch colors persisted in the project JSON. The engine-side
  runtime mirror (`Instrument.color` / `Instrument.patch_color`) is read back into the snapshot at
  save time and `snapshot_to_instrument_state` writes `InstrumentState.color` / `Patch.color`.
  (`SequencerTrack` color was already done.) Group color dropped.
- [x] **Standalone patch save** — `Patch.color` travels with the .json patch file: `create_patch_from_editor`
  pulls `patch_color` from the engine snapshot when "Save Patch…" is invoked.
- [x] **Project load → engine mirror** — on load, each saved instrument/patch color is pushed into the
  engine runtime mirror (`set_instrument_color` / `set_patch_color`) so subsequent MCP reads see what
  was loaded, not stale defaults.
- [x] **No partial states** — both setters ship with their save+load paths wired (verified by the
  schema round-trip + full test suite). Group color was dropped rather than shipped half-wired.

### Use case (motivation)

When AI builds a multi-instrument song via MCP, every track defaults to the same color (e.g. all
tracks in the just-built sidechain demo render as `{r:100, g:100, b:255}`). With color setters AI
can make the arrangement self-documenting at a glance — e.g. red kick, blue pad, green bass.

---

## 1. Core Usability & Workflow

### 1.2 MIDI learn

- [ ] Map MIDI CC to any module parameter via right-click → "MIDI Learn"
- [ ] Visual indicator on mapped parameters
- [ ] Save/load MIDI mappings with patch or settings

### 1.3 Module presets

- [ ] Save/load parameter presets per module type (not the whole patch)
- [ ] Preset browser in module context menu or header
- [ ] Ship default presets for common module types

### 1.5 Settings & utilities

- [ ] Add Browse button in Settings dialog to change patches directory
- [ ] Extract `magnitude_to_normalized_db()` into `synth_core` or `synth_dsp` — the
  `20·log10(mag)` → normalized-dB pattern is repeated in 5+ locations (`synth_osc/src/sender.rs`,
  `mcp_bridge.rs`, `audio/analysis.rs`, two in `gui/widgets/meter.rs`) with no shared helper.
- [ ] Add ergonomic constructor `SamplerParam::sample_select(u64) -> Self` (or `Param::sample_select`) in
  `synth_core/src/params/sampler.rs` — 4 call sites currently spell out
  `Param::Sampler(SamplerParam::SampleSelect(SampleId(id)))` verbatim (`session.rs`, `mcp_bridge.rs`,
  `gui/patch_editor.rs`, `gui/egui_backend.rs`)
- [ ] Add `param_sample_id(name, id)` to `ModuleStateBuilder` in `pertylizer/src/patch.rs` for symmetry with
  `param_f` / `param_i` / `param_b` / `param_choice` (no current callers — for API completeness)
- [ ] **Bundle piano-roll coordinate plumbing into a `PianoRollCoords` struct.**
  `handle_piano_roll_interaction` (`gui/sequencer/mod.rs`) currently takes 17 parameters and
  `draw_arrangement` takes 9. Four of those (`x_to_tick`, `y_to_pitch`, `tick_to_x`, `pitch_to_y`) plus
  `view_pitch_min`/`view_pitch_max`/`note_row_height` are one coherent concept — a piano-roll coordinate
  transform. Extracting a struct collapses 7 args → 1 and removes the need to thread `note_row_height`
  through `note_at_pos` separately.
- [ ] **Context-struct refactor to finish decomposing `draw_piano_roll` / `draw_arrangement`.**
  The GUI cleanup branch (`cleanup/gui-dead-duplicate-code`) fully decomposed `App::ui`
  (~1900 → ~400 lines, ~16 methods) and extracted the *low-parameter* sub-sections of the two
  sequencer god-functions: `draw_arrangement_toolbar` (3 params) and
  `draw_piano_roll_selection_inspector` (5 params). The remaining sections —
  `draw_piano_roll`'s two toolbar rows + note-grid painter, and `draw_arrangement`'s track-header
  panel + timeline painter + ~330-line context menu — each touch 6–8 of the *same* locals
  (`data`, `song`, `view_state`, `handle`, `undo_manager`, `instruments`). Mechanical extraction
  there trips `clippy::too_many_arguments` and would force `#[allow]` on every helper, trading one
  long function for many parameter-heavy ones — not a clear win. The clean path is to bundle the
  shared state into context structs, e.g.
  `struct PianoRollCtx<'a> { data: &'a PianoRollData, song: &'a Arc<RwLock<Song>>,
  view_state: &'a mut SequencerViewState, handle: &'a mut EngineHandle,
  undo_manager: &'a mut UndoManager, instruments: &'a [InstrumentUiState] }` (and an analogous
  `ArrangementCtx<'a>`), thread one `&mut ctx` into each extracted sub-function, then split the
  bodies. This is a design-level change (borrow-splitting the `&mut` fields across sub-calls needs
  care) and has no GUI behaviour tests to catch regressions, so it warrants its own focused
  session rather than being rushed. Once the ctx structs exist, the toolbar rows / grid / header /
  timeline / context-menu sections drop out as 1–2-arg methods. Relatedly subsumes the
  `PianoRollCoords` item above (coords can live on `PianoRollCtx`).
- [ ] **Deduplicate "Set Length" write+undo in the arrangement context menu.**
  `gui/sequencer/mod.rs` "Set Length…" submenu has the same ~22-line "read old length → write new → push
  `SetPatternLength` undo" block in two places: the free-input `DragValue` + Apply branch and the preset
  buttons loop. Extract `fn apply_pattern_length(song, undo_manager, pat_id, new_len)`.
- [ ] **Unify `SeqInstrumentId` ↔ `InstrumentId` raw conversions.**
  ~9 sites in `gui/sequencer/mod.rs` do `inst.id.0 == seq_id.0 as u64` and a few do the reverse
  `SeqInstrumentId::new(inst.id.0 as u16)` (lossy `u64 → u16` cast is silent). Pick one of: add
  `impl From<SeqInstrumentId> for InstrumentId` + `TryFrom<InstrumentId> for SeqInstrumentId`, or add a
  single `find_instrument_by_seq_id(&[InstrumentUiState], SeqInstrumentId) -> Option<&InstrumentUiState>`
  helper. (A `build_instrument_colour_cache` helper already exists for the hot-path lookups.)
- [ ] **Cache `Song::calculate_length()` instead of recomputing per tick on the audio
  thread.** `SequencerEngine::update_cached_state` (`crates/synth_engine/src/sequencer_engine.rs`
  around line 306/328) calls `song.calculate_length()` once per tick during playback. After
  the E1 migration (`Song.patterns: Vec<Pattern>`) that cost is `O(arrangement × patterns)`
  per tick — ~50 placements × 22 patterns × 1920 ticks/s ≈ 2.1M linear-find ops/s.
  Still well within audio-thread budget at the current scale (no allocs, no locks, no panics),
  but it's recomputing a value that only changes on structural mutation. Recommended fix:
  cache `cached_song_length: Tick` on `SequencerEngine`, refresh on `play()` / `seek()` and
  the structural-change command set; drop the per-tick recompute. Pre-existing with BTreeMap
  too — surfaced as a code-review finding during the E1 commit (2026-05-21).

### 1.6 Workflow quality of life

- [ ] A/B comparison — quick-switch between two patch versions to compare sound
- [ ] Parameter locking — lock parameters to prevent accidental changes
- [ ] Favorite modules — quick access to frequently used modules in "Add Module"

---

## 2. Sequencer & Arrangement

### 2.1 Tempo automation

- [ ] **Expose + edit the tempo map** (`Song::set_tempo_at` / `tempo_changes`): MCP tools
  and a GUI tempo-track/curve editor, with interpolation between adjacent tempo points
  (accelerando/ritardando ramps) rather than the current step changes. The engine tempo
  map already exists and the GUI uses `set_tempo_at`, but MCP only exposes `set_song_tempo`
  (global default), there is no tempo-map editor, and changes are step-only. This is the real
  "tempo automation" feature — built on the tempo map, not a generic automation lane. (The old
  `Global(Tempo)` automation lane was removed 2026-06-01: tempo changes the playback time grid
  itself, so it can't be a per-block value, and a lane would be a second source of truth competing
  with the tempo map.)

### 2.2 Section markers

- [ ] Verse, chorus, bridge labels in the arrangement

### 2.3 Macro controllers

- [ ] Map multiple parameters to a single macro knob for live performance

### 2.4 MIDI export

- [ ] Export sequences as .mid files

### 2.5 Track reorder via drag-handle

- [ ] Replace (or complement) the current ↑/↓ arrow buttons in the track header with a drag-handle
  (e.g. `ri::DRAG_MOVE_2_LINE`) on the left edge of each row. Drag should snap the row vertically to the
  nearest neighbour while dragging, then commit on release via `Song::reorder_track`. The arrow buttons
  shipped because they are simpler and robust, but drag-handle reorder is the DAW convention.

### 2.6 Pattern looping within placement length (future)

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
  `gui/sequencer/mod.rs` (mini-note loop, near the `inst_color_cache` use) should repeat the miniature
  across the placement's `effective_length / pattern.length` iterations, so the user sees what they hear.
- [ ] **Add a toggle on `PatternPlacement`** (`loop_mode: PlacementLoopMode { Clip, Repeat }`, default
  `Repeat` to match DAW expectations). Surface in the placement context menu and in the right-edge
  resize-grab tooltip so the user can choose per placement. Migration of older songs: default existing
  placements to `Clip` so behaviour is preserved, or `Repeat` if we accept a one-time semantic change.

---

## 3. Sound Design — Expanded Capabilities

### 3.1 Sample & wavetable import

- [ ] Sample import — load .wav files as oscillator source or in granular synth
- [ ] Wavetable import — load custom wavetables (Serum format, single-cycle .wav)

### 3.2 Alternative tunings

- [ ] Scala file (.scl) support, just intonation, microtonality

### 3.3 Expression & articulation

**History.** The staged note-expression roadmap (`plans/note-expression-roadmap.md`,
now retired) shipped almost in full: generic sequencer→module automation (Phase A1
bugfix — the 6 dead FilterCutoff/ADSR macros now sound; A2 — `AutomationTarget::Module`),
per-note legato/glide (B), per-note vibrato + expression block (C), the A1/A2 deferred
cross-cutting follow-ups (Track F — F2/F3/F4 resolved), and export-robustness tooling
(Parallel track P). See `docs/history.md` (v0.292.0–0.297.0). Phase D's *routing* half
was delivered by the mixer/return-bus work (per-instrument/return/master effect chains,
any of which can carry a `Filter`). What remains of that roadmap is the three items below
plus the Note Processors plan; the roadmap doc itself was deleted once these were
extracted here.

**Remaining open work from the retired roadmap:**

- [ ] **Phase D residual — automate master/return effect params.** A `Filter` can be
  placed on the master or a return bus and set today (`set_master_effect_parameter` /
  `set_return_effect_parameter`), but its cutoff **cannot be swept by an automation
  lane**: `AutomationTarget::Module` resolves only against instrument-owned modules
  (`synth_engine.rs:~3293`, `instruments.iter_mut().find(...)`). Add a target variant
  (e.g. `AutomationTarget::{MasterEffect, ReturnEffect}` keyed by slot + `param_id`)
  dispatched through the same override layer as A2. Delivers exact *shared* SID-style
  filter sweeps. **S** task — build only when a tune genuinely needs a shared (not
  per-instrument) automated sweep; per-instrument sweeps are already covered by A2.
- [ ] **`ParamId(Arc<str>)` off-thread drop (F1 residual).** Cloning a `Module`
  automation target is now alloc-free, but the engine's cached clone can become the
  last `Arc` reference and so *drop* (free) on the audio thread — but only if the
  source lane was removed mid-playback. Strict improvement over the prior `String`
  (which freed on every drop). Full fix: route cleared/replaced targets through the
  engine's `return_producer` off-thread drop channel. Low priority (bounded, rare).

**Note Processors (generative articulation): SHIPPED in v0.311.0 except the GUI.**
The engine and every processor landed per `plans/note-processors-plan.md`: arpeggiator
(flagship), timed-repeat ornaments (flam/drag/ruff/roll/grace), chord + strum,
scale-quantize + humanize, the MCP surface, and save/load persistence. The one
remaining item is **NP6 — the per-track rack + per-note ornament GUI**, deferred for
interactive egui work with the user (not headless-testable). Verified still absent from
`crates/pertylizer/src/gui/` as of 2026-06-12.

**Iceboxed — the rest of Phase E (build on demand only).** The expensive,
narrow-audience remainder of the old north-star phase. No plan doc; pick up only when a
concrete need appears:

- [ ] MPE support — MIDI Polyphonic Expression for per-note pitch bend, pressure, slide
  (needs MPE hardware to be worth it; the Phase C expression block already defaults to
  the MPE dimension set — bend/pressure/timbre/velocity/release-velocity — so input
  mapping is the missing piece).
- [ ] Polyphonic aftertouch routing to module parameters.
- [ ] Per-note hand-drawn expression curves + the **piano-roll per-note curve editor**
  (the real cost center that gated the whole phase).
- [ ] Per-note **spatial via AWE** — primitive 1 with an AWE room param as the target
  (per-note position in the simulated room). Genuine differentiator; no equivalent in
  other synths — worth keeping on the list even though it is niche.

### 3.5 Polyphony settings

- [ ] Voice stealing mode selection (oldest, quietest, none) — engine + persistence done, GUI selector
  not yet added (`stealing_strategy` field is read but has no ComboBox; `allocation_mode` already does).
- [ ] Unison detune/spread controls

---

## 4. UI & Visual Polish

### 4.1 Improve module knobs

- [ ] Better visual design — gradient fill, shadow, tick marks, value tooltip
- [ ] Consistent sizing across module types
- [ ] Arc-style knobs with colored fill showing current value

### 4.2 Redesign instrument list

- [ ] Tabbed interface, mixer-style vertical strips, or collapsible panels

### 4.3 Module Groups — Phase 2–3

- [ ] Phase 2: Template variants (parameter presets with remap)
- [ ] Phase 3: Probes data pipeline (ringbuffers, audio-thread safe collection)
- [ ] Phase 3: Probe rendering (waveform/spectrum/meter) with PortType-based signal type
- [ ] Phase 3: Polyphony probes = sum of voices (mixdown)

### 4.4 Mod Matrix routing visibility

When a module (e.g. `env-2`, `lfo-1`) is referenced only via Mod Matrix slots — not via cables —
it *looks* unused: no visible cable in the patch-editor graph. Header badges and MCP surfacing
shipped in v0.289.0 (`get_mod_matrix_routings`, virtual `"matrix"` port on `list_modules`, header
arrow badge with tooltip). What remains is the in-canvas visual + navigation polish:

- [ ] **GUI: ghost cables for Mod Matrix routings.** In the cable view, draw faint dashed lines
  (different colour) from matrix sources to their destinations (e.g. `env-2` → `flt-1.cutoff`),
  togglable in the View menu. Both routing paradigms become visible in one canvas.
- [ ] **GUI (optional polish): click a Mod Matrix slot source/dest to focus + pulse that module.**
  Two-way matrix↔graph navigation — was Phase 4 of the (now-completed, removed)
  mod-matrix-routing-visibility plan; Phases 1–3 (MCP surface, framed zone, header badges)
  shipped in v0.289.0.
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

---

## 5. Template Library & Presets

### 5.1 Template library

- [ ] Add patch template directory and `Save Patch as Template` action
- [ ] Add Patch Template browser to load patch templates
- [ ] Support optional `license` and `min_app_version` metadata in group templates

### 5.2 Preset sharing

- [ ] Community format for sharing patches online

---

## 6. AI & Automation

### 6.1 MCP & AI Interaction

Tier-0 music-analysis tools shipped in v0.276.0 (`analyze_harmony`,
`analyze_mix_bus`, `analyze_section`); Tier-1 follow-ups for drum-track
filtering and per-track contribution breakdown shipped in v0.277.0. The
roadmap doc (`docs/mcp-music-tools-plan.md`) was closed and deleted on
2026-05-17 — remaining work for that roadmap lives below.

- [ ] Tier 3: `compare_to_reference`, `compare_patterns`, `compare_patches`,
  `humanize_notes`, `generate_variation`, `analyze_track`, `get_mix_meters`.
- [ ] Enable AI to "play freely" via MCP to autonomously generate complete songs and arrangements
- [ ] Implement real-time parameter interpolation (gliding) to allow smoother AI-driven sound design

### 6.2 Technical follow-ups from the MCP music tools plan

Moved from §7 of the (now-deleted) MCP music tools plan on 2026-05-17 when the plan was
closed. The four shipped follow-ups (`OfflineEngineSession` for arrangement renders,
rayon-parallel per-track renders, `Song::tracks_mut` / `set_solo_only` helpers, embed
`MixBusMetrics` in `TrackContribution`) are documented in `docs/history.md` against their
ship dates.

- [ ] **`HarmonyScope` enum to fix `analyze_song_harmony` argument sprawl.** `analyze_song_harmony`
  takes 8 arguments and carries `#[allow(clippy::too_many_arguments)]`. Two of them
  (`exclude_drums`, `exclude_track_ids`) are only meaningful in arrangement scope; pattern scope
  currently emits a runtime warning if they're passed. Introduce
  `enum HarmonyScope { Pattern { pattern_id: PatternId }, Arrangement { start: Option<u64>,
  end: Option<u64>, exclude_drums: bool, exclude_track_ids: HashSet<TrackId> } }` at the bridge
  boundary; the flat `AnalyzeHarmonyParam` (JSON-schema layer requires it) maps into the enum
  inside the bridge. The runtime "ignored in pattern scope" warning becomes a compile-time
  impossibility and the `#[allow]` disappears. Touches `synth_mcp::bridge`, the bridge impl,
  and the arrangement-vs-pattern branch in `analyze_song_harmony`. Medium impact — pick up
  when next touching the harmony analyzer.
- [ ] **`synth_sequencer::shared_song(Song) -> Arc<RwLock<Song>>` constructor.** Grep finds ~9 sites
  that wrap a `Song` in `Arc::new(parking_lot::RwLock::new(...))` verbatim
  (`crates/pertylizer/src/audio/export.rs:214`, `main.rs:83`, `mcp_shared.rs:56`, sequencer
  tests, the per-track render loop, etc.). Strictly cosmetic; not on any hot path. Pick up
  as a drive-by when next touching one of those sites.
- [ ] **`OfflineNoteSession` — engine reuse across patch-sweep steps.** `analyze_instrument_range_impl`
  and `analyze_velocity_response_impl` (`crates/pertylizer/src/mcp_bridge.rs`) call
  `analyze_rendered_note` once per swept value; each call goes through
  `audio::preview::render_note_to_buffer`, spins up a fresh `SynthEngine`, and reloads the
  instrument's module graph + sample data. For a 60-note semitone-step sweep that's 60 fresh
  engines; for the default 8-step velocity sweep that's 8. Mirror §7.1's `OfflineEngineSession`
  — wrapper takes `SynthSession` + `SharedSampleLibrary` + `InstrumentId` at construction,
  builds the engine + loads the patch + samples once, then exposes
  `render(note, velocity, duration_ms, tail_ms) -> RenderedNote` per call. Reproduce the
  voice-bleed drain between renders (same problem as §7.1). Determinism tests would mirror
  `tests/arrangement_render_determinism.rs::session_render_range_is_bit_exact_across_three_calls`.
  After session-reuse lands, parallelize the sweep target vector with `par_iter` for a 2-4×
  speedup on top (same sequence as §7.1 → §7.2).
- [ ] **Static `#[schemars(range(...))]` on fixed-range numeric MCP fields.** Module/AWE *parameter*
  values are now validated at the bridge boundary against the descriptor's `ValueRange`
  (`ParameterDescriptor::validate_f32`), but the *globally* fixed numeric tool fields — MIDI note
  (0–127), velocity (0–127), MIDI channel (1–16), LFO index (1–4) — still expose only a plain
  `u8`/`f32` in their `JsonSchema` (prose-only bounds in `#[schemars(description=...)]`). They are
  enforced at runtime via the `validate_*` helpers in `synth_mcp::server` but a schema-aware client
  sees no `minimum`/`maximum`. Add `#[schemars(range(min = …, max = …))]` to those fields in the
  param structs (`crates/synth_mcp/src/server.rs`) so the constraint is machine-readable. Verify the
  attribute syntax/feature against the pinned `schemars` 1.2 first (it differs from 0.8). Low risk,
  small; skipped during the 2026-05-27 validation pass for scope.
- [ ] **Uniform machine-readable bounds on `synth_core` newtypes.** Newtype clamping is inconsistent:
  `NormalizedValue`/`BipolarValue`/`Velocity`/`Phase` clamp in `new()`, but `Hertz`/`Gain`/`Cents`/
  `Semitones` do not, and there is no uniform `const RANGE: ValueRange` on any newtype — so a shared
  "spec → (schema | validation)" abstraction can only read bounds from the per-module
  `ParameterDescriptor`, never from the type. `Param::with_f32` (`crates/synth_core/src/params/*.rs`)
  then re-clamps ad hoc and sometimes hardcodes a range that duplicates the descriptor (e.g. the
  2026-05-27 `Detune::with_f32` `-100..100` clamp mirrors `oscillator.rs:308` by hand). Consider a
  `BoundedNewtype` trait (or `const RANGE`) so bounds live on the type and descriptors/`with_f32`
  derive from it. Larger, cross-cutting refactor — plan separately.

---

## 7. AWE Improvements

Findings and concrete ideas: `docs/AWE-Improvement-Findings.md`.

### 7.0 AWE acoustic engine — prioritized plan

#### Phase 2 — Medium complexity

- [ ] **7. Per-surface materials** — `MaterialConfig { floor, walls, ceiling }` instead of single global `Material`, ISM
  uses correct material per reflection
- [ ] **8. Second-order reflections** — extend ISM from 6 to ~30 taps (configurable `ReflectionOrder(u8)` 1–3)
- [ ] **10. Resonant objects** — sympathetic resonance from objects in the room (strings, membranes, plates, Helmholtz
  cavities, loose panels, chimes), implemented as bandpass + feedback at object frequency
- [ ] **12. Doppler effect** — track radial velocity between source/listener, shift pitch via variable delay read speed:
  `ratio = v_sound / (v_sound + v_radial)`

### 7.1 Rework room visualization

- [ ] Redesign the 3D isometric room rendering
- [ ] Improve animations (sound rings, reflection paths)
- [ ] Better visual clarity for room shape and dimensions

### 7.2 Differentiate effects more clearly

- [ ] Each material/effect should have more distinct visual representation
- [ ] Color-coded zones, animated textures per material, spectral visualization

---

## 8. Visualizer & OSC

### 8.1 OSC control & connectivity

- [ ] OSC enable/disable toggle in Pertylizer settings GUI
- [ ] `/viz/` OSC control endpoints (effect select, param set, scene load)
- [ ] OSC `/viz/theme/select` control endpoint
- [ ] OSC parameter tweaking — live control of intensity, speed, scale per effect
- [ ] Support connecting multiple OSC clients simultaneously

### 8.2 Post-processing & shaders

- [ ] Chromatic aberration — intensity scales with RMS level
- [ ] Glitch/distortion effect — triggered by CPU spikes or spectral flux
- [ ] Kaleidoscope mode — radial scene mirroring (configurable segment count)
- [ ] CRT/VHS filter — scanlines, color bleed, static noise
- [ ] Motion blur — strength synced to tempo

### 8.3 Multi-effect layering

- [ ] Show 2–3 effects simultaneously instead of one at a time
- [ ] Per-instrument visual layers — each instrument gets its own color/effect layer
- [ ] Blending modes between layers (additive, multiply, screen)
- [ ] Layer opacity control via OSC

### 8.4 Reactive environment

- [ ] Skybox that reacts to music — stars pulse with RMS, clouds move with tempo
- [ ] Reactive ground — ripples on note-on, cracks on bass hits
- [ ] Fog/mist density driven by reverb level or sustain
- [ ] Day/night cycle driven by song position
- [ ] Weather effects — rain on high spectral flux, lightning on transients

### 8.5 Advanced simulations

- [ ] Swarm/flock simulation — particles flock or scatter based on dynamics
- [ ] Cloth simulation — fabric that billows and ripples with FFT energy
- [ ] Text/typography — display song title, BPM, key in stylized 3D text
- [ ] AWE spatialization — visualize sound source position in 3D space

### 8.6 Video export

- [ ] Video recording — render to MP4 or image sequence

---

## 9. Advanced / Long-term

### 9.1 Audio tracks

- [ ] Import and arrange audio files, not just synth tracks

### 9.2 Audio recording

- [ ] Record external audio via cpal input

### 9.3 Clip launching

- [ ] Ableton-style live mode with follow actions

### 9.4 Plugin export

- [ ] Export instruments as VST3/CLAP plugins
