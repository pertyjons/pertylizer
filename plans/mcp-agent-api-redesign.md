# MCP agent API redesign

> **Status:** proposed; amended 2026-08-11 — the internal review's findings
> are folded into the body below (see Review history).
>
> **Planning baseline:** Pertylizer `79f22189`, 2026-08-11, with `rmcp 3.1.2`
> and MCP protocol revision `2026-07-28`.

## Objective

Replace the current method-per-action MCP surface with a smaller, coherent
agent API backed by one typed operation registry. The registry must be the
single source of truth for tool schemas, invocation, validation, capabilities,
direct replies, dry runs, and batched or transactional execution.

The redesigned API should make an agent's common workflows easier and cheaper:

- fewer tools to select between;
- strict, bounded, self-describing inputs and outputs;
- stable machine-readable errors and suggestions;
- atomic project edits by default;
- explicit concurrency and side-effect semantics;
- resources for documents and large data;
- tasks for long-running renders and analyses;
- no parsing of confirmation prose or result shapes to determine what happened.

This is an API replacement, not a compatibility migration. Active development
does not require preserving the existing tool names, prose, payload shapes, or
batch protocol.

## Why a redesign is needed

Structured output did not create the current complexity. It exposed that the
server has no single abstraction for an operation and its result.

At the planning baseline:

- `tools/list` contains 219 tools and measures approximately 804 KB: 451 KB of
  output schemas, 292 KB of input schemas, and 61 KB of descriptions;
- 116 mutation tools repeat the same approximately 1.1 KB mutation schema,
  accounting for 138 KB of the catalog;
- `BatchResult` and `MutationResult<T>` describe the same broad concept with
  different fields and different outcome rules;
- direct calls and `batch_execute` use different dispatch paths;
- `dispatch_tools!` repeats all registered tool names and parameter types
  because the rmcp route cannot validate parameters without invoking a handler;
- reply behavior is split between prose, `Json<T>`, and hand-built
  `CallToolResult` values;
- generic code recovers status from `isError`, `_meta`, payload predicates, or
  legacy text depending on the reply family;
- facts such as prose-only output, batch support, destructive behavior, and
  rollback scope live in separate lists or annotations;
- application concerns such as project snapshot fidelity and render errors are
  colored by MCP types.

Adding another result envelope or special-case list would make the existing
surface more internally consistent, but it would not remove these causes.

## Design principles

1. **Application operations are primary.** MCP is one adapter over an
   application API also usable by the GUI, CLI, tests, and future integrations.
2. **One registration produces every protocol surface.** A tool name, input
   type, output type, validator, capabilities, and implementation are declared
   once.
3. **Atomic project edits are the default.** Validate first, then commit the
   complete edit or commit nothing.
4. **Error and effect are orthogonal.** Error answers whether the operation
   went wrong; effect answers how much state changed.
5. **Use the protocol according to payload shape.** Tools perform bounded
   operations, resources carry documents and large state, and tasks represent
   long-running work.
6. **Optimize for agent decisions, not method count parity.** Group related
   edits behind a discriminated operation enum, but do not create one enormous
   catch-all tool whose schema is as difficult as the catalog it replaces.
7. **Make races explicit.** Project writes accept an expected revision and
   return the resulting revision.
8. **Keep essential semantics in structured content.** `_meta` may duplicate
   generic execution metadata but must not be the only place an agent can learn
   the result.
9. **Return identifiers as domain newtypes.** Do not introduce a generic ID or
   expose raw numeric domain identifiers.
10. **Measure agent ergonomics.** Schema validity is necessary but does not show
    whether an agent selects the right tool or completes a workflow efficiently.

## Non-goals

- Preserve existing MCP tool names or byte-identical prose.
- Preserve `batch_execute` or its rollback behavior.
- Make a generic remote procedure call surface for every internal method.
- Put complete module catalogs, project schemas, or manuals into tool
  descriptions or ordinary tool results.
- Depend on clients implementing dynamic tool-list notifications correctly.
- Move application or domain types into `synth_mcp`.
- Address every current MCP TODO independently before starting the redesign.

## Target architecture

```text
MCP tools / resources / tasks
              |
              v
      Tool definition registry
       |       |        |
       |       |        +--> schemas, annotations, catalog
       |       +-----------> decode, validate, dry-run
       +-------------------> direct and transactional dispatch
              |
              v
       Application operations
       |                    |
       +--> queries         +--> prepare + commit mutations
              |
              v
   Canonical project and runtime state
```

`synth_mcp` depends inward on the application layer. No application crate may
depend on an MCP request, response, scope, or error type.

## Application operation layer

Introduce an application-facing service or crate that owns use cases shared by
MCP, GUI, CLI, and tests. Exact crate placement should follow dependency-graph
constraints discovered during implementation; the important boundary is that
it is independent of rmcp and the wire format.

