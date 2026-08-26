# ADR-0047: Note Identity in the Event Contract

| Field | Value |
|---|---|
| ID | ADR-0047 |
| Status | Accepted |
| Phase | 3 |
| Created | 2026-08-25 |
| Last reviewed | 2026-08-26 |
| Related | [ADR-0046](ADR-0046-destination-quantum-admission.md) clauses 3, 5 and 6; [ADR-0032](ADR-0032-sample-time-and-event-timestamps.md) clauses 17 and 20; [ADR-0021](ADR-0021-host-profile-and-admission-policy.md) part 3; ADR-0025 (`Proposed`, tuning); [REV-P02](../reviews/phase-02-exit-review.md)'s `NoteEdge` deviation row; `HOST-INV-009`; `HOST-INV-013`; `HOST-INV-018`; `HOST-INV-021`; `SOUND-INV-016` |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

This record crosses three of `PROCESS.md`'s durable boundaries. It defines the **public event vocabulary** every
renderer producer writes to; it fixes a **real-time ownership boundary**, namely who may name a sounding note and
where that name is resolved without a search on the audio thread; and it **binds several later phases**, because
Phase 6's voice allocator and Phase 7's per-note modulation both address notes through whatever this record selects.
Reversing it later changes an accepted producer contract and every admitted producer's declaration.

**Why it is ready now.** [ADR-0046](ADR-0046-destination-quantum-admission.md) clause 3 is `Accepted` and contains a
sentence that cannot be implemented under the current event vocabulary:

> A live note-on acquires both its event slot and a release hold atomically. If it cannot, that note-on is the event
> dropped at the live boundary; **a later edge for it is an orphan and is counted rather than allowed to release
> another note.**

Today a note event is `{ slot: NoteSlot, edge: NoteEdge }` (`crates/synth_engine_v2/src/render.rs:227-232`). The two
values a producer can send are "play this compiled node" and "let go of this compiled node". Nothing in that pair
distinguishes the orphan from the legitimate release, so the emphasised clause has no implementable meaning. The
publication arbiter is the immediately next dependent slice and is the first consumer; it cannot be built safely
without the answer.

**The coupled decision that is deliberately left open.** ADR-0025 (tuning representation and ownership) is `Proposed`
and targets Phase 6. A note *pitch* field cannot be selected before it without committing the public event contract to
a pitch representation that ADR-0025 exists to choose. Under `PROCESS.md`'s coupling rule this record therefore
decides identity, which is not coupled to tuning, and decides pitch not at all.

**This record fixes relations, not widths.** It takes the same posture ADR-0046 takes toward share values: the
quantities below are selected by Phase 3 measurement and checked by construction, and this record states the
relations that make any selected value safe. An earlier draft instead asserted concrete widths from a reuse-rate
estimate; the independent review falsified that estimate's arithmetic and its premise, and the repair was to make the
property checkable rather than to restate it with different numbers.

## Decision boundary

### What this record decides

1. Every note event, on both edges, carries a **note identity** distinct from the node it plays.
2. That the identity, not the node address, is the sole authority for which occurrence an event names.
3. Which party mints an identity, and why two producers cannot mint the same one.
4. How an identity that names no live note is recognised, so ADR-0046 clause 3's orphan sentence becomes executable.
5. That the index space is bounded by a construction-checked relation, that a generation value is never reused
   within one identity table, and what happens when a finite identity space runs out.
6. How a release resolves to its note without a search on the audio thread.
7. That per-note expression — polyphonic pressure, per-note bend, release velocity, MPE and MIDI 2.0 — addresses a
   note through this identity. This is the reservation the master plan's Phase 3 work list asks for.

### What this record does not decide

- **Note pitch.** Owner: **Phase 3**, as REV-P02 assigned it, and this record does not move it. What this record adds
  is that the limb is coupled to ADR-0025, which is `Proposed`. Nothing in Phase 3 *reads* a pitch — a Phase 3 note
  event plays a compiled node whose sounding pitch is an ordinary control, which is the reading `SOUND-INV-016`
  already fixes and which Phase 2 verified — so the coupling is a question about when the decision can be taken, not
  about whether Phase 3 needs the value.
