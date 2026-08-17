# EVD-0007: Expanded Reference-Corpus Baseline

| Field | Value |
|-------|-------|
| ID | EVD-0007 |
| Status | Complete |
| Phase | 00A |
| Created | 2026-08-15 |
| Last reviewed | 2026-08-15 |
| Retention | Permanent |
| Related | P00A-T001, P00A-T003, REV-P00A, EVD-0001, EVD-0002, EVD-0003 |
| Superseded by | — |

## Question or hypothesis

Does the current ten-case reference corpus remain bit-exact deterministic, and does it have reviewable V1 CPU,
resident-memory, and full-block timing baselines at the four operating sample rates used by EVD-0003?

This is a supplement, not a rewrite of EVD-0001 through EVD-0003. Those records remain the authority for what their
historical inputs measured. This record closes the coverage gap created when CORPUS-0005 through CORPUS-0010 became
executable fixtures after those measurements.

## Acceptance criteria

These criteria are fixed before collection:

- `cargo test -p pertylizer --test corpus_manifest` passes with ten executable cases and one owned reproducibility gap;
- for every case, two release renders in separate process invocations produce identical output SHA-256 digests, the
  comparison reports bit-identical samples and zero for every numeric delta, and neither render nor comparison warns;
- a deliberately different pair of cases compares non-identically before the same-case determinism run, proving that
  the comparison control can observe a difference;
- CPU cost is collected for all ten cases at 44.1, 48, 96, and 192 kHz over ten interleaved rounds, with one warm-up
  and five recorded renders per round: 50 recorded renders per case/rate cell. The retained table reports minimum,
  median, and arithmetic mean milliseconds per rendered second;
- block timing and RSS are collected for the same case/rate matrix over five interleaved rounds, with one warm-up and
  three recorded renders per round: 15 recorded renders per cell. The retained table reports the median of run-level
  p50 and p99 full-block times, the largest observed run-level block maximum, mean signed RSS phase deltas, and mean
  final and peak RSS;
- every harness record identifies the case, sample rate, build block size, repetition, and warm-up status. Any missing
  cell, warning, non-finite value, or count below the thresholds above makes the record incomplete rather than being
  silently omitted.

No performance threshold is attached. These are baselines against which V2 will later be compared, not an admission
test for V1.

## Source and environment

- Source revision: V1 at `54cd6d3f`, plus the Phase 0A fixture, manifest, and evidence change that lands this record
  (the commit introducing CORPUS-0006..0010 and this harness).
- Platform and architecture: Linux 7.1.8-200.fc44.x86_64, x86-64, 13th Gen Intel Core i7-13700H.
- Rust/tool versions: rustc 1.97.1, cargo 1.97.1.
- Build profile: `--release`; V1 `BUFFER_SIZE` remains 256 frames.
- Audio/sample configuration: stereo; 44 100, 48 000, 96 000, and 192 000 Hz; each case keeps its declared duration
  and tail.
- Feature flags: workspace defaults.
- Host or device simulation: offline renderer only; no GUI, audio device, or callback deadline.
- CPU policy: `performance`; each measurement process pinned to logical CPUs 10 and 11.

## Inputs

The ten cases and their input SHA-256 digests are the current entries in
`corpus/v2-reference/manifest.json`. CORPUS-0001 through CORPUS-0004 retain the earlier measurements; CORPUS-0005
through CORPUS-0010 are the coverage added by this supplement. `sampler-patch` remains the sole explicit gap because
its project and sample asset need one deterministic bundle.

## Method

1. Build `pertylizer`, `render_cost`, and `render_profile` once in release mode.
2. Run a comparison control between two intentionally different cases. Stop if the samples compare identical.
3. Render each case twice in separate release-process invocations with the manifest settings and compare the WAVs.
4. Interleave the four sample rates inside each CPU round. Each `render_cost` invocation walks every manifest case.
5. Interleave the four sample rates inside each timing/RSS round. Each `render_profile` invocation walks every case.
6. Reject warm-up rows and aggregate only after checking the complete case/rate/count matrix and finite numeric fields.

The cost estimator matches EVD-0003. The timing/RSS aggregation is stated here because the harness emits per-render
summaries: medians represent the typical p50/p99, the largest `block_max` retains the worst observed full block, and
signed mean RSS deltas avoid rewriting page reuse as zero allocation.

## Commands

```text
python3 plans/v2/evidence/phase-00a/collect_evd_0007.py \
  --output-dir plans/v2/evidence/phase-00a \
  --cpu-list 10,11
```

