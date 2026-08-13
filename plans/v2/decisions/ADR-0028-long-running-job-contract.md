# ADR-0028: Long-Running Job Contract

| Field         | Value                                                        |
|---------------|--------------------------------------------------------------|
| ID            | ADR-0028                                                     |
| Status        | Deferred                                                     |
| Phase         | 0A, deferred to the Phase 4 entry gate                       |
| Created       | 2026-08-13                                                   |
| Last reviewed | 2026-08-13                                                   |
| Related       | ADR-0027, ADR-0029, ADR-0030, ADR-0035, ADR-0032, P00A-T006  |
| Supersedes    | —                                                            |
| Superseded by | —                                                            |

**Class.** `Contract`. It decides a lifecycle, an ownership boundary, and failure behavior, not a value.

## The deferral

| Field                | Value                                                                                   |
|----------------------|-----------------------------------------------------------------------------------------|
| Deferred to          | The **Phase 4 entry gate**. Phase 4 (V1 lowering and offline A/B) may not begin before this is `Accepted` |
| Owner                | Project maintainer — this is a single-maintainer repository, so there is no second party to assign |
| Evidence required    | A workflow analysis of every existing render and analysis caller: what each needs of progress, cancellation, pinning, and a result; how MCP clients behave on a long synchronous call; and whether the GUI's export-progress model survives a second consumer |
| Why not now          | The register's basis is `Render/analysis workflow analysis`, and that analysis has not been done. Nothing in Phases 1-3 creates a job |
| What makes it safe   | Phase 0A's own executable work — the corpus and `pertylizer compare` — runs synchronously and is not a job |

The Phase 0A exit gate accepts this record as `Deferred` on those four fields, and the master plan permits deferral
**only** to the Phase 4 entry gate.

## Context

A long-running job is a revision-pinned, cancellable operation such as render, export, or analysis, with progress,
diagnostics, and a result receipt. V2 needs one contract for it because every frontend will want the same operation.

V1 has no such contract, and the interesting part is not that it is missing but *how* it is missing: three of the four
pieces already exist, in three places, each built for one caller. Read at `e4873d0b`:

- **There is no job abstraction at all.** No `JobId`, no job registry, no handle type anywhere in the workspace; a
  search for one returns nothing. Every long operation is a function call.
- **Progress and cancellation exist once**, as `ExportProgress` (`crates/pertylizer/src/audio/export.rs:49-90`): atomic
  frame counters plus a cancel flag, polled by the GUI so it can draw a bar. It is not reachable from MCP or the CLI.
- **The MCP long operations are synchronous.** `render_to_wav_impl` and `analyze_section_impl`
  (`crates/pertylizer/src/mcp_bridge/analysis_impl.rs:2784`, `:4327`) render inside the request. There is no progress,
  no cancellation, and no way to observe an operation in flight — the client waits or gives up.
- **A result receipt exists, for exactly one path.** The headless renderer produces a versioned `RenderReceipt`
  (`crates/pertylizer/src/render/receipt.rs:25`) with a protocol version, input and output digests, audio and mix
  information, and warnings including everything a project load could not reconstruct. It is the best-developed piece
  of the contract and nothing else can produce one.
- **Lifecycle is improvised where it is handled at all.** The headless loader has a 300-second wall-clock
  `LOAD_DEADLINE` (`crates/pertylizer/src/render/headless.rs:60`) as a deadlock backstop, with a comment explaining why
  it is wall-clock rather than a block count. That is job-lifecycle reasoning, written for one call site.

**What this record will decide**, at the Phase 4 entry gate:

1. Job identity and lifecycle: the states, which are terminal, and who owns the handle.
2. Revision pinning: which project revision a job operates on, and what happens when the document changes while it
   runs. This is where it meets ADR-0035's transaction semantics.
3. Cancellation: cooperative or immediate, what partial output survives, and what the receipt says about it.
4. Progress: its granularity, and whether it is lossy telemetry under ADR-0027's class or a delivered value.
5. The result receipt: whether `RenderReceipt` generalizes to analysis and export, and how it is versioned.
6. Failure and diagnostics, including how a job's error reaches a caller that has already disconnected.
7. Which surface each frontend gets — an asynchronous MCP tool pair, a blocking CLI, a GUI poll — which touches
   ADR-0029 and ADR-0030.
8. Whether an offline render inside a job is one of the engine's own prepared streams, and therefore carries its own
   `StreamEpoch` under ADR-0032 clause 12.

**Outside it.** What the analyzers observe and own (ADR-0027), remote authorization (ADR-0029), the public facade
(ADR-0030), and transaction atomicity (ADR-0035).

