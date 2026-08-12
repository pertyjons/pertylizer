# Pertylizer Core V2 Decision Register

This is the canonical register for V2 architecture and product decisions. It answers whether a decision is still open;
individual files under
[decisions/](decisions/README.md) explain context, alternatives, evidence, the decision, and consequences.

The topics below originate in Part VII of the
[master plan](master-plan.md#part-vii-open-decisions), where each carries the same identifier used here. Registration
does not accept a decision. All initial entries remain `Proposed` until their individual ADR has been reviewed and
accepted.

Next free identifier: `ADR-0038`.

## Status vocabulary

- `Proposed` — open and under consideration;
- `Accepted` — authoritative for implementation;
- `Rejected` — considered and explicitly not selected;
- `Deferred` — intentionally postponed with a named revisit condition;
- `Superseded` — replaced by another ADR.

Only an accepted ADR is an implementation constraint. A likely choice or text in a discussion is not an accepted
decision.

## Register

| ID       | Topic                                       | Status   | Target phase          | Required basis                           |
|----------|---------------------------------------------|----------|-----------------------|------------------------------------------|
| ADR-0001 | Render quantum semantics and splitting      | Accepted | 0A/1                  | Partition-invariance requirement         |
| ADR-0002 | Internal channel layout                     | Proposed | 2                     | Design review and measurement            |
| ADR-0003 | Event segmentation API                      | Proposed | 3                     | Prototype and timing tests               |
| ADR-0004 | Native node representation                  | Proposed | 2/5                   | Benchmark and ergonomics review          |
| ADR-0005 | Buffer liveness strategy                    | Proposed | 2                     | Correctness first, profiling later       |
| ADR-0006 | Parameter ramp representation               | Proposed | 3/5                   | Automation tests                         |
| ADR-0007 | Parameter modulation laws                   | Proposed | 5                     | Existing-parameter inventory             |
| ADR-0008 | YAMS state identity and reload policy       | Proposed | 7                     | Prototype and state tests                |
| ADR-0009 | Plan-swap crossfade and latency             | Proposed | 9                     | Listening and CPU tests                  |
| ADR-0010 | Compatible node-state migration surface     | Proposed | 9                     | Multi-module prototype                   |
| ADR-0011 | Shared V1 instrument conversion             | Proposed | 10                    | Fixture conversion review                |
| ADR-0012 | Automation conflict policy                  | Proposed | 10                    | Product semantics and diagnostics review |
| ADR-0013 | Project/session/settings boundary cases     | Proposed | 0B/10A                | State-ownership inventory                |
| ADR-0014 | Persistent ID generation and encoding       | Proposed | 0B/10A                | Identity inventory and format review     |
| ADR-0015 | History representation                      | Proposed | 10C                   | Prototype and memory measurements        |
| ADR-0016 | Known-version unknown-field policy          | Proposed | 0B/10D                | Format safety review                     |
| ADR-0017 | Asset identity and external references      | Proposed | 0B/10D                | Asset workflow analysis                  |
| ADR-0018 | Editor metadata persistence scope           | Proposed | 0B/10A                | State-ownership inventory                |
| ADR-0019 | Remote mutation history semantics           | Proposed | 10B/10C               | Operation conformance review             |
| ADR-0020 | Final crate boundaries and names            | Proposed | After vertical slices | Dependency evidence                      |
| ADR-0021 | Host profile and admission policy           | Accepted | 0A/1                  | V1 cap inventory                         |
| ADR-0022 | Hardware time mapping and latency ownership | Proposed | 0A/3/9                | Simulated-host evidence                  |
| ADR-0023 | Same-sample session event ordering          | Proposed | 3                     | Deterministic scenario tests             |
| ADR-0024 | Recording take and commit semantics         | Proposed | 0B/9/10B              | Workflow and failure analysis            |
| ADR-0025 | Tuning representation and ownership         | Proposed | 0B/6/10A              | Format and pitch-path review             |
| ADR-0026 | Minimum SampleMap and SampleZone model      | Proposed | 6/10A/10D             | Sampler migration analysis               |
| ADR-0027 | Observation and analyzer ownership          | Proposed | 0B/5/9                | Resource and protocol analysis           |
| ADR-0028 | Long-running job contract                   | Proposed | 0A/4/10B              | Render/analysis workflow analysis        |
| ADR-0029 | Host configuration and remote authorization | Proposed | 0B/10E                | Deployment and threat review             |
| ADR-0030 | Public facade and compatibility surface     | Proposed | Before 10E            | Consumer inventory                       |
| ADR-0031 | Supported build and release matrix          | Proposed | 0B/12                 | CI and consumer inventory                |
| ADR-0032 | Sample-time and event-timestamp model       | Proposed | 0A/3                  | Range analysis and timing tests          |
| ADR-0033 | Graph feedback and delay-boundary rule      | Proposed | 2/3                   | Compiler prototype and cycle cases       |
| ADR-0034 | Track, source, and channel ownership        | Proposed | 0B/10A                | Product workflow and V1 track audit      |
| ADR-0035 | Transaction and concurrency semantics       | Proposed | 0B/10B                | Operation conformance corpus             |
| ADR-0036 | Audio device and input lifecycle            | Proposed | 0B/9                  | Simulated-host and platform review       |
| ADR-0037 | Render quantum frame count                  | Proposed | 0A/1                  | Benchmark                                |

### Records created

Topics without a link below have no individual record yet; the table above is still authoritative for their status.

- [ADR-0001: Render quantum semantics and splitting contract](decisions/ADR-0001-internal-render-quantum.md) —
  `Accepted`
- [ADR-0021: Host profile and admission policy](decisions/ADR-0021-host-profile-and-admission-policy.md) — `Accepted`
- [ADR-0037: Render quantum frame count](decisions/ADR-0037-render-quantum-value.md) — `Proposed`

ADR-0001 and ADR-0021 were accepted after three review passes. Each carries a *Review history* note recording the
defects corrected before acceptance; the immutability rule in [decisions/README.md](decisions/README.md) now applies.

### Registered splits

ADR-0037 is not a Part VII topic of its own. It splits the master plan's topic 1 (internal render quantum) so that the
measurement-dependent frame count does not block the semantics, which are decidable now. Both identifiers are required
`Accepted` by the Phase 0A exit gate, so the split does not weaken it. The Phase 0A tracker records the deviation.

`Target phase` lists investigation, implementation, and later verification milestones; it is not permission to defer a
decision through the last listed phase. Explicit entry and exit gates in the master plan define the acceptance deadline.
In particular, a target beginning with `0A` or `0B` places the ADR in that sub-phase tracker even when later phases
refine or verify it.

## Register maintenance

When work begins on an entry:

1. copy [templates/adr.md](templates/adr.md) to
   `decisions/ADR-NNNN-short-title.md`;
2. add links to relevant inventories and evidence;
3. keep the register status synchronized with the individual ADR;
4. if accepted, update affected specifications and phase tasks;
5. if superseded, retain the old ADR and link both directions.

An ADR may cover several tightly coupled register entries only when they cannot be decided independently. If so, retain
all identifiers as aliases in its metadata and explain the coupling.
