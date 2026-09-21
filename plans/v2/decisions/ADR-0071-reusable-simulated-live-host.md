# ADR-0071: Reusable simulated live host

| Field | Value |
|---|---|
| ID | ADR-0071 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-21 |
| Last reviewed | 2026-09-21 |
| Related | ADR-0067, ADR-0068, ADR-0069, ADR-0070, IO-INV-004, TAKE-INV-003 |
| Supersedes | ADR-0067's initial-time-only recording Play restriction |
| Superseded by | — |

## Boundary

The next host must accept transport commands after callbacks have started and
retain earlier takes across explicit fresh attempts. This extends the finite
ordered loop consumer; it does not make its immutable recording context reusable.

## Decision

`LoopCaptureSession::arm_at` reserves an exact future quantum-aligned start while
preparing off-thread. It fixes the raw window and loop mapping before publication.
The journal still starts at engine zero with its prepared carry. Before Play,
device time advances through silence and musical position stays fixed. One Play
may be published at the reserved start; it cannot move the window or retry later.
The original `arm` keeps its initial-time behavior. Direct standalone start refuses
a reserved time that differs from the journal's current clock.

Input composition checks that each declared synthetic clock can represent the
reserved start exactly, including its entire uncertainty interval. `exact_tick`
returns such a tick or refuses preparation. Producers still owe an explicit
frontier at that tick, before advancing beyond it. The host publishes the initial
frame-zero frontiers before releasing callbacks. Neither preparation nor queue
occupancy substitutes for these source promises.

A missing start fence yields the existing capture-refusal outcome and leaves an
unstarted take for explicit Stop or interruption. A packet arriving after its
reserved boundary retains a Boundary refusal; the host must close that attempt
through Stop or terminal recovery. A take which never starts selects an empty
window at its explicit end. Stop before the reserved start cancels pending Play.
Late pre-roll input follows the existing source-fault and quality rules. None of
these failures permits an automatic delayed start, automatic retry or result loss.

The non-shipping host owns bounded packet and completion SPSC rings around the
existing split core. Failed sends retain the original packet before any later
publication. One callback captures and admits a finite FIFO prefix, renders its
whole block, and returns bounded completions. Semantic core refusals remain
identified outcomes; protocol delivery failure requests SourceInvalid, not user
Stop. A cancelled unpublished packet has an explicit host cancellation outcome.
Input receipts remain in their core cells until separate collection.

Each source has a bounded producer ring. A failed send returns its original value
to the producer for ordered retry or explicit shutdown refusal. Merger service
resolves every popped observation as an admitted input ID or an original-value
refusal, including after closure. The host joins producers and resolves both their
retry values and every queued observation before acknowledging source quiescence.
Audio join alone supplies no such acknowledgement.

Normal closure first stops and joins callback access, without setting a terminal
signal. Joined service can drain admitted source fences and collect outcomes.
Reunion then permits finalization, normal closure and individual source
acknowledgements. Missing fences keep that attempt unresolved; explicit recovery
can interrupt it. The failure path sets the existing terminal signal, joins
callbacks, cancels unpublished transfers, collects all outcomes and reconciles
quality before source retirement and finalization. It needs no final callback.
Failed owning moves retain the complete owners, including host queue cells.

Restart is explicit preparation and activation of fresh owners with new epochs,
generations and terminal handles. It requires a reserved retained-result slot.
Old finalized, closed and quiescent owners remain until explicit collection;
collection transfers ownership rather than discarding raw capture. A full archive
refuses a new activation. Its aggregate ceiling includes each slot's declared
full-attempt preparation ceiling, inline storage, and one preparation candidate.
The host must admit every attempt within that declared ceiling. This allows
stopped reconstruction, not gapless device restart or arbitrary memory growth.

## Verification and limits

Falsifiers are partition-dependent audio/raw/pass results, missing terminal
outcomes, an inaccessible start fence accepted during preparation, a callback
allocation or destruction, source acknowledgement before queue resolution,
old handles mutating a replacement, and count/byte ceilings exceeded by retained
attempts. False claims, contradictions, unfillable contracts and safety defects
block acceptance. Optional implementation detail does not.

The scheduled-start tests live in
[`scheduled.rs`](../../../crates/synth_engine_v2/src/recording/notes/loop_capture/tests/ordered/scheduled.rs).
The concrete host and queue tests live in
[`tests.rs`](../../../crates/pertylizer/examples/support/v2_input_host/tests.rs).
The core retains its system-library-free dependency closure. Physical timing,
production budgets, audible live input, musical projection and the full transport
vocabulary retain their subsequent implementation gates.

The initial independent design consultation found missing decision amendments,
start-fence reachability, undefined failed-start ownership, missing normal join,
producer-queue custody, halt reason and aggregate retention budget. This record
states the repaired boundaries. Independent uncommitted review and focused repair
review passed; the final custody follow-up used the reader's explicit self-audit
stopping rule. The complete repository gate and concrete host tests passed.
