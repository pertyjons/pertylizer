# ADR-0040: V2 Owns Its DSP

| Field         | Value                                                        |
|---------------|--------------------------------------------------------------|
| ID            | ADR-0040                                                     |
| Status        | Proposed — **decided, not yet accepted**; see clause 7 and ADR-0041 |
| Phase         | 2                                                            |
| Created       | 2026-08-18                                                   |
| Last reviewed | 2026-08-18                                                   |
| Related       | ADR-0002, ADR-0004, ADR-0041, P02-T006, P02-T008, EVD-0008, EVD-0010, master plan Phase 2 and Phase 5 |
| Supersedes    | —                                                            |
| Superseded by | —                                                            |

## Context

The master plan's Phase 2 work list says: *"Use existing DSP kernels where their state and configuration can be
separated cleanly. Use a narrow temporary adapter where that is cheaper than extraction."* The Phase 2 tracker turned
that permission into a commitment — **the extraction path**, one shared home in `synth_dsp` that both engines call,
decided by the user on 2026-08-17 with the stated aim of mixing as little V1 into V2 as possible.

**The tracker's own survey already knew part of this.** Its pre-choice notes record that V1's envelope recomputes its
stage times, its exponential coefficient and its curve shaping *per sample*, and that V2 wants that as prepared
per-quantum data. The extraction path was chosen with that in view. What P02-T005 added is not the discovery that the
algorithms differ — it is a working V2 vocabulary against which the cost of reconciling them is concrete rather than
anticipated, and a second question the survey did not weigh: which engine governs the shared code afterwards.

**What the vocabulary actually looks like beside V1's**, established by reading both:

| V2 kernel | V1 counterpart | Are they the same algorithm? |
|-----------|----------------|------------------------------|
| Filter | [`SvfCoeffs`](../../../crates/synth_dsp/src/filters.rs), `synth_modules::Filter` | **The recurrence, yes** — both are `f32` over stored `f32` coefficients. The coefficient *semantics*, no: V1 maps resonance as `k = 2 − 2r` with `r` a `NormalizedValue` clamped to 0.99 and prepares in `f32`; V2 uses `k = 1/Q` and prepares in `f64` before storing `f32` |
| Sine | [`synth_modules::Oscillator`](../../../crates/synth_modules/src/oscillator.rs) | **No.** V1 evaluates `crate::math::fast_sin_turns` — an `f32` approximation — over an `f32` `Phase`, with per-sample FM/PM, sync and unison. V2 accumulates in `f64` and calls `f64::sin` |
| Envelope | [`synth_modules::Envelope`](../../../crates/synth_modules/src/envelope.rs) | **No.** V1 is a one-pole with an overshoot target and per-segment curve controls. V2 is a linear ramp over frame counters, so that a segment lasts exactly the frames it was authored |
| Amplifier | `synth_modules::Amplifier` | **The scalar multiply, yes.** The surrounding node is not: V1 carries stereo selection, pan, clipping, bipolar CV and level smoothing |

What that table establishes is the algorithms' *present* difference — not that no seam exists. Other seams are
arguable and one was named by the tracker's own survey: a shared stage machine with per-engine segment laws, or shared
mathematical primitives below the kernel. P02-T005 chose different semantics; it did not prove extraction impossible.
What it did establish is that the seam is not the obvious one, and that finding it is a design question rather than a
move.

**The asymmetry that makes this a decision rather than a detail, stated no more strongly than it holds.** The corpus's
ten `output.sha256` values bind **across the extraction** — that is what the tracker requires, and it is not a claim
that V1's output is immutable forever; V1 development and releases continue, and a shared function could later be
versioned or forked.

What sharing changes is *who has to justify a change*. In a shared kernel, an improvement V2 wants is a modification to
code V1 renders through, so it arrives with a bit-exact comparison attached and a fork as its escape hatch. The
argument for separate ownership is therefore not that sharing makes divergence impossible — it is that it makes every
divergence cost a fork, and that the discipline of maintaining that fork lands on the phase that is trying to build
something new. Whether that price is worth paying is what this record decides.

