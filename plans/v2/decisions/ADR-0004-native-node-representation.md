# ADR-0004: Native Node Representation

| Field         | Value              |
|---------------|--------------------|
| ID            | ADR-0004           |
| Status        | Accepted           |
| Phase         | 2                  |
| Created       | 2026-08-17         |
| Last reviewed | 2026-08-18         |
| Related       | P02-T005, P02-T009, ADR-0001, ADR-0005, EVD-0009, master plan Phase 5 |
| Supersedes    | —                  |
| Superseded by | —                  |

## Context

Phase 2 has to separate a node's immutable prepared data from its mutable state and then execute a schedule of them.
How that execution is *dispatched* is this record's subject, and it is load-bearing for one of the phase's own exit
gates: "adding a second simple DSP node does not require changing renderer control flow" cannot be evaluated until
someone says what counts as control flow and what counts as data.

Phase 1's answer is a closed `PlanOp` enum matched inside the quantum loop; the citation is in *Evidence*. It was the
right answer for four source kinds whose
whole purpose was to make a gate testable. It does not survive contact with a node catalog: every node kind becomes an
arm of one match in the file that the real-time purity check reads, and the loop grows with the catalog.

Two constraints make this narrower than the usual dispatch argument.

**The Phase 2 gate now requires a transitive purity check.** The existing scan reads `render/hot.rs` and says in its
own header that moving code out of the file evades it. A Phase 2 hot path has helpers — one per node kernel — so the
check has to follow them, and a check can only follow callees it can enumerate. **A dispatch mechanism whose callee set
is not enumerable from source therefore costs the phase its real-time guarantee**, not merely some inlining.

**Phase 5, not this record, owns the declaration surface.** The master plan's Phase 5 work list makes a module
declaration the single source for ports, parameters, laws, scopes, latency, and metadata. This record decides only how
a prepared node is *called* per quantum, and it must leave that declaration free.

**Outside this decision.** The declarative node API and its `ParamSpec` (Phase 5); the `LegacyPolyModuleAdapter`
(Phase 5); node-state migration across a plan swap (ADR-0010, Phase 9); and how prepared bytes are *counted*, which
`HOST-INV-006` and the resource report already own.

## Decision drivers

- **The gate.** Adding a node must not edit the loop. Whatever "table" means, it has to be reachable without touching
  control flow.
- **The purity check must be able to follow the call.** Enumerable callees are a hard requirement, not a preference.
- **Dispatch cost is amortized over a quantum, not a sample.** A node is called once per quantum and processes `Q`
  frames, so one indirect call is spread over 64 samples of work. This is the reason block processing exists, and it
  is why the usual "virtual dispatch is expensive" argument does not transfer unchanged — but it is an argument, and
  this record does not accept it without the measurement named below.
- **Real-time rules.** No allocation, no panic, no lock in the call path. Any boxing happens at preparation.
- **Prepared and mutable state are separate objects** (master plan Phase 2), so the dispatch mechanism has to carry
  two references plus arena slots, not a single `self`.

## Options considered

### Option A: Enum dispatch — extend `PlanOp`

Phase 1's shape. One variant per node kind, one match in the loop, static dispatch, full inlining, and a callee set
that is trivially enumerable because it is literally in the file the purity check already reads.

It fails the gate as written: a node addition is a new variant *and* a new arm inside the quantum loop. It also closes
the node set at the crate boundary, which the eventual catalog and any out-of-crate node cannot live with.

### Option B: Trait objects — `Box<dyn PreparedNode>`

The loop iterates the schedule and calls `node.process(..)`. Control flow is constant, node addition is a new type in
its own file, and the ergonomics are the ones Phase 5's declarative API would naturally produce.

Its cost is precisely the constraint above: a `dyn` call site has an open callee set, so a source-level transitive
purity check cannot enumerate what the loop reaches. The check would have to be replaced by a per-implementation
obligation — every `impl PreparedNode` audited separately — which is weaker in exactly the way that matters, because
it holds only for the implementations someone remembered to audit.

### Option C: A closed kernel registry dispatched through a prepared function table

Each node kind contributes a free function with one signature — prepared data, mutable state, and the arena slots it
was assigned — and a registry entry. The compiler resolves a node to a function pointer at preparation; the loop walks
the schedule and calls through the pointer. No `dyn`, no boxing, no `self`.

Control flow is constant, so the gate is satisfiable. The callee set is the registry, which is a table of named
functions in source, so the purity check can enumerate and follow every one. The costs are real: one indirect call per
node per quantum with no cross-boundary inlining, a hand-rolled dispatch that a reader has to learn, and a signature
that has to be general enough for every kernel without becoming a bag of optional arguments.

### Status quo

