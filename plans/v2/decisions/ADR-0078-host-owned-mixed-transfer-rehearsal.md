# ADR-0078: Host-owned mixed transfer rehearsal

| Field | Value |
|---|---|
| ID | ADR-0078 |
| Status | Proposed |
| Phase | 9 |
| Created | 2026-10-10 |
| Last reviewed | 2026-10-10 |
| Related | ADR-0023, ADR-0032, ADR-0043, ADR-0046, ADR-0047, ADR-0050, ADR-0053, ADR-0054, ADR-0068, ADR-0072, ADR-0075, ADR-0076 |
| Amends | If accepted: ADR-0046 clause 6, ADR-0047 clause 7, ADR-0050 clause 8, ADR-0072's exclusive-live-owner exception and ADR-0076 decision 3, solely for the default-off rehearsal below |
| Supersedes | — |
| Superseded by | — |

## Decision boundary

The private mixed owner can offer note-on, bend and release beside compiled
playback, but its command and result rings and mixed ingress offers compile
only under `cfg(all(test, feature = "simulated-ingress"))`. A non-test host cannot
reach them. ADR-0076 keeps a mixed stream refused until Phase 9 exit, and
ADR-0072's audio-owned identity and release-timing exception covers only an
exclusive live owner. This proposal asks for a **separate, non-shipping Rust
API rehearsal**, not acceptance of the combined host or Phase 9 exit.

The new default-off `mixed-transfer-rehearsal` feature would imply
`simulated-ingress`. It adds a feature declaration to the crate manifest but
no dependency edge, persisted or wire change, application route, or change
to `PerformanceIngress::prepare`'s `MixedProducerPlan` refusal. The public
boundary and its narrow exception to accepted ownership rules require the
maintainer's explicit approval before implementation.

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

## Proposed rule

1. **Scope and feature.** A new constructor under
   `mixed-transfer-rehearsal` arms only the existing bound, closed, initial
   one-shot plan with exactly one compiled and one live note producer and one
   registered simulated live source. Only note-on, channel bend and exact
   key-up packets are exposed. Pedal, lift, Panic, a second source, plan
   replacement, loop playback, hardware input and an application route remain
   refused. Repository shipping targets must not enable the feature, and the
   crate-boundary test must enforce that condition.
2. **Host-owned queue custody.** The crate mints, moves and consumes opaque,
   non-cloneable command and result packets through host-owned bounded SPSC
   queues. The host holds non-cloneable bound wrappers and cannot directly
   push, pop or clone packets through them. The host must retain every wrapper
   until joined teardown and destroy it only off the audio thread; dropping a
   wrapper earlier violates the host contract and may lose queued packets.
   `ringbuf` remains dev-only here. Arm checks
   capacities observed from both queues against its admitted command/result
   bound, including the places withheld for exact key-ups, and charges a
   preallocated outstanding-command journal. A pre-consumption command/result
   journal-capacity or bend-credit refusal returns the source observation for
   ordered retry without minting a packet. A failed physical command push
   after that preflight is a terminal host fault: control retains the minted
   packet in one prepared pending slot for joined teardown. No younger command
   passes it, and its command ID and any release reservation remain reserved.
   There is no arbitrary cancellation while the stream continues.
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
   and one gap-free increasing command ID. The control owner checks that each
   consumed source observation's **nominal** `SampleTime` is not earlier than
   the preceding consumed observation. Equal stamps retain command-ID order.
   A transient pre-consumption journal-capacity or bend-credit refusal returns the
   original without advancing that frontier. Identity exhaustion includes
   the increasing command ID and the live-occurrence ledger **when a new
   onset finds no vacant, unexhausted cell despite available command and hold
   room**. Retiring one cell at its generation limit does not fault the stream
   while another cell is free. At the no-vacancy onset, identity exhaustion
   returns the original without advancing the frontier and raises a terminal
   transfer fault. It must not be reported as a transient journal-capacity
   refusal or become a permanent onset retry on a running stream. A failed
   physical queue push is a terminal host fault, not a transient journal-capacity
   refusal.
   An observation consumed into a packet advances it even if audio later
   refuses the note or bend. The audio owner independently checks ID, origin
   and nondecreasing nominal time for every served packet **before** ledger or
   ingress mutation, including a key-up of a refused onset tombstone. A
   backward-time source observation is a terminal control-side fault that
   returns its original and keeps every open handle and reserved release
   place for teardown. A foreign, duplicate, reordered or backward-time
   audio packet likewise faults the whole transfer and retains its original
   for teardown. The current private path's
   per-event `NonMonotoneStamp` outcome is not this public transfer rule.
