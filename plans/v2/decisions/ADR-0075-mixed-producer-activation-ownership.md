# ADR-0075: Mixed producer activation ownership

| Field | Value |
|---|---|
| ID | ADR-0075 |
| Status | Proposed |
| Phase | 9 |
| Created | 2026-09-26 |
| Last reviewed | 2026-09-26 |
| Related | ADR-0009, ADR-0022, ADR-0023, ADR-0046, ADR-0047, ADR-0048, ADR-0050, ADR-0051, ADR-0055, ADR-0058, ADR-0065, ADR-0072, ADR-0073, ADR-0074, EVD-0024 |
| Amends | None while proposed. Acceptance requires explicit amendments to affected ownership, release and host-profile contracts. |
| Supersedes | — |
| Superseded by | — |

## Problem and current boundary

Phase 9 needs one admitted stream that can render compiled playback and live
notes through transport activation and loop wrap. ADR-0050 clause 8 leaves
non-compiled hold redemption at activation unresolved and relies on control
being the only minter while a candidate is outstanding. Its clause 3
currently promotes the whole copied identity table. ADR-0051 clause 6's
catch-up may write the live voice's rows. The current
`LiveInputStream` owns control, ingress and renderer as one mutable value; its
offer receipt can still resolve to `AuditionOutcome::Refused` at staging.
EVD-0024 measures components, not this combined host.

This record is a design frame, not permission to lift those refusals. No
persisted, wire, shipping or production-facing contract changes here.

## Candidate ownership rule

Test a single immutable table identity with disjoint compiled and live
producer ranges. The audio callback would be the sole writer of the live
range's identity, hold and sounding state. A source thread would submit
bounded operations, not mint identities. In transport mode, control would
stamp candidates against a copy of only the compiled range and promote that
range at collection, before audio adoption. In loop mode, audio would own the
compiled range under a custody token. A mode transfer must revoke the old
writer before the new one mints. The live range must not be overwritten by a
compiled candidate or rebuilt at a loop boundary.

Plan admission would bind each note producer to its playable target node
instances and control rows. A one-instance node is shared even when producers
write different controls on it; that overlap refuses admission until a
shared-node law exists. A multi-instance node belongs to the producer whose
identity range contains that instance. The binding must be enforced when a
compiled stream is admitted and when live operations enter ingress, including
note edges, expression, bend and producer-scoped release groups. The renderer's
table identity check alone does not enforce a producer's range or playable target.
Compiled mass release, catch-up, slot seeding and loop restoration would be
scoped to compiled-owned per-note rows. Any per-note row reached by both
producers would refuse plan admission until a shared-row law exists. A
non-note target written by the live parameter lane also needs an explicit
catch-up ownership or ordering law; a compiled catch-up must not silently
overwrite its latest live value. The Session admission sum must cover the
catch-up batch, live parameter lane, Stop group and any reset or other
same-quantum Session contributor. The representation, share charge and
renderer scratch must be remeasured; the old one-row-per-address count is not
evidence for a scoped implementation.

An initial target-binding prerequisite may validate exactly one compiled and
one live note producer in one immutable plan, one bound live note slot and
`StealingPolicy::None`. Its off-thread artifact must own the entire admitted
compiled stream, not a caller-supplied list that can later be replaced, and
enumerate every gate and magnitude destination against each producer's
identity range. Foreign or unrepresentable targets refuse. This artifact
accepts only note-target writers: compiled `SetParameter` and `Controller`
writes refuse, and it provides no live parameter lane. `Fade`, `Reset` and
`RestoreController` require explicit accounting before a mixed host can emit
them; disabling stealing prevents the first two today but is not a proof for
a future emitter. The artifact does not prepare mixed ingress or authorize
rendering. Current mixed-plan, activation and loop refusals remain in force.
The later combined host must admit same-quantum capacity. A compiled-only
activation must neither write nor reseed live-owned instance-local control
slots, their current ramps, next live writes' ramp behavior or local
modulation histories. A shared upstream source or non-note target whose
change can affect a live instance requires its own declared ownership or
ordering law.

