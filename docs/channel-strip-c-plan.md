# Plan: Channel-strip-C — track ↔ instrument unification (§0.1 of docs/TODO.md)

Resolves the two HIGH items in §0.1 (track `pan`/`volume` never reach audio;
pattern/track automation targets silently no-op) and the §2.1 tempo item, **builds a
real per-channel bus** (the defining piece of Model C), and lays the foundation for
the §1.4 mixer view (faders/pan/sends/inserts).

This plan builds Model C, not just the groundwork for it: **Phase 1 introduces the
actual per-track bus node** as a contained, audio-identical refactor, and every later
phase layers onto it.

## Status at a glance

- [x] **Phase 1** — Per-channel bus node (the Model-C core; audio unchanged)
- [x] **Phase 2** — Track vol/pan/mute/solo into the bus fader + Track/Global automation arms (Tempo→§2.1, Swing/Solo deferred)
- [x] **Phase 3** — Make `track.instrument` mandatory; **sharing allowed** (no strict 1:1); remove "— None — (per-note)"
- [x] **Phase 4** — Remove `note.instrument` (route only via `track.instrument`; preview via threaded instrument)
- [x] **Phase 5** — Orphan-preview: **kept** the engine-side branch; scratch-track rework closed as not-worth-it; added the missing preview/lifecycle tests
- [x] **Phase 6** — Save/load + MCP cleanup (per-note `instrument_id` removed; descriptions + track color via MCP/GUI; instrument/patch/group color split to its own PR — engine refactor)
- [~] **Phase 7** — Sends/returns + mixer view (§1.4): **7a (engine sends + dynamic return busses + persistence + MCP) done**; 7b (mixer view) + return effect-chain persistence remain
- [ ] **Phase 8** — (future) Independent faders for *shared* instruments — per-voice-tagged per-track bus

**Revised direction (2026-05-27, user decision):** strict 1 track ↔ 1 instrument is
*not* enforced. Multiple tracks may share an instrument (intentional layering, e.g.
Kick + Syncopated Kick). The rule is weaker: **every track has an instrument, and
instruments may be shared.** With the channel-bus we built (keyed by `InstrumentId`),
a shared instrument means a **shared fader** ("last routed track wins per block") —
the notes still layer correctly. Independent faders per track on a *shared* instrument
need per-voice track-tagging (lands pre-FX) — captured as **Phase 8**, deliberately
*not* built now.

Mark each `- [ ]` as `- [x]` and flip the per-phase **Status** line when a phase
lands. Keep `docs/history.md` updated against the ship date for each phase.

## Context

### The bug
`SequencerTrack.pan`/`.volume` (`track.rs:30`, `track.rs:32`) are stored, round-trip
through save/load, and are settable via GUI sliders and MCP — **but never reach the
audio output.** The only pan/volume that actually sounds is per *instrument*
(`Instrument::stereo_gain`, `instrument.rs:1294`, applied in the mix loop at
`instrument.rs:1280`). The sequencer's `Parameter` handler (`synth_engine.rs:2470`)
only matches `AutomationTarget::Instrument` → `Volume`/`Pan`; `Track`/`Global`
targets are not matched at all (`_ => {}` at `synth_engine.rs:2482`). So track faders
are dead UI, and most automation lanes are silent.

### Key architectural insight
The engine's `Instrument` (`instrument.rs:363`) is **already most of a channel strip**:
`process()` sums its voices → runs its own `effect_chain` → applies `stereo_gain()`
(vol×pan) → adds into the shared `mix_buffer` (`instrument.rs:1280`). The sidechain
path (`prev_instrument_outputs: HashMap<InstrumentId, AudioBuffer>`,
`synth_engine.rs:383`) already captures each instrument's post-FX output into a
per-instrument buffer (today as a one-buffer-delayed copy for sidechain). Phase 1
**formalizes that capture into the live signal path** and moves the fader out of the
instrument into a real bus stage — that is the per-track bus, with a fraction of the
work a from-scratch bus engine would take.

