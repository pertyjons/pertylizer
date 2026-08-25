# EVD-0016: Host-Clock Mapping and Release-Platform Timestamps

| Field | Value |
|---|---|
| ID | EVD-0016 |
| Status | Active |
| Phase | 09 exit evidence; historical `phase-03` path retained for stable links |
| Created | 2026-08-25 |
| Last reviewed | 2026-08-25 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `f3df8b2e` — harness source only; no physical acceptance artifact is claimed |
| Retention | Permanent |
| Related | ADR-0022; ADR-0001 clauses 7, 11, and 16; ADR-0032 clauses 12-14 and 17-22; Phase 9 physical-ingress qualification and exit gate |
| Artifacts | [`evd_0016_host_time.rs`](../../../../crates/synth_engine_v2/examples/evd_0016_host_time.rs), [`evd_0016_cpal_timestamps.rs`](../../../../crates/pertylizer/examples/evd_0016_cpal_timestamps.rs), [`evd_0016_analyse.py`](evd_0016_analyse.py), [`evd_0016_endpoint_policy.tsv`](../../../../crates/pertylizer/examples/evd_0016_endpoint_policy.tsv); retained callback artifacts are still required for the final three-platform result |

## Question and falsifier

**Question.** Which measured host quantities can establish and maintain the
ADR-0032 clause 13 mapping from a hardware timestamp to the current epoch's
`SampleTime`, without making the answer depend on host-block partitioning,
moving an event after it has been stamped, hiding lateness, or claiming more
precision than the host observations support? The same evidence must settle
latency ownership, input alignment, disconnect behavior, and the uncertainty
reported by every initial untimestamped performance-event adapter before
ADR-0022 can become `Accepted`.

This record retains its historical `phase-03` path so existing links remain stable after the phase-boundary
correction. Its simulator is a deterministic control for the host-clock mapper: partition dependence is one way to
falsify that mapper, but the method renders no audio and is not evidence for Phase 3's rendered-output partition gate.
The physical observations and mapping decision belong to Phase 9. Missing macOS, Windows, or adapter hardware
therefore does not block Phase 3; it does block Phase 9 exit and any claim that physical live timing is qualified.

The preferred conclusion is **wrong** if any one of the following occurs:

- **F1 — insensitive harness.** A preparation-only mapper, deliberately given
  +100 parts-per-million clock error for 30 minutes, does not finish at least
  8,000 frames from the simulated device position at 48 kHz. The exact
  expected separation is 8,640 frames; the lower threshold allows only the
  harness's integer rounding. This negative control runs first.
- **F2 — quiet-case error or partition dependence.** With zero drift, zero
  timestamp disturbance, and exactly representable event positions, the
  selected mapper differs by even one frame from truth, or maps any event
  differently under `4096`, `16 x 256`, `64 x 64`, and the declared irregular
  partition of the same 4,096 frames.
- **F3 — uncovered uncertainty.** In any simulated drift, timestamp-noise,
  block-size, or arrival-delay case, an absolute mapping error exceeds the
  uncertainty published with that mapping. A large honest interval may pass
  this safety rule but still fail usefulness under F4.
- **F4 — unusable live precision.** On any release platform, the composed
  input-to-output clock-bridge uncertainty required to map hardware-stamped
  performance input into the current output `SampleTime` is at least one render
  quantum. The probe embeds the V2 `QUANTUM_FRAMES` value in the artifact; it is
  currently 64 frames. Input and output are independently bounded as
  `ceil(callback-fit p99.9 + bridge-fit p99.9 + maximum observer-bracket
  half-width + Stream::now freshness bound + one frame)` and the two integer
  bounds are then added. The final frame in each bound is an explicit
  integer-mapping rounding margin. Too few valid
  observations instead makes the artifact invalid and the record
  `Inconclusive`. This threshold does not assert sample-perfect physical input;
  it rejects a mapping whose admitted uncertainty is too coarse to qualify the
  live-timing boundary before Phase 9 exits.
- **F5 — mutable acceptance.** A calibration update changes the stored
  `(epoch, SampleTime, TimeSource)` of an already accepted event.
