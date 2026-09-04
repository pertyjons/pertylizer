# SPEC: Project Lowering and Fidelity Contract

| Field            | Value                        |
|------------------|------------------------------|
| Status           | Current                      |
| Phase            | 4                            |
| Created          | 2026-09-02                   |
| Last reviewed    | 2026-09-04                   |
| Based on         | ADR-0057, ADR-0056, ADR-0025 |
| Invariant prefix | LOWER                        |
| Supersedes       | —                            |
| Superseded by    | —                            |

Allowed status values are defined in [../specs/README.md](README.md). Only a `Current`
specification constrains implementation.

## Scope

What the V1-to-V2 project lowerer must do: what it produces for a saved project it can
represent, what it produces for one it cannot, and what may be claimed about the result.
It governs `crates/pertylizer/src/lowering/` and every current or future consumer of a
lowered outcome.

This contract exists because
[`spec-sound-core-render-contract.md`](spec-sound-core-render-contract.md) deliberately
excludes project lowering, while the rules below bind implementation now.

## Non-goals

It does not define the V2 render plan, the compiler, event scheduling or the note payload —
those are the Sound Core render contract's, and `SOUND-INV-021` owns the payload's
magnitudes. It does not decide how V1's two velocity sensitivities compose; that is Phase
6's composition law, and `LOWER-INV-003` is what holds until it exists. It does not define
a comparison harness, a shared render request or a job contract; ADR-0028 owns those and
Phase 10B accepts it. It does not decide which V1 module types are supported, which is a
per-phase subset question rather than a contract.

## Terminology

- **Lowering** — the pure transformation from a saved V1 project into a V2 `ProjectGraphIr`
  and its events. It reads a project, never a file, and never the live engine.
- **Outcome** — a lowering's result: its plan or refusal, its diagnostics, and the counts
  taken while producing them.
- **Fidelity** — whether an outcome represents everything the project asked for.
- **Parity verdict** — any claim that a V2 render reproduces V1's for a corpus case,
  including a per-claim judgement in an A/B report.

## Accepted decisions

| ADR | Decision it fixes here |
|-----|------------------------|
| [ADR-0057](../decisions/ADR-0057-refuse-parity-verdict-over-a-placed-note.md) | A lowering that places a note is `UnsupportedScope`, and no parity verdict may read such an outcome, until Phase 6's composition law |
| [ADR-0056](../decisions/ADR-0056-v1-to-v2-consumer-boundary.md) | The lowerer lives in `pertylizer` behind a non-default feature; V2 is reached only through it |
| [ADR-0025](../decisions/ADR-0025-tuning-representation-and-ownership.md) | A note's magnitudes are a validated key identity and a velocity, so the lowerer sends a saved note's own values rather than substitutes |

## Invariants

1. **LOWER-INV-001** — Every lowering produces a **typed** diagnostic for anything it
   refuses or cannot represent. A diagnostic names its **subject** — the project object, as
   a closed enum — and its **reason**, also a closed enum. Free text is not a diagnostic.
   A refusal carries `Severity::Refused` and produces no plan; something unrepresented
   carries `Severity::Unrepresented` and lowering continues.

2. **LOWER-INV-002** — An outcome's fidelity is **derived** from its diagnostics and is
   never set independently. It is `Faithful` exactly when the diagnostic set is empty, and
   `UnsupportedScope` otherwise. An outcome that holds a diagnostic and claims `Faithful`
   must not be constructible.

3. **LOWER-INV-003** — **No parity verdict may read an outcome that is not `Faithful`.**
   While V1's two velocity sensitivities and their composition are unimplemented in V2, a
   lowering that **produces a performance placing a note** raises exactly one `Unrepresented`
   diagnostic naming that capability and its owning phase, and is therefore
   `UnsupportedScope`. It is raised once per lowering, not once per note: it is a property
   of how the lowerer composes velocity, not of any note the project holds. The render
   itself is unaffected and still happens.

   **A lowering that refuses earlier does not carry this marker, and does not need to.** A
   refused graph, an unreadable arrangement, an overlap or a note expression stops before a
   performance exists; the outcome is `UnsupportedScope` through that refusal's own
   diagnostic, and the first sentence — which is the one a comparison caller must obey — holds
   for it either way. The marker's guarantee is about lowerings that **succeed**.

   This invariant scopes the prohibition to **lowered outcomes**. A controlled V1/V2
   comparison that does not go through the lowerer — an evidence harness building its own
   fixture patches, such as EVD-0013's — is outside it and remains valid.

