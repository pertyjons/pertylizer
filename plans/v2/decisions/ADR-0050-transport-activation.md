# ADR-0050: Transport activation

| Field | Value |
|---|---|
| ID | ADR-0050 |
| Status | Accepted |
| Phase | 3 |
| Created | 2026-08-27 |
| Last reviewed | 2026-08-27 |
| Related | ADR-0001, ADR-0021, ADR-0032, ADR-0043, ADR-0046, ADR-0047, `SPEC` sound-core render contract, `SPEC` host profile and render limits |
| Supersedes | — |
| Amends | ADR-0047 clause 7, for the transport-activation case only |
| Superseded by | — |

## Durable boundary

**Delivered timing behaviour, and a real-time ownership boundary.**

Phase 3 owes four re-anchoring moments — play, seek, loop wrap and offline range start — and three of them happen while
a stream is already rendering. `SessionScheduler` re-anchors its own pairing today, but nothing moves a *prepared*
renderer, so a seek during playback is unimplementable: `CompiledEventScheduler::prepare` refuses an anchor that
disagrees with the renderer's, which is the refusal standing in for this decision.

What "takes effect at engine sample `T`" means is delivered timing behaviour. It is not persisted and not on a wire —
ADR-0032 clause 7 keeps `SampleTime` and `PlanPosition` out of both — but a listener hears it, and changing it later
changes what they hear. That is the durable boundary.

The second boundary is ownership. Building an activation allocates and mints note identities, so it cannot run in a
callback; adopting one must run in a callback, because only the audio thread knows which quantum is next. The two
halves of a stream therefore need separate owners, and which state belongs to which half is a real-time boundary under
`PROCESS.md`'s durable-decision test.

**Why now.** The immediately next dependent slice is re-anchoring a prepared renderer, and an independent design review
blocked it before implementation with five findings, each of which turned out to be a question this record answers.
Deferring is not free: every further slice that touches transport, session commands or position-aware kernels
accumulates around an undecided activation boundary, which is when reversal cost starts multiplying.

**Coupled decisions that must close with it.** None of the six below can be decided alone; they are decided together
here. Decisions this record deliberately does **not** close, and does not need: ADR-0022 (hardware mapping),
ADR-0023 (same-sample ordering), ADR-0009 (plan-swap crossfade and voice-state migration), ADR-0048 (identity-table
rebuild), ADR-0049 (the tempo-ramp law) and the sub-quantum representation of an activation point.

## Decision boundary

This record settles six things, and the sixth is one the blocking review named and the original list of five omitted.

1. the effective activation point and its half-open boundary;
2. what happens to already-rendered carry;
3. the atomic state set and its failure behaviour;
4. the policy for note obligations crossing activation;
5. the freshness rule; and
6. **locate catch-up** — what restores the state an admitted suffix cannot reach.

**Scope: a stream whose note producers are compiled.** Successive review rounds established that extending the identity
and hold halves of this contract to a **non-compiled** producer cannot be decided here, because the resources they
turn on — release holds and the live ingress boundary — do not exist. Clause 8 states the two obligations that
creates and what they bind.

The scope is the whole of what is buildable today, and the reason has to be stated precisely rather than
comfortably: a plan **may** declare a non-compiled note producer — `NoteProducerDeclaration` has a `compiled` flag
and admission partitions ranges across every declared producer — but nothing non-compiled can yet *emit*, so the
compiled producer is the only one that mints. It is that narrower fact the scope rests on, not the false one that
plans declare nothing else.

**Non-goals.** This record does not claim sample-exact seek or loop. It fixes a quantum-granular activation point and
says so in every place a reader could mistake it for the master plan's sample-exact requirement. It does not decide
how a sub-quantum activation would be represented, how voice state migrates across a plan swap, or how two activations
that fall in one quantum are ordered against each other — that last one is ADR-0023's.

## Evidence

Verified against the code at this commit's parent.

- The renderer serves callers from an output carry primed with `Q` frames of silence
  (`crates/synth_engine_v2/src/render.rs`, `PreparedRenderer::prepare`), so delivered output trails engine time by
  exactly `Q` frames. That is the stream's declared `added_latency`, not an additional cost this record introduces.
- The render loop derives **one** plan position per quantum (`render/hot.rs`, `render_quantum` calling
  `plan_position_of(quantum_start)`), and one of the nine kernels reads it. Events are unaffected: they are placed
  off-thread at full resolution and carry sample offsets inside their quantum.
- `StreamAnchor` is a `(SampleTime, PlanPosition)` pair (`src/time.rs`). An anchor **value** does not identify a
  timeline: replace a tempo map so it differs only after the current tick and the anchor is bit-identical while every
  future position moves. ADR-0032 clause 26 already says a compiled list is invalidated by a tempo edit.
- `CompiledEventScheduler::prepare` allocates and calls `stamp_compiled`, which mints identities, so it cannot run in
  a callback (`src/schedule.rs`).
