# ADR-0057: Refuse a Parity Verdict Over a Placed Note

| Field | Value |
|---|---|
| ID | ADR-0057 |
| Status | Accepted |
| Phase | 4/6 |
| Created | 2026-09-02 |
| Last reviewed | 2026-09-02 |
| Related | ADR-0025, ADR-0028, ADR-0042, `SOUND-INV-021`, `P04-R001`, `P04-R002`, `P04-R004`, `CORPUS-0001-P2` |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

Two boundaries, and the first is the one `PROCESS.md` names last: this **required an
explicit product choice from the user**. What a lowered payload means — how hard a note
was struck, or the amplitude that striking produces — is a product question, and so is
whether Phase 4's gate may keep asking for evidence the phase cannot produce. The second
is **cross-phase**: the obligation this record leaves open is bound to Phase 6, and two
further bullets of the Phase 4 gate are bound to Phase 10A and Phase 10B.

**Why it is ready.** Phase 4's exit is the active slice and cannot proceed without it. The
alternative to deciding is not deferral but one of two failures: a gate that claims parity
evidence the phase cannot produce, or a phase blocked on Phase 6 — which depends on Phase
5, which depends on Phase 4, so the block would close a cycle the master plan forbids.

**Coupled decision.** [ADR-0025](ADR-0025-tuning-representation-and-ownership.md) is
accepted and is what makes this record decidable: the payload's meaning is settled, so the
only open question is what may be *claimed* about a render built from it. This record does
not reopen it.

## Decision boundary

It decides the **reporting** boundary while V1's velocity composition is unimplemented, and
the phase-exit consequences that rest on it.

It does **not** decide Phase 6's composition law, does not change the note payload or
`SOUND-INV-021`'s key and velocity clauses, does not accept
[ADR-0028](ADR-0028-long-running-job-contract.md), and does not change what a saved project
renders. A lowering that places a note still renders; what it may not do is be compared.

**The verified premise.** V1 consumes one saved velocity twice, under two independent
sensitivities. `synth_modules::math::velocity_sensitivity` returns
`1 − sensitivity × (1 − velocity)` and the envelope multiplies its emitted level by it;
`Voice::process` then applies `(1 − amp_sens) + amp_sens × velocity` at voice output. Both
sensitivities default to `NormalizedValue::MAX`, so both factors reduce to `velocity` and
V1's product carries it squared. V2 applies it once, as one scale on the envelope, which is
what `SOUND-INV-021` requires. For the corpus's saved `0.7559055` the two renders stand at
`0.572` against `0.756`: **2.4 dB** at the sustain, against the `-0.0027` dB
[EVD-0013](../evidence/phase-02/EVD-0013-minimal-patch-equivalence.md) measured there for the
envelope difference [ADR-0042](ADR-0042-envelope-segment-shape.md) accepted as intentional.

## Evidence

- `pertylizer`'s `a_render_that_places_a_note_still_refuses_a_parity_comparison` asserts the
  `UnsupportedScope` marker, the diagnostic's named owner, and that it is raised once per
  lowering rather than once per note over a four-note song.
- `Fidelity::of` derives the verdict from the diagnostics rather than taking it as an
  argument, so an outcome holding an `Unrepresented` diagnostic and claiming `Faithful` is
  not constructible, and `admits_parity_comparison` is false for it.
- `a_saved_notes_own_velocity_reaches_the_render` pins the V2 side as a **ratio**: half the
  velocity renders half the peak. It is the test option 1 below would move.
- `exactly_two_saved_projects_in_the_repository_lower_to_a_plan` lowers all 28 saved
  projects the repository holds, in both saved forms and every instrument of each, and
  asserts the eligible set exactly. It is the measurement the corpus-count clause rests on.
- `CORPUS-0001-P2` is the declared corpus claim this refusal actually holds: the envelope's
  landmarks staying within measurement tolerance of V1's. `CORPUS-0001-P1` and
  `CORPUS-0009-P2` are representable now and are simply unjudged.

**Uncertainty that could change this.** If Phase 6's composition law turns out to be
expressible as one payload magnitude after all, clause 1's refusal becomes stricter than it
needs to be. That is a reason to revisit at Phase 6, not to weaken the refusal now: a
comparison admitted early cannot be un-reported.

## Options

1. **Compose in the lowerer.** Send the product of V1's two response factors — not of the
   two sensitivity settings, which are `1 × 1` at the defaults — as the one magnitude the
   payload carries, making parity claimable in Phase 4. Rejected: the payload would carry an
   amplitude scale rather than how hard the note was struck, which ADR-0025 declined on its
   own merits, so that record would need amending rather than working around; and it moves
   the `0.5` peak ratio a current test pins.
2. **Make Phase 6 a prerequisite of the Phase 4 exit.** Rejected: Phase 6 depends on Phase
   5, which depends on Phase 4, so the amendment alone does not remove the cycle — the phase
   order or the law's ownership would have to move with it.
3. **Refuse the comparison and carry the obligation as a named residual.** Selected. The
   behaviour fails closed, the owner is already named by ADR-0025 and `SOUND-INV-021`, and
   `PROCESS.md`'s phase-exit rule supplies the mechanism, provided the gate is rewritten not
   to claim the deferred behaviour — which clause 3 below does.

## Decision

