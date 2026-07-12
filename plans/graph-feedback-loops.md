# Graph feedback loops (allow cycles via a delay)

Status: PROPOSED

Origin: external architecture review (§1, "Hantera Feedback-loopar"), evaluated
against the code 2026-07-12.

## 0. Correcting the premise

The review worried that a cyclic connection could make the sort *"fastna eller
krascha"*. **It cannot** — the graph already rejects cycles defensively:

- `crates/synth_engine/src/graph.rs:653` — `would_create_cycle()` (DFS from the
  proposed destination back to the source) is checked in `validate_connection`
  (`graph.rs:646`) and the connection is refused with `GraphError::CycleDetected`
  (*"Connection would create a cycle"*, `graph.rs:917`).
- `calculate_processing_order` (`graph.rs:682`, Kahn's algorithm) additionally
  falls back to arbitrary order if a cycle ever slipped through
  (`graph.rs:744`), so there is no hang.

So this is **not a bug fix** — it's a *feature request*: today feedback is simply
**forbidden** at the graph level. Feedback that users do get lives *inside*
modules (delay lines, comb/Karplus structures, `audio_script` IIR state). The
proposal is to allow **explicit graph-level feedback edges**, resolved with a
delay so the topo sort still succeeds. That is what the review's z⁻¹ suggestion
is really asking for.

## 1. Goal

Allow a user to connect an output back to an upstream input (feedback FM,
graph-level resonators, effect feedback paths) without the connection being
rejected — by treating the back-edge as delayed by one block, so the graph stays
acyclic for scheduling.

## 2. Design

### A. Feedback edges instead of rejection

When `validate_connection` detects that a new edge would close a cycle, instead
of erroring, allow it and **tag it as a feedback edge**
(`Connection { .., is_feedback: bool }`, `graph.rs:34`). A feedback edge is
**excluded** from `calculate_processing_order`'s adjacency/in-degree build
(`graph.rs:700`), so Kahn's sort still linearises every node exactly once — the
back-edge just isn't part of the ordering constraints.

### B. One-block delay (z⁻ᵇˡᵒᶜᵏ) semantics

Because the consumer of a feedback edge runs **before** its producer within a
block (the edge is excluded from ordering), the consumer must read the
producer's output from the **previous** block:

- Preserve the feedback source's output buffer across the block boundary (don't
  clear/overwrite it before the consumer reads). Simplest: give each feedback
  **source port** a small retained buffer that holds last block's output; the
  incoming-connection resolver (`incoming_map`, `graph.rs:751`) reads that
  retained buffer for feedback edges and the live buffer for normal edges.
- Latency is one block (e.g. 64–128 samples). Acceptable and well-defined for
  graph-level feedback; document it.

### C. Sample-accurate feedback is out of scope

True single-sample z⁻¹ feedback requires evaluating the whole graph
sample-by-sample, which this **block-based** engine does not do — a major
rearchitecture. Keep that out of scope: sample-accurate feedback stays the
domain of dedicated modules (delay/comb/Karplus, `audio_script` per-sample IIR
state). This plan delivers block-latency feedback only.

### D. Stability / safety

Feedback can diverge. At minimum, document it. Consider a lightweight safety on
feedback paths (e.g. a soft-clip/limiter on the retained feedback buffer, or a
NaN/Inf flush) so a runaway loop degrades to noise/silence rather than poisoning
the buffer — behind the existing non-finite defensiveness. Keep it optional and
cheap; modular users expect to manage feedback gain themselves.

### E. UI / API surface

- `connect` (MCP) and the GUI patch drag stop erroring on a back-edge; they
  create a feedback edge. `check_connection` reports "would be a feedback
  (delayed) connection" instead of "cycle — rejected".
- `get_connections` flags which edges are feedback.
- The GUI draws feedback cables distinctly — the cable-routing plan
  (`plans/cable-routing.md` §2.A) already contemplates a wider downward arc for
  "Backward/Feedback Loops"; reuse that so feedback is visually obvious.

## 3. Real-time safety

Retained feedback buffers are **pre-allocated** when topology changes (same time
`calculate_processing_order` runs, off the audio hot path). `process()` only
reads/writes existing buffers — no allocation, no lock. The optional limiter is
a scalar clamp.

## 4. Files to touch

- `crates/synth_engine/src/graph.rs` — `is_feedback` on `Connection`; classify
  back-edges in `validate_connection` instead of rejecting; exclude them from
  `calculate_processing_order`; retained-buffer handling in the incoming
  resolver; allocate retained buffers on topology change.
- `crates/synth_engine/src/voice.rs` — ensure the per-voice graph honours the
  retained-buffer read for feedback edges.
- `crates/pertylizer/src/patch.rs` — persist `is_feedback` on `ConnectionState`
  (`patch.rs:575`) so feedback patches round-trip.
- MCP (`connect` / `check_connection` / `get_connections`) — accept + report
  feedback edges.
- GUI wiring — draw feedback cables distinctly; stop surfacing the cycle error
  on back-edges.

## 5. Open questions

- **Auto-detect vs explicit.** Auto-promoting any cycle-closing edge to feedback
  is the least friction, but a user could create an *unintended* delayed loop.
  Alternative: require an explicit "feedback" gesture/flag. Recommend
  auto-promote + clear visual marking so it's never silent.
- **Where to retain the buffer** — per source-port vs per feedback-edge. Per
  source-port is simpler (one retained copy feeds all its feedback consumers).
- **Limiter default** — on or off for feedback paths? Lean off (purist), document
  loudly, offer a per-connection safety toggle later.
- **Legacy load.** Additive: `is_feedback` defaults false; old patches
  unaffected.

## 6. Exit gate

- Connecting `flt-1.out → osc-1.fm_amt` (a back-edge) succeeds and is marked as
  feedback rather than rejected.
- The patch produces stable, bounded feedback (block-latency), audibly
  responding to feedback amount.
- Feedback edges are visually distinct in the patch view and round-trip through
  save/load.
- Normal (acyclic) patches are unchanged; `CycleDetected` no longer fires for
  intended feedback.
- A runaway loop degrades gracefully (no NaN poisoning the mix).
- Workspace green (`build` / `clippy --all-targets` / `test` / `fmt --check`).