4. **LOWER-INV-004** — Compatibility knowledge lives in the lowerer and nowhere else. Every
   V1/V2 asymmetry the supported subset can reach is represented or refused here, and V1's
   own defaults and clamps are **read from V1's descriptors** rather than transcribed. The
   V2 render plan carries none of it and must not depend on the lowerer.

   **Every saved field has a stated disposition, and something mechanical asks for it.** Two
   mechanisms, because the types live in two crates. `InstrumentState`, `SequencerTrack` and
   `GlobalProjectState` are destructured **exhaustively, without `..`**, so a new saved field
   is a compile error at the disposition site; `TrackMode` is matched exhaustively for the same
   reason. `Song` and `Pattern` belong to `synth_sequencer` and expose their contents through
   accessors, so they cannot be destructured from here; `Note` and `PatternPlacement` could be,
   and are pinned the same way so that the four persisted lists sit in one test with a
   disposition per field. The lists are taken from each type's JSON schema — not from a
   serialized default, because `skip_serializing_if` hides an empty collection and an empty
   collection is the shape a new field arrives in. Either way a new field fails, with the
   disposition question attached. Neither reaches the types those fields *hold* — a field
   added to `TempoChange`, `AutomationLane`, `TrackSend`, `ModGraph` or `NoteGraph` changes no
   pinned list — so a third mechanism closes the set: every persisted name under
   `ProjectFile`, nested or not, is registered in `lowering/persisted_fields.txt` from the live
   project schema, walked into every `properties` object *and its values* so a struct
   variant's fields inside an enum are reached, and `every_persisted_project_name_is_registered`
   fails with the added and removed names. The register carries no disposition of its own;
   its comments say where each type is read, which is where the disposition is written. Three
   independent reads shaped this: one found `Song` and `Note` under neither mechanism, the
   next found the sentence claiming closure while the nested types were still unpinned, and
   the third found the walk stopping at property keys, which missed
   `AutomationTarget::Instrument`'s fields.

   Each field is represented, refused, reported, or recorded as never reaching audio *with its
   reason*. The pin is not decoration: it is what found `Pattern::processors`, a note-processor
   rack that expands the notes a pattern plays exactly as a per-note ornament does, and which
   the per-note refusal could not see because the rack lives on the pattern.

   **Which fields reach V1's audio is measured, not read off the engine.**
   `crates/pertylizer/tests/offline_instrument_settings.rs` changes one saved field at a time
   and asserts the rendered bytes change; the dispositions cite it. A field it measures as
   audible and this lowerer says nothing about is exactly the silent difference this invariant
   forbids.

   **The rule for choosing a disposition.** Anything that changes *which notes sound* is
   **refused** — the key range, the transpose, a voice-allocation setting, a sidechain source,
   a Mod Grid graph, a Note Grid graph, a pattern's note-processor rack, a placed pattern's
   automation, a placement length override, an enabled send, a master chain. Anything that only
   scales or places the sound is **reported**: the instrument's volume and pan, the track's
   fader and pan, a placement's gain, the project master volume and glide, oversampling.

   **The song's end is V1's.** `Song::calculate_length` — the later of the last placement's
   end and the last section's — is where V1's sequencer auto-stops and releases every note it
   holds, so a note held past it is released there in the lowering too, and the render extends
   to it even when the last release comes earlier: a trailing rest, or a section drawn past the
   last placement, is silence V1 renders. A placement whose end would overflow that function's
   unchecked addition is refused by name first. An independent read of the squash found the
   authored release used and the render stopping at the last release.

   **Topology is keyed by resolved identity, not spelling.** `ModuleId` parses its instance as
   a number, so `amp-01` and `amp-1` are one module to V1; the tables that decide whether an
   amplifier's control is patched, whether a cable is a verbatim repeat, and whether a port
   receives fan-in compare parsed identities and keep the spelling only for the diagnostic. A
   spelling that does not parse is refused by name where it appears. The same read found the
   tables keyed by text, which called a respelled control unpatched.

   **An absent choice is the descriptor's declared default.** V1 creates a module from its
   descriptor and applies the saved parameters over it, so a saved map that omits `waveform`
   or a filter `type` means the descriptor's default there; the lowerer reads
   `range.default` as the index into the descriptor's `choices`, the way `gen_schemas`
   reads it, rather than naming the default in a literal that would outlive V1's. The same
   read found two literals.

   **A tempo ramp is a marked difference, not a translation.** The two `TempoChange` types
   share their fields, but V1 ramps the tempo number linearly in tick space and V2 ramps the
   beat's period (`SOUND-INV-019`), so every event after a ramp that has a later change to
   ramp toward lands elsewhere. ADR-0049 accepts that as an intentional semantic change that
   must map to a comparison category, so such a lowering is `UnsupportedScope` with a
   diagnostic naming it; a ramp with nothing after it, and a step, are exact.

   **The declared event peak is counted as admission counts it.** Admission slides a `Q`-frame
   window over the plan's edges — the worst case over every anchor phase — so the lowerer
   declares the same figure, not a count per absolute quantum, which admits a plan whose
   stream is refused after a seek. The review of the squash found the buckets and the
   forwarded ramp flag.

   **A value V1 normalizes is read through V1's own boundary rather than compared raw**, and
   through the *same function* V1's loader calls where one exists. A saved key range of
   `(127, 0)` is the full keyboard, because `KeyRange::new` swaps reversed endpoints; a saved
   oversampling of `3` is `X1`, decided by calling the loader's own decoder rather than a second
   copy of its `match`; and an instrument transpose of `0.4` moves no note, because
   `MidiNote::transpose` rounds. Each of those is neutral here exactly as it is to V1.

   **Audibility is checked where the state acts, not where the notes are — and only state V1
   acts on is refused.** V1 runs a pattern's automation whether or not that track's notes are
   audible, so automation is inspected over every placement before any note-level filtering;
   but a lane with no points emits nothing there, so only a lane holding a point is automation.
   A Mod Grid graph runs when V1's own builder makes an instance of it, which a graph with no
   routing sink or a track-scoped graph assigned to no track does not, so the lowerer asks that
   builder rather than the pool. A note-processor rack or a Note Grid binding acts where V1
   expands a pattern — on a placement that passed the instrument, mute and solo filters — so
   it is refused there and not on a pattern the arrangement never plays; a binding is resolved
   through the pool as V1 resolves it, and a dangling one is the pass-through it is in V1. An
   independent read found each of the three refusing a project V1 plays unchanged.

   **Where that stops, stated so it is a rule rather than a gap.** A stage is refused when V1
   installs it on a placement it walks and it has something to act with: a lane with a point,
   a Mod Grid instance V1's own builder returns, a rack with a processor, a bound graph with a
   node. What V1 short-circuits *before* running is neutral — a lane with no points, a
   zero-length pattern or a zero-length placement override that `pattern_tick_at` resolves
   no tick into, a graph with no nodes whose
   expansion is its seeded source, a builder that returns no instance. What an installed stage
   then **computes** is not evaluated: a rack over a pattern with no note, a Mod Grid target
   with no cable or a zero amount, a node off the graph's spine. These are refused exactly as
   a master effect at neutral settings is refused rather than measured, because that class has
   no floor — every installed stage has a setting at which it does nothing — and the contract
   would otherwise promise a neutrality analysis of V1 it does not perform. A second
   independent read asked for three of those refinements; this paragraph is the answer.

   **And the other direction is the one that matters more.** V1 evaluates two per-note things
   on every active tick regardless of the note's own start: a note-scope graph, which it seeds
   for every note, and an ornament, whose lead-in hits land before the note's onset. A
   source-independent generator or a lead-in figure on a note past the pattern's end therefore
   still sounds inside it, so both checks run before the hidden-note skip; an expression acts
   only when the note itself plays and stays after it. Two reads found one each on the wrong
   side, which was a silent difference rather than a false refusal. The rule's precedence is
   V1's own too: a resolved Note Grid binding is the arm V1 takes, so a rack under a node-less
   graph never runs and is not refused; a graph with a node is.