Size this honestly: the layer replaces a bridge trait of ~255 methods
implemented across ~14,500 lines inside the binary crate, and `synth_mcp`
cannot depend on `pertylizer` — so `Application` is a new crate that takes
ownership of (or defines traits over) the session, shared song, and sample
library. That extraction is the bulk of phases 1–2 and is where the schedule
risk lives.

Do not move the 255-method bridge unchanged into that crate. Migrate vertical
use-case slices: define one application operation, put its domain validation
and result there, adapt GUI/CLI/MCP callers, then remove the corresponding
bridge methods. Binary-crate code may temporarily implement narrow
application-layer ports while ownership is extracted. The disposition table
decides the slices and prevents the new crate from becoming the same
method-per-tool architecture under a different name.

The application layer owns:

- canonical read models for project, instrument, sequencer, mix, sample, and
  runtime state;
- query and mutation input types;
- domain validation and near-miss suggestions;
- project revisions and optimistic concurrency;
- project transactions;
- structured diagnostics and application errors;
- canonical project snapshots, including persistent UI metadata;
- long-running render and analysis jobs.

### Canonical project state

Persistent UI metadata must not exist only inside the GUI. Module positions,
groups, canvas metadata, visualizers, colors, and other persisted state should
have one canonical application owner that the GUI edits and every save path
reads.

`save_project`, bundle save, rollback snapshots, GUI save, and any future
autosave must call the same project snapshot service.

Size this honestly too: canonical ownership is a GUI refactor, not a
prerequisite chore. Positions, groups, and canvas metadata live in
`PatchEditor`, one per instrument, edited in an immediate-mode frame loop —
canonical ownership means write-through on every drag release, group edit, and
canvas change. Because the live data-loss bug must not wait for that
migration, Phase 0 ships the already-reviewed request-queue snapshot (TODO
§6.7: a queue of requests each carrying a one-shot reply channel, serviced
after `reconcile_with_session`, with a GUI timeout distinct from and longer
than the engine's `SNAPSHOT_SYNC_TIMEOUT_MS`, falling back to the engine
reconstruction when no GUI answers). Canonical ownership then replaces the
stopgap in the later phases; the stopgap must not shape the application model.

### Prepared mutations

Mutation execution has two stages:

```rust
pub(crate) trait ApplicationMutation {
    type Input;
    type Prepared;
    type Output;

    fn prepare(
        app: &Application,
        input: Self::Input,
    ) -> Result<Self::Prepared, Problem>;

    fn commit(
        app: &Application,
        prepared: Self::Prepared,
    ) -> Execution<Self::Output>;
}
```

The illustrative signatures may change for asynchronous operations, but the
semantic split is required:

- `prepare` performs the same domain validation for dry-run and real execution;
- `prepare` does not mutate project, runtime, device, or filesystem state;
- `commit` consumes a successfully prepared operation;
- project transaction tools prepare every operation before committing any;
- a failed staged project-state commit changes nothing; if the selected engine
  commit mechanism can still fail after publishing a subset, the operation
  reports `error + partial`. The MCP adapter never attempts snapshot rollback.

Serde deserialization is boundary validation, not a sufficient dry run.

### Atomicity against the engine command queue

"Commit the complete edit or commit nothing" must be designed against how
Pertylizer actually holds state. Project state is split: sequencer data
(`SharedSong`) is lockable and snapshotable, but the voice graph is mutated by
lock-free commands to the audio thread and mirrored back asynchronously — the
reason `apply_project`, the save barrier, and the `CommandSync::dropped`
counter exist. A command cannot be un-sent once published, but the redesign
must not assume that commands have to be published one at a time.

### Engine-batch design spike

Complete a bounded engine-batch spike before fixing the final partial-effect
contract. The current `CommandSender` already serializes every producer through
one mutex-protected ring producer, and the production ring holds 16,384
commands. Prototype an all-or-none `send_batch` that:

1. rejects a batch larger than the ring's total capacity;
2. acquires the shared producer lock once;
3. checks that the whole prepared batch fits;
4. atomically reserves every required deferred-drop return slot;
5. publishes control snapshots and enqueues every command without allowing
   another producer to interleave;
6. advances command-sync counters and returns a drain watermark for the whole
   accepted batch;
7. publishes nothing and releases every reservation if preflight cannot be
   satisfied.

While the producer lock is held, no rival producer can consume capacity and
the audio consumer can only *increase* vacancy by popping — so a capacity
check at lock acquisition is a lower bound, and a successful reservation makes
producer-side enqueue all-or-none. (Verified against today's `send`,
`synth_engine.rs:190`: the lock already spans the capacity check,
deferred-slot reservation, control-snapshot publish, push, and counter bump —
`send_batch` extends an existing invariant rather than inventing one.) A
queue-capacity failure would then be `error + none`, not `error + partial`.

All-or-none enqueue is not automatically audio-observable atomicity: the audio
thread may begin consuming the prefix while the producer is still publishing
the suffix. The spike must compare at least these stronger alternatives:

- one `EngineCommand::ApplyBatch` processed as a unit at a block boundary,
  with its pre-allocated command storage returned to the control thread for
  destruction rather than freed on the audio thread;
