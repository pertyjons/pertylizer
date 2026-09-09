# EVD-0021: The Session Share's Catch-Up High Water Over Every Saved Project

| Field | Value |
|---|---|
| ID | EVD-0021 |
| Status | Complete |
| Phase | 08 |
| Created | 2026-09-09 |
| Last reviewed | 2026-09-09 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `233a9252e9dac6718c2ba59fe2719a33f30dc459` |
| Retention | Permanent |
| Related | ADR-0054, ADR-0051, ADR-0046, HOST-INV-022, SOUND-INV-034, P08-S004 |
| Artifacts | [survey table](artifacts/EVD-0021-results.csv), [run and environment](artifacts/EVD-0021-run.txt), [subject bundle](artifacts/EVD-0021-subject.bundle), [SHA256](artifacts/EVD-0021-SHA256SUMS.txt) |

## Question and falsifier

What value of `session_event_share` admits the real projects this repository holds, now
that a whole project — instruments, inserts, sends and returns — lowers into one plan?
ADR-0054 clause 2 stages the reselection of a share at the first real consumer that
measures its occupancy. The session share's one plan-dependent contributor is ADR-0051's
locate catch-up: one event per writable control per node of the plan, plus the boundary
release, which admission charges exactly as `HOST-INV-022`'s bounded contributor. Its
occupancy is therefore not a runtime observation but the session row of every real
project's admission report, and that is what this record measures.

The selection rule, stated before the survey was run: the next multiple of 32 at or above
twice the high water `H` over the measured population, with `max_events_per_quantum`
raised by the same amount so the other five provisional shares keep their values and the
six still sum to the cap exactly. `Supported` requires the control below to report exactly
one, every measured request to match the count admission charges, the selected share to
admit every measured project and fixture whose request is at or below it under the
engine's **default** profile, and the corpus insert case — refused by name at 29 against
the first provisional 24 (`P08-S003`) — to be among them. A project that reaches a plan
under the measurement partition and is refused under the selected default is
`Not supported`. A population in which no saved project reaches a plan is `Inconclusive`.

## Inputs and controls

The population is every saved project the repository holds — the ten pinned corpus cases
under `corpus/v2-reference/projects` and the eighteen shipped examples under
`assets/examples/projects`, both saved forms — plus two synthetic fixtures for the bus and
send shapes `P08-S004` builds and no saved project exercises end to end: one instrument
with one post-fader send into one return running the corpus delay fully wet, and two
instruments on one patch each sending into their own such return. The corpus's own send
case (`CORPUS-0004`) is in the population and does not reach a plan: its return runs a
reverb and its master a compressor, neither of which V2 has carried, so it contributes no
occupancy and is listed as an exclusion rather than counted.

Every lowering runs under the stereo 48 kHz harness profile with its **events group
scaled eight-fold** (`project_profile` in `lowering::tests::phase8`), so that admission
cannot censor what the survey counts: a request above the measurement partition's own
share would be refused and its row would still be read, since the report is produced
whether admission succeeded or failed (`HOST-INV-006`). No request reached that bound.

The control, run first as an ordinary test: a plan holding one silent source and its
output — no writable control — requests exactly one session event, the boundary release
alone. A method that reported zero for it, or that counted controls a write cannot
reach, would be counting something other than the batch.

## Method

The ignored test `lowering::tests::evidence::evd_0021_session_share_high_water_over_every_saved_project`
loads every project in the population in path order, lowers and renders it through
`smoke_render_project` under the measurement partition with the parity output policy,
reads the session row's requested amount from the admission report the render carries,
and records whether the project reached a plan and, where it did not, the first refusal.
The high water is the largest request among the rows that reached a plan. The synthetic
fixtures are measured by the same call. Nothing is timed; the count is exact and
deterministic, so one run is the measurement.

The consequence is then held by an ordinary test rather than by this record:
`the_selected_session_share_admits_every_project_the_survey_measured_below_it` renders the
corpus insert case and both fixtures under the engine's default profile and asserts each
is admitted at its measured request, and that the default share and cap are the selected
values.

## Reproduction

```text
cargo test -p pertylizer --lib -- lowering::tests::evidence
cargo test -p pertylizer --lib -- lowering::tests::evidence::evd_0021_session_share_high_water_over_every_saved_project --ignored --nocapture
```

The bundle retains the exact committed subject relative to `main`'s tip at the time,
`b919b88ecac8aa3dca76679e593de813a7ab439f`, so that a squash merge cannot lose it. From a clone containing that
commit:

```bash
cd plans/v2/evidence/phase-08/artifacts
sha256sum -c EVD-0021-SHA256SUMS.txt
git bundle verify EVD-0021-subject.bundle
git fetch ./EVD-0021-subject.bundle HEAD
cd ../../../../..
git worktree add --detach /tmp/evd-0021-reproduce FETCH_HEAD
cd /tmp/evd-0021-reproduce
cargo test -p pertylizer --lib -- lowering::tests::evidence
cargo test -p pertylizer --lib -- lowering::tests::evidence::evd_0021_session_share_high_water_over_every_saved_project --ignored --nocapture
```

The run transcript records the subject, its parent, a clean worktree and the toolchain.

## Results

The full table, one row per project with its first refusal where it did not reach a plan,
is the [survey artifact](artifacts/EVD-0021-results.csv), with the run transcript beside it. Of the thirty rows, six reach a
plan:

| Row | Session request |
|---|---|
| `subtractive-voice` | 21 |
| `tempo-map-arrangement` | 21 |
| `mod-matrix` | 23 |
| `instrument-inserts` | 29 |
| synthetic: one instrument, one post-fader send, one delayed return | 33 |
| synthetic: two instruments, two sends, two delayed returns | 64 |

Twenty-four saved projects do not reach a plan, each for a refusal the lowering
specification names — an effect type, a module type, a parameter value or a shape V2 has
not carried — and they contribute no occupancy. The control reports one. The high water
is `H = 64`; the rule selects `32 × ⌈2 × 64 / 32⌉ = 128`, and the cap rises by
`128 − 24 = 104` to 360. Under that default the four saved projects and both fixtures are
admitted at their measured requests, and the corpus insert case renders the same samples
under the default profile as under the measurement partition.

## Limitations

The high water is a bound over the projects that **currently lower**, not over every
saved project: twenty-four of thirty are excluded by refusals unrelated to this share,
and a project among them that later lowers may request more. The two fixtures are the
slice's own; a real project with several returns, several sends per channel and inserts
on each would request more than 64, and admission would refuse it by name rather than
borrow, as ADR-0046 requires. The factor of two and the rounding are a choice with
headroom, not a measurement. EVD-0019's publication cost was measured at a cap of 256; the
bounded serial mechanism's cost scales with the cap and is not re-measured here. Nothing
in this record qualifies the partition for production live ingress; ADR-0054 clause 3's
complete reselection remains Phase 9's.

## Conclusion

**Supported.** `session_event_share` is reselected from 24 to 128 and
`max_events_per_quantum` from 256 to 360, both still provisional under ADR-0054 clause 4.
The corpus insert case, refused by name at 29 since `P08-S003`, is admitted under the
engine's default profile, and the default-profile survey of eligible saved projects counts
four where it counted three.
