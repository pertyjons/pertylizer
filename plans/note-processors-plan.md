# Plan: Note Processors (generative articulation layer)

A non-destructive, composable layer that **expands a small set of source notes
into the actual note / gate / expression events at playback time** — arpeggiator,
ornaments (trill / mordent / turn / flam / drag / roll / grace), strum, chord,
scale-quantize, and humanize.

Carved out of the retired `note-expression-roadmap.md` (its Phase E, primitive 4).
The rest of that Phase E — per-note hand-drawn curves, MPE input, per-note AWE
spatial — is **deliberately not here**: it needs MPE hardware and an expensive
piano-roll curve editor, and is iceboxed in `plans/TODO.md §3.3`. Note Processors
are the one slice of Phase E with broad, daily use (the arpeggiator alone), and
they need none of that UI investment, so they stand on their own.

## Why this is a coherent unit (the taxonomy)

The retired roadmap classified all musical-expression terms into **four orthogonal
primitives**, with the rule *place the primitive, not the term*:

1. **Per-note modulator on a generic target** — vibrato/tremolo/auto-pan/brightness
   (shipped in miniature as Phase C's expression block).
2. **Inter-note transition** — portamento/glide/glissando/legato (shipped as Phase B).
3. **Note-shape scalars** — accent, staccato/tenuto, ghost, probability (additive
   `Note` fields, shipped with B/C).
4. **Generators / ornaments** — things that *expand* into primitives 1–3 or into
   **extra notes**, and are therefore **not storage**. ← **this plan.**

The payoff of the taxonomy: most "features" here are the *same generator* with
different parameters. **Trill / mordent / turn = a two-note arpeggio.**
**Flam / drag / ruff / roll = a timed-repeat generator.** Build the generator
once, parameterize it — never ship "trill", "mordent", and "turn" as three features.

**Guardrail (one sentence):** if a requested articulation expands into more notes or
into primitives 1–3, it is a Note Processor — do **not** add a `Note` field for it.

## Status at a glance

- [ ] **NP0** — Architecture decision: expansion model + attachment scope (gating)
- [ ] **NP1** — `NoteProcessor` engine + the playback-time expansion point
- [ ] **NP2** — Arpeggiator (flagship; trill/mordent/turn are special cases of it)
- [ ] **NP3** — Timed-repeat ornaments: flam / drag / ruff / roll + grace note
- [ ] **NP4** — Chord + strum
- [ ] **NP5** — Pitch/statistical processors: scale-quantize + humanize
- [ ] **NP6** — GUI (per-track rack + per-note ornament menu)
- [ ] **NP7** — MCP surface
- [ ] **X-cut** — Persistence (round-trip) + ordering/chaining + RT-safety

Build order is value-first: NP0 → NP1 → **NP2 (stop here and ship if that's enough)**
→ NP3/NP4/NP5 as appetite allows.

---

## Context — verified engine state (carry-over from the retired roadmap)

These were confirmed against live code during the note-expression work and remain
the relevant facts for this plan:

- **`Note` is 6 fields** (`id, start, duration, pitch, velocity, track`),
  `crates/synth_sequencer/src/note.rs` — plus the per-note legato/glide (Phase B)
  and the expression block (Phase C) added since.
- **The allocator already does mono/legato/glide.** `AllocationMode::{Polyphonic,
  Mono, Legato, Unison}` (`voice_allocator.rs`), `GlideState` exponential portamento
  (`voice.rs`). A generator that emits tied/legato notes rides this for free.
- **Playback dispatch lives on the audio thread.** Notes are collected per tick in
  `SequencerEngine` (`crates/synth_engine/src/sequencer_engine.rs`,
  `collect_events_at_tick`) and routed via `route_sequencer_events`
  (`synth_engine.rs`). **A naive "generate extra notes here" allocates on the audio
  thread** — see the RT-safety cross-cutting section; this is the single biggest
  design constraint.
- **Automation/override groundwork is done.** `AutomationTarget::Module` addressing
  (`module_type` + `instance` + `param_id`) and the base-vs-override value model
  (`PolyModule::set_param_override`) shipped with A2/Track-F. Generated notes that
  carry expression reuse this, not a new path.

---

## NP0 — Architecture decision (the gating step)

Two decisions must be locked before any code; both shape everything after.

### Decision 1 — Expansion model: playback-time vs edit-time bake

| | **Model B — playback-time expand (recommended)** | **Model A — edit-time bake** |
|---|---|---|
| Source note stays | one note in the pattern | replaced by N concrete notes |
| Edit/undo | edit the *processor params*, source intact | edit the generated notes directly |
| Piano-roll truth | shows intent (1 note + a badge) | shows the literal result |
| Cost | must expand on/near the audio thread (RT-safety) | expansion is a UI-thread op, audio path unchanged |
| Round-trip | store processor config | store the baked notes (free — already notes) |

**Recommendation: Model B, with a "Freeze to notes" command** that one-shot bakes a
processor into concrete notes (= Model A on demand) for hand-editing. This matches
DAW convention (Ableton/Bitwig arpeggiators are live, with a "to MIDI" bake) and
keeps the source musically legible. The cost is the RT-safety work in NP1 — accept it.

> **Open sub-question for NP0:** does expansion run *inside* `SequencerEngine` on the
> audio thread (sample-accurate, but must be alloc-free/bounded), or in the snapshot/
> collection step just ahead of it (simpler, slightly coarser timing)? Lean audio-thread
> with a **pre-allocated bounded expansion buffer** so arp/roll timing stays tight; fall
> back to snapshot-step if the bound proves awkward. Lock this in NP0.

### Decision 2 — Attachment scope: where a processor lives

Generators split cleanly into **two scopes** — the plan needs both:

- **Region/track-scoped generators** — apply to a stream of notes: **arpeggiator**,
  **humanize**, **scale-quantize**, **chord**. Natural home: a small ordered
  *processor rack* on the `SequencerTrack` (or on a `PatternPlacement`). One arp over a
  held chord is the canonical case.
- **Per-note ornaments** — attach to a single note: **trill/mordent/turn**, **flam/
  drag/roll**, **grace note**, **strum** (strum attaches to a chord = a note cluster).
  Natural home: an `Option<Ornament>` on `Note` (a `Copy` enum + small params, so it
  stays RT-cheap), set via a piano-roll context menu.

Lock the two storage sites in NP0 so NP1's trait can serve both.

**Deliverable of NP0:** a one-page decision doc appended here (expansion model,
expansion location, the two storage sites, and the chaining order from the X-cut
section) — then the checkboxes below become buildable.

---

## NP1 — `NoteProcessor` engine + expansion point

The shared machinery every generator plugs into. Build it once.

- [ ] Define a `NoteProcessor` abstraction that takes *(source notes in a window,
  transport/tempo, a bounded output sink)* and emits `NoteOn`/`NoteOff` (and, where
  relevant, the Phase C expression block) into the sequencer event stream. Generated
  notes are **first-class** — they carry velocity, gate %, legato/glide, and
  expression so they reuse Phase B/C, not a parallel path.
- [ ] Implement the expansion point chosen in NP0 (audio-thread bounded buffer, or
  snapshot step). Pre-allocate the output buffer to a hard cap (e.g. max
  notes-per-tick) and **`log`/drop with a documented policy** if a pathological
  config (e.g. a 1 ms roll) overflows it — never allocate to grow.
- [ ] "Freeze to notes" command: run a processor once over its region and replace it
  with the concrete notes (the Model-A escape hatch). Undoable.
- [ ] Lock the chaining order (X-cut) in the engine so a track with multiple
  processors is deterministic.

---

## NP2 — Arpeggiator (flagship)

The single highest-value item; ship the plan here if appetite is limited. Region/
track-scoped (Decision 2). Trill/mordent/turn fall out as **presets of this same
generator** — do not build them separately.

- [ ] Arp over the currently-held notes in its region: **mode** (up / down / up-down /
  down-up / as-played / random / chord), **rate** (sync division: 1/4…1/32, dotted,
  triplet), **octave range** (1–4), **gate %** (reuses primitive 3), **swing**,
  **note order / step pattern**.
- [ ] **Latch** option (hold the last chord) and **velocity mode** (as-played /
  ramp / pattern).
- [ ] Tempo-sync via the transport already feeding `SequencerEngine`; respect host
  swing.
- [ ] **Trill / mordent / turn = arp presets.** Trill = a 2-note up-down arp at a
  fast rate over `{note, note+interval}`; mordent = a single fast alternation;
  turn = a 4-step figure. Expose these as one-click presets on the per-note ornament
  menu (NP6) that instantiate a constrained arp — *not* a second code path.
- [ ] Export payoff (ties into the `sid-analyzer` origin of the parent roadmap):
  an arpeggio that today replays as machine-gun re-triggered notes can instead be
  authored as *one chord + an arp processor*.

---

## NP3 — Timed-repeat ornaments: flam / drag / ruff / roll + grace note

Per-note scope (Decision 2). All are **one generator**: emit N copies of the note
offset in time, with a velocity/spacing curve.

- [ ] A timed-repeat ornament on `Note`: **count** (flam = 1 grace + main; drag = 2;
  ruff = 3; roll = N), **spacing** (absolute ms or synced), **spacing curve**
  (accelerating/decelerating/even), **velocity curve** (crescendo/decrescendo),
  **lead-in vs centred** (grace notes before the beat vs on it).
- [ ] **Grace note / acciaccatura** = a count-1 timed-repeat with a pitch offset on
  the grace and a very short gate — same generator, pitch-offset param.
- [ ] RT-safety: a roll's count must be bounded by the NP1 cap.

---

## NP4 — Chord + strum

- [ ] **Chord generator** (region-scoped): expand a single source note into a chord
  by interval set / named quality (maj/min/7/sus…), optionally scale-aware (composes
  with NP5 scale-quantize). Feeds the arp naturally (chord → arp is the classic chain).
- [ ] **Strum** (per-note/cluster scope): offset the onsets of a chord's notes by a
  small per-note delay (up/down/in/out, time spread, optional velocity spread). Strum
  is just chord-expansion + a time offset per generated note — reuse NP1's emit path.

---

## NP5 — Pitch & statistical processors: scale-quantize + humanize

These transform notes rather than multiplying them, but they are still generators
(non-destructive, playback-time) so they live in the same rack.

- [ ] **Scale-quantize**: snap generated/source pitches to a scale (root + scale
  table). Especially valuable *upstream* of the arp and chord generators so randomized
  or interval-built notes stay in key. Reuse any tuning table work (`plans/TODO.md
  §3.2` Scala support) if it lands.
- [ ] **Humanize**: bounded random offsets on timing, velocity, and (optionally)
  micro-pitch, with a seed so a render is reproducible (mirror the offline-render
  determinism constraint used elsewhere — no `Math.random()` on the audio thread; use
  a seeded PRNG seeded per region/placement).

---

## NP6 — GUI

- [ ] **Per-track processor rack** (region-scoped generators): an ordered, add/
  remove/reorder list on the track header or a dedicated panel — mirrors the existing
  effect-chain rack UI so it's familiar. Each processor gets the descriptor-driven
  param grid (`gui/widgets/param_grid.rs`) for free if its params are descriptor-backed.
- [ ] **Per-note ornament menu** (per-note scope): piano-roll right-click → "Add
  ornament…" with the trill/mordent/turn/flam/drag/roll/grace presets; a small badge
  on ornamented notes.
- [ ] **Freeze to notes** action in both places (NP1).
- [ ] Visualize generated notes faintly in the piano roll (ghosted) so the user sees
  what they hear without the source becoming uneditable.

---

## NP7 — MCP surface

Mirror the existing effect-chain MCP shape (`add_*_effect` / `set_*_effect_parameter`
/ `reorder_*_effect`) so AI composition can drive generators:

- [ ] `add_note_processor(track_id, kind, params)` / `remove_note_processor` /
  `set_note_processor_parameter` / `reorder_note_processor`.
- [ ] `set_note_ornament(pattern_id, note_id, ornament)` / clear with `""`/`null`.
- [ ] `freeze_note_processor(...)` → bakes to notes (the Model-A escape hatch over MCP).
- [ ] Surface configured processors/ornaments in the relevant `list_*` / `get_*_info`
  readers so AI can read existing intent (parallels the description/color MCP work).

---

## Cross-cutting

### Chaining order (deterministic)
Lock one order so a stacked rack is predictable. Proposed:
**scale-quantize → chord → arp → ornaments/strum → humanize.**
(Quantize first so everything downstream stays in key; humanize last so it perturbs
the final result.) Document on the `NoteProcessor` engine; lock with a test.

### Persistence (must round-trip on save/load)
Same discipline as `Patch.description` / color: processors and ornaments must survive
project save/load **and** standalone-pattern operations, or AI/user-authored
articulation silently vanishes.
- [ ] Project save/load round-trip test: add an arp + a per-note trill → `save_project`
  → `new_project` → `load_project` → both are present and play identically.
- [ ] No partial states: do not ship a processor whose config is set live but not
  persisted; document as known-broken until both halves land.

### RT-safety (the hard constraint)
Expansion touches the audio thread (per NP0 Decision 1). Forbidden on that path:
heap allocation, blocking locks, unbounded loops, `Math.random()`. Required:
pre-allocated bounded output buffer, seeded PRNG for humanize, a documented
drop/log policy on overflow.

---

## Critical files

| Concern | File |
|---|---|
| Note model (ornament field) | `crates/synth_sequencer/src/note.rs` |
| Track (processor rack) | `crates/synth_sequencer/src/song.rs` (`SequencerTrack`) |
| Playback expansion point | `crates/synth_engine/src/sequencer_engine.rs` (`collect_events_at_tick`) |
| Event routing | `crates/synth_engine/src/synth_engine.rs` (`route_sequencer_events`) |
| Allocator (legato/glide reuse) | `crates/synth_engine/src/voice_allocator.rs`, `voice.rs` |
| Effect-chain rack (GUI pattern to mirror) | `crates/pertylizer/src/gui/widgets/param_grid.rs` + mixer/return-insert UI |
| MCP effect-chain shape to mirror | `crates/synth_mcp/src/server.rs` (`add_*_effect` family), `crates/pertylizer/src/mcp_bridge.rs` |
