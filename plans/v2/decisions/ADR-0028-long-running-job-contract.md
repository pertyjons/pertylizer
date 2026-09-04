# ADR-0028: Long-Running Job Contract

| Field         | Value                                                        |
|---------------|--------------------------------------------------------------|
| ID            | ADR-0028                                                     |
| Status        | Deferred                                                     |
| Phase         | 0A/4/10B, deferred to Phase 10B — see `P04-R004`             |
| Created       | 2026-08-13                                                   |
| Last reviewed | 2026-09-01                                                   |
| Related       | ADR-0027, ADR-0029, ADR-0030, ADR-0035, ADR-0032, P00A-T006  |
| Supersedes    | —                                                            |
| Superseded by | —                                                            |

**Class.** `Contract`. It decides a lifecycle, an ownership boundary, and failure behavior, not a value.

## The deferral

| Field                | Value                                                                                   |
|----------------------|-----------------------------------------------------------------------------------------|
| Deferred to          | **Phase 10B**, which builds the revision-pinned job service that captures a job's immutable project/asset snapshot, and which follows Phase 10A's canonical document revision. The Phase 4 deadline is withdrawn by `P04-R004` |
| Owner                | Project maintainer — this is a single-maintainer repository, so there is no second party to assign |
| Evidence required    | A workflow analysis of every existing render and analysis caller: what each needs of progress, cancellation, pinning, and a result; how MCP clients behave on a long synchronous call; and whether the GUI's export-progress model survives a second consumer |
| Why not now          | The analysis this record required has now been done, in Phase 4, and is under *Evidence*. What blocks acceptance is the word *revisioned*: a project revision is Phase 10A's, and capturing a job's snapshot from one is Phase 10B's, so a contract accepted earlier would name a revision that does not exist |
| What makes it safe   | Phase 0A's own executable work — the corpus and `pertylizer compare` — runs synchronously and is not a job |

The Phase 0A exit gate accepted this record as `Deferred` to the Phase 4 entry
gate. Its 2026-09-01 boundary correction leaves that completed review unchanged
and narrows what may start before acceptance: only the pure lowerer and one
bounded in-process smoke render. No shared `RenderRequest`/`RenderResult`,
multi-project A/B orchestration, streaming sink, progress, cancellation, GUI,
CLI or MCP surface may be built before this record is accepted.

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

**What this record will decide**, before the first render-orchestration slice — which
`P04-R004` moved out of Phase 4 along with this record's deadline:

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
- Nothing in Phases 1-3 creates a job, and neither does a pure lowering pass or
  one bounded in-process smoke render. Multi-project offline A/B is the first
  consumer: it runs many renders and is where an uncancellable, unobservable
  render becomes painful.
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

**The workflow analysis, done in Phase 4 at `4cfcc23c`.** The deferral asks what each caller
*needs* of progress, cancellation, pinning and a result — not only what each has — so both are
below. "Needs pinning" asks whether the caller reads state that can change while it runs.