## Decision drivers

- The register's basis for this topic is a workflow analysis, not a measurement. The analysis is cheap but has not been
  done, and deciding without it would design for the callers that come to mind rather than the ones that exist.
- The pieces that exist are evidence about requirements: someone needed progress for export, a receipt for the headless
  renderer, and a deadline for loading. A contract that cannot express all three is already known to be wrong.
- Nothing in Phases 1-3 creates a job, so the decision has no earlier consumer. Phase 4 is the first: offline A/B runs
  many renders and is where an uncancellable, unobservable render becomes painful.
- A job contract touches four other open decisions. Settling it early, alone, would fix a boundary that those four have
  not yet had a chance to argue with.

## Options considered

**Deliberately not surveyed**, for the same reason ADR-0022's are not: the choice is what the missing analysis makes.
The shape of the space is visible — a handle-and-poll model like `ExportProgress` generalized; a job registry with
identifiers; an operation queue folded into Application Core V2's transaction path — and each implies a different
answer to revision pinning and to what a disconnected client can retrieve. Recording a preference now would be a
prediction, not a decision.

What is *not* open: a job must be able to carry the `RenderReceipt` shape that already exists, because that receipt is
the load-diagnostics surface a render depends on, and losing it would be a regression rather than a redesign.

### Status quo

Keep V1's arrangement: synchronous MCP renders, one bespoke GUI progress mechanism, one receipt, and one wall-clock
deadline. Phase 4's A/B work would then either block the MCP client for the length of each render or grow a second
bespoke mechanism beside `ExportProgress`, which is precisely the shape this record exists to stop.

## Evidence

- Source reads at `e4873d0b`: `crates/pertylizer/src/audio/export.rs:49-90`,
  `crates/pertylizer/src/mcp_bridge/analysis_impl.rs:2784,4327`, `crates/pertylizer/src/render/receipt.rs:25`,
  `crates/pertylizer/src/render/headless.rs:60`, plus a workspace-wide search finding no job identifier or registry.
- The glossary's definition of a runtime job, which this record must satisfy rather than redefine.

**The evidence that is missing** is the workflow analysis itself: which callers need which parts of the contract, and
what an MCP client actually does when a call takes minutes. Both are obtainable at any time; neither has been done.

## Decision

**Deferred to the Phase 4 entry gate**, with the owner and evidence recorded in *The deferral* above.

Two constraints hold in the meantime:

1. **No new long-running synchronous surface** should be added to MCP or the CLI without recording that it will be
   migrated. The existing ones are a known cost; new ones are a growing one.
2. **No second progress or cancellation mechanism** may be built beside `ExportProgress`. A caller that needs one is
   evidence for this record and should be written down as such rather than served with a third bespoke channel.

Neither constraint blocks Phase 1-3 work, because none of it creates a job.

## Consequences

### Positive

- The contract will be written against the callers that exist, which are already enumerated above, rather than against
  an imagined set.
- Phase 4 gets the decision exactly when it acquires its first real consumer, and the four adjacent decisions get to
  influence it first.

### Negative

- MCP keeps its blocking renders until Phase 4, so an agent driving a long render still has no way to observe or cancel
  it.
- `ExportProgress` and `RenderReceipt` keep diverging in the meantime, and the eventual contract will have to absorb
  both rather than replace one.

### Risks and controls

- **Risk: a third bespoke mechanism appears** before the contract does, making the migration larger than the decision.
  Control: constraint 2, which turns the need into evidence instead of into code.
- **Risk: the deferral is extended past Phase 4** because A/B work can be done with blocking renders if one is patient.
  Control: the master plan permits deferral only to the Phase 4 entry gate.
- **Risk: the workflow analysis is skipped** and the record is written from this context section instead. Control: the
  register's basis names the analysis, and the Phase 4 entry gate checks the basis, not the prose.

## Follow-up work

| Task                                                                              | Phase | Status      |
|-----------------------------------------------------------------------------------|-------|-------------|
| Workflow analysis of every render and analysis caller                             | 4     | Not started |
| Establish what MCP clients do on a multi-minute synchronous call                  | 4     | Not started |
| Write and accept this record against that analysis                                | 4     | Not started |
| Decide whether `RenderReceipt` generalizes or is one instance of a receipt family  | 4     | Not started |

## Revisit conditions

Superseded by its own accepted version at the Phase 4 entry gate. It would be revisited earlier only if a Phase 1-3
task turned out to need a job — the plausible case being a Phase 2 or Phase 3 test harness that renders long enough to
want cancellation, which would be an argument for bringing the analysis forward rather than for improvising a
mechanism.
