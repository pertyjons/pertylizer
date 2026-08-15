# EVD-0005: Resource-Ledger Use-Site Audit

| Field         | Value                                                        |
|---------------|---------------------------------------------------------------|
| ID            | EVD-0005                                                      |
| Status        | Complete                                                      |
| Phase         | 00A                                                           |
| Created       | 2026-08-15                                                    |
| Last reviewed | 2026-08-15                                                    |
| Retention     | Permanent                                                     |
| Related       | P00A-T004, P00A-T005, ADR-0021, ADR-0038                      |
| Superseded by | —                                                             |

## Question or hypothesis

The [resource inventory](../../inventories/resource-limits.md) claims that every fixed cap, truncation point, bounded
queue, buffer capacity, and script budget in the workspace appears exactly once, with its enforcement site, overflow
behaviour, owner, proposed V2 rule, and diagnostic. Passes 4 and 5 audited that claim by reading what each constant is
**used for** rather than where it is defined.

This record asks two questions about those passes:

1. **Coverage** — did the two passes between them reach all of the ledger's entries?
2. **Accuracy** — what proportion of the entries they read were wrong, and in what way?

**This record is retrospective, and that is a defect in it, not a detail.** Passes 4 and 5 ran on 2026-08-14 and
2026-08-15; this record was written after both. The evidence directory's rule is to
[state the falsifier before measuring and run its control first](../README.md#state-the-falsifier-before-measuring-and-run-its-control-first),
and a record written after the fact cannot satisfy it: the acceptance criteria below were not fixed before collection,
and nothing prevents them having been shaped by what the passes happened to find. What can be done honestly is done —
the criteria are stated, the controls that exist are named as controls and one of them is executable and was observed
firing — and the rest is recorded as a limitation rather than glossed. A future ledger pass states its criteria first.

## Acceptance criteria

Fixed here, after the fact, from what the passes were convened to establish:

- **Coverage is satisfied** only if every ledger entry has had at least one use-site read — resolving its cited
  `file:line` and reading the uses of the constant, not only its definition. A row read by definition alone does not
  count, because that is exactly the method the passes existed to correct.
- **Accuracy is reported as a rate, not as a pass/fail.** An entry is *substantively wrong* when a reader acting on it
  would build the wrong thing: a misdescribed resource, a reversed overflow behaviour, a misattributed owner. It is
  *citationally wrong* when the values are right and the `file:line` does not resolve to what it names. The two are
  counted separately because they have different consequences and different fixes.
- **The coverage claim is falsified** if a re-audit of any sample of entries finds one that no pass reached, or one
  whose recorded behaviour contradicts its source.

## Source and environment

- Source revision: `3555c52c` (pass 4), `bac88c0c` (pass 5), `29c22ef4` (the disposition pass and every re-resolution
  in this record).
- Platform and architecture: Linux 7.1.8, x86-64.
- Rust/tool versions: workspace toolchain; `ripgrep` for search, `cargo test` for the executable control.
- Audio/sample configuration: not applicable — this is a source audit, nothing is rendered.
- Feature flags: default.
- Relevant host or device simulation: none.

## Inputs

- The [resource inventory](../../inventories/resource-limits.md) at each revision above: 74 entries at pass 3, 75 after
  pass 4 added `LIMIT-0075`, 76 after ADR-0038 split `LIMIT-0014` into `LIMIT-0014` and `LIMIT-0076`.
- The workspace sources under `crates/`.
- `crates/pertylizer/tests/ledger_citations.rs` — three tests over the ledger's citations, written during pass 5.

## Method

Three methods ran, and the differences between them are part of the result.

**Pass 4 — full use-site read, 30 entries.** The entries that were `HostProfile`-owned when it started. For each:
resolve every cited `file:line`, then read *every* use of the named constant rather than its definition. Prompted by
external review finding that `LIMIT-0014`'s constant sizes an egress ring rather than a renderer scratch buffer.

**Pass 5 — partial use-site read, 44 entries.** The remainder. It resolved every citation, then read the uses of the
constants whose names were generic enough to hide a second purpose, and spot-checked the rest. **This is a weaker
method than pass 4's, and the record of it said so only after review objected**: the pass first reported "38 of 44
accurate" on that partial basis, and two further errors turned up immediately when the gap was pointed at.

**Disposition pass — narrow re-audit, 4 entries, `29c22ef4`.** The four rows that had left `Classified`. Every claim
re-read from source rather than carried over, on the grounds that those rows were open precisely because an earlier
pass had recorded something it had not verified. This doubles as the sample re-audit the coverage criterion asks for.

**Controls.**

- *Executable, and observed firing.* `ledger_citations.rs` resolves every citation in the ledger and fails on a missing
  path, an ambiguous bare file name, a line past end of file, or a line holding only whitespace, punctuation, or a bare
  `///`. It is a control rather than a formality: during the disposition pass it failed with eight stale citations,
  which is the evidence that a green run means something. It also caught **fifteen** stale citations that pass 5 had
  missed, several of them broken by the audit's own bug fixes.
- *Negative control on the strong check.* The third test only checks citations written in the annotated form
  `` `path.rs:60` (`MAX_TAIL_SECONDS`) ``. It asserts that at least one such citation exists, so a change that silently
  destroyed the annotation form fails rather than passing vacuously.

**What is not a control, and was called one in an earlier draft of this record.** Pass 4 and pass 5 used different
methods on *disjoint* halves of the ledger, so method and population vary together and nothing separates them. That is
a confound, not a control, and it means the difference between their error rates measures neither the method nor the
entries on its own. Calling it a control would have been the same error EVD-0004 made when it read a comparison with
no control as a result — the failure this directory's own rules were written from. The comparison is reported below
because the direction is still informative; it is not evidence for a cause.

## Commands

```text
# The executable control over every citation in the ledger.
cargo test --workspace --test ledger_citations

# Citation inventory used for the coverage figures below.
python3 - <<'EOF'
import re, io
s = io.open("plans/v2/inventories/resource-limits.md", encoding="utf-8").read()
print("rows        ", len(re.findall(r"(?m)^\| LIMIT-\d{4} \|", s)))
print("path:line   ", len(re.findall(r"`([A-Za-z0-9_./-]+\.rs):(\d+)", s)))
print("bare :line  ", len(re.findall(r"`:(\d+)", s)))
print("annotated   ", len(re.findall(r"\.rs:(\d+)` \(`([A-Za-z0-9_:]+)`\)", s)))
EOF
```

## Results

**Coverage.** 74 entries existed when pass 4 began. Pass 4 read 30, pass 5 read 44, and the two sets are disjoint and
exhaustive over that population. `LIMIT-0075` was added *by* pass 4 from its own reading, and `LIMIT-0076` was split
out of an entry both passes had read. Every entry in the ledger has therefore had at least one use-site read.

**Accuracy, by pass:**

| Pass | Entries | Substantively wrong | Citationally wrong | Method                 |
|------|--------:|--------------------:|-------------------:|------------------------|
| 4    |      30 |                   5 |                  2 | Full use-site read     |
| 5    |      44 |                   2 |                  5 | Partial use-site read  |
| Disposition | 4 |                   0 |                  2 | Full re-read of 4 rows |

Pass 4's five: `LIMIT-0015` misowned (four deferred-drop channels, not a return-bus scratch); `LIMIT-0014`
misdescribed (recorded as a per-block renderer limit; V1 has no per-quantum event cap); `LIMIT-0024` an unrecorded
silent-truncation site; `LIMIT-0023`/`LIMIT-0041` recorded as a 1:1 coupling where the relation is a `<=` floor; and
`LIMIT-0043` reclassified. Pass 5's two: `LIMIT-0016` naming one ring where its constant sizes two and stating its
overflow backwards, and `LIMIT-0059` claiming no production path constructs `ChannelCount::Multi` when a multichannel
device does.

**The passes' own false findings.** Pass 4 produced two, both caught by external review, and both are the failure the
pass was convened to correct: an absence asserted from a name search (three constants reported as unregistered are
`LIMIT-0037`, `LIMIT-0038` and `LIMIT-0040`, registered under prose names), and a truncation asserted from a `.min()`
without reading the `push` that feeds it. Pass 5 overstated its own depth, corrected by review.

**Silent-truncation register.** Six entries before pass 4, eleven after pass 5. Five of the eleven were found outside
any search — by classifying, by reading use sites, or by review of a correction.

**Citation health at `29c22ef4`**, from the command above:

| Quantity                                                   | Count |
|------------------------------------------------------------|------:|
| Ledger rows                                                 |    76 |
| Explicit `path.rs:line` citations                          |   162 |
| Bare `:line` continuations                                 |    64 |
| Citations in the annotated form the strong test checks     |    10 |
| Ledger rows carrying at least one annotated citation       |    10 |

**The disposition pass's two citation findings**, both of which every guard test passed: `LIMIT-0017` cited
`crates/synth_engine/src/hub.rs:145`, a `Debug` impl field seven lines short of `CLIENT_EVENT_BUFFER_SIZE` at line 152;
and `LIMIT-0015` cited its four ring constructions at lines 819, 823, 829 and 856, each exactly five short of the real
824, 828, 834 and 861.

## Interpretation

**The coverage claim is supported.** Every entry has had a use-site read, by disjoint passes that between them cover
the population.

**The accuracy claim is not what the coverage claim implies, and this is the finding worth carrying.** Pass 4 found
five substantive errors in 30 entries (17%) and pass 5 found two in 44 (5%) — a factor of about three, after review
corrected the ledger's first-pass claim of an order of magnitude. **What that factor is caused by is not established
here.** Pass 4's 30 were both the entries a specification was being written against and the entries read by the
deeper method; the two explanations are inseparable in this data.

The two passes also differ in method, and the weaker method looks better on the numbers. Pass 5's partial read
plausibly *missed* errors that pass 4's full read would have found, and review did find two it had missed. So its 5%
is a **lower** bound on its error rate, and 95% correspondingly an upper bound on its accuracy — a full read of the
same 44 entries could only move both against it. **No causal claim survives this**, in either direction: the harder
entries and the deeper method are the same 30 rows, so "the entries reasoned about hardest were wrong more often" and
"the deeper method finds more errors" predict the identical numbers. Separating them needs a full read of a sample of
pass 5's population, which has not been done.

**Citation accuracy is separately weak, and the guard is partial by construction.** Only 10 of 76 rows carry the
annotated form that the sole drift-onto-valid-code test checks — 13% of rows, 6% of the explicit citations. The
disposition pass re-read four rows and found drift in two of them, both invisible to all three tests. The honest
statement is that citation drift onto valid code is **known present and unbounded**: two instances found in a
four-row sample, with 66 rows carrying no strong check at all.

**What the audit could not do.** Neither pass executes anything. A truncation that is both unnamed and undocumented is
invisible to every method used here, which is why ADR-0021's executable probe remains the only thing that can close the
completeness question. Four separate entries — `LIMIT-0004`, `LIMIT-0024`, `LIMIT-0014`, `LIMIT-0021` — were each
found by a method the previous one could not see, and none by searching.

## Limitations

- **Retrospective.** The acceptance criteria were written after the results existed. This record cannot exclude that
  they were fitted to them.
- **No control on the coverage claim itself.** "Every entry was read" is attested by the passes that did the reading.
  The four-row disposition re-audit is a 5% sample and it found citation errors in half of what it re-read, so the
  sample argues against the ledger being clean rather than for it.
- **The two passes are not comparable as measurements.** Different methods on different populations, one of them
  self-reported as shallower after review. No error-rate difference here is attributable to the entries alone.
- **Error counts are of *found* errors.** Every figure is a lower bound. Three of the errors in this record were found
  by external review of a pass that had already declared itself finished.
- **One class of entry cannot be audited this way at all.** `LIMIT-0075` has no constant to read the uses of — it is an
  uncapped `Vec` whose absence of a bound is the finding. Pass 1 could not match a name and pass 2 could not match a
  truncation comment; it exists because external review noticed the hole left when a wrong description was removed.

## Conclusion

`Supported` for coverage: every ledger entry has had a use-site read, and the passes covering it are disjoint and
exhaustive.

`Inconclusive` for accuracy. The audit establishes that the ledger *was* substantively wrong at a measurable rate —
at least seven entries across the two passes — and it does not establish the residual rate. **It also does not
establish why the two halves differ**, because the deeper method and the harder entries are the same rows; the
attribution to "the entries reasoned about hardest" that an earlier draft of this record made is withdrawn as
unsupported. Its own sample re-audit found two citation errors in four rows.

**Gate impact.** This record supplies the evidence P00A-T004's coverage claim was missing, which was one of the two
things keeping the task open; the other was the four rows outside `Classified`, disposed at `29c22ef4` under ADR-0021
and [ADR-0038](../../decisions/ADR-0038-engine-egress-queue-classification.md). It does **not** satisfy the Phase 0A
exit gate's completeness requirement, and nothing here should be read as claiming it does: the gate asks that every
current fixed cap appear once in the inventory, and a search-based audit cannot demonstrate that no unnamed,
undocumented truncation remains. ADR-0021's executable probe is still the only instrument for that, and this record is
its fourth argument.

## Artifacts

| Artifact                                          | Location/digest                                              | Retention or reproduction                          |
|---------------------------------------------------|--------------------------------------------------------------|----------------------------------------------------|
| The ledger itself, with per-pass audit rows      | [`inventories/resource-limits.md`](../../inventories/resource-limits.md) | Permanent; the *Audit passes* table carries each pass's method and result |
| Executable citation control                      | `crates/pertylizer/tests/ledger_citations.rs`                | Permanent; `cargo test --workspace --test ledger_citations` |
| Citation-health counts                            | Reproduced by the script under *Commands*                    | Regenerate; the figures above are at `29c22ef4`    |
