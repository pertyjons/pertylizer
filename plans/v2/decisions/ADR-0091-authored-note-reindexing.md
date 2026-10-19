# ADR-0091: Authored note reindexing during native continuation

| Field | Value |
|---|---|
| ID | ADR-0091 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-19 |
| Last reviewed | 2026-10-19 |
| Related | ADR-0050, ADR-0083, ADR-0085, ADR-0087, ADR-0089, ADR-0090 |
| Supersedes | — |
| Amends | ADR-0085 global note-driven refusal and ADR-0087 fixed logical note indices, only for the bounded isolated native case below |
| Superseded by | — |

## Decision

The isolated compiled session's native continuation resolves playable targets by
stable authored node identity rather than equal logical note index. The native
voice kernel program, instance geometry and ordered routing keep ADR-0087's
existing proof. Physical voice relocation remains supported. Changed voice
meaning selects the explicit running reset; it is not preserved implicitly.

A cold prepared forward/inverse note map binds the old and new plan identities.
Every retained actual note must pass a bounded lookup with matching inverse,
plan and destination extent before mutation. The actual renderer LiveNotes
registry is rebound at the installation boundary; the actual StreamControl
IdentityTable is rebound when control collects the owning installation receipt.
Table identity, epoch, generations, victim ages and pool positions stay actual.
No minter is imported from a snapshot or prepared successor.

The timeline comparison and stamped witness resolve the same authored target.
Each retained voice note's ordered magnitude group is compared through mapped
node, parameter and tuning bindings; a global insertion may change its range
offset without changing what that voice plays. Both map vectors' actual
capacities are charged to transition bytes, and structural work includes old
and new target extents in the existing record bound.

Structural global note-driven DSP remains fresh. An old global playable target
has no preservation mapping even if its node is retained. A surviving identity
or score/witness reference to it refuses native preservation and selects reset.
A parameter writer to its gate or magnitude nodes also makes it active. A dormant global target must be an unconnected native Envelope in Global scope,
with no scheduled output consumer and no magnitude destination on another node.
Absence of note events alone cannot authorize resetting audible global DSP.
These dormant targets may be added, retained or removed while the voice program
stays unchanged. Exact-layout continuation keeps its existing broader
unchanged-target preservation; this decision does not reset an exact-layout gate.

The existing total session-command freeze remains in force while a replacement
owns its credit, including Play, Stop and Panic. No command stamped under the old
plan crosses a logical reindex boundary. Callers retain transport intent and
submit it after collecting the owning replacement. No origin protocol or command
settlement order changes. The fixed-native mixed continuation remains fixed:
this decision grants no structural mixed plan migration.

## Consumer and falsifiers

Adding a native global Envelope at authored node 2000 moves the real compiler's
existing voice Envelope20 from logical note index 0 to 1; removing it moves back.
The graph edit changes neither the compiler's ordering policy nor persisted
identity. Deterministic tests repeatedly perform both edits at four sample rates,
with whole and irregular callbacks, warm Delay history, future note/automation
edges and an in-progress steal. Normal unedited rendering is the bitwise oracle.
An observed control hands off the actual tap store beside the reindexing voice.

Acceptance fails on wrong-envelope routing, changed reference samples, replaced
identity authority, lost future releases, callback allocation/deallocation,
partial mutation after a malformed map, an active global target silently reset
under a preservation claim, or a session command crossing the frozen interval.
Foreign, out-of-range, missing and wrong in-range map destinations must refuse
before actual mutation. Unsupported voice meanings keep explicit reset controls.
A contract hole blocks acceptance; optional implementation detail does not.
Production capacity, general mixed migration and physical qualification remain
separate work.
