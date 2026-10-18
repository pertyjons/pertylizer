# ADR-0082: Running compiled-plan reset

| Field | Value |
|---|---|
| ID | ADR-0082 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-17 |
| Last reviewed | 2026-10-17 |
| Related | ADR-0009, ADR-0047, ADR-0048, ADR-0050, ADR-0054, ADR-0064, ADR-0077 |
| Supersedes | ADR-0077's playing-edit deferral for isolated compiled sessions only |
| Superseded by | ADR-0083 for bounded compatible application selection |

## Boundary and evidence

The maintainer selected combined-host integration and live editing on 2026-10-17,
with final capacity reselection deferred to production enablement. This successor
extends ADR-0064's owning envelope to an isolated compiled-only running reset and
applies ADR-0009's Q-frame fade without its external release obligations. It does
not accept mixed production ingress, compatible voice migration or Phase 9 exit.
The coherent current rule is the
[running-reset specification](../specs/spec-host-io-lifecycle.md#experimental-running-compiled-reset).

## Decision

Preparation requires acknowledged Playing, no ordinary credits, no other candidate
and no external producer declaration. A fresh pair, generation, epoch and table are
prepared off-thread. After the pair and scratch exist, choose a future aligned B
and derive P from the unchanged old anchor. The private successor adopts and collects
its own initial Play without rendering: clock B, carry zero, empty exchange/credits,
and restoration plus its charge owed for the first actual quantum. If the new song
has ended at P, prepare Stopped(P) without clamping P or creating activation debt.
No old minter snapshot is promoted.

At exact B with empty carry, audio checks old origin/table/watermark/anchor and the
fresh successor certificate. It exchanges whole owners and renders both for Q output
frames, mixing old*(1-t)+new*t with t=(frame+1)/Q. The final frame preserves successor
bits. Crossing notes are cut; later onsets use the fresh table. No sounding state
migrates. Ordinary rendering cannot bypass the fade owner.

At B+Q the isolated old compiled owner is terminally fenced. Its actual registry
count, including old onsets during the fade, is reported, and its authoritative
minter and registry retire together off-thread. This resolves only the faded
compiled-owner ending case of ADR-0048. It needs neither a fake Stop nor a discarded
release quantum: no external key or release entitlement survives. Live and mixed
reset retain their forced-release laws.

Only the completed fade returns Installed. Collection clears the old controller's
replacement credit before promoting the fresh controller with its own counters,
anchor and ledger; only terminal close-admission intent follows the old controller.
A post-exchange failure silences the whole callback, retains both owners and returns
Interrupted for closed-origin teardown. Old audio never resumes after exchange.
Late or changed-state refusal before exchange leaves old audio untouched and returns
the owning candidate once; it is never retimed in place.

The application uses one owning lane and one attempt per edit. Compile/preparation
or pre-install refusal keeps the old sound and metadata until pause, Stop or a newer
edit. A newer pending edit survives an older decline. Pause waits for collection;
a later Play cancels an unsent Pause. Explicit Stop/disable mute and retire the whole
wrapper, including both fading owners, without another old callback. Plan identity
and omissions follow the installation that successfully rendered. Inside the old
four-margin end-stop lookahead, preparation defers to the ordinary end stop.

The existing five-credit shell ledger remains. The running constructor permits only
one candidate; its boxed payload and actual Q scratch capacity are separately charged
per credit. The application provisionally caps cold timeline preparation at 16,384
events. This changes no producer share. Each renderer is separately admitted; aggregate
transition CPU, complete transient cold-memory qualification and representative
endurance remain production-enablement work under ADR-0054 and P03-R004.

## Falsifiers and stopping rule

A wrong occurrence controlling the new plan, minter-generation rewind, false clean
acknowledgement, partial mutation on pre-install refusal, lost owning credit,
callback allocation/destruction, partition-dependent fade or revival after fault
blocks acceptance. Optional implementation detail does not. The 17 core controls
cover bootstrap, nonzero-position automation versus normal adoption, ordinary-player
oscillator references at four rates, partitions, maximum-width lateness, fresh
issuer continuation, interrupted custody and post-fade overflow. The 19 application
edit controls include latest-wins, Pause/Play, old metadata, compile refusal and
Stop/disable during a partial fade. The complete repository gate, additional
application/harness controls and independent Core/application reviews passed for
implementation commit `aaa3242c`.

## Consequences

The development application can apply a running structural edit through audible
reset semantics while retaining its last valid plan on refusal. It remains default
off under `v2-lowering`, with MIDI disconnected and no seek/loop addition. Compatible
sounding-voice migration remains a separate first consumer under ADR-0010/0048.