### Why channel-strip (cheap fader) — and where per-voice tagging would be needed
The channel-bus keys the fader by `InstrumentId`, so the per-channel cost is flat
(one buffer copy + sum per channel). The expensive parts of a general per-track-bus
design (per-voice track tagging, gain applied pre-effect-chain) are only needed to
give **independent faders to two tracks that share one instrument**. We *allow*
sharing but accept a **shared fader** for it (last-routed-track-wins per block) — so
those costs never appear. Independent faders for shared instruments are **Phase 8**
(future), not built now.

### The invariant (revised)
**Every `SequencerTrack` has an instrument; instruments may be shared across tracks.**
(Not strict 1:1 — sharing is intentional layering, e.g. Kick + Syncopated Kick on one
drum instrument.) Per-note `note.instrument` routing is removed (Phase 4): a note's
instrument comes solely from its track. A shared instrument shares its channel/fader;
use two instruments (same patch) when independent faders are wanted, until Phase 8.

## Current state (what already exists)

- `Instrument` per-channel effect chain + `stereo_gain()` mixing — the basis the bus
  node formalizes.
- `prev_instrument_outputs` (`synth_engine.rs:383`) + `last_output_interleaved` —
  per-instrument post-FX capture, half the bus-node work already done.
- `InstrumentMapping` (`instrument_mapping.rs:19`, `engine_id` at `:57`) maps
  `SeqInstrumentId` ↔ engine `InstrumentId`.
- `route_sequencer_events` (`synth_engine.rs:2415`) is the note/automation write point;
  its `Parameter` arm (`:2469`) already calls `set_volume`/`set_pan` on instruments.
- Track GUI sliders + MCP `set_track_volume`/`set_track_pan` already write
  `track.volume`/`.pan` and persist them — only the audio wiring is missing.
- Orphan-preview mode (`sequencer_engine.rs:361`, field `preview_pattern` at `:88`)
  loops an unplaced pattern by routing notes via `note.instrument`.

---

## Phased plan

### Phase 1 — Per-channel bus node (the Model-C core)
**Status:** ☑ Done — branch `feat/channel-strip-phase1-bus-node`

Built the real per-track bus stage. Deliverable was a **bit-identical audio output**
structural refactor, gated by tests.

- [x] Per-channel pre-fader signal: `Instrument::effect_buffer` already holds the
  post-FX, pre-fader channel signal (exposed via `last_output_interleaved`), so it
  *is* the channel bus input — no new buffer added. A **persistent post-fader** bus
  buffer was deliberately deferred to Phase 7 (sends/returns): adding it now would be
  unread → `dead_code` under `-D warnings`.
- [x] Moved `stereo_gain()` + soft-clip mix out of `Instrument::process` into a new
  engine-level bus stage `mix_channel_busses` (`synth_engine.rs`). The instrument is
  now a pure sound source and no longer takes an `output` buffer.
- [x] Fader source kept = instrument vol/pan (no track wiring yet) → output unchanged.
- [x] Sidechain preserved: tap stays **pre-fader** (the bus stage never writes gain
  back into `effect_buffer`; `last_output_interleaved` → `prev_instrument_outputs`
  unchanged). Decision documented in code.
- [x] Tests: existing determinism suite still green; added `channel_bus_pan_biases_
  left_channel` and `channel_bus_volume_scales_level` in
  `tests/arrangement_render_determinism.rs` to pin that fader/pan apply at the new
  stage. `fmt`/`build`/`clippy --all-targets`/`test` all clean.

### Phase 2 — Wire track vol/pan/mute/solo into the bus fader
**Status:** ☑ Done — static fader + automation arms (Tempo deferred to §2.1)

The bus stage now exists; route the track controls into it. First user-visible win
(the dead-fader fix, §0.1 first HIGH item).

