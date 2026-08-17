# ADR-0021: Host Profile and Admission Policy

| Field         | Value                                               |
|---------------|-----------------------------------------------------|
| ID            | ADR-0021                                            |
| Status        | Accepted                                            |
| Phase         | 0A                                                  |
| Created       | 2026-08-12                                          |
| Last reviewed | 2026-08-12                                          |
| Related       | P00A-T004, P00A-T005, P00A-T006, ADR-0001, ADR-0022, ADR-0037 |
| Supersedes    | —                                                   |
| Superseded in part | [ADR-0038](ADR-0038-engine-egress-queue-classification.md) replaces three named clauses: part 1's runtime-overflow paragraph, part 3's `LIMIT-0013` bullet, and the decision driver asserting that `LIMIT-0013` has per-priority drop counters published on OSC. ADR-0038 is `Accepted`; every other clause of this record still binds |
| Superseded by | —                                                   |

## Context

The [resource inventory](../inventories/resource-limits.md) records 74 fixed caps, truncation points, bounded queues,
buffer capacities, and script budgets at `dd69b657`. None has a disposition, because the inventory correctly treats the
`Proposed V2 rule` column as this ADR's output rather than its input.

The Phase 0A exit gate requires that every cap appear once in the inventory with a proposed V2 admission rule and a
user-visible overflow diagnostic, and that **no unexplained silent truncation is accepted as baseline behavior**. The
inventory's second audit pass compiled the register that clause needs: five sites where V1 silently loses something, of
which only `LIMIT-0013` counts what it dropped.

This ADR decides the classification scheme, the failure semantics for each class, and the disposition of those five
sites.

**Outside this decision.** Numeric defaults: P00A-T005 sets only `HostProfile`/render defaults once P00A-T003 has
measured the reference corpus; defaults owned by node, domain/format, job, application, or protocol contracts are set by
their respective specifications or ADRs. This record deliberately fixes none of them (see *Decision scope* below).
Also outside: the render quantum's semantics (ADR-0001) and its frame count (ADR-0037), hardware time mapping and
latency compensation (ADR-0022), observation and analyzer ownership as a product question (ADR-0027), and remote
authorization (ADR-0029).

### Decision scope, and why it excludes numbers

The register lists this ADR's required basis as "V1 cap inventory and measurements". The inventory is complete; no
measurement exists. Rather than block the Phase 0A gate on a benchmark, this ADR splits the topic along the line where
the evidence actually falls:

- **Which class a limit belongs to and what happens when it is exceeded** is determinable from the code that is already
  read. A compile-time array length is a hard capability whatever a benchmark says; a heuristic ceiling with a comment
  explaining the author's tradeoff is a budget.
- **What number to set a budget to** is not determinable from the inventory alone. P00A-T005 owns measured
  `HostProfile`/render defaults; every other number belongs to the specification or ADR for its configuration owner.

This is a scope decision, not an evidence waiver. Measurement may change owned defaults, but not the ownership and
failure semantics accepted here.

## Decision drivers

- The master plan's resource-and-admission section already states the intended principle: exceeding a hard capability is
  a compile or preparation error; approaching a configurable budget may be a warning or a configured refusal; runtime
  overflow is reserved for genuinely live bounded queues and is counted and reported; and it never silently changes
  authored topology, automation, sample mapping, note expansion, routing, or polyphony.
- A truncation that changes authored data is a correctness defect, not a resource policy. `LIMIT-0056` silently rewrites
  an authored voice count; `LIMIT-0067` silently discards authored rack stages at load.
- A diagnostic is only useful if it reaches the user. `LIMIT-0013` has per-priority drop counters published on OSC,
  which is closer to a telemetry channel than to a user-visible diagnostic.
- V1 conflates the document with the plan, which is why `LIMIT-0067` faced a false choice between dropping stages and
  refusing the load. V2 separates them, so the same situation has a third answer.
