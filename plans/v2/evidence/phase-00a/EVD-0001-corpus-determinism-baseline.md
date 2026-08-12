# EVD-0001: Reference corpus determinism and V1 baseline

| Field         | Value                        |
|---------------|------------------------------|
| ID            | EVD-0001                     |
| Status        | Complete                     |
| Phase         | 00A                          |
| Created       | 2026-08-12                   |
| Last reviewed | 2026-08-12                   |
| Retention     | Permanent                    |
| Related       | P00A-T001, P00A-T002, P00A-T003, ADR-0037 |
| Superseded by | —                            |

Permanent rather than until-phase-exit: the digests below are what a later
comparison is measured against, so they have to outlive the phase that produced
them.

## Question or hypothesis

Two questions, both answerable negatively.

1. **Is the reference corpus deterministic?** Every case declares
   `determinism: bit-exact`. Does rendering a case twice with one build actually
   produce identical bytes?
2. **Does the comparison harness measure anything?** A tool that reported zero
   for every input would satisfy question 1 without doing any work, so the same
   harness must also report a large, correctly-located difference between two
   cases that genuinely differ.

## Acceptance criteria

Fixed before the renders were taken.

- **Determinism.** For each of the four cases, the two passes' output SHA-256
  must be equal, `samples.identical` must be `true`, and every reported delta
  must be exactly `0`. Any non-zero value falsifies the corpus's `bit-exact`
  claim and is a finding, not a tolerance to widen.
- **No warnings.** A case's render and its self-comparison must both come back
  with an empty `warnings` list. A warning means some metric was not computed,
  and an uncomputed metric cannot support a determinism claim.
- **Sensitivity.** Comparing two *different* cases must report `samples.identical
  false`, a band difference above 10 dB, and a pitch interval matching the two
  fixtures' opening notes to within 20 cents. This is the control against a
  harness that reports zeroes unconditionally.

## Source and environment

- Source revision: `b5cdc021` on `main` — the corpus and the harness — on top of
  `f8867990`, the offline-renderer fix the renders depend on. These are the
  squash-merge commits; the branches they came from (`feat/v2-reference-corpus`
  and `fix/offline-render-fidelity`) no longer exist, which is why they are not
  named as the source.
- **Why this field was filled in only at merge.** A squash-merge replaces every
  commit on a branch with one new commit, so any hash written before the merge is
  scheduled for deletion by the merge that lands the document citing it. Two
  earlier revisions of this record cited hashes a rebase had already orphaned.
  The hashes above are stable because they are on `main`. The input digests under
  *Inputs* remain the provenance that survives even a history rewrite.
- Corpus revision: `corpus/v2-reference/manifest.json`, `manifest_version` 1,
  four cases, input digests as listed under *Inputs*
- Platform and architecture: Linux 7.1.7-200.fc44.x86_64, x86_64
- Rust/tool versions: rustc 1.97.1 (8bab26f4f), cargo 1.97.1 (c980f4866)
- Build profile: `--release`
- Audio configuration: 44 100 Hz, stereo, 32-bit float, per each case's `render`
  block
- Feature flags: workspace defaults (`gui-egui`, `mcp`, `osc`)
- Host or device simulation: none. Nothing here opens an audio device or a
  window, which is the Phase 0A exit gate's requirement.

## Inputs

The four corpus cases, each pinned by the digest of its input project. The
digests are the durable identity here — unlike the commit hashes above, they
survive any rebase or squash:

| Case        | Input                                    | Input SHA-256 |
|-------------|------------------------------------------|---------------|
| CORPUS-0001 | `projects/subtractive-voice.ptz`         | `d3fd763fddd3a373d83548d25d89087e864584e9000128497d73f2c7cdbca85d` |
| CORPUS-0002 | `projects/polyphonic-voice-stealing.ptz` | `539c71be4e7fa74d15f9ffa9a52c40c84499dc24a3d50fc990b1cfefe7681449` |
| CORPUS-0003 | `projects/mod-matrix.ptz`                | `baeff623168808cbd078ffe3587e87a0687a058acb1f67bb536e45df3b41b5e5` |
| CORPUS-0004 | `projects/sends-returns-master.ptz`      | `6e0cba6438a230b5f788fc3a1f4bb0b446e9792ef408e86a2c5f6c0ac090d07d` |

No seeds. V1 has no render-level seed; every random-family module derives its
state from its voice index and module instance number, and no corpus fixture
uses one in any case.

## Method

For each case, in one process each:

1. render the case twice from the same input at the same settings, writing a WAV
   and a receipt per pass;
