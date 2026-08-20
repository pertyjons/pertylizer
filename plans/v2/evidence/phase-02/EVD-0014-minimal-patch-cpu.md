# EVD-0014: What the minimal patch costs, V2 against V1

| Field | Value |
|---|---|
| ID | EVD-0014 |
| Status | Draft — method stated and reviewed, no data collected |
| Phase | 2 |
| Created | 2026-08-20 |
| Last reviewed | 2026-08-20 |
| Supersedes | — |
| Superseded by | — |
| Source revision | `3acb7e6f` |
| Retention | Permanent |
| Conclusion | Pending collection |
| Related | ADR-0001 clause 5, ADR-0004, ADR-0005, ADR-0037, ADR-0041, EVD-0009, EVD-0010, EVD-0011, EVD-0012, EVD-0013, P02-T009 |
| Artifacts | To be named on collection |

## Question and falsifier

The Phase 2 exit gate's fifth bullet asks whether **CPU use is no worse than V1
for the equivalent minimal patch, allowing a temporary documented margin for
adapters**. This record is that comparison.

Let `c(arm)` be the measured cost of rendering one second of audio through that
arm, in milliseconds of elapsed time, and `r(a, b) = c(a)/c(b) - 1`. These are
EVD-0010's, EVD-0011's and EVD-0012's quantities, and the estimator is
deliberately theirs so the figures can be read beside them.

### What would make the preferred conclusion wrong

The preferred conclusion is that V2 is not dearer than V1 on the path Phase 2
built. It is wrong if `r(V2, V1)` is positive by more than the measured noise
floor and the excess is not both **independently sized** and **temporary**. A
margin with no mechanism behind it is a gate failure, not a margin: "allowing a
temporary documented margin" licenses explaining an excess that a named later
phase removes, not having one.

### The rule table, fixed before collection

Evaluated **in order**, stopping at the first that applies, on the **governing
pair** defined below. `N` is the noise floor from control **C1**.

| # | Condition | Outcome |
|---|---|---|
| 0 | `N > N_max` | **Inconclusive.** The instrument cannot resolve this comparison, and no later rule is read. See below |
| 1 | `r(V2, V1) + N < 0` | **Pass.** V2 is measurably cheaper; the bullet closes with no margin |
| 2 | `\|r(V2, V1)\| ≤ N`, with `N ≤ N_max` | **Pass, not separable.** The instrument resolves better than `N_max` and still cannot distinguish the two, so the difference is smaller than the smallest one worth acting on |
| 3 | `r(V2, V1) - N > 0`, and rule 3's two conditions below are both met | **Pass with a documented margin** |
| 4 | `r(V2, V1) - N > 0` otherwise | **Fail.** A blocking P02-T009 finding |

#### Rule 0 exists so an unresolved comparison cannot close the gate

Without it, rule 2 rewards a bad collection: a run whose noise floor came out at
20% would declare "no worse" about a V2 that was 10% slower. An unresolved
comparison supports no conclusion, which is the evidence README's rule, so a
floor too wide to decide anything produces `Inconclusive` rather than a pass.

**`N_max` = 5.0%**, fixed here and not after seeing `N`. The justification is
this machine's own record rather than a preference: EVD-0010, EVD-0011 and
EVD-0012 all resolved margins on it with floors between roughly 0.9 and 4.6
percentage points, and EVD-0012's *History* records what a floor at the top of
that range looked like and what caused it. A floor above 5% would put this
collection outside everything previously measured here, which is a fact about
the run — thermal state, background load, a harness defect — and not about the
two engines.

#### Rule 3's two conditions, both required

The exit gate permits a **temporary** margin **for adapters**. Rule 3 is
narrowed to that and no further:

1. **Independently sized.** Every component of the excess is measured by its own
   arm in the same binary — a counterfactual that runs with the component and
   without it — not by subtracting the other components from the total. Sizing
   by subtraction makes any excess explicable by definition, because the last
   component absorbs whatever is left. The named components must then sum to
   within `N` of the measured excess; a remainder larger than `N` is rule 4.
