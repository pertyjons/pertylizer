# V2 Evidence Records

Evidence records make architectural claims and exit gates reproducible. They include benchmarks, audio comparisons, code
audits, simulated-host tests, workflow analyses, and focused prototypes.

## Organization

Group records by the phase that produced them:

```text
evidence/
├── README.md
├── phase-00a/
│   ├── EVD-0001-reference-corpus.md
│   └── EVD-0002-v1-resource-baseline.md
└── phase-03/
    └── EVD-00NN-callback-partition-invariance.md
```

Create phase directories only when they contain evidence. Allocate `EVD`
identifiers globally rather than restarting numbering in each phase.

Next free identifier: `EVD-0013`. Update this line when you allocate one.

## Status and retention vocabulary

Evidence status is one of:

- `Draft` — the question and method are being prepared;
- `Active` — collection or analysis is in progress;
- `Complete` — results, limitations, and conclusion are reviewable;
- `Superseded` — a newer evidence record replaces its decision value;
- `Archived` — non-authoritative preliminary material retained only for
  history;
- `Abandoned` — the work stopped before a usable conclusion, with the reason
  recorded.

Conclusion (`Supported`, `Not supported`, or `Inconclusive`) is separate from
status. An inconclusive experiment can therefore still be `Complete`.

Retention is either `Permanent` or `Until phase exit`. Evidence referenced by
an accepted ADR or exit review becomes `Permanent` regardless of its original
retention value.

## Requirements

Copy [../templates/evidence.md](../templates/evidence.md). Every record must identify:

- the question or hypothesis;
- relevant ADRs, tasks, inventory entries, and gates;
- the exact source revision;
- environment, inputs, method, and commands;
- summarized observations and retained artifacts;
- interpretation, limitations, and conclusion.

An assertion such as "faster", "real-time safe", "equivalent", or "deterministic"
is incomplete without a metric, threshold, automated test, or explicit review criterion.

### State the falsifier before measuring, and run its control first

Before a measurement is taken, the record states **what the method would report
if the effect were absent**, and that control is run before the measurement it
guards. A number produced without one is an artifact until proven otherwise, and
it will look exactly like a result.

[EVD-0004](phase-00a/EVD-0004-corpus-0005-claim-counterfactuals.md) is the
worked example, and it failed this twice.

- It modelled a per-voice effect chain by rendering each note alone and summing,
  and got a clean +0.67 dB. The same construction on a project with **no effects
  at all** produced a difference of the same size. Two claims were withdrawn and
  a migration contract was written against a defect that did not exist. With the
  confounding parameter removed the control sits at −147 dB and the method
  works.
- Its cost check compared a 15-draw minimum against a 50-draw baseline and
  argued the shift was real because it was *uniform across cases*. Small-sample
  bias in a minimum estimator is uniform too, so uniformity was never evidence.
  Matching the draw counts reversed the sign.

Two rules follow, and both are cheap:

- **A comparison across sessions is not a measurement** unless the estimator,
  the draw count, the build profile, and the binary all match. Where they cannot,
  the record says what it cannot conclude rather than interpreting the number.
- **A control that fails is telling you about your fixture**, not about the
  engine. Diagnose it before reporting anything downstream of it.

## Artifact policy

Small reviewable CSV, JSON, or text results may be committed next to a record. Large audio files, traces, profiler
dumps, build outputs, and generated reports do not belong under `plans/v2/`.

For an external or regenerable artifact, record:

- its stable name and purpose;
- content digest when it must be preserved exactly;
- storage location when applicable;
- the command and source revision needed to regenerate it;
- the compact result that reviewers need.

Important audio and project inputs intended for automated regression testing belong with repository test fixtures, not
in the planning directory.

## Retention

Evidence referenced by an accepted ADR or exit review is permanent and should not be moved. If better evidence replaces
it, mark it `Superseded`, link to the replacement, and keep its stable path. Preliminary material with no durable
reference may be archived after its conclusions have been captured.
