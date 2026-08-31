# ADR-0049: The tempo-ramp law under clause 15

| Field | Value |
|---|---|
| ID | ADR-0049 |
| Status | Accepted |
| Phase | 3 |
| Created | 2026-08-31 |
| Last reviewed | 2026-08-31 |
| Related | ADR-0032 clause 15, ADR-0042, `SPEC` sound core render contract, `CORPUS-0001` |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

**Delivered musical timing, and an explicit product choice.**

A tempo ramp decides the frame every later note lands on. It is not persisted and not on a wire — the ramp
*declaration* is V1's and this record does not touch it — but a listener hears the result, and changing it later
changes what they hear. That is the durable boundary, and it is the same one ADR-0042 crossed for the envelope
segment shape.

ADR-0032 clause 15 already fixed the constraint and named this hole: the conversion law "must be expressible in
operations whose results are identical on every supported target — the four IEEE-754 arithmetic operations,
comparison, and rounding", because "a tempo ramp implemented through a transcendental function would make the frame a
note lands on depend on the platform's libm, which the determinism digest cannot tolerate". It then says in as many
words that "Phase 3 owns what the conversion *is*, including ramp semantics". This record fills that hole. It does
not amend clause 15.

**Why now.** The tempo-map slice landed steps only and left `TempoChange::ramp` unbuilt, because the choice between
stating an exact evaluation and selecting an expressible shape is durable in two different directions and the
maintainer declined to make it under time pressure. Phase 3's exit gate requires that "tempo steps **and ramps** map
to stable sample positions", so the immediately next slice in that stream cannot proceed without the answer.
Deferring further is not free either: Phase 4 lowers current projects through this map, and a lowering built while
the law is open would commit to a shape by accident rather than by decision.

**Coupled decisions.** None. The record needs no other open decision to be implementable, and it closes none of
theirs.

## Decision boundary

**What law converts a musical position inside a ramp segment into a `PlanPosition`.** That is the whole of it.

**Non-goals, each stated because a reader could reasonably expect it here.**

- **The inverse direction is not defined.** Nothing on the render path converts a position back to a tick: ADR-0032
  clause 27 makes anchoring the only place plan time and engine time meet, and the one position-aware kernel reads
  frames. A GUI playhead that wants a musical readout is Phase 11's, and the inverse of the selected law needs a
  square root — correctly rounded on every IEEE-754 target, but not in clause 15's listed set. Extending the set is
  that phase's decision to ask for, on evidence this record does not have.
- **The declaration is V1's and is unchanged.** `TempoChange` keeps `{ tick, bpm, ramp }`, where a ramp runs from
  `bpm` at `tick` toward the **next** change's tempo, reaching it exactly at that change, and holds constant when no
  change follows. Phase 4's lowering therefore translates nothing; only the interpolation between the two declared
  endpoints differs.
- **Clause 15's single rounding is not reopened.** Seconds accumulate in `f64` and are rounded to a frame exactly
  once, half away from zero, at the query. A ramp changes what is summed, not how many times it is rounded.
- **Time-signature and tempo-map *replacement* semantics are elsewhere.** Re-admitting entitlements against a
  replacement map is ADR-0046's and is owed by its own slice.

## Evidence

Verified against the code and the corpus at this commit's parent.

- **V1 ramps tempo linearly in tick space**, so elapsed time is the integral of `60/bpm` over a linear `bpm`, which
  is a logarithm. `crates/synth_sequencer/src/song.rs` evaluates it as `K * ln_1p((b_q - b0) / b0) / (b1 - b0)` with
  `K = full_beats * 60`, and takes a **separate branch when `|b1 - b0| < 1e-5`**, falling back to a constant tempo at
  the endpoint average to avoid the `0/0` cancellation. The inverse uses `exp_m1` and clamps its fraction to
  `[0, 1]`.
- **V2's map has no ramp constructor.** `crates/synth_engine_v2/src/tempo.rs` builds segments of constant tempo and
  computes `beats * 60.0 / bpm`; a standing source scan asserts the conversion uses only the four operations, so a
  ramp cannot arrive later by quietly calling a library.
