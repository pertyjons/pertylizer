# ADR-0078: Host-owned mixed transfer rehearsal

| Field | Value |
|---|---|
| ID | ADR-0078 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-10 |
| Last reviewed | 2026-10-17 |
| Related | ADR-0023, ADR-0032, ADR-0043, ADR-0046, ADR-0047, ADR-0050, ADR-0053, ADR-0054, ADR-0068, ADR-0072, ADR-0075, ADR-0076 |
| Amends | ADR-0046 clause 6, ADR-0047 clause 7, ADR-0050 clause 8, ADR-0072's exclusive-live-owner exception and ADR-0076 decision 3, solely for the default-off rehearsal below |
| Supersedes | — |
| Superseded by | [ADR-0081](ADR-0081-combined-host-software-boundary.md) extends the bound default-off software host; production qualification remains gated |

## Decision boundary

Before this decision, the private mixed owner could offer note-on, bend and
release beside compiled playback only under
`cfg(all(test, feature = "simulated-ingress"))`; a non-test host could not reach
its command and result rings. ADR-0076 keeps a production mixed stream
refused until Phase 9 exit, and
ADR-0072's audio-owned identity and release-timing exception covers only an
exclusive live owner. This record permits a **separate, non-shipping Rust API
rehearsal**, not acceptance of the combined host or Phase 9 exit.

The new default-off `mixed-transfer-rehearsal` feature implies
`simulated-ingress`. It adds a feature declaration to the crate manifest but
no dependency edge, persisted or wire change, application route, or change
to `PerformanceIngress::prepare`'s `MixedProducerPlan` refusal. The public
boundary and its narrow exception to accepted ownership rules require the
maintainer's explicit approval. The maintainer approved this public rehearsal
boundary and feature on 2026-10-14 when requesting Phase 9 mixed-producer
work items 1–6, which included this step.

## Verified premises

- `ringbuf` is a dev-dependency of `synth_engine_v2`; its normal dependency
  allowlist excludes it. Existing session and loop transfer paths make the
  host own their concrete queues.
- The private mixed owner already has a registered ingress store, bend-credit
  accounting, a handle ledger and reserved release places. It also has pedal,
  lift and Panic machinery, but this rehearsal does not expose those offers.
- A memberless mixed bend advances the ingress store's `last_accepted` fence
  without writing a queue entry. A release of a refused onset tombstone
  retires its handle without an ingress offer, so the store cannot be the
  sole source-order or horizon guard.
- ADR-0032 clause 21 and HOST-INV-013 require a far-future ingress event to
  be rejected and counted once, without parking it until its timestamp.

## Decision

1. **Scope and feature.** A new constructor under
   `mixed-transfer-rehearsal` arms only the existing bound, closed, initial
   one-shot plan with exactly one compiled and one live note producer and one
   registered simulated live source. Only note-on, channel bend and exact
   key-up packets are exposed. Pedal, lift, Panic, a second source, plan
   replacement, loop playback, hardware input and an application route remain
   refused. No shipping-target dependency edge, default feature or forwarded
   feature under any shipping package feature set, including `v2-lowering`, may
   enable the rehearsal. Implementation must extend the crate-boundary test to
   check the dependency and feature graph for each shipping feature set. CI's
   explicit all-features compilation is a test configuration, not a shipping
   artifact.
   Arm requires both the control reservation table and audio bend table to
   cover all sixteen channel scopes of its sole registered source; a legal
   first bend cannot exhaust either table. Sixteen first non-zero bends on
   distinct channels must each be accepted by control and applied by audio,
   with all sixteen non-zero scope cells still held in both tables at the end.
