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
- [ ] **Phase 2** — Wire track vol/pan/mute/solo into the bus fader (dead-fader fix)
- [ ] **Phase 3** — Establish & enforce the 1-track-↔-1-instrument invariant
- [ ] **Phase 4** — Deprecate `note.instrument` (route only via `track.instrument`)
- [ ] **Phase 5** — Orphan-preview via scratch channel (removes the special branch)
- [ ] **Phase 6** — Save/load + MCP cleanup
- [ ] **Phase 7** — Sends/returns + mixer view (§1.4 payoff, now just taps off the bus)

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

### Why channel-strip (and not full per-voice-tagging C)
The expensive parts of a general per-track-bus design (per-voice track tagging,
duplicated effect chains for a *shared* instrument) exist only to preserve
N-tracks → 1-instrument multitimbral sharing. Channel-strip drops that case by
construction (layering = multiple instruments/tracks), so those costs never appear.
CPU steady-state is essentially flat vs today in the 1:1 case — one extra buffer copy
and sum per channel.

### The invariant
Every `SequencerTrack` owns exactly one instrument; every audible instrument is
fronted by exactly one track. Layering uses multiple instruments/tracks. Per-note
`note.instrument` multitimbral routing is removed.

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
**Status:** ☐ Not started

The bus stage now exists; route the track controls into it. First user-visible win
(the dead-fader fix). Resolves the second §0.1 HIGH item and §2.1.

- [ ] Carry a per-channel track-control snapshot (vol, pan, mute, solo) into the bus
  stage; resolve via `InstrumentMapping` / `track.instrument`.
- [ ] Bus fader composes: `gain = inst.vol × track.vol`,
  `pan = clamp(inst.pan + track.pan, -1, 1)`, `audible = inst_audible & track_audible`
  (until Phase 4 optionally unifies them).
- [ ] Wire the `Track { Volume, Pan, Mute, Solo }` and `Global(Tempo)` →
  `transport.set_tempo` automation arms at `synth_engine.rs:2469` — they write to the
  same bus stage.
- [ ] Document shared-instrument behaviour as "last routed track wins per block"
  (removed in Phase 3).
- [ ] Engine test: bus-fader composition (track×inst, pan clamp, mute/solo gating).

### Phase 3 — Establish & enforce the 1:1 invariant
**Status:** ☐ Not started

- [ ] Track creation binds/creates an instrument; instrument creation surfaces a track.
- [ ] Guarantee `track.instrument == Some` for audible tracks (`track.rs:28`).
- [ ] Migration: for each placement ensure its track has an instrument; per-note
  instruments that differ from the track instrument → split into a new track or
  coerce to the track instrument. Pick and document the policy.
- [ ] Make `TrackMode` (`track.rs:40`, currently dead) meaningful → map onto the
  instrument's `AllocationMode`.
- [ ] Migration round-trip test (old song with per-note instruments → load → audio
  identical or documented re-interpretation).

### Phase 4 — Deprecate `note.instrument`
**Status:** ☐ Not started

- [ ] Route only via `track.instrument`: replace
  `effective_instrument = track_instrument.unwrap_or(note.instrument)`
  (`sequencer_engine.rs:443`) with `track.instrument`.
- [ ] Remove the `note.instrument` field (no backward compatibility required per
  CLAUDE.md). Update `pattern.rs` (`instrument_override.unwrap_or(...)`),
  the GUI per-note instrument selector → per-track, and MCP `add_notes`.
- [ ] (Recommended) Unify the faders: the track drives the bus fader directly (one
  fader, the purest channel strip), removing the "which fader?" ambiguity.

### Phase 5 — Orphan-preview via scratch channel
**Status:** ☐ Not started

Deprecating `note.instrument` removes orphan-preview's trackless routing, so this
phase pays that cost — and *removes* a whole code path in return.

- [ ] Represent preview as a temporary placement on a hidden scratch track bound to
  the chosen instrument, so the normal arrangement loop (and its bus) handle it.
- [ ] Delete the special `preview_pattern` branch (`sequencer_engine.rs:361`) and its
  field/setter (`:88`, `set_preview_pattern` at `:155`).
- [ ] Re-test the REC-arm flow on unplaced patterns (cf. `e674f39`, v0.290.0).
- [ ] Determinism test à la `tests/arrangement_render_determinism.rs` on
  preview-via-scratch-track.

### Phase 6 — Save/load + MCP cleanup
**Status:** ☐ Not started

- [ ] Confirm save/load round-trips without `note.instrument`; track↔instrument
  binding persists.
- [ ] MCP: deprecate per-note instrument in `add_notes`; align track/instrument
  color + description setters (pending §"Color fields" / §"Description fields" TODO
  items) with the unified model.

### Phase 7 — Sends/returns + mixer view (§1.4)
**Status:** ☐ Not started

The per-channel bus from Phase 1 already exists, so this is now just adding taps off
each bus — not building bus infrastructure.

- [ ] Add send taps off each channel bus (pre/post-fader configurable) → return
  busses with their own effect chains.
- [ ] Dedicated mixer view with faders, pan, sends, inserts.

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
