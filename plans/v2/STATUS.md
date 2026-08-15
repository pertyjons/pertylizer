# Pertylizer Core V2 Status

| Field                  | Value                                            |
|------------------------|--------------------------------------------------|
| Last updated           | 2026-08-15                                       |
| Documentation stage    | Workflow accepted; first specification at `Draft` |
| Master plan status     | Proposed and architecture-audited                |
| Active migration phase | 0A and 0B, both `Active` in parallel             |
| Decision records       | 8 of 38 drafted: 4 accepted, 2 deferred, 2 proposed |
| Evidence records       | 5 (`EVD-0001`..`EVD-0005`), all `Complete`       |
| Executable Phase 0A    | Corpus, comparison command, and cost harness are used |

This is a current-state dashboard, not a work log. Replace stale information instead of appending a chronology.
Historical conclusions belong in ADRs, evidence records, phase reviews, and Git history.

## Current objective

Establish a reviewable Phase 0A baseline and the contracts required to begin the experimental Sound Core V2 path without
weakening or silently replacing the V1 production path. Phase 0B's migration inventories run alongside that work; they
gate Phase 10, not Phase 1.

## Current state

- The consolidated [architecture and migration plan](master-plan.md) exists.
- The V2 documentation responsibilities, registers, and templates are defined, reviewed, and **accepted**. All 23
  documents were checked: every internal Markdown link and heading anchor resolves, both phase trackers conform to
  [templates/phase.md](templates/phase.md), and the identifier series are consistent across README, registers, and
  trackers. No authority conflict was found.
