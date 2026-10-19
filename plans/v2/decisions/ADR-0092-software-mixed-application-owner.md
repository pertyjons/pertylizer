# ADR-0092: Software mixed application owner

| Field | Value |
|---|---|
| ID | ADR-0092 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-19 |
| Last reviewed | 2026-10-19 |
| Related | ADR-0001, ADR-0054, ADR-0078, ADR-0081, ADR-0086, ADR-0088, ADR-0089, ADR-0090 |
| Supersedes | — |
| Amends | ADR-0078 and ADR-0081 application refusal only within ADR-0088's approved default-off software route |
| Superseded by | — |

## Decision

The separately selected `v2-mixed-app` route lowers the actual saved project
with a declared live range and resolves its live envelope by the selected stable
instrument identity. The binding is immutable for the armed owner. Changing GUI
selection never retargets a held note. Default builds and the ordinary compiled
V2 route retain their selection and transport policy.

The route owns two logical software connections from the shared checked
connection-generation issuer. Reopening never reuses either generation. Its
constructor selects `Arrival` provenance before the ingress store accepts any
event. Existing simulated rehearsal arming keeps `Simulated`; neither constructor
can assert hardware timestamps. No native MIDI endpoint or physical clock mapping
is attached. Raw capture, recording, strict publication and mixed tap observers
are unavailable in this route. The isolated compiled observer completion remains
separate.

The application declares a provisional stereo profile with a 4096-frame callback
ceiling and the engine's validated default limits, at 44.1, 48, 96 or 192 kHz.
This is not EVD-0025's compiled qualification or EVD-0026's retained mixed
selection. Admission enforces the declared bounds; no production CPU or latency
qualification follows. A device callback is never split to manufacture a smaller
qualified callback. Oversized callbacks reach the owner's terminal-fault path.

An input nominal stamp is the maximum of the audio-published first-unrendered
boundary plus two quanta and that source's previous frontier. Addition is checked.
Both source frontiers advance after staged events on every GUI pump, including
an idle peer. Continuous repaint services the tolerant bridge. GUI stalls stop
new input admission while existing audio continues; resumed input receives a
fresh dispatch estimate. No physical alignment or latency ceiling is claimed.
Each source owns a bounded held-key table, checked before stage, and checked
packet serials. A split onset retains its physical-key entry until its key-up
settles the bridge tombstone. Repeat key-down is ignored. Focus loss releases
both software sources, including pedal lifts. Teardown accounts for still-held
keys rather than inventing release delivery. Each physical key keeps its onset
packet identity; joined teardown must find that identity in a staged/in-flight,
open-handle or refusal-tombstone record. The application retains typed counts of
physical keys, pending packets, open handles, refused keys and pedal scopes.
Lost or unknown command custody refuses successful joined settlement.

A private audio-session enum extends the existing application switch. Prepared
sessions, transport packets and returned owners cross bounded SPSC rings. The
callback neither allocates nor destroys owning payloads. One operation credit
serializes compiled transport and plan edits. Control coalesces the latest
requested playing state, explicit destination and explicit loop selection
independently while that credit is outstanding. Play cannot erase a pending
Stop's rewind, and Seek cannot erase a pending Pause. The owning operation
carries a checked location-intent revision. Every explicit destination or loop
request advances it, including a repeated identical seek. An applied activation
clears location dimensions only when that revision still matches its captured
request; an older returned receipt cannot acknowledge a newer command.
Pause uses compiled Stop and collects its actual stopped position. Explicit Stop
also leaves the loop and retains a rewind to zero. Plain Pause keeps the loop.
A stopped Seek promotes its adopted cursor before later Play or reset. A stopped
reset returns its actual audio cursor in the owning completion; control checks
it against the admitted destination before adopting it. Play resumes the
acknowledged cursor and an existing loop unless an explicit destination or loop
exit replaces them. These operations affect compiled sound only.

Enabling the whole-song loop starts at zero, including while paused. Leaving a
loop rewinds to zero and preserves the requested playing state. Seek exits the
loop and preserves that state. These destinations are explicit GUI behavior,
not inferred continuation positions. An activation refused at its audio boundary
retains its requested dimensions for a fresh offer. A cold preparation refusal
rejects the unpublished compound transport request, clears its location intent
and restores requested playing state to the acknowledged state. It reports the
refusal once and leaves sound unchanged; a refused loop never starts unlooped
playback as a fallback. A declined plan edit remains pending with its previous
audible plan; Pause, Stop, Seek, explicit loop exit
or a newer project edit permits another attempt.

Compatible edits use ADR-0090's owning fixed-native continuation. Changed program
meaning or identity lineage uses an explicit whole-owner reset, including its
seal, ready fence, fade receipt, source/tombstone handoff and joined old-owner
teardown. The reset destination is a checked projection of the last actually
adopted non-loop anchor while running, or the acknowledged stopped cursor.
A whole-owner edit while a compiled loop is active is deferred until an explicit
loop exit; no advisory cut receipt reconstructs a loop successor.

The install sequence identifies a successful complete outer callback. The source
owner also retains its initial install identity for cold retirement; a later plan
install or refused candidate cannot change which control owns that retired audio.
GUI and MCP use the same owned playback view. The existing project-only lowering
digest retains its meaning and closed wire schema. The immutable live binding
and host profile belong to the mixed session's private configuration; digest
identity alone never proves full-plan equality. Canonical project revision stays
with Phase 0B/10A.

After explicit removal returns an audio owner, control settles owning returns on
the cold thread. A pending reset can be withdrawn and abandoned. An already
started fade may render at most one extra quantum into discarded stack storage
solely to obtain its retirement receipt and join its owners. This happens only
after callback quiescence, writes no switch acknowledgement or GUI/MCP plan
publication, and is not audible-output evidence. A terminal failure reports cold
destruction and failed joined settlement. CPAL processor field destruction is
also destruction, not a fabricated owning return or native-device fence.

## Falsifiers and stopping rule

Acceptance fails on callback allocation or destruction, a default-route behavior
change, an unpaired or unaccounted source identity, stale rendered-plan publication,
a lost pending transport intent, or use of a hardware/production qualification
that this route does not possess. Real saved-project tests exercise two sources,
Pause/Play, compatible continuation, explicit changed-program reset, old physical
key-up after reset, bounded pressure and quiescent removal. Callback controls
measure allocations and deallocations. Silent-source output must match the
ordinary saved-project reference bit for bit after ADR-0001's one-quantum
streaming priming; the entire leading quantum must be silent, and the comparison
includes the complete requested reference tail. Offline rendering removes that
priming, so unaligned output is not the reference criterion.

A false ownership, timing or publication claim blocks acceptance. Optional mixed
observation or native input implementation does not expand this decision.
