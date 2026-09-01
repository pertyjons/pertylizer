# ADR-0051: What a locate owes a gate

| Field | Value |
|---|---|
| ID | ADR-0051 |
| Status | Accepted |
| Phase | 3 |
| Created | 2026-08-28 |
| Last reviewed | 2026-08-28 |
| Related | ADR-0001, ADR-0046, ADR-0047, ADR-0050, `SPEC` sound-core render contract, `SPEC` host profile and render limits |
| Supersedes | — |
| Amends | ADR-0050 clauses 5, 7 and 8 |
| Superseded by | — |

## Durable boundary

**Delivered audible behaviour.** What a listener hears after seeking into a note that was sounding is settled here,
and changing it later changes what they hear. ADR-0050 named delivered timing behaviour as its own durable boundary
for the same reason.

**Why now.** ADR-0050 clause 7 is the immediately next implementation slice and it cannot proceed safely without this
answer. It was cut from the transport-activation slice precisely because the answer was missing: a working
implementation was built and independently reviewed **eleven times without converging**, six of the eleven `P1`s were
the previous round's own repair, and every one of those collisions was in this one interaction. `PROCESS.md`'s
decision-timing rule names that situation directly — a design that was never written down.

## Decision boundary

ADR-0050 clause 7 says a locate restores, for every prepared target, the last value established before the new plan
position. ADR-0050 clause 5 says a note crossing the activation is **ended** at the boundary and explicitly not
resumed. A gate is one control that both automation and a note contract write, so on such a gate the two clauses give
different answers and ADR-0050 does not say which governs. That is the hole this record fills.

Non-goals: note chasing (ADR-0050 clause 5 already declines it); same-sample ordering between session, note and
automation events, which is ADR-0023's; and any live-ingress producer, which stays outside ADR-0050's scope.

## Evidence

Verified in the current code rather than inferred from the records:

- a gate is **boolean and edge-triggered**. `Run::gate` computes `raised = value > 0.0` and returns early when
  `raised == self.held`, so re-asserting a held gate is not an edge and does not retrigger
  ([`node/kernels.rs`](../../../crates/synth_engine_v2/src/node/kernels.rs));
- `NoteEdge::value` is `ONE` for an on edge and `ZERO` for an off edge
  ([`render.rs`](../../../crates/synth_engine_v2/src/render.rs));
- the renderer lowers a gate on **every** note-off that resolves against the registry, by its registered slot and with
  no depth accounting; it is a falling edge only where the gate is presently held
  ([`render/hot.rs`](../../../crates/synth_engine_v2/src/render/hot.rs));
- clause 5's mass release emits `TimedControl { offset: ZERO, value: ZERO }` into the boundary quantum, and the
  renderer places adoption gates **before** the quantum's event-derived controls, so the catch-up batch is the later
  of the two writes ([`render/hot.rs`](../../../crates/synth_engine_v2/src/render/hot.rs));
- `ParameterTarget` and `NoteTarget` both carry `(NodeSlot, ControlIndex)`, so a physical target is a comparable key
  ([`plan.rs`](../../../crates/synth_engine_v2/src/plan.rs)).

The uncertainty that could change this decision is named in clause 3 below: a second producer emitting onto one gate
has no ownership law, and this record does not supply one.

## Options

**A — clause 7 taken literally.** The last pre-destination write wins on a gate as on any other target. This is what
the preserved implementation on `wip/transport-activation-full` built.

**B — a gate held open at the destination is owed `ZERO`.** The last-write rule computes the batch and is then
overridden for those gates.

The status quo is neither: clause 7 as written *is* A, but it was written without the gate case in view, and the
implementation that followed it did not converge.

The tradeoff that decides it is not preference. Under A the boundary delivers `ZERO` from the mass release and then
`ONE` from the batch **at the same offset**, and because the control is edge-triggered that pair is a **rising edge**:
the envelope attacks again, with no note contract behind it, since the registry has already discarded the occurrence
and the suffix omits its release. That is the note chasing ADR-0050 clause 5 declines.

The deeper property is that automation raising a gate a note already holds is **inert while playing through** — the
kernel returns early. It becomes audible only because the locate inserted a synthetic gate-down in front of it. So
"last writer" is not a semantics-preserving rule for an edge-triggered control across an activation, and A's appeal is
an artefact of reading the rule on a control kind it was not written for.

## Decision

### 1. The rule

A locate computes the last pre-destination write for **every** prepared target, note edges included, and then
substitutes `ZERO` for every physical `(node, control)` gate held open by an in-scope note contract immediately before
the destination.

Note edges write that history like any other write, and dropping them is not available: automation raising a gate,
then a note-off before the destination, would otherwise restore the raised value the note-off had dropped.

### 2. The predicate is the destination-open contract, not the release scope

