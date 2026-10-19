# Phase 9 backend instrumentation proposal

## Implementation decision requested

The software deliverable is this concrete scope for the maintainer's decision.
It adds no unsafe implementation, dependency fork, physical observation or
accepted estimator. ADR-0022's existing collector/analyzer and the application
remain unchanged. The six-point request authorizes preparing this proposal;
repository instructions still require explicit approval before new unsafe getter
implementations are written.

The proposed permission is limited to three getter implementations inside a pinned
development-only fork of alsa 0.11.0, diagnostic extraction in a pinned CPAL
0.18.2 fork, and an isolated standalone development probe. It excludes changing
shipping dependencies, exposing raw pointers, adding a second status query,
changing ALSA timestamp requests, and enabling a production clock mapping.

## Exact source and interface boundary

The inspected resolved sources are `alsa-0.11.0/src/pcm.rs`,
`alsa-sys-0.4.0/src/generated.rs` and `cpal-0.18.2/src/host/alsa/mod.rs` in Cargo's
registry. Retain their package checksums from `Cargo.lock` and the exact fork
commits in the probe's eventual isolated lockfile. No workspace patch or
`Cargo.lock` update is part of this proposal.

| Getter on `alsa::pcm::Status` | Existing FFI symbol | Owned return contract |
|---|---|---|
| `audio_timestamp_report` | `snd_pcm_status_get_audio_htstamp_report` | Report validity bit, raw actual-type value, accuracy-availability bit and accuracy in nanoseconds; validity and availability remain separate |
| `driver_timestamp` | `snd_pcm_status_get_driver_htstamp` | Owned seconds/nanoseconds with checked conversion; zero remains unavailable and invalid nanoseconds are rejected |
| `checked_state` | `snd_pcm_status_get_state` | Fallible known PCM state with the raw native value retained on rejection; never call the wrapper's panicking `get_state` for added diagnostics |

These are proposed API names, not implemented functions. The report uses the
existing generated `snd_pcm_audio_tstamp_report_t` ABI, whose safe bit accessors
are `valid()`, `actual_type()` and `accuracy_report()`, with `accuracy` separately
stored. Unknown actual-type values remain unknown raw values in diagnostic
records and cannot acquire a known source classification. Invalid reports retain
their raw fields for diagnosis but cannot establish timestamp provenance.

The only new unsafe operations proposed are calls to the three named C getters
with `Status`'s existing private pointer and appropriately initialized writable
output storage where required. The raw state getter returns a value directly;
convert it without `unwrap`, preserving unknown/private values as errors. No raw pointer escapes; the result holds no reference to native
storage. Before implementation, audit the linked ALSA declarations and ownership
requirements, output initialization, lifetime and aliasing arguments, architecture
layout and zero/unwritten-field behavior. The approval does not authorize other
FFI calls or changes to `Status` storage, its private pointer accessor or native
lifetime. Existing generated bindings are dependencies, not newly authored unsafe
implementations in this repository.

## CPAL extraction sites

The stream driven by CPAL is the only source under investigation. Extract from
already acquired status objects on that handle; never open a second PCM handle.

| Existing site | Required diagnostic |
|---|---|
| `build_stream_inner`, prepare-time `handle.status()` | Direction, actual device configuration, creation timestamp and selected `TimestampMode`; retain whether the zero-origin branch selected `CreationInstant` |
| `set_sw_params_from_format` | Requested `MonotonicRaw`, actual `sw_params` acceptance/error and the existing fallback to `Monotonic`; no extra configuration attempt |
| Poll path, existing `avail_delay()` | Its independent query identity, raw delay/availability, errors and suspension/xrun recovery; preserve its order before the status query and the negative-delay clamp flag |
| Poll path, existing `StatusBuilder::build(&stream.handle)` | Requested audio timestamp type and delay policy, exact query success/failure, owned report/driver/system/audio/trigger timestamps and PCM state from this same status |
| `StreamInner::callback_instant` | Selected mode and actual branch: creation `Instant`, system timestamp, audio-link timestamp, non-running fallback, or invalid-trigger-origin fallback |
| `htstamp_elapsed` | Signed difference before the existing clamp and explicit negative-clamp flag; retain raw values rather than reconstructing them from the clamped instant |
| `StreamTrait::now` | Existing status-query success with its actual timestamp request, or status failure and creation-`Instant` fallback; distinguish the direct creation-mode branch |
| Input/output callback construction | Correlate both the status and the separate `avail_delay` query; record capture's checked-subtraction clamp and playback conversion, with no inferred hardware position |
| Input read loop | Actual read count, zero-read EAGAIN without callback delivery, partial-read xrun, and the additional suspend-state query; retain status identity even when it never serves a callback |
| Output write loop | Callback frames offered, actual successful write count, zero-write EAGAIN after callback delivery, partial-write xrun and the additional suspend-state query; never claim an unwritten frame window reached playback |

The prepare-time status query uses its existing configuration. Extraction must
not add `audio_htstamp_config` to it. Poll and `now` keep their existing requested
type and `report_delay` values. No diagnostic path calls `now` just to observe it:
that would add a status query. Status errors and interrupted/startup/recovery
branches generate their own explicit diagnostic events where they already occur.
The existing `avail_delay` and status calls remain distinct observations with
explicit dependency ordering; they do not become one atomic hardware sample.
A missing callback or report cannot be represented as a fabricated valid row.

