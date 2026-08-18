# ADR-0005: Buffer Liveness Strategy

| Field         | Value              |
|---------------|--------------------|
| ID            | ADR-0005           |
| Status        | Accepted           |
| Phase         | 2                  |
| Created       | 2026-08-17         |
| Last reviewed | 2026-08-17         |
| Related       | P02-T004, ADR-0001, ADR-0002, ADR-0021, EVD-0008 |
| Supersedes    | —                  |
| Superseded by | Clauses 1, 2, 4, 5, 7 and 8, by [ADR-0041](ADR-0041-interleaved-internal-channel-layout.md) **when that record is accepted** — 2 and 4 refined rather than reversed; clauses 3, 6 and 9 stand |

## Context

Phase 2 replaces Phase 1's hand-written operation list with a compiled schedule, and a compiled schedule needs
somewhere to put its intermediate signals. Phase 1 gives **every source its own quantum-sized buffer** and never reuses
one — the code says so and names this record's task as the owner; the citations are in *Evidence* below. That is fine for four source kinds and a single output; it is not fine for a mixer graph, where the number of
simultaneously *declared* signals is far larger than the number simultaneously *live*.

This record decides **how the compiler assigns arena storage to signals**, and it decides it now because the choice is
a correctness contract, not a size: two signals that share storage while both are live produce wrong audio, and the
defect is a wrong number rather than a crash. It is also the point at which in-place processing becomes expressible,
because "may this node write over its input" is the same question as "is the input's live range ending here".

**Outside this decision.** The arena's *bytes* and their admission, which `HOST-INV-006`'s resource report and
ADR-0021 already own; the channel layout of a buffer, which [ADR-0002](ADR-0002-internal-channel-layout.md) owns; the
node representation that declares in-place safety, which [ADR-0004](ADR-0004-native-node-representation.md) owns; and
any cross-quantum storage such as delay lines and the ADR-0001 carries, which are node state or renderer state rather
than arena signals.

## Decision drivers

- **A reuse bug is silent.** Aliasing two live signals produces plausible audio, not a panic, and a listening test
  will not reliably catch it. Whatever is chosen must come with a check that fails loudly.
- **Determinism is a contract, not a preference.** ADR-0001 clause 17 makes a render digest the comparison instrument
  for this whole migration. If storage assignment could vary between two compilations of the same plan, a digest
  comparison would be measuring the allocator.
- **The render loop decides nothing.** The Phase 2 exit gate forbids topology decisions in the hot path; a liveness
  strategy that resolved anything at render time would violate it directly.
- **The master plan's own guidance is conservative-first** — "begin with correct conservative reuse; optimize only
  after profiling" — and there is no profile yet, because there is no graph big enough to profile.
- **Optimality is worth very little here and costs a lot.** The minimal voice path has five nodes. An assignment that
  is optimal rather than merely good saves bytes nobody has measured a shortage of.

## Options considered

### Option A: No reuse — one buffer per signal

Phase 1's behaviour, generalized. Every edge's signal gets its own arena slot for the whole render.

Correct by construction, trivially deterministic, and it makes the aliasing question disappear. It also makes the arena
grow with the *size of the plan* rather than with its *depth*, which is the wrong asymptote for a mixer: a hundred
channels each with a four-node chain would hold four hundred quantum buffers live at once when at most a handful are
readable at any point in the schedule. The master plan asks for liveness analysis in this phase by name, so choosing
this would be a deviation rather than a decision.

### Option B: Conservative linear-scan liveness over the compiled schedule

Compute each signal's live range as an interval in the topologically ordered schedule — from the operation that writes
it to the last operation that reads it — and let two signals share a slot only when their intervals do not overlap.
Assignment happens once, at compile time, by a single forward pass with a free list.

Simple enough to review in one sitting, deterministic because the schedule is, and it captures nearly all of the
available reuse in a signal-flow graph, where live ranges are naturally short. Its cost is that it is *interval*-based:
a signal whose consumers are far apart in the order keeps its slot alive across everything in between, even where a
finer analysis would have interleaved. That is exactly the conservative direction — it wastes storage, never audio.

### Option C: Schedule-aware assignment — reorder to reduce peak liveness

**Not interference-graph colouring.** Colouring is the obvious third option and it is a dead end here: with equal-sized
mono slots and live ranges that are intervals over one total schedule order, the interference graph is an interval
graph, and a left-edge scan with a free list already achieves the minimum — the maximum number of simultaneously live
intervals. Colouring cannot use fewer slots than that. Naming it as the upgrade path would have promised an
optimization that does not exist.

The real headroom is one level up: the peak itself is a property of the **schedule**, and a topological order is not
unique. Interleaving two independent branches keeps both alive at once; finishing one before starting the other does
not. A schedule-aware assignment would choose among valid topological orders to minimise peak liveness, then assign as
in Option B.

That is a genuinely stronger option and a much larger one: the ordering choice interacts with cache behaviour and, in
a later phase, with anything that parallelises the schedule. There is no plan in this workspace whose arena has been
measured at all, so there is nothing yet to justify it.

### Status quo

No V2-specific decision means Phase 2 keeps Phase 1's one-buffer-per-signal behaviour by default, and the phase's
liveness gate is closed by prose or not at all. The reuse then arrives later, in a phase where the graphs are large
enough that a first aliasing bug is also hard to isolate.

## Evidence

