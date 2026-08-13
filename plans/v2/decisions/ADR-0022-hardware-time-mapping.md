# ADR-0022: Hardware Time Mapping and Latency Ownership

| Field         | Value                                                        |
|---------------|--------------------------------------------------------------|
| ID            | ADR-0022                                                     |
| Status        | Deferred                                                     |
| Phase         | 0A, deferred to the Phase 3 entry gate                       |
| Created       | 2026-08-13                                                   |
| Last reviewed | 2026-08-13                                                   |
| Related       | ADR-0001, ADR-0032, ADR-0021, ADR-0036, ADR-0023, P00A-T006  |
| Supersedes    | —                                                            |
| Superseded by | —                                                            |

**Class.** `Contract`. It decides ownership boundaries and error behavior, not a value.

## The deferral

| Field                | Value                                                                                   |
|----------------------|-----------------------------------------------------------------------------------------|
| Deferred to          | The **Phase 3 entry gate**. Phase 3 implementation may not begin before this is `Accepted` |
| Owner                | Project maintainer — this is a single-maintainer repository, so there is no second party to assign |
| Evidence required    | A simulated-host harness with controllable timestamps, drift, block sizes, and disconnects; measured per-callback timestamps on the three release platforms; a measured arrival-time uncertainty per untimestamped adapter |
| Why not now          | The evidence does not exist and cannot be produced in Phase 0A, and nothing before Phase 3 maps a hardware clock |
| What makes it safe   | ADR-0032 already fixed the *shape* the mapping must produce, so Phases 1-2 cannot bake in a conflicting assumption |

The Phase 0A exit gate accepts this record as `Deferred` on those four fields. It does not accept silence, and it does
not accept a deferral without a named later gate: the master plan permits deferral **only** to the Phase 3 entry gate.

## Context

Something must state how a device or host clock relates to the engine's own time, and who pays for the latency the
engine declares. Neither question is answerable from V1, because V1 does not attempt either — and, more usefully, it is
now clear that V1 *has the raw materials on both ends and no consumer between them*. Read at `e4873d0b`:

- **The output side already measures true latency and throws it away.** The cpal backend takes the host's per-callback
  timestamps and computes the real gap between callback and playback
  (`ts.playback.duration_since(ts.callback)`, `cpal_backend.rs:329-334`), falling back to the static estimate before
  the clock warms up. It passes the result as `AudioCallbackContext.output_latency`. `SynthEngine::process` reads
  `sample_rate`, `frames`, and `channels` from that context and nothing else, so the measurement reaches no consumer.
- **`stream_time` is a wall clock, not a device clock.** It is `start_time.elapsed()` from an `Instant`
  (`cpal_backend.rs:341`), which is a different quantity from the stream position beside it and cannot serve as a
  mapping basis.
- **The input side discards its timestamps.** The MIDI callback receives the driver's timestamp and ignores it
  (`io/midi.rs:247`), so a live event's only time is when the audio thread happened to drain the queue.
- **Nothing estimates drift, calibrates an anchor, or compensates anything.** There is no code to review for a
  candidate policy, only the absence of one.

**What this record will decide**, at the Phase 3 entry gate:

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

- Phase 3's exit gate requires that "equivalent timestamped hardware/live and precompiled event streams reach the same
  sample offsets after ingress mapping". That is a statement about this mapping, and it cannot be evaluated without one.
- A mapping decided without measurement would be a guess recorded as a contract, and the repository's own rule is that
  real-time and correctness claims require a reproducible `EVD` record or a named test.
- V1's own numbers are not a baseline here: it has no mapping to measure. The evidence must come from a simulated host,
  which is Phase 3 work in the master plan's testing section.
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
handling. Phase 3's ingress-equivalence gate is unachievable, and live timing keeps the block-boundary quantization of
up to 21.3 ms at 48 kHz that ADR-0032 recorded.

## Evidence

- Source reads at `e4873d0b`: `crates/pertylizer/src/audio/backends/cpal_backend.rs:329-342`,
  `crates/pertylizer/src/io/midi.rs:247`, `crates/synth_engine/src/synth_engine.rs:4084-4094`,
  `crates/synth_core/src/audio/types.rs:213-226`.
- [ADR-0032](ADR-0032-sample-time-and-event-timestamps.md) clauses 13, 17-19, and 22, which name the obligations this
  record inherits.
- [ADR-0001](ADR-0001-internal-render-quantum.md) clauses 7 and 16.

**The evidence that is missing** is the whole basis of the decision: nothing in this project has yet measured a host
timestamp, a clock drift, or an arrival-time uncertainty. The register's stated basis for this topic is
`Simulated-host evidence`, and no simulated host exists.

## Decision

**Deferred to the Phase 3 entry gate**, with the owner and evidence recorded in *The deferral* above.

Three constraints hold in the meantime, so that the deferral cannot be used as permission to improvise:

1. **No implementation may invent a mapping.** Phases 1 and 2 are offline; a hardware clock has no place in them. Code
   that needs a time uses `SampleTime` and the anchor ADR-0032 defines, never a device quantity.
2. **No path may consume `output_latency`, a host timestamp, or `stream_time` before this record is accepted.** Reading
   one is what would create an unwritten mapping.
3. **An untimestamped adapter still declares its fallback**, per ADR-0032 clause 19, with an explicit `unmeasured`
   marker until this record supplies the number. The declaration is not deferred; only its value is.

## Consequences

### Positive

- Phase 1 and Phase 2 proceed without waiting for evidence that Phase 3 produces anyway.
- The decision will be made against measurements rather than against a plausible story, which is what the register's
  basis for this topic asks for.
- The three constraints above keep the gap honest: nothing accumulates a de facto mapping while the record is open.

### Negative

- Phase 3 cannot start until both the harness exists and this record is accepted, so the harness is on the critical
  path rather than beside it.
- Any Phase 1-2 diagnostic that would have wanted a real latency figure gets a declared contributor instead.

### Risks and controls

- **Risk: Phase 3 arrives and the harness has not been built**, turning an entry gate into a stall. Control: the
  simulated-host harness is Phase 3's own work item in the master plan, and this record names it as the gate's
  precondition rather than as a background wish.
- **Risk: a mapping accumulates by accident** — a GUI meter reads `output_latency`, then a recording path does, and the
  contract is written afterwards to match. Control: constraint 2, and the fact that the field currently has no reader
  to imitate.
- **Risk: the deferral is quietly extended past Phase 3.** Control: the master plan permits deferral only to the
  Phase 3 entry gate; extending it requires changing the plan, in the open.

## Follow-up work

| Task                                                                        | Phase | Status      |
|-----------------------------------------------------------------------------|-------|-------------|
| Build the simulated-host harness (timestamps, drift, block sizes, disconnects) | 3   | Not started |
| Measure per-callback host timestamps on Linux, macOS, and Windows            | 3     | Not started |
| Measure each untimestamped adapter's arrival-time uncertainty                | 3     | Not started |
| Write and accept this record against that evidence                           | 3     | Not started |

## Revisit conditions

This record is not a decision, so it has no revisit condition in the usual sense. It is superseded by its own accepted
version at the Phase 3 entry gate. It would be revisited *earlier* only if a Phase 1 or Phase 2 task turned out to
require a hardware clock, which would mean the phase boundary was drawn wrongly and the plan, not this record, is what
needs correcting.
