# ADR-0090: Fixed native mixed continuation

| Field | Value |
|---|---|
| ID | ADR-0090 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-19 |
| Last reviewed | 2026-10-19 |
| Related | ADR-0050, ADR-0081, ADR-0083, ADR-0087, ADR-0088 |
| Supersedes | — |
| Superseded by | — |

## Decision

A mixed source owner is identified by its transfer serial, table and epoch, not
by the replaceable DSP plan. Its key-up handles, queued raw commands and source
frontiers remain valid across an admitted in-place plan continuation. This is
an experimental Rust boundary change, not a persisted or wire-format change.

The first mixed continuation admits only identical native voice geometry,
producer partitions and compiled programs modulo plan-labelled addresses.
Global Trim base changes are permitted under the existing continuation proof.
[ADR-0093](ADR-0093-real-mixed-producer-load-consumers.md) excludes targets
owned by a runtime parameter program; their base changes select explicit reset.
Structural changes and logical note reindexing are refused. The renderer keeps
its actual registry, state, histories and identity authorities; it never imports
another minter or an identity snapshot. All prospective addresses in actual
minters, queued ingress, active compiled events, current program custody and
loop templates are checked before mutation and relabelled at one quantum
boundary. Already redeemed historical end records keep their original labels.

One owning packet and one cold credit bound outstanding continuation custody.
Preparation completes proofs, renderer preparation and metadata admission before asking
the host for a fresh aligned sample boundary. The metadata allowance charges
actual vector capacities and inline packet storage; independently admitted
immutable plans and renderer buffers are additional resources. This allowance
is neither a total-memory claim nor production calibration.

Installation runs before same-boundary compiled Stop, activation and live
command service. Initial adoption must already have completed. A pending or
uncollected activation, reset seal, mismatched activation sequence, late boundary,
fault or invalid prospective binding refuses before changing actual state.
Cold activation custody is counted from successful preparation through owning
collection or cold destruction, including cancelled/refused receipts taken out of
the audio slot. Continuation refuses while any such owner exists.
Cold plan-specific activation, reset and continuation preparation cannot overlap
an outstanding continuation credit. Ordinary source input and protected Panic
continue through the existing ordering law. The reset wrapper additionally
requires empty secondary custody.

Every completed packet returns owning custody. InstalledRendered confirms that
the complete installing callback succeeded; InstalledFaulted records that the
new geometry became actual before a terminal callback fault. Both outcomes
promote the actual cold plan metadata, but only the former evidences successful
output. A fault publishes the existing terminal ingress halt. Refusal retains
the old actual plan. Quiescent withdrawal cancels an unpublished installation
without requiring another callback; joining refuses unresolved custody intact.
A weak packet-lifetime marker permits explicit quiescent recovery after a host
destroys a packet without collecting it. An unpublished packet clears its credit.
For an installed packet, recovery re-admits only immutable plan/program metadata
from the actual renderer and unchanged score; it preserves both actual minters
and registry. InstalledUnconfirmed explicitly supplies no successful-output
evidence. A foreign or still-live packet refuses recovery before mutation.

## Falsifiers and stopping rule

Acceptance fails on changed samples in a neutral continuation compared with an
unedited normal renderer; lost or rebound live identity; changed authority,
source frontiers or release ownership; partial mutation after refusal; callback
allocation or deallocation; lost packet custody; or successful-output evidence
for a faulted callback. Controls include repeated replacements at four sample
rates with irregular callbacks, warm delay history, future ingress, later key
release, running loops, two actual simulated raw-source paths with sustain,
protected Panic at the edit boundary, malformed target refusal and no-callback
withdrawal. These controls qualify the bounded library consumer only. The
application route, broader voice edits, producer qualification and physical
measurements remain separate work. A contract hole blocks acceptance; optional
implementation detail does not.