- A freshly prepared schedule starts with `arbiter: None` (`src/schedule.rs`), so an unconstrained replacement could
  adopt a different arbiter than the stream's, against ADR-0046 clause 2.
- `stamp_compiled` refuses an unpaired **release** but not a leftover **note-on**, so a discarded schedule holding one
  keeps a minter index. There is no `Drop` in the crate, so nothing reclaims it.
- ADR-0046 clause 1 requires the session share to cover "the largest catch-up batch over every legal locate position
  in that plan", and clause 3 requires an accepted session snapshot to publish completely. `AdmittedCompiledStream`
  refuses every position before its anchor (`SchedulePrepareError::BeforeAnchor`), so a suffix cannot carry the last
  automation value preceding a seek target.
- `IdentityTable::release_all(ReleaseScope)` and `ReleaseScope::{Everything, Producer}` already exist
  (`src/identity/hot.rs`, `src/identity.rs`); `LiveNotes` has no counterpart.

**Uncertainty that could change the decision.** The activation point is the one choice a measurement could overturn:
if a listener can reliably distinguish quantum-granular from sample-exact seek placement at `Q` = 64 and 48 kHz
(1.33 ms), the first stage is a worse default than it looks. No such measurement exists, which is why this record
states the limitation rather than claiming the gate.

## Options

### The effective activation point

**A. Sample-exact.** Activation takes effect at engine sample `T` exactly. It requires either splitting a quantum,
which ADR-0001 forbids, or a sub-quantum plan mapping the position-aware kernels can read. It also creates a junction
window: the old stream's events before `T` and the new stream's events after it can fall in one `Q`-frame window that
neither admission covered, so it needs a junction check in the shape of ADR-0046 clause 4's periodic loop
extension.

**B. The first quantum boundary at or after `T`.** Quantum-granular, and the boundary is a function of `T` and `Q`
alone, so it is invariant to how the host partitions its callbacks. Because shares are charged per **destination
quantum** and the boundary is a quantum boundary, the quantum ending at the boundary holds only old events and the
quantum starting at it holds only new ones — **the junction never mixes the two streams inside one quantum**, and no
new admission rule is needed. Its cost is placement error of up to `Q - 1` frames, 1.33 ms at 48 kHz.

**C. The start of the next render call.** Simplest, and wrong: the activation's engine time would depend on the host's
block size, so the same request would land at different times under `1 x 4096` and `64 x 64`. That is exactly what
ADR-0001 exists to prevent.

### Already-rendered carry

**A. Keep it.** The carry holds audio for engine samples strictly before the renderer's clock, and the activation
point is at or after the clock, so the carry is the old mapping's audio for the old mapping's time range.

**B. Discard it**, as `PreparedRenderer::fault` does. Between calls the live carry is at most `Q` frames, so it
deletes up to `Q` frames of legitimately rendered audio and delivers a gap of the same length.

**C. Crossfade.** ADR-0009's topic, and it needs voice-state migration to be worth anything.

### The atomic state set

**A. A renderer-only `reanchor`.** Cheap to delete, and rejected for the reason it looked attractive: it lets the
mapping move independently of the placed schedule, and naming that insufficient in prose does not stop a caller
reaching the invalid intermediate state.

**B. One value carrying every piece, adopted or refused as a unit.**

### Freshness

**A. Epoch alone.** Insufficient: an anchor value does not identify a timeline, so a candidate built against a
superseded tempo map passes.

**B. A single-slot exchange the builder can withdraw from.** At most one candidate exists, so nothing can be stale.
It requires the builder to observe adoption before it may build again, which puts the off-thread half's progress
behind the audio thread's.

**C. A monotone activation sequence the candidate names as its predecessor.** Compare-and-swap semantics: an offer is
accepted only when it supersedes exactly the state in force. Two candidates may be *built*; the one whose predecessor
has moved is refused rather than silently applied. The builder never waits on the audio thread.

### Note obligations crossing activation

**A. Carry them across.** Impossible without cross-schedule identity plumbing: the new schedule mints its own
occurrences and has no release naming the old ones.

**B. Leave them sounding.** A stuck note, and a leaked minter index with it.

**C. End them at the activation boundary**, as ADR-0046 clause 6's bounded mass release, scoped to the producers whose
schedule is being replaced.

### Locate catch-up

**A. Nothing.** Every automated parameter keeps whatever the pre-seek position left it at, which is audible and wrong.

**B. Re-admit the whole plan from position zero at every seek.** Correct and unbounded: the work scales with the
distance sought.

**C. A bounded catch-up batch in the activation**, holding the last value in force at the new position for every
prepared target, admitted against the session share as ADR-0046 clause 1 already requires.

## Decision

**Transport activation is a value, built off the audio thread, adopted whole at a quantum boundary.** The nine
clauses below are one contract.

### 1. The effective point is quantum-granular and half-open

An activation names a **requested engine time** `T`. Its effective point is the first quantum boundary at or after
`max(T, clock)`, where `clock` is the renderer's own clock at the call that adopts it. The new mapping governs the
half-open engine-time interval `[effective, ...)`; the old mapping governed `[..., effective)`.