2. compare the two renders with `pertylizer compare`;
3. record the output digests, the receipt's frame count and peak, and every
   delta the comparison reports.

Then the control: compare CORPUS-0001's first pass against CORPUS-0003's.

The two passes are separate process invocations rather than two calls inside
one, so the result covers process-level state as well as engine-level state.

## Commands

```text
# per case, twice
./target/release/pertylizer render \
  --input corpus/v2-reference/projects/<input> \
  --output <case>.pass<n>.wav --result-json <case>.pass<n>.json \
  --sample-rate 44100 --bit-depth 32f \
  --seconds <seconds> --tail-seconds <tail_seconds>

# per case
./target/release/pertylizer compare \
  --reference <case>.pass1.wav --candidate <case>.pass2.wav \
  --result-json <case>.compare.json

# control
./target/release/pertylizer compare \
  --reference CORPUS-0001.pass1.wav --candidate CORPUS-0003.pass1.wav \
  --result-json control.json
```

Settings per case are the manifest's `render` block, field for field.

## Results

### Determinism

Every case: both passes produced the same output digest, `samples.identical`
was `true`, and every delta was exactly `0.0` — peak, RMS, onset, envelope lag,
pitch interval, maximum band difference, and loudness. Neither the renders nor
the comparisons emitted a warning.

### V1 baseline, first pass

| Case        | Output SHA-256 | Frames | Peak (dBFS) | RMS (dBFS) | Loudness (LUFS) | Fundamental (Hz) |
|-------------|----------------|--------|-------------|------------|-----------------|------------------|
| CORPUS-0001 | `1b5a9d918676ffc2b9bc0b6820e542feef58a5e981afe74707925889b9b9c22d` | 132 300 | -13.80 | -25.09 | -21.46 | 220.0 |
| CORPUS-0002 | `2fcf8f06519225050875f004c3259f44b4358da5593ae0ee1fd7e764d9a790dc` | 176 400 | -6.83  | -20.43 | -16.44 | 130.8 |
| CORPUS-0003 | `98adc4446edc8c51714a0a961011f964def9cb870938c5390ac5c611cde6390c` | 132 300 | -13.17 | -22.69 | -19.36 | 109.8 |
| CORPUS-0004 | `2fb238e69f7b511c33bbf4719a5a14aa475165a2d910762bd542a3098a4af8f2` | 176 400 | -11.50 | -32.75 | -26.82 | 263.2 |

The fundamental is measured over the head of the render, not the whole of it, so
it is the opening note. Each matches: A3 (220.00), C3 (130.81), A2 (110.00), and
C4 (261.63) — the last within the resolution its 240-tick notes allow.

### Control

CORPUS-0001 against CORPUS-0003:

| Metric | Value |
|--------|-------|
| `files_identical` | `false` |
| `samples.identical` | `false`, first divergence at frame 0 |
| `samples.error_to_signal_db` | +4.17 dB |
| Peak / RMS delta | +0.63 dB / +2.40 dB |
| Largest band difference | 50.51 dB, in the 80-160 Hz band |
| Band difference, RMS over bands | 21.13 dB |
| Pitch interval | -1203.6 cents |
| Loudness delta | +2.10 LU |

The pitch interval is the assertion that matters here: the two fixtures open on
A3 and A2, one octave apart, and an octave is 1200 cents. The harness recovered
it to within 3.6 cents from audio alone.

### Re-verification after a harness change

Two later passes on the same branch changed what the comparison harness reports,
without changing what it renders. First, `envelope.max_delta_db` gained a
companion figure gated to windows above a reported level floor, and
`envelope.correlation` became optional rather than being computed from ripple on
a flat envelope. Then review turned the stereo correlation and the two
fundamentals optional for the same reason, split the ungated and gated envelope
maxima into separate fields, and made the manifest's `seed` key required.

Every case was rendered and compared again after each. Nothing above moved: all
four output digests are identical to the table, every delta is still exactly
zero, no case warns, and the correlation is present and 1.0 on all four — the
fixtures all have envelope structure. The level floor does exclude silence, as
intended: it sits 60 dB below the louder peak and is reported in every result, so
a reader can see the threshold rather than infer it. Of CORPUS-0001's 300
envelope windows 213 clear it, and of CORPUS-0004's 400, 253 — the rest are the
reverb tail decaying past it.

The manifest changed shape in the second pass — `seed` is now written
explicitly as `null` on every case — but the fixture projects it names did not,
so the input digests in the table above are unchanged and the renders are the
same bytes.

## Interpretation

Both questions are answered.