- **Velocity and release-velocity magnitudes.** Owner: **Phase 3**, likewise unmoved. This limb carries no ADR-0025
  coupling; it is an attribute of a note this record can already name, so adding it is additive to a payload whose
  identity is settled.
- **The per-note expression parameter vocabulary.** Owner: Phase 7, ADR-0007.
- **Voice identity and voice allocation.** Owner: Phase 6. ADR-0046 already states that it "requires a bounded
  mass-release event; Phase 6 decides how the allocator applies it". A note may sound as several voices; the mapping
  from one identity to the voices it owns is the allocator's and never appears in the event contract.
- **The numeric index and generation widths.** Owner: Phase 3 measurement. The index width is bounded below by
  clause 5's checked relation; the generation width decides only how long a stream runs before indices retire.
- **What happens to an outstanding obligation when its identity table is rebuilt.** Owner: **ADR-0048**, Phase 9,
  beside ADR-0009. Clause 8 refuses the rebuild in the meantime, which is sufficient for Phase 3 and is stated there
  as an interim limit rather than as the answer.
- **Persisted or wire representation.** A note identity is runtime-only, for the same reason `HOST-INV-010` keeps
  `HostProfile` out of a project document: it names an occurrence inside one plan activation, not anything a document
  holds.

### What this record does not discharge

REV-P02's deviation row is "**`NoteEdge` carries neither pitch nor velocity**". This record decides neither, so it
does not discharge that row, in whole or in part. It adds a third thing the row does not name — identity — because
ADR-0046 clause 3 requires it.

**The row keeps the owner and the deadline REV-P02 gave it.** That review assigns it to Phase 3 and says the decision
is "owed before ingress". This record does not move it to another phase and does not extend that deadline; doing
either would rewrite an accepted review's disposition, which is not this record's to do. The row is the **next**
decision after this one, and it is separate work.

What this record does establish about that next decision is one obstacle, recorded rather than resolved: its pitch
limb is coupled to ADR-0025, which is `Proposed` and targets Phase 6, so Phase 3 cannot decide pitch without either
accepting ADR-0025 early or recording why the limb cannot meet REV-P02's deadline. Its velocity limb carries no such
coupling. `NOW.md` gains that obstacle; today `NOW.md` names Phase 3's ingress as the row's first consumer and says
nothing about ADR-0025.

## Evidence

- `crates/synth_engine_v2/src/render.rs:168-179` defines `NoteEdge` as a two-valued edge, and the doc comment
  immediately above it names this record's subject: "It carries no pitch, velocity or note identity ... Phase 3's
  ingress and Phase 6's voice pool are where they arrive."
- `crates/synth_engine_v2/src/render.rs:227-232` is the payload the identity must join. `NoteSlot`
  (`crates/synth_engine_v2/src/plan.rs:205-208`) addresses a **compiled node**, resolved off the audio thread by
  `CompiledPlan::resolve_note` (`plan.rs:757-762`). It is an address, not an occurrence: two simultaneous notes on one
  node share it, so it cannot serve as the identity.
- `crates/synth_engine_v2/src/render/hot.rs:98-105` already rejects and counts an event whose slot belongs to another
  plan, deliberately before the per-quantum tally, and its comment states why: "after a plan swap, in-flight events
  are *ordinary*". A plan-scoped rejection path therefore exists and is the one clause 8 extends, rather than a new
  mechanism this record invents.
- `crates/synth_engine_v2/src/render/hot.rs:86-91` implements ADR-0032 clause 20's epoch rejection. Epoch and plan
  identity are **two** filters in that source, not one; clause 8 depends on the distinction.
- `crates/synth_engine_v2/src/quantities.rs:188-192` already separates the two counts this record depends on:
  `HeldNoteCount` is "a source obligation" and `VoiceCount` "a resource", "deliberately not convertible", because
  "more notes can be held than sounded. A sustain pedal, a stealing allocator, and an MPE source all do it." The
  identity belongs to the obligation, not the resource.
