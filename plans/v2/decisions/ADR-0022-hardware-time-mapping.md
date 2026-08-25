# ADR-0022: Hardware Time Mapping and Latency Ownership

| Field         | Value                                                        |
|---------------|--------------------------------------------------------------|
| ID            | ADR-0022                                                     |
| Status        | Deferred                                                     |
| Phase         | 0A/9, deferred to the Phase 9 exit gate                       |
| Created       | 2026-08-13                                                   |
| Last reviewed | 2026-08-25                                                   |
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
was uncontrolled, the raw trace is not retained, and the worktree has no final source revision. Final-revision Linux,
macOS, and Windows callback artifacts remain missing, as does the paired-reference arrival measurement for every
initial untimestamped V2 adapter. The Active-record self-audit also made
explicit that every hardware-timestamped adapter needs its own connection-clock bridge; a midir timestamp and a CPAL
`StreamInstant` do not share a raw origin. No replacement is yet characterized, and the diagnostic observation is not
a basis for accepting this decision.

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
  Control: the simulator and analyzer now execute in the documentation gate;
  macOS and Windows are rejected until their freshness methods are reviewed,
  and the record names retained platform and adapter artifacts as preconditions.
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
| Measure per-callback host timestamps on Linux, macOS, and Windows            | 9     | Diagnostic direct-PCM Linux run observed; final-revision retained artifacts missing |
| Bridge each hardware-timestamped adapter's connection clock                  | 9     | Not started; no physical MIDI endpoint is attached to the Linux host |
| Measure each untimestamped adapter's arrival-time uncertainty                | 9     | Not started |
| Write and accept this record against that evidence                           | 9     | Required before Phase 9 exit |

## Revisit conditions

This record is not a decision, so it has no revisit condition in the usual sense. It is superseded by its own accepted
version at the Phase 9 exit gate. It would be revisited *earlier* only if a Phase 3-8 task turned out to require a
hardware clock rather than pre-mapped `SampleTime`, which would mean the phase boundary was drawn wrongly and the plan,
not this record, is what needs correcting.
