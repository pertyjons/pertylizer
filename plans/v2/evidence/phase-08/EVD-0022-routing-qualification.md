# EVD-0022: Phase 8 Routing Qualification

| Field | Value |
|---|---|
| ID | EVD-0022 |
| Status | Complete |
| Phase | 8 |
| Created | 2026-09-10 |
| Last reviewed | 2026-09-10 |
| Supersedes | — |
| Superseded by | — |
| Source revision | 22739cba14413b189765c8ac2698845131e91910 |
| Retention | Permanent |
| Related | P08-S008, REV-P08, ADR-0028, ADR-0033, LOWER-INV-003 |
| Artifacts | `EVD-0022-corpus.jsonl`, `EVD-0022-native-digests.txt`, `EVD-0022-SHA256SUMS` |

## Question and falsifier

Does every pinned corpus input receive a deterministic bounded V2 render or a
named refusal after Phase 8, without presenting unsupported scope as parity?
The rule was stated before the run and independently reviewed by `agy` /
`gemini-3.8-flash-high`: all ten inputs must be included, their input hashes
verified, and the admitted population must remain CORPUS-0001, 0003, 0005 and
0009 under the default profile. A changed population requires requalification;
it is not silently incorporated into this conclusion.

Missing input coverage, nonfinite or silent admitted audio, samples returned
alongside refusal, changed samples/events/diagnostics between equivalent callback
partitions, or parity eligibility alongside any diagnostic falsifies the claim.
`Supported` requires all assertions to pass; any failure is `Not supported`, and
an incomplete run is `Inconclusive`. This claim is not whole-project V1/V2 parity.

## Inputs and controls

The unchanged ten project files pinned by
`corpus/v2-reference/manifest.json`, at each case's declared sample rate,
with the default `HostProfile::harness` limits and stereo output. The manifest
verifier checks the input bytes before rendering. The V2 bounded smoke API
renders the complete arrangement plus one second of tail. This is explicitly
not the manifest's independently specified V1 render window, a shared render
request or a new job surface.

The null control, `a_single_placed_note_has_no_missing_stages_and_admits_comparison`,
and the rejection control, `any_diagnostic_denies_a_parity_comparison`, passed
in S007's workspace gate before collection. They distinguish represented input
from a blanket refusal or unconditional parity flag. The corpus test additionally
holds fidelity to the full diagnostic set for every case.

The independent-state control is
`phase8_exit::two_channels_on_one_patch_keep_independent_insert_state_and_tails`:
two copies of the same patch, nonlinear distortion and delay, different faders
and overlapping notes. Their joint output must equal the sum of each isolated
route. Removing only A's inserts must change A and preserve B's original output
as the exact other summand; removing B's delay must eliminate its measured tail.
This is a separate functional check, not a corpus comparison arm.

Environment: Linux 7.1.13-200.fc44.x86_64, x86_64-unknown-linux-gnu,
Rust 1.98.0 (`88d9e12ae178fab0fb5cc050a94da85685d449ea`), development test
profile. Native reference digests use the release profile. No timing estimator,
performance threshold or hardware qualification is selected here.

## Method

The ordinary test in `crates/pertylizer/src/lowering/tests/phase8_exit.rs`
loads the manifest and verifies every pinned input, renders one project at a
time through the existing bounded in-process smoke function, and checks the
256-frame baseline against block sizes 37, 64 and 512. These are four equivalent
V2 callback partitions at the same sample rate, output policy and tail length.
Each render starts from a fresh compiled plan and renderer. Diagnostics and event
counts must agree exactly; output hashes use SHA-256 over each interleaved
sample's `f32::to_bits().to_le_bytes()` in order. Refused cases retain no samples
and no output hash. The emitted `P08_CORPUS` JSON rows retain the complete named
diagnostics, input hash, sample rate, frame/event counts and output hash.

All four admitted cases remain marked. Same-input DSP parity is evaluated
separately by actual V1 module oracles for distortion, delay, compressor,
amplification, terminal processing and master insert order. Channel/send tests
hold V1's arithmetic order directly. Intentional timing differences are named;
no metric is computed over an unsupported whole-project comparison.

## Reproduction

Check out `22739cba14413b189765c8ac2698845131e91910` and run from the repository root:

```bash
cargo test -p pertylizer any_diagnostic_denies_a_parity_comparison
cargo test -p pertylizer a_single_placed_note_has_no_missing_stages_and_admits_comparison
cargo test -p pertylizer phase8_corpus_inputs_render_deterministically_or_refuse_with_named_diagnostics -- --nocapture
cargo test -p pertylizer two_channels_on_one_patch_keep_independent_insert_state_and_tails
cargo test -p pertylizer lowering::tests::phase8
cargo run --release -p synth_engine_v2 --example quantum_cost -- 1 10
```

Retain each complete line beginning with `P08_CORPUS ` after removing that
prefix, in emitted manifest order. The ordinary test fails if any partition or
population assertion fails. Verify the retained artifacts from this directory:

```bash
sha256sum -c EVD-0022-SHA256SUMS
```

The three reference hashes come from S007 `90677d50`, the last native render-path
change. S008 changes only qualification tests and diagnostic owner text. The
small `quantum_cost` run checks hashes against EVD-0012; its timing output is
excluded and is not evidence of a performance improvement or regression.

## Results

| Case | Result | Frames | Parity eligible |
|---|---|---:|---|
| CORPUS-0001 | Rendered with note-lifetime marker | 132300 | No |
| CORPUS-0002 | Refused | 0 | No |
| CORPUS-0003 | Rendered with modulation timing and note-lifetime markers | 132300 | No |
| CORPUS-0004 | Refused: uncarried return reverb | 0 | No |
| CORPUS-0005 | Rendered with note-lifetime marker | 132300 | No |
| CORPUS-0006 | Refused | 0 | No |
| CORPUS-0007 | Refused | 0 | No |
| CORPUS-0008 | Refused | 0 | No |
| CORPUS-0009 | Rendered with markers | 161700 | No |
| CORPUS-0010 | Refused | 0 | No |

The [raw rows](EVD-0022-corpus.jsonl) name every diagnostic rather than fold it
into these summaries. All ten cases agree across the four block sizes. The
independent insert-state and tail check passes. All three reference native audio
hashes reproduce; [the retained digest list](EVD-0022-native-digests.txt) gives
their full values.

## Limitations

Four rendered cases and six refusals qualify the current bounded lowerer; they
do not establish complete corpus parity, every conceivable graph, cross-platform
floating-point identity, or a performance ceiling. The one-second caller-selected
tail is not proof that all effects have decayed. Native unknown/recurring tails
remain explicitly unknown. Physical hardware timing and oversampling are outside
this method. ADR-0028/P04-R004 still block the shared V1/V2 surface until Phase
10B; unsupported verdicts remain forbidden under LOWER-INV-003.

## Conclusion

Conclusion: Supported

The bounded corpus path is deterministic across the four tested host partitions,
and no retained outcome presents an unsupported input as parity. Together with
the direct DSP, routing, latency, feedback and RT checks mapped by
[REV-P08](../../reviews/phase-08-exit-review.md), this supplies S008's qualification.
It selects no capacity and discharges no deferred whole-project A/B obligation.