4. **One horizon check.** Audio checks every ordered packet against the
   profile's forward horizon at ingress admission, before any occurrence or
   ingress mutation. This includes a key-up that would only consume a
   tombstone. The checked admission is carried into the store offer without
   rechecking the horizon. A beyond-horizon packet is rejected and counted
   **once**, then raises a terminal transfer fault with its original ID and
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
6. **Result and terminal safety.** Before collecting a result, control checks
   its owner, plan, epoch, table, command ID and request kind against the
   head of the preallocated outstanding journal. A foreign, duplicate,
   reordered or mismatched result changes no ledger, count or reservation;
   it remains owned for joined teardown and faults the transfer without
   panicking. Every terminal host fault is a transfer fault. Every transfer
   fault, whether detected by control or audio,
   stops command service and causes the audio owner to silence the complete
   callback in which it observes the fault and every later callback until
   off-thread reprepare. A control-side fault reaches audio through a
   prepared atomic halt signal. Joined teardown reports sounding identities
   and outstanding release places; no note keeps sounding through a halted
   stream. Packet service, including calls into the host's qualified queue
   implementation, allocates nothing, takes no lock, makes no system call and
   does not panic. The outstanding-command bound means a truthful, sufficiently
   sized result queue cannot fill from this transfer's own results. A failed
   physical result push is therefore a terminal host fault: audio retains the
   result in its prepared pending slot, stops command service and silences the
   observing callback. A missing command/result is a terminal host fault when
   detected by a later ID gap or joined teardown; a missing final packet has
   no earlier liveness signal. The outstanding journal prevents credit reuse
   and names its command ID at teardown.

## Falsifiers and required proof

- The feature is enabled by default or a repository shipping target; it
  fails to imply `simulated-ingress`; or the constructor admits a missing or
  second producer, a second source, pedal, Panic, a non-initial candidate,
  plan replacement, loop playback or a crossed target partition.
- An undersized or non-empty command or result queue arms; a falsely reported
  capacity causes a failed push without a terminal fault; an unbound endpoint
  reaches a queue operation; or a failed physical push loses its prepared
  pending packet, handle, command ID or reserved place.
- A source observation or forged audio packet at a stamp below its respective
  frontier mutates a note, bend or ledger, or lets a younger packet run. Equal
  stamps fail to retain command-ID order.
  A transient refusal advances the control frontier, or an audio-refused
  packet fails to advance its frontier.
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
  reported. Check that the control pending slot retains the command's ID,
  handle and release reservation, or the audio pending slot retains the
  result. No younger command is served. Joined teardown names each packet;
  the callback that observes the fault and every later callback are silent.
- Deliver foreign and reordered commands and results from two prepared
  owners. Pass one owner's bound wrapper into the other's control and audio
  calls. Check that the receiving half faults before queue access and that
  audio silences the observing callback. A join with a wrong wrapper returns
  both halves and all wrappers unchanged without draining either queue;
  retrying with the correct wrappers accounts for every outstanding packet.
- Raise a control-side backward-time fault and verify that its atomic halt
  reaches audio. Count allocations around the complete callback, including
  concrete queue operations, and inspect that queue implementation for locks,
  blocking, system calls, panic and callback-time destruction.
- Omit one wrapper after quiescence and verify that `MissingEndpoint` names
  outstanding IDs and handles, marks the queue whose consumer was omitted as
  having unknown contents, and never claims complete joined custody. A
  loss-injecting queue fixture confirms that a missing final packet is first
  reported at teardown.

Crate-internal unit tests force command-ID exhaustion, a no-vacancy onset
with exhausted empty cells and other held cells despite command room, and
continued use of another free cell after one cell exhausts. They also inject
a valid-ID, earlier-stamped audio
packet and a duplicate command; the opaque public API cannot forge or
practically exhaust those cases.
A deterministic render-step unit fixture injects an audio-detected horizon
fault **after the first quantum** of a multi-quantum callback, and proves
that the complete observing block and later blocks are silent. The
required gate commands are:

```bash
cargo test -p synth_engine_v2 --no-default-features --features mixed-transfer-rehearsal --test mixed_transfer_rehearsal
cargo test -p synth_engine_v2 --no-default-features --features mixed-transfer-rehearsal --lib
```

Implementation must add both commands to `.github/workflows/quality.yml` and
`AGENTS.md`'s complete gate before claiming CI proof. Ordinary workspace
tests do not execute a default-off feature test.

## Open work and consequence

The user-selected stage-full bend sound split, two-source frontiers, pedal
and lift, Panic, raw/recorder shared charge, combined outcomes, plan swap,
loop behavior, ADR-0054's full six-share reselection, platform timing and
Phase 9 exit remain open. The user-owned combined-host DRAFT is not accepted
by this record. A later two-source decision must resolve any different
stamp-order rule explicitly before enabling a production live adapter.

If accepted, this record extends ADR-0072's audio-owned live identity and
release-at-offer exception beyond an exclusive owner solely for this
default-off, non-shipping initial one-shot rehearsal. It narrows ADR-0046
clause 6, ADR-0047 clause 7, ADR-0050 clause 8 and ADR-0076 decision 3
only to that extent. ADR-0075 remains deferred. Update the current host I/O
specification before exposing the API. The maintainer must approve this
public rehearsal boundary and new manifest feature; acceptance alone does
not mark Phase 9 complete.