## Record ownership and bounded cost

Use a fixed-capacity owned diagnostic record and a preallocated bounded queue
from the backend worker to an off-thread writer. No formatting, serialization,
file I/O, allocation, blocking lock or waiting occurs in extraction or an audio
callback. Record construction only copies bounded scalar values. One sampled
status has a stream/epoch identity and a monotonically checked sequence; all
branch events refer to that status identity without adding a query. The
already-existing `avail_delay` query has a separate identity, and callback
construction records both; never assign its delay to the status's acquisition
interval. Exhaustion stops admission and is reported, never wrapped into reuse.

Producer ownership must be decided before implementation: callback processing
and application `now` calls can originate on different threads. They must not
share one SPSC producer. Give each participating producer its own admitted queue
and sequence, with explicit status/callback correlation; merge only off-thread.
The writer cannot infer a total physical order from independent sequences.
Predeclare a finite producer set; unregistered thread access is counted and
makes the run inconclusive rather than allocating a new lane.

Each lane counts queue-full losses and refuses overwritten or reused sequence
identities. Preserve cumulative loss/error counters in a separately readable
terminal summary so a full queue cannot hide its own final failure. Instrumented
and uninstrumented runs are distinct artifacts; diagnostic overhead must be
measured before using the candidate for timing bounds. Retain extraction's
observer interval and queue occupancy. The writer must not claim an unobserved host-query interval as measured.
For the standalone diagnostic probe only, bracket each existing backend query
with two `Instant` observations from one retained arm-local observer origin.
Record that independent observer clock and its interval separately from native
ALSA clocks. These explicit observer reads are permitted diagnostic overhead;
they can use a system-call slow path and are not part of the application's
real-time kernel or a production timing guarantee. Measure their overhead in
instrumented/uninstrumented controls before deriving any timing bound. This
interval can supply the version-1 analyzer's observer ordering fields, but it
cannot supply a hardware clock-age or frame-alignment bound.

Shutdown fences every producer before the writer drains and publishes a final
artifact. Device recovery, a changed trigger origin, reconfiguration and restart
start a new epoch or explicitly stop the arm. The writer preserves partial/error
artifacts and reports whether every producer fence and terminal counter arrived.
No successful file write is inferred from an attempted send.

## Existing analyzer mapping

`scripts/clock_observability.py` consumes closed version-1 owned reports. Keep
that contract unchanged. Build an explicit off-thread adapter after the raw instrumentation format is
specified and versioned. Only the callback-serving producer's rows enter a
direction's contiguous analyzer sequence. Prepare, `now`, write and recovery
events stay in the raw diagnostic artifact and its terminal summary; they are
not discarded or interleaved into that callback sequence. Unexpected concurrent
query/configuration events invalidate the arm rather than being reordered into
an apparent physical total order. It validates integer ranges,
clock/type spellings, direction, branch, loss/error counters and required nulls.
A diagnostic value that cannot be represented faithfully causes a clear refusal;
it is not silently dropped or recast as another field's meaning.

Actual hardware frame position, frame/timestamp alignment and clock-age bounds
are not supplied by these three getters. They remain null unless a separately
justified same-stream derivation establishes them. A callback frame counter,
ALSA delay/availability or the requested type alone does not establish that
alignment. With representable required fields and real observer brackets, the best
reachable observability verdict under this scope cannot exceed `Inconclusive`:
there is no authorized hardware-position/alignment/age derivation. A duplex arm
is expected to `Stop` on playback's initial PREPARED-state callbacks; its
sequence-zero startup prefix must not be dropped to manufacture an inconclusive
result. An input-only arm may reach `Inconclusive`. Missing
required non-null fields or unknown native states cause explicit adapter
refusal instead of a fabricated row. The affine synthetic controls remain
synthetic, with `physical_qualification: false`.

## Falsifiers and verification after approval

Acceptance fails if instrumentation adds a status query or timestamp-config
write, changes selected/fallback branches, exposes native ownership, loses or
mixes provenance, hides queue loss, aliases a sequence, adds a panic, allocates/blocks in callback processing, or asserts a hardware frame/clock bound absent from the observation.
These are implementation acceptance criteria; they do not accept ADR-0022.

The new probe does not run the old collector's `Stream::now()` bridge bursts.
The old collector stays unchanged and runs separately. Natural `now` calls
remain instrumented when exercised in a separate declared arm; they never
serve as an added observer on the callback arm. Branch controls can inject
owned failures without extra hardware queries. No interleaving equivalence
between those arms is claimed.

Before collecting hardware, inspect the actual running kernel/library path and
prove that the selected getters extract the acquired status without an extra
query/configuration interleaving. Specifically exclude a request bitfield
being echoed and decoded as a valid returned report. Raw validity bits alone
are not an attestation that the backend populated the report; an unverified
pack/unpack/kernel path refuses interpretation while preserving the raw data. The previously cited upstream kernel example
is not evidence about the running host. Test raw owned extraction with explicit
invalid, unknown, zero, missing and out-of-range cases; exercise every existing
fallback and error branch; saturate each queue, stall the writer, exhaust sequence
IDs through injected private issuers and close after failures. Compare callback
samples/receipts and backend call counts with uninstrumented controls. Run the
isolated probe's checks independently of workspace gates and record its exact
build/dependency graph. Physical paired-reference, loopback and platform evidence
still require their actual hardware and representative environment.