## Unresolved acceptance work

The candidate rule is insufficient until a concrete combined host answers
and tests all of the following. None is delegated to an implementer as an
unstated assumption.

Acceptance must explicitly amend ADR-0050 clauses 3, 5, 7, 8 and 9; ADR-0046
clauses 3 and 6 and ADR-0047 clauses 3 and 7 for the split source receipt,
audio acceptance and mixed-host release timing; ADR-0051
clauses 4 and 6; ADR-0072's exclusive-owner mint and release limit;
ADR-0073's Session-only Stop assumption and terminal credit-exhaustion policy
for protected releases and, if changed, its inherited-bend failure and other
credit-exhaustion policies;
ADR-0074's reset custody;
ADR-0055's loop-activation refusal; and ADR-0065's exclusive loop owner.
ADR-0009's separate reset owner must be
preserved or explicitly amended. Host-profile and sound-core render
invariants, including HOST-INV-009, HOST-INV-022, SOUND-INV-017 and
SOUND-INV-018, also need explicit updates. ADR-0055's loop-activation
refusal and ADR-0065's no-live-ingress limit remain in force until this
combined-host loop acceptance gate is met and those limits are explicitly
amended.

1. **Source acceptance and release custody.** Define which receipt is only a
   reservation, where HOST-INV-009 accepts a live note, and how source queue,
   occurrence, identity, hold, result, protected note-off and any inherited-bend
   credits compose.
   Decide and test the exact redemption boundary, source/channel scope and
   late-edge outcome for individual release, key-up under sustain, pedal lift,
   Stop, Panic, reset/crossfade and stealing if admitted. In particular, a
   key-up under sustain cannot free a hold for a still-gated note, and a
   compiled-only activation must leave live holds untouched. State whether a
   crossfade ends obligations at adoption or after its tail; do not infer
   timing from old-owner retirement. A drop before acceptance must create no
   hold, and an accepted note's protected release cannot be silently lost;
   reset, Panic or an admitted steal needs a distinct terminal disposition.
   Decide whether inherited bend is precharged atomically or retains
   ADR-0073's terminal callback policy, and amend that policy if changed.
   If any terminal fault path remains in the combined host, acceptance must
   select whether it ends the whole stream, only the live partition or only
   the callback. It must define callback rollback, fault occurrence and time
   across callback partitions, compiled playback and candidate custody, loop
   custody, raw capture, accepted live holds and releases, later source
   operations, parameter versions and a fading old owner. This may be in this
   ADR or a separately accepted contract; a changed ADR-0073 host-termination
   policy needs an explicit amendment.

   Pedal-up, Stop/Panic and reset/crossfade need bounded, protected
   mass-release work, including a fading old owner. Every shortage and
   cancelled edge needs a named outcome and diagnostic category. Before a
   mixed host runs, joined teardown must also classify a still-held onset,
   an accepted pending release and an unexamined source packet; no one of
   those may disappear with the owner.
2. **Command order and generations.** Define one quantum-boundary order for
   Stop, Panic, Start, reset/crossfade, activation, loop entry/exit/wrap and
   source operations. Specify token publication, stale candidate rejection,
   stopped-state behaviour, already-adopted generation collection and
   source releases after a command. A normal release tail must be allowed
   while a gate and identity are ended. A prior drop or steal must keep its
   original terminal classification.
3. **Owner lifetime.** Prove that a compiled activation and loop handoff keep
   the exact live renderer, rows, holds and ramp. A whole-renderer reset may
   instead end them, but must reconcile receipt and result custody before
   old-owner retirement. Refuse identity-table rebuild while either producer
   range has outstanding obligations, as ADR-0047 clause 8 currently
   requires; resolve ADR-0048 before enabling any mixed-host rebuild path.
   Bound active and fading owners, return slots and
   final destruction off-thread. Define how a reset while stopped affects a
   prepared Start.
