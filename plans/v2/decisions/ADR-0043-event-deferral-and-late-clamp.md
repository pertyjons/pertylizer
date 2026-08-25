# ADR-0043: Event Deferral, and What a Timestamp Survives

| Field | Value |
|---|---|
| ID | ADR-0043 |
| Status | Accepted |
| Phase | 3 |
| Created | 2026-08-20 |
| Last reviewed | 2026-08-25 |
| Related | ADR-0001 clauses 12, 14 and 16; ADR-0021 part 2; ADR-0023; ADR-0032 clauses 17, 18, 19, 22 and 23; `HOST-INV-021`; REV-P00A; REV-P02 |
| Supersedes | **ADR-0001 clauses 12, 14 and 16, and ADR-0032 clause 16**, as Option D with a preserving clamp requires. Clause 12's first sentence is restated over render position; clause 14's "declared sample within the quantum" becomes the offset within the quantum that renders the event; clause 16's late condition is fixed to a single evaluation, when an event first becomes due; and ADR-0032 clause 16's derivation takes the render position as its input rather than the stamp, because a preserved stamp no longer identifies the quantum the event renders in. Clause 12's **second** sentence is untouched and does all the safety work. **ADR-0032 clause 18 is not touched**, because the selected clamp preserves the stamp and no pairing here rewrites one. This record is itself the ADR-0032 successor its *Specification update* requires — a successor accepted in the same transaction, not an amendment of ADR-0032's prose; see that section |
| Superseded by | **[ADR-0046](ADR-0046-destination-quantum-admission.md), capacity-deferral rule only.** The preserving late clamp, immutable stamp, control-response rule and prohibition on applying an event to produced samples remain in force |

