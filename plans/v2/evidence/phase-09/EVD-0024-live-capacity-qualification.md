# EVD-0024: Live Capacity Qualification Coverage

| Field | Value |
|---|---|
| ID | EVD-0024 |
| Status | Active |
| Phase | 09 |
| Created | 2026-09-21 |
| Last reviewed | 2026-09-26 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `ab1cd27d9a98b1f05880f359d5334b88ecb0d5f3` for the retained component rerun |
| Retention | Until phase exit |
| Related | ADR-0054, ADR-0050, ADR-0074, EVD-0020, EVD-0021, P03-R004 |
| Artifacts | [Full component run](EVD-0024-component-run.txt), [extracted result rows](EVD-0024-component-results.txt), and [checksums](EVD-0024-SHA256SUMS.txt); no production acceptance evidence |

## Question and falsifier

Can the current host qualify the complete simultaneously legal producer partition
for production live use? The component matrix is retained against a committed
source revision, but it does not provide production acceptance evidence. A passing
component stress test does not open that gate.

The selection rule precedes collection: retain the provisional numbers unless
every enabled producer has an observed destination high water and an admitted
bound, every simultaneously legal combination has a common host execution path,
and all live queues, release reservations and identity widths have representative
workload coverage. Missing composition or population coverage means **Not
supported** for production qualification, even if all executable probes pass.
The method must never fill a missing producer's share with synthetic charges and
count that as its occupancy. An absent renderer-internal producer is unavailable,
not a measured zero-cost producer.

For the new component probe, absence of offered work must report zero compiled,
live and release occupancy. A saturated real compiled scheduler plus real ingress
must report the exact offered counts. A compiled event above its share must be
refused before playback; an ordinary live event above queue capacity must be
counted and refused while every reserved release remains deliverable. Nonfinite
audio, an orphan, a leaked hold or a renderer fault fails the probe.

## Inputs and controls

The new probe uses the existing voice envelope/constant/amplifier graph with eight
live note identities. It runs at 44.1, 48 and 96 kHz, mono and stereo, with 64,
256 and 8192-frame maximum host blocks. These are controlled component fixtures,
not a representative production project population.

Every fixture first accepts exactly the compiled share at position zero and
rejects one more there specifically with `WindowOverShare`, checking its requested
count and share. It adopts the real ingress with a beyond-horizon offer, checks
that exact refusal and its counter, and confirms that no entry or hold remains.
Every render thereafter passes that adopted store, including the empty control.
For each profile, run the empty scheduler/ingress control before both loaded arms.
Exercise 64 loaded callbacks of each arm:
compiled automation at its share in the callback's first rendered quantum with either a full ordinary live queue, or eight
reserved releases and the remaining ordinary queue capacity. Release-bearing
arms start the eight real notes before each measured callback. The compiled
stream uses the same parameter and values as live traffic; both pass through
their real producers. The shared arbiter supplies occupancy, never elapsed time.
Each arm first consumes the initial Q-frame output carry without input; this
call renders no quantum. The subsequent setup callback can then render the notes
even when its length is only Q. An
assertion aligns compiled destinations with the renderer's actual engine clock.

## Method

Count exact high waters, queue refusals, delivered releases and final holds. Report
each arm independently; maxima from different arms must not be added and called
simultaneous. Timing, physical callback deadlines and physical round-trip latency
are outside this measurement. No loopback is connected.

Run the existing authored-source, live/activation refusal, concurrent mailbox,
capture, PCM worker and identity controls as complementary coverage. Their owners
and partitions remain distinct. Existing EVD-0020 and EVD-0021 are retained
evidence for their original source revisions, not evidence for a new combined
production host.

## Reproduction

```text
cargo test -p synth_engine_v2 --test simulated_ingress capacity_probe_controls -- --nocapture
cargo test -p synth_engine_v2 --release --test simulated_ingress evd_0024_capacity_matrix -- --ignored --nocapture
cargo test -p synth_engine_v2 --lib authored::tests
cargo test -p synth_engine_v2 --test simulated_ingress
cargo test -p pertylizer --example v2_cpal_output --example v2_live_session
```

From the repository root, regenerate and verify the retained component rows:

```bash
rg '^capacity,|^scope=' plans/v2/evidence/phase-09/EVD-0024-component-run.txt > plans/v2/evidence/phase-09/EVD-0024-component-results.txt
sha256sum -c plans/v2/evidence/phase-09/EVD-0024-SHA256SUMS.txt
```

## Results

On 2026-09-26, the `evd_0024_capacity_matrix` release test was rerun from committed revision
`ab1cd27d9a98b1f05880f359d5334b88ecb0d5f3` in a fresh Cargo target
directory. The [full run](EVD-0024-component-run.txt) records the exact command,
UTC time, worktree status, confirmation that Rust paths matched that revision,
Rust/Cargo and Linux versions, compilation, one passing test and exit status zero.
The [extracted rows](EVD-0024-component-results.txt), regenerated from that run,
retain the 36 loaded cases plus the scope marker. The
[checksum file](EVD-0024-SHA256SUMS.txt) covers both artifacts. The test asserts
zero occupancy in one empty control per profile, before its two loaded arms:
18 controls for 36 loaded cases. It does not print those controls. All loaded
cases passed, and each arm executed 64 loaded callbacks. These are retained
component checks, not production acceptance evidence. The matrix reports:

| Arm | Asserted compiled peak | Asserted live peak | Asserted release peak | Checked same-quantum external total |
|---|---|---|---|---|
| Empty control | 0 | 0 | 0 | 0 |
| Ordinary queue | 96 | 32 | 0 | 128 |
| Reserved releases | 96 | 24 | 8 | 128 |

The empty-control row comes from passing assertions. For loaded rows, the test
asserts the per-class peaks, refusals and final holds before printing their
expected values; only `external_total` is printed from the arbiter's measured
high water. The retained rows are therefore an audit of passing checks, not
independent measurements of every displayed value.

Across the 36 loaded cases, 2,304 deliberate extra ordinary offers were refused;
9,216 reserved releases were delivered, every final hold count was zero, and no
renderer fault, orphan or nonfinite output occurred. The release arm also checks
that its notes actually sound during setup, that every release callback is silent,
and that the final output is silent.
The observed total is 128, not the sum of independent per-class maxima, 136.

The default values remain unchanged:

| Resource | Default | Coverage and selection disposition |
|---|---|---|
| Compiled share | 96 | Real automation reaches 96 in this probe; no production project survey here |
| Authored-runtime share | 48 | EVD-0020 covers the finite standalone Note YAMS owner; not composed with live input here |
| Live share | 32 | One adopted ingress store reaches 32; the concrete host's source merge and MIDI expansion have additional bounds |
| Session share | 128 | EVD-0021 selected it from catch-up admission requests; no combined production session/live owner here |
| Internal share | 16 | No executable renderer-internal event producer found; positive reserved share is not measured occupancy |
| Release share | 40 | Eight actual releases coincide with ordinary traffic here; this does not exercise 40 |
| Release holds | 40 | Eight held notes in this probe; no combined authored/live hold population |
| Total events per quantum | 360 | The six reserved shares sum to 360; observed external total 128 cannot qualify the entire cap |
| Performance ingress depth | 32 | Full queue and first excess offer covered; no representative arrival burst distribution |

The host adds other finite stores, including its 128 audition observation credits,
64-cell transport rings, and the duplex runner's 32,768-frame PCM queues. Those
are distinct units and owners, not additional event shares. Existing lifecycle,
allocation and concurrency tests exercise them separately. This probe does not
select their production depths or claim a combined retained-byte ceiling.

Before the 2026-09-21 control repairs, that day's uncommitted development run
passed the complete repository gate, including the ordinary authored-source
tests, the live/activation refusal regression and the million-mint identity
test. The unchanged concrete example suites were also run before the repairs:
`v2_cpal_output` passed 60 tests (including mailbox, capture and concurrent PCM
coverage), and `v2_live_session` passed three. The ignored historical EVD-0020
measurement was not re-run. After those repairs, the release matrix, evidence and
documentation gates, formatting, package-wide Clippy and all `synth_engine_v2`
tests were re-run and passed in the 2026-09-21 development worktree. Those
historical checks are separate from the retained 2026-09-26 component rerun;
neither is a combined host load measurement.

## Limitations

`AuthoredNoteStream` exclusively owns its renderer, arbiter and single authored
note producer; its compiled lane accepts parameter/controller automation only.
`LiveInputStream` owns a separate renderer and one noncompiled producer with at
most eight voices. `PerformanceIngress::prepare` refuses a compiled-note producer
in the same plan. `StreamControl::plan_activation` refuses an adopted live store.
The concrete capture host composes separate capture, audition and metronome
owners; physical PCM duplex has a separate runner. None establishes a common
production ownership or aggregate admission policy for all these components.
The eight-note probe does not qualify the full declared identity index space or
the generation width for representative long-lived use; the earlier million-mint
test remains a mechanism control. Maximum callback execution time, underruns and
compiler/worker contention under combined host load are unmeasured here.

## Conclusion

**Not supported for production qualification; component capacity controls pass.**
The coverage audit fails the predeclared composition and population conditions.
No default is reselected and ADR-0054's production-live gate remains closed.
This is an active investigation, not a completed production acceptance record.

The next implementation dependency is a common host admission and ownership
boundary for playback, live input, session operations, recording, monitoring and
plan replacement. In particular, same-stream transport activation must redeem
live release holds and preserve minter ownership before its current refusal can
be removed. Any authored/live combination must also establish independent gate
ownership. Once those paths exist, repeat the complete six-share/queue selection
over the production workload and retain an exact committed source and results.
Physical round-trip calibration remains separately unavailable without loopback.
