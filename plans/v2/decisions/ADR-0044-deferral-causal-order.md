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

> **Deferred, not open for improvisation.** [ADR-0043](ADR-0043-event-deferral-and-late-clamp.md) is `Accepted` and
> creates the hazard this record must close. It named the hazard rather than solving it, because the repair is
> scheduler design and ADR-0043's decision boundary excludes it. **Phase 3 implementation may not begin before this
> record is `Accepted`**, alongside [ADR-0022](ADR-0022-hardware-time-mapping.md).

## The deferral

| Field | Value |
|---|---|
| Deferred to | The **Phase 3 entry gate**. Phase 3 implementation may not begin before this is `Accepted` |
| Owner | Project maintainer — this is a single-maintainer repository, so there is no second party to assign |
| Input required | **For successor propagation only** — the Phase 3 ingress and deferred-store contract, because deferring an event's causal successors adds relationship state to that store and cannot be costed against a container nobody has shaped. **The other two candidates do not depend on it**: a refusal, whether a counted fault or an admission-time reservation that keeps a pair together, adds no store state, and remembering an early note-off is allocator state. So the survey may begin now and this record is not waiting on the store. Two earlier drafts over-claimed this dependency — first for every candidate, then for both scheduler-side ones — and the option space below contradicted each |
| Why not now | The candidate repairs are scheduler and voice-pool design, not timing semantics, and ADR-0043's decision boundary excludes them. Deciding them inside that record would have made a timing decision carry a scheduler design its reader was not reviewing |
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
   narrower. **Selecting this candidate requires a successor to
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

## Options considered

**Deliberately not surveyed.** The candidate space is visible — defer the successors with the predecessor; refuse the
deferral that would invert; repair at the voice allocator. Only the **first** costs relationship state in a deferred
store whose shape Phase 3 has not chosen, so only its cost is uncostable today; the refusal and the allocator repair
can be weighed now — though both scheduler-side candidates carry a second record with them, since each changes
which events ADR-0043 permits the renderer to move. Only the allocator repair does not. **The survey is therefore not blocked**, and this record does not claim it is. What it is deferred
on is the design work itself, which is scheduler and voice-pool territory that ADR-0043's boundary excludes and that
deserves its own frames and its own independent reader rather than arriving as a subordinate clause of a timing
decision. An accepted record's options section is supposed to record why the winner won.

What is **not** open, and is recorded here so the survey starts from it:

- clause 12's second sentence stands — no event is ever applied to samples already produced, and no repair may reach
  backwards to fix an order;
- the envelope's `time` stays immutable, so a repair may not re-stamp an event to reorder it;
- a repair may not allocate, lock, or block on the audio thread;
- a repair may not silently drop an event the renderer has already admitted, which `HOST-INV-019` forbids;
- a repair that changes **which events the renderer may move, or why**, changes ADR-0043's rule and needs a successor
  to that record. `SOUND-INV-016` permits exactly two reasons for a render position to differ from a stamp — the late
  clamp, and deferral out of an over-full quantum — so **both scheduler-side candidates need that successor**:
  refusal replaces the `+Q` displacement, and successor propagation moves an event whose own quantum had room. Only
  the voice-allocator repair reaches neither, because it moves nothing. That asymmetry is a cost the survey weighs; it
  is not a reason to prefer the allocator candidate before the survey runs.

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
- The Phase 3 entry gate now names every conjunct, so nobody can read ADR-0043's acceptance as clearing it.
- The policy gets its own independent review instead of arriving as a subordinate clause of a timing decision.

### Negative

- Phase 3's entry gate still has two conjuncts, but the second one changed character: ADR-0043 discharged the
  ADR-0001 obligation and this record took its place, so what was a decision needing no new work became unstarted
  design. The count is unchanged; the distance to the gate is not.
- The repair may turn out to be expensive enough that ADR-0043's selection is worth revisiting, which would mean
  reopening an accepted record rather than merely completing it.

### Risks and controls

- **Risk: the gate stalls** because nobody starts the design, or because this record is read as waiting on the
  deferred store when only **one** of its three candidates does. Control: this record is a named Phase 3 entry
  prerequisite in [`NOW.md`](../NOW.md) and in the decision index, on the same footing as ADR-0022, and *The deferral*
  above scopes the store dependency to successor propagation alone. The survey can start today.
- **Risk: a repair is implemented informally** inside a Phase 3 task and the record is written afterwards to match.
  Control: constraint 1 above, and the fact that no deferral code exists to grow one.
- **Risk: the hazard is quietly downgraded** to "rare under realistic load". Control: rarity is not the test — the
  symptom is a stuck voice, and `PROCESS.md` requires a named automated test rather than a frequency argument.

## Follow-up work

| Task | Phase | Status |
|---|---|---|
| Choose the deferred store's shape, so the candidate repairs can be costed against it | 3 | Not started |
| Survey the candidate repairs and select one | 3 | Not started |
| Name the conformance test that fails on an inverted pair | 3 | Not started |
| Write and accept this record | 3 | Not started |

## Revisit conditions

This record is not a decision, so it has no revisit condition in the usual sense. It is superseded by its own accepted
version at the Phase 3 entry gate. It would be revisited *earlier* only if the selected repair proved expensive enough
to change ADR-0043's selection, which would mean accepting a successor to ADR-0043 rather than completing this record.

## Review

Reviewer: reviewed as part of [ADR-0043](ADR-0043-event-deferral-and-late-clamp.md)'s acceptance transaction, which is
where the hazard was found and where the choice to carry it as a separate record was made.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