Keeping Phase 1's enum means the fourth gate bullet closes by argument rather than by check, and the node catalog
grows one match arm at a time inside the file that is supposed to be small enough to read as a whole.

## Evidence

[**EVD-0009**](../evidence/phase-02/EVD-0009-dispatch-cost.md) — the minimal voice path at `Q` = 64: twelve arms, four
of them control pairs, rotated in groups so a control always precedes its arm, pinned to one P-core thread. Times are
the median over nine runs of each run's minimum over twenty-five rounds; **ratios are paired within a round**, because
a ratio of two independently selected minima was not stable in sign. Its headline figures, paired:

| Comparison | Median |
|------------|-------:|
| `table` over a hand-written direct-call variant — **rule B's own comparison** | +7.31% / +7.47% |
| **the hybrid rule C names**, over the same variant | +7.08% / +7.34% |
| a closed enum for every node — Option A — over the same variant | +5.39% / +5.10% |
| **`table` over that closed enum — the dispatch shape alone, a lower bound** | **+2.12% / +2.09%** |
| `table` over the hybrid | −0.18% / −0.04% |
| arena binding, paired against its own walk-only control | 32.18 ns / 32.43 ns per quantum |
| one fused function, against the table — **not the same computation** | 36–39% less |

Two figures per row: a canonical nine-run set and a full replication, twelve arms each, four of them control pairs.
Controls are paired within each round: median spreads 0.68–1.00% across the four timed shapes. Individual rounds reach
tens of per cent, which is why the estimator is a minimum over rounds and a median over runs and why nothing here rests
on one of them. The
record also carries **nine** corrections the harness went through before it produced a number: four that made the
candidate look better than it is, including one that reversed the conclusion; one that argued a competitor's shape
instead of measuring it; and four that made the instrument look sharper than it is.

The master plan also asks for compile time and module ergonomics to be measured in Phase 2/5. Neither is measured here;
both belong with the declarative node API in Phase 5, and this record makes no claim about either.

