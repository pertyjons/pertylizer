# TODO - Pertylizer

## 0. Known Bugs

### 0.1 Misc findings

- [ ] **★ HIGH: `SequencerTrack.pan` and `.volume` are stored but never
  applied to audio output.** Discovered while migrating `track.pan` to
  `BipolarValue` (commit `61e46e1`). The audio thread (`sequencer_engine.rs`)
  only reads `track.is_audible(any_solo)` for mute/solo gating and routes
  notes via `track.instrument`. Pan and volume per *track* are then ignored
  — the only pan/volume that actually reaches the bus is at the *instrument*
  level (`InstrumentState.pan`/`.volume` → `Gain::from_pan` in
  `instrument.rs:1295`). The GUI slider, MCP `set_track_pan`/`set_track_volume`,
  and the `track.volume`/`.pan` storage round-trip cleanly through save/load,
  but the values do nothing audibly. Likely fix: in `sequencer_engine.rs`
  around line ~358, multiply the dispatched note's velocity by
  `track.volume.as_f32()` and apply `Gain::from_pan(track.pan)` either by
  attenuating the routed note's velocity asymmetrically or — cleaner — by
  applying a per-track stereo gain on the instrument's output bus before
  summing. Worth deciding whether track-pan should override or compose with
  instrument-pan (probably compose: `final_pan = clamp(inst_pan + track_pan, -1, 1)`).
  Also: **`SequencerTrack.mode: TrackMode` is dead** — the enum has a single
  `Polyphonic` variant and is never read anywhere; either remove it or land
  the planned `Mono` / `Legato` / `Unison` variants and wire them into voice
  allocation per track (analogous to `Instrument`'s `AllocationMode`).
  **Audit follow-up:** sweep every other domain struct for the same pattern.
  Candidates worth checking: `Pattern.*` (row_resolution, automation lanes —
  do they all reach the engine?), `Instrument.*` (some fields like
  `velocity_amp_sensitivity` / `velocity_filter_sensitivity` were added but
  may still be unwired), `PatternPlacement.length_override` (added in v0.281,
  confirmed used), AWE-side material/room fields. A small `cargo run --bin
  audit_dormant_fields` style check — or just a grep for every `pub` field on
  a domain struct and a hand-trace into the audio path — would surface these.

  **Design findings (2026-05-27, pre-implementation — deferred pending decision).**
  Traced the full path before touching code. Notes route to *instruments* by
  `SeqInstrumentId`; the instrument is the audio-producing/mixing entity, and its
  `volume`/`pan` are applied **post-effect-chain** at the output mix
  (`Instrument::stereo_gain`, `instrument.rs:1294`; consumed in the mix loop
  `instrument.rs:1280`). Track vol/pan never enter this path. Routing happens in
  `route_sequencer_events` (`synth_engine.rs:2415`); the `Parameter` arm
  (`:2469`) already sets instrument vol/pan via `set_volume`/`set_pan`, so that
  is the natural write point. Per-voice panning already exists in the voice-sum
  loop (`instrument.rs:1219`, used by the spatial bank), so a per-voice path is
  *technically* available too.

  - **How real DAWs do it (the consistent model).** Channel strip = instrument →
    insert FX → **volume fader (post-FX)** → pan → sends → master. Crucially:
    (1) track *is* the channel — there is no separate track-volume vs
    instrument-volume, it's one fader; (2) the fader is post-FX (pulling it down
    dims the reverb tail); (3) **volume ≠ velocity** — velocity is per-note (sets
    timbre/attack at note-on, already correct here), the fader is a continuous
    *post-synth* channel control. Automation is not a separate math layer: it
    writes to the *same control, at the same point in the signal path*, that the
    user would turn by hand. This gives one rule covering all automation targets.
  - **Implication: TODO suggestion (a) above is the wrong model.** Multiplying the
    dispatched note's velocity by `track.volume` would alter timbre via velocity
    sensitivity. Track volume is a fader, not a velocity scaler.
  - **The real ambiguity is N tracks → 1 instrument**, not "one pattern on two
    tracks". The latter (two placements of pattern P on tracks A/B with different
    instruments) is legitimate *layering* and already works — each placement
    routes through its own track's instrument. The hard case is two tracks
    pointing at the *same* instrument: doubled notes + undefined "which fader".
    The DAW channel-strip convention (1 track ↔ 1 instrument) *removes* this case
    by construction — layering uses two instruments, not a shared one.
  - **Three candidate models.**
    - **A — per-instrument post-FX gain (recommended).** Compose track vol/pan
      with instrument vol/pan in `stereo_gain()`:
      `gain = inst.vol × track.vol`, `pan = clamp(inst.pan + track.pan, -1, 1)`.
      All Volume/Pan automation (track *and* instrument) writes to this same
      post-FX stage → trivially consistent, sets up §0.1 automation item and §2.1
      tempo. DAW-correct. Imperfect only when tracks share one instrument
      (define as "last routed track wins per block").
    - **B — per-voice gain/pan.** Carry track vol/pan in `SequencerEvent::NoteOn`,
      apply per voice (like the spatial pan). Correct under instrument-sharing,
      *but* lands **pre-effect-chain** (track volume changes reverb send level)
      and won't affect already-ringing voices on a live fader move — i.e. *not*
      how a DAW fader behaves. Rejected as inconsistent with the fader model.
    - **C — full per-track output bus.** A real channel bus per track. Correct in
      all cases but needs per-voice track-tagging to split a shared instrument's
      output across buses, plus duplicated effect chains. Large rework, not
      justified now.
  - **Open decision (why this is deferred).** Whether to also do the deeper
    *struct merge* — collapse `SequencerTrack` + `Instrument` into one channel
    strip (1 instrument per track). That would make per-not `note.instrument`
    redundant (track instrument already overrides it via
    `effective_instrument = track.instrument.unwrap_or(note.instrument)`,
    `sequencer_engine.rs:443`) and remove the "track has no instrument →
    per-note multitimbral routing" mode entirely. Bigger refactor: deprecate
    per-note instrument, guarantee track→instrument, migrate save/load + MCP.
  - **Recommended split (when picked up).** (1) *Now:* implement model A — compose
    track vol/pan post-FX in `stereo_gain`, route via the existing
    `route_sequencer_events` path; assume channel-strip semantics (1 track ↔ 1
    instrument) as idiomatic, document shared-instrument as last-wins. This fixes
    the bug and gives the automation hook. (2) *Later, separate session:* the full
    struct merge above. The two are independent — A does not require the merge.