- **F6 — invalid or falsely shared stream bridge.** A bridge clock moves
  backwards, its affine fit does not advance, an observer bracket is inverted,
  or the method subtracts raw CPAL `StreamInstant` values belonging to different
  streams. CPAL guarantees a shared origin only within one stream. Input and
  output must each be bridged independently to the observer clock, with the
  complete bracketing interval retained as uncertainty.
- **F7 — invalid timestamp surface.** A release-platform callback clock moves
  backwards; output `playback` precedes `callback`; input `capture` follows
  `callback`; the callback frame count is zero or not divisible by the channel
  count; or a probe record is silently lost. A platform-specific fallback may
  be designed from that result, but the direct candidate is then not supported.
- **F8 — stale epoch.** A simulated disconnect, reconfiguration, or hotplug
  lets an old-epoch timestamp enter the new epoch, reuses an epoch identifier,
  or preserves a calibration across streams. The required result is a terminal
  old-epoch mapping, a strictly newer epoch starting at `SampleTime::ZERO`, and
  an attributable stale-event count.
- **F9 — hidden latency or false compensation.** The reported live output
  latency is not the sum of the per-callback host output latency and ADR-0001's
  constant `Q` contributor, or the mapper moves a causal live event earlier to
  disguise either contributor. Offline remains compensated under ADR-0001;
  live monitoring and recorded-take placement report distinct quantities.
- **F10 — unmeasured fallback.** An initial V2 adapter that can originate a
  `PerformanceEvent` has neither a usable hardware timestamp nor a retained
  paired-reference measurement of its arrival fallback. Merely timing queue
  drain is not enough: the reference must expose the error introduced by
  discarding or lacking the source timestamp.
- **F11 — unbridged hardware-adapter clock.** An adapter labels an event
  `Hardware` without independently bridging that adapter connection's timestamp
  origin to the observer clock used by the audio mapping. A midir timestamp is
  elapsed microseconds from an arbitrary origin that is stable only for one
  `MidiInputConnection`; it is not a CPAL `StreamInstant`. The adapter must
  bridge that clock or use a measured `Arrival` fallback, never subtract the two
  raw values.

The result is `Supported` only if F1 detects its negative control, F2 and F5-F11
do not fire, every simulated error is covered as F3 requires, and every release
platform satisfies F4. It is `Not supported` when a completed valid observation
fires a falsifier. It is `Inconclusive` when no completed observation fires a
falsifier but a required platform, adapter, or control artifact is missing or
invalid. Neither an inconclusive result nor a not-supported candidate can
qualify live timing or close ADR-0022 at the Phase 9 exit gate.

## Inputs and controls

The simulator uses integer nanoseconds and integer frame positions. Its short
matrix crosses 8, 44.1, 48, 96, and 192 kHz with drift of -1,000, -100, -20,
0, +20, +100, and +1,000 ppm; deterministic timestamp disturbance bounded by
0, 20, and 250 microseconds; and `4096`, `16 x 256`, `64 x 64`, and the
predeclared irregular partition. Each short case spans 4,096 frames. A separate
30-minute 48 kHz control crosses both -1,000 and +1,000 ppm, the 250-microsecond
disturbance, and all four partitions. The fixture also covers pre-epoch stamps,
preparation, disconnect and re-preparation, events accepted before a
calibration update, live-latency accounting, and a bounded arrival fallback.
The disturbance pattern is the fixed signed sequence `[-8, 5, -3, 8, -1, 2,
-6, 4] / 8` of the declared bound; there is no random seed. After adding that
disturbance, the simulated host clamps an observation to the preceding observed
callback time when necessary. This models the required monotone timestamp
surface and can only reduce the applied disturbance; the independently declared
uncertainty still uses the full, unclamped bound.

The candidate is not given the fixture's true drift or disturbance. Every
matrix case instead publishes uncertainty from one independent envelope of
±1,500 ppm drift and 500 microseconds of callback-timestamp disturbance. Those
bounds exceed the full ±1,000 ppm / 250-microsecond fixture matrix. The generic
arrival control declares 256 frames before checking delays whose largest fixture
value is 240 frames; mapper rounding adds one further frame to the published
257-frame combined bound.

