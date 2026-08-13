# Pertylizer Core V2 Status

| Field                  | Value                                            |
|------------------------|--------------------------------------------------|
| Last updated           | 2026-08-13                                       |
| Documentation stage    | Workflow accepted; first specification at `Draft` |
| Master plan status     | Proposed and architecture-audited                |
| Active migration phase | 0A and 0B, both `Active` in parallel             |
| Decision records       | 6 of 37 drafted: 4 accepted, 2 deferred          |
| Evidence records       | 3 (`EVD-0001`..`EVD-0003`), all `Complete`       |
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
- The decision topics are registered in [ADR.md](ADR.md), now 37 after one split. Six have individual records, and
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
- All four [inventories](inventories/README.md) have completed two audit passes against `dd69b657` and are `Active`:
  74 `LIMIT`, 59 `STATE`, 55 `CAP`, and 31 `IDN` entries. Pass 1 was a census from schemas and constant names; pass 2
  read the enforcing code, which resolved every gate-blocking question and **disproved three pass-1 hypotheses** — those
  are corrected in place rather than appended. Each ledger records both methods and what each is blind to; none is
  `Current`.
- **The [resource ledger](inventories/resource-limits.md) has had a third pass, and P00A-T004 is `Complete`.** All 74
  entries carry a terminal failure class, one of ADR-0021's seven configuration owners, a proposed V2 rule, and a
  diagnostic; no entry is `Unknown`. The owner axis is where the interest is: **30 entries are `HostProfile` and 44 are
  not** — 12 node contracts, 7 protocol, 7 application settings, 7 removed outright, 6 domain/format, 5 job policy. The
  single-axis model ADR-0021 was first accepted with, and then had withdrawn, would have put those 44 in a
  render-preparation input they have nothing to do with.
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
  [`corpus/v2-reference/`](../../corpus/v2-reference/README.md) holds a validated manifest, a generator, and four
  fixture projects, but the master plan asks for eleven categories and four are covered. The other seven are recorded
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
- **The `HostProfile` contract is written and reviewed once, and it is the register's first specification.**
  [Host profile and render limits](specs/spec-host-profile-and-render-limits.md), invariant prefix `HOST`, is at
  `Draft` with 21 invariants, a conformance test named for each, and a value for every field the master plan lists plus
  ADR-0032's forward event horizon. Each of the ledger's 30 `HostProfile`-owned entries has a named successor field;
  seven fields have no V1 antecedent. **Exactly one value is derived from measurement** — ADR-0021 asked P00A-T005 for
  measured defaults and EVD-0003 measured cost, not capacity — with two more *chosen and anchored on* it. Every default
  carries one of those stated bases rather than implying a measurement the record does not have.
- **The independent review pass found five defects, two of them High, and all are corrected.** The first was a
  `maximum_block_size >= Q` clause that would have **refused hosts the render model is built for**: ADR-0001 clause 6
  primes the output carry precisely so that a callback of `N < Q` can be served, and the constraint had no purpose. The
  second was a runtime contract that permitted loss only at live bounded queues while three fields visibly did
  something else — the telemetry ring overwrote, a recording take stopped, and an over-full quantum had no defined
  behaviour at all, although the four ingress queues hold 3 072 events against a 256-event scratch. The specification
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
- **Four consecutive passes over a correction have each found something in the correction.** ADR-0001, ADR-0021 and
  ADR-0032 recorded one instance each; P00A-T005 has now produced two of its own. This has stopped being a warning
  about same-session acceptance and become a measured property of the work: **treat a correction as new material, not
  as a fix.**
- **EVD-0003's finding produced a field, not just a number.** A block costs the same absolute time at every sample rate
  while its real-time budget shrinks with the rate, so the profile carries an advisory cost budget expressed as a
  *ratio of times* — 0.15 of the quantum's real-time budget, from the observed 2.7x–6.8x max-to-median block spread.
  It is the only field in the profile that is not a capacity, and it is the reason a count-only profile would admit a
  512-voice plan identically at 44.1 kHz, where it costs about 60% of one measured core, and at 192 kHz, where the same
  plan is not real time at all.
