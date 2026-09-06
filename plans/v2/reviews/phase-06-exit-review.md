# REV-P06: Phase 6 Exit Review

| Field | Value |
|---|---|
| ID | REV-P06 |
| Status | Accepted |
| Phase | 6 |
| Created | 2026-09-06 |
| Last reviewed | 2026-09-06 |
| Reviewed source revision | `e7e2d0a9` on `main` — the eighth and last slice's merge (`P06-S007`; the slices are `S001`, `S002`, `S002b`, `S003`–`S007`); the gate correction is committed with this review |
| Roadmap outcome | [Phase 6 — polyphony and instruments](../ROADMAP.md#phase-6--polyphony-and-instruments) |

## Review scope

This review covers the voice runtime in `synth_engine_v2` as Phase 6's eight slices built
it: the voice scope instantiated once per identity index of the producer that plays it, one
prepared shape and `N` instances of state (`SOUND-INV-025`); voice stealing under ADR-0058's
decided policies, on the compiled path and at the live boundary, with the taken voice faded,
reset and the new note started at a precise displaced sample; per-note expression as the
bend clause of `SOUND-INV-021`, routed by occurrence after a steal; velocity composed as V1
composes it, under ADR-0059, which closed Phase 4's residual `P04-R001`; the one-zone sampler
on ADR-0026's prepared map and zone contract (`SOUND-INV-026`), started by a declared trigger
destination rather than as a second playable node; one prepared tuning held to the live,
sequenced, offline and analysis-facing paths; and determinism under stealing pressure across
those paths and every host partition. It covers the three decisions the phase waited on —
ADR-0058, ADR-0059 and ADR-0026 — and the correction of the phase gate made on 2026-09-06.

It excludes mono, legato and unison allocation modes and note priority (a later record, when
a lowering reaches them), the lowering of a saved sampler module (it waits for the first
sampler corpus case, which Phase 0B's bundle fixture owns), per-project tuning selection
(Phase 10A's authored model), a project seed (Phase 7's ADR-0008), a live note carried across
a plan recompilation (ADR-0050 clause 8 keeps live ingress out of an activation's scope),
resampling of a sample recorded at another rate (refused by name until a consumer decides the
ratio), and every frontend or protocol surface. Phase 0B's `P00B-T003` runs in parallel and is
not part of this phase.

## Required decisions

| ADR | Required status | Actual status | Result |
|---|---|---|---|
| ADR-0025 tuning representation and ownership | Accepted before Phase 6 implementation begins | Accepted in Phase 4; a note-on names a key and the plan resolves it through a prepared tuning per scope | Pass |
| ADR-0058 voice allocation and stealing | Accepted before the slice that steals | Accepted 2026-09-05 with the user's selection — `None`, `Oldest` and `SameNote`, V1's fade-then-start with the fade declared; built by `P06-S002` and `P06-S002b` | Pass |
| ADR-0059 velocity composition | Accepted before `P06-S004` builds | Accepted 2026-09-06 with the user's selection — two destinations, V1's formulas bit for bit; built by `P06-S004`, closing `P04-R001` | Pass |
| ADR-0026 minimum sample map and zone model | Accepted before the sampler slice | Accepted 2026-09-06 with the user's selection — the master plan's split with a plan-side prepared table, a trigger destination carrying both edges, V1's playback law forward only; built by `P06-S005` | Pass |
| ADR-0047 note identity in the event contract | Accepted; its clause 9's reserved per-note event built by the expression slice | Accepted in Phase 3; the bend is that event, built by `P06-S003` | Pass |
| ADR-0008 YAMS state identity and reload policy | Not a Phase 6 gate; owns what a project seed is | `Proposed`, Phase 7 | N/A — the seed clause is carried as `P06-R001` |

## Inventory closure

| Inventory/scope | Unclassified entries | Evidence | Result |
|---|---:|---|---|
| Decision register `ADR.md` | 0 | `scripts/check_v2_docs.py` validates the register; ADR-0058, ADR-0059 and ADR-0026 are `Accepted` with their phase listed | Pass |
| Sound Core render contract conformance rows | 0 | `SOUND-INV-025` and `SOUND-INV-026` have filled rows; `SOUND-INV-021`'s row carries the bend, composition and tuning-path clauses and `SOUND-INV-025`'s the determinism evidence; no row reads "Not built" for an invariant this phase owns | Pass |
| Node kinds | 0 | Twelve kinds, every one declared through `NodeDeclaration`; the sampler is the twelfth, and the registry scan and the declaration-facts test hold every kind as they held ten | Pass |
| Phase 6 task table in `NOW.md`, archived at exit | 0 | Eight slices merged — `S001` `7f9c5ed1`, `S002` `15adfbd3`, `S002b` `7d602dee`, `S003` `4b754619`, `S004` `faa5e535`, `S005` `ca0c66bf`, `S006` `e7134c53`, `S007` `e7e2d0a9`; the table moved to `archive/phase-06/` | Pass |

## Exit gates

The gate as corrected on 2026-09-06 — [Gate correction](../master-plan.md#gate-correction-2026-09-06).

| Gate | Evidence or named tests | Result |
|---|---|---|
| Polyphonic output is deterministic for a fixed event stream under stealing pressure (the project-seed clause is carried as `P06-R001`, amended 2026-09-06) | `tests/determinism.rs`: `a_polyphonic_render_under_stealing_pressure_is_bit_identical_run_to_run_and_across_partitions` (five overlapping notes on two voices under `Oldest`, three steals, rendered twice and under four host partitions, every render held to the first by bits with the same count of releases after a steal), `the_offline_render_under_stealing_pressure_is_the_compiled_streams_render` (one priming quantum apart, and to itself), `the_live_boundary_under_stealing_pressure_is_bit_identical_run_to_run_and_to_the_compiled_stream` (the same edges offered live, the boundary's count equal to the scheduler's). Three mutations caught, among them a sine's initial phase taken from a process-wide counter, which fails all three. Nothing in V2 consumes randomness and no kind carries a seed, so there is nothing a seed could vary until Phase 7 gives a node one | Pass |
| Voice stealing begins and completes at precise sample offsets | `voice_tests`: `a_full_producer_takes_the_oldest_voice_fades_it_and_starts_the_new_note_when_the_fade_ends` (a note-on at `4Q + 5` takes the oldest voice; the oracle is three single-note renders combined sample for sample, with the fade from the note's own sample and the start displaced by exactly the declared fade), `a_same_note_policy_retriggers_the_held_key_at_once_and_without_a_fade`, `the_taken_voice_is_reset_so_the_new_note_attacks_from_silence`, `a_note_shorter_than_the_fade_still_starts_and_ends_on_the_voice_it_took`; at the live boundary `simulated_ingress`' `a_live_note_on_into_a_full_producer_steals_as_the_compiled_one_does`, `a_live_release_offered_while_the_start_is_pending_is_displaced_with_it` and `a_voice_taken_twice_before_its_first_victims_release_counts_every_release_and_leaks_no_hold`, each the compiled render by bits; `transport_activation`'s `a_loops_repeating_pass_charges_a_steals_expansion_where_it_lands` | Pass |
| Increasing configured polyphony changes prepared memory, not audio-thread allocation behavior | `voice_tests`' `prepared_data_is_shared_and_state_is_per_instance` and `the_voice_count_is_admitted_as_derived_from_the_producers` (the report's mutable and prepared rows scale with `N` for state and slots and not at all for prepared records, and preparation allocates exactly what the rows charge); `the_preflight_arena_bound_covers_a_voiced_plans_exact_arena`; `a_quantum_full_of_fanned_out_writes_has_room_in_the_control_scratch`; `render_allocation`'s counting allocator holds every render and every event pass at zero allocations (`the_first_render_after_preparation_allocates_nothing`, `repeated_renders_at_varying_block_sizes_allocate_nothing`, `resolving_and_applying_events_allocates_nothing`); `render_loop_purity` scans the loop, the kernels and the ingress store for anything the audio thread may not do | Pass |
| Shared immutable plan data is not cloned once per voice | `prepared_data_is_shared_and_state_is_per_instance` (`N` node slots per voice-scope node over **one** prepared record); a prepared sample's frames sit behind one `Arc` per plan (`SOUND-INV-026`'s row, `equal_samples_are_held_once_and_the_charge_is_what_the_plan_holds`); a prepared tuning is one table per distinct scale however many nodes reference it (`SOUND-INV-021`'s row) | Pass |
| A native per-voice module has no knowledge of the voice allocator | Every kernel is a free function over its prepared record, its state and `NodeIo`; the renderer routes a note's gate and magnitudes to the instance its identity index names (`voice_row` in `render/hot.rs`) and the kernel sees only controls due in its quantum — `each_note_lands_on_its_own_instance_and_the_rest_stay_at_rest`; a steal reaches a kernel as the loop-reserved `RESET` and `FADE_OUT` controls, which no declaration may name (`every_declared_control_is_below_the_reserved_floor`); the sampler, the one kind built after the allocator, declares three destinations and no note control and knows nothing of which instance it is (`SOUND-INV-026`'s row) | Pass |
| Built-in 12-TET and at least one non-12-TET/Scala mapping produce the same pitches through live, sequenced, offline, and analysis-facing paths | `tests/tuning_paths.rs` under nineteen-tone equal temperament: `the_sequenced_offline_and_live_paths_render_one_key_the_same_under_nineteen_tet` (the compiled stream, the offline render and the live boundary render one key as the same bits, differing from twelve-tone, and two keys resolve to two frequencies at the plan), `the_observation_tap_reads_the_pitch_the_output_carries_under_nineteen_tet` (the analysis-facing path: the tap holds the output's samples), `a_locate_restores_the_pitch_through_the_tuning_the_plan_states`; `simulated_ingress`' `a_live_note_under_nineteen_tet_reaches_the_same_samples_as_the_compiled_one`; `sampler_tests`' `the_rate_is_the_scopes_tunings_ratio_under_a_non_twelve_tone_scale`; and `tuning_tests`' `the_twelve_tone_table_is_v1s_formula_bit_for_bit`. A Scala mapping is not a separate path: `synth_core::TuningTable::from_scala` produces the same table type nineteen-tone does, and the lowerer states twelve-tone for every saved project because that is all V1 plays | Pass |
| Note identity remains sufficient to route per-note expression after voice stealing; across a plan recompilation a stale identity is refused by its table rather than routed (the live-note-across-recompilation clause is carried as `P06-R002`, and "polyphonic pressure" is named as a later payload of the same reserved event, amended 2026-09-06) | `voice_tests`: `a_bend_of_a_note_that_took_a_voice_is_displaced_with_its_start`, `a_bend_for_a_note_a_steal_ended_is_dropped_and_counted_with_its_release`, `a_bend_reaches_only_the_occurrence_it_names`, `a_new_occurrence_on_a_voice_starts_unbent`; `simulated_ingress`' `a_live_bend_of_a_note_whose_start_is_deferred_waits_with_it` and `a_live_bend_naming_a_note_the_producer_does_not_hold_is_refused_as_an_orphan`; `transport_activation`'s `a_bend_of_a_note_opened_before_the_anchor_is_omitted_and_counted`; `identity`'s `an_identity_from_another_table_is_not_an_orphan` (an identity minted against a table a recompilation replaced resolves as **foreign** to the new table — refused, and told apart from an orphan because the new table cannot say whether the note is live — never routed to another voice) and `note_identity`'s `stamping_uses_the_streams_own_epoch_and_a_foreign_renderer_refuses_the_schedule`. The routing is per occurrence and does not read the payload: the bend is the one payload of ADR-0047 clause 9's reserved event built; polyphonic pressure is a second payload of that event with no producer, and is owed to the first producer that emits one, not carried as a residual | Pass |
| A native one-zone sampler uses the prepared map/zone contract without per-note allocation or a special single-sample voice API | `SOUND-INV-026`'s row: `sampler_tests` holds V1's playback law by exact oracle (rate, interpolation, downmix, one-shot, sustain fade, loop, start offset, velocity), the one-zone refusals by name, the prepared table held once and charged once; one player state per instance is reset by the on edge and the kernel is in the purity scan's region; the sampler is a `SampleMap` consumer through `SampleMapRef`, with no API of its own for a single sample | Pass |

## Quality gates

Run on the tree the reviewed revision holds — the squash of `feat/v2-phase6-s007` — in this
environment (Linux, stable toolchain plus Rust 1.98.0 for the MSRV check).

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
| EVD-0013 aligned V2 render digest | release | `4c0f4ce4…` reproduced after every slice | bit-identical through all eight slices |
| `quantum_cost` digests (voice-mono, voice-stereo, gain-chain) | release | `0fe495…`, `e954b9…`, `acf121…` reproduced after every slice | — |
| Determinism under pressure | debug | `tests/determinism.rs` passes | run to run, four partitions, offline and live |

## Deviations and residual risks

| Item | Impact | Owner/task | Acceptance basis |
|---|---|---|---|
| `P06-R001` — the gate's "fixed project seed" is carried, not claimed | Fails closed: no node kind accepts or consumes a seed, so a render is a function of its event stream alone, held by bits in `tests/determinism.rs`; there is no seed to vary until Phase 7 binds one | Phase 7's ADR-0008, binding the first slice that gives a node a seed | Amended by the user 2026-09-06; the gate is rewritten not to claim the seed, and the event-stream half is held by bits on every path |
| `P06-R002` — a live note carried across a plan recompilation is not routed; a stale identity is refused by its table | Fails closed: an identity from a replaced table is an orphan by table, never another voice's note; the compiled path replays through activation, whose live half ADR-0050 clause 8 keeps out of scope | Phase 9's live host, with ADR-0050 clause 8 | Amended by the user 2026-09-06, put with this review's draft; the gate names the refusal rather than the routing. Fails closed: `an_identity_from_another_table_is_not_an_orphan` holds the foreign-table refusal |
| `P05-R001` — no declared `Smoothing` policy is anything but `None` | Unchanged by this phase; the sampler's level is a step as every quantum-rate control is | The first lowering that maps V1's amplifier level or writes a V2 amplitude dynamically | Inherited and untouched: no slice in this phase reached it |
| A sample recorded at a rate other than the stream's is refused by name | Refused rather than played mis-pitched; V1's speed formula reads no source rate either | The first consumer with a sample at another rate decides the ratio (ADR-0026's amendment at build) | Fails closed; recorded in ADR-0026 and `SOUND-INV-026` |
| The lowering of a saved sampler module is not built | `ModuleType::Sampler` is refused by the lowerer as before | The first sampler corpus case, owned by Phase 0B's bundle fixture; ADR-0026 clause 10 fixes the mapping | No sampler corpus case exists to measure against |
| The adoption's trigger gate-downs serve live notes only and are read, not measured | On the compiled path the catch-up's restore lowers a crossing sampler note's trigger, measured; the live-note case has no consumer | Phase 9, with ADR-0050 clause 8 | `SOUND-INV-026`'s row states the mechanism from combined mutations |
| Mono, legato and unison allocation modes and note priority are not decided | A note-on that finds no free voice steals or is refused under the decided policies; no mode routes notes otherwise | A later record, when a lowering reaches a saved `AllocationMode` | ADR-0058 names them as out of its boundary |
| Every slice was merged on one independent read, without a second read of the squash, over eight slices | A defect only a squash-level read would see could have passed | — | The user's standing decision for these slices, recorded in each merge commit; codex was out of quota for slices 2–7 and `agy` read them, a different model family from the author as the rule requires |

## What this phase did deliver

A voice scope that is one prepared shape and `N` instances of state, routed by the identity
index; stealing under three decided policies with a declared fade, on the compiled path and
at the live boundary, with the taken note's later release counted rather than mis-applied; a
bend as the reserved per-note event, layered under the modulation law and following the
occurrence through a steal; velocity applied as V1 applies it, twice with two sensitivities,
which closed Phase 4's parity marker on placed notes; a sampler kind that plays one zone of a
prepared map, held once per plan, started by a trigger the note's expansion writes; one
prepared tuning through every path; and a polyphonic render that is the same bits run to run,
under every host partition, offline and live. Every slice rendered bit-identically to the one
before it on the evidence digests.

## Outcome

Outcome: Accepted

Every bullet of the gate as corrected on 2026-09-06 passes on named tests, and every quality
gate the repository requires passes on the reviewed revision. The phase exits with two named
residuals, `P06-R001` and `P06-R002`, each failing closed with a named owner; the other rows in
the deviations table are refusals by name with a named first consumer, an inherited residual
this phase did not reach, and the user's standing merge decision. One independent read of this
review's draft (agy, `gemini-3.8-flash-high`; codex out of quota) found eight points, repaired
here: the draft status against the work list's; the seventh bullet's "polyphonic pressure"
dropped rather than dispositioned; the foreign-table test described as an orphan; "slices
1–7" for eight; the archive's recoverability claim; `P06-R001`'s fail-closed clause; "/Scala"
dropped from the sixth bullet; and stale blocker text in the work list. Phase 7 depends on this
phase's voice runtime and identity, and neither is weakened: a modulator addresses a voice
through the same declared parameters and slots, and the seed it needs is its own to define.