2. **Host-owned queue custody.** The crate mints, moves and consumes opaque,
   non-cloneable command and result packets through host-owned bounded SPSC
   queues. The host holds non-cloneable bound wrappers and cannot directly
   push, pop or clone packets through them. The host must retain every wrapper
   until joined teardown and destroy it only off the audio thread; dropping a
   wrapper earlier violates the host contract and may lose queued packets.
   `ringbuf` remains dev-only here. Arm checks
   capacities observed from both queues against its admitted command/result
   bound `C`, including the places withheld for exact key-ups, and charges a
   preallocated outstanding-command journal. A pre-consumption command/result
   journal-capacity or bend-credit refusal returns the source observation for
   ordered retry without minting a packet. A failed physical command push
   after that preflight is a terminal host fault:
   control retains the minted packet in one prepared pending slot for joined
   teardown. The command ID, journal entry and all associated capacity remain
   charged. For a failed
   key-up push, its previously reserved place becomes the charge of that
   unsent key-up; the release-reservation counter decreases, but no capacity
   becomes free. For a failed onset push, its exact key-up place and any
   inherited bend credit stay reserved. No younger command passes it. There is
   no arbitrary cancellation while the stream continues.
   A pre-consumption `Full` returns the exact observation but does not promise
   that retry will eventually succeed. Open handles can keep their release
   places after all commands and results have drained. The later combined host
   must either retry while a named outstanding obligation can free room, or
   record a per-occurrence split refusal and consume that onset's raw-linked
   release without offering it to mixed ingress. A lost source is closed
   through joined teardown, which classifies its unresolved handles; Panic
   alone cannot recover a missing key-up because it retains a tombstone for
   that key-up. This rehearsal admits no source-loss recovery rule.
   With all bound wrappers returned and queue obligations honored, joined
   terminal teardown names every unsent, unserved and uncollected packet and
   outstanding handle. If a wrapper is missing, the retained journal still
   names its outstanding command IDs and handles, but the lost queue contents
   cannot be recovered or claimed as
   accounted for. After callback quiescence, a missing-wrapper shutdown reports
   `MissingEndpoint`, stops both halves, lists the retained journal IDs and
   handles, drains available bound consumers, and marks each queue whose
   consumer is missing as having unknown contents. It cannot report complete
   joined custody. The qualified host allocates and destroys queues off the
   audio thread.
   The rehearsal uses crate-defined bounded SPSC queue traits. Arm consumes
   one unsplit host command queue and one unsplit host result queue, checks
   their reported capacities and that both are empty, and returns both queues
   unchanged on any arm refusal.
   A successful arm returns privately bound command-producer/result-consumer
   and command-consumer/result-producer wrappers to the host. The host owns the
   concrete queues inside those wrappers; neither mixed owner retains a queue
   half. The host lends its control wrappers mutably to each submit and collect
   call and its audio wrappers mutably to each callback; ownership stays with
   the host. Each call checks the wrapper's private owner and queue binding
   before touching it; a foreign wrapper raises a terminal transfer fault and
   remains with its host owner. A queue trait implementation must provide
   exclusive, unaliased halves and FIFO delivery,
   report its true fixed capacity and occupancy, retain every accepted packet
   exactly once, and return the exact packet on a failed `try_push`. Its callback
   `try_push`/`try_pop` must be bounded, nonblocking, allocation-free, lock-free
   and panic-free. Joined teardown first checks every supplied wrapper binding.
   On any mismatch it refuses the entire join, returning both mixed halves
   and every supplied wrapper unchanged, with neither queue drained; the host
   can retry with the correct wrappers. Only when no wrapper is foreign does
   a missing wrapper select the partial `MissingEndpoint` shutdown above.
   With correct wrappers, teardown accounts for their remaining contents
   beside each half's internal custody.
