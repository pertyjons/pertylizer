# Pertylizer Core V2 Decision Register

This is the canonical register for V2 architecture and product decisions. It answers whether a decision is still open;
individual files under
[decisions/](decisions/README.md) explain context, alternatives, evidence, the decision, and consequences.

The topics below originate in Part VII of the
[master plan](master-plan.md#part-vii-open-decisions), where each carries the same identifier used here. Registration
does not accept a decision. All initial entries remain `Proposed` until their individual ADR has been reviewed and
accepted.

Next free identifier: `ADR-0042`.

## Status vocabulary

- `Proposed` — open and under consideration;
- `Accepted` — authoritative for implementation;
- `Rejected` — considered and explicitly not selected;
- `Deferred` — intentionally postponed with a named revisit condition;
- `Superseded` — replaced by another ADR.

A record may also carry **`Superseded in part`** in its own metadata while keeping its register status. That is not a
status: it is a pointer to a successor that replaced named clauses, and every replaced clause is listed in the
successor's `Supersedes` field. The rest of the record still binds, which is why the register status does not change.
**The replacement itself takes effect only when the successor is accepted** — until then the pointer says a
replacement exists, and the old clauses are still the authority.
See [decisions/README.md](decisions/README.md#decision-lifecycle) for when this is the wrong tool.

Only an accepted decision is an implementation constraint. A likely choice or text in a discussion is not an accepted
decision.

## Decision classes

Every entry in this register is one of two classes. The class decides how much apparatus the decision gets, not how
binding it is: an accepted decision of either class is authoritative for implementation.

- **`Contract`** — the default. Gets an individual record under [decisions/](decisions/README.md), with options,
  tradeoffs, evidence, and consequences, and is accepted there.
- **`Reversible`** — a *value* whose later change costs a rebuild and nothing else. Gets a row in the
  [reversible-decisions table](#reversible-decisions) and no file.

### The reversibility test

An entry is `Reversible` only when **all four** hold. State each in the entry; an entry that cannot state test 3 in one
sentence is a `Contract` decision.

1. **Value, not shape.** It picks a number, a default, or a threshold. It does not define a type, a field, a contract
   clause, an ownership boundary, or an error behavior.
2. **Nothing persists it.** No serialized field, file-format field, protocol message, or public API signature carries it
   or is sized by it. A value a saved file records is a migration to change, not a rebuild.
3. **Reversal is a rebuild.** Changing it later costs recompilation and re-measurement — not a superseding contract, not
   a data migration, not a re-review of everything that cited it.
4. **A named revisit point exists.** A phase gate, or a stated condition. Without one, "provisional" means "forgotten",
   and the entry is a `Contract` decision that has not been made yet.

### What a reversible entry must carry

The chosen value; one or two sentences of why, including what would change it; the revisit point; and this standing
restriction, which is what keeps the value reversible in practice rather than only on paper:

> Until its revisit point, nothing may be tuned to the value — no hand-unrolled kernel, no layout sized by it, no test
> asserting it as a constant.

What the fast path removes is the individual file, the options-and-tradeoffs survey, an acceptance rule table fixed
before measurement, and the requirement that evidence exist *before* acceptance. Evidence moves to the revisit point,
where it is measured against the thing being built rather than against a proxy for it.

### Guards

- **Reclassification is one-way in practice.** If a reversible value is cited as a reason for another design decision,
  it has stopped being reversible: promote it to a `Contract` record, keeping its identifier, before that decision is
  accepted.
- **A gate may not lean on a reversible entry for a contract question.** Where a gate needs semantics settled, the
  semantics are a separate `Contract` record. ADR-0001 and ADR-0037 are that split done by hand; this class makes it
  the default shape rather than a deviation.
- **`Reversible` is not `unreviewed`.** The value binds implementation the moment it is accepted, and a wrong one is
  corrected in the register, not ignored.

ADR-0037 (render quantum frame count) is the case that motivated this class, and it would qualify under all four tests.
It is not reclassified: it is `Accepted` and therefore immutable apart from spelling and links, and rewriting an
accepted record to match a later workflow would destroy the reasoning the record exists to preserve.

No topic has been swept for reclassification. Judge the class when work begins on an entry, not in bulk.

## Register

| ID       | Topic                                       | Status   | Target phase          | Required basis                           |
|----------|---------------------------------------------|----------|-----------------------|------------------------------------------|
| ADR-0001 | Render quantum semantics and splitting      | Accepted | 0A/1                  | Partition-invariance requirement         |
| ADR-0002 | Internal channel layout                     | Accepted, superseded by ADR-0041 on that record's acceptance | 2 | Design review and measurement (EVD-0008, then EVD-0010) |
| ADR-0003 | Event segmentation API                      | Proposed | 3                     | Prototype and timing tests               |
| ADR-0004 | Native node representation                  | Accepted | 2/5                   | EVD-0009 benchmark; redrafted under its own rule C |
| ADR-0005 | Buffer liveness strategy                    | Accepted; clauses 1, 2, 4, 5, 7 and 8 superseded or refined by ADR-0041 on its acceptance | 2 | Correctness first, profiling later |
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
| ADR-0022 | Hardware time mapping and latency ownership | Deferred | 0A/3/9                | Simulated-host evidence                  |
| ADR-0023 | Same-sample session event ordering          | Proposed | 3                     | Deterministic scenario tests             |
| ADR-0024 | Recording take and commit semantics         | Proposed | 0B/9/10B              | Workflow and failure analysis            |
| ADR-0025 | Tuning representation and ownership         | Proposed | 0B/6/10A              | Format and pitch-path review             |
| ADR-0026 | Minimum SampleMap and SampleZone model      | Proposed | 6/10A/10D             | Sampler migration analysis               |
| ADR-0027 | Observation and analyzer ownership          | Proposed | 0B/5/9                | Resource and protocol analysis           |
| ADR-0028 | Long-running job contract                   | Deferred | 0A/4/10B              | Render/analysis workflow analysis        |
| ADR-0029 | Host configuration and remote authorization | Proposed | 0B/10E                | Deployment and threat review             |
| ADR-0030 | Public facade and compatibility surface     | Proposed | Before 10E            | Consumer inventory                       |
| ADR-0031 | Supported build and release matrix          | Proposed | 0B/12                 | CI and consumer inventory                |
| ADR-0032 | Sample-time and event-timestamp model       | Accepted | 0A/3                  | Range analysis and timing tests          |
| ADR-0033 | Graph feedback and delay-boundary rule      | Proposed | 2/3                   | Compiler prototype and cycle cases       |
| ADR-0034 | Track, source, and channel ownership        | Proposed | 0B/10A                | Product workflow and V1 track audit      |
| ADR-0035 | Transaction and concurrency semantics       | Proposed | 0B/10B                | Operation conformance corpus             |
| ADR-0036 | Audio device and input lifecycle            | Proposed | 0B/9                  | Simulated-host and platform review       |
| ADR-0037 | Render quantum frame count                  | Accepted | 0A/1                  | Benchmark                                |
| ADR-0038 | Engine-egress queue classification          | Accepted | 0A/1                  | Use-site audit (EVD-0005)                |
| ADR-0039 | Initial multi-client hub omission             | Proposed | 0A/10E                | Public-surface inventory, bounded use-site evidence, and independent review |
| ADR-0040 | V2 owns its DSP                             | Proposed | 2                     | Implementation reading of both engines, plus independent review of the extraction plan |
| ADR-0041 | Interleaved internal channel layout          | Proposed | 2                     | EVD-0010, the re-measurement against the real voice path; supersedes ADR-0002 in full |

### Records created

Topics without a link below have no individual record yet; the table above is still authoritative for their status.

- [ADR-0001: Render quantum semantics and splitting contract](decisions/ADR-0001-internal-render-quantum.md) —
  `Accepted`
- [ADR-0014: Persistent ID generation and encoding](decisions/ADR-0014-persistent-id-generation-and-encoding.md) —
  `Proposed`, one author pass and one review pass
- [ADR-0021: Host profile and admission policy](decisions/ADR-0021-host-profile-and-admission-policy.md) — `Accepted`
- [ADR-0022: Hardware time mapping and latency ownership](decisions/ADR-0022-hardware-time-mapping.md) — `Deferred`
  to the Phase 3 entry gate
- [ADR-0028: Long-running job contract](decisions/ADR-0028-long-running-job-contract.md) — `Deferred` to the Phase 4
  entry gate
- [ADR-0032: Sample-time and event-timestamp model](decisions/ADR-0032-sample-time-and-event-timestamps.md) —
  `Accepted` after three passes
- [ADR-0037: Render quantum frame count](decisions/ADR-0037-render-quantum-value.md) — `Accepted`, value provisional
- [ADR-0038: Engine-egress queue classification](decisions/ADR-0038-engine-egress-queue-classification.md) —
  `Accepted`, superseding three named ADR-0021 clauses
- [ADR-0039: Initial multi-client hub omission](decisions/ADR-0039-multi-client-hub-delivery-contract.md) —
  `Proposed`; the initial hub is an explicit public-API break and its final contract moves to the Phase 10E entry gate
- [ADR-0002: Internal channel layout](decisions/ADR-0002-internal-channel-layout.md) — `Accepted`, and **not on its
  measurement**. [EVD-0008](evidence/phase-02/EVD-0008-internal-channel-layout-cost.md) selected the interleaved arena
  by a median 9.3% end to end; the record was accepted on the shared-kernel argument instead, with that cost written
  into it and a re-measurement against the real voice path owed. That re-measurement is
  [EVD-0010](evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md), and the shared-kernel premise is what
  [ADR-0040](decisions/ADR-0040-v2-owns-its-dsp.md) withdraws — so the record is **superseded in full by
  [ADR-0041](decisions/ADR-0041-interleaved-internal-channel-layout.md) once that record is accepted**. It remains the
  standing authority until then. `Contract` class
- [ADR-0041: Interleaved internal channel layout](decisions/ADR-0041-interleaved-internal-channel-layout.md) —
  `Proposed`, **decided but not yet accepted**: the working agreement forbids accepting a `Contract` ADR in the session
  that drafts it. Supersedes ADR-0002 in full and ADR-0005's clauses 1, 2, 4, 5, 7 and 8. The arena becomes interleaved — one
  signal is one buffer of `Q` frames of `c` channels, a mono signal is unchanged, a kernel is told its channel count,
  and the host boundary becomes a copy rather than a transpose. Decided by the user on 2026-08-18 on
  [EVD-0010](evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md), which measured interleaved cheaper in
  all nine runs of all three shapes — 2.54% on the plan compiled today, 21.56% and 11.05% on two shapes the compiler
  cannot yet build. The price is written into the record: every stateful kernel becomes channel-aware, Phase 5's
  `LegacyPolyModuleAdapter` must convert at every V1 module port and **that cost is unmeasured**, and variable-width
  slots void ADR-0005's optimality argument. `Contract` class
- [ADR-0004: Native node representation](decisions/ADR-0004-native-node-representation.md) — `Accepted` **as a
  redraft**, which is not the same as accepted. Its rule B was run as written and failed —
  [EVD-0009](evidence/phase-02/EVD-0009-dispatch-cost.md) measured the prepared function table at +7.31% and +7.47% over
  a hand-written direct-call variant in two nine-run sets, against a 3% threshold — so its own rule C required a
  redraft. The redraft keeps Option C because **no dispatch shape passes**: the hybrid rule C names was built and
  measured at +7.08% and +7.34%, the same measurement as the table, and a closed enum for every node at +5.39% and
  +5.10%. What the measurement settles precisely is the price of the choice — **+2.12% and +2.09%** for the table
  against that closed enum over the same arena, two shapes differing in dispatch and nothing else, agreeing across two
  sets to three hundredths of a point The price is written into the record rather than waived. The second
  Phase 2 record decided against its own measurement, after ADR-0002
- [ADR-0040: V2 owns its DSP](decisions/ADR-0040-v2-owns-its-dsp.md) — `Proposed`, and **not acceptable alone**. It
  removes the shared-kernel premise that ADR-0002 was accepted on, which triggers that record's second revisit
  condition verbatim. Its clause 7 stops P02-T006 and P02-T007 and requires a companion layout decision (ADR-0041)
  before either record is accepted. Registered here so the conflict is visible in the register rather than only in the
  record that caused it
- [ADR-0005: Buffer liveness strategy](decisions/ADR-0005-buffer-liveness-strategy.md) — `Accepted` after three
  passes. Conservative linear-scan liveness, in-place expressed as a merged value chain, and a bit-identical
  reuse-off comparison that makes an aliasing bug fail rather than sound plausible

**ADR-0038 supersedes clauses, not a record.** ADR-0021 part 1 permits runtime overflow only for queues fed by external
unbounded input, which leaves three engine-egress rings — `LIMIT-0013`, `LIMIT-0014`, `LIMIT-0017` — with no admissible
failure behaviour at all, and its part 3 disposition for `LIMIT-0013` rests on two claims the use-site audits disproved:
that the drop counters are published on OSC, and that the channel is a live bounded queue. It is not published and has
no in-workspace production constructor or caller; public external use remains unobservable. A third clause goes with
them: the decision driver asserting the OSC publication,
superseded on the same evidence. ADR-0021 stays `Accepted` and authoritative for everything else. ADR-0038 is
accepted, so the three replaced clauses no longer bind. The three superseded
clauses are named in ADR-0038's metadata and linked from ADR-0021. This is the first partial supersession in the
register, and it is written that way because rewriting an accepted record is forbidden while re-deciding all of
ADR-0021 would discard reasoning that is still correct.

It is `Contract` class: it defines a queue direction, a payload test, and an error behaviour, so reversibility tests 1
and 3 both fail. It was accepted only after four independent passes over the drafting change reached the repository's
stopping condition: no remaining finding required a contract-clause change.

ADR-0001 and ADR-0021 were accepted after three review passes. Each carries a *Review history* note recording the
defects corrected before acceptance; the immutability rule in [decisions/README.md](decisions/README.md) now applies.

**ADR-0014 is `Proposed` after an author pass and one review pass**, and is deliberately not accepted. The review
found four defects, three P1 sharing one root: the first revision derived allocation state from surviving content and
called the absence of a persisted cursor an improvement on V1. It is not — deleting the highest-ordinal entity lowers
the derived maximum, so the next allocation reissues a retired ordinal, which the master plan forbids outright. The
record now carries a validated allocation record and says plainly that seven unvalidated cursors became one checked
one rather than none. It is `Contract` class: it
defines types, an encoding, a scope boundary, and an error behavior, so reversibility tests 1 and 3 both fail, and every
saved file would carry the encoding. Its record cites two questions the identity ledger leaves open — whether a master
or return chain's module id can collide with a patch's (`IDN-0021`), and what the closed parameter-name set is
(`IDN-0015`) — as format-review work Phase 0B owes **before** acceptance, rather than as things acceptance would settle.

ADR-0032 was judged `Contract` when work began on it, per the rule below: it defines types, an epoch, an ownership
boundary, and an error behavior, so tests 1 and 3 of the reversibility test both fail. Its one numeric choice — the
width of the quantum-local offset — is deliberately made independent of ADR-0037's provisional `Q`.

**Its first acceptance was withdrawn**, on the same grounds as ADR-0001's: it had been accepted on a single review pass
by its author. A second, independent pass found five defects, two of them substantive — the tempo map was made to
produce an engine time, which is not well defined across seek and offline renders, and one of two required
`HostProfile` horizons controlled nothing. Exhaustion and epoch reuse were undefined, and the pre-epoch clamp
contradicted ADR-0001's late-event policy. A third, bounded closure pass over those corrections found one more
substantive defect — the forward horizon, as written, would have rejected most of a compiled song — plus five smaller
ones, and fixed them without adding architecture. The record is `Accepted` on that third pass, the same shape ADR-0001
and ADR-0021 took.

That is now two records whose accept-on-first-draft was undone. The lesson is the pattern, not the record: a
same-session acceptance is provisional until an independent pass has run, whatever the gate pressure.

**ADR-0022 and ADR-0028 are `Deferred`, each to a named entry gate**, which is the second thing the Phase 0A exit gate
accepts. Each record carries the target gate, an owner, and the evidence still missing, and each states the constraints
that hold while it is open — a deferral is not permission to improvise the decision in code. ADR-0022 waits on a
simulated-host harness that Phase 3 builds anyway; ADR-0028 waits on a workflow analysis that is cheap but undone. Both
are `Contract` class: a deferred decision has a class from the moment work begins on it, and neither is a value.

**The three Phase 2 records are all `Contract` class**, and the class was judged when work began on each: ADR-0002
defines what a buffer is and when a conversion is a compile error, ADR-0005 defines a storage-sharing rule whose
violation is wrong audio, and ADR-0004 defines a call signature and an error behaviour. Reversibility test 1 fails for
all three.

**ADR-0004's rule was recorded before the measurement, run as written, and failed.** Its acceptance rule, its 3%
threshold and its falsifier were fixed at drafting; P02-T005 built the prototype, EVD-0009 measured it, and the figure
came back at +7.31% and +7.47%. The record is `Accepted` **as a redraft under its own rule C** rather than as the
acceptance it was drafted for, and its decision section says so in its first line — because a register that recorded
this as a plain acceptance would have made the difference between a rule that binds and a rule that decorates
invisible. The status field carries the vocabulary's `Accepted` and nothing else; the qualification lives in the prose,
where a reader will meet it.

**Two of Phase 2's three records now stand against their own measurements**, for different reasons that are worth
keeping apart. ADR-0002 was accepted against a measurement that *selected the other option* by 9.3%, on an
architectural argument about sharing one kernel shape with V1. ADR-0004 was redrafted after a measurement in which **no** dispatch
shape reached its threshold — the shape its own fallback proposed is indistinguishable from the one it was drafted for
— and in which the largest identified item, the arena binding every shape pays, could not be turned into a percentage
of the gap. The record says both, including the part it declines to quantify. Neither is a licence for the next record: what makes both admissible is that the measurement, the
figure and the reasoning are all in the record, and a reader who disagrees has everything needed to say so.

**ADR-0033 stays `Proposed`, and Phase 2 does not need it to move.** Phase 2 refuses every cycle rather than scheduling
one, which is the status quo and needs no feedback rule; a delay-boundary rule is what would *relax* that refusal, and
it is Phase 3's. What this costs is recorded rather than absorbed: until ADR-0033 is accepted, nothing in Phase 2 may
assume a cycle can be broken, and the cycle diagnostic may not promise a rule that does not exist.

ADR-0037 was accepted on [EVD-0002](evidence/phase-00a/EVD-0002-render-quantum-cost-proxy.md), which selected the
record's own rule 1: the V1 proxy could not resolve the comparison to better than its stated margin, so `Q` = 64 is
authoritative but provisional, and re-measuring it against real V2 nodes is a Phase 2 exit-gate item. `Accepted` is
still the right status — it is an implementation constraint, and nothing may treat the value as settled enough to tune
against.

### Reversible decisions

The second half of the register. A topic appears in exactly one of the two tables; this one carries the decisions that
meet the [reversibility test](#the-reversibility-test), and its rows are the record — there is no file behind them.

| ID | Topic | Value | Status | Revisit at | Why reversal is a rebuild |
|----|-------|-------|--------|------------|---------------------------|
| —  | —     | —     | —      | —          | *(no entry yet)*          |

Each row's `Why reversal is a rebuild` cell is test 3, stated in one sentence. A row that cannot fill it does not belong
in this table.

### Registered splits

ADR-0037 is not a Part VII topic of its own. It splits the master plan's topic 1 (internal render quantum) so that the
measurement-dependent frame count does not block the semantics, which are decidable now. Both identifiers are required
`Accepted` by the Phase 0A exit gate, so the split does not weaken it. The Phase 0A tracker records the deviation.

`Target phase` lists investigation, implementation, and later verification milestones; it is not permission to defer a
decision through the last listed phase. Explicit entry and exit gates in the master plan define the acceptance deadline.
In particular, a target beginning with `0A` or `0B` places the ADR in that sub-phase tracker even when later phases
refine or verify it.

## Register maintenance

When work begins on an entry, first decide its [class](#decision-classes).

For a `Contract` entry:

1. copy [templates/adr.md](templates/adr.md) to
   `decisions/ADR-NNNN-short-title.md`;
2. add links to relevant inventories and evidence;
3. keep the register status synchronized with the individual ADR;
4. if accepted, update affected specifications and phase tasks;
5. if superseded, retain the old ADR and link both directions.

For a `Reversible` entry:

1. move its row to the [reversible-decisions table](#reversible-decisions), filling the value, the revisit point, and
   test 3;
2. update affected phase tasks, and the master plan when a gate names the revisit point;
3. if it is later promoted to `Contract`, move the row back and write the record under its existing identifier;
4. if the revisit point changes the value, edit the row — a reversible decision does not need a superseding record,
   which is the entire point of the class.

An ADR may cover several tightly coupled register entries only when they cannot be decided independently. If so, retain
all identifiers as aliases in its metadata and explain the coupling.