2. **Temporary, with a named owner.** Each component is either work a named
   later phase removes or replaces, or work V2 does that V1 does not do *yet*.
   A component that is permanent — a cost V2 will still pay when Phase 5 and
   Phase 10 are done — is not a margin under this bullet, and an excess made of
   permanent costs is rule 4 however well explained.

Two components are already sized and can be claimed without new work: the
`clause-5` variant is its own arm here, and EVD-0009 sizes the arena binding at
32.18 ns per quantum against a walk-only control. Anything else needs its arm
before it can be named.

## Inputs and controls

### The two pairs, and which one governs

V1's offline render carries a sequencer, voice allocation, a mixer, a master
stage and stereo output. V2 carries none of them, because Phase 2 has not built
them. Comparing the two whole engines therefore measures, in part, **what V2 has
not yet implemented**, and reading that as a win would be reading absence as
speed. Comparing only the DSP path leaves out what a user actually pays. Both
are measured, and which one decides the gate is fixed here:

| Pair | V1 arm | V2 arm | Role |
|---|---|---|---|
| **`voice-dsp`** | `ModuleGraph::process` (`crates/synth_engine/src/graph.rs:452`) over the fixture's module graph, driven by `note_on` / `note_off` directly | `PreparedRenderer::render` over the compiled plan | **Governing.** It is the comparison between what each engine does to render a voice, which is what Phase 2 built |
| **`whole-render`** | `OfflineEngineSession`'s render of the `.ptz`, the path `pertylizer render` uses | `render_offline` over the same plan | **Context.** Reported, decomposed, and unable to fail the gate on its own |

**The two pairs are expected to disagree, and that disagreement is not
irresolution.** EVD-0012 made shape disagreement an inconclusive outcome because
its shapes were two answers to one question about one constant. Here the pairs
answer *different* questions — what the DSP costs, and what the engine around it
costs — and a difference between them localises the cost rather than
invalidating it. What `whole-render` can do is raise an alarm: if V2 were dearer
there while cheaper on `voice-dsp`, the engine layer V2 does not have would
somehow be costing more than V1's, which would mean the harness is wrong.

### `voice-dsp` is not symmetric work, and the asymmetry runs one way

`ModuleGraph::process` runs the fixture's five modules and then copies the
output module's `PortName::OUT`. That output module is `StereoOutput`, and its
`process` does more than V2's `Output` node does: it applies a master level and
an equal-power pan per channel, runs the clip/limit stage, updates two peak
meters with decay, writes an interleaved stereo buffer, and then writes three
output ports — `OUT_L`, `OUT_R`, and `OUT` as the mono downmix of the two
(`crates/synth_modules/src/output.rs:320`–`341`). V2's `Output` writes its mono
source to the profile's channels and does none of the rest.

This is stated rather than neutralised, because it cannot be removed: the module
is the graph's output and a `ModuleGraph` without one renders nothing. Its
direction is what makes it safe to leave in: it is work **V1 does and V2 does
not**, so it can only flatter V2. The `voice-dsp` figure is therefore an
**upper bound on V2's advantage**, and if the gate closes on rule 1 the margin
is smaller than measured. If instead rule 3 or 4 is reached — V2 dearer despite
carrying less work — that reading is strengthened, not weakened.

Sizing this component is rule 3 condition 1's job if it is ever claimed, and its
counterfactual arm is available: the same `ModuleGraph` measured with its output
module's meters and limiter reachable and with them not.

### The fixture

**EVD-0013's `aligned` fixture, unchanged.** Same graph, same parameters, same
44 100 Hz rate, **same stereo channel count**, same note at plan sample 0. The
two records share a fixture on purpose: a CPU figure for a patch whose
equivalence is measured elsewhere is worth more than one for a patch nobody
compared, and control **C3** below is what holds them to it.

