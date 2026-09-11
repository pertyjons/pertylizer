# REV-P08: Phase 8 Exit Review

| Field | Value |
|---|---|
| ID | REV-P08 |
| Status | Accepted |
| Phase | 8 |
| Created | 2026-09-10 |
| Last reviewed | 2026-09-10 |
| Reviewed source revision | 22739cba14413b189765c8ac2698845131e91910; the exit records are committed immediately after this revision |
| Roadmap outcome | [Phase 8 — mixer and latency graph](../ROADMAP.md#phase-8--mixer-and-latency-graph) |

## Review scope

The compiled Mono/Stereo mixer graph: channel strips and track balance, linear
sums, instrument and return inserts, pre/post-fader sends, chained return buses,
master inserts and output policies; declared node history, tail and path latency,
compensation and explicit shared feedback; current-quantum compressor detectors;
and bounded channel, return and master observation. `SOUND-INV-031` through
`SOUND-INV-037` and `LOWER-INV-004` state the implemented contracts.

The native insert catalog carries soft-clip distortion, mono-mode delay and the
V1-law compressor. The saved-project lowerer carries only explicitly represented
ports, parameters, stages and timing. Native fan-in and feedback do not silently
infer a meaning for ambiguous saved routes. A saved diagnostic excludes a parity
verdict, including an intentional timing difference. This phase does not deliver
complete V1 migration, rate islands, hardware latency, a shipping V2 dependency,
or the shared V1/V2 job surface that ADR-0028 defers to Phase 10B.

The stopping rule is a false behavioral claim, a contradiction, an unsupported
acceptance conclusion or a contract hole required by a dependent phase. Optional
catalog expansion is not a gate. Evidence is audited at its stated scope; accepted
ADRs and old performance measurements are not re-decided here.

## Required decisions and inventory closure

| Item | Actual state and evidence | Result |
|---|---|---|
| ADR-0033 explicit feedback | Accepted in S006. One shared quantum of history, deferred capture after graph reads, automatic compensation refused at the boundary; current-quantum sidechains and their saved timing difference are explicit | Pass |
| ADR-0034 persisted track/source/channel ownership | Proposed for Phase 0B/10A; compiled `ChannelId` and `BusId` introduce no persisted ownership contract. Differing track controls/sends on one saved instrument refuse rather than select an owner | Pass |
| ADR-0027 observation | Accepted; existing subscription mechanism reads declared taps. Mixer and observation ceilings remain independently enforced | Pass |
| ADR-0054 capacity | EVD-0021 reselected the session share before S004 enabled its wider consumer. This exit selects no new capacity and makes no live-ingress qualification | Pass |
| ADR-0022 hardware latency and ADR-0028 revisioned jobs | Deferred to their named Phase 9 and Phase 10B gates. No physical host or shared render service consumes an unaccepted contract | Pass |
| Native catalog | 41 discoverable kinds, excluding the output sink; registry and representation tests cover their declarations. Latency, tail and history are derived from the same declaration used by compilation and admission | Pass |
| Current conformance rows | `SOUND-INV-031`–`037` and the lowering rows name executable checks; the documentation gate validates coverage and links | Pass |
| Saved corpus | All ten pinned inputs classified in [EVD-0022](../evidence/phase-08/EVD-0022-routing-qualification.md); four bounded renders with markers, six refusals, no unsupported parity verdict | Pass |
| Phase task table | S001–S005 landed on `main`; S006 `6f3862be`, S007 `90677d50`, S008 22739cba14413b189765c8ac2698845131e91910 complete on `feat/v2-phase8-completion`. Historical coordination is retained in [the archive](../archive/phase-08/INDEX.md) | Pass |

## Exit gates

The six master-plan bullets retain their written scope. The first is covered by
complementary intervention tests: the same-patch project fixtures exercise real
lowering, while the native tap fixture directly observes each compiled strip.

| Gate | Evidence or named automated checks | Result |
|---|---|---|
| Two independent channels using the same patch retain independent faders, inserts, sends, meters, voices and tails | `lowering::tests::phase8::two_instruments_on_one_patch_are_two_channels_summed_exactly` renders overlapping notes with unequal faders as the exact sum of each route alone. `phase8_exit::two_channels_on_one_patch_keep_independent_insert_state_and_tails` adds nonlinear distortion and delay to both copies; the full render equals each isolated route's sum, removing only A's inserts changes A while B's complete ring-out remains the exact other summand, and removing B's delay eliminates its measured tail. `buses::two_instruments_on_one_patch_keep_independent_sends` changes only one send with the other route unchanged. `observe::tests::channel_return_and_master_taps_are_stereo_lossy_and_audio_invariant` resolves distinct compiled meter identities and reads distinct channel strips and the return | Pass |
| Sidechain latency is reported and independent of callback size | `tests/sidechains.rs::detector_latency_aligns_main_audio_and_is_independent_of_callback_size` asserts the main/detector alignment and report under different host partitions; `detector_changes_gain_without_becoming_an_audible_input` distinguishes patched silence from the unpatched fallback. The saved source-resolution test carries `SidechainTiming` and refuses a missing source | Pass |
| Float offline output preserves documented headroom | `mix_channel::a_sum_above_full_scale_is_preserved_in_float` and `lowering::tests::phase8::the_parity_policy_clamps_at_full_scale_and_headroom_preserves_the_sum` exceed full scale. Headroom bypasses terminal limiting, channel/return clipping and final clamp; explicit insert processing remains authored processing | Pass |
| Node and route latency is visible and compensated according to policy | `tests/path_latency.rs` holds aligned/declined impulses, min/max spreads through successive merges, serial/disconnected paths, overflow, exact resource charging, partitions, and separate path/tail/offline-trim reporting. Voice tests isolate authored and inserted histories on stealing; kernel tests invalidate history logically without clearing an unbounded line | Pass |
| Return and group routing needs no renderer topological sort per block | `tests/sends.rs` holds a three-bus chain against V1's ordered laws across partitions and refuses ordinary cycles by identity. The lowerer's bus-shape test checks scoped entry, inserts, strips and sends. `compile.rs` emits the immutable operations; `render/hot.rs` executes them, including deferred feedback capture, with no graph sort | Pass |
| Common channel and bus processing remains allocation-free under the RT guard | `render_loop_purity` scans the hot path and kernels; the channel/return/master observation test arms the allocator before the first callback with subscriptions absent and saturated. `feedback_tests::feedback_and_compressor_allocate_nothing_from_the_first_callback` covers both new stateful paths | Pass |

The roadmap's routing/effect-parity boundary is bounded by the represented
catalog: actual V1 modules run over the same inputs in
`v2s_delay_and_distortion_are_v1s_modules_bit_for_bit`,
`v2_compressor_matches_v1_with_internal_external_and_unpatched_detectors`,
`voice_amplifier_and_terminating_output_match_actual_v1_modules`, and
`saved_master_inserts_keep_order_before_volume_and_refuse_bad_identities`.
Channel and send tests independently spell V1's arithmetic in its exact order.
Intentional current-quantum detector/modulation timing and summation-order
changes stay marked. [EVD-0022](../evidence/phase-08/EVD-0022-routing-qualification.md)
qualifies whole saved inputs for deterministic V2 rendering or named refusal; it
issues no numeric V1/V2 verdict over them. ADR-0028's existing boundary remains.

Feedback evidence covers an exact quantum recurrence under declaration-order
and callback changes, in-place source protection, two simultaneous boundaries,
history charge/admission, ordinary-cycle and wrong-scope refusals, zero-allocation
execution and shared history surviving voice stealing, release and transport
relocation. Traversal latency ends at the explicit cut; no finite recurrence tail
or hardware delay is claimed.

## Quality gates and independent review

Run on the tree committed with this review, Linux x86_64, Rust 1.98.0. Every
command is required; the release-mode regression compiles the alternate assertion
branch and is not a release operation.

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

All three native reference audio digests (mono, stereo, gain-chain) reproduced
on S007; EVD-0022 retains them. The tiny digest run is not performance evidence.
S006 and S007 each passed the complete gate and fresh independent Gemini reads.
S008's method, test and ownership scopes received fresh independent reviews
before its commit. A final review of the complete branch in core and lowering
packets found no defects. A fresh semantic exit review checked the six gates,
evidence scope and residual ownership; a focused final documentation read
checked the retained records and cross-file consistency. Both reported no
defects. All readers were `agy` / `gemini-3.8-flash-high`, with fresh contexts,
reviewing Codex-authored changes directly without delegation. No review waiver
was used. The reproduction command-order repair received a focused reread
with no defects.

## Deviations and residual obligations

| Item | Impact and fail-closed behavior | Owner and pull-forward rule |
|---|---|---|
| P08-R001 — rate declarations, whole-plan oversampling and rate islands remain unbuilt | Saved nonunity oversampling is diagnosed and excludes comparison. None of the six exit bullets or the Phase 9 dependency claims nonunity-rate execution | First Sound Core rate-extension slice, before any nonunity-rate node or saved oversampling consumer: decide rates, conversion, history and composed latency and qualify them. Phase 9 inherits this if it expands into that scope |
| P08-R002 — bounded saved-route catalog | Multiple terminals, module-input fan-in, implicit feedback, additional distortion/delay modes and tempo-following delay remain refused by name; native fan-in and explicit shared feedback are already represented | First lowering slice for each named route: define its law, explicit representation and same-input V1 oracle before lifting refusal |
| P04-R004 / ADR-0028 — whole-project V1/V2 orchestration | S008 qualifies V2 corpus output and same-input DSP oracles separately; no shared request, progress or cancellation service exists and no unsupported parity verdict is issued | Phase 10B after Phase 10A's canonical revision; binds the first shared render service |
| P05-R001 — amplifier-level smoothing | S001 decided unsmoothed channel faders against V1. Nonunity saved amplifier level still refuses; S007 represents only unity-level amplification | First lowering that maps that level must qualify its smoothing law against V1 before acceptance |
| Saved state not in the bounded lowering catalog | Reverb and other uncarried effects/ports, saved YAMS, unsupported modulation targets, same-key overlap and voice-stealing differences remain diagnosed; no complete migration is claimed | First lowering consumer of each route; saved Note Grid/racks specifically remain Phase 10A's P07-R001. Mod Grid amplitude targets require a law decision, not merely an address |
| P05-R002, P06-R002 and ADR-0054 clause 3 | Semantic project digest, live notes across recompilation and production six-share qualification are unchanged | Phase 10D and Phase 9 respectively, before their first consumers |
| Unknown declared tails | Filters, envelopes, samplers, scripts and recurrence do not promise a finite composed tail; bounded offline smoke tail is caller-selected | First finite-tail consumer must establish the missing bound; no inferred truncation guarantee |
| Hardware and persisted ownership | Compiled identity and path latency cannot be consumed as physical latency or saved ownership guarantees | ADR-0022 at Phase 9 exit and ADR-0034 at Phase 0B/10A |

The master-plan rate-island work item is explicitly carried under P08-R001,
following S003's recorded decision. The six exit bullets require no weaker
behavior. The S008 task wording is clarified to obey the already accepted
ADR-0028 deferral: exact DSP-stage comparison and V2 corpus qualification do not
claim whole-project V1/V2 A/B. No dependent phase loses a real-time, persistence,
protocol or correctness guarantee.

## Outcome

Outcome: Accepted

All six exit criteria and all eleven complete repository gate commands pass.
Independent semantic and final consistency reviews report no remaining defects.
P08-R001 and P08-R002 retain their fail-closed first-consumer obligations; the
existing ADR-0028 deferral is unchanged. Phase 9 remains unselected and Phase 0B
continues independently.
