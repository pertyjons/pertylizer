# EVD-0018: State-Ownership Ledger Coverage of the Persisted Project Schema

| Field | Value |
|---|---|
| ID | EVD-0018 |
| Status | Complete |
| Phase | 00B |
| Created | 2026-08-29 |
| Last reviewed | 2026-08-29 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `688e83a2` |
| Retention | Permanent |
| Related | P00B-T001, ADR-0013, ADR-0014, ADR-0018 |
| Artifacts | `scripts/check_state_ownership_coverage.py`, `scripts/test_check_state_ownership_coverage.py` |

## Question and falsifier

Phase 0B's first exit bullet requires that **every currently persisted field appears exactly once** in the
[state-ownership ledger](../../inventories/state-ownership.md) with a proposed V2 owner or an explicit removal
decision. Three passes of the audit asserted that coverage from a count — 121 `$defs` and 276 declared properties
walked programmatically — and the inventory rules say in as many words that "coverage is not complete merely because a
known baseline count matches". A count cannot distinguish a field nobody claimed from a field two entries claim.

The question is therefore not *how many* fields there are but whether the claim is **checkable**: can a persisted field
exist that no ledger entry claims, or a ledger entry survive that describes a field the format no longer has?

The conclusion is `Supported` only if a mechanical check exists that

1. passes on the repository as committed, and
2. **fails** when either failure mode is introduced.

The second condition is what the record turns on. A check that has only ever passed establishes nothing, so a passing
run on unmutated input is not evidence by itself and is not reported as such below.

## Inputs and controls

- **Persisted surface**: `schemas/project.schema.json`, walked from the root through `$ref`, `items`, and the three
  combinators. The unit of coverage is a **leaf-valued path**, written with `[]` for an array element — `.song.tracks[].solo`.
  A container contributes no path of its own; it is covered when its leaves are. Recursion is cut by remembering the
  resolved node on the current branch, so a self-referential definition yields the path that re-enters it and stops.
- **Claim**: the *Schema coverage map* the ledger carries. The map lives in the ledger rather than in the script so that
  the ledger stays the single authority for what it covers; the script only enforces it.
- **Resolution rule**: longest matching prefix, so a field-level entry beats the container it sits in. This is what lets
  `solo` and `patch.settings.octave_offset` sit in their own entries while their containers keep the rest.
- **Controls**: the check is run against the committed repository and against two mutated copies of the real schema —
  one with a field added, one with a claimed field deleted. Both mutations are applied to the **real** schema rather
  than to a fixture, because a synthetic input can be shaped to make a check look sharper than it is.

## Method

`scripts/check_state_ownership_coverage.py` enumerates the leaf paths, reads the two tables from the ledger, and reports
eight distinct failures: an unclaimed persisted field; a map rule matching nothing; two rules claiming the same path
prefix; a rule naming an entry the ledger does not define; an outside-the-schema declaration naming an undefined
entry; a ledger entry that is neither mapped nor declared outside the schema; an entry that is both; and an entry
marked `Classified` while a required cell is blank. It exits non-zero on any of them.

`scripts/test_check_state_ownership_coverage.py` exercises each of those failures against a mutated minimal input,
plus the longest-prefix rule, the container rule, and recursion termination, and finishes with an integration test that
the committed repository passes.

`scripts/check_v2_docs.py` runs both, so the claim is enforced by the Core V2 documentation gate and by the quality
workflow that already invokes it, rather than by remembering to run a script. That the hook fires was verified the
same way as the checker itself: a stale coverage rule added to the ledger makes the whole documentation gate exit
non-zero.

**Two holes in the first version of the checker were found by review and closed**, and both would have made this
record claim more than it could support. A map that named the same path prefix twice was accepted, so the exact
double claim the record exists to falsify passed; a repeated prefix is now itself a failure. And the scan for which
entries the ledger *defines* accepted any row starting with an entry id, including the coverage map's own rows and
the outside-the-schema table's — so a table naming an undefined entry defined it in passing, and the two checks for
that could never fire. Two focused re-reads narrowed the fix twice: shape alone still let a ten-column row anywhere
define an entry, and scoping by table header still carried across a blank line into the next table. A row now
counts only between the ledger's own header and the first line that is not a table row.

## Reproduction

```text
python3 -B scripts/check_state_ownership_coverage.py
python3 -B -m unittest scripts/test_check_state_ownership_coverage.py
```

## Results

The committed repository reports **1,359 persisted leaf paths, 83 coverage rules, 64 ledger entries, 16 of them
declared outside the project schema**. **1,116** of those paths — 372 each under the three module chains — are the
open module-parameter maps, claimed by three rules, so 243 paths carry the rest of the format.

Both mutations of the real schema were observed to fail, which is the observation this record exists for:

| Mutation of `schemas/project.schema.json` | Exit | Reported |
|---|---|---|
| none | 0 | coverage passes |
| a new `record_arm_state` property added at the root | 1 | `persisted fields claimed by no ledger entry` |
| `active_instrument_id` deleted | 1 | `coverage rules matching no persisted field` |
| none, with a stale rule added to the ledger instead | 1 | the same, reported by `check_v2_docs.py` |

All seventeen unit tests pass, including the eight that neutralise one guard each.

## Limitations

- **The schema is the surface, not the writer.** Coverage is over what the schema declares. A field a save path writes
  but the schema does not declare is invisible here; the ledger's *Save-path map* is what covers that, and it is read
  rather than executed.
- **A claim is not a verification.** This record establishes that every persisted field is *claimed* by exactly one
  entry, not that the claimed owner is correct, nor that a round trip preserves the field. The owners rest on the traces
  recorded in the ledger's audit passes, and round-trip preservation is P00B-T005's fixtures.
- **Open maps are covered as maps.** The 1,116 module-parameter paths are claimed by prefix, so adding a module
  parameter does not fail the check. That is deliberate — the ledger records `module.parameters` as an open map whose
  key set is ADR-0016's question — but it means this check cannot detect a new parameter needing its own migration
  question.
- **Settings, recovery, bundle and session entries are declared, not derived.** The sixteen entries outside the project
  schema are asserted to be outside it; nothing here walks `settings.json` or the bundle envelope to confirm the list is
  complete.

The check also enforces the ledger's own status rule, which is row-level: **37 entries are `Classified`** and 27 remain
`Investigating` with a blank `Migration` cell. Marking an entry `Classified` while a required cell is blank fails the
gate — the defect that downgraded every status in this ledger once already.

## Conclusion

`Supported`. The ledger's exactly-once claim is now mechanically enforced rather than asserted, and both failure modes
were observed on the real schema. P00B-T001's third completion bullet — that entries reach `Classified` only once an
evidence record carries the coverage claim — is discharged for coverage.

The limitations above are what keep the entries' *dispositions* from being verified by this record alone; the ledger's
status column moves on coverage and the recorded traces, and P00B-T005's fixtures are what would move it further.
