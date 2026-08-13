# Pertylizer Core V2 Status

| Field                  | Value                                            |
|------------------------|--------------------------------------------------|
| Last updated           | 2026-08-13                                       |
| Documentation stage    | Workflow accepted; inventories at pass 2         |
| Master plan status     | Proposed and architecture-audited                |
| Active migration phase | 0A and 0B, both `Active` in parallel             |
| Decision records       | 4 of 37 drafted, 3 accepted                      |
| Evidence records       | 2 (`EVD-0001`, `EVD-0002`), both `Complete`      |
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
- The decision topics are registered in [ADR.md](ADR.md), now 37 after one split. Four have individual records, and
  **three are `Accepted`** — [ADR-0001](decisions/ADR-0001-internal-render-quantum.md) (render quantum semantics)
  and [ADR-0021](decisions/ADR-0021-host-profile-and-admission-policy.md) (host profile and admission) after three
  review passes, and [ADR-0037](decisions/ADR-0037-render-quantum-value.md) (quantum frame count) on the strength of
  [EVD-0002](evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md). ADR-0037 is a split of the master plan's
  topic 1, recorded as a deviation in the Phase 0A tracker; both quantum records are required `Accepted` at the Phase 0A
  exit, so the split does not weaken the gate.
- **[ADR-0032](decisions/ADR-0032-sample-time-and-event-timestamps.md) exists but is `Proposed` again, and it is the
  one decision still blocking the Phase 0A exit gate on its own.** It was accepted on a single author review pass; a
  second, independent pass withdrew that acceptance over five defects and the corrections now await a closure review.
  Two were substantive: the tempo map had been made to produce an engine `SampleTime`, which is not well defined
  because the render clock is monotone across seek and restarts at zero per offline render — musical time now
  converts to a `PlanPosition` that the session scheduler anchors to the epoch — and one of two required `HostProfile`
  horizons turned out to control nothing, because the backward direction is fully determined by ADR-0001's late-event
  rule. The other three: the pre-epoch clamp contradicted that same rule by counting every stream start as late,
  exhaustion had no defined behavior, and the epoch identifier could be reused as `A -> B -> A`.
- **The contract as it now stands.** `SampleTime(u64)` in the engine-input epoch, with `FrameCount`, `FrameDelta`,
  `PlanPosition`, `QuantumOffset(u16)`, and a strictly increasing `StreamEpoch`; musical time is rounded to a frame at
  exactly one point under a platform-independent law; every queued event carries `(epoch, time, provenance)` so that a
  late, stale, out-of-horizon, pre-epoch-clamped, or arrival-stamped event is counted rather than silent. The range
  analysis behind it is arithmetic, not measurement: `f32` stops being sample-exact after 349 s at 48 kHz, `u32` after
  6.2 h at 192 kHz, `u64` after three million years. Reading V1 to write it produced four findings — the cpal backend
  computes a `u64` stream position and a measured output latency the engine never reads, two offline paths build a
  `u64::MAX` sentinel for that unread field, live MIDI discards the driver's timestamp and lands on the next block
  boundary (up to 21.3 ms, then anchored to a tick by recording), and the plan's intended `SampleOffset` name is
  already an unrelated `f32` in `synth_core`. The last one renamed the V2 type to `QuantumOffset` in the master plan.
- **Twice now, a record accepted in the session that drafted it has had to have its acceptance withdrawn** — ADR-0001
  and ADR-0021 after three passes, ADR-0032 after one. Treat a same-session acceptance as provisional until an
  independent pass has run, whatever the gate pressure.
- **`Q` = 64 frames, provisionally.** The V1 proxy landed inside ADR-0037's own inconclusive band — `r(64,256)` is
  +9.9%, and whether that is within 5 pp of the 15% threshold is decided by the choice of estimator rather than by the
  data — so the record's rule 1 applied: accept 64, and make re-measuring it against real V2 nodes a **Phase 2
  exit-gate item**. Until that passes, nothing may tune against the value: no hand-unrolled kernel, no `Q`-specific
  buffer layout, no test asserting a control rate in Hz. The measurement also produced the first V2-relevant cost
  model this project has: per-block overhead is about 0.8 µs and scales with active voice and module count.