- `crates/synth_engine_v2/src/profile.rs:344-376` shows that `max_held_notes` is today constrained only to be
  nonzero. Clause 5's index relation is therefore a **new** checked relation, not a restatement of an existing one.

**Uncertainty that could change the decision.** Clause 5's non-wrapping rule removes the aliasing question but moves
the residual risk to retirement: an index that exhausts its generation space is withdrawn, so a long stream slowly
loses index space. The falsifier is a measured retirement rate at which a realistic session exhausts a producer's
whole range, which would make the selected width a liveness defect even though it cannot be a safety one. Phase 3
measures that rate; nothing in this record asserts it is small.

## Options

### Status quo: `{ slot, edge }` with no identity

Rejected, and not merely incomplete. A producer whose note-on was dropped at the live boundary sends a note-off that
is indistinguishable from a legitimate one, so it releases whichever note the allocator believes is sounding on that
node. ADR-0046 clause 3 forbids exactly this. The status quo also cannot express two simultaneous notes on one node,
which is what Phase 6 exists to add.

### Match by `(node, pitch)`, as MIDI 1.0 does

Rejected for two independent reasons. It requires a pitch field, which ADR-0025 owns and this record must not
pre-empt. Independently, it cannot represent two simultaneous notes at the same pitch on one node — which MPE
produces routinely, which a layered or unison patch produces, and which a repeated note under a sustain pedal
produces. A contract that cannot name the second of two identical-pitch notes cannot release the right one.

### A free-running per-producer counter

Viable, and it remains the fallback if clause 5's retirement cost proves unacceptable in measurement. It is not
free of the finiteness problem — a fixed-size counter must eventually refuse further identities, exactly as the
selected option must eventually retire indices — so it trades a different failure mode, not an absence of one. Its
cost is resolution: nothing relates the counter to a storage position, so redeeming a release means scanning the
held-note table. That scan is bounded by `max_held_notes` and therefore real-time legal, but it puts a linear pass on
the audio thread at the same boundary ADR-0046 already loads with the arbiter's serial publication pass, whose cost
Phase 3 must separately measure.

### Selected: a table-scoped generational handle

The identity is the position of the note's obligation plus a counter that changes whenever that position is reused.
It resolves in one indexing step, and a stale identity is detected by comparison rather than by absence.

## Decision

**Select the table-scoped generational handle.** The following clauses are one contract.

### 1. Both note edges carry an identity, and only the on edge carries a node

A note-on names two things: a `NoteSlot`, which is *what is played* and is an address into the compiled plan, and an
identity, which is *which occurrence*. A release and a per-note expression event carry the identity **alone**.

The node address is deliberately absent from the release rather than carried and required to agree. Carrying both
admits an event whose identity names occurrence A while its slot names node B, and no reading of that event is safe:
honouring the slot releases the wrong note, and honouring the identity silently redefines what the node field means.
Removing the field removes the case instead of adjudicating it. `SOUND-INV-016`'s rule that a note event names a node
rather than one of its controls is preserved where it does work — the on edge, where the node is what is played.

### 2. The identity is a table, an index and a generation

The identity is a fixed-size `Copy` value of three private fields: the identity of the **table** that minted it, an
**index** into the minting producer's range within that table, and a **generation** that advances each time the index
is reused. No field is separately meaningful and none is exposed as a raw primitive. The index width is selected
under clause 5; the generation's is a liveness choice under the same clause.

The table identity is carried for the reason `NoteSlot` carries a `PlanId` (`plan.rs:205-208`) — clause 1 removes the
node address from the release, so without some provenance a release would arrive with none and clause 8's filter
would have nothing to compare — but it is a **new** value rather than a reuse of `PlanId`, because clause 8 shows
that neither the plan nor the epoch changes on every table rebuild.

### 3. Producers mint from disjoint ranges assigned at admission

Each admitted note-on producer receives an identity range at plan admission, disjoint from every other producer's,
and mints only from its own. Disjointness is by construction, so an identity is attributable to one producer without
carrying a producer tag.

