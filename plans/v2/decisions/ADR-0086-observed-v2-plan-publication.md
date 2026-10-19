# ADR-0086: Observed V2 plan publication

| Field | Value |
|---|---|
| ID | ADR-0086 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-18 |
| Last reviewed | 2026-10-18 |
| Related | ADR-0027, ADR-0077, ADR-0082, ADR-0083, ADR-0085 |
| Supersedes | — |
| Superseded by | — |

## Decision

Publish the application's last GUI observation of its experimental V2 switch
through a separate read-only `get_v2_playback_status` MCP tool. Keep
`get_engine_status`'s V1 meter/transport contract unchanged. This status is an
observation of a rendered session, not Phase 0B/10A's canonical project revision.

Read the callback's installation acknowledgement once per GUI poll/action.
Select the matching plan, lowering-input hash and omissions from that value,
then retain one owned view for both the GUI badge and MCP publication. A queued
candidate never borrows an old acknowledgement. A running replacement retains
the previous plan until its successful rendered installation; mute/removal
filters the acknowledgement. Report refused and inactive/releasing states
separately. `requested_playing` describes control intent/acknowledgement under
the existing switch law, not a hardware transport measurement.

The new closed `pertylizer.v2-playback/1` response has an object root and an
observation field. Unavailability distinguishes not compiled, no attached GUI,
not yet published, no publisher, invalid observation and exhausted observational
identities. Published observations carry a probabilistic service-instance nonce, checked publication identity, age
in milliseconds and the owned playback state. Active states carry a checked
process-local switch scope; plan/install/publication/scope identities are
prefixed decimal strings, preserving all u64 bits in clients. Exhaustion never
wraps, never relabels an old payload and never stops sound. Detaching the GUI
clears its last payload without restarting its issuer.

The age measures the GUI publication, not the last callback. A stopped device
can leave a previously confirmed plan visible; no field certifies device
liveness or acoustic output. Status reads do not advance the MCP mutation
counter, including via batch dispatch. All publication and cloning happen off
the audio thread.

`sha256:pertylizer.v2-lowering-input/1` hashes sorted JSON of the captured
instrument, song and global fields already consumed by lowering. Explicitly
sort every object, independent of serde_json's map feature selection. Equality
is defined only within one service instance; an upgrade or restart starts a new
comparison scope even when the projection label is unchanged. The hash includes
cosmetic instrument fields carried in that projection, while cosmetic
GUI edits need not rebuild the plan. It is only an opaque equality token for
the input actually lowered: not the current project, a saved-file digest, an
atomic multi-owner capture, a command-application barrier or a canonical
semantic revision. Non-finite raw floats also share JSON's null representation.
The tool exposes no claim about completed concurrent MCP mutations or batches.

## Falsifiers and stopping rule

A candidate reported as audible before a successful callback, a muted/refused
plan reported as audible, mismatched plan/omissions, a silently reused identity,
loss of large integer identity bits, observation on the callback allocator path,
or a status read counted as a mutation blocks acceptance. Missing/unknown fields,
invalid identity strings and unsupported versions are rejected. Nullable audible
state is always present, including explicit null before confirmation.

Tests use the actual application switch and callback, compare the cached view
before/after acknowledgement, exercise replacement omissions, mute/removal,
GUI detach/reattach and near-limit issuers. The protocol tests validate the real
headless reply against its catalog schema and ensure batch reachability.

## Phase 10 dependency

The multi-owner `ProjectFile` builder, per-owner change detection and identity
lineage fence remain their existing mechanisms. They do not establish a single
canonical capture. A future Phase 10 project revision must have its own kind
and version; never reinterpret this hash or these session identities as that
revision. The canonical revision integration obligation remains open. This
slice changes no saved data, existing wire contract, capacity gate or phase exit.