- **The reference corpus contains a ramp**, and the claim that it does not is false. `CORPUS-0001`'s
  `corpus/v2-reference/projects/tempo-map-arrangement.ptz` declares
  `[{tick 0, 90 BPM, ramp}, {tick 1920, 180 BPM, step}, {tick 3840, 120 BPM, step}]` — a ramp from 90 to 180 BPM over
  two beats at 960 ticks per quarter. `tempo.rs`'s module documentation and `NOW.md` both said no project in this
  repository sets one; the tempo slice's deferral rested partly on that sentence. Both are repaired by this record's
  acceptance. No project under `assets/examples/` sets a ramp, which is the narrower true statement.
- **The three candidate laws, over that fixture and over the documentation example**, computed from the closed forms
  rather than measured:

  | Case | V1, tempo linear in beats | Period linear in beats | Tempo linear in time |
  |---|---|---|---|
  | 90 → 180 BPM over 2 beats (the corpus fixture) | 0.924196 s | **1.000000 s** | 0.888889 s |
  | 60 → 120 BPM over 4 beats (the worked example) | 2.772589 s | **3.000000 s** | 2.666667 s |

  The selected law lengthens the corpus fixture's ramp segment by **75.8 ms**, 3 639 frames at 48 kHz, and every
  event after tick 1920 moves with it.

**Uncertainty that could change the decision.** Whether a musician reads "ramp" as *the tempo number moves at a
constant rate* or as *the beat lengthens at a constant rate*. The two are the same only in the limit of a small
change, and no measurement of that expectation exists. The selected law commits to the second reading, which is why
the consequence is stated in the delivered-behaviour section rather than left for a listener to discover.

## Options

### A. State V1's exact evaluation

Keep tempo linear in beats and satisfy clause 15 by writing the logarithm's evaluation into the specification: a
named series or rational approximation with a fixed term count, evaluated in the four operations, so every target
produces the same bits.

Preserves V1's delivered timing exactly, which is worth something the other options are not: the corpus fixture
renders identically and Phase 4's A/B needs no ramp category at all.

Its cost is a permanent numeric law this project owns and must prove. Bit-exactness would have to be established
rather than assumed, over the argument range a tempo ratio can reach, and against a reference the project also has to
choose. V1's `1e-5` fallback would have to be **specified rather than inherited**, because it is a discontinuity in
the law: at that threshold the shape switches to a constant tempo at the endpoint average, and a specification that
copied the branch without owning it would be copying an implementation accident into a durable contract.

### B. The period is linear in beats

Interpolate **seconds per beat** linearly between the segment's endpoints instead of beats per minute. Elapsed time
is then the integral of a linear function — a quadratic — which the four operations express exactly, with no series,
no approximation and no near-flat branch.

Its cost is delivered timing: the figures above, and a tempo *curve* that is no longer a straight line when drawn
against beats.

### C. The tempo is linear in time

Arguably the most natural reading of "the tempo ramps": beats per minute change at a constant rate per second. Beats
are then quadratic in time, and inverting that to place a beat needs a square root — correctly rounded under
IEEE-754 on every target, so it is deterministic, but it is not in clause 15's listed set and admitting it is an
amendment to an accepted record.

It is the furthest of the three from V1 (0.888889 s against 0.924196 s on the fixture) and buys nothing the second
does not, unless the straight-line tempo curve is itself the requirement.

### D. Keep refusing ramps

The status quo. It fails Phase 3's exit gate, which names ramps explicitly, and it leaves a corpus fixture the
lowering path cannot convert.

## Decision

**A ramp's period is linear in beats.** Option B, chosen by the maintainer on 2026-08-31 after the corpus finding was
put to them, in the knowledge that the fixture's timing moves.

### 1. The law

For a ramp segment spanning `[t0, t1)` at `TICKS_PER_QUARTER` ticks to the beat, with declared tempo `b0` at `t0` and
`b1` at `t1`, write `p0 = 60 / b0` and `p1 = 60 / b1` for the endpoint **periods** in seconds per beat, `B` for the
segment's length in beats, and `beta` for beats elapsed from `t0`:

```text
p(beta)       = p0 * (1 - beta / B) + p1 * (beta / B)
seconds(beta) = beta * 60 / b0 + (p1 - p0) * beta * beta / (2 * B)
```