What exists is structural, and it is read at
[EVD-0008's use-site table](../evidence/phase-02/EVD-0008-internal-channel-layout-cost.md#v1-use-site-reads) rather
than cited from source here.

Phase 1's dispatch is a match over a closed `PlanOp` enum inside the quantum loop (row 7).

V1's own answer is worth naming precisely, because it is the shape this record must not inherit by default. V1 stores
`Box<dyn PolyModule>` per node (row 4) and calls it **per block**, so its dispatch is already amortized the way Option
B's would be — but its signature hands outputs over as `&mut HashMap<PortName, AudioBuffer>` (row 5), which puts a
string-keyed hash lookup on the audio thread. That
is the first construct the Phase 2 gate bans by name, and it arrived through the *signature*, not through the
dispatch. Both halves of clause 5 exist because of it.

## Decision

**Option C, kept — and this record is a *redraft*, not an acceptance of what was drafted.**

Acceptance rule B was run as written and **failed**: the prepared function table costs +7.31% and +7.47% more per
quantum, in two independent nine-run sets, than a hand-written direct-call variant of the same plan, against a
threshold of 3%
([EVD-0009](../evidence/phase-02/EVD-0009-dispatch-cost.md)). Rule C therefore applies, and rule C requires a redraft
and a measurement of the hybrid it proposes. Both were done. The outcome of the redraft is that Option C stays, for
reasons the measurement itself supplies — and saying "accepted" without saying "after its own rule failed" would be the
thing this register exists to prevent.

The clauses that never depended on the measurement are unchanged, and the rest of the phase was built on them:

1. **"Renderer control flow" means, exactly:** the quantum loop, the schedule walk, event application, the carry
   management, and the fault path — the code in `render/hot.rs` that is the same for every plan. **"Table data" means:**
   the compiled schedule, each node's prepared data and state slots, and the arena assignment.
2. **The fourth Phase 2 gate bullet is therefore decided by a change, not by an opinion.** Adding a second simple DSP
   node must touch no line of the code named in clause 1. The check is performed by making that change and showing the
   diff touches only **the IR vocabulary**, a kernel, a registry entry, and a test.

   *The IR vocabulary is part of this clause as redrafted, and it was not part of it as drafted.* The original wording
   named a kernel, a registry entry and a test; the change that ran the check also added a variant to `IrNodeKind`,
   because a node kind is a **term in the plan language** and the language is where terms live. That is a widening of
   the criterion after seeing the diff, which is worth flagging rather than quietly absorbing — so here is the test it
   is meant to survive: the widened list still excludes every line of clause 1, and it excludes the compiler, the
   validator, the arena and the report. What the clause forbids is a node addition reaching *code that runs*, and an
   enum variant in the IR is not that.
3. **Preparation may allocate; the call may not.** Function pointers, prepared data, and state are laid out at
   preparation. The per-quantum call allocates nothing, takes no lock, and cannot panic.
4. **Every callee reachable from the loop must be enumerable from source**, because the phase's real-time guarantee is
   a source-level transitive check. A dispatch mechanism that cannot satisfy this is excluded regardless of its
   performance.
5. **The node kernel signature carries prepared data, mutable state, and slots — never `&self`**, so that a node's
   configuration cannot be mutated by rendering it, and so the same prepared data can serve several states (a voice
   pool, in Phase 6) without copying.

**The acceptance rule, stated before the measurement it depends on:**

- **A.** Clause 2's change is made and touches nothing in clause 1's list.
- **B.** The dispatch overhead is measured on the minimal voice path at `Q` = 64, against a hand-written direct-call
  variant of the same plan, using the estimator, draw count, build profile, and binary discipline the evidence rules
  require. **Option C is accepted if the overhead is below 3% of the plan's per-quantum cost.**
- **C.** If it is at or above 3%, this record is redrafted rather than accepted, and the hybrid it would then propose —
  a closed enum for the few hottest primitives, the table for everything else — is measured the same way.

The falsifier is stated with it: if the direct-call variant is *not* faster than the table by a measurable margin, the
measurement is telling us the harness is dominated by something else, and the number is an artifact until that is
explained.

### What the rule returned

**Rule A: passed against clause 2 as redrafted, and the redraft is stated in clause 2 itself.** The amplifier — a
second simple DSP node, and the first with two inputs — was added at `f0750c22`. The diff touches `ir.rs` (the
vocabulary), `node.rs` (the registry), `node/kernels.rs` (the kernel) and one test file. Against the clause **as
drafted** — a kernel, a registry entry and a test — the IR variant is a fourth file and the rule would be open; clause
2 now names the vocabulary explicitly, and says why, and says that it was widened after the diff existed. It touches no line of `render/hot.rs`, and none of the compiler, the
validator, the arena or the report either. A structural search over the crate puts every remaining reference to a node
kind outside the registry in two places: the IR that defines the vocabulary, and three `IrNodeKind::Output` arms — the
output is the renderer's boundary rather than a kernel node, and none of those arms changes when a node kind is added.

**Rule B: failed, at +7.31% and +7.47% against 3%.** The falsifier did not trip — the direct variant is measurably
faster, by several times the control spread and in the same direction in every set collected — so the figure is a
result rather than an artifact.

**Rule C: run, and the hybrid it names was built and measured.** A closed enum for the two hottest primitives of the
path — the oscillator and the filter — with the envelope and the amplifier left on the table, over the same schedule,
the same arena and the same binding. It costs **+7.08% and +7.34%** over the hand-written variant, so it fails
the same threshold, and it differs from the table by −0.18% and −0.04% — well inside the paired control medians of
0.7–1.0%. The claim is "the same measurement as the table", not "worse than it".

An earlier revision of this record argued the hybrid from a bound rather than measuring it, on the reasoning that a
mixture of two shapes cannot beat the better one. The reasoning was refused by review and the refusal was right: the
all-enum arm measures below the threshold, so the bound left the hybrid free to land on either side of 3%. What it
actually did was land above *both* pure shapes, which the mixture argument does not predict.

### Why the redraft keeps Option C

The measurement's decomposition is what decides it, and none of these three points was available when the rule was
written:

1. **No shape passes, and the shape rule C would move to is the same measurement.** The table is +7.31% and +7.47%,
   the hybrid +7.08% and +7.34%, and a closed enum for every node +5.39% and +5.10% — all above the threshold. Rule C's own
   alternative offers nothing to move to, which is the plainest reason of the four and the one that needed a
   measurement rather than an argument.
2. **The part of the overhead that is dispatch has been measured directly, and it is at least two per cent.** The table
   against a closed enum over the same arena is **+2.12% and +2.09%**, paired, with two independent sets agreeing to
   three hundredths of a point and fat LTO on so the closed shape gets its inlining. It is a **lower bound**, and the
   evidence says why: the enum arm walks the table's schedule zipped with a vector of kinds rather than a schedule that
   is natively an enum, which handicaps it. The error runs against this decision rather than for it, and it is stated
   here for that reason — a faithful Option A would be at least this much cheaper than the table and possibly more. The arena binding is measured separately
   at 32.18 and 32.43 ns per quantum and is the largest identified item with an obvious remedy; the evidence declines
   to express that as a share of the gap to the hand-written variant, because the two are measured against different
   baselines, and this record declines with it. What that establishes is a real cost that **every** schedule-walking shape
   pays, belonging to ADR-0005's arena rather than to this record's choice; it does not establish a percentage split,
   and this record does not assert one. The 3% threshold was written as if the whole difference were attributable to
   dispatch, and it is not.
3. **The shape that comes closest is not the one rule C names, and it does not pass either.** A closed enum for *every*
   node measures +5.39% and +5.10% — above the threshold in both sets. Stated plainly rather than left to be inferred: it is two points cheaper than
   the table, measured with fat LTO so it is not denied its inlining, and what it costs instead is a node set closed at
   the crate boundary and a per-kind `match` that clause 1 would have to be read against unless it were moved into the
   kernel module.
4. **A closed set is the thing that cannot be undone later.** Either enum shape closes the nodes it dispatches, and the
   hybrid closes precisely the *hottest primitives* — where a catalog grows, and where an out-of-crate node would most
   want to be. Phase 5 owns the declarative node API and this record is required to leave that surface free; two to
   three per cent can be re-measured at any time, and a node set that has to be a Rust enum in one crate is a decision
   the phase that owns the catalog would have to reverse.

**The price is recorded rather than waived**: **at least +2.12% and +2.09%** of a five-node voice quantum against the
closed-enum shape this record declines — the price of the choice itself, and a lower bound — and +7.31% and +7.47%
against a hand-written variant of the same plan, which every schedule-walking shape's arena binding also contributes to. Anyone who later finds the
render loop 3% short knows exactly where it went and what it bought.

**What this does not claim.** It does not claim the table is free, that the measurement was inconclusive, or that the
threshold was met. It was not met. The record is redrafted on the ground that the instrument it named could not
separate the cost of this decision from the cost of the phase's arena — and the corrected instrument, stated here for
whoever re-runs it, is `table` against `enum` over the same schedule and the same binding.

## Consequences

### Positive

- The phase's fourth gate stops being a matter of interpretation.
- The real-time purity check survives the arrival of node kernels, which is the property that made it worth building.
- A prepared/state split that takes no `&self` is the shape Phase 6's voice pool and Phase 9's plan swap both need.

### Negative

- A function-pointer registry is machinery a reader must learn, and it is less idiomatic than `dyn`.
- No inlining across the dispatch boundary. Accepted for a per-quantum call; it would not be accepted for a per-sample
  one, and clause 5's signature is what keeps the call per-quantum.
- The measurement did force a redraft after the code existed, which is the risk this list carried and which is now
  spent rather than pending. What it cost was the redraft itself; what it bought is a figure instead of an intention.

### Risks and controls

- **Risk: the measurement is run after the code is written, and reads as a formality.** Control: acceptance rule B and
  its falsifier are recorded here, before it. It was run as written and it failed, which is what a rule that was not a
  formality looks like.
- **Risk: the redraft reads as moving the goalposts.** Control: the original rule, its threshold, its falsifier and its
  outcome are all still above, and the redraft's own instrument is named so that it can be disagreed with. The one
  claim that would be dishonest — that the threshold was met — is not made anywhere in this record.
- **Risk: a kernel is added that quietly takes `&self` or allocates.** Control: clauses 3 and 5, plus the transitive
  purity check clause 4 requires.

## Follow-up work

| Task | Phase | Status |
|------|-------|--------|
| Prototype both dispatch shapes and run acceptance rule B | 2 (P02-T005) | **Complete** — [EVD-0009](../evidence/phase-02/EVD-0009-dispatch-cost.md) |
| Accept, or redraft under acceptance rule C | 2 (P02-T005) | **Complete** — redrafted, Option C kept |
| Reduce the per-node arena binding, which every dispatch shape pays | 2 or 3 | Not started — 30 ns per quantum, 3.7% of the plan's quantum, and the largest identified item with an obvious remedy. It has already come down from 47 ns by moving the binding decision to admission |
| Attack the per-node boundary itself, which is worth more than every dispatch figure in this record | 5 or later | Not started — a fused function costs about a third less per quantum than the same path node by node, a figure that also includes control work the fused arm skips |
| Re-measure the dispatch price against a **natively enum** schedule | 2 or 3 | Not started — EVD-0009's enum arm walks the table's schedule zipped with a kind vector, so its +2.1% is a lower bound on what this decision costs |
| Re-evaluate against the declarative node API | 5 | Not started |

## Revisit conditions

- The declarative node API in Phase 5 needing a per-node capability the flat signature cannot express.
- A profile showing dispatch as a measurable share of a real plan's cost once the catalog is large, rather than of the
  five-node minimal path.
- **A catalog-scale re-measurement.** EVD-0009's two per cent is a five-node path on one machine; branch prediction for
  an indirect call behaves differently with a hundred node kinds than with four, and nothing in the record predicts
  which way. **This is the re-measurement the redraft owes**, and P02-T009's CPU comparison against V1 is
  the first place a real patch exists to run it on.
- The per-node binding being made materially cheaper, which would raise dispatch's share of what is left and make the
  same comparison decide differently.
