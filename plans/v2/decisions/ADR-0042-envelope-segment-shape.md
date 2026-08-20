# ADR-0042: V2's envelope segments are exact and linear, and the shape difference is CORPUS-0001-C2

| Field | Value |
|---|---|
| ID | ADR-0042 |
| Status | Proposed |
| Phase | 2 |
| Created | 2026-08-20 |
| Last reviewed | 2026-08-20 |
| Related | EVD-0013, ADR-0040 clauses 3 and 4, P02-T005, P02-T008, CORPUS-0001 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

Two, and either alone would require an ADR.

**An explicit product choice.** [EVD-0013](../evidence/phase-02/EVD-0013-minimal-patch-equivalence.md)
measured V2's envelope against V1's and found the segment **shape** differs
while every **landmark** agrees to within the instrument's resolution. Whether that is a defect or the intended
behaviour is a question about whose envelope is right, which `PROCESS.md`
classifies as requiring the user's decision and which
[ADR-0040](ADR-0040-v2-owns-its-dsp.md) explicitly declines to settle in
advance.

**A change to what the corpus manifest records.** The manifest's `change` list
is where a reader looks to find what V2 does differently on purpose, and the
envelope's segment shape belongs there. Adding an `intentional-correction` is a
change to a durable migration contract, not a manifest edit.

**`CORPUS-0001-P2` is not amended, and an earlier draft of this record wrongly
said it would be.** That draft proposed rewording P2 to claim *landmarks*; the
manifest already says exactly that, and has since the case was written. The
correction matters because it changes the finding: P2 claims landmarks, every
landmark is within the instrument's resolution, so **no `preserve` claim is
broken** and ADR-0040 clause 4's failure branch does not apply.

## Decision boundary

The concrete choice is what `CORPUS-0001-P2` claims, and therefore what V2's
envelope is obliged to reproduce.

**Verified premises**, all measured in EVD-0013 rather than argued:

- Every envelope **landmark** agrees to within the instrument's 10 ms
  resolution. `rise_to_90_ms` is at zero difference at all six sweep points and
  E2b's decay endpoint at zero; the sustain level agrees to −0.0027 dB;
  `tail_end_ms` is at one window, and `peak_ms` at zero everywhere except the
  110 Hz point, where it is also at one window.
- The **paths between** the landmarks differ. V1's segments are exponential,
  aimed past their endpoint so the curve crosses it at the authored time
  (`crates/synth_modules/src/envelope.rs:291`); V2's are linear ramps of an
  exact frame count. Measured by region: attack and decay carry +0.186 dB more
  energy in V2, and the release **+1.137 dB** with each engine measured from its
  own gate. (A window shared between the two reads +1.333 dB, but that folds in
  the 102-frame scheduling difference E4 owns, so it is not the envelope's
  figure.)
- One reported field sees it, and that field is **not a landmark**. With sustain
  at 0.700 the envelope first reaches half its peak during the release, so
  `EnvelopeDifference::fall_to_50_ms` probes the release *curve*. It reads
  **+20 ms against EVD-0013's 10 ms threshold** at four of six sweep points —
  so EVD-0013's E2a, which applied that threshold to all four of the metric's
  fields, is a **stricter** test than CORPUS-0001-P2 asks for.
- **The release figure separates cleanly from the scheduling difference.** V1
  applies a note-off at the start of the block containing it, 102 frames early
  at this fixture, so a shared measurement window compares V1's release already
  under way against V2's at its start. Measured from **each engine's own gate**,
  the release difference is **+1.137 dB**; the remaining 0.196 dB of the
  shared-window figure is E4's subject rather than the envelope's.

**Non-goals.** This record does not change V2's envelope, does not add a curve
control, and does not claim V2's shape sounds better than V1's. It decides what
is *claimed*, and Phase 5 owns whatever shaping the node catalog eventually
grows.

## Evidence

- **[EVD-0013](../evidence/phase-02/EVD-0013-minimal-patch-equivalence.md)** —
  the measurement, its five closed asymmetries, and the region-by-region
  attribution that puts the difference in the envelope rather than in the
  oscillator or the filter.
- **`crates/synth_engine_v2/src/tests/kernels.rs` and `tests/voice_nodes.rs`** —
  V2's envelope's exact segment durations, its release from any level, its
  edge-triggered gate and its boundary level, all asserted as the node's own
  criteria.
- **Uncertainty that could change this.** Nobody has *listened* to the two
  envelopes. The difference is 1.137 dB of release energy on one patch, and
  whether an exponential release is musically preferable to a linear one is not
  something EVD-0013 can measure. If it is, option B below becomes the right
  answer and this record is revisited.

## Options

### A. The difference is intentional; the manifest records it as a change

