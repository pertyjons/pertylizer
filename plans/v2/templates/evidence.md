# EVD-NNNN: Evidence Title

| Field         | Value                         |
|---------------|-------------------------------|
| ID            | EVD-NNNN                      |
| Status        | Draft                         |
| Phase         | NN                            |
| Created       | YYYY-MM-DD                    |
| Last reviewed | YYYY-MM-DD                    |
| Retention     | Permanent or Until phase exit |
| Related       | ADR-NNNN, PNN-TNNN            |
| Superseded by | —                             |

Allowed status and retention values are defined in
`plans/v2/evidence/README.md`. Conclusion is recorded separately and does not
replace lifecycle status.

## Question or hypothesis

State the question so that the result can be positive, negative, or inconclusive without changing the question after
measurement.

## Acceptance criteria

Define metrics, thresholds, comparison categories, or review criteria before collecting results.

## Source and environment

- Source revision:
- Platform and architecture:
- Rust/tool versions:
- Audio/sample configuration:
- Feature flags:
- Relevant host or device simulation:

## Inputs

List fixtures, projects, seeds, scripts, and asset digests needed to reproduce the work.

## Method

Describe the procedure and controls.

## Commands

```text
Exact reproducible commands
```

## Results

Present compact raw measurements or link retained artifacts. Separate observed data from interpretation.

## Interpretation

Explain what the results support and what they do not establish.

## Limitations

List missing platforms, unrealistic inputs, measurement noise, prototype shortcuts, or other reasons not to
overgeneralize.

## Conclusion

Use `Supported`, `Not supported`, or `Inconclusive`, followed by the decision or gate impact.

## Artifacts

| Artifact | Location/digest | Retention or reproduction |
|----------|-----------------|---------------------------|
