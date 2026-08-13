# EVD-0004: CORPUS-0005's claims against their counterfactuals

| Field         | Value                                      |
|---------------|--------------------------------------------|
| ID            | EVD-0004                                   |
| Status        | Complete                                   |
| Phase         | 00A                                        |
| Created       | 2026-08-13                                 |
| Last reviewed | 2026-08-13                                 |
| Retention     | Permanent                                  |
| Related       | P00A-T001, CORPUS-0005, EVD-0001, ADR-0021 |
| Superseded by | —                                          |

Permanent for two reasons. It is the only record that CORPUS-0005 tests what it
says it tests, and it is where the corpus-wide consequence was found: the
oscillator randomizes phase on every note-on by default, from a generator seeded
by the voice index, so every fixture pinned a variable none of them intends.
P00A-T001 has since turned it off in all five.

## Question or hypothesis

Does [CORPUS-0005](../../../../corpus/v2-reference/manifest.json) distinguish the
behaviours it claims, and by how much?

The question exists because a corpus case can be **vacuous** — well formed,
rendering plausible audio, and exercising none of the behaviour its claims name.
That is not hypothetical in this corpus. CORPUS-0002 was authored to force voice
stealing and, until the offline renderer began replaying allocator settings,
rendered with the default eight voices and stole nothing; every structural test
passed throughout.

## Acceptance criteria

Fixed before the renders were taken:

- **Each claim needs a counterfactual** — a variant that differs only in the
  behaviour the claim names — and the two renders must differ measurably. A
  claim whose counterfactual measures zero is one the case does not test.
- **A null control decides whether the counterfactual means anything.** The same
  construction applied to a project with the behaviour absent must measure
  nothing. This criterion is the one that did the work here.
- **The difference must be attributable.** A variant that changes two things at
  once establishes nothing about either.
- **A negative result is recorded, not retried until it passes.**
- **Determinism** is EVD-0001's criterion applied to the new case: two renders
  in separate processes produce the same receipt digest.

## Source and environment

- Source revision: `15745328` on `main`, plus the change that adds the fixture
  and the case.
- Platform: Linux 7.1.7-200.fc44.x86_64, x86_64, 13th Gen Intel Core i7-13700H.
- Rust/tool versions: rustc 1.97.1 (8bab26f4f), cargo 1.97.1 (c980f4866).
- **Build profile: `dev`.** These are correctness measurements, not cost ones.
- Render settings: the case's own `render` block, field for field — 44 100 Hz,
  `32f`, 2.0 s window, 2.0 s tail. No audio device and no GUI.

## Inputs

`corpus/v2-reference/projects/instrument-inserts.ptz` and variants built from it
by editing the loaded JSON. Each changes exactly one thing:

| Variant group      | Change from the committed project                               |
|--------------------|------------------------------------------------------------------|
| `reversed`         | `effect_chain_order` becomes `["dly-1", "dst-1"]`                |
| `dry`              | Both insert modules and the chain order removed                  |
| `full-*`           | Notes reduced to the dyad, to pitch 48 alone, or to pitch 55 alone |
| `dist-*`           | `dly-1` removed, notes as above                                  |
| `dly-*`            | `dst-1` removed, notes as above                                  |
| `none-*`           | Both inserts removed, notes as above — **the null control**      |
| `none-dyad-swapped`| Both inserts removed; the two notes reversed in the note list    |

The variants are not committed. They are derivable from the committed input by
the edits above.

## Method

1. Render the committed input twice, in separate processes, for determinism.
2. Render `reversed` and `dry`; compare each against the committed render.
3. Measure windowed RMS of the committed render and of `dry`, locating the delay
   tail against the final note-off at 1.125 s.
4. Render the dyad, and each of its notes alone, through four chains: both
   inserts, the clipper alone, the delay alone, and **nothing**. Sum each pair of
   single-note renders and compare against the corresponding whole-dyad render.
   Summing two single-note renders is what a per-voice chain would produce; the
   empty chain is the control that says whether the construction measures
   anything at all.

## Results

### Determinism

Two renders of the committed input in separate processes: receipt digests
identical (`c4d3671bf47395ac…`). `determinism: bit-exact` holds.

