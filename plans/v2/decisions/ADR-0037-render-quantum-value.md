# ADR-0037: Internal Render Quantum — Frame Count

| Field         | Value                                    |
|---------------|------------------------------------------|
| ID            | ADR-0037                                 |
| Status        | Accepted (provisional value, see rule 1) |
| Phase         | 0A                                       |
| Created       | 2026-08-12                               |
| Last reviewed | 2026-08-12                               |
| Related       | ADR-0001, EVD-0002, P00A-T003, P00A-T006 |
| Supersedes    | —                                        |
| Superseded by | —                                        |

**Scope note.** [ADR-0001](ADR-0001-internal-render-quantum.md) decides how the quantum behaves; this record decides
only how many frames it is. Both are `Accepted`, and both were required `Accepted` by the Phase 0A exit gate. The split
exists because the frame count is the one part of the quantum question that depends on a measurement, and holding the
semantics hostage to that measurement would block Phase 1 for no gain.

**Outcome.** The measurement was taken and is recorded as
[EVD-0002](../evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md). It selected **rule 1**, so `Q` = 64 is accepted
**provisionally** and the Phase 2 re-measurement below is binding. See *Evidence*.

This ADR is registered as a split of the master plan's Part VII topic 1, which names one decision. The Phase 0A
tracker records the split as a deviation.

## Context

ADR-0001 fixes a compile-time constant `Q`, identical across live, offline, and test rendering, with whole-quantum-only
production through a carry buffer. Every clause there is stated in terms of `Q`. The remaining question is its value.

The master plan proposes 32 or 64 frames. The choice trades control resolution against per-quantum overhead, and the
overhead side has never been measured in this project — all 74 entries in the
[resource inventory](../inventories/resource-limits.md) are read from source.

**Outside this decision.** Everything in ADR-0001: the carry buffer, the ban on partial quanta, control evaluated once
per quantum, and the rejection of a configurable quantum.

## Decision drivers

- `Q` sets worst-case control latency and block-rate modulation resolution. At 48 kHz: 32 frames is 0.67 ms, 64 is
  1.33 ms, 128 is 2.67 ms. These are arithmetic, not measurements.
- Per-quantum overhead is paid once per node per quantum, so halving `Q` doubles it for identical audio. This is the
  only driver requiring measurement, and the only reason this record is not already accepted.
- A constant `Q` frames of carry latency is charged to the live path under ADR-0001 clause 7, so `Q` is also a latency
  decision and not only a CPU one.
- A `Q` that is a multiple of the widest vector width (8 f32 for AVX2, 16 for AVX-512) keeps kernels vectorizable with
  no scalar remainder loop. 32, 64, and 128 all qualify.
- V1 comparability: the A/B corpus is rendered offline at 256 frames. A `Q` that divides 256 evenly makes "same total
  frames" partitions exact rather than approximate. 32, 64, and 128 all qualify.

## Options considered

### Option A: 32 frames

Best control resolution (0.67 ms at 48 kHz) and the finest block-rate modulation; lowest carry latency. Highest
per-quantum overhead — twice the dispatch count of 64 for identical audio, paid by every node in every voice.
Attractive only if the measured overhead turns out to be negligible.

### Option B: 64 frames

1.33 ms control resolution at 48 kHz. Divides 128, 256, 512, and 1024 evenly, so it partitions every buffer size V1 can
request and the offline renderer's 256 exactly. Halves Option A's dispatch count. The master plan's leading candidate
alongside 32.

### Option C: 128 frames

Lowest overhead of the three. 2.67 ms control latency is audible as sluggishness on fast envelopes and percussive
material, and it is coarse relative to the modulation rates the Mod Matrix and YAMS already support. Reasonable only if
measurement shows 64 to be materially expensive.

### Status quo

Leave the value open. ADR-0001's semantics are implementable in terms of `Q`, but Phase 1 cannot compile a constant it
does not have, and the Phase 0A exit gate is not satisfiable.

## Evidence

[EVD-0002](../evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md), the V1
proxy measurement defined below. 8 640 recorded renders of the four corpus cases
across four builds, interleaved so machine drift could not be read as a
block-size effect.

| Estimator | c(32) | c(64) | c(128) | c(256) | r(64,256) | r(128,256) | r(32,64) |
|-----------|-------|-------|--------|--------|-----------|------------|----------|
| minimum   | 6.942 | 6.468 | 6.210  | 5.884  | +9.93%    | +5.54%     | +7.34%   |
| median    | 7.486 | 6.803 | 6.428  | 6.198  | +9.78%    | +3.72%     | +10.04%  |
| mean      | 7.666 | 6.978 | 6.618  | 6.322  | +10.38%   | +4.69%     | +9.86%   |

Costs are milliseconds of CPU per second of rendered audio, pooled across the
corpus and weighted by each case's rendered duration.

