# ADR-0037: Internal Render Quantum — Frame Count

| Field         | Value                                    |
|---------------|------------------------------------------|
| ID            | ADR-0037                                 |
| Status        | Proposed                                 |
| Phase         | 0A                                       |
| Created       | 2026-08-12                               |
| Last reviewed | 2026-08-12                               |
| Related       | ADR-0001, P00A-T003, P00A-T006           |
| Supersedes    | —                                        |
| Superseded by | —                                        |

**Scope note.** [ADR-0001](ADR-0001-internal-render-quantum.md) decides how the quantum behaves; this record decides
only how many frames it is. ADR-0001 is `Accepted`; this record remains `Proposed` pending its measurement, and both are
required `Accepted` by the Phase 0A exit gate. The split exists because the frame count is the one part of the quantum
question that depends on a measurement, and holding the semantics hostage to that measurement would block Phase 1 for
no gain.

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

None yet. This is the record's defining weakness and the reason it is `Proposed`.

### Why the measurement cannot be taken directly

The quantity that separates the options is per-quantum overhead **in the V2 compiled node model**, and no V2 renderer
exists — Phase 1 has not started. Measuring the real thing requires the artifact whose construction this decision is
supposed to gate. Waiting for it inverts the plan's own ordering.

### The V1 proxy measurement

The available substitute is to measure V1's cost as a function of block size:

- `BUFFER_SIZE` in `crates/pertylizer/src/audio/arrangement_render.rs:51` is a constant. Rendering the P00A-T001 corpus
  offline at 32, 64, 128, and 256 frames yields cost per rendered second at four block sizes.
- V1 evaluates control scripts once per block (`voice.rs:1088`), so the measurement captures the same per-block control
  cost V2 will pay per quantum.

**The proxy has no established direction of error.** An earlier revision claimed it was conservative, on the grounds
that V1 pays a per-voice `mono_buffer.resize()` every block (`voice.rs:1139`) that V2 removes. That argument does not
survive inspection: the buffer is allocated once at `MAX_BUFFER_SIZE` (`voice.rs:570`), and `AudioBuffer::resize` is
`Vec::resize` (`module_traits.rs:156`), so after the first shrink a same-size resize neither allocates nor fills. The
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

Proposed: **`Q` = 64 frames**, pending the V1 proxy measurement below.

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
  P00A-T001 corpus, which the master plan already enumerates, rather than over an ad-hoc patch.
- **Risk: 64 becomes entrenched before it is verified.** Control: this record stays `Proposed`, so it is not an
  implementation constraint, and the Phase 0A tracker lists it as a gate item.

## Follow-up work

| Task                                                                                   | Phase | Status      |
|-----------------------------------------------------------------------------------------|-------|-------------|
| Render the P00A-T001 corpus at 32/64/128/256 frames and record cost per rendered second | 0A    | Not started |
| Record the proxy measurement as an `EVD` record and apply the rule table                | 0A    | Not started |
| **If rule 1 fires:** add the Phase 2 re-measurement to the Phase 2 exit gate            | 1     | Not started |
| Re-measure against real V2 nodes and confirm or supersede                               | 2     | Not started |

## Revisit conditions

- The Phase 2 re-measurement contradicts the proxy's cost-curve shape.
- A confirmed requirement for control-rate modulation faster than `sample_rate / Q` that cannot be met by moving the
  affected signal to audio rate.
- A host integration whose latency budget cannot absorb `Q` frames, where reducing `Q` is cheaper than compensating
  under ADR-0022.