### The construction, and its control

Dyad rendered whole against its two notes rendered separately and summed, over
the dyad window:

| Chain in the instrument   | Dyad whole   | Singles summed | Difference    | Relative | max sample |
|---------------------------|-------------:|---------------:|--------------:|---------:|-----------:|
| Distortion and delay      | −22.05 dBFS  | −19.28 dBFS    | −24.93 dBFS   | −2.88 dB | 3.5e−01 |
| Distortion only           | −18.51 dBFS  | −15.76 dBFS    | −21.57 dBFS   | −3.07 dB | 7.1e−01 |
| Delay only                | −37.39 dBFS  | −37.31 dBFS    | −72.60 dBFS   | −35.21 dB | 9.6e−03 |
| **Nothing — the control** | −34.77 dBFS  | −34.77 dBFS    | **−181.78 dBFS** | **−147.00 dB** | 3.0e−08 |

The control is floating-point rounding, 147 dB below the render. The
construction is therefore valid, and each row above it is attributable to the
chain it names.

- **`CORPUS-0005-P1`, the summed-voices claim**, rests on the clipper row:
  −3.07 dB relative, 144 dB above the control.
- **`CORPUS-0005-P2`, the shared-state claim**, rests on the delay row:
  −35.21 dB relative, still 112 dB above the control. Two notes can only
  interact inside a delay whose line they share — through the soft clip on its
  feedback write — so a per-voice delay would land at the control.

Reversing the two notes in the pattern's note list renders **bit-identically**,
which is the same property from the other side: with the allocation-order
variable removed, nothing about the render depends on which voice took which
note.

### `CORPUS-0005-P3` — chain state outlives the notes

Windowed RMS against the `dry` control. The final note-off is at 1.125 s.

| Window        | Committed render | `dry` control |
|---------------|-----------------:|--------------:|
| 0.60 – 0.95 s |       −26.2 dBFS |       silence |
| 1.20 – 1.45 s |       −14.8 dBFS |    −64.6 dBFS |
| 1.70 – 1.95 s |       −29.0 dBFS |       silence |
| 2.45 – 2.70 s |       −49.9 dBFS |       silence |
| 3.20 – 3.45 s |       −70.7 dBFS |       silence |

The 0.60–0.95 s window is the gap between the dyad and the isolated note, by
which time every voice has been released.

### `CORPUS-0005-P4` — the authored chain order

| Comparison             | Peak delta | RMS delta |
|------------------------|-----------:|----------:|
| authored vs `reversed` |   +5.46 dB |  +5.97 dB |

## Why this record was rewritten twice

The measurements above are the third attempt. The first two are kept here
because each failure is a reusable lesson and neither is visible in the numbers
that survived.

**Attempt 1 — a probe that guessed at the DSP.** The summed-voices claim was
first probed by measuring the difference tone of the fifth, 196.00 − 130.81 =
65.19 Hz, in neither sawtooth's harmonic series. It measured 7.7 dB above the
control's floor while the render as a whole was 12.2 dB louder — relative to the
signal, *weaker* than in the control. Where a clipper's intermodulation lands
depends on the source spectrum, and sawtooths are already dense: a probe aimed at
one predicted frequency is a guess about the DSP rather than a measurement of the
property.

**Attempt 2 — the summed-versus-whole construction, run without its control.**
It produced a clean-looking +0.67 dB for the clipper. Review then found that the
delay soft-clips its feedback write, so an assumed linearity was false; chasing
that produced the isolation runs, and those produced the **null control, which
came out at −1.41 dB — the same size as the effect.** The record concluded that
V1's renders are not additive across polyphony, that the sample a note sounds on
depends on its position in the note list, and that the construction was
disqualified. Two claims were withdrawn and an intentional-correction claim was
written against a sequencer defect.

**None of that was a sequencer defect.** A second review found the cause:
`Oscillator`'s `uni_phase` parameter defaults to 1.0 — the field is initialized
to `NormalizedValue::MAX` and the descriptor's default is 1.0 — and
`set_voice_index` seeds the generator behind it from the voice index. Each
allocated voice therefore starts a note at a different phase. In a solo render
the note takes voice 0; in the dyad the second note takes voice 1. That is the
entire 46.55° phase difference the record had measured, and it explains why the
implied offset was not a constant number of samples: it was never a delay.