- **The master plan is synchronized with all three accepted records.** Part VII topic 1 and the Phase 0A exit gate name
  both quantum records; the quantum is no longer configurable, and `maximum_block_size` is owned only by `HostProfile`
  instead of being duplicated in `RenderConfig`. ADR-0037's acceptance added the re-measurement to the **Phase 2 exit
  gate**, because the plan — not the ADR — is authoritative for gates and a binding obligation recorded only in a
  decision record would not be enforced by one. Each change landed with the acceptance that caused it, per the
  same-change rule in [README.md](README.md#sources-of-truth).
- **The accepted contracts now unblock execution.** ADR-0001 fixes quantum, carry, end-of-stream, event-horizon, and
  latency semantics in terms of `Q`. ADR-0021 fixes admission behavior, seven configuration owners, an explicit lossy
  retention/presentation class, and a terminal `needs_reprepare` policy for an oversized host callback.
- All four [inventories](inventories/README.md) have completed two audit passes against `dd69b657` and are `Active`:
  74 `LIMIT`, 59 `STATE`, 55 `CAP`, and 31 `IDN` entries. Pass 1 was a census from schemas and constant names; pass 2
  read the enforcing code, which resolved every gate-blocking question and **disproved three pass-1 hypotheses** — those
  are corrected in place rather than appended. Each ledger records both methods and what each is blind to; none is
  `Current`.
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
- **Building the corpus found a V1 defect, fixed on `fix/offline-render-fidelity`.** The offline renderer rebuilt instruments without an
  allocator config and replayed only volume, pan, and solo, so polyphony, allocation mode, transpose, key range,
  oversampling, velocity sensitivities, and the sidechain source were silently defaulted. It affected every consumer of
  the offline renderer, not only the corpus — the `analyze_*` tools and the WAV export measured audio the live engine
  never produced. It is the third instance of an offline reader disagreeing with the live engine while looking healthy.
- Phase 0A is **not** complete: P00A-T003 has CPU figures at one operating point and still no memory or timing figures,
  ADR-0032 is back to `Proposed`, the two deferrable ADRs (ADR-0022, ADR-0028) have neither a record nor a written
  deferral, and no exit review exists. No code has been written for V2 itself.
- V2 implementation status must be established from repository evidence before this dashboard makes code-level
  completion claims.

## Next actions

One decision still gates Phase 0A on its own, and it is a closure review rather than an open question. Everything else
is measurement, coverage, and two written deferrals; the corpus, the comparison command, and a timing harness all
exist, so nothing below is blocked on tooling.

1. **Close ADR-0032's second review.** The five defects are corrected in the record; what is missing is an independent
   pass over the corrections and then re-acceptance. Read the corrected clauses first: 12 (epoch non-reuse), 15 and
   26-27 (plan position and anchoring), 21 (one forward horizon), 22 (pre-epoch clamp versus lateness), and 28
   (exhaustion). Do not accept it in the same pass that reviews it — that shortcut is what produced both withdrawals.
2. **Finish P00A-T003.** [EVD-0001](evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md) covers determinism and
   level and [EVD-0002](evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md) covers CPU at one polyphony and one
   sample rate; memory, timing, and CPU across common polyphony and sample rates are still unmeasured, and the task does
   not close without them. `render_cost` is the harness for the CPU half and takes a corpus directory, so widening it is
   a matter of cases and operating points rather than of tooling.
3. **Write the ADR-0022 and ADR-0028 deferrals.** They are the only required-decisions rows left, and the gate accepts
   a deferral with a named target gate, an owner, and the evidence still missing. Doing it as a written deferral rather
   than leaving the rows empty is what keeps the exit review from having to rediscover them; ADR-0022 also inherits
   concrete obligations from ADR-0032 (the epoch anchor's calibration, and the arrival-time uncertainty an
   untimestamped adapter must declare).
4. **Sweep all 74 resource-inventory entries** under accepted ADR-0021, assigning both axes — one of six failure classes
   and one of seven configuration owners — plus the rule and diagnostic. Every `Unknown`-class entry must reach a
   terminal class as part of it.
5. **Open ADR-0014.** The identity ledger's central finding is that the module id encodes its type at *runtime*
   (`IDN-0029`), not merely on disk, and that a module's script PRNG seed is derived from its instance number — so
   renumbering is audible, not just referential.
6. **Write the first round-trip fixture (P00B-T005) for `STATE-0004`** — changing the focused instrument changes the
   saved file while no dirty term observes it. It is the cheapest executable check the ledgers produced.
7. **Add corpus cases as their blockers clear.** Instrument inserts need nothing and are the cheapest; the sampler case
   waits on the bundle round-trip fixtures, the shared-instrument case on ADR-0014, and the tempo-map case on whether a
   ramp's event positions fall under the sample-timing correction — still open, since ADR-0032 fixed only that the
   conversion is rounded once and stays platform-independent, leaving the ramp law itself to Phase 3. EVD-0002 raised
   the stakes on this: the corpus's category mix, not only its size, now demonstrably moves a measured result.
8. **Record both audit passes as `EVD` records** so the ledgers' claims are reproducible rather than asserted. No value
   in the resource ledger has been measured; all of them are read from source.

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