- A profile built from hardcoded constants is not a host profile. `LIMIT-0057` hardcodes the advertised buffer range on
  all three branches, including the one that successfully queried the device.

## Options considered

### Option A: Ratify the master plan principle and resolve each site against it

Classify every limit into the decision's six classes, bind a failure semantic to each class, and dispose of
the five silent-truncation sites individually. Costs the most decision work up front and forces several V1 behaviors to
change. Produces exactly the artifact the exit gate asks for.

### Option B: Fail-closed on everything

Any limit exceeded anywhere is a preparation error. Simple, uniformly safe, and unusable: it turns live bounded queues
into a source of hard failures under load, and would refuse to start a stream rather than drop one low-priority event.

### Option C: Preserve V1 behavior and only add diagnostics

Keep every current clamp and truncation, add a counter and a report to each. Cheapest, and it satisfies a literal
reading of "no *unexplained* silent truncation". It also permanently blesses silently rewriting an authored voice count
as correct behavior, which contradicts the master plan invariant that admission never changes authored polyphony.

### Status quo

No V2-specific decision. The inventory keeps 74 entries with no disposition, the Phase 0A exit gate cannot pass, and
Phase 1 has no rule for what to do when a plan does not fit, which in practice means the V1 answer — clamp and continue.

## Evidence

- The [resource inventory](../inventories/resource-limits.md) at `dd69b657`: 74 entries, the limit-class taxonomy, and
  the silent-truncation register.
- Site reads at `5cd24de8` confirming the five register entries: `voice.rs:1139` and `synth_engine.rs:4082-4086`
  (`LIMIT-0001`), `event_priority.rs:75-78` (`LIMIT-0013`), `state.rs:192,234-245` (`LIMIT-0020`),
  `types/audio.rs:470` (`LIMIT-0056`), `song.rs:881-887` (`LIMIT-0067`).
- The master plan's resource-profile and admission section, and the Phase 1 exit gate clause requiring that the renderer
  never silently clips the graph, event fan-out, voices, sends, or observation set to fit.

**Uncertainty that remains.** No value in the inventory has been measured; all 74 are read from source. Two audit
methods with opposite blind spots were run, but both are searches — a truncation that is both unnamed and undocumented
would still be missing from the register, and closing that needs an executable probe rather than a third pass. This ADR
therefore decides policy over a register that is thorough but not proven complete.

## Decision

Accepted. Class semantics, configuration ownership, and failure behavior are decided here; numeric defaults remain with
the owning contracts identified above. Four parts.

**Review history.** An earlier revision was marked `Accepted` with a single-axis class model that placed every
configurable budget in `HostProfile`. Review found that the ledger contains budgets which are not render-preparation
inputs at all — undo depth and retained audio, the autosave debounce, and the MCP reply caps — so the model could not
be applied to the inventory it governs, and the accompanying claim that every non-queue limit is known at plan
preparation was false for them. The acceptance was withdrawn and part 1 now separates failure behavior from
configuration owner. Three further defects from that review are fixed: the record hardcoded `64` for the quantum while
ADR-0037 was still `Proposed`; its `HostProfile` field list contained a render quantum that ADR-0001 forbids; and its
oversized-callback disposition said only "serve what the carry buffer holds", which left part of a real-time output
buffer unwritten and said nothing about the excess input.

A second review pass then found the owner axis still not exhaustive — node and DSP contracts, representation-level
bounds, and removed artifacts had no owner — so part 1's owner list is now a closed set of seven. The acceptance review
added the missing failure class for explicitly lossy retention and presentation budgets, assigned numeric defaults to
their actual owners, and made an oversized callback a terminal stream-contract fault rather than an ambiguous partial
recovery.

### 1. Two independent axes