4. **Capacity and stealing.** Admit the worst-case same-quantum live and
   compiled release fanout, including a fading owner. Keep ADR-0058's victim
   search off audio or explicitly amend it with a bounded real-time proof.
   ADR-0065's loop-mode `StealingPolicy::None` remains in force until amended.
   Re-run the full producer partition on the common host; component evidence
   alone cannot qualify production values.

## Falsifiers and order of work

For the same admitted source and command prefix, single-callback, 64-frame, 256-frame
and irregular runs must have equal output, outcomes and raw take when every
compared callback succeeds. If any terminal fault path remains in the host,
the eventual fault contract must state whether fault occurrence and time are
invariant across partitions or may differ under a named rule, and must give
the rollback criterion before acceptance. A
compiled activation or loop transition that cuts, retriggers, re-pitches or
jumps the ramp of a held live note falsifies the ownership rule. A stale
candidate that reissues a spent generation, a hold stranded, redeemed twice
or freed at key-up while the note remains sustained, a release silently lost
without a named terminal disposition, an unreported receipt, or a callback allocation, lock or
final drop also blocks acceptance. The eventual redemption law must add
falsifiers for each release cause, source/channel scope, chosen boundary,
late-edge and cancelled-onset/release outcome, crossfade ending point, and
diagnostic category before
acceptance. It must also falsify the selected inherited-bend policy, each
source queue, occurrence, identity, hold, result, protected note-off and any
inherited-bend shortage outcome, and the per-quantum fanout and credit bound
for pedal-up, Stop, Panic and reset/crossfade mass release, including a fading
old owner. If any terminal fault path remains, its contract must add
falsifiers for selected scope, fault occurrence and time across partitions,
callback rollback, compiled output and candidate disposition, loop
generation, raw capture, accepted live holds/releases, fading-owner custody,
post-fault source operations and parameter-version disposition before
acceptance.
A test suite that never combines held live
notes, transport commands, loop transitions and saturated credits cannot
discharge these falsifiers.
A target-binding artifact that accepts two note producers whose playable
targets reach the same per-note row or one-instance node falsifies its first
slice, even if that node's control rows differ; a later shared-node law needs
separate acceptance. Within the first slice's declared shape — exactly one
compiled and one live producer, one bound live slot, `StealingPolicy::None`,
valid slots and note-target writers only — refusing disjoint multi-instance
node rows also fails its supported case. Substituting a compiled stream or live
slot after binding falsifies that artifact. A later mixed ingress that accepts
a cross-producer release group fails its producer binding. No mixed owner may
accept Start, Stop, Panic, activation or loop commands until that command's
scope, hold redemption and outcome law is explicitly amended and tested.
Compiled-only catch-up must not write or reseed any live-owned instance-local
control slot, current ramp, next live write's ramp behavior or local
modulation history. A shared upstream source or live parameter-lane target
that can affect that instance follows its declared ordering law or refuses
admission. The combined Session charge and
same-quantum producer total must fit admission.

One candidate order is to build an isolated audio-owned live ingress with
source/result/held-cell credits and protected releases, then add
producer-to-target admission and scoped restoration, then integrate compiled
partition custody and test the full command matrix above. The target-binding
prerequisite leaves every mixed-plan, adopted-store, activation and loop-owner
live-ingress refusal in force. A later implementation may lift only the
refusal whose replacement contract and falsifiers it has proved and explicitly
amended; ADR-0072's exclusive owner remains unchanged until that boundary.
EVD-0024's component matrix remains
insufficient for production capacity; ADR-0022's hardware timing and physical
round-trip gates remain separate.

## Review and stopping rule

An independent design consultation compared a cross-thread live ledger with
an audio-owned live range and identified the latter as the candidate above.
Repeated independent semantic reads of a broader draft found unresolved
Stop/Start/reset, pedal, release and capacity contracts. Those rules were
removed from this proposal and are listed as acceptance work, not decisions.

A false code premise, conflicting ownership, a release without guaranteed
credit, an ambiguous ordering rule, or real-time allocation, lock or final
drop blocks acceptance. Optional implementation detail does not.
