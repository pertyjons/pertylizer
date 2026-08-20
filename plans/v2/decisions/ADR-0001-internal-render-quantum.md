# ADR-0001: Internal Render Quantum — Semantics and Splitting Contract

| Field         | Value                                               |
|---------------|-----------------------------------------------------|
| ID            | ADR-0001                                            |
| Status        | Accepted                                            |
| Phase         | 0A                                                  |
| Created       | 2026-08-12                                          |
| Last reviewed | 2026-08-12                                          |
| Related       | ADR-0037, P00A-T005, P00A-T006, ADR-0021, ADR-0022  |
| Supersedes    | —                                                   |
| Superseded by | Clauses 12, 14 and 16 by [ADR-0043](ADR-0043-event-deferral-and-late-clamp.md), 2026-08-20. Clause 12's first sentence is restated over render position, clause 14's "declared sample within the quantum" becomes the offset within the quantum that renders the event, and clause 16's late condition is fixed to a single evaluation at first due. **Clause 12's second sentence and every other clause stand.** Implementers read the current [host-profile](../specs/spec-host-profile-and-render-limits.md) and [Sound Core render](../specs/spec-sound-core-render-contract.md) specifications rather than merging these records |

**Scope note.** This record decides how the quantum behaves; **[ADR-0037](ADR-0037-render-quantum-value.md) decides how
many frames it is.** The split exists because only the frame count depends on a measurement that does not exist yet,
while every semantic clause below follows from the partition-invariance requirement and is decidable from V1's code as
read. Both records must be `Accepted` before the Phase 0A exit gate passes; splitting one register topic into two does
not weaken that gate, and the Phase 0A tracker lists both.

**Review history.** An earlier revision of this record was marked `Accepted` while its splitting contract covered only
the *output* side. Review found that a callback shorter than `Q` has neither the audio input nor the live events needed
to render a full quantum, so the contract was incomplete for any plan with live input. The acceptance was withdrawn and
the decision now defines the input carry, the initial fill, the event horizon, and the `SampleTime` epoch. The added
latency figure changed with it, from `Q - 1` to `Q`.

## Context

Phase 1 requires the V2 renderer to accept varying caller block sizes up to a configured maximum and split every such
block into a fixed internal quantum. This ADR fixes that quantum's splitting semantics.

V1 has no internal quantum: it processes whatever block the caller hands it. Four consequences follow, all read from
source at `5cd24de8`:

- **Control rate is a function of the caller's block size.** `voice.rs:1088` computes
  `control_rate = sample_rate / block`, and that value drives the YAMS `cr` variable and every time-based script
  operation (`lag`, `phasor`). The same project therefore evaluates its control scripts at a different rate for every
  buffer size.
- **Offline and live rendering already disagree.** The offline arrangement renderer is hardcoded to 256 frames
  (`audio/arrangement_render.rs:51`), while the live path requests up to `BufferSize::VERY_LARGE` = 1024 (`LIMIT-0057`).
  A project rendered by `render_to_wav` or `pertylizer render` is not evaluated at the control rate the user heard while
  authoring it.
- **The rate is not even constant within one offline render.** The render loop clamps each block with
  `remaining.min(BUFFER_SIZE).min(before_stop)` (`arrangement_render.rs:587`), so a short block is emitted at the stop
  boundary and at the end of the range, and the control rate jumps for exactly those blocks.
- **Events land on block boundaries.** `note_on_block` is a one-block pulse cleared after every control-script consumer
  has read it (`voice.rs:1132-1136`), so note-on granularity today is the caller's block, not a sample.

This is the concrete defect the fixed quantum exists to remove. It is not a hypothesis about V1; it follows directly
from `control_rate` being computed from `samples`.

**Outside this decision.** The quantum's frame count (ADR-0037), the structure and timestamp representation of events
(ADR-0032), the host block ceiling and admission of a plan against a profile (ADR-0021), hardware clock calibration and
who compensates the latency this record declares (ADR-0022), and the control-value interpolation law between evaluation
points (Phase 3).

**Inside it, and originally missed.** Which quantum an event belongs to, whether control evaluation may look ahead
within a quantum, what happens to an event that arrives too late, which epoch `SampleTime` counts in, and how a stream
ends. These are consequences of choosing whole-quantum-only production, not independent questions: the buffering model
is what creates a gap between host time and render time and what leaves audio in a carry buffer at the end, so the
record that creates them must define them. ADR-0003 and ADR-0032 refine the event mechanism and ADR-0022 owns
compensation, but none of them can be handed an undefined base semantics.

