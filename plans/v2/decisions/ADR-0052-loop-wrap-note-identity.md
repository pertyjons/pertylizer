# ADR-0052: Where a loop wrap's note identity comes from

| Field | Value |
|---|---|
| ID | ADR-0052 |
| Status | Proposed |
| Phase | 3 |
| Created | 2026-08-31 |
| Last reviewed | 2026-09-01 |
| Related | ADR-0023, ADR-0032, ADR-0046, ADR-0047, ADR-0050, ADR-0055, `SPEC` sound core render contract, `SPEC` host profile and render limits |
| Supersedes | — |
| Superseded by | — |

## Status note

**This record is `Proposed` and deliberately takes no decision.** Three designs were put to independent design
consultation and all three were refuted against the accepted contract and the code. What the rounds established is
a **coupled boundary** the topic cannot be decided without: a loop wrap is correct only at a granularity ADR-0050
clause 1 deferred. `PROCESS.md` is explicit that an option which "cannot be implemented safely until another
undecided policy is resolved" is not accepted — "either decide the coupled boundary together or keep the ADR
`Proposed` or `Deferred`", and that "replacing one phase prerequisite with a new prerequisite is not progress by
itself".

So this record is the frame rather than the answer: the constraints an implementable wrap must satisfy, the four options
that are closed and the one that is not, and the questions that have to be decided together. Each is written down because the
alternative is rediscovering it, which two of these rounds already did once.

## Durable boundary

**Delivered behaviour, and a real-time ownership boundary**, the same pair ADR-0050 crossed.

What a loop wrap does to a sounding note is audible, and which note a release ends is not recoverable once wrong.
The ownership half is sharper here than for a seek. ADR-0046 clause 4 promises that once accepted, a wrap cannot
fail **for compiled capacity** — the qualifier is the clause's own and this record keeps it, because an earlier
draft dropped it and an independent review caught the misquotation. What the clause settles is that the admission
already done is enough; what it does not settle is whether a wrap may be refused for some *other* reason, and that
question is what makes this an ownership boundary rather than an implementation detail. `PROCESS.md`'s
durable-decision test names a real-time ownership boundary as requiring a record.

**Why the topic is open now rather than later.** ADR-0050 originally recorded a loop interval without enforcing it.
ADR-0055 now fails closed at the offer: a loop-bearing candidate can pass its off-thread bounds, but it cannot enter
active transport state until this record's coupled sample-exact contract is resolved. The loop's *admission* remains
built — `admit_loop` and `admit_loop_polyphony` both have callers and both refuse — so the wrap is the last unbuilt
half of runtime loop playback.

## Decision boundary

**Where the note identity of the `k`-th pass of a loop comes from**, given that the compiled list a wrap replays was
stamped **once**, off the audio thread.

**What the rounds showed is that this question cannot be separated from a second one**, and that is this record's
result rather than an evasion: *at what granularity does a wrap take effect?* Constraint 5 below is why. The two are
one decision.

**Non-goals.** Same-sample ordering of two activations in one quantum is ADR-0023's. The hardware clock is
ADR-0022's. This record does not reopen ADR-0046 clause 4's admission, which is built and correct for what it
proves.

## Evidence

Every constraint below was verified against the code by an independent design consultation, in two rounds, and each
is stated with the mechanism that makes it true rather than as a conclusion. Nine were established in the first
round and six in the second; the numbering here is by subject rather than by round.

### The contract

1. **A wrap is a transport activation.** ADR-0050 treats a wrap as one of the four re-anchoring moments;
   `SOUND-INV-018` carries that rule, and ADR-0032 requires a loop wrap to establish a new anchor. ADR-0055 prevents
   a loop interval from becoming active while the wrap is absent; it does not change the eventual wrap boundary. A
   design in which a wrap is *not* an activation must supersede those accepted records rather than sit beside them.