The channel count is part of the fixture and not an afterthought. V2 renders at
the stereo profile in EVD-0013 so both arms are two-channel, and it renders at
the stereo profile here for the same reason: a mono V2 arm against a stereo V1
one would be a cheaper arm, and the cheapness would be the channel count rather
than the engine.

Its definition, its five closed asymmetries and the two differences that are its
subject are [EVD-0013](EVD-0013-minimal-patch-equivalence.md)'s and are not
repeated here.

### The variants

ADR-0001 clause 5 gives V2's renderer an input carry that
`PreparedRenderer::input_carry` allocates and never reads, because no Phase 2
node consumes live input. EVD-0012 measured the same renderer in two variants
for that reason, and the same two apply here — with one difference that matters
for the direction of the error:

| Variant | V2's arm | V1's arm |
|---|---|---|
| `as-built` | The renderer as it exists at the source revision | Unchanged |
| `clause-5` | The same, with clause 5's input-carry procedure performed around every call | **Unchanged — V1 has no counterpart** |

So `clause-5` is a **surcharge on V2 only**, and it moves `r(V2, V1)` upward.
Unlike EVD-0012, where the omission shrank every ratio toward zero and moved two
rules in opposite directions, here its direction is known: omitting it can only
flatter V2. The gate is therefore evaluated on **`clause-5`**, the conservative
variant, with `as-built` reported beside it. That choice is made here, before
collection, and is the opposite of choosing the favourable arm afterwards.

### The four arm slots, and what a sweep is

An **arm slot** is one measurement unit: an engine plus a repetition index.
There are exactly four, and they are the four things the permutation permutes:

| Slot | Engine | Runs |
|---|---|---|
| `V1·a`, `V1·b` | V1 | Both pairs. V1 has one variant, so its figure is labelled `as-built` and there is no `clause-5` V1 arm |
| `V2·a`, `V2·b` | V2 | Both pairs, in both variants |

A **sweep** is one pass over all four slots in that sweep's permutation order,
and every figure below is formed inside one sweep:

- a slot's figure for a pair and variant is the **minimum over 3 rounds**, each
  round timing one batch of calls;
- the sweep's engine figure is the **mean of that engine's two slots**, which is
  what balances slot against position;
- the comparison ratio is `r_sweep(V2, V1) = mean(V2·a, V2·b) / mean(V1·a, V1·b) - 1`;
- the two **null** ratios are `c(V1·a)/c(V1·b) - 1` and `c(V2·a)/c(V2·b) - 1`,
  each of which has a true value of zero because both members are the same
  engine measured twice;
- the reported `r(a, b)` is the **median over sweeps** of the paired ratios.

Pairing within a sweep is what makes the comparison survive drift; a ratio of
two separately pooled medians absorbs drift instead of cancelling it.

**All 24 permutations of the four slots, one per sweep**, so every ordered pair
of slots occupies every separation equally often. This is the gap EVD-0012 had
to amend around: its cyclic rotation held every pair at a fixed position
distance, so its null bounded some comparisons and not others. Here the design
removes it by construction rather than by correction, and sweeps are run in
multiples of 24 for that reason.

### Symmetry, stated before collecting

- **The same audio, the same length, the same call size.** Both arms of a pair
  render the same fixture for the same number of frames in calls of **256**
  frames — V1's `BUFFER_SIZE`
  (`crates/pertylizer/src/audio/arrangement_render.rs:58`), which is a
  compile-time constant and therefore the only block size V1 can be measured at
  without a rebuild. V2's caller block is free, so V2 matches V1 rather than the
  reverse.