- a prepared immutable graph or control-state snapshot swapped at a block
  boundary;
- all-or-none adjacent enqueue, only if the engine's command-drain behavior can
  prove no audio processing observes an intermediate graph.

Only if no all-or-none mechanism is viable may queue failure remain an
`error + partial`; that fallback must name exactly which operations were
accepted. The decision and its real-time ownership consequences are recorded
before project edit tools depend on it.

Audit `synth_engine::transactions` as part of the spike. It currently declares
atomic command batches without a production executor. Reuse or redesign it if
its model fits the chosen mechanism; otherwise remove it so the engine does not
advertise a second, non-functional transaction abstraction.

Regardless of the selected queue mechanism:

- **Prepare hard enough that commit cannot fail for domain reasons.** Module
  types, ports, parameter names, ranges, and referenced ids are all checkable
  against descriptors and canonical control state without touching the engine.
- **Sequencer-side changes stage and swap atomically.** Engine-side changes are
  submitted only after the entire transaction has prepared. If engine batch
  submission fails before publication, the staged song is not swapped.
- **One application-level `ProjectMutationGate` serializes the commit window.**
  This is a new gate, not the `SharedSong` write lock. Every project writer —
  GUI, MCP, undo/redo, project apply/load, sample and mixer editing, and other
  integrations — must pass through it. Runtime-only actions such as notes,
  transport, monitoring, and metering do not take it.
- **The gate covers the final revision check, engine-batch
  reservation/submission, staged `SharedSong` swap, and revision assignment.**
  It is never held while awaiting audio-thread drain, task completion, file I/O,
  or another long-running operation.
- **`ProjectEditRevision` increments when a commit is accepted under the gate,**
  not when the audio thread finishes applying it. Paths that need applied state
  keep using the command-drain watermark.

### Project revision

Introduce an application-layer `ProjectEditRevision` newtype for optimistic
concurrency. It is distinct from `dirty::ProjectRevision`, the existing
composite of counters and fingerprints used only to detect unsaved changes
inside one GUI session. The dirty-state type may remain until canonical
ownership makes it possible to simplify; it is not published as the wire
concurrency token.

Every project mutation input accepts an optional `expected_revision:
ProjectEditRevision`. Every successful or partially effective project mutation
returns the observed resulting edit revision.

If the expected revision is stale, preparation fails with a structured
`revision_conflict` problem containing the current revision. This replaces the
need for a batch to predict which `mutation_seq` increments belong to it.

GUI-originated project edits must increment the same revision as MCP edits.
All project writers must therefore use `ProjectMutationGate`; merely bumping a
counter beside direct `EngineHandle::send` calls is insufficient.

Revision rules are:

- a complete state-changing commit increments exactly once;
- a no-op does not increment;
- a partial effect increments if any project mutation was accepted;
- accepted does not mean applied by the audio thread;
- a caller that needs applied state uses the returned command-drain watermark;
- revision assignment and command submission occur in the same gate-held
  commit window.

One consequence to decide in phase 1 rather than discover: GUI parameter drags
emit engine commands at gesture rate, so a gated per-commit revision advances
many times per second while a user turns a knob — and a concurrent
`expected_revision` edit conflicts for the entire gesture. Either accept that
(the project genuinely is changing under the caller) or coalesce a drag
gesture into one revision at gesture end; the choice sets how aggressively MCP
clients should retry `revision_conflict`.

## One execution protocol

Use one tagged result representation for direct calls and nested transaction
items. A representative shape is:

```rust
#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Execution<T> {
    Success {
        effect: Effect,
        data: T,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        diagnostics: Vec<Diagnostic>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<ProjectEditRevision>,
    },
    Error {
        effect: Effect,
        problem: Problem,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial_data: Option<T>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        diagnostics: Vec<Diagnostic>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        revision: Option<ProjectEditRevision>,
    },
}

#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Effect {
    None,
    Complete,
    Partial,
}
```

The final types must follow the project's newtype policy. `Effect` may be
renamed if a more precise domain term emerges, but its meaning must remain
separate from error status. One mechanical note from prior work: an internally
tagged generic enum needs `serde(bound(deserialize = "T: DeserializeOwned"))`
to derive `Deserialize`; this codebase has hit that once already.

### Required semantics

| Status | Effect | Meaning |
|---|---|---|
| success | none | A read completed, or an idempotent write required no change. |
| success | complete | The complete requested mutation landed. |
| success | partial | Some requested work was deliberately best-effort and warnings explain the omissions. |
| error | none | The request was rejected or failed before changing observable state. |
| error | partial | Execution failed after an irreversible or explicitly non-atomic partial effect. |

`error + complete` is not a valid combination. Project transactions should
normally produce only `success + complete`, `success + none`, or `error + none`.

The MCP adapter derives outer `isError` only from the tagged status. It never
infers status from text, counts, optional fields, or a result type's incidental
shape.

### Problems and diagnostics

