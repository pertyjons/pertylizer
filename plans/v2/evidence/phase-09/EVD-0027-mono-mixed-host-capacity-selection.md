# EVD-0027: Mono mixed-host capacity selection

| Field | Value |
|---|---|
| ID | EVD-0027 |
| Status | Complete |
| Phase | 09 |
| Created | 2026-10-17 |
| Last reviewed | 2026-10-17 (reviewed freeze and first Mono collection) |
| Supersedes | — |
| Superseded by | — |
| Source revision | `836b7527af66fcd422915fa20f83ca76e9e769f0` (reviewed clean implementation/method freeze) |
| Retention | Until phase exit |
| Related | ADR-0080, ADR-0041, ADR-0054, EVD-0026 |
| Artifacts | [Workload checksums](EVD-0027-workload-SHA256SUMS.txt), [compact results](EVD-0027-mono-results.txt), [artifact checksums](EVD-0027-mono-SHA256SUMS.txt); full trace retained externally below |

## Question and falsifier

Does the explicit terminal mono fold support the same frozen saved-project,
two-source sound workload and custody entitlements as EVD-0026, with its own
reference/selected capacity runs, no callback heap activity, complete sound/raw
custody, bounded fence delay and the predeclared callback budget at all four
candidate rates?

The conclusion is **Supported** only for this fixed serial Mono candidate if
all controls pass, both runs complete at every rate, all observations fit the
independent bounds, the selected rerun reproduces its configuration, and raw
callback maxima fit the budget. A false demand bound, lost protected release,
callback allocation/free or blocking operation, changed folded sound, failed
selected runtime, selected deadline overrun or unstable selection is **Not
supported**. A missing observation/required path, preparation or admission
failure, reference runtime fault or reference deadline overrun is
**Inconclusive**. No failed prefix can be discarded, and no limit, CPU or
threshold may be raised after seeing a result. Missing or unavailable producers
must not be reported as measured zero.

## Inputs and sound boundary

ADR-0080 defines the explicit `StereoToMono` node. Project lowering keeps the
stereo instruments, inserts, master processing and stereo master tap, then
inserts the reserved terminal fold before the Mono output. The kernel computes
the mean using f64 intermediates and rounds once to f32. The lowerer emits
`MonoOutputFold`, so no stereo V1 parity verdict can read this mono outcome.
Application mono-device admission and production mixed refusals remain closed.

The saved input remains `CORPUS-0005`'s `instrument-inserts`, SHA-256
`1d55668d424e71cb49d23c5da46f7f35541b8c16669dc1399a8ef28fefe12dbc`.
The independent sound controls run in the quality gate before collection:
`stereo_to_mono` pins the kernel law, and
`lowering::render::evd_0026::mono_saved_lowering_keeps_the_stereo_graph_and_marks_the_explicit_terminal_fold`
pins the saved graph and audio. The sustained collector does not independently
compare its complete host waveform with a stereo host, including reset fades.
The lowerer's ordinary and mixed Mono graphs must equal the checked layout-aware
transcription. Their audio must equal the specified fold of their Stereo
counterparts at 44,100, 48,000, 96,000 and 192,000 Hz. The four independent
literal note timelines remain unchanged. The node adds no producer event,
parameter control, latency, history or tail; its declared operation and separate
arena region remain accounted resources. EVD-0026's source revision and stereo
artifacts remain historical and are not rewritten or reused as mono timing.

The candidate constructor fixes Mono and 64-frame callbacks and admits only
those four rates. It refuses other rates before graph compilation. The
compiled span is S=2; the legal simultaneous live sound holds are G=4, two per
source. Source-ring custody separately preserves 128 physical cells, 16 ordinary
cells, 64 held keys and 16 pedals per connection with protected releases. Those
custody entitlements do not authorize more than four simultaneous sound notes.
The source fence uses `peer_wait=64` and one healthy frontier per callback.

## Frozen workload, bounds and selection

The complete executable script is `sustained_host::sustained_run_with`:
1,024 warm-up plus 10,000 timed callbacks, priming included in the 11,024 total.
Seeks and 28-frame loops occur every four quanta; loops wrap two or three times
per quantum. Stop/Start, full-G4 Panic, reset with both audio owners, replenished
notes and subsequent activations execute after warm-up. At script callback
4,003 both source rings hold 16 ordinary bends and two protected key-ups;
64 live entries and four releases coincide with three actual wraps. Four notes
are replenished at 4,004. The frozen request times, raw message/frontier receipts,
exact sound/release links and both reset-owner teardowns must remain intact.