2. **Adoption is infallible and advances the freshness sequence.** ADR-0050 clause 6 changes the in-force sequence
   at adoption alone, and an accepted candidate is valid only while the value it supersedes is still in force.
   Adoption itself has no branch that can fail, by clause 3, so it cannot re-check anything at the boundary.
3. **The off-thread half learns the new state only at collection.** `StreamControl` promotes the sequence, anchor,
   minter copy and outstanding set when it collects a retired value. A wrap that returns no retirement leaves the
   control describing pre-wrap state, and every candidate it builds afterwards is built against that.

### What the audio thread holds

4. **The scheduler owns a suffix, not a pass.** `plan_activation` derives the placed list from the requested
   position forward; events in `[loop_start, requested_position)` are history and are not in the list the scheduler
   receives. `TimedEvent` carries an absolute `SampleTime` and no `PlanPosition`. So no cursor operation on the
   scheduler's own storage can reconstruct the pass a wrap must replay, and no modulo of an engine time recovers a
   plan position the value never carried.
5. **A quantum-granular wrap truncates or overruns the pass whenever the loop's length is not a whole number of
   quanta.** This is the constraint that couples the two questions, and it is a worked case rather than a
   worry. With `Q` = 64, a loop of 65 frames and a first wrap whose requested time is congruent to 1 modulo 64, the
   first two effective wraps snap to frames 64 and 128 — **64 frames apart for a 65-frame loop**. An event at
   `loop_end - 1` then falls at or after the next boundary and is retired unpublished; at other phases the pass
   instead lasts 128 frames and the position-aware kernels run past `loop_end`. Either way the pass that plays is
   not the pass `admit_loop` and `admit_loop_polyphony` judged, which is what the render contract says the subject
   is.

   **The restriction that would avoid it is not musically available.** Requiring a loop length that is a whole
   number of quanta bites nearly every tempo: at 48 kHz, of the 281 integer tempi from 20 to 300 BPM, **30** give a
   one-bar 4/4 loop that is a whole number of quanta. 120 BPM is one of them; 128, 140 and 174 BPM are not.
6. **Position-aware kernels read one plan position per quantum and treat it as the start of a linear span.** A wrap
   inside a quantum therefore puts events and kernels on different timelines for that quantum — the impulse kernel
   emits nothing for a position that should have occurred after the wrap. This is the same failure class the
   repository already recorded once.
7. **The renderer resolves a whole call's note events, and mutates the live-note registry, before rendering its
   first quantum.** So two boundaries inside one call cannot each scope a mass release correctly: the later pass's
   note-ons may already occupy the registry when the earlier wrap's release runs, and a scoped release then clears
   the wrong occurrence. One boundary per renderer call is a structural property, not a tuning choice.
8. **The prepared control storage holds one boundary release, at offset zero.** `adoption_gates` is sized to one
   identity partition and the timed-control scratch adds that partition once, which is sound today precisely
   because activations are quantum-aligned.

### What a wrap costs

9. **A wrap costs strictly more than an ordinary activation, and the excess is superlinear.** Because of constraint
   7, each wrap needs its own renderer call; each call opens a publication pass that clears the arbiter's whole
   maximum-sized ledger rather than its active rows; and each wrap's mass release scans the producer's full
   identity span. For a loop of exactly `Q` and a maximum callback of `N` quanta, that is `O(N^2)` ledger clearing
   and `O(N * identity_span)` release work in one callback, against at most one split and one release for an
   ordinary activation.
10. **Session-share admission covers one catch-up batch and one mass release per activation.** ADR-0046 clause 4's
    periodic extension repeats **compiled** positions only; it says nothing about repeated session-share operations,
    and `admit_loop_polyphony` is explicitly a separate `SOUND-INV-017` proof rather than part of clause 4. Several
    wraps in one quantum would multiply a session contribution nothing admitted.

### What identity is

11. **A generation is unique per index within one table, not globally.** Every new table starts every index at
    generation zero; reuse across tables is safe because the table identity differs, which ADR-0047 states.
