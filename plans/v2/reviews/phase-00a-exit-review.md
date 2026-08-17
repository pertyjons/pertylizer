# REV-P00A: Phase 0A Exit Review

| Field                    | Value                                                                     |
|--------------------------|----------------------------------------------------------------------------|
| ID                       | REV-P00A                                                                   |
| Status                   | Accepted                                                                  |
| Phase                    | 00A                                                                        |
| Created                  | 2026-08-15                                                                 |
| Last reviewed            | 2026-08-15                                                                 |
| Reviewed source revision | V1 boundaries `54cd6d3f` and `29c22ef4`, plus the workflow-reset change that lands this review — the commit introducing EVD-0006, EVD-0007, the three `resource_limit_probe_` tests, and CORPUS-0006..0010 |
| Related phase tracker    | [Phase 0A tracker](../phases/phase-00a-baseline-and-render-contracts.md)   |

## Current review state

The original evaluation below rejected Phase 0A because two gates had been interpreted as open-ended completeness
claims. The master plan now fixes their source revisions and bounded inputs, EVD-0006 supplies the executable probe,
and five former corpus gaps are now fixtures. A second independent review found that ADR-0039 confused absence of a
workspace caller with unreachability, did not enforce its later decision in the authoritative master plan, and mapped
a constant Control YAMS fixture to a cadence correction it cannot observe. Those findings are now corrected. ADR-0039
and the last resource row remain open for Phase 10E, not as an added Phase 0A exit condition.

The repository quality gate passes. The final independent pass found no actionable defect after two preceding passes'
authority, citation, publication, and serialized-contract findings were corrected. The historical pass record is
retained because it is the authority for the corrections that led here; it is not current task status.

## Review scope

