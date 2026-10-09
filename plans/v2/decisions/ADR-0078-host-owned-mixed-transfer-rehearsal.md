# ADR-0078: Host-owned mixed transfer rehearsal

| Field | Value |
|---|---|
| ID | ADR-0078 |
| Status | Proposed |
| Phase | 9 |
| Created | 2026-10-10 |
| Last reviewed | 2026-10-10 |
| Related | ADR-0023, ADR-0043, ADR-0046, ADR-0047, ADR-0050, ADR-0053, ADR-0054, ADR-0068, ADR-0072, ADR-0075, ADR-0076 |
| Amends | If accepted: ADR-0046 clause 6, ADR-0047 clause 7, ADR-0050 clause 8, ADR-0072's exclusive-live-owner exception and ADR-0076 decision 3, only for the dedicated non-shipping rehearsal feature below |
| Supersedes | — |
| Superseded by | — |

## Decision boundary

ADR-0075 and ADR-0076 keep every production mixed offer closed. The private
one-shot owner now proves many command and audio cases, but its command and
result rings, mixed ingress preparation and offers are compiled only under
`cfg(all(test, feature = "simulated-ingress"))`. An integration test cannot use
that path, so it cannot prove that a non-test host can carry one mixed bend.

The proposed change is an **additive experimental Rust API** under a new,
default-off `mixed-transfer-rehearsal` Cargo feature in `synth_engine_v2`.
That feature would add a manifest declaration, but no new dependency edge,
persisted format, wire contract or application route. The public
`PerformanceIngress::prepare` refusal remains. It needs explicit approval
because it would permit a limited mixed owner outside the test configuration
while the accepted records listed above authorize the live identity/release
exception only for an exclusive owner or defer mixed streams until Phase 9
exit. The final mixed-host contract remains undecided.

## Verified premises

- `synth_engine_v2` has `ringbuf` only as a dev-dependency. Its normal
  dependency allowlist in `tests/crate_boundary.rs` does not include it.
- The existing session and loop transfer boundaries make the host own their
  concrete queues. The crate has no safe exported SPSC queue suitable for this
  new transfer.
- The private mixed owner already has separate command and result payloads,
  bend-credit accounting, protected release and lift places, and a Panic place.
  Its control side still has test-only queue operations and assumptions that
  must be replaced before this API can be reached by a host.
- `PerformanceIngress::prepare` refuses a compiled-plus-live plan with
  `MixedProducerPlan`; the application keeps MIDI inactive in V2 playback.

## Proposed rule

1. **Limited owner.** The default-off `mixed-transfer-rehearsal` feature
   implies `simulated-ingress` and exposes a new constructor from the existing
   bound, closed, initial one-shot mixed plan. It returns paired control and audio
   owners. It admits exactly the existing one compiled and one live note
   producer, without Panic, plan replacement, loop playback, hardware input
   or an application playback route. The existing general mixed-plan refusal
   remains. Repository shipping targets must not enable this feature; the
   crate-boundary test must enforce that condition.
2. **Host-owned transfer.** The crate returns opaque, non-cloneable command
   packets and accepts opaque result packets. The host owns the bounded SPSC
   command and result queues; `synth_engine_v2` gains no normal `ringbuf`
   dependency. Arm takes the actual capacities observed from both queues and
   refuses either one below the admitted total command/result bound. That
   bound already includes places withheld for accepted releases and lifts;
   it does not add those reservations to the ring size twice. The host must
   pass those observed capacities, and the integration host checks them at
   construction. The crate preallocates and charges an outstanding-command
   journal; the host charges both queues.
   A failed command push returns the same packet for exact retry. Its ID and
   reservation remain outstanding, and no younger command may pass it.
   Arbitrary cancellation while the stream continues is not supported.
   Joined terminal teardown returns every unsent packet and names every
   outstanding command/result ID; it does not recycle a missing release or
   lift credit into a live stream. Queue allocation, final queue destruction
   and packet recovery occur off the audio thread.
3. **Order and identity.** Each command binds the owner, plan, epoch and identity
   table, and has one gap-free increasing command ID while the stream runs.
   Audio refuses a foreign, duplicate or reordered packet before changing
   state and halts the transfer; the packet remains owned for teardown. An
   ordered packet whose nominal time is already behind the first unrendered
   sample executes at that first unrendered boundary, never before it. Its
   result names both nominal and effective times and the late clamp. This
   follows ADR-0043 and ADR-0072's existing rule, including for a protected
   release or lift, so neither can be refused merely for age. On-time bends
   execute in command order at their admitted `SampleTime`; neither the host
   nor the crate may coalesce them. A later live onset on a bent scope **must**
   inherit the last accepted bend at the onset's effective sample, spending one
   reserved bend credit. If none is free, its offer refuses before minting
   its command or handle. A full host queue retains its exact head and cannot
   let a younger command pass it.
