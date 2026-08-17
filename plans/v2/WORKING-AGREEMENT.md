# Core V2 Working Agreement

This agreement defines how Core V2 work is investigated, decided, implemented, reviewed, and reported. It exists to
keep evidence, decisions, and progress moving in one direction instead of copying the same conclusion through several
documents.

## Authority

Each kind of information has one authority:

| Information | Authority |
|-------------|-----------|
| Scope, phase order, and exit gates | [master-plan.md](master-plan.md) |
| Normative decisions | [ADR.md](ADR.md) and the linked decision record |
| Measurements and their limitations | [evidence/](evidence/README.md) |
| V1 facts and migration dispositions | [inventories/](inventories/README.md) |
| Current task status, blockers, and next action | [phase tracker](phases/README.md) |
| Current technical contract | [specifications](specs/README.md) |
| Gate verdict and review findings | [exit review](reviews/README.md) |

`STATUS.md` is a short index over those authorities. A document links to an authoritative fact; it does not retell the
fact or maintain a second count. Review chronology stays in the review that produced it. A durable record may retain a
compact corrections table when a disproved premise would otherwise be easy to repeat.

When authorities disagree, stop the dependent work, report the conflict, and repair the non-authoritative copy. Do not
select whichever wording makes a gate pass.

## Evidence before normative prose

1. A claim about V1 behaviour needs either a use-site read or an executable observation before it enters an ADR or
   specification. A definition, constant name, comment, or re-export is not evidence that a production path uses it.
2. Cite the authoritative inventory or evidence record from normative documents. Keep current `file:line` citations in
   the inventory or evidence record where their drift can be checked once. A decision may retain an explicitly
   revision-pinned source citation as historical audit evidence; it is not a second authority for current source state.
3. When a question can be answered by a test, probe, instrumentation, or a small implementation, run that before
   another decision-writing pass. Search-based audits may establish coverage of their declared search space; they may
   not claim that unnamed and undocumented behaviour does not exist.
4. State a falsifier and acceptance criteria before collecting decision-driving evidence. A retrospective record says
   that it is retrospective and cannot close a claim its method cannot establish.
5. A number that supports the current conclusion receives the same source check as a number that challenges it.
6. Performance, correctness, parity, and real-time claims — about V1 or V2 alike — require a reproducible `EVD`
   record or a named automated test, not prose.

## Decisions

1. Accept an ADR when its factual premises are verified and its policy choice has been independently reviewed. Do not
   accept it because a ledger or phase needs its status.
2. Do not accept a contract ADR in the session that drafts or materially rewrites it. At least one reader who did not
   author the change reviews it first.
3. Keep accepted records immutable. Prefer a focused successor when independent clauses survive. Supersede the whole
   record when a reader would otherwise need both records for most questions. Clause-level supersession is not chosen
   merely because rewriting is inconvenient.
4. Do not introduce decision machinery without a concrete decision that uses it. Revisit unused machinery when it
   adds more reading cost than the decisions it avoids.

## Phase gates

Every gate must identify:

- the source revision or bounded input set it evaluates;
- the surfaces and types in scope;
- the discovery or measurement method;
- named tests, probes, or evidence records;
- the policy for findings made after closure.

A later finding becomes a tracked defect or inventory entry. It reopens a passed gate only when it invalidates evidence
used by that gate or violates a safety or correctness guarantee that a dependent phase relies on. Open-ended discovery
continues without making completed work perpetually provisional.

A V1 corpus case is blocked only when it cannot be reproduced or when its measured output is itself a function of an
open decision. An open V2 policy is not, by itself, a reason to postpone observing V1.

## Review protocol

For each change:

1. State scope and acceptance criteria before editing.
2. Change the authoritative record, then mechanically search for every consumer of the identifiers and terms whose
   meaning changed.
3. Run the smallest relevant executable checks before review.
4. Obtain at least one review from a reader who did not author the change.
5. Treat every correction as new material. Re-read the corrected clause against its surrounding contract and search
   its consumers again.
6. Put findings outside the declared scope in separate tasks unless they invalidate the change's central conclusion.
7. Stop when the acceptance criteria are verified and the review no longer requires a contract, authority, safety, or
   correctness change. There is no fixed pass count; editorial improvements alone do not keep a review open.

The repository-wide quality gate and `codex review --uncommitted` remain mandatory before a commit, as specified by
the repository instructions.

## Progress

Primary progress is:

- executable tests and probes that reduce a named risk;
- implemented vertical slices;
- decisions required by the next concrete implementation;
- blockers removed with reproducible evidence.

Document count, review count, and classified-row count are supporting indicators. They are not deliverables by
themselves.
