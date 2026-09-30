# REV-P09A: Phase 9A Engine Gate Review

| Field | Value |
|---|---|
| ID | REV-P09A |
| Status | Accepted |
| Phase | 9A |
| Created | 2026-09-30 |
| Last reviewed | 2026-09-30 |
| Reviewed source revision | The `phase-9a` commit that adds this review: 777bb4e0 plus the oversized-callback repair the review drove |
| Roadmap outcome | [Phase 9A engine gate](../master-plan.md#phase-9a-engine-gate) |

## Review scope

This review covers the twelve items of the Phase 9A engine gate added by
[ADR-0076](../decisions/ADR-0076-phase-9a-engine-gate-and-mixed-producer-deferral.md).
It is not a Phase 9 exit. ADR-0076 requires it to be reviewed like an exit
review. The evidence is deterministic simulated-host tests in `synth_engine_v2`,
the non-shipping Linux harness under `crates/pertylizer/examples/support/v2_input_host/`,
and the experimental application song playback of
[ADR-0077](../decisions/ADR-0077-experimental-v2-song-playback-in-the-application.md),
built only with the non-default `v2-lowering` feature under `crates/pertylizer/src/lowering/`.

The review makes no physical-device claim. It excludes several items, all of
which remain in Phase 9's exit gate:

- physical recovery, monitoring and latency;
- ADR-0022 acceptance;
- production timestamped live input;
- mixed producers;
- the GUI/MCP project revision.

Where an outcome ends in a project transaction, only the V2 side is claimed.

The stopping rule is a false behavioral claim, a contradiction, an unsupported
acceptance conclusion, or a contract hole required by Phase 11. Optional
implementation detail is not a gate.

## Required decisions and inventory closure

| Item | Actual state and evidence | Result |
|---|---|---|
| ADR-0076 engine gate and mixed-producer deferral | Accepted. ADR-0075 is `Deferred`, and its mixed-producer obligation blocks its first consumer and Phase 12 | Pass |
| ADR-0077 experimental song playback in the application | Accepted; the code lives under `src/lowering/` behind `v2-lowering`, and `crate_boundary::enabling_the_lowering_feature_adds_exactly_one_dependent` and `no_feature_of_any_crate_adds_a_second_dependent` hold the dependency edge | Pass |
| ADR-0022 hardware timing | `Deferred` to Phase 9's exit, which ADR-0076 permits. V2 mode disconnects MIDI before it is entered and consumes no host timestamp; no timestamp-capable adapter reaches V2 | Pass |
| ADR-0054 capacity | Accepted. [EVD-0025](../evidence/phase-09/EVD-0025-song-playback-partition.md) is `Complete` and reselects the compiled and session partition that song playback admits | Pass |
| Resource-limit inventory | `LIMIT-0021` cites the restored V1 state lines; no new unclassified limit. The application switch's rings are bounded by `SLOTS` and the callback by `MAX_CALLBACK_FRAMES` | Pass |
| Branch commits | `4cc3a9ea`, `85e474d4`, `609bb2ee`, `93790a4e`, `225235e4`, `eb840f0d`, `18791b01`, `e513584d`, `41fdc24d`, `730d6731`, `60d9afad`, `777bb4e0` and the commit adding this review, on `phase-9a` | Pass |

## Gate items

| # | Gate text (abridged) | Evidence or named automated checks | Result |
|---|---|---|---|
| 1 | V2 selectable live without changing save semantics; a mixed stream refused by name | `lowering::tests::app::v2_mode_takes_over_the_audio_with_its_own_transport_and_hands_back_to_v1`, `v2_mode_refuses_an_unlowerable_project_a_mono_device_and_an_unknown_stream` and `entering_v2_keeps_its_omissions_visible_until_it_ends`; V2 mode never writes project state. The mixed stream refusal is `IngressPrepareError::MixedProducerPlan`, held by `simulated_ingress::a_live_store_cannot_join_a_plan_that_also_declares_a_compiled_producer`. The GUI toggle in `gui/egui_backend/v2_flow.rs` is covered by the `--all-features` build, not a GUI test | Pass |
| 2 | A structural edit is absent or present as one active plan | Harness `swaps::a_structural_edit_sounds_as_one_whole_plan_and_is_acknowledged` changes the graph and sounds exactly as the new plan prepared alone. No saved-project structural edit is tested, as NOW.md records; the application's edit tests prove only the install path (one whole prepared session, installed only when stopped or paused: `an_edit_while_playing_applies_once_paused`, `play_waits_for_an_edit_it_cannot_install_yet`), not a changed topology | Pass, in the harness |
| 3 | A failed compile keeps the previous plan sounding | `simulated_ingress::continuous::failed_compile_and_refused_candidate_keep_the_active_plan_sounding` compares audio. In the application, a refused edit never plays the old session, and V2 mode is left instead: `a_refused_edit_never_plays_the_old_session`. That behavior was chosen for the application; it does not weaken the engine law | Pass |
| 4 | Plan swap, event delivery and parameter updates allocate nothing and take no locks | `tests::live_allocation::{live_ingress_delivery_and_parameter_updates_allocate_nothing, a_running_plan_swap_allocates_nothing_on_the_callback, a_prepared_plan_handoff_and_retirement_move_allocate_nothing}`; the `render_loop_purity` scan for locks; every application callback in `lowering::tests::app`, including the oversized one that faults, is measured with `allocation_counter`, and the switch never hands V1 a callback above the ceiling while V2 is active (V1 skips it). In V1 mode the device's callback reaches V1 unchanged, as on `main` | Pass |
| 5 | Disconnect, buffer-size and rate changes recover through the lifecycle or fail visibly | `host_lifecycle::{device_reconfiguration_quiesces_visibly_and_recovers_through_reprepare, oversize_fault_recovers_only_through_reprepare_with_a_larger_bound}`, `host::capture::tests::device_reconfiguration_interrupts_capture_as_reprepare_without_another_callback`; in the application an oversized callback reaches the session, takes its terminal fault under IO-INV-002, is silent, and is visibly prepared again at the song start (`an_oversized_callback_faults_v2_and_prepares_it_again`) | Pass |
| 6 | Independent clocks remain latency-bounded with visible counters and no hidden backlog | `audio_input::{slow_input_clock_with_stalled_worker_keeps_monitoring_in_window_and_capture_exact, drift_beyond_the_correction_bound_faults_visibly_without_hidden_backlog, monitoring_overflow_never_changes_original_pcm_or_timing}` | Pass |
| 7 | Simulated live events, monitoring, note and audio recording; no timestamp-capable adapter in production | `note_capture` (arm, count-in, pairing, late input, panic), `audio_input::stalled_worker_pool_exhaustion_and_storage_exhaustion_retain_exact_prefixes` for the immutable in-memory take, and `simulated_ingress` stamp and horizon tests. The persistent asset and commit are Phase 10 work; the gate text scopes them out | Pass |
| 8 | Count-in, metronome, loop recording, sustain, stop and panic order at boundaries | `ordered::boundaries::{panic_at_exact_loop_wrap_retains_the_take_and_closes_held_key_and_pedal, sustain_across_a_wrap_keeps_key_pairing_and_pass_ownership, a_pedal_pressed_before_several_wraps_decides_the_stop_closure}`; harness `count_in_metronome_keeps_its_beat_across_a_loop_wrap_and_stop_at_the_wrap_wins`. Arm-time replace/overdub intent per take and pass: `loop_capture::tests::projection::pass_projection_preserves_occurrence_carry_and_replace_overdub_intent` | Pass |
| 9 | Observers cannot alter audio, block the renderer, or need GUI state | `tests::live_observation::{live_observation_subscribers_change_no_sample_and_never_block_the_renderer, a_store_for_another_plan_is_refused_and_silenced}`: no store, a draining subscriber thread and a stalled one leave audio identical | Pass |
| 10 | Retired resources are reclaimed off-thread under saturation | `replacement::tests::saturated_reclamation_across_threads_never_moves_destruction_onto_the_callback`. The application switch retires sessions through a ring and parks them when it is full, never dropping them on the callback (`a_disable_with_a_full_command_ring_still_takes_effect`) | Pass |
| 11 | The active-plan acknowledgement matches the audio across swaps, refusals, failed compiles and failed callbacks | Harness `swaps::acknowledgement_survives_refusals_and_reports_a_failed_callback`, `tests::a_failed_outer_callback_is_acknowledged_without_rendered_output`. Mapping to a GUI/MCP project revision stays in the exit gate | Pass |
| 12 | ADR-0054 reselection and P03-R004 cover the admitted producer partition before a production live adapter | EVD-0025 (`evd_0025_song_partition_matrix`, `one_measured_run_satisfies_the_falsifiers`, `f1_admission_refuses_one_compiled_event_over_the_share`, `f2_compilation_refuses_a_session_share_below_the_activation_cost`, `stopped_control_charges_nothing`) qualifies compiled 96 and session 128 over every saved project that lowers; `lowering::tests::live::the_qualified_gate_opens_and_a_refused_project_still_cannot_play`. Song playback admits no live ingress, so P03-R004's width validation is not reached and stays with the first live adapter | Pass |

The application's song playback equals the offline render shifted by one quantum
(`lowering::tests::live::a_lowered_project_plays_live_exactly_as_it_renders_offline`).
Pause and resume keep the song position, and playback stops at the arrangement
end. The switch silences V2 from the next callback after a stop, a refused edit
or leaving V2 mode, and no session queued earlier can lift that silence. The
checks are `installs_queued_before_a_stop_stay_silent`,
`nothing_but_leaving_lifts_a_refusal` and
`a_refused_edit_at_stop_silences_a_playing_session`.

## Quality gates and independent review

These commands ran on Linux x86_64 on 777bb4e0 and again on the squash of `phase-9a` onto `main`. The release-mode regression
compiles the alternate assertion branch; it is not a release operation.

| Command | Result |
|---|---|
| `python3 -B scripts/check_v2_docs.py --evidence` | Pass |
| `python3 -B -m unittest scripts/test_check_v2_docs.py` | Pass |
| `cargo fmt --check` | Pass |
| `cargo build --workspace` | Pass |
| `cargo clippy --workspace --all-targets` | Pass |
| `cargo test --workspace` | Pass |
| `cargo test -p synth_engine --release resource_limit_probe_oversized_callback_exposes_build_mode_failure` | Pass |
| `cargo doc --workspace --no-deps` | Pass |
| `cargo check --workspace --all-targets --no-default-features` | Pass |
| `cargo check --workspace --all-targets --all-features` | Pass |
| `cargo +1.98.0 check --workspace` | Pass |

Each slice on `phase-9a` received an independent Codex review before its commit.
The Codex review is a different model family from the Claude Code author. No
review waiver was used. The mute repair in 777bb4e0 received a focused Codex
reread. Its one further finding, that a stop after a refusal could lift the
mute, was not reachable because play is refused after a refusal. It was closed
with guards anyway, and the contract is held by
`nothing_but_leaving_lifts_a_refusal`.

A fresh Codex reader reviewed this record against the gate text, ADR-0076,
ADR-0077, the host I/O specification and NOW.md. It found four defects:

- the application skipped an oversized callback silently, contradicting
  IO-INV-002;
- item 2 overclaimed a saved-project structural edit;
- item 8 lacked evidence for arm-time intent;
- item 4 overclaimed allocation coverage.

The code now takes V2's terminal fault with a visible re-preparation, and the
rows were corrected. A focused reread found that the real V1 engine was still
handed the oversized callback in V2 mode. The switch now skips V1 for it, and a
second focused reread found no defects.

## Deviations and residual obligations

| Item | Impact and fail-closed behavior | Owner and pull-forward rule |
|---|---|---|
| Mixed producers (ADR-0075) | Refused by `MixedProducerPlan`; song playback has no live ingress | Phase 9 exit; blocks its first consumer and Phase 12 |
| Physical timing, loopback, recovery and ADR-0022 | No physical claim; V2 mode disconnects MIDI and consumes no host timestamp | Phase 9 exit |
| P03-R004 width validation | Not reached: no live ingress in any shipping-feature path | The first production live adapter, before it is enabled |
| Application song-playback limits | Only 4 of 28 saved projects lower; the rest refuse with a named reason. No seek, loop, pattern preview or live edits while playing; play and pause wait two maximum callbacks plus a quantum | ADR-0077's revisit condition: locate, loop entry or running replacement, or a wider lowering |
| GUI/MCP project revision mapping | The acknowledgement is proven in the harness only | Phase 9 exit |
| Saved-project structural edit through the application | Only the harness proves item 2; the application installs whole sessions but no test changes a saved topology | Phase 11's first live-edit consumer, before it relies on an application edit |
| Persistent recording assets and commit | Only the in-memory take and V2-side intent are proven | Phases 10A, 10B and 10D |

No real-time, persisted-data, protocol or correctness guarantee that Phase 11
needs is weakened. Project save semantics are unchanged, and a default build has
no V2 code path.

## Outcome

Outcome: Accepted

All twelve gate items pass at their written scope; item 2 is proven in the harness,
with the application-edit residual owned above. The complete repository gate passes
and the independent reader's findings are repaired and reread. Phase 11 may begin
once Phase 10E is also complete. Phase 9's exit is unchanged and remains open.