3. **Order.** Each packet binds the owner, plan, epoch, identity table, source
   and one gap-free increasing command ID. A key-up handle carries the same
   owner binding. The control owner checks that each
   consumed source observation's **nominal** `SampleTime` is not earlier than
   the preceding consumed observation. Wrapper and key-up handle owner checks
   run after the initial halt read and before this backward-time check. The
   time check runs before the remaining checks for every exposed request.
   A key-up then checks whether this owner's handle is open. Every request
   that can mint then checks command-ID exhaustion before capacity, credit or
   onset-cell vacancy. An in-order key-up for this owner's retired handle
   returns `NotOpen` even if the next ID is exhausted; a key-up for an open
   handle faults on ID exhaustion. An onset with an available command ID checks
   capacity and credit before treating exhausted handle cells as a terminal
   identity fault. There is no separate control-side hold-room refusal: each
   held ledger cell is charged to an outstanding command or reserved release
   place, except at most the live hold entitlement `H` sustained cells. The
   ledger has command capacity `C` plus `H` cells. Thus a table with no vacant
   cell while two command places remain must contain exhausted generations;
   ordinary full polyphony reaches command-capacity refusal first. Audio may
   still refuse an offered onset at its hold limit as an ordinary result.
   Equal stamps retain command-ID order. A transient
   pre-consumption journal-capacity or bend-credit refusal returns the
   original without advancing that frontier. Identity exhaustion includes
   the increasing command ID and the live-occurrence ledger **when a new onset
   finds no vacant, unexhausted cell despite available command room and any
   required inherited bend credit**. Retiring one cell at its generation
   limit does not fault the stream while another cell is free. At the
   no-vacancy onset, identity exhaustion
   returns the original without advancing the frontier and raises a terminal
   transfer fault. It must not be reported as a transient journal-capacity
   refusal or become a permanent onset retry on a running stream. A failed
   physical queue push is a terminal host fault, not a transient journal-capacity
   refusal.
   A physical command push failure happens after ID minting and is terminal;
   every typed submit commits and retains the packet's journal, outstanding
   charge and associated handle and bend reservations before returning that
   terminal fault. A first non-zero bend on a new scope retains its reserved
   scope-table cell and bend credits. A failed onset on an already bent scope
   retains its inherited bend credit. A failed key-up push changes its handle
   from open to releasing under the pending key-up's ID; it does not retire the
   handle. Joined teardown names that releasing handle, its pending packet and
   the packet's charge in place of the former release reservation.
   A pre-consumption halt, capacity, bend-credit or identity refusal mints no
   packet, commits none of these charges and leaves no provisional reservation or
   journal entry. A typed submit's first action is its Acquire halt read. It
   then checks the lent wrapper and any key-up handle's owner binding before
   checking nominal time, capacity or the open handle. A foreign wrapper or
   handle raises a terminal transfer fault, returns the original observation
   without minting, and changes no frontier or reservation. A read that
   sees the monotone halt returns the original source observation without
   minting, even if that key-up would otherwise be `NotOpen`. A submit whose read
   precedes a concurrent halt publication may finish; once the control half
   observes the halt, its later submits also refuse without minting.
   On a running stream, the backward-time check precedes a key-up's open-handle
   check. A backward-time key-up for a retired handle is terminal. An in-order
   `NotOpen` key-up for this owner's retired handle is a non-terminal refusal
   that returns its original,
   mints nothing and leaves the source frontier unchanged.
   Identity exhaustion also halts, including an onset that has available
   command room and any required inherited bend credit but no eligible vacant
   handle cell. When a physical command push fails, its pending packet is the
   only unsent minted command
   and is moved into joined teardown exactly once. Other terminal faults need
   not have a pending command.
   An observation consumed into a packet advances it even if audio later
   refuses the note or bend. The audio owner independently checks ID, origin
   and nondecreasing nominal time for every served packet **before** ledger or
   ingress mutation, including a key-up of a refused onset tombstone. A
   backward-time source observation is a terminal control-side fault that
   returns its original and keeps every open handle and reserved release
   place for teardown. A foreign, duplicate, reordered or backward-time
   audio packet likewise faults the whole transfer and retains its original
   in audio's prepared invalid-command slot for teardown. A beyond-horizon
   packet uses that same slot. The current private path's
   per-event `NonMonotoneStamp` outcome is not this public transfer rule.
4. **One horizon decision.** Audio checks every ordered packet against the
   profile's forward horizon at ingress admission, before any occurrence or
   ingress mutation. This includes a key-up that would only consume a
   tombstone. The store's offer may repeat the same comparison after audio
   preflight while its clock and the packet stamp are unchanged; a successful
   preflight cannot then refuse or count again. A beyond-horizon packet is
   rejected and counted **once**, then raises a terminal transfer fault with its original ID and
   handle retained for joined teardown. No packet waits in a queue for a
   distant time. The control source frontier may already name that packet;
   the terminal halt prevents later observations from being misattributed.
