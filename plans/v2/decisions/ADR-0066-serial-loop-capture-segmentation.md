# ADR-0066: Serial loop capture segmentation

| Field | Value |
|---|---|
| ID | ADR-0066 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-12 |
| Last reviewed | 2026-09-12 |
| Related | ADR-0065, ADR-0062, ADR-0024, TAKE-INV-003, TAKE-INV-006 |
| Supersedes | — |
| Superseded by | — |

## Boundary and decision

`LoopCaptureSession` exclusively owns an actual `JournaledLoopStream` and an exact
serial `SimulatedNoteRecorder`. No mutable inner owner or supplied boundary packet
can substitute another render history. It is a reference consumer for loop capture;
there is no concurrent source merge, physical input mapper, ordinary session control
or live audition. The Linux loop-output harness remains a separate journal consumer.

One capture reserves a continuous raw source-FIFO log and logical pass segments.
This clarifies TAKE-INV-003's separate segments: each has its own recording pass
identity and half-open engine interval, while sharing accepted raw cells. Splitting
storage never injects a played note-on or resets physical occurrence pairing. At a
boundary T, nominal times before T belong to the old segment and T belongs to the
new one. This logical routing does not reorder source publication. The serial caller
still supplies globally nondecreasing publication time and each source's FIFO order;
nominal regressions retain their existing anomaly diagnostics and late-refusal law.

Only actual successful whole-render observations authorize pass boundaries. The
journal's first terminal endpoint remains immutable while playback may continue.
Input can arrive before the journal advances; its accepted raw data stays owned.
No pass result is sealed from a worker clock or a partly failed render invocation.
Audio observation and source publication cuts are independent authorities.

## Context and identities

An explicit `NoteMapping` discriminant distinguishes the ordinary linear anchor
from `LoopCaptureMapping`. Loop preparation retains the original target, revision,
musical interval, tempo and quantization. Its tempo rate must match the loop renderer,
and its forward-mapped musical endpoints must equal the loop's frame endpoints.
Arbitrary frame loops that cannot meet that check refuse capture without being
rounded or assigned a fabricated tempo. The finite raw interval starts at the
journal's actual initial clock and covers its initial suffix and P-1 full passes.

Arm checks and reserves P fresh `CapturePassId` values, including unused identities,
before another take could reuse them. They are distinct from renderer `LoopPassId`
and storage indices. Each materialized descriptor binds both identities, its engine
interval and its initial plan position. An already pending wrap can give the initial
pass an empty interval; an exclusive terminal boundary does not add an empty final
pass. The combined owner permits one arm and retains its result until explicit discard
or owner destruction off-thread.

Every single-anchor host/projection consumer now explicitly requests a linear anchor.
Loop takes have none. Automatic note projection refuses loop mapping before allocating
anything. Key-carry metadata is not a musical projection certificate: nonpositive
lifetimes, unknown pedal state, isolated continuation and explicit trimming still
need the later pass-aware projection and recovery selection contract.

## Finalization and bounded carry

Loop arm closes an explicit seal-readiness barrier. Ordinary hot start, publish,
fence and quiescence operations can stop the raw capture but cannot seal through
that barrier. Ordinary linear captures retain their existing automatic sealing.
The off-thread `finalize` operation first requires an actual journal terminal and
all participating consumed source fences at the selected endpoint; interruption
also requires source quiescence. It owns retrospective shortening to the earlier
of the raw and acknowledged audio endpoints. It writes pass/carry metadata before
opening the barrier and invoking ordinary raw finalization. No metadata is written
after sealing, and the heavy scan is not reachable from a hot operation.

Finishing observation normally selects a complete admitted prefix. Reaching the
prepared P-pass endpoint is also complete for that admitted window. A render fault
selects `Interrupted` with `LoopRenderFault`, while the journal retains its exact
render cause. Earlier raw capacity, source failure and quality still apply: the
existing minimum-endpoint and outcome-severity rules cannot be replaced by a later
complete audio result. The journal's first reason and capture quality are separate.

The physical held-note limit H does not bound nominal-time carry. For H=1, source
FIFO on(0), off(100), on(1), off(101), on(2), off(102), published monotonically at or
after those nominal times without an intervening fence, has only one physically
held key at a time but three positive lifetimes crossing T=50. Sorting source
pairing or rejecting all accepted regressions would silently change the contract.

At each actual interior boundary, the worker reconstructs captured key continuation
from accepted onsets strictly before T and accepted paired releases not strictly before T.
A release exactly at T therefore retains the old pass's key-held carry-out and the
new pass's corresponding carry-in before its release. A refused release is not an
accepted raw release and cannot silently remove that log's continuation. Sticky
quality accompanies the metadata; a partial log is not certified physical state. Uncaptured initial keys never
become captured attacks or carry records. Sustain remains separate raw controller
and key-release data, not unbounded additional held-key entries. Later projection
must reconstruct selected controller state from that retained history.

Before writing either carry half, count all required occurrences. If more than H
are needed, select `Partial` at that boundary with `LoopCarryCapacity`, retain its
location separately, and admit neither the next pass nor that boundary's carry
pair. Preserve earlier metadata and every accepted raw record, including raw beyond
the shortened selection. Existing per-onset closure fields can represent more than
H historical terminal notes; H's reusable live slots are not used for that task.
The source/finalization requirements remain in force. Source interruption can retain
the stronger interrupted outcome alongside the carry-capacity location.

## Storage and falsifier

P pass cells and 2H(P-1) carry cells are reserved through the existing size-based
recording layout. New cell variants and immutable contexts participate in that
same preflight. The recording ceiling also charges the combined inline owner once;
the journal heap has its separate explicit ceiling, and compiled-loop renderer and
program allocations retain their own profile/budget. No claim sums these into an
unmeasured whole-process allocation figure. Finalization scans the admitted E/P
storage off-thread and allocates no intermediate collection. Reading does not renew
raw, pass, carry, result or journal entitlements.

A dropped accepted raw record, fabricated boundary, FIFO reassociation, event at T
assigned to the old pass, more than H admitted carry, premature sealing, hidden
linear loop projection or hot allocation/deallocation falsifies this consumer.
Tests under `recording/notes/loop_capture/tests.rs` compare actual impulse audio and
segments across callback partitions, exercise the H asymmetry, source/audio stalls,
quiescence without another render, pre-capture keys, pending wraps, exact byte limits,
identity exhaustion and late quality after sealing. The ordinary recorder, host
session and projection suites remain required.

The author framed the held-count asymmetry before building. A fresh Claude Sonnet
design consultation found no blocking defect in the raw-log, retrospective sealing
or bounded carry law. A fresh uncommitted code/specification review found no blocking defect. A focused
reread accepted the repair separating accepted raw releases from refused physical
pairing diagnostics, the typed owner-byte charge and the added boundary/rate tests. Concurrent transfer, loop Play/Stop, pass-aware
projection, audio capture/monitoring and ADR-0022 qualification remain later consumers.