Use stable typed codes rather than an arbitrary error string:

```rust
pub(crate) struct Problem {
    pub code: ProblemCode,
    pub message: String,
    pub path: Option<InputPath>,
    pub retryable: bool,
    pub suggestions: Vec<String>,
}

pub(crate) struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: DiagnosticCode,
    pub message: String,
    pub path: Option<InputPath>,
}
```

Messages remain useful to humans and models, but control flow keys on typed
codes and status. Near-miss module, parameter, track, and target suggestions
belong in `suggestions` rather than only in prose.

### Eliminate ambiguous operations

Where an existing method naturally reports a partial effect, first ask whether
the use case is actually two operations:

- Creating a pattern and applying its automation should validate both before
  creating either, unless the caller explicitly requests best-effort behavior.
- Saving note-graph source and activating compiled code are named separately
  as `save_script_draft` and `activate_script`. Not atomic-as-one: an atomic
  save-and-activate would lose a non-compiling source on failure, and
  always-saving the draft exists precisely so a broken script can be fixed and
  re-sent.
- A multi-item edit should default to all-or-nothing. Any `best_effort` option
  must be explicit in the input and reflected in the schema.

Do not preserve partial behavior merely because the current bridge happens to
mutate before it finishes validating.

## Typed tool definition registry

Define a Pertylizer-owned tool abstraction with associated input and output
types:

```rust
pub(crate) trait PertylizerTool {
    type Input: serde::de::DeserializeOwned
        + schemars::JsonSchema
        + Send
        + 'static;
    type Output: serde::Serialize
        + schemars::JsonSchema
        + Send
        + 'static;

    const NAME: &'static str;
    const DESCRIPTION: &'static str;
    const CAPABILITIES: ToolCapabilities;

    async fn execute(
        server: &SynthMcpServer,
        input: Self::Input,
    ) -> Execution<Self::Output>;
}
```

A generic `register::<T>()` builds an rmcp `Tool` and `ToolRoute` using the
associated types. It must explicitly attach `schema_for_input::<T::Input>()`
and `schema_for_output::<Execution<T::Output>>()`; output-schema publication
must not depend on a macro syntactically recognizing `Json<T>` in a return type.

Internally, the registry keeps erased closures for:

- decode and prepare without execution;
- execute;
- convert `Execution<T>` to the wire representation;
- return the complete capability record.

Both direct tool calls and any transactional dispatcher use those same
closures. rmcp's router is generated from the registry and is not itself the
application's operation registry.

### rmcp integration

Use rmcp 3.1.2 for:

- protocol models and transport;
- streamable HTTP and stdio serving;
- `ToolRouter` and dynamically built `ToolRoute` values;
- schema generation helpers;
- structured tool errors;
- `CallToolResponse` variants for tasks and multi-round-trip requests;
- tool-list cache hints and change notifications;
- cancellation and progress notification plumbing.

Do not base Pertylizer semantics on:

- `Json<T>` choosing a success result;
- return-type syntax inference for `outputSchema`;
- `CallToolResult::structured` deciding the domain verdict;
- `ToolRoute` providing a validation-only path;
- duplicated text and structured serialization being the only possible reply
  construction.

Build `CallToolResult` in one Pertylizer adapter so it can set a concise text
summary, structured payload, `isError`, and optional metadata consistently.

## Tool capabilities

Declare separate capability axes in every tool definition:

```rust
pub(crate) struct ToolCapabilities {
    pub reply_kind: ReplyKind,
    pub mutability: Mutability,
    pub side_effects: SideEffectSet,
    pub transaction: TransactionSupport,
    pub batch_support: BatchSupport,
    pub latency: LatencyClass,
    pub confirmation: ConfirmationPolicy,
}
```

At minimum, side-effect classification must distinguish:

- project state;
- runtime or transport state;
- filesystem output;
- recording state;
- audio-device state.

The registry derives standard rmcp annotations such as read-only, destructive,
and idempotent hints from these values. No second annotation list is allowed.

Project-atomic transaction tools may contain only operations whose complete
effect is covered by that transaction. Filesystem, device, transport, and
recording operations are rejected during preparation if included in such a
transaction.

## Proposed tool surface

The exact catalog must be validated with agent workflow tests. The initial goal
is approximately 20–40 tools and a catalog small enough to enter a model's
context without dominating it. Do not treat the number as a hard requirement
if splitting a confusing union materially improves tool selection.

### Discovery and inspection

- `search_module_types`
- `get_module_type`
- `inspect_project`
- `inspect_instrument`
- `inspect_sequence`
- `inspect_mix`
- `inspect_samples`
- `get_runtime_state`

Inspection tools should use selectors, limits, and cursors rather than return
the entire application state by default.

### Project editing

- `edit_patch`
- `edit_sequence`
- `edit_automation`
- `edit_mix`
- `edit_samples`
- `edit_project_metadata`

Each edit tool accepts an ordered array of a domain-specific tagged operation
enum. The complete request is prepared before commit and is atomic by default.
The reply contains one typed value per operation plus the new project revision.