Four operations and nothing else, which is what clause 15 requires. A step segment keeps the law it has,
`seconds(beta) = beta * 60 / b0`.

**The linear term is the step law's own value, shared rather than recomputed, and that is what clause 3 rests on.**
The implementation computes it once and either returns it or adds the quadratic term to it, so the ramp cannot
disagree with the step about a quantity they both need. A second copy of the expression could disagree: `beta * 60 /
b0` and `(60 / b0) * beta` are equal in exact arithmetic and differ by an `f64` rounding in about a third of random
argument pairs.

**What that difference is not is observable, and an earlier draft of this record claimed it was.** Sharing is the
property; the particular order is not. Four million random `(tempo, tick)` pairs drawn over 20 to 300 BPM and four
thousand bars produced **no** case where the two orders round to different frames — the discrepancy is around one
unit in the last place of a value in seconds, and half a frame is eleven orders of magnitude above it. The draft
said the alternative order "would make clause 3's bit-identity false", which is false twice over: the identity holds
under either order as long as both laws use the same one, and a mutation to the other order passes every check in
the suite. It was caught by running that mutation rather than by rereading the sentence.

**`p(beta)` weights the two periods rather than adding a difference to one of them**, and that is not cosmetic —
clause 7 says what each rejected form produced.

### 2. A segment's total duration is the trapezoid

Substituting `beta = B` gives `B * (p0 + p1) / 2`: the segment lasts its length in beats times the **mean of its two
periods**. The map does not evaluate that expression. It stores the next segment's prefix by evaluating clause 1 at
the segment's own end, which is the same code path a query inside the segment takes, so the two cannot disagree about
a boundary — the trapezoid is a fact about the law rather than a second implementation of it. Nothing downstream
learns whether a segment ramped.

### 3. A ramp with equal endpoints is exactly a step

`p1 - p0` is then `+0.0`, the quadratic term is a signed zero for every `beta`, and adding it to a non-negative sum
leaves that sum unchanged. The result is bit-identical to the step law, because clause 1 has the ramp **share** the
step's linear term rather than recompute it. **No near-flat branch exists, and none is needed** — which is the
concrete thing this law buys over option A, whose `1e-5` fallback is a discontinuity in the shape rather than a
numerical convenience.

The property is checkable rather than argued: `ramp(b, b)` and `step(b)` produce the same `PlanPosition` for every
queried tick, bit for bit. It was brute-forced before being written here, over 300 000 random triples of tempo,
elapsed beats and segment length drawn across nine decades in each, with no counterexample — the rule this
repository applies to an "always" claim — and its check is mutation-verified against the ramp recomputing the linear
term instead of sharing it.

### 4. A ramp with no following change holds constant

V1's rule, kept because the declaration is kept: a ramp toward nothing has no second endpoint, so the segment is a
step at `b0`. The map does not invent a destination tempo.

### 5. The single rounding is unchanged

Clause 15's rounding stays where it is, at the query, over the sum of the stored prefix and the offset inside the
segment. A ramp changes the second addend's formula and nothing else about the rounding.

### 6. Monotonicity is checked over the domain music reaches, and is not guaranteed beyond it

This clause withdraws a guarantee two earlier drafts made. The withdrawal is the decision, and the reason is a
measured trade rather than an inability to find a form.

**Why the accepted form is not monotone by composition.** The step law is: every operation in it is monotone in its
argument, so rounding can merge two ticks onto one frame but never reverse them. For a rising tempo the ramp's exact
function is a positive linear term **minus** a positive quadratic one, and the accepted evaluation inherits that
subtraction. Adjacent ticks can therefore convert to decreasing positions once the position's own rounding exceeds
the per-tick increment. The smallest such case anyone has constructed sits at about `2^45.85` frames — some
forty-two years of audio at 48 kHz — with a tempo ratio of five thousand.

**A monotone form exists, and this record rejects it on measurement rather than ignoring it.** An earlier draft of
this clause asserted that no algebraic form could be a composition of monotone operations; an independent review
refuted that by writing one. Taking the rising case relative to the segment's end,
`S(B) - [p1 * u + (p0 - p1) * u * u / (2B)]` with `u = B - beta`, the bracket is non-increasing in `beta` and the
whole expression is therefore non-decreasing by composition.