5. **LOWER-INV-005** — Lowering resolves a project's string and positional identities into
   stable typed identities. No file loading reaches V2, and assets arrive already prepared.
   Project save and load are unchanged: the lowerer is a consumer only.

## Types and ownership

The lowerer owns its diagnostics and its outcome; the caller owns what it does with them.
No V2 type carries a lowering concept.

```rust,ignore
pub enum Severity { Refused, Unrepresented }
pub enum ProjectSubject { /* closed: project, track, instrument, module, connection, ... */ }
pub enum LoweringReason { /* closed, and includes: */ OwnedByLaterPhase { capability: &'static str, owner: &'static str } }

pub struct LoweringDiagnostic { /* private fields; built through `refused` or `unrepresented` */ }

pub enum Fidelity { Faithful, UnsupportedScope }
impl Fidelity {
    pub fn of(diagnostics: &[LoweringDiagnostic]) -> Self;   // LOWER-INV-002: derived, not set
    pub const fn admits_parity_comparison(self) -> bool;     // LOWER-INV-003: false unless Faithful
}
```

## Lifecycle and timing

Lowering is pure and happens entirely off the audio thread. It reads an already-loaded
project value. It has two halves with different positions against compilation, and the
order is forced rather than chosen: **graph** lowering produces the `ProjectGraphIr` a plan
is compiled from, so it precedes compilation; **performance** lowering produces the events,
and an event names a note slot only a `CompiledPlan` can resolve, so it follows compilation
and reads the plan. Neither half mutates what it produced, and no step of either runs while
a render is in progress. An earlier revision of this paragraph placed all lowering before
compilation, which the event half cannot satisfy; an independent read found the claim.