Every entry in the resource inventory is assigned a value on **each** of two axes. Passes 1-2 and the first draft of
this ADR collapsed them into one, which produced a model that could not be applied to the ledger it was written for:
undo depth (`LIMIT-0063`), retained undo audio (`LIMIT-0064`), the autosave debounce (`LIMIT-0066`), and the MCP reply
caps (`LIMIT-0068`..`LIMIT-0071`) are all budgets, and none of them belongs in a render preparation input. The axes are
orthogonal: configuration ownership never determines failure behavior.

**Axis 1 — failure behavior**, which the limit class determines:

| Class                               | Behavior when exceeded                                                        |
|-------------------------------------|-------------------------------------------------------------------------------|
| `Platform capability`               | Compile or preparation error with an attributable resource diagnostic         |
| `Configurable safety budget`        | Configured refusal or warning, per an explicit policy field; never truncation |
| `Lossy retention/presentation budget` | Explicit eviction or omission of non-project-authoritative history, recovery, telemetry, or presentation data; always expose the loss or continuation mechanism |
| `Warning threshold`                 | Reported; never blocks                                                        |
| `Implementation artifact to remove` | Entry closes when the removing change lands                                   |
| `Unknown`                           | Not terminal; must be resolved before the Phase 0A exit review                |

**Axis 2 — configuration owner**, which says where the value is declared and who may change it:

| Owner                    | Holds                                                              | Example                                  |
|--------------------------|--------------------------------------------------------------------|------------------------------------------|
| `HostProfile`            | Render preparation capacity and budgets                            | voices, nodes, taps, buffer bytes        |
| Node contract            | A capacity intrinsic to one node's DSP, declared by the node        | `LIMIT-0072`, `LIMIT-0073`, `LIMIT-0074` |
| Domain/format contract   | A bound that is part of a value's representation or identity        | channel layouts, id encodings (ADR-0014) |
| Job policy               | Bounds of a long-running render or analysis job                    | offline render limits (ADR-0028)         |
| Application settings     | Editor and session budgets, unrelated to any render plan           | `LIMIT-0063`, `LIMIT-0064`, `LIMIT-0066` |
| Protocol contract        | Caps on a reply or message, owned by the surface that serializes it | `LIMIT-0068`..`LIMIT-0071`               |
| `N/A — removed`          | Nothing; the limit ceases to exist in V2                            | every `Implementation artifact to remove` |

The owner list is closed: every inventory entry must take exactly one of these seven, and an entry that fits none is a
finding about this table rather than a reason to leave the cell blank. `N/A — removed` is the only owner that pairs
with the `Implementation artifact to remove` class, and pairing any other class with it is an error.

The lossy class is deliberately narrow. It may bound undo/history retention, recovery generations, recent-item lists,
telemetry rings, or summaries whose complete data remains available through pagination or a detail surface. The owner
must expose an evicted/omitted count, a continuation marker, or an equivalent user-visible diagnostic. It may never be
used for canonical project data, authored topology, render input, automation, routing, sample mapping, or polyphony.

Only `HostProfile`-owned entries participate in plan admission and appear in the `ResourceReport`. **A node contract
declares its capacity into that admission** — a node's intrinsic ceiling is reported at compile time and contributes to
the `ResourceReport`, but it is not a profile field the host may raise. The remaining owners carry the same failure
semantics within their own boundary.

**Runtime overflow** is permitted **only** for genuinely live bounded queues — those fed by external,
unbounded-in-time input such as MIDI or user gestures. Every such queue counts its drops and surfaces the count in the
structured diagnostics report. A prepared plan may not exceed a `HostProfile` limit at runtime. If an external host
violates its negotiated maximum callback size, that is instead the terminal stream-contract fault defined in part 3;
the stream does not attempt to continue on a discontinuous timeline. Limits under the other owners are enforced at
their own admission, retention, or presentation boundary rather than at plan preparation.

### 2. Admission never rewrites authored data

A limit may refuse a plan. It may never silently change authored topology, automation, sample mapping, note expansion,
routing, or polyphony to make the plan fit. Where V1 does so today, the V2 behavior is a diagnostic and a refusal, not a
quieter clamp.