With `uni_phase` set to 0 in the fixture, the control drops from −1.41 dB to
−147 dB and both withdrawn claims are re-established, stronger than before.

## Interpretation

**The construction works, and it needed one parameter turned off.** The lesson
is not that null controls are prudent — it is that a null control is the only
thing that distinguishes a measurement from an artifact, and that a default
parameter three modules away can be the artifact.

**The other four fixtures carried the same default, and P00A-T001 has since
turned it off in all five.** The generator is not only seeded by voice index —
`next_unit` advances it, so every note-on draws a fresh phase. Each fixture
therefore pinned a phase *sequence* that depends on which voice took which note
and on how many note-ons preceded it, which is the variable the module header in
`corpus/fixtures.rs` says a fixture avoids. A V2 with an equivalent but
differently-indexed allocator would change it without changing any behaviour a
case claims.

The change cost four committed digests. EVD-0001 records the superseded input
digests in full and re-asks its determinism question at `--release` — all five
cases still render bit-identically across two processes. EVD-0003 records a
re-measurement at its own 50 draws per case: the pooled minimum moves −4.0%,
while a first three-round check had reported +2.2%, and resampling puts
small-sample bias at only +0.14%. Neither figure supports a cost claim about the
fixture change; what dominates is session-to-session variation on an unquiesced
machine.

**`CORPUS-0005-C2` is withdrawn.** It claimed that simultaneous note-ons should
sound at the same sample and that rendered audio should not depend on note-list
position. The first half was never measured — there is no timing defect here —
and the second is the documented consequence of a parameter the case now sets.
A migration contract written against an artifact is worse than no contract.

## Limitations

- **Six of the eleven categories still have no case**, and this record says
  nothing about them.
- **`dev` profile.** Float arithmetic may differ between profiles, so these
  figures are not asserted to hold bit-exactly against a `--release` render.
  Every comparison is between renders of the same profile in one session.
- **The per-voice model is a model.** Summing two single-note renders reproduces
  what a per-voice chain would produce for *this* patch, where the only
  nonlinearities are the two inserts. It is not a general statement about how a
  V2 per-voice chain would be built.
- **One machine, one platform, one sample rate**, and one patch.
- **The counterfactuals are hand-built variants**, not fixtures, and are not
  committed. Nothing automated re-derives them, so a future change to the
  fixture would not invalidate this record automatically.
- **The phase-randomization finding was not quantified per fixture.** That the
  other four carried the default is read from the code and from this case's
  behaviour; no per-case measurement of what it changed was taken before they
  were regenerated.

## Conclusion

**Supported.** All four of CORPUS-0005's preserve claims have a counterfactual
that measurably differs, each against a null control at −147 dB relative:

- `P1`, summed voices: −3.07 dB relative with the clipper isolated;
- `P2`, shared chain state: −35.21 dB relative with the delay isolated, 112 dB
  above the control;
- `P3`, state outliving the notes: a tail 2.3 s past the last note-off against a
  silent control;
- `P4`, authored chain order: 5.97 dB RMS.

`determinism: bit-exact` holds across processes, and with phase randomization
off the render no longer depends on note-list order.

**Acted on**: all five fixtures now render with note-on phase randomization off.
Four committed digests changed; EVD-0001 re-verified determinism on the new
bytes and EVD-0003 spot-checked cost.

## Artifacts

| Artifact | Location/digest | Retention or reproduction |
|----------|-----------------|---------------------------|
| Corpus input | `corpus/v2-reference/projects/instrument-inserts.ptz` | Permanent, in-repository; regenerated by `gen_corpus` |
| Case and claims | [`corpus/v2-reference/manifest.json`](../../../../corpus/v2-reference/manifest.json), CORPUS-0005 | Permanent, in-repository |
| Counterfactual recipe | [`corpus/v2-reference/README.md`](../../../../corpus/v2-reference/README.md), *Checking that a case tests what it claims* | Permanent, in-repository |
| Variant projects and WAVs | — | Not retained; rebuild from the *Inputs* table |