For example, `edit_patch` may cover add/remove module, set parameter,
connect/disconnect, module metadata, and layout. `edit_sequence` may cover
patterns, notes, tracks, placements, note graphs, and mod graphs, but should be
split if its schema or agent selection behavior becomes unwieldy.

### Runtime and external effects

- `transport`
- `play_notes`
- `audio_input`
- `project_file`
- `preview_audio`

These do not participate in project rollback merely because they are tools.
Their capability metadata states the actual side-effect scope.

### Analysis and rendering

Group related analysis modes where their inputs and outputs are coherent, for
example:

- `analyze_music`
- `analyze_audio`
- `analyze_mix`
- `compare_audio`
- `render_audio`

Do not combine unrelated analyses solely to reduce the count. Use agent tests
and schema size to decide the boundary.

Long-running variants return MCP task handles and report progress. Large
results are stored as resources or artifacts and the final task result contains
a bounded summary plus resource links.

## Resources, documents, and large payloads

Use resources or resource templates for information whose value is the
document itself:

- project schema;
- YAMS reference;
- detailed module descriptors;
- example-patch documents;
- large project snapshots;
- full analysis reports;
- rendered files and other artifacts.

Representative URI families:

```text
synth://docs/yams
synth://schema/project
synth://module-types/{type_key}
synth://patches/{name}
synth://projects/current/snapshots/{revision}
synth://analysis/{analysis_id}
synth://renders/{render_id}
```

Tool results that create a large artifact return its typed identifier, concise
facts needed for the next decision, and a resource link. They do not embed the
artifact as both text and structured JSON.

Resource listings must be paginated where their size is not tightly bounded.
Completion remains useful for resource-template parameters but is not treated
as completion for tool arguments.

## Structured output rules

1. Every ordinary tool publishes an explicit `outputSchema` generated from its
   associated output type.
2. Every ordinary structured payload has an object root, even where the newest
   protocol permits another JSON root. Object roots allow future pagination,
   diagnostics, revisions, and links without another wire break.
3. Direct success and error replies both use `Execution<T>` and conform to the
   published schema.
4. `content` contains a short human-readable summary. It is not required to be
   byte-identical to legacy prose or a second copy of the full JSON value.
   This deviates from a spec SHOULD (serialized JSON in `content` for backward
   compatibility), so the adapter keeps a temporary evaluation switch for a
   full JSON text copy until decision 5 is settled against the actually
   supported clients. Phase 5 chooses one documented production behavior (or
   derives it deterministically from a negotiated protocol revision); it does
   not retain two open-ended wire profiles. `structuredContent` is identical
   during the evaluation regardless of the text-copy setting.
5. `structuredContent` contains the complete machine-readable result.
6. `_meta` may repeat generic effect or tracing information for clients but
   never carries the only copy of essential status or data.
7. Optional serialized fields use serde defaults consistent with their schema.
8. Request types reject unknown fields unless a concrete extensibility reason
   requires otherwise. Two boundaries on that strictness: `deny_unknown_fields`
   does not compose with `serde(flatten)`, so flattened request types need
   another mechanism or none; and field strictness must not become value
   strictness — the deliberate module-key leniency (short key, snake_case, or
   display name) stays.
9. Tagged enums use stable lowercase wire names and explicit variants.
10. Every listing is bounded and reports `total` or a cursor when truncated.
11. Every created object is returned using its existing domain ID newtype.
12. Schema doc comments contain only concise caller-facing guidance;
    implementation rationale uses ordinary comments or external documentation.

## Catalog and context budget

Add a CI measurement for a real stdio `tools/list` response. Record:

- complete serialized catalog bytes;
- input-schema bytes;
- output-schema bytes;
- description bytes;
- largest individual input and output schemas;
- tool count;
- repeated schema fragments worth investigating.

Budgets are set bottom-up, not aspirationally. Measured at the baseline:
input schemas total 292 KB (224 KB of it on mutators), and union tools
concatenate inputs rather than shrinking them — a naive `edit_sequence`
covering today's sequencer mutators is ~81 KB of input schema by itself. The
largest inputs are the note/mod-graph configs (`add_note_graph_module` 23.3 KB
and `set_note_graph_module` 22.9 KB, largely the same `NoteModuleConfig`), and
the current `$ref` inlining duplicates a definition at every use site, so
inside a union tool the same config inlines once per referencing operation.
The savings this plan does deliver are mostly output-side: the shared envelope
repeats ~30 times instead of 116 (an accepted cost), and the large analysis
reports move to resources.

Therefore:

- publish a per-tool schema estimate for the proposed surface **before**
  adopting numeric budgets, and keep separate input and output budgets so a
  miss is attributable;
- decide `$defs`-versus-inlining **for the new surface** as part of that
  estimate — "inlining is not the dominant cost" was measured across 219 small
  tools and does not transfer to union tools referencing the same 20 KB config
  repeatedly;
