# ADR-0041: Interleaved Internal Channel Layout

| Field         | Value                                                                                   |
|---------------|-----------------------------------------------------------------------------------------|
| ID            | ADR-0041                                                                                |
| Status        | Proposed — the decision is made; see *Status* below for what acceptance still needs      |
| Phase         | 2                                                                                       |
| Created       | 2026-08-18                                                                              |
| Last reviewed | 2026-08-18                                                                              |
| Related       | ADR-0002, ADR-0040, ADR-0005, ADR-0004, EVD-0008, EVD-0010, P02-T012, P02-T013           |
| Supersedes    | ADR-0002 in full; ADR-0005 clauses 1, 2, 4, 5, 7 and 8                                  |
| Superseded by | —                                                                                       |

## Status

The **decision** in this record is the user's, made on 2026-08-18 with
[EVD-0010](../evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md) in front of it: V2 owns its DSP
([ADR-0040](ADR-0040-v2-owns-its-dsp.md) option B) and the internal arena becomes **interleaved**.

The **record** is `Proposed` rather than `Accepted` because the working agreement's decision rule 2 forbids accepting a
`Contract`-class ADR in the session that drafts it, and requires a reader who did not author it. Nothing about the
choice is open; what is open is the review. Until then ADR-0002 remains the standing authority, on the withdrawn
premise that is why the phase is stopped.

**Accepting this record is one transaction of seven edits, and a partial application is worse than none** — the register
contract requires every status consumer to agree, so stopping halfway leaves two authorities disagreeing about which
layout the engine has:

1. this record to `Accepted`, with the reviewer named;
2. [ADR-0002](ADR-0002-internal-channel-layout.md) to `Superseded`, its text untouched;
3. [ADR-0040](ADR-0040-v2-owns-its-dsp.md) to `Accepted`, which its clause 7 has been waiting for;
4. [ADR-0005](ADR-0005-buffer-liveness-strategy.md)'s header to record that clauses 1, 2, 4, 5, 7 and 8 are superseded or refined;
5. the **register** — `plans/v2/ADR.md`, both the status table and the record entries for ADR-0002, ADR-0005,
   ADR-0040 and this record;
6. the **status consumers** — `plans/v2/STATUS.md` and the
   [Phase 2 tracker](../phases/phase-02-minimal-compiled-voice-graph.md), whose *Decided, pending one review* section,
   decision table and next actions all carry the pending state, and the
   [master plan](../master-plan.md)'s two layout directives, which are written as effective on this acceptance;
7. the **specification and inventory consumers**, which an earlier draft of this list missed and which an independent
   read found: [`spec-host-profile-and-render-limits.md`](../specs/spec-host-profile-and-render-limits.md) still calls
   ADR-0002 `Proposed`, assigns `channel_layout`'s meaning to it, and lists it as an unresolved owner; and
   [`resource-limits.md`](../inventories/resource-limits.md)'s `LIMIT-0059` hands the live `Multi(n)` construction path
   to ADR-0002 to decide. Both name this record instead — and `LIMIT-0059` is worth reading during P02-T013 rather
   than only re-pointing, because a device reporting more than two channels is exactly the case an interleaved arena
   has to widen or refuse.

**What the transaction must not touch.** Phase 1's tracker and its exit review both record ADR-0002 as `Proposed`,
and that is correct: they state what was true when that phase closed, and a closed phase's record is history rather
than a status consumer. Editing them would make the archive agree with the present at the cost of no longer recording
the past. The same test applies to anything else the transaction reaches: change it if it describes what is true now,
leave it if it describes what was true then.

Nothing in the phase proceeds before all four: P02-T006 closes on ADR-0040, and P02-T013 — the conversion — is written
against clauses 1 to 17 below.

### Review history

**The first independent read refused this record**, and the refusal is kept here rather than smoothed away, because
four of its findings were holes a `Contract` cannot have and one of them was in the acceptance transaction itself:

| Refused because | What the record says now |
|-----------------|--------------------------|
| Variable-width slots were introduced with "first fit by width" and nothing else, while ADR-0005's clauses 5 and 8 — in-place safety and the anti-aliasing check — silently assumed equal sizes | Clause 13 is an allocator contract: regions as offset and length, first fit by ascending offset, splitting, coalescing, and a stated extent. Clause 14 supersedes ADR-0005 clauses **1, 5, 7 and 8**, adds width compatibility to in-place, and strengthens the structural check to physical-range non-intersection |
| Clauses 3 and 12 contradicted each other: control signals are always mono, yet every kernel had to be tested at two channels — and the envelope is a mono-only control node | Clause 12 is scoped to the counts a kernel's **own ports admit**, with a test asserting the port table for the mono-only ones |
| Clause 15's acceptance check was not executable: no fixture set, no render length, no events, no digest artifact, no command | Clause 16 names a fixture manifest, a per-fixture digest committed from the planar build, a comparison test, and the four plan shapes the manifest must contain |
| The acceptance transaction would have left the host-profile specification and the resource-limit inventory pointing at ADR-0002 | Item 7 of the transaction adds both |
| Four factual slips: "three places" encode uniform width (there are five), "the catalog is five kinds" (eight kernels; five is the fixture's node count), a conversion appears in the resource report's "buffer count" (that is the plan's; the report carries scratch bytes), and EVD-0010's qualifications were not repeated where its figures were used | All four corrected in place, each saying what it previously said |
| "Every mono path pays nothing" was not established, since EVD-0010 measured today's kernels rather than the channel-aware ABI clause 4 requires; ADR-0002's down-mix risk and its oversampling exclusion were dropped by a successor claiming to be readable alone | Clause 3 states the limit, *Risks* carries the mono-regression risk with a re-measurement as its control, and both of ADR-0002's items are restored |

**The second read refused it too**, and on a shorter list — three of its four items were consequences of the first
round's own amendments:

| Refused because | What the record says now |
|-----------------|--------------------------|
| Clause 13 was deterministic in its free list and not in its **sequence of requests**, and left arena growth and the meaning of "extent" unstated | The allocation order is now total — ascending schedule index of the operation writing each chain's first value — growth appends at the extent when nothing fits, and the extent is the **exclusive end**, which is the only reading that yields a sample count |
| Clauses 2 and 4 of ADR-0005 were listed as unaffected while the record changed what their words denote: "share a slot" became physical intersection, and "slot indices" became offsets and lengths | Both are listed as **refined**, with the change spelled out and the principle each protects noted as intact. The scope is now clauses 1, 2, 4, 5, 7 and 8 |
| Clause 16 described how to write an acceptance specification instead of being one: no fixture parameters, no digest representation, and a whole-render digest cannot locate a differing frame | Clause 16 names five fixtures with their profiles, quantum count and event script, fixes the digest as SHA-256 over little-endian `f32` bit patterns, and makes it **one digest per quantum** so the first differing quantum is reportable |
| Three inconsistencies the amendments introduced: a transaction announced as six edits and listing seven, a follow-up still demanding `c = 1` and `c = 2` against clause 12's port-scoped rule, and clause references left behind by renumbering | All corrected |

**A third read closed clause 13 and refused clause 16 on one line**: the text promised the *first differing frame*
while a per-quantum baseline can only name the quantum. Rather than commit 82 000 lines of per-frame digests across
the five fixtures, the clause now states its resolution honestly — one quantum, 1.33 ms — and the failing test dumps
that quantum's rendered samples so the developer has one half of the comparison in hand and a one-command recipe for
the other.

**A fourth read refused clause 16 on its fixtures**, which is the round that turned a described test into a buildable
one. The list named no parameter values, execution scopes or identities; it gave a gate to two fixtures that have no
envelope to gate; it asked for a reuse plan the compiler cannot build, since the output node declares one input port
and validation admits one source per port; and it gave the unpatched-input fixture a region that was already zero, so
the fixture could not tell a kernel that writes silence from one that writes nothing. Each fixture is now a compilable
graph with its buffer and slot counts read off the planar build, the two ungated fixtures say they are ungated, and
both the reuse and the unpatched case are built on a **disconnected branch** whose region is freed and handed on. The
first `render` call is written down with them: it returns the primed carry, renders no quantum, and refuses an event —
which is what would otherwise have shifted every baseline line by one.

## Context

[ADR-0002](ADR-0002-internal-channel-layout.md) made V2's arena **planar** — one buffer is one channel — and was
accepted **against its own measurement**. [EVD-0008](../evidence/phase-02/EVD-0008-internal-channel-layout-cost.md)
selected the interleaved arena by a median 9.3% end to end, and ADR-0002 took planar anyway on a single architectural
argument: Phase 2's recorded execution choice was **kernel extraction into a shared home both engines call**, V1 is
planar, and a kernel written over `&mut [f32]` therefore serves V1 unchanged. That record wrote the cost of the choice
into itself rather than waving it away, and it named the condition under which the answer flips: *"If the shared-kernel
constraint is dropped — a V2 that owns its own kernels, or a kernel interface expressed over a channel-strided view —
the measurement selects interleaved."*

[ADR-0040](ADR-0040-v2-owns-its-dsp.md) drops exactly that constraint. Its clause 7 says so in advance and refuses to
be accepted alone: accepting it leaves a `Contract`-class decision resting on a premise that no longer exists, and the
working agreement's rule for two authorities in conflict is to stop the dependent work and repair the non-authoritative
copy. That is what this record does.

Between the two, P02-T012 replaced EVD-0008's model with a measurement of the real thing.
[EVD-0010](../evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md) ran the crate's own kernels over the
crate's own arena, in three shapes, with six review-found asymmetries closed before any data was collected and both
arms required to render bit-identical carries at every quantum of a 200-quantum settle. Two qualifications travel with
every figure it produced and are repeated wherever this record uses one: **only shape A is constructible by the
compiler**, and the **interleaved kernels are hand-written counterparts** of the crate's, held to it by bit-identity
rather than by being the same code.

**Outside this decision.** Which layouts exist beyond `Mono` and `Stereo`, which Phase 8's mixer metadata and Phase 9's
device negotiation own; the summing law for a stereo-to-mono down-mix, still refused here for the same reason ADR-0002
refused it; **sample-rate conversion and oversampling islands**, which are Phase 8's and which this record does not
touch even though both would introduce buffers of a different length — carried over from ADR-0002's own exclusion list
because a standalone successor that dropped it would read as having decided it; buffer *storage assignment*, which
[ADR-0005](ADR-0005-buffer-liveness-strategy.md) owns except for the clauses this record supersedes; and how a node is
declared, which ADR-0004 and Phase 5 own.

## Decision drivers

- **The premise is gone, not weakened.** ADR-0002's clauses were accepted on the shared-kernel argument and on nothing
  else — the record says so in its own first paragraph. With ADR-0040 taken, retaining planar would mean retaining a
  `Contract` on a rationale its author explicitly did not make.
- **The measured cost, on the real path and on the path it becomes.** EVD-0010: interleaved cheaper in **all nine runs
  of all three shapes**, against control spreads of 0.37% to 0.50% — **2.54%** on the plan a stereo profile compiles
  today, **21.56%** on a stereo chain, **11.05%** with a control signal per channel.
- **The two large shapes are projections, and that is stated rather than smoothed.** No node kind produces a stereo
  signal yet, so the compiler cannot build shape B or shape C. What they establish is the *shape of the cost curve* —
  small while signals are mono, 11% to 22% the moment one is not — not a figure the phase is paying today.
- **The cheapest moment is now, and ADR-0002 is the record that says so.** Its own text: the change would touch the
  arena, ADR-0005's liveness unit, the port and edge model, the conversion operations, the in-place rule, the resource
  report *"and every node kernel's signature"*, and that set *"grows monotonically with the node catalog, so the
  cheapest moment to flip is now and the most expensive is exactly the moment V1 retires."* The catalog is **eight
  kernels** today — `IrNodeKind` has nine variants and eight of them have one — against the forty-odd V1 modules Phase
  5 is meant to bring across. An earlier draft of this record said five, which is the count of *authored nodes in the
  minimal voice fixture*, not of the catalog.
- **What the measurement cannot price, recorded because it is the argument this decision overrides.** ADR-0002 clause
  2's kernel contract — *"a kernel receives one channel and is never told how many there are"* — is a real
  simplification for a catalog that is meant to grow to forty-odd kinds, and no harness can put a number on it. It is
  given up deliberately, and clause 12 below is what replaces the safety it provided.

## Options considered

### Option A: Retain planar, on a rationale that survives without the shared kernel

Keep every ADR-0002 clause and re-found it on clause 2's kernel contract: every kernel is written for one channel,
never learns a channel count, and cannot get channel handling wrong because it has none to get wrong. Phase 5's
`LegacyPolyModuleAdapter` also stays cheap, because V1's modules are mono-buffer-per-port by construction.

The cost is EVD-0010's figures: 2.54% now, and 11% to 22% on any path whose signal has channels — paid forever,
because the flip only gets dearer.

### Option B: Interleaved — **chosen**

A signal is one buffer of `Q` frames of `c` channels. Mono signals are unchanged, so the mono half of every path costs
exactly what it costs today; stereo work stops being *n* separate traversals with *n* separate node calls; and the host
boundary becomes a copy instead of a transpose.

The cost is that a kernel now knows its channel count and must be right for every value of it, and that Phase 5's
adapter has to convert at every V1 module port. Both are stated in *Consequences* rather than discovered.

### Option C: Defer until a node produces stereo

Superficially attractive, because shape A — the only shape that exists — is worth 2.54%, and the two shapes worth
11% to 22% are projections. It is rejected on ADR-0002's own reasoning: deferring means paying the conversion when the
catalog is large rather than when it is five kinds, and the record already establishes that nobody pays a catalog-wide
rewrite later. Deferring would also leave ADR-0002 standing on a withdrawn premise in the meantime, which is the exact
state ADR-0040 clause 7 exists to prevent.

## Evidence

- [EVD-0010](../evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md), the measurement this record is
  decided on: nine runs, three shapes, every margin outside its own control spread, and the arms proved to compute the
  same signal bit for bit before anything was timed. Its limitations are part of what is being weighed — in
  particular that shapes B and C are projections and that its interleaved kernels are hand-written counterparts.
- [EVD-0008](../evidence/phase-02/EVD-0008-internal-channel-layout-cost.md), which reached the same direction from a
  model and whose corrections table records how easy this comparison is to get wrong.
- **What the crate assumes today**, read for the conversion's blast radius rather than asserted. Four places encode
  *uniform* slot width, which is what clause 2 changes: `bind` resolves a slot as `slot.index() * quantum`
  (`crates/synth_engine_v2/src/node/kernels.rs:403`), the renderer's output operation does the same
  (`crates/synth_engine_v2/src/render/hot.rs:278`), the arena is allocated as `buffer_count * quantum`
  (`crates/synth_engine_v2/src/render.rs:352`), and admission accounts for it as `arena_buffers * Q`
  (`crates/synth_engine_v2/src/compile.rs:682`). The arena assignment itself is a fifth: it allocates identical
  slots, which is what clause 13 replaces. An earlier draft said "three places" and an independent read found the
  other two.
- **Not measured, and named as such:** the cost of Phase 5's `LegacyPolyModuleAdapter` converting at every V1 module
  port. It is the largest quantity this decision moves and nobody has put a number on it; the follow-up table has the
  task.

## Decision

**The internal arena is interleaved.** This record supersedes [ADR-0002](ADR-0002-internal-channel-layout.md) **in
full** rather than clause by clause: three of its eight clauses reverse, and they are the three every other clause is
read through, so a reader would otherwise need both records for most questions. The clauses that survive are restated
here in their own terms, which is what makes this record readable alone.

1. **Internal audio is interleaved. A buffer holds `Q` frames of `c` channels**, contiguously, frame-major: sample
   `(f, ch)` is at `f * c + ch`. `c` is the channel count of the signal's layout.
2. **A signal occupies exactly one arena slot, whatever its channel count.** Slot width is `c * Q` samples, so slots
   are **no longer uniform** and a slot's position in the arena is an offset the plan records rather than an index
   multiplied by the quantum.
3. **A mono signal is `Q` contiguous samples**, which is bit-for-bit the arrangement the crate has today. Every mono
   path — and every control signal, which is always mono — keeps that arrangement. What this record *cannot* promise
   is that a mono path costs the same afterwards: EVD-0010's mono arms ran **today's** kernels, not the
   channel-aware ones clause 4 requires, so the storage is unchanged while the kernel ABI around it is not. The risk
   and its control are in *Consequences*.
4. **A kernel is told how many channels it has**, and must be correct for every count the build can construct. This is
   the reversal of ADR-0002 clause 2, and it is the price of this decision rather than an incidental detail; clause 12
   is the check that keeps it from being paid in silent defects.
5. **Channel layout is a property of a port and its edge**, as before, and it now also fixes the width of the buffer
   the signal occupies. A layout the build cannot render is not constructible.
6. **`ChannelLayout` is an ordered sequence of channels.** Ordering is part of the layout: channel 0 of a stereo signal
   is the left channel, and no conversion, copy, or boundary operation may permute silently. Unchanged from ADR-0002
   clause 4, and it is the clause that makes the vocabulary extensible without touching clauses 1 to 3.
7. **This phase admits `Mono` and `Stereo` and refuses to invent the rest.** Unchanged from ADR-0002 clause 5.
8. **The only implicit conversion Phase 2 inserts is mono to stereo**, by duplicating each sample into both channels of
   **one wider buffer**. A stereo-to-mono edge is a compile error naming the edge, both endpoints and both layouts —
   not a silent down-mix, and the summing law stays a product decision with no caller in this phase. Unchanged from
   ADR-0002 clause 6 except for what the conversion writes into.
9. **Every conversion is a scheduled operation with an identity**, appearing in the schedule, in the compiled plan's
   buffer count, in the arena extent the resource report states as scratch bytes, and in diagnostics. Unchanged from
   ADR-0002 clause 7 except in naming where the count actually lives: `buffer_count` is the plan's, and the report
   carries `BufferScratchBytes`.
10. **A layout mismatch that no permitted conversion resolves is refused at compile time**, before admission reports
    resources, with a path-local diagnostic. The renderer never sees a plan with an unresolved layout. Unchanged from
    ADR-0002 clause 8.
11. **Interleaving is no longer a boundary operation; matching the host's layout is.** Where a plan's output signal has
    the stream's layout, the output operation is **one contiguous copy** of one buffer into the carry, replacing the
    per-channel strided writes the renderer performs today. Where it does not, the conversion that resolves it is an
    ordinary scheduled operation under clause 9. External boundaries still own their own arrangement: a file writer or
    a device input converts at the edge, never by reinterpreting an arena slot.
12. **Every kernel is tested at every channel count its own ports admit**, and a node kind may not be added without
    them. Not "at `c = 1` and `c = 2`" flatly: clause 3 makes every control signal mono, and the envelope is a
    control-domain node whose output port is mono by declaration, so a stereo envelope is not constructible and a rule
    demanding one would be unsatisfiable. The obligation is therefore: an **audio** kernel is tested at every count
    its ports admit — today one and two — and a kernel whose ports admit only one is tested at one, **with a test
    asserting that its port table admits only one**, so the exemption is checked rather than assumed. This is what
    replaces the safety ADR-0002 clause 2 gave away for free.
13. **A slot is an offset and a length, and the allocator has a contract rather than an adjective.** Equal-sized
    slots let ADR-0005 leave the assignment to the phrase *"a conservative linear scan with a free list"*; mixed
    widths do not, because every question that phrase used to answer implicitly now changes the memory a plan needs
    and, worse, could change it between two compilations of the same plan. The contract:
    - a **physical region** is `(offset, length)` in samples within one allocation, where `length = c * Q`;
    - **value chains are allocated in one order and it is total**: ascending index, in the compiled schedule, of the
      operation that writes the chain's first value. Every operation writes exactly one output, so no two chains
      share that index and no tie-break is needed. Without this the free list is deterministic and the *sequence of
      requests* is not, which is the same non-determinism one level up;
    - the free list holds physical regions, ordered by ascending offset, and assignment is **first fit by ascending
      offset** — the first hole whose length is at least the requested length;
    - a hole strictly larger than the request is **split**, the remainder staying free at the higher offset; a wider
      region is never handed to a narrower signal without splitting, so no slack is silently carried;
    - **when no hole fits, the arena grows**: the region is appended at the current extent, exactly `length` wide.
      Growth is the fallback, never the first choice, which is what makes reuse the normal path;
    - a region returned to the free list **coalesces** with any free region it abuts on either side;
    - the arena's **extent is the exclusive end** — the greatest `offset + length` over every assignment — because
      that, and not the greatest offset, is the number of samples the allocation must hold. Admission reports it;
    - the whole procedure is therefore a pure function of the compiled plan, which is what lets ADR-0005 clause 3's
      determinism survive. It matters more here than under equal slots: a digest comparison must measure audio, not
      the allocator.
14. **[ADR-0005](ADR-0005-buffer-liveness-strategy.md) clauses 1, 2, 4, 5, 7 and 8 are superseded or refined;
    clauses 3, 6 and 9 stand unchanged.** Two of those six are *refinements* rather than reversals, and they are
    listed because a successor that called them "unaffected" while changing what their words denote would be the
    quiet kind of wrong:
    - **clause 2** — its rule is unchanged and its terms are not: "two signals may share a slot only when their live
      ranges do not overlap" is now read over **physical regions** rather than over slot identity, because with mixed
      widths two distinct slots can still intersect in samples;
    - **clause 4** — the render loop still performs no part of liveness, which is the clause's whole point; what
      changes is the interface it reads. It reads a recorded **offset and length** where it read a slot index, and
      `bind` resolves regions rather than multiplying an index by the quantum;
    - **clause 1** — the unit of liveness becomes **one signal**, not one channel of one signal: a stereo signal is
      one live range over one region, not two independent ones;
    - **clause 5** — in-place processing keeps its two conditions and gains a third: the input's and the output's
      **layouts must be identical**, so their regions have the same length. A node cannot write a stereo output over
      a mono input's region, and the compiler allocates separately where the widths differ;
    - **clause 7** — the arena is one allocation of **variable-width** regions, still sized at admission and still
      reported;
    - **clause 8** — the structural check becomes stronger, because slot identity no longer implies extent: it is no
      longer enough that two overlapping live ranges were given different *slots*. The check is that their
      **physical sample ranges do not intersect at all**, partial overlap included, which is the defect variable
      widths make possible and equal widths made unrepresentable. Clause 8's second, behavioural check — a
      bit-identical render with reuse disabled — is unchanged and still mandatory.
    **Clause 3 is the one this record leans on hardest and does not touch**: assignment stays a pure function of the
    compiled plan, and clause 13's total ordering plus its stated growth and extent rules are what make that true of
    variable widths.
15. **ADR-0005's optimality argument is withdrawn, and nothing replaces it.** That record rejected interference-graph
    colouring because *"with equal-sized mono slots and live ranges that are intervals over one total schedule order,
    the interference graph is an interval graph, and a left-edge scan with a free list already achieves the minimum."*
    Variable widths void the premise: first fit over mixed sizes is a storage-allocation problem, not an
    interval-colouring one, and it can leave holes a better assignment would not. This record makes **no optimality
    claim**; the direction of the error is wasted memory rather than wrong audio, which is what clause 14's two checks
    police.
16. **The conversion is verified by a bit-identical render against a committed baseline.** "Every plan the test
    suite compiles" is not a check an implementer can run — the suite also compiles plans that are refused and plans
    that are never rendered — so the test is specified here rather than described. An implementer who follows this
    clause and nothing else has enough:

    **The harness, shared by all five.** 48 kHz, `Q` = 64 ([ADR-0037](ADR-0037-render-quantum-value.md)), a harness
    profile whose maximum block size is **64 frames**, so one `render` call is exactly one quantum; the stream
    anchored at `SampleTime::ZERO` and `PlanPosition::ZERO`; the arena in `Reuse`, its only policy a host profile
    reaches. Every value below was read off the planar build rather than reasoned out, because a fixture whose
    parameters are left to its implementer is a different fixture in every hand.

    **257 calls, 256 digested quanta.** `PreparedRenderer::prepare` primes the output carry with `Q` frames of
    silence — ADR-0001 clause 6 — so the **first** `render` call of exactly `Q` frames returns that primed silence,
    renders no quantum, and **refuses any event presented with it**: the call fails with `EventOutsideCallSpan`
    rather than gating anything. The harness therefore makes **257** calls and digests calls 1 to 256, so baseline
    line `k` is call `k + 1`. Unstated, this shifts every line of every baseline by one.

    **The fixtures.** Identities, kinds, parameter values, execution scopes and edges are given, because every one of
    them changes the samples. All five end in node `9`, an `Output` at `Global` scope; every edge leaves its source's
    first port, and the destination port is named only where it is not the first.

    1. **The minimal voice path, mono.** `1` `Envelope { attack 0.010 s, decay 0.100 s, sustain 0.700, release
       0.200 s }` at `Voice`; `2` `Sine { frequency 440 Hz, amplitude 0.5 }` at `Voice`; `3` `Filter { cutoff
       1 000 Hz, resonance Butterworth }` at `Voice`; `4` `Amplifier` at `Voice`. Edges `2 → 3`, `3 → 4` and
       `4 → 9` are `Audio`; `1 → 4` is `Control` into the amplifier's **second** port. It compiles to four virtual
       buffers in two slots, the filter and the amplifier both merged in place.
    2. **The same graph at a stereo profile**, which is the one that exercises the widening: five virtual buffers in
       two slots, where the widening copy takes the slot the envelope's chain has just freed.
    3. **A merged chain**, ungated, mono: `2` `Sine { 440 Hz, 0.5 }` at `Voice` → `5` `Gain { factor 0.5 }` at
       `Global` → `6` `Gain { factor 0.25 }` at `Global` → `9`, every edge `Audio`. Three virtual buffers in one
       slot: ADR-0005 clause 5's in-place path, taken twice.
    4. **A reuse plan**: fixture 1's graph and event script, mono, plus `40` `Constant { level 0.5 }` at `Global`
       **connected to nothing**. Its value is written and never read, so its region is free at the operation that
       writes it and is handed on — five virtual buffers in two slots, slot 0 holding the constant's dead value and
       then the whole sine-to-output chain. **One region assigned twice**, which is what this fixture is for. An
       earlier draft asked instead for *two independent chains into one output*: no such plan exists, because the
       output node declares **one** input port and validation admits one source per port, and a record may not ask
       for a fixture the compiler refuses.
    5. **An unpatched input**, ungated, mono: `40` `Constant { level 0.5 }` at `Global` connected to nothing, and
       `3` `Filter { 1 000 Hz, Butterworth }` at `Voice` whose input port is **unconnected**, into `9`. Two virtual
       buffers in one slot — and the filter's output *is* the region the constant filled with 0.5. That is the whole
       power of the fixture, and why the constant is in it: the kernel must write silence over a region holding
       something else, so a conversion that widens a region without widening the write renders 0.5 where the
       baseline holds zeros. A filter alone would be handed a region that was already zero and could not tell a
       kernel that writes silence from one that writes nothing. On the planar build it renders silence.

    **The event script.** Fixtures 1, 2 and 4 carry the envelope and are gated through the slot
    `CompiledPlan::resolve_parameter(node 1, parameters::ENVELOPE_GATE)` returns: `SetParameter` with value `1.0`
    presented with **call 1** at `SampleTime::new(0)`, and `ParameterValue::ZERO` presented with **call 193** at
    `SampleTime::new(12_288)`, the first sample of quantum 192 — which is baseline line 192. Both are stamped in the
    stream's own epoch with `TimeSource::Compiled`. Any value above zero raises the gate and zero lowers it, so each
    edge is one event and no other event exists. Fixtures 3 and 5 have no envelope and therefore no slot to address:
    they render with `TimedEvents::EMPTY` on every call, and a gate written for them would name a parameter that does
    not resolve. The release is 0.200 s while the gate falls 63 quanta — 84 ms — before the last digested one, so
    every gated fixture ends **on a moving release ramp**: a tail that stops moving is a difference the last lines
    catch.

    **The digest.** For each fixture, one digest **per digested call**, over the `Q` frames that call wrote: every
    sample's IEEE-754 `f32` bit pattern as four little-endian bytes, in frame order with the frame's channels
    adjacent, hashed with SHA-256 and rendered lowercase hexadecimal. It is taken over the **caller's block**, which
    is interleaved on both builds, so the conversion cannot change what the digest means — only whether the samples
    are the same. Per quantum rather than per render, and that is what makes the failure legible: a whole-render digest
    can say *that* something differs and never *where*, so the baseline is 256 lines per fixture —
    `<quantum index>,<digest>`, the index running 0 to 255 and naming the quantum call `index + 1` rendered — and the
    test reports the first line that differs.

    **The resolution is one quantum, not one frame, and the record says so rather than implying better.** A
    per-quantum digest localizes a difference to 1.33 ms at 48 kHz and cannot name the frame inside it, because the
    baseline holds no expected samples to compare against. Per-frame digests would give that resolution and cost
    16 384 lines per fixture — 82 000 across the five — for a localization a developer can get in one step anyway, so
    they are declined deliberately. What the failure gives instead: the test **writes the rendered samples of the
    first differing quantum to a file** and names it in the failure message. The other half of the comparison is one
    command away — check out the baseline commit, run the same fixture, diff the two dumps — and that recipe belongs
    in the test's own documentation.

    **The artifact and the order.** The baseline is generated **from the planar build, in the commit immediately
    before the conversion**, and committed beside the test as one text file per fixture. Generating it afterwards
    would be writing the answer down after seeing it.

    **The comparison.** A test renders each fixture and compares per-quantum digests to the committed lines, failing
    with the fixture's name, the **first differing quantum index**, both digests, and the path of the dump it wrote.
    A fixture whose baseline file is missing fails; it does not skip.

    The arithmetic is not changing, only where the samples live, so any difference is a defect rather than a
    renegotiation. EVD-0010 already demonstrated that both layouts can render one signal bit for bit over 200
    consecutive quanta, so this is a check the phase knows can pass.
17. **It lands as one task, before another node kind is added.** P02-T013, tracked in the phase, with P02-T007 waiting
    on it: every node kind added before the conversion is a kernel to rewrite during it.

## Consequences

### Positive

- The measured figures, at the top of this record, on both the path that exists and the paths it becomes.
- A stereo signal is one buffer and one node call per operation instead of two of each — fewer scheduled operations,
  fewer bindings, one traversal.
- The host boundary becomes a copy. The transpose the renderer performs today goes away, and with it the strided
  per-channel writes whose current implementation EVD-0010 measured at **45.8 ns per quantum** more than a
  frame-strided form.
- Cross-channel work — mid/side, correlation, any operation that wants both channels at once — becomes expressible
  without gathering two buffers. ADR-0002 listed this as a real ergonomic cost paid by a minority of nodes; that cost
  is now refunded, and the majority pays instead.

### Negative

- **Every stateful kernel becomes channel-aware.** Per-channel state arrays and frame-wise stepping, in place of a
  loop over one channel's samples. EVD-0010's interleaved kernels are exactly that shape, and they are visibly more
  code than their planar counterparts. This is the largest recurring cost of the decision and it is paid once per node
  kind, forever.
- **Phase 5's `LegacyPolyModuleAdapter` has to convert at every V1 module port**, in both directions, per quantum. V1's
  `AudioBuffer` is a flat mono buffer per port by construction, so an interleaved V2 hosting a V1 module de-interleaves
  in and re-interleaves out. **Nobody has measured this**, it applies to the whole legacy catalog rather than to five
  nodes, and it is the one quantity that could make this decision look wrong in retrospect. The follow-up table has
  the measurement, and the revisit conditions have the trigger.
- **The arena needs more memory in every shape EVD-0010 measured**, because a mono-to-stereo widening cannot reuse a
  mono slot: 3 `Q` slots against planar's 2 for the path compiled today, 4 against 3 for shape C. At `Q` = 64 this is
  below measurement; on a large graph it is a direction, and it points against this decision.
- **Variable-width slots complicate the compiler**, void ADR-0005's optimality argument (clause 15), and break the
  uniform-stride assumption five places in the crate encode today — `bind`, the renderer's output operation, and the
  arena allocation, all cited above.
- **A mono operation on one channel of a stereo signal** now needs a stride or a de-interleave, where planar expressed
  it as an ordinary contiguous pass. This is the mirror image of the ergonomic cost ADR-0002 accepted, and it lands on
  a different minority of nodes.
- **V2 loses the argument that its kernels are V1's kernels**, which ADR-0040 already spent — noted here only because
  the two decisions are one choice and this is where its second half is paid.

### Risks and controls

- **Risk: a channel is silently swapped** by a conversion, a copy, or the boundary. Control: clause 6's ordering rule,
  with a test that a stereo signal whose channels carry distinguishable content survives every conversion and the
  boundary write in order.
- **Risk: a kernel is correct for stereo and wrong for mono, or the reverse.** This is the defect class ADR-0002
  clause 2 made unconstructible. Control: clause 12 — every kernel tested at every constructible channel count, as an
  obligation on adding a node kind.
- **Risk: the conversion changes what the engine sounds like.** Control: clause 16's per-quantum digest comparison
  against a baseline generated from the planar build in the commit before the conversion.
- **Risk: the adapter cost is discovered in Phase 5, when it is too late to matter.** Control: partial, and stated as
  such. The follow-up table schedules the measurement before Phase 5 commits to the adapter, but nothing in this
  record forces Phase 5 to wait for it, and the revisit conditions carry the consequence if it comes back large.
- **Risk: variable-width assignment wastes materially more than the equal-slot scan did.** Control: clause 14's
  structural and bit-identical checks still apply, and clause 15 refuses to claim an optimality nobody has measured.
  The direction of that error is memory rather than audio, which is why it is a risk and not a defect class.
- **Risk: the mono path gets slower, and the decision was taken on figures that could not see it.** A channel-aware
  kernel carries a count, and may carry a loop over it, where today's kernel carries neither; EVD-0010 measured the
  storage question with the current kernels, so a regression here would be invisible to every figure in this record.
  Control: **P02-T013 re-measures the mono path after the conversion**, against the pre-conversion figure, under
  EVD-0010's own discipline — paired controls, group rotation, and the same fixtures. A regression outside the control
  spread is a finding to fix or to record, not a rounding error to absorb.
- **Risk: a down-mix is added later as a quiet default.** Retained verbatim from ADR-0002, because a standalone
  successor that dropped it would make the next person's silent stereo-to-mono edge easier, not harder. Control:
  clause 8 makes it a refusal today, so adding one requires a decision rather than an edit.

## Follow-up work

| Task | Phase | Status |
|------|-------|--------|
| **P02-T013** — convert the arena, the compiler, the kernels and the report to interleaved, verified by clause 16's per-quantum digest comparison | 2 | **Next**, and P02-T007 waits on it |
| Every kernel tested at every channel count **its own ports admit**, under clause 12, with the mono-only ones asserting their port table | 2 | Part of P02-T013 |
| Re-measure the **mono path** after the conversion against its pre-conversion figure, under EVD-0010's discipline | 2 | Part of P02-T013; the control for the risk clause 3 names |
| **Measure the `LegacyPolyModuleAdapter`'s conversion cost** against a V1 module's per-quantum work, as an `EVD` record | 5, measured earlier if cheap | Not started — the largest unmeasured quantity this decision moves |
| Re-run P02-T009's CPU comparison against V1 **after** the conversion, since it is the arena being compared | 2 | Not started; already ordered behind P02-T010 |
| Multichannel layout vocabulary under clause 6 | 8/9 | Not started |
| The down-mix law, if a caller ever needs one | 8 | Not started |

## Revisit conditions

- **The adapter measurement comes back large enough to dominate Phase 5**, at which point the question is whether the
  adapter converts or whether V2 grows a planar island for hosted modules — not whether this record was wrong.
- **A profiled plan where variable-width assignment wastes materially more memory** than the equal-slot scan it
  replaced, which is the cost clause 14 declines to bound.
- **A node class whose kernel is fundamentally per-channel** and cannot be expressed over an interleaved frame without
  a de-interleave that dominates it — the mirror of ADR-0002's own third revisit condition.
- **V1 retires**, which removes the adapter question entirely and with it the largest remaining argument on the other
  side.