The two are different sets and the distinction is load-bearing.

A **forward** seek can land inside a note the retired stream never sounded — the registry holds nothing for the mass
release to end — and that gate must still be low, because the new stream does not carry the note-on that would raise
it. Conversely the retired stream may release a note before a destination where no contract is open, and automation
raising that gate there is legitimate and survives.

So the justification is specifically *do not chase a destination-open note*. "The activation is newer than the
history" would be too broad and would decide the second case wrongly.

**In scope means the replaced producer's**, exactly as ADR-0050 clause 5's release scope. A contract belonging to a
producer the activation does not replace is neither chased nor cut: a seek moves plan time, it does not lift a
performer's finger. The predicate carries the scope now, before a non-compiled producer can hold such a contract,
because retrofitting it is what would go unnoticed.

### 3. The substitution aggregates by physical target

Two prepared parameter slots aliasing one `(node, control)` would otherwise disagree — one forced low, a later one
restoring what it read — and whichever published last would win. Aggregating on the physical target, rather than on
the note slot or the parameter slot, is what keeps the **substitution** single-valued.

It does not make the last-write half single-valued, and an independent review was right to separate them: two aliased
slots with different pre-destination writes would still produce two rows that disagree, and publication order rather
than chronology would decide. The situation is unreachable rather than handled — a plan lowers exactly one prepared
target per `(node, control)`, one row per control per node — so this is a bound on the claim and not a branch to
write. A node kind that ever aliases must decide the last-write half before it does.

Multiple occurrences on one note slot are possible, so the open-contract depth is tracked separately from the gate
history. The history reproduces the renderer: every resolving note-off writes `ZERO` even where another occurrence on
that slot remains open. Gate history is never derived from depth.

### 4. The batch's size does not change

Every prepared target still gets exactly one row. This record decides a row's **value**, never whether it exists, so
`HOST-INV-022`'s bound and the admission check that rests on it are untouched.

### 5. An omitted crossing release keeps its gate-down

ADR-0050 clause 5 omits a release whose note-on lies before the anchor, because stamping refuses a release with
nothing to pair. That omission drops two different things at once, and only one of them should go: the **note
contract**, which the boundary release has already ended, and the **gate-down the plan authored**, which nothing has
replaced.

Dropping the second is a defect, and the failure is a stuck note rather than a subtle one. Take an in-scope note-on
before the destination, automation raising that same gate at or after the destination, and the release later. The
catch-up forces the gate low at the boundary, correctly; the automation is a **suffix** event, so it applies after the
batch and raises the gate; and the release that would have lowered it is omitted. The gate stays high with nothing
left in the stream that can lower it. Playing through the same span ends the note at the release.

So an omitted crossing release **still contributes its gate write**: a bare `SetParameter` of `ZERO` on the release's
physical gate target, at the release's own position, carrying no note identity. The note contract ends at the
boundary as clause 5 says; the authored gate timeline is preserved exactly. The omission remains counted, because it
is still a transformation and ADR-0001 clause 16 requires it be named.

Two consequences are stated rather than left to be found, and the first is smaller than an earlier draft of this
record claimed. **Admission needs no change.** The gate write takes the omitted release's place at its own position,
one for one, so the candidate's event count at every position is at most the admitted stream's — and that stream was
already admitted against the compiled share. The suffix is larger than it would have been under the bare omission,
which is what the draft meant, but it is never larger than the list admission already judged.

The second is a real precondition: the write requires the gate to be a **prepared parameter target**. That holds for
the envelope today, whose note control and exposed gate parameter are the same control, but the types do not enforce
it for a future node kind. A note slot whose gate has no prepared row is **refused when the activation is built**,
named as such, rather than silently losing its gate-down. The refusal is where the value is needed, which is also the
first point that can name the offending event.

### 6. A gate reached by more than one producer has no ownership law

ADR-0050 clause 8's obligations become three. A compiled note and a surviving live note can address one scalar gate;
ending the compiled one writes `ZERO` to the gate they share and cuts the live note with it — audibly, and for the
same reason the scope in clause 2 exists. A gate carries neither producer attribution nor depth, so nothing at the
boundary can tell the two apart, and **the scope predicate alone is therefore not sufficient**.

**The obligation covers the whole catch-up row, not only the forced-`ZERO` substitution.** Excluding an out-of-scope
contract from the substitution is not sufficient: every prepared target still gets a row, so a gate a live producer
holds would receive its plan-history or prepared value — typically `ZERO` — and the performer's finger would be
lifted by the row rather than by the substitution. An independent review found that clause 2's promise was therefore
not delivered by the substitution alone.

