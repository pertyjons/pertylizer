# ADR-0038: Engine-Egress Queue Classification

| Field         | Value                                                              |
|---------------|--------------------------------------------------------------------|
| ID            | ADR-0038                                                           |
| Status        | Proposed                                                           |
| Phase         | 0A                                                                 |
| Created       | 2026-08-15                                                         |
| Last reviewed | 2026-08-15                                                         |
| Related       | P00A-T004, P00A-T005, LIMIT-0013, LIMIT-0014, LIMIT-0017, LIMIT-0076, EVD-0005 |
| Supersedes    | **Three named ADR-0021 clauses, and no others**: (a) part 1's runtime-overflow paragraph; (b) part 3's `LIMIT-0013` bullet; (c) the decision driver stating that `LIMIT-0013` "has per-priority drop counters published on OSC". ADR-0021 keeps `Accepted` and carries `Superseded in part` |
| Superseded by | —                                                                  |

## Context

[ADR-0021](ADR-0021-host-profile-and-admission-policy.md) part 1 permits runtime overflow **only** for "genuinely live
bounded queues — those fed by external, unbounded-in-time input such as MIDI or user gestures". Every other limit is
enforced at admission, retention, or presentation.

The pass-4 and pass-5 use-site audits of the [resource inventory](../inventories/resource-limits.md) found three bounded
queues that overflow at runtime and are fed by **the engine**, not by anything external:

- `LIMIT-0013` — the prioritized event rings, 256/256/512/2048 `TimestampedEvent` slots
  (`crates/synth_engine/src/event_priority.rs:75-78`);
- `LIMIT-0014` — the engine-to-GUI `EngineEvent` ring (`crates/synth_engine/src/synth_engine.rs:816`);
- `LIMIT-0017` — the per-client hub ring, 1024 `TimestampedEvent` slots
  (`crates/synth_engine/src/hub.rs:152`, created at `:218`).

All three carry data **out** of the engine toward a GUI, an OSC surface, or a remote client, so ADR-0021's permission —
which is written about queues fed *into* the engine from outside — does not reach any of them. The ledger therefore had
to leave all three outside `Classified`, and the Phase 0A exit gate's "no unexplained silent truncation" clause cannot
be satisfied while they sit there.

**They are not all in the same position relative to the render path, and an earlier revision of this record wrongly
said they were.** `LIMIT-0013` and `LIMIT-0014` are written from inside the audio callback, where blocking, refusing
and allocating are all forbidden, so dropping is the only move left. `LIMIT-0017` is not: `broadcast_event`
(`crates/synth_engine/src/hub.rs:256`) acquires an `RwLock` and a per-client `Mutex`, so it cannot be running on the
audio thread. Its drop is a chosen protocol behaviour rather than a forced one. The distinction changes what has to be
*justified*, not which rule applies, and part 1 below carries it.

**Two of the three are also unreachable in V1 today**, which is why this record disposes of them by removal rather than
by sizing them. `prioritized_event_channel` is constructed only in its own tests, and `broadcast_event` has no caller
anywhere in the workspace. A capacity nobody reaches has never been exercised, so nothing about its shape — four
priority tiers, 1 024 slots per client — is validated by use.

The same audits disproved the two factual claims ADR-0021's `LIMIT-0013` disposition rests on. Both are recorded in the
*Evidence* section below.

**Outside this decision.** Ring *sizes*, which stay with each entry's configuration owner
([P00A-T005](../specs/spec-host-profile-and-render-limits.md) for `HostProfile`, the protocol surface for a protocol
cap). ADR-0021's two axes, its five other class semantics, part 2, part 3's other four dispositions, and part 4 are
untouched and remain authoritative. Renderer **ingress** — what feeds the renderer and how much of it there can be — is
explicitly not decided here; it is Phase 3's, per the Phase 3 work list.

## Decision drivers

- **A class that describes no entry is a finding about the taxonomy.** ADR-0021 says exactly this about its owner axis
  ("an entry that fits none is a finding about this table"), and the same standard applies to its failure axis.
- **On the render path the producer has no other option.** For an egress queue written inside the callback, blocking
  violates real-time safety, refusing has no caller to return to, and growing the queue allocates. Dropping is the
  only admissible behaviour, so the question is not *whether* but *what may be dropped and who is told*.
