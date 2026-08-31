# ADR-0023: Same-sample event ordering

| Field | Value |
|---|---|
| ID | ADR-0023 |
| Status | Accepted |
| Phase | 03 |
| Created | 2026-08-31 |
| Last reviewed | 2026-08-31 |
| Related | ADR-0032, ADR-0043, ADR-0046, ADR-0047, ADR-0050, ADR-0051, ADR-0053, `SPEC` sound-core render contract |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

**Delivered behaviour, and it binds every later producer.** Which of two events applied at one sample takes effect
second decides what is heard: for a parameter the second write is the value the quantum renders, and for a gate the
second edge decides whether a note sounds at all. The rule also reaches outside this phase — Phase 5's declarative
nodes, Phase 6's voice pool and Phase 7's scripted sources each add a producer, and each needs to know where its
events fall against the ones already there.

**The ingress slice is the immediately next dependent slice, and it cannot proceed safely without the answer.** It
adds the second producer that publishes into the arbiter. The renderer breaks a tie by the order the batch presents,
so charging live events anywhere in `render_one` *is* a same-sample decision; there is no neutral place to put the
call. Building it first and recording the choice afterwards is the failure mode
[ADR-0053](ADR-0053-simulated-ingress-provenance.md) already names for `TimeSource`: a public component shipped with
tests asserting a value stops the choice from being provisional whether or not a record says it is.

**One coupled policy is undecided, and it bounds what may be built on this record rather than this record itself.**
[ADR-0051](ADR-0051-locate-catch-up-gate-exception.md) clause 6 — ADR-0050 clause 8's third obligation — states that
a scalar gate reached by two producers has **no ownership law**: a gate carries neither producer attribution nor
depth, so ending one producer's note writes `ZERO` to a gate a second producer holds and cuts its note audibly. That
is a question about *who may address a target*, and it has the same answer whatever order this record picks; the
order is well defined either way. What it forbids is a plan in which a live and a compiled producer can reach one
gate, and therefore any fixture built on one.

**A narrower reading was tried and withdrawn.** A draft of this paragraph forbade only both producers *sounding*, on
the reasoning that the hazard is a gate write at render time — which would have let a fixture declare both producers
and refuse at the offer. ADR-0051 clause 6's rule is broader than that: it forbids more than one producer *emitting*
onto one gate, and narrowing an accepted record from inside this one would be an amendment this record does not
declare. An independent review found the pair. **The boundary is now a check**: a live ingress store refuses a plan
that also declares a compiled producer, so no such plan can emit from both. Declaring both remains representable and
harmless — several fixtures do, none builds a store — and one fixture builds one to assert the refusal.

The ingress slice's exit-gate obligation — the same `SampleTime` sequence through a simulated producer and through
the compiled path reaching the same offsets — presents one producer at a time, so it is reachable now. A fixture with
both producers in one plan is not, and waits for that law.

## Decision boundary

**In scope:** the relative order in which the renderer applies two events that carry the same render position inside
one quantum, where "render position" is the position after ADR-0043's preserving late clamp has moved a genuinely
late event.

**Non-goals**, each with its owner:

- *Which sample an event lands on.* ADR-0032 clause 15 rounds musical time once, ADR-0043 clamps a late event
  forward. A tie is the input to this record, never its output: nothing here moves an event to a different sample.
- *The effective point of play, seek, loop wrap and offline range start.* ADR-0050 clause 1 makes those four
  quantum-granular and adopts them **between** two renderer sub-calls, so the old stream's events and the new
  stream's are never presented in one batch. They are activations, not events, and what they contribute at a tie is
  the boundary release and the locate catch-up, which are events and are ordered below.
- *Whether two producers may address one target at all.* ADR-0051 clause 6, named above.
- *What a stop, count-in, metronome, preview or panic **does**.* Their **order** is decided here, because the exit
  gate asks for exactly that and a position is not an event kind. Each is assigned to a drain block by what it is:
  transport stop, count-in, metronome, preview and recording state are session and transport state, so they take
  block 1 with the boundary release and the locate catch-up; panic and sustain lift are charged to the live share by
  ADR-0046 clause 6, so they take block 4 and are ordered against other live edges by that queue rather than by any
  cross-producer rule. That assignment is a consequence of clause 1's partition rather than a new choice: an event's
  block follows the producer that emits it, and each of these has exactly one it can come from.

  **What is not decided here is their effect.** Whether a stop also suppresses the timeline's own events at its
  position is a question about what a stop *does*, and it belongs to the slice that builds one — **transport stop is
  not one of ADR-0050's four activations**, so this record inherits no answer for it. A first draft decided it and
  contradicted itself doing so. Nothing here invents an event kind, a payload or a producer; the order is stated so
  the slice that adds one has a position to build against rather than a choice to make.