- Phase 1's arena and its explicit deferral of this work: `crates/synth_engine_v2/src/plan.rs:18-26` at `33bf1162`,
  and [EVD-0008's use-site table](../evidence/phase-02/EVD-0008-internal-channel-layout-cost.md#v1-use-site-reads)
  row 6 for the arena's shape.
- The master plan's Phase 2 work list requires "initial liveness analysis so non-overlapping signal lifetimes reuse
  buffer storage" and in-place processing "only when declared safe by the node".
- No measurement supports a *choice between* B and C, and this record does not pretend otherwise: nothing in V2 has an
  arena large enough to measure, and the V1 baselines (EVD-0003, EVD-0007) measure a different engine's memory. The
  decision is therefore made on correctness, determinism, and reviewability, with the upgrade path stated below.

## Decision

Accepted after three review passes over the draft. Two of them changed a clause rather than its wording: in-place
processing contradicted clauses 1 and 2 as first written and is now expressed as a merged value chain, and the third
option was interference-graph colouring, which is a dead end for equal-sized slots over a fixed total order — the
linear scan already reaches that order's minimum, so the real headroom is the schedule, not the assignment.

1. **The unit of liveness is one channel of one signal**, which is exactly one arena slot —
   [ADR-0002](ADR-0002-internal-channel-layout.md) clause 1 makes a buffer always mono, so a stereo signal is two
   independent units with two live ranges, not one wide one. Each unit's live range runs from the operation that
   writes it to the last operation in the compiled schedule that reads it, inclusive of both. Channels of one signal
   commonly share endpoints and just as commonly do not: an operation that reads only the left channel ends that
   channel's range and not the right one's, and nothing here forces them to be assigned together.
2. **Two signals may share a slot only when their live ranges do not overlap** in the compiled schedule order.
   Assignment is a conservative linear scan with a free list — Option B.
   **In-place processing is the one permitted overlap, and it is permitted by merging rather than by exception.** An
   operation that consumes a unit's last read and writes its output unit at the same schedule position produces one
   *value chain*, not two units: the pair is recorded as a single live range running from the input's write to the
   output's last read, occupying one slot. Clause 2 is then stated over value chains, so an in-place alias never
   appears as an overlap and clause 8's structural check needs no special case. Where the compiler declines in-place
   under clause 5, the two are separate chains and clause 2 applies to them unchanged.
3. **Assignment is a pure function of the compiled plan**, performed once at compile time and recorded in it. Two
   compilations of the same plan produce the same assignment, so a digest comparison measures audio rather than
   allocation order.
4. **The render loop performs no part of it.** It reads slot indices; it never computes, compares, or extends a live
   range.
5. **In-place processing requires two independent conditions**, and the compiler checks both: the node declares the
   operation in-place-safe, **and** the input signal's live range ends at that operation. A node that declares safety
   does not thereby get in-place treatment where its input is read again later; the compiler allocates a separate
   output instead.
6. **An observation tap extends the live range of the signal it observes** through the tap operation. A tap is a
   reader, and a signal whose only remaining reader is a tap is still live.
7. **The arena is one allocation of quantum-sized mono slots**, sized at admission and counted in the resource report,
   with the dominant contributor named as `HOST-INV-006` requires.
8. **The strategy is verified by two checks, and neither is optional:**
   - a **structural** check that no two overlapping live ranges were assigned one slot, run over every compiled plan
     in the test suite rather than over a hand-picked example;
   - a **behavioural** check that compiling a plan with reuse *disabled* renders **bit-identical** audio to the same
     plan compiled with reuse enabled. This is what makes a later upgrade to Option C safe: the same test decides it.
9. **Reuse is switchable at compile time for that check only.** The disabled mode is not a supported configuration, is
   not reachable from a host profile, and exists so that clause 8's second check can exist.

## Consequences

### Positive

- Arena size grows with the schedule's live depth rather than with the plan's size, which is the asymptote a mixer
  graph needs.
- The bit-identical reuse-off comparison turns "is the aliasing correct" from a review question into a test, and it
  keeps working unchanged if the strategy is later replaced.
- In-place processing gets a rule that cannot be applied by accident: the declaration alone is not sufficient.

### Negative

- Interval-based liveness holds a slot across a gap that a finer analysis would have reused. Accepted deliberately;
  the direction of the error is wasted memory.
- The compiler carries a free list and a live-range table that Phase 1 did not have, and both have to be correct
  before the first useful render exists.
- The reuse-off mode is code that ships without being a product feature. It is justified by clause 8 and by nothing
  else, and if clause 8's check is ever removed the mode goes with it.

### Risks and controls

- **Risk: an aliasing defect reaches audio.** Controls: clause 8's structural check on every compiled plan, and the
  bit-identical reuse-off comparison.
- **Risk: in-place processing corrupts a signal that is read again.** Control: clause 5's two conditions, with a
  refusal case proving the compiler declines in-place where a later reader exists.
- **Risk: a non-deterministic assignment makes digests incomparable.** Control: clause 3, checked by compiling one
  plan twice and comparing the assignment, not only the audio.

## Follow-up work

| Task | Phase | Status |
|------|-------|--------|
| Implement the arena, the live-range table, and both checks | 2 (P02-T004) | Complete |
| Measure arena occupancy on the first plan large enough to measure, and decide whether Option C is worth revisiting | 8 | Not started |

## Revisit conditions

- A profiled plan whose **peak liveness** is materially higher than a different valid topological order would produce,
  on a graph the product actually renders. That is the quantity Option C would attack; a colouring assignment over one
  fixed order would attack nothing, because the linear scan already reaches that order's minimum.
- A scheduling change that makes the schedule order no longer a total order — parallel execution across cores would do
  this — since clause 2 is stated over that order.
