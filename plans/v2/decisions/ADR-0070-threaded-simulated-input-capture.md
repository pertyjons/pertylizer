# ADR-0070: Threaded simulated input capture

| Field | Value |
|---|---|
| ID | ADR-0070 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-20 |
| Last reviewed | 2026-09-20 |
| Related | ADR-0036, ADR-0068, ADR-0069, IO-INV-004, IO-INV-005, TAKE-INV-003 |
| Supersedes | — |
| Superseded by | — |

## Ownership decision

A fresh `InputCaptureSession` splits into a merger-side `InputCaptureControl`
and an exclusively audio-owned `InputCaptureAudio`. This joins ADR-0069's
independent synthetic clocks and fixed input cells to ADR-0068's finite transfer
credits. Neither half exposes mutable inner owners or a readable raw result.
Concrete queues and producer threads remain host responsibilities.

A synthetic producer sends generation-tagged original observations through a
fixed queue. Queue acceptance precedes model admission: a failed send retains
its original value, and the host must resolve every queued observation, including
explicit refusals after closure. The merger admits observations to fixed input
cells and issues their input identities. A cell retains the original tick,
arrival, clock and actual outcome through packet preparation, callback admission,
receipt transport and explicit input-receipt collection. Core transfer collection
returns a transfer credit, but does not return that input cell's credit.

The serial and split consumers use the same arrival-order selector. Only explicit
source frontiers advance the common prefix; fences precede messages at equal
arrival. Queue occupancy, producer scheduling and thread joins never manufacture
a source frontier. The two promises in ADR-0069 remain unchanged. Zero capture
lateness also remains unchanged: withholding by another source or delaying the
worker may cause an identified refusal. This slice does not qualify a worker
deadline or guarantee complete capture under arbitrary stalls.

A full core source-credit population leaves the input cell untouched. A failed
packet send returns the packet to the host; the host retains it for FIFO retry or
explicit cancellation. Cancelling an accepted source packet also requests a terminal
halt; it cannot silently omit accepted input from a Complete take. A transfer
mapping is located before consuming its core completion. Unknown or foreign transfers return ownership. Closure cancels only
unforwarded input cells; packets and completions already in transit retain their
mappings until resolved. Command outcomes return directly to the host, and source
outcomes return to their original input cells.

## Independent terminal halt

A separately prepared, shared atomic signal provides a finite, first-request-wins
terminal halt. It has no queue capacity dependency and is never reset or reused.
The host may request Stop or DeviceLost independently of the merger; input
admission or delivery failures request SourceInvalid. Audio also publishes the
signal when its renderer closes the lanes, so a render fault closes merger admission.
The first cause remains the take's reason even if a later input diagnostic records
a different cause. Handles are cloned and retired off-callback. The callback only borrows the shared allocation.

When the merger observes any terminal signal, it marks previously undiagnosed
inputs `PeerInterrupted` and preserves existing diagnostics. In this composition
that marker denotes imposed closure, including user Stop or backend halt; it is
not evidence that another input failed. The recorder retains the terminal reason,
and joined quality attribution skips this marker to find the original input fault.

Audio observes the signal before admitting packets and before rendering. Packet
admission refuses new custody immediately. Rendering or joined recovery
closes lanes and interrupts capture at the last whole successfully acknowledged
callback, retaining all command/source outcomes. A request racing after a poll
is handled by the next poll or by recovery after callback access has joined.
A successful callback is never retroactively silenced. Returned packet ownership
survives rejection. Joined recovery polls the same signal and needs no final
callback.

This terminal halt produces an interrupted take, including when its reason is
Stop. It does not fabricate a timestamped Stop command or receipt. Normal ordered
Play/Stop packets retain ADR-0067 behavior, including Stop progress when sources
or the worker stall after command delivery.

## Reunion, quality and retirement

The host joins callback access before moving audio ownership to its recovery
worker. It retains all queue backing and failed-send cells until that join.
Reunion requires the matching transfer namespace and resolution of every core
packet/completion credit. It preserves uncollected input receipts. A refusal
returns all owners; it never destroys a take to report an error.

