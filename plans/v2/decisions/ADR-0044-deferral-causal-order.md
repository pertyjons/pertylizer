# ADR-0044: Deferral-Induced Causal Order

| Field | Value |
|---|---|
| ID | ADR-0044 |
| Status | Deferred |
| Phase | 3 |
| Created | 2026-08-20 |
| Last reviewed | 2026-08-20 |
| Related | ADR-0043; ADR-0001 clauses 12, 13 and 14; ADR-0023; ADR-0032 clauses 22 and 23; `HOST-INV-021`; `SOUND-INV-016` |
| Supersedes | — |
| Superseded by | — |

This record meets `PROCESS.md`'s durable-decision test on two counts: it defines a real-time ownership boundary — which
event the renderer may move, and what moves with it — and it binds the scheduler, the voice pool, and the offline/live
equivalence gate. It fixes no value.

> **Narrowed on 2026-08-24 to same-control causal order.** The option survey below asked, under frame F1, for a repair
> holding for *every* pair whose meaning depends on order, and **no candidate qualified**; the residual they all shared
> was the cross-control one. On the maintainer's scope decision the same day this record keeps the same-control
> question, where a mechanism exists, and the remainder is registered as
> [ADR-0045](ADR-0045-cross-control-causal-order.md) — deliberately **without** a named gate, since
> `PROCESS.md`'s decision-timing rule warns that replacing one phase prerequisite with a new one is not progress.
> **F1 is not restated.** It stands as written, as the record of what it eliminated; what changed is the question it
> is applied to. The narrowing left 1b's run form as the only mechanism in scope, and a second read then found four
> defects in it, so **this record still has no candidate** — see *Options considered*.

> **Deferred, not open for improvisation.** [ADR-0043](ADR-0043-event-deferral-and-late-clamp.md) is `Accepted` and
> creates the hazard this record must close. It named the hazard rather than solving it, because the repair is
> scheduler design and ADR-0043's decision boundary excludes it. **Phase 3 implementation may not begin before this
> record is `Accepted`**, alongside [ADR-0022](ADR-0022-hardware-time-mapping.md).

## The deferral