### 3. Disposition of the five silent-truncation sites

- **`LIMIT-0001` — oversized audio block.** `maximum_block_size` becomes a `HostProfile` field established at stream
  preparation from the queried device capability. Under ADR-0001 the renderer serves caller blocks from carry buffers
  and only ever processes whole quanta of `Q` frames, so an oversized callback cannot resize a buffer on the audio
  thread; the reallocation path is removed rather than guarded.

  A callback of `N > maximum_block_size` is a **terminal, counted stream-contract fault with fully specified output**,
  because a real-time callback may not be left partly unwritten and a partially retained input block cannot preserve the
  engine-input `SampleTime` epoch:

    1. **Every sample of this and every subsequent output callback is written as silence** until the stream is prepared
       again. There is no path that returns with samples untouched or serves stale carry data after the fault.
    2. **The entire input block is discarded**, both carries are invalidated, and no later callback is concatenated onto
       a partial earlier block. The engine makes no claim that the old input epoch continues across the fault.
    3. **An atomic `needs_reprepare` state is published.** Recovery requires host-side stream deactivation and
       preparation, which establishes fresh carries, capacity, and epoch before rendering resumes.
    4. **Nothing is allocated and no quantum is rendered.** Atomic counters for oversized callbacks, discarded input
       frames, and silence-filled output frames reach the structured diagnostics report.

  Processing the oversized callback in bounded chunks or attempting partial recovery was considered and rejected: both
  risk joining input across a missing interval, and chunking puts more work into a callback that has already exceeded its
  budget. The terminal fault loses more immediate audio than a partial serve, but it preserves real-time bounds and never
  presents a discontinuous stream as timeline-correct.
- **`LIMIT-0013` — prioritized event rings.** Correctly classified as a live bounded queue; it keeps runtime dropping.
  The existing per-priority counters are promoted from an OSC-only publication to a field of the structured diagnostics
  report, so a dropped event is visible to a user who is not listening on OSC. Ring sizes stay `HostProfile` budgets
  whose measured defaults P00A-T005 owns; ADR-0027 continues to own what the taps are *for*.
- **`LIMIT-0020` — meter slots beyond 128.** Observation taps become a `HostProfile` field. A plan requesting more taps
  than the profile allows is a **compile error** naming the requested and available tap counts. Silently dropping a
  meter is withdrawn; the Phase 1 exit gate already forbids clipping the observation set to fit.
- **`LIMIT-0056` — voice count clamped to `[1, 128]`.** Polyphony is authored data, so clamping is forbidden by part 2.
  A voice count outside the profile range is a **preparation error**. `VoiceCount` construction becomes fallible rather
  than clamping, and the `new_unchecked` path that only `debug_assert!`s — and therefore accepts an out-of-range value
  in a release build — is removed.
- **`LIMIT-0067` — note-processor rack stages beyond 32, dropped at load.** V1 faced a false choice because the document
  and the plan are the same thing; the comment records that a hard error was rejected for blocking the load. V2
  separates them: **Project Format V2 imposes no stage cap and the document loads with every authored stage intact**,
  while plan compilation refuses a rack that exceeds the node budget with a diagnostic naming the rack and its stage
  count. The project stays loadable and editable; only rendering it is refused. Silent dropping is withdrawn.

### 4. `HostProfile` field set

The initial contract covers the areas the master plan lists: maximum host block; channel layouts and sample-rate range;
nodes, voices, channels, buses, sends, event fan-out, and delayed events; parameter, control, and event slots and
observation taps; prepared immutable bytes, mutable state bytes, buffer and scratch bytes, and the crossfade/retirement
budget; YAMS instructions, state, and emits multiplied by scope and polyphony; and recording-result and real-time
communication capacities.