The release-platform probe is tied to the workspace's `0.18.2` CPAL requirement
and the resolved `Cargo.lock` version, reports the matching embedded version
constant, embeds V2's `QUANTUM_FRAMES`,
and requires explicit input/output device IDs from its `list` command. It
rejects CPAL's structured `DeviceType::Virtual`, the exact ALSA default and
pulse aliases, and IDs or names that identify null, PipeWire, PulseAudio,
virtual, dummy, BlackHole, Soundflower, loopback, Aggregate Device,
Multi-Output Device, VB-Cable, or `cable input` endpoints even when the operator supplies
`--physical`; on Linux it additionally admits only direct ALSA `hw:` PCMs, not
`plughw`, `dmix`, `dsnoop`, `sysdefault`, or other plugins. The endpoint policy
is enforced redundantly by the probe and analyzer against a shared
cross-platform case matrix that includes Linux `snd-aloop`. Their ASCII
case-folding and marker sets are deliberately identical; this is a consistency
control, not an independent semantic review. Negative name matching on every
platform is a conservative rejection policy, not proof that an admitted
endpoint is physical; a Linux `hw:` ID can still name a kernel-virtual device.
The artifact therefore also retains the operator's `--physical`
attestation as provenance; the analyzer checks that provenance field for
schema consistency, but the literal does not prove that a device is physical.
It records the negotiated sample format, rate, channel count,
callback frame counts, and either the queried stream buffer size or the query
error when a backend cannot expose it. Its audio
callbacks rely on CPAL 0.18.2's documented pre-filled-silence guarantee, read
CPAL's supplied timestamp, and attempt one push into a preallocated SPSC ring.
They do not allocate, lock, read a process clock, log, or perform file I/O. An
off-audio-thread collector does all formatting and observer-clock bracketing.
Each bridge retains exactly one `observer-before / Stream::now() /
observer-after` bracket; it does not select the narrowest of repeated samples.
That bracket bounds the duration of the read, not the age of the backend value.
The analyzer therefore requires a separate reviewed freshness bound. CPAL 0.18.2
selects the ALSA mode at `src/host/alsa/mod.rs:406-415`; `CreationInstant` reads
a current process instant, `SystemClock` uses the latest DMA timestamp, and a
running `AudioLink` uses a hardware sample-counter/TSC cross-timestamp, with
DMA-timestamp fallbacks (`:692-707,778-819,1433-1453`). Across those modes, one
negotiated period is a deliberately conservative upper bound: it covers the
DMA case and over-bounds the fresher paths. The Linux direct candidate adds
that period from `Stream::buffer_size()`. No such
source audit is yet accepted for macOS or Windows, so those artifacts remain
invalid rather than assuming zero staleness. Only an artifact with the explicit
synthetic continuous-clock control marker declares zero freshness cost; a
free-form host name cannot license that exemption.
This evidence method is intentionally pinned to CPAL 0.18.2. A later dependency
bump requires updating the analyzer and taking new platform observations rather
than silently relabeling old artifacts. The documentation gate checks that the
workspace requirement, resolved lock version, probe constant, and analyzer
constant agree.

Controls, in execution order:

1. Run F1's deliberately wrong static mapper before any candidate result.
   The Core V2 documentation gate executes the complete simulator, so these
   controls cannot silently rot while their figures remain cited here.
2. Feed the analyzer a synthetic valid single-direction artifact, a valid
   duplex artifact, and nineteen classified mutations:
   a reversed callback timestamp, invalid endpoint direction, reversed endpoint
   timestamp, reversed bridge timestamp, missing sequence, duplicate sequence,
   inconsistent derived frame position, malformed frame count, and internally
   inconsistent error summary, a missing freshness bound, a mismatched
   freshness source, an unmarked synthetic fixture, missing reviewed freshness
   methods on macOS and Windows, a virtual endpoint, a callback target below
   the floor, insufficient callback and bridge populations, and a truncated
   summary row.
   Timestamp, endpoint, bridge, and frame-shape
   failures must be reported as five falsifier hits; both sequence failures,
   derived-position inconsistency, the counter contradiction, the five
   freshness/control-contract failures, and the five remaining artifact
   failures must be fourteen invalid
   artifacts. Separate wide-bracket and wide-fit observations must each
   remain structurally valid and produce F4 `Not supported`. The self-test also
   evaluates every row in the shared Linux/macOS/Windows endpoint-policy matrix
   in both implementations and sends one rejected virtual endpoint through the
   analyzer's complete artifact path. It also tests both failing and complete
   release-platform coverage. It exits zero only after asserting all expected
   classifications and does not publish a deliberately invalid result as a
   retained measurement row. Its synthetic valid artifact is the estimator's
   absent-effect control; it does not replace a controlled release-platform
   fixture run. The single-direction control must remain `Inconclusive`, while
   the duplex control must be `Within F4`. A separate `RealtimeDenied` warning control proves that a denied
   priority request remains reported without being misclassified as a fatal
   stream error.
