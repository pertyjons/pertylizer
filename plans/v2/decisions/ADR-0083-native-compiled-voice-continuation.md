# ADR-0083: Native compiled voice continuation

| Field | Value |
|---|---|
| ID | ADR-0083 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-17 |
| Last reviewed | 2026-10-17 |
| Related | ADR-0009, ADR-0010, ADR-0047, ADR-0048, ADR-0064, ADR-0077, ADR-0082 |
| Supersedes | ADR-0082's reset-only application selection for the bounded compatible case |
| Superseded by | — |

## Boundary and decision

The first sounding-state consumer is an internal native compiled-session mechanism,
not a public node migration hook. This accepts only the exact native program/layout
case carved out of ADR-0010; that general migration surface remains Proposed. It keeps the actual renderer, DSP storage,
identity table and authoritative control-side minter. No snapshot minter is promoted.
The plan objects remain immutable: installation changes the bound plan object and
checked plan-local addresses at a quantum boundary. There is no table rebuild, so
ADR-0048's general rebuild question remains Proposed. The coherent current rule is
[native compiled voice continuation](../specs/spec-host-io-lifecycle.md#experimental-native-compiled-voice-continuation).

Continuation requires identical physical execution layout, authored IDs and exact
scopes/tags/slots, native prepared configuration, voice policy, tuning and the whole
ordered semantic event program, including equal-time ordering and scalar bits.
Only authored non-voice Trim bases may change, with no smoothing, automation or
note-magnitude write to those rows. The application first consumes master Trim
changes. Scripts, controllers, feedback, samplers and structural voice edits choose reset.
Loop/capture and external producer ownership remain outside this running lane. Identical passive mixer/tap
descriptors are checked with plan-local addresses normalized; this session API
exposes no observer subscription to migrate.

The actual adopted stamped list L is copied before Play publication into bounded
cold metadata. Only an applied Play makes it authoritative. Its plan, epoch,
table, activation sequence and anchor must still match. Continuation relabels L
without minting, retaining every occurrence, generation, timestamp and ordering.
It prepares both the successor audio list and successor witness L' off-thread;
collection installs L' for a subsequent edit. Stale metadata selects reset.
A failed initial or private-reset witness copy is reported but leaves ordinary
Play/reset usable. Inconsistent bindings or continuation resource errors fail
preparation and retain old sound under the current fail-closed preparation policy. Reset fallback on a cold
resource failure is a possible later refinement, not an identity requirement.

At exact B with empty carry, no ordinary credits and unchanged acknowledged
Playing anchor, audio preflights all storage and live-note bindings before mutation.
Changed Trim rows must be exactly resting base state or normal Play-restored base
state, with no active ramp/controller/modulation contribution. Whole row state is
reset to the new base. The actual renderer retains oscillator phase, envelopes,
filter/delay histories, steal fades, ages, table generations and scheduler cursor.
Old immutable plan and event/catch-up owners move into the return envelope.
Collection rebinds the actual minter's plan-local note addresses, retaining its
issuers, identity table, generations and ledger. This is exclusive custody, not
parallel use of two renderers for the same table.

The first actual successor fragment must render before Installed. A zero-frame
call remains pending. Ordinary render cannot bypass the transition owner. A
post-install fault returns Interrupted with the actual successor binding closed
for teardown; old audio never resumes. Pre-install refusal leaves it untouched.
Application metadata and omissions follow successful rendered installation.
The GUI authority's lineage fence must hold through publication. A lower-level
caller without an attached authority must ensure graph identity continuity itself.
The host must preserve its end/loop policy. Repeated edits, Pause/Play and Stop retain the owning
lane and reset consumer's latest-wins rules. Identical complete note programs are
the same performance for this consumer, even after coalesced note deletion/re-add;
no persisted placement-identity claim is made. Graph identity replacement is
separately fenced by authoritative operation lineage.

## Bounds, falsifiers and stopping rule

`ContinuationLimits` separately bounds stamped event count, one witness's actual
capacity and a transition's actual capacities. Steal expansion counts as stamped
events. Per-command witnesses remain with their command until collection; at most
the bounded command credits plus one active witness are retained. A continuation
adds two event copies plus proof and fixed payload, all charged to its transition
limit. Old owners remain held until off-thread collection. The application uses
16,384 stamped events, 4 MiB per witness and 4 MiB per transition provisionally.
These bounds select no producer share or production aggregate memory/CPU budget.

Wrong release/expression routing, generation rewind, changed event order, lost
owning credit, partial mutation on refusal, callback allocation/destruction,
partition-dependent audio or Installed before successor rendering blocks acceptance.
Optional implementation detail does not. Core controls compare held native voices,
steal fades and later bends against normal single-plan Trim-write references at
44.1/48/96/192 kHz and whole, 1/64/256/512/irregular callbacks. They cover repeated
edits, actual minter rebinding, zero-frame pending custody, interrupted teardown,
stale/foreign metadata and resource refusal. Application controls exercise saved
voice, insert and return projects, lineage cancellation and reset-then-continuation.

General node relocation with sounding voices, changed voice configuration or
programs, mixed/live voice-state migration, observer handoff and production
qualification remain outside this decision. Paused non-voice migration and running
reset remain available under their own rules. Phase 9 is not declared complete.
