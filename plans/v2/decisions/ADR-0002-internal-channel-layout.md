# ADR-0002: Internal Channel Layout

| Field         | Value              |
|---------------|--------------------|
| ID            | ADR-0002           |
| Status        | Superseded         |
| Phase         | 2                  |
| Created       | 2026-08-17         |
| Last reviewed | 2026-08-17         |
| Related       | EVD-0008, P02-T001, P02-T002, P02-T003, ADR-0001, ADR-0005, REV-P01 deviation 4 |
| Supersedes    | —                  |
| Superseded by | [ADR-0041](ADR-0041-interleaved-internal-channel-layout.md), in full, on 2026-08-19 |

## Context

Phase 2 validates channel-layout compatibility across every edge and inserts the conversions a valid plan needs. Both
require an answer to a question Phase 1 deliberately did not answer: **what a multi-channel signal is, in the arena.**

Phase 1 carried the minimum it rendered and said so. Its `ChannelLayout` has `Mono` and `Stereo` and nothing else, and
[REV-P01 deviation 4](../phases/phase-01-experimental-sound-core.md#deviations) states the reason in full: adding a
`Multi(n)` variant would have *claimed* that a layout is a channel count, which is a decision belonging here. Its arena
is per-source mono buffers of `Q` frames, and the one place interleaving appears is the output carry the host is served
from; both citations are in *Evidence* below.

This record decides the internal representation and where conversion is permitted. It does **not** decide the eventual
multichannel *vocabulary* — which layouts exist, and whether a layout is a count or a set of speaker roles — beyond
fixing the property every layout must satisfy. Phase 9 is the first phase with a real device to query, and the master
plan puts extensible multichannel metadata in Phase 8's mixer work.

**Outside this decision.** Buffer *storage* assignment and reuse ([ADR-0005](ADR-0005-buffer-liveness-strategy.md));
the summing law for a stereo-to-mono down-mix, which is a product decision with no caller in this phase and is refused
rather than guessed (see clause 6); sample-rate conversion and oversampling islands (Phase 8); and the host's own
device layout negotiation (Phase 9).

## Decision

**`Accepted` — and worth stating plainly: not on its own measurement, which came back against this record's preferred
option.** [EVD-0008](../evidence/phase-02/EVD-0008-internal-channel-layout-cost.md) was run to a rule fixed before the
data, and under that rule **criterion D selects the interleaved arena** — a median 9.3% cheaper end to end across nine
runs, negative in every run, against a control spread of 1.75%. Criterion B holds (the planar transpose is a median
3.62% of a minimal chain) and criterion A holds but bounds only *naive* strided implementations, which the measured
interleaved chain is not.

An earlier revision of this record was `Accepted` on the opposite result. That result was an artifact of an
interleaved amplifier that did an integer division per sample; review caught it, and the corrected arm reversed the
margin by more than twenty points. The reversal is kept in EVD-0008's corrections table.

**The clauses below are accepted on the architectural argument, not the performance one** — decided by the user on
2026-08-17 after the measurement was presented against it — and the cost of taking it is a measured number rather than
an assumption:

- **For planar.** Phase 2's recorded execution choice is kernel extraction into a shared home that **both engines
  call**. V1 is planar — one mono `AudioBuffer` per port — so a kernel written over `&mut [f32]` serves V1 unchanged.
  An interleaved V2 would need either a second frame-wise variant of every shared kernel or a de-interleave into
  scratch before each call, which is the extraction path's whole premise weakened. Planar also keeps a mono operation
  on one channel expressible without a stride, and it is what V2 Phase 1 already is.
- **Against planar.** About **9% of a minimal stereo chain's per-quantum work** on this fixture and this machine,
  coming from eight node calls where interleaved makes five, a second buffer, and a transpose where interleaved
  memcpys. That is not a rounding error and this record does not present it as one.
- **What would change the answer.** If the shared-kernel constraint is dropped — a V2 that owns its own kernels, or a
  kernel interface expressed over a channel-strided view — the measurement selects interleaved.

**And a consequence worth stating rather than discovering.** The obvious "we can revisit this once V1 retires" is true
in kind and misleading in practice. Nothing persists the layout, so a later change is a rebuild rather than a
migration — but it would touch the arena, ADR-0005's per-channel liveness unit, the port/edge model, the conversion
operations, the in-place rule, the resource report, **and every node kernel's signature**. That set grows monotonically
with the node catalog, so the cheapest moment to flip is now and the most expensive is exactly the moment V1 retires.
Given a margin of about 2% of a realistic per-voice cost (EVD-0008's ~33 ns per quantum against EVD-0003's measured
1.173 ms per second per voice — arithmetic over two records, not a third measurement), nobody will pay for a
catalog-wide rewrite later. This record is therefore made on merit now rather than deferred as reversible, which is
also why the register classes it `Contract`.

The acceptance rule is retained verbatim below, because a rule that is only readable after it passed is not a rule.

1. **Internal audio is planar. A buffer is always mono**: `Q` contiguous frames of exactly one channel. There is no
   such thing as a multi-channel arena buffer.
2. **Channel layout is a property of a port and its edge, not of a buffer.** A signal with `n` channels occupies `n`
   buffers, and the plan records which slot is which channel. A kernel receives one channel and is never told how many
   there are.
3. **Interleaving occurs only at an external boundary** — the host callback's output carry, an audio file writer, or a
   device input. It is an explicit operation in the plan or in the renderer's boundary code, never an implicit
   property of an arena slot.
4. **`ChannelLayout` is an ordered sequence of channels.** Ordering is part of the layout: channel 0 of a stereo
   signal is the left channel, and a conversion may not permute silently. This is the property every future layout
   must satisfy, and it is what makes the vocabulary extensible without changing clauses 1 to 3.
5. **This phase admits `Mono` and `Stereo` and refuses to invent the rest.** Extending the set — multichannel
   metadata in Phase 8, a queried device layout in Phase 9 — adds variants under clause 4 and requires no change here.
   A layout this build cannot render is not constructible, so there is nothing to refuse at runtime.
6. **The only implicit conversion Phase 2 inserts is mono to stereo**, by duplication into each channel. A
   stereo-to-mono edge is a **compile error** naming the edge, both endpoints, and both layouts — not a silent
   down-mix. The summing law for a down-mix (sum, halved sum, or −3 dB) is a product decision with no caller in this
   phase, and picking one here would make it authoritative by accident.
7. **Every conversion is a scheduled operation with an identity**, so it appears in the schedule, in the resource
   report's buffer count, and in a diagnostic — the same treatment Phase 1 gave its declared `OutputMono` duplication.
8. **A layout mismatch that no permitted conversion resolves is refused at compile time**, before admission reports
   resources, with a path-local diagnostic. The renderer never sees a plan with an unresolved layout.

## Consequences

### Positive

- Every extracted kernel keeps the signature it already has, which is what makes the extraction path in Phase 2
  cheap rather than a rewrite.
- The one transpose per callback is where a transpose has to be anyway, and it is already written.
- Adding a wider layout later is an enum extension plus a conversion rule, not a change to how a buffer is addressed.

### Negative

- Slot count and operation count grow linearly with channel width. A stereo master chain schedules twice the
  operations of a mono one, and each is dispatched separately.
- Any future kernel that genuinely wants interleaved stereo — a mid/side or correlation operation over both channels
  at once — has to take two mono blocks instead. That is a real ergonomic cost, paid by the minority of nodes.

### Risks and controls

- **Risk: a channel is silently swapped by a conversion or a copy.** Control: clause 4's ordering rule, with a test
  that a stereo signal whose channels carry distinguishable content survives every conversion and the boundary
  interleave in order.
- **Risk: a down-mix is added later as a quiet default.** Control: clause 6 makes it a refusal today, so adding it
  requires a decision rather than an edit.

## Follow-up work

| Task | Phase | Status |
|------|-------|--------|
| Layout validation and the mono-to-stereo conversion operation | 2 (P02-T002, P02-T003) | Not started |
| The down-mix law, if a caller ever needs one | 8 | Not started |
| Multichannel layout vocabulary under clause 4 | 8/9 | Not started |

## Revisit conditions

- **The real voice path, re-measured at P02-T008.** EVD-0008's margin was measured on the cheapest defensible
  five-node chain, where fixed overheads weigh heaviest. P02-T008 is the first task with both a rendered path and an
  `EVD` record to put the figure in, and it owes this before the phase's exit review;
  a margin materially larger than 9% there is a reason to reopen this record rather than to note it.
- The shared-kernel constraint disappearing **while the node catalog is still small** — the only window in which the
  measurement's preference could be acted on cheaply.
- A node class the product needs whose kernel is fundamentally cross-channel and cannot be expressed over `n` mono
  blocks without a copy that itself dominates.
- A profiled plan where the boundary transpose is a measurable share of a callback's cost on a real device.