4. **Result and audio custody.** Packet service allocates nothing, takes no
   lock, makes no system call and does not panic. An undersized or full result
   queue cannot drop a result: audio retains one pending result in prepared
   storage and stops command service until the host collects it. Every normal
   result carries the original command identity and outcome. A missing or
   discarded command/result packet is a terminal host fault, visible through
   the outstanding journal at joined teardown; the stream may stall but cannot
   continue with reclaimed credit. Packet types contain no final-drop audio
   resources.

The user-selected stage-full bend sound split remains a **separate combined-host
question**. The current private prototype also splits when a younger frontier
waits, and a split return to zero can leave a prior non-zero bend and its
credit cost indefinitely. This API neither owns raw capture or the stage nor
authorizes a split after a mixed command is minted. Production acceptance must
join the source, raw ID, stage disposition and mixed result in source order
and revisit the scope-capacity law before enabling a live adapter.

## Falsifiers and required proof

- An integration test cannot compile the path without `cfg(test)`, the
  default-off feature does not imply `simulated-ingress`, or a repository
  shipping target enables `mixed-transfer-rehearsal`.
- The constructor accepts a missing or second compiled or live producer, a
  non-initial candidate, Panic, loop or plan replacement, or crosses the
  admitted target partition.
- A command or result ring smaller than its required bound arms;
  a full ring loses a protected release/lift packet, its handle or its credit.
- A foreign, replayed or reordered command changes audio, or a late release,
  lift or bend executes before the first unrendered boundary, loses its
  original ID, or omits its nominal and effective times from the result.
- An on-time bend's direct writes occur at a frame other than its admitted
  stamp under one-call, 64-frame, 256-frame or irregular callback partitioning,
  or it changes a compiled producer's note. A later live onset fails to
  inherit the accepted bend at its effective sample with one bend credit, or
  mints a handle when that credit is unavailable.
- A full result queue drops a result or serves a younger command past the
  pending result; a dropped packet is absent from the joined unresolved-ID
  report or lets the host reuse its ID or credit.
- A callback allocates, locks, panics or performs final resource destruction.

The first integration fixture must use ordinary library code with a host-owned
SPSC ring to carry a live onset, bend and release beside a compiled note. It
must verify command/result identity, note isolation and sample-accurate audio.
The required gate runs it with
`cargo test -p synth_engine_v2 --no-default-features --features mixed-transfer-rehearsal --test mixed_transfer_rehearsal`;
the ordinary workspace test alone cannot execute a default-off feature test.
The implementation must add this command to the repository quality workflow
and `AGENTS.md`'s complete gate before claiming its proof runs in CI.
It proves the transfer boundary only. The concrete two-source stage, raw and
recorder shared charge, complete combined outcomes, Panic, loop and plan-swap
laws, ADR-0054's full six-share reselection, platform timing and Phase 9 exit
remain open. No production live adapter is enabled by this proposal.

## Options and consequence

1. **Keep every mixed operation test-only.** No new contract is needed, but a
   production host cannot use or falsify the existing command chain.
2. **Move `ringbuf` into the crate's normal dependencies.** This changes its
   manifest and Phase 1 dependency allowlist, and requires separate approval.
3. **Host-owned queues with opaque packets.** Proposed here. It preserves the
   current normal dependency surface and makes the transfer ownership and
   failure rules explicit.

If accepted, this record narrows ADR-0046 clause 6, ADR-0047 clause 7,
ADR-0050 clause 8 and ADR-0072's exception solely to permit audio-owned live
minting and release-at-offer in the simulated initial one-shot rehearsal under
`mixed-transfer-rehearsal`; it does not authorize these for an application or
hardware host. It narrows ADR-0076 decision 3's refusal solely for that same
non-shipping integration path. ADR-0075 remains deferred, including its
combined-host acceptance work; its test-only rehearsal description is
historical rather than authority for this new path. Update the current host
I/O specification before exposing the API. Acceptance does not mark the Phase
9 exit complete. The user must approve this public rehearsal boundary and its
new default-off manifest feature.