**This partition is not ADR-0046's hold partition, and the two are not two views of one thing.** ADR-0046 clause 6
gives a hold only to a non-compiled note-on "whose complete note-on/release pair is not already present in one
indivisible materialized open-window batch"; a compiled note-on takes no hold at all, and neither does an authored
note-on whose release is already in the same sealed batch. Every note-on nevertheless needs an identity, because
every release must name its occurrence. The identity partition therefore covers a **superset** of the producers the
hold partition covers, and it is sized by the held-note capacity rather than by `release_hold_capacity`.

Where ADR-0046 clause 3 does require a hold, minting the identity, acquiring the hold and charging the event slot are
one indivisible step, and a note-on that cannot complete all three is the note-on dropped at the live boundary. Where
no hold is required, minting is atomic with charging the event slot alone. The hold is a resource this contract does
not resize; the identity is a name the hold contract needs.

### 4. A stale identity is an orphan, is counted, and releases nothing

**An identity that names no live note is an orphan**, and that is the definition — the reachable cases follow from
it rather than constituting it. There are three, and clause 5's retirement is why the third has to be named: an
identity whose index is **free**, one whose generation **differs** from the generation held at a live index, and one
whose index has been **retired** and will never hold a live note again. A definition listing only the first two would
leave an implementer with no rule for the third, which is a state this record's own clause 5 creates.

Such an event is refused, counted against its offering producer with the identity named, and reaches the structured
diagnostics report. It never resolves to another note.

This is what makes ADR-0046 clause 3's orphan sentence executable, and it is stronger than that sentence requires:
clause 3 only asks that the orphan not release another note, while a generation mismatch also distinguishes an orphan
from an identity that was never minted.

### 5. The index is bounded by a checked relation; the generation never wraps

**The index relation.** The index space is at least `max_held_notes`, checked at profile construction with the same
checked arithmetic ADR-0046 clause 1 requires of the shares. An index addresses a simultaneously outstanding
obligation, and `max_held_notes` bounds how many exist. Today that field is constrained only to be nonzero
(`profile.rs:344-376`), so a profile can already name more held notes than a narrow index could address; the relation
is what closes that, and it is a construction refusal rather than an assumption about what profiles are reasonable.

**The generation rule.** A generation value is **never reused**. The counter at an index is monotone: it advances on
every reuse of that index and on every advance clause 7 causes, and it is never wound back. When an index's
generation space is exhausted, that index is **retired** — removed from its producer's free list and never minted
again — rather than restarted.

**Within one activation, aliasing cannot occur before exhaustion.** A stale identity's generation can never again
equal the generation live at its index, whatever retained that identity and however long it was retained. That is
what the never-reused rule buys, and it is bought without any bound on how long a stale identity survives — the
quantity the two previous drafts each tried and failed to bound:

- The first draft argued from a note rate divided evenly across indices. The arithmetic was wrong by three orders of
  magnitude, and even division does not hold, because with every other index held the single free index absorbs the
  whole rate.
- The second draft argued from the capacity of the stores a stale edge can wait in. That bounds concurrent occupancy
  and not the number of reuse cycles that can elapse while one stale reference survives elsewhere; a stale identity
  can also be retained in a compiled schedule under ADR-0032 clause 21, or in a producer's own note mapping, neither
  of which is a renderer-ingress store.

Both defects share one shape: they bound how long a stale identity survives, which nothing in this system actually
bounds. The rule above does not need that quantity.

**What the rule does not buy, stated plainly.** No finite identity is unconditionally alias-free, and this record does
not claim otherwise. Every scheme, including the free-running counter above, eventually runs out of distinct values;
the only real choices are what happens then and what a wrong estimate costs. This record makes both explicit:

- **Exhaustion is diagnosed, never silent.** Retiring an index shrinks its producer's usable range by one and is
  counted. A producer whose remaining range falls below its admitted simultaneous demand raises a named exhaustion
  condition; it does not quietly wrap and it does not release another note. This is the posture ADR-0032 already
  takes toward the stream clock and the epoch space: refuse rather than reissue a value that would make two things
  indistinguishable.