The retained harness contains every render and compare argument, validates the different-case control, checks every
same-case metric, captures the exact JSONL emitted by both measurement binaries, rejects incomplete matrices, and
derives all three CSV artifacts in a temporary staging directory. It publishes the complete CSV set only after every
collection and validation succeeds, using one Linux `renameat2(RENAME_EXCHANGE)` operation to switch the stable
`EVD-0007-artifacts/` directory from the old generation to the new one. The two raw JSONL files remain outside the
switched directory and inside temporary staging, so they are never published as evidence. The permanent aggregate
tables and harness are retained together. The command above repeats the recorded machine's affinity. On another Linux
host, omit `--cpu-list` to select up to two CPUs from the process's allowed affinity set, or provide a list allowed by
that host's cpuset.

## Results

The comparison control, CORPUS-0006 against CORPUS-0008, reported non-identical files and samples with the first
sample divergence at frame 1 and no warnings.

All ten same-case comparisons passed. Each pair of independently rendered WAVs had identical file and sample data,
identical output SHA-256 digests, zero numeric deltas, and no render or comparison warnings. The retained determinism
table records the input digest, output digest, and frame count for every case. One recorded frame count is not
`(seconds + tail) x rate`: CORPUS-0009 renders 198,438 frames, 12 short of the declared 4.5 s at 44.1 kHz, because
V1's seconds-to-span conversion quantizes through the authored tempo map. The recorded count, not the declared
seconds, is the baseline a comparison must match.

The CPU run produced 2,000 retained observations: 10 cases × 4 rates × 50 renders. Every cell contains ten rounds
and exactly 50 recorded renders. Across the 40 cells, the minimum-cost column ranges from 0.9697 ms/rendered second
for CORPUS-0009 at 44.1 kHz to 33.4045 ms/rendered second for CORPUS-0002 at 192 kHz. The largest cell arithmetic
mean is 35.2762 ms/rendered second for that same 192 kHz CORPUS-0002 cell.

The timing/RSS run produced 600 retained observations: 10 cases × 4 rates × 15 renders. Every cell uses 256-frame
blocks and contains exactly 15 recorded renders. The largest observed full-block time relative to its offline block
budget is 8.43%, for CORPUS-0002 at 192 kHz. Cell mean final RSS ranges from 13.93 MiB to 30.66 MiB; the largest cell
mean peak RSS is 31.02 MiB. These timing ratios describe the controlled offline run only and are not a live deadline
or headroom claim.

## Interpretation

The current executable corpus has a deterministic V1 reference for every case and a complete, reviewable CPU,
full-block timing, and RSS measurement matrix at the four declared sample rates. EVD-0001 through EVD-0003 remain
valid for their historical fixture population; this record supplies the missing current-corpus coverage rather than
retroactively changing those inputs.

The results support later V1/V2 comparisons. They do not establish a portable performance threshold, an allocation
bound, or live-device safety.

## Limitations

- One Linux laptop and one run date; the numbers do not characterize other operating systems or processors.
- Offline execution has no device, callback jitter, competing audio thread, or hard deadline. Timing is a V1 workload
  baseline, not a live-headroom guarantee.
- RSS is page-granular process state, not allocator accounting. A phase delta is an upper bound on retained pages and a
  lower bound on bytes touched; page reuse couples later cases to earlier cases in one invocation.
- The sampler category remains unmeasured until Phase 0B supplies a deterministic project-plus-asset bundle.

## Conclusion

**Supported.** The expanded ten-case corpus satisfies the predeclared determinism and measurement-completeness
criteria. P00A-T003 and the Phase 0A baseline gate have reviewable evidence for the current corpus.

## Artifacts

| Artifact | Location/digest | Retention or reproduction |
|----------|-----------------|---------------------------|
| Determinism results | [`EVD-0007-determinism.csv`](EVD-0007-artifacts/EVD-0007-determinism.csv) | Permanent, in the atomically switched artifact directory |
| CPU aggregates | [`EVD-0007-cost.csv`](EVD-0007-artifacts/EVD-0007-cost.csv) | Permanent, in the same generation |
| Timing/RSS aggregates | [`EVD-0007-timing-memory.csv`](EVD-0007-artifacts/EVD-0007-timing-memory.csv) | Permanent, in the same generation |
| Collection and aggregation harness | [`collect_evd_0007.py`](collect_evd_0007.py) | Permanent; the command above reproduces the complete run and its raw intermediates |