- **The same binary.** One release binary contains both engines, which is what
  the exit gate's "estimator, draw count, build profile, and binary matched
  across the two arms" asks for and what EVD-0012's five separate builds could
  not offer. `synth_engine_v2` becomes a **dev-dependency** of `pertylizer` for
  it; that is not a widening of V2's own dependency surface, which is what
  Phase 1's `crate_boundary` test reads, and it does not reach the delivered
  dependency graph.
- **The same estimator, rounds and iterations in every slot**, and the same
  settle before the timed loop: both arms enter it with the envelope past its
  10 ms attack and 100 ms decay and held in sustain, so no arm times a different
  segment mixture than another.
- **Elapsed time, not CPU time**, as in EVD-0010 through EVD-0012, so these
  figures are comparable with those and not with EVD-0002's.

### The controls

- **C1 — the null pass, run first and as its own collection.** Before any
  comparison figure exists, the whole 24-sweep protocol is run with **all four
  slots holding the same engine**, so every ratio it reports has a true value of
  zero. Its median absolute ratio is one of the two quantities `N` is taken
  from. The gate's "control run first" is met literally: this collection
  completes and is read before the comparison collection starts.
- **The noise floor `N`** is the **larger** of C1's median and the comparison's
  own median absolute deviation across sweeps, which is EVD-0012's amended
  definition. Taking the larger can only make a comparison harder to resolve.
- **C2 — the in-process spread**, reported as a diagnostic and not as a
  threshold: each invocation measures each slot twice and reports the spread, so
  a noisy run is visible. EVD-0012 records why this is not the floor — it is a
  within-process quantity and every `r(a, b)` here spans slots.
- **C3 — the arms render the same music, and it is checked across arms rather
  than only within one.** A stability check would pass an arm that rendered
  silence every sweep, which is the cheapest possible way to win a timing
  comparison. So:

  - each `whole-render` arm digests its output, and the digest must equal the
    corresponding render in EVD-0013 under ADR-0041 clause 16's encoding;
  - each `voice-dsp` arm writes its output once, and the **two arms' outputs are
    compared against each other** through EVD-0013's own thresholds E1, E2 and
    E3b. `voice-dsp` stops before the master stage, so it has no counterpart
    render in that record and cannot be checked against one — but it can be
    checked against the other arm, which is the property that actually matters:
    that the two arms are rendering the same music;
  - and both `voice-dsp` outputs must be non-silent, asserted explicitly, since
    a silent pair would satisfy a difference threshold trivially.

  Without C3 the cheapest way to win this comparison is to render less, and
  nothing in a timing figure would say so.

### Environment

Release profile, `taskset -c 10,11`, matching EVD-0010 through EVD-0012 so the
figures sit in one series. This machine's two-level turbo behaviour is recorded
in EVD-0012's *History* and *Limitations* and is why an invocation is kept to a
few seconds: a sweep whose slots are minutes apart is a sweep whose slots were
measured at different clock frequencies.

## Method

The estimator is EVD-0012's, unchanged: a round times **one batch** of calls and
divides by the frames that batch rendered, so `Instant::now()` is read twice per
round rather than twice per call; the **minimum** over 3 rounds is that slot's
figure for that sweep. What a sweep is, and how its ratios are formed, is under
*The four arm slots* above.

Order of work:

1. Add the dev-dependency and write the harness.
2. Run **C3**'s digest and cross-arm checks on one invocation. It comes first
   because a harness whose arms render different music is not worth timing.
3. Run **C1**, the null pass, and compute `N`. If `N > N_max` the collection is
   `Inconclusive` under rule 0 and the comparison sweeps are not run — the
   instrument is what needs attention, not the engines.
4. Collect the comparison sweeps.
5. Apply the rule table mechanically, in a committed script, rather than by eye.

## Reproduction

To be completed on collection, in the shape EVD-0012's *Reproduction* uses.

## Results

Not collected. The method has had the independent review `PROCESS.md` requires
before a decision-driving measurement is taken, and the findings it returned are
in *History*.

## History

