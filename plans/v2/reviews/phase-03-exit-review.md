# REV-P03: Phase 3 Exit Review

| Field | Value |
|---|---|
| ID | REV-P03 |
| Status | Accepted |
| Phase | 3 |
| Created | 2026-09-01 |
| Last reviewed | 2026-09-01 |
| Reviewed source revision | Uncommitted exit transaction based on `8d6c9075` |
| Roadmap outcome | [Phase 3 — sample-accurate scheduling](../ROADMAP.md#phase-3--sample-accurate-scheduling) |

## Review scope

This review covers Phase 3's typed sample/plan time, compiled scheduling,
transport activation, tempo conversion, publication admission, simulated
renderer ingress, deterministic reference render, current sound/host contracts,
Phase 3 evidence, and the fail-closed loop boundary.

It excludes physical hardware-clock qualification, production live adapters,
runtime loop playback, producers that do not exist yet, full tuning/expression,
current-project lowering, and Phase 0B. Those exclusions are allowed only by
the bounded residual or later-phase gates below; none is claimed as delivered.

The reviewed transaction is bounded by its parent and its resulting commit:
the review and change land together, so a review file cannot name its own commit
hash in advance. The independent reader receives the complete staged and
unstaged diff plus all untracked paths.

## Approved gate amendments and inherited dispositions

The maintainer explicitly approved the process relaxations and the resulting
Phase 3 boundary changes on 2026-09-01. The exit therefore does not derive its
authority solely from the residual-exit rule added by the same transaction.
Two prior gate readings are narrowed and recorded here:

- The loop-transition gate requires declared same-sample ordering and an
  executable boundary response; it does not claim runtime loop playback.
  ADR-0055 makes the unsupported path fail closed, and P03-R001 blocks the
  first consumer that needs actual looping.
- The publication-cost gate is satisfied by publishing a synthetically full
  provisional partition within the callback budget. It does not qualify the
  provisional producer values. ADR-0054 supersedes the earlier Phase 3
  numeric-selection deadline and moves real-producer occupancy measurements to
  their first consumers plus the complete Phase 9 pre-live gate.

REV-P02's historical finding that `NoteEdge` carries neither pitch nor velocity
remains unchanged. This review supersedes only that row's Phase 3 owner and
"owed before ingress" scheduling disposition: deterministic simulated ingress
does not consume either magnitude, so P03-R003 blocks the first Phase 4 saved
pitched-note render instead. It likewise replaces ADR-0047's Phase 3
numeric-width evidence schedule with P03-R004's pre-live endurance gate. The
identity contract and every checked safety relation in ADR-0047 remain accepted
and unchanged.

## Required decisions

| ADR | Required status | Actual status | Result |
|---|---|---|---|
| ADR-0022 hardware time mapping | Deferred to Phase 9 exit; no physical timing claim in Phase 3 | Deferred | Pass |
| ADR-0023 same-sample ordering | Accepted | Accepted | Pass |
| ADR-0032 sample-time and timestamps | Accepted | Accepted | Pass |
| ADR-0043 preserving late clamp | Accepted | Accepted | Pass |
| ADR-0046 destination-quantum admission | Accepted, with an explicit successor for calibration timing | Accepted; timing amended by ADR-0054 | Pass |
| ADR-0047 note identity | Accepted | Accepted; P03-R003/R004 carry later consumer evidence | Pass |
| ADR-0049 tempo-ramp law | Accepted | Accepted | Pass |
| ADR-0050 transport activation | Accepted for the executable non-live scope | Accepted | Pass |
| ADR-0051 locate catch-up gate exception | Accepted | Accepted | Pass |
| ADR-0053 simulated ingress | Accepted | Accepted | Pass |
| ADR-0054 staged producer calibration | Accepted | Accepted | Pass |
| ADR-0055 unsupported-loop refusal | Accepted | Accepted | Pass |
| ADR-0052 sample-exact loop wrap and per-pass identity | May remain Proposed only while playback fails closed | Proposed; ADR-0055 refuses every loop-bearing offer | Pass |

## Inventory closure

| Inventory/scope | Unclassified entries | Evidence | Result |
|---|---:|---|---|
| Phase 3 host-profile event capacities and ingress stores | 0 in Phase 3 scope | `HOST-INV-007/009/021/022` and the renderer-ingress registry | Pass |
| Phase 3 sound/render invariants | 0 in Phase 3 scope | `SOUND-INV-016` through `SOUND-INV-020` conformance rows | Pass |
| Physical platform time sources | Outside Phase 3 | EVD-0016 and ADR-0022 assign qualification to Phase 9 | N/A |
| Repository-wide resource ledger | `LIMIT-0017` remains investigating outside this scope | ADR-0039 and Phase 10E own it | N/A |

## Exit gates

| Gate | Evidence or named tests | Result |
|---|---|---|
| An on-time note begins at its exact requested sample and a genuinely late note uses the preserving clamp | `compiled_schedule`; `render_contract::a_late_note_edge_takes_effect_at_its_clamped_render_position` | Pass |
| A note ending inside the same host block has the expected non-empty duration | `compiled_schedule` across all four declared host partitions | Pass |
| The offline stamp-window selector cannot omit or re-present an event across an uneven partition | `note_events::every_event_of_a_sorted_list_is_presented_across_an_uneven_partition` and its late-counter assertion | Pass |
| Tempo steps and ramps map through the anchor to stable sample positions | `session::a_step_and_a_ramp_reach_stable_engine_times_through_the_anchor` plus the `tempo` law suite | Pass |
| Equivalent simulated and precompiled events with the same `SampleTime` reach the same offsets | `simulated_ingress` equivalence render across four partitions | Pass |
| Session, transport, note, controller, automation and panic behavior has declared same-sample ordering | ADR-0023, ADR-0051 and `SOUND-INV-020`; unsupported loop playback is refused before adoption | Pass |
| Reference V2 output is invariant to equivalent host-block partitions | `reference_render` over `1 x 4096`, `16 x 256`, `64 x 64` and the declared irregular partition | Pass |
| Renderer ingress is bounded, producer shares satisfy checked relations, and overload cannot silently move or trim events | `admission`, `publication`, `simulated_ingress`, EVD-0019, ADR-0054's staged consumer gates | Pass |

The roadmap summary's bounded-exhaustion requirement is carried by named
refusal/fault paths: queue/hold acquisition refuses before accepting a live
obligation, invalid compiled production faults the stream, identity generation
retires rather than aliases, and unsupported loop playback refuses before
active state changes.

## Quality gates

The complete repository gate is required because this transaction changes CI,
an experimental Rust API, scheduling behavior, process rules and phase state.

| Command/check | Environment | Result | Evidence |
|---|---|---|---|
| `python3 -B scripts/check_v2_docs.py --evidence` | Local Linux | Pass | Documentation structure and EVD-0016 deterministic simulator passed |
| `python3 -B -m unittest scripts/test_check_v2_docs.py` | Local Linux | Pass | 35 tests |
| `cargo fmt --check` | Local Linux | Pass | No formatting diff |
| `cargo build --workspace` | Local Linux | Pass | Workspace build completed |
| `cargo clippy --workspace --all-targets` | Local Linux | Pass | No warning or error |
| `cargo test --workspace` | Local Linux | Pass | Workspace and doc tests completed without failure |
| `cargo test -p synth_engine --release resource_limit_probe_oversized_callback_exposes_build_mode_failure` | Local Linux | Pass | Release-only branch passed |
| `cargo doc --workspace --no-deps` | Local Linux | Pass | Workspace documentation completed |
| `cargo check --workspace --all-targets --no-default-features` | Local Linux | Pass | Configuration check completed |
| `cargo check --workspace --all-targets --all-features` | Local Linux | Pass | Configuration check completed |
| `cargo +1.98.0 check --workspace` | Local Linux, MSRV | Pass | Workspace MSRV check completed |
| Independent semantic review of the complete uncommitted transaction | Fresh read-only Claude Code invocations | Pass after repair | The branch and staged merge reads reported their stopping-rule defects; focused fresh rereads confirmed every repair |

## Deviations and residual risks

| Item | Impact | Owner/task | Acceptance basis |
|---|---|---|---|
| P03-R001: runtime loop playback and per-pass identity are not implemented | V2 cannot play loops | First V2 loop consumer; must close before Phase 9 exit | ADR-0055 refuses every loop-bearing offer before state changes |
| P03-R002: event shares, release holds and ingress depth are provisional | Current values are not production-live capacity claims | First real authored/internal producer and Phase 9 complete pre-live calibration | ADR-0054 keeps every safety relation and makes measurement a pull-forward gate |
| P03-R003: note payload lacks typed pitch and velocity | A saved pitched note cannot yet be rendered faithfully by Phase 4 | Before Phase 4's first saved pitched-note render | Pure lowering can proceed without choosing Phase 6's full tuning model |
| P03-R004: note index/generation widths lack live endurance evidence | Too-small widths can shorten stream lifetime, but cannot alias an identity | Phase 9 before production live ingress | Checked index bounds and generation retirement fail closed |
| Physical clock mapping is unqualified | No hardware-live timing claim | ADR-0022 and Phase 9 exit | Phase 3 consumes pre-mapped engine-epoch `SampleTime`; EVD-0016 preserves the method and limits |
| Activation cannot coexist with live ingress | Plan swap under a live producer is unavailable | Phase 9 live activation slice | `plan_activation` refuses once a live ingress store is adopted; ADR-0050 clause 8 owns the successor |
| A shared scalar gate has no two-producer ownership law | Mixed producers cannot yet drive the same gate through activation/catch-up | First such producer combination | ADR-0051 clause 6 keeps the combination outside the executable scope |

No residual permits silently accepted unsupported behavior. Each blocks its
first real consumer, and none is consumed by the selected pure Phase 4 lowering
slice.

## Independent semantic review

Authorship family: Codex. Reader family: Claude Code in separate fresh
read-only invocations. The branch review found five stopping-rule defects, all
reported before repair; its focused reread confirmed the repairs before the
status moved from `Draft` to `Accepted`. The staged merge review then found the
independence question plus historical-record, disposition, gate-amendment and
stale-specification defects repaired in this transaction. Its focused reread
closed those items and found the follow-up status, type-name, gate-split and
historical-pointer defects; the final focused reread confirmed those repairs
and found no remaining stopping-rule defect. No reader authored the transaction
or received shell, MCP, slash-command or delegation capability.

Stopping rule: a false claim, internal contradiction, unfillable contract,
safety/correctness defect, or evidence incapable of carrying a Phase 3 gate
blocks acceptance. Optional implementation detail does not.

## Outcome

Outcome: Accepted

Every Phase 3 exit gate is linked to executable evidence or a fail-closed
boundary. The complete repository gate passed, the independent semantic review
closed after repair, and P03-R001 through P03-R004 block their first consumers
without weakening a Phase 4 lowering guarantee. Phase 3 is complete.