`T` is immutable in the value, exactly as an event's stamp is under ADR-0043. When `T` precedes the clock **at the
offer** the activation is **late**: it activates at the clock and increments an attributable late-activation counter, so the delay
is reportable rather than silent. This is ADR-0043's preserving late clamp applied to an activation, and it is the
same rule for the same reason.

**Which clock, and the answer is the one at the offer rather than at adoption.** The implementation slice established
this and the record is amended to say it, because leaving it implicit is what let three successive implementations
disagree with the contract. Lateness is a statement about the *candidate*: it was finished after the time it named had
already passed, which is the only thing an attributable counter can tell a reader to fix. Asked at adoption the
question has no useful answer — the clock stands on the effective boundary by then, so every request that does not
fall on a quantum boundary would answer yes, and the counter would report ordinary snapping as delayed preparation.

Lateness and displacement are therefore **independent**, and neither implies the other. A `T` one frame past a
boundary, offered against a clock already standing on the next one, is late and displaced by nothing: it activates
exactly where its own snap would have put it. A `T` offered well before the clock reaches it is displaced by up to
`Q - 1` frames and is not late at all. The effective point remains `max(T, clock)` snapped forward, evaluated at the
call that adopts; only the counter's question is asked earlier.

**The adopted anchor is `(effective, requested position)`, and the placed stream shifts with it.** The engine-time half
of the anchor is snapped; the plan-position half is not, because moving it would silently seek somewhere other than
where the caller asked. The candidate is stamped off the audio thread against `T`, so when `effective` differs from `T`
every placed event is displaced by the same `effective - T`. The stream therefore carries that displacement as **one
offset applied when an event is published**, not as a rewrite of the stamped list: the shift is uniform, so an
`O(1)` value reproduces it and an `O(n)` pass on the audio thread is not needed.

This is a **placement**, not a clamp, and the distinction matters because ADR-0043's immutable stamp is about not
rewriting one late event's declared time. Here the anchor moved, and an event's engine time is a function of its plan
position and the anchor. The published stamp is the time the event genuinely happens.

The shift preserves everything this clause relies on. Admission is over plan positions against every anchor phase, so any
anchor is admissible; the new stream's first event is still at or after `effective`, so the junction still puts the
two streams in different quanta.

The effective point is a function of `T`, `Q` and the clock alone. It does not depend on how the host partitions its
callbacks, and a named test asserts that over the four partitions the exit gate lists.

**This is not sample-exact seek or loop.** Placement error is up to `Q - 1` frames — 1.33 ms at 48 kHz. What a
listener *hears* is that plus the stream's existing `Q`-frame output latency, so an on-time request can be heard as
much as `Q + (Q - 1)` = 127 frames after the sample it named, about **2.65 ms at 48 kHz**. Only the first `Q - 1` of
those is new; the rest is the latency clause 2 leaves alone. The master plan's Part III requirement for sample-exact
loop and seek events is **not** satisfied by this stage, and Phase 3's exit gate may not be read as closing it on the
strength of this record.

**A repeating wrap keeps its ideal phase, and that is a rule rather than an implementation detail.** A loop whose
length is not a whole number of quanta snaps at every wrap. The requested time of the `k`-th wrap must therefore be
derived from the **ideal** timeline — `T_0 + k * length` — and never from the previous wrap's effective point. Under
the ideal derivation each wrap's error is an independent value in `[0, Q)`, so the audible period jitters by less than
one quantum and returns; under the other, the errors accumulate and the loop's period is permanently longer than the
one the user set. The two look identical for a loop length that happens to be a multiple of `Q`, which is exactly why
the rule has to be written down.

**What quantum granularity buys, beyond implementability.** Shares are charged per destination quantum. Because the
effective point is a quantum boundary, the last quantum of the old stream and the first of the new are different
quanta, so no quantum ever holds events from both streams. The junction is therefore admissible under the admissions
the two streams already passed, and needs no third rule. A sample-exact activation would need a junction check in the
shape of **ADR-0046** clause 4's periodic loop extension, and that check is not in this record.

### 2. The already-rendered carry is kept

Adoption does not touch either carry. The output carry holds audio for engine samples strictly before the clock, the
effective point is at or after the clock, and that audio was rendered correctly under the mapping in force when it was
rendered. Discarding it would deliver a gap for no correctness gain.

**How much audio that is, exactly.** Between renderer calls the live carry never exceeds `Q` frames. A call that
renders `k` quanta leaves `carry + kQ - frames` live, and `k` is the smallest count for which that is non-negative,
so the remainder is in `[0, Q)`; a call that renders none only shortens the carry it started with, and the stream
starts with exactly `Q`. The `maximum_block_size + Q` the buffer is *sized* to is the peak reached **inside** a call,
after its quanta are appended and before its frames are copied out — which is another way of seeing why clause 4 puts
adoption between calls rather than inside one.

