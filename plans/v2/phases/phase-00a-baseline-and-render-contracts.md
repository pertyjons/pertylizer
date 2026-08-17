# Phase 0A: Baseline, Limits, and Render-Core Contracts

| Field | Value |
|-------|-------|
| Status | Complete |
| Started | 2026-08-12 |
| Last updated | 2026-08-15 |
| Exit review | [`REV-P00A`](../reviews/phase-00a-exit-review.md), `Accepted` |

## Objective

Establish the bounded V1 evidence and the initial contracts needed by the experimental Phase 1 renderer. Phase 0A does
not design live scheduling, project identity, asset storage, or application operations; those remain with their named
later phases.

The [master plan](../master-plan.md#phase-0a-baseline-limits-and-render-core-contracts) is authoritative for scope and
gates. This tracker is authoritative only for task state, blockers, and next actions. The working method is defined by
[the Core V2 working agreement](../WORKING-AGREEMENT.md).

## Required decisions

| ADR | Required at exit | Current status | Later verification |
|-----|------------------|----------------|--------------------|
| ADR-0001 | `Accepted` | `Accepted` | Phase 3 |
| ADR-0037 | `Accepted` | `Accepted` | Phase 2 remeasurement |
| ADR-0021 | `Accepted` | `Accepted` | Revisited if the executable resource probe escapes its taxonomy |
| ADR-0032 | `Accepted` | `Accepted` | Phase 3 |
| ADR-0022 | `Accepted`, or bounded deferral | `Deferred` | Phase 3 entry gate |
| ADR-0028 | `Accepted`, or bounded deferral | `Deferred` | Phase 4 entry gate |

ADR-0038 and its focused ADR-0039 successor were discovered after the required table was satisfied. They are not
hidden additional Phase 1 contracts: ADR-0038 repairs the disposition of V1 engine-egress queues, while proposed
ADR-0039 would make omission of a public multi-client surface from initial V2 an explicit compatibility break.

## Tasks

| ID | Deliverable | Status | Dependency | Authority/evidence |
|----|-------------|--------|------------|--------------------|
| P00A-T001 | Define the V1 corpus, category vocabulary, and preserve/change manifest | Complete for Phase 0A; ten fixtures and one owned reproducibility gap | None | [Corpus manifest](../../../corpus/v2-reference/manifest.json), [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md), [EVD-0004](../evidence/phase-00a/EVD-0004-corpus-0005-claim-counterfactuals.md) |
| P00A-T002 | Define the comparison model and headless command | Complete | P00A-T001 | `pertylizer compare`, EVD-0001 |
| P00A-T003 | Capture CPU, memory, timing, and determinism baselines | Complete for the current ten-case corpus | P00A-T001 | EVD-0001 through EVD-0003, [EVD-0007](../evidence/phase-00a/EVD-0007-expanded-corpus-baseline.md) |
| P00A-T004 | Complete the bounded fixed-limit and overflow audit | Complete for the Phase 0A gate; all 76 rows have proposed rules and diagnostics, the probe passes, and 75 rows are `Classified` | None for Phase 1; ADR-0039 remains Phase 10E work | [Resource inventory](../inventories/resource-limits.md), [EVD-0005](../evidence/phase-00a/EVD-0005-resource-ledger-use-site-audit.md), [EVD-0006](../evidence/phase-00a/EVD-0006-resource-limit-runtime-probe.md), ADR-0039 |
| P00A-T005 | Define the initial `HostProfile` and `RenderLimits` contract | Complete | P00A-T004 evidence available | [Host-profile specification](../specs/spec-host-profile-and-render-limits.md), REV-P00A |
| P00A-T006 | Satisfy the required-decision table | Complete | P00A-T003, P00A-T004 evidence available | [Decision register](../ADR.md) |
| P00A-T007 | Evaluate the formal exit gates | Complete | Applicable tasks | REV-P00A |

## P00A-T001 — Define the reference V1 corpus and preserve/change manifest

Phase 0A closes this task on a **coverage matrix**, not on implementing later-phase semantics. At source revision
`54cd6d3f`, all eleven master-plan categories must be represented exactly once as either:

- an executable fixture with classified preserve/change claims; or
- an explicit gap naming the reproducibility problem or later owner.

`cargo test -p pertylizer --test corpus_manifest` enforces the closed category vocabulary, the case/gap partition,
required gap owners, claim classes, fixture generation, parsing, digests, and render-level behaviour controls for the
five new cases (panner channel movement, YAMS audibility and signed gains, tempo intervals, shared-instrument faders,
each also asserting a clean load report). Ten categories have fixtures. One remains an explicit reproducibility gap:

| Category | Disposition |
|----------|-------------|
| `sampler-patch` | Phase 0B bundle fixtures own the project-plus-asset reproducibility boundary |

The former effort-only rows are now deterministic fixtures for Keyboard Panner stereo, YAMS control rate, YAMS audio
rate, and a tempo ramp/step arrangement. Independent review rejected the sharing gap because V1 can reproduce sharing
without a random script; CORPUS-0010 now exercises two track-local faders through one deterministic shared instrument.
Each case renders headlessly without warnings — CORPUS-0006..0010 under the named test binary, CORPUS-0001..0005 as
recorded by [EVD-0007](../evidence/phase-00a/EVD-0007-expanded-corpus-baseline.md)'s determinism run. The remaining
sampler gap carries a required owner and a concrete reason the project-plus-asset input cannot yet be reproduced
independently.

## P00A-T004 — Complete the fixed-limit and overflow audit

The discovery boundary is the V1 workspace at revision `29c22ef4`. The inventory contains 76 rows produced by three
declared methods: limit/constant discovery, truncation/comment discovery, and use-site reading. EVD-0005 supports
coverage of that declared population and is inconclusive about residual accuracy; it does not claim that unnamed,
undocumented behaviour cannot exist.

EVD-0006 runs the executable resource probe over its three named axes — oversized callbacks, meter slots beyond 128,
and rack migration beyond 32 stages. All three map to existing inventory rows and classes. The oversized case corrected
`LIMIT-0001` after debug found a fixed-buffer assertion and release confirmed audio-thread allocation; ADR-0021's
taxonomy revisit condition was not triggered.

`LIMIT-0017` has no workspace caller, but `EngineHub` is a public Rust API and external reachability is not observable
from this repository. Corrected ADR-0039 proposes omitting the public hub from initial V2 as an explicit compatibility
break and requires a Phase 10E successor decision. The row remains `Investigating` until that proposal passes an
independent review and is accepted. That inventory lifecycle state does not fail the Phase 0A gate: the authoritative
gate requires a proposed rule and diagnostic for every row, not an accepted disposition.

Findings after `29c22ef4` become new `LIMIT` rows and tracked defects. They reopen Phase 0A only when they invalidate
its evidence or a safety/correctness guarantee consumed by Phase 1.

## P00A-T005 — Define the initial HostProfile and RenderLimits contract

The deliverable is the field set named by the master plan, not a complete live-runtime contract. The `Current`
specification maps all 28 `HostProfile`-owned inventory rows to fields and names the basis of each default.

Phase 3 owns the renderer-ingress streams, deferred-event store, and the ADR-0001 clarification needed by
`HOST-INV-021`. The invariant retains its identifier but is non-normative until those mechanisms can be implemented and
tested. Phase 1 must not invent them.

## Completed tasks

- **P00A-T002:** `pertylizer compare` runs headlessly and reports the versioned comparison model.
- **P00A-T003:** EVD-0001 through EVD-0003 retain their historical measurements and limitations. EVD-0007 supplies
  the ten-case determinism, CPU, timing, and RSS supplement required for the current corpus.
- **P00A-T006:** the four required ADRs are accepted; ADR-0022 and ADR-0028 carry bounded later gates and constraints.

## Corrections retained

| Disproved premise | Current statement | Authority |
|-------------------|-------------------|-----------|
| Constant-name and definition reads made the resource inventory complete | Only the declared search population is covered; executable probes bound the named runtime axes | EVD-0005 and P00A-T004 |
| `EVENT_BUFFER_SIZE` was a per-block renderer cap | It sizes two engine-egress rings; V1's sequencer buffer is an uncapped `Vec` | Resource inventory, `LIMIT-0014` and `LIMIT-0075` |
| The prioritized event channel was live and published counters on OSC | It has no workspace production caller and no workspace reader of its counters; its public export makes external use unknown | ADR-0038 |
| Host-profile event deferral could be specified from V1 queue sizes | V1 has no timestamped renderer-ingress queue; ingress and deferred storage belong to Phase 3 | Host-profile specification and REV-P00A |
| Corpus phase differences proved a sequencer timing defect | V1 voice allocation changed oscillator phase; fixtures now disable note-on phase randomization | EVD-0004 |
| Offline rendering applied full instrument state | The renderer omitted non-default instrument settings; fixed by `f8867990` and protected by corpus fixtures | Corpus tests and git history |

## Next actions

1. Start the minimal Phase 1 vertical slice against the accepted render-core contracts.
2. Continue Phase 0B independently; it gates Phase 10 rather than Phase 1.
3. Review ADR-0039 independently before Phase 10E; do not use that later lifecycle step to block Phase 1.

## Exit readiness

Status: **Accepted**. All six evidence gates and the repository quality gate pass, and the final independent review
found no actionable defect. ADR-0039 and `LIMIT-0017` remain open for Phase 10E without adding a seventh Phase 0A gate.
