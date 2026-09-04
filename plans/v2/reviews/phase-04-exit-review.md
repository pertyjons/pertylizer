# REV-P04: Phase 4 Exit Review

| Field | Value |
|---|---|
| ID | REV-P04 |
| Status | Accepted |
| Phase | 4 |
| Created | 2026-09-02 |
| Last reviewed | 2026-09-04, after the thirteenth and last independent read — the review of the repaired squash onto `main` |
| Reviewed source revision | Uncommitted exit transaction based on `51a50ec5` |
| Roadmap outcome | [Phase 4 — current-project lowering and offline A/B](../ROADMAP.md#phase-4--current-project-lowering-and-offline-ab) |

## Review scope

This review covers the `LegacyProjectLowerer` and its typed boundary, the V1-to-V2 consumer
boundary, the sawtooth node kind, the bounded in-process smoke render, `SOUND-INV-021`'s key
and velocity clauses as the lowerer and the engine implement them, and the four obligations
`P04-R001` through `P04-R004` recorded against the phase.

**It is a re-run.** Its first pass rejected the phase at `ea7acd9c` with four of seven gate
bullets failing. Three of those four failed for want of a decision; ADR-0025 was then accepted
with option B, `SOUND-INV-021` was written with it, and the payload was built, which moved two
of them. The remaining two were adjudicated by the user on 2026-09-02, who chose the
named-residual route for the parity bullet and the amendment for the corpus count. This pass
therefore reviews an **amended** gate, and the amendment is itself in scope: `PROCESS.md`
permits it only under conditions, and each condition is checked below rather than asserted.

It excludes the shared render request and result, streaming, progress, cancellation,
multi-project A/B and every frontend surface — not as a scoping choice but because ADR-0028's
third standing constraint refuses them as task selections until that record is accepted, which
`P04-R004` establishes cannot happen in this phase.

## Required decisions

| ADR | Required status | Actual status | Result |
|---|---|---|---|
| ADR-0056 V1-to-V2 consumer boundary | Accepted before the first non-harness consumer | Accepted, with the user's specific approval of the manifest change and the shipping dependency edge | Pass |
| ADR-0025 tuning representation and ownership | Required by ADR-0047 before any note pitch field | **Accepted** with option B on 2026-09-02, and `SOUND-INV-021` written with it | **Pass** |
| ADR-0028 long-running job contract | Its Phase 4 deadline is withdrawn to Phase 10B by `P04-R004`, and the gate's last bullet is rewritten with it | `Deferred`, with all three standing constraints in force and the workflow analysis it required delivered | **Carried** — a named residual, conditions checked below |
| ADR-0047 note identity | Accepted, and its coupled-decision clause honoured | Accepted. The clause forbids selecting a pitch field *before* ADR-0025; the field was selected after that acceptance and takes ADR-0025's own option B — a validated key identity resolved through a prepared tuning, not a frequency. Its clause 9 still reserves the bend event, which nothing here emits | Pass |
| ADR-0004 native node representation | Accepted; a new kind is a descriptor and a kernel | Accepted; the sawtooth is exactly that | Pass |
| ADR-0057 refuse a parity verdict over a placed note | Accepted before this exit, because the amendment it records is what the exit rests on | **Accepted** on 2026-09-02, with the user's selection of option 3 and the corpus-count clause | **Pass** |

## Inventory closure

| Inventory/scope | Unclassified entries | Evidence | Result |
|---|---:|---|---|
| Saved-project eligibility for Phase 4 | 0 | `exactly_two_saved_projects_in_the_repository_lower_to_a_plan` lowers all 28 saved projects the repository holds — both the plain and bundled forms, every instrument of each — and asserts the eligible set exactly. `P04-R002` recorded the shortfall against the former count of three and is **discharged** by the amendment | Pass |
| Long-running render and analysis callers | 0 | The workflow analysis in ADR-0028, covering eleven caller groups with what each has and needs | Pass |
| V1 module types in the supported subset | 0 | Five types map; every other type is refused by name and by project object | Pass |

## Exit gates

Seven bullets, as [amended on 2026-09-02](../master-plan.md#gate-amendment-2026-09-02). The
three that changed are marked, with the before-text, so this table can be read against the
first pass without consulting it.

| Gate | Evidence or named tests | Result |
|---|---|---|
| **Amended.** Every saved project the Phase 4 subset can take lowers and renders through V2 without hand-rebuilding its patch in tests. *Before: "at least three existing saved projects"* | The set is measured, not chosen: `exactly_two_saved_projects_in_the_repository_lower_to_a_plan` lowers all 28 `.ptz` the repository holds — ten pinned corpus cases and eighteen shipped examples, both the plain and the bundled form, every instrument of each — and asserts the eligible set exactly. Both members render from their own pinned bytes: `the_pinned_corpus_project_lowers_to_events_and_renders` and `the_second_eligible_pinned_project_lowers_to_events`, each asserting `is_audible`, the second over its own tempo map and its exact twelve edges | **Pass** |
| Their note pitch and velocity reach the V2 note payload through typed values | `a_saved_notes_own_pitch_reaches_the_render` and `a_saved_notes_own_velocity_reaches_the_render`, each a **ratio** rather than an inequality: an octave renders twice the zero crossings and half the velocity renders half the peak, so a lowerer sending a constant fails them. `a_placement_transpose_moves_the_notes_it_places` applies the placement transpose and renders what authoring the transposed pitch renders. The types are `KeyIdentity` and `NoteVelocity`, validated rather than clamped | **Pass** |
| **Amended.** A lowering carries a fidelity verdict derived from its own diagnostics, and one that cannot represent what a project asked for refuses a parity comparison rather than offering one. *Before: "corpus A/B evidence exists for those projects"* | `Fidelity::of` is derived from the diagnostics rather than set, so an outcome holding an `Unrepresented` diagnostic and claiming `Faithful` is not constructible, and `admits_parity_comparison` is false for it. `a_render_that_places_a_note_still_refuses_a_parity_comparison` asserts the marker, the diagnostic's named owner, and that it is raised once over a four-note song rather than once per note. The parity **verdict** is `P04-R001`, carried | **Pass** |
| Unsupported modules and targets produce structured diagnostics naming the project object and reason | `ProjectSubject` and `LoweringReason` are closed typed enums; 92 lowering tests, including refusals naming an unsupported module type, an unsupported waveform, an unresolved endpoint, an unknown port, a domain mismatch, fan-in, a second output, a missing output, a note expression, an overlap, a muted instrument, a note graph, an instrument key range, an instrument transpose, a voice-allocation setting, a sidechain source, a placed pattern's automation, and a persisted placement transpose refused by value before the arithmetic that would panic on it. The saved state V1 applies *around* the patch is covered by an exhaustive destructure rather than a list of checks, so a new saved field is a compile error rather than a silent difference | **Pass** |
| V1 remains the default for GUI, MCP, CLI, and release rendering | ADR-0056 and `crate_boundary`: with default features `cargo tree --edges normal --invert` lists no dependent, measured rather than asserted, and a further check confirms no feature of any crate adds a second one | **Pass** |
| The lowerer contains compatibility knowledge; the V2 render plan does not | Every V1/V2 asymmetry lives in `crates/pertylizer/src/lowering/`: the resonance law, V1's clamps and defaults read from its own `ModuleDescriptor`, the unpatched-amplifier disagreement, the pan stage, fan-in, the terminal-node fallback, the transpose fallback that matches `sequencer_engine::make_pending_note`. `synth_engine_v2` has no dependency on any of it and does not know the module exists | **Pass** |
| **Amended.** The bounded in-process smoke render is the only V2 render surface this phase builds. *Before: "offline V2 rendering can stream, report progress, and cancel"* | `smoke_render` is the whole V2 render surface: no streaming sink, progress channel, cancellation, shared `RenderRequest`/`RenderResult` or multi-project A/B reaches V2, and ADR-0028's third standing constraint is what refuses the rest as task selections. V1's `ExportProgress` and `RenderReceipt` are untouched — they are the known cost constraint 2 names, and nothing was added beside them | **Pass** |

## The gate amendment, checked against `PROCESS.md`

The amendment is the reason this pass can accept what the first rejected, so reviewing it is
not optional. `PROCESS.md` permits a phase to exit with named residuals when four conditions
hold. Each is checked per residual rather than for the amendment as a whole.

Its durable record is [ADR-0057](../decisions/ADR-0057-refuse-parity-verdict-over-a-placed-note.md),
whose acceptance creates the current specification
[`spec-project-lowering-and-fidelity.md`](../specs/spec-project-lowering-and-fidelity.md) —
`Current`, prefix `LOWER`, five invariants each with a named conformance test. That
specification is checked here as part of the acceptance: without it the refusal would survive
only in an ADR and a phase gate, and `PROCESS.md` has code follow a current specification
rather than a chain of ADR clauses.
An independent read of this transaction found the amendment recorded only in the master plan's
Phase 4 section while it was an explicit product choice by the user binding work to Phases 6,
10A and 10B — `PROCESS.md`'s durable-decision threshold, and the same shape as ADR-0055, which
is the refusal Phase 3's own residual exit rested on. The record was written and accepted
before this pass concluded, and this table checks the conditions its clause 3 invokes.

| Condition | `P04-R001` — the velocity composition | `P04-R004` — the job contract |
|---|---|---|
| The current gate is rewritten not to claim the deferred behaviour | Bullet 3 now asks for a refused comparison, not a parity verdict; the before-text is recorded beside it | Bullet 7 now asks that no V2 streaming, progress or cancellation surface be built, which is the constraint already in force |
| The implemented behaviour fails closed rather than accepting and silently ignoring the unsupported case | Every lowering that places a note is `UnsupportedScope` and `admits_parity_comparison` is false; the marker is derived from the diagnostics, so the two cannot disagree. The control is that derivation plus the absence of any comparator, **not** an encapsulation: a lowered outcome's fields are public, and ADR-0057 clause 5 is what refuses building the first caller. `LOWER-INV-003` carries the rule forward | No partial V2 job surface exists to mislead a caller: constraint 3 refuses them as task selections and none reaches V2. V1's existing blocking renders are unchanged, which is the cost this record already accepted |
| Each obligation has a named owner and blocks its first real consumer | Phase 6, with the composition law, named in the diagnostic itself. First consumer: the first parity verdict over a **lowered** outcome that places a note. A harness that never lowers, as EVD-0013's does not, is outside the rule | Phase 10A for the canonical revision, Phase 10B for the job service. First consumer: the first shared render request, A/B batch or frontend render surface |
| No real-time safety, persisted-data, protocol or correctness guarantee a dependent phase needs is weakened | Phase 5 depends on the lowerer, its typed boundary and its diagnostics; the deferred item is an amplitude law inside an offline render whose comparison is refused | Phase 5 depends on this phase's lowerer, not on render orchestration; no persisted or protocol surface is involved |

**The cycle the master plan forbids does not close.** Naming Phase 6 the owner of a *residual*
is not naming it a prerequisite: Phase 4 exits without it, and `PROCESS.md` binds the
obligation forward to the phase that expands into it. The dependency still runs 4 → 5 → 6.

**The named outcome is amended too, and this pass says so rather than reading past it.**
`PROCESS.md` has the exit review check the phase's *named outcomes*, and the roadmap named
rendering "through the same headless comparison path as V1". That join is **not** delivered:
the work-list row below records the development-only offline engine selection as carried, and
this phase builds no shared V2 render surface. An independent read of this transaction caught
the review claiming the outcome was not weakened while that sentence still stood, which was the
one place the deferred behaviour would have kept being claimed. The roadmap's outcome sentence
is therefore amended with the bullets, and this acceptance is against the amended one.

**Why that is bounding rather than deletion.** The first pass refused this route because a
rewrite would then "delete the phase's own outcome rather than bound it — a lowering that
renders nothing faithfully is not 'current projects rendered through the same headless
comparison path as V1'". Its first half no longer holds: saved projects lower **and render**
through V2, at their own pitches and velocities, from their own pinned bytes, and the first two
bullets assert exactly that. What leaves the outcome is the join between the two render paths,
whose two halves are `P04-R001` and `P04-R004`, each with an owner and a first consumer. The
phase exits with a smaller outcome than it was given, which is what a residual exit is.

**A hole class this pass found, and closed with a mechanism rather than a patch.** Successive
independent reads of this exit transaction each found another saved field the lowerer never
read: the track's fader and pan, which V1 mixes through as `auto.volume.unwrap_or(track.volume)`;
the instrument's key range and transpose, which `Instrument::note_on_expr` uses to suppress
notes outside the range and to move — and sometimes drop — every note; then its oversampling
and its voice-allocation settings; then a placed pattern's automation lanes. Each was a project
V1 and V2 rendered differently with **no diagnostic**, which is the class `P04-R002`'s
measurement found in project-global state.

Patching them one at a time was not the fix, and the review history is the evidence: four
successive passes each found the next one. The lowerer now carries **two mechanisms**, one per
crate. `InstrumentState`, `SequencerTrack` and `GlobalProjectState` are destructured
**exhaustively, without `..`**, so a new saved field is a compile error at the disposition site,
and `TrackMode` is matched exhaustively for the same reason. `Song` and `Pattern` live in
`synth_sequencer` behind accessors and cannot be destructured from here, so their **persisted**
field lists are pinned from each type's JSON schema, and `Note` and `PatternPlacement` — which
could be destructured — are pinned beside them so the four lists sit in one test; a new field
fails that test with the disposition question attached. Every field carries a written
disposition, and which fields reach V1's audio is taken from
`tests/offline_instrument_settings.rs`, which measures it one field per test, rather than from a
reading of the engine. The set took two more reads to close. The eighth found the specification
promising a mechanism for every saved field while `Song` and `Note` sat under neither and
`GlobalProjectState` was read field by field; all three went under a mechanism and the
specification's two open rows recording the gap went with them. The ninth found the sentence
then claiming closure while every *nested* persisted type — `TempoChange`, `AutomationLane`,
`TrackSend`, `ModGraph`, `NoteGraph` — was still pinned by nothing, since a field added to one of
them changes no list above it. A third mechanism answers that: every persisted name under
`ProjectFile`, walked from the live project schema, is registered in
`lowering/persisted_fields.txt`, and `every_persisted_project_name_is_registered` fails with the
added and removed names. The register carries no disposition of its own; its comments point at
the site that reads each type, and the disposition is written there.

**The pin found a hole on its first run**, which is the clearest evidence it is a mechanism
rather than a formality: `Pattern::processors` is a note-processor rack that expands the notes a
pattern plays exactly as a per-note ornament does, and the existing per-note refusal could never
have seen it, because the rack lives on the pattern.

What that produced beyond the mechanisms: the key range, the transpose, the allocator settings,
a sidechain source, a Mod Grid graph, a pattern's note-processor rack, a placed pattern's
automation and a placement length override are **refused**, because each changes which notes
sound; the track fader and pan and the instrument's oversampling are **reported**.

Three normalizations came with it, each read through V1's own boundary rather than compared raw:
a saved key range of `(127, 0)` is the full keyboard because `KeyRange::new` swaps reversed
endpoints; a saved oversampling of `3` is `X1`, decided by **calling V1's loader's own decoder**
rather than a second copy of its `match`; and an instrument transpose of `0.4` moves no note,
because `MidiNote::transpose` rounds. Refusing any of the three would have rejected a project V1
plays exactly as a neutral one.

Two placement questions moved with them. A length override was *reported* until a review pointed
out that it clips a pattern's later onsets or repeats it — a change to the note set, which the
phase's own rule refuses. And the automation check sat **after** the note-track filter, where a
muted track's automation never reached it, although V1 runs automation whether or not that
track's notes are audible and a lane can target another track, an instrument or a global
control; it is now a song-level pass, with a muted-track case that fails if it moves back.

**The eighth read found the opposite defect, three times: refusing state V1 does not act on.**
Each was a project V1 plays unchanged that the lowerer rejected, which is a false refusal rather
than a silent difference, and each is now decided the way V1 decides it. An automation lane with
no points emits nothing — `AutomationLane::value_at` is `None` for it — so only a lane holding
a point is automation. A Mod Grid graph runs only when V1's own builder makes an instance of it:
a graph with no routing sink or a track-scoped graph assigned to no track builds none, so the
lowerer asks `build_mod_grid_runtime` rather than whether the pool is empty — the same reason the
oversampling decoder is shared, that a copy of the rules compiles happily after the original
changes. And a note-processor rack acts where V1 expands a pattern, which is a placement that
passed the instrument, mute and solo filters, so the rack check moved from a song-wide scan into
that walk, and the Note Grid pool went with it: a pooled graph nothing binds is inert, a pattern
binding is resolved through the pool as V1 resolves it, a dangling binding is the pass-through it
is in V1, and a note-scope binding is refused through its note. Each direction has a case that
renders and a case that refuses, and seven further mutations — the three earlier checks restored,
each new refusal dropped, and a binding read as a bare `Option` — are caught.

The same read found two claims in the specification that the code did not bear out. Its
lifecycle section said all lowering precedes compilation, while performance lowering needs the
`CompiledPlan` to resolve a note slot and so follows it; the section now states the two halves
and why the order is forced. And it promised a mechanism for every saved field while `Song` and
`Note` were pinned by nothing, which the paragraph above records as closed.

**The ninth read, a focused reread of those repairs, found one silent difference and drew the
rule's boundary.** The silent difference is the one that matters: V1 seeds every note's
note-scope graph on every active tick regardless of the note's own start, so a
source-independent generator bound to a note past the pattern's end still emits — and the
note-scope check sat *after* the hidden-note skip, where that note never reached it. It now runs
before the skip, with a hidden note bound to a graph holding a node as the case. Two more of its
findings were V1 short-circuits the lowerer can read as cheaply as the ones already read: a bound
Note Grid graph with no nodes expands to its seeded source, and a zero-length pattern resolves no
tick in `pattern_tick_at`, so neither its lanes nor its rack are ever run. Both render now. The
node check is structural rather than derived on purpose: a graph's spine and processing order are
recomputed after load, and a freshly deserialized graph does not carry them, so reading them
would call a real graph empty.

Its remaining two findings asked for more of the same — a Mod Grid target with no cable or a
zero amount, a rack on an audible pattern that holds no note — and the answer is a rule rather
than two more checks, because that class has no floor: every installed stage has a setting at
which it computes nothing, and a master effect at neutral settings is already refused rather
than measured. The specification now states where refusal stops: a stage V1 installs on a
placement it walks, with something to act with, is refused; what V1 short-circuits before running
is neutral; what an installed stage then computes is not evaluated. The rack-on-an-empty-pattern
case is pinned as refused so the boundary is a decision the tests hold rather than an oversight
the next read finds.

**The tenth read, focused on those repairs, found the same silent-difference shape once more
and two contradictions with the rule as stated.** An ornament's lead-in hits land before the
note's own onset, so a note at the pattern's end with a lead-in figure sounds inside the pattern
exactly as a note-scope generator does — and the ornament check sat after the hidden-note skip
too. It now precedes the skip, the expression check stays after it because an expression acts
only when the note itself plays, and the refusal test — which had exercised only an expression —
puts a lead-in ornament on a hidden note. The register's schema walk stopped at property keys,
so a struct variant's fields inside an enum, `AutomationTarget::Instrument`'s among them, were
never collected; it walks property values now, and a mutation restoring the old walk fails. The
two contradictions were with the rule's own precedence: a rack was refused beneath a node-less
bound graph although the resolved graph is the arm V1 takes and the rack never runs, and a
zero-length pattern was refused for a length override before the zero-length skip could say it
never plays. Both are ordered as V1 orders them and have a rendering case. A fifth finding was a
false sentence in both documents — that `Note` and `PatternPlacement` cannot be destructured;
they could be, and are pinned beside the two that cannot so the four lists sit in one test.

**The twelfth read was the independent review of the squash onto `main`, over the whole
branch rather than the exit transaction, and it found two things the phase's own reads could
not see.** Both were evidence harnesses. When the note payload gained a key — commit
`05edb36a` on this branch — the EVD-0012 and EVD-0014 harnesses were given a note-on at key 60,
which now *retunes* the 440 Hz oscillator each fixture prepares to C4 while V1's arm still plays
A4. EVD-0012's checked-in digests stopped reproducing (`voice-mono` rendered `a4e2aa…` against
the pinned `0fe495…`; `gain-chain`, which plays no note, still matched), and EVD-0014's V2
whole-render arm could no longer bit-match EVD-0013. Both harnesses now send A4, key 69, and
both were **run** rather than rebuilt: EVD-0012's three digests equal the pinned values again,
and EVD-0014's C3 reports `whole-render-v2,true` against `v2-aligned.wav`. The V1 arm of C3 was
not re-rendered here; nothing on that side changed. The lesson is the one an earlier memory
already held — the gate compiles the harnesses and never runs them — and the phase-scoped reads
never opened them because the exit transaction did not touch them.

The same read found three asymmetries in the lowerer itself, each now read through V1's own
boundary: the song's end is `Song::calculate_length`, so a note held past it is released where
V1's auto-stop releases it and the render extends to it through a trailing rest or a section
drawn past the last placement — which also corrects the `sections` disposition, recorded as
inert until this read; the topology tables are keyed by parsed `ModuleId` rather than by
spelling, so a cable to `amp-01` reaches `amp-1` as it does in V1; and an absent `waveform` or
filter `type` is the descriptor's declared default, read as `gen_schemas` reads it, rather than
a literal. Each has a case, and four mutations restoring the old behaviour are caught.

**The thirteenth read reviewed the repaired squash, and is the last: the user closed the loop
by budget.** Its four findings were all repairs under the stopping rule rather than
refinements, and were made without a further independent read, at the user's direction. The
declared event peak was bucketed by absolute quantum while admission slides a `Q`-frame window
— the worst case over every anchor phase — so a plan could be admitted on its declaration and
have its stream refused after a seek; the lowerer now counts as admission counts, with two
edges across an absolute boundary as the case. A saved tempo ramp toward a later change was
forwarded as a flag although V1 ramps the tempo number and V2 the beat's period, which
ADR-0049 accepts as a semantic change that must map to a comparison category; such a lowering
is marked unrepresented now, with a step and a trailing ramp as the controls. A placement
whose `length_override` is zero was refused as an override although `pattern_tick_at` resolves
no tick in it, which is the same inactivity as a zero-length pattern; both conditions are read
now, since V1 has both. And ADR-0025 said a plan "declaring more than one voice scope" is
refused, which a single scope kind cannot express and which neither the sound contract nor the
implementation states — it is one execution scope holding two playable nodes, and the ADR now
says so, with the correction dated. Three mutations restoring the old behaviour are caught.

One difference that was silent is now named, and named no more strongly than the evidence
supports: V1 allocates a voice per note, so a release can ring under the next one, while V2
retriggers its single gate and cuts it. Whether a release *actually* rings depends on the
envelope against the gap, which this lowerer does not compute — so the diagnostic names the
single-gate shape rather than asserting a ringing release, after a review caught the stronger
wording.

**Thirty-four mutations were run and caught** — twelve before the eighth read, seven after it,
four after the ninth, four after the tenth, four after the twelfth and three after the
thirteenth — plus both mechanisms: adding a field to `InstrumentState` produces `E0027` at the disposition
site, and the persisted pin failed on its first run. One mutation found a defect in this pass's *own* test — reporting the fader per
placement rather than per track passed, because the fixture held a single placement; it now
holds two.

The eligible set is unchanged, and measured rather than assumed: both pinned cases carry V1's
defaults in every field the new dispositions read, and
`exactly_two_saved_projects_in_the_repository_lower_to_a_plan` still asserts the same set.

**A correction this review made to itself, and how it resolved.** An earlier draft recorded
that `CORPUS-0001` *rendered* audibly and treated `Fidelity::UnsupportedScope` as sufficient,
because a render that cannot be compared for parity cannot mislead. An independent read showed
that is not what the contract says: the work list's "before rendering the first saved pitched
note, close P03-R003" and `SOUND-INV-017`'s restatement put the precondition on the **render**,
while the fidelity marker governs **reporting**. Every saved note carries a pitch, so while the
payload was empty no saved note could be rendered at all, and the implementation was changed
rather than the review — `smoke_render` refused, naming `P04-R001`.

That refusal has since been **discharged rather than relaxed**. The precondition was closing
`P03-R003` with minimum typed pitch and velocity payload semantics; those semantics were built
after ADR-0025 was accepted, so the precondition is met and the render proceeds. The division
the correction established still stands and is what the amended third bullet rests on: the
precondition governed the render, the fidelity marker governs reporting, and it is reporting
where the remaining gap is. A note-free arrangement still renders as its own case, which keeps
the render path checked independently of any note.

## Work-list closure

Separate from the exit gates above, which are the seven the roadmap adjudicates. These are
work-list items whose completion is checkable but which are not themselves gate bullets.

| Work-list item | Evidence | Result |
|---|---|---|
| No file loading reaches V2; assets arrive already prepared | `no_file_loading_reaches_v2` scans both production trees recursively — the lowering module and `synth_engine_v2/src` — and is mutation-verified against a nested module, the repository's own `project::load_file`, and V2 reading a file itself. The asset half is vacuous today and the test says so: V2 has no sampler, so there is nothing to prepare until ADR-0026's zone model | **Done** |
| Project save and load are unchanged; V2 is a consumer only | This branch touches neither `project.rs`, `patch.rs` nor `synth_sequencer`, and `corpus_manifest` still pins every fixture by digest | **Done** |
| Resolve current string and positional identities during lowering | `ResolvedIdentities`, with the address computed from the identity alone so an unrelated insertion moves nothing | **Done** |
| A development-only offline engine selection in the render and analysis harnesses | **Not done**, and it is one of the two residuals' first consumers rather than an omission: ADR-0028's third standing constraint refuses it as a task selection, and a selection that let a harness report a lowered project's V2 render as V1's would be the parity verdict `P04-R001` refuses. EVD-0013's existing harness is unaffected — it builds its own fixtures and never lowers | **Carried** — Phase 10B for the surface, Phase 6 for the verdict it would report |

## Quality gates

| Command/check | Environment | Result | Evidence |
|---|---|---|---|
| `python3 -B scripts/check_v2_docs.py --evidence` | Local | Pass | Run on every commit in the range, and again over this exit transaction |
| `python3 -B -m unittest scripts/test_check_v2_docs.py` | Local | Pass | 35 tests |
| `cargo fmt --check` | Local | Pass | — |
| `cargo build --workspace` | Local | Pass | — |
| `cargo clippy --workspace --all-targets` | Local | Pass | No warning; warnings are denied by configuration |
| `cargo clippy -p pertylizer --all-targets --features v2-lowering` | Local | Pass | The non-default feature is not covered by the workspace run |
| `cargo test --workspace` | Local | Pass | 3 276 tests, including the 92 lowering tests, which run under default features by design |
| `cargo test -p synth_engine --release resource_limit_probe_…` | Local | Pass | — |
| `cargo doc --workspace --no-deps` | Local | Pass | No warning |
| `cargo check --workspace --all-targets --no-default-features` | Local | Pass | — |
| `cargo check --workspace --all-targets --all-features` | Local | Pass | Compiles the lowering path |
| `cargo +1.98.0 check --workspace` | Local | Pass | MSRV |

Every command in this table was re-run over the exit transaction this pass reviews, not carried
forward from the first pass.

## Deviations and residual risks

| Item | Impact | Owner/task | Acceptance basis |
|---|---|---|---|
| `P04-R001`: V1 applies one saved velocity twice, under two independent sensitivities, and V2 applies it once as one scale on the envelope | No parity verdict may be issued over a placed note. At V1's defaults both response factors reduce to `v`, so for the corpus's saved `0.756` the two renders stand at `0.572` against `0.756` — 2.4 dB at the sustain, against the `-0.0027` dB EVD-0013 measured there for the envelope difference ADR-0042 accepted. `CORPUS-0001-P2` is the declared claim this residual actually holds; the pitch and onset claims `CORPUS-0001-P1` and `CORPUS-0009-P2` are representable and simply unjudged | Phase 6, with the composition law, which `SOUND-INV-021` and ADR-0025 already assign it | **Accepted as a named residual.** Its four conditions are checked in the table above. The payload half of this obligation — typed pitch and velocity reaching the render — is **discharged**, and `P03-R003` with it |
| `P04-R002`: the repository supplies two eligible saved projects, not three | None remaining | — | **Discharged.** The user amended the count on 2026-09-02 rather than authoring a project to satisfy it. The bullet now asks for the measured set, so the count is evidence rather than a target, and the test asserts that set exactly in both directions |
| `P04-R003`: no V2 node rendered a sawtooth | Blocked every corpus lowering, because nine of ten cases author one | Phase 4 | **Discharged.** V2 has a band-limited sawtooth, checked at the bins its aliases fold into, and `CORPUS-0001` lowers through it |
| `P04-R004`: ADR-0028 cannot be accepted here | No streaming, progress, cancellation, shared render request/result or multi-project A/B surface exists, and none may be built | Phase 10A for the canonical revision; Phase 10B for the job service and the frontend surfaces | **Accepted as a named residual.** Its four conditions are checked in the table above, and ADR-0028's third standing constraint is what keeps the refused work from being built under another name |

**Two residuals, and both were blocking obligations in the first pass.** What changed is not the
engineering but the gate: `PROCESS.md`'s residual route was unavailable while the deferred item
was the phase's own outcome, and it became available once the outcome landed and what remained
was a composition law a later phase already owned. The first pass said as much — "what would
change this outcome" named `SOUND-INV-021` implemented, then an amendment to the corpus count,
then either ADR-0028 accepted or an explicit rewrite of the job-contract requirement. All three
happened, in that order.

## What this phase did deliver

- Both corpus cases `P04-R002` measures as eligible lower from the bytes
  `corpus/v2-reference/manifest.json` pins by digest, compile, schedule their own notes through
  their own tempo maps, and **render** — at their own pitches and their own velocities, with a
  placement's transpose applied. A note-free arrangement renders end to end as its own case,
  which keeps that path checked independently of any note.
- `SOUND-INV-021`'s key and velocity clauses are built end to end: a validated key identity in
  `0..=127` refused rather than clamped, one normalized velocity, a `PreparedTuning` the plan
  holds once and each pitch-producing node references, destinations bound by execution scope so
  a note's magnitudes reach nodes it did not name, and admission charging the expansion. Its
  bend clause is not built, is not reached by anything in this phase, and says so in its own
  conformance row rather than being discovered later.
- The V1-to-V2 consumer boundary is decided, approved and executable, and the Phase 1
  deletability claim survives it for every build that ships.
- Every V1/V2 asymmetry the supported subset can reach is either represented or refused with a
  diagnostic naming the project object — and the defaults and clamps are read from V1's own
  descriptors rather than transcribed, so a rule V1 declares cannot go missing.
- V2 owns a band-limited sawtooth whose band-limiting is measured at the bins its aliases fold
  into, not by likeness to V1. It is what makes the corpus lowerable at all: nine of the ten
  pinned cases author one.
- The workflow analysis ADR-0028 named as its missing evidence is done, covering eleven caller
  groups, and it is what established that the record cannot be accepted here.
- ADR-0025 is decided. It was the blocker with no record; it was drafted here with three options
  and their falsifiers, the user selected **B** on 2026-09-02, and `SOUND-INV-021` was written
  with the acceptance so the specification governs implementation rather than the decision
  chain.
- Eligibility is measured rather than asserted: every saved project in the repository is lowered
  by a test that names the eligible set exactly — a measurement that found the lowerer blind to
  project-global state while establishing the count, and fixed it.

## Outcome

Outcome: **Accepted.**

Phase 4 exits. All seven gate bullets pass against the gate as amended on 2026-09-02, and the
amendment itself satisfies `PROCESS.md`'s four conditions for each of the two obligations it
defers.

**What this acceptance does not claim.** No parity verdict exists for a placed note, and this
review does not treat one as delivered: the third bullet asks for a refusal and gets one. No
streaming, progress or cancellation surface exists either. Both are carried as named residuals
with owners, and both block their first real consumer rather than being scheduled.

**What a later phase inherits.** Phase 6 inherits `P04-R001` before it builds the composition
law it already owns; the first parity verdict over a placed note is its first consumer, and no
harness may report a V2 render beside V1's until then. Phase 10A and Phase 10B inherit
`P04-R004`; ADR-0028's three standing constraints hold until that acceptance, and constraint 3
is what keeps the refused surfaces from being built under another name.

**What would reopen this gate.** A later finding that invalidates relied-on evidence or a
safety or correctness guarantee a dependent phase needs — specifically: a saved project that
becomes eligible and does not lower and render, since the first bullet now asks for the measured
set rather than a count; or a lowering that places a note and is not marked `UnsupportedScope`,
since that is the fail-closed condition both the third bullet and `P04-R001` rest on.