**That statement depends on where adoption happens, and clause 4 is what fixes it.** Adoption occurs *between*
renderer calls, never inside one: a `render` that spans several quanta accumulates them in the carry before copying
out, so an adoption in the middle of one would leave the carry holding audio from both sides of the boundary and the
sentence above would be false. Clause 4 splits the crossing host block so that the statement stays true by
construction rather than by care.

A listener therefore hears an activation `Q` frames after its effective engine time. That is the stream's declared
`added_latency` and this record adds nothing to it. An operation that must silence what is already rendered is a
**fault**, not an activation, and takes ADR-0021's terminal response instead.

### 3. The atomic state set

One activation carries, and swaps together:

- the **anchor**, the `(SampleTime, PlanPosition)` pairing both the placed events and `plan_position_of` read;
- the **placed, stamped compiled schedule** and its cursor;
- the **loop interval** in force, already admitted under ADR-0046 clause 4 by `admit_loop` — **recorded, and not
  yet enforced**: a wrap is not implemented, so nothing repeats the interval and the candidate's schedule is not
  bounded by it. An event past `end` therefore plays, and reserves an identity, where under wrapping it would be
  unreachable. That is a debt the wrap slice inherits rather than a rule this one breaks, and it is written here
  because an independent review found it worth asking about three times;
- the **tempo map** in force, when the activation replaces one; and
- the **locate catch-up batch** of clause 7.

The stream's **arbiter** is not carried and is not swapped: ADR-0046 clause 2 admits exactly one per stream, so an
adopted schedule inherits the latched arbiter identity of the schedule it replaces rather than starting unlatched.
The **identity table** is likewise not swapped — both schedules mint from the one table, which is what makes an
occurrence resolvable across an activation — but clause 5 governs what the retired schedule leaves in it.

**Failure leaves everything as it was, and refusal happens at the offer rather than at adoption.** Most checks run
while the candidate is being built, off the audio thread: admission of the stream and the loop, placement against the
new anchor, and stamping, which `stamp_compiled` already makes all-or-nothing on the minter — a *failed* build leaves
nothing behind.

**A candidate is stamped against a copy of the minter, and that is what makes "unchanged" true rather than
aspirational.** Two earlier drafts of this record got it wrong in two different ways, and both were found by
independent review. Stamping is not reversible by releasing what it minted: a paired note-on and release inside one
list already advanced that index's generation and may have retired it, and `IdentityTable`'s own documentation says
so. A withdrawal that released the outstanding set would therefore restore nothing for a fully paired candidate while
a refused one had permanently spent generations.

So the control takes a **working copy** of the minter for the build, and before stamping it releases into that copy
the occurrences the outgoing schedule still **reserves in the allocator** — the note-ons its own list never paired,
recorded when it was stamped. The candidate is then minted against the table as it will be once the outgoing schedule
is gone, rather than against one that still holds it.

**That set is not the set of notes the boundary ends, and equating them would be false.** They overlap without
containing each other, and a review round caught an earlier draft saying otherwise. A note-on at plan sample 0 paired
at 100, with the activation at 50, *is* sounding at the boundary but freed its index during stamping, so the
allocator does not hold it. An unpaired note-on at 100 *is* held by the allocator but never sounded. Each half deals
with its own set — the allocator with what it still reserves, the registry with what is actually sounding — and
neither needs the other's. Nothing leaks either way, and the reason is that the two halves model different things,
which is the same reason `LiveNotes` exists at all.

Four properties follow, and the last is the one a range-wide reclaim could not have given:

- **Withdrawal is free and exact.** A candidate that is abandoned, or returned refused at the offer, drops its copy.
  The authoritative table never saw the mint, so nothing is consumed — not an index, not a generation.
- **The copy is promoted when the retired value is collected, not when the offer is accepted.** An accepted offer
  makes adoption infallible *if it is reached*, which is not the same as certain: the renderer can end the epoch
  first — an oversized callback, a publication fault, a clock exhaustion — and then no later call advances toward the
  boundary. Promoting at acceptance would have released the outgoing reservations and spent the candidate's
  generations for an activation that never happened. An independent review found exactly that, and collection is the
  first moment at which adoption is a fact rather than a plan. A faulted epoch needs no reclamation at all: the
  stream is over, re-preparation issues new tables, and the copy is discarded with everything else.