12. **The live-note registry overwrites unconditionally.** `LiveNotes::admit` replaces whatever a slot holds,
    whatever its generation, and the release path counts an orphan in the renderer rather than in the registry. So
    replaying an identity is not caught anywhere: it silently reassigns which note a later release ends.
13. **A pass is not idempotent in the allocator when it leaves a note open at `loop_end` and the boundary release
    reaches the allocator.** Stamping leaves that index live and returns the occurrence as outstanding, and
    re-adopting an already-stamped list mints nothing; an allocator-reaching release then frees the index and
    advances its generation, so occupancy after two adoptions is not occupancy after one. Reinstating the pre-pass
    snapshot instead would wind back a generation the release advanced, against `SOUND-INV-017`.

    **The qualifier matters, and an earlier draft dropped it.** Where the release stays in the registry — which is
    where ADR-0050 clause 5 actually puts it, the allocator half having run at build and being promoted at
    collection — replaying a stamped list leaves the allocator and the outstanding set structurally unchanged. That
    case is not allocator non-idempotence at all; it is **constraint 12**, the same occurrence being installed
    twice, which is the aliasing this whole record is about. An independent review found the two conflated.
14. **Issuing a distinct occurrence is minting, whatever it is called.** ADR-0047 fixes an occurrence as exactly
    table, index and generation, and the table's mint is what creates one. A fourth component written on the audio
    thread would still be issuing a new occurrence, so it needs an explicit amendment of ADR-0047 rather than an
    argument that it is not minting.
15. **ADR-0047 clause 7 requires a mass release to advance every affected index's generation**, and ADR-0050 amends
    that timing for the activation case only. So the two designs face different rules rather than no rule: a wrap
    that **is** an activation takes ADR-0050's amended timing, and one that is not takes clause 7's general rule
    unamended — which advances the generation *at application*, on the audio thread, where the allocator is not.
    An earlier draft said such a wrap fell under neither, which is backwards and was caught by an independent
    review.

## Options: four closed, and one left open with its cost stated

### A. Replay the stamped identities unchanged

The cheapest, and it is refused by constraint 12: a stale release from the previous pass resolves as live against the
new pass's note and ends it, so a stale-event defect becomes a wrong-note action with nothing counting it. It also
contradicts what an occurrence *is* — two simultaneously distinguishable notes would share one identity, and
`SOUND-INV-017` makes the occurrence the sole authority for which note an event resolves to.

### B. A generation displacement returned through the retirement

Refuted in the first consultation, before any code: a candidate is stamped **before** the wraps it would have to be
ahead of, so the displacement it would carry is not knowable when it is needed.

### C. An off-thread pass built per wrap — **open**, and the only one that is

Building and offering can be refused — a superseded sequence, an occupied exchange, a control that has not
collected. **This option is not closed by ADR-0046 clause 4, and saying it was is the misquotation named above.**
None of those refusals is a compiled-capacity failure, so the clause does not reach them; an independent review
found both the misquotation and, in a later round, this section still calling the option closed on the strength of
it.

So it is left open, with its cost stated rather than borrowed: the loop's continuation would depend on the
off-thread half completing a build before every wrap, so a stall there is heard as a missed pass. Whether that is
acceptable is the deciding record's judgement, and it belongs with question 3, since a per-wrap build also changes
what a wrap costs. The second consultation is what sharpened this: a refusal reaches clause 4 only when it lies on
the path of an **already-accepted** wrap, which is what made option E worth putting.

### D. A wrap that is not an activation

Cursor rewind plus a modulo plan position. Refused by constraints 1, 4, 6 and 7 together: it contradicts three
accepted records, the audio thread does not hold the list it would rewind, and it desynchronises events from
position-aware kernels inside the wrap quantum.

### E. A standing pass activation, accepted once and adopted in place

