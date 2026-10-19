# ADR-0085: Stable-voice global structural continuation

| Field | Value |
|---|---|
| ID | ADR-0085 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-18 |
| Last reviewed | 2026-10-18 |
| Related | ADR-0010, ADR-0048, ADR-0082, ADR-0083, ADR-0084 |
| Supersedes | ADR-0083's exact whole-plan geometry restriction for this bounded global-path consumer |
| Superseded by | ADR-0087 permits physical storage relocation for unchanged logical voice programs |

## Decision

Permit structural changes to an isolated compiled session's native global path
while the complete physical voice program and playable addresses stay unchanged.
This is an additional compatible case before ADR-0082's reset fallback, not
general voice relocation or identity-table rebuilding.

Keep channel layout, rate, maximum block, event shares, producer ranges/holds,
tuning, voice policies, instance and sum groups, prepass and output latency.
Both plans have no compensation records. Every voice-scope authored identity,
scope, physical row, prepared kernel, input binding and parameter row matches.
The complete note targets/magnitude fan-out and note addresses stay unchanged;
note-driven global rows refuse this consumer. Matched authored kernels and
parameter bases cannot change. Native global additions start prepared/fresh;
removed global state retires. Matched role, layout or parameter-row incompatibility
refuses continuation rather than silently freshening that state. Scripts, samples, controllers, modulation,
feedback, authored runtime producers and positional sends remain outside it.
Existing mixer records match under their checked plan-local normalization.

Prepare the existing non-voice stable-ID/scope migration map off-thread, beside
the exact voice/history rows. Relabel compiled parameter events through authored
node/parameter addresses, never a coincident numeric slot. The complete ordered
semantic program still matches before any stamped successor witness is built.
Unsupported structure chooses reset; resource/binding failures keep old sound.
Shape eligibility is decided before the structural record budget, including when
new strip rows extend past the old plan's parameter table.

At the exact owning transition boundary, preflight both global and voice maps,
all state/history/parameter extents and live note bindings before any mutation.
Copy small state/parameter records and swap history owners into the already
prepared geometry. Then exchange geometry owners into the actual renderer.
Keep its clock, anchor, epoch, authoritative live registry, carries and
diagnostics. Keep the actual control-side minter and rebind its unchanged note
indices at collection. There is no new authoritative identity table and no
snapshot promotion. Outgoing geometry stays in the owning return envelope.
Existing first-successful-fragment acknowledgement and interrupted custody apply.

`maximum_transition_records` separately caps the sum of both plans' operations
and parameter rows plus the identity partition before preparing a structural
map. The provisional application value is 16,384. Actual metadata capacities
also fit the existing transition byte allowance. Neither limit qualifies
aggregate production CPU/memory or changes a producer share.

## Falsifiers and scope

Acceptance fails on partial mutation before refusal, changed retained voice
audio through a neutral structural stage, lost delay history, wrong later
release/bend routing, issuer rewind, callback allocator activity, lost owning
credits or partition-dependent output. The controls compare an unedited normal
session at four rates and five callback partitions, and repeatedly add/remove a
neutral stage around a warm delay while holding notes. Stop retains dormant
compiled note custody under its existing law.

The application still requires its graph-lineage fence and unchanged authored
end. Core removal controls do not authorize bypassing a GUI deletion fence.
ADR-0087 adds bounded voice storage relocation. Added note destinations, logical
playable reindexing, changed voice programs/configuration,
mixed/live continuation and compensation changes remain separate consumers.
This consumer supplies passive normal-session taps; ADR-0084 supplies coordinated
observer handoff through observed stopped installation and paired observed
running-replacement entries. Legacy unobserved entries retain the gap policy:
end the old store before offering, and bind a fresh store after installation.
Queued old frames retain their old label; the gap has no subscribed coverage.
Under that gap policy, after refusal or cancellation, bind a fresh store to the
retained old plan; the ended store's queued tail retains its old label.
No Phase 9 exit or production qualification is claimed.