- **No candidate built between acceptance and collection may be adopted**, which is what removes the window that
  promoting late would otherwise open. In that window the authoritative table still describes the outgoing schedule,
  so a build against it would be wrong.

  **Outstanding** means *accepted at the offer and not yet collected*, and the precision matters because this clause
  deliberately allows two candidates to be **built** against one in-force sequence and refuses the loser. Those two
  rules are about different moments: building competing candidates is fine while neither has been accepted, and it is
  acceptance that closes the door until collection.

  **Amended by the implementation slice, and the amendment is a change of mechanism rather than of effect.** An
  earlier revision of this clause said the control must not *build* in that window — "the rule is that there is no
  such build, not that it compensates". The control cannot **observe** an acceptance in the shape clause 9 gives it:
  `offer` is a method on the schedule, which the audio thread owns, so the only two moments the control sees are the
  build and the collection. A prohibition it cannot evaluate was implemented as "no build while any candidate exists",
  which forbade the competing builds the paragraph above deliberately allows — an independent review found exactly
  that.

  What replaces it is structural rather than compensatory. The control **issues** what a candidate supersedes, from
  the sequence it last promoted; a caller cannot name one. A candidate built between acceptance and collection
  therefore carries the superseded value necessarily rather than by accident, and the offer refuses it. That is not
  compensation after the fact: no such candidate can reach adoption, which is the property the prohibition existed to
  guarantee, and it is now guaranteed by what a candidate *is* rather than by a rule about when one may be made.

  The refusal a candidate in that window receives is `RetiredUncollected` rather than `Superseded`, because the
  uncollected retirement is the cause and the stale sequence is its consequence. Reporting the consequence would send
  a reader looking for a racing seek when the fix is that the off-thread half has not collected.
- **The two schedules never compete for the range.** Releasing the outgoing set into the copy first is what removes
  the overlap; without it a producer whose declared polyphony is exactly used by the outgoing schedule could not
  build any replacement at all, because the replacement's first note-on would be refused as over-emission. Neither
  ADR-0046 nor ADR-0047 reserves capacity for a preparation-time overlap, and this design does not create one. **Offering** the
candidate to the stream can refuse for exactly five further reasons — a schedule paired with another stream's
renderer, a stream that has already faulted, a stale epoch, a superseded sequence (clause 6), and a return slot the
off-thread half has not yet emptied — and each leaves the stream running on the state in force. **Four of the five
increment the refusal counter and the pairing does not**, which is why it is checked first: the counters belong to
the stream that was offered to, and attributing this refusal to a renderer that is not this schedule's half would
report it against a stream nobody asked anything.

The last two were found by the implementation and are recorded here rather than left implicit. After a terminal fault
no later call advances toward a boundary, so a candidate accepted then would never be adopted, never be collected,
and never be withdrawable — an impossible state change reported as accepted.

**Adoption itself is an infallible move.** Once an offer is accepted the candidate is pending, nothing between the
offer and the boundary can invalidate it, and the swap at the boundary has no branch that can fail. That is
deliberate: a refusal discovered at the boundary would have to either roll back a partly-applied state set or fault
the stream, and the first is what the atomic set exists to avoid while the second would turn a caller mistake into a
terminal fault. A violation detected *after* adoption is ADR-0046 clause 7's terminal response and not a rollback.

**The retired schedule's unpublished events are not dropped events.** A seek or a wrap means the rest of the old
timeline is no longer in force, so those events describe a timeline that has ended rather than work the renderer
declined to do. ADR-0001 clause 16's prohibition is on the renderer silently discarding an event of the timeline it
is rendering; this is the timeline itself being replaced, which is the operation the user asked for. No counter
records them, because a count of "events on a timeline that was seeked away from" is a count of the seek.

**Nothing is dropped on the audio thread.** Adoption *exchanges*: **every** piece the activation replaces moves into
a return slot the off-thread half collects and deallocates — the retired anchor, schedule, loop, tempo map and
catch-up batch. Naming only some of them would be worse than saying nothing, because the ones left out are the ones
that own allocations: a `TempoMap` holds a `Vec`, so a replacement that did not return it would free that allocation
on the audio thread while claiming the exchange is real-time safe.

**The slot is one slot used in both directions, and it is occupied in two different ways.** Between an accepted offer
and adoption it holds the pending candidate; between adoption and collection it holds the retired value. An offer
made while it is occupied is refused either way, which is backpressure rather than a fault — but the two causes are
not the same condition and a diagnostic must not report one as the other. Only the second means the off-thread half
has not collected.

### 4. Adoption splits the crossing call

When the effective point falls strictly inside the quanta a host block would render, the stream renders that block as
**two** renderer calls — the quanta before the boundary, then the quanta at and after it — adopting between them.
Each call is served by its own sealed publication over its own quanta, which are disjoint.

This is free rather than clever: ADR-0001's partition invariance is exactly the property that the same total frames
rendered as two calls produce the same audio as one, and the four-partition test already asserts it. It is what makes
clause 1's boundary reachable without splitting a quantum.

**Where the block is cut is determined rather than chosen.** With `c` the live carry, `Q` the quantum and
`k = (effective - clock) / Q` the number of quanta the old mapping still owns, the crossing case is exactly
`frames > c + kQ`, and the first sub-call is `c + kQ` frames — the largest request that renders precisely `k` quanta,
because `quanta_needed_for(f)` is `ceil((f - c) / Q)`. When `frames <= c + kQ` the boundary is not crossed inside the
call and no split happens; when `k` is zero the activation is adopted before the call rather than inside it. Both
sub-calls stay within `max_quanta_per_callback`, since the first renders `k` of the quanta the whole call would have
rendered and the second requests fewer frames than the original.