- treat 150 KB warning / 200 KB failure as provisional targets pending the
  estimate, then revisit after the first agent evaluation rather than
  weakening them merely to fit a ported tool.

The primary size levers are:

- fewer, domain-coherent tools;
- short descriptions;
- resources for manuals and documents;
- bounded result types;
- factoring or narrowing the note/mod-graph operation surface, which dominates
  the input side, without hiding fields a client needs to construct a valid
  request;
- avoiding a large generic mutation envelope on every tiny setter.

Do not remove schema detail required by real clients merely to reduce bytes.

## Tasks and confirmation

### Long-running tasks

Offline rendering and analyses that can exceed an ordinary client timeout use
the MCP tasks extension. A task must support:

- a stable task identifier newtype;
- progress notifications with bounded, meaningful phases;
- status polling;
- cancellation;
- a final `Execution<T>` result;
- resource links for large artifacts;
- cleanup or expiry semantics.

Cancellation must be propagated to application work rather than only changing
the task record.

Client support is verified, not assumed. Before any render or analysis moves
behind a task handle, confirm that the actually supported clients poll
`tasks/get` and surface progress — by exercising them, per §6's standing rule
of confirming a feature reaches the surface it is claimed for. Where a client
does not, keep an explicitly bounded synchronous operation or return a
structured error asking the caller to shorten the requested range. Do not let
an opaque runtime latency threshold make the same input unpredictably return a
complete result or a task. The execution mode or response family must be known
from the tool contract and explicit input before work starts.

### Destructive confirmation

Use standard destructive annotations for every destructive operation. Add
multi-round-trip confirmation only where the server itself must protect user
data, such as replacing a dirty project or overwriting a file.

Confirmation policy belongs in `ToolCapabilities`. A client that does not
support the relevant interaction must receive a structured error explaining
the required alternative; it must never cause an implicit destructive action.

## Agent-facing behavior

The redesign is successful only if an agent can reliably perform workflows,
not merely if schemas validate.

Every write response should include what the agent needs next:

- IDs of created objects;
- the resulting project revision;
- typed diagnostics and suggestions;
- resource links for large results;
- a concise summary suitable for a human activity log.

Avoid forcing an agent to:

- parse IDs from prose;
- call a full catalog listing before every edit;
- infer success from counters or optional values;
- guess whether a partial mutation survived;
- know which external side effects a generic rollback cannot restore;
- send a second inspection call merely to learn an ID the mutation created.

## Verification strategy

### Registry contract tests

- Every registry entry produces exactly one visible rmcp route.
- No duplicate name can be registered.
- Every visible route has capability metadata.
- Every structured route publishes the expected output schema.
- Disabled routes are absent from both listing and invocation.
- An explicitly unsupported transaction member is rejected during preparation.

### Schema and serialization tests

- Validate representative success and error payloads against each published
  output schema.
- Validate actual wire responses from each reply family against the schema from
  the same `tools/list` session.
- Check that serde and JSON Schema accept and reject the same generated corpus
  of representative arguments.
- Pin tagged enum names, domain ID representations, defaults, and rejection of
  unknown fields.
- Ensure every ordinary output schema has an object root.
- Ensure no empty or skipped field violates its own required-field contract.

### Operation semantics tests

- Dry-run and real execution use the same preparation result.
- A failed project edit commits nothing.
- A successful multi-operation edit commits everything once and returns one new
  revision.
- A stale expected revision changes nothing and returns the current revision.
- GUI and MCP edits increment the same revision.
- Explicit best-effort edits report every partial item without contradictory
  status fields.
- External effects cannot enter a project-only transaction.
- Project save and rollback preserve layout and other persistent UI metadata.

### Transport tests

Keep and adapt the real stdio JSON-RPC contract test. Cover:

- initialization and non-empty `tools/list`;
- direct structured success;
- direct structured tool error;
- invalid parameters;
- resource links;
- task creation, progress, completion, and cancellation;
- multi-round-trip confirmation where enabled;
- panic isolation;
- old protocol negotiation only if it remains an explicitly supported target.

### Agent workflow evaluation

Create repeatable evaluations for at least:

1. Search for suitable modules, build an instrument, connect it, and set its
   parameters.
2. Create patterns, notes, tracks, and placements for a short arrangement.
3. Inspect a mix, identify a problem, apply a fix, and verify the new analysis.
4. Make a multi-operation edit with one invalid reference and confirm that no
   partial project state lands.
5. Render a section asynchronously and retrieve the artifact.
6. Recover from a revision conflict without losing another client's edit.

Measure:

- completion rate;
- calls per successful workflow;
- invalid tool selections;
- invalid arguments;
- catalog/context bytes;
- tool-result bytes;
- recovery rate after structured errors.

Compare the redesigned surface with the current 219-tool surface before
removing the latter.

## Migration phases

### Phase 0: Fix independent correctness defects

- Fix the MCP save/rollback data loss now, via the reviewed request-queue GUI
  snapshot (see Canonical project state) — not by waiting for canonical
  UI-state ownership, which is phase 1–3 work. The stopgap is explicitly
  disposable.
