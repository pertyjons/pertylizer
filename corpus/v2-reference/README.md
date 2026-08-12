# Sound Core V2 reference render corpus

The fixed set of V1 renders that Sound Core V2 is measured against. It is the
deliverable of task **P00A-T001** in
[`plans/v2/phases/phase-00a-baseline-and-render-contracts.md`](../../plans/v2/phases/phase-00a-baseline-and-render-contracts.md).

No audio is stored here. A case names an input project and the exact settings
that render it, so any revision can regenerate the reference bytes; what is
pinned is the *input*, by SHA-256, because a corpus whose inputs drift silently
rebaselines every measurement taken against it.

## Layout

```text
corpus/v2-reference/
├── README.md        # this file
├── manifest.json    # the machine-readable corpus
└── projects/        # generated fixture projects
```

## Reading the manifest

Each entry in `cases` carries:

| Field                     | Meaning                                                              |
|---------------------------|----------------------------------------------------------------------|
| `id`                      | Stable `CORPUS-NNNN` identifier. Cite this, not the title.           |
| `category`                | Which of the master plan's bounded-corpus categories the case covers. |
| `input`, `sha256`         | The project, relative to this directory, pinned by content.          |
| `render`                  | Sample rate, bit depth, window, tail, and mix selection.             |
| `seed`                    | External PRNG seed. Required key; `null` on every case — see below.  |
| `determinism`             | What two renders by one build must produce.                          |
| `preserve`                | Behaviour V2 must keep, each with a comparison class.                |
| `change`                  | Behaviour V2 is intended to alter, each with a class and a rationale. |

`class` is one of the four categories of the master plan's audio comparison
policy — `exact-parity`, `feature-parity`, `intentional-correction`,
`unsupported-scope`. A `preserve` claim may only carry one of the two parity
classes and a `change` claim only one of the other two; the loader refuses a
manifest that mixes them, because "we preserve this intentional correction"
reads as reasonable and means nothing.

`planned` lists the categories with no case yet, each with the reason. Every
category is either covered or listed there — the loader refuses a manifest where
one is neither, so the corpus cannot quietly lose coverage as it grows.

### Why no seeds

V1 has no render-level seed. Every random-family module derives its state from
its voice index and module instance number, so a render is reproducible without
one. The field exists so that a V2 introducing an explicit seed cannot change
what a case renders without changing the case.

The key is nevertheless **required**, and every case spells it `null`. Omitting
it is a parse error, not a shorthand: with an optional key, "this case has no
seed" and "somebody forgot to write one" would be the same file, and the point of
the field is to record the answer rather than leave it to be inferred from an
absence.

## Regenerating

```bash
cargo run -p pertylizer --bin gen_corpus
```

This rewrites every project under `projects/` from the builders in
`crates/pertylizer/src/corpus/fixtures.rs` and refreshes each case's `sha256`.
It owns the digests and nothing else: titles, purposes, render settings, and
claims are written by hand, because they are judgements rather than derivations.

```bash
cargo test -p pertylizer --test corpus_manifest
```

then checks that the committed files are exactly what the builders produce and
that every digest still matches. Note the `--test` selector: a bare
`cargo test --workspace corpus` filters by *test name*, which matches only the
unit tests under `corpus::` and runs none of these integration tests.

## Rendering a case

```bash
cargo run --release -- render \
  --input corpus/v2-reference/projects/subtractive-voice.ptz \
  --output /tmp/CORPUS-0001.wav \
  --sample-rate 44100 --bit-depth 32f --seconds 2 --tail-seconds 1 \
  --result-json /tmp/CORPUS-0001.receipt.json
```

The arguments are the case's `render` block, field for field. The receipt's
`output.sha256` is what answers the `determinism` claim: two renders of a case by
one build must produce the same digest.

## Comparing two renders

```bash
cargo run --release -- compare \
  --reference /tmp/CORPUS-0001.v1.wav \
  --candidate /tmp/CORPUS-0001.v2.wav \
  --result-json /tmp/CORPUS-0001.comparison.json
```

The report carries peak and RMS error, onset offset and envelope alignment,
fundamental frequency and pitch drift, envelope landmarks, stereo correlation and
per-channel energy, per-octave-band spectrum differences, integrated loudness, and
both files' SHA-256. Every `delta` is candidate minus reference.

It reports no verdict, and that is deliberate: whether a difference is acceptable
depends on the case's `preserve` and `change` claims and on the decision behind
them, so the judgement belongs in the evidence record that cites both, not in a
tolerance compiled into the tool. A metric that could not be computed is absent
from the report rather than zero, with a `warnings` entry saying which
precondition failed — an absent section and a section of zeroes mean opposite
things.

## Adding a case

1. Add a builder to `crates/pertylizer/src/corpus/fixtures.rs` and an entry to
   `FIXTURES`.
2. Add the case to `manifest.json` with a zeroed `sha256`, and remove the
   matching entry from `planned` if the category was listed there.
3. Run the generator, then `cargo test -p pertylizer --test corpus_manifest`.

A case that points at an existing project instead of a generated fixture needs
only steps 2 and 3 — the generator refreshes its digest and leaves the file
alone.
