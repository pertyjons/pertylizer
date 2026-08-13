# ADR-0032: Sample-Time and Event-Timestamp Model

| Field         | Value                                                        |
|---------------|--------------------------------------------------------------|
| ID            | ADR-0032                                                     |
| Status        | Accepted                                                     |
| Phase         | 0A                                                           |
| Created       | 2026-08-13                                                   |
| Last reviewed | 2026-08-13                                                   |
| Related       | ADR-0001, ADR-0021, ADR-0037, ADR-0003, ADR-0022, ADR-0023, P00A-T006 |
| Supersedes    | —                                                            |
| Superseded by | —                                                            |

**Class.** `Contract`. It fails [reversibility test](../ADR.md#the-reversibility-test) 1 — it defines types, an epoch,
an ownership boundary, and an error behavior, not a value — and it fails test 3, because changing the width or the epoch
of a timestamp later is a change to every event producer and consumer, not a rebuild. The one *value* it names, the
width of the quantum-local offset, is deliberately chosen so that it does not have to be revisited when ADR-0037's
provisional `Q` changes; see clause 4.

**Review history.** One review pass ran against the draft, reading it back against ADR-0001, ADR-0021, the master
plan's Phase 3 gate, and the source cited below. It found five defects, each fixed in the clause that carried it:

- The offline path had no stated relationship between engine time and plan position, so ADR-0001 clause 9's trim could
  be re-derived incorrectly by any consumer. Clause 11 now names plan position as a third quantity.
- The event envelope demanded an epoch field on every event, including ones the renderer generates for the quantum it
  is rendering, where staleness is impossible. Clause 17 now scopes the field to events that cross a queue.
- The rounding clause fixed *where* musical time is rounded but not *how deterministically*: a tempo ramp evaluated
  through a platform libm would still place a note on different frames on different targets. Clause 15 adds the
  constraint.
- Clause 22 clamped an underflowing hardware timestamp to zero without saying what happens next, which would have
  become a second late-event policy competing with ADR-0001 clause 16. It now defers to that clause.
- Two range claims cited monotonicity where they meant the range analysis. Corrected in clauses 3 and 8.

That is one pass by the record's author, not the three independent passes ADR-0001 and ADR-0021 received. The record is
`Accepted` because the Phase 0A exit gate requires it and because nothing outstanding was found; a later review that
finds a defect should withdraw the acceptance the same way ADR-0001's was withdrawn, rather than editing an accepted
record.

**Scope note.** This record fixes **how time is represented and where it starts**.
[ADR-0001](ADR-0001-internal-render-quantum.md) already fixed *which quantum an event belongs to*, *what happens to a
late one*, and *which epoch `SampleTime` counts in*; this record does not restate those clauses, it supplies the types
they are written in and the rules that make them implementable. It does not choose the mechanism that removes the
`Q - 1` control-response delay — see clause 24.

## Context

Phase 3 introduces a sample-accurate scheduler. Before any of it can be written, three things must be fixed: what an
absolute time *is*, where it starts counting, and what a live event carries. The master plan
([Phase 3 work](../master-plan.md#phase-3-sample-accurate-scheduler-and-block-partition-invariance)) names
`SampleTime` and a quantum-local offset as newtypes to introduce, and Part VII topic 32 states the decision as *width,
origin, and overflow behavior*.

V1 cannot supply an answer by example, because it does not have one clock — it has four, none of them shared, and none
of them reaching the events. Read from source at `7e361271`:

- **The output stream has a frame counter that nothing consumes.** The cpal backend counts delivered output frames and
  passes them as `AudioCallbackContext.sample_position`
  (`crates/pertylizer/src/audio/backends/cpal_backend.rs:342`), alongside `stream_time: f64` taken from an `Instant` and
  a per-callback `output_latency`. `SynthEngine::process` reads `sample_rate`, `frames`, and `channels` from that
  context and **nothing else** — the position, the stream time, and the measured output latency are all discarded.
- **Two offline paths construct a sentinel for that unread field.** Both `arrangement_render.rs:602` and
  `preview.rs:347` pass `sample_position: u64::MAX` for their warm-up block, each with a comment explaining that the
  sentinel "keeps the engine from seeing a duplicate position 0". No consumer exists, so the sentinel is inert. It is
  harmless today and is recorded here as the clearest available evidence of the actual state: V1 has a stream-position
  field in its callback contract, two call sites that reason carefully about it, and no reader.
- **The transport's sample position is a playhead, not a clock.** `TransportState.position_samples` is advanced per
  block (`state.rs:352`) and **zeroed by `reset()`** (`state.rs:368`) on stop. It measures elapsed playback, so it moves
  backwards on seek and stands still while the stream keeps running.
- **Each instrument has its own voice-age clock.** `VoiceAllocator.time` advances once per instrument process call
  (`instrument.rs:1533`), and every voice started during a block gets the same start time
  (`voice_allocator.rs:530,554`). Voice age — the input to the `Oldest` stealing strategy — is therefore
  block-quantized and per-instrument.
- **The only sub-block-accurate time in V1 is in ticks, not samples.** `SequencerEngine` keeps an `f64`
  `tick_accumulator` and chunks each block at tick boundaries (`sequencer_engine.rs:582-604`). Its resolution is one
  tick of 960 PPQN — 25 frames at 48 kHz and 120 BPM, 100 frames at 192 kHz — and it is reset on play, stop, and seek.
- **Live input has no timestamp at all.** The MIDI callback receives the driver's timestamp and discards it:
  `move |_timestamp, message, _|` (`crates/pertylizer/src/io/midi.rs:247`). The parsed command goes into a queue that
  `SynthEngine::process` drains in full at the top of the block (`synth_engine.rs:4094`), so a live note starts at the
  first sample of the next block — up to `BufferSize::VERY_LARGE` = 1024 frames, 21.3 ms at 48 kHz (`LIMIT-0057`).
  Recording then anchors the captured note to the tick the block boundary happened to fall in
  (`recording.rs:241,259`), so the quantization is inherited by the saved project.

The existing time newtypes do not fit either, which is why this record introduces new ones rather than reusing them:

- `SampleCount(usize)` (`synth_core/src/types/samples.rs:17`) is a *duration* in a platform-dependent width.
- `SamplePosition(u64)` (`samples.rs:140`) is the closest existing type, but its documented purpose is "position in a
  buffer or stream", its only reduction is `wrap(buffer_size) -> usize`, and it is in production use as the voice-age
  clock described above.
- `SampleOffset(f32)` (`synth_core/src/types/geometry.rs:158`) is a **fractional offset for interpolated delays**. The
  master plan's Phase 3 work list asks for a quantum-local newtype under exactly this name; the name is taken, in the
  same crate, by an unrelated float. Clause 4 renames the V2 type and syncs the plan.
- `Tick(pub u64)` (`synth_sequencer/src/time.rs:28`) is musical time at 960 PPQN, with a public field, and is far
  coarser than a sample.

**Outside this decision.** The quantum's semantics and its frame count (ADR-0001, ADR-0037); the tempo map's
musical-to-sample conversion law and ramp semantics (Phase 3); hardware clock calibration, drift, and who compensates
latency (ADR-0022); ordering of several events at the same sample (ADR-0023); the node-facing event-span API
(ADR-0003); and what a saved project stores, which stays musical time (Project Format V2).

## Decision drivers

- Phase 3's exit gate requires that "equivalent timestamped hardware/live and precompiled event streams reach the same
  sample offsets after ingress mapping". Two streams cannot be shown equal unless both carry a comparable timestamp in
  one epoch.
- ADR-0001 clause 11 already fixed the epoch semantically — sample `S` is the `S`-th input frame consumed — and clause
  16 already fixed the late-event rule. Both are unimplementable without a type that can express `S` and a subtraction
  that cannot wrap.
- Determinism: the same project and the same event stream must produce the same output on every platform and every host
  partitioning. A time that is rounded at more than one place, or in a width that differs by target, cannot promise it.
- The repository's own rules: newtypes validate their invariant and expose only meaningful arithmetic; floats are not
  identity or ordering keys; conversions at boundaries use `TryFrom` rather than `as`.
- Nothing may be sized or tuned to ADR-0037's provisional `Q` = 64 before its Phase 2 re-measurement.
- Real-time safety: an event is a fixed-size value copied through a lock-free queue. Every field this record adds costs
  bytes in that queue and must earn them.

## Options considered

### Option A: `u64` frame index in the engine input epoch, with a separate quantum-local offset

Absolute time is an integer count of input frames since stream preparation. Event scheduling derives the quantum index
and the offset within it by division. Exact, cheap to compare and hash, orders totally, and needs no rounding after the
one conversion that produces it. Costs 8 bytes per timestamp and a second type for differences, because `u64`
subtraction underflows instead of going negative.

### Option B: `f64` seconds (or `f64` frames) as the universal time

Attractive because tempo mathematics is already in `f64` and because a seconds-valued time is host-agnostic. Precision
is not the objection: `f64` seconds keeps sub-sample resolution for centuries, and `f64` frames is exact to 2^53. The
objection is that a float invites the seconds-to-frame rounding to happen more than once, at different places, with
results that differ by a frame; that it is a poor identity and ordering key, which the repository forbids for exactly
this reason; and that a digest computed over event placement then depends on rounding rather than on the plan.

### Option C: `(quantum index, offset)` pair as the canonical timestamp

Removes the division at scheduling time. Rejected: it bakes `Q` into every timestamp in flight, in every fixture, and
in every test expectation — and ADR-0037 explicitly keeps the right to change `Q` at the Phase 2 gate. It also makes
arithmetic across quanta a two-field operation for the sake of an integer division that is not on any measured hot
path. Deriving the pair at scheduling, from an absolute time, costs nothing that has been shown to matter and keeps `Q`
out of the data.

### Option D: `u32` frames, or `usize` frames

Half the bytes, and 32 bits is enough for a session — until it is not. A `u32` frame counter exhausts after 24.9 hours
at 48 kHz and **6.2 hours at 192 kHz**, the engine's own ceiling (`DeviceSampleRate::MAX_SUPPORTED`,
`audio/types.rs:23`). A `usize` counter is the same defect with a target-dependent trigger: 64-bit today on all three
release platforms, and a 6.2-hour clock the day a 32-bit target is added. ADR-0031 has not fixed the build matrix, so
that is not a hypothesis anybody has ruled out. A long-running instrument, an installation, or a headless render service
is not an exotic case, and a wrapped clock is a silent timing corruption rather than an error.

### Status quo

Keep V1's arrangement: an unread stream position, a transport playhead that resets, per-instrument voice-age counters,
and untimestamped live events applied at block boundaries. Phase 3's ingress-equivalence and partition-invariance gates
are unachievable, live and recorded timing keeps its up-to-21 ms block quantization, and the corpus cannot distinguish
an intended timing change from a scheduling defect.

## Evidence

- Source reads at `7e361271`, cited inline above: `cpal_backend.rs:342`, `synth_engine.rs:4094`,
  `arrangement_render.rs:602`, `preview.rs:347`, `state.rs:352,368`, `instrument.rs:1533`,
  `voice_allocator.rs:445,530,554`, `sequencer_engine.rs:582-604`, `io/midi.rs:247`, `recording.rs:241,259`,
  `samples.rs:17,140`, `geometry.rs:158`, `sequencer/time.rs:10,28`, `audio/types.rs:23,213-226`.
- `LIMIT-0057` (1024-frame maximum requested buffer) and `LIMIT-0001` (oversized-block behavior) in the
  [resource inventory](../inventories/resource-limits.md).
- **Range analysis.** Exhaustion is the point at which the representation can no longer name the next frame exactly.

  | Representation      | Exact to            | Exhausted at 48 kHz | Exhausted at 192 kHz |
  |---------------------|---------------------|---------------------|----------------------|
  | `f32` frames        | 2^24 frames         | 349 s               | 87 s                 |
  | `u32`/32-bit `usize`| every frame         | 24.9 h              | 6.2 h                |
  | `f64` frames        | 2^53 frames         | ≈ 5 900 years       | ≈ 1 490 years        |
  | `i64` frames        | every frame         | ≈ 6.1 million years | ≈ 1.5 million years  |
  | `u64` frames        | every frame         | ≈ 12 million years  | ≈ 3.0 million years  |

  For scale, one tick at 960 PPQN and 120 BPM is 25 frames at 48 kHz; a 1024-frame block is 21.3 ms. The `f32` row is
  the one that matters in practice: a float frame index stops being sample-exact after **under six minutes** of a
  48 kHz render, which is shorter than a song.
- **External conventions.** VST3 carries `projectTimeSamples` and `continousTimeSamples` as signed 64-bit sample
  counts and event offsets as block-relative integers; CLAP carries `steady_time` as a signed 64-bit sample count and
  `clap_event_header.time` as a `uint32` block-relative frame index. Both are integer frame counts, which is what
  clause 8's boundary conversion has to meet.

**Uncertainty that remains.** No timing has been measured: P00A-T003's timing and memory halves are still open, and
every figure above is arithmetic or a source read rather than an observation. Two consequences. First, the arrival-time
uncertainty that clause 19 requires an untimestamped adapter to declare has no measured value yet — ADR-0022 owns
obtaining it, and until then an adapter declares its fallback and reports it as unmeasured. Second, the byte cost of
the event envelope in clauses 17-20 is asserted from struct layout, not from a profile; the *Revisit conditions*
section records what would reopen it.

## Decision

Accepted. `Q` denotes the render quantum in frames, fixed by [ADR-0037](ADR-0037-render-quantum-value.md).

### Types

1. **`SampleTime(u64)` is the engine's absolute time.** It counts input frames in the epoch of ADR-0001 clause 11:
   sample `S` is the `S`-th input frame the renderer has consumed since the stream was prepared. It is a private-field
   newtype in the V2 crate; it is not `SamplePosition`, which stays V1's voice-age type until V1 retires.
2. **`FrameCount(u64)` is a non-negative duration** — a distance, a length, a horizon. `SampleTime + FrameCount` is a
   `SampleTime`; `FrameCount` is what a quantum, a latency contribution, and a horizon are measured in.
3. **`FrameDelta(i64)` is the difference of two `SampleTime`s**, and is the *only* way to subtract them. Raw `u64`
   subtraction on a time is forbidden: the difference of two times is signed by nature, and a wrapped difference is the
   classic form this defect takes — an event 1 frame early becoming an event 18 quintillion frames late. `FrameDelta`
   also carries drift and latency figures, which ADR-0022 consumes. Its range covers every difference reachable inside
   one epoch by the margin in the range analysis; a difference it cannot represent is a fault, not a wrap.
4. **`QuantumOffset(u16)` is the position within a quantum**, with the invariant `0 <= offset < Q` enforced at
   construction; construction from an out-of-range value fails rather than clamping. The width is `u16`, not `u8`,
   deliberately: `u8` would cap `Q` at 256 and would be sizing a type to ADR-0037's provisional value, which its
   acceptance forbids. A compile-time assertion pins the relationship (`Q <= QuantumOffset::MAX`) so that raising `Q`
   at the Phase 2 gate fails the build rather than truncating silently.
5. **The V2 offset type is named `QuantumOffset`, not `SampleOffset`.** `synth_core::SampleOffset` is an existing
   `f32` fractional offset for interpolated delays; reusing the name in the same workspace would put two unrelated
   meanings one import away from each other. The master plan's Phase 3 work list is updated in the same change as this
   record, per the [documentation authority rule](../README.md#sources-of-truth).
6. **No time type is a float, and no float is converted to one implicitly.** There is no `From<Seconds>` and no
   `From<f32>` for `SampleTime`, `FrameCount`, or `QuantumOffset`. `Seconds(f32)` may describe a duration a user typed;
   it may never carry a position.
7. **`SampleTime` is never persisted.** It is meaningful only within one prepared stream, so no project file, patch,
   manifest, or receipt stores one. Persisted musical time stays `Tick`; a recording take's placement is resolved to
   musical time before it is saved, exactly as V1 does today. A protocol or report that publishes a `SampleTime` must
   publish the epoch identifier of clause 12 with it.
8. **Conversions at the host boundary are checked.** A host's signed 64-bit stream position (VST3, CLAP) converts
   through `TryFrom` after the epoch anchor of clause 13 has been subtracted; a value that cannot be represented is a
   counted ingress fault, not an `as` cast. In the outward direction a `SampleTime` above `i64::MAX` is unreachable by
   the margin in the range analysis and is reported as a fault rather than truncated.

### The clock and its epoch

9. **There is exactly one render clock.** `SampleTime` is owned by the renderer and advanced only by rendering: it
   increases by `Q` per rendered quantum and by nothing else. It is not derived from a host counter, a wall clock, or a
   transport position.
10. **It is monotone and never reset by musical operations.** Play, stop, seek, loop wrap, tempo change, count-in, and
    panic do not move it. This is the property V1's `TransportState.position_samples` does not have, and it is what
    makes a timestamp comparable across a transport operation.
11. **The transport playhead is a separate quantity and a separate type.** Where the timeline is meant, the type is
    musical (`Tick` or its V2 successor); where the clock is meant, the type is `SampleTime`. Neither converts to the
    other without the tempo map, which is Phase 3's.

    **Plan position is a third quantity, and offline rendering is where it shows.** ADR-0001 clause 9 defines an
    offline request for `N` frames starting at plan sample `S` as returning frames whose first sample *is* plan sample
    `S`, which it achieves by discarding the priming head. An offline render is a prepared stream like any other:
    its `SampleTime` still starts at zero at its own preparation, and the request's `S` is an offset applied by the
    render path, not a starting value for the clock. A harness that treats an engine time as a plan position — or the
    reverse — reintroduces exactly the `Q`-frame shift ADR-0001's impulse-alignment test exists to catch.
12. **A prepared stream carries a `StreamEpoch(u32)` identifier**, assigned at preparation and different from the
    previous one. `SampleTime` restarts at zero in each epoch, so a time without its epoch is ambiguous by
    construction. Sample rate, channel layout, and capacity are fixed for the life of an epoch — changing any of them
    is a re-preparation — so a `SampleTime` never has to carry the rate it was taken at.
13. **Preparation records the epoch anchor.** The mapping from the host's clock to `SampleTime` zero is established
    once, at preparation, and is the ingress mapper's input for the whole epoch. Calibrating and correcting that
    mapping — drift, jitter, and the latency ADR-0001 clause 7 declares — is ADR-0022's, not this record's.
14. **A new epoch invalidates every timestamp of the old one.** ADR-0021 already requires a fresh epoch after the
    terminal oversized-callback fault and states that the engine makes no claim that the old input epoch continues.
    This record makes that checkable rather than only stated: see clause 20.

### Rounding and derivation

15. **Musical time is rounded to a frame exactly once.** The tempo map computes in `f64` and produces a `SampleTime` at
    a single, named conversion point, using round-half-away-from-zero. No later stage re-rounds, and no stage carries
    a fractional frame position forward.
    **The law must also be platform-independent.** Phase 3 owns what the conversion *is*, including ramp semantics, but
    it must be expressible in operations whose results are identical on every supported target — the four IEEE-754
    arithmetic operations, comparison, and rounding. A tempo ramp implemented through a transcendental function would
    make the frame a note lands on depend on the platform's libm, which the determinism digest cannot tolerate. Where a
    ramp shape needs more than that, the law states the exact evaluation rather than calling a library function.
16. **The quantum index and offset are derived, never stored.** For a time `t`, the quantum index is
    `t.0 / Q` and the offset is `QuantumOffset(t.0 % Q)`. Events carry absolute `SampleTime`; the scheduler derives the
    pair when it assigns an event to a quantum under ADR-0001 clause 12.

### The live-event envelope

17. **Every event that crosses into the renderer carries `{ epoch: StreamEpoch, time: SampleTime, source: TimeSource }`**
    in addition to its payload. The envelope is `Copy`, fixed-size, and allocation-free, so it crosses the lock-free
    queue under the audio thread's rules. An event the renderer generates for the quantum it is currently rendering
    cannot be stale and does not carry the epoch field; everything arriving through a queue does, including a plan's
    precompiled event list, because a queue is exactly where an event can outlive its epoch.
18. **`TimeSource` is a three-valued provenance tag**: `Hardware` (the timestamp came from a driver or host and was
    mapped through clause 13), `Arrival` (the adapter had no timestamp and the mapper stamped the event on arrival),
    and `Compiled` (the event was generated from the plan and the timeline, where the timestamp is exact by
    construction).
19. **An adapter with no hardware timestamp must declare its arrival-time fallback**, and that declaration reaches the
    structured diagnostics report with its measured uncertainty — or with an explicit "unmeasured" marker until
    ADR-0022's evidence exists. The uncertainty is *reported*, never used to move an event: an adapter may not
    compensate its own unmeasured error. This is the plan's requirement that an untimestamped adapter not pretend to be
    exact. V1's MIDI path is such an adapter today only because it throws the driver's timestamp away; V2's obligation
    is to stop doing that first, and to declare `Arrival` only where the source genuinely has no timestamp.
20. **An event whose epoch does not match the renderer's current epoch is discarded and counted**, never interpreted in
    the new epoch. Without this, an event stamped microseconds before a reprepare is applied against a clock that
    restarted at zero, producing a timing error bounded only by how long the previous stream ran. The counter is a
    field of the structured diagnostics report, alongside ADR-0001's late-event counter and ADR-0021's fault counters.
21. **A timestamp is admitted only inside a bounded horizon.** An event earlier than the current quantum is late and is
    handled by ADR-0001 clause 16 — clamped forward to the first not-yet-rendered quantum boundary and counted. An
    event beyond a forward horizon is rejected and counted rather than queued for an unbounded time. The two horizon
    values are `HostProfile` budgets owned by P00A-T005 under ADR-0021; this record fixes only that they exist, that
    both directions are bounded, and that neither is silent.
22. **Underflow at the epoch start is a late event, not a wrap.** A hardware timestamp that maps before `SampleTime`
    zero — normal in the first callbacks after preparation, when calibration is still settling — produces a negative
    `FrameDelta` from the anchor. It is clamped to `SampleTime` zero and then treated as any other early event under
    ADR-0001 clause 16, counted with the same late counter. It never becomes a large positive `u64`.
23. **A timestamp alone does not define order.** Several events may share one `SampleTime`; their relative order is
    ADR-0023's decision. ADR-0023 may not implement that order by perturbing timestamps — moving an event off its
    declared sample to encode precedence would make the sample position a lie and would break the ingress-equivalence
    gate.

### What this record hands on

24. **The `Q - 1` control-response delay is ADR-0003's to remove, not this record's.** ADR-0001 clause 15 left the
    choice to "ADR-0003 or ADR-0032"; this record declines it, because the remaining question is the node-facing
    mechanism — event-boundary splitting versus segmented control values — and that is ADR-0003's registered topic and
    Phase 3's work. What this record owes it is a representation that permits either, and `QuantumOffset` with an
    absolute `SampleTime` is that: a split point and a segment boundary are both expressible without changing an event.
    ADR-0001's follow-up row for that work is therefore ADR-0003's alone.
25. **The tempo map, the ingress calibration, and the same-sample ordering** are Phase 3, ADR-0022, and ADR-0023
    respectively, each of which now has a fixed representation to build on.

## Consequences

### Positive

- Phase 3's ingress-equivalence gate becomes expressible: a live stream and a compiled stream are the same when their
  `(epoch, time)` pairs match, which is a comparison rather than a listening test.
- Integer time makes event placement independent of rounding, so a render digest measures the plan rather than the
  arithmetic path that produced it.
- The late, the stale, the out-of-horizon, and the arrival-stamped event are each counted and named. None of the four
  is silent, which is the property the Phase 0A exit gate demands of every truncation.
- Separating the clock from the playhead removes a whole class of V1 confusion in which "position" meant one of four
  different things depending on the reader.
- Nothing in the representation is sized by `Q`, so the Phase 2 re-measurement can change it without touching an event,
  a fixture, or a stored value.

### Negative

- Every queued event grows by the epoch and provenance fields — five bytes on top of the timestamp, eight after
  padding. The queue is fixed-capacity, so that is a real budget cost on a path that carries no timestamp at all today.
- Four new types where V1 had one loosely used `u64`, and a subtraction that returns a third type. The ergonomics are
  worse than raw integers, deliberately.
- Every producer must know the current epoch to stamp an event, which the V1 command path does not, so the ingress
  mapper becomes a required component rather than an optimization.
- `SamplePosition` and `SampleOffset` keep their V1 meanings while the V2 types exist alongside them, so the workspace
  carries two vocabularies until V1 retires. Clause 5 keeps them from colliding by name, not from coexisting.
- Round-half-away-from-zero at the tempo-map boundary means a note authored exactly between two frames lands one frame
  later than a truncating implementation would place it. That is a defined choice, not an accident, and it changes
  nothing audible; it is recorded because a V1-to-V2 comparison will see it.

### Risks and controls

- **Risk: a path routes a position through `Seconds(f32)`** and silently loses sample exactness after ~349 s at 48 kHz.
  Control: clause 6 removes the conversion entirely, and a test places an event an hour into a render and asserts the
  exact frame.
- **Risk: the seconds-to-frame rounding happens twice** — once in the tempo map and once in a consumer — putting an
  event one frame off depending on the path. Control: clause 15 names one conversion point; the partition-invariance
  and event-placement tests of ADR-0001's follow-up list fail when a second one appears.
- **Risk: `QuantumOffset` is sized to today's `Q`.** Control: `u16` plus the compile-time assertion of clause 4, so
  raising `Q` is a build failure rather than a truncation.
- **Risk: an event outlives its epoch** across a reprepare and is applied against a clock that restarted. Control:
  clause 20's epoch check and its counter, exercised by a test that prepares, queues an event, prepares again, and asserts
  the event is discarded and counted.
- **Risk: the stale-epoch counter becomes another counter nobody reads**, the failure mode `LIMIT-0013` already has in
  V1. Control: clauses 19-21 place all of them in the structured diagnostics report, the same one ADR-0021 and
  ADR-0001 publish into.
- **Risk: `Arrival` becomes the default** because it always works, and hardware timestamps are never wired up — leaving
  V2 with V1's timing and a tag that documents it. Control: the tag is per-adapter and appears in diagnostics, and
  Phase 3's ingress-equivalence gate cannot pass for an adapter that stamps on arrival.
- **Risk: a `u64` time is subtracted directly** somewhere and wraps. Control: `SampleTime` has no `Sub<SampleTime>`
  returning itself; the only subtraction yields `FrameDelta`, so the wrapping form does not compile.

## Follow-up work

| Task                                                                                        | Phase | Status      |
|---------------------------------------------------------------------------------------------|-------|-------------|
| Rename the plan's Phase 3 quantum-local newtype to `QuantumOffset`                          | 0A    | Complete    |
| Introduce `SampleTime`, `FrameCount`, `FrameDelta`, `QuantumOffset`, `StreamEpoch`          | 1     | Not started |
| Publish the stale-epoch, out-of-horizon, and arrival-stamp counters in the diagnostics report | 1     | Not started |
| Fix the late/forward horizon budgets as `HostProfile` fields                                 | 0A/1  | P00A-T005   |
| Ingress mapper: stamp live events, stop discarding the driver timestamp (`io/midi.rs:247`)   | 3     | Not started |
| Declare and measure each untimestamped adapter's arrival-time uncertainty                    | 3     | ADR-0022    |
| Event-placement test: an event one hour in lands on its exact frame                          | 3     | Not started |
| Epoch-invalidation test: an event queued before a reprepare is discarded and counted         | 3     | Not started |
| Remove the `Q - 1` control-response delay                                                    | 3     | ADR-0003    |

## Revisit conditions

- A supported host cannot express, or cannot be mapped onto, a monotone integer frame clock — for example a clock that
  rewinds on reconfiguration. That is ADR-0022's evidence to produce, and it would revisit clauses 9-13.
- `Q` is raised above `QuantumOffset::MAX` by a superseding ADR-0037, which clause 4 turns into a build failure.
- Measurement shows the per-event epoch field to be a material cost in the event queue, and a drain-at-preparation
  protocol is demonstrated race-free against a producer that is mid-stamp. Clause 17 would then shrink.
- A V2 feature requires a persisted engine-epoch timestamp — recording takes under ADR-0024 are the plausible case —
  which would revisit clause 7 rather than quietly weaken it.
