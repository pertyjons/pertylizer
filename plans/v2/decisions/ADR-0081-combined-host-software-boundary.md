# ADR-0081: Combined-host software boundary

| Field | Value |
|---|---|
| ID | ADR-0081 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-17 |
| Last reviewed | 2026-10-17 |
| Related | ADR-0046, ADR-0047, ADR-0048, ADR-0050, ADR-0051, ADR-0054, ADR-0075, ADR-0076, ADR-0078, ADR-0079, ADR-0080 |
| Amends | ADR-0078's default-off host boundary; ADR-0050 clause 8 and ADR-0051 solely for this bound mixed owner; resolves ADR-0048's mixed stopped-owner-reset case only |
| Supersedes | ADR-0075's software-boundary deferral only; its complete production qualification remains deferred |
| Superseded by | — |

## Boundary

The maintainer requested continuous rolling delivery and the common compiled/live
input boundary on 2026-10-17. This accepts the software ownership and routing law
for the existing `mixed-transfer-rehearsal` feature. It changes no manifest,
persisted format, wire contract, shipping dependency edge or physical adapter.
ADR-0075 remains deferred for full combined-host qualification; ADR-0076's
production mixed refusal and ADR-0054's complete reselection remain in force.

The bound owner has one immutable plan, disjoint compiled/live identity ranges,
one compiled producer, one live producer and two simulated input connections.
Target admission refuses shared playable instances and crossed per-note rows.
There is no live parameter writer, stealing or different-plan migration here.

## Ownership and admission

The audio half is the only authoritative minter for both producer ranges for its
whole lifetime. Compiled preparation carries pairing tokens, never a working
copy to promote over a live range. Live commands carry occurrence handles; exact
key-ups and sustain lifts spend places reserved by their own accepted onsets or
pedal-downs. Raw event IDs never become sound occurrence identity.

Compiled Stop, Start, seek and loop activation affect compiled-owned notes and
restoration rows. They do not redeem live holds, reseed live ramps or change
live expression. Panic and a whole-owner reset have their declared wider scope.
Shared playable targets remain refused rather than acquiring an implicit
aggregation rule. Existing immutable target admission, scoped restoration and
the mixed owner implement these ADR-0050/0051 obligations. ADR-0048's mixed
whole-owner-reset case ends old live occurrences and keeps source key-ups as
reset tombstones; general identity-table rebuild/migration remains proposed.

Arm checks the actual host queues, release backlog, protected Panic place,
source stage/held bounds and bend-scope storage. A capacity refusal is not a
claim that another profile is measured or suitable. The six producer shares,
total event cap, release reserves and queue depths remain provisional outside
their exact frozen EVD populations. Unknown authored/internal publishers remain
refused; there is no inferred spare share for them.

## Common input route

`MixedBridgeControl::route_raw_first` is the shared off-thread entry. It consumes
the original source packet identity and observation, maps its configured raw
source clock and completes raw admission before sound staging. A sound refusal
does not erase raw admission. It returns raw ID/pairing metadata and the exact
sound stage/frontier outcome separately. A raw admission or metadata failure
returns the original and any already admitted raw ID and publishes the terminal
bridge signal; the host retains that refusal and requests its shared/raw halts.
An already halted bridge admits no additional raw observation. A raw `State`
refusal racing an unclassified self-halt reports source-neutral `RawSelfHalt`
and preserves the source head; only a classified late halt continues sound.

`MixedRawPolicy` binds the raw owner's closed source generations and classifies
actual correlated outcomes under ADR-0079. A foreign policy/outcome cannot
authorize another host's raw-only interruption. A terminal
outcome dominates a late one in either order. Only a classified late raw halt
skips raw for both sources while routing sound onward. Unknown self-halts and
direct raw render/preparation errors remain terminal. The host retains every
original receipt and direct error; classification never replaces diagnostics.
Source retries, raw completions and joined teardown remain custodial.

## Strict promised-prefix mode

The optional strict mode is for sources promising complete future prefixes. It
does not replace tolerant source fencing or ADR-0079's raw-only interruption law.
A strict host missing a control publication closes its whole callback; an
ordinary tolerant host can instead retain late input's separate outcomes.

