# ADR-0045: Cross-Control Causal Order Under Deferral

| Field | Value |
|---|---|
| ID | ADR-0045 |
| Status | Superseded |
| Phase | 3 |
| Created | 2026-08-24 |
| Last reviewed | 2026-08-25 |
| Related | ADR-0043; [ADR-0044](ADR-0044-deferral-causal-order.md); ADR-0023; ADR-0032 clause 21; `SOUND-INV-016`; `HOST-INV-021` |
| Supersedes | — |
| Superseded by | [ADR-0046](ADR-0046-destination-quantum-admission.md) |

This record meets `PROCESS.md`'s durable-decision test on one count: it defines a real-time ownership boundary — whether
the renderer may hold, on the audio thread, any relation between events addressing **different** controls. It fixes no
value.

> **Superseded on 2026-08-25, dissolved rather than answered.** ADR-0046 removes capacity deferral, so the selective
> `+Q` movement that creates this cross-control hazard no longer exists. The analysis below remains historical
> evidence; it is not an implementation prerequisite.
> **Everything below this banner is pre-supersession history unless a sentence explicitly records ADR-0046's
> disposition.** No decision, constraint, work item, risk control, revisit condition or review scope below remains
> active. ADR-0022's later boundary correction moved its physical evidence to the Phase 9 exit gate.

> **Opened by [ADR-0044](ADR-0044-deferral-causal-order.md)'s option survey, 2026-08-24.** That survey's generality
> frame asked a repair to hold for every pair whose meaning depends on order. No candidate did, and the residual every
> candidate shared was the cross-control one. On the maintainer's scope decision the same day, ADR-0044 narrowed to
> **same-control** causal order and this record took the remainder, so that narrowing a question did not quietly
> discard the part that was hard.

## Historical deferral

| Field | Value |
|---|---|
| Deferred to | **Historical, no longer active.** Before ADR-0046 dissolved the question, this record, ADR-0044 and ADR-0022 formed the Phase 3 entry gate. ADR-0022 remained there at supersession and was later moved to the Phase 9 exit gate |
| Owner | Project maintainer — this is a single-maintainer repository, so there is no second party to assign |
| Historical input required | A reachability argument or measurement: which cross-control orders a real plan depends on, and at which rates. ADR-0046 eliminated the capacity-deferral mechanism instead |
| Historical reason | The candidate space was empty on the terms then accepted. ADR-0044's F2 frame refused an audio-thread walk of a general dependency graph, and every mechanism its survey found was keyed to one control. ADR-0046 later removed the capacity movement instead of selecting a causal-order mechanism |
| Historical safety basis | **Unreachability, and nothing else.** No V2 code deferred, so the hazard could not occur before the code that caused it was written. A draft called the symptom bounded and non-persistent; that was false, as *Why this gated Phase 3 before supersession* establishes. ADR-0046 now removes the cause rather than deferring the answer |

## The hazard

`Q` = 64, two events on **different** controls, established by evaluation rather than by argument:

- a control-rate automation stamped at sample 63 takes effect at the first quantum boundary at or after its render
  position, which is 64, and so lands **before** a sample-rate note edge at sample 65;
- defer the automation to 127 under ADR-0043's `+Q` rule, and its effect begins at boundary 128 — **after** that note.

Historically, the order the plan declared was reversed and no same-control repair saw it: the two events shared no
`(node, control)` pair. ADR-0044 was then `Deferred` and had selected nothing. Both records are now `Superseded` because
ADR-0046 removes the capacity movement; `ADR-0023` still does not reach the historical example because 65 and 128 are
not a tie.

**This refutes an argument ADR-0044's first draft made** — that `SOUND-INV-016`'s quantum-boundary rule for
control-rate responses closed most of the cross-control residual, because a pair less than one quantum apart was never
ordered by sample position anyway. The boundary rule does not protect such a pair; it relocates it. That correction is
recorded in ADR-0044's survey and is the reason this record exists rather than a sentence there.

## Why this gated Phase 3 before supersession

**A first draft of this record argued the opposite, and both of its premises were false.** It is corrected here rather
than quietly rewritten, because the error is instructive: it read `PROCESS.md`'s decision-timing rule as licence to
leave a known correctness violation outside a gate.

- **Phase 3's outcome does name it.** The draft quoted [`ROADMAP.md`](../ROADMAP.md#phase-order)'s *Exit requires*
  line, which lists within-block placement, tempo mapping, same-sample ordering, exhaustion and partition determinism,
  and concluded the outcome did not depend on this choice. It skipped the **Outcome** line directly above, which reads
  "sample-accurate note/transport/**automation** ordering". The hazard above reverses an automation against a note.
  That is the phase's stated outcome, so `PROCESS.md`'s test — a gate requires an accepted ADR when its observable
  outcome depends on the durable choice — is met rather than avoided.
- **The symptom is not transient.** The draft claimed the mis-timing affects only the events involved and that the
  next event on either control clears it. That is false for stateful DSP. The sine kernel integrates its frequency
  into a phase accumulator carried across quanta (`node/kernels.rs`, read at `d1dd12a3`: `increment` is added per
  sample and `*phase = running` persists), so delaying a frequency automation by a quantum leaves a **permanent**
  phase offset that survives the controls converging. A filter's state has the same property. The symptom therefore
  persists exactly as ADR-0044's stranded gate does, and the distinction the draft drew between them does not exist.

**Historical consequence.** Before ADR-0046, Phase 3 could not close with this unresolved or begin before this record,
ADR-0044 and ADR-0022 were `Accepted`. That made it a third prerequisite where the scope split was meant to avoid
adding one — `PROCESS.md` warns that replacing one prerequisite with another is not progress. ADR-0046 later removed
the capacity-deferral mechanism and dissolved both causal-order questions, leaving ADR-0022 as the sole prerequisite.

## Decision

**Historical decision, now superseded:** deferred to the Phase 3 entry gate. Its interim constraint was that no
specification or review could claim V2 preserved declared cross-control order under capacity deferral. ADR-0046 removed
that mechanism and dissolved the question, so neither the decision nor the interim constraint remains current.

## Historical revisit conditions

Before supersession, the record named these triggers:

- a candidate mechanism is proposed that does not require an audio-thread dependency-graph walk;
- a phase exit, specification, or external consumer needs a general order-preservation promise;
- ADR-0044's F2 frame is revisited for another reason, since the audio-thread dependency-graph refusal emptied this
  record's candidate space.

ADR-0046 exercised the architectural alternative: it removed capacity movement and dissolved the candidate space.
These triggers are not live work.

## Review

Reviewer: opened as part of ADR-0044's survey repair transaction, which is where the hazard was established and where
the maintainer's scope decision assigned it here. Independent reads found three pre-supersession defects: the question
needed a gate, that gate had not reached its then-current consumers, and the safety row called a persistent symptom
transient. ADR-0046 later dissolved the gate, making consumer propagation obsolete; the *Historical safety basis* row
above retains the correction to the transient-symptom claim.

**ADR-0046 dissolved this record rather than answering it.** With no capacity deferral there is no
deferral-induced cross-control inversion. ADR-0044's survey remains historical evidence for why removing that movement
closes both questions.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