- **Recovery is a new identity table, and that is why clause 8 scopes identity to the table.** Retirement is
  permanent *within one table*. Building a fresh table restores the full index space and may reuse index and
  generation values, because those values belong to a different table and clause 8 rejects an identity from any
  other one. Without that scoping, recovery would be incoherent: either retirement is permanent and rebuilding
  restores nothing, or the table resets and a retained identity matches a new live entry.
- **A wrong width costs liveness, not correctness.** If Phase 3's measurement underestimates the rate, the stream
  reaches a reported exhaustion and must be re-prepared. It never releases a note it does not own. That asymmetry is
  the reason to prefer this shape over any argument that has to be *right* about a rate to be safe.

**The table identity this depends on.** Clause 8's rejection is only as good as the table identity's uniqueness, so
that issuer must **refuse** on exhaustion rather than reissue. `issue_epoch` is the model; `issue_plan_id` is the
counter-example, saturating at `u64::MAX` and returning it repeatedly thereafter (`plan.rs:145-150`). That bound is
unreachable by the order-of-magnitude argument that function's own doc comment makes, but unreachable-by-argument is
the property this record has already been wrong about three times, and refusal costs nothing to implement.

### 6. Resolution is one indexing step

A release, a redemption and a per-note expression event resolve their note by indexing the identity's index and
comparing the generation. There is no search over held notes on the audio thread. This is the property that
distinguishes the selected option from the free-running counter, and it is why the identity is a handle rather than a
name.

### 7. Mass release redeems by scope, not by enumeration

Panic, transport stop and sustain lift stay what ADR-0046 clause 6 makes them: one bounded event charged once to its
source share. Applying one ends every obligation in its scope and advances each affected index's generation, so every
identity that named one of those notes becomes an orphan by clause 4. No per-voice event is emitted and no second
release-share event is charged. A release for a note the mass operation already took is therefore an orphan rather
than a double release, which is a consequence of clause 4 rather than a separate rule.

### 8. The identity is scoped to its identity table, and neither the plan nor the epoch alone names one

An identity is valid only for the **identity table instantiation** that minted it. A fresh table identity is issued
whenever such a table is built, and the issuer refuses on exhaustion as `issue_epoch` already does rather than
saturating as `issue_plan_id` does today (`plan.rs:145-150`).

Neither existing value can serve alone, and each is ruled out by a case the other misses:

- **The epoch alone is insufficient.** ADR-0046 clause 5 activates a recompiled aggregate, and its clause 4 a
  replacement tempo map, atomically and without re-preparing the renderer, so ranges can be reassigned while the
  epoch is unchanged.
- **The plan alone is insufficient.** `PreparedRenderer::prepare` issues a new epoch but no new plan identity
  (`render.rs:404`), and one compiled plan can be prepared repeatedly, so a table can be rebuilt while the `PlanId`
  is unchanged.

One dedicated table identity covers both cases, rather than making an implementer compare two values and reason about
which combinations are reachable. An identity from any other table is rejected and counted, in the same class and on
the same path `hot.rs:98-105` already uses for a foreign `NoteSlot`. ADR-0032 clause 20's epoch rejection remains a
separate and earlier filter; this clause adds to it rather than relying on it.

**A rebuild with obligations outstanding is refused, and that limit is deliberate.** Rejecting every identity from the
previous table is safe only when no note from that table is still sounding. If one is, its release carries an
identity this clause now rejects, and ADR-0046 clause 3 guarantees that "an accepted obligation is never refused
later" — so rejecting it would break an accepted record, and stranding the obligation would leave a note sounding
with nothing able to release it. This record does not decide whether such obligations are migrated to the new table,
mass-released and redeemed atomically, or kept alive under their old table. It instead refuses the rebuild while any
obligation from the outgoing table is outstanding, which preserves ADR-0046's guarantee by refusing the operation
that has no accepted outcome rather than the release that does.