Before multiple producers may emit onto one gate, one of three must be decided: refuse target sharing across release
scopes at admission, make a voice's gate producer-exclusive, or design a depth-and-ownership aggregation law —
**and, with it, what the catch-up row does for a gate the activation does not own**: suppressed, carrying the value
that producer holds, or refused at admission. This record does not choose among them; it forbids the situation until
one is chosen. The question does not arise while the compiled producer is the only one that emits, which is the fact
the present scope rests on.

**Phase 3's live ingress preserves that fact by a check rather than leaving it true by accident.** A non-compiled
producer can now mint and emit, so the premise above stopped holding on its own. Building an activation is therefore
refused once a stream has adopted a live ingress store: no stream that can activate has one, so no gate this record
reasons about is reached by two producers. Store adoption is also refused while
an activation candidate is outstanding, which closes the opposite ordering.
The check is on the **store** rather than on notes currently open, because
a count of those returns to zero while both edges of a live note are still queued and neither has rendered — an
activation built there would sit over a note about to sound. Recorded here because the premise is this record's, and
the check that keeps it true is not.

## Consequences and risks

**The audible consequence, stated rather than hidden.** Seeking into a note that was sounding leaves it silent until
the new stream's own next note-on, even where automation moved that gate after the note-on. That is ADR-0050 clause
5's "cut, and not resumed" extended to the one case where the catch-up could have undone it.

**What this costs that A did not.** Automation can no longer hold a gate high across a locate while an in-scope note
contract is open at the destination. Playthrough-state fidelity is sacrificed to preserve the no-note-chasing policy.
The trade is deliberate and is the reason this record exists rather than a silent implementation choice.

**What it removes.** The `written_by_note` reconciliation — the single piece of state behind six of the eleven `P1`s,
all in this interaction — has no counterpart in the rule above. It is removed rather than reconciled.

**Where it stops holding.** Clause 5 above is the boundary. Live ingress cannot be enabled on this record's strength.

## Specification update

The current contract presents one coherent rule rather than a clause plus an exception, per `PROCESS.md`:

- `SOUND-INV-018`'s catch-up paragraph states the rule, the predicate and the physical-target aggregation together,
  and its obligation list becomes three;
- `HOST-INV-022` states the same rule where it bounds the batch, with the in-scope qualifier, and records that the
  count is unchanged;
- `SOUND-INV-018` also states that an omitted crossing release keeps its gate write, per clause 5;
- `SOUND-INV-017` is untouched: this record does not move any generation.

## Review

**Design consultation — Codex.** Asked the one question `NOW.md` recorded as blocking the slice. It selected option B
over A, corrected the predicate from the mass-release scope to the destination-open contract, and supplied the
physical-target aggregation rule and the third obligation in clause 6. It was then asked to read the drafted record
and found four further defects, all repaired here: the amendment had been written into ADR-0050 in place instead of
into this successor; the operative specification paragraph still stated the unamended rule; `HOST-INV-022` stated the
rule without its scope qualifier; and the obligation count was left at two in the specification. A second read found
clause 5's stuck gate and the incompleteness of clause 6's obligation, both of which are decided above.

**Codex is the design consultant here and therefore cannot be this record's independent semantic reviewer.**
`AGENTS.md` requires a reader that did not author the constraints, and Codex authored them. Its own second read
raised this, which is the finding that produced this section's present shape.

**The waiver this record first carried is discharged; a qualifying reader was found.** The first draft recorded that
none existed: Claude Code authored the text, Codex authored the constraints, and the Gemini CLI cannot authenticate
— it returns `IneligibleTierError`, its individual tier having been withdrawn. What that draft missed is the
replacement the same error names. The Antigravity CLI (`agy`) authenticates and runs Gemini, so it satisfies both
properties: it did not author this change, and it is not the author's family. The review was taken before the merge
that carries this record, which is the point `AGENTS.md` makes the waiver unavailable at.

**It found two, and the first is why the rule exists.** `HOST-INV-022` still stated clause 2's promise — "a seek does
not lift a performer's finger" — as fact, which clause 6 of this record had already established the batch does not
deliver: every prepared target receives a row, so a gate an out-of-scope producer holds is moved by the row rather
than by the substitution. `SOUND-INV-018` carried the qualification and the host-profile specification did not, so
two normative specifications disagreed about a safety property. It is repaired.

The second bounds clause 3 rather than changing it: aggregating on the physical target keeps the **substitution**
single-valued, but two aliased slots with different pre-destination writes would still produce disagreeing rows in
the last-write half. That situation is unreachable — a plan lowers exactly one prepared target per `(node, control)`
— so the claim is bounded above rather than a branch being written for it. Both are recorded where they land.

Three Codex reads remain the design evidence, and they found six defects between them, all repaired above. Codex
stays disqualified as this record's semantic reviewer for the reason given directly above.

The maintainer approved option B and its audible consequence on 2026-08-28, and clause 5's gate-down rule the same
day, each put separately.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
