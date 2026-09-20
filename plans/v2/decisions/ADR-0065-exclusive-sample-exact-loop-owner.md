# ADR-0065: Exclusive sample-exact loop owner

| Field | Value |
|---|---|
| ID | ADR-0065 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-12 |
| Last reviewed | 2026-09-12 |
| Related | ADR-0032, ADR-0046, ADR-0047, ADR-0050, ADR-0051, ADR-0052, ADR-0055 |
| Supersedes | ADR-0052; loop-owner scope of ADR-0032, ADR-0046, ADR-0047, ADR-0050 and ADR-0055 |
| Superseded by | — |

## Durable boundary and readiness

The selected Phase 9 runtime-loop work needs an implementable answer to ADR-0052's
coupled sample granularity, note identity and repeated admission questions. The
choice binds the audio owner's minting authority and audible boundary behavior.
The historical proposals in ADR-0052 cannot supply a runtime consumer unchanged.

## Decision

A standalone `CompiledLoopStream` exclusively owns its control/minter, renderer,
normalized initial suffix, normalized full pass and publication arbiter. None of
those mutable authorities escapes. It accepts a fixed loop interval and entry,
with `StealingPolicy::None`; it exposes no ordinary activation or live ingress.
Its internal wrap is a sample-exact timeline boundary, not an adoption of a
previously stamped `TransportActivation`.

This explicitly amends ADR-0032's wrap-anchor mechanism and ADR-0050's inclusion
of wraps among ordinary activations for this consumer. ADR-0055's offer refusal
continues to protect ordinary schedulers and compiled sessions. The new owner
cannot create a pending activation whose minter view would become stale at a wrap.
General seek, tempo replacement, ordered Stop/Play and mixed live-note ownership
are outside this constructor and must be integrated before those controls exist.

Each newly rendered quantum still contains exactly 64 samples. One typed timeline
map supplies every sample's plan position to both the modulation prepass and main
kernel walk. One publication and one renderer call cover that quantum even when
it holds many wraps. Priming latency and carry remain the renderer's normal Q.
Every wrap ends held occurrences through chronological timed NoteOff events,
restores destination state, then emits that sample's new compiled events.

ADR-0047's occurrence remains exactly table, index and generation. Its minting
owner is extended to this exclusive audio owner: every new note-on calls the
existing authoritative minter, every release advances or retires that index,
and no working-copy snapshot is restored at a wrap. A separate checked u64
`LoopPassId`, scoped to the stream epoch, starts at one and identifies musical
passes; it is not a fourth component of a note identity.

## Preparation, catch-up and admission

Preparation preserves source history before entry and discards events at or after
the exclusive loop end. It reuses the existing activation compiler for occurrence
pairing, source-history gates and catch-up. An empty preparation-only scheduler
satisfies that compiler's ownership precondition without stamping any note into
the authoritative minter. Both temporary activation candidates are withdrawn;
their working copies are never promoted. The renderer and runtime minter start
fresh, and the loop stores tokenized templates instead of those stamped identities.

ADR-0051's destination-open gate predicate remains the source history immediately
before entry or loop start. Each prepared target has a catch-up row; gates held
by a crossing source note restore zero, even if later automation wrote another
value. A crossing release keeps its physical gate write when required but loses
the missing note contract. Omitted releases and expressions are exposed as
separate initial/full-pass template counts. They are not lifetime render counters.

For this loop consumer, ADR-0046's single mass-release operation is replaced by
one Session charge per actual timed held-note release, plus one boundary operation
and every catch-up row. Ordinary activations and Panic keep their existing charge.
Compiled events keep the Compiled share. Preparation admits both shares over the
full periodic pass and separately over the initial suffix-to-first-wrap junction.
The latter can fail even when each full pass fits. Existing normalized-pass
polyphony admission remains required. A one-frame loop is legal only when its
actual repeated work fits; there is no artificial minimum of one quantum.

A new crate-private arbiter factory prepares exactly one quantum using the real
profile's shares and event cap, including when the maximum host callback is below
Q. It shares allocation with the ordinary maximum-window factory. The loop never
clears a maximum-callback ledger per wrap. Token cleanup visits only the compact
held prefix, not every occurrence in a potentially long pass. Work is bounded by
Q cursor steps, admitted events and profile-bounded note/control expansion.

