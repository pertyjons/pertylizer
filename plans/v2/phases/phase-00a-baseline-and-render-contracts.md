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
| P00A-T001 | Define the reference V1 corpus and preserve/change manifest | Active      | None                 | [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md) |
| P00A-T002 | Define the comparison result model and headless command     | Complete    | P00A-T001            | [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md) |
| P00A-T003 | Capture V1 CPU, memory, timing, and determinism baselines   | Complete    | P00A-T001            | [EVD-0003](../evidence/phase-00a/EVD-0003-cpu-memory-timing-baseline.md), with EVD-0001 and EVD-0002 |
| P00A-T004 | Complete the fixed-limit and overflow audit                 | Complete    | None                 | [Resource inventory](../inventories/resource-limits.md) |
| P00A-T005 | Define the initial HostProfile and RenderLimits contract    | Not started | P00A-T004            | Future specification/ADRs                               |
| P00A-T006 | Satisfy every entry in the required-decisions table         | Complete    | P00A-T003/P00A-T004  | [Decision register](../ADR.md)                          |
| P00A-T007 | Prepare the formal Phase 0A exit review                     | Not started | All applicable tasks | Future `REV-P00A`                                       |

Phase 0B runs in parallel and has its own tracker. Do not move its inventory work into this phase to make the gate look
complete.

P00A-T006 must accept ADR-0001, ADR-0037, ADR-0021, and ADR-0032, and it has. ADR-0022 may be deferred only to the
Phase 3 entry gate and ADR-0028 only to the Phase 4 entry gate; either deferral records an owner and the missing
evidence. Both are now written on those terms, so no other deferral is in play.

## Active tasks

**P00A-T001 — Define the reference V1 corpus and preserve/change manifest.**

- **Scope.** The master plan's [Phase 0A work list](../master-plan.md#work) names eleven categories of representative V1
  render. The task is complete when all eleven are captured, not when the format that holds them exists.
- **State.** The manifest, the model, the generator, and the validation are done and in use. Four categories are
  exercised: `subtractive-voice`, `polyphonic-voice-stealing`, `mod-matrix-patch`, and `sends-returns-master`. Seven are
  recorded as gaps with reasons.
- **Why this is `Active` and not `Complete`.** A recorded gap is an honest statement about coverage, but it is not the
  baseline the plan asks for. Marking the task complete with seven of eleven missing would move the shortfall out of the
  task list and into a document nobody re-reads, and the Phase 0A exit review would then have to rediscover it. The
  register is authoritative for what the task is; this tracker does not get to shrink it.
- **Remaining, in the order the blockers clear.**

  | Category                     | Blocked on                                                                        |
  |------------------------------|-----------------------------------------------------------------------------------|
  | `instrument-inserts`         | Nothing. Cheapest of the seven; build it on CORPUS-0001                            |
  | `stereo-or-spatial-voice`    | Choosing the source. The Spatial Panner's positions are not modulatable in V1, so a case built on it pins behaviour already scheduled to change |
  | `tempo-map-arrangement`      | Whether a tempo ramp's event positions fall under the sample-timing correction, which the case must cite rather than decide |
  | `yams-control-patch`         | A script small enough that a render difference names one language feature          |
  | `yams-audio-script-patch`    | Same, and see the note below                                                       |
  | `sampler-patch`              | The Phase 0B bundle round-trip fixtures; a sample makes the input a bundle and pulls asset identity (ADR-0027) into a Phase 0A baseline |
  | `shared-patch-or-instrument` | ADR-0014. A module's script PRNG seed derives from its instance number (`IDN-0029`), so a shared instrument's audio depends on how the two references are numbered |

- **Correction to an earlier note.** The `yams-audio-script-patch` gap said the case "should be authored together with
  the ADR-0037 measurement". That reads as a dependency and is not one: ADR-0037's proxy is defined over whatever the
  corpus contains, and waiting for an AudioScript case would block a measurement that is otherwise ready. The two are
  independent, and the gap entry now says so.

## Completed tasks

Kept here rather than deleted: the exit review has to be able to see how a completed gate item was satisfied.

**P00A-T004 — Complete the fixed-limit and overflow audit.**

- **Scope.** Populate the [resource inventory](../inventories/resource-limits.md) with every fixed cap, truncation
  point, bounded queue, buffer capacity, and script budget in the workspace, each with its enforcement site, its
  overflow behavior, a proposed V2 rule, and a diagnostic.
