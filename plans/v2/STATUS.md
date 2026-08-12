# Pertylizer Core V2 Status

| Field                  | Value                                            |
|------------------------|--------------------------------------------------|
| Last updated           | 2026-08-12                                       |
| Documentation stage    | Workflow accepted; inventories at pass 2         |
| Master plan status     | Proposed and architecture-audited                |
| Active migration phase | 0A and 0B, both `Active` in parallel             |
| Decision records       | 3 of 37 drafted, 2 accepted                      |
| Evidence records       | 1 (`EVD-0001`), `Complete`                       |
| Executable Phase 0A    | Corpus and comparison command exist and are used |

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
- The decision topics are registered in [ADR.md](ADR.md), now 37 after one split. Three have individual records —
  [ADR-0001](decisions/ADR-0001-internal-render-quantum.md) (render quantum semantics),
  [ADR-0021](decisions/ADR-0021-host-profile-and-admission-policy.md) (host profile and admission), and
  [ADR-0037](decisions/ADR-0037-render-quantum-value.md) (quantum frame count). **ADR-0001 and ADR-0021 are `Accepted`**
  after three review passes; ADR-0037 remains `Proposed` pending measurement. ADR-0037 is a split of the master plan's
  topic 1, recorded as a deviation in the Phase 0A tracker; both quantum records are required `Accepted` at the Phase 0A
  exit, so the split does not weaken the gate.
- **The master plan is synchronized with ADR-0001 and ADR-0021.** Part VII topic 1 and the Phase 0A exit gate name both
  quantum records. The quantum is no longer configurable, and `maximum_block_size` is owned only by `HostProfile`
  instead of being duplicated in `RenderConfig`. These changes landed with the ADRs' acceptance, per the same-change
  rule in [README.md](README.md#sources-of-truth).
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
- **Building the corpus found a V1 defect, fixed on `fix/offline-render-fidelity`.** The offline renderer rebuilt instruments without an
  allocator config and replayed only volume, pan, and solo, so polyphony, allocation mode, transpose, key range,
  oversampling, velocity sensitivities, and the sidechain source were silently defaulted. It affected every consumer of
  the offline renderer, not only the corpus — the `analyze_*` tools and the WAV export measured audio the live engine
  never produced. It is the third instance of an offline reader disagreeing with the live engine while looking healthy.
- Phase 0A is **not** complete: P00A-T003 still has no CPU, memory, or timing figures, three of six required ADRs have no
  record, and no exit review exists. No code has been written for V2 itself.
- V2 implementation status must be established from repository evidence before this dashboard makes code-level
  completion claims.

## Next actions

The corpus and the comparison command now exist, so every measurement Phase 0A was waiting on can actually be run. The
inventories have stopped being search-limited; what each still lacks is either a decision or an executable check.

1. **Run ADR-0037's V1 proxy measurement.** Render the corpus at 32/64/128/256 frames by varying `BUFFER_SIZE` in
   `arrangement_render.rs`, record it as `EVD-0002`, and apply the record's ordered rule table. An inconclusive result is
   a real possible outcome. This is the last thing standing between ADR-0037 and acceptance, and it is now unblocked.
2. **Finish P00A-T003.** [EVD-0001](evidence/phase-00a/EVD-0001-corpus-determinism-baseline.md) covers determinism and
   level; CPU, memory, and timing at common polyphony and sample rates are still unmeasured, and the task does not close
   without them.
3. **Sweep all 74 resource-inventory entries** under accepted ADR-0021, assigning both axes — one of six failure classes
   and one of seven configuration owners — plus the rule and diagnostic. Every `Unknown`-class entry must reach a
   terminal class as part of it.
4. **Open ADR-0032** (`SampleTime` and event timestamps), the remaining record the Phase 0A exit gate requires
   `Accepted`. ADR-0001 now fixes the epoch and the late-event rule, so ADR-0032 refines the representation on top of
   that rather than inventing it.
5. **Open ADR-0014.** The identity ledger's central finding is that the module id encodes its type at *runtime*
   (`IDN-0029`), not merely on disk, and that a module's script PRNG seed is derived from its instance number — so
   renumbering is audible, not just referential.
6. **Write the first round-trip fixture (P00B-T005) for `STATE-0004`** — changing the focused instrument changes the
   saved file while no dirty term observes it. It is the cheapest executable check the ledgers produced.
7. **Add corpus cases as their blockers clear.** Instrument inserts need nothing and are the cheapest; the sampler case
   waits on the bundle round-trip fixtures, the shared-instrument case on ADR-0014, and the tempo-map case on whether a
   ramp's event positions fall under the sample-timing correction.
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