That refusal is sufficient for Phase 3, which does not swap a plan under a sounding note, and it is not sufficient
forever. The transition contract is a separate durable decision, registered as **ADR-0048** and targeted at Phase 9
beside ADR-0009's plan-swap crossfade, which is where a live swap under sounding notes is actually required. Splitting
it out is not deferral of this record's own question: identity minting, orphan detection and expression addressing are
complete without it, and the interim refusal is implementable and checkable today.

An identity is never persisted and never crosses a wire contract.

### 9. Per-note expression addresses the identity

Polyphonic pressure, per-note bend and release velocity are, for this contract, values carried by an event that names
a note identity. MPE's per-note channel and MIDI 2.0's explicit note identifier are adapter-side inputs that a Phase 9
mapper turns into an identity from that adapter's range. None of those protocols is required, implemented or
represented by this record; what it fixes is that each of them has a note to address when its phase arrives.

## Producer-by-failure-mode audit

The acceptance invariant is: **a producer that obeyed every clause above cannot release a note it does not own.** It
is deliberately *not* "cannot fail at runtime": clause 5 retires indices, so a conforming producer's range erodes
over a long stream and every class can eventually reach exhaustion. What the rows below establish is that each class
reaches it *visibly*, by a route its own contract already licenses, and never by releasing another note.

| Failure mode | Producers reached | Why the contract closes it |
|---|---|---|
| A dropped note-on is followed by its note-off | Live ingress | The producer minted nothing, so it holds no identity for that note. The edge is refused at that producer's boundary and counted there under `HOST-INV-009`; it never becomes an event. If the producer instead sends a retired identity, clause 4 makes it an orphan |
| Two producers mint the same identity | All | Clause 3's ranges are disjoint at admission |
| An identity outside the presenting producer's range | All | Attributable by disjointness; a producer-contract defect, not a load condition |
| A stale identity aliases a live note within one activation | All | Clause 5 never reuses a generation value, so no stale identity can match the generation live at its index, whatever retained it and for however long. An identity naming a retired index cannot alias either: the index holds no live note at all |
| A stale identity survives into a rebuilt table | All | Clause 8 compares the table identity, which changes on every rebuild — including a re-preparation that leaves the `PlanId` unchanged and a re-admission that leaves the epoch unchanged — and whose issuer refuses on exhaustion rather than reissuing |
| More outstanding obligations than the index can address | All | Clause 5's index relation, checked against `max_held_notes` |
| A release names an occurrence and a node that disagree | All | Clause 1 removes the node from the release; the case is unrepresentable |
| A compiled producer exceeds its admitted simultaneous demand | Compiled | It cannot. Its simultaneous obligations are statically known and admitted with the plan under ADR-0046 clause 4; a runtime miss is that clause's producer defect |
| An authored runtime producer exceeds its declaration | Authored runtime | Admission checks the source's declared simultaneous-hold maximum under ADR-0046 clause 5. Exceeding one's own admitted declaration is that clause's producer-contract defect, taking clause 7's terminal response — not a runtime refusal of a conforming note-on |
| Retirement erodes a **conforming** compiled or authored producer's range below its admitted demand | Compiled, authored runtime | This is possible, and neither admission nor the producer-defect route describes it: the producer declared correctly and did not over-emit. Clause 5 makes it a named exhaustion condition, reported and recovered by re-preparation. Attributing it as a producer defect would be false, which is why the two rows above are scoped to over-emission alone |
| A live producer exhausts its range at runtime | Live ingress | This is the one class where exhaustion is a runtime drop. It is a **third** live-input drop cause beside the queue slot and the release hold, so the specification update below adds it to `HOST-INV-009`'s licensed causes and to the exhausted-resource name it requires; without that amendment this row would contradict that invariant's closed list. Disjointness confines the drop to the offending producer |
| A mass release is followed by individual releases for the same notes | All | Clause 7 advances the generations; the later releases are orphans by clause 4 |
| A recompile reassigns ranges while **no** obligation is outstanding | All | The reassignment builds a new table, so clause 8 rejects any stale identity on the existing foreign-slot path |
| A rebuild is attempted while an obligation from the outgoing table is outstanding | All | Clause 8 refuses the rebuild. Rejecting the eventual release would contradict ADR-0046 clause 3's guarantee, and stranding the obligation would leave a note nothing can release; ADR-0048 owns the transition that lifts this limit |
| An identity leaks and is never returned | All | It consumes only its own producer's range, bounded by clause 5's index relation |