5. **Sound and release.** An accepted packet keeps its nominal event stamp
   unchanged. The renderer alone applies ADR-0043's preserving late clamp
   and late counter; same-sample live events apply in command order under
   ADR-0023. A valid, in-order, in-horizon key-up uses the place reserved by
   its accepted onset and cannot be refused later for capacity or the store's
   order fence. A refused onset leaves a tombstone that its exact key-up
   consumes without ending a newer note. A bend is never coalesced or moved
   early. A bent-scope onset inherits the last accepted bend at its effective
   render sample and spends one pre-reserved bend credit; absent credit
   refuses the onset before any command or handle is minted. A refused bend
   returns all of its reserved credits. Immediate command results retain the
   original ID and nominal stamp. This rehearsal proves audible effective
   samples by callback comparison; a per-event audible-time receipt belongs
   to the later joined host outcome.
6. **Result and terminal safety.** Before settling a popped result, control checks
   its owner, plan, epoch, table, command ID and request kind against the
   head of the preallocated outstanding journal. A foreign, duplicate,
   reordered or mismatched result changes no ledger, count or reservation;
   control retains that popped packet in a separate invalid-result slot for
   joined teardown and faults the transfer without
   panicking. Every terminal host fault is a transfer fault. Every transfer
   fault, whether detected by control or audio,
   stops command service and causes the audio owner to silence the complete
   callback in which it observes the fault and every later callback until
   off-thread reprepare. Both halves Release-store to and Acquire-read one
   prepared atomic terminal halt signal. Control checks it before every typed
   submit, including a key-up's open-handle check, and before every ordinary
   result collection; audio checks it before and after command service and
   during rendering. Each detecting half retains its exact cause; the other
   half may report a generic remote
   transfer halt. Each ordinary collection pops at most one result. Its Acquire
   read before that pop is the halt decision point: if it observes a halt,
   collection returns it without
   popping or settling anything. A halt published after the read can leave
   that already-started collection to finish; once the control half observes
   the halt, its later collections also stop.
   Joined teardown reads the stopped renderer registry for sounding identities,
   including an accepted future key-up whose note-off has not published. A
   registry snapshot failure is reported explicitly. It also reports
   outstanding release places; no note keeps sounding through a halted
   stream. Packet service, including calls into the host's qualified queue
   implementation, allocates nothing, takes no lock, makes no system call and
   does not panic. The outstanding-command bound means a truthful, sufficiently
   sized result queue cannot fill from this transfer's own results. A failed
   physical result push is therefore a terminal host fault: audio retains the
   result in its prepared pending slot, stops command service and silences the
   observing callback. After control observes a transfer halt, ordinary result
   collection no longer pops a result, settles the ledger or returns credits;
   joined teardown takes both pending packets when both exist, the separate
   invalid-result and invalid-command slots, queued packets and outstanding
   journal.
   A missing command/result is a terminal host fault when
   detected by a later ID gap or joined teardown; a missing final packet has
   no earlier liveness signal. The outstanding journal prevents credit reuse
   and names its command ID at teardown.

## Falsifiers and required proof

- The feature is enabled by default or a repository shipping target; it
  fails to imply `simulated-ingress`; or the constructor admits a missing or
  second producer, a second source, pedal, Panic, a non-initial candidate,
  plan replacement, loop playback, either bend table smaller than sixteen
  channel scopes or a crossed target partition. A shipping feature such as
  `v2-lowering` forwards to the rehearsal even when its default feature set
  does not.
- An undersized or non-empty command or result queue arms; a falsely reported
  capacity causes a failed push without a terminal fault; an unbound endpoint
  reaches a queue operation; or a failed physical push loses its prepared
  pending packet, handle, command ID or charged place. A failed key-up push
  double-charges or frees its previously reserved place, or a failed onset
  push loses its key-up reservation or inherited bend credit. A failed first
  non-zero bend push loses its scope-table cell or credits. An invalid command or result disappears
  from its dedicated slot before joined teardown.
