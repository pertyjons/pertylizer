# REV-P07: Phase 7 Exit Review

| Field | Value |
|---|---|
| ID | REV-P07 |
| Status | Accepted |
| Phase | 7 |
| Created | 2026-09-09 |
| Last reviewed | 2026-09-09 |
| Reviewed source revision | `a2195b67` on `main` — the eighth slice's merge (`P07-S007`; the slices are `S001`, `S002a`, `S002b`, `S003`–`S008`) — plus the ninth slice, `P07-S008`, whose tests and this review are committed together as the squash that adds this file |
| Roadmap outcome | [Phase 7 — YAMS and modulation](../ROADMAP.md#phase-7--yams-and-modulation) |

## Review scope

This review covers modulation, automation, controllers and the three YAMS domains in
`synth_engine_v2` as Phase 7's nine slices built them: a modulation edge from a control-domain
output into a declared parameter's modulation layer, with the `Lfo` as the first native source
(`SOUND-INV-027`); the filter's and the envelope's fields as quantum-rate controls under their
laws; a placed pattern's instrument automation lowered onto the override layer as V1 emits it,
with two absolute writers on one target at one sample refused by name; V1's per-module Mod
Matrix slots and the global Mod Grid instances V1's own builder returns lowered onto that edge at
V1's per-target scale (`LOWER-INV-004`); declared controllers and occurrence-scoped expression as
sources through the existing event capacities; YAMS Control as a scheduled node kind with stable
local parameter keys and identity-seeded state (`SOUND-INV-028`, ADR-0008); YAMS Audio as a
per-sample kind, bounded and allocation-free at the profile's maxima (`SOUND-INV-029`); YAMS Note
as one finite, exclusively owned authored source through the shared publication arbiter
(`SOUND-INV-030`, ADR-0060), measured under ADR-0054 clause 2 by EVD-0020 before its consumer was
enabled; and the exit's own evidence — one target driven by a Mod Matrix edge, a Mod Grid edge
and a script, held to the documented order, and a render held to a fixed project seed on every
path, which closes `P06-R001`. It covers the decisions the phase waited on — ADR-0007's laws,
ADR-0008, ADR-0054 clause 2 and ADR-0060 — and ADR-0012's continued deferral.

It excludes V2 in the shipping application; live reload and program replacement (Phase 9's
plan-swap decisions, ADR-0009 and ADR-0010); saved Note Grid graphs and note-processor racks,
refused by name and assigned to Phase 10A as `P07-R001`; the saved sources, targets and lane
classes `P07-S003` and `P07-S002b` refuse by name (an LFO's rate or depth, envelope and macro
sources, scripted slots, random shapes, tempo-synced LFOs, the grid's cheap sources and its
track, master and channel targets); a saved YAMS script, still reported as unrepresented by the
lowerer; live or compiled notes coexisting with an authored Note stream, stealing, activation or
runtime offers into it; the full six-share reselection of ADR-0054 clause 3 (Phase 9); a product
conflict policy for automation (ADR-0012, Phase 10); and every frontend or protocol surface. The
saved-project lowering this phase adds is `P07-S003`'s explicit subset, each edge marked for its
timing (`CORPUS-0003-C1`); this review claims no complete V1 migration or fidelity. Phase 0B's
`P00B-T003` runs in parallel and is not part of this phase.

## Required decisions

| ADR | Required status | Actual status | Result |
|---|---|---|---|
| ADR-0007 parameter modulation laws | Accepted before a modulator composes | Accepted in Phase 5; every Phase 7 contribution — edge, lane, controller, script — enters the target's one slot under its one declared law, and no law outside the closed set was needed, so the revisit condition did not fire | Pass |
| ADR-0008 YAMS state identity and seeds | Accepted before the first slice that gives a node a seed | Accepted 2026-09-09 for identity, stable local keys and the project/node/instance/state seed, built by `P07-S005`; the hot-reload clauses stay Phase 9's plan-swap obligation and no reload operation is enabled | Pass |
| ADR-0012 automation conflict policy | Not a Phase 7 gate; may stay `Proposed` | `Proposed`, Phase 10. This phase refuses two absolute writers on one target at one sample at compilation, by name (`two_writers_on_one_target_at_one_sample_are_refused_by_name`, `two_writers_landing_on_one_sample_from_disjoint_ticks_are_refused`); no product policy is selected | Pass |
| ADR-0054 staged producer-capacity calibration, clause 2 | The first real authored-runtime producer measured, its share reselected or retained before a consumer enables it | Accepted; EVD-0020 measured the committed subject `d632f874` and retained 48 authored events per quantum for at most 24 total inputs, and the qualification flag was enabled only after that run on the same producer and harness bytes. Clause 3's simultaneous six-share reselection remains Phase 9's | Pass |
| ADR-0060 bounded Note YAMS ownership and publication | Accepted before the Note slice builds | Accepted 2026-09-09 as a successor to the older accepted records it refines: one exclusive finite authored source, exact provenance, a stable invocation seed, bounded reservations and non-dropping storage registered apart from live ingress | Pass |

## Inventory closure

| Inventory/scope | Unclassified entries | Evidence | Result |
|---|---:|---|---|
| Decision register `ADR.md` | 0 | `scripts/check_v2_docs.py` validates the register; ADR-0008 and ADR-0060 are `Accepted` with their phases listed, ADR-0012 `Proposed` for Phase 10 | Pass |
| Sound Core render contract conformance rows | 0 | `SOUND-INV-023`'s row carries the modulation layer's producer and the controller layer; `SOUND-INV-027` through `SOUND-INV-030` have filled rows naming executable tests; no row this phase owns reads "Not built" | Pass |
| Node kinds | 0 | Twenty-four kinds discoverable through the catalog, the output node apart, every one declared through `NodeDeclaration`: the `Lfo`, eight controller and note-source kinds and the three script kinds joined Phase 6's; `node_representation` holds the count and the registry scan holds every kind | Pass |
| Saved authoring routes | 0 | `P07-S002b`, `P07-S003` and `P07-S004` lowering diagnostics separate represented modulation and macros, timing corrections and refused routes by name; a saved YAMS script is reported unrepresented; `P07-R001` assigns saved Note Grid graphs and racks to Phase 10A's named work item | Pass |
| Phase 7 task table in `NOW.md`, archived at exit | 0 | Nine slices merged — `S001` `aeb41092`, `S002a` `eb679358`, `S002b` `5d453fa4`, `S003` `7eda9406`, `S004` `d7ef68fb`, `S005` `852fef1c`, `S006` `d871d70d`, `S007` `a2195b67`, `S008` with this review; the table moved to `archive/phase-07/` | Pass |

## Exit gates

The six bullets of the master plan's gate, unchanged; no correction was needed.

| Gate | Evidence or named tests | Result |
|---|---|---|
| A time-varying Control YAMS program produces sample-identical output under multiple host-callback partitions; a constant-output fixture alone is not cadence evidence | `tests/scripts.rs`: `time_varying_control_and_local_automation_are_partition_invariant` — a stateful quantum counter whose output changes every quantum, with a local parameter automated under it, rendered under whole, one-frame and irregular partitions and held to the first by bits; `previous_resolved_reads_last_quantum_before_future_boundary_writes` for the feedback source's cadence. Under seeds: `random_program_seed_survives_recompile_and_declaration_order` and, in `tests/determinism.rs`, `fixed_project_seed_survives_stealing_recompile_partitions_offline_and_live` (below) | Pass |
| Audio YAMS remains bounded and allocation-free at maximum configured block size and voice count | `render_allocation`'s `yams_audio_allocates_nothing_at_the_profiles_voice_and_block_maximum` arms the counting allocator before the first render of a stereo program at both configured maxima; `tests/audio_scripts.rs`' `audio_work_uses_actual_cadence_and_the_configured_voice_ceiling` holds the per-quantum instruction work and the maximum-polyphony workload advisory to independently counted figures; `render_loop_purity` scans fourteen regions, the VM's evaluator and bytecode among them, for anything the audio thread may not do | Pass |
| A missing, cyclic, or scope-invalid binding is rejected by the compiler with a source-level diagnostic | `tests/scripts.rs`: `binding_failures_name_the_authored_source` (a binding to a missing node, a cycle through a script source and a scope-invalid binding, each refused with the authored span) and `a_modulation_cycle_without_a_script_source_keeps_its_graph_diagnostic`; `tests/audio_scripts.rs`: `audio_signal_cycles_and_scope_errors_retain_the_source_span` and `output_and_input_shape_mismatches_are_refused`; the Note domain shares the compiler and `authored::tests` refuses its unsupported source and scope forms (`plain_stream_cannot_silently_skip_a_note_program`, `compiler_note_knobs_are_opt_in_and_keep_the_note_output_grammar`) | Pass |
| Automation of a YAMS local parameter uses the same parameter pipeline as a native module | `tests/scripts.rs`: `local_knobs_compose_automation_and_modulation_in_the_declared_range` and `explicit_base_and_automated_sources_select_distinct_layers` (a local knob is an ordinary declared control in the one slot type — base, override, modulation sum, law); `tests/audio_scripts.rs`: `audio_local_automation_uses_the_central_quantum_parameter_slot`; `authored::tests`: `note_context_captures_local_automation_and_signal_on_the_quantum_clock` and `parameter_bindings_initialize_from_base_then_capture_the_named_layer` | Pass |
| Removing or renaming a script parameter cannot silently retarget an automation lane | `tests/scripts.rs`: `local_parameter_identity_survives_reordering_and_never_retargets_renames` — a lane's target is the stable local key, reordering the declarations moves nothing, and a removed or renamed parameter leaves the lane orphaned with a diagnostic rather than bound to the parameter that took its position; `failed_compile_preserves_the_parameter_namespace` | Pass |
| Mod Matrix, Mod Grid, and YAMS contributions have one documented combine order | `pertylizer`'s `lowering::tests::phase7::matrix_grid_and_yams_share_one_sum_law_and_override_order` (run under `cfg(test)` and again under `--features v2-lowering`): a saved per-module Mod Matrix slot and a saved global Mod Grid target, both into the filter's cutoff, lower through their real routes into two edges — the grid's target as a global-scope source at `−0.5 × 48` semitones, the matrix slot as a voice-scope source at `0.7 × 48` — and a global Control YAMS program at one semitone joins the same slot; the render is held by bits, with and without an override write doubling the base at the sixteenth quantum, to the same graph without edges driven by explicit per-quantum writes of `base × 2^((matrix + grid + script) / 12)` — the sum in the law's units first, the law once, an override replacing the base and leaving the sum in force, as `SOUND-INV-023` and ADR-0007 state the order; removing any one contributor changes the sound. The oracle is V2's documented arithmetic, not V1's timing (`CORPUS-0003-C1`); the cutoff has no narrower clamp to test | Pass |

The roadmap's boundary — deterministic dependency ordering, explicit conflict diagnostics, and no
script compilation or allocation on the audio thread — is held by `signals_schedule_before_a_script_modulating_a_native_parameter`
and `the_pre_pass_holds_the_sources_and_every_composition_ahead_of_the_main_walk` (ordering),
the two-writer refusals and the source-level binding diagnostics (conflicts), and by compilation
living outside the hot files — in `synth_script` and the crate's `script.rs` and `authored.rs` — with the
first-render allocator tests and the fourteen-region purity scan over every hot file (no
compilation or allocation in the loop).

## Inherited seed obligation

`P06-R001` closes with `tests/determinism.rs`' `fixed_project_seed_survives_stealing_recompile_partitions_offline_and_live`.
Phase 6's two-voice pressure fixture is kept and seeded: a per-voice Control program's `rand`
modulates the sine's frequency at three semitones, and a per-voice Audio program's `rand` scales
the amplifier's output, so the seed reaches both the pitch and the samples of every voice. The
same six notes force three steals. For one project seed the render is the same bits under a fresh
compilation, whole, 256-frame, quantum, one-frame and irregular partitions, offline (the compiled
stream's one priming quantum apart, and to a fresh offline render), and two simulated live runs
with the same count of releases after a steal; a different seed changes actual samples. The seed
enters where ADR-0008 states it: the project seed, the node, the instance and the stable script
state identity, never plan order or revision (`random_program_seed_survives_recompile_and_declaration_order`).
The finite Note source holds its own seed under its exclusive ownership —
`seeded_notes_and_audio_are_partition_invariant` and `occurrence_seed_is_independent_of_same_time_input_order`
hold both the generated keys and times and the rendered sound, keyed by `AuthoredOccurrenceId`
rather than voice assignment or list order — and makes no claim across an activation.

## Quality gates

Run on the tree this review is committed with — the squash of `feat/p07-s008-phase-exit` — in
this environment (Linux, stable toolchain plus Rust 1.98.0 for the MSRV check).

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
| `cargo test -p pertylizer --features v2-lowering lowering::tests` | `v2-lowering` | Pass | 115 tests; the lowering module compiles under `cfg(test)` through the dev-dependency, so the workspace run executes them too, and this run holds them under the shipping feature as well |
| EVD-0013 aligned V2 render digest | release | `4c0f4ce4…` reproduced on `a2195b67`, the last render-path change of the phase | bit-identical through every slice that touched the render path |
| `quantum_cost` digests (voice-mono, voice-stereo, gain-chain) | release | `0fe495…`, `e954b9…`, `acf121…` reproduced on `a2195b67` | — |
| EVD-0020 authored Note capacity | release | 16 ordinary controls and the retained harness pass on subject `d632f874`, artifacts verified by SHA256 and `git bundle verify` | retained under `evidence/phase-07/artifacts/` |
| `P07-S008` mutations | debug | Four run and caught: the project seed dropped from the script identity's hash (the changed seed no longer changes the sound); each contribution doubled in the slot's sum; the override read after the law rather than replacing the base; the semitone law dividing by 24 — each fails the oracle's bit comparison; every file restored | — |

## Deviations and residual risks

| Item | Impact | Owner/task | Acceptance basis |
|---|---|---|---|
| `P07-R001` — saved Note Grid graphs and note-processor racks remain refused by name | Fails closed: the lowerer refuses them naming this residual; the finite standalone authored stream's previous-quantum inputs and explicit cuts are not a saved-rack fidelity claim | Phase 10A's canonical note-processing work item, which defines the model's ordering, timing, identity, source bounds and fidelity dispositions before any saved consumer, then tests saved inputs through the lowerer; Phase 10A cannot exit with the refusal unassigned | Recorded at `P07-S007` in `NOW.md` and the master plan's Phase 10A work item |
| ADR-0008's hot-edit clauses — program-only swap, interface change, compile-failure retention, incompatible-state policy — are not enabled | Programs are immutable once compiled; no reload path exists to misbehave | Phase 9's plan-swap decisions, ADR-0009 and ADR-0010, before any reload consumer | ADR-0008 accepted for identity and seeds only, its reload half deferred by name |
| `P06-R002` — a live note carried across a plan recompilation is refused by its table, not routed | Unchanged by this phase; a controller or expression update for a stale identity is refused as foreign | Phase 9's live host, with ADR-0050 clause 8 | Inherited and untouched |
| `P03-R002` / ADR-0054 clause 3 — the six producer shares remain provisional as a partition | EVD-0020 retains only the authored producer's share for its first consumer; the internal producer is unmeasured because none exists, and production live ingress stays refused | Phase 9 reselects all six shares together before a production live adapter; the internal producer measures when first built | ADR-0054 clause 2 satisfied for the one real producer; clause 3 named as Phase 9's |
| `P05-R001` — no declared `Smoothing` policy is anything but `None` | Untriggered: the combined target is the filter's cutoff and the seeded modulation targets the pitch; an Audio program multiplying a signal is not a write to an amplitude parameter | The first lowering that maps V1's amplifier level or first writes a V2 amplitude dynamically | Inherited and untouched |
| `P05-R002` — observation and the semantic project digest | Unchanged | Phase 10D | Inherited and untouched |
| The ordinary finite Note consumer's capacity is a single retained share, not a live-ingress qualification | The source is admitted against 48 events per quantum and 24 total inputs; a larger source is refused before playback; timing figures in EVD-0020 are local descriptive data, not a hardware claim | Phase 9, with ADR-0054 clause 3 | EVD-0020's limitations section |
| `S001`–`S003` were merged on one independent read each without a read of the squash; the squashes of `S004`–`S007` were read whole | A defect only a squash-level read would see could have passed for `S001`–`S003` | — | The user's standing decision for these slices, recorded in each merge commit; codex read `S002b` and `S003`, Claude and agy read `S007`, agy the rest, each a different model family from the author as the rule requires |

## What this phase did deliver

A modulation edge that feeds a parameter's modulation layer from any control-domain output,
summed in the law's units and composed once per quantum, with the LFO as the first source; the
filter's corner and quality and the envelope's times as addressable controls; a placed pattern's
instrument automation as override writes at V1's own emissions, with simultaneous absolute
writers refused by name; V1's Mod Matrix and Mod Grid lowered onto that edge at V1's scale, and
its six macros onto declared controllers; per-note expression and controllers as sources through
the existing event capacities; three YAMS domains on one off-thread compiler — Control as a
scheduled node with stable local keys and identity-seeded state, Audio per sample and
allocation-free at the profile's maxima, Note as a finite exclusively owned source measured
before it was enabled; and the exit's evidence that every contributor meets its target in one
slot under one law, and that a render is a function of its event stream and its project seed
alone. The evidence digests reproduced after every slice that touched the render path.

## Outcome

Outcome: Accepted

Every bullet of the master plan's gate passes on named tests, unchanged from its written form,
and every quality gate the repository requires passes on the reviewed tree, with the
`v2-lowering` run and the four evidence digests beside them. The phase exits with one named
residual, `P07-R001`, failing closed with a named owner; the other rows in the deviations table
are deferrals by name with a named first consumer, inherited residuals this phase did not
reach, the retained single-share capacity, and the user's standing merge decision. `P06-R001`
closes here. One independent read of this review's draft, the two tests and the exit records
(agy, `gemini-3.8-flash-high`) found no defects; the four S008 mutations were run and caught
before it. Phase 8 remains `Not started` and unselected when this phase closes;
its activation is the user's selection. Phase 8 depends on this phase's one-slot composition
and on nothing here that is weakened: a bus or a send parameter is a declared control in the
same slot type, addressed by the same edge, lane, controller and script producers.