- *Precedence between `Hardware` and `Arrival` ingress timestamps.* The host-profile specification routes that to
  ADR-0022 and Phase 9's measured uncertainty. Both arrive through one live store here, and this record orders that
  store by its own queue order, not by provenance.

## Evidence

**P1 — the presentation order is the tie-break, and it already is.** `PreparedRenderer` copies each admitted event
into scratch with an `arrival` index equal to its position in the presented slice, then sorts the live prefix by
`(position, arrival)` (`crates/synth_engine_v2/src/render/hot.rs`, `resolve_events`). The sort is total because
`arrival` is unique, so the applied order is a pure function of the batch's order. Nothing else in the renderer
consults a producer.

**P2 — the arbiter's store is charge-ordered.** `Publication::charge` writes each event to the next free index of one
flat preallocated store, and `SealedBatch::events()` presents that store
(`crates/synth_engine_v2/src/publish/hot.rs`). So the order producers are drained in is the order the renderer sees.

**P3 — one order is already delivered, for one pair.** `CompiledEventScheduler::render_one` charges ADR-0050
clause 5's boundary mass release and ADR-0051's locate catch-up to `ProducerClass::Session`, then the call's due
events to `ProducerClass::Compiled` (`crates/synth_engine_v2/src/schedule/hot.rs`). Session-before-compiled at a tie
is therefore not a new choice; it is the existing behaviour, and the specification already states its reason for the
catch-up alone — "the batch is the state already in force at the destination and the stream is what happens from
there".

**P4 — a producer's own note pair is split across two capacity classes.** ADR-0046 clause 6: a live note-on is
charged to the live share and "when an individual release becomes publishable it redeems one hold into the
guaranteed-release share". One producer, one FIFO of edges, two classes. A compiled pair is not split — clause 6
gives a compiled release the plan entitlement clause 4 established — so the split is a property of non-compiled
producers specifically.

**P5 — occurrence identity protects the registry, not the gate.** `SOUND-INV-017` resolves a release through its
occurrence, so one producer's release cannot remove another's entry from the live-note registry. It does **not**
follow that a cross-producer tie is harmless: ADR-0051 clause 6 establishes that both occurrences may write one
scalar gate and that ending either cuts both. An earlier draft of this record drew the stronger conclusion, and an
independent review refuted it.

**P6 — the renderer depends on end-before-start within one walk.** `resolve_events` resolves targets in application
order precisely so that "a walk in application order sees the first end before the next start on the same index"
(`crates/synth_engine_v2/src/render.rs`). An order that put a note's release after a later note-on on the same node
would defeat that walk.

**Uncertainty that could change the decision.** When this record was accepted only two producers existed — compiled
and the session machinery — so the position given to authored runtime expansion was argued from what that class *is*
rather than from an observed conflict. **That was the evidence at acceptance, not a standing fact**: the same stream
that accepted this record added `PerformanceIngress`, a live producer, and a later slice let a plan declare authored
and internal ones. A merge-gate review found the sentence still reading as present tense. What is unchanged is the
argument's basis — no authored *producer* exists even now, so nothing has yet exercised that limb of the order. If
Phase 7's scripted sources need a tie against live input decided the other way, that is the revisit condition below.

## Options

**Option 1 — arrival order alone (the status quo, undeclared).** Whatever sequence the arbiter's caller charges in
wins. It is deterministic today, because that sequence is fixed code. What it is not is a *contract*: adding the
ingress drain anywhere in `render_one` changes delivered behaviour with nothing recording that it did, and a reader
cannot answer "does a live knob turn beat the automation at the same sample?" from anything but the current line
order of one function. It also cannot be tested — a test would assert the code against itself.

**Option 2 — order by payload kind across producers.** Every release, then every note-on, then every parameter
write, regardless of who emitted it. It answers the end-before-start case directly. It also **reorders a producer
against its own document**: a compiled plan that writes a filter cutoff and then plays a note at one tick would be
applied in the other order, which is a change to what the timeline means. And it needs a total order over payload
kinds that every later phase adding a kind must extend.