**Rule 1 applies.** Rules 2, 3, and 4 fire on no estimator. Rule 1 is close, and
the closeness is the finding: the minimum and the median put `r(64,256)` 5.07 and
5.22 pp from the 15% threshold — just outside the margin — while the mean puts it
4.62 pp away, inside it. A bootstrap over rounds puts 30–42% of resampled
measurements inside the band. Which side of the rule the data falls on is decided
by the choice of estimator at a margin of a tenth of a percentage point, which is
the situation the margin exists to catch. EVD-0002 records the alternative
reading and its outcome (rule 5, confirm 64); both select 64, and the difference
is only whether the Phase 2 re-measurement binds.

**The corpus composition is doing visible work.** Per case, `r(64,256)` runs from
+2.42% to +17.35%, and the polyphonic case alone is above rule 2's threshold. The
measurement resolves per-block overhead at about 0.8 µs pooled, and it scales with
how many voices and modules are active — so a corpus weighted differently would
move the pooled ratio. Four of the master plan's eleven categories are covered.
That does not invalidate the result, but it is a second reason not to treat it as
conclusive, and it is recorded in EVD-0002's *Limitations*.

**What the measurement was waiting on.** P00A-T001's corpus and the `pertylizer
compare` harness, both of which now exist. The sweep additionally needed a
harness that times the render call alone; that is
`crates/pertylizer/src/bin/render_cost.rs`, added with EVD-0002.

Note what changing `BUFFER_SIZE` does beyond cost: V1 dispatches sequencer events
on block boundaries, so the renders at 32 and at 256 frames are **not the same
audio**. Onsets quantize differently. The proxy measures cost, not agreement, and
the `EVD` record should say so — while noting that the audible difference across
block sizes is itself the evidence behind the corpus's own `CORPUS-0001-C1`
claim, which is that V2 fixes exactly this.

### Why the measurement cannot be taken directly

The quantity that separates the options is per-quantum overhead **in the V2 compiled node model**, and no V2 renderer
exists — Phase 1 has not started. Measuring the real thing requires the artifact whose construction this decision is
supposed to gate. Waiting for it inverts the plan's own ordering.

### The V1 proxy measurement

The available substitute is to measure V1's cost as a function of block size:

- `BUFFER_SIZE` in `crates/pertylizer/src/audio/arrangement_render.rs:51` is a constant. Rendering the P00A-T001 corpus
  offline at 32, 64, 128, and 256 frames yields cost per rendered second at four block sizes.
- V1 evaluates control scripts once per block (`voice.rs:1096`), so the measurement captures the same per-block control
  cost V2 will pay per quantum.

**The proxy has no established direction of error.** An earlier revision claimed it was conservative, on the grounds
that V1 pays a per-voice `mono_buffer.resize()` every block (`voice.rs:1147`) that V2 removes. That argument does not
survive inspection: the buffer is allocated once at `MAX_BUFFER_SIZE` (`voice.rs:570`), and `AudioBuffer::resize` is
`Vec::resize` (`module_traits.rs:162`), so after the first shrink a same-size resize neither allocates nor fills. The
claim is withdrawn.

The error runs in both directions and the net is unknown:

- **V1 charges small blocks more than V2 will** through per-block work V2 removes or compiles away — dynamic graph
  traversal and per-block bookkeeping repeated once per block.
- **V2 charges small blocks more than V1 does** through work V1 has never paid: the two carry-buffer copies and the
  input/output bookkeeping of ADR-0001 clause 5, plus per-quantum scheduler cost.

**What the proxy can still show.** The *shape* of the cost-versus-block-size curve for the per-block work both engines
share, which is enough to distinguish a mild slope from a steep one. It cannot produce a transferable absolute figure.
Because the direction of error is unknown, a result within 5 percentage points of any threshold below is inconclusive
by definition and defers the decision to the Phase 2 re-measurement rather than resolving it.

## Decision

**`Q` = 64 frames**, provisionally under rule 1 below, on the evidence above.

64 is proposed rather than 32 because it is the more conservative choice against the one unmeasured driver: if the
overhead is larger than expected, 64 absorbs it, whereas 32 would have to be corrected after Phase 1 has compiled
kernels around it. The measurement is defined to be capable of overturning this in either direction.

Acceptance requires an `EVD` record containing the proxy measurement. Let `c(Q)` be the measured cost per rendered
second at quantum `Q`, and let `r(a, b) = c(a)/c(b) - 1` be the relative cost increase from `b` to `a`.

The outcome is decided by evaluating these rules **in order** and stopping at the first that applies. They are ordered
and exhaustive, so exactly one outcome is reachable for any measurement:

| # | Condition                                     | Outcome                                                   |
|---|-----------------------------------------------|-----------------------------------------------------------|
| 1 | any comparison used below falls within 5 pp of its threshold | **Provisional 64** — accept this record with the binding Phase 2 obligation below |
| 2 | `r(64, 256) > 15%` and `r(128, 256) <= 15%`   | **Select 128** — supersede this record                    |
| 3 | `r(64, 256) > 15%` and `r(128, 256) > 15%`    | **Escalate** — every candidate is expensive; reconsider the Option C rejection in ADR-0001 with evidence |
| 4 | `r(32, 64) <= 2%`                             | **Select 32** — the finer resolution and lower latency are free; supersede this record |
| 5 | otherwise                                     | **Confirm 64** — accept this record                       |

Rules 2 and 3 are what the 128-frame measurement is for; the first revision of this record measured 128 without using
it in any criterion, so a "64 is too expensive" result had no defined successor.

### Why rule 1 accepts rather than defers

The first revision made an inconclusive measurement defer the decision to Phase 2. That deadlocks the plan: the
Phase 0A exit gate requires this record `Accepted` before Phase 1 may begin, and Phase 2 is downstream of Phase 1, so
the deferral target is unreachable from the state that produces it. The measurement most likely to be inconclusive
would therefore have been the one that stops the project.

Rule 1 resolves it in the direction that keeps the gate intact. An inconclusive proxy accepts `Q` = 64 **provisionally**
and carries a binding obligation:

- The Phase 2 re-measurement against real V2 nodes is a **Phase 2 exit-gate item**, not a suggestion. Phase 2 may not
  close with the question still open.
- If that re-measurement selects 32 or 128, this record is superseded and the constant changes. Phase 2 is chosen as
  the deadline precisely because a constant is still cheap to change there — before Phase 5's declarative node API and
  before any kernel is tuned around a specific `Q`.
- Until then, no work may treat 64 as settled: no hand-unrolled kernel, no `Q`-specific buffer layout, no test
  asserting a control rate in Hz.

The alternatives were considered and rejected. Building a Phase 0A prototype good enough to measure V2 dispatch is
Phase 1 and 2 work done early under another name. Marking the ADR `Deferred` to a later gate weakens the exit gate the
master plan wrote deliberately, and the Phase 0A tracker would have to record it as a deviation.

## Consequences

### Positive

- Phase 1 gets a concrete constant to compile against.
- The acceptance criterion is falsifiable and stated before the data is collected, so the measurement cannot be read to
  confirm a preference after the fact.
- Splitting the value out means a later change of `Q` supersedes only this record, leaving ADR-0001's semantics intact.

### Negative

- The proxy measures V1, not V2, so acceptance rests on an argued transfer rather than a direct observation, and the
  transfer is now explicitly two-directional rather than conservative.
- Rule 1 makes a provisional constant a likely outcome rather than an edge case, so Phase 1 may well be built on a `Q`
  that Phase 2 changes. That is the honest cost of measuring a proxy, and it is why rule 1 forbids tuning anything to
  the value.
- 64 frames charges the live path 64 frames of carry latency under ADR-0001 — 1.33 ms at 48 kHz — before any host
  buffer is counted.

### Risks and controls

- **Risk: the proxy misranks the options**, because V2's carry copies and scheduler cost fall differently across
  quanta than V1's per-block work. Control: rule 1's 5-point margin, plus the Phase 2 re-measurement named in the
  follow-up table, run against real V2 nodes while changing `Q` is still cheap.
- **Risk: the measurement is taken on a corpus too small to be representative.** Control: it is defined over the
  P00A-T001 corpus, which the master plan already enumerates, rather than over an ad-hoc patch. **Materialized.**
  EVD-0002 measures a per-case `r(64, 256)` spread of +2.42% to +17.35% across the four covered categories, which
  straddles rule 2's threshold; the pooled figure is therefore a property of the current corpus as well as of the
  renderer. The control that remains is rule 1, which fired.
- **Risk: 64 becomes entrenched before it is verified.** Control: the value is accepted *provisionally*, and the last
  bullet above forbids tuning anything to it until the Phase 2 re-measurement.

## Follow-up work

| Task                                                                                   | Phase | Status      |
|-----------------------------------------------------------------------------------------|-------|-------------|
| Render the P00A-T001 corpus at 32/64/128/256 frames and record cost per rendered second | 0A    | Done — EVD-0002 |
| Record the proxy measurement as an `EVD` record and apply the rule table                | 0A    | Done — EVD-0002, rule 1 |
| **If rule 1 fires:** add the Phase 2 re-measurement to the Phase 2 exit gate            | 1     | Done early — rule 1 fired, so the item is in the master plan's Phase 2 exit gate now rather than waiting for Phase 1 to add it |
| Re-measure against real V2 nodes and confirm or supersede                               | 2     | Not started — binding |

## Revisit conditions

- The Phase 2 re-measurement contradicts the proxy's cost-curve shape.
- A confirmed requirement for control-rate modulation faster than `sample_rate / Q` that cannot be met by moving the
  affected signal to audio rate.
- A host integration whose latency budget cannot absorb `Q` frames, where reducing `Q` is cheaper than compensating
  under ADR-0022.