It is not adopted because it moves the error rather than removing it. Near `beta = 0` it subtracts two large
quantities that the accepted form never forms at all: over random ramps that cancellation placed a segment's own
start as much as **128 seconds before its own prefix**, which is a backwards step across the segment boundary and an
error many orders of magnitude larger than the one-frame inversion it removes. Trading a one-frame artefact at
forty-two years of audio for a two-minute artefact at the start of every steep ramp is the wrong direction, and the
figure is what makes that a judgement rather than a preference.

**What is claimed instead.** Positions are non-decreasing across steep ramps in both directions over an hour of
audio at 48 kHz, at a tempo ratio of three hundred, checked **tick by tick** in four windows rather than by a stride
— a stride steps straight over a one-frame inversion that recovers, which an independent review found the first
version of the check doing. Outside that domain the map's existing `2^53` refusal is the only bound, and a consumer
needing the guarantee further out must obtain a tighter position bound rather than assume one.

**Revisit path, so the next reader does not re-derive it.** Two routes, either of which would turn the checked
property into a guaranteed one: a form that is monotone by composition *and* accurate at both ends of the segment,
which neither of the two evaluated so far is; or a refusal at construction of maps whose frame span and tempo ratio
put them where the increment falls below the evaluation's own error. Neither is built, because nothing in this
project reaches a position decades of audio out at a tempo ratio in the thousands, and a rule nothing exercises is a
rule nobody has checked.

### 7. The reported tempo inside a ramp is the reciprocal of the period

`tempo_at` inside a ramp segment answers `60 / p(beta)`, in the four operations. The consequence is stated rather
than buried: the tempo **curve** is a hyperbola in beats, not a straight line, so a future tempo lane that draws a
ramp as a straight line between two BPM values would be drawing something the engine does not play. What is linear
is the beat's length.

**`p(beta)` weights the two periods and is then clamped to the interval they define, and every simpler form
produced a `Bpm` this type's own constructor refuses.** Three were tried and an independent review found each in
turn. `p0 + (p1 - p0) * beta / B` multiplies before dividing and overflows to infinity for a ramp toward a very slow
tempo, whose reciprocal is zero. Forming the fraction first leaves a subtraction that still cancels: for a
6000-to-`1e100` BPM ramp spanning the whole tick range the fraction reaches exactly `1.0` one tick before the end,
`(p1 - p0)` rounds to `-p0`, the sum is exactly zero, and the reciprocal is infinite. The weighted form alone can
round one unit in the last place below the endpoint period, and for a tempo near `f64::MAX` that reciprocal
overflows — a positive finite denominator does not imply a finite reciprocal.

The clamp is what closes it, and it is not a repair applied to a value that might be wrong. `p(beta)` is a convex
combination of `p0` and `p1`, so it **lies** between them for every `beta` in the segment; rounding is the only
thing that can put it outside, and the reciprocal of a value inside the interval is bounded by the two tempi the
caller declared, both of which `Bpm::new` has already accepted. This is clamping as documented domain behaviour,
which is the only kind this repository permits.

### 8. A tempo whose period is not finite is refused at construction

`Bpm::new` refuses a tempo below `60 / f64::MAX`, about `3.34e-307`, alongside the values it already refused. Such a
tempo's period overflows to infinity, and the ramp then evaluates `infinity * 0` at a segment's own start and
produces `NaN` — which reaches a position and a `Bpm` built inside `tempo_at` without passing through the
constructor at all. An independent review found it.

The check belongs at the newtype rather than at each use, which is where this repository puts an invariant, and it is
what lets clause 7 argue that its reciprocal is a real tempo instead of checking one. The refused range is one beat
per far longer than the age of the universe, so no caller loses a tempo it could have meant — but it **is** a
narrowing of what `Bpm::new` accepts, and it is named here rather than left to be discovered.

## Consequences and risks