- **The specification stays `Draft` for one step only.** The closure pass introduced three pieces of new semantics of
  its own — the capacity-deferral counter, the derived horizon floor, the derived retirement budget — and by the
  pattern above that is where the next defect lives. What it needs is a confirmation read of those three, not a fourth
  full pass. Its *Review status* section names them, alongside the standing checks and the two structural choices
  ADR-0021 left open: the `HostCapabilities`/`RenderLimits` split,
  and a `CapabilitySource` tag with two constructors, which is what makes the queried-capability rule enforceable by
  API shape instead of by a runtime tag that cannot prove a query happened.
- Phase 0A is **not** complete: P00A-T001 covers four of eleven corpus categories, P00A-T005 is `Active` with a
  once-reviewed draft, and no exit review exists. **Five of seven tasks are `Complete`**; the two open ones are corpus
  coverage and the `HostProfile` contract's closure pass. No code has been written for V2 itself.
- V2 implementation status must be established from repository evidence before this dashboard makes code-level
  completion claims.

## Next actions

Five of Phase 0A's seven tasks are `Complete`: the contracts, the measurements, and the limit audit. The sixth,
P00A-T005, has a contract that has now had three review passes. What is left is one confirmation read, corpus
coverage, and the exit review.

1. **Confirmation read of the three changes the closure pass introduced** in the
   [host profile specification](specs/spec-host-profile-and-render-limits.md) — the capacity-deferral counter, the
   derived forward-horizon floor, and the derived retirement budget — then promote it to `Current` and close
   P00A-T005. The target is specific because the pattern is: the closure pass predicted HOST-INV-021 would be where
   ADR-0001 clause 16 interacted badly, and it was, but the defect was the *counter and the position rule*, not the
   interaction anyone expected. The remaining question these corrections raised and did not answer is whether
   deferral can starve a low-priority event under sustained overrun; it is recorded as an open question rather than
   as a resolved one.
2. **Open ADR-0014.** The identity ledger's central finding is that the module id encodes its type at *runtime*
   (`IDN-0029`), not merely on disk, and that a module's script PRNG seed is derived from its instance number — so
   renumbering is audible, not just referential.
3. **Write the first round-trip fixture (P00B-T005) for `STATE-0004`** — changing the focused instrument changes the
   saved file while no dirty term observes it. It is the cheapest executable check the ledgers produced.
4. **Add corpus cases as their blockers clear.** Instrument inserts need nothing and are the cheapest; the sampler case
   waits on the bundle round-trip fixtures, the shared-instrument case on ADR-0014, and the tempo-map case on whether a
   ramp's event positions fall under the sample-timing correction — still open, since ADR-0032 fixed only that the
   conversion is rounded once and stays platform-independent, leaving the ramp law itself to Phase 3. EVD-0002 raised
   the stakes on this: the corpus's category mix, not only its size, now demonstrably moves a measured result.
5. **Record the inventory audit passes as `EVD` records** so the ledgers' claims are reproducible rather than
   asserted, and run ADR-0021's executable truncation probe — oversized blocks, more than 128 metered channels, more
   than 32 rack stages. `LIMIT-0004` is the third argument for it: three passes have now each found something the
   previous method could not see.

## Documentation-workflow review notes

Two conformance details were noted during the acceptance review and left as they are, because both trackers carry the
information the template asks for:

- Neither phase tracker uses the template's exact `Required decisions` column set. Phase 0A uses
  `Required at Phase 0A exit` / `Later acceptance gate`; Phase 0B uses `Topic` / `Earlier deadline, if any` and states
  the required status in prose above the table rather than in a column.
- The trackers' task tables use a `Primary record` column where the template has `ADRs/inventories`.

## Blockers

No documentation blocker is currently recorded. Open decisions become blockers only when a phase task or exit gate
requires them to be accepted.

## Phase overview

| Phase | Name                                     | Status      | Tracker                                                          | Exit review |
|-------|------------------------------------------|-------------|------------------------------------------------------------------|-------------|
| 0A    | Baseline, limits, and render contracts   | Active      | [Tracker](phases/phase-00a-baseline-and-render-contracts.md)     | Not created |
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