## Failure and diagnostics

A refusal yields no plan and at least one `Refused` diagnostic naming the subject and
reason. An unrepresented capability yields a plan, at least one `Unrepresented` diagnostic,
and `UnsupportedScope`. A caller that only wants to know whether it may compare asks
`Fidelity::admits_parity_comparison`; a caller that wants to tell the user what happened
reads the diagnostics, each of which names its own subject.

**What the fail-closed boundary is, stated exactly.** It is the derivation in
`LOWER-INV-002` plus `LOWER-INV-003`'s prohibition, and there is no comparator in the tree
for either to gate. A lowered outcome's samples and diagnostics are readable, so a future
caller could compare them without asking for the verdict; what prevents that today is that
no such caller exists and ADR-0057 clause 5 refuses building one. The first consumer that
needs a parity verdict brings the encapsulation with it, and inherits `P04-R001` first.

## Real-time and resource constraints

`N/A` for the audio thread: lowering never runs on it, allocates freely, and produces a
value the engine admits afterwards. The plan it produces is admitted against `HostProfile`
by the Sound Core render contract's own rules, which this specification does not restate.

## Conformance tests

| Invariant | Named test or evidence |
|-----------|------------------------|
| LOWER-INV-001 | `pertylizer`'s lowering tests: refusals naming an unsupported module type, an unsupported waveform, an unresolved endpoint, an unknown port, a domain mismatch, fan-in, a second output, a missing output, a note expression, an overlap, a muted instrument, a note graph, and a persisted transpose refused **by value** before the arithmetic that would panic on it |
| LOWER-INV-002 | `Fidelity::of` takes the diagnostics and returns the verdict, so the two cannot disagree; `a_render_that_places_a_note_still_refuses_a_parity_comparison` asserts the marker on a real lowering |
| LOWER-INV-003 | `a_render_that_places_a_note_still_refuses_a_parity_comparison`: the `UnsupportedScope` verdict, the diagnostic's named owner, and a count of exactly one over a four-note song. Its scoping is checked by EVD-0013's harness, which compiles a V2 graph directly and never calls the lowerer |
| LOWER-INV-004 | `crate_boundary`, measured with `cargo tree --edges normal --invert` under default features; `synth_engine_v2` has no dependency on the lowering module. For the saved-state half: `every_persisted_song_field_has_a_disposition` pins the persisted field lists of `Song`, `Pattern`, `Note`, `PatternPlacement` and `SequencerTrack` with a disposition per field; `every_persisted_project_name_is_registered` pins every persisted name under `ProjectFile`, nested types included, against `lowering/persisted_fields.txt`; `every_audible_instrument_setting_is_dispositioned` walks the fields `offline_instrument_settings` measures as audible and asserts each is refused or reported, with a neutral instrument as its control; `song_level_state_is_refused_rather_than_ignored` covers the Mod Grid graph — routed and global or assigned refused, empty or unassigned rendering — the note-processor rack on a placed pattern against one unplaced, placed only on a muted track or zero-length, or zero-length under a length override, and the rule's boundary pinned as a rack on an audible pattern with no note, the Note Grid pool against a node-less binding, a rack shadowed by a node-less binding, a binding with a node, a dangling binding and a note-scope binding on a **hidden** note, the length override and the rounded-away transpose; `a_note_expression_is_refused_rather_than_played_as_authored`, whose second half puts a lead-in ornament on a hidden note; `the_render_is_bounded_by_the_song_end_as_v1_bounds_it`, a release past the song's end and a section past the last placement; `a_cable_spelled_with_a_leading_zero_resolves_as_v1_does`; `an_absent_choice_lowers_as_the_descriptor_declares`, which reads the declared default itself and asserts sample equality against it; `the_declared_event_peak_slides_a_window_as_admission_does`, two edges across an absolute quantum boundary; `a_tempo_ramp_toward_a_later_change_is_marked_unrepresented`, against a step and a trailing ramp as controls; `a_zero_length_override_is_as_inactive_as_a_zero_length_pattern`; `instrument_note_input_is_refused_rather_than_ignored`; `track_mixer_state_is_reported_rather_than_ignored`; `pattern_automation_is_refused_rather_than_flattened`, whose first half asserts a lane with no points renders, whose middle places the automation on a **muted** track so moving the check back behind the note filter fails, and whose last half places a pointed lane on a zero-length pattern; and `project_global_state_is_read_rather_than_ignored`. Mutation-verified in thirty-four directions: dropping either note-input refusal, the oversampling report, the sidechain refusal, the automation refusal, the Mod Grid refusal, the note-processor refusal, the pattern Note Grid refusal, the note-scope refusal, the length-override refusal, the fader report; reporting the fader per placement rather than per track; comparing the key range as a raw tuple; comparing the transpose without V1's rounding; reading the lane list's length instead of its points; reading the Mod Grid pool instead of V1's builder; scanning every pattern for a rack instead of the placements V1 plays; reading a Note Grid binding as a bare `Option` instead of through the pool; moving the note-scope check or the ornament check behind the hidden-note skip; refusing a node-less bound graph, or the rack beneath one; refusing a zero-length pattern's lanes or its rack, or its rack under a length override; stopping the register's schema walk at property keys; keying the amplifier's control check by spelling; naming a waveform default in a literal; leaving a release unclipped past the song's end; stopping the render at the last release; bucketing the event peak by absolute quantum; dropping the ramp diagnostic; and refusing a zero-length override as an override. **Both mechanisms are mutation-verified**: adding a field to `InstrumentState` produces `E0027` at the disposition site, and the persisted pin failed on first run — which is how `Pattern::processors` was found |
| LOWER-INV-005 | `no_file_loading_reaches_v2`, which scans both production trees recursively and is mutation-verified against a nested module, the repository's own `project::load_file`, and V2 reading a file itself; `ResolvedIdentities` for the identity half |

## Unresolved questions

| Question | Blocking? | ADR or task |
|----------|-----------|-------------|
| How V1's envelope sensitivity and voice-output sensitivity compose into one payload magnitude | Yes for any parity verdict over a placed note; no for lowering or rendering | Phase 6's composition law; carried as `P04-R001` |
| Where a comparison harness reads a lowered outcome, and what encapsulation it needs so `LOWER-INV-003` cannot be bypassed | Yes for the first comparison consumer | ADR-0028 and Phase 10B for the surface; ADR-0057 clause 5 refuses it meanwhile |
| Which further V1 module types the lowerer supports | No — a per-phase subset choice, refused by name meanwhile | Phase 5 and later |
| An instrument soloed **elsewhere** silences this one in V1, and a lowering's input is one instrument, so it cannot be seen from here | No for a single-instrument lowering, which is all this phase performs; yes for the first caller that lowers a whole project | The first multi-instrument consumer, which needs Phase 8's mixer model anyway |
