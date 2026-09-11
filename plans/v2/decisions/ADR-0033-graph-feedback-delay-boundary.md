# ADR-0033: Explicit Quantum Feedback Boundaries

| Field | Value |
|---|---|
| ID | ADR-0033 |
| Status | Accepted |
| Phase | 8/9 |
| Created | 2026-09-10 |
| Last reviewed | 2026-09-10 |
| Related | ADR-0001, ADR-0005, ADR-0037, ADR-0050, SOUND-INV-036 |
| Supersedes | — |
| Superseded by | — |

## Boundary and readiness

P08-S006 needs a rule for cycles without making every sidechain read a hidden
host-callback cache. This decision binds the compiler, arena and renderer, and
therefore crosses a real-time ownership boundary. V1's instrument sidechain
cache reads the previous callback; the current-quantum graph deliberately does
not reproduce that timing. No persisted format or shipping dependency changes.

## Decision

1. Ordinary audio, gate, control and modulation dependencies remain acyclic.
   Only an explicit `FeedbackDelay` cuts a dependency: its sole audio input is
   written after the graph has rendered, while its stereo output reads the
   previous quantum. The delay is exactly `Q` plan frames, independent of host
   callbacks. An ordinary `Delay` or `Latency` node does not break a cycle.
2. This boundary has shared state. Its node and its input source must be outside
   voice scope. A voice sum can feed an ordinary shared node before entering the
   boundary. No per-voice feedback lifetime is implied. Unsupported placements
   and missing inputs are refused by node identity; usual type, layout, fan-in
   and scope validation still applies.
3. The compiler removes only incoming audio cables of these boundaries from
   current-quantum ordering. Any cycle remaining is refused. A boundary may not
   occur anywhere in a modulation source's transitive dependencies: the
   modulation prepass has no previous-quantum feedback state. Current-quantum
   audio/control dependencies can cross the explicit boundary; modulation
   bindings and event cycles cannot acquire such a cut.
4. Every boundary reads once and writes once per quantum. Writes occur after
   all graph reads, and source buffers remain live until their deferred writes.
   ADR-0005's last-read condition therefore forbids in-place mutation of a write
   source by an earlier consumer. The arena sees deferred writes as reads, not
   as invisible state effects. All history and write metadata are admitted
   before rendering; no allocation, locks, graph traversal or callback-sized
   work enters the hot path.
5. A graph containing a boundary requires the explicit `Decline` compensation
   policy. Automatic compensation is refused rather than altering recurrence
   timing. Diagnostics separately report each boundary's `Q` delay. Path bounds
   describe **one current traversal of the cut graph**, treating boundary
   outputs as roots at zero; they do not bound the age of recirculating content.
   The node timing still declares `Q`. The existing `added_latency` scalar is
   adapter `Q` plus maximum current-traversal output latency, and offline trims
   only the adapter `Q`. A recurrent graph has no claimed finite composed tail.
   Phase 9 must not use this traversal figure as an overall recurrence-age or
   automatic host timeline-correction claim.
6. A fresh renderer starts with zero history. A shared boundary survives voice
   steals, note releases and same-plan transport relocations, like shared
   inserts: relocation does not reconstruct DSP state. Stopping advancement
   preserves state; resuming continues it. Replacing the renderer/plan starts
   fresh. No live panic/flush API is introduced here; a future host that needs
   one must explicitly clear shared state rather than assuming a seek did it.
7. Saved V1 cyclic cables have no authored boundary of this kind and remain
   refused by cable identity. Lowering never silently inserts a delay. Native
   IR authors can declare this boundary; persisted feedback authoring belongs
   to its first Project-model consumer.

## Alternatives and consequences

- Hidden previous-callback delay: rejected because changing the callback size
  changes the sound and the detector's timing.
- Sample-by-sample cyclic execution: unnecessary for this boundary and would
  change the fixed-quantum kernel contract. Sub-quantum feedback remains refused.
- Treat every delay as a cycle cut: rejected because ordinary kernels consume
  current inputs and do not separate state reads from writes.
- Compensate a recurrence: rejected here because a finite DAG latency sum does
  not describe repeated loop travel. The explicit policy refusal leaves that
  choice visible to callers.

## Falsifier and stopping rule

Acceptance requires an impulse recurrence `y[n] = x[n] + g*y[n-Q]` at the exact
sample positions under whole, 64, 256 and irregular callbacks; source-order
invariance; two boundaries that capture the same quantum; a downstream in-place
candidate that cannot corrupt capture; named refusals for zero-delay cycles,
modulation-prepass feedback, voice placement and automatic compensation; exact
history/resource accounting; silent fresh state; retained shared state under
voice/transport events; and the allocation/purity guard covering read and write.
A false timing claim, state ownership hole, uncharged buffer or callback-dependent
result blocks acceptance. Optional broader feedback support does not.

## Design consultation

A fresh `agy` / `gemini-3.8-flash-high` read reviewed the author's frame before
implementation. It requested explicit capture immutability, shared source scope,
reset lifetime and mixed-cycle semantics; these are stated above. Its proposed
removal of the fixed output `Q` was rejected: V2 primes that carry for every
callback partition, including exact multiples of `Q`. Its claim that voice
instances reuse one buffer between iterations was also rejected: they have
separate scheduled instances and an explicit compiled sum. The shared-source
restriction is the chosen initial boundary, not a repair for that claimed reuse.
The arena already merges an in-place chain only at the input's last read; the
new deferred read must participate in that calculation and is tested directly.

## Review

A fresh `agy` / `gemini-3.8-flash-high` reviewed the full uncommitted
implementation, new files and current specification: no defects. A focused
independent reread of the typed compressor gain conversion also found no defects.
The complete repository gate and the direct recurrence, capture, resource and
transport/voice lifetime checks passed before the implementation commit.
