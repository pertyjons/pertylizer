# Execution plan: Phases B & C (per-note legato/glide + expression block)

Companion to `docs/note-expression-roadmap.md` (the north-star roadmap). This is
the **commit-sized execution plan** for Phase B and Phase C, sequenced for a
step-by-step loop: each step is one logical commit, **preceded by a
`/code-review`** and the standard build gate (`cargo fmt --check && cargo build
&& cargo clippy --all-targets && cargo test`).

Read the roadmap's two governing sections before starting and keep them open:

- **Expression primitive taxonomy** (roadmap §"expression primitive taxonomy") —
  *place the primitive, not the term*. B is primitive 2 (inter-note transition);
  C is primitive 1 (parametric modulator) + primitive 3 (note-shape scalars).
- **Automation value model** (base vs. override) — vibrato (C) rides the
  **additive** mod-matrix offset path, never the destructive `set_param`.

## Loop protocol (every step)

1. Implement exactly the one step.
2. `/code-review` the working tree (effort per the step's note; default `medium`,
   `high` for the two audio-thread steps B3/C4). Apply or consciously dismiss
   each finding.
3. Build gate: `cargo fmt --check && cargo build && cargo clippy --all-targets && cargo test` — all zero-warning.
4. Run gen_schemas if they are updated
5. Commit with the step's suggested message. Do **not** bundle two steps.
6. Flip the step's checkbox here; flip the roadmap `Status` line only when the
   whole phase lands; append a one/two-sentence `docs/history.md` line at phase end.

## Verified code anchors (confirmed 2026-05-29)

| Concern                                                                     | Location                                                                                                                              | 
|-----------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------|
| `Note` struct (6 fields, additive target)                                   | `synth_sequencer/src/note.rs:13-26`                                                                                                   |
| Seq event model (`SequencerEvent::NoteOn` = tick/pitch/velocity/instrument) | `synth_sequencer/src/events.rs:15-44`                                                                                                 |
| Seq playback emission (note-on push + placement-boundary legato coalesce)   | `synth_engine/src/sequencer_engine.rs:514-540`                                                                                        |
| Audio-thread event consumer → `instruments[idx].note_on(note, vel)`         | `synth_engine/src/synth_engine.rs:2617-2660`                                                                                          |
| `Instrument::note_on` → `allocator.note_on`                                 | `synth_engine/src/instrument.rs` (`note_on`), `voice_allocator.rs:242-252`                                                            |
| `AllocationMode::{Polyphonic,Mono,Legato,Unison}`                           | `voice_allocator.rs:17-28`                                                                                                            |
| Legato glides pitch without re-gate; mono retriggers w/ glide               | `voice_allocator.rs:248-258`                                                                                                          |
| `GlideState::start(target_freq, glide_time)` (exp portamento)               | `voice.rs:172-191`                                                                                                                    |
| Per-instrument glide config                                                 | `voice_allocator.rs` config `glide_time`; setter `voice.rs:405`; cmd `synth_engine.rs:1358,1699`                                      |
| Mod-matrix pitch dest `ModDestination::OscPitch(u8)` (semitones, additive)  | `synth_core/src/params/mod_matrix.rs:255-260,388`                                                                                     |
| Override layer (Phase A1/A2) — additive offset model                        | `instrument.rs:951` (`apply_normalized_override`), `instrument.rs:982` (`apply_module_param_override`); `voice.rs:389`/`graph.rs:512` |
| GUI piano-roll                                                              | `crates/pertylizer/src/gui/sequencer/mod.rs` (`collect_piano_roll_data`, automation picker ~3375)                                     |
| MCP note creation                                                           | `crates/pertylizer/src/mcp_bridge.rs` (`add_notes`)                                                                                   |

**RT-safety constraint (governs B2/B3/C2/C3/C4):** `SequencerEvent::NoteOn` is
consumed on the audio thread (`synth_engine.rs:2617`) and the event vec is cloned
per tick in `sequencer_engine`. Every per-note expression field added to the event
must be `Copy` and alloc-free (`f32`/`bool`/small `enum`/`Option<Copy struct>`) —
no `String`, no `Vec`. This is the lesson from the deferred A2 `param_id: String`
RT-safety item; do not repeat it here.