### 5. Notes crossing activation end at the boundary

Every note sounding at the effective point that was started by a **producer whose schedule the activation replaces**
is ended by the activation, at the boundary, as ADR-0046 clause 6's bounded mass-release operation: one operation
scoped to those producers, charged to the **session share**, never expanded into one event per voice.

**A compiled producer has no hold to redeem, and saying otherwise would misstate the accounting.** ADR-0046 clause 6
gives compiled releases a plan entitlement rather than a hold — "Compiled releases use plan entitlements and need no
hold" — so ending a compiled note frees an identity and nothing else. That is the whole of what this clause covers,
because this record's scope is a stream whose note producers are compiled. What a mass release must do when its
scope reaches a **non-compiled** producer — redeem that producer's holds atomically as it is applied — is clause 8's
first obligation, and it is unsupported here rather than described loosely.

The scope is the retired schedule's producers and not everything. A seek moves plan time; it does not lift a
performer's finger, and ending a live ingress note because the transport moved would be an audible defect. Clause 6's
own words are "owned voices within the source event".

**The operation has two halves in two places, and only one of them is on the audio thread.**

On the **audio thread**, adoption clears the retired producers' entries from the live-note registry and lowers the
gates they name. `IdentityTable::release_all` already takes a `ReleaseScope`; `LiveNotes` has no counterpart and must
gain one, together with the producer ranges a scope names.

Scoping that clear to a producer range is safe, and the reason has to be stated exactly, because a plausible-sounding
version of it is false. It is **not** that an occurrence enters the registry when its event is applied — the renderer
admits occurrences during resolution, before the first quantum of the call renders. It is that the candidate's events
are not presented to **any** render call until after adoption, so at the moment of adoption the registry can hold no
occurrence of the incoming schedule whatever the order inside a call.

The **allocator** half is clause 3's copy, and it ran earlier: the copy released the outgoing schedule's outstanding
occurrences before the candidate was stamped, and clause 3 promotes it when the retired value is collected.

**This record amends ADR-0047 clause 7 for the activation case, and calls it an amendment rather than a
reconciliation.** Two drafts tried to argue the two records already agreed; an independent review was right both
times that they do not. Clause 7 says that *applying* a mass release advances each affected index's generation. Here
the allocator advance happens earlier — at stamping for a note the outgoing list paired, at build time in the copy
for one it never paired — and the authoritative table takes it later still, at collection. Neither is "at
application". Saying otherwise would be the kind of false agreement this project's review protocol exists to catch.

**What the amendment preserves is the property clause 7 is for**, which is that a release arriving after a mass
release resolves as an **orphan** rather than ending some other note. That behaviour is the registry's, the registry
is cleared at the boundary, and a release arriving afterwards therefore resolves through nothing and is counted —
exactly the outcome clause 7 names. The protection against a stale identity matching a note the incoming schedule
started comes from SOUND-INV-017's never-reused generation: an index the candidate mints carries a new one whenever
it was advanced.

**What the amendment costs is stated rather than buried.** Between the boundary and collection the authoritative
allocator does not yet reflect the ended notes. Nothing observes it in that window — the renderer reads only the
registry, and clause 6's issued sequence makes any candidate built there unadoptable — but that is a property of the
present single-minter arrangement, and clause 8's second obligation is where it stops holding. Acceptance therefore
records the amendment in `SOUND-INV-017` rather than leaving the two specifications to disagree.

Neither table is *replaced*: both halves keep the identity they were opened with, so an occurrence stays resolvable
across an activation and the registry identity the renderer's foreign filter compares against does not move under
it.

**A suffix omits a release whose note-on lies before its anchor, and that is what makes an ordinary seek buildable
at all.** Seek between a compiled note's on and off edges and the suffix contains a release with nothing to pair it
with; stamping refuses such a list, so without this rule the commonest seek there is could not produce a schedule.
The omission is correct rather than lenient: after the seek that note is not sounding — the boundary mass release
ended it and the new stream never started it — so a release for it has no meaning in that stream. It is performed
where the suffix is built, off the audio thread, and **counted**, so it is a named transformation rather than a
silent drop by the renderer, which ADR-0001 clause 16 would forbid.

**Note chasing is deliberately not done.** A player that re-starts such a note at the seek destination is a legitimate
and different product choice; it would begin the note's envelope again at the destination rather than continuing it,
which is not the same sound as having played through. That choice needs its own record, and this one does not make it.

**The audible consequence is stated rather than hidden.** A note sustaining across a loop wrap or a seek is cut, and
not resumed. That is what a wrap means under a compiled plan whose release lies inside the loop, and carrying it
would require the new pass to release an occurrence it did not mint.

### 6. Freshness is a sequence, not a value comparison

The off-thread half issues a strictly increasing **activation sequence**. Every candidate names the sequence it
supersedes, and the stream **accepts the offer** only when that equals the sequence in force. A candidate built
against a state that has since been superseded is refused there and counted; it is never applied.

