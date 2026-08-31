# ADR-0053: What a simulated ingress producer stamps

| Field | Value |
|---|---|
| ID | ADR-0053 |
| Status | Accepted |
| Phase | 3 |
| Created | 2026-08-31 |
| Last reviewed | 2026-08-31 |
| Related | ADR-0021, ADR-0022, ADR-0032 clauses 13, 18, 19 and 21, EVD-0016, `SPEC` host profile and render limits |
| Supersedes | — |
| Amends | ADR-0032 clauses 18 and 21 |
| Superseded by | — |

## Durable boundary

**A public API, and a rule that binds Phase 9.**

`TimeSource` is a public enum on every event that crosses into the renderer, and ADR-0032 clause 18 fixes it at three
values. Phase 3's exit gate requires a **deterministic simulated-ingress producer**, and the moment one ships with
tests asserting what it stamps, whichever value it uses stops being provisional: a diagnostics report, a later
adapter, and `HOST-INV-013`'s horizon all read that field and would have to keep meaning what the first shipped
producer made it mean.

**Why now.** The simulated-ingress equivalence slice was built once, reviewed, and discarded before commit, and this
was one of the four findings the next attempt inherits. That attempt is the immediately next slice in the stream, and
it cannot proceed without the answer: the producer's first line of code is the envelope it stamps. Deferring through
the slice would create exactly the public commitment the durable-decision test names, and it would do so silently,
because a wrong tag here is not a compile error — it is a diagnostics report that misdescribes where a timestamp came
from.

**Coupled decisions.** None must close first. ADR-0022 stays `Deferred` to the Phase 9 exit gate and this record does
not touch it: nothing here qualifies a physical clock, and the value this record creates is the one that says so.

## Decision boundary

**What provenance a deterministic simulated-ingress producer stamps on the events it offers**, and what that tag
means for the forward horizon and for the diagnostics report.

**Non-goals.** The producer's own design — its queue, its split handles, its horizon check at `offer` — is the
implementation slice's. The hardware mapper, the arrival fallback and their measured uncertainty stay with ADR-0022
and Phase 9. This record does not decide whether a live adapter may ever be simulated in a release build; it decides
what the tag means, and adds the rule that keeps a release build from producing one.

## Evidence

Verified against the code and the accepted records at this commit's parent.

- **`TimeSource` has exactly three values today** — `Hardware`, `Arrival`, `Compiled` — in
  `crates/synth_engine_v2/src/time.rs`, with one behavioural consumer: `is_ingress()`, which is true for the first
  two and gates the forward-horizon rejection in the render loop. A second, narrower consumer counts `Arrival` events
  into the diagnostics report's `arrival_stamped`.
- **`Hardware` means bridged, and labelling an unbridged clock is a named defect.** ADR-0032 clause 13 makes the
  epoch anchor the ingress mapper's input for the whole epoch, and EVD-0016's falsifier **F11** fires when "an
  adapter labels an event `Hardware` without independently bridging that adapter connection's timestamp origin to the
  observer clock used by the audio mapping". F11 was added by that record's own self-audit after the first Linux CPAL
  observation, so it is an observed hazard rather than a hypothetical one.
- **`Arrival` means the adapter had no timestamp.** ADR-0032 clause 19 requires such an adapter to "declare its
  arrival-time fallback" with "its measured uncertainty — or with an explicit 'unmeasured' marker", and says the
  obligation is "to declare `Arrival` only where the source genuinely has no timestamp".
- **`Compiled` is exempt from the forward horizon.** ADR-0032 clause 21 binds the horizon to ingress provenance
  alone, because "a compiled event list spans the whole piece" and measuring it against a live-input horizon "would
  reject most of a song". `HOST-INV-013` repeats the enumeration: the horizon "binds only events whose provenance is
  `Hardware` or `Arrival`".
- **The discarded slice's finding.** An ingress producer that only forwards events already inside the imminent window
  never exercises the horizon at all, so `HOST-INV-013` and ADR-0032 clause 21 put the check at `offer` and the
  equivalence fixture has to reach it. A producer tagged `Compiled` is exempt by clause 21 and cannot reach it,
  whatever the fixture does.

**Uncertainty that could change the decision.** Whether Phase 9's adapters will want a provenance axis finer than a
single enum — a bridged-and-measured hardware stamp and an unmeasured one are both `Hardware` today. If they do, the
axis is what changes, and this record's fourth value moves with it rather than blocking it.

