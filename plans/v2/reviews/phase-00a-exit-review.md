# REV-P00A: Phase 0A Exit Review

| Field                    | Value                                                                     |
|--------------------------|----------------------------------------------------------------------------|
| ID                       | REV-P00A                                                                   |
| Status                   | Draft                                                                      |
| Phase                    | 00A                                                                        |
| Created                  | 2026-08-15                                                                 |
| Last reviewed            | 2026-08-15                                                                 |
| Reviewed source revision | `29c22ef4`, plus the uncommitted documentation change this review is part of |
| Related phase tracker    | [Phase 0A tracker](../phases/phase-00a-baseline-and-render-contracts.md)   |

## Why this review exists now, before the phase can pass

**This review does not close Phase 0A, and its `Draft` status is not a formality.** Two gates fail, and one of them
fails for reasons no amount of writing fixes: the reference corpus covers five of the master plan's eleven categories.

It is opened now because **one scoping decision was blocking work that does not depend on the corpus**, and the phase
tracker records that the call belongs here rather than to the task it constrains. P00A-T005 had accumulated eleven
review passes and four substantive blockers, and the open question was whether the task closes narrowly with the
blockers moved to Phase 3, or stays open until all four are resolved. Everything else in the phase queued behind that
answer. The decision is recorded in [*Scoping decision*](#scoping-decision) below; the gate evaluation is recorded
honestly alongside it and says `Fail` where it must.

## Review scope

- **Covered.** The Phase 0A exit gates in the
  [master plan](../master-plan.md#phase-0a-baseline-limits-and-render-core-contracts); the phase's seven tasks; the
  decision register entries targeted at 0A; the [resource ledger](../inventories/resource-limits.md); the
  [host profile specification](../specs/spec-host-profile-and-render-limits.md); evidence records EVD-0001 to EVD-0005.
- **Not covered.** Phase 0B, which has its own tracker and gates and runs in parallel. V1 behaviour outside the audit's
  reach. Any V2 implementation, of which none exists.
- **Source reads.** Every source claim newly made in this change was re-resolved at `29c22ef4`. Claims carried over
  from earlier passes are marked as such where they matter and are not re-attested here.

## Scoping decision

**Decision: P00A-T005 closes on the master plan's field list. Two of its four blockers move to Phase 3; two do not and
are settled inside Phase 0A.**

### The ground

The Phase 0A exit gate contains **no bullet requiring the host profile specification to reach any status.** Its six
bullets require the corpus and comparison command to run headlessly, the intentional semantic changes to have named
comparison categories, the baselines to be saved, four ADRs `Accepted`, two `Accepted`-or-`Deferred`, and every fixed
cap to appear once in the resource inventory. What binds P00A-T005 is instead the phase's **Work** list, which asks for
"an **initial** `HostProfile`/`RenderLimits` contract" over a named field list — maximum host block, layouts, voices,
nodes, event fan-out, channels, buses, sends, telemetry taps, recording buffers, prepared memory, script work, and
ADR-0032's forward event horizon.

**Renderer-ingress capacity and the deferred store are not in that list.** They are not in it because they are not
V1 concepts the plan was cataloguing: they were discovered while specifying a deferral mechanism, after the premise
that mechanism rested on turned out to be false. Nor are they derivable from what is in the list — the specification
establishes that no store bound follows from the existing fields, because deferral frees the upstream slot and note
expansion multiplies one released event into many.

### What was rejected, and why it matters

The framing this review was asked to consider was "close narrowly, move all four blockers to Phase 3". **Two of the
four could not move**, and the reason is gate bullet six: *every current fixed RT/resource cap appears once in the
resource inventory with a proposed V2 admission rule and a user-visible overflow diagnostic; no unexplained silent
truncation is accepted as baseline behavior.* Four ledger rows sat outside `Classified` on precisely the two questions
those blockers name. Moving them to Phase 3 would have moved a Phase 0A gate obligation into a later phase — which is
not a scoping decision, it is dropping a gate.

The other correction is to the *reason* usually given for the narrow close. "Phase 1 compiles rather than renders live"
is imprecise and should not be relied on: **Phase 1 does render, offline.** Its API accepts a `TimedEvents` span, its
harness renders caller-selected frame counts, and its exit gate requires deterministic rendering with an
allocation-free render loop and no silent clipping of event fan-out. The narrow close is defensible on the gate text
and the Work list, not on Phase 1 being inert.

### Disposition

| Blocker | Disposition | Basis |
|---------|-------------|-------|
| **1. ADR-0001 clarification or successor** — when clause 16's late condition is evaluated, and whether a quantum may defer at all under clauses 12 and 14 | **Phase 3**, as an entry-gate obligation | Not in the Work list's field set; unimplementable before a scheduler exists. Recorded in the master plan's Phase 3 section, not only here |
| **2. ADR-0021 clarification** — the `Live bounded queue` class given to engine-egress rings | **Phase 0A**, settled by [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) | Gate bullet six. The scope is also wider than the blocker stated: `LIMIT-0017` has the same class mismatch as `LIMIT-0013`, so the correction is the general engine-egress rule, not one row |
| **3. `LIMIT-0014`'s GUI/OSC split** | **Phase 0A**, settled by ADR-0038 part 4 | Gate bullet six, and the fields it feeds — event fan-out, telemetry taps — are in the Work list |
| **4. V2's renderer-ingress streams, and separately a bound and exhaustion policy for the deferred store** | **Phase 3**, as two work items and two gate bullets | Not in the Work list; V1 has nothing to carry over. They are not one item: deferring frees the upstream slot, so ingress capacity bounds arrival rate rather than backlog |

### What the decision costs

- **HOST-INV-021 is `Deferred`, not deleted.** Its identifier is retained. Four external passes are indexed to it, and
  reissuing the mechanism under a new number in Phase 3 would strand that record.
- **`max_events_per_quantum` stays normative.** It is `LIMIT-0075`'s successor — V1's uncapped per-block `Vec` that
  grows inside the audio callback — and dropping it with the mechanism would leave V2 with the same unbounded
  allocation. Only the runtime behaviour on exceeding it moves.
- **Phase 1's event boundary needed a rule in the mechanism's absence**, or the public renderer would accept arbitrary
  input while defining neither an error nor an overflow behaviour. The specification states one in two halves: a plan
  whose statically knowable per-quantum count exceeds the field is **refused at preparation**, which the existing
  `CompileError` path already expresses; and the per-call span is a **caller precondition**, which the master plan's
  `Renderer::render` — returning `()` — cannot report. **The second half is not enforceable as written**, and the
  specification says so and lists it as blocking for Phase 1 rather than presenting it as settled. Phase 1 owes either
  a release-active mechanism — a fallible signature being the obvious one. A `debug_assert` does not qualify: it
  compiles out of the build that runs.
- **`event_queue_capacity` is withdrawn from the profile entirely.** It was admitted through `LIMIT-0013`'s
  `HostProfile` ownership, and ADR-0038 moves that entry to `N/A — removed` because its channel is never constructed
  outside its own tests. Carrying a V2 capacity derived from four rings V1 never ran would have propagated an
  unvalidated shape.
- **The specification edit is a contract change and was reviewed as one.** See
  [*External review of this change*](#external-review-of-this-change) — the pass found ten defects in it, five P1.

## External review of this change

An independent pass ran over the change this review is part of — scoped to the change, not to the host-profile
specification, which has had eleven passes and whose recent ones mostly found consequences of their own predecessors'
corrections. **It found ten defects, five of them P1**, and the two most consequential were both the failure mode this
phase has recorded four times: a claim recorded without reading what feeds it.

| Finding | Severity | What it changed |
|---------|----------|-----------------|
| **`LIMIT-0017` is not written from the render path at all.** `EngineHub::broadcast_event` takes an `RwLock` and a per-client `Mutex`, so it cannot run on the audio thread — and it has **no caller in the workspace**. ADR-0038's first draft asserted all three queues were audio-thread producers | P1 | ADR-0038 gains a fourth condition for off-render-path egress: dropping there is a *choice* and the record must say why blocking or backpressure were rejected. `LIMIT-0017`'s row records that its hazard cannot currently fire, and its loss policy is handed to ADR-0029 |
| **The specification still carried `event_queue_capacity`**, a four-ring field admitted through `LIMIT-0013`'s ownership, which ADR-0038 had just removed. The change had corrected `event_egress_capacity` and never checked which *other* fields cite `LIMIT-0013` | P1 | The field is withdrawn |
| **HOST-INV-021 was not actually narrowed out.** Marking the invariant and its conformance rows left the failure taxonomy, the release-window semantics, the five-behaviour count, and the real-time constraints all still asserting deferral normatively | P1 | Every one of those carries an explicit deferral marker, and the negative constraint that survives — no allocation to absorb an over-full quantum — is stated separately |
| **The Phase 1 bounded-span rule was not implementable** through an API returning `()`, and was wrongly attributed to HOST-INV-005, which governs field provenance rather than refusal | P1 | Split into a preparation-time refusal that works and a per-call precondition that does not, with the gap listed as blocking for Phase 1 |
| **Partial supersession had no lifecycle state.** `decisions/README.md` says a changed decision produces a successor and marks the old record `Superseded`; there was no clause-level form | P1 | `Superseded in part` is documented in the lifecycle and the register's vocabulary, with a stated test for when whole-record supersession is the right tool instead. ADR-0038's `Supersedes` field now names all three replaced clauses, including the decision driver it had replaced silently |
| Four `Investigating` rows, not three — this review said three and called `LIMIT-0017` resolved | P2 | Corrected here and in the dashboard: 72 of 76 `Classified` |
| ADR-0038's own driver conflated the two rings it exists to separate | P2 | Corrected |
| The observational/custodial test was claimed decidable from the payload type; `EngineEvent` holds both kinds | P2 | The claim is withdrawn and replaced by an enforceable control: an explicit closed classification that fails to compile when a variant is unclassified |
| EVD-0005 called two disjoint populations a control, then drew a causal conclusion its own limitations section rules out — plus an inverted bound | P2 | The control claim is withdrawn, the attribution is withdrawn, and the bound is stated correctly |
| Tracker and dashboard kept present-tense statements contradicting the new state | P2 | Corrected |

**Every finding in this table is a defect in the change as originally written**, because this pass ran before any
correction existed. That is what makes the later passes' counts interpretable: each of those was scoped to the
previous round's corrections, so every finding in them is by construction a defect in a correction. No per-row
provenance column is needed; the scope of each pass supplies it.

**The pass also confirmed** the owner arithmetic (76 entries; 28 `HostProfile` / 48 elsewhere, split 12/10/8/7/6/5) and
the source claims about `prioritized_event_channel`, `EVENT_BUFFER_SIZE`, and `LIMIT-0015`'s four channels.

### The second pass, over the corrections

**A second narrow pass reviewed only the ten fixes, and found seven more defects, two P1.** Both P1s were in
corrections, not in the original change — which is the outcome this project has now recorded six times.

| Finding | Severity | What it changed |
|---------|----------|-----------------|
| **ADR-0038's definition of engine egress excluded `LIMIT-0017` from its own rule.** The first correction defined egress as "written from the render path" and then added a fourth condition for off-render-path queues — a condition the definition made unreachable. Worse, the record called condition 4 unmet and settled the class anyway, so accepting ADR-0038 would have closed a row whose loss policy nobody has chosen | P1 | The definition is now about direction alone; position selects which conditions apply. **An entry whose condition 4 is unmet is not classified by this record**, so `LIMIT-0017` stays `Investigating` past ADR-0038's acceptance — 75 of 76, not 76 |
| **The handoff of `LIMIT-0017`'s loss policy to ADR-0029 was asserted, not checked.** ADR-0029's registered topic is host configuration and remote authorization, which does not own queue loss semantics | P1 (same finding) | Withdrawn. The multi-client hub's delivery contract has **no owner in the register**, which is recorded as a finding and as ADR-0038 follow-up work rather than filled in with a plausible-looking cell |
| **HOST-INV-021 still had a binding clause after being declared wholly deferred** — "the admission order binds meanwhile" — plus HOST-INV-009 counting deferral among five in-force behaviours, and the release-window prose stating a failure behaviour | P1 | All three carry the deferral; the "binds meanwhile" claim is withdrawn, and what actually survives is the single negative constraint |
| **Partial supersession took effect before the successor was authoritative.** Only accepted decisions constrain implementation, yet a `Proposed` ADR-0038 was marking accepted clauses as replaced — an authority gap worse than the missing lifecycle state it was introduced to fix | P2 | The lifecycle gains rule 9: partial supersession takes effect **on acceptance**; a pointer is a notice, never a repeal. The specification now says plainly that `event_egress_capacity` rests on a `Proposed` record and cannot be `Current` before it |
| **The `debug_assert` option cannot define release behaviour**, so it did not close the unchecked precondition it was offered for | P2 | Withdrawn; the options are a fallible signature or another release-active mechanism |
| The four-row correction had not reached the dashboard or four sections of this review | P2 | Corrected |
| Three-clauses-not-two had not reached the register, ADR-0038's consequences, the tracker, or this review; two unresolved-questions rows still routed ADR-0021's evidence and class to Phase 3 | P2 | Corrected; both rows are struck and marked resolved in Phase 0A, subject to ADR-0038's acceptance |
| Pre-split arithmetic in the specification's non-goals and a truncation-register count in the tracker | P3 | Corrected |

**Fix 2 was confirmed to have landed cleanly** — no active sizing or mapping depends on `event_queue_capacity`, and the
master plan's event-fan-out item stays answered by `max_fan_out_per_port` — as were the ADR-0038 driver fix, the
observational/custodial control, and the EVD-0005 corrections.

### The third pass

**A third pass over the second round of corrections found five more, two P1 — and it answered the stopping question
explicitly: not met.** Its own words: *"Because findings 1-3 require contract or decision-ownership corrections, this
pass does change contract clauses."*

| Finding | Severity | What it changed |
|---------|----------|-----------------|
| **The direction-only definition — itself a fix from pass two — captured `LIMIT-0021`**, a render-produced GUI-consumed visualization ring already `Classified` under a different diagnostic contract. Accepting ADR-0038 would have put an already-settled row in violation of it | P1 | **Condition 3 now admits two forms**: the structured diagnostics report, or a count that travels with the data. `LIMIT-0021`'s `read_samples_into` already does the second, under `#[must_use]`, and it is the *stronger* diagnostic — it says which window has a gap, not merely that something was lost. The record names `LIMIT-0021` as the worked example rather than leaving the widening implicit |
| **The withdrawn ADR-0029 handoff was still live in two places** — `LIMIT-0017`'s ADR cell and this review's inventory-closure table — contradicting the record that withdrew it | P1 | Removed from both. The cell now records that no ADR owns the loss policy and that ADR-0029 was named and withdrawn |
| **"Three clauses, not two" still stale in the register, the ledger and the tracker**, and ADR-0038's own introduction mapped the supersessions to the wrong parts — it is part 1 and part 3, not parts 3 and 4 | P2 | Corrected; the introduction now states which part supersedes what and that parts 2 and 4 supersede nothing |
| **The closure arithmetic was wrong**: the ledger said acceptance yields 73 of 76 where 72 plus three is 75, and the tracker and dashboard both still said P00A-T004 was open on completeness alone | P2 | 75 of 76. P00A-T004 has **two** blockers — completeness and `LIMIT-0017` |
| A broken ordinal introduced by the deferral edit | P3 | Corrected |

**The pass also confirmed** that lifecycle rule 9 is coherent with rules 7-8 and the status vocabulary, that the
specification correctly bars `Current` while ADR-0038 is `Proposed`, and that `LIMIT-0012` remains ingress while
`LIMIT-0015` and `LIMIT-0016` remain custodial — the definition-width check the pass was asked to run.

### The fourth pass, and the stopping condition

**A fourth pass over the third round of corrections found six defects — four P2, two P3, and no P1 — and none of them
changes a contract clause.** In its own words: *"no finding above changes a contract clause. This pass therefore ends
the correction loop under the document set's own rule, despite finding editorial and consistency defects."* That is
the stopping condition, and it is the first time in this document's history that it has been met.

The six were: the `LIMIT-0021` source correction not propagating to the ledger row and the `telemetry_ring_frames`
field row, both of which still described the pre-`bac88c0c` state; ADR-0038 still calling four entries the complete
egress set when its definition reaches seven; one surviving "two superseded clauses" in the ledger; the two-blocker
closure correction stale in three present-state summaries; a citation range that did not contain the lines it cited;
and the method claim below being unauditable. All are corrected.

The pass also ran the check this round's own lesson demanded, and **that is the result worth keeping**: it swept all
76 ledger rows against ADR-0038's widened definition and found seven that satisfy it, not the four the record named.
The three extra — `LIMIT-0015`, `LIMIT-0016`, `LIMIT-0021` — all turned out consistent with the record's conditions,
so the widening was safe; but it was safe by luck until somebody checked. ADR-0038 now carries that sweep as a table.

### What four passes say about the method

**Twenty-eight findings, nine P1: ten in the original change and eighteen in corrections.**

**An earlier revision of this section said "two of twenty-two", and that was simply wrong** — the first pass reviewed
the change before any correction existed, so all ten of its findings are defects in the original by construction. The
error survived two chat reports and into this document, and it was not caught by review: the fourth pass flagged the
claim as *unauditable* rather than false, and it was found only when someone tried to tag the rows to make it
checkable. **A number nobody can check is not a weaker claim than a wrong one; it is how a wrong one survives.** That
is this exercise's own failure mode reproduced in its own write-up.

Findings per pass: ten, seven, five, six. **The count did not converge; the severity did** — five P1, two, two, none —
and the stopping rule keyed on contract clauses rather than on finding count is the reason this terminated at all. A
rule of "keep going until a pass finds nothing" would still be running.

Three conclusions worth carrying, all cheap:

- **The first pass's lesson was necessary and insufficient.** "Search for every consumer of an entry whose meaning
  changed" catches propagation failures. It does not catch the failure that produced both of pass two's P1s and one of
  pass three's: a *fix* that is locally correct and globally inconsistent with a document the fixer had just edited.
  The check that would catch those is **re-read the corrected clause against what it now contradicts**, not only
  against the finding it answers.
- **Widening a definition to admit one case admits others.** Pass three's first P1 is the clearest instance: a
  definition was widened to include `LIMIT-0017` and silently swept in `LIMIT-0021`, which was already settled
  elsewhere. **A definition change is not finished until every entry it now reaches has been checked against it** —
  which is a specific, runnable step, not a disposition to be careful. Pass four ran it and found seven, not four.
- **State a claim in a form somebody can check, or do not state it.** The "two of twenty-two" error above cost nothing
  because it was about the review rather than about the contract. The same shape inside a contract clause is what
  `LIMIT-0013` was: a number that supported the conclusion its author already held, unaudited for three passes.

**What this says about the method, and it is not flattering.** The change was made by someone who had just written the
rule they were applying, and five of the ten findings are places where the correction did not propagate to a document
the corrector had already edited. That is the same shape as the four earlier corrections. The transferable part is
narrower than "review corrections too": **when a record changes what a ledger entry means, search for every consumer
of that entry rather than the ones you remember touching.** Both P1 propagation failures — `event_queue_capacity` and
the deferral markers — would have been caught by a mechanical search that was never run.

## Required decisions

| ADR      | Required status                                       | Actual status | Result |
|----------|-------------------------------------------------------|---------------|--------|
| ADR-0001 | `Accepted`                                            | `Accepted`    | Pass   |
| ADR-0021 | `Accepted`                                            | `Accepted`    | Pass — three named clauses superseded by ADR-0038, taking effect only when that record is accepted; the record itself is unchanged and remains authoritative for the rest |
| ADR-0037 | `Accepted`                                            | `Accepted`    | Pass — value provisional, re-measurement bound to the Phase 2 exit gate |
| ADR-0032 | `Accepted`                                            | `Accepted`    | Pass   |
| ADR-0022 | `Accepted`, or `Deferred` with owner and evidence gap | `Deferred`    | Pass — Phase 3 entry gate, owner and missing evidence recorded, constraints stated while open |
| ADR-0028 | `Accepted`, or `Deferred` with owner and evidence gap | `Deferred`    | Pass — Phase 4 entry gate, same shape |
| ADR-0038 | Not in the required table; created by this review's scoping decision | `Proposed` | **Outstanding.** Three ledger rows carry its disposition and cannot reach `Classified` until it is accepted; a fourth, `LIMIT-0017`, stays open past its acceptance because its condition 4 has no owner. It is deliberately not accepted in the session that drafted it — the register has two withdrawn same-session acceptances |

## Inventory closure

| Inventory/scope   | Unclassified entries | Evidence | Result |
|-------------------|---------------------:|----------|--------|
| Resource limits   | 4 of 76 | [EVD-0005](../evidence/phase-00a/EVD-0005-resource-ledger-use-site-audit.md); [ledger](../inventories/resource-limits.md) | **Fail, pending ADR-0038's acceptance.** Four rows carry their disposition at `Investigating` — `LIMIT-0013`, `LIMIT-0014`, `LIMIT-0076` and `LIMIT-0017` — because a disposition's supporting evidence must be an accepted decision, so 72 of 76 are `Classified`. Only `LIMIT-0015` is fully resolved, by completing an overflow read the audit had left partial; it needed no new decision. **`LIMIT-0017` is not disposed even then**: its producer is off the render path, so ADR-0038's fourth condition applies, and **no registered decision owns** whether it may drop — ADR-0029 was named and withdrawn, its topic being host configuration and remote authorization. Registering an owner is ADR-0038 follow-up work. So acceptance takes the ledger to 75 of 76, not 76 |
| State ownership   | Not assessed | — | N/A — Phase 0B's scope |
| Capabilities      | Not assessed | — | N/A — Phase 0B's scope |
| Identities        | Not assessed | — | N/A — Phase 0B's scope |

## Exit gates

Gate text copied from the [master plan](../master-plan.md#phase-0a-baseline-limits-and-render-core-contracts).

| Gate | Evidence or named tests | Result |
|------|-------------------------|--------|
| The reference corpus and comparison command can run without a GUI or physical audio device. | `pertylizer compare`; `cargo test -p pertylizer --test corpus_manifest`; [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md) | **Pass** |
| Every known intentional V1-to-V2 semantic change has a named comparison category rather than being treated as generic error. | Corpus manifest claim classes, validated by `corpus_manifest` | **Fail.** The categories exist and are enforced for the five authored corpus cases. The gate is about *every known* change, and six of eleven categories are unauthored — a change that would surface only in `sampler-patch` or `tempo-map-arrangement` has had no opportunity to be named. This cannot pass before P00A-T001 does |
| CPU, memory, timing, and determinism baselines are saved in a reviewable format. | [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md), [EVD-0002](../evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md), [EVD-0003](../evidence/phase-00a/EVD-0003-cpu-memory-timing-baseline.md) | **Pass**, with the scope note that all three cover four of the five corpus cases and that real-time headroom is explicitly not measured |
| ADR-0001, ADR-0037, ADR-0021, and ADR-0032 are `Accepted`. | Required-decisions table above | **Pass** |
| ADR-0022 and ADR-0028 are either `Accepted` or `Deferred` to their named Phase 3 and Phase 4 entry gates with an owner and outstanding evidence recorded. | [ADR-0022](../decisions/ADR-0022-hardware-time-mapping.md), [ADR-0028](../decisions/ADR-0028-long-running-job-contract.md) | **Pass** |
| Every current fixed RT/resource cap appears once in the resource inventory with a proposed V2 admission rule and a user-visible overflow diagnostic; no unexplained silent truncation is accepted as baseline behavior. | [Ledger](../inventories/resource-limits.md), [EVD-0005](../evidence/phase-00a/EVD-0005-resource-ledger-use-site-audit.md), ADR-0021 §3, ADR-0038 | **Fail**, for two independent reasons. (a) Four rows are `Investigating`. Three wait on ADR-0038's acceptance and are mechanical; `LIMIT-0017` does not resolve with that record, because its producer is off the render path and no registered decision owns whether it may drop. (b) **Completeness is not demonstrated and cannot be by the methods used.** Every pass so far is a search or a read; a truncation that is both unnamed and undocumented is invisible to all of them. Four separate entries were each found by a method the previous one could not see. ADR-0021's executable probe is the only instrument for this, and it has not been built |

## Quality gates

| Command/check                            | Environment | Result  | Evidence |
|------------------------------------------|-------------|---------|----------|
| `cargo fmt --check`                      | Linux x86-64 | **Pass** | Documentation-only change; run to confirm nothing under `crates/` moved |
| `cargo build`                            | Linux x86-64 | **Pass** | As above |
| `cargo clippy --workspace --all-targets` | Linux x86-64 | **Pass** | As above |
| `cargo test --workspace`                 | Linux x86-64 | **Pass** | As above; this is what runs the citation guards below |
| `cargo doc --workspace --no-deps`        | Linux x86-64 | **Pass** | As above |
| `cargo test --workspace --test ledger_citations` | Linux x86-64 | **Pass** (3/3) | The three citation guards over the ledger. It **failed first** on eight stale citations introduced by this change and passes after they were corrected, which is what makes the green run informative |

## Deviations and residual risks

| Item | Impact | Owner/task | Acceptance basis |
|------|--------|------------|------------------|
| ADR-0038 is `Proposed`, so three ledger rows cannot be `Classified`; `LIMIT-0017` stays open past its acceptance for a different reason | Blocks gate bullet six's mechanical half | P00A-T004 | Accepted deliberately: same-session acceptance has been withdrawn twice in this register. An independent pass runs before acceptance |
| ADR-0038 is the register's first **partial** supersession | A reader of ADR-0021 must know three of its clauses are replaced once ADR-0038 is accepted, and still binding until then | ADR register | ADR-0021 is immutable, so the alternative was re-deciding a record that is still right about everything else. Both directions are linked |
| Blockers 1 and 4 move to Phase 3 | Phase 3 inherits two work items and two gate bullets | Phase 3 entry gate | Recorded in the master plan, which is authoritative for scope and phase order. A note in this review would not have moved them |
| Citation drift onto valid code is known present and unbounded | The ledger's `file:line` citations may misdirect a future reader | P00A-T004 | Measured rather than assumed: only 10 of 76 rows carry the annotated form the strong test checks, and a four-row re-audit found drift in two. Recorded in EVD-0005 |
| The ledger's completeness rests on searches and reads | Gate bullet six cannot pass on the current evidence | P00A-T004 | ADR-0021's own follow-up names the executable probe. EVD-0005 is its fourth argument |
| EVD-0005 is retrospective | Its acceptance criteria were written after the results existed | P00A-T004 | Stated in the record itself rather than glossed. A future ledger pass states its criteria first |
| P00A-T001 covers five of eleven corpus categories | Two gates fail on it | P00A-T001 | Not accepted — this is why the review is `Draft` |

## Outcome

Outcome: `Draft`

**Phase 0A does not pass.** Two of the six exit gates fail: the semantic-change gate and the resource-inventory gate,
the first on corpus coverage and the second on both ADR-0038's status and an unproven completeness claim. No condition
would make this a `Conditionally accepted`, because the completeness question is not bounded — it asks for evidence
that does not exist yet, and the instrument for producing it has not been built.

**What this review does settle** is the scoping question the phase was queued behind, and the record of it is the
section above rather than this outcome. Three of the phase's seven tasks were `Complete` before it. P00A-T004's two
open items are still two — its coverage claim now has EVD-0005, but `LIMIT-0017` replaced coverage as the second
blocker alongside completeness — and P00A-T005 closes on the master plan's field list with its deferred mechanism recorded in
the plan rather than left as an absence.

**What stands between here and an `Accepted` outcome**, in dependency order:

1. An independent pass over this change, then ADR-0038 accepted, then three ledger rows to `Classified` — 75 of 76, not all 76, because `LIMIT-0017`'s loss policy has no registered owner and ADR-0038 declines to invent one.
2. ADR-0021's executable truncation probe — oversized blocks, more than 128 metered channels, more than 32 rack
   stages — which is the only thing that can move gate bullet six's completeness half.
3. Six more corpus categories, several blocked on decisions rather than effort, which is what gate bullet two waits on.

This review is reopened and re-evaluated when those land. It is not edited to make a gate pass.