**The state machine is exactly this, and the precision matters because "the next one issued" is ambiguous.** The
value **in force** starts at the sequence the stream was opened with and changes at **adoption alone** — never at
issue. Issuing a candidate takes the next sequence and records which value it supersedes. Adoption requires
`candidate.supersedes == in_force`, and on success sets `in_force = candidate.sequence`.

Two consequences follow, and both are the ones a ticket model has to argue for separately. A candidate that is
abandoned, withdrawn or simply never offered **consumes no sequence** — clause 3 governs the identities it does
reserve — because the value in force did not move, so the next
candidate built against it is adoptable and cancellation cannot wedge the stream. And two candidates built against
one in-force value are ordered rather than raced — whichever is adopted first moves the value, and the other is
refused, so a superseded intent can never activate after the intent that replaced it.

**Serialised activation can make an activation late, and that is why clause 1's counter exists.** Preparation is
off-thread work of unbounded duration; a candidate that takes longer to build than the distance to its requested time
activates at the clock instead. The counter is what makes that visible rather than leaving a seek that felt sluggish
unattributable.

An anchor value cannot serve here, and neither can the epoch alone: a tempo map replaced so that it differs only after
the current tick leaves the anchor bit-identical and every future position moved. The sequence is what distinguishes
timelines that agree at one point.

The candidate also carries the **stream epoch**, and a candidate from another epoch is refused as stale exactly as an
event from another epoch is under ADR-0032 clause 20.

A single-slot withdrawable exchange would make a stale candidate impossible and is a legitimate alternative; it is not
selected because it makes the off-thread half's progress depend on the audio thread's, and the sequence obtains the
same refusal without that coupling.

### 7. A locate carries its catch-up batch

An activation whose new position is not the continuation of the old one — a seek, a loop wrap, an offline range start
— carries a **catch-up batch**, and it covers **every** prepared target rather than only the automated ones. For a
target with a value established before the new plan position it carries the last such value; for a target with none it
carries the value that target was **prepared** with.

The second half is not a detail, and leaving it out was a hole an independent review found. A control value lives in
node state across an activation, so playing past a parameter change and then seeking to before that parameter's first
automation point — position zero included — would leave the value the automation set. The stale value would be exactly
the one the seek was supposed to leave behind. Covering every target also makes the batch's size precisely the plan's
prepared-target count, which is the quantity admission has to check. The batch is built off the audio thread with the rest of the candidate, is charged to the **session
share**, and publishes completely, which is ADR-0046 clause 3's rule for an accepted session snapshot.

Its size is bounded by the number of prepared targets, which the plan fixes, and plan admission checks that bound
against the session share over **every legal locate position in that plan** — the check clause 1 already names. A plan
whose catch-up does not fit is refused at admission, where a caller can still be told.

Without it an admitted suffix is not a correct seek: `AdmittedCompiledStream` refuses every position before its
anchor, so nothing in the new stream carries the automation value that was in force when the user seeked past it.

**The catch-up applies before the new stream's events at the same sample**, and that is a restatement of what a
catch-up *is* rather than a general same-sample policy. The batch carries the state that was already in force at the
destination; the stream carries what happens from there. Applying them the other way round would let a stale
restoration overwrite the first event of the new timeline, which is the one case a catch-up must never produce.
ADR-0023 may refine how session, note and automation events at one sample order against each other, but it cannot
invert this pair without making the catch-up meaningless.

### 8. Two obligations this record does not close

Both fall outside the scope stated at the top, and both are named here rather than left to be discovered.

**A non-compiled producer's release holds have no redemption authority at the boundary.** ADR-0046 clause 6 requires
a mass release to redeem every affected hold atomically as it is applied. Clause 5's audio-thread half clears the
registry and lowers gates; it names no hold store, because none exists. Redeeming earlier — at build or at offer —
would be wrong, since the outgoing note is accepted and sounding until the boundary. So a schedule replacement whose
producers include a non-compiled one is **not supported** until the slice that introduces holds supplies that
authority. The compiled case is complete: clause 6 gives compiled releases a plan entitlement, so there is no hold.

**A second minting producer breaks clause 3's copy.** The copy is taken at build and promoted at collection, and it
is the whole table. If another producer minted into the authoritative table in that window — which `HOST-INV-009`
will permit a live note-on to do at its own boundary — promotion would erase that mint and wind its index's
generation backwards, against SOUND-INV-017. While an activation is outstanding, therefore, **the control is the
only minter**. That is satisfied by construction today, because the compiled producer is the only one that mints;
enabling live ingress is what makes it a real constraint, and resolving it is that slice's entry cost rather than a
defect discovered later.

Both obligations are recorded in `NOW.md` against the slices that own them.

### 9. The stream has two owners

The off-thread half owns the tempo map, the current anchor and loop, the admitted streams, the identity **minter**,
and the building of candidates. The audio-thread half owns the clock, the carries, node state, the **live-note
registry** and adoption.