---

## Phase B — Per-note legato/tie + glide

**Goal (roadmap):** retire the two worst pitch artifacts — machine-gun arpeggio
re-gating and portamento staircases — by driving the *existing* allocator
Legato + `GlideState` machinery from per-note sequencer data. The DSP exists;
this is data + wiring. Taxonomy: **primitive 2** (inter-note transition),
parameterised on one axis — interpolation type (continuous = portamento,
stepped = glissando).

### B1 — Note data model: `legato` flag + `Glide` value type ✅ (4115525)

- [x] In `note.rs`, add two additive fields to `Note`:
    - `legato: bool` (tie / no-retrigger intent for this note onto its successor).
    - `glide: Option<Glide>` where `Glide` is a new `Copy` struct in `note.rs`:
      `{ from: GlideFrom, time: Milliseconds, interp: GlideInterp }`.
        - `GlideFrom` enum: `Semitones(Semitones)` (signed, relative) or
          `Pitch(Pitch)` (absolute source). Default to relative semitones.
        - `GlideInterp` enum: `Continuous` | `Stepped` — the single taxonomy axis
          separating portamento from glissando. (`Stepped` is a stub here; honoured
          in B3.)
    - Use existing newtypes (`Milliseconds`/`Seconds`, `Semitones`, `Pitch`) — no
      raw `f32`. Per CLAUDE.md newtype rule.