## Options

### A. `Compiled`

Free, and wrong for a reason the discarded slice already paid for: clause 21 exempts `Compiled` from the horizon, so
the exit gate's equivalence fixture would assert a boundary it never crosses. The test would stay green with the
horizon check missing entirely, which is the defect it exists to catch. It is also false on its face — the event was
not generated from the plan and the timeline.

### B. `Hardware`

The producer's timestamps are exact and pre-mapped, which is what a bridged hardware stamp is. But no driver
produced them and no clause 13 bridge exists, and F11 names labelling an unbridged clock `Hardware` as a defect. A
report could not then distinguish a simulated event from a genuinely bridged one, which is precisely the confusion
F11 was written to prevent.

### C. `Arrival`

Understates. Clause 19 admits `Arrival` "only where the source genuinely has no timestamp"; a simulated producer's
timestamp is exact by construction. It would also feed the `arrival_stamped` counter and require a declared
uncertainty, so the report would carry a fallback measurement for an adapter that has no fallback.

### D. A fourth provenance

Says what is true: engine-external, so the horizon binds; exact by construction, so no uncertainty is declared and no
arrival counter moves; and not a driver's, so no report can mistake it for one. Its cost is an amendment to an
accepted clause, an API break inside the experimental crate, and a rule that keeps it out of a release path — a rule
this record has to state, because an enum variant is constructible by anyone who can name it.

### E. Split provenance from horizon policy

Make the horizon exemption a separate property rather than a consequence of the tag. It is the cleaner axis in the
abstract, and it is more machinery than the one question here needs: today exactly one value is exempt and the
exemption has exactly one reason. Recorded as the shape to revisit if Phase 9's adapters need a finer axis, not
selected now.

## Decision

**A fourth provenance, `Simulated`, and this record amends ADR-0032 clause 18 from three values to four.**

### 1. What it means

`Simulated` is the provenance of an event offered by a deterministic in-engine producer that supplies its own
engine-epoch `SampleTime`. The timestamp is exact by construction, exactly as `Compiled`'s is, and its origin is
outside the renderer, exactly as `Hardware`'s and `Arrival`'s are. Those two facts are why none of the three existing
values fits: each of them fuses one of those halves to the other.

### 2. The forward horizon binds it, and clause 21 is amended to say so

`is_ingress()` is true for `Simulated`, so `HOST-INV-013`'s check applies at ingress admission, once, against the
timestamp as stamped. This is the half of the decision the exit gate depends on: the equivalence fixture reaches the
horizon because the tag does not exempt it, and a producer that offers an event beyond the horizon is rejected and
counted like any other external one.

**ADR-0032 clause 21 is amended alongside clause 18**, and naming only the first was an omission an independent
review found. Clause 21 enumerates the provenances the horizon binds as "`Hardware` or `Arrival`" in as many words,
so a fourth ingress value that the horizon binds changes that clause too. Leaving it would put two accepted rules in
conflict and make `HOST-INV-013` cite a clause that contradicts it.

### 3. It declares no uncertainty and moves no arrival counter

Clause 19's declaration obligation is an **adapter's**, and it exists because an untimestamped adapter must not
pretend to be exact. A simulated producer is exact, so it has nothing to declare, and reporting it as
arrival-stamped would put a fallback measurement in the report for a fallback that does not exist. The diagnostics
report counts it under its own name or not at all; it never borrows another provenance's counter.

### 4. It is not a claim about any hardware clock, and nothing may read it as one

`Simulated` carries no qualification of a physical adapter, no bridged origin and no measured drift. ADR-0022 and
Phase 9 own those, and the exit gate's equivalence claim is scoped by this clause: the fixture proves the scheduler
boundary and nothing about a CPAL or MIDI timestamp. That scoping was already Phase 3's stated limit; this value is
what makes it visible in the data rather than only in prose.

### 5. No live adapter may stamp one, and what enforces that is named rather than assumed

A live adapter that stamped `Simulated` would be claiming exactness it did not measure, which is F11's defect
wearing a different tag. The rule is therefore stated as a prohibition on adapters.

**What a source scan can establish, and what it cannot.** A standing scan — in the spirit of the tempo module's
four-operations scan and the render loop's purity scan — can assert that this repository constructs
`TimeSource::Simulated` in exactly one place, the simulated producer's own. That is worth having and it is what the
implementing slice owes. It does **not** establish that a release build cannot *call* that producer, and it cannot
constrain a downstream consumer of a public enum at all; an independent review found this record claiming
otherwise.