This is a split of what already exists rather than a new structure: `PreparedRenderer` documents `minter` as "off the
audio thread" and `live_notes` as "the audio thread's half" in the same struct today. Giving them different owners is
what lets a candidate be built while the stream renders, without a lock the real-time rules forbid and without the
`&mut PreparedRenderer` that `prepare` needs today.

## Consequences and risks

- **Accepted cost.** Activation placement is quantum-granular: up to `Q - 1` frames, 1.33 ms at 48 kHz, and up to
  `Q + (Q - 1)` = 127 frames — 2.65 ms — as *heard*, of which only the first `Q - 1` is new. A loop whose length is
  not a whole number of quanta jitters by under one quantum per wrap; it does not drift, but only because clause 1
  requires the ideal derivation. A note sustaining across a wrap or a seek is cut at the boundary. A crossing host
  block costs a second publication pass. EVD-0017 measures one such pass at 0.014 % of the callback budget, but its
  full arm fills the **compiled share only** and the record itself calls the figure a floor, because the other five
  producer classes do not exist yet. It is therefore evidence that a pass is cheap today, not that two passes of the
  eventual full partition are; the reselection measurement the host-profile specification still owes is what would
  establish that, and this record does not claim it has. Moving the minter out of `PreparedRenderer` breaks
  `CompiledEventScheduler::prepare`'s signature and `stamp_compiled`'s, inside the experimental crate, and
  `LiveNotes` grows a scoped mass release and the producer ranges it needs.
- **Safety/correctness control.** Failure is confined to the off-thread build and to the offer: offering refuses for
  exactly the five reasons clause 3 lists, each leaves the stream running, and adoption itself cannot fail. Nothing
  is dropped on the audio thread, because adoption exchanges rather than replaces. The junction needs no admission rule of its own, and clause 1 states why. Named tests:
  partition invariance of the effective point over the four partitions; a superseded candidate refused; a stale-epoch
  candidate refused; a full return slot refusing rather than dropping; a note cut at the boundary with its index
  reclaimed; a withdrawn candidate leaving the table untouched, whose falsifier is the later build that would
  otherwise fail as over-emission; a seek between a note's edges producing a buildable suffix with the omission
  counted; and a repeating wrap at a non-quantum length whose period returns rather than drifting over many wraps.

  **Two cases this list first named are not covered, and the omission is deliberate rather than an oversight.** A
  live ingress note **not** cut needs a producer that can emit, which clause 8 puts out of scope; a catch-up batch
  restoring a seek target's preceding value needs clause 7, which the implementation slice did not build. Claiming
  either as evidence would have made this record assert tests that do not exist, which an independent review found
  it doing. `SOUND-INV-018`'s conformance row is the inventory of what is actually checked.
- **Owed, and named rather than discovered.** Clause 8's two obligations: a non-compiled producer's holds have no
  redemption authority at the boundary, and a second minting producer would be erased by the copy's promotion. Both
  bind the slice that enables live ingress, and both hold by construction until it does.
- **Revisit condition.** The activation point is revisited when the master plan's sample-exact loop and seek
  requirement is taken up, or when a position-aware kernel makes quantum-granular plan position audible. Either
  reopens clause 1 and, with it, the junction question clause 1 defers. ADR-0009 reopens clause 2 if a plan swap wants
  a crossfade, and clause 5 if voice state is to migrate. ADR-0023 owns the order of two activations in one quantum.

## Specification update

Acceptance creates `SOUND-INV-018` in the sound-core render contract, which carries clauses 1 to 9 as the rule
implementation follows; **amends `SOUND-INV-017`**, which records clause 5's earlier generation advance rather than
being left to disagree with it; and updates the host-profile and render-limits specification with `HOST-INV-022`:

- the **activation exchange** is recorded as a fixed non-dropping session/transport store, distinct from
  `command_queue_capacity`. It takes **no** row in the renderer-ingress source-store registry: that table is
  `HOST-INV-009`'s live-input drop-licence register and states that non-dropping session/transport stores do not
  belong there. A full slot refuses the offer at the caller boundary, which is clause 3's backpressure and not a drop,
  so no drop licence is being requested;
- the session share's plan-admission check names the locate catch-up batch as its bounded contributor, per clause 7;
  and
- a new invariant records that an activation's effective point is a quantum boundary and that sample-exact seek and
  loop are not claimed by it.

## Review

Reviewer: Codex, five times and in two roles. A **design consultation** before drafting, which produced the ideal-wrap-phase
rule, the compiled-producer hold correction, the two-halves identity finding and the honest heard-cost figure; and an
**independent semantic review** of the drafted record, then three focused rereads of the repairs. The four review
rounds produced seven, four, four and four findings. Every one is repaired above, withdrawn into clause 8, or — in
the last round's substantive case — reclassified from a claimed reconciliation with ADR-0047 clause 7 into an
explicit amendment of it. The fourth round found no unfillable contract hole for a compiled-producer stream.
`NOW.md` carries the chronology and the two withdrawals.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