## Decision drivers

- Partition invariance is a Phase 3 exit requirement: `1 x 4096`, `16 x 256`, `64 x 64`, and irregular host blocks with
  the same total frames must produce the same output. No quantum that varies with the caller can satisfy it.
- The master plan requires internal control rate to be a function of the fixed quantum, never the host callback size.
- Determinism must not depend on a configuration value that the render digest does not cover.
- A compile-time constant is what permits fixed-size scratch buffers, constant loop bounds, and vectorized kernels with
  no scalar remainder path. A runtime-variable quantum forfeits all three.
- Whatever the frame count turns out to be, the semantics must not have to change with it. Every clause below holds for
  32, 64, or 128 frames alike, which is what makes the split from ADR-0037 clean rather than expedient.

## Options considered

### Option A: Fixed compile-time quantum, whole quanta only, input-aligned

The renderer buffers input and output, consumes exactly `Q` frames of input per quantum, and serves caller blocks of any
size from the output carry. Partition invariance holds by construction: the sequence of quanta is identical regardless
of how the caller chops its requests. Costs a constant `Q` frames of latency and two carry buffers plus their copies.

### Option A′: Whole quanta, output carry only, render-ahead

The variant the withdrawn revision described. The renderer produces whole quanta on demand and serves output from a
carry buffer, with no input carry. For a plan with no live input this works and costs no latency. For any plan with
live audio input it is not implementable: rendering a quantum requires `Q` input frames, and a callback delivering
`N < Q` has not supplied them. It also renders past the point where live events are known, which silently makes those
events late. Rejected as a general contract; it remains the obvious specialization for input-free plans and is named as
follow-up work rather than decided here.

### Option B: Fixed quantum with a partial trailing quantum

The renderer splits a caller block into whole quanta and processes the remainder as a short quantum. No added latency
and no carry buffer. It reintroduces V1's defect in a smaller form: the short quantum evaluates control at a different
rate than a full one, so output again depends on how the caller partitions its requests, and the Phase 3
partition-invariance tests cannot pass.

### Option C: Configurable per `HostProfile`

Lets a constrained host trade control resolution for CPU. It reintroduces the exact defect this ADR removes — render
output becomes a function of a host/config value — unless the quantum is folded into the determinism digest and every
comparison is scoped to a quantum. It also multiplies the partition-invariance test matrix and prevents compiling the
quantum in as a constant.

### Status quo

Process the caller's block with no internal quantum, as V1 does. Partition invariance is unachievable by construction,
the determinism digest is meaningless across buffer sizes, offline and live renders keep disagreeing, and the V1-to-V2
A/B harness has no defensible reference partition to compare against.

## Evidence

- `LIMIT-0001`, `LIMIT-0057` in the [resource inventory](../inventories/resource-limits.md) — the block ceiling, its two
  silent behaviors, and the 1024-frame request cap.
- Source reads at `5cd24de8`: `crates/synth_engine/src/voice.rs:1088` (control rate from block size),
  `voice.rs:1132-1139` (one-block gate pulse; per-block buffer resize),
  `crates/pertylizer/src/audio/arrangement_render.rs:51` and `:587` (offline 256, short boundary blocks),
  `crates/synth_core/src/audio/types.rs:87` (`MAX_BLOCK_SIZE`).

**Uncertainty that remains.** No CPU measurement exists; every value in the resource inventory is read from source. That
uncertainty bears on the frame count and is carried by ADR-0037. None of the clauses below depends on it: they follow
from the partition-invariance requirement and from V1's observed behavior, not from a performance figure.

## Decision

Accepted. Let `Q` denote the quantum in frames, fixed by [ADR-0037](ADR-0037-render-quantum-value.md).

### Quantum

1. **The internal render quantum is a compile-time constant.** It is not a `HostProfile` field, not a `RenderConfig`
   parameter, and not derived from the sample rate. Option C is rejected.
2. **It is identical in live, offline, and test rendering.** No render path may introduce a second value. This is the
   clause `arrangement_render.rs`'s hardcoded 256 would violate.
3. **The renderer never processes a partial quantum.** A caller block that is not a multiple of `Q` never shortens a
   quantum. Option B is rejected.
4. **Control-rate evaluation happens exactly once per quantum**, so the control rate is `sample_rate / Q` Hz. The
   quantum stays constant in frames across sample rates, which means control rate in Hz scales with the sample rate
   rather than the reverse.

### Buffering