- Move `AnalysisScope` and render errors out of `synth_mcp` where they cross
  into shared application code (`render/command.rs:11`, `render/wav.rs:7`).
- Keep the current real-wire tests green while the replacement is built.

**Exit:** an MCP save and a rollback restore preserve module layout and the
other persistent UI metadata (verified in the running app, not only in the
gate), and application rendering has no dependency on MCP error or request
types.

### Phase 1: Define the semantic foundation

- Add `ProjectEditRevision` and `ProjectMutationGate`, distinct from the
  existing dirty-state `ProjectRevision`.
- Route every project writer through the mutation gate in vertical use-case
  slices; do not move the existing 255-method bridge wholesale.
- Define `Execution<T>`, `Effect`, `Problem`, and `Diagnostic` in the application
  layer.
- Define preparation and transaction semantics.
- Convert the two known ambiguous cases: pattern plus automation creation, and
  script draft versus activation.
- Complete the engine-batch spike, audit or remove
  `synth_engine::transactions`, and select the strongest viable all-or-none
  submission/apply mechanism before fixing the queue-failure surface.
- Specify the remaining engine-queue commit rules from the atomicity section:
  prepare-hard validation coverage, gate scope, revision assignment, and drain
  watermark semantics.
- Add truth-table and atomicity tests before exposing new MCP replies.
- Produce the **disposition table**: every one of the 219 current tools mapped
  to a new tool/operation, a resource, a task, or an explicit drop. Tools with
  no obvious home in the proposed surface, to be placed first: `lint_project`,
  `suggest_music_fixes`, `generate_chord`, `create_chord_progression_pattern`,
  `quantize_*`, `auto_gain_stage`, `auto_layout`, `optimize_project`, example
  patches, `get_ui_snapshot`.
  Each row also records the new capability class, side-effect scope,
  synchronous/task behavior, estimated input/output schema size, migration
  test, and replacement documentation.
- Verify client task support (see Tasks and confirmation) so phase 3 does not
  move renders behind handles the supported clients cannot poll.

**Exit:** application operations can express ordinary success, no-change,
warning, rejection, and unavoidable partial error without strings or inferred
states; the disposition table is complete; all project writers use the
mutation gate; and the selected engine-batch and revision rules are implemented
and tested.

### Phase 2: Build the registry and wire adapter

- Implement `PertylizerTool` and generic registration.
- Generate rmcp routes, schemas, annotations, direct dispatch, validation, and
  capabilities from one definition.
- Implement the sole `Execution<T>` to `CallToolResult` adapter.
- Add catalog measurement and budgets.
- Prove the design with one query, one atomic mutation, and one long-running
  task.
- Build the agent workflow evaluation harness as a named deliverable — the
  phase 3 split/combine decisions, the budget revision, and the phase 4
  removal gate all key on its metrics, so it cannot be an activity started
  inside phase 3.

**Exit:** pilot tools require no parallel dispatch table, output-schema list,
reply-family switch, or status inspection; the evaluation harness runs against
the pilot tools.

### Phase 3: Introduce the smaller agent surface

- Implement discovery and inspection tools.
- Implement domain edit tools with tagged operation arrays.
- Add runtime and external-effect tools with correct capabilities.
- Move manuals, schemas, and large reports to resources.
- Move long renders and analyses to tasks.
- Run the agent workflow evaluation and split or combine tools based on
  evidence.

**Exit:** the target workflows complete against the new surface, catalog budgets
pass, and every mutation has explicit transaction and side-effect semantics.

### Phase 4: Remove the legacy MCP architecture

- Remove the old 219-tool catalog.
- Remove `batch_execute` and its snapshot rollback machinery.
- Remove `dispatch_tools!` and duplicate registration tests.
- Remove `BatchResult`, `BatchItemResult`, `MutationResult`, `MutationItem`, and
  `ToolOutcome` after their last application consumers migrate.
- Remove `stamp_outcome`, `reply_outcome`, payload-shape predicates, and
  prose-status conventions.
- Remove `PROSE_TOOLS`, `BATCH_EXEMPT`, and any separate capability lists.
- Update every document that actually names tools or describes their behavior,
  including CLAUDE.md's MCP section, `plans/TODO.md`, and notes describing
  `batch_execute`. `.mcp.json` and `.gemini/settings.json` need changes only if
  the endpoint, transport, timeout, or other connection settings change; they
  currently contain no tool names.
- Retain applicable real-wire, panic-isolation, cache, and transport tests.

**Exit:** every exposed operation is registered once and every tool result uses
the single execution protocol.

### Phase 5: Harden and document

- Document the final agent API through concise tool descriptions and resources.
- Record the supported MCP protocol revisions and client expectations.
- Add an rmcp-upgrade audit that checks only genuine SDK integration
  assumptions still present in the new adapter.
- Run all quality gates and the agent evaluation on the final catalog.

