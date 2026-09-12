# ADR-0063: Linux CPAL callback custody

| Field | Value |
|---|---|
| ID | ADR-0063 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-12 |
| Last reviewed | 2026-09-12 |
| Related | ADR-0036, ADR-0061, ADR-0064, ADR-0065, IO-INV-001, IO-INV-002 |
| Supersedes | — |
| Superseded by | — |

## Boundary and decision

The first non-shipping Linux output harness uses V1's CPAL dependency, with the
host explicitly fixed to ALSA. The private control-thread owner keeps the entire
SoundCore callback state in a one-slot ring buffer. The callback owns only its
consumer handle and borrows the occupied item in place; it never pops it. A
controller-held `Arc` retains the storage until the backend worker has joined.

This fixes a real-time custody boundary that a normal owning callback would
violate: CPAL's ALSA worker owns and destroys its callback closure. The closure
must therefore contain no finally owned SoundCore renderer, plan or buffer. A
loose custodian/callback pair would not enforce the required order, so the harness
exposes neither half. Its owner is explicitly neither `Send` nor `Sync`.

The owner implements `Drop`: clear Ready, take and drop the stream, then allow
field destruction. Explicit close uses the same sequence before extracting the
state. A successful open is immediately wrapped in this owner before any further
fallible capability query or preparation. A failed build has no prepared state;
a later failure still joins before releasing storage. A callback may race the
start of shutdown or already be running. Quiescence is promised after join only.

The supporting implementation facts are pinned to CPAL 0.18.2
`src/host/alsa/mod.rs` (`Stream::drop`, `StreamTrait::buffer_size` and the output
worker), and ringbuf 0.5.1's consumer and shared-buffer ownership. ALSA stream
destruction releases the play latch, signals stopping, wakes and joins the worker.
Its reported buffer size is the negotiated period, and the worker presents one
period of interleaved samples. A dependency-version test forces reconsideration
before either supporting version changes. Selecting another CPAL host also
requires a new lifecycle proof; enabling another backend feature cannot select it
through this harness.

## Current scope and falsifier

The [output custody harness](../specs/spec-host-io-lifecycle.md#linux-output-custody-harness)
opens an exact ALSA endpoint, prepares a silent V2 graph off-thread and converts
its output into F32 or I32 device samples. The preparation ceiling is a budget,
not a device guarantee. An unavailable, zero or over-budget period refuses before
play. Malformed callbacks refuse before rendering, leave CPAL's prefilled silence
and retain a scalar fault. The controller prints diagnostics after stream join.

The falsifier is a path that releases the retained state before callback
quiescence or destroys it on the worker. Tests run the actual owner destructor
against a worker that destroys its consumer without a final callback, with zero
or repeated callbacks, and while a state borrow remains in flight. Constructor
cleanup with empty storage is also exercised. Output tests hold the render clock
and silence to callback partitioning and both supported sample formats; invalid
buffers and counter exhaustion advance neither render clock nor successful count.

These tests establish custody and bounded data-callback behavior, not a timing or
whole-worker allocation measurement. CPAL backend errors may themselves own
strings. No errors are leaked to conceal their destruction. Physical timing,
input, MIDI and held-note plan replacement keep their existing gates.
The concrete concurrent-transport owner and queues are covered by
[ADR-0064](ADR-0064-concurrent-compiled-session-handoff.md#concrete-linux-queue-consumer).
A fresh renderer starts at clock zero with its initial silent carry. ADR-0064
also owns the separate stopped-plan mailbox and preserved-clock readmission.

## Exclusive loop output consumer

The `loops` mode places ADR-0065's `JournaledLoopStream` inside the same occupied
callback-state pool. A private enum selects the ordinary session or loop owner
once off-thread before Ready; no callback can replace or drop its variant. Only
the session variant has a `ControlLane`. Its existing transfer-memory charge now
counts the actual enum layout instead of the narrower `AudioLane`, once. Loop
preparation checks that same inline-owner ceiling and sets additional heap
ceilings of 262144 bytes for loop programs and 8192 bytes for the journal.

The loop's interval is `[0,513)`, its entry is zero and its journal admits four
observed passes. The graph remains silent for the physical custody exercise.
Each entire backend callback calls the loop owner once. Splitting that call
would incorrectly retain early success when a later quantum fails. Shared
progress therefore uses the journal's latest successful acknowledgement, never
a partly advanced underlying renderer. Ordinary sessions retain their existing
ingress cut, quantum windows and off-thread command/plan collection.

Explicit joined close finishes the loop journal and exposes it to the controller
before any run error is propagated. Implicit close finishes and reports it after
joining as well. The first observation endpoint cannot be overwritten by later
playback or backend failure; backend error categories remain separate retained
diagnostics. `Finished` means that observation was explicitly closed, not that a
recorded take or a physical timing qualification succeeded.

Four added example tests cover both output formats and callback partitions,
whole-call refusal despite smaller admissible quanta, callback disappearance
through both close paths and shutdown with a loop borrow in flight. The original
transport/mailbox/custody checks remain required. An owner destroyed before join,
a renewed pass budget, an ordinary control owner in loop mode or progress from a
failed whole call falsifies this extension. No physical input mapper, recording
source fence, loop Play/Stop control or live-note authority is introduced.

A fresh Claude Sonnet uncommitted review found no blocking defect in this
extension's ownership, whole-callback authority, memory charge, control exclusion
or diagnostics. The summary's coarse `Render` fault category is supplemented by
the retained journal's exact terminal cause.