This record meets `PROCESS.md`'s durable-decision test on two counts: it defines a real-time ownership boundary, and it
binds several later phases. It fixes no value. It declares no decision *class* — that vocabulary belongs to the former
workflow and [`ADR.md`](../ADR.md#decision-classes) keeps it for historical records only.

> **Partially superseded on 2026-08-25.** ADR-0046 removes this record's `+Q` capacity-deferral rule. The preserving
> late clamp and the rest of the accepted timing contract remain authoritative.
> **Current authority after supersession:** clauses 1, 2, 4, 5 and 6 in *Decision*, as rewritten below, define the
> surviving contract; clause 3 is withdrawn. Every other statement in this record about capacity deferral, a deferred
> store, its admission or starvation policy, or ADR-0044 as a Phase 3 gate is historical analysis from the original
> selection, not a current requirement. ADR-0022 is Phase 3's sole remaining entry prerequisite.
> Sections whose headings begin *Historical* describe the 2026-08-20 selection snapshot; any present tense inside
> those sections is historical present and makes no claim about the current specification.

> **Decided on 2026-08-20: Option D, with a preserving clamp, and the control-rate boundary derived from the render
> position.** The maintainer selected it against the options survey below, which is retained unchanged as the record of
> why the winner won. The original recommendation was *conditional* on a causal-order policy for capacity deferral,
> represented at the time by [ADR-0044](ADR-0044-deferral-causal-order.md). ADR-0046 later removed that mechanism and
> dissolved the condition. Accepting this record still does not by itself unblock Phase 3 because ADR-0022 remains.

## Historical durable boundary at original selection

Two boundaries, and either alone would require a record.

**A real-time ownership boundary.** Whether the renderer may move an event to a later quantum than its timestamp names
is a property of the engine's timing contract, not an implementation detail. Every later phase — the scheduler, the
voice pool, recording, and the offline/live equivalence gate — reasons over the answer.

**The cross-phase contradiction this record resolved.** On 2026-08-20,
[ADR-0001](ADR-0001-internal-render-quantum.md) clauses 12 and 14 and the then-current
[`HOST-INV-021`](../specs/spec-host-profile-and-render-limits.md) could not both be implemented as written. That
specification said so about itself and ran an interim rule it stated it had no authority to make. Accepting this record
settled which contract moved; ADR-0046 later removed the capacity-deferral half while retaining this record's late
clamp.

## Historical decision boundary at original selection

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

- the **ingress capacities** and the **deferred store's bound**, which the then-current `HOST-INV-021` said were not
  derivable from its field set — Phase 3. **This held for Option A too**, which two revisions of this record denied: A
  was said to force a source partition because an aggregate overflow would otherwise be unhandled. It would not — the
  Phase 1–2 contract rejected an over-full quantum before mutation, whatever mix of sources filled it — so A needed no
  capacity architecture decided here either. ADR-0046 later selected a source partition for a different reason:
  making Phase 3 renderer capacity a construction invariant;
- the deferred store's **exhaustion policy** and any **starvation** guarantee for an event that deferred repeatedly —
  Phase 3 at the time. ADR-0046 later removed that store, so neither remains current work;
- **same-sample ordering** — `ADR-0023`, which is `Proposed` and has no record yet;
- the **hardware clock mapping** and whether a `Hardware`-stamped tier should outrank an `Arrival`-stamped one —
  [ADR-0022](ADR-0022-hardware-time-mapping.md);
- the **capacity admission order** among events due in one quantum. The original `HOST-INV-021` stated one; ADR-0046
  later removed capacity admission and therefore left no current admission order for this record to reopen.

## Historical evidence at original selection

- **The contradiction was stated by the then-current specification against itself**, in `HOST-INV-021`: deferral
  "departs from both" clause 12 and clause 14, clause 16 is "the only exception ADR-0001 grants" and is granted for an
  already-rendered quantum, "so the precedent does not cover deferral". Three earlier revisions of that invariant
  reasoned wrongly about the same point and were recorded there.
- **Clause 16 cannot be reused as the mechanism.** Its rule — clamp to the first not-yet-rendered quantum boundary — is
  circular for a capacity shortfall, because the quantum that could not admit the event has not rendered either, so
  applying it literally puts the event back where it did not fit.
- **The then-current `HOST-INV-021` named three places a rewritten stamp was observable, and this record accepted one
  of them outright.** On **diagnostics**, that invariant argued the engine loses *how far* an event was displaced — but the
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
- **One site reads the stamp instead of the position:** the two window predicates in `offline.rs`'s `events_for` —
  `:178` and `:182` at that revision, `:179` and `:183` after ADR-0046's doc edit shifted them by one — select the
  event slice by `event.envelope().time().quantum_index()` while the renderer admits by position. It is not a live
  defect — its
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

## Historical options at original selection

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
  Any repair has to reach past the deferred event itself — moving the *successor's render position* too, by deferring
  an event's causal successors along with it, or neutralising the inversion where the voice is allocated, so that a
  note-off for a voice that has not started is remembered rather than discarded. Choosing between those is scheduler
  design that this record's boundary excludes. **So it is named as a blocker on B, C and D rather than papered over**, and it is
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

**Selected: from the render position**, for the same reason as the selection overall — one quantity, no
carve-out. The cost is that a control change deferred `d` times is delayed up to `(d + 1)Q - 1` frames rather than
`Q - 1`, and **`d` is unbounded in this record** because repeated deferral is part of the model and the starvation
policy that would bound it is Phase 3's. That is the largest cost this answer carries, and it is stated as unbounded
rather than as a comfortable single-deferral figure.

### The clamp, as a sub-question on the same axis

**Selected: preserve.** That is the shipped behaviour, so this half of the selection is a documentation change and not
a code change.

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

**Selected: Option D, with a preserving clamp, and the control-rate boundary derived from the render position.** The
maintainer selected it on 2026-08-20 against the survey above, which is retained as the record of why the winner won.

Stated as one rule rather than as a departure from one — which is Option D's whole point, and which the
[host-profile specification](../specs/spec-host-profile-and-render-limits.md) now presents as the current contract:

1. **An event is assigned to the quantum containing its render position.** For an on-time event that position is the
   envelope's `time`; for a genuinely late event it is clause 16's first not-yet-rendered quantum boundary. ADR-0046
   forbids moving either event for capacity.
2. **The envelope's `time` is immutable.** The preserving late clamp does not rewrite it, so the lateness displacement
   remains the difference between two quantities the renderer already holds.
3. ~~**A deferred event renders at the same intra-quantum offset, one quantum later**, and another deferral adds
   `+Q`.~~ **Superseded by ADR-0046.** The renderer never moves an event to recover capacity.
4. **Clause 16's late condition is evaluated once**, when an event first becomes due. This is the only reading under
   which the late count means "how many events arrived after their quantum had rendered".
5. **A control-rate response begins at the first quantum boundary at or after the event's render position.** Where that
   position falls exactly on a boundary — offset 0, or a clamped event — the response runs in that same quantum. It
   may never apply to samples already produced.
6. **Clause 12's second sentence is unchanged** — "no event is ever applied to samples already produced" — and it is
   what prevents the actual harm.

**The original condition is now dissolved.** It was first named, rather than solved, in
[ADR-0044](ADR-0044-deferral-causal-order.md). ADR-0046 superseded that record and removed the capacity movement that
created its causal-order hazard. ADR-0022 is the sole current Phase 3 entry prerequisite.

Historically, the condition was not a formality. `+Q` can reverse cause and effect: a note-on
at sample 63 defers to 127 while its note-off at 65 renders first, leaving a voice sounding. Every deferral option
carries it, `ADR-0023` cannot repair it because the positions differ, and no policy in this record prevents it. So
**B, C and D each carried a named correctness hole** under that mechanism, while Option A did not. ADR-0046 selected
the no-capacity-movement architecture, so this hazard is no longer reachable through overload handling.

Option D reaches the same runtime behaviour as Option B while leaving no weakened clause behind. The distinction is not
cosmetic: under Option B, clause 12's first sentence becomes a statement that is true except when it is not, and the
next capacity question arrives with a precedent for carving another exception. Under Option D there is one rule, the
safety sentence that actually prevents harm is untouched, and `+Q` is derivable from the rule rather than granted by it.

**The selection rests on one argument only, and review had already removed the other before it was made.** An earlier revision of
this record argued that the current renderer's stamp/position split shows the code has outgrown clause 12's wording.
**That argument is withdrawn**: the split exists only for clause 16's clamp, which ADR-0001 authorises outright, and B
and D compile to the same thing over the code that exists. So the selection rests on a governance judgement about
which text is easier to reason from next time — a real argument, but a weaker one than this record first made, and a
maintainer who weighs it differently is not disagreeing with any fact.

**No observable difference between B and D has been found.** The sixth read suggested the control-rate sub-question
might be one; the eighth established it is not, because clause 13 names the timestamp explicitly and neither framing
supersedes it — so each option may select either answer. Both attempts to find behavioural daylight between B and D
have failed, and the record says so rather than leaving the earlier suggestion standing. The choice is a governance
judgement about which text is easier to reason from, and nothing else.

**Historical falsifiers for the original selection.** ADR-0046 exercised the capacity branch and retired the
capacity-deferral conditions below; they no longer describe live revisit work.

1. **A Phase 1 or Phase 2 consumer that reads clause 12 literally.** Swept at `e9590577`, and the result is **neutral
   between B and D**: the one stamp-reading site is `events_for` in `offline.rs` — `:178`/`:182` at that revision and
   `:179`/`:183` in the current file — whose premise is that the offline path cannot present a late event. Neither
   option carries a migration cost beyond that site. This risk is discharged for both, and it discriminates between
   neither.
2. **If Phase 3 can admit destination occupancy before playback**, capacity movement is unnecessary. ADR-0046 selected
   exactly that architecture and retired the B/D capacity path.
3. **If the causal-order policy is expensive or contentious**, remove the movement that creates the ordering hazard.
   ADR-0046 did so and superseded ADR-0044 rather than completing a deferral-order policy.
4. **If restating an accepted clause proves to be a heavier governance act than granting a bounded exception would
   have been**, Option B was the better reading of the same facts. The two differ in what they leave for the next
   decision to inherit and need not differ in what the renderer does, so switching later would be a documentation
   change rather than a behavioural one — which is what makes this the cheapest of the four conditions to be wrong
   about.

Option C was rejected on accepted text rather than on preference: it falsifies **ADR-0032 clause 18's
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

## Historical consequences and risks of the original selection

This section records the cost model reviewed on 2026-08-20. Capacity-deferral costs, the causal-order risk and their
revisit conditions were retired by ADR-0046; only costs of the preserving late clamp remain current.

- **Accepted cost — added latency, and it is not bounded here.** Under the selected answer to the control-rate
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
  | **D** — restated over render position | **2 — selected** | 2, for deferred events |

  **The selection pays this cost.** D with a preserving clamp is the "2" cell: every consumer that asks which quantum
  renders an event asks the render position, and the stamp answers only what the producer declared. That is the price
  of keeping the stamp truthful, and it is accepted deliberately.

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
- **Retired risk — the causal-order inversion.** Under the original rule a note-on could render after its own note-off
  and strand a voice. ADR-0044 recorded that hazard as a Phase 3 prerequisite. ADR-0046 later removed the capacity
  movement, superseded ADR-0044 and dissolved the risk rather than selecting a reordering policy.
- **Surviving safety/correctness control.** Clause 12's second sentence still prohibits applying an event to samples
  already produced. The former both-late-and-deferred composition test is retired because ADR-0046 makes capacity
  deferral unreachable; the independent late-clamp tests remain current.
- **Revisit condition, exercised by ADR-0046.** The destination-admission design made capacity movement unnecessary.
  It therefore retired the deferral and starvation machinery while retaining the independent late-clamp contract.

## Historical specification update at original acceptance

Performed as one transaction on the original acceptance. ADR-0046 later replaced the capacity half; each affected item
below records that closure explicitly.

1. **This record becomes `Accepted`** and the [decision index](../ADR.md) row moves with it. Its `Supersedes` metadata
   and the `Superseded by` metadata of ADR-0001 and ADR-0032 record the relationship the selection creates:
   **ADR-0001 clauses 12, 14 and 16**, and **ADR-0032 clause 16**. Clause 12's second sentence is not among them.
2. **This record is the ADR-0032 successor**, and the reading is stated so it can be attacked rather than assumed.
   The pre-acceptance draft required "a successor to ADR-0032 accepted in the same transaction — a successor, not an
   amendment". A separate record would have had to restate this record's own rule to say anything at all, which is the
   duplication [`PROCESS.md`](../PROCESS.md#authorities) forbids by giving one fact one authority. So ADR-0043 is that
   successor for **clause 16 only**, ADR-0032's prose is not edited, and its metadata records the supersession.
   **Clause 18 is untouched**: it becomes false only under a stamp-rewriting pairing, and neither half of the selection
   rewrites a stamp.
3. **`HOST-INV-021` was originally split between timing and deferred Phase 3 capacity work.** ADR-0046 replaced that
   split with one current destination-admission contract: fixed producer shares, plan-time envelopes, release holds,
   one publication arbiter and terminal producer faults. There is no deferred-event store, admission order, starvation
   policy or capacity displacement. The immutable stamp, the one-time late evaluation and the preserving clamp survive.
4. **The [host-profile specification](../specs/spec-host-profile-and-render-limits.md) presents one coherent current
   rule**, not a departure from ADR-0001, per `PROCESS.md`'s rule that a current specification states what
   implementation must do now. The unresolved-question row asking whether clause 16's clamp preserves or rewrites the
   stamp is **answered and removed**; the row asking when clause 16's condition is evaluated is answered and removed
   with it.
5. **The [Sound Core render contract](../specs/spec-sound-core-render-contract.md) moves in the same transaction, and
   this is not optional.** `SOUND-INV-016` says a note-on, note-off, gate or retrigger "occurs at its declared sample
   within the quantum". It is **restated over the render position outright**, which covers the clamped and the deferred
   case together. Narrowing it to the non-deferred case would not have been enough: a late but never-deferred edge
   already occurs at its *clamped* position rather than at its declared sample, and the invariant as written does not
   cover that today. Clause 13's control-rate boundary is stated over the render position with it.
   **`SOUND-INV-006` moves too, and only in its derivation input.** Its promise that an event "retains its
   `StreamEpoch` and absolute `SampleTime`" is what the preserving selection makes true rather than false, so that
   half is untouched; what changes is that the quantum index and offset are named as derived from the **render
   position** rather than left to be read as derived from the stamp. That is ADR-0032 clause 16's supersession
   surfacing in the specification a reader actually implements from, and omitting it would have left `SOUND-INV-006`
   and `SOUND-INV-016` disagreeing about which quantity assigns a quantum. Accepting a selection while updating only
   the host-profile specification would leave two `Current` specifications with incompatible rules.
6. **ADR-0044 originally carried the causal-order prerequisite.** ADR-0046 removed the capacity movement, superseded
   ADR-0044 and dissolved the prerequisite. ADR-0022 is now the sole Phase 3 entry prerequisite.
7. **The master plan and `ROADMAP.md` were originally qualified for deferral.** ADR-0046 replaces those qualifications
   with pre-render destination admission and no capacity movement; no current gate bullet requires ADR-0044.
8. **[`NOW.md`](../NOW.md) now records the replacement contract.** Phase 3 remains blocked only on ADR-0022's evidence.

## Review

Reviewer: an independent `codex` read, repeatedly. Seventy-five findings, forty-nine of them P1, none editorial. The
round-by-round chronology is not retained: it documented the drafting process rather than the decision, and a reader
choosing between these options does not need it.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.

### Disposition of items the original selection left open

1. **The capacity-deferral causal-order hole is dissolved.** ADR-0046 removes `+Q` movement and supersedes ADR-0044;
   an admitted note-on can no longer move behind its note-off merely because a destination quantum is full.
2. **The replacement leaves one Phase 3 prerequisite.** ADR-0022's hardware-time evidence remains open. ADR-0044 is
   `Superseded`, not `Deferred`, and is not a gate.
3. **There is no starvation or deferred-store item.** ADR-0046 replaces them with plan-time envelopes, fixed shares,
   release holds and terminal producer faults. The renderer never delays an event to recover capacity.
4. **The offline late-clamp premise remains an implementation obligation.** `events_for` in `offline.rs` — `:179` and
   `:183` — windows by the immutable stamp while the renderer admits by the clamped render position. Phase 1–2 rely on
   the offline path never
   presenting a late event; a named test must assert that premise, or the selector must window by render position.
   Phase 3 owns that test before its offline path relies on the same boundary; this is implementation work, not an
   additional entry prerequisite. Its arbiter presents sealed admitted batches for the imminent call and must preserve
   the same guarantee. The obligation is tracked in [`NOW.md`](../NOW.md#later-owned-work) and Phase 3's
   [exit gate](../master-plan.md#phase-3-sample-accurate-scheduler-and-block-partition-invariance).
5. **The recording argument stays conditional on `ADR-0024`**, which has not decided what a recorded event is. The
   selection preserves the stamp, so the question does not arise for it — but it is recorded rather than dropped,
   because a future rewriting proposal would inherit it.

### Current review focus

Review the surviving ADR-0032 clause 16 successor, immutable stamp, preserving late clamp, one-time late evaluation,
control-response boundary and prohibition on applying an event to produced samples. ADR-0046 owns capacity admission;
the historical option survey above is no longer load-bearing for that architecture.

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