- [ ] **★ HIGH: most pattern automation targets are GUI-editable but silently no-op.**
  Resolves the audit follow-up above ("automation lanes — do they all reach the engine?").
  The automation system (`synth_sequencer/src/automation.rs`) defines
  `AutomationTarget::{Instrument, Track, Global}` covering 8 instrument params
  (`AutoInstrumentParam::ALL` — Volume, Pan, FilterCutoff, FilterResonance, Attack, Decay,
  Sustain, Release), 4 track params (Volume, Pan, Mute, Solo) and 3 global params (Tempo,
  MasterVolume, Swing). The sequencer reads every lane per tick
  (`sequencer_engine.rs:379` / `:454` via `lane.value_at`), deduplicates, and emits
  `SequencerEvent::Parameter` (`sequencer_engine.rs:504`). But the engine handler
  (`synth_engine.rs:2469`) only matches `AutomationTarget::Instrument`, and within it only
  `Volume` (`set_volume`) and `Pan` (`set_pan`) — every other instrument param hits
  `_ => {}` (comment: "requires module routing (future)"), and `Track` / `Global` targets
  are not matched at all. Net effect: the automation-lane ComboBox
  (`gui/sequencer/mod.rs:3309`) offers all 8 instrument params for the selected instrument,
  so a user can draw a Filter Cutoff / Resonance / ADSR curve and hear nothing. Two fixes:
  (a) route the non-Volume/Pan instrument params through the same per-module parameter path
  `set_parameter` uses (map `AutoInstrumentParam::FilterCutoff` → the instrument's filter
  module cutoff param, etc.) — this needs the module routing the comment defers;
  (b) add match arms for `Global(Tempo)` → `transport.set_tempo`, `Global(MasterVolume)`,
  `Global(Swing)`, and `Track { .. }` — though track automation shares the same missing
  per-track output bus as the static `track.pan`/`.volume` item above, so land that
  plumbing first. See §2.1 (tempo) — that entry assumed tempo automation was already applied.
