# ADR-0039: Initial Multi-Client Hub Omission

| Field | Value |
|-------|-------|
| ID | ADR-0039 |
| Status | Proposed |
| Phase | 0A/10E |
| Created | 2026-08-15 |
| Last reviewed | 2026-08-15 |
| Related | ADR-0030, ADR-0038, CAP-0017, LIMIT-0017, Phase 10E, EVD-0005 |
| Supersedes | ADR-0038's `LIMIT-0017`-specific applications: the part 1 seven-entry table row; part 4's paragraphs assigning `Lossy retention/presentation budget`, `Protocol contract`, and `Investigating`; and the matching consequences and follow-up clauses saying no decision owns the row. ADR-0038's general condition 4 remains unchanged |
| Superseded by | — |

## Context

`EngineHub::broadcast_event` writes to one bounded ring per client and drops an event when that client's ring is full.
The drop is counted per client. The method takes an `RwLock` over the client map and a `Mutex` per client, so it is not
an audio-thread mechanism; blocking, backpressure, disconnection, buffering outside the engine, and subscription
refusal are all technically available alternatives.

At source revision `29c22ef4`, `LIMIT-0017` in the [resource inventory](../inventories/resource-limits.md) and
`CAP-0017` in the [capability inventory](../inventories/capabilities.md) record no workspace caller for
`EngineHub::broadcast_event`. That observation does **not** make the capability unreachable: those authoritative
inventory rows also record the public `synth_engine` exports and Pertylizer re-export. An external Rust consumer can
construct the hub and fill the bounded ring; repository search cannot establish whether one does. This ADR consumes
those facts rather than copying their source citation.

Core V2 is allowed to break that public surface, but the break must be explicit. Phase 1 has no client, protocol, or
remote surface. Phase 10E is where MCP, CLI, OSC, the public facade, remote authorization, and any promoted
multi-client service are available together, so it is the first phase that can choose and test a successor delivery
contract. EVD-0005 supports only the bounded no-workspace-caller claim.

## Decision drivers

- A slow client must not silently lose an authoritative or custodial payload.
- Off-render-path dropping is a choice, not a real-time necessity, under ADR-0038 condition 4.
- Phase 0A must not turn an exported but unused-in-workspace V1 mechanism into a premature V2 contract.
- Removing the public V1 hub from initial V2 is an intentional compatibility break, not proof that no consumer exists.
- Phase 10E introduces the service and public-facade migration where client lifecycle and backpressure can be tested.

## Options to evaluate at the Phase 10E entry gate

1. Disconnect a client that exceeds its negotiated delivery capacity, with an attributable reason and counters.
2. Apply bounded per-client buffering outside the render path, then disconnect or refuse when the bound is exhausted.
3. Provide protocol backpressure when the transport supports it, with an explicit policy for transports that do not.
4. Permit lossy observational delivery under ADR-0038, while separating any custodial payload and carrying an
   attributable loss count with the affected stream.

## Decision

The initial Core V2 contract intentionally omits the public V1 multi-client hub. The final compatibility and delivery
contract is deferred to the Phase 10E entry gate. With this record accepted, and until a Phase 10E successor is
accepted:

- Sound Core V2 exposes no multi-client hub and carries no `CLIENT_EVENT_BUFFER_SIZE` successor;
- no Core V2 frontend, protocol, or adapter exposes a replacement hub implicitly;
- `LIMIT-0017` is a deliberately broken V1 protocol capacity, not carried into the initial V2 renderer;
- a later hub proposal must classify every payload as observational or custodial and define capacity, exhaustion,
  client lifecycle, diagnostics, and ordering before implementation begins.
- the Phase 10E successor may retain the omission as a tested local-only/public-facade policy, or introduce a new
  service-level multi-client contract; it may not silently inherit V1's public type or 1,024-entry drop-on-full ring.

This bounded omission is the Phase 0A disposition. It satisfies ADR-0038 condition 4 by choosing **none of the delivery
mechanisms yet** and excluding the capability from initial V2, rather than classifying drop-on-full as acceptable
runtime behaviour. It does not claim that V1 is unreachable and does not choose dropping, blocking, buffering, or
disconnection.

## Acceptance basis available in Phase 0A

- the revision-pinned use-site audit finds no workspace caller of `broadcast_event`, while CAP-0017 records its public
  export and the limit of that evidence;
- Phase 1 has no client, protocol, or remote-delivery surface;
- this repository explicitly requires no backward compatibility during active development;
- the master plan makes an accepted successor an entry condition for Phase 10E;
- an independent reader verifies that omission leaves ADR-0038's general delivery constraints intact and does not
  select a later loss or backpressure policy.

## Evidence required for the Phase 10E successor

- concrete GUI/service multi-client scenarios, including a stalled client;
- the transports that must support backpressure;
- a prototype measuring bounded buffering and disconnect behavior off the render path;
- a closed payload inventory showing which payloads remain observational after leaving the engine.
- the Phase 0B public-facade and capability dispositions, including any known external consumers and whether the V1
  export is intentionally removed or replaced.

## Consequences

- Phase 1 is not blocked by a service contract for a surface the phase does not implement.
- The initial V2 API deliberately breaks the public V1 `EngineHub` surface; CAP-0017 keeps that break visible.
- Phase 10E cannot accidentally inherit V1's drop-on-full behavior.
- If a multi-client hub becomes reachable earlier, this deferral is violated and the ADR must be resolved before that
  change lands.

## Revisit condition

Before Phase 10E implementation begins, or before any earlier Core V2 change introduces a multi-client event hub.