**Outside this decision.** Which memory layout V2's arena uses (ADR-0002, and see *Consequences*); how a node is
declared (Phase 5); the `LegacyPolyModuleAdapter` that lets V2 host V1 modules (Phase 5); and anything about V1's own
architecture, which this record does not touch at all.

## Decision drivers

- **V2 exists to be better, and "better" has to be able to differ.** A shared kernel cannot be improved without either
  changing what V1 renders or forking the function. Neither is forbidden; both put a bit-exact comparison in front of
  every V2 improvement, which is a tax on exactly the work this phase exists to do.
- **The engine with the contract sets the default.** Not by anyone's intent — by which side has digests attached.
- **Duplication has a real cost**, and it is the cost this record is buying something with.
- **Phase 2's node count is five.** The "do not rewrite forty modules" argument is real and belongs to Phase 5, which
  already owns the adapter for exactly that.
- **The exit gate does not demand imitation.** It reads *"musically equivalent to V1 **or** has a documented
  intentional difference"*, and the corpus manifest carries `preserve` and `change` lists per case. The apparatus for
  "different on purpose, recorded" was built in Phase 0A.

## Options considered

### Option A: Extraction — one shared home both engines call

The tracker's recorded choice. Move each kernel's arithmetic into `synth_dsp`, keep parameter resolution and
modulation in each engine, and hold the corpus digests bit-identical across the move.

Where the algorithms coincide this is genuinely good: one implementation, one place to fix a bug, and V2 inherits DSP
that has shipped for years. Where they do not, it produces a function that takes a policy — `OnePoleEnvelope` or
`LinearEnvelope`, `from_normalized_resonance` or `from_quality_factor`, a per-quantum state flush one caller wants and
the other does not have, a block size one caller assumes. After four nodes `synth_dsp` is a museum of two engines'
semantics, and every later change to it arrives with V1's digests attached.

(V1 is not indifferent to denormals — [`DenormalGuard`](../../../crates/synth_core/src/types/denormal.rs) sets
flush-to-zero for the audio thread. What it has no counterpart for is V2's explicit `1e-30` flush of a filter's stored
state, which is a different mechanism at a different layer.)

Its failure mode is quiet: nobody decides to lower V2's quality. The kernel simply already exists, so the question of
whether it is the right one never gets asked.

### Option B: V2 owns its DSP

`synth_dsp` stays V1's. V2's kernels live in `synth_engine_v2::node::kernels` and answer to V2's contracts. The corpus
still runs, as a comparison rather than as a target.

Costs: the same arithmetic exists twice, a bug found in one engine is not fixed in the other, and V2's DSP has to be
justified on its own terms instead of inheriting a reputation. That last cost is also the point.

### Option C: V2 adopts V1's algorithms wholesale

V2 takes the one-pole envelope, the `fast_sin_turns` oscillator and V1's resonance mapping. Then one shared kernel
really does serve both, and the equivalence gate gets its best chance — though **not by construction**: identical
kernels do not make identical renders while parameter resolution, modulation, smoothing, phase initialisation, voice
allocation and stereo handling still differ between the engines.

It is the cheapest way to close Phase 2's third gate bullet, and it is the option this record most nearly took. What
it costs is the phase's purpose: V2 would inherit an `f32` sine approximation into a crate whose whole claim is that it
can state its guarantees, and lose P02-T005's exact segment durations — a property built two days ago *because* an
accumulated increment was measured arriving tens of samples early.

### Status quo

Attempt the extraction as recorded and settle each collision as it appears. Every one of those settlements is a
decision about whose sound is right, made inside a task whose subject is where code lives, without a record. This is
the option that produces the museum without anyone choosing it.

## Evidence

- **Direct reading of both implementations**, cited in the table above with file paths. The four comparisons are
  structural facts about the code, not measurements, and they do not need to be measured: two algorithms either are
  the same expression or they are not.
- **[EVD-0008](../evidence/phase-02/EVD-0008-internal-channel-layout-cost.md)** — relevant because ADR-0002 was
  accepted *on the shared-kernel premise this record removes*. See *Consequences*.