3. Run F2's exact-clock partition family before drift or noise cases.
4. Run the real probe first without a stream, list the exact host and device
   configuration, then pass explicit non-virtual input/output IDs to `record`.
   A default, server-backed, virtual, or null endpoint does not satisfy a
   release-platform artifact.

Each platform artifact must retain at least 10,000 consecutive output callbacks,
at least 1,010 observer-clock bridges per recorded direction, and, where the
selected device exposes input, 10,000 consecutive input callbacks. A thousand
post-warm-up bridges is the minimum population from which this method permits a
p99.9 bridge residual. If physical input is absent, that platform's input and
arrival-fallback obligations remain explicitly missing rather than being
silently reduced to output-only evidence.

### Linux environment

The diagnostic Linux observation ran on Fedora Linux 44 Workstation, kernel
7.1.9-200.fc44.x86_64, x86_64, on a 20-logical-CPU Intel Core i7-13700H. CPAL's
host was ALSA and the selected device reported the name `sof-hda-dsp`,
`hw:CARD=0,DEV=0`, in a release build. The negotiated callback configuration is
present in the diagnostic artifact. On eligible ALSA streams CPAL's bare
`realtime` feature calls `audio_thread_priority`, but CPAL builds that Linux
helper without its D-Bus feature. The call is therefore a documented no-op in
this dependency configuration. The observer thread was also deliberately not
promoted. A zero `realtime_denied_count` records that the no-op helper returned
success; it does not mean Linux granted real-time priority. System load was not controlled or recorded, so the
run cannot establish a deployment envelope or a universal Linux result. It is
still a valid observation of the direct candidate under the production priority
policy selected by this build; its F4 outcome is not discarded.

## Method

The simulator, analyzer controls, and Linux observation were executed on
2026-08-25. The remaining procedure is unchanged:

1. Run the simulator's negative and quiet controls, then its full matrix. It
   emits one compact CSV row per case and exits non-zero on any contract breach.
2. On Linux, macOS, and Windows, build one release binary from the same source
   revision. List devices, select physical endpoints, and collect callbacks.
   The probe records per-stream callback/capture/playback instants, frame counts,
   ring loss counters, error callbacks, and off-thread bracket samples from
   `Stream::now()` against one process-monotonic observer.
3. Analyze each stream independently. Convert only durations within the stream
   until its bridge is applied; never infer shared raw origins between streams.
   Report monotonicity, timestamp direction, callback-frame distribution,
   callback-period residuals, host input/output latency distributions, drift,
   bridge width, and the resulting frame uncertainty. Per direction, compute
   `ceil(callback-fit p99.9 + bridge-fit p99.9 + maximum bracket half-width +
   reviewed Stream::now freshness bound + 1 frame)`. Add the input and output integer bounds for F4's duplex
   input-to-output result. A single retained direction is `Inconclusive` unless
   its component alone is already at least `Q`, which is sufficient to reject
   the sum. The final frame covers integer conversion and stamp rounding; the
   callback-period residual is diagnostic and is not a second uncertainty term.
   It is computed from the same callback points as the callback fit. The
   source-audited freshness term may overlap the statistical bridge residual,
   but is still added because a p99.9 residual does not prove correlation with
   or coverage of the worst-case backend age; the conservative method does not
   subtract an unproved overlap.
   The affine callback and bridge fits discard exactly their first 10 retained records as startup
   warm-up. Counts in the result table remain retained-record counts. The
   maximum bridge half-width deliberately covers every retained bracket,
   including warm-up, because excluding a wider observed bracket would make the
   uncertainty smaller rather than merely stabilize a fit.
