# Phase 0A: Baseline, Limits, and Render-Core Contracts

| Field         | Value                                                                            |
|---------------|----------------------------------------------------------------------------------|
| Status        | Active                                                                           |
| Phase         | 00A                                                                              |
| Last reviewed | 2026-08-13                                                                       |
| Master plan   | [Phase 0A](../master-plan.md#phase-0a-baseline-limits-and-render-core-contracts) |
| Exit review   | Not created                                                                      |

## Objective

Establish measurable V1 baselines, a headless comparison harness, the real-time limit audit, and the few contracts and
decisions Sound Core V2 needs before Phase 1 begins.

The master plan defines scope and exit gates. This tracker records execution state only: the tasks below decompose the
Phase 0A `Work` list and add no scope of their own.

## Entry conditions

- The master plan is available and reviewed for current relevance.
- V1 remains the production engine.
- The documentation workflow and identifier rules are understood.
- A task is assigned and marked `Active` before implementation begins.

## Required decisions

| ADR      | Required at Phase 0A exit                             | Status      | Later acceptance gate |
|----------|-------------------------------------------------------|-------------|-----------------------|
| ADR-0001 | `Accepted`                                            | `Accepted`  | —                     |
| ADR-0021 | `Accepted`                                            | `Accepted`  | —                     |
| ADR-0037 | `Accepted`                                            | `Accepted`  | Phase 2 re-measurement |
| ADR-0032 | `Accepted`                                            | `Accepted`  | Phase 3 verification  |
| ADR-0022 | `Accepted`, or `Deferred` with owner and evidence gap | `Deferred`  | Phase 3 entry gate    |
| ADR-0028 | `Accepted`, or `Deferred` with owner and evidence gap | `Deferred`  | Phase 4 entry gate    |

ADR-0037 is not in the master plan's decision list. It carries the frame count split out of ADR-0001 and is required
`Accepted` here so that the split cannot be used to pass the gate with the quantum's value still open. See *Deviations*.

Its `Later acceptance gate` entry is not a deferral: ADR-0037 is `Accepted` and satisfies this phase. The entry records
that its measurement selected the record's rule 1, so the value is provisional and re-measuring it against real V2 nodes
is now a Phase 2 exit-gate item.

ADR-0032 is `Accepted` after three passes: an author pass, an independent pass that withdrew the first acceptance, and
a bounded closure pass over the corrections. Its `Later acceptance gate` entry is not a deferral. The register's basis
for the topic is `Range analysis and timing tests`; the range analysis is in the record and the timing tests cannot
exist before there is a scheduler, so those tests verify the contract in Phase 3 rather than being an outstanding
acceptance condition.

## Tasks

| ID        | Deliverable                                                 | Status      | Dependencies         | Primary record                                          |
|-----------|-------------------------------------------------------------|-------------|----------------------|---------------------------------------------------------|
| P00A-T001 | Define the reference V1 corpus and preserve/change manifest | Active      | None                 | [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md), [EVD-0004](../evidence/phase-00a/EVD-0004-corpus-0005-claim-counterfactuals.md) |
| P00A-T002 | Define the comparison result model and headless command     | Complete    | P00A-T001            | [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md) |
| P00A-T003 | Capture V1 CPU, memory, timing, and determinism baselines   | Complete    | P00A-T001            | [EVD-0003](../evidence/phase-00a/EVD-0003-cpu-memory-timing-baseline.md), with EVD-0001 and EVD-0002 |
| P00A-T004 | Complete the fixed-limit and overflow audit                 | Active      | None                 | [Resource inventory](../inventories/resource-limits.md) |
| P00A-T005 | Define the initial HostProfile and RenderLimits contract    | Active      | P00A-T004            | [Host profile specification](../specs/spec-host-profile-and-render-limits.md) |
| P00A-T006 | Satisfy every entry in the required-decisions table         | Complete    | P00A-T003/P00A-T004  | [Decision register](../ADR.md)                          |
| P00A-T007 | Prepare the formal Phase 0A exit review                     | Not started | All applicable tasks | Future `REV-P00A`                                       |

Phase 0B runs in parallel and has its own tracker. Do not move its inventory work into this phase to make the gate look
complete.

P00A-T006 must accept ADR-0001, ADR-0037, ADR-0021, and ADR-0032, and it has. ADR-0022 may be deferred only to the
Phase 3 entry gate and ADR-0028 only to the Phase 4 entry gate; either deferral records an owner and the missing
evidence. Both are now written on those terms, so no other deferral is in play.

## Active tasks

**P00A-T005 — Define the initial `HostProfile` and `RenderLimits` contract.**

- **Scope.** One normative contract covering the master plan's field list — maximum host block, layouts, voices, nodes,
  event fan-out, channels, buses, sends, telemetry taps, recording buffers, prepared memory, script work — plus
  ADR-0032 clause 21's forward event horizon, with a value and a stated basis for every field.
- **State.** [The specification](../specs/spec-host-profile-and-render-limits.md) exists at `Draft`, invariant prefix
  `HOST`, 21 invariants and a conformance-test row for each. Every field the plan lists has a value; each of the
  ledger's 28 settled `HostProfile`-owned entries has a named successor field, with `LIMIT-0014` pending a split; seven fields have no V1 antecedent and are
  listed as such.
- **Review history — eleven passes, six by an author of the document and five external, every one of which found
  something.** The bullets below record what each found; the current conclusion is at the end.
- **The first independent review pass raised five, two of them High**, and the
  fifth held three separate defects. The two High ones were substantive: a `maximum_block_size >= Q` clause that
  **refused hosts the render model is built for** — ADR-0001 clause 6 primes the output carry precisely so a callback
  of `N < Q` can be served — and a runtime contract that permitted loss only at live bounded queues while three fields
  visibly did something else, including an over-full quantum whose behaviour was undefined altogether. The
  specification now names five distinct runtime behaviours (admission refuses, a queue drops, a quantum defers, a lossy
  budget evicts, a session limit stops), with three new invariants carrying them.
- **Applying the review found a sixth defect, in the corrections.** `max_held_notes` had been set to the per-instrument
  voice ceiling on the reasoning that a held note cannot outnumber a voice. It can — sustain pedal, stealing allocator,
  MPE or sequencer source — and the allocator tracks a held note precisely so it can re-sound one.
- **The bounded closure pass has run, and found four more, one substantive.** The substantive one is in the previous
  pass's own correction: HOST-INV-021 had reused ADR-0001 clause 16's late counter and position rule for a deferred
  event, and **neither fits** — a deferred event's producer was on time and the *engine* was full, so counting it as
  late would publish a capacity shortfall as an external timing fault, and clause 16's "first not-yet-rendered quantum
  boundary" is circular for a quantum that has itself not rendered. Deferral now has its own counter and its own
  position rule, with ADR-0032 clause 22 cited as the precedent that separated the pre-epoch clamp from the late
  counter on identical grounds. The other three: `forward_event_horizon`'s flat one-second default **could fail its own
  validation** on a device reporting a very large block, so it now takes a derived floor; `max_concurrent_retiring_voices`
  = 64 **recreated the defect the previous pass had just fixed** — a runtime-enforced field with no defined behaviour
  on reaching it — and is now derived from `max_active_voices` so it cannot bind; and HOST-INV-009's claim that every
  field falls under one of four runtime behaviours was false twice, omitting admission refusal and having no place for
  a field that is a *size* rather than a bound.
- **The confirmation read of those three changes found two more**, neither of them in the three changes themselves: the
  deferral and late counters were implied to partition the cases and do not — one event can raise both, so a consumer
  summing them overcounts — and `max_scheduled_events_in_flight` had no defined behaviour on reaching it, the third
  runtime-enforced field in the specification with none.
- **The independent read the confirmation pass asked for has run, and found seven, two of them High.** The first is in
  the confirmation read's own correction: `max_scheduled_events_in_flight`'s new failure behaviour charged a delayed
  scheduler release to ADR-0001 clause 16's late counter, which is exactly the merge HOST-INV-021 forbids two sections
  earlier — the producer is the engine and the bound is a profile capacity, so it raises the capacity-deferral counter
  instead. The second: **HOST-INV-021 said *that* the excess defers but not *which* events are the excess**, with no
  rule among ingress events and none at all for compiled events overrunning a quantum alone, which
  `max_note_expansion_per_tick`'s data-dependent case makes reachable; admission is now compiled-before-ingress then
  ascending `SampleTime`, with priority deliberately not reordering against the timestamp. The other five: deferral to
  the quantum *boundary* moved a sample-positioned event off its declared sample — which ADR-0001 clause 14 preserves,
  and which the text excused as the delay clause 14 already charges to the *control* response — so deferral is now
  `+Q` with the offset preserved; HOST-INV-005's admission rule did not admit six of the specification's own fields;
  HOST-INV-018 was unsatisfiable for the two enum-typed fields, the HOST-INV-003 shape again; HOST-INV-011 was
  contradicted by `retirement_crossfade` and `telemetry_ring_frames`, both durations stated flat in frames; and
  `max_held_notes`'s coupling to `max_active_voices` exists only in prose, since the types cannot convert.
- **The targeted pass over those two rules found six, three High — and all six fell inside its targets**, the first
  time a pass here has been contained by its own scope. The three High ones: **`+Q` never said whether it rewrites the
  event's `SampleTime`**, which decides whether per-event displacement is reportable at all, whether ADR-0032 clause
  7's take resolution silently quantizes a played performance forward under overload, and whether clause 23's stated
  harm arrives through the back door — the stamp is now immutable and the render position derived. **The reason for
  subordinating queue priority proved too much**: rule 1 reorders against the position exactly as priority ordering
  would, so the clause-23 argument killed both or neither; rule 1's basis is restated as provenance exactness, and
  priority stays subordinate because it is a *delivery* class that says nothing about timestamp accuracy. And
  **preserving the offset created a second starvation channel**, ingress against ingress, since the order has no age
  term. The other three: the horizon could re-reject a repeatedly deferred event, turning "deferred, not dropped" into
  a drop; the admission order lacked the position-monotonicity precondition that makes it a five-way merge rather than
  an audio-thread sort; and rule 3's tie-break left same-position events from one ring undecided.
- **An external pass — the first by a reader who authored none of the document — then found five, three P1, and the
  worst had stood since the first independent pass.** The specification had been reasoning from `LIMIT-0013`'s
  prioritized rings as *renderer ingress*; they carry `EngineEvent` **out** of the engine toward the GUI, and V1 does
  not wire the channel to anything. So the "3 072 events against a 256-event scratch" motivation for HOST-INV-021 was
  never true, and **V1 has no timestamped renderer-ingress queue to carry over at all** — the profile has no field for
  the capacity deferral operates against. Second: `+Q` deferral de-orders a FIFO, so the five-way merge the sixth pass
  made normative could not work even with its own monotone-enqueue precondition; the merge is withdrawn to a
  constraint, and the deferred store is a further missing capacity. Third: suppressing ADR-0001 clause 16's late
  counter for a delayed release **overrode an accepted decision** — clause 16 triggers on a condition, not a cause, so
  both counters rise. Plus HOST-INV-005's grounds were neither disjoint nor exhaustive.
- **A second external pass over those corrections found five more, one P1.** The immutable stamp and ADR-0001 clause 16
  **cannot both be implemented as written**: once the quantum a deferred event could not enter has rendered, the
  event's preserved timestamp does fall in an already-rendered quantum — clause 16's condition — while HOST-INV-021
  forbids the late counter there. The interim rule is that the condition is asked once, when an event first becomes
  due, and because that narrows an accepted decision it needs an **ADR-0001 clarification or successor before Phase
  3**. The other four: the producer-based gloss had survived in two places after the invariant was fixed,
  HOST-INV-005's conformance row tested different grounds than the invariant defines, the withdrawn clause-23
  rationale survived in the unresolved-questions table, and the queue-direction correction itself carried no
  `file:line` citation — a violation of the rule this repository added for exactly that failure mode.
- **A third external pass found six more, two P1**: the **bounded deferred store had no exhaustion policy**, so
  HOST-INV-021's "no event is lost" could not coexist with the prohibition on allocating; and `LIMIT-0013`'s rings
  **kept the `Live bounded queue` class** after being identified as egress, which ADR-0021 reserves for queues fed by
  external unbounded input. Four consistency defects came with them, introduced by the corrections themselves.
- **A fourth external pass found four more, two P1**, both in the third's corrections: the deferred-store size did not
  bound what it claimed, and the admission tie-break named ingress rings this document had just shown do not exist.
- **A fifth external pass found the other half of the same premise, and it lands in P00A-T004.**
  `max_events_per_quantum` = 256 is **not** a V1 carry-over: `EVENT_BUFFER_SIZE` is an egress ring size
  (`synth_engine.rs:81`, used at `:811` and `:815`), and V1's actual per-block sequencer buffer is an uncapped
  `Vec::with_capacity(128)` at `:895`. So V1 has no per-quantum event cap, and **`LIMIT-0014`'s ledger description is
  wrong** — recorded from a constant's name by two discovery passes, with the class pass not reaching its use sites.
  P00A-T004 is marked `Complete`; this is a finding against it.
- **Eleven passes, six by an author of the document and five external. Every one found something.** Three of the five
  external passes found a defect in the immediately preceding correction; the other two found long-standing misreadings
  no author pass had questioned — the egress-ring premise and `LIMIT-0014`. Four of the five landed on
  **HOST-INV-021**: deferral cannot be specified without knowing what feeds the renderer and how much of it there can
  be, and **every V1 number the invariant was built on has now been falsified**. Two of those were ledger entries read
  from constant names rather than use sites. **The gap is in the contract, not in the reviewing**, and the pass-4
  use-site audit has since run for the `HostProfile`-owned entries — which is what the exit review has to act on. The companion to this phase's "a correction is new material" rule is now:
  **a number that supports the conclusion you already hold does not get audited.** An author re-reads the reasoning;
  an external reader checks the claims, and only the second kind has caught this document's factual and cross-record
  errors.
- **Why this is still `Active`, and the sixth pass's recommendation to close is withdrawn.** That recommendation rested
  on the pass's findings being contained by its targets and on its additions being preconditions rather than mechanism.
  The external pass falsified both — the containment was an artefact of the targeting, and the precondition was
  insufficient for a mechanism built on queues running the wrong way. Closing this task now needs **four substantive
  things**: an **ADR-0001 clarification or successor** covering both when clause 16's late condition is evaluated and whether a quantum may defer at all under clauses 12 and 14, without which
  Phase 3 cannot implement deferral and lateness together; an **ADR-0021 one** on the `Live bounded queue` class given
  to `LIMIT-0013`'s engine-egress rings; **`LIMIT-0014`'s GUI/OSC split**, without which `event_egress_capacity` leaves
  HOST-INV-005 unsatisfied; and **V2's renderer-ingress streams plus a separate bound or exhaustion policy for the
  deferred store**, without which HOST-INV-021's "no event is lost" rests on a store with no capacity. **All four may move to Phase 3
  with a narrowed P00A-T005**, since Phase 1 compiles rather than renders live; that scoping call is the exit review's.
  Closing on the current text is not available.
- **What the evidence actually supplied, and what it did not.** ADR-0021 assigned P00A-T005 "measured
  `HostProfile`/render defaults", and EVD-0003 measured *cost*, not capacity. **Exactly one field is derived from
  measurement**: `predicted_quantum_cost_ratio` (0.15, where the stated target and the measured 2.7x–6.8x
  max-to-median block spread together fix the number). Two more are *chosen and anchored on* EVD-0003 —
  `max_active_voices` (512, checked against the 1.173 ms/s per-voice slope: about 60% of one measured core at 44.1 kHz
  and not real time at all at 192 kHz) and `prepared_immutable_bytes` (64 MiB against the 5.33 MiB the render phase
  adds). The draft called all three *derived*; review was right that a value picked and then checked is a weaker claim
  than one a rule produces, and the basis vocabulary now separates the two. **Every other value is queried, carried
  over from V1, or chosen**, and the specification labels each one rather than implying a measurement.
- **The one field the evidence made necessary.** EVD-0003 found that a block costs the same in absolute time at every
  sample rate while its real-time budget shrinks with the rate, and warned that an admission policy reasoning in frames
  would get that backwards. The profile therefore carries a cost budget expressed as a ratio of *times*, not a count —
  the only field in it that is not a capacity, and the only one the evidence drives rather than merely informs. It is
  advisory: the cost model behind it is a V1 prediction on one machine with no callback deadline, and HOST-INV-015
  forbids it from refusing anything until Phase 3's simulated host exists.
- **Two structural decisions the specification makes that ADR-0021 left open.** First, `HostProfile` is one input with
  two halves — `HostCapabilities`, which is queried and which nothing may raise, and `RenderLimits`, which the operator
  chooses. This is what the master plan's two names mean; it is a refinement of ADR-0021 part 4's "capability fields are
  established from queried capability", not a replacement, and the review must confirm that reading. Second, a
  `CapabilitySource` tag records whether the capability half was queried from a device, declared by an offline job, or
  declared by a harness, which is what keeps the queried-capability rule total for paths that have no device.
- **Three findings from writing it.** `LIMIT-0031`'s ledger owner is `N/A — removed`, but its disposition creates a
  profile field (`max_held_notes`), so an entry outside the 28 lands here. **Two of the master plan's terms are
  answered rather than carried**, and both are accounted for in the `ResourceReport` instead of being budgeted:
  "parameter and control slots" are prepared memory or a buffer, so a separate count would only have to be kept in step
  with the node budget by hand; and the script-work aggregate is a reported quantity with no threshold until Phase 7
  measures what an instruction costs. The aggregate was first written as a `RenderLimits` field with the value *unset*,
  and review correctly rejected that — a limit with no value is not a limit, and it contradicted the specification's
  own promise of a value for every field.

**P00A-T001 — Define the reference V1 corpus and preserve/change manifest.**

- **Scope.** The master plan's [Phase 0A work list](../master-plan.md#work) names eleven categories of representative V1
  render. The task is complete when all eleven are captured, not when the format that holds them exists.
- **State.** The manifest, the model, the generator, and the validation are done and in use. **Five categories are
  exercised**: `subtractive-voice`, `polyphonic-voice-stealing`, `mod-matrix-patch`, `sends-returns-master`, and
  `instrument-inserts` (CORPUS-0005). Six are recorded as gaps with reasons.
- **Why this is `Active` and not `Complete`.** A recorded gap is an honest statement about coverage, but it is not the
  baseline the plan asks for. Marking the task complete with seven of eleven missing would move the shortfall out of the
  task list and into a document nobody re-reads, and the Phase 0A exit review would then have to rediscover it. The
  register is authoritative for what the task is; this tracker does not get to shrink it.
- **Remaining, in the order the blockers clear.**

  | Category                     | Blocked on                                                                        |
  |------------------------------|-----------------------------------------------------------------------------------|
  | `stereo-or-spatial-voice`    | Choosing the source. The Spatial Panner's positions are not modulatable in V1, so a case built on it pins behaviour already scheduled to change |
  | `tempo-map-arrangement`      | Whether a tempo ramp's event positions fall under the sample-timing correction, which the case must cite rather than decide |
  | `yams-control-patch`         | A script small enough that a render difference names one language feature          |
  | `yams-audio-script-patch`    | Same, and see the note below                                                       |
  | `sampler-patch`              | The Phase 0B bundle round-trip fixtures; a sample makes the input a bundle and pulls asset identity (ADR-0027) into a Phase 0A baseline |
  | `shared-patch-or-instrument` | ADR-0014. A module's script PRNG seed derives from its instance number (`IDN-0029`), so a shared instrument's audio depends on how the two references are numbered |

- **What CORPUS-0005 pins, and why it is not a smaller CORPUS-0004.** An insert chain sits *inside* the instrument,
  between voice summing and the mixer; CORPUS-0004's effects are on a return bus and on the master and only ever see a
  signal that has already left the instrument. Four claims, each measured against a counterfactual **and a null
  control** in [EVD-0004](../evidence/phase-00a/EVD-0004-corpus-0005-claim-counterfactuals.md): the chain runs on the
  **summed** voices (−3.07 dB relative with the clipper isolated), its state is **shared across voices** (−35.21 dB
  with the delay isolated — two notes can only interact inside a delay line they share), that state **outlives the
  notes** (a tail 2.3 s past the last note-off against a silent control), and the **authored order** is load-bearing
  (5.97 dB). The null control — the same construction with an empty chain — sits at −147 dB relative, which is
  floating-point rounding, so each figure is attributable.
- **Getting there took three attempts, and the two failures are the useful part.** The first probe guessed a frequency
  where the clipper's intermodulation would land and found nothing; a probe aimed at a predicted frequency is a guess
  about the DSP. The second ran the summed-versus-whole construction **without its null control**, produced a
  plausible +0.67 dB, and — when review found the delay's feedback soft-clip and prompted the isolation runs — the
  control came out at −1.41 dB, the same size as the effect. That record concluded V1's renders are not additive
  across polyphony, wrote an intentional-correction claim against a sequencer timing defect, and withdrew two claims.
- **There was no timing defect.** A second review found the cause: `Oscillator`'s `uni_phase` defaults to 1.0 and
  `set_voice_index` seeds the generator behind it from the voice index, so a note starts at a different phase
  depending on which voice took it — voice 0 in a solo render, voice 1 for a chord's second note. That was the whole
  46.55° phase difference, and it explains why the implied offset was never a constant number of samples: it was not a
  delay. `CORPUS-0005-C2` is withdrawn; **a migration contract written against an artifact is worse than none**. With
  `uni_phase` at 0 the control drops to −147 dB and both withdrawn claims come back stronger than they were written.
- **The finding that survives is about the corpus, not the engine, and P00A-T001 has now acted on it.** `uni_phase`
  defaults to full note-on phase randomization, drawn from a generator seeded by the voice index **and advanced on
  every note-on** — so every fixture pinned a phase sequence that depends on which voice took which note and on how
  many note-ons preceded it. A V2 with an equivalent but differently-indexed allocator would change that without
  changing any behaviour a case claims, and the corpus would report it as a regression. **It is now off in all five
  fixtures**, which changed four committed digests.
- **What that cost, measured rather than waved past.** [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md)
  gains a revision section: its four input digests are superseded, and the determinism question was **re-asked rather
  than assumed** — all five cases render bit-identically across two `--release` processes, with the full replacement
  output digests recorded, because a truncated digest from a `dev` build is a note rather than a baseline and this
  record's acceptance criteria fix `--release`.
- **The cost re-measurement is the part worth reading, because it failed twice before it said anything.** A first
  three-round check reported +2.2% and argued the shift was real because it was *uniform*. Review objected that a
  minimum over fewer draws is biased upward and that the bias would itself be uniform — so uniformity was never
  evidence against noise, and that argument is withdrawn. Re-run under
  [EVD-0003](../evidence/phase-00a/EVD-0003-cpu-memory-timing-baseline.md)'s own protocol at **50 draws per case**, the
  pooled minimum is **−4.0%**, the opposite sign. Resampling 15-draw minima out of those 50 puts the small-sample bias
  at **+0.14%**, which does not account for a 6.5% gap between two runs of the same binary on the same fixtures hours
  apart. **The objection to the reasoning holds; the mechanism it proposed does not.** What dominates is
  session-to-session variation on an unquiesced machine, which EVD-0003 already lists as a limitation — so **no cost
  claim about the fixture change is supportable at this resolution**, in either direction, and the 0.5% agreement
  EVD-0002 and EVD-0003 reached is a weaker guarantee than it reads as.
- **The case is the one that would have justified a V1 fallback, so it records the correction instead.** V1 appends any
  effect missing from `effect_chain_order` with a warning, which means the rendered order is partly V1's choice. Under
  ADR-0021 part 2 admission may refuse a plan but may never silently change authored topology to make it fit, so
  CORPUS-0005-C1 records that appending becomes a refusal. The fixture authors a complete order, so the correction
  changes nothing about this case's audio — it is recorded because this is the case a future reader would otherwise
  point at to defend the fallback.
- **Two of the three new tests are general rather than case-specific**, because the failure they catch is silent:
  `effect_chain_order` matches module ids by string, so a typo drops an entry and V1's append path supplies an order
  nobody authored — and every other fixture test still passes. One test rejects an entry that names no module or names
  a non-effect; the other rejects an effect that no entry names.
- **Correction to an earlier note.** The `yams-audio-script-patch` gap said the case "should be authored together with
  the ADR-0037 measurement". That reads as a dependency and is not one: ADR-0037's proxy is defined over whatever the
  corpus contains, and waiting for an AudioScript case would block a measurement that is otherwise ready. The two are
  independent, and the gap entry now says so.

## Completed tasks

Kept here rather than deleted: the exit review has to be able to see how a completed gate item was satisfied.

**P00A-T004 — Complete the fixed-limit and overflow audit. `Active` again; kept in this section because most of its
work stands.** Passes 1-3 are unchanged and their conclusions hold. What reopened it is pass 4, the **use-site audit**:
the first pass to read what a constant is *used for* rather than where it is defined. Over the 30 entries that were `HostProfile`-owned when it started it found `LIMIT-0015` misowned (four deferred-drop channels, not a return-bus scratch — it moves to
`N/A — removed`; with the audit's other results the split is 28 `HostProfile` / 1 undecided / 46 elsewhere across 75 entries), `LIMIT-0014` misdescribed (V1 has no per-quantum event limit), two
citations pointing at unrelated lines, a `<=` floor relation recorded as a 1:1 coupling, and **three further
silent-truncation sites**, taking the register from six to ten. Four of those nine were found outside any search.
Pass 4 also made two false findings of its own — an absence asserted from a name search, and a truncation asserted from
one line without reading the `push` that feeds it — both caught by external review, and both the same failure the pass
was convened to correct. **The remaining 44 entries have had no use-site read of their own** — passes 1-3 read some enforcement sites, but none asked what a constant is used *for*, which is the check pass 4 applied, and three rows left `Classified` — `LIMIT-0013` (its ADR-0021 disposition rests on two disproved readings), `LIMIT-0014` (one constant, two rings, two owners), and `LIMIT-0015` (overflow only partly read). 72 of 75 are classified. Both facts keep this task open.

- **Scope.** Populate the [resource inventory](../inventories/resource-limits.md) with every fixed cap, truncation
  point, bounded queue, buffer capacity, and script budget in the workspace, each with its enforcement site, its
  overflow behavior, a proposed V2 rule, and a diagnostic.
- **State.** **Four passes, the fourth partial** — 31 of 75 entries use-site read. Two discovery passes with opposite blind spots at `dd69b657`, and a third
  classification pass at `b435887c` that applied accepted ADR-0021 to the 74 entries that existed then. Every entry
  carries a terminal failure class, a rule, and a diagnostic, and **74 of 75 have a settled owner** — `LIMIT-0014` is
  undecided pending a GUI/OSC split, which is one reason this task reopened. **No entry is `Unknown`** — the 19
  that were are resolved.
- **What the owner axis showed.** 28 entries are `HostProfile`, one is undecided (`LIMIT-0014`, pending a split), and 46 are not: 12 node contracts, 9 removed outright,
  7 protocol, 7 application settings, 6 domain/format, 5 job policy. (`LIMIT-0015` moved from `HostProfile` to removed
  in the pass-4 use-site audit.) A single-axis model — the one the first
  revision of ADR-0021 was accepted with and then had withdrawn — would have put those 46 into a render-preparation
  input they have nothing to do with. The split paid for itself on its first application.
- **Classifying found a sixth silent-truncation site that neither search did.** `LIMIT-0004`: the render command accepts
  up to 384 kHz while `SampleRate::MAX_SUPPORTED` — the ceiling real-time look-ahead buffers size themselves from — is
  192 kHz, and `SampleRate::new` validates only positivity. The limiter's ring is sized `0.005 × 192 000` and its
  request is clamped to it, so a 384 kHz render silently delivers half the look-ahead the parameter advertises. Nothing
  is unsafe; the audio is simply not what was asked for, with no diagnostic. It does **not** trigger ADR-0021's revisit
  condition, because a class rule covers it — what was incomplete is the register, not the taxonomy. **It is fixed in
  V1**: the render ceiling now derives from the engine ceiling instead of restating it, three tests pin the result, and
  both measurement harnesses got the same bound so a future baseline cannot be taken on degraded DSP.
- **What `Classified` does not mean here.** The supporting evidence for every disposition is an accepted decision, not a
  measurement. **No value in the ledger has been measured.** A classified row says what happens when the limit is
  exceeded and who owns the number; it does not claim the number is right. That stays with each owner — P00A-T005 for
  `HostProfile`, the relevant contract or ADR for the other six.
- **What remains open, and is not this task's.** The executable probe ADR-0021 lists as follow-up — oversized blocks,
  more than 128 metered channels, more than 32 rack stages — which is the only thing that can close the completeness
  question all three passes record about themselves. `LIMIT-0004` is now the third argument for it.
- **Implementation revision.** The sweep itself was documentation only. The `LIMIT-0004` finding it produced was then fixed in code: a derived `MAX_RENDER_SAMPLE_RATE`, three tests, and the same ceiling in `render_cost` and `render_profile`.

**P00A-T003 — Capture V1 CPU, memory, timing, and determinism baselines.**

- **Scope.** Measured V1 figures for the reference corpus at common polyphony and
  sample rates, in a reviewable format.
- **Scope note added with CORPUS-0005.** All three evidence records were taken
  over the four cases that existed then. CORPUS-0005 postdates them, so it is
  measured by EVD-0001's method but is not part of EVD-0002's or EVD-0003's
  pooled figures — a pooled cost number is a property of the corpus that
  produced it, which EVD-0002 demonstrated when its per-case ratio ranged from
  +2.42% to +17.35%. Re-pooling is not required for the gate and would
  invalidate the cross-check between EVD-0002 and EVD-0003; the honest statement
  is that the baselines cover four of five cases, and it is recorded here rather
  than by quietly widening a record's claim.
- **State.** Complete over the four corpus categories that existed when it was
  measured. Determinism and
  level are in [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md);
  the block-size cost curve is in [EVD-0002](../evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md);
  and [EVD-0003](../evidence/phase-00a/EVD-0003-cpu-memory-timing-baseline.md) adds
  the three quantities that were missing — cost across sample rates and polyphony,
  per-block time against the block's real-time budget, and memory.
- **What the measurements say.** Cost per frame is constant, so cost per rendered
  second is proportional to sample rate to within 2% and the real-time factor
  falls from 171x at 44.1 kHz to 40x at 192 kHz. Cost is linear in polyphony at
  1.173 ms/s per voice plus 0.428 ms/s fixed, with R² = 1.0000 over 1 to 64
  voices. A 256-frame block costs 19 to 66 µs depending on the case and **does
  not depend on the sample rate** — but its budget does, so the same block goes
  from 3.1% of budget at 44.1 kHz to 13.3% at 192 kHz in the worst case observed.
  Peak RSS is 14 to 30 MiB per case.
- **What it deliberately does not measure.** Real-time headroom. There is no host,
  no device, and no callback deadline, so every timing figure is a lower bound on
  the same work live. That measurement needs the simulated host Phase 3 builds and
  ADR-0022 is deferred to.
- **An unplanned cross-check.** EVD-0003 re-measured EVD-0002's operating point a
  day later on a changed binary and landed 0.5% away (5.856 against 5.884 ms/s),
  which is evidence the harness measures the renderer rather than the session it
  ran in.
- **Implementation revision.** `render_profile` (new), `render_cost`'s
  `--sample-rate` and `--polyphony` flags, an opt-in per-block timing collector on
  `OfflineEngineSession`, and `corpus::fixtures::polyphony_probe`. The timing
  collector is `None` for every production caller, which costs one `Option` check
  per block; the cross-check above is the evidence that it costs nothing
  measurable.

**P00A-T006 — Satisfy every entry in the required-decisions table.**

- **Scope.** One accepted record under [decisions/](../decisions/README.md) for ADR-0001, ADR-0037, ADR-0021, and
  ADR-0032, plus an accepted-or-deferred ADR-0022 and ADR-0028.
- **State.** All six records now exist. **Four are `Accepted`**:
  [ADR-0001](../decisions/ADR-0001-internal-render-quantum.md) (quantum semantics),
  [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md) (admission policy),
  [ADR-0037](../decisions/ADR-0037-render-quantum-value.md) (quantum frame count, `Q` = 64 provisional), and
  [ADR-0032](../decisions/ADR-0032-sample-time-and-event-timestamps.md) (sample time and event timestamps) after
  three passes. **Every decision this gate requires on its own is accepted.**
  [ADR-0022](../decisions/ADR-0022-hardware-time-mapping.md) and
  [ADR-0028](../decisions/ADR-0028-long-running-job-contract.md) are `Deferred` to the Phase 3 and Phase 4 entry gates,
  each with an owner, the evidence still missing, and the constraints that hold while it is open. **The
  required-decisions table is now satisfied in full.**
- **A review withdrew the acceptance of ADR-0001 and ADR-0021**, which had been marked `Accepted` in the same session
  they were drafted. Four defects made that premature, and each is fixed in the record that carried it:
    - ADR-0001's splitting contract covered only the output side. A callback shorter than `Q` has neither the audio
      input nor the live events to render a full quantum, so the contract was unimplementable for any plan with live
      input. The decision now defines the input carry, the initial fill, the event horizon, and the `SampleTime` epoch,
      and the declared latency changed from `Q - 1` to a constant `Q`.
    - ADR-0021's class model put every configurable budget in `HostProfile`, but the ledger holds budgets that are not
      render-preparation inputs at all (`LIMIT-0063`, `LIMIT-0064`, `LIMIT-0066`, `LIMIT-0068`..`LIMIT-0071`). Failure
      behavior and configuration owner are now separate axes.
    - ADR-0021 hardcoded `64` while ADR-0037 was `Proposed`, and listed a render quantum among its `HostProfile` fields
      that ADR-0001 forbids. Both corrected.
    - ADR-0037's outcome rules overlapped — a measurement could satisfy both "select 32" and "select 128" — and never
      used its own 128-frame datapoint. Replaced by an ordered, exhaustive rule table with an explicit inconclusive
      case.
  A third, bounded closure review corrected the remaining retention, ownership, measurement, and host-fault issues before
  ADR-0001 and ADR-0021 were accepted.
- **Why ADR-0001 was split.** Only the frame count depends on the missing measurement; the splitting semantics follow
  from the partition-invariance requirement and from V1's code as read. Holding the semantics for a benchmark would
  block Phase 1 for no gain, so ADR-0001 states every clause in terms of `Q` and ADR-0037 carries the value. The gate
  is unchanged in strength: both must be `Accepted` at exit, and ADR-0037 is listed in the table above for that reason.
- **ADR-0037's measurement cannot be taken directly** — the quantity is per-quantum overhead in the V2 node model, and
  no V2 renderer exists. The record names a V1 proxy (render the corpus at 32/64/128/256 by varying
  `arrangement_render.rs`'s `BUFFER_SIZE`) and fixes its outcome rules before the data is collected. Review also
  withdrew the claim that the proxy errs in a safe direction: V1's per-block `resize` is not a real cost, because the
  buffer is preallocated at `MAX_BUFFER_SIZE` (`voice.rs:570`), while V2 adds carry copies and scheduler work V1 never
  paid. The proxy shows curve shape only, with an explicit inconclusive band.
- **The proxy was run, and it landed in that band.**
  [EVD-0002](../evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md) records 8 640 timed renders across four
  interleaved builds. `r(64,256)` is +9.9%, which is 5.07 pp from the 15% threshold on the primary estimator and
  4.62 pp on the mean — so which side of the 5 pp margin the measurement falls on is decided by the estimator rather
  than by the data, and rule 1 fires. `Q` = 64 is accepted provisionally, and the Phase 2 re-measurement is now an
  exit-gate item in the master plan rather than a follow-up note. Two things the measurement established that the
  record had assumed: per-block overhead is about 0.8 µs and scales with active voice and module count, and the pooled
  ratio depends visibly on corpus composition — per case it runs from +2.42% to +17.35%, straddling the threshold that
  would have selected 128.
- **ADR-0021 deliberately excludes numbers.** Its register basis named measurements; the record splits the topic so
  that class semantics and failure behavior — determinable from code already read — are decided there. P00A-T005 owns
  measured `HostProfile`/render defaults; other defaults stay with their node, domain/format, job, application, or
  protocol owner. The accepted register basis is `V1 cap inventory`.
- **What the records changed elsewhere.** Drafting ADR-0021 surfaced a contradiction in the resource ledger: the
  preamble claimed ADR-0021 owned every row of the silent-truncation register while `LIMIT-0013` and `LIMIT-0020`
  carried ADR-0027 alone. Both now carry ADR-0021 for the overflow question and ADR-0027 for tap ownership. That fix
  is independent of acceptance and stands. The five entries' `Proposed V2 rule` cells were filled while ADR-0021 was
  marked accepted and were **reverted** when that was withdrawn, per the ledger's own status rule.
- **The master plan is synchronized.** The Phase 0A `HostProfile` work item and field list name the maximum host block,
  not a configurable quantum. `RenderConfig` carries neither the quantum nor a duplicate `maximum_block_size`; all
  capacity comes through `HostProfile`. These edits landed in the acceptance change.
- **What ADR-0032 decided, and what it found.** `SampleTime` is a `u64` frame index in ADR-0001's engine-input epoch,
  with `FrameCount`, `FrameDelta`, `QuantumOffset(u16)`, and a `StreamEpoch` identifier; musical time is rounded to a
  frame at exactly one point; and every queued event carries `(epoch, time, provenance)` so that a stale, late,
  out-of-horizon, or arrival-stamped event is counted rather than silent. Reading V1 for the range analysis produced
  four findings the record cites: the cpal backend already computes a `u64` stream position and a measured per-callback
  output latency that `SynthEngine::process` never reads; two offline paths construct a `u64::MAX` sentinel for that
  unread field; live MIDI discards the driver's timestamp at `io/midi.rs:247` and is applied at the next block
  boundary, up to 21.3 ms at 48 kHz, which recording then anchors to a tick; and the master plan's intended
  `SampleOffset` name is already taken in `synth_core` by an unrelated `f32`. The name is changed to `QuantumOffset` in
  the plan in the same change.
- **What the second review pass changed.** Five defects, two substantive. The tempo map had been made to produce a
  `SampleTime`, which is not well defined: the render clock is monotone across seek and restarts at zero for every
  offline render, so one tick maps to many engine times and a precompiled event stream could not be timestamped at all.
  Musical time now converts to a `PlanPosition`, which the session scheduler anchors to the epoch (clauses 26-27), and
  the master plan gains that step. The second substantive defect was a `HostProfile` field with no semantics: one of
  the two required horizons was fully determined by ADR-0001's late-event rule, so P00A-T005 would have had to size a
  budget that controls nothing — there is now one forward horizon, corrected in the plan. The other three: the
  pre-epoch clamp had been made to fire the late counter on every stream start, contradicting ADR-0001 clause 16;
  exhaustion had no defined behavior although "overflow" is the register's own basis for this topic; and `StreamEpoch`
  permitted `A -> B -> A`, which a producer paused across two preparations could ride through the staleness check.
- **What the closure pass added.** Reviewing only the corrections found one more substantive defect: clause 17 sends a
  precompiled event list through the queue, and clause 21's forward horizon would then have rejected most of a song,
  which spans hours. The horizon now binds ingress provenance only, and clause 27 states the order of the whole path —
  compile to a plan position, anchor, stamp, enqueue — with the scheduler releasing compiled events as their quanta
  approach. Five smaller fixes came with it, including that a published `(epoch, time)` pair needs session scope
  because epochs restart at zero per process, and that a tempo edit invalidates every compiled plan position.
- **The two deferrals are written, not merely intended.** Each names its gate, its owner, its missing evidence, and —
  the part a bare register row could not carry — the constraints that hold while it is open. ADR-0022 forbids any path
  from consuming `output_latency`, a host timestamp, or `stream_time` before it is accepted, because reading one is
  what would create an unwritten time mapping; ADR-0028 forbids a second progress or cancellation mechanism beside
  `ExportProgress`, so a caller that needs one becomes evidence rather than a third bespoke channel. Writing them also
  produced the finding that V1 has three of the four pieces of a job contract — `ExportProgress`, `RenderReceipt`, and
  a wall-clock `LOAD_DEADLINE` — in three places, each for one caller.
- **Remaining in this task.** Nothing. P00A-T006 is **complete**: four accepted records and two written deferrals
  satisfy every row of the required-decisions table.
- **Implementation revision.** The ADR-0001/ADR-0021 work was documentation only, and its records cite source reads at
  `5cd24de8`, one commit later than the inventories' `dd69b657`. ADR-0037's acceptance required code: the
  `render_cost` measurement harness, and making `arrangement_render.rs`'s `BUFFER_SIZE` readable so a run can report
  the constant it was built with. The constant's value is unchanged. ADR-0032 was documentation only, with source
  reads at `7e361271`; it changes no V1 behaviour, including the four V1 findings it records. The two deferrals are
  documentation only as well, with source reads at `e4873d0b`.

## What building the corpus found

Producing the first corpus renders surfaced a defect in V1's offline renderer,
fixed on `fix/offline-render-fidelity` with a thirteen-case regression test.

`OfflineEngineSession` rebuilt each instrument without an `AllocatorConfig` and
replayed only volume, pan, and solo, so `max_voices`, `allocation_mode`,
`stealing_strategy`, the unison pair, `transpose`, `key_range`, `oversampling`,
both velocity sensitivities, and the sidechain source were left at engine
defaults. Nothing warned: the values were never sent rather than lost. A project
edited from `max_voices: 4` to `1` rendered byte-identically, and so did one
edited from `transpose: 0` to `12`.

Three consequences worth recording here rather than only in the commit.

- **It reached further than this phase.** `analyze_mix_bus`, `analyze_section`,
  the WAV export, and every other consumer of the offline renderer measured audio
  the live engine never produced. This is the third instance of the same shape —
  an offline reader disagreeing with the live engine while looking healthy —
  after the `analyze_*` snapshot bug and the save-barrier one.
- **It would have made CORPUS-0002 vacuous.** That case exists to force voice
  stealing; before the fix it rendered with the default eight voices and stole
  nothing, so a V2 with any allocator at all would have satisfied its preserve
  claims.
- **It is an argument for the corpus itself.** The defect had been reachable by
  every offline render for as long as the offline renderer has existed, and was
  found within hours of there being fixtures that set a non-default instrument
  field. The two inventories found two V1 defects by reading; this one needed
  something executable.

Also noted and **not** fixed: `StealingStrategy::Quietest` is implemented as
"oldest releasing voice, then oldest active" (its own `For now` comment), so it
is indistinguishable from `Oldest` on material with nothing in release. That is a
V1 behaviour gap, not a migration question; it is recorded here because a corpus
case that varied the stealing strategy would otherwise look like it was testing
something.

## Deliverables and verification

| Task      | Output/revision                                                                                                                                                        | Verification/evidence                                                                                                                                                                                                                                    | Result                                       |
|-----------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------|
| P00A-T001 | [Corpus manifest](../../../corpus/v2-reference/manifest.json) and five generated fixtures; five of eleven categories covered, six recorded as gaps | The manifest is loaded and validated by `cargo test -p pertylizer --test corpus_manifest`: category coverage, claim classes, digests, and the fact that each committed project is exactly what its builder produces. CORPUS-0005 adds two general fixture tests over `effect_chain_order`, which is matched by string and whose typo would otherwise be absorbed by V1's append path | Partial — format and validation done, five of eleven categories exercised |
| P00A-T002 | `pertylizer compare` and the versioned report model                                   | Each metric unit-tested against a synthetic signal with a known deviation (6.02 dB of gain, 100 ms of delay, a semitone of detune, a band-limited change, an inverted channel); end-to-end through render→render→compare in `compare_command.rs` | Complete — runs with no GUI and no audio device |
| P00A-T003 | [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md), [EVD-0002](../evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md), and [EVD-0003](../evidence/phase-00a/EVD-0003-cpu-memory-timing-baseline.md) | Two process-separate renders per case, bit-identical on all four, every comparison delta exactly zero, with a two-case control that resolves their octave to 3.6 cents; 8 640 timed renders giving the cost-versus-block-size curve; and 1 700 renders over four sample rates, seven voice counts, and 15 timing/memory profiles per operating point | Complete for the four categories that existed when it was measured — CPU, memory, timing, and determinism all measured, with real-time headroom explicitly out of scope |
| P00A-T004 | [Resource inventory](../inventories/resource-limits.md) passes 1-2 at `dd69b657` and the classification pass at `b435887c` | Two independent discovery methods with opposite blind spots — pass 1 matched constant names, pass 2 matched documented truncation behavior — then a third pass applying accepted ADR-0021 to all 74 entries: terminal class, owner, rule, and diagnostic each citing the clause that supports it. Classifying found a sixth silent-truncation site the searches missed. Neither search executes anything, so a truncation that is both unnamed and undocumented would still be missed | **Reopened by the pass-4 use-site audit.** Passes 1-3 all read definitions; pass 4 read *uses*, and only for the 30 entries that were `HostProfile`-owned when it started. It found `LIMIT-0015` misowned, `LIMIT-0014` misdescribed, two citations pointing at unrelated lines, a `<=` relation recorded as an equality, and three further silent-truncation sites — taking the register from six to ten, with five of the ten found outside any search. It also made two false findings of its own, both caught by external review. **The other 44 entries have not been use-site read**, so this task no longer meets its own all-limits scope |
| P00A-T005 | [Host profile and render limits](../specs/spec-host-profile-and-render-limits.md), `Draft`, invariant prefix `HOST` | Coverage is checkable rather than asserted: the specification maps each of the ledger's 28 settled `HostProfile`-owned entries to a successor field, with `LIMIT-0014` pending a split, lists the seven fields with no V1 antecedent, and answers the master plan's field list item by item. Every default carries one of four stated bases — queried, derived, V1 carry-over, or chosen-and-anchored — and exactly one value is derived from measurement. Each of the 21 invariants has a named conformance test in the phase that builds what it tests; none of those tests can exist before Phase 1. Eleven passes have run — author, independent (five, two High), bounded closure (four, one substantive), confirmation (two), independent (seven, two High), targeted (six, three High), and **five external** (five with three P1, five with one P1, six with two P1, four with two P1, three with one P1) — with every finding corrected and recorded in the specification's *Review status*. Six of the eleven were by an author. The five external ones found two long-standing misreadings and three defects in their predecessors' corrections, and four of them landed on HOST-INV-021 | Partial — the sixth pass's recommendation to close is **withdrawn**. Four blockers stand, all from external review: an **ADR-0001** clarification covering both when clause 16's late condition is evaluated **and** whether a quantum may defer at all under clauses 12 and 14; an **ADR-0021** one on the `Live bounded queue` class given to what are engine-egress rings; **`LIMIT-0014`'s GUI/OSC split**, without which `event_egress_capacity` leaves HOST-INV-005 unsatisfied; and **V2's renderer-ingress streams**, plus — separately, since deferral frees the upstream slot — a bound or exhaustion policy for the deferred store. All four may become Phase 3 work with a narrowed P00A-T005 |
| P00A-T006 | [ADR-0001](../decisions/ADR-0001-internal-render-quantum.md), [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md), [ADR-0037](../decisions/ADR-0037-render-quantum-value.md), and [ADR-0032](../decisions/ADR-0032-sample-time-and-event-timestamps.md) `Accepted` | Three review passes resolved the buffering, event, retention, ownership, host-fault, and measurement-boundary defects before the first two were accepted. ADR-0037 was accepted on EVD-0002 by applying the rule table it fixed before the data existed; the outcome was rule 1, so its value is provisional and binds Phase 2. ADR-0032 took three passes: an author pass, an independent pass that withdrew its acceptance over a tempo map producing engine times, a `HostProfile` horizon with no semantics, a pre-epoch clamp contradicting ADR-0001 clause 16, undefined exhaustion, and a reusable epoch identifier, and a bounded closure pass that caught the forward horizon rejecting a compiled song and fixed the order of the compile-anchor-stamp-enqueue path. ADR-0022 and ADR-0028 are `Deferred` to the Phase 3 and Phase 4 entry gates, each with an owner, its missing evidence, and constraints that hold while it is open | Complete — four accepted records and two written deferrals satisfy every row |

## Deviations

**One registered topic split into two identifiers.** The master plan's Part VII topic 1 names a single internal-quantum
decision, and the Phase 0A exit gate names `ADR-0001 (RenderQuantum)`. That topic is now carried by two records:
ADR-0001 for the splitting semantics and ADR-0037 for the frame count, both `Accepted`.

- **Why.** The frame count is the only part depending on a measurement that cannot yet be taken, and Phase 1 needs the
  semantics to begin. One record could not hold two statuses. Both are now `Accepted`; the split's purpose is served
  and its cost was one extra identifier.
- **How the gate is preserved.** ADR-0037 is added to this tracker's required-decisions table as `Accepted`-at-exit, so
  the exit gate still requires the quantum's value to be settled. The split moves where the value is recorded, not
  whether it must be decided.
- **What a reviewer should check.** That no clause of ADR-0001 silently assumes a particular `Q`, and that ADR-0037's
  acceptance criteria were fixed before its measurement was taken.
- **Master-plan sync, step 1 — done.** The plan is authoritative for exit gates, so it must not lag a registered split.
  [Part VII](../master-plan.md#part-vii-open-decisions) topic 1 now names both records and states which fixes what, and
  the [Phase 0A exit gate](../master-plan.md#phase-0a-baseline-limits-and-render-core-contracts) now requires both
  `Accepted`.
- **Master-plan sync, step 2 — done.** The Phase 0A `HostProfile` work item and field list now name the maximum host
  block rather than a configurable quantum, and `RenderConfig` no longer duplicates the profile field.

Any scope, ordering, or contract change must link to an ADR or an explicit master-plan update. Do not bury architecture
changes in this tracker.

## Exit readiness

Status: Not ready

The formal review must evaluate every Phase 0A exit gate in the master plan and link direct evidence. Phase 1 may not
begin until that review is accepted.