**The two gaps are not the same kind of thing, and an earlier draft offered one mechanism for both.** That a release
build cannot reach this repository's producer is enforceable, by a visibility or feature gate on the producer, and
**choosing which is the ingress slice's** on the shape that slice takes. That a downstream consumer of a public enum
never constructs the variant is **not** enforceable by any mechanism this crate has — the enum is public and its
variants are nameable. It is a contractual prohibition, and this clause is where it is stated: an adapter that
stamps `Simulated` is claiming exactness it did not measure, whoever wrote it. An independent review found the two
merged into one closable obligation.

This record fixes what the tag means and names both obligations; it does not pre-commit the mechanism for the first
or pretend the second has one.

### 6. Every existing match stays total, and that is checked rather than assumed

`TimeSource` is not `#[non_exhaustive]`, so adding a variant breaks every exhaustive match — inside this crate, which
is the API break named below. The break is the point: each site is made to answer for the new value rather than
inheriting a default. `is_ingress()` is the one site whose answer is load-bearing, and clause 2 is its answer.

## Consequences and risks

- **Accepted cost.** An accepted clause is amended, and `synth_engine_v2::time::TimeSource` gains a variant, which
  breaks an exhaustive match for any consumer. The crate is experimental and is not a dependency of the workspace's
  default members; `crate_boundary` permits one in-repo consumer, `pertylizer` as a dev-dependency for the
  measurement examples. **The maintainer approved this break on 2026-08-31**, on the same footing as
  `EventLimits::new`'s and `AdmissionError`'s, and it was asked for separately because no earlier approval covers
  it.

  **The variant lands with the producer that stamps it, not with this record.** Nothing constructs a `Simulated`
  envelope until the simulated-ingress slice exists, and this repository builds the reachable branch rather than the
  one a later phase will reach. So acceptance decides what the tag means and leaves the enum as it is; the slice
  that adds the producer adds the variant under the approval above.
- **Safety/correctness control, owed by the implementing slice rather than claimed here.** Four checks, named so the
  slice cannot quietly ship without them: `Simulated` is ingress, so an event beyond the horizon offered with it is
  rejected and counted **and** the same event inside the horizon is admitted — the pair, because either alone passes
  with the check inverted; it moves no arrival counter, falsified by a mutation that folds it into
  `arrival_stamped`; and the standing source scan finds exactly one construction site in this repository. None of
  them exists today, and this record does not assert otherwise. The scan is **not** evidence for clause 5's whole
  prohibition, which needs the boundary that clause names. The exit gate's equivalence fixture is what proves the tag is
  reachable at all.
- **Owed, and named rather than discovered.** The diagnostics report gains a counter or a documented decision not to
  have one; that is the implementation slice's, and clause 3 fixes which of the two it may not do. Phase 9 owns
  whether a real adapter needs a finer provenance axis, which is option E.
- **Revisit condition.** Phase 9's first real adapter reopens the axis if a bridged-and-measured stamp and an
  unmeasured one have to be told apart, since both are `Hardware` today. A second in-engine producer that is *not*
  deterministic would reopen clause 1, because "exact by construction" is what the value asserts.

## Specification update

Acceptance amends **ADR-0032 clauses 18 and 21** — the first to four values, the second because its enumeration of
the provenances the forward horizon binds is explicit — and updates the host-profile and render-limits
specification's **`HOST-INV-013`**, whose enumeration of the provenances the forward horizon binds becomes
`Hardware`, `Arrival` or `Simulated`. The invariant's other halves — evaluated exactly once, at ingress admission,
against the timestamp as stamped — are unchanged.

The specification is what implementation follows, so it states the rule now even though the variant it names arrives
with the producer. That is the ordinary order in this project: the contract precedes the code that satisfies it, and
a specification describing three provenances while the accepted decision fixes four would be the disagreement
`PROCESS.md` requires to be repaired rather than tolerated.

## Review

Reviewer: Codex, an independent semantic review of the drafted record and one focused reread of its repairs. Three
findings, all repaired: the record amended ADR-0032 clause 18 while its horizon decision also changes clause 21's
explicit enumeration, which would have left two accepted rules in conflict; clause 5 claimed a source scan could
enforce a prohibition it cannot reach; and the repair of that then offered one mechanism for two obligations of
different kinds. The last two both narrowed clause 5 rather than restating it.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