An explicit additional byte budget covers retained templates, catch-up rows,
token routing, held routing and the maximum callback's boundary observations.
Its preflight uses conservative source-event counts before normalization, so
omitted events may be overcharged. This budget is for retained additional heap
storage, not transient compiler allocations or total process memory. Existing
renderer, minter and arbiter storage retains the supplied profile's admission.

## Timing, failures and observation

Entry must lie in the interval; exactly the exclusive end normalizes to the start
of pass one. Position at the end in a snapshot means the next render step wraps:
a boundary after the final sample of a quantum is processed with the next quantum.
Engine clock and position snapshots describe rendered input, not delivered audio
or hardware timestamps. Last-call borrowed boundary observations carry epoch,
engine sample, previous/next pass and interval. They must be consumed before the
next successful call replaces them; they are not a concurrent capture queue.

Sample-rate controls, note edges and position-aware kernels share the exact map.
A sub-quantum catch-up to a quantum-rate control takes effect at the next Q boundary
under its normal smoothing policy. This explicitly narrows SOUND-INV-024's
activation-seeding exception to ordinary activations: a loop wrap does not seed
all slots or reset unrelated DSP state. Current declared policies remain None.

Clock, pass or note-generation exhaustion, publication failure or inconsistent
prepared routing terminates the owner: the entire outer callback is silenced,
both carries are invalidated and the exact fault is retained. Later calls return
that fault with silence. Owners remain retained for off-thread destruction.
An ordinary output-shape refusal leaves state intact; an oversized callback is
terminal under the existing renderer contract.

## Retained finite observations

`JournaledLoopStream` takes exclusive ownership of a healthy compiled loop and
retains its initial snapshot, P-1 interior boundary cells and one inline terminal
descriptor for an admitted `LoopPassCount` observation window of P passes.
This quantity is distinct from the recorder's `CapturePassCount`. Preparation
rejects zero passes and checks the additional heap against an explicit byte ceiling.
P=1 needs no boundary heap. The initial position can be the exclusive loop end,
meaning the next render starts with a pending wrap and an empty initial segment.
These are loop epoch/pass observations, not recording-session pass identities.

Only a successful whole outer callback appends boundaries and acknowledges its
render snapshot. The first boundary beyond the pass budget ends observation at
that sample, without admitting a new observed pass or stopping playback. Reads
never clear cells or replenish the total budget. The acknowledged render snapshot
can continue advancing after this terminal endpoint; neither is hardware time.

A terminal render fault contributes no boundaries from the failed callback and
ends observation at the previous successful render frontier. Partial internal
advancement in the silenced callback cannot move that frontier. A shape refusal
leaves observations unchanged. Explicit finish needs no further callback and
uses the same successful frontier. The first terminal descriptor is immutable,
including when playback later faults. All owning storage remains retained for
off-thread destruction. The wrapper exposes no mutable inner-loop access.

The non-shipping example's test module transfers Copy boundary and terminal
records through a preallocated P-cell SPSC lane while the reader retains its
backing through OS-thread join. P-1 interiors plus one terminal fit even when the
reader never drains. An intentionally undersized lane retains its first failed
record and stops publishing; the authoritative journal remains available after
join, including its unsent suffix. This test publisher scans the retained prefix;
its test bounds do not qualify a physical callback's transfer cost or provide a
production capture adapter. No intermediate audio-progress queue is claimed.

Losing an admitted boundary, renewing P through consumption, advancing the
acknowledgement on a failed call or retaining a transition from a silenced call
falsifies this observation contract. Seven core tests cover audio and boundary
oracles over 1/37/64/256/512-frame partitions, exact and one-under byte budgets,
one-pass zero-heap storage, mid-callback failure, shape refusal, pending-wrap
attachment, repeated finish, refusal of an already faulted source and first-use
allocation/deallocation guards. Five example tests cover actual retained heap
bytes, stalled readers, non-renewal, failed-lane retention and joined finish
without another callback. Their guards cover rendering and queue publication,
with owning destruction after join.
The core guards call `render_allocation::count_allocs`: its `GlobalAlloc::dealloc`
increments the same counter as allocation, so their zero assertion excludes both.
There is no separate deallocation counter that those tests need to assert.