- **Off the render path, dropping is a choice and must be argued rather than assumed.** An earlier revision of this
  record asserted that all three queues are written from the render path. `LIMIT-0017` is not:
  `EngineHub::broadcast_event` (`crates/synth_engine/src/hub.rs:256`) takes an `RwLock` over the client map and a
  `Mutex` per client, both forbidden on the audio thread, so whatever calls it is not the callback. A producer in
  that position **may** block or apply backpressure, so a drop there is a protocol design decision with a cost
  somebody chose to pay. Part 1 therefore licenses dropping by direction **and** by position relative to the render
  path, and requires the second to be stated rather than assumed.
- **One ring can carry payloads with different loss semantics, and a per-ring rule flattens them.** The
  engine-to-GUI `EngineEvent` ring carries two kinds through the same 256 slots: observational events whose pushes
  are discarded with `let _ =`, and `RecordedNotesFlushed`, which is retried and parked because it holds notes the
  user played. The `NoteEvent` telemetry ring is a **separate** ring that only shares the constant — that is
  `LIMIT-0076`, and conflating the two is the misreading this record exists to end, so it must not be repeated in
  this record's own drivers. The ledger's conclusion is that "a single drop-counter policy over this ring would
  demote (b) to (a) and regress recorded-note delivery, so the V2 rule must classify producers, not the ring."
- **A diagnostic that reaches nobody is not a diagnostic.** ADR-0021 already states this, and `LIMIT-0013` is the
  counter-example that proves it: per-priority counters exist and no code reads them.
- **ADR-0021 is accepted and therefore immutable.** Correcting it requires a superseding record, not an edit.

## Options considered

### Option A: Widen ADR-0021's live-bounded-queue class to cover egress

Change "fed by external, unbounded-in-time input" to "fed by an unbounded-in-time producer", so the engine counts as one.
One-line fix, no new class. It is wrong in the way that matters: the external-input wording is not incidental, it is what
makes runtime loss acceptable — nobody can be told to send less MIDI, whereas the engine's own emission rate is a design
choice. Widening the class silently blesses an engine that emits faster than anything drains as a resource policy, and
it still has no answer for `RecordedNotesFlushed`.

### Option B: Classify egress queues by payload, with a distinct egress rule

Add one rule covering queues the engine writes and something outside the render path reads, and make the failure class a
property of the **payload** rather than of the ring. Observational payloads take the existing
`Lossy retention/presentation budget` class, which already names telemetry rings and already requires the loss to be
exposed. Custodial payloads — the only remaining copy of authored or performed data — may not share an egress ring at
all. Costs one new normative rule and forces `LIMIT-0014`'s ring to be split in V2. Answers every case the audits found.

### Option C: Make egress overflow a preparation-time impossibility

Size every egress ring so it provably cannot fill, and treat a full ring as a defect. This is what `LIMIT-0016`'s
authors argued in comments ("sized so it will not realistically fill"). The ledger's verdict on that argument stands:
plausible, and not a guarantee. A bound that depends on the drain rate of a GUI thread cannot be established at
preparation time, and the failure mode when the reasoning is wrong is a silent loss with no counter — the exact
situation this ADR exists to end.

### Status quo

Three ledger rows stay outside `Classified`, so P00A-T004 cannot close and the Phase 0A exit gate's silent-truncation
clause is unsatisfiable. `LIMIT-0014` cannot be split, so `event_egress_capacity` leaves HOST-INV-005 unsatisfied and
P00A-T005 cannot close either. ADR-0021 keeps a disposition resting on two claims known to be false.

## Evidence

Source reads at `29c22ef4`, each re-resolved for this record rather than carried over from the ledger:

- **`LIMIT-0013`'s counters are published nowhere.** `get_dropped_counts` is defined at
  `crates/synth_engine/src/event_priority.rs:194` and has no caller in the workspace. ADR-0021's decision driver
  ("`LIMIT-0013` has per-priority drop counters published on OSC") and its part 3 disposition ("promoted from an
  OSC-only publication") both describe a publication that does not exist. OSC's `/synth/engine/event_drops` reads
  `EngineState::event_drops` (`crates/synth_engine/src/state.rs:585`), a different counter on a different ring.
- **`LIMIT-0013`'s channel does not run in production.** `prioritized_event_channel` is defined at
  `crates/synth_engine/src/event_priority.rs:99`; its only callers are `event_priority.rs:296` and `:350`, both inside
  the module's own `#[cfg(test)]` block (`event_priority.rs:281`). `crates/synth_engine/src/lib.rs:57-60` re-exports it
  and nothing else references it. ADR-0021 classified as "a genuinely live bounded queue" a channel that is never
  constructed outside tests.
- **Critical events are droppable.** `PrioritizedEventProducer::send` uses one-shot `try_push` for every priority
  including `Critical` and increments that priority's counter on failure
  (`crates/synth_engine/src/event_priority.rs:141-150`), while the module documents critical events as never dropped.
- **`LIMIT-0014` is one constant sizing two rings with different destinations.** `EVENT_BUFFER_SIZE`
  (`crates/synth_engine/src/synth_engine.rs:81`) sizes `HeapRb::<EngineEvent>` at `:816`, consumed by the GUI, and
  `HeapRb::<NoteEvent>` at `:820`, created "for OSC telemetry" at `:819` and handed to `synth_osc` from
  `crates/pertylizer/src/main.rs:403-416`.
- **`LIMIT-0017` drops per client and now counts it.** `broadcast_event` discards a full-ring `try_push`
  (`crates/synth_engine/src/hub.rs:275`) and increments that client's own `dropped_events` (`:277-281`), readable
  through `EngineHub::dropped_events_for` (`:472`).

The audit method that produced these reads, its coverage, and its known weaknesses are recorded in
[EVD-0005](../evidence/phase-00a/EVD-0005-resource-ledger-use-site-audit.md).

**Uncertainty that remains.** No egress ring's size has been measured, and this record does not set one. Whether V1's
GUI ring actually overflows in practice is unknown: the `EngineEvent` side has no counter today, which is precisely the
gap the decision closes. The `LIMIT-0014` drop counters this ADR requires will be the first measurement of it.

## Decision

Proposed. Four parts, and which of them supersede is stated exactly because an earlier revision got it wrong.
**Part 1 supersedes** ADR-0021 part 1's runtime-overflow paragraph, replacing it with a rule that covers engine
egress. **Part 3 supersedes** ADR-0021 part 3's `LIMIT-0013` bullet and the decision driver behind it. **Parts 2 and
4 supersede nothing**: part 2 adds the payload distinction ADR-0021 lacks, and part 4 applies the result to the
ledger. Three superseded clauses in total, all named in this record's `Supersedes` field, and none by implication.

### 1. Engine egress is a queue direction, with its own rule

A bounded queue is **engine egress** when **the engine is the producer and the consumer is outside the engine** — a
GUI, a telemetry surface, a remote client. The definition is about direction and does **not** require the producer to
be on the render path; an earlier revision of this record wrote it that way and thereby excluded `LIMIT-0017`, the very
entry that showed the two are separate questions. Where the producer sits decides which conditions apply, not whether
the queue is egress.

Runtime dropping is permitted at an engine-egress queue, in addition to the live bounded queues ADR-0021 part 1
permits, and **only** under all three of:

1. the payload is observational, per part 2;
2. every drop is counted, and the count is attributable to the queue that dropped it;
3. the loss is **attributable by a consumer**, through either the structured diagnostics report or a count that
   travels with the data it belongs to. A telemetry channel alone does not satisfy this; nor does a counter with no
   reader.

Condition 3 is the one `LIMIT-0013` fails today and the one ADR-0021 already argues for in prose. An egress queue that
cannot meet all three is a defect to fix, not a budget to size.

**Condition 3 admits two forms deliberately, and `LIMIT-0021` is why.** An earlier revision required the structured
diagnostics report specifically. That would have forced a regression on the master-scope visualization ring, which
already does something stronger: `read_samples_into`
(`crates/synth_engine/src/visualizers/mod.rs:281`) is annotated `#[must_use]` at `:280`, takes the omission count at
`:321` and returns it at `:329`, so the count is paired with the window it belongs to and a caller cannot discard it
silently. A global counter tells a reader that *something* was dropped; a count returned with
the data tells it which window has a gap. Requiring the weaker form would have been the rule optimising for its own
uniformity. What both forms have in common is the property that matters — a consumer can attribute the loss — and that
is what the condition now says.

`LIMIT-0021` therefore falls under this record's definition and keeps its existing disposition unchanged, which is the
test a definition of this width has to pass. It is listed here rather than left implicit, because a rule that silently
reclassifies rows already settled under another contract is how an accepted decision acquires consequences nobody
reviewed.

**The definition reaches seven ledger entries, and all seven were checked against it.** Four are the rows this record
reopens — `LIMIT-0013`, `LIMIT-0014`, `LIMIT-0076`, `LIMIT-0017` — and three were already settled elsewhere:

| Entry | Payload kind | Result under this record |
|-------|--------------|--------------------------|
| `LIMIT-0013` | Observational | Removed; the channel is never constructed outside its own tests |
| `LIMIT-0014` | Mixed — observational plus custodial `RecordedNotesFlushed` | Classified lossy; part 2 forbids the sharing, so V2 separates the custodial path |
| `LIMIT-0076` | Observational | Classified lossy; condition 3 unmet in V1, where the count reaches OSC only |
| `LIMIT-0017` | Observational | **Not classified** — conditions 3 and 4 both unmet, and 4 has no owner |
| `LIMIT-0015` | Custodial | Unchanged. Condition 1 fails, so dropping is not licensed at all; the entry is removed for that reason |
| `LIMIT-0016` | Custodial | Unchanged, for the same reason |
| `LIMIT-0021` | Observational | Unchanged and `Classified`. Counted, the count travels with its window, and condition 4 does not apply on the render path |

`LIMIT-0012` is **ingress** and outside this record. Entries such as `LIMIT-0020`, `LIMIT-0051` and `LIMIT-0052` are
shared slots or retention stores rather than engine-produced queues, so the direction test does not reach them.

The sweep is recorded because the rule it applies was learned here: an earlier revision widened this definition to
admit `LIMIT-0017` and silently swept in `LIMIT-0021`, which review caught. **A definition change is not finished
until every entry it now reaches has been checked against it**, and the table above is that check rather than an
assurance that it was performed.

**A fourth condition applies to an egress queue whose producer is off the render path**, such as `LIMIT-0017`'s: the
record for it must state *why* dropping was chosen over blocking or backpressure, both of which are available there. On
the render path that question does not arise and the reason is the real-time constraint itself, so conditions 1-3 are
the whole test for `LIMIT-0013`, `LIMIT-0014` and `LIMIT-0076`. This condition exists because the absence of it is what
let an earlier revision of this record treat a chosen protocol behaviour as a forced one.

**An entry whose condition 4 is unmet is not classified by this record.** Condition 4 is part of the rule, not an
annotation on it, so a queue that has not answered it has no settled disposition — the class names what happens on
overflow, and "drop, for a reason nobody has given" is not that. This matters because it is the one place where
accepting this ADR does *not* close a ledger row.

The failure class for an entry meeting all three is `Lossy retention/presentation budget`, which ADR-0021 part 1 already
defines and which already names telemetry rings. **No new failure class is added.** What this record adds is the
direction, the payload test, and the standing that lets the lossy class apply to a queue rather than only to a retention
policy.

### 2. Observational and custodial payloads, and why they may not share a queue

A payload is **observational** when losing it costs the consumer a view of something the engine still knows: meters,
peaks, voice counts, event mirrors, note telemetry, CPU samples. The engine remains the authority; the consumer's
picture is merely stale or incomplete, and the count tells it so.

A payload is **custodial** when the queue holds the only remaining copy, or when the handoff is what keeps a heap value
from being freed on the audio thread. `RecordedNotesFlushed` is custodial in the first sense — it carries notes the user
played, which exist nowhere else once the engine hands them over. `LIMIT-0015`'s and `LIMIT-0016`'s deferred-drop
channels are custodial in the second.

**A custodial payload may not travel on an observational egress queue.** Its loss is a correctness or real-time defect,
not a budget, and it is not made acceptable by counting it. V2 carries custodial payloads on a path with a declared
capacity, a defined behaviour on exhaustion, and no silent drop; where V1 shares one ring between the two kinds, the V2
rule separates them.

This is what forbids the rule a per-ring policy would have produced. `LIMIT-0014`'s ring carries observational
`EngineEvent`s and custodial `RecordedNotesFlushed` through the same 256 slots, and a uniform drop-and-count policy over
it would have demoted the retried custodial payload to a counted loss — regressing recorded-note delivery while
appearing to improve the diagnostics.

### 3. `LIMIT-0013`'s disposition, superseding ADR-0021 part 3

ADR-0021 part 3's `LIMIT-0013` bullet is superseded in full. It rests on two claims this record's evidence disproves:
that the counters are published on OSC, and that the channel is a live bounded queue. It is neither published nor live —
it is not constructed outside its own tests.

The replacement disposition: **the prioritized event channel is dead code and does not carry over.** Its class becomes
`Implementation artifact to remove` and its owner `N/A — removed`. V2's engine egress is the rule in part 1 with a
capacity owned by the entry's configuration owner, not a port of V1's four-ring structure — which was never exercised,
so nothing about it has been validated, including whether four priorities are the right number.

ADR-0027 continues to own what the taps are *for*. The decision driver in ADR-0021 that cites "per-priority drop
counters published on OSC" is superseded by the same evidence; it is stated as a fact, and the fact is false.

### 4. `LIMIT-0014` splits, and the ledger identifier rule that follows

One entry may not carry two owners (ADR-0021 part 1's closed set requires exactly one), and `EVENT_BUFFER_SIZE` sizes a
GUI stream and a protocol stream. The entry splits:

- **`LIMIT-0014`** keeps the engine-to-GUI `EngineEvent` ring. Owner `HostProfile` — it is the real-time communication
  capacity ADR-0021 part 4's field list already covers — and class `Lossy retention/presentation budget` under part 1
  above, with the custodial carve-out of part 2 applying to `RecordedNotesFlushed`.
- **`LIMIT-0076`** is allocated for the engine-to-OSC `NoteEvent` telemetry ring. Owner `Protocol contract`, since the
  OSC surface serializes it, and class `Lossy retention/presentation budget`.

The two may take different sizes and different loss semantics from the moment they are separate rows, which is the
point of splitting them. In V1 they share one constant; V2 does not.

`LIMIT-0017` takes the same class as `LIMIT-0014` and keeps its `Protocol contract` owner, but on different footing
from either of the other two. Its per-client counter exists (`crates/synth_engine/src/hub.rs:277-281`) and must reach
the structured diagnostics report rather than only `EngineHub::dropped_events_for`, which is part 1 condition 3. **Its
condition 4 is unmet, so this record does not classify the row**: `broadcast_event` is off the render path, so
blocking a slow client or applying backpressure to it are both available, and nothing in V1 records why dropping was
preferred. The comment argues that a slow client must not stall the broadcast — a real reason for not blocking *the
broadcast*, and not a reason for dropping rather than disconnecting, buffering off-thread, or refusing the
subscription.

**No registered decision owns that choice, and that is itself a finding.** An earlier revision of this record handed it
to ADR-0029, whose registered topic is *host configuration and remote authorization* — deployment and threat review,
not queue loss semantics. The handoff was asserted rather than checked. The multi-client hub's delivery contract has no
owner in the register, which under ADR-0021's own standard ("an entry that fits none is a finding about this table") is
a gap to record rather than a cell to fill. **`LIMIT-0017` therefore stays `Investigating` after this record is
accepted**, alone among the four rows this record reopens, and the follow-up table below carries registering that decision. Nothing
about this is urgent in V1 — the hub is unreached — but a row that is silently marked `Classified` on a policy nobody
chose is exactly how `LIMIT-0013` came to rest on two false claims for three passes.

**The hub is also unreached in V1**, so none of this is currently a live loss: `broadcast_event`
(`crates/synth_engine/src/hub.rs:256`) has no caller in the workspace. That does **not** make it
`Implementation artifact to remove` the way `LIMIT-0013` is — a multi-client hub is a planned capability with an
`Proposed` decision behind it, whereas the prioritized channel is a mechanism V2 replaces outright. It does mean the
entry describes a hazard that cannot fire today, and the silent-truncation register says so rather than implying a
loss users are experiencing.

## Consequences

### Positive

- Three of the four egress rows that were *reopened* — `LIMIT-0013`, `LIMIT-0014`, `LIMIT-0076` — can be classified. `LIMIT-0017` cannot, because its condition 4 has no owner, and saying so is the point: P00A-T004 is closer to its all-limits scope and not at it, and the Phase 0A exit
  gate's silent-truncation clause becomes satisfiable for them.
- `event_egress_capacity` gets a settled ledger antecedent, which is what HOST-INV-005 was missing.
- The custodial/observational distinction gives `RecordedNotesFlushed` — and the one `Vec` still dropped on the audio
  thread behind it — a named contract violation rather than a known-but-unowned defect.
- The taxonomy gains a test it did not have: a queue with no admissible failure behaviour is now a finding rather than
  an unclassifiable row.

### Negative

- One more record to read before ADR-0021 can be applied, and ADR-0021's text now has three superseded clauses a reader
  must know about. The `Superseded by` link in ADR-0021 is the only mitigation available, since the record is immutable.
- Removing `LIMIT-0013` discards a prioritization design without replacing it. If V2 wants priority classes on egress,
  it designs them from the requirement rather than porting four untested rings — which is more work than keeping them.
- Splitting `LIMIT-0014` renumbers nothing but does mean the ledger has two rows where one constant exists in V1, so a
  reader of V1 source finds one thing and a reader of the ledger finds two. Both rows cite the same constant and say so.

### Risks and controls

- **Risk: the diagnostics report becomes the new place counters go to die**, repeating `LIMIT-0013` one level up.
  Control: part 1 condition 3 names the structured diagnostics report specifically because that is the report the exit
  review inspects; a counter that is not in it does not satisfy the condition.
- **Risk: the observational/custodial test is applied by intent rather than by inspection**, so a payload gets called
  observational because dropping it is convenient — or a variant added later inherits the wrong semantics silently.
  **The test is not decidable from the payload type, and this record does not claim it is.** "Does an authoritative
  copy remain" is a fact about ownership and lifecycle, and V1's `EngineEvent`
  (`crates/synth_engine/src/commands.rs:1013-1046`) is the counter-example: one enum holds presentation variants and
  the custodial `RecordedNotesFlushed` side by side, so no property of the type distinguishes them. Control: V2's
  egress payload enum carries an **explicit closed classification** — a total function from variant to kind, matched
  exhaustively — so that adding a variant without classifying it fails to compile rather than defaulting to
  observational. Classifying a variant is then a reviewable decision with a diff, which is the only form of this
  control that a future contributor cannot bypass by accident.
- **Risk: V2 reintroduces a shared ring** because one queue is cheaper than two. Control: part 2 forbids it as a
  contract clause, and the conformance test for the egress capacity field checks the payload kinds admitted to it.

## Follow-up work

| Task                                                                                                     | Phase | Status      |
|----------------------------------------------------------------------------------------------------------|-------|-------------|
| Classify `LIMIT-0013`, `LIMIT-0014` and `LIMIT-0076` in the resource ledger under this record | 0A | Complete |
| Register a decision topic for the multi-client hub's delivery contract, which `LIMIT-0017`'s condition 4 needs and no existing ADR owns | 0A | Not started |
| Give the `EngineEvent` egress side a drop counter in V1, so the first measurement of it exists            | —     | Not started |
| Route `LIMIT-0017`'s per-client counter into the structured diagnostics report                            | 9     | Not started |
| Separate `RecordedNotesFlushed` from the observational egress ring, removing the audio-thread `Vec` drop  | 9     | Not started |
| Define V2's engine-egress capacity fields and their conformance test                                     | 1     | Not started |

## Revisit conditions

- An engine-egress queue is found whose payload is observational by the part 2 test but whose loss is nevertheless
  user-visible as a correctness fault, which would mean the test is wrong rather than the entry.
- Measurement shows a GUI or protocol egress ring overflowing in normal use, which would make its size a budget to set
  from evidence rather than a capacity nobody has had to think about.
- Phase 3's renderer-ingress work finds that ingress and egress need one common queue contract, which would make this
  record a part of that one rather than a standalone rule.