5. **The renderer holds an input carry and an output carry**, each sized `maximum_block_size + Q`. On a callback of `N`
   frames it appends `N` input frames to the input carry; while the input carry holds at least `Q` frames and the
   output carry holds fewer than `N`, it consumes exactly `Q` input frames, renders one quantum, and appends `Q` frames
   to the output carry; it then serves `N` frames from the output carry. Both carries are preallocated at preparation;
   neither is resized on the audio thread.
6. **The output carry is primed with `Q` frames of silence at stream start.** This initial fill is what makes clause 5's
   loop serve any `N`, including `N < Q` and any irregular sequence, without ever rendering a quantum whose input has
   not arrived.
7. **Added latency is a constant `Q` frames**, not `Q - 1` and not a function of `N`. It is charged unconditionally,
   including for hosts whose `N` is always a multiple of `Q` and which would not otherwise need it, because a latency
   that varies with the caller's block pattern cannot be declared once or compensated statically. It is reported as a
   named contributor in the plan's latency accounting; ADR-0022 owns whether and how it is compensated.

### End of stream

8. **A stream has an explicit end, and ending it drains the carries.** On end-of-stream the renderer pushes `Q` frames
   of silent input, renders the quanta that completes, and emits the remaining output. Without this a render loses its
   last `Q` frames to the carry buffer.
9. **Offline rendering is latency-compensated; live rendering is not.** An offline request for `N` frames starting at
   plan sample `S` returns exactly `N` frames whose first sample *is* plan sample `S`: the renderer discards the `Q`
   priming frames from the head and drains `Q` frames past the end to fill the tail. The live path cannot do this —
   there is nothing to discard into — so live output carries the `Q`-frame latency of clause 7 while offline output
   carries none.
10. **Content is therefore identical between live and offline, and only the real-time delay differs.** This is what
    makes the P00A-T001 corpus a valid reference for both, and it is a stricter requirement than V1 meets today: V1's
    offline path renders at a different control rate than the live path, so its offline and live outputs are not the
    same signal at all.

### Time and events

11. **`SampleTime` counts in the engine input epoch**: sample `S` is the `S`-th input frame the renderer has consumed.
    The host's output stream position for the same instant is `SampleTime + Q`. That offset is a constant of the
    contract, not a per-callback measurement.
12. **An event is assigned to the quantum containing its timestamp** and is resolved before that quantum renders. No
    event is ever applied to samples already produced.
13. **Control evaluation is causal.** The evaluation at a quantum's start may observe only events whose timestamp is at
    or before that quantum's first sample. An event at a positive offset inside the quantum must not influence the
    control value used from offset 0 — the alternative is lookahead, and it would make a value depend on the future.
14. **The consequence is split, deliberately.** Sample-positioned effects of an event — note-on, note-off, gate,
    retrigger — occur at their declared sample within the quantum. The *control-rate* response to a mid-quantum event
    begins at the next quantum boundary, so control response carries up to `Q - 1` frames of additional delay beyond
    the event's sample position.
15. **Removing that control delay is left to ADR-0003 and ADR-0032, under two binding constraints:** whatever
    mechanism they choose — splitting the quantum at event boundaries, or segmented/ramped control values — must
    preserve clause 13's causality, and must not make control evaluation depend on how the caller partitioned its
    blocks. Splitting *internally* at event boundaries satisfies both, because the event set is a property of the plan
    and the timeline, not of the callback pattern. This record does not choose between them; it fixes what a choice may
    not break.
16. **An event whose timestamp falls in an already-rendered quantum is late.** It is clamped to the first
    not-yet-rendered quantum boundary and **counted**; it is never applied retroactively and never silently dropped.
    The counter is a field of the structured diagnostics report. ADR-0022 may refine the policy for a late event with
    hardware-clock evidence; it may not restore silent lateness.

### Comparison

17. **A render digest is comparable only within one quantum value.** Since the quantum is a constant, this constrains
    comparison across a superseding ADR-0037, not across configurations.

## Consequences

### Positive

- Partition invariance becomes achievable by construction rather than by test.
- Offline and live rendering of the same project evaluate control at the same rate, which is a prerequisite for the
  P00A-T001 corpus to represent what the user actually hears.
- The V1 short-block-at-stop-boundary rate jump disappears; there are no boundary blocks.
- A compile-time constant permits fixed-size scratch buffers, constant loop bounds, and unrolled or vectorized kernels
  without a remainder path.

### Negative