- **An independent review of the extraction plan**, run before any code was written, which reached the same
  conclusion from the same files and added the concrete traps: that regrouping V1's `base_level * cv` products can
  change rounding, that V2's `1e-30` flush of a filter's stored state must not enter a shared recurrence because V1 has no counterpart to
  it at that layer — V1 handles denormals with a flush-to-zero guard on the audio thread instead — and
  that *"calling a policy-dispatching function 'one shared kernel' would conceal the actual design decision rather
  than resolve it."*
- **Uncertainty that remains.** Nobody has measured whether V2's kernels sound better, worse, or merely different
  from V1's. This record does not claim they are better; it claims the question should be cheap to ask, and that a
  shared kernel prices it at a bit-exact comparison or a fork. P02-T008 is where the difference gets measured.

## Decision

1. **V2 owns the DSP it renders.** Kernels reachable from V2's render loop live in `synth_engine_v2`, and
   `synth_dsp` remains V1's. V2 may depend on `synth_dsp` for a value, a table, or a mathematical primitive that is
   *not* a kernel; it may not route its audio through a function whose behaviour V1's corpus digests pin.
2. **No shared kernel may carry a policy that exists to serve two engines.** A parameter that selects between V1's law
   and V2's law is the thing this record refuses. If two engines need two behaviours, they get two functions with two
   names in two crates, and the difference is legible from the names.
3. **Each V2 node kind justifies itself on its own criteria**, stated as executable checks in the crate rather than as
   likeness to V1. This is the obligation Option B buys, and it applies to every node added from here: the filter's
   unity gain at DC and its three refusal classes, the envelope's exact segment durations, and whatever the next node's
   equivalent is.
4. **The Phase 2 exit gate's third bullet keeps both of its branches, and this record chooses neither in advance.**
   P02-T008 runs the comparison over the corpus and records **every** difference it finds. What each difference then
   needs depends on what it is, and the distinction matters because two renders can be musically equivalent while
   differing in every sample:

   - a difference that leaves the case's `preserve` claims satisfied needs a **cause**, and the gate's first branch
     carries it;
   - a difference that **breaks a `preserve` claim is a failure** — not a documented difference — unless that claim is
     itself changed by a recorded decision. The manifest asserts envelope landmarks and filter spectral balance for
     the minimal case, and this record has no authority over those;
   - a difference the phase *wants* takes the gate's second branch, and then it needs a **named intentional
     disposition** rather than only an explanation of where it came from.

   What this record removes is the assumption that bit equality is the only way through. It does not remove the gate,
   and it does not choose the branch in advance.
5. **P02-T006 stops being an extraction task, and nothing of it survives inside this phase.** The extraction is not
   deferred; it is not happening. The eleventh corpus fixture went with it: that fixture existed to catch a regression
   in V1's parameter-override path *during the extraction*, and with no change to V1 there is no such regression to
   catch. It may still be worth adding on the corpus's own terms — the manifest's coverage is P00A-T001's subject, not
   this record's — and V2's own automation belongs to Phase 5. This record therefore makes no claim on it either way,
   which is a change from an earlier draft that kept it without a reason.
6. **This decision does not license divergence for its own sake.** Where V1's algorithm is the right one, V2 should
   implement the same algorithm — written into V2, tested in V2, and free to be improved later without a corpus
   digest deciding it.

7. **This record cannot be accepted on its own, and it says so rather than leaving it to be noticed.** Accepting it
   triggers [ADR-0002](ADR-0002-internal-channel-layout.md)'s second revisit condition **verbatim** — *"the
   shared-kernel constraint disappearing while the node catalog is still small"* — and that record's decision section
   states the consequence in advance: *"If the shared-kernel constraint is dropped… the measurement selects
   interleaved."* ADR-0002 is `Accepted`, `Contract` class, and its supporting argument is the premise this record
   removes.

   The Core V2 process governs what follows: conflicting authorities stop only
   their dependent work, and the current specification must present one
   coherent rule. Therefore:

   - **P02-T006 and P02-T007 do not proceed** while this conflict is open. T006's subject is the extraction this
     record removes; T007 adds nodes, and every node added makes the layout question dearer to answer — which is
     precisely what ADR-0002's revisit condition is about.
   - **A companion decision on the internal channel layout is required before either record is accepted.** It must
     either supersede ADR-0002's layout clauses or retain planar on a rationale that survives without the
     shared-kernel premise — for instance clause 2's kernel contract, *"a kernel receives one channel and is never
     told how many there are"*, which is an independent merit that ADR-0002 states only in passing.
   - **The instrument for that decision exists now and did not before.** EVD-0008 says of itself that it *"models the
     two memory layouts under the same arithmetic"* rather than V2's code; P02-T005 built the real voice path, so the
     re-measurement ADR-0002 already owes could be run against real kernels instead of a model. **It has been**:
     [EVD-0010](../evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md), which is what ADR-0041 now has to
     weigh — and which found the layout worth 2.54% on the path this phase renders today and 11% to 22% on a path
     whose signal genuinely has channels.

   Accepting this record while leaving ADR-0002 accepted on a withdrawn premise would leave two authorities in
   conflict and a `Contract`-class decision resting on nothing. That is the state this clause exists to prevent.