4. Exercise every initial untimestamped `PerformanceEvent` adapter with paired
   reference timestamps. Report the absolute `Arrival - Hardware` frame error
   distribution without using it to move events.
5. For every initial hardware-timestamped adapter, independently bridge its
   connection-local timestamp to the same observer clock used by the audio
   streams and retain the bridge uncertainty.
6. Apply the declared falsifiers, then write ADR-0022 from the observations.
   A missing macOS or Windows artifact is an inconclusive gate, not evidence for
   the other platform.

The first Linux exploration used `alsa:default` through PipeWire. Independent
review correctly rejected that as a physical-device control: routing to a
physical sink/source does not make the CPAL endpoint or its predictions a
direct hardware PCM. The repaired probe therefore requires explicit IDs and the
diagnostic Linux method uses `alsa:hw:CARD=0,DEV=0` for both directions. It
negotiated 48 kHz stereo, 512-frame callbacks, and CPAL's `I24` input / `I32`
output formats.

The first direct-device run sampled one observer bracket per interval and
legitimately fired F4 at 65 input frames and 89 output frames (temporary CSV
SHA-256 `ecab83f2d46c7d8736dff27a621a616893bd745b04a04701d853ea65e7c223ab`).
The method was then changed after seeing that failure to take eight back-to-back
brackets and retain the narrowest; that post-hoc estimator produced 63 input and
62 output frames. Their 125-frame duplex sum was still `Not supported` even
before adding a freshness term. Independent review correctly rejected the
method anyway: increasing the
burst can only reduce a minimum, so the method had no bound against tuning a
future platform through F4. The final method therefore returned to one
unselected bracket. The final Linux result below is a new observation under
that non-selecting method, and the eight-sample result contributes no support.

Callback-period residual is reported as a scheduling-jitter diagnostic but is
not added independently to the mapping uncertainty. It is derived from adjacent
points in the same callback timestamp/frame series whose deviation is already
represented by the callback affine-fit residual; adding both would count the
same timestamp movement twice. The observer bridge fit and retained bracket
half-width are independent terms and are added separately.

Both streams are dropped before their preallocated SPSC rings are drained. The
diagnostic direct-device CSV contains 30,331 lines and approximately 2.16 MB of
callback trace, so the evidence artifact policy excludes it from `plans/v2/`.
Its SHA-256 binds the table below to the temporary bytes used for this review,
but a digest without an exact source revision and stable storage is not retained
acceptance evidence. The record remains `Active` until final-revision runs have
the declared controls and retention on all release platforms, adapter
measurements, and a characterized mapping candidate.

F11 was added by the Active-record self-audit after the first Linux CPAL
observation and before independent review or acceptance. That observation
exposed that CPAL's per-stream clock bridge alone cannot justify mapping
midir's independent per-connection clock. The amendment strengthens the
stopping rule and does not change or discard an observed result.

## Reproduction

```bash
cargo run -p synth_engine_v2 --example evd_0016_host_time --quiet \
  > /tmp/evd_0016_simulator.csv

python3 -B plans/v2/evidence/phase-03/evd_0016_analyse.py --self-test

cargo build --release -p pertylizer --example evd_0016_cpal_timestamps
target/release/examples/evd_0016_cpal_timestamps list
target/release/examples/evd_0016_cpal_timestamps \
  record duplex 10000 --physical \
  --output-device 'alsa:hw:CARD=0,DEV=0' \
  --input-device 'alsa:hw:CARD=0,DEV=0' \
  > /tmp/EVD-0016-linux-direct-cpal.csv

python3 -B plans/v2/evidence/phase-03/evd_0016_analyse.py \
  /tmp/EVD-0016-linux-direct-cpal.csv
# This diagnostic run exits 1 because its observation fires F4.

sha256sum /tmp/evd_0016_simulator.csv \
  /tmp/EVD-0016-linux-direct-cpal.csv

# Before final acceptance, add reviewed macOS and Windows Stream::now freshness
# methods to the probe and analyzer; the current analyzer rejects those platform
# artifacts deliberately. Then supply all three artifacts and use:
python3 -B plans/v2/evidence/phase-03/evd_0016_analyse.py \
  --require-release-platforms <linux.csv> <macos.csv> <windows.csv>
```