- A source observation or forged audio packet at a stamp below its respective
  frontier mutates a note, bend or ledger, or lets a younger packet run. Equal
  stamps fail to retain command-ID order.
  A transient refusal advances the control frontier, or an audio-refused
  packet fails to advance its frontier.
- A foreign wrapper or key-up handle with a backward stamp is classified as a
  time fault instead of an owner fault, loses its original, or changes the
  frontier or a reservation.
- A backward-time onset or bend returns a capacity, credit or
  identity refusal instead of its terminal time-order fault. An in-order
  `NotOpen` key-up halts the stream, mints a command or advances the frontier,
  or a backward-time
  key-up for that retired handle returns `NotOpen` instead of faulting. An
  in-order `NotOpen` key-up faults solely because the next command ID is
  exhausted, or an open-handle key-up continues after that exhaustion. An
  exhausted command ID combined with full command capacity or insufficient
  bend credit returns a transient refusal instead of a terminal identity
  fault. With an available command ID, a no-vacancy onset short of command
  capacity or inherited bend credit raises an identity fault instead of the
  corresponding transient refusal.
- Command-ID exhaustion or an otherwise admissible onset with no unexhausted
  vacant handle cell lets the stream continue, advances the control frontier,
  loses the original observation, reports transient journal-capacity refusal,
  or fails to silence audio once the transfer fault is observed. Conversely,
  retiring one exhausted cell while another is free must not fault the stream.
- A refused onset's tombstone key-up bypasses the horizon check, a far-future
  packet is counted more or less than once, or its stream keeps serving later
  packets. The original and outstanding handle must appear at teardown.
- An on-time live bend writes at another sample or changes a compiled note;
  a late bend or key-up renders before the first unrendered boundary; or a
  bend's value is coalesced into or heard before another command.
- A valid key-up refuses for capacity or the ingress order fence, frees a
  different occurrence, loses its immutable nominal stamp, or fails to
  retire its accepted handle. An onset on a bent scope misses its inherited
  value or spends no credit; a refused bend leaks one.
- A forced full result queue loses its pending result, serves a younger
  command, or fails to raise a terminal transfer fault; a foreign or reordered
  result changes the ledger or panics instead of faulting; or the concrete
  host endpoint used in the fixture allocates, locks, blocks, panics or performs
  final resource destruction in a callback.
- Any transfer fault fails to halt a control submit or collection whose first
  Acquire read observes the halt; or control settles a result,
  returns its credits, accepts a source observation or mints a command after
  such an observation and before teardown. A halt-refused submit loses its
  original source observation.
- Any transfer fault, including a source-order fault, foreign or reordered
  command or result, horizon failure or missing packet, fails to silence the
  complete callback in which audio observes it and every later callback.

The first integration fixture compiles the library **without** `cfg(test)`.
It carries a simulated note-on, bend and exact key-up beside a compiled note
over a host-owned SPSC ring, and compares fault-free output across one-call,
64-frame, 256-frame and irregular callback partitions. Separate fault fixtures:

- Refuse undersized and host-reported non-empty command and result queues at
  arm and verify that both concrete queues return unchanged. A crate-internal
  arm fixture seeds an opaque packet before the check and verifies its custody.
- Inject full command and result queues whose capacities were falsely
  reported. Separate onset, exact key-up and bend push failures check the
  control pending packet's ID and journal. An onset on a bent scope keeps its
  key-up reservation and inherited bend credit; the key-up moves its reserved
  place into the unsent command's charge without freeing or double-charging it
  and changes its handle to releasing under that pending command ID; a first
  non-zero bend on a new scope retains its scope-table cell and reserved
  credits. A result push failure keeps the exact result in audio's pending
  slot and makes later control submits and collections
  return the halt without taking a packet. No younger command is served.
  Joined teardown names each packet; the callback that observes the fault and
  every later callback are silent.