- [ ] **★ HIGH: expand Sub Oscillator waveform set from 3 to 6.** `SubOscWaveform`
  (`crates/synth_core/src/params/sub_osc.rs:13`) currently exposes only
  `Sine / Square / Pulse25`, while the main `Oscillator` exposes 6
  (`Sine / Triangle / Sawtooth / Square / Pulse / DsfSaw`). Add the three
  missing shapes: `Triangle`, `Sawtooth`, `DsfSaw`. Keep `Pulse25` distinct
  from `Pulse` (Pulse25 = fixed 25 % duty, dedicated bass shape — Pulse
  needs a PulseWidth param that the lean Sub Osc workflow deliberately
  skips). The waveform-selector widget already filters by descriptor
  choices (`gui/widgets/waveform.rs::WaveformType::from_id`, landed in
  commit `177cb0e`) so the GUI picks up the new buttons automatically as
  long as `WaveformType::from_id` covers the new ids. Touch points:
  `sub_osc.rs:23-55` (variants + `ALL` + `name` + `id` + `to_choices` +
  rendering branch in `generate_sample`), `WaveformType::from_id`
  (mappings for `triangle`/`sawtooth`/`dsf_saw` — already exist), and any
  example projects that pin Sub Osc waveform via numeric index (resaved
  to string form in `f7a6121` so the migration is free). No save-format
  bump required; existing `"sine"` / `"square"` / `"pulse25"` keep
  loading.
- [ ] **Follow-up: remove dead modules inside patch graphs.** Original §0.1 entry included this as
  a stretch goal — modules not reverse-reachable from `StereoOutput` (and any sidechain source)
  through `connections` should also go in `optimize_project`. Bigger scope (needs graph traversal
  per instrument); pick up later.
- [ ] **LPC Vocoder: missing synthesis gain + auto-vocoder positive feedback.** The
  `Vocoder` effect (`crates/synth_modules/src/effects/vocoder.rs`) is "stable" but
  unmusical on resonant inputs — it amplified the `Formant Voice` patch by ~25× and
  produced a -0 dB peak at 18–20 kHz in the §0.1 Formant Voice investigation. Two
  concrete bugs:
  (a) **No gain factor `G` from LPC analysis.** The canonical LPC synthesis filter
  is `y[n] = G·x[n] − Σ a_k·y[n−k]` where `G = sqrt(error)` (residual energy).
  `levinson_durbin_fixed` (`math.rs:825`) computes `error` but never returns it, and
  `lpc_analysis_fixed` (`math.rs:869`) discards it. `Vocoder::filter_sample`
  (`vocoder.rs:79`) uses `G = 1` implicit. Fix: have `lpc_analysis_fixed` return the
  final `error`, store it on the `Vocoder`, and multiply `input` by `sqrt(error)` in
  `filter_sample`.
  (b) **Auto-vocoder: single input used as both modulator and carrier.** LPC analysis
  places poles at the input's spectral peaks; filtering the same input through
  `1/A(z)` amplifies exactly those peaks → positive feedback. Standard fix: split
  into `in_carrier` + `in_modulator` ports, run LPC on the modulator, filter the
  carrier. Backwards-compatible fallback: when only one input is connected, derive
  the carrier as a saw/noise pulse train so the auto-mode still sounds like a
  vocoder instead of a self-resonator. Note that the existing
  `vocoder_stable_on_decaying_carrier` test (`vocoder.rs:271`) only checks
  `|output| < 50` — it documents the loudness problem rather than guards against it;
  tighten to a "musical" threshold (e.g. `peak ≤ 2.5·input_peak`) once the gain fix
  lands.
  Effect-chain order matters but is only a symptom: vocoder first → narrowband input
  → poles cluster at formant → 25× gain; vocoder after chorus+reverb → broader
  input → poles spread → modest gain.