The probe command itself must exit zero before its redirected file is eligible
for analysis. On callback-record loss it prints every recorded direction and
summary with nonzero loss counters before exiting nonzero; that complete file is
useful for diagnosis but is not an acceptance artifact. The analyzer
independently rejects nonzero loss even if an operator overlooks the probe's
exit status.

## Results

The provisional simulator CSV from the current uncommitted Active worktree has
SHA-256
`4dfa9f27d15c78cbd4751ed15423f1b36fc116ae16e843305d7497aa238e9b98`.
It is not an acceptance artifact: after the harness has an exact source
revision, it must be rerun and either retained or tied to that revision before
this record can complete.
The documentation gate reruns the simulator and requires its complete control
sequence to exit zero; the workspace test gate independently runs the same
example as a Rust test.
F1 observed the exact predeclared 8,640-frame separation. F2 mapped all 36
quiet-partition observations exactly. The 420-case short matrix made 28,665
observations; its maximum error was 48 frames and its largest published bound
was 104. The eight 30-minute cases made 11,517,204 observations; their maximum
error was 16 frames and their largest published bound was 32. No simulated
error exceeded the independent declared envelope.

F5 admitted the event into the candidate ledger, updated calibration, and read
the same ledger entry back unchanged; its deliberate remap mutation moved the
stamp by four frames, which the identity check detected. F8's stream-replacement
operation rejected and counted the stale stamp, accepted the new epoch's origin,
and reset the calibrated anchor. Its retained-calibration mutation displaced the
new origin by 192,000 frames and was detected. F9 admitted the causal event into
the same ledger while reporting `host + Q` latency; its deliberate
latency-subtraction mutation moved the event 256 frames and was detected. The
four arrival fixtures plus the F9 latency operation made five observations; the
maximum arrival error was 240 frames under the independently declared 257-frame
combined bound. That generic control is not a substitute for F10's per-adapter
measurements.

The analyzer classified all nineteen mutations as declared—five falsifier hits
and fourteen invalid-artifact outcomes—and its valid duplex control, whose input
and output carry distinct raw stream-clock origins, was `Within F4`.
Its separate wide-bracket and wide-fit controls both produced an F4 `Not
supported` result. Its `RealtimeDenied`
warning control remained nonfatal, and both implementations matched every
shared endpoint-policy case. The overall self-test then exited zero, which means
it observed every expected positive and negative control outcome, including the
analyzer's virtual-endpoint rejection and release-platform coverage rule.

The final-method diagnostic direct-device Linux callback CSV has SHA-256
`5894a87422ffb178f40da8242419d20cd8605b7635573d422dba531a65afb458`.
It produced:

| Direction | Retained callbacks | Effective rate | Drift | Callback-fit p99.9 | Period residual p99.9 | Bridge-fit p99.9 | Maximum bridge half-width | `Stream::now` freshness bound | Conservative bridged uncertainty | Endpoint latency p50 / p99.9 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Input | 10,002 | 47,999.994921 Hz | -0.106 ppm | 27.377 frames | 44.814 frames | 35.259 frames | 5.884 frames | 512 frames | **582 frames** | 540 / 556 frames |
| Output | 10,002 | 47,999.994091 Hz | -0.123 ppm | 25.979 frames | 41.110 frames | 36.858 frames | 25.778 frames | 512 frames | **602 frames** | 496 / 512 frames |

F4 requires the composed duplex uncertainty to remain strictly below `Q = 64`.
The input and output components sum to **1,184 frames**, so the direct Linux
observation fails that requirement. Each direction
retained 5,139 observer bridges. The affine-fit figures exclude the
first 10 callback and bridge records as declared warm-up, while the maximum
bracket term covers every retained bridge. The bridge-fit residual was 35.259
input frames and 36.858 output frames; the maximum bracket half-width was 5.884
input frames and 25.778 output frames. The separately reviewed one-period
freshness bound contributes 512 frames in each direction and dominates both
components. It is not capable of manufacturing this verdict by itself: without
either freshness term the observed components are still `ceil(27.377 + 35.259 +
5.884 + 1) = 70` input frames and `ceil(25.979 + 36.858 + 25.778 + 1) = 90`
output frames, whose 160-frame duplex sum already exceeds `Q`. The composed mapping exceeded F4 in this
run, and the record does not yet characterize a replacement. All callback and
endpoint clocks were monotone, every artifact frame shape and sequence was
internally consistent, and the error, ring-loss, xrun, device-unavailable,
stream-invalidated, route-change, and real-time-denial counts were zero. The
probe's derived `start_frame` continuity checks artifact integrity; it does not
claim to expose a device-side frame discontinuity. Device/host loss instead
comes from CPAL's typed error callback. The analyzer bridges input and output
independently through affine fits to the one observer clock; it never subtracts
their raw `StreamInstant` values.