## Consequences and risks

- **Accepted cost:** identity storage is one generation per index over the admitted index space, and every note-on
  producer must return its indices.
- **Accepted cost:** within one identity table an index is spent permanently when its generation space runs out, so
  index space is consumed by a long stream and not only by concurrent notes. Clause 5 states this as a liveness
  property Phase 3 measures rather than as a bound this record asserts.
- **Accepted cost:** the identity carries a table identity, so it is wider than an index and a generation alone, and
  the crate gains a second non-reissuing issuer beside `issue_epoch`. Clause 8 states why neither existing value can
  be reused for the purpose, and `issue_plan_id` is not a third such issuer — it saturates.
- **Safety/correctness control:** clause 4's generation comparison, counted and attributable, is the executable form
  of ADR-0046 clause 3's orphan sentence. Clause 5's index relation is a construction refusal, and its generation
  rule needs no relation at all: a value that is never reused cannot alias whatever retained it.
- **Revisit condition:** reopen if the measured retirement rate makes a realistic session exhaust a producer's range;
  if Phase 6 finds that an identity cannot be assigned before the allocator has decided how many voices it owns; or
  if the measured cost of maintaining generations at publication is material against the arbiter cost Phase 3
  measures.

## Specification update

Acceptance updates the current specifications in the same transaction:

- `spec-sound-core-render-contract.md` gains the note-identity clause near `SOUND-INV-016`: a note-on names an
  occurrence as well as a compiled node, a release names the occurrence alone, and the identity is the sole authority
  for which note an event resolves to.
- `spec-host-profile-and-render-limits.md` gains clause 5's index relation alongside ADR-0046's share relations, and
  `HOST-INV-021`'s hold contract names the identity as what a hold is acquired against and redeemed by.
- **`HOST-INV-009` is amended, not merely extended.** It today licenses exactly two live-input drop causes — a queue
  slot and a producer release hold — and states that "no other live-input shortage may be discharged as a drop".
  Clause 3 creates a third, an exhausted identity range, so acceptance adds it to that closed list and to the
  exhausted-resource name the invariant requires a report to carry, keeping the causes distinguishable. Its
  attributable counters also gain the orphan and the never-minted causes, which are refusals rather than drops.
- The crate gains the identity-table issuer clause 8 requires, refusing on exhaustion as `issue_epoch` does. This
  record does **not** require `issue_plan_id` to change; clause 8 cites its saturation as the failure mode to avoid,
  not as a defect to repair here.
- [`ADR.md`](../ADR.md) records the entry. [`NOW.md`](../NOW.md) gains one line: REV-P02's `NoteEdge` deviation row
  is **not** discharged by this record and keeps the owner and the "owed before ingress" deadline REV-P02 gave it,
  and its pitch limb is blocked on ADR-0025. `NOW.md` says nothing about that coupling today.

## Review

Reviewer: independent Codex design review, read-only, before any code was written. Five rounds; the record was
split at the fifth.

**Round 1** returned six stopping-rule defects against the first draft, all confirmed against the sources by the
author before repair: the generation-width arithmetic was wrong by three orders of magnitude and its even-division
premise was false; the identity partition was incorrectly claimed to be ADR-0046's hold partition; recompilation was
incorrectly claimed to be an epoch boundary, with ADR-0032 clause 17 cited where clause 20 applies; a release
carrying both an identity and a node address had no stated authority; producer exhaustion was applied uniformly
across classes ADR-0046 keeps distinct, and the never-minted release was undefined; and the record falsely claimed to
discharge part of REV-P02's deviation row while dropping its velocity limb.

**Round 2** was a focused reread scoped to those six repairs. It confirmed two — the partition separation and the
removal of the node address from the release — and found the other four still unsound:

1. The replacement generation *relation* was also unfounded. Fixed store capacity bounds concurrent occupancy, not
   how many reuse cycles elapse while one stale reference survives; a stale identity can also sit in a compiled
   schedule under ADR-0032 clause 21 or in a producer's own note mapping, neither of which is a registered ingress
   store; and a relation counting reuse cannot bound the advances clause 7 causes without reuse. Repaired by removing
   the relation and making the generation non-wrapping, which needs no bound on how long a stale identity survives.
2. Clause 8's plan filter was unimplementable from the selected payload: with the node address removed from the
   release, nothing carried plan provenance for the existing foreign-slot path to compare. Repaired by carrying a
   `PlanId` in the identity itself, mirroring `NoteSlot`.
3. The exhaustion audit was wrong in both directions. Authored-runtime over-emission is a runtime producer-contract
   defect under ADR-0046 clause 5, not an admission catch; and live identity-range exhaustion is a **third**
   live-input drop cause, which `HOST-INV-009`'s closed list does not license. Repaired by splitting the audit rows
   and by making the `HOST-INV-009` amendment explicit in the specification update.
4. Withdrawing the discharge claim was correct, but the record then moved both of REV-P02's limbs to Phase 6, which
   silently rewrites that review's owner and its "owed before ingress" deadline, and it asserted a `NOW.md` state
   that does not exist. Repaired by leaving the row's owner and deadline exactly as REV-P02 set them and recording
   the ADR-0025 coupling as an obstacle for that separate decision rather than as a reason to move it.

**Round 3** was scoped to those four repairs. It confirmed the `HOST-INV-009` amendment and the never-reused
generation rule, and found four defects. Three were one defect seen three times: `PlanId` saturates rather than
refusing, so activation uniqueness is not guaranteed; index retirement erodes a conforming producer's range, so the
audit's "cannot exhaust" rows were false and retirement is not "not a fault"; and the fallback option restated the
impossibility premise the generation repair had just rejected. The repair withdrew the
unconditional alias-impossibility claim from clause 5 and stated exhaustion as a diagnosed and recoverable condition
in the crate's existing refuse-rather-than-wrap idiom. It did not reach two other sections that still carried the
withdrawn claim, which round 4 found. The fourth finding
was that the round-2 REV-P02 repair had been applied to one section while the Decision-boundary list, the
specification update and this Review section still carried the withdrawn Phase 6 assignment; all three are repaired
here. That residue was a self-audit failure, not a disagreement.

**Round 4** was scoped to the round-3 repairs. It confirmed the rescoped over-emission rows, the REV-P02
ownership chain and the fallback paragraph, and found three defects. The substantive one dissolved the remaining
design question: re-preparation could not be the recovery route, because permanent retirement means it restores
nothing, while a reset table reuses values under an unchanged `PlanId` — `prepare` issues an epoch but no plan
identity, and one plan can be prepared repeatedly. Read together with round 2's finding that a re-admission changes
the plan but not the epoch, neither existing value scopes an identity, and the repair introduces one dedicated
table identity that changes on every rebuild. The other two were residue from the round-3 repairs: a sentence still
described `issue_plan_id` as refusing when it saturates, and two sections still claimed unconditional
alias-impossibility after clause 5 had withdrawn it.

**Round 5** confirmed the table-scoped repair's coherence and found one new substantive defect plus three further
residues. The substantive one is why this record is now a split rather than a fifth repair: rebuilding a table
rejects every identity from the outgoing one, but a note from that table may still be sounding, and refusing its
release would contradict ADR-0046 clause 3's guarantee that an accepted obligation is never refused later. That
transition — migrate, mass-release, or keep the old table alive — is a durable decision of its own, coupled to
ADR-0009's plan swap, and it is registered as ADR-0048 for Phase 9. This record keeps the identity contract and adds
the interim refusal that preserves ADR-0046's guarantee until ADR-0048 is decided. The three residues were the
normative selection still reading "plan-scoped", `issue_plan_id` still being counted as non-reissuing, and this
Review section's own header and history contradicting themselves.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