- **Covered.** The Phase 0A exit gates in the
  [master plan](../master-plan.md#phase-0a-baseline-limits-and-render-core-contracts); the phase's seven tasks; the
  decision register entries targeted at 0A; the [resource ledger](../inventories/resource-limits.md); the
  [host profile specification](../specs/spec-host-profile-and-render-limits.md); evidence records EVD-0001 to EVD-0007;
  the three executable resource-probe tests.
- **Not covered.** Phase 0B, which has its own tracker and gates and runs in parallel. V1 behaviour outside the audit's
  reach. Any V2 implementation, of which none exists yet.
- **Source reads.** Current source citations remain in the resource inventory and evidence records, where citation
  guards check them. The dashboard, phase tracker, and host-profile specification link to those authorities instead of
  maintaining independent source-line copies.

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
  input while defining neither an error nor an overflow behaviour. At the time of this scoping review the master-plan
  signature returned `()` and left an unchecked caller precondition. The current contract corrects that finding:
  preparation returns `CompileError`, and `Renderer::render` returns `Result<(), RenderError>` for per-call violations.
  A `debug_assert` may assist development but does not define release behaviour.
- **`event_queue_capacity` is withdrawn from the profile entirely.** It was admitted through `LIMIT-0013`'s
  `HostProfile` ownership, and ADR-0038 moves that entry to `N/A — removed` because it has no in-workspace production
  caller or validated delivery requirement. The API is public, so external use is unknown and removal is an explicit
  compatibility break. Carrying a V2 capacity derived from four rings without in-repository production evidence would
  have propagated an unvalidated shape.
- **The specification edit is a contract change and was reviewed as one.** See
  [*Historical external review of the scoping change*](#historical-external-review-of-the-scoping-change) — the pass
  found ten defects in it, five P1.

## Historical external review of the scoping change

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

### Final gate review of the workflow reset

The first independent review of the complete workflow-reset change found four P1 defects. They were reported before
being corrected:

| Finding | Correction |
|---------|------------|
| CORPUS-0006 claimed left/center/right panning while prior voices accumulated across notes | An amplitude envelope now gates each note; a render-level test verifies left, centered, and right windows |
| CORPUS-0009 had no onset after the 120 BPM step it claimed to cover | A sixth onset now follows the step; a timing test verifies the ramp intervals and post-step interval |
| `shared-patch-or-instrument` was recorded as blocked although V1 can share one deterministic instrument across tracks | CORPUS-0010 exercises two track-local gains through one shared instrument; a render-level test verifies the gain ratio |
| The ten-case corpus had no matching determinism, CPU, memory, or timing baseline | EVD-0007 records ten-case determinism plus complete 40-cell CPU and timing/RSS matrices |

These corrections subsequently passed the repository gate and independent re-review. This table remains the durable
correction record; the later result, not the table itself, is what accepts them.

### Second independent review of the workflow reset

The next independent pass found two P1 authority/contract defects and one P2 evidence defect. They were reported before
correction:

| Finding | Severity | Correction |
|---------|----------|------------|
| ADR-0039 treated no workspace caller as proof that the public `EngineHub` path was unreachable | P1 | ADR-0039 and both inventories now distinguish bounded workspace evidence from unknown external use and record initial omission as an explicit compatibility break |
| ADR-0039 promised a later successor without placing a gate in the authoritative phase plan | P1 | Phase 10E now requires an accepted successor before implementation and names the contract dimensions it must settle |
| CORPUS-0007's constant `out1 = 0.65` could not observe control-rate cadence | P2 | The unsupported correction claim was removed; Phase 7 instead requires a time-varying partition-invariance test |

These corrections were initially treated as reopening the Phase 0A resource gate because ADR-0039 is `Proposed` and
`LIMIT-0017` is `Investigating`. A later authority review disproved that conclusion: the master gate requires a
proposed rule and diagnostic plus the probe, not an accepted disposition. The open lifecycle states remain visible and
owned by Phase 10E without blocking Phase 1.

## Required decisions

| ADR | Required status | Actual status | Result |
|-----|-----------------|---------------|--------|
| ADR-0001 | `Accepted` | `Accepted` | Pass |
| ADR-0021 | `Accepted` | `Accepted` | Pass; ADR-0038 supersedes three named clauses and ADR-0021 remains authoritative elsewhere |
| ADR-0037 | `Accepted` | `Accepted` | Pass; Phase 2 owns remeasurement |
| ADR-0032 | `Accepted` | `Accepted` | Pass |
| ADR-0022 | `Accepted`, or bounded deferral | `Deferred` | Pass; Phase 3 gate, owner, constraints, and missing evidence are recorded |
| ADR-0028 | `Accepted`, or bounded deferral | `Deferred` | Pass; Phase 4 gate, owner, constraints, and missing evidence are recorded |
| ADR-0038 | Supporting correction discovered by this review | `Accepted` | Pass; four independent passes reached the semantic stop condition before acceptance |
| ADR-0039 | Explicit initial omission of the public multi-client hub | `Proposed` | Not required by the Phase 0A gate; Phase 10E owns independent review and the enforced successor decision |

## Inventory closure

| Inventory/scope | Unclassified entries | Evidence | Result |
|-----------------|---------------------:|----------|--------|
| Resource limits | 1 of 76 | [Inventory](../inventories/resource-limits.md), [EVD-0005](../evidence/phase-00a/EVD-0005-resource-ledger-use-site-audit.md), [EVD-0006](../evidence/phase-00a/EVD-0006-resource-limit-runtime-probe.md), ADR-0039 | **Pass for Phase 0A.** All 76 rows have the proposed rule and diagnostic the master gate requires, and the probe passes. `LIMIT-0017` remains `Investigating` for its stricter inventory lifecycle and Phase 10E decision |
| State ownership | Not assessed | — | N/A; Phase 0B owns it |
| Capabilities | Not assessed | — | N/A; Phase 0B owns it |
| Identities | Not assessed | — | N/A; Phase 0B owns it |

## Accepted-ADR comparison map

This table evaluates only semantic changes named by ADRs already accepted at the gate's fixed revision. It does not
claim that every future V2 decision already has a fixture.

| Accepted decision | Named comparison disposition |
|-------------------|------------------------------|
| ADR-0001 and ADR-0037 — fixed internal quantum independent of caller blocks | `subtractive-voice` and `mod-matrix-patch`; their `intentional-correction` claims name sample timing and control-rate independence. CORPUS-0007 preserves constant-script loading/routing only and makes no cadence claim |
| ADR-0021 — refusal instead of silently changing authored topology or overrunning admitted budgets | `instrument-inserts` names the topology refusal; `polyphonic-voice-stealing` covers an admitted voice ceiling; resource-bound differences use these named classes rather than generic comparison failure |
| ADR-0032 — absolute sample time and positioned events | `subtractive-voice` names the sample-accurate onset correction; the `tempo-map-arrangement` fixture classifies the later ramp-position law as `unsupported-scope` |

## Exit gates

Gate text is copied from the current [master plan](../master-plan.md#phase-0a-baseline-limits-and-render-core-contracts).

| Gate | Evidence or named test | Result |
|------|------------------------|--------|
| The corpus and comparison command run headlessly; every category at `54cd6d3f` is one executable fixture or one explicit reproducibility gap | `pertylizer compare`; `cargo test -p pertylizer --test corpus_manifest`; EVD-0001; manifest case/gap partition | **Pass.** Ten categories have deterministic fixtures; sampler has a required owner and a concrete project-plus-asset reproducibility problem. Behaviour tests guard the corrected panner, tempo-step, and shared-instrument claims |
| Every intentional change named by an ADR accepted at `54cd6d3f` maps to a comparison category | Accepted-ADR comparison map above; manifest claim classes | **Pass.** The gate requires a named disposition, not a fixture for a later mechanism |
| CPU, memory, timing, and determinism baselines are reviewable | EVD-0001, EVD-0002, EVD-0003, EVD-0007 | **Pass.** EVD-0007 supplements the historical records with ten-case determinism and complete 40-cell CPU and timing/RSS matrices |
| ADR-0001, ADR-0037, ADR-0021, and ADR-0032 are accepted | Required-decisions table | **Pass** |
| ADR-0022 and ADR-0028 are accepted or boundedly deferred | Required-decisions table and both ADRs | **Pass** |
| At `29c22ef4`, every cap found by the three declared methods appears once with a proposed V2 rule and diagnostic; the three-axis probe passes and later findings follow the bounded reopening policy | Resource inventory; EVD-0005; EVD-0006; debug and release probe commands | **Pass.** All 76 rows contain the required fields, the probe passes in both build modes, and no observation triggers ADR-0021's taxonomy revisit |

## Quality gates

### Third independent review

| Finding | Severity | Correction |
|---------|----------|------------|
| The debug oversized-callback probe observed a panic but did not prove the allocation happened only in release; voice-buffer growth precedes the fixed-effect assertion | P2 | Debug now inspects the voice buffer's length and increased capacity after unwind, independently of allocator events from panic machinery; release also counts the allocation. EVD-0006 and `LIMIT-0001` record the ordered behaviour |
| EVD-0007 retained aggregates but its reproduction section used placeholder render/compare commands and omitted the exact capture and aggregation procedure | P2 | A permanent standard-library harness now owns every command, raw JSONL capture, matrix check, aggregation, and determinism assertion |
| ADR-0039 copied a source citation for a normative premise instead of consuming its authoritative inventory rows | P2 | The ADR now links `LIMIT-0017` and `CAP-0017` as the fact authorities and contains no copied source citation |

### Fourth independent review

| Finding | Severity | Correction |
|---------|----------|------------|
| ADR-0038 condition 3 permits a data-paired omission count, but its Risks section still said every count outside the structured report fails | P1 | The Risks control now repeats the same consumer-attributable two-path rule and names `LIMIT-0021` as the valid data-paired case |
| Counting allocations across a caught panic includes panic-runtime allocations and therefore did not prove pre-panic voice-buffer growth | P2 | Debug now records the voice buffer's length and capacity before the call and verifies both after unwind; release additionally counts allocation events |

### Gate-authority and evidence-publication review

| Finding | Severity | Correction |
|---------|----------|------------|
| The exit review made ADR-0039 acceptance and a `Classified` inventory state a Phase 0A condition that the master gate does not contain | P1 | The gate is now evaluated verbatim: all 76 rows have proposed rules and diagnostics and the probe passes; ADR-0039 and `LIMIT-0017` remain open for Phase 10E |
| EVD-0007 published each permanent CSV before later collections and validations completed | P2 | All artifacts are generated in same-filesystem staging and the complete artifact directory is published with one atomic exchange after validation |
| EVD-0007 wrote non-permanent raw JSONL captures into the checked-in evidence directory | P2 | Raw captures remain in the temporary staging directory and are removed with it on success or failure |

| Command/check | Current result |
|---------------|----------------|
| `cargo test --workspace resource_limit_probe_ -- --nocapture` | **Pass** — 3/3 named probe tests |
| `cargo test -p synth_engine --release resource_limit_probe_oversized_callback_exposes_build_mode_failure -- --nocapture` | **Pass** — release allocation path observed |
| `cargo test -p pertylizer --test ledger_citations` | **Pass** — 4/4 citation/authority guards |
| `cargo fmt --check` | **Pass** |
| `cargo build` | **Pass** |
| `cargo clippy --workspace --all-targets` | **Pass** |
| `cargo test --workspace` | **Pass** |
| `cargo doc --workspace --no-deps` | **Pass** |
| Independent review of the bounded gate/probe change | **Pass** — final pass found no actionable defect |

### Final independent re-review

The first re-review after the transactional publication correction found one P2 citation drift: the authoritative
`LIMIT-0001` row still pointed at the pre-probe line for `mono_buffer.resize`. The inventory and its ADR consumer were
updated, and the citation guard passed 4/4.

The next re-review found one P1 authority error and one P2 serialized-contract gap. The resource-owner totals had
applied proposed ADR-0039 before acceptance; they now retain ADR-0038's accepted 8/10 distribution and label 7/11 as
provisional. The newly required planned-category `owner` key now has a parse-level missing-field rejection test.

The final independent pass found **no actionable defect**. It reran the workspace tests, Clippy, formatting, rustdoc,
targeted corpus and citation tests, and the oversized-callback probe in debug and release. No contract clause changed
in that pass, so the working agreement's semantic stopping condition is satisfied.

## Deviations and residual risks

| Item | Impact | Owner/control |
|------|--------|---------------|
| One corpus category remains a gap | It cannot detect regressions until its input bundle becomes reproducible | Phase 0B bundle fixtures own the sampler project-plus-asset boundary |
| EVD-0005 is retrospective and cannot prove unnamed behaviour absent | Its conclusion is bounded to its declared methods | EVD-0006 probes three runtime axes; later findings follow the new-finding policy |
| The oversized-callback probe is topology-specific | A different graph may reach truncation or allocation before the fixed effect buffer | `LIMIT-0001` records all three known outcomes and V2 treats any oversized callback as one terminal stream fault |
| Source-line citations can drift onto valid neighbouring code | A citation may still misdirect despite shape checks | Current source claims remain in the guarded inventory; derived documents are forbidden from copying them |
| HOST-INV-021 remains deferred | Phase 3 still must design ingress and deferred storage before live scheduling | The initial Phase 1 renderer may not implement the deferred mechanism |

## Outcome

Outcome: `Accepted`

All six bounded Phase 0A evidence gates pass. ADR-0039 and `LIMIT-0017` retain their open lifecycle states for Phase 10E
without blocking Phase 1. The final independent pass required no contract, authority, safety, or correctness change,
so Phase 1 may begin.