Merger-side diagnostics retain refused exact or uncertain observations while the
raw recorder belongs to audio. Reunion applies their quality attribution before
exposing the serial composition or any result. Finalization is unavailable while
split, so a fault before reunion cannot escape
as a Complete take. It interrupts even after ordered Stop, and retains the late or
uncertain quality marker when the refused observation intersects the selection.
For a still-active take, audio may halt before merger diagnostics reach the raw
recorder. Uncertainty attribution therefore uses the final frozen selection at
reunion. An ambiguous observation wholly outside that retained interval remains
in the input diagnostic but need not mark its quality; the take is Interrupted.
The serial oracle can mark the larger pre-interruption window. Equivalence tests
compare fully delivered non-faulting input, not that timing-dependent diagnostic.
After reunion and normal finalization, the existing serial sealed-Complete quality
rule still applies until source retirement.

No producer can be acknowledged quiescent while split. After joining each producer
and resolving its queued observations, the host explicitly acknowledges that
source through the reunited composition. Audio join alone is insufficient.
Finalization and old-take retention need no further callback.

Only resolved input receipts and source quiescence permit retirement. A new
explicit attempt gets a fresh connection generation and prepares to Ready; old
observations and old stop handles cannot mutate or resume that replacement.

Existing prepared budgets cover input and serial recording storage. A split
budget additionally covers finite transfer credits/mappings, wrapper metadata
and a conservative shared-signal allocation charge. Concrete host queues have
separate byte ceilings, including return queues and failed-transfer storage.
Callback operations allocate and deallocate nothing.

## Falsifiers and remaining scope

Acceptance fails if any accepted observation loses its original value or terminal
outcome; a credit can be reused before its collection; ordering depends on queue
occupancy; stale identity changes a replacement; quality becomes readable before
reconciliation; halt waits on a full queue or worker; a callback allocates or
frees; a failed owning operation drops retained state; or audio join implies
producer quiescence. Optional implementation detail does not block acceptance.

The executable checks in
[`tests/ordered/input/threads.rs`](../../../crates/synth_engine_v2/src/recording/notes/loop_capture/tests/ordered/input/threads.rs)
are:

- `producers_merger_and_audio_match_serial_with_full_queues_and_stalled_receipt_worker`:
  two real input producers, one merger and one audio thread; one-cell transport
  queues, failed-send retention, worker stalls, independent clocks, identical
  PCM/raw records/pass/carry/input outcomes across callback partitions.
- `independent_halt_stops_audio_with_worker_stalled_and_full_completion_queue`
  and `source_stall_does_not_hold_ordered_stop_and_delayed_note_retains_its_refusal`:
  terminal halt and ordered Stop progress independently of worker/source progress.
- `no_final_callback_loss_retains_outcomes_and_old_take_across_reconnect`:
  joined loss, separate source acknowledgements, original outcomes, old-take
  retention and isolated replacement identities/signals.
- `audio_fault_publishes_closure_and_returns_queued_unadmitted_observations`:
  audio-originated faults close merger admission and preserve refused values.
- `late_quality_from_second_input_survives_peer_closure_before_reunion`
  and `uncertainty_uses_frozen_selection_and_cannot_report_complete_after_fault`:
  quality attribution before result exposure, including a non-first source and
  both retained and excluded uncertain intervals after interruption.
- `halt_refuses_packet_custody_without_losing_credit_or_allocating`:
  immediate refusal before rendering retains the packet and its credit, and the
  halt/refusal path allocates and frees nothing.
- `input_cell_credit_is_held_after_core_collection_until_input_receipt_collection`,
  `split_budget_and_unresolved_reunion_preserve_original_input_and_transfer_owners`
  and `foreign_completion_and_reunion_return_owners_without_spending_local_credit`:
  layered credits, exact split budgets, owning refusals and namespace isolation.

Run them with `cargo test -p synth_engine_v2 recording::notes::loop_capture::tests::ordered::input`.
The counting allocator and static hot-call closure check include the new callback wrapper.

Physical timing remains under ADR-0022 and its existing evidence. This finite
synthetic consumer does not add live audition, restart, physical adapters, audio
input, monitoring, complete worker timing qualification or phase exit.