- Deliver foreign and reordered commands and results from two prepared
  owners. Pass one owner's bound wrapper into the other's control and audio
  calls. Check that the receiving half faults before queue access and that
  audio silences the observing callback. After a bad result, a later control
  submit and collection that observe the halt refuse without minting or
  popping; a key-up for an already retired handle returns the halt rather
  than `NotOpen`. A join with a wrong wrapper returns both halves and all
  wrappers unchanged without draining either queue;
  retrying with the correct wrappers accounts for every outstanding packet.
- Cross one owner's key-up handle into the other owner's submit, including a
  backward-stamped case, and verify the owner fault returns the original
  before time or open-handle checks change any state.
- On a running stream, offer an in-order key-up for a retired handle and check
  its non-terminal `NotOpen` refusal without ID or frontier change, including
  when the next command ID is exhausted. A key-up for an open handle with an
  exhausted next ID must fault. In a
  separate fixture with an advanced source frontier, offer a backward-time
  key-up for a retired handle and check that its
  terminal time-order fault reaches audio through the atomic halt.
  Count allocations around the complete callback, including concrete queue
  operations, and inspect that queue implementation for locks,
  blocking, system calls, panic and callback-time destruction.
- Omit one wrapper after quiescence and verify that `MissingEndpoint` names
  outstanding IDs and handles, marks the queue whose consumer was omitted as
  having unknown contents, and never claims complete joined custody. A
  loss-injecting queue fixture confirms that a missing final packet is first
  reported at teardown.

Crate-internal unit tests force command-ID exhaustion, a no-vacancy onset
with exhausted empty cells and other held cells despite command room and any
required inherited bend credit, and continued use of another free cell after
one cell exhausts. For each
table separately, an undersized bend-table fixture refuses arm and returns
both host queues unchanged. In a correctly armed transfer, sixteen first
non-zero bends across the registered source's distinct channels must each be
accepted by control and applied by audio; collect each result and verify all
sixteen non-zero cells remain held simultaneously in both tables. Combined
condition tests exhaust the command ID while command capacity is full or bend
credit is short, and leave the onset ledger without a vacant cell while
command capacity or inherited bend credit is short. Each must verify the
declared error precedence and unchanged original/frontier. Tests also inject
a valid-ID, earlier-stamped audio packet and a duplicate command; the opaque
public API cannot forge or
practically exhaust those cases.
A deterministic render-step unit fixture injects an audio-detected horizon
fault **after the first quantum** of a multi-quantum callback, and proves
that the complete observing block and later blocks are silent. The
required feature-specific gate commands are:

```bash
cargo test -p synth_engine_v2 --no-default-features --features mixed-transfer-rehearsal --test mixed_transfer_rehearsal
cargo test -p synth_engine_v2 --no-default-features --features mixed-transfer-rehearsal --lib
cargo test -p synth_engine_v2 --no-default-features --features mixed-transfer-rehearsal --doc
cargo clippy -p synth_engine_v2 --all-targets --no-default-features --features mixed-transfer-rehearsal
```

Implementation must add all four commands to `.github/workflows/quality.yml`
and `AGENTS.md`'s core gate (and thus its complete gate) before claiming CI
proof. Ordinary workspace tests and Clippy do not exercise a default-off
feature.

## Open work and consequence

[ADR-0081](ADR-0081-combined-host-software-boundary.md) extends the default-off
software boundary to the two-source bridge, raw-first routing, scoped compiled
transport and explicit stopped/running whole-owner reset. It owns strict publication proofs and
independent software callback cadence. The initial single-source transfer arm
above remains narrower than that bridge. ADR-0079 owns raw-only interruption.
The user-owned combined-host DRAFT remains unaccepted as a whole.
ADR-0054's full six-share reselection, representative production workload,
platform timing, shipping admission and Phase 9 exit remain open.

This record extends ADR-0072's audio-owned live identity and
release-at-offer exception beyond an exclusive owner solely for this
default-off, non-shipping initial one-shot rehearsal. It narrows ADR-0046
clause 6, ADR-0047 clause 7, ADR-0050 clause 8 and ADR-0076 decision 3
only to that extent. ADR-0075 remains deferred. The current host I/O
specification records the exposed scope. Acceptance does not mark Phase 9
complete.