The strongest candidate, and refused on three independent grounds plus a fourth that is narrower than an earlier
draft claimed, none of which needs clause 4. Constraint 2:
adoption must move the freshness sequence, and a standing value either leaves the wrap invisible to freshness or
supersedes an already-accepted candidate that adoption cannot then refuse. Constraint 3: with no retirement
returned, the control goes stale after the first wrap and builds later candidates against pre-wrap state.
Constraint 13: the value is not idempotent in the allocator where the boundary release reaches it — one ground
rather than two, since the registry-only case is constraint 12's subject instead. And the activation value is not
re-adoptable **as a value** at all — the first adoption fills its retirement fields, swaps its event and catch-up vectors with the
scheduler's, and sets its effective time, so adopting it again would swap the retired schedule and the spent
catch-up back into force.

## What must be decided together

The record that closes this topic has to answer these as one contract, and the first is what the two rounds added:

1. **At what granularity a wrap takes effect.** Constraint 5 makes quantum-granular wrapping wrong for nearly every
   musical loop length, not merely imprecise, so this is where the master plan's sample-exact loop requirement and
   ADR-0050 clause 1's deferral of it actually meet. A sample-exact wrap reopens clause 1, needs the sub-quantum
   representation clause 1 declined to fix, and has to answer constraints 6, 7 and 8, each of which is a structural
   property of the current renderer rather than a parameter.
2. **Where the `k`-th pass's occurrences come from**, under constraints 11 to 15 — including whether ADR-0047's
   three-component occurrence is amended, and by which owner.
3. **What a wrap costs and what admits it**, under constraints 9 and 10.
4. **The first wrap origin**, including the two failure modes a naive derivation misses: a representable requested
   time need not have a representable quantum boundary, and an entering locate whose position is exactly `loop_end`
   would put the first wrap at that locate's own boundary — two activations in one quantum, which is ADR-0023's.
5. **Precedence between a wrap and a pending activation**, compared against the pending activation's **effective**
   point rather than its requested time; comparing against the requested time changes playback before the
   activation takes effect.
6. **The exhaustion outcome**, if the selected identity mechanism has one. A loop may be one frame long today —
   `LoopInterval::new` requires only a positive interval — so a per-pass counter of `u32` width is exhausted in
   about 24.9 hours at 48 kHz, not the two months a `Q`-length floor would give.

## Consequences and risks

- **Accepted cost.** Loop playback stays unavailable. ADR-0055 refuses a loop-bearing activation at the runtime
  offer, names the interval and leaves active state unchanged. No event can silently play past an allegedly active
  loop end; the first consumer that needs loop playback must resolve this record.
- **Safety/correctness control.** The admission halves already built are unaffected and keep refusing: a loop whose
  periodic extension overruns the compiled share, and one whose repeating pass exceeds the producer's admitted
  range, are both refused at the build today. Nothing in this record relaxes either.
- **Revisit condition.** This record becomes decidable when ADR-0050 clause 1's activation granularity is reopened,
  which the master plan's sample-exact loop and seek requirement already schedules. It is also decidable — at a
  stated musical cost — if loop lengths are restricted to whole numbers of quanta, and constraint 5 quantifies that
  cost so the trade can be made rather than assumed.

## Specification update

None. A `Proposed` record changes no current specification. ADR-0055 independently updates the current render
contract with fail-closed behavior while this record remains undecided.

## Review

Reviewer: Codex, four times and in two roles. Two **design consultations** before drafting — the protocol's "review
the design before building", with the frames authored here rather than by the reader — of which the first refuted a
non-activation wrap on nine findings and the second a standing pass activation on six. Then an **independent
semantic review** of the drafted record and one focused reread of its repairs, which together produced seven more:
a misquotation of ADR-0046 clause 4's capacity qualifier, two constraints that misstated allocator behaviour, and a
section still calling option C closed on the strength of the misquotation after it had been corrected. Every one is
repaired above, and two of them **narrowed a conclusion**: option C is now open rather than closed, and constraint
13 covers one branch rather than two.

This record makes no decision, so the stopping rule below applies to its constraints and its options, not to a
conclusion.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