- **Accepted cost — delivered behaviour.** A V1 ramp lowered to V2 lasts longer than it did whenever the tempo rises,
  and shorter whenever it falls, by up to the difference between the arithmetic mean of the periods and V1's
  logarithmic mean. On the one fixture in the reference corpus that is +75.8 ms, and every event after that segment
  moves by the same amount. Under `REV-P00A`'s rule this is an **intentional V1-to-V2 semantic change and must map to
  a comparison category rather than being treated as generic error** — the branch ADR-0042 took for the envelope
  segment shape. Phase 4's A/B owns creating that category; this record owes it the name and the reason, which are
  here.
- **Accepted cost — a musical reading is fixed.** "Linear" now names the period, not the tempo number. Clause 7 says
  what a display must not claim.
- **Safety/correctness control.** Nine named checks, each mutation-verified against its own falsifier and none
  against another's: an equal-endpoint ramp equals a step bit for bit, falsified by the ramp recomputing the shared
  linear term; a ramp's total duration equals `B * (p0 + p1) / 2`, the fixture's segment answering exactly
  1.000000 s at 48 kHz and explicitly not V1's 0.924196 s, falsified by the quadratic term's sign and by treating
  every change as a ramp; positions are non-decreasing across steep ramps in both directions, checked over adjacent ticks in
  four sampled windows and falsified by that same sign; a tempo whose period overflows is refused at construction, and a
  6000-to-`1e100` BPM ramp reports a real tempo one tick before its end, falsified separately by dropping the period
  check and by any of the three rejected interpolation forms; chained ramps each reach the **next** declared tempo with a
  continuous junction, falsified by pointing a ramp at the last one; a trailing ramp behaves as a step, falsified by
  giving it a degenerate destination; and the reported tempo is the reciprocal of the interpolated period rather
  than a straight line between two tempo numbers, falsified by reporting the declared tempo.

  **Two of those checks were written vacuous first and are recorded that way.** A four-beat fixture cannot make the
  elapsed fraction reach exactly `1.0` inside a ramp, so the first version of the interpolation check passed the
  mutation it was written against; and the source scan's stripping rule cannot be exercised by this module's own
  source, which holds no line where a `//` hides inside a string. Both were found by running the mutation rather
  than by reading the test, and both now reach their case.

  **The standing source scan is closed under calls, which is what lets it support the claim at all.** It scans the
  five functions the law reaches, asserts that every call those bodies make is either to one of the five or to a
  named arithmetic or accessor method, **and** asserts that no allowlisted name is itself a function this module
  defines — because a scan that stopped at the first of those would prove nothing: `segment_seconds` could call a
  harmlessly named `curve(x)` whose body holds `x.ln()`, and one that stopped at the second would let the same body
  hide behind the allowlisted name `min`. An independent review found each hole in turn, and both are
  mutation-verified. Adding the closure assertion immediately pulled `segment_for` and a newtype accessor into the
  scanned set, which is the check working rather than a nuisance. It strips comments, because the substring list is broad enough
  that the word "rising" failed a correct implementation once, and attributes, because `#[allow(...)]` is not a
  call — but it does **not** strip a line holding a quote, since splitting on `//` would truncate
  `("https://x", beats.sin()).1` and remove the call it exists to find. The `2^53` exactness guard and the
  finite-and-in-range check already in `position_of` cover the quadratic term without a new branch.

- **Owed, and named rather than discovered.** Phase 4's comparison category for a ramp. The inverse conversion, when
  a GUI needs one, with the clause 15 amendment it implies.
- **Revisit condition.** A user report that a drawn ramp and a heard ramp disagree reopens clause 7 and, with it, the
  choice between options B and C. Evidence that a target's `ln` is in fact reproducible across every supported
  platform would reopen option A, but only together with the branch that option would have to specify.

## Specification update

Acceptance creates **`SOUND-INV-019`** in the sound-core render contract, carrying clauses 1 to 7 as the rule
implementation follows, with a conformance row naming the four checks above. It also repairs two current statements
that this record's evidence falsifies: `tempo.rs`'s module documentation and `NOW.md` both assert that no project in
this repository sets a ramp.

## Review

Reviewer:

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract, safety/correctness defect, or
evidence incapable of supporting the claim. Editorial detail does not block.