- When saving a project with samples, the save should always be in zip-format and file extention .zip, and all other
  should be saved in json with file extention .json
- [ ] **Follow-up: stale `list_instruments` readback inside one `batch_execute`.** The primary
  bridge-race (set/get validation failing with `"instrument not found"` right after
  `apply_example_patch`) was fixed by adding a synchronous `alive_instruments` mirror on
  `SynthSession` (parallel to the existing module `registry`). What's left: `list_instruments`,
  `get_instrument_info`, and other readers that pull `volume`/`pan`/`mute` etc. still read
  `EngineState::instrument_snapshots`, which is only rebuilt on the audio thread. So
  `set_instrument_volume(5, 0.8)` followed by `list_instruments` in the same batch still reports
  the old `volume: 1.0` until the audio thread ticks (~one buffer, 5–10 ms at 44.1 kHz / 256
  samples). The audio itself is already correct; only the metadata is stale. Fix by layering
  write-through onto the `set_instrument_*` handlers in `session.rs` — patch
  `instrument_snapshots[i].volume` (etc.) under the same write lock as the queued
  `EngineCommand`. Maintenance burden: every new `set_*` tool needs to remember the write-through.
  Original diagnosis with file:line references in commit history.
- [ ] **MCP disconnects = tokio worker thread death (strace-confirmed 2026-05-18).** Until today
  the working hypothesis from the §1 MCP-stability investigation was "tool-handler panics inside
  `block_in_place` kill the worker; `LocalSessionManager` loses session state with the dying worker;
  rmcp returns `404 Session not found` on the next request and the client experiences a disconnect".
  This was proven during the Prodigy session: with `strace -e write=2` attached to all 20
  `tokio-rt-worker` threads of the GUI process (PID 671814) covering TIDs 671846–671865, a
  `build_instrument` call with a non-default mix of param values (numeric enum indices for `Waveform` /
  `Model`, the param name `"Key Tracking"`, and a small `Env Amount: 0.1`) returned
  `Streamable HTTP error: Not Found: Session not found` on the client. The strace log showed:
  `671856 +++ exited with 0 +++`, `671862 +++ exited with 0 +++`, `671863 +++ exited with 0 +++`
  (three workers terminated). A subsequent thread-list of the same process showed the worker count
  was back at 20, but with three new TIDs (678806, 678948, 679054) replacing the dead ones — the
  tokio runtime respawned, but the session state was already gone.
  Crucially **none of the §1 tracing fires for this** — the panic happens beneath the tracing
  layer, in `block_in_place`'s panic-handling path, so `on_initialized` / `Drop` / the `tracing::warn!`
  on the dispatch-error branch all sit silent. The only signal an operator sees is the worker exit
  in `strace -e write=2` (or an external panic hook, which is not currently installed).
  This is concrete validation for two §1 follow-ups that should be promoted to actual TODO items:
  (a) **panic catching around tool dispatch** in `synth_mcp::server` (`AssertUnwindSafe` +
  `FutureExt::catch_unwind` around the `tool_router` and `dispatch_tool` calls) so a single bad
  tool call surfaces as `ErrorData::internal_error` to the client instead of killing a worker;
  (b) **migrate the CPU-bound bridge calls from `block_in_place` to `spawn_blocking`** so panic
  recovery is the standard tokio task path (`spawn_blocking` returns a `JoinHandle` whose `await`
  yields `JoinError::Panic`, while a panic in `block_in_place` propagates up the worker's polling
  loop and kills it). Both fixes are explicitly named in §1 of the MCP stability plan; (a) is the
  high-leverage one (one place, ~20 lines, removes the entire class of "single bad call kills
  the session"). After either lands, also extend the §1 tracing with a
  `std::panic::set_hook` that logs `tracing::error!("MCP task panicked", message, location, ...)`
  so operators see panics without needing `strace`.
- [ ] **`WidgetHint::PanKnob` parameters are never rendered in the auto-renderer.** The shared
  descriptor-driven parameter grid (`gui/widgets/param_grid.rs::draw_parameter_grid`, used by both
  the Rack patch editor and the mixer's return inserts) groups parameters by hint into
  WaveformSelector / Slider+TimeSlider / Dropdown / Toggle / Knob+FrequencySlider — `PanKnob` (and
  the other unused hints `XYPad`, `PercentSlider`, `DecibelSlider`) is in none of them, so any such
  param silently disappears from the UI. Currently only `amplifier.rs` and `output.rs` set `PanKnob`
  (neither is a return-insert effect), and this was already the behavior before the param-grid
  extraction, so it's latent rather than a regression. Fix: fold `PanKnob` into the knob group (it
  is semantically a knob) — or add a catch-all so no descriptor hint can drop a parameter — and
  decide intended widgets for `PercentSlider`/`DecibelSlider`/`XYPad` while there. Note this will
  start rendering the amp/output Pan knobs that are currently invisible in the Rack.

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

| Entity                       | Field exists?                        | MCP read | MCP write       |
|------------------------------|--------------------------------------|----------|-----------------|
| `InstrumentState`            | ✅ `patch.rs:697`                     | ❌        | ❌               |
| `Patch`                      | ✅ `patch.rs:130, 307` (Option)       | ❌        | ❌               |
| `AwePresetFile`              | ✅ `patch.rs:792`                     | ❌        | ❌               |
| `Song`                       | ❌ — add to `synth_sequencer/song.rs` | ❌        | ❌               |
| `Pattern`                    | ❌ — add to `synth_sequencer`         | ❌        | ❌               |
| `SequencerTrack`             | ❌ — add to `synth_sequencer`         | ❌        | ❌               |
| `Sample` entry               | ❌ — add to sample registry           | ❌        | ❌               |
| Module *instance* (in patch) | ❌ — separate from `ModuleDescriptor` | ❌        | ❌               |
| `ModuleDescriptor` (type)    | ✅ `module_traits.rs:869`             | ✅        | n/a (hardcoded) |
| `ParameterDescriptor` (type) | ✅ `module_traits.rs:586`             | ✅        | n/a             |
| `PortDescriptor` (type)      | ✅                                    | ✅        | n/a             |
| `ChoiceOption` (type)        | ✅ `module_traits.rs:557` (Option)    | partial  | n/a             |

### Phase 2 — add new description fields + MCP read/write tools

- [x] Add `description: String` to `Song` (`synth_sequencer/src/song.rs`); surface in `get_song_info`;
  add `set_song_description` MCP tool
- [x] Add `description: String` to `Pattern`; surface in `list_patterns` / pattern resource;
  add `set_pattern_description` MCP tool (e.g. `"chorus drop, half-time feel"`)
- [x] Add `description: String` to `SequencerTrack`; surface in `list_tracks`;
  add `set_track_description` MCP tool
- [x] Add `description: String` to sample registry entries (`SampleMeta`); surface in `list_samples` /
  `get_sample_info`; add `set_sample_description` MCP tool
- [x] Editable from GUI (song properties, pattern properties dialog, track header context menu, sample library)

### Phase 3 — per-module-instance notes (different concept from type docs)

- [ ] Add per-instance `description: String` on placed modules in a patch (e.g. annotate "this LFO is the
  wobble modulator" on a specific `lfo-1` instance). Distinct from `ModuleDescriptor.description` which
  documents the module *type* and is shared across all instances.
- [ ] Surface in `get_module_info` (MCP read); add `set_module_description(instrument_id, module_id, description)`
  MCP tool (MCP write). Accept `""` to clear.
- [ ] Editable from GUI (module header context menu)

#### Persistence (must round-trip on save/load)

Module-instance descriptions **must** survive serialization, both at the project and standalone-patch
level — otherwise AI-applied notes silently vanish on save/reload. Same pattern as
`Patch.description` and the planned color persistence above.

- [ ] **Project save** — every module instance's description persisted inside its containing patch
  in the project JSON. Round-trip test: MCP-set description on `lfo-1` → `save_project` →
  `new_project` → `load_project` → `get_module_info` returns the same text.
- [ ] **Standalone patch save** — the per-instance description travels with the .json patch file
  when the user invokes "Save Patch…". A patch saved by AI must carry its module notes into other
  projects that load it.
- [ ] **Project / patch load → engine mirror** — on load, copy each saved module description into
  the engine's runtime mirror so subsequent MCP reads see what was loaded, not stale defaults.
  Mirror the `Patch.description` → `Instrument.description` plumbing used in Phase 1.
- [ ] **No partial states** — do not ship `set_module_description` without the full save/load path;
  document as known-broken until both halves land.

### Cross-cutting

- [ ] Include all instance-level descriptions in `get_graph_diagnostics` / `analyze_note` output so AI sees
  intent alongside structure
- [ ] Surface descriptions as tooltips on the corresponding GUI elements
- [ ] Decide on max length (suggest 500 chars soft, 2000 hard) — long enough for a paragraph, short enough to
  stay readable in tooltips
- [ ] Persistence format: inline in the existing JSON containers (no sidecar files)

---

## ★ Color fields via MCP

Color fields already exist on several entities (Patch, Instrument, Group, SequencerTrack), but
**no MCP setter** exposes them — AI can build a song but can't paint the strips/tracks to make the
arrangement visually scannable. Parallel structure to the description roadmap above: read on the
existing getter response, write via a dedicated setter routed through
`bridge.rs` → `mcp_bridge.rs` → `server.rs`. Color is also already GUI-editable in most cases.

### Current status of color fields

| Entity            | Field exists?                        | MCP read | MCP write |
|-------------------|--------------------------------------|----------|-----------|
| `InstrumentState` | ✅ `patch.rs:700` `Option<HexColor>`  | ❌        | ❌         |
| `Patch`           | ✅ `patch.rs:255` `Option<HexColor>`  | ❌        | ❌         |
| `Group`           | ✅ `patch.rs:316` `Option<HexColor>`  | ❌        | ❌         |
| `SequencerTrack`  | ✅ in song JSON `{r, g, b}` per track | ❌        | ❌         |

### Work to do

- [ ] Surface color on `get_instrument_info` / `list_instruments` (MCP read); add
  `set_instrument_color(instrument_id, color)` MCP tool. Accept `"#RRGGBB"` / `"#RRGGBBAA"` and
  `""`/`null` to clear back to "auto" / default. **Blocked: needs engine-ownership refactor** —
  instrument color is GUI-only (`InstrumentUiState.color`) and never transits the engine snapshot
  (`project_apply.rs` hardcodes `color: None`). Make it engine-owned like `description` (new
  `EngineCommand` + `Instrument` runtime field + snapshot + save/load mirror). Own PR.
- [ ] Surface patch color on the same getters as a separate `patch_color` field, mirroring how
  `patch_description` is exposed alongside `description`. Add `set_patch_color` MCP tool.
  **Blocked: `Patch.color` does not exist yet** — add the field first, then mirror the description flow.
- [x] Surface track color on `list_tracks`; add `set_track_color(track_id, color)` MCP tool.
  Accepts `"#RRGGBB"`/`"#RRGGBBAA"`; backed by `TrackColor::to_hex`/`from_hex`. Track color is a
  pure `Song` write, so it round-trips for free and was already GUI-editable.
- [ ] Surface group color on `get_instrument_info` (or wherever groups are listed); add
  `set_group_color` MCP tool. **Group color is GUI-only** (no engine path) — needs MCP to mutate
  the `PatchEditor` group state directly. Own PR.
- [ ] Decide whether AI-friendly named palettes are useful (`"warm-orange"`, `"cool-blue"`) on top of
  raw hex — same pattern as `set_awe_preset` vs `set_awe_parameter`. Out of scope for v1; raw hex
  is enough.

### Persistence (must round-trip on save/load)

Color writes via MCP **must** survive serialization, both at the project and standalone-patch level
— otherwise AI-applied colors silently vanish on save/reload. This is the same architectural
pattern used for `Patch.description` (runtime mirror in the engine, project load copies in, project
save reads back).

- [ ] **Project save** — all four entities' colors persisted in the project JSON. Verify by
  round-tripping: MCP-set a color → `save_project` → `new_project` → `load_project` → color is the
  same. `InstrumentState.color`, `SequencerTrack` color and `Group.color` already serialize via
  serde; mainly need to confirm the engine-side runtime mirror is read back into the snapshot at
  save time (mirror the `description` plumbing).
- [ ] **Standalone patch save** — `Patch.color` travels with the .json patch file when the user
  invokes "Save Patch…" from the instrument-edit window. Important because a patch saved by AI
  should carry its color into other projects that load it.
- [ ] **Project load → engine mirror** — on `load_project`, push each saved color into the engine's
  runtime mirror (analogous to how `Patch.description` is copied into `Instrument.description` at
  load time) so subsequent MCP reads see what was loaded, not stale defaults.
- [ ] **No partial states** — if any color setter is added without the corresponding save+load path
  wired, document it as known-broken until both halves land; do not ship a setter that only
  updates the live engine but not the project file.

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

### 1.4 Mixer view

- [ ] Dedicated mixer view with faders, pan, sends, and inserts
- [ ] Send/return effect busses — shared effects instead of per-instrument chains only

### 1.5 Settings & utilities

- [ ] Add Browse button in Settings dialog to change patches directory
- [ ] Extract `magnitude_to_normalized_db()` into `synth_core` or `synth_dsp` — repeated in 4+ locations
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

- [ ] **Wire `Global(Tempo)` automation into the engine at all** (see the §0.1 HIGH item on
  unwired automation targets). A tempo lane currently emits an unhandled
  `SequencerEvent::Parameter` and does nothing — the "step changes only" note below assumed
  it was already applied. Drive `transport.set_tempo` from the lane value first.
- [ ] Tempo curve interpolation (once wired, currently would be step changes only — accelerando
  ramps would smooth between two adjacent points)

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

**Roadmap:** `docs/note-expression-roadmap.md` — staged plan toward per-note
expression that also retires the `sid-analyzer` export gaps. Covers generic
sequencer→module automation (Phase A1 bugfix: the 6 GUI-exposed but dead
FilterCutoff/ADSR instrument macros; A2: `AutomationTarget::Module`), per-note
legato/glide (B), per-note vibrato (C), shared/bus filter (D, rides on
channel-strip Phase 7), and the full Note Expression + MPE system (E).

**Phase A1/A2 deferred follow-ups (first cut shipped v0.292.0).** The generic
sequencer→module automation landed (the 6 dead FilterCutoff/ADSR macros now sound,
plus the generic `AutomationTarget::Module`). These three were explicitly deferred
from that first cut and are the remaining open work:

- [x] **Stable (non-positional) ModuleId identity — verified unnecessary (F2).**
  `AutomationTarget::Module` keys on `module_type` + `instance`. Verified the worry
  (a lane "silently re-points" when modules are added/removed) does not occur: instance
  numbers are monotonic and never reused, `remove_module` does not renumber survivors,
  and load preserves instances via `add_module_with_id`. Removing a same-type sibling
  never re-points a surviving lane; worst case is a harmless orphaned lane (dispatch
  no-ops on an absent module). No stable-id migration needed. Locked by
  `graph::module_instance_identity_is_stable_across_removal`.
- [ ] **`ParamId(Arc<str>)` off-thread drop (F1 residual).** Cloning a `Module`
  automation target is now alloc-free, but the engine's cached clone can become the
  last `Arc` reference and so *drop* (free) on the audio thread — but only if the
  source lane was removed mid-playback. Strict improvement over the prior `String`
  (which freed on every drop). Full fix: route cleared/replaced targets through the
  engine's `return_producer` off-thread drop channel. Low priority (bounded, rare).
- [ ] **Mod-matrix vs. automation combine ordering ("two controllers").** When a
  mod-matrix offset *and* an automation override target the same param, the
  precedence/combine rule is unspecified. The first cut applies the override as an
  absolute *replace* of the base, with mod-matrix offsets still added additively on
  top — but the "two controllers drive one param" case needs a defined, documented
  rule (and possibly a user-facing choice).
- [ ] **Offline-render parity for `analyze_*`.** The offline render path behind the
  `analyze_*` tools does not apply automation override state, so analysis can see a
  value the live audio engine never produced. Same bug class as the analyze_*
  offline-render snapshot issue (see `docs/history.md` 74d18da) — extend the offline
  reader to honour the automation overrides.

The two bullets below are **Phase E** of that roadmap.

- [ ] MPE support — MIDI Polyphonic Expression for per-note pitch bend, pressure, slide
- [ ] Polyphonic aftertouch routing to module parameters

### 3.5 Polyphony settings

- [ ] Voice stealing mode selection (oldest, quietest, none) — engine + persistence done, GUI selector
  not yet added (defaults still applied).
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
it *looks* unused: no visible cable in the patch-editor graph, and `list_modules` reports
`output_ports: []` because the matrix routes by parameter selectors, not ports. The graph is
healthy and `get_graph_diagnostics` confirms that, but the user has no way to see *why* from the
cable view alone. Real example: the `Acid Bass` example patch — `env-2` modulates Filter cutoff
via Mod Matrix slot 1 with amount 0.9, zero cables, and looks dead.

- [x] **GUI: badge on module headers when referenced by Mod Matrix.** Done in 0.289.0 —
  `PatchAnalysis` collects matrix sources/destinations and the header shows an
  arrow badge with a tooltip pointing at the Mod Matrix module.
- [ ] **GUI: ghost cables for Mod Matrix routings.** In the cable view, draw faint dashed lines
  (different colour) from matrix sources to their destinations (e.g. `env-2` → `flt-1.cutoff`),
  togglable in the View menu. Both routing paradigms become visible in one canvas.
- [x] **MCP: surface Mod Matrix routings in `get_connections`.** Done in 0.289.0 —
  new `get_mod_matrix_routings` tool returns `[{source, source_name, destination,
  destination_name, amount, enabled, slot}]` with positional `ModSource` /
  `ModDestination` resolution.
- [x] **MCP: stop reporting `output_ports: []` on matrix-only modules.** Done in
  0.289.0 — `list_modules` surfaces a virtual `"matrix"` port on the matrix
  module and on every module referenced by an active matrix slot.

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
- [x] **`batch_execute` dispatch-coverage guard test.** Done in
  `crates/pertylizer/tests/mcp_batch_dispatch_coverage.rs`: enumerates the rmcp tool-router's
  registered names (`SynthMcpServer::router_tool_names`) and asserts each — except the documented
  exemptions `batch_execute`, `preview_note`, `analyze_note` — is reachable through the
  `dispatch_tool_inner` table (probed via `dispatch_tool_for_test` with scalar params so no tool
  body runs). Prevents the router/dispatch drift fixed in `00845fb`.
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
