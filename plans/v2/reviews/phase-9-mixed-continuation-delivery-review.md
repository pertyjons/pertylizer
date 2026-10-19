# Phase 9 fixed native mixed continuation delivery

Date: 2026-10-19. Scope: [ADR-0090](../decisions/ADR-0090-fixed-native-mixed-continuation.md),
not Phase 9 acceptance or mixed application enablement.

## Delivered behavior

A fixed-native mixed plan can change its immutable DSP binding while retaining
actual compiled/live minters, the renderer registry, warm history, future input,
release ownership and both source frontiers. Native global Trim edits follow
the existing proof. Source handles carry the stable transfer/table/epoch owner.
Structural and logical note-index changes remain refused on this path.

One owning packet returns explicit refusal, InstalledRendered or
InstalledFaulted. Only the full installing callback can confirm successful
output. Cold metadata follows the actual installed plan even after a fault.
A dropped packet has an explicit quiescent recovery path returning
InstalledUnconfirmed rather than fabricated output evidence. Every prepared
activation's custody remains counted through adoption, cancellation or refusal
until collected or destroyed cold.

## Independent review

Fresh Claude Code reviewed the uncommitted scope without shell, MCP or delegation.
Its first read found premature wrapper success, cancelled-receipt custody,
dropped-packet recovery and an index-table defect. Repairs added outer-callback
finalization, owning activation custody and explicit lost-packet recovery. The
focused read accepted those repairs but found missing bridge recovery access;
the final focused read accepted the public bridge/reset recovery paths and the
two-source recovery test with no blocking defects. The author also reviewed the
complete diff. Review transcripts and command logs are retained under
`/tmp/pertylizer-six-points/` for this work session.

## Verification

The complete 24-command repository gate passed, followed by the final
documentation/evidence gate, all 41 documentation-checker controls and formatting verification. The mixed
library gate passed 1,103 tests with one existing ignored test. The delivery
documentation received an additional independent semantic review with no blockers.
Deterministic controls cover four rates, irregular callbacks, repeated replacements, warm Delay history,
held and future live notes, both source paths with sustain, protected Panic,
same-boundary compiled Stop, loop wrapping, cold/binding refusal, no-callback
withdrawal and discarded-packet recovery. A wrapper test faults in the second
quantum of a 128-frame callback and requires complete silence plus
InstalledFaulted. Callback guards observe both allocation and deallocation;
render-loop source controls cover every new hot helper.

Production capacity, the remaining producer classes, the application route and
physical/backend qualification remain separate work.