**Option 3 — rank by ADR-0046 producer class.** A fixed total order over the five publishable classes, then
production order inside each. It reads naturally because the arbiter already holds the class, and an earlier draft of
this record selected it. **It is wrong, and P4 is why.** ADR-0046's classes are a *capacity* partition — who pays for
an event — not a causal one. A live note-on and its own note-off fall in different classes, so any class rank that
puts `Release` before `Live` applies a live note's release before the note-on it belongs to whenever the late clamp
or a zero-length note puts both at one position: the release is refused as an orphan, and the note-on that follows it
sounds with nothing left to end it. Reversing the two ranks does not repair it either — it inverts P6 for the pair
the ranks exist to protect. An independent review found this; the option is kept here because the reasoning that
selects a class rank is natural enough to be tried again.

**Option 4 — declared drain sequence, with the producer as the unit of order.** Same-sample order is the order the
single publication pass charged the events in, that sequence is declared rather than incidental, and each producer is
drained in **one contiguous block, in its own emission order**. A producer's edges keep their sequence whichever share each is
charged to, because the drain walks the producer's queue and not the class ledger.

## Decision

**Select option 4.** Same-sample order is the declared drain order of one publication pass. The unit of ordering is
the **producer**, not the capacity class.

### 1. The declared sequence

One publication pass drains, in this order:

| Position | Producer | Why here |
|---|---|---|
| 1 | Session and transport | The state already in force at this sample rather than work happening at it: ADR-0050 clause 5's boundary release ends the previous stream's notes and ADR-0051's catch-up restores every prepared target. P3 already delivers this, and the specification already gives its reason. A future session event is drained in this block; what such an event *does* is its own slice's, and only its position is fixed here |
| 2 | Compiled timeline and automation | The document's own timeline: the baseline position 1 prepares and the two below act on. Its releases are compiled too (P4), so a compiled pair is never split |
| 3 | Authored runtime expansion, in plan declaration order | Expansion derived from the compiled stream follows what it expands, so a source that rewrites a compiled note is applied after the note it rewrites. Several such sources are ordered by their position in the plan's declarations, which is already a total order and already the source of a producer's `ProducerId` |
| 4 | Live ingress, in queue order | A performer acts on top of the timeline: at a tie on one parameter the second write is what the quantum renders, and the performer's is the one that should be heard. ADR-0046 clause 6 charges panic and sustain lift here, so both end what is sounding at this sample including a compiled note that starts at it |

**Within one producer the order is that producer's own emission order**, and the drain preserves it by walking that
producer's storage once. This is the clause that option 3 could not deliver: a live note-on charged to the live share
and its matching release charged to the guaranteed-release share are one queue's two entries, drained in the order
they were offered, so the release can never precede the note-on it discharges.

**Where one position holds several producers, they are ordered by plan declaration order.** The table names
positions, and more than one producer can occupy one: ADR-0046 clause 1 admits several internal producers and sums
their declared per-quantum maxima, the host-profile specification admits a second live store once it has its own
registry row and admitting ground, and authored runtime is several sources by construction. A position alone is
therefore not a total order, and an earlier draft stated the declaration rule for authored runtime only — which left
two of the four positions with no rule at all, as an independent review found. Declaration order is already total and
already load-bearing: it is what makes a note producer's position its `ProducerId`, so no second numbering is
introduced to carry it.

Charging is unchanged. Which share an event spends is ADR-0046 clause 1's question and this record does not touch it;
a drain that walks one queue simply charges each entry to whichever class that entry belongs to, and the pass's
charges are therefore interleaved across classes by design.

### 2. Renderer-internal emissions apply last at their position

An internal emission is produced by the rendering of the quantum it takes effect in (ADR-0046 clause 2), so it cannot
precede its own cause: at one render position it applies after every external event there. Storage isolation alone
does not answer this — the arena decides where the event lives, not which write is heard — and an earlier draft left
it unanswered while claiming to bind every later producer. An independent review found that. No internal producer
exists, so the rule is stated and becomes testable with the first one. **Two internal producers are ordered against
each other by the same declaration rule as any other position**, which an earlier draft left unstated along with the
live one.

### 3. Three consequences, and each is reachable by a test the ingress slice can write

- **A live parameter write beats a compiled one at the same sample.** Live is drained last, so the performer's value
  is what the quantum renders. This is the whole reason live sits at position 4 rather than position 2.
- **A live note-on and its own release at one render position keep the order they were offered in**, although the
  first is charged to the live share and the second redeems a hold into the guaranteed-release share. This is the
  case option 3 could not deliver.
