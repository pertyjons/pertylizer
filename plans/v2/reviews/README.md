# V2 Reviews

Reviews formally evaluate phase gates and final cutover readiness. A task list, successful build, or informal statement
is not a phase review.

## Naming

Use:

```text
phase-NN-exit-review.md
phase-NNx-exit-review.md   # sub-phase, e.g. phase-10c-exit-review.md
v2-cutover-review.md
```

The review identifier is `REV-PNN` for phase reviews, `REV-PNNx` for a sub-phase such as `REV-P10C`, and `REV-CUTOVER`
for the final cutover review. Phase 10 has no review of its own; 10A–10E are reviewed individually.

## Outcomes

- `Draft` — evidence is still being assembled;
- `Accepted` — every required gate passed;
- `Rejected` — one or more gates failed and work must continue;
- `Conditionally accepted` — allowed only when every condition is explicit, bounded, owned by a task, and does not
  weaken a safety or correctness gate.

## Review rules

Copy [../templates/exit-review.md](../templates/exit-review.md). Evaluate each
applicable outcome and exit boundary in [`../ROADMAP.md`](../ROADMAP.md) and link
reproducible evidence or named automated tests. Record `N/A` only with a reason
grounded in an accepted scope decision.

A review must also confirm:

- every durable decision required by the roadmap outcome has the permitted
  status; a deferral names its later acceptance gate, owner, and missing
  evidence;
- inventories have no unclassified entries in the reviewed scope;
- current specifications match implementation;
- deviations from the plan are approved and documented;
- relevant repository quality gates pass;
- failures and residual risks are visible.

Do not edit an accepted review to make later regressions disappear. Record a
new review addendum or reopen the phase explicitly. An exit review audits the
named evidence; it does not perform another broad review of accepted artifacts
unless later evidence invalidates them.
