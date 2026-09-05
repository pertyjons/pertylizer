# REV-P05: Phase 5 Exit Review

| Field | Value |
|---|---|
| ID | REV-P05 |
| Status | Accepted |
| Phase | 5 |
| Created | 2026-09-05 |
| Last reviewed | 2026-09-05 |
| Reviewed source revision | `1b1252f4` on `main` — slices 1–9 merged and the plan correction committed |
| Roadmap outcome | [Phase 5 — declarative nodes and parameters](../ROADMAP.md#phase-5--declarative-nodes-and-parameters) |

## Review scope

This review covers the declarative node API in `synth_engine_v2` as Phase 5's nine slices
built it: `NodeDeclaration` as the single source for each of the ten node kinds — kernel,
ports, controls with unit, default, law and smoothing policy, in-place safety, note control,
taps, preparation and byte attribution; the discovery catalog derived from it; the parameter
slot that composes the base, override and modulation layers under a declared law and holds a
linear segment; the observation tap as a compiler artifact and the host's subscription over
it; and the four invariants those slices wrote, `SOUND-INV-022`, `SOUND-INV-023`,
`SOUND-INV-024` and `HOST-INV-023`, with the guard on `SOUND-INV-012`. It covers the two decisions the phase waited on — ADR-0006 and ADR-0007 — and
the correction of the phase plan made on 2026-09-05.

It excludes the subscription's live half — subscribing across a thread boundary while the
stream renders, decimation, the versioned facade, and the verification under a real host,
which `HOST-INV-023` assigns to Phase 9 — the declaration's latency, tail, reset and
execution-scope fields (no consumer in this phase reads one), the legacy adapter (withdrawn
by the plan correction, not deferred), and every frontend or protocol surface.

**Its first draft was rejected by an independent read**, which found the observation bullet
unevidenced: without a subscription surface, compilation having no observation input
establishes only the unobserved path. The user chose to build the surface over amending the
gate, and `P05-S009` did; this pass reviews the phase with it.
Its second draft was read again: the same bullet's semantic-digest clause rested on prose,
since the digest is Phase 10D's; the user chose to carry that clause as `P05-R002`, and this
pass reviews the gate as amended. Phase 0B's `P00B-T003` runs in parallel and is not part of this
phase.

## Required decisions

| ADR | Required status | Actual status | Result |
|---|---|---|---|
| ADR-0027 observation and analyzer ownership | Accepted before Phase 5 implementation begins | Accepted 2026-09-04 with the master plan's split ownership; `SOUND-INV-022` and `HOST-INV-023` written with it | Pass |
| ADR-0007 parameter modulation laws | Accepted before the slot composes | Accepted 2026-09-04, option 3, a closed law set composed centrally; `SOUND-INV-023` written with it | Pass |
| ADR-0006 parameter ramp representation | Accepted before the slot ramps | Accepted 2026-09-04, option 2, a linear segment per slot; `SOUND-INV-024` written with it | Pass |
| ADR-0005 buffer liveness strategy | Accepted, and its clause 6 honoured once a tap is a reader | Accepted in Phase 2; clause 6's case exists since `P05-S008` and is tested | Pass |
| ADR-0025 tuning representation and ownership | Accepted before Phase 6; not a Phase 5 gate | Accepted in Phase 4 | N/A — not this phase's prerequisite |

## Inventory closure

| Inventory/scope | Unclassified entries | Evidence | Result |
|---|---:|---|---|
| Decision register `ADR.md` | 0 | `scripts/check_v2_docs.py` validates the register; every ADR this phase touched is `Accepted` with its phase listed | Pass |
| Sound Core render contract conformance rows | 0 | `SOUND-INV-022`, `-023`, `-024` have filled rows; `SOUND-INV-012` has its guard row; no row reads "Not built" for an invariant this phase owns | Pass |
| Node kinds | 0 | Ten kinds, every one declared through `NodeDeclaration`; `a_declared_kind_appears_in_the_registry_only_by_deferring_to_its_declaration` scans every registry arm, `a_declared_kinds_registry_facts_derive_from_its_declaration` holds every kind's facts to its declaration, and the output node's absence is by rule | Pass |
| Phase 5 task table in `NOW.md`, archived at exit | 0 | Slices 1–9 merged (`51d3cf45`, `120a8e98`, `b7200890`, `36c132fd`, `b58602a5`, `d7d52375`, `8a7e81d6`, `f55f9422`, `ff5d47c6`, `1b1252f4`); the `SOUND-INV-012` guard row built in slice 1; the adapter row withdrawn by the plan correction; `P05-R001` carried as a residual with an owner; the table moved to `archive/phase-05/` | Pass |

## Exit gates

The gate as corrected on 2026-09-05 — [Gate correction](../master-plan.md#gate-correction-2026-09-05).

| Gate | Evidence or named tests | Result |
|---|---|---|
| A native simple module implements DSP without `set_param`, `get_param`, `get_params`, output hash maps, manual generic modulation storage, or engine-specific YAMS hooks | Every kernel is a free function over a prepared record, a state record and `NodeIo`; a quantum-rate control reaches it per frame from the slot's buffer (`ramp_of`) and a sample-positioned one as a `TimedControl`. `NodeState::set_control` was removed in `P05-S007b`; no kernel composes, by the scan `a_kernel_composes_nothing_and_the_law_is_applied_in_one_place`; `render_loop_purity` forbids `HashMap`, `BTreeMap` and every allocating construct in the region | Pass |
| The same declaration drives compiler validation and user-facing discovery | `node::catalog()` and the compiler's `descriptor` derive from one `NodeDeclaration`: `discovery_is_derived_from_the_declarations_admission_reads` (ids, names, ports, parameters with unit, default, law, smoothing, rate, magnitude; taps) and `discovery_and_validation_describe_the_same_ports` in `node_representation` | Pass |
| Automation and modulation combine identically for every native module | One slot type composes every parameter: `SOUND-INV-023`'s row — every write path reaches a kernel through `SlotState::write_override` (`parameter_slot_tests`), the law's arithmetic is defined once (`a_kernel_composes_nothing_and_the_law_is_applied_in_one_place`), and `a_declared_control_pairs_its_law_with_its_unit` holds every kind to the record's pairing | Pass |
| Stable targets survive node reorder and insertion | A target is a `(NodeId, ParameterId)` resolved once to a plan-scoped slot; the schedule and the arena are functions of identity, not declaration order (`the_schedule_is_a_function_of_identity_not_declaration_order`, `fan_out_order_is_identity_ordered_too`, `a_node_id_is_an_identity_and_not_a_position`); a slot from another plan is refused by identity (`a_slot_from_another_plan_is_refused_by_identity_rather_than_applied`); the lowerer's `reordering_the_modules_array_changes_no_assignment` and `extra_outputs_are_named_by_identity_not_array_order` hold the V1 side; note and tap slots follow the same shape (`NoteSlot`, `TapSlot`) | Pass |
| The same project compiles headless and with GUI/OSC observation enabled; observation changes no audible sample (the digest clause is carried as `P05-R002`, amended 2026-09-05) | Tested three ways since `P05-S009`: `observation_changes_no_sample_with_no_reader_one_reader_or_a_saturated_one` renders one compiled plan with no subscriber, one reader and one saturated reader and compares the output bit for bit; the plan is one object a subscription only reads, and compilation takes no observation input at all. A monitor is passive (`a_monitor_is_passive_and_its_tap_reads_the_signal_that_passed_through`), a tapped region stays live to the end of the quantum (`a_tapped_signal_stays_live_to_the_end_of_the_quantum`), the admitted tap count is the declarations' (`admission`'s `MaxObservationTaps` case), and the reader that keeps up reads exactly the rendered frames while the saturated one is told what it lost. The semantic project digest does not exist yet — Phase 10D defines it — so its clause has no falsifier here and is carried as `P05-R002` rather than claimed; an independent read of this review's second draft found the clause asserted on prose alone. GUI and OSC themselves reach V2 in Phase 10E; what they will subscribe through is this surface | Pass |
| No legacy adapter exists and the renderer needs none: a V1 module the lowerer cannot map to a native kind is refused under `LOWER` rather than adapted | No adapter type exists in the workspace; the lowerer's refusal diagnostics are Phase 4's `LOWER` contract, tested in `crates/pertylizer/src/lowering/tests.rs`; the bullet is the corrected one | Pass |

## Quality gates

Run on the tree the reviewed revision holds — the squash of `feat/v2-phase5-s009` at `ffa64b10`, whose diff against `1b1252f4` is empty — in this environment (Linux, stable toolchain plus Rust 1.98.0 for the MSRV check).

| Command/check | Environment | Result | Evidence |
|---|---|---|---|
| `python3 -B scripts/check_v2_docs.py --evidence` | — | Pass | doc structure, registers, EVD-0016 simulator |
| `python3 -B -m unittest scripts/test_check_v2_docs.py` | — | Pass | — |
| `cargo fmt --check` | — | Pass | — |
| `cargo build --workspace` | — | Pass | — |
| `cargo clippy --workspace --all-targets` | — | Pass | `build.warnings = deny` |
| `cargo test --workspace` | — | Pass | — |
| `cargo test -p synth_engine --release resource_limit_probe_oversized_callback_exposes_build_mode_failure` | release | Pass | — |
| `cargo doc --workspace --no-deps` | — | Pass | — |
| `cargo check --workspace --all-targets --no-default-features` | — | Pass | — |
| `cargo check --workspace --all-targets --all-features` | — | Pass | — |
| `cargo +1.98.0 check --workspace` | MSRV | Pass | — |
| EVD-0013 aligned V2 render digest | release | `4c0f4ce4…` reproduced after every slice | bit-identical through slices 7a, 7b, 8 and 9 |
| `quantum_cost` digests (voice-mono, voice-stereo, gain-chain) | release | `0fe495…`, `e954b9…`, `acf121…` reproduced after every slice | — |
| EVD-0009 and EVD-0010 cost harnesses | release | run to completion, arm checks passing | both had panicked at admission since `SOUND-INV-021`; fixed in `P05-S007b` |

## Deviations and residual risks

| Item | Impact | Owner/task | Acceptance basis |
|---|---|---|---|
| `P05-R002` — the observation bullet's semantic-digest clause is carried, not claimed | No digest exists to be changed or to misreport; ADR-0027 clause 2 keeps observation fields out of the serialized project | Phase 10D, which defines the semantic project digest | Amended by the user 2026-09-05 after an independent read found the clause unevidenced; the gate is rewritten not to claim it, and the digest test that will hold it is Phase 10D's |
| `P05-R001` — no declared `Smoothing` policy is anything but `None` | A write is a step. For the V2 amplitude that is V1 parity: the lowerer maps V1's oscillator level there and V1 applies it unsmoothed. V1's per-block de-zipper is on its amplifier level, which the lowerer refuses unless unity | The first lowering that maps V1's amplifier level onto a V2 parameter, or first writes a V2 amplitude dynamically | Decided by the user 2026-09-05; fails closed — nothing is silently ramped; the mechanism is built and mutation-verified; the trigger was corrected by an independent read |
| `HOST-INV-023`'s live half is not verified: subscribing across a thread boundary while rendering, decimation, the facade, saturation and staleness under a real host | The subscription exists and is tested single-threaded, between render calls | Phase 9's live host; Phase 10E's facade | The invariant assigns the live verification to Phase 9 by name; the reachable half is built and its loss exposed at the API (`HOST-INV-019`) |
| The declaration's latency, tail, reset and execution-scope fields are not built | A kind cannot yet state a latency or a tail | The first consumer that reads one | No consumer in this phase; the master plan's "single source" clause binds the field's first reader |
| The legacy adapter is withdrawn, not built | None: `LOWER` refuses what it cannot map | — | Plan correction of 2026-09-05 by the user; not a residual |
| The modulation sum has no producer | The modulation layer is written only by a test seam | Phase 7's modulation edges | `SOUND-INV-023`'s row; the seam is `#[cfg(test)]` and comes off with the first modulator |
| Every slice was merged on one independent read, without a second read of the squash, over nine slices | A defect only a squash-level read would see could have passed | — | The user's standing decision for these slices, recorded in each merge commit; Phase 4's squash read found harness fixtures the phase reads had missed, and `P05-S007b`'s harness repair is the analogue found by running rather than reading |

## What this phase did deliver

Ten node kinds declared once; a discovery catalog and a compiler descriptor derived from the
same declarations; a `ModulationLaw` per parameter from ADR-0007's closed set and a
`Smoothing` policy per parameter from ADR-0006's shape; a parameter slot that composes every
write — quantum-rate, sample-positioned, a note's gate and magnitudes, an adoption's
gate-downs, a catch-up — under the law and advances a linear segment per quantum into a buffer
the kernel reads per frame; node state without a quantum-rate control; a `Monitor` kind whose
declared tap is the plan's only tap, pinned in the arena as ADR-0005 clause 6 requires; and
resource accounting for the slots, their buffers and their index tables; and the host's
bounded, lossy subscription over a declared tap, pushed by the renderer after each quantum and
invisible to the plan and the audio. Every slice rendered bit-identically to the one before it.

## Outcome

Outcome: Accepted

Every bullet of the gate as corrected and amended on 2026-09-05 passes on named tests, and
every quality gate the repository requires passes on the reviewed revision. The phase exits
with two named residuals, `P05-R001` and `P05-R002`, each failing closed with a named owner; the other rows in the
deviations table are owed fields with no consumer in this phase and a plan correction the user
authorised, not residuals. Phase 6 depends on this phase's declaration and slot, and neither is
weakened: a voice pool will address a voice through the same declared parameters and taps, and
the tuning contract Phase 6 needs is ADR-0025's, already accepted.