CPAL exposes `StreamInstant` but not the ALSA backend's selected
`CreationInstant`, `SystemClock`, or `AudioLink` mode; the artifact records that
selection as unobservable through CPAL 0.18.2's public API. The pinned backend
source nevertheless defines the selection conditions: a zero prepare-time
hardware timestamp selects `CreationInstant`, otherwise link-synchronized
timestamp support selects `AudioLink`, and the remainder selects `SystemClock`.
The first condition can be discriminated externally, while CPAL does not expose
the second capability or the selected enum. On input, the 35.259-frame bridge-fit
residual far exceeded the 5.884-frame maximum observer-bracket half-width, so
read duration does not explain the dominant variation. Output additionally had
a 25.778-frame worst observer-bracket half-width and a 36.858-frame bridge-fit
residual. The observation therefore cannot treat `Stream::now()` as a
high-precision colocated observer-clock read, but it does not identify which
private ALSA mode was active. A replacement may mirror and validate the pinned
selection conditions, expose the selected mode, or calibrate a black-box stream
clock while bounding its observed quantization and staleness; this direct
affine observer bridge does none of those.

The analyzer's outcome for this valid diagnostic observation is `Not supported`
under F4. It rejects the direct candidate under the measured callback-priority
configuration, but it is not a universal result for every Linux endpoint or
load. Its raw trace is not retained and the worktree has no final source
revision, so it cannot be promoted into final acceptance evidence. Callback-fit
uncertainty is material—27.377 input frames and 25.979 output frames—so the
observation also cannot attribute the failure solely to the observer bridge.
macOS and Windows release artifacts are absent, and F10's paired-reference
measurements for the initial untimestamped V2 adapters are missing. F11's
per-connection bridge for hardware-timestamped adapters is also missing; this
Linux host exposes only the virtual `Midi Through` port, not a physical MIDI
device with which to make that observation.

The table is diagnostic and manually transcribed from the temporary artifact;
no repository gate can re-derive it after those bytes disappear. The digest
only permits verification while an identical external copy exists. This is why
the table cannot satisfy acceptance even though the analyzer produced it.

## Limitations

- A simulated clock can establish arithmetic, partition, correction, and epoch
  behavior, but cannot establish a real backend's timestamp semantics or
  precision.
- Three operating-system runs do not characterize every driver or device. The
  accepted contract must therefore publish observed uncertainty and fail or
  degrade explicitly when runtime observations violate the measured envelope.
- CPAL predicts DAC/ADC timing; this method does not measure analog round-trip
  latency without a physical loopback. Host latency and physical converter
  latency remain distinct.
- The rejected PipeWire exploration describes a server-backed graph, not a
  direct hardware PCM, and therefore contributes no physical-device result.
- The diagnostic Linux run fires F4 on the one measured device under the
  callback-priority configuration. Its uncontrolled system load prevents
  generalizing the result to every Linux endpoint or deployment envelope.
- This record was still `Active` when the endpoint policy, rejected eight-sample
  estimator, and F11 clock-bridge criterion were added. Every final retained
  platform and adapter artifact must be collected after those criteria and use
  the same final method; earlier exploratory files cannot be promoted later.

## Conclusion

`Not supported` for the direct candidate under the measured Linux
callback-priority configuration: the diagnostic observation fires F4 at 582
input and 602 output frames, with a 1,184-frame duplex sum. This is not a universal Linux result, and the wider
record remains incomplete because final-revision retained platform observations,
per-adapter arrival measurements, hardware-clock bridges, and a replacement
mapping are absent. ADR-0022 remains `Deferred`; this does not block the active
Phase 3 scheduler, but it does block Phase 9 exit and every physical live-timing
qualification.