- [x] Carry a per-channel track-control snapshot (vol, pan, audible) into the bus
  stage. `TrackControl` map on `SynthEngine`, pre-allocated per instrument (mirrors
  `prev_instrument_outputs`), rebuilt each block in `update_track_controls` from the
  Song via `try_read`, resolved through `InstrumentMapping`. A `NEUTRAL` entry composes
  bit-identically to the instrument-only fader (verified).
- [x] Bus fader composes: `volume = inst.vol × track.vol`, `pan = inst.pan + track.pan`
  (clamped by `BipolarValue::new`, one constant-power law — additive, not cascaded),
  track `audible` (mute / solo-exclusion via `SequencerTrack::is_audible`) gates the
  channel. (Fader unification deferred to Phase 4.)
- [x] Documented shared-instrument behaviour as "last routed track wins per block"
  (in `update_track_controls` doc; removed in Phase 3).
- [x] Tests: `track_pan_biases_output`, `track_volume_scales_output` (prove track
  fader reaches audio); existing bit-exact determinism suite still green. Independent
  RT-safety + equivalence review passed.
- [x] Automation arms (resolves the §0.1 *second* HIGH item):
  - `Track { Volume, Pan, Mute }` → sequencer-owned `TrackAutoOverride` map
    (`SequencerEngine::track_auto`), composed over the static track fader in
    `update_track_controls`. Sequencer-owned so its `clear()` mirrors
    `last_automation_values` at all four transport-reset sites (incl. the
    audio-thread loop-wrap/auto-stop the engine can't see).
  - `Global(MasterVolume)` → `apply_global_automation` (mirrors
    `handle_set_master_volume`: field + shared atomic).
  - Tests: `track_volume_automation_ramps_down`, `global_master_volume_automation_ramps_down`.
- [ ] **Deferred:** `Global(Tempo)` → **§2.1**. Playback rate is driven by the
  sequencer's `cached_tempo` (from `Song::tempo_at`), *not* `state.transport`, so
  `transport.set_tempo` alone changes only the readout, not the render — wiring it
  properly is the §2.1 tempo-curve work. `Global(Swing)` (no engine impl) and
  `Track(Solo)` automation (cross-track concept) also deferred.

### Phase 3 — Make `track.instrument` mandatory (sharing allowed)
**Status:** ☑ Done

Revised from "enforce strict 1:1" to the user's model: every track has an instrument,
but instruments **may be shared** across tracks (shared fader — see Phase 8 for
independent faders). No project migration needed — the audit found all 13 example
projects already have an instrument on every arranged track; the 2 shared-instrument
projects (Oxygene 80s, Synth Pop) are left as-is.

- [x] Made `SequencerTrack.instrument: SeqInstrumentId` (dropped the `Option`;
  `SeqInstrumentId` gained `Default = 0`, field is `#[serde(default)]`).
  `Song::create_track` defaults to `SeqInstrumentId(0)`. ~17 `.instrument` sites
  de-Optioned across sequencer/engine/GUI/MCP/analysis/tests (compiler-guided).
- [x] GUI: removed the "— None — (per-note)" combobox option; the selector lists only
  real instruments; label shows "— (none) —" only when id 0 maps to no loaded inst.
- [x] MCP: `set_track_instrument(None)` is now a no-op (can't clear); `create_track`
  keeps `Option<u16>` (None → default 0). *Signature tightening (required arg) deferred
  to Phase 6 MCP cleanup.*
- [x] `update_track_controls` drops the `None` skip; a shared instrument → last-wins
  fader (documented in code; Phase 8 for independent faders).
- [x] Serialization: non-null values serialize identically (bare integer), all 13
  example projects load unchanged; `project.schema.json` regenerated (instrument field
  no longer nullable). Tests: `new_track_has_a_default_instrument`,
  `instruments_may_be_shared_across_tracks`; full suite green; independent review passed.
- [ ] *(Deferred follow-up, not strictly Phase 3)* `TrackMode` → `AllocationMode`:
  introduces a second source of truth for alloc mode under sharing — revisit with the
  struct merge.

### Phase 4 — Remove `note.instrument`
**Status:** ☑ Done — landed in two commits (4a preview plumbing, 4b removal)

- [x] **4a — preview instrument:** the orphan-preview path had no track context and
  routed via `note.instrument`. `SetPreviewPattern`/`PlayPattern` now carry an
  instrument; the sequencer stores `preview_instrument` and plays through it; GUI
  passes `selected_instrument`. (Prerequisite so removing the field doesn't break
  orphan preview/REC.)
- [x] **4b — field removal:** dropped `Note.instrument` + the `Note::new`/`add_note`
  params; deleted dead `Pattern::generate_events`; `Song::remove_unused` collects
  instruments from tracks only. Arrangement playback was already track-only (Phase 3).
- [x] GUI: notes have no instrument — colour/tooltip derive from the pattern's track
  instrument (`track_overrides`, else working instrument); arrangement miniatures
  colour by the placement's track instrument; dropped `instrument` from
  `NoteMiniature`/`PianoRollNote`/`ClipboardNote`; removed the now-vestigial
  `recording_instrument`; fixed the stale override-badge hover text.
- [x] MCP: `instrument_id` inputs are kept but **ignored** (descriptions updated);
  full API removal deferred to Phase 6. undo `NoteSnapshot.instrument` removed.
- [x] Serialization: no `deny_unknown_fields`, so old saves load (extra `instrument`
  key ignored); `project.schema.json` + gen_schemas examples regenerated.
- [x] Tests green (447); independent review passed.
- Note: `Note.track` (also vestigial) intentionally left for a later cleanup.
- Fader unification (track drives instrument vol/pan directly) deferred to the
  struct-merge follow-up.

### Phase 5 — Orphan-preview: kept the mechanism, added tests
**Status:** ☑ Done — scratch-track rework **closed as not-worth-it**

Phase 4a already solved the original motivation (orphan preview had no instrument)
by threading `preview_instrument`, so the only remaining rationale was "remove the
special branch." Three exploration agents independently concluded the scratch-track
rework is a **net-complexity increase, not a simplification**:

- The current orphan-preview is a self-contained, RT-safe, audio-thread-only ~30-line
  branch that touches **nothing** in the shared `Song` — zero save/load and zero
  display surface.
- A scratch track must live in the `Song` (the audio thread can only `try_read`), so
  entry is UI-thread and teardown spans ~9 sites — several on the audio thread, which
  **cannot write-lock**. A missed teardown leaves a **phantom track/placement that
  gets saved to disk** — a worse, *persisted* failure class than today's runtime-only
  state.
- The `Song` serializes tracks+arrangement unconditionally, and **16 `song.tracks()`
  sites across 6 crates** assume all tracks are real/visible (plus new invariants in
  `calculate_length`, `any_solo`, the bus-fader loop) — each needs a scratch-exclusion.
- The "engine-local synthetic placement" alternative is just the special branch in a
  different shape — no net deletion.

**Decision:** keep the engine-side `preview_pattern` branch; do not build the scratch
track. Instead the real gap (the preview/REC path had a history of lifecycle bugs —
`e674f39`, v0.290.0 — and **zero test coverage**) was filled:

- [x] Sequencer tests (`sequencer_engine.rs`): orphan preview emits NoteOn through the
  supplied `preview_instrument`; an unplaced pattern is silent without preview;
  clearing preview re-silences it; zero-length preview pattern doesn't panic.
- [x] Command-handler lifecycle tests (`synth_engine.rs`): `Stop` and `SetSong` clear
  preview; solo and preview are mutually exclusive — guarding the v0.290.0 bug class.
- Also fixed a regression the prior phase's grep-based check masked: `synth_engine`
  tests didn't compile after the `add_note` signature change (commit `a5cd8be`).
  Verification now checks `cargo test`'s exit code, not just result-line greps.

If routing preview through the channel bus ever becomes a hard requirement, the
lowest-risk option is the engine-local synthetic placement — but it must be justified
by bus-routing need, not sold as a simplification.

### Phase 6 — Save/load + MCP cleanup
**Status:** ☑ Done — instrument/patch/group **color** carved out to its own PR

- [x] **Per-note `instrument_id` removed from MCP.** Dropped the kept-but-ignored
  `instrument_id` from `AddNoteParam`/`NoteInput`/`BridgeNoteData`, the `add_note`
  trait param, and the five handler mappings (`add_note`, `add_notes`,
  `replace_notes`, `create_patterns`, `set_song`).
- [x] **Save/load round-trips** confirmed without `note.instrument`; no
  `deny_unknown_fields`, `track.instrument` is `#[serde(default)]`. Added a focused
  round-trip test for descriptions + track color + the track↔instrument binding,
  plus a legacy-load (missing `description` keys) default test.
- [x] **Description setters for all sequencer/sample entities.** Added
  `description: String` (`#[serde(default)]`) to `Song`, `Pattern`,
  `SequencerTrack`, and `SampleMeta`; surfaced on `get_song_info` / `list_patterns`
  / `list_tracks` / `list_samples`; added `set_song_description`,
  `set_pattern_description`, `set_track_description`, `set_sample_description`
  (instrument/patch description setters already existed). GUI editing wired too.
- [x] **Track color via MCP.** `set_track_color` (accepts `"#RRGGBB"` /
  `"#RRGGBBAA"`) + color surfaced on `list_tracks`, backed by new
  `TrackColor::to_hex`/`from_hex`. Track color was already GUI-editable.
- [ ] **Deferred — instrument/patch/group color (own PR).** Tracing showed
  instrument color lives GUI-side only (`InstrumentUiState.color`) and never
  transits the engine snapshot (`project_apply.rs` hardcodes `color: None`); MCP
  control needs it engine-owned like description (new `EngineCommand` +
  `Instrument` runtime field + snapshot + save/load mirror). `Patch.color` doesn't
  exist yet and group color is GUI-only. That's an engine-ownership refactor, not a
  setter add — split out to keep this PR free of `synth_engine` changes.

### Phase 7 — Sends/returns + mixer view (§1.4)
**Status:** ◑ 7a done (engine sends + dynamic return busses + persistence + MCP); 7b (mixer view) next

The per-channel bus from Phase 1 already exists, so this was just adding taps off
each bus — not building bus infrastructure.

- [x] **7a — send taps + dynamic return busses (engine).** `TrackSend`
  (target/level/pre-or-post-fader) on `SequencerTrack`; taps added in
  `mix_channel_busses` into per-return accumulation buffers, then each return runs its
  own `EffectChain` and is mixed back to master (`ReturnBusChannel::mix_into`). RT-safe:
  per-channel send snapshot pre-allocated (`MAX_CHANNEL_SENDS`, no audio-thread alloc);
  return channels created/removed via `CreateReturnBus`/`RemoveReturnBus` off the hot
  path. **Return-bus definitions live in the `Song`** (name/volume/pan/mute), read live
  by the engine each block like track controls (Model C) — so routing + faders
  round-trip via save/load automatically and are reconstructed on load
  (`project_apply`) and in the offline renderer. Effects on returns via
  `AddReturnEffect`/`RemoveReturnEffect`/`SetReturnEffect{Parameter,Enabled}`. MCP:
  `create_/delete_/list_return_busses`, `set_return_bus_volume/pan/mute`,
  `rename_return_bus`, `set_/remove_track_send` (+ sends surfaced on `list_tracks`).
  Tests at every layer (engine DSP/commands/send-resolution/audio, sequencer serde
  round-trip, MCP bridge round-trip); fmt/build/clippy/test all clean.
- [ ] **Deferred — return effect-chain *content* persistence.** Return-bus effect
  chains (the reverb instance + its params) are not yet saved/loaded — same level as
  master effects today. Needs serializing each return's chain (type + params via
  `AudioEffect::get_params`) and rebuilding on load (`create_effect` +
  `AddReturnEffect` + `SetReturnEffectParameter`).
- [ ] **7b — dedicated mixer view** with faders, pan, sends, inserts.

### Phase 8 — (future) Independent faders for shared instruments
**Status:** ☐ Not started — deferred, build only if needed

Today two tracks sharing one instrument share its channel/fader (last-routed-track-
wins per block); their notes still layer correctly. Giving each track an *independent*
volume/pan on a shared instrument requires **per-voice track-tagging**: tag each voice
with the track that triggered it and apply the track fader per-voice in the voice-sum
loop. Caveats (the reason this is deferred, not default):
- The per-voice gain lands **pre-effect-chain** (voices share the instrument's one
  effect chain), so it is *not* a true post-FX fader for shared instruments.
- Adds per-voice bookkeeping cost (the "more CPU" path discussed during planning).

Workaround until then: use two instruments with the same patch when independent faders
are wanted. Build this only if a real need appears.

---

## Critical files

| File                                                      | What changes                                                        |
|-----------------------------------------------------------|---------------------------------------------------------------------|
| `crates/synth_engine/src/synth_engine.rs:2290,383`        | Phase 1: bus stage + per-channel bus buffers (formalize capture)    |
| `crates/synth_engine/src/instrument.rs:1280,1294`         | Phase 1: move `stereo_gain` out into the bus stage                  |
| `crates/synth_engine/src/synth_engine.rs:2469`            | Phase 2: track-control snapshot + `Track{…}`/`Global(Tempo)` arms   |
| `crates/synth_sequencer/src/track.rs:28,40`               | Phase 3: enforce `instrument: Some`; activate `TrackMode`           |
| `crates/synth_sequencer/src/song.rs:91`                   | Phase 3: track↔instrument binding + migration                      |
| `crates/synth_engine/src/sequencer_engine.rs:443`         | Phase 4: drop `note.instrument` fallback                            |
| `crates/synth_sequencer/src/pattern.rs`                   | Phase 4: remove `instrument_override`                               |
| `crates/synth_engine/src/sequencer_engine.rs:361,88,155`  | Phase 5: remove orphan-preview branch + field + setter              |
| save/load + `crates/synth_mcp/src/server.rs`, `bridge.rs` | Phase 6: persistence + MCP `add_notes` migration                    |
| bus send taps + GUI mixer view (new)                      | Phase 7: sends/returns off the Phase-1 bus + mixer view             |

## Verification

- **Phase 1:** arrangement render bit-identical to pre-refactor (determinism test);
  sidechain compressor patch behaves unchanged.
- **Phase 2:** set a track volume/pan via GUI or MCP and confirm it is audible;
  draw a Track Volume automation lane and hear it; engine unit test on bus-fader
  composition.
- **Phase 3–4:** load a pre-migration song with per-note instruments → audio
  identical or documented re-interpretation; per-note selector gone from GUI/MCP.
- **Phase 5:** REC-arm an unplaced pattern, play, confirm notes are captured and
  audible; preview path no longer uses the special branch.
- **Phase 6:** MCP set track/instrument vol/pan/color → `save_project` →
  `new_project` → `load_project` → values round-trip.
- **Phase 7:** route a channel send to a reverb return bus; pulling the channel fader
  dims the dry but not the send (or vice-versa, per pre/post choice).

## What this resolves
A real per-channel bus (Phase 1), dead track faders (Phase 2), no-op track automation
lanes (Phase 2), dead `TrackMode` enum (Phase 3), orphan-preview special branch
*removed* (Phase 5), and the §1.4 mixer view reduced to taps off an existing bus
(Phase 7).
