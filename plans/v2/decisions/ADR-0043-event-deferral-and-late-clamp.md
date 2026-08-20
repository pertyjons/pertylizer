# ADR-0043: Event Deferral, and What a Timestamp Survives

| Field | Value |
|---|---|
| ID | ADR-0043 |
| Status | Proposed |
| Phase | 3 |
| Created | 2026-08-20 |
| Last reviewed | 2026-08-20 |
| Related | ADR-0001 clauses 12, 14 and 16; ADR-0021 part 2; ADR-0023; ADR-0032 clauses 17, 18, 19, 22 and 23; `HOST-INV-021`; REV-P00A; REV-P02 |
| Supersedes | — while `Proposed`. Under **Option B or D**: ADR-0001 clauses 12, 14 and 16, because a preserved stamp no longer identifies the quantum the event renders in. Under **Option C**: clause 14 only — rewriting the stamp keeps clauses 12 and 16 arithmetically true, but the sample the edge lands on is no longer the one the *producer* declared, which is what clause 14 promises; C additionally needs an ADR-0032 clause 18 successor. Under **Option A**: nothing if Phase 3's capacities make its runtime overrun unreachable. If the overrun stays reachable, A supersedes **clause 16** — a rejected callback can hold a genuinely late event that is then never clamped, which clause 16 requires — and needs an ADR-0021 successor besides. A paired with a rewriting clamp needs the clause 18 successor either way |
| Superseded by | — |