`MixedPublication` is opaque and belongs to the bridge and active transfer. Its
exclusive end is the minimum of the actual publication window, both consumed
source frontiers, each earliest unforwarded stage head and any staged Panic.
It reads actual bridge state, never an old `Idle` reason or caller-supplied end.
Each earlier source observation was forwarded or settled. A later asynchronous
host Panic/transport request is not a source promise and may be late; its
nominal, forwarded and execution times remain explicit in the existing reports.

`MixedRawPublisher` advances an opaque `MixedRawPublication` only after a real
FIFO push succeeds. Preparation, `next_packet() == None` and a failed push grant
no credit. The host retains a failed packet and retries it before younger
packets. The raw proof binds both source generations and survives a sound reset.
Queue implementations and callers must honor the FIFO/exact-retention contract.
After an actual classified late halt, a separate opaque interrupted raw proof
retains its last queued frontier and exempts only further raw coverage. It never
claims that more raw data was sent. Sound coverage and terminal halts still apply;
strict mode cannot turn ADR-0079's raw-only interruption into a later sound stop.

Audio acquires a publication before draining its raw and sound commands. The
guard accepts only the matching bridge/active arm and raw source generations.
It retains one complete sound/raw pair, never component-wise maxima from
different snapshots. Required coverage comes from the actual renderer's clock
and carry, rounded to whole opened quanta. Failure latches the first host reason;
the host silences the complete callback and requests both halts before raw can
render it. A late complete proof cannot revive the guard.

The common publication channel replaces its latest complete pair without waiting
for audio. One writer publishes an odd version, atomic payload fields and then
an even version, all in one sequentially consistent order. Audio makes one
bounded read attempt: equal even validation stamps prove a complete copy; a
collision leaves the previous proof intact. The checked version never wraps.
Transfer serials are globally issued and never reused, so copying their atomic
representation preserves arm identity without copying identity-table state.

## Reset coordination

Reset remains stopped-owner only. Before offering a successor, control seals it,
collects every old command and waits until the renderer has passed the last old
command's quantum. An old command receipt alone does not prove ingress drained.
The wrapper independently checks Stop, ingress, activation and the seal again.
An offer refused as late or busy retains the exact owner; the host may abandon
and prepare a later boundary off-thread. Callbacks never wait for that work.

A sealed-transition certificate binds the old arm, exact successor and boundary.
It is issued only after old command custody is collected, and retains the source,
head and window caps. It proves that nothing is due in the unserviceable interval;
it never credits successor commands which have not been forwarded. The wrapper
requires that exact transition when a callback crosses a pending reset. An
ordinary old-arm certificate is invalid on the successor. A staged event inside
the gap limits strict coverage and causes the declared silent stop.

Retirements produced during a render are offered to their return queue immediately
after it, so control has the inter-callback interval to commit and refill. A full
return queue retains the complete owner in one audio-side slot while rendering
continues. Control commits the exact reset receipt and joins the old owner off
audio; the raw owner and source FIFOs retain their independent lifetimes.

## Falsifiers and executable checks

Acceptance fails on changed serial audio/receipts, a late healthy source command,
unprepared source work, an accepted foreign/stale arm or raw generation, callback
heap activity, revival after a strict fault, or incomplete joined custody.

The `mixed_source_bridge` integration target exercises the library raw-first
route, late/terminal classification, protected releases, source pressure, loops,
transport, resets and joined ownership. Its
[continuous saved-project control][continuous] adds independent software callback
cadence, two source workers, rolling control, reset with held notes, compiled
seek and retirement backpressure. The serial oracle does not use publication
certificates or the thread protocol. This is functional software evidence,
not measured device cadence, worst-case latency or capacity selection.

The required mixed integration/library/documentation/Clippy gates and complete
repository gate apply. A false claim, contradiction or missing ownership rule
blocks acceptance; optional implementation detail does not.

[continuous]: ../../../crates/synth_engine_v2/tests/support/continuous_saved_host.rs

## Remaining enablement gates

Production live adapters still require ADR-0054's representative complete producer
reselection, P03-R004 endurance/width validation, physical timing and adapter
evidence under ADR-0022, and a reviewed shipping dependency/feature decision.
Application mono-device admission and other project populations have separate
gates. This record accepts no Phase 9 exit and lifts no production refusal.