1. A lowering that **produces a performance** placing a note raises one `OwnedByLaterPhase`
   diagnostic naming the capability and its owner, and the outcome's fidelity is therefore
   `UnsupportedScope`. No parity verdict may read an outcome that is not `Faithful` — a
   lowering refused earlier reaches that verdict through its own refusal rather than through
   this marker, which is a guarantee about lowerings that succeed. The render is unaffected.
2. The obligation is `P04-R001`, owned by **Phase 6** with the composition law. Phase 6
   inherits it before it builds that law. Naming Phase 6 the owner of a *residual* is not
   naming it a prerequisite, so the dependency still runs 4 → 5 → 6.
3. Phase 4's Part I exit gate and the `ROADMAP.md` outcome are amended not to claim the
   deferred behaviour, as `PROCESS.md` requires of a residual exit. The gate's third bullet
   asks for the refusal instead of the verdict; its last bullet asks that no V2 streaming,
   progress, cancellation or shared render surface be built, which is `P04-R004`; and the
   named outcome no longer claims that current projects render through the *same headless
   comparison path as V1*, because the join between the two paths is what these two
   obligations defer.
4. The first gate bullet's project count becomes the **measured** eligible set rather than a
   fixed three. `P04-R002` established that the repository holds two, so the alternative was
   a project authored to satisfy a count rather than to demonstrate a capability.
5. While clause 1 stands, nothing may issue a parity verdict over a **lowered** outcome that
   is not `Faithful`: no development-only offline engine selection over saved projects, no
   corpus A/B batch, and no harness that reports such a lowering's render as V1's. The scope
   is the lowered outcome, not every controlled V1/V2 comparison — an evidence harness that
   builds its own fixture patches and compiles a V2 graph directly, as EVD-0013's does, never
   produces a lowered outcome and is untouched. Phase 9 and Phase 12 may not treat this
   refusal as the product's comparison behaviour.
6. This record does not extend to `P04-R004`, whose owner and constraints remain
   ADR-0028's. It is named in clause 3 only because the same amendment carries it.

## Falsifier and stopping rule

This decision is violated if a lowering that places a note is reported as `Faithful`, if any
parity verdict is issued over an outcome that is not `Faithful`, if the diagnostic omits the
owner, or if the marker is raised per note rather than per successful lowering — the last because a per-note diagnostic
would make the marker a property of the project rather than of the lowerer. Any of those is
a correctness defect and blocks the consuming slice. The wording of the diagnostic does not.

## Consequences and risks

- **Accepted cost.** Phase 4 exits with a narrower outcome than it was given: it delivers the
  V2 side of the comparison path, not the join between the two. The corpus's own claims are
  unjudged rather than met, and no A/B evidence exists for any saved project.
- **Safety/correctness control, stated as what it is rather than more.** The verdict is
  **derived** from the diagnostics rather than set, so an outcome cannot claim `Faithful`
  while holding a diagnostic; and clause 5 keeps the first comparison consumer from
  appearing before the law that would make it honest. It is **not** an encapsulation: a
  lowered outcome's samples and diagnostics are public, so a caller could compare them
  without asking for the verdict. What prevents that today is that no comparator exists and
  clause 5 refuses building one — an independent review corrected an earlier, stronger claim
  here. The first consumer brings the checked API with it, and inherits `P04-R001` first.
- **Risk: the residual is read as a deferral with no end.** Control: it blocks its first real
  consumer, and Phase 6 cannot build the composition law without discharging it.
- **Revisit condition.** At Phase 6, when the composition law is decided, or earlier at the
  first consumer that needs a parity verdict over a placed note.

## Specification update

Acceptance creates
[`spec-project-lowering-and-fidelity.md`](../specs/spec-project-lowering-and-fidelity.md),
prefix `LOWER`, which is the current contract implementation follows: `LOWER-INV-002`
derives the fidelity verdict, and `LOWER-INV-003` is clause 1's refusal and clause 5's
scope. It exists because the Sound Core render contract explicitly excludes project
lowering, so without it this record's rules would survive only in an ADR and a phase gate —
which an independent review identified as leaving no authoritative current contract once
the phase closed.

`SOUND-INV-021`'s conformance row records the refusal and Phase 6's ownership of the
composition; no Sound Core invariant changes, because this record decides what may be
*claimed* about a render rather than what a render does.
`spec-host-profile-and-render-limits.md`'s `LIMIT-0004` row moves its job-admission
obligation from Phase 4 to Phase 10B with ADR-0028's deadline. Phase 4's Part I exit gate
and `ROADMAP.md`'s Phase 4 outcome are amended as clause 3 states, and REV-P04 accepts the
exit against them.

## Review

Design consultation: the three options and their costs were put to the user on 2026-09-02,
who selected option 3 together with clause 4's count amendment.

Independent semantic reviewer: `codex review --uncommitted` over the uncommitted exit
transaction. Its first pass found the named roadmap outcome still claiming the deferred
comparison path while the review asserted no outcome was weakened; clause 3's outcome
amendment is the repair. Its second pass found this record missing, and required the
product-level amendment to have a durable home rather than living only in the master plan's
Phase 4 section. Both are recorded in REV-P04.

Stopping rule: a render reported as faithful while the composition is unimplemented, a
parity verdict issued over a placed note, or an amendment that leaves any authority still
claiming the deferred behaviour blocks acceptance. Editorial detail does not.