This record meets `PROCESS.md`'s durable-decision test on two counts: it defines a real-time ownership boundary, and it
binds several later phases. It fixes no value. It declares no decision *class* — that vocabulary belongs to the former
workflow and [`ADR.md`](../ADR.md#decision-classes) keeps it for historical records only.

> **Not decided. This record presents the options and a recommendation; the choice is the maintainer's.** The
> [working agreement](../WORKING-AGREEMENT.md) requires an independent reader and a reread of every semantic change,
> and the author of these frames should not also be the reader who reviews them. Holding the decision open is this
> record's own choice rather than a repository rule — an earlier revision asserted a same-session prohibition that does
> not exist in the process documents. The *Decision* section states which option is recommended and what would make
> that recommendation wrong; it does not select on the maintainer's behalf.

## Durable boundary

Two boundaries, and either alone would require a record.

**A real-time ownership boundary.** Whether the renderer may move an event to a later quantum than its timestamp names
is a property of the engine's timing contract, not an implementation detail. Every later phase — the scheduler, the
voice pool, recording, and the offline/live equivalence gate — reasons over the answer.

**A cross-phase boundary that is currently self-contradictory.** [ADR-0001](ADR-0001-internal-render-quantum.md)
clauses 12 and 14 and [`HOST-INV-021`](../specs/spec-host-profile-and-render-limits.md) cannot both be implemented as
written. The specification says so about itself and runs an interim rule it states it has no authority to make. That is
the narrowest possible reason for an ADR: an accepted decision and a current specification disagree, and only a decision
record can settle which one moves.

## Decision boundary

### What this record decides

1. **Whether a quantum may defer an event at all**, given ADR-0001 clause 12's "an event is assigned to the quantum
   containing its timestamp" and clause 14's "sample-positioned effects occur at their declared sample within the
   quantum".
2. **If it may, the shape of the departure** — whether it is an exception granted to clause 12 or a restatement of
   clause 12 that needs none, and whether the displacement is `+Q` or a boundary.
3. **Whether a deferred event's stamp is preserved or rewritten.**
4. **Whether clause 16's late *clamp* preserves or rewrites the stamp.** The specification records this as ADR-0001's
   question rather than its own, and leaves it unresolved.
5. **When clause 16's late condition is evaluated** — once, when an event first becomes due, or on every quantum it is
   considered for.

### What this record does not decide

Named explicitly, because a record that quietly settled these would be deciding overload policy behind a timing
question:

- the **ingress capacities** and the **deferred store's bound**, which `HOST-INV-021` says are not derivable from the
  current field set — Phase 3. **This holds for Option A too**, which two revisions of this record denied: A was said
  to force a source partition because an aggregate overflow would otherwise be unhandled. It would not — the current
  contract already rejects any over-full quantum before mutation, whatever mix of sources filled it — so A needs no
  capacity architecture decided here either;
- the **exhaustion policy** when the deferred store is itself full, and any **starvation** guarantee for an event that
  defers repeatedly — Phase 3, and this record's answer must not be read as promising either;
- **same-sample ordering** — `ADR-0023`, which is `Proposed` and has no record yet;
- the **hardware clock mapping** and whether a `Hardware`-stamped tier should outrank an `Arrival`-stamped one —
  [ADR-0022](ADR-0022-hardware-time-mapping.md);
- the **admission order** among events due in one quantum, which `HOST-INV-021` already states and this record does not
  reopen.

## Evidence

- **The contradiction is stated by the specification against itself**, in `HOST-INV-021`: deferral "departs from both"
  clause 12 and clause 14, clause 16 is "the only exception ADR-0001 grants" and is granted for an already-rendered
  quantum, "so the precedent does not cover deferral". Three earlier revisions of that invariant reasoned wrongly about
  the same point and are recorded there.
- **Clause 16 cannot be reused as the mechanism.** Its rule — clamp to the first not-yet-rendered quantum boundary — is
  circular for a capacity shortfall, because the quantum that could not admit the event has not rendered either, so
  applying it literally puts the event back where it did not fit.
- **`HOST-INV-021` names three places a rewritten stamp is observable, and this record accepts one of them
  outright.** On **diagnostics**, that invariant argues the engine loses *how far* an event was displaced — but the
  loss is not inherent, as this record establishes for both the deferral and the clamp: the displacement is computable
  at the moment of the rewrite and can be carried as metadata. The honest form is that rewriting costs extra state to
  keep what preserving keeps for free. On **recording**, it would quantize a played performance
  forward by however many quanta the engine was overloaded, which [ADR-0021](ADR-0021-host-profile-and-admission-policy.md)
  part 2 forbids outright as an authored-data change **if a recorded note's placement derives from the stamp** — and
  that premise is *not* settled: the host-profile specification leaves what a recorded event is to `ADR-0024`, and
  ADR-0032 clause 7 says only that placement resolves to musical time. So the recording argument is a **conditional**
  reason against rewriting the stamp, not a settled one, and this record does not lean on it. What is settled is
  ADR-0032 clause 23's stated harm — that moving an event off its declared sample "would make the sample position a
  lie" — which does not care what caused the move.
- **There are two different drops, and only one of them is closed.** Dropping an event the renderer has already
  admitted, or trimming authored note expansion to fit, is forbidden: `HOST-INV-019`'s lossy class "may never bound
  canonical project data, authored topology, render input, automation, routing, sample mapping, or polyphony", and
  ADR-0021 part 2 forbids silently changing authored note expansion. But ADR-0021 also states that **runtime dropping
  is reserved for live bounded queues** and is counted and reported — so an ingress queue *may* drop an excess live
  event before the renderer ever sees it. The option survey below keeps these apart; an earlier revision of this record
  collapsed them and made Option A look worse than it is.
- **Compiled events can overrun a quantum on their own.** `max_note_expansion_per_tick` makes this reachable whenever a
  script-driven note graph's expansion is not statically knowable, so this is not only a live-ingress question and
  cannot be answered by admission alone.
- **No measurement is required.** Unlike ADR-0022, every premise here is in accepted text and current source. Nothing
  about this record waits on a simulated host, a hardware clock, or cross-platform access.
- **The renderer already separates the stamp from the render position, and the clamp already preserves the stamp.**
  Read at `e9590577`: `render/hot.rs:127-131` computes a `position` that is `self.clock` for a late event and
  `envelope.time()` otherwise, `:134` derives the quantum from **that position** rather than from the stamp, and the
  comment at `:126` says "The stamp itself is untouched." So *preserve* is the implemented status quo for the **clamp**
  rather than a proposal. **It says nothing about Option C**, which went back and forth twice before settling there: a
  fifth read objected that C was being charged for the clamp, a ninth read excluded C-with-preserving-clamp and so
  charged it again, and a fourteenth showed that exclusion was wrong because the clamp and the deferral fire in
  sequence. C admits either clamp, so C alone entails no change to shipped behaviour; only a *rewriting clamp* does,
  under whichever option it is paired with.
- **One site reads the stamp instead of the position:** `offline.rs:178` and `:182` window the event slice by
  `event.envelope().time().quantum_index()` while the renderer admits by position. It is not a live defect — its
  premise is that the offline path, walking a sorted list with a monotone clock, cannot present a late event — but it
  is where the literal reading of clause 12 is built in.
- **What that sweep does *not* establish, stated because an earlier revision of this record claimed it did.** The
  renderer's `position` differs from the stamp **only** for clause 16's late clamp, which ADR-0001 already authorises
  explicitly; capacity deferral does not exist in the code at all. Option B's named exception and Option D's
  render-position formulation would produce **the same implementation of what exists today**, so this evidence cannot
  discriminate between them and does not show that the code has already outgrown clause 12's wording. It bears on two
  other things instead:
  that *preserve* is the status quo for the clamp sub-question, and that neither B nor D carries a migration cost
  beyond the one `offline.rs` site.

**Uncertainty that could change the decision.** Nobody has measured how often a quantum actually overruns, because no
V2 ingress exists to overrun. If Phase 3's capacities make overrun unreachable in practice, options A and B converge in
behaviour and differ only in what happens at a boundary nobody reaches — which is an argument about cost of being
wrong, not about frequency.

## Options

### The status quo

Clause 12 and clause 14 stand, `HOST-INV-021` proposes an exception to them, and the specification marks itself
non-normative for that invariant. This is not a viable end state: it is the defect, and Phase 3 cannot begin
implementation against it.

### Option A — No deferral. Capacity overrun is a fault

Clauses 12 and 14 are left unamended and no exception is granted. Two mechanisms keep `max_events_per_quantum`, and
the boundary between them is what makes the option coherent:

1. **Ahead of renderer admission**, a live bounded queue may drop an excess external event, counted and reported, under
   ADR-0021's runtime-dropping rule. This absorbs live bursts and never reaches the renderer.
2. **At the renderer**, any quantum whose admitted set still exceeds the capacity is a **fault** — whatever mix of
   sources filled it. A compiled stream overrunning alone (`max_note_expansion_per_tick` makes that reachable) and a
   compiled-plus-ingress aggregate that neither bound catches are **the same fault**, not two contracts. An earlier
   revision of this record defined only the compiled-only case here and then discussed the aggregate case separately,
   which left Phase 3 two incompatible readings.

- **For.** The accepted timing guarantee survives intact and literally. There is one fewer moving part in the hot path:
  no deferred store and no per-quantum re-evaluation. **It does not remove the second counter**, which an earlier
  revision claimed: `max_scheduled_events_in_flight`'s release window survives every option and shares the
  capacity-deferral counter with per-quantum deferral, so A keeps the counter and loses only the store. Live overload is absorbed at the boundary where the
  repository already permits loss, rather than by inventing a timing exception for it.
- **The aggregate case is the one that decides whether this option is coherent, and it widens what selecting A
  decides.** Ingress and compiled events can each be under their own bound and exceed `max_events_per_quantum`
  **together**: the queue is not full, so the permitted queue drop never fires, and the compiled stream did not overrun
  on its own. Dropping ingress to make room would pick the compiled-before-ingress policy this record leaves undecided.

  **The aggregate case already has a legal handling, and an earlier revision of this record missed it.** The current
  contract says that if any one quantum exceeds `max_events_per_quantum`, "the call is rejected before renderer state
  or output is mutated" — it does not distinguish sources. Option A can keep that rule, and no admission order and no
  disjoint budget has to be chosen for A to be coherent. Two revisions of this record called the partition *mandatory*
  for A, which overstated its cost.

  **But keeping the rule is not the same as keeping it unchanged, and A has to say which fault it is.** Today that
  rejection is a *caller-contract* violation: the host asked for something the plan was admitted against. Under A it
  becomes a **reachable runtime overload** on the audio callback, and "return without mutating output or state" is not
  a defined outcome there — a real-time callback may not be left partly unwritten. ADR-0021's terminal-fault precedent
  is fully specified for exactly this reason: silence on this and every subsequent callback, both carries invalidated,
  an atomic `needs_reprepare`, nothing allocated, counters in the diagnostics report.

  **Two consequences follow, and together they remove Option A's remaining cheapness.** First, an implementer needs the
  output, state and epoch behaviour named — terminal fault or some other fully defined recovery — so **selecting A does
  not close Phase 3's ADR-0001 prerequisite on its own either**; it closes it only once that recovery is chosen.
  Second — and this record asserted the opposite one read ago — **A does not need an ADR-0021 successor.** The
  record briefly argued that a compiled or mixed-source overrun is exactly ADR-0021's "a prepared plan may not exceed a
  `HostProfile` limit at runtime", after checking ADR-0021's wording but not its reach. The
  current specification answers that directly: live ingress "is unbounded in time by definition" and a script-driven
  expansion "is data-dependent", and ADR-0021 part 1's prohibition "binds the *plan*, and neither of these is part of
  it". So the prohibition does not cover A's overrun, the ADR-0021 successor is withdrawn, and what A actually owes on
  its reachable branch is the **recovery definition** above and clause 16's supersession — no more.

  Disjoint budgeting — a reserved ingress allowance plus a compiled allowance that cannot jointly exceed the bound, the
  shape the specification floats under ADR-0032 clause 27 — remains available as a way to make the fault *unreachable*
  rather than merely defined. That is a Phase 3 capacity choice, not something selecting A forces.
- **Against.** A dropped live note-on is a note the performer played and did not hear, and the drop is permitted rather
  than good. The compiled-overrun case still has no non-fault answer, so Option A does not remove the hard case — it
  moves it to a rarer one. And "sized so overrun is unreachable" for the compiled side needs a bound on script-driven
  expansion that nobody has computed.
- **The simplicity claim is conditional, and the condition is the clamp.** An event's render position is a pure
  function of its stamp **only if Option A is paired with a stamp-rewriting clamp**. Paired with the stamp-preserving
  clamp offered below — which is what the code does today — a late event still carries an immutable stamp and a
  separate clamped position, so the two-quantity cost is not avoided and this option's advantage over B and D narrows
  to the absence of the deferred store.
- **What would make it wrong.** A burst that **fits the ingress queue** but makes the renderer's due set exceed
  `max_events_per_quantum` — for instance a chord arriving inside one quantum alongside a busy compiled bar. That is
  the only comparison that separates the options: an overflow of the *queue* is dropped before the renderer under every
  option including B and D, which cannot defer an event they never received, so a queue-overflow burst distinguishes
  nothing. An earlier revision of this record used exactly that non-comparison and drew a conclusion from it. In the
  case that does distinguish, the queue *fits*, so ADR-0021's queue-drop permission and its counter never come into
  play. What the current contract does instead is **reject the whole render call before renderer state or output is
  mutated** — a callback-wide fault, not one counted missing note. So Option A's cost in the distinguishing case is a
  dropout, where B and D delay the offending event by `Q` and report the delay.

  This record has now mis-stated that cost in both directions: first as a silent loss, then — correcting too far — as
  a counted missing note. The *Review* section had flagged that an overcorrection was possible here, and it happened.

### Option B — A narrow exception to clause 12, stamp preserved

Deferral is permitted as a named exception. The event's stamp is immutable; only its **render position** advances, by
exactly `Q`, so it renders in the following quantum at the same offset. It is counted under its own capacity-deferral
counter. Clause 16's condition is evaluated **once**, when the event first becomes due.

**Deferral itself adds no late count** — clause 16's condition, a timestamp in an already-rendered quantum, is not what
happened. That is a statement about the *deferral*, not about the event: an event that was **already late** was clamped
and counted late before capacity could reject it, and if the destination quantum then defers it, it raises **both**
counters. Writing this as "a deferred event does not fire the late counter" would tell an implementer to suppress a
genuine late diagnostic, and the both-late-and-deferred case is exactly the one `HOST-INV-021` requires to compose.

This is the rule `HOST-INV-021` already describes. **Adopting it means ratifying an interim rule the specification wrote
while stating it lacked the authority**, which is a reason to scrutinise it harder than the others, not a reason to
prefer it.

- **For.** It preserves the observables the evidence names: displacement is reportable and the sample offset survives.
  It would also keep a recorded take from being quantized forward **if** a take's placement derives from the stamp —
  the same premise `ADR-0024` has not settled, so it is not counted for B any more than it is counted against C.
  `+Q` rather than a boundary keeps the spacing between two deferred events rather than collapsing every deferred
  event in a quantum onto offset 0, which moving to the *boundary* would do. It composes: a second deferral is
  another `+Q`.

  **It does not avoid ties, and an earlier revision of this record wrongly claimed it did.** An event deferred from
  `kQ + o` lands on `(k+1)Q + o`, where an event may already be natively due — same render position, different stamps.
  That tie must be ordered, and `ADR-0023` is where it belongs; the inherited admission rule already delegates equal
  render-position ties there. What `+Q` avoids is *manufacturing* a pile-up at one offset, not collisions as such.

  **`+Q` can also reverse causal order, and this record does not solve it.** At `Q` = 64, a note-on stamped at
  sample 63 defers to 127, while its own note-off stamped at 65 is natively due in the next quantum and renders at 65.
  The note-off therefore renders **before** the note-on, and the voice is left sounding. `ADR-0023` cannot repair this:
  the two render positions differ, so there is no tie for a same-sample rule to break. Nothing in `+Q`, in the
  admission order, or in the deferral counter prevents it.

  **This is a correctness hole that Options B, C and D all inherit**, since all three defer by `+Q`. Fixing it needs a
  causal-order policy, and the obvious candidate does not work: **ordering the admitted set by stamp changes only which
  event is inspected first, not where either renders**, so sample 127 still comes after 65 and the voice still hangs.
  Any repair has to move the *successor's render position* too — deferring an event's causal successors along with it,
  or some rule the maintainer prefers — and choosing one is scheduler design that this record's boundary excludes. **So it is named as a blocker on B, C and D rather than papered over**, and it is
  the strongest argument Option A has: no deferral, no reordering, and therefore **not this inversion**. An earlier
  revision of this record said only that repeated `+Q` "composes", which was true of the arithmetic and false of the
  music.

  **Option A is not a general defence against stuck notes**, and saying so would overstate it. ADR-0021 permits a live
  bounded queue to drop an excess external event with no exception for note-offs, so A can drop a note-off whose
  note-on already rendered and leave the voice sounding just the same. What A avoids is specifically the
  *deferral-induced* reordering; the queue-drop hazard is common to every option here.
- **Against.** It leaves clause 12's first sentence true only "ordinarily", which is exactly the reading `HOST-INV-021`
  says is "not ADR-0001's to give". An exception invites the next one; the record has to say why this is the last.
- **What would make it wrong.** A consumer that must derive a quantum from a stamp without also being handed the
  deferral count. If such a consumer exists, the stamp and the render position have drifted into two sources of truth.

### Option C — A narrow exception, stamp rewritten

As Option B, but the deferred event's stamp is rewritten to its new render position, so clause 12's first sentence
stays literally true.

- **For.** Clause 12 needs no exception at its first sentence and no restatement, and **clause 16 stays true as well**,
  because the rewritten stamp always identifies the quantum the event renders in. **Clause 14 does not**: the effect
  still lands `Q` frames after the sample the producer declared, and rewriting the stamp changes what the record *says*
  about that sample rather than where the effect happens. C supersedes clause 14 like B and D do — it buys back the
  arithmetic clauses, not the promise to the author. **Paired with a rewriting clamp** it is the one option reaching a
  single quantity everywhere; paired with a preserving clamp it keeps two for late events, as the table shows.
  The clamp stays independently selectable here as it does everywhere else.
- **Against.** It breaks one observable outright and two conditionally. On diagnostics, the loss is **not inherent**:
  a renderer could accumulate `new_position - old_position` as preallocated deferred-store metadata at each rewrite and
  still report displacement without keeping a second absolute position. This record leaves the store's shape to
  Phase 3, so it cannot claim displacement is unreportable under C — only that C is the one option where reporting it
  costs extra state that the others get for free. And **the provenance tag becomes false**: ADR-0032 clause 17 gives the
  envelope `{epoch, time, source}`, and clause 18 makes `source` a promise about where `time` came from — `Compiled`
  says the timestamp is "exact by construction". Rewriting `time` to a displaced position while `source` still reads
  `Compiled` asserts an exactness the value no longer has, and `Hardware` and `Arrival` stop describing where their
  timestamp originated.
  **The recording objection is conditional and is not counted against C here**: a recorded performance would be
  silently quantized forward under overload *if* a take's placement derives from the stamp, and `ADR-0024` has not
  decided that it does.
- **What would make it wrong.** It is already wrong against accepted text unless **ADR-0032 clause 18 is superseded**
  — by a successor, never by amending that accepted record, which `decisions/README.md` forbids — either widening what
  a provenance tag claims, or adding a displacement marker so a rewritten stamp still announces itself as one. Naming ADR-0021 part 2 here instead, as an earlier revision did, pointed at a record whose
  recording basis this one has since made conditional, and clause 17 fixes the envelope's *shape* rather than forbidding
  a rewrite. Clause 18 is the contract that actually conflicts. C is surveyed because the option space should not be
  presented as a choice of two.

### Option D — Restate clause 12 over render position, so no exception is needed

Clause 12's first sentence is restated: an event is assigned to the quantum containing its **render position**, where
`render_position = clamped_position + Q x deferrals` and `clamped_position` is the stamp for an on-time event and
clause 16's first not-yet-rendered boundary for a late one. Clause 14's "declared sample within the quantum" becomes the
offset within the quantum that renders it, which `+Q` preserves exactly. Clause 12's second sentence — "no event is ever
applied to samples already produced" — is untouched and does all the safety work.

**Clause 16's condition is evaluated once, when the event first becomes due** — the same answer Option B gives, and it
has to be stated rather than inherited. A deferred event's preserved stamp points into an already-rendered quantum on
every later reconsideration, so evaluating the condition each time would count one event late repeatedly and could
re-clamp it from a moving base. Once, at first due, is the only reading under which the late count means "how many
events arrived after their quantum had rendered".

- **For.** It removes the exception rather than granting one, so there is no "ordinarily" left in an accepted clause and
  no precedent for the next exception. The stamp stays immutable, so every observable Option B protects is protected
  here. It is also what the code would compute under Option B anyway, stated as the rule instead of as a departure from
  one.
- **Against.** It changes what an accepted clause *means* rather than carving an exception out of it, which is the
  larger move and reaches every consumer of clause 12 — including ones written against the old reading in Phases 1
  and 2.
- **What would make it wrong.** A Phase 1 or Phase 2 site that relies on "the quantum containing its timestamp"
  literally, where render position and stamp are assumed identical. **This is not a cost that separates D from B**,
  and an earlier revision claimed it was: B's deferral makes stamp and render position diverge exactly as D's
  restatement does, so a consumer that reads the stamp is equally wrong under either. The sweep was run — see
  *Evidence* — and its result is neutral between them.

### The control-rate response, as a second sub-question

Clause 14 splits an event's effect: the sample-positioned part lands on its declared sample, while the **control-rate**
part begins at the next quantum boundary under clause 13's causality rule. Deferral makes "next boundary" ambiguous,
and the two readings are observably different. For a control-rate event stamped at `kQ + o` and deferred to
`(k+1)Q + o`:

**The boundary case, and the hole an earlier revision of this record opened in it.** When a position falls **exactly
on a boundary** — `o = 0`, or a late event clamped to one — clause 13 permits the response in that same quantum,
because the position is *at* the quantum's first sample rather than after it, and neither reading may add a `Q` there.
**But that permission is keyed on the render position, never on the stamp.** A deferred event stamped at `kQ` was
deferred precisely because quantum `k` could not admit it; letting its control response land at `kQ` would apply part
of the event inside the quantum that rejected it and bypass `max_events_per_quantum` outright — no second deferral
needed. So: the same-quantum permission applies when the **render position** is on a boundary, and a deferred event's
control response may never land in a quantum that refused it.

- **From the stamp**, the next boundary after `kQ + o` is `(k+1)Q`, so the control response is unaffected by the
  deferral. Clause 13 permits it: the stamp is at or before that quantum's first sample. **This reading has an
  unresolved problem.** If the event also exceeds capacity in quantum `k+1`, applying its response at `(k+1)Q` anyway
  bypasses `max_events_per_quantum`, while deferring it again misses the boundary the stamp promised. Repeated deferral
  is explicit in the model, so this alternative needs either an admission guarantee that reserves room for it or a rule
  that permits and counts the bypass. Neither exists, and this record does not invent one.
- **From the render position**, the next boundary after the position is derived the same way at every deferral, so
  repeated deferral composes without a special case.

Neither is forced by accepted text, so this record has to choose rather than leave Phase 3 to guess — an implementer
cannot write the scheduler without an answer. **The choice is independent of the deferral option.** Restating clause 12
over render position does not supersede clause 13, which names the timestamp explicitly, so Option D can coherently use
the stamp just as B can use the render position; an earlier revision claimed the stamp reading contradicted D's premise
and it does not.

**Recommended: from the render position**, for the same reason as the recommendation overall — one quantity, no
carve-out. The cost is that a control change deferred `d` times is delayed up to `(d + 1)Q - 1` frames rather than
`Q - 1`, and **`d` is unbounded in this record** because repeated deferral is part of the model and the starvation
policy that would bound it is Phase 3's. That is the largest cost this answer carries, and it is stated as unbounded
rather than as a comfortable single-deferral figure.

### The clamp, as a sub-question on the same axis

The clamp is selected **independently of the deferral question**, so every option needs an answer to it — including
C, where rewriting a deferred event's stamp says nothing about a late event that is clamped and never deferred. Clause
16's clamp asks the same thing about the stamp, and consistency across the two is the point:

- **Preserve.** The late event keeps its stamp and gains a clamped render position, so lateness magnitude is derivable
  from the envelope **for as long as the stamp survives**. Under Options A, B and D that is indefinitely. **Under
  Option C it is until the first deferral**, which rewrites the stamp — after which the lateness is recoverable only
  from the metadata described below, which is a different contract and worth naming rather than gliding over.
  Preserving is what the shipped renderer does today.
- **Rewrite.** The late event's stamp becomes the boundary it was clamped to. Symmetric with Option C. **It does not
  inherently lose the lateness magnitude**, and an earlier revision of this record said it did: the renderer holds both
  `envelope.time()` and `self.clock` at the moment it decides lateness (`render/hot.rs:127`), so it can retain the
  difference as diagnostic metadata before rewriting — the same escape this record already grants Option C for
  deferral displacement. The real cost is symmetrical with C's: extra state to keep what preserving gets for free.

  Neither today's counter settles this either way. `DiagnosticsReport::late_events` is a `u64` count
  (`diagnostics.rs:492`) and clause 16 requires only a count, so **no magnitude is exposed under any pairing today** —
  an earlier revision attributed magnitude visibility to that counter, which it has never had.

**C works with either clamp, and this record has now been wrong about that in both directions.** A ninth read
excluded C with a *preserving* clamp, reasoning that the both-late-and-deferred case needs the clamp to preserve a
stamp that C then rewrites. A fourteenth read showed the exclusion was wrong, and the reason is worth keeping: **the
two mechanisms are sequential, not simultaneous.** The clamp fires first and only needs the lateness delta captured at
that moment — the same preallocated metadata this record already permits for rewritten displacement — after which C is
free to rewrite the stamp. Nothing requires the original position to survive as a *field*; it only has to be read
before it is overwritten. No third envelope field is needed, and none is proposed.

The symmetry is the check: B and D already permit a rewriting clamp followed by preserving deferral, which is the same
composition in the other order. Excluding only the reverse ordering was inconsistent.

An event can be both late and deferred, and `HOST-INV-021` already requires the render position to be derived from the
**post-clamp** position for exactly that case — otherwise a both-late-and-deferred event can land back in a quantum that
has already rendered. Whatever is chosen must keep that composition well defined.

## Decision

**Open. The recommendation is Option D with a preserving clamp, *conditional* on the maintainer being willing to
decide a causal-order policy alongside it — and if that willingness is not there, Option A is the better answer.**

The condition is new as of the eighteenth read and it is not a formality. `+Q` can reverse cause and effect: a note-on
at sample 63 defers to 127 while its note-off at 65 renders first, leaving a voice sounding. Every deferral option
carries it, `ADR-0023` cannot repair it because the positions differ, and no policy in this record prevents it. So
**B, C and D each ship with a named correctness hole** that Phase 3 must close before any of them is implementable,
and Option A — which never reorders anything — is the only option that does not.

Option D reaches the same runtime behaviour as Option B while leaving no weakened clause behind. The distinction is not
cosmetic: under Option B, clause 12's first sentence becomes a statement that is true except when it is not, and the
next capacity question arrives with a precedent for carving another exception. Under Option D there is one rule, the
safety sentence that actually prevents harm is untouched, and `+Q` is derivable from the rule rather than granted by it.

**The recommendation rests on one argument only, and review has already removed the other.** An earlier revision of
this record argued that the current renderer's stamp/position split shows the code has outgrown clause 12's wording.
**That argument is withdrawn**: the split exists only for clause 16's clamp, which ADR-0001 authorises outright, and B
and D compile to the same thing over the code that exists. So the recommendation rests on a governance judgement about
which text is easier to reason from next time — a real argument, but a weaker one than this record first made, and a
maintainer who weighs it differently is not disagreeing with any fact.

**No observable difference between B and D has been found.** The sixth read suggested the control-rate sub-question
might be one; the eighth established it is not, because clause 13 names the timestamp explicitly and neither framing
supersedes it — so each option may select either answer. Both attempts to find behavioural daylight between B and D
have failed, and the record says so rather than leaving the earlier suggestion standing. The choice is a governance
judgement about which text is easier to reason from, and nothing else.

**What would make this recommendation wrong.**

1. **A Phase 1 or Phase 2 consumer that reads clause 12 literally.** Swept at `e9590577`, and the result is **neutral
   between B and D**: the one stamp-reading site is `offline.rs:178`/`:182`, whose premise is that the offline path
   cannot present a late event. Neither option carries a migration cost beyond that site. This risk is discharged for
   both, and it discriminates between neither.
2. **If Phase 3's capacities make overrun unreachable in practice**, then Option A is the cheapest correct answer and
   both B and D are machinery for a case that does not occur. This cannot be settled today, which is itself an argument
   for not choosing A now.
3. **If the causal-order policy turns out to be expensive or contentious**, the recommendation inverts. Deferral's
   whole appeal is that it delays an event instead of losing it; an option that delays it *into the wrong order* has
   not delivered that, and a policy that defers an event's causal successors along with it starts to look like the
   deferred store growing a dependency graph. A's dropout is a worse outcome per occurrence and a much smaller
   contract.
4. **If the maintainer judges that restating an accepted clause is a heavier governance act than granting a bounded
   exception**, that is a legitimate reading of the same facts and selects Option B. The two differ in what they leave
   for the next decision to inherit. They need not differ in what the renderer does — provided B answers the
   control-rate sub-question the same way, which it is free to do and which this record's recommendation assumes.

Option C is recommended against on accepted text rather than on preference: it falsifies **ADR-0032 clause 18's
provenance promise** — clause 17 fixes the envelope's shape and does not make its timestamp immutable — and it discards
the exactness a compiled position carries by construction. It does **not** lose displacement reporting outright — a
deferred-store field could carry it — but it is the only option that has to pay extra state for what the others get
free. Note that its recording argument is
**conditional** on `ADR-0024`, which has not decided what a recorded event is, so this record does not count that
against C. Separately, the *clamp* preserves the stamp in shipped code today, so choosing a rewriting clamp means
changing working code **under any of the four options**, not only under A and C — the clamp is selected independently
of the deferral question, and `render/hot.rs` does not care which option it is paired with. It does **not** mean reaching a contract that reports less: an
earlier revision said so, which is the withdrawn diagnostic-loss premise wearing different words. What a rewriting
clamp costs is the extra state needed to keep the lateness delta, not the delta itself.

## Consequences and risks

- **Accepted cost — added latency, and it is not bounded here.** Under the recommended answer to the control-rate
  sub-question, a control response deferred `d` times is delayed by up to `(d + 1)Q - 1` frames **measured from its
  clamped position**, against `Q - 1` for an undeferred one. **A late event is a separate case**, because its clamped
  position is always a quantum boundary and the boundary rule above lets the control response run in that same quantum:
  after `d` deferrals it lands at `clamped_position + dQ`, so the delay measured from the *producer's* stamp — which is
  what a performer experiences — is `(clamped_position - stamp) + dQ`. An earlier revision wrote `(d + 1)Q - o` here,
  which added a quantum the boundary rule removes and an offset term a clamped event does not have. The clamp term
  itself is bounded by nothing in this record. **`d` has no bound in this record**, because repeated deferral is explicit in the model — a second
  deferral is another `+Q` — and the starvation policy that would bound it is named under *what this record does not
  decide*. An earlier revision stated `2Q - 1`, which is only the single-deferral case and contradicted the model.
  Bounding `d` is Phase 3's, and until it does, the honest statement of this cost is that it is unbounded under
  sustained overload.
- **Accepted cost — two quantities where a naive reading expects one**, an immutable stamp and a derived render
  position, so every consumer that asks "which quantum" must ask it of the render position. **The cost follows the
  *pairing*, not the option**, and stating it as prose is what let three earlier revisions of this record drift. As a
  table, where "2" means a stamp and a separate position coexist:

  | Deferral option | Preserving clamp | Rewriting clamp |
  |---|---|---|
  | **A** — no deferral | 2, for late events only | **1 everywhere** |
  | **B** — exception, stamp preserved | 2 | 2, for deferred events |
  | **C** — exception, stamp rewritten | 2, for late events only | **1 everywhere** |
  | **D** — restated over render position | 2 | 2, for deferred events |

  Only **A or C with a rewriting clamp** reaches one quantity everywhere. It buys that not by losing the displacement —
  which stays recoverable at the moment of the rewrite — but by needing a deferred-store field to hold what every other
  cell carries in the envelope for free — or, under **Option A**, which retires the deferred store entirely, in bounded
  diagnostics metadata instead. An earlier revision asked A for storage the option removes.

  **A rewriting clamp changes shipped behaviour under all four options**, not only under A and C:
  `render/hot.rs` preserves the stamp today, so any rewriting selection is a code
  change on top of whatever documentation the selection already costs. The documentation cost is broader than the
  rewriting question, though — see the acceptance transaction. B and D falsify ADR-0032 clause 16 whatever the clamp
  does, because a preserved stamp stops identifying the quantum that renders the event; C leaves clause 16 true and
  falsifies clause 18 instead. **Option A is the only one-record selection**, on either branch — its reachable branch
  supersedes clause 16 and owes a recovery definition, but both live inside this record.
- **Safety/correctness control.** Clause 12's second sentence is the invariant that prevents the actual harm, and no
  option touches it. The both-late-and-deferred case is the conformance test that catches a wrong composition, and
  `HOST-INV-021` already names it.
- **Revisit condition.** Phase 3's measured ingress capacities, if they show overrun is either unreachable or routine.
  Unreachable strengthens Option A retroactively; routine raises the starvation question this record explicitly does not
  answer.

## Specification update

On acceptance:

1. this record becomes `Accepted`, the [decision index](../ADR.md) row moves with it, and its `Supersedes` metadata
   and ADR-0001's `Superseded by` record whichever relationship the selection creates — **clauses 12, 14 and 16 under
   Option B or D; clause 14 alone under Option C; under Option A, clause 16 if its runtime overrun stays reachable and
   nothing if disjoint budgets retire it**;
2. **only the timing and clamp rules selected here become normative.** `HOST-INV-021` is a compound invariant: it also
   carries the admission order, the deferred store's semantics, and the ingress-capacity question, none of which this
   record decides. Acceptance therefore retires exactly one thing — the sentence saying the invariant "proposes
   exceptions to an accepted decision rather than applying one", which is what this record exists to make false. **The
   rest of `HOST-INV-021` stays `Deferred to Phase 3`**, and the specification splits the invariant if that is what it
   takes to mark the two halves differently. Promoting the whole invariant would make policies normative that this
   record's own *decision boundary* excludes. Under B, C and D the mechanism exists and the deferred store, admission
   order and exhaustion policy remain Phase 3's, unpromoted.

   **Under Option A the invariant is split, not retired**, and an earlier revision of this record said "retired" —
   which would have deleted a contract that has nothing to do with per-quantum capacity. `HOST-INV-021` also carries
   the **release window** for `max_scheduled_events_in_flight`: reaching that capacity delays a release rather than
   failing one, and when the delay makes an event miss its quantum, clause 16 clamps it and **both** counters rise.
   That field is the scheduler's and survives every option here. Retiring the whole invariant under A would remove its
   behaviour while leaving the field in the profile, which is exactly the shape the specification names as a defect:
   a runtime-bounded field with no defined behaviour on reaching it;
3. the [host-profile specification](../specs/spec-host-profile-and-render-limits.md) states the chosen rule as one
   coherent contract rather than as a departure from ADR-0001, per `PROCESS.md`'s rule that a current specification
   presents one current rule;
4. **if the selection touches either clause below, a successor to
   [ADR-0032](ADR-0032-sample-time-and-event-timestamps.md) is accepted in the same transaction** — a *successor*, not
   an amendment: `decisions/README.md` says accepted reasoning is not rewritten and "a semantic change uses a
   successor". **Option A with a preserving clamp touches neither of ADR-0032's clauses**; whether it is free depends
   on its reachability branch, below. **ADR-0032's two clauses trigger on different things.**

   **Clause 16** says the scheduler derives an event's quantum index as `t.0 / Q` from its `time` when assigning it
   under ADR-0001 clause 12. Under the **stamp-preserving deferral options, B and D**, a preserved stamp still resolves
   to quantum `k` while the event is assigned to `k + 1`, so clause 16 becomes false the moment deferral exists.
   **Option C does not falsify it** — it rewrites the stamp to the assigned position, which is precisely what keeps the
   arithmetic true. Two earlier revisions got this wrong in opposite directions: one scheduled no ADR-0032 update for
   the recommended preserving pairings at all, and its repair then swept C in along with them.

   **Clause 18** additionally becomes false under **any stamp-rewriting pairing** — Option C, and equally A, B or D
   paired with a rewriting clamp — because a rewritten `time` under an unchanged `source` asserts an origin the value
   no longer has. The successor must then also say what a provenance tag claims once the value it describes can be
   displaced.

   So **every deferral or rewriting selection is at least a two-record change**, through ADR-0032. **Option A is a
   one-record change on both branches** — no ADR-0032 successor is needed, and the ADR-0021 successor an earlier
   revision demanded was withdrawn once the specification showed the prohibition binds the *plan* and A's overrun
   sources are not part of it. What its reachable branch owes is not another record but two pieces of content in this
   one: the callback fault's recovery, and clause 16's supersession;
5. **the [Sound Core render contract](../specs/spec-sound-core-render-contract.md) moves with it, and this is not
   optional.** `SOUND-INV-016` says a note-on, note-off, gate or retrigger "occurs at its declared sample within the
   quantum". Under Option B, C or D a deferred edge occurs at its **render position** instead — `clamped_position + dQ`
   after `d` deferrals, equal to `stamp + Q` only for an on-time event deferred exactly once.

   **Narrowing it to "the non-deferred case" is not enough, and every option needs the update including A.** A late but
   never-deferred edge (`d = 0`) already occurs at the *clamped* position rather than at its declared sample, under
   every option, because every option has a clamp. The invariant as written does not cover that today; it is coherent
   only if clause 16's clamp is read as an exception it does not mention. **So `SOUND-INV-016` is restated over the
   render position outright**, which covers the clamped and deferred cases together, rather than being scoped around
   whichever case the selection introduces. **Clause 13's control-rate boundary** is stated over whichever quantity the
   control-rate sub-question selects.
   **`SOUND-INV-006` says an event "retains its `StreamEpoch` and absolute `SampleTime`"** — that is falsified by *any*
   rewriting pairing: Option C, and equally A, B or D paired with a rewriting clamp, so every such pairing must update
   it too. Accepting a selection while updating only the host-profile specification would leave
   **two `Current` specifications with incompatible rules**, the failure `PROCESS.md` forbids by making one fact have
   one authority;
6. the unresolved question recording that the clamp's stamp behaviour "is ADR-0001's question, not this
   specification's" is answered and removed;
7. **under Option B, C or D, the causal-order policy is decided in the same transaction or named as an explicit
   further prerequisite.** `+Q` can reorder a note-on after its own note-off, this record's boundary excludes the
   policy that fixes it, and no artifact here supplies one — so accepting ADR-0043 alone does **not** close Phase 3's
   ADR-0001 prerequisite under a deferral option. Saying otherwise would mark a gate complete over a known correctness
   hole. **Option A closes it unaided once its content is complete.** On the unreachable-fault branch nothing further
   is owed; on the reachable branch A owes a named output, state and epoch behaviour for the callback fault before an
   implementer can write it. Two earlier reads recorded first that A always closes it and then that it never does;
   neither was right;
8. [`NOW.md`](../NOW.md) records how much of Phase 3's ADR-0001 entry decision the selection actually closed — **and that Phase 3 implementation
   remains blocked, because the entry conditions are conjunctive and ADR-0022 is still `Deferred`.**

## Review

Reviewer: an independent `codex` read, repeatedly. Seventy-five findings, forty-nine of them P1, none editorial. The
round-by-round chronology is not retained: it documented the drafting process rather than the decision, and a reader
choosing between these options does not need it.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.

### Open items a selection has to close

**These are named rather than solved, and two of them block implementation.**

1. **The causal-order hole blocks Options B, C and D.** `+Q` can render a note-on after its own note-off — at `Q` = 64,
   a note-on stamped at sample 63 defers to 127 while its note-off at 65 renders first, stranding the voice. `ADR-0023`
   cannot repair it, because a same-sample rule needs a tie and these positions differ. Ordering admission by stamp does
   not repair it either: that changes which event is inspected, not where either renders. Any fix must move the
   successor's render position, which is scheduler design outside this record's boundary.
2. **Option A's reachable branch owes a callback-fault recovery** — the output, state and epoch behaviour of a runtime
   overrun, terminal-fault or otherwise. Until it is named, an implementer cannot write it.
3. **No selection closes Phase 3's ADR-0001 prerequisite on this record alone**, except Option A on its
   unreachable-fault branch. Accepting this record is not the same as clearing that half of the Phase 3 gate.
4. **The starvation policy, the deferred store's bound and its exhaustion behaviour stay Phase 3's**, and `+Q` must not
   be read as having settled any of them.
5. **`offline.rs:178`/`:182` windows events by stamp** while the renderer admits by position. Not a live defect — its
   premise is that the offline path cannot present a late event — but the premise should be asserted rather than
   assumed once deferral exists.
6. **The recording argument stays conditional on `ADR-0024`**, which has not decided what a recorded event is. It is
   counted neither for Option B nor against Option C.

### What a reader should attack first

Whether the *does not decide* list is honest — specifically whether choosing `+Q` commits a starvation policy this
record claims to leave open. And whether Option A's survey is now fair: its cost has been mis-stated four times, in
alternating directions, and a fifth correction is more likely than a settled one.

### Four transferable lessons

- **A comparison that flatters the recommendation is the failure mode to watch.** Three times this record built one: a
  code sweep read as favouring Option D when it discriminated nothing; an Option A falsifier built on a burst that
  distinguishes no option; and "no stuck note" for A, which `ADR-0021`'s unqualified queue-drop permission contradicts.
- **Verifying that a cited clause says what you claim is the easy half; verifying that it reaches your case is the half
  that decides the conclusion.** An ADR-0021 successor requirement was written into this record and later withdrawn
  because the specification says that prohibition "binds the *plan*", and neither overrun source is part of one.
- **A branching option needs branching sentences.** Option A's cost depends on whether its overrun is reachable, and
  three consecutive rounds found this record asserting one branch as though it were both.
- **A repair is finished when every other sentence making the same claim has been found** — and grep finds only the
  author's own vocabulary. "Reports less" survived four sweeps for "loses" and "discards".
