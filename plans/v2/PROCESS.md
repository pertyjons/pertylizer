# Core V2 Process

This file is the single authority for how Core V2 work is selected, evidenced,
decided, reviewed, and closed. Repository-wide engineering and commit rules
remain in [`AGENTS.md`](../../AGENTS.md).

## Authorities

| Question | Authority |
|---|---|
| What is active, blocked, and next? | [`NOW.md`](NOW.md) |
| What outcomes are built, and in what order? | [`ROADMAP.md`](ROADMAP.md) |
| What must a phase prove at exit? | Its Part I exit gate linked from [`ROADMAP.md`](ROADMAP.md) |
| What must implementation do now? | Current files under [`specs/`](specs/README.md) |
| Why was a durable choice made? | The individual ADR under [`decisions/`](decisions/README.md) |
| What did an observation or measurement establish? | The EVD under [`evidence/`](evidence/README.md) |
| What exists in V1 and what is its disposition? | [`inventories/`](inventories/README.md) |
| Did a phase pass? | Its accepted review under [`reviews/`](reviews/README.md) |

One fact has one authority. Other documents link to it and state only the
consequence needed by their own scope. Operational files do not copy benchmark
figures, review chronology, or decision rationale.

Claims about current V1 behavior link to the owning inventory row or evidence
record. Current workflow, architecture, and specification documents do not copy
`path.rs:N` citations that would drift independently.

The mechanically guarded active-document set is the top-level navigation and
control files (`README.md`, `PROCESS.md`, `ROADMAP.md`, `NOW.md`, `ADR.md`, the
two stable-path pointers, and `glossary.md`), every V2 directory `README.md`,
and every Markdown file under `architecture/`, `specs/`, and `templates/`.
Decision, evidence, inventory, review, frozen-phase, and legacy-master bodies
remain outside that set because they own evidence or preserve historical text.
The documentation checker uses this one set for both line width and the ban on
copied Rust source-line citations.

When authorities disagree, stop only the work that depends on the conflict,
report it, and repair the non-authoritative copy.

## Work classes

Choose the class before writing a new artifact.

### Ordinary implementation

The default for internal code, refactoring, tests, and task execution. State the
observable completion check in `NOW.md`, implement it, run the relevant tests,
and obtain the repository review required by `AGENTS.md`. No ADR is needed.

### Internal experiment

Use for a prototype or implementation choice that is cheap to replace inside
the experimental V2 boundary. Record its question, command, and result in the
code, test, or an EVD when the result must survive. It does not become an ADR
merely because it defines an internal type or layout.

### Durable decision or evidence

An ADR is required only when a choice does at least one of these:

- changes persisted data, a wire protocol, or a public API;
- defines a real-time safety or ownership boundary;
- binds several later phases or an external consumer;
- requires data migration or breaks delivered behavior when reversed;
- requires an explicit product choice from the user.

Decision-driving measurements use an EVD with the falsifier and acceptance rule
written before collection. Review the method before collecting data when an
asymmetry could change the conclusion. The ADR review may also review the
result; do not require a second broad review of the same material.

### Decision timing and readiness

Crossing a durable boundary determines how a decision is recorded, not when it
must be made. Draft and accept an ADR only when:

- the active or immediately next dependent implementation slice cannot proceed
  safely without the answer; or
- deferring through that slice would create a persisted, public, protocol, or
  delivered-behavior commitment, or would materially multiply reversal cost.

If a provisional internal choice keeps the durable boundary open, record that
choice in code, tests, an EVD, or `NOW.md`, together with its revisit point. A
registered ADR topic is a question, not automatically a work item.

Do not accept an option that cannot be implemented safely until another
undecided policy is resolved. Either decide the coupled boundary together or
keep the ADR `Proposed` or `Deferred`. Replacing one phase prerequisite with a
new prerequisite is not progress by itself.

A phase gate requires an accepted ADR only when its observable outcome depends
on that durable choice. Completing or classifying the decision register is not
a phase outcome.

### Phase exit

An exit review checks the named outcomes and evidence for that phase. It does
not re-review accepted ADRs or repeat measurements unless new evidence
invalidates them.

## Execution loop

1. Select one bounded vertical slice in `NOW.md`.
2. Name its observable completion checks and non-goals.
3. Build the smallest test, probe, or implementation that can answer the open
   question.
4. Create an ADR only if the durable-decision test above is met.
5. Run the change-appropriate gate in `AGENTS.md`.
6. Obtain one review covering the declared risk.
7. Repair blocking findings and reread only the changed claims and their direct
   consumers.
8. Commit the coherent slice and advance `NOW.md` once.

## Review stopping rule

A review remains open only for:

- a false factual claim that affects the conclusion;
- an internal contradiction;
- a contract hole an implementer cannot fill;
- a safety or correctness defect;
- evidence whose method cannot establish the claimed result.

Editorial preferences and optional implementation detail do not block. Findings
outside scope become separate work unless they invalidate the current
conclusion. There is no required number of passes.

Every semantic repair receives a focused independent reread. A mechanical
status, link, or wording repair receives self-audit and the documentation check;
it does not automatically start another independent review.

## Decisions and current specifications

An ADR records why a durable choice was made. A current specification records
what implementation must do now. Code follows the specification, not a chain of
superseded ADR clauses.

When an accepted decision changes:

1. accept the successor ADR;
2. update the affected current specification;
3. update the decision index and `NOW.md`;
4. leave historical phase records and accepted reviews unchanged.

Avoid clause-level supersession for the current contract. The successor may
describe the historical relationship, but the specification presents one
coherent current rule.

## Evidence and gates

Claims about performance, correctness, parity, or real-time behavior require a
named automated test or reproducible EVD. An EVD owns its method, numbers,
limitations, and conclusion; operational documents only link it.

A phase gate identifies its bounded input or revision, required outcomes, named
checks, and later-finding policy. A later finding reopens a passed gate only when
it invalidates relied-on evidence or a safety/correctness guarantee needed by a
dependent phase.

## Progress

Primary progress is executable behavior, tests, evidence that retires a named
risk, and removal of a concrete blocker. Document count, review count, and
classified-row count are not deliverables.

## Documentation style

Actively maintained workflow prose has a soft maximum of 120 characters per
line. Markdown tables, code blocks, and literal links are exempt. Do not reflow
frozen phase records, accepted reviews, ADR history, evidence, inventories, or
the legacy master plan solely for line width; that creates noise without making
the current workflow clearer. The documentation checker enforces the limit on
the active-document set defined under [Authorities](#authorities).