The corpus is deterministic at this revision, at the level its cases claim —
byte equality of the rendered file, not merely audible similarity. That makes
the corpus usable as a baseline: a future difference against these digests is a
change in the renderer, not noise in the measurement.

The harness measures. The control's octave, located to 3.6 cents, and its 50 dB
band difference placed in the band where the two fixtures' fundamentals actually
sit, are not results a stub could produce.

The four rows above are the V1 side of the eventual V1-versus-V2 comparison for
these cases. They are level and loudness figures only; they are **not** the CPU,
memory, or timing baselines P00A-T003 requires, and they do not close it.

## Limitations

- **One platform, one build.** Linux x86_64, one rustc, one release profile. The
  digests are not claimed to be portable: floating-point differences across
  targets are exactly the kind of thing that would move them, and nothing here
  tests for it. A cross-platform determinism claim needs its own record.
- **Two passes, not many.** Two identical renders demonstrate determinism far
  more weakly than a hundred would. Two was chosen because the plausible failure
  mode — uninitialized or time-dependent state — shows up on the second pass or
  not at all; a scheduling-dependent race would not be reliably caught by either.
- **Four of eleven categories.** Seven of the master plan's corpus categories
  have no case yet, each recorded as a gap in the manifest. Nothing here says
  anything about sampler, YAMS, stereo/spatial, insert, tempo-map, or
  shared-instrument determinism.
- **A digest is not a hearing.** No corpus render has been listened to. The
  fixtures are asserted to be non-silent and to have measurable pitch, envelope,
  and spectrum, which is not the same as being musically sensible.
- **These numbers postdate a renderer fix.** See below.

## The defect this work found

The first attempt at this measurement was taken against a renderer that did not
render what the live engine plays. `OfflineEngineSession` rebuilt each instrument
without an `AllocatorConfig` and replayed only volume, pan, and solo, so
`max_voices`, `allocation_mode`, `stealing_strategy`, the unison pair,
`transpose`, `key_range`, `oversampling`, both velocity sensitivities, and the
sidechain source were left at engine defaults. Nothing warned: the values were
not lost, they were never sent.

Demonstrated by editing a fixture and rendering it: `max_voices` 4 against 1
produced a byte-identical WAV, and so did `transpose` 0 against 12. Both differ
now.

For this corpus the consequence was specific and serious. CORPUS-0002 renders
eight overlapping notes against a four-voice instrument; before the fix it
rendered against the default eight and stole nothing, so its preserve claims
about voice stealing would have been satisfied by a V2 with any allocator at all.
Rendered at 2 s plus a 1 s tail on both sides of the fix, its output digest moved
from `db63a459631e33652b16824da7dca58fcdf1288eb80f5a3dce34b3222d128d32` to
`7e9916d2450cc2d02b3ccbff244432d92246e268ffce4145909fdf22201eb4e8` — the audio
changed because voices are now actually stolen. CORPUS-0001 and CORPUS-0003 are
unaffected: their fixtures leave every instrument setting at its default, which
is precisely why the defect stayed invisible for so long.

The fix and its regression tests are the `fix/offline-render-fidelity` branch;
the baseline above is taken after it. Every consumer of the offline renderer was affected, not only
the corpus — `analyze_mix_bus`, `analyze_section`, and the WAV export all
measured audio the live engine never produced.

## Conclusion

**Supported.** The corpus is bit-exact reproducible on this platform at every
revision of these two branches, and the comparison harness resolves a real
difference correctly.

Gate impact: this closes P00A-T002 and supplies the first half of the Phase 0A
exit gate's first bullet — the corpus and the comparison command run with no GUI
and no audio device.

It does **not** close P00A-T001. That task is the master plan's eleven corpus
categories, and four are covered; what this record demonstrates is that the four
are sound, not that the corpus is complete. It does not close P00A-T003 either,
which needs CPU, memory, and timing baselines this record does not measure, and
it does not settle ADR-0037, whose proxy measurement is defined over this corpus
but not taken here.

## Artifacts

| Artifact | Location/digest | Retention or reproduction |
|----------|-----------------|---------------------------|
| Corpus inputs | `corpus/v2-reference/projects/`, digests in the manifest | Permanent, in-repository |
| Rendered WAVs, 8 files | Digests in the results table above | Not retained — regenerate with the commands above at any revision of these two branches; the digests are unchanged by every revision since the renderer fix |
| Render receipts and comparison reports, 13 files | — | Not retained; every number they carry that matters is transcribed above |

The audio is deliberately not committed: `plans/v2` is for reviewable documents,
and the digests are what identify these renders anyway.