| Caller | Surface and bounds | Has | Needs |
|---|---|---|---|
| GUI export | `start_export` with `ExportProgress`; duration, tail, format and rate bounded in the dialog | progress as frames and total in atomics; a cooperative cancel flag; `ExportDialogResult`; no receipt. **It already pins**: `begin_export` takes an owned `ProjectFile` and the worker builds a fresh engine from it, so a mid-render edit cannot alter the render | it has progress and cancellation because a user watches it. It needs a **result with diagnostics** — today a failed export reports a string. Its snapshot has no revision identity, which is the part Phase 10A supplies |
| GUI analyze | a spawned thread with one `pending` slot; duration 50–10 000 ms and tail 0–10 000 ms | no progress, no cancellation; a **partial** stale guard — the result is discarded when the target instrument changed, but an edit to the *same* instrument still lands, because the comparison is on `InstrumentId` alone | it needs **stale-result labelling** rather than a silent discard, and it needs the comparison to be on a revision rather than a target. Cancellation cannot replace the guard, because a cancel races with completion. Its bounds keep it short enough not to need progress |
| CLI `render` | `WavRenderRequest` and `render_window_to_wav`, blocking; duration, tail, sample rate and projected buffer size bounded | `RenderReceipt` v1 | it needs the **result**, and has the best one. Progress would be a convenience; cancellation is the terminal's. Pinning: it loads a file, so its input cannot change under it |
| MCP `render_to_wav` | the **same** `WavRenderRequest`, synchronous; window and duration limits | a tool payload that **does** carry setup and render warnings, so reconstruction problems reach the client | it needs **progress, cancellation and pinning**. Its result already reports what was lost; what it lacks is a durable receipt, which is the CLI's |
| MCP spectrum and section analysis | the arrangement renderer directly, synchronous; some clamp and warn, others reject | a tool payload | as above, and **pinning** most of all: they read live engine and song state, so a concurrent edit changes what is measured |
| MCP `preview_note` and `analyze_note` | offline note rendering, synchronous; their own duration and tail bounds | two further tool payload shapes | short enough that progress is moot; they need **pinning** and a **diagnostic-carrying result** |
| MCP `analyze_instrument_range` and `analyze_velocity_response` | one offline render **per sweep step**, synchronous | a tool payload with warnings | the strongest case for **progress and cancellation** in the tree: aggregate duration is the step count times a render, and neither is bounded by one render's limits |
| MCP `analyze_mix_bus`, `analyze_master_chain`, `analyze_return_busses`, `analyze_masking_matrix` | one full render **per track, effect prefix or bus**, synchronous | a tool payload with warnings | the same needs as the sweeps, and for the same reason: duration is a product. They add a **pinning** requirement the sweeps do not, because each render in the series must see the same song and engine state or the series is not comparable with itself |
| Corpus and EVD harnesses | direct calls; fixture-sized | none | none. They are the control: a caller that needs no part of the contract shows the contract is not universally required |

Five findings, four of which the record's drivers anticipated:

1. **A shared render request already exists**, so this record's request is not greenfield. What
   it lacks is what matters: it takes a live `&Arc<SharedSong>` rather than a pinned snapshot,
   it is file-shaped, it carries no stem selection, and it has no progress, cancellation or
   failure state.
2. **The pieces do not compose.** `ExportProgress` has progress and cancellation and no
   receipt; `RenderReceipt` has a receipt and neither; the bounds report nothing. The driver
   "a contract that cannot express all three is already known to be wrong" is now evidenced
   rather than anticipated.
3. **There is a fourth mechanism**, and it is the one about result ownership: the GUI's analyze
   worker already discards a completed result whose target has since changed. That is
   stale-result behaviour nothing states, and this record owns stating it.
4. **A file sink can fail after every audio frame was accepted.** WAV output writes samples
   individually and finalizes afterwards, so finalization is a second failure point that a
   per-quantum contract cannot represent. Any streaming sink this record defines needs a
   completion operation, not only an accept.
5. **The callers that most need progress and cancellation are the aggregates**, not the single
   renders. Six MCP tools run a render per sweep step, track, effect prefix or bus, so their
   duration is a product rather than a bound and no per-render limit constrains it. A contract
   shaped only around one render would miss all six — and the four bus-shaped ones add a
   pinning requirement the sweeps do not, because a series of renders that do not see the same
   state is not comparable with itself.
6. **One caller already pins, and it is not the one that looks like it does.** GUI export takes
   an owned `ProjectFile` and builds a fresh engine from it, so a mid-render edit cannot reach
   it; the MCP analyzers, which read live engine and song state, are the ones that cannot say
   what they measured. What export's snapshot lacks is a revision identity — which is exactly
   the piece Phase 10A supplies and the reason this record waits for it.

**The evidence that remains missing** is what an MCP client does when a call takes minutes.
The bridge has no timeout, retry or progress channel, so the repository establishes that the
client blocks and can establish nothing about what it then does. That is a property of the
client, not of this tree, and no reading of this repository will supply it.

## Decision

**Deferred to Phase 10B**, with the owner, scope guard and evidence recorded in *The
deferral* above.