- A constant `Q` frames of added output latency on the live path, on top of the host buffer — charged even to hosts
  that would not need it, which is the price of a latency that can be declared once.
- Two carry buffers and their copies on every callback, which V1 does not pay.
- Live events gain a hard horizon: an event that misses its quantum is deferred by up to `Q` frames rather than being
  applied where it was stamped.
- Control response to a mid-quantum event lags its sample position by up to `Q - 1` frames until ADR-0003 or ADR-0032
  supplies event-boundary splitting or segmented control. Note and gate timing is unaffected.
- Offline rendering must drain and trim, so the offline path is not simply the live path with a different caller. That
  asymmetry is a permanent property of the contract, and a render path that forgets the trim produces output shifted by
  `Q` frames — silently, since it is still valid audio.
- The dispatch count rises relative to V1 for any host buffer larger than `Q`. Whether that is measurable in total
  render CPU is ADR-0037's question, not this one's.
- A constrained host cannot trade control resolution for CPU, because Option C was rejected.

### Risks and controls

- **Risk: a render path quietly adds a second quantum** (as `arrangement_render.rs` did with 256 in V1). Control: the
  quantum is a constant with no configuration surface, and the Phase 3 partition-invariance tests fail if any path
  splits differently.
- **Risk: the carry latency is forgotten in latency compensation.** Control: it is a named contributor in the
  resource/latency report, so it appears in the report ADR-0022 consumes rather than being implicit.
- **Risk: the `SampleTime + Q` offset is applied inconsistently** across the ingress mapper, the tempo map, and
  recording, producing a fixed timing error nobody attributes. Control: the offset is stated as a contract constant
  here, and Phase 3's partition-invariance tests are extended with an event-placement case that fails if any path
  disagrees.
- **Risk: the late-event counter becomes the second `LIMIT-0013`** — a counter published nowhere a user looks. Control:
  clause 16 places it in the structured diagnostics report, the same report ADR-0021 requires drop counts to reach.
- **Risk: an offline render path skips the trim** and emits audio shifted by `Q` frames, which no listening test would
  catch and which would silently corrupt every A/B comparison built on it. Control: a test that renders a known impulse
  at plan sample 0 offline and asserts it lands at output sample 0, run on every offline path — the WAV export, the
  headless renderer, and the comparison harness.
- **Risk: the carry copies show up as real cost at small quanta.** Control: ADR-0037's V1 proxy cannot measure this
  V2-only cost, so its mandatory Phase 2 re-measurement compares the real V2 renderer at every candidate quantum before
  kernels or buffer layouts may be tuned around the provisional value.

## Follow-up work

| Task                                                                                          | Phase | Status      |
|------------------------------------------------------------------------------------------------|-------|-------------|
| Fix the quantum's frame count (ADR-0037)                                                      | 0A    | Proposed    |
| Remove "maximum quantum" from the Phase 0A `HostProfile` work item                         | 0A    | Complete    |
| Remove `quantum` from `RenderConfig` in the master plan                                    | 0A    | Complete    |
| Remove "maximum render quantum" from the plan's `HostProfile` list                         | 0A    | Complete    |
| Declare the carry latency contributor in the `ResourceReport`                                 | 1     | Not started |
| Impulse-alignment test on every offline path: plan sample 0 lands at output sample 0          | 1     | Not started |
| Partition-invariance tests over `1 x 4096`, `16 x 256`, `64 x 64`, and irregular host blocks   | 3     | Not started |
| Event-placement test: the same events at the same sample times survive any host block pattern | 3     | Not started |
| Remove the `Q - 1` control-response delay via ADR-0003 or ADR-0032                            | 3     | Not started |
| Decide whether input-free plans may render ahead without the input carry (Option A′)          | 3     | Not started |

The three master-plan edits landed in the same change that accepted this record, as required by the
[documentation authority rule](../README.md#sources-of-truth).

## Revisit conditions

- A Phase 3 event-timing requirement (ADR-0032) proves incompatible with evaluating control exactly once per quantum,
  which would force either intra-quantum control evaluation or event-boundary splitting.
- A host integration is required whose latency budget cannot absorb `Q` frames, and for which no compensation under
  ADR-0022 is available. Rejecting Option C is what this condition would revisit.
- Measurement shows the unconditional `Q` priming to be a material cost for hosts that deliver whole multiples of `Q`,
  making a variable-latency scheme worth its compensation complexity.
- The frame count changes under a superseding ADR-0037. That alone does not revisit this record: every clause is stated
  in terms of `Q`.
