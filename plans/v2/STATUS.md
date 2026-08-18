# Pertylizer Core V2 Status

| Field | Value |
|-------|-------|
| Last updated | 2026-08-18 |
| Active phases | 0B and 2 |
| Phase 0A review | [`REV-P00A`](reviews/phase-00a-exit-review.md), `Accepted` |
| Phase 1 review | [`REV-P01`](reviews/phase-01-exit-review.md), `Accepted` |
| Sound Core V2 implementation | The experimental `synth_engine_v2` crate exists and is gated — [tracker](phases/phase-01-experimental-sound-core.md) |
| Phase 2 | **Decided, pending one review** — [tracker](phases/phase-02-minimal-compiled-voice-graph.md). T001–T005 and **T012** complete. The user decided on 2026-08-18: V2 owns its DSP ([ADR-0040](decisions/ADR-0040-v2-owns-its-dsp.md)) and the arena becomes **interleaved** ([ADR-0041](decisions/ADR-0041-interleaved-internal-channel-layout.md), superseding ADR-0002 in full and six ADR-0005 clauses), on [EVD-0010](evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md). Both records are `Proposed` until a reader who did not draft them reviews; then **P02-T013**, the conversion, and T006 closes |

This file is a short index, not a work log or an independent source of task status. The
[Phase 0A tracker](phases/phase-00a-baseline-and-render-contracts.md),
[Phase 0B tracker](phases/phase-00b-inventories-and-project-contracts.md),
[Phase 1 tracker](phases/phase-01-experimental-sound-core.md),
[Phase 2 tracker](phases/phase-02-minimal-compiled-voice-graph.md), decision
[register](ADR.md), and [exit review](reviews/phase-00a-exit-review.md) are authoritative for their respective data.
The workflow and authority rules are in [WORKING-AGREEMENT.md](WORKING-AGREEMENT.md).

## Current objective

Run Phase 2, the minimal compiled voice graph, against the render core Phase 1 built. Continue Phase 0B
independently; it gates Phase 10.

## Phase 0A summary

| Task | Current state | Authority |
|------|---------------|-----------|
| Reference corpus | Complete for Phase 0A; ten fixtures and one owned reproducibility gap | [P00A-T001](phases/phase-00a-baseline-and-render-contracts.md#p00a-t001--define-the-reference-v1-corpus-and-preservechange-manifest) |
| Comparison command | Complete | [P00A-T002](phases/phase-00a-baseline-and-render-contracts.md#completed-tasks) |
| Baseline measurements | Complete; EVD-0007 covers the current ten-case corpus | [P00A-T003](phases/phase-00a-baseline-and-render-contracts.md#completed-tasks) |
| Resource audit | Complete for Phase 0A; every row has the required proposed rule and diagnostic, and the probe passes | [P00A-T004](phases/phase-00a-baseline-and-render-contracts.md#p00a-t004--complete-the-fixed-limit-and-overflow-audit) |
| Initial host profile | Complete and `Current` for the Phase 1 field set | [P00A-T005](phases/phase-00a-baseline-and-render-contracts.md#p00a-t005--define-the-initial-hostprofile-and-renderlimits-contract) |
| Required Phase 0A decisions | Complete | [P00A-T006](phases/phase-00a-baseline-and-render-contracts.md#completed-tasks) |
| Exit review | Complete at `Accepted` | [REV-P00A](reviews/phase-00a-exit-review.md) |

## Next actions

1. **Accept ADR-0040 and ADR-0041**, which is one independent review pass and the four status edits ADR-0041's
   *Status* section lists. Both decisions are made, on
   [EVD-0010](evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md); P02-T006's extraction closes as
   not-happening with ADR-0040.
2. **Then P02-T013** — the arena, compiler, kernels and report converted to interleaved, verified by a bit-identical
   render against digests taken from the planar build.
3. Carry the **binding** `Q` re-measurement to the Phase 2 exit gate: ADR-0037's 64 was accepted provisionally on an
   inconclusive V1 proxy, and Phase 2 is the last point at which changing the constant is cheap.
4. Continue Phase 0B independently.
5. Keep ADR-0039 and `LIMIT-0017` visible as Phase 10E work.

## Later work already owned

- Phase 3 owns renderer-ingress capacity, deferred-event storage, and the ADR-0001 clarification.
- Phase 4 owns the long-running job contract.
- Phase 0B owns project, identity, format, asset, and application-operation contracts before their named implementation
  gates.