**The Phase 4 deadline is withdrawn, and `P04-R004` records why.** This record's
scope includes retention and stale-result labelling, and its exit requirement is a
*revisioned* contract. A project revision is the canonical document identity Phase 10A
creates, and capturing a job's immutable project/asset snapshot from one is the
revision-pinned job service Phase 10B builds. Phase 4 has neither, so a contract accepted
there would name a revision that does not exist; and an ADR has no partially-accepted
status, so deciding the render core alone would not be an acceptance. The deadline is
therefore **Phase 10B**, and all three standing constraints below hold until then.

The workflow analysis this record named as its missing evidence **has now been done**, in
Phase 4, and is recorded under *Evidence* below. What it establishes is that four
mechanisms exist rather than three, that a shared render request already exists and pins
nothing, and that the GUI's analyze worker already suppresses a stale result by target —
a rule nothing states.

Three constraints hold in the meantime, and all three hold until acceptance in Phase 10B:

1. **No new long-running synchronous surface** should be added to MCP or the CLI without recording that it will be
   migrated. The existing ones are a known cost; new ones are a growing one.
2. **No second progress or cancellation mechanism** may be built beside
   `ExportProgress`. A caller that needs one is evidence for this record and
   pulls acceptance forward rather than receiving a third bespoke channel.
3. **The pre-acceptance scope is closed, and it holds until Phase 10B:** pure lowering plus one bounded
   in-process smoke render. Multi-project A/B, a shared render request/result,
   streaming, progress, cancellation and frontend integration are refused as
   task selections until this record is accepted.

All three constraints hold until this record is accepted in Phase 10B. They do not block
Phases 1-3 or the closed initial Phase 4 scope, because none creates a job — and constraint 3
is what `P04-R004` relies on to keep Phase 4 from building the streaming surface under another
name.

## Consequences

### Positive

- The contract will be written against the callers that exist, which are already enumerated above, rather than against
  an imagined set.
- Phase 10B gets the decision exactly when it acquires its first real consumer, and the four adjacent decisions get to
  influence it first.

### Negative

- MCP keeps its blocking renders until Phase 10B, so an agent driving a long render still has no way to observe or cancel
  it.
- `ExportProgress` and `RenderReceipt` keep diverging in the meantime, and the eventual contract will have to absorb
  both rather than replace one.

### Risks and controls

- **Risk: a third bespoke mechanism appears** before the contract does, making the migration larger than the decision.
  Control: constraint 2, which turns the need into evidence instead of into code.
- **The deferral is now extended past Phase 4**, and `P04-R004` records the reason rather than
  leaving it as a risk that materialised: a *revisioned* contract needs an identity Phase 10A
  creates. **Phase 4 nevertheless exited**, on 2026-09-02, with `P04-R004` accepted as a named
  residual under `PROCESS.md`'s phase-exit rule and its gate bullet rewritten to ask that this
  phase build no streaming, progress, cancellation or shared render surface. So the control the
  original wording names — a phase that cannot exit — is replaced by that rewrite together with
  constraint 3, which is what still refuses the work as a task selection. The original wording
  follows, because what it predicted is what happened.
- **Risk: the deferral is extended through Phase 4** because A/B work can be
  done with blocking renders if one is patient. Control: multi-project A/B and
  every shared render-orchestration surface are outside the closed initial
  scope, and Phase 4 cannot exit while this record remains deferred.
- **Risk: the workflow analysis is skipped** and the record is written from this context section instead. Control:
  the register's basis names the analysis, and the first shared render-orchestration slice checks the basis, not the
  prose.

## Follow-up work

| Task                                                                              | Phase | Status      |
|-----------------------------------------------------------------------------------|-------|-------------|
| Workflow analysis of every render and analysis caller                             | 4 | **Done**, under *Evidence* |
| Establish what MCP clients do on a multi-minute synchronous call                  | 10B | Not started — and not obtainable from this repository, which has no timeout, retry or progress channel to observe |
| Write and accept this record against that analysis                                | 10B, once Phase 10A's revision exists | Not started |
| Decide whether `RenderReceipt` generalizes or is one instance of a receipt family  | 10B   | Not started — generalizing its serialized v1 meaning in place would be a protocol break needing explicit approval |

## Revisit conditions

Superseded by its own accepted version in Phase 10B, before the first
render-orchestration slice. Any earlier task that needs a job, shared render
request, streaming, progress or cancellation pulls the analysis forward rather
than improvising a mechanism.