Before each reference/selected run, repeat the actual configuration's empty and
one-unit attribution controls, actual admission N/N−1 controls, both-source fence
controls, whole/8×8/7+13+1+43 pressure partitions and raw 64-key reserved-release
control. The allocator observer's positive controls detect transient allocation
and external-buffer freeing; its timing control deliberately exceeds every
candidate budget. These run in the quality gate before collection.

The following B values are independently authored entitlements and coincidence
bounds, fixed before observing this workload. They retain EVD-0026's derivation;
the terminal fold has no publisher/control and cannot increase them.

| Resource | Frozen B |
|---|---|
| Compiled | 6 |
| Authored-runtime (unavailable) | 48 |
| Live | 96 |
| Session | 64 |
| Internal (unavailable) | 16 |
| Release | 8 |
| Live holds | 4 |
| Performance ingress | 64 ordinary + 7 protected = 71 |
| Command and result rings, each | 32 ordinary + 7 protected = 39 |
| Per-source stage | 16 ordinary + 3 protected = 19 |
| Per-source raw input | 136, preserving existing custody entitlement |
| Raw packet and completion rings, each | 64 |
| Raw session commands | 4 |
| Raw source actions | 256 |

The actual proof and executable inventory are in `frozen_capacity`,
`capacity_boundaries` and EVD-0026's
[independent demand inventory](EVD-0026-combined-host-capacity-selection.md#independent-demand-inventory).
Actual N−1 controls use true admission floors, not B−1, which can still admit
under a conservative bound. `validate_frozen_bounds` must reject H>B before
configuration validation or max(H,B) selection can absorb an undercount.

`P_ref` uses the Mono profile's default shares, hold capacity and physical queue
depths scaled eightfold, with checked arithmetic, an event cap equal to the six
scaled shares, and the full 4,096-event authored scheduled-window headroom.
Its raw source-action byte budget is 2,097,152; the selected budget stays
262,144. Other default limits and the reporting-only legacy command/event-egress
fields stay unchanged. The source-ring physical contract stays fixed.

Use the unchanged checked `capacity_selection` rule. Select each physical queue
as round32(2×max(H,B,1)); include the performance floor 2G+1 and call its result C.
Command/result depths additionally satisfy depth≥C+1 before rounding. Select
holds as round32(2×max(H,G,1)). Select executable shares as
round32(2×max(H,B,1)), with C included for Live and C+ceil(S/8) plus selected holds
included for Release. Unavailable shares use B alone and retain undefined H.
Set the total event cap to the six-share sum; preserve the default scheduled
window or the compiled callback floor plus the full 4,096 authored headroom,
whichever is greater. Admit the graph, run the full selected configuration,
and require identical reselection. Do not replace a running configuration.

## Measurement and environment

The shared `capacity_collection::collect` brackets the complete
`HostCallback::render`, including mixed output, raw recording, completions and
both reset owners, with `Instant` reads inside an allocation-counter scope.
All 11,024 callbacks must have zero total allocations, zero net allocation
count and zero net bytes. Audio-side prepared handoffs also require zero heap
activity. Scope setup/collection is outside the retained time; clock overhead
is included. The lint-grade render purity gate plus source review covers
blocking operations; no continuous-thread scheduling guarantee is inferred.

For each rate and run, retain exactly 10,000 post-warm-up durations, their raw
maximum and nearest-rank p99.9, path maxima and an empty-callback floor. The
fixed budget is floor(16,000,000,000/rate) ns, **25% of a 64-frame period**.
The raw maximum must fit; never subtract the floor. Attribution, boundary,
fence and partition controls are functional and untimed. Both sources' silent
peer test must observe the strict threshold and 127-frame offset versus the
known-prefix control. The conservative 192-frame bound is below 5 ms at
44,100 Hz; a healthy peer must not be fenced.

The entire fresh lifecycle from graph construction/compilation through joined
teardown has a separate outer requested-heap scope. Its fixed bound is
536,870,912 bytes (512 MiB), including all queues, simultaneous owners,
compiler temporaries and retained test buffers. Nested scopes may overstate
its peak. This is not RSS, allocator bookkeeping, stack or a per-plan budget.
All required/written timed controls, payloads, seeds and adoption gates must fit
prepared writable extents; rejected writes and unrepresentable usage invalidate
the run. Keep owner-local observations separate and combine maxima, never
sum them; shared source/raw queues retain host-level peaks.

Before collection, commit the independently reviewed implementation, workload,
method, selector and checksums. Prebuild release mode and finish competing
build/review work. Use the lowest CPU in initial allowed affinity, preserve
that single-CPU affinity and `performance` governor, and record CPU model,
clock source, Linux/toolchain versions, source SHA and exact command. Frequency
may vary. The collector verifies clean HEAD equals the explicitly supplied
freeze SHA, matching workload checksums and unchanged governor. It changes
no machine policy.

## Reproduction

```bash
CARGO_BUILD_JOBS=2 cargo test -p synth_engine_v2 --release --no-default-features --features mixed-transfer-rehearsal --test mixed_source_bridge --no-run
PERTYLIZER_EVD_0027_COLLECT=1 PERTYLIZER_EVD_0027_FREEZE_REVISION=<reviewed-sha> taskset -c <cpu> cargo test -p synth_engine_v2 --release --no-default-features --features mixed-transfer-rehearsal --test mixed_source_bridge saved_project_host::sustained_host::capacity_collection::mono::collect_mono_reference_then_selected_at_all_four_rates -- --ignored --exact --nocapture --test-threads=1
```

Keep the complete raw trace outside `plans/v2/` under the evidence artifact
policy. Commit a compact summary, content digests and the stable raw location.
A failed prefix is retained; regeneration does not promise identical times.

## Results

The first collection ran on 2026-10-17 at 12:46:09 UTC from clean, independently
reviewed freeze `836b7527af66fcd422915fa20f83ca76e9e769f0`. The selected
half-sum implementation, graph transcription, independent bounds, collector
and 48-file checksum inventory were committed before collection. No failed
prefix, retry, limit increase or CPU change occurred. The process exited zero.

The host was an Intel Core i7-13700H, Linux 7.2.11-200.fc44.x86_64 with glibc
2.43, Rust/Cargo 1.99.1 and LLVM 23.1.1. The release binary was prebuilt;
competing build/review tasks had ended. Logical CPU 0 was the lowest initially
allowed CPU; affinity stayed fixed, the governor remained `performance`, and
the clock source was `tsc`. This is the serial simulated host described above.

All eight runs completed in reference-then-selected order at each rate. Each
retained 11,024 callbacks: 1,024 excluded only from the estimator and 10,000 timed.
All **88,192 callbacks** passed zero total allocations, zero net allocation
count and zero net bytes. The following maximum and nearest-rank p99.9 values
were recomputed separately from the retained duration rows. The empty-callback
floor was 9 ns throughout; it was never subtracted.

| Rate (Hz) | Reference max (µs) | Reference p99.9 (µs) | Selected max (µs) | Selected p99.9 (µs) | Budget (µs) |
|---|---|---|---|---|---|
| 44,100 | 70.269 | 34.043 | 56.463 | 27.828 | 362.811 |
| 48,000 | 75.574 | 30.184 | 57.817 | 30.545 | 333.333 |
| 96,000 | 66.112 | 33.835 | 58.354 | 28.118 | 166.666 |
| 192,000 | 69.159 | 33.421 | 76.709 | 38.848 | 83.333 |

Every required path executed after warm-up, including three-wrap ordinary
pressure with protected releases and reset rendering both owners. Separate
seek/loop, Stop/Start, Panic, pressure and reset maxima are in the compact
results. At 192,000 Hz, the selected reset callback produced the 76.709 µs
maximum. Passing these finite observations is not a worst-case scheduling or
execution-time guarantee.

Resource observations were identical in all eight runs. Publisher/ingress
observations are owner-local, combined componentwise across reset owners;
shared stage/raw queues retain their own host-level peaks.

| Resource | Observed high water | Frozen B | Selected capacity |
|---|---|---|---|
| Compiled | 6 | 6 | 32 |
| Authored-runtime | Undefined | 48 | 96 |
| Live | 64 | 96 | 320 |
| Session | 57 | 64 | 128 |
| Internal | Undefined | 16 | 32 |
| Release | 4 | 8 | 352 |
| Live holds | 4 | 4 | 32 |
| Performance ingress | 68 | 71 | 160 |
| Command ring | 36 | 39 | 192 |
| Result ring | 36 | 39 | 192 |
| Stage, per source | 18 | 19 | 64 |
| Raw input, per source | 20 | 136 | 288 |
| Raw packet ring | 38 | 64 | 128 |
| Raw completion ring | 38 | 64 | 128 |
| Raw session commands | 1 | 4 | 32 |
| Raw source actions | 40 | 256 | 512 |

The selected six-share sum is **960 events per quantum**, its scheduled window
**4,192**, and its raw action byte budget **262,144**. The fixed rule reproduced
each selected configuration after the complete rerun. No running profile was
replaced. The four sound holds remain the admitted sound entitlement; the
selected hold capacity of 32 is storage headroom. Unavailable producers remain
undefined observations and provisional reservations, not enabled publishers.

All actual-configuration attribution, admission-boundary, fence, partition and
raw-key custody controls passed before their runs. The known-prefix/healthy/silent
peer controls passed for both sources at all rates: no healthy peer was fenced,
the strict equality boundary did not fence, and the silent-peer offset was
127 frames. The conservative 192-frame added-delay bound remains below 5 ms
at the minimum admitted rate. The source rings retain their physical contract.

All renderers reported zero rejected control writes and usage within prepared
writable extents. Both sound owners and raw capture joined with complete custody,
ordered source message/frontier receipts and exact release links. The complete
repository gate plus saved-lowering and app controls passed all 21 commands;
the five DSP controls also passed in release mode. The standalone sound controls
verified the specified fold and saved graph in offline renders, separately from
the timed host script. The additional commands on the freeze source were:

```bash
cargo test -p pertylizer --features v2-lowering --lib lowering::render::evd_0026::
cargo test -p pertylizer --features v2-lowering --lib lowering::tests::app::
cargo test -p synth_engine_v2 --release --no-default-features --features mixed-transfer-rehearsal --test stereo_to_mono
```

The lint-grade render purity checks and source review found no blocking lock
or wait in the render path; device-thread scheduling remains unqualified.

The outer requested-heap peaks ranged from 23,504,687 to 25,871,087 bytes for
reference runs and 21,186,351 to 23,552,751 for selected runs. Every peak fits
the independently frozen 536,870,912-byte bound. This includes preparation,
compiler temporaries, simultaneous reset owners and retained test buffers;
it is not RSS, allocator metadata, stack or a per-plan memory claim. The full
trace separately retains all owner storage and writable extents.

The [compact results](EVD-0027-mono-results.txt) retain per-run heap peaks,
path maxima, observations and exact selected configurations. The complete
2,790,288-byte raw trace is
`/home/per/pertylizer-evidence/phase-09/EVD-0027-836b7527-mono-run.txt`,
SHA-256 `570c69d2d35645e55965ec622a3d35460c931e7d7e5eb4692c852472670514d2`.
It retains command/environment metadata, all callback durations, owner snapshots
and queue maxima. Preserve this external artifact when moving the work between
machines. Reproduction uses the collector command in
[Reproduction](#reproduction) on the clean freeze revision,
CPU 0, and `PERTYLIZER_EVD_0027_FREEZE_REVISION=836b7527af66fcd422915fa20f83ca76e9e769f0`.
Fresh timing samples will differ. EVD-0026's historical results are unchanged;
no cross-session speed comparison is made.

## Limitations

This candidate is one serial simulated host, one saved graph, four sound holds
and 64-frame callbacks. It does not qualify other projects/block sizes,
continuous/concurrent device callbacks, physical timing or the unavailable
mixed authored/internal publishers. The complete sustained host waveform,
including reset fades, was not compared against a stereo host under either
reference or selected configuration. The standalone offline sound controls do
not establish that waveform equivalence. The selected 192 kHz maximum leaves
6.624 microseconds of the frozen callback budget; this observed margin does not
establish a worst-case bound for continuous device scheduling.
No stereo/mono speed comparison is inferred
from different source revisions or sessions. Application and production mixed
refusals remain in force. Phase 9 remains open.

## Conclusion

**Supported for the frozen serial Mono candidate.** Standalone offline controls
verified the explicit half-sum law and saved graph. Static purity checks and
source review passed; the untimed attribution, admission-boundary, fence,
partition and raw-key custody controls passed before each actual configuration's
run. The timed runs exercised the enabled producer paths and passed independent resource bounds, callback allocator
checks, custody and raw callback budgets at all four rates for the reference
and selected configurations. Queue/hold reselection was stable. This verdict
does not establish sustained host waveform equivalence with a stereo host.

This completes the bounded Mono experiment. It supplies fresh mono evidence
alongside EVD-0026's stereo experiment; it does not close Phase 9 or enable
mono devices in the application. Other projects/block sizes, continuous and
concurrent callbacks, physical qualification and unavailable mixed publishers
retain their separate gates. Production mixed refusals remain in force.