- **The register now has two decision classes.** A `Contract` decision gets an individual record as before; a
  `Reversible` one — a value whose later change costs a rebuild and nothing else, meeting all four of
  [the reversibility test](ADR.md#the-reversibility-test) — is accepted as a register row with a value and a named
  revisit point, with no file, no options survey, and no evidence required before acceptance. ADR-0037 is the case
  that motivated it and would qualify; it is not reclassified, because an accepted record is immutable. No topic has
  been swept, and the class is judged when work begins on an entry.
- The decision topics are registered in [ADR.md](ADR.md), now 38 after one split and one partial supersession. Seven have individual records, and
  **four are `Accepted`** — [ADR-0001](decisions/ADR-0001-internal-render-quantum.md) (render quantum semantics)
  and [ADR-0021](decisions/ADR-0021-host-profile-and-admission-policy.md) (host profile and admission) after three
  review passes, [ADR-0037](decisions/ADR-0037-render-quantum-value.md) (quantum frame count) on the strength of
  [EVD-0002](evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md), and
  [ADR-0032](decisions/ADR-0032-sample-time-and-event-timestamps.md) (sample time and event timestamps), also after
  three. ADR-0037 is a split of the master plan's topic 1, recorded as a deviation in the Phase 0A tracker; both
  quantum records are required `Accepted` at the Phase 0A exit, so the split does not weaken the gate.
- **The decision half of Phase 0A is finished, and P00A-T006 is `Complete`.** Four records are accepted, and
  [ADR-0022](decisions/ADR-0022-hardware-time-mapping.md) and
  [ADR-0028](decisions/ADR-0028-long-running-job-contract.md) are written `Deferred` to the Phase 3 and Phase 4 entry
  gates — each with an owner, its missing evidence, and the constraints that hold while it is open. A deferral is not
  permission to improvise the decision in code: ADR-0022 forbids any path from consuming `output_latency`, a host
  timestamp, or `stream_time` before it is accepted, and ADR-0028 forbids a second progress or cancellation mechanism
  beside `ExportProgress`. What remains between here and the gate is measurement and coverage.
- **Writing ADR-0028 found the shape of the job problem.** V1 has three of the four pieces of a job contract and no
  contract: `ExportProgress` (progress and cancel, GUI only), `RenderReceipt` (a versioned receipt, headless CLI
  only), and a 300-second wall-clock `LOAD_DEADLINE` (lifecycle, one call site). MCP's renders and analyses are
  synchronous with no progress, no cancellation, and no way to observe them in flight.
- **ADR-0032 needed all three passes.** Its first acceptance, given on one author pass, was withdrawn by an independent
  one over five defects: the tempo map had been made to produce an engine `SampleTime`, which is not well defined
  because the render clock is monotone across seek and restarts at zero per offline render; one of two required
  `HostProfile` horizons controlled nothing; the pre-epoch clamp would have counted every stream start as late,
  contradicting ADR-0001 clause 16; exhaustion had no defined behavior; and the epoch identifier could be reused as
  `A -> B -> A`. The bounded closure pass then caught a sixth, in the correction itself: the forward horizon as
  rewritten would have rejected most of a compiled song, since a plan spans hours and clause 17 sends compiled lists
  through the queue. The horizon now binds ingress only, and the path is fixed as compile to a plan position, anchor,
  stamp, enqueue.
- **The contract.** `SampleTime(u64)` in the engine-input epoch, with `FrameCount`, `FrameDelta`,
  `PlanPosition`, `QuantumOffset(u16)`, and a strictly increasing `StreamEpoch`; musical time is rounded to a frame at
  exactly one point under a platform-independent law; every queued event carries `(epoch, time, provenance)` so that a
  late, stale, out-of-horizon, pre-epoch-clamped, or arrival-stamped event is counted rather than silent. The range
  analysis behind it is arithmetic, not measurement: `f32` stops being sample-exact after 349 s at 48 kHz, `u32` after
  6.2 h at 192 kHz, `u64` after three million years. Reading V1 to write it produced four findings — the cpal backend
  computes a `u64` stream position and a measured output latency the engine never reads, two offline paths build a
  `u64::MAX` sentinel for that unread field, live MIDI discards the driver's timestamp and lands on the next block
  boundary (up to 21.3 ms, then anchored to a tick by recording), and the plan's intended `SampleOffset` name is
  already an unrelated `f32` in `synth_core`. The last one renamed the V2 type to `QuantumOffset` in the master plan.
- **Every record accepted in the session that drafted it has had that acceptance withdrawn** — ADR-0001 and ADR-0021
  once, ADR-0032 once. All three then needed three passes. Treat a same-session acceptance as provisional until an
  independent pass has run, whatever the gate pressure, and expect the pass that reviews a correction to find something
  in the correction: ADR-0032's closure pass did.
- **`Q` = 64 frames, provisionally.** The V1 proxy landed inside ADR-0037's own inconclusive band — `r(64,256)` is
  +9.9%, and whether that is within 5 pp of the 15% threshold is decided by the choice of estimator rather than by the
  data — so the record's rule 1 applied: accept 64, and make re-measuring it against real V2 nodes a **Phase 2
  exit-gate item**. Until that passes, nothing may tune against the value: no hand-unrolled kernel, no `Q`-specific
  buffer layout, no test asserting a control rate in Hz. The measurement also produced the first V2-relevant cost
  model this project has: per-block overhead is about 0.8 µs and scales with active voice and module count.
- **The master plan is synchronized with all four accepted records.** ADR-0032 added the time types, the anchoring
  step, and one forward event horizon — and removed the backward one — to the Phase 3 work list and the Phase 0A
  `HostProfile` list. Part VII topic 1 and the Phase 0A exit gate name
  both quantum records; the quantum is no longer configurable, and `maximum_block_size` is owned only by `HostProfile`
  instead of being duplicated in `RenderConfig`. ADR-0037's acceptance added the re-measurement to the **Phase 2 exit
  gate**, because the plan — not the ADR — is authoritative for gates and a binding obligation recorded only in a
  decision record would not be enforced by one. Each change landed with the acceptance that caused it, per the
  same-change rule in [README.md](README.md#sources-of-truth).
- **The accepted contracts now unblock execution.** ADR-0001 fixes quantum, carry, end-of-stream, event-horizon, and
  latency semantics in terms of `Q`. ADR-0021 fixes admission behavior, seven configuration owners, an explicit lossy
  retention/presentation class, and a terminal `needs_reprepare` policy for an oversized host callback. ADR-0032 fixes
  the types those two are written in, and the path from a note to the frame it sounds on.
- All four [inventories](inventories/README.md) are `Active`: **76** `LIMIT`, 59 `STATE`, 55 `CAP`, and 31 `IDN`
  entries. The three non-resource ledgers have had two audit passes against `dd69b657`; the resource ledger has had
  five: two discovery passes, a classification sweep, and **two use-site audits** that between them cover all 75 pre-split
  entries (see below).
  **The use-site coverage claim now has an `EVD` record**, [EVD-0005](evidence/phase-00a/EVD-0005-resource-ledger-use-site-audit.md),
  which reports coverage `Supported` and accuracy `Inconclusive` and states plainly that it is retrospective. Pass 1 was a census from schemas and constant names; pass 2
  read the enforcing code, which resolved every gate-blocking question and **disproved three pass-1 hypotheses** — those
  are corrected in place rather than appended. Each ledger records both methods and what each is blind to; none is
  `Current`.
- **The [resource ledger](inventories/resource-limits.md) has had five passes plus a disposition pass.** Passes 4 and 5
  were the first to read what a constant is *used for*; between them they reopened four rows, and the disposition pass
  at `29c22ef4` closed them. The register is **76** entries — pass 4 added `LIMIT-0075`, and
  [ADR-0038](decisions/ADR-0038-engine-egress-queue-classification.md) split `LIMIT-0014` into a GUI ring and
  `LIMIT-0076`. Each entry carries a terminal failure class, a proposed V2 rule, and a diagnostic; no entry is
  `Unknown`; and **every entry has a settled owner for the first time**. The owner axis is where the interest is:
  **28 `HostProfile` and 48 elsewhere** — 12 node contracts, 10 removed outright, 8 protocol, 7 application settings,
  6 domain/format, 5 job policy. The single-axis model ADR-0021 was first accepted with, and then had withdrawn, would
  have put those 48 in a render-preparation input they have nothing to do with.
- **Four rows are `Investigating` rather than `Classified`, for two different reasons.** `LIMIT-0013`, `LIMIT-0014`
  and `LIMIT-0076` carry ADR-0038's disposition and wait only on that record being `Proposed` — a disposition's
  supporting evidence is an *accepted* decision, the register has two withdrawn same-session acceptances behind that
  rule, and this record is not exempt from it. **`LIMIT-0017` stays `Investigating` even after ADR-0038 is accepted**:
  its producer is off the render path, so ADR-0038's fourth condition applies — dropping there is a choice that must be
  argued — and no registered decision owns the multi-client hub's delivery contract. Only `LIMIT-0015` is fully
  resolved, by finishing an overflow read the audit had left partial. So 72 of 76 are `Classified` now and 75 of 76
  after ADR-0038.
- **Classifying found a silent truncation that neither search pass did, and it is now fixed.** `LIMIT-0004`: the render
  command accepted up to 384 kHz while `SampleRate::MAX_SUPPORTED` — the ceiling real-time look-ahead buffers size
  themselves from — is 192 kHz, and `SampleRate::new` validates only positivity, so a 384 kHz render silently got half
  the limiter look-ahead its parameter advertised. `MAX_RENDER_SAMPLE_RATE` now *derives* from the engine ceiling
  instead of restating it, with tests pinning the derivation, the refusal, and one consequence: with the ceiling halved,
  no legal request can reach `MAX_RENDER_BYTES` any more, so that guard is now a backstop against the other bounds
  moving rather than a check that fires. It is the sixth entry in the silent-truncation register and does **not**
  trigger ADR-0021's revisit condition — a class rule covers it, so what was incomplete is the register, not the
  taxonomy.
- **`Classified` in that ledger means disposed, not measured.** The supporting evidence for every rule is an accepted
  decision. No value in the ledger has been measured, and setting numbers stays with each owner.
- **Phase 0A now has something executable.** P00A-T002 is **complete**:
  `pertylizer compare` measures how two renders differ, with no GUI and no audio device.
  P00A-T001 is **still `Active`** — its infrastructure is done, its coverage is not.
  [`corpus/v2-reference/`](../../corpus/v2-reference/README.md) holds a validated manifest, a generator, and five
  fixture projects, but the master plan asks for eleven categories and five are covered. The other six are recorded
  as gaps with reasons, several blocked on decisions rather than on effort. Together the two supply the first half of
  the Phase 0A exit gate's first bullet: the corpus and the comparison command run headlessly.
- **[EVD-0001](evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md) is `Complete` and `Supported`.** Every corpus
  case renders bit-identically across two separate processes, and a two-case control resolves their octave to 3.6 cents
  — so the zero deltas are a measurement rather than a stub.
- **[EVD-0002](evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md) is `Complete`, `Supported` for the curve shape
  and `Inconclusive` at ADR-0037's resolution.** 8 640 timed renders across four interleaved builds, with a committed
  harness (`crates/pertylizer/src/bin/render_cost.rs`) rather than a one-off script. Its most consequential finding is
  not the ratio but the spread behind it: per case, `r(64,256)` runs from +2.42% to +17.35% and straddles the threshold
  that would have selected 128, so the pooled figure is a property of a four-of-eleven corpus as much as of the
  renderer.
- **[EVD-0003](evidence/phase-00a/EVD-0003-cpu-memory-timing-baseline.md) is `Complete` and `Supported`, and it
  closes P00A-T003.** 1 700 renders over four sample rates and seven voice counts, plus 15 timing and memory profiles
  per operating point. Cost per frame is constant, so cost per second is proportional to sample rate to within 2% and
  the real-time factor falls from 171x to 40x between 44.1 and 192 kHz. Cost is linear in polyphony —
  `1.173 ms/s per voice + 0.428 ms/s`, R² = 1.0000 over a 64-fold range — so V1 has no per-voice interaction cost in
  that patch. The finding with consequences is the third: **a 256-frame block costs the same 19-66 µs at every sample
  rate, while its budget shrinks with the rate**, so the same block goes from 3.1% of budget at 44.1 kHz to 13.3% at
  192 kHz. An admission policy that reasons in frames rather than in seconds would get that backwards. Peak RSS is
  14-30 MiB per case. Real-time headroom is explicitly *not* measured: there is no host and no deadline, so every
  timing figure is a lower bound on the same work live.
- **Building the corpus found a V1 defect, fixed on `fix/offline-render-fidelity`.** The offline renderer rebuilt instruments without an
  allocator config and replayed only volume, pan, and solo, so polyphony, allocation mode, transpose, key range,
  oversampling, velocity sensitivities, and the sidechain source were silently defaulted. It affected every consumer of
  the offline renderer, not only the corpus — the `analyze_*` tools and the WAV export measured audio the live engine
  never produced. It is the third instance of an offline reader disagreeing with the live engine while looking healthy.
- **The `HostProfile` contract is the register's first specification, and it has been reviewed eleven times.**
  [Host profile and render limits](specs/spec-host-profile-and-render-limits.md), invariant prefix `HOST`, is at
  `Draft` with 21 invariants, a conformance test named for each, and a value for every field the master plan lists plus
  ADR-0032's forward event horizon. Each of the ledger's 28 `HostProfile`-owned entries has a named successor field —
  total for the first time since ADR-0038 — and `event_queue_capacity` is withdrawn, its ledger antecedent having
  turned out to be a channel V1 never constructs; seven fields have no V1 antecedent. **Exactly one value is derived from measurement** — ADR-0021 asked P00A-T005 for
  measured defaults and EVD-0003 measured cost, not capacity — with two more *chosen and anchored on* it. Every default
  carries one of those stated bases rather than implying a measurement the record does not have.
- **The independent review pass found five defects, two of them High, and all are corrected.** The first was a
  `maximum_block_size >= Q` clause that would have **refused hosts the render model is built for**: ADR-0001 clause 6
  primes the output carry precisely so that a callback of `N < Q` can be served, and the constraint had no purpose. The
  second was a runtime contract that permitted loss only at live bounded queues while three fields visibly did
  something else — the telemetry ring overwrote, a recording take stopped, and an over-full quantum had no defined
  behaviour at all. (That finding was argued from "the four ingress queues hold 3 072 events against a 256-event
  scratch", which external review later showed to be false — those rings are engine egress. The finding stands; its
  supporting number did not.) The specification
  now names **five distinct runtime behaviours** — admission refuses, a queue drops, a quantum defers, a lossy budget
  evicts, a session limit stops — with three new invariants carrying them, and each field takes exactly one.
- **Applying the review found a sixth defect, in the corrections.** `max_held_notes` had been sized to the
  per-instrument voice ceiling on the reasoning that a held note cannot outnumber a voice. It can — sustain pedal,
  stealing allocator, MPE or sequencer source — and the allocator tracks a held note precisely so that it can re-sound
  one. It now carries its own type so the two concepts cannot be assigned to each other.
- **The bounded closure pass has run: four findings, one substantive, and it is in the previous pass's own
  correction.** HOST-INV-021 had reused ADR-0001 clause 16's late counter and position rule for a deferred event, and
  neither fits — a deferred event's producer was on time and the **engine** was full, so counting it as late would
  publish a capacity shortfall as an external timing fault, and clause 16's "first not-yet-rendered quantum boundary"
  is circular for a quantum that has itself not rendered. ADR-0032 clause 22 is the precedent: it separated the
  pre-epoch clamp from the late counter on identical grounds and warned that one test would pass on the wrong policy.
  The other three: the forward horizon's flat one-second default **could fail its own validation** on a device with a
  very large block, and now takes a derived floor; `max_concurrent_retiring_voices` = 64 **recreated the defect the
  previous pass had just fixed** — a runtime-enforced field with no defined behaviour on reaching it — and is now
  derived from `max_active_voices` so it cannot bind; and HOST-INV-009's four-behaviour partition was false twice over,
  omitting admission refusal and having no place for a field that is a size rather than a bound.
- **The confirmation read ran and found two more; the independent read it asked for then found seven, two of them
  High** — and the first of those was in the confirmation read's own correction, which had charged a delayed scheduler
  release to ADR-0001 clause 16's late counter. The second: **HOST-INV-021 said *that* the excess defers but never
  *which* events are the excess.** Admission is now compiled-before-ingress then ascending position. Deferral also
  moved from the quantum boundary to `+Q`, since collapsing to the boundary shifted a **sample-positioned** event off
  its declared sample, which ADR-0001 clause 14 preserves.
- **A targeted pass over those two rules then found six, three High**, all inside its targets: `+Q` had never said
  whether it rewrites the event's `SampleTime` — which decides whether displacement is reportable, and whether
  ADR-0032 clause 7's take resolution silently quantizes a played performance forward under overload; the stated reason
  for subordinating queue priority **proved too much**, since rule 1 reorders against the position exactly as priority
  ordering would; and preserving the offset created a **second starvation channel**, ingress against ingress.
- **Then an external pass — the first by a reader who authored none of the document — found five, three P1**, and the
  worst had stood since the first independent pass. The specification had been reasoning from `LIMIT-0013`'s
  prioritized rings as **renderer ingress**; they carry `EngineEvent` *out* of the engine toward the GUI, and V1 does
  not wire the channel to anything. The "3 072 events against a 256-event scratch" motivation for HOST-INV-021 was
  therefore never true, and **V1 has no timestamped renderer-ingress queue to carry over at all** — so the profile has
  no field for the capacity deferral operates against. Second: `+Q` de-orders a FIFO, so the five-way merge the sixth
  pass made normative could not work even under its own monotone-enqueue precondition; the merge is withdrawn to a
  constraint, and a bounded deferred store is a further missing capacity. Third: suppressing ADR-0001 clause 16's late
  counter **overrode an accepted decision** — clause 16 triggers on a condition, not a cause — so both counters rise.
- **A second external pass over those corrections found five more, one P1**: the immutable stamp and ADR-0001 clause
  16 **cannot both be implemented as written** — once the quantum a deferred event could not enter has rendered, its
  preserved timestamp does fall in an already-rendered quantum, which is clause 16's condition, while HOST-INV-021
  forbids the late counter there. The interim rule is that the condition is asked once, when an event first becomes
  due; because that narrows an accepted decision, it needs an **ADR-0001 clarification or successor before Phase 3**
  rather than a ruling from the specification. The other four were stale duplicates of corrections already made
  elsewhere in the file, plus a missing `file:line` citation on the queue-direction claim itself.
- **A third external pass found six more, two P1**, both consequences of the queue-direction discovery that the
  previous correction had propagated only partway: the **bounded deferred store had no exhaustion policy**, so
  HOST-INV-021's "no event is lost" could not coexist with the audio thread's prohibition on allocating; and
  `LIMIT-0013`'s rings **kept the `Live bounded queue` class** after being identified as egress, a class ADR-0021
  reserves for queues fed by external unbounded input. The other four were consistency defects the corrections
  themselves introduced, including three documents disagreeing on the pass count.
- **A fourth external pass found four more, two P1**, both in the third's corrections: the deferred-store size did not
  bound what it claimed — deferring *frees* the upstream slot, and note expansion multiplies one released event into
  many — and the admission tie-break named ingress rings this document had just established do not exist.
- **A fifth external pass found the other half of the same premise, and it lands in P00A-T004.**
  `max_events_per_quantum` = 256 is **not** a V1 carry-over — `EVENT_BUFFER_SIZE` is an egress ring size, and V1's real
  per-block sequencer buffer is an uncapped `Vec::with_capacity(128)`. **`LIMIT-0014`'s ledger description is wrong**,
  recorded from a constant's name by two discovery passes with the class pass not reaching its use sites.
- **The use-site audit that finding called for has now run as ledger pass 4, over the 30 entries that were `HostProfile`-owned when it started.**
  Five findings: `LIMIT-0015` is not a return-bus scratch but four deferred-drop channels, so it moves to
  `N/A — removed` and the owner split became **28 `HostProfile` / 1 undecided / 46 elsewhere, with 9 removed** at that point — it is 28 / 48, with 10 removed and none undecided, after ADR-0038; `LIMIT-0024` is a seventh silent-truncation
  site (sends beyond sixteen dropped per block); two citations pointed at unrelated lines; and `LIMIT-0023`/`LIMIT-0041`
  are a `<=` floor relation rather than one capacity, so the profile now carries two fields. **The audit also made two
  false findings of its own**, both caught by external review and both the very failure it was convened to correct — an
  absence asserted from a name search, and a truncation asserted from one line without reading the `push` that feeds
  it, plus two more silent-truncation sites external review found while checking the corrections — `EngineEvent` pushes
  discarded with `let _ =`, and a visualization ring that drops the *newest* samples with no omitted count. The
  register is now **nine** entries, four of them found outside any search. **P00A-T004's `Complete` status no longer
  holds under its own all-limits scope**: the audit reopened it. **Pass 5 has since completed the sweep — all 75
  entries are use-site read.** What keeps the task open is no longer coverage but the **four** rows that left
  `Classified` — `LIMIT-0013`, `LIMIT-0014`, `LIMIT-0015`, `LIMIT-0017` — and the citations that need re-pinning.
- **Eleven passes, six by an author of the document and five external. Every one found something.** Three of the five
  external passes found a defect in the immediately preceding correction; the other two found long-standing misreadings
  no author pass had questioned. Four of the five landed on the same object: **HOST-INV-021**. Deferral cannot be
  specified without knowing what feeds the renderer and how much of it there can be, and **every V1 number the
  invariant was built on has now been falsified**. **The gap is in the contract, not in the reviewing.** ADR-0001, ADR-0021 and ADR-0032 recorded one correction-defect each; P00A-T005 has now produced four of
  its own. The rule this project already had — **treat a correction as new material, not a fix** — gains a companion:
  **a number that supports the conclusion you already hold does not get audited**, and an author's pass will not find
  it. An author re-reads the reasoning; an external reader checks the claims.
- **EVD-0003's finding produced a field, not just a number.** A block costs the same absolute time at every sample rate
  while its real-time budget shrinks with the rate, so the profile carries an advisory cost budget expressed as a
  *ratio of times* — 0.15 of the quantum's real-time budget, from the observed 2.7x–6.8x max-to-median block spread.
  It is the only field in the profile that is not a capacity, and it is the reason a count-only profile would admit a
  512-voice plan identically at 44.1 kHz, where it costs about 60% of one measured core, and at 192 kHz, where the same
  plan is not real time at all.
- **The specification stays `Draft`, and P00A-T005 is narrowed rather than closed on the current text.** The sixth
  pass's recommendation to close on the whole contract stays withdrawn: it rested on that pass's findings being
  contained by its targets and on its additions being preconditions rather than mechanism, and the external pass
  falsified both. [REV-P00A](reviews/phase-00a-exit-review.md) has since made the scoping call the tracker said it
  owned, and it did **not** split the four blockers the way the question was posed — two are gate-bound to Phase 0A
  and could not move. The specification now closes on the master plan's field list, with **HOST-INV-021 `Deferred` to
  Phase 3, its identifier retained rather than deleted and renumbered**, and with the ingress capacities and the
  deferred store recorded in the master plan's Phase 3 work list and entry gate. Two things the narrowing had to keep:
  `max_events_per_quantum` stays normative, being `LIMIT-0075`'s successor and the only bound on V1's uncapped
  per-block `Vec`, and Phase 1's event boundary gains an explicit rule in two halves — a preparation-time refusal for
  what is statically knowable, and a caller precondition for the per-call span that the master plan's `render`
  signature **cannot currently report**, listed as blocking for Phase 1 rather than presented as settled. Both because
  Phase 1 **does** render offline and would otherwise accept arbitrary input with no defined
  error. Also still open there: ADR-0021's two structural choices — the `HostCapabilities`/`RenderLimits` split, and a
  `CapabilitySource` tag with two constructors, which is what makes the queried-capability rule enforceable by API
  shape instead of by a runtime tag that cannot prove a query happened.
- **CORPUS-0005 covers `instrument-inserts`, taking the corpus to five of eleven categories.** Four claims, each
  measured against a counterfactual **and a null control** in
  [EVD-0004](evidence/phase-00a/EVD-0004-corpus-0005-claim-counterfactuals.md): the chain runs on the summed voices
  (−3.07 dB relative), its state is shared across voices (−35.21 dB — two notes can only interact inside a delay line
  they share), that state outlives the notes (a tail 2.3 s past the last note-off against a silent control), and the
  authored chain order is load-bearing (5.97 dB). The null control sits at −147 dB relative, which is floating-point
  rounding, so every figure is attributable.
- **Three attempts, and the two failures are the transferable part.** The second one ran the counterfactual without a
  null control, got a plausible +0.67 dB, and then measured −1.41 dB on a project with no effects at all. It concluded
  that V1's renders are not additive across polyphony and wrote a migration contract against a sequencer timing
  defect. **There was no defect**: `Oscillator`'s `uni_phase` defaults to full randomization, seeded from the voice
  index, so a note starts at a different phase depending on which voice took it. With it off the control drops to
  −147 dB and both withdrawn claims come back stronger. A control that fails is telling you about your fixture.
- **The finding that survives is about the corpus, and P00A-T001 has acted on it.** `uni_phase` defaults to full
  note-on phase randomization, seeded by voice index and advanced on every note-on, so every fixture pinned a phase
  sequence that depends on which voice took which note. A V2 with an equivalent but differently-indexed allocator would
  change that without changing any claimed behaviour. **Now off in all five fixtures**; four committed digests changed.
- **The cost of that took three measurements to say anything.** EVD-0001 gains a revision: its four input digests are
  superseded and determinism was re-asked — all five cases bit-identical across two `--release` processes, full
  replacement digests recorded. EVD-0003's first check reported +2.2% and argued the shift was real because it was
  *uniform*; review objected that a small-sample minimum is biased upward and that the bias is uniform too, so that
  argument is withdrawn. Re-run at EVD-0003's own 50 draws per case the pooled minimum is **−4.0%** — the opposite
  sign — while resampling puts the small-sample bias at **+0.14%**. **The objection holds, its proposed mechanism does
  not**: what dominates is session-to-session variation on an unquiesced machine. No cost claim about the fixture
  change is supportable at this resolution, in either direction.
- **EVD-0002's and EVD-0003's pooled figures still cover four of five cases, and are not being widened.** They are
  properties of the cases that produced them — EVD-0002 showed exactly that, with a per-case ratio spanning +2.42% to
  +17.35% — so re-pooling would break the cross-check between the two records. Read them for shape rather than as
  absolute costs; a V2 comparison re-measures V1 in the same session.
- Phase 0A is **not** complete, and [REV-P00A](reviews/phase-00a-exit-review.md) now says so against the gates rather
  than by inference. **Two of the six exit gates fail.** The named-comparison-category gate fails on P00A-T001's
  five-of-eleven corpus coverage: a semantic change that would surface only in `sampler-patch` or
  `tempo-map-arrangement` has had no opportunity to be named. The resource-inventory gate fails twice over — four
  rows wait on ADR-0038's acceptance, which is mechanical, and **completeness is not demonstrable by any method used
  so far**, which is not. Every ledger pass is a search or a read, and a truncation that is both unnamed and
  undocumented is invisible to all of them; ADR-0021's executable probe is the only instrument for it and is unbuilt.
  **Three of seven tasks are `Complete`** — T002, T003, T006. P00A-T004 has **two** blockers now that its coverage
  claim has [EVD-0005](evidence/phase-00a/EVD-0005-resource-ledger-use-site-audit.md): **completeness**, and
  **`LIMIT-0017`**, whose loss policy no registered decision owns and which therefore stays `Investigating` even after
  ADR-0038 is accepted. P00A-T005 is narrowed and closes when an independent pass over the narrowing has run. P00A-T007
  is `Active`: the review exists at `Draft`.
- V2 implementation status must be established from repository evidence before this dashboard makes code-level
  completion claims.

## Next actions

Three of Phase 0A's seven tasks are `Complete` — the comparison command, the measurements, and the decisions.
[REV-P00A](reviews/phase-00a-exit-review.md) is open at `Draft` and has made the scoping call, so the order below is
what it left, in dependency order rather than in the order the work was found.

1. **Run an independent pass over the narrowing change, then accept ADR-0038.** The change touches the ledger, the
   host-profile specification, the master plan, and the exit review; the specification's last four corrections each
   contained a defect found by the pass that reviewed them, so this one is reviewed the same way. The pass is
   deliberately **narrow** — the narrowing diff and the four disposed ledger rows, not a twelfth read of the whole
   specification. Six of the eleven earlier passes were by an author, and that is the variable that mattered. When
   ADR-0038 is accepted, `LIMIT-0013`, `LIMIT-0014` and `LIMIT-0076` move to `Classified` and P00A-T005 closes.
2. **Build ADR-0021's executable truncation probe** — oversized blocks, more than 128 metered channels, more than 32
   rack stages. It is the only instrument that can move gate bullet six's completeness half, and it now has four
   arguments rather than one: `LIMIT-0004`, `LIMIT-0024`, `LIMIT-0014` and `LIMIT-0021` were each found by a method the
   previous one could not see, and none by searching.
3. **Add corpus cases as their blockers clear**, which is what the other failing gate waits on. Instrument inserts are
   done (CORPUS-0005); the sampler case waits on the bundle round-trip fixtures, the shared-instrument case on
   ADR-0014, and the tempo-map case on whether a ramp's event positions fall under the sample-timing correction — still
   open, since ADR-0032 fixed only that the conversion is rounded once and stays platform-independent, leaving the ramp
   law itself to Phase 3. EVD-0002 raised the stakes: the corpus's category mix, not only its size, demonstrably moves
   a measured result.
4. **Review [ADR-0014](decisions/ADR-0014-persistent-id-generation-and-encoding.md) independently.** It is `Proposed`
   after an author pass and one review pass that found four defects, three of them P1 with a single root: the first
   revision derived allocation state from surviving content, so deleting the highest-ordinal entity reissued its
   ordinal, a file copy produced two documents minting from one origin, and nothing said which origin allocates after
   a merge. The model now carries a **validated allocation record** — one origin and one high-water mark, checked
   against the document on load — and forking mints a fresh origin while remapping nothing. The two clauses worth
   attacking next are the ones the fix could not close: a document copied *outside* the application still mints from
   a shared origin, detected only at merge, and clause 11's ban on deriving audio state from identity is what unblocks
   the corpus's shared-instrument case.
5. **Write the first round-trip fixture (P00B-T005) for `STATE-0004`** — changing the focused instrument changes the
   saved file while no dirty term observes it. It is the cheapest executable check the ledgers produced, and it is
   Phase 0B's rather than this phase's.

**Not on this list, and deliberately:** the four smaller questions travelling with the deferred mechanism — an age term
in the deferral admission order, a reserved ingress allowance, a `Hardware`-before-`Arrival` ingress tier needing
ADR-0022's calibration evidence, and whether `retirement_crossfade` and `telemetry_ring_frames` should be stated in
seconds rather than frames. All four are Phase 3's, and the last is the one with an audible consequence: both shrink
4.4x across the supported rate range.

## Documentation-workflow review notes

Two conformance details were noted during the acceptance review and left as they are, because both trackers carry the
information the template asks for:

- Neither phase tracker uses the template's exact `Required decisions` column set. Phase 0A uses
  `Required at Phase 0A exit` / `Later acceptance gate`; Phase 0B uses `Topic` / `Earlier deadline, if any` and states
  the required status in prose above the table rather than in a column.
- The trackers' task tables use a `Primary record` column where the template has `ADRs/inventories`.

## Blockers

**The Phase 0A exit review has made the scoping call, and the four blockers split two-and-two — but not the way the
question was posed.** Two could not move to Phase 3, because the exit gate's resource-inventory clause binds them; they
are settled inside Phase 0A instead. See [REV-P00A](reviews/phase-00a-exit-review.md).

**Settled in Phase 0A by [ADR-0038](decisions/ADR-0038-engine-egress-queue-classification.md), which is `Proposed`:**

1. **The engine-egress classification.** ADR-0021 permits runtime overflow only for queues fed by external unbounded
   input, which leaves `LIMIT-0013`, `LIMIT-0014` and `LIMIT-0017` — bounded queues the engine feeds — with no
   admissible failure behaviour at all. The blocker was stated as a `LIMIT-0013` class question; it is wider, since
   `LIMIT-0017` has the same mismatch. ADR-0038 defines engine egress as a direction and makes the failure class a
   property of the **payload** rather than of the ring, which is what keeps a uniform drop policy from demoting
   `RecordedNotesFlushed` to a counted loss.
2. **`LIMIT-0014`'s GUI/OSC split.** ADR-0038 part 4 splits it: the GUI ring keeps `LIMIT-0014` under `HostProfile`,
   and the OSC note-telemetry ring becomes `LIMIT-0076` under the protocol contract that serializes it — so
   `event_egress_capacity` is one field, not two, and HOST-INV-005 is satisfied for it.

**Moved to Phase 3, recorded in the master plan's work list and entry gate rather than only in a review:**

3. **An ADR-0001 clarification or successor** — when clause 16's late condition is evaluated, and whether a quantum may
   defer at all under clauses 12 and 14. The two cannot both be implemented as written.
4. **V2's renderer-ingress streams, and — separately — a bound and exhaustion policy for the deferred store.** Not one
   item: deferring frees the upstream slot, so ingress capacity bounds arrival rate rather than backlog.

**The one blocker that remains, and it is not on that list: gate bullet six's completeness half.** Every ledger pass so
far is a search or a read, and a truncation that is both unnamed and undocumented is invisible to all of them —
[EVD-0005](evidence/phase-00a/EVD-0005-resource-ledger-use-site-audit.md) says so as its own conclusion. ADR-0021's
executable probe is the only instrument for it and has not been built. P00A-T001's five-of-eleven corpus coverage is
the other failing gate.

Open decisions become blockers only when a phase task or exit gate requires them to be accepted.

## Phase overview

| Phase | Name                                     | Status      | Tracker                                                          | Exit review |
|-------|------------------------------------------|-------------|------------------------------------------------------------------|-------------|
| 0A    | Baseline, limits, and render contracts   | Active      | [Tracker](phases/phase-00a-baseline-and-render-contracts.md)     | [REV-P00A](reviews/phase-00a-exit-review.md) — `Draft`, two gates fail |
| 0B    | Inventories and project contracts        | Active      | [Tracker](phases/phase-00b-inventories-and-project-contracts.md) | Not created |
| 1     | Experimental Sound Core V2 crate         | Not started | Create when activated                                            | Not created |
| 2     | Minimal compiled voice graph             | Not started | Create when activated                                            | Not created |
| 3     | Sample-accurate scheduler                | Not started | Create when activated                                            | Not created |
| 4     | V1 lowering and offline A/B              | Not started | Create when activated                                            | Not created |
| 5     | Declarative node and parameter API       | Not started | Create when activated                                            | Not created |
| 6     | Polyphony and instrument runtime         | Not started | Create when activated                                            | Not created |
| 7     | YAMS and unified modulation              | Not started | Create when activated                                            | Not created |
| 8     | Mixer, buses, effects, and latency       | Not started | Create when activated                                            | Not created |
| 9     | Live integration and plan swapping       | Not started | Create when activated                                            | Not created |
| 10A   | Canonical project model and identity     | Not started | Create when activated                                            | Not created |
| 10B   | Application operations and transactions  | Not started | Create when activated                                            | Not created |
| 10C   | History, dirty state, save, and recovery | Not started | Create when activated                                            | Not created |
| 10D   | Project Format V2, assets, conversion    | Not started | Create when activated                                            | Not created |
| 10E   | MCP, CLI, import, and service migration  | Not started | Create when activated                                            | Not created |
| 11    | GUI and workflow migration               | Not started | Create when activated                                            | Not created |
| 12    | Default cutover and V1 retirement        | Not started | Create when activated                                            | Not created |

Phase 0A gates Phase 1; Phase 0B gates Phase 10 and runs in parallel with Phases 1-4. Phase 10 has no gate of its own
and is complete when 10A–10E are. Every sub-phase has its own tracker, exit gate, and review.

## Status maintenance

Update this file when the active task, phase, blocker, or next action changes. Do not mark a phase `Complete` until its
formal exit review is accepted. Do not list speculative implementation progress as verified status.
