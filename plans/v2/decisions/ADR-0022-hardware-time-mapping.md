# ADR-0022: Hardware Time Mapping and Latency Ownership

| Field         | Value                                                        |
|---------------|--------------------------------------------------------------|
| ID            | ADR-0022                                                     |
| Status        | Deferred                                                     |
| Phase         | 0A/9, deferred to the Phase 9 exit gate                       |
| Created       | 2026-08-13                                                   |
| Last reviewed | 2026-10-18                                                   |
| Related       | ADR-0001, ADR-0032, ADR-0021, ADR-0036, ADR-0023, EVD-0016, P00A-T006 |
| Supersedes    | —                                                            |
| Superseded by | —                                                            |

**Class.** `Contract`. It decides ownership boundaries and error behavior, not a value.

## The deferral

| Field                | Value                                                                                   |
|----------------------|-----------------------------------------------------------------------------------------|
| Deferred to          | The **Phase 9 exit gate**. Phase 9 may build evidence candidates, but cannot exit or qualify live timing before this is `Accepted` |
| Owner                | Project maintainer — this is a single-maintainer repository, so there is no second party to assign |
| Evidence required    | A simulated-host harness with controllable timestamps, drift, block sizes, and disconnects; retained per-callback timestamps on the three release platforms; a paired-reference arrival uncertainty per untimestamped adapter; and an observer-clock bridge per hardware-timestamped adapter connection |
| Why not now          | The evidence does not exist and cannot be completed without retained observations from every claimed release platform and initial adapter |
| What makes it safe   | ADR-0032 fixed the mapping's output shape; Phases 1-8 consume pre-mapped `SampleTime` and may not introduce a physical mapping or qualified live-timing claim |

The Phase 0A exit gate accepts this record as `Deferred` on those four fields. It does not accept silence, and it does
not accept a deferral without a named later gate: the master plan permits deferral **only** to the Phase 9 exit gate.

**Boundary correction, 2026-08-25.** The maintainer moved acceptance from Phase 3 entry to Phase 9 exit. Phase 3's
scheduler operates entirely on the engine-domain `SampleTime` produced at its ingress boundary, so requiring macOS,
Windows, and physical-adapter evidence before that scheduler existed made unrelated hardware availability block the
engine-time implementation needed to evaluate later ingress candidates. This does not weaken the evidence or support
bar: Phase 9 cannot exit, and no live hardware-timing configuration may be qualified, until this record is accepted
against the retained artifacts named above.

## Context

Something must state how a device or host clock relates to the engine's own time, and who pays for the latency the
engine declares. Neither question is answerable from V1, because V1 does not attempt either — and, more usefully, it is
now clear that V1 *has the raw materials on both ends and no consumer between them*. Read at `e4873d0b`:

- **The output side already measures true latency and throws it away.** The cpal backend takes the host's per-callback
  timestamps and computes the real gap between callback and playback
  (`ts.playback.duration_since(ts.callback)`, `cpal_backend.rs:329-334`; current worktree lines 338-343), falling back
  to the static estimate before the clock warms up. It passes the result as `AudioCallbackContext.output_latency`.
  `SynthEngine::process` reads
  `sample_rate`, `frames`, and `channels` from that context and nothing else, so the measurement reaches no consumer.
- **`stream_time` is a wall clock, not a device clock.** It is `start_time.elapsed()` from an `Instant`
  (`cpal_backend.rs:341`; current worktree line 350), which is a different quantity from the stream position beside it
  and cannot serve as a mapping basis.
- **The input side discards its timestamps.** The MIDI callback receives the driver's timestamp and ignores it
  (`io/midi.rs:247`), so a live event's only time is when the audio thread happened to drain the queue.
- **Nothing estimates drift, calibrates an anchor, or compensates anything.** There is no code to review for a
  candidate policy, only the absence of one.

**What this record will decide**, no later than the Phase 9 exit gate:

1. How the epoch anchor of [ADR-0032](ADR-0032-sample-time-and-event-timestamps.md) clause 13 is established at
   preparation, and from which host quantity.
2. Whether the anchor is corrected during an epoch — drift and jitter — and whether a correction may move events that
   are already scheduled. A correction that can move a scheduled event is a timing contract, not an implementation
   detail.
3. Who compensates the constant `Q` frames of latency ADR-0001 clause 7 declares: the engine, the host, or nobody, and
   how it is reported. ADR-0001 made it a named contributor precisely so that this record has something to own.
4. The input-side latency, and how it relates to monitoring and to recorded-take alignment.
5. The measured arrival-time uncertainty each untimestamped adapter must declare under ADR-0032 clause 19. That clause
   requires the declaration today and permits an explicit `unmeasured` marker until this record supplies the number.
6. Whether the late-event policy is refined. ADR-0001 clause 16 permits this record to refine it with hardware-clock
   evidence and forbids it restoring silent lateness.
7. Behavior across device reconfiguration and hotplug, where it meets ADR-0036's device lifecycle.

**Outside it.** The representation of time (ADR-0032), the quantum and the latency figure itself (ADR-0001), admission
and profile ownership (ADR-0021), and same-sample ordering (ADR-0023).

## Decision drivers

- Phase 3's exit gate requires that equivalent simulated timestamped-ingress and precompiled streams expressed as the
  same `SampleTime` sequence reach the same sample offsets. That establishes scheduler and ingress-boundary behavior
  without claiming that any physical clock has already been mapped correctly.
- Phase 9's exit and qualified live-timing claims require an actual hardware mapping and retained platform and adapter
  evidence. Those claims cannot be evaluated without this decision.
- A mapping decided without measurement would be a guess recorded as a contract, and the repository's own rule is that
  real-time and correctness claims require a reproducible `EVD` record or a named test.
- V1's own numbers are not a baseline here: it has no mapping to measure. The evidence must come from a simulated host,
which can be developed before Phase 9 while the physical evidence remains owned by its exit gate.
- The cost of being wrong is asymmetric. A wrong representation (ADR-0032) breaks compilation; a wrong *mapping* is
  audible as timing that drifts or jitters, and is discovered late.

## Options considered

**Deliberately not surveyed.** The candidate space is visible — a static anchor versus a continuously corrected one;
compensation in the engine versus in the host versus none; a drift correction that may move scheduled events versus one
that may not — but choosing among them is exactly what the missing evidence decides. Writing a fair options survey now
would produce three plausible paragraphs and no basis for preferring one, and an accepted record's options section is
supposed to record why the winner won.

What is *not* open, and is recorded here so the survey starts from it: the mapping must produce an engine-epoch
`SampleTime` (ADR-0032 clauses 13 and 17), a late event may not become silent (ADR-0001 clause 16), and an adapter may
not compensate its own unmeasured error (ADR-0032 clause 19).

### Status quo

Keep V1's arrangement: measured output latency computed and discarded, driver timestamps discarded, no anchor, no drift
handling. Phase 9's live qualification is unachievable, and live timing keeps the block-boundary quantization of
up to 21.3 ms at 48 kHz that ADR-0032 recorded.

## Evidence

**Active-evidence amendment, 2026-08-25.** Before acceptance, EVD-0016's first
platform observation exposed two missing controls in the deferral row: raw
callback retention was not explicit, and a hardware-timestamped adapter's
connection clock was not bridged to the audio observer. The row now requires
retained platform artifacts, paired-reference arrival measurements, and a
per-connection observer bridge. The amendment also disambiguates two fields
named `output_latency`: `AudioCallbackContext::output_latency` contains the
host-timestamp-derived value and has no reader, while the GUI reads the separate
`StreamInfo::output_latency` buffer-size estimate. The interim prohibition now
names the context field precisely and makes two existing producers explicit:
V1 may continue producing the unused context value, and isolated evidence probes
may observe host timestamps. No new production consumer is permitted. The net
change is a stronger evidence requirement and a disambiguated reader
prohibition; it does not reinterpret accepted evidence or implementation
because neither yet exists.