- **State.** Complete. Three passes: two discovery passes with opposite blind spots at `dd69b657`, and a third
  classification pass at `b435887c` that applied accepted ADR-0021 to all 74 entries. Every entry now carries a terminal
  failure class, one of the seven configuration owners, a rule, and a diagnostic. **No entry is `Unknown`** — the 19
  that were are resolved.
- **What the owner axis showed.** 30 entries are `HostProfile`, and 44 are not: 12 node contracts, 7 protocol, 7
  application settings, 7 removed outright, 6 domain/format, 5 job policy. A single-axis model — the one the first
  revision of ADR-0021 was accepted with and then had withdrawn — would have put those 44 into a render-preparation
  input they have nothing to do with. The split paid for itself on its first application.
- **Classifying found a sixth silent-truncation site that neither search did.** `LIMIT-0004`: the render command accepts
  up to 384 kHz while `SampleRate::MAX_SUPPORTED` — the ceiling real-time look-ahead buffers size themselves from — is
  192 kHz, and `SampleRate::new` validates only positivity. The limiter's ring is sized `0.005 × 192 000` and its
  request is clamped to it, so a 384 kHz render silently delivers half the look-ahead the parameter advertises. Nothing
  is unsafe; the audio is simply not what was asked for, with no diagnostic. It does **not** trigger ADR-0021's revisit
  condition, because a class rule covers it — what was incomplete is the register, not the taxonomy.
- **What `Classified` does not mean here.** The supporting evidence for every disposition is an accepted decision, not a
  measurement. **No value in the ledger has been measured.** A classified row says what happens when the limit is
  exceeded and who owns the number; it does not claim the number is right. That stays with each owner — P00A-T005 for
  `HostProfile`, the relevant contract or ADR for the other six.
- **What remains open, and is not this task's.** The executable probe ADR-0021 lists as follow-up — oversized blocks,
  more than 128 metered channels, more than 32 rack stages — which is the only thing that can close the completeness
  question all three passes record about themselves. `LIMIT-0004` is now the third argument for it.
- **Implementation revision.** Documentation only; no code changed. The `LIMIT-0004` finding is recorded, not fixed.

**P00A-T003 — Capture V1 CPU, memory, timing, and determinism baselines.**

- **Scope.** Measured V1 figures for the reference corpus at common polyphony and
  sample rates, in a reviewable format.
- **State.** Complete over the four corpus categories that exist. Determinism and
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
| P00A-T001 | [Corpus manifest](../../../corpus/v2-reference/manifest.json) and four generated fixtures; four of eleven categories covered, seven recorded as gaps | The manifest is loaded and validated by `cargo test -p pertylizer --test corpus_manifest`: category coverage, claim classes, digests, and the fact that each committed project is exactly what its builder produces | Partial — format and validation done, four of eleven categories exercised |
| P00A-T002 | `pertylizer compare` and the versioned report model                                   | Each metric unit-tested against a synthetic signal with a known deviation (6.02 dB of gain, 100 ms of delay, a semitone of detune, a band-limited change, an inverted channel); end-to-end through render→render→compare in `compare_command.rs` | Complete — runs with no GUI and no audio device |
| P00A-T003 | [EVD-0001](../evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md), [EVD-0002](../evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md), and [EVD-0003](../evidence/phase-00a/EVD-0003-cpu-memory-timing-baseline.md) | Two process-separate renders per case, bit-identical on all four, every comparison delta exactly zero, with a two-case control that resolves their octave to 3.6 cents; 8 640 timed renders giving the cost-versus-block-size curve; and 1 700 renders over four sample rates, seven voice counts, and 15 timing/memory profiles per operating point | Complete for the four categories that exist — CPU, memory, timing, and determinism all measured, with real-time headroom explicitly out of scope |
| P00A-T004 | [Resource inventory](../inventories/resource-limits.md) passes 1-2 at `dd69b657` and the classification pass at `b435887c` | Two independent discovery methods with opposite blind spots — pass 1 matched constant names, pass 2 matched documented truncation behavior — then a third pass applying accepted ADR-0021 to all 74 entries: terminal class, owner, rule, and diagnostic each citing the clause that supports it. Classifying found a sixth silent-truncation site the searches missed. Neither search executes anything, so a truncation that is both unnamed and undocumented would still be missed | Complete — every entry classified and disposed; no value measured, and the completeness probe remains ADR-0021 follow-up work |
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
