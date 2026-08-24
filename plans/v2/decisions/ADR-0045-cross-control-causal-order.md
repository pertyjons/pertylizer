# ADR-0045: Cross-Control Causal Order Under Deferral

| Field | Value |
|---|---|
| ID | ADR-0045 |
| Status | Deferred |
| Phase | 3 |
| Created | 2026-08-24 |
| Last reviewed | 2026-08-24 |
| Related | ADR-0043; [ADR-0044](ADR-0044-deferral-causal-order.md); ADR-0023; ADR-0032 clause 21; `SOUND-INV-016`; `HOST-INV-021` |
| Supersedes | — |
| Superseded by | — |

This record meets `PROCESS.md`'s durable-decision test on one count: it defines a real-time ownership boundary — whether
the renderer may hold, on the audio thread, any relation between events addressing **different** controls. It fixes no
value.

> **Opened by [ADR-0044](ADR-0044-deferral-causal-order.md)'s option survey, 2026-08-24.** That survey's generality
> frame asked a repair to hold for every pair whose meaning depends on order. No candidate did, and the residual every
> candidate shared was the cross-control one. On the maintainer's scope decision the same day, ADR-0044 narrowed to
> **same-control** causal order and this record took the remainder, so that narrowing a question did not quietly
> discard the part that was hard.

## The deferral

| Field | Value |
|---|---|
| Deferred to | The **Phase 3 entry gate**, alongside [ADR-0044](ADR-0044-deferral-causal-order.md) and ADR-0022. A first draft of this record left it ungated; that was wrong, and the independent read established why — see *Why this gates Phase 3* |
| Owner | Project maintainer — this is a single-maintainer repository, so there is no second party to assign |
| Input required | A reachability argument or measurement: which cross-control orders a real plan actually depends on, and at which rates. Nothing in V2 can produce one yet, because no ingress exists to overload and no deferral exists to invert |
| Why not now | The candidate space is empty on the terms already accepted. ADR-0044's F2 frame refuses an audio-thread walk of a general dependency graph, and every mechanism its survey found was keyed to one control. Deciding this now would mean either relaxing F2 or inventing a mechanism nobody has proposed |
| What makes it safe | **Unreachability, and nothing else.** No V2 code defers, so the hazard cannot occur before the code that causes it is written. A draft of this row also called the symptom bounded and non-persistent; that is false and *Why this gates Phase 3* below establishes why, so only the unreachability rationale is retained. Unreachability makes deferring the decision safe; it does not make the hazard mild |

## The hazard

`Q` = 64, two events on **different** controls, established by evaluation rather than by argument:

- a control-rate automation stamped at sample 63 takes effect at the first quantum boundary at or after its render
  position, which is 64, and so lands **before** a sample-rate note edge at sample 65;
- defer the automation to 127 under ADR-0043's `+Q` rule, and its effect begins at boundary 128 — **after** that note.

The order the plan declared is reversed, and no same-control repair sees it: the two events share no `(node, control)`
pair, so any same-control mechanism ADR-0044 may accept cannot relate them — that record is still `Deferred` and has
selected nothing. `ADR-0023` does not reach it either, since 65 and 128 are not a tie.

**This refutes an argument ADR-0044's first draft made** — that `SOUND-INV-016`'s quantum-boundary rule for
control-rate responses closed most of the cross-control residual, because a pair less than one quantum apart was never
ordered by sample position anyway. The boundary rule does not protect such a pair; it relocates it. That correction is
recorded in ADR-0044's survey and is the reason this record exists rather than a sentence there.

## Why this gates Phase 3

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

**What follows.** Phase 3 may not close with this unresolved, and Phase 3 implementation may not begin before this is
`Accepted`, on the same footing as ADR-0044 and ADR-0022. That does make it a third prerequisite where the scope split
was meant to avoid adding one — `PROCESS.md` warns that replacing one prerequisite with another is not progress. The
split is still worth keeping, because the two questions have different candidate spaces and ADR-0044's is nearly
settled while this one is empty. But it must be recorded as **what it is**: the survey found the problem larger than
the record assumed, not smaller.

## Decision

**Deferred to the Phase 3 entry gate.** One constraint holds meanwhile: **no specification or review may claim that V2
preserves declared cross-control order under deferral.** Whatever mechanism ADR-0044 eventually accepts is bounded to
same-control order — that record is still `Deferred` and has selected nothing, so nothing here may be read as deciding
it — and a document that generalised such a mechanism would be asserting this record's answer before it exists.

## Revisit conditions

Revisit when any of these becomes true:

- a candidate mechanism is proposed that does not require an audio-thread dependency-graph walk;
- a phase exit, specification, or external consumer needs a general order-preservation promise;
- ADR-0044's F2 frame is revisited for another reason, since the audio-thread dependency-graph refusal is what empties
  this record's candidate space.

## Review

Reviewer: opened as part of ADR-0044's survey repair transaction, which is where the hazard was established and where
the maintainer's scope decision assigned it here. Two independent reads then corrected this record itself: the first
established that it must gate Phase 3 rather than sit ungated, and the second found that the gate had been declared
here without reaching `master-plan.md`, `NOW.md` or the host-profile specification, and that the *What makes it safe*
row still called the symptom transient after the body had refuted that. Both are repaired above.

**This record is dissolved, not merely answered, if an ADR-0043 successor reopens that record's Option A**: with no
deferral there is no deferral-induced cross-control inversion. ADR-0044's survey records why that path is live.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