**The render quantum is not a `HostProfile` field.** The earlier master-plan list opened with "maximum render quantum
and host block", but ADR-0001 clause 1 makes the quantum a compile-time constant with no configuration surface — the two
could not both hold. This record defers to ADR-0001, which owns the quantum, and drops the field. The Phase 0A work item
and `HostProfile` field list now name only the maximum host block, and `RenderConfig` no longer duplicates either the
quantum or `maximum_block_size` outside `HostProfile`.

`HostProfile` is an immutable preparation input, never a set of globals the renderer reads. Capability fields are
established from queried host and device capability, not from hardcoded constants — `LIMIT-0057`, which discards the
device's own reported buffer range on the branch that successfully queried it, is the anti-pattern this clause exists to
forbid. Compilation returns a `ResourceReport` with requested, available, and dominant contributors for every field.

## Consequences

### Positive

- The Phase 0A exit gate's silent-truncation clause becomes satisfiable: each of the five sites has either a diagnostic
  or an explicit decision, and four of the five change behavior rather than being blessed.
- Phase 1 gains a rule for what to do when a plan does not fit, which is currently undefined.
- The inventory's `Proposed V2 rule` and configuration-owner columns become fillable from one common policy rather than
  one argument per entry.
- `LIMIT-0067`'s resolution demonstrates the document/plan split paying for itself on a real V1 dilemma.

### Negative

- Four V1 behaviors change, and three of them can refuse work that V1 accepted: an out-of-range voice count, an
  over-tapped plan, and an over-long note-processor rack. Projects that relied on the clamp will surface errors.
- Making `VoiceCount` fallible touches every construction site.
- Deciding policy over an unmeasured register means a budget could be classified correctly and defaulted badly; the
  relevant owning contract must still establish its number from evidence.

### Risks and controls

- **Risk: the register is incomplete**, so an undocumented and unnamed truncation is silently accepted as baseline after
  all. Control: the executable probe named in the follow-up table — oversized blocks, more than 128 metered channels,
  more than 32 rack stages — which converts the register's weakest claim into a test.
- **Risk: the terminal host fault becomes a counter nobody reads**, repeating `LIMIT-0013`'s OSC-only situation.
  Control: the fault counters and `needs_reprepare` state must reach the structured diagnostics report, which is the
  report the exit review inspects.
- **Risk: refusals appear on real user projects at cutover.** Control: the refusal cases are enumerable from the
  inventory before Phase 12, and the reference corpus is the place to find out.

## Follow-up work

| Task                                                                                               | Phase | Status      |
|----------------------------------------------------------------------------------------------------|-------|-------------|
| Fill failure class, configuration owner, rule, and diagnostic for all 74 inventory entries              | 0A | Not started |
| Add a `Configuration owner` column to the resource inventory, with the seven-value closed set       | 0A    | Not started |
| Executable truncation probe: oversized blocks, >128 metered channels, >32 rack stages              | 0A    | Not started |
| Set measured `HostProfile`/render defaults in P00A-T005; route every other default to its owning contract | 0A | Not started |
| Resolve every `Unknown`-class entry to a terminal class                                            | 0A    | Not started |
| Remove the duplicate `maximum_block_size` field from `RenderConfig`                                | 0A    | Complete    |
| Make `VoiceCount` construction fallible and remove the release-build `new_unchecked` hole          | 6     | Not started |
| Build capability fields from queried device capability, retiring `LIMIT-0057`'s hardcoded range    | 9     | Not started |

This ADR is accepted, so the inventory class sweep is unblocked. The first follow-up row assigns **both** axes to every
entry, not just the failure class; entries remain `Investigating` until their rule and supporting evidence are recorded.

## Revisit conditions

- The executable probe finds a silent truncation that is neither in the register nor covered by a class rule, which
  would mean the class taxonomy is incomplete rather than merely the register.
- A live bounded queue is found that cannot count its drops without violating real-time safety.
- Measurement shows that a limit classified here as a configurable budget is in fact a hard platform capability, or the
  reverse.