- Source reads at `e4873d0b`: `crates/pertylizer/src/audio/backends/cpal_backend.rs:329-342`
  (the same block is at lines 338-351 in the current worktree),
  `crates/pertylizer/src/io/midi.rs:247`, `crates/synth_engine/src/synth_engine.rs:4084-4094`,
  `crates/synth_core/src/audio/types.rs:213-226`.
- [ADR-0032](ADR-0032-sample-time-and-event-timestamps.md) clauses 13, 17-19, and 22, which name the obligations this
  record inherits.
- [ADR-0001](ADR-0001-internal-render-quantum.md) clauses 7 and 16.

**The evidence remains incomplete.** [EVD-0016](../evidence/phase-03/EVD-0016-host-time-mapping.md) now contains an
active simulated-host harness. An initial Linux observation through PipeWire's ALSA route was rejected as a physical
control. The final non-selected-bracket diagnostic Linux observation reports conservative components of 582 input
frames and 602 output frames after adding the pinned ALSA source audit's one-period `Stream::now()` freshness bound.
F4 requires their input-to-output sum to remain strictly below `Q = 64`; 1,184 frames is therefore `Not supported` for
the measured direct candidate. It is not a universal Linux result: load
was uncontrolled, the earlier raw trace was not retained, and that worktree had no final source revision.
A new [retained Linux observation](../evidence/phase-03/EVD-0016-host-time-mapping.md#retained-linux-observation-2026-09-12)
is tied to a clean source revision and remains `Not supported` under F4. A second
[retained Linux fixed-buffer diagnostic](../evidence/phase-03/EVD-0016-host-time-mapping.md#retained-linux-fixed-buffer-diagnostic-2026-09-26)
on a different ALC256 device configuration fires F7 for the input callback clock
moving backwards; the retained audit separately counts output backward steps and xruns.
It provides no F4 estimate or replacement mapping. The
[retained 2026-10-17 fixed-256 diagnostic](../evidence/phase-03/EVD-0016-host-time-mapping.md#retained-linux-fixed-256-diagnostic-2026-10-17)
on sof-hda-dsp at a fixed 256-frame buffer records over 10,000 callbacks and 2,579
bridges per direction without backwards callback clocks, backend errors or lost
records. It still fails F4: the independently bridged uncertainty sums to 642
frames against `Q = 64`. This is uncertainty, not measured round-trip latency;
zero xruns does not qualify the direct mapping. No replacement was introduced.
macOS and Windows callback artifacts remain missing, as does the paired-reference arrival measurement for every initial untimestamped V2 adapter. The Active-record self-audit also made
explicit that every hardware-timestamped adapter needs its own connection-clock bridge; a midir timestamp and a CPAL
`StreamInstant` do not share a raw origin. No replacement is yet characterized, and the diagnostic observation is not
a basis for accepting this decision.

## Replacement candidate frame — 2026-10-17

**Scope: source-access audit and a proposed observation-only Linux experiment.**
This frame defines no estimator and accepts no hardware mapping. The completed
source audit below finds the required safe access unavailable in the resolved
stack. No new physical observations were collected for this frame.

### Completed access audit

At `7aa92d1a`, the stack resolves CPAL 0.18.2 and alsa 0.11.0. In CPAL's
`src/host/alsa/mod.rs`, `build_stream_inner` selects a private `TimestampMode`
from the prepare-time timestamp and advertised `LinkSynchronized` capability.
`callback_instant` can instead use `get_htstamp()` before `Running` or with an
invalid trigger origin. `htstamp_elapsed` clamps negative differences to zero.
`StreamTrait::now` can fall back to `Instant` elapsed time since stream
creation after a status-query failure. Exposing the selected enum alone would not expose these fallbacks.
These are wrapper code paths, not proof of the kernel timestamp's physical age.

The [kernel documentation](https://www.kernel.org/doc/html/latest/sound/designs/timestamping.html)
distinguishes DMA, link and analog positions and describes requested timestamp
types falling back. The [ALSA report](https://www.alsa-project.org/alsa-doc/alsa-lib/structsnd__pcm__audio__tstamp__report__t.html)
contains actual type, validity and accuracy availability, with accuracy in
nanoseconds. Capability bits do not attest the source of an individual value.

CPAL exposes neither its PCM handle nor that report. The resolved alsa wrapper
exposes state, system/trigger/audio timestamps and avail/delay, but no typed
public accessor for the audio timestamp report or driver timestamp. Its
`StatusBuilder::audio_htstamp_config` sets a requested type and delay policy;
it does not expose the returned report. The resolved
alsa-sys 0.4.0 declares `snd_pcm_status_get_audio_htstamp_report` and
`snd_pcm_status_get_driver_htstamp`, but as raw FFI; `Status` storage and its
pointer accessor are private. Opening a second handle cannot attest the source
used by the CPAL stream under test.

**Audit result: access unavailable through the resolved safe interfaces.** This
is a static interface result, not `Not supported` for the physical candidate.
It is falsified by identifying a public safe interface in those versions that
returns the required per-observation report for CPAL's own handle. No hardware
uncertainty threshold is needed to test this access question.

### Candidate and implementation boundary

The preferred investigation is diagnostic instrumentation at CPAL's existing
ALSA status-query points, on the handle CPAL actually drives. It would extract
the report and driver timestamp from the **already acquired status**, together
with the actual fallback branch. It must not add a concurrent status-querying
observer, change the requested type, or silently replace CPAL with a bespoke
ALSA stream. A native ALSA collector would be a different backend candidate and
would need its own frame covering start thresholds, parameters and recovery.

There is a concrete observer-effect reason: in
[Linux v6.18's `snd_pcm_status64`](https://github.com/torvalds/linux/blob/v6.18/sound/core/pcm_native.c),
a status request writes the runtime audio timestamp configuration and can
update the hardware pointer; `Compat` clears report validity. This pinned
upstream example is not an audit of the running Fedora 7.2.11 kernel. Before
collection, audit the actual kernel/library path and prove that extraction
adds no query or configuration interleaving. Missing reports in `Compat` mode
remain missing metadata; they must not acquire inferred validity.

The minimum proposed dependency work is two safe, owned-value getters on the
alsa status wrapper, plus diagnostic extraction at CPAL's existing query and
fallback sites. Raw PCM/status pointers must stay private. Any fork must be
pinned and isolated to a standalone development probe, outside the workspace's
shipping dependency graph; the application dependency and old probe stay
unchanged. This is a proposal, not authorization to add that fork or its FFI.
The project maintainer must explicitly approve new `unsafe` getter
implementations before code is written, under the repository's unsafe rule.
No such implementation or approval is claimed here.

The owned-fixture controls below implement the synthetic observability step
independently of backend access. Next, present the exact getter and
instrumentation scope for the maintainer's implementation decision. The
interface audit is complete; do not repeat it as a hardware feasibility spike.

### Owned-report software controls — 2026-10-18

[`clock_observability.py`](../../../scripts/clock_observability.py) implements
the observation-only preflight. Its closed version-1 JSON artifact has exactly
`version` and `reports`; each report requires every `ClockReport` field. Null
records missing metadata, separately from validity flags. Unknown, missing,
duplicate, non-finite and invalid typed fields are rejected. A backend extractor
must report the actual status-query branch and validity flags; a requested
timestamp type or advertised capability cannot fill an observed field.

Reports retain one duplex arm, direction-local contiguous sequences starting at
zero, requested/actual clocks and timestamp types, rate, running/status/report/
accuracy/trigger validity, explicit fallback and negative-clamp flags, system
observer brackets, status-system/audio/driver/trigger timestamps, accuracy, independently
justified clock-age and frame-alignment bounds, frame position/derivation, and
loss/error counters. Recovery starts a separate artifact; no startup prefix is
discarded. The extractor must derive frame position from the actual queried
hardware position, never the software callback count. This preflight checks
availability and consistency, not the truth of an extractor's physical claims.
Audio time may be trigger-relative while the trigger itself is absolute: their
raw numbers are not compared across origins. Trigger validity is explicit.
Clock names and audio timestamp types use the exact enum variant spellings in
alsa 0.11.0 (`TstampType` and `AudioTstampType`). Unknown spellings are invalid
input. Frame derivation is `status-hardware-position`, `synthetic-frame-counter`
or `callback-count`; both non-hardware derivations are explicitly inconclusive.
Owned positive controls model a hardware-position report; no fixture is a
physical observation.

`Observable` means both directions supplied at least two advancing complete
reports. Fallback, unknown accuracy or missing metadata is `Inconclusive`.
Invalid trigger origins, failed status queries, non-running streams, losses, clamping, clock reversals, repeated values,
sequence gaps and source/rate/origin changes stop the candidate. Stop dominates
missing metadata. Actual `Compat` cannot become observable by requesting it.
Last known values survive null gaps, so missing metadata cannot hide a reversal
or source change. Brackets are ordered within each direction; cross-direction
query order is unspecified and supplies no common-clock proof.
These are readiness classifications, not EVD-0016 physical verdicts. Accuracy
never substitutes for clock age or frame correspondence; even arbitrarily
large known bounds can be observable without passing F4.

Run `python3 -B -m unittest scripts.test_clock_observability` for owned-fixture
controls, which also run in the fast documentation gate. For a future extracted
artifact use `python3 -B scripts/clock_observability.py <artifact.json>`; exit 0
means observable, 1 means inconclusive/stopped and 2 means invalid input. Every
valid output explicitly reports `physical_qualification: false`. No backend
fork, new unsafe getter, production estimator or physical result is introduced.

### Synthetic affine interval investigation — 2026-10-18

`scripts/clock_interval_fit.py` investigates one candidate entirely with owned
synthetic scenarios. It has no report loader, backend access, physical input
mode or production consumer. Its output always identifies synthetic data and
`physical_qualification: false`. Run it directly for the four-rate summary;
`python3 -B -m unittest scripts.test_clock_interval_fit` runs its controls,
also required separately by the fast documentation gate.

The candidate assumes an affine frame position in one observer-clock axis,
with a predeclared strictly positive nominal-rate/ppm slope interval. Every
uncertainty is a hard bound, never a percentile or measured maximum treated as
a guarantee. A report supplies the observer interval `[before - age, after]`
and a floored position `p` whose frame interval is
`[p - alignment, p + 1 + alignment]`. The closed upper endpoint conservatively
includes the floor boundary. Frame alignment stays in frames, not converted
through an unknown slope. No report-clock timestamp is added to the observer
axis; its bridge remains an independent measurement obligation.

Intersect the resulting linear half-planes in slope/intercept space using
exact rational arithmetic. Re-center time at the first report's earliest
possible observer time. Compute clipped corners from original boundary-line
pairs. Prediction evaluates the surviving corners over the event-time interval
and rounds outward to integer frames. Keep every startup constraint. The
bounded experiment accepts at most 128 reports and uses only reports available
at or before its declared decision time and predicts only at or after that decision;
both midpoint negative controls use the
same report prefix. Missing bounds, changed arm/source/clock, raw ordering or
position reversals, a missing startup sequence, an empty model, a pre-start
event or an exceeded declared horizon refuses prediction. Infeasibility
invalidates the scenario; callers must discard every earlier conditional
prediction. Returned interval copies do not self-revoke. It never silently
drops a report and resumes.

Coverage controls use four rates, both slope-prior extremes, zero drift,
one-sided maximum age and same-sign frame alignment, quantization boundaries,
causal horizons, nested constraints and growing event uncertainty. In the
fixed 48 kHz summary scenario, true drift is +100 ppm inside a +/-200 ppm prior,
12 reports are 250 ms apart, maximum age is 200 us, query brackets are 10 us,
frame alignment is three frames and prediction is one second after the last
report. Its conservative integer interval is 29 frames wide and covers truth.
Nominal-slope and free-slope midpoint/max-residual controls both exclude truth
under that same one-sided pattern. This is a sensitivity/control result, not a
hardware precision measurement or a like-for-like comparison with F4's
percentile-based sum. No duplex bound or Q-threshold support follows from it.

A nonempty polygon does not validate the model or its hard bounds. Explicit
counterexamples retain a nonempty model while excluding truth after one
under-declared report bound, an unbridged clock offset, curvature not yet spanned by a report,
a forward jump or drift just outside the prior over a short prefix. In the
controls, three reports whose declared bounds admit no common line refuse,
even though the true instantaneous slopes remain inside the prior. A wrong
nominal rate refuses over the tested span. These failures block
using feasibility as a reset detector or as evidence that a chosen ppm bound
is sufficient. Curvature-aware fitting and outlier-tolerant statistical
methods are not implemented or selected. A mathematically permitted repeated
position also does not relax the separate observability preflight's stop rule.

The investigation therefore does not select a replacement estimator. Physical
preparation still requires independently justified age, frame/timestamp
alignment and pointer granularity, slope/curvature limits, observer/report and
adapter clock bridges, capture/playback position semantics and link/converter/
analog offset. Loopback must measure the latter; no affine fit can identify an
unobserved constant offset. The existing collector/analyzer and retained F4
results remain unchanged. New backend unsafe access still requires the
maintainer's explicit approval, and the actual physical collection method
still requires its independent review before collection.

### Later measurement requirements and falsifiers

Metadata access alone cannot solve the timing problem. In the retained
2026-10-17 result, removing each direction's 256-frame freshness contribution
would still leave 63 input plus 67 output frames: 130 against strict `Q = 64`.
That arithmetic is not a valid revised bound or a new measurement. A new
estimator must independently justify the frame correspondence, observer bridge,
fit residual, bracket duration, clock age and rounding. A common kernel clock
might permit a different bridge, but equality of clock domains and uncertainty
must be demonstrated, not assumed from similar timestamp values.

A future collection method must be independently reviewed before collection,
with predeclared counts, cadence, retention, estimator and uncertainty terms.
Retain both directions, requested/actual clock and type, state, trigger origin,
system/audio/driver timestamps, report and accuracy validity/value, sequence,
loss/backend counters and frame position with its derivation. Specify capture
and playback delay semantics separately. Keep observer brackets and independent
stream origins explicit; software callback counts are not hardware counters.
No startup prefix may be discarded; restart/recovery have separate arms and
new identities/anchors.

The synthetic controls must cover type fallback, missing reports (including
`Compat`), unknown accuracy, stale/repeated values, status failure, non-running
states, invalid trigger origin, zero-clamped negative differences, backward
clocks, missing frame correspondence, losses and source changes. A nominal
complete fixture must remain observable. Unknown metadata is `Inconclusive`;
invalid clocks or broken streams stop the candidate. Observable means only that
required inputs exist, with no precision claim. Silent acceptance of a fallback
blocks physical collection.

A later **valid physical** estimate whose conservative input/output sum is
`>= 64` frames is `Not supported` under F4; one below 64 only passes that
criterion, subject to every other EVD-0016/platform/adapter gate. Missing terms
are `Inconclusive`, never zero. Accuracy is not clock age or frame alignment.
No new result is established here. Old CSVs and the frozen analyzer stay intact;
loopback, adapter references, other platforms and capacity/endurance remain owed.

## Decision

**Deferred to the Phase 9 exit gate**, with the owner and evidence recorded in *The deferral* above.

Three constraints hold in the meantime, so that the deferral cannot be used as permission to improvise:

1. **No implementation may invent a mapping.** Phases 1-8 consume engine-domain `SampleTime`; code that schedules an
   event uses that representation and the anchor ADR-0032 defines, never an unapproved device quantity. Phase 3 may
   exercise deterministic simulated ingress that already supplies `SampleTime`, but that is not hardware evidence.
2. **No production path may consume `AudioCallbackContext::output_latency`, a host timestamp, or `stream_time` before
   this record is accepted.** V1's CPAL adapter may continue reading its callback/playback pair solely to produce the
   currently unread context field. The GUI's `StreamInfo::output_latency` is a separate buffer-size estimate. The
   isolated EVD-0016 probes may observe host values solely to produce this decision's evidence. A new GUI, recording,
   or event-mapping consumer would create an unwritten contract.
3. **An untimestamped adapter still declares its fallback**, per ADR-0032 clause 19, with an explicit `unmeasured`
   marker until this record supplies the number. The declaration is not deferred; only its value is.

## Consequences

### Positive

- Phases 1-8 proceed on the engine-time contract without waiting for unavailable physical-platform evidence.
- The decision will be made against measurements rather than against a plausible story, which is what the register's
  basis for this topic asks for.
- The three constraints above keep the gap honest: nothing accumulates a de facto mapping while the record is open.

### Negative

- Phase 9 cannot exit and no physical live-timing configuration can be qualified until this record is accepted. The
  harness is complete, but retained platform, adapter, and replacement evidence remains mandatory at that boundary.
- Any Phase 1-2 diagnostic that would have wanted a real latency figure gets a declared contributor instead.

### Risks and controls

- **Risk: the remaining platform methods or replacement mapping arrive without
  executable controls**, turning an exit gate into an unverifiable assertion.
  Control: the analyzer self-test executes in the fast documentation gate, and
  the simulator executes in the evidence gate and CI; macOS and Windows are
  rejected until their freshness methods are reviewed, and the record names
  retained platform and adapter artifacts as preconditions.
- **Risk: a mapping accumulates by accident** — a GUI meter reads `output_latency`, then a recording path does, and the
  contract is written afterwards to match. Control: constraint 2; the named evidence probes and V1's unread
  host-timestamp-derived context field publish no mapping to a V2 production consumer. The GUI's distinct buffer-size
  estimate is not a hardware-clock mapping.
- **Risk: the deferral is quietly extended past Phase 9.** Control: the master plan permits deferral only to the
  Phase 9 exit gate; extending it requires changing the plan, in the open.

## Follow-up work

| Task                                                                        | Phase | Status      |
|-----------------------------------------------------------------------------|-------|-------------|
| Build the simulated-host harness (timestamps, drift, block sizes, disconnects) | 9   | Implemented early with executable controls in Active EVD-0016 |
| Measure per-callback host timestamps on Linux, macOS, and Windows            | 9     | Revision-bound Linux direct-PCM data retained; F4 and F7 Not supported in separate runs. macOS/Windows missing |
| Bridge each hardware-timestamped adapter's connection clock                  | 9     | Not started; requires a physical MIDI endpoint on a selected qualification host |
| Measure each untimestamped adapter's arrival-time uncertainty                | 9     | Not started |
| Build synthetic clock-report controls; decide isolated backend instrumentation | 9 | Owned-report controls implemented; new FFI requires maintainer approval before implementation |
| Write and accept this record against that evidence                           | 9     | Required before Phase 9 exit |

## Revisit conditions

This record is not a decision, so it has no revisit condition in the usual sense. It is superseded by its own accepted
version at the Phase 9 exit gate. It would be revisited *earlier* only if a Phase 3-8 task turned out to require a
hardware clock rather than pre-mapped `SampleTime`, which would mean the phase boundary was drawn wrongly and the plan,
not this record, is what needs correcting.