| Field | Value |
|---|---|
| Deferred to | The **Phase 3 entry gate**. Phase 3 implementation may not begin before this is `Accepted` |
| Owner | Project maintainer — this is a single-maintainer repository, so there is no second party to assign |
| Input required | **The survey ran on 2026-08-24 without it, and a second read showed that was only true for a candidate that does not work.** The run form the survey ended on must be costed against the ingress and deferred-store capacities, because its scan fires exactly when the per-quantum limit is exceeded. So this record does depend on the store's shape after all, for the surviving candidate. What follows was written before that was established, and is kept because the *relation* half of it stands: and what remains is the maintainer's selection and the coupled ADR-0043 successor. This field previously scoped a deferred-store dependency to successor propagation; the survey narrowed that again. It holds for [option 1a](#1a--successor-by-declared-note-identity), which pairs events by a declared note identity and stores the pairing, and **not** for [option 1b](#1b--successor-by-compiled-control-slot), whose relation is equality of a compiled control slot the renderer already derives per event, with one `SampleTime` per slot against a count admission fixes. Three earlier drafts over-claimed this dependency — first for every candidate, then for both scheduler-side ones, then for successor propagation as a whole |
| Why not now | The candidate repairs are scheduler and voice-pool design, not timing semantics, and ADR-0043's decision boundary excludes them. Deciding them inside that record would have made a timing decision carry a scheduler design its reader was not reviewing. **That reason is spent**: the survey below is the design work, and what is left is a selection and the successor it names, both of which belong to the Phase 3 entry gate transaction rather than to ADR-0043's |
| What makes it safe | The hazard is **unreachable today**: no V2 code defers. Deferral does not exist in the renderer, Phase 1 and Phase 2 are offline with a prevalidated bounded event span, and `HOST-INV-021` keeps the store and the ingress capacities `Deferred to Phase 3`. Nothing can reach the inversion before the code that would cause it is written |

## Durable boundary

**A real-time ownership boundary.** Whether a deferred event drags other events with it decides what the deferred store
holds — positions, or a relation between events — and that is a hot-path data structure with a preallocated bound. It
cannot be discovered by an implementer at a call site.

**A correctness boundary.** The failure mode is an audible stuck voice, not a timing inaccuracy, and it survives every
mechanism ADR-0043 selected.

## Context

ADR-0043 selected Option D: an event is assigned to the quantum containing its **render position**, and a quantum that
cannot admit an event advances that event's render position by exactly `Q`. The stamp is immutable and the offset is
preserved.

`+Q` can therefore reverse cause and effect. At `Q` = 64:

- a note-on stamped at sample 63 is deferred, and renders at position 127;
- its own note-off, stamped at sample 65, is natively due in the next quantum and renders at 65;
- so the note-off renders **before** the note-on, and the voice is left sounding.

Two candidate repairs are already excluded, and stating them is most of what this record inherits:

- **`ADR-0023` cannot repair it.** Same-sample ordering needs a tie to break, and 65 and 127 are not a tie.
- **Ordering the admitted set by stamp cannot repair it.** That changes which event the renderer *inspects* first, not
  where either one *renders*. Sample 127 still comes after 65 and the voice still hangs.

So a repair must move the successor's render position too, or neutralise the inversion where the voice is allocated.
Choosing between those is what this record is for.

**The hazard is not specific to note pairs.** Any two events whose meaning depends on their order — note-on and
note-off, gate open and close, a retrigger against the edge it retriggers — can be separated by a deferral that moves
only one of them. Note pairs are the case with an audible, persistent symptom, which is why they are the example.

**Deferral is not the only way to strand a voice**, and this record does not claim otherwise. ADR-0021 permits a live
bounded queue to drop an excess external event with no exception for note-offs, so a queue drop can strand a voice
under any timing rule. What this record owns is specifically the *deferral-induced* inversion — an event the renderer
accepted and then moved.

## What this record will decide

1. **Whether a deferred event's causal successors are deferred with it**, and if so what "successor" means to the
   renderer — a relation the compiler declares, a per-voice or per-node relation the scheduler derives, or something
   narrower. **Since the 2026-08-24 narrowing, "successor" is bounded to the same resolved `(node, control)`**; a
   successor on another control is [ADR-0045](ADR-0045-cross-control-causal-order.md)'s. **Selecting this candidate requires a successor to
   [ADR-0043](ADR-0043-event-deferral-and-late-clamp.md)**, because it moves an event whose own quantum had room and
   `SOUND-INV-016` admits no such reason.
2. **What the deferred store holds to represent that**, and how it stays preallocated and bounded under
   `HOST-INV-021`'s eventual capacity contract.
3. **Whether a deferral that would invert an order is instead refused**, and what the refusal is — an admission-time
   reservation that keeps the pair together, a counted fault, or a bounded exception. **Selecting this candidate
   requires a successor to [ADR-0043](ADR-0043-event-deferral-and-late-clamp.md)**, and that is a cost the survey has
   to carry rather than discover: ADR-0043 says an over-full quantum defers by `+Q`, so refusing instead is a
   different timing rule and not an elaboration of this one. An **admission-time reservation** may be the exception,
   if it keeps a causal pair together without ever refusing a deferral — the survey has to establish that rather than
   assume it.
4. **Whether the repair lives in the scheduler or in the voice allocator.** A voice that receives a note-off for a
   voice it has not started could remember it rather than discard it, which neutralises the audible symptom without
   ordering anything. Whether that is a legitimate repair or a way of hiding a broken order is exactly the question,
   and it decides which component owns the invariant.
5. **What the conformance test asserts**, which under `PROCESS.md` must be a named automated test rather than a claim.

## Outside it

- The timing rule itself, the immutable stamp, and the clamp — [ADR-0043](ADR-0043-event-deferral-and-late-clamp.md),
  `Accepted`.
- Same-sample ordering — `ADR-0023`.
- The ingress capacities, the deferred store's bound, its exhaustion policy, and the starvation question —
  `HOST-INV-021`, `Deferred to Phase 3`.
- The hardware clock mapping — [ADR-0022](ADR-0022-hardware-time-mapping.md).
- **Cross-control causal order** — [ADR-0045](ADR-0045-cross-control-causal-order.md), `Deferred` to the same Phase 3
  entry gate. Outside this record only since the 2026-08-24 narrowing, and outside it as an open question rather than
  a settled one: the survey below found no mechanism that reaches it under frame F2.

## Options considered

**Surveyed on 2026-08-24.** It needed neither the deferred store's shape nor any measurement, and one of its findings
is that this record's own account of which candidates needed the store was too strong. That correction is carried into
*The deferral* above and into [`NOW.md`](../NOW.md). The frames come first, then the candidates against them, then the
result — which is that **no candidate passes F1**, so the survey eliminates and costs rather than selects. The
independent read is what established that: it refuted the draft's recommendation on two conclusion-affecting points,
both recorded in place below rather than quietly repaired.

### The frames

Written before the candidates, so that no candidate supplies its own test. F8 is a cost to weigh; every other frame is
falsifiable, and the right-hand column is what refutes a candidate rather than what disappoints in it.

| Frame | What it requires | What falsifies a candidate |
|---|---|---|
| **F1 Generality** | The repair holds for every pair whose meaning depends on order, not only a note-on and its note-off | Exhibit one ordered pair the candidate leaves inverted |
| **F2 Real-time admissibility** | No allocation, lock, or unbounded scan on the audio thread; every container preallocated against a capacity admission already computes | The candidate needs state sized by a quantity admission cannot compute, or a scan not bounded by a declared capacity |
| **F3 No backward reach** | ADR-0001 clause 12's second sentence: nothing is applied to samples already produced | The candidate can produce a render position earlier than the clock |
| **F4 Immutable stamp** | A repair may not re-stamp an event to reorder it | The candidate writes `envelope.time()` |
| **F5 No post-admission drop** | `HOST-INV-019`, and ADR-0021's reservation of runtime dropping to live bounded queues *before* the renderer sees the event | An event the renderer admitted does not render, and no counted, reported fault says so |
| **F6 Interval fidelity** | The rendered distance between two events the repair moves equals the distance it found | Exhibit a pair whose rendered interval differs from its stamped interval |
| **F7 Provenance symmetry** | Identical behaviour for compiled and ingress events. ADR-0043's timing rule makes no provenance distinction, unlike the forward horizon, which ADR-0032 clause 21 binds to ingress deliberately; Phase 9's exit requires live/offline render agreement, which is the observable a provenance-dependent causal rule would break. **Not** Phase 3's exit gate, whose determinism requirement is across host-block partitions rather than across sources | The candidate needs both members of a pair to be known before either is admitted |
| **F8 Record cost** | Whether an ADR-0043 successor is required, and whether it can be accepted in the same transaction | — a cost, weighed rather than passed |
| **F9 Bounded cascade** | A displacement the repair induces terminates | The repair's own output can require a further application of itself with no bound |
| **F10 Conformance test** | A named automated test, per `PROCESS.md` | No test can be named that fails on an inverted pair and passes otherwise |

### Option 1 — translate the successors

#### 1a — successor by declared note identity

The compiler declares which off-edge belongs to which on-edge, and deferral moves the pair. `NoteEdge` carries no
identity today and says so: read at `d1dd12a3`, `render.rs:167-171` records that it carries "no pitch, velocity or note
identity" and that "Phase 3's ingress and Phase 6's voice pool are where they arrive". So 1a first widens the ingress
contract, then adds the per-event relationship state this record predicted for successor propagation.

**It fails F1.** An identity pairs an on-edge with an off-edge and says nothing about a retrigger deferred past the edge
it retriggers, nor about a gate addressed as a parameter rather than played as a note — which `EventPayload`'s contract
(`render.rs:195-205`) says reaches the same control under the same timing law. A relation built from note semantics
cannot cover pairs that are not notes, and this record's *Context* puts those in scope.

#### 1b — successor by compiled control slot

**The rule.** When an event at render position `p` is deferred to `p + Q`, every event addressing the **same resolved
`(node, control)`** whose render position lies in the open interval `(p, p + Q)` is advanced by exactly `Q` as well.

**The key is the resolved pair, not the payload's slot index**, and the distinction is load-bearing rather than
pedantic. A note slot and a parameter slot are separate index spaces that can reach one control: `EventPayload`'s own
contract says "addressing a gate as a parameter and playing its node as a note reach the same control under the same
timing law" (`render.rs:195-205`). Keying on either slot index would leave that pair unrelated and reintroduce the
hazard through the other payload. A first draft of this survey made exactly that error.

Six properties, each checkable against current source and none against the deferred store:

1. **It does *not* terminate in one application, and the first draft's proof that it did was wrong.** That proof
   showed a translated successor lands after its *predecessor* and never asked whether it lands before the next
   **untranslated** event on the same control. The independent read supplied the counterexample: at `Q` = 64, events
   at 63, 65 and 128 on one control, deferring 63 to 127 moves 65 to 129 and leaves 128 where it is, so 129 and 128
   are reversed — a **new** inversion the repair created. Verified by evaluation, not by re-reading the argument.

   The rule survives only in a wider form: translate the **maximal run** of same-control events in which each
   consecutive gap is smaller than `Q`, starting at the deferred event. On the counterexample the run is 63, 65, 128
   — since 128 − 65 = 63 < `Q` — so all three translate, to 127, 129 and 192, and each gap is preserved exactly. The
   run ends at the first gap of `Q` or more, which is the condition under which the last translated event cannot
   **overtake** the first untranslated one. That terminates, because the run is finite; it is **not** one
   application, and F9 is answered by a bound rather than by a proof of single application.

   **"Cannot overtake" is true and insufficient, which brute force established and argument did not.** A gap of
   *exactly* `Q` ends the run and then receives the translated event on top of the event already sitting there: at
   `Q` = 64, events at 1, 16, 94, 300, 364, 372 with 300 deferred give **364 twice**, because 364 − 300 = 64 is not
   less than `Q`. Nothing is reordered, but a pair the plan declared in a strict order becomes a **same-sample tie**,
   and resolving ties is `ADR-0023`'s — `Proposed`, with no record. So the run form does not merely fail the frames
   listed above; it also manufactures work for a decision that does not exist. Measured over 200 000 random
   same-control event sets: no gap violations, no overtakes, collisions in about 0.7% of cases. The check is three
   lines, and it was available before any of this was written down.

   **The wider form then failed a second independent read on four counts, and the survey records them rather than
   patching around them.** Together they mean the run form is *not* a viable candidate as specified either:

   - **Its scan has no bound to be costed against.** A draft claimed `max_events_per_quantum` times the quanta a call
     renders. That cannot bound it: the mechanism fires **precisely when** the candidate set exceeds that per-quantum
     limit, and the [host-profile specification](../specs/spec-host-profile-and-render-limits.md) leaves the ingress
     and deferred-store capacities `Deferred to Phase 3`, so the candidate set has no declared bound at all. Marking
     F2 as passing put producer-sized work on the audio thread. **This restores the store dependency the survey
     claimed to have removed** — the run must be costed against those capacities, so *The deferral*'s "no input
     required" is wrong for this candidate.
   - **Destination overflow breaks the interval it was chosen to preserve.** Events at 63 and 65 translate to 127 and
     129; if quantum 2 is already full, 129 defers again to 193 while 127 has already rendered — an interval of 2
     samples rendered as 66. F6 fails unless the design also reserves destination capacity or moves the run
     atomically, and neither is specified.
   - **The run is not well defined across callbacks.** A callback ending at sample 64 processes the deferred event at
     63 without the successor at 65, which arrives in the next callback. A scan over the call span therefore forms a
     different run than a larger callback would, so the output depends on host-block partitioning — which is exactly
     what Phase 3's exit gate forbids. A single `SampleTime` per control does not say how a later callback's events
     inherit a translation, least of all after repeated deferrals.
   - **The key does not cover control-rate parameters.** Property 4 below cites `timed_target` and
     `timed_control_index`, but both **filter `SetParameter` to `ControlRate::Sample`** (`render/hot.rs:348-371`,
     read at `d1dd12a3`). Two events on one control-rate control resolve to nothing and stay unrelated, so they can
     still invert under the narrowed same-control question. The mechanism needs a rate-independent resolver those
     helpers do not provide.
2. **It preserves order and interval.** Translation by a constant is monotone, so successors keep their order among
   themselves and each keeps its exact distance from the predecessor. The record's own example: a note-on stamped at 63
   defers to 127 and its note-off at 65 to 129 — two samples apart before, two samples apart after (F6).
3. **Its state is one `SampleTime` per distinct `(node, control)`** — the position that control has been deferred to.
   The count is bounded by `note_targets().len()` plus `parameter_targets().len()`, since every event resolves through
   one of those two tables and both are fixed when admission builds the plan (`plan.rs`); admission can lower the
   resolved pairs to a dense index in the same pass, so the renderer indexes a preallocated array rather than
   searching (F2). Sizing the array by the sum and indexing it by the payload's own slot would be cheaper and wrong,
   for the reason stated above.
4. **The relation needs no new identity and no compiler declaration — but the helpers named here do not supply it.**
   The draft cited `timed_target` and `timed_control_index` (`render/hot.rs:341-372`) as deriving `(node, control)`
   for either payload. They do not: both filter `SetParameter` to `ControlRate::Sample`, so a control-rate parameter
   resolves to `None`. The *idea* that admission can resolve every payload to a rate-independent `(node, control)`
   stands, since both target tables carry the pair; the claim that the renderer already computes it for every event
   does not. This is the finding that makes the candidate costable now: the *relation* is a slot equality, not a
   relationship stored per event, and this record's earlier claim that successor propagation "adds relationship state"
   to the deferred store holds for 1a but not for 1b.
5. **It moves nothing backwards and rewrites no stamp** (F3, F4). It only ever adds `Q` to a render position.
6. **It drops nothing** (F5).

**What it does not cover — larger than the first draft claimed.** The relation is same-control, so two events on
*different* controls whose meaning depends on their order are not paired by it. Widening the key is not available on
the terms F2 sets: a general cross-control dependency is a graph the renderer would have to walk on the audio thread.

The draft then argued that `SOUND-INV-016` closed most of that residual, because a control-rate response begins at the
first quantum boundary at or after the render position, so a cross-control pair less than one quantum apart was never
ordered by sample position anyway. **That argument is false, and the independent read refuted it with a case the draft
had not evaluated.** A control-rate automation at 63 takes effect at boundary 64, *before* a sample-rate note at 65.
Defer the automation to 127 and it takes effect at boundary 128, *after* that note. The boundary rule does not protect
the pair; it relocates it. So the residual is not the narrow sample-rate-against-sample-rate case the draft named — it
is every cross-control order, at either rate. `ADR-0023` reaches none of it, for the reason this record already gives:
65 and 127 are not a tie.

#### 1c — collapse instead of translate

Clamp each successor to the predecessor's new position rather than translating it. **It fails F6**: the record's pair
becomes zero samples apart, so a note renders as silence. This is the same preserve-versus-collapse axis ADR-0043
settled for the late clamp, and the survey settles it the same way and for the same reason.

#### What 1b costs

**An ADR-0043 successor** (F8). `SOUND-INV-016` permits a render position to differ from a stamp for exactly two
reasons, and a translated successor is a third: its own quantum had room. The successor is small — it adds one reason
to a list of two, and changes no mechanism ADR-0043 selected — but `PROCESS.md`'s decision-timing rule requires the
coupled boundary to be decided in the same transaction rather than promised after it.

### Option 2 — refuse the deferral that would invert

#### 2a — a counted fault, in two variants a draft conflated

A draft of this survey treated refusal as one option and eliminated it by arguing that the renderer must find a
successor before it can know a deferral "would invert", so refusal costs 1b's machinery in order to do less. **The
independent read refuted that**: the argument holds only for a *conditional* refusal, and there is an unconditional
variant it does not touch. The two are separated here.

**2a-conditional — fault only when an inversion is detected.** The draft's argument stands against this one. It needs
the same successor lookahead 1b performs, and then discards an event instead of moving one. It is dominated.

**2a-unconditional — refuse every event that does not fit, and report the fault immediately.** No lookahead, no
successor relation, no state. And it repairs this record's hazard completely rather than partially: **nothing is ever
deferred, so nothing can be moved past anything.** It is the only construction in this survey that satisfies F1, and
it satisfies [ADR-0045](ADR-0045-cross-control-causal-order.md)'s cross-control hazard on the same grounds, because
both hazards are *deferral-induced* and it removes deferral.

**But it is not this record's to select, and that is the accurate reason to set it aside.** It is
[ADR-0043](ADR-0043-event-deferral-and-late-clamp.md)'s **Option A** — "No deferral. Capacity overrun is a fault" —
already surveyed in full there, with its cost worked out over several revisions: the over-full quantum stops being a
caller-contract violation and becomes a reachable runtime overload on the audio callback, which needs the terminal
fault fully specified (silence on this and every subsequent callback, carries invalidated, an atomic `needs_reprepare`,
nothing allocated, counters reported). The maintainer selected Option D against it on 2026-08-20. Choosing it now is
**not a repair to the causal-order hazard; it is a reversal of an accepted decision**, and this record's *Revisit
conditions* already name that as a distinct path from completing it.

**What the survey does owe the maintainer is that the reversal got cheaper — or rather, that Option D got more
expensive.** ADR-0043 chose D at a moment when the causal-order cost was named but not measured; its own *Negative*
consequences anticipated "the repair may turn out to be expensive enough that ADR-0043's selection is worth
revisiting". This survey is that measurement, and the answer is that D's repair costs an empty candidate space, a
second gating record in ADR-0045, and a mechanism that fails four frames. Option A's cost — a fully specified terminal
fault — has not changed. **The comparison the maintainer made on 2026-08-20 is therefore no longer the comparison in
front of them**, and this record cannot resolve that: an ADR-0043 successor can.

#### 2b — an admission-time reservation

This record names it as the possible exception that keeps a causal pair together without ever refusing a deferral, and
asks the survey to establish that rather than assume it. **It does not survive F7.**

For a *compiled* pair both edges are known when the first is admitted, so capacity for the off-edge can be reserved
then. For *live ingress* the off-edge does not exist yet — a player has not released the key — so there is nothing to
reserve against. Reserving a general release pool instead does not repair the hazard: it keeps off-edges from
deferring, and the inversion is caused by the **on**-edge deferring; an off-edge that renders on time into a gate the
deferred on-edge has not yet raised still leaves that gate latched when the on-edge lands. And a reservation that does
keep the compiled pair together is 1b restricted to the case where the successor is known in advance. So 2b is either
1b under another name, or a rule that behaves differently by provenance — which Phase 9's live/offline render
agreement forbids, and which ADR-0043's provenance-blind timing rule gives no basis for.

### Option 3 — neutralise it where the gate is latched

This record calls it the voice-allocator repair. Two facts move it before the frames are applied.

**The component is Phase 6's and the gate is Phase 3's.** [`ROADMAP.md`](../ROADMAP.md#phase-order) gives Phase 6
"bounded voice allocation, stealing", and V2 has no allocator today: read at `d1dd12a3`, `voice` occurs in
`synth_engine_v2/src` only as declared counts in the IR and `HostProfile`, the `VoiceCount` and `HeldNoteCount`
quantities, `ResourceReport` field names, EVD-0003's cost model, and prose — no code assigns a note to a voice. The
repair as named cannot be implemented in the phase whose entry it must clear.

**It has a Phase 3 form, and that form is what the survey weighs.** A note today is a gate raise or lower on a
statically compiled control — `NoteTarget` is a node and a control (`plan.rs:563-568`) and `NoteEdge::value` is one of
two constants (`render.rs:181-192`) — so the inversion's Phase 3 symptom is a latched gate rather than an orphaned
voice, and the analogous repair is a per-slot latch: an off-edge applied to a slot whose on-edge has not been applied
is remembered, and cancels that on-edge when it lands. One bit per control slot, preallocated. It passes F2 through F7
and F9, and it needs **no ADR-0043 successor**, because it moves nothing. That is the one real advantage in the survey.

**It fails F1, and that is decisive.** A latch pairs a raise with a lower. It does not repair a retrigger deferred past
the edge it retriggers, and it repairs no pair whose members are not opposite edges of one gate — both of which this
record's *Context* puts in scope. What it does is convert the audible symptom from a stuck voice into a missing note.
That is a better failure, and a better failure is not an invariant. Question 4 asked whether this is a legitimate
repair or a way of hiding a broken order; on the frames it is the second.

**A draft of this survey also claimed the latch covers ADR-0021's queue drop, and that claim was false.** The
independent read caught it: a dropped off-edge never reaches the renderer, so a latch that remembers *early* off-edges
has nothing to remember, and the raised gate stays raised. The latch addresses one thing only — an off-edge that
renders before its on-edge. Covering a lost off-edge needs a producer-side or queue-side policy, which is neither this
record's nor this survey's.

### Against the frames

**No candidate passes F1.** That is the survey's result, and it is stated before the table rather than softened
inside it. F1 asks a repair to hold for *every* pair whose meaning depends on order; each candidate is scoped to some
proper subset — a note pair, a control, a gate — and the cross-control order is outside all of them. A first draft
marked 1b as passing F1 "with a declared cross-control limit", which is not a pass but a contradiction of the frame,
and the independent read refused it on exactly that ground.

| | F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 |
|---|---|---|---|---|---|---|---|---|---|---|
| **1a** identity | **fails** — note pairs only | costs store state | pass | pass | pass | pass | pass | successor + ingress contract | pass | yes |
| **1b** resolved `(node, control)`, run form | **fails** — same-control only, and not even all of that: control-rate parameters resolve to nothing; and it converts some strict orders into ties `ADR-0023` does not yet own | **fails as specified** — the scan's bound is exactly the capacity Phase 3 has not chosen | pass | pass | pass | **fails** — a re-deferred tail event stretches the interval (2 samples rendered as 66) | pass | successor, small | terminates within one call, but the run itself is **partition-dependent** across callbacks | yes, plus a partition case and a three-event case the first draft's test missed |
| **1c** collapse | **fails** — same-control only | pass | pass | pass | pass | **fails** | pass | successor, small | pass | yes |
| **2a** conditional fault | **fails** — same-control only | pass | pass | pass | needs a new post-admission permission | n/a — it moves nothing | pass | successor, replaces `+Q` | pass | yes |
| **2a** unconditional fault (= ADR-0043 Option A) | **pass** — nothing is deferred, so nothing can invert | pass — no state at all | pass | pass | a counted, reported fault, but post-admission | n/a — it moves nothing | pass | **reverses ADR-0043**, and needs its terminal-fault recovery specified | pass | yes |
| **2b** reservation | **fails** — known pairs only | pass | pass | pass | pass | pass | **fails** | successor, replaces `+Q` | pass | yes |
| **3** latch | **fails** — one gate's two edges only | pass | pass | pass | pass | n/a — it moves nothing | pass | **none** | pass | yes |

### Recommendation

**The survey does not recommend a candidate, because under F1 as written none of them qualifies.** A first draft
recommended 1b; the independent read found two conclusion-affecting defects in it — a new same-control inversion the
one-window rule created, and a cross-control residual larger than the draft claimed — and both are confirmed by
evaluation rather than by argument. Recording that outcome is more useful than weakening the frame until a winner
reappears, which is the failure mode `PROCESS.md`'s design protocol exists to prevent: a frame relaxed *after* seeing
which candidate it kills is no longer an independent test.

**What the survey did settle**, and what a later pass should not redo:

- **1c and 2b are eliminated on their merits**, by F6 and F7, independently of how F1 is scoped. 1c collapses a pair
  to zero length; 2b cannot be stated for live ingress, where the second member of a pair does not exist yet.
- **2a-conditional is dominated** — it needs the same successor lookahead in order to discard an event rather than
  move one. **2a-unconditional is not**, and a draft that eliminated both together was wrong: it passes every frame
  including F1, because it removes deferral rather than repairing it. It is out of scope here only because it is
  ADR-0043's Option A, rejected on 2026-08-20 — a reversal of that record, not a completion of this one.
- **1a is dominated by 1b**: it covers a strict subset of the pairs, needs a wider ingress contract, and stores a
  relation per event where 1b compares a key the renderer already computes.
- **Option 3 is not the repair**, and the one argument that made it attractive beside a repair — that it also covered
  ADR-0021's queue drop — is false, as recorded above.
- **The best-scoped mechanism is 1b in its run form**, and its true cost is now on the record: a bounded scan, an
  interaction with per-quantum capacity, and a small ADR-0043 successor.

**The scope question this survey handed back was answered on 2026-08-24: the record narrows to same-control causal
order**, and the cross-control remainder is [ADR-0045](ADR-0045-cross-control-causal-order.md). That split stands. What
did **not** survive is the draft's accompanying claim that ADR-0045 needed no gate; a second independent read
established that Phase 3's *Outcome* names automation ordering and that the symptom persists in stateful DSP, so
ADR-0045 is a Phase 3 entry prerequisite too. The gate therefore has **three** conjuncts now, not two.

**No candidate is recommended, and the run form is not one.** The narrowing left 1b's run form as the only mechanism
in scope, and the second read found four defects in it — an unbounded scan, a broken interval under destination
overflow, partition-dependence across callbacks, and a key that misses control-rate parameters. They are recorded
above in full. Two of them are repairable in principle by deciding the deferred store's capacities and by reserving
destination capacity; the partition-dependence is the one that looks structural, because it makes the mechanism's own
input depend on how the host divides the work.

**What the survey therefore establishes, which is less than it set out to and more than the record had.** Within this
record's own boundary the candidate space is **empty as specified**: 1a, 1c, 2a-conditional, 2b and option 3 are
eliminated on their merits, and the one construction left standing fails four frames. The next step is not a
selection. It is one of three things, and the third is new:

1. the deferred store's capacities decided first, so the run form can be costed and its destination reservation
   specified — which reverses this record's long-held claim that it did not depend on the store; or
2. a mechanism nobody has proposed; or
3. **a successor to [ADR-0043](ADR-0043-event-deferral-and-late-clamp.md)** that reopens its Option A. Removing
   deferral dissolves this record and [ADR-0045](ADR-0045-cross-control-causal-order.md) together, since both hazards
   are deferral-induced. That is not this record's decision to take, and it is the one path the survey's result makes
   more attractive than it was when D was selected.

**Two things this survey did not do, stated so nobody reads them into it.** It did not measure how often a quantum
actually overruns — nothing in V2 can, since no ingress exists — so it cannot say whether any of this is reachable
under realistic load. And it did not re-review ADR-0043's Option A on its merits; it only established that the
comparison which rejected that option has changed on one side.

**The conformance test** (question 5). A live render that presents a note-on at sample `Q - 1`
into a quantum already filled to `max_events_per_quantum` and its note-off at sample `Q + 1`, asserting an exact value
per frame across three quanta: non-zero between the pair's rendered positions and zero after the second. It fails on a
latched gate, on a zero-length note, and on a swallowed note, which is what makes it discriminate between the
candidates rather than merely detect the original hazard. It belongs beside `render_contract`'s
`a_late_note_edge_takes_effect_at_its_clamped_render_position`, the existing test of that shape for the clamp. **It
does not cover the new inversion the read found**; a second case — three same-control events straddling two quanta —
is what covers that, and a repaired 1b must be tested by both.

## Decision

**Deferred to the Phase 3 entry gate**, with the owner and required input recorded in *The deferral* above.

Two constraints hold in the meantime, so the deferral cannot be used as permission to improvise:

1. **No implementation may invent a causal-order rule.** Until this record is `Accepted`, no code may defer an event,
   and therefore no code may need one.
2. **No specification may narrow ADR-0043 by glossing it.** The host-profile specification states the deferral rule as
   ADR-0043 decided it and records this hazard as an open obligation; it does not describe a repair that has not been
   chosen.

## Consequences

### Positive

- The hazard is written down at the point where it was created, rather than surviving as a sentence inside the record
  that created it.
- The Phase 3 entry gate names every conjunct, so nobody can read ADR-0043's acceptance as clearing it. The survey
  added a third — [ADR-0045](ADR-0045-cross-control-causal-order.md) — rather than removing one, which is a worse
  position honestly stated instead of a narrower gate wrongly claimed.
- The policy gets its own independent review instead of arriving as a subordinate clause of a timing decision.

### Negative

- Phase 3's entry gate had two conjuncts when this record was opened, and the second changed character: ADR-0043
  discharged the ADR-0001 obligation and this record took its place, so what was a decision needing no new work became
  unstarted design. **The survey has since made it three**, by establishing that cross-control order is a separate
  question that also gates the phase. The distance to the gate grew; it did not shrink.
- The repair may turn out to be expensive enough that ADR-0043's selection is worth revisiting, which would mean
  reopening an accepted record rather than merely completing it. **The survey did not find that**, but it did find
  that both scheduler-side candidates need a successor to ADR-0043, so the recommended one adds a second record to the
  gate's acceptance transaction rather than avoiding one.

### Risks and controls

- **Risk: the gate stalls** because nobody starts the design, or because this record is read as waiting on the
  deferred store when no candidate the survey recommends does. Control: this record is a named Phase 3 entry
  prerequisite in [`NOW.md`](../NOW.md) and in the decision index, on the same footing as ADR-0022, and the survey
  below is complete. The control is weaker than a draft of it claimed: the survey ends with **no candidate**, and the
  surviving construction must be costed against the deferred store's capacities, so what remains is design rather than
  a selection.
- **Risk: a repair is implemented informally** inside a Phase 3 task and the record is written afterwards to match.
  Control: constraint 1 above, and the fact that no deferral code exists to grow one.
- **Risk: the hazard is quietly downgraded** to "rare under realistic load". Control: rarity is not the test — the
  symptom is a stuck voice, and `PROCESS.md` requires a named automated test rather than a frequency argument.

## Follow-up work

| Task | Phase | Status |
|---|---|---|
| Survey the candidate repairs | 3 | **Complete** — [*Options considered*](#options-considered), 2026-08-24. It eliminates 1a, 1c, 2a, 2b and option 3, and selects none: no candidate passes F1 |
| Decide the record's scope | 3 | **Done** 2026-08-24 — narrowed to same-control order; cross-control split out as [ADR-0045](ADR-0045-cross-control-causal-order.md) |
| Take one independent read of the corrected 1b run form | 3 | **Done** 2026-08-24 — four defects, recorded in [*Options considered*](#options-considered). The run form is not viable as specified |
| Decide the deferred store's capacities, or propose a mechanism that needs none, **or reopen ADR-0043's Option A** | 3 | Not started — the survey's actual next step. The first reverses this record's earlier independence claim; the third dissolves this record and ADR-0045 together and belongs to an ADR-0043 successor |
| Name the conformance test that fails on an inverted pair | 3 | **Named** in [*Recommendation*](#recommendation); writing it is Phase 3 implementation work |
| Select a candidate, and accept the ADR-0043 successor the selected one needs, in one transaction | 3 | Not started — the maintainer's call |
| Write and accept this record | 3 | Not started |
| Choose the deferred store's shape | 3 | Not started — no longer a prerequisite of this record under the recommended candidate, and still Phase 3's own work |

## Revisit conditions

This record is not a decision, so it has no revisit condition in the usual sense. It is superseded by its own accepted
version at the Phase 3 entry gate. It would be revisited *earlier* only if the selected repair proved expensive enough
to change ADR-0043's selection, which would mean accepting a successor to ADR-0043 rather than completing this record.

## Review

Reviewer: the record as first drafted was reviewed as part of
[ADR-0043](ADR-0043-event-deferral-and-late-clamp.md)'s acceptance transaction, which is where the hazard was found and
where the choice to carry it as a separate record was made. **[*Options considered*](#options-considered) is new
material and carries its own independent read**, taken on the uncommitted change under `AGENTS.md`'s normative-document
row. That read covers the frames, the candidate analysis, and the claims the survey makes about current source.

It returned three blocking findings, and **all three were confirmed rather than argued with** — the first two by
evaluating the counterexamples, not by rereading the reasoning:

1. the one-window translation rule created a **new** same-control inversion (63, 65, 128 at `Q` = 64 renders as 127,
   129, 128), so the draft's single-application proof and its F1 and F9 claims were false;
2. the cross-control residual was **larger** than the draft claimed — a control-rate automation at 63 takes effect at
   boundary 64 before a sample-rate note at 65, and at boundary 128 after it once deferred — so the argument that
   `SOUND-INV-016` closed most of the residual was false;
3. the latch does **not** cover ADR-0021's queue drop, because a dropped off-edge never reaches a latch at all.

The repairs withdrew the recommendation rather than rescuing it, and restated F1's result as "no candidate passes".
A second read then covered the corrected run form and the scope split, and returned four more blocking findings,
**all six confirmed rather than argued with** — two of them against source: `timed_target` and `timed_control_index`
filter `SetParameter` to `ControlRate::Sample`, and the sine kernel's phase accumulator persists across quanta, which
falsified this survey's "transient symptom" premise for ADR-0045. The repairs withdrew the run form as a candidate
rather than rescuing it.

A third read covered the withdrawal and the scope split. It returned one blocking finding and four secondary ones, and
**all five were confirmed**: ADR-0045 had been declared a Phase 3 prerequisite without reaching the authorities that
gate the phase (`master-plan.md`'s Phase 3 section and exit checklist, `NOW.md`'s prerequisite table, and the
host-profile specification), three summaries still carried conclusions the repairs had superseded, and — the one that
changed a result — **this survey's elimination of the counted fault used an argument valid only for a conditional
variant.** The unconditional variant needs no lookahead, passes every frame including F1, and is
[ADR-0043](ADR-0043-event-deferral-and-late-clamp.md)'s Option A. Splitting it out is what produced this record's third
path above, and it is the survey's most consequential correction: without it the record would have concluded that
nothing repairs the hazard, when in fact something does and it lies outside this record's boundary.

Across three reads the survey took **fourteen** findings and confirmed all fourteen; four were checked against source
rather than argued. **The repairs from the third read have not themselves been re-read**, and that is the deliberate
stopping point under `PROCESS.md`'s stopping rule: they add no new claim about the mechanism, and the record's
conclusion is that it has nothing to select.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
