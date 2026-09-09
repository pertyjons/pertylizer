# ADR-0060: Bounded Note YAMS Ownership and Publication

| Field | Value |
|---|---|
| ID | ADR-0060 |
| Status | Accepted |
| Phase | 7/9/10A |
| Created | 2026-09-09 |
| Last reviewed | 2026-09-09 |
| Related | ADR-0008, ADR-0032, ADR-0050, ADR-0053, ADR-0054, EVD-0020, P07-S007 |
| Supersedes | Prior ownership, provenance and source-registration rules for the new finite Note consumer only |
| Superseded by | — |

## Durable boundary

P07-S007 introduces the first real authored-runtime producer. Its identity
minter, release obligations, quantum inputs and publication share cross a
real-time ownership boundary. Deferring these choices would let a consumer
commit to callback-dependent values or an unbounded event source. They must
close before this source can be enabled. This decision does not change a
persisted format, wire protocol or a shipping dependency.

## Decision boundary

The current engine has a bounded Note VM, compiled parameter scheduling,
identity partitions and one publication arbiter. Its existing split stream
ownership supports an off-thread minter and audio-thread registry; it cannot
safely share a mutable minter with an additional source during activation.
The new consumer therefore owns both fresh halves exclusively after preparation.
It has no runtime input or activation interface and refuses other note producers.

The consumer transforms a complete, finite raw input list at render time. Its
Note node captures previous-quantum Control and parameter values into semantic
state through the graph schedule. Runtime transformation occurs once per due
note before publication. Its output time is exact engine time, distinct from
both compiled timeline output and external simulated ingress.

## Evidence

`authored::tests` verifies callback partition invariance, the input-capture
oracle, local automation, release-class accounting, terminal silence and
allocation-free maximum-source rendering. `identity/hot.rs` and `tempo/hot.rs`
place the newly reachable bounded operations inside the purity region.
[EVD-0020](../evidence/phase-07/EVD-0020-authored-note-capacity.md) owns numeric
selection; its committed measurement is required before ordinary enablement.
No timing or production-live capacity conclusion is assumed by this decision.

## Options

1. Sample the last completed callback from another thread. Rejected because
   callback partitions then select different semantic input values.
2. Add runtime note offers beside activation and live ingress. Deferred because
   it requires another admission/handoff contract and release authority.
3. Prepare a finite source, give it exclusive stream ownership and explicit
   previous-quantum inputs, and publish through the existing arbiter. Selected.

## Decision

SOUND-INV-030 is the complete current contract. N total inputs reserve at most N
identities, N distinct destination gates and N potential release obligations;
2N bounds both authored destination occupancy and retained-future declarations.
Pending reservation transfers into an actual hold rather than adding another
entitlement. Same-batch on/off uses AuthoredRuntime for both; a later release
redeems its hold under Release. Mandatory explicit cuts bound until-cut output.
A source refusal occurs before playback; a runtime violation is stream-terminal.

The shared Note seed extends ADR-0008 with an explicit AuthoredOccurrenceId,
resetting registers per invocation. The assigned voice and list order are not
seed inputs. `TimeSource::Authored` extends the provenance set with exact,
engine-generated output and is not ingress. Existing tag meanings remain.

This exclusive owner is the new case beyond ADR-0050's split ownership. It owns
one minter, renderer and arbiter, with no detached copy, concurrent control half,
activation, seek or live/compiled note coexistence. It accepts a separately
admitted compiled stream of parameter/controller writes and publishes those
under Compiled in its shared window. Every missed event in this owner, including
a compiled write, is terminal. A renderer refusal after producer state advances
also ends the stream rather than retrying partial state.

For ADR-0054 source registration, a non-dropping authored store belongs in the
host-profile specification's separate **Non-dropping authored source registry**.
HOST-INV-009's live-input registry remains a drop licence and receives no new
row. EVD-0020 must select the finite source's capacity before its qualification
flag enables an ordinary consumer. Phase 9's full simultaneous partition
selection is unchanged.

## Consequences and risks

- Accepted cost: a Note invocation reads explicitly previous-quantum inputs;
  a large host callback takes Q-sized publication subcalls. The complete input
  count is conservatively bounded, so this is not unbounded song streaming.
- Safety/correctness control: fixed source storage, exhaustive finite-duration
  mapping validation, one authoritative identity table, attributable terminal
  counters and zero allocator activity during rendering.
- Revisit condition: another source, runtime input, activation, stealing, voice-
  scoped Note programs or an unbounded input stream requires a new admitted and
  measured contract. P07-R001 assigns saved Note Grid/rack semantics and lowering
  to Phase 10A; hot program replacement remains Phase 9's existing obligation.

## Specification update

Creates SOUND-INV-030, extends SOUND-INV-018's ownership model, and records the
non-dropping source under HOST-INV-021. The sound-core and host-profile
specifications present the coherent current rules. Existing accepted ADR prose
is retained as history; the decision index points to this successor.

## Review

Independent design consultations and the uncommitted review use Claude Code
with only Read/Grep/Glob, no MCP, commands or delegation. Findings are reported
before repair; boundary repairs receive a focused reread before commit.

Stopping rule: a false factual premise, contradictory ownership or capacity
rule, partial accepted playback, incorrect seed/time behavior, or evidence that
cannot support its stated selection blocks acceptance. Optional detail does not.
