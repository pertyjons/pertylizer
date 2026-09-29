# ADR-0076: Phase 9A engine gate and mixed-producer deferral

| Field | Value |
|---|---|
| ID | ADR-0076 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-29 |
| Last reviewed | 2026-09-29 |
| Related | ADR-0022, ADR-0048, ADR-0050, ADR-0051, ADR-0054, ADR-0055, ADR-0065, ADR-0072, ADR-0073, ADR-0075, EVD-0016, EVD-0024 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

This is an **explicit product and cross-phase choice made by the user** on
2026-09-29. It decides what Phase 9 must deliver before later phases may build on
the live engine, and which Phase 9 obligations wait for physical verification.

**Why it is ready.** The active slice was ADR-0075's mixed-producer rehearsal
series. Ninety-three commits followed ADR-0075's framing in `0fe36722` through
`869c116b`, most adding test-only rehearsals, while ADR-0075 stayed `Proposed`
with its four acceptance items open. Physical qualification needs hardware this host lacks:
macOS and Windows endpoints, a paired arrival reference and an audio loopback
([ADR-0022](ADR-0022-hardware-time-mapping.md) follow-up table). Continuing
either way would keep every simulated live-engine outcome waiting on those two
streams. The user chose to finish the simulated engine first and verify
physically afterwards.

## Decision boundary

This record introduces an intermediate **Phase 9A engine gate** inside Phase 9.
It defers mixed-producer activation behind that gate. It does **not** relax
Phase 9's final exit gate or accept, weaken or re-scope ADR-0022, ADR-0048,
ADR-0050 clause 8, ADR-0051 or ADR-0075. It does not lift a current refusal.

**Verified premises.**

- `LiveInputStream` owns an empty compiled scheduler and no activation
  ([ADR-0072](ADR-0072-finite-live-audition-and-count-in.md#exclusive-live-owner)).
  `PerformanceIngress::prepare` refuses a mixed plan, and the private mixed owner
  exists only under `#[cfg(test)]`
  ([ADR-0075](ADR-0075-mixed-producer-activation-ownership.md#first-mixed-ingress-rehearsal-boundary)).
  The deferred case therefore already fails closed.
- ADR-0022 may stay `Deferred` until the Phase 9 exit gate, and its constraint 2
  forbids a production consumer of host timestamps or callback latency until it
  is accepted. That constraint keeps physical results confined to the adapter
  boundary.
- The deterministic simulated host, the ALSA output custody harness and the
  evidence gate run without the missing hardware.

## Evidence

- [`NOW.md`](../NOW.md#phase-9--active) records P09-S001 through P09-S007, the
  concurrent live host and the ordered transport work as implemented and
  reviewed. Physical MIDI, loopback and timing qualification remain open.
- [EVD-0016](../evidence/phase-03/EVD-0016-host-time-mapping.md) rejects the two
  retained Linux observations under F4 and F7. macOS and Windows artifacts,
  the F10 paired reference and the F11 connection bridge are missing.
- [EVD-0024](../evidence/phase-09/EVD-0024-live-capacity-qualification.md) names
  common host admission, same-stream live transport activation and a
  representative workload as its missing capacity coverage.
- On 2026-09-29 two `aseqdump` sessions on an Akai MPK mini Play mk3 showed
  complete three-byte note, CC1 and pitch-bend messages. This is a diagnostic
  observation, not EVD-0016 evidence.

**Uncertainty that could change this.** Physical evidence could reject the
arrival fallback or the monitoring clock policy. Constraint 2 of ADR-0022 limits
that revision to host adapters. If physical work instead invalidates an engine
contract that the 9A gate accepted, that is a false 9A claim and reopens it.

## Options

1. **Continue ADR-0075 rehearsals toward one combined host.** Rejected for now.
   Its acceptance needs amendments to about a dozen accepted records, and every
   later live outcome would wait for them.
2. **Write the combined-host law directly and test it once.** Viable, but it
   still gates every other Phase 9 live outcome on the hardest contract.
3. **Defer mixed producers and add a simulated engine gate.** Selected. Current
   refusals already fail closed, and physical evidence keeps its exit role.

## Decision

1. **Engine gate.** Phase 9 gains an intermediate *Phase 9A engine gate*,
   defined in the [master plan](../master-plan.md#phase-9a-engine-gate). It lists
   the Phase 9 outcomes provable by deterministic simulated-host tests and the
   non-shipping Linux harnesses. Physical-device evidence is not part of it.
2. **Final exit unchanged.** Phase 9's existing exit gate remains the only
   Phase 9 exit. It now also requires the deferred mixed-producer work.
   Every existing reference to the "Phase 9 exit gate", including ADR-0022's
   deferral and ADR-0055's loop condition, keeps its meaning.
3. **Mixed-producer deferral.** One admitted stream carrying both compiled note
   playback or activation and live note ingress remains refused until the final
   exit. That refusal must be a named diagnostic, not a silent drop. ADR-0075
   becomes `Deferred`. ADR-0048, ADR-0050 clause 8 and ADR-0051's shared-gate
   ownership remain with it. The obligation blocks its first consumer and Phase 12.
4. **Live input during 9A.** No timestamp-capable live input adapter, such as
   a midir MIDI connection, enters a production path before ADR-0022 is
   `Accepted`. ADR-0032 clause 19 forbids discarding an available driver
   timestamp and declaring `Arrival`, and ADR-0022 constraint 2 forbids
   consuming it. At 9A, live note input reaches V2 only through simulated
   sources and the non-shipping Linux harnesses. A source with genuinely no
   timestamp may still declare `Arrival` with clause 19's `unmeasured` marker.
   9A claims no qualified live timing. Physical measurement and production
   timestamped input belong to the final exit.
5. **Capacity during 9A.** If a production live adapter is enabled at 9A,
   ADR-0054 reselection first covers the producer partition that 9A admits.
   The final exit repeats it for any partition that mixed producers widen.
6. **Dependencies.** Phase 11 may begin after the 9A engine gate and Phase 10E.
   Phase 12 requires Phase 9's final exit.

## Consequences and risks

- **Accepted cost.** Until the final exit, live notes and compiled playback of
  the same instrument stream cannot sound together. The 9A slice that makes V2
  selectable for live playback must state the resulting product behaviour and
  refuse the mixed case visibly. Live timing remains unqualified, and a
  physical MIDI keyboard cannot drive V2 in a production path before the
  final exit.
- **Safety/correctness control.** The deferred case is refused at plan and
  ingress admission today. ADR-0022 constraint 2 keeps host timestamps out of
  production code. The 9A gate is reviewed like an exit review. Phase 12 cannot
  start on 9A alone.
- **Risk: the final exit is never reached.** Control: Phase 12 depends on it,
  and ADR-0022 and this record name it as the only Phase 9 exit.
- **Revisit condition.** When physical hardware for EVD-0016 is available, when
  a 9A or later consumer needs one stream with both producers, or when physical
  evidence contradicts a 9A engine contract.

## Specification update

No specification invariant changes. The master plan gains the 9A engine gate,
and [`ROADMAP.md`](../ROADMAP.md) records the Phase 11 and Phase 12
dependencies. [`NOW.md`](../NOW.md) records the deferred ADR-0075 work. The
9A slice that selects V2 for live playback must name its mixed-plan refusal in
the [host I/O specification](../specs/spec-host-io-lifecycle.md).

## Review

Reviewer: independent uncommitted review before commit.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable
contract, safety/correctness defect, or evidence incapable of supporting the
claim. Editorial detail does not block.