- **The locate catch-up still precedes the new stream's first event at one sample.** The existing rule becomes an
  instance of this one rather than an exception to it, and the behaviour is unchanged.

Two consequences follow for producers that do not exist yet, and they are stated as positions rather than as
behaviour: a transport stop is drained in the session block, before the same sample's compiled and live work; panic
and sustain lift are charged to the live share (ADR-0046 clause 6), so they are drained with live input and ordered
against other live edges by that queue rather than by any cross-producer rule.

### 4. How much of this is enforced, stated exactly

The order lives in the drain sequence of one function, so **no type refuses a drain written in the wrong order**.
What changes against option 1 is not enforcement but that the sequence is declared, so a test can assert it against
the record instead of against the code that produces it.

Two properties are checkable, and they need different fixtures. The **relative** order of two producers is one event
from each at one render position, compared with the table. **Contiguity is a second property, and one event per
producer cannot see it**: a drain emitting `A1`, `B1`, `A2` preserves A's own sequence and still breaks its block, so
that fixture needs two events from one producer with a second producer's event offered between them, asserting the
applied order is `A1`, `A2`, `B1`. An earlier draft claimed the first fixture covered both, and an independent review
refuted it.

**What no test can do is notice a producer nobody wrote a case for.** There is no exhaustive producer registry to
iterate over, so a later producer drained at the wrong position fails only once its own case exists. Saying otherwise
would claim a guarantee from the absence of a test.

Claiming more about enforcement would be false too. An earlier draft proposed refusing a charge whose class ranked
below one already charged in the pass, which is cheap and mechanical — and it is exactly the check that would reject
the correct interleaving section 1 requires.

## Consequences and risks

- **Accepted cost:** the arbiter's drain sequence becomes contractual. A later producer is drained at its declared
  position rather than where it is convenient, and `render_one`'s ordering of its drains is no longer an
  implementation detail. Nothing is added to an event and nothing is sorted twice.
- **Safety/correctness control:** the five falsifiers below, each a named test. The producer-granular unit is what
  makes the second one expressible at all.
- **Revisit condition:** a payload kind whose correctness needs to cross a producer boundary at a tie, which Phase
  5's declarative nodes or Phase 6's voice pool could produce; or an observed case where authored expansion must
  precede live input. Moving any position changes delivered behaviour and needs a successor record, not an edit.

### Falsifiers

1. **Partition dependence.** Two events from different producers at one render position apply in a different order
   when the same stream is rendered as `1 x 4096` and as `64 x 64`, or depending on which call accepted the live one.
2. **A producer reordered against itself across two shares.** A live note-on and its matching release at one render
   position apply release-first, leaving an orphan and a note nothing ends.
3. **Start before end.** A live note-off at the sample a live note-on reuses the same node leaves the new note
   silent, because the release applied second.
4. **A producer reordered against its own document.** A parameter write and a note-on emitted by one producer at one
   position apply in an order that producer did not emit.
5. **A producer's block broken.** Two events from one producer at one position, with another producer's event offered
   between them, apply interleaved rather than as one contiguous block.

## Specification update

Acceptance creates **`SOUND-INV-020` — same-sample application order** in
[the render contract](../specs/spec-sound-core-render-contract.md): two events sharing a render position apply in the
publication pass's declared drain order — session and transport, compiled, authored runtime in plan declaration
order, live ingress in queue order — with each producer drained in one contiguous block in its own emission order, and a
renderer-internal emission applying after every external event at that position. The capacity class an event is
charged to does not order it.

`SOUND-INV-018`'s catch-up sentence is unchanged and becomes an instance of the general rule rather than an
exception to it. No host-profile invariant changes: the order spends no capacity and moves no event between quanta.

## Review

Reviewer: `codex exec` (gpt-5.6-sol), 2026-08-31, two passes. Five blocking findings on the first draft, all
repaired: the class rank splitting a live producer's own note pair (option 3, now recorded as refuted); P4's
over-strong "cross-class note ties are harmless", which hid ADR-0051 clause 6's open ownership law; transport stop
wrongly listed as a non-goal when ADR-0050's four activations do not include it; "production order" not being total
across two producers of one class; and `Internal` left unordered while the record claimed to bind every later
producer. A focused reread of those repairs confirmed them and found two more, repaired here: the declaration rule
was stated for authored runtime alone, leaving the internal and live positions with no rule where several producers
occupy one; and the claimed test coverage was overstated, since one event per producer cannot see a broken block and
no test notices a producer nobody wrote a case for.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
