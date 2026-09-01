# ADR-0054: Stage Producer-Capacity Calibration at Real Consumers

| Field | Value |
|---|---|
| ID | ADR-0054 |
| Status | Accepted |
| Phase | 3/5/7/9 |
| Created | 2026-09-01 |
| Last reviewed | 2026-09-01 |
| Related | ADR-0021, ADR-0046, `HOST-INV-021`, EVD-0017, EVD-0019 |
| Supersedes | ADR-0046's Phase 3 numeric-selection deadline |
| Superseded by | — |

## Decision boundary

The six shares and their admission relations are a real-time ownership boundary
consumed by later producer classes. The calibration deadline also binds Phase 3,
the first authored/internal producer, and Phase 9's production-live boundary.
Changing it therefore crosses the cross-phase and real-time tests in
`PROCESS.md`.

The decision is ready because the Phase 3 exit cannot measure producers that do
not exist without creating disposable implementations solely to satisfy a
deadline. No coupled safety choice is open: ADR-0046 already fixes ownership,
relations and failure behavior. Only the evidence point for numeric values
moves.

This record decides only when numeric calibration becomes binding. It does not
change producer ownership, the six-share partition, admission relations,
no-borrowing, release obligations or terminal failure behavior. It does not
qualify any current number for production live use.

## Context

ADR-0046 fixed six disjoint producer shares, a release-hold capacity, checked
admission relations and terminal behavior. It also required Phase 3 to select
their numeric values from measured producer occupancy before enabling the
contract. Four classes now have executable producers. Authored-runtime and
renderer-internal production do not, so Phase 3 cannot observe their real
occupancy without building placeholder producers solely for a measurement.

The wording also became ambiguous. The accepted ADR says both "before the
Phase 3 contract is enabled" and "before enabling ingress"; the master plan
later says "before enabling live ingress". A deterministic simulated producer
now exercises the ingress boundary, while no production live adapter exists.
This record replaces those deadlines with one staged rule rather than treating
one wording as more authoritative than another.

EVD-0019 establishes a separate fact: synthetically filling every publishable
share costs 0.034% to 0.052% of the callback budget in its measured profiles.
That supports the bounded publication mechanism, not the producer values.

## Options

1. Keep the Phase 3 deadline and build placeholder authored/internal producers.
   This produces measurements for implementations no consumer will use.
2. Treat the provisional partition as final because its synthetic publication
   cost is low. This confuses arbiter cost with producer occupancy and is
   rejected by the falsifier below.
3. Stage calibration at each first real producer and require one complete
   pre-live selection. This is selected because every number is measured before
   it can constrain a real downstream consumer.

## Decision

1. Phase 3 may use the provisional profile for compiled, session and simulated
   ingress coverage while every checked relation and terminal fault remains in
   force. This is an experimental qualification, not a production-live sizing
   claim.
2. The first slice that makes a real authored-runtime or renderer-internal
   producer executable measures that class's high-water destination occupancy
   and retained resources. It reselects the affected share or records why the
   conservative admitted bound is the selected value before a downstream
   consumer can enable that producer.
3. Before the first hardware or production live adapter can be enabled, Phase 9
   measures the complete simultaneously legal partition and reselects all six
   shares, `release_hold_capacity`, `max_events_per_quantum` and every live
   ingress depth. Fitting inside 256 does not waive reselection.
4. Until clause 3 passes, 256 and the default partition remain provisional.
   Release notes, UI and external APIs may not describe them as qualified live
   capacity.
5. A new producer class or source store still needs its admitting declaration,
   checked relation, registry row and fail-closed overload behavior before it
   can run. Staging the measurement does not stage those safety properties.

## Falsifier and stopping rule

This decision is false if an unmeasured real producer can be enabled by a
downstream consumer, if a production live adapter can start under the
provisional partition, or if the final selection is inferred from synthetic
publication cost rather than observed producer occupancy and declared bounds.
Any of those blocks the consuming slice. A request for a different benchmark
presentation does not.

## Evidence

- `HostProfile` construction and plan admission enforce the fixed share sum,
  positive shares, release-hold relation and producer declarations.
- EVD-0019 measures the publication mechanism at a synthetically full
  partition. It deliberately does not claim real producer occupancy.
- The current crate has no real authored-runtime or renderer-internal producer,
  which is the observable reason their occupancy cannot be measured at Phase 3
  exit.

The first downstream producer measurement falsifies this decision if it can run
without recording its high-water occupancy and reselecting or explicitly
retaining its bound. The Phase 9 gate falsifies it if a production live adapter
can start before the complete partition is reselected.

## Consequences and risks

- Phase 3 can close on its executable scheduler, admission and simulated-ingress
  outcomes without inventing two producers early.
- Phase 5 or Phase 7 pulls the internal/authored measurement forward when it
  introduces the corresponding producer.
- Phase 9 has an executable pre-adapter gate rather than a residual note at its
  exit.
- ADR-0046's ownership, checked relations, no-borrowing rule and terminal fault
  are unchanged. Only the numeric-selection timing is superseded.
- A provisional value may be inefficient or too small for a future real
  producer. It cannot silently borrow capacity or trim playback: the existing
  refusal/fault rules contain that risk until the producer's pull-forward gate.
- Revisit when the first authored-runtime or renderer-internal producer exists,
  when the legal producer combination changes, and at Phase 9's complete
  pre-live selection.

## Specification update

The host-profile specification presents the staged calibration deadlines and
marks every current numeric default provisional until the Phase 9 full-partition
selection.

## Review

Design consultation: a Claude Code read-only invocation on 2026-09-01 rejected
changing only the Phase 3 checklist because the old deadline was part of
accepted ADR-0046. This successor is the explicit amendment that consultation
required.

Independent semantic reviewer: a separate fresh Claude Code read-only
invocation reviews the final uncommitted transaction under the repository
stopping rule. Its result is recorded in REV-P03.

Stopping rule: a false producer-availability claim, weakened admission relation,
or path that enables an unmeasured real producer blocks acceptance. Editorial
detail does not.