`CORPUS-0001-P2` stands as written, because what it claims — landmark parity —
is met. The segment shape becomes a named intentional difference alongside
`CORPUS-0001-C1`, in the `change` list where a reader looks for what V2 does
differently on purpose.

V2's exact segment durations are a property P02-T005 built **deliberately**, and
the reason is on record: an accumulated increment was measured arriving tens of
samples early, so the envelope was given a frame count rather than a coefficient.
[ADR-0040](ADR-0040-v2-owns-its-dsp.md) clause 3 then makes each V2 node kind
justify itself "on its own criteria rather than as likeness to V1", which is the
standard this envelope already meets.

What it costs: V2's envelope will not sound like V1's on a release, and no
record will require it to.

### B. The difference is a defect; V2 grows a curve control

The manifest is left alone and V2's envelope acquires a shaping parameter able
to reproduce V1's exponential form.

No `preserve` claim requires this — P2 asks for landmarks and gets them — so it
would be a change made because the difference is *undesirable* rather than
because a contract is broken. That is a legitimate reason, and a curve control
is a normal thing for an envelope to have; Phase 5's catalog will want one. What
it costs is that Phase 2 grows a node feature to match V1 rather than to meet a
need, which is the direction ADR-0040 was written to stop, and the new parameter
would have to preserve the exact frame counts P02-T005's tests pin.

### C. Status quo

Record nothing. The difference is real and measured, and would then appear in no
durable record at all — so the next reader of the manifest would find a `change`
list that does not mention a 1.137 dB difference in every note's release.
Whether that is *audible* is exactly what nobody has established, which is an
argument for recording it rather than against. It is the option that produces an
undocumented divergence without anyone choosing it.

## Decision

**Option A.** The user decided on 2026-08-20, with EVD-0013's figures in front
of the choice.

1. **V2's envelope segments are linear ramps of an exact frame count, and that
   is the intended behaviour.** It is not a temporary approximation of V1's
   exponential form, and no later phase owes a correction for it.
2. **`CORPUS-0001-P2` is not changed.** It claims landmark parity and V2
   delivers it: every landmark is within the instrument's 10 ms resolution and
   the sustain level within 0.0027 dB.
3. **The segment shape becomes `CORPUS-0001-C2`**, an `intentional-correction`
   alongside C1, with EVD-0013's own-gate figure of +1.137 dB in the release as
   its rationale.
4. **EVD-0013's E2a stays as declared, and stays exceeded.** Its threshold is
   **not** rewritten. What this record establishes is narrower and is stated as
   such: E2a applied a 10 ms landmark tolerance to all four fields of a metric
   whose `fall_to_50_ms` is not a landmark on this fixture, so E2a is a stricter
   test than P2 asks for. The exceedance is real and is retained; it is a
   finding about the record's own operationalisation rather than about a broken
   contract.

**What this record does not license.** It settles the segment *shape* only, and
it does not soften E2a. Every envelope property CORPUS-0001-P2 covers stays a
preserve claim, and EVD-0013 measured all of them as met.

## Consequences and risks

- **Accepted cost.** V2's release carries **1.137 dB** more energy than V1's on
  this patch, measured from each engine's own gate so the figure is the curve's
  and not the scheduler's. A project migrated from V1 will sound different on
  every note's release, and nothing in the build will say so.
- **Safety/correctness control.** Clause 4 keeps the measurement rather than the
  threshold: E2a stays as written and stays exceeded, so a reader of EVD-0013
  sees the difference and its size rather than a threshold retro-fitted around
  it. `CORPUS-0001-C2` is where the disposition lives.
- **Revisit condition.** Someone listens to both and finds V1's release
  musically preferable, or Phase 5's node API adds a curve control for its own
  reasons — at which point reproducing V1's shape becomes cheap and this record
  is worth reopening.
- **Risk: "landmark parity" becomes a licence to differ anywhere between the
  landmarks.** Control: partial. E2a's four fields still bound peak, rise, and
  tail, and E2b bounds the decay endpoint and the sustain level, so what is
  actually unconstrained is the curvature within a segment. A record that wanted
  to constrain that would need a shape metric no current tool reports.

## Specification update

No current specification under [`specs/`](../specs/README.md) changes: V2's
envelope already behaves as clause 1 describes, and its own tests already pin
it.

What acceptance changes is the **corpus manifest**,
`corpus/v2-reference/manifest.json`: one new entry, `CORPUS-0001-C2`, in
`CORPUS-0001`'s `change` list. **No `preserve` claim is edited** — an earlier
draft of this record said P2 would be reworded, and it was already worded that
way. The manifest's `sha256` fields are digests of the fixture projects rather
than of the claims, so no fixture is regenerated and no digest moves.

## Review

Reviewer:

Stopping rule: false conclusion-affecting fact, contradiction, unfillable
contract, safety/correctness defect, or evidence incapable of supporting the
claim. Editorial detail does not block.