**Exit:** the new surface is the only supported MCP API and its agent metrics,
catalog size, and wire contracts are recorded.

## Code expected to survive the redesign

Preserve and adapt where useful:

- domain result and discovery structs that remain meaningful outside MCP;
- existing newtypes for module, instrument, pattern, track, note, graph, sample,
  and transaction identifiers;
- near-miss suggestion logic;
- real stdio wire-contract testing;
- schema validation tests;
- bounded search and listing behavior;
- panic isolation and structured logging;
- session tracking, list caching, completion for resource templates, and
  deterministic catalog order;
- offline render and analysis implementations after their MCP-colored inputs
  and errors are removed.

## Decisions to make during implementation

These decisions should be made with small prototypes and agent evidence rather
than in the abstract:

1. Whether `edit_sequence` remains one tool or splits note/mod graphs from song
   arrangement editing.
2. Whether cross-domain atomic project edits are common enough to justify a
   bounded `edit_project` transaction tool.
3. Which analysis families have coherent enough inputs and outputs to share one
   discriminated tool.
4. Whether the default catalog alone meets the context budget or optional tool
   profiles and `tools/list_changed` are worth the client-compatibility cost.
5. Whether short text summaries plus structured content work across the actual
   supported clients, and what single deterministic production rule replaces
   the temporary full-JSON evaluation switch.
6. Which destructive operations require server-driven confirmation beyond
   annotations.

None of these decisions changes the foundational requirements: one registry,
one execution protocol, application-owned semantics, and atomic project edits
by default.

## Completion criteria

The redesign is complete when all of the following are true:

- the default MCP catalog is generated from one registry and stays within the
  agreed context budget;
- no tool name or parameter type is repeated in a second dispatch table;
- every direct and nested result uses the same tagged execution protocol;
- no generic path infers status from prose, counters, or payload shape;
- every tool has complete capability metadata;
- project mutation dry-run and execution share preparation;
- every project writer uses the application-level mutation gate;
- optimistic concurrency uses `ProjectEditRevision`, not the GUI dirty-state
  fingerprint;
- the engine-batch mechanism has an explicit all-or-none boundary, with
  `error + partial` retained only for effects that can genuinely fail after
  publication;
- project edits are atomic by default and revision-aware;
- generic project rollback is no longer used to pretend external effects are
  reversible;
- large documents and artifacts use resources;
- long-running calls use tasks with progress and cancellation;
- all save paths preserve persistent GUI metadata;
- real wire replies validate against their advertised schemas;
- the defined agent workflow evaluations meet their agreed success and
  efficiency thresholds;
- the legacy `batch_execute`, result envelopes, special-case lists, and
  219-entry tool surface have been removed.

---

## Review history

Reviewed internally 2026-08-11 against baseline `79f22189`; verdict:
**adopt with amendments**. Commit `9074089b` carries the full review text. All
amendments are folded into the body above, where their verification detail now
lives:

- the engine command-queue atomicity section (review gap 1);
- the measured, split, bottom-up catalog budget (gap 2);
- Phase 0 re-scoped around the request-queue stopgap, with the application
  layer sized at ~255 bridge methods / ~14,500 lines (gap 3);
- the disposition table and client task-support verification as Phase 1 exits;
- the `save_script_draft` / `activate_script` split with the atomic
  alternative struck;
- the temporary evaluation switch for summary-only `content`, the
  unknown-fields boundaries, the evaluation harness as a named Phase 2
  deliverable, and the docs-that-name-tools migration in Phase 4.

A second review was folded in on 2026-08-11 after checking the proposed
atomicity rules against `CommandSender`, `SharedSong`, the existing dirty-state
revision, and the unused engine transaction module. It added:

- an engine-batch spike with all-or-none enqueue as the first target and
  block-atomic apply/snapshot swap as the stronger alternatives;
- the application-level `ProjectMutationGate` and its exact commit scope;
- `ProjectEditRevision` as a concurrency token distinct from
  `dirty::ProjectRevision`;
- the removal of the staged-commit rollback contradiction;
- vertical use-case migration rather than relocating the 255-method bridge;
- deterministic task fallback and a time-bounded content-copy evaluation;
- disposition-table columns for capabilities, effects, task behavior, schema
  cost, tests, and documentation;
- the correction that MCP connection files change only when connection
  settings do, not merely because tool names change.

The second review's engine claims were verified against the code on 2026-08-11
(`CommandSender`'s mutex-held send path at `synth_engine.rs:190`,
`CommandCapacity::DEFAULT = 16_384`, `DeferredDropSlots` reservation in `send`,
the executor-less `synth_engine::transactions`, `dirty::ProjectRevision` at
`dirty.rs:56`, and the tool-name-free connection files). All held. Two
adjustments came out of that pass: the capacity-monotonicity sentence in the
spike was reworded — the consumer frees capacity regardless of the producer
lock; what the lock buys is that vacancy only grows and no rival producer
consumes it — and the revision rules gained the GUI drag-gesture churn
decision.