The author framed this boundary before building it. A fresh Claude Sonnet
consultation found no defect in the finite journal's custody, bounds or failure
authority. It required clearer separation from the later source-ordering work.
The independent implementation review found that the initial Rust signature used
the recording subsystem's pass-count type. The repaired signature uses the
distinct loop-observation quantity throughout the core and example consumers.
This journal neither changes source fences nor seals raw takes at wraps. A
concurrent capture consumer still must establish source ordering, raw-record
retention, carry metadata and pass-aware projection under TAKE-INV-003. The
borrowed interface remains available on the unwrapped standalone owner.

## Evidence, alternatives and limits

The author framed the falsifiers before implementing. A fresh Claude Sonnet design
consultation identified missing explicit release charging, an insufficiently stated
ADR-0051 gate predicate and a factory described before it existed. The repaired
frame passed a focused consultation. The independent uncommitted review found
no ownership, timeline, admission or real-time defect; it questioned the wording
of the allocator evidence, clarified below after inspecting the existing guard.

`src/looping/tests.rs` checks actual audio and pass traces across 1, 37, 64, 256 and
512-frame callbacks, lengths 1, 2, 63, 64, 65, 127 and 513, interior/end entry,
held-note wrap and fresh generations, crossing releases, compact removal, byte
ceilings, exact Session share and the initial junction, clock/pass/generation
exhaustion, mapped-span refusal and first-use allocator guards. The existing
`src/tests/render_allocation.rs::count_allocs` counts allocation, deallocation,
zeroed allocation and reallocation during the guarded render calls. A zero count
therefore excludes heap reclamation during those calls; it does not claim that
the test destroys the complete loop owner on the audio thread.
`src/time/timeline/tests.rs` and the mapped impulse kernel test check timeline
lookup independently. The existing linear renderer evidence must remain unchanged.
These are deterministic correctness checks, not physical timing qualification.

The status quo leaves loops refused. Per-wrap off-thread preparation would retain
that owner's authority but make continuation depend on timely control work.
Replaying a stamped activation would alias identities and reuse consumed ownership.
The exclusive owner pays retained full-pass storage and runtime mint/release work
to make already-admitted continuation independent of control-thread progress.

Revisit this boundary before enabling live/authored ingress, held-plan changes,
stealing, general seeks or concurrent loop capture. No shipping dependency,
persisted field or wire meaning changes, and no phase exit is claimed.

## Specification update

[SOUND-INV-018 and SOUND-INV-024](../specs/spec-sound-core-render-contract.md#exclusive-compiled-loops)
carry the consumer-specific mechanism, minting and control-rate exception.

## Review

Reviewer: fresh Claude Sonnet, direct read-only uncommitted review. Its sole
reported defect inferred missing deallocation coverage from the helper's name;
the actual GlobalAlloc implementation counts deallocation too. The evidence
wording now states the guarded operations and its scope explicitly. The focused
reread confirmed the guard and successor scope, and found one remaining ADR-0055
index row implying that the ordinary guard would end when a successor existed.
That row now explicitly preserves the ordinary guard; its final focused reread
found no remaining blocking defects.

The finite-journal extension received a separate fresh Claude Sonnet uncommitted
review and focused reread. The dedicated `LoopPassCount` repair, allocator-byte
test and shared allocation/deallocation counter were accepted with no remaining
blocker. Its optional request to name the existing faulted-source rejection test
is reflected in the coverage list above.

Stopping rule: a false claim, internal contradiction, unfillable contract,
identity/real-time defect or unsupported evidence blocks acceptance. Optional
implementation detail does not.

## Subsequent finite serial control consumer

[ADR-0067](ADR-0067-ordered-serial-loop-recording.md) resolves the deferred finite
serial Play/Stop recording consumer while retaining this standalone constructor's
contract. Its current rules live in the
[host I/O specification](../specs/spec-host-io-lifecycle.md#ordered-serial-loop-recording).
Concurrent transfer, restart, physical timing and mixed live ownership remain outside
that consumer.
