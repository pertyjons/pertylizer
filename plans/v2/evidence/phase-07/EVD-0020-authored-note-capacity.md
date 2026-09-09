# EVD-0020: Real Note YAMS Producer Capacity

| Field | Value |
|---|---|
| ID | EVD-0020 |
| Status | Complete |
| Phase | 07 |
| Created | 2026-09-09 |
| Last reviewed | 2026-09-09 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `d632f874a6a76c728a5b5fb7cb4a8da2a9138a10` |
| Retention | Permanent |
| Related | ADR-0054, ADR-0060, SOUND-INV-030, P07-S007 |
| Artifacts | [CSV](artifacts/EVD-0020-results.csv), [run and environment](artifacts/EVD-0020-run.txt), [subject bundle](artifacts/EVD-0020-subject.bundle), [SHA256](artifacts/EVD-0020-SHA256SUMS.txt) |

## Question and falsifier

Can the real finite Note YAMS producer retain the provisional authored share
for its explicitly admitted input envelope? Supported requires observed maximum
authored occupancy equal to its conservative 2N bound, no share borrowing,
retained/hold peaks at most their declarations, zero allocation during first
render, and the partition/terminal controls passing. A violation is Not
supported. Missing committed provenance or controls is Inconclusive.

## Inputs and controls

The unit harness `authored::tests::evd_0020_authored_capacity` uses a 48 kHz mono
4096-frame harness profile, one shared Note script, a Control counter and a
voice sine/envelope/amplifier graph. Project seed is 99 and Note state ID is 42;
occurrences are explicitly numbered, independently of the voice allocator.
N is authored share / 2. Cases are empty, script drop, same-batch on/off,
future held release, until-cut, retained inputs, and staggered random pitch.
The empty case runs first and asserts zero audio and every occupancy peak zero;
it is what the method reports when no input is transformed. The same graph and
profile are used throughout. Programs differ by case, so timing differences
are not estimates of isolated producer overhead.

Before retained measurements, run the ordinary unit controls: exact same-batch
versus held-release accounting, local automation and signal-capture oracle,
whole/Q/one-frame/irregular seeded audio and traces, whole-source N+1 refusal,
forged late/overfull source terminal silence, and allocation at the maximum.
The shared VM's existing non-finite StoreOut-to-zero rule is tested separately
from a forged invalid mapping's terminal response.

## Method

Run the release-profile ignored test on a committed subject. Each case prepares
101 independent streams off-clock, times one complete 4096-frame render with
Instant, and retains median and maximum nanoseconds. Time includes the actual
Q-sized subdivision, publication passes, VM, graph DSP and output copies; it
excludes preparation. Every draw must have identical source high-water counts.
A separate first render per case uses the counting allocator and requires zero
allocator events. Occupancy, rather than a timing comparison, decides selection.
Run order and draw count are fixed; no historical minimum is a comparator.

## Reproduction

```text
cargo test -p synth_engine_v2 --lib authored::tests
cargo test -p synth_engine_v2 --release --lib authored::tests::evd_0020_authored_capacity -- --ignored --exact --nocapture
```

The bundle retains the exact committed subject relative to main parent
`d871d70d314dafe32c0ca854356e699f770d5258`. From a clone containing that parent:

```bash
cd plans/v2/evidence/phase-07/artifacts
sha256sum -c EVD-0020-SHA256SUMS.txt
git bundle verify EVD-0020-subject.bundle
git fetch ./EVD-0020-subject.bundle HEAD
cd ../../../../..
git worktree add --detach /tmp/evd-0020-reproduce FETCH_HEAD
cd /tmp/evd-0020-reproduce
cargo test -p synth_engine_v2 --release --lib authored::tests
cargo test -p synth_engine_v2 --release --lib authored::tests::evd_0020_authored_capacity -- --ignored --exact --nocapture
```

The normal qualification flag changes after measurement; producer and harness
bytes remain identical to the retained subject. The run started from a clean
worktree. Both commands exited zero: 16 ordinary controls and the ignored harness.

## Results

| Case | Inputs | Pending | Holds / future | Reservations | Evaluations/Q | Authored/Q | Releases/Q | Allocations | Median ns | Max ns |
|---|---|---|---|---|---|---|---|---|---|---|
| empty | 0 | 0 | 0 / 0 | 0 | 0 | 0 | 0 | 0 | 1976474 | 2304628 |
| drop | 24 | 24 | 0 / 0 | 24 | 24 | 0 | 0 | 0 | 1963507 | 3175499 |
| same_batch | 24 | 24 | 0 / 0 | 24 | 24 | 48 | 0 | 0 | 1948564 | 2058452 |
| held | 24 | 24 | 24 / 24 | 24 | 24 | 24 | 24 | 0 | 1937871 | 2009143 |
| until_cut | 24 | 24 | 24 / 24 | 24 | 24 | 24 | 24 | 0 | 1945226 | 2055017 |
| retained_inputs | 24 | 24 | 0 / 0 | 24 | 0 | 0 | 0 | 0 | 1912157 | 2039455 |
| staggered | 24 | 24 | 7 / 7 | 24 | 1 | 1 | 1 | 0 | 1965538 | 2045986 |

Each case has 101 timed draws. The nonempty source storage is 2,928 bytes;
the empty source owns 48 bytes of tempo storage. These figures report the source's
occurrence and tempo allocation, not the graph/renderer's separately admitted storage.
All first-render allocator counts are zero. The empty control observes zero
occupancy and silence. Same-batch output reaches exactly 48 authored events
(2N), and held/until-cut cases reach 24 holds, future records and releases.
No class borrows another share; the ordinary terminal and N+1 controls pass.

## Limitations

This finite source admits N inputs in total, not a general unbounded song. Its
source scope is shared and it refuses live/compiled note coexistence, stealing,
activation and saved rack/Note Grid lowering. Timing is local to the recorded
environment and does not qualify a production-live partition or hardware host.
Phase 9 still owns ADR-0054 clause 3's simultaneous six-share reselection.

## Conclusion

**Supported for this finite source.** Retain the provisional AuthoredRuntime
share of 48 destination events per quantum, with a maximum of 24 total inputs
under this profile. The measured source stays within N identities/gates/holds
and 2N occupancy/future declarations. This enables the ordinary finite Note
consumer after independent qualification review; it does not reselect the full
six-share partition or qualify production live ingress.