## Consequences

### Positive

- V2's DSP can be improved without a bit-exact comparison against V1 attached to the change.
- `synth_dsp` does not acquire two-branch kernels that nobody dares touch afterwards.
- The quality question is asked once per node, deliberately, instead of being answered by whichever implementation
  happened to exist.
- V1 is not modified by Phase 2 at all, which removes the largest risk the extraction path carried: that a change made
  for V2's benefit alters what a user's existing project sounds like.

### Negative

- **The same arithmetic exists twice**, and a bug fixed in one engine is not fixed in the other. This is the price, and
  it is not small: a filter correction in V2 leaves V1's users with the old behaviour, and nothing in the build will
  say so.
- **V2 forfeits an argument it could have leaned on.** "It sounds the same as V1" is no longer available as evidence
  that V2 is correct; each kernel has to carry its own.
- **Two engines will drift**, and the longer both ship the wider the gap. Whether that gap is a problem is a product
  question this record does not answer.

### Risks and controls

- **Risk: V2 quietly sounds worse, and nobody notices because equality is no longer required.** Control: clause 4
  keeps the measurement mandatory even though the equality is not. A difference that is not explained is a P02-T008
  finding, not a pass.
- **Risk: "V2 owns its DSP" becomes licence to rewrite what already works.** Control: clause 6, and the fact that
  Phase 5's adapter — not this record — is how V2 gets the rest of the catalog.
- **Risk: divergence is discovered late, when both engines have users.** Control: partial. P02-T008 measures the gap
  once, and no accepted record requires the comparison to be repeated after it. Making it recurrent would be a change
  to the Core V2 process or roadmap, and this record notes the gap rather than legislating one.

## Follow-up work

| Task | Phase | Status |
|------|-------|--------|
| **Companion decision on the internal channel layout** — supersede ADR-0002's layout clauses or retain planar on a surviving rationale | 2 | **Drafted** as [ADR-0041](ADR-0041-interleaved-internal-channel-layout.md), which supersedes ADR-0002 in full and takes the interleaved arena. User decision, 2026-08-18. Clause 7 still binds: **this record stays `Proposed` until ADR-0041 is accepted** |
| Re-measure the layout against the **real** voice path, as the input to that decision | 2 | **Complete** — [EVD-0010](../evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md). Interleaved cheaper in all nine runs of all three shapes: 2.54% on the plan compiled today, 21.56% on a stereo chain, 11.05% with per-channel control |
| Close P02-T006 as not-happening, and record the dropped extraction as a deviation rather than an omission | 2 | Not started — after clause 7 clears. Clause 5 leaves nothing of the task inside this phase, including the eleventh fixture |
| P02-T008 records every corpus difference with a cause, under clause 4 | 2 | Not started |
| Decide how V2 acquires the rest of the catalog: adapter, port, or rewrite | 5 | Not started — ADR-0004's declarative node API is the surface it lands on |

## Revisit conditions

- **V1 retires.** Then no corpus digest stands behind the shared code, and one home is simply better than two.
- **A specific kernel where V1's implementation is measurably superior and V2's is not improvable**, at which point
  copying that algorithm into V2 is the right move — which clause 6 already permits, and which is not the same as
  sharing the code.
- **Drift becoming a product problem**: two engines whose corpus comparison shows differences a user would call a bug
  rather than a change.