**The two records' methods were reviewed together before any data existed.**
Five of the review's nine blocking findings landed on this record. Four of them
changed a rule rather than a sentence:

- **The channel layout was an unclosed asymmetry.** V2 was to render mono
  against a stereo V1, which would have made the governing workloads unequal in
  V2's favour for a reason that has nothing to do with either engine's DSP. Both
  arms now render stereo, and the residual asymmetry that cannot be removed —
  `StereoOutput`'s pan, limiter, meters and interleave — is named, its direction
  stated, and its counterfactual arm identified.
- **Rule 2 turned lack of resolution into a pass.** With no ceiling on `N`, a
  collection whose floor came out at 20% would have declared "no worse" about a
  V2 that was 10% slower. Rule 0 and `N_max` = 5.0% are the repair, and the
  ceiling is justified from this machine's three prior records rather than
  chosen after seeing `N`.
- **Rule 3 was broader than the gate and operationally unfalsifiable.** It
  admitted permanent costs, which the bullet does not, and it let a component be
  named after the fact and sized by subtraction — under which any excess is
  explicable, because the last component absorbs the remainder. Rule 3 now
  requires each component to be temporary with a named owner **and** sized by
  its own counterfactual arm in the same binary.
- **The four permuted "arm slots" were never defined.** An implementer could not
  tell whether a slot was an engine, an engine-and-variant, a pair member or a
  repetition, which decides what C1's ratios mean and whether the permutations
  balance anything. *The four arm slots* now defines the slot, the sweep, and
  every ratio formed from them.

One finding was corrected in one direction and rejected in the other. The type
at `crates/synth_engine/src/graph.rs:452` is `ModuleGraph`, not `VoiceGraph`, and
that is fixed. But the claim that the V1 `voice-dsp` arm would therefore digest
silence — because `StereoOutput::process` populates only `OUT_L` and `OUT_R` —
was checked and is false: the module declares an `out` audio output
(`crates/synth_modules/src/output.rs:155`) and `process` writes it as the mono
downmix of the two channels (`:333`). The finding's underlying point was
nonetheless right and is taken: **C3 as drafted checked stability, not
equivalence**, and a stability check cannot catch an arm that renders silence
consistently. C3 now compares the two `voice-dsp` arms against each other
through EVD-0013's thresholds and asserts both are non-silent.

## Limitations

- **One machine, one processor, one build profile**, as EVD-0008 through
  EVD-0012. `r(V2, V1)` is a ratio of two per-sample costs and both move on
  another processor.
- **One patch.** The fixture is the smallest graph both engines can express, and
  it is the *only* graph both engines can express. Nothing here extrapolates to
  a patch with two oscillators, modulation, or more than one voice.
- **V1 is measured at one block size, because it has only one.** `BUFFER_SIZE`
  is a compile-time constant, so a block-size sweep on the V1 side is a rebuild,
  which EVD-0002 did and this record does not.
- **`clause-5` is an estimate of a cost that is not implemented**, exactly as in
  EVD-0012, and it is clause 5's procedure over a buffer of clause 5's size
  rather than the eventual implementation.
- **The governing pair is not symmetric work**, and the record does not pretend
  otherwise: V1's output module does more than V2's output node, so
  `r(V2, V1)` on `voice-dsp` is an upper bound on V2's advantage rather than a
  point estimate of it.
- **`whole-render` compares an engine against a renderer.** V2's sequencer,
  allocation and mixing do not exist, so that pair's figure is not a prediction
  of what V2 will cost when they do. It is reported for localisation, and the
  gate does not rest on it.
- **The known room is not measured here.** The phase record carries two sizeable
  bounds into this task — EVD-0009's fused-arm comparison, which is an upper
  bound on an opportunity rather than a measurement of what node boundaries
  cost, and the arena binding at 32.18 ns per quantum. This record measures V2
  against V1, not V2 against a fused ideal, so neither bound is retired by it.

## Conclusion

Pending collection.