- [x] `#[serde(default)]` on both new fields so existing `.pertyproj` /
  pattern JSON loads unchanged (additive schema). Derive `Copy` is impossible
  (`Note` already isn't `Copy`); ensure `Glide`/`GlideFrom`/`GlideInterp` are
  `Copy + PartialEq + Serialize + Deserialize + JsonSchema`.
- [x] Builder methods `Note::with_legato(bool)` / `with_glide(Glide)` (`#[must_use]`).
- [x] Unit tests: serde round-trip incl. defaults-absent JSON; builder set/clear.
- **Verify:** no behavior change yet (engine ignores the fields). `/code-review` medium.
- **Commit:** `Phase B1: add per-note legato + Glide value type to Note (additive)`
- **Done:** types exported from crate root; schema regenerated; 292 tests green.

### B2 — Sequencer event plumbing ✅ (next commit)

- [x] Extend `SequencerEvent::NoteOn` (`events.rs:17`) with `legato: bool` and
  `glide: Option<Glide>` (both `Copy` — RT-safety constraint above).
- [x] In `sequencer_engine.rs:534` emission, populate the two new fields from the
  source `Note`. Introduced a `Copy` `PendingNote` scratch struct (replaces the
  4-tuple) carrying the fields alloc-free. Documented the **superset** interaction
  with the placement-boundary legato coalesce as a comment; the actual coalesce
  *generalisation* (suppress NoteOff+NoteOn across any successor + glide) is
  deferred to B3 so B2 stays behavior-neutral.
- [x] Update `events.rs` constructors/tests; consumer (`synth_engine.rs:2617`)
  uses `..` so it ignores the new fields — no exhaustive match broke.
- [x] Absolute `GlideFrom::Pitch` source is transposed into the placement key at
  emission; review fix: drop the glide if that transpose leaves MIDI range
  (avoids a desynced source/target pair) rather than `unwrap_or(p)`.
- **Verify:** events now carry the data; consumer still ignores it → no audible
  change. Confirm existing sequencer_engine legato tests still pass. `/code-review` medium.
- **Commit:** `Phase B2: carry per-note legato/glide through SequencerEvent::NoteOn`
- **Done:** full suite green; code-review (1 low-sev transpose-desync finding) applied.

### B3 — Engine consumption: drive Legato + GlideState per note *(audio thread)* ✅ (next commit)

- [x] Threaded a `Copy` engine-native `NoteTrigger { legato, glide: Option<GlideSpec> }`
  (`GlideSpec { from_offset: Semitones, time, stepped }`) down
  `Instrument::note_on_expr` → `VoiceAllocator::note_on_expr` → `Voice::note_on_expr`/
  `glide_to_note_expr`. The old `note_on`/`glide_to_note` are now thin
  default-trigger wrappers (behavior-preserving, verified equivalent in review).
- [x] Consumer (`synth_engine.rs` `note_trigger()`) reads `legato`/`glide` off the
  event and builds the trigger alloc-free; absolute `GlideFrom::Pitch` → a
  target-relative semitone offset (transpose-invariant), so the engine never sees
  sequencer `GlideFrom`.
- [x] Drive machinery: `GlideState::start_from(from, to, time, stepped)` seeds an
  explicit source; `legato` forces the no-retrigger `glide_to_note` path
  regardless of allocation mode; `GlideInterp::Stepped` quantises the trajectory
  to integer semitones in `update()` (control-rate per block; div-by-zero guarded).
- [x] Precedence documented at the dispatch: per-note `glide.time` overrides the
  instrument `glide_time`; absent per-note glide → instrument default (preserved).
- [x] Tests: per-note legato on a poly allocator → one voice, no retrigger
  (start_time unchanged); per-note glide seeds explicit source/target; stepped
  glide holds at semitones; `start_from` seeds both endpoints.
- **Verify (the audible payoff):** arpeggio of tied notes plays under one held
  gate; slides ramp. `/code-review` **high** (audio-thread, RT-safety). Confirm
  no heap/lock/panic added in `process()`/trigger path.
- **Commit:** `Phase B3: drive per-note legato + glide from the sequencer trigger`
- **Done:** full suite green; high code-review = 2 finders, both `[]`.
- **Deferred (recorded):** per-note glide is **dropped on a stolen voice**
  (`steal_for`/`pending_note` carry no `NoteTrigger`). Acceptable first cut — note
  still sounds, just without its glide. Fix later by carrying the trigger through
  the steal fade-out.

### B4 — GUI: piano-roll tie/legato toggle + glide handle ✅ (next commit)

- [x] Added per-note editing to the **selection inspector**
  (`draw_piano_roll_selection_inspector`): a "Tie" (legato) toggle, a "Glide"
  enable toggle (installs a sensible default), and From (semitone offset) / Time
  (ms) / Stepped controls. (Chose the inspector over canvas drag-handles — more
  robust + matches the existing velocity multi-edit pattern.)
- [x] Full **undo** integration: new `SetLegatoBatch`/`SetGlideBatch` actions
  (`undo.rs` enum + invert, `egui_backend.rs` apply) and `Pattern::set_note_legato`
  /`set_note_glide`. Glide DragValues collapse a drag into one undo entry on release.
- [x] Visual markers in the note-draw loop: a tie underline (accent-yellow) for
  legato and a left-edge ramp glyph (accent-cyan) for glide. Snapshot pattern kept
  (`PianoRollNote` carries `legato`/`glide`).
- **Verify:** round-trip edit → save → reload preserves fields; playback matches
  B3. `/code-review` medium.
- **Commit:** `Phase B4: piano-roll tie/legato toggle + glide handle`
- **Done:** gate green; code-review fix applied — From/Time edits only touch
  already-gliding notes (never force glide onto a mixed selection). Pre-existing
  scroll-wheel-without-drag undo gap left as-is (mirrors velocity inspector).

### B5 — MCP: expose legato/glide on note creation ✅ (next commit)

- [x] `NoteInput` (`synth_mcp/server.rs`) gains optional `legato: bool` and
  `glide { from_semitones | from_pitch, time_ms, interp }`; `BridgeNoteData`
  gains `legato` + `BridgeGlide`. A new `note_input_to_bridge` helper centralises
  the (previously 4×-duplicated) `NoteInput → BridgeNoteData` mapping. Forgiving
  parsing: `interp` accepts stepped/step/glissando/gliss (else continuous);
  `from_pitch` precedence; defaults time 100 ms, offset −2 st.
- [x] Both insert paths (`try_insert_note_into_pattern` + bulk
  `insert_note_into_pattern`) resolve the glide via `glide_from_bridge` and apply
  legato/glide, so inline-note creation gets expression too.
- [x] Round-trip tests (relative glide + stepped; absolute `from_pitch`
  precedence) + a `README_MCP` per-note-expression section.
- **Verify:** create a tied/gliding note via MCP, confirm it plays. `/code-review` medium.
- **Commit:** `Phase B5: MCP add_notes accepts per-note legato/glide`
- **Done:** gate green; review fixes applied — validate `glide.from_pitch`
  (range) + `glide.time_ms` (finite, 0..60000) at the MCP boundary; interp aliases.

### B-close ✅ (next commit)

- [x] Flipped roadmap **Phase B** `Status` → ☑ Done (v0.293.0) and the §"Status at
  a glance" checkbox. Added a `docs/history.md` 0.293.0 section; bumped
  `pertylizer` to 0.293.0. Final full build gate green. (Docs/version only — no
  code changed since B5's review, so no fresh code-review needed.)
- **Commit:** `Phase B: history + roadmap status`

---

## Phase C — Per-note vibrato + expression block

**Goal (roadmap):** make vibrato (today only patch-level via mod-matrix
LFO→OscPitch) **per-note**, and seed the miniature note-expression block.
Taxonomy: **primitive 1** (parametric modulator: depth/rate/delay → an additive
offset on a target, reusing the mod-matrix `OscPitch` path) **plus primitive 3**
(note-shape scalars — accent, gate %, ghost, probability — the only terms that
justify new `Note` fields). Glide *time* stays in B (primitive 2), not here.

**Field-shape guardrail (roadmap C bullet 2 + taxonomy consequence 1):** design
the expression block against the **full** MPE/MIDI-2.0 minimal dimension set —
*bend, pressure, timbre/slide, velocity, release-velocity* — so Phase E's
hand-drawn curves *extend* this block rather than replace it. Pick field names
that map 1:1 onto those dimensions now even if only vibrato is wired in C.

### C1 — Note expression-block data model ✅ (next commit)

- [x] Added `Note.expression: Option<NoteExpression>` (`#[serde(default)]`) plus
  `NoteExpression`/`Vibrato`/`VibratoShape` (`Copy + PartialEq + serde + JsonSchema`):
    - **primitive 1:** `vibrato: Option<Vibrato>` = `{ depth: Semitones, rate: Hertz,
      delay: Milliseconds, shape: VibratoShape }`. Used a self-contained
      `VibratoShape` (Sine/Triangle/Square/Saw) rather than coupling to a core LFO
      enum (consistent with B's local `GlideInterp`); the engine maps it in C4. The
      block's doc comment names the intended MPE dimension set so Phase E extends it.
    - **primitive 3:** `accent: Option<f32>` (velocity ×), `gate: Option<NormalizedValue>`,
      `ghost: bool`, `probability: Option<NormalizedValue>`.
- [x] Builders + serde tests (absent block → `None`; present-but-partial block
  fills field defaults — inner `#[serde(default)]` per review). Schema regenerated.
- **Verify:** additive, no behavior change. `/code-review` medium.
- **Commit:** `Phase C1: add per-note NoteExpression block (vibrato + note-shape scalars)`
- **Done:** review verdict — `accent: f32` kept (dimensionless ratio, no fitting
  newtype); fix applied — inner `NoteExpression` fields are `#[serde(default)]` so
  partial blocks load.

### C2 — Sequencer event plumbing

- [ ] Carry `expression: Option<NoteExpression>` (`Copy`) on `SequencerEvent::NoteOn`
  (RT-safety constraint). Populate from `Note` at emission (`sequencer_engine.rs:534`).
- [ ] **Probability** is resolved sequencer-side at emission, not on the audio
  thread (avoids RNG in `process()` and keeps the event deterministic per the
  no-`Math.random` posture). Decide note inclusion before pushing `NoteOn`; seed
  source TBD (pattern/tick-derived, documented).
- **Verify:** events carry the block; consumer ignores (except probability gating
  the emit). `/code-review` medium.
- **Commit:** `Phase C2: carry NoteExpression through SequencerEvent::NoteOn`

### C3 — Engine consumption: note-shape scalars *(cheap, additive)*

- [ ] At the trigger consumer, apply the primitive-3 scalars — all `Copy`, no DSP:
    - `accent` → multiply velocity before `note_on`.
    - `gate` → scale the effective note length / schedule the note-off earlier
      (staccato/tenuto). Implement at the sequencer note-off scheduling
      (`check_note_offs`, `sequencer_engine.rs:576`) or as a per-note release —
      pick the one that keeps gate a function of duration only.
    - `ghost` → velocity floor / fixed low velocity.
    - (`probability` already handled in C2.)
- [ ] Tests for each scalar's effect on the emitted/triggered note.
- **Verify:** accents/staccato audible; defaults unchanged. `/code-review` medium.
- **Commit:** `Phase C3: apply per-note note-shape scalars (accent/gate/ghost)`

### C4 — Engine consumption: per-note vibrato via mod-matrix offset *(audio thread)*

- [ ] Wire `Vibrato` to the **additive** per-voice pitch-offset path — the
  generalised mod-matrix `OscPitch` route (`mod_matrix.rs:255-260,388`), **not**
  `set_param` (roadmap value-model section). A per-voice LFO (depth/rate/delay/shape)
  adds semitone offset on top of base pitch, reset per note — exactly the
  mod-matrix model, now seeded per note instead of per patch.
- [ ] Reuse, don't duplicate: drive the existing per-voice OscPitch offset
  accumulator if the voice already exposes one; otherwise add a minimal per-voice
  vibrato LFO that feeds the same offset sum the mod matrix writes to (so mod-matrix
  vibrato + per-note vibrato compose additively — consistent with the documented
  combine rule).
- [ ] `delay` → onset ramp of depth after note start. `shape` → LFO waveform.
  All pre-allocated / atomics; for-loops in the sample path. No heap/lock/panic.
- [ ] Tests: a vibrato note produces periodic pitch modulation of the right
  depth/rate after the delay; zero depth = no change.
- **Verify:** dead leads gain motion per note. `/code-review` **high** (audio-thread,
  RT-safety). Confirm additive-offset (base pitch untouched) and no allocation.
- **Commit:** `Phase C4: per-note vibrato via additive mod-matrix OscPitch offset`

### C5 — GUI: expression editing in the piano-roll

- [ ] Per-note expression affordances: accent/velocity tweak, gate-length handle,
  ghost toggle, probability, and a vibrato mini-control (depth/rate). Keep it
  compact — this block is the seed of the Phase E curve UI; lay out so a future
  curve editor extends it.
- **Verify:** edit → save → reload → playback. `/code-review` medium.
- **Commit:** `Phase C5: piano-roll per-note expression editing`

### C6 — MCP: expose the expression block

- [ ] Extend `add_notes` with optional `expression { vibrato{depth,rate,delay,shape},
  accent, gate, ghost, probability }`; forgiving tokens; defaults = current.
  Round-trip test + `README_MCP` note.
- **Verify:** MCP-created expressive note plays. `/code-review` medium.
- **Commit:** `Phase C6: MCP add_notes accepts NoteExpression block`

### C-close

- [ ] Flip roadmap **Phase C** `Status` ☑ Done + glance checkbox; `docs/history.md`
  line. Final build gate.
- **Commit:** `Phase C: history + roadmap status`

---

## Notes / deferred-out-of-scope (record, don't silently drop)

- **Stable (non-positional) module identity** and **mod-matrix-vs-automation
  combine ordering** remain the A1/A2 deferred items (`docs/TODO.md`); C4's
  additive composition assumes the documented "override replaces base, mod offset
  added on top" rule and does not re-open ordering.
- **MPE input mapping**, **hand-drawn per-note curves**, **per-note AWE spatial**,
  and **Note Processors (arp/trill/flam/strum)** are all **Phase E** — C only
  designs the block's field shape so E extends it. Do not implement them here.
- **Offline-render parity** for any new per-note expression inherits the existing
  deferred `analyze_*` offline-render item; flag if a C step makes analysis read a
  value the live engine never produced.
